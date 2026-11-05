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
    /// The result did not fit the node's type, but parts of it did: the node
    /// holds `value`, in which each part that did not fit is `#TYPE!`, and
    /// the formula ends in `error`. `misfits` are the nodes that did not fit
    /// (a record holding a field it does not define is one itself).
    Partial {
        value: Value,
        error: ErrorValue,
        misfits: Vec<Path>,
    },
}

impl Computed {
    /// The error the formula ended in.
    pub fn error(&self) -> Option<&ErrorValue> {
        match self {
            Computed::Error(e) | Computed::Partial { error: e, .. } => Some(e),
            _ => None,
        }
    }
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
    // What fits is kept, and each part that does not is `#TYPE!` in its
    // place. A null in an optional field, at any depth, is that field left
    // absent, as `None` at the node itself is, and as a null is taken on import.
    let mut misfits = Vec::new();
    let mut fitted = fit(spec, here.ty, &value, &loc.path, &mut misfits);
    if misfits.is_empty() {
        // A rule on a record or array as a whole (a number of items, say),
        // which following it in does not check.
        let mut whole = Node::new(fitted);
        if let Some(issue) = validate_at(spec, here.ty, &whole, &loc.path)
            .into_iter()
            .next()
        {
            let inside = Path::from(
                issue
                    .path
                    .segments()
                    .get(loc.path.len()..)
                    .unwrap_or_default()
                    .to_vec(),
            );
            let part = misfit(&issue.path, &issue.message);
            if inside.is_root() {
                *whole.value_mut() = part;
            } else {
                let _ = whole.set(&inside, part);
            }
            misfits.push((issue.path, issue.message));
        }
        fitted = std::mem::replace(whole.value_mut(), Value::Null);
    }
    let Some((at, message)) = misfits.first() else {
        return (Computed::Value(fitted), None, None);
    };
    let message = if *at == loc.path {
        format!("{loc} does not accept this result: {message}")
    } else {
        format!("{loc} does not accept this result: at {at}, {message}")
    };
    let error = ErrorValue {
        kind: ErrorKind::Type,
        message,
        origin: Some(loc.path.clone()),
    };
    let computed = if matches!(fitted, Value::Error(_)) {
        Computed::Error(error)
    } else {
        Computed::Partial {
            value: fitted,
            error,
            misfits: misfits.into_iter().map(|(p, _)| p).collect(),
        }
    };
    (computed, None, Some(value))
}

/// The part of `value` at `path` that fits `ty`: records, arrays and maps are
/// followed into, and anything else that does not fit is `#TYPE!` in its
/// place. Each place that did not fit is added to `misfits` with the reason.
/// A record's field it does not define is left out, and the record is the
/// place; a required field that is missing is `#TYPE!`.
fn fit(
    spec: &TypeSpec,
    ty: TypeId,
    value: &Value,
    path: &Path,
    misfits: &mut Vec<(Path, String)>,
) -> Value {
    match (&spec.get(ty).kind, value) {
        (Kind::Object { fields }, Value::Object(map)) => {
            let mut out = IndexMap::new();
            for (name, node) in map {
                match fields.iter().find(|f| &f.name == name) {
                    Some(f) if !f.required && matches!(node.value(), Value::Null) => {}
                    Some(f) => {
                        let here = path.key(name.clone());
                        let v = fit(spec, f.ty, node.value(), &here, misfits);
                        out.insert(name.clone(), Node::new(v));
                    }
                    None => {
                        misfits.push((path.clone(), format!("{name} is not one of its fields")))
                    }
                }
            }
            for f in fields
                .iter()
                .filter(|f| f.required && !map.contains_key(&f.name))
            {
                let here = path.key(f.name.clone());
                let message = "it is required, and the result has none".to_owned();
                out.insert(f.name.clone(), Node::new(misfit(&here, &message)));
                misfits.push((here, message));
            }
            Value::Object(out)
        }
        (Kind::Array { items, .. }, Value::Array(list)) => Value::Array(
            list.iter()
                .enumerate()
                .map(|(i, item)| {
                    Node::new(fit(spec, *items, item.value(), &path.index(i), misfits))
                })
                .collect(),
        ),
        (Kind::Map { value: of }, Value::Object(map)) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let here = path.key(k.clone());
                    (
                        k.clone(),
                        Node::new(fit(spec, *of, v.value(), &here, misfits)),
                    )
                })
                .collect(),
        ),
        // Any value: kept as it is, with its null fields left out as anywhere
        // else (what is inside an `any` is `any`).
        (Kind::Any, Value::Object(map)) => Value::Object(
            map.iter()
                .filter(|(_, v)| !matches!(v.value(), Value::Null))
                .map(|(k, v)| {
                    let here = path.key(k.clone());
                    (
                        k.clone(),
                        Node::new(fit(spec, ty, v.value(), &here, misfits)),
                    )
                })
                .collect(),
        ),
        (Kind::Any, Value::Array(list)) => Value::Array(
            list.iter()
                .enumerate()
                .map(|(i, item)| Node::new(fit(spec, ty, item.value(), &path.index(i), misfits)))
                .collect(),
        ),
        // An error read from elsewhere stays what it is.
        (_, Value::Error(error)) => {
            misfits.push((
                path.clone(),
                format!("{} {}", error.kind.code(), error.message),
            ));
            value.clone()
        }
        // A scalar, or a record or array where the result has something
        // else: checked as it is.
        _ => match validate_at(spec, ty, &Node::new(value.clone()), path).first() {
            Some(issue) => {
                misfits.push((path.clone(), issue.message.clone()));
                misfit(path, &issue.message)
            }
            None => value.clone(),
        },
    }
}

/// The `#TYPE!` that stands for a part of a result that did not fit.
fn misfit(path: &Path, message: &str) -> Value {
    Value::Error(ErrorValue {
        kind: ErrorKind::Type,
        message: format!("this does not fit: {message}"),
        origin: Some(path.clone()),
    })
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
        Computed::Partial { value, .. } => value.clone(),
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
        "list": {"type": "array", "items": {"type": "string"}},
        "free": {}
    }, "required": ["a"]}"#;

    fn checked(result: &str) -> Computed {
        let spec = jsonschema::lower_str(SCHEMA).unwrap();
        let value = json::parse(result).unwrap().node.value().clone();
        finish(&spec, &Loc::new("T", Path::root()), Ok(value)).computed
    }

    fn kind(computed: &Computed) -> Option<ErrorKind> {
        computed.error().map(|e| e.kind)
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

    /// A field with no `type` takes any value, as it is, with its null
    /// fields left out.
    #[test]
    fn a_field_of_any_type_takes_any_value() {
        for free in ["73568", r#""Z26 7XT""#, "[1, \"x\", true]"] {
            let computed = checked(&format!(r#"{{"a": "x", "free": {free}}}"#));
            assert!(
                matches!(computed, Computed::Value(_)),
                "{free}: {computed:?}"
            );
        }
        let Computed::Value(Value::Object(map)) =
            checked(r#"{"a": "x", "free": {"k": null, "v": [null]}}"#)
        else {
            panic!();
        };
        let Value::Object(free) = map["free"].value() else {
            panic!()
        };
        assert_eq!(free.keys().collect::<Vec<_>>(), ["v"]);
    }

    /// A result of which only parts fit: those parts are kept, each part
    /// that does not is `#TYPE!` in its place, and the formula ends in
    /// `#TYPE!` naming the first.
    #[test]
    fn what_fits_of_a_result_is_kept_and_the_rest_is_type_in_its_place() {
        let computed = checked(r#"{"a": "x", "b": 7, "inner": {"c": "nine"}, "list": ["y", 2]}"#);
        let Computed::Partial {
            value,
            error,
            misfits,
        } = &computed
        else {
            panic!("{computed:?}");
        };
        let at = |path: &str| {
            gossia_core::Node::new(value.clone())
                .get(&path.parse().unwrap())
                .map(|n| n.value().clone())
        };
        assert_eq!(at("a"), Some(Value::Str("x".into())));
        assert_eq!(at("list[0]"), Some(Value::Str("y".into())));
        for path in ["b", "inner.c", "list[1]"] {
            assert!(
                matches!(at(path), Some(Value::Error(e)) if e.kind == ErrorKind::Type),
                "{path}"
            );
        }
        let misfits: Vec<String> = misfits.iter().map(ToString::to_string).collect();
        assert_eq!(misfits, ["b", "inner.c", "list[1]"]);
        assert_eq!(error.kind, ErrorKind::Type);
        assert!(error.message.contains("at b,"), "{}", error.message);

        // A field the record does not define is left out, and the record is
        // the part that did not fit; a missing required field is `#TYPE!`.
        let Computed::Partial { value, misfits, .. } = checked(r#"{"b": "y", "extra": 1}"#) else {
            panic!();
        };
        let Value::Object(map) = &value else { panic!() };
        assert_eq!(map.keys().collect::<Vec<_>>(), ["b", "a"]);
        assert!(matches!(map["a"].value(), Value::Error(_)));
        let misfits: Vec<String> = misfits.iter().map(ToString::to_string).collect();
        assert_eq!(misfits, ["", "a"]);
    }
}
