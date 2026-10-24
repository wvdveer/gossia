//! Tests of XML tabs through the calculation layer and the real engine
//! (DESIGN.md, "Data tree", "Schema binding" and "Name resolution"): a tab
//! whose schema is an XSD, schema tabs that hold XSD documents, and `XMLPATH`.

use std::sync::Arc;

use cliotree_core::{Path, Value, metaschema, validate, xml, xsd};

use super::PythonEngine;
use super::syntax::PythonSyntax;
use crate::{Calc, EditError, SchemaFormat};

const XSD: &str = r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="customer" type="xs:string"/>
        <xs:element name="line" maxOccurs="unbounded">
          <xs:complexType>
            <xs:sequence>
              <xs:element name="sku" type="xs:string"/>
              <xs:element name="qty" type="xs:integer"/>
              <xs:element name="price" type="xs:decimal"/>
            </xs:sequence>
            <xs:attribute name="id" type="xs:integer" use="required"/>
          </xs:complexType>
        </xs:element>
        <xs:element name="placed" type="xs:date" minOccurs="0"/>
        <xs:element name="total" type="xs:decimal" minOccurs="0"/>
        <xs:element name="n" type="xs:integer" minOccurs="0"/>
        <xs:element name="skus" type="xs:string" minOccurs="0"/>
        <xs:element name="year" type="xs:integer" minOccurs="0"/>
      </xs:sequence>
      <xs:attribute name="status" type="xs:string"/>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

const DATA: &str = r#"<order status="open">
  <customer>Acme</customer>
  <line id="1"><sku>A</sku><qty>2</qty><price>9.5</price></line>
  <line id="2"><sku>B</sku><qty>1</qty><price>24</price></line>
  <line id="3"><sku>C</sku><qty>12</qty><price>0.75</price></line>
  <placed>2026-10-23</placed>
</order>"#;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

struct Book {
    calc: Calc,
    engine: PythonEngine,
}

impl Book {
    /// A schema tab holding the XSD and a data tab bound to it.
    fn new() -> Book {
        let mut calc = Calc::new(Arc::new(PythonSyntax));
        let meta = metaschema::xsd_meta();
        let schema = xml::parse(XSD, meta).unwrap();
        calc.add_xsd_sheet("OrderSchema", schema.node, schema.document)
            .unwrap();
        let spec = xsd::lower_str(XSD).unwrap();
        let data = xml::parse(DATA, &spec).unwrap();
        calc.add_sheet("Orders", spec, data.node).unwrap();
        calc.set_xml_document("Orders", data.document).unwrap();
        calc.bind_schema("Orders", "OrderSchema").unwrap();
        Book {
            calc,
            engine: PythonEngine::new(),
        }
    }

    fn formula(&mut self, path: &str, text: &str) {
        self.calc.set_formula("Orders", &p(path), text).unwrap();
    }

    fn run(&mut self) {
        self.calc.recalc(&self.engine);
    }

    fn get(&self, path: &str) -> Option<Value> {
        self.calc
            .data("Orders")
            .unwrap()
            .get(&p(path))
            .map(|n| n.value().clone())
    }
}

#[test]
fn an_xsd_tab_gives_the_tabs_bound_to_it_xml_data() {
    let b = Book::new();
    assert_eq!(b.calc.schema_format("OrderSchema"), Some(SchemaFormat::Xsd));
    assert!(b.calc.is_xml("Orders"));
    assert_eq!(b.calc.schema_error("OrderSchema"), None);
    let spec = b.calc.spec("Orders").unwrap();
    assert_eq!(spec.xml().unwrap().root, "order");
    // The tab's data is valid against the schema the tab gave it.
    assert_eq!(
        validate::validate(spec, b.calc.data("Orders").unwrap()),
        vec![]
    );
    // What the document said about itself is kept for writing it back.
    assert!(b.calc.xml_document("Orders").unwrap().declaration.is_none());
}

/// Name resolution: attributes are reached by `["@name"]`, repeated elements
/// are a list, and the text of an element is its value.
#[test]
fn formulas_read_attributes_elements_and_repeated_elements() {
    let mut b = Book::new();
    b.formula("total", "sum(l.qty * l.price for l in Orders.line)");
    b.formula("n", "len(Orders.line)");
    b.formula("skus", "','.join(l.sku for l in Orders.line)");
    b.formula("year", "Orders.placed.year");
    b.run();
    assert_eq!(b.get("total"), Some(Value::Float(52.0)));
    assert_eq!(b.get("n"), Some(Value::Int(3)));
    assert_eq!(b.get("skus"), Some(Value::Str("A,B,C".into())));
    // A date node reads as a Python date.
    assert_eq!(b.get("year"), Some(Value::Int(2026)));
}

#[test]
fn an_attribute_can_be_read_and_can_hold_a_formula() {
    let mut b = Book::new();
    b.formula("n", "Orders.line[1][\"@id\"]");
    b.formula("[\"@status\"]", "'paid' if Orders.n == 2 else 'open'");
    b.run();
    assert_eq!(b.get("n"), Some(Value::Int(2)));
    assert_eq!(b.get("[\"@status\"]"), Some(Value::Str("paid".into())));
    // Editing the attribute it read changes the result.
    b.calc
        .set_value("Orders", &p("line[1][\"@id\"]"), Value::Int(7))
        .unwrap();
    b.run();
    assert_eq!(b.get("n"), Some(Value::Int(7)));
    assert_eq!(b.get("[\"@status\"]"), Some(Value::Str("open".into())));
}

/// Name resolution: `XMLPATH` for queries that do not fit member syntax.
#[test]
fn xmlpath_selects_elements_attributes_and_text() {
    // A query over a whole tab reads all of it, so it is circular with the
    // formulas on that tab; these read the lines only.
    let mut b = Book::new();
    b.formula("n", "len(XMLPATH('.[qty > 1]', Orders.line))");
    b.formula("skus", "','.join(XMLPATH('.[qty > 1]/sku', Orders.line))");
    b.formula("total", "sum(XMLPATH('//price', Orders.line))");
    b.formula("year", "max(XMLPATH('@id', Orders.line))");
    b.run();
    assert_eq!(b.get("n"), Some(Value::Int(2)));
    assert_eq!(b.get("skus"), Some(Value::Str("A,C".into())));
    assert_eq!(b.get("total"), Some(Value::Float(34.25)));
    assert_eq!(b.get("year"), Some(Value::Int(3)));
}

#[test]
fn xmlpath_depends_on_what_it_reads_and_says_why_a_query_is_wrong() {
    let mut b = Book::new();
    b.formula("n", "len(XMLPATH('.[qty > 1]', Orders.line))");
    b.run();
    assert_eq!(b.get("n"), Some(Value::Int(2)));
    b.calc
        .set_value("Orders", &p("line[1].qty"), Value::Int(5))
        .unwrap();
    b.run();
    assert_eq!(b.get("n"), Some(Value::Int(3)));
    b.formula("n", "len(XMLPATH('line[', Orders.line))");
    b.run();
    let Some(Value::Error(e)) = b.get("n") else {
        panic!("{:?}", b.get("n"))
    };
    assert!(e.message.contains("XMLPATH query"), "{}", e.message);
    b.formula("n", "len(XMLPATH('line', 5))");
    b.run();
    let Some(Value::Error(e)) = b.get("n") else {
        panic!()
    };
    assert!(e.message.contains("XMLPATH takes"), "{}", e.message);
}

/// Schema binding: editing the XSD tab relowers the tabs bound to it.
#[test]
fn editing_the_xsd_changes_the_type_of_the_tab_bound_to_it() {
    let mut b = Book::new();
    let before = b.calc.spec("Orders").unwrap();
    assert!(matches!(
        before
            .get(before.resolve(&p("line[0].qty")).unwrap().ty)
            .kind,
        cliotree_core::Kind::Integer(_)
    ));
    // The type of `qty` is the `type` attribute of the third element in the
    // sequence of the repeated `line`'s complex type.
    let at =
        p("element.complexType.sequence.element[1].complexType.sequence.element[1][\"@type\"]");
    assert_eq!(
        b.calc
            .data("OrderSchema")
            .unwrap()
            .get(&at)
            .unwrap()
            .value(),
        &Value::Str("xs:integer".into())
    );
    b.calc
        .set_value("OrderSchema", &at, Value::Str("xs:decimal".into()))
        .unwrap();
    let after = b.calc.spec("Orders").unwrap();
    assert!(matches!(
        after.get(after.resolve(&p("line[0].qty")).unwrap().ty).kind,
        cliotree_core::Kind::Number(_)
    ));
}

#[test]
fn an_xsd_that_cannot_be_lowered_says_why_and_leaves_the_tab_as_it_was() {
    let mut b = Book::new();
    let at =
        p("element.complexType.sequence.element[1].complexType.sequence.element[1][\"@type\"]");
    b.calc
        .set_value("OrderSchema", &at, Value::Str("xs:time".into()))
        .unwrap();
    let why = b.calc.schema_error("OrderSchema").expect("an error");
    assert!(why.contains("time"), "{why}");
    let spec = b.calc.spec("Orders").unwrap();
    assert!(matches!(
        spec.get(spec.resolve(&p("line[0].qty")).unwrap().ty).kind,
        cliotree_core::Kind::Integer(_)
    ));
}

#[test]
fn changing_an_xml_schema_from_a_data_tab_is_not_supported_yet_and_says_so() {
    let mut b = Book::new();
    b.calc.set_auto_update("OrderSchema", true).unwrap();
    let e = b.calc.field_places("Orders", &Path::root()).unwrap_err();
    let EditError::Path(message) = e else {
        panic!("{e:?}")
    };
    assert!(
        message.contains("XSD") && message.contains("not supported yet"),
        "{message}"
    );
}

#[test]
fn an_xml_tab_is_written_back_as_xml() {
    let mut b = Book::new();
    b.formula("n", "len(Orders.line)");
    b.run();
    let spec = b.calc.spec("Orders").unwrap();
    let text = xml::to_string(
        b.calc.data("Orders").unwrap(),
        spec,
        b.calc.xml_document("Orders").unwrap(),
        true,
    )
    .unwrap();
    assert!(text.contains("<n>3</n>"), "{text}");
    assert!(text.contains("<line id=\"2\">"), "{text}");
}

#[test]
fn the_default_document_of_a_new_schema_binds_the_xs_prefix() {
    let doc = xsd::default_document();
    assert_eq!(doc.root_prefix.as_deref(), Some("xs"));
    assert_eq!(doc.declarations.len(), 1);
}
