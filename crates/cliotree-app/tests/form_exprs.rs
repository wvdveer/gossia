//! The expressions on form controls, in the window (DESIGN.md, "Forms",
//! "Expressions on controls"): *visible when* hides a control, *enabled when*
//! disables it, the colour expressions colour it, and each follows the data.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_app::form_exprs::{Gate, gate};
use cliotree_core::form::{Control, ControlKind};
use cliotree_core::{Path, Value};
use egui::Color32;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const PROCEDURES: &str = "\
def pay(form, ctx):
    form.customer.status = 'paid'
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

fn button(label: &str) -> Control {
    let mut control = Control::new(ControlKind::Button);
    control.label = Some(label.to_owned());
    control.procedure = Some("pay".to_owned());
    control
}

/// The Default form of Orders with buttons whose expressions are given, at
/// the end of the form, in this order.
fn form(buttons: Vec<Control>) -> (Harness<'static, ClioApp>, usize) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    let first;
    {
        let app = harness.state_mut();
        app.session
            .calc_mut()
            .add_module("procs", PROCEDURES)
            .unwrap();
        let mut design = app.design_of("Default");
        first = design.controls.len();
        design.controls.extend(buttons);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    (harness, first)
}

fn the_gate(harness: &Harness<'_, ClioApp>, at: usize) -> Gate {
    gate(harness.state().calc(), "Orders", "Default", &[at])
}

fn disabled(harness: &Harness<'_, ClioApp>, label: &str) -> bool {
    harness.get_by_label(label).accesskit_node().is_disabled()
}

fn set(harness: &mut Harness<'_, ClioApp>, path: &str, value: Value) {
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_value("Orders", &p(path), value)
        .unwrap();
    settle(harness);
}

#[test]
fn visible_when_hides_a_control_and_follows_the_data() {
    let mut ship = button("Ship");
    ship.visible_when = Some("Orders.customer.vip".to_owned());
    let (mut harness, _) = form(vec![ship]);
    assert!(harness.query_by_label("Ship").is_some());
    set(&mut harness, "customer.vip", Value::Bool(false));
    assert!(harness.query_by_label("Ship").is_none());
    set(&mut harness, "customer.vip", Value::Bool(true));
    assert!(harness.query_by_label("Ship").is_some());
}

#[test]
fn enabled_when_disables_a_control_and_an_error_says_why() {
    let mut pay = button("Pay");
    pay.enabled_when = Some("Orders.customer.status != 'paid'".to_owned());
    let mut broken = button("Broken");
    broken.visible_when = Some("Orders.customer.nothing".to_owned());
    let (mut harness, first) = form(vec![pay, broken]);
    assert!(disabled(&harness, "Pay"), "the sample's order is paid");
    set(&mut harness, "customer.status", Value::Str("open".into()));
    assert!(!disabled(&harness, "Pay"));

    // An expression that fails leaves its control in view, disabled, and
    // says why.
    assert!(disabled(&harness, "Broken"));
    let why = the_gate(&harness, first + 1).why.join("\n");
    assert!(why.starts_with("Visible when:"), "{why}");
    assert!(why.contains("nothing"), "{why}");
}

#[test]
fn the_colour_expressions_give_the_colours_of_the_text_and_the_fill() {
    let mut flag = button("Flag");
    flag.text_colour = Some("'white' if Orders.customer.vip else None".to_owned());
    flag.fill_colour = Some("(200, 0, 0) if Orders.customer.vip else '#00ff00'".to_owned());
    let mut odd = button("Odd");
    odd.fill_colour = Some("'reddish'".to_owned());
    let (mut harness, first) = form(vec![flag, odd]);
    let g = the_gate(&harness, first);
    assert_eq!(g.text, Some(Color32::WHITE));
    assert_eq!(g.fill, Some(Color32::from_rgb(200, 0, 0)));
    set(&mut harness, "customer.vip", Value::Bool(false));
    let g = the_gate(&harness, first);
    assert_eq!(g.text, None);
    assert_eq!(g.fill, Some(Color32::from_rgb(0, 255, 0)));

    // A value that is not a colour leaves the usual look and says why; the
    // control can still be used.
    let g = the_gate(&harness, first + 1);
    assert_eq!(g.fill, None);
    assert!(!g.disabled);
    assert!(g.why[0].contains("reddish"), "{:?}", g.why);
    harness.run();
    assert!(!disabled(&harness, "Odd"));
}

#[test]
fn in_a_workbook_that_is_not_trusted_nothing_runs() {
    let mut ship = button("Ship");
    ship.visible_when = Some("False".to_owned());
    let mut pay = button("Pay");
    pay.enabled_when = Some("True".to_owned());
    let (mut harness, first) = form(vec![ship, pay]);
    assert!(harness.query_by_label("Ship").is_none());
    harness.state_mut().session.calc_mut().set_trusted(false);
    settle(&mut harness);
    // Shown, and an *enabled when* is disabled with the reason.
    assert!(harness.query_by_label("Ship").is_some());
    let g = the_gate(&harness, first + 1);
    assert!(g.disabled);
    assert!(g.why[0].contains("trusted"), "{:?}", g.why);
}
