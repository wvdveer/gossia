//! Reading and writing JSON data.
//!
//! JSON text becomes a [`Node`] tree and back (see DESIGN.md, "Core model":
//! "JSON details not preserved"). Field order is kept. Number spelling is not:
//! `1e3` reads as the float `1000`, and `1.0` stays a float. An integer beyond
//! the signed 64-bit range reads as a float, with a warning.

use std::fmt;

use crate::path::Path;
use crate::value::{DateTimeValue, Node, Value};

/// Something noticed while reading that did not stop the read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub path: Path,
    pub message: String,
}

/// The result of reading JSON text.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub node: Node,
    pub warnings: Vec<Warning>,
}

/// The text is not JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// The tree holds something JSON cannot express.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteError {
    pub path: Path,
    pub message: String,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot write `{}`: {}", self.path, self.message)
    }
}

impl std::error::Error for WriteError {}

/// Reads JSON text. Nesting deeper than 128 levels is refused.
pub fn parse(text: &str) -> Result<Parsed, ParseError> {
    let json: serde_json::Value = serde_json::from_str(text).map_err(|e| ParseError {
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    })?;
    let mut warnings = Vec::new();
    let node = convert(&json, &Path::root(), &mut warnings);
    Ok(Parsed { node, warnings })
}

fn convert(json: &serde_json::Value, path: &Path, warnings: &mut Vec<Warning>) -> Node {
    use serde_json::Value as J;
    Node::new(match json {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                if n.is_u64() {
                    warnings.push(Warning {
                        path: path.clone(),
                        message: "integer beyond the signed 64-bit range was read as a floating-point number".into(),
                    });
                }
                Value::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        J::String(s) => Value::Str(s.clone()),
        J::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| convert(item, &path.index(i), warnings))
                .collect(),
        ),
        J::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), convert(v, &path.key(k.clone()), warnings)))
                .collect(),
        ),
    })
}

/// Writes a tree as JSON text, with two-space indentation if `pretty`.
///
/// A node that holds an error value is written **absent**, because JSON cannot
/// hold one (see DESIGN.md, "Errors in saved files"). Dates are written as ISO
/// 8601 text. A float that is NaN or infinite cannot be written.
pub fn to_string(node: &Node, pretty: bool) -> Result<String, WriteError> {
    let json = to_json(node, &Path::root())?.ok_or_else(|| WriteError {
        path: Path::root(),
        message: "the root holds an error value".into(),
    })?;
    Ok(if pretty {
        serde_json::to_string_pretty(&json)
    } else {
        serde_json::to_string(&json)
    }
    .expect("a JSON value always serializes"))
}

fn to_json(node: &Node, path: &Path) -> Result<Option<serde_json::Value>, WriteError> {
    use serde_json::Value as J;
    Ok(Some(match node.value() {
        Value::Error(_) => return Ok(None),
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::from(*i),
        Value::Float(x) => {
            J::Number(serde_json::Number::from_f64(*x).ok_or_else(|| WriteError {
                path: path.clone(),
                message: "NaN and infinity are not JSON numbers".into(),
            })?)
        }
        Value::Str(s) => J::String(s.clone()),
        Value::Date(d) => J::String(d.format("%Y-%m-%d").to_string()),
        Value::DateTime(DateTimeValue::Naive(dt)) => {
            J::String(dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
        }
        Value::DateTime(DateTimeValue::Zoned(dt)) => J::String(dt.to_rfc3339()),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                if let Some(json) = to_json(item, &path.index(i))? {
                    out.push(json);
                }
            }
            J::Array(out)
        }
        Value::Object(fields) => {
            let mut out = serde_json::Map::new();
            for (k, v) in fields {
                if let Some(json) = to_json(v, &path.key(k.clone()))? {
                    out.insert(k.clone(), json);
                }
            }
            J::Object(out)
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{ErrorKind, ErrorValue};
    use proptest::prelude::*;

    fn read(text: &str) -> Node {
        parse(text).unwrap().node
    }

    #[test]
    fn reads_every_json_kind() {
        let node = read(
            r#"{"a": null, "b": true, "c": 3, "d": 1.5, "e": "x", "f": [1, 2], "g": {"h": 1}}"#,
        );
        let fields = node.value().as_object().unwrap();
        assert_eq!(fields["a"].value(), &Value::Null);
        assert_eq!(fields["b"].value(), &Value::Bool(true));
        assert_eq!(fields["c"].value(), &Value::Int(3));
        assert_eq!(fields["d"].value(), &Value::Float(1.5));
        assert_eq!(fields["e"].value(), &Value::Str("x".into()));
        assert_eq!(fields["f"].value().as_array().unwrap().len(), 2);
        assert_eq!(
            fields["g"].value().as_object().unwrap()["h"].value(),
            &Value::Int(1)
        );
    }

    #[test]
    fn keeps_field_order() {
        let node = read(r#"{"z": 1, "a": 2, "m": 3}"#);
        let keys: Vec<_> = node.value().as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["z", "a", "m"]);
        assert_eq!(to_string(&node, false).unwrap(), r#"{"z":1,"a":2,"m":3}"#);
    }

    #[test]
    fn number_spelling_is_not_preserved_but_kind_is() {
        assert_eq!(read("1e3").value(), &Value::Float(1000.0));
        assert_eq!(read("1.0").value(), &Value::Float(1.0));
        assert_eq!(read("10").value(), &Value::Int(10));
        assert_eq!(to_string(&read("1.0"), false).unwrap(), "1.0");
        assert_eq!(to_string(&read("1e3"), false).unwrap(), "1000.0");
    }

    #[test]
    fn integers_beyond_i64_become_floats_with_a_warning() {
        let parsed = parse(r#"{"big": 18446744073709551615, "ok": 9223372036854775807}"#).unwrap();
        let fields = parsed.node.value().as_object().unwrap();
        assert!(matches!(fields["big"].value(), Value::Float(_)));
        assert_eq!(fields["ok"].value(), &Value::Int(i64::MAX));
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.warnings[0].path, Path::root().key("big"));
    }

    #[test]
    fn syntax_errors_say_where() {
        let e = parse("{\n  \"a\": }").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(!e.message.is_empty());
        assert!(parse("").is_err());
        assert!(parse("[1,]").is_err());
    }

    #[test]
    fn very_deep_nesting_is_refused_not_a_stack_overflow() {
        let text = "[".repeat(10_000) + &"]".repeat(10_000);
        assert!(parse(&text).is_err());
    }

    #[test]
    fn errored_nodes_are_written_absent() {
        let error = ErrorValue {
            kind: ErrorKind::DivZero,
            message: "x".into(),
            origin: None,
        };
        let node = Node::new(Value::Object(
            [
                ("a".to_string(), Node::new(Value::Int(1))),
                ("b".to_string(), Node::new(Value::Error(error.clone()))),
                (
                    "c".to_string(),
                    Node::new(Value::Array(vec![
                        Node::new(Value::Int(2)),
                        Node::new(Value::Error(error.clone())),
                    ])),
                ),
            ]
            .into_iter()
            .collect(),
        ));
        assert_eq!(to_string(&node, false).unwrap(), r#"{"a":1,"c":[2]}"#);
        assert!(to_string(&Node::new(Value::Error(error)), false).is_err());
    }

    #[test]
    fn non_finite_floats_cannot_be_written() {
        let node = Node::new(Value::Array(vec![Node::new(Value::Float(f64::NAN))]));
        let e = to_string(&node, false).unwrap_err();
        assert_eq!(e.path, Path::root().index(0));
    }

    #[test]
    fn dates_are_written_as_iso_text() {
        use chrono::{FixedOffset, NaiveDate, TimeZone};
        let d = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        assert_eq!(
            to_string(&Node::new(Value::Date(d)), false).unwrap(),
            "\"2026-10-23\""
        );
        let naive = d.and_hms_opt(14, 30, 5).unwrap();
        assert_eq!(
            to_string(
                &Node::new(Value::DateTime(DateTimeValue::Naive(naive))),
                false
            )
            .unwrap(),
            "\"2026-10-23T14:30:05\""
        );
        let zoned = FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .from_local_datetime(&naive)
            .unwrap();
        assert_eq!(
            to_string(
                &Node::new(Value::DateTime(DateTimeValue::Zoned(zoned))),
                false
            )
            .unwrap(),
            "\"2026-10-23T14:30:05+02:00\""
        );
    }

    #[test]
    fn pretty_output_is_indented() {
        let text = to_string(&read(r#"{"a":[1]}"#), true).unwrap();
        assert_eq!(text, "{\n  \"a\": [\n    1\n  ]\n}");
    }

    fn value() -> impl Strategy<Value = Node> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(Value::Int),
            any::<f64>()
                .prop_filter("finite", |x| x.is_finite())
                .prop_map(Value::Float),
            ".{0,8}".prop_map(Value::Str),
        ];
        leaf.prop_map(Node::new).prop_recursive(4, 32, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4)
                    .prop_map(|items| Node::new(Value::Array(items))),
                prop::collection::vec((".{0,5}", inner), 0..4)
                    .prop_map(|fields| Node::new(Value::Object(fields.into_iter().collect()))),
            ]
        })
    }

    proptest! {
        #[test]
        fn data_round_trips_through_json(node in value()) {
            let text = to_string(&node, false).unwrap();
            let back = parse(&text).unwrap();
            prop_assert_eq!(back.node, node);
            prop_assert!(back.warnings.is_empty());
        }

        #[test]
        fn saving_twice_gives_identical_text(node in value()) {
            let first = to_string(&node, true).unwrap();
            let second = to_string(&parse(&first).unwrap().node, true).unwrap();
            prop_assert_eq!(first, second);
        }
    }
}
