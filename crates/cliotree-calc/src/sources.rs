//! Data source definitions (DESIGN.md, "Data sources"): named connections that
//! belong to the user and the machine, not to the workbook. A workbook refers
//! to a source by name only (`source("Sales")` in Python); the definition on
//! this computer says where that name goes, with its credentials.
//!
//! The definitions are kept in one JSON file in the per-user configuration
//! directory, beside `trust.json`, passwords included. Another window may
//! change the file, so every read goes to the file and every change reads it
//! again first. What a newer version wrote and this one does not know is kept.

use std::cell::RefCell;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

/// The name of the file in the configuration directory.
pub const SOURCES_FILE: &str = "data-sources.json";
/// The version of the file that this build writes.
pub const SOURCES_VERSION: u64 = 1;

/// What a source connects to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Http,
    Sqlite,
    Postgres,
    Mariadb,
}

impl SourceKind {
    pub const ALL: [SourceKind; 4] = [
        SourceKind::Http,
        SourceKind::Sqlite,
        SourceKind::Postgres,
        SourceKind::Mariadb,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Http => "HTTP / HTTPS",
            SourceKind::Sqlite => "SQLite",
            SourceKind::Postgres => "PostgreSQL",
            SourceKind::Mariadb => "MariaDB",
        }
    }

    /// The name Python uses: `sources.define("Sales", "postgres", ...)`.
    pub fn code(self) -> &'static str {
        match self {
            SourceKind::Http => "http",
            SourceKind::Sqlite => "sqlite",
            SourceKind::Postgres => "postgres",
            SourceKind::Mariadb => "mariadb",
        }
    }

    pub fn from_code(code: &str) -> Option<SourceKind> {
        SourceKind::ALL.into_iter().find(|k| k.code() == code)
    }

    /// The port a database server listens on unless it is told otherwise.
    pub fn default_port(self) -> Option<u16> {
        match self {
            SourceKind::Postgres => Some(5432),
            SourceKind::Mariadb => Some(3306),
            _ => None,
        }
    }

    pub fn is_database_server(self) -> bool {
        matches!(self, SourceKind::Postgres | SourceKind::Mariadb)
    }
}

/// How an HTTP source signs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HttpAuth {
    #[default]
    None,
    /// A user name and a password.
    Basic,
    /// `Authorization: Bearer <token>`.
    Bearer,
    /// The key in a header named `key_name`.
    KeyHeader,
    /// The key in a query parameter named `key_name`.
    KeyQuery,
}

impl HttpAuth {
    pub const ALL: [HttpAuth; 5] = [
        HttpAuth::None,
        HttpAuth::Basic,
        HttpAuth::Bearer,
        HttpAuth::KeyHeader,
        HttpAuth::KeyQuery,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HttpAuth::None => "None",
            HttpAuth::Basic => "User name and password",
            HttpAuth::Bearer => "Bearer token",
            HttpAuth::KeyHeader => "API key in a header",
            HttpAuth::KeyQuery => "API key in the query",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            HttpAuth::None => "none",
            HttpAuth::Basic => "basic",
            HttpAuth::Bearer => "bearer",
            HttpAuth::KeyHeader => "key-header",
            HttpAuth::KeyQuery => "key-query",
        }
    }

    /// Whether the secret is a token or key (rather than a password).
    pub fn uses_token(self) -> bool {
        matches!(
            self,
            HttpAuth::Bearer | HttpAuth::KeyHeader | HttpAuth::KeyQuery
        )
    }
}

/// One definition. Which fields mean something depends on the kind; the
/// others are left empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceDef {
    pub name: String,
    pub kind: SourceKind,
    /// Database servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// Database servers, and HTTP with basic authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// HTTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "is_no_auth")]
    pub auth: HttpAuth,
    /// The bearer token or the API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The header or query parameter that carries an API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    /// What the Test fetches, below the base URL. Absent: the base URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_path: Option<String>,
    /// SQLite: the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Pooling: the most connections kept at once. Absent: no pooling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<u32>,
    /// Whether the source may be written to. Off: SQLite is opened read-only,
    /// database sessions are read-only, and HTTP may only `GET`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub writes: bool,
    /// What a newer version wrote that this one does not know.
    #[serde(flatten)]
    pub extra: Map<String, Json>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_no_auth(a: &HttpAuth) -> bool {
    *a == HttpAuth::None
}

/// The settings `sources.define` takes, besides the name and the kind.
pub const SETTINGS: &[&str] = &[
    "host",
    "port",
    "database",
    "user",
    "password",
    "url",
    "headers",
    "auth",
    "token",
    "key_name",
    "test_path",
    "path",
    "pool",
    "writes",
];

fn filled(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl SourceDef {
    pub fn new(name: &str, kind: SourceKind) -> SourceDef {
        SourceDef {
            name: name.to_owned(),
            kind,
            host: None,
            port: None,
            database: None,
            user: None,
            password: None,
            url: None,
            headers: Vec::new(),
            auth: HttpAuth::None,
            token: None,
            key_name: None,
            test_path: None,
            path: None,
            pool: None,
            writes: false,
            extra: Map::new(),
        }
    }

    /// What is wrong with the definition, in the order of the form. Empty
    /// when it can be saved.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(why) = name_problem(&self.name) {
            problems.push(why);
        }
        match self.kind {
            SourceKind::Http => match filled(&self.url) {
                None => problems.push("an HTTP source needs a base URL".to_owned()),
                Some(url) => {
                    if http_host_port(url).is_none() {
                        problems.push(format!(
                            "{url} is not a URL that starts with http:// or https://"
                        ));
                    }
                }
            },
            SourceKind::Sqlite => {
                if filled(&self.path).is_none() {
                    problems.push("a SQLite source needs the path of its file".to_owned());
                }
                if self.pool.is_some() {
                    problems.push("a SQLite source cannot pool connections".to_owned());
                }
            }
            SourceKind::Postgres | SourceKind::Mariadb => {
                let kind = self.kind.label();
                if filled(&self.host).is_none() {
                    problems.push(format!("a {kind} source needs a host"));
                }
                if filled(&self.database).is_none() {
                    problems.push(format!("a {kind} source needs a database"));
                }
                if filled(&self.user).is_none() {
                    problems.push(format!("a {kind} source needs a user name"));
                }
                if self.port == Some(0) {
                    problems.push("the port must be from 1 to 65535".to_owned());
                }
            }
        }
        if self.kind == SourceKind::Http {
            match self.auth {
                HttpAuth::Basic if filled(&self.user).is_none() => {
                    problems.push("basic authentication needs a user name".to_owned());
                }
                HttpAuth::KeyHeader | HttpAuth::KeyQuery if filled(&self.key_name).is_none() => {
                    problems.push(
                        "an API key needs the name of the header or parameter that carries it"
                            .to_owned(),
                    );
                }
                _ => {}
            }
        }
        if self.pool == Some(0) {
            problems.push("pooling needs at least 1 connection".to_owned());
        }
        problems
    }

    /// Where the source goes, in full, so that the user sees where a secret
    /// would be sent: `postgres://db.example.com:5432/sales as reporting_ro`.
    pub fn target(&self) -> String {
        let user = filled(&self.user);
        match self.kind {
            SourceKind::Http => {
                let url = filled(&self.url).unwrap_or("(no URL)");
                match (self.auth, user) {
                    (HttpAuth::Basic, Some(user)) => format!("{url} as {user}"),
                    _ => url.to_owned(),
                }
            }
            SourceKind::Sqlite => format!("sqlite:{}", filled(&self.path).unwrap_or("(no file)")),
            SourceKind::Postgres | SourceKind::Mariadb => {
                let scheme = self.kind.code();
                let host = filled(&self.host).unwrap_or("(no host)");
                let port = self.port.or(self.kind.default_port()).unwrap_or(0);
                let db = filled(&self.database).unwrap_or("");
                match user {
                    Some(user) => format!("{scheme}://{host}:{port}/{db} as {user}"),
                    None => format!("{scheme}://{host}:{port}/{db}"),
                }
            }
        }
    }

    /// The definition without its secrets, for what Python and the panel's
    /// list may show.
    pub fn without_secrets(&self) -> SourceDef {
        SourceDef {
            password: None,
            token: None,
            ..self.clone()
        }
    }

    /// Whether a password or token is stored.
    pub fn has_secret(&self) -> bool {
        filled(&self.password).is_some() || filled(&self.token).is_some()
    }

    /// Makes a definition from the settings `sources.define` was given, as a
    /// JSON object. An unknown setting is refused, so that a misspelt one does
    /// not go unnoticed.
    pub fn from_settings(
        name: &str,
        kind: &str,
        settings: &Map<String, Json>,
    ) -> Result<SourceDef, String> {
        let kind = SourceKind::from_code(kind).ok_or_else(|| {
            format!(
                "{kind:?} is not a kind of data source: use {}",
                SourceKind::ALL
                    .map(|k| format!("{:?}", k.code()))
                    .join(", ")
            )
        })?;
        for key in settings.keys() {
            if !SETTINGS.contains(&key.as_str()) {
                return Err(format!(
                    "{key} is not a setting of a data source: the settings are {}",
                    SETTINGS.join(", ")
                ));
            }
        }
        let mut object: Map<String, Json> = settings
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Headers as a dict, the way Python writes them.
        if let Some(Json::Object(headers)) = object.get("headers") {
            let pairs: Vec<Json> = headers
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        Json::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    Json::Array(vec![Json::String(k.clone()), Json::String(v)])
                })
                .collect();
            object.insert("headers".to_owned(), Json::Array(pairs));
        }
        object.insert("name".to_owned(), Json::String(name.to_owned()));
        object.insert("kind".to_owned(), Json::String(kind.code().to_owned()));
        let def: SourceDef = serde_json::from_value(Json::Object(object))
            .map_err(|e| format!("the settings of the data source do not fit: {e}"))?;
        match def.problems().first() {
            Some(why) => Err(why.clone()),
            None => Ok(def),
        }
    }
}

/// Why a source cannot be called `name`, or `None` if it can: letters, digits
/// and underscores, not starting with a digit, as a Python name would be.
pub fn name_problem(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Some("a data source needs a name".to_owned());
    };
    if !(first == '_' || first.is_alphabetic()) || !chars.all(|c| c == '_' || c.is_alphanumeric()) {
        return Some(format!(
            "{name} is not a valid data source name: use letters, digits and underscores, \
             and do not start with a digit"
        ));
    }
    if name.chars().count() > 64 {
        return Some("a data source name has at most 64 characters".to_owned());
    }
    None
}

/// The host and port of an `http://` or `https://` URL.
pub fn http_host_port(url: &str) -> Option<(String, u16)> {
    let (rest, default) = match url.strip_prefix("https://") {
        Some(rest) => (rest, 443),
        None => (url.strip_prefix("http://")?, 80),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    // [::1]:8080
    if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None => default,
        };
        return Some((host.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Some((host.to_owned(), port.parse().ok()?)),
        Some(_) => None,
        None => Some((authority.to_owned(), default)),
    }
}

/// The definitions of this user on this machine: in a file, or in memory for
/// the tests and when there is no configuration directory. Cloning shares
/// them, so the evaluation thread and the window see the same definitions.
#[derive(Debug, Clone)]
pub struct SourceStore {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug)]
struct Inner {
    file: Option<PathBuf>,
    /// The definitions when there is no file.
    memory: Vec<SourceDef>,
}

impl Default for SourceStore {
    fn default() -> Self {
        SourceStore::in_memory()
    }
}

impl PartialEq for SourceStore {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// The file as it was read: the definitions, and the rest of the object.
struct Document {
    sources: Vec<SourceDef>,
    rest: Map<String, Json>,
}

impl SourceStore {
    pub fn in_memory() -> SourceStore {
        SourceStore {
            inner: Arc::new(Mutex::new(Inner {
                file: None,
                memory: Vec::new(),
            })),
        }
    }

    /// The definitions in `file`, which need not exist yet.
    pub fn at(file: impl Into<PathBuf>) -> SourceStore {
        SourceStore {
            inner: Arc::new(Mutex::new(Inner {
                file: Some(file.into()),
                memory: Vec::new(),
            })),
        }
    }

    /// The file the definitions are kept in, if any.
    pub fn file(&self) -> Option<PathBuf> {
        self.lock().file.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while the lock was held leaves the definitions as they were
        // read or written last, which is still usable.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every definition, sorted by name.
    pub fn list(&self) -> Result<Vec<SourceDef>, String> {
        let inner = self.lock();
        let mut sources = match &inner.file {
            None => inner.memory.clone(),
            Some(_) => read(&inner)?.sources,
        };
        sources.sort_by_key(|d| d.name.to_lowercase());
        Ok(sources)
    }

    pub fn get(&self, name: &str) -> Result<Option<SourceDef>, String> {
        Ok(self.list()?.into_iter().find(|d| d.name == name))
    }

    /// Saves `def`. `old` is the name it had, when an existing definition is
    /// edited (and perhaps renamed); `None` adds a new one. A name that is
    /// already taken by another definition is refused unless `replace`.
    pub fn save(&self, def: SourceDef, old: Option<&str>, replace: bool) -> Result<(), String> {
        if let Some(why) = def.problems().first() {
            return Err(why.clone());
        }
        self.change(|sources| {
            let clash = sources
                .iter()
                .position(|d| d.name == def.name && Some(d.name.as_str()) != old);
            match clash {
                Some(i) if replace => {
                    sources.remove(i);
                }
                Some(_) => {
                    return Err(format!(
                        "a data source called {} is already defined on this computer",
                        def.name
                    ));
                }
                None => {}
            }
            match old.and_then(|old| sources.iter().position(|d| d.name == old)) {
                Some(i) => sources[i] = def.clone(),
                None => sources.push(def.clone()),
            }
            Ok(())
        })
    }

    pub fn remove(&self, name: &str) -> Result<(), String> {
        self.change(|sources| {
            sources.retain(|d| d.name != name);
            Ok(())
        })
    }

    fn change(
        &self,
        f: impl FnOnce(&mut Vec<SourceDef>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut inner = self.lock();
        match inner.file.clone() {
            None => f(&mut inner.memory),
            Some(file) => {
                let mut doc = read(&inner)?;
                f(&mut doc.sources)?;
                write(&file, doc)
            }
        }
    }
}

fn read(inner: &Inner) -> Result<Document, String> {
    let empty = || Document {
        sources: Vec::new(),
        rest: Map::new(),
    };
    let Some(file) = &inner.file else {
        return Ok(empty());
    };
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty()),
        Err(e) => return Err(format!("{} cannot be read: {e}", file.display())),
    };
    // A file that cannot be read is left alone, never overwritten: it holds
    // passwords the user may still want.
    let unreadable = |why: String| {
        format!(
            "{} cannot be read ({why}); fix or remove it to change the data sources",
            file.display()
        )
    };
    let mut object = match serde_json::from_str::<Json>(&text) {
        Ok(Json::Object(object)) => object,
        Ok(_) => return Err(unreadable("it is not a JSON object".to_owned())),
        Err(e) => return Err(unreadable(e.to_string())),
    };
    let sources = match object.remove("sources") {
        None => Vec::new(),
        Some(list) => serde_json::from_value(list).map_err(|e| unreadable(e.to_string()))?,
    };
    object.remove("version");
    Ok(Document {
        sources,
        rest: object,
    })
}

fn write(file: &std::path::Path, doc: Document) -> Result<(), String> {
    let mut object = Map::new();
    object.insert("version".to_owned(), Json::from(SOURCES_VERSION));
    object.insert(
        "sources".to_owned(),
        serde_json::to_value(&doc.sources).map_err(|e| e.to_string())?,
    );
    object.extend(doc.rest);
    let text = serde_json::to_string_pretty(&Json::Object(object)).map_err(|e| e.to_string())?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("{} cannot be made: {e}", dir.display()))?;
    }
    // Written beside the file and moved over it, so that a crash leaves the
    // old file or the new one, never half of one.
    let temp = file.with_extension("json.saving");
    std::fs::write(&temp, text)
        .and_then(|()| std::fs::rename(&temp, file))
        .map_err(|e| format!("{} cannot be written: {e}", file.display()))
}

/// What the Test found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    pub ok: bool,
    pub message: String,
}

impl TestOutcome {
    fn failed(message: impl Into<String>) -> TestOutcome {
        TestOutcome {
            ok: false,
            message: message.into(),
        }
    }
}

/// Checks that the source can be reached: for SQLite that its file can be
/// opened for reading, for the others that its server answers on its port.
/// It does not run Python, so it works in an untrusted workbook.
///
/// Signing in, and the query the full Test runs, come with the support for
/// each kind (DESIGN.md, build order, 6b).
pub fn test(def: &SourceDef, timeout: Duration) -> TestOutcome {
    if let Some(why) = def.problems().first() {
        return TestOutcome::failed(why.clone());
    }
    let start = Instant::now();
    match def.kind {
        SourceKind::Sqlite => {
            let path = filled(&def.path).unwrap_or_default();
            match std::fs::File::open(path) {
                Ok(file) if file.metadata().is_ok_and(|m| m.is_file()) => TestOutcome {
                    ok: true,
                    message: format!("{path} can be opened for reading"),
                },
                Ok(_) => TestOutcome::failed(format!("{path} is not a file")),
                Err(e) => TestOutcome::failed(format!("{path} cannot be opened: {e}")),
            }
        }
        SourceKind::Http => crate::http::test(def, timeout),
        SourceKind::Postgres | SourceKind::Mariadb => {
            let host = filled(&def.host).unwrap_or_default().to_owned();
            let port = def.port.or(def.kind.default_port()).unwrap_or(0);
            reach(&host, port, timeout, start)
        }
    }
}

/// Time spent waiting for a data source, which does not count against the
/// time limit of the formula or procedure that waits (DESIGN.md, "Data
/// sources", "Limits"). The evaluation thread gives each of its threads one,
/// and reads it when it decides whether a formula is stuck.
#[derive(Debug, Default)]
pub struct IoClock {
    /// When the wait now going on began.
    since: Mutex<Option<Instant>>,
    /// The waits of the current formula that are over.
    spent: Mutex<Duration>,
}

impl IoClock {
    /// A new formula: nothing waited yet.
    pub fn reset(&self) {
        *self.since.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.spent.lock().unwrap_or_else(|e| e.into_inner()) = Duration::ZERO;
    }

    /// The time waited so far by the current formula, the wait going on
    /// included.
    pub fn waited(&self) -> Duration {
        let spent = *self.spent.lock().unwrap_or_else(|e| e.into_inner());
        let now = self
            .since
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|s| s.elapsed())
            .unwrap_or_default();
        spent + now
    }
}

thread_local! {
    static IO_CLOCK: RefCell<Option<Arc<IoClock>>> = const { RefCell::new(None) };
}

/// Gives this thread the clock its waits are counted on.
pub fn set_io_clock(clock: Arc<IoClock>) {
    IO_CLOCK.with(|c| *c.borrow_mut() = Some(clock));
}

/// A wait for a data source, while it lasts.
pub struct Waiting {
    started: Instant,
}

impl Waiting {
    pub fn begin() -> Waiting {
        let started = Instant::now();
        IO_CLOCK.with(|c| {
            if let Some(clock) = c.borrow().as_ref() {
                *clock.since.lock().unwrap_or_else(|e| e.into_inner()) = Some(started);
            }
        });
        Waiting { started }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let took = self.started.elapsed();
        IO_CLOCK.with(|c| {
            if let Some(clock) = c.borrow().as_ref() {
                *clock.since.lock().unwrap_or_else(|e| e.into_inner()) = None;
                *clock.spent.lock().unwrap_or_else(|e| e.into_inner()) += took;
            }
        });
    }
}

/// Opens a TCP connection and closes it again.
fn reach(host: &str, port: u16, timeout: Duration, start: Instant) -> TestOutcome {
    let addrs = match (host, port).to_socket_addrs() {
        Ok(addrs) => addrs.collect::<Vec<_>>(),
        Err(e) => return TestOutcome::failed(format!("{host} cannot be found: {e}")),
    };
    let mut last = None;
    for addr in addrs {
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&addr, left) {
            Ok(_) => {
                return TestOutcome {
                    ok: true,
                    message: format!(
                        "{host}:{port} answered in {} ms",
                        start.elapsed().as_millis()
                    ),
                };
            }
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) if e.kind() == std::io::ErrorKind::TimedOut => TestOutcome::failed(format!(
            "{host}:{port} did not answer within {} s",
            timeout.as_secs_f32()
        )),
        Some(e) => TestOutcome::failed(format!("{host}:{port} cannot be reached: {e}")),
        None => TestOutcome::failed(format!("{host}:{port} cannot be reached")),
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod sources_tests;
