//! SQLite data sources (DESIGN.md, "Data sources", step 6b part 3): reading
//! with `query`, changing data with `execute`, and the Test.
//!
//! Each call opens the file, runs one statement and closes it again (SQLite
//! sources do not pool). The file is never created: a mistyped path is an
//! error, not a new empty database. Without *Allow writes*, and always in a
//! formula, the file is opened read-only, so SQLite itself refuses a change.

use std::time::{Duration, Instant};

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, ErrorCode, OpenFlags, Statement};
use serde_json::{Map, Value as Json, json};

use crate::http::Limits;
use crate::sources::{SourceDef, SourceKind, TestOutcome};

/// What a `query` found: the column names, their declared types (which the
/// prelude uses to give dates as dates), and the rows as lists of values.
pub fn query(
    def: &SourceDef,
    sql: &str,
    params: &Json,
    limits: Limits,
    writable: bool,
) -> Result<Json, String> {
    let _waiting = crate::sources::Waiting::begin();
    let conn = open(def, writable, limits.timeout)?;
    let mut stmt = prepare(&conn, def, sql)?;
    if !stmt.readonly() {
        return Err(format!(
            "{}: query() is for reading; use execute() for a statement that changes data",
            def.name
        ));
    }
    bind(&mut stmt, def, params)?;
    let columns: Vec<Json> = stmt
        .column_names()
        .map(|n| Json::String(n.to_owned()))
        .collect();
    let types: Vec<Json> = stmt
        .columns()
        .map(|c| {
            c.decl_type()
                .map_or(Json::Null, |t| Json::String(t.to_owned()))
        })
        .collect();
    let width = columns.len();
    let mut rows = Vec::new();
    let mut size: u64 = 0;
    let mut found = stmt.raw_query();
    loop {
        let row = match found.next() {
            Ok(Some(row)) => row,
            Ok(None) => break,
            Err(e) => return Err(describe(def, &e, limits)),
        };
        let mut values = Vec::with_capacity(width);
        for i in 0..width {
            let value = row.get_ref(i).map_err(|e| describe(def, &e, limits))?;
            size += match value {
                ValueRef::Text(t) | ValueRef::Blob(t) => t.len() as u64,
                _ => 8,
            };
            values.push(to_json(value));
        }
        if size > limits.max_bytes {
            return Err(format!(
                "{}: the rows are larger than the maximum response size of {} bytes",
                def.name, limits.max_bytes
            ));
        }
        rows.push(Json::Array(values));
    }
    Ok(json!({ "columns": columns, "types": types, "rows": rows }))
}

/// Runs a statement that changes data and gives the number of rows it
/// changed. The caller has checked *Allow writes* and where it runs.
pub fn execute(def: &SourceDef, sql: &str, params: &Json, limits: Limits) -> Result<u64, String> {
    let _waiting = crate::sources::Waiting::begin();
    let conn = open(def, true, limits.timeout)?;
    let mut stmt = prepare(&conn, def, sql)?;
    bind(&mut stmt, def, params)?;
    let changed = stmt.raw_execute().map_err(|e| describe(def, &e, limits))?;
    Ok(changed as u64)
}

/// The Test: opens the file read-only and reads its version and its tables,
/// which also shows that the file is a database.
pub fn test(def: &SourceDef, timeout: Duration) -> TestOutcome {
    let start = Instant::now();
    let path = def.path.as_deref().unwrap_or_default().trim();
    let conn = match open(def, false, timeout) {
        Ok(conn) => conn,
        Err(why) => return TestOutcome::failed(why),
    };
    let found = conn.query_row(
        "SELECT sqlite_version(), (SELECT count(*) FROM sqlite_schema WHERE type = 'table')",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    );
    match found {
        Ok((version, tables)) => TestOutcome {
            ok: true,
            message: format!(
                "SQLite {version}: {path} opened read-only, {tables} {}, in {} ms",
                if tables == 1 { "table" } else { "tables" },
                start.elapsed().as_millis()
            ),
        },
        Err(e) => TestOutcome::failed(format!("{path}: {}", plain(&e))),
    }
}

fn open(def: &SourceDef, writable: bool, timeout: Duration) -> Result<Connection, String> {
    if def.kind != SourceKind::Sqlite {
        return Err(format!(
            "{} is a {} source, not a SQLite one",
            def.name,
            def.kind.label()
        ));
    }
    let path = def.path.as_deref().unwrap_or_default().trim();
    if path.is_empty() {
        return Err(format!(
            "{}: a SQLite source needs the path of its file",
            def.name
        ));
    }
    // Opening a missing file without SQLITE_OPEN_CREATE fails, but with a
    // message that does not say which file; say it first.
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Err(format!("{path} is not a file")),
        Err(e) => return Err(format!("{path} cannot be opened: {e}")),
    }
    let access = match writable {
        true => OpenFlags::SQLITE_OPEN_READ_WRITE,
        false => OpenFlags::SQLITE_OPEN_READ_ONLY,
    };
    let mut conn = Connection::open_with_flags(path, access | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("{path} cannot be opened: {}", plain(&e)))?;
    // A locked database is waited for, and a long statement stopped, within
    // the call timeout.
    conn.busy_timeout(timeout)
        .map_err(|e| format!("{path}: {}", plain(&e)))?;
    let deadline = Instant::now() + timeout;
    conn.progress_handler(1000, Some(move || Instant::now() > deadline))
        .map_err(|e| format!("{path}: {}", plain(&e)))?;
    Ok(conn)
}

fn prepare<'c>(conn: &'c Connection, def: &SourceDef, sql: &str) -> Result<Statement<'c>, String> {
    match conn.prepare(sql) {
        Ok(stmt) => Ok(stmt),
        Err(rusqlite::Error::MultipleStatement) => Err(format!(
            "{}: one statement at a time, please: the SQL goes on after its first ;",
            def.name
        )),
        Err(e) => Err(format!("{}: {}", def.name, plain(&e))),
    }
}

/// Binds the parameters: a list for `?` (or `?1`), a dict for `:name`.
fn bind(stmt: &mut Statement<'_>, def: &SourceDef, params: &Json) -> Result<(), String> {
    let wanted = stmt.parameter_count();
    match params {
        Json::Null => {
            if wanted > 0 {
                return Err(format!(
                    "{}: the SQL has {wanted} {}, and no values were given",
                    def.name,
                    parameters(wanted)
                ));
            }
        }
        Json::Array(values) => {
            if values.len() != wanted {
                return Err(format!(
                    "{}: the SQL has {wanted} {}, and {} {} given",
                    def.name,
                    parameters(wanted),
                    values.len(),
                    if values.len() == 1 {
                        "value was"
                    } else {
                        "values were"
                    }
                ));
            }
            for (i, value) in values.iter().enumerate() {
                let value = from_json(value).map_err(|e| format!("{}: {e}", def.name))?;
                stmt.raw_bind_parameter(i + 1, value)
                    .map_err(|e| format!("{}: {}", def.name, plain(&e)))?;
            }
        }
        Json::Object(named) => {
            let mut bound = 0;
            for (name, value) in named {
                let key = match name.starts_with([':', '@', '$']) {
                    true => name.clone(),
                    false => format!(":{name}"),
                };
                let index = stmt
                    .parameter_index(&key)
                    .map_err(|e| format!("{}: {}", def.name, plain(&e)))?
                    .ok_or_else(|| format!("{}: the SQL has no parameter {key}", def.name))?;
                let value = from_json(value).map_err(|e| format!("{}: {e}", def.name))?;
                stmt.raw_bind_parameter(index, value)
                    .map_err(|e| format!("{}: {}", def.name, plain(&e)))?;
                bound += 1;
            }
            if bound < wanted {
                return Err(format!(
                    "{}: the SQL has {wanted} {}, and {bound} {} given",
                    def.name,
                    parameters(wanted),
                    if bound == 1 {
                        "value was"
                    } else {
                        "values were"
                    }
                ));
            }
        }
        _ => {
            return Err(format!(
                "{}: the parameters are a list (for ?) or a dict (for :name)",
                def.name
            ));
        }
    }
    Ok(())
}

fn parameters(n: usize) -> &'static str {
    if n == 1 { "parameter" } else { "parameters" }
}

/// A value from the prelude: JSON, with bytes as `{"$bytes": "<hex>"}`.
/// Dates arrive as ISO text, as SQLite keeps them.
fn from_json(value: &Json) -> Result<SqlValue, String> {
    Ok(match value {
        Json::Null => SqlValue::Null,
        Json::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Json::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => SqlValue::Real(n.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(s) => SqlValue::Text(s.clone()),
        Json::Object(o) if o.len() == 1 && o.contains_key("$bytes") => {
            let hex = o["$bytes"].as_str().unwrap_or_default();
            SqlValue::Blob(from_hex(hex).ok_or("bytes that did not arrive whole")?)
        }
        other => return Err(format!("{other} cannot be passed to SQL")),
    })
}

fn to_json(value: ValueRef<'_>) -> Json {
    match value {
        ValueRef::Null => Json::Null,
        ValueRef::Integer(i) => Json::from(i),
        // JSON has no NaN or infinity; SQLite keeps NaN as NULL anyway.
        ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number),
        ValueRef::Text(t) => Json::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => {
            let mut o = Map::new();
            o.insert("$bytes".to_owned(), Json::String(to_hex(b)));
            Json::Object(o)
        }
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// A failed call in words, with the timeout named when that was the cause.
fn describe(def: &SourceDef, error: &rusqlite::Error, limits: Limits) -> String {
    match error.sqlite_error_code() {
        Some(ErrorCode::OperationInterrupted) => format!(
            "{}: the statement did not finish within {} s",
            def.name,
            limits.timeout.as_secs_f32()
        ),
        Some(ErrorCode::DatabaseBusy) | Some(ErrorCode::DatabaseLocked) => format!(
            "{}: the database stayed locked by another program for {} s",
            def.name,
            limits.timeout.as_secs_f32()
        ),
        Some(ErrorCode::ReadOnly) => format!(
            "{}: the database is open read-only: {}",
            def.name,
            plain(error)
        ),
        _ => format!("{}: {}", def.name, plain(error)),
    }
}

/// SQLite's own message, without rusqlite's wrapping.
fn plain(error: &rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(_, Some(message)) => message.clone(),
        rusqlite::Error::SqlInputError { msg, .. } => msg.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "sqlite_tests.rs"]
pub(crate) mod sqlite_tests;
