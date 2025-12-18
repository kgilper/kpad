# kpad (terminal text editor) — beginner notes

This project is modularly structured to make it easy to maintain and explore.

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| [anyhow](https://crates.io/crates/anyhow) | 1.0 | Ergonomic error handling with context |
| [crossterm](https://crates.io/crates/crossterm) | 0.28 | Cross-platform terminal manipulation (raw mode, colors, input) |
| [unicode-width](https://crates.io/crates/unicode-width) | 0.2 | Determine display width of Unicode characters |
| [arboard](https://crates.io/crates/arboard) | 3 | Cross-platform clipboard access (copy/paste) |
| [rhai](https://crates.io/crates/rhai) | 1 | Embedded scripting language for plugins |
| [serde](https://crates.io/crates/serde) | 1 | Serialization framework (with derive macros) |
| [toml](https://crates.io/crates/toml) | 0.8 | Parse plugin.toml manifest files |
| [regex](https://crates.io/crates/regex) | 1 | Regular expressions for syntax highlighting |

## How to build & run

From this folder:

```powershell
cargo build --release
.\target\release\kpad.exe
```

## Global Installation

To make `kpad` available from any terminal on your machine:

```powershell
# Install
cargo install --path .

# Uninstall
cargo uninstall kpad
```

## Project Structure

If you are new to Rust, this order tends to feel natural:

1. **`main.rs`**: the main entry point and loop (render → read key → update state).
2. **`terminal.rs`**: how raw mode + alternate screen are set up and restored (RAII / `Drop`).
3. **`types.rs`**: foundational data structures like `Pos` and `Snapshot`.
4. **`buffer.rs`**: editing primitives (insert/delete/range delete).
5. **`utils.rs`**: UTF‑8 helper conversions and general utilities.
6. **`editor.rs`**: "the app" state + key handling + rendering + prompts + undo/redo.
7. **`commands.rs`**: `CommandRegistry` and command handling.
8. **`plugins.rs`**: `PluginManager` loads `plugins/*/plugin.toml` and runs Rhai scripts via `PluginApi`.

## Why there are UTF‑8 helper functions

Rust strings are UTF‑8, so you can't safely do `s[3..7]` unless those indices are **byte offsets**
that land on character boundaries. This editor stores cursor columns as **char indices** (what a
user thinks of as "characters"), and uses helper functions in `utils.rs` when it needs to slice a `String`.

## Plugins

Plugins live under `plugins/<plugin_id>/` and include:
- `plugin.toml`: metadata + command declarations + keybindings
- `main.rhai`: the script functions

The Rhai code receives a `PluginApi` object (`api`) that exposes editor operations such as:
- `api.text()` / `api.set_text(...)`
- `api.has_selection()` / `api.selection_text()` / `api.replace_selection(...)`
- `api.current_line_text()` / `api.set_current_line_text(...)`
- `api.status("...")`