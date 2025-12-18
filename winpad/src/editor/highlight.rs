//! Syntax highlighting system for plugin-registered rules.

use crate::types::{HighlightColor, HighlightRule, HighlightSpan};
use crossterm::style::Color;
use regex::Regex;
use std::collections::HashMap;

/// Convert a HighlightColor to a crossterm Color.
pub fn highlight_to_crossterm(color: HighlightColor) -> Color {
    match color {
        HighlightColor::Red => Color::Red,
        HighlightColor::Green => Color::Green,
        HighlightColor::Yellow => Color::Yellow,
        HighlightColor::Blue => Color::Blue,
        HighlightColor::Magenta => Color::Magenta,
        HighlightColor::Cyan => Color::Cyan,
        HighlightColor::White => Color::White,
        HighlightColor::Grey => Color::Grey,
        HighlightColor::BrightRed => Color::DarkRed,
        HighlightColor::BrightGreen => Color::DarkGreen,
        HighlightColor::BrightYellow => Color::DarkYellow,
        HighlightColor::BrightBlue => Color::DarkBlue,
        HighlightColor::BrightMagenta => Color::DarkMagenta,
        HighlightColor::BrightCyan => Color::DarkCyan,
    }
}

/// A compiled highlight rule ready for matching.
struct CompiledRule {
    regex: Regex,
    color: HighlightColor,
    priority: i32,
    group: usize,
}

/// Manages syntax highlighting rules registered by plugins.
#[derive(Default)]
pub struct Highlighter {
    /// Rules grouped by file extension (e.g., "md", "rs").
    /// Empty string key "" means applies to all files.
    rules_by_ext: HashMap<String, Vec<CompiledRule>>,
    /// Cache of computed highlights per line (cleared on edit).
    cache: HashMap<usize, Vec<HighlightSpan>>,
    /// Current file extension being edited.
    current_ext: String,
}

impl Highlighter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a highlight rule for a file extension.
    /// Extension should be without the dot (e.g., "md" not ".md").
    /// Use "" for rules that apply to all files.
    pub fn register_rule(&mut self, extension: &str, rule: HighlightRule) {
        let ext = extension.to_lowercase();

        // Try to compile the regex
        let regex = match Regex::new(&rule.pattern) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Invalid highlight pattern '{}': {}", rule.pattern, e);
                return;
            }
        };

        let compiled = CompiledRule {
            regex,
            color: rule.color,
            priority: rule.priority,
            group: rule.group,
        };

        self.rules_by_ext
            .entry(ext)
            .or_insert_with(Vec::new)
            .push(compiled);

        // Clear cache when rules change
        self.cache.clear();
    }

    /// Clear all rules for a specific extension.
    pub fn clear_rules(&mut self, extension: &str) {
        let ext = extension.to_lowercase();
        self.rules_by_ext.remove(&ext);
        self.cache.clear();
    }

    /// Clear all registered rules.
    pub fn clear_all_rules(&mut self) {
        self.rules_by_ext.clear();
        self.cache.clear();
    }

    /// Set the current file extension (called when opening a file).
    pub fn set_file_extension(&mut self, ext: &str) {
        let new_ext = ext.to_lowercase();
        if self.current_ext != new_ext {
            self.current_ext = new_ext;
            self.cache.clear();
        }
    }

    /// Invalidate the cache for a specific line (call after edits).
    pub fn invalidate_line(&mut self, line: usize) {
        self.cache.remove(&line);
        // Also invalidate nearby lines for multi-line constructs
        if line > 0 {
            self.cache.remove(&(line - 1));
        }
        self.cache.remove(&(line + 1));
    }

    /// Invalidate the entire cache (call after major edits).
    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }

    /// Get highlight spans for a line, using cache if available.
    pub fn get_highlights(&mut self, line_idx: usize, line_text: &str) -> Vec<HighlightSpan> {
        // Check cache first
        if let Some(spans) = self.cache.get(&line_idx) {
            return spans.clone();
        }

        // Compute highlights
        let spans = self.compute_highlights(line_text);

        // Cache the result
        self.cache.insert(line_idx, spans.clone());

        spans
    }

    /// Compute highlight spans for a line of text.
    fn compute_highlights(&self, text: &str) -> Vec<HighlightSpan> {
        let mut spans = Vec::new();

        // Get rules for current extension + global rules
        let ext_rules = self.rules_by_ext.get(&self.current_ext);
        let global_rules = self.rules_by_ext.get("");

        let rules: Vec<&CompiledRule> = ext_rules
            .into_iter()
            .flatten()
            .chain(global_rules.into_iter().flatten())
            .collect();

        if rules.is_empty() {
            return spans;
        }

        // Apply each rule
        for rule in rules {
            for caps in rule.regex.captures_iter(text) {
                let m = if rule.group == 0 {
                    caps.get(0)
                } else {
                    caps.get(rule.group)
                };

                if let Some(m) = m {
                    // Convert byte indices to char indices
                    let start = text[..m.start()].chars().count();
                    let end = text[..m.end()].chars().count();

                    spans.push(HighlightSpan {
                        start,
                        end,
                        color: rule.color,
                        priority: rule.priority,
                    });
                }
            }
        }

        // Sort by start position, then by priority (higher priority last)
        spans.sort_by(|a, b| {
            a.start.cmp(&b.start).then(a.priority.cmp(&b.priority))
        });

        spans
    }

    /// Get the color for a specific character position, considering overlapping spans.
    pub fn color_at(&self, spans: &[HighlightSpan], char_idx: usize) -> Option<HighlightColor> {
        // Find the highest priority span that contains this position
        let mut best: Option<&HighlightSpan> = None;

        for span in spans {
            if char_idx >= span.start && char_idx < span.end {
                match best {
                    None => best = Some(span),
                    Some(b) if span.priority > b.priority => best = Some(span),
                    _ => {}
                }
            }
        }

        best.map(|s| s.color)
    }

    /// Check if highlighting is active for the current file.
    pub fn is_active(&self) -> bool {
        self.rules_by_ext.contains_key(&self.current_ext)
            || self.rules_by_ext.contains_key("")
    }
}
