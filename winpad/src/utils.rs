//! Utility functions.

use std::cmp::min; // comparison helpers

/// Convert a "character index" to a "byte index" in a UTF‑8 string.
///
/// Why this exists: Rust strings are UTF‑8, so you cannot safely slice with `s[a..b]` unless
/// `a` and `b` are **byte offsets** that lie on UTF‑8 character boundaries.
pub fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
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

/// Convert a byte offset back into a character index.
pub fn byte_to_char_index(s: &str, byte_idx: usize) -> usize {
    s[..min(byte_idx, s.len())].chars().count()
}

/// Number of decimal digits in `n` (used to size the line-number gutter).
pub fn digits(n: usize) -> usize {
    n.to_string().len()
}

/// Clamp an `isize` (which may be negative) into a `[lo, hi]` range and return `usize`.
pub fn clamp_usize(v: isize, lo: usize, hi: usize) -> usize {
    if v < lo as isize {
        lo
    } else if v > hi as isize {
        hi
    } else {
        v as usize
    }
}

/// Clamp an `i64` (which may be negative) into a `[lo, hi]` range and return `usize`.
pub fn clamp_usize_i64(v: i64, lo: usize, hi: usize) -> usize {
    if v < lo as i64 {
        lo
    } else if v > hi as i64 {
        hi
    } else {
        v as usize
    }
}

/// Get the default plugin search directories.
///
/// Returns:
/// - `./plugins` relative to the current working directory
/// - `plugins/` next to the executable (useful for distributing a folder)
pub fn default_plugin_dirs() -> anyhow::Result<Vec<std::path::PathBuf>> {
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

/// Calculate the Levenshtein distance between two strings.
/// This is used for "did you mean?" suggestions for unknown commands.
pub fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();
    if len1 == 0 { return len2; }
    if len2 == 0 { return len1; }

    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    for i in 0..=len1 { matrix[i][0] = i; }
    for j in 0..=len2 { matrix[0][j] = j; }

    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();

    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = if s1_chars[i - 1] == s2_chars[j - 1] { 0 } else { 1 };
            matrix[i][j] = min(
                matrix[i - 1][j] + 1,
                min(
                    matrix[i][j - 1] + 1,
                    matrix[i - 1][j - 1] + cost
                )
            );
        }
    }

    matrix[len1][len2]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== char_to_byte_index tests ====================

    #[test]
    fn char_to_byte_ascii() {
        let s = "hello";
        assert_eq!(char_to_byte_index(s, 0), 0);
        assert_eq!(char_to_byte_index(s, 1), 1);
        assert_eq!(char_to_byte_index(s, 5), 5);
    }

    #[test]
    fn char_to_byte_unicode() {
        // "héllo" - 'é' is 2 bytes in UTF-8
        let s = "héllo";
        assert_eq!(char_to_byte_index(s, 0), 0); // 'h'
        assert_eq!(char_to_byte_index(s, 1), 1); // 'é' starts at byte 1
        assert_eq!(char_to_byte_index(s, 2), 3); // 'l' starts at byte 3 (after 2-byte é)
        assert_eq!(char_to_byte_index(s, 3), 4); // 'l'
        assert_eq!(char_to_byte_index(s, 4), 5); // 'o'
    }

    #[test]
    fn char_to_byte_emoji() {
        // Emoji are typically 4 bytes in UTF-8
        let s = "a😀b";
        assert_eq!(char_to_byte_index(s, 0), 0); // 'a'
        assert_eq!(char_to_byte_index(s, 1), 1); // '😀' starts at byte 1
        assert_eq!(char_to_byte_index(s, 2), 5); // 'b' starts at byte 5 (after 4-byte emoji)
    }

    #[test]
    fn char_to_byte_cjk() {
        // CJK characters are 3 bytes each
        let s = "日本語";
        assert_eq!(char_to_byte_index(s, 0), 0);
        assert_eq!(char_to_byte_index(s, 1), 3);
        assert_eq!(char_to_byte_index(s, 2), 6);
        assert_eq!(char_to_byte_index(s, 3), 9); // end of string
    }

    #[test]
    fn char_to_byte_beyond_end() {
        let s = "abc";
        assert_eq!(char_to_byte_index(s, 10), 3); // clamps to string length
    }

    #[test]
    fn char_to_byte_empty() {
        let s = "";
        assert_eq!(char_to_byte_index(s, 0), 0);
        assert_eq!(char_to_byte_index(s, 5), 0);
    }

    // ==================== byte_to_char_index tests ====================

    #[test]
    fn byte_to_char_ascii() {
        let s = "hello";
        assert_eq!(byte_to_char_index(s, 0), 0);
        assert_eq!(byte_to_char_index(s, 3), 3);
        assert_eq!(byte_to_char_index(s, 5), 5);
    }

    #[test]
    fn byte_to_char_unicode() {
        let s = "héllo"; // é is 2 bytes
        assert_eq!(byte_to_char_index(s, 0), 0); // before 'h'
        assert_eq!(byte_to_char_index(s, 1), 1); // after 'h', before 'é'
        assert_eq!(byte_to_char_index(s, 3), 2); // after 'é', before 'l'
    }

    #[test]
    fn byte_to_char_beyond_end() {
        let s = "abc";
        assert_eq!(byte_to_char_index(s, 100), 3);
    }

    // ==================== roundtrip tests ====================

    #[test]
    fn roundtrip_char_byte_char() {
        let s = "héllo 日本語 😀";
        for i in 0..=s.chars().count() {
            let byte_idx = char_to_byte_index(s, i);
            let char_idx = byte_to_char_index(s, byte_idx);
            assert_eq!(char_idx, i, "roundtrip failed for char index {}", i);
        }
    }

    // ==================== other utils tests ====================

    #[test]
    fn test_digits() {
        assert_eq!(digits(0), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(digits(99), 2);
        assert_eq!(digits(100), 3);
        assert_eq!(digits(1000), 4);
    }

    #[test]
    fn test_clamp_usize() {
        assert_eq!(clamp_usize(-5, 0, 10), 0);
        assert_eq!(clamp_usize(5, 0, 10), 5);
        assert_eq!(clamp_usize(15, 0, 10), 10);
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("save", "dave"), 1);
    }
}

