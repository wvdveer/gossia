//! What expressions show on a form, in the window (DESIGN.md, Forms,
//! "Additions in 0.2.0, shared with reports"): a field that shows an
//! expression, a label that is a template, and repeaters whose items an
//! expression gives, as a table and one at a time.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::form::{ColumnDesign, Control, ControlKind, RepeaterMode};
use cliotree_core::{Path, Value};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn settle(harness: &mut Harness<'_, ClioApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60)),
        "the workbook did not settle"
    );
    harness.run();
    harness.run();
}

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

/// The Default form of Orders with these controls at its end.
fn form(controls: Vec<Control>) -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1800.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        design.controls.extend(controls);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness
}

fn value_of(harness: &Harness<'_, ClioApp>, label: &str) -> Option<String> {
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .accesskit_node()
        .value()
}

fn lines_field() -> Control {
    let mut field = Control::new(ControlKind::Field);
    field.label = Some("Line count".into());
    field.value = Some("len(Orders.items)".into());
    field
}

fn summary_label() -> Control {
    let mut label = Control::new(ControlKind::Label);
    label.template = true;
    label.text = Some("{Orders.customer.name} ordered {len(Orders.items)} lines".into());
    label
}

#[test]
fn a_value_field_and_a_template_follow_the_data() {
    let mut harness = form(vec![lines_field(), summary_label()]);
    assert_eq!(value_of(&harness, "Line count").as_deref(), Some("3"));
    assert!(harness.query_by_label("Acme Ltd ordered 3 lines").is_some());
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_value("Orders", &p("customer.name"), Value::Str("Ada".into()))
        .unwrap();
    settle(&mut harness);
    assert!(harness.query_by_label("Ada ordered 3 lines").is_some());
}

#[test]
fn an_untrusted_workbook_shows_marks_in_place_of_results() {
    let mut harness = form(vec![lines_field(), summary_label()]);
    harness.state_mut().session.calc_mut().set_trusted(false);
    harness.run();
    assert_eq!(
        value_of(&harness, "Line count").as_deref(),
        Some("#UNTRUSTED!")
    );
    assert!(
        harness
            .query_by_label("#UNTRUSTED! ordered #UNTRUSTED! lines")
            .is_some()
    );
}

#[test]
fn a_table_over_an_expression_shows_its_fields_and_computed_columns() {
    let mut big = Control::new(ControlKind::Repeater);
    big.label = Some("Big lines".into());
    big.items = Some("[i for i in Orders.items if i.qty > 1]".into());
    big.item_name = Some("line".into());
    let mut total = ColumnDesign::named("amount");
    total.label = Some("Amount".into());
    total.value = Some("f'{line.qty * line.price:.2f} EUR'".into());
    big.table_columns = vec![ColumnDesign::named("sku"), total];
    let harness = form(vec![big]);
    assert!(harness.query_by_label("Big lines").is_some());
    assert!(harness.query_by_label("2 items").is_some());
    assert!(harness.query_by_label("Amount").is_some());
    assert!(harness.query_by_label("A-100").is_some());
    assert!(harness.query_by_label("C-310").is_some());
    assert!(harness.query_by_label("B-220").is_none(), "one is ordered");
    assert!(harness.query_by_label("19.00 EUR").is_some(), "2 x 9.5");
    assert!(harness.query_by_label("9.00 EUR").is_some(), "12 x 0.75");
}

#[test]
fn stacked_items_of_an_expression_step_and_show_their_design() {
    let mut sku = Control::new(ControlKind::Field);
    sku.label = Some("Code".into());
    sku.value = Some("f'{line.sku} of {len(lines)}'".into());
    let mut stacked = Control::new(ControlKind::Repeater);
    stacked.mode = Some(RepeaterMode::Stacked);
    stacked.label = Some("Each line".into());
    stacked.items = Some("Orders.items".into());
    stacked.item_name = Some("line".into());
    stacked.list_name = Some("lines".into());
    stacked.children = vec![sku];
    let mut harness = form(vec![stacked]);
    assert_eq!(value_of(&harness, "Code").as_deref(), Some("A-100 of 3"));
    assert!(harness.query_by_label("1 of 3").is_some());
    // The last Next item button is this repeater's: the form's own stacked
    // repeaters come before it.
    let next = harness.get_all_by_label("Next item").last().unwrap();
    next.click();
    harness.run();
    settle(&mut harness);
    assert_eq!(value_of(&harness, "Code").as_deref(), Some("B-220 of 3"));
}

/// As the owner tried it: a new field in layout mode, given a value on its
/// property page, shows the value and no longer asks for a path.
#[test]
fn a_new_field_given_a_value_on_the_property_page_needs_no_path() {
    let mut harness = form(Vec::new());
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.get_by_label("Add field").click();
    harness.run();
    assert!(
        harness
            .query_all_by_label_contains("a field needs a path")
            .count()
            > 0
    );
    choose_source(&mut harness, "Value");
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, "Value").click();
    harness.run();
    harness
        .get_by_role_and_label(role, "Value")
        .type_text("len(Orders.items)");
    harness.run();
    // Nothing is stored, and nothing is wrong, while it is typed.
    assert_eq!(
        harness
            .state()
            .design_of("Default")
            .walk()
            .iter()
            .filter(|(_, c)| c.value.is_some())
            .count(),
        0
    );
    assert!(harness.query_by_label_contains("syntax error").is_none());
    harness.key_press(egui::Key::Enter);
    harness.run();
    settle(&mut harness);
    let design = harness.state().design_of("Default");
    let field = design
        .walk()
        .into_iter()
        .find(|(_, c)| c.kind == ControlKind::Field && c.path.is_none())
        .map(|(_, c)| c.clone())
        .expect("the new field");
    assert_eq!(field.value.as_deref(), Some("len(Orders.items)"));
    assert!(
        harness
            .query_all_by_label_contains("a field needs a path")
            .count()
            == 0
    );
}

/// Chooses the source of the selected field on its property page.
fn choose_source(harness: &mut Harness<'_, ClioApp>, source: &str) {
    harness.get_by_label("Source of the field").click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, source)
        .click();
    harness.run();
}

const WRITERS: &str = "\
def rename(form, ctx, value):
    form.customer.name = value.strip()

def set_sku(form, ctx, value):
    form.items[ctx.index].sku = value

def set_double(form, ctx, value):
    form.items[ctx.index].qty = int(value) // 2
";

/// A form with the module of write procedures, and these controls at its end.
fn form_with_writers(controls: Vec<Control>) -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1800.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    harness
        .state_mut()
        .session
        .calc_mut()
        .add_module("writers", WRITERS)
        .unwrap();
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        design.controls.extend(controls);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness
}

fn shouting(write: Option<&str>) -> Control {
    let mut field = Control::new(ControlKind::Field);
    field.label = Some("Shouted".into());
    field.value = Some("Orders.customer.name.upper()".into());
    field.write_procedure = write.map(str::to_owned);
    field
}

#[test]
fn a_value_field_with_a_write_procedure_passes_what_is_typed_to_it() {
    let mut harness = form_with_writers(vec![shouting(Some("rename"))]);
    assert_eq!(value_of(&harness, "Shouted").as_deref(), Some("ACME LTD"));
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, "Shouted").click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    harness
        .get_by_role_and_label(role, "Shouted")
        .type_text("  Ada ");
    harness.run();
    // Nothing runs while it is typed into.
    assert_eq!(
        harness
            .state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&p("customer.name"))
            .unwrap()
            .value(),
        &Value::Str("Acme Ltd".into())
    );
    harness.key_press(egui::Key::Enter);
    harness.run();
    settle(&mut harness);
    assert_eq!(
        harness
            .state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&p("customer.name"))
            .unwrap()
            .value(),
        &Value::Str("Ada".into())
    );
    assert_eq!(value_of(&harness, "Shouted").as_deref(), Some("ADA"));
}

#[test]
fn a_value_field_without_a_write_procedure_is_read_only() {
    let harness = form_with_writers(vec![shouting(None)]);
    let node = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Shouted");
    assert!(node.accesskit_node().is_disabled());
    let harness = form_with_writers(vec![shouting(Some("rename"))]);
    let node = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Shouted");
    assert!(!node.accesskit_node().is_disabled());
}

/// A field has a path or a value: choosing one on the property page clears
/// the other, and the write procedure goes with the value.
#[test]
fn the_source_of_a_field_is_a_path_or_a_value() {
    let mut harness = form_with_writers(Vec::new());
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.get_by_label("Field customer.city").click();
    harness.run();
    let at = harness
        .state()
        .design_of("Default")
        .walk()
        .into_iter()
        .find(|(_, c)| c.path.as_deref() == Some("customer.city"))
        .map(|(at, _)| at)
        .unwrap();
    choose_source(&mut harness, "Value");
    let field = harness
        .state()
        .design_of("Default")
        .get(&at)
        .unwrap()
        .clone();
    assert_eq!(field.path, None);
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, "Value").click();
    harness.run();
    harness
        .get_by_role_and_label(role, "Value")
        .type_text("'x'");
    harness.key_press(egui::Key::Enter);
    harness.run();
    harness.get_by_label("Write procedure of the field").click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "rename")
        .click();
    harness.run();
    let field = harness
        .state()
        .design_of("Default")
        .get(&at)
        .unwrap()
        .clone();
    assert_eq!(field.value.as_deref(), Some("'x'"));
    assert_eq!(field.write_procedure.as_deref(), Some("rename"));
    choose_source(&mut harness, "Path");
    let field = harness
        .state()
        .design_of("Default")
        .get(&at)
        .unwrap()
        .clone();
    assert_eq!(field.value, None);
    assert_eq!(field.write_procedure, None);
}

/// A field with a write procedure inside the items of a repeater: the
/// procedure learns which item from `ctx.index`.
#[test]
fn a_write_procedure_in_an_item_knows_its_index() {
    let mut sku = Control::new(ControlKind::Field);
    sku.label = Some("Code".into());
    sku.value = Some("line.sku".into());
    sku.write_procedure = Some("set_sku".into());
    let mut stacked = Control::new(ControlKind::Repeater);
    stacked.mode = Some(RepeaterMode::Stacked);
    stacked.label = Some("Each line".into());
    stacked.items = Some("Orders.items".into());
    stacked.item_name = Some("line".into());
    stacked.children = vec![sku];
    let mut harness = form_with_writers(vec![stacked]);
    let next = harness.get_all_by_label("Next item").last().unwrap();
    next.click();
    harness.run();
    settle(&mut harness);
    assert_eq!(value_of(&harness, "Code").as_deref(), Some("B-220"));
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, "Code").click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    harness.get_by_role_and_label(role, "Code").type_text("Z-1");
    harness.key_press(egui::Key::Enter);
    harness.run();
    settle(&mut harness);
    let data = harness.state().calc().data("Orders").unwrap();
    assert_eq!(
        data.get(&p("items[1].sku")).unwrap().value(),
        &Value::Str("Z-1".into())
    );
    assert_eq!(
        data.get(&p("items[0].sku")).unwrap().value(),
        &Value::Str("A-100".into())
    );
}

/// A column that shows an expression, added to a table bound to an array:
/// each row shows what it comes to, and what is typed into a cell goes to
/// the column's write procedure with `ctx.index` the row.
#[test]
fn a_bound_table_takes_a_column_with_a_value_and_a_write_procedure() {
    let mut harness = form_with_writers(Vec::new());
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        let at = design
            .walk()
            .into_iter()
            .find(|(_, c)| c.kind == ControlKind::Repeater && c.path.as_deref() == Some("items"))
            .map(|(at, _)| at)
            .unwrap();
        let mut double = ColumnDesign::named("double");
        double.label = Some("Twice".into());
        double.value = Some("this.qty * 2".into());
        double.write_procedure = Some("set_double".into());
        design.get_mut(&at).unwrap().table_columns = vec![double];
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
    }
    settle(&mut harness);
    assert_eq!(value_of(&harness, "Twice [0]").as_deref(), Some("4"));
    assert_eq!(value_of(&harness, "Twice [2]").as_deref(), Some("24"));
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, "Twice [1]").click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    harness
        .get_by_role_and_label(role, "Twice [1]")
        .type_text("10");
    harness.key_press(egui::Key::Enter);
    harness.run();
    settle(&mut harness);
    let data = harness.state().calc().data("Orders").unwrap();
    assert_eq!(
        data.get(&p("items[1].qty")).unwrap().value(),
        &Value::Int(5)
    );
    assert_eq!(
        data.get(&p("items[0].qty")).unwrap().value(),
        &Value::Int(2)
    );
    assert_eq!(value_of(&harness, "Twice [1]").as_deref(), Some("10"));
}

/// The property page adds such a column to a table bound to an array, and
/// its page has the value and the write procedure.
#[test]
fn the_property_page_adds_a_column_with_an_expression_to_a_bound_table() {
    let mut harness = form_with_writers(Vec::new());
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.get_by_label("Repeater items").click();
    harness.run();
    harness
        .get_by_label("Add a column with an expression")
        .click();
    harness.run();
    let design = harness.state().design_of("Default");
    let items = design
        .walk()
        .into_iter()
        .find(|(_, c)| c.path.as_deref() == Some("items"))
        .map(|(_, c)| c.clone())
        .unwrap();
    let added = items
        .table_columns
        .iter()
        .find(|c| c.name == "column1")
        .expect("the column is added");
    assert_eq!(added.value.as_deref(), Some("this"));
    harness.get_by_label("Properties of column column1").click();
    harness.run();
    assert!(
        harness
            .query_by_label("Write procedure of the column")
            .is_some()
    );
}
