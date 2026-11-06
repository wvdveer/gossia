//! Workbooks of other programs as data (DESIGN.md, "Excel and ODS files,
//! 0.4.0"): `.xlsx`, `.xlsm`, `.xlsb`, `.xls` and `.ods`, read with
//! `calamine`. Each worksheet is read as a CSV file is: an array with an
//! object for each row, the fields named by the header row, each column
//! typed as a whole.
//!
//! What differs from CSV is that the cells come typed: a number is a number,
//! text is text even when it reads as one (`007`), and a cell whose number
//! format is a date is a date. A column of dates becomes text with
//! `"format": "date"`; a column of dates with times stays text, since a date
//! and time in JSON Schema needs a time zone, which a cell does not have. A
//! cell with a formula is read as the value last calculated; an error cell
//! (`#N/A`) is left out.

use std::io::Cursor;

use calamine::{Data, Reader, SheetType, open_workbook_auto_from_rs};
use chrono::{NaiveDate, NaiveDateTime, Timelike};
use gossia_i18n::tr;
use indexmap::IndexMap;

use crate::delimited::{column_name, in_column_order};
use crate::infer::{JsonInference, infer_json};
use crate::value::{Node, Value};

/// One worksheet, read.
#[derive(Debug, Clone)]
pub struct SheetRead {
    /// The worksheet's name in the workbook.
    pub name: String,
    /// How many rows of data it has (the header row not counted).
    pub rows: usize,
    pub inference: JsonInference,
}

/// A cell, as its column is typed from it.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
    /// A time of day without a date, kept as text: `07:30:00`.
    Time(String),
    /// `#N/A`, `#DIV/0!`: left out.
    Error,
}

/// What a column holds, the most general kind its cells need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Number,
    Boolean,
    Date,
    DateTime,
    Text,
}

/// The largest whole number a cell holds exactly (2^53).
const EXACT: f64 = 9_007_199_254_740_992.0;

fn cell_of(data: &Data) -> Cell {
    match data {
        Data::Empty => Cell::Empty,
        Data::String(s) if s.is_empty() => Cell::Empty,
        Data::String(s) => Cell::Text(s.clone()),
        Data::Int(i) => Cell::Number(*i as f64),
        Data::Float(x) if x.is_finite() => Cell::Number(*x),
        Data::Float(_) => Cell::Error,
        Data::Bool(b) => Cell::Bool(*b),
        Data::Error(_) => Cell::Error,
        Data::DateTime(d) if d.is_datetime() => {
            let serial = d.as_f64();
            match d.as_datetime() {
                // Less than a day from the start: a time of day.
                Some(dt) if (0.0..1.0).contains(&serial) => {
                    Cell::Time(dt.format("%H:%M:%S").to_string())
                }
                Some(dt) if dt.num_seconds_from_midnight() == 0 && dt.nanosecond() == 0 => {
                    Cell::Date(dt.date())
                }
                Some(dt) => Cell::DateTime(dt),
                None => Cell::Text(data.to_string()),
            }
        }
        // A duration (`[h]:mm`) is text as the program shows it.
        Data::DateTime(_) | Data::DurationIso(_) => Cell::Text(data.to_string()),
        Data::DateTimeIso(s) => {
            if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                Cell::Date(d)
            } else if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
                if dt.num_seconds_from_midnight() == 0 && dt.nanosecond() == 0 {
                    Cell::Date(dt.date())
                } else {
                    Cell::DateTime(dt)
                }
            } else {
                Cell::Text(s.clone())
            }
        }
    }
}

fn kind_of(cell: &Cell) -> Option<Kind> {
    Some(match cell {
        Cell::Empty | Cell::Error => return None,
        Cell::Number(x) if x.fract() == 0.0 && x.abs() < EXACT => Kind::Integer,
        Cell::Number(_) => Kind::Number,
        Cell::Bool(_) => Kind::Boolean,
        Cell::Date(_) => Kind::Date,
        Cell::DateTime(_) => Kind::DateTime,
        Cell::Text(_) | Cell::Time(_) => Kind::Text,
    })
}

fn joined(a: Kind, b: Kind) -> Kind {
    match (a, b) {
        (a, b) if a == b => a,
        (Kind::Integer | Kind::Number, Kind::Integer | Kind::Number) => Kind::Number,
        (Kind::Date | Kind::DateTime, Kind::Date | Kind::DateTime) => Kind::DateTime,
        _ => Kind::Text,
    }
}

fn date_time_text(dt: &NaiveDateTime) -> String {
    if dt.nanosecond() == 0 {
        dt.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        dt.format("%Y-%m-%dT%H:%M:%S%.3f").to_string()
    }
}

/// A cell as text, for a column of text.
fn text_of(cell: &Cell) -> String {
    match cell {
        Cell::Empty | Cell::Error => String::new(),
        Cell::Text(s) | Cell::Time(s) => s.clone(),
        Cell::Number(x) if x.fract() == 0.0 && x.abs() < EXACT => format!("{}", *x as i64),
        Cell::Number(x) => format!("{x}"),
        Cell::Bool(b) => b.to_string(),
        Cell::Date(d) => d.format("%Y-%m-%d").to_string(),
        Cell::DateTime(dt) => date_time_text(dt),
    }
}

fn value_of(cell: &Cell, kind: Kind) -> Value {
    match (kind, cell) {
        (Kind::Integer, Cell::Number(x)) => Value::Int(*x as i64),
        (Kind::Number, Cell::Number(x)) => Value::Float(*x),
        (Kind::Boolean, Cell::Bool(b)) => Value::Bool(*b),
        _ => Value::Str(text_of(cell)),
    }
}

/// Reads every worksheet of a workbook, in the workbook's order. Chart sheets
/// and dialog sheets have no cells and are passed over.
pub fn read(bytes: &[u8]) -> Result<Vec<SheetRead>, String> {
    let mut book = open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let names: Vec<String> = book
        .sheets_metadata()
        .iter()
        .filter(|s| s.typ == SheetType::WorkSheet)
        .map(|s| s.name.clone())
        .collect();
    let mut out = Vec::new();
    for name in names {
        let range = book.worksheet_range(&name).map_err(|e| e.to_string())?;
        // Formulas are counted where the reader has them; a workbook whose
        // formulas cannot be read is still read for its values.
        let formulas = book
            .worksheet_formula(&name)
            .map(|f| f.used_cells().filter(|(_, _, t)| !t.is_empty()).count())
            .unwrap_or(0);
        let rows: Vec<Vec<Cell>> = range
            .rows()
            .map(|row| row.iter().map(cell_of).collect::<Vec<Cell>>())
            .collect();
        out.push(sheet(name, rows, formulas));
    }
    Ok(out)
}

/// One worksheet's rows, from its first used row and column, as data.
fn sheet(name: String, mut rows: Vec<Vec<Cell>>, formulas: usize) -> SheetRead {
    let mut notes = Vec::new();
    let errors = rows
        .iter()
        .flatten()
        .filter(|c| matches!(c, Cell::Error))
        .count();
    rows.retain(|row| row.iter().any(|c| !matches!(c, Cell::Empty)));
    let header = rows.first().is_some_and(|first| {
        first
            .iter()
            .all(|c| matches!(c, Cell::Empty | Cell::Text(_) | Cell::Error))
    });
    let mut names: Vec<String> = Vec::new();
    if header {
        let first = rows.remove(0);
        for (index, cell) in first.iter().enumerate() {
            let text = text_of(cell);
            let base = if text.trim().is_empty() {
                column_name(index)
            } else {
                text.trim().to_owned()
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
                "The first row holds a number, a date or a yes or no, so it is data, not \
                 names; the columns are named column_1, column_2, ..."
            )
            .to_owned(),
        );
    }
    // The columns past the last name that hold nothing are not columns.
    let width = rows
        .iter()
        .map(|row| {
            row.iter()
                .rposition(|c| !matches!(c, Cell::Empty | Cell::Error))
                .map_or(0, |i| i + 1)
        })
        .max()
        .unwrap_or(0);
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
    let kinds: Vec<Kind> = (0..names.len())
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column).and_then(kind_of))
                .reduce(joined)
                .unwrap_or(Kind::Text)
        })
        .collect();
    let items: Vec<Node> = rows
        .iter()
        .map(|row| {
            let mut fields = IndexMap::new();
            for (column, cell) in row.iter().enumerate().take(names.len()) {
                if kind_of(cell).is_none() {
                    continue;
                }
                fields.insert(
                    names[column].clone(),
                    Node::new(value_of(cell, kinds[column])),
                );
            }
            Node::new(Value::Object(fields))
        })
        .collect();
    let count = items.len();
    let mut inference = infer_json(&Node::new(Value::Array(items)));
    in_column_order(&mut inference.schema, &names);
    let dates: Vec<&str> = names
        .iter()
        .zip(&kinds)
        .filter(|(_, k)| **k == Kind::Date)
        .map(|(n, _)| n.as_str())
        .collect();
    with_date_format(&mut inference.schema, &dates);
    let date_times: Vec<String> = names
        .iter()
        .zip(&kinds)
        .filter(|(_, k)| **k == Kind::DateTime)
        .map(|(n, _)| n.clone())
        .collect();
    if !date_times.is_empty() {
        notes.push(tr!(
            "These columns hold dates with times, kept as text, since a date and time in JSON \
             Schema needs a time zone: {columns}.",
            columns = date_times.join(", ")
        ));
    }
    if formulas > 0 {
        notes.push(tr!(
            "Cells with a formula, read as the value last calculated: {n}.",
            n = formulas
        ));
    }
    if errors > 0 {
        notes.push(tr!(
            "Cells left out because they hold an error such as #N/A: {n}.",
            n = errors
        ));
    }
    notes.append(&mut inference.warnings);
    inference.warnings = notes;
    SheetRead {
        name,
        rows: count,
        inference,
    }
}

/// Gives the text fields named `"format": "date"` in the schema of the items.
fn with_date_format(schema: &mut Node, names: &[&str]) {
    let Value::Object(top) = schema.value_mut() else {
        return;
    };
    let Some(Value::Object(items)) = top.get_mut("items").map(Node::value_mut) else {
        return;
    };
    let Some(Value::Object(fields)) = items.get_mut("properties").map(Node::value_mut) else {
        return;
    };
    for name in names {
        if let Some(Value::Object(field)) = fields.get_mut(*name).map(Node::value_mut) {
            let text = field
                .get("type")
                .is_some_and(|t| t.value().as_str() == Some("string"));
            if text {
                field.insert(
                    "format".to_owned(),
                    Node::new(Value::Str("date".to_owned())),
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "spreadsheet_tests.rs"]
mod spreadsheet_tests;
