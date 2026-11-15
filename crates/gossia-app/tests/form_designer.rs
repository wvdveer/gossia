//! The designer of canvas forms (DESIGN.md, Later versions, "Form
//! canvases, 0.7.0", "The designer"): the stack of sections with their
//! names on the left, dragging a section's height and the form's width,
//! adding and removing repeaters, moving a control into another section,
//! and the property pages of the form, a section and a context field.

use std::time::Duration;

use egui::accesskit::Role;
use egui::{Modifiers, Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable};
use gossia_app::GossiaApp;
use gossia_app::form::View;
use gossia_core::form::FormDesign;

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

/// The Default form of the sample's Orders in layout mode: a header, the
/// items repeater section and a hidden footer.
fn designer() -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1000.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        assert!(app.set_view(View::Form("Default".to_owned())));
        app.set_layout(true);
    }
    harness.run();
    harness.run();
    harness
}

fn design(harness: &Harness<'_, GossiaApp>) -> FormDesign {
    harness.state().design_of("Default")
}

fn by_role(label: &str, role: Role) -> By<'_> {
    By::new().label(label).role(role)
}

fn rect_of(harness: &Harness<'_, GossiaApp>, label: &str, role: Role) -> Rect {
    harness.get(by_role(label, role)).rect()
}

fn drag(harness: &mut Harness<'_, GossiaApp>, from: Pos2, to: Pos2) {
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
        modifiers: Modifiers::NONE,
    });
    harness.run();
    harness.remove_cursor();
    harness.run();
}

#[test]
fn the_sections_are_named_on_the_left_and_a_click_on_a_name_selects_one() {
    let mut harness = designer();
    for name in [
        "Form header",
        "Repeater: items (row)",
        "Form footer (hidden)",
    ] {
        assert!(
            harness.query_by_label(name).is_some(),
            "{name} is in the strip"
        );
    }
    // Nothing selected: the form's own page.
    assert!(
        harness
            .query(by_role("Width of the form", Role::SpinButton))
            .is_some()
    );
    harness
        .get(by_role("Repeater: items (row)", Role::Button))
        .click();
    harness.run();
    assert_eq!(harness.state().selected_control(), Some(vec![1]));
    assert!(harness.query_by_label("Height of the section").is_some());
    assert!(
        harness
            .query(by_role("Row name", Role::TextInput))
            .is_some()
    );
    assert!(harness.query_by_label("Remove repeater").is_some());
    // The header has no row name, and cannot be removed.
    harness.get(by_role("Form header", Role::Button)).click();
    harness.run();
    assert_eq!(harness.state().selected_control(), Some(vec![0]));
    assert!(
        harness
            .query(by_role("Row name", Role::TextInput))
            .is_none()
    );
    assert!(harness.query_by_label("Remove repeater").is_none());
}

#[test]
fn dragging_the_line_under_a_section_sets_its_height_as_one_change() {
    let mut harness = designer();
    let before = design(&harness).controls[0].height.unwrap();
    let line = rect_of(&harness, "Height of Form header", Role::Splitter);
    drag(
        &mut harness,
        line.center(),
        line.center() + egui::vec2(0.0, 41.0),
    );
    let snapped = ((before as f32 + 41.0) / 4.0).round() as u32 * 4;
    assert_eq!(design(&harness).controls[0].height, Some(snapped));
    harness.state_mut().layout_undo();
    harness.run();
    assert_eq!(design(&harness).controls[0].height, Some(before));
}

#[test]
fn a_section_is_not_made_shorter_than_its_lowest_control() {
    let mut harness = designer();
    let line = rect_of(&harness, "Height of Form header", Role::Splitter);
    drag(
        &mut harness,
        line.center(),
        line.center() - egui::vec2(0.0, 400.0),
    );
    let d = design(&harness);
    let height = d.controls[0].height.unwrap();
    assert!(height > 100, "kept under its controls: {height}");
}

#[test]
fn dragging_the_right_edge_sets_the_width_of_the_form() {
    let mut harness = designer();
    let before = design(&harness).form_width();
    let edge = rect_of(&harness, "Width of the form", Role::Splitter);
    let from = egui::pos2(edge.center().x, edge.top() + 20.0);
    drag(&mut harness, from, from + egui::vec2(101.0, 0.0));
    let snapped = ((before as f32 + 101.0) / 4.0).round() as u32 * 4;
    assert_eq!(design(&harness).form_width(), snapped);
    // Not narrower than 240 points.
    let edge = rect_of(&harness, "Width of the form", Role::Splitter);
    let from = egui::pos2(edge.center().x, edge.top() + 20.0);
    drag(&mut harness, from, from - egui::vec2(900.0, 0.0));
    assert_eq!(design(&harness).form_width(), 240);
}

#[test]
fn add_repeater_asks_for_the_array_and_remove_repeater_takes_it_away() {
    let mut harness = designer();
    // Nothing selected: Add repeater on the form's page (the palette has no
    // repeater on a canvas form).
    assert_eq!(harness.query_all_by_label("Add repeater").count(), 1);
    harness.get_by_label("Add repeater").click();
    harness.run();
    harness.get_by_label("items").click();
    harness.run();
    harness.run();
    let d = design(&harness);
    // A section between the two repeaters, then the new one, before the
    // footer, selected.
    assert_eq!(d.controls.len(), 5);
    assert_eq!(d.controls[3].path.as_deref(), Some("items"));
    assert_eq!(d.row_name_of(3).as_deref(), Some("row2"));
    assert_eq!(harness.state().selected_control(), Some(vec![3]));
    // The section between starts hidden, at height 0.
    assert!(harness.query_by_label("Section (hidden)").is_some());
    assert!(
        harness
            .query(by_role("Repeater: items (row2)", Role::Button))
            .is_some()
    );
    // Removing it takes the section above it with it, it being the last.
    harness.get_by_label("Remove repeater").click();
    harness.run();
    let d = design(&harness);
    assert_eq!(d.controls.len(), 3);
    assert_eq!(d.row_name_of(1).as_deref(), Some("row"));
    assert_eq!(harness.state().selected_control(), None);
}

#[test]
fn the_menu_of_a_section_adds_and_removes_repeaters() {
    let mut harness = designer();
    harness
        .get(by_role("Repeater: items (row)", Role::Button))
        .click_secondary();
    harness.run();
    harness.get_by_label("Remove repeater").click();
    harness.run();
    let d = design(&harness);
    assert_eq!(d.controls.len(), 2, "only the header and the footer");
    assert!(d.repeater_sections().is_empty());
    // Selected, so that the form's page, with its own Add repeater, is not
    // shown.
    harness.get(by_role("Form header", Role::Button)).click();
    harness.run();
    harness
        .get(by_role("Form header", Role::Button))
        .click_secondary();
    harness.run();
    harness.get_by_label("Add repeater").click();
    harness.run();
    // The last: items is also among the fields not on the form now.
    harness
        .query_all(by_role("items", Role::Button))
        .last()
        .unwrap()
        .click();
    harness.run();
    let d = design(&harness);
    assert_eq!(
        d.repeater_sections(),
        vec![1],
        "no section before the first"
    );
}

#[test]
fn a_control_dragged_into_another_section_moves_there_keeping_its_path() {
    let mut harness = designer();
    let d = design(&harness);
    let (header, row) = (d.controls[0].children.len(), d.controls[1].children.len());
    let field = d.controls[0].children[1].path.clone().unwrap();
    let name = format!("Field {field}");
    let rect = rect_of(&harness, &name, Role::Button);
    let strip = rect_of(&harness, "Repeater: items (row)", Role::Button);
    let to = egui::pos2(strip.right() + 300.0, strip.center().y);
    drag(&mut harness, rect.left_center() + egui::vec2(20.0, 0.0), to);
    let d = design(&harness);
    assert_eq!(d.controls[0].children.len(), header - 1);
    assert_eq!(d.controls[1].children.len(), row + 1);
    let moved = d.controls[1].children.last().unwrap();
    assert_eq!(moved.path.as_deref(), Some(field.as_str()));
    assert_eq!(harness.state().selected_control(), Some(vec![1, row]));
    // Its path is read from the item there, which has no such field: it is
    // listed as broken.
    let spec = harness.state().calc().spec("Orders").unwrap().clone();
    assert!(d.problem(&spec, &[1, row]).is_some());
    // The drag is one change.
    harness.state_mut().layout_undo();
    harness.run();
    let d = design(&harness);
    assert_eq!(d.controls[0].children.len(), header);
    assert_eq!(d.controls[1].children.len(), row);
}

#[test]
fn a_control_added_with_a_section_selected_goes_on_that_section() {
    let mut harness = designer();
    let row = design(&harness).controls[1].children.len();
    harness.state_mut().select_control(Some(vec![1]));
    harness.run();
    harness.get_by_label("Add label").click();
    harness.run();
    let d = design(&harness);
    assert_eq!(d.controls[1].children.len(), row + 1);
    assert_eq!(harness.state().selected_control(), Some(vec![1, row]));
}

#[test]
fn a_field_on_the_header_becomes_a_context_field() {
    let mut harness = designer();
    harness.state_mut().select_control(Some(vec![0, 1]));
    harness.run();
    harness.get_by_label("Source of the field").click();
    harness.run();
    harness.get_by_label("Context").click();
    harness.run();
    let d = design(&harness);
    let c = d.get(&[0, 1]).unwrap();
    assert_eq!(c.context.as_deref(), Some("sel"));
    assert_eq!(c.path, None);
    assert!(
        harness
            .query_by_label("Type of the context field")
            .is_some()
    );
    harness.get_by_label("Hidden").click();
    harness.run();
    assert!(design(&harness).get(&[0, 1]).unwrap().hidden);
    // A field in a row cannot be one.
    harness.state_mut().select_control(Some(vec![1, 0]));
    harness.run();
    harness.get_by_label("Source of the field").click();
    harness.run();
    assert!(harness.query_by_label("Context").is_none());
}
