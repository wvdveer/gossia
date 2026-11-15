//! The add/delete button of a canvas form's repeater section (DESIGN.md,
//! Later versions, "Form canvases, 0.7.0", "The add/delete button"): Delete
//! in each row, the blank row after the items with its draft, Add, what the
//! button allows, its property page and the palette.

use std::time::Duration;

use egui::Key;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_core::form::canvas::RowAllows;
use gossia_core::form::{Control, ControlKind, FormDesign, LabelPosition};
use gossia_core::{Path, Value};

fn p(text: &str) -> Path {
    text.parse().unwrap()
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

fn field(path: &str, x: u32) -> Control {
    Control {
        path: Some(path.to_owned()),
        x: Some(x),
        y: Some(2),
        width: Some(200),
        label_position: Some(LabelPosition::None),
        ..Control::new(ControlKind::Field)
    }
}

fn row_button(allows: Option<RowAllows>) -> Control {
    Control {
        x: Some(640),
        y: Some(2),
        width: Some(72),
        height: Some(24),
        allows,
        ..Control::new(ControlKind::RowButton)
    }
}

/// The Orders tab shown by a canvas form whose items repeater has the sku,
/// the qty and the total (a column formula) in its row, and an add/delete
/// button allowing `allows`.
fn rows(allows: Option<RowAllows>) -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let mut design = FormDesign::new_canvas(None);
        design.controls[0].height = Some(40);
        let at = design.add_repeater("items").unwrap();
        design.controls[at].height = Some(28);
        design.controls[at].children = vec![
            field("sku", 8),
            field("qty", 220),
            field("total", 430),
            row_button(allows),
        ];
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness.run();
    harness
}

fn count(harness: &Harness<'_, GossiaApp>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

fn buttons(harness: &Harness<'_, GossiaApp>, label: &str) -> usize {
    harness
        .query_all(By::new().label(label).role(Role::Button))
        .count()
}

fn items(harness: &Harness<'_, GossiaApp>) -> Vec<Value> {
    let app = harness.state();
    let data = app.calc().data("Orders").unwrap();
    data.get(&p("items"))
        .and_then(|n| n.value().as_array().map(|a| a.to_vec()))
        .unwrap_or_default()
        .iter()
        .map(|n| n.value().clone())
        .collect()
}

fn sku(item: &Value) -> Option<String> {
    item.as_object()?
        .get("sku")?
        .value()
        .as_str()
        .map(str::to_owned)
}

/// Types into a field and leaves it with Enter.
fn type_into(harness: &mut Harness<'_, GossiaApp>, label: &str, text: &str) {
    harness.get_by_label(label).click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[Key::A]);
    harness.get_by_label(label).type_text(text);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    settle(harness);
}

/// Types into a field and leaves it as it is, still being typed.
fn type_only(harness: &mut Harness<'_, GossiaApp>, label: &str, text: &str) {
    harness.get_by_label(label).click();
    harness.run();
    harness.get_by_label(label).type_text(text);
    harness.run();
}

fn click_add(harness: &mut Harness<'_, GossiaApp>) {
    harness
        .get(By::new().label("Add").role(Role::Button))
        .click();
    harness.run();
    settle(harness);
    harness.run();
}

#[test]
fn delete_in_a_row_deletes_its_item_as_one_edit() {
    let mut harness = rows(None);
    assert_eq!(buttons(&harness, "Delete"), 3);
    harness
        .query_all(By::new().label("Delete").role(Role::Button))
        .nth(1)
        .unwrap()
        .click();
    harness.run();
    settle(&mut harness);
    let left = items(&harness);
    assert_eq!(left.len(), 2);
    assert_eq!(sku(&left[0]).as_deref(), Some("A-100"));
    assert_eq!(sku(&left[1]).as_deref(), Some("C-310"));
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(items(&harness).len(), 3);
}

#[test]
fn the_blank_row_keeps_what_is_typed_until_add() {
    let mut harness = rows(None);
    // Blank, editable, after the items; the column formula is not typed.
    assert_eq!(count(&harness, "Orders.items[new].sku"), 1);
    type_into(&mut harness, "Orders.items[new].sku", "D-400");
    type_into(&mut harness, "Orders.items[new].qty", "5");
    // Not in the document yet.
    assert_eq!(items(&harness).len(), 3);
    assert_eq!(count(&harness, "Orders.items[3].sku"), 0);
    click_add(&mut harness);
    let now = items(&harness);
    assert_eq!(now.len(), 4);
    let added = now[3].as_object().unwrap();
    assert_eq!(sku(&now[3]).as_deref(), Some("D-400"));
    assert_eq!(added.get("qty").unwrap().value(), &Value::Int(5));
    // The schema's defaults for the rest: the required price.
    assert!(added.contains_key("price"));
    // The draft is emptied, and a new blank row follows.
    harness.run();
    let state = harness.state();
    assert!(state.form.row_drafts.is_empty());
    assert_eq!(count(&harness, "Orders.items[3].sku"), 1);
    assert_eq!(count(&harness, "Orders.items[new].sku"), 1);
    // One edit.
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(items(&harness).len(), 3);
}

#[test]
fn text_still_being_typed_counts_and_refused_text_keeps_add_from_adding() {
    let mut harness = rows(None);
    type_only(&mut harness, "Orders.items[new].sku", "E-500");
    click_add(&mut harness);
    let now = items(&harness);
    assert_eq!(now.len(), 4);
    assert_eq!(sku(&now[3]).as_deref(), Some("E-500"));

    type_only(&mut harness, "Orders.items[new].qty", "many");
    click_add(&mut harness);
    assert_eq!(items(&harness).len(), 4, "refused text adds nothing");
    let notice = harness.state().notice.clone().unwrap_or_default();
    assert!(notice.contains("not added"), "says why: {notice}");
}

#[test]
fn add_only_has_no_delete_and_delete_only_no_blank_row() {
    let harness = rows(Some(RowAllows::Add));
    assert_eq!(buttons(&harness, "Delete"), 0);
    assert_eq!(buttons(&harness, "Add"), 1);
    let harness = rows(Some(RowAllows::Delete));
    assert_eq!(buttons(&harness, "Delete"), 3);
    assert_eq!(buttons(&harness, "Add"), 0);
    assert_eq!(count(&harness, "Orders.items[new].sku"), 0);
}

#[test]
fn a_computed_array_has_no_blank_row_and_no_delete() {
    let mut harness = rows(None);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_formula_replacing(
            "Orders",
            &p("items"),
            "[{'sku': 'Z', 'qty': 1, 'price': 1.0}]",
        )
        .unwrap();
    harness.state_mut().session.tick();
    settle(&mut harness);
    harness.run();
    assert_eq!(buttons(&harness, "Delete"), 0);
    assert_eq!(buttons(&harness, "Add"), 0);
}

#[test]
fn the_property_page_sets_what_it_allows_and_its_labels() {
    let mut harness = rows(None);
    harness.state_mut().set_layout(true);
    harness.state_mut().select_control(Some(vec![1, 3]));
    harness.run();
    harness
        .get_by_label("What the add/delete button allows")
        .click();
    harness.run();
    harness.get_by_label("Delete only").click();
    harness.run();
    let design = harness.state().design_of("Default");
    assert_eq!(
        design.controls[1].children[3].allows,
        Some(RowAllows::Delete)
    );
    assert!(harness.query_by_label("Add configuration").is_some());
    assert!(harness.query_by_label("Delete configuration").is_some());
}

#[test]
fn the_palette_puts_an_add_delete_button_on_a_repeater_without_one() {
    let mut harness = rows(None);
    harness.state_mut().set_layout(true);
    harness.run();
    // The only repeater has one already.
    let add = harness.get_by_label("Add an add/delete button");
    assert!(add.accesskit_node().is_disabled());
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        design.controls[1].children.pop();
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
    }
    harness.run();
    harness.get_by_label("Add an add/delete button").click();
    harness.run();
    let design = harness.state().design_of("Default");
    let button = design.controls[1].children.last().unwrap();
    assert_eq!(button.kind, ControlKind::RowButton);
    assert_eq!(harness.state().selected_control(), Some(vec![1, 3]));
}
