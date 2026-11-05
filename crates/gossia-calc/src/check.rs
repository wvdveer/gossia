//! Turning what a formula returned into what its node holds: checking it
//! against the schema (DESIGN.md, "Result checking") and writing it into the
//! tree.

use gossia_core::validate::validate_at;
use gossia_core::{ErrorKind, ErrorValue, Kind, Node, Path, Segment, TypeId, TypeSpec, Value};
use indexmap::IndexMap;

use crate::engine::Failure;
use crate::model::Loc;

/// What a formula's node holds after the formula has run.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Computed {
    /// The result, which matches the node's type.
    Value(Value),
    /// The formula returned `None` for an optional node: no value, so the node
    /// is left absent.
    Absent,
    /// The node holds this error value.
    Error(ErrorValue),
}

/// What a formula came to: what its node holds, the Python traceback of a
/// failure, and a result its node's type refused.
pub(crate) struct Finished {
    pub computed: Computed,
    pub traceback: Option<String>,
    pub rejected: Option<Value>,
}

/// Checks a formula's result. Conversion is strict: a result must already be
/// the right kind, and nothing is coerced (DESIGN.md, "Scalar types").
pub(crate) fn finish(spec: &TypeSpec, loc: &Loc, result: Result<Value, Failure>) -> Finished {
    let (computed, traceback, rejected) = check(spec, loc, result);
    Finished {
        computed,
        traceback,
        rejected,
    }
}

fn check(
    spec: &TypeSpec,
    loc: &Loc,
    result: Result<Value, Failure>,
) -> (Computed, Option<String>, Option<Value>) {
    let fail = |kind: ErrorKind, message: String| {
        Computed::Error(ErrorValue {
            kind,
            message,
            origin: Some(loc.path.clone()),
        })
    };
    let value = match result {
        Err(failure) => {
            let mut error = failure.error;
            error.origin.get_or_insert_with(|| loc.path.clone());
            return (Computed::Error(error), failure.traceback, None);
        }
        Ok(Value::Error(mut error)) => {
            // `=Orders.total` where the source has failed: the same error.
            error.origin.get_or_insert_with(|| loc.path.clone());
            return (Computed::Error(error), None, None);
        }
        Ok(value) => value,
    };
    let Some(here) = spec.resolve(&loc.path) else {
        return (
            fail(
                ErrorKind::Ref,
                format!("the schema defines no node at {loc}"),
            ),
            None,
            None,
        );
    };
    if matches!(value, Value::Null) {
        return if here.required {
            (
                fail(
                    ErrorKind::Type,
                    format!("the formula returned None, but {loc} is required"),
                ),
                None,
                None,
            )
        } else {
            (Computed::Absent, None, None)
        };
    }
    // A null in an optional field is that field left absent, as `None` at the
    // node itself is, and as a null is taken on import.
    let value = if has_null(&value) {
        without_optional_nulls(spec, here.ty, &value)
    } else {
        value
    };
    let issues = validate_at(spec, here.ty, &Node::new(value.clone()), &loc.path);
    match issues.first() {
        Some(issue) => (
            fail(
                ErrorKind::Type,
                format!("{loc} does not accept this result: {}", issue.message),
            ),
            None,
            Some(value),
        ),
        None => (Computed::Value(value), None, None),
    }
}

/// Whether a result holds a null anywhere (below its top).
fn has_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.iter().any(|i| has_null(i.value())),
        Value::Object(map) => map.values().any(|v| has_null(v.value())),
        _ => false,
    }
}

/// A result with the nulls in its optional fields taken out, following the
/// type. A null in a required field or as an item stays, for the check to
/// refuse.
fn without_optional_nulls(spec: &TypeSpec, ty: TypeId, value: &Value) -> Value {
    match (&spec.get(ty).kind, value) {
        (Kind::Object { fields }, Value::Object(map)) => Value::Object(
            map.iter()
                .filter_map(
                    |(name, node)| match fields.iter().find(|f| &f.name == name) {
                        Some(f) if !f.required && matches!(node.value(), Value::Null) => None,
                        Some(f) => Some((
                            name.clone(),
                            Node::new(without_optional_nulls(spec, f.ty, node.value())),
                        )),
                        None => Some((name.clone(), node.clone())),
                    },
                )
                .collect(),
        ),
        (Kind::Array { items, .. }, Value::Array(list)) => Value::Array(
            list.iter()
                .map(|i| Node::new(without_optional_nulls(spec, *items, i.value())))
                .collect(),
        ),
        (Kind::Map { value: of }, Value::Object(map)) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        Node::new(without_optional_nulls(spec, *of, v.value())),
                    )
                })
                .collect(),
        ),
        (_, other) => other.clone(),
    }
}

/// Writes a computed result into a tree. Objects and arrays on the way that do
/// not exist yet are created empty, so that a formula on an optional node
/// inside an absent optional parent can create both. An array item that does
/// not exist is not created: a formula on it gives `#REF!`.
pub(crate) fn store(
    spec: &TypeSpec,
    data: &mut Node,
    path: &Path,
    computed: &Computed,
) -> Result<(), String> {
    let value = match computed {
        Computed::Absent => {
            // Removing what is not there is not a failure.
            let _ = data.remove(path);
            return Ok(());
        }
        Computed::Value(v) => v.clone(),
        Computed::Error(e) => Value::Error(e.clone()),
    };
    // A formula puts its result in a node; it does not add items to an array.
    // The item has to be there, at the end of the path and on the way to it.
    for depth in 1..=path.len() {
        let prefix = Path::from(path.segments()[..depth].to_vec());
        if data.get(&prefix).is_none()
            && matches!(prefix.segments().last(), Some(Segment::Index(_)))
        {
            return Err(format!(
                "there is no item at {prefix}; a formula does not add items to an array"
            ));
        }
    }
    for depth in 1..path.len() {
        let prefix = Path::from(path.segments()[..depth].to_vec());
        if data.get(&prefix).is_some() {
            continue;
        }
        let empty = match spec.resolve(&prefix).map(|r| &spec.get(r.ty).kind) {
            Some(Kind::Object { .. } | Kind::Map { .. }) => Value::Object(IndexMap::new()),
            Some(Kind::Array { .. }) => Value::Array(Vec::new()),
            _ => return Err(format!("there is no node at {prefix} to put this under")),
        };
        data.set(&prefix, empty).map_err(|e| e.to_string())?;
    }
    data.set(path, value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use gossia_core::{ErrorKind, Path, Value, json, jsonschema};

    use super::{Computed, finish};
    use crate::model::Loc;

    const SCHEMA: &str = r#"{"type": "object", "properties": {
        "a": {"type": "string"},
        "b": {"type": "string"},
        "inner": {"type": "object", "properties": {"c": {"type": "integer"}}},
        "list": {"type": "array", "items": {"type": "string"}}
    }, "required": ["a"]}"#;

    fn checked(result: &str) -> Computed {
        let spec = jsonschema::lower_str(SCHEMA).unwrap();
        let value = json::parse(result).unwrap().node.value().clone();
        finish(&spec, &Loc::new("T", Path::root()), Ok(value)).computed
    }

    fn kind(computed: &Computed) -> Option<ErrorKind> {
        match computed {
            Computed::Error(e) => Some(e.kind),
            _ => None,
        }
    }

    /// A null in an optional field, at any depth, is the field left absent.
    #[test]
    fn a_null_in_an_optional_field_is_that_field_absent() {
        let computed = checked(r#"{"a": "x", "b": null, "inner": {"c": null}}"#);
        let Computed::Value(Value::Object(map)) = &computed else {
            panic!("{computed:?}");
        };
        assert!(!map.contains_key("b"));
        assert!(matches!(map["inner"].value(), Value::Object(inner) if inner.is_empty()));
    }

    /// A null in a required field, or as an item, is still refused.
    #[test]
    fn a_null_in_a_required_field_or_as_an_item_is_refused() {
        assert_eq!(kind(&checked(r#"{"a": null}"#)), Some(ErrorKind::Type));
        assert_eq!(
            kind(&checked(r#"{"a": "x", "list": ["y", null]}"#)),
            Some(ErrorKind::Type)
        );
    }
}
