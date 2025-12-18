//! `kpad` — a small Notepad-like terminal editor for Windows consoles (PowerShell, cmd.exe,
//! Windows Terminal).
//!
//! ## Reading guide (high level architecture)
//! - **`main()` / `run()`**: sets up the terminal and runs the main input/render loop.
//! - **`terminal::TerminalGuard`**: switches the terminal into "raw mode" + an alternate screen, then
//!   reliably restores it on exit (even on panic unwind).
//! - **`buffer::Buffer`**: the document model (a `Vec<String>` of lines) and the low-level editing
//!   operations (insert/delete/replace ranges).
//! - **`editor::Editor`**: application state + key handling + rendering + prompts + undo/redo.
//! - **Plugins**: loaded from `./plugins/*/plugin.toml` + Rhai scripts; they register commands
//!   and can modify editor state through **`plugins::PluginApi`**.

mod buffer;
mod commands;
mod editor;
mod plugins;
mod terminal;
mod types;
mod utils;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use editor::Editor;
use std::io;
use std::time::Duration;
use terminal::TerminalGuard;

/// Program entry point.
///
/// We return `anyhow::Result` so we can use `?` with rich error context throughout the code.
fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {:?}", e);
        std::process::exit(1);
    }
}

/// Runs the editor:
/// - parses command line arguments
/// - sets up the terminal (raw mode + alternate screen)
/// - initializes `Editor` state
/// - loops: render → read input events → update state
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    
    // Simple argument parsing
    let mut file_to_open = None;
    
    if args.len() > 1 {
        match args[1].as_str() {
            "-h" | "--help" => {
                println!("kpad — A simple TUI text editor");
                println!();
                println!("USAGE:");
                println!("    kpad [FILE]          Open a file (creates if doesn't exist)");
                println!("    kpad -h, --help      Show this help message");
                println!("    kpad -v, --version   Show version information");
                println!();
                println!("KEYBINDINGS:");
                println!("    Ctrl+P                 Command Palette (Discovery)");
                println!("    Ctrl+S                 Save");
                println!("    Ctrl+O                 Open file prompt");
                println!("    Ctrl+Q                 Quit");
                println!("    Alt+W                  Toggle Word Wrap");
                println!("    Home / End             Top / Bottom of document");
                return Ok(());
            }
            "-v" | "--version" => {
                println!("kpad v0.1.0");
                return Ok(());
            }
            path if path.starts_with('-') => {
                eprintln!("Error: Unknown flag '{}'", path);
                eprintln!("Try 'kpad --help' for more information.");
                std::process::exit(1);
            }
            path => {
                file_to_open = Some(std::path::PathBuf::from(path));
            }
        }
    }

    let mut stdout = io::stdout();
    let _term = TerminalGuard::new(&mut stdout)?;

    let mut editor = Editor::new(file_to_open)?;

    // Main UI loop:
    // - render the whole screen (simple + robust)
    // - poll for input so we can also update time-based UI (status message expiration)
    loop {
        editor.render(&mut stdout)?;

        // Poll so we can expire transient status messages.
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    let should_quit = editor.handle_key(key)?;
                    if should_quit {
                        break;
                    }
                }
                Event::Mouse(mouse) => {
                    editor.handle_mouse(mouse)?;
                }
                Event::Resize(_, _) => {
                    editor.on_resize()?;
                }
                _ => {}
            }
        } else {
            editor.tick();
        }
    }

    Ok(())
}
