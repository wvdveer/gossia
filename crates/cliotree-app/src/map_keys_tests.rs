//! Tests of adding and renaming map entries from the data tree.

use std::time::Duration;

use cliotree_core::{Path, Value, json};

use crate::ClioApp;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

/// The sample with a tab `Stock` holding the map `counts`.
fn with_map() -> ClioApp {
    let mut app = ClioApp::sample();
    app.files.browse = false;
    let schema = json::parse(
        r#"{"type": "object", "properties": {"counts": {"type": "object",
            "additionalProperties": {"type": "integer"}}}}"#,
    )
    .unwrap()
    .node;
    let data = json::parse(r#"{"counts": {"apple": 3, "pear": 5}}"#)
        .unwrap()
        .node;
    let calc = app.session.calc_mut();
    calc.create_json_schema_tab("StockSchema", schema).unwrap();
    calc.create_data_tab("Stock", "StockSchema", data, None)
        .unwrap();
    app.follow_tabs();
    app.show_tab("Stock");
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app
}

fn keys(app: &ClioApp) -> Vec<String> {
    match app
        .calc()
        .data("Stock")
        .unwrap()
        .get(&p("counts"))
        .unwrap()
        .value()
    {
        Value::Object(fields) => fields.keys().cloned().collect(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_entry_is_added_to_a_map_from_the_tree() {
    let mut app = with_map();
    assert!(app.is_map(&p("counts")));
    assert!(!app.is_map(&p("counts.apple")));
    app.begin_add_entry(&p("counts"));
    assert!(app.key_dialog.is_some());
    assert!(
        app.key_problem(&p("counts"), None, "apple")
            .unwrap()
            .contains("already has an entry apple")
    );
    assert!(app.key_problem(&p("counts"), None, "").is_some());
    app.add_map_entry(&p("counts"), "plum").unwrap();
    assert_eq!(keys(&app), ["apple", "pear", "plum"]);
    assert_eq!(
        app.calc()
            .data("Stock")
            .unwrap()
            .get(&p("counts.plum"))
            .unwrap()
            .value(),
        &Value::Int(0)
    );
    app.undo();
    assert_eq!(keys(&app), ["apple", "pear"]);
}

#[test]
fn a_key_is_renamed_in_its_place_with_its_formula() {
    let mut app = with_map();
    app.session
        .calc_mut()
        .set_formula("Stock", &p("counts.pear"), "2 + 2")
        .unwrap();
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app.select(&p("counts.pear"));
    app.begin_rename_key(&p("counts.pear"));
    assert_eq!(app.key_dialog.as_ref().unwrap().key, "pear");
    assert!(app.rename_map_key(&p("counts"), "pear", "apple").is_err());
    app.rename_map_key(&p("counts"), "pear", "quince").unwrap();
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    assert_eq!(keys(&app), ["apple", "quince"]);
    assert_eq!(app.selection, Some(p("counts.quince")));
    let calc = app.calc();
    assert_eq!(
        calc.formula_text("Stock", &p("counts.quince")).as_deref(),
        Some("2 + 2")
    );
    assert!(calc.formula_text("Stock", &p("counts.pear")).is_none());
    assert_eq!(
        calc.undo_label(),
        Some("Rename Stock.counts.pear to quince")
    );
    app.undo();
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    assert_eq!(keys(&app), ["apple", "pear"]);
    assert_eq!(
        app.calc()
            .formula_text("Stock", &p("counts.pear"))
            .as_deref(),
        Some("2 + 2")
    );
    assert!(
        app.calc()
            .formula_text("Stock", &p("counts.quince"))
            .is_none()
    );
    app.redo();
    assert_eq!(keys(&app), ["apple", "quince"]);
}
