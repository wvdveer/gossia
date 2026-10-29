//! HTTP/HTTPS data sources (DESIGN.md, "Data sources"): one request to the
//! base URL of a definition, with its authentication and default headers.
//!
//! - **Below the base URL only.** A path is joined to the base URL; a full URL
//!   is refused, so the definition alone decides where a request goes, and its
//!   credentials are never sent anywhere else.
//! - **Redirects are not followed.** A 3xx comes back as it is, so that a
//!   server cannot send the credentials on to another host.
//! - **Certificates** are checked against the operating system's store, so a
//!   company root certificate installed there is trusted as the browser
//!   trusts it.
//! - Every request has the call timeout and the maximum response size.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

use crate::sources::{HttpAuth, SourceDef, SourceKind};

/// The methods a source may be called with. Only `GET` reads.
pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];

/// A request, before the definition's settings are added.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    /// Below the base URL: `orders/12` or `/orders/12`.
    pub path: String,
    pub params: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// What came back.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The limits of one call.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub max_bytes: u64,
}

/// Escapes text for a URL's query or path segment (RFC 3986 unreserved
/// characters are kept).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The URL a request goes to: the base URL of `def`, the path below it, the
/// parameters, and an API key in the query if the definition says so.
pub fn url(def: &SourceDef, request: &HttpRequest) -> Result<String, String> {
    let base = def
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or("the data source has no base URL")?;
    let path = request.path.trim();
    if path.contains("://") || path.starts_with("//") {
        return Err(format!(
            "{path} is a full URL: a data source is called with a path below its base URL, {base}"
        ));
    }
    if path.split(['/', '?', '#']).any(|segment| segment == "..") {
        return Err(format!("{path} goes above the base URL with \"..\""));
    }
    // Without a path, the base URL as it was typed: some servers answer
    // `/api/` and `/api` differently, and a redirect is not followed.
    let path = path.trim_start_matches('/');
    let mut url = match path.is_empty() {
        true => base.to_owned(),
        false => format!("{}/{path}", base.trim_end_matches('/')),
    };
    let mut params: Vec<(String, String)> = request.params.clone();
    if def.auth == HttpAuth::KeyQuery
        && let (Some(name), Some(key)) = (&def.key_name, &def.token)
    {
        params.push((name.clone(), key.clone()));
    }
    if !params.is_empty() {
        let query: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{}={}", escape(k), escape(v)))
            .collect();
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&query.join("&"));
    }
    Ok(url)
}

/// The headers a request carries: the definition's defaults, then its
/// authentication, then the request's own (which win over the defaults but
/// may not replace the authentication).
pub fn headers(def: &SourceDef, request: &HttpRequest) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = def.headers.clone();
    let auth: Option<(String, String)> = match def.auth {
        HttpAuth::None | HttpAuth::KeyQuery => None,
        HttpAuth::Basic => {
            let user = def.user.clone().unwrap_or_default();
            let password = def.password.clone().unwrap_or_default();
            Some((
                "Authorization".to_owned(),
                format!("Basic {}", base64(format!("{user}:{password}").as_bytes())),
            ))
        }
        HttpAuth::Bearer => Some((
            "Authorization".to_owned(),
            format!("Bearer {}", def.token.clone().unwrap_or_default()),
        )),
        HttpAuth::KeyHeader => Some((
            def.key_name.clone().unwrap_or_default(),
            def.token.clone().unwrap_or_default(),
        )),
    };
    for (name, value) in &request.headers {
        let is_auth = auth
            .as_ref()
            .is_some_and(|(a, _)| a.eq_ignore_ascii_case(name));
        if is_auth {
            continue;
        }
        out.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        out.push((name.clone(), value.clone()));
    }
    if let Some((name, value)) = auth {
        out.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
        out.push((name, value));
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn config(def: &SourceDef, limits: Limits) -> ureq::config::Config {
    let mut builder = Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_global(Some(limits.timeout))
        .user_agent(format!("ClioTree/{}", crate::VERSION))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        );
    builder = match def.pool {
        // Each call a connection of its own.
        None => builder.max_idle_connections(0),
        Some(n) => {
            let n = usize::try_from(n).unwrap_or(usize::MAX);
            builder
                .max_idle_connections(n)
                .max_idle_connections_per_host(n)
        }
    };
    builder.build()
}

/// The agent for a definition: a new one each call without pooling, and one
/// kept per definition (as it is now, with its timeout) with pooling, so
/// that its connections are reused. Editing the definition makes a new one.
fn agent(def: &SourceDef, limits: Limits) -> Agent {
    if def.pool.is_none() {
        return Agent::new_with_config(config(def, limits));
    }
    static AGENTS: OnceLock<Mutex<HashMap<String, Agent>>> = OnceLock::new();
    let key = format!(
        "{}|{}|{:?}",
        serde_json::to_string(def).unwrap_or_default(),
        limits.timeout.as_millis(),
        limits.max_bytes
    );
    let mut agents = AGENTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // A definition that changed leaves its old agent behind: keep the map
    // from growing without end.
    if agents.len() > 64 {
        agents.clear();
    }
    agents
        .entry(key)
        .or_insert_with(|| Agent::new_with_config(config(def, limits)))
        .clone()
}

/// Sends one request. A response of any status is `Ok`; `Err` is a request
/// that could not be made or answered, in words for the user.
pub fn send(
    def: &SourceDef,
    request: &HttpRequest,
    limits: Limits,
) -> Result<HttpResponse, String> {
    if def.kind != SourceKind::Http {
        return Err(format!(
            "{} is a {} source, not an HTTP one",
            def.name,
            def.kind.label()
        ));
    }
    let method = request.method.to_ascii_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return Err(format!(
            "{method} is not a method a data source can be called with"
        ));
    }
    let url = url(def, request)?;
    let mut builder = ureq::http::Request::builder()
        .method(method.as_str())
        .uri(url.as_str());
    for (name, value) in headers(def, request) {
        builder = builder.header(name, value);
    }
    let body = request.body.clone().unwrap_or_default();
    let http = builder
        .body(body)
        .map_err(|e| format!("the request to {url} cannot be made: {e}"))?;
    let _waiting = crate::sources::Waiting::begin();
    let mut response = agent(def, limits)
        .run(http)
        .map_err(|e| describe(&e, &url, limits))?;
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = response
        .body_mut()
        .with_config()
        .limit(limits.max_bytes)
        .read_to_vec()
        .map_err(|e| describe(&e, &url, limits))?;
    Ok(HttpResponse {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or("").to_owned(),
        headers,
        body,
    })
}

/// A failed request in words, with the URL (whose query may hold an API key,
/// so the query is left out).
fn describe(error: &ureq::Error, url: &str, limits: Limits) -> String {
    let shown = url.split('?').next().unwrap_or(url);
    match error {
        ureq::Error::Timeout(_) => format!(
            "{shown} did not answer within {} s",
            limits.timeout.as_secs_f32()
        ),
        ureq::Error::HostNotFound => format!("the host of {shown} cannot be found"),
        ureq::Error::ConnectionFailed => format!("{shown} cannot be reached"),
        ureq::Error::BodyExceedsLimit(limit) => format!(
            "the response from {shown} is larger than the maximum response size of {limit} bytes"
        ),
        other => format!("{shown}: {other}"),
    }
}

/// The Test of an HTTP source: a `GET` of its test path, or of the base URL.
pub fn test(def: &SourceDef, timeout: Duration) -> crate::sources::TestOutcome {
    use crate::sources::TestOutcome;
    let request = HttpRequest {
        method: "GET".to_owned(),
        path: def.test_path.clone().unwrap_or_default(),
        ..HttpRequest::default()
    };
    let started = Instant::now();
    let limits = Limits {
        timeout,
        max_bytes: 1 << 20,
    };
    match send(def, &request, limits) {
        Err(why) => TestOutcome {
            ok: false,
            message: why,
        },
        Ok(response) => {
            let ms = started.elapsed().as_millis();
            let status = format!("{} {}", response.status, response.reason);
            match response.status {
                200..=399 => TestOutcome {
                    ok: true,
                    message: format!("answered {} in {ms} ms", status.trim()),
                },
                401 | 403 => TestOutcome {
                    ok: false,
                    message: format!("signing in was refused: {}", status.trim()),
                },
                _ => TestOutcome {
                    ok: true,
                    message: format!(
                        "reached in {ms} ms, but the server answered {}",
                        status.trim()
                    ),
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "http_tests.rs"]
pub(crate) mod http_tests;
