//! Visual snapshot tests (DESIGN.md, "Testing and CI", level 7). Key screens
//! are rendered with wgpu and compared with stored images with a small
//! tolerance. They need a graphics adapter (a software Vulkan driver, Mesa
//! lavapipe, on Linux CI), so they run only when `CLIOTREE_SNAPSHOTS` is set:
//! `cargo xtask snapshots`.
//!
//! The images differ between operating systems, so each has its own directory
//! under `tests/snapshots/`. `UPDATE_SNAPSHOTS=1` writes the ones that differ.
//! A failing run leaves `*.new.png` and `*.diff.png` beside the stored image,
//! which the CI job uploads.

use std::time::Duration;

use cliotree_app::ClioApp;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};

fn enabled() -> bool {
    let on = std::env::var_os("CLIOTREE_SNAPSHOTS").is_some_and(|v| !v.is_empty());
    if !on {
        eprintln!("skipped: set CLIOTREE_SNAPSHOTS=1 (needs a graphics adapter)");
    }
    on
}

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .wgpu()
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    settle(&mut harness);
    harness
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

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

/// Chooses a view of the current tab: View, Show as, the view.
fn show_as(harness: &mut Harness<'_, ClioApp>, view: &str) {
    harness.get_by_label("View").click();
    harness.run();
    harness.get_by_label("Show as").click();
    harness.run();
    harness.get_by_label(view).click();
    harness.run();
}

fn options() -> SnapshotOptions {
    let directory = format!("tests/snapshots/{}", std::env::consts::OS);
    // A few pixels may differ between drivers; a moved or missing widget differs
    // by far more.
    SnapshotOptions::new()
        .output_path(directory)
        .max_failed_pixels(50)
}

fn check(harness: &mut Harness<'_, ClioApp>, name: &str, results: &mut SnapshotResults) {
    results.add(harness.try_snapshot_options(name, &options()));
}

#[test]
fn the_sample_workbook() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    harness.state_mut().reveal(&"customer".parse().unwrap());
    harness.run();
    check(&mut harness, "tree_of_the_sample", &mut results);

    click(&mut harness, "Prices");
    check(&mut harness, "prices_tab", &mut results);
}

#[test]
fn the_tabular_view_and_its_dialogs() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    harness
        .state_mut()
        .set_tabular(&"items".parse().unwrap(), true);
    harness.run();
    settle(&mut harness);
    check(&mut harness, "table_of_items", &mut results);

    harness.get_by_label("sku").click_secondary();
    harness.run();
    check(&mut harness, "column_menu", &mut results);
    click(&mut harness, "Filter...");
    check(&mut harness, "filter_dialog", &mut results);
}

#[test]
fn errors_are_marked_and_listed() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    click(&mut harness, "0.21");
    harness.event(egui::Event::Text("=1 / 0".to_owned()));
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    settle(&mut harness);
    click(&mut harness, "View");
    click(&mut harness, "Issues panel");
    check(&mut harness, "errors_and_issues", &mut results);
}

#[test]
fn the_insert_function_dialog_and_completion() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    click(&mut harness, "0.21");
    harness.event(egui::Event::Text("=Orders.".to_owned()));
    harness.run();
    check(&mut harness, "completion", &mut results);
    harness.key_combination_modifiers(Modifiers::NONE, &[Key::Escape]);
    harness.run();

    click(&mut harness, "fx");
    check(&mut harness, "insert_function", &mut results);
}

#[test]
fn a_schema_tab_and_its_field_dialog() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    click(&mut harness, "Orders schema");
    check(&mut harness, "schema_tab", &mut results);
    click(&mut harness, "Auto-update");
    click(&mut harness, "Orders");
    harness.get_by_label("customer").click_secondary();
    harness.run();
    click(&mut harness, "Add field...");
    check(&mut harness, "add_field_dialog", &mut results);
}

#[test]
fn the_default_form() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    show_as(&mut harness, "Default");
    check(&mut harness, "default_form_orders", &mut results);

    click(&mut harness, "Prices");
    show_as(&mut harness, "Default");
    check(&mut harness, "default_form_prices", &mut results);
}

#[test]
fn the_raw_view() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    show_as(&mut harness, "Raw");
    check(&mut harness, "raw_view_orders", &mut results);
}

#[test]
fn layout_mode_and_a_custom_form() {
    if !enabled() {
        return;
    }
    let mut results = SnapshotResults::new();
    let mut harness = window();
    show_as(&mut harness, "Default");
    harness.get_by_label("Edit form").click();
    harness.run();
    click(&mut harness, "Field customer.name");
    check(&mut harness, "layout_mode_orders", &mut results);

    // A form of its own: the customer group in two columns, no layout mode.
    harness.get_by_label("Edit form").click();
    harness.run();
    harness.state_mut().edit_design(|d| {
        d.get_mut(&[0])?.columns = Some(2);
        Some(None)
    });
    harness.run();
    check(&mut harness, "two_column_form_orders", &mut results);
}
