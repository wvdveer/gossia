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
