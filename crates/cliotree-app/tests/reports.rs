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
use egui_kittest::kittest::Queryable;
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

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

#[test]
fn edit_report_is_layout_mode_with_a_palette_without_buttons() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    assert!(harness.state().layout_active());
    assert!(harness.query_by_label("Add field").is_some());
    assert!(harness.query_by_label("Add button").is_none());
    // The canvas is drawn as the report is, with its value.
    assert_eq!(texts(&harness), ["Total", "62.92"]);
    assert!(harness.state().report.canvas_at.is_some());
    click(&mut harness, "Edit report");
    assert!(!harness.state().layout_active());
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}

#[test]
fn a_control_added_in_layout_mode_is_in_the_report_and_undo_layout_takes_it_back() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    let undo_before = harness.state().calc().undo_label().map(str::to_owned);
    click(&mut harness, "Edit report");
    click(&mut harness, "Add label");
    let controls = |h: &Harness<'_, ClioApp>| {
        h.state()
            .calc()
            .report("Summary")
            .unwrap()
            .layout
            .controls
            .len()
    };
    assert_eq!(controls(&harness), 2);
    // Not an edit of the workbook.
    assert_eq!(
        harness.state().calc().undo_label().map(str::to_owned),
        undo_before
    );
    click(&mut harness, "Undo layout");
    assert_eq!(controls(&harness), 1);
    click(&mut harness, "Redo layout");
    assert_eq!(controls(&harness), 2);
}

#[test]
fn the_property_page_of_a_report_field_has_its_value_and_nothing_for_editing() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().select_control(Some(vec![0]));
    settle(&mut harness);
    assert!(harness.query_all_by_label("Value").next().is_some());
    assert!(harness.query_all_by_label("Prefix").next().is_some());
    for gone in [
        "Source of the field",
        "Editor of the field",
        "Write procedure of the field",
        "Help",
        "Enabled when",
        "Choices",
    ] {
        assert!(
            harness.query_all_by_label(gone).next().is_none(),
            "{gone} is shown"
        );
    }
}

#[test]
fn edit_report_on_the_tab_menu_shows_the_report_in_layout_mode() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    harness.state_mut().switch_to("Orders");
    settle(&mut harness);
    harness.get_by_label("Summary").click_secondary();
    harness.run();
    click(&mut harness, "Edit report");
    assert_eq!(harness.state().sheet, "Summary");
    assert!(harness.state().layout_active());
}

// ------------------------------------------------------------ the canvas

fn drag(harness: &mut Harness<'_, ClioApp>, from: egui::Pos2, to: egui::Pos2) {
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
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    harness.remove_cursor();
    harness.run();
}

fn click_at(harness: &mut Harness<'_, ClioApp>, pos: egui::Pos2) {
    harness.hover_at(pos);
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
}

/// Where a point of the canvas, in report points, is on the screen.
fn on_screen(harness: &Harness<'_, ClioApp>, x: f32, y: f32) -> egui::Pos2 {
    let (origin, zoom) = harness.state().report.canvas_at.unwrap();
    origin + egui::vec2(x * zoom, y * zoom)
}

fn first_control(harness: &Harness<'_, ClioApp>) -> Control {
    harness
        .state()
        .calc()
        .report("Summary")
        .unwrap()
        .layout
        .controls[0]
        .clone()
}

#[test]
fn a_report_made_before_the_canvas_is_turned_into_one_when_shown() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    let design = harness.state().calc().report("Summary").unwrap().clone();
    assert!(design.canvas);
    let field = &design.layout.controls[0];
    assert_eq!((field.x, field.y), (Some(0), Some(0)));
    assert!(field.width.is_some() && field.height.is_some());
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}

#[test]
fn a_control_is_moved_by_dragging_it_on_the_canvas_and_undo_layout_puts_it_back() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.run();
    let from = on_screen(&harness, 20.0, 8.0);
    let to = on_screen(&harness, 120.0, 88.0);
    drag(&mut harness, from, to);
    let moved = first_control(&harness);
    assert_eq!((moved.x, moved.y), (Some(100), Some(80)));
    assert_eq!(harness.state().selected_control(), Some(vec![0]));
    click(&mut harness, "Undo layout");
    let back = first_control(&harness);
    assert_eq!((back.x, back.y), (Some(0), Some(0)));
}

#[test]
fn a_handle_resizes_the_selected_control_to_the_grid() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.state_mut().select_control(Some(vec![0]));
    harness.run();
    let c = first_control(&harness);
    let (w, h) = (c.width.unwrap() as f32, c.height.unwrap() as f32);
    // The corner at the bottom right, dragged 41 points right and 30 down.
    let from = on_screen(&harness, w, h);
    let to = on_screen(&harness, w + 41.0, h + 30.0);
    drag(&mut harness, from, to);
    let resized = first_control(&harness);
    assert_eq!(resized.width.unwrap() % 4, 0);
    assert!(resized.width.unwrap() as f32 >= w + 38.0);
    assert!(resized.height.unwrap() as f32 >= h + 28.0);
}

#[test]
fn the_palette_adds_a_control_with_a_box_under_the_lowest_one() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    click(&mut harness, "Add label");
    let design = harness.state().calc().report("Summary").unwrap().clone();
    let (first, added) = (&design.layout.controls[0], &design.layout.controls[1]);
    assert!(added.y.unwrap() >= first.y.unwrap() + first.height.unwrap());
    assert!(added.width.is_some() && added.height.is_some());
    assert_eq!(harness.state().selected_control(), Some(vec![1]));
    // Delete takes the selected control away.
    harness.key_press(egui::Key::Delete);
    harness.run();
    let count = harness
        .state()
        .calc()
        .report("Summary")
        .unwrap()
        .layout
        .controls
        .len();
    assert_eq!(count, 1);
}

#[test]
fn a_click_on_the_canvas_selects_and_on_the_empty_page_selects_nothing() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    harness.state_mut().report.zoom = Some(1.0);
    harness.run();
    let on = on_screen(&harness, 10.0, 8.0);
    click_at(&mut harness, on);
    harness.run();
    assert_eq!(harness.state().selected_control(), Some(vec![0]));
    let off = on_screen(&harness, 300.0, 400.0);
    let mut design = harness.state().calc().report("Summary").unwrap().clone();
    design.height = Some(600);
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_report("Summary", design)
        .unwrap();
    harness.run();
    click_at(&mut harness, off);
    harness.run();
    assert_eq!(harness.state().selected_control(), None);
}

#[test]
fn show_expressions_shows_each_field_s_expression() {
    let mut harness = window();
    report_with_field(&mut harness, "Total", "Orders.total");
    click(&mut harness, "Edit report");
    click(&mut harness, "Show expressions");
    assert_eq!(texts(&harness), ["Total", "[Orders.total]"]);
    click(&mut harness, "Show expressions");
    assert_eq!(texts(&harness), ["Total", "62.92"]);
}
