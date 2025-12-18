//! Editor: the main application state and all editing operations.

mod builtin_commands;
mod clipboard;
mod file_ops;
mod input;
mod movement;
mod render;
mod screens;
mod undo;

use crate::buffer::Buffer;
use crate::commands::{CommandRegistry, CommandSource};
use crate::plugins::{Hook, PluginManager};
use crate::types::{LineEnding, Pos, Prompt, StatusMsg, UndoEntry};
use crate::utils::{char_to_byte_index, default_plugin_dirs, digits};
use anyhow::{Context, Result};
use crossterm::terminal;
use std::cmp::max;
use std::fs;
use std::mem;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

pub use builtin_commands::register_builtin_commands;

/// The top-level application state.
pub struct Editor {
    /// The editable document (lines of text).
    pub buf: Buffer,
    /// Cursor position in the buffer.
    pub cursor: Pos,
    /// Selection anchor.
    pub anchor: Option<Pos>,
    /// Viewport scroll position.
    pub scroll_y: usize,
    pub scroll_x: usize,
    /// Path we'll save to.
    pub file_path: Option<PathBuf>,
    /// "Dirty" means there are unsaved changes.
    pub dirty: bool,
    /// Optional bottom-line prompt.
    pub(crate) prompt: Option<Prompt>,
    /// Short-lived status message.
    pub(crate) status: Option<StatusMsg>,
    /// Tracks quit confirmation timing.
    pub(crate) last_quit_hint: Option<Instant>,
    /// Undo and redo stacks.
    pub(crate) undo: Vec<UndoEntry>,
    pub(crate) redo: Vec<UndoEntry>,
    /// Clipboard access.
    pub(crate) clipboard: Option<arboard::Clipboard>,
    /// Command registry.
    pub(crate) commands: CommandRegistry,
    /// Loaded plugins.
    pub(crate) plugins: PluginManager,
    /// Last find query.
    pub(crate) last_find: Option<String>,
    /// Whether the screen needs to be redrawn.
    pub(crate) needs_redraw: bool,
    /// Whether word wrapping is enabled.
    pub word_wrap: bool,
    /// Whether the help screen is displayed.
    pub show_help: bool,
    /// Whether the stats screen is displayed.
    pub show_stats: bool,
}

impl Editor {
    /// Create a new editor.
    pub fn new(path: Option<PathBuf>) -> Result<Self> {
        let mut buf = Buffer::new();
        let mut file_path = None;

        if let Some(p) = path {
            if p.exists() {
                let s = fs::read_to_string(&p)
                    .with_context(|| format!("Failed to read file: {}", p.display()))?;
                buf = Buffer::from_string(&s);
                file_path = Some(p);
            } else {
                file_path = Some(p);
            }
        }

        let clipboard = arboard::Clipboard::new().ok();
        let mut commands = CommandRegistry::new();
        register_builtin_commands(&mut commands);
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
            needs_redraw: true,
            word_wrap: false,
            show_help: false,
            show_stats: false,
        };

        if let Some(p) = ed.file_path.clone() {
            let mut plugins = mem::take(&mut ed.plugins);
            plugins.call_hook(&mut ed, Hook::OnOpen, Some(&p))?;
            ed.plugins = plugins;
        }

        ed.set_status("Ctrl+P commands • Ctrl+S save • Ctrl+Q quit", Duration::from_secs(4));
        Ok(ed)
    }

    /// Mark that the screen needs to be redrawn.
    pub fn mark_redraw(&mut self) {
        self.needs_redraw = true;
    }

    /// Toggle word wrapping.
    pub fn toggle_word_wrap(&mut self) {
        self.word_wrap = !self.word_wrap;
        if self.word_wrap { self.scroll_x = 0; }
        self.set_status(format!("Word wrap: {}", if self.word_wrap { "on" } else { "off" }), Duration::from_secs(2));
        self.mark_redraw();
    }

    /// Toggle line endings.
    pub fn toggle_line_ending(&mut self) {
        self.buf.line_ending = match self.buf.line_ending {
            LineEnding::LF => LineEnding::CRLF,
            LineEnding::CRLF => LineEnding::LF,
        };
        self.dirty = true;
        self.set_status(format!("Line endings: {}", self.buf.line_ending.name()), Duration::from_secs(2));
        self.mark_redraw();
    }

    /// Periodic updates (expire status messages).
    pub fn tick(&mut self) {
        if let Some(st) = &self.status {
            if Instant::now() >= st.until {
                self.status = None;
                self.mark_redraw();
            }
        }
    }

    /// Called when the terminal is resized.
    pub fn on_resize(&mut self) -> Result<()> {
        self.mark_redraw();
        self.ensure_visible()?;
        Ok(())
    }

    /// Show a message in the status bar.
    pub fn set_status(&mut self, msg: impl Into<String>, ttl: Duration) {
        self.status = Some(StatusMsg { text: msg.into(), until: Instant::now() + ttl });
        self.mark_redraw();
    }

    /// Return the normalized selection range.
    pub fn selection_range(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cursor { None }
        else if a <= self.cursor { Some((a, self.cursor)) }
        else { Some((self.cursor, a)) }
    }

    /// Clear any selection.
    pub fn clear_selection(&mut self) {
        self.anchor = None;
        self.mark_redraw();
    }

    /// Select the entire buffer.
    pub fn select_all(&mut self) {
        self.anchor = Some(Pos { y: 0, x: 0 });
        let last_y = self.buf.line_count().saturating_sub(1);
        let last_x = self.buf.line_len_chars(last_y);
        self.cursor = Pos { y: last_y, x: last_x };
        self.mark_redraw();
    }

    /// Extract the selected text.
    pub fn selected_text(&self) -> String {
        let Some((a, b)) = self.selection_range() else { return String::new(); };
        if a.y == b.y {
            let line = &self.buf.lines[a.y];
            let b0 = char_to_byte_index(line, a.x);
            let b1 = char_to_byte_index(line, b.x);
            return line[b0..b1].to_string();
        }
        let mut out = String::new();
        {
            let line = &self.buf.lines[a.y];
            let b0 = char_to_byte_index(line, a.x);
            out.push_str(&line[b0..]);
            out.push('\n');
        }
        for y in (a.y + 1)..b.y {
            out.push_str(&self.buf.lines[y]);
            out.push('\n');
        }
        {
            let line = &self.buf.lines[b.y];
            let b1 = char_to_byte_index(line, b.x);
            out.push_str(&line[..b1]);
        }
        out
    }

    /// Delete the current selection.
    pub fn delete_selection(&mut self) {
        if let Some((a, b)) = self.selection_range() {
            self.cursor = self.buf.delete_range(a, b);
            self.clear_selection();
            self.dirty = true;
        }
    }

    /// Replace the selection or insert at cursor.
    pub fn replace_selection_or_insert(&mut self, text: &str) {
        if self.selection_range().is_some() { self.delete_selection(); }
        self.cursor = self.buf.insert_str(self.cursor, text);
        self.dirty = true;
        self.mark_redraw();
    }

    /// Update scroll so the cursor is visible.
    pub fn ensure_visible(&mut self) -> Result<()> {
        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;
        let prompt_lines = if self.prompt.is_some() { 1 } else { 0 };
        let editor_h = height.saturating_sub(prompt_lines + 1);
        let old_scroll_y = self.scroll_y;
        let old_scroll_x = self.scroll_x;

        if self.word_wrap {
            self.ensure_visible_wrapped(width, editor_h)?;
        } else {
            self.ensure_visible_normal(width, editor_h)?;
        }

        if old_scroll_y != self.scroll_y || old_scroll_x != self.scroll_x {
            self.mark_redraw();
        }
        Ok(())
    }

    fn ensure_visible_wrapped(&mut self, width: usize, editor_h: usize) -> Result<()> {
        let lnw = max(2, digits(self.buf.line_count()));
        let gutter = lnw + 2;
        let avail = width.saturating_sub(gutter);

        let mut cursor_screen_row = 0;
        for (i, line) in self.buf.lines.iter().enumerate() {
            if i == self.cursor.y {
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
        Ok(())
    }

    fn ensure_visible_normal(&mut self, width: usize, editor_h: usize) -> Result<()> {
        if self.cursor.y < self.scroll_y {
            self.scroll_y = self.cursor.y;
        } else if self.cursor.y >= self.scroll_y + editor_h {
            self.scroll_y = self.cursor.y.saturating_sub(editor_h.saturating_sub(1));
        }

        let lnw = max(2, digits(self.buf.line_count()));
        let gutter = lnw + 2;
        let avail = width.saturating_sub(gutter).saturating_sub(1);

        let line = &self.buf.lines[self.cursor.y];
        let cursor_col: usize = line.chars().take(self.cursor.x)
            .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1)).sum();
        let scroll_col: usize = line.chars().take(self.scroll_x)
            .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1)).sum();

        if cursor_col < scroll_col {
            self.scroll_x = self.cursor.x;
        } else if cursor_col >= scroll_col + avail {
            let target_col = cursor_col.saturating_sub(avail.saturating_sub(1));
            let mut col = 0;
            let mut new_scroll_x = 0;
            for (i, ch) in line.chars().enumerate() {
                if col >= target_col { new_scroll_x = i; break; }
                col += UnicodeWidthChar::width(ch).unwrap_or(1);
            }
            self.scroll_x = new_scroll_x;
        }
        Ok(())
    }

    /// Run a command by name.
    pub fn run_command_by_name(&mut self, name: &str) -> Result<bool> {
        let name = name.trim();
        if name.eq_ignore_ascii_case("quit") { return Ok(self.try_quit()); }
        if name.eq_ignore_ascii_case("save_and_quit") {
            self.cmd_save()?;
            return Ok(self.try_quit());
        }

        let cmd_opt = self.commands.get(name).cloned();
        if let Some(cmd) = cmd_opt {
            match cmd.source {
                CommandSource::Builtin(f) => { f(self)?; }
                CommandSource::Plugin { plugin_id, func } => {
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
            let mut msg = format!("Unknown command: '{}'", name);
            if let Some(suggestion) = self.commands.suggest_command(name) {
                msg.push_str(&format!(". Did you mean '{}'?", suggestion.name));
            }
            self.set_status(msg, Duration::from_secs(3));
            Ok(false)
        }
    }
}
