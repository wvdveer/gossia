//! Tests of *Tools > Data sources...* (DESIGN.md, "Data sources"): the
//! definitions of this user on this machine, made, tested, edited and deleted
//! in the window.

use std::time::Duration;

use cliotree_app::ClioApp;
use cliotree_calc::sources::{SourceDef, SourceKind};
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn window() -> Harness<'static, ClioApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(|ui, app: &mut ClioApp| app.show(ui), ClioApp::sample());
    harness.run();
    harness
}

fn click(harness: &mut Harness<'_, ClioApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run();
}

fn has(harness: &Harness<'_, ClioApp>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

fn type_into(harness: &mut Harness<'_, ClioApp>, field: &str, text: &str) {
    harness.get_by_label(field).click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[Key::A]);
    harness.get_by_label(field).type_text(text);
    harness.run();
}

fn open_panel(harness: &mut Harness<'_, ClioApp>) {
    click(harness, "Tools");
    click(harness, "Data sources...");
}

fn names(harness: &Harness<'_, ClioApp>) -> Vec<String> {
    harness
        .state()
        .user
        .sources
        .list()
        .unwrap()
        .into_iter()
        .map(|d| d.name)
        .collect()
}

/// Runs frames until `done`, while a test finishes in the background.
fn wait_for(harness: &mut Harness<'_, ClioApp>, done: impl Fn(&Harness<'_, ClioApp>) -> bool) {
    for _ in 0..200 {
        harness.run();
        if done(harness) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the test did not finish");
}

#[test]
fn a_definition_is_made_in_the_form_and_tested() {
    let mut harness = window();
    open_panel(&mut harness);
    assert!(has(
        &harness,
        "No data sources are defined on this computer."
    ));
    click(&mut harness, "New...");
    type_into(&mut harness, "Name of the data source", "Local");
    click(&mut harness, "Kind of the data source");
    click(&mut harness, "SQLite");
    let dir = std::env::temp_dir().join(format!("cliotree-sources-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("local.db");
    std::fs::write(&file, b"").unwrap();
    type_into(
        &mut harness,
        "File of the data source",
        &file.display().to_string(),
    );
    click(&mut harness, "Test");
    wait_for(&mut harness, |h| {
        h.query_by_label_contains("Passed").is_some()
            || h.query_by_label_contains("Failed").is_some()
    });
    assert!(harness.query_by_label_contains("Passed").is_some());
    click(&mut harness, "Save");
    assert_eq!(names(&harness), ["Local"]);
    // Back on the list, with the definition chosen.
    assert!(has(&harness, "Local"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stored_password_is_never_shown_and_is_kept_unless_changed() {
    let mut harness = window();
    let def = SourceDef {
        host: Some("db.example.com".into()),
        database: Some("sales".into()),
        user: Some("ro".into()),
        password: Some("hunter2".into()),
        ..SourceDef::new("Sales", SourceKind::Postgres)
    };
    harness.state().user.sources.save(def, None, false).unwrap();
    open_panel(&mut harness);
    click(&mut harness, "Sales");
    click(&mut harness, "Edit...");
    assert!(has(&harness, "Stored"));
    assert!(!has(&harness, "Password of the data source"));
    type_into(&mut harness, "Database of the data source", "sales2");
    click(&mut harness, "Save");
    let saved = harness.state().user.sources.get("Sales").unwrap().unwrap();
    assert_eq!(saved.database.as_deref(), Some("sales2"));
    assert_eq!(saved.password.as_deref(), Some("hunter2"));

    // Changing it shows a password box, whose text is dots.
    click(&mut harness, "Edit...");
    click(&mut harness, "Change");
    type_into(&mut harness, "Password of the data source", "s3cret");
    assert!(
        !harness
            .get_by_label("Password of the data source")
            .value()
            .unwrap_or_default()
            .contains("s3cret")
    );
    click(&mut harness, "Save");
    let saved = harness.state().user.sources.get("Sales").unwrap().unwrap();
    assert_eq!(saved.password.as_deref(), Some("s3cret"));
}

#[test]
fn a_definition_is_deleted_after_asking() {
    let mut harness = window();
    let mut def = SourceDef::new("Api", SourceKind::Http);
    def.url = Some("https://api.example.com".into());
    harness.state().user.sources.save(def, None, false).unwrap();
    open_panel(&mut harness);
    click(&mut harness, "Api");
    click(&mut harness, "Delete");
    click(&mut harness, "Keep");
    assert_eq!(names(&harness), ["Api"]);
    click(&mut harness, "Delete");
    click(&mut harness, "Delete it");
    assert!(names(&harness).is_empty());
    click(&mut harness, "Close");
    assert!(harness.state().sources_panel.is_none());
}

/// A server that answers each request with how many it has had.
fn counting_server() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { break };
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
            let text = format!("call {}", n + 1);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    url
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

#[test]
fn f9_runs_the_formulas_again_and_they_read_the_source_afresh() {
    let mut harness = window();
    let mut def = SourceDef::new("Api", SourceKind::Http);
    def.url = Some(counting_server());
    harness.state().user.sources.save(def, None, false).unwrap();
    let path: cliotree_core::Path = "customer.name".parse().unwrap();
    harness
        .state_mut()
        .session
        .calc_mut()
        .set_formula("Orders", &path, "source('Api').get('n').text")
        .unwrap();
    settle(&mut harness);
    let name = |h: &Harness<'_, ClioApp>| {
        h.state()
            .calc()
            .data("Orders")
            .unwrap()
            .get(&path)
            .map(|n| n.value().clone())
    };
    assert_eq!(
        name(&harness),
        Some(cliotree_core::Value::Str("call 1".into()))
    );
    harness.key_press(Key::F9);
    harness.run();
    settle(&mut harness);
    assert_eq!(
        name(&harness),
        Some(cliotree_core::Value::Str("call 2".into()))
    );
    // And from the menu.
    click(&mut harness, "Tools");
    // The item's label has its shortcut with it.
    harness.get_by_label_contains("Recalculate").click();
    harness.run();
    settle(&mut harness);
    assert_eq!(
        name(&harness),
        Some(cliotree_core::Value::Str("call 3".into()))
    );
}
