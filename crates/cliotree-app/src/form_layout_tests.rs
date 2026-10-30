//! Tests of named forms and of customising them, without a window.

use std::time::Duration;

use cliotree_core::form::{Control, ControlKind, Editor};
use cliotree_core::{Path, Value};

use crate::ClioApp;
use crate::form::{DEFAULT_FORM, View};
use crate::form_layout::FormCommand;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

/// The sample, showing the Default form of Orders.
fn app() -> ClioApp {
    let mut app = ClioApp::sample();
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    assert!(app.set_view(View::Form(DEFAULT_FORM.to_owned())));
    app
}

fn paths(app: &ClioApp) -> Vec<String> {
    app.design_of(&app.form_name())
        .walk()
        .iter()
        .filter_map(|(_, c)| c.path.clone())
        .collect()
}

#[test]
fn default_is_generated_until_the_first_change_and_is_then_stored() {
    let mut app = app();
    assert!(!app.is_customised(DEFAULT_FORM));
    let generated = app.design_of(DEFAULT_FORM);
    assert_eq!(generated.controls[0].path.as_deref(), Some("customer"));
    app.set_layout(true);
    assert!(app.layout_active());
    // Turning layout mode on changes nothing; the first edit stores the form.
    assert!(!app.is_customised(DEFAULT_FORM));
    assert!(app.edit_design(|d| d.remove(&[1]).map(|_| None)));
    assert!(app.is_customised(DEFAULT_FORM));
    assert_eq!(
        app.design_of(DEFAULT_FORM).controls.len(),
        generated.controls.len() - 1
    );
    // Taking the change back leaves the form generated again.
    app.layout_undo();
    assert!(!app.is_customised(DEFAULT_FORM));
    assert!(app.can_layout_redo());
    app.layout_redo();
    assert!(app.is_customised(DEFAULT_FORM));
    // Reset discards the design.
    assert!(app.reset_form().is_ok());
    assert!(!app.is_customised(DEFAULT_FORM));
    assert!(!app.layout_active(), "resetting leaves layout mode");
}

#[test]
fn a_new_form_starts_from_the_generated_one_and_is_shown() {
    let mut app = app();
    app.new_form("Packing").unwrap();
    assert_eq!(app.form_name(), "Packing");
    assert!(app.is_customised("Packing"));
    assert_eq!(app.design_of("Packing"), app.design_of(DEFAULT_FORM));
    let names: Vec<_> = app
        .view_names()
        .iter()
        .map(|v| v.label().to_owned())
        .collect();
    assert_eq!(names, ["Data tree", "Raw", "Default", "Packing"]);
    // Other tabs have only their own forms.
    app.switch_to("Prices");
    let names: Vec<_> = app
        .view_names()
        .iter()
        .map(|v| v.label().to_owned())
        .collect();
    assert_eq!(names, ["Data tree", "Raw", "Default"]);
}

#[test]
fn a_form_name_follows_the_rules_of_a_tab_name() {
    let mut app = app();
    app.new_form("Packing").unwrap();
    app.set_view(View::Form(DEFAULT_FORM.to_owned()));
    for (name, why) in [
        ("packing", "already"),
        ("DEFAULT", "already"),
        ("two words", "not a valid"),
        ("", "needs a name"),
        ("class", "keyword"),
        ("raw", "Raw"),
    ] {
        let error = app.new_form(name).unwrap_err();
        assert!(error.contains(why), "{name}: {error}");
    }
    assert_eq!(
        app.form_name(),
        DEFAULT_FORM,
        "a refused name changes nothing"
    );
}

#[test]
fn a_form_is_duplicated_renamed_and_deleted() {
    let mut app = app();
    app.new_form("Packing").unwrap();
    app.set_layout(true);
    app.edit_design(|d| d.remove(&[0]).map(|_| None));
    app.duplicate_form("Packing_copy").unwrap();
    assert_eq!(app.form_name(), "Packing_copy");
    assert_eq!(app.design_of("Packing_copy"), app.design_of("Packing"));
    assert!(!app.layout_active(), "a copy is shown, not edited");

    app.rename_form("Copy").unwrap();
    assert_eq!(app.form_name(), "Copy");
    assert!(!app.is_customised("Packing_copy"));
    let names: Vec<_> = app
        .view_names()
        .iter()
        .map(|v| v.label().to_owned())
        .collect();
    assert_eq!(names, ["Data tree", "Raw", "Default", "Packing", "Copy"]);

    app.delete_form().unwrap();
    assert_eq!(app.form_name(), DEFAULT_FORM);
    assert!(app.rename_form("Main").is_err(), "Default keeps its name");
    assert!(app.delete_form().is_err(), "Default cannot be deleted");
    app.set_view(View::Form("Packing".to_owned()));
    assert!(app.reset_form().is_err(), "only Default is reset");
}

#[test]
fn a_form_that_is_gone_shows_default() {
    let mut app = app();
    app.views
        .insert("Orders".to_owned(), View::Form("Missing".to_owned()));
    assert_eq!(app.form_name(), DEFAULT_FORM);
}

#[test]
fn a_dialog_waits_for_a_valid_name_and_keeps_its_reason() {
    let mut app = app();
    app.begin_form_command(FormCommand::New);
    app.form.dialog.as_mut().unwrap().name = "two words".to_owned();
    app.apply_form_dialog();
    assert!(app.form.dialog.as_ref().unwrap().error.is_some());
    app.form.dialog.as_mut().unwrap().name = "Packing".to_owned();
    app.apply_form_dialog();
    assert!(app.form.dialog.is_none());
    assert_eq!(app.form_name(), "Packing");
    // Rename starts from the form's own name.
    app.begin_form_command(FormCommand::Rename);
    assert_eq!(app.form.dialog.as_ref().unwrap().name, "Packing");
}

#[test]
fn controls_are_added_into_the_selected_group_or_after_the_selected_control() {
    let mut app = app();
    app.set_layout(true);
    let before = app.design_of(DEFAULT_FORM).controls.len();
    // Nothing selected: at the end.
    assert!(app.add_control(Control::new(ControlKind::Separator)));
    assert_eq!(app.selected_control(), Some(vec![before]));
    // A group is selected: into it.
    app.select_control(Some(vec![0]));
    assert!(app.add_control(Control::new(ControlKind::Separator)));
    let design = app.design_of(DEFAULT_FORM);
    assert_eq!(
        design.controls[0].children.last().unwrap().kind,
        ControlKind::Separator
    );
    assert_eq!(
        app.selected_control(),
        Some(vec![0, design.controls[0].children.len() - 1])
    );
    // A control that is not a group is selected: after it.
    app.select_control(Some(vec![1]));
    assert!(app.add_control(Control::new(ControlKind::Label)));
    assert_eq!(app.selected_control(), Some(vec![2]));
    assert_eq!(
        app.design_of(DEFAULT_FORM).controls[2].kind,
        ControlKind::Label
    );
}

#[test]
fn a_field_that_is_not_on_the_form_is_added_back() {
    let mut app = app();
    app.set_layout(true);
    app.edit_design(|d| d.remove(&[0, 1]).map(|_| None));
    let spec = app.calc().spec("Orders").unwrap().clone();
    let unplaced = app.design_of(DEFAULT_FORM).unplaced(&spec);
    assert_eq!(unplaced, [p("customer.city")]);
    assert!(app.add_field_to_form(&p("customer.city")));
    assert!(paths(&app).contains(&"customer.city".to_owned()));
    assert!(app.design_of(DEFAULT_FORM).unplaced(&spec).is_empty());
    assert!(!app.add_field_to_form(&p("nothing.here")));
}

#[test]
fn a_form_keeps_its_controls_when_the_schema_loses_their_fields() {
    let mut app = app();
    app.new_form("Packing").unwrap();
    // The form is stored, so it no longer follows the schema: a control whose
    // path the schema lacks is broken, and the rest still works.
    app.edit_design(|d| {
        d.get_mut(&[0, 0])?.path = Some("customer.nickname".to_owned());
        Some(None)
    });
    let spec = app.calc().spec("Orders").unwrap().clone();
    let broken = app.design_of("Packing").broken(&spec);
    assert_eq!(broken.len(), 1);
    assert_eq!(broken[0].0, vec![0, 0]);
    assert!(broken[0].1.contains("customer.nickname"));
}

#[test]
fn changes_to_a_design_are_not_edits_of_the_workbook() {
    let mut app = app();
    app.set_layout(true);
    let history = app.calc().history_len();
    let revision = app.calc().revision();
    app.add_control(Control::new(ControlKind::Separator));
    assert_eq!(app.calc().history_len(), history);
    assert_eq!(app.calc().revision(), revision);
}

#[test]
fn a_field_can_be_given_a_label_an_editor_and_made_read_only() {
    let mut app = app();
    app.set_layout(true);
    let edited = app.edit_design(|d| {
        let c = d.get_mut(&[0, 0])?;
        c.label = Some("Customer".to_owned());
        c.editor = Some(Editor::Multiline);
        c.read_only = true;
        c.masked = true;
        Some(Some(vec![0, 0]))
    });
    assert!(edited);
    let design = app.design_of(DEFAULT_FORM);
    let c = design.get(&[0, 0]).unwrap();
    assert_eq!(c.label.as_deref(), Some("Customer"));
    assert!(c.read_only);
    assert!(c.masked);
    // The document it is stored as round-trips.
    let text = design.to_json();
    assert_eq!(
        cliotree_core::form::FormDesign::from_json(&text).unwrap(),
        design
    );
}

#[test]
fn the_formula_of_a_field_is_entered_from_the_property_page_and_is_undoable() {
    let mut app = app();
    app.set_layout(true);
    let before = app.calc().history_len().0;
    app.commit_formula(&p("tax_rate"), "0.25");
    assert!(app.notice.is_none(), "{:?}", app.notice);
    assert_eq!(
        app.calc().formula_text("Orders", &p("tax_rate")).as_deref(),
        Some("0.25")
    );
    assert_eq!(app.calc().history_len().0, before + 1);
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    assert_eq!(
        app.calc()
            .data("Orders")
            .unwrap()
            .get(&p("tax_rate"))
            .map(|n| n.value().clone()),
        Some(Value::Float(0.25))
    );
    app.commit_formula(&p("tax_rate"), "");
    assert!(app.calc().formula_text("Orders", &p("tax_rate")).is_none());
}

/// The order of the columns of the table of the items, hidden ones in
/// brackets.
fn column_order(app: &ClioApp) -> Vec<String> {
    let design = app.design_of(DEFAULT_FORM);
    let control = design.get(&[1]).unwrap();
    let names: Vec<String> = ["sku", "qty", "price", "total", "note"]
        .map(String::from)
        .to_vec();
    control
        .arrange_columns(&names)
        .iter()
        .map(|c| {
            if c.hidden {
                format!("({})", c.name)
            } else {
                c.name.clone()
            }
        })
        .collect()
}

#[test]
fn the_columns_of_a_table_are_moved_shown_and_given_properties() {
    use crate::form_layout::ColumnEdit;
    use cliotree_core::form::{Alignment, ColumnDesign};
    let mut app = app();
    app.set_layout(true);
    let items = vec![1];
    assert_eq!(
        app.design_of(DEFAULT_FORM).get(&items).unwrap().kind,
        ControlKind::Repeater
    );
    assert_eq!(
        column_order(&app),
        ["#", "sku", "qty", "price", "total", "note"]
    );
    // The row numbers are a column like the others.
    assert!(app.edit_column_place(&items, "#", ColumnEdit::Step(false)));
    assert!(app.edit_column_place(&items, "note", ColumnEdit::Show(false)));
    assert!(app.edit_column_place(&items, "price", ColumnEdit::Step(true)));
    assert_eq!(
        column_order(&app),
        ["sku", "#", "price", "qty", "total", "(note)"]
    );
    // Moving on the screen goes past the columns that are hidden, and not
    // past the last one that is shown.
    assert!(app.edit_column_place(&items, "qty", ColumnEdit::Show(false)));
    assert!(app.edit_column_place(&items, "price", ColumnEdit::Move(false)));
    assert_eq!(
        column_order(&app),
        ["sku", "#", "(qty)", "total", "price", "(note)"]
    );
    assert!(!app.edit_column_place(&items, "price", ColumnEdit::Move(false)));
    // The last column of the items that is shown stays shown.
    for name in ["price", "total"] {
        assert!(app.edit_column_place(&items, name, ColumnEdit::Show(false)));
    }
    assert!(!app.edit_column_place(&items, "sku", ColumnEdit::Show(false)));
    assert!(app.edit_column_place(&items, "#", ColumnEdit::Show(false)));
    // Each column has its own properties, kept with the table.
    app.select_table_column(items.clone(), "qty");
    // As the property page has it: the column as it is, changed.
    let qty = ColumnDesign {
        label: Some("Quantity".to_owned()),
        width: Some(70),
        align: Some(Alignment::Center),
        thousands: true,
        hidden: true,
        ..ColumnDesign::named("qty")
    };
    assert!(app.set_column(None, &items, qty.clone()));
    assert_eq!(
        app.selected_column(),
        Some((items.clone(), "qty".to_owned()))
    );
    let design = app.design_of(DEFAULT_FORM);
    let stored = design.get(&items).unwrap();
    let found = stored
        .table_columns
        .iter()
        .find(|c| c.name == "qty")
        .unwrap();
    assert_eq!(found.label.as_deref(), Some("Quantity"));
    assert!(found.hidden, "a property keeps the column hidden");
    // Each change is one step of the layout's undo.
    app.layout_undo();
    let design = app.design_of(DEFAULT_FORM);
    let found = design
        .get(&items)
        .unwrap()
        .table_columns
        .iter()
        .find(|c| c.name == "qty")
        .cloned();
    assert_eq!(found.and_then(|c| c.label), None);
}

#[test]
fn columns_back_in_their_first_order_and_shown_store_nothing() {
    use crate::form_layout::ColumnEdit;
    let mut app = app();
    app.set_layout(true);
    let items = vec![1];
    assert!(app.edit_column_place(&items, "qty", ColumnEdit::Step(true)));
    assert!(app.edit_column_place(&items, "note", ColumnEdit::Show(false)));
    assert!(
        !app.design_of(DEFAULT_FORM)
            .get(&items)
            .unwrap()
            .table_columns
            .is_empty()
    );
    assert!(app.edit_column_place(&items, "qty", ColumnEdit::Step(false)));
    assert!(app.edit_column_place(&items, "note", ColumnEdit::Show(true)));
    assert!(
        app.design_of(DEFAULT_FORM)
            .get(&items)
            .unwrap()
            .table_columns
            .is_empty()
    );
}

#[test]
fn a_dragged_control_goes_where_the_pointer_says() {
    use crate::form::{DropZone, drop_place};
    use crate::form_layout::{DragItem, drop_allowed};
    let rect = |top: f32, bottom: f32| {
        egui::Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(400.0, bottom))
    };
    let zone = |at: Vec<usize>, top, bottom, group| DropZone {
        at,
        rect: rect(top, bottom),
        group,
        free: None,
        grid: None,
    };
    // A field at the top, then a group of two fields.
    let drops = vec![
        zone(vec![0], 0.0, 20.0, false),
        zone(vec![1, 0], 50.0, 70.0, false),
        zone(vec![1, 1], 80.0, 100.0, false),
        zone(vec![1], 30.0, 110.0, true),
    ];
    let area = rect(0.0, 300.0);
    let place =
        |y: f32| drop_place(&drops, egui::pos2(100.0, y), area).map(|p| (p.parent, p.index));
    // By halves of a field.
    assert_eq!(place(5.0), Some((vec![], 0)));
    assert_eq!(place(15.0), Some((vec![], 1)));
    assert_eq!(place(95.0), Some((vec![1], 2)));
    // Near the edges of a group, before or after it; inside, among its fields.
    assert_eq!(place(32.0), Some((vec![], 1)));
    assert_eq!(place(108.0), Some((vec![], 2)));
    assert_eq!(place(40.0), Some((vec![1], 0)));
    assert_eq!(place(75.0), Some((vec![1], 1)));
    // Below everything: the end of the form; outside the form: nowhere.
    assert_eq!(place(200.0), Some((vec![], 2)));
    assert_eq!(place(400.0), None);
    // A group does not go into itself, and a control put back where it is
    // is no move.
    let group = DragItem::Control(vec![1]);
    assert!(!drop_allowed(&group, &[1], 0));
    assert!(!drop_allowed(&group, &[], 1));
    assert!(!drop_allowed(&group, &[], 2));
    assert!(drop_allowed(&group, &[], 0));
    assert!(drop_allowed(&DragItem::New(ControlKind::Label), &[1], 0));
}

#[test]
fn a_resize_sets_the_size_and_one_drag_is_one_step_of_undo() {
    let mut app = app();
    app.set_layout(true);
    let at = vec![0, 0];
    app.edit_design(|d| {
        d.get_mut(&[0, 0])?.fill = true;
        Some(None)
    });
    let merge = Some("resize:1".to_owned());
    // The frames of one drag.
    assert!(app.resize_control(&at, Some(Some(200)), None, merge.clone()));
    assert!(app.resize_control(&at, Some(Some(240)), Some(Some(60)), merge));
    let control = app.design_of(DEFAULT_FORM).get(&at).cloned().unwrap();
    assert_eq!((control.width, control.height), (Some(240), Some(60)));
    assert!(!control.fill, "a width ends fill");
    app.layout_undo();
    let control = app.design_of(DEFAULT_FORM).get(&at).cloned().unwrap();
    assert_eq!(
        (control.width, control.height, control.fill),
        (None, None, true)
    );
    app.layout_redo();
    // Back to automatic, for the width only.
    assert!(app.resize_control(&at, Some(None), None, None));
    let control = app.design_of(DEFAULT_FORM).get(&at).cloned().unwrap();
    assert_eq!((control.width, control.height), (None, Some(60)));
}

#[test]
fn a_dragged_size_snaps_to_the_grid_within_its_range() {
    use crate::form::snapped;
    use cliotree_core::form::CONTROL_WIDTHS;
    assert_eq!(snapped(201.0, true, &CONTROL_WIDTHS), 200);
    assert_eq!(snapped(203.0, true, &CONTROL_WIDTHS), 204);
    assert_eq!(snapped(203.0, false, &CONTROL_WIDTHS), 203);
    assert_eq!(snapped(3.0, true, &CONTROL_WIDTHS), 24);
    assert_eq!(snapped(9000.0, true, &CONTROL_WIDTHS), 2000);
}
