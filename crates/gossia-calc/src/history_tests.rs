//! Tests of undo and redo (DESIGN.md, "Undo and redo").

use std::sync::Arc;

use gossia_core::{Path, Value, json, jsonschema};
use proptest::prelude::*;

use crate::Calc;
use crate::fake::FakeEngine;

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "a": {"type": "integer"},
    "b": {"type": "integer"},
    "note": {"type": "string"},
    "list": {"type": "array", "items": {"type": "integer"}},
    "by": {"type": "object", "additionalProperties": {"type": "integer"}}
  },
  "required": ["a", "b", "list", "by"]
}"#;

const DATA: &str = r#"{"a": 1, "b": 2, "list": [10, 20, 30, 40], "by": {"x": 1, "y": 2, "z": 3}}"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

struct Book {
    calc: Calc,
    engine: FakeEngine,
}

impl Book {
    fn new() -> Book {
        let engine = FakeEngine::new();
        let mut calc = Calc::new(Arc::new(engine.clone()));
        calc.add_sheet(
            "S",
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(DATA).unwrap().node,
        )
        .unwrap();
        Book { calc, engine }
    }

    fn run(&mut self) {
        self.calc.recalc(&self.engine);
    }

    fn get(&self, path: &str) -> Option<Value> {
        self.calc
            .data("S")
            .unwrap()
            .get(&p(path))
            .map(|n| n.value().clone())
    }

    fn json(&self) -> String {
        json::to_string(self.calc.data("S").unwrap(), false).unwrap()
    }

    fn id(&self, path: &str) -> gossia_core::NodeId {
        self.calc.data("S").unwrap().get(&p(path)).unwrap().id()
    }

    fn undo(&mut self) -> Option<String> {
        let label = self.calc.undo().unwrap();
        self.run();
        label
    }

    fn redo(&mut self) -> Option<String> {
        let label = self.calc.redo().unwrap();
        self.run();
        label
    }
}

#[test]
fn a_literal_can_be_undone_and_redone_and_keeps_its_node_identity() {
    let mut b = Book::new();
    let id = b.id("a");
    b.calc.set_value("S", &p("a"), Value::Int(5)).unwrap();
    assert_eq!(b.calc.undo_label(), Some("Set S.a"));
    assert_eq!(b.undo().as_deref(), Some("Set S.a"));
    assert_eq!(b.get("a"), Some(Value::Int(1)));
    assert_eq!(b.id("a"), id);
    assert_eq!(b.calc.redo_label(), Some("Set S.a"));
    b.redo();
    assert_eq!(b.get("a"), Some(Value::Int(5)));
    assert_eq!(b.id("a"), id);
    assert_eq!(b.undo().as_deref(), Some("Set S.a"));
    assert_eq!(b.undo(), None, "nothing more to undo");
    assert_eq!(b.redo().as_deref(), Some("Set S.a"));
    assert_eq!(b.redo(), None);
}

/// A new edit after an undo ends the redo history.
#[test]
fn a_new_edit_after_an_undo_clears_redo() {
    let mut b = Book::new();
    b.calc.set_value("S", &p("a"), Value::Int(5)).unwrap();
    b.calc.set_value("S", &p("b"), Value::Int(6)).unwrap();
    b.undo();
    assert!(b.calc.can_redo());
    b.calc.set_value("S", &p("a"), Value::Int(7)).unwrap();
    assert!(!b.calc.can_redo());
    assert_eq!(b.calc.history_len(), (2, 0));
}

#[test]
fn removing_and_inserting_restore_position_and_identity() {
    let mut b = Book::new();
    let before = b.json();
    let id2 = b.id("list[2]");
    let id_y = b.id("by.y");
    b.calc.remove("S", &p("list[2]")).unwrap();
    b.calc.remove("S", &p("by.y")).unwrap();
    b.calc
        .insert_item("S", &p("list"), 1, Value::Int(99))
        .unwrap();
    assert_eq!(b.get("list[1]"), Some(Value::Int(99)));
    b.undo();
    b.undo();
    b.undo();
    assert_eq!(b.json(), before, "same order of items and of fields");
    assert_eq!(b.id("list[2]"), id2);
    assert_eq!(b.id("by.y"), id_y);
    b.redo();
    b.redo();
    b.redo();
    assert_eq!(b.get("list[1]"), Some(Value::Int(99)));
    assert_eq!(b.get("list[3]"), Some(Value::Int(40)));
    assert_eq!(b.get("by.y"), None);
}

/// An entry is one gesture: several edits can be one entry.
#[test]
fn a_group_of_edits_is_one_entry() {
    let mut b = Book::new();
    b.calc.begin_entry("Paste");
    b.calc.set_value("S", &p("a"), Value::Int(5)).unwrap();
    b.calc.begin_entry("inner, ignored");
    b.calc.set_value("S", &p("b"), Value::Int(6)).unwrap();
    b.calc.remove("S", &p("list[0]")).unwrap();
    b.calc.end_entry();
    assert!(b.calc.undo().is_err(), "not while an edit is in progress");
    b.calc.end_entry();
    assert_eq!(b.calc.history_len(), (1, 0));
    assert_eq!(b.calc.undo_label(), Some("Paste"));
    let before = r#"{"a":1,"b":2,"list":[10,20,30,40],"by":{"x":1,"y":2,"z":3}}"#;
    b.undo();
    assert_eq!(b.json(), before);
    b.redo();
    assert_eq!(b.get("a"), Some(Value::Int(5)));
    assert_eq!(b.get("list[0]"), Some(Value::Int(20)));
}

#[test]
fn an_empty_group_leaves_no_entry() {
    let mut b = Book::new();
    b.calc.begin_entry("Nothing");
    b.calc.end_entry();
    assert_eq!(b.calc.history_len(), (0, 0));
}

/// Rapid consecutive edits to the same text cell are merged.
#[test]
fn typing_in_one_cell_is_one_entry_until_the_run_is_sealed() {
    let mut b = Book::new();
    for n in [2, 3, 4] {
        b.calc
            .set_value_merging("S", &p("a"), Value::Int(n))
            .unwrap();
    }
    assert_eq!(b.calc.history_len(), (1, 0));
    b.calc
        .set_value_merging("S", &p("b"), Value::Int(9))
        .unwrap();
    assert_eq!(b.calc.history_len(), (2, 0), "another cell");
    b.calc
        .set_value_merging("S", &p("b"), Value::Int(10))
        .unwrap();
    b.calc.seal_history();
    b.calc
        .set_value_merging("S", &p("b"), Value::Int(11))
        .unwrap();
    assert_eq!(b.calc.history_len(), (3, 0), "sealed");
    b.undo();
    assert_eq!(b.get("b"), Some(Value::Int(10)));
    b.undo();
    assert_eq!(b.get("b"), Some(Value::Int(2)), "back to before the typing");
    b.undo();
    assert_eq!(b.get("a"), Some(Value::Int(1)));
}

#[test]
fn an_ordinary_edit_between_does_not_merge() {
    let mut b = Book::new();
    b.calc
        .set_value_merging("S", &p("a"), Value::Int(2))
        .unwrap();
    b.calc.set_value("S", &p("b"), Value::Int(7)).unwrap();
    b.calc
        .set_value_merging("S", &p("a"), Value::Int(3))
        .unwrap();
    assert_eq!(b.calc.history_len(), (3, 0));
}

/// Computed results are not logged: after an undo the formulas that read the
/// change are calculated again.
#[test]
fn undo_recalculates_what_depends_on_the_change() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("b"), "S.a + 1").unwrap();
    b.run();
    b.calc.set_value("S", &p("a"), Value::Int(10)).unwrap();
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(11)));
    b.calc.undo().unwrap();
    assert!(b.calc.pending() > 0);
    b.run();
    assert_eq!(b.get("b"), Some(Value::Int(2)));
    b.redo();
    assert_eq!(b.get("b"), Some(Value::Int(11)));
}

/// A formula overwrote the literal that was there; undoing it brings the literal
/// back.
#[test]
fn undoing_a_formula_restores_the_literal_it_replaced() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("a"), "S.b * 100").unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(200)));
    assert_eq!(
        b.calc.formula_text("S", &p("a")).as_deref(),
        Some("S.b * 100")
    );
    b.undo();
    assert_eq!(b.get("a"), Some(Value::Int(1)), "the literal is back");
    assert_eq!(b.calc.formula_text("S", &p("a")), None);
    b.redo();
    assert_eq!(b.get("a"), Some(Value::Int(200)));
    assert_eq!(
        b.calc.formula_text("S", &p("a")).as_deref(),
        Some("S.b * 100")
    );
}

#[test]
fn undoing_a_literal_typed_over_a_formula_brings_the_formula_back() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("a"), "S.b + 1").unwrap();
    b.run();
    b.calc.set_value("S", &p("a"), Value::Int(50)).unwrap();
    assert_eq!(b.calc.formula_text("S", &p("a")), None);
    b.undo();
    assert_eq!(
        b.calc.formula_text("S", &p("a")).as_deref(),
        Some("S.b + 1")
    );
    assert_eq!(b.get("a"), Some(Value::Int(3)));
    b.redo();
    assert_eq!(b.get("a"), Some(Value::Int(50)));
}

#[test]
fn replacing_and_clearing_formulas_can_be_undone() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("a"), "1 + 1").unwrap();
    b.calc.set_formula("S", &p("a"), "2 + 2").unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(4)));
    assert!(b.calc.remove_formula("S", &p("a")).unwrap());
    b.undo();
    assert_eq!(b.calc.formula_text("S", &p("a")).as_deref(), Some("2 + 2"));
    b.undo();
    b.run();
    assert_eq!(b.calc.formula_text("S", &p("a")).as_deref(), Some("1 + 1"));
    assert_eq!(b.get("a"), Some(Value::Int(2)));
    b.undo();
    assert_eq!(b.get("a"), Some(Value::Int(1)));
}

/// A formula that left an optional node absent, or created one, is undone too.
#[test]
fn undoing_a_formula_on_an_absent_optional_node() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("note"), "'hello'").unwrap();
    b.run();
    assert_eq!(b.get("note"), Some(Value::Str("hello".into())));
    b.undo();
    assert_eq!(b.get("note"), None, "it did not exist before");
    b.redo();
    assert_eq!(b.get("note"), Some(Value::Str("hello".into())));
}

/// The log tracks the saved state, so undoing back to the saved point clears
/// the modified mark.
#[test]
fn the_modified_mark_follows_the_saved_state() {
    let mut b = Book::new();
    assert!(
        !b.calc.is_modified(),
        "a new workbook is as saved as it gets"
    );
    b.calc.set_value("S", &p("a"), Value::Int(5)).unwrap();
    assert!(b.calc.is_modified());
    b.calc.mark_saved();
    assert!(!b.calc.is_modified());
    b.calc.set_value("S", &p("b"), Value::Int(6)).unwrap();
    assert!(b.calc.is_modified());
    b.undo();
    assert!(!b.calc.is_modified(), "back at the saved point");
    b.undo();
    assert!(b.calc.is_modified());
    b.redo();
    assert!(!b.calc.is_modified());
}

#[test]
fn an_edit_after_undoing_past_the_saved_state_means_it_cannot_be_reached_again() {
    let mut b = Book::new();
    b.calc.set_value("S", &p("a"), Value::Int(5)).unwrap();
    b.calc.mark_saved();
    b.undo();
    b.calc.set_value("S", &p("a"), Value::Int(7)).unwrap();
    // The saved state was in the redo history that this edit discarded.
    assert!(b.calc.is_modified());
    b.undo();
    assert!(b.calc.is_modified());
}

/// The history is bounded; the oldest entries are dropped first.
#[test]
fn the_history_is_bounded_and_drops_the_oldest() {
    let mut b = Book::new();
    b.calc.set_history_limits(3, usize::MAX);
    for n in 0..6 {
        b.calc.set_value("S", &p("a"), Value::Int(100 + n)).unwrap();
    }
    assert_eq!(b.calc.history_len(), (3, 0));
    b.undo();
    b.undo();
    b.undo();
    assert_eq!(b.undo(), None);
    assert_eq!(
        b.get("a"),
        Some(Value::Int(102)),
        "the three oldest are gone"
    );
}

#[test]
fn the_weight_of_what_is_held_bounds_the_history() {
    let mut b = Book::new();
    // A value edit holds the node before and after: about 2 each way.
    b.calc.set_history_limits(1000, 3);
    b.calc.set_value("S", &p("a"), Value::Int(5)).unwrap();
    assert_eq!(b.calc.history_len().0, 1);
    b.calc.set_value("S", &p("b"), Value::Int(6)).unwrap();
    assert_eq!(
        b.calc.history_len().0,
        1,
        "the older one was dropped to fit"
    );
    assert_eq!(b.calc.undo_label(), Some("Set S.b"));
}

#[test]
fn clearing_the_history_forgets_everything() {
    let mut b = Book::new();
    b.calc.set_value("S", &p("a"), Value::Int(5)).unwrap();
    b.calc.clear_history();
    assert_eq!(b.calc.history_len(), (0, 0));
    assert!(!b.calc.can_undo());
    assert!(
        b.calc.is_modified(),
        "the saved state can no longer be reached"
    );
}

#[test]
fn refused_edits_leave_no_entry() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("nothing"), "1").unwrap_err();
    b.calc
        .set_value("S", &p("nothing[3]"), Value::Int(1))
        .unwrap_err();
    b.calc.remove("S", &p("list[9]")).unwrap_err();
    b.calc
        .insert_item("S", &p("list"), 9, Value::Int(1))
        .unwrap_err();
    assert_eq!(b.calc.history_len(), (0, 0));
}

// ------------------------------------------------------------ properties

#[derive(Debug, Clone)]
enum Edit {
    Set(usize, i64),
    Insert(usize, i64),
    Remove(usize),
    Formula(usize, usize),
    ClearFormula(usize),
    SetField(String, i64),
    RemoveField(String),
    Recalc,
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    let field = prop_oneof![
        Just("x".to_owned()),
        Just("y".to_owned()),
        Just("w".to_owned())
    ];
    prop_oneof![
        (0usize..6, -5i64..5).prop_map(|(i, n)| Edit::Set(i, n)),
        (0usize..6, -5i64..5).prop_map(|(i, n)| Edit::Insert(i, n)),
        (0usize..6).prop_map(Edit::Remove),
        (0usize..6, 0usize..6).prop_map(|(i, j)| Edit::Formula(i, j)),
        (0usize..6).prop_map(Edit::ClearFormula),
        (field.clone(), -5i64..5).prop_map(|(f, n)| Edit::SetField(f, n)),
        field.prop_map(Edit::RemoveField),
        Just(Edit::Recalc),
    ]
}

fn apply(book: &mut Book, edit: &Edit) {
    let list = p("list");
    // Edits that do not fit the data are refused and leave no trace.
    let _ = match edit {
        Edit::Set(i, n) => book
            .calc
            .set_value("S", &list.index(*i), Value::Int(*n))
            .map_err(drop),
        Edit::Insert(i, n) => book
            .calc
            .insert_item("S", &list, *i, Value::Int(*n))
            .map_err(drop),
        Edit::Remove(i) => book.calc.remove("S", &list.index(*i)).map_err(drop),
        Edit::Formula(i, j) => book
            .calc
            .set_formula("S", &list.index(*i), &format!("S.list[{j}] + 1"))
            .map_err(drop),
        Edit::ClearFormula(i) => book
            .calc
            .remove_formula("S", &list.index(*i))
            .map(drop)
            .map_err(drop),
        Edit::SetField(f, n) => book
            .calc
            .set_value("S", &p("by").key(f.as_str()), Value::Int(*n))
            .map_err(drop),
        Edit::RemoveField(f) => book
            .calc
            .remove("S", &p("by").key(f.as_str()))
            .map_err(drop),
        Edit::Recalc => {
            book.run();
            Ok(())
        }
    };
}

fn state(book: &Book) -> (String, Vec<(String, String)>) {
    let formulas = book
        .calc
        .formulas("S")
        .into_iter()
        .map(|(path, text)| (path.to_string(), text.to_owned()))
        .collect();
    (book.json(), formulas)
}

proptest! {
    /// Undoing every edit restores the original, with formulas and
    /// recalculation in between.
    #[test]
    fn undoing_everything_restores_the_original(
        edits in prop::collection::vec(edit_strategy(), 1..30)
    ) {
        let mut book = Book::new();
        let original = state(&book);
        for edit in &edits {
            apply(&mut book, edit);
        }
        book.run();
        while book.calc.can_undo() {
            book.calc.undo().unwrap();
            book.run();
        }
        prop_assert_eq!(state(&book), original);
    }

    /// Redoing every edit gives the state the edits made. (Without a
    /// recalculation in the middle: a value that a formula computed and that
    /// is left behind when the formula is cleared depends on when the formula
    /// was last calculated, and redoing does not recalculate at those points.)
    #[test]
    fn redoing_everything_gives_the_final_state(
        edits in prop::collection::vec(edit_strategy(), 1..30)
    ) {
        let mut book = Book::new();
        for edit in edits.iter().filter(|e| !matches!(e, Edit::Recalc)) {
            apply(&mut book, edit);
        }
        book.run();
        let final_state = state(&book);
        while book.calc.can_undo() {
            book.calc.undo().unwrap();
            book.run();
        }
        while book.calc.can_redo() {
            book.calc.redo().unwrap();
        }
        book.run();
        prop_assert_eq!(state(&book), final_state);
    }
}

// ---------------------------------------------------------------- reorder

#[test]
fn reordering_is_one_entry_that_keeps_the_items_and_can_be_undone() {
    let mut b = Book::new();
    let (id0, id3) = (b.id("list[0]"), b.id("list[3]"));
    let before = b.json();
    b.calc
        .reorder("S", &p("list"), &[3, 2, 1, 0], "Reverse")
        .unwrap();
    assert_eq!(b.calc.undo_label(), Some("Reverse S.list"));
    assert_eq!(b.get("list[0]"), Some(Value::Int(40)));
    assert_eq!(b.id("list[0]"), id3, "the items keep their identity");
    assert_eq!(b.id("list[3]"), id0);
    assert_eq!(b.calc.history_len(), (1, 0));
    b.undo();
    assert_eq!(b.json(), before);
    b.redo();
    assert_eq!(b.get("list[1]"), Some(Value::Int(30)));
}

#[test]
fn an_order_that_is_not_a_permutation_is_refused_and_the_identity_is_no_edit() {
    let mut b = Book::new();
    for bad in [&[0, 1, 2][..], &[0, 1, 2, 2], &[0, 1, 2, 9]] {
        b.calc.reorder("S", &p("list"), bad, "Sort").unwrap_err();
    }
    b.calc.reorder("S", &p("a"), &[0], "Sort").unwrap_err();
    b.calc
        .reorder("S", &p("list"), &[0, 1, 2, 3], "Sort")
        .unwrap();
    assert_eq!(b.calc.history_len(), (0, 0));
}

/// A formula stays at its place and is calculated against the item now there;
/// undoing puts every item back, including the one the formula overwrote.
#[test]
fn a_formula_inside_an_item_stays_put_when_the_array_is_reordered() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("list[1]"), "S.a * 100").unwrap();
    b.run();
    assert_eq!(b.get("list[1]"), Some(Value::Int(100)));
    let before = b.json();
    b.calc
        .reorder("S", &p("list"), &[3, 2, 1, 0], "Reverse")
        .unwrap();
    b.run();
    assert_eq!(b.get("list[1]"), Some(Value::Int(100)), "recalculated");
    assert_eq!(
        b.calc.cell("S", &p("list[1]")).unwrap().formula,
        Some("S.a * 100")
    );
    assert_eq!(b.get("list[2]"), Some(Value::Int(100)), "the moved result");
    b.undo();
    assert_eq!(b.json(), before);
}

#[test]
fn a_computed_array_cannot_be_reordered() {
    let mut b = Book::new();
    b.calc.set_formula("S", &p("list"), "[3, 2, 1]").unwrap();
    b.run();
    let e = b
        .calc
        .reorder("S", &p("list"), &[2, 1, 0], "Sort")
        .unwrap_err();
    assert!(e.to_string().contains("computed"), "{e}");
}
