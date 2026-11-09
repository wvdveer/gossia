//! The bundled meta-schemas: schemas that describe schema documents.
//!
//! See DESIGN.md, "Schema binding". The meta-schema is not the official one;
//! it describes exactly the subset that [`crate::jsonschema`] accepts, written
//! in that subset, and goes through the same lowering as any other schema.
//! Unions cannot be written, so it can only approximate: `additionalProperties`
//! is a schema here (the `false` form is not offered), and whatever the shape
//! cannot say is caught by lowering the document, which is the authority.

use std::sync::OnceLock;

use crate::jsonschema;
use crate::typespec::TypeSpec;
use crate::xsd;

/// What a schema object may hold, as the `properties` of an object type. Shared
/// by the root, which adds `$defs` and `$schema`, and by every subschema.
const KEYWORDS: &str = r##"
    "type": {"type": "string", "enum": ["object", "array", "string", "integer", "number", "boolean"]},
    "title": {"type": "string"},
    "description": {"type": "string"},
    "properties": {"type": "object", "additionalProperties": {"$ref": "#/$defs/Schema"}},
    "required": {"type": "array", "items": {"type": "string"}},
    "items": {"$ref": "#/$defs/Schema"},
    "additionalProperties": {"$ref": "#/$defs/Schema"},
    "minItems": {"type": "integer", "minimum": 0},
    "maxItems": {"type": "integer", "minimum": 0},
    "minimum": {"type": "number"},
    "maximum": {"type": "number"},
    "minLength": {"type": "integer", "minimum": 0},
    "maxLength": {"type": "integer", "minimum": 0},
    "pattern": {"type": "string"},
    "format": {"type": "string", "enum": ["date", "date-time"]},
    "enum": {"type": "array", "items": {"type": "string"}},
    "$ref": {"type": "string"}
"##;

/// The text of the JSON Schema meta-schema (`builtin:jsonschema`).
pub fn jsonschema_source() -> String {
    format!(
        r##"{{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$ref": "#/$defs/Root",
  "$defs": {{
    "Root": {{
      "type": "object",
      "title": "JSON Schema (Gossia subset)",
      "properties": {{
        "$schema": {{"type": "string"}},
        "$defs": {{"type": "object", "additionalProperties": {{"$ref": "#/$defs/Schema"}}}},
        {KEYWORDS}
      }}
    }},
    "Schema": {{
      "type": "object",
      "properties": {{
        {KEYWORDS}
      }}
    }}
  }}
}}"##
    )
}

/// The lowered meta-schema for JSON Schema documents, made once.
pub fn jsonschema_meta() -> &'static TypeSpec {
    static META: OnceLock<TypeSpec> = OnceLock::new();
    META.get_or_init(|| {
        jsonschema::lower_str(&jsonschema_source()).expect("the bundled meta-schema is valid")
    })
}

/// The text of the XSD meta-schema (`builtin:xsd`): an XML Schema that
/// describes XML Schema documents of the supported subset. Its target namespace
/// is the XML Schema namespace, because the documents it describes are in it.
pub fn xsd_source() -> &'static str {
    r##"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           targetNamespace="http://www.w3.org/2001/XMLSchema"
           elementFormDefault="qualified">
  <xs:element name="schema" type="xs:Schema"/>

  <xs:complexType name="Schema">
    <xs:sequence>
      <xs:element name="annotation" type="xs:Annotation" minOccurs="0"/>
      <xs:element name="import" type="xs:Import" minOccurs="0" maxOccurs="unbounded"/>
      <xs:element name="element" type="xs:Element" maxOccurs="unbounded"/>
      <xs:element name="attribute" type="xs:Attribute" minOccurs="0" maxOccurs="unbounded"/>
      <xs:element name="complexType" type="xs:ComplexType" minOccurs="0" maxOccurs="unbounded"/>
      <xs:element name="simpleType" type="xs:SimpleType" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="targetNamespace" type="xs:string"/>
    <xs:attribute name="elementFormDefault" type="xs:FormDefault"/>
    <xs:attribute name="attributeFormDefault" type="xs:FormDefault"/>
  </xs:complexType>

  <xs:complexType name="Import">
    <xs:attribute name="namespace" type="xs:string" use="required"/>
    <xs:attribute name="schemaLocation" type="xs:string"/>
  </xs:complexType>

  <xs:complexType name="Annotation">
    <xs:sequence>
      <xs:element name="documentation" type="xs:string" minOccurs="0"/>
    </xs:sequence>
  </xs:complexType>

  <xs:complexType name="Element">
    <xs:sequence>
      <xs:element name="annotation" type="xs:Annotation" minOccurs="0"/>
      <xs:element name="complexType" type="xs:ComplexType" minOccurs="0"/>
      <xs:element name="simpleType" type="xs:SimpleType" minOccurs="0"/>
    </xs:sequence>
    <xs:attribute name="name" type="xs:string"/>
    <xs:attribute name="ref" type="xs:string"/>
    <xs:attribute name="type" type="xs:string"/>
    <xs:attribute name="minOccurs" type="xs:Occurs"/>
    <xs:attribute name="maxOccurs" type="xs:MaxOccurs"/>
    <xs:attribute name="form" type="xs:FormDefault"/>
  </xs:complexType>

  <xs:complexType name="ComplexType">
    <xs:sequence>
      <xs:element name="annotation" type="xs:Annotation" minOccurs="0"/>
      <xs:element name="sequence" type="xs:Sequence" minOccurs="0"/>
      <xs:element name="choice" type="xs:Choice" minOccurs="0"/>
      <xs:element name="simpleContent" type="xs:SimpleContent" minOccurs="0"/>
      <xs:element name="attribute" type="xs:Attribute" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="name" type="xs:string"/>
    <xs:attribute name="mixed" type="xs:boolean"/>
  </xs:complexType>

  <xs:complexType name="Sequence">
    <xs:sequence>
      <xs:element name="annotation" type="xs:Annotation" minOccurs="0"/>
      <xs:element name="element" type="xs:Element" minOccurs="0" maxOccurs="unbounded"/>
      <xs:element name="any" type="xs:Any" minOccurs="0"/>
    </xs:sequence>
  </xs:complexType>

  <xs:complexType name="Choice">
    <xs:sequence>
      <xs:element name="annotation" type="xs:Annotation" minOccurs="0"/>
      <xs:element name="element" type="xs:Element" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="minOccurs" type="xs:Occurs"/>
    <xs:attribute name="maxOccurs" type="xs:MaxOccurs"/>
  </xs:complexType>

  <xs:complexType name="Any">
    <xs:attribute name="processContents" type="xs:ProcessContents"/>
    <xs:attribute name="minOccurs" type="xs:Occurs"/>
    <xs:attribute name="maxOccurs" type="xs:MaxOccurs"/>
    <xs:attribute name="namespace" type="xs:string"/>
  </xs:complexType>

  <xs:complexType name="SimpleContent">
    <xs:sequence>
      <xs:element name="extension" type="xs:Extension"/>
    </xs:sequence>
  </xs:complexType>

  <xs:complexType name="Extension">
    <xs:sequence>
      <xs:element name="attribute" type="xs:Attribute" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="base" type="xs:string" use="required"/>
  </xs:complexType>

  <xs:complexType name="Attribute">
    <xs:sequence>
      <xs:element name="annotation" type="xs:Annotation" minOccurs="0"/>
      <xs:element name="simpleType" type="xs:SimpleType" minOccurs="0"/>
    </xs:sequence>
    <xs:attribute name="name" type="xs:string"/>
    <xs:attribute name="ref" type="xs:string"/>
    <xs:attribute name="type" type="xs:string"/>
    <xs:attribute name="use" type="xs:Use"/>
    <xs:attribute name="form" type="xs:FormDefault"/>
  </xs:complexType>

  <xs:complexType name="SimpleType">
    <xs:sequence>
      <xs:element name="annotation" type="xs:Annotation" minOccurs="0"/>
      <xs:element name="restriction" type="xs:Restriction"/>
    </xs:sequence>
    <xs:attribute name="name" type="xs:string"/>
  </xs:complexType>

  <xs:complexType name="Restriction">
    <xs:sequence>
      <xs:element name="enumeration" type="xs:Facet" minOccurs="0" maxOccurs="unbounded"/>
      <xs:element name="pattern" type="xs:Facet" minOccurs="0" maxOccurs="unbounded"/>
      <xs:element name="minInclusive" type="xs:Facet" minOccurs="0"/>
      <xs:element name="maxInclusive" type="xs:Facet" minOccurs="0"/>
      <xs:element name="minLength" type="xs:Facet" minOccurs="0"/>
      <xs:element name="maxLength" type="xs:Facet" minOccurs="0"/>
    </xs:sequence>
    <xs:attribute name="base" type="xs:string" use="required"/>
  </xs:complexType>

  <xs:complexType name="Facet">
    <xs:attribute name="value" type="xs:string" use="required"/>
  </xs:complexType>

  <xs:simpleType name="FormDefault">
    <xs:restriction base="xs:string">
      <xs:enumeration value="qualified"/>
      <xs:enumeration value="unqualified"/>
    </xs:restriction>
  </xs:simpleType>

  <xs:simpleType name="ProcessContents">
    <xs:restriction base="xs:string">
      <xs:enumeration value="skip"/>
    </xs:restriction>
  </xs:simpleType>

  <xs:simpleType name="Use">
    <xs:restriction base="xs:string">
      <xs:enumeration value="optional"/>
      <xs:enumeration value="required"/>
    </xs:restriction>
  </xs:simpleType>

  <xs:simpleType name="Occurs">
    <xs:restriction base="xs:string">
      <xs:pattern value="[0-9]+"/>
    </xs:restriction>
  </xs:simpleType>

  <xs:simpleType name="MaxOccurs">
    <xs:restriction base="xs:string">
      <xs:pattern value="[0-9]+|unbounded"/>
    </xs:restriction>
  </xs:simpleType>
</xs:schema>"##
}

/// The lowered meta-schema for XSD documents, made once.
pub fn xsd_meta() -> &'static TypeSpec {
    static META: OnceLock<TypeSpec> = OnceLock::new();
    META.get_or_init(|| xsd::lower_str(xsd_source()).expect("the bundled meta-schema is valid"))
}

/// The text of the copybook meta-schema (`builtin:copybook`), a JSON
/// Schema: a copybook's code page and its items, each with its clauses and
/// the items below it. Lowering the copybook checks the rest.
pub fn copybook_source() -> String {
    let usages: Vec<String> = crate::copybook::Usage::ALL
        .iter()
        .map(|u| format!("\"{}\"", u.code()))
        .collect();
    let signs: Vec<String> = crate::copybook::Sign::CODES
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect();
    let pages: Vec<String> = crate::copybook::CodePage::ALL
        .iter()
        .map(|p| format!("\"{}\"", p.code()))
        .collect();
    let floats: Vec<String> = crate::copybook::FloatFormat::ALL
        .iter()
        .map(|f| format!("\"{}\"", f.code()))
        .collect();
    format!(
        r##"{{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "COBOL copybook",
  "properties": {{
    "codePage": {{"type": "string", "enum": [{pages}]}},
    "floatFormat": {{"type": "string", "enum": [{floats}]}},
    "items": {{"type": "array", "items": {{"$ref": "#/$defs/Item"}}}}
  }},
  "required": ["codePage", "items"],
  "$defs": {{
    "Item": {{
      "type": "object",
      "properties": {{
        "level": {{"type": "integer", "minimum": 1, "maximum": 77}},
        "name": {{"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9-]*$"}},
        "redefines": {{"type": "string"}},
        "picture": {{"type": "string"}},
        "usage": {{"type": "string", "enum": [{usages}]}},
        "sync": {{"type": "boolean"}},
        "sign": {{"type": "string", "enum": [{signs}]}},
        "occursFrom": {{"type": "integer", "minimum": 0}},
        "occurs": {{"type": "integer", "minimum": 1}},
        "dependingOn": {{"type": "string"}},
        "value": {{"type": "string"}},
        "other": {{"type": "string"}},
        "conditions": {{"type": "array", "items": {{"$ref": "#/$defs/Condition"}}}},
        "items": {{"type": "array", "items": {{"$ref": "#/$defs/Item"}}}},
        "renames": {{"type": "array", "items": {{"$ref": "#/$defs/Renames"}}}}
      }},
      "required": ["level", "name"]
    }},
    "Condition": {{
      "type": "object",
      "properties": {{
        "name": {{"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9-]*$"}},
        "values": {{"type": "string"}}
      }},
      "required": ["name", "values"]
    }},
    "Renames": {{
      "type": "object",
      "properties": {{
        "name": {{"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9-]*$"}},
        "from": {{"type": "string"}},
        "thru": {{"type": "string"}}
      }},
      "required": ["name", "from"]
    }}
  }}
}}"##,
        pages = pages.join(", "),
        floats = floats.join(", "),
        usages = usages.join(", "),
        signs = signs.join(", "),
    )
}

/// The lowered meta-schema for copybook schema tabs, made once.
pub fn copybook_meta() -> &'static TypeSpec {
    static META: OnceLock<TypeSpec> = OnceLock::new();
    META.get_or_init(|| {
        jsonschema::lower_str(&copybook_source()).expect("the bundled meta-schema is valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use crate::validate::validate;

    const SAMPLE: &str = r##"{
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "type": "object",
      "title": "Order",
      "properties": {
        "customer": {"$ref": "#/$defs/Party"},
        "items": {"type": "array", "items": {"type": "object",
          "properties": {"sku": {"type": "string"}, "qty": {"type": "integer", "minimum": 1}},
          "required": ["sku"]}},
        "prices": {"type": "object", "additionalProperties": {"type": "number"}},
        "status": {"type": "string", "enum": ["new", "paid"]}
      },
      "required": ["customer"],
      "$defs": {"Party": {"type": "object", "properties": {"name": {"type": "string"}}}}
    }"##;

    #[test]
    fn the_meta_schema_lowers_and_accepts_a_schema_in_the_subset() {
        let meta = jsonschema_meta();
        let doc = json::parse(SAMPLE).unwrap().node;
        let issues = validate(meta, &doc);
        assert!(issues.is_empty(), "{issues:?}");
        // The document it accepted is one that lowers.
        assert!(jsonschema::lower(&doc).is_ok());
    }

    #[test]
    fn the_meta_schema_refuses_what_is_not_in_the_subset() {
        let meta = jsonschema_meta();
        let doc = json::parse(r#"{"type": "object", "oneOf": []}"#)
            .unwrap()
            .node;
        assert!(!validate(meta, &doc).is_empty());
        let doc = json::parse(r#"{"type": "float"}"#).unwrap().node;
        assert!(!validate(meta, &doc).is_empty());
    }

    // ------------------------------------------------------------ the XSD one

    use crate::xml;
    use crate::xsd::tests::ORDER;

    #[test]
    fn the_xsd_meta_schema_lowers_and_describes_the_xml_schema_namespace() {
        let meta = xsd_meta();
        let info = meta.xml().unwrap();
        assert_eq!(info.root, "schema");
        assert_eq!(info.target_namespace.as_deref(), Some(xsd::XSD_NS));
        assert!(info.qualified);
    }

    #[test]
    fn a_schema_in_the_subset_reads_as_a_tree_the_meta_schema_accepts_and_that_lowers() {
        let meta = xsd_meta();
        let parsed = xml::parse(ORDER, meta).unwrap();
        let issues = validate(meta, &parsed.node);
        assert!(issues.is_empty(), "{issues:?}");
        // The tree lowers to the same schema as the text does.
        let from_tree = xsd::lower_node(&parsed.node, &parsed.document.declarations).unwrap();
        let from_text = xsd::lower_str(ORDER).unwrap();
        assert_eq!(from_tree.len(), from_text.len());
        assert_eq!(from_tree.xml(), from_text.xml());
    }

    #[test]
    fn the_meta_schema_flags_what_is_not_in_the_subset() {
        let meta = xsd_meta();
        for body in [
            "<xs:all/>",
            "<xs:group ref=\"g\"/>",
            "<xs:sequence><xs:element name=\"a\" type=\"xs:string\" nillable=\"true\"/></xs:sequence>",
            "<xs:sequence><xs:element name=\"a\" type=\"xs:string\" maxOccurs=\"many\"/></xs:sequence>",
            "<xs:attribute name=\"a\" type=\"xs:string\" use=\"prohibited\"/>",
        ] {
            let text = format!(
                "<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"><xs:element name=\"r\"><xs:complexType>{body}</xs:complexType></xs:element></xs:schema>"
            );
            let parsed = xml::parse(&text, meta).unwrap();
            assert!(!validate(meta, &parsed.node).is_empty(), "{body}");
        }
    }

    #[test]
    fn a_schema_tree_written_as_xml_is_the_schema_again() {
        let meta = xsd_meta();
        let parsed = xml::parse(ORDER, meta).unwrap();
        let text = xml::to_string(&parsed.node, meta, &parsed.document, true).unwrap();
        assert!(text.contains("<xs:schema"), "{text}");
        let spec = xsd::lower_str(&text).unwrap();
        assert_eq!(spec.xml().unwrap().root, "order");
        // A new document starts with the `xs` prefix bound.
        let fresh = xml::to_string(&parsed.node, meta, &xsd::default_document(), false).unwrap();
        assert!(xsd::lower_str(&fresh).is_ok(), "{fresh}");
    }
}
