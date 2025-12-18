//! Built-in command registration.

use crate::commands::{Command, CommandRegistry, CommandSource};
use crate::types::{Prompt, PromptKind};

/// Register all built-in editor commands.
pub fn register_builtin_commands(reg: &mut CommandRegistry) {
    reg.register(Command {
        name: "save".to_string(),
        description: "Save file (Ctrl+S)".to_string(),
        key: Some("Ctrl+S".to_string()),
        source: CommandSource::Builtin(|ed| ed.cmd_save()),
    });

    reg.register(Command {
        name: "open".to_string(),
        description: "Open file (Ctrl+O)".to_string(),
        key: Some("Ctrl+O".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(PromptKind::Open, ""));
            Ok(())
        }),
    });

    reg.register(Command {
        name: "find".to_string(),
        description: "Find (Ctrl+F)".to_string(),
        key: Some("Ctrl+F".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(PromptKind::Find, ed.last_find.clone().unwrap_or_default()));
            Ok(())
        }),
    });

    reg.register(Command {
        name: "stats".to_string(),
        description: "Show document statistics (F2)".to_string(),
        key: Some("F2".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.show_stats = true;
            ed.mark_redraw();
            Ok(())
        }),
    });

    reg.register(Command {
        name: "eol".to_string(),
        description: "Toggle line endings (LF/CRLF)".to_string(),
        key: None,
        source: CommandSource::Builtin(|ed| {
            ed.toggle_line_ending();
            Ok(())
        }),
    });

    reg.register(Command {
        name: "help".to_string(),
        description: "Show help screen (F1)".to_string(),
        key: Some("F1".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.show_help = true;
            ed.mark_redraw();
            Ok(())
        }),
    });

    reg.register(Command {
        name: "command".to_string(),
        description: "Command prompt / palette (Ctrl+P)".to_string(),
        key: Some("Ctrl+P".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(PromptKind::Command, ""));
            ed.mark_redraw();
            Ok(())
        }),
    });

    reg.register(Command {
        name: "goto_line".to_string(),
        description: "Go to line (Ctrl+G)".to_string(),
        key: Some("Ctrl+G".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.prompt = Some(Prompt::new(PromptKind::GotoLine, ""));
            ed.mark_redraw();
            Ok(())
        }),
    });

    reg.register(Command {
        name: "undo".to_string(),
        description: "Undo (Ctrl+Z)".to_string(),
        key: Some("Ctrl+Z".to_string()),
        source: CommandSource::Builtin(|ed| ed.undo()),
    });

    reg.register(Command {
        name: "redo".to_string(),
        description: "Redo (Ctrl+Y)".to_string(),
        key: Some("Ctrl+Y".to_string()),
        source: CommandSource::Builtin(|ed| ed.redo()),
    });

    reg.register(Command {
        name: "quit".to_string(),
        description: "Quit (Ctrl+Q)".to_string(),
        key: Some("Ctrl+Q".to_string()),
        source: CommandSource::Builtin(|_ed| Ok(())),
    });

    reg.register(Command {
        name: "copy".to_string(),
        description: "Copy selection (Ctrl+C)".to_string(),
        key: Some("Ctrl+C".to_string()),
        source: CommandSource::Builtin(|ed| ed.copy()),
    });

    reg.register(Command {
        name: "cut".to_string(),
        description: "Cut selection (Ctrl+X)".to_string(),
        key: Some("Ctrl+X".to_string()),
        source: CommandSource::Builtin(|ed| ed.cut()),
    });

    reg.register(Command {
        name: "paste".to_string(),
        description: "Paste clipboard (Ctrl+V)".to_string(),
        key: Some("Ctrl+V".to_string()),
        source: CommandSource::Builtin(|ed| ed.paste()),
    });

    reg.register(Command {
        name: "select_all".to_string(),
        description: "Select entire buffer (Ctrl+A)".to_string(),
        key: Some("Ctrl+A".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.select_all();
            ed.ensure_visible()?;
            Ok(())
        }),
    });

    reg.register(Command {
        name: "wrap".to_string(),
        description: "Toggle word wrapping".to_string(),
        key: Some("Alt+W".to_string()),
        source: CommandSource::Builtin(|ed| {
            ed.toggle_word_wrap();
            Ok(())
        }),
    });
}
