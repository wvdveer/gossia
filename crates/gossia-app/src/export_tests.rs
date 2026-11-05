//! Tests of exporting a tab as a standalone file (DESIGN.md, "Import and
//! export", "Exporting").

use std::path::PathBuf;
use std::time::Duration;

use gossia_calc::ExportFormat;
use gossia_core::dataformat::DataFormat;
use serde_json::json;

use crate::GossiaApp;
use crate::control;
use crate::import::SchemaChoice;
use crate::workbook_file::{Browsing, new_workbook_id};

fn sample() -> GossiaApp {
    let mut app = GossiaApp::sample();
    app.files.browse = false;
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app
}

/// A folder of its own for the files of one test.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gossia-{test}-{}-{}",
        std::process::id(),
        new_workbook_id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn the_dialog_starts_on_the_tabs_own_format_indented_without_the_schema() {
    let app = sample();
    let plan = app.export_plan("Orders").unwrap();
    assert_eq!(
        plan.formats,
        [
            ExportFormat::JSON,
            ExportFormat::Data(DataFormat::Json5),
            ExportFormat::Data(DataFormat::Jsonc),
            ExportFormat::Data(DataFormat::Toml),
            ExportFormat::Data(DataFormat::Yaml),
        ]
    );
    assert!(plan.pretty);
    assert_eq!(plan.schema.as_deref(), Some("Orders_schema"));
    assert!(!plan.with_schema);
    assert_eq!(app.export_extension(&plan), "json");
    // A schema tab has no schema to write beside it.
    let schema = app.export_plan("Orders_schema").unwrap();
    assert_eq!(schema.schema, None);
    assert!(app.export_plan("Nothing").is_err());
}

#[test]
fn an_exported_tab_with_its_schema_imports_again_onto_the_same_schema() {
    let dir = folder("export-json");
    let mut app = sample();
    app.begin_export("Orders");
    app.export.as_mut().unwrap().with_schema = true;
    app.choose_export_file();
    // Without the browser, the file is asked for by name.
    let dialog = app.files.dialog.as_mut().unwrap();
    assert_eq!(dialog.purpose, Browsing::Export);
    assert!(dialog.text.ends_with("Orders.json"), "{}", dialog.text);
    dialog.text = dir.join("out").display().to_string();
    app.apply_file_dialog();
    assert!(app.files.dialog.is_none(), "{:?}", app.files.dialog);
    assert!(app.export.is_none());

    // The extension was added, and the schema is beside the file.
    let data = read(&dir.join("out.json"));
    let schema = read(&dir.join("out.schema.json"));
    assert!(data.contains("\n  \""), "indented: {data}");
    assert!(!data.contains("parent.qty"), "no formulas: {data}");
    gossia_core::jsonschema::lower_str(&schema).unwrap();
    // The computed totals are there.
    let value: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert!(value["items"][0]["total"].is_number(), "{data}");

    // Importing it again finds the schema it was exported with.
    let plan = app.plan_import("out.json", data.as_bytes()).unwrap();
    assert_eq!(
        plan.choice,
        SchemaChoice::Existing("Orders_schema".to_owned())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_imported_xml_file_exports_to_the_same_data_and_its_xsd() {
    let dir = folder("export-xml");
    let mut app = sample();
    let text = r#"<?xml version="1.0"?>
<catalog><!-- two books --><book id="b1"><title>Dune</title><year>1965</year></book><book id="b2"><title>Emma</title><year>1815</year></book></catalog>"#;
    let plan = app.plan_import("catalog.xml", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();

    let answer = control::run(
        &mut app,
        "export",
        &json!({ "tab": "catalog", "path": dir.join("c.xml").display().to_string(), "schema": true }),
    )
    .unwrap();
    let xsd_path = dir.join("c.xsd");
    assert_eq!(answer["schema"], xsd_path.display().to_string());
    let exported = read(&dir.join("c.xml"));
    assert!(exported.starts_with("<?xml"), "{exported}");
    assert!(exported.contains("<title>Emma</title>"), "{exported}");
    // The XSD read back describes the data written.
    let spec = gossia_core::xsd::lower_str(&read(&xsd_path)).unwrap();
    let back = gossia_core::xml::parse(&exported, &spec).unwrap();
    assert!(gossia_core::validate::validate(&spec, &back.node).is_empty());
    assert_eq!(
        gossia_core::xml::to_string(&back.node, &spec, &back.document, true).unwrap(),
        exported
    );

    // The XSD tab exports on its own as an .xsd file.
    let plan = app.export_plan("catalogSchema").unwrap();
    assert_eq!(app.export_extension(&plan), "xsd");

    // Compact XML has no line breaks between elements.
    control::run(
        &mut app,
        "export",
        &json!({ "tab": "catalog", "path": dir.join("compact.xml").display().to_string(), "compact": true }),
    )
    .unwrap();
    let compact = read(&dir.join("compact.xml"));
    assert!(compact.contains("<book id=\"b1\"><title>"), "{compact}");
    assert!(
        !dir.join("compact.xsd").exists(),
        "the schema was not asked for"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_text_file_comes_back_out_as_the_same_lines() {
    let dir = folder("export-text");
    let mut app = sample();
    let text = "first\n\nthird\n";
    let plan = app.plan_import("notes.txt", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let plan = app.export_plan("notes").unwrap();
    assert_eq!(
        plan.formats,
        [
            ExportFormat::JSON,
            ExportFormat::Data(DataFormat::Json5),
            ExportFormat::Data(DataFormat::Jsonc),
            ExportFormat::Data(DataFormat::JsonLines),
            ExportFormat::Data(DataFormat::Yaml),
            ExportFormat::Data(DataFormat::YamlDocuments),
            ExportFormat::Text,
        ]
    );

    let path = dir.join("notes.txt");
    control::run(
        &mut app,
        "export",
        // The schema is not written beside text.
        &json!({ "tab": "notes", "path": path.display().to_string(), "format": "text", "schema": true }),
    )
    .unwrap();
    assert_eq!(read(&path), text);
    assert!(!dir.join("notes.schema.json").exists());

    // An object is not text.
    let error = control::run(
        &mut app,
        "export",
        &json!({ "tab": "Orders", "path": path.display().to_string(), "format": "text" }),
    )
    .unwrap_err();
    assert!(error.contains("cannot be exported as text"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_schema_file_that_would_be_the_data_file_is_refused() {
    let dir = folder("export-clash");
    let mut app = sample();
    let text = "<a><b>1</b></a>";
    let plan = app.plan_import("a.xml", text.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let path = dir.join("a.xsd");
    let error = control::run(
        &mut app,
        "export",
        &json!({ "tab": "a", "path": path.display().to_string(), "schema": true }),
    )
    .unwrap_err();
    assert!(error.contains("another name"), "{error}");
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_write_keeps_the_dialog_with_the_reason() {
    let dir = folder("export-fail");
    let mut app = sample();
    app.begin_export("Prices");
    app.choose_export_file();
    // A folder that is not there.
    app.files.dialog.as_mut().unwrap().text =
        dir.join("missing").join("p.json").display().to_string();
    app.apply_file_dialog();
    let dialog = app.files.dialog.as_ref().unwrap();
    assert!(dialog.error.is_some());
    assert!(app.export.is_some(), "the choices are kept");
    let _ = std::fs::remove_dir_all(&dir);
}
