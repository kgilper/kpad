//! Input handling: keyboard, mouse, and prompt events.

use crate::commands::canonical_key_string;
use crate::types::{EditOperation, Pos, Prompt, PromptKind};
use crate::utils::clamp_usize;
use super::Editor;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use std::cmp::min;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

/// Get path completions for a partial path.
/// Returns a sorted list of matching paths (directories first, with trailing `/`).
fn get_path_completions(partial: &str) -> Vec<String> {
    let path = Path::new(partial);

    // Determine the directory to search and the prefix to match
    let (dir, prefix) = if partial.is_empty() {
        (Path::new("."), "")
    } else if partial.ends_with('/') || partial.ends_with('\\') {
        (path, "")
    } else if path.is_dir() {
        (path, "")
    } else {
        let parent = path.parent().unwrap_or(Path::new("."));
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        (parent, file_name)
    };

    let mut completions = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Check if name starts with prefix (case-insensitive on Windows)
            #[cfg(windows)]
            let matches = name_str.to_lowercase().starts_with(&prefix.to_lowercase());
            #[cfg(not(windows))]
            let matches = name_str.starts_with(prefix);

            if matches {
                let full_path = if dir == Path::new(".") && !partial.starts_with("./") {
                    name_str.to_string()
                } else {
                    dir.join(&*name_str).to_string_lossy().to_string()
                };

                // Append / for directories
                let completion = if entry.path().is_dir() {
                    format!("{}/", full_path)
                } else {
                    full_path
                };
                completions.push(completion);
            }
        }
    }

    // Sort: directories first, then alphabetically
    completions.sort_by(|a, b| {
        let a_is_dir = a.ends_with('/');
        let b_is_dir = b.ends_with('/');
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.to_lowercase().cmp(&b.to_lowercase()),
        }
    });

    completions
}

/// Find the longest common prefix among a list of strings.
fn longest_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    if strings.len() == 1 {
        return strings[0].clone();
    }

    let first = &strings[0];
    let mut prefix_len = first.len();

    for s in &strings[1..] {
        prefix_len = first
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a == b)
            .count();
        if prefix_len == 0 {
            break;
        }
        prefix_len = first.chars().take(prefix_len).collect::<String>().len();
    }

    first.chars().take(prefix_len).collect()
}

impl Editor {
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

        // Prompt mode consumes keys first
        if self.prompt.is_some() {
            return self.handle_prompt_key(key);
        }

        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Turn the raw key event into a canonical string like "Ctrl+S"
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

        // If key matches a registered command (built-in or plugin), run it
        if let Some(cmd_name) = self.commands.resolve_key(&key_str) {
            return Ok(self.run_command_by_name(&cmd_name)?);
        }

        // Common direct-edit keys
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
                self.clear_selection();
            }
            KeyCode::Enter => {
                let op = EditOperation::Insert { pos: self.cursor, text: "\n".to_string() };
                self.record_edit(op);
                if self.selection_range().is_some() {
                    self.delete_selection();
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
                self.replace_selection_or_insert("    ");
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
                    self.replace_selection_or_insert(&text);
                    self.ensure_visible()?;
                }
            }
            _ => {}
        }

        Ok(false)
    }

    /// Quit handling with a safety confirmation if there are unsaved changes.
    pub fn try_quit(&mut self) -> bool {
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

    /// Handle keys while a prompt is active.
    pub fn handle_prompt_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(prompt) = &mut self.prompt else { return Ok(false); };

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match (key.code, ctrl) {
            (KeyCode::Esc, _) => {
                self.prompt = None;
                self.mark_redraw();
                return Ok(false);
            }
            (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
                // Tab completion for Open/SaveAs prompts
                if prompt.kind == PromptKind::Open || prompt.kind == PromptKind::SaveAs {
                    let shift = key.code == KeyCode::BackTab;

                    // Check if input changed since last Tab
                    if prompt.completion_base != prompt.input {
                        // Fresh completion: get new completions
                        prompt.completions = get_path_completions(&prompt.input);
                        prompt.completion_base = prompt.input.clone();
                        prompt.completion_index = None;
                    }

                    if prompt.completions.is_empty() {
                        self.set_status("No completions", Duration::from_secs(1));
                    } else if prompt.completions.len() == 1 {
                        // Single match: complete it
                        prompt.input = prompt.completions[0].clone();
                        prompt.cursor = prompt.input.chars().count();
                        prompt.completion_base = prompt.input.clone();
                        // Get new completions for the completed path
                        prompt.completions = get_path_completions(&prompt.input);
                    } else {
                        // Multiple matches
                        if prompt.completion_index.is_none() {
                            // First Tab: complete to common prefix
                            let prefix = longest_common_prefix(&prompt.completions);
                            if prefix.len() > prompt.input.len() {
                                prompt.input = prefix;
                                prompt.cursor = prompt.input.chars().count();
                                prompt.completion_base = prompt.input.clone();
                                prompt.completions = get_path_completions(&prompt.input);
                            } else {
                                // Already at common prefix, start cycling
                                prompt.completion_index = Some(0);
                                prompt.input = prompt.completions[0].clone();
                                prompt.cursor = prompt.input.chars().count();
                            }
                        } else {
                            // Subsequent Tab: cycle through completions
                            let idx = prompt.completion_index.unwrap();
                            let new_idx = if shift {
                                if idx == 0 { prompt.completions.len() - 1 } else { idx - 1 }
                            } else {
                                (idx + 1) % prompt.completions.len()
                            };
                            prompt.completion_index = Some(new_idx);
                            prompt.input = prompt.completions[new_idx].clone();
                            prompt.cursor = prompt.input.chars().count();
                        }

                        // Show completion options in status
                        let display: Vec<&str> = prompt.completions.iter()
                            .map(|s| s.rsplit('/').next().unwrap_or(s).trim_end_matches('/'))
                            .take(8)
                            .collect();
                        let msg = if prompt.completions.len() > 8 {
                            format!("{} (+{} more)", display.join(" | "), prompt.completions.len() - 8)
                        } else {
                            display.join(" | ")
                        };
                        self.set_status(msg, Duration::from_secs(3));
                    }
                    self.mark_redraw();
                }
                return Ok(false);
            }
            (KeyCode::Enter, _) => {
                let kind = prompt.kind;
                let input = prompt.input.clone();
                self.prompt = None;
                self.mark_redraw();

                match kind {
                    PromptKind::Open => {
                        let p = std::path::PathBuf::from(input.trim());
                        if p.as_os_str().is_empty() {
                            return Ok(false);
                        }
                        self.open_path(p)?;
                    }
                    PromptKind::SaveAs => {
                        let p = std::path::PathBuf::from(input.trim());
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
                        // Support Vim-like shorthands
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
                // Ctrl+U clears prompt line
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
}
