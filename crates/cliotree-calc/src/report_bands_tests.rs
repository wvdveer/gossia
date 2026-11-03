//! The bands of a multipage report (DESIGN.md, Reports, "Multipage"): each
//! runs once for each place it is drawn, and sees the items of the sections
//! around it by name.

use std::sync::Arc;

use cliotree_core::form::{Control, ControlKind};
use cliotree_core::report::{ReportDesign, ReportKind, add_section};
use cliotree_core::{Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, ExprKind};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "units": {"type": "array", "items": {
      "type": "object",
      "properties": {
        "name": {"type": "string"},
        "orders": {"type": "array", "items": {
          "type": "object", "properties": {"amount": {"type": "integer"}}
        }}
      }
    }}
  }
}"#;

const DATA: &str = r#"{"units": [
  {"name": "Sales", "orders": [{"amount": 5}, {"amount": 7}]},
  {"name": "Finance", "orders": [{"amount": 1}]}
]}"#;

fn field(value: &str) -> Control {
    let mut c = Control::new(ControlKind::Field);
    c.value = Some(value.to_owned());
    c
}

/// Report header, page header, the header of the section of units, the
/// detail band of their orders, the section's footer, page footer, report
/// footer: each with one field.
fn design() -> ReportDesign {
    let mut design = ReportDesign::new(ReportKind::Multipage);
    add_section(&mut design.layout);
    let bands = &mut design.layout.controls;
    bands[2].items = Some("Shop.units".into());
    bands[2].item_name = Some("unit".into());
    bands[2].list_name = Some("units".into());
    bands[3].items = Some("unit.orders".into());
    bands[3].item_name = Some("order".into());
    bands[3].list_name = Some("rows".into());
    let fields = [
        "len(units)",
        "f'{page.number}/{page.count}'",
        "unit.name",
        "f'{unit.name} {order.amount * 10}'",
        "sum(r.amount for r in rows)",
        "page.number",
        "sum(len(u.orders) for u in units)",
    ];
    for (band, text) in bands.iter_mut().zip(fields) {
        band.children.push(field(text));
    }
    design
}

struct Book {
    calc: Calc,
    engine: PythonEngine,
}

fn book(design: ReportDesign) -> Book {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Shop",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.create_report("Units", design).unwrap();
    let mut b = Book {
        calc,
        engine: PythonEngine::new(),
    };
    b.calc.show_report(Some("Units"));
    b.settle();
    b
}

impl Book {
    /// Recalculates until nothing is left to run: the items of each level
    /// decide what runs inside them.
    fn settle(&mut self) {
        for _ in 0..8 {
            self.calc.sync_expressions();
            if self.calc.pending_expressions() == 0 {
                return;
            }
            self.calc.recalc(&self.engine);
        }
    }

    fn at(&self, band: usize, rows: &[usize]) -> Option<Value> {
        match self
            .calc
            .report_expression("Units", &[band, 0], rows, ExprKind::Value)
            .and_then(|s| s.result)
        {
            Some(Ok(v)) => Some(v),
            Some(Err(e)) => panic!("band {band} at {rows:?}: {e}"),
            None => None,
        }
    }
}

fn s(text: &str) -> Option<Value> {
    Some(Value::Str(text.to_owned()))
}

fn i(n: i64) -> Option<Value> {
    Some(Value::Int(n))
}

#[test]
fn each_band_runs_for_each_place_it_is_drawn() {
    let b = book(design());
    // The section's header and footer for each unit.
    assert_eq!(b.at(2, &[0]), s("Sales"));
    assert_eq!(b.at(2, &[1]), s("Finance"));
    assert_eq!(b.at(2, &[2]), None);
    // The detail band for each order of each unit, seeing both items.
    assert_eq!(b.at(3, &[0, 1]), s("Sales 70"));
    assert_eq!(b.at(3, &[1, 0]), s("Finance 10"));
    assert_eq!(b.at(3, &[1, 1]), None);
    // A footer totals the list of the detail band inside it.
    assert_eq!(b.at(4, &[0]), i(12));
    assert_eq!(b.at(4, &[1]), i(1));
    // The report header and footer see the list of the outermost section.
    assert_eq!(b.at(0, &[]), i(2));
    assert_eq!(b.at(6, &[]), i(3));
    // The page header and footer, once for each page.
    assert_eq!(b.at(1, &[0]), s("1/1"));
    assert_eq!(b.at(5, &[0]), i(1));
}

#[test]
fn page_values_follow_the_pages() {
    let mut design = design();
    // Each order is 400 points tall: one to a page and a bit, so three pages.
    design.layout.controls[3].height = Some(400);
    let b = book(design);
    assert_eq!(b.at(1, &[0]), s("1/3"));
    assert_eq!(b.at(1, &[2]), s("3/3"));
    assert_eq!(b.at(5, &[2]), i(3));
    assert_eq!(b.at(1, &[3]), None);
}

#[test]
fn a_change_to_the_data_changes_the_occurrences() {
    let mut b = book(design());
    let path = "units[1].orders".parse().unwrap();
    let orders = json::parse(r#"[{"amount": 1}, {"amount": 2}]"#)
        .unwrap()
        .node;
    b.calc
        .set_value("Shop", &path, orders.value().clone())
        .unwrap();
    b.settle();
    assert_eq!(b.at(3, &[1, 1]), s("Finance 20"));
    assert_eq!(b.at(4, &[1]), i(3));
}

#[test]
fn a_hidden_band_takes_no_room_on_the_pages() {
    let mut design = design();
    design.layout.controls[3].height = Some(400);
    // Only the first order of each unit is drawn: two pages, not three.
    design.layout.controls[3].visible_when = Some("order == unit.orders[0]".into());
    let b = book(design);
    assert_eq!(b.at(1, &[0]), s("1/2"));
}

#[test]
fn a_comprehension_in_a_band_sees_the_names_of_the_sections_around() {
    let mut design = design();
    design.layout.controls[3].items =
        Some("[o for o in unit.orders if o.amount > len(unit.name)]".into());
    let b = book(design);
    // Sales has 5 letters: only the order of 7.
    assert_eq!(b.at(3, &[0, 0]), s("Sales 70"));
}
