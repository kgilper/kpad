//! The document buffer: stores lines of text and provides editing operations.

use crate::types::{LineEnding, Pos}; // core editor types
use crate::utils::char_to_byte_index; // utf-8 index conversion
use std::cmp::min; // comparison helpers

/// The document buffer: a list of lines (each line is a `String`).
///
/// This is intentionally simple: for a production editor you'd use a rope or piece table for
/// better performance on large files, but for this project a `Vec<String>` is fine.
pub struct Buffer {
    pub lines: Vec<String>,
    pub line_ending: LineEnding,
}

impl Buffer {
    /// Create a new empty buffer with a single empty line and default to LF.
    pub fn new() -> Self {
        Self { 
            lines: vec![String::new()],
            line_ending: LineEnding::LF,
        }
    }

    /// Build a buffer from an on-disk string, detecting and honoring line endings.
    pub fn from_string(s: &str) -> Self {
        // Detect line ending by looking for the first \r\n
        let line_ending = if s.contains("\r\n") {
            LineEnding::CRLF
        } else {
            LineEnding::LF
        };

        let mut lines: Vec<String> = s
            .split('\n')
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect();
            
        if lines.is_empty() {
            lines.push(String::new());
        }
        
        Self { lines, line_ending }
    }

    /// Serialize the buffer for saving to disk, using the detected line ending.
    pub fn to_string(&self) -> String {
        self.lines.join(self.line_ending.as_str())
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_len_chars(&self, y: usize) -> usize {
        self.lines.get(y).map(|l| l.chars().count()).unwrap_or(0)
    }

    /// Clamp a position to a valid line and a valid column within that line.
    pub fn clamp_pos(&self, mut p: Pos) -> Pos {
        if self.lines.is_empty() {
            return Pos { y: 0, x: 0 };
        }
        p.y = min(p.y, self.lines.len().saturating_sub(1));
        p.x = min(p.x, self.line_len_chars(p.y));
        p
    }

    /// Insert a single character at a position, returning the new cursor position.
    pub fn insert_char(&mut self, p: Pos, ch: char) -> Pos {
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

    /// Insert a newline at a position, splitting the current line in two.
    pub fn insert_newline(&mut self, p: Pos) -> Pos {
        let y = p.y;
        let x = p.x;
        let line = &mut self.lines[y];
        let bi = char_to_byte_index(line, x);
        let rest = line.split_off(bi);
        self.lines.insert(y + 1, rest);
        Pos { y: y + 1, x: 0 }
    }

    /// Backspace behavior:
    /// - If `x > 0`, delete the previous character.
    /// - If at start of line (`x == 0`) and not the first line, merge with previous line.
    pub fn delete_backspace(&mut self, p: Pos) -> Pos {
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

    /// Delete-key behavior:
    /// - If within the line, delete the character at the cursor.
    /// - If at end of line and there is a next line, merge with next line.
    pub fn delete_delete(&mut self, p: Pos) -> Pos {
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

    /// Extract a range of text as a string.
    pub fn get_range(&self, start: Pos, end: Pos) -> String {
        if start == end {
            return String::new();
        }
        let (a, b) = if start <= end { (start, end) } else { (end, start) };

        if a.y == b.y {
            let line = &self.lines[a.y];
            let b0 = char_to_byte_index(line, a.x);
            let b1 = char_to_byte_index(line, b.x);
            return line[b0..b1].to_string();
        }

        let mut out = String::new();
        // first line
        {
            let line = &self.lines[a.y];
            let b0 = char_to_byte_index(line, a.x);
            out.push_str(&line[b0..]);
            out.push('\n');
        }
        // middle lines
        for y in (a.y + 1)..b.y {
            out.push_str(&self.lines[y]);
            out.push('\n');
        }
        // last line
        {
            let line = &self.lines[b.y];
            let b1 = char_to_byte_index(line, b.x);
            out.push_str(&line[..b1]);
        }
        out
    }

    /// Delete a (start, end) range and return the new cursor position (start of the range).
    ///
    /// This is used for "delete selection" and for cut/replace operations.
    pub fn delete_range(&mut self, start: Pos, end: Pos) -> Pos {
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

    /// Insert a string at a position.
    ///
    /// The string may contain newlines; we split them into multiple lines.
    pub fn insert_str(&mut self, p: Pos, text: &str) -> Pos {
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

    /// Calculate the end position if `text` was inserted at `p`.
    pub fn calc_end_pos(&self, p: Pos, text: &str) -> Pos {
        let normalized = text.replace("\r\n", "\n");
        let parts: Vec<&str> = normalized.split('\n').collect();
        if parts.len() == 1 {
            return Pos { y: p.y, x: p.x + parts[0].chars().count() };
        }
        Pos {
            y: p.y + parts.len() - 1,
            x: parts[parts.len() - 1].chars().count(),
        }
    }
}

