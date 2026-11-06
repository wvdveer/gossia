//! Workbooks are made here, not kept as files: `.xlsx` with
//! `rust_xlsxwriter`, `.ods` by zipping its XML.

use std::io::Write;

use rust_xlsxwriter::{ExcelDateTime, Format, Formula, Workbook};

use super::*;
use crate::json;

fn field(sheet: &SheetRead, row: usize, name: &str) -> Option<Value> {
    let Value::Array(items) = sheet.inference.data.value() else {
        panic!("an array");
    };
    let Value::Object(fields) = items[row].value() else {
        panic!("an object");
    };
    fields.get(name).map(|n| n.value().clone())
}

fn schema_of(sheet: &SheetRead, name: &str) -> String {
    let Value::Object(top) = sheet.inference.schema.value() else {
        panic!()
    };
    let Value::Object(items) = top["items"].value() else {
        panic!()
    };
    let Value::Object(fields) = items["properties"].value() else {
        panic!()
    };
    json::to_string(&fields[name], false).unwrap()
}

fn field_names(sheet: &SheetRead) -> Vec<String> {
    let Value::Object(top) = sheet.inference.schema.value() else {
        panic!()
    };
    let Value::Object(items) = top["items"].value() else {
        panic!()
    };
    let Value::Object(fields) = items["properties"].value() else {
        panic!()
    };
    fields.keys().cloned().collect()
}

/// A row of the orders: sku, qty, price, paid, day and the hour of `at`.
type Row = (&'static str, f64, f64, bool, (u16, u8, u8), u16);

/// Two sheets: orders with a header, typed columns, a date column, a column
/// of dates with times, a formula, an error and a code with a leading zero;
/// and an empty one.
fn orders_xlsx() -> Vec<u8> {
    let mut book = Workbook::new();
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let stamp = Format::new().set_num_format("yyyy-mm-dd hh:mm");
    let sheet = book.add_worksheet();
    sheet.set_name("Orders").unwrap();
    // The table starts at B3.
    for (col, name) in ["sku", "qty", "price", "paid", "day", "at", "total", "code"]
        .iter()
        .enumerate()
    {
        sheet.write_string(2, 1 + col as u16, *name).unwrap();
    }
    let rows: [Row; 3] = [
        ("A-100", 2.0, 9.5, true, (2026, 1, 31), 9),
        ("B-220", 1.0, 24.0, false, (2026, 2, 1), 14),
        ("C-3", 5.0, 1.0, true, (2026, 2, 2), 0),
    ];
    for (i, (sku, qty, price, paid, (y, m, d), hour)) in rows.iter().enumerate() {
        let r = 3 + i as u32;
        sheet.write_string(r, 1, *sku).unwrap();
        sheet.write_number(r, 2, *qty).unwrap();
        sheet.write_number(r, 3, *price).unwrap();
        sheet.write_boolean(r, 4, *paid).unwrap();
        let day = ExcelDateTime::from_ymd(*y, *m, *d).unwrap();
        sheet.write_datetime_with_format(r, 5, &day, &date).unwrap();
        let at = day.clone().and_hms(*hour, 30, 0).unwrap();
        sheet.write_datetime_with_format(r, 6, &at, &stamp).unwrap();
        let total = qty * price;
        sheet
            .write_formula(
                r,
                7,
                Formula::new(format!("=C{0}*D{0}", r + 1)).set_result(total.to_string()),
            )
            .unwrap();
    }
    sheet.write_string(3, 8, "007").unwrap();
    // An error where a formula failed.
    sheet
        .write_formula(4, 8, Formula::new("=NA()").set_result("#N/A"))
        .unwrap();
    book.add_worksheet().set_name("Empty").unwrap();
    book.save_to_buffer().unwrap()
}

#[test]
fn a_sheet_is_read_as_csv_is_with_its_cells_typed() {
    let sheets = read(&orders_xlsx()).unwrap();
    assert_eq!(
        sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Orders", "Empty"]
    );
    let orders = &sheets[0];
    assert_eq!(orders.rows, 3);
    assert_eq!(
        field_names(orders),
        ["sku", "qty", "price", "paid", "day", "at", "total", "code"]
    );
    assert_eq!(field(orders, 0, "sku"), Some(Value::Str("A-100".into())));
    assert_eq!(field(orders, 0, "qty"), Some(Value::Int(2)));
    assert_eq!(field(orders, 0, "price"), Some(Value::Float(9.5)));
    assert_eq!(field(orders, 1, "paid"), Some(Value::Bool(false)));
    assert_eq!(
        field(orders, 0, "day"),
        Some(Value::Str("2026-01-31".into()))
    );
    assert_eq!(
        schema_of(orders, "day"),
        r#"{"type":"string","format":"date"}"#
    );
    assert_eq!(
        field(orders, 1, "at"),
        Some(Value::Str("2026-02-01T14:30:00".into()))
    );
    assert_eq!(schema_of(orders, "at"), r#"{"type":"string"}"#);
    // The value the formula last gave.
    assert_eq!(field(orders, 1, "total"), Some(Value::Int(24)));
    // Text stays text, and an error is left out.
    assert_eq!(field(orders, 0, "code"), Some(Value::Str("007".into())));
    assert_eq!(field(orders, 1, "code"), None);
    let report = orders.inference.warnings.join("\n");
    assert!(report.contains("names of the columns"), "{report}");
    assert!(
        report.contains("dates with times") && report.contains(": at."),
        "{report}"
    );
    assert!(
        report.contains("formula") && report.contains(": 4."),
        "{report}"
    );
    assert!(report.contains("#N/A: 1."), "{report}");
    assert_eq!(sheets[1].rows, 0);
}

#[test]
fn a_first_row_of_numbers_is_data_and_mixed_columns_are_text() {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.write_number(0, 0, 1.0).unwrap();
    sheet.write_string(0, 1, "one").unwrap();
    sheet.write_number(1, 0, 2.5).unwrap();
    sheet.write_number(1, 1, 2.0).unwrap();
    // A blank row in between is passed over; a cell past the others is a
    // column of its own.
    sheet.write_number(3, 0, 3.0).unwrap();
    sheet.write_boolean(3, 2, true).unwrap();
    let sheets = read(&book.save_to_buffer().unwrap()).unwrap();
    let s = &sheets[0];
    assert_eq!(s.rows, 3);
    assert_eq!(field_names(s), ["column_1", "column_2", "column_3"]);
    assert_eq!(field(s, 0, "column_1"), Some(Value::Float(1.0)));
    assert_eq!(field(s, 1, "column_2"), Some(Value::Str("2".into())));
    assert_eq!(field(s, 2, "column_3"), Some(Value::Bool(true)));
    assert!(s.inference.warnings[0].contains("is data, not names"));
}

/// An OpenDocument spreadsheet of one table, as LibreOffice writes one.
fn ods(table: &str) -> Vec<u8> {
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  office:version="1.2">
<office:body><office:spreadsheet>{}</office:spreadsheet></office:body>
</office:document-content>"#,
        // As LibreOffice writes it: nothing between the cells.
        table.lines().map(str::trim).collect::<String>()
    );
    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
 <manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#;
    let mut out = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/vnd.oasis.opendocument.spreadsheet")
        .unwrap();
    zip.start_file("META-INF/manifest.xml", stored).unwrap();
    zip.write_all(manifest.as_bytes()).unwrap();
    zip.start_file("content.xml", stored).unwrap();
    zip.write_all(content.as_bytes()).unwrap();
    zip.finish().unwrap();
    out.into_inner()
}

#[test]
fn an_ods_file_is_read_with_its_dates() {
    let table = r#"<table:table table:name="People">
<table:table-row>
 <table:table-cell office:value-type="string"><text:p>name</text:p></table:table-cell>
 <table:table-cell office:value-type="string"><text:p>born</text:p></table:table-cell>
 <table:table-cell office:value-type="string"><text:p>age</text:p></table:table-cell>
</table:table-row>
<table:table-row>
 <table:table-cell office:value-type="string"><text:p>Ada</text:p></table:table-cell>
 <table:table-cell office:value-type="date" office:date-value="1815-12-10"><text:p>10/12/1815</text:p></table:table-cell>
 <table:table-cell office:value-type="float" office:value="36"><text:p>36</text:p></table:table-cell>
</table:table-row>
</table:table>"#;
    let sheets = read(&ods(table)).unwrap();
    assert_eq!(sheets.len(), 1);
    let s = &sheets[0];
    assert_eq!(s.name, "People");
    assert_eq!(field(s, 0, "name"), Some(Value::Str("Ada".into())));
    assert_eq!(field(s, 0, "born"), Some(Value::Str("1815-12-10".into())));
    assert_eq!(schema_of(s, "born"), r#"{"type":"string","format":"date"}"#);
    assert_eq!(field(s, 0, "age"), Some(Value::Int(36)));
}

#[test]
fn what_is_not_a_workbook_is_refused() {
    assert!(read(b"name,qty\na,1\n").is_err());
}
