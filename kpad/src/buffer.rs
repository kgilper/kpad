//! The document buffer: stores text using a Rope for O(log n) operations on large files.

use crate::types::{LineEnding, Pos};
use ropey::Rope;
use std::borrow::Cow;
use std::io::{self, Write};

/// The document buffer using a Rope data structure.
///
/// A Rope provides O(log n) insert/delete operations, making it suitable for
/// files with 100,000+ lines. This replaces the previous Vec<String> implementation.
pub struct Buffer {
    /// The text content stored as a Rope.
    pub text: Rope,
    /// Line ending style for this buffer.
    pub line_ending: LineEnding,
}

impl Buffer {
    /// Create a new empty buffer with a single empty line and default to LF.
    pub fn new() -> Self {
        Self {
            text: Rope::new(),
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

        // Normalize to LF internally, store CRLF preference for saving
        let normalized = s.replace("\r\n", "\n");
        let text = Rope::from_str(&normalized);

        Self { text, line_ending }
    }

    /// Serialize the buffer for saving to disk, using the detected line ending.
    pub fn to_string(&self) -> String {
        let s: String = self.text.chars().collect();
        if self.line_ending == LineEnding::CRLF {
            s.replace('\n', "\r\n")
        } else {
            s
        }
    }

    /// Stream the buffer to a writer, avoiding full String allocation.
    /// This is more efficient for large files.
    pub fn write_to<W: Write>(&self, mut writer: W) -> io::Result<()> {
        if self.line_ending == LineEnding::CRLF {
            // Need to convert LF to CRLF while streaming
            for chunk in self.text.chunks() {
                let converted = chunk.replace('\n', "\r\n");
                writer.write_all(converted.as_bytes())?;
            }
        } else {
            // Stream chunks directly using ropey's efficient iterator
            for chunk in self.text.chunks() {
                writer.write_all(chunk.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Search for a query string starting from a char index.
    /// Returns the char index of the match, or None if not found.
    pub fn search_from(&self, query: &str, start_char_idx: usize) -> Option<usize> {
        if query.is_empty() || start_char_idx >= self.text.len_chars() {
            return None;
        }

        // Get the slice from start position to end
        let slice = self.text.slice(start_char_idx..);

        // Search through chunks, handling boundary crossings
        let query_chars: Vec<char> = query.chars().collect();
        let mut match_start: Option<usize> = None;
        let mut match_len = 0;
        let mut char_offset = 0;

        for chunk in slice.chunks() {
            for ch in chunk.chars() {
                if ch == query_chars[match_len] {
                    if match_len == 0 {
                        match_start = Some(char_offset);
                    }
                    match_len += 1;
                    if match_len == query_chars.len() {
                        return Some(start_char_idx + match_start.unwrap());
                    }
                } else if match_len > 0 {
                    // Reset and check if current char starts a new match
                    match_len = 0;
                    match_start = None;
                    if ch == query_chars[0] {
                        match_start = Some(char_offset);
                        match_len = 1;
                    }
                }
                char_offset += 1;
            }
        }
        None
    }

    /// Convert a char index to a Pos (line, column).
    pub fn char_idx_to_pos_public(&self, char_idx: usize) -> Pos {
        self.char_idx_to_pos(char_idx)
    }

    /// Convert a Pos to a char index.
    pub fn pos_to_char_idx_public(&self, p: Pos) -> usize {
        self.pos_to_char_idx(p)
    }

    /// Number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        // Rope counts trailing newline as an extra line, adjust for consistency
        let len = self.text.len_lines();
        if len == 0 { 1 } else { len }
    }

    /// Get the character count of a specific line (excluding newline).
    pub fn line_len_chars(&self, y: usize) -> usize {
        if y >= self.text.len_lines() {
            return 0;
        }
        let line = self.text.line(y);
        // Exclude trailing newline from count
        let len = line.len_chars();
        if len > 0 && line.char(len - 1) == '\n' {
            len - 1
        } else {
            len
        }
    }

    /// Get the text of a specific line (without trailing newline).
    pub fn line(&self, y: usize) -> Cow<'_, str> {
        if y >= self.text.len_lines() {
            return Cow::Borrowed("");
        }
        let line = self.text.line(y);
        let s: String = line.chars().collect();
        // Remove trailing newline
        Cow::Owned(s.trim_end_matches('\n').to_string())
    }

    /// Replace the content of a specific line.
    pub fn set_line(&mut self, y: usize, content: &str) {
        if y >= self.line_count() {
            return;
        }
        let start = self.text.line_to_char(y);
        let old_len = self.line_len_chars(y);
        // Remove old content (but not the newline if it exists)
        if old_len > 0 {
            self.text.remove(start..start + old_len);
        }
        // Insert new content
        self.text.insert(start, content);
    }

    /// Clamp a position to a valid line and a valid column within that line.
    pub fn clamp_pos(&self, mut p: Pos) -> Pos {
        let line_count = self.line_count();
        if line_count == 0 {
            return Pos { y: 0, x: 0 };
        }
        p.y = p.y.min(line_count.saturating_sub(1));
        p.x = p.x.min(self.line_len_chars(p.y));
        p
    }

    /// Convert a Pos (line, char) to a global char index in the Rope.
    fn pos_to_char_idx(&self, p: Pos) -> usize {
        if p.y >= self.text.len_lines() {
            return self.text.len_chars();
        }
        let line_start = self.text.line_to_char(p.y);
        let line_len = self.line_len_chars(p.y);
        line_start + p.x.min(line_len)
    }

    /// Convert a global char index to a Pos (line, char).
    fn char_idx_to_pos(&self, char_idx: usize) -> Pos {
        let char_idx = char_idx.min(self.text.len_chars());
        let y = self.text.char_to_line(char_idx);
        let line_start = self.text.line_to_char(y);
        let x = char_idx - line_start;
        Pos { y, x }
    }

    /// Insert a single character at a position, returning the new cursor position.
    pub fn insert_char(&mut self, p: Pos, ch: char) -> Pos {
        let idx = self.pos_to_char_idx(p);
        self.text.insert_char(idx, ch);
        if ch == '\n' {
            Pos { y: p.y + 1, x: 0 }
        } else {
            Pos { y: p.y, x: p.x + 1 }
        }
    }

    /// Insert a newline at a position, splitting the current line in two.
    pub fn insert_newline(&mut self, p: Pos) -> Pos {
        self.insert_char(p, '\n')
    }

    /// Backspace behavior:
    /// - If `x > 0`, delete the previous character.
    /// - If at start of line (`x == 0`) and not the first line, merge with previous line.
    pub fn delete_backspace(&mut self, p: Pos) -> Pos {
        let idx = self.pos_to_char_idx(p);
        if idx == 0 {
            return p;
        }

        // Check what character we're deleting
        let prev_char = self.text.char(idx - 1);

        if prev_char == '\n' {
            // Merging with previous line - calculate new cursor pos before removal
            let new_y = p.y.saturating_sub(1);
            let new_x = self.line_len_chars(new_y);
            self.text.remove(idx - 1..idx);
            Pos { y: new_y, x: new_x }
        } else {
            self.text.remove(idx - 1..idx);
            Pos { y: p.y, x: p.x - 1 }
        }
    }

    /// Delete-key behavior:
    /// - If within the line, delete the character at the cursor.
    /// - If at end of line and there is a next line, merge with next line.
    pub fn delete_delete(&mut self, p: Pos) -> Pos {
        let idx = self.pos_to_char_idx(p);
        if idx >= self.text.len_chars() {
            return p;
        }

        self.text.remove(idx..idx + 1);
        p
    }

    /// Extract a range of text as a string.
    pub fn get_range(&self, start: Pos, end: Pos) -> String {
        if start == end {
            return String::new();
        }
        let (a, b) = if start <= end { (start, end) } else { (end, start) };

        let start_idx = self.pos_to_char_idx(a);
        let end_idx = self.pos_to_char_idx(b);

        self.text.slice(start_idx..end_idx).chars().collect()
    }

    /// Delete a (start, end) range and return the new cursor position (start of the range).
    pub fn delete_range(&mut self, start: Pos, end: Pos) -> Pos {
        if start == end {
            return start;
        }
        let (a, b) = if start <= end { (start, end) } else { (end, start) };

        let start_idx = self.pos_to_char_idx(a);
        let end_idx = self.pos_to_char_idx(b);

        self.text.remove(start_idx..end_idx);
        a
    }

    /// Insert a string at a position.
    /// The string may contain newlines; they are handled correctly.
    pub fn insert_str(&mut self, p: Pos, text: &str) -> Pos {
        let normalized = text.replace("\r\n", "\n");
        let idx = self.pos_to_char_idx(p);
        self.text.insert(idx, &normalized);

        // Calculate new position
        self.char_idx_to_pos(idx + normalized.chars().count())
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
    fn new_buffer_is_empty() {
        let buf = Buffer::new();
        assert_eq!(buf.text.len_chars(), 0);
        assert_eq!(buf.line_ending, LineEnding::LF);
    }

    #[test]
    fn from_string_empty() {
        let buf = Buffer::from_string("");
        assert_eq!(buf.text.len_chars(), 0);
    }

    #[test]
    fn from_string_single_line() {
        let buf = Buffer::from_string("hello world");
        assert_eq!(buf.line(0).as_ref(), "hello world");
    }

    #[test]
    fn from_string_lf_lines() {
        let buf = Buffer::from_string("line1\nline2\nline3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line(0).as_ref(), "line1");
        assert_eq!(buf.line(1).as_ref(), "line2");
        assert_eq!(buf.line(2).as_ref(), "line3");
        assert_eq!(buf.line_ending, LineEnding::LF);
    }

    #[test]
    fn from_string_crlf_lines() {
        let buf = Buffer::from_string("line1\r\nline2\r\nline3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line(0).as_ref(), "line1");
        assert_eq!(buf.line(1).as_ref(), "line2");
        assert_eq!(buf.line(2).as_ref(), "line3");
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
        assert_eq!(buf.line(0).as_ref(), "a");
    }

    #[test]
    fn insert_char_unicode() {
        let mut buf = Buffer::from_string("hllo");
        let pos = buf.insert_char(Pos { y: 0, x: 1 }, 'é');
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.line(0).as_ref(), "héllo");
    }

    #[test]
    fn insert_char_emoji() {
        let mut buf = Buffer::from_string("ab");
        let pos = buf.insert_char(Pos { y: 0, x: 1 }, '😀');
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.line(0).as_ref(), "a😀b");
    }

    #[test]
    fn insert_newline() {
        let mut buf = Buffer::from_string("hello world");
        let pos = buf.insert_newline(Pos { y: 0, x: 5 });
        assert_eq!(pos, Pos { y: 1, x: 0 });
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.line(0).as_ref(), "hello");
        assert_eq!(buf.line(1).as_ref(), " world");
    }

    #[test]
    fn insert_str_single_line() {
        let mut buf = Buffer::from_string("ac");
        let pos = buf.insert_str(Pos { y: 0, x: 1 }, "b");
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.line(0).as_ref(), "abc");
    }

    #[test]
    fn insert_str_multiline() {
        let mut buf = Buffer::from_string("start end");
        let pos = buf.insert_str(Pos { y: 0, x: 6 }, "line1\nline2\nline3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line(0).as_ref(), "start line1");
        assert_eq!(buf.line(1).as_ref(), "line2");
        assert_eq!(buf.line(2).as_ref(), "line3end");
        assert_eq!(pos.y, 2);
    }

    // ==================== Delete tests ====================

    #[test]
    fn delete_backspace_middle() {
        let mut buf = Buffer::from_string("abc");
        let pos = buf.delete_backspace(Pos { y: 0, x: 2 });
        assert_eq!(pos, Pos { y: 0, x: 1 });
        assert_eq!(buf.line(0).as_ref(), "ac");
    }

    #[test]
    fn delete_backspace_unicode() {
        let mut buf = Buffer::from_string("héllo");
        let pos = buf.delete_backspace(Pos { y: 0, x: 2 });
        assert_eq!(pos, Pos { y: 0, x: 1 });
        assert_eq!(buf.line(0).as_ref(), "hllo");
    }

    #[test]
    fn delete_backspace_merge_lines() {
        let mut buf = Buffer::from_string("line1\nline2");
        let pos = buf.delete_backspace(Pos { y: 1, x: 0 });
        assert_eq!(pos, Pos { y: 0, x: 5 });
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0).as_ref(), "line1line2");
    }

    #[test]
    fn delete_delete_middle() {
        let mut buf = Buffer::from_string("abc");
        let pos = buf.delete_delete(Pos { y: 0, x: 1 });
        assert_eq!(pos, Pos { y: 0, x: 1 });
        assert_eq!(buf.line(0).as_ref(), "ac");
    }

    #[test]
    fn delete_delete_merge_lines() {
        let mut buf = Buffer::from_string("line1\nline2");
        let pos = buf.delete_delete(Pos { y: 0, x: 5 });
        assert_eq!(pos, Pos { y: 0, x: 5 });
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0).as_ref(), "line1line2");
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
        assert_eq!(buf.line(0).as_ref(), "hello");
    }

    #[test]
    fn delete_range_multiline() {
        let mut buf = Buffer::from_string("start\nmiddle\nend");
        let pos = buf.delete_range(Pos { y: 0, x: 3 }, Pos { y: 2, x: 1 });
        assert_eq!(pos, Pos { y: 0, x: 3 });
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0).as_ref(), "stand");
    }

    // ==================== Unicode edge cases ====================

    #[test]
    fn operations_with_cjk() {
        let mut buf = Buffer::from_string("日本語");
        assert_eq!(buf.line_len_chars(0), 3);

        let pos = buf.insert_char(Pos { y: 0, x: 1 }, '中');
        assert_eq!(pos, Pos { y: 0, x: 2 });
        assert_eq!(buf.line(0).as_ref(), "日中本語");

        buf.delete_backspace(Pos { y: 0, x: 2 });
        assert_eq!(buf.line(0).as_ref(), "日本語");
    }

    #[test]
    fn clamp_pos_works() {
        let buf = Buffer::from_string("short\nlonger line");

        let p = buf.clamp_pos(Pos { y: 100, x: 0 });
        assert_eq!(p.y, 1);

        let p = buf.clamp_pos(Pos { y: 0, x: 100 });
        assert_eq!(p.x, 5);
    }
}
