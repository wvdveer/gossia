//! The parameters of reports (DESIGN.md, Reports, "Report parameters"):
//! `params` in expressions, defaults, lists, dropped choices and required
//! parameters.

use std::sync::Arc;

use gossia_core::form::{Control, ControlKind};
use gossia_core::report::{ParamKind, PeriodPreset, ReportDesign, ReportKind, ReportParam};
use gossia_core::{Node, Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, ExprKind, today};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "units": {"type": "array", "items": {"type": "string"}}
  }
}"#;

struct Book {
    calc: Calc,
    engine: PythonEngine,
}

fn field(value: &str) -> Control {
    let mut c = Control::new(ControlKind::Field);
    c.value = Some(value.to_owned());
    c
}

fn book(controls: Vec<Control>, params: Vec<ReportParam>) -> Book {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Org",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"units": ["North", "South", "East"]}"#)
            .unwrap()
            .node,
    )
    .unwrap();
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls = controls;
    design.params = params;
    calc.create_report("Sales", design).unwrap();
    let mut b = Book {
        calc,
        engine: PythonEngine::new(),
    };
    b.calc.show_report(Some("Sales"));
    b.calc.recalc(&b.engine);
    b
}

impl Book {
    fn value(&self, at: usize) -> Option<Result<Value, String>> {
        self.calc
            .report_expression("Sales", &[at], &[], ExprKind::Value)
            .and_then(|s| s.result)
    }

    fn set(&mut self, name: &str, value: Option<Value>) {
        self.calc.set_report_param("Sales", name, value);
        self.calc.recalc(&self.engine);
    }
}

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

fn unit() -> ReportParam {
    let mut p = ReportParam::new("unit", ParamKind::Choice);
    p.choices = Some("Org.units".into());
    p.default = Some("'North'".into());
    p
}

#[test]
fn expressions_see_params_and_a_change_runs_only_those_that_name_it() {
    let mut b = book(
        vec![field("f'Unit {params.unit}'"), field("len(Org.units)")],
        vec![unit()],
    );
    assert_eq!(b.value(0), Some(Ok(s("Unit North"))));
    assert_eq!(b.value(1), Some(Ok(Value::Int(3))));
    let state = &b.calc.report_params("Sales")[0];
    assert!(!state.chosen);
    assert_eq!(state.choices.as_ref().unwrap().as_ref().unwrap().len(), 3);

    b.calc.set_report_param("Sales", "unit", Some(s("South")));
    b.calc.sync_expressions();
    let pending = |at: usize| {
        b.calc
            .report_expression("Sales", &[at], &[], ExprKind::Value)
            .unwrap()
            .pending
    };
    assert!(pending(0));
    assert!(!pending(1), "it does not name params");
    b.calc.recalc(&b.engine);
    assert_eq!(b.value(0), Some(Ok(s("Unit South"))));
    assert!(b.calc.report_params("Sales")[0].chosen);

    // Back to the default.
    b.set("unit", None);
    assert_eq!(b.value(0), Some(Ok(s("Unit North"))));
}

#[test]
fn a_choice_no_longer_in_the_list_is_dropped() {
    let mut choices = ReportParam::new("units", ParamKind::Choices);
    choices.choices = Some("Org.units".into());
    let mut b = book(
        vec![field("params.unit"), field("params.units")],
        vec![unit(), choices],
    );
    b.set("unit", Some(s("East")));
    b.set(
        "units",
        Some(Value::Array(vec![
            Node::new(s("South")),
            Node::new(s("East")),
        ])),
    );
    assert_eq!(b.value(0), Some(Ok(s("East"))));
    // East goes from the list.
    let path = "units[2]".parse().unwrap();
    b.calc.remove("Org", &path).unwrap();
    b.calc.recalc(&b.engine);
    let states = b.calc.report_params("Sales");
    assert_eq!(states[0].value, Value::Null);
    assert_eq!(states[0].dropped, vec![s("East")]);
    assert_eq!(states[1].value, Value::Array(vec![Node::new(s("South"))]));
    assert_eq!(states[1].dropped, vec![s("East")]);
    assert_eq!(b.value(0), Some(Ok(Value::Null)));
}

#[test]
fn a_period_starts_at_its_preset_and_a_date_at_its_default() {
    let mut when = ReportParam::new("when", ParamKind::Period);
    when.preset = Some(PeriodPreset::LastMonth);
    let mut day = ReportParam::new("day", ParamKind::Date);
    day.default = Some("datetime.date(2026, 2, 3)".into());
    let mut bad = ReportParam::new("bad", ParamKind::Date);
    bad.default = Some("'soon'".into());
    let b = book(
        vec![
            field("params.when.start"),
            field("params.when.end"),
            field("params.day"),
            field("params.bad"),
        ],
        vec![when, day, bad],
    );
    let (start, end) = PeriodPreset::LastMonth.dates(today());
    assert_eq!(b.value(0), Some(Ok(Value::Date(start))));
    assert_eq!(b.value(1), Some(Ok(Value::Date(end))));
    assert_eq!(
        b.value(2),
        Some(Ok(Value::Date(
            chrono::NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()
        )))
    );
    assert_eq!(b.value(3), Some(Ok(Value::Null)));
    let states = b.calc.report_params("Sales");
    assert!(
        states[2]
            .default_error
            .as_deref()
            .unwrap()
            .contains("string"),
        "{:?}",
        states[2].default_error
    );
}

#[test]
fn nothing_runs_while_a_required_parameter_is_empty() {
    let mut customer = ReportParam::new("customer", ParamKind::Text);
    customer.label = Some("Customer".into());
    customer.required = true;
    let mut b = book(vec![field("params.customer.upper()")], vec![customer]);
    assert_eq!(b.calc.missing_params("Sales"), vec!["Customer".to_owned()]);
    assert_eq!(b.value(0), None);
    b.set("customer", Some(s("acme")));
    assert!(b.calc.missing_params("Sales").is_empty());
    assert_eq!(b.value(0), Some(Ok(s("ACME"))));
    // Chosen empty is empty again.
    b.set("customer", Some(s("")));
    assert_eq!(b.calc.missing_params("Sales"), vec!["Customer".to_owned()]);
}

#[test]
fn chosen_values_follow_a_rename_of_the_report() {
    let mut b = book(vec![field("params.unit")], vec![unit()]);
    b.set("unit", Some(s("South")));
    b.calc.rename_sheet("Sales", "Revenue").unwrap();
    assert_eq!(
        b.calc.chosen_params("Revenue").and_then(|m| m.get("unit")),
        Some(&s("South"))
    );
    assert!(b.calc.chosen_params("Sales").is_none());
}
