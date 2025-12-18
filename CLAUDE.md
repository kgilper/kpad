# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

kpad is a Notepad-like terminal text editor for Windows (PowerShell, cmd.exe, Windows Terminal) written in Rust. It features:
- TUI with line numbers, status bar, selection highlighting
- Standard keybindings (Ctrl+S/O/C/X/V/Z/Y, etc.)
- Undo/redo with delta-based operations
- Plugin system using Rhai scripts

## Build Commands

```bash
# Build (from winpad directory)
cd winpad && cargo build --release

# Run
./target/release/kpad.exe [FILE]

# Global install/uninstall
cargo install --path .
cargo uninstall kpad
```

## Architecture

The editor is in `winpad/src/` with this structure:

- **main.rs**: Entry point and main event loop (render -> read input -> update state)
- **terminal.rs**: Raw mode + alternate screen setup via RAII (`TerminalGuard`)
- **types.rs**: Core types: `Pos`, `LineEnding`, `EditOperation`, `UndoEntry`, `Prompt`
- **buffer.rs**: Document model (`Vec<String>` of lines) with editing primitives
- **commands.rs**: `CommandRegistry` for built-in and plugin commands with keymap resolution
- **utils.rs**: UTF-8 helpers (`char_to_byte_index`, `byte_to_char_index`)

### editor/ module
- **mod.rs**: `Editor` struct definition, state management, core methods
- **input.rs**: Key/mouse/prompt event handling
- **movement.rs**: Cursor movement and word boundary detection
- **render.rs**: Terminal rendering (lines, status bar, scroll indicator)
- **screens.rs**: Full-screen overlays (help, statistics)
- **clipboard.rs**: Copy/cut/paste operations
- **undo.rs**: Undo/redo stack management
- **file_ops.rs**: Open/save/search operations
- **builtin_commands.rs**: Built-in command registration

### plugins/ module
- **mod.rs**: `PluginManager`, manifest parsing, hook execution
- **api.rs**: `PluginApi` with script-exposed methods

## Key Patterns

**UTF-8 Handling**: Cursor positions use char indices, but string slicing requires byte indices. Always use helpers from `utils.rs` when converting.

**Delta-Based Undo**: Uses `EditOperation` (Insert/Delete with text) rather than full buffer snapshots. See `record_edit()` in editor/undo.rs.

**Plugin API**: Plugins receive a `PluginApi` object with methods like `text()`, `set_text()`, `selection_text()`, `replace_selection()`. Commands register via `plugin.toml`.

**Rendering**: Full redraw strategy with `needs_redraw` flag to avoid unnecessary renders. Word wrap mode calculates screen rows from logical lines.

## Plugin Structure

```
plugins/
  <plugin_id>/
    plugin.toml     # Metadata, commands, keybindings
    main.rhai       # Script functions
```

Example plugin.toml:
```toml
id = "uppercase"
name = "Uppercase Tools"
script = "main.rhai"

[[commands]]
name = "uppercase_selection"
func = "uppercase_selection"
key = "Ctrl+U"
```
