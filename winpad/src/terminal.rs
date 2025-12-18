//! Terminal setup and teardown.

use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{EnableMouseCapture, DisableMouseCapture},
    style,
    terminal::{self, ClearType},
    ExecutableCommand,
};
use std::io::{self, Stdout, Write};

/// RAII guard for terminal state.
///
/// In Rust, "RAII" means you acquire a resource in `new()` and release it in `Drop`.
/// That guarantees cleanup even if the function returns early.
pub struct TerminalGuard;

impl TerminalGuard {
    /// Enable raw mode, alternate screen, and mouse capture.
    pub fn new(stdout: &mut Stdout) -> Result<Self> {
        terminal::enable_raw_mode().context("enable_raw_mode failed")?;
        stdout.execute(terminal::EnterAlternateScreen)?;
        stdout.execute(EnableMouseCapture)?;
        stdout.execute(cursor::Hide)?;
        stdout.execute(terminal::Clear(ClearType::All))?;
        stdout.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    /// Always restore terminal state when exiting the editor.
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = stdout.execute(style::ResetColor);
        let _ = stdout.execute(cursor::Show);
        let _ = stdout.execute(DisableMouseCapture);
        let _ = stdout.execute(terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        let _ = stdout.flush();
    }
}


