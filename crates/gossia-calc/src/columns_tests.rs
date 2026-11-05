//! Tests of column formulas (DESIGN.md, "Array and object formulas", "Column
//! formulas").

use std::sync::Arc;

use gossia_core::{Path, Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, EditError, Meta};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "qty": {"type": "integer"},
          "price": {"type": "number"},
          "total": {"type": "number"}
        },
        "required": ["qty", "price"]
      }
    }
  },
  "required": ["items"]
}"#;

const DATA: &str = r#"{"items": [
  {"qty": 2, "price": 1.5, "total": 99},
  {"qty": 3, "price": 2.0},
  {"qty": 4, "price": 0.5}
]}"#;

const TOTAL: &str = "parent.qty * parent.price";

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Orders",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.add_schema_sheet("OrdersSchema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.bind_schema("Orders", "OrdersSchema").unwrap();
    calc
}

fn run(calc: &mut Calc) {
    calc.recalc(&PythonEngine::new());
}

fn value(calc: &Calc, path: &str) -> Option<Value> {
    calc.data("Orders")
        .unwrap()
        .get(&p(path))
        .map(|n| n.value().clone())
}

fn number(calc: &Calc, path: &str) -> f64 {
    match value(calc, path) {
        Some(Value::Int(n)) => n as f64,
        Some(Value::Float(n)) => n,
        other => panic!("{path} is {other:?}"),
    }
}

fn column(calc: &mut Calc) {
    calc.set_column_formula("Orders", &p("items"), "total", TOTAL)
        .unwrap();
    run(calc);
}

#[test]
fn a_column_formula_gives_every_item_the_formula_and_replaces_what_was_there() {
    let mut calc = book();
    column(&mut calc);
    assert_eq!(calc.formulas("Orders").len(), 3);
    assert_eq!(number(&calc, "items[0].total"), 3.0);
    assert_eq!(number(&calc, "items[1].total"), 6.0);
    assert_eq!(number(&calc, "items[2].total"), 2.0);
    assert_eq!(
        calc.column_formula("Orders", &p("items"), "total"),
        Some(TOTAL)
    );
    assert!(calc.column_of("Orders", &p("items[1].total")).is_some());
    assert!(calc.column_of("Orders", &p("items[1].qty")).is_none());
}

#[test]
fn an_item_added_later_has_the_formula() {
    let mut calc = book();
    column(&mut calc);
    let item = json::parse(r#"{"qty": 5, "price": 3.0}"#).unwrap().node;
    calc.insert_item("Orders", &p("items"), 3, item.value().clone())
        .unwrap();
    run(&mut calc);
    assert_eq!(calc.formulas("Orders").len(), 4);
    assert_eq!(number(&calc, "items[3].total"), 15.0);
    // At the front: every place has a formula and the new last place gets one.
    let item = json::parse(r#"{"qty": 1, "price": 1.0}"#).unwrap().node;
    calc.insert_item("Orders", &p("items"), 0, item.value().clone())
        .unwrap();
    run(&mut calc);
    assert_eq!(calc.formulas("Orders").len(), 5);
    assert_eq!(number(&calc, "items[0].total"), 1.0);
    assert_eq!(number(&calc, "items[4].total"), 15.0);
    // One undo takes the item and its formula back.
    calc.undo().unwrap();
    run(&mut calc);
    assert_eq!(calc.formulas("Orders").len(), 4);
    assert_eq!(number(&calc, "items[0].total"), 3.0);
}

#[test]
fn removing_an_item_removes_the_formula_past_the_end() {
    let mut calc = book();
    column(&mut calc);
    calc.remove("Orders", &p("items[1]")).unwrap();
    run(&mut calc);
    assert_eq!(calc.formulas("Orders").len(), 2);
    assert_eq!(number(&calc, "items[1].total"), 2.0);
    calc.undo().unwrap();
    run(&mut calc);
    assert_eq!(calc.formulas("Orders").len(), 3);
    assert_eq!(number(&calc, "items[1].total"), 6.0);
}

#[test]
fn a_literal_in_one_item_ends_the_column_and_the_others_keep_their_formula() {
    let mut calc = book();
    column(&mut calc);
    calc.set_value("Orders", &p("items[1].total"), Value::Int(7))
        .unwrap();
    run(&mut calc);
    assert_eq!(calc.column_formula("Orders", &p("items"), "total"), None);
    assert_eq!(calc.formulas("Orders").len(), 2);
    assert_eq!(number(&calc, "items[1].total"), 7.0);
    assert_eq!(number(&calc, "items[0].total"), 3.0);
    // Items added afterwards have the field blank.
    let item = json::parse(r#"{"qty": 5, "price": 3.0}"#).unwrap().node;
    calc.insert_item("Orders", &p("items"), 3, item.value().clone())
        .unwrap();
    run(&mut calc);
    assert_eq!(calc.formulas("Orders").len(), 2);
    assert_eq!(value(&calc, "items[3].total"), None);
    // One undo brings the column formula back.
    calc.undo().unwrap();
    calc.undo().unwrap();
    run(&mut calc);
    assert_eq!(
        calc.column_formula("Orders", &p("items"), "total"),
        Some(TOTAL)
    );
    assert_eq!(calc.formulas("Orders").len(), 3);
    assert_eq!(number(&calc, "items[1].total"), 6.0);
}

#[test]
fn a_different_formula_in_one_item_ends_the_column_and_the_same_one_does_not() {
    let mut calc = book();
    column(&mut calc);
    calc.set_formula("Orders", &p("items[0].total"), TOTAL)
        .unwrap();
    assert!(
        calc.column_formula("Orders", &p("items"), "total")
            .is_some()
    );
    calc.set_formula("Orders", &p("items[0].total"), "parent.qty")
        .unwrap();
    run(&mut calc);
    assert_eq!(calc.column_formula("Orders", &p("items"), "total"), None);
    assert_eq!(calc.formulas("Orders").len(), 3);
    assert_eq!(number(&calc, "items[0].total"), 2.0);
    assert_eq!(number(&calc, "items[1].total"), 6.0);
}

#[test]
fn clearing_one_item_ends_the_column_too() {
    let mut calc = book();
    column(&mut calc);
    assert!(calc.remove_formula("Orders", &p("items[2].total")).unwrap());
    assert_eq!(calc.column_formula("Orders", &p("items"), "total"), None);
    assert_eq!(calc.formulas("Orders").len(), 2);
}

#[test]
fn removing_the_column_formula_keeps_the_values() {
    let mut calc = book();
    column(&mut calc);
    assert!(
        calc.remove_column_formula("Orders", &p("items"), "total")
            .unwrap()
    );
    assert!(calc.formulas("Orders").is_empty());
    assert_eq!(number(&calc, "items[1].total"), 6.0);
    assert!(
        !calc
            .remove_column_formula("Orders", &p("items"), "total")
            .unwrap()
    );
    calc.undo().unwrap();
    assert_eq!(calc.formulas("Orders").len(), 3);
    assert!(
        calc.column_formula("Orders", &p("items"), "total")
            .is_some()
    );
}

#[test]
fn creating_and_changing_a_column_formula_is_one_entry_each() {
    let mut calc = book();
    assert!(!calc.can_undo());
    column(&mut calc);
    assert_eq!(calc.history_len().0, 1);
    calc.set_column_formula("Orders", &p("items"), "total", "parent.qty")
        .unwrap();
    assert_eq!(calc.history_len().0, 2);
    run(&mut calc);
    assert_eq!(number(&calc, "items[1].total"), 3.0);
    calc.undo().unwrap();
    assert_eq!(
        calc.column_formula("Orders", &p("items"), "total"),
        Some(TOTAL)
    );
    run(&mut calc);
    assert_eq!(number(&calc, "items[1].total"), 6.0);
    calc.undo().unwrap();
    assert_eq!(calc.column_formula("Orders", &p("items"), "total"), None);
    assert!(calc.formulas("Orders").is_empty());
    // The first value the field held comes back.
    assert_eq!(number(&calc, "items[0].total"), 99.0);
    calc.redo().unwrap();
    assert!(
        calc.column_formula("Orders", &p("items"), "total")
            .is_some()
    );
}

#[test]
fn what_cannot_be_a_column_is_refused() {
    let mut calc = book();
    let refused = |calc: &mut Calc, array: &str, field: &str| {
        calc.set_column_formula("Orders", &p(array), field, "1")
            .unwrap_err()
    };
    assert!(matches!(
        refused(&mut calc, "items", "nothing"),
        EditError::NotInSchema(_)
    ));
    assert!(matches!(
        refused(&mut calc, "items", "a.b"),
        EditError::Path(_)
    ));
    assert!(matches!(
        refused(&mut calc, "items[0]", "qty"),
        EditError::Path(_)
    ));
    assert!(matches!(
        refused(&mut calc, "missing", "qty"),
        EditError::NotInSchema(_)
    ));
    // Nothing changed.
    assert!(!calc.can_undo());
    assert!(calc.formulas("Orders").is_empty());
    // A formula inside an item's field stops it.
    calc.set_formula("Orders", &p("items[0]"), "{}").unwrap();
    assert!(matches!(
        refused(&mut calc, "items", "total"),
        EditError::InsideComputed { .. }
    ));
}

#[test]
fn the_trust_record_has_one_item_for_the_column() {
    let mut calc = book();
    column(&mut calc);
    let items: Vec<String> = calc.fingerprint().items.keys().cloned().collect();
    assert_eq!(items, ["column formula Orders!items[].total"]);
}

#[test]
fn a_column_is_saved_once_and_comes_back_with_a_formula_in_every_item() {
    let mut calc = book();
    column(&mut calc);
    let package = calc.to_package(&Meta::default()).unwrap();
    let overlay =
        String::from_utf8(package.get("sheets/Orders/overlay.json").unwrap().to_vec()).unwrap();
    assert_eq!(overlay.matches(TOTAL).count(), 1, "{overlay}");

    let opened = crate::Calc::from_package(Arc::new(PythonSyntax), &package).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let back = opened.calc;
    assert_eq!(back.formulas("Orders").len(), 3);
    assert_eq!(
        back.column_formula("Orders", &p("items"), "total"),
        Some(TOTAL)
    );
    assert_eq!(back.pending(), 0, "nothing is out of date after opening");
    assert_eq!(number(&back, "items[2].total"), 2.0);
    // The same bytes again.
    let again = back.to_package(&Meta::default()).unwrap();
    assert!(again == package, "the second save differs");
}

#[test]
fn a_column_item_in_error_is_saved_with_its_error() {
    let mut calc = book();
    calc.set_column_formula("Orders", &p("items"), "total", "1 / (parent.qty - 3)")
        .unwrap();
    run(&mut calc);
    let package = calc.to_package(&Meta::default()).unwrap();
    let opened = crate::Calc::from_package(Arc::new(PythonSyntax), &package).unwrap();
    let back = opened.calc;
    assert!(
        matches!(value(&back, "items[1].total"), Some(Value::Error(_))),
        "{:?}",
        value(&back, "items[1].total")
    );
    assert_eq!(back.formulas("Orders").len(), 3);
}

/// Formulas, "A formula over formulas inside": when the author says yes, a
/// formula on the array replaces the column formula inside it, as one entry
/// that undo takes back whole.
#[test]
fn a_formula_can_replace_the_column_formula_inside_it_and_undo_restores_both() {
    let mut calc = book();
    column(&mut calc);
    assert!(matches!(
        calc.set_formula("Orders", &p("items"), "[]"),
        Err(EditError::ContainsFormulas { .. })
    ));
    assert_eq!(calc.formulas_inside("Orders", &p("items")).len(), 3);
    calc.set_formula_replacing("Orders", &p("items"), "[]")
        .unwrap();
    assert!(calc.column_formulas("Orders").is_empty());
    assert_eq!(calc.formulas("Orders").len(), 1);
    assert_eq!(
        calc.formula_text("Orders", &p("items")).as_deref(),
        Some("[]")
    );
    run(&mut calc);
    assert_eq!(value(&calc, "items"), Some(Value::Array(Vec::new())));

    calc.undo().unwrap();
    assert_eq!(calc.formula_text("Orders", &p("items")), None);
    assert_eq!(
        calc.column_formula("Orders", &p("items"), "total"),
        Some(TOTAL)
    );
    assert_eq!(calc.formulas("Orders").len(), 3);
    assert_eq!(number(&calc, "items[1].qty"), 3.0);
    calc.redo().unwrap();
    assert!(calc.column_formulas("Orders").is_empty());
    assert_eq!(calc.formulas("Orders").len(), 1);
}

/// Replacing one item's formulas leaves the column formula of the others as
/// ordinary formulas, as entering anything else in one item does.
#[test]
fn a_formula_on_one_item_can_replace_the_formula_inside_it() {
    let mut calc = book();
    column(&mut calc);
    calc.set_formula_replacing("Orders", &p("items[1]"), "{'qty': 1, 'price': 1.0}")
        .unwrap();
    assert!(calc.column_formulas("Orders").is_empty());
    assert_eq!(calc.formula_text("Orders", &p("items[1].total")), None);
    assert!(calc.formula_text("Orders", &p("items[0].total")).is_some());
    assert!(calc.formula_text("Orders", &p("items[2].total")).is_some());
    calc.undo().unwrap();
    assert_eq!(
        calc.column_formula("Orders", &p("items"), "total"),
        Some(TOTAL)
    );
    assert!(calc.formula_text("Orders", &p("items[1].total")).is_some());
}

/// What a column formula would erase: the values the items hold in the field,
/// not the formulas of a column formula already there.
#[test]
fn the_detail_inside_a_column_is_its_values_but_not_its_own_formulas() {
    let mut calc = book();
    let detail = calc.detail_inside_column("Orders", &p("items"), "total");
    assert_eq!((detail.values, detail.formulas.len()), (1, 0));
    assert!(
        calc.detail_inside_column("Orders", &p("items"), "qty")
            .values
            == 3
    );
    column(&mut calc);
    assert!(
        calc.detail_inside_column("Orders", &p("items"), "total")
            .is_empty()
    );
    let detail = calc.detail_inside("Orders", &p("items"));
    assert_eq!((detail.values, detail.formulas.len()), (6, 3));
}
