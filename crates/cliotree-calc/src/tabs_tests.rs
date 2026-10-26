//! Tests of renaming a tab (DESIGN.md, "Renaming a tab").

use std::sync::Arc;

use cliotree_core::form::FormDesign;
use cliotree_core::{ErrorKind, Path, Value, json, jsonschema};

use crate::fake::FakeEngine;
use crate::{Calc, EditError, Mentions};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {"n": {"type": "integer"}, "m": {"type": "integer"}},
  "required": ["n"]
}"#;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

struct Book {
    calc: Calc,
    engine: FakeEngine,
}

impl Book {
    /// Tabs `A`, `B` and `C`, each holding `{"n": 1}`.
    fn new() -> Book {
        let engine = FakeEngine::new();
        let mut calc = Calc::new(Arc::new(engine.clone()));
        for name in ["A", "B", "C"] {
            calc.add_sheet(
                name,
                jsonschema::lower_str(SCHEMA).unwrap(),
                json::parse(r#"{"n": 1}"#).unwrap().node,
            )
            .unwrap();
        }
        calc.clear_history();
        Book { calc, engine }
    }

    fn run(&mut self) {
        self.calc.recalc(&self.engine);
    }

    fn get(&self, sheet: &str, path: &str) -> Option<Value> {
        self.calc
            .data(sheet)?
            .get(&p(path))
            .map(|n| n.value().clone())
    }

    fn error(&self, sheet: &str, path: &str) -> Option<ErrorKind> {
        match self.get(sheet, path) {
            Some(Value::Error(e)) => Some(e.kind),
            _ => None,
        }
    }

    fn names(&self) -> Vec<&str> {
        self.calc.sheet_names().collect()
    }
}

#[test]
fn a_renamed_tab_keeps_its_place_data_and_formulas() {
    let mut b = Book::new();
    b.calc.set_formula("B", &p("m"), "A.n + 1").unwrap();
    b.run();
    b.calc.rename_sheet("B", "Middle").unwrap();
    assert_eq!(b.names(), ["A", "Middle", "C"]);
    assert_eq!(
        b.calc.formula_text("Middle", &p("m")).as_deref(),
        Some("A.n + 1")
    );
    assert!(b.calc.data("B").is_none());
    b.run();
    assert_eq!(b.get("Middle", "m"), Some(Value::Int(2)));
    // What the formula reads is still known under the tab's new name.
    b.calc.set_value("A", &p("n"), Value::Int(10)).unwrap();
    assert_eq!(b.calc.pending(), 1);
    b.run();
    assert_eq!(b.get("Middle", "m"), Some(Value::Int(11)));
}

#[test]
fn formulas_are_not_rewritten_and_the_old_name_is_missing() {
    let mut b = Book::new();
    b.calc.set_formula("A", &p("m"), "B.n").unwrap();
    b.calc.set_formula("C", &p("m"), "Middle.n").unwrap();
    b.run();
    assert_eq!(b.get("A", "m"), Some(Value::Int(1)));
    assert_eq!(b.error("C", "m"), Some(ErrorKind::Name));
    b.calc.rename_sheet("B", "Middle").unwrap();
    assert_eq!(b.calc.formula_text("A", &p("m")).as_deref(), Some("B.n"));
    b.run();
    assert_eq!(b.error("A", "m"), Some(ErrorKind::Name), "B is gone");
    assert_eq!(b.get("C", "m"), Some(Value::Int(1)), "Middle is found");
}

#[test]
fn a_rename_is_one_entry_to_undo_and_redo() {
    let mut b = Book::new();
    b.calc.set_formula("A", &p("m"), "B.n").unwrap();
    b.run();
    b.calc.rename_sheet("B", "Middle").unwrap();
    assert_eq!(b.calc.undo_label(), Some("Rename tab B to Middle"));
    assert_eq!(
        b.calc.take_renames(),
        [("B".to_owned(), "Middle".to_owned())]
    );
    b.run();
    assert_eq!(b.error("A", "m"), Some(ErrorKind::Name));

    b.calc.undo().unwrap();
    assert_eq!(b.names(), ["A", "B", "C"]);
    assert_eq!(
        b.calc.take_renames(),
        [("Middle".to_owned(), "B".to_owned())]
    );
    b.run();
    assert_eq!(b.get("A", "m"), Some(Value::Int(1)));

    b.calc.redo().unwrap();
    assert_eq!(b.names(), ["A", "Middle", "C"]);
    assert!(b.calc.take_renames().len() == 1 && b.calc.take_renames().is_empty());
}

#[test]
fn edits_made_before_a_rename_are_undone_after_it() {
    let mut b = Book::new();
    b.calc.set_value("B", &p("n"), Value::Int(5)).unwrap();
    b.calc.rename_sheet("B", "Middle").unwrap();
    b.calc.set_value("Middle", &p("n"), Value::Int(6)).unwrap();
    for _ in 0..3 {
        b.calc.undo().unwrap();
    }
    assert_eq!(b.get("B", "n"), Some(Value::Int(1)));
    for _ in 0..3 {
        b.calc.redo().unwrap();
    }
    assert_eq!(b.get("Middle", "n"), Some(Value::Int(6)));
}

#[test]
fn names_that_break_the_rules_or_clash_are_refused() {
    let mut b = Book::new();
    b.calc
        .add_module("Tax", "def vat(x):\n    return x\n")
        .unwrap();
    for (name, word) in [
        ("", "needs a name"),
        ("2nd", "not a valid"),
        ("SUM", "built-in function"),
        ("c", "exists already"),
        ("tax", "module"),
    ] {
        match b.calc.rename_sheet("B", name) {
            Err(EditError::BadName(why)) => assert!(why.contains(word), "{name}: {why}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    assert_eq!(b.names(), ["A", "B", "C"]);
    assert!(!b.calc.can_undo());
    assert!(matches!(
        b.calc.rename_sheet("Nope", "X"),
        Err(EditError::NoSheet(_))
    ));
}

#[test]
fn a_tab_may_change_the_case_of_its_name_and_the_same_name_is_no_edit() {
    let mut b = Book::new();
    b.calc.rename_sheet("B", "B").unwrap();
    assert!(!b.calc.can_undo());
    assert_eq!(b.calc.tab_name_problem("b", Some("B")), None);
    b.calc.rename_sheet("B", "b").unwrap();
    assert_eq!(b.names(), ["A", "b", "C"]);
    assert!(
        b.calc.tab_name_problem("b", None).is_some(),
        "for a new tab"
    );
}

#[test]
fn bindings_follow_a_renamed_schema_tab() {
    let engine = FakeEngine::new();
    let mut calc = Calc::new(Arc::new(engine.clone()));
    calc.add_schema_sheet("Order_schema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.add_sheet(
        "Order",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"n": 1}"#).unwrap().node,
    )
    .unwrap();
    calc.bind_schema("Order", "Order_schema").unwrap();
    calc.rename_sheet("Order_schema", "OrderTypes").unwrap();
    assert_eq!(calc.schema_of("Order"), Some("OrderTypes"));
    assert_eq!(calc.schema_users("OrderTypes"), ["Order"]);
    calc.undo().unwrap();
    assert_eq!(calc.schema_of("Order"), Some("Order_schema"));
}

#[test]
fn forms_follow_the_tab() {
    let mut b = Book::new();
    b.calc.forms_mut().set("B", "Entry", FormDesign::default());
    b.calc.rename_sheet("B", "Middle").unwrap();
    assert_eq!(b.calc.forms().names("Middle"), ["Entry"]);
    assert!(b.calc.forms().names("B").is_empty());
    b.calc.undo().unwrap();
    assert_eq!(b.calc.forms().names("B"), ["Entry"]);
}

#[test]
fn mentions_count_formulas_and_modules_that_use_a_name() {
    let mut b = Book::new();
    b.calc.set_formula("A", &p("m"), "B.n + 1").unwrap();
    b.calc.set_formula("C", &p("m"), "A.n").unwrap();
    b.calc
        .add_module("Tax", "def vat():\n    return B.n\n")
        .unwrap();
    assert_eq!(
        b.calc.mentions("B"),
        Mentions {
            formulas: 1,
            modules: 1
        }
    );
    assert!(b.calc.mentions("Nothing").is_empty());
}
