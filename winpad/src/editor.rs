//! Editor: the main application state and all editing operations.

use crate::buffer::Buffer;
use crate::commands::{canonical_key_string, Command, CommandRegistry, CommandSource};
use crate::plugins::{Hook, PluginManager};
use crate::types::{DocumentStats, EditOperation, LineEnding, Pos, Prompt, PromptKind, StatusMsg, UndoEntry};
use crate::utils::{byte_to_char_index, char_to_byte_index, clamp_usize, default_plugin_dirs, digits};
use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind},
    style::{self, Attribute, Color},
    terminal::{self, ClearType},
    QueueableCommand,
};
use std::cmp::{max, min};
use std::fs;
use std::io::{Stdout, Write};
use std::mem;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

/// The top-level application state.
///
/// Think of `Editor` as "the app": it owns the document, the cursor/selection, the UI state,
/// and all the subsystems (undo/redo, clipboard, commands, plugins).
pub struct Editor {
    /// The editable document (lines of text).
    pub buf: Buffer,

    /// Cursor position in the buffer (0-based line + 0-based char index in the line).
    pub cursor: Pos,

    /// Selection anchor. If `Some`, the selection runs from this anchor to `cursor`.
    /// If `None`, there is no selection.
    pub anchor: Option<Pos>,

    /// Viewport scroll position (top-left of what we are currently showing).
    /// These are in **line** and **char** units (not pixels).
    pub scroll_y: usize,
    pub scroll_x: usize,

    /// Path we'll save to. `None` means the buffer hasn't been associated with a file yet.
    pub file_path: Option<PathBuf>,

    /// "Dirty" means there are unsaved changes.
    pub dirty: bool,

    /// Optional bottom-line prompt (open/save/find/command/goto).
    prompt: Option<Prompt>,

    /// Short-lived status message shown in the status bar.
    status: Option<StatusMsg>,

    /// When the user first presses Ctrl+Q with unsaved changes, we record time here so we can
    /// require a second press within a time window to actually quit.
    last_quit_hint: Option<Instant>,

    /// Undo and redo stacks. Each item is a delta entry.
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,

    /// Clipboard access. On some systems/environments this can fail, so we keep it optional.
    clipboard: Option<arboard::Clipboard>,

    /// Command registry (built-ins + plugin-provided commands).
    commands: CommandRegistry,

    /// Loaded plugins and their scripts.
    plugins: PluginManager,

    /// Remember the last find query so Ctrl+F can pre-fill it.
    last_find: Option<String>,

    /// Whether the screen needs to be redrawn (set to true when state changes).
    /// This prevents unnecessary flickering by only rendering when something actually changed.
    needs_redraw: bool,

    /// Whether word wrapping is enabled.
    pub word_wrap: bool,

    /// Whether the help screen is currently displayed.
    pub show_help: bool,

    /// Whether the stats screen is currently displayed.
    pub show_stats: bool,
}

impl Editor {
    /// Create a new editor and optionally open the path passed on the command line.
    ///
    /// Steps:
    /// - initialize the buffer (empty or file contents)
    /// - register built-in commands
    /// - load plugins and let them register their commands
    /// - if a file is opened, call the plugin `on_open` hook
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let mut buf = Buffer::new();
        let mut file_path = None;

        // Open initial file (optional). If path doesn't exist, we keep it as the save target
        // and start with an empty buffer.
        if let Some(p) = path {
            if p.exists() {
                let s = fs::read_to_string(&p)
                    .with_context(|| format!("Failed to read file: {}", p.display()))?;
                buf = Buffer::from_string(&s);
                file_path = Some(p);
            } else {
                // open empty but set path (save goes there)
                file_path = Some(p);
            }
        }

        // Clipboard is "best effort": `arboard` can fail in headless / unusual terminals.
        let clipboard = arboard::Clipboard::new().ok();

        // Built-in commands are registered here, then plugins may add more commands.
        let mut commands = CommandRegistry::new();
        register_builtin_commands(&mut commands);

        // Load plugins from disk and allow them to register commands/keybindings.
        let plugins = PluginManager::load(default_plugin_dirs()?, &mut commands)?;

        let mut ed = Self {
            buf,
            cursor: Pos { y: 0, x: 0 },
            anchor: None,
            scroll_y: 0,
            scroll_x: 0,
            file_path,
            dirty: false,
            prompt: None,
            status: None,
            last_quit_hint: None,
            undo: vec![],
            redo: vec![],
            clipboard,
            commands,
                   plugins,
                   last_find: None,
                   needs_redraw: true, // Initial render needed
                   word_wrap: false,
                   show_help: false,
                   show_stats: false,
               };

        // Call plugin on_open hook if relevant.
        // This is a good example of why `Editor` is passed into plugin hooks: plugins can
        // modify the buffer, selection, or status message at startup.
        if let Some(p) = ed.file_path.clone() {
            // Temporarily move plugins out to avoid borrow checker conflict
            let mut plugins = mem::take(&mut ed.plugins);
            plugins.call_hook(&mut ed, Hook::OnOpen, Some(&p))?;
            ed.plugins = plugins;
        }

        ed.set_status("Ctrl+P commands • Ctrl+S save • Ctrl+Q quit", Duration::from_secs(4));
        Ok(ed)
    }

    /// Mark that the screen needs to be redrawn.
    /// Call this whenever editor state changes (cursor, text, selection, etc.).
    fn mark_redraw(&mut self) {
        self.needs_redraw = true;
    }

    /// Toggle word wrapping.
    pub fn toggle_word_wrap(&mut self) {
        self.word_wrap = !self.word_wrap;
        if self.word_wrap {
            self.scroll_x = 0;
        }
        self.set_status(format!("Word wrap: {}", if self.word_wrap { "on" } else { "off" }), Duration::from_secs(2));
        self.mark_redraw();
    }

    /// Toggle between LF and CRLF line endings.
    pub fn toggle_line_ending(&mut self) {
        self.buf.line_ending = match self.buf.line_ending {
            LineEnding::LF => LineEnding::CRLF,
            LineEnding::CRLF => LineEnding::LF,
        };
        self.dirty = true;
        self.set_status(format!("Line endings converted to {}", self.buf.line_ending.name()), Duration::from_secs(2));
        self.mark_redraw();
    }

    /// Periodic "background" updates (called when no input event arrives).
    ///
    /// We currently only use it to expire status messages after their TTL.
    pub fn tick(&mut self) {
        // expire status messages
        if let Some(st) = &self.status {
            if Instant::now() >= st.until {
                self.status = None;
                self.mark_redraw(); // Status bar changed
            }
        }
    }

    /// Called when the terminal is resized.
    ///
    /// We need to redraw because the layout changed.
    pub fn on_resize(&mut self) -> Result<()> {
        self.mark_redraw();
        self.ensure_visible()?;
        Ok(())
    }

    /// Show a message in the status bar for `ttl` duration.
    pub fn set_status(&mut self, msg: impl Into<String>, ttl: Duration) {
        self.status = Some(StatusMsg { text: msg.into(), until: Instant::now() + ttl });
        self.mark_redraw();
    }

    /// Capture an undo snapshot and clear the redo stack.
    ///
    /// Typical usage: call this right *before* a user-visible edit (insert/delete/paste/etc).
    /// Capture an undo entry.
    ///
    /// Typical usage: call this right *before* a user-visible edit (insert/delete/paste/etc).
    /// Provide the operation that is about to be performed.
    fn record_edit(&mut self, op: EditOperation) {
        // cap undo history
        const CAP: usize = 1000; // Can be much higher now that we use deltas!
        let entry = UndoEntry {
            op,
            cursor_before: self.cursor,
            anchor_before: self.anchor,
        };
        self.undo.push(entry);
        if self.undo.len() > CAP {
            self.undo.drain(0..(self.undo.len() - CAP));
        }
        self.redo.clear();
    }

    /// Undo the most recent edit (if any).
    fn undo(&mut self) -> Result<()> {
        if let Some(entry) = self.undo.pop() {
            // Determine the inverse operation to store in redo
            let redo_op = match &entry.op {
                EditOperation::Insert { pos, text } => {
                    // Undo an insert: delete the text we just inserted
                    let end = self.buf.calc_end_pos(*pos, text);
                    self.buf.delete_range(*pos, end);
                    EditOperation::Delete { start: *pos, _end: end, deleted_text: text.clone() }
                }
                EditOperation::Delete { start, _end: _, deleted_text } => {
                    // Undo a delete: insert the text we just deleted
                    self.buf.insert_str(*start, deleted_text);
                    EditOperation::Insert { pos: *start, text: deleted_text.clone() }
                }
            };

            self.redo.push(UndoEntry {
                op: redo_op,
                cursor_before: self.cursor,
                anchor_before: self.anchor,
            });

            self.cursor = entry.cursor_before;
            self.anchor = entry.anchor_before;
            self.dirty = true;
            self.mark_redraw();
            self.ensure_visible()?;
        }
        Ok(())
    }

    /// Redo the most recently undone edit (if any).
    fn redo(&mut self) -> Result<()> {
        if let Some(entry) = self.redo.pop() {
            // Determine the inverse operation to store back in undo
            let undo_op = match &entry.op {
                EditOperation::Insert { pos, text } => {
                    let end = self.buf.calc_end_pos(*pos, text);
                    self.buf.delete_range(*pos, end);
                    EditOperation::Delete { start: *pos, _end: end, deleted_text: text.clone() }
                }
                EditOperation::Delete { start, _end: _, deleted_text } => {
                    self.buf.insert_str(*start, deleted_text);
                    EditOperation::Insert { pos: *start, text: deleted_text.clone() }
                }
            };

            self.undo.push(UndoEntry {
                op: undo_op,
                cursor_before: self.cursor,
                anchor_before: self.anchor,
            });

            self.cursor = entry.cursor_before;
            self.anchor = entry.anchor_before;
            self.dirty = true;
            self.mark_redraw();
            self.ensure_visible()?;
        }
        Ok(())
    }

    /// Return the normalized selection range as `(start, end)` (both 0-based).
    ///
    /// Normalized means `start <= end` regardless of selection direction.
    pub fn selection_range(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cursor {
            None
        } else if a <= self.cursor {
            Some((a, self.cursor))
        } else {
            Some((self.cursor, a))
        }
    }

    /// Clear any selection (cursor remains where it is).
    fn clear_selection(&mut self) {
        self.anchor = None;
        self.mark_redraw();
    }

    /// Select the entire buffer.
    fn select_all(&mut self) {
        self.anchor = Some(Pos { y: 0, x: 0 });
        let last_y = self.buf.line_count().saturating_sub(1);
        let last_x = self.buf.line_len_chars(last_y);
        self.cursor = Pos { y: last_y, x: last_x };
        self.mark_redraw();
    }

    /// Extract the selected text as a string (empty string if no selection).
    ///
    /// If the selection spans multiple lines, we join them with `\n`.
    pub fn selected_text(&self) -> String {
        let Some((a, b)) = self.selection_range() else { return String::new(); };
        if a.y == b.y {
            let line = &self.buf.lines[a.y];
            let b0 = char_to_byte_index(line, a.x);
            let b1 = char_to_byte_index(line, b.x);
            return line[b0..b1].to_string();
        }
        let mut out = String::new();
        // first line
        {
            let line = &self.buf.lines[a.y];
            let b0 = char_to_byte_index(line, a.x);
            out.push_str(&line[b0..]);
            out.push('\n');
        }
        // middle lines
        for y in (a.y + 1)..b.y {
            out.push_str(&self.buf.lines[y]);
            out.push('\n');
        }
        // last line
        {
            let line = &self.buf.lines[b.y];
            let b1 = char_to_byte_index(line, b.x);
            out.push_str(&line[..b1]);
        }
        out
    }

    /// Delete the current selection (no-op if nothing is selected).
    fn delete_selection(&mut self) {
        if let Some((a, b)) = self.selection_range() {
            self.cursor = self.buf.delete_range(a, b);
            self.clear_selection();
            self.dirty = true;
        }
    }

    /// Replace the current selection with `text` (or insert at cursor if nothing selected).
    pub fn replace_selection_or_insert(&mut self, text: &str) {
        if self.selection_range().is_some() {
            self.delete_selection();
        }
        self.cursor = self.buf.insert_str(self.cursor, text);
        self.dirty = true;
        self.mark_redraw();
    }

    /// Update `scroll_x/scroll_y` so the cursor is visible within the current terminal size.
    ///
    /// This runs after cursor moves and after edits that might change line lengths.
    /// Marks redraw needed if scroll position changed.
    fn ensure_visible(&mut self) -> Result<()> {
        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;

        let prompt_lines = if self.prompt.is_some() { 1 } else { 0 };
        let status_lines = 1;
        let editor_h = height.saturating_sub(prompt_lines + status_lines);

        let old_scroll_y = self.scroll_y;
        let old_scroll_x = self.scroll_x;

        if self.word_wrap {
            // Calculate cursor screen row
            let lnw = max(2, digits(self.buf.line_count()));
            let gutter = lnw + 2;
            let avail = width.saturating_sub(gutter);
            
            let mut cursor_screen_row = 0;
            for (i, line) in self.buf.lines.iter().enumerate() {
                if i == self.cursor.y {
                    // Find segment
                    let mut current_col = 0;
                    let mut seg_idx = 0;
                    for (char_i, ch) in line.chars().enumerate() {
                        if char_i >= self.cursor.x { break; }
                        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
                        if current_col + ch_w > avail {
                            seg_idx += 1;
                            current_col = 0;
                        }
                        current_col += ch_w;
                    }
                    cursor_screen_row += seg_idx;
                    break;
                }
                
                // Add rows for this line
                if line.is_empty() {
                    cursor_screen_row += 1;
                } else {
                    let mut current_col = 0;
                    let mut rows = 1;
                    for ch in line.chars() {
                        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
                        if current_col + ch_w > avail {
                            rows += 1;
                            current_col = 0;
                        }
                        current_col += ch_w;
                    }
                    cursor_screen_row += rows;
                }
            }

            if cursor_screen_row < self.scroll_y {
                self.scroll_y = cursor_screen_row;
            } else if cursor_screen_row >= self.scroll_y + editor_h {
                self.scroll_y = cursor_screen_row.saturating_sub(editor_h.saturating_sub(1));
            }
            self.scroll_x = 0;
        } else {
            // vertical
            if self.cursor.y < self.scroll_y {
                self.scroll_y = self.cursor.y;
            } else if self.cursor.y >= self.scroll_y + editor_h {
                self.scroll_y = self.cursor.y.saturating_sub(editor_h.saturating_sub(1));
            }

            // horizontal scrolling
            let lnw = max(2, digits(self.buf.line_count()));
            let gutter = lnw + 2; // "NN│ "
            let avail = width.saturating_sub(gutter).saturating_sub(1);

            // Calculate the display column of the cursor (relative to start of line)
            let cursor_col_full = {
                let line = &self.buf.lines[self.cursor.y];
                let mut col = 0usize;
                for ch in line.chars().take(self.cursor.x) {
                    col += UnicodeWidthChar::width(ch).unwrap_or(1);
                }
                col
            };

            // Calculate the display column of the current scroll position
            let scroll_col_full = {
                let line = &self.buf.lines[self.cursor.y];
                let mut col = 0usize;
                for ch in line.chars().take(self.scroll_x) {
                    col += UnicodeWidthChar::width(ch).unwrap_or(1);
                }
                col
            };

            if cursor_col_full < scroll_col_full {
                // Scroll left: jump directly to cursor char index
                self.scroll_x = self.cursor.x;
            } else if cursor_col_full >= scroll_col_full + avail {
                // Scroll right: find a scroll_x such that cursor is at the right edge
                let line = &self.buf.lines[self.cursor.y];
                let mut col = 0usize;
                let mut new_scroll_x = 0;
                let target_col = cursor_col_full.saturating_sub(avail.saturating_sub(1));
                
                for (i, ch) in line.chars().enumerate() {
                    if col >= target_col {
                        new_scroll_x = i;
                        break;
                    }
                    col += UnicodeWidthChar::width(ch).unwrap_or(1);
                }
                self.scroll_x = new_scroll_x;
            }
        }

        // Mark redraw if scroll changed
        if old_scroll_y != self.scroll_y || old_scroll_x != self.scroll_x {
            self.mark_redraw();
        }

        Ok(())
    }

    /// Top-level mouse handler.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
        
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if shift && !self.word_wrap {
                    // Shift + Scroll Up = Scroll Left
                    let mut p = self.cursor;
                    p.x = p.x.saturating_sub(1);
                    self.cursor = self.buf.clamp_pos(p);
                    self.ensure_visible()?;
                } else {
                    // Normal Scroll Up = Move Cursor Up
                    let mut p = self.cursor;
                    p.y = p.y.saturating_sub(1);
                    p.x = min(p.x, self.buf.line_len_chars(p.y));
                    self.cursor = self.buf.clamp_pos(p);
                }
                self.clear_selection();
                self.ensure_visible()?;
                self.mark_redraw();
            }
            MouseEventKind::ScrollDown => {
                if shift && !self.word_wrap {
                    // Shift + Scroll Down = Scroll Right
                    let mut p = self.cursor;
                    p.x += 1;
                    self.cursor = self.buf.clamp_pos(p);
                    self.ensure_visible()?;
                } else {
                    // Normal Scroll Down = Move Cursor Down
                    let mut p = self.cursor;
                    if p.y + 1 < self.buf.line_count() {
                        p.y += 1;
                        p.x = min(p.x, self.buf.line_len_chars(p.y));
                    }
                    self.cursor = self.buf.clamp_pos(p);
                }
                self.clear_selection();
                self.ensure_visible()?;
                self.mark_redraw();
            }
            MouseEventKind::ScrollLeft => {
                if !self.word_wrap {
                    let mut p = self.cursor;
                    p.x = p.x.saturating_sub(1);
                    self.cursor = self.buf.clamp_pos(p);
                    self.clear_selection();
                    self.ensure_visible()?;
                    self.mark_redraw();
                }
            }
            MouseEventKind::ScrollRight => {
                if !self.word_wrap {
                    let mut p = self.cursor;
                    p.x += 1;
                    self.cursor = self.buf.clamp_pos(p);
                    self.clear_selection();
                    self.ensure_visible()?;
                    self.mark_redraw();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Top-level key handler.
    ///
    /// Returns `Ok(true)` if the editor should quit, `Ok(false)` otherwise.
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // If help or stats is shown, any key closes it
        if self.show_help || self.show_stats {
            self.show_help = false;
            self.show_stats = false;
            self.mark_redraw();
            return Ok(false);
        }

        // Prompt mode consumes keys first: when a prompt is open, most keys edit the prompt
        // input, not the document.
        if self.prompt.is_some() {
            return self.handle_prompt_key(key);
        }

        // Special selection movement with Shift+Arrows/Home/End
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Turn the raw key event into a canonical string like "Ctrl+S".
        // We use this string as the key into the command registry.
        let key_str = canonical_key_string(&key);

        // F1 toggles help
        if key.code == KeyCode::F(1) {
            self.show_help = true;
            self.mark_redraw();
            return Ok(false);
        }
        // F2 toggles stats
        if key.code == KeyCode::F(2) {
            self.show_stats = true;
            self.mark_redraw();
            return Ok(false);
        }

        // Movement keys (selection-aware)
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
            | KeyCode::PageUp | KeyCode::PageDown => {
                let selecting = shift;
                self.move_cursor(key, selecting)?;
                return Ok(false);
            }
            _ => {}
        }

        // If key matches a registered command (built-in or plugin), run it.
        // This makes plugin commands feel "first class": they work exactly like built-ins.
        if let Some(cmd_name) = self.commands.resolve_key(&key_str) {
            return Ok(self.run_command_by_name(&cmd_name)?);
        }

        // Common direct-edit keys. We also keep these as dedicated code paths even though
        // some are registered as commands, because it’s clearer for beginners to see them.
        match (key.code, ctrl) {
            (KeyCode::Char('q'), true) => return Ok(self.try_quit()),
            (KeyCode::Char('s'), true) => { self.cmd_save()?; return Ok(false); }
            (KeyCode::Char('o'), true) => { self.prompt = Some(Prompt::new(PromptKind::Open, "")); self.mark_redraw(); return Ok(false); }
            (KeyCode::Char('f'), true) => { self.prompt = Some(Prompt::new(PromptKind::Find, self.last_find.clone().unwrap_or_default())); self.mark_redraw(); return Ok(false); }
            (KeyCode::Char('p'), true) => { self.prompt = Some(Prompt::new(PromptKind::Command, "")); self.mark_redraw(); return Ok(false); }
            (KeyCode::Char('g'), true) => { self.prompt = Some(Prompt::new(PromptKind::GotoLine, "")); self.mark_redraw(); return Ok(false); }
            (KeyCode::Char('a'), true) => { self.select_all(); self.ensure_visible()?; return Ok(false); }
            (KeyCode::Char('z'), true) => { self.undo()?; return Ok(false); }
            (KeyCode::Char('y'), true) => { self.redo()?; return Ok(false); }
            (KeyCode::Char('c'), true) => { self.copy()?; return Ok(false); }
            (KeyCode::Char('x'), true) => { self.cut()?; return Ok(false); }
            (KeyCode::Char('v'), true) => { self.paste()?; return Ok(false); }
            _ => {}
        }

        match key.code {
            KeyCode::Esc => {
                self.clear_selection(); // clear_selection already marks redraw
            }
            KeyCode::Enter => {
                let op = EditOperation::Insert { pos: self.cursor, text: "\n".to_string() };
                self.record_edit(op);
                if self.selection_range().is_some() {
                    self.delete_selection(); // delete_selection already marks redraw
                } else {
                    self.mark_redraw();
                }
                self.cursor = self.buf.insert_newline(self.cursor);
                self.dirty = true;
                self.ensure_visible()?;
            }
            KeyCode::Backspace => {
                if let Some((a, b)) = self.selection_range() {
                    let deleted_text = self.buf.get_range(a, b);
                    let op = EditOperation::Delete { start: a, _end: b, deleted_text };
                    self.record_edit(op);
                    self.delete_selection();
                } else if self.cursor.y > 0 || self.cursor.x > 0 {
                    // Backspace a single character
                    let end = self.cursor;
                    let start = if self.cursor.x > 0 {
                        Pos { y: self.cursor.y, x: self.cursor.x - 1 }
                    } else {
                        let prev_y = self.cursor.y - 1;
                        Pos { y: prev_y, x: self.buf.line_len_chars(prev_y) }
                    };
                    let deleted_text = self.buf.get_range(start, end);
                    let op = EditOperation::Delete { start, _end: end, deleted_text };
                    self.record_edit(op);
                    self.cursor = self.buf.delete_backspace(self.cursor);
                    self.dirty = true;
                    self.mark_redraw();
                }
                self.ensure_visible()?;
            }
            KeyCode::Delete => {
                if let Some((a, b)) = self.selection_range() {
                    let deleted_text = self.buf.get_range(a, b);
                    let op = EditOperation::Delete { start: a, _end: b, deleted_text };
                    self.record_edit(op);
                    self.delete_selection();
                } else {
                    let start = self.cursor;
                    let end = if self.cursor.x < self.buf.line_len_chars(self.cursor.y) {
                        Pos { y: self.cursor.y, x: self.cursor.x + 1 }
                    } else if self.cursor.y + 1 < self.buf.line_count() {
                        Pos { y: self.cursor.y + 1, x: 0 }
                    } else {
                        start
                    };
                    
                    if start != end {
                        let deleted_text = self.buf.get_range(start, end);
                        let op = EditOperation::Delete { start, _end: end, deleted_text };
                        self.record_edit(op);
                        self.cursor = self.buf.delete_delete(self.cursor);
                        self.dirty = true;
                        self.mark_redraw();
                    }
                }
                self.ensure_visible()?;
            }
            KeyCode::Tab => {
                let op = EditOperation::Insert { pos: self.cursor, text: "    ".to_string() };
                self.record_edit(op);
                self.replace_selection_or_insert("    "); // replace_selection_or_insert already marks redraw
                self.ensure_visible()?;
            }
            KeyCode::Char(ch) => {
                // Text input (ignore control chars)
                if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
                    // ignore (handled above / keymap)
                } else {
                    let text = ch.to_string();
                    let op = EditOperation::Insert { pos: self.cursor, text: text.clone() };
                    self.record_edit(op);
                    self.replace_selection_or_insert(&text); // replace_selection_or_insert already marks redraw
                    self.ensure_visible()?;
                }
            }
            _ => {}
        }

        Ok(false)
    }

    /// Quit handling with a safety confirmation if there are unsaved changes.
    ///
    /// Behavior:
    /// - If not dirty: quit immediately
    /// - If dirty: require pressing Ctrl+Q twice within ~2 seconds
    fn try_quit(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        let now = Instant::now();
        if let Some(t) = self.last_quit_hint {
            if now.duration_since(t) <= Duration::from_secs(2) {
                return true;
            }
        }
        self.last_quit_hint = Some(now);
        self.set_status("Unsaved changes! Press Ctrl+Q again to quit.", Duration::from_secs(2));
        false
    }

    /// Move the cursor in response to a movement key.
    ///
    /// If `selecting` is true (Shift held), we create/extend a selection. Otherwise we clear
    /// the selection as we move.
    fn move_cursor(&mut self, key: KeyEvent, selecting: bool) -> Result<()> {
        if selecting && self.anchor.is_none() {
            self.anchor = Some(self.cursor);
            self.mark_redraw();
        }
        if !selecting {
            self.clear_selection(); // clear_selection already marks redraw
        }

        let (_w, h) = terminal::size()?;
        let height = h as usize;
        let prompt_lines = if self.prompt.is_some() { 1 } else { 0 };
        let status_lines = 1;
        let editor_h = height.saturating_sub(prompt_lines + status_lines);

        let mut p = self.cursor;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Left => {
                if ctrl {
                    p = self.move_to_prev_boundary(p);
                } else if p.x > 0 {
                    p.x -= 1;
                } else if p.y > 0 {
                    p.y -= 1;
                    p.x = self.buf.line_len_chars(p.y);
                }
            }
            KeyCode::Right => {
                if ctrl {
                    p = self.move_to_next_boundary(p);
                } else {
                    let len = self.buf.line_len_chars(p.y);
                    if p.x < len {
                        p.x += 1;
                    } else if p.y + 1 < self.buf.line_count() {
                        p.y += 1;
                        p.x = 0;
                    }
                }
            }
            KeyCode::Up => {
                if ctrl {
                    p = self.move_to_prev_line_boundary(p);
                } else if p.y > 0 {
                    p.y -= 1;
                    p.x = min(p.x, self.buf.line_len_chars(p.y));
                }
            }
            KeyCode::Down => {
                if ctrl {
                    p = self.move_to_next_line_boundary(p);
                } else if p.y + 1 < self.buf.line_count() {
                    p.y += 1;
                    p.x = min(p.x, self.buf.line_len_chars(p.y));
                }
            }
            KeyCode::Home => {
                p.y = 0;
                p.x = 0;
            }
            KeyCode::End => {
                p.y = self.buf.line_count().saturating_sub(1);
                p.x = self.buf.line_len_chars(p.y);
            }
            KeyCode::PageUp => {
                let jump = editor_h.saturating_sub(1);
                p.y = p.y.saturating_sub(jump);
                p.x = min(p.x, self.buf.line_len_chars(p.y));
            }
            KeyCode::PageDown => {
                let jump = editor_h.saturating_sub(1);
                p.y = min(p.y + jump, self.buf.line_count().saturating_sub(1));
                p.x = min(p.x, self.buf.line_len_chars(p.y));
            }
            _ => {}
        }

        let old_cursor = self.cursor;
        self.cursor = self.buf.clamp_pos(p);
        if old_cursor != self.cursor {
            self.mark_redraw();
        }
        self.ensure_visible()?;
        Ok(())
    }

    /// Helper to categorize a character for boundary detection.
    fn get_char_category(&self, ch: char) -> usize {
        if ch.is_whitespace() {
            0 // Whitespace
        } else if ch.is_alphanumeric() && ch != '_' && ch != '-' {
            1 // Word (excluding user-specified punctuation)
        } else {
            2 // Punctuation (including _ and - as requested)
        }
    }

    /// Move to the first boundary on the next line.
    fn move_to_next_line_boundary(&self, p: Pos) -> Pos {
        let line_count = self.buf.line_count();
        if p.y + 1 < line_count {
            let next_y = p.y + 1;
            let line = &self.buf.lines[next_y];
            let chars: Vec<char> = line.chars().collect();
            
            // Find first non-whitespace character on the next line
            let mut i = 0;
            while i < chars.len() && self.get_char_category(chars[i]) == 0 {
                i += 1;
            }
            return Pos { y: next_y, x: i };
        }
        // If on last line, move to end of document
        Pos { y: p.y, x: self.buf.line_len_chars(p.y) }
    }

    /// Move to the first boundary on the previous line.
    fn move_to_prev_line_boundary(&self, p: Pos) -> Pos {
        if p.y > 0 {
            let prev_y = p.y - 1;
            let line = &self.buf.lines[prev_y];
            let chars: Vec<char> = line.chars().collect();
            
            // Find first non-whitespace character on the previous line
            let mut i = 0;
            while i < chars.len() && self.get_char_category(chars[i]) == 0 {
                i += 1;
            }
            return Pos { y: prev_y, x: i };
        }
        // If on first line, move to start of document
        Pos { y: 0, x: 0 }
    }

    /// Move to the next boundary (word start or punctuation).
    fn move_to_next_boundary(&self, p: Pos) -> Pos {
        let line_count = self.buf.line_count();
        if p.y >= line_count { return p; }
        
        let line = &self.buf.lines[p.y];
        let chars: Vec<char> = line.chars().collect();
        
        if p.x >= chars.len() {
            if p.y + 1 < line_count {
                return Pos { y: p.y + 1, x: 0 };
            }
            return p;
        }

        let mut i = p.x;
        let start_cat = self.get_char_category(chars[i]);

        // 1. Move past the current character/cluster
        if start_cat == 1 {
            // If in a word, move to the end of the word
            while i < chars.len() && self.get_char_category(chars[i]) == 1 {
                i += 1;
            }
        } else {
            // If on punctuation or whitespace, just move past the single character
            i += 1;
        }

        // 2. Now skip any whitespace to find the BEGINNING of the next word/punctuation
        while i < chars.len() && self.get_char_category(chars[i]) == 0 {
            i += 1;
        }

        Pos { y: p.y, x: i }
    }

    /// Move to the previous boundary (word start or punctuation).
    fn move_to_prev_boundary(&self, p: Pos) -> Pos {
        if p.x == 0 {
            if p.y > 0 {
                let prev_y = p.y - 1;
                return Pos { y: prev_y, x: self.buf.line_len_chars(prev_y) };
            }
            return p;
        }

        let line = &self.buf.lines[p.y];
        let chars: Vec<char> = line.chars().collect();
        let mut i = p.x - 1;

        // 1. Skip any whitespace immediately to the left
        while i > 0 && self.get_char_category(chars[i]) == 0 {
            i -= 1;
        }

        // 2. If we land on a word character, find the START of that word
        if self.get_char_category(chars[i]) == 1 {
            while i > 0 && self.get_char_category(chars[i - 1]) == 1 {
                i -= 1;
            }
        }
        // 3. If it's punctuation, 'i' is already the start of that punctuation boundary

        Pos { y: p.y, x: i }
    }

    /// Copy the current selection to the system clipboard (Ctrl+C).
    ///
    /// If there is no selection, we do nothing and show a status message.
    fn copy(&mut self) -> Result<()> {
        let text = self.selected_text();
        if text.is_empty() {
            self.set_status("Nothing selected to copy.", Duration::from_secs(2));
            return Ok(());
        }
        if let Some(cb) = &mut self.clipboard {
            cb.set_text(text).ok();
            self.set_status("Copied selection.", Duration::from_secs(2));
        } else {
            self.set_status("Clipboard unavailable.", Duration::from_secs(2));
        }
        Ok(())
    }

    /// Cut the current selection to the system clipboard (Ctrl+X).
    ///
    /// Implementation: copy → delete selection (recording undo).
    fn cut(&mut self) -> Result<()> {
        let text = self.selected_text();
        if text.is_empty() {
            self.set_status("Nothing selected to cut.", Duration::from_secs(2));
            return Ok(());
        }
        
        let (a, b) = self.selection_range().unwrap();
        let op = EditOperation::Delete { start: a, _end: b, deleted_text: text.clone() };
        self.record_edit(op);

        if let Some(cb) = &mut self.clipboard {
            cb.set_text(text).ok();
        }
        self.delete_selection();
        self.ensure_visible()?;
        self.set_status("Cut selection.", Duration::from_secs(2));
        Ok(())
    }

    /// Paste from the system clipboard at the cursor (Ctrl+V).
    ///
    /// If there is a selection, it is replaced by the pasted content.
    fn paste(&mut self) -> Result<()> {
        if let Some(cb) = &mut self.clipboard {
            if let Ok(text) = cb.get_text() {
                if let Some((a, b)) = self.selection_range() {
                    let deleted_text = self.buf.get_range(a, b);
                    let op = EditOperation::Delete { start: a, _end: b, deleted_text };
                    self.record_edit(op);
                }
                
                let op = EditOperation::Insert { pos: self.cursor, text: text.clone() };
                self.record_edit(op);

                self.replace_selection_or_insert(&text);
                self.ensure_visible()?;
                self.set_status("Pasted.", Duration::from_secs(2));
                return Ok(());
            }
        }
        self.set_status("Clipboard unavailable.", Duration::from_secs(2));
        Ok(())
    }

    /// Save the buffer to the current file path (Ctrl+S).
    ///
    /// If the buffer has no path yet, this opens the “Save as” prompt instead.
    fn cmd_save(&mut self) -> Result<()> {
        if self.file_path.is_none() {
            self.prompt = Some(Prompt::new(PromptKind::SaveAs, ""));
            return Ok(());
        }
        self.save_to_path(self.file_path.clone().unwrap())
    }

    /// Save the buffer to a specific path.
    ///
    /// Also triggers the plugin `on_save` hook (best-effort).
    fn save_to_path(&mut self, path: PathBuf) -> Result<()> {
        let content = self.buf.to_string();
        fs::write(&path, content).with_context(|| format!("Failed writing {}", path.display()))?;
        self.file_path = Some(path.clone());
        self.dirty = false;
        self.set_status(format!("Saved: {}", path.display()), Duration::from_secs(2));

        // Allow plugins to react to saves (formatters, whitespace trimming, etc.).
        let mut plugins = mem::take(&mut self.plugins);
        plugins.call_hook(self, Hook::OnSave, Some(&path))?;
        self.plugins = plugins;
        Ok(())
    }

    /// Open a file from disk into the buffer.
    ///
    /// This resets cursor/selection/scroll and clears undo/redo history.
    fn open_path(&mut self, path: PathBuf) -> Result<()> {
        let s = fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        self.buf = Buffer::from_string(&s);
        self.cursor = Pos { y: 0, x: 0 };
        self.anchor = None;
        self.scroll_y = 0;
        self.scroll_x = 0;
        self.file_path = Some(path.clone());
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.ensure_visible()?;

        // Notify plugins that a file was opened.
        let mut plugins = mem::take(&mut self.plugins);
        plugins.call_hook(self, Hook::OnOpen, Some(&path))?;
        self.plugins = plugins;
        self.set_status(format!("Opened: {}", path.display()), Duration::from_secs(2));
        Ok(())
    }

    /// Find the next occurrence of `query` starting at the current cursor position.
    ///
    /// This is used by the Find prompt (Ctrl+F).
    fn find_next(&mut self, query: &str) -> Result<()> {
        if query.is_empty() {
            return Ok(());
        }
        self.last_find = Some(query.to_string());

        let start_pos = self.cursor;
        if let Some(p) = self.search_forward(query, start_pos, true) {
            self.cursor = p;
            self.clear_selection(); // clear_selection already marks redraw
            self.ensure_visible()?;
            self.set_status("Match found.", Duration::from_secs(1)); // set_status already marks redraw
        } else {
            self.set_status("No matches.", Duration::from_secs(2)); // set_status already marks redraw
        }
        Ok(())
    }

    /// Search forward for `query` starting at `from`.
    ///
    /// If `wrap` is true, we wrap around to the top after reaching EOF.
    fn search_forward(&self, query: &str, from: Pos, wrap: bool) -> Option<Pos> {
        // current line: search from current x
        let mut y = from.y;
        let mut x = from.x;

        // helper to search within a line starting at char x
        let find_in_line = |line: &str, start_char: usize| -> Option<usize> {
            let b0 = char_to_byte_index(line, start_char);
            let sub = &line[b0..];
            let idx = sub.find(query)?;
            Some(start_char + byte_to_char_index(sub, idx))
        };

        // from current line to end
        while y < self.buf.line_count() {
            let line = &self.buf.lines[y];
            if let Some(cx) = find_in_line(line, x) {
                return Some(Pos { y, x: cx });
            }
            y += 1;
            x = 0;
        }

        if !wrap {
            return None;
        }

        // wrap: from top to original line
        y = 0;
        while y <= from.y && y < self.buf.line_count() {
            let line = &self.buf.lines[y];
            let start = if y == from.y { 0 } else { 0 };
            if let Some(cx) = find_in_line(line, start) {
                return Some(Pos { y, x: cx });
            }
            y += 1;
        }
        None
    }

    /// Handle keys while a prompt is active.
    ///
    /// In prompt mode, most keys edit the prompt input string (not the main buffer).
    /// `Enter` “submits” the prompt and performs the prompt action.
    fn handle_prompt_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(prompt) = &mut self.prompt else { return Ok(false); };

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match (key.code, ctrl) {
            (KeyCode::Esc, _) => {
                self.prompt = None;
                self.mark_redraw();
                return Ok(false);
            }
            (KeyCode::Enter, _) => {
                let kind = prompt.kind;
                let input = prompt.input.clone();
                self.prompt = None;
                self.mark_redraw();

                match kind {
                    PromptKind::Open => {
                        // Open prompt: treat input as a path relative to the current directory.
                        let p = PathBuf::from(input.trim());
                        if p.as_os_str().is_empty() {
                            return Ok(false);
                        }
                        self.open_path(p)?;
                    }
                    PromptKind::SaveAs => {
                        // Save-as prompt: set a new path and write the buffer there.
                        let p = PathBuf::from(input.trim());
                        if p.as_os_str().is_empty() {
                            return Ok(false);
                        }
                        self.save_to_path(p)?;
                    }
                    PromptKind::Find => {
                        // Find prompt: find the next match starting from the cursor.
                        self.find_next(input.trim())?;
                    }
                            PromptKind::GotoLine => {
                                // Goto prompt: parse a 1-based line number and clamp it into valid range.
                                let n: isize = input.trim().parse().unwrap_or(1);
                                let target = clamp_usize(n - 1, 0, self.buf.line_count().saturating_sub(1));
                                self.cursor.y = target;
                                self.cursor.x = min(self.cursor.x, self.buf.line_len_chars(self.cursor.y));
                                self.clear_selection(); // clear_selection already marks redraw
                                self.ensure_visible()?;
                            }
                    PromptKind::Command => {
                        let cmdline = input.trim();
                        if cmdline.is_empty() {
                            return Ok(false);
                        }
                        // Command prompt supports Vim-like shorthands (":w", ":q", ":wq").
                        let cmd = cmdline.trim_start_matches(':');
                        let cmd = match cmd {
                            "w" => "save",
                            "q" => "quit",
                            "wq" => "save_and_quit",
                            other => other,
                        };
                        let should_quit = self.run_command_by_name(cmd)?;
                        if should_quit {
                            return Ok(true);
                        }
                    }
                }
                return Ok(false);
            }
            (KeyCode::Backspace, _) => {
                if prompt.cursor > 0 {
                    let mut chars: Vec<char> = prompt.input.chars().collect();
                    chars.remove(prompt.cursor - 1);
                    prompt.input = chars.into_iter().collect();
                    prompt.cursor -= 1;
                    self.mark_redraw();
                }
            }
            (KeyCode::Delete, _) => {
                let len = prompt.input.chars().count();
                if prompt.cursor < len {
                    let mut chars: Vec<char> = prompt.input.chars().collect();
                    chars.remove(prompt.cursor);
                    prompt.input = chars.into_iter().collect();
                    self.mark_redraw();
                }
            }
            (KeyCode::Left, _) => {
                prompt.cursor = prompt.cursor.saturating_sub(1);
                self.mark_redraw();
            }
            (KeyCode::Right, _) => {
                let len = prompt.input.chars().count();
                prompt.cursor = min(prompt.cursor + 1, len);
                self.mark_redraw();
            }
            (KeyCode::Home, _) => {
                prompt.cursor = 0;
                self.mark_redraw();
            }
            (KeyCode::End, _) => {
                prompt.cursor = prompt.input.chars().count();
                self.mark_redraw();
            }
            (KeyCode::Char(ch), true) if ch == 'u' => {
                // Ctrl+U clears prompt line (handy in shells)
                prompt.input.clear();
                prompt.cursor = 0;
                self.mark_redraw();
            }
            (KeyCode::Char(ch), _) => {
                if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) {
                    // ignore
                } else {
                    let mut chars: Vec<char> = prompt.input.chars().collect();
                    chars.insert(prompt.cursor, ch);
                    prompt.input = chars.into_iter().collect();
                    prompt.cursor += 1;
                    self.mark_redraw();
                }
            }
            _ => {}
        }

        Ok(false)
    }

    /// Run a command by name (built-in or plugin).
    ///
    /// Returns whether this command requested quitting the editor.
    fn run_command_by_name(&mut self, name: &str) -> Result<bool> {
        let name = name.trim();
        if name.eq_ignore_ascii_case("quit") {
            return Ok(self.try_quit());
        }
        if name.eq_ignore_ascii_case("save_and_quit") {
            self.cmd_save()?;
            return Ok(self.try_quit());
        }

        let cmd_opt = self.commands.get(name).cloned();

        if let Some(cmd) = cmd_opt {
            match cmd.source {
                CommandSource::Builtin(f) => {
                    f(self)?;
                }
                CommandSource::Plugin { plugin_id, func } => {
                    // Plugin commands can edit the buffer, so we take an undo snapshot before running.
                    // Note: Plugins currently aren't descriptive about their edits, 
                    // so we still snapshot the whole buffer for them.
                    // This is a future optimization area.
                    let mut plugins = mem::take(&mut self.plugins);
                    let res = plugins.run_command(self, &plugin_id, &func);
                    self.plugins = plugins;
                    res?;
                    self.ensure_visible()?;
                }
            }
            self.mark_redraw();
            Ok(false)
        } else {
            // Unknown command - handle gracefully
            let mut msg = format!("Unknown command: '{}'", name);
            if let Some(suggestion) = self.commands.suggest_command(name) {
                msg.push_str(&format!(". Did you mean '{}'?", suggestion.name));
            }
            self.set_status(msg, Duration::from_secs(3));
            self.mark_redraw();
            Ok(false)
        }
    }

    /// Render the entire UI.
    ///
    /// This editor uses a simple "full redraw" strategy: clear and redraw each frame.
    /// It's not the most efficient approach, but it's straightforward and reliable.
    ///
    /// Only renders if `needs_redraw` is true, to avoid unnecessary flickering.
    pub fn render(&mut self, stdout: &mut Stdout) -> Result<()> {
        if !self.needs_redraw {
            return Ok(());
        }
        self.needs_redraw = false;

        if self.show_help {
            return self.render_help(stdout);
        }
        if self.show_stats {
            return self.render_stats(stdout);
        }

        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;

        let lnw = max(2, digits(self.buf.line_count()));
        let gutter = lnw + 2; // "NN│"

        let has_prompt = self.prompt.is_some();
        let editor_h = height.saturating_sub(1 + if has_prompt { 1 } else { 0 });
        let prompt_y = if has_prompt { editor_h } else { 0 };
        let status_y = height.saturating_sub(1);

        stdout.queue(cursor::Hide)?;
        stdout.queue(style::ResetColor)?;
        // REMOVE: stdout.queue(terminal::Clear(ClearType::All))?;
        // We now clear lines incrementally to eliminate flickering.

        // ===== Editor lines =====
        // We render a viewport of lines.
        let mut rows_rendered = 0;
        
        if self.word_wrap {
            // Word wrap mode: 1 buffer line can span multiple screen rows.
            // scroll_y now represents the index of the SCREEN ROW to start rendering from.
            let avail = width.saturating_sub(gutter);
            
            let mut current_screen_row = 0;
            for line_idx in 0..self.buf.line_count() {
                let line = &self.buf.lines[line_idx];
                
                // Wrap this line into segments
                let mut segments = Vec::new();
                if line.is_empty() {
                    segments.push(0);
                } else {
                    let mut current_col = 0;
                    let mut start_idx = 0;
                    for (i, ch) in line.chars().enumerate() {
                        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
                        if current_col + ch_w > avail {
                            segments.push(start_idx);
                            start_idx = i;
                            current_col = 0;
                        }
                        current_col += ch_w;
                    }
                    segments.push(start_idx);
                }

                for (seg_idx, &start_char_idx) in segments.iter().enumerate() {
                    if current_screen_row >= self.scroll_y && rows_rendered < editor_h {
                        let screen_row = rows_rendered;
                        stdout.queue(cursor::MoveTo(0, screen_row as u16))?;
                        stdout.queue(terminal::Clear(ClearType::CurrentLine))?;

                        let is_current_line = line_idx == self.cursor.y;
                        let base_bg = if is_current_line { Some(Color::DarkBlue) } else { None };

                        // Gutter
                        if let Some(bg) = base_bg {
                            stdout.queue(style::SetBackgroundColor(bg))?;
                        }
                        stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                        if seg_idx == 0 {
                            stdout.queue(style::Print(format!("{:>width$}", line_idx + 1, width = lnw)))?;
                        } else {
                            stdout.queue(style::Print(" ".repeat(lnw)))?;
                        }
                        stdout.queue(style::Print("│ "))?;
                        stdout.queue(style::ResetColor)?;

                        // Content
                        let chars: Vec<char> = line.chars().skip(start_char_idx).collect();
                        let mut col_used = 0;
                        let mut seg_char_i = start_char_idx;
                        let mut seg_text = String::new();
                        let mut seg_selected: Option<bool> = None;
                        let sel = self.selection_range();

                        let flush_seg = |stdout: &mut Stdout,
                                         seg: &mut String,
                                         seg_selected: &mut Option<bool>,
                                         base_bg: Option<Color>|
                         -> Result<()> {
                            if seg.is_empty() { return Ok(()); }
                            let selected = seg_selected.unwrap_or(false);
                            if selected {
                                stdout.queue(style::SetForegroundColor(Color::Black))?;
                                stdout.queue(style::SetBackgroundColor(Color::Grey))?;
                                stdout.queue(style::SetAttribute(Attribute::Bold))?;
                            } else {
                                if let Some(bg) = base_bg { stdout.queue(style::SetBackgroundColor(bg))?; }
                                stdout.queue(style::SetForegroundColor(Color::Reset))?;
                                stdout.queue(style::SetAttribute(Attribute::Reset))?;
                            }
                            stdout.queue(style::Print(seg.as_str()))?;
                            stdout.queue(style::ResetColor)?;
                            seg.clear();
                            *seg_selected = None;
                            Ok(())
                        };

                        for ch in chars {
                            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
                            if col_used + ch_w > avail { break; }

                            let selected = if let Some((a, b)) = sel {
                                if line_idx < a.y || line_idx > b.y { false }
                                else if line_idx == a.y && line_idx == b.y { (seg_char_i >= a.x) && (seg_char_i < b.x) }
                                else if line_idx == a.y { seg_char_i >= a.x }
                                else if line_idx == b.y { seg_char_i < b.x }
                                else { true }
                            } else { false };

                            if seg_selected.is_none() {
                                seg_selected = Some(selected);
                            } else if seg_selected.unwrap() != selected {
                                flush_seg(stdout, &mut seg_text, &mut seg_selected, base_bg)?;
                                seg_selected = Some(selected);
                            }
                            seg_text.push(ch);
                            col_used += ch_w;
                            seg_char_i += 1;
                        }
                        flush_seg(stdout, &mut seg_text, &mut seg_selected, base_bg)?;

                        if is_current_line && col_used < avail {
                            stdout.queue(style::SetBackgroundColor(Color::DarkBlue))?;
                            stdout.queue(style::Print(" ".repeat(avail - col_used)))?;
                            stdout.queue(style::ResetColor)?;
                        }

                        // Render scroll indicator on the far right
                        let total_lines = self.buf.line_count();
                        let thumb_size = max(1, (editor_h * editor_h) / max(1, total_lines));
                        let thumb_start = (line_idx * editor_h) / max(1, total_lines);
                        let thumb_end = thumb_start + thumb_size;

                        stdout.queue(cursor::MoveTo((width - 1) as u16, screen_row as u16))?;
                        if screen_row >= thumb_start && screen_row < thumb_end {
                            stdout.queue(style::SetForegroundColor(Color::White))?;
                            stdout.queue(style::Print("█"))?;
                        } else {
                            stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                            stdout.queue(style::Print("│"))?;
                        }
                        stdout.queue(style::ResetColor)?;

                        rows_rendered += 1;
                    }
                    current_screen_row += 1;
                }
                if rows_rendered >= editor_h { break; }
            }
        } else {
            // Normal mode: 1 buffer line = 1 screen row
            for row in 0..editor_h {
                let y = self.scroll_y + row;
                stdout.queue(cursor::MoveTo(0, row as u16))?;
                stdout.queue(terminal::Clear(ClearType::CurrentLine))?;

                if y >= self.buf.line_count() {
                    // tildes
                    stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                    stdout.queue(style::Print("~"))?;
                    stdout.queue(style::ResetColor)?;
                    rows_rendered += 1;
                    continue;
                }

                let is_current_line = y == self.cursor.y;
                let base_bg = if is_current_line { Some(Color::DarkBlue) } else { None };

                // Line number gutter (helps orientation when scrolling).
                if let Some(bg) = base_bg {
                    stdout.queue(style::SetBackgroundColor(bg))?;
                }
                stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                stdout.queue(style::Print(format!("{:>width$}", y + 1, width = lnw)))?;
                stdout.queue(style::Print("│ "))?;
                stdout.queue(style::ResetColor)?;

                // line content with selection highlighting
                let line = &self.buf.lines[y];
                let avail = width.saturating_sub(gutter);

                let sel = self.selection_range();
                let mut col_used = 0usize;
                let mut char_i = self.scroll_x;

                // skip scroll_x chars
                let mut chars: Vec<char> = line.chars().collect();
                if self.scroll_x < chars.len() {
                    chars = chars[self.scroll_x..].to_vec();
                } else {
                    chars.clear();
                }

                // render segments
                let mut seg = String::new();
                let mut seg_selected: Option<bool> = None;

                let flush_seg = |stdout: &mut Stdout,
                                 seg: &mut String,
                                 seg_selected: &mut Option<bool>,
                                 base_bg: Option<Color>|
                 -> Result<()> {
                    if seg.is_empty() {
                        return Ok(());
                    }
                    let selected = seg_selected.unwrap_or(false);

                    if selected {
                        stdout.queue(style::SetForegroundColor(Color::Black))?;
                        stdout.queue(style::SetBackgroundColor(Color::Grey))?;
                        stdout.queue(style::SetAttribute(Attribute::Bold))?;
                    } else {
                        if let Some(bg) = base_bg {
                            stdout.queue(style::SetBackgroundColor(bg))?;
                        }
                        stdout.queue(style::SetForegroundColor(Color::Reset))?;
                        stdout.queue(style::SetAttribute(Attribute::Reset))?;
                    }

                    stdout.queue(style::Print(seg.as_str()))?;
                    stdout.queue(style::ResetColor)?;
                    seg.clear();
                    *seg_selected = None;
                    Ok(())
                };

                for ch in chars {
                    let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
                    if col_used + ch_w > avail {
                        break;
                    }

                    let selected = if let Some((a, b)) = sel {
                        if y < a.y || y > b.y {
                            false
                        } else if y == a.y && y == b.y {
                            (char_i >= a.x) && (char_i < b.x)
                        } else if y == a.y {
                            char_i >= a.x
                        } else if y == b.y {
                            char_i < b.x
                        } else {
                            true
                        }
                    } else {
                        false
                    };

                    if seg_selected.is_none() {
                        seg_selected = Some(selected);
                    } else if seg_selected.unwrap() != selected {
                        flush_seg(stdout, &mut seg, &mut seg_selected, base_bg)?;
                        seg_selected = Some(selected);
                    }

                    seg.push(ch);
                    col_used += ch_w;
                    char_i += 1;
                }

                flush_seg(stdout, &mut seg, &mut seg_selected, base_bg)?;

                // fill remainder of line with base_bg if current line
                if is_current_line && col_used < avail {
                    stdout.queue(style::SetBackgroundColor(Color::DarkBlue))?;
                    stdout.queue(style::Print(" ".repeat(avail - col_used)))?;
                    stdout.queue(style::ResetColor)?;
                }

                // Render scroll indicator on the far right
                let total_lines = self.buf.line_count();
                
                // Calculate where the "thumb" should be
                let thumb_size = max(1, (editor_h * editor_h) / max(1, total_lines));
                let thumb_start = (self.scroll_y * editor_h) / max(1, total_lines);
                let thumb_end = thumb_start + thumb_size;

                stdout.queue(cursor::MoveTo((width - 1) as u16, row as u16))?;
                if row >= thumb_start && row < thumb_end {
                    stdout.queue(style::SetForegroundColor(Color::White))?;
                    stdout.queue(style::Print("█"))?;
                } else {
                    stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                    stdout.queue(style::Print("│"))?;
                }
                stdout.queue(style::ResetColor)?;

                rows_rendered += 1;
            }
        }

        // Draw tildes for remaining empty rows
        for row in rows_rendered..editor_h {
            stdout.queue(cursor::MoveTo(0, row as u16))?;
            stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
            stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
            stdout.queue(style::Print("~"))?;
            stdout.queue(style::ResetColor)?;
        }

            // Prompt line
            if let Some(p) = &self.prompt {
                // If it's a command prompt, show matching commands as a "floating" list above the prompt
                if p.kind == PromptKind::Command {
                    let hits = self.commands.search(p.input.trim(), 10);
                    if !hits.is_empty() {
                        let list_h = hits.len();
                        let start_y = prompt_y.saturating_sub(list_h);
                        
                        for (i, cmd) in hits.iter().enumerate() {
                            let row = start_y + i;
                            if row >= editor_h { continue; }
                            
                            stdout.queue(cursor::MoveTo(0, row as u16))?;
                            stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
                            
                            // Draw a nice suggestion line
                            stdout.queue(style::SetBackgroundColor(Color::AnsiValue(235)))?; // Dark grey
                            stdout.queue(style::SetForegroundColor(Color::Yellow))?;
                            stdout.queue(style::Print(format!("  {:15}", cmd.name)))?;
                            stdout.queue(style::SetForegroundColor(Color::White))?;
                            stdout.queue(style::Print(format!(" │ {:30}", cmd.description)))?;
                            
                            if let Some(key) = &cmd.key {
                                stdout.queue(style::SetForegroundColor(Color::Grey))?;
                                stdout.queue(style::Print(format!(" ({})", key)))?;
                            }
                            
                            // Fill rest of line
                            let used = 2 + 15 + 3 + 30 + cmd.key.as_ref().map(|k| k.len() + 3).unwrap_or(0);
                            if used < width {
                                stdout.queue(style::Print(" ".repeat(width - used)))?;
                            }
                            stdout.queue(style::ResetColor)?;
                        }
                    }
                }

                stdout.queue(cursor::MoveTo(0, prompt_y as u16))?;
                stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
                stdout.queue(style::SetForegroundColor(Color::Yellow))?;

                let label = match p.kind {
                    PromptKind::Open => "Open: ",
                    PromptKind::SaveAs => "Save as: ",
                    PromptKind::Find => "Find: ",
                    PromptKind::Command => "Command: ",
                    PromptKind::GotoLine => "Goto line: ",
                };

                stdout.queue(style::Print(label))?;
                stdout.queue(style::ResetColor)?;
                stdout.queue(style::Print(&p.input))?;
            }

        // Status bar
        stdout.queue(cursor::MoveTo(0, status_y as u16))?;
        stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
        stdout.queue(style::SetForegroundColor(Color::Black))?;
        stdout.queue(style::SetBackgroundColor(Color::White))?;

        let path_str = self
            .file_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<new file>".to_string());

        let sel_info = if let Some((a, b)) = self.selection_range() {
            format!("SEL {}:{}-{}:{}", a.y + 1, a.x + 1, b.y + 1, b.x + 1)
        } else {
            " ".to_string()
        };

        let dirty = if self.dirty { "*" } else { " " };

        let msg = self.status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        let wrap_info = if self.word_wrap { "[WRAP]" } else { "" };
        let left = format!(" {}{} {} {}  Ln {}, Col {}  {} ",
            dirty,
            "",
            path_str,
            wrap_info,
            self.cursor.y + 1,
            self.cursor.x + 1,
            sel_info,
        );

        let mut bar = left;
        if !msg.is_empty() {
            bar.push_str(" | ");
            bar.push_str(&msg);
        }

        // pad or truncate
        if bar.chars().count() < width {
            bar.push_str(&" ".repeat(width - bar.chars().count()));
        } else {
            bar = bar.chars().take(width).collect();
        }

        stdout.queue(style::Print(bar))?;
        stdout.queue(style::ResetColor)?;

        // ===== Place the terminal cursor =====
        let mut cursor_x = gutter;
        let mut cursor_y = 0;

        if self.word_wrap {
            // Find which screen row and column the cursor is on
            let avail = width.saturating_sub(gutter);
            let mut current_screen_row = 0;
            let mut found = false;

            for line_idx in 0..self.buf.line_count() {
                let line = &self.buf.lines[line_idx];
                let mut segments = Vec::new();
                if line.is_empty() {
                    segments.push(0);
                } else {
                    let mut current_col = 0;
                    let mut start_idx = 0;
                    for (i, ch) in line.chars().enumerate() {
                        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
                        if current_col + ch_w > avail {
                            segments.push(start_idx);
                            start_idx = i;
                            current_col = 0;
                        }
                        current_col += ch_w;
                    }
                    segments.push(start_idx);
                }

                if line_idx == self.cursor.y {
                    // This is the line with the cursor. Find the segment.
                    let mut seg_idx = 0;
                    for (i, &start) in segments.iter().enumerate() {
                        if self.cursor.x >= start {
                            seg_idx = i;
                        } else {
                            break;
                        }
                    }

                    cursor_y = (current_screen_row + seg_idx).saturating_sub(self.scroll_y);
                    
                    let start_char = segments[seg_idx];
                    let mut col = 0;
                    for ch in line.chars().skip(start_char).take(self.cursor.x - start_char) {
                        col += UnicodeWidthChar::width(ch).unwrap_or(1);
                    }
                    cursor_x = gutter + col;
                    found = true;
                    break;
                }
                current_screen_row += segments.len();
            }
            if !found {
                // Fallback (shouldn't happen)
                cursor_y = 0;
                cursor_x = gutter;
            }
        } else {
            // Normal mode
            let cursor_row = self.cursor.y.saturating_sub(self.scroll_y);
            let cursor_col = {
                let line = &self.buf.lines[self.cursor.y];
                let start = self.scroll_x;
                let end = self.cursor.x;

                let mut col = 0usize;
                let mut idx = 0usize;
                for (i, ch) in line.chars().enumerate() {
                    if i < start { continue; }
                    if i >= end { break; }
                    if idx >= (end - start) { break; }
                    col += UnicodeWidthChar::width(ch).unwrap_or(1);
                    idx += 1;
                }
                col
            };
            cursor_x = gutter + cursor_col;
            cursor_y = cursor_row;
        }

        let final_x = cursor_x.min(width.saturating_sub(1));
        let final_y = cursor_y.min(editor_h.saturating_sub(1));

        stdout.queue(cursor::MoveTo(final_x as u16, final_y as u16))?;
        stdout.queue(cursor::Show)?;
        stdout.flush()?;
        Ok(())
    }

    /// Calculate various document statistics.
    fn calculate_stats(&self) -> DocumentStats {
        let mut word_count = 0;
        let mut char_count = 0;
        let mut byte_count = 0;
        let mut histogram = vec![0; 10]; // 10 buckets for line length distribution

        for line in &self.buf.lines {
            char_count += line.chars().count();
            byte_count += line.len();
            word_count += line.split_whitespace().count();
            
            // Populate histogram (bucketed by 10s up to 90, then 90+)
            let bucket = (line.chars().count() / 10).min(9);
            histogram[bucket] += 1;
        }

        // Add line ending bytes to the total
        let le_len = self.buf.line_ending.as_str().len();
        if self.buf.line_count() > 1 {
            byte_count += (self.buf.line_count() - 1) * le_len;
        }

        DocumentStats {
            line_count: self.buf.line_count(),
            word_count,
            char_count,
            byte_count,
            line_ending: self.buf.line_ending,
            encoding: "UTF-8 (Unicode)",
            line_length_histogram: histogram,
        }
    }

    /// Render the Document Statistics screen.
    fn render_stats(&mut self, stdout: &mut Stdout) -> Result<()> {
        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;
        let stats = self.calculate_stats();

        stdout.queue(cursor::Hide)?;
        stdout.queue(style::SetBackgroundColor(Color::DarkMagenta))?;
        stdout.queue(style::SetForegroundColor(Color::White))?;
        stdout.queue(terminal::Clear(ClearType::All))?;

        let mut lines = vec![
            " DOCUMENT STATISTICS ".to_string(),
            "=====================".to_string(),
            "".to_string(),
            format!("  Lines:      {}", stats.line_count),
            format!("  Words:      {}", stats.word_count),
            format!("  Characters: {}", stats.char_count),
            format!("  File Size:  {} bytes", stats.byte_count),
            format!("  End of Line: {} ({})", stats.line_ending.name(), stats.line_ending.as_str().escape_debug()),
            format!("  Encoding:   {}", stats.encoding),
            "".to_string(),
            " LINE LENGTH DISTRIBUTION: ".to_string(),
        ];

        // Add a small bar chart
        let max_val = *stats.line_length_histogram.iter().max().unwrap_or(&1).max(&1);
        let chart_width = 30;
        for (i, &count) in stats.line_length_histogram.iter().enumerate() {
            let label = if i == 9 { "90+ ".to_string() } else { format!("{:>2}-{} ", i * 10, (i + 1) * 10 - 1) };
            let bar_len = (count * chart_width) / max_val;
            let bar = "█".repeat(bar_len);
            lines.push(format!("  {} {:<30} ({})", label, bar, count));
        }

        lines.push("".to_string());
        lines.push(" Press any key to close... ".to_string());

        let start_y = (height.saturating_sub(lines.len())) / 2;
        for (i, line) in lines.iter().enumerate() {
            let x = (width.saturating_sub(line.chars().count())) / 2;
            stdout.queue(cursor::MoveTo(x as u16, (start_y + i) as u16))?;
            stdout.queue(style::Print(line))?;
        }

        stdout.flush()?;
        Ok(())
    }

    /// Render the Help screen overlay.
    fn render_help(&mut self, stdout: &mut Stdout) -> Result<()> {
        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;

        stdout.queue(cursor::Hide)?;
        stdout.queue(style::SetBackgroundColor(Color::DarkBlue))?;
        stdout.queue(style::SetForegroundColor(Color::White))?;
        stdout.queue(terminal::Clear(ClearType::All))?;

        let help_text = vec![
            " KPAD HELP — Keybindings and Modifiers ",
            "========================================",
            "",
            " NAVIGATION:",
            "  Arrows          Move cursor by 1 character / 1 line",
            "  Ctrl + Left     Jump to previous word or punctuation",
            "  Ctrl + Right    Jump to next word or punctuation",
            "  Home / End      Jump to top / bottom of document",
            "  PageUp / Down   Move up / down one full screen",
            "",
            " SELECTION:",
            "  Shift + Arrows  Select text while moving",
            "  Ctrl + A        Select All",
            "",
            " EDITING:",
            "  Ctrl + S        Save file",
            "  Ctrl + O        Open file",
            "  Ctrl + Z        Undo",
            "  Ctrl + Y        Redo",
            "  Ctrl + C / X    Copy / Cut selection",
            "  Ctrl + V        Paste",
            "  Tab             Insert 4 spaces",
            "",
            " SYSTEM:",
            "  Ctrl + P        Command Palette (Discovery)",
            "  Alt + W         Toggle Word Wrap",
            "  Ctrl + Q        Quit (asks if unsaved)",
            "  F1 / :help      Toggle this Help screen",
            "",
            " Press any key to close help...",
        ];

        let start_y = (height.saturating_sub(help_text.len())) / 2;
        for (i, line) in help_text.iter().enumerate() {
            let x = (width.saturating_sub(line.len())) / 2;
            stdout.queue(cursor::MoveTo(x as u16, (start_y + i) as u16))?;
            stdout.queue(style::Print(line))?;
        }

        stdout.flush()?;
        Ok(())
    }
}


fn register_builtin_commands(reg: &mut CommandRegistry) {
    // These are visible to Ctrl+P / command prompt too.
    reg.register(Command {
        name: "save".to_string(),
        description: "Save file (Ctrl+S)".to_string(),
        key: Some("Ctrl+S".to_string()),
        source: CommandSource::Builtin(|ed| ed.cmd_save()),
    });
    reg.register(Command {
        name: "open".to_string(),
        description: "Open file (Ctrl+O)".to_string(),
        key: Some("Ctrl+O".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(PromptKind::Open, ""));
            Ok(())
        }),
    });
    reg.register(Command {
        name: "find".to_string(),
        description: "Find (Ctrl+F)".to_string(),
        key: Some("Ctrl+F".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(
                PromptKind::Find,
                ed.last_find.clone().unwrap_or_default(),
            ));
            Ok(())
        }),
    });
           reg.register(Command {
               name: "stats".to_string(),
               description: "Show document statistics (F2)".to_string(),
               key: Some("F2".to_string()),
               source: CommandSource::Builtin(|ed| {
                   ed.show_stats = true;
                   ed.mark_redraw();
                   Ok(())
               }),
           });
           reg.register(Command {
               name: "eol".to_string(),
               description: "Toggle line endings (LF/CRLF)".to_string(),
               key: None,
               source: CommandSource::Builtin(|ed| {
                   ed.toggle_line_ending();
                   Ok(())
               }),
           });
           reg.register(Command {
               name: "help".to_string(),
               description: "Show help screen (F1)".to_string(),
               key: Some("F1".to_string()),
               source: CommandSource::Builtin(|ed| {
                   ed.show_help = true;
                   ed.mark_redraw();
                   Ok(())
               }),
           });
           reg.register(Command {
               name: "command".to_string(),
               description: "Command prompt / palette (Ctrl+P)".to_string(),
               key: Some("Ctrl+P".to_string()),
               source: CommandSource::Builtin(|ed| {
                   ed.prompt = Some(Prompt::new(PromptKind::Command, ""));
                   ed.mark_redraw();
                   Ok(())
               }),
           });
           reg.register(Command {
               name: "goto_line".to_string(),
               description: "Go to line (Ctrl+G)".to_string(),
               key: Some("Ctrl+G".to_string()),
               source: CommandSource::Builtin(|ed| {
                   ed.prompt = Some(Prompt::new(PromptKind::GotoLine, ""));
                   ed.mark_redraw();
                   Ok(())
               }),
           });
    reg.register(Command {
        name: "undo".to_string(),
        description: "Undo (Ctrl+Z)".to_string(),
        key: Some("Ctrl+Z".to_string()),
        source: CommandSource::Builtin(|ed| ed.undo()),
    });
    reg.register(Command {
        name: "redo".to_string(),
        description: "Redo (Ctrl+Y)".to_string(),
        key: Some("Ctrl+Y".to_string()),
        source: CommandSource::Builtin(|ed| ed.redo()),
    });
           reg.register(Command {
               name: "quit".to_string(),
               description: "Quit (Ctrl+Q)".to_string(),
               key: Some("Ctrl+Q".to_string()),
               source: CommandSource::Builtin(|ed| {
                   // handled specially
                   let _ = ed;
                   Ok(())
               }),
           });
           reg.register(Command {
               name: "copy".to_string(),
               description: "Copy selection (Ctrl+C)".to_string(),
               key: Some("Ctrl+C".to_string()),
               source: CommandSource::Builtin(|ed| ed.copy()),
           });
           reg.register(Command {
               name: "cut".to_string(),
               description: "Cut selection (Ctrl+X)".to_string(),
               key: Some("Ctrl+X".to_string()),
               source: CommandSource::Builtin(|ed| ed.cut()),
           });
           reg.register(Command {
               name: "paste".to_string(),
               description: "Paste clipboard (Ctrl+V)".to_string(),
               key: Some("Ctrl+V".to_string()),
               source: CommandSource::Builtin(|ed| ed.paste()),
           });
           reg.register(Command {
               name: "select_all".to_string(),
               description: "Select entire buffer (Ctrl+A)".to_string(),
               key: Some("Ctrl+A".to_string()),
               source: CommandSource::Builtin(|ed| {
                   ed.select_all();
                   ed.ensure_visible()?;
                   Ok(())
               }),
           });
           reg.register(Command {
               name: "wrap".to_string(),
               description: "Toggle word wrapping".to_string(),
               key: Some("Alt+W".to_string()),
               source: CommandSource::Builtin(|ed| {
                   ed.toggle_word_wrap();
                   Ok(())
               }),
           });
}
