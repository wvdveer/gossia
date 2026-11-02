//! Reports in the window (DESIGN.md, "Reports"): a report tab draws its
//! controls from the results of its expressions, which run while it is
//! shown and follow the data.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::control;
use cliotree_core::form::{Control, ControlKind};
use cliotree_core::report::ReportDesign;
use cliotree_core::{Path, Value};
use egui_kittest::Harness;
use serde_json::json;

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    harness
}

/// Draws until nothing is waiting: a frame shows the report, the evaluation
/// runs its expressions, and the next frame draws what they came to.
fn settle(harness: &mut Harness<'_, ClioApp>) {
    for _ in 0..3 {
        harness.run();
        assert!(
            harness
                .state_mut()
                .session
                .wait_idle(Duration::from_secs(60)),
            "the workbook did not settle"
        );
    }
    harness.run();
}

/// The text the report tab drew.
fn texts(harness: &Harness<'_, ClioApp>) -> Vec<String> {
    harness
        .state()
        .report
        .laid
        .as_ref()
        .map(|l| l.texts().into_iter().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn new_report(harness: &mut Harness<'_, ClioApp>, args: serde_json::Value) {
    control::run(harness.state_mut(), "new_tab", &args).unwrap();
    settle(harness);
}

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

#[test]
fn a_report_from_an_array_shows_its_items_and_follows_the_data() {
    let mut harness = window();
    new_report(
        &mut harness,
        json!({"name": "Lines", "report": true, "start_tab": "Orders", "start_path": "items"}),
    );
    let shown = texts(&harness);
    for sku in ["A-100", "B-220", "C-310"] {
        assert!(shown.iter().any(|t| t == sku), "{shown:?}");
    }
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_value("Orders", &p("items[0].sku"), Value::Str("Z-999".into()))
        .unwrap();
    settle(&mut harness);
    let shown = texts(&harness);
    assert!(shown.iter().any(|t| t == "Z-999"), "{shown:?}");
    assert!(!shown.iter().any(|t| t == "A-100"));
}

/// A report with one field showing `expression`, labelled `label`.
fn report_with_field(harness: &mut Harness<'_, ClioApp>, label: &str, expression: &str) {
    new_report(harness, json!({"name": "Summary", "report": true}));
    let mut design = ReportDesign::new(Default::default());
    let mut field = Control::new(ControlKind::Field);
    field.label = Some(label.to_owned());
    field.value = Some(expression.to_owned());
    design.layout.controls.push(field);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Summary", design)
        .unwrap();
    settle(harness);
}

#[test]
fn a_field_shows_what_its_expression_comes_to() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}

#[test]
fn a_report_runs_only_while_its_tab_is_shown() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    assert_eq!(harness.state().calc().shown_report(), Some("Summary"));
    harness.state_mut().switch_to("Orders");
    settle(&mut harness);
    assert_eq!(harness.state().calc().shown_report(), None);
}

#[test]
fn an_untrusted_workbook_shows_the_mark_instead_of_running_the_report() {
    let mut harness = window();
    harness.state_mut().session.calc_mut().set_trusted(false);
    report_with_field(&mut harness, "Total", "Orders.total");
    assert_eq!(texts(&harness), ["Total", "#UNTRUSTED!"]);
}

#[test]
fn an_error_in_an_expression_shows_in_its_control_and_the_others_still_show() {
    let mut harness = window();
    report_with_field(&mut harness, "Broken", "1 / 0");
    let mut design = harness.state().calc().report("Summary").unwrap().clone();
    let mut fine = Control::new(ControlKind::Field);
    fine.label = Some("Fine".into());
    fine.value = Some("2 + 2".into());
    design.layout.controls.push(fine);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Summary", design)
        .unwrap();
    settle(&mut harness);
    assert_eq!(texts(&harness), ["Broken", "#ERROR!", "Fine", "4"]);
    let laid = harness.state().report.laid.clone().unwrap();
    assert!(laid.hovers.iter().any(|h| h.text.contains("ZeroDivision")));
}
