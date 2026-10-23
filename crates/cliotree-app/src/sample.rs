//! The workbook the application opens with, until it can open files: two tabs
//! and the schema tab of each, with a few formulas, enough to try everything the window does.

use std::sync::Arc;

use cliotree_calc::Calc;
use cliotree_calc::python::PythonSyntax;
use cliotree_core::{Path, json, jsonschema};

const ORDERS_SCHEMA: &str = r#"{
  "type": "object",
  "title": "An order",
  "properties": {
    "customer": {
      "type": "object",
      "properties": {
        "name": {"type": "string"},
        "city": {"type": "string"},
        "vip": {"type": "boolean"},
        "status": {"type": "string", "enum": ["new", "paid", "shipped"]}
      },
      "required": ["name", "status"]
    },
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "sku": {"type": "string"},
          "qty": {"type": "integer", "minimum": 1},
          "price": {"type": "number", "minimum": 0},
          "total": {"type": "number"},
          "note": {"type": "string"}
        },
        "required": ["sku", "qty", "price"]
      }
    },
    "tax_rate": {"type": "number", "minimum": 0, "maximum": 1},
    "subtotal": {"type": "number"},
    "tax": {"type": "number"},
    "total": {"type": "number"}
  },
  "required": ["customer", "items", "tax_rate", "subtotal", "tax", "total"]
}"#;

const ORDERS_DATA: &str = r#"{
  "customer": {"name": "Acme Ltd", "city": "Utrecht", "vip": true, "status": "paid"},
  "items": [
    {"sku": "A-100", "qty": 2, "price": 9.5},
    {"sku": "B-220", "qty": 1, "price": 24.0, "note": "gift wrap"},
    {"sku": "C-310", "qty": 12, "price": 0.75}
  ],
  "tax_rate": 0.21,
  "subtotal": 0, "tax": 0, "total": 0
}"#;

const PRICES_SCHEMA: &str = r#"{
  "type": "object",
  "title": "Prices",
  "properties": {
    "currency": {"type": "string"},
    "list": {"type": "object", "additionalProperties": {"type": "number"}}
  },
  "required": ["currency", "list"]
}"#;

const PRICES_DATA: &str = r#"{
  "currency": "EUR",
  "list": {"A-100": 9.5, "B-220": 24.0, "C-310": 0.75, "D-400": 119.0}
}"#;

/// The sample workbook with its formulas entered (not yet calculated).
pub fn workbook() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    for (name, schema, data) in [
        ("Orders", ORDERS_SCHEMA, ORDERS_DATA),
        ("Prices", PRICES_SCHEMA, PRICES_DATA),
    ] {
        calc.add_sheet(
            name,
            jsonschema::lower_str(schema).expect("the sample schema is valid"),
            json::parse(data).expect("the sample data is JSON").node,
        )
        .expect("the sample tabs have different names");
    }
    // Each tab's type is written on a schema tab of its own, which is where
    // the tab's fields can be changed once auto-update is switched on.
    for (tab, schema) in [("Orders", ORDERS_SCHEMA), ("Prices", PRICES_SCHEMA)] {
        let name = format!("{tab} schema");
        calc.add_schema_sheet(
            &name,
            json::parse(schema).expect("the sample schema is JSON").node,
        )
        .expect("the sample tabs have different names");
        calc.bind_schema(tab, &name)
            .expect("the sample schema can be used");
    }
    let p = |text: &str| -> Path { text.parse().expect("a path") };
    for i in 0..3 {
        calc.set_formula(
            "Orders",
            &p(&format!("items[{i}].total")),
            "parent.qty * parent.price",
        )
        .expect("a formula can go there");
    }
    for (path, formula) in [
        ("subtotal", "round(sum(Orders.items.total), 2)"),
        ("tax", "round(subtotal_ * Orders.tax_rate, 2)"),
        ("total", "Orders.subtotal + Orders.tax"),
    ] {
        let formula = formula.replace("subtotal_", "Orders.subtotal");
        calc.set_formula("Orders", &p(path), &formula)
            .expect("a formula can go there");
    }
    // The sample starts as the saved state, with nothing to undo.
    calc.clear_history();
    calc.mark_saved();
    calc
}
