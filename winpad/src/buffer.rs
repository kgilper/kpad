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

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Buffer creation tests ====================

    #[test]
    fn new_buffer_has_one_empty_line() {
        let buf = Buffer::new();
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.lines[0], "");
        assert_eq!(buf.line_ending, LineEnding::LF);
    }

    #[test]
    fn from_string_empty() {
        let buf = Buffer::from_string("");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.lines[0], "");
    }

    #[test]
    fn from_string_single_line() {
        let buf = Buffer::from_string("hello world");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.lines[0], "hello world");
    }

    #[test]
    fn from_string_lf_lines() {
        let buf = Buffer::from_string("line1\nline2\nline3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.lines[0], "line1");
        assert_eq!(buf.lines[1], "line2");
        assert_eq!(buf.lines[2], "line3");
        assert_eq!(buf.line_ending, LineEnding::LF);
    }

    #[test]
    fn from_string_crlf_lines() {
        let buf = Buffer::from_string("line1\r\nline2\r\nline3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.lines[0], "line1");
        assert_eq!(buf.lines[1], "line2");
        assert_eq!(buf.lines[2], "line3");
        assert_eq!(buf.line_ending, LineEnding::CRLF);
    }

    #[test]
    fn to_string_preserves_line_ending() {
        let buf_lf = Buffer::from_string("a\nb");
        assert_eq!(buf_lf.to_string(), "a\nb");

        let buf_crlf = Buffer::from_string("a\r\nb");
        assert_eq!(buf_crlf.to_string(), "a\r\nb");
    }

    // ==================== Insert tests ====================

    #[test]
    fn insert_char_ascii() {
        let mut buf = Buffer::new();
        let pos = buf.insert_char(Pos { y: 0, x: 0 }, 'a');
        assert_eq!(pos, Pos { y: 0, x: 1 });
        assert_eq!(buf.lines[0], "a");
    }

    #[test]
    fn insert_char_unicode() {
        let mut buf = Buffer::from_string("hllo");
        // Insert 'é' at position 1 to make "héllo"
        let pos = buf.insert_char(Pos { y: 0, x: 1 }, 'é');
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.lines[0], "héllo");
    }

    #[test]
    fn insert_char_emoji() {
        let mut buf = Buffer::from_string("ab");
        let pos = buf.insert_char(Pos { y: 0, x: 1 }, '😀');
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.lines[0], "a😀b");
    }

    #[test]
    fn insert_newline() {
        let mut buf = Buffer::from_string("hello world");
        let pos = buf.insert_newline(Pos { y: 0, x: 5 });
        assert_eq!(pos, Pos { y: 1, x: 0 });
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.lines[0], "hello");
        assert_eq!(buf.lines[1], " world");
    }

    #[test]
    fn insert_str_single_line() {
        let mut buf = Buffer::from_string("ac");
        let pos = buf.insert_str(Pos { y: 0, x: 1 }, "b");
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.lines[0], "abc");
    }

    #[test]
    fn insert_str_multiline() {
        let mut buf = Buffer::from_string("start end");
        let pos = buf.insert_str(Pos { y: 0, x: 6 }, "line1\nline2\nline3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.lines[0], "start line1");
        assert_eq!(buf.lines[1], "line2");
        assert_eq!(buf.lines[2], "line3end");
        assert_eq!(pos.y, 2);
    }

    // ==================== Delete tests ====================

    #[test]
    fn delete_backspace_middle() {
        let mut buf = Buffer::from_string("abc");
        let pos = buf.delete_backspace(Pos { y: 0, x: 2 });
        assert_eq!(pos, Pos { y: 0, x: 1 });
        assert_eq!(buf.lines[0], "ac");
    }

    #[test]
    fn delete_backspace_unicode() {
        let mut buf = Buffer::from_string("héllo");
        // Delete the 'é' (at char index 1)
        let pos = buf.delete_backspace(Pos { y: 0, x: 2 });
        assert_eq!(pos, Pos { y: 0, x: 1 });
        assert_eq!(buf.lines[0], "hllo");
    }

    #[test]
    fn delete_backspace_merge_lines() {
        let mut buf = Buffer::from_string("line1\nline2");
        let pos = buf.delete_backspace(Pos { y: 1, x: 0 });
        assert_eq!(pos, Pos { y: 0, x: 5 });
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.lines[0], "line1line2");
    }

    #[test]
    fn delete_delete_middle() {
        let mut buf = Buffer::from_string("abc");
        let pos = buf.delete_delete(Pos { y: 0, x: 1 });
        assert_eq!(pos, Pos { y: 0, x: 1 });
        assert_eq!(buf.lines[0], "ac");
    }

    #[test]
    fn delete_delete_merge_lines() {
        let mut buf = Buffer::from_string("line1\nline2");
        let pos = buf.delete_delete(Pos { y: 0, x: 5 });
        assert_eq!(pos, Pos { y: 0, x: 5 });
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.lines[0], "line1line2");
    }

    // ==================== Range operations tests ====================

    #[test]
    fn get_range_same_line() {
        let buf = Buffer::from_string("hello world");
        let text = buf.get_range(Pos { y: 0, x: 0 }, Pos { y: 0, x: 5 });
        assert_eq!(text, "hello");
    }

    #[test]
    fn get_range_multiline() {
        let buf = Buffer::from_string("line1\nline2\nline3");
        let text = buf.get_range(Pos { y: 0, x: 3 }, Pos { y: 2, x: 3 });
        assert_eq!(text, "e1\nline2\nlin");
    }

    #[test]
    fn delete_range_same_line() {
        let mut buf = Buffer::from_string("hello world");
        let pos = buf.delete_range(Pos { y: 0, x: 5 }, Pos { y: 0, x: 11 });
        assert_eq!(pos, Pos { y: 0, x: 5 });
        assert_eq!(buf.lines[0], "hello");
    }

    #[test]
    fn delete_range_multiline() {
        let mut buf = Buffer::from_string("start\nmiddle\nend");
        let pos = buf.delete_range(Pos { y: 0, x: 3 }, Pos { y: 2, x: 1 });
        assert_eq!(pos, Pos { y: 0, x: 3 });
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.lines[0], "stand");
    }

    // ==================== Unicode edge cases ====================

    #[test]
    fn operations_with_cjk() {
        let mut buf = Buffer::from_string("日本語");
        assert_eq!(buf.line_len_chars(0), 3);

        // Insert at middle
        let pos = buf.insert_char(Pos { y: 0, x: 1 }, '中');
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.lines[0], "日中本語");

        // Delete
        buf.delete_backspace(Pos { y: 0, x: 2 });
        assert_eq!(buf.lines[0], "日本語");
    }

    #[test]
    fn clamp_pos_works() {
        let buf = Buffer::from_string("short\nlonger line");

        // Beyond last line
        let p = buf.clamp_pos(Pos { y: 100, x: 0 });
        assert_eq!(p.y, 1);

        // Beyond line length
        let p = buf.clamp_pos(Pos { y: 0, x: 100 });
        assert_eq!(p.x, 5);
    }
}
