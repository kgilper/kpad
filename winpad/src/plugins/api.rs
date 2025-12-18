//! Plugin API exposed to Rhai scripts.
//!
//! Plugins get a `PluginApi` object. Methods query/mutate the real `Editor`.
//!
//! Important safety note:
//! - We pass a pointer to the editor into Rhai so scripts can call back into Rust.
//! - This uses `unsafe` internally because Rust cannot statically prove that a raw pointer is valid.
//! - It is safe *in this program* because:
//!   - plugin calls are synchronous (we don't store the API and call it later)
//!   - the editor is single-threaded
//!   - `PluginApi` is only used during the call where the `Editor` reference is alive

use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::types::Pos;
use crate::utils::clamp_usize_i64;
use std::cmp::min;
use std::time::Duration;

/// API wrapper passed to Rhai scripts.
#[derive(Clone)]
pub struct PluginApi {
    /// Raw pointer back to the `Editor`.
    ed: *mut Editor,
}

impl PluginApi {
    /// Create a new API wrapper for this script call.
    pub fn new(ed: &mut Editor) -> Self {
        Self { ed }
    }

    /// Temporarily borrow the underlying editor mutably and run `f` against it.
    fn with_editor<T>(&mut self, f: impl FnOnce(&mut Editor) -> T) -> T {
        unsafe { f(&mut *self.ed) }
    }

    /// Get the entire buffer contents as a single string.
    pub fn text(&mut self) -> String {
        self.with_editor(|ed| ed.buf.to_string())
    }

    /// Replace the entire buffer contents with `s`.
    pub fn set_text(&mut self, s: String) {
        self.with_editor(|ed| {
            ed.buf = Buffer::from_string(&s);
            ed.cursor = Pos { y: 0, x: 0 };
            ed.anchor = None;
            ed.scroll_y = 0;
            ed.scroll_x = 0;
            ed.dirty = true;
        })
    }

    /// Whether there is an active selection.
    pub fn has_selection(&mut self) -> bool {
        self.with_editor(|ed| ed.selection_range().is_some())
    }

    /// Get the selected text.
    pub fn selection_text(&mut self) -> String {
        self.with_editor(|ed| ed.selected_text())
    }

    /// Replace the selection with `s`.
    pub fn replace_selection(&mut self, s: String) {
        self.with_editor(|ed| {
            ed.replace_selection_or_insert(&s);
        })
    }

    /// Insert text at the cursor.
    pub fn insert(&mut self, s: String) {
        self.with_editor(|ed| {
            ed.replace_selection_or_insert(&s);
        })
    }

    /// 1-based cursor line.
    pub fn cursor_line(&mut self) -> i64 {
        self.with_editor(|ed| (ed.cursor.y as i64) + 1)
    }

    /// 1-based cursor column.
    pub fn cursor_col(&mut self) -> i64 {
        self.with_editor(|ed| (ed.cursor.x as i64) + 1)
    }

    /// Set the cursor position using 1-based coordinates.
    pub fn set_cursor(&mut self, line: i64, col: i64) {
        self.with_editor(|ed| {
            let y = clamp_usize_i64(line - 1, 0, ed.buf.line_count().saturating_sub(1));
            let max_x = ed.buf.line_len_chars(y);
            let x = clamp_usize_i64(col - 1, 0, max_x);
            ed.cursor = Pos { y, x };
            ed.anchor = None;
        })
    }

    /// Get the full text of the current line.
    pub fn current_line_text(&mut self) -> String {
        self.with_editor(|ed| ed.buf.lines.get(ed.cursor.y).cloned().unwrap_or_default())
    }

    /// Replace the current line with `s`.
    pub fn set_current_line_text(&mut self, s: String) {
        self.with_editor(|ed| {
            if ed.cursor.y < ed.buf.lines.len() {
                ed.buf.lines[ed.cursor.y] = s;
                ed.cursor.x = min(ed.cursor.x, ed.buf.line_len_chars(ed.cursor.y));
                ed.dirty = true;
            }
        })
    }

    /// Show a short status message.
    pub fn status(&mut self, msg: String) {
        self.with_editor(|ed| ed.set_status(msg, Duration::from_secs(2)))
    }

    /// Return the current file path as a string.
    pub fn file_path(&mut self) -> String {
        self.with_editor(|ed| {
            ed.file_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        })
    }
}

/// Register all PluginApi methods with the Rhai engine.
pub fn register_api(engine: &mut rhai::Engine) {
    engine.register_type::<PluginApi>();
    engine.register_fn("text", PluginApi::text);
    engine.register_fn("set_text", PluginApi::set_text);
    engine.register_fn("has_selection", PluginApi::has_selection);
    engine.register_fn("selection_text", PluginApi::selection_text);
    engine.register_fn("replace_selection", PluginApi::replace_selection);
    engine.register_fn("insert", PluginApi::insert);
    engine.register_fn("cursor_line", PluginApi::cursor_line);
    engine.register_fn("cursor_col", PluginApi::cursor_col);
    engine.register_fn("set_cursor", PluginApi::set_cursor);
    engine.register_fn("current_line_text", PluginApi::current_line_text);
    engine.register_fn("set_current_line_text", PluginApi::set_current_line_text);
    engine.register_fn("status", PluginApi::status);
    engine.register_fn("file_path", PluginApi::file_path);
}
