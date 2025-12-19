# kpad

A fast, lightweight terminal text editor written in Rust. Designed for Windows terminals (PowerShell, cmd.exe, Windows Terminal) but works cross-platform.

## Features

- **Large file support**: Uses a Rope data structure for O(log n) editing operations on files with 100,000+ lines
- **Full Unicode support**: Handles UTF-8, emoji, CJK characters with proper display widths
- **Standard keybindings**: Familiar Ctrl+S/O/C/X/V/Z/Y shortcuts
- **Selection and clipboard**: Shift+Arrow selection, system clipboard integration
- **Undo/redo**: Delta-based undo system with 1000-entry history
- **Word wrap**: Toggle with Alt+W
- **Find**: Ctrl+F with wrap-around search
- **Go to line**: Ctrl+G
- **Command palette**: Ctrl+P for command discovery
- **Plugin system**: Extend functionality with Rhai scripts
- **Syntax highlighting**: Plugin-based regex highlighting with priority layers
- **Tab completion**: File path completion in Open/Save dialogs
- **Help screen**: F1 for keybinding reference
- **Statistics screen**: F2 for document stats (lines, words, characters)

## Installation

### Prerequisites

- [Rust](https://rustup.rs/) (1.70 or later recommended)

### Build from source

```bash
cd winpad
cargo build --release
```

The executable will be at `target/release/kpad.exe` (Windows) or `target/release/kpad` (Unix).

### Global installation

```bash
# Install to ~/.cargo/bin (must be in PATH)
cargo install --path .

# Run from anywhere
kpad [FILE]

# Uninstall
cargo uninstall kpad
```

## Usage

```bash
# Open a new file
kpad

# Open an existing file
kpad myfile.txt

# Open with path
kpad src/main.rs
```

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| Arrow keys | Move cursor |
| Ctrl+Left/Right | Jump by word |
| Home/End | Go to start/end of document |
| PageUp/PageDown | Move by screen |
| Ctrl+G | Go to line number |

### Selection

| Key | Action |
|-----|--------|
| Shift+Arrows | Select text |
| Ctrl+A | Select all |
| Esc | Clear selection |

### Editing

| Key | Action |
|-----|--------|
| Ctrl+Z | Undo |
| Ctrl+Y | Redo |
| Ctrl+C | Copy |
| Ctrl+X | Cut |
| Ctrl+V | Paste |
| Tab | Insert 4 spaces |

### File Operations

| Key | Action |
|-----|--------|
| Ctrl+S | Save |
| Ctrl+O | Open file (with Tab completion) |
| Ctrl+Q | Quit (press twice if unsaved) |

### Search & Commands

| Key | Action |
|-----|--------|
| Ctrl+F | Find (Enter to find next) |
| Ctrl+P | Command palette |
| F1 | Help screen |
| F2 | Document statistics |

### Display

| Key | Action |
|-----|--------|
| Alt+W | Toggle word wrap |

## Project Structure

```
winpad/
├── Cargo.toml          # Dependencies and lint configuration
├── src/
│   ├── main.rs         # Entry point, event loop
│   ├── terminal.rs     # Raw mode setup (RAII TerminalGuard)
│   ├── types.rs        # Core types (Pos, LineEnding, EditOperation, etc.)
│   ├── buffer.rs       # Document model using ropey::Rope
│   ├── commands.rs     # CommandRegistry, keymap resolution
│   ├── utils.rs        # Utility functions (digits, clamping, Levenshtein)
│   ├── editor/         # Editor module (split for maintainability)
│   │   ├── mod.rs          # Editor struct, state management
│   │   ├── input.rs        # Keyboard/mouse/prompt handling
│   │   ├── movement.rs     # Cursor movement, word boundaries
│   │   ├── render.rs       # Terminal rendering
│   │   ├── highlight.rs    # Syntax highlighting engine
│   │   ├── screens.rs      # Help and stats overlays
│   │   ├── clipboard.rs    # Copy/cut/paste
│   │   ├── undo.rs         # Undo/redo stack
│   │   ├── file_ops.rs     # Open/save/search
│   │   └── builtin_commands.rs  # Built-in command registration
│   └── plugins/        # Plugin system
│       ├── mod.rs          # PluginManager, manifest parsing
│       └── api.rs          # PluginApi exposed to Rhai scripts
└── plugins/            # Plugin directory (user-created)
    └── <plugin_id>/
        ├── plugin.toml     # Plugin manifest
        └── main.rhai       # Plugin script
```

### Key Components

#### `buffer.rs` - Document Model

Uses `ropey::Rope` for efficient text storage:
- O(log n) insert/delete operations
- Handles files with 100,000+ lines without slowdown
- Streaming file save to avoid memory spikes

#### `editor/mod.rs` - Application State

The `Editor` struct holds:
- Buffer (document text)
- Cursor position and selection anchor
- Viewport scroll position
- Undo/redo stacks
- Plugin manager
- Syntax highlighter

#### `editor/render.rs` - Display

- Line numbers with dynamic width
- Syntax highlighting with selection overlay
- Word wrap mode with proper cursor tracking
- Scroll indicator

#### `editor/highlight.rs` - Syntax Highlighting

- Regex-based pattern matching
- Priority system for overlapping rules
- Per-extension rule sets
- Caching with edit invalidation

## Plugin System

Plugins extend kpad with custom commands and syntax highlighting.

### Plugin Structure

```
plugins/
└── my_plugin/
    ├── plugin.toml
    └── main.rhai
```

### plugin.toml

```toml
id = "my_plugin"
name = "My Plugin"
description = "Does something useful"
version = "1.0.0"
script = "main.rhai"

[[commands]]
name = "do_something"
description = "Does the thing"
func = "do_something"
key = "Ctrl+Shift+D"  # Optional keybinding

[hooks]
on_open = "setup"     # Called when a file is opened
on_save = "cleanup"   # Called when a file is saved
```

### Plugin API

Scripts receive an `api` object with these methods:

**Text Operations**
- `api.text()` - Get entire buffer as string
- `api.set_text(s)` - Replace entire buffer
- `api.insert(s)` - Insert at cursor

**Selection**
- `api.has_selection()` - Check if text is selected
- `api.selection_text()` - Get selected text
- `api.replace_selection(s)` - Replace selection

**Cursor**
- `api.cursor_line()` - Get cursor line (1-based)
- `api.cursor_col()` - Get cursor column (1-based)
- `api.set_cursor(line, col)` - Move cursor

**Current Line**
- `api.current_line_text()` - Get current line text
- `api.set_current_line_text(s)` - Replace current line

**File Info**
- `api.file_path()` - Get current file path
- `api.file_extension()` - Get file extension (lowercase)

**UI**
- `api.status(msg)` - Show status message

**Syntax Highlighting**
- `api.add_highlight(ext, pattern, color, priority)` - Add highlight rule
- `api.add_highlight_group(ext, pattern, color, priority, group)` - Highlight capture group
- `api.clear_highlights(ext)` - Clear rules for extension
- `api.clear_all_highlights()` - Clear all rules

Available colors: `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `grey`, `bright_red`, `bright_green`, `bright_yellow`, `bright_blue`, `bright_magenta`, `bright_cyan`

### Example: Markdown Highlighting

```toml
# plugins/markdown/plugin.toml
id = "markdown"
name = "Markdown Highlighter"
script = "main.rhai"

[hooks]
on_open = "setup_highlights"
```

```rhai
// plugins/markdown/main.rhai
fn setup_highlights(api, path) {
    if api.file_extension() != "md" { return; }

    api.clear_highlights("md");
    api.add_highlight("md", "^#{1,6}\\s.*$", "yellow", 10);  // Headers
    api.add_highlight("md", "\\*\\*[^*]+\\*\\*", "white", 8); // Bold
    api.add_highlight("md", "\\*[^*]+\\*", "cyan", 7);        // Italic
    api.add_highlight("md", "`[^`]+`", "green", 6);           // Inline code
    api.add_highlight("md", "^```.*$", "green", 5);           // Code fences
    api.add_highlight("md", "\\[([^\\]]+)\\]", "blue", 4);    // Links
}
```

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| [ropey](https://crates.io/crates/ropey) | 1 | Rope data structure for large file editing |
| [anyhow](https://crates.io/crates/anyhow) | 1.0 | Ergonomic error handling |
| [crossterm](https://crates.io/crates/crossterm) | 0.28 | Cross-platform terminal manipulation |
| [unicode-width](https://crates.io/crates/unicode-width) | 0.2 | Unicode character display widths |
| [arboard](https://crates.io/crates/arboard) | 3 | System clipboard access |
| [rhai](https://crates.io/crates/rhai) | 1 | Embedded scripting for plugins |
| [serde](https://crates.io/crates/serde) | 1 | Serialization for plugin manifests |
| [toml](https://crates.io/crates/toml) | 0.8 | Parse plugin.toml files |
| [regex](https://crates.io/crates/regex) | 1 | Syntax highlighting patterns |

## Development

### Running Tests

```bash
cargo test
```

### Linting

The project uses strict lint settings:
- `dead_code = "deny"` - Unused code is a compile error
- `clippy::all = "warn"` and `clippy::pedantic = "warn"`

```bash
cargo clippy
```

### Building for Release

```bash
cargo build --release
```

## License

MIT
