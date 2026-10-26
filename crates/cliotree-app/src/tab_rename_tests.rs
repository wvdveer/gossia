//! Tests of renaming a tab from the window (DESIGN.md, "Renaming a tab").

use std::path::PathBuf;
use std::time::Duration;

use cliotree_calc::Mentions;
use cliotree_core::{ErrorKind, Path, Value};
use serde_json::{Value as Json, json};

use crate::ClioApp;
use crate::control;
use crate::file::{Limits, read_zip};
use crate::form::View;
use crate::tab_rename::mentions_text;
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

fn value(app: &ClioApp, sheet: &str, path: &str) -> Option<Value> {
    app.calc()
        .data(sheet)?
        .get(&p(path))
        .map(|n| n.value().clone())
}

fn tabs(app: &ClioApp) -> Vec<&str> {
    app.calc().sheet_names().collect()
}

#[test]
fn the_window_follows_a_renamed_tab_and_its_undo() {
    let mut app = sample();
    app.set_tabular(&p("items"), true);
    app.hide_column(&p("items"), "note");
    let total = value(&app, "Orders", "total");
    assert!(matches!(total, Some(Value::Float(_))), "{total:?}");

    app.rename_tab("Orders", "Sales").unwrap();
    settle(&mut app);
    assert_eq!(
        tabs(&app),
        ["Sales", "Prices", "Orders_schema", "Prices_schema"]
    );
    assert_eq!(app.sheet, "Sales", "the tab stays the current one");
    assert!(app.is_tabular(&p("items")), "its tables go with it");
    assert!(
        app.view_of(&p("items"))
            .unwrap()
            .hidden_columns
            .contains("note")
    );
    assert_eq!(app.calc().schema_users("Orders_schema"), ["Sales"]);
    // Its own formulas name it Orders, which is now missing.
    match value(&app, "Sales", "total") {
        Some(Value::Error(e)) => assert_eq!(e.kind, ErrorKind::Name),
        other => panic!("{other:?}"),
    }

    app.undo();
    settle(&mut app);
    assert_eq!(app.sheet, "Orders");
    assert!(app.is_tabular(&p("items")));
    assert_eq!(value(&app, "Orders", "total"), total);

    app.redo();
    assert_eq!(app.sheet, "Sales");
}

#[test]
fn a_renamed_tab_is_saved_under_its_new_name() {
    let folder = std::env::temp_dir().join(format!(
        "cliotree-rename-{}-{}",
        std::process::id(),
        new_workbook_id()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let file: PathBuf = folder.join("r.clio");
    let mut app = sample();
    app.switch_to("Prices");
    assert!(app.set_view(View::Raw));
    app.rename_tab("Orders_schema", "OrderTypes").unwrap();
    app.rename_tab("Prices", "PriceList").unwrap();
    app.save_as(&file).unwrap();

    let package = read_zip(&std::fs::read(&file).unwrap(), &Limits::default()).unwrap();
    assert!(package.get("sheets/OrderTypes/data.json").is_some());
    assert!(package.get("sheets/Orders_schema/data.json").is_none());
    let manifest: Json = serde_json::from_slice(package.get("manifest.json").unwrap()).unwrap();
    let orders = manifest["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Orders")
        .unwrap();
    assert_eq!(orders["schema"], json!("sheet:OrderTypes"));
    let overlay: Json =
        serde_json::from_slice(package.get("sheets/PriceList/overlay.json").unwrap()).unwrap();
    assert_eq!(
        overlay["view"],
        json!({ "show": "raw" }),
        "the view went with it"
    );

    let mut again = sample();
    again.open_path(&file).unwrap();
    settle(&mut again);
    assert_eq!(again.notice, None);
    assert_eq!(
        tabs(&again),
        ["Orders", "PriceList", "OrderTypes", "Prices_schema"]
    );
    assert_eq!(again.calc().schema_of("Orders"), Some("OrderTypes"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_dialog_checks_the_name_and_warns_about_formulas() {
    let mut app = sample();
    app.begin_rename_tab("Orders");
    assert_eq!(app.rename_tab.as_ref().unwrap().name, "Orders");
    assert!(
        app.rename_problem("Orders", "prices")
            .unwrap()
            .contains("exists already")
    );
    assert!(
        app.rename_problem("Orders", "my tab")
            .unwrap()
            .contains("not a valid")
    );
    assert_eq!(
        app.rename_problem("Orders", "orders"),
        None,
        "only the case"
    );
    assert_eq!(app.rename_problem("Orders", "Sales"), None);
    let mentions = app.calc().mentions("Orders");
    assert_eq!(mentions.formulas, 3);
    assert_eq!(
        mentions_text("Orders", mentions).unwrap(),
        "3 formulas use Orders. They are not changed, and will show #NAME! until they are edited."
    );
    assert_eq!(
        mentions_text(
            "X",
            Mentions {
                formulas: 1,
                modules: 0
            }
        )
        .unwrap(),
        "1 formula uses X. It is not changed, and will show #NAME! until it is edited."
    );
    assert_eq!(mentions_text("X", Mentions::default()), None);
}

#[test]
fn the_control_channel_renames_a_tab() {
    let mut app = sample();
    control::run(
        &mut app,
        "rename_tab",
        &json!({ "from": "Prices", "to": "PriceList" }),
    )
    .unwrap();
    assert_eq!(tabs(&app)[1], "PriceList");
    let refused = control::run(
        &mut app,
        "rename_tab",
        &json!({ "from": "PriceList", "to": "SUM" }),
    )
    .unwrap_err();
    assert!(refused.contains("built-in function"), "{refused}");
}
