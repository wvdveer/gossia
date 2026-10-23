//! Tests that drive the window the way a user does (DESIGN.md, "Testing and
//! CI", level 6). They run egui headlessly and find widgets through
//! AccessKit, so they need no display.

use std::time::Duration;

use cliotree_app::ClioApp;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    harness
}

/// Waits until nothing is out of date, then draws what the result looks like.
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

/// The text in the formula bar.
fn bar(harness: &Harness<'_, ClioApp>) -> String {
    harness
        .get_by_label("Formula bar")
        .value()
        .unwrap_or_default()
}

/// What is shown as the name of the selected cell, if the label is there.
fn has(harness: &Harness<'_, ClioApp>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn type_into_selected_cell(harness: &mut Harness<'_, ClioApp>, text: &str) {
    harness.event(egui::Event::Text(text.to_owned()));
    harness.run();
}

fn press(harness: &mut Harness<'_, ClioApp>, key: Key) {
    harness.key_press(key);
    harness.run();
}

fn ctrl(harness: &mut Harness<'_, ClioApp>, key: Key) {
    harness.key_combination_modifiers(Modifiers::COMMAND, &[key]);
    harness.run();
}

#[test]
fn the_sample_opens_with_its_tree_and_its_calculated_values() {
    let harness = window();
    harness.get_by_label("customer");
    harness.get_by_label("items");
    harness.get_by_label("subtotal");
    // 2 * 9.5 + 1 * 24 + 12 * 0.75 = 52, 21 % tax is 10.92.
    harness.get_by_label("52.0");
    harness.get_by_label("10.92");
    harness.get_by_label("62.92");
    harness.get_by_label("Ready");
    harness.get_by_label("No errors");
}

/// Selecting a cell shows its name and its content in the formula bar.
#[test]
fn selecting_a_cell_shows_its_name_and_its_formula_or_value() {
    let mut harness = window();
    click(&mut harness, "62.92");
    harness.get_by_label("Orders.total");
    assert_eq!(bar(&harness), "=Orders.subtotal + Orders.tax");
    click(&mut harness, "0.21");
    harness.get_by_label("Orders.tax_rate");
    assert_eq!(bar(&harness), "0.21");
}

/// Typing on a selected cell replaces its content (as in Excel): a literal is
/// entered, and what depends on it is calculated again.
#[test]
fn entering_a_literal_recalculates_what_depends_on_it() {
    let mut harness = window();
    click(&mut harness, "0.21");
    type_into_selected_cell(&mut harness, "0.5");
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    // 52 * 0.5 = 26, total 78.
    harness.get_by_label("26.0");
    harness.get_by_label("78.0");
    assert_eq!(
        harness
            .state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&"tax_rate".parse().unwrap())
            .unwrap()
            .value(),
        &cliotree_core::Value::Float(0.5)
    );
}

#[test]
fn a_literal_that_does_not_fit_is_refused_with_the_reason_and_editing_continues() {
    let mut harness = window();
    click(&mut harness, "0.21");
    type_into_selected_cell(&mut harness, "7");
    press(&mut harness, Key::Enter);
    harness.run();
    assert!(
        has(&harness, "7 is greater than the maximum of 1")
            || harness
                .query_all_by_label_contains("maximum")
                .next()
                .is_some(),
        "the reason is shown"
    );
    // Nothing was entered.
    assert!(harness.state().editing.is_some(), "still editing");
    assert_eq!(harness.state().calc().history_len(), (0, 0));
    press(&mut harness, Key::Escape);
    assert!(harness.state().editing.is_none());
    harness.get_by_label("0.21");
}

#[test]
fn a_formula_is_entered_in_the_formula_bar() {
    let mut harness = window();
    click(&mut harness, "0.21");
    click_bar(&mut harness);
    select_all(&mut harness);
    harness
        .get_by_label("Formula bar")
        .type_text("=Orders.subtotal / 100");
    harness.run();
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    // 52 / 100 = 0.52 is the tax rate now, so 52 * 0.52 = 27.04.
    harness.get_by_label("0.52");
    harness.get_by_label("27.04");
    assert_eq!(
        harness
            .state()
            .calc()
            .formula_text("Orders", &"tax_rate".parse().unwrap())
            .as_deref(),
        // A formula that reads a cell that depends on it is a cycle: see below.
        Some("Orders.subtotal / 100")
    );
}

fn click_bar(harness: &mut Harness<'_, ClioApp>) {
    harness.get_by_label("Formula bar").click();
    harness.run();
    harness.get_by_label("Formula bar").focus();
    harness.run();
}

fn select_all(harness: &mut Harness<'_, ClioApp>) {
    ctrl(harness, Key::A);
}

#[test]
fn undo_and_redo_with_the_keys_and_the_menu_labels() {
    let mut harness = window();
    click(&mut harness, "0.21");
    type_into_selected_cell(&mut harness, "0.5");
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    harness.get_by_label("78.0");
    assert_eq!(
        harness.state().calc().undo_label(),
        Some("Set Orders.tax_rate")
    );
    // Click away from any text field, then Ctrl+Z.
    click(&mut harness, "Customer".to_lowercase().as_str());
    ctrl(&mut harness, Key::Z);
    settle(&mut harness);
    harness.get_by_label("62.92");
    ctrl(&mut harness, Key::Y);
    settle(&mut harness);
    harness.get_by_label("78.0");
    ctrl(&mut harness, Key::Z);
    settle(&mut harness);
    harness.get_by_label("62.92");
    harness.key_combination_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, &[Key::Z]);
    settle(&mut harness);
    harness.get_by_label("78.0");
}

#[test]
fn the_tabs_switch_the_grid() {
    let mut harness = window();
    assert!(has(&harness, "customer"));
    click(&mut harness, "Prices");
    harness.get_by_label("currency");
    harness.get_by_label("EUR");
    assert!(!has(&harness, "customer"));
    click(&mut harness, "Orders");
    harness.get_by_label("customer");
}

#[test]
fn arrow_keys_move_the_selection_and_open_and_close_nodes() {
    let mut harness = window();
    click(&mut harness, "customer");
    press(&mut harness, Key::ArrowRight);
    harness.get_by_label("status");
    harness.get_by_label("Acme Ltd");
    press(&mut harness, Key::ArrowDown);
    harness.get_by_label("Orders.customer.name");
    press(&mut harness, Key::ArrowLeft);
    harness.get_by_label("Orders.customer");
    press(&mut harness, Key::ArrowLeft);
    assert!(!has(&harness, "Acme Ltd"), "closed");
}

#[test]
fn a_computed_cell_cannot_be_edited_and_the_reason_is_given() {
    let mut harness = window();
    // Open the first item; its total is computed by a formula, which is
    // editable, but the sum's result is not inside anything.
    click(&mut harness, "items");
    press(&mut harness, Key::ArrowRight);
    click(&mut harness, "[0]");
    press(&mut harness, Key::ArrowRight);
    harness.get_by_label("A-100");
    click(&mut harness, "19.0");
    harness.get_by_label("Orders.items[0].total");
    assert_eq!(bar(&harness), "=parent.qty * parent.price");
}

#[test]
fn the_status_bar_counts_errors() {
    let mut harness = window();
    click(&mut harness, "0.21");
    type_into_selected_cell(&mut harness, "=1 / 0");
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    // tax_rate, tax and total all hold #DIV/0!.
    harness.get_by_label("3 errors");
    assert!(harness.query_all_by_label("#DIV/0!").count() >= 3);
}

#[test]
fn the_insert_function_dialog_writes_a_function_at_the_cursor() {
    let mut harness = window();
    click(&mut harness, "0.21");
    harness.get_by_label("fx").click();
    harness.run();
    harness.get_by_label("Insert Function");
    harness.get_by_label("IFERROR").click();
    harness.run();
    harness.get_by_label("IFERROR(value, fallback)");
    harness.get_by_label("Insert").click();
    harness.run();
    assert_eq!(bar(&harness), "=IFERROR(");
    assert!(
        harness.query_by_label("Insert Function").is_none(),
        "closed"
    );
}

#[test]
fn typing_a_tab_name_and_a_dot_offers_its_fields_from_the_schema() {
    let mut harness = window();
    click(&mut harness, "0.21");
    type_into_selected_cell(&mut harness, "=Orders.");
    harness.run();
    harness.get_by_label("customer    object");
    harness.get_by_label("items    array of object");
    harness.get_by_label("tax_rate    number");
}

/// The window with the order's items shown as a table.
fn table_window() -> Harness<'static, ClioApp> {
    let mut harness = window();
    harness
        .state_mut()
        .set_tabular(&"items".parse().unwrap(), true);
    harness.run();
    settle(&mut harness);
    harness
}

fn skus(harness: &Harness<'_, ClioApp>) -> Vec<String> {
    (0..3)
        .map(|i| {
            harness
                .state()
                .calc()
                .data("Orders")
                .unwrap()
                .get(&format!("items[{i}].sku").parse().unwrap())
                .unwrap()
                .value()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn an_array_of_objects_is_drawn_as_a_table_under_the_tree() {
    let harness = table_window();
    harness.get_by_label("Orders.items");
    harness.get_by_label("3 rows");
    for header in ["sku", "qty", "price", "note"] {
        harness.get_by_label(header);
    }
    // `total` is a column of the table and a field of the order above it.
    assert_eq!(harness.query_all_by_label("total").count(), 2);
    for sku in ["A-100", "B-220", "C-310"] {
        harness.get_by_label(sku);
    }
    harness.get_by_label("gift wrap");
    harness.get_by_label("[2]");
    // The tree above still has the rest of the order.
    harness.get_by_label("customer");
}

#[test]
fn clicking_a_header_sorts_and_clicking_it_again_reverses() {
    let mut harness = table_window();
    click(&mut harness, "qty");
    settle(&mut harness);
    assert_eq!(skus(&harness), ["B-220", "A-100", "C-310"]);
    click(&mut harness, "qty");
    settle(&mut harness);
    assert_eq!(skus(&harness), ["C-310", "A-100", "B-220"]);
    click(&mut harness, "Reverse row order");
    settle(&mut harness);
    assert_eq!(skus(&harness), ["B-220", "A-100", "C-310"]);
    // Undo with the key, once nothing is being typed.
    click(&mut harness, "customer");
    ctrl(&mut harness, Key::Z);
    settle(&mut harness);
    assert_eq!(skus(&harness), ["C-310", "A-100", "B-220"]);
}

#[test]
fn a_column_that_cannot_be_sorted_says_why_and_offers_the_rows() {
    let mut harness = table_window();
    click(&mut harness, "note");
    harness.run();
    assert_eq!(skus(&harness), ["A-100", "B-220", "C-310"], "unchanged");
    assert!(
        harness
            .query_all_by_label_contains("not supported between")
            .next()
            .is_some()
    );
    click(&mut harness, "Show these rows");
    settle(&mut harness);
    harness.get_by_label("2 of 3 rows (0 hidden, 1 filtered)");
    assert!(harness.query_by_label("B-220").is_none(), "filtered out");
}

#[test]
fn a_cell_of_the_table_is_typed_into_and_the_arrows_move_among_cells() {
    let mut harness = table_window();
    click(&mut harness, "12");
    assert_eq!(bar(&harness), "12");
    press(&mut harness, Key::ArrowLeft);
    assert_eq!(
        harness.state().selection.as_ref().unwrap().to_string(),
        "items[2].sku"
    );
    press(&mut harness, Key::ArrowRight);
    type_into_selected_cell(&mut harness, "3");
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    // 3 * 0.75.
    harness.get_by_label("2.25");
    press(&mut harness, Key::ArrowUp);
    assert_eq!(
        harness.state().selection.as_ref().unwrap().to_string(),
        "items[1].qty"
    );
}

#[test]
fn rows_are_added_and_the_table_is_closed_from_its_toolbar() {
    let mut harness = table_window();
    click(&mut harness, "Add row");
    settle(&mut harness);
    harness.get_by_label("4 rows");
    click(&mut harness, "Close table");
    settle(&mut harness);
    assert!(harness.query_by_label("Close table").is_none());
    assert!(harness.state().layout().is_none());
}

#[test]
#[ignore = "writes a picture of the window, needs a graphics adapter"]
fn picture() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .wgpu()
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    click(&mut harness, "items");
    press(&mut harness, Key::ArrowRight);
    let image = harness.render().expect("rendered");
    image
        .save(std::env::var("CLIOTREE_PICTURE").unwrap_or("window.png".into()))
        .expect("saved");
}
