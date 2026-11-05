//! Sending a request with `ureq`, and the Test of an HTTP source, on the
//! desktop. The wasm version has none of this: its requests are made by the
//! host (DESIGN.md, Web port, 0.3.0).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

use super::{HttpRequest, HttpResponse, Limits, METHODS, headers, url};
use crate::sources::{SourceDef, SourceKind};

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
