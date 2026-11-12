//! Tests of the entries a workbook is saved as and read back from (DESIGN.md,
//! "File format"), without a zip or a file.

use std::collections::BTreeMap;
use std::sync::Arc;

use gossia_core::form::generate;
use gossia_core::{ErrorKind, Path, Value, json, jsonschema, metaschema, xml, xsd};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, Meta, Opened, Package, PackageError, SchemaFormat};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "customer": {"type": "string"},
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "qty": {"type": "integer"},
          "price": {"type": "number"},
          "total": {"type": "number"}
        },
        "required": ["qty", "price"]
      }
    },
    "sum": {"type": "number"},
    "ratio": {"type": "number"}
  },
  "required": ["customer", "items"]
}"#;

const DATA: &str = r#"{
  "customer": "Acme",
  "items": [{"qty": 2, "price": 1.5}, {"qty": 3, "price": 2.0}]
}"#;

const XSD: &str = r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="note">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="to" type="xs:string"/>
        <xs:element name="size" type="xs:integer" minOccurs="0"/>
      </xs:sequence>
      <xs:attribute name="id" type="xs:integer"/>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

const NOTE: &str = r#"<note id="7"><to>Ward</to><size>3</size></note>"#;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn syntax() -> Arc<dyn crate::Syntax> {
    Arc::new(PythonSyntax)
}

fn meta() -> Meta {
    Meta {
        workbook_id: "0123456789abcdef".to_owned(),
        app_version: "0.1.0".to_owned(),
        ..Meta::default()
    }
}

/// Two tabs of JSON and one of XML, with the schema tab of each, formulas, a
/// customised form and a module. The tab "Orders" comes before its schema tab.
fn book() -> Calc {
    let mut calc = Calc::new(syntax());
    calc.add_sheet(
        "Orders",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.add_schema_sheet("OrdersSchema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.bind_schema("Orders", "OrdersSchema").unwrap();

    let schema = xml::parse(XSD, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet("NoteSchema", schema.node, schema.document)
        .unwrap();
    let spec = xsd::lower_str(XSD).unwrap();
    let note = xml::parse(NOTE, &spec).unwrap();
    calc.add_sheet("Note", spec, note.node).unwrap();
    calc.set_xml_document("Note", note.document).unwrap();
    calc.bind_schema("Note", "NoteSchema").unwrap();

    for i in 0..2 {
        calc.set_formula(
            "Orders",
            &p(&format!("items[{i}].total")),
            "parent.qty * parent.price",
        )
        .unwrap();
    }
    calc.set_formula("Orders", &p("sum"), "sum(Orders.items.total)")
        .unwrap();
    calc.add_module("util", "def double(x):\n    return x * 2\n")
        .unwrap();
    let design = generate(calc.spec("Orders").unwrap());
    calc.forms_mut().set("Orders", "Packing", design);
    calc.set_auto_update("OrdersSchema", true).unwrap();
    calc
}

fn run(calc: &mut Calc) {
    calc.recalc(&PythonEngine::new());
}

fn saved(calc: &Calc) -> Package {
    calc.to_package(&meta()).unwrap()
}

fn open(package: &Package) -> Opened {
    Calc::from_package(syntax(), package).unwrap()
}

fn text(package: &Package, path: &str) -> String {
    String::from_utf8(package.get(path).unwrap().to_vec()).unwrap()
}

fn data_text(calc: &Calc, tab: &str) -> String {
    if calc.is_xml(tab) {
        xml::to_string(
            calc.data(tab).unwrap(),
            calc.spec(tab).unwrap(),
            &calc.xml_document(tab).cloned().unwrap_or_default(),
            true,
        )
        .unwrap()
    } else {
        json::to_string(calc.data(tab).unwrap(), true).unwrap()
    }
}

#[test]
fn a_workbook_is_the_same_after_it_is_saved_and_opened() {
    let mut calc = book();
    run(&mut calc);
    let opened = open(&saved(&calc));
    let back = &opened.calc;

    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert_eq!(opened.workbook_id, "0123456789abcdef");
    let tabs: Vec<_> = back.sheet_names().collect();
    assert_eq!(tabs, ["Orders", "OrdersSchema", "NoteSchema", "Note"]);
    for tab in &tabs {
        assert_eq!(data_text(back, tab), data_text(&calc, tab), "{tab}");
        assert_eq!(back.formulas(tab), calc.formulas(tab), "{tab}");
        assert_eq!(back.schema_of(tab), calc.schema_of(tab), "{tab}");
        assert_eq!(back.is_xml(tab), calc.is_xml(tab), "{tab}");
    }
    assert!(back.is_xml("Note") && back.is_xml("NoteSchema"));
    assert_eq!(back.schema_format("NoteSchema"), Some(SchemaFormat::Xsd));
    assert_eq!(back.schema_format("OrdersSchema"), Some(SchemaFormat::Json));
    assert!(back.auto_update("OrdersSchema"));
    assert!(!back.auto_update("NoteSchema"));
    assert_eq!(
        back.module_source("util"),
        Some("def double(x):\n    return x * 2\n")
    );
    assert_eq!(back.forms().names("Orders"), ["Packing"]);
    assert_eq!(
        back.forms().get("Orders", "Packing"),
        calc.forms().get("Orders", "Packing")
    );
    // The computed values are what was saved.
    assert_eq!(
        back.data("Orders")
            .unwrap()
            .get(&p("sum"))
            .map(|n| n.value().clone()),
        Some(Value::Float(9.0))
    );
    // Nothing is out of date, to undo, or unsaved.
    assert_eq!(back.pending(), 0);
    assert_eq!(back.history_len(), (0, 0));
    assert!(!back.is_modified());
}

#[test]
fn saving_what_has_not_changed_gives_the_same_bytes() {
    let mut calc = book();
    run(&mut calc);
    let first = saved(&calc);
    assert!(first == saved(&calc), "saving twice gave different bytes");
    // Reading and writing again changes nothing either.
    let opened = open(&first);
    let again = opened
        .calc
        .to_package(&Meta {
            unknown: opened.unknown.clone(),
            views: opened.views.clone(),
            ..meta()
        })
        .unwrap();
    assert!(
        first == again,
        "reading and writing again changed the bytes"
    );
}

#[test]
fn the_entries_come_in_a_fixed_order_manifest_first() {
    let mut calc = book();
    run(&mut calc);
    let package = saved(&calc);
    let paths: Vec<_> = package.entries().map(|(p, _)| p).collect();
    assert_eq!(
        paths,
        [
            "manifest.json",
            "sheets/Orders/data.json",
            "sheets/Orders/overlay.json",
            "sheets/Orders/forms/Packing.json",
            "sheets/OrdersSchema/data.json",
            "sheets/NoteSchema/data.xml",
            "sheets/Note/data.xml",
            "modules/util.py",
        ]
    );
    // Text is UTF-8 with LF line endings.
    for (path, bytes) in package.entries() {
        let text = std::str::from_utf8(bytes).unwrap();
        assert!(!text.contains('\r'), "{path}");
        assert!(text.ends_with('\n'), "{path}");
    }
}

#[test]
fn the_manifest_says_what_wrote_the_file_and_how_the_tabs_are_bound() {
    let mut calc = book();
    run(&mut calc);
    let manifest: serde_json::Value =
        serde_json::from_str(&text(&saved(&calc), "manifest.json")).unwrap();
    assert_eq!(manifest["format_version"], "1.0");
    assert_eq!(manifest["workbook_id"], "0123456789abcdef");
    assert_eq!(manifest["engine"]["name"], "rustpython");
    assert_eq!(manifest["modules"], serde_json::json!(["util"]));
    let tabs = manifest["tabs"].as_array().unwrap();
    let schemas: Vec<_> = tabs.iter().map(|t| t["schema"].as_str().unwrap()).collect();
    assert_eq!(
        schemas,
        [
            "sheet:OrdersSchema",
            "builtin:jsonschema",
            "builtin:xsd",
            "sheet:NoteSchema"
        ]
    );
    assert_eq!(tabs[0]["forms"], serde_json::json!(["Packing"]));
    assert_eq!(tabs[1]["auto_update"], true);
    assert!(tabs[2].get("auto_update").is_none());
}

#[test]
fn an_error_is_left_out_of_the_data_and_kept_in_the_overlay() {
    let mut calc = book();
    calc.set_formula("Orders", &p("ratio"), "1 / 0").unwrap();
    run(&mut calc);
    let package = saved(&calc);
    assert!(!text(&package, "sheets/Orders/data.json").contains("ratio"));
    let overlay = text(&package, "sheets/Orders/overlay.json");
    assert!(overlay.contains("#DIV/0!"), "{overlay}");

    let back = open(&package).calc;
    let cell = back.cell("Orders", &p("ratio")).unwrap();
    let error = cell.error.expect("the error is restored");
    assert_eq!(error.kind, ErrorKind::DivZero);
    assert!(cell.traceback.is_some());
    assert!(matches!(cell.node.unwrap().value(), Value::Error(_)));
    assert_eq!(
        back.formula_text("Orders", &p("ratio")).as_deref(),
        Some("1 / 0")
    );
}

#[test]
fn a_formula_that_does_not_parse_comes_back_as_a_syntax_error() {
    let mut calc = book();
    calc.set_formula("Orders", &p("ratio"), "1 +").unwrap();
    let back = open(&saved(&calc)).calc;
    let cell = back.cell("Orders", &p("ratio")).unwrap();
    assert_eq!(cell.error.unwrap().kind, ErrorKind::Syntax);
    assert_eq!(
        back.formula_text("Orders", &p("ratio")).as_deref(),
        Some("1 +")
    );
}

#[test]
fn a_formula_saved_out_of_date_is_saved_as_pending_and_not_run_on_opening() {
    let calc = book();
    assert!(calc.pending() > 0);
    let package = saved(&calc);
    assert!(text(&package, "sheets/Orders/overlay.json").contains("pending"));
    let back = open(&package).calc;
    assert_eq!(
        back.pending(),
        0,
        "the caller decides whether to recalculate"
    );
    assert_eq!(back.formulas("Orders").len(), 3);
}

#[test]
fn the_state_of_the_view_is_carried_and_not_interpreted() {
    let mut calc = book();
    run(&mut calc);
    let mut views = BTreeMap::new();
    views.insert(
        "Orders".to_owned(),
        serde_json::json!({"view": "Packing", "tables": ["items"]}),
    );
    let package = calc.to_package(&Meta { views, ..meta() }).unwrap();
    let opened = open(&package);
    assert_eq!(
        opened.views["Orders"],
        serde_json::json!({"view": "Packing", "tables": ["items"]})
    );
    assert!(!opened.views.contains_key("Note"));
}

#[test]
fn the_view_of_the_workbook_is_carried_in_its_own_entry() {
    let mut calc = book();
    run(&mut calc);
    // Nothing to carry: no entry.
    let plain = calc.to_package(&meta()).unwrap();
    assert!(plain.get("view.json").is_none());
    let empty = calc
        .to_package(&Meta {
            workbook_view: Some(serde_json::json!({})),
            ..meta()
        })
        .unwrap();
    assert!(empty.get("view.json").is_none());

    let view = serde_json::json!({"data_font": {"family": "serif", "size": 16}, "later": 1});
    let package = calc
        .to_package(&Meta {
            workbook_view: Some(view.clone()),
            ..meta()
        })
        .unwrap();
    let opened = open(&package);
    assert_eq!(opened.workbook_view, Some(view));
    assert!(opened.unknown.is_empty(), "view.json is known");
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);

    // One that cannot be read opens with the defaults and says so.
    let mut broken = package.clone();
    broken.insert("view.json", b"[1, 2]\n".to_vec()).unwrap();
    let opened = open(&broken);
    assert_eq!(opened.workbook_view, None);
    assert!(opened.warnings.iter().any(|w| w.contains("view.json")));
}

#[test]
fn what_this_version_does_not_know_is_kept_and_handed_back() {
    let mut calc = book();
    run(&mut calc);
    let mut package = saved(&calc);
    package
        .insert("datasources.json", b"{\"sources\": []}\n".to_vec())
        .unwrap();
    package
        .insert("reports/Sales.json", b"{}\n".to_vec())
        .unwrap();
    let opened = open(&package);
    let names: Vec<_> = opened.unknown.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, ["datasources.json", "reports/Sales.json"]);
    let again = opened
        .calc
        .to_package(&Meta {
            unknown: opened.unknown.clone(),
            views: opened.views.clone(),
            ..meta()
        })
        .unwrap();
    assert_eq!(
        again.get("datasources.json"),
        package.get("datasources.json")
    );
    assert_eq!(
        again.get("reports/Sales.json"),
        package.get("reports/Sales.json")
    );
    // They are written after what is known, in path order.
    let paths: Vec<_> = again.entries().map(|(p, _)| p).collect();
    assert_eq!(
        &paths[paths.len() - 2..],
        ["datasources.json", "reports/Sales.json"]
    );
}

#[test]
fn a_newer_major_version_is_refused_and_a_newer_minor_one_warns() {
    let mut calc = book();
    run(&mut calc);
    let package = saved(&calc);
    let with_version = |version: &str| {
        let manifest = text(&package, "manifest.json").replace(
            "\"format_version\": \"1.0\"",
            &format!("\"format_version\": \"{version}\""),
        );
        let mut changed = package.clone();
        changed
            .insert("manifest.json", manifest.into_bytes())
            .unwrap();
        changed
    };
    let error = Calc::from_package(syntax(), &with_version("2.0"))
        .err()
        .unwrap();
    assert_eq!(
        error,
        PackageError::TooNew {
            needed: "2.0".to_owned()
        }
    );
    assert!(error.to_string().contains("2.0"));

    let opened = open(&with_version("1.7"));
    assert!(opened.warnings.iter().any(|w| w.contains("newer version")));
    assert_eq!(opened.format_version, (1, 7));

    let bad = Calc::from_package(syntax(), &with_version("one"))
        .err()
        .unwrap();
    assert!(matches!(bad, PackageError::BadManifest(_)));
}

#[test]
fn a_file_without_a_readable_manifest_is_refused() {
    let empty = Package::new();
    assert_eq!(
        Calc::from_package(syntax(), &empty).err().unwrap(),
        PackageError::NoManifest
    );
    let mut garbage = Package::new();
    garbage
        .insert("manifest.json", b"not json".to_vec())
        .unwrap();
    assert!(matches!(
        Calc::from_package(syntax(), &garbage).err().unwrap(),
        PackageError::BadManifest(_)
    ));
}

#[test]
fn an_entry_path_cannot_reach_outside_the_archive() {
    let mut package = Package::new();
    for path in [
        "",
        "/etc/passwd",
        "../x",
        "a/../x",
        "a//b",
        "a\\b",
        "C:/x",
        "./x",
        "a/\u{1}",
    ] {
        assert!(package.insert(path, vec![]).is_err(), "{path:?}");
    }
    assert!(package.insert("sheets/Orders/data.json", vec![]).is_ok());
}

#[test]
fn a_tab_name_that_is_not_safe_as_a_file_name_is_refused() {
    let mut calc = book();
    run(&mut calc);
    let package = saved(&calc);
    for name in ["../evil", "a/b", ""] {
        let manifest = text(&package, "manifest.json").replace("\"Note\"", &format!("{name:?}"));
        let mut changed = package.clone();
        changed
            .insert("manifest.json", manifest.into_bytes())
            .unwrap();
        let error = Calc::from_package(syntax(), &changed).err().unwrap();
        assert!(
            matches!(error, PackageError::BadManifest(_)),
            "{name:?}: {error}"
        );
    }
    // Two tabs whose names differ only in case are not two tabs.
    let manifest = text(&package, "manifest.json").replace("\"Note\"", "\"orders\"");
    let mut changed = package.clone();
    changed
        .insert("manifest.json", manifest.into_bytes())
        .unwrap();
    let error = Calc::from_package(syntax(), &changed).err().unwrap();
    assert!(error.to_string().contains("names ignore case"), "{error}");
}

#[test]
fn a_tab_that_was_given_its_type_directly_cannot_be_saved() {
    let mut calc = Calc::new(syntax());
    calc.add_sheet(
        "Loose",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    let error = calc.to_package(&meta()).unwrap_err();
    assert!(error.to_string().contains("Loose"), "{error}");
}

#[test]
fn a_missing_data_file_or_schema_tab_says_which_tab() {
    let mut calc = book();
    run(&mut calc);
    let package = saved(&calc);

    let mut without_data = Package::new();
    for (path, bytes) in package.entries() {
        if path != "sheets/Orders/data.json" {
            without_data.insert(path, bytes.to_vec()).unwrap();
        }
    }
    let error = Calc::from_package(syntax(), &without_data).err().unwrap();
    assert!(
        matches!(&error, PackageError::Tab { tab, .. } if tab == "Orders"),
        "{error}"
    );

    let manifest = text(&package, "manifest.json").replace("sheet:OrdersSchema", "sheet:Nowhere");
    let mut changed = package.clone();
    changed
        .insert("manifest.json", manifest.into_bytes())
        .unwrap();
    let error = Calc::from_package(syntax(), &changed).err().unwrap();
    assert!(error.to_string().contains("Nowhere"), "{error}");
}

#[test]
fn a_form_that_cannot_be_read_is_dropped_with_a_warning_and_the_rest_opens() {
    let mut calc = book();
    run(&mut calc);
    let mut package = saved(&calc);
    package
        .insert(
            "sheets/Orders/forms/Packing.json",
            b"{\"nonsense\": true}\n".to_vec(),
        )
        .unwrap();
    let opened = open(&package);
    assert!(
        opened.warnings.iter().any(|w| w.contains("Packing")),
        "{:?}",
        opened.warnings
    );
    assert!(opened.calc.forms().names("Orders").is_empty());
    assert_eq!(opened.calc.formulas("Orders").len(), 3);
}

#[test]
fn a_formula_the_schema_cannot_hold_is_dropped_with_a_warning() {
    let mut calc = book();
    run(&mut calc);
    let package = saved(&calc);
    let overlay = text(&package, "sheets/Orders/overlay.json").replace("\"sum\"", "\"nothing\"");
    let mut changed = package.clone();
    changed
        .insert("sheets/Orders/overlay.json", overlay.into_bytes())
        .unwrap();
    let opened = open(&changed);
    assert!(
        opened.warnings.iter().any(|w| w.contains("nothing")),
        "{:?}",
        opened.warnings
    );
    assert_eq!(opened.calc.formulas("Orders").len(), 2);
}

#[test]
fn a_tab_name_that_breaks_the_naming_rules_opens_with_a_warning() {
    let mut calc = Calc::new(syntax());
    calc.add_schema_sheet("My schema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.add_sheet(
        "My tab",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.bind_schema("My tab", "My schema").unwrap();
    let opened = open(&saved(&calc));
    assert!(
        opened.warnings.iter().any(|w| w.contains("My tab")),
        "{:?}",
        opened.warnings
    );
    // It opens all the same.
    assert_eq!(opened.calc.sheet_names().count(), 2);
}

/// A tab named `plt`, made before Gossia pyplot reserved the name, opens with
/// a warning to rename it, and until then formulas see the tab, not pyplot.
#[test]
fn a_tab_named_plt_from_before_0_6_opens_and_formulas_see_it() {
    let mut calc = Calc::new(syntax());
    calc.add_sheet(
        "plt",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.add_sheet(
        "Orders",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.add_schema_sheet("OrdersSchema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.bind_schema("plt", "OrdersSchema").unwrap();
    calc.bind_schema("Orders", "OrdersSchema").unwrap();
    calc.set_formula("Orders", &p("customer"), "plt.customer + '!'")
        .unwrap();
    run(&mut calc);
    let mut opened = open(&saved(&calc));
    assert!(
        opened
            .warnings
            .iter()
            .any(|w| w.starts_with("Tab plt:") && w.contains("Rename the tab")),
        "{:?}",
        opened.warnings
    );
    // As opening a trusted workbook does.
    opened.calc.recalc_all();
    run(&mut opened.calc);
    let value = |calc: &Calc| {
        calc.cell("Orders", &p("customer"))
            .and_then(|c| c.node)
            .map(|n| n.value().clone())
    };
    assert_eq!(value(&opened.calc), Some(Value::Str("Acme!".into())));
    // A change of the tab runs the formula again, which reads the tab.
    opened
        .calc
        .set_value("plt", &p("customer"), Value::Str("Beta".into()))
        .unwrap();
    run(&mut opened.calc);
    assert_eq!(value(&opened.calc), Some(Value::Str("Beta!".into())));
    // Renamed, the tab hides nothing: `plt` is Gossia pyplot again.
    opened.calc.rename_sheet("plt", "Old").unwrap();
    opened
        .calc
        .set_formula("Orders", &p("customer"), "str(callable(plt.svg))")
        .unwrap();
    run(&mut opened.calc);
    assert_eq!(value(&opened.calc), Some(Value::Str("True".into())));
}

// ------------------------------------------------------------------ export

#[test]
fn an_exported_json_tab_has_its_computed_values_and_no_formulas_or_errors() {
    let mut calc = book();
    calc.set_formula("Orders", &p("ratio"), "1 / 0").unwrap();
    run(&mut calc);
    assert_eq!(
        calc.export_formats("Orders"),
        [
            crate::ExportFormat::JSON,
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::Json5),
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::Jsonc),
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::Toml),
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::Yaml),
            crate::ExportFormat::Csv,
            crate::ExportFormat::Tsv,
            crate::ExportFormat::Xlsx,
            crate::ExportFormat::Ods,
        ],
        "an object is not lines of text, nor JSON Lines; its items are rows"
    );
    let text = calc
        .export_text("Orders", crate::ExportFormat::JSON, true)
        .unwrap();
    assert!(text.ends_with("}\n"), "{text}");
    assert!(text.contains("\n  \"customer\": \"Acme\""), "{text}");
    let back: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(back["items"][0]["total"], 3.0);
    assert_eq!(back["sum"], 9.0);
    // The errored node is absent, and no formula is written.
    assert!(back.get("ratio").is_none(), "{text}");
    assert!(!text.contains("parent.qty"), "{text}");
    // Compact is one line.
    let compact = calc
        .export_text("Orders", crate::ExportFormat::JSON, false)
        .unwrap();
    assert_eq!(compact.lines().count(), 1, "{compact}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&compact).unwrap(),
        back
    );
    // The schema tab beside it, and none beside a schema tab.
    assert_eq!(calc.export_schema("Orders"), Some("OrdersSchema"));
    assert_eq!(calc.export_schema("OrdersSchema"), None);
    let schema = calc
        .export_text("OrdersSchema", crate::ExportFormat::JSON, true)
        .unwrap();
    jsonschema::lower_str(&schema).unwrap();
}

#[test]
fn an_exported_xml_tab_and_its_xsd_read_back() {
    let calc = book();
    assert_eq!(calc.export_format("Note"), Some(crate::ExportFormat::XML));
    assert_eq!(calc.export_formats("Note"), [crate::ExportFormat::XML]);
    assert_eq!(calc.export_schema("Note"), Some("NoteSchema"));
    assert_eq!(
        calc.export_format("NoteSchema"),
        Some(crate::ExportFormat::XML)
    );
    let xsd_text = calc
        .export_text("NoteSchema", crate::ExportFormat::XML, true)
        .unwrap();
    let spec = xsd::lower_str(&xsd_text).unwrap();
    let text = calc
        .export_text("Note", crate::ExportFormat::XML, true)
        .unwrap();
    assert!(text.contains("\n  <to>Ward</to>"), "{text}");
    let back = xml::parse(&text, &spec).unwrap();
    assert_eq!(
        back.node.get(&p("size")).map(|n| n.value().clone()),
        Some(Value::Int(3))
    );
    let compact = calc
        .export_text("Note", crate::ExportFormat::XML, false)
        .unwrap();
    assert!(
        compact.contains("<note id=\"7\"><to>Ward</to>"),
        "{compact}"
    );
    // XML is not written as JSON.
    let error = calc
        .export_text("Note", crate::ExportFormat::JSON, true)
        .unwrap_err();
    assert!(error.contains("XML"), "{error}");
}

#[test]
fn an_array_of_strings_exports_as_lines_of_text() {
    const LINES: &str = r#"{"type": "array", "items": {"type": "string"}}"#;
    let mut calc = Calc::new(syntax());
    calc.add_sheet(
        "Lines",
        jsonschema::lower_str(LINES).unwrap(),
        json::parse(r#"["one", "", "three", "four"]"#).unwrap().node,
    )
    .unwrap();
    calc.set_formula("Lines", &p("[3]"), "1 / 0").unwrap();
    run(&mut calc);
    assert_eq!(
        calc.export_formats("Lines"),
        [
            crate::ExportFormat::JSON,
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::Json5),
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::Jsonc),
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::JsonLines),
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::Yaml),
            crate::ExportFormat::Data(gossia_core::dataformat::DataFormat::YamlDocuments),
            crate::ExportFormat::Text,
        ]
    );
    // The errored item is left out, as it is from JSON.
    assert_eq!(
        calc.export_text("Lines", crate::ExportFormat::Text, true)
            .unwrap(),
        "one\n\nthree\n"
    );
    // What text import reads back is the same array.
    let back = gossia_core::infer::text_lines("one\n\nthree\n").data;
    assert_eq!(
        json::to_string(&back, false).unwrap(),
        r#"["one","","three"]"#
    );
    // A line break inside a string cannot be written as lines.
    calc.set_value("Lines", &p("[0]"), Value::Str("a\nb".into()))
        .unwrap();
    assert!(
        !calc
            .export_formats("Lines")
            .contains(&crate::ExportFormat::Text)
    );
    let error = calc
        .export_text("Lines", crate::ExportFormat::Text, true)
        .unwrap_err();
    assert!(error.contains("line break"), "{error}");
}
