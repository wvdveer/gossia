//! What Python reaches the data source definitions through (DESIGN.md, "Data
//! sources"): `source(name)`, `sources.define(...)` and `sources.check(name)`
//! in the prelude call one native function, `_source_access(op, payload)`,
//! with JSON text both ways. The reply is `{"ok": ...}` or `{"error": "..."}`,
//! and the prelude raises `SourceError` for the latter, which a formula shows
//! as `#SOURCE!`.

use serde_json::{Map, Value as Json, json};

use super::shared::Shared;
use crate::sources::{SourceDef, test};

/// Runs one request and gives the reply as JSON text.
pub(super) fn access(shared: &Shared, op: &str, payload: &str) -> String {
    let reply = match serde_json::from_str::<Json>(payload) {
        Ok(payload) => run(shared, op, &payload),
        Err(e) => Err(format!("a request to the data sources did not parse: {e}")),
    };
    match reply {
        Ok(value) => json!({ "ok": value }),
        Err(message) => json!({ "error": message }),
    }
    .to_string()
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
                false if e.contains("already defined") => {
                    format!("{e}: pass replace=True to replace it")
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
                    let outcome = test(&def, shared.source_timeout.get());
                    json!({ "exists": true, "ok": outcome.ok, "message": outcome.message })
                }
            })
        }
        _ => Err(format!("{op} is not a request the data sources know")),
    }
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
    format!("no data source called {name} is defined on this computer")
}

/// `define` and `check` change things or wait, so they are for button
/// procedures and the Immediate window, not for formulas, which run again on
/// their own.
fn side_effects_allowed(shared: &Shared, what: &str) -> Result<(), String> {
    if shared.side_effects.get() {
        Ok(())
    } else {
        Err(format!(
            "{what} can be used only in a button procedure or the Immediate window, not in a formula"
        ))
    }
}
