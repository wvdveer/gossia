//! Tests of schema tabs, auto-update and the field commands (DESIGN.md,
//! "Schema binding", "Editing the schema from a data tab").

use std::sync::Arc;

use gossia_core::{Path, Value, json, jsonschema};

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
fn the_last_field_of_a_record_can_be_deleted() {
    let mut b = Book::new();
    b.calc.delete_field("Order", &p("billing"), "zip").unwrap();
    b.calc.delete_field("Order", &p("billing"), "city").unwrap();
    assert!(b.fields("billing").is_empty());
    assert!(!b.data().contains("Utrecht"));
    b.calc.undo().unwrap();
    assert_eq!(b.fields("billing"), ["city"]);
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

#[test]
fn an_object_or_an_array_field_is_added_and_takes_fields_of_its_own() {
    let mut b = Book::new();
    b.calc
        .add_field("Order", &p(""), "contact", FieldKind::Object, true)
        .unwrap();
    b.calc
        .add_field("Order", &p(""), "lines", FieldKind::Array, false)
        .unwrap();
    let schema = b.schema();
    assert!(
        schema.contains(r#""contact":{"type":"object","properties":{}}"#),
        "{schema}"
    );
    assert!(
        schema.contains(r#""lines":{"type":"array","items":{"type":"object","properties":{}}}"#),
        "{schema}"
    );
    // An item of the array is an object, which takes fields.
    b.calc
        .set_value("Order", &p("lines"), Value::Array(Vec::new()))
        .unwrap();
    b.calc
        .insert_item("Order", &p("lines"), 0, Value::Object(Default::default()))
        .unwrap();
    b.calc
        .add_field("Order", &p("lines[0]"), "amount", FieldKind::Number, false)
        .unwrap();
    assert_eq!(b.fields("lines[0]"), ["amount"]);
    b.calc
        .set_value("Order", &p("lines[0].amount"), Value::Float(2.5))
        .unwrap();
    assert!(
        b.data().contains(r#""lines":[{"amount":2.5}]"#),
        "{}",
        b.data()
    );
}

#[test]
fn force_object_definition_types_a_field_from_the_records_its_formula_returned() {
    let mut b = Book::new();
    b.calc
        .add_field("Order", &p(""), "copy", FieldKind::Array, false)
        .unwrap();
    b.calc
        .add_field("Order", &p(""), "first", FieldKind::Object, false)
        .unwrap();
    b.calc
        .set_formula("Order", &p("copy"), "Order.items")
        .unwrap();
    b.calc
        .set_formula("Order", &p("first"), "Order.billing")
        .unwrap();
    // A formula that fits is not offered it, nor is a field without one.
    assert!(!b.calc.force_type_offered("Order", &p("items")));
    b.run();
    // The records' fields are not defined, so each item is an empty record
    // that did not fit, and the formula ends in #TYPE!.
    let cell = b.calc.cell("Order", &p("copy")).unwrap();
    let kind = cell.error.map(|e| e.kind);
    assert_eq!(kind, Some(gossia_core::ErrorKind::Type), "{:?}", cell.node);
    assert!(cell.misfit.is_some(), "the tree shows #TYPE! on copy");
    let item = b.calc.cell("Order", &p("copy[0]")).unwrap();
    assert!(item.misfit.is_some(), "and on its items");
    assert!(b.calc.force_type_offered("Order", &p("copy")));
    assert!(b.calc.force_type_offered("Order", &p("first")));

    b.calc.force_type("Order", &p("copy")).unwrap();
    b.calc.force_type("Order", &p("first")).unwrap();
    b.run();
    assert_eq!(
        b.fields("copy[0]"),
        names(&["sku", "qty", "price", "total"])
    );
    assert_eq!(b.fields("first"), names(&["city", "zip"]));
    let copy = b.calc.cell("Order", &p("copy")).unwrap();
    assert!(
        matches!(copy.node.map(|n| n.value()), Some(Value::Array(items)) if items.len() == 2),
        "{:?} pending {}",
        copy.node,
        b.calc.pending()
    );
    assert!(!b.calc.force_type_offered("Order", &p("copy")));

    // One entry each in the history.
    b.calc.undo().unwrap();
    b.calc.undo().unwrap();
    b.run();
    assert!(b.calc.force_type_offered("Order", &p("copy")));
    assert!(
        !b.schema()
            .contains(r#""copy":{"type":"array","items":{"type":"object","properties":{"sku""#)
    );
}

/// A response shaped as randomuser.me's: an array of nested records, with a
/// null among them, and a record beside it. Defined from the root, the
/// result fits and is kept.
#[test]
fn force_object_definition_on_the_root_takes_nested_records_and_arrays() {
    let mut b = Book::new();
    let empty = r#"{"type": "object", "properties": {}}"#;
    b.calc
        .add_sheet(
            "Data",
            jsonschema::lower_str(empty).unwrap(),
            json::parse("{}").unwrap().node,
        )
        .unwrap();
    b.calc
        .add_schema_sheet("Data schema", json::parse(empty).unwrap().node)
        .unwrap();
    b.calc.bind_schema("Data", "Data schema").unwrap();
    b.calc.set_auto_update("Data schema", true).unwrap();
    // The response, in a tab of its own with the schema inferred from it.
    let response = json::parse(
        r#"{"results": [{"gender": "female", "name": {"title": "Ms", "first": "Ann"},
             "id": {"name": "", "value": null}, "dob": {"age": 41},
             "location": {"city": "Natal", "postcode": 73568}}],
            "info": {"seed": "abc", "results": 1, "page": 1, "version": "1.4"}}"#,
    )
    .unwrap()
    .node;
    let inferred = gossia_core::infer::infer_json(&response).schema;
    let inferred = json::to_string(&inferred, false).unwrap();
    b.calc
        .add_sheet("Api", jsonschema::lower_str(&inferred).unwrap(), response)
        .unwrap();
    b.calc.set_formula("Data", &p(""), "Api").unwrap();
    b.run();
    assert!(b.calc.force_type_offered("Data", &p("")));
    b.calc.force_type("Data", &p("")).unwrap();
    b.run();
    // id.value is null: the inferred type makes it optional, and the null in
    // the result is that field left absent, so the result fits.
    let cell = b.calc.cell("Data", &p("")).map(|c| format!("{:?}", c.node));
    assert!(!format!("{cell:?}").contains("Error"), "{cell:?}");
    let data = json::to_string(b.calc.data("Data").unwrap(), false).unwrap();
    assert!(data.contains(r#""id":{"name":""}"#), "{data}");
    assert!(data.contains(r#""version":"1.4""#), "{data}");
    assert_eq!(
        b.calc
            .field_places("Data", &p("results[0]"))
            .unwrap()
            .fields,
        names(&["gender", "name", "id", "dob", "location"])
    );

    // The next answer is another person, without `dob`, with an `id.value`,
    // and without `info`: every field found was made optional, so it fits.
    let next = json::parse(
        r#"{"results": [{"gender": "male", "name": {"title": "Mr", "first": "Bo"},
             "id": {"name": "DNI", "value": "07247621-R"}}]}"#,
    )
    .unwrap()
    .node;
    let inferred = gossia_core::infer::infer_json(&next).schema;
    let inferred = json::to_string(&inferred, false).unwrap();
    b.calc
        .add_sheet("Next", jsonschema::lower_str(&inferred).unwrap(), next)
        .unwrap();
    b.calc.set_formula("Data", &p(""), "Next").unwrap();
    b.run();
    let cell = b.calc.cell("Data", &p("")).map(|c| format!("{:?}", c.node));
    assert!(!format!("{cell:?}").contains("Error"), "{cell:?}");
    let data = json::to_string(b.calc.data("Data").unwrap(), false).unwrap();
    assert!(data.contains(r#""value":"07247621-R""#), "{data}");
    let schema = json::to_string(b.calc.data("Data schema").unwrap(), false).unwrap();
    assert!(!schema.contains("required"), "{schema}");

    // A postcode that is text where it was a number: the formula ends in
    // #TYPE!, the tree shows it on the way down to the postcode, and every
    // other field keeps its value.
    let third = json::parse(
        r#"{"results": [{"gender": "female", "name": {"title": "Ms", "first": "Cy"},
             "location": {"city": "Leeds", "postcode": "Z26 7XT"}}]}"#,
    )
    .unwrap()
    .node;
    let inferred = gossia_core::infer::infer_json(&third).schema;
    let inferred = json::to_string(&inferred, false).unwrap();
    b.calc
        .add_sheet("Third", jsonschema::lower_str(&inferred).unwrap(), third)
        .unwrap();
    b.calc.set_formula("Data", &p(""), "Third").unwrap();
    b.run();
    let root = b.calc.cell("Data", &p("")).unwrap();
    let message = root.error.map(|e| e.message.clone()).unwrap_or_default();
    assert!(
        message.contains("results[0].location.postcode"),
        "{message}"
    );
    for path in [
        "",
        "results",
        "results[0]",
        "results[0].location",
        "results[0].location.postcode",
    ] {
        let cell = b.calc.cell("Data", &p(path)).unwrap();
        let shown =
            cell.misfit.is_some() || matches!(cell.node.map(|n| n.value()), Some(Value::Error(_)));
        assert!(shown, "#TYPE! at {path:?}");
    }
    for (path, value) in [
        ("results[0].name.first", "Cy"),
        ("results[0].location.city", "Leeds"),
        ("results[0].gender", "female"),
    ] {
        let cell = b.calc.cell("Data", &p(path)).unwrap();
        assert!(cell.misfit.is_none(), "no #TYPE! at {path}");
        assert_eq!(
            cell.node.map(|n| n.value().clone()),
            Some(Value::Str(value.into())),
            "{path}"
        );
    }
    assert!(
        b.calc
            .cell("Data", &p("results[0].name"))
            .unwrap()
            .misfit
            .is_none()
    );
}

#[test]
fn force_object_definition_on_the_root_gives_its_record_the_fields_returned() {
    let mut b = Book::new();
    let empty = r#"{"type": "object", "properties": {}}"#;
    b.calc
        .add_sheet(
            "Copy",
            jsonschema::lower_str(empty).unwrap(),
            json::parse("{}").unwrap().node,
        )
        .unwrap();
    b.calc
        .add_schema_sheet("Copy schema", json::parse(empty).unwrap().node)
        .unwrap();
    b.calc.bind_schema("Copy", "Copy schema").unwrap();
    b.calc.set_auto_update("Copy schema", true).unwrap();
    b.calc.set_formula("Copy", &p(""), "Order.billing").unwrap();
    b.run();
    assert!(b.calc.force_type_offered("Copy", &p("")));

    b.calc.force_type("Copy", &p("")).unwrap();
    b.run();
    let fields = b.calc.field_places("Copy", &p("")).unwrap().fields;
    assert_eq!(fields, names(&["city", "zip"]));
    let data = json::to_string(b.calc.data("Copy").unwrap(), false).unwrap();
    assert!(data.contains("Utrecht"), "{data}");
    assert!(!b.calc.force_type_offered("Copy", &p("")));

    b.calc.undo().unwrap();
    assert!(
        b.calc
            .field_places("Copy", &p(""))
            .unwrap()
            .fields
            .is_empty()
    );
}

/// *Go to schema definition*: a field is its entry under its record's
/// `properties`, in the record's own place (the `$defs` of a shared type);
/// a node the schema does not define goes to its record.
#[test]
fn a_field_is_defined_by_its_entry_in_the_schema_tab() {
    let b = Book::new();
    let at = |path: &str| b.calc.definition_of("Order", &p(path));
    // `$defs` is not a name a path can be written with, so the keys are
    // given one by one.
    let place = |text: &str| {
        let path = text
            .split('.')
            .filter(|k| !k.is_empty())
            .fold(Path::root(), |path, key| path.key(key));
        Some(("Order schema".to_owned(), path))
    };
    assert_eq!(at(""), place(""));
    assert_eq!(at("items"), place("properties.items"));
    assert_eq!(at("items[1]"), place("properties.items.items"));
    assert_eq!(
        at("items[1].qty"),
        place("properties.items.items.properties.qty")
    );
    assert_eq!(at("billing"), place("properties.billing"));
    assert_eq!(at("billing.city"), place("$defs.Address.properties.city"));
    assert_eq!(at("billing.nothing"), place("properties.billing"));
    assert_eq!(b.calc.definition_of("Order schema", &p("")), None);
}

/// Reported by the owner (2026-11-10): an array field was always an array of
/// objects. Its items can be of a simple type.
#[test]
fn an_array_field_can_hold_items_of_a_simple_type() {
    let mut b = Book::new();
    b.calc
        .add_array_field("Order", &p(""), "tags", FieldKind::Text, false)
        .unwrap();
    assert!(
        b.schema()
            .contains(r#""tags":{"type":"array","items":{"type":"string"}}"#),
        "{}",
        b.schema()
    );
    let spec = b.calc.spec("Order").unwrap();
    assert_eq!(spec.describe(spec.resolve(&p("tags")).unwrap().ty), "array");
    b.calc
        .set_value("Order", &p("tags"), Value::Array(Vec::new()))
        .unwrap();
    b.calc
        .insert_item("Order", &p("tags"), 0, Value::Str("red".into()))
        .unwrap();
    assert!(b.data().contains(r#""tags":["red"]"#), "{}", b.data());
    assert!(
        error_text(
            b.calc
                .add_array_field("Order", &p(""), "grid", FieldKind::Array, false)
        )
        .contains("cannot be arrays")
    );
}

/// Reported by the owner (2026-11-10): changing the items' `type` on the
/// schema tab left `properties` behind, so the schema did not lower and the
/// tab still showed an array of objects. The keywords the new type cannot
/// have go with the change, in one entry.
#[test]
fn changing_a_type_on_the_schema_tab_drops_the_keywords_it_cannot_have() {
    let mut b = Book::new();
    b.calc
        .add_field("Order", &p(""), "tags", FieldKind::Array, false)
        .unwrap();
    b.calc
        .set_value(
            "Order schema",
            &p("properties.tags.items.type"),
            Value::Str("string".into()),
        )
        .unwrap();
    assert_eq!(b.calc.schema_error("Order schema"), None);
    assert!(
        b.schema()
            .contains(r#""tags":{"type":"array","items":{"type":"string"}}"#),
        "{}",
        b.schema()
    );
    let spec = b.calc.spec("Order").unwrap();
    assert_eq!(spec.describe(spec.resolve(&p("tags")).unwrap().ty), "array");
    // One undo brings back both the type and its fields.
    b.calc.undo().unwrap();
    assert!(
        b.schema()
            .contains(r#""tags":{"type":"array","items":{"type":"object","properties":{}}}"#),
        "{}",
        b.schema()
    );
    // A keyword the new type has stays: `required` on an object kept as one.
    b.calc
        .set_value(
            "Order schema",
            &p("properties.items.items.type"),
            Value::Str("object".into()),
        )
        .unwrap();
    assert!(b.schema().contains(r#""required":["sku","qty","price"]"#));
}

/// Reported by the owner (2026-11-10): an array formula giving text was not
/// offered *Force type definition*, so its items stayed empty records.
#[test]
fn force_type_definition_types_an_array_of_simple_values() {
    let mut b = Book::new();
    b.calc
        .add_array_field("Order", &p(""), "names", FieldKind::Text, false)
        .unwrap();
    b.calc
        .set_value(
            "Order",
            &p("names"),
            Value::Array(vec![
                gossia_core::Node::new(Value::Str("a".into())),
                gossia_core::Node::new(Value::Str("b".into())),
            ]),
        )
        .unwrap();
    b.calc
        .add_field("Order", &p(""), "copy", FieldKind::Array, false)
        .unwrap();
    b.calc
        .set_formula("Order", &p("copy"), "Order.names")
        .unwrap();
    b.run();
    assert!(b.calc.force_type_offered("Order", &p("copy")));
    b.calc.force_type("Order", &p("copy")).unwrap();
    b.run();
    assert!(
        b.schema()
            .contains(r#""copy":{"type":"array","items":{"type":"string"}}"#),
        "{}",
        b.schema()
    );
    let copy = b.calc.cell("Order", &p("copy")).unwrap();
    assert!(
        copy.error.is_none() && copy.misfit.is_none(),
        "{:?}",
        copy.node
    );
    assert!(!b.calc.force_type_offered("Order", &p("copy")));
}

/// Owner, 2026-11-10: a node of a simple type whose formula returned the
/// wrong kind keeps that result, for the formula bar to show in red.
#[test]
fn a_simple_node_keeps_the_result_it_refused() {
    let mut b = Book::new();
    b.calc
        .add_field("Order", &p(""), "count", FieldKind::Integer, false)
        .unwrap();
    b.calc
        .set_formula("Order", &p("count"), "'twelve'")
        .unwrap();
    b.calc
        .add_field("Order", &p(""), "info", FieldKind::Object, false)
        .unwrap();
    b.calc.set_formula("Order", &p("info"), "12").unwrap();
    b.run();
    assert_eq!(
        b.calc.refused_simple_value("Order", &p("count")),
        Some(&Value::Str("twelve".into()))
    );
    // A record is not a simple type, and a result that fits is not kept.
    assert_eq!(b.calc.refused_simple_value("Order", &p("info")), None);
    b.calc.set_formula("Order", &p("count"), "12").unwrap();
    b.run();
    assert_eq!(b.calc.refused_simple_value("Order", &p("count")), None);
}

/// Owner, 2026-11-10: inside a formula's result, a part of a simple type
/// that did not fit is kept too, for the Formula column to show in red.
#[test]
fn a_misfit_inside_a_result_keeps_what_it_was() {
    let mut b = Book::new();
    b.calc
        .add_field("Order", &p(""), "info", FieldKind::Object, false)
        .unwrap();
    b.calc
        .add_field("Order", &p("info"), "n", FieldKind::Integer, false)
        .unwrap();
    b.calc
        .add_field("Order", &p("info"), "label", FieldKind::Text, false)
        .unwrap();
    // The same fields, both text, copied by a formula.
    b.calc
        .add_field("Order", &p(""), "src", FieldKind::Object, false)
        .unwrap();
    for name in ["n", "label"] {
        b.calc
            .add_field("Order", &p("src"), name, FieldKind::Text, false)
            .unwrap();
    }
    let mut fields = indexmap::IndexMap::new();
    for (name, text) in [("n", "1234 AB"), ("label", "ok")] {
        fields.insert(
            name.to_owned(),
            gossia_core::Node::new(Value::Str(text.into())),
        );
    }
    b.calc
        .set_value("Order", &p("src"), Value::Object(fields))
        .unwrap();
    b.calc
        .set_formula("Order", &p("info"), "Order.src")
        .unwrap();
    b.run();
    assert_eq!(
        b.calc.refused_simple_value("Order", &p("info.n")),
        Some(&Value::Str("1234 AB".into()))
    );
    // The part that fitted, and the record on the way, show nothing.
    assert_eq!(b.calc.refused_simple_value("Order", &p("info.label")), None);
    assert_eq!(b.calc.refused_simple_value("Order", &p("info")), None);
}

/// A tab bound to a schema tab with auto-update on, for the commands that
/// change the schema from the data.
fn bound_with_auto_update(schema: &str, data: &str) -> Calc {
    let mut calc = Calc::new(Arc::new(FakeEngine::new()));
    calc.add_sheet(
        "Data",
        jsonschema::lower_str(schema).unwrap(),
        json::parse(data).unwrap().node,
    )
    .unwrap();
    calc.add_schema_sheet("DataSchema", json::parse(schema).unwrap().node)
        .unwrap();
    calc.bind_schema("Data", "DataSchema").unwrap();
    calc.set_auto_update("DataSchema", true).unwrap();
    calc.clear_history();
    calc
}

#[test]
fn force_type_definition_defines_an_array_of_any_values_from_its_items() {
    // Owner, 2026-11-14 (issue 3): an array of any values can still be given
    // the type of what it holds.
    let mut calc = bound_with_auto_update(
        r#"{"type": "array"}"#,
        r#"[{"name": "Ann", "age": 41}, {"name": "Bo"}]"#,
    );
    let schema = |calc: &Calc| json::to_string(calc.data("DataSchema").unwrap(), false).unwrap();
    // On the array, and on any one of its items, which means the same.
    assert!(calc.force_type_offered("Data", &p("")));
    assert!(calc.force_type_offered("Data", &p("[1]")));
    assert!(calc.force_type_of_any("Data", &p("")));
    calc.force_type("Data", &p("[1]")).unwrap();
    let after = schema(&calc);
    assert!(after.starts_with(r#"{"type":"array","items":{"#), "{after}");
    assert!(after.contains(r#""name":{"type":"string"}"#), "{after}");
    assert!(after.contains(r#""age":{"type":"integer"}"#), "{after}");
    assert_eq!(calc.schema_error("DataSchema"), None);
    let spec = calc.spec("Data").unwrap();
    let gossia_core::Kind::Array { items, .. } = spec.get(spec.root()).kind else {
        panic!("the root is a list");
    };
    assert!(matches!(
        spec.get(items).kind,
        gossia_core::Kind::Object { .. }
    ));
    // Defined now, so not offered again; one undo brings the `any` back.
    assert!(!calc.force_type_offered("Data", &p("")));
    calc.undo().unwrap();
    assert_eq!(schema(&calc), r#"{"type":"array"}"#);

    // An empty list holds nothing to define its items from.
    let empty = bound_with_auto_update(r#"{"type": "array"}"#, "[]");
    assert!(!empty.force_type_offered("Data", &p("")));
}

#[test]
fn force_type_definition_defines_a_field_of_any_value_and_keeps_its_description() {
    let mut calc = bound_with_auto_update(
        r#"{"type": "object", "properties": {"meta": {"description": "Anything"}}}"#,
        r#"{"meta": {"tags": ["a", "b"]}}"#,
    );
    assert!(calc.force_type_offered("Data", &p("meta")));
    calc.force_type("Data", &p("meta")).unwrap();
    let after = json::to_string(calc.data("DataSchema").unwrap(), false).unwrap();
    assert!(
        after.contains(r#""meta":{"description":"Anything","#),
        "{after}"
    );
    assert!(after.contains(r#""tags":{"#), "{after}");
    assert_eq!(calc.schema_error("DataSchema"), None);
    let spec = calc.spec("Data").unwrap();
    let meta = spec.resolve(&p("meta")).unwrap().ty;
    assert!(matches!(
        spec.get(meta).kind,
        gossia_core::Kind::Object { .. }
    ));
    let tags = spec.resolve(&p("meta.tags")).unwrap().ty;
    let gossia_core::Kind::Array { items, .. } = spec.get(tags).kind else {
        panic!("tags is a list");
    };
    assert!(matches!(spec.get(items).kind, gossia_core::Kind::String(_)));

    // Greyed, with the reason, when the schema tab cannot be changed.
    let mut off = bound_with_auto_update(
        r#"{"type": "object", "properties": {"meta": {}}}"#,
        r#"{"meta": 3}"#,
    );
    off.set_auto_update("DataSchema", false).unwrap();
    assert!(off.force_type_offered("Data", &p("meta")));
    assert!(off.force_any_schema_tab("Data").is_err());
    assert!(off.force_type("Data", &p("meta")).is_err());
}

#[test]
fn a_value_typed_under_an_absent_record_of_the_schema_creates_it() {
    // A new workbook's schema, made a list on its schema tab: the type, then
    // the type of the items, which are not there yet (issue 3).
    let engine = FakeEngine::new();
    let mut calc = Calc::new(Arc::new(engine.clone()));
    let blank = r#"{"type": "object", "properties": {}}"#;
    calc.add_sheet(
        "Data",
        jsonschema::lower_str(blank).unwrap(),
        json::parse("{}").unwrap().node,
    )
    .unwrap();
    calc.add_schema_sheet("DataSchema", json::parse(blank).unwrap().node)
        .unwrap();
    calc.bind_schema("Data", "DataSchema").unwrap();
    calc.clear_history();
    let schema = |calc: &Calc| json::to_string(calc.data("DataSchema").unwrap(), false).unwrap();
    let root_kind = |calc: &Calc| {
        let spec = calc.spec("Data").unwrap();
        format!("{:?}", spec.get(spec.root()).kind)
    };

    calc.set_value("DataSchema", &p("type"), Value::Str("array".into()))
        .unwrap();
    // A list with no `items` yet is a list of any values, not an error.
    assert_eq!(calc.schema_error("DataSchema"), None);
    assert!(root_kind(&calc).starts_with("Array"));
    calc.set_value("DataSchema", &p("items.type"), Value::Str("string".into()))
        .unwrap();
    assert_eq!(
        schema(&calc),
        r#"{"type":"array","items":{"type":"string"}}"#
    );
    assert_eq!(calc.schema_error("DataSchema"), None);
    assert!(root_kind(&calc).starts_with("Array"));

    // An array of arrays of text: two records down, both created at once.
    calc.remove("DataSchema", &p("items")).unwrap();
    calc.set_value(
        "DataSchema",
        &p("items.items.type"),
        Value::Str("string".into()),
    )
    .unwrap();
    calc.set_value("DataSchema", &p("items.type"), Value::Str("array".into()))
        .unwrap();
    assert_eq!(
        schema(&calc),
        r#"{"type":"array","items":{"items":{"type":"string"},"type":"array"}}"#
    );
    assert_eq!(calc.schema_error("DataSchema"), None);
    // Two undos: the first takes back `items.type`, the second both records
    // the value was put in, at once.
    calc.undo().unwrap();
    calc.undo().unwrap();
    assert_eq!(schema(&calc), r#"{"type":"array"}"#);

    // An array of any value: `items` made present and empty.
    calc.set_value("DataSchema", &p("items"), Value::Object(Default::default()))
        .unwrap();
    assert_eq!(schema(&calc), r#"{"type":"array","items":{}}"#);
    assert_eq!(calc.schema_error("DataSchema"), None);
    let spec = calc.spec("Data").unwrap();
    let gossia_core::Kind::Array { items, .. } = spec.get(spec.root()).kind else {
        panic!("the root is a list");
    };
    assert!(matches!(spec.get(items).kind, gossia_core::Kind::Any));
}
