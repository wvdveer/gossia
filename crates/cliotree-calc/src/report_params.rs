//! The values of a report's parameters (DESIGN.md, Reports, "Report
//! parameters"), which its expressions see as `params.<name>`.
//!
//! A value the user chose is kept here, by report and parameter; one never
//! chosen is the parameter's default. A default and a list of choices are
//! expressions of the report, keyed with no place in the design (`at` is
//! empty and `column` is the parameter's name), and run while the report is
//! shown, as its other expressions do; they do not see `params`. A choice
//! that is no longer in its list is dropped from the value, and said.
//!
//! Only the expressions that name `params` are given it, so a change to a
//! parameter runs again only those.

use chrono::NaiveDate;
use cliotree_core::report::{ParamKind, ReportParam, choice_values, param_is_empty, period};
use cliotree_core::{DateTimeValue, Node, Value};
use indexmap::IndexMap;

use super::{Calc, ExprKey, ExprKind, REPORT_FORM};

/// An item of a list of choices: its value, and its label if it has one.
pub type Choice = (Value, Option<Value>);

/// A parameter as the parameter bar draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamState {
    pub name: String,
    /// What expressions see.
    pub value: Value,
    /// The user chose it; otherwise it is the default.
    pub chosen: bool,
    /// Chosen values that are no longer in the list, so are not in `value`.
    pub dropped: Vec<Value>,
    /// The list to choose from, each value with its label, or why there is
    /// none. None for a parameter that is not chosen from a list.
    pub choices: Option<Result<Vec<Choice>, String>>,
    /// Its default or its list is waiting to run again.
    pub pending: bool,
    /// Why the default could not be used.
    pub default_error: Option<String>,
}

/// The key of a parameter's default (`ExprKind::Value`) or list
/// (`ExprKind::Choices`).
pub(crate) fn param_key(report: &str, name: &str, kind: ExprKind) -> ExprKey {
    ExprKey {
        tab: report.to_owned(),
        form: REPORT_FORM.to_owned(),
        at: Vec::new(),
        column: Some(name.to_owned()),
        rows: Vec::new(),
        kind,
    }
}

/// Whether an expression names `params`.
pub(crate) fn mentions_params(text: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices("params").any(|(i, _)| {
        !text[..i].chars().next_back().is_some_and(ident)
            && !text[i + 6..].chars().next().is_some_and(ident)
    })
}

/// Today's date where the user is.
pub fn today() -> NaiveDate {
    let d = jiff::Zoned::now().date();
    NaiveDate::from_ymd_opt(i32::from(d.year()), d.month() as u32, d.day() as u32)
        .expect("today is a date")
}

/// A default that is a date and time, for a date: its day.
fn fitted(kind: ParamKind, value: Value) -> Result<Value, String> {
    let value = match (kind, value) {
        (ParamKind::Number, Value::Bool(_)) => {
            return Err("the default is yes or no, not a number".to_owned());
        }
        (ParamKind::Date, Value::DateTime(dt)) => Value::Date(match dt {
            DateTimeValue::Naive(t) => t.date(),
            DateTimeValue::Zoned(t) => t.date_naive(),
        }),
        (ParamKind::Choices, Value::Null) => Value::Null,
        (_, v) => v,
    };
    if kind.accepts(&value) {
        Ok(value)
    } else {
        Err(format!(
            "the default is a value of kind {}, which a {} parameter cannot hold",
            value.kind_name(),
            kind.label().to_lowercase()
        ))
    }
}

impl Calc {
    /// The values chosen for the parameters of `report`, by name. A
    /// parameter not here takes its default; one chosen empty is `None`.
    pub fn chosen_params(&self, report: &str) -> Option<&IndexMap<String, Value>> {
        self.params.get(report)
    }

    /// Chooses a value for a parameter of `report`, or (None) goes back to
    /// its default. This is not an edit: it is not in the history and
    /// recalculates only what names `params`.
    pub fn set_report_param(&mut self, report: &str, name: &str, value: Option<Value>) {
        let values = self.params.entry(report.to_owned()).or_default();
        let changed = match value {
            Some(v) => values.insert(name.to_owned(), v.clone()) != Some(v),
            None => values.shift_remove(name).is_some(),
        };
        if values.is_empty() {
            self.params.remove(report);
        }
        if changed {
            self.params_version += 1;
            self.exprs.resync = true;
        }
    }

    /// Puts back the chosen values of every report, as a workbook remembers
    /// them.
    pub fn set_chosen_params(&mut self, report: &str, values: IndexMap<String, Value>) {
        if values.is_empty() {
            self.params.remove(report);
        } else {
            self.params.insert(report.to_owned(), values);
        }
        self.params_version += 1;
        self.exprs.resync = true;
    }

    /// Changes whenever a parameter's chosen value changes.
    pub fn params_version(&self) -> u64 {
        self.params_version
    }

    /// The parameters of `report` as the parameter bar draws them, in its
    /// order.
    pub fn report_params(&self, report: &str) -> Vec<ParamState> {
        let Some(design) = self.reports.get(report) else {
            return Vec::new();
        };
        let today = today();
        design
            .params
            .iter()
            .map(|p| self.param_state(report, p, today))
            .collect()
    }

    /// The parameters of `report` that are required and empty, by what the
    /// parameter bar calls them. The report is not drawn while there are any.
    pub fn missing_params(&self, report: &str) -> Vec<String> {
        let Some(design) = self.reports.get(report) else {
            return Vec::new();
        };
        let today = today();
        design
            .params
            .iter()
            .filter(|p| p.required)
            .filter(|p| param_is_empty(&self.param_state(report, p, today).value))
            .map(|p| p.shown_name().to_owned())
            .collect()
    }

    /// `params` as expressions see it.
    pub(crate) fn params_value(&self, report: &str) -> Value {
        let today = today();
        let fields = self
            .reports
            .get(report)
            .map(|d| {
                d.params
                    .iter()
                    .map(|p| {
                        let state = self.param_state(report, p, today);
                        (p.name.clone(), Node::new(state.value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Value::Object(fields)
    }

    fn param_state(&self, report: &str, p: &ReportParam, today: NaiveDate) -> ParamState {
        let state = |kind| self.expression_state(&param_key(report, &p.name, kind));
        let mut pending = false;
        let choices = p.choices_expression().map(|_| {
            if !self.trusted {
                return Err(
                    "the list runs Python, so this workbook has to be trusted first".to_owned(),
                );
            }
            match state(ExprKind::Choices) {
                Some(s) => {
                    pending |= s.pending;
                    match s.result {
                        None => Ok(Vec::new()),
                        Some(Err(e)) => Err(e),
                        Some(Ok(list)) => choice_values(&list),
                    }
                }
                None => Ok(Vec::new()),
            }
        });
        let mut default_error = None;
        let chosen = self
            .params
            .get(report)
            .and_then(|m| m.get(&p.name))
            .cloned();
        let value = match &chosen {
            Some(v) => v.clone(),
            None if p.kind == ParamKind::Period => p.preset.map_or(Value::Null, |preset| {
                let (start, end) = preset.dates(today);
                period(start, end)
            }),
            None => match p.default_expression().and_then(|_| state(ExprKind::Value)) {
                Some(s) => {
                    pending |= s.pending;
                    match s.result {
                        Some(Ok(v)) => fitted(p.kind, v).unwrap_or_else(|e| {
                            default_error = Some(e);
                            Value::Null
                        }),
                        Some(Err(e)) => {
                            default_error = Some(e);
                            Value::Null
                        }
                        None => Value::Null,
                    }
                }
                None => Value::Null,
            },
        };
        // A choice no longer in the list is dropped. While the list has not
        // run yet, or failed, nothing is.
        let mut dropped = Vec::new();
        let value = match (&choices, value) {
            (Some(Ok(list)), value) if !(pending && list.is_empty()) => {
                let known = |v: &Value| list.iter().any(|(x, _)| x == v);
                match value {
                    Value::Array(items) => {
                        let (kept, gone): (Vec<Node>, Vec<Node>) =
                            items.into_iter().partition(|n| known(n.value()));
                        dropped = gone.into_iter().map(|n| n.value().clone()).collect();
                        if kept.is_empty() {
                            Value::Null
                        } else {
                            Value::Array(kept)
                        }
                    }
                    Value::Null => Value::Null,
                    v if known(&v) => v,
                    v => {
                        dropped.push(v);
                        Value::Null
                    }
                }
            }
            (_, Value::Array(items)) if items.is_empty() => Value::Null,
            (_, v) => v,
        };
        // Nothing runs in a workbook that is not trusted: the parameters are
        // as the results cache has them.
        let (value, choices) = match self.report_cache(report).filter(|_| !self.trusted) {
            Some(cache) => (
                cache.params.get(&p.name).cloned().unwrap_or(value),
                match cache.lists.get(&p.name) {
                    Some(list) => Some(choice_values(list)),
                    None => choices,
                },
            ),
            None => (value, choices),
        };
        ParamState {
            name: p.name.clone(),
            value,
            chosen: chosen.is_some(),
            dropped,
            choices,
            pending,
            default_error,
        }
    }
}
