//! Plugin system: loads Rhai scripts from `plugins/*/plugin.toml` and provides a safe API.

use crate::buffer::Buffer;
use crate::commands::{Command, CommandRegistry, CommandSource};
use crate::editor::Editor;
use crate::types::Pos;
use crate::utils::clamp_usize_i64;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::cmp::min;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Optional lifecycle hooks that plugins may implement.
#[derive(Debug, Clone, Copy)]
pub enum Hook {
    OnOpen,
    OnSave,
}

/// Parsed representation of a plugin's `plugin.toml` manifest.
///
/// This tells us:
/// - the plugin identity (`id`, `name`)
/// - which Rhai script file to load (`script`)
/// - which commands to register (with keybindings)
/// - optional lifecycle hooks (`hooks`)
#[derive(Debug, Deserialize)]
struct PluginManifest {
    id: String,
    name: Option<String>,
    script: String,

    #[serde(default)]
    commands: Vec<PluginCommand>,

    #[serde(default)]
    hooks: PluginHooks,
}

/// A command declaration inside `plugin.toml`.
#[derive(Debug, Deserialize)]
struct PluginCommand {
    name: String,
    description: String,
    func: String,
    key: Option<String>,
}

/// Optional plugin hook function names (in the Rhai script).
#[derive(Debug, Deserialize, Default)]
struct PluginHooks {
    on_open: Option<String>,
    on_save: Option<String>,
}

/// A loaded plugin: compiled Rhai AST + metadata.
struct Plugin {
    id: String,
    _name: String,
    ast: rhai::AST,
    hooks: PluginHooks,
}

/// Loads plugins from disk and runs plugin commands/hooks.
#[derive(Default)]
pub struct PluginManager {
    engine: rhai::Engine,
    plugins: Vec<Plugin>,
}

impl PluginManager {
    /// Load all plugins from `search_dirs` and register their commands into `reg`.
    pub fn load(search_dirs: Vec<PathBuf>, reg: &mut CommandRegistry) -> Result<Self> {
        let mut engine = rhai::Engine::new();
        engine.set_max_operations(2_000_000); // keep plugins from hanging the editor

        // Register PluginApi type and methods.
        // This defines the functions available to plugin scripts.
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

        let mut plugins = Vec::new();

        // Walk each search directory and look for subfolders containing `plugin.toml`.
        for dir in search_dirs {
            if !dir.exists() {
                continue;
            }
            // Expect structure: plugins/<plugin>/plugin.toml
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for ent in entries.flatten() {
                let path = ent.path();
                if !path.is_dir() {
                    continue;
                }
                let manifest_path = path.join("plugin.toml");
                if !manifest_path.exists() {
                    continue;
                }

                let manifest_s = fs::read_to_string(&manifest_path)
                    .with_context(|| format!("Reading {}", manifest_path.display()))?;
                let manifest: PluginManifest = toml::from_str(&manifest_s)
                    .with_context(|| format!("Parsing {}", manifest_path.display()))?;

                let script_path = path.join(&manifest.script);
                let ast = engine
                    .compile_file(script_path.clone())
                    .map_err(|e| anyhow!("Compiling {}: {}", script_path.display(), e))?;

                let id = manifest.id.clone();
                let name = manifest.name.clone().unwrap_or_else(|| id.clone());

                // Register commands declared in the manifest with the editor's command registry.
                for c in &manifest.commands {
                    reg.register(Command {
                        name: c.name.clone(),
                        description: format!("{} (plugin: {})", c.description, name),
                        key: c.key.as_ref().map(|k| normalize_key_string(k)),
                        source: CommandSource::Plugin {
                            plugin_id: id.clone(),
                            func: c.func.clone(),
                        },
                    });
                }

                plugins.push(Plugin {
                    id,
                    _name: name,
                    ast,
                    hooks: manifest.hooks,
                });
            }
        }

        Ok(Self { engine, plugins })
    }

    /// Find a loaded plugin by id.
    fn find(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// Run a plugin command function (by name) in the Rhai engine.
    ///
    /// We pass a `PluginApi` into the script so it can query/modify editor state.
    ///
    /// Note: This takes `&mut self` to avoid borrow checker issues when called from Editor methods.
    pub fn run_command(&mut self, ed: &mut Editor, plugin_id: &str, func: &str) -> Result<()> {
        let plugin = self
            .find(plugin_id)
            .ok_or_else(|| anyhow!("Plugin not found: {}", plugin_id))?;
        let api = PluginApi::new(ed);
        let mut scope = rhai::Scope::new();
        let _ = self.engine
            .call_fn::<rhai::Dynamic>(&mut scope, &plugin.ast, func, (api,))
            .map_err(|e| anyhow!("Plugin command failed: {}::{}: {}", plugin_id, func, e))?;
        Ok(())
    }

    /// Call a lifecycle hook on all plugins (best-effort).
    ///
    /// Hooks are *non-fatal*: if a plugin fails, we show a status message but keep the editor
    /// running.
    ///
    /// Note: This takes `&mut self` to avoid borrow checker issues when called from Editor methods.
    pub fn call_hook(&mut self, ed: &mut Editor, hook: Hook, path: Option<&PathBuf>) -> Result<()> {
        // Best-effort hooks (don't crash editor)
        for p in &self.plugins {
            let func = match hook {
                Hook::OnOpen => p.hooks.on_open.as_deref(),
                Hook::OnSave => p.hooks.on_save.as_deref(),
            };
            let Some(func) = func else { continue; };

            let api = PluginApi::new(ed);
            let mut scope = rhai::Scope::new();
            let res = if let Some(path) = path {
                self.engine.call_fn::<rhai::Dynamic>(
                    &mut scope,
                    &p.ast,
                    func,
                    (api, path.display().to_string()),
                )
            } else {
                self.engine
                    .call_fn::<rhai::Dynamic>(&mut scope, &p.ast, func, (api,))
            };
            if let Err(e) = res {
                // show but keep going
                ed.set_status(
                    format!("Plugin hook error ({}): {}", p.id, e),
                    Duration::from_secs(3),
                );
            }
        }
        Ok(())
    }
}

/// Normalize a user-provided keybinding string into our canonical form.
///
/// Plugin manifests (`plugin.toml`) may contain keys in various casings/spacings:
/// - `"ctrl+s"`
/// - `"CTRL+Shift+u"`
/// - `"Alt + t"`
///
/// We normalize these into the same format produced by `canonical_key_string()` so they match.
fn normalize_key_string(s: &str) -> String {
    // Accept e.g. "ctrl+s", "CTRL+Shift+u"
    // Output canonical: Ctrl+Shift+U
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None::<String>;

    for part in s.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let p = part.to_lowercase();
        match p.as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            _ => {
                key = Some(match p.as_str() {
                    "enter" => "Enter".to_string(),
                    "esc" | "escape" => "Esc".to_string(),
                    "backspace" => "Backspace".to_string(),
                    "delete" | "del" => "Delete".to_string(),
                    "tab" => "Tab".to_string(),
                    "left" => "Left".to_string(),
                    "right" => "Right".to_string(),
                    "up" => "Up".to_string(),
                    "down" => "Down".to_string(),
                    "home" => "Home".to_string(),
                    "end" => "End".to_string(),
                    "pageup" => "PageUp".to_string(),
                    "pagedown" => "PageDown".to_string(),
                    other => {
                        if other.len() == 1 {
                            other.chars().next().unwrap().to_ascii_uppercase().to_string()
                        } else if other.starts_with('f')
                            && other[1..].chars().all(|c| c.is_ascii_digit())
                        {
                            format!("F{}", &other[1..])
                        } else {
                            // fallback
                            part.to_string()
                        }
                    }
                });
            }
        }
    }

    let key = key.unwrap_or_else(|| "?".to_string());
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    parts.push(key);
    parts.join("+")
}

// ===== Plugin API exposed to Rhai =====
//
// Plugins get a `PluginApi` object. Methods query/mutate the real `Editor`.
//
// Important safety note:
// - We pass a pointer to the editor into Rhai so scripts can call back into Rust.
// - This uses `unsafe` internally because Rust cannot statically prove that a raw pointer is valid.
// - It is safe *in this program* because:
//   - plugin calls are synchronous (we don't store the API and call it later)
//   - the editor is single-threaded
//   - `PluginApi` is only used during the call where the `Editor` reference is alive
//
#[derive(Clone)]
pub struct PluginApi {
    /// Raw pointer back to the `Editor`.
    ///
    /// We use a raw pointer here because the script engine needs values that it can move around,
    /// but a normal `&mut Editor` borrow cannot safely "escape" into Rhai.
    ///
    /// We keep it safe by only constructing `PluginApi` right before a script call and by only
    /// using it synchronously within that call.
    ed: *mut Editor,
}

impl PluginApi {
    /// Create a new API wrapper for this script call.
    pub fn new(ed: &mut Editor) -> Self {
        Self { ed }
    }

    /// Temporarily borrow the underlying editor mutably and run `f` against it.
    ///
    /// This is the only place we dereference the raw pointer (`unsafe`).
    fn with_editor<T>(&mut self, f: impl FnOnce(&mut Editor) -> T) -> T {
        unsafe { f(&mut *self.ed) }
    }

    /// Get the entire buffer contents as a single string (joined with `\n`).
    pub fn text(&mut self) -> String {
        self.with_editor(|ed| ed.buf.to_string())
    }

    /// Replace the entire buffer contents with `s`.
    ///
    /// This resets cursor/selection/scroll for simplicity.
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

    /// Get the selected text (empty string if no selection).
    pub fn selection_text(&mut self) -> String {
        self.with_editor(|ed| ed.selected_text())
    }

    /// Replace the selection with `s` (or insert at cursor if no selection).
    pub fn replace_selection(&mut self, s: String) {
        self.with_editor(|ed| {
            // plugin edits should be undoable: record snapshot once per command already,
            // so just do the edit
            ed.replace_selection_or_insert(&s);
        })
    }

    /// Insert text at the cursor (replacing selection if present).
    pub fn insert(&mut self, s: String) {
        self.with_editor(|ed| {
            ed.replace_selection_or_insert(&s);
        })
    }

    /// 1-based cursor line (for scripting convenience).
    pub fn cursor_line(&mut self) -> i64 {
        self.with_editor(|ed| (ed.cursor.y as i64) + 1)
    }

    /// 1-based cursor column (for scripting convenience).
    pub fn cursor_col(&mut self) -> i64 {
        self.with_editor(|ed| (ed.cursor.x as i64) + 1)
    }

    /// Set the cursor position using 1-based `(line, col)` coordinates.
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

    /// Replace the current line with `s` (cursor clamped to the new line length).
    pub fn set_current_line_text(&mut self, s: String) {
        self.with_editor(|ed| {
            if ed.cursor.y < ed.buf.lines.len() {
                ed.buf.lines[ed.cursor.y] = s;
                ed.cursor.x = min(ed.cursor.x, ed.buf.line_len_chars(ed.cursor.y));
                ed.dirty = true;
            }
        })
    }

    /// Show a short status message from a plugin.
    pub fn status(&mut self, msg: String) {
        self.with_editor(|ed| ed.set_status(msg, Duration::from_secs(2)))
    }

    /// Return the current file path as a string (empty if unnamed).
    pub fn file_path(&mut self) -> String {
        self.with_editor(|ed| {
            ed.file_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        })
    }
}


