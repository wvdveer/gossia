//! A form used from the keyboard and its smaller marks (DESIGN.md, "Forms"):
//! the arrow keys going from control to control, the mark of a formula not
//! run, the calendar of a date, and going to a node that a stacked item
//! shows.

use std::sync::Arc;
use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::form::{ControlKind, RepeaterMode};
use cliotree_core::{Path, json, jsonschema};
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn settle(harness: &mut Harness<'_, ClioApp>) {
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
}

fn harness_of(app: ClioApp) -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), app);
    settle(&mut harness);
    harness
}

/// The Default form of the sample's Orders.
fn orders_form() -> Harness<'static, ClioApp> {
    let mut harness = harness_of(ClioApp::sample());
    assert!(
        harness
            .state_mut()
            .set_view(View::Form("Default".to_owned()))
    );
    harness.run();
    harness.run();
    harness
}

fn focused(harness: &Harness<'_, ClioApp>, name: &str) -> bool {
    harness
        .get(
            egui_kittest::kittest::By::new()
                .label(name)
                .role(egui::accesskit::Role::TextInput),
        )
        .is_focused()
}

fn click_field(harness: &mut Harness<'_, ClioApp>, name: &str) {
    harness
        .get(
            egui_kittest::kittest::By::new()
                .label(name)
                .role(egui::accesskit::Role::TextInput),
        )
        .click();
    harness.run();
}

fn press(harness: &mut Harness<'_, ClioApp>, key: Key) {
    harness.key_press(key);
    harness.run();
    harness.run();
}

fn value(harness: &Harness<'_, ClioApp>, path: &str) -> String {
    let app = harness.state();
    app.calc()
        .data(&app.sheet)
        .unwrap()
        .get(&p(path))
        .map(|n| cliotree_app::rows::display(n.value()))
        .unwrap_or_default()
}

#[test]
fn up_and_down_go_to_the_field_above_and_below_and_enter_what_was_typed() {
    let mut harness = orders_form();
    click_field(&mut harness, "Orders.customer.name");
    press(&mut harness, Key::ArrowDown);
    assert!(focused(&harness, "Orders.customer.city"));
    // What is typed goes in when the key leaves the field.
    harness.key_press(Key::End);
    harness.event(egui::Event::Text("s".to_owned()));
    harness.run();
    press(&mut harness, Key::ArrowUp);
    settle(&mut harness);
    assert!(focused(&harness, "Orders.customer.name"));
    assert_eq!(value(&harness, "customer.city"), "Utrechts");
}

#[test]
fn in_a_table_the_keys_go_from_cell_to_cell() {
    let mut harness = orders_form();
    click_field(&mut harness, "Orders.items[0].qty");
    press(&mut harness, Key::ArrowDown);
    assert!(focused(&harness, "Orders.items[1].qty"));
    // Left and Right move the caret first, and leave from its end.
    press(&mut harness, Key::End);
    press(&mut harness, Key::ArrowRight);
    assert!(focused(&harness, "Orders.items[1].price"));
    press(&mut harness, Key::Home);
    press(&mut harness, Key::ArrowLeft);
    assert!(focused(&harness, "Orders.items[1].qty"));
    press(&mut harness, Key::ArrowUp);
    assert!(focused(&harness, "Orders.items[0].qty"));
}

#[test]
fn the_keys_pass_by_a_computed_field() {
    let mut harness = orders_form();
    // total is computed by a column formula, so it is read only.
    click_field(&mut harness, "Orders.items[0].price");
    press(&mut harness, Key::End);
    press(&mut harness, Key::ArrowRight);
    assert!(focused(&harness, "Orders.items[0].note"));
    press(&mut harness, Key::ArrowLeft);
    assert!(focused(&harness, "Orders.items[0].price"));
    // Below the table only computed fields follow tax_rate: Down stays.
    click_field(&mut harness, "Orders.tax_rate");
    press(&mut harness, Key::ArrowDown);
    assert!(focused(&harness, "Orders.tax_rate"));
}

#[test]
fn a_formula_that_was_not_run_is_marked_on_the_form() {
    let mut harness = orders_form();
    assert!(harness.query_by_label("#UNTRUSTED!").is_none());
    harness.state_mut().session.calc_mut().set_trusted(false);
    settle(&mut harness);
    // subtotal, tax and total; the column of totals is computed by a column
    // formula, whose cells are marked too.
    assert!(harness.query_all_by_label("#UNTRUSTED!").count() >= 3);
}

fn dated() -> ClioApp {
    let mut calc = cliotree_calc::Calc::new(Arc::new(cliotree_calc::python::PythonSyntax));
    let spec = jsonschema::lower_str(
        r#"{"type": "object", "properties": {
            "due": {"type": "string", "format": "date"},
            "at": {"type": "string", "format": "date-time"}}}"#,
    )
    .unwrap();
    let data = json::parse(r#"{"due": "2026-03-02", "at": "2026-03-02T09:30:00Z"}"#)
        .unwrap()
        .node;
    calc.add_sheet("T", spec, data).unwrap();
    ClioApp::new(calc)
}

#[test]
fn a_date_is_picked_from_a_calendar() {
    let mut harness = harness_of(dated());
    assert!(
        harness
            .state_mut()
            .set_view(View::Form("Default".to_owned()))
    );
    harness.run();
    harness.get_by_label("Pick a date for T.due").click();
    harness.run();
    harness.get_by_label("15").click();
    harness.run();
    harness.get_by_label("Save").click();
    settle(&mut harness);
    assert_eq!(value(&harness, "due"), "2026-03-15");
    // A date and time keeps its time.
    harness.get_by_label("Pick a date for T.at").click();
    harness.run();
    harness.get_by_label("20").click();
    harness.run();
    harness.get_by_label("Save").click();
    settle(&mut harness);
    assert_eq!(value(&harness, "at"), "2026-03-20T09:30:00Z");
}

#[test]
fn going_to_a_node_of_a_stacked_item_steps_to_that_item() {
    let mut harness = harness_of(ClioApp::sample());
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        let at = design
            .walk()
            .into_iter()
            .find(|(_, c)| c.kind == ControlKind::Repeater && c.bound_path() == Some(p("items")))
            .map(|(at, _)| at)
            .unwrap();
        design.get_mut(&at).unwrap().mode = Some(RepeaterMode::Stacked);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        app.set_open(&p("items"), true);
        app.select(&p("items[2].sku"));
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    for _ in 0..4 {
        harness.run();
    }
    harness.get_by_label("3 of 3");
    assert!(harness.query_by_label("Orders.items[2].sku").is_some());
}
