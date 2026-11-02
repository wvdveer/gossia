//! The expressions of reports (DESIGN.md, "Reports"): they run as a form's
//! do, while the report is shown, with no node of their own.

use std::sync::Arc;

use cliotree_core::form::{ColumnDesign, Control, ControlKind, RepeaterMode};
use cliotree_core::report::{ReportDesign, ReportKind};
use cliotree_core::{Path, Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, ExprKind};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "items": {"type": "array", "items": {
      "type": "object",
      "properties": {"sku": {"type": "string"}, "qty": {"type": "integer"}}
    }}
  }
}"#;

const DATA: &str = r#"{"items": [{"sku": "a", "qty": 2}, {"sku": "b", "qty": 5}]}"#;

struct Book {
    calc: Calc,
    engine: PythonEngine,
}

fn book(controls: Vec<Control>) -> Book {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Orders",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls = controls;
    calc.create_report("Sales", design).unwrap();
    Book {
        calc,
        engine: PythonEngine::new(),
    }
}

impl Book {
    fn show(&mut self) {
        self.calc.show_report(Some("Sales"));
        self.calc.recalc(&self.engine);
    }

    fn at(&self, at: &[usize], rows: &[usize], kind: ExprKind) -> Option<Result<Value, String>> {
        self.calc
            .report_expression("Sales", at, rows, kind)
            .and_then(|s| s.result)
    }

    fn cell(&self, at: &[usize], column: &str, row: usize) -> Option<Result<Value, String>> {
        self.calc
            .report_column_expression("Sales", at, &[], column, Some(row), ExprKind::Value)
            .and_then(|s| s.result)
    }
}

fn field(value: &str) -> Control {
    let mut c = Control::new(ControlKind::Field);
    c.value = Some(value.to_owned());
    c
}

fn ok(v: Value) -> Option<Result<Value, String>> {
    Some(Ok(v))
}

#[test]
fn a_report_runs_only_while_it_is_shown_and_follows_the_data() {
    let mut b = book(vec![field("sum(i.qty for i in Orders.items)")]);
    b.calc.recalc(&b.engine);
    assert_eq!(b.at(&[0], &[], ExprKind::Value), None);

    b.show();
    assert_eq!(b.calc.shown_report(), Some("Sales"));
    assert_eq!(b.at(&[0], &[], ExprKind::Value), ok(Value::Int(7)));
    let path: Path = "items[1].qty".parse().unwrap();
    b.calc.set_value("Orders", &path, Value::Int(10)).unwrap();
    b.calc.recalc(&b.engine);
    assert_eq!(b.at(&[0], &[], ExprKind::Value), ok(Value::Int(12)));

    b.calc.show_report(None);
    b.calc.recalc(&b.engine);
    assert_eq!(b.at(&[0], &[], ExprKind::Value), None);
}

#[test]
fn this_parent_and_root_are_none_outside_items() {
    let mut b = book(vec![field(
        "this is None and parent is None and root is None",
    )]);
    b.show();
    assert_eq!(b.at(&[0], &[], ExprKind::Value), ok(Value::Bool(true)));
}

#[test]
fn designed_items_and_tables_over_expressions_run_for_each_item() {
    let mut stacked = Control::new(ControlKind::Repeater);
    stacked.mode = Some(RepeaterMode::Stacked);
    stacked.items = Some("Orders.items".into());
    stacked.item_name = Some("item".into());
    stacked.children = vec![field("f'{item.sku}: {this.qty}'")];

    let mut table = Control::new(ControlKind::Repeater);
    table.mode = Some(RepeaterMode::Table);
    table.items = Some("Orders.items".into());
    table.item_name = Some("line".into());
    let mut twice = ColumnDesign::named("twice");
    twice.value = Some("line.qty * 2".into());
    table.table_columns = vec![ColumnDesign::named("sku"), twice];

    let mut b = book(vec![stacked, table]);
    b.show();
    let s = |t: &str| ok(Value::Str(t.to_owned()));
    assert_eq!(b.at(&[0, 0], &[0], ExprKind::Value), s("a: 2"));
    assert_eq!(b.at(&[0, 0], &[1], ExprKind::Value), s("b: 5"));
    assert_eq!(b.cell(&[1], "twice", 1), ok(Value::Int(10)));
}

#[test]
fn a_renamed_report_runs_under_its_new_name_once_shown() {
    let mut b = book(vec![field("1 + 1")]);
    b.show();
    b.calc.rename_sheet("Sales", "Revenue").unwrap();
    b.calc.show_report(Some("Revenue"));
    b.calc.recalc(&b.engine);
    assert_eq!(
        b.calc
            .report_expression("Revenue", &[0], &[], ExprKind::Value)
            .and_then(|s| s.result),
        ok(Value::Int(2))
    );
}
