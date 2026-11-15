//! Canvas forms in the form view (DESIGN.md, Later versions, "Form
//! canvases, 0.7.0"): the stack of sections, the rows of a repeater
//! section, the current row, and context fields whose values bind paths.

use std::time::Duration;

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_core::form::{Control, ControlKind, FormDesign, LabelPosition};
use gossia_core::{Path, Value};

const PROCEDURES: &str = "def note_row(form, ctx):
    form.items[ctx.index].note = 'row ' + str(ctx.context.row)
";

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

fn field(path: &str, x: u32, y: u32) -> Control {
    Control {
        path: Some(path.to_owned()),
        x: Some(x),
        y: Some(y),
        width: Some(200),
        label_position: Some(LabelPosition::None),
        ..Control::new(ControlKind::Field)
    }
}

fn context(name: &str, default: &str) -> Control {
    Control {
        context: Some(name.to_owned()),
        context_default: Some(default.to_owned()),
        label: Some("Which".to_owned()),
        x: Some(8),
        y: Some(8),
        ..Control::new(ControlKind::Field)
    }
}

/// The Orders tab shown by a canvas form: `header` on its header, and a
/// repeater over the items with their sku and qty in its row.
fn canvas(header: Vec<Control>) -> Harness<'static, GossiaApp> {
    canvas_with(header, Vec::new())
}

/// As [`canvas`], with `more` in the row besides the sku and qty, and a
/// module of procedures.
fn canvas_with(header: Vec<Control>, more: Vec<Control>) -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let mut design = FormDesign::new_canvas(None);
        design.controls[0].children = header;
        design.controls[0].height = Some(80);
        let at = design.add_repeater("items").unwrap();
        design.controls[at].children = vec![field("sku", 8, 2), field("qty", 220, 2)];
        design.controls[at].children.extend(more);
        app.session
            .calc_mut()
            .add_module("procs", PROCEDURES)
            .unwrap();
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

fn chosen(harness: &Harness<'_, GossiaApp>, name: &str) -> Option<Value> {
    harness
        .state()
        .calc()
        .chosen_contexts("Orders", "Default")
        .and_then(|c| c.get(name).cloned())
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

#[test]
fn a_repeater_section_draws_a_row_for_each_item() {
    let harness = canvas(vec![]);
    for i in 0..3 {
        assert_eq!(count(&harness, &format!("Orders.items[{i}].sku")), 1);
        assert_eq!(count(&harness, &format!("Orders.items[{i}].qty")), 1);
    }
    assert_eq!(count(&harness, "Orders.items[3].sku"), 0);
}

#[test]
fn a_context_field_chooses_what_a_path_binds_to() {
    let mut harness = canvas(vec![context("sel", "0"), field("items[sel].sku", 8, 40)]);
    // The header's field and the first row's.
    assert_eq!(count(&harness, "Orders.items[0].sku"), 2);
    type_into(&mut harness, "context.sel", "2");
    assert_eq!(chosen(&harness, "sel"), Some(Value::Int(2)));
    assert_eq!(count(&harness, "Orders.items[0].sku"), 1);
    assert_eq!(count(&harness, "Orders.items[2].sku"), 2);
}

#[test]
fn a_value_its_type_does_not_read_is_not_chosen() {
    let mut harness = canvas(vec![context("sel", "0")]);
    type_into(&mut harness, "context.sel", "two");
    assert_eq!(chosen(&harness, "sel"), None);
}

#[test]
fn past_the_end_of_the_list_the_field_is_empty_and_disabled() {
    let mut harness = canvas(vec![context("sel", "0"), field("items[sel].sku", 8, 40)]);
    type_into(&mut harness, "context.sel", "7");
    assert_eq!(chosen(&harness, "sel"), Some(Value::Int(7)));
    assert_eq!(count(&harness, "Orders.items[7].sku"), 0);
    assert_eq!(count(&harness, "Orders.items[sel].sku"), 1);
    // The rows are all still there.
    assert_eq!(count(&harness, "Orders.items[0].sku"), 1);
}

#[test]
fn a_hidden_context_field_is_not_drawn() {
    let mut hidden = context("sel", "1");
    hidden.hidden = true;
    let harness = canvas(vec![hidden, field("items[sel].sku", 8, 40)]);
    assert_eq!(count(&harness, "context.sel"), 0);
    assert_eq!(count(&harness, "Orders.items[1].sku"), 2);
}

#[test]
fn a_click_in_a_row_makes_it_the_current_row() {
    let mut harness = canvas(vec![field("items[row].qty", 8, 40)]);
    assert_eq!(count(&harness, "Orders.items[0].qty"), 2);
    harness.get_by_label("Orders.items[2].sku").click();
    harness.run();
    settle(&mut harness);
    assert_eq!(chosen(&harness, "row"), Some(Value::Int(2)));
    assert_eq!(count(&harness, "Orders.items[2].qty"), 2);
}

#[test]
fn only_the_rows_on_the_screen_are_drawn() {
    let mut harness = canvas(vec![]);
    {
        let app = harness.state_mut();
        let text: Vec<String> = (0..2000)
            .map(|i| format!(r#"{{"sku": "S{i}", "qty": 1}}"#))
            .collect();
        let items = gossia_core::json::parse(&format!("[{}]", text.join(",")))
            .unwrap()
            .node
            .value()
            .clone();
        app.session
            .calc_mut()
            .set_value("Orders", &p("items"), items)
            .unwrap();
    }
    settle(&mut harness);
    harness.run();
    assert_eq!(count(&harness, "Orders.items[0].sku"), 1);
    assert_eq!(count(&harness, "Orders.items[1999].sku"), 0);
    let drawn = (0..2000)
        .filter(|i| count(&harness, &format!("Orders.items[{i}].sku")) > 0)
        .count();
    assert!(drawn < 100, "{drawn} rows drawn");
}

#[test]
fn the_contexts_chosen_are_saved_with_the_view() {
    let mut harness = canvas(vec![context("sel", "0")]);
    type_into(&mut harness, "context.sel", "1");
    let views = harness.state().view_state();
    let orders = &views["Orders"];
    assert_eq!(orders["contexts"]["Default"]["sel"], serde_json::json!(1));
    // Put back into a window that has not chosen it.
    let mut other = canvas(vec![context("sel", "0")]);
    other.state_mut().restore_view_state(&views);
    settle(&mut other);
    assert_eq!(chosen(&other, "sel"), Some(Value::Int(1)));
}

#[test]
fn a_button_in_a_row_runs_its_procedure_with_that_row() {
    let button = Control {
        label: Some("Note".to_owned()),
        procedure: Some("note_row".to_owned()),
        x: Some(440),
        y: Some(2),
        ..Control::new(ControlKind::Button)
    };
    let mut harness = canvas_with(vec![], vec![button]);
    assert_eq!(count(&harness, "Note"), 3);
    harness.query_all_by_label("Note").nth(1).unwrap().click();
    harness.run();
    settle(&mut harness);
    let note = harness
        .state()
        .calc()
        .data("Orders")
        .and_then(|d| d.get(&p("items[1].note")))
        .map(|n| n.value().clone());
    assert_eq!(note, Some(Value::Str("row 1".into())));
    assert_eq!(chosen(&harness, "row"), Some(Value::Int(1)));
}

#[test]
fn a_form_made_before_canvases_is_converted_when_shown() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let spec = app.calc().spec("Orders").unwrap().clone();
        // As 0.6.0 stored it: Flow, a table of the items, and a list of
        // the large lines from an expression, which cannot be a section.
        let mut old = gossia_core::form::generate(&spec);
        assert!(!old.canvas);
        let mut big = Control::new(ControlKind::Repeater);
        big.label = Some("Big lines".to_owned());
        big.items = Some("[i for i in Orders.items if i.qty > 1]".to_owned());
        old.controls.push(big);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Packing", old);
        assert!(app.set_view(View::Form("Packing".to_owned())));
    }
    settle(&mut harness);
    let app = harness.state();
    let design = app.calc().forms().get("Orders", "Packing").unwrap();
    assert!(design.canvas);
    assert_eq!(design.repeater_sections().len(), 1);
    let notice = app.notice.clone().unwrap_or_default();
    assert!(notice.contains("Big lines"), "{notice}");
    assert_eq!(count(&harness, "Orders.items[2].sku"), 1);
}

#[test]
fn a_section_grows_to_hold_a_control_drawn_taller_than_its_room() {
    // The header is 80 points tall; the group of four fields is taller.
    let mut group = Control {
        label: Some("Customer".to_owned()),
        x: Some(8),
        y: Some(8),
        width: Some(400),
        ..Control::new(ControlKind::Group)
    };
    group.children = ["name", "city", "vip", "status"]
        .iter()
        .map(|f| Control {
            path: Some(format!("customer.{f}")),
            ..Control::new(ControlKind::Field)
        })
        .collect();
    let mut harness = canvas(vec![group]);
    harness.run();
    let status = harness.get_by_label("Orders.customer.status").rect();
    let row = harness.get_by_label("Orders.items[0].sku").rect();
    assert!(status.bottom() < row.top(), "{status:?} {row:?}");
}
