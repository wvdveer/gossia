//! The MariaDB side of `database` (the MySQL protocol, so a MySQL server
//! works too).
//!
//! Statements are prepared, so parameters travel apart from the SQL, and
//! results come back typed. `?` takes a list and `:name` a dict.

use gossia_i18n::tr;
use std::collections::HashMap;
use std::time::Duration;

use mysql::consts::{ColumnFlags, ColumnType};
use mysql::prelude::Queryable;
use mysql::{Conn, DriverError, Error, OptsBuilder, Params, SslOpts, Value};
use serde_json::Value as Json;

use super::{Rows, bytes_json, count_mismatch, decimal_json, json_bytes, parameters};
use crate::sources::{SourceDef, TlsMode};

/// The character set number of binary data.
const BINARY: u16 = 63;

pub(super) fn open(def: &SourceDef, timeout: Duration) -> Result<Conn, String> {
    let host = def.host.as_deref().unwrap_or_default().trim();
    let port = def.port.unwrap_or(3306);
    let timeout = timeout.max(Duration::from_millis(1));
    let opts = OptsBuilder::new()
        .ip_or_hostname(Some(host))
        .tcp_port(port)
        .db_name(Some(def.database.as_deref().unwrap_or_default().trim()))
        .user(Some(def.user.as_deref().unwrap_or_default().trim()))
        .pass(def.password.clone())
        .prefer_socket(false)
        .tcp_connect_timeout(Some(timeout))
        // A server that stops answering is given up within the call timeout.
        .read_timeout(Some(timeout))
        .write_timeout(Some(timeout));
    let failed = |e: Error| format!("{}: {host}:{port}: {}", def.name, describe(&e));
    match def.tls {
        TlsMode::Off => Conn::new(opts).map_err(failed),
        TlsMode::Require => Conn::new(opts.ssl_opts(SslOpts::default())).map_err(failed),
        // Any certificate, for any host name.
        TlsMode::Unchecked => Conn::new(
            opts.ssl_opts(
                SslOpts::default()
                    .with_danger_accept_invalid_certs(true)
                    .with_danger_skip_domain_validation(true),
            ),
        )
        .map_err(failed),
        // Encrypted when the server can; again without TLS when it says it
        // cannot. A certificate that cannot be checked is still an error.
        TlsMode::Prefer => match Conn::new(opts.clone().ssl_opts(SslOpts::default())) {
            Err(Error::DriverError(DriverError::TlsNotSupported)) => {
                Conn::new(opts).map_err(failed)
            }
            other => other.map_err(failed),
        },
    }
}

/// The server's version, and whether this connection is encrypted.
pub(super) fn about(conn: &mut Conn) -> Result<(String, bool), String> {
    let version: Option<String> = conn
        .query_first("SELECT VERSION()")
        .map_err(|e| describe(&e))?;
    let cipher: Option<(String, String)> = conn
        .query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
        .unwrap_or(None);
    let encrypted = cipher.is_some_and(|(_, c)| !c.is_empty());
    Ok((version.unwrap_or_default(), encrypted))
}

pub(super) fn query(
    conn: &mut Conn,
    def: &SourceDef,
    sql: &str,
    params: &Json,
    rows: &mut Rows,
) -> Result<(), String> {
    let failed = |e: Error| format!("{}: {}", def.name, describe(&e));
    conn.query_drop("START TRANSACTION READ ONLY")
        .map_err(failed)?;
    let outcome = read_rows(conn, def, sql, params, rows);
    // Nothing a query did is kept.
    let rolled = conn.query_drop("ROLLBACK").map_err(failed);
    outcome.and(rolled)
}

fn read_rows(
    conn: &mut Conn,
    def: &SourceDef,
    sql: &str,
    params: &Json,
    rows: &mut Rows,
) -> Result<(), String> {
    let failed = |e: Error| format!("{}: {}", def.name, describe(&e));
    let stmt = conn.prep(sql).map_err(failed)?;
    let params = bind(def, params, stmt.num_params() as usize)?;
    let mut result = conn.exec_iter(&stmt, params).map_err(failed)?;
    let columns = result.columns();
    let columns: Vec<mysql::Column> = columns.as_ref().to_vec();
    rows.columns = columns.iter().map(|c| c.name_str().into_owned()).collect();
    rows.types = columns.iter().map(type_name).collect();
    for row in result.by_ref() {
        let row = row.map_err(failed)?;
        let values: Vec<Json> = row
            .unwrap()
            .into_iter()
            .zip(&columns)
            .map(|(value, column)| read(value, column))
            .collect();
        rows.push(values)?;
    }
    Ok(())
}

pub(super) fn execute(
    conn: &mut Conn,
    def: &SourceDef,
    sql: &str,
    params: &Json,
) -> Result<u64, String> {
    let failed = |e: Error| format!("{}: {}", def.name, describe(&e));
    let stmt = conn.prep(sql).map_err(failed)?;
    let params = bind(def, params, stmt.num_params() as usize)?;
    conn.exec_drop(&stmt, params).map_err(failed)?;
    Ok(conn.affected_rows())
}

/// The parameters: a list for `?`, a dict for `:name`.
fn bind(def: &SourceDef, params: &Json, wanted: usize) -> Result<Params, String> {
    let value = |v: &Json| -> Result<Value, String> {
        Ok(match v {
            Json::Null => Value::NULL,
            Json::Bool(b) => Value::Int(i64::from(*b)),
            Json::Number(n) => match n.as_i64() {
                Some(i) => Value::Int(i),
                None => match n.as_u64() {
                    Some(u) => Value::UInt(u),
                    None => Value::Double(n.as_f64().unwrap_or(f64::NAN)),
                },
            },
            Json::String(s) => Value::Bytes(s.clone().into_bytes()),
            other => match json_bytes(other) {
                Some(bytes) => Value::Bytes(bytes),
                None => {
                    return Err(tr!(
                        "{name}: {other} cannot be passed to SQL",
                        name = def.name,
                        other = other
                    ));
                }
            },
        })
    };
    let count = |given: usize| count_mismatch(&def.name, wanted, given);
    match params {
        Json::Null if wanted == 0 => Ok(Params::Empty),
        Json::Null => Err(count(0)),
        Json::Array(values) => {
            if values.len() != wanted {
                return Err(count(values.len()));
            }
            if values.is_empty() {
                return Ok(Params::Empty);
            }
            Ok(Params::Positional(
                values.iter().map(value).collect::<Result<_, _>>()?,
            ))
        }
        Json::Object(named) => {
            let mut map = HashMap::new();
            for (name, v) in named {
                let key = name.strip_prefix(':').unwrap_or(name);
                map.insert(key.as_bytes().to_vec(), value(v)?);
            }
            Ok(Params::Named(map))
        }
        _ => Err(tr!(
            "{name}: the parameters are a list (for ?) or a dict (for :name)",
            name = def.name
        )),
    }
}

/// What the prelude needs to know of a column's type: which text is a date,
/// a date and time, or JSON.
fn type_name(column: &mysql::Column) -> Option<String> {
    Some(
        match column.column_type() {
            ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => "DATE",
            ColumnType::MYSQL_TYPE_DATETIME | ColumnType::MYSQL_TYPE_DATETIME2 => "DATETIME",
            ColumnType::MYSQL_TYPE_TIMESTAMP | ColumnType::MYSQL_TYPE_TIMESTAMP2 => "TIMESTAMP",
            ColumnType::MYSQL_TYPE_JSON => "JSON",
            _ => return None,
        }
        .to_owned(),
    )
}

/// Reads one value by its column's type.
fn read(value: Value, column: &mysql::Column) -> Json {
    let ty = column.column_type();
    match value {
        Value::NULL => Json::Null,
        Value::Int(i) => Json::from(i),
        Value::UInt(u) => match i64::try_from(u) {
            Ok(i) => Json::from(i),
            Err(_) => Json::from(u as f64),
        },
        Value::Float(f) => float(f as f64),
        Value::Double(d) => float(d),
        Value::Bytes(bytes) => match ty {
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => {
                decimal_json(&String::from_utf8_lossy(&bytes))
            }
            ColumnType::MYSQL_TYPE_BIT => {
                Json::from(bytes.iter().fold(0u64, |n, b| (n << 8) | *b as u64))
            }
            _ if column.character_set() == BINARY
                && ty != ColumnType::MYSQL_TYPE_JSON
                && (column.flags().contains(ColumnFlags::BINARY_FLAG)
                    || column.flags().contains(ColumnFlags::BLOB_FLAG)) =>
            {
                bytes_json(&bytes)
            }
            _ => Json::String(String::from_utf8_lossy(&bytes).into_owned()),
        },
        Value::Date(y, mo, d, h, mi, s, us) => {
            let date = format!("{y:04}-{mo:02}-{d:02}");
            if matches!(
                ty,
                ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE
            ) {
                Json::String(date)
            } else if us > 0 {
                Json::String(format!("{date} {h:02}:{mi:02}:{s:02}.{us:06}"))
            } else {
                Json::String(format!("{date} {h:02}:{mi:02}:{s:02}"))
            }
        }
        Value::Time(negative, days, h, mi, s, us) => {
            let hours = days * 24 + h as u32;
            let sign = if negative { "-" } else { "" };
            if us > 0 {
                Json::String(format!("{sign}{hours:02}:{mi:02}:{s:02}.{us:06}"))
            } else {
                Json::String(format!("{sign}{hours:02}:{mi:02}:{s:02}"))
            }
        }
    }
}

fn float(f: f64) -> Json {
    serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number)
}

/// A MariaDB error in words.
fn describe(error: &Error) -> String {
    match error {
        Error::MySqlError(e) => {
            let mut text = e.message.clone();
            // 1969: max_statement_time (MariaDB); 3024: max_execution_time
            // (MySQL); the read timeout is an I/O error.
            if matches!(e.code, 1969 | 3024) {
                text.push_str(": the call timeout was reached");
            }
            text
        }
        Error::DriverError(DriverError::TlsNotSupported) => {
            tr!("the server does not offer TLS, and the definition requires it").to_owned()
        }
        Error::DriverError(DriverError::MismatchedStmtParams(wanted, given)) => tr!(
            "the SQL has {parameters}, and {given} were given",
            parameters = parameters(*wanted as usize),
            given = given
        ),
        Error::DriverError(DriverError::MixedParams) => {
            tr!("the SQL mixes ? and :name parameters; use one or the other").to_owned()
        }
        Error::DriverError(e) => e.to_string(),
        Error::IoError(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            tr!("the server did not answer within the call timeout").to_owned()
        }
        Error::IoError(e) => e.to_string(),
        Error::TlsError(e) => tr!("TLS failed: {e}", e = e),
        other => other.to_string(),
    }
}
