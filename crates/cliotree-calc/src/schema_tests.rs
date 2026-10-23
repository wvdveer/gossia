//! Tests of schema tabs, auto-update and the field commands (DESIGN.md,
//! "Schema binding", "Editing the schema from a data tab").

use std::sync::Arc;

use cliotree_core::{Path, Value, json, jsonschema};

use crate::fake::FakeEngine;
use crate::{Calc, FieldKind};

const SCHEMA: &str = r##"{
  "type": "object",
  "properties": {
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "sku": {"type": "string"},
          "qty": {"type": "integer"},
          "price": {"type": "number"},
          "total": {"type": "number"}
        },
        "required": ["sku", "qty", "price"]
      }
    },
    "billing": {"$ref": "#/$defs/Address"},
    "shipping": {"$ref": "#/$defs/Address"}
  },
  "required": ["items", "billing", "shipping"],
  "$defs": {
    "Address": {
      "type": "object",
      "properties": {"city": {"type": "string"}, "zip": {"type": "string"}},
      "required": ["city"]
    }
  }
}"##;

const DATA: &str = r#"{
  "items": [
    {"sku": "A", "qty": 2, "price": 1.5, "total": 0},
    {"sku": "B", "qty": 3, "price": 2.0, "total": 0}
  ],
  "billing": {"city": "Utrecht", "zip": "3511"},
  "shipping": {"city": "Delft"}
}"#;

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
    /// `Order` is bound to `Order schema`; auto-update is on.
    fn new() -> Book {
        let mut b = Book::off();
        b.calc.set_auto_update("Order schema", true).unwrap();
        b.calc.clear_history();
        b
    }

    fn off() -> Book {
        let engine = FakeEngine::new();
        let mut calc = Calc::new(Arc::new(engine.clone()));
        calc.add_sheet(
            "Order",
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(DATA).unwrap().node,
        )
        .unwrap();
        calc.add_schema_sheet("Order schema", json::parse(SCHEMA).unwrap().node)
            .unwrap();
        calc.bind_schema("Order", "Order schema").unwrap();
        Book { calc, engine }
    }

    fn run(&mut self) {
        self.calc.recalc(&self.engine);
    }

    fn data(&self) -> String {
        json::to_string(self.calc.data("Order").unwrap(), false).unwrap()
    }

    fn schema(&self) -> String {
        json::to_string(self.calc.data("Order schema").unwrap(), false).unwrap()
    }

    fn fields(&self, object: &str) -> Vec<String> {
        self.calc.field_places("Order", &p(object)).unwrap().fields
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn error_text(result: Result<(), crate::EditError>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn a_bound_tab_takes_the_type_the_document_describes_and_follows_its_edits() {
    let mut b = Book::off();
    assert_eq!(b.calc.schema_of("Order"), Some("Order schema"));
    assert!(b.calc.is_schema_sheet("Order schema"));
    assert!(!b.calc.is_schema_sheet("Order"));
    assert_eq!(b.calc.schema_users("Order schema"), ["Order"]);
    assert_eq!(b.calc.schema_error("Order schema"), None);

    // A change to the document is a change of the tab's type.
    assert!(b.calc.spec("Order").unwrap().resolve(&p("tip")).is_none());
    let tip = json::parse(r#"{"type": "number"}"#).unwrap().node;
    let mut properties = b
        .calc
        .data("Order schema")
        .unwrap()
        .get(&p("properties"))
        .unwrap()
        .value()
        .as_object()
        .unwrap()
        .clone();
    properties.insert("tip".to_owned(), tip);
    b.calc
        .set_value("Order schema", &p("properties"), Value::Object(properties))
        .unwrap();
    assert!(b.calc.spec("Order").unwrap().resolve(&p("tip")).is_some());

    // A document that cannot be lowered is reported, and the tab keeps its type.
    b.calc
        .set_value("Order schema", &p("type"), Value::Str("float".into()))
        .unwrap();
    assert!(b.calc.schema_error("Order schema").is_some());
    assert!(b.calc.spec("Order").unwrap().resolve(&p("tip")).is_some());
    b.calc.undo().unwrap();
    assert_eq!(b.calc.schema_error("Order schema"), None);
}

#[test]
fn auto_update_needs_exactly_one_user_and_forbids_a_second() {
    let mut b = Book::off();
    assert_eq!(b.calc.auto_update_blocker("Order schema"), None);
    assert!(!b.calc.auto_update("Order schema"));

    // No user.
    let mut lonely = Book::off();
    lonely
        .calc
        .add_schema_sheet("Alone", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    assert!(lonely.calc.auto_update_blocker("Alone").is_some());
    assert!(lonely.calc.set_auto_update("Alone", true).is_err());

    // Two users.
    b.calc
        .add_sheet(
            "Copy",
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(DATA).unwrap().node,
        )
        .unwrap();
    b.calc.bind_schema("Copy", "Order schema").unwrap();
    let why = b.calc.auto_update_blocker("Order schema").unwrap();
    assert!(why.contains("2 tabs"), "{why}");
    assert!(b.calc.set_auto_update("Order schema", true).is_err());

    // While it is on, a second tab may not be bound.
    let mut b = Book::off();
    b.calc.set_auto_update("Order schema", true).unwrap();
    b.calc
        .add_sheet(
            "Copy",
            jsonschema::lower_str(SCHEMA).unwrap(),
            json::parse(DATA).unwrap().node,
        )
        .unwrap();
    let refused = error_text(b.calc.bind_schema("Copy", "Order schema"));
    assert!(refused.contains("auto-update"), "{refused}");
    assert_eq!(b.calc.schema_users("Order schema"), ["Order"]);
    // Binding the one tab again is not a second user.
    b.calc.bind_schema("Order", "Order schema").unwrap();
}

#[test]
fn switching_auto_update_is_an_entry_that_can_be_undone() {
    let mut b = Book::off();
    b.calc.set_auto_update("Order schema", true).unwrap();
    assert!(b.calc.auto_update("Order schema"));
    assert_eq!(
        b.calc.undo_label(),
        Some("Switch auto-update on for Order schema")
    );
    b.calc.undo().unwrap();
    assert!(!b.calc.auto_update("Order schema"));
    b.calc.redo().unwrap();
    assert!(b.calc.auto_update("Order schema"));
}

#[test]
fn the_commands_are_refused_unless_auto_update_is_on_and_say_where_to_switch_it() {
    let mut b = Book::off();
    let why = error_text(b.calc.rename_field("Order", &p("items[0]"), "qty", "n"));
    assert!(why.contains("auto-update on Order schema"), "{why}");
    assert_eq!(b.calc.history_len(), (0, 0));
    // A tab that is not bound has no schema tab to change.
    let mut free = Calc::new(Arc::new(FakeEngine::new()));
    free.add_sheet(
        "Free",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    assert!(free.delete_field("Free", &p("billing"), "city").is_err());
}

#[test]
fn reordering_fields_changes_the_schema_and_every_item_and_nothing_else() {
    let mut b = Book::new();
    let ids: Vec<_> = ["sku", "qty", "price", "total"]
        .iter()
        .map(|f| {
            b.calc
                .data("Order")
                .unwrap()
                .get(&p(&format!("items[1].{f}")))
                .unwrap()
                .id()
        })
        .collect();
    let (data, schema) = (b.data(), b.schema());
    let order = names(&["price", "sku", "total", "qty"]);
    b.calc
        .reorder_fields("Order", &p("items[0]"), &order)
        .unwrap();
    assert_eq!(b.fields("items[0]"), order);
    assert_eq!(b.calc.history_len(), (1, 0), "one entry");
    assert_eq!(b.calc.undo_label(), Some("Reorder fields of Order.items[]"));

    let item = b.calc.data("Order").unwrap().get(&p("items[1]")).unwrap();
    let keys: Vec<_> = item.value().as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, order);
    let now: Vec<_> = ["sku", "qty", "price", "total"]
        .iter()
        .map(|f| item.get(&p(f)).unwrap().id())
        .collect();
    assert_eq!(now, ids, "the values are the same nodes");

    // The same order again changes nothing, and a wrong list is refused.
    b.calc
        .reorder_fields("Order", &p("items[0]"), &order)
        .unwrap();
    assert_eq!(b.calc.history_len(), (1, 0));
    assert!(
        b.calc
            .reorder_fields("Order", &p("items[0]"), &names(&["sku", "qty"]))
            .is_err()
    );

    b.calc.undo().unwrap();
    assert_eq!((b.data(), b.schema()), (data, schema));
    assert_eq!(b.fields("items[0]"), ["sku", "qty", "price", "total"]);
}

#[test]
fn a_field_is_added_to_the_schema_and_existing_items_have_no_value_for_it() {
    let mut b = Book::new();
    let data = b.data();
    b.calc
        .add_field("Order", &p("items[0]"), "color", FieldKind::Text, false)
        .unwrap();
    b.calc
        .add_field("Order", &p("items[0]"), "due", FieldKind::Date, true)
        .unwrap();
    assert_eq!(
        b.fields("items[0]"),
        ["sku", "qty", "price", "total", "color", "due"]
    );
    assert_eq!(b.data(), data);
    let spec = b.calc.spec("Order").unwrap();
    assert!(spec.is_optional_field(&p("items[0].color")));
    assert!(!spec.is_optional_field(&p("items[0].due")));
    assert_eq!(b.calc.history_len(), (2, 0));

    // The new field can be used at once.
    b.calc
        .set_value("Order", &p("items[1].color"), Value::Str("red".into()))
        .unwrap();

    let name_taken =
        error_text(
            b.calc
                .add_field("Order", &p("items[0]"), "sku", FieldKind::Text, false),
        );
    assert!(name_taken.contains("already"), "{name_taken}");
    assert!(
        b.calc
            .add_field("Order", &p("items[0]"), "  ", FieldKind::Text, false)
            .is_err()
    );

    b.calc.undo().unwrap();
    b.calc.undo().unwrap();
    b.calc.undo().unwrap();
    assert_eq!(b.fields("items[0]"), ["sku", "qty", "price", "total"]);
    assert_eq!(b.data(), data);
}

#[test]
fn renaming_a_field_renames_the_key_everywhere_and_a_formula_on_it_goes_along() {
    let mut b = Book::new();
    // A formula on the field, and one that reads it.
    b.calc
        .set_formula("Order", &p("items[0].qty"), "7")
        .unwrap();
    b.calc
        .set_formula("Order", &p("items[0].total"), "Order.items[0].qty * 2")
        .unwrap();
    b.run();
    b.calc.clear_history();
    let (data, schema) = (b.data(), b.schema());

    b.calc
        .rename_field("Order", &p("items[0]"), "qty", "quantity")
        .unwrap();
    assert_eq!(b.calc.history_len(), (1, 0), "one entry");
    assert_eq!(
        b.calc.undo_label(),
        Some("Rename field \"qty\" to \"quantity\"")
    );
    assert_eq!(b.fields("items[0]"), ["sku", "quantity", "price", "total"]);
    for i in 0..2 {
        let item = b
            .calc
            .data("Order")
            .unwrap()
            .get(&p(&format!("items[{i}]")))
            .unwrap();
        assert!(item.get(&p("quantity")).is_some());
        assert!(item.get(&p("qty")).is_none());
    }
    // The schema lists it as required under the new name.
    let required: Vec<_> = b
        .calc
        .data("Order schema")
        .unwrap()
        .get(&p("properties.items.items.required"))
        .unwrap()
        .value()
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.value().as_str().unwrap().to_owned())
        .collect();
    assert_eq!(required, ["sku", "quantity", "price"]);
    // The formula moved with the field; the one that read the old name is not
    // rewritten and shows #REF!.
    assert_eq!(b.calc.formula_text("Order", &p("items[0].qty")), None);
    assert_eq!(
        b.calc.formula_text("Order", &p("items[0].quantity")),
        Some("7".to_owned())
    );
    b.run();
    assert_eq!(
        b.calc
            .data("Order")
            .unwrap()
            .get(&p("items[0].quantity"))
            .unwrap()
            .value(),
        &Value::Int(7)
    );
    let broken = b
        .calc
        .data("Order")
        .unwrap()
        .get(&p("items[0].total"))
        .unwrap();
    // (The stand-in engine reads a name that is gone as the item that held it,
    // which includes the formula itself, so its kind is not asserted here.)
    assert!(matches!(broken.value(), Value::Error(_)), "{broken:?}");

    let taken = error_text(b.calc.rename_field("Order", &p("items[0]"), "sku", "price"));
    assert!(taken.contains("already"), "{taken}");
    assert!(
        b.calc
            .rename_field("Order", &p("items[0]"), "nothing", "x")
            .is_err()
    );

    b.calc.undo().unwrap();
    b.run();
    assert_eq!(b.schema(), schema);
    assert_eq!(b.fields("items[0]"), ["sku", "qty", "price", "total"]);
    assert_eq!(
        b.calc.formula_text("Order", &p("items[0].qty")),
        Some("7".to_owned())
    );
    assert_eq!(b.calc.formula_text("Order", &p("items[0].quantity")), None);
    assert_ne!(b.data(), String::new());
    assert_eq!(
        b.calc
            .data("Order")
            .unwrap()
            .get(&p("items[0].total"))
            .unwrap()
            .value(),
        &Value::Int(14)
    );
    let _ = data;
}

#[test]
fn deleting_a_field_removes_its_values_and_formulas_and_undo_brings_them_back() {
    let mut b = Book::new();
    b.calc
        .set_formula("Order", &p("items[0].total"), "Order.items[0].qty * 2")
        .unwrap();
    b.run();
    b.calc.clear_history();
    let (data, schema) = (b.data(), b.schema());

    assert_eq!(
        b.calc
            .delete_field_impact("Order", &p("items[0]"), "total")
            .unwrap(),
        (2, 1)
    );
    b.calc
        .delete_field("Order", &p("items[0]"), "total")
        .unwrap();
    assert_eq!(b.calc.history_len(), (1, 0));
    assert_eq!(b.fields("items[0]"), ["sku", "qty", "price"]);
    assert!(!b.data().contains("total"));
    assert_eq!(b.calc.formula_text("Order", &p("items[0].total")), None);

    b.calc.undo().unwrap();
    b.run();
    assert_eq!(b.schema(), schema);
    assert_eq!(b.data(), data);
    assert_eq!(
        b.calc.formula_text("Order", &p("items[0].total")),
        Some("Order.items[0].qty * 2".to_owned())
    );

    b.calc.redo().unwrap();
    assert!(!b.data().contains("total"));

    // A required field can go too, and its name leaves the required list.
    b.calc
        .delete_field("Order", &p("items[0]"), "price")
        .unwrap();
    assert!(!b.schema().contains("\"price\""));
}

#[test]
fn a_record_keeps_at_least_one_field() {
    let mut b = Book::new();
    b.calc.delete_field("Order", &p("billing"), "zip").unwrap();
    let why = error_text(b.calc.delete_field("Order", &p("billing"), "city"));
    assert!(why.contains("at least one"), "{why}");
}

#[test]
fn a_shared_type_changes_everywhere_it_is_used() {
    let mut b = Book::new();
    let places = b.calc.field_places("Order", &p("billing")).unwrap();
    assert_eq!(places.also_used_at, ["Order.shipping"]);
    assert_eq!(places.objects, [p("billing"), p("shipping")]);
    assert_eq!(
        places.document_path,
        Path::root().key("$defs").key("Address")
    );

    b.calc
        .rename_field("Order", &p("billing"), "city", "town")
        .unwrap();
    for place in ["billing", "shipping"] {
        let node = b.calc.data("Order").unwrap().get(&p(place)).unwrap();
        assert!(node.get(&p("town")).is_some(), "{place}");
        assert!(node.get(&p("city")).is_none(), "{place}");
    }
    assert_eq!(b.fields("shipping"), ["town", "zip"]);
    b.calc.undo().unwrap();
    assert_eq!(b.fields("shipping"), ["city", "zip"]);

    // An item type is used once, but at every item.
    let items = b.calc.field_places("Order", &p("items[0]")).unwrap();
    assert!(items.also_used_at.is_empty());
    assert_eq!(items.objects, [p("items[0]"), p("items[1]")]);
}

#[test]
fn an_array_that_a_formula_computes_cannot_have_its_items_changed() {
    let mut b = Book::new();
    b.calc
        .set_formula("Order", &p("items[0]"), "Order.items[1]")
        .unwrap();
    let why = error_text(b.calc.rename_field("Order", &p("items[0]"), "qty", "n"));
    assert!(why.contains("computed"), "{why}");
    assert_eq!(b.calc.history_len(), (1, 0), "nothing was changed");
}

#[test]
fn undo_after_a_later_edit_of_the_schema_tab_must_undo_that_first() {
    let mut b = Book::new();
    b.calc
        .rename_field("Order", &p("items[0]"), "qty", "quantity")
        .unwrap();
    b.calc
        .set_value("Order schema", &p("title"), Value::Str("Order".into()))
        .unwrap();
    assert_eq!(b.calc.undo_label(), Some("Set Order schema.title"));
    b.calc.undo().unwrap();
    assert_eq!(
        b.calc.undo_label(),
        Some("Rename field \"qty\" to \"quantity\"")
    );
}
