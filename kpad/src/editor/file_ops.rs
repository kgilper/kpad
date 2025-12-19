//! File operations: open, save, search.

use crate::buffer::Buffer; // document model
use crate::plugins::Hook; // plugin lifecycle hooks
use crate::types::{Pos, Prompt, PromptKind}; // core types
use super::Editor; // editor state
use anyhow::{Context, Result}; // anyhow error handling
use std::fs::{self, File}; // file system access and file handle
use std::io::BufWriter; // buffered writing
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
    /// Uses streaming write to avoid allocating the entire file as a String.
    pub fn save_to_path(&mut self, path: PathBuf) -> Result<()> {
        let file = File::create(&path)
            .with_context(|| format!("Failed to create {}", path.display()))?;
        let writer = BufWriter::new(file);
        self.buf.write_to(writer)
            .with_context(|| format!("Failed writing {}", path.display()))?;
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

    /// Search forward for query using optimized Rope traversal.
    /// Avoids line-by-line iteration by searching through the entire text.
    pub fn search_forward(&self, query: &str, from: Pos, wrap: bool) -> Option<Pos> {
        if query.is_empty() {
            return None;
        }

        // Convert starting position to char index
        let start_idx = self.buf.pos_to_char_idx_public(from);

        // Search from cursor to end
        if let Some(match_idx) = self.buf.search_from(query, start_idx) {
            return Some(self.buf.char_idx_to_pos_public(match_idx));
        }

        // Wrap around: search from beginning to cursor
        if wrap && start_idx > 0 {
            if let Some(match_idx) = self.buf.search_from(query, 0) {
                // Only return if match is before original position
                if match_idx < start_idx {
                    return Some(self.buf.char_idx_to_pos_public(match_idx));
                }
            }
        }

        None
    }
}
