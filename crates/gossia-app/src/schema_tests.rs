//! Tests of changing the schema from a data tab, without a window.

use std::time::Duration;

use gossia_core::{Path, Value};

use crate::GossiaApp;
use crate::schema::SchemaCommand;
use crate::table::Filter;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

fn app() -> GossiaApp {
    let mut app = GossiaApp::sample();
    settle(&mut app);
    app
}

/// The sample with auto-update on for the schema of Orders.
fn updatable() -> GossiaApp {
    let mut app = app();
    app.set_auto_update("Orders_schema", true);
    assert!(app.notice.is_none(), "{:?}", app.notice);
    app
}

fn settle(app: &mut GossiaApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app.refresh_rows();
    app.refresh_table();
}

fn fields(app: &GossiaApp, object: &str) -> Vec<String> {
    app.calc()
        .field_places("Orders", &p(object))
        .unwrap()
        .fields
}

fn rename(from: &str, to: &str) -> SchemaCommand {
    SchemaCommand::Rename {
        from: from.into(),
        to: to.into(),
    }
}

#[test]
fn the_sample_has_a_schema_tab_for_each_tab_and_auto_update_is_off() {
    let app = app();
    let names: Vec<_> = app.calc().sheet_names().collect();
    assert_eq!(
        names,
        ["Orders", "Prices", "Orders_schema", "Prices_schema"]
    );
    assert_eq!(app.calc().schema_of("Orders"), Some("Orders_schema"));
    assert!(!app.calc().auto_update("Orders_schema"));
}

#[test]
fn a_command_is_unavailable_until_auto_update_is_on_and_the_reason_names_the_tab() {
    let mut app = app();
    let why = app.schema_edit_allowed(&p("items[0]")).unwrap_err();
    assert!(why.contains("auto-update on Orders_schema"), "{why}");
    app.start_schema_command(&p("items[0]"), rename("qty", "n"));
    assert!(app.schema_dialog.is_none());
    assert!(app.notice.as_deref().unwrap().contains("Orders_schema"));

    app.set_auto_update("Orders_schema", true);
    assert!(app.schema_edit_allowed(&p("items[0]")).is_ok());
    // Undoing the switch is an ordinary undo.
    app.undo();
    assert!(!app.calc().auto_update("Orders_schema"));
}

#[test]
fn rows_and_columns_say_which_record_and_field_they_are() {
    let app = app();
    assert_eq!(app.object_of_row(&p("customer")), Some(p("customer")));
    assert_eq!(app.object_of_row(&p("items")), Some(p("items[0]")));
    assert_eq!(app.object_of_row(&p("tax_rate")), None);
    assert_eq!(
        app.field_of(&p("items[1].qty")),
        Some((p("items[1]"), "qty".to_owned()))
    );
    assert_eq!(app.field_of(&p("items[1]")), None, "an item is not a field");
    assert_eq!(app.field_of(&Path::root()), None);
    // The keys of a map are data, not fields.
    assert_eq!(app.field_of(&p("tax_rate")).unwrap().0, Path::root());
}

#[test]
fn renaming_asks_for_the_name_then_moves_the_selection_the_open_nodes_and_the_view_state() {
    let mut app = updatable();
    app.set_tabular(&p("items"), true);
    app.hide_column(&p("items"), "qty");
    app.set_filter(
        &p("items"),
        "qty",
        Some(Filter {
            condition: Some(crate::table::Condition::Greater("1".into())),
            ..Filter::default()
        }),
    );
    settle(&mut app);
    app.select(&p("items[0].qty"));

    app.start_schema_command(&p("items[0]"), rename("qty", "qty"));
    assert!(app.schema_dialog.is_some(), "a rename asks for the name");
    // The name is what the user types.
    if let Some(SchemaCommand::Rename { to, .. }) =
        app.schema_dialog.as_mut().map(|d| &mut d.command)
    {
        *to = "price".into();
    }
    app.apply_schema_dialog();
    let error = app.schema_dialog.as_ref().unwrap().error.clone().unwrap();
    assert!(error.contains("already"), "{error}");

    if let Some(SchemaCommand::Rename { to, .. }) =
        app.schema_dialog.as_mut().map(|d| &mut d.command)
    {
        *to = "quantity".into();
    }
    app.apply_schema_dialog();
    assert!(app.schema_dialog.is_none());
    settle(&mut app);

    assert_eq!(
        fields(&app, "items[0]"),
        ["sku", "quantity", "price", "total", "note"]
    );
    assert_eq!(app.selection, Some(p("items[0].quantity")));
    assert_eq!(
        app.calc().undo_label(),
        Some("Rename field \"qty\" to \"quantity\"")
    );
    // What was set on the column is set on it under its new name.
    let view = app.view_of(&p("items")).unwrap();
    assert!(view.hidden_columns.contains("quantity"));
    assert!(view.filters.contains_key("quantity"));
    assert!(!view.hidden_columns.contains("qty"));

    app.undo();
    settle(&mut app);
    assert_eq!(
        fields(&app, "items[0]"),
        ["sku", "qty", "price", "total", "note"]
    );
}

#[test]
fn adding_a_field_checks_the_name_in_the_dialog_and_shows_a_new_column() {
    let mut app = updatable();
    app.set_tabular(&p("items"), true);
    settle(&mut app);
    let add = |name: &str| SchemaCommand::Add {
        name: name.into(),
        kind: gossia_calc::FieldKind::Integer,
        required: false,
        attribute: false,
    };
    app.start_schema_command(&p("items[0]"), add(""));
    app.apply_schema_dialog();
    assert!(
        app.schema_dialog.as_ref().unwrap().error.is_some(),
        "needs a name"
    );
    app.schema_dialog = None;

    app.start_schema_command(&p("items[0]"), add("weight"));
    app.apply_schema_dialog();
    assert!(app.schema_dialog.is_none());
    settle(&mut app);
    let names: Vec<_> = app
        .layout()
        .unwrap()
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["sku", "qty", "price", "total", "note", "weight"]);
}

#[test]
fn deleting_a_field_confirms_with_the_counts_and_selection_falls_back_to_the_record() {
    let mut app = updatable();
    app.select(&p("items[1].note"));
    app.start_schema_command(
        &p("items[0]"),
        SchemaCommand::Delete {
            name: "total".into(),
        },
    );
    let dialog = app.schema_dialog.as_ref().unwrap();
    assert_eq!((dialog.values, dialog.formulas), (3, 3));
    assert!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("items[0].total"))
            .is_some()
    );
    app.apply_schema_dialog();
    settle(&mut app);
    assert!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("items[0].total"))
            .is_none()
    );
    assert_eq!(fields(&app, "items[0]"), ["sku", "qty", "price", "note"]);
    // The switch, and the deletion as one entry.
    assert_eq!(app.calc().history_len().0, 2);

    app.select(&p("items[1].note"));
    app.start_schema_command(
        &p("items[0]"),
        SchemaCommand::Delete {
            name: "note".into(),
        },
    );
    app.apply_schema_dialog();
    assert_eq!(app.selection, Some(p("items[1]")));

    app.undo();
    app.undo();
    settle(&mut app);
    assert_eq!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("items[2].total"))
            .unwrap()
            .value(),
        &Value::Float(9.0)
    );
}

#[test]
fn moving_a_field_reorders_without_asking_when_the_type_is_not_shared() {
    let mut app = updatable();
    app.set_tabular(&p("items"), true);
    settle(&mut app);
    app.move_field(&p("items[0]"), "note", "qty");
    assert!(app.schema_dialog.is_none(), "applied at once");
    settle(&mut app);
    assert_eq!(
        fields(&app, "items[0]"),
        ["sku", "note", "qty", "price", "total"]
    );
    let names: Vec<_> = app
        .layout()
        .unwrap()
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["sku", "note", "qty", "price", "total"]);
    assert_eq!(
        app.calc().undo_label(),
        Some("Reorder fields of Orders.items[]")
    );

    // Dropping a field on itself is nothing.
    let entries = app.calc().history_len();
    app.move_field(&p("items[0]"), "qty", "qty");
    assert_eq!(app.calc().history_len(), entries);
}

#[test]
fn a_column_knows_its_record_and_field() {
    let mut app = updatable();
    app.set_tabular(&p("items"), true);
    settle(&mut app);
    let layout = app.layout().unwrap();
    let column = layout.columns.iter().find(|c| c.name == "price").unwrap();
    assert_eq!(
        app.column_owner(&p("items"), column),
        Some((p("items[0]"), "price".to_owned()))
    );
}

#[test]
fn on_an_xml_tab_a_new_field_is_an_element_or_an_attribute() {
    let mut app = app();
    let text = r#"<catalog><book id="b1"><title>Dune</title></book><book id="b2"><title>Emma</title></book></catalog>"#;
    let plan = app.plan_import("catalog.xml", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    app.switch_to("catalog");
    app.set_auto_update("catalogSchema", true);
    assert!(app.notice.is_none(), "{:?}", app.notice);
    settle(&mut app);
    let book_fields = |app: &GossiaApp| {
        app.calc()
            .field_places("catalog", &p("book[0]"))
            .unwrap()
            .fields
    };
    assert_eq!(book_fields(&app), ["@id", "title"]);

    let add = |name: &str, attribute: bool| SchemaCommand::Add {
        name: name.into(),
        kind: gossia_calc::FieldKind::Integer,
        required: false,
        attribute,
    };
    app.start_schema_command(&p("book[0]"), add("year", false));
    assert!(app.schema_dialog.as_ref().unwrap().xml);
    app.apply_schema_dialog();
    assert!(app.schema_dialog.is_none(), "{:?}", app.schema_dialog);
    // An attribute's name is typed without its @.
    app.start_schema_command(&p("book[1]"), add("pages", true));
    app.apply_schema_dialog();
    assert!(app.schema_dialog.is_none(), "{:?}", app.schema_dialog);
    assert_eq!(book_fields(&app), ["@id", "@pages", "title", "year"]);

    // Renaming an attribute keeps it one.
    app.start_schema_command(&p("book[0]"), rename("@id", "code"));
    app.apply_schema_dialog();
    assert!(app.schema_dialog.is_none(), "{:?}", app.schema_dialog);
    settle(&mut app);
    assert_eq!(book_fields(&app), ["@code", "@pages", "title", "year"]);
    assert_eq!(
        app.calc()
            .data("catalog")
            .unwrap()
            .get(&p("book[1][\"@code\"]"))
            .map(|n| n.value().clone()),
        Some(Value::Str("b2".into()))
    );
}
