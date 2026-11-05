//! Tests for JSON Schema lowering. Test names follow the sections of DESIGN.md.

use super::*;
use crate::typespec::Kind;

fn lower_ok(schema: &str) -> TypeSpec {
    lower_str(schema).unwrap_or_else(|e| panic!("should lower: {e}\n{schema}"))
}

fn lower_err(schema: &str) -> SchemaError {
    match lower_str(schema) {
        Err(LoadError::Schema(e)) => e,
        Err(other) => panic!("expected a schema error, got {other}"),
        Ok(_) => panic!("should not lower:\n{schema}"),
    }
}

/// Asserts that lowering fails naming `keyword` at `location`.
fn rejects(schema: &str, keyword: &str, location: &str) -> SchemaError {
    let e = lower_err(schema);
    assert_eq!(
        (e.keyword.as_str(), e.location.as_str()),
        (keyword, location),
        "{e}"
    );
    assert!(!e.message.is_empty());
    e
}

fn root_kind(spec: &TypeSpec) -> &Kind {
    &spec.get(spec.root()).kind
}

// --- Schema model: what lowering produces ---------------------------------

#[test]
fn an_object_keeps_declared_field_order_and_required_flags() {
    let spec = lower_ok(
        r#"{"type": "object",
            "properties": {"z": {"type": "string"}, "a": {"type": "integer"}, "m": {"type": "boolean"}},
            "required": ["m", "z"]}"#,
    );
    let Kind::Object { fields } = root_kind(&spec) else {
        panic!("object expected")
    };
    let summary: Vec<_> = fields
        .iter()
        .map(|f| (f.name.as_str(), f.required))
        .collect();
    assert_eq!(summary, [("z", true), ("a", false), ("m", true)]);
    assert!(matches!(spec.get(fields[0].ty).kind, Kind::String(_)));
    assert!(matches!(spec.get(fields[1].ty).kind, Kind::Integer(_)));
    assert!(matches!(spec.get(fields[2].ty).kind, Kind::Bool));
}

#[test]
fn every_simple_kind_lowers() {
    assert!(matches!(
        root_kind(&lower_ok(r#"{"type": "string"}"#)),
        Kind::String(_)
    ));
    assert!(matches!(
        root_kind(&lower_ok(r#"{"type": "integer"}"#)),
        Kind::Integer(_)
    ));
    assert!(matches!(
        root_kind(&lower_ok(r#"{"type": "number"}"#)),
        Kind::Number(_)
    ));
    assert!(matches!(
        root_kind(&lower_ok(r#"{"type": "boolean"}"#)),
        Kind::Bool
    ));
    assert!(matches!(
        root_kind(&lower_ok(
            r#"{"type": "array", "items": {"type": "string"}}"#
        )),
        Kind::Array { .. }
    ));
    assert!(matches!(
        root_kind(&lower_ok(r#"{"type": "object", "properties": {}}"#)),
        Kind::Object { .. }
    ));
}

#[test]
fn constraints_are_kept() {
    let spec = lower_ok(
        r#"{"type": "string", "pattern": "^a", "minLength": 1, "maxLength": 5, "format": "date"}"#,
    );
    let Kind::String(c) = root_kind(&spec) else {
        panic!("string expected")
    };
    assert_eq!(c.pattern.as_ref().unwrap().source(), "^a");
    assert_eq!(
        (c.min_length, c.max_length, c.format),
        (Some(1), Some(5), Some(StringFormat::Date))
    );

    let spec = lower_ok(r#"{"type": "integer", "minimum": -2, "maximum": 7.5}"#);
    let Kind::Integer(r) = root_kind(&spec) else {
        panic!("integer expected")
    };
    assert_eq!((r.min, r.max), (Some(Num::Int(-2)), Some(Num::Float(7.5))));

    let spec = lower_ok(
        r#"{"type": "array", "items": {"type": "integer"}, "minItems": 1, "maxItems": 3.0}"#,
    );
    let Kind::Array {
        min_items,
        max_items,
        ..
    } = root_kind(&spec)
    else {
        panic!("array expected")
    };
    assert_eq!((*min_items, *max_items), (Some(1), Some(3)));
}

#[test]
fn an_enum_is_a_string_kind_and_needs_no_type() {
    for schema in [
        r#"{"type": "string", "enum": ["a", "b"]}"#,
        r#"{"enum": ["a", "b"]}"#,
    ] {
        let spec = lower_ok(schema);
        let Kind::Enum { values, .. } = root_kind(&spec) else {
            panic!("enum expected")
        };
        assert_eq!(values, &["a", "b"]);
    }
}

#[test]
fn an_enum_can_carry_string_constraints() {
    let spec = lower_ok(r#"{"type": "string", "enum": ["ab", "c"], "minLength": 2}"#);
    let Kind::Enum { strings, .. } = root_kind(&spec) else {
        panic!("enum expected")
    };
    assert_eq!(strings.min_length, Some(2));
}

#[test]
fn additional_properties_with_a_schema_is_a_map() {
    let spec = lower_ok(r#"{"type": "object", "additionalProperties": {"type": "integer"}}"#);
    let Kind::Map { value } = root_kind(&spec) else {
        panic!("map expected")
    };
    assert!(matches!(spec.get(*value).kind, Kind::Integer(_)));
}

#[test]
fn additional_properties_false_says_what_objects_already_are() {
    let spec = lower_ok(
        r#"{"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false}"#,
    );
    assert!(matches!(root_kind(&spec), Kind::Object { fields } if fields.len() == 1));
    let spec = lower_ok(r#"{"type": "object", "additionalProperties": false}"#);
    assert!(matches!(root_kind(&spec), Kind::Object { fields } if fields.is_empty()));
}

#[test]
fn titles_and_descriptions_are_kept_as_annotations() {
    let spec = lower_ok(r#"{"type": "string", "title": "Name", "description": "Who."}"#);
    let def = spec.get(spec.root());
    assert_eq!(
        (def.title.as_deref(), def.description.as_deref()),
        (Some("Name"), Some("Who."))
    );
}

#[test]
fn each_type_remembers_where_it_was_written() {
    let spec = lower_ok(
        r##"{"type": "object", "properties": {"a": {"type": "array", "items": {"type": "string"}}},
             "$defs": {"X": {"type": "integer"}}}"##,
    );
    let sources: Vec<_> = (0..spec.len())
        .map(|i| spec.get(TypeId(i as u32)).source.clone())
        .collect();
    for expected in ["#", "#/properties/a", "#/properties/a/items", "#/$defs/X"] {
        assert!(
            sources.iter().any(|s| s == expected),
            "{expected} not in {sources:?}"
        );
    }
}

#[test]
fn a_property_name_with_a_slash_is_escaped_in_locations() {
    let e = lower_err(r#"{"type": "object", "properties": {"a/b": {"type": "oops"}}}"#);
    assert_eq!(e.location, "#/properties/a~1b");
}

// --- References ------------------------------------------------------------

#[test]
fn a_ref_shares_the_type_it_names() {
    let spec = lower_ok(
        r##"{"type": "object",
             "properties": {"billing": {"$ref": "#/$defs/Address"}, "shipping": {"$ref": "#/$defs/Address"}},
             "$defs": {"Address": {"type": "object", "properties": {"city": {"type": "string"}}}}}"##,
    );
    let Kind::Object { fields } = root_kind(&spec) else {
        panic!("object expected")
    };
    assert_eq!(fields[0].ty, fields[1].ty);
    assert_eq!(spec.get(fields[0].ty).name.as_deref(), Some("Address"));
}

#[test]
fn a_type_can_refer_to_itself() {
    let spec = lower_ok(
        r##"{"$ref": "#/$defs/Folder", "$defs": {"Folder": {"type": "object",
             "properties": {"children": {"type": "array", "items": {"$ref": "#/$defs/Folder"}}}}}}"##,
    );
    let Kind::Object { fields } = root_kind(&spec) else {
        panic!("root is the Folder type")
    };
    let Kind::Array { items, .. } = spec.get(fields[0].ty).kind else {
        panic!("array expected")
    };
    assert_eq!(items, spec.root());
}

#[test]
fn the_root_can_refer_to_itself_with_a_hash() {
    let spec = lower_ok(
        r##"{"type": "object", "properties": {"next": {"type": "array", "items": {"$ref": "#"}}}}"##,
    );
    let Kind::Object { fields } = root_kind(&spec) else {
        panic!("object expected")
    };
    let Kind::Array { items, .. } = spec.get(fields[0].ty).kind else {
        panic!("array expected")
    };
    assert_eq!(items, spec.root());
}

#[test]
fn an_alias_is_the_type_it_points_to() {
    let spec = lower_ok(
        r##"{"type": "object", "properties": {"a": {"$ref": "#/$defs/A"}, "b": {"$ref": "#/$defs/B"}},
             "$defs": {"A": {"$ref": "#/$defs/B"}, "B": {"type": "string"}}}"##,
    );
    let Kind::Object { fields } = root_kind(&spec) else {
        panic!("object expected")
    };
    assert_eq!(fields[0].ty, fields[1].ty);
}

#[test]
fn a_ref_may_carry_a_title_and_a_description() {
    lower_ok(
        r##"{"type": "object", "properties": {"a": {"$ref": "#/$defs/A", "title": "t", "description": "d"}},
             "$defs": {"A": {"type": "string"}}}"##,
    );
}

#[test]
fn a_ref_with_other_keywords_is_refused() {
    rejects(
        r##"{"type": "object", "properties": {"a": {"$ref": "#/$defs/A", "minLength": 1}}, "$defs": {"A": {"type": "string"}}}"##,
        "minLength",
        "#/properties/a",
    );
}

#[test]
fn refs_to_anything_but_hash_or_defs_are_refused() {
    for target in [
        "#/properties/a",
        "http://example.com/schema.json",
        "other.json",
        "#foo",
        "#/$defs/A/properties/x",
    ] {
        let schema = format!(
            r##"{{"type": "object", "properties": {{"a": {{"$ref": "{target}"}}}}, "$defs": {{"A": {{"type": "string"}}}}}}"##
        );
        let e = lower_err(&schema);
        assert_eq!(
            (e.keyword.as_str(), e.location.as_str()),
            ("$ref", "#/properties/a"),
            "{target}"
        );
    }
}

#[test]
fn a_ref_to_a_missing_definition_names_it() {
    let e = rejects(
        r##"{"type": "object", "properties": {"a": {"$ref": "#/$defs/Nope"}}}"##,
        "$ref",
        "#/properties/a",
    );
    assert!(e.message.contains("Nope"));
}

#[test]
fn references_that_only_point_at_each_other_are_a_cycle() {
    let e = lower_err(
        r##"{"$ref": "#/$defs/A", "$defs": {"A": {"$ref": "#/$defs/B"}, "B": {"$ref": "#/$defs/A"}}}"##,
    );
    assert_eq!(e.keyword, "$ref");
    let e = lower_err(r##"{"$ref": "#"}"##);
    assert_eq!(e.keyword, "$ref");
}

#[test]
fn mistakes_in_unused_definitions_still_fail_the_load() {
    rejects(
        r##"{"type": "string", "$defs": {"Unused": {"type": "oops"}}}"##,
        "type",
        "#/$defs/Unused",
    );
}

#[test]
fn defs_are_read_at_the_root_only() {
    rejects(
        r##"{"type": "object", "properties": {"a": {"type": "string", "$defs": {}}}}"##,
        "$defs",
        "#/properties/a",
    );
}

// --- Schema subsets: excluded constructs are refused, by name ---------------

#[test]
fn every_excluded_keyword_is_refused_naming_it_and_where() {
    let excluded = [
        "oneOf",
        "anyOf",
        "allOf",
        "not",
        "if",
        "then",
        "else",
        "dependentSchemas",
        "dependentRequired",
        "dependencies",
        "patternProperties",
        "unevaluatedProperties",
        "unevaluatedItems",
        "propertyNames",
        "$id",
        "$anchor",
        "$dynamicRef",
        "$dynamicAnchor",
        "$recursiveRef",
        "$recursiveAnchor",
        "$vocabulary",
        "const",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "uniqueItems",
        "prefixItems",
        "contains",
        "minContains",
        "maxContains",
        "minProperties",
        "maxProperties",
        "$comment",
        "default",
        "examples",
        "deprecated",
        "readOnly",
        "writeOnly",
        "contentEncoding",
        "contentMediaType",
        "contentSchema",
    ];
    for keyword in excluded {
        let at_root = format!(r#"{{"type": "string", "{keyword}": 1}}"#);
        let e = lower_err(&at_root);
        assert_eq!(
            (e.keyword.as_str(), e.location.as_str()),
            (keyword, "#"),
            "{at_root}"
        );
        // And nested, where the location must follow.
        let nested =
            format!(r#"{{"type": "array", "items": {{"type": "string", "{keyword}": 1}}}}"#);
        let e = lower_err(&nested);
        assert_eq!(
            (e.keyword.as_str(), e.location.as_str()),
            (keyword, "#/items"),
            "{nested}"
        );
    }
}

#[test]
fn an_excluded_keyword_is_reported_before_a_missing_type() {
    // The root has no `type`, but the real problem is the `oneOf` below it.
    rejects(
        r#"{"properties": {"a": {"oneOf": [{"type": "string"}]}}}"#,
        "oneOf",
        "#/properties/a",
    );
    rejects(
        r#"{"$defs": {"A": {"anyOf": []}}, "type": "string"}"#,
        "anyOf",
        "#/$defs/A",
    );
}

#[test]
fn unknown_keywords_are_errors_so_typos_show() {
    let e = rejects(r#"{"type": "string", "minLenght": 1}"#, "minLenght", "#");
    assert!(e.message.contains("recognise"));
    rejects(
        r#"{"type": "string", "x-internal": true}"#,
        "x-internal",
        "#",
    );
}

#[test]
fn null_and_type_lists_are_refused() {
    rejects(r#"{"type": "null"}"#, "type", "#");
    rejects(r#"{"type": ["string", "null"]}"#, "type", "#");
    rejects(r#"{"type": ["string"]}"#, "type", "#");
    rejects(r#"{"type": "text"}"#, "type", "#");
    rejects(r#"{"type": 3}"#, "type", "#");
}

/// A schema with no `type` is any value (owner, 2026-11-06), so a keyword
/// of one type alone is refused, and a description alone is fine.
#[test]
fn a_schema_without_a_type_takes_no_keywords_of_a_type() {
    rejects(r#"{"minimum": 1}"#, "minimum", "#");
    let spec = lower_ok(r#"{"type": "object", "properties": {"a": {"description": "no type"}}}"#);
    let a = spec.resolve(&"a".parse().unwrap()).unwrap();
    assert!(matches!(spec.get(a.ty).kind, Kind::Any));
}

#[test]
fn boolean_schemas_and_other_non_objects_are_refused() {
    rejects("true", "$schema", "#");
    rejects("[]", "$schema", "#");
    rejects(r#"{"type": "array", "items": true}"#, "items", "#");
    rejects(
        r#"{"type": "array", "items": [{"type": "string"}]}"#,
        "items",
        "#",
    );
    rejects(
        r#"{"type": "object", "properties": {"a": false}}"#,
        "type",
        "#/properties/a",
    );
}

#[test]
fn keywords_that_do_not_apply_to_the_type_are_refused() {
    rejects(r#"{"type": "string", "minimum": 1}"#, "minimum", "#");
    rejects(r#"{"type": "integer", "minLength": 1}"#, "minLength", "#");
    rejects(r#"{"type": "boolean", "enum": ["a"]}"#, "enum", "#");
    rejects(
        r#"{"type": "array", "items": {"type": "string"}, "properties": {}}"#,
        "properties",
        "#",
    );
    rejects(
        r#"{"type": "object", "properties": {}, "items": {"type": "string"}}"#,
        "items",
        "#",
    );
    rejects(r#"{"type": "number", "pattern": "a"}"#, "pattern", "#");
}

#[test]
fn enums_are_string_only_non_empty_and_without_duplicates() {
    rejects(r#"{"type": "string", "enum": [1, 2]}"#, "enum", "#");
    rejects(r#"{"type": "string", "enum": ["a", 2]}"#, "enum", "#");
    rejects(r#"{"enum": []}"#, "enum", "#");
    rejects(r#"{"enum": ["a", "a"]}"#, "enum", "#");
    rejects(r#"{"enum": "a"}"#, "enum", "#");
    rejects(r#"{"type": "integer", "enum": [1]}"#, "enum", "#");
    rejects(r#"{"enum": ["a"], "minimum": 1}"#, "minimum", "#");
}

#[test]
fn objects_are_closed() {
    rejects(r#"{"type": "object"}"#, "properties", "#");
    rejects(
        r#"{"type": "object", "additionalProperties": true}"#,
        "additionalProperties",
        "#",
    );
    rejects(
        r#"{"type": "object", "additionalProperties": 3}"#,
        "additionalProperties",
        "#",
    );
    rejects(
        r#"{"type": "object", "additionalProperties": "x"}"#,
        "additionalProperties",
        "#",
    );
}

#[test]
fn properties_and_a_schema_valued_additional_properties_are_not_combined() {
    rejects(
        r#"{"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": {"type": "string"}}"#,
        "additionalProperties",
        "#",
    );
    rejects(
        r#"{"type": "object", "additionalProperties": {"type": "string"}, "required": ["a"]}"#,
        "required",
        "#",
    );
}

#[test]
fn required_names_must_be_declared_and_unique() {
    let e = rejects(
        r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": ["b"]}"#,
        "required",
        "#",
    );
    assert!(e.message.contains('b'));
    rejects(
        r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a", "a"]}"#,
        "required",
        "#",
    );
    rejects(
        r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": [1]}"#,
        "required",
        "#",
    );
    rejects(
        r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": "a"}"#,
        "required",
        "#",
    );
    rejects(r#"{"type": "object", "properties": []}"#, "properties", "#");
}

#[test]
fn an_array_needs_items() {
    rejects(r#"{"type": "array"}"#, "items", "#");
    rejects(r#"{"type": "array", "minItems": 1}"#, "items", "#");
}

#[test]
fn counts_must_be_non_negative_whole_numbers() {
    for bad in ["-1", "1.5", "\"1\"", "null", "true"] {
        let schema =
            format!(r#"{{"type": "array", "items": {{"type": "string"}}, "minItems": {bad}}}"#);
        assert_eq!(lower_err(&schema).keyword, "minItems", "{bad}");
    }
    rejects(r#"{"type": "string", "maxLength": -1}"#, "maxLength", "#");
}

#[test]
fn bounds_must_be_numbers() {
    rejects(r#"{"type": "integer", "minimum": "1"}"#, "minimum", "#");
    rejects(r#"{"type": "number", "maximum": null}"#, "maximum", "#");
}

#[test]
fn only_date_and_date_time_formats_are_supported() {
    lower_ok(r#"{"type": "string", "format": "date"}"#);
    lower_ok(r#"{"type": "string", "format": "date-time"}"#);
    for format in ["email", "uri", "time", "uuid", ""] {
        let schema = format!(r#"{{"type": "string", "format": "{format}"}}"#);
        assert_eq!(lower_err(&schema).keyword, "format", "{format}");
    }
    rejects(r#"{"type": "string", "format": 1}"#, "format", "#");
}

#[test]
fn patterns_must_compile() {
    rejects(r#"{"type": "string", "pattern": "("}"#, "pattern", "#");
    // Look-ahead is not supported by the regular-expression engine.
    rejects(r#"{"type": "string", "pattern": "^(?=a)"}"#, "pattern", "#");
    rejects(r#"{"type": "string", "pattern": 1}"#, "pattern", "#");
}

#[test]
fn annotations_must_be_strings() {
    rejects(r#"{"type": "string", "title": 1}"#, "title", "#");
    rejects(
        r#"{"type": "string", "description": []}"#,
        "description",
        "#",
    );
}

#[test]
fn the_schema_keyword_may_name_draft_2020_12_at_the_root_only() {
    lower_ok(r#"{"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"}"#);
    lower_ok(r#"{"$schema": "https://json-schema.org/draft/2020-12/schema#", "type": "string"}"#);
    rejects(
        r#"{"$schema": "http://json-schema.org/draft-07/schema#", "type": "string"}"#,
        "$schema",
        "#",
    );
    rejects(r#"{"$schema": 1, "type": "string"}"#, "$schema", "#");
    rejects(
        r#"{"type": "array", "items": {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"}}"#,
        "$schema",
        "#/items",
    );
}

#[test]
fn deep_nesting_is_refused_not_a_stack_overflow() {
    let depth = 300;
    // 300 levels also exceeds the JSON reader's own limit, so build the tree directly.
    let mut node = Node::new(Value::Object(
        [("type".to_string(), Node::new(Value::Str("string".into())))]
            .into_iter()
            .collect(),
    ));
    for _ in 0..depth {
        node = Node::new(Value::Object(
            [
                ("type".to_string(), Node::new(Value::Str("array".into()))),
                ("items".to_string(), node),
            ]
            .into_iter()
            .collect(),
        ));
    }
    let e = lower(&node).unwrap_err();
    assert!(e.message.contains("deep"), "{e}");
}

#[test]
fn text_that_is_not_json_is_reported_as_such() {
    assert!(matches!(lower_str("{"), Err(LoadError::NotJson(_))));
}

#[test]
fn errors_read_as_location_keyword_and_reason() {
    let e = lower_err(r#"{"type": "string", "oneOf": []}"#);
    assert_eq!(
        e.to_string(),
        "#: `oneOf`: unions and combinators are not supported in version 1"
    );
}

/// A schema with no `type` is any value, and what is inside it is too; its
/// type is described as `any`. Keywords of a type do not apply to it.
#[test]
fn a_schema_with_no_type_is_any_value() {
    let spec = lower_ok(
        r#"{"type": "object", "properties": {"postcode": {}, "extra": {"title": "Extra"}}}"#,
    );
    let postcode = spec.resolve(&"postcode".parse().unwrap()).unwrap();
    assert!(matches!(spec.get(postcode.ty).kind, Kind::Any));
    assert_eq!(spec.describe(postcode.ty), "any");
    let inside = spec.resolve(&"postcode.a[2].b".parse().unwrap()).unwrap();
    assert_eq!(inside.ty, postcode.ty, "inside an any is any");
    for value in [
        r#"{"postcode": 73568}"#,
        r#"{"postcode": "Z26 7XT"}"#,
        r#"{"postcode": {"a": [1, "x", true]}}"#,
    ] {
        let data = crate::json::parse(value).unwrap().node;
        assert!(
            crate::validate::validate(&spec, &data).is_empty(),
            "{value}"
        );
    }
    rejects(
        r#"{"type": "object", "properties": {"n": {"minimum": 1}}}"#,
        "minimum",
        "#/properties/n",
    );
}
