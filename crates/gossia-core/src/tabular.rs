//! An array of objects as a table, for the formats that hold rows (DESIGN.md,
//! "Excel and ODS files, 0.4.0", "Exporting a tab"): CSV, TSV and Excel
//! workbooks here, and OpenDocument spreadsheets in [`crate::ods`].
//!
//! The columns are the fields of the items' type in their declared order.
//! A field that is an object is flattened into dotted columns
//! (`address.city`), to any depth, as the table view shows it; a field that
//! is a list or a map has no place in a row, so it is left out and named.
//! Values are written, never formulas: a computed node as its value, an
//! error value as its code (`#TYPE!`).

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use gossia_i18n::tr;

use crate::delimited::{Cell as ReadCell, reads_as_text};
use crate::path::{Path, Segment};
use crate::typespec::{Kind, TypeId, TypeSpec};
use crate::value::{DateTimeValue, Node, Value};

/// A value in a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Empty,
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Date(NaiveDate),
    /// A date and time without a time zone.
    DateTime(NaiveDateTime),
    /// A date and time with its time zone, as text in ISO 8601: a
    /// spreadsheet's cells have no zone, so it is written as text there too.
    ZonedDateTime(String),
}

/// An array of objects laid out in rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The names of the columns, dotted for the fields of nested objects.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    /// The fields that are lists or maps, which have no place in a row.
    pub left_out: Vec<String>,
}

/// Whether `ty` is an array whose items are objects: what can be a table.
pub fn holds_rows(spec: &TypeSpec, ty: TypeId) -> bool {
    match &spec.get(ty).kind {
        Kind::Array { items, .. } => matches!(spec.get(*items).kind, Kind::Object { .. }),
        _ => false,
    }
}

/// The arrays of objects in a tab that can be tables: the root, if it is
/// one, and those reached through the fields of objects, in the order of
/// the schema. Arrays inside arrays are not offered, since which item's
/// would be meant.
pub fn row_arrays(spec: &TypeSpec) -> Vec<Path> {
    let mut out = Vec::new();
    walk(spec, spec.root(), &Path::root(), &mut out, 0);
    out
}

fn walk(spec: &TypeSpec, ty: TypeId, at: &Path, out: &mut Vec<Path>, depth: usize) {
    // A type that contains itself is cut off.
    if depth > 32 {
        return;
    }
    if holds_rows(spec, ty) {
        out.push(at.clone());
        return;
    }
    if let Kind::Object { fields } = &spec.get(ty).kind {
        for field in fields {
            walk(spec, field.ty, &at.key(&field.name), out, depth + 1);
        }
    }
}

/// One column: its name and how to reach its value from an item.
struct Column {
    name: String,
    path: Vec<String>,
}

fn columns(
    spec: &TypeSpec,
    ty: TypeId,
    prefix: &[String],
    out: &mut Vec<Column>,
    left_out: &mut Vec<String>,
) {
    let Kind::Object { fields } = &spec.get(ty).kind else {
        return;
    };
    for field in fields {
        let mut path = prefix.to_vec();
        path.push(field.name.clone());
        let name = path.join(".");
        match &spec.get(field.ty).kind {
            // A type that contains itself is cut off.
            Kind::Object { .. } if path.len() < 32 => {
                columns(spec, field.ty, &path, out, left_out);
            }
            Kind::Object { .. } | Kind::Array { .. } | Kind::Map { .. } => left_out.push(name),
            _ => out.push(Column { name, path }),
        }
    }
}

fn cell_of(node: Option<&Node>) -> Cell {
    let Some(node) = node else {
        return Cell::Empty;
    };
    match node.value() {
        Value::Null => Cell::Empty,
        Value::Bool(b) => Cell::Bool(*b),
        Value::Int(i) => Cell::Int(*i),
        Value::Float(x) => Cell::Float(*x),
        Value::Str(s) => Cell::Text(s.clone()),
        Value::Date(d) => Cell::Date(*d),
        Value::DateTime(DateTimeValue::Naive(dt)) => Cell::DateTime(*dt),
        Value::DateTime(DateTimeValue::Zoned(dt)) => Cell::ZonedDateTime(dt.to_rfc3339()),
        Value::Error(e) => Cell::Text(e.kind.code().to_owned()),
        // `any` may hold a list or a record: written as JSON.
        Value::Array(_) | Value::Object(_) => {
            Cell::Text(crate::json::to_string(node, false).unwrap_or_default())
        }
    }
}

/// The columns the array at `at` would have, and the fields left out as
/// lists or maps, without reading any data; `None` if it is not an array
/// of objects.
pub fn columns_of(spec: &TypeSpec, at: &Path) -> Option<(Vec<String>, Vec<String>)> {
    let resolved = spec.resolve(at)?;
    let Kind::Array { items, .. } = spec.get(resolved.ty).kind else {
        return None;
    };
    if !holds_rows(spec, resolved.ty) {
        return None;
    }
    let mut cols = Vec::new();
    let mut left_out = Vec::new();
    columns(spec, items, &[], &mut cols, &mut left_out);
    Some((cols.into_iter().map(|c| c.name).collect(), left_out))
}

/// The array at `at` in `data` as a table, with the columns its items'
/// type gives; `None` if it is not an array of objects.
pub fn table(spec: &TypeSpec, data: &Node, at: &Path) -> Option<Table> {
    let resolved = spec.resolve(at)?;
    if !holds_rows(spec, resolved.ty) {
        return None;
    }
    let Kind::Array { items, .. } = spec.get(resolved.ty).kind else {
        return None;
    };
    let mut cols = Vec::new();
    let mut left_out = Vec::new();
    columns(spec, items, &[], &mut cols, &mut left_out);
    let nodes = match data.get(at).map(Node::value) {
        Some(Value::Array(nodes)) => nodes.as_slice(),
        _ => &[],
    };
    let rows = nodes
        .iter()
        // An item that is itself an error has no fields to write.
        .filter(|item| !matches!(item.value(), Value::Error(_)))
        .map(|item| {
            cols.iter()
                .map(|c| {
                    let mut node = Some(item);
                    for key in &c.path {
                        node = node.and_then(|n| n.child(&Segment::Key(key.clone())));
                    }
                    cell_of(node)
                })
                .collect()
        })
        .collect();
    Some(Table {
        columns: cols.into_iter().map(|c| c.name).collect(),
        rows,
        left_out,
    })
}

/// A number as text that reads back as the same kind: a whole float keeps
/// its `.0`, so a column of numbers is not read back as whole numbers.
fn number_text(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

/// Whether text must be quoted to be read back as the same text: it holds
/// the separator, a quote or a line break, starts or ends with a space
/// (which the reader trims), is empty, or would be read as a number or a
/// yes or no; and text that starts with `=`, `+`, `-` or `@`, which a
/// spreadsheet would take for a formula.
fn needs_quotes(text: &str, separator: char) -> bool {
    text.is_empty()
        || text.contains([separator, '"', '\n', '\r'])
        || text.starts_with(' ')
        || text.ends_with(' ')
        || text.starts_with(['=', '+', '-', '@'])
        || !reads_as_text(&ReadCell {
            text: text.to_owned(),
            quoted: false,
        })
}

fn field_text(text: &str, separator: char) -> String {
    if needs_quotes(text, separator) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// The table as comma- or tab-separated text, as the reader reads it back:
/// a header row of the column names, RFC 4180 quoting, `\r\n` line ends and
/// UTF-8 without a byte order mark. An empty cell is nothing between the
/// separators, which the reader leaves out.
pub fn to_delimited(table: &Table, separator: char) -> String {
    let mut out = String::new();
    let sep = separator.to_string();
    let header: Vec<String> = table
        .columns
        .iter()
        .map(|c| field_text(c, separator))
        .collect();
    out.push_str(&header.join(&sep));
    out.push_str("\r\n");
    for row in &table.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|cell| match cell {
                Cell::Empty => String::new(),
                Cell::Int(i) => i.to_string(),
                Cell::Float(x) => number_text(*x),
                Cell::Bool(b) => b.to_string(),
                Cell::Text(s) => field_text(s, separator),
                Cell::Date(d) => d.format("%Y-%m-%d").to_string(),
                Cell::DateTime(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
                Cell::ZonedDateTime(s) => field_text(s, separator),
            })
            .collect();
        out.push_str(&cells.join(&sep));
        out.push_str("\r\n");
    }
    out
}

/// The most rows a worksheet holds, the header row included.
pub(crate) const MAX_ROWS: usize = 1_048_576;
/// The most columns a worksheet holds.
pub(crate) const MAX_COLUMNS: usize = 16_384;
/// The most characters a cell holds.
const MAX_TEXT: usize = 32_767;
/// The largest whole number a cell holds exactly (2^53).
pub(crate) const EXACT: i64 = 9_007_199_254_740_992;

/// A worksheet name a spreadsheet accepts: at most 31 characters, none of
/// `[ ] : * ? / \`, not starting or ending with an apostrophe, not empty.
pub fn sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '[' | ']' | ':' | '*' | '?' | '/' | '\\' => '_',
            c => c,
        })
        .take(31)
        .collect();
    let trimmed = cleaned.trim_matches('\'').trim();
    if trimmed.is_empty() {
        "Sheet1".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The table as an Excel workbook (`.xlsx`) of one worksheet: a bold header
/// row, kept in view when scrolling, and the values typed, as the
/// spreadsheet reader reads them back. Numbers are numbers, yes or no are
/// booleans, dates and dates with times are dates with a date format; text
/// is always text, even when it starts with `=`. A whole number too large
/// for a cell to hold exactly, a number that is not finite, a date before
/// 1900 (which Excel cannot hold) and a date and time with a zone are
/// written as text, so nothing is changed on the way.
pub fn to_xlsx(table: &Table, sheet: &str) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, XlsxError};

    if table.rows.len() + 1 > MAX_ROWS {
        return Err(tr!(
            "{rows} rows are more than a worksheet holds ({max})",
            rows = table.rows.len(),
            max = MAX_ROWS - 1
        ));
    }
    if table.columns.len() > MAX_COLUMNS {
        return Err(tr!(
            "{columns} columns are more than a worksheet holds ({max})",
            columns = table.columns.len(),
            max = MAX_COLUMNS
        ));
    }
    let too_long = |text: &str, row: usize, column: usize| {
        if text.chars().count() > MAX_TEXT {
            Err(tr!(
                "the text in row {row}, column {column} is longer than the {max} characters a \
                 cell holds",
                row = row + 1,
                column = table.columns[column],
                max = MAX_TEXT
            ))
        } else {
            Ok(())
        }
    };
    let failed = |e: XlsxError| tr!("the workbook could not be written: {e}", e = e);
    let excel_date = |d: &NaiveDate| {
        ExcelDateTime::from_ymd(d.year() as u16, d.month() as u8, d.day() as u8).map_err(failed)
    };

    let mut book = Workbook::new();
    let bold = Format::new().set_bold();
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let date_time = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
    let ws = book.add_worksheet();
    ws.set_name(sheet_name(sheet)).map_err(failed)?;
    for (c, name) in table.columns.iter().enumerate() {
        too_long(name, 0, c)?;
        ws.write_string_with_format(0, c as u16, name, &bold)
            .map_err(failed)?;
    }
    for (r, row) in table.rows.iter().enumerate() {
        let at = (r + 1) as u32;
        for (c, cell) in row.iter().enumerate() {
            let col = c as u16;
            let excel_year = |year: i32| (1900..=9999).contains(&year);
            match cell {
                Cell::Empty => continue,
                Cell::Int(i) if i.unsigned_abs() <= EXACT as u64 => {
                    ws.write_number(at, col, *i as f64)
                }
                Cell::Int(i) => ws.write_string(at, col, i.to_string()),
                Cell::Float(x) if x.is_finite() => ws.write_number(at, col, *x),
                Cell::Float(x) => ws.write_string(at, col, x.to_string()),
                Cell::Bool(b) => ws.write_boolean(at, col, *b),
                Cell::Date(d) if excel_year(d.year()) => {
                    ws.write_datetime_with_format(at, col, excel_date(d)?, &date)
                }
                Cell::Date(d) => ws.write_string(at, col, d.format("%Y-%m-%d").to_string()),
                Cell::DateTime(dt) if excel_year(dt.year()) => {
                    let seconds = dt.time().second() as f64 + dt.time().nanosecond() as f64 / 1e9;
                    let value = excel_date(&dt.date())?
                        .and_hms(dt.hour() as u16, dt.minute() as u8, seconds)
                        .map_err(failed)?;
                    ws.write_datetime_with_format(at, col, value, &date_time)
                }
                Cell::DateTime(dt) => {
                    ws.write_string(at, col, dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
                }
                Cell::Text(s) | Cell::ZonedDateTime(s) => {
                    too_long(s, r + 1, c)?;
                    ws.write_string(at, col, s)
                }
            }
            .map_err(failed)?;
        }
    }
    ws.set_freeze_panes(1, 0).map_err(failed)?;
    ws.autofit();
    book.save_to_buffer().map_err(failed)
}

#[cfg(test)]
#[path = "tabular_tests.rs"]
mod tabular_tests;
