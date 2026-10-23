//! Turning what a formula returned into what its node holds: checking it
//! against the schema (DESIGN.md, "Result checking") and writing it into the
//! tree.

use cliotree_core::validate::validate_at;
use cliotree_core::{ErrorKind, ErrorValue, Kind, Node, Path, Segment, TypeSpec, Value};
use indexmap::IndexMap;

use crate::engine::Failure;
use crate::model::Loc;

/// What a formula's node holds after the formula has run.
#[derive(Debug, Clone, PartialEq)]
pub enum Computed {
    /// The result, which matches the node's type.
    Value(Value),
    /// The formula returned `None` for an optional node: no value, so the node
    /// is left absent.
    Absent,
    /// The node holds this error value.
    Error(ErrorValue),
}

/// Checks a formula's result. Conversion is strict: a result must already be
/// the right kind, and nothing is coerced (DESIGN.md, "Scalar types").
pub(crate) fn finish(
    spec: &TypeSpec,
    loc: &Loc,
    result: Result<Value, Failure>,
) -> (Computed, Option<String>) {
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
            return (Computed::Error(error), failure.traceback);
        }
        Ok(Value::Error(mut error)) => {
            // `=Orders.total` where the source has failed: the same error.
            error.origin.get_or_insert_with(|| loc.path.clone());
            return (Computed::Error(error), None);
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
            )
        } else {
            (Computed::Absent, None)
        };
    }
    let issues = validate_at(spec, here.ty, &Node::new(value.clone()), &loc.path);
    match issues.first() {
        Some(issue) => (
            fail(
                ErrorKind::Type,
                format!("{loc} does not accept this result: {}", issue.message),
            ),
            None,
        ),
        None => (Computed::Value(value), None),
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
