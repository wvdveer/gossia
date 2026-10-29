//! The ODBC side of `database`, on Windows only: any database with an ODBC
//! driver, reached through a DSN or a connection string.
//!
//! Read-only is best effort (DESIGN.md, "Decisions made for ODBC data
//! sources"): without *Allow writes* the connection asks the driver for
//! `ReadOnly=1`, and a query runs in a transaction that is rolled back where
//! the driver has transactions. Drivers that ignore both are protected only by
//! the account in the definition. Parameters are `?` with a list.

use std::sync::OnceLock;
use std::time::Duration;

use odbc_api::handles::DataType;
use odbc_api::parameter::InputParameter;
use odbc_api::{
    Bit, Connection, ConnectionOptions, Cursor, CursorRow, Environment, IntoParameter,
    ResultSetMetadata,
};
use serde_json::Value as Json;

use super::{Rows, bytes_json, decimal_json, json_bytes, plural};
use crate::sources::SourceDef;

/// The ODBC environment, made once for the whole program.
fn environment() -> Result<&'static Environment, String> {
    static ENV: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| Environment::new().map_err(|e| format!("ODBC cannot be set up: {e}")))
        .as_ref()
        .map_err(Clone::clone)
}

/// A value in a connection string: in braces when `;`, a brace or spaces at
/// either end would otherwise change its meaning (a `}` is doubled), as it
/// is otherwise. Some drivers (PostgreSQL's) keep the braces of a user name
/// as part of it, so plain values stay plain.
fn braced(value: &str) -> String {
    let special = value.contains([';', '{', '}']) || value.trim() != value;
    match special {
        true => format!("{{{}}}", value.replace('}', "}}")),
        false => value.to_owned(),
    }
}

/// The connection string: the DSN or the string of the definition, the user
/// name and password from their own fields, and `ReadOnly=1` without *Allow
/// writes*.
pub(super) fn connection_string(def: &SourceDef) -> String {
    let given = def.connection.as_deref().unwrap_or_default().trim();
    let mut text = if given.contains('=') {
        given.trim_end_matches(';').to_owned()
    } else {
        format!("DSN={}", braced(given))
    };
    if let Some(user) = def.user.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        text.push_str(&format!(";UID={}", braced(user)));
    }
    if let Some(password) = def.password.as_deref().filter(|p| !p.is_empty()) {
        text.push_str(&format!(";PWD={}", braced(password)));
    }
    if !def.writes {
        text.push_str(";ReadOnly=1");
    }
    text
}

pub(super) fn open(def: &SourceDef, timeout: Duration) -> Result<Connection<'static>, String> {
    let env = environment()?;
    let text = connection_string(def);
    let seconds = timeout.as_secs_f64().ceil().max(1.0) as u32;
    let name = def.name.clone();
    // The login timeout is the driver's to honour; a thread given up at the
    // deadline covers a driver that does not.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("odbc sign-in".into())
        .spawn(move || {
            let options = ConnectionOptions {
                login_timeout_sec: Some(seconds),
                ..ConnectionOptions::default()
            };
            let _ = sender.send(
                env.connect_with_connection_string(&text, options)
                    .map_err(|e| describe(&e)),
            );
        })
        .map_err(|e| format!("{name}: {e}"))?;
    match receiver.recv_timeout(timeout.max(Duration::from_millis(1))) {
        Ok(Ok(conn)) => Ok(conn),
        Ok(Err(why)) => Err(format!("{}: {}", def.name, why)),
        Err(_) => Err(format!(
            "{}: the data source did not answer within the call timeout",
            def.name
        )),
    }
}

/// The database the driver reaches, by the name it gives.
pub(super) fn about(conn: &Connection<'static>) -> Result<String, String> {
    conn.database_management_system_name()
        .map_err(|e| describe(&e))
}

pub(super) fn query(
    conn: &Connection<'static>,
    def: &SourceDef,
    sql: &str,
    params: &Json,
    rows: &mut Rows,
    timeout: Duration,
) -> Result<(), String> {
    // A transaction that is rolled back, where the driver has transactions;
    // one that has not (Excel, text files) refuses manual commit, and the
    // query runs as it is.
    let manual = conn.set_autocommit(false).is_ok();
    let outcome = read_rows(conn, def, sql, params, rows, timeout);
    if manual {
        let _ = conn.rollback();
        let _ = conn.set_autocommit(true);
    }
    outcome
}

fn read_rows(
    conn: &Connection<'static>,
    def: &SourceDef,
    sql: &str,
    params: &Json,
    rows: &mut Rows,
    timeout: Duration,
) -> Result<(), String> {
    let failed = |e: odbc_api::Error| format!("{}: {}", def.name, describe(&e));
    let mut prepared = conn.prepare(sql).map_err(failed)?;
    let wanted = prepared.num_params().ok().map(usize::from);
    let values = bind(def, params, wanted)?;
    let _ = prepared.set_query_timeout_sec(timeout.as_secs().max(1) as usize);
    let Some(mut cursor) = prepared.execute(&values[..]).map_err(failed)? else {
        return Ok(());
    };
    let count = cursor.num_result_cols().map_err(failed)?.max(0) as u16;
    let mut types = Vec::new();
    for i in 1..=count {
        rows.columns.push(cursor.col_name(i).map_err(failed)?);
        let ty = cursor.col_data_type(i).map_err(failed)?;
        rows.types.push(type_name(ty));
        types.push(ty);
    }
    while let Some(mut row) = cursor.next_row().map_err(failed)? {
        let mut values = Vec::with_capacity(types.len());
        for (i, ty) in types.iter().enumerate() {
            values.push(read(&mut row, i as u16 + 1, *ty).map_err(failed)?);
        }
        rows.push(values)?;
    }
    Ok(())
}

pub(super) fn execute(
    conn: &Connection<'static>,
    def: &SourceDef,
    sql: &str,
    params: &Json,
    timeout: Duration,
) -> Result<u64, String> {
    let failed = |e: odbc_api::Error| format!("{}: {}", def.name, describe(&e));
    let mut prepared = conn.prepare(sql).map_err(failed)?;
    let wanted = prepared.num_params().ok().map(usize::from);
    let values = bind(def, params, wanted)?;
    let _ = prepared.set_query_timeout_sec(timeout.as_secs().max(1) as usize);
    drop(prepared.execute(&values[..]).map_err(failed)?);
    Ok(prepared.row_count().map_err(failed)?.unwrap_or(0) as u64)
}

/// The parameters, a list for `?`. `wanted` is what the driver says the SQL
/// takes; a driver that cannot tell (some file drivers) is given the list as
/// it is.
fn bind(
    def: &SourceDef,
    params: &Json,
    wanted: Option<usize>,
) -> Result<Vec<Box<dyn InputParameter>>, String> {
    let values: &[Json] = match params {
        Json::Null => &[],
        Json::Array(values) => values,
        Json::Object(_) => {
            return Err(format!(
                "{}: ODBC parameters are ? with a list, not :name with a dict",
                def.name
            ));
        }
        _ => {
            return Err(format!("{}: the parameters are a list, for ?", def.name));
        }
    };
    if let Some(wanted) = wanted
        && wanted != values.len()
    {
        return Err(format!(
            "{}: the SQL has {}, and {} given",
            def.name,
            plural(wanted, "parameter", "parameters"),
            match values.len() {
                1 => "1 value was".to_owned(),
                n => format!("{n} values were"),
            }
        ));
    }
    values
        .iter()
        .map(|v| -> Result<Box<dyn InputParameter>, String> {
            Ok(match v {
                Json::Null => Box::new(None::<String>.into_parameter()),
                Json::Bool(b) => Box::new(Bit::from_bool(*b)),
                Json::Number(n) => match n.as_i64() {
                    // As a 32-bit integer when it fits: some drivers (the
                    // Access and text drivers) cannot take a 64-bit one.
                    Some(i) => match i32::try_from(i) {
                        Ok(small) => Box::new(small),
                        Err(_) => Box::new(i),
                    },
                    None => Box::new(n.as_f64().unwrap_or(f64::NAN)),
                },
                Json::String(s) => Box::new(s.clone().into_parameter()),
                other => match json_bytes(other) {
                    Some(bytes) => Box::new(bytes.into_parameter()),
                    None => {
                        return Err(format!("{}: {other} cannot be passed to SQL", def.name));
                    }
                },
            })
        })
        .collect()
}

/// What the prelude needs to know of a column's type.
fn type_name(ty: DataType) -> Option<String> {
    match ty {
        DataType::Date => Some("DATE".to_owned()),
        DataType::Timestamp { .. } => Some("TIMESTAMP".to_owned()),
        _ => None,
    }
}

/// Reads one value by its column's type: numbers as numbers, bytes as bytes,
/// everything else as text (dates as ISO text, which the prelude turns into
/// dates by the column's type).
fn read(row: &mut CursorRow<'_>, col: u16, ty: DataType) -> Result<Json, odbc_api::Error> {
    match ty {
        DataType::Binary { .. } | DataType::Varbinary { .. } | DataType::LongVarbinary { .. } => {
            let mut buf = Vec::new();
            Ok(match row.get_binary(col, &mut buf)? {
                true => bytes_json(&buf),
                false => Json::Null,
            })
        }
        DataType::Integer
        | DataType::SmallInt
        | DataType::TinyInt
        | DataType::BigInt
        | DataType::Bit
        | DataType::Real
        | DataType::Float { .. }
        | DataType::Double
        | DataType::Numeric { .. }
        | DataType::Decimal { .. }
        | DataType::Date
        | DataType::Timestamp { .. } => {
            let mut buf = Vec::new();
            if !row.get_text(col, &mut buf)? {
                return Ok(Json::Null);
            }
            let text = String::from_utf8_lossy(&buf).trim().to_owned();
            Ok(match ty {
                DataType::Bit => Json::Bool(text != "0"),
                DataType::Real | DataType::Float { .. } | DataType::Double => text
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Json::String(text), Json::Number),
                DataType::Date => Json::String(text),
                DataType::Timestamp { .. } => Json::String(without_zero_fraction(&text)),
                _ => decimal_json(&text),
            })
        }
        _ => {
            let mut buf = Vec::new();
            Ok(match row.get_wide_text(col, &mut buf)? {
                true => Json::String(String::from_utf16_lossy(&buf)),
                false => Json::Null,
            })
        }
    }
}

/// `2026-10-02 09:30:00.000` as `2026-10-02 09:30:00`; a fraction that is not
/// zero stays.
fn without_zero_fraction(text: &str) -> String {
    match text.split_once('.') {
        Some((whole, fraction)) if fraction.chars().all(|c| c == '0') => whole.to_owned(),
        _ => text.to_owned(),
    }
}

/// An ODBC error in words: the driver's message, without the diagnostic
/// record's state code around it where it can be taken off.
fn describe(error: &odbc_api::Error) -> String {
    match error {
        odbc_api::Error::Diagnostics { record, .. } => {
            let text = record.to_string();
            // "State: 28000, Native error: 0, Message: [driver] ...".
            match text.split_once("Message: ") {
                Some((_, message)) => message.trim().to_owned(),
                None => text,
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::SourceKind;

    #[test]
    fn the_connection_string_keeps_the_secret_in_braces() {
        let mut def = SourceDef::new("Sales", SourceKind::Odbc);
        def.connection = Some("Sales DSN".into());
        assert_eq!(connection_string(&def), "DSN=Sales DSN;ReadOnly=1");
        def.connection = Some("Driver={PostgreSQL Unicode};Server=db;".into());
        def.user = Some("ro".into());
        def.password = Some("a;b}c".into());
        def.writes = true;
        assert_eq!(
            connection_string(&def),
            "Driver={PostgreSQL Unicode};Server=db;UID=ro;PWD={a;b}}c}"
        );
        assert_eq!(
            without_zero_fraction("2026-10-02 09:30:00.000"),
            "2026-10-02 09:30:00"
        );
        assert_eq!(
            without_zero_fraction("2026-10-02 09:30:00.5"),
            "2026-10-02 09:30:00.5"
        );
    }
}
