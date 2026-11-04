//! The parameters of a report on the command line (DESIGN.md, "Batch export
//! from the command line", "Parameter values"): each value is text, read by
//! the parameter's kind. A parameter not given takes its default; a required
//! one with neither, an unknown name, a value that does not read and one
//! that is not in the list are each an error naming the parameter.

use chrono::NaiveDate;
use cliotree_calc::{Calc, ParamState};
use cliotree_core::Value;
use cliotree_core::report::{ParamKind, PeriodPreset, ReportParam, param_text, period};

use crate::{Exit, Failure, Opened};

/// Starts the report afresh, without its cache or remembered values, and
/// waits for its defaults and lists.
pub fn start(opened: &mut Opened, report: &str) -> Result<(), Failure> {
    let calc = opened.session.calc_mut();
    calc.forget_report_run(report);
    calc.show_report(Some(report));
    opened.wait()
}

/// Chooses the values given, waits for the report, and checks that no
/// required parameter is empty.
pub fn choose(
    opened: &mut Opened,
    report: &str,
    given: &[(String, String)],
) -> Result<(), Failure> {
    let design = opened
        .calc()
        .report(report)
        .expect("the report is there")
        .clone();
    for (name, _) in given {
        if !design.params.iter().any(|p| &p.name == name) {
            let names: Vec<&str> = design.params.iter().map(|p| p.name.as_str()).collect();
            let known = if names.is_empty() {
                "it has no parameters".to_owned()
            } else {
                format!("its parameters are {}", names.join(", "))
            };
            return Err(Failure::new(
                Exit::Param,
                format!("the report {report} has no parameter {name}: {known}"),
            ));
        }
    }
    let states = opened.calc().report_params(report);
    for (param, state) in design.params.iter().zip(&states) {
        let texts: Vec<&str> = given
            .iter()
            .filter(|(n, _)| n == &param.name)
            .map(|(_, v)| v.as_str())
            .collect();
        if texts.is_empty() {
            continue;
        }
        let value = read(param, state, &texts)
            .map_err(|why| Failure::new(Exit::Param, format!("--param {}: {why}", param.name)))?;
        opened
            .session
            .calc_mut()
            .set_report_param(report, &param.name, Some(value));
    }
    opened.wait()?;
    let missing = opened.calc().missing_params(report);
    if !missing.is_empty() {
        return Err(Failure::new(
            Exit::Param,
            format!(
                "the report {report} needs {}, which {} no value and no default",
                missing.join(", "),
                if missing.len() == 1 { "has" } else { "have" }
            ),
        ));
    }
    Ok(())
}

/// Reads the text given for a parameter by its kind.
pub fn read(param: &ReportParam, state: &ParamState, texts: &[&str]) -> Result<Value, String> {
    if param.kind != ParamKind::Choices && texts.len() > 1 {
        return Err("is given more than once; only a Choices parameter takes several".to_owned());
    }
    let text = texts[0];
    match param.kind {
        ParamKind::Date => read_date(text).map(Value::Date),
        ParamKind::Period => read_period(text),
        ParamKind::Number => read_number(text),
        ParamKind::Text => Ok(Value::Str(text.to_owned())),
        ParamKind::YesNo => match text.trim().to_lowercase().as_str() {
            "yes" | "true" => Ok(Value::Bool(true)),
            "no" | "false" => Ok(Value::Bool(false)),
            _ => Err(format!("{text} is not yes, no, true or false")),
        },
        ParamKind::Choice => in_list(state, text),
        ParamKind::Choices => {
            let mut values = Vec::new();
            for text in texts {
                let value = in_list(state, text)?;
                if !values.contains(&value) {
                    values.push(value);
                }
            }
            Ok(Value::Array(
                values.into_iter().map(cliotree_core::Node::new).collect(),
            ))
        }
    }
}

fn read_date(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| format!("{text} is not a date written as 2026-10-31"))
}

fn read_number(text: &str) -> Result<Value, String> {
    let t = text.trim();
    if let Ok(n) = t.parse::<i64>() {
        return Ok(Value::Int(n));
    }
    match t.parse::<f64>() {
        Ok(x) if x.is_finite() => Ok(Value::Float(x)),
        _ => Err(format!("{text} is not a number")),
    }
}

fn read_period(text: &str) -> Result<Value, String> {
    let t = text.trim();
    if let Some((start, end)) = t.split_once("..") {
        let (start, end) = (read_date(start)?, read_date(end)?);
        if end < start {
            return Err(format!("{text} ends before it starts"));
        }
        return Ok(period(start, end));
    }
    let today = cliotree_calc::today();
    PeriodPreset::ALL
        .into_iter()
        .find(|p| p.key() == t.to_lowercase())
        .map(|p| {
            let (start, end) = p.dates(today);
            period(start, end)
        })
        .ok_or_else(|| {
            let presets: Vec<String> = PeriodPreset::ALL
                .into_iter()
                .map(PeriodPreset::key)
                .collect();
            format!(
                "{text} is neither a period written as 2026-10-01..2026-10-31 nor one of {}",
                presets.join(", ")
            )
        })
}

/// The value of the list that is written `text`, matched exactly against
/// the values, not the labels.
fn in_list(state: &ParamState, text: &str) -> Result<Value, String> {
    let list = match &state.choices {
        Some(Ok(list)) => list,
        Some(Err(why)) => return Err(format!("its list of choices cannot be made: {why}")),
        None => return Err("it has no list of choices".to_owned()),
    };
    list.iter()
        .map(|(value, _)| value)
        .find(|value| param_text(value) == text)
        .cloned()
        .ok_or_else(|| {
            let values: Vec<String> = list.iter().map(|(v, _)| param_text(v)).collect();
            if values.is_empty() {
                format!("{text} is not in its list, which is empty")
            } else {
                format!("{text} is not in its list: {}", values.join(", "))
            }
        })
}

/// What `--list-params` prints: each parameter with its kind, its default
/// and its list of choices.
pub fn listing(calc: &Calc, report: &str) -> String {
    let Some(design) = calc.report(report) else {
        return String::new();
    };
    if design.params.is_empty() {
        return format!("The report {report} has no parameters.\n");
    }
    let mut out = String::new();
    for (param, state) in design.params.iter().zip(calc.report_params(report)) {
        let mut head = format!("{} ({}", param.name, param.kind.label());
        if param.required {
            head.push_str(", required");
        }
        head.push(')');
        if param.shown_name() != param.name {
            head.push_str(&format!(" \"{}\"", param.shown_name()));
        }
        out.push_str(&head);
        out.push('\n');
        let default = match &state.default_error {
            Some(why) => format!("cannot be used: {why}"),
            None if cliotree_core::report::param_is_empty(&state.value) => "none".to_owned(),
            None => param_text(&state.value),
        };
        out.push_str(&format!("  default: {default}\n"));
        if param.kind == ParamKind::Period {
            let presets: Vec<String> = PeriodPreset::ALL
                .into_iter()
                .map(PeriodPreset::key)
                .collect();
            out.push_str(&format!("  presets: {}\n", presets.join(", ")));
        }
        match &state.choices {
            Some(Ok(list)) => {
                let items: Vec<String> = list
                    .iter()
                    .map(|(value, label)| match label {
                        Some(label) if param_text(label) != param_text(value) => {
                            format!("{} ({})", param_text(value), param_text(label))
                        }
                        _ => param_text(value),
                    })
                    .collect();
                out.push_str(&format!("  choices: {}\n", items.join(", ")));
            }
            Some(Err(why)) => out.push_str(&format!("  choices: cannot be made: {why}\n")),
            None => {}
        }
    }
    out
}
