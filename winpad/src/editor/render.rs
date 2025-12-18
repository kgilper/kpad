//! Rendering: drawing the editor UI to the terminal.

use crate::types::{HighlightColor, PromptKind};
use crate::utils::digits;
use super::Editor;
use anyhow::Result;
use crossterm::{
    cursor,
    style::{self, Attribute, Color},
    terminal::{self, ClearType},
    QueueableCommand,
};
use std::cmp::max;
use std::io::{Stdout, Write};
use unicode_width::UnicodeWidthChar;

/// Convert a HighlightColor to crossterm Color.
fn highlight_to_crossterm(color: HighlightColor) -> Color {
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

impl Editor {
    /// Render the entire UI.
    pub fn render(&mut self, stdout: &mut Stdout) -> Result<()> {
        if !self.needs_redraw { return Ok(()); }
        self.needs_redraw = false;

        if self.show_help { return self.render_help(stdout); }
        if self.show_stats { return self.render_stats(stdout); }

        let (w, h) = terminal::size()?;
        let width = w as usize;
        let height = h as usize;

        let lnw = max(2, digits(self.buf.line_count()));
        let gutter = lnw + 2;

        let has_prompt = self.prompt.is_some();
        let editor_h = height.saturating_sub(1 + if has_prompt { 1 } else { 0 });
        let prompt_y = if has_prompt { editor_h } else { 0 };
        let status_y = height.saturating_sub(1);

        stdout.queue(cursor::Hide)?;
        stdout.queue(style::ResetColor)?;

        let rows_rendered = if self.word_wrap {
            self.render_lines_wrapped(stdout, width, editor_h, gutter)?
        } else {
            self.render_lines_normal(stdout, width, editor_h, gutter)?
        };

        for row in rows_rendered..editor_h {
            stdout.queue(cursor::MoveTo(0, row as u16))?;
            stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
            stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
            stdout.queue(style::Print("~"))?;
            stdout.queue(style::ResetColor)?;
        }

        if let Some(p) = &self.prompt {
            self.render_prompt(stdout, prompt_y, editor_h, width)?;
            stdout.queue(cursor::MoveTo(0, prompt_y as u16))?;
            stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
            stdout.queue(style::SetForegroundColor(Color::Yellow))?;
            let label = match p.kind {
                PromptKind::Open => "Open: ",
                PromptKind::SaveAs => "Save as: ",
                PromptKind::Find => "Find: ",
                PromptKind::Command => "Command: ",
                PromptKind::GotoLine => "Goto line: ",
            };
            stdout.queue(style::Print(label))?;
            stdout.queue(style::ResetColor)?;
            stdout.queue(style::Print(&p.input))?;
        }

        self.render_status_bar(stdout, status_y, width)?;

        let (cursor_x, cursor_y) = self.calculate_cursor_position(width, gutter, editor_h)?;
        let final_x = cursor_x.min(width.saturating_sub(1));
        let final_y = cursor_y.min(editor_h.saturating_sub(1));

        stdout.queue(cursor::MoveTo(final_x as u16, final_y as u16))?;
        stdout.queue(cursor::Show)?;
        stdout.flush()?;
        Ok(())
    }

    fn render_lines_normal(&mut self, stdout: &mut Stdout, width: usize, editor_h: usize, gutter: usize) -> Result<usize> {
        let lnw = gutter - 2;
        let avail = width.saturating_sub(gutter);

        for row in 0..editor_h {
            let y = self.scroll_y + row;
            stdout.queue(cursor::MoveTo(0, row as u16))?;
            stdout.queue(terminal::Clear(ClearType::CurrentLine))?;

            if y >= self.buf.line_count() {
                stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                stdout.queue(style::Print("~"))?;
                stdout.queue(style::ResetColor)?;
                continue;
            }

            let is_current_line = y == self.cursor.y;
            let base_bg = if is_current_line { Some(Color::DarkBlue) } else { None };

            if let Some(bg) = base_bg { stdout.queue(style::SetBackgroundColor(bg))?; }
            stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
            stdout.queue(style::Print(format!("{:>width$}", y + 1, width = lnw)))?;
            stdout.queue(style::Print("│ "))?;
            stdout.queue(style::ResetColor)?;

            self.render_line_content(stdout, y, avail, base_bg)?;

            if is_current_line {
                let line = &self.buf.lines[y];
                let chars: Vec<char> = line.chars().skip(self.scroll_x).collect();
                let col_used: usize = chars.iter().take_while(|_| true)
                    .map(|ch| UnicodeWidthChar::width(*ch).unwrap_or(1))
                    .take_while({ let mut sum = 0; move |w| { sum += w; sum <= avail }})
                    .sum();
                if col_used < avail {
                    stdout.queue(style::SetBackgroundColor(Color::DarkBlue))?;
                    stdout.queue(style::Print(" ".repeat(avail - col_used)))?;
                    stdout.queue(style::ResetColor)?;
                }
            }

            self.render_scroll_indicator(stdout, row, width, editor_h)?;
        }
        Ok(editor_h)
    }

    fn render_lines_wrapped(&mut self, stdout: &mut Stdout, width: usize, editor_h: usize, gutter: usize) -> Result<usize> {
        let lnw = gutter - 2;
        let avail = width.saturating_sub(gutter);
        let mut rows_rendered = 0;
        let mut current_screen_row = 0;

        let line_count = self.buf.line_count();
        for line_idx in 0..line_count {
            let line = self.buf.lines[line_idx].clone();
            let segments = self.calculate_wrap_segments(&line, avail);

            for (seg_idx, &start_char_idx) in segments.iter().enumerate() {
                if current_screen_row >= self.scroll_y && rows_rendered < editor_h {
                    let screen_row = rows_rendered;
                    stdout.queue(cursor::MoveTo(0, screen_row as u16))?;
                    stdout.queue(terminal::Clear(ClearType::CurrentLine))?;

                    let is_current_line = line_idx == self.cursor.y;
                    let base_bg = if is_current_line { Some(Color::DarkBlue) } else { None };

                    if let Some(bg) = base_bg { stdout.queue(style::SetBackgroundColor(bg))?; }
                    stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
                    if seg_idx == 0 {
                        stdout.queue(style::Print(format!("{:>width$}", line_idx + 1, width = lnw)))?;
                    } else {
                        stdout.queue(style::Print(" ".repeat(lnw)))?;
                    }
                    stdout.queue(style::Print("│ "))?;
                    stdout.queue(style::ResetColor)?;

                    self.render_wrapped_segment(stdout, line_idx, &line, start_char_idx, avail, base_bg)?;

                    if is_current_line {
                        let chars: Vec<char> = line.chars().skip(start_char_idx).collect();
                        let mut col_used = 0;
                        for ch in chars { let w = UnicodeWidthChar::width(ch).unwrap_or(1); if col_used + w > avail { break; } col_used += w; }
                        if col_used < avail {
                            stdout.queue(style::SetBackgroundColor(Color::DarkBlue))?;
                            stdout.queue(style::Print(" ".repeat(avail - col_used)))?;
                            stdout.queue(style::ResetColor)?;
                        }
                    }

                    self.render_scroll_indicator(stdout, screen_row, width, editor_h)?;
                    rows_rendered += 1;
                }
                current_screen_row += 1;
            }
            if rows_rendered >= editor_h { break; }
        }
        Ok(rows_rendered)
    }

    fn calculate_wrap_segments(&self, line: &str, avail: usize) -> Vec<usize> {
        let mut segments = Vec::new();
        if line.is_empty() { segments.push(0); return segments; }
        let mut current_col = 0;
        let mut start_idx = 0;
        for (i, ch) in line.chars().enumerate() {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
            if current_col + ch_w > avail { segments.push(start_idx); start_idx = i; current_col = 0; }
            current_col += ch_w;
        }
        segments.push(start_idx);
        segments
    }

    fn render_wrapped_segment(&mut self, stdout: &mut Stdout, line_idx: usize, line: &str, start_char_idx: usize, avail: usize, base_bg: Option<Color>) -> Result<()> {
        let sel = self.selection_range();

        // Get syntax highlights for this line
        let highlights = self.highlighter.get_highlights(line_idx, line);

        let chars: Vec<char> = line.chars().skip(start_char_idx).collect();
        let mut col_used = 0;
        let mut seg_char_i = start_char_idx;

        for ch in chars {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
            if col_used + ch_w > avail { break; }

            let selected = self.is_char_selected(sel, line_idx, seg_char_i);

            // Determine color: selection overrides syntax highlighting
            if selected {
                stdout.queue(style::SetForegroundColor(Color::Black))?;
                stdout.queue(style::SetBackgroundColor(Color::Grey))?;
                stdout.queue(style::SetAttribute(Attribute::Bold))?;
            } else {
                // Check for syntax highlight color
                let hl_color = self.highlighter.color_at(&highlights, seg_char_i);
                if let Some(bg) = base_bg { stdout.queue(style::SetBackgroundColor(bg))?; }
                if let Some(hc) = hl_color {
                    stdout.queue(style::SetForegroundColor(highlight_to_crossterm(hc)))?;
                } else {
                    stdout.queue(style::SetForegroundColor(Color::Reset))?;
                }
                stdout.queue(style::SetAttribute(Attribute::Reset))?;
            }

            stdout.queue(style::Print(ch))?;
            stdout.queue(style::ResetColor)?;

            col_used += ch_w;
            seg_char_i += 1;
        }
        Ok(())
    }

    fn render_line_content(&mut self, stdout: &mut Stdout, y: usize, avail: usize, base_bg: Option<Color>) -> Result<()> {
        let line = self.buf.lines[y].clone();
        let sel = self.selection_range();

        // Get syntax highlights for this line
        let highlights = self.highlighter.get_highlights(y, &line);

        let mut col_used = 0;
        let mut char_i = self.scroll_x;
        let chars: Vec<char> = if self.scroll_x < line.chars().count() { line.chars().skip(self.scroll_x).collect() } else { vec![] };

        for ch in chars {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
            if col_used + ch_w > avail { break; }

            let selected = self.is_char_selected(sel, y, char_i);

            // Determine color: selection overrides syntax highlighting
            if selected {
                stdout.queue(style::SetForegroundColor(Color::Black))?;
                stdout.queue(style::SetBackgroundColor(Color::Grey))?;
                stdout.queue(style::SetAttribute(Attribute::Bold))?;
            } else {
                // Check for syntax highlight color
                let hl_color = self.highlighter.color_at(&highlights, char_i);
                if let Some(bg) = base_bg { stdout.queue(style::SetBackgroundColor(bg))?; }
                if let Some(hc) = hl_color {
                    stdout.queue(style::SetForegroundColor(highlight_to_crossterm(hc)))?;
                } else {
                    stdout.queue(style::SetForegroundColor(Color::Reset))?;
                }
                stdout.queue(style::SetAttribute(Attribute::Reset))?;
            }

            stdout.queue(style::Print(ch))?;
            stdout.queue(style::ResetColor)?;

            col_used += ch_w;
            char_i += 1;
        }
        Ok(())
    }

    fn is_char_selected(&self, sel: Option<(crate::types::Pos, crate::types::Pos)>, y: usize, char_i: usize) -> bool {
        if let Some((a, b)) = sel {
            if y < a.y || y > b.y { false }
            else if y == a.y && y == b.y { char_i >= a.x && char_i < b.x }
            else if y == a.y { char_i >= a.x }
            else if y == b.y { char_i < b.x }
            else { true }
        } else { false }
    }

    fn render_scroll_indicator(&self, stdout: &mut Stdout, row: usize, width: usize, editor_h: usize) -> Result<()> {
        let total_lines = self.buf.line_count();
        let thumb_size = max(1, (editor_h * editor_h) / max(1, total_lines));
        let thumb_start = (self.scroll_y * editor_h) / max(1, total_lines);
        let thumb_end = thumb_start + thumb_size;

        stdout.queue(cursor::MoveTo((width - 1) as u16, row as u16))?;
        if row >= thumb_start && row < thumb_end {
            stdout.queue(style::SetForegroundColor(Color::White))?;
            stdout.queue(style::Print("█"))?;
        } else {
            stdout.queue(style::SetForegroundColor(Color::DarkGrey))?;
            stdout.queue(style::Print("│"))?;
        }
        stdout.queue(style::ResetColor)?;
        Ok(())
    }

    fn render_prompt(&self, stdout: &mut Stdout, prompt_y: usize, editor_h: usize, width: usize) -> Result<()> {
        if let Some(p) = &self.prompt {
            if p.kind == PromptKind::Command {
                let hits = self.commands.search(p.input.trim(), 10);
                if !hits.is_empty() {
                    let list_h = hits.len();
                    let start_y = prompt_y.saturating_sub(list_h);
                    for (i, cmd) in hits.iter().enumerate() {
                        let row = start_y + i;
                        if row >= editor_h { continue; }
                        stdout.queue(cursor::MoveTo(0, row as u16))?;
                        stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
                        stdout.queue(style::SetBackgroundColor(Color::AnsiValue(235)))?;
                        stdout.queue(style::SetForegroundColor(Color::Yellow))?;
                        stdout.queue(style::Print(format!("  {:15}", cmd.name)))?;
                        stdout.queue(style::SetForegroundColor(Color::White))?;
                        stdout.queue(style::Print(format!(" │ {:30}", cmd.description)))?;
                        if let Some(key) = &cmd.key {
                            stdout.queue(style::SetForegroundColor(Color::Grey))?;
                            stdout.queue(style::Print(format!(" ({})", key)))?;
                        }
                        let used = 2 + 15 + 3 + 30 + cmd.key.as_ref().map(|k| k.len() + 3).unwrap_or(0);
                        if used < width { stdout.queue(style::Print(" ".repeat(width - used)))?; }
                        stdout.queue(style::ResetColor)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn render_status_bar(&self, stdout: &mut Stdout, status_y: usize, width: usize) -> Result<()> {
        stdout.queue(cursor::MoveTo(0, status_y as u16))?;
        stdout.queue(terminal::Clear(ClearType::CurrentLine))?;
        stdout.queue(style::SetForegroundColor(Color::Black))?;
        stdout.queue(style::SetBackgroundColor(Color::White))?;

        let path_str = self.file_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "<new file>".to_string());
        let sel_info = if let Some((a, b)) = self.selection_range() { format!("SEL {}:{}-{}:{}", a.y + 1, a.x + 1, b.y + 1, b.x + 1) } else { " ".to_string() };
        let dirty = if self.dirty { "*" } else { " " };
        let msg = self.status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        let wrap_info = if self.word_wrap { "[WRAP]" } else { "" };

        let left = format!(" {}{} {} {}  Ln {}, Col {}  {} ", dirty, "", path_str, wrap_info, self.cursor.y + 1, self.cursor.x + 1, sel_info);
        let mut bar = left;
        if !msg.is_empty() { bar.push_str(" | "); bar.push_str(&msg); }
        if bar.chars().count() < width { bar.push_str(&" ".repeat(width - bar.chars().count())); }
        else { bar = bar.chars().take(width).collect(); }

        stdout.queue(style::Print(bar))?;
        stdout.queue(style::ResetColor)?;
        Ok(())
    }

    fn calculate_cursor_position(&self, width: usize, gutter: usize, _editor_h: usize) -> Result<(usize, usize)> {
        let avail = width.saturating_sub(gutter);
        if self.word_wrap {
            let mut current_screen_row = 0;
            for line_idx in 0..self.buf.line_count() {
                let line = &self.buf.lines[line_idx];
                let segments = self.calculate_wrap_segments(line, avail);
                if line_idx == self.cursor.y {
                    let mut seg_idx = 0;
                    for (i, &start) in segments.iter().enumerate() { if self.cursor.x >= start { seg_idx = i; } else { break; } }
                    let cursor_y = (current_screen_row + seg_idx).saturating_sub(self.scroll_y);
                    let start_char = segments[seg_idx];
                    let col: usize = line.chars().skip(start_char).take(self.cursor.x - start_char).map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1)).sum();
                    return Ok((gutter + col, cursor_y));
                }
                current_screen_row += segments.len();
            }
            Ok((gutter, 0))
        } else {
            let cursor_row = self.cursor.y.saturating_sub(self.scroll_y);
            let line = &self.buf.lines[self.cursor.y];
            let col: usize = line.chars().skip(self.scroll_x).take(self.cursor.x.saturating_sub(self.scroll_x)).map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1)).sum();
            Ok((gutter + col, cursor_row))
        }
    }
}
