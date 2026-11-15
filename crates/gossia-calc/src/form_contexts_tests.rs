//! Tests of the contexts of canvas forms (DESIGN.md, Later versions, "Form
//! canvases, 0.7.0"): their values, paths bound with them, the rows of a
//! repeater section, and `context` in expressions, with the Python engine.

use std::sync::Arc;

use gossia_core::form::{Control, ControlKind, FormDesign};
use gossia_core::{Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, Contexts, ExprKind, contexts_of};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "customer": {"type": "string"},
    "orders": {"type": "array", "items": {
      "type": "object",
      "properties": {
        "no": {"type": "integer"},
        "lines": {"type": "array", "items": {
          "type": "object",
          "properties": {"sku": {"type": "string"}, "qty": {"type": "integer"}}
        }}
      }
    }}
  }
}"#;

const DATA: &str = r#"{"customer": "Ada",
  "orders": [
    {"no": 1, "lines": [{"sku": "a", "qty": 2}, {"sku": "b", "qty": 3}]},
    {"no": 2, "lines": [{"sku": "c", "qty": 10}]}
  ]}"#;

fn context(name: &str, default: Option<&str>) -> Control {
    Control {
        context: Some(name.to_owned()),
        context_default: default.map(str::to_owned),
        ..Control::new(ControlKind::Field)
    }
}

fn shows(value: &str) -> Control {
    Control {
        value: Some(value.to_owned()),
        ..Control::new(ControlKind::Field)
    }
}

/// A canvas form: `header` on its header, a repeater over `orders` with
/// `orders_row` in its row, and a second over `orders[row].lines` with
/// `lines_row`.
fn design(header: Vec<Control>, orders_row: Vec<Control>, lines_row: Vec<Control>) -> FormDesign {
    let mut d = FormDesign::new_canvas(None);
    d.controls[0].children = header;
    let at = d.add_repeater("orders").unwrap();
    d.controls[at].children = orders_row;
    let at = d.add_repeater("orders[row].lines").unwrap();
    d.controls[at].children = lines_row;
    d
}

struct Book {
    calc: Calc,
    engine: PythonEngine,
}

fn book(design: FormDesign) -> Book {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Sales",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();

    calc.forms_mut().set("Sales", "View", design);
    let mut b = Book {
        calc,
        engine: PythonEngine::new(),
    };
    b.calc.recalc(&b.engine);
    b
}

impl Book {
    fn at(&self, at: &[usize], rows: &[usize], kind: ExprKind) -> Option<Result<Value, String>> {
        self.calc
            .control_expression_at("Sales", "View", at, rows, kind)
            .and_then(|s| s.result)
    }

    fn value(&self, at: &[usize], rows: &[usize]) -> Option<Result<Value, String>> {
        self.at(at, rows, ExprKind::Value)
    }

    fn set(&mut self, name: &str, value: Value) {
        self.calc.set_form_context("Sales", "View", name, value);
        self.calc.recalc(&self.engine);
    }

    fn contexts(&self) -> Contexts {
        let design = self.calc.forms().get("Sales", "View").unwrap().clone();
        self.calc.form_contexts("Sales", "View", &design)
    }
}

fn ok(v: Value) -> Option<Result<Value, String>> {
    Some(Ok(v))
}

#[test]
fn contexts_start_at_their_defaults_and_rows_at_zero() {
    let d = design(
        vec![context("sel", Some("1")), context("name", None)],
        vec![],
        vec![],
    );
    let data = json::parse(DATA).unwrap().node;
    let got = contexts_of(&d, Some(&data), None);
    let want: Contexts = [
        ("sel".to_owned(), Value::Int(1)),
        ("name".to_owned(), Value::Null),
        ("row".to_owned(), Value::Int(0)),
        ("row2".to_owned(), Value::Int(0)),
    ]
    .into_iter()
    .collect();
    assert_eq!(got, want);
    // A row chosen past the end is the last; the lines follow the order
    // chosen, which has one line.
    let chosen: Contexts = [
        ("row".to_owned(), Value::Int(7)),
        ("row2".to_owned(), Value::Int(5)),
    ]
    .into_iter()
    .collect();
    let got = contexts_of(&d, Some(&data), Some(&chosen));
    assert_eq!(got["row"], Value::Int(1));
    assert_eq!(got["row2"], Value::Int(0));
    // An empty array: no current row.
    let empty = json::parse(r#"{"orders": []}"#).unwrap().node;
    let got = contexts_of(&d, Some(&empty), None);
    assert_eq!(got["row"], Value::Null);
    assert_eq!(got["row2"], Value::Null);
    // A form that is not a canvas has none.
    assert!(contexts_of(&FormDesign::default(), Some(&data), None).is_empty());
}

#[test]
fn each_row_runs_its_expressions_with_its_item_and_its_row_number() {
    let b = book(design(
        vec![],
        vec![shows("this.no * 10"), shows("context.row")],
        vec![],
    ));
    assert_eq!(b.value(&[1, 0], &[0]), ok(Value::Int(10)));
    assert_eq!(b.value(&[1, 0], &[1]), ok(Value::Int(20)));
    // Inside a row its row name is that row.
    assert_eq!(b.value(&[1, 1], &[0]), ok(Value::Int(0)));
    assert_eq!(b.value(&[1, 1], &[1]), ok(Value::Int(1)));
}

#[test]
fn a_path_follows_the_context_it_uses() {
    let mut field = Control::new(ControlKind::Field);
    field.path = Some("orders[sel].no".into());
    field.visible_when = Some("this > 1".into());
    let mut b = book(design(
        vec![context("sel", Some("0")), field, shows("context.sel")],
        vec![],
        vec![],
    ));
    assert_eq!(
        b.at(&[0, 1], &[], ExprKind::Visible),
        ok(Value::Bool(false))
    );
    assert_eq!(b.value(&[0, 2], &[]), ok(Value::Int(0)));
    b.set("sel", Value::Int(1));
    assert_eq!(b.at(&[0, 1], &[], ExprKind::Visible), ok(Value::Bool(true)));
    assert_eq!(b.value(&[0, 2], &[]), ok(Value::Int(1)));
}

#[test]
fn the_detail_follows_the_current_row_of_its_master() {
    let mut b = book(design(vec![], vec![], vec![shows("this.qty")]));
    assert_eq!(b.value(&[3, 0], &[0]), ok(Value::Int(2)));
    assert_eq!(b.value(&[3, 0], &[1]), ok(Value::Int(3)));
    b.set("row", Value::Int(1));
    assert_eq!(b.contexts()["row"], Value::Int(1));
    assert_eq!(b.value(&[3, 0], &[0]), ok(Value::Int(10)));
    assert_eq!(b.value(&[3, 0], &[1]), None);
}

#[test]
fn a_context_change_leaves_alone_what_does_not_use_contexts() {
    let mut b = book(design(
        vec![
            context("sel", None),
            shows("Sales.customer"),
            shows("context.sel"),
        ],
        vec![],
        vec![],
    ));
    b.calc
        .set_form_context("Sales", "View", "sel", Value::Int(4));
    b.calc.sync_expressions();
    let state = |at: &[usize]| {
        b.calc
            .control_expression("Sales", "View", at, ExprKind::Value)
            .unwrap()
    };
    assert!(!state(&[0, 1]).pending);
    assert!(state(&[0, 2]).pending);
}

#[test]
fn chosen_contexts_are_kept_by_form() {
    let mut b = book(design(vec![context("sel", None)], vec![], vec![]));
    assert!(b.calc.chosen_contexts("Sales", "View").is_none());
    b.set("sel", Value::Int(2));
    assert_eq!(
        b.calc.chosen_contexts_of_tab("Sales"),
        vec![(
            "View".to_owned(),
            [("sel".to_owned(), Value::Int(2))].into_iter().collect()
        )]
    );
    b.calc.set_chosen_contexts("Sales", "View", Contexts::new());
    assert!(b.calc.chosen_contexts_of_tab("Sales").is_empty());
}
