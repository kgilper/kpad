//! File operations: open, save, search.

use crate::buffer::Buffer; // document model
use crate::plugins::Hook; // plugin lifecycle hooks
use crate::types::{Pos, Prompt, PromptKind}; // core types
use crate::utils::{byte_to_char_index, char_to_byte_index}; // index conversion
use super::Editor; // editor state
use anyhow::{Context, Result}; // anyhow error handling
use std::fs; // file system access
use std::mem; // memory manipulation
use std::path::PathBuf; // file path handling
use std::time::Duration; // timing for status messages

impl Editor {
    /// Save the buffer.
    pub fn cmd_save(&mut self) -> Result<()> {
        if self.file_path.is_none() {
            self.prompt = Some(Prompt::new(PromptKind::SaveAs, ""));
            return Ok(());
        }
        self.save_to_path(self.file_path.clone().unwrap())
    }

    /// Save the buffer to a specific path.
    pub fn save_to_path(&mut self, path: PathBuf) -> Result<()> {
        let content = self.buf.to_string();
        fs::write(&path, content).with_context(|| format!("Failed writing {}", path.display()))?;
        self.file_path = Some(path.clone());
        self.dirty = false;
        self.set_status(format!("Saved: {}", path.display()), Duration::from_secs(2));

        let mut plugins = mem::take(&mut self.plugins);
        plugins.call_hook(self, Hook::OnSave, Some(&path))?;
        self.plugins = plugins;
        Ok(())
    }

    /// Open a file.
    pub fn open_path(&mut self, path: PathBuf) -> Result<()> {
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

        // Update highlighter for new file extension
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            self.highlighter.set_file_extension(ext);
        } else {
            self.highlighter.set_file_extension("");
        }

        self.ensure_visible()?;

        let mut plugins = mem::take(&mut self.plugins);
        plugins.call_hook(self, Hook::OnOpen, Some(&path))?;
        self.plugins = plugins;
        self.set_status(format!("Opened: {}", path.display()), Duration::from_secs(2));
        Ok(())
    }

    /// Find the next occurrence of query.
    pub fn find_next(&mut self, query: &str) -> Result<()> {
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

    /// Search forward for query.
    pub fn search_forward(&self, query: &str, from: Pos, wrap: bool) -> Option<Pos> {
        let mut y = from.y;
        let mut x = from.x;

        let find_in_line = |line: &str, start_char: usize| -> Option<usize> {
            let b0 = char_to_byte_index(line, start_char);
            let sub = &line[b0..];
            let idx = sub.find(query)?;
            Some(start_char + byte_to_char_index(sub, idx))
        };

        while y < self.buf.line_count() {
            let line = self.buf.line(y);
            if let Some(cx) = find_in_line(&line, x) {
                return Some(Pos { y, x: cx });
            }
            y += 1;
            x = 0;
        }

        if !wrap { return None; }

        y = 0;
        while y <= from.y && y < self.buf.line_count() {
            let line = self.buf.line(y);
            if let Some(cx) = find_in_line(&line, 0) {
                return Some(Pos { y, x: cx });
            }
            y += 1;
        }
        None
    }
}
