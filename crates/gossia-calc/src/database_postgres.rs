//! The PostgreSQL side of `database`.
//!
//! Parameters are sent as text, which the server converts to the type the
//! statement needs, as it does for a value typed into `psql`: a date as
//! `2026-10-29`, a number as `12.5`, bytes as `\x...`. Results come back in
//! PostgreSQL's binary form and are read here by their type.

use std::error::Error as StdError;
use std::time::Duration;

use ::postgres::config::SslMode;
use ::postgres::fallible_iterator::FallibleIterator;
use ::postgres::types::{FromSql, IsNull, Kind, ToSql, Type};
use ::postgres::{Client, Config};
use bytes::BytesMut;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use serde_json::Value as Json;

use super::{Rows, bytes_json, decimal_json, json_bytes, plural};
use crate::sources::{SourceDef, TlsMode};

pub(super) fn open(def: &SourceDef, timeout: Duration) -> Result<Client, String> {
    let host = def.host.as_deref().unwrap_or_default().trim();
    let port = def.port.unwrap_or(5432);
    let mut config = Config::new();
    config
        .host(host)
        .port(port)
        .dbname(def.database.as_deref().unwrap_or_default().trim())
        .user(def.user.as_deref().unwrap_or_default().trim())
        .application_name("Gossia")
        .connect_timeout(timeout.max(Duration::from_millis(1)))
        // The server stops a statement that runs past the call timeout,
        // waits for locks included.
        .options(&format!(
            "-c statement_timeout={}",
            timeout.as_millis().max(1)
        ))
        .ssl_mode(match def.tls {
            TlsMode::Prefer => SslMode::Prefer,
            TlsMode::Require | TlsMode::Unchecked => SslMode::Require,
            TlsMode::Off => SslMode::Disable,
        });
    if let Some(password) = def.password.as_deref() {
        config.password(password);
    }
    // The operating system's TLS, which checks the certificate against its
    // own certificate store.
    // Unchecked accepts any certificate, for any host name.
    let unchecked = def.tls == TlsMode::Unchecked;
    let connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(unchecked)
        .danger_accept_invalid_hostnames(unchecked)
        .build()
        .map_err(|e| format!("{}: TLS cannot be set up: {e}", def.name))?;
    let tls = postgres_native_tls::MakeTlsConnector::new(connector);
    // The driver's connect timeout covers only the TCP connect: a server that
    // accepts and then says nothing would hold the sign-in forever. So it
    // runs on its own thread, given up at the deadline; that thread ends when
    // the server finally answers or closes.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("postgres sign-in".into())
        .spawn(move || {
            let _ = sender.send(config.connect(tls));
        })
        .map_err(|e| format!("{}: {e}", def.name))?;
    match receiver.recv_timeout(timeout.max(Duration::from_millis(1))) {
        Ok(Ok(client)) => Ok(client),
        Ok(Err(e)) => Err(format!("{}: {host}:{port}: {}", def.name, describe(&e))),
        Err(_) => Err(format!(
            "{}: {host}:{port}: the server did not answer within the call timeout",
            def.name
        )),
    }
}

/// The server's version, and whether this connection is encrypted.
pub(super) fn about(client: &mut Client) -> Result<(String, bool), String> {
    let version: String = client
        .query_one("SHOW server_version", &[])
        .and_then(|row| row.try_get(0))
        .map_err(|e| describe(&e))?;
    // pg_stat_ssl shows every session its own row.
    let encrypted = client
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .unwrap_or(false);
    Ok((version, encrypted))
}

pub(super) fn query(
    client: &mut Client,
    def: &SourceDef,
    sql: &str,
    params: &Json,
    rows: &mut Rows,
) -> Result<(), String> {
    let failed = |e: ::postgres::Error| format!("{}: {}", def.name, describe(&e));
    let mut tx = client
        .build_transaction()
        .read_only(true)
        .start()
        .map_err(failed)?;
    let stmt = tx.prepare(sql).map_err(failed)?;
    let values = bind(def, params, stmt.params().len())?;
    let columns = stmt.columns();
    rows.columns = columns.iter().map(|c| c.name().to_owned()).collect();
    rows.types = columns.iter().map(|c| type_name(c.type_())).collect();
    let mut found = tx.query_raw(&stmt, values.iter()).map_err(failed)?;
    while let Some(row) = found.next().map_err(failed)? {
        let mut values = Vec::with_capacity(columns.len());
        for (i, column) in columns.iter().enumerate() {
            let raw: Raw = row.try_get(i).map_err(failed)?;
            values.push(read(def, column.name(), column.type_(), raw.0.as_deref())?);
        }
        rows.push(values)?;
    }
    drop(found);
    // Nothing a query did is kept.
    tx.rollback().map_err(failed)
}

pub(super) fn execute(
    client: &mut Client,
    def: &SourceDef,
    sql: &str,
    params: &Json,
) -> Result<u64, String> {
    let failed = |e: ::postgres::Error| format!("{}: {}", def.name, describe(&e));
    let stmt = client.prepare(sql).map_err(failed)?;
    let values = bind(def, params, stmt.params().len())?;
    let refs: Vec<&(dyn ToSql + Sync)> = values.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
    client.execute(&stmt, &refs).map_err(failed)
}

/// The parameters, for `$1`, `$2`...: a list, as many as the SQL has.
fn bind(def: &SourceDef, params: &Json, wanted: usize) -> Result<Vec<Text>, String> {
    let values = match params {
        Json::Null => Vec::new(),
        Json::Array(values) => values.clone(),
        Json::Object(_) => {
            return Err(format!(
                "{}: PostgreSQL takes the parameters as a list, for $1, $2 and so on, \
                 not as a dict",
                def.name
            ));
        }
        _ => {
            return Err(format!(
                "{}: the parameters are a list, for $1, $2 and so on",
                def.name
            ));
        }
    };
    if values.len() != wanted {
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
    values.iter().map(|v| Text::of(def, v)).collect()
}

/// A parameter as text, in the form the server reads for any type; `None`
/// is SQL `NULL`.
#[derive(Debug)]
struct Text(Option<String>);

impl Text {
    fn of(def: &SourceDef, value: &Json) -> Result<Text, String> {
        Ok(Text(match value {
            Json::Null => None,
            Json::Bool(b) => Some(b.to_string()),
            Json::Number(n) => Some(n.to_string()),
            Json::String(s) => Some(s.clone()),
            other => match json_bytes(other) {
                Some(bytes) => Some(format!(
                    "\\x{}",
                    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
                )),
                None => {
                    return Err(format!("{}: {other} cannot be passed to SQL", def.name));
                }
            },
        }))
    }
}

impl ToSql for Text {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn StdError + Sync + Send>> {
        match &self.0 {
            None => Ok(IsNull::Yes),
            Some(text) => {
                out.extend_from_slice(text.as_bytes());
                Ok(IsNull::No)
            }
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn StdError + Sync + Send>> {
        self.to_sql(ty, out)
    }

    fn encode_format(&self, _ty: &Type) -> ::postgres::types::Format {
        ::postgres::types::Format::Text
    }
}

/// A result value as the server sent it, of any type.
struct Raw(Option<Vec<u8>>);

impl<'a> FromSql<'a> for Raw {
    fn from_sql(_ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn StdError + Sync + Send>> {
        Ok(Raw(Some(raw.to_vec())))
    }

    fn from_sql_null(_ty: &Type) -> Result<Self, Box<dyn StdError + Sync + Send>> {
        Ok(Raw(None))
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

/// What the prelude needs to know of a column's type: which text is a date,
/// a date and time, or JSON.
fn type_name(ty: &Type) -> Option<String> {
    let ty = base(ty);
    Some(
        match *ty {
            Type::DATE => "DATE",
            Type::TIMESTAMP => "TIMESTAMP",
            Type::TIMESTAMPTZ => "TIMESTAMPTZ",
            Type::JSON | Type::JSONB => "JSON",
            _ => return None,
        }
        .to_owned(),
    )
}

/// A domain is read as the type it is based on.
fn base(ty: &Type) -> &Type {
    match ty.kind() {
        Kind::Domain(inner) => base(inner),
        _ => ty,
    }
}

fn epoch() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2000, 1, 1)
        .unwrap_or_default()
        .and_time(NaiveTime::MIN)
}

/// Reads one value by its type.
fn read(def: &SourceDef, column: &str, ty: &Type, raw: Option<&[u8]>) -> Result<Json, String> {
    let Some(raw) = raw else {
        return Ok(Json::Null);
    };
    let bad = || {
        format!(
            "{}: the value of column {column} did not arrive whole",
            def.name
        )
    };
    let be = |n: usize| -> Result<[u8; 8], String> {
        let mut b = [0u8; 8];
        if raw.len() != n {
            return Err(bad());
        }
        b[8 - n..].copy_from_slice(raw);
        Ok(b)
    };
    let ty = base(ty);
    let text = || Json::String(String::from_utf8_lossy(raw).into_owned());
    Ok(match *ty {
        Type::BOOL => Json::Bool(raw.first().is_some_and(|b| *b != 0)),
        Type::INT2 => Json::from(i16::from_be_bytes(raw.try_into().map_err(|_| bad())?)),
        Type::INT4 => Json::from(i32::from_be_bytes(raw.try_into().map_err(|_| bad())?)),
        Type::INT8 => Json::from(i64::from_be_bytes(be(8)?)),
        Type::OID => Json::from(u32::from_be_bytes(raw.try_into().map_err(|_| bad())?)),
        Type::FLOAT4 => float(f32::from_be_bytes(raw.try_into().map_err(|_| bad())?) as f64),
        Type::FLOAT8 => float(f64::from_be_bytes(be(8)?)),
        Type::NUMERIC => decimal_json(&numeric(raw).ok_or_else(bad)?),
        Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::UNKNOWN
        | Type::CHAR
        | Type::XML => text(),
        Type::BYTEA => bytes_json(raw),
        Type::JSON => text(),
        // JSONB starts with its version, 1.
        Type::JSONB => {
            Json::String(String::from_utf8_lossy(raw.get(1..).unwrap_or_default()).into_owned())
        }
        Type::DATE => {
            let days = i32::from_be_bytes(raw.try_into().map_err(|_| bad())?);
            match days {
                i32::MAX => Json::String("infinity".to_owned()),
                i32::MIN => Json::String("-infinity".to_owned()),
                _ => Json::String(
                    (epoch().date() + TimeDelta::days(days as i64))
                        .format("%Y-%m-%d")
                        .to_string(),
                ),
            }
        }
        Type::TIMESTAMP | Type::TIMESTAMPTZ => {
            let micros = i64::from_be_bytes(be(8)?);
            let zone = if *ty == Type::TIMESTAMPTZ {
                "+00:00"
            } else {
                ""
            };
            match micros {
                i64::MAX => Json::String("infinity".to_owned()),
                i64::MIN => Json::String("-infinity".to_owned()),
                _ => {
                    let at = epoch() + TimeDelta::microseconds(micros);
                    Json::String(format!("{}{zone}", at.format("%Y-%m-%d %H:%M:%S%.f")))
                }
            }
        }
        Type::TIME => {
            let micros = i64::from_be_bytes(be(8)?);
            let at = NaiveTime::MIN + TimeDelta::microseconds(micros);
            Json::String(at.format("%H:%M:%S%.f").to_string())
        }
        Type::UUID => {
            let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
            if hex.len() != 32 {
                return Err(bad());
            }
            Json::String(format!(
                "{}-{}-{}-{}-{}",
                &hex[0..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            ))
        }
        _ if matches!(ty.kind(), Kind::Enum(_)) || ty.name() == "citext" => text(),
        _ => {
            return Err(format!(
                "{}: column {column} is of type {}, which cannot be read yet: cast it in the \
                 SQL, for example {column}::text",
                def.name,
                ty.name()
            ));
        }
    })
}

fn float(f: f64) -> Json {
    serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number)
}

/// A `NUMERIC` in PostgreSQL's binary form, as decimal text: the number of
/// digits, the weight of the first, the sign and the scale, then the digits
/// in base 10000.
fn numeric(raw: &[u8]) -> Option<String> {
    let word =
        |i: usize| -> Option<u16> { Some(u16::from_be_bytes([*raw.get(i)?, *raw.get(i + 1)?])) };
    let ndigits = word(0)? as usize;
    let weight = word(2)? as i16 as i32;
    let sign = word(4)?;
    let dscale = word(6)? as usize;
    match sign {
        0xC000 => return Some("NaN".to_owned()),
        0xD000 => return Some("inf".to_owned()),
        0xF000 => return Some("-inf".to_owned()),
        _ => {}
    }
    let digits: Vec<u16> = (0..ndigits)
        .map(|i| word(8 + 2 * i))
        .collect::<Option<_>>()?;
    let mut int = String::new();
    for k in 0..=weight.max(-1) {
        let d = digits.get(k as usize).copied().unwrap_or(0);
        if int.is_empty() {
            if d != 0 {
                int = d.to_string();
            }
        } else {
            int.push_str(&format!("{d:04}"));
        }
    }
    if int.is_empty() {
        int.push('0');
    }
    let mut frac = String::new();
    let mut k = weight + 1;
    while frac.len() < dscale {
        let d = if k < 0 {
            0
        } else {
            digits.get(k as usize).copied().unwrap_or(0)
        };
        frac.push_str(&format!("{d:04}"));
        k += 1;
    }
    frac.truncate(dscale);
    let sign = if sign == 0x4000 { "-" } else { "" };
    Some(match frac.is_empty() {
        true => format!("{sign}{int}"),
        false => format!("{sign}{int}.{frac}"),
    })
}

/// A PostgreSQL error in words: the server's message, with its detail and
/// hint, or the reason the connection failed.
fn describe(error: &::postgres::Error) -> String {
    if let Some(db) = error.as_db_error() {
        let mut text = db.message().to_owned();
        if let Some(detail) = db.detail() {
            text.push_str(": ");
            text.push_str(detail);
        }
        if let Some(hint) = db.hint() {
            text.push_str(" (");
            text.push_str(hint);
            text.push(')');
        }
        if db.code() == &::postgres::error::SqlState::QUERY_CANCELED {
            text.push_str(": the call timeout was reached");
        }
        return text;
    }
    // The outer error is often only "error connecting to server"; the cause
    // says why (refused, timed out, a certificate that cannot be checked).
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::numeric;

    fn encode(ndigits: u16, weight: i16, sign: u16, dscale: u16, digits: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        for w in [ndigits, weight as u16, sign, dscale] {
            out.extend_from_slice(&w.to_be_bytes());
        }
        for d in digits {
            out.extend_from_slice(&d.to_be_bytes());
        }
        out
    }

    #[test]
    fn numerics_read_as_decimal_text() {
        // 12345.678 = 1|2345|6780, weight 1, scale 3.
        assert_eq!(
            numeric(&encode(3, 1, 0, 3, &[1, 2345, 6780])).as_deref(),
            Some("12345.678")
        );
        // -0.05: 500 at weight -1, scale 2.
        assert_eq!(
            numeric(&encode(1, -1, 0x4000, 2, &[500])).as_deref(),
            Some("-0.05")
        );
        // 0.00001234: 1234 at weight -2 (0.0000|1234), scale 8.
        assert_eq!(
            numeric(&encode(1, -2, 0, 8, &[1234])).as_deref(),
            Some("0.00001234")
        );
        // 1000000 = 100|0000 with the trailing group left out, scale 0.
        assert_eq!(
            numeric(&encode(1, 1, 0, 0, &[100])).as_deref(),
            Some("1000000")
        );
        assert_eq!(numeric(&encode(0, 0, 0, 2, &[])).as_deref(), Some("0.00"));
        assert_eq!(
            numeric(&encode(0, 0, 0xC000, 0, &[])).as_deref(),
            Some("NaN")
        );
    }
}
