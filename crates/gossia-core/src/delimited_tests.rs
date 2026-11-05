//! Tests of reading CSV and TSV (DESIGN.md, "CSV and TSV").

use crate::delimited::{infer_delimited, rows};
use crate::validate::validate;
use crate::value::{Node, Value};
use crate::{json, jsonschema};

fn data(text: &str, separator: char) -> String {
    json::to_string(&infer_delimited(text, separator).data, false).unwrap()
}

fn schema(text: &str, separator: char) -> String {
    json::to_string(&infer_delimited(text, separator).schema, false).unwrap()
}

#[test]
fn a_header_row_names_the_fields_and_each_column_is_typed_as_a_whole() {
    let text = "sku,qty,price,gift,code\nA,2,9.5,true,007\nB,1,24,FALSE,12\n";
    assert_eq!(
        data(text, ','),
        r#"[{"sku":"A","qty":2,"price":9.5,"gift":true,"code":"007"},{"sku":"B","qty":1,"price":24.0,"gift":false,"code":"12"}]"#
    );
    assert_eq!(
        schema(text, ','),
        r#"{"type":"array","items":{"type":"object","properties":{"sku":{"type":"string"},"qty":{"type":"integer"},"price":{"type":"number"},"gift":{"type":"boolean"},"code":{"type":"string"}},"required":["sku","qty","price","gift","code"]}}"#
    );
    let inferred = infer_delimited(text, ',');
    let spec = jsonschema::lower(&inferred.schema).unwrap();
    assert!(validate(&spec, &inferred.data).is_empty());
    assert_eq!(
        inferred.warnings,
        ["The first row is taken as the names of the columns."]
    );
}

#[test]
fn a_first_row_with_a_number_is_data_and_the_columns_are_numbered() {
    let text = "A\t2\t9.5\nB\t1\t24\n";
    assert_eq!(
        data(text, '\t'),
        r#"[{"column_1":"A","column_2":2,"column_3":9.5},{"column_1":"B","column_2":1,"column_3":24.0}]"#
    );
    assert!(infer_delimited(text, '\t').warnings[0].contains("column_1"));
    // A yes or no in the first row is data as well.
    assert!(data("true,x\nfalse,y", ',').starts_with(r#"[{"column_1":true"#));
}

#[test]
fn quotes_hold_separators_line_breaks_and_quotes() {
    let (read, open) = rows("a,\"b, \"\"c\"\"\nd\",e\r\n\r\nf,g", ',');
    assert!(!open);
    let texts: Vec<Vec<&str>> = read
        .iter()
        .map(|r| r.iter().map(|c| c.text.as_str()).collect())
        .collect();
    assert_eq!(texts, [vec!["a", "b, \"c\"\nd", "e"], vec!["f", "g"]]);
    // A quoted number is text, and an unquoted cell is trimmed.
    assert_eq!(
        data("id, name\n\"1\", Ann \n2,Bob", ','),
        r#"[{"id":"1","name":"Ann"},{"id":"2","name":"Bob"}]"#
    );
    let (_, open) = rows("a,\"b\nc", ',');
    assert!(open);
}

#[test]
fn empty_cells_leave_the_field_out_and_ragged_rows_get_names() {
    let text = "a,b,c\n1,,3\n4,5\n6,7,8,9\n";
    let inferred = infer_delimited(text, ',');
    assert_eq!(
        json::to_string(&inferred.data, false).unwrap(),
        r#"[{"a":1,"c":3},{"a":4,"b":5},{"a":6,"b":7,"c":8,"column_4":9}]"#
    );
    // The fields keep the order of the columns, and only `a` is in every row.
    assert_eq!(
        json::to_string(&inferred.schema, false).unwrap(),
        r#"{"type":"array","items":{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"},"c":{"type":"integer"},"column_4":{"type":"integer"}},"required":["a"]}}"#
    );
    assert!(
        inferred.warnings.iter().any(|w| w.contains("column_4")),
        "{:?}",
        inferred.warnings
    );
}

#[test]
fn blank_and_duplicate_header_names_are_made_unique() {
    assert_eq!(
        data(",x,x\n1,2,3", ','),
        r#"[{"column_1":1,"x":2,"x_2":3}]"#
    );
    // A mix of whole numbers and decimals is numbers; a mix with text is text.
    assert_eq!(
        data("n,m\n1,1\n2.5,x", ','),
        r#"[{"n":1.0,"m":"1"},{"n":2.5,"m":"x"}]"#
    );
    assert_eq!(
        infer_delimited("", ',').data,
        Node::new(Value::Array(Vec::new()))
    );
}
