//! Full-screen overlays: help screen, statistics screen.

use crate::types::{DocumentStats, LineEnding};
use super::Editor;
use anyhow::Result;
use crossterm::{
    cursor,
    style::{self, Color},
    terminal::{self, ClearType},
    QueueableCommand,
};
use std::io::{Stdout, Write};

impl Editor {
    /// Calculate document statistics.
    pub fn calculate_stats(&self) -> DocumentStats {
        let mut word_count = 0;
        let mut char_count = 0;
        let mut byte_count = 0;
        let mut histogram = vec![0; 10];

        for line in &self.buf.lines {
            char_count += line.chars().count();
            byte_count += line.len();
            word_count += line.split_whitespace().count();
            let bucket = (line.chars().count() / 10).min(9);
            histogram[bucket] += 1;
        }

        let le_len = self.buf.line_ending.as_str().len();
        if self.buf.line_count() > 1 {
            byte_count += (self.buf.line_count() - 1) * le_len;
        }

        DocumentStats {
            line_count: self.buf.line_count(),
            word_count,
            char_count,
            byte_count,
            line_ending: self.buf.line_ending,
            encoding: "UTF-8 (Unicode)",
            line_length_histogram: histogram,
        }
    }

    /// Render the document statistics screen.
    pub fn render_stats(&mut self, stdout: &mut Stdout) -> Result<()> {
        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;
        let stats = self.calculate_stats();

        stdout.queue(cursor::Hide)?;
        stdout.queue(style::SetBackgroundColor(Color::DarkMagenta))?;
        stdout.queue(style::SetForegroundColor(Color::White))?;
        stdout.queue(terminal::Clear(ClearType::All))?;

        let mut lines = vec![
            " DOCUMENT STATISTICS ".to_string(),
            "=====================".to_string(),
            "".to_string(),
            format!("  Lines:      {}", stats.line_count),
            format!("  Words:      {}", stats.word_count),
            format!("  Characters: {}", stats.char_count),
            format!("  File Size:  {} bytes", stats.byte_count),
            format!("  End of Line: {} ({})", stats.line_ending.name(), stats.line_ending.as_str().escape_debug()),
            format!("  Encoding:   {}", stats.encoding),
            "".to_string(),
            " LINE LENGTH DISTRIBUTION: ".to_string(),
        ];

        let max_val = *stats.line_length_histogram.iter().max().unwrap_or(&1).max(&1);
        let chart_width = 30;
        for (i, &count) in stats.line_length_histogram.iter().enumerate() {
            let label = if i == 9 { "90+ ".to_string() } else { format!("{:>2}-{} ", i * 10, (i + 1) * 10 - 1) };
            let bar_len = (count * chart_width) / max_val;
            let bar = "█".repeat(bar_len);
            lines.push(format!("  {} {:<30} ({})", label, bar, count));
        }

        lines.push("".to_string());
        lines.push(" Press any key to close... ".to_string());

        let start_y = (height.saturating_sub(lines.len())) / 2;
        for (i, line) in lines.iter().enumerate() {
            let x = (width.saturating_sub(line.chars().count())) / 2;
            stdout.queue(cursor::MoveTo(x as u16, (start_y + i) as u16))?;
            stdout.queue(style::Print(line))?;
        }

        stdout.flush()?;
        Ok(())
    }

    /// Render the help screen.
    pub fn render_help(&mut self, stdout: &mut Stdout) -> Result<()> {
        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;

        stdout.queue(cursor::Hide)?;
        stdout.queue(style::SetBackgroundColor(Color::DarkBlue))?;
        stdout.queue(style::SetForegroundColor(Color::White))?;
        stdout.queue(terminal::Clear(ClearType::All))?;

        let help_text = vec![
            " KPAD HELP — Keybindings and Modifiers ",
            "========================================",
            "",
            " NAVIGATION:",
            "  Arrows          Move cursor by 1 character / 1 line",
            "  Ctrl + Left     Jump to previous word or punctuation",
            "  Ctrl + Right    Jump to next word or punctuation",
            "  Home / End      Jump to top / bottom of document",
            "  PageUp / Down   Move up / down one full screen",
            "",
            " SELECTION:",
            "  Shift + Arrows  Select text while moving",
            "  Ctrl + A        Select All",
            "",
            " EDITING:",
            "  Ctrl + S        Save file",
            "  Ctrl + O        Open file",
            "  Ctrl + Z        Undo",
            "  Ctrl + Y        Redo",
            "  Ctrl + C / X    Copy / Cut selection",
            "  Ctrl + V        Paste",
            "  Tab             Insert 4 spaces",
            "",
            " SYSTEM:",
            "  Ctrl + P        Command Palette (Discovery)",
            "  Alt + W         Toggle Word Wrap",
            "  Ctrl + Q        Quit (asks if unsaved)",
            "  F1 / :help      Toggle this Help screen",
            "",
            " Press any key to close help...",
        ];

        let start_y = (height.saturating_sub(help_text.len())) / 2;
        for (i, line) in help_text.iter().enumerate() {
            let x = (width.saturating_sub(line.len())) / 2;
            stdout.queue(cursor::MoveTo(x as u16, (start_y + i) as u16))?;
            stdout.queue(style::Print(line))?;
        }

        stdout.flush()?;
        Ok(())
    }
}
