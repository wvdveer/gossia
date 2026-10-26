//! Tests of adding, deleting and rebinding tabs, and of importing files
//! (DESIGN.md, "Managing tabs" and "Import and export").

use std::path::PathBuf;
use std::time::Duration;

use cliotree_core::validate::validate;
use cliotree_core::{Path, Value, json};
use serde_json::json;

use crate::ClioApp;
use crate::control;
use crate::import::{ImportFormat, SchemaChoice, format_of, name_from_file};
use crate::tab_commands::NewSchema;
use crate::workbook_file::new_workbook_id;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn settle(app: &mut ClioApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

fn sample() -> ClioApp {
    let mut app = ClioApp::sample();
    app.files.browse = false;
    settle(&mut app);
    app
}

fn tabs(app: &ClioApp) -> Vec<&str> {
    app.calc().sheet_names().collect()
}

fn valid(app: &ClioApp, tab: &str) -> bool {
    let calc = app.calc();
    validate(calc.spec(tab).unwrap(), calc.data(tab).unwrap()).is_empty()
}

/// A folder of its own for the files of one test.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cliotree-{test}-{}-{}",
        std::process::id(),
        new_workbook_id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

// ------------------------------------------------------------- new tabs

#[test]
fn a_new_tab_with_its_own_schema_is_one_entry() {
    let mut app = sample();
    app.begin_new_tab();
    let dialog = app.tab_dialogs.new.clone().unwrap();
    assert_eq!(
        (dialog.name.as_str(), dialog.schema_name.as_str()),
        ("Data", "DataSchema")
    );
    app.create_tab(&dialog).unwrap();
    assert_eq!(
        tabs(&app),
        [
            "Orders",
            "Prices",
            "Orders_schema",
            "Prices_schema",
            "DataSchema",
            "Data"
        ]
    );
    assert_eq!(app.sheet, "Data", "the new tab is shown");
    assert!(
        app.calc().auto_update("DataSchema"),
        "fields are added from Data"
    );
    assert_eq!(app.calc().undo_label(), Some("New tab Data"));

    app.undo();
    assert_eq!(tabs(&app).len(), 4);
    assert_eq!(app.sheet, "Prices_schema", "the last tab is shown instead");
    app.redo();
    assert_eq!(tabs(&app).len(), 6);
}

#[test]
fn a_new_tab_can_use_a_schema_that_is_there() {
    let mut app = sample();
    app.begin_new_tab();
    let mut dialog = app.tab_dialogs.new.clone().unwrap();
    dialog.name = "Order2".to_owned();
    dialog.schema = NewSchema::Existing("Orders_schema".to_owned());
    app.create_tab(&dialog).unwrap();
    assert_eq!(app.calc().schema_of("Order2"), Some("Orders_schema"));
    assert!(valid(&app, "Order2"), "the required fields are filled in");
    assert_eq!(
        app.calc().schema_users("Orders_schema"),
        ["Orders", "Order2"]
    );

    dialog.name = "orders".to_owned();
    assert!(
        app.new_tab_problem(&dialog)
            .unwrap()
            .contains("exists already")
    );
}

// ------------------------------------------------------------ deleting

#[test]
fn deleting_the_current_tab_shows_the_next_and_undo_puts_it_back() {
    let mut app = sample();
    app.set_tabular(&p("items"), true);
    let refused = app.delete_tab("Orders_schema").unwrap_err();
    assert!(refused.contains("Orders uses this schema"), "{refused}");

    app.delete_tab("Orders").unwrap();
    assert_eq!(tabs(&app), ["Prices", "Orders_schema", "Prices_schema"]);
    assert_eq!(app.sheet, "Prices");
    assert!(
        app.tables.tabular.is_empty(),
        "what the window kept is gone"
    );
    assert_eq!(app.calc().undo_label(), Some("Delete tab Orders"));
    assert_eq!(app.calc().delete_problem("Orders_schema"), None);

    app.undo();
    settle(&mut app);
    assert_eq!(tabs(&app)[0], "Orders");
    assert!(matches!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("total"))
            .unwrap()
            .value(),
        Value::Float(_)
    ));
}

#[test]
fn the_delete_dialog_warns_about_formulas_that_use_the_tab() {
    let app = sample();
    let mentions = app.calc().mentions("Prices");
    let text = crate::tab_commands::delete_mentions_text("Prices", mentions);
    match mentions.formulas {
        0 => assert_eq!(text, None),
        _ => assert!(text.unwrap().contains("#NAME! until the tab is back")),
    }
    assert_eq!(
        crate::tab_commands::delete_mentions_text(
            "X",
            cliotree_calc::Mentions {
                formulas: 2,
                modules: 1
            }
        )
        .as_deref(),
        Some("2 formulas and 1 module use X; they will show #NAME! until the tab is back.")
    );
}

// ----------------------------------------------------------- rebinding

#[test]
fn a_tab_is_given_another_schema() {
    let mut app = sample();
    app.switch_to("Prices");
    app.bind_tab("Prices", "Orders_schema").unwrap();
    assert_eq!(app.calc().schema_of("Prices"), Some("Orders_schema"));
    assert!(!valid(&app, "Prices"));
    assert_eq!(
        app.calc().undo_label(),
        Some("Bind Prices to Orders_schema")
    );
    app.undo();
    assert_eq!(app.calc().schema_of("Prices"), Some("Prices_schema"));
    assert!(valid(&app, "Prices"));
}

// ----------------------------------------------------------- importing

#[test]
fn names_are_made_from_the_file_name() {
    let free = |_: &str| false;
    assert_eq!(name_from_file("my-file.json", free), "my_file");
    assert_eq!(name_from_file("2024 orders.xml", free), "Data2024_orders");
    assert_eq!(name_from_file("_x.json", free), "Data_x");
    assert_eq!(name_from_file("class.json", free), "class2", "a keyword");
    assert_eq!(
        name_from_file("Orders.json", |n| n.eq_ignore_ascii_case("orders")),
        "Orders2"
    );
    assert_eq!(format_of("A.JSON"), ImportFormat::Json);
    assert_eq!(format_of("a.xml"), ImportFormat::Xml);
    assert_eq!(format_of("notes.txt"), ImportFormat::Text);
}

#[test]
fn data_that_fits_a_schema_tab_is_given_it() {
    let dir = folder("import-fits");
    let mut app = sample();
    let orders = json::to_string(app.calc().data("Orders").unwrap(), true).unwrap();
    let file = write(&dir, "orders.json", &orders);
    app.begin_import_path(&file).unwrap();
    let plan = app.imports.pop_front().unwrap();
    assert_eq!(plan.name, "orders2", "Orders is taken, ignoring case");
    assert_eq!(
        plan.choice,
        SchemaChoice::Existing("Orders_schema".to_owned())
    );
    let prices = plan
        .candidates
        .iter()
        .find(|c| c.schema == "Prices_schema")
        .unwrap();
    assert!(prices.problems > 0);
    app.import(&plan).unwrap();
    assert_eq!(tabs(&app).len(), 5, "no schema tab is added");
    assert_eq!(app.sheet, "orders2");
    assert_eq!(app.calc().schema_of("orders2"), Some("Orders_schema"));
    assert!(valid(&app, "orders2"));
    assert_eq!(app.calc().undo_label(), Some("Import orders.json"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn data_that_fits_no_schema_tab_gets_an_inferred_one() {
    let mut app = sample();
    let text = r#"{"city": "Utrecht", "people": [{"name": "Ann", "age": 31}, {"name": "Bo", "note": null}]}"#;
    let plan = app.plan_import("people.json", text.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    assert_eq!(plan.schema_name, "peopleSchema");
    assert_eq!(
        plan.report(),
        [
            "people[].note: 1 null left out, since version 1 has no null, so it is optional.",
            "people[].note: inferred from no data, so it is text.",
        ]
    );
    app.import(&plan).unwrap();
    assert_eq!(&tabs(&app)[4..], ["peopleSchema", "people"]);
    assert_eq!(app.calc().schema_of("people"), Some("peopleSchema"));
    assert!(!app.calc().auto_update("peopleSchema"));
    assert!(valid(&app, "people"));

    // The next file of that shape fits the schema that was inferred.
    let again = app
        .plan_import("people.json", br#"{"city": "Delft", "people": []}"#)
        .unwrap();
    assert_eq!(
        again.choice,
        SchemaChoice::Existing("peopleSchema".to_owned())
    );
    assert_eq!(again.name, "people2");

    app.undo();
    assert_eq!(tabs(&app).len(), 4, "both tabs go in one undo");
}

#[test]
fn xml_is_imported_with_an_inferred_xsd() {
    let mut app = sample();
    let text = r#"<?xml version="1.0"?>
<catalog><book id="b1"><title>Dune</title><year>1965</year></book><book id="b2"><title>Emma</title><year>1815</year></book></catalog>"#;
    let plan = app.plan_import("catalog.xml", text.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    app.import(&plan).unwrap();
    assert!(app.calc().is_xml("catalog"));
    assert_eq!(
        app.calc().schema_format("catalogSchema"),
        Some(cliotree_calc::SchemaFormat::Xsd)
    );
    let year = app
        .calc()
        .data("catalog")
        .unwrap()
        .get(&p(r#"book[1].year"#))
        .map(|n| n.value().clone());
    assert_eq!(year, Some(Value::Int(1815)));
    assert!(valid(&app, "catalog"));

    let again = app.plan_import("more.xml", text.as_bytes()).unwrap();
    assert_eq!(
        again.choice,
        SchemaChoice::Existing("catalogSchema".to_owned())
    );
    app.import(&again).unwrap();
    assert!(app.calc().is_xml("more"));
    assert!(valid(&app, "more"));
}

#[test]
fn xml_outside_the_subset_can_still_go_into_an_xsd_tab_or_is_refused() {
    let app = sample();
    let refused = app
        .plan_import("p.xml", b"<p>Hello <b>you</b></p>")
        .unwrap_err();
    assert!(refused.contains("mixed content"), "{refused}");
    let broken = app.plan_import("x.xml", b"<a><b></a>").unwrap_err();
    assert!(
        broken.starts_with("x.xml is not well-formed XML"),
        "{broken}"
    );
    let not_json = app.plan_import("x.json", b"{nope").unwrap_err();
    assert!(not_json.starts_with("x.json is not JSON"), "{not_json}");
    let binary = app.plan_import("x.bin", &[0xFF, 0xFE, 0x00]).unwrap_err();
    assert!(binary.contains("not UTF-8"), "{binary}");
}

#[test]
fn plain_text_is_an_array_of_lines() {
    let mut app = sample();
    let plan = app
        .plan_import("notes.txt", b"\xEF\xBB\xBFone\r\ntwo\n")
        .unwrap();
    app.import(&plan).unwrap();
    let data = json::to_string(app.calc().data("notes").unwrap(), false).unwrap();
    assert_eq!(data, r#"["one","two"]"#);
}

#[test]
fn dropped_files_are_imported_one_dialog_after_another() {
    let dir = folder("drop");
    let mut app = sample();
    let a = write(&dir, "a.json", r#"{"x": 1}"#);
    let b = write(&dir, "b.xml", "<b><c>1</c></b>");
    let bad = write(&dir, "c.json", "{");
    app.drop_files(vec![a, b, bad]);
    assert_eq!(app.imports.len(), 2);
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("c.json is not JSON")
    );
    let first = app.imports.pop_front().unwrap();
    app.import(&first).unwrap();
    let second = app.imports.pop_front().unwrap();
    app.import(&second).unwrap();
    assert_eq!(&tabs(&app)[4..], ["aSchema", "a", "bSchema", "b"]);

    // A workbook that is dropped is opened.
    let saved = dir.join("w.clio");
    app.save_as(&saved).unwrap();
    let mut other = sample();
    other.drop_files(vec![saved]);
    assert_eq!(tabs(&other).len(), 8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_control_channel_manages_tabs() {
    let dir = folder("control-tabs");
    let mut app = sample();
    control::run(&mut app, "new_tab", &json!({ "name": "Extra" })).unwrap();
    assert_eq!(app.calc().schema_of("Extra"), Some("ExtraSchema"));
    control::run(
        &mut app,
        "bind_schema",
        &json!({ "tab": "Prices", "schema": "Orders_schema" }),
    )
    .unwrap();
    control::run(&mut app, "delete_tab", &json!({ "name": "Extra" })).unwrap();
    assert!(app.calc().data("Extra").is_none());
    let file = write(&dir, "list.json", "[1, 2, 3]");
    let answer = control::run(
        &mut app,
        "import",
        &json!({ "path": file.display().to_string(), "name": "Numbers" }),
    )
    .unwrap();
    assert_eq!(answer["tab"], "Numbers");
    assert_eq!(answer["schema"], "NumbersSchema");
    assert_eq!(app.calc().schema_of("Numbers"), Some("NumbersSchema"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_written_by_hand_with_a_trailing_comma_is_imported() {
    let mut app = sample();
    // The shape of the randomuser.me sample, with its comma before `}`.
    let text = r#"{"results": [{"gender": "female", "location": {"street": {"number": 8929, "name": "Valwood Pkwy",
}, "postcode": "63104"}, "dob": {"date": "1992-03-08T15:13:16.688Z", "age": 30}}],
"info": {"seed": "56d27f4a53bd5441", "results": 1, "page": 1, "version": "1.4"}}"#;
    let plan = app.plan_import("randomuser.json", text.as_bytes()).unwrap();
    assert_eq!(
        plan.notes,
        ["a comma before a closing bracket, on line 1, was ignored."]
    );
    assert_eq!(plan.choice, SchemaChoice::Inferred);
    app.import(&plan).unwrap();
    assert!(valid(&app, "randomuser"));
}
