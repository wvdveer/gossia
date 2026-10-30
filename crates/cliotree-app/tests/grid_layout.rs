//! The Grid layout of a group in the form designer (DESIGN.md, "Forms",
//! "Layout"): rows and columns with sizes, controls in cells, moved to
//! another cell by dragging, and turned on from the property page.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::form::{Control, GroupLayout, TrackSize};
use egui::{Modifiers, Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable};

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

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    assert!(
        harness
            .state_mut()
            .set_view(View::Form("Default".to_owned()))
    );
    harness
}

/// The Default form of Orders, its customer group (the first control) a Grid
/// changed by `change`, in layout mode when `layout`.
fn grid_form(layout: bool, change: impl FnOnce(&mut Control)) -> Harness<'static, ClioApp> {
    let mut harness = window();
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        let group = design.get_mut(&[0]).unwrap();
        group.layout = Some(GroupLayout::Grid);
        change(group);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        app.set_layout(layout);
    }
    settle(&mut harness);
    // Sizes are learnt from the frame before.
    harness.run();
    harness
}

fn name(harness: &Harness<'_, ClioApp>, at: &[usize]) -> String {
    let design = harness.state().design_of("Default");
    format!("Field {}", design.get(at).unwrap().path.clone().unwrap())
}

fn button(label: &str) -> By<'_> {
    By::new().label(label).role(egui::accesskit::Role::Button)
}

fn target(harness: &Harness<'_, ClioApp>, at: &[usize]) -> Rect {
    let name = name(harness, at);
    harness.get(button(&name)).rect()
}

fn editor(harness: &Harness<'_, ClioApp>, field: &str) -> Rect {
    harness
        .get(
            By::new()
                .label(field)
                .role(egui::accesskit::Role::TextInput),
        )
        .rect()
}

fn cell(harness: &Harness<'_, ClioApp>, at: &[usize]) -> (Option<u32>, Option<u32>) {
    let design = harness.state().design_of("Default");
    let c = design.get(at).unwrap();
    (c.grid_row, c.grid_column)
}

fn drag(harness: &mut Harness<'_, ClioApp>, from: Pos2, to: Pos2) {
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
fn the_controls_of_a_grid_line_up_in_its_rows_and_columns() {
    // Two equal shares: name and city in the first row, vip and status in
    // the second, each in the next free cell.
    let harness = grid_form(false, |g| {
        g.grid_columns = vec![TrackSize::Share(1), TrackSize::Share(1)];
    });
    let name = editor(&harness, "Orders.customer.name");
    let city = editor(&harness, "Orders.customer.city");
    let status = harness.get_by_label("Orders.customer.status").rect();
    assert!((name.top() - city.top()).abs() < 1.0, "{name:?} {city:?}");
    assert!(city.left() > name.right(), "{name:?} {city:?}");
    assert!(status.top() > name.bottom(), "{status:?}");
    // The second column starts at the same place in both rows.
    assert!(
        (city.left() - status.left()).abs() < 1.0,
        "{city:?} {status:?}"
    );
}

#[test]
fn a_column_of_points_is_that_wide() {
    let harness = grid_form(false, |g| {
        g.grid_columns = vec![TrackSize::Points(600), TrackSize::Auto];
    });
    let name = editor(&harness, "Orders.customer.name");
    let city = editor(&harness, "Orders.customer.city");
    // Both editors are the same distance after their labels: the second
    // column starts 600 points and a gap after the first.
    let moved = city.left() - name.left();
    assert!((600.0..620.0).contains(&moved), "moved {moved}");
}

#[test]
fn dragging_a_control_puts_it_in_the_cell_where_it_is_let_go() {
    let mut harness = grid_form(true, |g| {
        g.grid_columns = vec![TrackSize::Share(1), TrackSize::Share(1)];
    });
    let city = target(&harness, &[0, 1]);
    let vip = target(&harness, &[0, 2]);
    // Below the last row, in the first column: a new third row.
    let from = city.left_center() + egui::vec2(20.0, 0.0);
    let to = egui::pos2(vip.left() + 20.0, vip.bottom() + 16.0);
    drag(&mut harness, from, to);
    assert_eq!(cell(&harness, &[0, 1]), (Some(3), Some(1)));
    assert_eq!(harness.state().selected_control(), Some(vec![0, 1]));
    let moved = target(&harness, &[0, 1]);
    assert!(moved.top() > vip.bottom(), "{moved:?} {vip:?}");
    assert!((moved.left() - vip.left()).abs() < 1.0);
    // One change.
    harness.state_mut().layout_undo();
    harness.run();
    assert_eq!(cell(&harness, &[0, 1]), (None, None));
}

#[test]
fn a_new_control_dropped_on_a_cell_goes_in_it() {
    let mut harness = grid_form(true, |g| {
        g.grid_columns = vec![TrackSize::Share(1), TrackSize::Share(1)];
        for (i, c) in g.children.iter_mut().enumerate() {
            c.grid_row = Some(i as u32 + 1);
            c.grid_column = Some(1);
        }
        // Status in the second column of the last row.
        let status = &mut g.children[3];
        status.grid_column = Some(2);
    });
    let name = target(&harness, &[0, 0]);
    let status = target(&harness, &[0, 3]);
    let from = harness.get_by_label("Add label").rect().center();
    // The second column of the first row, which is empty.
    let to = egui::pos2(status.left() + 30.0, name.center().y);
    drag(&mut harness, from, to);
    let design = harness.state().design_of("Default");
    let label = &design.get(&[0]).unwrap().children[4];
    assert_eq!(label.text.as_deref(), Some("Label"));
    assert_eq!((label.grid_row, label.grid_column), (Some(1), Some(2)));
}

#[test]
fn the_property_page_turns_a_flow_into_a_grid_and_adds_columns() {
    let mut harness = window();
    harness.state_mut().set_layout(true);
    harness.run();
    let before: Vec<Rect> = (0..4).map(|i| target(&harness, &[0, i])).collect();
    harness.get_by_label("Group customer").click();
    harness.run();
    harness.get_by_label("Layout of the group").click();
    harness.run();
    harness.get_by_label("Grid").click();
    harness.run();
    harness.run();
    let design = harness.state().design_of("Default");
    let group = design.get(&[0]).unwrap();
    assert!(group.is_grid());
    assert_eq!(group.grid_columns, vec![TrackSize::Share(1)]);
    for i in 0..4 {
        assert_eq!(cell(&harness, &[0, i]), (Some(i as u32 + 1), Some(1)));
    }
    let after: Vec<Rect> = (0..4).map(|i| target(&harness, &[0, i])).collect();
    for (b, a) in before.iter().zip(&after) {
        assert!((a.left() - b.left()).abs() < 3.0, "{b:?} -> {a:?}");
    }
    // The rows and columns are listed with their sizes; one more column.
    assert!(harness.query_by_label("Size of column 1").is_some());
    assert!(harness.query_by_label("Size of row 4").is_some());
    harness.get_by_label("Add column").click();
    harness.run();
    let design = harness.state().design_of("Default");
    assert_eq!(
        design.get(&[0]).unwrap().grid_columns,
        vec![TrackSize::Share(1), TrackSize::Auto]
    );
    // A control in a grid shows its cell.
    harness.get(button(&name(&harness, &[0, 1]))).click();
    harness.run();
    assert!(harness.query_by_label("Cell in the grid").is_some());
    assert!(harness.query_by_label("Row of the cell").is_some());
}
