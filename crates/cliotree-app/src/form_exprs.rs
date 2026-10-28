//! What the expressions on a form control make of it (DESIGN.md, "Forms",
//! "Expressions on controls"): whether it is shown, whether it can be used,
//! and the colours of its text and background. The expressions run on the
//! evaluation thread; this reads their latest results.

use cliotree_calc::{Calc, ExprKind};
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

/// What the expressions of the control at `at` of a form come to.
pub fn gate(calc: &Calc, tab: &str, form: &str, at: &[usize]) -> Gate {
    let mut gate = Gate::default();
    let trusted = calc.is_trusted();
    for kind in ExprKind::ALL {
        let Some(state) = calc.control_expression(tab, form, at, kind) else {
            continue;
        };
        gate.pending |= state.pending;
        let what = match kind {
            ExprKind::Visible => "Visible when",
            ExprKind::Enabled => "Enabled when",
            ExprKind::TextColour => "Text colour",
            ExprKind::FillColour => "Fill colour",
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
