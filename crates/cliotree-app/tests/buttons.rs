//! Tests of the buttons of a form that drive the window (DESIGN.md, "Button
//! procedures"): a click runs a procedure of a workbook module on the
//! evaluation thread, and its writes are one undoable edit, or nothing.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::form::{Control, ControlKind};
use cliotree_core::{Path, Value};
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const PROCEDURES: &str = "\
def mark_shipped(form, ctx):
    form.customer.status = 'shipped'
    form.items.append({'sku': 'Z-1', 'qty': 1, 'price': 2.0})

def shout_city(form, ctx):
    form.customer.name = form.customer.city + '!'

def fail(form, ctx):
    form.customer.name = 'changed'
    raise ValueError('no stock left')

def bad_write(form, ctx):
    form.customer.name = 'changed'
    form.tax_rate = 5
";

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

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn value(harness: &Harness<'_, ClioApp>, path: &str) -> Option<Value> {
    harness
        .state()
        .calc()
        .data("Orders")?
        .get(&p(path))
        .map(|n| n.value().clone())
}

fn button(label: &str, procedure: &str) -> Control {
    let mut control = Control::new(ControlKind::Button);
    control.label = Some(label.to_owned());
    control.procedure = Some(procedure.to_owned());
    control
}

/// The Default form of Orders with a button for each procedure, over a
/// workbook that has the module of procedures.
fn form() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        app.session
            .calc_mut()
            .add_module("procs", PROCEDURES)
            .unwrap();
        let mut design = app.design_of("Default");
        for (label, procedure) in [
            ("Mark shipped", "mark_shipped"),
            ("Shout city", "shout_city"),
            ("Fail", "fail"),
            ("Bad write", "bad_write"),
            ("Nothing", "missing"),
        ] {
            design.controls.push(button(label, procedure));
        }
        design.controls.push(Control::new(ControlKind::Button));
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness
}

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

#[test]
fn a_click_runs_the_procedure_and_its_writes_are_one_undoable_edit() {
    let mut harness = form();
    harness.state_mut().session.calc_mut().clear_history();
    click(&mut harness, "Mark shipped");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.status"),
        Some(Value::Str("shipped".into()))
    );
    assert_eq!(
        value(&harness, "items[3].sku"),
        Some(Value::Str("Z-1".into()))
    );
    assert_eq!(harness.state().calc().history_len().0, 1);
    assert_eq!(
        harness.state().calc().undo_label(),
        Some("Run Mark shipped")
    );
    // The formulas follow the new item.
    assert_eq!(value(&harness, "items[3].total"), Some(Value::Float(2.0)));

    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.status"),
        Some(Value::Str("paid".into()))
    );
    assert_eq!(value(&harness, "items[3]"), None);
}

#[test]
fn what_is_being_typed_is_entered_before_the_procedure_runs() {
    let mut harness = form();
    harness.get_by_label("Orders.customer.city").click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[Key::A]);
    harness
        .get_by_label("Orders.customer.city")
        .type_text("Rotterdam");
    harness.run();
    // No Enter: the click on the button leaves the field.
    click(&mut harness, "Shout city");
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.name"),
        Some(Value::Str("Rotterdam!".into()))
    );
}

#[test]
fn an_exception_is_shown_and_the_data_is_as_it_was() {
    let mut harness = form();
    harness.state_mut().session.calc_mut().clear_history();
    click(&mut harness, "Fail");
    settle(&mut harness);
    let notice = harness.state().notice.clone().unwrap_or_default();
    assert!(notice.contains("no stock left"), "{notice}");
    assert!(notice.starts_with("Fail"), "{notice}");
    assert_eq!(
        value(&harness, "customer.name"),
        Some(Value::Str("Acme Ltd".into()))
    );
    assert!(!harness.state().calc().can_undo());
}

#[test]
fn a_write_that_does_not_fit_is_refused_with_all_the_others() {
    let mut harness = form();
    click(&mut harness, "Bad write");
    settle(&mut harness);
    let notice = harness.state().notice.clone().unwrap_or_default();
    assert!(notice.contains("tax_rate"), "{notice}");
    assert_eq!(
        value(&harness, "customer.name"),
        Some(Value::Str("Acme Ltd".into()))
    );
    assert_eq!(value(&harness, "tax_rate"), Some(Value::Float(0.21)));
}

#[test]
fn a_button_whose_procedure_does_not_exist_says_so() {
    let mut harness = form();
    click(&mut harness, "Nothing");
    settle(&mut harness);
    let notice = harness.state().notice.clone().unwrap_or_default();
    assert!(notice.contains("no procedure called missing"), "{notice}");
}

#[test]
fn buttons_are_disabled_while_the_workbook_is_not_trusted() {
    let mut harness = form();
    harness.state_mut().session.calc_mut().set_trusted(false);
    harness.run();
    let node = harness.get_by_label("Mark shipped");
    assert!(node.accesskit_node().is_disabled());
    node.click();
    harness.run();
    settle(&mut harness);
    assert_eq!(
        value(&harness, "customer.status"),
        Some(Value::Str("paid".into()))
    );
    // Trusting it enables them.
    harness.state_mut().session.calc_mut().set_trusted(true);
    harness.run();
    assert!(
        !harness
            .get_by_label("Mark shipped")
            .accesskit_node()
            .is_disabled()
    );
}

/// The property page offers the functions of the modules; a name no module
/// has is kept and marked until another is chosen.
#[test]
fn the_procedure_of_a_button_is_chosen_from_the_functions_of_the_modules() {
    let mut harness = form();
    click(&mut harness, "Edit form");
    click(&mut harness, "Button Nothing");
    let combo = harness.get_by_label("Procedure of the button");
    assert_eq!(
        combo.value().as_deref(),
        Some("missing (not in any module)")
    );
    combo.click();
    harness.run();
    for name in ["(none)", "mark_shipped", "fail", "bad_write"] {
        harness.get_by_label(name);
    }
    click(&mut harness, "shout_city");
    let design = harness.state().design_of("Default");
    let nothing = design
        .walk()
        .into_iter()
        .map(|(_, c)| c.clone())
        .find(|c| c.label.as_deref() == Some("Nothing"))
        .unwrap();
    assert_eq!(nothing.procedure.as_deref(), Some("shout_city"));

    // Out of layout mode the button runs it.
    click(&mut harness, "Edit form");
    click(&mut harness, "Nothing");
    settle(&mut harness);
    let name = value(&harness, "customer.name").unwrap();
    assert!(
        matches!(&name, Value::Str(s) if s.ends_with('!')),
        "{name:?}"
    );
}

#[test]
fn a_button_with_no_procedure_is_disabled() {
    let harness = form();
    assert!(
        harness
            .get_by_label("Button")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn undo_waits_while_a_procedure_runs() {
    let mut harness = form();
    let app = harness.state_mut();
    app.session.calc_mut().clear_history();
    app.session
        .calc_mut()
        .set_value("Orders", &p("tax_rate"), Value::Float(0.5))
        .unwrap();
    app.click_button("mark_shipped", "Mark shipped").unwrap();
    assert!(app.session.running_procedure().is_some());
    app.undo();
    assert_eq!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("tax_rate"))
            .unwrap()
            .value(),
        &Value::Float(0.5),
        "the undo was refused while the procedure was waiting"
    );
    let again = app.click_button("mark_shipped", "Mark shipped");
    assert!(again.unwrap_err().contains("running"));
    settle(&mut harness);
}

#[test]
fn the_control_channel_clicks_a_button_by_its_label() {
    use cliotree_app::control::run;
    let mut harness = form();
    let app = harness.state_mut();
    run(app, "click_button", &json!({"label": "Mark shipped"})).unwrap();
    run(app, "wait_idle", &json!({})).unwrap();
    let status = run(app, "status", &json!({})).unwrap();
    assert_eq!(status["notice"], serde_json::Value::Null);
    assert_eq!(status["text"], "Ready");
    assert_eq!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("customer.status"))
            .unwrap()
            .value(),
        &Value::Str("shipped".into())
    );
    assert!(run(app, "click_button", &json!({"label": "Missing"})).is_err());
    run(app, "click_button", &json!({"label": "Fail"})).unwrap();
    run(app, "wait_idle", &json!({})).unwrap();
    let status = run(app, "status", &json!({})).unwrap();
    assert!(
        status["notice"]
            .as_str()
            .unwrap_or_default()
            .contains("no stock left"),
        "{status}"
    );
}
