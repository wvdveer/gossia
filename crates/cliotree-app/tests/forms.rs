//! Tests of the form view that drive the window (DESIGN.md, "Forms"): the
//! view selector, the generated Default form, and the way a field reads and
//! writes the tab.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::{Path, Value};
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, ClioApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60)),
        "the workbook did not settle"
    );
    harness.run();
}

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn has(harness: &Harness<'_, ClioApp>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn value(harness: &Harness<'_, ClioApp>, sheet: &str, path: &str) -> Option<Value> {
    harness
        .state()
        .calc()
        .data(sheet)?
        .get(&p(path))
        .map(|n| n.value().clone())
}

/// Types into a field and leaves it with Enter.
fn type_into(harness: &mut Harness<'_, ClioApp>, field: &str, text: &str) {
    harness.get_by_label(field).click();
    harness.run();
    // Replace what is there.
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[Key::A]);
    harness.get_by_label(field).type_text(text);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
}

fn form() -> Harness<'static, ClioApp> {
    let mut harness = window();
    click(&mut harness, "Default");
    settle(&mut harness);
    harness
}

#[test]
fn the_view_selector_switches_between_the_tree_and_the_default_form() {
    let mut harness = window();
    assert!(harness.state().current_view() == &View::Tree);
    assert!(has(&harness, "Formula bar"));
    click(&mut harness, "Default");
    assert!(harness.state().in_form());
    // A form has no formula bar, and shows the fields with their values.
    assert!(!has(&harness, "Formula bar"));
    harness.get_by_label("Orders.customer.name");
    click(&mut harness, "Data tree");
    assert!(!harness.state().in_form());
    assert!(has(&harness, "Formula bar"));
}

#[test]
fn each_tab_remembers_its_view() {
    let mut harness = window();
    click(&mut harness, "Default");
    click(&mut harness, "Prices");
    // The other tab is still the tree.
    assert!(harness.state().current_view() == &View::Tree);
    assert!(has(&harness, "Formula bar"));
    click(&mut harness, "Orders");
    assert!(harness.state().in_form());
}

#[test]
fn a_committed_field_is_an_ordinary_undoable_edit_of_the_tab() {
    let mut harness = form();
    type_into(&mut harness, "Orders.customer.name", "Globex");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "customer.name"),
        Some(Value::Str("Globex".into()))
    );
    assert_eq!(harness.state().calc().history_len().0, 1);
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "customer.name"),
        Some(Value::Str("Acme Ltd".into()))
    );
}

#[test]
fn a_value_that_does_not_fit_stays_in_the_field_and_is_not_written() {
    let mut harness = form();
    type_into(&mut harness, "Orders.tax_rate", "7");
    settle(&mut harness);
    // The tab keeps the previous value, nothing was recorded to undo, and the
    // reason is shown with the field.
    assert_eq!(
        value(&harness, "Orders", "tax_rate"),
        Some(Value::Float(0.21))
    );
    assert_eq!(harness.state().calc().history_len().0, 0);
    assert!(
        harness
            .query_all_by_label_contains("maximum")
            .next()
            .is_some(),
        "the reason is shown"
    );
    // The view cannot be left with an entry that is refused.
    harness.get_by_label("Data tree").click();
    harness.run();
    assert!(harness.state().in_form());
    // Correcting it lets the value in, and the view can change.
    type_into(&mut harness, "Orders.tax_rate", "0.5");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "tax_rate"),
        Some(Value::Float(0.5))
    );
    click(&mut harness, "Data tree");
    assert!(!harness.state().in_form());
}

#[test]
fn a_form_has_no_formulas() {
    let mut harness = form();
    type_into(&mut harness, "Orders.tax_rate", "=1/2");
    assert_eq!(
        value(&harness, "Orders", "tax_rate"),
        Some(Value::Float(0.21))
    );
    assert!(
        harness
            .query_all_by_label_contains("no formulas")
            .next()
            .is_some()
    );
}

#[test]
fn a_computed_field_is_read_only_and_follows_the_data() {
    let mut harness = form();
    // The total is a formula: it shows the calculated value, marked, and a
    // click on it enters nothing.
    assert!(harness.query_all_by_label_contains("fx").next().is_some());
    harness.get_by_label("Orders.total");
    harness.get_by_label("Orders.tax_rate").click();
    harness.run();
    type_into(&mut harness, "Orders.tax_rate", "0.5");
    settle(&mut harness);
    // 52 * 0.5 = 26 and 52 + 26 = 78, shown without leaving the form.
    harness.get_by_label("Orders.total");
    assert_eq!(value(&harness, "Orders", "total"), Some(Value::Float(78.0)));
}

#[test]
fn a_toggle_and_a_choice_enter_at_once() {
    let mut harness = form();
    click(&mut harness, "Orders.customer.vip");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "customer.vip"),
        Some(Value::Bool(false))
    );
    // The choice of an enumeration.
    click(&mut harness, "Orders.customer.status");
    click(&mut harness, "shipped");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "customer.status"),
        Some(Value::Str("shipped".into()))
    );
}

#[test]
fn an_optional_field_that_is_absent_is_empty_and_a_value_creates_it() {
    let mut harness = form();
    assert_eq!(value(&harness, "Orders", "items[0].note"), None);
    type_into(&mut harness, "Orders.items[0].note", "fragile");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "items[0].note"),
        Some(Value::Str("fragile".into()))
    );
}

#[test]
fn the_items_of_an_array_are_a_table_with_row_commands() {
    let mut harness = form();
    harness.get_by_label("Orders.items[2].sku");
    // New adds a row at the end.
    click(&mut harness, "New");
    settle(&mut harness);
    assert_eq!(
        harness
            .state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&p("items"))
            .unwrap()
            .value()
            .as_array()
            .unwrap()
            .len(),
        4
    );
    harness.get_by_label("Orders.items[3].sku");
    // A row is deleted from its own buttons.
    harness.get_all_by_label("×").nth(3).unwrap().click();
    harness.run();
    settle(&mut harness);
    assert_eq!(
        harness
            .state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&p("items"))
            .unwrap()
            .value()
            .as_array()
            .unwrap()
            .len(),
        3
    );
    // Moving a row down swaps it with the next.
    harness.get_all_by_label("v").next().unwrap().click();
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "items[0].sku"),
        Some(Value::Str("B-220".into()))
    );
    assert_eq!(
        value(&harness, "Orders", "items[1].sku"),
        Some(Value::Str("A-100".into()))
    );
}

#[test]
fn the_columns_of_a_table_sort_the_array() {
    let mut harness = form();
    // Ascending by quantity: 1, 2, 12.
    click(&mut harness, "qty");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "items[0].sku"),
        Some(Value::Str("B-220".into()))
    );
    assert_eq!(
        harness.state().calc().undo_label(),
        Some("Sort Orders.items")
    );
}

#[test]
fn a_map_is_a_list_of_entries_that_can_be_added_to_and_removed_from() {
    let mut harness = window();
    click(&mut harness, "Prices");
    click(&mut harness, "Default");
    harness.get_by_label("Prices.list[\"A-100\"]");
    harness.get_by_label("New key of Prices.list").click();
    harness
        .get_by_label("New key of Prices.list")
        .type_text("E-500");
    harness.run();
    click(&mut harness, "Add entry");
    settle(&mut harness);
    assert!(value(&harness, "Prices", "list[\"E-500\"]").is_some());
    harness.get_all_by_label("×").next().unwrap().click();
    harness.run();
    settle(&mut harness);
    assert!(value(&harness, "Prices", "list[\"A-100\"]").is_none());
}

#[test]
fn switching_back_to_the_tree_selects_the_node_that_had_the_focus() {
    let mut harness = form();
    harness.get_by_label("Orders.customer.city").click();
    harness.run();
    assert_eq!(harness.state().selection, Some(p("customer.city")));
    click(&mut harness, "Data tree");
    assert_eq!(harness.state().selection, Some(p("customer.city")));
    harness.get_by_label("Orders.customer.city");
}

#[test]
fn the_control_channel_shows_a_view_and_enters_into_a_field() {
    use cliotree_app::control::run;
    use serde_json::json;
    let mut harness = window();
    let app = harness.state_mut();
    run(app, "view", &json!({"name": "default"})).unwrap();
    assert_eq!(run(app, "status", &json!({})).unwrap()["view"], "Default");
    run(app, "select", &json!({"path": "customer.city"})).unwrap();
    run(app, "enter", &json!({"text": "Delft"})).unwrap();
    // A refused entry is reported, and nothing is written.
    run(app, "select", &json!({"path": "tax_rate"})).unwrap();
    let why = run(app, "enter", &json!({"text": "7"})).unwrap_err();
    assert!(why.contains("maximum"), "{why}");
    assert!(run(app, "view", &json!({"name": "Data tree"})).is_err());
    assert!(run(app, "view", &json!({"name": "nothing"})).is_err());
    assert_eq!(
        value(&harness, "Orders", "customer.city"),
        Some(Value::Str("Delft".into()))
    );
}
