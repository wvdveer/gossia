//! The expressions on form controls, in the window (DESIGN.md, "Forms",
//! "Expressions on controls"): *visible when* hides a control, *enabled when*
//! disables it, the colour expressions colour it, and each follows the data.

use std::time::Duration;

use egui::Color32;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_app::form_exprs::{Gate, gate};
use gossia_core::form::{Control, ControlKind};
use gossia_core::{Path, Value};

const PROCEDURES: &str = "\
def pay(form, ctx):
    form.customer.status = 'paid'
";

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
/// the bottom of its header, in this order; gives the place of the first
/// among the header's controls.
fn form(buttons: Vec<Control>) -> (Harness<'static, GossiaApp>, usize) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    let first;
    {
        let app = harness.state_mut();
        app.session
            .calc_mut()
            .add_module("procs", PROCEDURES)
            .unwrap();
        let mut design = app.design_of("Default");
        let spec = app.calc().spec("Orders").unwrap().clone();
        first = design.controls[0].children.len();
        for b in buttons {
            design.place_on_header(&spec, b);
        }
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    (harness, first)
}

fn the_gate(harness: &Harness<'_, GossiaApp>, at: usize) -> Gate {
    gate(harness.state().calc(), "Orders", "Default", &[0, at], &[])
}

fn disabled(harness: &Harness<'_, GossiaApp>, label: &str) -> bool {
    harness.get_by_label(label).accesskit_node().is_disabled()
}

fn set(harness: &mut Harness<'_, GossiaApp>, path: &str, value: Value) {
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

// ------------------------------------------------- fields in the rows

/// A field of the row of the items, by path, and how its design is changed.
type RowField = (&'static str, fn(&mut Control));

/// The Default form with these expressions on the fields of the row of its
/// repeater section of items, by path; gives the section's place.
fn with_row_fields(set: Vec<RowField>) -> (Harness<'static, GossiaApp>, usize) {
    let (mut harness, _) = form(Vec::new());
    let section;
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        section = design.repeater_sections()[0];
        for (path, change) in set {
            let field = design.controls[section]
                .children
                .iter_mut()
                .find(|c| c.path.as_deref() == Some(path))
                .expect("the row has the field");
            change(field);
        }
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
    }
    settle(&mut harness);
    (harness, section)
}

/// The gate of the field `path` in row `row` of the section at `section`.
fn cell(harness: &Harness<'_, GossiaApp>, section: usize, path: &str, row: usize) -> Gate {
    let design = harness.state().design_of("Default");
    let k = design.controls[section]
        .children
        .iter()
        .position(|c| c.path.as_deref() == Some(path))
        .unwrap();
    gate(
        harness.state().calc(),
        "Orders",
        "Default",
        &[section, k],
        &[row],
    )
}

fn row_fields() -> Vec<RowField> {
    vec![
        ("sku", |c| c.enabled_when = Some("parent.qty < 10".into())),
        ("qty", |c| {
            c.fill_colour = Some("'red' if this > 5 else None".into());
        }),
        ("price", |c| {
            c.visible_when = Some("Orders.customer.vip".into())
        }),
        // The first item has no note: `this` is None.
        ("note", |c| {
            c.text_colour = Some("'blue' if this else None".into());
        }),
    ]
}

#[test]
fn the_expressions_of_a_field_run_for_each_row() {
    let (mut harness, at) = with_row_fields(row_fields());
    let red = Some(Color32::from_rgb(255, 0, 0));
    assert_eq!(cell(&harness, at, "qty", 0).fill, None);
    assert_eq!(cell(&harness, at, "qty", 2).fill, red);
    assert_eq!(cell(&harness, at, "note", 0).text, None);
    assert_eq!(
        cell(&harness, at, "note", 1).text,
        Some(Color32::from_rgb(0, 0, 255))
    );
    assert!(!disabled(&harness, "Orders.items[0].sku"));
    assert!(disabled(&harness, "Orders.items[2].sku"));
    // Each cell follows its row.
    set(&mut harness, "items[2].qty", Value::Int(3));
    assert_eq!(cell(&harness, at, "qty", 2).fill, None);
    assert!(!disabled(&harness, "Orders.items[2].sku"));
    set(&mut harness, "items[0].qty", Value::Int(30));
    assert_eq!(cell(&harness, at, "qty", 0).fill, red);
    assert!(disabled(&harness, "Orders.items[0].sku"));
}

#[test]
fn visible_when_on_a_field_in_a_row_follows_each_row() {
    let (mut harness, _) = with_row_fields(row_fields());
    assert!(harness.query_by_label("Orders.items[0].price").is_some());
    set(&mut harness, "customer.vip", Value::Bool(false));
    for row in 0..3 {
        let label = format!("Orders.items[{row}].price");
        assert!(harness.query_by_label(&label).is_none(), "{label}");
    }
    assert!(harness.query_by_label("Orders.items[0].qty").is_some());
    set(&mut harness, "customer.vip", Value::Bool(true));
    assert!(harness.query_by_label("Orders.items[0].price").is_some());
}

#[test]
fn the_expressions_of_a_row_do_not_run_in_a_workbook_that_is_not_trusted() {
    let (mut harness, at) = with_row_fields(row_fields());
    harness.state_mut().session.calc_mut().set_trusted(false);
    settle(&mut harness);
    assert!(disabled(&harness, "Orders.items[0].sku"));
    let g = cell(&harness, at, "sku", 0);
    assert!(g.why[0].contains("trusted"), "{:?}", g.why);
    assert_eq!(cell(&harness, at, "qty", 2).fill, None);
}

// --------------------------------------------------- errors on the property page

fn type_expression(harness: &mut Harness<'_, GossiaApp>, field: &str, text: &str) {
    let role = egui::accesskit::Role::TextInput;
    harness.get_by_role_and_label(role, field).click();
    harness.run();
    // Over what is there; Enter stores it.
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    harness.get_by_role_and_label(role, field).type_text(text);
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    settle(harness);
    harness.run();
}

fn error_shown(harness: &Harness<'_, GossiaApp>, text: &str) -> bool {
    harness.query_by_label_contains(text).is_some()
}

/// A colour typed without quotes is a Python name, which fails: the property
/// page says so under the field, and says nothing once it works.
#[test]
fn the_property_page_shows_why_an_expression_failed() {
    let (mut harness, _) = form(Vec::new());
    harness.get_by_label("View").click();
    harness.run();
    harness.get_by_label("Forms").click();
    harness.run();
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.get_by_label("Field customer.name").click();
    harness.run();
    assert!(!error_shown(&harness, "NameError"));
    type_expression(&mut harness, "Text colour", "red");
    assert!(error_shown(
        &harness,
        "#NAME!: NameError: name 'red' is not defined"
    ));
    // Only the first line; the traceback is on hover.
    assert!(!error_shown(&harness, "Traceback"));

    type_expression(&mut harness, "Text colour", "'red'");
    assert!(!error_shown(&harness, "NameError"));
}

/// For a field in the row of a repeater section, the first row that failed.
#[test]
fn the_property_page_of_a_field_in_a_row_names_the_row_that_failed() {
    let (mut harness, _) = with_row_fields(vec![("note", |c| {
        c.text_colour = Some("'red' if this.startswith('g') else None".into());
    })]);
    harness.get_by_label("View").click();
    harness.run();
    harness.get_by_label("Forms").click();
    harness.run();
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.get_by_label("Field note").click();
    harness.run();
    // The first item has no note: None has no startswith.
    assert!(error_shown(&harness, "Row [0]: "));
    assert!(error_shown(&harness, "startswith"));
}
