//! Tests of the PostgreSQL and MariaDB sources. The ones that need a server
//! run only when an environment variable points at one, as a URL:
//!
//! - `CLIOTREE_TEST_POSTGRES=postgres://user:password@host:5432/database`
//! - `CLIOTREE_TEST_MARIADB=mariadb://user:password@host:3306/database`
//!
//! `?tls=off`, `?tls=require` or `?tls=unchecked` at the end sets the TLS
//! setting (default: prefer). The account needs to create and drop a table. Without the
//! variable such a test passes without doing anything, and says so.

use serde_json::json;

use super::*;
use crate::sources::TlsMode;

/// A definition of the server named by the environment variable, if set.
pub(crate) fn server(kind: SourceKind) -> Option<SourceDef> {
    let var = match kind {
        SourceKind::Postgres => "CLIOTREE_TEST_POSTGRES",
        _ => "CLIOTREE_TEST_MARIADB",
    };
    // CI passes a repository secret, which is empty when not set.
    let Some(url) = std::env::var(var).ok().filter(|u| !u.trim().is_empty()) else {
        eprintln!("{var} is not set: skipped");
        return None;
    };
    let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (account, place) = rest
        .rsplit_once('@')
        .expect("user:password@host:port/database");
    let (user, password) = account.split_once(':').unwrap_or((account, ""));
    let (hostport, database) = place.split_once('/').expect("host:port/database");
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()),
        None => (hostport, None),
    };
    let tls = match query {
        "tls=off" => TlsMode::Off,
        "tls=require" => TlsMode::Require,
        "tls=unchecked" => TlsMode::Unchecked,
        _ => TlsMode::Prefer,
    };
    Some(SourceDef {
        host: Some(host.to_owned()),
        port,
        database: Some(database.to_owned()),
        user: Some(user.to_owned()),
        password: Some(password.to_owned()).filter(|p| !p.is_empty()),
        tls,
        writes: true,
        ..SourceDef::new("Live", kind)
    })
}

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(10),
        max_bytes: 1 << 20,
    }
}

#[test]
fn values_travel_as_the_prelude_expects() {
    assert_eq!(bytes_json(&[0xca, 0xfe]), json!({"$bytes": "cafe"}));
    assert_eq!(
        json_bytes(&json!({"$bytes": "cafe"})),
        Some(vec![0xca, 0xfe])
    );
    assert_eq!(json_bytes(&json!({"$bytes": "caf"})), None);
    assert_eq!(json_bytes(&json!({"a": 1})), None);
    assert_eq!(decimal_json("12"), json!(12));
    assert_eq!(decimal_json("-12.50"), json!(-12.5));
    assert_eq!(decimal_json("99999999999999999999"), json!(1e20));
    assert_eq!(decimal_json("NaN"), json!("NaN"));
}

#[test]
fn a_server_that_cannot_be_reached_says_where() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    for kind in [SourceKind::Postgres, SourceKind::Mariadb] {
        let def = SourceDef {
            host: Some("127.0.0.1".into()),
            port: Some(port),
            database: Some("x".into()),
            user: Some("u".into()),
            tls: TlsMode::Off,
            ..SourceDef::new("Gone", kind)
        };
        let why = query(&def, "SELECT 1", &Json::Null, limits()).unwrap_err();
        assert!(
            why.starts_with("Gone: ") && why.contains(&format!("127.0.0.1:{port}")),
            "{why}"
        );
        // With a pool, the failed connection is not counted as open: the
        // next call tries again rather than waiting for a free one.
        let pooled = SourceDef {
            pool: Some(1),
            ..def.clone()
        };
        for _ in 0..2 {
            let why = query(&pooled, "SELECT 1", &Json::Null, limits()).unwrap_err();
            assert!(!why.contains("stayed in use"), "{why}");
        }
    }
}

/// The SQL of the live test for each kind: a table with a whole number,
/// text, a decimal, a date, a date and time, bytes, JSON and a flag.
fn table(kind: SourceKind, name: &str) -> String {
    match kind {
        SourceKind::Postgres => format!(
            "CREATE TABLE {name} (id int PRIMARY KEY, customer text, total numeric(10,2), \
             placed date, shipped timestamp, stamp bytea, info jsonb, flag boolean)"
        ),
        _ => format!(
            "CREATE TABLE {name} (id INT PRIMARY KEY, customer VARCHAR(40), total DECIMAL(10,2), \
             placed DATE, shipped DATETIME, stamp VARBINARY(10), info JSON, flag BOOLEAN)"
        ),
    }
}

fn live(kind: SourceKind) {
    let Some(def) = server(kind) else {
        return;
    };
    let outcome = test(&def, Duration::from_secs(10));
    assert!(outcome.ok, "{}", outcome.message);
    eprintln!("{}", outcome.message);
    let name = format!("cliotree_test_{}", std::process::id());
    let p = |i: usize| match kind {
        SourceKind::Postgres => format!("${i}"),
        _ => "?".to_owned(),
    };
    let _ = execute(
        &def,
        &format!("DROP TABLE IF EXISTS {name}"),
        &Json::Null,
        limits(),
    );
    execute(&def, &table(kind, &name), &Json::Null, limits()).unwrap();
    let insert = format!(
        "INSERT INTO {name} VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
        p(1),
        p(2),
        p(3),
        p(4),
        p(5),
        p(6),
        p(7),
        p(8)
    );
    let changed = execute(
        &def,
        &insert,
        &json!([1, "Ada", 12.5, "2026-10-01", "2026-10-02 09:30:00", {"$bytes": "cafe"}, "{\"a\": 1}", true]),
        limits(),
    )
    .unwrap();
    assert_eq!(changed, 1);
    execute(
        &def,
        &insert,
        &json!([2, "Bob", 7, null, null, null, null, false]),
        limits(),
    )
    .unwrap();

    let found = query(
        &def,
        &format!("SELECT * FROM {name} ORDER BY id"),
        &Json::Null,
        limits(),
    )
    .unwrap();
    assert_eq!(
        found["columns"],
        json!([
            "id", "customer", "total", "placed", "shipped", "stamp", "info", "flag"
        ])
    );
    assert_eq!(found["types"][3], json!("DATE"));
    let row = &found["rows"][0];
    assert_eq!(row[0], json!(1));
    assert_eq!(row[1], json!("Ada"));
    assert_eq!(row[2], json!(12.5));
    assert_eq!(row[3], json!("2026-10-01"));
    assert_eq!(row[4], json!("2026-10-02 09:30:00"));
    assert_eq!(row[5], json!({"$bytes": "cafe"}));
    let info: Json = serde_json::from_str(row[6].as_str().unwrap()).unwrap();
    assert_eq!(info, json!({"a": 1}));
    // PostgreSQL has booleans; MariaDB's BOOLEAN is a TINYINT(1).
    assert!(row[7] == json!(true) || row[7] == json!(1), "{}", row[7]);
    assert_eq!(found["rows"][1][3], Json::Null);

    // Parameters, and the count checked.
    let found = query(
        &def,
        &format!("SELECT customer FROM {name} WHERE total > {}", p(1)),
        &json!([10]),
        limits(),
    )
    .unwrap();
    assert_eq!(found["rows"], json!([["Ada"]]));
    let why = query(
        &def,
        &format!("SELECT customer FROM {name} WHERE id = {}", p(1)),
        &json!([]),
        limits(),
    )
    .unwrap_err();
    assert!(
        why.contains("1 parameter, and 0 values were given"),
        "{why}"
    );
    if kind == SourceKind::Mariadb {
        let found = query(
            &def,
            &format!("SELECT customer FROM {name} WHERE id = :id"),
            &json!({"id": 2}),
            limits(),
        )
        .unwrap();
        assert_eq!(found["rows"], json!([["Bob"]]));
    }

    // A query runs read-only, whatever the definition allows.
    let why = query(&def, &format!("DELETE FROM {name}"), &Json::Null, limits()).unwrap_err();
    assert!(why.to_lowercase().contains("read"), "{why}");
    assert_eq!(
        query(
            &def,
            &format!("SELECT count(*) FROM {name}"),
            &Json::Null,
            limits()
        )
        .unwrap()["rows"],
        json!([[2]])
    );

    // A pool of one connection serves call after call.
    let pooled = SourceDef {
        pool: Some(1),
        ..def.clone()
    };
    for _ in 0..3 {
        query(&pooled, "SELECT 1", &Json::Null, limits()).unwrap();
    }
    // A failed statement does not spoil the pooled connection.
    assert!(query(&pooled, "SELEC 1", &Json::Null, limits()).is_err());
    query(&pooled, "SELECT 1", &Json::Null, limits()).unwrap();

    // The size limit.
    let small = Limits {
        max_bytes: 4,
        ..limits()
    };
    let why = query(
        &def,
        &format!("SELECT customer FROM {name}"),
        &Json::Null,
        small,
    )
    .unwrap_err();
    assert!(why.contains("maximum response size"), "{why}");

    execute(&def, &format!("DROP TABLE {name}"), &Json::Null, limits()).unwrap();
}

#[test]
fn postgres_reads_writes_and_pools() {
    live(SourceKind::Postgres);
}

#[test]
fn mariadb_reads_writes_and_pools() {
    live(SourceKind::Mariadb);
}

#[test]
fn a_wrong_password_is_refused_by_the_test() {
    for kind in [SourceKind::Postgres, SourceKind::Mariadb] {
        let Some(mut def) = server(kind) else {
            continue;
        };
        def.password = Some("not the password".into());
        let outcome = test(&def, Duration::from_secs(10));
        assert!(!outcome.ok, "{}", outcome.message);
        eprintln!("{}", outcome.message);
    }
}

/// An ODBC definition from `CLIOTREE_TEST_ODBC`, a connection string with
/// the account in it (`Driver={PostgreSQL Unicode};Server=...;UID=...;PWD=...`):
/// the account goes into its own fields, as a definition keeps it.
#[cfg(windows)]
fn odbc_server() -> Option<SourceDef> {
    let Some(text) = std::env::var("CLIOTREE_TEST_ODBC")
        .ok()
        .filter(|t| !t.trim().is_empty())
    else {
        eprintln!("CLIOTREE_TEST_ODBC is not set: skipped");
        return None;
    };
    let mut def = SourceDef {
        writes: true,
        ..SourceDef::new("Live", SourceKind::Odbc)
    };
    let mut rest = Vec::new();
    for pair in text.split(';').filter(|p| !p.trim().is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = value.trim_matches(['{', '}']).to_owned();
        match key.trim().to_ascii_uppercase().as_str() {
            "UID" => def.user = Some(value),
            "PWD" => def.password = Some(value),
            _ => rest.push(pair.to_owned()),
        }
    }
    def.connection = Some(rest.join(";"));
    Some(def)
}

/// PostgreSQL through its ODBC driver: rows and types, `?` parameters and
/// their count, a query rolled back, `execute` counting rows, a pool, and
/// read-only asked of the driver without *Allow writes*.
#[cfg(windows)]
#[test]
fn odbc_reads_writes_and_pools() {
    let Some(def) = odbc_server() else {
        return;
    };
    let outcome = test(&def, Duration::from_secs(10));
    assert!(outcome.ok, "{}", outcome.message);
    eprintln!("{}", outcome.message);
    let name = format!("cliotree_odbc_{}", std::process::id());
    let run = |sql: &str, params: Json| execute(&def, sql, &params, limits());
    let _ = run(&format!("DROP TABLE IF EXISTS {name}"), Json::Null);
    run(
        &format!(
            "CREATE TABLE {name} (id int PRIMARY KEY, customer text, total numeric(10,2), \
             placed date, shipped timestamp, stamp bytea, flag boolean)"
        ),
        Json::Null,
    )
    .unwrap();
    let insert = format!("INSERT INTO {name} VALUES (?, ?, ?, ?, ?, ?, ?)");
    assert_eq!(
        run(
            &insert,
            json!([1, "Ada", 12.5, "2026-10-01", "2026-10-02 09:30:00", {"$bytes": "cafe"}, true])
        )
        .unwrap(),
        1
    );
    run(&insert, json!([2, "Bob", 7, null, null, null, false])).unwrap();

    let found = query(
        &def,
        &format!("SELECT * FROM {name} ORDER BY id"),
        &Json::Null,
        limits(),
    )
    .unwrap();
    assert_eq!(
        found["columns"],
        json!([
            "id", "customer", "total", "placed", "shipped", "stamp", "flag"
        ])
    );
    assert_eq!(found["types"][3], json!("DATE"));
    assert_eq!(found["types"][4], json!("TIMESTAMP"));
    let row = &found["rows"][0];
    assert_eq!(row[0], json!(1));
    assert_eq!(row[1], json!("Ada"));
    assert_eq!(row[2], json!(12.5));
    assert_eq!(row[3], json!("2026-10-01"));
    assert_eq!(row[4], json!("2026-10-02 09:30:00"));
    assert_eq!(row[5], json!({"$bytes": "cafe"}));
    // The driver may give a boolean as a bit or as the text "1".
    assert!(row[6] == json!(true) || row[6] == json!("1"), "{}", row[6]);
    assert_eq!(found["rows"][1][3], Json::Null);

    let found = query(
        &def,
        &format!("SELECT customer FROM {name} WHERE total > ?"),
        &json!([10]),
        limits(),
    )
    .unwrap();
    assert_eq!(found["rows"], json!([["Ada"]]));
    let why = query(
        &def,
        &format!("SELECT customer FROM {name} WHERE id = ?"),
        &json!([]),
        limits(),
    )
    .unwrap_err();
    assert!(
        why.contains("1 parameter, and 0 values were given"),
        "{why}"
    );
    let why = query(&def, "SELECT 1", &json!({"id": 1}), limits()).unwrap_err();
    assert!(why.contains("not :name"), "{why}");

    // A query's change is rolled back, even with Allow writes.
    let _ = query(&def, &format!("DELETE FROM {name}"), &Json::Null, limits());
    let count = |def: &SourceDef| {
        query(
            def,
            &format!("SELECT count(*) FROM {name}"),
            &Json::Null,
            limits(),
        )
        .unwrap()["rows"][0][0]
            .clone()
    };
    assert_eq!(count(&def), json!(2));

    // Without Allow writes the driver is asked for read-only access, which
    // the PostgreSQL driver honours: execute fails at the server.
    let read_only = SourceDef {
        writes: false,
        ..def.clone()
    };
    assert!(
        execute(
            &read_only,
            &format!("DELETE FROM {name}"),
            &Json::Null,
            limits()
        )
        .is_err()
    );
    assert_eq!(count(&read_only), json!(2));

    // A pool of one connection serves call after call, and survives an error.
    let pooled = SourceDef {
        pool: Some(1),
        ..def.clone()
    };
    for _ in 0..3 {
        assert_eq!(count(&pooled), json!(2));
    }
    assert!(query(&pooled, "SELEC 1", &Json::Null, limits()).is_err());
    assert_eq!(count(&pooled), json!(2));

    run(&format!("DROP TABLE {name}"), Json::Null).unwrap();

    let mut wrong = def.clone();
    wrong.password = Some("not the password".into());
    let outcome = test(&wrong, Duration::from_secs(10));
    assert!(!outcome.ok, "{}", outcome.message);
    eprintln!("{}", outcome.message);
}

/// A folder of CSV files through the Access text driver, which comes with
/// Office and the Access Database Engine; skipped where it is not installed.
/// The driver has no transactions, so a query runs as it is.
#[cfg(windows)]
#[test]
fn odbc_reads_a_csv_file() {
    let dir = std::env::temp_dir().join(format!("cliotree_odbc_csv_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("orders.csv"),
        "id,customer,total\n1,Ada,12.5\n2,Bob,7\n",
    )
    .unwrap();
    let def = SourceDef {
        connection: Some(format!(
            "Driver={{Microsoft Access Text Driver (*.txt, *.csv)}};DBQ={}",
            dir.display()
        )),
        ..SourceDef::new("Csv", SourceKind::Odbc)
    };
    let outcome = test(&def, Duration::from_secs(10));
    if !outcome.ok && outcome.message.contains("not found") {
        eprintln!(
            "the Access text driver is not installed: skipped ({})",
            outcome.message
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(outcome.ok, "{}", outcome.message);
    eprintln!("{}", outcome.message);
    let found = query(
        &def,
        "SELECT customer, total FROM orders.csv WHERE total > ? ORDER BY id",
        &json!([5]),
        limits(),
    )
    .unwrap();
    assert_eq!(found["columns"], json!(["customer", "total"]));
    // The driver takes the column for a float, so 7 comes back as 7.0.
    assert_eq!(found["rows"], json!([["Ada", 12.5], ["Bob", 7.0]]));
    let _ = std::fs::remove_dir_all(&dir);
}
