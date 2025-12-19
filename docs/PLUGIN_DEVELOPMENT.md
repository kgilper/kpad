# kpad Plugin Development Guide

This guide provides everything you need to develop plugins for kpad, the terminal text editor.

## Table of Contents

1. [Overview](#overview)
2. [Getting Started](#getting-started)
3. [Plugin Structure](#plugin-structure)
4. [The Manifest File](#the-manifest-file)
5. [Writing Rhai Scripts](#writing-rhai-scripts)
6. [Plugin API Reference](#plugin-api-reference)
7. [Syntax Highlighting](#syntax-highlighting)
8. [Lifecycle Hooks](#lifecycle-hooks)
9. [Keybindings](#keybindings)
10. [Example Plugins](#example-plugins)
11. [Best Practices](#best-practices)
12. [Troubleshooting](#troubleshooting)

---

## Overview

kpad plugins are written in [Rhai](https://rhai.rs/), a simple embedded scripting language for Rust. Plugins can:

- Add custom commands accessible via the command palette (Ctrl+P)
- Bind commands to keyboard shortcuts
- Manipulate text (insert, replace, transform)
- Access cursor and selection information
- Register syntax highlighting rules
- React to file open/save events via lifecycle hooks

Plugins are sandboxed with a maximum of 2,000,000 operations per call to prevent infinite loops.

---

## Getting Started

### Plugin Location

Plugins are loaded from two directories:

1. `./plugins/` - Relative to your current working directory
2. `<executable_dir>/plugins/` - Next to the kpad executable

### Creating Your First Plugin

1. Create a new directory in the plugins folder:
   ```
   plugins/
   └── my_plugin/
       ├── plugin.toml
       └── main.rhai
   ```

2. Create the manifest file (`plugin.toml`):
   ```toml
   id = "my_plugin"
   name = "My First Plugin"
   script = "main.rhai"

   [[commands]]
   name = "hello_world"
   description = "Say hello"
   func = "hello"
   key = "Ctrl+Shift+H"
   ```

3. Create the script file (`main.rhai`):
   ```rhai
   fn hello(api) {
       api.status("Hello from my plugin!");
   }
   ```

4. Restart kpad - your plugin will be loaded automatically.

5. Press `Ctrl+Shift+H` or use `Ctrl+P` and type "hello" to run your command.

---

## Plugin Structure

Every plugin requires:

```
plugins/
└── <plugin_id>/
    ├── plugin.toml    # Required: Plugin manifest
    └── main.rhai      # Required: Main script (name configurable in manifest)
```

You can name the script file anything, as long as you specify it in the manifest's `script` field.

---

## The Manifest File

The `plugin.toml` manifest defines your plugin's metadata, commands, and hooks.

### Full Schema

```toml
# Required: Unique identifier (must match folder name)
id = "my_plugin"

# Optional: Human-readable name (defaults to id)
name = "My Plugin"

# Required: Path to the Rhai script file
script = "main.rhai"

# Optional: Command definitions (can have multiple)
[[commands]]
name = "command_name"           # Required: Command identifier
description = "What it does"    # Required: Shown in command palette
func = "rhai_function_name"     # Required: Function to call in script
key = "Ctrl+Shift+X"            # Optional: Keyboard shortcut

[[commands]]
name = "another_command"
description = "Another action"
func = "another_function"

# Optional: Lifecycle hooks
[hooks]
on_open = "function_name"       # Called when a file is opened
on_save = "function_name"       # Called when a file is saved
```

---

## Writing Rhai Scripts

### Rhai Basics

Rhai is a simple scripting language with Rust-like syntax:

```rhai
// Variables
let x = 42;
let name = "kpad";
let list = [1, 2, 3];

// Strings
let s = "hello";
let upper = s.to_upper();  // "HELLO"
let len = s.len;           // 5

// Conditionals
if x > 10 {
    // ...
} else {
    // ...
}

// Loops
for item in list {
    // ...
}

// Functions
fn my_function(api) {
    // api is the PluginApi object
    api.status("Hello!");
}

// String methods
let text = "  hello world  ";
text.trim();                    // "hello world"
text.to_upper();                // "  HELLO WORLD  "
text.to_lower();                // "  hello world  "
text.split(" ");                // ["", "", "hello", "world", "", ""]
text.contains("world");         // true
text.replace("world", "rhai");  // "  hello rhai  "
text.sub_string(2, 7);          // "hello"
```

### Command Functions

Command functions receive the `api` object as their only parameter:

```rhai
fn my_command(api) {
    // Your code here
}
```

### Hook Functions

Hook functions receive the `api` object and the file path:

```rhai
fn on_file_open(api, path) {
    let ext = api.file_extension();
    api.status("Opened: " + path);
}
```

---

## Plugin API Reference

All API methods are called on the `api` object passed to your functions.

### Text Operations

| Method | Description | Returns |
|--------|-------------|---------|
| `api.text()` | Get entire buffer contents | `String` |
| `api.set_text(s)` | Replace entire buffer with `s` | - |
| `api.insert(s)` | Insert `s` at cursor position | - |

### Selection

| Method | Description | Returns |
|--------|-------------|---------|
| `api.has_selection()` | Check if text is selected | `bool` |
| `api.selection_text()` | Get selected text | `String` |
| `api.replace_selection(s)` | Replace selection with `s` | - |

### Cursor

| Method | Description | Returns |
|--------|-------------|---------|
| `api.cursor_line()` | Get cursor line (1-based) | `i64` |
| `api.cursor_col()` | Get cursor column (1-based) | `i64` |
| `api.set_cursor(line, col)` | Move cursor (1-based coordinates) | - |

### Current Line

| Method | Description | Returns |
|--------|-------------|---------|
| `api.current_line_text()` | Get text of current line | `String` |
| `api.set_current_line_text(s)` | Replace current line with `s` | - |

### File Information

| Method | Description | Returns |
|--------|-------------|---------|
| `api.file_path()` | Get current file path | `String` |
| `api.file_extension()` | Get file extension (lowercase, no dot) | `String` |

### User Interface

| Method | Description | Returns |
|--------|-------------|---------|
| `api.status(msg)` | Show status message for 2 seconds | - |

### Syntax Highlighting

| Method | Description |
|--------|-------------|
| `api.add_highlight(ext, pattern, color, priority)` | Add highlight rule |
| `api.add_highlight_group(ext, pattern, color, priority, group)` | Add rule with capture group |
| `api.clear_highlights(ext)` | Clear rules for extension |
| `api.clear_all_highlights()` | Clear all highlight rules |

---

## Syntax Highlighting

### Adding Highlight Rules

```rhai
fn setup_highlights(api, path) {
    let ext = api.file_extension();
    if ext != "myext" {
        return;
    }

    // Clear existing rules for this extension
    api.clear_highlights("myext");

    // Add rules: extension, regex pattern, color, priority
    api.add_highlight("myext", "\\b(if|else|while|for)\\b", "magenta", 10);
    api.add_highlight("myext", "\"[^\"]*\"", "green", 5);
    api.add_highlight("myext", "//.*$", "grey", 1);
}
```

### Available Colors

| Standard | Bright |
|----------|--------|
| `red` | `bright_red` |
| `green` | `bright_green` |
| `yellow` | `bright_yellow` |
| `blue` | `bright_blue` |
| `magenta` | `bright_magenta` |
| `cyan` | `bright_cyan` |
| `white` | - |
| `grey` | - |

### Priority System

Higher priority rules override lower ones when patterns overlap:

```rhai
// Comments (low priority) - can be overridden
api.add_highlight("rs", "//.*$", "grey", 1);

// Keywords (high priority) - take precedence
api.add_highlight("rs", "\\b(fn|let|if|else)\\b", "magenta", 10);
```

### Capture Groups

Use `add_highlight_group` to highlight specific parts of a match:

```rhai
// Highlight only the function name (group 1), not the whole pattern
api.add_highlight_group("rs", "fn\\s+(\\w+)", "yellow", 10, 1);
```

### Regex Pattern Tips

- Patterns use Rust's `regex` crate syntax
- Escape backslashes: use `\\b` for word boundary, `\\s` for whitespace
- Common patterns:
  - `\\b(word1|word2)\\b` - Match whole words
  - `\"[^\"]*\"` - Double-quoted strings
  - `'[^']*'` - Single-quoted strings
  - `//.*$` - Single-line comments
  - `#.*$` - Hash comments
  - `\\d+` - Numbers
  - `^\\s*` - Leading whitespace

---

## Lifecycle Hooks

Hooks allow your plugin to respond to editor events.

### on_open

Called when a file is opened. Receives the file path as the second argument.

```toml
[hooks]
on_open = "handle_open"
```

```rhai
fn handle_open(api, path) {
    let ext = api.file_extension();
    if ext == "md" {
        // Set up markdown-specific behavior
        api.status("Markdown file opened");
    }
}
```

### on_save

Called when a file is saved. Receives the file path as the second argument.

```toml
[hooks]
on_save = "handle_save"
```

```rhai
fn handle_save(api, path) {
    api.status("File saved: " + path);
}
```

---

## Keybindings

### Supported Keys

**Modifiers:** `Ctrl`, `Alt`, `Shift` (can be combined)

**Special Keys:**
- `Enter`, `Esc`, `Tab`, `Backspace`, `Delete`
- `Left`, `Right`, `Up`, `Down`
- `Home`, `End`, `PageUp`, `PageDown`
- `F1` through `F12`

**Letters and Numbers:** `A`-`Z`, `0`-`9`

### Format

Keys are case-insensitive and normalized automatically:

```toml
key = "Ctrl+Shift+X"    # Recommended format
key = "ctrl+shift+x"    # Also works
key = "CTRL+SHIFT+X"    # Also works
```

### Examples

```toml
key = "Ctrl+U"          # Ctrl + U
key = "Ctrl+Shift+F"    # Ctrl + Shift + F
key = "Alt+Enter"       # Alt + Enter
key = "F5"              # F5 key
key = "Ctrl+Alt+T"      # Ctrl + Alt + T
```

### Avoiding Conflicts

Built-in keybindings take precedence. Avoid these combinations:

| Key | Built-in Action |
|-----|-----------------|
| `Ctrl+S` | Save |
| `Ctrl+O` | Open |
| `Ctrl+Q` | Quit |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` | Redo |
| `Ctrl+C` | Copy |
| `Ctrl+X` | Cut |
| `Ctrl+V` | Paste |
| `Ctrl+A` | Select All |
| `Ctrl+F` | Find |
| `Ctrl+G` | Go to Line |
| `Ctrl+P` | Command Palette |
| `Alt+W` | Toggle Word Wrap |
| `F1` | Help |
| `F2` | Statistics |

---

## Example Plugins

### Text Transformation: Uppercase

Converts selected text (or current line) to uppercase.

**plugin.toml:**
```toml
id = "uppercase"
name = "Uppercase Tools"
script = "main.rhai"

[[commands]]
name = "uppercase_selection"
description = "Uppercase selection (or current line if no selection)"
func = "uppercase_selection"
key = "Ctrl+U"
```

**main.rhai:**
```rhai
fn uppercase_selection(api) {
    if api.has_selection() {
        let s = api.selection_text();
        api.replace_selection(s.to_upper());
        api.status("Uppercased selection.");
    } else {
        let line = api.current_line_text();
        api.set_current_line_text(line.to_upper());
        api.status("Uppercased line.");
    }
}
```

### Code Cleanup: Trim Whitespace

Removes trailing whitespace from all lines.

**plugin.toml:**
```toml
id = "trim_whitespace"
name = "Trim Whitespace"
script = "main.rhai"

[[commands]]
name = "trim_trailing_ws"
description = "Trim trailing whitespace on every line"
func = "trim_trailing_ws"
key = "Ctrl+Alt+T"
```

**main.rhai:**
```rhai
fn trim_trailing_ws(api) {
    let t = api.text();
    let lines = t.split("\n");
    let result = process_all_lines(lines, 0, []);
    api.set_text(result.join("\n"));
    api.status("Trimmed trailing whitespace.");
}

fn process_all_lines(lines, idx, acc) {
    if idx >= lines.len {
        return acc;
    }
    let line = lines[idx];
    let trimmed = trim_trailing(line, line.len);
    let new_acc = acc + [trimmed];
    return process_all_lines(lines, idx + 1, new_acc);
}

fn trim_trailing(s, pos) {
    if pos <= 0 {
        return "";
    }
    if pos > s.len {
        return trim_trailing(s, s.len);
    }
    let ch = s.sub_string(pos - 1, pos);
    if ch == " " || ch == "\t" || ch == "\r" {
        return trim_trailing(s, pos - 1);
    }
    if pos == s.len {
        return s;
    }
    return s.sub_string(0, pos);
}
```

### Syntax Highlighting: Markdown

Adds syntax highlighting for Markdown files.

**plugin.toml:**
```toml
id = "markdown_highlight"
name = "Markdown Syntax Highlighting"
script = "main.rhai"

[hooks]
on_open = "setup_highlights"
```

**main.rhai:**
```rhai
fn setup_highlights(api, path) {
    let ext = api.file_extension();
    if ext != "md" && ext != "markdown" {
        return;
    }

    api.clear_highlights("md");
    api.clear_highlights("markdown");

    // Headers - highest priority
    api.add_highlight("md", "^#{1,6}\\s.*$", "yellow", 10);

    // Bold text
    api.add_highlight("md", "\\*\\*[^*]+\\*\\*", "bright_yellow", 5);
    api.add_highlight("md", "__[^_]+__", "bright_yellow", 5);

    // Italic text
    api.add_highlight("md", "(?<![*_])\\*[^*]+\\*(?![*_])", "cyan", 4);

    // Inline code
    api.add_highlight("md", "`[^`]+`", "green", 6);

    // Code blocks
    api.add_highlight("md", "^```.*$", "green", 8);

    // Links [text](url)
    api.add_highlight("md", "\\[[^\\]]+\\]\\([^)]+\\)", "blue", 5);

    // Blockquotes
    api.add_highlight("md", "^>.*$", "grey", 3);

    // Lists
    api.add_highlight("md", "^\\s*[-*+]\\s", "cyan", 2);
    api.add_highlight("md", "^\\s*\\d+\\.\\s", "cyan", 2);

    api.status("Markdown highlighting enabled");
}
```

### Insert Template: Date Header

Inserts a formatted date header at the cursor.

**plugin.toml:**
```toml
id = "date_header"
name = "Date Header"
script = "main.rhai"

[[commands]]
name = "insert_date_header"
description = "Insert a date header comment"
func = "insert_date"
key = "Ctrl+Shift+D"
```

**main.rhai:**
```rhai
fn insert_date(api) {
    // Note: Rhai doesn't have built-in date functions
    // This is a template example
    let header = "// ============================================\n";
    header += "// TODO: Add date here\n";
    header += "// ============================================\n\n";
    api.insert(header);
    api.status("Inserted header template");
}
```

### Line Operations: Duplicate Line

Duplicates the current line.

**plugin.toml:**
```toml
id = "duplicate_line"
name = "Duplicate Line"
script = "main.rhai"

[[commands]]
name = "duplicate_line"
description = "Duplicate the current line"
func = "duplicate"
key = "Ctrl+Shift+D"
```

**main.rhai:**
```rhai
fn duplicate(api) {
    let line = api.current_line_text();
    let current_line = api.cursor_line();

    // Move to end of line and insert newline + copy
    let text = api.text();
    let lines = text.split("\n");

    let result = [];
    let idx = 0;
    for l in lines {
        result += [l];
        if idx == current_line - 1 {
            result += [l];  // Duplicate this line
        }
        idx += 1;
    }

    api.set_text(result.join("\n"));
    api.set_cursor(current_line + 1, 1);
    api.status("Line duplicated");
}
```

---

## Best Practices

### Performance

1. **Avoid processing large files line-by-line when possible**
   - Use `api.text()` and process in chunks
   - The editor has a 2,000,000 operation limit per call

2. **Use hooks appropriately**
   - `on_open` is best for syntax highlighting setup
   - `on_save` is best for file cleanup operations

3. **Clear highlights before adding**
   - Always call `api.clear_highlights(ext)` before registering rules
   - Prevents duplicate rules from accumulating

### User Experience

1. **Provide feedback with `api.status()`**
   - Let users know when an action completes
   - Report errors clearly

2. **Handle edge cases**
   - Check `api.has_selection()` before operating on selection
   - Validate input before transformations

3. **Use appropriate priorities for highlighting**
   - Comments: 1-3
   - Strings: 4-6
   - Keywords: 8-10
   - Special syntax: 7-9

### Code Organization

1. **Keep functions focused**
   - One function per command
   - Helper functions for complex logic

2. **Use descriptive names**
   - Command names should indicate action
   - Function names should match command purpose

3. **Comment complex regex patterns**
   - Document what each pattern matches
   - Note any edge cases

---

## Troubleshooting

### Plugin Not Loading

1. **Check folder structure**
   - Must be `plugins/<id>/plugin.toml`
   - Folder name should match `id` in manifest

2. **Verify manifest syntax**
   - TOML parsing errors will prevent loading
   - Check for missing quotes or brackets

3. **Check script path**
   - `script` field must point to valid file
   - Path is relative to plugin folder

### Command Not Appearing

1. **Verify command registration**
   - Check `[[commands]]` syntax in manifest
   - Ensure `func` matches function name in script

2. **Check for script errors**
   - Syntax errors prevent command registration
   - Look for error messages in status bar

### Keybinding Not Working

1. **Check for conflicts**
   - Built-in bindings take precedence
   - Other plugins may use the same key

2. **Verify key format**
   - Use `Ctrl+X` format, not `C-x`
   - Keys are case-insensitive

### Syntax Highlighting Not Working

1. **Check file extension**
   - Extension in `add_highlight` must match file
   - Extensions are lowercase, no dot

2. **Verify regex pattern**
   - Test patterns in a regex tester
   - Remember to escape backslashes: `\\b` not `\b`

3. **Check priority values**
   - Lower priority rules may be overridden
   - Ensure priority matches intended behavior

### Script Errors

1. **Operation limit exceeded**
   - Infinite loops will hit the 2M operation limit
   - Use recursion carefully

2. **Type errors**
   - Rhai is dynamically typed but strict
   - Check method names and parameter types

3. **Status bar messages**
   - Plugin errors are shown in status bar
   - Check for "Plugin hook error" messages

---

## Resources

- [Rhai Language Documentation](https://rhai.rs/book/)
- [Rust regex Syntax](https://docs.rs/regex/latest/regex/#syntax)
- [kpad Source Code](https://github.com/kgilper/kpad)

---

## Contributing

To contribute plugins to the kpad ecosystem:

1. Create your plugin following this guide
2. Test thoroughly with various file types
3. Document usage in a README within your plugin folder
4. Share via GitHub or the kpad community

Happy plugin development!
