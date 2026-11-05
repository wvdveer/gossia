//! Tests of renaming a tab (DESIGN.md, "Renaming a tab").

use std::sync::Arc;

use gossia_core::form::FormDesign;
use gossia_core::{ErrorKind, Path, Value, json, jsonschema};

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
    b.calc.clear_history();
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

// ------------------------------------------- adding, deleting, rebinding

/// `Shop` bound to `ShopSchema`, beside the tabs of [`Book::new`].
fn with_schema(b: &mut Book) {
    b.calc
        .create_json_schema_tab("ShopSchema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    b.calc
        .create_data_tab(
            "Shop",
            "ShopSchema",
            json::parse(r#"{"n": 5}"#).unwrap().node,
            None,
        )
        .unwrap();
}

#[test]
fn new_tabs_go_at_the_end_and_each_is_an_entry() {
    let mut b = Book::new();
    b.calc.set_formula("A", &p("m"), "Shop.n").unwrap();
    b.run();
    assert_eq!(b.error("A", "m"), Some(ErrorKind::Name));
    with_schema(&mut b);
    assert_eq!(b.names(), ["A", "B", "C", "ShopSchema", "Shop"]);
    assert_eq!(b.calc.schema_of("Shop"), Some("ShopSchema"));
    assert_eq!(b.calc.undo_label(), Some("New tab Shop"));
    b.run();
    assert_eq!(b.get("A", "m"), Some(Value::Int(5)), "the formula finds it");

    b.calc.undo().unwrap();
    assert_eq!(b.names(), ["A", "B", "C", "ShopSchema"]);
    assert_eq!(b.calc.take_removed(), ["Shop"]);
    b.run();
    assert_eq!(b.error("A", "m"), Some(ErrorKind::Name));
    b.calc.redo().unwrap();
    assert_eq!(b.names(), ["A", "B", "C", "ShopSchema", "Shop"]);
    b.run();
    assert_eq!(b.get("A", "m"), Some(Value::Int(5)));
}

#[test]
fn an_import_of_two_tabs_is_one_entry() {
    let mut b = Book::new();
    b.calc.begin_entry("Import shop.json");
    with_schema(&mut b);
    b.calc.end_entry();
    assert_eq!(b.calc.undo_label(), Some("Import shop.json"));
    b.calc.undo().unwrap();
    assert_eq!(b.names(), ["A", "B", "C"]);
    b.calc.redo().unwrap();
    assert_eq!(b.calc.schema_users("ShopSchema"), ["Shop"]);
}

#[test]
fn a_new_tab_is_checked_like_a_renamed_one() {
    let mut b = Book::new();
    let schema = || json::parse(SCHEMA).unwrap().node;
    assert!(matches!(
        b.calc.create_json_schema_tab("b", schema()),
        Err(EditError::BadName(_))
    ));
    assert!(matches!(
        b.calc.create_json_schema_tab("SUM", schema()),
        Err(EditError::BadName(_))
    ));
    assert!(
        b.calc.create_data_tab("D", "A", schema(), None).is_err(),
        "A is no schema"
    );
    assert!(!b.calc.can_undo());
}

#[test]
fn a_deleted_tab_comes_back_with_its_formulas_forms_and_place() {
    let mut b = Book::new();
    b.calc.set_formula("B", &p("m"), "A.n + 1").unwrap();
    b.calc.set_formula("C", &p("m"), "B.m * 10").unwrap();
    b.calc.forms_mut().set("B", "Entry", FormDesign::default());
    b.run();
    assert_eq!(b.get("C", "m"), Some(Value::Int(20)));
    assert_eq!(b.calc.mentions("B").formulas, 1);

    b.calc.delete_sheet("B").unwrap();
    assert_eq!(b.names(), ["A", "C"]);
    assert_eq!(b.calc.undo_label(), Some("Delete tab B"));
    assert!(b.calc.forms().names("B").is_empty());
    assert_eq!(b.calc.take_removed(), ["B"]);
    b.run();
    assert_eq!(b.error("C", "m"), Some(ErrorKind::Name));
    // What B's formula read no longer runs it.
    b.calc.set_value("A", &p("n"), Value::Int(4)).unwrap();
    assert_eq!(b.calc.pending(), 0);

    b.calc.undo().unwrap(); // the edit of A
    b.calc.undo().unwrap();
    assert_eq!(b.names(), ["A", "B", "C"]);
    assert_eq!(b.calc.forms().names("B"), ["Entry"]);
    assert_eq!(
        b.calc.formula_text("B", &p("m")).as_deref(),
        Some("A.n + 1")
    );
    b.run();
    assert_eq!(b.get("B", "m"), Some(Value::Int(2)), "calculated again");
    assert_eq!(b.get("C", "m"), Some(Value::Int(20)));
}

#[test]
fn a_schema_in_use_and_the_last_tab_are_not_deleted() {
    let mut b = Book::new();
    with_schema(&mut b);
    let why = b.calc.delete_problem("ShopSchema").unwrap();
    assert_eq!(
        why,
        "Shop uses this schema; delete that tab or give it another schema first"
    );
    assert!(b.calc.delete_sheet("ShopSchema").is_err());
    b.calc.delete_sheet("Shop").unwrap();
    assert_eq!(b.calc.delete_problem("ShopSchema"), None);

    let mut one = Book::new();
    one.calc.delete_sheet("A").unwrap();
    one.calc.delete_sheet("B").unwrap();
    assert_eq!(
        one.calc.delete_problem("C").as_deref(),
        Some("a workbook keeps at least one tab")
    );
}

#[test]
fn a_tab_is_bound_to_another_schema_and_back() {
    let mut b = Book::new();
    with_schema(&mut b);
    const STRICT: &str = r#"{"type": "object", "properties": {"n": {"type": "string"}}}"#;
    b.calc
        .create_json_schema_tab("Strict", json::parse(STRICT).unwrap().node)
        .unwrap();
    assert_eq!(
        b.calc.rebind_problem("Shop", "Shop"),
        Some("Shop is not a schema tab".to_owned())
    );
    b.calc.rebind_sheet("Shop", "Strict").unwrap();
    assert_eq!(b.calc.schema_of("Shop"), Some("Strict"));
    assert_eq!(b.calc.undo_label(), Some("Bind Shop to Strict"));
    let spec = b.calc.spec("Shop").unwrap().clone();
    let data = b.calc.data("Shop").unwrap().clone();
    assert_eq!(
        gossia_core::validate::validate(&spec, &data).len(),
        1,
        "n is not text"
    );
    b.calc.undo().unwrap();
    assert_eq!(b.calc.schema_of("Shop"), Some("ShopSchema"));
    assert_eq!(b.calc.schema_users("Strict"), Vec::<String>::new());
    b.calc.redo().unwrap();
    assert_eq!(b.calc.schema_users("Strict"), ["Shop"]);
}

/// Moving a tab changes only the order: one entry to undo and redo, and
/// nothing recalculated.
#[test]
fn a_tab_is_moved_in_the_order_as_one_entry() {
    let mut book = Book::new();
    let calc = &mut book.calc;
    let order = |calc: &Calc| calc.sheet_names().map(str::to_owned).collect::<Vec<_>>();
    calc.move_sheet("C", 0).unwrap();
    assert_eq!(order(calc), ["C", "A", "B"]);
    assert_eq!(calc.undo_label(), Some("Move tab C"));
    assert!(calc.is_modified());
    // Past the end is the last place; the same place is no edit.
    calc.move_sheet("C", 9).unwrap();
    assert_eq!(order(calc), ["A", "B", "C"]);
    calc.move_sheet("C", 2).unwrap();
    assert_eq!(calc.undo_label(), Some("Move tab C"));
    calc.undo().unwrap();
    assert_eq!(order(calc), ["C", "A", "B"]);
    calc.undo().unwrap();
    assert_eq!(order(calc), ["A", "B", "C"]);
    calc.redo().unwrap();
    assert_eq!(order(calc), ["C", "A", "B"]);
    assert!(calc.move_sheet("D", 0).is_err());
}
