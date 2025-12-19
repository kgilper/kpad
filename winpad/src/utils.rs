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


