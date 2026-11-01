//! Reading and writing TOML (DESIGN.md, "Other file formats", "TOML"). The
//! text is parsed with `toml_edit`, which keeps the comments around each key
//! and value; they become the comments of the nodes, and an empty line
//! before a key is kept on its node. Writing is ClioTree's own, so that the
//! layout is always the same: the plain values of a table first, then a
//! `[table]` for each object in it and `[[array]]` for each array of
//! objects.

use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use toml_edit::{DocumentMut, Item, RawString, Table, Value as Toml};

use crate::json::{ParseError, Parsed, Warning, WriteError};
use crate::path::Path;
use crate::value::{CommentPlace, DateTimeValue, Node, Value};

/// Reads TOML text.
pub fn parse(text: &str) -> Result<Parsed, ParseError> {
    let text = text.trim_start_matches('\u{FEFF}');
    let document: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        let (line, column) = place(text, e.span().map_or(0, |s| s.start));
        ParseError {
            line,
            column,
            message: e.message().trim().to_owned(),
        }
    })?;
    let mut warnings = Vec::new();
    let mut node = table(document.as_table(), &Path::root(), &mut warnings)?;
    if let Some(comment) = comments(Some(document.trailing())) {
        node.set_comment_at(CommentPlace::End, Some(comment));
    }
    Ok(Parsed { node, warnings })
}

/// The line and column (both from 1) of a byte offset.
fn place(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

/// The text of the `#` comments in white space, one line each, or None.
fn comments(raw: Option<&RawString>) -> Option<String> {
    let text = raw?.as_str()?;
    let lines: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix('#'))
        .map(|l| l.strip_prefix(' ').unwrap_or(l).trim_end())
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Whether white space before a key holds an empty line.
fn has_empty_line(raw: Option<&RawString>) -> bool {
    let Some(text) = raw.and_then(RawString::as_str) else {
        return false;
    };
    // The last piece is the indentation of the key's own line.
    let mut lines: Vec<&str> = text.split('\n').collect();
    lines.pop();
    lines.iter().any(|l| l.trim().is_empty())
}

/// The comments of white space split at its first line break: those on the
/// line it starts on, and those after.
fn split_line(raw: Option<&RawString>) -> (Option<String>, Option<String>) {
    let Some(text) = raw.and_then(RawString::as_str) else {
        return (None, None);
    };
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    (
        comments(Some(&RawString::from(first))),
        comments(Some(&RawString::from(rest))),
    )
}

/// Adds a comment to what a node has at `place`.
fn add(node: &mut Node, place: CommentPlace, comment: Option<String>) {
    let comment = joined(node.comment_at(place).map(str::to_owned), comment);
    node.set_comment_at(place, comment);
}

/// Two comments as one: the one above, then the one after.
fn joined(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
        (a, b) => a.or(b),
    }
}

fn table(t: &Table, path: &Path, warnings: &mut Vec<Warning>) -> Result<Node, ParseError> {
    let mut fields = indexmap::IndexMap::new();
    for (key, item) in t.iter() {
        let here = path.key(key);
        let above = t.key(key).and_then(|k| comments(k.leaf_decor().prefix()));
        let mut node = match item {
            Item::None => continue,
            Item::Value(v) => {
                let mut node = value(v, &here, warnings)?;
                let after = comments(v.decor().suffix());
                let comment = joined(joined(above, node.comment().map(str::to_owned)), after);
                node.set_comment(comment);
                let first = fields.is_empty();
                node.set_blank_before(
                    !first && has_empty_line(t.key(key).and_then(|k| k.leaf_decor().prefix())),
                );
                node
            }
            Item::Table(sub) => {
                let mut node = table(sub, &here, warnings)?;
                node.set_comment(joined(above, comments(sub.decor().prefix())));
                node
            }
            Item::ArrayOfTables(array) => {
                let mut items = Vec::new();
                for (i, sub) in array.iter().enumerate() {
                    let mut node = table(sub, &here.index(i), warnings)?;
                    node.set_comment(comments(sub.decor().prefix()));
                    items.push(node);
                }
                let mut node = Node::new(Value::Array(items));
                node.set_comment(above);
                node
            }
        };
        if node.comment().is_some_and(str::is_empty) {
            node.set_comment(None);
        }
        fields.insert(key.to_owned(), node);
    }
    Ok(Node::new(Value::Object(fields)))
}

fn value(v: &Toml, path: &Path, warnings: &mut Vec<Warning>) -> Result<Node, ParseError> {
    let refused = |message: String| ParseError {
        line: 1,
        column: 1,
        message: format!("{path}: {message}"),
    };
    let node = match v {
        Toml::String(s) => Node::new(Value::Str(s.value().clone())),
        Toml::Integer(n) => Node::new(Value::Int(*n.value())),
        Toml::Float(x) => {
            let x = *x.value();
            if !x.is_finite() {
                return Err(refused(format!(
                    "{x} cannot be read: ClioTree holds only finite numbers"
                )));
            }
            Node::new(Value::Float(x))
        }
        Toml::Boolean(b) => Node::new(Value::Bool(*b.value())),
        Toml::Datetime(d) => Node::new(datetime(d.value(), path, warnings).map_err(refused)?),
        Toml::Array(array) => {
            // White space after a comma belongs to the next item, but a
            // comment on the line of the comma is the item before it's.
            let mut items: Vec<Node> = Vec::new();
            for (i, item) in array.iter().enumerate() {
                let mut node = value(item, &path.index(i), warnings)?;
                let (after_last, above) = split_line(item.decor().prefix());
                if let Some(last) = items.last_mut() {
                    add(last, CommentPlace::Before, after_last);
                }
                let comment = joined(above, comments(item.decor().suffix()));
                node.set_comment(joined(comment, node.comment().map(str::to_owned)));
                items.push(node);
            }
            let (after_last, end) = split_line(Some(array.trailing()));
            if let Some(last) = items.last_mut() {
                add(last, CommentPlace::Before, after_last);
            }
            let mut node = Node::new(Value::Array(items));
            if let Some(end) = end {
                node.set_comment_at(CommentPlace::End, Some(end));
            }
            node
        }
        Toml::InlineTable(t) => {
            let mut fields = indexmap::IndexMap::new();
            for (key, item) in t.iter() {
                let mut node = value(item, &path.key(key), warnings)?;
                let above = t.key(key).and_then(|k| comments(k.leaf_decor().prefix()));
                node.set_comment(joined(above, node.comment().map(str::to_owned)));
                fields.insert(key.to_owned(), node);
            }
            Node::new(Value::Object(fields))
        }
    };
    Ok(node)
}

fn datetime(
    d: &toml_edit::Datetime,
    path: &Path,
    warnings: &mut Vec<Warning>,
) -> Result<Value, String> {
    let date = d
        .date
        .map(|x| NaiveDate::from_ymd_opt(i32::from(x.year), u32::from(x.month), u32::from(x.day)));
    let time = d.time.map(|t| {
        NaiveTime::from_hms_nano_opt(
            u32::from(t.hour),
            u32::from(t.minute),
            u32::from(t.second.unwrap_or(0)),
            t.nanosecond.unwrap_or(0),
        )
    });
    let bad = || format!("{d} is not a date and time ClioTree can hold");
    Ok(match (date, time) {
        (Some(date), None) => Value::Date(date.ok_or_else(bad)?),
        (Some(date), Some(time)) => {
            let naive = NaiveDateTime::new(date.ok_or_else(bad)?, time.ok_or_else(bad)?);
            match d.offset {
                None => Value::DateTime(DateTimeValue::Naive(naive)),
                Some(offset) => {
                    let minutes = match offset {
                        toml_edit::Offset::Z => 0,
                        toml_edit::Offset::Custom { minutes } => i32::from(minutes),
                    };
                    let zone = FixedOffset::east_opt(minutes * 60).ok_or_else(bad)?;
                    let zoned = zone.from_local_datetime(&naive).single().ok_or_else(bad)?;
                    Value::DateTime(DateTimeValue::Zoned(zoned))
                }
            }
        }
        (None, _) => {
            warnings.push(Warning {
                path: path.clone(),
                message: format!(
                    "the time of day {d} has no type of its own in ClioTree, so it was read as text"
                ),
            });
            Value::Str(d.to_string())
        }
    })
}

/// Writes a tree as TOML. The root must be an object. A node that holds an
/// error value, or nothing, is left out (TOML has no empty value). Comments
/// are written as `#` lines above what they belong to; inside an inline
/// table (an object in an array of plain values) there is no place for them.
pub fn to_string(node: &Node) -> Result<String, WriteError> {
    let Value::Object(_) = node.value() else {
        return Err(WriteError {
            path: Path::root(),
            message: "TOML holds a table at the top, so the root must be an object".into(),
        });
    };
    let mut out = String::new();
    comment(&mut out, node.comment(), "");
    write_table(&mut out, node, &Path::root(), &[], false)?;
    if let Some(end) = node.comment_at(CommentPlace::End) {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        comment(&mut out, Some(end), "");
    }
    Ok(out)
}

fn comment(out: &mut String, comment: Option<&str>, indent: &str) {
    for line in comment.into_iter().flat_map(str::lines) {
        out.push_str(indent);
        out.push('#');
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
        out.push('\n');
    }
}

fn skipped(node: &Node) -> bool {
    matches!(node.value(), Value::Error(_) | Value::Null)
}

/// Whether a field is written as a `[table]` or `[[array]]` of its own.
fn is_section(node: &Node) -> bool {
    match node.value() {
        Value::Object(_) => true,
        Value::Array(items) => {
            let items: Vec<&Node> = items.iter().filter(|n| !skipped(n)).collect();
            !items.is_empty() && items.iter().all(|n| matches!(n.value(), Value::Object(_)))
        }
        _ => false,
    }
}

/// The fields of a table: its plain values, then its sections, each under
/// its header. `keys` is the header of the table itself.
fn write_table(
    out: &mut String,
    node: &Node,
    path: &Path,
    keys: &[String],
    _in_array: bool,
) -> Result<(), WriteError> {
    let Value::Object(fields) = node.value() else {
        unreachable!("a table is an object");
    };
    for (n, (key, field)) in fields
        .iter()
        .filter(|(_, f)| !skipped(f) && !is_section(f))
        .enumerate()
    {
        if n > 0 && field.blank_before() {
            out.push('\n');
        }
        comment(out, field.comment(), "");
        out.push_str(&bare_or_quoted(key));
        out.push_str(" = ");
        inline(out, field, &path.key(key.clone()), "")?;
        out.push('\n');
    }
    for (key, field) in fields.iter().filter(|(_, f)| !skipped(f) && is_section(f)) {
        let mut header: Vec<String> = keys.to_vec();
        header.push(bare_or_quoted(key));
        let here = path.key(key.clone());
        if !out.is_empty() {
            out.push('\n');
        }
        match field.value() {
            Value::Object(_) => {
                comment(out, field.comment(), "");
                out.push('[');
                out.push_str(&header.join("."));
                out.push_str("]\n");
                write_table(out, field, &here, &header, false)?;
            }
            Value::Array(items) => {
                // The array's own comment goes above its first item.
                let mut first = true;
                for (i, item) in items.iter().enumerate().filter(|(_, n)| !skipped(n)) {
                    if !first {
                        out.push('\n');
                    }
                    if first {
                        comment(out, field.comment(), "");
                    }
                    first = false;
                    comment(out, item.comment(), "");
                    out.push_str("[[");
                    out.push_str(&header.join("."));
                    out.push_str("]]\n");
                    write_table(out, item, &here.index(i), &header, true)?;
                }
            }
            _ => unreachable!("a section is an object or an array of objects"),
        }
    }
    if let Some(end) = node
        .comment_at(CommentPlace::End)
        .filter(|_| !path.is_root())
    {
        comment(out, Some(end), "");
    }
    Ok(())
}

/// A value on the line of its key: text, a number, yes or no, a date, an
/// array or an inline table.
fn inline(out: &mut String, node: &Node, path: &Path, indent: &str) -> Result<(), WriteError> {
    match node.value() {
        Value::Error(_) | Value::Null => unreachable!("left out by the caller"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(x) => {
            if !x.is_finite() {
                return Err(WriteError {
                    path: path.clone(),
                    message: "NaN and infinity cannot be written".into(),
                });
            }
            out.push_str(&serde_json::to_string(x).expect("a finite float serializes"));
        }
        Value::Str(s) => quote(out, s),
        Value::Date(d) => out.push_str(&d.format("%Y-%m-%d").to_string()),
        Value::DateTime(DateTimeValue::Naive(dt)) => {
            out.push_str(&dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
        }
        Value::DateTime(DateTimeValue::Zoned(dt)) => out.push_str(&dt.to_rfc3339()),
        Value::Array(items) => {
            let items: Vec<(usize, &Node)> = items
                .iter()
                .enumerate()
                .filter(|(_, n)| !skipped(n))
                .collect();
            let end = node.comment_at(CommentPlace::End);
            let commented = end.is_some() || items.iter().any(|(_, n)| n.comment().is_some());
            if !commented {
                out.push('[');
                for (n, (i, item)) in items.iter().enumerate() {
                    if n > 0 {
                        out.push_str(", ");
                    }
                    inline(out, item, &path.index(*i), indent)?;
                }
                out.push(']');
            } else {
                let inner = format!("{indent}  ");
                out.push_str("[\n");
                for (i, item) in &items {
                    comment(out, item.comment(), &inner);
                    out.push_str(&inner);
                    inline(out, item, &path.index(*i), &inner)?;
                    out.push_str(",\n");
                }
                comment(out, end, &inner);
                out.push_str(indent);
                out.push(']');
            }
        }
        Value::Object(fields) => {
            let fields: Vec<(&String, &Node)> =
                fields.iter().filter(|(_, n)| !skipped(n)).collect();
            if fields.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            out.push_str("{ ");
            for (n, (key, field)) in fields.iter().enumerate() {
                if n > 0 {
                    out.push_str(", ");
                }
                out.push_str(&bare_or_quoted(key));
                out.push_str(" = ");
                inline(out, field, &path.key((*key).clone()), indent)?;
            }
            out.push_str(" }");
        }
    }
    Ok(())
}

/// A key as TOML writes it: bare when it is letters, digits, `_` and `-`.
fn bare_or_quoted(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        key.to_owned()
    } else {
        let mut out = String::new();
        quote(&mut out, key);
        out
    }
}

/// A basic string. JSON's escapes are TOML's too; TOML also wants DEL escaped.
fn quote(out: &mut String, text: &str) {
    let json = serde_json::to_string(text).expect("a string serializes");
    out.push_str(&json.replace('\u{7F}', "\\u007F"));
}

#[cfg(test)]
#[path = "toml_format_tests.rs"]
mod toml_format_tests;
