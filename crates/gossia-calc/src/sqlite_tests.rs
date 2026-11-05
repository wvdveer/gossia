use std::path::PathBuf;

use super::*;

/// A fresh folder with `shop.db` in it: a table of orders with a date, a
/// time, a real, text, NULL and bytes.
pub(crate) fn shop(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("gossia-sqlite-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("shop.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT, total REAL, \
             placed DATE, shipped DATETIME, note TEXT, stamp BLOB);
         INSERT INTO orders VALUES (1, 'Ada', 12.5, '2026-10-01', '2026-10-02 09:30:00', NULL, x'cafe');
         INSERT INTO orders VALUES (2, 'Bob', 7.0, '2026-10-03', NULL, 'gift', NULL);
         INSERT INTO orders VALUES (3, 'Ada', 3.25, 'soon', NULL, NULL, NULL);",
    )
    .unwrap();
    (dir, db)
}

/// A SQLite definition of the file.
pub(crate) fn def(db: &std::path::Path, writes: bool) -> SourceDef {
    SourceDef {
        path: Some(db.display().to_string()),
        writes,
        ..SourceDef::new("Shop", SourceKind::Sqlite)
    }
}

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(5),
        max_bytes: 1 << 20,
    }
}

#[test]
fn a_query_gives_columns_types_and_rows_with_parameters() {
    let (dir, db) = shop("query");
    let shop = def(&db, false);
    let found = query(
        &shop,
        "SELECT id, customer, total, placed, note, stamp FROM orders WHERE customer = ? ORDER BY id",
        &json!(["Ada"]),
        limits(),
        false,
    )
    .unwrap();
    assert_eq!(
        found["columns"],
        json!(["id", "customer", "total", "placed", "note", "stamp"])
    );
    assert_eq!(found["types"][3], json!("DATE"));
    assert_eq!(
        found["rows"],
        json!([
            [1, "Ada", 12.5, "2026-10-01", null, {"$bytes": "cafe"}],
            [3, "Ada", 3.25, "soon", null, null]
        ])
    );
    // Named parameters, with or without the colon, and bytes as a parameter.
    let found = query(
        &shop,
        "SELECT count(*) AS n FROM orders WHERE total > :least AND stamp IS NOT :stamp",
        &json!({"least": 5, ":stamp": {"$bytes": "beef"}}),
        limits(),
        false,
    )
    .unwrap();
    assert_eq!(found["rows"], json!([[2]]));
    // The wrong number of values, an unknown name, two statements, and SQL
    // that does not parse say so.
    let why = query(&shop, "SELECT ? + ?", &json!([1]), limits(), false).unwrap_err();
    assert!(why.contains("2 parameters, and 1 value was given"), "{why}");
    let why = query(&shop, "SELECT :a", &json!({"b": 1}), limits(), false).unwrap_err();
    assert!(why.contains("no parameter :b"), "{why}");
    let why = query(&shop, "SELECT 1; SELECT 2", &Json::Null, limits(), false).unwrap_err();
    assert!(why.contains("one statement at a time"), "{why}");
    let why = query(&shop, "SELEC 1", &Json::Null, limits(), false).unwrap_err();
    assert!(
        why.starts_with("Shop: ") && why.contains("syntax error"),
        "{why}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_only_file_refuses_a_change_and_query_refuses_one_too() {
    let (dir, db) = shop("readonly");
    // query() is only for reading, whatever the connection.
    let why = query(
        &def(&db, true),
        "DELETE FROM orders",
        &Json::Null,
        limits(),
        true,
    )
    .unwrap_err();
    assert!(why.contains("use execute()"), "{why}");
    // A read-only connection: SQLite itself refuses the change.
    let conn = open(&def(&db, false), false, Duration::from_secs(1)).unwrap();
    let e = conn.execute("DELETE FROM orders", []).unwrap_err();
    assert_eq!(e.sqlite_error_code(), Some(ErrorCode::ReadOnly));
    drop(conn);
    // execute() changes rows and says how many.
    let changed = execute(
        &def(&db, true),
        "UPDATE orders SET note = ? WHERE customer = ?",
        &json!(["paid", "Ada"]),
        limits(),
    )
    .unwrap();
    assert_eq!(changed, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_file_is_not_created_and_the_size_limit_holds() {
    let (dir, db) = shop("missing");
    let missing = dir.join("nope.db");
    let why = execute(
        &def(&missing, true),
        "CREATE TABLE t (a)",
        &Json::Null,
        limits(),
    )
    .unwrap_err();
    assert!(why.contains("cannot be opened"), "{why}");
    assert!(!missing.exists());
    let small = Limits {
        max_bytes: 20,
        ..limits()
    };
    let why = query(
        &def(&db, false),
        "SELECT * FROM orders",
        &Json::Null,
        small,
        false,
    )
    .unwrap_err();
    assert!(why.contains("maximum response size of 20 bytes"), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_long_statement_stops_at_the_timeout() {
    let (dir, db) = shop("timeout");
    let short = Limits {
        timeout: Duration::from_millis(300),
        ..limits()
    };
    let started = Instant::now();
    let why = query(
        &def(&db, false),
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) \
         SELECT count(*) FROM n",
        &Json::Null,
        short,
        false,
    )
    .unwrap_err();
    assert!(why.contains("did not finish within 0.3 s"), "{why}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_test_reads_the_version_and_the_tables() {
    let (dir, db) = shop("test");
    let outcome = test(&def(&db, false), Duration::from_secs(5));
    assert!(outcome.ok, "{}", outcome.message);
    assert!(
        outcome.message.starts_with("SQLite 3.") && outcome.message.contains("1 table,"),
        "{}",
        outcome.message
    );
    // A file that is not a database fails the Test, and so does a folder.
    let text = dir.join("notes.txt");
    std::fs::write(&text, "this is not a database, it is a long line of text").unwrap();
    let outcome = test(&def(&text, false), Duration::from_secs(5));
    assert!(
        !outcome.ok && outcome.message.contains("not a database"),
        "{}",
        outcome.message
    );
    let outcome = test(&def(&dir, false), Duration::from_secs(5));
    assert!(
        !outcome.ok && outcome.message.contains("is not a file"),
        "{}",
        outcome.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}
