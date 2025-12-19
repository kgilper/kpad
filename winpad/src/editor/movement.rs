//! Cursor movement and boundary detection.

use crate::types::Pos; // document position type
use super::Editor; // main editor logic
use anyhow::Result; // anyhow error handling
use crossterm::{event::{KeyCode, KeyEvent, KeyModifiers}, terminal}; // terminal events and manipulation
use std::cmp::min; // comparison helpers

impl Editor {
    /// Move the cursor in response to a movement key.
    ///
    /// If `selecting` is true (Shift held), we create/extend a selection. Otherwise we clear
    /// the selection as we move.
    pub fn move_cursor(&mut self, key: KeyEvent, selecting: bool) -> Result<()> {
        if selecting && self.anchor.is_none() {
            self.anchor = Some(self.cursor);
            self.mark_redraw();
        }
        if !selecting {
            self.clear_selection();
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
}
