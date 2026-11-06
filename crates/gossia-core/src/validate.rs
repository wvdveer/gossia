//! Checking a data tree against a [`TypeSpec`].
//!
//! The same check serves loaded data and formula results (see DESIGN.md,
//! "Result checking"): kind, required fields, exact field names, constraints.
//! The validator only reports. What to do with the report differs: a literal in
//! a loaded file that fails is kept and flagged, while a formula result that
//! fails becomes an error value.

use gossia_i18n::tr;
use std::cmp::Ordering;

use chrono::{DateTime, NaiveDate};

use crate::num::Num;
use crate::path::Path;
use crate::typespec::{Kind, Range, StringConstraints, StringFormat, TypeId, TypeSpec};
use crate::value::{Node, Value};

/// What sort of problem an [`Issue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// The value is of the wrong kind for the schema.
    TypeMismatch,
    /// A required field is absent.
    MissingRequired,
    /// An object has a field the schema does not declare.
    UndeclaredField,
    /// The value is of the right kind but breaks a constraint (range, length,
    /// pattern, enumeration, format, item count).
    Constraint,
}

/// One problem found in the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// The node with the problem. For a missing field, the object that lacks it.
    pub path: Path,
    pub kind: IssueKind,
    pub message: String,
}

/// Checks a whole tree against the root type. Nodes holding an error value are
/// skipped: they are reported as errors, not as invalid data.
pub fn validate(spec: &TypeSpec, root: &Node) -> Vec<Issue> {
    validate_at(spec, spec.root(), root, &Path::root())
}

/// Checks `node`, which sits at `path`, against the type `ty`.
pub fn validate_at(spec: &TypeSpec, ty: TypeId, node: &Node, path: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    check(spec, ty, node, path, &mut issues);
    issues
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::String(_) => "string",
        Kind::Integer(_) => "integer",
        Kind::Number(_) => "number",
        Kind::Bool => "boolean",
        Kind::Date => "date",
        Kind::DateTime => "dateTime",
        Kind::Array { .. } => "array",
        Kind::Object { .. } => "object",
        Kind::Map { .. } => "map",
        Kind::Enum { .. } => "string",
        Kind::Any => "any",
    }
}

fn check(spec: &TypeSpec, ty: TypeId, node: &Node, path: &Path, issues: &mut Vec<Issue>) {
    let def = spec.get(ty);
    let value = node.value();
    if matches!(value, Value::Error(_)) {
        return;
    }
    let mut push = |kind: IssueKind, message: String| {
        issues.push(Issue {
            path: path.clone(),
            kind,
            message,
        })
    };
    let mismatch = |push: &mut dyn FnMut(IssueKind, String)| {
        push(
            IssueKind::TypeMismatch,
            format!(
                "expected {}, got {}",
                kind_name(&def.kind),
                value.kind_name()
            ),
        );
    };
    match (&def.kind, value) {
        // Anything goes, at any depth (an error value was skipped above).
        (Kind::Any, _) => {}
        (Kind::String(c), Value::Str(s)) => check_string(c, s, &mut push),
        (Kind::Enum { values, strings }, Value::Str(s)) => {
            if !values.iter().any(|v| v == s) {
                push(
                    IssueKind::Constraint,
                    tr!(
                        "`{s}` is not one of the allowed values: {values}",
                        s = s,
                        values = values.join(", ")
                    ),
                );
            }
            check_string(strings, s, &mut push);
        }
        (Kind::Integer(r), Value::Int(i)) => check_range(r, Num::Int(*i), &mut push),
        (Kind::Number(r), Value::Int(i)) => check_range(r, Num::Int(*i), &mut push),
        (Kind::Number(r), Value::Float(x)) => check_number(r, *x, &mut push),
        (Kind::Bool, Value::Bool(_))
        | (Kind::Date, Value::Date(_))
        | (Kind::DateTime, Value::DateTime(_)) => {}
        (
            Kind::Array {
                items,
                min_items,
                max_items,
            },
            Value::Array(nodes),
        ) => {
            if min_items.is_some_and(|m| nodes.len() < m) {
                push(
                    IssueKind::Constraint,
                    tr!(
                        "{count} items, but at least {min_items} are required",
                        count = nodes.len(),
                        min_items = min_items.unwrap_or(0)
                    ),
                );
            }
            if max_items.is_some_and(|m| nodes.len() > m) {
                push(
                    IssueKind::Constraint,
                    tr!(
                        "{count} items, but at most {max_items} are allowed",
                        count = nodes.len(),
                        max_items = max_items.unwrap_or(0)
                    ),
                );
            }
            for (i, item) in nodes.iter().enumerate() {
                check(spec, *items, item, &path.index(i), issues);
            }
        }
        (Kind::Object { fields }, Value::Object(map)) => {
            for field in fields {
                match map.get(&field.name) {
                    Some(child) => {
                        check(spec, field.ty, child, &path.key(field.name.clone()), issues)
                    }
                    None if field.required => issues.push(Issue {
                        path: path.clone(),
                        kind: IssueKind::MissingRequired,
                        message: tr!("the required field `{name}` is missing", name = field.name),
                    }),
                    None => {}
                }
            }
            for key in map.keys() {
                if !fields.iter().any(|f| &f.name == key) {
                    issues.push(Issue {
                        path: path.key(key.clone()),
                        kind: IssueKind::UndeclaredField,
                        message: tr!("`{key}` is not a field of this object", key = key),
                    });
                }
            }
        }
        (Kind::Map { value: value_ty }, Value::Object(map)) => {
            for (key, child) in map {
                check(spec, *value_ty, child, &path.key(key.clone()), issues);
            }
        }
        _ => mismatch(&mut push),
    }
}

fn check_string(c: &StringConstraints, s: &str, push: &mut dyn FnMut(IssueKind, String)) {
    // Lengths count code points, not bytes.
    if c.min_length.is_some() || c.max_length.is_some() {
        let len = s.chars().count();
        if let Some(min) = c.min_length.filter(|min| len < *min) {
            push(
                IssueKind::Constraint,
                tr!(
                    "{len} characters, but at least {min} are required",
                    len = len,
                    min = min
                ),
            );
        }
        if let Some(max) = c.max_length.filter(|max| len > *max) {
            push(
                IssueKind::Constraint,
                tr!(
                    "{len} characters, but at most {max} are allowed",
                    len = len,
                    max = max
                ),
            );
        }
    }
    if let Some(pattern) = &c.pattern
        && !pattern.is_match(s)
    {
        push(
            IssueKind::Constraint,
            tr!(
                "does not match the pattern `{pattern}`",
                pattern = pattern.source()
            ),
        );
    }
    match c.format {
        Some(StringFormat::Date) if !is_date(s) => {
            push(
                IssueKind::Constraint,
                tr!("is not a date (expected YYYY-MM-DD)").to_string(),
            );
        }
        Some(StringFormat::DateTime) if !is_date_time(s) => {
            push(
                IssueKind::Constraint,
                tr!("is not a date and time (expected YYYY-MM-DDThh:mm:ssZ)").to_string(),
            );
        }
        _ => {}
    }
}

fn check_number(r: &Range, x: f64, push: &mut dyn FnMut(IssueKind, String)) {
    if !x.is_finite() {
        push(
            IssueKind::Constraint,
            tr!("is not a finite number").to_string(),
        );
        return;
    }
    check_range(r, Num::Float(x), push);
}

fn check_range(r: &Range, n: Num, push: &mut dyn FnMut(IssueKind, String)) {
    if let Some(min) = r.min.filter(|min| n.compare(*min) == Some(Ordering::Less)) {
        push(
            IssueKind::Constraint,
            tr!("{n} is below the minimum {min}", n = n, min = min),
        );
    }
    if let Some(max) = r
        .max
        .filter(|max| n.compare(*max) == Some(Ordering::Greater))
    {
        push(
            IssueKind::Constraint,
            tr!("{n} is above the maximum {max}", n = n, max = max),
        );
    }
}

/// An RFC 3339 full date: exactly `YYYY-MM-DD`, and a real day.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// An RFC 3339 date and time: a full date, `T` (or `t`), a time, and an offset.
fn is_date_time(s: &str) -> bool {
    let Some(separator) = s.get(10..11) else {
        return false;
    };
    if separator != "T" && separator != "t" {
        return false;
    }
    is_date(&s[..10]) && DateTime::parse_from_rfc3339(&format!("{}T{}", &s[..10], &s[11..])).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use crate::jsonschema::lower_str;

    fn spec(schema: &str) -> TypeSpec {
        lower_str(schema).unwrap_or_else(|e| panic!("schema should lower: {e}"))
    }

    fn data(text: &str) -> Node {
        json::parse(text).unwrap().node
    }

    fn issues(schema: &str, text: &str) -> Vec<(String, IssueKind)> {
        validate(&spec(schema), &data(text))
            .into_iter()
            .map(|i| (i.path.to_string(), i.kind))
            .collect()
    }

    fn ok(schema: &str, text: &str) {
        let found = issues(schema, text);
        assert!(found.is_empty(), "{text} should be valid, but: {found:?}");
    }

    const ORDER: &str = r#"{
        "type": "object",
        "properties": {
            "id": {"type": "integer", "minimum": 1},
            "customer": {"type": "string", "minLength": 1},
            "items": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {"sku": {"type": "string"}, "price": {"type": "number", "minimum": 0}},
                "required": ["sku"]
            }},
            "note": {"type": "string"}
        },
        "required": ["id", "customer", "items"]
    }"#;

    #[test]
    fn a_valid_document_has_no_issues() {
        ok(
            ORDER,
            r#"{"id": 1, "customer": "Acme", "items": [{"sku": "A", "price": 2.5}, {"sku": "B"}]}"#,
        );
    }

    #[test]
    fn missing_required_fields_are_reported_on_the_object() {
        assert_eq!(
            issues(ORDER, r#"{"id": 1, "items": [{"sku": "A"}]}"#),
            [("".to_string(), IssueKind::MissingRequired)]
        );
        assert_eq!(
            issues(
                ORDER,
                r#"{"id": 1, "customer": "x", "items": [{"price": 1}]}"#
            ),
            [("items[0]".to_string(), IssueKind::MissingRequired)]
        );
    }

    #[test]
    fn undeclared_fields_are_reported_on_the_field() {
        assert_eq!(
            issues(
                ORDER,
                r#"{"id": 1, "customer": "x", "items": [{"sku": "A", "colour": "red"}], "extra": 1}"#
            ),
            [
                ("items[0].colour".to_string(), IssueKind::UndeclaredField),
                ("extra".to_string(), IssueKind::UndeclaredField),
            ]
        );
    }

    #[test]
    fn kinds_are_strict_and_nothing_is_coerced() {
        let s = r#"{"type": "object", "properties": {"a": {"type": "integer"}}}"#;
        assert_eq!(
            issues(s, r#"{"a": "1"}"#),
            [("a".to_string(), IssueKind::TypeMismatch)]
        );
        assert_eq!(
            issues(s, r#"{"a": 1.0}"#),
            [("a".to_string(), IssueKind::TypeMismatch)]
        );
        assert_eq!(
            issues(s, r#"{"a": true}"#),
            [("a".to_string(), IssueKind::TypeMismatch)]
        );
        assert_eq!(
            issues(s, r#"{"a": null}"#),
            [("a".to_string(), IssueKind::TypeMismatch)]
        );
        assert_eq!(
            issues(s, r#"{"a": [1]}"#),
            [("a".to_string(), IssueKind::TypeMismatch)]
        );
        ok(s, r#"{"a": 1}"#);
    }

    #[test]
    fn numbers_accept_integers_and_floats_but_not_bool_or_string() {
        let s = r#"{"type": "array", "items": {"type": "number"}}"#;
        ok(s, "[1, 2.5, -3]");
        assert_eq!(issues(s, r#"[true, "1"]"#).len(), 2);
    }

    #[test]
    fn booleans_and_strings_are_exact() {
        assert_eq!(issues(r#"{"type": "boolean"}"#, "1").len(), 1);
        assert_eq!(issues(r#"{"type": "string"}"#, "1").len(), 1);
        ok(r#"{"type": "boolean"}"#, "false");
        ok(r#"{"type": "string"}"#, "\"\"");
    }

    #[test]
    fn the_root_can_be_a_scalar() {
        assert_eq!(
            issues(r#"{"type": "integer"}"#, r#""x""#),
            [("".to_string(), IssueKind::TypeMismatch)]
        );
    }

    #[test]
    fn ranges_are_inclusive_and_exact() {
        let s = r#"{"type": "integer", "minimum": 1, "maximum": 3}"#;
        ok(s, "1");
        ok(s, "3");
        assert_eq!(issues(s, "0"), [("".to_string(), IssueKind::Constraint)]);
        assert_eq!(issues(s, "4"), [("".to_string(), IssueKind::Constraint)]);
        // A fractional bound on an integer type.
        let f = r#"{"type": "integer", "minimum": 1.5}"#;
        assert_eq!(issues(f, "1").len(), 1);
        ok(f, "2");
        // Beyond float precision.
        let big = r#"{"type": "integer", "maximum": 9007199254740992}"#;
        ok(big, "9007199254740992");
        assert_eq!(issues(big, "9007199254740993").len(), 1);
    }

    #[test]
    fn string_lengths_count_code_points() {
        let s = r#"{"type": "string", "minLength": 2, "maxLength": 2}"#;
        ok(s, "\"\u{1F600}\u{1F600}\"");
        assert_eq!(issues(s, "\"\u{1F600}\"").len(), 1);
        assert_eq!(issues(s, "\"abc\"").len(), 1);
    }

    #[test]
    fn patterns_match_anywhere_unless_anchored() {
        let s = r#"{"type": "string", "pattern": "^[A-Z]{2}[0-9]+$"}"#;
        ok(s, "\"AB123\"");
        assert_eq!(issues(s, "\"xAB123\"").len(), 1);
        ok(r#"{"type": "string", "pattern": "b"}"#, "\"abc\"");
    }

    #[test]
    fn enumerations_list_the_allowed_values() {
        let s = r#"{"type": "string", "enum": ["red", "green"]}"#;
        ok(s, "\"red\"");
        let found = validate(&spec(s), &data("\"blue\""));
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("red, green"));
        assert_eq!(issues(s, "1"), [("".to_string(), IssueKind::TypeMismatch)]);
    }

    #[test]
    fn date_formats_are_constraints_on_strings() {
        let d = r#"{"type": "string", "format": "date"}"#;
        ok(d, "\"2026-10-23\"");
        ok(d, "\"2024-02-29\"");
        for bad in [
            "\"2026-02-29\"",
            "\"2026-1-2\"",
            "\"20261023\"",
            "\"2026-10-23T00:00:00Z\"",
            "\"\"",
            "1",
        ] {
            assert_eq!(issues(d, bad).len(), 1, "{bad}");
        }
        let dt = r#"{"type": "string", "format": "date-time"}"#;
        ok(dt, "\"2026-10-23T14:30:00Z\"");
        ok(dt, "\"2026-10-23t14:30:00.5+02:00\"");
        for bad in [
            "\"2026-10-23\"",
            "\"2026-10-23T14:30:00\"",
            "\"2026-10-23 14:30:00Z\"",
            "\"2026-13-23T14:30:00Z\"",
            "\"\u{e9}\"",
        ] {
            assert_eq!(issues(dt, bad).len(), 1, "{bad}");
        }
    }

    #[test]
    fn array_sizes_and_items_are_checked_with_paths() {
        let s = r#"{"type": "array", "minItems": 1, "maxItems": 2, "items": {"type": "integer"}}"#;
        assert_eq!(issues(s, "[]"), [("".to_string(), IssueKind::Constraint)]);
        assert_eq!(
            issues(s, "[1,2,3]"),
            [("".to_string(), IssueKind::Constraint)]
        );
        assert_eq!(
            issues(s, r#"[1,"x"]"#),
            [("[1]".to_string(), IssueKind::TypeMismatch)]
        );
    }

    #[test]
    fn maps_check_every_value_and_allow_any_key() {
        let s = r#"{"type": "object", "additionalProperties": {"type": "integer"}}"#;
        ok(s, r#"{"a": 1, "any key at all": 2}"#);
        ok(s, "{}");
        assert_eq!(
            issues(s, r#"{"a": 1, "b": "x"}"#),
            [("b".to_string(), IssueKind::TypeMismatch)]
        );
    }

    #[test]
    fn recursive_types_validate_to_the_depth_of_the_data() {
        let s = r##"{
            "$ref": "#/$defs/Folder",
            "$defs": {"Folder": {"type": "object",
                "properties": {"name": {"type": "string"}, "children": {"type": "array", "items": {"$ref": "#/$defs/Folder"}}},
                "required": ["name"]}}
        }"##;
        ok(
            s,
            r#"{"name": "a", "children": [{"name": "b", "children": [{"name": "c"}]}]}"#,
        );
        assert_eq!(
            issues(
                s,
                r#"{"name": "a", "children": [{"name": "b", "children": [{"name": 3}]}]}"#
            ),
            [(
                "children[0].children[0].name".to_string(),
                IssueKind::TypeMismatch
            )]
        );
    }

    #[test]
    fn error_values_are_not_reported_as_invalid_data() {
        use crate::value::{ErrorKind, ErrorValue};
        let s = spec(
            r#"{"type": "object", "properties": {"a": {"type": "integer"}}, "required": ["a"]}"#,
        );
        let mut root = data(r#"{"a": 1}"#);
        let error = ErrorValue {
            kind: ErrorKind::DivZero,
            message: "x".into(),
            origin: None,
        };
        root.set(&"a".parse().unwrap(), Value::Error(error))
            .unwrap();
        assert!(validate(&s, &root).is_empty());
    }

    #[test]
    fn validate_at_checks_a_result_against_a_node_type() {
        let s = spec(ORDER);
        let Kind::Object { fields } = &s.get(s.root()).kind else {
            panic!("root is an object")
        };
        let price_items = fields.iter().find(|f| f.name == "items").unwrap().ty;
        let good = data(r#"[{"sku": "A"}]"#);
        assert!(validate_at(&s, price_items, &good, &Path::root().key("items")).is_empty());
        let bad = data(r#"[{"sku": 1}]"#);
        let found = validate_at(&s, price_items, &bad, &Path::root().key("items"));
        assert_eq!(found[0].path.to_string(), "items[0].sku");
    }
}
