// Markdown Syntax Highlighting Plugin
// Registers highlight rules for Markdown files (.md)

fn setup_highlights(api, path) {
    // Only set up highlights for Markdown files
    let ext = api.file_extension();
    if ext != "md" && ext != "markdown" {
        return;
    }

    // Clear any existing highlights for markdown
    api.clear_highlights("md");
    api.clear_highlights("markdown");

    // Headers (# to ######)
    // Priority 10 for headers so they stand out
    api.add_highlight("md", "^#{1,6}\\s.*$", "yellow", 10);
    api.add_highlight("markdown", "^#{1,6}\\s.*$", "yellow", 10);

    // Bold text (**text** or __text__)
    api.add_highlight("md", "\\*\\*[^*]+\\*\\*", "bright_yellow", 5);
    api.add_highlight("md", "__[^_]+__", "bright_yellow", 5);
    api.add_highlight("markdown", "\\*\\*[^*]+\\*\\*", "bright_yellow", 5);
    api.add_highlight("markdown", "__[^_]+__", "bright_yellow", 5);

    // Italic text (*text* or _text_) - be careful not to match bold
    api.add_highlight("md", "(?<![*_])\\*[^*]+\\*(?![*_])", "cyan", 4);
    api.add_highlight("md", "(?<![*_])_[^_]+_(?![*_])", "cyan", 4);
    api.add_highlight("markdown", "(?<![*_])\\*[^*]+\\*(?![*_])", "cyan", 4);
    api.add_highlight("markdown", "(?<![*_])_[^_]+_(?![*_])", "cyan", 4);

    // Inline code (`code`)
    api.add_highlight("md", "`[^`]+`", "green", 6);
    api.add_highlight("markdown", "`[^`]+`", "green", 6);

    // Code blocks (``` or ~~~)
    api.add_highlight("md", "^```.*$", "green", 8);
    api.add_highlight("md", "^~~~.*$", "green", 8);
    api.add_highlight("markdown", "^```.*$", "green", 8);
    api.add_highlight("markdown", "^~~~.*$", "green", 8);

    // Links [text](url)
    api.add_highlight("md", "\\[[^\\]]+\\]\\([^)]+\\)", "blue", 5);
    api.add_highlight("markdown", "\\[[^\\]]+\\]\\([^)]+\\)", "blue", 5);

    // Images ![alt](url)
    api.add_highlight("md", "!\\[[^\\]]*\\]\\([^)]+\\)", "magenta", 5);
    api.add_highlight("markdown", "!\\[[^\\]]*\\]\\([^)]+\\)", "magenta", 5);

    // Blockquotes (> text)
    api.add_highlight("md", "^>.*$", "grey", 3);
    api.add_highlight("markdown", "^>.*$", "grey", 3);

    // Unordered lists (- or * or +)
    api.add_highlight("md", "^\\s*[-*+]\\s", "cyan", 2);
    api.add_highlight("markdown", "^\\s*[-*+]\\s", "cyan", 2);

    // Ordered lists (1. 2. etc)
    api.add_highlight("md", "^\\s*\\d+\\.\\s", "cyan", 2);
    api.add_highlight("markdown", "^\\s*\\d+\\.\\s", "cyan", 2);

    // Horizontal rules (---, ***, ___)
    api.add_highlight("md", "^[-*_]{3,}\\s*$", "grey", 3);
    api.add_highlight("markdown", "^[-*_]{3,}\\s*$", "grey", 3);

    // Strikethrough (~~text~~)
    api.add_highlight("md", "~~[^~]+~~", "red", 4);
    api.add_highlight("markdown", "~~[^~]+~~", "red", 4);

    api.status("Markdown highlighting enabled");
}
