//! PostgreSQL, MariaDB and (on Windows) ODBC data sources (DESIGN.md, "Data
//! sources", step 6b parts 4 and 5): reading with `query`, changing data with
//! `execute`, the Test, and connection pooling.
//!
//! A `query` always runs in a read-only transaction that is rolled back
//! afterwards, whatever the definition allows, so a formula never changes
//! data. `execute` commits its statement on its own (autocommit). The caller
//! has checked *Allow writes* and where `execute` runs.
//!
//! What both kinds give back has the shape SQLite's does (see `sqlite`):
//! column names, a type name per column that tells the prelude which text is
//! a date, a time or JSON, and the rows as lists of JSON values, with bytes as
//! `{"$bytes": "<hex>"}`.

use gossia_i18n::tr;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Map, Value as Json};

use crate::http::Limits;
use crate::sources::{SourceDef, SourceKind, TestOutcome, TlsMode};

#[path = "database_mariadb.rs"]
mod mariadb;
#[cfg(windows)]
#[path = "database_odbc.rs"]
mod odbc;
#[path = "database_postgres.rs"]
mod postgres;

/// An open connection to either kind of server.
pub(crate) enum Conn {
    Postgres(Box<::postgres::Client>),
    Mariadb(::mysql::Conn),
    #[cfg(windows)]
    Odbc(::odbc_api::Connection<'static>),
}

/// The rows of a query while they are gathered, with the size limit.
pub(crate) struct Rows {
    pub columns: Vec<String>,
    pub types: Vec<Option<String>>,
    pub rows: Vec<Json>,
    size: u64,
    max: u64,
    name: String,
}

impl Rows {
    fn new(def: &SourceDef, limits: Limits) -> Rows {
        Rows {
            columns: Vec::new(),
            types: Vec::new(),
            rows: Vec::new(),
            size: 0,
            max: limits.max_bytes,
            name: def.name.clone(),
        }
    }

    /// Adds a row, or says that the rows are now too large.
    pub fn push(&mut self, values: Vec<Json>) -> Result<(), String> {
        for value in &values {
            self.size += match value {
                Json::String(s) => s.len() as u64,
                Json::Object(o) => o
                    .get("$bytes")
                    .and_then(Json::as_str)
                    .map_or(8, |h| h.len() as u64 / 2),
                _ => 8,
            };
        }
        if self.size > self.max {
            return Err(tr!(
                "{name}: the rows are larger than the maximum response size of {max} bytes",
                name = self.name,
                max = self.max
            ));
        }
        self.rows.push(Json::Array(values));
        Ok(())
    }

    fn into_json(self) -> Json {
        let mut o = Map::new();
        o.insert(
            "columns".to_owned(),
            Json::Array(self.columns.into_iter().map(Json::String).collect()),
        );
        o.insert(
            "types".to_owned(),
            Json::Array(
                self.types
                    .into_iter()
                    .map(|t| t.map_or(Json::Null, Json::String))
                    .collect(),
            ),
        );
        o.insert("rows".to_owned(), Json::Array(self.rows));
        Json::Object(o)
    }
}

/// The rows of a statement that reads, in a read-only transaction.
pub fn query(def: &SourceDef, sql: &str, params: &Json, limits: Limits) -> Result<Json, String> {
    let _waiting = crate::sources::Waiting::begin();
    let mut lease = take(def, limits.timeout)?;
    let mut rows = Rows::new(def, limits);
    let outcome = match lease.conn_mut() {
        Conn::Postgres(client) => postgres::query(client, def, sql, params, &mut rows),
        Conn::Mariadb(conn) => mariadb::query(conn, def, sql, params, &mut rows),
        #[cfg(windows)]
        Conn::Odbc(conn) => odbc::query(conn, def, sql, params, &mut rows, limits.timeout),
    };
    lease.check(&outcome);
    outcome.map(|()| rows.into_json())
}

/// Runs a statement that changes data and gives the number of rows changed.
pub fn execute(def: &SourceDef, sql: &str, params: &Json, limits: Limits) -> Result<u64, String> {
    let _waiting = crate::sources::Waiting::begin();
    let mut lease = take(def, limits.timeout)?;
    let outcome = match lease.conn_mut() {
        Conn::Postgres(client) => postgres::execute(client, def, sql, params),
        Conn::Mariadb(conn) => mariadb::execute(conn, def, sql, params),
        #[cfg(windows)]
        Conn::Odbc(conn) => odbc::execute(conn, def, sql, params, limits.timeout),
    };
    lease.check(&outcome);
    outcome
}

/// The Test: a single throwaway connection (never one from a pool) that
/// signs in and asks for the server's version and whether the connection is
/// encrypted.
pub fn test(def: &SourceDef, timeout: Duration) -> TestOutcome {
    let start = Instant::now();
    #[cfg(windows)]
    if def.kind == SourceKind::Odbc {
        return match open(def, timeout).and_then(|conn| match conn {
            Conn::Odbc(conn) => odbc::about(&conn),
            _ => Err(tr!("not an ODBC connection").to_owned()),
        }) {
            Ok(dbms) => TestOutcome {
                ok: true,
                message: format!(
                    "{dbms} through ODBC, {}{}, in {} ms",
                    def.target(),
                    if def.writes {
                        ""
                    } else {
                        ", read-only asked of the driver"
                    },
                    start.elapsed().as_millis()
                ),
            },
            Err(why) => TestOutcome::failed(why),
        };
    }
    let found = open(def, timeout).and_then(|conn| match conn {
        Conn::Postgres(mut client) => postgres::about(&mut client),
        Conn::Mariadb(mut conn) => mariadb::about(&mut conn),
        #[cfg(windows)]
        Conn::Odbc(_) => Err(tr!("not a database server").to_owned()),
    });
    match found {
        Ok((version, encrypted)) => {
            let host = def.host.as_deref().unwrap_or_default().trim();
            let port = def.port.or(def.kind.default_port()).unwrap_or(0);
            let db = def.database.as_deref().unwrap_or_default().trim();
            let user = def.user.as_deref().unwrap_or_default().trim();
            // A MariaDB source may be a MySQL server: say which it is. MariaDB
            // puts its name in the version ("10.11.6-MariaDB-0+deb12u1").
            let server = if def.kind == SourceKind::Mariadb && !version.contains("MariaDB") {
                "MySQL"
            } else {
                def.kind.label()
            };
            TestOutcome {
                ok: true,
                message: format!(
                    "{server} {version} at {host}:{port}/{db} as {user}, {}, in {} ms",
                    if encrypted && def.tls == TlsMode::Unchecked {
                        tr!("encrypted (certificate not checked)")
                    } else if encrypted {
                        "encrypted"
                    } else {
                        tr!("not encrypted")
                    },
                    start.elapsed().as_millis()
                ),
            }
        }
        Err(why) => TestOutcome::failed(why),
    }
}

/// Opens one connection, signed in.
fn open(def: &SourceDef, timeout: Duration) -> Result<Conn, String> {
    if def.kind == SourceKind::Postgres {
        postgres::open(def, timeout).map(|c| Conn::Postgres(Box::new(c)))
    } else if def.kind == SourceKind::Mariadb {
        mariadb::open(def, timeout).map(Conn::Mariadb)
    } else if def.kind == SourceKind::Odbc {
        open_odbc(def, timeout)
    } else {
        Err(tr!(
            "{name} is a {kind} source, not a database server",
            name = def.name,
            kind = def.kind.label()
        ))
    }
}

#[cfg(windows)]
fn open_odbc(def: &SourceDef, timeout: Duration) -> Result<Conn, String> {
    odbc::open(def, timeout).map(Conn::Odbc)
}

/// ODBC needs the driver manager that comes with Windows.
#[cfg(not(windows))]
fn open_odbc(def: &SourceDef, _timeout: Duration) -> Result<Conn, String> {
    Err(tr!(
        "{name}: ODBC data sources are available on Windows only",
        name = def.name
    ))
}

/// The connections of one definition that allows pooling: at most `max`
/// open at once, the idle ones kept for the next call.
struct Pool {
    max: usize,
    state: Mutex<PoolState>,
    returned: Condvar,
}

struct PoolState {
    idle: Vec<Conn>,
    /// Open connections, idle and in use.
    open: usize,
}

/// A connection for one call: from a pool, given back when the call is over,
/// or a fresh one that is closed when the call is over.
struct Lease {
    conn: Option<Conn>,
    pool: Option<Arc<Pool>>,
    /// The connection failed in a way that may have left it unusable.
    broken: bool,
}

impl Lease {
    fn conn_mut(&mut self) -> &mut Conn {
        self.conn
            .as_mut()
            .expect("a lease holds its connection until dropped")
    }

    /// After a failed call, a connection that is closed is not given back.
    fn check<T>(&mut self, outcome: &Result<T, String>) {
        if outcome.is_err() {
            self.broken = match &self.conn {
                Some(Conn::Postgres(client)) => client.is_closed(),
                // The MySQL client does not say; a reset on return tells.
                Some(Conn::Mariadb(_)) => false,
                #[cfg(windows)]
                Some(Conn::Odbc(conn)) => conn.is_dead().unwrap_or(true),
                None => true,
            };
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(pool) = self.pool.take() else {
            return;
        };
        let mut conn = self.conn.take();
        // A connection goes back reset: session state discarded, so the next
        // call starts as on a fresh one.
        let reusable = !self.broken
            && match conn.as_mut() {
                Some(Conn::Postgres(client)) => client.batch_execute("DISCARD ALL").is_ok(),
                Some(Conn::Mariadb(c)) => c.reset().is_ok(),
                #[cfg(windows)]
                Some(Conn::Odbc(c)) => {
                    let _ = c.rollback();
                    c.set_autocommit(true).is_ok() && !c.is_dead().unwrap_or(true)
                }
                None => false,
            };
        let mut state = pool.state.lock().unwrap_or_else(|e| e.into_inner());
        match (reusable, conn) {
            (true, Some(conn)) => state.idle.push(conn),
            _ => state.open = state.open.saturating_sub(1),
        }
        drop(state);
        pool.returned.notify_one();
    }
}

/// A connection for one call, from the definition's pool when it has one.
fn take(def: &SourceDef, timeout: Duration) -> Result<Lease, String> {
    let Some(max) = def.pool else {
        return Ok(Lease {
            conn: Some(open(def, timeout)?),
            pool: None,
            broken: false,
        });
    };
    let pool = pool_of(def, max as usize);
    let deadline = Instant::now() + timeout;
    let mut state = pool.state.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if let Some(conn) = state.idle.pop() {
            return Ok(Lease {
                conn: Some(conn),
                pool: Some(pool.clone()),
                broken: false,
            });
        }
        if state.open < pool.max {
            state.open += 1;
            drop(state);
            return match open(def, deadline.saturating_duration_since(Instant::now())) {
                Ok(conn) => Ok(Lease {
                    conn: Some(conn),
                    pool: Some(pool),
                    broken: false,
                }),
                Err(why) => {
                    let mut state = pool.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.open = state.open.saturating_sub(1);
                    drop(state);
                    pool.returned.notify_one();
                    Err(why)
                }
            };
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(tr!(
                "{name}: all {max} connections of its pool stayed in use for {timeout} s",
                name = def.name,
                max = pool.max,
                timeout = timeout.as_secs_f32()
            ));
        }
        state = pool
            .returned
            .wait_timeout(state, left)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

/// The pools by definition name, each with the definition it was made for.
type Pools = HashMap<String, (String, Arc<Pool>)>;

/// The pool of a definition as it is now. An edited definition (another
/// host, password or pool size) gets a new pool; the old one's connections
/// close when they are dropped.
fn pool_of(def: &SourceDef, max: usize) -> Arc<Pool> {
    static POOLS: OnceLock<Mutex<Pools>> = OnceLock::new();
    let key = serde_json::to_string(def).unwrap_or_default();
    let mut pools = POOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((k, pool)) = pools.get(&def.name)
        && *k == key
    {
        return pool.clone();
    }
    let pool = Arc::new(Pool {
        max: max.max(1),
        state: Mutex::new(PoolState {
            idle: Vec::new(),
            open: 0,
        }),
        returned: Condvar::new(),
    });
    pools.insert(def.name.clone(), (key, pool.clone()));
    pool
}

/// Bytes as the prelude takes them.
pub(crate) fn bytes_json(bytes: &[u8]) -> Json {
    let mut o = Map::new();
    o.insert(
        "$bytes".to_owned(),
        Json::String(bytes.iter().map(|b| format!("{b:02x}")).collect()),
    );
    Json::Object(o)
}

/// Bytes the prelude sent as `{"$bytes": "<hex>"}`.
pub(crate) fn json_bytes(value: &Json) -> Option<Vec<u8>> {
    let o = value.as_object()?;
    if o.len() != 1 {
        return None;
    }
    let hex = o.get("$bytes")?.as_str()?;
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// A number in decimal text (`NUMERIC`, `DECIMAL`) as JSON: an integer when
/// it has no fraction and fits, otherwise a float.
pub(crate) fn decimal_json(text: &str) -> Json {
    let text = text.trim();
    if !text.contains(['.', 'e', 'E'])
        && let Ok(i) = text.parse::<i64>()
    {
        return Json::from(i);
    }
    text.parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or_else(|| Json::String(text.to_owned()), Json::Number)
}

/// "1 parameter" or "3 parameters".
pub(crate) fn parameters(n: usize) -> String {
    gossia_i18n::counted(n, tr!("1 parameter"), tr!("{n} parameters"))
}

/// Why the values given do not fit the parameters of the SQL.
pub(crate) fn count_mismatch(source: &str, wanted: usize, given: usize) -> String {
    let given = match given {
        0 => tr!("no values were given").to_owned(),
        1 => tr!("1 value was given").to_owned(),
        n => tr!("{n} values were given", n = n),
    };
    tr!(
        "{source}: the SQL has {parameters}, and {given}",
        source = source,
        parameters = parameters(wanted),
        given = given
    )
}

#[cfg(test)]
#[path = "database_tests.rs"]
pub(crate) mod database_tests;
