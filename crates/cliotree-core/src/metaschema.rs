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
      "title": "JSON Schema (ClioTree subset)",
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
}
