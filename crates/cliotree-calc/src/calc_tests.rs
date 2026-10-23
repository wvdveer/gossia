//! Tests of the calculation layer against the stand-in engine. Each test says
//! which part of DESIGN.md it checks.

use std::sync::Arc;

use cliotree_core::{ErrorKind, Path, Value, json, jsonschema};
use proptest::prelude::*;

use crate::fake::FakeEngine;
use crate::{Calc, Computed, EditError, Loc};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "a": {"type": "integer"},
    "b": {"type": "integer"},
    "c": {"type": "integer"},
    "flag": {"type": "boolean"},
    "name": {"type": "string"},
    "ratio": {"type": "number"},
    "note": {"type": "string"},
    "list": {"type": "array", "items": {"type": "integer"}},
    "copy": {"type": "array", "items": {"type": "integer"}},
    "total": {"type": "number"}
  },
  "required": ["a", "b", "c", "flag", "name", "ratio", "list", "copy", "total"]
}"#;

const DATA: &str = r#"{"a": 1, "b": 0, "c": 0, "flag": true, "name": "x", "ratio": 1.5,
  "list": [1, 2, 3, 4, 5, 6], "copy": [], "total": 0}"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

fn sheet(calc: &mut Calc, name: &str, schema: &str, data: &str) {
    let spec = jsonschema::lower_str(schema).unwrap();
    let node = json::parse(data).unwrap().node;
    calc.add_sheet(name, spec, node).unwrap();
}

struct Book {
    calc: Calc,
    engine: FakeEngine,
}

impl Book {
    fn new() -> Book {
        let engine = FakeEngine::new();
        let mut calc = Calc::new(Arc::new(engine.clone()));
        sheet(&mut calc, "S", SCHEMA, DATA);
        Book { calc, engine }
    }

    fn formula(&mut self, path: &str, text: &str) {
        self.calc.set_formula("S", &p(path), text).unwrap();
    }

    fn set(&mut self, path: &str, value: Value) {
        self.calc.set_value("S", &p(path), value).unwrap();
    }

    fn run(&mut self) -> usize {
        self.calc.recalc()
    }

    fn get(&self, path: &str) -> Option<Value> {
        self.calc
            .data("S")
            .unwrap()
            .get(&p(path))
            .map(|n| n.value().clone())
    }

    fn error(&self, path: &str) -> Option<ErrorKind> {
        match self.get(path) {
            Some(Value::Error(e)) => Some(e.kind),
            _ => None,
        }
    }

    fn evaluations(&self) -> usize {
        self.engine.evaluations()
    }
}

// -------------------------------------------------------------- basics

/// Formulas, "Name resolution": a formula reads other nodes, and an edit to
/// one of them recalculates it.
#[test]
fn a_formula_calculates_and_follows_edits() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    assert_eq!(b.calc.pending(), 1);
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(2)));
    b.set("a", Value::Int(10));
    assert_eq!(b.calc.pending(), 1, "the dependent is out of date");
    assert!(b.calc.cell("S", &p("b")).unwrap().pending);
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(11)));
    assert_eq!(b.calc.pending(), 0);
}

/// Formulas, "Dependency graph and recalculation": a chain is calculated in
/// dependency order whatever order the formulas were entered in, and a
/// recalculation runs each formula once.
#[test]
fn a_chain_runs_in_dependency_order_and_each_formula_once() {
    let mut b = Book::new();
    b.formula("c", "S.b + 1");
    b.formula("b", "S.a + 1");
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(3)));

    let before = b.evaluations();
    b.set("a", Value::Int(5));
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(7)));
    assert_eq!(b.evaluations() - before, 2);

    // Recalculate everything: the order is guessed from what was read last
    // time, so `b` goes before `c` although `c` was entered first.
    let before = b.evaluations();
    b.calc.recalc_all();
    b.run();
    assert_eq!(b.evaluations() - before, 2);
}

#[test]
fn unrelated_formulas_are_not_run_again() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.formula("c", "S.list[0] * 2");
    b.run();
    let before = b.evaluations();
    b.set("a", Value::Int(4));
    b.run();
    assert_eq!(b.evaluations() - before, 1);
    assert_eq!(b.get("c"), Some(Value::Int(2)));
}

#[test]
fn a_diamond_is_calculated_from_fresh_values() {
    let mut b = Book::new();
    b.formula("total", "S.b + S.c");
    b.formula("b", "S.a + 1");
    b.formula("c", "S.a * 2");
    b.run();
    assert_eq!(b.get("total"), Some(Value::Int(4)));
    let before = b.evaluations();
    b.set("a", Value::Int(3));
    b.run();
    assert_eq!(b.get("total"), Some(Value::Int(10)));
    assert_eq!(b.evaluations() - before, 3);
}

/// Formulas, "Name resolution": dependencies are recorded as the formula runs,
/// so a branch that is not taken is not a dependency.
#[test]
fn dependencies_are_the_nodes_actually_read() {
    let mut b = Book::new();
    b.set("b", Value::Int(7));
    b.formula("c", "IF(S.flag, S.a, S.b)");
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(1)));

    b.set("b", Value::Int(99));
    assert_eq!(b.calc.pending(), 0, "`b` is not read while the flag is set");

    b.set("flag", Value::Bool(false));
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(99)));

    b.set("a", Value::Int(5));
    assert_eq!(b.calc.pending(), 0, "`a` is no longer read");
    b.set("b", Value::Int(8));
    assert_eq!(b.calc.pending(), 1);
}

/// Formulas, "Name resolution": reading something that is missing is also a
/// dependency, so the formula runs again when it appears.
#[test]
fn a_formula_reading_a_missing_node_reruns_when_it_appears() {
    let mut b = Book::new();
    b.formula("a", "S.note + 'x'");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Ref));
    b.set("note", Value::Str("hi".into()));
    b.run();
    assert_eq!(
        b.error("a"),
        Some(ErrorKind::Type),
        "a string for an integer node"
    );
}

#[test]
fn a_formula_for_a_tab_that_does_not_exist_runs_when_the_tab_is_added() {
    let mut b = Book::new();
    b.formula("a", "Other.n");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Name));
    sheet(
        &mut b.calc,
        "Other",
        r#"{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}"#,
        r#"{"n": 42}"#,
    );
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(42)));
}

#[test]
fn formulas_read_other_tabs() {
    let mut b = Book::new();
    sheet(
        &mut b.calc,
        "Other",
        r#"{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}"#,
        r#"{"n": 42}"#,
    );
    b.formula("a", "Other.n + 1");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(43)));
    b.calc.set_value("Other", &p("n"), Value::Int(1)).unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(2)));
}

#[test]
fn tab_names_are_unique_ignoring_case() {
    let mut b = Book::new();
    let spec = jsonschema::lower_str(SCHEMA).unwrap();
    let node = json::parse(DATA).unwrap().node;
    let err = b.calc.add_sheet("s", spec, node).unwrap_err();
    assert_eq!(err, EditError::DuplicateSheet("s".into()));
}

// -------------------------------------------------------------- cycles

/// Formulas, "Dependency graph and recalculation": a cycle gives `#CIRCULAR!`
/// on every node in it, and dependents see the error.
#[test]
fn a_cycle_gives_circular_on_every_node_in_it() {
    let mut b = Book::new();
    b.formula("a", "S.b");
    b.formula("b", "S.a");
    b.formula("c", "S.a + 1");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Circular));
    assert_eq!(b.error("b"), Some(ErrorKind::Circular));
    assert_eq!(
        b.error("c"),
        Some(ErrorKind::Circular),
        "a dependent sees the error"
    );
    let Some(Value::Error(e)) = b.get("a") else {
        panic!("no error");
    };
    assert!(e.message.contains("circular reference"), "{}", e.message);
    assert!(
        e.message.contains("S.a") && e.message.contains("S.b"),
        "{}",
        e.message
    );
}

#[test]
fn a_formula_that_reads_itself_is_circular() {
    let mut b = Book::new();
    b.formula("a", "S.a + 1");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Circular));
}

#[test]
fn a_formula_that_reads_something_inside_itself_is_circular() {
    // The array depends on its items, and the item is part of the array.
    let mut b = Book::new();
    b.formula("list", "S.copy");
    b.formula("copy", "S.list");
    b.run();
    assert_eq!(b.error("list"), Some(ErrorKind::Circular));
    assert_eq!(b.error("copy"), Some(ErrorKind::Circular));
}

#[test]
fn a_cycle_is_cleared_by_editing_a_formula_on_it() {
    let mut b = Book::new();
    b.formula("a", "S.b");
    b.formula("b", "S.a");
    b.formula("c", "S.a + 1");
    b.run();
    b.set("a", Value::Int(5));
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(5)));
    assert_eq!(b.get("b"), Some(Value::Int(5)));
    assert_eq!(b.get("c"), Some(Value::Int(6)));
}

/// A formula that only waits beside a cycle, and handles the error, is not
/// part of it.
#[test]
fn a_formula_beside_a_cycle_is_not_swept_into_it() {
    let mut b = Book::new();
    b.formula("list[0]", "S.list[1] + S.list[2]");
    b.formula("list[1]", "S.list[0]");
    b.formula("list[2]", "IFERROR(S.list[1], 5)");
    b.run();
    assert_eq!(b.error("list[0]"), Some(ErrorKind::Circular));
    assert_eq!(b.error("list[1]"), Some(ErrorKind::Circular));
    assert_eq!(b.get("list[2]"), Some(Value::Int(5)));
}

/// A read made after an old value was seen is a guess: the formula may read
/// something else once the value is fresh. It must not make a cycle.
#[test]
fn a_read_that_depends_on_a_stale_value_does_not_make_a_false_cycle() {
    let mut b = Book::new();
    // `list[0]` reads `list[1]` only while the flag is set, and `list[1]`
    // always reads `list[0]`.
    b.formula("list[0]", "IF(S.flag, S.list[1], 7)");
    b.formula("list[1]", "S.list[0] + 1");
    b.run();
    assert_eq!(b.error("list[0]"), Some(ErrorKind::Circular));
    b.set("flag", Value::Bool(false));
    b.run();
    assert_eq!(b.get("list[0]"), Some(Value::Int(7)));
    assert_eq!(b.get("list[1]"), Some(Value::Int(8)));

    // And the other way: the flag is itself a formula that is out of date, with
    // an old value that would have caused the cycle.
    b.formula("flag", "S.a == 1");
    b.run();
    assert_eq!(b.error("list[0]"), Some(ErrorKind::Circular));
    b.set("a", Value::Int(2));
    b.run();
    assert_eq!(b.get("flag"), Some(Value::Bool(false)));
    assert_eq!(b.get("list[0]"), Some(Value::Int(7)));
    assert_eq!(b.get("list[1]"), Some(Value::Int(8)));
}

// -------------------------------------------------------------- errors

/// Formulas, "Error values": an error reaches dependents with its kind and its
/// origin, unless handled.
#[test]
fn errors_propagate_with_their_kind_and_origin() {
    let mut b = Book::new();
    b.formula("a", "1 / 0");
    b.formula("b", "S.a + 1");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::DivZero));
    assert_eq!(b.error("b"), Some(ErrorKind::DivZero));
    let Some(Value::Error(e)) = b.get("b") else {
        panic!("no error");
    };
    assert_eq!(e.origin, Some(p("a")), "the origin is the node that failed");
}

#[test]
fn a_formula_that_returns_an_error_value_puts_the_same_error_in_its_node() {
    let mut b = Book::new();
    b.formula("a", "1 / 0");
    b.formula("b", "S.a");
    b.run();
    assert_eq!(b.error("b"), Some(ErrorKind::DivZero));
}

#[test]
fn iferror_and_iserror_handle_errors() {
    let mut b = Book::new();
    b.formula("a", "1 / 0");
    b.formula("b", "IFERROR(S.a, 7)");
    b.formula("flag", "ISERROR(S.a)");
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(7)));
    assert_eq!(b.get("flag"), Some(Value::Bool(true)));
}

#[test]
fn navigating_below_an_errored_node_gives_its_error() {
    let mut b = Book::new();
    b.formula("copy", "1 / 0");
    b.formula("c", "S.copy[0] + 1");
    b.run();
    assert_eq!(b.error("copy"), Some(ErrorKind::DivZero));
    assert_eq!(b.error("c"), Some(ErrorKind::DivZero));
}

#[test]
fn a_failed_formula_keeps_its_error_in_the_overlay() {
    let mut b = Book::new();
    b.formula("a", "1 / 0");
    b.run();
    let cell = b.calc.cell("S", &p("a")).unwrap();
    assert_eq!(cell.formula, Some("1 / 0"));
    assert_eq!(cell.error.unwrap().kind, ErrorKind::DivZero);
    assert!(!cell.pending);
}

#[test]
fn an_error_clears_when_its_cause_is_fixed() {
    let mut b = Book::new();
    b.formula("a", "1 / S.b");
    b.formula("c", "S.a + 1");
    b.run();
    assert_eq!(b.error("c"), Some(ErrorKind::DivZero));
    b.set("b", Value::Int(1));
    b.run();
    // `1 / 1` is a float, which an integer node rejects.
    assert_eq!(b.error("a"), Some(ErrorKind::Type));
    b.formula("a", "2 * S.b");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(2)));
    assert_eq!(b.get("c"), Some(Value::Int(3)));
    assert!(b.calc.cell("S", &p("a")).unwrap().error.is_none());
}

/// Formulas, "Syntax errors": a formula that does not parse is kept, the node
/// holds `#SYNTAX!`, and it is never run.
#[test]
fn a_syntax_error_is_kept_and_not_run() {
    let mut b = Book::new();
    b.formula("a", "1 +");
    let before = b.evaluations();
    assert_eq!(
        b.error("a"),
        Some(ErrorKind::Syntax),
        "set at once, no run needed"
    );
    assert_eq!(b.calc.pending(), 0);
    b.run();
    assert_eq!(b.evaluations(), before);
    let cell = b.calc.cell("S", &p("a")).unwrap();
    assert_eq!(
        cell.formula,
        Some("1 +"),
        "the text stays for the author to correct"
    );
    let Some(Value::Error(e)) = b.get("a") else {
        panic!("no error");
    };
    assert!(e.message.contains("column"), "{}", e.message);

    b.formula("a", "1 + 1");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(2)));
    assert!(b.calc.cell("S", &p("a")).unwrap().error.is_none());
}

#[test]
fn a_syntax_error_reaches_dependents() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.run();
    b.formula("a", "(");
    b.run();
    assert_eq!(b.error("b"), Some(ErrorKind::Syntax));
}

// ------------------------------------------------------ result checking

/// Formulas, "Result checking" and "Scalar types": conversion is strict.
#[test]
fn results_must_match_the_nodes_type_exactly() {
    let mut b = Book::new();
    b.formula("name", "1");
    b.formula("a", "1.5 * 2");
    b.formula("flag", "1");
    b.formula("b", "True");
    b.formula("ratio", "2");
    b.formula("c", "'x'");
    b.run();
    assert_eq!(b.error("name"), Some(ErrorKind::Type));
    assert_eq!(
        b.error("a"),
        Some(ErrorKind::Type),
        "even a whole float is not an integer"
    );
    assert_eq!(b.error("flag"), Some(ErrorKind::Type));
    assert_eq!(
        b.error("b"),
        Some(ErrorKind::Type),
        "a bool is not an integer"
    );
    assert_eq!(
        b.get("ratio"),
        Some(Value::Int(2)),
        "an int is valid for a number node"
    );
    assert_eq!(b.error("c"), Some(ErrorKind::Type));
}

#[test]
fn a_type_error_says_what_was_wrong() {
    let mut b = Book::new();
    b.formula("name", "1");
    b.run();
    let Some(Value::Error(e)) = b.get("name") else {
        panic!("no error");
    };
    assert!(e.message.contains("S.name"), "{}", e.message);
    assert!(e.message.contains("string"), "{}", e.message);
}

/// Formulas, "Absent nodes": `None` leaves an optional node absent and is a
/// `#TYPE!` error on a required one, and a result creates an absent node.
#[test]
fn none_means_no_value() {
    let mut b = Book::new();
    b.set("note", Value::Str("old".into()));
    b.formula("note", "None");
    b.formula("a", "None");
    b.run();
    assert_eq!(b.get("note"), None, "the optional node is left absent");
    assert_eq!(b.error("a"), Some(ErrorKind::Type));

    b.formula("note", "'hi'");
    b.run();
    assert_eq!(b.get("note"), Some(Value::Str("hi".into())));
}

#[test]
fn a_formula_cannot_go_where_the_schema_defines_no_node() {
    let mut b = Book::new();
    let err = b.calc.set_formula("S", &p("nothing"), "1").unwrap_err();
    assert!(matches!(err, EditError::NotInSchema(_)), "{err}");
    let err = b.calc.set_formula("Nope", &p("a"), "1").unwrap_err();
    assert_eq!(err, EditError::NoSheet("Nope".into()));
}

// ------------------------------------------------------ computed subtrees

/// Formulas, "Array and object formulas": the result is materialised as
/// children that the formula owns.
#[test]
fn an_array_formula_materialises_children_that_it_owns() {
    let mut b = Book::new();
    b.formula("copy", "S.list");
    b.run();
    assert_eq!(b.get("copy[1]"), Some(Value::Int(2)));
    assert_eq!(
        b.calc.cell("S", &p("copy[1]")).unwrap().computed_by,
        Some(p("copy"))
    );

    let err = b
        .calc
        .set_value("S", &p("copy[1]"), Value::Int(9))
        .unwrap_err();
    assert!(matches!(err, EditError::InsideComputed { .. }), "{err}");
    let err = b.calc.set_formula("S", &p("copy[0]"), "1").unwrap_err();
    assert!(matches!(err, EditError::InsideComputed { .. }), "{err}");
    let err = b.calc.remove("S", &p("copy[0]")).unwrap_err();
    assert!(matches!(err, EditError::InsideComputed { .. }), "{err}");
    let message = err.to_string();
    assert!(message.contains("S.copy"), "{message}");

    b.set("list[1]", Value::Int(20));
    b.run();
    assert_eq!(b.get("copy[1]"), Some(Value::Int(20)));
}

#[test]
fn an_array_result_shorter_or_longer_replaces_the_children() {
    let mut b = Book::new();
    b.formula("copy", "S.list");
    b.run();
    b.calc.remove("S", &p("list[5]")).unwrap();
    b.calc
        .insert_item("S", &p("list"), 0, Value::Int(0))
        .unwrap();
    b.run();
    let Some(Value::Array(items)) = b.get("copy") else {
        panic!("no array");
    };
    let numbers: Vec<_> = items.iter().map(|n| n.value().clone()).collect();
    assert_eq!(
        numbers,
        [0, 1, 2, 3, 4, 5].map(Value::Int).to_vec(),
        "six items again, after one was removed and one inserted"
    );
}

/// Formulas, "Cells exist only where the schema says": a formula cannot go on a
/// node that already has formulas inside it.
#[test]
fn a_formula_on_a_node_with_formulas_inside_is_refused() {
    let mut b = Book::new();
    b.formula("list[2]", "1");
    let err = b.calc.set_formula("S", &p("list"), "S.copy").unwrap_err();
    assert!(matches!(err, EditError::ContainsFormulas { .. }), "{err}");
    let err = b.calc.set_formula("S", &Path::root(), "1").unwrap_err();
    assert!(matches!(err, EditError::ContainsFormulas { .. }), "{err}");
}

/// Formulas, "Array and object formulas": items may carry formulas when the
/// array itself is not computed.
#[test]
fn items_can_carry_formulas_when_the_array_is_not_computed() {
    let mut b = Book::new();
    b.formula("list[0]", "S.a + 10");
    b.formula("copy", "S.list");
    b.run();
    assert_eq!(b.get("copy[0]"), Some(Value::Int(11)));
    b.set("a", Value::Int(2));
    b.run();
    assert_eq!(b.get("copy[0]"), Some(Value::Int(12)));
}

/// Formulas, "Stored values": a literal replaces a formula.
#[test]
fn a_literal_replaces_a_formula() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.run();
    b.set("b", Value::Int(100));
    assert!(b.calc.cell("S", &p("b")).unwrap().formula.is_none());
    b.set("a", Value::Int(5));
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(100)));
    assert_eq!(b.calc.pending(), 0);
}

#[test]
fn removing_a_formula_keeps_the_value() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.run();
    assert!(b.calc.remove_formula("S", &p("b")).unwrap());
    assert!(!b.calc.remove_formula("S", &p("b")).unwrap());
    b.set("a", Value::Int(5));
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(2)));
}

/// Formulas, "References are literal": a formula belongs to its location, not
/// to the item that is there.
#[test]
fn a_formula_belongs_to_its_location_when_items_move() {
    let mut b = Book::new();
    b.formula("list[1]", "S.a * 100");
    b.run();
    assert_eq!(b.get("list[1]"), Some(Value::Int(100)));
    b.calc
        .insert_item("S", &p("list"), 0, Value::Int(50))
        .unwrap();
    b.run();
    // The formula is still at index 1. The item that held its result moved to
    // index 2 as a plain value, and the item that was at index 2 is now at 3.
    assert_eq!(b.get("list[1]"), Some(Value::Int(100)));
    assert_eq!(
        b.calc.cell("S", &p("list[1]")).unwrap().formula,
        Some("S.a * 100")
    );
    assert_eq!(b.calc.cell("S", &p("list[2]")).unwrap().formula, None);
    assert_eq!(b.get("list[3]"), Some(Value::Int(3)));
}

#[test]
fn a_formula_reading_an_index_follows_the_index_not_the_item() {
    let mut b = Book::new();
    b.formula("c", "S.list[0]");
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(1)));
    b.calc
        .insert_item("S", &p("list"), 0, Value::Int(50))
        .unwrap();
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(50)));
    b.calc.remove("S", &p("list[0]")).unwrap();
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(1)));
}

#[test]
fn an_index_past_the_end_is_a_ref_error() {
    let mut b = Book::new();
    b.formula("c", "S.list[40]");
    b.run();
    assert_eq!(b.error("c"), Some(ErrorKind::Ref));
}

/// Formulas, "Absent nodes": a formula on a node inside an absent parent
/// creates the parent.
#[test]
fn a_result_creates_missing_parents() {
    let mut calc = Calc::new(Arc::new(FakeEngine::new()));
    sheet(
        &mut calc,
        "T",
        r#"{"type":"object","properties":{
            "inner":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}}}"#,
        "{}",
    );
    calc.set_formula("T", &p("inner.n"), "5").unwrap();
    calc.recalc();
    let n = calc.data("T").unwrap().get(&p("inner.n")).unwrap();
    assert_eq!(n.value(), &Value::Int(5));
}

// ---------------------------------------------------- consistency, versions

/// Evaluation and scheduling, "Consistency": an evaluation reads its snapshot,
/// not what the user has edited since.
#[test]
fn an_evaluation_sees_only_its_snapshot() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.run();
    b.set("a", Value::Int(5));
    let evaluation = b.calc.snapshot().evaluate(&b.engine);
    b.set("a", Value::Int(7));
    let outcome = &evaluation.outcomes()[0];
    assert_eq!(outcome.computed, Computed::Value(Value::Int(6)));
}

/// Evaluation and scheduling, "Consistency": a result computed from superseded
/// inputs is discarded, and the formula stays out of date.
#[test]
fn a_result_overtaken_by_an_edit_is_discarded() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.run();
    b.set("a", Value::Int(5));
    let evaluation = b.calc.snapshot().evaluate(&b.engine);
    b.set("a", Value::Int(7));
    let applied = b.calc.apply(evaluation);
    assert!(applied.applied.is_empty());
    assert_eq!(applied.discarded, vec![Loc::new("S", p("b"))]);
    assert_eq!(
        b.get("b"),
        Some(Value::Int(2)),
        "never shows a result from old inputs"
    );
    assert_eq!(b.calc.pending(), 1);
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(8)));
}

#[test]
fn a_result_that_read_a_discarded_result_is_discarded_too() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.formula("c", "S.b + 1");
    b.run();
    b.set("a", Value::Int(5));
    let evaluation = b.calc.snapshot().evaluate(&b.engine);
    b.set("a", Value::Int(7));
    let applied = b.calc.apply(evaluation);
    assert!(applied.applied.is_empty());
    assert_eq!(applied.discarded.len(), 2);
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(9)));
}

#[test]
fn an_edit_to_something_unread_does_not_discard_a_result() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    b.run();
    b.set("a", Value::Int(5));
    let evaluation = b.calc.snapshot().evaluate(&b.engine);
    b.set("name", Value::Str("y".into()));
    let applied = b.calc.apply(evaluation);
    assert_eq!(applied.applied, vec![Loc::new("S", p("b"))]);
    assert_eq!(b.get("b"), Some(Value::Int(6)));
    assert_eq!(b.calc.pending(), 0);
}

#[test]
fn editing_a_formula_during_evaluation_discards_its_old_result() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    let evaluation = b.calc.snapshot().evaluate(&b.engine);
    b.formula("b", "S.a + 100");
    let applied = b.calc.apply(evaluation);
    assert_eq!(applied.discarded.len(), 1);
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(101)));
}

#[test]
fn a_formula_removed_during_evaluation_is_not_resurrected() {
    let mut b = Book::new();
    b.formula("b", "S.a + 1");
    let evaluation = b.calc.snapshot().evaluate(&b.engine);
    b.set("b", Value::Int(3));
    let applied = b.calc.apply(evaluation);
    assert!(applied.applied.is_empty());
    assert_eq!(b.get("b"), Some(Value::Int(3)));
    assert_eq!(b.calc.pending(), 0);
}

#[test]
fn a_snapshot_shares_the_tree_instead_of_copying_it() {
    let b = Book::new();
    let snapshot = b.calc.snapshot();
    let live = b.calc.data("S").unwrap();
    assert!(snapshot.sheets["S"].data.ptr_eq(live));
}

// -------------------------------------------------------------- scale

/// Evaluation and scheduling: the chain of formulas waiting for each other is
/// not held on the call stack.
#[test]
fn a_long_chain_does_not_overflow_the_stack_and_runs_each_formula_once_later() {
    const N: usize = 20_000;
    let schema = r#"{"type":"object","properties":{
        "list":{"type":"array","items":{"type":"integer"}},
        "total":{"type":"integer"}},"required":["list","total"]}"#;
    let data = format!(r#"{{"list":[{}],"total":0}}"#, vec!["0"; N].join(","));
    let engine = FakeEngine::new();
    let mut calc = Calc::new(Arc::new(engine.clone()));
    sheet(&mut calc, "L", schema, &data);
    for i in (1..N).rev() {
        calc.set_formula(
            "L",
            &p(&format!("list[{i}]")),
            &format!("L.list[{}] + 1", i - 1),
        )
        .unwrap();
    }
    calc.set_formula("L", &p("total"), "SUM(L.list)").unwrap();
    calc.recalc();
    let total = calc.data("L").unwrap().get(&p("total")).unwrap();
    assert_eq!(total.value(), &Value::Int((N * (N - 1) / 2) as i64));

    // Now that it knows what reads what, a change runs every formula once.
    let before = engine.evaluations();
    calc.set_value("L", &p("list[0]"), Value::Int(1)).unwrap();
    calc.recalc();
    assert_eq!(engine.evaluations() - before, N);
    let total = calc.data("L").unwrap().get(&p("total")).unwrap();
    assert_eq!(total.value(), &Value::Int((N * (N - 1) / 2 + N) as i64));
}

// ------------------------------------------------------------ properties

#[derive(Debug, Clone)]
enum Op {
    Literal(usize, i64),
    Formula(usize, String),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let index = 0usize..6;
    let formula = prop_oneof![
        (index.clone(), -3i64..4).prop_map(|(j, k)| format!("S.list[{j}] + {k}")),
        (index.clone(), index.clone()).prop_map(|(j, k)| format!("S.list[{j}] * S.list[{k}]")),
        (index.clone(), index.clone()).prop_map(|(j, k)| format!("S.list[{j}] / S.list[{k}]")),
        (index.clone(), index.clone(), index.clone())
            .prop_map(|(c, j, k)| format!("IF(S.list[{c}] > 1, S.list[{j}], S.list[{k}])")),
        (index.clone(), index.clone())
            .prop_map(|(j, k)| format!("IFERROR(S.list[{j}] + 1, S.list[{k}])")),
        (index.clone(), -3i64..4).prop_map(|(j, k)| format!("SUM(S.list) + S.list[{j}] * {k}")),
    ];
    prop_oneof![
        (index.clone(), -3i64..4).prop_map(|(i, n)| Op::Literal(i, n)),
        (index, formula).prop_map(|(i, f)| Op::Formula(i, f)),
    ]
}

fn apply_op(calc: &mut Calc, op: &Op) {
    match op {
        Op::Literal(i, n) => calc
            .set_value("S", &p(&format!("list[{i}]")), Value::Int(*n))
            .unwrap(),
        Op::Formula(i, f) => calc.set_formula("S", &p(&format!("list[{i}]")), f).unwrap(),
    }
}

fn outcome_of(calc: &Calc) -> Vec<String> {
    (0..6)
        .map(|i| {
            let node = calc
                .data("S")
                .unwrap()
                .get(&p(&format!("list[{i}]")))
                .unwrap();
            match node.value() {
                Value::Error(e) => e.kind.code().to_string(),
                other => format!("{other:?}"),
            }
        })
        .collect()
}

proptest! {
    /// Recalculating after every edit gives the same answer as making all the
    /// edits and calculating once: nothing depends on the order of work, and no
    /// stale value is left behind. Cycles and errors are included.
    #[test]
    fn incremental_recalculation_matches_a_full_one(ops in prop::collection::vec(op_strategy(), 1..24)) {
        let mut step_by_step = Book::new();
        let mut at_once = Book::new();
        for op in &ops {
            apply_op(&mut step_by_step.calc, op);
            step_by_step.run();
            apply_op(&mut at_once.calc, op);
        }
        at_once.run();
        prop_assert_eq!(step_by_step.calc.pending(), 0);
        prop_assert_eq!(at_once.calc.pending(), 0);
        prop_assert_eq!(outcome_of(&step_by_step.calc), outcome_of(&at_once.calc));

        // And a full recalculation changes nothing.
        let before = outcome_of(&step_by_step.calc);
        step_by_step.calc.recalc_all();
        step_by_step.run();
        prop_assert_eq!(outcome_of(&step_by_step.calc), before);
    }
}
