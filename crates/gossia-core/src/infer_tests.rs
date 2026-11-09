//! Tests of schema inference (DESIGN.md, "Schema inference").

use crate::infer::{infer_json, infer_xsd, text_lines};
use crate::validate::validate;
use crate::value::{Node, Value};
use crate::{json, jsonschema, xml, xmltree, xsd};

fn schema_text(node: &Node) -> String {
    json::to_string(node, false).unwrap()
}

fn infer(text: &str) -> crate::infer::JsonInference {
    infer_json(&json::parse(text).unwrap().node)
}

#[test]
fn objects_arrays_and_scalars_are_inferred_with_what_is_required() {
    let inferred = infer(
        r#"{"name": "Acme", "items": [
            {"sku": "A", "qty": 2, "price": 9.5, "gift": true},
            {"sku": "B", "qty": 1, "price": 24}
        ]}"#,
    );
    assert_eq!(
        schema_text(&inferred.schema),
        r#"{"type":"object","properties":{"name":{"type":"string"},"items":{"type":"array","items":{"type":"object","properties":{"sku":{"type":"string"},"qty":{"type":"integer"},"price":{"type":"number"},"gift":{"type":"boolean"}},"required":["sku","qty","price"]}}},"required":["name","items"]}"#
    );
    assert!(inferred.warnings.is_empty(), "{:?}", inferred.warnings);
    let spec = jsonschema::lower(&inferred.schema).unwrap();
    assert!(validate(&spec, &inferred.data).is_empty());
}

#[test]
fn nulls_are_left_out_and_reported() {
    let inferred = infer(r#"{"a": [1, null, 2], "b": null, "c": [{"x": null}, {"x": 1}]}"#);
    assert_eq!(
        schema_text(&inferred.data),
        r#"{"a":[1,2],"c":[{},{"x":1}]}"#
    );
    assert_eq!(
        inferred.dropped,
        [
            "a: 1 null item left out, since version 1 has no null.",
            "b: 1 null left out, since version 1 has no null, so it is optional.",
            "c[].x: 1 null left out, since version 1 has no null, so it is optional.",
        ]
    );
    // `b` was only ever null: it is typed from nothing.
    assert!(
        inferred
            .warnings
            .iter()
            .any(|w| w.starts_with("b: inferred from no data"))
    );
    let spec = jsonschema::lower(&inferred.schema).unwrap();
    assert!(validate(&spec, &inferred.data).is_empty());
}

#[test]
fn the_most_common_kind_wins_and_a_tie_goes_to_the_more_general() {
    let inferred = infer(r#"[{"v": 1}, {"v": 2}, {"v": "three"}, {"w": true}, {"w": "x"}]"#);
    let text = schema_text(&inferred.schema);
    assert!(text.contains(r#""v":{"type":"integer"}"#), "{text}");
    assert!(text.contains(r#""w":{"type":"string"}"#), "{text}");
    assert_eq!(inferred.warnings.len(), 2, "{:?}", inferred.warnings);
    assert!(inferred.warnings[0].contains("2 whole number, 1 text"));
    // The value of the other kind is kept, and flagged.
    let spec = jsonschema::lower(&inferred.schema).unwrap();
    assert_eq!(validate(&spec, &inferred.data).len(), 2);
}

#[test]
fn whole_numbers_mixed_with_others_are_numbers() {
    let inferred = infer("[1, 2.5, 3]");
    assert_eq!(
        schema_text(&inferred.schema),
        r#"{"type":"array","items":{"type":"number"}}"#
    );
    assert!(inferred.warnings.is_empty());
}

#[test]
fn an_empty_array_has_items_of_text_and_says_so() {
    let inferred = infer(r#"{"tags": []}"#);
    assert!(
        schema_text(&inferred.schema)
            .contains(r#""tags":{"type":"array","items":{"type":"string"}}"#)
    );
    assert_eq!(
        inferred.warnings,
        ["tags[]: inferred from no data (the arrays are empty), so the items are text."]
    );
}

#[test]
fn plain_text_is_an_array_of_its_lines() {
    let lines = |t: &str| schema_text(&text_lines(t).data);
    assert_eq!(lines("a\r\nb\n\nc\n"), r#"["a","b","","c"]"#);
    assert_eq!(lines("one"), r#"["one"]"#);
    assert_eq!(lines(""), "[]");
    let inferred = text_lines("x\n");
    let spec = jsonschema::lower(&inferred.schema).unwrap();
    assert!(validate(&spec, &inferred.data).is_empty());
}

// ------------------------------------------------------------------- XML

/// Infers an XSD, lowers it, and reads the document back with it.
fn round_trip(text: &str) -> (String, Vec<String>, Node) {
    let doc = xmltree::parse(text).unwrap();
    let inferred = infer_xsd(&doc).unwrap();
    let spec = xsd::lower_str(&inferred.text).unwrap_or_else(|e| panic!("{e}\n{}", inferred.text));
    let parsed = xml::parse(text, &spec).unwrap();
    assert!(
        validate(&spec, &parsed.node).is_empty(),
        "{:?}\n{}",
        validate(&spec, &parsed.node),
        inferred.text
    );
    // The schema can also be a schema tab: the XSD meta-schema reads it.
    xml::parse(&inferred.text, crate::metaschema::xsd_meta()).unwrap();
    (inferred.text, inferred.warnings, parsed.node)
}

#[test]
fn elements_attributes_and_repeats_are_inferred() {
    let (xsd, warnings, node) = round_trip(
        r#"<orders date="2026-10-26">
             <order id="1"><sku>A-1</sku><qty>2</qty><price cur="EUR">9.50</price></order>
             <order id="2" rush="true"><sku>007</sku><qty>1</qty><note>gift</note><price>24</price></order>
           </orders>"#,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    for part in [
        r#"<xs:element name="order" maxOccurs="unbounded">"#,
        r#"<xs:element name="sku" type="xs:string"/>"#,
        r#"<xs:element name="qty" type="xs:integer"/>"#,
        r#"<xs:element name="note" type="xs:string" minOccurs="0"/>"#,
        r#"<xs:extension base="xs:decimal">"#,
        r#"<xs:attribute name="cur" type="xs:string"/>"#,
        r#"<xs:attribute name="id" type="xs:integer" use="required"/>"#,
        r#"<xs:attribute name="rush" type="xs:boolean"/>"#,
        r#"<xs:attribute name="date" type="xs:date" use="required"/>"#,
    ] {
        assert!(xsd.contains(part), "{part}\n{xsd}");
    }
    // `note` was seen between `qty` and `price`, so it goes there.
    let order = xsd.find("name=\"qty\"").unwrap();
    assert!(order < xsd.find("name=\"note\"").unwrap());
    assert!(xsd.find("name=\"note\"").unwrap() < xsd.find("name=\"price\"").unwrap());
    let qty = node
        .get(&"order[1].qty".parse().unwrap())
        .map(|n| n.value().clone());
    assert_eq!(qty, Some(Value::Int(1)));
}

#[test]
fn a_target_namespace_is_kept() {
    let (xsd, _, _) = round_trip(r#"<list xmlns="urn:x"><item>1</item><item>2</item></list>"#);
    assert!(
        xsd.contains(r#"targetNamespace="urn:x" elementFormDefault="qualified""#),
        "{xsd}"
    );
}

#[test]
fn disagreeing_orders_are_reported() {
    let (_, warnings, _) = round_trip("<r><a><x>1</x><y>2</y></a><a><y>3</y><x>4</x></a></r>");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].starts_with("<r/a>: the child elements"));
}

/// XML Schemas and namespaces, 0.5.0: mixed content is inferred as a mixed
/// type, its inside kept as markup, and written back as it was.
#[test]
fn mixed_content_is_kept_as_markup() {
    let text = "<doc><p class=\"a\">Hello <b>you</b> &amp; <i>them</i><!--c--></p><p>plain</p><title>T</title><note>Odd <x:y xmlns:x=\"urn:x\"/></note></doc>";
    let (xsd, warnings, node) = round_trip(text);
    assert!(xsd.contains(r#"<xs:complexType mixed="true">"#), "{xsd}");
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].starts_with("<doc/p> holds text next to elements"));
    let Value::Object(doc) = node.value() else {
        panic!("{node:?}")
    };
    let Value::Array(ps) = doc["p"].value() else {
        panic!("{node:?}")
    };
    let Value::Object(first) = ps[0].value() else {
        panic!("{node:?}")
    };
    assert_eq!(
        first["#text"].value(),
        &Value::Str("Hello <b>you</b> &amp; <i>them</i><!--c-->".to_string())
    );
    assert_eq!(
        doc["note"].value(),
        &Value::Str("Odd <x:y xmlns:x=\"urn:x\"/>".to_string())
    );
    let spec = xsd::lower_str(&xsd).unwrap();
    let parsed = xml::parse(text, &spec).unwrap();
    let written = xml::to_string(&parsed.node, &spec, &parsed.document, false).unwrap();
    assert_eq!(written, text, "no declaration, as in the text");
}

#[test]
fn more_namespaces_are_refused() {
    let refused = |text: &str| infer_xsd(&xmltree::parse(text).unwrap()).unwrap_err();
    assert!(
        refused(r#"<r xmlns="urn:a"><x xmlns="urn:b"/></r>"#).contains("more than one namespace")
    );
    assert!(
        refused(r#"<r xmlns:o="urn:o" o:id="1"/>"#).contains("is in a namespace"),
        "an attribute with a prefix"
    );
}

#[test]
fn text_is_typed_by_all_its_values() {
    let (xsd, _, _) = round_trip(
        "<r><i>-3</i><i>10</i><d>1</d><d>2.25</d><z>0</z><z>01</z><e/><e>x</e>\
         <t>2026-10-26T10:00:00Z</t><b>false</b></r>",
    );
    for part in [
        r#"name="i" type="xs:integer""#,
        r#"name="d" type="xs:decimal""#,
        r#"name="z" type="xs:string""#,
        r#"name="e" type="xs:string""#,
        r#"name="t" type="xs:dateTime""#,
        r#"name="b" type="xs:boolean""#,
    ] {
        assert!(xsd.contains(part), "{part}\n{xsd}");
    }
}
