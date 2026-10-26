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

/// Chooses a view of the current tab: View, Show as, the view.
fn show_as(harness: &mut Harness<'_, ClioApp>, view: &str) {
    harness.get_by_label("View").click();
    harness.run();
    harness.get_by_label("Show as").click();
    harness.run();
    harness.get_by_label(view).click();
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
    show_as(&mut harness, "Default");
    settle(&mut harness);
    harness
}

#[test]
fn the_view_selector_switches_between_the_tree_and_the_default_form() {
    let mut harness = window();
    assert!(harness.state().current_view() == &View::Tree);
    assert!(has(&harness, "Formula bar"));
    show_as(&mut harness, "Default");
    assert!(harness.state().in_form());
    // A form has no formula bar, and shows the fields with their values.
    assert!(!has(&harness, "Formula bar"));
    harness.get_by_label("Orders.customer.name");
    show_as(&mut harness, "Data tree");
    assert!(!harness.state().in_form());
    assert!(has(&harness, "Formula bar"));
}

#[test]
fn each_tab_remembers_its_view() {
    let mut harness = window();
    show_as(&mut harness, "Default");
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
    show_as(&mut harness, "Data tree");
    assert!(harness.state().in_form());
    // Correcting it lets the value in, and the view can change.
    type_into(&mut harness, "Orders.tax_rate", "0.5");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "tax_rate"),
        Some(Value::Float(0.5))
    );
    show_as(&mut harness, "Data tree");
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

/// Opens the context menu of the table row whose header reads `row` and
/// chooses `item`.
fn row_menu(harness: &mut Harness<'_, ClioApp>, row: &str, item: &str) {
    harness.get_by_label(row).click_secondary();
    harness.run();
    harness.get_by_label(item).click();
    harness.run();
    settle(harness);
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
    // A row is deleted from its context menu.
    row_menu(&mut harness, "[3]", "Delete row");
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
    row_menu(&mut harness, "[0]", "Move down");
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
    show_as(&mut harness, "Default");
    harness.get_by_label("Prices.list[\"A-100\"]");
    harness.get_by_label("New key of Prices.list").click();
    harness
        .get_by_label("New key of Prices.list")
        .type_text("E-500");
    harness.run();
    click(&mut harness, "Add entry");
    settle(&mut harness);
    assert!(value(&harness, "Prices", "list[\"E-500\"]").is_some());
    harness
        .get_all_by_label("Delete entry")
        .next()
        .unwrap()
        .click();
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
    show_as(&mut harness, "Data tree");
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

#[test]
fn the_raw_view_shows_the_document_and_a_paste_replaces_it() {
    let mut harness = window();
    show_as(&mut harness, "Raw");
    assert!(!has(&harness, "Formula bar"));
    let shown = harness
        .get_by_label("Raw document")
        .value()
        .unwrap_or_default();
    // The pretty-printed JSON of the tab, with its calculated values.
    assert!(shown.contains("\"name\": \"Acme Ltd\""), "{shown}");
    assert!(shown.contains("\"total\": 62.92"), "{shown}");
    assert!(shown.contains('\n'));
    // A document that conforms is accepted, as one undoable edit.
    let mut doc = shown.replace("Acme Ltd", "Globex");
    doc = doc.replace("\"tax_rate\": 0.21", "\"tax_rate\": 0.5");
    harness.event(egui::Event::Paste(doc));
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "customer.name"),
        Some(Value::Str("Globex".into()))
    );
    assert_eq!(value(&harness, "Orders", "total"), Some(Value::Float(78.0)));
    assert_eq!(harness.state().calc().history_len().0, 1);
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "Orders", "customer.name"),
        Some(Value::Str("Acme Ltd".into()))
    );
}

#[test]
fn a_paste_that_does_not_conform_is_refused_with_the_reasons() {
    let mut harness = window();
    show_as(&mut harness, "Raw");
    let before = harness
        .get_by_label("Raw document")
        .value()
        .unwrap_or_default();
    // Valid JSON, but the tax rate is above its maximum and `items` is gone.
    harness.event(egui::Event::Paste(
        r#"{"customer": {"name": "X", "status": "paid"}, "tax_rate": 7}"#.to_owned(),
    ));
    harness.run();
    assert!(
        harness
            .query_all_by_label_contains("does not conform")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label_contains("maximum")
            .next()
            .is_some()
    );
    // Text that is not JSON at all.
    harness.event(egui::Event::Paste("{not json".to_owned()));
    harness.run();
    assert!(
        harness
            .query_all_by_label_contains("not valid JSON")
            .next()
            .is_some()
    );
    // Nothing changed.
    assert_eq!(harness.state().calc().history_len().0, 0);
    assert_eq!(
        harness
            .get_by_label("Raw document")
            .value()
            .unwrap_or_default(),
        before
    );
}

#[test]
fn the_raw_view_of_an_xml_tab_is_xml_and_a_paste_is_checked_against_the_xsd() {
    use cliotree_core::{metaschema, xml, xsd};
    const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order"><xs:complexType><xs:sequence>
    <xs:element name="customer" type="xs:string"/>
    <xs:element name="qty" type="xs:integer"/>
  </xs:sequence></xs:complexType></xs:element></xs:schema>"#;
    let mut calc =
        cliotree_calc::Calc::new(std::sync::Arc::new(cliotree_calc::python::PythonSyntax));
    let schema = xml::parse(XSD, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet("S", schema.node, schema.document)
        .unwrap();
    let spec = xsd::lower_str(XSD).unwrap();
    let data = xml::parse(
        "<order><customer>Acme</customer><qty>2</qty></order>",
        &spec,
    )
    .unwrap();
    calc.add_sheet("Order", spec, data.node).unwrap();
    calc.set_xml_document("Order", data.document).unwrap();
    calc.bind_schema("Order", "S").unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::new(calc));
    settle(&mut harness);
    click(&mut harness, "Order");
    show_as(&mut harness, "Raw");
    let shown = harness
        .get_by_label("Raw document")
        .value()
        .unwrap_or_default();
    assert!(shown.contains("<customer>Acme</customer>"), "{shown}");
    let app = harness.state_mut();
    let why = app
        .paste_document("<order><customer>X</customer><qty>two</qty></order>")
        .unwrap_err();
    assert!(
        why.contains("does not conform") && why.contains("qty"),
        "{why}"
    );
    app.paste_document("<order><customer>Globex</customer><qty>5</qty></order>")
        .unwrap();
    assert_eq!(
        value(&harness, "Order", "customer"),
        Some(Value::Str("Globex".into()))
    );
}

// --------------------------------------------------- named forms, layout mode

fn new_form(harness: &mut Harness<'_, ClioApp>, name: &str) {
    harness.get_by_label("View").click();
    harness.run();
    harness.get_by_label("Forms").click();
    harness.run();
    harness.get_by_label("New form...").click();
    harness.run();
    harness.get_by_label("Form name").click();
    harness.run();
    harness.get_by_label("Form name").type_text(name);
    harness.run();
    click(harness, "Create");
    settle(harness);
}

fn edit_form(harness: &mut Harness<'_, ClioApp>) {
    harness.get_by_label("Edit form").click();
    harness.run();
}

#[test]
fn a_new_form_is_made_from_the_view_menu_and_shown() {
    let mut harness = form();
    new_form(&mut harness, "Packing");
    assert_eq!(harness.state().form_name(), "Packing");
    assert!(harness.state().in_form());
    harness.get_by_label("Orders.customer.name");
    // It is offered by the view selector.
    show_as(&mut harness, "Default");
    assert_eq!(harness.state().form_name(), "Default");
    show_as(&mut harness, "Packing");
    assert_eq!(harness.state().form_name(), "Packing");
}

#[test]
fn a_refused_form_name_is_explained_and_the_dialog_stays() {
    let mut harness = form();
    new_form(&mut harness, "two words");
    assert!(has(&harness, "Create"));
    assert!(harness.state().form.dialog.is_some());
    click(&mut harness, "Cancel");
    assert!(harness.state().form.dialog.is_none());
}

#[test]
fn layout_mode_selects_a_control_and_edits_its_label_on_the_property_page() {
    let mut harness = form();
    edit_form(&mut harness);
    assert!(harness.state().layout_active());
    click(&mut harness, "Field customer.name");
    assert_eq!(harness.state().selected_control(), Some(vec![0, 0]));
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Label")
        .click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[Key::A]);
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Label")
        .type_text("Customer");
    harness.run();
    let design = harness.state().design_of("Default");
    assert_eq!(
        design.get(&[0, 0]).unwrap().label.as_deref(),
        Some("Customer")
    );
    assert!(harness.state().is_customised("Default"));
    // Editing a design is not an edit of the workbook.
    assert_eq!(harness.state().calc().history_len().0, 0);
}

#[test]
fn a_control_is_deleted_from_its_context_menu_and_listed_as_not_on_the_form() {
    let mut harness = form();
    edit_form(&mut harness);
    let before = harness.state().design_of("Default").walk().len();
    harness
        .get_by_label("Field customer.name")
        .click_secondary();
    harness.run();
    harness.get_by_label("Delete control").click();
    harness.run();
    assert_eq!(
        harness.state().design_of("Default").walk().len(),
        before - 1
    );
    harness.get_by_label("Add customer.name to the form");
    click(&mut harness, "Add customer.name to the form");
    assert_eq!(harness.state().design_of("Default").walk().len(), before);
}

/// The height of the widget with this label.
fn height(harness: &Harness<'_, ClioApp>, label: &str) -> f32 {
    harness.get_by_label(label).rect().height()
}

#[test]
fn a_control_has_a_font_and_the_controls_of_a_group_inherit_it() {
    use cliotree_core::form::{Control, ControlKind, FontFamily};
    let mut harness = form();
    new_form(&mut harness, "Fonts");
    let label = |text: &str| Control {
        text: Some(text.to_owned()),
        ..Control::new(ControlKind::Label)
    };
    harness.state_mut().edit_design(|d| {
        let mut group = Control::new(ControlKind::Group);
        group.font = Some(FontFamily::Serif);
        group.font_size = Some(28);
        let mut small = label("Small print");
        small.font_size = Some(10);
        group.children = vec![label("Big title"), small];
        d.controls.insert(0, label("Plain text"));
        d.controls.insert(1, group);
        Some(None)
    });
    harness.run();
    harness.run();
    let plain = height(&harness, "Plain text");
    let big = height(&harness, "Big title");
    let small = height(&harness, "Small print");
    assert!(big > plain * 1.5, "inherited 28 pt: {big} against {plain}");
    assert!(small < plain, "its own 10 pt: {small} against {plain}");
}

#[test]
fn the_font_of_a_control_is_chosen_on_the_property_page() {
    use cliotree_core::form::FontFamily;
    let mut harness = form();
    edit_form(&mut harness);
    click(&mut harness, "Field customer.name");
    click(&mut harness, "Font of the control");
    click(&mut harness, "Script (Dancing Script)");
    click(&mut harness, "Font size of the control");
    click(&mut harness, "20 pt");
    let design = harness.state().design_of("Default");
    let control = design.get(&[0, 0]).unwrap();
    assert_eq!(control.font, Some(FontFamily::Script));
    assert_eq!(control.font_size, Some(20));
}

#[test]
fn a_control_whose_field_is_gone_is_a_placeholder_that_can_be_removed() {
    let mut harness = form();
    new_form(&mut harness, "Packing");
    harness.state_mut().edit_design(|d| {
        d.get_mut(&[0, 0])?.path = Some("customer.nickname".to_owned());
        Some(None)
    });
    harness.run();
    assert!(
        harness
            .query_all_by_label_contains("customer.nickname")
            .next()
            .is_some()
    );
    edit_form(&mut harness);
    let name = "Remove Field customer.nickname";
    click(&mut harness, name);
    assert!(!has(&harness, name));
}
