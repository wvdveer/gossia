//! Tests of the field commands on a tab whose schema tab is an XSD (DESIGN.md,
//! "Editing the schema from a data tab", "Attributes and elements").

use std::sync::Arc;

use cliotree_core::{Path, Value, metaschema, xml, xsd};

use crate::fake::FakeEngine;
use crate::{Calc, ExportFormat, FieldKind};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="customer" type="xs:string"/>
        <xs:element name="item" type="Item" maxOccurs="unbounded"/>
        <xs:element name="note" type="Note" minOccurs="0"/>
      </xs:sequence>
      <xs:attribute name="id" type="xs:string" use="required"/>
    </xs:complexType>
  </xs:element>
  <xs:complexType name="Item">
    <xs:sequence>
      <xs:element name="sku" type="xs:string"/>
      <xs:element name="qty" type="xs:integer"/>
    </xs:sequence>
    <xs:attribute name="line" type="xs:integer"/>
  </xs:complexType>
  <xs:complexType name="Note">
    <xs:simpleContent>
      <xs:extension base="xs:string">
        <xs:attribute name="lang" type="xs:string"/>
      </xs:extension>
    </xs:simpleContent>
  </xs:complexType>
</xs:schema>
"#;

const DATA: &str = r#"<order id="7"><customer>Ann</customer><item line="1"><sku>A</sku><qty>2</qty></item><item line="2"><sku>B</sku><qty>3</qty></item><note lang="en">Ring</note></order>"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

/// `Order` is bound to `Order schema`, an XSD; auto-update is on.
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(FakeEngine::new()));
    let schema = xml::parse(XSD, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet("Order schema", schema.node, schema.document)
        .unwrap();
    let spec = xsd::lower_str(XSD).unwrap();
    let data = xml::parse(DATA, &spec).unwrap();
    calc.add_sheet("Order", spec, data.node).unwrap();
    calc.set_xml_document("Order", data.document).unwrap();
    calc.bind_schema("Order", "Order schema").unwrap();
    calc.set_auto_update("Order schema", true).unwrap();
    calc.clear_history();
    calc
}

fn fields(calc: &Calc, object: &str) -> Vec<String> {
    calc.field_places("Order", &p(object)).unwrap().fields
}

fn xsd_text(calc: &Calc) -> String {
    calc.export_text("Order schema", ExportFormat::Xml, false)
        .unwrap()
}

/// The data written as XML, read back against the schema written as XSD:
/// the two still fit each other.
fn data_text(calc: &Calc) -> String {
    let text = calc.export_text("Order", ExportFormat::Xml, false).unwrap();
    let spec = xsd::lower_str(&xsd_text(calc)).unwrap();
    xml::parse(&text, &spec).unwrap();
    text
}

fn error_text(result: Result<(), crate::EditError>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn the_fields_of_an_xml_record_are_its_attributes_then_its_text_then_its_elements() {
    let calc = book();
    assert_eq!(fields(&calc, ""), ["@id", "customer", "item", "note"]);
    assert_eq!(fields(&calc, "item[0]"), ["@line", "sku", "qty"]);
    assert_eq!(fields(&calc, "note"), ["@lang", "#text"]);
}

#[test]
fn an_element_or_an_attribute_is_added_to_the_complex_type() {
    let mut calc = book();
    calc.add_field("Order", &p("item[0]"), "colour", FieldKind::Text, false)
        .unwrap();
    calc.add_field("Order", &p("item[1]"), "@gift", FieldKind::Boolean, true)
        .unwrap();
    assert_eq!(calc.history_len(), (2, 0));
    assert_eq!(
        fields(&calc, "item[0]"),
        ["@line", "@gift", "sku", "qty", "colour"]
    );
    let schema = xsd_text(&calc);
    assert!(
        schema.contains(r#"<xs:element name="colour" type="xs:string" minOccurs="0"/>"#),
        "{schema}"
    );
    assert!(
        schema.contains(r#"<xs:attribute name="gift" type="xs:boolean" use="required"/>"#),
        "{schema}"
    );
    // The new attribute comes after the old one, in the complexType.
    assert!(
        schema.find(r#"name="line""#) < schema.find(r#"name="gift""#),
        "{schema}"
    );
    // A value can be put in the new field, and the file reads back.
    calc.set_value("Order", &p("item[0].colour"), Value::Str("red".into()))
        .unwrap();
    let data = data_text(&calc);
    assert!(data.contains("<qty>2</qty><colour>red</colour>"), "{data}");

    calc.undo().unwrap();
    calc.undo().unwrap();
    calc.undo().unwrap();
    assert_eq!(xsd_text(&calc), xsd_text(&book()));
}

#[test]
fn an_element_that_holds_text_takes_attributes_only() {
    let mut calc = book();
    let why = error_text(calc.add_field("Order", &p("note"), "by", FieldKind::Text, false));
    assert!(why.contains("add an attribute instead"), "{why}");
    assert_eq!(calc.history_len(), (0, 0));
    calc.add_field("Order", &p("note"), "@by", FieldKind::Text, false)
        .unwrap();
    assert_eq!(fields(&calc, "note"), ["@lang", "@by", "#text"]);
    let schema = xsd_text(&calc);
    assert!(
        schema.contains(r#"<xs:attribute name="lang" type="xs:string"/><xs:attribute name="by" type="xs:string"/></xs:extension>"#),
        "{schema}"
    );
}

#[test]
fn a_new_name_must_be_one_xml_allows() {
    let mut calc = book();
    for bad in ["2nd", "a b", "xmlish", "@", "@a:b"] {
        let result = calc.add_field("Order", &p(""), bad, FieldKind::Text, false);
        assert!(result.is_err(), "{bad}");
    }
    assert_eq!(calc.history_len(), (0, 0));
}

#[test]
fn renaming_keeps_an_attribute_an_attribute_and_renames_the_data() {
    let mut calc = book();
    calc.rename_field("Order", &p("item[0]"), "@line", "@no")
        .unwrap();
    calc.rename_field("Order", &p("item[0]"), "qty", "count")
        .unwrap();
    assert_eq!(fields(&calc, "item[0]"), ["@no", "sku", "count"]);
    let data = data_text(&calc);
    assert!(
        data.contains(r#"<item no="2"><sku>B</sku><count>3</count></item>"#),
        "{data}"
    );

    for (from, to) in [("sku", "@sku"), ("@no", "no")] {
        let why = error_text(calc.rename_field("Order", &p("item[0]"), from, to));
        assert!(why.contains("stays an attribute"), "{why}");
    }
    let why = error_text(calc.rename_field("Order", &p("note"), "#text", "body"));
    assert!(why.contains("no name"), "{why}");
    assert_eq!(calc.history_len(), (2, 0));
}

#[test]
fn deleting_an_attribute_removes_its_values_and_undo_brings_them_back() {
    let mut calc = book();
    let before = data_text(&calc);
    assert_eq!(
        calc.delete_field_impact("Order", &p("item[0]"), "@line")
            .unwrap(),
        (2, 0)
    );
    calc.delete_field("Order", &p("item[0]"), "@line").unwrap();
    assert_eq!(fields(&calc, "item[0]"), ["sku", "qty"]);
    let schema = xsd_text(&calc);
    assert!(!schema.contains(r#"name="line""#), "{schema}");
    assert!(!data_text(&calc).contains("line="));

    calc.undo().unwrap();
    assert_eq!(data_text(&calc), before);

    let why = error_text(calc.delete_field("Order", &p("note"), "#text"));
    assert!(why.contains("text of an element"), "{why}");
}

#[test]
fn elements_move_among_elements_and_attributes_among_attributes() {
    let mut calc = book();
    let order = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    calc.reorder_fields("Order", &p("item[0]"), &order(&["@line", "qty", "sku"]))
        .unwrap();
    assert_eq!(fields(&calc, "item[0]"), ["@line", "qty", "sku"]);
    let data = data_text(&calc);
    assert!(data.contains("<qty>3</qty><sku>B</sku>"), "{data}");

    let why =
        error_text(calc.reorder_fields("Order", &p("item[0]"), &order(&["qty", "@line", "sku"])));
    assert!(why.contains("among the attributes"), "{why}");
    assert_eq!(calc.history_len(), (1, 0));
}

#[test]
fn a_named_type_changes_everywhere_it_is_used() {
    let mut calc = book();
    let places = calc.field_places("Order", &p("item[0]")).unwrap();
    assert_eq!(places.objects.len(), 2);
    calc.add_field("Order", &p("item[0]"), "@x", FieldKind::Integer, false)
        .unwrap();
    assert_eq!(fields(&calc, "item[1]"), ["@line", "@x", "sku", "qty"]);
}
