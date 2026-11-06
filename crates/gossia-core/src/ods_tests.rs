use super::*;
use crate::json;
use crate::spreadsheet;
use crate::tabular::{Cell, Table};

fn table() -> Table {
    let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    Table {
        columns: vec![
            "sku".to_owned(),
            "qty".to_owned(),
            "price".to_owned(),
            "paid".to_owned(),
            "day".to_owned(),
            "at".to_owned(),
            "note".to_owned(),
        ],
        rows: vec![
            vec![
                Cell::Text("A-1".to_owned()),
                Cell::Int(2),
                Cell::Float(9.5),
                Cell::Bool(true),
                Cell::Date(date(2026, 1, 31)),
                Cell::DateTime(date(2026, 1, 31).and_hms_opt(14, 30, 0).unwrap()),
                Cell::Text("  two  spaces\tand a tab\nand a line <&> \"q\"".to_owned()),
            ],
            vec![
                Cell::Text("007".to_owned()),
                Cell::Int(1),
                Cell::Float(24.0),
                Cell::Empty,
                Cell::Date(date(2026, 2, 1)),
                Cell::ZonedDateTime("2026-02-01T09:00:00+08:00".to_owned()),
                Cell::Text("=SUM(A1)".to_owned()),
            ],
        ],
        left_out: Vec::new(),
    }
}

/// What is written reads back, through the workbook reader of the import, as
/// the same values: text with its spaces, tabs and line breaks, numbers,
/// booleans and dates.
#[test]
fn an_ods_file_is_written_as_the_workbook_reader_reads_it_back() {
    let bytes = write(&table(), "Orders").unwrap();
    let sheets = spreadsheet::read(&bytes).unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].name, "Orders");
    assert_eq!(sheets[0].rows, 2);
    let read = &sheets[0].inference;
    assert_eq!(
        json::to_string(&read.data, false).unwrap(),
        r#"[{"sku":"A-1","qty":2,"price":9.5,"paid":true,"day":"2026-01-31","at":"2026-01-31T14:30:00","note":"  two  spaces\tand a tab\nand a line <&> \"q\""},{"sku":"007","qty":1,"price":24.0,"day":"2026-02-01","at":"2026-02-01T09:00:00+08:00","note":"=SUM(A1)"}]"#
    );
    assert!(
        json::to_string(&read.schema, false)
            .unwrap()
            .contains(r#""day":{"type":"string","format":"date"}"#)
    );
}

/// The `mimetype` comes first and is not compressed, so that a program can
/// tell the kind of file from its first bytes.
#[test]
fn the_mimetype_comes_first_uncompressed() {
    let bytes = write(&table(), "Orders").unwrap();
    assert_eq!(&bytes[30..38], b"mimetype");
    assert_eq!(&bytes[38..38 + MIMETYPE.len()], MIMETYPE.as_bytes());
}

#[test]
fn spaces_at_the_ends_and_in_runs_are_kept_as_text_s() {
    assert_eq!(paragraph("a b"), "<text:p>a b</text:p>");
    assert_eq!(paragraph(" a"), "<text:p><text:s/>a</text:p>");
    assert_eq!(
        paragraph("a  b "),
        r#"<text:p>a <text:s/>b<text:s/></text:p>"#
    );
    assert_eq!(
        paragraph("a\r\n   b"),
        r#"<text:p>a<text:line-break/><text:s text:c="3"/>b</text:p>"#
    );
    assert_eq!(paragraph("x\u{1}"), "<text:p>x\u{FFFD}</text:p>");
}
