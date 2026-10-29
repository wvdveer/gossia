//! Tests of the data sources as Python sees them (DESIGN.md, "Data sources"):
//! `source(name)`, `sources.define` and `sources.check`, against the real
//! interpreter.

use std::time::Duration;

use cliotree_core::{ErrorKind, Path, Value, json, jsonschema};

use super::tests::Book;
use super::{PythonEngine, PythonSettings, PythonSyntax};
use crate::sources::{SourceDef, SourceKind, SourceStore};
use crate::{Calc, EvalSettings, Evaluator, Session};

fn sales() -> SourceDef {
    SourceDef {
        host: Some("db.example.com".into()),
        database: Some("sales".into()),
        user: Some("ro".into()),
        password: Some("hunter2".into()),
        ..SourceDef::new("Sales", SourceKind::Postgres)
    }
}

fn settings(store: &SourceStore) -> PythonSettings {
    PythonSettings {
        sources: store.clone(),
        source_timeout: Duration::from_secs(5),
        ..PythonSettings::default()
    }
}

#[test]
fn a_formula_finds_a_source_by_name_and_never_its_secret() {
    let store = SourceStore::in_memory();
    store.save(sales(), None, false).unwrap();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    b.formula("name", "source('Sales').target");
    b.formula("text", "source('Sales').kind + str(source('Sales').writes)");
    b.formula("note", "source('Nowhere').target");
    b.formula(
        "flag",
        "hasattr(source('Sales'), 'password') or 'hunter2' in repr(source('Sales'))",
    );
    b.run();
    assert_eq!(
        b.get("name"),
        Some(Value::Str(
            "postgres://db.example.com:5432/sales as ro".into()
        ))
    );
    assert_eq!(b.get("text"), Some(Value::Str("postgresFalse".into())));
    assert_eq!(b.error("note"), Some(ErrorKind::Source));
    assert!(b.message("note").contains("no data source called Nowhere"));
    assert_eq!(b.get("flag"), Some(Value::Bool(false)));
}

#[test]
fn a_formula_cannot_define_or_check_a_source() {
    let store = SourceStore::in_memory();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    b.formula("name", "sources.define('Local', 'sqlite', path='x.db')");
    b.formula("text", "sources.check('Local').message");
    b.run();
    assert_eq!(b.error("name"), Some(ErrorKind::Source));
    assert!(b.message("name").contains("button procedure"));
    assert_eq!(b.error("text"), Some(ErrorKind::Source));
    assert!(store.list().unwrap().is_empty());
}

const SCHEMA: &str = r#"{"type": "object", "properties": {"a": {"type": "integer"}}}"#;

fn session(store: &SourceStore) -> Session {
    let mut calc = Calc::new(std::sync::Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"a": 1}"#).unwrap().node,
    )
    .unwrap();
    Session::new(
        calc,
        Evaluator::new(super::factory(settings(store)), EvalSettings::default()),
    )
}

fn immediate(s: &mut Session, text: &str) -> Result<String, String> {
    s.run_immediate(text, "S", &Path::root())?;
    assert!(s.wait_idle(Duration::from_secs(60)));
    s.take_immediate().remove(0).result
}

#[test]
fn the_immediate_window_defines_and_checks_a_source() {
    let store = SourceStore::in_memory();
    let mut s = session(&store);
    let dir = std::env::temp_dir().join(format!("cliotree-src-py-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("x.db");
    std::fs::write(&db, b"").unwrap();
    // Forward slashes, so the path needs no escaping inside a Python string.
    let path = db.display().to_string().replace('\\', "/");

    let missing = immediate(&mut s, "sources.check('Local')").unwrap();
    assert!(missing.contains("exists=False"), "{missing}");
    assert_eq!(
        immediate(
            &mut s,
            &format!("sources.define('Local', 'sqlite', path='{path}')")
        ),
        Ok("None".to_owned())
    );
    assert_eq!(
        store.get("Local").unwrap().unwrap().kind,
        SourceKind::Sqlite
    );
    assert_eq!(
        immediate(&mut s, "bool(sources.check('Local'))"),
        Ok("True".to_owned())
    );
    // Defining it again needs replace=True; a misspelt setting is refused.
    let again = immediate(
        &mut s,
        &format!("sources.define('Local', 'sqlite', path='{path}')"),
    )
    .unwrap_err();
    assert!(
        again.starts_with("#SOURCE!") && again.contains("replace=True"),
        "{again}"
    );
    assert_eq!(
        immediate(
            &mut s,
            &format!("sources.define('Local', 'sqlite', path='{path}', writes=True, replace=True)")
        ),
        Ok("None".to_owned())
    );
    assert!(store.get("Local").unwrap().unwrap().writes);
    let typo = immediate(&mut s, "sources.define('X', 'sqlite', paht='y.db')").unwrap_err();
    assert!(typo.contains("paht"), "{typo}");
    assert_eq!(
        immediate(&mut s, "source('Local').writes"),
        Ok("True".to_owned())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn source_names_are_reserved() {
    for name in ["source", "sources", "SourceError"] {
        assert!(crate::names::is_reserved(name), "{name}");
    }
}

#[test]
fn a_formula_reads_an_http_source_and_a_failure_is_source() {
    let server = crate::http::http_tests::Server::start();
    let store = SourceStore::in_memory();
    store
        .save(crate::http::http_tests::api(&server.url), None, false)
        .unwrap();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    b.formula("a", "source('Api').get('json').json()['total']");
    b.formula(
        "name",
        "source('Api').get('echo', params={'q': 'x y', 'on': True}).text",
    );
    b.formula("note", "source('Api').get('missing').text");
    b.formula("tags", "source('Api').get('page').json()");
    b.formula("text", "source('Api').post('orders', json={'a': 1}).text");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(42)));
    assert_eq!(b.get("name"), Some(Value::Str("ok".into())));
    assert!(
        server
            .received
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.target == "/v1/echo?q=x%20y&on=true")
    );
    assert_eq!(b.error("note"), Some(ErrorKind::Source));
    assert!(b.message("note").contains("404"), "{}", b.message("note"));
    // Text where JSON was expected says what came back.
    let why = b.message("tags");
    assert!(
        why.contains("Api: GET page answered 200 OK") && why.contains("not JSON: ok"),
        "{why}"
    );
    // A write is refused: the source does not allow writes.
    assert_eq!(b.error("text"), Some(ErrorKind::Source));
    assert!(
        b.message("text").contains("Allow writes"),
        "{}",
        b.message("text")
    );
}

#[test]
fn a_read_is_made_once_per_pass_and_again_on_the_next() {
    let server = crate::http::http_tests::Server::start();
    let store = SourceStore::in_memory();
    store
        .save(crate::http::http_tests::api(&server.url), None, false)
        .unwrap();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    // The same read in two formulas, and a failing one twice: one request each.
    b.formula("a", "source('Api').get('json').json()['total']");
    b.formula("name", "str(source('Api').get('json').json()['items'])");
    b.formula("note", "source('Api').get('missing').text");
    b.formula("text", "source('Api').get('missing').text");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(42)));
    assert_eq!(b.get("name"), Some(Value::Str("[1, 2]".into())));
    assert_eq!(b.error("text"), Some(ErrorKind::Source));
    assert_eq!(server.count(), 2);
    // Other parameters are another read.
    b.formula("text", "source('Api').get('json', params={'p': 2}).text");
    b.run();
    assert!(matches!(b.get("text"), Some(Value::Str(t)) if t.contains("42")));
    assert_eq!(server.count(), 3);
    // Recalculate (F9) is a new pass, which reads everything again.
    b.calc.recalc_all();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(42)));
    assert_eq!(server.count(), 6);
    // The Immediate window always reads afresh.
    let mut s = session(&store);
    for _ in 0..2 {
        assert_eq!(
            immediate(&mut s, "source('Api').get('json').status"),
            Ok("200".to_owned())
        );
    }
    assert_eq!(server.count(), 8);
}

#[test]
fn a_write_needs_allow_writes_and_a_procedure_or_the_immediate_window() {
    let server = crate::http::http_tests::Server::start();
    let store = SourceStore::in_memory();
    let mut def = crate::http::http_tests::api(&server.url);
    def.writes = true;
    store.save(def, None, false).unwrap();
    // Not in a formula, even when writes are allowed.
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    b.formula("text", "source('Api').post('orders', json={'a': 1}).text");
    b.run();
    assert!(
        b.message("text").contains("button procedure"),
        "{}",
        b.message("text")
    );
    // In the Immediate window, with JSON and with a form.
    let mut s = session(&store);
    assert_eq!(
        immediate(
            &mut s,
            "source('Api').post('orders', json={'a': [1, 2]}).status"
        ),
        Ok("200".to_owned())
    );
    let sent = server.last();
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.body.replace(' ', ""), r#"{"a":[1,2]}"#);
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(
        immediate(
            &mut s,
            "source('Api').put('orders/1', data={'name': 'a&b'}).text"
        ),
        Ok("'ok'".to_owned())
    );
    let sent = server.last();
    assert_eq!(
        (sent.method.as_str(), sent.body.as_str()),
        ("PUT", "name=a%26b")
    );
    assert_eq!(server.count(), 2);
}

#[test]
fn waiting_for_a_source_does_not_count_against_the_formula_limit() {
    let server = crate::http::http_tests::Server::start();
    let store = SourceStore::in_memory();
    store
        .save(crate::http::http_tests::api(&server.url), None, false)
        .unwrap();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    // The server takes 1.5 s; the formula may run for 0.5 s.
    b.engine.set_time_limit(Some(Duration::from_millis(500)));
    b.formula("name", "source('Api').get('slow').text");
    b.run();
    assert_eq!(b.get("name"), Some(Value::Str("late".into())));
}

#[test]
fn the_evaluation_thread_does_not_give_up_on_a_formula_that_waits_for_a_source() {
    let server = crate::http::http_tests::Server::start();
    let store = SourceStore::in_memory();
    store
        .save(crate::http::http_tests::api(&server.url), None, false)
        .unwrap();
    let mut calc = Calc::new(std::sync::Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"a": 1}"#).unwrap().node,
    )
    .unwrap();
    let eval = EvalSettings {
        formula_limit: Some(Duration::from_millis(300)),
        grace: Duration::from_millis(200),
        ..EvalSettings::default()
    };
    let mut s = Session::new(calc, Evaluator::new(super::factory(settings(&store)), eval));
    s.calc_mut()
        .set_formula(
            "S",
            &Path::root().key("a"),
            "len(source('Api').get('slow').text)",
        )
        .unwrap();
    assert!(s.wait_idle(Duration::from_secs(60)));
    let a = s
        .calc()
        .data("S")
        .unwrap()
        .get(&Path::root().key("a"))
        .map(|n| n.value().clone());
    assert_eq!(a, Some(Value::Int(4)));
    assert_eq!(s.evaluator().abandoned(), 0);
}

/// The owner's test case, through Python: see `http_tests::RANDOM_USER`.
#[test]
#[ignore = "uses the internet: randomuser.me"]
fn a_formula_reads_randomuser_me() {
    let store = SourceStore::in_memory();
    store
        .save(crate::http::http_tests::random_user(), None, false)
        .unwrap();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(PythonSettings {
        source_timeout: Duration::from_secs(30),
        ..settings(&store)
    });
    b.formula(
        "a",
        "len(source('RandomUser').get(params={'results': 2}).json()['results'])",
    );
    b.formula(
        "name",
        "source('RandomUser').get().json()['results'][0]['name']['first']",
    );
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(2)));
    assert!(
        matches!(b.get("name"), Some(Value::Str(s)) if !s.is_empty()),
        "{:?}",
        b.get("name")
    );
}

#[test]
fn a_formula_queries_a_sqlite_source() {
    let (dir, db) = crate::sqlite::sqlite_tests::shop("formula");
    let store = SourceStore::in_memory();
    store
        .save(crate::sqlite::sqlite_tests::def(&db, false), None, false)
        .unwrap();
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    b.formula("a", "len(source('Shop').query('SELECT * FROM orders'))");
    b.formula(
        "name",
        "source('Shop').query('SELECT customer FROM orders WHERE id = ?', [2])[0]['customer']",
    );
    b.formula(
        "x",
        "sum(r['total'] for r in source('Shop').query('SELECT total FROM orders'))",
    );
    // Dates and times by the declared type, bytes, and NULL as None.
    b.formula(
        "tags",
        "[str(v) for v in source('Shop').query(\
         'SELECT placed, shipped, stamp, note FROM orders WHERE id = 1')[0].values()]",
    );
    b.formula(
        "c",
        "sum(r['placed'].year + r['shipped'].hour + len(r['stamp']) \
         for r in source('Shop').query('SELECT * FROM orders WHERE id = 1'))",
    );
    // Text in a DATE column that is not a date stays text.
    b.formula(
        "text",
        "source('Shop').query('SELECT placed FROM orders WHERE id = 3')[0]['placed']",
    );
    // A node as a parameter, by name.
    b.formula(
        "nums",
        "[r['id'] for r in source('Shop').query(\
         'SELECT id FROM orders WHERE id >= :least ORDER BY id', {'least': S.b})]",
    );
    b.formula("note", "source('Shop').execute('DELETE FROM orders')");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(3)));
    assert_eq!(b.get("name"), Some(Value::Str("Bob".into())));
    assert_eq!(b.get("x"), Some(Value::Float(22.75)));
    assert_eq!(
        b.get("tags"),
        Some(Value::Array(
            ["2026-10-01", "2026-10-02 09:30:00", r"b'\xca\xfe'", "None"]
                .map(|t| Value::Str(t.into()).into())
                .to_vec()
        ))
    );
    assert_eq!(b.get("c"), Some(Value::Int(2026 + 9 + 2)));
    assert_eq!(b.get("text"), Some(Value::Str("soon".into())));
    assert_eq!(
        b.get("nums"),
        Some(Value::Array(vec![
            Value::Int(2).into(),
            Value::Int(3).into()
        ]))
    );
    assert_eq!(b.error("note"), Some(ErrorKind::Source));
    assert!(
        b.message("note").contains("Allow writes"),
        "{}",
        b.message("note")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sqlite_writes_need_allow_writes_and_the_immediate_window() {
    let (dir, db) = crate::sqlite::sqlite_tests::shop("writes");
    let store = SourceStore::in_memory();
    store
        .save(crate::sqlite::sqlite_tests::def(&db, true), None, false)
        .unwrap();
    // Not in a formula, even when writes are allowed; and a formula's
    // query() is refused a statement that changes data.
    let mut b = Book::new();
    b.engine = PythonEngine::with_settings(settings(&store));
    b.formula("text", "source('Shop').execute('DELETE FROM orders')");
    b.formula("note", "source('Shop').query('DELETE FROM orders')");
    b.formula("name", "source('Shop').get('x').text");
    b.run();
    assert!(
        b.message("text").contains("button procedure"),
        "{}",
        b.message("text")
    );
    assert!(
        b.message("note").contains("use execute()"),
        "{}",
        b.message("note")
    );
    assert!(
        b.message("name").contains("read with query()"),
        "{}",
        b.message("name")
    );
    // The Immediate window changes data and reads the change back.
    let mut s = session(&store);
    assert_eq!(
        immediate(
            &mut s,
            "source('Shop').execute('UPDATE orders SET note = ? WHERE customer = ?', ['paid', 'Ada'])"
        ),
        Ok("2".to_owned())
    );
    assert_eq!(
        immediate(
            &mut s,
            "[r['note'] for r in source('Shop').query('SELECT note FROM orders ORDER BY id')]"
        ),
        Ok("['paid', 'gift', 'paid']".to_owned())
    );
    let _ = std::fs::remove_dir_all(&dir);
}
