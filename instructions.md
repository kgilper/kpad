Below is a **working, Notepad-like terminal text editor for Windows** (PowerShell + CMD + Windows Terminal) written in **Rust**, with:

* **Attractive TUI** (line numbers, status bar, selection highlight)
* **Convenient keybindings** (Ctrl+S save, Ctrl+O open, Ctrl+P command line, Ctrl+F find, etc.)
* **Selection support** (Shift+Arrows, Ctrl+A)
* **Clipboard** (Ctrl+C/X/V using Windows clipboard)
* **Undo/Redo** (Ctrl+Z / Ctrl+Y)
* A **plugin architecture** that loads plugins from `./plugins/*/plugin.toml` + `*.rhai` scripts
  (plugins can add commands, formatting, selection transforms, etc.)

This is designed specifically to be comfortable in **PowerShell** and **cmd.exe**.

---

## Project layout

Create a new project:

```powershell
cargo new winpad
cd winpad
```

Then create this structure:

```
winpad/
  Cargo.toml
  src/
    main.rs
  plugins/
    uppercase/
      plugin.toml
      main.rhai
    trim_whitespace/
      plugin.toml
      main.rhai
```

---

## Cargo.toml

Replace your `Cargo.toml` with:

```toml
[package]
name = "winpad"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1.0"
crossterm = "0.28"
unicode-width = "0.2"
arboard = "3"
rhai = { version = "1", features = ["std"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
```

---

## src/main.rs

Replace `src/main.rs` with this (single-file editor + plugin system):

```rust
use anyhow::{anyhow, Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    style::{self, Attribute, Color},
    terminal::{self, ClearType},
    ExecutableCommand, QueueableCommand,
};
use serde::Deserialize;
use std::{
    cmp::{max, min},
    collections::HashMap,
    fs,
    io::{self, Stdout, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthChar;

//
// ===== Small Notepad-like terminal editor for Windows =====
// - Runs in PowerShell/cmd/Windows Terminal
// - Selection (Shift+Arrows), clipboard, undo/redo
// - Plugin system via ./plugins/*/plugin.toml + Rhai scripts
//

fn main() -> Result<()> {
    run()
}

fn run() -> Result<()> {
    let mut stdout = io::stdout();
    let _term = TerminalGuard::new(&mut stdout)?;

    let file_arg = std::env::args().nth(1).map(PathBuf::from);
    let mut editor = Editor::new(file_arg)?;

    loop {
        editor.render(&mut stdout)?;

        // Poll so we can expire transient status messages.
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    let should_quit = editor.handle_key(key)?;
                    if should_quit {
                        break;
                    }
                }
                Event::Resize(_, _) => {
                    editor.on_resize()?;
                }
                _ => {}
            }
        } else {
            editor.tick();
        }
    }

    Ok(())
}

struct TerminalGuard;

impl TerminalGuard {
    fn new(stdout: &mut Stdout) -> Result<Self> {
        terminal::enable_raw_mode().context("enable_raw_mode failed")?;
        stdout.execute(terminal::EnterAlternateScreen)?;
        stdout.execute(cursor::Hide)?;
        stdout.execute(terminal::Clear(ClearType::All))?;
        stdout.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = stdout.execute(style::ResetColor);
        let _ = stdout.execute(cursor::Show);
        let _ = stdout.execute(terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        let _ = stdout.flush();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pos {
    y: usize,
    x: usize, // char index within line
}

impl Ord for Pos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.y, self.x).cmp(&(other.y, other.x))
    }
}
impl PartialOrd for Pos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Pos,
    anchor: Option<Pos>,
    scroll_y: usize,
    scroll_x: usize,
    dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Open,
    SaveAs,
    Find,
    Command,
    GotoLine,
}

#[derive(Debug, Clone)]
struct Prompt {
    kind: PromptKind,
    input: String,
    cursor: usize, // char index in input
}

impl Prompt {
    fn new(kind: PromptKind, initial: impl Into<String>) -> Self {
        let input = initial.into();
        let cursor = input.chars().count();
        Self { kind, input, cursor }
    }
}

#[derive(Clone)]
struct StatusMsg {
    text: String,
    until: Instant,
}

#[derive(Clone)]
enum CommandSource {
    Builtin(fn(&mut Editor) -> Result<()>),
    Plugin { plugin_id: String, func: String },
}

#[derive(Clone)]
struct Command {
    name: String,
    description: String,
    key: Option<String>, // canonical string e.g. "Ctrl+S"
    source: CommandSource,
}

struct CommandRegistry {
    commands: Vec<Command>,
    by_name: HashMap<String, usize>,
    keymap: HashMap<String, String>, // key -> command_name
}

impl CommandRegistry {
    fn new() -> Self {
        Self {
            commands: vec![],
            by_name: HashMap::new(),
            keymap: HashMap::new(),
        }
    }

    fn register(&mut self, cmd: Command) {
        let name_key = cmd.name.to_lowercase();
        if let Some(k) = cmd.key.as_ref() {
            self.keymap.insert(k.clone(), cmd.name.clone());
        }

        if let Some(&idx) = self.by_name.get(&name_key) {
            self.commands[idx] = cmd;
        } else {
            let idx = self.commands.len();
            self.commands.push(cmd);
            self.by_name.insert(name_key, idx);
        }
    }

    fn get(&self, name: &str) -> Option<&Command> {
        let idx = *self.by_name.get(&name.to_lowercase())?;
        self.commands.get(idx)
    }

    fn list_names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.commands.iter().map(|c| c.name.clone()).collect();
        v.sort();
        v
    }

    fn resolve_key(&self, key: &str) -> Option<String> {
        self.keymap.get(key).cloned()
    }

    fn search(&self, query: &str, limit: usize) -> Vec<&Command> {
        let q = query.to_lowercase();
        let mut items: Vec<&Command> = self
            .commands
            .iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&q) || c.description.to_lowercase().contains(&q)
            })
            .collect();
        items.sort_by_key(|c| c.name.to_lowercase());
        items.truncate(limit);
        items
    }
}

struct Buffer {
    lines: Vec<String>,
}

impl Buffer {
    fn new() -> Self {
        Self { lines: vec![String::new()] }
    }

    fn from_string(s: &str) -> Self {
        let mut lines: Vec<String> = s
            .split('\n')
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self { lines }
    }

    fn to_string(&self) -> String {
        self.lines.join("\n")
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line_len_chars(&self, y: usize) -> usize {
        self.lines.get(y).map(|l| l.chars().count()).unwrap_or(0)
    }

    fn clamp_pos(&self, mut p: Pos) -> Pos {
        if self.lines.is_empty() {
            return Pos { y: 0, x: 0 };
        }
        p.y = min(p.y, self.lines.len().saturating_sub(1));
        p.x = min(p.x, self.line_len_chars(p.y));
        p
    }

    fn insert_char(&mut self, p: Pos, ch: char) -> Pos {
        let y = p.y;
        let x = p.x;
        if y >= self.lines.len() {
            self.lines.push(String::new());
        }
        let line = &mut self.lines[y];
        let bi = char_to_byte_index(line, x);
        line.insert(bi, ch);
        Pos { y, x: x + 1 }
    }

    fn insert_newline(&mut self, p: Pos) -> Pos {
        let y = p.y;
        let x = p.x;
        let line = &mut self.lines[y];
        let bi = char_to_byte_index(line, x);
        let rest = line.split_off(bi);
        self.lines.insert(y + 1, rest);
        Pos { y: y + 1, x: 0 }
    }

    fn delete_backspace(&mut self, p: Pos) -> Pos {
        let y = p.y;
        let x = p.x;
        if y >= self.lines.len() {
            return Pos { y: 0, x: 0 };
        }
        if x > 0 {
            let line = &mut self.lines[y];
            let bi = char_to_byte_index(line, x - 1);
            line.remove(bi);
            Pos { y, x: x - 1 }
        } else if y > 0 {
            // merge with previous line
            let cur = self.lines.remove(y);
            let prev = &mut self.lines[y - 1];
            let prev_len = prev.chars().count();
            prev.push_str(&cur);
            Pos { y: y - 1, x: prev_len }
        } else {
            p
        }
    }

    fn delete_delete(&mut self, p: Pos) -> Pos {
        let y = p.y;
        let x = p.x;
        if y >= self.lines.len() {
            return Pos { y: 0, x: 0 };
        }
        let len = self.lines[y].chars().count();
        if x < len {
            let line = &mut self.lines[y];
            let bi = char_to_byte_index(line, x);
            line.remove(bi);
            p
        } else if y + 1 < self.lines.len() {
            // merge with next line
            let next = self.lines.remove(y + 1);
            self.lines[y].push_str(&next);
            p
        } else {
            p
        }
    }

    fn delete_range(&mut self, start: Pos, end: Pos) -> Pos {
        if start == end {
            return start;
        }
        let (a, b) = if start <= end { (start, end) } else { (end, start) };

        if a.y == b.y {
            let line = &mut self.lines[a.y];
            let b0 = char_to_byte_index(line, a.x);
            let b1 = char_to_byte_index(line, b.x);
            line.replace_range(b0..b1, "");
            return a;
        }

        let end_suffix = {
            let end_line = &self.lines[b.y];
            let b_end = char_to_byte_index(end_line, b.x);
            end_line[b_end..].to_string()
        };

        // truncate start line at a.x
        {
            let start_line = &mut self.lines[a.y];
            let b_start = char_to_byte_index(start_line, a.x);
            start_line.truncate(b_start);
        }

        // remove middle lines including original end line
        self.lines.drain(a.y + 1..=b.y);

        // append suffix
        self.lines[a.y].push_str(&end_suffix);

        if self.lines.is_empty() {
            self.lines.push(String::new());
            return Pos { y: 0, x: 0 };
        }

        a
    }

    fn insert_str(&mut self, p: Pos, text: &str) -> Pos {
        let normalized = text.replace("\r\n", "\n");
        let parts: Vec<&str> = normalized.split('\n').collect();
        if parts.len() == 1 {
            let mut pos = p;
            for ch in parts[0].chars() {
                pos = self.insert_char(pos, ch);
            }
            return pos;
        }

        let y = p.y;
        let x = p.x;

        let suffix = {
            let line = &mut self.lines[y];
            let bi = char_to_byte_index(line, x);
            line.split_off(bi)
        };

        // append first part
        self.lines[y].push_str(parts[0]);

        // insert middle lines
        let mut insert_at = y + 1;
        for mid in &parts[1..parts.len() - 1] {
            self.lines.insert(insert_at, (*mid).to_string());
            insert_at += 1;
        }

        // last line = last part + old suffix
        let mut last = parts[parts.len() - 1].to_string();
        last.push_str(&suffix);
        self.lines.insert(insert_at, last);

        Pos {
            y: y + parts.len() - 1,
            x: parts[parts.len() - 1].chars().count(),
        }
    }
}

fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    let mut ci = 0usize;
    for (bi, _) in s.char_indices() {
        if ci == char_idx {
            return bi;
        }
        ci += 1;
    }
    s.len()
}

fn byte_to_char_index(s: &str, byte_idx: usize) -> usize {
    s[..min(byte_idx, s.len())].chars().count()
}

fn digits(n: usize) -> usize {
    n.to_string().len()
}

fn clamp_usize(v: isize, lo: usize, hi: usize) -> usize {
    if v < lo as isize {
        lo
    } else if v > hi as isize {
        hi
    } else {
        v as usize
    }
}

struct Editor {
    buf: Buffer,
    cursor: Pos,
    anchor: Option<Pos>, // selection anchor
    scroll_y: usize,
    scroll_x: usize,

    file_path: Option<PathBuf>,
    dirty: bool,

    prompt: Option<Prompt>,
    status: Option<StatusMsg>,
    last_quit_hint: Option<Instant>,

    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,

    clipboard: Option<arboard::Clipboard>,

    commands: CommandRegistry,
    plugins: PluginManager,

    // last find query
    last_find: Option<String>,
}

impl Editor {
    fn new(path: Option<PathBuf>) -> Result<Self> {
        let mut buf = Buffer::new();
        let mut file_path = None;

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
        };

        // Call plugin on_open hook if relevant
        if let Some(p) = ed.file_path.clone() {
            ed.plugins.call_hook(&mut ed, Hook::OnOpen, Some(&p))?;
        }

        ed.set_status("Ctrl+P commands • Ctrl+S save • Ctrl+Q quit", Duration::from_secs(4));
        Ok(ed)
    }

    fn tick(&mut self) {
        // expire status messages
        if let Some(st) = &self.status {
            if Instant::now() >= st.until {
                self.status = None;
            }
        }
    }

    fn on_resize(&mut self) -> Result<()> {
        // just ensure visible; render will query size
        self.ensure_visible()?;
        Ok(())
    }

    fn set_status(&mut self, msg: impl Into<String>, ttl: Duration) {
        self.status = Some(StatusMsg { text: msg.into(), until: Instant::now() + ttl });
    }

    fn push_undo(&mut self) {
        // cap undo history
        const CAP: usize = 200;
        let snap = Snapshot {
            lines: self.buf.lines.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            scroll_y: self.scroll_y,
            scroll_x: self.scroll_x,
            dirty: self.dirty,
        };
        self.undo.push(snap);
        if self.undo.len() > CAP {
            let excess = self.undo.len() - CAP;
            self.undo.drain(0..excess);
        }
        self.redo.clear();
    }

    fn undo(&mut self) -> Result<()> {
        if let Some(prev) = self.undo.pop() {
            let cur = Snapshot {
                lines: self.buf.lines.clone(),
                cursor: self.cursor,
                anchor: self.anchor,
                scroll_y: self.scroll_y,
                scroll_x: self.scroll_x,
                dirty: self.dirty,
            };
            self.redo.push(cur);

            self.buf.lines = prev.lines;
            self.cursor = prev.cursor;
            self.anchor = prev.anchor;
            self.scroll_y = prev.scroll_y;
            self.scroll_x = prev.scroll_x;
            self.dirty = prev.dirty;
            self.ensure_visible()?;
        }
        Ok(())
    }

    fn redo(&mut self) -> Result<()> {
        if let Some(next) = self.redo.pop() {
            let cur = Snapshot {
                lines: self.buf.lines.clone(),
                cursor: self.cursor,
                anchor: self.anchor,
                scroll_y: self.scroll_y,
                scroll_x: self.scroll_x,
                dirty: self.dirty,
            };
            self.undo.push(cur);

            self.buf.lines = next.lines;
            self.cursor = next.cursor;
            self.anchor = next.anchor;
            self.scroll_y = next.scroll_y;
            self.scroll_x = next.scroll_x;
            self.dirty = next.dirty;
            self.ensure_visible()?;
        }
        Ok(())
    }

    fn selection_range(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cursor {
            None
        } else if a <= self.cursor {
            Some((a, self.cursor))
        } else {
            Some((self.cursor, a))
        }
    }

    fn clear_selection(&mut self) {
        self.anchor = None;
    }

    fn select_all(&mut self) {
        self.anchor = Some(Pos { y: 0, x: 0 });
        let last_y = self.buf.line_count().saturating_sub(1);
        let last_x = self.buf.line_len_chars(last_y);
        self.cursor = Pos { y: last_y, x: last_x };
    }

    fn selected_text(&self) -> String {
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

    fn delete_selection(&mut self) {
        if let Some((a, b)) = self.selection_range() {
            self.cursor = self.buf.delete_range(a, b);
            self.clear_selection();
            self.dirty = true;
        }
    }

    fn replace_selection_or_insert(&mut self, text: &str) {
        if self.selection_range().is_some() {
            self.delete_selection();
        }
        self.cursor = self.buf.insert_str(self.cursor, text);
        self.dirty = true;
    }

    fn ensure_visible(&mut self) -> Result<()> {
        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;

        let prompt_lines = if self.prompt.is_some() { 1 } else { 0 };
        let status_lines = 1;
        let editor_h = height.saturating_sub(prompt_lines + status_lines);

        // vertical
        if self.cursor.y < self.scroll_y {
            self.scroll_y = self.cursor.y;
        } else if self.cursor.y >= self.scroll_y + editor_h {
            self.scroll_y = self.cursor.y.saturating_sub(editor_h.saturating_sub(1));
        }

        // horizontal (simple char-index based)
        let lnw = max(2, digits(self.buf.line_count()));
        let gutter = lnw + 2; // "NN│"
        let avail = width.saturating_sub(gutter).saturating_sub(1);

        if self.cursor.x < self.scroll_x {
            self.scroll_x = self.cursor.x;
        } else if self.cursor.x >= self.scroll_x + avail {
            self.scroll_x = self.cursor.x.saturating_sub(avail.saturating_sub(1));
        }

        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // Prompt mode consumes keys first
        if self.prompt.is_some() {
            return self.handle_prompt_key(key);
        }

        // Special selection movement with Shift+Arrows/Home/End
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Normalize a key string for command keymap
        let key_str = canonical_key_string(&key);

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

        // If key matches registered command (built-in or plugin), run it
        if let Some(cmd_name) = self.commands.resolve_key(&key_str) {
            return Ok(self.run_command_by_name(&cmd_name)?);
        }

        // Common direct-edit keys
        match (key.code, ctrl) {
            (KeyCode::Char('q'), true) => return Ok(self.try_quit()),
            (KeyCode::Char('s'), true) => { self.cmd_save()?; return Ok(false); }
            (KeyCode::Char('o'), true) => { self.prompt = Some(Prompt::new(PromptKind::Open, "")); return Ok(false); }
            (KeyCode::Char('f'), true) => { self.prompt = Some(Prompt::new(PromptKind::Find, self.last_find.clone().unwrap_or_default())); return Ok(false); }
            (KeyCode::Char('p'), true) => { self.prompt = Some(Prompt::new(PromptKind::Command, "")); return Ok(false); }
            (KeyCode::Char('g'), true) => { self.prompt = Some(Prompt::new(PromptKind::GotoLine, "")); return Ok(false); }
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
                self.clear_selection();
            }
            KeyCode::Enter => {
                self.push_undo();
                if self.selection_range().is_some() {
                    self.delete_selection();
                }
                self.cursor = self.buf.insert_newline(self.cursor);
                self.dirty = true;
                self.ensure_visible()?;
            }
            KeyCode::Backspace => {
                self.push_undo();
                if self.selection_range().is_some() {
                    self.delete_selection();
                } else {
                    self.cursor = self.buf.delete_backspace(self.cursor);
                    self.dirty = true;
                }
                self.ensure_visible()?;
            }
            KeyCode::Delete => {
                self.push_undo();
                if self.selection_range().is_some() {
                    self.delete_selection();
                } else {
                    self.cursor = self.buf.delete_delete(self.cursor);
                    self.dirty = true;
                }
                self.ensure_visible()?;
            }
            KeyCode::Tab => {
                self.push_undo();
                self.replace_selection_or_insert("    ");
                self.ensure_visible()?;
            }
            KeyCode::Char(ch) => {
                // Text input (ignore control chars)
                if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
                    // ignore (handled above / keymap)
                } else {
                    self.push_undo();
                    let mut s = String::new();
                    s.push(ch);
                    self.replace_selection_or_insert(&s);
                    self.ensure_visible()?;
                }
            }
            _ => {}
        }

        Ok(false)
    }

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

    fn move_cursor(&mut self, key: KeyEvent, selecting: bool) -> Result<()> {
        if selecting && self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        if !selecting {
            self.clear_selection();
        }

        let (w, h) = terminal::size()?;
        let height = h as usize;
        let prompt_lines = if self.prompt.is_some() { 1 } else { 0 };
        let status_lines = 1;
        let editor_h = height.saturating_sub(prompt_lines + status_lines);

        let mut p = self.cursor;
        match key.code {
            KeyCode::Up => {
                if p.y > 0 {
                    p.y -= 1;
                    p.x = min(p.x, self.buf.line_len_chars(p.y));
                }
            }
            KeyCode::Down => {
                if p.y + 1 < self.buf.line_count() {
                    p.y += 1;
                    p.x = min(p.x, self.buf.line_len_chars(p.y));
                }
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    p = self.move_word_left(p);
                } else if p.x > 0 {
                    p.x -= 1;
                } else if p.y > 0 {
                    p.y -= 1;
                    p.x = self.buf.line_len_chars(p.y);
                }
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    p = self.move_word_right(p);
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
            KeyCode::Home => p.x = 0,
            KeyCode::End => p.x = self.buf.line_len_chars(p.y),
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

        self.cursor = self.buf.clamp_pos(p);
        self.ensure_visible()?;
        Ok(())
    }

    fn move_word_left(&self, mut p: Pos) -> Pos {
        let line = &self.buf.lines[p.y];
        let chars: Vec<char> = line.chars().collect();
        if p.x == 0 {
            return p;
        }
        let mut i = min(p.x, chars.len());
        // skip whitespace left
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        // skip non-whitespace left
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        p.x = i;
        p
    }

    fn move_word_right(&self, mut p: Pos) -> Pos {
        let line = &self.buf.lines[p.y];
        let chars: Vec<char> = line.chars().collect();
        let mut i = min(p.x, chars.len());
        // skip non-whitespace right
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        // skip whitespace right
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        p.x = i;
        p
    }

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

    fn cut(&mut self) -> Result<()> {
        let text = self.selected_text();
        if text.is_empty() {
            self.set_status("Nothing selected to cut.", Duration::from_secs(2));
            return Ok(());
        }
        self.push_undo();
        if let Some(cb) = &mut self.clipboard {
            cb.set_text(text).ok();
        }
        self.delete_selection();
        self.ensure_visible()?;
        self.set_status("Cut selection.", Duration::from_secs(2));
        Ok(())
    }

    fn paste(&mut self) -> Result<()> {
        if let Some(cb) = &mut self.clipboard {
            if let Ok(text) = cb.get_text() {
                self.push_undo();
                self.replace_selection_or_insert(&text);
                self.ensure_visible()?;
                self.set_status("Pasted.", Duration::from_secs(2));
                return Ok(());
            }
        }
        self.set_status("Clipboard unavailable.", Duration::from_secs(2));
        Ok(())
    }

    fn cmd_save(&mut self) -> Result<()> {
        if self.file_path.is_none() {
            self.prompt = Some(Prompt::new(PromptKind::SaveAs, ""));
            return Ok(());
        }
        self.save_to_path(self.file_path.clone().unwrap())
    }

    fn save_to_path(&mut self, path: PathBuf) -> Result<()> {
        let content = self.buf.to_string();
        fs::write(&path, content).with_context(|| format!("Failed writing {}", path.display()))?;
        self.file_path = Some(path.clone());
        self.dirty = false;
        self.set_status(format!("Saved: {}", path.display()), Duration::from_secs(2));

        self.plugins.call_hook(self, Hook::OnSave, Some(&path))?;
        Ok(())
    }

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

        self.plugins.call_hook(self, Hook::OnOpen, Some(&path))?;
        self.set_status(format!("Opened: {}", path.display()), Duration::from_secs(2));
        Ok(())
    }

    fn find_next(&mut self, query: &str) -> Result<()> {
        if query.is_empty() {
            return Ok(());
        }
        self.last_find = Some(query.to_string());

        let start_pos = self.cursor;
        if let Some(p) = self.search_forward(query, start_pos, true) {
            self.cursor = p;
            self.clear_selection();
            self.ensure_visible()?;
            self.set_status("Match found.", Duration::from_secs(1));
        } else {
            self.set_status("No matches.", Duration::from_secs(2));
        }
        Ok(())
    }

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

    fn handle_prompt_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(prompt) = &mut self.prompt else { return Ok(false); };

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match (key.code, ctrl) {
            (KeyCode::Esc, _) => {
                self.prompt = None;
                return Ok(false);
            }
            (KeyCode::Enter, _) => {
                let kind = prompt.kind;
                let input = prompt.input.clone();
                self.prompt = None;

                match kind {
                    PromptKind::Open => {
                        let p = PathBuf::from(input.trim());
                        if p.as_os_str().is_empty() {
                            return Ok(false);
                        }
                        self.open_path(p)?;
                    }
                    PromptKind::SaveAs => {
                        let p = PathBuf::from(input.trim());
                        if p.as_os_str().is_empty() {
                            return Ok(false);
                        }
                        self.save_to_path(p)?;
                    }
                    PromptKind::Find => {
                        self.find_next(input.trim())?;
                    }
                    PromptKind::GotoLine => {
                        let n: isize = input.trim().parse().unwrap_or(1);
                        let target = clamp_usize(n - 1, 0, self.buf.line_count().saturating_sub(1));
                        self.cursor.y = target;
                        self.cursor.x = min(self.cursor.x, self.buf.line_len_chars(self.cursor.y));
                        self.clear_selection();
                        self.ensure_visible()?;
                    }
                    PromptKind::Command => {
                        let cmdline = input.trim();
                        if cmdline.is_empty() {
                            return Ok(false);
                        }
                        // allow ":w" ":q"
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
                }
            }
            (KeyCode::Delete, _) => {
                let len = prompt.input.chars().count();
                if prompt.cursor < len {
                    let mut chars: Vec<char> = prompt.input.chars().collect();
                    chars.remove(prompt.cursor);
                    prompt.input = chars.into_iter().collect();
                }
            }
            (KeyCode::Left, _) => {
                prompt.cursor = prompt.cursor.saturating_sub(1);
            }
            (KeyCode::Right, _) => {
                let len = prompt.input.chars().count();
                prompt.cursor = min(prompt.cursor + 1, len);
            }
            (KeyCode::Home, _) => prompt.cursor = 0,
            (KeyCode::End, _) => prompt.cursor = prompt.input.chars().count(),
            (KeyCode::Char(ch), true) if ch == 'u' => {
                // Ctrl+U clears prompt line (handy in shells)
                prompt.input.clear();
                prompt.cursor = 0;
            }
            (KeyCode::Char(ch), _) => {
                if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) {
                    // ignore
                } else {
                    let mut chars: Vec<char> = prompt.input.chars().collect();
                    chars.insert(prompt.cursor, ch);
                    prompt.input = chars.into_iter().collect();
                    prompt.cursor += 1;
                }
            }
            _ => {}
        }

        Ok(false)
    }

    fn run_command_by_name(&mut self, name: &str) -> Result<bool> {
        let name = name.trim();
        if name.eq_ignore_ascii_case("quit") {
            return Ok(self.try_quit());
        }
        if name.eq_ignore_ascii_case("save_and_quit") {
            self.cmd_save()?;
            return Ok(self.try_quit());
        }

        let cmd = self.commands.get(name).ok_or_else(|| anyhow!("Unknown command: {}", name))?.clone();

        match cmd.source {
            CommandSource::Builtin(f) => {
                f(self)?;
            }
            CommandSource::Plugin { plugin_id, func } => {
                // Plugins may modify: record undo first
                self.push_undo();
                self.plugins.run_command(self, &plugin_id, &func)?;
                self.ensure_visible()?;
            }
        }

        Ok(false)
    }

    fn render(&mut self, stdout: &mut Stdout) -> Result<()> {
        self.ensure_visible()?;

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
        stdout.queue(terminal::Clear(ClearType::All))?;

        // Editor lines
        for row in 0..editor_h {
            let y = self.scroll_y + row;
            stdout.queue(cursor::MoveTo(0, row as u16))?;
            stdout.queue(terminal::Clear(ClearType::CurrentLine))?;

            if y >= self.buf.line_count() {
                // tildes
                stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                stdout.queue(style::Print("~"))?;
                stdout.queue(style::ResetColor)?;
                continue;
            }

            let is_current_line = y == self.cursor.y;
            let base_bg = if is_current_line { Some(Color::DarkBlue) } else { None };

            // line number
            if let Some(bg) = base_bg {
                stdout.queue(style::SetBackgroundColor(bg))?;
            }
            stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
            stdout.queue(style::Print(format!("{:>width$}", y + 1, width = lnw)))?;
            stdout.queue(style::Print("│"))?;
            stdout.queue(style::ResetColor)?;

            // line content with selection highlighting
            let line = &self.buf.lines[y];
            let avail = width.saturating_sub(gutter);

            let sel = self.selection_range();
            let mut col_used = 0usize;
            let mut char_i = 0usize;

            // skip scroll_x chars
            let mut chars: Vec<char> = line.chars().collect();
            if self.scroll_x < chars.len() {
                chars = chars[self.scroll_x..].to_vec();
                char_i = self.scroll_x;
            } else {
                chars.clear();
                char_i = self.scroll_x;
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
        }

        // Prompt line
        if let Some(p) = &self.prompt {
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

            // For command prompt, show a few matches in status bar area (just as hint)
            if p.kind == PromptKind::Command && !p.input.trim().is_empty() {
                let hits = self.commands.search(p.input.trim(), 5);
                if !hits.is_empty() {
                    let mut hint = String::from("Matches: ");
                    for (i, c) in hits.iter().enumerate() {
                        if i > 0 {
                            hint.push_str(" • ");
                        }
                        hint.push_str(&c.name);
                    }
                    self.set_status(hint, Duration::from_millis(900));
                }
            }
        }

        // Status bar
        stdout.queue(cursor::MoveTo(0, status_y as u16))?;
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
        let left = format!(" {}{}  {}  Ln {}, Col {}  {} ",
            dirty,
            "",
            path_str,
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

        // Put terminal cursor at editing position
        let cursor_row = self.cursor.y.saturating_sub(self.scroll_y);
        let cursor_col = {
            let line = &self.buf.lines[self.cursor.y];
            let start = self.scroll_x;
            let end = self.cursor.x;

            let mut col = 0usize;
            let mut idx = 0usize;
            for (i, ch) in line.chars().enumerate() {
                if i < start {
                    continue;
                }
                if i >= end {
                    break;
                }
                if idx >= (end - start) {
                    break;
                }
                col += UnicodeWidthChar::width(ch).unwrap_or(1);
                idx += 1;
            }
            col
        };

        let cursor_x = (gutter + cursor_col).min(width.saturating_sub(1));
        let cursor_y = cursor_row.min(editor_h.saturating_sub(1));

        stdout.queue(cursor::MoveTo(cursor_x as u16, cursor_y as u16))?;
        stdout.queue(cursor::Show)?;
        stdout.flush()?;

        Ok(())
    }
}

fn canonical_key_string(key: &KeyEvent) -> String {
    // Canonical ordering: Ctrl, Alt, Shift + Key
    let mut parts: Vec<&str> = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift");
    }

    let key_name = match key.code {
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        _ => format!("{:?}", key.code),
    };

    if parts.is_empty() {
        key_name
    } else {
        parts.push(&key_name);
        parts.join("+")
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
        name: "command".to_string(),
        description: "Command prompt / palette (Ctrl+P)".to_string(),
        key: Some("Ctrl+P".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(PromptKind::Command, ""));
            Ok(())
        }),
    });
    reg.register(Command {
        name: "goto_line".to_string(),
        description: "Go to line (Ctrl+G)".to_string(),
        key: Some("Ctrl+G".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(PromptKind::GotoLine, ""));
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
}

fn default_plugin_dirs() -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();

    // 1) ./plugins relative to current working directory
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("plugins"));
    }

    // 2) plugins next to executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("plugins"));
        }
    }

    Ok(dirs)
}

#[derive(Debug, Clone, Copy)]
enum Hook {
    OnOpen,
    OnSave,
}

#[derive(Debug, Deserialize)]
struct PluginManifest {
    id: String,
    name: Option<String>,
    script: String,

    #[serde(default)]
    commands: Vec<PluginCommand>,

    #[serde(default)]
    hooks: PluginHooks,
}

#[derive(Debug, Deserialize)]
struct PluginCommand {
    name: String,
    description: String,
    func: String,
    key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PluginHooks {
    on_open: Option<String>,
    on_save: Option<String>,
}

struct Plugin {
    id: String,
    _name: String,
    ast: rhai::AST,
    hooks: PluginHooks,
}

struct PluginManager {
    engine: rhai::Engine,
    plugins: Vec<Plugin>,
}

impl PluginManager {
    fn load(search_dirs: Vec<PathBuf>, reg: &mut CommandRegistry) -> Result<Self> {
        let mut engine = rhai::Engine::new();
        engine.set_max_operations(2_000_000); // keep plugins from hanging the editor

        // Register PluginApi type and methods
        engine.register_type::<PluginApi>();
        engine.register_fn("text", PluginApi::text);
        engine.register_fn("set_text", PluginApi::set_text);
        engine.register_fn("has_selection", PluginApi::has_selection);
        engine.register_fn("selection_text", PluginApi::selection_text);
        engine.register_fn("replace_selection", PluginApi::replace_selection);
        engine.register_fn("insert", PluginApi::insert);
        engine.register_fn("cursor_line", PluginApi::cursor_line);
        engine.register_fn("cursor_col", PluginApi::cursor_col);
        engine.register_fn("set_cursor", PluginApi::set_cursor);
        engine.register_fn("current_line_text", PluginApi::current_line_text);
        engine.register_fn("set_current_line_text", PluginApi::set_current_line_text);
        engine.register_fn("status", PluginApi::status);
        engine.register_fn("file_path", PluginApi::file_path);

        let mut plugins = Vec::new();

        for dir in search_dirs {
            if !dir.exists() {
                continue;
            }
            // Expect structure: plugins/<plugin>/plugin.toml
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for ent in entries.flatten() {
                let path = ent.path();
                if !path.is_dir() {
                    continue;
                }
                let manifest_path = path.join("plugin.toml");
                if !manifest_path.exists() {
                    continue;
                }

                let manifest_s = fs::read_to_string(&manifest_path)
                    .with_context(|| format!("Reading {}", manifest_path.display()))?;
                let manifest: PluginManifest = toml::from_str(&manifest_s)
                    .with_context(|| format!("Parsing {}", manifest_path.display()))?;

                let script_path = path.join(&manifest.script);
                let ast = engine
                    .compile_file(script_path.clone())
                    .with_context(|| format!("Compiling {}", script_path.display()))?;

                let id = manifest.id.clone();
                let name = manifest.name.clone().unwrap_or_else(|| id.clone());

                // Register commands
                for c in &manifest.commands {
                    reg.register(Command {
                        name: c.name.clone(),
                        description: format!(
                            "{} (plugin: {})",
                            c.description,
                            name
                        ),
                        key: c.key.as_ref().map(|k| normalize_key_string(k)),
                        source: CommandSource::Plugin {
                            plugin_id: id.clone(),
                            func: c.func.clone(),
                        },
                    });
                }

                plugins.push(Plugin {
                    id,
                    _name: name,
                    ast,
                    hooks: manifest.hooks,
                });
            }
        }

        Ok(Self { engine, plugins })
    }

    fn find(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    fn run_command(&self, ed: &mut Editor, plugin_id: &str, func: &str) -> Result<()> {
        let plugin = self.find(plugin_id).ok_or_else(|| anyhow!("Plugin not found: {}", plugin_id))?;
        let api = PluginApi::new(ed);
        let mut scope = rhai::Scope::new();
        self.engine
            .call_fn::<rhai::Dynamic>(&mut scope, &plugin.ast, func, (api,))
            .with_context(|| format!("Plugin command failed: {}::{}", plugin_id, func))?;
        Ok(())
    }

    fn call_hook(&self, ed: &mut Editor, hook: Hook, path: Option<&PathBuf>) -> Result<()> {
        // Best-effort hooks (don’t crash editor)
        for p in &self.plugins {
            let func = match hook {
                Hook::OnOpen => p.hooks.on_open.as_deref(),
                Hook::OnSave => p.hooks.on_save.as_deref(),
            };
            let Some(func) = func else { continue; };

            let api = PluginApi::new(ed);
            let mut scope = rhai::Scope::new();
            let res = if let Some(path) = path {
                self.engine.call_fn::<rhai::Dynamic>(&mut scope, &p.ast, func, (api, path.display().to_string()))
            } else {
                self.engine.call_fn::<rhai::Dynamic>(&mut scope, &p.ast, func, (api,))
            };
            if let Err(e) = res {
                // show but keep going
                ed.set_status(format!("Plugin hook error ({}): {}", p.id, e), Duration::from_secs(3));
            }
        }
        Ok(())
    }
}

fn normalize_key_string(s: &str) -> String {
    // Accept e.g. "ctrl+s", "CTRL+Shift+u"
    // Output canonical: Ctrl+Shift+U
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None::<String>;

    for part in s.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let p = part.to_lowercase();
        match p.as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            _ => {
                key = Some(match p.as_str() {
                    "enter" => "Enter".to_string(),
                    "esc" | "escape" => "Esc".to_string(),
                    "backspace" => "Backspace".to_string(),
                    "delete" | "del" => "Delete".to_string(),
                    "tab" => "Tab".to_string(),
                    "left" => "Left".to_string(),
                    "right" => "Right".to_string(),
                    "up" => "Up".to_string(),
                    "down" => "Down".to_string(),
                    "home" => "Home".to_string(),
                    "end" => "End".to_string(),
                    "pageup" => "PageUp".to_string(),
                    "pagedown" => "PageDown".to_string(),
                    other => {
                        if other.len() == 1 {
                            other.chars().next().unwrap().to_ascii_uppercase().to_string()
                        } else if other.starts_with('f') && other[1..].chars().all(|c| c.is_ascii_digit()) {
                            format!("F{}", &other[1..])
                        } else {
                            // fallback
                            part.to_string()
                        }
                    }
                });
            }
        }
    }

    let key = key.unwrap_or_else(|| "?".to_string());
    let mut parts = Vec::new();
    if ctrl { parts.push("Ctrl".to_string()); }
    if alt { parts.push("Alt".to_string()); }
    if shift { parts.push("Shift".to_string()); }
    parts.push(key);
    parts.join("+")
}

// ===== Plugin API exposed to Rhai =====
//
// Plugins get a PluginApi object. Methods mutate the real editor.
//
#[derive(Clone)]
struct PluginApi {
    // Minimal + safe: a raw pointer back into Editor for this call only.
    // This is okay because calls are synchronous and single-threaded.
    ed: *mut Editor,
}

impl PluginApi {
    fn new(ed: &mut Editor) -> Self {
        Self { ed }
    }

    fn with_editor<T>(&mut self, f: impl FnOnce(&mut Editor) -> T) -> T {
        unsafe { f(&mut *self.ed) }
    }

    fn text(&mut self) -> String {
        self.with_editor(|ed| ed.buf.to_string())
    }

    fn set_text(&mut self, s: String) {
        self.with_editor(|ed| {
            ed.buf = Buffer::from_string(&s);
            ed.cursor = Pos { y: 0, x: 0 };
            ed.anchor = None;
            ed.scroll_y = 0;
            ed.scroll_x = 0;
            ed.dirty = true;
        })
    }

    fn has_selection(&mut self) -> bool {
        self.with_editor(|ed| ed.selection_range().is_some())
    }

    fn selection_text(&mut self) -> String {
        self.with_editor(|ed| ed.selected_text())
    }

    fn replace_selection(&mut self, s: String) {
        self.with_editor(|ed| {
            // plugin edits should be undoable: record snapshot once per command already,
            // so just do the edit
            ed.replace_selection_or_insert(&s);
        })
    }

    fn insert(&mut self, s: String) {
        self.with_editor(|ed| {
            ed.replace_selection_or_insert(&s);
        })
    }

    fn cursor_line(&mut self) -> i64 {
        self.with_editor(|ed| (ed.cursor.y as i64) + 1)
    }

    fn cursor_col(&mut self) -> i64 {
        self.with_editor(|ed| (ed.cursor.x as i64) + 1)
    }

    fn set_cursor(&mut self, line: i64, col: i64) {
        self.with_editor(|ed| {
            let y = clamp_usize(line - 1, 0, ed.buf.line_count().saturating_sub(1));
            let max_x = ed.buf.line_len_chars(y);
            let x = clamp_usize(col - 1, 0, max_x);
            ed.cursor = Pos { y, x };
            ed.anchor = None;
        })
    }

    fn current_line_text(&mut self) -> String {
        self.with_editor(|ed| ed.buf.lines.get(ed.cursor.y).cloned().unwrap_or_default())
    }

    fn set_current_line_text(&mut self, s: String) {
        self.with_editor(|ed| {
            if ed.cursor.y < ed.buf.lines.len() {
                ed.buf.lines[ed.cursor.y] = s;
                ed.cursor.x = min(ed.cursor.x, ed.buf.line_len_chars(ed.cursor.y));
                ed.dirty = true;
            }
        })
    }

    fn status(&mut self, msg: String) {
        self.with_editor(|ed| ed.set_status(msg, Duration::from_secs(2)))
    }

    fn file_path(&mut self) -> String {
        self.with_editor(|ed| {
            ed.file_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        })
    }
}
```

---

## Plugins

### Plugin 1: Uppercase selection

Create `plugins/uppercase/plugin.toml`:

```toml
id = "uppercase"
name = "Uppercase Tools"
script = "main.rhai"

[[commands]]
name = "uppercase_selection"
description = "Uppercase selection (or current line if no selection)"
func = "uppercase_selection"
key = "Ctrl+U"
```

Create `plugins/uppercase/main.rhai`:

```rhai
fn uppercase_selection(api) {
    if api.has_selection() {
        let s = api.selection_text();
        api.replace_selection(s.to_upper());
        api.status("Uppercased selection.");
    } else {
        let line = api.current_line_text();
        api.set_current_line_text(line.to_upper());
        api.status("Uppercased line.");
    }
}
```

### Plugin 2: Trim trailing whitespace

Create `plugins/trim_whitespace/plugin.toml`:

```toml
id = "trim_whitespace"
name = "Trim Whitespace"
script = "main.rhai"

[[commands]]
name = "trim_trailing_ws"
description = "Trim trailing whitespace on every line"
func = "trim_trailing_ws"
key = "Ctrl+Alt+T"
```

Create `plugins/trim_whitespace/main.rhai`:

```rhai
fn trim_trailing_ws(api) {
    let t = api.text();
    let lines = t.split("\n");
    let mut out = [];

    for line in lines {
        // naive trim of spaces + tabs at end
        let mut s = line;
        while s.len > 0 && (s.ends_with(" ") || s.ends_with("\t") || s.ends_with("\r")) {
            s = s.sub_string(0, s.len - 1);
        }
        out.push(s);
    }

    api.set_text(out.join("\n"));
    api.status("Trimmed trailing whitespace.");
}
```

---

## Build & run

```powershell
cargo build --release
.\target\release\winpad.exe
```

Open a file:

```powershell
.\target\release\winpad.exe .\notes.txt
```

---

## Keybindings (built-in)

* **Typing** inserts text
* **Arrow keys** move
* **Shift + Arrows/Home/End** selection
* **Ctrl+A** select all
* **Ctrl+C / Ctrl+X / Ctrl+V** copy/cut/paste
* **Ctrl+Z / Ctrl+Y** undo/redo
* **Ctrl+S** save
* **Ctrl+O** open
* **Ctrl+F** find
* **Ctrl+G** go to line
* **Ctrl+P** command prompt (type a command name, Enter)
* **Ctrl+Q** quit (press twice if unsaved changes)

Plugin commands show up automatically and can be invoked by:

* their **keybinding** (from `plugin.toml`), or
* **Ctrl+P** and typing the command name (e.g. `uppercase_selection`)

---

## How to extend the plugin architecture

To add new editor powers (formatters, selection transforms, new editing behaviors), you:

1. Create a new folder under `plugins/your_plugin/`
2. Add `plugin.toml` with commands + keybindings
3. Write functions in `main.rhai` using the `PluginApi` methods

If you want richer plugins (e.g. multi-command palettes, structured selection ranges, regex search/replace, indentation engines, syntax coloring), tell me what features you want first and I’ll extend the **PluginApi** and core editor accordingly (without changing how plugins are discovered/loaded).
