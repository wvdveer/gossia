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
