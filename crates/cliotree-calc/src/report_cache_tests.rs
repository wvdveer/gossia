//! The results cache of reports (DESIGN.md, Reports, "Report storage and
//! the results cache"): taking a run, saving and reading it, its digest,
//! drawing from it when nothing runs, and keeping only the parameters.

use std::sync::Arc;

use cliotree_core::form::{Control, ControlKind};
use cliotree_core::report::{ParamKind, ReportDesign, ReportKind, ReportParam};
use cliotree_core::{ErrorKind, ErrorValue, Node, Value, json, jsonschema};
use indexmap::IndexMap;

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, ExprKind, Meta, ReportCache};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {"units": {"type": "array", "items": {"type": "string"}}}
}"#;

fn field(value: &str) -> Control {
    let mut c = Control::new(ControlKind::Field);
    c.value = Some(value.to_owned());
    c
}

fn workbook() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Org",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"units": ["North", "South"]}"#)
            .unwrap()
            .node,
    )
    .unwrap();
    calc.add_schema_sheet("OrgSchema", json::parse(SCHEMA).unwrap().node)
        .unwrap();
    calc.bind_schema("Org", "OrgSchema").unwrap();
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls = vec![
        field("f'{params.unit}: {len(Org.units)}'"),
        field("datetime.date(2026, 1, 31)"),
        field("1 / 0"),
    ];
    let mut unit = ReportParam::new("unit", ParamKind::Choice);
    unit.choices = Some("Org.units".into());
    unit.default = Some("'North'".into());
    design.params = vec![unit];
    calc.create_report("Sales", design).unwrap();
    calc
}

/// Shows the report, runs it, and shows nothing again, which takes its run.
fn run_report(calc: &mut Calc) {
    let engine = PythonEngine::new();
    calc.show_report(Some("Sales"));
    calc.recalc(&engine);
    calc.show_report(None);
}

fn reopen(calc: &Calc) -> Calc {
    let package = calc.to_package(&Meta::default()).unwrap();
    Calc::from_package(Arc::new(PythonSyntax), &package)
        .unwrap()
        .calc
}

fn s(t: &str) -> Value {
    Value::Str(t.to_owned())
}

#[test]
fn values_keep_their_kinds_in_the_file() {
    let mut odd = IndexMap::new();
    odd.insert("$ref".to_owned(), Node::new(Value::Int(1)));
    let values = [
        Value::Null,
        Value::Bool(true),
        Value::Int(3),
        Value::Float(2.0),
        Value::Float(f64::NAN),
        s("text"),
        Value::Date(chrono::NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()),
        Value::Array(vec![Node::new(s("a"))]),
        Value::Object(odd),
        Value::Error(ErrorValue {
            kind: ErrorKind::DivZero,
            message: "division by zero".into(),
            origin: None,
        }),
    ];
    for value in values {
        let back = crate::calc::value_from_json(&crate::calc::value_to_json(&value)).unwrap();
        match (&value, &back) {
            (Value::Float(a), Value::Float(b)) if a.is_nan() => assert!(b.is_nan()),
            _ => assert_eq!(back, value),
        }
    }
}

#[test]
fn a_run_is_taken_when_the_report_stops_being_shown_and_saved_with_the_workbook() {
    let mut calc = workbook();
    calc.set_report_param("Sales", "unit", Some(s("South")));
    run_report(&mut calc);
    let cache = calc.report_cache("Sales").unwrap().clone();
    assert!(!cache.calculated.is_empty());
    assert_eq!(cache.chosen.get("unit"), Some(&s("South")));
    assert_eq!(cache.params.get("unit"), Some(&s("South")));
    assert_eq!(
        calc.cached_expression("Sales", &[0], None, &[], ExprKind::Value),
        Some(&Ok(s("South: 2")))
    );
    assert!(matches!(
        calc.cached_expression("Sales", &[2], None, &[], ExprKind::Value),
        Some(Err(_))
    ));

    // Saved and read back: drawn from it, with the parameter chosen.
    let opened = reopen(&calc);
    assert_eq!(opened.report_cache("Sales"), Some(&cache));
    assert_eq!(
        opened.chosen_params("Sales").and_then(|m| m.get("unit")),
        Some(&s("South"))
    );

    // Run again with nothing changed: the time it was calculated is kept.
    let mut again = opened;
    run_report(&mut again);
    assert_eq!(again.report_cache("Sales"), Some(&cache));
}

#[test]
fn a_cache_for_another_design_is_not_used() {
    let mut calc = workbook();
    run_report(&mut calc);
    assert!(calc.report_cache("Sales").is_some());
    let mut design = calc.report("Sales").unwrap().clone();
    design.layout.controls.push(field("2"));
    calc.set_report("Sales", design).unwrap();
    assert!(calc.report_cache("Sales").is_none());
    assert!(calc.stored_report_cache("Sales").is_some());
    // Nor are its parameters, when the workbook is read.
    calc.set_report_param("Sales", "unit", Some(s("South")));
    let opened = reopen(&calc);
    assert!(opened.chosen_params("Sales").is_none());
}

#[test]
fn an_untrusted_workbook_shows_the_parameters_of_the_cache() {
    let mut calc = workbook();
    calc.set_report_param("Sales", "unit", Some(s("South")));
    run_report(&mut calc);
    let mut opened = reopen(&calc);
    opened.set_trusted(false);
    let state = &opened.report_params("Sales")[0];
    assert_eq!(state.value, s("South"));
    assert_eq!(state.choices.as_ref().unwrap().as_ref().unwrap().len(), 2);
}

#[test]
fn without_keeping_results_only_the_parameters_are_written() {
    let mut calc = workbook();
    calc.set_report_param("Sales", "unit", Some(s("South")));
    run_report(&mut calc);
    let mut settings = calc.settings().clone();
    settings.set_keep_report_results(false);
    calc.set_settings(settings);
    let package = calc.to_package(&Meta::default()).unwrap();
    let text =
        String::from_utf8(package.get("reports/Sales/cache.json").unwrap().to_vec()).unwrap();
    assert!(text.contains("\"chosen\""), "{text}");
    assert!(!text.contains("\"results\""), "{text}");
    let settings_text = String::from_utf8(package.get("settings.json").unwrap().to_vec()).unwrap();
    assert!(settings_text.contains("\"keep_report_results\": false"));
    let opened = Calc::from_package(Arc::new(PythonSyntax), &package)
        .unwrap()
        .calc;
    assert!(!opened.settings().keep_report_results());
    assert_eq!(
        opened.chosen_params("Sales").and_then(|m| m.get("unit")),
        Some(&s("South"))
    );
    // Nothing to draw from.
    assert_eq!(
        opened.cached_expression("Sales", &[0], None, &[], ExprKind::Value),
        None
    );
    // The bytes are the same when saved again unchanged.
    let again = ReportCache::from_json(&text).unwrap();
    assert_eq!(again.to_json(false), text);
}
