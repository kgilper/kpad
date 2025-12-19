//! Plugin system: loads Rhai scripts from `plugins/*/plugin.toml` and provides a safe API.

mod api; // plugin api for rhai scripts

pub use api::PluginApi; // expose the api type

use crate::commands::{Command, CommandRegistry, CommandSource}; // command system
use crate::editor::Editor; // editor state
use anyhow::{anyhow, Context, Result}; // anyhow error handling
use serde::Deserialize; // trait for deserializing toml
use std::fs; // file system access
use std::path::PathBuf; // file path handling
use std::time::Duration; // timing for status messages

/// Optional lifecycle hooks that plugins may implement.
#[derive(Debug, Clone, Copy)]
pub enum Hook {
    OnOpen,
    OnSave,
}

/// Parsed representation of a plugin's `plugin.toml` manifest.
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

/// Optional plugin hook function names.
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
        engine.set_max_operations(2_000_000);

        // Register PluginApi type and methods
        api::register_api(&mut engine);

        let mut plugins = Vec::new();

        for dir in search_dirs {
            if !dir.exists() {
                continue;
            }

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

                // Register commands
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

    /// Run a plugin command function.
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
    pub fn call_hook(&mut self, ed: &mut Editor, hook: Hook, path: Option<&PathBuf>) -> Result<()> {
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
                ed.set_status(
                    format!("Plugin hook error ({}): {}", p.id, e),
                    Duration::from_secs(3),
                );
            }
        }
        Ok(())
    }
}

/// Normalize a user-provided keybinding string into canonical form.
fn normalize_key_string(s: &str) -> String {
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
                            part.to_string()
                        }
                    }
                });
            }
        }
    }

    let key = key.unwrap_or_else(|| "?".to_string());
    let mut parts = Vec::new();
    if ctrl { parts.push("Ctrl".to_string()); }
    if alt { parts.push("Alt".to_string()); }
    if shift { parts.push("Shift".to_string()); }
    parts.push(key);
    parts.join("+")
}
