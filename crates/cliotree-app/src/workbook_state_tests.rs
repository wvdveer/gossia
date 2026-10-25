//! Tests of what a workbook carries besides its tabs, and what the user keeps
//! apart from it (build step 8c): the view state in the overlay, the trust
//! record and the preferences on disk, the workbook settings, File > New and
//! the recent files. Without a window.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use cliotree_calc::{Setting, WorkbookSettings};
use cliotree_core::Path;
use serde_json::{Value as Json, json};

use crate::ClioApp;
use crate::control;
use crate::file::{Limits, read_zip, write_zip};
use crate::form::{DEFAULT_FORM, View};
use crate::prefs::{Config, TRUST_FILE, User};
use crate::table::{Condition, Filter};
use crate::workbook_file::{Pending, new_workbook_id};

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

/// A folder of its own, removed when it goes out of scope.
struct Folder(PathBuf);

impl Folder {
    fn new(tag: &str) -> Folder {
        let path = std::env::temp_dir().join(format!(
            "cliotree-state-{tag}-{}-{}",
            std::process::id(),
            new_workbook_id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// A user whose configuration is in this folder.
    fn user(&self) -> User {
        let (user, warnings) = User::load(Config::at(self.file("config")));
        assert!(warnings.is_empty(), "{warnings:?}");
        user
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn settle(app: &mut ClioApp) {
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

/// The sample, calculated, for a user with no configuration on disk.
fn sample() -> ClioApp {
    let mut app = ClioApp::sample();
    app.files.browse = false;
    settle(&mut app);
    app
}

fn entry(path: &std::path::Path, name: &str) -> Option<String> {
    let package = read_zip(&std::fs::read(path).unwrap(), &Limits::default()).unwrap();
    package
        .get(name)
        .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
}

// --------------------------------------------------------------- view state

/// A table of the items with a hidden column, a hidden row and a filter.
fn with_a_table(app: &mut ClioApp) {
    app.set_tabular(&p("items"), true);
    app.hide_column(&p("items"), "note");
    app.hide_row(&p("items"), 1);
    app.set_filter(
        &p("items"),
        "qty",
        Some(Filter {
            values: None,
            condition: Some(Condition::Greater("1".to_owned())),
            ignore_case: false,
        }),
    );
}

#[test]
fn the_view_of_each_tab_is_saved_and_comes_back() {
    let folder = Folder::new("views");
    let mut app = sample();
    with_a_table(&mut app);
    app.switch_to("Prices");
    assert!(app.set_view(View::Raw));
    app.switch_to("Orders");
    // View state is not an edit of the workbook.
    assert!(!app.is_dirty());
    app.save_as(&folder.file("v.clio")).unwrap();

    let overlay: Json =
        serde_json::from_str(&entry(&folder.file("v.clio"), "sheets/Orders/overlay.json").unwrap())
            .unwrap();
    assert_eq!(
        overlay["view"],
        json!({
            "tables": [{
                "array": "items",
                "hidden_columns": ["note"],
                "hidden_rows": [1],
                "filters": {"qty": {
                    "condition": {"is": "greater", "operands": ["1"]},
                    "ignore_case": false
                }}
            }],
            "table": "items"
        })
    );
    // A tab with no formulas still gets an overlay for its view.
    let prices: Json =
        serde_json::from_str(&entry(&folder.file("v.clio"), "sheets/Prices/overlay.json").unwrap())
            .unwrap();
    assert_eq!(prices["view"], json!({ "show": "raw" }));

    let mut again = sample();
    again.open_path(&folder.file("v.clio")).unwrap();
    settle(&mut again);
    assert!(again.is_tabular(&p("items")));
    assert_eq!(again.active_table(), Some(&p("items")));
    let view = again.view_of(&p("items")).unwrap();
    assert_eq!(view.hidden_columns, BTreeSet::from(["note".to_owned()]));
    let hidden: Vec<usize> = again.hidden_rows(&p("items")).iter().map(|r| r.0).collect();
    assert_eq!(hidden, [1]);
    assert_eq!(
        again.filter_of(&p("items"), "qty").unwrap().condition,
        Some(Condition::Greater("1".to_owned()))
    );
    again.refresh_table();
    // Item 1 is hidden by hand; item 0 (qty 2) and item 2 (qty 12) pass.
    assert_eq!(again.layout().unwrap().visible.rows, [0, 2]);
    again.switch_to("Prices");
    assert_eq!(again.current_view(), &View::Raw);

    // Saving what was opened gives the same file.
    let first = std::fs::read(folder.file("v.clio")).unwrap();
    again.save().unwrap();
    assert!(first == std::fs::read(folder.file("v.clio")).unwrap());
}

#[test]
fn a_hidden_row_stays_with_its_item_when_the_file_is_read() {
    let folder = Folder::new("rows");
    let mut app = sample();
    app.set_tabular(&p("items"), true);
    app.hide_row(&p("items"), 2);
    // A sort moves the hidden item; it is saved where it is now.
    app.sort_table(&p("items"), "sku", Some(true));
    let hidden: Vec<usize> = app.hidden_rows(&p("items")).iter().map(|r| r.0).collect();
    assert_eq!(hidden, [0]);
    app.save_as(&folder.file("r.clio")).unwrap();
    app.open_path(&folder.file("r.clio")).unwrap();
    let hidden: Vec<usize> = app.hidden_rows(&p("items")).iter().map(|r| r.0).collect();
    assert_eq!(hidden, [0]);
}

#[test]
fn a_form_view_comes_back_and_one_that_is_gone_is_dropped() {
    let folder = Folder::new("formview");
    let mut app = sample();
    assert!(app.set_view(View::Form(DEFAULT_FORM.to_owned())));
    app.new_form("Packing").unwrap();
    assert_eq!(app.current_view(), &View::Form("Packing".to_owned()));
    app.save_as(&folder.file("f.clio")).unwrap();
    app.open_path(&folder.file("f.clio")).unwrap();
    assert_eq!(app.current_view(), &View::Form("Packing".to_owned()));

    // A view of a form the file no longer has shows the data tree.
    let path = folder.file("f.clio");
    let mut package = read_zip(&std::fs::read(&path).unwrap(), &Limits::default()).unwrap();
    let overlay = entry(&path, "sheets/Orders/overlay.json")
        .unwrap()
        .replace("\"Packing\"", "\"Gone\"");
    package
        .insert("sheets/Orders/overlay.json", overlay.into_bytes())
        .unwrap();
    std::fs::write(&path, write_zip(&package).unwrap()).unwrap();
    app.open_path(&path).unwrap();
    assert_eq!(app.current_view(), &View::Tree);
}

#[test]
fn view_state_a_newer_version_wrote_is_kept() {
    let folder = Folder::new("newer");
    let mut app = sample();
    app.save_as(&folder.file("n.clio")).unwrap();
    let path = folder.file("n.clio");
    let mut package = read_zip(&std::fs::read(&path).unwrap(), &Limits::default()).unwrap();
    let mut overlay: Json =
        serde_json::from_str(&entry(&path, "sheets/Orders/overlay.json").unwrap()).unwrap();
    overlay["view"] = json!({ "zoom": 150, "show": "raw" });
    package
        .insert(
            "sheets/Orders/overlay.json",
            serde_json::to_vec_pretty(&overlay).unwrap(),
        )
        .unwrap();
    std::fs::write(&path, write_zip(&package).unwrap()).unwrap();

    app.open_path(&path).unwrap();
    assert_eq!(app.current_view(), &View::Raw);
    assert!(app.set_view(View::Tree));
    app.save().unwrap();
    let overlay: Json =
        serde_json::from_str(&entry(&path, "sheets/Orders/overlay.json").unwrap()).unwrap();
    assert_eq!(overlay["view"], json!({ "zoom": 150 }));
}

// -------------------------------------------------------- trust on disk

#[test]
fn a_saved_workbook_is_trusted_by_the_next_session_of_the_same_user() {
    let folder = Folder::new("trustdisk");
    let mut first = sample().with_user(folder.user());
    first.save_as(&folder.file("t.clio")).unwrap();
    assert!(folder.file("config").join(TRUST_FILE).exists());
    drop(first);

    // A new session reads the record from disk.
    let mut second = sample().with_user(folder.user());
    second.open_path(&folder.file("t.clio")).unwrap();
    assert!(second.calc().is_trusted());
    assert!(!second.banner_shown());
}

#[test]
fn trusting_a_workbook_from_the_banner_is_recorded_on_disk() {
    let folder = Folder::new("banner");
    let mut author = sample();
    author.save_as(&folder.file("b.clio")).unwrap();

    let mut reader = sample().with_user(folder.user());
    reader.open_path(&folder.file("b.clio")).unwrap();
    assert!(!reader.calc().is_trusted());
    reader.trust_workbook();
    settle(&mut reader);
    drop(reader);

    let mut later = sample().with_user(folder.user());
    later.open_path(&folder.file("b.clio")).unwrap();
    assert!(later.calc().is_trusted());
}

#[test]
fn an_untrusted_copy_keeps_what_was_recorded_for_the_original() {
    let folder = Folder::new("carry");
    let mut app = sample().with_user(folder.user());
    app.save_as(&folder.file("a.clio")).unwrap();
    app.open_path(&folder.file("a.clio")).unwrap();
    app.withhold_trust(Vec::new());
    app.save_as(&folder.file("b.clio")).unwrap();
    let id = app.doc.workbook_id.clone();
    assert!(folder.user().trust.is_recorded(&id));
}

// ------------------------------------------------------------- settings

fn long_formulas() -> WorkbookSettings {
    let mut settings = WorkbookSettings::default();
    settings.set(Setting::FormulaTimeLimit, 5_000);
    settings
}

#[test]
fn the_workbook_settings_govern_the_evaluation_thread() {
    let mut app = sample();
    let limit = |app: &ClioApp| app.session.evaluator().settings().formula_limit;
    assert_eq!(limit(&app), Some(Duration::from_secs(1)));
    app.session.calc_mut().set_settings(long_formulas());
    app.apply_settings();
    assert_eq!(limit(&app), Some(Duration::from_secs(5)));
    // Undoing the change puts the old limit back.
    app.undo();
    app.apply_settings();
    assert_eq!(limit(&app), Some(Duration::from_secs(1)));
    settle(&mut app);
}

#[test]
fn settings_are_saved_and_a_ceiling_holds_them_down_when_opened() {
    let folder = Folder::new("ceiling");
    let mut app = sample();
    app.session.calc_mut().set_settings(long_formulas());
    app.save_as(&folder.file("s.clio")).unwrap();
    assert_eq!(
        entry(&folder.file("s.clio"), "settings.json").as_deref(),
        Some("{\n  \"formula_time_limit_ms\": 5000\n}\n")
    );

    let mut user = folder.user();
    user.change_prefs(|p| p.ceilings.set(Setting::FormulaTimeLimit, 2_000))
        .unwrap();
    let mut other = app.with_user(user);
    other.open_path(&folder.file("s.clio")).unwrap();
    assert_eq!(other.calc().settings(), &long_formulas());
    assert_eq!(
        other.session.evaluator().settings().formula_limit,
        Some(Duration::from_secs(2))
    );
    let notice = other.notice.clone().unwrap_or_default();
    assert!(notice.contains("ceiling of 2000"), "{notice}");
}

#[test]
fn the_limits_for_reading_files_come_from_the_preferences() {
    let folder = Folder::new("limits");
    let mut app = sample();
    app.save_as(&folder.file("l.clio")).unwrap();
    let mut user = folder.user();
    user.change_prefs(|p| p.limits.entries = 3).unwrap();
    let mut app = app.with_user(user);
    let error = app.open_path(&folder.file("l.clio")).unwrap_err();
    assert!(error.contains("more than the limit of 3"), "{error}");
}

// ------------------------------------------------------------------ new

#[test]
fn file_new_gives_an_empty_data_tab_bound_to_its_schema() {
    let folder = Folder::new("new");
    let mut app = sample();
    app.save_as(&folder.file("old.clio")).unwrap();
    app.new_workbook();
    let tabs: Vec<_> = app.calc().sheet_names().collect();
    assert_eq!(tabs, ["Data", "DataSchema"]);
    assert_eq!(app.sheet, "Data");
    assert!(app.calc().auto_update("DataSchema"));
    assert!(app.calc().is_trusted());
    assert!(app.doc.path.is_none());
    assert!(!app.is_dirty());
    assert_eq!(app.title(), "Untitled - ClioTree");
    assert_eq!(app.calc().undo_label(), None);

    // It can be saved and opened like any other.
    app.save_as(&folder.file("new.clio")).unwrap();
    app.open_path(&folder.file("new.clio")).unwrap();
    let tabs: Vec<_> = app.calc().sheet_names().collect();
    assert_eq!(tabs, ["Data", "DataSchema"]);
}

#[test]
fn file_new_asks_about_unsaved_changes_first() {
    let mut app = sample();
    app.session
        .calc_mut()
        .set_value(
            "Orders",
            &p("customer.name"),
            cliotree_core::Value::Str("Globex".into()),
        )
        .unwrap();
    app.begin_new();
    assert_eq!(app.files.confirm, Some(Pending::New));
    assert_eq!(app.sheet, "Orders");
    let ctx = egui::Context::default();
    app.answer_unsaved(&ctx, false);
    assert_eq!(app.sheet, "Data");
}

#[test]
fn the_control_channel_makes_a_new_workbook() {
    let mut app = sample();
    let made = control::run(&mut app, "new_workbook", &json!(null)).unwrap();
    assert_eq!(made["tabs"], json!(["Data", "DataSchema"]));
    let status = control::run(&mut app, "status", &json!({})).unwrap();
    assert_eq!(status["tab"], "Data");
    assert_eq!(status["file"], Json::Null);
    assert_eq!(status["modified"], false);
}

// --------------------------------------------------------------- recent

#[test]
fn opened_and_saved_files_are_listed_newest_first_and_kept_on_disk() {
    let folder = Folder::new("recent");
    let mut app = sample().with_user(folder.user());
    app.save_as(&folder.file("a.clio")).unwrap();
    app.save_as(&folder.file("b.clio")).unwrap();
    app.open_path(&folder.file("a.clio")).unwrap();
    let names = |app: &ClioApp| -> Vec<String> {
        app.user
            .prefs
            .recent
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(names(&app), ["a.clio", "b.clio"]);
    let later = sample().with_user(folder.user());
    assert_eq!(names(&later), ["a.clio", "b.clio"]);
}

#[test]
fn a_recent_file_that_is_gone_is_taken_off_the_list() {
    let folder = Folder::new("gone");
    let mut app = sample().with_user(folder.user());
    app.save_as(&folder.file("a.clio")).unwrap();
    std::fs::remove_file(folder.file("a.clio")).unwrap();
    let recent = app.user.prefs.recent[0].clone();
    app.begin_open_recent(&recent);
    assert!(app.notice.as_deref().unwrap_or_default().contains("a.clio"));
    assert!(app.user.prefs.recent.is_empty());
    // The sample is still shown.
    assert_eq!(app.sheet, "Orders");
}
