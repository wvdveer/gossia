//! The commands of the test control protocol (DESIGN.md, "Testing and CI",
//! level 8 and "The test-control protocol"). They act on the application the
//! way a user would, and answer with JSON. This module has no network: the
//! in-process tests call [`run`] directly, and the `test-control` build
//! (`control_server.rs`) calls it for each request that arrives on its
//! loopback port, so one scenario can be written once and run at either level.

use std::time::Duration;

use cliotree_core::{Node, Path, Value};
use serde_json::{Map, Number, Value as Json, json};

use crate::app::ClioApp;
use crate::rows::display;
use crate::status::{evaluation_text, problems_text};

/// The version of the protocol, returned by `hello`.
pub const PROTOCOL: u32 = 1;

/// The longest `wait_idle` may block, however long a request asks for.
const MAX_WAIT: Duration = Duration::from_secs(120);

/// Commands of the protocol that need a workbook layer, which does not exist
/// yet.
const WAIT_FOR_WORKBOOKS: &[&str] = &[
    "open_workbook",
    "new_workbook",
    "save",
    "save_as",
    "close",
    "menu",
    "context_menu",
    "set_option",
    "grant",
];

/// Does one command. `args` is the `args` object of the request (or `null`).
pub fn run(app: &mut ClioApp, cmd: &str, args: &Json) -> Result<Json, String> {
    match cmd {
        "hello" => Ok(json!({ "protocol": PROTOCOL, "app": "ClioTree" })),
        "tab" => {
            let name = text_arg(args, "name")?;
            if !app.calc().sheet_names().any(|n| n == name) {
                return Err(format!("there is no tab called {name}"));
            }
            app.switch_to(name);
            app.refresh_rows();
            Ok(Json::Null)
        }
        "select" => {
            let path = path_arg(args, "path")?;
            if app
                .calc()
                .spec(&app.sheet)
                .and_then(|s| s.resolve(&path))
                .is_none()
            {
                return Err(format!("the schema defines no node at {path}"));
            }
            app.reveal(&path);
            if app.selection.as_ref() == Some(&path) {
                Ok(Json::Null)
            } else {
                Err(app
                    .notice
                    .clone()
                    .unwrap_or_else(|| format!("cannot select {path}")))
            }
        }
        // Shows a view of the current tab: `Data tree`, or a form by name.
        "view" => {
            let name = text_arg(args, "name")?;
            let view = app
                .view_names()
                .into_iter()
                .find(|v| v.label().eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("there is no view called {name}"))?;
            if app.set_view(view) {
                app.refresh_rows();
                Ok(Json::Null)
            } else {
                Err(app.notice.clone().unwrap_or_default())
            }
        }
        "enter" => {
            let text = text_arg(args, "text")?;
            let path = app
                .selection
                .clone()
                .ok_or("nothing is selected, so there is nowhere to enter the text")?;
            if app.in_form() {
                // What the user types into a field of the form, then leaves it.
                return app.enter_in_form(&path, text).map(|()| Json::Null);
            }
            // What the user types into a cell: begin, replace, then Enter.
            if !app.begin_editing(&path, true, Some(text.to_owned())) {
                return Err(app.notice.clone().unwrap_or_default());
            }
            if app.commit_editing() {
                Ok(Json::Null)
            } else {
                let why = app.notice.clone().unwrap_or_default();
                app.cancel_editing();
                Err(why)
            }
        }
        // The decision of trust, so that a test need not drive the banner:
        // `{"trusted": false}` withholds it, as opening an unknown file does.
        "trust" => {
            let trusted = args
                .get("trusted")
                .and_then(Json::as_bool)
                .ok_or("the argument trusted (true or false) is missing")?;
            if trusted {
                app.trust_workbook();
            } else {
                app.withhold_trust(Vec::new());
            }
            Ok(Json::Null)
        }
        "key" => key(app, &text_arg(args, "chord")?.to_ascii_lowercase()),
        "cell" => cell(app, &path_arg(args, "path")?),
        "tree" => {
            let path = path_arg(args, "path")?;
            let depth = args.get("depth").and_then(Json::as_u64).unwrap_or(1) as usize;
            let node = app
                .calc()
                .data(&app.sheet)
                .and_then(|d| d.get(&path))
                .ok_or_else(|| format!("there is no data at {path}"))?;
            Ok(tree(node, depth))
        }
        "status" => {
            app.session.tick();
            app.refresh_summary();
            let status = app.session.status();
            Ok(json!({
                "text": evaluation_text(&status),
                "problems": problems_text(app.summary.errors, app.summary.invalid),
                "pending": status.pending,
                "abandoned": status.abandoned,
                "trusted": app.calc().is_trusted(),
                "stale": app.calc().stale(),
                "notice": app.notice,
                "tab": app.sheet,
                "view": app.current_view().label(),
                "selection": app.selection.as_ref().map(ToString::to_string),
            }))
        }
        "wait_idle" => {
            let ms = args
                .get("timeout_ms")
                .and_then(Json::as_u64)
                .unwrap_or(10_000);
            let timeout = Duration::from_millis(ms).min(MAX_WAIT);
            if app.session.wait_idle(timeout) {
                app.refresh_rows();
                Ok(Json::Null)
            } else {
                Err(format!("still calculating after {ms} ms"))
            }
        }
        // These need the window, so only the running application does them.
        "quit" | "screenshot" => Err(format!("{cmd} is done by the running application")),
        other if WAIT_FOR_WORKBOOKS.contains(&other) => Err(format!(
            "{other} is not supported yet: it waits for the workbook layer"
        )),
        other => Err(format!("unknown command {other}")),
    }
}

fn text_arg<'a>(args: &'a Json, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Json::as_str)
        .ok_or_else(|| format!("the argument {name} (text) is missing"))
}

/// A path as written in formulas and messages, or `""` or `$` for the root.
fn path_arg(args: &Json, name: &str) -> Result<Path, String> {
    let text = text_arg(args, name)?;
    if text.is_empty() || text == "$" {
        return Ok(Path::root());
    }
    text.parse()
        .map_err(|e| format!("the argument {name} is not a path: {e}"))
}

/// A key the window handles by itself, as a chord such as `ctrl+z`.
fn key(app: &mut ClioApp, chord: &str) -> Result<Json, String> {
    match chord {
        "ctrl+z" => app.undo(),
        "ctrl+y" | "ctrl+shift+z" => app.redo(),
        "enter" => {
            if !app.commit_editing() {
                return Err(app.notice.clone().unwrap_or_default());
            }
        }
        "escape" => app.cancel_editing(),
        "up" => app.move_selection(-1),
        "down" => app.move_selection(1),
        "right" | "left" => {
            if let Some(path) = app.selection.clone() {
                app.set_open(&path, chord == "right");
                app.refresh_rows();
            }
        }
        other => {
            return Err(format!(
                "the key {other} is not known to the control channel"
            ));
        }
    }
    app.refresh_rows();
    Ok(Json::Null)
}

fn cell(app: &ClioApp, path: &Path) -> Result<Json, String> {
    let cell = app
        .calc()
        .cell(&app.sheet, path)
        .ok_or_else(|| format!("there is no tab {}", app.sheet))?;
    let node = cell
        .node
        .ok_or_else(|| format!("there is no data at {path}"))?;
    Ok(json!({
        "path": path.to_string(),
        "value": leaf(node.value()),
        "text": display(node.value()),
        "formula": cell.formula.map(|f| format!("={f}")),
        "error": cell.error.map(|e| json!({ "code": e.kind.code(), "message": e.message })),
        "pending": cell.pending,
        "untrusted": cell.untrusted,
        "stale": cell.stale,
        "computed_by": cell.computed_by.map(|p| p.to_string()),
        "edit_text": app.cell_text(path),
    }))
}

/// A value as JSON, containers left out.
fn leaf(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => Json::from(*i),
        Value::Float(f) => {
            Number::from_f64(*f).map_or_else(|| Json::String(display(value)), Json::Number)
        }
        Value::Array(_) | Value::Object(_) => Json::Null,
        other => Json::String(display(other)),
    }
}

/// A subtree as JSON, `depth` levels deep; deeper containers show as `...`.
fn tree(node: &Node, depth: usize) -> Json {
    match node.value() {
        Value::Array(items) if depth == 0 => json!(format!("[{} items]", items.len())),
        Value::Object(fields) if depth == 0 => json!(format!("{{{} fields}}", fields.len())),
        Value::Array(items) => Json::Array(items.iter().map(|n| tree(n, depth - 1)).collect()),
        Value::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(k, n)| (k.clone(), tree(n, depth - 1)))
                .collect::<Map<_, _>>(),
        ),
        Value::Error(e) => json!(e.kind.code()),
        other => leaf(other),
    }
}
