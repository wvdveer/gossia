//! What Python reaches the data source definitions through (DESIGN.md, "Data
//! sources"): `source(name)` with its HTTP methods, `query` and `execute`,
//! `sources.define(...)` and `sources.check(name)` in the prelude call one
//! native function, `_source_access(op, payload)`,
//! with JSON text both ways. The reply is `{"ok": ...}` or `{"error": "..."}`,
//! and the prelude raises `SourceError` for the latter, which a formula shows
//! as `#SOURCE!`.

use gossia_core::Path;
use gossia_i18n::tr;
use serde_json::{Map, Value as Json, json};

use super::shared::Shared;
use crate::engine::Depth;
use crate::http::{self, HttpRequest, Limits};
use crate::sources::{SOURCE_SHEET, SourceDef, SourceKind, test};
use crate::{database, journal, sqlite};

/// Runs one request and gives the reply as JSON text.
pub(super) fn access(shared: &Shared, op: &str, payload: &str) -> String {
    let reply = match serde_json::from_str::<Json>(payload) {
        Ok(payload) => {
            note_read(shared, op, &payload);
            cached(shared, op, &payload)
        }
        Err(e) => Err(tr!(
            "a request to the data sources did not parse: {e}",
            e = e
        )),
    };
    if shared.waiting.get() {
        // The prelude stops what runs; it runs again with the answer.
        return json!({ "wait": true }).to_string();
    }
    match reply {
        Ok(value) => json!({ "ok": value }),
        Err(message) => json!({ "error": message }),
    }
    .to_string()
}

/// A formula that reads a journal depends on it: when one of its files
/// changes, `Calc::source_changed` runs the formula again.
fn note_read(shared: &Shared, op: &str, payload: &Json) {
    if op != "journal" || payload["method"].as_str() == Some("add") {
        return;
    }
    if let (Some(ctx), Some(name)) = (shared.ctx(), payload["name"].as_str()) {
        ctx.record(SOURCE_SHEET, &Path::root().key(name), Depth::Deep);
    }
}

/// Runs a request, from the pass's cache when it is a read made by a formula
/// or a control's expression: a `query`, or an HTTP `GET`. Procedures and the
/// Immediate window always read afresh, and leave the cache as it is.
fn cached(shared: &Shared, op: &str, payload: &Json) -> Result<Json, String> {
    let read = match op {
        "query" => true,
        "journal" => payload["method"].as_str() != Some("add"),
        "http" => payload["method"]
            .as_str()
            .is_none_or(|m| m.eq_ignore_ascii_case("GET")),
        _ => false,
    };
    if !read || shared.side_effects.get() {
        return run(shared, op, payload);
    }
    let key = format!("{op} {payload}");
    if let Some(reply) = shared.source_reads.borrow().get(&key) {
        return reply.clone();
    }
    let reply = run(shared, op, payload);
    if !shared.waiting.get() {
        shared.source_reads.borrow_mut().insert(key, reply.clone());
    }
    reply
}

fn run(shared: &Shared, op: &str, payload: &Json) -> Result<Json, String> {
    let store = shared.sources.borrow().clone();
    match op {
        "info" => {
            let name = payload.as_str().unwrap_or_default();
            match store.get(name)? {
                Some(def) => Ok(info(&def)),
                None => Err(missing(name)),
            }
        }
        "define" => {
            side_effects_allowed(shared, "sources.define")?;
            let name = payload["name"].as_str().unwrap_or_default();
            let kind = payload["kind"].as_str().unwrap_or_default();
            let empty = Map::new();
            let settings = payload["settings"].as_object().unwrap_or(&empty);
            let replace = payload["replace"].as_bool().unwrap_or(false);
            let def = SourceDef::from_settings(name, kind, settings)?;
            store.save(def, None, replace).map_err(|e| match replace {
                false if e == crate::sources::already_defined(name) => {
                    tr!("{e}: pass replace=True to replace it", e = e)
                }
                _ => e,
            })?;
            Ok(Json::Null)
        }
        "check" => {
            side_effects_allowed(shared, "sources.check")?;
            let name = payload.as_str().unwrap_or_default();
            Ok(match store.get(name)? {
                None => json!({ "exists": false, "ok": false, "message": missing(name) }),
                Some(def) => {
                    let hosted = shared.fetches.borrow().is_some();
                    let outcome =
                        match hosted && def.kind == SourceKind::Http && def.problems().is_empty() {
                            true => {
                                let limits = http::test_limits(shared.source_timeout.get());
                                let answer = send(shared, &def, &http::test_request(&def), limits);
                                http::test_outcome(answer, None)
                            }
                            false => test(&def, shared.source_timeout.get()),
                        };
                    json!({ "exists": true, "ok": outcome.ok, "message": outcome.message })
                }
            })
        }
        "http" => call_http(shared, payload),
        "query" | "execute" => call_sql(shared, op, payload),
        "journal" => call_journal(shared, payload),
        "xml" => read_xml(payload["text"].as_str().unwrap_or_default()),
        _ => Err(tr!("{op} is not a request the data sources know", op = op)),
    }
}

/// `source(name).get(...)` and the other methods.
fn call_http(shared: &Shared, payload: &Json) -> Result<Json, String> {
    let store = shared.sources.borrow().clone();
    let name = payload["name"].as_str().unwrap_or_default();
    let def = store.get(name)?.ok_or_else(|| missing(name))?;
    if def.kind == SourceKind::Beancount {
        return Err(tr!(
            "{name} is a Beancount journal: it is read with transactions(), balance(), \
             accounts() and prices(), not with HTTP methods",
            name = name
        ));
    }
    if def.kind != SourceKind::Http {
        return Err(tr!(
            "{name} is a {kind} source: it is read with query(), not with HTTP methods",
            name = name,
            kind = def.kind.label()
        ));
    }
    let method = payload["method"]
        .as_str()
        .unwrap_or("GET")
        .to_ascii_uppercase();
    if method != "GET" {
        if !def.writes {
            return Err(tr!(
                "{name} does not allow writes, so {method} is refused: tick Allow writes in its \
                 definition (Tools > Data sources...)",
                name = name,
                method = method
            ));
        }
        if !shared.side_effects.get() {
            return Err(tr!(
                "{method} changes data, so it can be used only in a button procedure or the \
                 Immediate window, not in a formula",
                method = method
            ));
        }
    }
    let pairs = |value: &Json| -> Vec<(String, String)> {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|pair| {
                        Some((
                            pair.get(0)?.as_str()?.to_owned(),
                            pair.get(1)?.as_str()?.to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut headers = pairs(&payload["headers"]);
    let form = pairs(&payload["form"]);
    let body = match (&payload["body"], form.is_empty()) {
        (Json::String(text), _) => Some(text.clone().into_bytes()),
        (_, false) => {
            if !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            {
                headers.push((
                    "Content-Type".to_owned(),
                    "application/x-www-form-urlencoded".to_owned(),
                ));
            }
            let encoded: Vec<String> = form
                .iter()
                .map(|(k, v)| format!("{}={}", http::escape(k), http::escape(v)))
                .collect();
            Some(encoded.join("&").into_bytes())
        }
        _ => None,
    };
    let path = payload["path"].as_str().unwrap_or_default().to_owned();
    let request = HttpRequest {
        method: method.clone(),
        path: path.clone(),
        params: pairs(&payload["params"]),
        headers,
        body,
    };
    let limits = Limits {
        timeout: shared.source_timeout.get(),
        max_bytes: shared.max_response.get(),
    };
    let response = send(shared, &def, &request, limits).map_err(|e| format!("{name}: {e}"))?;
    let text = String::from_utf8_lossy(&response.body).into_owned();
    if response.status >= 400 {
        let mut excerpt: String = text.chars().take(200).collect();
        if excerpt.len() < text.len() {
            excerpt.push_str("...");
        }
        let excerpt = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
        let excerpt = match excerpt.is_empty() {
            true => String::new(),
            false => format!(": {excerpt}"),
        };
        return Err(format!(
            "{name}: {method} {path} answered {} {}{excerpt}",
            response.status, response.reason
        ));
    }
    let headers: Map<String, Json> = response
        .headers
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), Json::String(v)))
        .collect();
    Ok(json!({
        "status": response.status,
        "reason": response.reason,
        "headers": headers,
        "text": text,
        "body": gossia_core::binary::to_base64(&response.body),
    }))
}

/// `response.xml()`: the body read as an XML file is imported, with the type
/// inferred from it, as JSON text. Elements are keys, attributes `@name`,
/// and the text of an element that has attributes too is `#text`.
fn read_xml(text: &str) -> Result<Json, String> {
    use gossia_core::{infer, xml, xmltree, xsd};
    let doc =
        xmltree::parse(text).map_err(|e| tr!("the body is not well-formed XML: {e}", e = e))?;
    let inference = infer::infer_xsd(&doc)?;
    let spec = xsd::lower_str(&inference.text).map_err(|e| e.to_string())?;
    let parsed = xml::parse(text, &spec).map_err(|e| e.to_string())?;
    let json = gossia_core::json::to_string(&parsed.node, false).map_err(|e| e.to_string())?;
    Ok(Json::String(json))
}

/// `source(name).query(sql, params)` and `.execute(sql, params)`.
fn call_sql(shared: &Shared, op: &str, payload: &Json) -> Result<Json, String> {
    let store = shared.sources.borrow().clone();
    let name = payload["name"].as_str().unwrap_or_default();
    let def = store.get(name)?.ok_or_else(|| missing(name))?;
    if def.kind == SourceKind::Http {
        return Err(tr!(
            "{name} is an HTTP source: it is read with get(), not with {op}()",
            name = name,
            op = op
        ));
    }
    if def.kind == SourceKind::Beancount {
        return Err(tr!(
            "{name} is a Beancount journal: it is read with transactions(), balance(), \
             accounts() and prices(), not with {op}()",
            name = name,
            op = op
        ));
    }
    let server = def.kind.is_database();
    let sql = payload["sql"].as_str().unwrap_or_default();
    let params = &payload["params"];
    let limits = Limits {
        timeout: shared.source_timeout.get(),
        max_bytes: shared.max_response.get(),
    };
    if op == "query" {
        // A formula always reads through a read-only connection, even from a
        // source that allows writes.
        let writable = def.writes && shared.side_effects.get();
        return unwatched(shared, || match server {
            // Always a read-only transaction, rolled back afterwards.
            true => database::query(&def, sql, params, limits),
            false => sqlite::query(&def, sql, params, limits, writable),
        });
    }
    if !def.writes {
        return Err(tr!(
            "{name} does not allow writes, so execute() is refused: tick Allow writes in its \
             definition (Tools > Data sources...)",
            name = name
        ));
    }
    if !shared.side_effects.get() {
        return Err(tr!(
            "execute() changes data, so it can be used only in a button procedure or \
             the Immediate window, not in a formula"
        )
        .to_owned());
    }
    let changed = unwatched(shared, || match server {
        true => database::execute(&def, sql, params, limits),
        false => sqlite::execute(&def, sql, params, limits),
    })?;
    Ok(Json::from(changed))
}

/// `source(name).transactions(...)` and the other methods of a journal, and
/// `.add(...)`, which appends a transaction.
fn call_journal(shared: &Shared, payload: &Json) -> Result<Json, String> {
    let store = shared.sources.borrow().clone();
    let name = payload["name"].as_str().unwrap_or_default();
    let def = store.get(name)?.ok_or_else(|| missing(name))?;
    if def.kind != SourceKind::Beancount {
        let how = match def.kind {
            SourceKind::Http => "get()",
            _ => "query()",
        };
        return Err(tr!(
            "{name} is a {kind} source, not a Beancount journal: it is read with {how}",
            name = name,
            kind = def.kind.label(),
            how = how
        ));
    }
    let method = payload["method"].as_str().unwrap_or_default();
    let args = &payload["args"];
    if method != "add" {
        return unwatched(shared, || journal::read_call(&def, method, args));
    }
    if !def.writes {
        return Err(tr!(
            "{name} does not allow writes, so add() is refused: tick Allow writes in its \
             definition (Tools > Data sources...)",
            name = name
        ));
    }
    if !shared.side_effects.get() {
        return Err(tr!(
            "add() changes the journal, so it can be used only in a button procedure or \
             the Immediate window, not in a formula"
        )
        .to_owned());
    }
    unwatched(shared, || journal::append(&def, args))
}

/// Runs a call to a data source with the formula's watchdog paused: waiting
/// for a source does not count against the formula's time limit.
/// Sends a request: on the desktop at once, and in the wasm version's worker
/// through the host, with the answer it has given or else asking for it
/// (which stops what runs, see `crate::fetches`).
fn send(
    shared: &Shared,
    def: &SourceDef,
    request: &HttpRequest,
    limits: Limits,
) -> Result<http::HttpResponse, String> {
    let mut fetches = shared.fetches.borrow_mut();
    let Some(fetches) = fetches.as_mut() else {
        return unwatched(shared, || http::send(def, request, limits));
    };
    let fetch = http::FetchRequest::new(def, request, limits)?;
    match fetches.answer(fetch, shared.side_effects.get()) {
        Some(answer) => answer,
        None => {
            shared.waiting.set(true);
            Err(tr!("waiting for the host's answer").to_owned())
        }
    }
}

fn unwatched<T>(shared: &Shared, call: impl FnOnce() -> T) -> T {
    let watchdog = shared.watchdog.borrow().clone();
    let left = watchdog.as_ref().and_then(|w| w.pause());
    let result = call();
    if let Some(w) = &watchdog {
        w.resume(left);
    }
    result
}

/// What `source(name)` shows of a definition: never its secrets.
fn info(def: &SourceDef) -> Json {
    json!({
        "name": def.name,
        "kind": def.kind.code(),
        "writes": def.writes,
        "target": def.target(),
    })
}

fn missing(name: &str) -> String {
    tr!(
        "no data source called {name} is defined on this computer",
        name = name
    )
}

/// `define` and `check` change things or wait, so they are for button
/// procedures and the Immediate window, not for formulas, which run again on
/// their own.
fn side_effects_allowed(shared: &Shared, what: &str) -> Result<(), String> {
    if shared.side_effects.get() {
        Ok(())
    } else {
        Err(tr!(
            "{what} can be used only in a button procedure or the Immediate window, not in a formula",
            what = what
        ))
    }
}
