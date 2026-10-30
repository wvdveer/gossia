//! The Free layout of a group in the form designer (DESIGN.md, "Forms",
//! "Layout" and "The designer for sizes and places"): controls at positions,
//! moved by dragging with snapping, by the arrow keys, lined up by the align
//! tools, with anchors and a tab order.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_app::form::View;
use cliotree_core::form::{Anchor, Control, GroupLayout};
use egui::{Key, Modifiers, Pos2, Rect};
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

/// The Default form of Orders in layout mode, its customer group (the first
/// control) Free with its four fields one under the other, 40 points apart,
/// after `change`.
fn free_form(change: impl FnOnce(&mut Control)) -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    {
        let app = harness.state_mut();
        let mut design = app.design_of("Default");
        let group = design.get_mut(&[0]).unwrap();
        group.layout = Some(GroupLayout::Free);
        for (i, c) in group.children.iter_mut().enumerate() {
            c.x = Some(0);
            c.y = Some(i as u32 * 40);
        }
        change(group);
        app.session
            .calc_mut()
            .forms_mut()
            .set("Orders", "Default", design);
        assert!(app.set_view(View::Form("Default".to_owned())));
        app.set_layout(true);
    }
    settle(&mut harness);
    // The height of a Free group is learnt from the frame before.
    harness.run();
    harness
}

fn name(harness: &Harness<'_, ClioApp>, at: &[usize]) -> String {
    let design = harness.state().design_of("Default");
    format!("Field {}", design.get(at).unwrap().path.clone().unwrap())
}

fn target(harness: &Harness<'_, ClioApp>, at: &[usize]) -> Rect {
    let name = name(harness, at);
    harness
        .get(
            By::new()
                .label(name.as_str())
                .role(egui::accesskit::Role::Button),
        )
        .rect()
}

fn place(harness: &Harness<'_, ClioApp>, at: &[usize]) -> (Option<u32>, Option<u32>) {
    let design = harness.state().design_of("Default");
    let c = design.get(at).unwrap();
    (c.x, c.y)
}

fn button(label: &str) -> By<'_> {
    By::new().label(label).role(egui::accesskit::Role::Button)
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
fn the_controls_of_a_free_group_are_drawn_at_their_places() {
    let harness = free_form(|g| {
        g.children[1].x = Some(120);
        g.children[1].y = Some(8);
    });
    let origin = target(&harness, &[0, 0]).min;
    let city = target(&harness, &[0, 1]).min;
    assert!(
        (city.x - origin.x - 120.0).abs() < 1.0,
        "{city:?} {origin:?}"
    );
    assert!((city.y - origin.y - 8.0).abs() < 1.0, "{city:?} {origin:?}");
    // The group is as tall as its lowest control: what follows the group
    // is under it.
    let lowest = target(&harness, &[0, 3]);
    let next = harness.get(button("Field tax_rate")).rect();
    assert!(lowest.bottom() < next.top(), "{lowest:?} {next:?}");
}

#[test]
fn dragging_a_control_moves_it_on_the_grid_as_one_change() {
    let mut harness = free_form(|_| {});
    let rect = target(&harness, &[0, 1]);
    let from = rect.left_center() + egui::vec2(20.0, 0.0);
    drag(&mut harness, from, from + egui::vec2(61.0, 200.0));
    assert_eq!(
        place(&harness, &[0, 1]),
        (Some(60), Some(240)),
        "snapped to 4 points"
    );
    assert_eq!(harness.state().selected_control(), Some(vec![0, 1]));
    let moved = target(&harness, &[0, 1]);
    assert!((moved.left() - rect.left() - 60.0).abs() < 1.0);
    // The others stay where they are, and the order is unchanged.
    assert_eq!(place(&harness, &[0, 0]), (Some(0), Some(0)));
    assert!(name(&harness, &[0, 1]).ends_with("city"));
    // The frames of the drag are one step of undo.
    harness.state_mut().layout_undo();
    harness.run();
    assert_eq!(place(&harness, &[0, 1]), (Some(0), Some(40)));
}

#[test]
fn a_dragged_control_snaps_to_the_edge_of_another() {
    let mut harness = free_form(|_| {});
    let rect = target(&harness, &[0, 2]);
    let from = rect.left_center() + egui::vec2(20.0, 0.0);
    // Three points to the right of the left edge of the others: it snaps
    // to it, not to the grid's 4.
    drag(&mut harness, from, from + egui::vec2(3.0, 200.0));
    assert_eq!(place(&harness, &[0, 2]), (Some(0), Some(280)));
}

#[test]
fn the_arrow_keys_move_the_selected_control_by_a_point_or_a_grid_step() {
    let mut harness = free_form(|_| {});
    harness.state_mut().select_control(Some(vec![0, 3]));
    harness.run();
    harness.key_press(Key::ArrowRight);
    harness.run();
    harness.key_press(Key::ArrowRight);
    harness.run();
    harness.key_press_modifiers(Modifiers::SHIFT, Key::ArrowDown);
    harness.run();
    assert_eq!(place(&harness, &[0, 3]), (Some(2), Some(124)));
    // The presses are one change.
    harness.state_mut().layout_undo();
    harness.run();
    assert_eq!(place(&harness, &[0, 3]), (Some(0), Some(120)));
}

#[test]
fn ctrl_and_a_click_selects_several_and_the_align_tools_line_them_up() {
    let mut harness = free_form(|g| {
        g.children[1].x = Some(40);
    });
    let city = name(&harness, &[0, 1]);
    let vip = name(&harness, &[0, 2]);
    harness.get(button(&city)).click();
    harness.run();
    harness
        .get(button(&vip))
        .click_modifiers(Modifiers::COMMAND);
    harness.run();
    assert_eq!(
        harness.state().selected_controls(),
        vec![vec![0, 1], vec![0, 2]]
    );
    assert!(harness.query_by_label("2 controls selected").is_some());
    harness.get_by_label("Align left").click();
    harness.run();
    assert_eq!(place(&harness, &[0, 2]).0, Some(40), "follows the first");
    assert_eq!(place(&harness, &[0, 1]).0, Some(40));
    // Ctrl and a click again takes it out.
    harness
        .get(button(&vip))
        .click_modifiers(Modifiers::COMMAND);
    harness.run();
    assert_eq!(harness.state().selected_controls(), vec![vec![0, 1]]);
    assert!(harness.query_by_label("2 controls selected").is_none());
}

#[test]
fn an_anchor_to_the_right_keeps_the_distance_to_the_right_edge() {
    // Drawn with its places made for a group 500 wide, then 400 wide: the
    // group is 100 wider than they were made for, and the control 100 to
    // the right.
    let left_for = |base: u32| {
        let harness = free_form(|g| {
            g.base_width = Some(base);
            let city = &mut g.children[1];
            city.width = Some(200);
            city.anchor_x = Some(Anchor::End);
        });
        target(&harness, &[0, 1]).left() - target(&harness, &[0, 0]).left()
    };
    let moved = left_for(400) - left_for(500);
    assert!((moved - 100.0).abs() < 1.0, "moved {moved}");
}

#[test]
fn the_property_page_turns_a_group_free_without_moving_its_controls() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    assert!(
        harness
            .state_mut()
            .set_view(View::Form("Default".to_owned()))
    );
    harness.state_mut().set_layout(true);
    harness.run();
    let before: Vec<Rect> = (0..4).map(|i| target(&harness, &[0, i])).collect();
    harness.get_by_label("Group customer").click();
    harness.run();
    harness.get_by_label("Layout of the group").click();
    harness.run();
    harness.get_by_label("Free").click();
    harness.run();
    harness.run();
    let design = harness.state().design_of("Default");
    assert!(design.get(&[0]).unwrap().is_free());
    assert_eq!(place(&harness, &[0, 0]), (Some(0), Some(0)));
    let after: Vec<Rect> = (0..4).map(|i| target(&harness, &[0, i])).collect();
    for (b, a) in before.iter().zip(&after) {
        assert!((a.left() - b.left()).abs() < 3.0, "{b:?} -> {a:?}");
        assert!((a.top() - b.top()).abs() < 4.0, "{b:?} -> {a:?}");
    }
    // A Free group lists its controls in tab order; moving one later keeps
    // the group selected.
    assert!(harness.query_by_label("Tab order").is_some());
    let first = name(&harness, &[0, 0]);
    harness
        .get_by_label(format!("Later: {first}").as_str())
        .click();
    harness.run();
    assert_eq!(name(&harness, &[0, 1]), first);
    assert_eq!(harness.state().selected_control(), Some(vec![0]));
}

#[test]
fn the_property_page_places_a_control_by_its_numbers() {
    let mut harness = free_form(|_| {});
    harness.state_mut().select_control(Some(vec![0, 1]));
    harness.run();
    assert!(harness.query_by_label("Place in the group").is_some());
    harness.get_by_label("Anchor across").click();
    harness.run();
    harness.get_by_label("Right").click();
    harness.run();
    let design = harness.state().design_of("Default");
    assert_eq!(design.get(&[0, 1]).unwrap().anchor_x, Some(Anchor::End));
    // Anchoring does not move it.
    assert_eq!(place(&harness, &[0, 1]), (Some(0), Some(40)));
}

#[test]
fn a_new_control_dragged_into_a_free_group_goes_where_it_is_let_go() {
    let mut harness = free_form(|g| g.height = Some(260));
    let origin = target(&harness, &[0, 0]).min;
    let from = harness.get_by_label("Add label").rect().center();
    let to = origin + egui::vec2(301.0, 181.0);
    drag(&mut harness, from, to);
    let design = harness.state().design_of("Default");
    let group = design.get(&[0]).unwrap();
    assert_eq!(group.children.len(), 5);
    let label = &group.children[4];
    assert_eq!(label.text.as_deref(), Some("Label"));
    assert_eq!((label.x, label.y), (Some(300), Some(180)));
}
