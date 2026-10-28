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
const WAIT_FOR_WORKBOOKS: &[&str] = &["close", "menu", "context_menu", "grant"];

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
        // Renames a tab, as the *Rename tab...* dialog does.
        "rename_tab" => {
            let from = text_arg(args, "from")?;
            let to = text_arg(args, "to")?;
            app.rename_tab(from, to)?;
            app.refresh_rows();
            Ok(Json::Null)
        }
        // A new tab, as *New tab...* makes it: with its own schema, or with
        // the schema of `schema`.
        "new_tab" => {
            let name = text_arg(args, "name")?;
            let schema = match args.get("schema").and_then(Json::as_str) {
                Some(schema) => crate::tab_commands::NewSchema::Existing(schema.to_owned()),
                None => crate::tab_commands::NewSchema::Own,
            };
            app.begin_new_tab();
            let mut dialog = app
                .tab_dialogs
                .new
                .take()
                .ok_or_else(|| app.notice.clone().unwrap_or_default())?;
            dialog.name = name.to_owned();
            dialog.schema_name = format!("{name}Schema");
            dialog.schema = schema;
            app.create_tab(&dialog)?;
            app.refresh_rows();
            Ok(Json::Null)
        }
        "delete_tab" => {
            let name = text_arg(args, "name")?;
            app.delete_tab(name)?;
            app.refresh_rows();
            Ok(Json::Null)
        }
        "bind_schema" => {
            let tab = text_arg(args, "tab")?;
            let schema = text_arg(args, "schema")?;
            app.bind_tab(tab, schema)?;
            app.refresh_rows();
            Ok(Json::Null)
        }
        // Imports a file with what the dialog starts with, or with `name`,
        // and `schema` (a schema tab, or `"new"` for the inferred schema).
        "import" => {
            let path = text_arg(args, "path")?;
            app.begin_import_path(std::path::Path::new(path))?;
            let mut plan = app.imports.pop_back().ok_or("nothing was read")?;
            if let Some(name) = args.get("name").and_then(Json::as_str) {
                plan.name = name.to_owned();
                plan.schema_name = format!("{name}Schema");
            }
            match args.get("schema").and_then(Json::as_str) {
                Some("new") => plan.choice = crate::import::SchemaChoice::Inferred,
                Some(schema) => {
                    plan.choice = crate::import::SchemaChoice::Existing(schema.to_owned());
                }
                None => {}
            }
            let schema = match &plan.choice {
                crate::import::SchemaChoice::Inferred => plan.schema_name.clone(),
                crate::import::SchemaChoice::Existing(s)
                | crate::import::SchemaChoice::Replace(s) => s.clone(),
            };
            app.import(&plan)?;
            app.refresh_rows();
            Ok(json!({ "tab": plan.name, "schema": schema, "report": plan.report() }))
        }
        // Sets one option as the Options dialog does and presses OK: `name`
        // (see `options::Field::key`, or `theme`) and `value` (a number in the
        // dialog's unit, or text as typed; for `theme`, `system`, `light` or
        // `dark`; for `display.row_density`, `compact`, `normal` or
        // `comfortable`). A workbook setting is an undoable entry.
        "set_option" => {
            let name = text_arg(args, "name")?;
            let value = args.get("value").ok_or("set_option needs a value")?;
            let text = match value {
                Json::String(s) => s.clone(),
                other => other.to_string(),
            };
            app.begin_options(None);
            let dialog = app.options.as_mut().expect("the dialog was opened");
            if name == "theme" {
                dialog.theme = crate::prefs::ThemeChoice::ALL
                    .into_iter()
                    .find(|t| t.key() == text)
                    .ok_or_else(|| format!("there is no theme {text:?}"))?;
            } else if name == "display.row_density" {
                dialog.density = crate::prefs::Density::from_key(&text)
                    .ok_or_else(|| format!("there is no row density {text:?}"))?;
            } else {
                let field = crate::options::Field::from_key(name)
                    .ok_or_else(|| format!("there is no option {name:?}"))?;
                dialog.texts.insert(field, text);
            }
            let done = app.apply_options();
            app.options = None;
            done?;
            Ok(Json::Null)
        }
        // Exports a tab (`tab`, or the current one) to `path`, with what the
        // dialog starts with, or `format` (`json`, `xml` or `text`),
        // `compact` and `schema` (also write the schema tab beside it).
        "export" => {
            let path = std::path::PathBuf::from(text_arg(args, "path")?);
            let tab = match args.get("tab").and_then(Json::as_str) {
                Some(tab) => tab.to_owned(),
                None => app.sheet.clone(),
            };
            let mut plan = app.export_plan(&tab)?;
            if let Some(format) = args.get("format").and_then(Json::as_str) {
                plan.format = match format {
                    "json" => cliotree_calc::ExportFormat::Json,
                    "xml" => cliotree_calc::ExportFormat::Xml,
                    "text" => cliotree_calc::ExportFormat::Text,
                    other => return Err(format!("there is no format {other:?}")),
                };
                if !plan.formats.contains(&plan.format) {
                    return Err(format!("tab {tab} cannot be exported as {format}"));
                }
            }
            plan.pretty = !args.get("compact").and_then(Json::as_bool).unwrap_or(false);
            plan.with_schema = args.get("schema").and_then(Json::as_bool).unwrap_or(false);
            let schema = app
                .export_schema_path(&plan, &path)
                .map(|p| p.display().to_string());
            app.export = Some(plan);
            let done = app.export_to(&path);
            app.export = None;
            done?;
            Ok(json!({ "schema": schema }))
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
        // Pastes a whole document in the Raw view.
        "paste" => {
            let text = text_arg(args, "text")?;
            if app.current_view().label() != "Raw" {
                return Err("paste needs the Raw view".to_owned());
            }
            app.paste_document(text).map(|()| Json::Null)
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
                let why = why_not_entered(app);
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
        // Inserts, edits or deletes (`text` null or absent) the comment on a
        // node of an XML tab, as the comment box does.
        "set_comment" => {
            let path = path_arg(args, "path")?;
            let text = args.get("text").and_then(Json::as_str);
            let sheet = app.sheet.clone();
            app.session
                .calc_mut()
                .set_comment(&sheet, &path, text)
                .map_err(|e| e.to_string())?;
            Ok(Json::Null)
        }
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
        // What a user does on a form: click the button with this label.
        "click_button" => {
            let label = text_arg(args, "label")?;
            if !app.in_form() {
                return Err("click_button needs a form view".to_owned());
            }
            let design = app.design_of(&app.form_name());
            let procedure = design
                .walk()
                .into_iter()
                .map(|(_, c)| c)
                .find(|c| {
                    c.kind == cliotree_core::form::ControlKind::Button
                        && c.label.as_deref() == Some(label)
                })
                .ok_or_else(|| format!("there is no button called {label} on this form"))?
                .procedure
                .clone()
                .ok_or_else(|| format!("the button {label} has no procedure"))?;
            app.click_button(&procedure, label).map(|()| Json::Null)
        }
        "status" => {
            app.session.tick();
            app.collect_procedures();
            app.refresh_summary();
            let status = app.session.status();
            Ok(json!({
                "text": evaluation_text(&status),
                "procedure": status.procedure,
                "problems": problems_text(app.summary.errors, app.summary.invalid),
                "pending": status.pending,
                "abandoned": status.abandoned,
                "trusted": app.calc().is_trusted(),
                "stale": app.calc().stale(),
                "notice": app.notice,
                "tab": app.sheet,
                "file": app.doc.path.as_ref().map(|p| p.display().to_string()),
                "modified": app.is_dirty(),
                "recent_files": app
                    .user
                    .prefs
                    .recent
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
                "view": app.current_view().label(),
                "selection": app.selection.as_ref().map(ToString::to_string),
            }))
        }
        "open_workbook" => {
            let path = text_arg(args, "path")?;
            app.open_path(std::path::Path::new(path))?;
            app.refresh_rows();
            Ok(json!({
                "tabs": app.calc().sheet_names().collect::<Vec<_>>(),
                "trusted": app.calc().is_trusted(),
                "notice": app.notice,
            }))
        }
        // File > New, without the question about unsaved changes.
        "new_workbook" => {
            app.new_workbook();
            app.refresh_rows();
            Ok(json!({ "tabs": app.calc().sheet_names().collect::<Vec<_>>() }))
        }
        "save" => {
            app.save()?;
            Ok(Json::Null)
        }
        "save_as" => {
            let path = text_arg(args, "path")?;
            app.save_as(std::path::Path::new(path))?;
            Ok(Json::Null)
        }
        "wait_idle" => {
            let ms = args
                .get("timeout_ms")
                .and_then(Json::as_u64)
                .unwrap_or(10_000);
            let timeout = Duration::from_millis(ms).min(MAX_WAIT);
            if app.session.wait_idle(timeout) {
                app.collect_procedures();
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

/// Why what was typed was not entered: the refusal, or the question it waits
/// for.
fn why_not_entered(app: &ClioApp) -> String {
    match (&app.notice, &app.replace_inside) {
        (Some(notice), _) => notice.clone(),
        (None, Some(ask)) => ask.question(),
        (None, None) => String::new(),
    }
}

/// A key the window handles by itself, as a chord such as `ctrl+z`.
fn key(app: &mut ClioApp, chord: &str) -> Result<Json, String> {
    match chord {
        "ctrl+z" => app.undo(),
        "ctrl+y" | "ctrl+shift+z" => app.redo(),
        "enter" => {
            if !app.commit_editing() {
                return Err(why_not_entered(app));
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
        "comment": node.comment(),
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
