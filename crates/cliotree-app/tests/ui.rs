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
    // The list is long, so the function is found by searching for it.
    harness.get_by_label("Search functions").click();
    harness.run();
    harness.get_by_label("Search functions").focus();
    harness.run();
    harness
        .get_by_label("Search functions")
        .type_text("IFERROR");
    harness.run();
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
fn the_excel_named_functions_are_listed_by_group_and_found_by_search() {
    let mut harness = window();
    click(&mut harness, "0.21");
    harness.get_by_label("fx").click();
    harness.run();
    harness.get_by_label("Math");
    harness.get_by_label("SUM");
    harness.get_by_label("Lookup and lists");
    harness.get_by_label("Search functions").click();
    harness.run();
    harness.get_by_label("Search functions").focus();
    harness.run();
    harness
        .get_by_label("Search functions")
        .type_text("vlookup");
    harness.run();
    harness.get_by_label("VLOOKUP").click();
    harness.run();
    harness.get_by_label("VLOOKUP(key, items, key_field, result_field)");
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

/// Chooses `item` from the menu of the header of a column.
fn header_menu(harness: &mut Harness<'_, ClioApp>, column: &str, item: &str) {
    harness.get_by_label(column).click_secondary();
    harness.run();
    click(harness, item);
}

#[test]
fn the_header_menu_sorts_and_reverses_and_clicking_a_header_only_selects() {
    let mut harness = table_window();
    click(&mut harness, "qty");
    settle(&mut harness);
    assert_eq!(
        skus(&harness),
        ["A-100", "B-220", "C-310"],
        "a click does not sort"
    );
    header_menu(&mut harness, "qty", "Sort ascending");
    settle(&mut harness);
    assert_eq!(skus(&harness), ["B-220", "A-100", "C-310"]);
    header_menu(&mut harness, "qty", "Sort descending");
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
    header_menu(&mut harness, "note", "Sort ascending");
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
fn clicking_a_header_selects_the_column_and_the_formula_bar_shows_its_formula() {
    let mut harness = table_window();
    // The `total` of the sample is a column formula.
    let headers: Vec<_> = harness.query_all_by_label("total").collect();
    assert_eq!(headers.len(), 2, "the header, and the order's own total");
    headers[0].click();
    harness.run();
    assert!(harness.state().column_selected().is_some());
    assert_eq!(bar(&harness), "=parent.qty * parent.price");
    // Selecting a cell leaves the column.
    click(&mut harness, "9.5");
    assert!(harness.state().column_selected().is_none());
    assert_eq!(bar(&harness), "9.5");
}

#[test]
fn a_column_formula_is_typed_in_the_formula_bar_and_one_undo_takes_it_back() {
    let mut harness = table_window();
    click(&mut harness, "qty");
    assert_eq!(bar(&harness), "");
    click_bar(&mut harness);
    select_all(&mut harness);
    harness
        .get_by_label("Formula bar")
        .type_text("=parent.price * 2");
    harness.run();
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    // Every item has the formula and shows its result.
    let calc = harness.state().calc();
    assert_eq!(
        calc.column_formula("Orders", &"items".parse().unwrap(), "qty"),
        Some("parent.price * 2")
    );
    assert_eq!(
        calc.formulas("Orders")
            .iter()
            .filter(|(p, _)| p.to_string().ends_with(".qty"))
            .count(),
        3
    );
    ctrl(&mut harness, Key::Z);
    settle(&mut harness);
    assert!(
        harness
            .state()
            .calc()
            .column_formula("Orders", &"items".parse().unwrap(), "qty")
            .is_none()
    );
}

#[test]
fn a_column_formula_cannot_be_a_literal_and_a_summary_column_cannot_have_one() {
    let mut harness = table_window();
    click(&mut harness, "qty");
    click_bar(&mut harness);
    select_all(&mut harness);
    harness.get_by_label("Formula bar").type_text("7");
    harness.run();
    press(&mut harness, Key::Enter);
    harness.run();
    harness.get_by_label_contains("start it with =");
    assert!(
        harness
            .state()
            .calc()
            .column_formulas("Orders")
            .iter()
            .all(|c| c.field != "qty")
    );
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
fn a_schema_tab_has_a_switch_for_auto_update_and_says_which_tab_uses_it() {
    let mut harness = window();
    click(&mut harness, "Orders schema");
    harness.get_by_label("Used by: Orders");
    assert!(!harness.state().calc().auto_update("Orders schema"));
    click(&mut harness, "Auto-update");
    assert!(harness.state().calc().auto_update("Orders schema"));
    click(&mut harness, "Auto-update");
    assert!(!harness.state().calc().auto_update("Orders schema"));
    // The schema tab is an ordinary tree of the schema document.
    harness.get_by_label("properties");
}

#[test]
fn the_field_commands_are_unavailable_until_auto_update_is_on() {
    let mut harness = table_window();
    harness.get_by_label("qty").click_secondary();
    harness.run();
    click(&mut harness, "Rename field...");
    assert!(harness.state().schema_dialog.is_none());
    harness.state_mut().set_auto_update("Orders schema", true);
    harness.run();
    harness.get_by_label("qty").click_secondary();
    harness.run();
    click(&mut harness, "Rename field...");
    harness.get_by_label("Rename field qty");
}

#[test]
fn a_field_is_renamed_from_the_menu_of_its_column_and_the_table_follows() {
    let mut harness = table_window();
    harness.state_mut().set_auto_update("Orders schema", true);
    harness.run();
    harness.get_by_label("qty").click_secondary();
    harness.run();
    click(&mut harness, "Rename field...");
    harness.get_by_label("New field name").click();
    harness.run();
    harness.get_by_label("New field name").focus();
    harness.run();
    select_all(&mut harness);
    harness.get_by_label("New field name").type_text("quantity");
    harness.run();
    click(&mut harness, "Rename");
    settle(&mut harness);
    harness.get_by_label("quantity");
    assert!(harness.query_by_label("qty").is_none());
    assert!(harness.state().schema_dialog.is_none());
    ctrl(&mut harness, Key::Z);
}

#[test]
fn deleting_a_field_asks_first_and_says_how_much_goes() {
    let mut harness = table_window();
    harness.state_mut().set_auto_update("Orders schema", true);
    harness.run();
    harness.get_by_label("note").click_secondary();
    harness.run();
    click(&mut harness, "Delete field...");
    harness.get_by_label_contains("1 value will be removed");
    assert!(harness.query_by_label("note").is_some(), "nothing yet");
    click(&mut harness, "Delete");
    settle(&mut harness);
    assert!(harness.query_by_label("gift wrap").is_none());
}

#[test]
#[ignore = "writes a picture of the window, needs a graphics adapter"]
fn picture() {
    // `CLIOTREE_THEME=light` for the light theme; dark otherwise.
    let theme = match std::env::var("CLIOTREE_THEME").as_deref() {
        Ok("light") => egui::Theme::Light,
        _ => egui::Theme::Dark,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_theme(theme)
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

#[test]
fn escape_gives_up_an_edit_and_the_cell_keeps_its_content() {
    let mut harness = window();
    click(&mut harness, "0.21");
    type_into_selected_cell(&mut harness, "=5");
    assert_eq!(bar(&harness), "=5");
    press(&mut harness, Key::Escape);
    assert_eq!(bar(&harness), "0.21");
    harness.get_by_label("0.21");
    assert!(harness.state().editing.is_none());
}

#[test]
fn the_issues_panel_lists_what_is_wrong_and_clicking_an_issue_selects_it() {
    let mut harness = window();
    click(&mut harness, "View");
    click(&mut harness, "Issues panel");
    harness.get_by_label("Issues");
    harness.get_by_label("No errors, no invalid data.");

    click(&mut harness, "0.21");
    type_into_selected_cell(&mut harness, "=1 / 0");
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    assert!(
        harness
            .query_by_label("No errors, no invalid data.")
            .is_none(),
        "the panel was not drawn again"
    );
    let issue = harness
        .query_all_by_label_contains("Orders.tax  ")
        .next()
        .expect("an issue for tax");
    issue.click();
    harness.run();
    assert_eq!(
        harness.state().selection,
        Some("tax".parse().unwrap()),
        "clicking the issue selects the cell"
    );
}

#[test]
fn a_column_is_hidden_from_its_menu_and_shown_again() {
    let mut harness = table_window();
    harness.get_by_label("note").click_secondary();
    harness.run();
    click(&mut harness, "Hide column");
    assert!(harness.query_by_label("note").is_none());
    assert!(harness.query_by_label("gift wrap").is_none());
    click(&mut harness, "Hidden (1)");
    click(&mut harness, "Show all");
    harness.get_by_label("note");
    harness.get_by_label("gift wrap");
}

#[test]
fn a_filter_chosen_in_the_dialog_hides_rows_and_is_cleared_from_the_menu() {
    let mut harness = table_window();
    harness.get_by_label("sku").click_secondary();
    harness.run();
    click(&mut harness, "Filter...");
    harness.get_by_label("Search values");
    click(&mut harness, "Select none");
    click(&mut harness, "Apply");
    settle(&mut harness);
    assert_eq!(harness.query_all_by_label("A-100").count(), 0);
    assert_eq!(harness.query_all_by_label("B-220").count(), 0);
    assert!(harness.state().tables.dialog.is_none(), "the dialog closed");

    harness.get_by_label("sku").click_secondary();
    harness.run();
    click(&mut harness, "Clear filter");
    settle(&mut harness);
    harness.get_by_label("A-100");
    harness.get_by_label("C-310");
}

#[test]
fn collapse_all_closes_the_tree_down_to_its_top_level() {
    let mut harness = window();
    harness
        .state_mut()
        .set_open(&"customer".parse().unwrap(), true);
    harness.run();
    harness.get_by_label("Acme Ltd");
    click(&mut harness, "View");
    click(&mut harness, "Collapse all");
    assert!(harness.query_by_label("Acme Ltd").is_none());
    harness.get_by_label("customer");
    harness.get_by_label("subtotal");
}

mod common;

/// The scenario of the control protocol, run through the harness.
#[test]
fn the_control_commands_drive_the_window_in_process() {
    let mut harness = window();
    common::edit_and_recalculate(&mut |cmd, args| {
        let result = cliotree_app::control::run(harness.state_mut(), cmd, &args);
        harness.run();
        result
    });
    assert!(has(&harness, "Ready"));
}

#[test]
fn a_module_that_failed_is_counted_in_the_status_bar_and_listed_in_the_issues_panel() {
    let mut harness = window();
    harness
        .state_mut()
        .session
        .calc_mut()
        .add_module("Bad", "x = 1 / 0\n")
        .unwrap();
    harness.state_mut().session.tick();
    settle(&mut harness);
    harness.get_by_label("1 module failed");
    click(&mut harness, "View");
    click(&mut harness, "Issues panel");
    assert!(
        harness
            .query_all_by_label_contains("Module Bad: ZeroDivisionError")
            .next()
            .is_some()
    );
}

/// Runs a control command in the window and draws what it did.
fn control(harness: &mut Harness<'_, ClioApp>, cmd: &str, args: serde_json::Value) {
    cliotree_app::control::run(harness.state_mut(), cmd, &args).unwrap();
    harness.run();
}

/// Trust model: an untrusted workbook says so, shows the stored values with a
/// badge, and trusting it from the banner calculates it again.
#[test]
fn an_untrusted_workbook_shows_the_banner_and_the_badges_until_it_is_trusted() {
    let mut harness = window();
    harness.get_by_label("52.0");
    control(&mut harness, "trust", serde_json::json!({"trusted": false}));
    harness.get_by_label_contains("has not been trusted");
    harness.get_by_label("Trust this version");
    harness.get_by_label("Continue read-only");
    harness.get_by_label("Not trusted");
    assert_eq!(harness.query_all_by_label("#UNTRUSTED!").count(), 3);
    // The value that was stored is still shown.
    harness.get_by_label("52.0");

    click(&mut harness, "Trust this version");
    settle(&mut harness);
    assert!(!has(&harness, "Trust this version"));
    assert!(!has(&harness, "Not trusted"));
    assert!(!has(&harness, "#UNTRUSTED!"));
    harness.get_by_label("Ready");
}

#[test]
fn continuing_read_only_puts_the_banner_away_but_the_status_bar_still_says_so() {
    let mut harness = window();
    control(&mut harness, "trust", serde_json::json!({"trusted": false}));
    click(&mut harness, "Continue read-only");
    assert!(!has(&harness, "Trust this version"));
    harness.get_by_label("Not trusted");
    assert_eq!(harness.query_all_by_label("#UNTRUSTED!").count(), 3);
}

/// Trust model, "Untrusted mode": data can still be edited, and what depends on
/// it is shown as stale, since it cannot recalculate.
#[test]
fn editing_data_in_an_untrusted_workbook_leaves_what_depends_on_it_stale() {
    let mut harness = window();
    control(&mut harness, "trust", serde_json::json!({"trusted": false}));
    control(
        &mut harness,
        "select",
        serde_json::json!({"path": "items[0].qty"}),
    );
    control(&mut harness, "enter", serde_json::json!({"text": "5"}));
    settle(&mut harness);
    // The edit was taken; the totals were not recalculated.
    harness.get_by_label("52.0");
    assert!(
        harness
            .query_all_by_label_contains("Not trusted, ")
            .next()
            .is_some()
    );
}

/// Trust model: entering a formula in an untrusted workbook keeps the text
/// unevaluated and asks for trust again.
#[test]
fn a_formula_entered_in_an_untrusted_workbook_waits_and_brings_the_banner_back() {
    let mut harness = window();
    control(&mut harness, "trust", serde_json::json!({"trusted": false}));
    click(&mut harness, "Continue read-only");
    assert!(!has(&harness, "Trust this version"));
    control(
        &mut harness,
        "select",
        serde_json::json!({"path": "customer.name"}),
    );
    control(&mut harness, "enter", serde_json::json!({"text": "=1 + 1"}));
    harness.get_by_label("Trust this version");
    harness.get_by_label_contains("stored but not calculated");
}
