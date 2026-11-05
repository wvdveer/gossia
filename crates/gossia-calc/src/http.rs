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

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::sources::{HttpAuth, SourceDef, SourceKind, TestOutcome};

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// The method, URL and headers a request goes out with.
pub type Prepared = (String, String, Vec<(String, String)>);

/// What a request goes out with, once the definition's settings are added;
/// or why it cannot be made.
pub fn prepare(def: &SourceDef, request: &HttpRequest) -> Result<Prepared, String> {
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
    Ok((method, url(def, request)?, headers(def, request)))
}

/// A request the host makes for the wasm version (DESIGN.md, Web port,
/// 0.3.0), complete: the worker has the definitions with their secrets and
/// builds it; the host only sends it, without following a redirect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetchRequest {
    /// Which request an answer belongs to; given by whoever asks.
    pub id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub max_bytes: u64,
}

/// What the host answered: a response of any status, or why there is none,
/// in words for the user.
pub type FetchAnswer = Result<HttpResponse, String>;

impl FetchRequest {
    /// The request for the host, with id 0.
    pub fn new(def: &SourceDef, request: &HttpRequest, limits: Limits) -> Result<Self, String> {
        let (method, url, headers) = prepare(def, request)?;
        Ok(FetchRequest {
            id: 0,
            method,
            url,
            headers,
            body: request.body.clone(),
            timeout: limits.timeout,
            max_bytes: limits.max_bytes,
        })
    }

    /// The URL without its query, which may hold an API key: what an error
    /// may show.
    pub fn shown_url(&self) -> &str {
        self.url.split('?').next().unwrap_or(&self.url)
    }

    /// The answer, refused if it is larger than the request allows (the host
    /// should have refused it already).
    pub fn checked(&self, answer: FetchAnswer) -> FetchAnswer {
        let response = answer?;
        if response.body.len() as u64 > self.max_bytes {
            return Err(format!(
                "the response from {} is larger than the maximum response size of {} bytes",
                self.shown_url(),
                self.max_bytes
            ));
        }
        Ok(response)
    }
}

/// The request the Test of an HTTP source makes: a `GET` of its test path,
/// or of the base URL.
pub fn test_request(def: &SourceDef) -> HttpRequest {
    HttpRequest {
        method: "GET".to_owned(),
        path: def.test_path.clone().unwrap_or_default(),
        ..HttpRequest::default()
    }
}

/// The limits of the Test's request.
pub fn test_limits(timeout: Duration) -> Limits {
    Limits {
        timeout,
        max_bytes: 1 << 20,
    }
}

/// What the Test found, from what its request came to and how long it took,
/// when that is known.
pub fn test_outcome(answer: FetchAnswer, ms: Option<u128>) -> TestOutcome {
    let response = match answer {
        Err(why) => return TestOutcome::failed(why),
        Ok(response) => response,
    };
    let status = format!("{} {}", response.status, response.reason);
    let status = status.trim();
    let took = ms.map(|ms| format!(" in {ms} ms")).unwrap_or_default();
    match response.status {
        200..=399 => TestOutcome {
            ok: true,
            message: format!("answered {status}{took}"),
        },
        401 | 403 => TestOutcome::failed(format!("signing in was refused: {status}")),
        _ => TestOutcome {
            ok: true,
            message: format!("reached{took}, but the server answered {status}"),
        },
    }
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

#[cfg(not(target_family = "wasm"))]
#[path = "http_send.rs"]
mod send;
#[cfg(not(target_family = "wasm"))]
pub use send::{send, test};

/// The wasm version has no sending of its own: the worker's engine asks the
/// host for its requests (see `crate::fetches`), and the page asks the host
/// for the Test's. What reaches here was asked outside a host.
#[cfg(target_family = "wasm")]
pub fn send(
    _def: &SourceDef,
    _request: &HttpRequest,
    _limits: Limits,
) -> Result<HttpResponse, String> {
    Err(UNAVAILABLE.to_owned())
}

/// The Test of an HTTP source in the wasm version (see `send`).
#[cfg(target_family = "wasm")]
pub fn test(_def: &SourceDef, _timeout: std::time::Duration) -> crate::sources::TestOutcome {
    crate::sources::TestOutcome::failed(UNAVAILABLE.to_owned())
}

#[cfg(target_family = "wasm")]
const UNAVAILABLE: &str = "HTTP sources are reached through the application's host";

#[cfg(test)]
#[path = "http_tests.rs"]
pub(crate) mod http_tests;
