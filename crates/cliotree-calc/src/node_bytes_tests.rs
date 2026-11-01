//! Bytes in formulas: `RAW`, the four conversion functions, and bytes read
//! as the node whose formula gave them (DESIGN.md, "Raw memory").

use std::sync::Arc;

use cliotree_core::copybook::{self, CodePage};
use cliotree_core::{Path, Value, json, jsonschema, xml, xsd};

use crate::Calc;
use crate::python::{PythonEngine, PythonSyntax};

const COPYBOOK: &str = "\
       01  CUSTOMER-REC.
           05  CUST-ID        PIC 9(4).
           05  CUST-NAME      PIC X(8).
           05  BALANCE        PIC S9(3)V99 COMP-3.
";

const INBOX: &str = r#"{"type": "object", "properties": {
  "payload": {"type": "string"},
  "a": {"type": "string"},
  "b": {"type": "string"},
  "obj": {"type": "object", "properties": {"n": {"type": "integer"}}},
  "count": {"type": "integer"},
  "rows": {"type": "array", "items": {"type": "object",
    "properties": {"v": {"type": "integer"}, "w": {"type": "integer"}}}}
}}"#;

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="customer" type="xs:string"/>
        <xs:element name="total" type="xs:integer"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

fn record(id: &str, name: &str) -> Vec<u8> {
    let page = CodePage::Cp037;
    let mut r = page.encode(id).unwrap();
    r.extend(page.encode(&format!("{name:<8}")).unwrap());
    // +12.34, packed.
    r.extend([0x01, 0x23, 0x4C]);
    r
}

/// A tab of one COBOL record, People, and a JSON tab, Inbox.
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    let tree = copybook::to_node(&copybook::parse(COPYBOOK).unwrap(), CodePage::Cp037);
    calc.create_copybook_schema_tab("Customer", tree).unwrap();
    let spec = calc.schema_spec("Customer").unwrap();
    let layout = spec.copybook().unwrap();
    let data = copybook::records::read(&record("0002", "Bo"), layout, &[])
        .unwrap()
        .node;
    calc.create_data_tab("People", "Customer", data, None)
        .unwrap();
    let inbox = json::parse(r#"{"payload": "", "rows": [{"v": 1}, {"v": 2}]}"#)
        .unwrap()
        .node;
    calc.add_sheet("Inbox", jsonschema::lower_str(INBOX).unwrap(), inbox)
        .unwrap();
    calc
}

fn at(calc: &Calc, sheet: &str, path: &str) -> Value {
    calc.data(sheet)
        .unwrap()
        .get(&p(path))
        .unwrap_or_else(|| panic!("nothing at {sheet}.{path}"))
        .value()
        .clone()
}

fn error_at(calc: &Calc, sheet: &str, path: &str) -> String {
    match at(calc, sheet, path) {
        Value::Error(e) => e.message,
        other => panic!("{sheet}.{path} holds {other:?}, not an error"),
    }
}

fn run(calc: &mut Calc, formulas: &[(&str, &str, &str)]) {
    for (sheet, path, text) in formulas {
        calc.set_formula(sheet, &p(path), text).unwrap();
    }
    calc.recalc(&PythonEngine::new());
}

#[test]
fn raw_gives_a_cobol_fields_own_bytes_and_the_record_for_the_root() {
    let mut calc = book();
    run(
        &mut calc,
        &[
            ("Inbox", "a", "HEXENCODE(RAW(People.CUST_ID))"),
            ("Inbox", "b", "HEXENCODE(raw(People.BALANCE))"),
            ("Inbox", "payload", "BASE64ENCODE(RAW(People))"),
        ],
    );
    assert_eq!(at(&calc, "Inbox", "a"), Value::Str("F0F0F0F2".into()));
    assert_eq!(at(&calc, "Inbox", "b"), Value::Str("01234C".into()));
    let whole = cliotree_core::binary::to_base64(&record("0002", "Bo"));
    assert_eq!(at(&calc, "Inbox", "payload"), Value::Str(whole));
}

#[test]
fn bytes_a_formula_gives_are_read_as_its_cobol_field_or_record() {
    let mut calc = book();
    run(
        &mut calc,
        &[
            // "Ann" in EBCDIC, padded with spaces.
            (
                "People",
                "CUST_NAME",
                "HEXDECODE('C1 95 95 40 40 40 40 40')",
            ),
            ("People", "BALANCE", "HEXDECODE('00500D')"),
        ],
    );
    assert_eq!(
        at(&calc, "People", "CUST_NAME"),
        Value::Str("Ann     ".into())
    );
    assert_eq!(at(&calc, "People", "BALANCE"), Value::Float(-5.0));

    // The whole record from another tab: its detail is computed.
    let mut calc = book();
    let payload = cliotree_core::binary::to_base64(&record("0007", "Cy"));
    calc.set_formula("Inbox", &p("payload"), &format!("'{payload}'"))
        .unwrap();
    run(&mut calc, &[("People", "", "BASE64DECODE(Inbox.payload)")]);
    assert_eq!(at(&calc, "People", "CUST_ID"), Value::Int(7));
    assert_eq!(
        at(&calc, "People", "CUST_NAME"),
        Value::Str("Cy      ".into())
    );
    assert_eq!(calc.records_bytes("People").unwrap(), record("0007", "Cy"));
}

#[test]
fn bytes_of_the_wrong_length_or_kind_give_an_error() {
    let mut calc = book();
    run(
        &mut calc,
        &[
            ("People", "CUST_ID", "HEXDECODE('F0')"),
            ("Inbox", "count", "b'\\xff'"),
            ("Inbox", "a", "BASE64DECODE('not base64!')"),
            ("Inbox", "b", "HEXENCODE('text')"),
        ],
    );
    let why = error_at(&calc, "People", "CUST_ID");
    assert!(
        why.contains("gave 1 bytes, and this field takes 4"),
        "{why}"
    );
    let why = error_at(&calc, "Inbox", "count");
    assert!(why.contains("not UTF-8"), "{why}");
    let why = error_at(&calc, "Inbox", "a");
    assert!(why.contains("not Base64"), "{why}");
    let why = error_at(&calc, "Inbox", "b");
    assert!(why.contains("takes bytes, not str"), "{why}");
}

#[test]
fn in_other_tabs_bytes_are_text_of_the_tabs_format() {
    let mut calc = book();
    run(
        &mut calc,
        &[
            ("Inbox", "obj", "b'{\"n\": 5}'"),
            ("Inbox", "count", "'42'.encode()"),
            ("Inbox", "a", "RAW(Inbox.obj).decode()"),
            ("Inbox", "b", "RAW(Inbox.count).decode()"),
        ],
    );
    assert_eq!(at(&calc, "Inbox", "obj.n"), Value::Int(5));
    assert_eq!(at(&calc, "Inbox", "count"), Value::Int(42));
    let Value::Str(text) = at(&calc, "Inbox", "a") else {
        panic!()
    };
    assert_eq!(
        json::parse(&text)
            .unwrap()
            .node
            .get(&p("n"))
            .unwrap()
            .value(),
        &Value::Int(5)
    );
    assert_eq!(at(&calc, "Inbox", "b"), Value::Str("42".into()));

    // A value of the wrong type is checked as any result is.
    run(&mut calc, &[("Inbox", "count", "b'many'")]);
    let why = error_at(&calc, "Inbox", "count");
    assert!(why.contains("whole number"), "{why}");
}

#[test]
fn an_xml_tab_reads_and_writes_its_document() {
    let mut calc = book();
    let spec = xsd::lower_str(XSD).unwrap();
    let doc = "<order><customer>Ada</customer><total>3</total></order>";
    let node = xml::parse(doc, &spec).unwrap().node;
    calc.add_sheet("Order", spec, node).unwrap();
    run(
        &mut calc,
        &[
            ("Inbox", "a", "RAW(Order).decode()"),
            ("Order", "total", "b' 12 '"),
        ],
    );
    assert_eq!(at(&calc, "Order", "total"), Value::Int(12));
    let Value::Str(text) = at(&calc, "Inbox", "a") else {
        panic!()
    };
    assert!(text.contains("<customer>Ada</customer>"), "{text}");
    assert!(text.contains("<total>12</total>"), "{text}");
}

#[test]
fn a_column_formula_cannot_give_bytes() {
    let mut calc = book();
    calc.set_column_formula("Inbox", &p("rows"), "w", "b'3'")
        .unwrap();
    calc.recalc(&PythonEngine::new());
    let why = error_at(&calc, "Inbox", "rows[0].w");
    assert!(why.contains("column formula cannot give bytes"), "{why}");
    // A formula of the field's own is read as the field.
    run(&mut calc, &[("Inbox", "rows[1].w", "b'4'")]);
    assert_eq!(at(&calc, "Inbox", "rows[1].w"), Value::Int(4));
}

#[test]
fn raw_of_something_that_is_not_a_node_is_refused() {
    let mut calc = book();
    run(&mut calc, &[("Inbox", "a", "HEXENCODE(RAW(1 + 2))")]);
    let why = error_at(&calc, "Inbox", "a");
    assert!(why.contains("RAW takes a node"), "{why}");
}

#[test]
fn an_expression_on_a_control_cannot_give_bytes() {
    use cliotree_core::form::{Control, ControlKind, FormDesign};

    let mut calc = book();
    let mut control = Control::bound(ControlKind::Field, &p("count"));
    control.visible_when = Some("RAW(Inbox.rows)".into());
    calc.forms_mut().set(
        "Inbox",
        "Entry",
        FormDesign {
            controls: vec![control],
            ..FormDesign::default()
        },
    );
    calc.sync_expressions();
    calc.recalc(&PythonEngine::new());
    let why = calc
        .control_expression("Inbox", "Entry", &[0], crate::ExprKind::Visible)
        .and_then(|s| s.result)
        .unwrap()
        .unwrap_err();
    assert!(why.contains("only a formula on a node can give"), "{why}");
}
