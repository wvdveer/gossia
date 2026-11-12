//! The commands of the test control protocol (DESIGN.md, "Testing and CI",
//! level 8 and "The test-control protocol"). They act on the application the
//! way a user would, and answer with JSON. This module has no network: the
//! in-process tests call [`run`] directly, and the `test-control` build
//! (`control_server.rs`) calls it for each request that arrives on its
//! loopback port, so one scenario can be written once and run at either level.

use std::time::Duration;

use gossia_core::{Node, Path, Value};
use serde_json::{Map, Number, Value as Json, json};

use crate::app::GossiaApp;
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
pub fn run(app: &mut GossiaApp, cmd: &str, args: &Json) -> Result<Json, String> {
    match cmd {
        "hello" => Ok(json!({ "protocol": PROTOCOL, "app": "Gossia" })),
        // Opens a module as the code editor's explorer does (in a host whose
        // editor has the modules, there).
        "open_module" => {
            let name = text_arg(args, "name")?;
            if app.calc().module_source(name).is_none() {
                return Err(format!("there is no module {name}"));
            }
            app.open_code_editor();
            app.open_module(name);
            Ok(json!({}))
        }
        // A command by its name, as a host's command list runs it.
        "command" => {
            app.run_command(text_arg(args, "name")?)?;
            Ok(json!({}))
        }
        "tab" => {
            let name = text_arg(args, "name")?;
            if app.calc().tab_kind(name).is_none() {
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
        // A new tab, as *New tab...* makes it: with its own schema (a JSON
        // Schema, or with `xml` true an XSD), or with the schema of `schema`;
        // `format` is the data format of a JSON Schema tab ("yaml", "toml", ...).
        // A new report, as *New report...* makes it: `kind` is "single_page"
        // (the default) or "multipage", and `start_tab` with `start_path`
        // ("items", "" for the root) the array it starts from.
        "new_tab" if args.get("report").and_then(Json::as_bool) == Some(true) => {
            let name = text_arg(args, "name")?;
            let kind = match args.get("kind").and_then(Json::as_str) {
                None | Some("single_page") => gossia_core::report::ReportKind::SinglePage,
                Some("multipage") => gossia_core::report::ReportKind::Multipage,
                Some(other) => return Err(format!("{other} is not a kind of report")),
            };
            let start = match args.get("start_tab").and_then(Json::as_str) {
                Some(tab) => {
                    let path = args.get("start_path").and_then(Json::as_str).unwrap_or("");
                    let path: gossia_core::Path =
                        path.parse().map_err(|e| format!("start_path: {e:?}"))?;
                    Some((tab.to_owned(), path))
                }
                None => None,
            };
            app.create_report_tab(name, kind, start.as_ref())?;
            Ok(Json::Null)
        }
        // *Duplicate* on a report tab's menu; gives the copy's name.
        "duplicate_tab" => {
            let name = text_arg(args, "name")?;
            let copy = app.duplicate_report_tab(name)?;
            Ok(json!({ "name": copy }))
        }
        "new_tab" => {
            let name = text_arg(args, "name")?;
            let xml = args.get("xml").and_then(Json::as_bool).unwrap_or(false);
            let schema = match args.get("schema").and_then(Json::as_str) {
                Some(schema) => crate::tab_commands::NewSchema::Existing(schema.to_owned()),
                None if xml => crate::tab_commands::NewSchema::Own(gossia_calc::SchemaFormat::Xsd),
                None => crate::tab_commands::NewSchema::Own(gossia_calc::SchemaFormat::Json),
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
            if let Some(code) = args.get("format").and_then(Json::as_str) {
                dialog.format = gossia_core::dataformat::DataFormat::from_code(code)
                    .filter(|f| f.json_shaped())
                    .ok_or_else(|| format!("{code} is not a format of JSON-shaped data"))?;
            }
            app.create_tab(&dialog)?;
            app.refresh_rows();
            Ok(Json::Null)
        }
        // Moves tab `name` to place `to` (0 is the first), as dragging it
        // along the bar does.
        "move_tab" => {
            let name = text_arg(args, "name")?;
            let to = args
                .get("to")
                .and_then(Json::as_u64)
                .ok_or("move_tab needs a place, to")?;
            app.move_tab(name, to as usize);
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
        // and `schema` (a schema tab, `"new"` for the inferred schema, or
        // `"self"` for a file that is a schema, as a schema tab).
        // Drops files on the window, as dragging them from a file manager
        // does, and presses Import in each import dialog that opens, with
        // its choices as they are. Gives the tabs made.
        // In place of `paths`, `files` gives each file as `name` and `text`,
        // dropped as the browser drops them (the page has no files of its own).
        "drop" => {
            app.imports.clear();
            app.notice = None;
            if let Some(files) = args.get("files").and_then(Json::as_array) {
                for file in files {
                    let name = text_arg(file, "name")?;
                    let text = text_arg(file, "text")?;
                    crate::host::picked(crate::host::Picked {
                        purpose: crate::host::purpose::DROP.to_owned(),
                        name: name.to_owned(),
                        bytes: text.as_bytes().to_vec(),
                    });
                }
                app.receive_picked();
            } else {
                let paths: Vec<std::path::PathBuf> = args
                    .get("paths")
                    .and_then(Json::as_array)
                    .ok_or("drop needs paths or files")?
                    .iter()
                    .map(|p| p.as_str().map(std::path::PathBuf::from))
                    .collect::<Option<_>>()
                    .ok_or("drop needs paths as text")?;
                app.drop_files(paths);
            }
            if let Some(notice) = app.notice.take() {
                return Err(notice);
            }
            let mut tabs = Vec::new();
            while let Some(plan) = app.imports.pop_front() {
                app.import(&plan)?;
                tabs.push(plan.name.clone());
            }
            app.refresh_rows();
            Ok(json!({ "tabs": tabs }))
        }
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
                Some("self") => plan.choice = crate::import::SchemaChoice::SchemaTab,
                Some(schema) => {
                    plan.choice = crate::import::SchemaChoice::Existing(schema.to_owned());
                }
                None => {}
            }
            // Calendar entries or contacts added to a tab of their kind.
            if let Some(tab) = args.get("add_to").and_then(Json::as_str) {
                plan.choice = crate::import::SchemaChoice::AddTo(tab.to_owned());
            }
            let schema = match &plan.choice {
                crate::import::SchemaChoice::Inferred => plan.schema_name.clone(),
                crate::import::SchemaChoice::SchemaTab => plan.name.clone(),
                crate::import::SchemaChoice::Existing(s)
                | crate::import::SchemaChoice::Replace(s) => s.clone(),
                crate::import::SchemaChoice::AddTo(tab) => {
                    app.calc().schema_of(tab).unwrap_or_default().to_owned()
                }
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
            } else if name == gossia_calc::KEEP_REPORT_RESULTS {
                dialog.keep_report_results = text == "true";
            } else if name == "appearance.language" {
                // A tag, or "system" to follow the system's (the host's).
                dialog.language = match text.as_str() {
                    "system" => None,
                    tag => Some(
                        gossia_i18n::Language::from_tag(tag)
                            .ok_or_else(|| format!("there is no language {tag:?}"))?,
                    ),
                };
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
        // dialog starts with, or `format` (`json`, `xml`, `text`, `csv`, `tsv`, `xlsx`, `ods`, `ics` or `vcf`),
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
                    "text" => gossia_calc::ExportFormat::Text,
                    "csv" => gossia_calc::ExportFormat::Csv,
                    "tsv" => gossia_calc::ExportFormat::Tsv,
                    "xlsx" => gossia_calc::ExportFormat::Xlsx,
                    "ods" => gossia_calc::ExportFormat::Ods,
                    "ics" => gossia_calc::ExportFormat::Calendar,
                    "vcf" => gossia_calc::ExportFormat::Contacts,
                    code => match gossia_core::dataformat::DataFormat::from_code(code) {
                        Some(f) => gossia_calc::ExportFormat::Data(f),
                        None => return Err(format!("there is no format {code:?}")),
                    },
                };
                if !plan.formats.contains(&plan.format) {
                    return Err(format!("tab {tab} cannot be exported as {format}"));
                }
            }
            // For CSV, TSV, xlsx and ods, the array written as rows (`rows`, a path).
            if let Some(rows) = args.get("rows").and_then(Json::as_str) {
                let at: gossia_core::Path = rows.parse().map_err(|e| format!("{rows}: {e}"))?;
                if !plan.rows.contains(&at) {
                    return Err(format!("{rows} of tab {tab} is not a list of records"));
                }
                plan.at = Some(at);
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
        // Writes a report as PDF or HTML, as *Export as PDF...* and *Export
        // as HTML...* do: `report` (the tab shown if left out) is shown and
        // calculated, with the parameter values it has, and written to
        // `path`.
        "export_pdf" | "export_html" => {
            let path = std::path::PathBuf::from(text_arg(args, "path")?);
            let report = match args.get("report").and_then(Json::as_str) {
                Some(report) => report.to_owned(),
                None => app.sheet.clone(),
            };
            if app.calc().report(&report).is_none() {
                return Err(format!("there is no report called {report}"));
            }
            if cmd == "export_pdf" {
                app.begin_export_pdf(&report);
            } else {
                app.begin_export_html(&report);
            }
            app.session.calc_mut().show_report(Some(&report));
            if !app.session.wait_idle(std::time::Duration::from_secs(60)) {
                app.pdf_export = None;
                return Err(format!(
                    "{report} was still being calculated after a minute"
                ));
            }
            let done = app.export_report_to(&path);
            let error = app.pdf_export.take().and_then(|e| e.error);
            done?;
            match error {
                Some(error) => Err(error),
                None => Ok(Json::Null),
            }
        }
        // Prints a report, as *Print...* does: `report` (the tab shown if
        // left out) is shown and calculated, its PDF written to a temporary
        // file and opened in the system's PDF program (unless `open` is
        // false). Answers with the file.
        "print" => {
            let report = match args.get("report").and_then(Json::as_str) {
                Some(report) => report.to_owned(),
                None => app.sheet.clone(),
            };
            if app.calc().report(&report).is_none() {
                return Err(format!("there is no report called {report}"));
            }
            let open = args.get("open").and_then(Json::as_bool).unwrap_or(true);
            let outside = std::mem::replace(
                &mut app.outside,
                if open {
                    crate::report_print::Outside::System
                } else {
                    crate::report_print::Outside::Record(Vec::new())
                },
            );
            app.begin_print(&report);
            app.session.calc_mut().show_report(Some(&report));
            let done = if app.session.wait_idle(std::time::Duration::from_secs(60)) {
                app.choose_pdf_file();
                match app.pdf_export.take() {
                    Some(export) => Err(export
                        .error
                        .unwrap_or_else(|| format!("{report} was not printed"))),
                    None => Ok(()),
                }
            } else {
                app.pdf_export = None;
                Err(format!(
                    "{report} was still being calculated after a minute"
                ))
            };
            let opened = std::mem::replace(&mut app.outside, outside);
            done?;
            let file = match opened {
                crate::report_print::Outside::Record(files) => files.last().cloned(),
                crate::report_print::Outside::System => app.last_printed.clone(),
            };
            Ok(json!({ "file": file.map(|f| f.display().to_string()) }))
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
                .find(|v| v.name().eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("there is no view called {name}"))?;
            if app.set_view(view) {
                app.refresh_rows();
                Ok(Json::Null)
            } else {
                Err(app.notice.clone().unwrap_or_default())
            }
        }
        // Pastes a whole document in the Raw view, or cells at the selection
        // of the tree or a table (`values` for *Paste values*).
        "paste" => {
            let text = text_arg(args, "text")?;
            if *app.current_view() == crate::form::View::Raw {
                return app.paste_document(text).map(|()| Json::Null);
            }
            if !app.in_tree() {
                return Err("paste needs the Raw view or the data tree".to_owned());
            }
            let values = args.get("values").and_then(Json::as_bool).unwrap_or(false);
            let mode = if values {
                crate::clipboard::PasteMode::Values
            } else {
                crate::clipboard::PasteMode::Cells
            };
            app.paste_clip(text, mode).map(|()| Json::Null)
        }
        // Copies the selection (`mode`: `values`, the default, or `formulas`)
        // and gives the text put on the clipboard.
        "copy" => {
            let mode = match args.get("mode").and_then(Json::as_str) {
                None | Some("values") => crate::clipboard::CopyMode::Cells,
                Some("formulas") => crate::clipboard::CopyMode::Formulas,
                Some(other) => return Err(format!("there is no copy mode called {other}")),
            };
            app.copy_cells(mode)
                .map(Json::String)
                .ok_or_else(|| "nothing is selected to copy".to_owned())
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
        // Chooses the value of a parameter of a report, as the parameter bar
        // does: `value` read by the parameter's kind (a date as "2026-01-31",
        // a period as {"start": ..., "end": ...}, choices as a list, null for
        // empty), or with no `value`, its default.
        "report_param" => {
            let report = text_arg(args, "report")?;
            let name = text_arg(args, "name")?;
            let kind = app
                .calc()
                .report(report)
                .ok_or_else(|| format!("there is no report called {report}"))?
                .param(name)
                .ok_or_else(|| format!("{report} has no parameter called {name}"))?
                .kind;
            let value = match args.get("value") {
                None => None,
                Some(v) => Some(param_value(kind, v)?),
            };
            app.session.calc_mut().set_report_param(report, name, value);
            Ok(Json::Null)
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
        // node of an XML tab, as the comment box does: before its element, or
        // with `place` `start` or `end`, inside it.
        "set_comment" => {
            let path = path_arg(args, "path")?;
            let text = args.get("text").and_then(Json::as_str);
            let place = match args.get("place").and_then(Json::as_str) {
                None | Some("before") => gossia_core::CommentPlace::Before,
                Some("start") => gossia_core::CommentPlace::Start,
                Some("end") => gossia_core::CommentPlace::End,
                Some(other) => {
                    return Err(format!("place is before, start or end, not {other}"));
                }
            };
            let sheet = app.sheet.clone();
            app.session
                .calc_mut()
                .set_comments(&sheet, &path, &[(place, text)])
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
                    c.kind == gossia_core::form::ControlKind::Button
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
                "view": if app.in_report() { "Report" } else { app.current_view().name() },
                "selection": app.selection.as_ref().map(ToString::to_string),
                "code_editor": app.code.open,
                // The interface's language, and the language whose font for
                // Japanese or Chinese the window has (null: none).
                "language": gossia_i18n::language().tag(),
                "cjk_font": app.cjk_font_for.flatten().map(gossia_i18n::Language::tag),
            }))
        }
        "open_workbook" => {
            let path = text_arg(args, "path")?;
            app.open_path(std::path::Path::new(path))?;
            app.refresh_rows();
            Ok(json!({
                "tabs": app.calc().tab_names(),
                "trusted": app.calc().is_trusted(),
                "notice": app.notice,
            }))
        }
        // File > New, without the question about unsaved changes.
        "new_workbook" => {
            app.new_workbook();
            app.refresh_rows();
            Ok(json!({ "tabs": app.calc().tab_names() }))
        }
        // In a host the bytes go to the host, as Ctrl+S gives them.
        "save" => {
            match crate::host::current() {
                Some(host) => app.save_to_host(&*host)?,
                None => app.save()?,
            }
            Ok(Json::Null)
        }
        // In a host that keeps files (the browser page), Save As asks it
        // where, without a path.
        "save_as" if args.get("path").is_none() => {
            app.begin_save_as();
            Ok(Json::Null)
        }
        "save_as" => {
            let path = text_arg(args, "path")?;
            app.save_as(std::path::Path::new(path))?;
            Ok(Json::Null)
        }
        // File > Open and File > Open Recent in a host that keeps files: the
        // host is asked for the workbook, which comes later.
        "open" => {
            app.begin_open();
            Ok(Json::Null)
        }
        "open_recent" => {
            app.begin_open_host_recent(text_arg(args, "key")?);
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

/// A parameter's value from JSON, read by its kind.
fn param_value(kind: gossia_core::report::ParamKind, v: &Json) -> Result<Value, String> {
    use gossia_core::report::{ParamKind, period};
    let date = |v: &Json| -> Result<chrono::NaiveDate, String> {
        v.as_str()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| format!("{v} is not a date written as 2026-01-31"))
    };
    let scalar = |v: &Json| -> Result<Value, String> {
        match v {
            Json::String(s) => Ok(Value::Str(s.clone())),
            Json::Bool(b) => Ok(Value::Bool(*b)),
            Json::Number(n) => Ok(n
                .as_i64()
                .map(Value::Int)
                .or_else(|| n.as_f64().map(Value::Float))
                .ok_or("a number that does not fit")?),
            other => Err(format!("{other} is not a value")),
        }
    };
    if v.is_null() {
        return Ok(Value::Null);
    }
    let value = match kind {
        ParamKind::Date => Value::Date(date(v)?),
        ParamKind::Period => period(
            date(v.get("start").unwrap_or(&Json::Null))?,
            date(v.get("end").unwrap_or(&Json::Null))?,
        ),
        ParamKind::Choices => Value::Array(
            v.as_array()
                .ok_or("choices are a list")?
                .iter()
                .map(|x| scalar(x).map(Node::new))
                .collect::<Result<_, _>>()?,
        ),
        _ => scalar(v)?,
    };
    if kind.accepts(&value) {
        Ok(value)
    } else {
        Err(format!(
            "{v} is not a value of a {} parameter",
            kind.label().to_lowercase()
        ))
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
fn why_not_entered(app: &GossiaApp) -> String {
    match (&app.notice, &app.replace_inside) {
        (Some(notice), _) => notice.clone(),
        (None, Some(ask)) => ask.question(),
        (None, None) => String::new(),
    }
}

/// A key the window handles by itself, as a chord such as `ctrl+z`.
fn key(app: &mut GossiaApp, chord: &str) -> Result<Json, String> {
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

fn cell(app: &GossiaApp, path: &Path) -> Result<Json, String> {
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
        "comment_start": node.comment_at(gossia_core::CommentPlace::Start),
        "comment_end": node.comment_at(gossia_core::CommentPlace::End),
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
