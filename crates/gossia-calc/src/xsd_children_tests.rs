//! Tests of the field commands on an ordered list of child elements
//! (`#children`, DESIGN.md, XML Schemas and namespaces, 0.5.0, step 15c-2).

use std::sync::Arc;

use gossia_core::{Path, Value, metaschema, xml, xsd};

use crate::fake::FakeEngine;
use crate::{Calc, ExportFormat, FieldKind};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="svg">
    <xs:complexType>
      <xs:choice minOccurs="0" maxOccurs="unbounded">
        <xs:element name="rect">
          <xs:complexType>
            <xs:attribute name="w" type="xs:integer"/>
          </xs:complexType>
        </xs:element>
        <xs:element name="title" type="xs:string"/>
      </xs:choice>
      <xs:attribute name="id" type="xs:string"/>
    </xs:complexType>
  </xs:element>
</xs:schema>
"#;

const DATA: &str = r#"<svg id="s"><rect w="1"/><title>T</title><rect w="2"/></svg>"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

/// `Pic` is bound to `Drawing`, an XSD; auto-update is on.
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(FakeEngine::new()));
    let schema = xml::parse(XSD, metaschema::xsd_meta()).unwrap();
    calc.add_xsd_sheet("Drawing", schema.node, schema.document)
        .unwrap();
    let spec = xsd::lower_str(XSD).unwrap();
    let data = xml::parse(DATA, &spec).unwrap();
    calc.add_sheet("Pic", spec, data.node).unwrap();
    calc.set_xml_document("Pic", data.document).unwrap();
    calc.bind_schema("Pic", "Drawing").unwrap();
    calc.set_auto_update("Drawing", true).unwrap();
    calc.clear_history();
    calc
}

/// The elements an item of the list can be.
fn elements(calc: &Calc) -> Vec<String> {
    let spec = calc.spec("Pic").unwrap();
    let list = spec.resolve(&p("[\"#children\"]")).unwrap().ty;
    spec.list_choices(list)
        .unwrap()
        .iter()
        .map(|f| f.name.clone())
        .collect()
}

fn xsd_text(calc: &Calc) -> String {
    calc.export_text("Drawing", ExportFormat::XML, false)
        .unwrap()
}

/// The data written as XML, read back against the schema written as XSD.
fn data_text(calc: &Calc) -> String {
    let text = calc.export_text("Pic", ExportFormat::XML, false).unwrap();
    let spec = xsd::lower_str(&xsd_text(calc)).unwrap();
    xml::parse(&text, &spec).unwrap();
    text
}

fn error(result: Result<(), crate::EditError>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn an_element_added_to_an_element_with_a_list_goes_into_the_choice() {
    let mut calc = book();
    calc.add_field("Pic", &Path::root(), "circle", FieldKind::Text, true)
        .unwrap();
    assert_eq!(elements(&calc), ["rect", "title", "circle"]);
    let text = xsd_text(&calc);
    assert!(
        text.contains(r#"<xs:element name="circle" type="xs:string"/></xs:choice>"#),
        "{text}"
    );
    // An attribute is the element's own.
    calc.add_field("Pic", &Path::root(), "@lang", FieldKind::Text, false)
        .unwrap();
    assert!(
        xsd_text(&calc).contains(r#"<xs:attribute name="lang" type="xs:string"/>"#),
        "{}",
        xsd_text(&calc)
    );
    // From an item, the same.
    calc.add_field(
        "Pic",
        &p("[\"#children\"][0]"),
        "g",
        FieldKind::Object,
        false,
    )
    .unwrap();
    assert_eq!(elements(&calc), ["rect", "title", "circle", "g"]);
    calc.undo().unwrap();
    assert_eq!(elements(&calc), ["rect", "title", "circle"]);
}

#[test]
fn renaming_an_element_of_a_list_renames_it_in_every_item() {
    let mut calc = book();
    calc.rename_field("Pic", &p("[\"#children\"][1]"), "title", "caption")
        .unwrap();
    assert_eq!(elements(&calc), ["rect", "caption"]);
    assert_eq!(
        calc.data("Pic")
            .unwrap()
            .get(&p("[\"#children\"][1].caption"))
            .unwrap()
            .value(),
        &Value::Str("T".into())
    );
    assert!(
        data_text(&calc)
            .trim_end()
            .ends_with(r#"<svg id="s"><rect w="1"/><caption>T</caption><rect w="2"/></svg>"#),
        "{}",
        data_text(&calc)
    );
}

#[test]
fn deleting_an_element_of_a_list_deletes_the_items_that_held_it() {
    let mut calc = book();
    assert_eq!(
        calc.delete_field_impact("Pic", &p("[\"#children\"][0]"), "rect")
            .unwrap(),
        (2, 0)
    );
    calc.delete_field("Pic", &p("[\"#children\"][0]"), "rect")
        .unwrap();
    assert_eq!(elements(&calc), ["title"]);
    assert!(
        data_text(&calc)
            .trim_end()
            .ends_with(r#"<svg id="s"><title>T</title></svg>"#),
        "{}",
        data_text(&calc)
    );
    let why = error(calc.delete_field("Pic", &p("[\"#children\"][0]"), "title"));
    assert!(why.contains("at least one element"), "{why}");
    calc.undo().unwrap();
    assert_eq!(elements(&calc), ["rect", "title"]);
    assert!(
        data_text(&calc).trim_end().ends_with(DATA),
        "{}",
        data_text(&calc)
    );
}

#[test]
fn the_elements_of_a_list_can_be_reordered() {
    let mut calc = book();
    let order = ["title".to_owned(), "rect".to_owned()];
    calc.reorder_fields("Pic", &p("[\"#children\"][0]"), &order)
        .unwrap();
    assert_eq!(elements(&calc), ["title", "rect"]);
    // The items keep their order.
    assert!(
        data_text(&calc).trim_end().ends_with(DATA),
        "{}",
        data_text(&calc)
    );
    // The list itself stays after the attributes.
    let why = error(calc.reorder_fields(
        "Pic",
        &Path::root(),
        &["#children".to_owned(), "@id".to_owned()],
    ));
    assert!(why.contains("attributes"), "{why}");
}

#[test]
fn what_a_list_cannot_take_is_refused() {
    let mut calc = book();
    let item = p("[\"#children\"][0]");
    let why = error(calc.rename_field("Pic", &Path::root(), "#children", "kids"));
    assert!(why.contains("ordered list"), "{why}");
    let why = error(calc.delete_field("Pic", &Path::root(), "#children"));
    assert!(why.contains("ordered list"), "{why}");
    let why = error(calc.add_field("Pic", &item, "rows", FieldKind::Array, false));
    assert!(why.contains("occurs once"), "{why}");
    let why = error(calc.add_field("Pic", &item, "@x", FieldKind::Text, false));
    assert!(why.contains("no attributes"), "{why}");
    let why = error(calc.rename_field("Pic", &item, "rect", "@rect"));
    assert!(why.contains("no attributes"), "{why}");
}
