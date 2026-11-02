//! What the expressions on a form control make of it (DESIGN.md, "Forms",
//! "Expressions on controls"): whether it is shown, whether it can be used,
//! the colours of its text and background, and the list of choices of a
//! dropdown or typeahead ("Choices"). The expressions run on the evaluation
//! thread; this reads their latest results.

use cliotree_calc::{Calc, ExprKind, ExprState};
use cliotree_core::Value;
use egui::Color32;

/// How a control is to be drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gate {
    pub hidden: bool,
    pub disabled: bool,
    pub text: Option<Color32>,
    pub fill: Option<Color32>,
    /// Why it looks as it does, when that is not plain: an error, or a
    /// workbook that is not trusted. Shown on hover.
    pub why: Vec<String>,
    /// An expression is waiting to run again.
    pub pending: bool,
}

impl Gate {
    /// With the fixed colours of a design (`#rrggbb`), which run nothing, so
    /// they are shown in an untrusted workbook and while designing too.
    pub fn with_fixed(mut self, text: Option<&str>, fill: Option<&str>) -> Gate {
        if let Some(colour) = text.and_then(fixed) {
            self.text = Some(colour);
        }
        if let Some(colour) = fill.and_then(fixed) {
            self.fill = Some(colour);
        }
        self
    }
}

/// The colour of a fixed colour property, or None for an expression.
pub fn fixed(text: &str) -> Option<Color32> {
    cliotree_core::form::fixed_colour(text).map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

/// What the expressions of the control at `at` of a form come to; in the
/// design of the items of repeaters, for the item `rows` names of each.
pub fn gate(calc: &Calc, tab: &str, form: &str, at: &[usize], rows: &[usize]) -> Gate {
    gate_of(calc.is_trusted(), |kind| {
        calc.control_expression_at(tab, form, at, rows, kind)
    })
}

/// What the expressions of a column of the table of the control at `at` come
/// to: with no row, whether the column is shown (its *visible when*); for a
/// row, how that cell is drawn (the others).
pub fn column_gate(
    calc: &Calc,
    tab: &str,
    form: &str,
    at: &[usize],
    column: &str,
    row: Option<usize>,
) -> Gate {
    column_gate_at(calc, tab, form, at, &[], column, row)
}

/// As [`column_gate`], for a table inside the items of repeaters, `outer`
/// being the item of each.
pub fn column_gate_at(
    calc: &Calc,
    tab: &str,
    form: &str,
    at: &[usize],
    outer: &[usize],
    column: &str,
    row: Option<usize>,
) -> Gate {
    gate_of(calc.is_trusted(), |kind| {
        if (kind == ExprKind::Visible) != row.is_none() {
            return None;
        }
        calc.column_expression_at(tab, form, at, outer, column, row, kind)
    })
}

fn gate_of(trusted: bool, state_of: impl Fn(ExprKind) -> Option<ExprState>) -> Gate {
    let mut gate = Gate::default();
    for kind in ExprKind::ALL {
        // The choices are the field's editor, and a value, a template and
        // items are what it shows, not how it is drawn.
        if matches!(
            kind,
            ExprKind::Choices | ExprKind::Value | ExprKind::Template | ExprKind::Items
        ) {
            continue;
        }
        let Some(state) = state_of(kind) else {
            continue;
        };
        gate.pending |= state.pending;
        let what = match kind {
            ExprKind::Visible => "Visible when",
            ExprKind::Enabled => "Enabled when",
            ExprKind::TextColour => "Text colour",
            ExprKind::FillColour => "Fill colour",
            ExprKind::Choices => "Choices",
            ExprKind::Value => "Value",
            ExprKind::Template => "Text",
            ExprKind::Items => "Items",
        };
        if !trusted {
            // Nothing runs: shown, and disabled if it has an *enabled when*.
            if kind == ExprKind::Enabled {
                gate.disabled = true;
                gate.why.push(
                    "Enabled when runs Python, so this workbook has to be trusted first."
                        .to_owned(),
                );
            }
            continue;
        }
        match state.result {
            // Not run yet: shown, and not usable until it is known.
            None => {
                if kind == ExprKind::Enabled {
                    gate.disabled = true;
                }
            }
            Some(Err(error)) => {
                if matches!(kind, ExprKind::Visible | ExprKind::Enabled) {
                    gate.disabled = true;
                }
                gate.why.push(format!("{what}: {error}"));
            }
            Some(Ok(value)) => match kind {
                ExprKind::Visible => gate.hidden = !truthy(&value),
                ExprKind::Enabled => gate.disabled |= !truthy(&value),
                ExprKind::TextColour | ExprKind::FillColour => match colour(&value) {
                    Ok(c) if kind == ExprKind::TextColour => gate.text = c,
                    Ok(c) => gate.fill = c,
                    Err(error) => gate.why.push(format!("{what}: {error}")),
                },
                ExprKind::Choices | ExprKind::Value | ExprKind::Template | ExprKind::Items => {}
            },
        }
    }
    gate
}

/// Black or white, whichever is easier to read on `fill`.
pub fn readable_on(fill: Color32) -> Color32 {
    let [r, g, b, _] = fill.to_srgba_unmultiplied();
    let brightness = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if brightness > 140.0 {
        Color32::from_rgb(0x20, 0x20, 0x20)
    } else {
        Color32::WHITE
    }
}

/// Python's truth of a value: false for `None`, `False`, zero, and an empty
/// string, list or record.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        Value::Date(_) | Value::DateTime(_) => true,
        Value::Error(_) => false,
    }
}

/// The colours that can be named, as CSS names them.
const NAMES: &[(&str, [u8; 3])] = &[
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("red", [255, 0, 0]),
    ("green", [0, 128, 0]),
    ("blue", [0, 0, 255]),
    ("yellow", [255, 255, 0]),
    ("orange", [255, 165, 0]),
    ("purple", [128, 0, 128]),
    ("pink", [255, 192, 203]),
    ("brown", [165, 42, 42]),
    ("grey", [128, 128, 128]),
    ("gray", [128, 128, 128]),
    ("silver", [192, 192, 192]),
    ("cyan", [0, 255, 255]),
    ("aqua", [0, 255, 255]),
    ("magenta", [255, 0, 255]),
    ("fuchsia", [255, 0, 255]),
    ("lime", [0, 255, 0]),
    ("maroon", [128, 0, 0]),
    ("navy", [0, 0, 128]),
    ("olive", [128, 128, 0]),
    ("teal", [0, 128, 128]),
    ("gold", [255, 215, 0]),
    ("lightgreen", [144, 238, 144]),
    ("lightblue", [173, 216, 230]),
    ("lightyellow", [255, 255, 224]),
    ("lightgrey", [211, 211, 211]),
    ("lightgray", [211, 211, 211]),
    ("darkred", [139, 0, 0]),
    ("darkgreen", [0, 100, 0]),
    ("darkblue", [0, 0, 139]),
];

/// The colour a value names: a name, `#rgb`, `#rrggbb` or `#rrggbbaa`, or a
/// list of three or four numbers from 0 to 255. `None` and the empty string
/// are the usual colour.
pub fn colour(value: &Value) -> Result<Option<Color32>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Str(s) if s.trim().is_empty() => Ok(None),
        Value::Str(s) => named(s.trim()).map(Some),
        Value::Array(items) if items.len() == 3 || items.len() == 4 => {
            let mut parts = [255u8; 4];
            for (slot, item) in parts.iter_mut().zip(items) {
                *slot = match item.value() {
                    Value::Int(i) if (0..=255).contains(i) => *i as u8,
                    _ => return Err(not_a_colour(value)),
                };
            }
            let [r, g, b, a] = parts;
            Ok(Some(Color32::from_rgba_unmultiplied(r, g, b, a)))
        }
        _ => Err(not_a_colour(value)),
    }
}

fn not_a_colour(value: &Value) -> String {
    format!(
        "a {} is not a colour; give a name such as \"red\", \"#rrggbb\" or (r, g, b)",
        value.kind_name()
    )
}

fn named(text: &str) -> Result<Color32, String> {
    let bad = || {
        format!("\"{text}\" is not a colour; give a name such as \"red\", \"#rrggbb\" or (r, g, b)")
    };
    if let Some(hex) = text.strip_prefix('#') {
        let digits: Option<Vec<u8>> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect();
        let digits = digits.ok_or_else(bad)?;
        let pair = |i: usize| digits[i] * 16 + digits[i + 1];
        return match digits.len() {
            3 => Ok(Color32::from_rgb(
                digits[0] * 17,
                digits[1] * 17,
                digits[2] * 17,
            )),
            6 => Ok(Color32::from_rgb(pair(0), pair(2), pair(4))),
            8 => Ok(Color32::from_rgba_unmultiplied(
                pair(0),
                pair(2),
                pair(4),
                pair(6),
            )),
            _ => Err(bad()),
        };
    }
    let lower = text.to_ascii_lowercase();
    NAMES
        .iter()
        .find(|(name, _)| *name == lower)
        .map(|(_, [r, g, b])| Color32::from_rgb(*r, *g, *b))
        .ok_or_else(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cliotree_core::Node;

    #[test]
    fn colours_by_name_by_hex_and_by_numbers() {
        assert_eq!(colour(&Value::Null), Ok(None));
        assert_eq!(
            colour(&Value::Str("Red".into())),
            Ok(Some(Color32::from_rgb(255, 0, 0)))
        );
        assert_eq!(
            colour(&Value::Str("#369".into())),
            Ok(Some(Color32::from_rgb(0x33, 0x66, 0x99)))
        );
        assert_eq!(
            colour(&Value::Str("#c00000".into())),
            Ok(Some(Color32::from_rgb(0xc0, 0, 0)))
        );
        let rgb = Value::Array(vec![
            Node::new(Value::Int(1)),
            Node::new(Value::Int(2)),
            Node::new(Value::Int(3)),
        ]);
        assert_eq!(colour(&rgb), Ok(Some(Color32::from_rgb(1, 2, 3))));
        assert!(colour(&Value::Str("#12".into())).is_err());
        assert!(colour(&Value::Str("reddish".into())).is_err());
        assert!(colour(&Value::Int(3)).is_err());
    }

    #[test]
    fn truth_is_pythons() {
        assert!(!truthy(&Value::Null));
        assert!(!truthy(&Value::Int(0)));
        assert!(!truthy(&Value::Str(String::new())));
        assert!(truthy(&Value::Str("no".into())));
        assert!(truthy(&Value::Float(0.5)));
    }
}

// ------------------------------------------------------------------ choices

/// An item of a list of choices: the code that is stored, as it is entered,
/// and the text that is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub code: String,
    pub text: String,
}

/// The list of choices of a field, as the field draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceList {
    /// The items, or why there are none: the expression failed, returned
    /// something that is not a list of items, or cannot run in a workbook
    /// that is not trusted. The field is then a text box of the code.
    pub items: Result<Vec<Choice>, String>,
    /// The expression is waiting to run again: the items are the last ones.
    pub pending: bool,
}

/// The choices of the field of the control at `at` (for item `item`, in the
/// design of a stacked repeater's items), or of a cell of a column of its
/// table (`column` and the row). None if it has no choices.
pub fn choice_list(
    calc: &Calc,
    tab: &str,
    form: &str,
    at: &[usize],
    column: Option<(&str, usize)>,
    rows: &[usize],
) -> Option<ChoiceList> {
    let state = match column {
        None => calc.control_expression_at(tab, form, at, rows, ExprKind::Choices),
        Some((name, row)) => {
            calc.column_expression_at(tab, form, at, rows, name, Some(row), ExprKind::Choices)
        }
    }?;
    Some(list_of(calc.is_trusted(), state))
}

fn list_of(trusted: bool, state: ExprState) -> ChoiceList {
    let items = if !trusted {
        Err("Choices runs Python, so this workbook has to be trusted first.".to_owned())
    } else {
        match state.result {
            // Not run yet: no items until it has.
            None => Ok(Vec::new()),
            Some(Err(error)) => Err(format!("Choices: {error}")),
            Some(Ok(value)) => choice_items(&value).map_err(|e| format!("Choices: {e}")),
        }
    };
    ChoiceList {
        items,
        pending: state.pending,
    }
}

/// The items of what a *choices* expression returned: a list (or tuple) whose
/// items are each a value (both the code and the text), a pair `(code, text)`,
/// or a dict with `value` and `label`. `None` is no list. An item that is
/// none of these is an error that names it.
pub fn choice_items(value: &Value) -> Result<Vec<Choice>, String> {
    let items = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        other => {
            return Err(format!(
                "a list of choices is expected, not {}",
                article(other.kind_name())
            ));
        }
    };
    let scalar = |v: &Value| match v {
        Value::Array(_) | Value::Object(_) | Value::Error(_) | Value::Null => None,
        other => Some(crate::rows::display(other)),
    };
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let wrong = || {
                format!(
                    "item {i} is not a value, a (code, text) pair or a dict with value and label"
                )
            };
            match item.value() {
                Value::Array(pair) if pair.len() == 2 => {
                    let code = scalar(pair[0].value()).ok_or_else(wrong)?;
                    let text = match pair[1].value() {
                        Value::Null => code.clone(),
                        v => scalar(v).ok_or_else(wrong)?,
                    };
                    Ok(Choice { code, text })
                }
                Value::Object(fields) => {
                    let code = fields
                        .get("value")
                        .and_then(|v| scalar(v.value()))
                        .ok_or_else(wrong)?;
                    let text = match fields.get("label").map(|v| v.value()) {
                        None | Some(Value::Null) => code.clone(),
                        Some(v) => scalar(v).ok_or_else(wrong)?,
                    };
                    Ok(Choice { code, text })
                }
                v => {
                    let code = scalar(v).ok_or_else(wrong)?;
                    Ok(Choice {
                        text: code.clone(),
                        code,
                    })
                }
            }
        })
        .collect()
}

fn article(kind: &str) -> String {
    if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
        format!("an {kind}")
    } else {
        format!("a {kind}")
    }
}

/// The items whose text contains `typed`, ignoring case, best matches first:
/// the text itself, then texts that start with it, then a word that starts
/// with it, then the rest; in the order of the list within each. Empty text
/// matches every item.
pub fn matching(items: &[Choice], typed: &str) -> Vec<usize> {
    let typed = typed.trim().to_lowercase();
    let mut ranked: Vec<(u8, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let text = item.text.to_lowercase();
            let rank = if typed.is_empty() || text == typed {
                0
            } else if text.starts_with(&typed) {
                1
            } else if text
                .match_indices(&typed)
                .any(|(at, _)| !text[..at].ends_with(char::is_alphanumeric))
            {
                2
            } else if text.contains(&typed) {
                3
            } else {
                return None;
            };
            Some((rank, i))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, i)| i).collect()
}

/// The item that typed text stands for: one whose text or code is the text,
/// ignoring case.
pub fn exact(items: &[Choice], typed: &str) -> Option<usize> {
    let typed = typed.trim().to_lowercase();
    items
        .iter()
        .position(|item| item.text.to_lowercase() == typed)
        .or_else(|| {
            items
                .iter()
                .position(|item| item.code.to_lowercase() == typed)
        })
}

#[cfg(test)]
mod choice_tests {
    use super::*;
    use cliotree_core::Node;

    fn list(items: Vec<Value>) -> Value {
        Value::Array(items.into_iter().map(Node::new).collect())
    }

    fn pair(code: &str, text: &str) -> Value {
        list(vec![Value::Str(code.into()), Value::Str(text.into())])
    }

    fn choice(code: &str, text: &str) -> Choice {
        Choice {
            code: code.into(),
            text: text.into(),
        }
    }

    #[test]
    fn items_of_every_shape() {
        let dict = [
            ("value".to_owned(), Node::new(Value::Int(3))),
            ("label".to_owned(), Node::new(Value::Str("Three".into()))),
        ]
        .into_iter()
        .collect();
        let value = list(vec![
            Value::Str("NL".into()),
            pair("BE", "Belgium"),
            Value::Object(dict),
            Value::Int(7),
        ]);
        assert_eq!(
            choice_items(&value).unwrap(),
            vec![
                choice("NL", "NL"),
                choice("BE", "Belgium"),
                choice("3", "Three"),
                choice("7", "7"),
            ]
        );
        assert_eq!(choice_items(&Value::Null).unwrap(), vec![]);
    }

    #[test]
    fn wrong_shapes_are_named() {
        let error = choice_items(&Value::Str("NL".into())).unwrap_err();
        assert_eq!(error, "a list of choices is expected, not a string");
        let error = choice_items(&list(vec![Value::Str("a".into()), list(vec![])])).unwrap_err();
        assert!(error.starts_with("item 1 is not"), "{error}");
        let dict = [("label".to_owned(), Node::new(Value::Str("x".into())))]
            .into_iter()
            .collect();
        assert!(choice_items(&list(vec![Value::Object(dict)])).is_err());
    }

    #[test]
    fn best_matches_first() {
        let items = vec![
            choice("1", "New Zealand"),
            choice("2", "Netherlands"),
            choice("3", "Zealand"),
            choice("4", "Brazil"),
            choice("5", "Ireland"),
        ];
        // "zeal": a word start in New Zealand, the text start of Zealand.
        assert_eq!(matching(&items, "zeal"), vec![2, 0]);
        // "land": inside a word in all but Brazil.
        assert_eq!(matching(&items, "LAND"), vec![0, 1, 2, 4]);
        assert_eq!(matching(&items, "ne"), vec![0, 1]);
        assert_eq!(matching(&items, "").len(), 5);
        assert_eq!(matching(&items, "xyz"), Vec::<usize>::new());
    }

    #[test]
    fn exact_by_text_then_code() {
        let items = vec![choice("NL", "Netherlands"), choice("BE", "Belgium")];
        assert_eq!(exact(&items, "belgium"), Some(1));
        assert_eq!(exact(&items, " nl "), Some(0));
        assert_eq!(exact(&items, "Bel"), None);
    }

    #[test]
    fn untrusted_or_failed_is_no_list() {
        let ok = ExprState {
            result: Some(Ok(list(vec![Value::Str("a".into())]))),
            pending: false,
        };
        assert!(list_of(false, ok.clone()).items.is_err());
        assert_eq!(list_of(true, ok).items.unwrap().len(), 1);
        let failed = ExprState {
            result: Some(Err("NameError: name 'x' is not defined".into())),
            pending: true,
        };
        let list = list_of(true, failed);
        assert!(list.pending);
        assert!(list.items.unwrap_err().contains("NameError"));
    }
}
