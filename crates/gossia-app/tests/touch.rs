//! Touch and small screens (DESIGN.md, The web version on GitHub Pages,
//! 17d): the larger targets follow a finger, and a narrow window has one
//! menu button and side panels that fold away.

use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;

fn window(width: f32) -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 760.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    assert!(
        harness
            .state_mut()
            .session
            .wait_idle(Duration::from_secs(60))
    );
    harness.run();
    harness
}

fn interact_height(harness: &Harness<'_, GossiaApp>) -> f32 {
    harness.ctx.global_style().spacing.interact_size.y
}

fn touch(harness: &mut Harness<'_, GossiaApp>) {
    let pos = egui::pos2(600.0, 400.0);
    for phase in [egui::TouchPhase::Start, egui::TouchPhase::End] {
        harness.event(egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos,
            force: None,
        });
        harness.run();
    }
}

#[test]
fn a_touch_makes_the_targets_larger_and_a_mouse_click_small_again() {
    let mut harness = window(1300.0);
    let small = interact_height(&harness);
    touch(&mut harness);
    assert!(harness.state().touch);
    let large = interact_height(&harness);
    assert!(large >= 36.0 && large > small, "{small} then {large}");
    harness.event(egui::Event::PointerButton {
        pos: egui::pos2(600.0, 400.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    assert!(!harness.state().touch);
    assert_eq!(interact_height(&harness), small);
}

#[test]
fn a_wide_window_has_its_menus_side_by_side() {
    let harness = window(1300.0);
    assert!(harness.query_by_label("Menu").is_none());
    harness.get_by_label("File");
    harness.get_by_label("Tools");
}

#[test]
fn a_narrow_window_has_one_menu_button_with_the_menus_in_it() {
    let mut harness = window(420.0);
    assert!(harness.query_by_label("File").is_none());
    harness.get_by_label("Menu").click();
    harness.run();
    harness.get_by_label("File");
    harness.get_by_label("Tools");
}

#[test]
fn on_a_narrow_window_the_side_panel_folds_away() {
    let mut harness = window(420.0);
    harness.state_mut().show_issues = true;
    harness.run();
    assert!(!harness.state().side_unfolded);
    harness.get_by_label("Unfold the side panel").click();
    harness.run();
    assert!(harness.state().side_unfolded);
    harness.get_by_label("Fold the side panel").click();
    harness.run();
    assert!(!harness.state().side_unfolded);
    harness.get_by_label("Unfold the side panel");
}

#[test]
fn on_a_wide_window_the_side_panel_does_not_fold() {
    let mut harness = window(1300.0);
    harness.state_mut().show_issues = true;
    harness.run();
    assert!(harness.query_by_label("Unfold the side panel").is_none());
    assert!(harness.query_by_label("Fold the side panel").is_none());
}

#[test]
fn on_a_narrow_window_the_grid_scrolls_sideways_to_the_formula_column() {
    let mut harness = window(420.0);
    let formula = harness.get_by_label("Formula").rect();
    assert!(
        formula.left() > 420.0,
        "off the window at first: {formula:?}"
    );
    harness.event(egui::Event::PointerMoved(egui::pos2(200.0, 400.0)));
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-1000.0, 0.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    harness.run();
    let formula = harness.get_by_label("Formula").rect();
    assert!(formula.right() <= 420.0, "on the window: {formula:?}");
}

#[test]
fn on_a_wide_window_the_formula_column_takes_the_rest() {
    let harness = window(1300.0);
    let formula = harness.get_by_label("Formula").rect();
    assert!(formula.right() <= 1300.0, "{formula:?}");
}
