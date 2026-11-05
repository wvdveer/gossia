//! The parameters of a report (DESIGN.md, Reports, "Report parameters"):
//! values the user chooses when the report is shown, such as a date, a
//! period or org units. Expressions see them as `params.<name>`.
//!
//! A parameter's default and its list of choices are expressions, run as
//! the report's other expressions are. A period's default is a preset
//! instead, worked out here from today's date.

use chrono::{Datelike, Months, NaiveDate};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{Node, Value};

/// A parameter of a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportParam {
    /// The name expressions use, `params.<name>`: a Python identifier,
    /// unique in the report.
    pub name: String,
    /// What the parameter bar shows; the name when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: ParamKind,
    /// An expression for its value when none was chosen (not for a period).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// For a period: the preset it starts at when none was chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<PeriodPreset>,
    /// For a choice or choices: an expression for the list to choose from.
    /// Each item is a value, a `(value, label)` pair or a dict with `value`
    /// and `label`, as the choices of a form's field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<String>,
    /// The report is not drawn while it is empty.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}

impl ReportParam {
    pub fn new(name: &str, kind: ParamKind) -> ReportParam {
        ReportParam {
            name: name.to_owned(),
            label: None,
            kind,
            default: None,
            preset: None,
            choices: None,
            required: false,
        }
    }

    /// What the parameter bar calls it: its label, or its name.
    pub fn shown_name(&self) -> &str {
        self.label
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .unwrap_or(&self.name)
    }

    /// Its default expression, if it has one that can be used.
    pub fn default_expression(&self) -> Option<&str> {
        if self.kind == ParamKind::Period {
            return None;
        }
        text(&self.default)
    }

    /// Its choices expression, if it has one that can be used.
    pub fn choices_expression(&self) -> Option<&str> {
        if !self.kind.has_choices() {
            return None;
        }
        text(&self.choices)
    }
}

fn text(t: &Option<String>) -> Option<&str> {
    t.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// The kinds of parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// One date.
    Date,
    /// A start and an end date (`.start`, `.end`).
    Period,
    /// One value from a list.
    Choice,
    /// A subset of a list.
    Choices,
    Number,
    Text,
    YesNo,
}

impl ParamKind {
    pub const ALL: [ParamKind; 7] = [
        ParamKind::Date,
        ParamKind::Period,
        ParamKind::Choice,
        ParamKind::Choices,
        ParamKind::Number,
        ParamKind::Text,
        ParamKind::YesNo,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ParamKind::Date => "Date",
            ParamKind::Period => "Period",
            ParamKind::Choice => "Choice",
            ParamKind::Choices => "Choices",
            ParamKind::Number => "Number",
            ParamKind::Text => "Text",
            ParamKind::YesNo => "Yes/no",
        }
    }

    /// Whether it is chosen from a list.
    pub fn has_choices(self) -> bool {
        matches!(self, ParamKind::Choice | ParamKind::Choices)
    }

    /// Whether `value` is of this kind (an empty value, `None`, always is).
    pub fn accepts(self, value: &Value) -> bool {
        match (self, value) {
            (_, Value::Null) => true,
            (ParamKind::Date, Value::Date(_)) => true,
            (ParamKind::Period, Value::Object(fields)) => {
                ["start", "end"].iter().all(|k| {
                    fields
                        .get(*k)
                        .is_some_and(|n| matches!(n.value(), Value::Date(_)))
                }) && fields.len() == 2
            }
            (ParamKind::Choices, Value::Array(items)) => items
                .iter()
                .all(|n| !matches!(n.value(), Value::Array(_) | Value::Object(_))),
            (ParamKind::Choice, v) => {
                !matches!(v, Value::Array(_) | Value::Object(_) | Value::Error(_))
            }
            (ParamKind::Number, Value::Int(_) | Value::Float(_)) => true,
            (ParamKind::Text, Value::Str(_)) => true,
            (ParamKind::YesNo, Value::Bool(_)) => true,
            _ => false,
        }
    }
}

/// Whether a parameter's value is empty: `None`, empty text or an empty
/// list. A required parameter that is empty stops the report.
pub fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Str(s) => s.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

/// The value of a period, as expressions see it.
pub fn period(start: NaiveDate, end: NaiveDate) -> Value {
    let mut fields = IndexMap::new();
    fields.insert("start".to_owned(), Node::new(Value::Date(start)));
    fields.insert("end".to_owned(), Node::new(Value::Date(end)));
    Value::Object(fields)
}

/// The start and end of a period's value.
pub fn period_dates(value: &Value) -> Option<(NaiveDate, NaiveDate)> {
    let Value::Object(fields) = value else {
        return None;
    };
    let date = |k: &str| match fields.get(k).map(Node::value) {
        Some(Value::Date(d)) => Some(*d),
        _ => None,
    };
    Some((date("start")?, date("end")?))
}

/// The presets of a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodPreset {
    ThisMonth,
    LastMonth,
    ThisQuarter,
    LastQuarter,
    ThisYear,
    LastYear,
    YearToDate,
}

impl PeriodPreset {
    pub const ALL: [PeriodPreset; 7] = [
        PeriodPreset::ThisMonth,
        PeriodPreset::LastMonth,
        PeriodPreset::ThisQuarter,
        PeriodPreset::LastQuarter,
        PeriodPreset::ThisYear,
        PeriodPreset::LastYear,
        PeriodPreset::YearToDate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PeriodPreset::ThisMonth => "This month",
            PeriodPreset::LastMonth => "Last month",
            PeriodPreset::ThisQuarter => "This quarter",
            PeriodPreset::LastQuarter => "Last quarter",
            PeriodPreset::ThisYear => "This year",
            PeriodPreset::LastYear => "Last year",
            PeriodPreset::YearToDate => "Year to date",
        }
    }

    /// Its name on the command line: `last-month`.
    pub fn key(self) -> String {
        self.label().to_lowercase().replace(' ', "-")
    }

    /// The first and last day of the period, on `today`.
    pub fn dates(self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        let month_start = today.with_day(1).expect("every month has a first day");
        let quarter_start = NaiveDate::from_ymd_opt(today.year(), (today.month0() / 3) * 3 + 1, 1)
            .expect("a quarter starts on a day that exists");
        let year_start =
            NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("every year has a 1 January");
        let before = |d: NaiveDate| d.pred_opt().expect("not the first day there is");
        let months = |d: NaiveDate, n: u32| d - Months::new(n);
        let plus = |d: NaiveDate, n: u32| d + Months::new(n);
        match self {
            PeriodPreset::ThisMonth => (month_start, before(plus(month_start, 1))),
            PeriodPreset::LastMonth => (months(month_start, 1), before(month_start)),
            PeriodPreset::ThisQuarter => (quarter_start, before(plus(quarter_start, 3))),
            PeriodPreset::LastQuarter => (months(quarter_start, 3), before(quarter_start)),
            PeriodPreset::ThisYear => (year_start, before(plus(year_start, 12))),
            PeriodPreset::LastYear => (months(year_start, 12), before(year_start)),
            PeriodPreset::YearToDate => (year_start, today),
        }
    }

    /// The preset whose dates, on `today`, are these.
    pub fn of(start: NaiveDate, end: NaiveDate, today: NaiveDate) -> Option<PeriodPreset> {
        PeriodPreset::ALL
            .into_iter()
            .find(|p| p.dates(today) == (start, end))
    }
}

/// The values of a list of choices: each item is a value, a `(value, label)`
/// pair, or a dict with `value` and `label`. Each comes with its label, if
/// it has one; an item that is none of these is an error that names it.
pub fn choice_values(list: &Value) -> Result<Vec<(Value, Option<Value>)>, String> {
    let items = match list {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        other => {
            return Err(format!(
                "a list of choices is expected, not a value of kind {}",
                other.kind_name()
            ));
        }
    };
    let scalar = |v: &Value| !matches!(v, Value::Array(_) | Value::Object(_) | Value::Error(_));
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let wrong = || {
                format!(
                    "item {i} is not a value, a (value, label) pair or a dict with value and label"
                )
            };
            match item.value() {
                Value::Array(pair) if pair.len() == 2 && scalar(pair[0].value()) => {
                    let label = Some(pair[1].value().clone()).filter(|l| *l != Value::Null);
                    Ok((pair[0].value().clone(), label))
                }
                Value::Object(fields) => {
                    let value = fields
                        .get("value")
                        .map(|n| n.value().clone())
                        .filter(scalar)
                        .ok_or_else(wrong)?;
                    let label = fields
                        .get("label")
                        .map(|n| n.value().clone())
                        .filter(|l| *l != Value::Null);
                    Ok((value, label))
                }
                v if scalar(v) && *v != Value::Null => Ok((v.clone(), None)),
                _ => Err(wrong()),
            }
        })
        .collect()
}

/// A parameter's value as it is written on the command line (DESIGN.md,
/// "Batch export from the command line"): a date as `2026-10-31`, a period
/// as `2026-10-01..2026-10-31`, yes or no, a number as Python writes it, and
/// text as it is. A list is its values joined with commas, for showing; the
/// command line gives each value of a list with its own `--param`.
pub fn param_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Str(s) => s.clone(),
        Value::Bool(true) => "yes".to_owned(),
        Value::Bool(false) => "no".to_owned(),
        Value::Int(_) | Value::Float(_) => crate::format::plain_number(value),
        Value::Date(d) => d.format("%Y-%m-%d").to_string(),
        Value::Array(items) => items
            .iter()
            .map(|n| param_text(n.value()))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) if period_dates(value).is_some() => {
            let (start, end) = period_dates(value).expect("checked");
            format!("{}..{}", start.format("%Y-%m-%d"), end.format("%Y-%m-%d"))
        }
        other => crate::json::to_string(&Node::new(other.clone()), false).unwrap_or_default(),
    }
}
