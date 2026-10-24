//! Tests of XSD lowering. Each says which part of DESIGN.md it checks.

use super::*;
use crate::path::Path;

pub(crate) const ORDER: &str = r#"<?xml version="1.0"?>
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
        <xs:element name="note" type="xs:string" minOccurs="0"/>
        <xs:element name="placed" type="xs:date" minOccurs="0"/>
      </xs:sequence>
      <xs:attribute name="status" type="Status"/>
    </xs:complexType>
  </xs:element>
  <xs:simpleType name="Status">
    <xs:restriction base="xs:string">
      <xs:enumeration value="open"/>
      <xs:enumeration value="paid"/>
    </xs:restriction>
  </xs:simpleType>
</xs:schema>"#;

fn lowered(text: &str) -> TypeSpec {
    match lower_str(text) {
        Ok(spec) => spec,
        Err(e) => panic!("{e}"),
    }
}

fn refused(text: &str) -> SchemaError {
    match lower_str(text) {
        Err(LoadError::Schema(e)) => e,
        Err(other) => panic!("not a schema error: {other}"),
        Ok(_) => panic!("the schema was accepted"),
    }
}

/// Wraps the body of a complex type in a schema with a root element.
fn schema(body: &str) -> String {
    format!(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r"><xs:complexType>{body}</xs:complexType></xs:element>
        </xs:schema>"#
    )
}

fn at<'a>(spec: &'a TypeSpec, path: &str) -> &'a TypeDef {
    let path: Path = if path.is_empty() {
        Path::root()
    } else {
        path.parse().unwrap()
    };
    spec.get(
        spec.resolve(&path)
            .unwrap_or_else(|| panic!("no {path}"))
            .ty,
    )
}

fn field_names(def: &TypeDef) -> Vec<String> {
    match &def.kind {
        Kind::Object { fields } => fields.iter().map(|f| f.name.clone()).collect(),
        other => panic!("not an object: {other:?}"),
    }
}

/// Schema subsets: a record is a complexType with a sequence of elements and
/// attributes; attributes are fields named `@name`.
#[test]
fn the_sample_lowers_to_records_arrays_and_attributes() {
    let spec = lowered(ORDER);
    let info = spec.xml().unwrap();
    assert_eq!(info.root, "order");
    assert_eq!(info.target_namespace, None);
    assert_eq!(
        field_names(at(&spec, "")),
        ["@status", "customer", "line", "note", "placed"]
    );
    assert!(matches!(at(&spec, "customer").kind, Kind::String(_)));
    assert!(matches!(at(&spec, "line[0].qty").kind, Kind::Integer(_)));
    assert!(matches!(at(&spec, "line[0].price").kind, Kind::Number(_)));
    assert!(matches!(at(&spec, "placed").kind, Kind::Date));
    assert_eq!(
        field_names(at(&spec, "line[0]")),
        ["@id", "sku", "qty", "price"]
    );
    // minOccurs 0 is optional, an attribute is optional unless required.
    let required = |path: &str| spec.resolve(&path.parse().unwrap()).unwrap().required;
    assert!(required("customer"));
    assert!(required("line"));
    assert!(!required("note"));
    assert!(!required("[\"@status\"]"));
    assert!(required("line[0][\"@id\"]"));
}

#[test]
fn a_repeating_element_is_an_array_with_its_bounds() {
    let spec = lowered(&schema(
        r#"<xs:sequence>
             <xs:element name="a" type="xs:string" minOccurs="2" maxOccurs="5"/>
             <xs:element name="b" type="xs:string" minOccurs="0" maxOccurs="unbounded"/>
           </xs:sequence>"#,
    ));
    match &at(&spec, "a").kind {
        Kind::Array {
            min_items,
            max_items,
            ..
        } => {
            assert_eq!((*min_items, *max_items), (Some(2), Some(5)));
        }
        other => panic!("{other:?}"),
    }
    match &at(&spec, "b").kind {
        Kind::Array {
            min_items,
            max_items,
            ..
        } => {
            assert_eq!((*min_items, *max_items), (None, None));
        }
        other => panic!("{other:?}"),
    }
    assert!(!spec.resolve(&"b".parse().unwrap()).unwrap().required);
}

#[test]
fn a_named_type_can_contain_itself_and_the_root_can_be_referred_to() {
    let spec = lowered(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="folder" type="Folder"/>
          <xs:complexType name="Folder">
            <xs:sequence>
              <xs:element name="name" type="xs:string"/>
              <xs:element name="folder" type="Folder" minOccurs="0" maxOccurs="unbounded"/>
            </xs:sequence>
          </xs:complexType>
        </xs:schema>"#,
    );
    // The same type, however deep.
    let deep = at(&spec, "folder[0].folder[1].folder[2]");
    assert_eq!(field_names(deep), ["name", "folder"]);

    let spec = lowered(&schema(
        r#"<xs:sequence>
             <xs:element name="n" type="xs:string"/>
             <xs:element ref="r" minOccurs="0" maxOccurs="unbounded"/>
           </xs:sequence>"#,
    ));
    assert_eq!(field_names(at(&spec, "r[0].r[0]")), ["n", "r"]);
}

/// Schema subsets: `xs:simpleContent` carries text beside attributes.
#[test]
fn simple_content_is_the_text_field_beside_the_attributes() {
    let spec = lowered(&schema(
        r#"<xs:sequence>
             <xs:element name="price">
               <xs:complexType>
                 <xs:simpleContent>
                   <xs:extension base="xs:decimal">
                     <xs:attribute name="currency" type="xs:string" use="required"/>
                   </xs:extension>
                 </xs:simpleContent>
               </xs:complexType>
             </xs:element>
           </xs:sequence>"#,
    ));
    assert_eq!(field_names(at(&spec, "price")), ["@currency", "#text"]);
    assert!(matches!(
        at(&spec, "price[\"#text\"]").kind,
        Kind::Number(_)
    ));
    assert!(
        !spec
            .resolve(&"price[\"#text\"]".parse().unwrap())
            .unwrap()
            .required
    );
}

#[test]
fn an_element_with_a_simple_type_and_no_attributes_is_its_value() {
    let spec = lowered(&schema(
        r#"<xs:sequence><xs:element name="n" type="xs:int"/></xs:sequence>"#,
    ));
    assert!(matches!(at(&spec, "n").kind, Kind::Integer(_)));
}

/// Schema subsets: restrictions and their facets.
#[test]
fn restrictions_become_enumerations_ranges_lengths_and_anchored_patterns() {
    let spec = lowered(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r">
            <xs:complexType><xs:sequence>
              <xs:element name="code" type="Code"/>
              <xs:element name="age" type="Age"/>
              <xs:element name="ratio" type="Ratio"/>
              <xs:element name="plate" type="Plate"/>
              <xs:element name="nonneg" type="xs:nonNegativeInteger"/>
            </xs:sequence></xs:complexType>
          </xs:element>
          <xs:simpleType name="Code"><xs:restriction base="xs:string">
            <xs:pattern value="[A-Z]{2}[0-9]+"/><xs:minLength value="3"/><xs:maxLength value="6"/>
          </xs:restriction></xs:simpleType>
          <xs:simpleType name="Age"><xs:restriction base="xs:integer">
            <xs:minInclusive value="0"/><xs:maxInclusive value="150"/>
          </xs:restriction></xs:simpleType>
          <xs:simpleType name="Ratio"><xs:restriction base="xs:decimal">
            <xs:minInclusive value="0.5"/>
          </xs:restriction></xs:simpleType>
          <xs:simpleType name="Plate"><xs:restriction base="xs:string">
            <xs:pattern value="[0-9]+"/><xs:pattern value="[a-z]+"/>
          </xs:restriction></xs:simpleType>
        </xs:schema>"#,
    );
    let Kind::String(code) = &at(&spec, "code").kind else {
        panic!()
    };
    let pattern = code.pattern.as_ref().unwrap();
    assert!(pattern.is_match("AB12"));
    assert!(!pattern.is_match("xAB12"), "anchored at the start");
    assert!(!pattern.is_match("AB12x"), "anchored at the end");
    assert_eq!((code.min_length, code.max_length), (Some(3), Some(6)));
    let Kind::Integer(age) = &at(&spec, "age").kind else {
        panic!()
    };
    assert_eq!((age.min, age.max), (Some(Num::Int(0)), Some(Num::Int(150))));
    let Kind::Number(ratio) = &at(&spec, "ratio").kind else {
        panic!()
    };
    assert_eq!(ratio.min, Some(Num::Float(0.5)));
    let Kind::String(plate) = &at(&spec, "plate").kind else {
        panic!()
    };
    let p = plate.pattern.as_ref().unwrap();
    assert!(p.is_match("123") && p.is_match("abc") && !p.is_match("a1"));
    let Kind::Integer(nonneg) = &at(&spec, "nonneg").kind else {
        panic!()
    };
    assert_eq!(nonneg.min, Some(Num::Int(0)));
    assert_eq!(at(&spec, "code").name.as_deref(), Some("Code"));
}

#[test]
fn an_enumeration_is_a_choice_of_strings() {
    let spec = lowered(ORDER);
    match &at(&spec, "[\"@status\"]").kind {
        Kind::Enum { values, .. } => assert_eq!(values, &["open", "paid"]),
        other => panic!("{other:?}"),
    }
}

/// Schema subsets: annotations carry documentation and nothing else.
#[test]
fn documentation_becomes_the_description() {
    let spec = lowered(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r" type="T"/>
          <xs:complexType name="T">
            <xs:annotation><xs:documentation> A thing. </xs:documentation></xs:annotation>
            <xs:sequence><xs:element name="a" type="xs:string"/></xs:sequence>
          </xs:complexType>
        </xs:schema>"#,
    );
    assert_eq!(at(&spec, "").description.as_deref(), Some("A thing."));
}

/// Notes on the XSD side: one namespace, and what the root says about it.
#[test]
fn a_target_namespace_and_the_form_of_local_elements_are_recorded() {
    let spec = lowered(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:t="urn:t"
              targetNamespace="urn:t" elementFormDefault="qualified">
          <xs:element name="r" type="t:R"/>
          <xs:complexType name="R"><xs:sequence>
            <xs:element name="a" type="xs:string"/>
          </xs:sequence></xs:complexType>
        </xs:schema>"#,
    );
    let info = spec.xml().unwrap();
    assert_eq!(info.target_namespace.as_deref(), Some("urn:t"));
    assert!(info.qualified);
    assert_eq!(field_names(at(&spec, "")), ["a"]);
}

#[test]
fn a_default_namespace_for_the_schema_namespace_works_too() {
    let spec = lowered(
        r#"<schema xmlns="http://www.w3.org/2001/XMLSchema">
          <element name="r"><complexType><sequence>
            <element name="a" type="string"/>
          </sequence></complexType></element>
        </schema>"#,
    );
    assert_eq!(field_names(at(&spec, "")), ["a"]);
}

#[test]
fn a_prefix_of_any_name_means_the_xml_schema_namespace() {
    let spec = lowered(
        r#"<x:schema xmlns:x="http://www.w3.org/2001/XMLSchema">
          <x:element name="r" type="x:integer"/>
        </x:schema>"#,
    );
    assert!(matches!(at(&spec, "").kind, Kind::Integer(_)));
}

// --------------------------------------------------------- what is refused

/// Schema subsets: each excluded construct fails with its name and a reason.
#[test]
fn excluded_constructs_are_refused_by_name() {
    let cases: &[(&str, &str, &str)] = &[
        (
            r#"<xs:choice><xs:element name="a" type="xs:string"/></xs:choice>"#,
            "choice",
            "alternatives",
        ),
        (r#"<xs:all/>"#, "all", "alternatives"),
        (
            r#"<xs:sequence><xs:any/></xs:sequence>"#,
            "any",
            "alternatives",
        ),
        (r#"<xs:group ref="g"/>"#, "group", "groups"),
        (r#"<xs:complexContent/>"#, "complexContent", "derivation"),
        (r#"<xs:anyAttribute/>"#, "anyAttribute", "open content"),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:string" nillable="true"/></xs:sequence>"#,
            "nillable",
            "nullable",
        ),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:string" default="x"/></xs:sequence>"#,
            "default",
            "default",
        ),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:string" fixed="x"/></xs:sequence>"#,
            "fixed",
            "default",
        ),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:string"/></xs:sequence><xs:attribute name="b" type="xs:string" default="x"/>"#,
            "default",
            "default",
        ),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:time"/></xs:sequence>"#,
            "type",
            "`time`",
        ),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:anyType"/></xs:sequence>"#,
            "type",
            "any content",
        ),
        (
            r#"<xs:sequence><xs:element name="a"><xs:simpleType><xs:list itemType="xs:string"/></xs:simpleType></xs:element></xs:sequence>"#,
            "list",
            "list types",
        ),
        (
            r#"<xs:sequence><xs:element name="a"><xs:simpleType><xs:union memberTypes="xs:string"/></xs:simpleType></xs:element></xs:sequence>"#,
            "union",
            "alternatives",
        ),
        (
            r#"<xs:sequence><xs:element name="a"><xs:simpleType><xs:restriction base="xs:integer"><xs:minExclusive value="1"/></xs:restriction></xs:simpleType></xs:element></xs:sequence>"#,
            "minExclusive",
            "",
        ),
        (
            r#"<xs:sequence><xs:element name="a"><xs:simpleType><xs:restriction base="xs:string"><xs:length value="1"/></xs:restriction></xs:simpleType></xs:element></xs:sequence>"#,
            "length",
            "",
        ),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:string" maxOccurs="0"/></xs:sequence>"#,
            "maxOccurs",
            "0 times",
        ),
        (r#"<xs:sequence maxOccurs="2"/>"#, "maxOccurs", "repeats"),
        (
            r#"<xs:attribute name="b" type="xs:string" use="prohibited"/>"#,
            "use",
            "prohibited",
        ),
        (
            r#"<xs:sequence><xs:element name="a" type="xs:string"><xs:key name="k"/></xs:element></xs:sequence>"#,
            "key",
            "identity constraints",
        ),
    ];
    for (body, keyword, reason) in cases {
        let e = refused(&schema(body));
        assert_eq!(e.keyword, *keyword, "{body}: {e}");
        assert!(e.message.contains(reason), "{body}: {e}");
        assert!(!e.location.is_empty());
    }
    // Mixed content, as the attribute and as text.
    let e = refused(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r"><xs:complexType mixed="true"><xs:sequence/></xs:complexType></xs:element>
        </xs:schema>"#,
    );
    assert_eq!(e.keyword, "mixed");
    // Imports and includes.
    let e = refused(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:import namespace="urn:x"/><xs:element name="r" type="xs:string"/></xs:schema>"#,
    );
    assert_eq!(e.keyword, "import");
}

#[test]
fn the_shape_of_the_document_is_checked() {
    // Two global elements, none, an unnamed type, a type in another namespace.
    let two = refused(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="a" type="xs:string"/><xs:element name="b" type="xs:string"/></xs:schema>"#,
    );
    assert!(two.message.contains("one global"), "{two}");
    let none = refused(r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"/>"#);
    assert!(none.message.contains("one global"), "{none}");
    let missing = refused(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="a" type="Nope"/></xs:schema>"#,
    );
    assert!(
        missing.message.contains("no type named `Nope`"),
        "{missing}"
    );
    let not_a_schema = refused(r#"<root/>"#);
    assert!(
        not_a_schema.message.contains("XML Schema namespace"),
        "{not_a_schema}"
    );
    let other_root =
        refused(r#"<xs:element xmlns:xs="http://www.w3.org/2001/XMLSchema" name="a"/>"#);
    assert!(other_root.message.contains("xs:schema"), "{other_root}");
    let foreign = refused(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:o="urn:o">
          <xs:element name="a" type="o:T"/></xs:schema>"#,
    );
    assert!(foreign.message.contains("another namespace"), "{foreign}");
}

#[test]
fn a_typo_in_an_attribute_or_an_element_is_an_error() {
    let e = refused(&schema(
        r#"<xs:sequence><xs:element name="a" type="xs:string" minOcurs="0"/></xs:sequence>"#,
    ));
    assert_eq!(e.keyword, "minOcurs");
    let e = refused(&schema(r#"<xs:sequnce/>"#));
    assert_eq!(e.keyword, "sequnce");
}

#[test]
fn names_and_occurrences_are_checked() {
    let e = refused(&schema(
        r#"<xs:sequence><xs:element name="a" type="xs:string"/><xs:element name="a" type="xs:string"/></xs:sequence>"#,
    ));
    assert!(e.message.contains("two elements named `a`"), "{e}");
    let e = refused(&schema(
        r#"<xs:attribute name="a" type="xs:string"/><xs:attribute name="a" type="xs:string"/>"#,
    ));
    assert!(e.message.contains("two attributes named `a`"), "{e}");
    let e = refused(&schema(
        r#"<xs:sequence><xs:element name="a" type="xs:string" minOccurs="3" maxOccurs="2"/></xs:sequence>"#,
    ));
    assert_eq!(e.keyword, "minOccurs");
    let e = refused(&schema(
        r#"<xs:sequence><xs:element name="a" type="xs:string" minOccurs="x"/></xs:sequence>"#,
    ));
    assert_eq!(e.keyword, "minOccurs");
    let e = refused(&schema(
        r#"<xs:sequence><xs:element name="a"/></xs:sequence>"#,
    ));
    assert_eq!(e.keyword, "type");
    let e = refused(&schema(
        r#"<xs:attribute name="a"><xs:simpleType><xs:restriction base="xs:string"/></xs:simpleType></xs:attribute><xs:attribute name="b" type="Nope"/>"#,
    ));
    assert!(e.message.contains("Nope"), "{e}");
}

#[test]
fn an_attribute_cannot_have_a_complex_type() {
    let e = refused(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r"><xs:complexType><xs:attribute name="a" type="C"/></xs:complexType></xs:element>
          <xs:complexType name="C"><xs:sequence/></xs:complexType>
        </xs:schema>"#,
    );
    assert!(e.message.contains("simple type"), "{e}");
}

#[test]
fn facets_that_do_not_fit_the_base_are_refused() {
    for (base, facet) in [
        ("xs:integer", r#"<xs:pattern value="[0-9]+"/>"#),
        ("xs:integer", r#"<xs:enumeration value="1"/>"#),
        ("xs:boolean", r#"<xs:minLength value="1"/>"#),
        ("xs:date", r#"<xs:minInclusive value="2020-01-01"/>"#),
        ("xs:string", r#"<xs:minInclusive value="1"/>"#),
        ("xs:integer", r#"<xs:minInclusive value="1.5"/>"#),
        (
            "xs:integer",
            r#"<xs:minInclusive value="5"/><xs:maxInclusive value="1"/>"#,
        ),
        (
            "xs:string",
            r#"<xs:minLength value="5"/><xs:maxLength value="1"/>"#,
        ),
        ("xs:string", r#"<xs:pattern value="("/>"#),
    ] {
        let text = schema(&format!(
            r#"<xs:sequence><xs:element name="a"><xs:simpleType><xs:restriction base="{base}">{facet}</xs:restriction></xs:simpleType></xs:element></xs:sequence>"#
        ));
        assert!(
            matches!(lower_str(&text), Err(LoadError::Schema(_))),
            "{base} {facet}"
        );
    }
}

#[test]
fn a_restriction_of_a_restriction_can_only_tighten() {
    let spec = lowered(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r" type="Small"/>
          <xs:simpleType name="Small"><xs:restriction base="Medium"><xs:maxInclusive value="5"/></xs:restriction></xs:simpleType>
          <xs:simpleType name="Medium"><xs:restriction base="xs:integer">
            <xs:minInclusive value="1"/><xs:maxInclusive value="10"/>
          </xs:restriction></xs:simpleType>
        </xs:schema>"#,
    );
    let Kind::Integer(r) = &at(&spec, "").kind else {
        panic!()
    };
    assert_eq!((r.min, r.max), (Some(Num::Int(1)), Some(Num::Int(5))));
    // A looser bound does not widen it.
    let spec = lowered(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="r" type="Loose"/>
          <xs:simpleType name="Loose"><xs:restriction base="Medium"><xs:maxInclusive value="50"/></xs:restriction></xs:simpleType>
          <xs:simpleType name="Medium"><xs:restriction base="xs:integer">
            <xs:maxInclusive value="10"/>
          </xs:restriction></xs:simpleType>
        </xs:schema>"#,
    );
    let Kind::Integer(r) = &at(&spec, "").kind else {
        panic!()
    };
    assert_eq!(r.max, Some(Num::Int(10)));
}

#[test]
fn not_xml_and_hostile_nesting_are_errors_not_crashes() {
    assert!(matches!(lower_str("<xs:schema"), Err(LoadError::NotXml(_))));
    let deep = format!(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="r" type="xs:string"/>{}{}</xs:schema>"#,
        "<xs:annotation>".repeat(500),
        "</xs:annotation>".repeat(500)
    );
    assert!(lower_str(&deep).is_err());
}

#[test]
fn a_schema_held_as_a_tree_lowers_the_same_as_its_text() {
    // The tree that the meta-schema types: attributes as `@` fields.
    use crate::value::Node;
    let element = |name: &str, ty: &str| {
        Node::new(Value::Object(
            [
                ("@name".to_string(), Node::new(Value::Str(name.into()))),
                ("@type".to_string(), Node::new(Value::Str(ty.into()))),
            ]
            .into_iter()
            .collect(),
        ))
    };
    let sequence = Node::new(Value::Object(
        [(
            "element".to_string(),
            Node::new(Value::Array(vec![
                element("a", "xs:string"),
                element("b", "xs:integer"),
            ])),
        )]
        .into_iter()
        .collect(),
    ));
    let complex = Node::new(Value::Object(
        [("sequence".to_string(), sequence)].into_iter().collect(),
    ));
    let root_element = Node::new(Value::Object(
        [
            ("@name".to_string(), Node::new(Value::Str("r".into()))),
            ("complexType".to_string(), complex),
        ]
        .into_iter()
        .collect(),
    ));
    let doc = Node::new(Value::Object(
        [("element".to_string(), root_element)]
            .into_iter()
            .collect(),
    ));
    let spec = lower_node(&doc, &[(Some("xs".to_string()), XSD_NS.to_string())]).unwrap();
    assert_eq!(field_names(at(&spec, "")), ["a", "b"]);
    assert!(matches!(at(&spec, "b").kind, Kind::Integer(_)));
}
