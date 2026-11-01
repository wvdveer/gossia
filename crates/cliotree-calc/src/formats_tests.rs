//! Tests of the file format of a tab (DESIGN.md, "Other file formats"):
//! changing it, its comments, saving and opening it, and exporting.

use std::sync::Arc;

use cliotree_core::dataformat::DataFormat;
use cliotree_core::{CommentPlace, Path, json, metaschema, xml, xsd};

use crate::python::PythonSyntax;
use crate::{Calc, ExportFormat, Meta};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "name": {"type": "string"},
    "tags": {"type": "array", "items": {"type": "string"}}
  }
}"#;

const LIST: &str = r#"{"type": "array", "items": {"type": "object",
  "properties": {"n": {"type": "integer"}}}}"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

/// A tab "Settings" bound to "SettingsSchema" and a tab "Rows" whose root is
/// an array, bound to "RowsSchema".
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.create_json_schema_tab("SettingsSchema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    let data = DataFormat::Json5
        .read("{\n  // who\n  name: 'Ann',\n  tags: ['x'],\n}")
        .unwrap()
        .node;
    calc.create_data_tab("Settings", "SettingsSchema", data, None)
        .unwrap();
    calc.create_json_schema_tab("RowsSchema", json::parse(LIST).unwrap().node)
        .unwrap();
    let rows = json::parse(r#"[{"n": 1}, {"n": 2}]"#).unwrap().node;
    calc.create_data_tab("Rows", "RowsSchema", rows, None)
        .unwrap();
    calc
}

#[test]
fn a_tab_offers_the_formats_that_can_hold_it() {
    let calc = book();
    assert_eq!(calc.format("Settings"), Some(DataFormat::Json));
    assert_eq!(
        calc.formats_for("Settings"),
        [
            DataFormat::Json,
            DataFormat::Json5,
            DataFormat::Jsonc,
            DataFormat::Toml,
            DataFormat::Yaml
        ]
    );
    assert!(
        calc.format_problem("Settings", DataFormat::JsonLines)
            .unwrap()
            .contains("array")
    );
    assert!(calc.formats_for("Rows").contains(&DataFormat::JsonLines));
    assert!(
        calc.format_problem("Rows", DataFormat::Toml)
            .unwrap()
            .contains("object")
    );
    assert!(
        calc.format_problem("SettingsSchema", DataFormat::Json5)
            .unwrap()
            .contains("schema tab")
    );
    assert!(calc.format_problem("Settings", DataFormat::Xml).is_some());
}

#[test]
fn comments_follow_the_tab_through_a_change_of_format() {
    let mut calc = book();
    // JSON has none in the file, so the workbook keeps them (owner,
    // 2026-11-01): the comment the data came with is there.
    assert!(calc.comment_refusal("Settings", &p("name")).is_none());
    assert!(calc.format_has_comments("Settings"));
    assert_eq!(calc.comment("Settings", &p("name")), Some("who"));
    calc.set_format("Settings", DataFormat::Json5).unwrap();
    assert_eq!(calc.comment("Settings", &p("name")), Some("who"));
    // An array can have one here, unlike on an XML tab, and `--` is fine.
    calc.set_comment("Settings", &p("tags"), Some("a -- b-"))
        .unwrap();
    // Back to JSON: nothing is lost, so nothing is asked, but export leaves
    // them out.
    assert_eq!(calc.comments_lost("Settings", DataFormat::Json), 0);
    assert_eq!(calc.comments_left_out("Settings", DataFormat::Json), 2);
    assert_eq!(calc.comments_left_out("Settings", DataFormat::Jsonc), 0);
    calc.set_format("Settings", DataFormat::Json).unwrap();
    assert_eq!(calc.comment("Settings", &p("tags")), Some("a -- b-"));
    calc.undo().unwrap();
    assert_eq!(calc.format("Settings"), Some(DataFormat::Json5));
    assert_eq!(calc.comment("Settings", &p("name")), Some("who"));
}

#[test]
fn a_tab_is_saved_and_opened_in_its_format_with_its_comments() {
    let mut calc = book();
    calc.set_format("Settings", DataFormat::Json5).unwrap();
    calc.set_comments("Settings", &p(""), &[(CommentPlace::End, Some("bye"))])
        .unwrap();
    calc.set_format("Rows", DataFormat::JsonLines).unwrap();
    let meta = Meta {
        workbook_id: "0123456789abcdef".to_owned(),
        app_version: "0.1.0".to_owned(),
        ..Meta::default()
    };
    let package = calc.to_package(&meta).unwrap();
    let text = |path: &str| String::from_utf8(package.get(path).unwrap().to_vec()).unwrap();
    assert_eq!(
        text("sheets/Settings/data.json5"),
        "{\n  // who\n  name: \"Ann\",\n  tags: [\n    \"x\"\n  ]\n  // bye\n}\n"
    );
    assert_eq!(text("sheets/Rows/data.jsonl"), "{\"n\":1}\n{\"n\":2}\n");
    assert!(package.get("sheets/Settings/data.json").is_none());
    assert!(text("manifest.json").contains("\"format\": \"json5\""));
    let opened = Calc::from_package(Arc::new(PythonSyntax), &package).unwrap();
    assert_eq!(opened.calc.format("Settings"), Some(DataFormat::Json5));
    assert_eq!(opened.calc.format("Rows"), Some(DataFormat::JsonLines));
    assert_eq!(opened.calc.format("SettingsSchema"), Some(DataFormat::Json));
    assert_eq!(opened.calc.comment("Settings", &p("name")), Some("who"));
}

#[test]
fn a_tab_can_be_rebound_only_to_a_schema_its_format_can_hold() {
    let mut calc = book();
    calc.set_format("Rows", DataFormat::JsonLines).unwrap();
    assert!(
        calc.rebind_problem("Rows", "SettingsSchema")
            .unwrap()
            .contains("JSON Lines")
    );
}

#[test]
fn a_tab_exports_in_any_format_it_could_be_written_in() {
    let mut calc = book();
    calc.set_format("Settings", DataFormat::Jsonc).unwrap();
    assert_eq!(
        calc.export_formats("Settings"),
        [
            ExportFormat::Data(DataFormat::Jsonc),
            ExportFormat::JSON,
            ExportFormat::Data(DataFormat::Json5),
            ExportFormat::Data(DataFormat::Toml),
            ExportFormat::Data(DataFormat::Yaml),
        ]
    );
    assert_eq!(
        calc.export_text("Settings", ExportFormat::Data(DataFormat::Jsonc), true)
            .unwrap(),
        "{\n  // who\n  \"name\": \"Ann\",\n  \"tags\": [\n    \"x\"\n  ]\n}\n"
    );
    assert_eq!(
        calc.export_text("Settings", ExportFormat::JSON, false)
            .unwrap(),
        "{\"name\":\"Ann\",\"tags\":[\"x\"]}\n"
    );
    assert!(
        calc.export_text("Settings", ExportFormat::Data(DataFormat::JsonLines), true)
            .is_err()
    );
    assert_eq!(
        calc.export_text("Rows", ExportFormat::Data(DataFormat::JsonLines), true)
            .unwrap(),
        "{\"n\":1}\n{\"n\":2}\n"
    );
    // An XML tab and an XSD schema tab stay XML.
    let schema = xml::parse(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="a" type="xs:string"/></xs:schema>"#,
        metaschema::xsd_meta(),
    )
    .unwrap();
    calc.create_xsd_schema_tab("ASchema", schema.node, schema.document)
        .unwrap();
    let _ = xsd::XSD_NS;
    assert_eq!(calc.export_formats("ASchema"), [ExportFormat::XML]);
}
