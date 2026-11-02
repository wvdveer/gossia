//! Tests of report tabs in the window (DESIGN.md, "Reports" and "Managing
//! tabs"): making one with *New tab...* or *New report...*, duplicating,
//! renaming and deleting it, and its design in the code editor.

use std::time::Duration;

use cliotree_core::Path;
use cliotree_core::form::{Control, ControlKind};
use cliotree_core::report::{ReportDesign, ReportKind};
use serde_json::json;

use crate::ClioApp;
use crate::code::CodeItem;
use crate::control;
use crate::tab_commands::NewKind;

fn sample() -> ClioApp {
    let mut app = ClioApp::sample();
    app.files.browse = false;
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app
}

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

/// Makes a report with *New report...*, as the dialog would.
fn new_report(app: &mut ClioApp, name: &str) {
    app.begin_new_report();
    let mut dialog = app.tab_dialogs.new.take().unwrap();
    dialog.name = name.to_owned();
    app.create_tab(&dialog).unwrap();
}

#[test]
fn new_report_offers_a_free_name_and_makes_one_entry() {
    let mut app = sample();
    app.begin_new_report();
    let dialog = app.tab_dialogs.new.take().unwrap();
    assert_eq!(dialog.kind, NewKind::Report);
    assert_eq!(dialog.name, "Report");
    assert_eq!(dialog.report_kind, ReportKind::SinglePage);
    assert!(app.new_tab_problem(&dialog).is_none());
    app.create_tab(&dialog).unwrap();

    assert_eq!(app.calc().tab_names().last().unwrap(), "Report");
    assert_eq!(app.sheet, "Report");
    assert!(app.in_report() && !app.in_tree() && !app.in_form());
    assert!(app.view_names().is_empty());
    assert_eq!(app.calc().undo_label(), Some("New report Report"));
    // A report is not a data tab.
    assert!(app.calc().data("Report").is_none());

    // The next one gets the next number.
    app.begin_new_report();
    assert_eq!(app.tab_dialogs.new.take().unwrap().name, "Report2");

    app.undo();
    app.follow_tabs();
    assert!(app.calc().report("Report").is_none());
    assert_ne!(app.sheet, "Report");
}

#[test]
fn a_report_name_is_checked_as_any_tab_name() {
    let mut app = sample();
    app.begin_new_report();
    let mut dialog = app.tab_dialogs.new.take().unwrap();
    dialog.name = "orders".to_owned();
    let why = app.new_tab_problem(&dialog).unwrap();
    assert!(why.starts_with("The tab name:"), "{why}");
    dialog.name = "not a name".to_owned();
    assert!(app.new_tab_problem(&dialog).is_some());
}

#[test]
fn a_report_can_start_from_an_array_of_a_tab() {
    let mut app = sample();
    let starts = app.starting_points();
    let shown: Vec<&str> = starts.iter().map(|(.., s)| s.as_str()).collect();
    assert!(shown.contains(&"Orders.items"), "{shown:?}");
    // Schema tabs are not data to report on.
    assert!(!shown.iter().any(|s| s.starts_with("Orders_schema")));

    app.begin_new_report();
    let mut dialog = app.tab_dialogs.new.take().unwrap();
    dialog.name = "Lines".to_owned();
    dialog.report_kind = ReportKind::Multipage;
    dialog.start = Some(("Orders".to_owned(), p("items")));
    app.create_tab(&dialog).unwrap();

    let design = app.calc().report("Lines").unwrap();
    assert_eq!(design.kind, ReportKind::Multipage);
    let repeater = &design.layout.controls[0];
    assert_eq!(repeater.kind, ControlKind::Repeater);
    assert_eq!(repeater.items.as_deref(), Some("Orders.items"));
    assert!(design.problems().is_empty(), "{:?}", design.problems());
}

#[test]
fn choosing_report_in_new_tab_keeps_the_data_choices_out() {
    let mut app = sample();
    app.begin_new_tab();
    let mut dialog = app.tab_dialogs.new.take().unwrap();
    dialog.kind = NewKind::Report;
    dialog.name = "Summary".to_owned();
    // The schema name of a data tab is not checked for a report.
    dialog.schema_name = "Orders".to_owned();
    assert!(app.new_tab_problem(&dialog).is_none());
    let before = app.calc().sheet_names().count();
    app.create_tab(&dialog).unwrap();
    assert_eq!(app.calc().sheet_names().count(), before);
    assert!(app.calc().is_report("Summary"));
}

#[test]
fn duplicate_puts_the_copy_after_its_report_and_shows_it() {
    let mut app = sample();
    new_report(&mut app, "Sales");
    app.move_tab("Sales", 0);
    let copy = app.duplicate_report_tab("Sales").unwrap();
    assert_eq!(copy, "Sales_2");
    assert_eq!(app.calc().tab_names()[..2], ["Sales", "Sales_2"]);
    assert_eq!(app.sheet, "Sales_2");
    assert_eq!(app.calc().undo_label(), Some("Duplicate report Sales"));
}

#[test]
fn deleting_a_report_shows_the_next_tab_and_undo_puts_it_back() {
    let mut app = sample();
    new_report(&mut app, "Sales");
    app.move_tab("Sales", 0);
    let next = app.calc().tab_names()[1].clone();
    assert!(app.calc().delete_problem("Sales").is_none());
    app.delete_tab("Sales").unwrap();
    assert_eq!(app.sheet, next);
    assert!(!app.calc().tab_names().contains(&"Sales".to_owned()));
    app.undo();
    app.follow_tabs();
    assert_eq!(app.calc().tab_names()[0], "Sales");
}

#[test]
fn the_current_report_tab_stays_shown_from_frame_to_frame() {
    let mut app = sample();
    new_report(&mut app, "Sales");
    app.follow_tabs();
    app.follow_tabs();
    assert_eq!(app.sheet, "Sales");
}

#[test]
fn a_renamed_report_keeps_its_design_open_in_the_code_editor() {
    let mut app = sample();
    new_report(&mut app, "Sales");
    app.open_code_editor();
    app.open_report_design("Sales");
    app.rename_tab("Sales", "Revenue").unwrap();
    assert_eq!(app.sheet, "Revenue");
    app.sync_code(0.0);
    assert!(
        app.code
            .tabs
            .iter()
            .any(|t| t.item == CodeItem::Report("Revenue".to_owned()))
    );
}

#[test]
fn the_code_editor_takes_a_report_design_that_a_report_can_have() {
    let mut app = sample();
    new_report(&mut app, "Sales");
    let undo = app.calc().undo_label().map(str::to_owned);

    // A field that shows an expression is fine, and not in the history.
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    let mut field = Control::new(ControlKind::Field);
    field.value = Some("SUM(i.qty for i in Orders.items)".to_owned());
    design.layout.controls.push(field);
    app.put_report_text("Sales", &design.to_json()).unwrap();
    assert_eq!(app.calc().report("Sales"), Some(&design));
    assert_eq!(app.calc().undo_label().map(str::to_owned), undo);
    assert!(app.calc().is_modified());

    // A button is not.
    let mut bad = design.clone();
    bad.layout.controls.push(Control::new(ControlKind::Button));
    let why = app.put_report_text("Sales", &bad.to_json()).unwrap_err();
    assert!(why.starts_with("control 1:"), "{why}");
    assert!(why.contains("buttons"), "{why}");
    assert_eq!(app.calc().report("Sales"), Some(&design));

    // Nor is text that is not a design.
    assert!(app.put_report_text("Sales", "{").is_err());
}

#[test]
fn the_control_channel_makes_and_duplicates_reports() {
    let mut app = sample();
    control::run(
        &mut app,
        "new_tab",
        &json!({"name": "Sales", "report": true, "kind": "multipage",
                "start_tab": "Orders", "start_path": "items"}),
    )
    .unwrap();
    assert_eq!(
        app.calc().report("Sales").unwrap().kind,
        ReportKind::Multipage
    );
    let state = control::run(&mut app, "status", &json!(null)).unwrap();
    assert_eq!(state["view"], "Report");
    let copy = control::run(&mut app, "duplicate_tab", &json!({"name": "Sales"})).unwrap();
    assert_eq!(copy["name"], "Sales_2");
    control::run(&mut app, "tab", &json!({"name": "Sales"})).unwrap();
    assert_eq!(app.sheet, "Sales");
    assert!(
        control::run(
            &mut app,
            "new_tab",
            &json!({"name": "Bad", "report": true, "kind": "sideways"})
        )
        .is_err()
    );
}
