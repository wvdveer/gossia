//! The toolbar and the node inspector (DESIGN.md, "Look and feel"), driven
//! as a user does.

use std::time::Duration;

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use gossia_app::GossiaApp;
use gossia_app::inspector::{Fact, Tone};
use gossia_app::prefs::ThemeChoice;
use gossia_core::Path;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn window() -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 760.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, GossiaApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
}

fn click(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn value(harness: &Harness<'_, GossiaApp>, path: &str) -> String {
    let app = harness.state();
    let node = app.calc().data("Orders").unwrap().get(&p(path)).cloned();
    node.map(|n| gossia_app::rows::display(n.value()))
        .unwrap_or_default()
}

/// The text of the first fact with this label.
fn said<'a>(facts: &'a [Fact], label: &str) -> Option<&'a str> {
    facts
        .iter()
        .find(|f| f.label == label)
        .map(|f| f.text.as_str())
}

#[test]
fn undo_and_redo_on_the_toolbar_follow_the_history() {
    let mut harness = window();
    assert!(harness.get_by_label("Undo").accesskit_node().is_disabled());
    click(&mut harness, "0.21");
    harness.event(egui::Event::Text("0.5".to_owned()));
    harness.run();
    harness.key_press(Key::Enter);
    settle(&mut harness);
    assert_eq!(value(&harness, "tax_rate"), "0.5");
    click(&mut harness, "Undo");
    settle(&mut harness);
    assert_eq!(value(&harness, "tax_rate"), "0.21");
    click(&mut harness, "Redo");
    settle(&mut harness);
    assert_eq!(value(&harness, "tax_rate"), "0.5");
}

#[test]
fn add_an_item_adds_to_the_selected_array_or_after_the_selected_item() {
    let mut harness = window();
    // Nothing in an array is selected.
    click(&mut harness, "0.21");
    assert!(
        harness
            .get_by_label("Add an item")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut harness, "items");
    click(&mut harness, "Add an item");
    settle(&mut harness);
    let count = |h: &Harness<'_, GossiaApp>| {
        h.state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&p("items"))
            .and_then(|n| n.value().as_array().map(<[_]>::len))
    };
    assert_eq!(count(&harness), Some(4));
    // A field of the second item: the new one comes after that item.
    harness.state_mut().select(&p("items[1].sku"));
    harness.run();
    click(&mut harness, "Add an item");
    settle(&mut harness);
    assert_eq!(count(&harness), Some(5));
    assert_eq!(value(&harness, "items[1].sku"), "B-220");
    assert_eq!(value(&harness, "items[3].sku"), "C-310");
}

#[test]
fn the_panels_and_the_theme_are_switched_from_the_toolbar() {
    let mut harness = window();
    click(&mut harness, "Show issues");
    harness.get_by_label("No errors, no invalid data.");
    click(&mut harness, "Show node inspector");
    harness.get_by_label("Inspector");
    harness.get_by_label("Select a node to see what it is.");
    click(&mut harness, "62.92");
    harness.get_by_label("Copy path");
    harness.get_by_label("Source");
    click(&mut harness, "Show node inspector");
    assert!(harness.query_by_label("Inspector").is_none());

    click(&mut harness, "Light theme");
    assert_eq!(
        harness.state().user.prefs.appearance.theme,
        ThemeChoice::Light
    );
    click(&mut harness, "Light theme");
    assert_eq!(
        harness.state().user.prefs.appearance.theme,
        ThemeChoice::System
    );

    // View > Toolbar takes the toolbar away.
    click(&mut harness, "View");
    click(&mut harness, "Toolbar");
    assert!(harness.query_by_label("New workbook").is_none());
}

#[test]
fn the_inspector_says_where_a_value_comes_from_and_what_is_allowed() {
    let mut harness = window();
    let app = harness.state();
    let total = app.inspect(&p("total"));
    assert_eq!(said(&total, "Path"), Some("Orders.total"));
    assert_eq!(said(&total, "Type"), Some("number"));
    assert_eq!(said(&total, "Required"), Some("Yes"));
    assert_eq!(said(&total, "Source"), Some("Formula"));
    assert_eq!(
        said(&total, "Formula"),
        Some("=Orders.subtotal + Orders.tax")
    );
    assert_eq!(said(&total, "Value"), Some("62.92"));

    let rate = app.inspect(&p("tax_rate"));
    assert_eq!(said(&rate, "Source"), Some("Value"));
    assert_eq!(said(&rate, "Allowed"), Some("from 0 to 1"));
    assert_eq!(said(&rate, "Formula"), None);

    let line = app.inspect(&p("items[0].total"));
    assert_eq!(said(&line, "Source"), Some("Column formula"));
    assert_eq!(said(&line, "Formula"), Some("=parent.qty * parent.price"));
    assert_eq!(said(&line, "Required"), Some("No"));

    let status = app.inspect(&p("customer.status"));
    assert_eq!(said(&status, "Allowed"), Some("one of new, paid, shipped"));
    let items = app.inspect(&p("items"));
    assert_eq!(said(&items, "Value"), Some("[3 items]"));
    assert_eq!(said(&items, "Allowed"), None, "{items:?}");
    let note = app.inspect(&p("items[0].note"));
    assert_eq!(said(&note, "Source"), Some("Absent"));
    assert_eq!(said(&note, "Problem"), None);

    // An error value.
    harness.state_mut().enter_directly(&p("tax_rate"), "=1/0");
    settle(&mut harness);
    let rate = harness.state().inspect(&p("tax_rate"));
    assert!(
        said(&rate, "Error").is_some_and(|e| e.starts_with("#DIV/0!")),
        "{rate:?}"
    );
}

#[test]
fn the_inspector_lists_what_is_wrong_with_data_read_from_a_file() {
    let mut calc = gossia_calc::Calc::new(std::sync::Arc::new(gossia_calc::python::PythonSyntax));
    let spec = gossia_core::jsonschema::lower_str(
        r#"{"type": "object", "properties": {
            "n": {"type": "integer", "minimum": 1},
            "name": {"type": "string"}}, "required": ["name"]}"#,
    )
    .unwrap();
    let data = gossia_core::json::parse(r#"{"n": 0}"#).unwrap().node;
    calc.add_sheet("T", spec, data).unwrap();
    let app = GossiaApp::new(calc);
    let n = app.inspect(&p("n"));
    let problem = n.iter().find(|f| f.label == "Problem").expect("a problem");
    assert_eq!(problem.tone, Tone::Error);
    assert!(problem.text.contains('1'), "{problem:?}");
    let name = app.inspect(&p("name"));
    assert_eq!(said(&name, "Source"), Some("Absent"));
    assert_eq!(said(&name, "Problem"), Some("Required, but absent"));
}
