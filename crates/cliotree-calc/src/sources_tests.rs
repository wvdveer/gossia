use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cliotree-sources-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn postgres(name: &str) -> SourceDef {
    SourceDef {
        host: Some("db.example.com".into()),
        database: Some("sales".into()),
        user: Some("reporting_ro".into()),
        password: Some("hunter2".into()),
        ..SourceDef::new(name, SourceKind::Postgres)
    }
}

#[test]
fn a_definition_says_what_it_lacks_and_where_it_goes() {
    let empty = SourceDef::new("Sales", SourceKind::Postgres);
    assert_eq!(empty.problems().len(), 3);
    let def = postgres("Sales");
    assert!(def.problems().is_empty());
    assert_eq!(
        def.target(),
        "postgres://db.example.com:5432/sales as reporting_ro"
    );
    assert!(def.has_secret());
    assert!(!def.without_secrets().has_secret());

    let mut http = SourceDef::new("Api", SourceKind::Http);
    http.url = Some("ftp://x".into());
    assert!(http.problems()[0].contains("http://"));
    http.url = Some("https://api.example.com/v1".into());
    http.auth = HttpAuth::KeyHeader;
    assert!(http.problems()[0].contains("header"));
    http.key_name = Some("X-Key".into());
    assert!(http.problems().is_empty());

    let mut lite = SourceDef::new("Local", SourceKind::Sqlite);
    lite.path = Some("x.db".into());
    lite.pool = Some(2);
    assert_eq!(lite.problems(), ["a SQLite source cannot pool connections"]);

    assert!(name_problem("9lives").is_some());
    assert!(name_problem("has space").is_some());
    assert!(name_problem("Sales_2").is_none());
}

#[test]
fn urls_give_their_host_and_port() {
    assert_eq!(
        http_host_port("https://api.example.com/v1?x=1"),
        Some(("api.example.com".into(), 443))
    );
    assert_eq!(
        http_host_port("http://user:pw@localhost:8080"),
        Some(("localhost".into(), 8080))
    );
    assert_eq!(
        http_host_port("http://[::1]:9000/"),
        Some(("::1".into(), 9000))
    );
    assert_eq!(http_host_port("http://"), None);
    assert_eq!(http_host_port("example.com"), None);
}

#[test]
fn definitions_are_kept_in_a_file_with_what_a_newer_version_wrote() {
    let dir = temp_dir("file");
    let file = dir.join(SOURCES_FILE);
    std::fs::write(
        &file,
        r#"{"version": 1, "future": true, "sources": [
            {"name": "Old", "kind": "sqlite", "path": "old.db", "colour": "red"}
        ]}"#,
    )
    .unwrap();
    let store = SourceStore::at(&file);
    store.save(postgres("Sales"), None, false).unwrap();
    // Another store on the same file, as another window would be, sees it.
    let other = SourceStore::at(&file);
    let names: Vec<String> = other.list().unwrap().into_iter().map(|d| d.name).collect();
    assert_eq!(names, ["Old", "Sales"]);
    assert_eq!(
        other.get("Sales").unwrap().unwrap().password.as_deref(),
        Some("hunter2")
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"future\": true"));
    assert!(text.contains("\"colour\": \"red\""));

    // A name that is taken is refused unless replacing; editing renames.
    assert!(store.save(postgres("Sales"), None, false).is_err());
    store.save(postgres("Sales"), None, true).unwrap();
    store
        .save(postgres("Reports"), Some("Sales"), false)
        .unwrap();
    assert!(store.get("Sales").unwrap().is_none());
    store.remove("Old").unwrap();
    let names: Vec<String> = store.list().unwrap().into_iter().map(|d| d.name).collect();
    assert_eq!(names, ["Reports"]);

    // A file that cannot be read is not overwritten.
    std::fs::write(&file, "{ not json").unwrap();
    assert!(store.list().is_err());
    assert!(store.save(postgres("New"), None, false).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn python_settings_become_a_definition() {
    let settings: Map<String, Json> = serde_json::from_str(
        r#"{"url": "https://api.example.com", "auth": "bearer", "token": "t0k",
            "headers": {"Accept": "application/json"}, "writes": true, "pool": null}"#,
    )
    .unwrap();
    let def = SourceDef::from_settings("Api", "http", &settings).unwrap();
    assert_eq!(def.auth, HttpAuth::Bearer);
    assert_eq!(
        def.headers,
        [("Accept".to_owned(), "application/json".to_owned())]
    );
    assert!(def.writes);
    assert!(SourceDef::from_settings("Api", "ftp", &settings).is_err());
    let mut wrong = settings.clone();
    wrong.insert("passwrod".into(), Json::from("x"));
    let why = SourceDef::from_settings("Api", "http", &wrong).unwrap_err();
    assert!(why.contains("passwrod"), "{why}");
    let why = SourceDef::from_settings("Api", "http", &Map::new()).unwrap_err();
    assert!(why.contains("base URL"), "{why}");
}

#[test]
fn the_test_opens_a_sqlite_file_and_signs_in_to_a_server() {
    let dir = temp_dir("test");
    let db = dir.join("x.db");
    std::fs::write(&db, b"").unwrap();
    let mut lite = SourceDef::new("Local", SourceKind::Sqlite);
    lite.path = Some(db.display().to_string());
    assert!(test(&lite, Duration::from_secs(1)).ok);
    lite.path = Some(dir.join("missing.db").display().to_string());
    assert!(!test(&lite, Duration::from_secs(1)).ok);

    // A server must sign in, not only answer: a listener that says nothing
    // fails within the timeout, and so does a closed port.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut pg = postgres("Sales");
    pg.host = Some("127.0.0.1".into());
    pg.port = Some(port);
    pg.tls = TlsMode::Off;
    let started = Instant::now();
    let outcome = test(&pg, Duration::from_millis(500));
    assert!(!outcome.ok, "{}", outcome.message);
    assert!(started.elapsed() < Duration::from_secs(10));
    drop(listener);
    let outcome = test(&pg, Duration::from_secs(5));
    assert!(
        !outcome.ok && outcome.message.contains(&format!("127.0.0.1:{port}")),
        "{}",
        outcome.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_odbc_definition_keeps_its_password_out_of_the_connection_string() {
    let mut def = SourceDef::new("Ledger", SourceKind::Odbc);
    assert_eq!(
        def.problems(),
        ["an ODBC source needs a DSN or a connection string"]
    );
    def.connection = Some("Ledger".into());
    def.user = Some("reader".into());
    assert!(def.problems().is_empty());
    assert_eq!(def.target(), "odbc:Ledger as reader");
    for text in [
        "Driver={SQL Server};Server=db;PWD=secret",
        "Driver={SQL Server};Server=db; password = secret",
    ] {
        def.connection = Some(text.into());
        assert!(
            def.problems()[0].contains("Password field"),
            "{:?}",
            def.problems()
        );
    }
    // From Python, by the setting's name.
    let settings = serde_json::json!({"connection": "Ledger", "user": "reader"});
    let made = SourceDef::from_settings("Ledger", "odbc", settings.as_object().unwrap()).unwrap();
    assert_eq!(made.connection.as_deref(), Some("Ledger"));
    // Offered by the form only where the driver manager is.
    assert_eq!(
        SourceKind::available().contains(&SourceKind::Odbc),
        cfg!(windows)
    );
}
