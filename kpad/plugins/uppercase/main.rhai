fn uppercase_selection(api) {
    // `api` is a PluginApi object provided by Rust.
    // It exposes methods like:
    // - api.has_selection()
    // - api.selection_text()
    // - api.replace_selection(...)
    // - api.current_line_text()
    // - api.set_current_line_text(...)
    // - api.status(...)
    //
    // This command uppercases either:
    // - the selected text, OR
    // - the current line (if nothing is selected).
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
