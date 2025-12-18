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
- **types.rs**: Core types: `Pos`, `LineEnding`, `EditOperation`, `UndoEntry`, `Prompt`, `HighlightColor`, `HighlightRule`, `HighlightSpan`
- **buffer.rs**: Document model (`Vec<String>` of lines) with editing primitives
- **commands.rs**: `CommandRegistry` for built-in and plugin commands with keymap resolution
- **utils.rs**: UTF-8 helpers (`char_to_byte_index`, `byte_to_char_index`)

### editor/ module
- **mod.rs**: `Editor` struct definition, state management, core methods
- **input.rs**: Key/mouse/prompt event handling
- **movement.rs**: Cursor movement and word boundary detection
- **render.rs**: Terminal rendering (lines, status bar, scroll indicator)
- **highlight.rs**: Syntax highlighting rule management and regex-based pattern matching
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

**Syntax Highlighting**: Plugin-based highlighting via `add_highlight(ext, pattern, color, priority)`. Rules are regex patterns registered per file extension. Higher priority wins on overlap. Colors: red, green, yellow, blue, magenta, cyan, white, grey, bright_red, bright_green, bright_yellow, bright_blue, bright_magenta, bright_cyan.

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

### Plugin API Reference

**Text/Selection**: `text()`, `set_text(s)`, `has_selection()`, `selection_text()`, `replace_selection(s)`, `insert(s)`

**Cursor**: `cursor_line()`, `cursor_col()`, `set_cursor(line, col)` (1-based)

**Current Line**: `current_line_text()`, `set_current_line_text(s)`

**File Info**: `file_path()`, `file_extension()`

**Status**: `status(msg)`

**Highlighting**:
- `add_highlight(ext, pattern, color, priority)` - Register a highlight rule
- `add_highlight_group(ext, pattern, color, priority, group)` - Highlight specific capture group
- `clear_highlights(ext)` - Clear rules for extension
- `clear_all_highlights()` - Clear all rules

### Hook-Based Syntax Highlighting

Use `on_open` hook to register highlights when files are opened:

```toml
[hooks]
on_open = "setup_highlights"
```

```rhai
fn setup_highlights(api, path) {
    if api.file_extension() != "md" { return; }
    api.clear_highlights("md");
    api.add_highlight("md", "^#{1,6}\\s.*$", "yellow", 10);  // Headers
    api.add_highlight("md", "`[^`]+`", "green", 6);          // Inline code
}
```
