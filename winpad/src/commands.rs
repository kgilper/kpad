//! Command registry and command execution system.

use crate::editor::Editor;
use crate::utils::levenshtein_distance;
use anyhow::Result;
use std::collections::HashMap;

/// Where a command comes from: built-in Rust function or plugin function.
#[derive(Clone)]
pub enum CommandSource {
    /// A built-in command implemented as a Rust function.
    Builtin(fn(&mut Editor) -> Result<()>),
    /// A plugin-provided command (plugin_id, function_name).
    Plugin { plugin_id: String, func: String },
}

/// A user-invokable action.
///
/// Commands can be invoked either by keybinding (`key`) or via the command palette prompt.
#[derive(Clone)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub key: Option<String>, // canonical string e.g. "Ctrl+S"
    pub source: CommandSource,
}

/// Registry of known commands + lookup tables for fast resolving.
pub struct CommandRegistry {
    commands: Vec<Command>,
    by_name: HashMap<String, usize>,
    keymap: HashMap<String, String>, // key -> command_name
}

impl CommandRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            commands: vec![],
            by_name: HashMap::new(),
            keymap: HashMap::new(),
        }
    }

    /// Add or replace a command.
    ///
    /// - Names are case-insensitive.
    /// - If a keybinding is present, we also add it to `keymap` so key presses can resolve fast.
    pub fn register(&mut self, cmd: Command) {
        let name_key = cmd.name.to_lowercase();
        if let Some(k) = cmd.key.as_ref() {
            self.keymap.insert(k.clone(), cmd.name.clone());
        }

        if let Some(&idx) = self.by_name.get(&name_key) {
            self.commands[idx] = cmd;
        } else {
            let idx = self.commands.len();
            self.commands.push(cmd);
            self.by_name.insert(name_key, idx);
        }
    }

    /// Lookup a command by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&Command> {
        let idx = *self.by_name.get(&name.to_lowercase())?;
        self.commands.get(idx)
    }

    /// List commands (sorted) for help/auto-complete UI.
    #[allow(dead_code)]
    pub fn list_names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.commands.iter().map(|c| c.name.clone()).collect();
        v.sort();
        v
    }

    /// Resolve a key chord like `"Ctrl+S"` to a command name.
    pub fn resolve_key(&self, key: &str) -> Option<String> {
        self.keymap.get(key).cloned()
    }

    /// Fuzzy-ish search over commands by name/description.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Command> {
        let q = query.to_lowercase();
        let mut items: Vec<&Command> = self
            .commands
            .iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&q) || c.description.to_lowercase().contains(&q)
            })
            .collect();
        items.sort_by_key(|c| c.name.to_lowercase());
        items.truncate(limit);
        items
    }

    /// Find the closest command by name using Levenshtein distance.
    pub fn suggest_command(&self, name: &str) -> Option<&Command> {
        let name = name.to_lowercase();
        let mut best_dist = usize::MAX;
        let mut best_cmd = None;

        for cmd in &self.commands {
            let dist = levenshtein_distance(&name, &cmd.name.to_lowercase());
            if dist < best_dist {
                best_dist = dist;
                best_cmd = Some(cmd);
            }
        }

        // Only suggest if the distance is small enough (e.g. < 40% of the word length)
        if let Some(cmd) = best_cmd {
            let threshold = (name.len().max(cmd.name.len()) as f32 * 0.4).ceil() as usize;
            if best_dist <= threshold.max(2) {
                return Some(cmd);
            }
        }
        None
    }
}

/// Convert a crossterm `KeyEvent` into a canonical string like `"Ctrl+S"`.
///
/// Canonical ordering: Ctrl, Alt, Shift + Key
pub fn canonical_key_string(key: &crossterm::event::KeyEvent) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};

    let mut parts: Vec<&str> = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift");
    }

    let key_name = match key.code {
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        _ => format!("{:?}", key.code),
    };

    if parts.is_empty() {
        key_name
    } else {
        parts.push(&key_name);
        parts.join("+")
    }
}


