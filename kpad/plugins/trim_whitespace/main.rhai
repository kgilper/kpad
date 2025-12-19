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
