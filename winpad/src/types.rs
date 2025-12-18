//! Common types used throughout the editor.

use std::time::Instant;

/// A position in the document.
///
/// - `y`: line index (0-based)
/// - `x`: **char index** within that line (0-based). This is *not* a byte index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub y: usize,
    pub x: usize, // char index within line
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

/// An atomic edit operation in the document.
#[derive(Clone, Debug)]
pub enum EditOperation {
    /// Text was inserted at a position.
    Insert { pos: Pos, text: String },
    /// A range of text was deleted.
    /// We store the `deleted_text` so we can restore it during undo.
    Delete { start: Pos, _end: Pos, deleted_text: String },
}

/// A single entry in the undo/redo stack.
#[derive(Clone)]
pub struct UndoEntry {
    /// The operation performed.
    pub op: EditOperation,
    /// Cursor position before the operation (to restore on undo).
    pub cursor_before: Pos,
    /// Anchor position before the operation (to restore on undo).
    pub anchor_before: Option<Pos>,
}

/// The different prompt modes shown in the bottom line (open/save/find/command/goto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Open,
    SaveAs,
    Find,
    Command,
    GotoLine,
}

/// Prompt state (what the user is typing at the bottom).
#[derive(Debug, Clone)]
pub struct Prompt {
    pub kind: PromptKind,
    pub input: String,
    pub cursor: usize, // char index in input
}

impl Prompt {
    /// Create a new prompt pre-filled with `initial`.
    pub fn new(kind: PromptKind, initial: impl Into<String>) -> Self {
        let input = initial.into();
        let cursor = input.chars().count();
        Self { kind, input, cursor }
    }
}

/// Short-lived status message shown in the status bar.
#[derive(Clone)]
pub struct StatusMsg {
    pub text: String,
    pub until: Instant,
}

/// The character sequence used to separate lines in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// Unix line ending: `\n` (LF)
    LF,
    /// Windows line ending: `\r\n` (CRLF)
    CRLF,
}

impl LineEnding {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LF => "\n",
            Self::CRLF => "\r\n",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::LF => "Unix (LF)",
            Self::CRLF => "Windows (CRLF)",
        }
    }
}

/// Statistics about the current document.
pub struct DocumentStats {
    pub line_count: usize,
    pub word_count: usize,
    pub char_count: usize,
    pub byte_count: usize,
    pub line_ending: LineEnding,
    pub encoding: &'static str,
    /// Distribution of line lengths for the "chart"
    pub line_length_histogram: Vec<usize>,
}

/// Colors available for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightColor {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Grey,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
}

impl HighlightColor {
    /// Parse a color name from a string (for plugin use).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "red" => Some(Self::Red),
            "green" => Some(Self::Green),
            "yellow" => Some(Self::Yellow),
            "blue" => Some(Self::Blue),
            "magenta" | "purple" => Some(Self::Magenta),
            "cyan" => Some(Self::Cyan),
            "white" => Some(Self::White),
            "grey" | "gray" => Some(Self::Grey),
            "bright_red" | "brightred" => Some(Self::BrightRed),
            "bright_green" | "brightgreen" => Some(Self::BrightGreen),
            "bright_yellow" | "brightyellow" => Some(Self::BrightYellow),
            "bright_blue" | "brightblue" => Some(Self::BrightBlue),
            "bright_magenta" | "brightmagenta" => Some(Self::BrightMagenta),
            "bright_cyan" | "brightcyan" => Some(Self::BrightCyan),
            _ => None,
        }
    }
}

/// A syntax highlighting rule registered by a plugin.
#[derive(Debug, Clone)]
pub struct HighlightRule {
    /// Regex pattern to match.
    pub pattern: String,
    /// Color to apply to matches.
    pub color: HighlightColor,
    /// Higher priority rules override lower ones (default: 0).
    pub priority: i32,
    /// Which capture group to highlight (0 = whole match).
    pub group: usize,
}

/// A highlighted span within a line.
#[derive(Debug, Clone)]
pub struct HighlightSpan {
    /// Start char index (inclusive).
    pub start: usize,
    /// End char index (exclusive).
    pub end: usize,
    /// Color for this span.
    pub color: HighlightColor,
    /// Priority (for overlapping spans).
    pub priority: i32,
}


