//! Tests that drive the window the way a user does (DESIGN.md, "Testing and
//! CI", level 6). They run egui headlessly and find widgets through
//! AccessKit, so they need no display.

use std::time::Duration;

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;

fn window() -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    harness
}

/// Waits until nothing is out of date, then draws what the result looks like.
fn settle(harness: &mut Harness<'_, GossiaApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60)),
        "the workbook did not settle"
    );
    harness.run();
}

fn click(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

/// The text in the formula bar.
fn bar(harness: &Harness<'_, GossiaApp>) -> String {
    harness
        .get_by_label("Formula bar")
        .value()
        .unwrap_or_default()
}

/// What is shown as the name of the selected cell, if the label is there.
fn has(harness: &Harness<'_, GossiaApp>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn type_into_selected_cell(harness: &mut Harness<'_, GossiaApp>, text: &str) {
    harness.event(egui::Event::Text(text.to_owned()));
    harness.run();
}

fn press(harness: &mut Harness<'_, GossiaApp>, key: Key) {
    harness.key_press(key);
    harness.run();
}

fn ctrl(harness: &mut Harness<'_, GossiaApp>, key: Key) {
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
        &gossia_core::Value::Float(0.5)
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

fn click_bar(harness: &mut Harness<'_, GossiaApp>) {
    harness.get_by_label("Formula bar").click();
    harness.run();
    harness.get_by_label("Formula bar").focus();
    harness.run();
}

fn select_all(harness: &mut Harness<'_, GossiaApp>) {
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
    harness.get_by_label("Search functions").type_text("lookup");
    harness.run();
    harness.get_by_label("LOOKUP").click();
    harness.run();
    harness.get_by_label("LOOKUP(key, items, key_field, result_field=None)");
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
fn table_window() -> Harness<'static, GossiaApp> {
    let mut harness = window();
    harness
        .state_mut()
        .set_tabular(&"items".parse().unwrap(), true);
    harness.run();
    settle(&mut harness);
    harness
}

fn skus(harness: &Harness<'_, GossiaApp>) -> Vec<String> {
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
fn header_menu(harness: &mut Harness<'_, GossiaApp>, column: &str, item: &str) {
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
    // The quantities typed in the items would be erased, so it asks first.
    let question = harness.state().replace_inside.as_ref().unwrap().question();
    assert!(question.contains("3 values"), "{question}");
    click(&mut harness, "Yes");
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
fn a_range_is_selected_with_shift_copied_and_pasted_as_one_undo() {
    let mut harness = table_window();
    click(&mut harness, "A-100");
    harness
        .get_by_label("gift wrap")
        .click_modifiers(Modifiers::SHIFT);
    harness.run();
    let range = harness.state().table_range().unwrap();
    assert_eq!((range.rows, range.columns), (0..=1, 0..=4));
    harness.event(egui::Event::Copy);
    // The copy is taken in one frame and handed to the clipboard in the next,
    // whose output carries it.
    harness.step();
    harness.step();
    let expected = "A-100\t2\t9.5\t19.0\t\nB-220\t1\t24.0\t24.0\tgift wrap\n";
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == expected));
    assert!(copied, "the values of the range are put on the clipboard");

    // Shift and the arrows grow the range too.
    click(&mut harness, "12");
    harness.key_combination_modifiers(Modifiers::SHIFT, &[Key::ArrowRight]);
    harness.run();
    let range = harness.state().table_range().unwrap();
    assert_eq!((range.rows, range.columns), (2..=2, 1..=2));
    harness.event(egui::Event::Paste("3\t1.25".to_owned()));
    settle(&mut harness);
    // 3 * 1.25.
    harness.get_by_label("3.75");
    ctrl(&mut harness, Key::Z);
    settle(&mut harness);
    // 12 * 0.75, both cells back in one undo.
    harness.get_by_label("9.0");
}

#[test]
fn the_cell_menu_copies_formulas_and_ctrl_shift_v_pastes_values() {
    let mut harness = table_window();
    harness.state_mut().tables.stand_in = Some(String::new());
    click(&mut harness, "A-100");
    // A right click outside the selection selects the cell first.
    harness.get_by_label("19.0").click_secondary();
    harness.run();
    // A menu item's label ends with its keys.
    harness.get_by_label_contains("Copy formulas").click();
    harness.run();
    assert_eq!(
        harness.state().tables.stand_in.as_deref(),
        Some("=parent.qty * parent.price\n")
    );
    harness.get_by_label("19.0").click_secondary();
    harness.run();
    harness.get_by_label_contains("Ctrl+C").click();
    harness.run();
    assert_eq!(harness.state().tables.stand_in.as_deref(), Some("19.0\n"));
    click(&mut harness, "0.75");
    harness.key_combination_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, &[Key::V]);
    settle(&mut harness);
    // 12 * 19.0: the value was pasted, not the formula.
    harness.get_by_label("228.0");
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
    click(&mut harness, "Orders_schema");
    harness.get_by_label("Used by: Orders");
    assert!(!harness.state().calc().auto_update("Orders_schema"));
    click(&mut harness, "Auto-update");
    assert!(harness.state().calc().auto_update("Orders_schema"));
    click(&mut harness, "Auto-update");
    assert!(!harness.state().calc().auto_update("Orders_schema"));
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
    harness.state_mut().set_auto_update("Orders_schema", true);
    harness.run();
    harness.get_by_label("qty").click_secondary();
    harness.run();
    click(&mut harness, "Rename field...");
    harness.get_by_label("Rename field qty");
}

#[test]
fn a_field_is_renamed_from_the_menu_of_its_column_and_the_table_follows() {
    let mut harness = table_window();
    harness.state_mut().set_auto_update("Orders_schema", true);
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
    harness.state_mut().set_auto_update("Orders_schema", true);
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
    // `GOSSIA_THEME=light` for the light theme; dark otherwise.
    let theme = match std::env::var("GOSSIA_THEME").as_deref() {
        Ok("light") => egui::Theme::Light,
        _ => egui::Theme::Dark,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_theme(theme)
        .wgpu()
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    click(&mut harness, "items");
    press(&mut harness, Key::ArrowRight);
    let image = harness.render().expect("rendered");
    image
        .save(std::env::var("GOSSIA_PICTURE").unwrap_or("window.png".into()))
        .expect("saved");
}

#[test]
fn view_font_sets_the_font_of_the_data_tree_of_every_tab() {
    use gossia_app::style::{DataFont, Typeface};
    let mut harness = window();
    let before = harness.get_by_label("tax_rate").rect().height();
    click(&mut harness, "View");
    click(&mut harness, "Font");
    click(&mut harness, "Serif (Source Serif 4)");
    click(&mut harness, "View");
    click(&mut harness, "Font");
    click(&mut harness, "24 pt");
    assert_eq!(
        harness.state().data_font,
        DataFont {
            family: Typeface::Serif,
            size: 24
        }
    );
    assert!(harness.get_by_label("tax_rate").rect().height() > before * 1.4);
    // It belongs to the workbook, not the tab: another tab keeps it.
    click(&mut harness, "Prices");
    assert_eq!(harness.state().data_font.size, 24);
    click(&mut harness, "View");
    click(&mut harness, "Font");
    click(&mut harness, "Default font");
    assert_eq!(harness.state().data_font, DataFont::default());
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
    // The window starts as the application does, with a blank workbook.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(
            |ui, app: &mut GossiaApp| app.show(ui),
            GossiaApp::new(gossia_app::workbook_file::blank_workbook()),
        );
    settle(&mut harness);
    common::edit_and_recalculate(&mut |cmd, args| {
        let result = gossia_app::control::run(harness.state_mut(), cmd, &args);
        harness.run();
        result
    });
    assert!(has(&harness, "Ready"));
}

/// `drop` with `files` drops them as the browser does: given by name and
/// text, imported as dropped files are, and a workbook refused.
#[test]
fn the_drop_command_takes_files_as_text_as_the_page_drops_them() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(
            |ui, app: &mut GossiaApp| app.show(ui),
            GossiaApp::new(gossia_app::workbook_file::blank_workbook()),
        );
    settle(&mut harness);
    let mut send = |cmd: &str, args: serde_json::Value| {
        let result = gossia_app::control::run(harness.state_mut(), cmd, &args);
        harness.run();
        result
    };
    let dropped = send(
        "drop",
        serde_json::json!({ "files": [{ "name": "Prices.json", "text": "{\"a\": 1.5}" }] }),
    )
    .unwrap();
    assert_eq!(dropped["tabs"], serde_json::json!(["Prices"]));
    send("tab", serde_json::json!({ "name": "Prices" })).unwrap();
    let cell = send("cell", serde_json::json!({ "path": "a" })).unwrap();
    assert_eq!(cell["value"], serde_json::json!(1.5));
    let refused = send(
        "drop",
        serde_json::json!({ "files": [{ "name": "Book.gossia", "text": "" }] }),
    )
    .unwrap_err();
    assert!(refused.contains("is a workbook"), "{refused}");
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
fn control(harness: &mut Harness<'_, GossiaApp>, cmd: &str, args: serde_json::Value) {
    gossia_app::control::run(harness.state_mut(), cmd, &args).unwrap();
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

/// Renaming a tab from its context menu (DESIGN.md, "Renaming a tab"): the
/// name is selected, so typing replaces it, and Enter renames.
#[test]
fn a_tab_is_renamed_from_its_context_menu() {
    let mut harness = window();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, "Orders")
        .click_secondary();
    harness.run();
    click(&mut harness, "Rename tab...");
    harness.get_by_label("Rename tab Orders");
    harness.get_by_label("Tab name").type_text("SUM");
    harness.run();
    assert!(has(
        &harness,
        "SUM cannot be a tab name: it is the name of a built-in function or of a special name in formulas"
    ));
    harness.get_by_label("Tab name").type_text("_list");
    harness.run();
    harness.get_by_label(
        "3 formulas use Orders. They are not changed, and will show #NAME! until they are edited.",
    );
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    let names: Vec<String> = harness
        .state()
        .calc()
        .sheet_names()
        .map(str::to_owned)
        .collect();
    assert_eq!(names[0], "SUM_list");
    assert!(!has(&harness, "Rename tab Orders"));
    harness.get_by_role_and_label(egui::accesskit::Role::Tab, "SUM_list");
}

/// The `+` after the tabs makes a tab with its own schema, and the context
/// menu of a tab deletes it again.
#[test]
fn a_tab_is_added_with_the_plus_and_deleted_from_its_menu() {
    let mut harness = window();
    click(&mut harness, "New tab");
    harness.get_by_label("New tab name").type_text("Notes");
    harness.run();
    click(&mut harness, "Create");
    settle(&mut harness);
    assert_eq!(harness.state().sheet, "Notes");
    assert_eq!(
        harness.state().calc().schema_of("Notes"),
        Some("DataSchema")
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, "Notes")
        .click_secondary();
    harness.run();
    click(&mut harness, "Delete tab...");
    harness.get_by_label("Delete Notes with its data, formulas and forms? Undo puts it back.");
    click(&mut harness, "Delete");
    settle(&mut harness);
    assert!(harness.state().calc().data("Notes").is_none());
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Tab, "Notes")
            .is_none()
    );
}

#[test]
fn the_options_dialog_changes_a_workbook_setting_on_ok() {
    let mut harness = window();
    click(&mut harness, "Tools");
    click(&mut harness, "Options...");
    harness.get_by_label("Gossia Options");
    click(&mut harness, "Evaluation");
    harness.get_by_label("Applies to this workbook, and is stored in it.");
    harness.get_by_label("Time limits");
    // "1" becomes "10" seconds.
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Formula time limit")
        .click();
    harness.run();
    press(&mut harness, Key::End);
    type_into_selected_cell(&mut harness, "0");
    assert_eq!(
        harness
            .state()
            .calc()
            .settings()
            .get(gossia_calc::Setting::FormulaTimeLimit),
        1_000,
        "nothing changes before OK"
    );
    click(&mut harness, "OK");
    settle(&mut harness);
    let state = harness.state();
    assert!(
        state.options.is_none(),
        "{:?} {:?}",
        state.options.as_ref().map(|d| d.problems()),
        state.notice
    );
    assert_eq!(
        harness
            .state()
            .calc()
            .settings()
            .get(gossia_calc::Setting::FormulaTimeLimit),
        10_000
    );
}

#[test]
fn every_options_page_fits_the_dialog() {
    let mut harness = window();
    click(&mut harness, "Tools");
    // A trusted workbook in a deep folder, listed on the Trust page.
    let user = &mut harness.state_mut().user;
    user.trust.trust(
        "0123456789abcdef",
        gossia_calc::Fingerprint {
            hash: "h".to_owned(),
            items: Default::default(),
        },
    );
    user.trust_files.insert(
        "0123456789abcdef".to_owned(),
        std::path::PathBuf::from(format!(
            "/{}book.gossia",
            "a very long folder name/".repeat(8)
        )),
    );
    click(&mut harness, "Options...");
    let mut widths = Vec::new();
    for page in gossia_app::options::Page::ALL {
        harness.state_mut().options.as_mut().unwrap().page = page;
        harness.run();
        let width = harness.get_by_label("Gossia Options").rect().width();
        widths.push((page, width.round()));
    }
    // A page wider than the dialog made the window wider than its title bar.
    // Content wider than the space inside the frame also widened the frame past
    // the title bar, leaving a strip beside it: the window keeps its 740 points.
    assert!(
        widths.iter().all(|(_, w)| *w == 740.0),
        "every page should be as wide as the dialog: {widths:?}"
    );
}

/// Formulas, "A formula over what is inside": a formula on a node that holds
/// values or formulas asks first. No keeps them and drops the formula; Yes
/// enters it and erases them, and undo brings them back.
#[test]
fn a_formula_that_would_erase_what_is_inside_asks_first() {
    let mut harness = window();
    let items: gossia_core::Path = "items".parse().unwrap();
    let enter = |harness: &mut Harness<'_, GossiaApp>| {
        let app = harness.state_mut();
        assert!(app.begin_editing(&items, true, Some("=[]".to_owned())));
        assert!(!app.commit_editing(), "it waits for the answer");
        harness.run();
    };
    let formula = |harness: &Harness<'_, GossiaApp>| {
        harness
            .state()
            .calc()
            .formula_text("Orders", &"items".parse().unwrap())
    };

    enter(&mut harness);
    assert!(has(&harness, "Erase what is inside?"));
    let question = harness.state().replace_inside.as_ref().unwrap().question();
    assert!(question.contains("Orders.items[0].total"), "{question}");
    assert!(question.contains("values"), "{question}");
    click(&mut harness, "No");
    assert!(!has(&harness, "Erase what is inside?"));
    assert!(harness.state().editing.is_none());
    assert_eq!(formula(&harness), None);
    assert!(
        harness
            .state()
            .calc()
            .column_formula("Orders", &"items".parse().unwrap(), "total")
            .is_some()
    );

    enter(&mut harness);
    click(&mut harness, "Yes");
    settle(&mut harness);
    assert!(!has(&harness, "Erase what is inside?"));
    assert_eq!(formula(&harness).as_deref(), Some("[]"));
    assert!(harness.state().calc().column_formulas("Orders").is_empty());

    ctrl(&mut harness, Key::Z);
    settle(&mut harness);
    assert_eq!(formula(&harness), None);
    assert!(
        harness
            .state()
            .calc()
            .column_formula("Orders", &"items".parse().unwrap(), "total")
            .is_some()
    );

    // A formula on an empty node, or a plain value, does not ask.
    let app = harness.state_mut();
    let rate: gossia_core::Path = "tax_rate".parse().unwrap();
    assert!(app.begin_editing(&rate, true, Some("=0.1".to_owned())));
    assert!(app.commit_editing());
}

/// The Display options change the tree grid: the indentation, the height of
/// the rows, and whether the Type and Formula columns are there.
#[test]
fn the_display_options_change_the_tree_grid() {
    let mut harness = window();
    let formula = "=round(sum(Orders.items.total), 2)";
    assert!(has(&harness, "Type") && has(&harness, "Formula") && has(&harness, formula));
    let left = |h: &Harness<'_, GossiaApp>| h.get_by_label("customer").rect().left();
    let step = |h: &Harness<'_, GossiaApp>| {
        h.get_by_label("items").rect().top() - h.get_by_label("customer").rect().top()
    };
    let (left_before, normal) = (left(&harness), step(&harness));

    harness.state_mut().user.prefs.display.indent = 40;
    harness.state_mut().user.prefs.display.density = gossia_app::prefs::Density::Compact;
    harness.run();
    assert!(
        (left(&harness) - left_before - 24.0).abs() < 0.5,
        "one level deeper by 24 px"
    );
    let compact = step(&harness);
    harness.state_mut().user.prefs.display.density = gossia_app::prefs::Density::Comfortable;
    harness.run();
    let comfortable = step(&harness);
    assert!(
        compact < normal && normal < comfortable,
        "{compact} {normal} {comfortable}"
    );

    harness.state_mut().user.prefs.display.type_column = false;
    harness.state_mut().user.prefs.display.formula_column = false;
    harness.run();
    assert!(!has(&harness, "Type") && !has(&harness, "Formula") && !has(&harness, formula));
    harness.get_by_label("Value");
    harness.get_by_label("62.92");
}

/// A tab dragged along the bar goes where it is let go; undo puts it back.
#[test]
fn a_tab_is_moved_by_dragging_it() {
    let mut harness = window();
    let order = |harness: &Harness<'_, GossiaApp>| {
        harness
            .state()
            .calc()
            .sheet_names()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let before = order(&harness);
    let tab = |harness: &Harness<'_, GossiaApp>, name: &str| {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Tab, name)
            .rect()
    };
    // The first tab, dragged past the middle of the third.
    let from = tab(&harness, &before[0]).center();
    let to = tab(&harness, &before[2]).center() + egui::vec2(10.0, 0.0);
    harness.hover_at(from);
    harness.run();
    harness.drag_at(from);
    harness.run();
    for step in 1..=5 {
        harness.hover_at(from.lerp(to, step as f32 / 5.0));
        harness.run();
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run();
    harness.remove_cursor();
    harness.run();
    let after = order(&harness);
    assert_eq!(
        after,
        [&before[1], &before[2], &before[0]]
            .into_iter()
            .chain(&before[3..])
            .cloned()
            .collect::<Vec<_>>()
    );
    ctrl(&mut harness, Key::Z);
    assert_eq!(order(&harness), before);
}

#[test]
fn a_place_on_another_tab_is_picked_into_the_formula_where_the_cursor_was() {
    let mut harness = window();
    click(&mut harness, "0.21");
    click_bar(&mut harness);
    select_all(&mut harness);
    harness.get_by_label("Formula bar").type_text("=len(");
    harness.run();
    click(&mut harness, "Pick a place");
    assert!(harness.state().picking());
    harness.get_by_label(gossia_app::pick::PICK_HINT);
    click(&mut harness, "Prices");
    click(&mut harness, "EUR");
    // Back on the tab of the formula, with the address put in.
    assert!(!harness.state().picking());
    assert_eq!(harness.state().sheet, "Orders");
    assert_eq!(bar(&harness), "=len(Prices.currency");
    harness.get_by_label("Formula bar").type_text(") / 100");
    harness.run();
    press(&mut harness, Key::Enter);
    settle(&mut harness);
    assert_eq!(
        harness
            .state()
            .calc()
            .formula_text("Orders", &"tax_rate".parse().unwrap())
            .as_deref(),
        Some("len(Prices.currency) / 100")
    );
    harness.get_by_label("0.03");
}

#[test]
fn a_place_on_the_same_tab_is_picked_and_esc_stops_picking() {
    let mut harness = window();
    click(&mut harness, "0.21");
    click_bar(&mut harness);
    select_all(&mut harness);
    harness.get_by_label("Formula bar").type_text("=1 + ");
    harness.run();
    click(&mut harness, "Pick a place");
    click(&mut harness, "52.0");
    assert_eq!(bar(&harness), "=1 + Orders.subtotal");
    // Picking again, then Esc: the text is as it was, still being typed.
    click(&mut harness, "Pick a place");
    press(&mut harness, Key::Escape);
    assert!(!harness.state().picking());
    assert_eq!(bar(&harness), "=1 + Orders.subtotal");
    assert!(harness.state().editing.is_some());
}

#[test]
fn a_click_anywhere_in_a_row_of_the_tree_selects_it() {
    let mut harness = window();
    // Right of the name, in the empty Formula column of the row.
    let name = harness.get_by_label("customer").rect();
    let at = egui::pos2(1000.0, name.center().y);
    harness.hover_at(at);
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.run();
    }
    assert_eq!(harness.state().selection, Some("customer".parse().unwrap()));
    harness.get_by_label("Orders.customer");
}

/// *New tab...* makes a report when Report is chosen; the report's tab has
/// its own menu, which duplicates and deletes it.
#[test]
fn a_report_is_made_from_new_tab_and_duplicated_and_deleted_from_its_menu() {
    let mut harness = window();
    click(&mut harness, "New tab");
    click(&mut harness, "Report");
    // The name offered follows the kind, until one is typed.
    assert_eq!(
        harness.get_by_label("New tab name").value().as_deref(),
        Some("Report")
    );
    click(&mut harness, "Create");
    settle(&mut harness);
    assert_eq!(harness.state().sheet, "Report");
    assert!(harness.state().calc().is_report("Report"));
    harness.get_by_label("This report has no controls yet.");
    // A report has no formula bar.
    assert!(!has(&harness, "Formula bar"));

    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, "Report")
        .click_secondary();
    harness.run();
    click(&mut harness, "Duplicate");
    settle(&mut harness);
    assert_eq!(harness.state().sheet, "Report_2");

    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, "Report_2")
        .click_secondary();
    harness.run();
    click(&mut harness, "Delete tab...");
    harness.get_by_label("Delete the report Report_2 with its design? Undo puts it back.");
    click(&mut harness, "Delete");
    settle(&mut harness);
    assert!(!harness.state().calc().is_report("Report_2"));
    assert_eq!(harness.state().sheet, "Report");
}

/// An array field added to a blank tab is not in the data until it has an
/// item: Add item makes it, holding that item, as one thing to undo. The
/// array's menu renames and deletes it but adds no field; its item's does.
#[test]
fn add_item_makes_an_array_just_added_and_the_array_adds_no_field() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(
            |ui, app: &mut GossiaApp| app.show(ui),
            GossiaApp::new(gossia_app::workbook_file::blank_workbook()),
        );
    settle(&mut harness);
    harness
        .state_mut()
        .session
        .calc_mut()
        .add_field(
            "Data",
            &gossia_core::Path::default(),
            "inventory",
            gossia_calc::FieldKind::Array,
            false,
        )
        .unwrap();
    harness.state_mut().refresh_rows();
    settle(&mut harness);

    harness.get_by_label("inventory").click_secondary();
    harness.run();
    // Not Add field (the items have the fields), but the array is a field
    // of Data, renamed and deleted like any other.
    assert!(harness.query_by_label("Add field...").is_none());
    assert!(harness.query_by_label("Rename field...").is_some());
    assert!(harness.query_by_label("Delete field...").is_some());
    click(&mut harness, "Add item");
    settle(&mut harness);
    assert_eq!(harness.state().notice, None);
    assert_eq!(data_text(&harness), r#"{"inventory":[{}]}"#);

    // The item is a record: its menu adds fields to the items.
    harness.get_by_label("[0]").click_secondary();
    harness.run();
    assert!(harness.query_by_label("Add field...").is_some());
    harness.key_press(Key::Escape);
    harness.run();

    harness.state_mut().session.calc_mut().undo().unwrap();
    assert_eq!(data_text(&harness), "{}");
}

fn data_text(harness: &Harness<'_, GossiaApp>) -> String {
    let data = harness.state().calc().data("Data").unwrap();
    gossia_core::json::to_string(data, false).unwrap()
}

/// *Go to schema definition* on a node's menu shows the schema tab, with the
/// field's entry selected; a schema tab's own nodes do not offer it.
#[test]
fn go_to_schema_definition_shows_the_field_on_its_schema_tab() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(
            |ui, app: &mut GossiaApp| app.show(ui),
            GossiaApp::new(gossia_app::workbook_file::blank_workbook()),
        );
    settle(&mut harness);
    let root = gossia_core::Path::default();
    let schema = harness.state().calc().schema_of("Data").unwrap().to_owned();
    harness
        .state_mut()
        .session
        .calc_mut()
        .add_field(
            "Data",
            &root,
            "price",
            gossia_calc::FieldKind::Number,
            false,
        )
        .unwrap();
    harness.state_mut().refresh_rows();
    settle(&mut harness);

    harness.get_by_label("price").click_secondary();
    harness.run();
    click(&mut harness, "Go to schema definition");
    settle(&mut harness);
    assert_eq!(harness.state().sheet, schema);
    assert_eq!(
        harness.state().selection.as_ref().map(ToString::to_string),
        Some("properties.price".to_owned())
    );

    // On the schema tab itself it is not offered.
    harness.get_by_label("price").click_secondary();
    harness.run();
    assert!(harness.query_by_label("Go to schema definition").is_none());
}

/// A field whose records have no fields yet, with a formula that returns
/// records, is `#TYPE!`; *Force object definition* on its menu defines the
/// fields from the result, which the node then holds.
#[test]
fn force_object_definition_defines_the_fields_from_the_formula_result() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(
            |ui, app: &mut GossiaApp| app.show(ui),
            GossiaApp::new(gossia_app::workbook_file::blank_workbook()),
        );
    settle(&mut harness);
    let root = gossia_core::Path::default();
    let inventory: gossia_core::Path = "inventory".parse().unwrap();
    let calc = harness.state_mut().session.calc_mut();
    calc.add_field(
        "Data",
        &root,
        "inventory",
        gossia_calc::FieldKind::Array,
        false,
    )
    .unwrap();
    calc.set_formula(
        "Data",
        &inventory,
        r#"[{"sku": "A1", "qty": 2}, {"sku": "B2", "qty": 5}]"#,
    )
    .unwrap();
    harness.state_mut().refresh_rows();
    settle(&mut harness);
    assert!(has(&harness, "#TYPE!"));

    harness.get_by_label("inventory").click_secondary();
    harness.run();
    click(&mut harness, "Force object definition");
    settle(&mut harness);
    assert_eq!(harness.state().notice, None);
    assert!(!has(&harness, "#TYPE!"));
    assert_eq!(
        data_text(&harness),
        r#"{"inventory":[{"sku":"A1","qty":2},{"sku":"B2","qty":5}]}"#
    );
    let fields = harness
        .state()
        .calc()
        .field_places("Data", &"inventory[0]".parse().unwrap())
        .unwrap()
        .fields;
    assert_eq!(fields, ["sku", "qty"]);
    // Once the type fits, the item is not offered again.
    harness.get_by_label("inventory").click_secondary();
    harness.run();
    assert!(harness.query_by_label("Force object definition").is_none());
}

/// The button at the end of the formula bar expands it, to show a formula of
/// several lines whole, while it is shown and while it is edited; pressed
/// again, it collapses the bar to one line.
#[test]
fn the_formula_bar_expands_to_show_several_lines_and_collapses_again() {
    let mut harness = window();
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_formula(
            "Orders",
            &"tax_rate".parse().unwrap(),
            "min(\n    0.5,\n    0.25,\n    0.75,\n)",
        )
        .unwrap();
    settle(&mut harness);
    harness.get_by_label("tax_rate").click();
    harness.run();
    let one_line = harness.get_by_label("Formula bar").rect().height();

    click(&mut harness, "Expand formula bar");
    let expanded = harness.get_by_label("Formula bar").rect().height();
    assert!(expanded > one_line * 3.0, "{one_line} {expanded}");
    assert!(harness.state().formula_bar_height.is_some());

    // Editing keeps the height.
    click_bar(&mut harness);
    assert!(harness.state().editing.is_some());
    let editing = harness.get_by_label("Formula bar").rect().height();
    assert!((editing - expanded).abs() < 2.0, "{expanded} {editing}");
    press(&mut harness, Key::Escape);

    click(&mut harness, "Collapse formula bar");
    assert!(harness.state().formula_bar_height.is_none());
    let collapsed = harness.get_by_label("Formula bar").rect().height();
    assert!((collapsed - one_line).abs() < 2.0, "{one_line} {collapsed}");
}

/// On the root, with no fields yet, a formula that returns a record is
/// `#TYPE!`; *Force object definition* on the root's menu gives the root
/// those fields. The menu opens from anywhere on the row: here its type.
#[test]
fn force_object_definition_on_the_root_from_a_right_click_on_its_type() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(
            |ui, app: &mut GossiaApp| app.show(ui),
            GossiaApp::new(gossia_app::workbook_file::blank_workbook()),
        );
    settle(&mut harness);
    let root = gossia_core::Path::default();
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_formula("Data", &root, r#"{"sku": "A1", "qty": 2}"#)
        .unwrap();
    harness.state_mut().refresh_rows();
    settle(&mut harness);
    assert!(has(&harness, "#TYPE!"));

    let ty = harness.state().rows[0].type_label.clone();
    harness.get_by_label(&ty).click_secondary();
    harness.run();
    click(&mut harness, "Force object definition");
    settle(&mut harness);
    assert_eq!(harness.state().notice, None);
    assert_eq!(data_text(&harness), r#"{"sku":"A1","qty":2}"#);
    let fields = harness
        .state()
        .calc()
        .field_places("Data", &root)
        .unwrap()
        .fields;
    assert_eq!(fields, ["sku", "qty"]);
}
