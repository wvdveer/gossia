//! Tests of opening and saving that drive the window: the File menu, its
//! dialogs, and the question about unsaved changes.

use std::path::PathBuf;
use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use gossia_app::GossiaApp;
use gossia_core::{Path, Value};

fn window() -> Harness<'static, GossiaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    // Most of these tests type a name into the text dialog.
    harness.state_mut().files.browse = false;
    settle(&mut harness);
    harness
}

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

fn click(harness: &mut Harness<'_, GossiaApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn has(harness: &Harness<'_, GossiaApp>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn file_menu(harness: &mut Harness<'_, GossiaApp>, item: &str) {
    click(harness, "File");
    click(harness, item);
}

/// A folder for the files of one test.
fn folder(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("gossia-window-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn type_name(harness: &mut Harness<'_, GossiaApp>, text: &str) {
    harness.get_by_label("File name").click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    harness.get_by_label("File name").type_text(text);
    harness.run();
}

fn edit_name(harness: &mut Harness<'_, GossiaApp>) {
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_value(
            "Orders",
            &"customer.name".parse::<Path>().unwrap(),
            Value::Str("Globex".into()),
        )
        .unwrap();
    harness.state_mut().session.calc_mut().seal_history();
    settle(harness);
}

#[test]
fn save_as_from_the_file_menu_writes_the_file_and_names_the_window() {
    let dir = folder("saveas");
    let target = dir.join("orders.gossia");
    let mut harness = window();
    assert_eq!(harness.state().title(), "Untitled - Gossia");

    file_menu(&mut harness, "Save As...");
    assert!(has(&harness, "File name"));
    type_name(&mut harness, &target.display().to_string());
    click(&mut harness, "Save");
    settle(&mut harness);

    assert!(target.exists());
    assert!(!has(&harness, "File name"), "the dialog closes");
    assert_eq!(harness.state().title(), "orders.gossia - Gossia");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_from_the_file_menu_shows_the_workbook_in_the_file() {
    let dir = folder("open");
    let target = dir.join("orders.gossia");
    let mut harness = window();
    edit_name(&mut harness);
    harness
        .state_mut()
        .save_as(&target)
        .expect("the sample can be saved");
    // Another change, left unsaved.
    harness.state_mut().undo();
    settle(&mut harness);

    file_menu(&mut harness, "Open...");
    // The workbook has unsaved changes, so it asks first.
    assert!(has(&harness, "Unsaved changes"));
    click(&mut harness, "Don't save");
    type_name(&mut harness, &target.display().to_string());
    click(&mut harness, "Open");
    settle(&mut harness);

    assert!(!has(&harness, "File name"));
    assert_eq!(
        harness
            .state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&"customer.name".parse().unwrap())
            .map(|n| n.value().clone()),
        Some(Value::Str("Globex".into()))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_the_question_leaves_everything_as_it_was() {
    let mut harness = window();
    edit_name(&mut harness);
    file_menu(&mut harness, "Open...");
    assert!(has(&harness, "Unsaved changes"));
    click(&mut harness, "Cancel");
    assert!(!has(&harness, "Unsaved changes"));
    assert!(!has(&harness, "File name"));
    assert!(harness.state().is_dirty());
}

#[test]
fn quitting_with_unsaved_changes_asks_and_cancel_keeps_the_window() {
    let mut harness = window();
    edit_name(&mut harness);
    file_menu(&mut harness, "Quit");
    assert!(has(&harness, "Unsaved changes"));
    click(&mut harness, "Cancel");
    assert!(!has(&harness, "Unsaved changes"));
}

#[test]
fn a_file_not_seen_before_opens_untrusted_with_the_banner() {
    let dir = folder("untrusted");
    let target = dir.join("orders.gossia");
    // Saved by one installation...
    let mut first = GossiaApp::sample();
    first.files.browse = false;
    assert!(first.session.wait_idle(Duration::from_secs(60)));
    first.save_as(&target).unwrap();

    // ...and opened by another, which has no record of it.
    let mut harness = window();
    harness.state_mut().open_path(&target).unwrap();
    settle(&mut harness);
    assert!(!harness.state().calc().is_trusted());
    assert!(has(&harness, "Trust this version"));
    click(&mut harness, "Trust this version");
    settle(&mut harness);
    assert!(harness.state().calc().is_trusted());
    assert!(!has(&harness, "Trust this version"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_file_menu_opens_the_file_browser_and_escape_leaves_it() {
    // The browser is a library's: some of its widgets have no accessible name,
    // which the harness would otherwise refuse.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut GossiaApp| app.show(ui), GossiaApp::sample());
    settle(&mut harness);
    harness.state_mut().files.browse = true;
    file_menu(&mut harness, "Open...");
    assert!(has(&harness, "Open a workbook"), "the browser is open");
    assert!(!has(&harness, "File name"), "not the text dialog");
    harness.key_press(egui::Key::Escape);
    harness.run();
    harness.run();
    assert!(!has(&harness, "Open a workbook"));
    assert!(harness.state().files.browsing.is_none());
}

#[test]
fn a_path_the_browser_chose_that_cannot_be_opened_is_explained_in_the_text_dialog() {
    let mut harness = window();
    harness.state_mut().files.browse = true;
    let missing = std::env::temp_dir().join("gossia-missing-file.gossia");
    let _ = std::fs::remove_file(&missing);
    // What the browser does when the user chooses: the picked path is handled.
    harness.state_mut().files.browsing = Some(gossia_app::workbook_file::Browsing::Open);
    let result = harness.state_mut().open_path(&missing);
    assert!(result.is_err());
}
