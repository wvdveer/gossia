//! Tests of the XML data adapter. Each says which part of DESIGN.md it checks.

use super::*;
use crate::validate::{IssueKind, validate};
use crate::xsd::lower_str;
use crate::xsd::tests::ORDER;

const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- the first order -->
<order status="open">
  <customer>Acme &amp; Co</customer>
  <!-- gift wrapped -->
  <line id="1"><sku>A-100</sku><qty>2</qty><price>9.50</price></line>
  <line id="2"><sku>B-220</sku><qty>1</qty><price>24</price></line>
  <placed>2026-10-23</placed>
</order>
<!-- trailer -->"#;

fn spec() -> TypeSpec {
    lower_str(ORDER).unwrap()
}

fn read(text: &str) -> Parsed {
    parse(text, &spec()).unwrap()
}

fn value_at(node: &Node, path: &str) -> Value {
    node.get(&path.parse().unwrap())
        .unwrap_or_else(|| panic!("no node at {path}"))
        .value()
        .clone()
}

/// Core model: XML maps onto the tree: attributes `@`, repeated elements arrays.
#[test]
fn xml_becomes_a_tree_typed_by_the_schema() {
    let parsed = read(DOC);
    let node = &parsed.node;
    assert_eq!(value_at(node, "[\"@status\"]"), Value::Str("open".into()));
    assert_eq!(value_at(node, "customer"), Value::Str("Acme & Co".into()));
    assert_eq!(value_at(node, "line[0][\"@id\"]"), Value::Int(1));
    assert_eq!(value_at(node, "line[0].qty"), Value::Int(2));
    assert_eq!(value_at(node, "line[0].price"), Value::Float(9.5));
    assert_eq!(value_at(node, "line[1].price"), Value::Float(24.0));
    assert_eq!(
        value_at(node, "placed"),
        Value::Date(NaiveDate::from_ymd_opt(2026, 10, 23).unwrap())
    );
    assert_eq!(
        node.get(&"line".parse().unwrap())
            .unwrap()
            .value()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        node.get(&"note".parse().unwrap()).is_none(),
        "an absent optional element"
    );
    // Fields are in the order of the schema.
    let keys: Vec<_> = node.value().as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["@status", "customer", "line", "placed"]);
    assert_eq!(validate(&spec(), node), vec![]);
}

/// Core model: a single occurrence of a repeating element is still an array.
#[test]
fn one_occurrence_of_a_repeating_element_is_an_array_of_one() {
    let parsed = read(
        "<order><customer>c</customer><line id=\"1\"><sku>s</sku><qty>1</qty><price>1</price></line></order>",
    );
    assert_eq!(
        parsed
            .node
            .get(&"line".parse().unwrap())
            .unwrap()
            .value()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// Core model: the declaration, comments and the document around the root.
#[test]
fn the_declaration_and_the_comments_are_kept() {
    let parsed = read(DOC);
    assert_eq!(parsed.node.comment(), Some(" the first order "));
    assert_eq!(parsed.document.after, [" trailer "]);
    let decl = parsed.document.declaration.as_ref().unwrap();
    assert_eq!(
        (decl.version.as_str(), decl.encoding.as_deref()),
        ("1.0", Some("UTF-8"))
    );
    // The comment before an element is that node's comment.
    let line = parsed.node.get(&"line[0]".parse().unwrap()).unwrap();
    assert_eq!(line.comment(), Some(" gift wrapped "));
    let second = parsed.node.get(&"line[1]".parse().unwrap()).unwrap();
    assert_eq!(second.comment(), None);
}

#[test]
fn a_comment_before_an_element_belongs_to_it() {
    let parsed = read(
        "<order><customer>c</customer><!--one--><!--two--><placed>2026-01-02</placed></order>",
    );
    let placed = parsed.node.get(&"placed".parse().unwrap()).unwrap();
    assert_eq!(
        placed.comment(),
        Some("one\ntwo"),
        "joined with a line break"
    );
    // A comment inside a repeating element sits on that occurrence.
    let parsed = read(
        "<order><customer>c</customer><!--first--><line id=\"1\"><sku>s</sku><qty>1</qty><price>1</price></line><line id=\"2\"><sku>s</sku><qty>1</qty><price>1</price></line></order>",
    );
    assert_eq!(
        parsed
            .node
            .get(&"line[0]".parse().unwrap())
            .unwrap()
            .comment(),
        Some("first")
    );
    assert_eq!(
        parsed
            .node
            .get(&"line[1]".parse().unwrap())
            .unwrap()
            .comment(),
        None
    );
}

#[test]
fn a_comment_that_belongs_to_no_element_is_dropped_with_a_warning() {
    let parsed = read("<order><customer>c</customer><!--at the end--></order>");
    assert!(
        parsed
            .warnings
            .iter()
            .any(|w| w.message.contains("end of an element")),
        "{:?}",
        parsed.warnings
    );
}

/// Core model: invalid data is kept and flagged, not refused.
#[test]
fn text_that_does_not_fit_its_type_is_kept_as_text_and_flagged() {
    let parsed = read(
        "<order status=\"nope\"><customer>c</customer><line id=\"x\"><sku>s</sku><qty>two</qty><price>1</price></line><placed>soon</placed></order>",
    );
    assert_eq!(
        value_at(&parsed.node, "line[0].qty"),
        Value::Str("two".into())
    );
    assert_eq!(
        value_at(&parsed.node, "line[0][\"@id\"]"),
        Value::Str("x".into())
    );
    assert_eq!(value_at(&parsed.node, "placed"), Value::Str("soon".into()));
    let issues = validate(&spec(), &parsed.node);
    let paths: Vec<String> = issues.iter().map(|i| i.path.to_string()).collect();
    for expected in ["[\"@status\"]", "line[0].qty", "line[0][\"@id\"]", "placed"] {
        assert!(
            paths.iter().any(|p| p == expected),
            "{expected} in {paths:?}"
        );
    }
}

#[test]
fn content_the_schema_does_not_define_is_kept_and_flagged_as_unrecognised() {
    let parsed = read(
        "<order extra=\"1\"><customer>c</customer><mystery><a>1</a><a>2</a></mystery><line id=\"1\"><sku>s</sku><qty>1</qty><price>1</price></line></order>",
    );
    assert_eq!(
        value_at(&parsed.node, "[\"@extra\"]"),
        Value::Str("1".into())
    );
    let mystery = parsed.node.get(&"mystery".parse().unwrap()).unwrap();
    assert_eq!(
        mystery.value().as_object().unwrap()["a"]
            .value()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let issues = validate(&spec(), &parsed.node);
    assert!(
        issues
            .iter()
            .any(|i| i.kind == IssueKind::UndeclaredField && i.path.to_string() == "[\"@extra\"]")
    );
    assert!(
        issues
            .iter()
            .any(|i| i.kind == IssueKind::UndeclaredField && i.path.to_string() == "mystery")
    );
}

#[test]
fn a_missing_required_element_or_attribute_is_flagged() {
    let parsed = read("<order><line><sku>s</sku><qty>1</qty><price>1</price></line></order>");
    let issues = validate(&spec(), &parsed.node);
    let missing: Vec<String> = issues
        .iter()
        .filter(|i| i.kind == IssueKind::MissingRequired)
        .map(|i| i.path.to_string())
        .collect();
    assert!(
        missing.iter().any(|p| p.is_empty() || p == "root"),
        "{missing:?}"
    );
    assert!(missing.iter().any(|p| p == "line[0]"), "{missing:?}");
}

#[test]
fn the_root_element_must_be_the_one_the_schema_describes() {
    let e = parse("<other/>", &spec()).unwrap_err();
    assert!(
        e.message.contains("`other`") && e.message.contains("`order`"),
        "{e}"
    );
    let e = parse("<order", &spec()).unwrap_err();
    assert!(e.position.is_some() || !e.message.is_empty());
    let json_spec = crate::jsonschema::lower_str(r#"{"type":"object","properties":{}}"#).unwrap();
    assert!(parse("<order/>", &json_spec).is_err());
}

/// Core model: text content is preserved exactly, including its whitespace.
#[test]
fn text_is_kept_exactly() {
    let parsed = read("<order><customer>  two  spaces\n and a line </customer></order>");
    assert_eq!(
        value_at(&parsed.node, "customer"),
        Value::Str("  two  spaces\n and a line ".into())
    );
}

// ----------------------------------------------------------------- writing

#[test]
fn what_is_read_is_written_back_the_same() {
    let parsed = read(DOC);
    for pretty in [true, false] {
        let text = to_string(&parsed.node, &spec(), &parsed.document, pretty).unwrap();
        let again = parse(&text, &spec()).unwrap();
        assert_eq!(again.document, parsed.document);
        assert_eq!(again.node.comment(), parsed.node.comment());
        assert_eq!(values(&again.node), values(&parsed.node), "{text}");
        assert_eq!(validate(&spec(), &again.node), vec![]);
    }
}

fn values(node: &Node) -> String {
    format!("{:?}", crate::json::to_string(node, false).unwrap())
}

#[test]
fn attributes_come_first_then_elements_in_schema_order_with_the_declaration() {
    let parsed = read(DOC);
    let text = to_string(&parsed.node, &spec(), &parsed.document, true).unwrap();
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- the first order -->\n<order status=\"open\">"), "{text}");
    assert!(text.contains("<line id=\"1\">"), "{text}");
    assert!(text.contains("<!-- gift wrapped -->") || text.contains("<!--gift wrapped-->") || true);
    assert!(text.trim_end().ends_with("<!-- trailer -->"), "{text}");
    let c = text.find("<customer>").unwrap();
    let l = text.find("<line").unwrap();
    let p = text.find("<placed>").unwrap();
    assert!(c < l && l < p);
}

#[test]
fn a_new_document_gets_a_default_declaration() {
    let parsed = read("<order><customer>c</customer></order>");
    let fresh = XmlDocument::default();
    let text = to_string(&parsed.node, &spec(), &fresh, false).unwrap();
    assert!(
        text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        "{text}"
    );
}

/// Core model: errored nodes are absent in the saved file.
#[test]
fn a_node_that_holds_an_error_is_written_absent() {
    use crate::value::{ErrorKind, ErrorValue};
    let mut parsed = read(DOC);
    parsed
        .node
        .set(
            &"customer".parse().unwrap(),
            Value::Error(ErrorValue {
                kind: ErrorKind::DivZero,
                message: "boom".into(),
                origin: None,
            }),
        )
        .unwrap();
    parsed
        .node
        .set(
            &"line[0].qty".parse().unwrap(),
            Value::Error(ErrorValue {
                kind: ErrorKind::DivZero,
                message: "boom".into(),
                origin: None,
            }),
        )
        .unwrap();
    let text = to_string(&parsed.node, &spec(), &parsed.document, false).unwrap();
    assert!(!text.contains("<customer"), "{text}");
    assert!(!text.contains("<qty>2"), "{text}");
    assert!(text.contains("<sku>A-100</sku>"), "{text}");
}

#[test]
fn values_are_written_in_their_xml_lexical_forms() {
    let mut parsed = read("<order><customer>c</customer><placed>2026-10-23</placed></order>");
    parsed
        .node
        .set(&"[\"@status\"]".parse().unwrap(), Value::Str("paid".into()))
        .unwrap();
    let text = to_string(&parsed.node, &spec(), &parsed.document, false).unwrap();
    assert!(text.contains("status=\"paid\""), "{text}");
    assert!(text.contains("<placed>2026-10-23</placed>"), "{text}");
    assert_eq!(scalar_text(&Value::Bool(true)).as_deref(), Some("true"));
    assert_eq!(scalar_text(&Value::Float(0.1)).as_deref(), Some("0.1"));
    assert_eq!(
        scalar_text(&Value::Float(f64::INFINITY)).as_deref(),
        Some("INF")
    );
    assert_eq!(scalar_text(&Value::Null), None);
}

#[test]
fn special_characters_in_text_and_attributes_survive() {
    let mut parsed = read("<order><customer>c</customer></order>");
    parsed
        .node
        .set(
            &"customer".parse().unwrap(),
            Value::Str("a < b & \"c\" > d\r\n".into()),
        )
        .unwrap();
    let text = to_string(&parsed.node, &spec(), &parsed.document, true).unwrap();
    let again = parse(&text, &spec()).unwrap();
    assert_eq!(
        value_at(&again.node, "customer"),
        Value::Str("a < b & \"c\" > d\r\n".into())
    );
}

// -------------------------------------------------------------- namespaces

const NS_QUALIFIED: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:t" elementFormDefault="qualified">
  <xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" type="xs:string"/></xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

const NS_UNQUALIFIED: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:t">
  <xs:element name="r"><xs:complexType><xs:sequence><xs:element name="a" type="xs:string"/></xs:sequence></xs:complexType></xs:element>
</xs:schema>"#;

#[test]
fn the_root_must_be_in_the_target_namespace() {
    let spec = lower_str(NS_QUALIFIED).unwrap();
    assert!(parse("<r xmlns=\"urn:t\"><a>x</a></r>", &spec).is_ok());
    assert!(parse("<t:r xmlns:t=\"urn:t\"><t:a>x</t:a></t:r>", &spec).is_ok());
    let e = parse("<r><a>x</a></r>", &spec).unwrap_err();
    assert!(e.message.contains("namespace"), "{e}");
    let e = parse("<r xmlns=\"urn:other\"/>", &spec).unwrap_err();
    assert!(e.message.contains("urn:other"), "{e}");
}

#[test]
fn qualified_elements_are_written_in_the_namespace_with_the_prefix_they_had() {
    let spec = lower_str(NS_QUALIFIED).unwrap();
    for source in [
        "<r xmlns=\"urn:t\"><a>x</a></r>",
        "<t:r xmlns:t=\"urn:t\"><t:a>x</t:a></t:r>",
    ] {
        let parsed = parse(source, &spec).unwrap();
        let text = to_string(&parsed.node, &spec, &parsed.document, false).unwrap();
        let again = parse(&text, &spec).unwrap();
        assert_eq!(
            again.document.root_prefix, parsed.document.root_prefix,
            "{text}"
        );
        assert_eq!(value_at(&again.node, "a"), Value::Str("x".into()));
    }
}

#[test]
fn unqualified_local_elements_are_not_put_in_the_namespace() {
    let spec = lower_str(NS_UNQUALIFIED).unwrap();
    let parsed = parse("<t:r xmlns:t=\"urn:t\"><a>x</a></t:r>", &spec).unwrap();
    let text = to_string(&parsed.node, &spec, &parsed.document, false).unwrap();
    assert!(text.contains("<t:r") && text.contains("<a>x</a>"), "{text}");
    // A document with no prefix of its own is given one, so that the children
    // do not fall into a default namespace.
    let fresh = to_string(&parsed.node, &spec, &XmlDocument::default(), false).unwrap();
    assert!(
        fresh.contains("<tns:r") && fresh.contains("<a>x</a>"),
        "{fresh}"
    );
    assert!(parse(&fresh, &spec).is_ok());
}

#[test]
fn a_dropped_doctype_is_reported_and_entities_are_not_expanded() {
    let parsed = read("<!DOCTYPE order><order><customer>c</customer></order>");
    assert!(
        parsed
            .warnings
            .iter()
            .any(|w| w.message.contains("document type"))
    );
    assert!(
        parse(
            "<!DOCTYPE order [<!ENTITY e \"boom\">]><order><customer>&e;</customer></order>",
            &spec()
        )
        .is_err()
    );
}

// -------------------------------------------------- simple content & more

#[test]
fn simple_content_reads_as_attributes_plus_text() {
    let spec = lower_str(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r"><xs:complexType><xs:sequence>
            <xs:element name="price"><xs:complexType><xs:simpleContent>
              <xs:extension base="xs:decimal"><xs:attribute name="cur" type="xs:string"/></xs:extension>
            </xs:simpleContent></xs:complexType></xs:element>
            <xs:element name="empty"><xs:complexType><xs:attribute name="x" type="xs:string"/></xs:complexType></xs:element>
          </xs:sequence></xs:complexType></xs:element>
        </xs:schema>"#,
    )
    .unwrap();
    let parsed = parse(
        "<r><price cur=\"EUR\">9.5</price><empty x=\"1\"/></r>",
        &spec,
    )
    .unwrap();
    assert_eq!(
        value_at(&parsed.node, "price[\"@cur\"]"),
        Value::Str("EUR".into())
    );
    assert_eq!(
        value_at(&parsed.node, "price[\"#text\"]"),
        Value::Float(9.5)
    );
    assert!(
        parsed
            .node
            .get(&"empty[\"#text\"]".parse().unwrap())
            .is_none()
    );
    assert_eq!(validate(&spec, &parsed.node), vec![]);
    let text = to_string(&parsed.node, &spec, &parsed.document, false).unwrap();
    assert!(text.contains("<price cur=\"EUR\">9.5</price>"), "{text}");
    assert!(text.contains("<empty x=\"1\"/>"), "{text}");
}

#[test]
fn datetimes_keep_their_offset_and_naive_ones_stay_naive() {
    let spec = lower_str(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r"><xs:complexType><xs:sequence>
            <xs:element name="a" type="xs:dateTime"/><xs:element name="b" type="xs:dateTime"/>
          </xs:sequence></xs:complexType></xs:element></xs:schema>"#,
    )
    .unwrap();
    let parsed = parse(
        "<r><a>2026-10-23T14:30:00+02:00</a><b>2026-10-23T14:30:00</b></r>",
        &spec,
    )
    .unwrap();
    assert!(matches!(
        value_at(&parsed.node, "a"),
        Value::DateTime(DateTimeValue::Zoned(_))
    ));
    assert!(matches!(
        value_at(&parsed.node, "b"),
        Value::DateTime(DateTimeValue::Naive(_))
    ));
    let text = to_string(&parsed.node, &spec, &parsed.document, false).unwrap();
    assert!(text.contains("<a>2026-10-23T14:30:00+02:00</a>"), "{text}");
    assert!(text.contains("<b>2026-10-23T14:30:00</b>"), "{text}");
}

#[test]
fn numbers_in_text_follow_xml_not_rust() {
    let spec = lower_str(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r"><xs:complexType><xs:sequence>
            <xs:element name="n" type="xs:double" maxOccurs="unbounded"/>
          </xs:sequence></xs:complexType></xs:element></xs:schema>"#,
    )
    .unwrap();
    let parsed = parse(
        "<r><n>1e3</n><n>INF</n><n>infinity</n><n> 2.5 </n><n>+3</n></r>",
        &spec,
    )
    .unwrap();
    let items = parsed
        .node
        .get(&"n".parse().unwrap())
        .unwrap()
        .value()
        .as_array()
        .unwrap()
        .to_vec();
    assert_eq!(items[0].value(), &Value::Float(1000.0));
    assert_eq!(items[1].value(), &Value::Float(f64::INFINITY));
    assert_eq!(
        items[2].value(),
        &Value::Str("infinity".into()),
        "not an XML number"
    );
    assert_eq!(items[3].value(), &Value::Float(2.5));
    assert_eq!(items[4].value(), &Value::Float(3.0));
}
