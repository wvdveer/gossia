//! Comma- and tab-separated values as data (DESIGN.md, "Import and export",
//! "CSV and TSV"): an array with an object for each row, the fields named by
//! the header row.
//!
//! The first row is taken as the header unless it cannot be: when one of its
//! cells is a number or `true`/`false`, it is data, and the columns are named
//! `column_1`, `column_2`, ... Each column is typed as a whole: whole numbers
//! if every value in it is one, numbers if every value is a number, yes or no
//! if every value is `true` or `false`, and text otherwise, so a column of
//! codes such as `007` stays text. An empty cell leaves the field out.

use gossia_i18n::tr;
use indexmap::IndexMap;

use crate::infer::{JsonInference, infer_json};
use crate::value::{Node, Value};

/// One cell as read: its text, and whether it was quoted (a quoted cell is
/// text as it is, never trimmed).
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    pub quoted: bool,
}

/// Splits the text into rows of cells, as RFC 4180 reads them: a cell in
/// double quotes may hold the separator, line breaks and `""` for a quote.
/// Blank lines are skipped. Returns whether a quote was left open.
pub fn rows(text: &str, separator: char) -> (Vec<Vec<Cell>>, bool) {
    let mut rows = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    let end_cell = |row: &mut Vec<Cell>, cell: &mut String, quoted: &mut bool| {
        let text = if *quoted {
            std::mem::take(cell)
        } else {
            let trimmed = cell.trim_matches(' ').to_owned();
            cell.clear();
            trimmed
        };
        row.push(Cell {
            text,
            quoted: *quoted,
        });
        *quoted = false;
    };
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' if cell.trim_matches(' ').is_empty() && !quoted => {
                cell.clear();
                quoted = true;
                in_quotes = true;
            }
            c if c == separator => end_cell(&mut row, &mut cell, &mut quoted),
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                end_cell(&mut row, &mut cell, &mut quoted);
                if !is_blank(&row) {
                    rows.push(std::mem::take(&mut row));
                }
                row.clear();
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || quoted || !row.is_empty() {
        end_cell(&mut row, &mut cell, &mut quoted);
        if !is_blank(&row) {
            rows.push(row);
        }
    }
    (rows, in_quotes)
}

fn is_blank(row: &[Cell]) -> bool {
    row.len() == 1 && row[0].text.is_empty() && !row[0].quoted
}

/// What a cell's text is, for typing its column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Number,
    Boolean,
    Text,
}

fn kind_of(cell: &Cell) -> Kind {
    if cell.quoted {
        return Kind::Text;
    }
    let t = cell.text.as_str();
    if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false") {
        return Kind::Boolean;
    }
    let digits = t.strip_prefix(['-', '+']).unwrap_or(t);
    // A leading zero makes a code, not a number: 007, 0123.
    let leading_zero = digits.len() > 1
        && digits.starts_with('0')
        && !digits.starts_with("0.")
        && !digits.starts_with("0e")
        && !digits.starts_with("0E");
    if digits.is_empty() || leading_zero {
        return Kind::Text;
    }
    if digits.chars().all(|c| c.is_ascii_digit()) && t.parse::<i64>().is_ok() {
        return Kind::Integer;
    }
    let numeric = digits
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '-' | '+'))
        && digits.starts_with(|c: char| c.is_ascii_digit() || c == '.');
    if numeric && t.parse::<f64>().is_ok_and(f64::is_finite) {
        return Kind::Number;
    }
    Kind::Text
}

/// The name of column `index` (from 0) when there is no header for it.
pub fn column_name(index: usize) -> String {
    format!("column_{}", index + 1)
}

/// Reads comma- or tab-separated text as data and infers its schema.
pub fn infer_delimited(text: &str, separator: char) -> JsonInference {
    let (mut rows, open_quote) = rows(text, separator);
    let mut notes = Vec::new();
    let header = rows.first().is_some_and(|first| {
        first
            .iter()
            .all(|c| kind_of(c) == Kind::Text || c.text.is_empty())
    });
    let mut names: Vec<String> = Vec::new();
    if header {
        let first = rows.remove(0);
        for (index, cell) in first.iter().enumerate() {
            let base = if cell.text.trim().is_empty() {
                column_name(index)
            } else {
                cell.text.trim().to_owned()
            };
            let mut name = base.clone();
            let mut k = 2;
            while names.iter().any(|n| n == &name) {
                name = format!("{base}_{k}");
                k += 1;
            }
            names.push(name);
        }
        notes.push(tr!("The first row is taken as the names of the columns.").to_owned());
    } else if !rows.is_empty() {
        notes.push(
            tr!(
                "The first row holds a number or a yes or no, so it is data, not names; the \
             columns are named column_1, column_2, ..."
            )
            .to_owned(),
        );
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if header && width > names.len() {
        notes.push(tr!(
            "Some rows have more cells than the header has names; the extra columns are \
             named {column} and on.",
            column = column_name(names.len())
        ));
    }
    while names.len() < width {
        names.push(column_name(names.len()));
    }
    if open_quote {
        notes.push("A quote is not closed; the rest of the text is one cell.".to_owned());
    }
    // Each column's kind: the most general one its values need.
    let kinds: Vec<Kind> = (0..names.len())
        .map(|column| {
            let mut kind = None;
            for row in &rows {
                let Some(cell) = row.get(column).filter(|c| !c.text.is_empty() || c.quoted) else {
                    continue;
                };
                kind = Some(match (kind, kind_of(cell)) {
                    (None, k) => k,
                    (Some(a), b) if a == b => a,
                    (Some(Kind::Integer | Kind::Number), Kind::Integer | Kind::Number) => {
                        Kind::Number
                    }
                    _ => Kind::Text,
                });
            }
            kind.unwrap_or(Kind::Text)
        })
        .collect();
    let items: Vec<Node> = rows
        .iter()
        .map(|row| {
            let mut fields = IndexMap::new();
            for (column, cell) in row.iter().enumerate() {
                if cell.text.is_empty() && !cell.quoted {
                    continue;
                }
                let value = match kinds[column] {
                    Kind::Integer => cell.text.parse().map(Value::Int).ok(),
                    Kind::Number => cell.text.parse().map(Value::Float).ok(),
                    Kind::Boolean => Some(Value::Bool(cell.text.eq_ignore_ascii_case("true"))),
                    Kind::Text => None,
                }
                .unwrap_or_else(|| Value::Str(cell.text.clone()));
                fields.insert(names[column].clone(), Node::new(value));
            }
            Node::new(Value::Object(fields))
        })
        .collect();
    let mut inference = infer_json(&Node::new(Value::Array(items)));
    in_column_order(&mut inference.schema, &names);
    notes.append(&mut inference.warnings);
    inference.warnings = notes;
    inference
}

/// Puts the fields of the items in the order of the columns. Inference takes
/// them in the order it first meets them, which differs when a row leaves a
/// cell empty.
fn in_column_order(schema: &mut Node, names: &[String]) {
    let Value::Object(top) = schema.value_mut() else {
        return;
    };
    let Some(items) = top.get_mut("items") else {
        return;
    };
    let Value::Object(items) = items.value_mut() else {
        return;
    };
    let Some(properties) = items.get_mut("properties") else {
        return;
    };
    if let Value::Object(fields) = properties.value_mut() {
        fields.sort_by_cached_key(|name, _| names.iter().position(|n| n == name));
    }
}
