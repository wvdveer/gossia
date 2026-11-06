//! Each language's file has every text the workspace's sources give `tr!`,
//! with the same named places, and nothing else (DESIGN.md, step 13f).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gossia_i18n::{Language, catalogue, places};

/// The workspace's crates.
fn crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// A Rust string literal's text from just after its opening quote: the text
/// and what follows the closing quote. Only the escapes texts use.
fn literal(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '\n' | '\r' => {
                    // A line continued: the next line's leading spaces go.
                    let rest = &s[i + 2..];
                    let skip = rest.len() - rest.trim_start().len();
                    let target = i + 2 + skip;
                    while chars.clone().next().is_some_and(|(j, _)| j < target) {
                        chars.next();
                    }
                }
                'u' => {
                    let rest = &s[i + 2..];
                    let end = rest.find('}')?;
                    let code = u32::from_str_radix(&rest[1..end], 16).ok()?;
                    out.push(char::from_u32(code)?);
                    let target = i + 2 + end + 1;
                    while chars.clone().next().is_some_and(|(j, _)| j < target) {
                        chars.next();
                    }
                }
                other => panic!("an escape tr! texts do not use: \\{other}"),
            },
            c => out.push(c),
        }
    }
    None
}

/// Every text given to `tr!` (or marked with `tr_noop!`) in the workspace's
/// sources, with the file it is first in.
fn texts() -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    rust_files(&crates(), &mut files);
    files.sort();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for file in files {
        // This file names the macro in its patterns.
        if file.ends_with("catalogues.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&file).unwrap();
        let mut rest = source.as_str();
        while let Some((i, len)) = next_macro(rest) {
            // Not the end of another macro's name, such as `include_str!(`.
            let word = rest[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            // Nor in a comment, where examples name it.
            let line = &rest[rest[..i].rfind('\n').map_or(0, |n| n + 1)..i];
            let comment = line.trim_start().starts_with("//");
            rest = &rest[i + len..];
            if word || comment {
                continue;
            }
            let after = rest.trim_start();
            let Some(body) = after.strip_prefix('"') else {
                panic!("{}: tr! takes a string literal", file.display());
            };
            let (text, tail) = literal(body)
                .unwrap_or_else(|| panic!("{}: an unfinished tr! text", file.display()));
            rest = tail;
            if seen.insert(text.clone()) {
                out.push((text, file.clone()));
            }
        }
    }
    out
}

/// Where the next `tr!(` or `tr_noop!(` starts, and its length.
fn next_macro(text: &str) -> Option<(usize, usize)> {
    ["tr!(", "tr_noop!("]
        .iter()
        .filter_map(|m| text.find(m).map(|i| (i, m.len())))
        .min()
}

#[test]
fn every_language_has_every_text_and_no_other() {
    let texts = texts();
    let wanted: BTreeSet<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
    let mut problems = Vec::new();
    for language in Language::ALL {
        if language == Language::English {
            continue;
        }
        let file = catalogue(language);
        for (text, source) in &texts {
            match file.get(text) {
                None => problems.push(format!(
                    "{}: missing {text:?} (from {})",
                    language.tag(),
                    source.display()
                )),
                Some(translation) => {
                    let mut want = places(text);
                    let mut got = places(translation);
                    want.sort_unstable();
                    got.sort_unstable();
                    if want != got {
                        problems.push(format!(
                            "{}: {text:?} has the places {want:?}, its translation {got:?}",
                            language.tag()
                        ));
                    }
                }
            }
        }
        for key in file.keys() {
            if !wanted.contains(key.as_str()) {
                problems.push(format!(
                    "{}: {key:?} is not a text of the sources",
                    language.tag()
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn literals_are_read_as_rust_reads_them() {
    let (text, rest) = literal(r#"Open \"{name}\"\n" , x"#).unwrap();
    assert_eq!(text, "Open \"{name}\"\n");
    assert_eq!(rest, " , x");
    let (text, _) = literal("one \\\n         two\"").unwrap();
    assert_eq!(text, "one two");
    let (text, _) = literal(r#"\u{2014}""#).unwrap();
    assert_eq!(text, "\u{2014}");
}
