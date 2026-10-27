//! What the code editor does to its text (DESIGN.md, "Python modules"): the
//! indentation kept and added on Enter, Tab and Shift+Tab, and the top-level
//! functions the procedure box lists. Positions are in characters, as egui's
//! cursors count them, not bytes.

/// The byte offset of character `at` in `text` (its end if `at` is past it).
pub fn byte_of(text: &str, at: usize) -> usize {
    text.char_indices().nth(at).map_or(text.len(), |(b, _)| b)
}

/// The character offset of byte `at`.
fn char_of(text: &str, at: usize) -> usize {
    text[..at].chars().count()
}

/// The byte range of the line that holds byte `at`, without its line break.
fn line_around(text: &str, at: usize) -> (usize, usize) {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    (start, end)
}

/// The 0-based line that character `at` is on.
pub fn line_of(text: &str, at: usize) -> usize {
    text[..byte_of(text, at)].matches('\n').count()
}

/// The character offset where 0-based `line` starts (the end of the text for a
/// line past the last).
pub fn start_of_line(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 1)
        .map_or(text.chars().count(), |(b, _)| char_of(text, b + 1))
}

/// An edit made by a key: the new text and the selection after it (equal ends
/// for a cursor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Enter: a line break, with the indentation of the line it breaks, and one
/// level more after a line that ends with a colon (`def f():`, `if x:`),
/// comments aside. The selection is replaced.
pub fn newline(text: &str, start: usize, end: usize, width: usize) -> Edited {
    let (a, b) = (byte_of(text, start), byte_of(text, end));
    let (line_start, _) = line_around(text, a);
    let before = &text[line_start..a];
    let mut indent: String = before.chars().take_while(|c| *c == ' ').collect();
    if opens_block(before) {
        indent.push_str(&" ".repeat(width));
    }
    let inserted = format!("\n{indent}");
    let cursor = start + inserted.chars().count();
    Edited {
        text: format!("{}{inserted}{}", &text[..a], &text[b..]),
        start: cursor,
        end: cursor,
    }
}

/// Whether a line, up to the cursor, ends a block header: its code, without a
/// trailing comment, ends with a colon.
fn opens_block(line: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut code = String::new();
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '#' => break,
            None => {}
        }
        code.push(c);
    }
    code.trim_end().ends_with(':')
}

/// The lines a selection touches, as byte offsets of their starts. A selection
/// that ends at the start of a line does not touch that line.
fn touched_lines(text: &str, a: usize, b: usize) -> Vec<usize> {
    let (first, _) = line_around(text, a);
    let last_at = if b > a && text[..b].ends_with('\n') {
        b - 1
    } else {
        b
    };
    let mut starts = vec![first];
    starts.extend(
        text[first..last_at]
            .match_indices('\n')
            .map(|(i, _)| first + i + 1),
    );
    starts
}

/// Tab: within one line, spaces up to the next multiple of `width` in place of
/// the selection; across lines, one level more at the start of each line.
pub fn indent(text: &str, start: usize, end: usize, width: usize) -> Edited {
    let (a, b) = (byte_of(text, start), byte_of(text, end));
    if !text[a..b].contains('\n') {
        let (line_start, _) = line_around(text, a);
        let column = text[line_start..a].chars().count();
        let spaces = width - column % width;
        let cursor = start + spaces;
        return Edited {
            text: format!("{}{}{}", &text[..a], " ".repeat(spaces), &text[b..]),
            start: cursor,
            end: cursor,
        };
    }
    let lines = touched_lines(text, a, b);
    let pad = " ".repeat(width);
    let mut out = String::with_capacity(text.len() + lines.len() * width);
    let mut from = 0;
    for &line in &lines {
        out.push_str(&text[from..line]);
        // An empty line stays empty.
        if line < text.len() && !text[line..].starts_with('\n') {
            out.push_str(&pad);
        }
        from = line;
    }
    out.push_str(&text[from..]);
    // The selection grows to whole lines, as editors do.
    let first = char_of(text, lines[0]);
    let added = out.chars().count() - text.chars().count();
    Edited {
        text: out,
        start: first,
        end: end + added,
    }
}

/// Shift+Tab: up to `width` spaces fewer at the start of each line the
/// selection touches (the cursor's line, without a selection).
pub fn dedent(text: &str, start: usize, end: usize, width: usize) -> Edited {
    let (a, b) = (byte_of(text, start), byte_of(text, end));
    let lines = touched_lines(text, a, b);
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    // (line start, spaces taken away there), all ASCII, so bytes are chars.
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for &line in &lines {
        out.push_str(&text[from..line]);
        let spaces = text[line..]
            .chars()
            .take(width)
            .take_while(|c| *c == ' ')
            .count();
        taken.push((line, spaces));
        from = line + spaces;
    }
    out.push_str(&text[from..]);
    // A position moves back by the spaces taken before it.
    let moved = |at_byte: usize, at: usize| {
        let before: usize = taken
            .iter()
            .filter(|(line, _)| *line < at_byte)
            .map(|(line, spaces)| (*spaces).min(at_byte - line))
            .sum();
        at - before
    };
    Edited {
        text: out,
        start: moved(a, start),
        end: moved(b, end),
    }
}

/// The top-level functions of a source, with the 0-based line of each, in the
/// order they appear: what the procedure box lists.
pub fn functions(source: &str) -> Vec<(String, usize)> {
    source
        .lines()
        .enumerate()
        .filter_map(|(line, text)| {
            let rest = text
                .strip_prefix("def ")
                .or_else(|| text.strip_prefix("async def "))?;
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some((name, line))
        })
        .collect()
}

/// The text of the line numbers beside `text`: one per line, right-aligned.
pub fn line_numbers(text: &str) -> String {
    let count = text.matches('\n').count() + 1;
    let digits = count.to_string().len();
    (1..=count)
        .map(|n| format!("{n:>digits$}"))
        .collect::<Vec<_>>()
        .join("\n")
}
