use super::*;
use crate::delimited::infer_delimited;
use crate::{json, jsonschema};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "title": {"type": "string"},
    "orders": {"type": "array", "items": {"type": "object", "properties": {
      "sku": {"type": "string"},
      "qty": {"type": "integer"},
      "price": {"type": "number"},
      "paid": {"type": "boolean"},
      "ship": {"type": "object", "properties": {
        "city": {"type": "string"},
        "geo": {"type": "object", "properties": {"lat": {"type": "number"}}}
      }},
      "tags": {"type": "array", "items": {"type": "string"}},
      "extra": {"type": "object", "additionalProperties": {"type": "integer"}}
    }}},
    "meta": {"type": "object", "properties": {
      "notes": {"type": "array", "items": {"type": "object", "properties": {
        "text": {"type": "string"}
      }}}
    }},
    "codes": {"type": "array", "items": {"type": "string"}}
  }
}"#;

const DATA: &str = r#"{
  "title": "t",
  "orders": [
    {"sku": "A-1", "qty": 2, "price": 9.5, "paid": true,
     "ship": {"city": "Perth", "geo": {"lat": -31.9}}, "tags": ["x"], "extra": {"a": 1}},
    {"sku": "007", "qty": 1, "price": 24.0, "ship": {"city": "a, \"b\""}},
    {"sku": "=SUM(A1)", "qty": 3, "price": 1.25, "paid": false, "ship": {"city": " padded "}}
  ],
  "meta": {"notes": [{"text": "12"}, {"text": "true"}]},
  "codes": ["a"]
}"#;

fn book() -> (TypeSpec, Node) {
    let spec = jsonschema::lower_str(SCHEMA).unwrap();
    let data = json::parse(DATA).unwrap().node;
    (spec, data)
}

#[test]
fn the_arrays_of_objects_are_found_through_objects() {
    let (spec, _) = book();
    let found: Vec<String> = row_arrays(&spec).iter().map(|p| p.to_string()).collect();
    assert_eq!(found, ["orders", "meta.notes"]);
}

#[test]
fn nested_objects_are_dotted_columns_and_lists_are_left_out() {
    let (spec, data) = book();
    let t = table(&spec, &data, &"orders".parse().unwrap()).unwrap();
    assert_eq!(
        t.columns,
        ["sku", "qty", "price", "paid", "ship.city", "ship.geo.lat"]
    );
    assert_eq!(t.left_out, ["tags", "extra"]);
    assert_eq!(t.rows.len(), 3);
    assert_eq!(t.rows[0][5], Cell::Float(-31.9));
    assert_eq!(t.rows[1][3], Cell::Empty);
    // Not an array of objects.
    assert!(table(&spec, &data, &"codes".parse().unwrap()).is_none());
    assert!(table(&spec, &data, &Path::root()).is_none());
}

#[test]
fn csv_is_written_as_it_is_read_back() {
    let (spec, data) = book();
    let t = table(&spec, &data, &"orders".parse().unwrap()).unwrap();
    let text = to_delimited(&t, ',');
    assert_eq!(
        text,
        "sku,qty,price,paid,ship.city,ship.geo.lat\r\n\
         A-1,2,9.5,true,Perth,-31.9\r\n\
         007,1,24.0,,\"a, \"\"b\"\"\",\r\n\
         \"=SUM(A1)\",3,1.25,false,\" padded \",\r\n"
    );
    // Read back: the same values, and the same kinds of column.
    let back = infer_delimited(&text, ',');
    assert_eq!(
        json::to_string(&back.data, false).unwrap(),
        r#"[{"sku":"A-1","qty":2,"price":9.5,"paid":true,"ship.city":"Perth","ship.geo.lat":-31.9},{"sku":"007","qty":1,"price":24.0,"ship.city":"a, \"b\""},{"sku":"=SUM(A1)","qty":3,"price":1.25,"paid":false,"ship.city":" padded "}]"#
    );
}

#[test]
fn text_that_reads_as_a_number_is_quoted_and_tabs_separate_tsv() {
    let (spec, data) = book();
    let t = table(&spec, &data, &"meta.notes".parse().unwrap()).unwrap();
    let text = to_delimited(&t, '\t');
    assert_eq!(text, "text\r\n\"12\"\r\n\"true\"\r\n");
    let back = infer_delimited(&text, '\t');
    assert_eq!(
        json::to_string(&back.data, false).unwrap(),
        r#"[{"text":"12"},{"text":"true"}]"#
    );
}

#[test]
fn a_header_that_reads_as_a_number_is_quoted_so_it_stays_a_header() {
    let t = Table {
        columns: vec!["2026".to_owned(), "name".to_owned()],
        rows: vec![vec![Cell::Int(5), Cell::Text("a\tb".to_owned())]],
        left_out: Vec::new(),
    };
    assert_eq!(to_delimited(&t, '\t'), "\"2026\"\tname\r\n5\t\"a\tb\"\r\n");
    assert_eq!(to_delimited(&t, ','), "\"2026\",name\r\n5,a\tb\r\n");
}

/// A workbook written from the orders reads back, through the workbook
/// reader of the import, as the same values and kinds of column.
#[test]
fn an_xlsx_workbook_is_written_as_the_workbook_reader_reads_it_back() {
    let (spec, data) = book();
    let t = table(&spec, &data, &"orders".parse().unwrap()).unwrap();
    let bytes = to_xlsx(&t, "Orders").unwrap();
    let sheets = crate::spreadsheet::read(&bytes).unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].name, "Orders");
    assert_eq!(sheets[0].rows, 3);
    // Text stays text: 007, a formula-like =SUM(A1) and the padded city.
    // 24.0 stays a number, as the column holds 9.5 too.
    assert_eq!(
        json::to_string(&sheets[0].inference.data, false).unwrap(),
        r#"[{"sku":"A-1","qty":2,"price":9.5,"paid":true,"ship.city":"Perth","ship.geo.lat":-31.9},{"sku":"007","qty":1,"price":24.0,"ship.city":"a, \"b\""},{"sku":"=SUM(A1)","qty":3,"price":1.25,"paid":false,"ship.city":" padded "}]"#
    );
}

#[test]
fn dates_are_dates_in_a_workbook_and_what_a_cell_cannot_hold_is_text() {
    let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    let t = Table {
        columns: vec!["day".to_owned(), "at".to_owned(), "big".to_owned()],
        rows: vec![
            vec![
                Cell::Date(date(2026, 1, 31)),
                Cell::DateTime(date(2026, 1, 31).and_hms_opt(14, 30, 0).unwrap()),
                Cell::Int(i64::MAX),
            ],
            vec![
                Cell::Date(date(2026, 2, 1)),
                Cell::ZonedDateTime("2026-02-01T09:00:00+08:00".to_owned()),
                Cell::Int(5),
            ],
        ],
        left_out: Vec::new(),
    };
    let sheets = crate::spreadsheet::read(&to_xlsx(&t, "a/b: [c]").unwrap()).unwrap();
    assert_eq!(sheets[0].name, "a_b_ _c_");
    let read = &sheets[0].inference;
    assert_eq!(
        json::to_string(&read.data, false).unwrap(),
        r#"[{"day":"2026-01-31","at":"2026-01-31T14:30:00","big":"9223372036854775807"},{"day":"2026-02-01","at":"2026-02-01T09:00:00+08:00","big":"5"}]"#
    );
    // The day column is read back as dates.
    assert!(
        json::to_string(&read.schema, false)
            .unwrap()
            .contains(r#""day":{"type":"string","format":"date"}"#),
        "{}",
        json::to_string(&read.schema, false).unwrap()
    );
}

#[test]
fn a_sheet_name_is_one_a_spreadsheet_accepts() {
    assert_eq!(sheet_name("Orders"), "Orders");
    assert_eq!(sheet_name("''"), "Sheet1");
    assert_eq!(sheet_name(&"x".repeat(40)).len(), 31);
    assert_eq!(sheet_name("meta.notes"), "meta.notes");
}
