//! Tests of opening and saving, without a window.

use std::path::PathBuf;
use std::time::Duration;

use gossia_core::{Path, Value};
use serde_json::json;

use crate::GossiaApp;
use crate::control;
use crate::file::{Limits, read_zip, write_zip};
use crate::form::{DEFAULT_FORM, View};
use crate::workbook_file::{Pending, new_workbook_id};

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

/// A folder of its own, removed when it goes out of scope.
struct Folder(PathBuf);

impl Folder {
    fn new(tag: &str) -> Folder {
        let path = std::env::temp_dir().join(format!(
            "gossia-test-{tag}-{}-{}",
            std::process::id(),
            new_workbook_id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The sample, calculated.
fn app() -> GossiaApp {
    let mut app = GossiaApp::sample();
    // These tests type a name into the text dialog.
    app.files.browse = false;
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app
}

fn settle(app: &mut GossiaApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

fn value(app: &GossiaApp, sheet: &str, path: &str) -> Option<Value> {
    app.calc()
        .data(sheet)?
        .get(&p(path))
        .map(|n| n.value().clone())
}

#[test]
fn a_workbook_is_saved_and_opened_again() {
    let folder = Folder::new("roundtrip");
    let mut app = app();
    assert!(!app.is_dirty());
    app.save_as(&folder.file("orders.gossia")).unwrap();
    assert_eq!(app.file_name(), "orders.gossia");

    app.open_path(&folder.file("orders.gossia")).unwrap();
    settle(&mut app);
    let names: Vec<_> = app.calc().sheet_names().collect();
    assert_eq!(
        names,
        ["Orders", "Prices", "Orders_schema", "Prices_schema"]
    );
    // Nothing in the sample breaks the rules of a file, its tab names included.
    assert_eq!(app.notice, None);
    assert_eq!(value(&app, "Orders", "total"), Some(Value::Float(62.92)));
    assert_eq!(
        app.calc().formula_text("Orders", &p("tax")).as_deref(),
        Some("round(Orders.subtotal * Orders.tax_rate, 2)")
    );
    // What the user saved is what they trust: it opens trusted, and runs.
    assert!(app.calc().is_trusted());
    assert!(!app.is_dirty());
}

#[test]
fn another_installation_does_not_trust_a_file_it_has_not_seen() {
    let folder = Folder::new("untrusted");
    let mut first = app();
    first.save_as(&folder.file("orders.gossia")).unwrap();

    // A different user config: nothing is recorded for this workbook.
    let mut second = GossiaApp::sample();
    second.open_path(&folder.file("orders.gossia")).unwrap();
    assert!(!second.calc().is_trusted());
    assert!(second.banner_shown());
    assert!(second.trust.changes.is_empty());
    // The stored values show, and nothing is run.
    settle(&mut second);
    assert_eq!(value(&second, "Orders", "total"), Some(Value::Float(62.92)));
    assert_eq!(second.calc().stale(), 0);

    // Trusting it runs it again.
    second.trust_workbook();
    settle(&mut second);
    assert!(second.calc().is_trusted());
    assert_eq!(value(&second, "Orders", "total"), Some(Value::Float(62.92)));
}

#[test]
fn a_file_changed_outside_the_application_says_what_changed() {
    let folder = Folder::new("changed");
    let mut app = app();
    app.save_as(&folder.file("orders.gossia")).unwrap();

    // Someone edits a formula in the zip.
    let path = folder.file("orders.gossia");
    let mut package = read_zip(&std::fs::read(&path).unwrap(), &Limits::default()).unwrap();
    let overlay = String::from_utf8(package.get("sheets/Orders/overlay.json").unwrap().to_vec())
        .unwrap()
        .replace("Orders.subtotal + Orders.tax", "Orders.subtotal * 100");
    package
        .insert("sheets/Orders/overlay.json", overlay.into_bytes())
        .unwrap();
    std::fs::write(&path, write_zip(&package).unwrap()).unwrap();

    app.open_path(&path).unwrap();
    assert!(!app.calc().is_trusted());
    assert!(app.banner_shown());
    let changes: Vec<_> = app.trust.changes.iter().map(|c| c.item.as_str()).collect();
    assert_eq!(changes, ["formula Orders!total"]);
}

#[test]
fn the_title_and_the_unsaved_mark_follow_the_changes() {
    let folder = Folder::new("title");
    let mut app = app();
    assert_eq!(app.title(), "Untitled - Gossia");
    app.save_as(&folder.file("orders.gossia")).unwrap();
    assert_eq!(app.title(), "orders.gossia - Gossia");

    app.calc_mut_for_tests_set("Orders", "customer.name", "Globex");
    assert!(app.is_dirty());
    assert_eq!(app.title(), "orders.gossia * - Gossia");
    app.save().unwrap();
    assert!(!app.is_dirty());
    // Undoing past the save is a change again.
    app.undo();
    assert!(app.is_dirty());
}

impl GossiaApp {
    fn calc_mut_for_tests_set(&mut self, sheet: &str, path: &str, text: &str) {
        self.session
            .calc_mut()
            .set_value(sheet, &p(path), Value::Str(text.into()))
            .unwrap();
        self.session.calc_mut().seal_history();
    }
}

#[test]
fn a_change_to_a_form_is_an_unsaved_change_and_is_saved() {
    let folder = Folder::new("forms");
    let mut app = app();
    assert!(app.set_view(View::Form(DEFAULT_FORM.to_owned())));
    app.save_as(&folder.file("orders.gossia")).unwrap();
    assert!(!app.is_dirty());
    app.new_form("Packing").unwrap();
    assert!(app.is_dirty());
    app.save().unwrap();
    assert!(!app.is_dirty());

    app.open_path(&folder.file("orders.gossia")).unwrap();
    assert_eq!(app.calc().forms().names("Orders"), ["Packing"]);
    assert!(!app.is_dirty(), "a file just opened has no unsaved changes");
}

#[test]
fn a_new_workbook_has_no_file_and_save_says_so() {
    let mut app = app();
    assert!(app.doc.path.is_none());
    let error = app.save().unwrap_err();
    assert!(error.contains("Save As"), "{error}");
}

#[test]
fn save_as_gives_the_copy_a_new_id_and_adds_the_extension() {
    let folder = Folder::new("saveas");
    let mut app = app();
    let first_id = app.doc.workbook_id.clone();
    app.save_as(&folder.file("one")).unwrap();
    assert_eq!(app.file_name(), "one.gossia");
    assert_ne!(app.doc.workbook_id, first_id);
    let id = app.doc.workbook_id.clone();
    // Saving over the same file keeps the id; another file gets another.
    app.save_as(&folder.file("one.gossia")).unwrap();
    assert_eq!(app.doc.workbook_id, id);
    app.save_as(&folder.file("two.gossia")).unwrap();
    assert_ne!(app.doc.workbook_id, id);
    assert!(app.calc().is_trusted());
    assert!(folder.file("one.gossia").exists() && folder.file("two.gossia").exists());
}

#[test]
fn saving_what_has_not_changed_gives_the_same_file() {
    let folder = Folder::new("same");
    let mut app = app();
    app.save_as(&folder.file("a.gossia")).unwrap();
    let first = std::fs::read(folder.file("a.gossia")).unwrap();
    app.save().unwrap();
    assert!(first == std::fs::read(folder.file("a.gossia")).unwrap());
}

#[test]
fn a_file_that_cannot_be_opened_leaves_the_workbook_as_it_was() {
    let folder = Folder::new("bad");
    let mut app = app();
    std::fs::write(folder.file("junk.gossia"), b"not a zip").unwrap();
    let error = app.open_path(&folder.file("junk.gossia")).unwrap_err();
    assert!(error.contains("not a zip"), "{error}");
    let error = app.open_path(&folder.file("missing.gossia")).unwrap_err();
    assert!(error.contains("missing.gossia"), "{error}");
    assert_eq!(app.calc().sheet_names().count(), 4);
    assert!(app.doc.path.is_none());
}

#[test]
fn a_newer_major_version_is_refused_with_the_version_that_is_needed() {
    let folder = Folder::new("newer");
    let mut app = app();
    app.save_as(&folder.file("a.gossia")).unwrap();
    let path = folder.file("a.gossia");
    let mut package = read_zip(&std::fs::read(&path).unwrap(), &Limits::default()).unwrap();
    let manifest = String::from_utf8(package.get("manifest.json").unwrap().to_vec())
        .unwrap()
        .replace("\"1.0\"", "\"3.2\"");
    package
        .insert("manifest.json", manifest.into_bytes())
        .unwrap();
    std::fs::write(&path, write_zip(&package).unwrap()).unwrap();
    let error = app.open_path(&path).unwrap_err();
    assert!(error.contains("3.2"), "{error}");
}

#[test]
fn the_dialog_keeps_its_reason_and_waits() {
    let folder = Folder::new("dialog");
    let mut app = app();
    app.begin_open();
    app.files.dialog.as_mut().unwrap().text = folder.file("missing.gossia").display().to_string();
    app.apply_file_dialog();
    assert!(app.files.dialog.as_ref().unwrap().error.is_some());
    app.files.dialog.as_mut().unwrap().text = String::new();
    app.apply_file_dialog();
    assert!(
        app.files
            .dialog
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Give the name")
    );

    app.begin_save_as();
    app.files.dialog.as_mut().unwrap().text = folder.file("made").display().to_string();
    app.apply_file_dialog();
    assert!(app.files.dialog.is_none());
    assert!(folder.file("made.gossia").exists());
}

#[test]
fn unsaved_changes_are_asked_about_before_opening_another_workbook() {
    let folder = Folder::new("unsaved");
    let mut app = app();
    app.save_as(&folder.file("a.gossia")).unwrap();
    // Nothing changed: the dialog opens at once.
    app.begin_open();
    assert!(app.files.dialog.is_some() && app.files.confirm.is_none());
    app.files.dialog = None;

    app.calc_mut_for_tests_set("Orders", "customer.name", "Globex");
    app.begin_open();
    assert_eq!(app.files.confirm, Some(Pending::Open));
    assert!(app.files.dialog.is_none());
    // Leaving without saving goes on to the dialog.
    let ctx = egui::Context::default();
    app.answer_unsaved(&ctx, false);
    assert!(app.files.dialog.is_some() && app.files.confirm.is_none());
    assert!(app.is_dirty(), "the changes were not saved");
}

#[test]
fn saving_from_the_question_keeps_the_changes_and_goes_on() {
    let folder = Folder::new("answer");
    let mut app = app();
    app.save_as(&folder.file("a.gossia")).unwrap();
    app.calc_mut_for_tests_set("Orders", "customer.name", "Globex");
    app.begin_open();
    let ctx = egui::Context::default();
    app.answer_unsaved(&ctx, true);
    assert!(!app.is_dirty());
    assert!(app.files.dialog.is_some());
    // The file has the change.
    app.open_path(&folder.file("a.gossia")).unwrap();
    assert_eq!(
        value(&app, "Orders", "customer.name"),
        Some(Value::Str("Globex".into()))
    );
}

#[test]
fn quitting_with_unsaved_changes_asks_first() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.request_quit(&ctx);
    assert!(app.files.confirm.is_none(), "nothing to save");
    app.calc_mut_for_tests_set("Orders", "customer.name", "Globex");
    app.request_quit(&ctx);
    assert_eq!(app.files.confirm, Some(Pending::Quit));
    app.answer_unsaved(&ctx, false);
    assert!(app.files.quit_confirmed);
}

#[test]
fn the_control_channel_saves_and_opens() {
    let folder = Folder::new("control");
    let mut app = app();
    let path = folder.file("c.gossia").display().to_string();
    control::run(&mut app, "save_as", &json!({ "path": path })).unwrap();
    let status = control::run(&mut app, "status", &json!({})).unwrap();
    assert_eq!(status["file"], json!(path));
    assert_eq!(status["modified"], json!(false));

    let opened = control::run(&mut app, "open_workbook", &json!({ "path": path })).unwrap();
    assert_eq!(opened["tabs"][0], json!("Orders"));
    assert_eq!(opened["trusted"], json!(true));
    control::run(&mut app, "save", &json!(null)).unwrap();

    let error = control::run(
        &mut app,
        "open_workbook",
        &json!({ "path": "no/such.gossia" }),
    )
    .unwrap_err();
    assert!(error.contains("such.gossia"), "{error}");
}
