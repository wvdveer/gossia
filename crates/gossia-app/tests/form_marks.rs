//! Marking rows on a canvas form (DESIGN.md, Later versions, "Form
//! canvases, 0.7.0", "Marking rows"): the row mark in each row, the rows
//! buttons on the header (Delete marked with its count, Add row, Toggle
//! all), the marks clearing when the items change, and `ctx.marked`.

use std::time::Duration;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_core::form::canvas::RowsAction;
use gossia_core::form::{Control, ControlKind, FormDesign, LabelPosition};
use gossia_core::{Path, Value};

const PROCEDURES: &str = "def show_marks(form, ctx):
    form.items[0].note = repr(ctx.marked['row'])
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

fn rows_button(action: RowsAction, x: u32) -> Control {
    Control {
        repeater: Some("row".to_owned()),
        rows_action: Some(action),
        x: Some(x),
        y: Some(8),
        width: Some(120),
        height: Some(24),
        ..Control::new(ControlKind::RowsButton)
    }
}

/// The Orders tab shown by a canvas form whose header has the three rows
/// buttons and a button that runs `show_marks`, and whose items repeater
/// has a row mark before the sku and qty.
fn marking() -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let mut design = FormDesign::new_canvas(None);
        design.controls[0].height = Some(40);
        design.controls[0].children = vec![
            rows_button(RowsAction::DeleteMarked, 8),
            rows_button(RowsAction::AddRow, 140),
            rows_button(RowsAction::ToggleAll, 272),
            Control {
                label: Some("Show marks".to_owned()),
                procedure: Some("show_marks".to_owned()),
                x: Some(404),
                y: Some(8),
                ..Control::new(ControlKind::Button)
            },
        ];
        let at = design.add_repeater("items").unwrap();
        design.controls[at].height = Some(28);
        design.controls[at].children = vec![
            Control {
                x: Some(4),
                y: Some(4),
                width: Some(20),
                height: Some(20),
                ..Control::new(ControlKind::RowMark)
            },
            field("sku", 30),
            field("qty", 240),
        ];
        let calc = app.session.calc_mut();
        calc.add_module("procs", PROCEDURES).unwrap();
        calc.forms_mut().set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
    }
    settle(&mut harness);
    harness.run();
    harness
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

fn click(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness
        .get(By::new().label(label).role(Role::Button))
        .click();
    harness.run();
    settle(harness);
    harness.run();
}

fn mark(harness: &mut Harness<'_, GossiaApp>, row: usize) {
    harness.get_by_label(&format!("Mark row {row}")).click();
    harness.run();
    harness.run();
}

fn has_button(harness: &Harness<'_, GossiaApp>, label: &str) -> bool {
    harness
        .query(By::new().label(label).role(Role::Button))
        .is_some()
}

fn enabled(harness: &Harness<'_, GossiaApp>, label: &str) -> bool {
    !harness
        .get(By::new().label(label).role(Role::Button))
        .accesskit_node()
        .is_disabled()
}

#[test]
fn delete_marked_deletes_the_marked_rows_as_one_edit() {
    let mut harness = marking();
    assert!(has_button(&harness, "Delete (0)"));
    assert!(!enabled(&harness, "Delete (0)"), "nothing is marked");
    mark(&mut harness, 1);
    mark(&mut harness, 3);
    assert!(has_button(&harness, "Delete (2)"));
    click(&mut harness, "Delete (2)");
    let left = items(&harness);
    assert_eq!(left.len(), 1);
    assert_eq!(sku(&left[0]).as_deref(), Some("B-220"));
    assert!(has_button(&harness, "Delete (0)"));
    // Undo brings them back, unmarked.
    harness.state_mut().undo();
    settle(&mut harness);
    harness.run();
    assert_eq!(items(&harness).len(), 3);
    assert!(has_button(&harness, "Delete (0)"));
}

#[test]
fn toggle_all_marks_every_row_then_clears_them() {
    let mut harness = marking();
    mark(&mut harness, 2);
    click(&mut harness, "Toggle all");
    assert!(
        has_button(&harness, "Delete (3)"),
        "one unmarked: all marked"
    );
    click(&mut harness, "Toggle all");
    assert!(
        has_button(&harness, "Delete (0)"),
        "all marked: all cleared"
    );
}

#[test]
fn add_row_adds_an_item_that_becomes_the_current_row() {
    let mut harness = marking();
    click(&mut harness, "Add");
    assert_eq!(items(&harness).len(), 4);
    let chosen = harness
        .state()
        .calc()
        .chosen_contexts("Orders", "Default")
        .and_then(|c| c.get("row").cloned());
    assert_eq!(chosen, Some(Value::Int(3)));
    // One edit, with no blank row or draft.
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(items(&harness).len(), 3);
}

#[test]
fn marks_are_cleared_when_the_number_of_items_changes() {
    let mut harness = marking();
    mark(&mut harness, 1);
    assert!(has_button(&harness, "Delete (1)"));
    let item = items(&harness)[0].clone();
    {
        let app = harness.state_mut();
        app.session
            .calc_mut()
            .insert_item("Orders", &p("items"), 0, item)
            .unwrap();
        app.session.tick();
    }
    settle(&mut harness);
    harness.run();
    assert!(has_button(&harness, "Delete (0)"));
}

#[test]
fn a_procedure_reads_the_marks_as_ctx_marked() {
    let mut harness = marking();
    mark(&mut harness, 1);
    mark(&mut harness, 3);
    click(&mut harness, "Show marks");
    let note = items(&harness)[0]
        .as_object()
        .and_then(|o| o.get("note").map(|n| n.value().clone()));
    assert_eq!(note, Some(Value::Str("[0, 2]".into())));
}

/// The palette puts a row mark at the start of a repeater's row, and a rows
/// button on the header acting on that repeater; without a row mark the
/// button adds rows.
#[test]
fn the_palette_places_a_row_mark_and_a_rows_button() {
    let mut harness = marking();
    {
        let app = harness.state_mut();
        let mut design = FormDesign::new_canvas(None);
        let at = design.add_repeater("items").unwrap();
        design.controls[at].children = vec![field("sku", 8)];
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.add_control(Control::new(ControlKind::RowsButton)));
        assert!(app.add_control(Control::new(ControlKind::RowMark)));
        assert!(
            !app.add_control(Control::new(ControlKind::RowMark)),
            "one per repeater"
        );
        assert!(
            !app.add_control(Control::new(ControlKind::RowButton)),
            "one way or the other"
        );
        assert!(app.add_control(Control::new(ControlKind::RowsButton)));
    }
    let design = harness.state().design_of("Default");
    let header: Vec<_> = design.controls[0]
        .children
        .iter()
        .map(|c| (c.kind, c.repeater.clone(), c.rows_action()))
        .collect();
    let row = Some("row".to_owned());
    assert_eq!(
        header,
        vec![
            (ControlKind::RowsButton, row.clone(), RowsAction::AddRow),
            (ControlKind::RowsButton, row, RowsAction::DeleteMarked),
        ]
    );
    let (at, _) = design.row_mark_of(1).unwrap();
    assert_eq!(at[0], 1);
}
