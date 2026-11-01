//! Tabs of COBOL records and their copybook schema tabs (DESIGN.md, "COBOL
//! copybooks").

use std::sync::Arc;

use cliotree_core::copybook::{self, CodePage};
use cliotree_core::dataformat::DataFormat;
use cliotree_core::{Path, Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, ExportFormat, Meta, SchemaFormat};

const COPYBOOK: &str = "\
      * A customer
       01  CUSTOMER-REC.
           05  CUST-ID        PIC 9(4).
           05  CUST-NAME      PIC X(8).
           05  AREA-TEXT      PIC X(4).
           05  AREA-NUMBER REDEFINES AREA-TEXT PIC 9(4).
";

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn record(id: &str, name: &str, area: &str) -> Vec<u8> {
    let page = CodePage::Cp037;
    let mut r = page.encode(id).unwrap();
    r.extend(page.encode(&format!("{name:<8}")).unwrap());
    r.extend(page.encode(area).unwrap());
    r
}

/// A copybook schema tab and a tab of one record bound to it.
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    let tree = copybook::to_node(&copybook::parse(COPYBOOK).unwrap(), CodePage::Cp037);
    calc.create_copybook_schema_tab("Customer", tree).unwrap();
    let bytes = record("0002", "Bo", "ABCD");
    let spec = calc.schema_spec("Customer").unwrap();
    let layout = spec.copybook().unwrap();
    let data = copybook::records::read(&bytes, layout, &[]).unwrap().node;
    calc.create_data_tab("People", "Customer", data, None)
        .unwrap();
    calc
}

fn at(calc: &Calc, path: &str) -> Value {
    calc.data("People")
        .unwrap()
        .get(&p(path))
        .unwrap()
        .value()
        .clone()
}

#[test]
fn a_copybook_schema_tab_types_its_records() {
    let calc = book();
    assert_eq!(calc.schema_format("Customer"), Some(SchemaFormat::Copybook));
    assert_eq!(calc.format("Customer"), Some(DataFormat::Json5));
    assert!(calc.format_has_comments("Customer"));
    assert_eq!(calc.format("People"), Some(DataFormat::Blk));
    assert_eq!(calc.formats_for("People"), [DataFormat::Blk]);
    assert_eq!(at(&calc, "CUST_NAME"), Value::Str("Bo      ".into()));
    assert_eq!(calc.records_bytes("People").unwrap().len(), 16);
    assert!(calc.field_places("People", &p("CUST_NAME")).is_err());
}

#[test]
fn a_json_tab_cannot_be_bound_to_a_copybook() {
    let mut calc = book();
    let schema = r#"{"type": "array", "items": {"type": "string"}}"#;
    calc.add_sheet(
        "Plain",
        jsonschema::lower_str(schema).unwrap(),
        json::parse("[]").unwrap().node,
    )
    .unwrap();
    let why = calc.rebind_problem("Plain", "Customer").unwrap();
    assert!(why.contains("JSON data"), "{why}");
}

#[test]
fn showing_another_layout_reads_the_same_bytes() {
    let mut calc = book();
    let groups = calc.redefine_groups("People");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].names, ["AREA-TEXT", "AREA-NUMBER"]);
    assert_eq!(groups[0].shown, 0);
    assert!(calc.redefine_formulas("People", 0).is_empty());

    // Letters are no number: an error that writes back as they were.
    calc.set_redefine("People", 0, 1).unwrap();
    assert!(matches!(at(&calc, "AREA_NUMBER"), Value::Error(_)));
    assert!(calc.data("People").unwrap().get(&p("AREA_TEXT")).is_none());
    assert_eq!(calc.redefine_choices("People"), [1]);
    assert_eq!(
        calc.records_bytes("People").unwrap()[12..],
        CodePage::Cp037.encode("ABCD").unwrap()[..]
    );
    calc.undo().unwrap();
    assert_eq!(calc.redefine_choices("People"), Vec::<usize>::new());
    assert_eq!(at(&calc, "AREA_TEXT"), Value::Str("ABCD".into()));

    // A formula in the layout shown is deleted by the change.
    calc.set_formula("People", &p("AREA_TEXT"), "'4321'")
        .unwrap();
    calc.recalc(&PythonEngine::new());
    assert_eq!(calc.redefine_formulas("People", 0), [p("AREA_TEXT")]);
    calc.set_redefine("People", 0, 1).unwrap();
    assert_eq!(at(&calc, "AREA_NUMBER"), Value::Int(4321));
    assert!(calc.formula_text("People", &p("AREA_TEXT")).is_none());
    calc.undo().unwrap();
    assert!(calc.formula_text("People", &p("AREA_TEXT")).is_some());
    assert!(calc.set_redefine("People", 0, 5).is_err());
}

#[test]
fn records_and_their_copybook_are_saved_and_read_back() {
    let mut calc = book();
    calc.set_redefine("People", 0, 1).unwrap();
    let package = calc.to_package(&Meta::default()).unwrap();
    assert_eq!(
        package.get("sheets/People/data.blk").map(<[u8]>::len),
        Some(16)
    );
    assert!(package.get("sheets/Customer/data.json5").is_some());
    let opened = Calc::from_package(Arc::new(PythonSyntax), &package).unwrap();
    let back = opened.calc;
    assert_eq!(back.redefine_choices("People"), [1]);
    assert_eq!(back.format("People"), Some(DataFormat::Blk));
    assert_eq!(back.data("People"), calc.data("People"));
    assert_eq!(back.data("Customer"), calc.data("Customer"));
    assert_eq!(
        back.data("Customer")
            .unwrap()
            .get(&p("items[0]"))
            .unwrap()
            .comment(),
        Some("A customer")
    );
}

#[test]
fn a_record_that_cannot_be_written_is_saved_as_json() {
    let mut calc = book();
    calc.set_value("People", &p("CUST_NAME"), Value::Str("far too long".into()))
        .unwrap();
    assert!(
        calc.records_bytes("People")
            .unwrap_err()
            .contains("CUST-NAME")
    );
    let package = calc.to_package(&Meta::default()).unwrap();
    assert!(package.get("sheets/People/data.blk").is_none());
    assert!(package.get("sheets/People/data.json").is_some());
    let back = Calc::from_package(Arc::new(PythonSyntax), &package)
        .unwrap()
        .calc;
    assert_eq!(back.format("People"), Some(DataFormat::Blk));
    assert_eq!(back.data("People"), calc.data("People"));
}

#[test]
fn records_export_as_blk_and_the_copybook_as_cpy() {
    let calc = book();
    assert_eq!(calc.export_formats("Customer"), [ExportFormat::Copybook]);
    let text = calc
        .export_text("Customer", ExportFormat::Copybook, true)
        .unwrap();
    assert!(text.contains("       01  CUSTOMER-REC.\n"), "{text}");
    assert!(text.contains("      * A customer\n"), "{text}");
    assert_eq!(
        calc.export_formats("People"),
        [ExportFormat::Data(DataFormat::Blk)]
    );
    let bytes = calc
        .export_bytes("People", ExportFormat::Data(DataFormat::Blk), true)
        .unwrap();
    assert_eq!(bytes, record("0002", "Bo", "ABCD"));
    assert!(
        calc.export_text("People", ExportFormat::Data(DataFormat::Blk), true)
            .is_err()
    );
}

/// A tab of COBOL records bound to a schema tab of `text`, holding the
/// record that `bytes` is.
fn book_of(text: &str, bytes: &[u8]) -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    let tree = copybook::to_node(&copybook::parse(text).unwrap(), CodePage::Cp037);
    calc.create_copybook_schema_tab("Book", tree).unwrap();
    let spec = calc.schema_spec("Book").unwrap();
    let data = copybook::records::read(bytes, spec.copybook().unwrap(), &[])
        .unwrap()
        .node;
    calc.create_data_tab("People", "Book", data, None).unwrap();
    calc
}

#[test]
fn a_tab_can_hold_another_record_of_the_same_length() {
    let text = format!(
        "{COPYBOOK}       01  OTHER-REC.\n           05  WHOLE PIC X(16).\n       01  SHORT-REC.\n           05  S PIC X(2).\n"
    );
    let mut calc = book_of(&text, &record("0002", "Bo", "ABCD"));
    assert_eq!(calc.record_of("People").as_deref(), Some("CUSTOMER-REC"));
    let problem = calc.record_problem("People", "SHORT-REC").unwrap();
    assert!(problem.contains("2 bytes"), "{problem}");
    assert!(calc.set_record("People", "SHORT-REC").is_err());
    assert!(calc.record_problem("People", "OTHER-REC").is_none());

    calc.set_formula("People", &p("CUST_NAME"), "'Cy'").unwrap();
    calc.recalc(&PythonEngine::new());
    calc.set_record("People", "OTHER-REC").unwrap();
    assert_eq!(calc.record_of("People").as_deref(), Some("OTHER-REC"));
    assert_eq!(at(&calc, "WHOLE"), Value::Str("0002Cy      ABCD".into()));
    assert!(calc.formula_text("People", &p("CUST_NAME")).is_none());
    // Saved and opened, it holds the record still.
    let package = calc.to_package(&Meta::default()).unwrap();
    let back = Calc::from_package(Arc::new(PythonSyntax), &package)
        .unwrap()
        .calc;
    assert_eq!(back.record_of("People").as_deref(), Some("OTHER-REC"));

    calc.undo().unwrap();
    assert_eq!(calc.record_of("People").as_deref(), Some("CUSTOMER-REC"));
    assert_eq!(at(&calc, "CUST_NAME"), Value::Str("Cy".into()));
    assert!(calc.formula_text("People", &p("CUST_NAME")).is_some());
}

#[test]
fn a_variable_length_record_is_saved_after_its_record_descriptor_word() {
    let text = "       01  R.\n           05  N PIC 9.\n           05  T PIC X OCCURS 0 TO 5 DEPENDING ON N.\n";
    let file = [0x00, 0x07, 0x00, 0x00, 0xF2, 0xC1, 0xC2];
    let mut calc = book_of(text, &file);
    assert!(!calc.is_variable("People"));
    assert_eq!(calc.records_bytes("People").unwrap().len(), 6);
    calc.set_variable("People", true).unwrap();
    assert_eq!(calc.records_bytes("People").unwrap(), file);
    let package = calc.to_package(&Meta::default()).unwrap();
    assert_eq!(package.get("sheets/People/data.blk"), Some(&file[..]));
    let back = Calc::from_package(Arc::new(PythonSyntax), &package)
        .unwrap()
        .calc;
    assert!(back.is_variable("People"));
    assert_eq!(back.data("People"), calc.data("People"));
    calc.undo().unwrap();
    assert!(!calc.is_variable("People"));
}

#[test]
fn a_renames_is_shown_as_another_layout() {
    let text = format!("{COPYBOOK}       66  WHO RENAMES CUST-ID THRU CUST-NAME.\n");
    let mut calc = book_of(&text, &record("0002", "Bo", "ABCD"));
    let groups = calc.redefine_groups("People");
    assert_eq!(groups[1].names, ["CUST-ID THRU CUST-NAME", "WHO"]);
    calc.set_redefine("People", 1, 1).unwrap();
    assert_eq!(at(&calc, "WHO"), Value::Str("0002Bo      ".into()));
    let fields: Vec<String> = match calc.data("People").unwrap().value() {
        Value::Object(fields) => fields.keys().cloned().collect(),
        _ => Vec::new(),
    };
    assert_eq!(fields, ["WHO", "AREA_TEXT"]);
    calc.set_value("People", &p("WHO"), Value::Str("0003Al".into()))
        .unwrap();
    calc.set_redefine("People", 1, 0).unwrap();
    assert_eq!(at(&calc, "CUST_ID"), Value::Int(3));
    assert_eq!(at(&calc, "CUST_NAME"), Value::Str("Al      ".into()));
}
