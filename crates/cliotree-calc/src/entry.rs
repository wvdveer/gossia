//! What typing into a cell means (DESIGN.md, "Formulas", "Entering formulas").
//!
//! Text that starts with `=` is a formula and what follows is one Python
//! expression. Anything else is a literal, read as the node's type: a number
//! for a number node, `true` or `false` for a boolean, text as it is for a
//! string. Entering an empty text clears the node. To enter text that starts
//! with `=` into a text node, start it with an apostrophe: `'=total` is the
//! text `=total`. A literal that does not fit the type or its constraints is
//! refused here, so the editors never put invalid data in.

use std::fmt;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use cliotree_core::validate::validate_at;
use cliotree_core::{DateTimeValue, Kind, Node, Path, Segment, TypeSpec, Value};

use crate::calc::{Calc, EditError};

/// What the text means.
#[derive(Debug, Clone, PartialEq)]
pub enum Entered {
    /// A formula, without the `=`.
    Formula(String),
    /// A literal of the node's type.
    Literal(Value),
    /// Nothing: the node is cleared.
    Clear,
}

/// Why the text was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryError {
    /// The schema defines no node at this place.
    NotInSchema,
    /// The text is not a value of the node's type, or breaks a constraint.
    Invalid(String),
    /// The calculation layer refused the edit.
    Refused(EditError),
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryError::NotInSchema => f.write_str("the schema defines no node here"),
            EntryError::Invalid(message) => f.write_str(message),
            EntryError::Refused(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for EntryError {}

impl From<EditError> for EntryError {
    fn from(e: EditError) -> Self {
        EntryError::Refused(e)
    }
}

/// Reads what was typed for the node at `path`.
pub fn classify(spec: &TypeSpec, path: &Path, text: &str) -> Result<Entered, EntryError> {
    let here = spec.resolve(path).ok_or(EntryError::NotInSchema)?;
    let kind = &spec.get(here.ty).kind;
    let is_text = matches!(kind, Kind::String(_) | Kind::Enum { .. });

    if text.trim().is_empty() {
        return Ok(Entered::Clear);
    }
    if let Some(rest) = text.strip_prefix('=') {
        let formula = rest.trim();
        return if formula.is_empty() {
            Ok(Entered::Clear)
        } else {
            Ok(Entered::Formula(formula.to_owned()))
        };
    }
    // A single leading apostrophe is removed on a text node, so that text that
    // starts with `=` can be entered. Text that really starts with an
    // apostrophe is typed with two.
    let literal = if is_text {
        text.strip_prefix('\'').unwrap_or(text)
    } else {
        text
    };
    let value = parse_literal(kind, literal)?;
    let issues = validate_at(spec, here.ty, &Node::new(value.clone()), path);
    if let Some(issue) = issues.first() {
        return Err(EntryError::Invalid(issue.message.clone()));
    }
    Ok(Entered::Literal(value))
}

fn parse_literal(kind: &Kind, text: &str) -> Result<Value, EntryError> {
    let invalid = |what: &str| EntryError::Invalid(format!("{what} is expected here"));
    match kind {
        Kind::String(_) | Kind::Enum { .. } => Ok(Value::Str(text.to_owned())),
        Kind::Integer(_) => text
            .trim()
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| invalid("a whole number")),
        Kind::Number(_) => {
            let t = text.trim();
            if let Ok(n) = t.parse::<i64>() {
                return Ok(Value::Int(n));
            }
            match t.parse::<f64>() {
                Ok(x) if x.is_finite() => Ok(Value::Float(x)),
                _ => Err(invalid("a number")),
            }
        }
        Kind::Bool => match text.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(invalid("true or false")),
        },
        Kind::Date => NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
            .map(Value::Date)
            .map_err(|_| invalid("a date such as 2026-10-23")),
        Kind::DateTime => {
            let t = text.trim();
            if let Ok(z) = DateTime::parse_from_rfc3339(t) {
                return Ok(Value::DateTime(DateTimeValue::Zoned(z)));
            }
            NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f")
                .map(|n| Value::DateTime(DateTimeValue::Naive(n)))
                .map_err(|_| invalid("a date and time such as 2026-10-23T14:30:00"))
        }
        Kind::Array { .. } | Kind::Object { .. } | Kind::Map { .. } => Err(EntryError::Invalid(
            "this holds other values: edit them, or enter a formula that gives the whole value"
                .to_owned(),
        )),
    }
}

/// Enters `text` into the node at `path` of the tab: a formula, a literal, or
/// a clearing, as [`classify`] reads it. Entering is one entry in the history;
/// `typing` joins a literal to the entry before it when that was typing in the
/// same cell.
pub fn enter(
    calc: &mut Calc,
    sheet: &str,
    path: &Path,
    text: &str,
    typing: bool,
) -> Result<(), EntryError> {
    let spec = calc
        .spec(sheet)
        .ok_or_else(|| EntryError::Refused(EditError::NoSheet(sheet.to_owned())))?;
    match classify(spec, path, text)? {
        Entered::Formula(formula) => calc.set_formula(sheet, path, &formula)?,
        Entered::Literal(value) => {
            if typing {
                calc.set_value_merging(sheet, path, value)?;
            } else {
                calc.set_value(sheet, path, value)?;
            }
        }
        Entered::Clear => clear(calc, sheet, path)?,
    }
    Ok(())
}

/// Removes the formula and the value of a node (DESIGN.md, "Entering
/// formulas"): an optional node becomes absent, and so does a required one,
/// which then shows as an empty flagged cell. An array item cannot be left
/// empty, because that would move the others; a text item becomes empty text.
fn clear(calc: &mut Calc, sheet: &str, path: &Path) -> Result<(), EditError> {
    let in_array = matches!(path.segments().last(), Some(Segment::Index(_)));
    calc.begin_entry(&format!("Clear {}", crate::Loc::new(sheet, path.clone())));
    let result = (|| {
        calc.remove_formula(sheet, path)?;
        if in_array {
            let is_text = calc.spec(sheet).is_some_and(|spec| {
                spec.resolve(path).is_some_and(|r| {
                    matches!(spec.get(r.ty).kind, Kind::String(_) | Kind::Enum { .. })
                })
            });
            if is_text {
                calc.set_value(sheet, path, Value::Str(String::new()))?;
            } else {
                return Err(EditError::Path(
                    "an item cannot be left empty; remove it instead".to_owned(),
                ));
            }
        } else if calc.data(sheet).is_some_and(|d| d.get(path).is_some()) {
            calc.remove(sheet, path)?;
        }
        Ok(())
    })();
    calc.end_entry();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use cliotree_core::jsonschema;

    const SCHEMA: &str = r#"{
      "type": "object",
      "properties": {
        "n": {"type": "integer", "minimum": 0, "maximum": 100},
        "x": {"type": "number"},
        "flag": {"type": "boolean"},
        "name": {"type": "string", "maxLength": 5},
        "kind": {"type": "string", "enum": ["a", "b"]},
        "note": {"type": "string"},
        "items": {"type": "array", "items": {"type": "string"}},
        "nums": {"type": "array", "items": {"type": "integer"}}
      },
      "required": ["n", "x", "flag", "name", "items", "nums"]
    }"#;

    fn spec() -> TypeSpec {
        jsonschema::lower_str(SCHEMA).unwrap()
    }

    fn p(text: &str) -> Path {
        text.parse().unwrap()
    }

    fn classify_at(path: &str, text: &str) -> Result<Entered, EntryError> {
        classify(&spec(), &p(path), text)
    }

    #[test]
    fn text_starting_with_equals_is_a_formula() {
        assert_eq!(
            classify_at("n", "=SUM(Orders.items.price)"),
            Ok(Entered::Formula("SUM(Orders.items.price)".into()))
        );
        assert_eq!(
            classify_at("n", "=  S.a + 1  "),
            Ok(Entered::Formula("S.a + 1".into()))
        );
        assert_eq!(classify_at("n", "="), Ok(Entered::Clear));
    }

    /// Entering formulas: the apostrophe rule applies to text nodes only.
    #[test]
    fn an_apostrophe_enters_text_that_starts_with_equals_or_an_apostrophe() {
        assert_eq!(
            classify_at("note", "'=total"),
            Ok(Entered::Literal(Value::Str("=total".into())))
        );
        assert_eq!(
            classify_at("note", "''quoted"),
            Ok(Entered::Literal(Value::Str("'quoted".into())))
        );
        assert_eq!(
            classify_at("note", "plain text"),
            Ok(Entered::Literal(Value::Str("plain text".into())))
        );
        // A number node has no such rule.
        assert!(classify_at("n", "'5").is_err());
    }

    #[test]
    fn literals_are_read_as_the_type_of_the_node() {
        assert_eq!(
            classify_at("n", " 42 "),
            Ok(Entered::Literal(Value::Int(42)))
        );
        assert_eq!(classify_at("x", "3"), Ok(Entered::Literal(Value::Int(3))));
        assert_eq!(
            classify_at("x", "2.5"),
            Ok(Entered::Literal(Value::Float(2.5)))
        );
        assert_eq!(
            classify_at("x", "1e3"),
            Ok(Entered::Literal(Value::Float(1000.0)))
        );
        assert_eq!(
            classify_at("flag", "True"),
            Ok(Entered::Literal(Value::Bool(true)))
        );
        assert_eq!(
            classify_at("flag", "false"),
            Ok(Entered::Literal(Value::Bool(false)))
        );
        assert_eq!(
            classify_at("kind", "b"),
            Ok(Entered::Literal(Value::Str("b".into())))
        );
        // Text keeps its spaces.
        assert_eq!(
            classify_at("note", "  padded  "),
            Ok(Entered::Literal(Value::Str("  padded  ".into())))
        );
    }

    /// Invalid data: the editors prevent entering an invalid literal.
    #[test]
    fn a_literal_that_does_not_fit_is_refused_with_a_reason() {
        for (path, text, fragment) in [
            ("n", "1.5", "whole number"),
            ("n", "abc", "whole number"),
            ("n", "101", "maximum"),
            ("n", "-1", "minimum"),
            ("x", "nan", "number"),
            ("x", "inf", "number"),
            ("flag", "yes", "true or false"),
            ("name", "toolong", "characters"),
            ("kind", "z", "one of"),
        ] {
            let e = classify_at(path, text).unwrap_err();
            let message = e.to_string();
            assert!(
                message.to_lowercase().contains(fragment),
                "{path} {text:?}: {message}"
            );
        }
    }

    #[test]
    fn containers_do_not_take_a_literal() {
        let e = classify_at("items", "x").unwrap_err();
        assert!(e.to_string().contains("formula"), "{e}");
    }

    #[test]
    fn empty_text_clears() {
        assert_eq!(classify_at("n", ""), Ok(Entered::Clear));
        assert_eq!(classify_at("note", "   "), Ok(Entered::Clear));
    }

    #[test]
    fn a_node_the_schema_does_not_define_is_refused() {
        assert_eq!(classify_at("nothing", "1"), Err(EntryError::NotInSchema));
    }

    fn book() -> Calc {
        use cliotree_core::json;
        let mut calc = Calc::new(std::sync::Arc::new(crate::python::PythonSyntax));
        calc.add_sheet(
            "S",
            spec(),
            json::parse(
                r#"{"n": 1, "x": 1.5, "flag": true, "name": "ab", "note": "hi",
                    "items": ["x", "y"], "nums": [1, 2, 3]}"#,
            )
            .unwrap()
            .node,
        )
        .unwrap();
        calc
    }

    #[test]
    fn entering_sets_a_literal_a_formula_or_clears() {
        let mut calc = book();
        enter(&mut calc, "S", &p("n"), "7", false).unwrap();
        assert_eq!(
            calc.data("S").unwrap().get(&p("n")).unwrap().value(),
            &Value::Int(7)
        );
        enter(&mut calc, "S", &p("x"), "=S.n * 2", false).unwrap();
        assert_eq!(calc.formula_text("S", &p("x")).as_deref(), Some("S.n * 2"));
        // A literal over a formula replaces it.
        enter(&mut calc, "S", &p("x"), "2.5", false).unwrap();
        assert_eq!(calc.formula_text("S", &p("x")), None);
        // Clearing an optional node leaves it absent.
        enter(&mut calc, "S", &p("note"), "", false).unwrap();
        assert!(calc.data("S").unwrap().get(&p("note")).is_none());
        // Clearing a required one too: it is then a missing required value.
        enter(&mut calc, "S", &p("name"), "", false).unwrap();
        assert!(calc.data("S").unwrap().get(&p("name")).is_none());
        // Each is one thing to undo, and clearing a formula and its value is one.
        enter(&mut calc, "S", &p("n"), "=S.x", false).unwrap();
        enter(&mut calc, "S", &p("n"), "", false).unwrap();
        calc.undo().unwrap();
        assert_eq!(calc.formula_text("S", &p("n")).as_deref(), Some("S.x"));
    }

    #[test]
    fn a_text_item_is_emptied_and_another_item_cannot_be() {
        let mut calc = book();
        enter(&mut calc, "S", &p("items[0]"), "", false).unwrap();
        assert_eq!(
            calc.data("S").unwrap().get(&p("items[0]")).unwrap().value(),
            &Value::Str(String::new())
        );
        let e = enter(&mut calc, "S", &p("nums[0]"), "", false).unwrap_err();
        assert!(e.to_string().contains("cannot be left empty"), "{e}");
        assert_eq!(calc.history_len().0, 1, "the refused clear left no entry");
    }

    #[test]
    fn typing_in_one_cell_is_one_entry_to_undo() {
        let mut calc = book();
        for text in ["2", "23", "234".replace("234", "50").as_str()] {
            enter(&mut calc, "S", &p("n"), text, true).unwrap();
        }
        assert_eq!(calc.history_len().0, 1);
        calc.undo().unwrap();
        assert_eq!(
            calc.data("S").unwrap().get(&p("n")).unwrap().value(),
            &Value::Int(1)
        );
    }

    #[test]
    fn a_refused_entry_changes_nothing() {
        let mut calc = book();
        assert!(enter(&mut calc, "S", &p("n"), "500", false).is_err());
        assert_eq!(calc.history_len(), (0, 0));
        assert!(enter(&mut calc, "Nope", &p("n"), "5", false).is_err());
    }
}
