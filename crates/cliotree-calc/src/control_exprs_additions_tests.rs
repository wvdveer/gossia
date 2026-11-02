//! Fields that show an expression, label templates, repeaters whose items an
//! expression gives, and designed items that nest (DESIGN.md, Forms,
//! "Additions in 0.2.0, shared with reports"), with the Python engine.

use std::sync::Arc;

use cliotree_core::form::{ColumnDesign, Control, ControlKind, FormDesign, RepeaterMode};
use cliotree_core::{Path, Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, ExprKind};

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

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

struct Book {
    calc: Calc,
    engine: PythonEngine,
}

fn book(controls: Vec<Control>) -> Book {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Sales",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.forms_mut().set(
        "Sales",
        "View",
        FormDesign {
            controls,
            ..FormDesign::default()
        },
    );
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

    fn cell(
        &self,
        at: &[usize],
        column: &str,
        row: usize,
        kind: ExprKind,
    ) -> Option<Result<Value, String>> {
        self.calc
            .column_expression("Sales", "View", at, column, Some(row), kind)
            .and_then(|s| s.result)
    }

    fn edit(&mut self, path: &str, value: Value) {
        self.calc.set_value("Sales", &p(path), value).unwrap();
        self.calc.recalc(&self.engine);
    }
}

fn ok(v: Value) -> Option<Result<Value, String>> {
    Some(Ok(v))
}

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

#[test]
fn a_field_shows_an_expression_and_follows_what_it_read() {
    let mut total = Control::new(ControlKind::Field);
    total.value = Some("sum(len(o.lines) for o in Sales.orders)".into());
    let mut b = book(vec![total]);
    assert_eq!(b.at(&[0], &[], ExprKind::Value), ok(Value::Int(3)));
    b.edit("orders[1].lines", Value::Array(Vec::new()));
    assert_eq!(b.at(&[0], &[], ExprKind::Value), ok(Value::Int(2)));
}

#[test]
fn a_label_template_is_filled_in_and_fixed_text_runs_nothing() {
    let mut label = Control::new(ControlKind::Label);
    label.template = true;
    label.text = Some("{Sales.customer} has {len(Sales.orders)} orders, {1234.5:,.2f}".into());
    let mut fixed = Control::new(ControlKind::Label);
    fixed.template = true;
    fixed.text = Some("No {{braces}} here".into());
    let mut broken = Control::new(ControlKind::Label);
    broken.template = true;
    broken.text = Some("open {".into());
    let b = book(vec![label, fixed, broken]);
    assert_eq!(
        b.at(&[0], &[], ExprKind::Template),
        ok(s("Ada has 2 orders, 1,234.50"))
    );
    assert_eq!(b.at(&[1], &[], ExprKind::Template), None);
    let Some(Err(message)) = b.at(&[2], &[], ExprKind::Template) else {
        panic!("a template that cannot be read is an error");
    };
    assert!(message.contains("not closed"), "{message}");
}

/// A stacked repeater over an expression, with a stacked repeater over
/// another inside its items: the inner controls see both items by name.
#[test]
fn repeaters_over_expressions_nest_and_name_their_items() {
    let mut no = Control::new(ControlKind::Field);
    no.value = Some("f'{order.no}/{line.sku}x{this.qty} of {len(lines)}'".into());
    let mut inner = Control::new(ControlKind::Repeater);
    inner.mode = Some(RepeaterMode::Stacked);
    inner.items = Some("order.lines".into());
    inner.item_name = Some("line".into());
    inner.list_name = Some("lines".into());
    inner.children = vec![no];
    let mut outer = Control::new(ControlKind::Repeater);
    outer.mode = Some(RepeaterMode::Stacked);
    outer.items = Some("[o for o in Sales.orders]".into());
    outer.item_name = Some("order".into());
    outer.children = vec![inner];
    let mut b = book(vec![outer]);
    let items = b.at(&[0], &[], ExprKind::Items);
    assert!(
        matches!(items, Some(Ok(Value::Array(ref a))) if a.len() == 2),
        "{items:?}"
    );
    assert_eq!(
        b.at(&[0, 0, 0], &[0, 0], ExprKind::Value),
        ok(s("1/ax2 of 2"))
    );
    assert_eq!(
        b.at(&[0, 0, 0], &[0, 1], ExprKind::Value),
        ok(s("1/bx3 of 2"))
    );
    assert_eq!(
        b.at(&[0, 0, 0], &[1, 0], ExprKind::Value),
        ok(s("2/cx10 of 1"))
    );
    assert_eq!(b.at(&[0, 0, 0], &[1, 1], ExprKind::Value), None);
    // A line added in the data is a new item, with its own expression.
    let lines = json::parse(r#"[{"sku": "c", "qty": 10}, {"sku": "d", "qty": 1}]"#).unwrap();
    b.edit("orders[1].lines", lines.node.value().clone());
    assert_eq!(
        b.at(&[0, 0, 0], &[1, 1], ExprKind::Value),
        ok(s("2/dx1 of 2"))
    );
}

#[test]
fn designed_items_bound_to_arrays_nest() {
    let mut sku = Control::bound(ControlKind::Field, &p("sku"));
    sku.text_colour = Some("'red' if this == 'b' else None".into());
    let mut lines = Control::bound(ControlKind::Repeater, &p("lines"));
    lines.mode = Some(RepeaterMode::Stacked);
    lines.children = vec![sku];
    let mut orders = Control::bound(ControlKind::Repeater, &p("orders"));
    orders.mode = Some(RepeaterMode::Stacked);
    orders.children = vec![lines];
    let b = book(vec![orders]);
    assert_eq!(
        b.at(&[0, 0, 0], &[0, 0], ExprKind::TextColour),
        ok(Value::Null)
    );
    assert_eq!(
        b.at(&[0, 0, 0], &[0, 1], ExprKind::TextColour),
        ok(s("red"))
    );
    assert_eq!(
        b.at(&[0, 0, 0], &[1, 0], ExprKind::TextColour),
        ok(Value::Null)
    );
}

#[test]
fn a_table_over_an_expression_runs_its_columns_for_each_row() {
    let mut qty = ColumnDesign::named("qty");
    qty.text_colour = Some("'red' if this > 2 else None".into());
    let mut double = ColumnDesign::named("double");
    double.value = Some("line.qty * 2".into());
    let mut table = Control::new(ControlKind::Repeater);
    table.items = Some("[l for o in Sales.orders for l in o.lines]".into());
    table.item_name = Some("line".into());
    table.table_columns = vec![qty, double];
    let b = book(vec![table]);
    assert_eq!(
        b.cell(&[0], "qty", 0, ExprKind::TextColour),
        ok(Value::Null)
    );
    assert_eq!(b.cell(&[0], "qty", 1, ExprKind::TextColour), ok(s("red")));
    assert_eq!(
        b.cell(&[0], "double", 2, ExprKind::Value),
        ok(Value::Int(20))
    );
}

#[test]
fn items_that_are_not_a_list_put_nothing_inside() {
    let mut field = Control::new(ControlKind::Field);
    field.value = Some("this".into());
    let mut repeater = Control::new(ControlKind::Repeater);
    repeater.mode = Some(RepeaterMode::Stacked);
    repeater.items = Some("Sales.customer".into());
    repeater.children = vec![field];
    let b = book(vec![repeater]);
    assert_eq!(b.at(&[0], &[], ExprKind::Items), ok(s("Ada")));
    assert_eq!(b.at(&[0, 0], &[0], ExprKind::Value), None);
}

#[test]
fn the_new_expressions_are_in_the_trust_digest() {
    let plain = book(Vec::new()).calc.fingerprint();
    let mut label = Control::new(ControlKind::Label);
    label.template = true;
    label.text = Some("{1 + 1}".into());
    assert_ne!(book(vec![label.clone()]).calc.fingerprint(), plain);
    label.text = Some("fixed".into());
    assert_eq!(book(vec![label]).calc.fingerprint(), plain);
    let mut repeater = Control::new(ControlKind::Repeater);
    repeater.items = Some("[1]".into());
    let with_items = book(vec![repeater.clone()]).calc.fingerprint();
    assert_ne!(with_items, plain);
    repeater.item_name = Some("n".into());
    assert_ne!(book(vec![repeater]).calc.fingerprint(), with_items);
}

/// A fixed colour (`#rrggbb`) runs no Python: it is not an expression of the
/// control and not in the trust digest; one given by an expression is both.
#[test]
fn a_fixed_colour_runs_nothing_and_is_not_in_the_trust_digest() {
    let plain = book(Vec::new()).calc.fingerprint();
    let mut label = Control::new(ControlKind::Label);
    label.text = Some("Hello".into());
    let bare = book(vec![label.clone()]).calc.fingerprint();
    assert_eq!(bare, plain);
    label.text_colour = Some("#ff0000".into());
    label.fill_colour = Some(" #00FF00 ".into());
    let mut column = ColumnDesign::named("no");
    column.fill_colour = Some("#123456".into());
    let mut table = Control::bound(ControlKind::Repeater, &p("orders"));
    table.mode = Some(RepeaterMode::Table);
    table.table_columns = vec![column];
    let b = book(vec![label.clone(), table.clone()]);
    assert_eq!(b.at(&[0], &[], ExprKind::TextColour), None);
    assert_eq!(b.cell(&[1], "no", 0, ExprKind::FillColour), None);
    let fixed = b.calc.fingerprint();
    assert_eq!(
        fixed,
        book(vec![label.clone(), {
            let mut t = table.clone();
            t.table_columns.clear();
            t
        }])
        .calc
        .fingerprint()
    );
    // An expression giving the same colour runs, and is in the digest.
    label.text_colour = Some("'#ff0000'".into());
    let b = book(vec![label, table]);
    assert_eq!(b.at(&[0], &[], ExprKind::TextColour), ok(s("#ff0000")));
    assert_ne!(b.calc.fingerprint(), fixed);
}
