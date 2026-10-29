//! Tests of the Options dialog (DESIGN.md, "The Options dialog"), without a
//! window.

use std::path::PathBuf;
use std::time::Duration;

use cliotree_calc::Setting;
use serde_json::json;

use crate::ClioApp;
use crate::control;
use crate::options::{Field, Page, Shown};
use crate::prefs::{Config, PREFERENCES_FILE, ThemeChoice, User};
use crate::workbook_file::new_workbook_id;

/// A folder of its own, removed when it goes out of scope.
struct Folder(PathBuf);

impl Folder {
    fn new(tag: &str) -> Folder {
        let path = std::env::temp_dir().join(format!(
            "cliotree-options-{tag}-{}-{}",
            std::process::id(),
            new_workbook_id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

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

fn sample() -> ClioApp {
    let mut app = ClioApp::sample();
    app.files.browse = false;
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app
}

fn formula_limit(app: &ClioApp) -> Option<Duration> {
    app.session.evaluator().settings().formula_limit
}

#[test]
fn numbers_are_shown_in_seconds_and_megabytes() {
    assert_eq!(Shown::Seconds.text(1_000), "1");
    assert_eq!(Shown::Seconds.text(500), "0.5");
    assert_eq!(Shown::Seconds.text(1_250), "1.25");
    assert_eq!(Shown::Seconds.parse("0.5"), Ok(500));
    assert_eq!(Shown::Seconds.parse(" 10 "), Ok(10_000));
    assert_eq!(Shown::Megabytes.text(16 << 20), "16");
    assert_eq!(Shown::Megabytes.parse("1.5"), Ok(3 << 19));
    assert_eq!(Shown::Count.parse("10_000_000"), Ok(10_000_000));
    assert!(Shown::Count.parse("2.5").is_err());
    assert!(Shown::Seconds.parse("soon").is_err());
    assert!(Shown::Seconds.parse("-1").is_err());
    assert!(Shown::Count.parse("").is_err());
    // Every field's key finds it again.
    for field in Field::all() {
        assert_eq!(Field::from_key(&field.key()), Some(field));
    }
}

#[test]
fn ok_with_nothing_changed_changes_nothing() {
    let mut app = sample();
    let entries = app.calc().history_len();
    let prefs = app.user.prefs.clone();
    app.begin_options(None);
    let dialog = app.options.as_ref().unwrap();
    assert_eq!(dialog.page, Page::General);
    assert!(dialog.problems().is_empty());
    assert_eq!(
        dialog.texts[&Field::Workbook(Setting::FormulaTimeLimit)],
        "1"
    );
    app.apply_options().unwrap();
    assert!(app.options.is_none());
    assert_eq!(app.calc().history_len(), entries);
    assert_eq!(app.user.prefs, prefs);
}

#[test]
fn a_workbook_setting_changed_on_ok_is_one_undoable_entry() {
    let mut app = sample();
    let entries = app.calc().history_len().0;
    app.begin_options(Some(Page::Evaluation));
    let dialog = app.options.as_mut().unwrap();
    dialog
        .texts
        .insert(Field::Workbook(Setting::FormulaTimeLimit), "2.5".into());
    dialog
        .texts
        .insert(Field::Workbook(Setting::MaxRange), "1000".into());
    // Nothing takes effect until OK.
    assert_eq!(app.calc().settings().get(Setting::FormulaTimeLimit), 1_000);
    app.apply_options().unwrap();
    assert_eq!(app.calc().history_len().0, entries + 1);
    assert_eq!(app.calc().settings().get(Setting::FormulaTimeLimit), 2_500);
    assert_eq!(app.calc().settings().get(Setting::MaxRange), 1_000);
    app.apply_settings();
    assert_eq!(formula_limit(&app), Some(Duration::from_millis(2_500)));
    // The dialog opens again on the page it was left on.
    app.begin_options(None);
    assert_eq!(app.options.as_ref().unwrap().page, Page::Evaluation);
    app.cancel_options();

    app.undo();
    app.apply_settings();
    assert_eq!(app.calc().settings().get(Setting::FormulaTimeLimit), 1_000);
    assert_eq!(formula_limit(&app), Some(Duration::from_secs(1)));
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

#[test]
fn a_number_out_of_range_is_refused_and_nothing_changes() {
    let mut app = sample();
    let prefs = app.user.prefs.clone();
    app.begin_options(None);
    let dialog = app.options.as_mut().unwrap();
    dialog.texts.insert(Field::Scale, "20".into());
    dialog
        .texts
        .insert(Field::Workbook(Setting::FormulaTimeLimit), "3".into());
    let problems = dialog.problems();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, Field::Scale);
    assert!(problems[0].1.contains("50%"), "{}", problems[0].1);
    // The preview keeps the scale it had.
    assert_eq!(dialog.appearance().scale_percent, 100);
    let error = app.apply_options().unwrap_err();
    assert!(error.contains("Interface scale"), "{error}");
    assert!(app.options.is_some(), "the dialog stays open");
    assert_eq!(app.user.prefs, prefs);
    assert_eq!(app.calc().settings().get(Setting::FormulaTimeLimit), 1_000);

    // A grace period may be 0; a time limit may not.
    let dialog = app.options.as_mut().unwrap();
    dialog.reset(Field::Scale);
    dialog
        .texts
        .insert(Field::Workbook(Setting::CancelGrace), "0".into());
    dialog
        .texts
        .insert(Field::Workbook(Setting::RecalcTimeLimit), "0".into());
    let problems = dialog.problems();
    assert_eq!(
        problems.iter().map(|p| p.0).collect::<Vec<_>>(),
        [Field::Workbook(Setting::RecalcTimeLimit)]
    );
}

#[test]
fn a_ceiling_holds_a_workbook_setting_down() {
    let mut app = sample();
    app.begin_options(Some(Page::Trust));
    let dialog = app.options.as_mut().unwrap();
    dialog
        .texts
        .insert(Field::Ceiling(Setting::FormulaTimeLimit), "2".into());
    dialog
        .texts
        .insert(Field::Workbook(Setting::FormulaTimeLimit), "30".into());
    app.apply_options().unwrap();
    // The workbook keeps what it asks for; the ceiling is what is used.
    assert_eq!(app.calc().settings().get(Setting::FormulaTimeLimit), 30_000);
    assert_eq!(
        app.user.prefs.ceilings.get(Setting::FormulaTimeLimit),
        2_000
    );
    app.apply_settings();
    assert_eq!(formula_limit(&app), Some(Duration::from_secs(2)));
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

#[test]
fn the_preferences_are_written_and_read_by_the_next_session() {
    let folder = Folder::new("prefs");
    let mut app = sample().with_user(folder.user());
    app.begin_options(None);
    let dialog = app.options.as_mut().unwrap();
    dialog.theme = ThemeChoice::Dark;
    dialog.texts.insert(Field::Scale, "150".into());
    dialog.texts.insert(Field::UndoEntries, "50".into());
    dialog.texts.insert(Field::ImportBytes, "2".into());
    // The appearance is previewed while the dialog is open.
    assert_eq!(app.appearance().theme, ThemeChoice::Dark);
    app.apply_options().unwrap();

    let text = std::fs::read_to_string(folder.file("config").join(PREFERENCES_FILE)).unwrap();
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        file["appearance"],
        json!({ "theme": "dark", "scale_percent": 150 })
    );
    assert_eq!(file["files"]["max_import_bytes"], 2 << 20);
    assert_eq!(file["undo"]["max_entries"], 50);

    let user = folder.user();
    assert_eq!(user.prefs.appearance.theme, ThemeChoice::Dark);
    assert_eq!(user.prefs.appearance.scale_percent, 150);
    assert_eq!(user.prefs.max_import_bytes, 2 << 20);
    assert_eq!(user.prefs.history_entries, 50);
}

#[test]
fn cancel_drops_every_change_and_the_preview() {
    let mut app = sample();
    let entries = app.calc().history_len();
    app.begin_options(None);
    let dialog = app.options.as_mut().unwrap();
    dialog.theme = ThemeChoice::Light;
    dialog.texts.insert(Field::Scale, "200".into());
    dialog
        .texts
        .insert(Field::Workbook(Setting::MaxAbandoned), "3".into());
    assert_eq!(app.appearance().scale_percent, 200);
    app.cancel_options();
    assert!(app.options.is_none());
    assert_eq!(app.appearance(), crate::prefs::Appearance::default());
    assert_eq!(app.calc().history_len(), entries);
    assert_eq!(app.calc().settings().get(Setting::MaxAbandoned), 20);
}

#[test]
fn the_import_limit_is_its_own_preference() {
    let folder = Folder::new("import");
    let mut app = sample();
    app.begin_options(Some(Page::Files));
    app.options
        .as_mut()
        .unwrap()
        .texts
        .insert(Field::ImportBytes, "0.0001".into());
    app.apply_options().unwrap();
    assert_eq!(app.user.prefs.max_import_bytes, 105);
    let path = folder.file("big.json");
    std::fs::write(&path, format!("[{}]", "1, ".repeat(100) + "1")).unwrap();
    let error = app.begin_import_path(&path).unwrap_err();
    assert!(error.contains("larger than the limit"), "{error}");
    // A workbook is still read with its own limits.
    assert!(app.user.prefs.limits.entry > 1 << 20);
}

#[test]
fn a_revoked_workbook_is_forgotten_on_ok_and_the_list_names_its_file() {
    let folder = Folder::new("revoke");
    let mut app = sample().with_user(folder.user());
    app.save_as(&folder.file("t.clio")).unwrap();
    let id = app.doc.workbook_id.clone();
    assert!(app.user.trust.is_recorded(&id));
    assert_eq!(
        app.user.trust_files.get(&id).and_then(|p| p.file_name()),
        Some(std::ffi::OsStr::new("t.clio"))
    );
    // The file is kept in trust.json for the next session.
    assert!(folder.user().trust_files.contains_key(&id));

    app.begin_options(Some(Page::Trust));
    app.options.as_mut().unwrap().revoke.insert(id.clone());
    // Not until OK.
    assert!(app.user.trust.is_recorded(&id));
    app.apply_options().unwrap();
    assert!(!app.user.trust.is_recorded(&id));
    assert!(!app.user.trust_files.contains_key(&id));
    let later = folder.user();
    assert!(!later.trust.is_recorded(&id));

    // The next session does not trust it.
    let mut next = sample().with_user(folder.user());
    next.open_path(&folder.file("t.clio")).unwrap();
    assert!(!next.calc().is_trusted());
}

#[test]
fn save_and_restart_needs_a_file_and_saves_it() {
    let folder = Folder::new("restart");
    let mut app = sample();
    let error = app.save_for_restart().unwrap_err();
    assert!(error.contains("Save As"), "{error}");
    app.save_as(&folder.file("r.clio")).unwrap();
    app.session
        .calc_mut()
        .set_value(
            "Prices",
            &"currency".parse().unwrap(),
            cliotree_core::Value::Str("USD".into()),
        )
        .unwrap();
    assert!(app.is_dirty());
    assert_eq!(app.save_for_restart().unwrap(), folder.file("r.clio"));
    assert!(!app.is_dirty());
}

#[test]
fn the_control_channel_sets_options() {
    let mut app = sample();
    control::run(
        &mut app,
        "set_option",
        &json!({ "name": "formula_time_limit_ms", "value": 4 }),
    )
    .unwrap();
    assert_eq!(app.calc().settings().get(Setting::FormulaTimeLimit), 4_000);
    control::run(
        &mut app,
        "set_option",
        &json!({ "name": "theme", "value": "light" }),
    )
    .unwrap();
    assert_eq!(app.user.prefs.appearance.theme, ThemeChoice::Light);
    let error = control::run(
        &mut app,
        "set_option",
        &json!({ "name": "scale_percent", "value": 400 }),
    )
    .unwrap_err();
    assert!(error.contains("300%"), "{error}");
    assert!(app.options.is_none());
    assert!(
        control::run(
            &mut app,
            "set_option",
            &json!({ "name": "nope", "value": 1 })
        )
        .is_err()
    );
    assert!(app.session.wait_idle(Duration::from_secs(60)));
}

#[test]
fn the_about_page_lists_the_libraries_with_their_versions_from_the_lock_file() {
    let libraries = crate::options::LIBRARIES;
    for name in ["egui", "eframe", "RustPython", "serde_json", "quick-xml"] {
        assert!(
            libraries.iter().any(|(n, _, _)| *n == name),
            "{name} should be listed"
        );
    }
    for (name, version, licence) in libraries {
        assert!(
            !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.'),
            "{name} has the version {version:?}"
        );
        assert!(!licence.is_empty(), "{name} has no licence");
    }
    // Cargo.lock has two versions of quick-xml: the one cliotree-core uses is shown.
    let lock = include_str!("../../../Cargo.lock");
    let core = lock
        .split("[[package]]")
        .find(|p| p.contains("name = \"cliotree-core\""))
        .unwrap();
    let (_, version, _) = libraries
        .iter()
        .find(|(n, _, _)| *n == "quick-xml")
        .unwrap();
    assert!(
        core.contains(&format!("\"quick-xml {version}\"")) || core.contains("\"quick-xml\""),
        "quick-xml {version} is not the version cliotree-core uses"
    );
}

/// The Code editor page: its numbers and yes-or-no options are preferences,
/// and *Clear history* clears the Immediate window's history on OK.
#[test]
fn the_code_editor_page_sets_the_editor_and_clears_the_immediate_history() {
    let folder = Folder::new("code");
    let mut app = sample();
    app.user = folder.user();
    app.user.add_immediate("1 + 1").unwrap();
    app.user.add_immediate("Orders").unwrap();
    app.user.add_immediate("1 + 1").unwrap();
    assert_eq!(
        app.user.immediate,
        ["Orders", "1 + 1"],
        "no repeats, newest last"
    );

    app.begin_options(Some(Page::CodeEditor));
    let dialog = app.options.as_mut().unwrap();
    dialog.set(Field::CodeFontSize, 15);
    dialog.set(Field::IndentWidth, 9);
    assert_eq!(
        dialog.problems(),
        [(Field::IndentWidth, "The largest allowed is 8.".to_owned())]
    );
    dialog.set(Field::IndentWidth, 2);
    dialog.set(Field::ImmediateOutput, 2 << 10);
    dialog.wrap = true;
    dialog.line_numbers = false;
    dialog.clear_history = true;
    app.apply_options().unwrap();
    let code = app.user.prefs.code;
    assert_eq!(
        (code.font_size, code.indent, code.max_output),
        (15, 2, 2048)
    );
    assert!(code.wrap && !code.line_numbers && code.highlighting);
    assert!(app.user.immediate.is_empty());

    // Both are on disk for the next session.
    let next = folder.user();
    assert_eq!(next.prefs.code, code);
    assert!(next.immediate.is_empty());

    // Fewer entries kept drops the oldest.
    app.user.add_immediate("a").unwrap();
    app.user.add_immediate("b").unwrap();
    app.user.add_immediate("c").unwrap();
    app.begin_options(Some(Page::CodeEditor));
    app.options
        .as_mut()
        .unwrap()
        .set(Field::ImmediateEntries, 2);
    app.apply_options().unwrap();
    assert_eq!(folder.user().immediate, ["b", "c"]);
}

#[test]
fn the_display_page_sets_the_data_views_and_the_editing_page_the_formula_bar() {
    use crate::prefs::Density;
    let folder = Folder::new("display");
    let mut app = sample();
    app.user = folder.user();
    app.begin_options(Some(Page::Display));
    let dialog = app.options.as_mut().unwrap();
    assert_eq!(dialog.texts[&Field::TreeIndent], "16");
    dialog.set(Field::TreeIndent, 65);
    dialog.set(Field::FormulaBarLines, 0);
    assert_eq!(
        dialog.problems(),
        [
            (
                Field::TreeIndent,
                "The largest allowed is 64 px.".to_owned()
            ),
            (
                Field::FormulaBarLines,
                "The smallest allowed is 1.".to_owned()
            ),
        ]
    );
    dialog.set(Field::TreeIndent, 24);
    dialog.set(Field::FormulaBarLines, 3);
    dialog.set(Field::ColumnWidth, 200);
    dialog.density = Density::Compact;
    dialog.formula_column = false;
    // The hover delay is typed in seconds.
    assert_eq!(dialog.texts[&Field::CommentDelay], "0.5");
    dialog.texts.insert(Field::CommentDelay, "0.25".to_owned());
    dialog.set(Field::CommentWidth, 100);
    assert_eq!(
        dialog.problems(),
        [(
            Field::CommentWidth,
            "The smallest allowed is 120 px.".to_owned()
        )]
    );
    dialog.set(Field::CommentWidth, 480);
    app.apply_options().unwrap();
    let display = app.user.prefs.display;
    assert_eq!((display.indent, display.column_width), (24, 200));
    assert_eq!(display.density, Density::Compact);
    assert!(display.type_column && !display.formula_column);
    assert_eq!(app.user.prefs.formula_bar_lines, 3);
    assert_eq!((display.comment_delay, display.comment_width), (250, 480));
    assert_eq!(display.comment_height, 240);

    // On disk, under the names of the file, for the next session.
    let text = std::fs::read_to_string(folder.file("config").join(PREFERENCES_FILE)).unwrap();
    let written: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(written["display"]["row_density"], "compact");
    assert_eq!(written["display"]["show_formula_column"], false);
    assert_eq!(written["editing"]["formula_bar_lines"], 3);
    assert_eq!(written["display"]["comment_hover_delay_ms"], 250);
    assert_eq!(written["display"]["comment_popup_width_px"], 480);
    let next = folder.user();
    assert_eq!(next.prefs.display, display);
    assert_eq!(next.prefs.formula_bar_lines, 3);

    // The control channel sets them by the same names.
    control::run(
        &mut app,
        "set_option",
        &json!({"name": "display.row_density", "value": "comfortable"}),
    )
    .unwrap();
    control::run(
        &mut app,
        "set_option",
        &json!({"name": "display.tree_indent_px", "value": 8}),
    )
    .unwrap();
    assert_eq!(app.user.prefs.display.density, Density::Comfortable);
    assert_eq!(app.user.prefs.display.indent, 8);
}

#[test]
fn the_title_bar_takes_the_chosen_theme() {
    assert_eq!(ThemeChoice::Dark.window_theme(), egui::SystemTheme::Dark);
    assert_eq!(ThemeChoice::Light.window_theme(), egui::SystemTheme::Light);
    assert_eq!(
        ThemeChoice::System.window_theme(),
        egui::SystemTheme::SystemDefault
    );
}
