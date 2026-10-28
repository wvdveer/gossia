//! Tests of the expressions on form controls (DESIGN.md, "Forms",
//! "Expressions on controls"), with the stand-in engine.

use std::sync::Arc;

use cliotree_core::form::{Control, ControlKind, FormDesign};
use cliotree_core::{Path, Value, json, jsonschema};

use crate::fake::FakeEngine;
use crate::{Calc, ExprKind};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "qty": {"type": "integer"},
    "price": {"type": "number"},
    "total": {"type": "number"},
    "note": {"type": "string"}
  }
}"#;

const DATA: &str = r#"{"qty": 2, "price": 1.5, "total": 0, "note": "x"}"#;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn field(path: &str) -> Control {
    Control::bound(ControlKind::Field, &p(path))
}

struct Book {
    calc: Calc,
    engine: FakeEngine,
}

impl Book {
    fn new(controls: Vec<Control>) -> Book {
        let engine = FakeEngine::new();
        let mut calc = Calc::new(Arc::new(engine.clone()));
        calc.add_sheet(
            "Order",
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(DATA).unwrap().node,
        )
        .unwrap();
        calc.forms_mut().set(
            "Order",
            "Entry",
            FormDesign {
                controls,
                ..FormDesign::default()
            },
        );
        Book { calc, engine }
    }

    fn run(&mut self) {
        self.calc.recalc(&self.engine);
    }

    fn result(&self, at: &[usize], kind: ExprKind) -> Option<Result<Value, String>> {
        self.calc
            .control_expression("Order", "Entry", at, kind)
            .and_then(|s| s.result)
    }

    fn pending(&self, at: &[usize], kind: ExprKind) -> bool {
        self.calc
            .control_expression("Order", "Entry", at, kind)
            .is_some_and(|s| s.pending)
    }
}

#[test]
fn an_expression_runs_and_runs_again_when_what_it_read_changes() {
    let mut qty = field("qty");
    qty.visible_when = Some("Order.qty > 1".into());
    qty.enabled_when = Some("Order.note == \"x\"".into());
    let mut b = Book::new(vec![qty, field("note")]);
    b.calc.sync_expressions();
    assert_eq!(b.calc.pending_expressions(), 2);
    assert!(b.pending(&[0], ExprKind::Visible));
    assert_eq!(b.result(&[0], ExprKind::Visible), None, "not run yet");
    // The second control has no expressions.
    assert!(
        b.calc
            .control_expression("Order", "Entry", &[1], ExprKind::Visible)
            .is_none()
    );

    b.run();
    assert_eq!(b.calc.pending_expressions(), 0);
    assert_eq!(
        b.result(&[0], ExprKind::Visible),
        Some(Ok(Value::Bool(true)))
    );
    assert_eq!(
        b.result(&[0], ExprKind::Enabled),
        Some(Ok(Value::Bool(true)))
    );

    // An edit of what one read makes that one out of date, keeping its last
    // result; the other is left alone.
    let runs = b.engine.evaluations();
    b.calc.set_value("Order", &p("qty"), Value::Int(1)).unwrap();
    assert!(b.pending(&[0], ExprKind::Visible));
    assert!(!b.pending(&[0], ExprKind::Enabled));
    assert_eq!(
        b.result(&[0], ExprKind::Visible),
        Some(Ok(Value::Bool(true)))
    );
    b.run();
    assert_eq!(b.engine.evaluations(), runs + 1);
    assert_eq!(
        b.result(&[0], ExprKind::Visible),
        Some(Ok(Value::Bool(false)))
    );
    // Nothing of this is an edit of the workbook.
    assert_eq!(b.calc.history_len(), (1, 0));
}

#[test]
fn an_expression_reads_the_results_of_the_formulas_it_follows() {
    let mut total = field("total");
    total.fill_colour = Some("IF(Order.total > 10, \"red\", None)".into());
    let mut b = Book::new(vec![total]);
    b.calc
        .set_formula("Order", &p("total"), "Order.qty * Order.price")
        .unwrap();
    b.run();
    assert_eq!(b.result(&[0], ExprKind::FillColour), Some(Ok(Value::Null)));
    b.calc
        .set_value("Order", &p("qty"), Value::Int(10))
        .unwrap();
    b.run();
    assert_eq!(
        b.result(&[0], ExprKind::FillColour),
        Some(Ok(Value::Str("red".into())))
    );
}

#[test]
fn an_error_and_a_syntax_error_are_results_too() {
    let mut a = field("qty");
    a.enabled_when = Some("Order.nothing > 1".into());
    let mut c = field("note");
    c.visible_when = Some("Order.qty >".into());
    let mut b = Book::new(vec![a, c]);
    b.calc.sync_expressions();
    // One that does not parse is not run.
    assert_eq!(b.calc.pending_expressions(), 1);
    let why = b.result(&[1], ExprKind::Visible).unwrap().unwrap_err();
    assert!(why.starts_with("syntax error"), "{why}");
    b.run();
    let why = b.result(&[0], ExprKind::Enabled).unwrap().unwrap_err();
    assert!(why.contains("nothing"), "{why}");
}

#[test]
fn a_changed_design_runs_what_changed_and_forgets_what_went() {
    let mut qty = field("qty");
    qty.visible_when = Some("Order.qty > 1".into());
    let mut b = Book::new(vec![qty.clone()]);
    b.run();
    let runs = b.engine.evaluations();

    // The same text stays as it was.
    let mut design = b.calc.forms().get("Order", "Entry").unwrap().clone();
    design.title = Some("Entry".into());
    b.calc.forms_mut().set("Order", "Entry", design.clone());
    b.run();
    assert_eq!(b.engine.evaluations(), runs);

    design.controls[0].visible_when = Some("Order.qty > 5".into());
    b.calc.forms_mut().set("Order", "Entry", design.clone());
    b.run();
    assert_eq!(
        b.result(&[0], ExprKind::Visible),
        Some(Ok(Value::Bool(false)))
    );

    design.controls[0].visible_when = None;
    b.calc.forms_mut().set("Order", "Entry", design);
    b.calc.sync_expressions();
    assert!(
        b.calc
            .control_expression("Order", "Entry", &[0], ExprKind::Visible)
            .is_none()
    );
}

#[test]
fn nothing_runs_in_a_workbook_that_is_not_trusted() {
    let mut qty = field("qty");
    qty.visible_when = Some("Order.qty > 1".into());
    let mut b = Book::new(vec![qty]);
    b.calc.set_trusted(false);
    b.calc.sync_expressions();
    assert_eq!(b.calc.pending_expressions(), 0);
    b.run();
    assert_eq!(b.engine.evaluations(), 0);
    let state = b
        .calc
        .control_expression("Order", "Entry", &[0], ExprKind::Visible)
        .unwrap();
    assert_eq!(state.result, None);
    assert!(!state.pending);

    b.calc.set_trusted(true);
    b.run();
    assert_eq!(
        b.result(&[0], ExprKind::Visible),
        Some(Ok(Value::Bool(true)))
    );
}

#[test]
fn an_expression_inside_a_group_and_on_a_renamed_tab() {
    let mut inner = field("note");
    inner.text_colour = Some("\"#336699\"".into());
    let mut group = Control::new(ControlKind::Group);
    group.children = vec![field("qty"), inner];
    let mut b = Book::new(vec![group]);
    b.run();
    assert_eq!(
        b.result(&[0, 1], ExprKind::TextColour),
        Some(Ok(Value::Str("#336699".into())))
    );
    b.calc.rename_sheet("Order", "Sale").unwrap();
    b.run();
    assert!(
        b.calc
            .control_expression("Sale", "Entry", &[0, 1], ExprKind::TextColour)
            .and_then(|s| s.result)
            .is_some()
    );
}

// ------------------------------------------------------- columns of a table

mod columns {
    use std::sync::Arc;

    use cliotree_core::form::{ColumnDesign, Control, ControlKind, FormDesign};
    use cliotree_core::{Path, Value, json, jsonschema};

    use crate::python::{PythonEngine, PythonSyntax};
    use crate::{Calc, ExprKind};

    const SCHEMA: &str = r#"{
      "type": "object",
      "properties": {
        "trade": {"type": "boolean"},
        "items": {"type": "array", "items": {
          "type": "object",
          "properties": {
            "qty": {"type": "integer"},
            "size": {"type": "object", "properties": {"w": {"type": "number"}}}
          }
        }},
        "tags": {"type": "array", "items": {"type": "string"}}
      }
    }"#;

    const DATA: &str = r#"{"trade": false,
      "items": [{"qty": 1, "size": {"w": 2}}, {"qty": 5, "size": {"w": 9}}],
      "tags": ["a", "bb"]}"#;

    fn p(text: &str) -> Path {
        text.parse().unwrap()
    }

    struct Book {
        calc: Calc,
        engine: PythonEngine,
    }

    /// A table of `items` with these columns, and one of `tags`.
    fn book(columns: Vec<ColumnDesign>, tags: Vec<ColumnDesign>) -> Book {
        let mut calc = Calc::new(Arc::new(PythonSyntax));
        calc.add_sheet(
            "Order",
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(DATA).unwrap().node,
        )
        .unwrap();
        let mut items = Control::bound(ControlKind::Repeater, &p("items"));
        items.table_columns = columns;
        let mut list = Control::bound(ControlKind::Repeater, &p("tags"));
        list.table_columns = tags;
        calc.forms_mut().set(
            "Order",
            "Entry",
            FormDesign {
                controls: vec![items, list],
                ..FormDesign::default()
            },
        );
        Book {
            calc,
            engine: PythonEngine::new(),
        }
    }

    impl Book {
        fn run(&mut self) {
            self.calc.recalc(&self.engine);
        }

        fn result(
            &self,
            at: usize,
            column: &str,
            row: Option<usize>,
            kind: ExprKind,
        ) -> Option<Result<Value, String>> {
            self.calc
                .column_expression("Order", "Entry", &[at], column, row, kind)
                .and_then(|s| s.result)
        }
    }

    fn column(name: &str, set: impl FnOnce(&mut ColumnDesign)) -> ColumnDesign {
        let mut c = ColumnDesign::named(name);
        set(&mut c);
        c
    }

    fn red() -> Option<Result<Value, String>> {
        Some(Ok(Value::Str("red".into())))
    }

    #[test]
    fn a_colour_runs_for_each_row_at_its_cell_and_follows_the_rows() {
        let qty = column("qty", |c| {
            c.fill_colour = Some("'red' if this > 2 else None".into());
            c.enabled_when = Some("parent.size.w < 5".into());
        });
        let mut b = book(vec![qty], Vec::new());
        b.run();
        assert_eq!(
            b.result(0, "qty", Some(0), ExprKind::FillColour),
            Some(Ok(Value::Null))
        );
        assert_eq!(b.result(0, "qty", Some(1), ExprKind::FillColour), red());
        assert_eq!(
            b.result(0, "qty", Some(1), ExprKind::Enabled),
            Some(Ok(Value::Bool(false)))
        );
        // An edit of one row runs that row again.
        b.calc
            .set_value("Order", &p("items[0].qty"), Value::Int(7))
            .unwrap();
        b.run();
        assert_eq!(b.result(0, "qty", Some(0), ExprKind::FillColour), red());

        // A new row gets its own; a removed one goes.
        b.calc
            .insert_item(
                "Order",
                &p("items"),
                2,
                json::parse(r#"{"qty": 3}"#).unwrap().node.value().clone(),
            )
            .unwrap();
        b.run();
        assert_eq!(b.result(0, "qty", Some(2), ExprKind::FillColour), red());
        b.calc.remove("Order", &p("items[2]")).unwrap();
        b.calc.remove("Order", &p("items[1]")).unwrap();
        b.run();
        assert!(
            b.calc
                .column_expression("Order", "Entry", &[0], "qty", Some(1), ExprKind::FillColour)
                .is_none()
        );
        assert_eq!(b.result(0, "qty", Some(0), ExprKind::FillColour), red());
    }

    #[test]
    fn visible_when_runs_once_for_the_column_at_the_array() {
        let qty = column("qty", |c| {
            c.visible_when = Some("len(this) > 1 and root.trade".into());
        });
        let mut b = book(vec![qty], Vec::new());
        b.run();
        assert_eq!(
            b.result(0, "qty", None, ExprKind::Visible),
            Some(Ok(Value::Bool(false)))
        );
        // Asked for with a row, it is the same one.
        assert_eq!(
            b.result(0, "qty", Some(1), ExprKind::Visible),
            Some(Ok(Value::Bool(false)))
        );
        b.calc
            .set_value("Order", &p("trade"), Value::Bool(true))
            .unwrap();
        b.run();
        assert_eq!(
            b.result(0, "qty", None, ExprKind::Visible),
            Some(Ok(Value::Bool(true)))
        );
    }

    #[test]
    fn a_dotted_column_and_a_column_of_plain_items_find_their_cells() {
        let w = column("size.w", |c| {
            c.text_colour = Some("'red' if this > 5 else None".into());
        });
        let value = column("value", |c| {
            c.text_colour = Some("'red' if len(this) > 1 else None".into());
        });
        // Not columns of the items: nothing is run for them.
        let nothing = column("nothing", |c| c.text_colour = Some("'red'".into()));
        let rows = column("#", |c| c.text_colour = Some("'red'".into()));
        let hidden = column("qty", |c| {
            c.hidden = true;
            c.text_colour = Some("'red'".into());
        });
        let mut b = book(vec![w, nothing, rows, hidden], vec![value]);
        b.run();
        assert_eq!(
            b.result(0, "size.w", Some(0), ExprKind::TextColour),
            Some(Ok(Value::Null))
        );
        assert_eq!(b.result(0, "size.w", Some(1), ExprKind::TextColour), red());
        assert_eq!(
            b.result(1, "value", Some(0), ExprKind::TextColour),
            Some(Ok(Value::Null))
        );
        assert_eq!(b.result(1, "value", Some(1), ExprKind::TextColour), red());
        for name in ["nothing", "#", "qty"] {
            assert_eq!(
                b.result(0, name, Some(0), ExprKind::TextColour),
                None,
                "{name}"
            );
        }
    }
}
