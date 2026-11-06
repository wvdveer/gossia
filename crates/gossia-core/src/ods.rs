//! A table as an OpenDocument spreadsheet (`.ods`), for the export of a list
//! of records (DESIGN.md, "Excel and ODS files, 0.4.0"). There is no mature
//! Rust writer for it, so the file is written here: a zip of the `mimetype`
//! (first, not compressed, as the format requires), a manifest, the content
//! of one table and the view settings that keep the header row in view.
//!
//! The XML is written without white space between elements, which is also
//! what the workbook reader (`calamine`) needs to read the file back.

use std::fmt::Write as _;
use std::io::{Cursor, Write as _};

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use gossia_i18n::tr;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::tabular::{Cell, EXACT, MAX_COLUMNS, MAX_ROWS, Table, sheet_name};

const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

const MANIFEST: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">"#,
    r#"<manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>"#,
    r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#,
    r#"<manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/>"#,
    r#"</manifest:manifest>"#
);

/// The styles of the content: a bold header, the date formats, and a width
/// for each column (added after these).
const STYLES: &str = concat!(
    r#"<number:date-style style:name="N1"><number:year number:style="long"/><number:text>-</number:text><number:month number:style="long"/><number:text>-</number:text><number:day number:style="long"/></number:date-style>"#,
    r#"<number:date-style style:name="N2"><number:year number:style="long"/><number:text>-</number:text><number:month number:style="long"/><number:text>-</number:text><number:day number:style="long"/><number:text> </number:text><number:hours number:style="long"/><number:text>:</number:text><number:minutes number:style="long"/><number:text>:</number:text><number:seconds number:style="long"/></number:date-style>"#,
    r#"<style:style style:name="head" style:family="table-cell"><style:text-properties fo:font-weight="bold"/></style:style>"#,
    r#"<style:style style:name="date" style:family="table-cell" style:data-style-name="N1"/>"#,
    r#"<style:style style:name="datetime" style:family="table-cell" style:data-style-name="N2"/>"#,
);

/// The table as an `.ods` file of one sheet: a bold header row, kept in view
/// when scrolling, columns about as wide as their contents, and the values
/// typed as the workbook reader reads them back. Numbers are numbers, yes or
/// no are booleans, dates and dates with times are dates with a date format,
/// and text is always text. As in `.xlsx`, a whole number too large to be
/// held exactly, a number that is not finite and a date and time with a
/// zone are written as text, so nothing is changed on the way.
pub fn write(table: &Table, sheet: &str) -> Result<Vec<u8>, String> {
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
    let name = sheet_name(sheet);
    let failed = |e: &dyn std::fmt::Display| tr!("the workbook could not be written: {e}", e = e);
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default());
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default());
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let files: [(&str, SimpleFileOptions, String); 4] = [
        ("mimetype", stored, MIMETYPE.to_owned()),
        ("META-INF/manifest.xml", deflated, MANIFEST.to_owned()),
        ("content.xml", deflated, content(table, &name)),
        ("settings.xml", deflated, settings(&name)),
    ];
    for (path, options, text) in files {
        zip.start_file(path, options).map_err(|e| failed(&e))?;
        zip.write_all(text.as_bytes()).map_err(|e| failed(&e))?;
    }
    let cursor = zip.finish().map_err(|e| failed(&e))?;
    Ok(cursor.into_inner())
}

/// `content.xml`: the styles and the one table.
fn content(table: &Table, name: &str) -> String {
    let mut out = String::with_capacity(256 + table.rows.len() * table.columns.len() * 48);
    out.push_str(concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
        r#" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0""#,
        r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#,
        r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
        r#" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0""#,
        r#" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#,
        r#" office:version="1.3"><office:automatic-styles>"#,
    ));
    out.push_str(STYLES);
    for (c, width) in widths(table).iter().enumerate() {
        let _ = write!(
            out,
            r#"<style:style style:name="co{c}" style:family="table-column"><style:table-column-properties style:column-width="{width:.2}cm"/></style:style>"#
        );
    }
    out.push_str("</office:automatic-styles><office:body><office:spreadsheet>");
    let _ = write!(out, r#"<table:table table:name="{}">"#, escape(name));
    for c in 0..table.columns.len() {
        let _ = write!(out, r#"<table:table-column table:style-name="co{c}"/>"#);
    }
    out.push_str("<table:table-row>");
    for column in &table.columns {
        out.push_str(r#"<table:table-cell table:style-name="head" office:value-type="string">"#);
        out.push_str(&paragraph(column));
        out.push_str("</table:table-cell>");
    }
    out.push_str("</table:table-row>");
    for row in &table.rows {
        out.push_str("<table:table-row>");
        for cell in row {
            cell_xml(&mut out, cell);
        }
        out.push_str("</table:table-row>");
    }
    out.push_str("</table:table></office:spreadsheet></office:body></office:document-content>");
    out
}

/// A year a date value of the format holds, written with four digits.
fn four_digit_year(year: i32) -> bool {
    (1..=9999).contains(&year)
}

fn date_text(d: &NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn date_time_text(dt: &NaiveDateTime) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
}

fn text_cell(out: &mut String, text: &str) {
    out.push_str(r#"<table:table-cell office:value-type="string">"#);
    out.push_str(&paragraph(text));
    out.push_str("</table:table-cell>");
}

fn cell_xml(out: &mut String, cell: &Cell) {
    match cell {
        Cell::Empty => out.push_str("<table:table-cell/>"),
        Cell::Int(i) if i.unsigned_abs() <= EXACT as u64 => {
            let _ = write!(
                out,
                r#"<table:table-cell office:value-type="float" office:value="{i}"><text:p>{i}</text:p></table:table-cell>"#
            );
        }
        Cell::Float(x) if x.is_finite() => {
            let _ = write!(
                out,
                r#"<table:table-cell office:value-type="float" office:value="{x}"><text:p>{x}</text:p></table:table-cell>"#
            );
        }
        Cell::Int(i) => text_cell(out, &i.to_string()),
        Cell::Float(x) => text_cell(out, &x.to_string()),
        Cell::Bool(b) => {
            let shown = if *b { "TRUE" } else { "FALSE" };
            let _ = write!(
                out,
                r#"<table:table-cell office:value-type="boolean" office:boolean-value="{b}"><text:p>{shown}</text:p></table:table-cell>"#
            );
        }
        Cell::Date(d) if four_digit_year(d.year()) => {
            let text = date_text(d);
            let _ = write!(
                out,
                r#"<table:table-cell table:style-name="date" office:value-type="date" office:date-value="{text}"><text:p>{text}</text:p></table:table-cell>"#
            );
        }
        Cell::DateTime(dt) if four_digit_year(dt.year()) => {
            let value = date_time_text(dt);
            let shown = dt.format("%Y-%m-%d %H:%M:%S");
            let _ = write!(
                out,
                r#"<table:table-cell table:style-name="datetime" office:value-type="date" office:date-value="{value}"><text:p>{shown}</text:p></table:table-cell>"#
            );
        }
        Cell::Date(d) => text_cell(out, &date_text(d)),
        Cell::DateTime(dt) => text_cell(out, &date_time_text(dt)),
        Cell::Text(s) | Cell::ZonedDateTime(s) => text_cell(out, s),
    }
}

/// A column's width: about as wide as its longest value, within limits.
fn widths(table: &Table) -> Vec<f64> {
    let shown = |cell: &Cell| match cell {
        Cell::Empty => 0,
        Cell::Int(i) => i.to_string().len(),
        Cell::Float(x) => x.to_string().len(),
        Cell::Bool(_) => 5,
        Cell::Date(_) => 10,
        Cell::DateTime(_) => 19,
        Cell::Text(s) | Cell::ZonedDateTime(s) => {
            s.lines().map(|l| l.chars().count()).max().unwrap_or(0)
        }
    };
    (0..table.columns.len())
        .map(|c| {
            let longest = table
                .rows
                .iter()
                .filter_map(|row| row.get(c).map(shown))
                .chain([table.columns[c].chars().count()])
                .max()
                .unwrap_or(0)
                .clamp(4, 60);
            longest as f64 * 0.22 + 0.4
        })
        .collect()
}

/// `settings.xml`: the view of the sheet, with the header row frozen.
fn settings(name: &str) -> String {
    let name = escape(name);
    let item = |key: &str, kind: &str, value: &str| {
        format!(
            r#"<config:config-item config:name="{key}" config:type="{kind}">{value}</config:config-item>"#
        )
    };
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            r#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
            r#" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0" office:version="1.3">"#,
            r#"<office:settings><config:config-item-set config:name="ooo:view-settings">"#,
            r#"<config:config-item-map-indexed config:name="Views"><config:config-item-map-entry>"#,
            "{view}",
            r#"<config:config-item-map-named config:name="Tables">"#,
            r#"<config:config-item-map-entry config:name="{name}">{split}</config:config-item-map-entry>"#,
            r#"</config:config-item-map-named>{active}"#,
            r#"</config:config-item-map-entry></config:config-item-map-indexed>"#,
            r#"</config:config-item-set></office:settings></office:document-settings>"#,
        ),
        view = item("ViewId", "string", "view1"),
        name = name,
        split = [
            item("VerticalSplitMode", "short", "2"),
            item("VerticalSplitPosition", "int", "1"),
            item("ActiveSplitRange", "short", "2"),
            item("PositionTop", "int", "0"),
            item("PositionBottom", "int", "1"),
        ]
        .concat(),
        active = item("ActiveTable", "string", &name),
    )
}

/// Text with the characters XML gives a meaning escaped; a character XML
/// cannot hold at all (a control character) becomes U+FFFD.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

/// Text as a paragraph of a cell, every space kept: a spreadsheet collapses
/// runs of spaces and drops those at the start and end, so these are
/// written as `<text:s/>`, a tab as `<text:tab/>` and a line break as
/// `<text:line-break/>`.
fn paragraph(text: &str) -> String {
    let mut out = String::from("<text:p>");
    let chars: Vec<char> = text.chars().collect();
    // At the start of the paragraph or after a tab or a line break.
    let mut fresh = true;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ' ' => {
                let start = i;
                while i < chars.len() && chars[i] == ' ' {
                    i += 1;
                }
                let n = i - start;
                let trailing = chars.get(i).is_none_or(|c| matches!(c, '\t' | '\n' | '\r'));
                let spaces = |out: &mut String, n: usize| match n {
                    0 => {}
                    1 => out.push_str("<text:s/>"),
                    n => {
                        let _ = write!(out, r#"<text:s text:c="{n}"/>"#);
                    }
                };
                if fresh || trailing {
                    spaces(&mut out, n);
                } else {
                    out.push(' ');
                    spaces(&mut out, n - 1);
                }
                fresh = false;
                continue;
            }
            '\t' => {
                out.push_str("<text:tab/>");
                fresh = true;
            }
            '\r' if chars.get(i + 1) == Some(&'\n') => {}
            '\n' | '\r' => {
                out.push_str("<text:line-break/>");
                fresh = true;
            }
            c => {
                out.push_str(&escape(&c.to_string()));
                fresh = false;
            }
        }
        i += 1;
    }
    out.push_str("</text:p>");
    out
}

#[cfg(test)]
#[path = "ods_tests.rs"]
mod ods_tests;
