//! The application with a host (DESIGN.md, Web port, 0.3.0), run natively:
//! a fake host that records what it is asked, as the test page's does. Each
//! test runs on a thread of its own, so the host set here is that test's.

use std::cell::RefCell;
use std::rc::Rc;

use crate::GossiaApp;
use crate::host::{self, Host, Picked, purpose};

#[derive(Default)]
struct Calls {
    picks: Vec<(String, Vec<String>)>,
    files: Vec<(String, Vec<u8>)>,
    workbooks: Vec<(String, Vec<u8>)>,
    storage: std::collections::BTreeMap<String, String>,
    secrets: std::collections::BTreeMap<String, String>,
    fetches: Vec<gossia_calc::http::FetchRequest>,
    /// Whether the host edits modules in its own editor.
    edits_modules: bool,
    opened_modules: Vec<(String, usize, usize)>,
    modules_told: Vec<Vec<(String, String)>>,
    module_answers: Vec<(u32, Option<String>)>,
    /// Whether the host ships fonts for Japanese and Chinese, and the tags
    /// of those asked for.
    ships_fonts: bool,
    fonts_asked: Vec<String>,
}

#[derive(Default)]
struct FakeHost(RefCell<Calls>);

impl Host for FakeHost {
    fn save_workbook(&self, name: &str, bytes: Vec<u8>) {
        self.0.borrow_mut().workbooks.push((name.to_owned(), bytes));
    }

    fn set_dirty(&self, _dirty: bool) {}

    fn read_storage(&self, name: &str) -> Option<String> {
        self.0.borrow().storage.get(name).cloned()
    }

    fn write_storage(&self, name: &str, text: &str) -> Result<(), String> {
        self.0
            .borrow_mut()
            .storage
            .insert(name.to_owned(), text.to_owned());
        Ok(())
    }

    fn read_secret(&self, name: &str) -> Option<String> {
        self.0.borrow().secrets.get(name).cloned()
    }

    fn write_secret(&self, name: &str, text: Option<&str>) {
        let mut calls = self.0.borrow_mut();
        match text {
            Some(text) => calls.secrets.insert(name.to_owned(), text.to_owned()),
            None => calls.secrets.remove(name),
        };
    }

    fn pick_file(&self, purpose: &str, extensions: &[&str]) {
        let extensions = extensions.iter().map(|e| (*e).to_owned()).collect();
        self.0
            .borrow_mut()
            .picks
            .push((purpose.to_owned(), extensions));
    }

    fn save_file(&self, name: &str, bytes: Vec<u8>) {
        self.0.borrow_mut().files.push((name.to_owned(), bytes));
    }

    fn open_outside(&self, _target: &str) {}

    fn dark(&self) -> Option<bool> {
        None
    }

    fn fetch(&self, request: &gossia_calc::http::FetchRequest) {
        self.0.borrow_mut().fetches.push(request.clone());
    }

    fn edits_modules(&self) -> bool {
        self.0.borrow().edits_modules
    }

    fn open_module(&self, name: &str, line: usize, column: usize) {
        self.0
            .borrow_mut()
            .opened_modules
            .push((name.to_owned(), line, column));
    }

    fn modules_changed(&self, modules: &[(String, String)]) {
        self.0.borrow_mut().modules_told.push(modules.to_vec());
    }

    fn load_font(&self, tag: &str) -> bool {
        let mut calls = self.0.borrow_mut();
        calls.fonts_asked.push(tag.to_owned());
        calls.ships_fonts
    }

    fn module_answer(&self, id: u32, error: Option<&str>) {
        self.0
            .borrow_mut()
            .module_answers
            .push((id, error.map(str::to_owned)));
    }
}

fn hosted() -> (Rc<FakeHost>, GossiaApp) {
    let fake = Rc::new(FakeHost::default());
    host::set(fake.clone());
    (fake, GossiaApp::sample())
}

fn pick(purpose: &str, name: &str, text: &str) {
    host::picked(Picked {
        purpose: purpose.to_owned(),
        name: name.to_owned(),
        bytes: text.as_bytes().to_vec(),
    });
}

#[test]
fn import_asks_the_host_for_any_file_and_the_file_it_gives_is_planned() {
    let (fake, mut app) = hosted();
    app.begin_import();
    assert_eq!(fake.0.borrow().picks, [("import".to_owned(), Vec::new())]);
    assert!(app.files.browsing.is_none(), "no file browser of our own");
    pick(purpose::IMPORT, "people.json", r#"[{"name": "Ann"}]"#);
    app.receive_picked();
    assert_eq!(app.notice, None);
    assert_eq!(app.imports.len(), 1);
}

#[test]
fn a_dropped_file_is_imported_and_a_dropped_workbook_is_not_opened() {
    let (_fake, mut app) = hosted();
    pick(purpose::DROP, "rates.csv", "code,rate\nA,1\n");
    pick(purpose::DROP, "Book.gossia", "PK");
    app.receive_picked();
    assert_eq!(app.imports.len(), 1);
    let notice = app.notice.clone().unwrap_or_default();
    assert!(notice.contains("Book.gossia is a workbook"), "{notice}");
}

#[test]
fn export_gives_the_host_the_tab_and_its_schema_as_files() {
    let (fake, mut app) = hosted();
    app.begin_export("Prices");
    if let Some(plan) = &mut app.export {
        plan.with_schema = true;
    }
    app.choose_export_file();
    assert!(app.export.is_none(), "the dialog closes");
    let calls = fake.0.borrow();
    let names: Vec<&str> = calls.files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Prices.json", "Prices.schema.json"]);
    let data = String::from_utf8(calls.files[0].1.clone()).unwrap();
    assert!(data.contains("EUR"), "{data}");
}

/// A host has no printing: *Print...* says to export and print from the
/// program that opens the file.
#[test]
fn print_is_refused_in_a_host() {
    let (_fake, mut app) = hosted();
    app.begin_print("Prices");
    assert!(app.pdf_export.is_none());
    let notice = app.notice.clone().unwrap_or_default();
    assert!(
        notice.contains("Printing is not available here"),
        "{notice}"
    );
}

#[test]
fn modules_go_out_and_come_in_through_the_host() {
    let (fake, mut app) = hosted();
    app.session
        .calc_mut()
        .add_module("Tax", "def vat(x):\n    return x * 0.21\n")
        .unwrap();
    app.begin_export_module("Tax");
    assert_eq!(fake.0.borrow().files[0].0, "Tax.py");

    app.begin_import_module();
    assert_eq!(
        fake.0.borrow().picks,
        [("import-module".to_owned(), vec!["py".to_owned()])]
    );
    pick(
        purpose::IMPORT_MODULE,
        "Tax.py",
        "def gross(x):\n    return x\n",
    );
    app.receive_picked();
    assert_eq!(app.code.notice, None);
    assert!(
        app.calc().module_source("Tax2").is_some(),
        "named as on the desktop"
    );
}

#[test]
fn save_gives_the_host_the_workbook_file() {
    let (fake, mut app) = hosted();
    app.begin_save();
    assert_eq!(app.notice, None);
    let calls = fake.0.borrow();
    assert_eq!(calls.workbooks.len(), 1);
    assert_eq!(calls.workbooks[0].0, "Untitled.gossia");
    assert!(calls.workbooks[0].1.starts_with(b"PK"), "a zip");
    assert!(!app.is_dirty());
    drop(calls);
    // The next save keeps that name.
    app.begin_save();
    assert_eq!(fake.0.borrow().workbooks[1].0, "Untitled.gossia");
}

/// A backup (VS Code's hot exit) is the workbook's file, and the workbook
/// still has its unsaved changes afterwards.
#[test]
fn a_backup_gives_the_bytes_without_counting_the_workbook_as_saved() {
    let (fake, mut app) = hosted();
    crate::control::run(&mut app, "tab", &serde_json::json!({ "name": "Orders" })).unwrap();
    crate::control::run(
        &mut app,
        "select",
        &serde_json::json!({ "path": "tax_rate" }),
    )
    .unwrap();
    crate::control::run(&mut app, "enter", &serde_json::json!({ "text": "0.5" })).unwrap();
    assert!(app.is_dirty());
    let bytes = app.backup_bytes().unwrap();
    assert!(bytes.starts_with(b"PK"), "a zip");
    assert!(app.is_dirty(), "still unsaved");
    assert!(
        fake.0.borrow().workbooks.is_empty(),
        "nothing given to the host"
    );
}

#[test]
fn data_sources_are_kept_in_the_host_with_their_secrets_apart() {
    let (fake, mut app) = hosted();
    let mut settings = serde_json::Map::new();
    settings.insert("url".to_owned(), "https://example.com/v1".into());
    settings.insert("auth".to_owned(), "bearer".into());
    settings.insert("token".to_owned(), "s3cret".into());
    let def = gossia_calc::sources::SourceDef::from_settings("Api", "http", &settings).unwrap();
    app.user.sources.save(def, None, false).unwrap();
    app.sync_sources();
    {
        let calls = fake.0.borrow();
        let stored = &calls.storage["data-sources.json"];
        assert!(
            stored.contains("example.com") && !stored.contains("s3cret"),
            "{stored}"
        );
        assert!(calls.secrets["source:Api"].contains("s3cret"));
    }

    // The next start reads them back, secret included.
    let store = gossia_calc::sources::SourceStore::in_memory();
    let mut warnings = Vec::new();
    host::load_sources(&*fake, &store, &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    let back = store.get("Api").unwrap().unwrap();
    assert_eq!(back.token.as_deref(), Some("s3cret"));

    // Removed: its secret goes too.
    app.user.sources.remove("Api").unwrap();
    app.sync_sources();
    assert!(fake.0.borrow().secrets.is_empty());
}

#[test]
fn the_test_of_an_http_source_is_made_through_the_host() {
    let (fake, _app) = hosted();
    let mut settings = serde_json::Map::new();
    settings.insert("url".to_owned(), "https://example.com/v1".into());
    settings.insert("auth".to_owned(), "bearer".into());
    settings.insert("token".to_owned(), "s3cret".into());
    settings.insert("test_path".to_owned(), "ping".into());
    let def = gossia_calc::sources::SourceDef::from_settings("Api", "http", &settings).unwrap();
    let running = crate::sources_ui::RunningTest::start(def, std::time::Duration::from_secs(5));
    assert!(running.poll().is_none(), "it waits for the host");
    let request = fake.0.borrow().fetches[0].clone();
    assert_eq!(request.method, "GET");
    assert_eq!(request.url, "https://example.com/v1/ping");
    assert!(
        request
            .headers
            .contains(&("Authorization".to_owned(), "Bearer s3cret".to_owned())),
        "{:?}",
        request.headers
    );
    let response = gossia_calc::http::HttpResponse {
        status: 200,
        reason: "OK".to_owned(),
        headers: Vec::new(),
        body: b"pong".to_vec(),
    };
    host::fetched(request.id, Ok(response));
    let outcome = running.poll().expect("the host answered");
    assert!(outcome.ok, "{outcome:?}");
    assert!(
        outcome.message.starts_with("answered 200 OK in "),
        "{}",
        outcome.message
    );
}

/// A host whose editor has the modules (VS Code): told them, and again only
/// when they change; what is saved there is made in the workbook, one entry
/// each, and answered.
#[test]
fn modules_are_told_to_a_host_that_edits_them_and_its_edits_made() {
    use crate::host::ModuleEdit;
    use crate::host_code::module_edited;
    let (fake, mut app) = hosted();
    fake.0.borrow_mut().edits_modules = true;
    app.session
        .calc_mut()
        .add_module(
            "Tax",
            "def vat(x):
    return x * 0.21
",
        )
        .unwrap();
    app.sync_host_modules();
    app.sync_host_modules();
    {
        let calls = fake.0.borrow();
        assert_eq!(calls.modules_told.len(), 1, "told once");
        assert_eq!(calls.modules_told[0][0].0, "Tax");
    }

    module_edited(ModuleEdit::Write {
        id: 1,
        name: "Tax".to_owned(),
        text: "def vat(x):
    return x * 0.2
"
        .to_owned(),
    });
    module_edited(ModuleEdit::Write {
        id: 2,
        name: "Util".to_owned(),
        text: "def one():
    return 1
"
        .to_owned(),
    });
    module_edited(ModuleEdit::Rename {
        id: 3,
        from: "Util".to_owned(),
        to: "Helpers".to_owned(),
    });
    module_edited(ModuleEdit::Write {
        id: 4,
        name: "not a name".to_owned(),
        text: String::new(),
    });
    app.sync_host_modules();
    assert!(app.calc().module_source("Tax").unwrap().contains(
        "0.2
"
    ));
    assert!(app.calc().module_source("Helpers").is_some());
    assert_eq!(
        app.calc().undo_label(),
        Some("Rename module Util to Helpers")
    );
    {
        let calls = fake.0.borrow();
        let answers: Vec<(u32, bool)> = calls
            .module_answers
            .iter()
            .map(|(id, e)| (*id, e.is_none()))
            .collect();
        assert_eq!(answers, [(1, true), (2, true), (3, true), (4, false)]);
        let told: Vec<&str> = calls.modules_told[1]
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(told, ["Tax", "Helpers"], "told again after the edits");
    }

    // An undo in the workbook is told too, so the host's file follows it.
    app.undo();
    app.sync_host_modules();
    let calls = fake.0.borrow();
    let told: Vec<&str> = calls.modules_told[2]
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(told, ["Tax", "Util"]);

    drop(calls);
    module_edited(ModuleEdit::Remove {
        id: 5,
        name: "Util".to_owned(),
    });
    app.sync_host_modules();
    assert!(app.calc().module_source("Util").is_none());
}

/// In such a host the code editor opens no module by itself, and opening
/// one (from the explorer, or a problem *Compile* lists) opens it there.
#[test]
fn a_module_opens_in_the_editor_of_a_host_that_edits_modules() {
    let (fake, mut app) = hosted();
    fake.0.borrow_mut().edits_modules = true;
    app.session
        .calc_mut()
        .add_module(
            "Tax",
            "def vat(x):
    return x
",
        )
        .unwrap();
    app.open_code_editor();
    assert!(app.code.open);
    assert!(app.code.tabs.is_empty(), "nothing opened by itself");
    app.open_module("Tax");
    assert!(app.code.tabs.is_empty(), "not in a tab here");
    assert_eq!(fake.0.borrow().opened_modules, [("Tax".to_owned(), 1, 1)]);
}

/// The commands a host lists run as their menu items do, and are refused
/// with the reason where those are greyed.
#[test]
fn commands_run_by_name_as_their_menu_items() {
    let (_fake, mut app) = hosted();
    app.run_command("codeEditor").unwrap();
    assert!(app.code.open);
    let shown = app.show_issues;
    app.run_command("issues").unwrap();
    assert_eq!(app.show_issues, !shown);
    assert!(app.run_command("exportPdf").unwrap_err().contains("report"));
    assert!(
        app.run_command("undo")
            .unwrap_err()
            .contains("nothing to undo")
    );
    assert!(app.run_command("frobnicate").is_err());
    for (name, title) in crate::commands::COMMANDS {
        assert!(!title.is_empty(), "{name}");
    }
}

/// The VS Code extension lists Gossia's commands, with their titles (in
/// `package.nls.json`, for VS Code to translate), and *Open Module...* of
/// its own.
#[test]
fn the_extension_lists_every_command() {
    let text = include_str!("../../../vscode/package.json");
    let package: serde_json::Value = serde_json::from_str(text).unwrap();
    let english: serde_json::Value =
        serde_json::from_str(include_str!("../../../vscode/package.nls.json")).unwrap();
    let listed: Vec<(String, String)> = package["contributes"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let title = c["title"].as_str().unwrap();
            let key = title.trim_matches('%');
            assert_eq!(
                title,
                format!("%{key}%"),
                "a title is a key of package.nls.json"
            );
            (
                c["command"].as_str().unwrap().to_owned(),
                english[key].as_str().unwrap_or(title).to_owned(),
            )
        })
        .collect();
    let mut expected: Vec<(String, String)> = crate::commands::COMMANDS
        .iter()
        .map(|(name, title)| (format!("gossia.{name}"), (*title).to_owned()))
        .collect();
    expected.push(("gossia.openModule".to_owned(), "Open Module...".to_owned()));
    assert_eq!(listed, expected);
}

#[test]
fn the_host_s_font_for_japanese_is_loaded_when_first_wanted() {
    use gossia_i18n::Language;
    let (fake, mut app) = hosted();
    fake.0.borrow_mut().ships_fonts = true;
    gossia_i18n::set_thread_language(Some(Language::Japanese));
    let ctx = egui::Context::default();
    app.apply_cjk_font(&ctx);
    app.apply_cjk_font(&ctx);
    assert_eq!(fake.0.borrow().fonts_asked, ["ja"], "asked once");
    assert_eq!(app.cjk_font_for, None, "waiting for it");
    host::font_loaded("ja", Ok(gossia_layout::fonts::INTER.to_vec()));
    app.apply_cjk_font(&ctx);
    assert_eq!(app.cjk_font_for, Some(Some(Language::Japanese)));
    assert_eq!(app.notice, None);
    // One that cannot be read says so, and the window goes on without it.
    gossia_i18n::set_language(Language::Chinese);
    app.apply_cjk_font(&ctx);
    assert_eq!(fake.0.borrow().fonts_asked, ["ja", "zh-Hans"]);
    host::font_loaded("zh-Hans", Err("404".to_owned()));
    app.apply_cjk_font(&ctx);
    assert_eq!(app.cjk_font_for, Some(Some(Language::Chinese)));
    let notice = app.notice.clone().unwrap_or_default();
    assert!(notice.contains("（404）"), "in Chinese: {notice}");
    // Japanese again: what came is kept, not asked for again.
    gossia_i18n::set_language(Language::Japanese);
    app.apply_cjk_font(&ctx);
    assert_eq!(app.cjk_font_for, Some(Some(Language::Japanese)));
    assert_eq!(fake.0.borrow().fonts_asked.len(), 2);
    gossia_i18n::set_thread_language(None);
}

#[test]
fn a_host_without_fonts_leaves_the_system_s() {
    use gossia_i18n::Language;
    let (fake, mut app) = hosted();
    gossia_i18n::set_thread_language(Some(Language::Japanese));
    let ctx = egui::Context::default();
    app.apply_cjk_font(&ctx);
    assert_eq!(fake.0.borrow().fonts_asked, ["ja"]);
    // Not waiting for the host: the system's font, or the notice.
    assert_eq!(app.cjk_font_for, Some(Some(Language::Japanese)));
    gossia_i18n::set_thread_language(None);
}

/// The `{0}` places of a VS Code text.
fn vscode_places(text: &str) -> Vec<&str> {
    let mut places: Vec<&str> = text
        .match_indices('{')
        .filter_map(|(at, _)| Some(&text[at..at + text[at..].find('}')? + 1]))
        .collect();
    places.sort_unstable();
    places
}

fn json_object(text: &str) -> serde_json::Map<String, serde_json::Value> {
    match serde_json::from_str(text).unwrap() {
        serde_json::Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

/// Every text of the extension is in the five other languages: the
/// manifest's (`package.nls.<locale>.json`, with the keys package.json
/// names) and those of `extension.js` (`l10n/bundle.l10n.<locale>.json`,
/// with every `vscode.l10n.t('...')`), with the same places.
#[test]
fn the_extension_s_texts_are_in_every_language() {
    let manifest = include_str!("../../../vscode/package.json");
    let english = json_object(include_str!("../../../vscode/package.nls.json"));
    let used: std::collections::BTreeSet<&str> = manifest
        .split('"')
        .filter(|s| s.len() > 2 && s.starts_with('%') && s.ends_with('%'))
        .map(|s| s.trim_matches('%'))
        .collect();
    let keys: std::collections::BTreeSet<&str> = english.keys().map(String::as_str).collect();
    assert_eq!(
        used, keys,
        "package.json names the keys of package.nls.json"
    );
    let script = include_str!("../../../vscode/extension.js");
    // In single quotes, or in double quotes when the text has an apostrophe.
    let quoted = |quote: char| {
        let call = if quote == '"' {
            "vscode.l10n.t(\""
        } else {
            "vscode.l10n.t('"
        };
        script
            .split(call)
            .skip(1)
            .map(move |rest| &rest[..rest.find(quote).unwrap()])
    };
    let texts: std::collections::BTreeSet<&str> = quoted('\'').chain(quoted('"')).collect();
    assert!(texts.contains("Open a Gossia workbook first."), "{texts:?}");
    let languages = [
        (
            "fr",
            include_str!("../../../vscode/package.nls.fr.json"),
            include_str!("../../../vscode/l10n/bundle.l10n.fr.json"),
        ),
        (
            "es",
            include_str!("../../../vscode/package.nls.es.json"),
            include_str!("../../../vscode/l10n/bundle.l10n.es.json"),
        ),
        (
            "pt-br",
            include_str!("../../../vscode/package.nls.pt-br.json"),
            include_str!("../../../vscode/l10n/bundle.l10n.pt-br.json"),
        ),
        (
            "ja",
            include_str!("../../../vscode/package.nls.ja.json"),
            include_str!("../../../vscode/l10n/bundle.l10n.ja.json"),
        ),
        (
            "zh-cn",
            include_str!("../../../vscode/package.nls.zh-cn.json"),
            include_str!("../../../vscode/l10n/bundle.l10n.zh-cn.json"),
        ),
    ];
    for (locale, nls, bundle) in languages {
        let nls = json_object(nls);
        let nls_keys: std::collections::BTreeSet<&str> = nls.keys().map(String::as_str).collect();
        assert_eq!(nls_keys, keys, "package.nls.{locale}.json");
        let bundle = json_object(bundle);
        let bundle_keys: std::collections::BTreeSet<&str> =
            bundle.keys().map(String::as_str).collect();
        assert_eq!(bundle_keys, texts, "bundle.l10n.{locale}.json");
        for (key, text) in nls.iter().chain(bundle.iter()) {
            let text = text.as_str().unwrap();
            assert!(!text.is_empty(), "{locale}: {key}");
            assert_eq!(vscode_places(key), vscode_places(text), "{locale}: {key}");
        }
    }
}
