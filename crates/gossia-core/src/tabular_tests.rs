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
