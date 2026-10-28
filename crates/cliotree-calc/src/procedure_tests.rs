//! Tests of button procedures (DESIGN.md, "Button procedures"): what the
//! writes do, all or nothing, and how a procedure runs on the evaluation
//! thread. They run the real interpreter.

use std::time::Duration;

use cliotree_core::{Path, Value, json, jsonschema};

use crate::python::{self, PythonEngine, PythonSettings, PythonSyntax};
use crate::{Calc, Call, EvalSettings, Evaluator, Finished, Session};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "name": {"type": "string"},
    "count": {"type": "integer"},
    "note": {"type": "string"},
    "total": {"type": "integer"},
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {"qty": {"type": "integer"}, "price": {"type": "number"}},
        "required": ["qty", "price"]
      }
    }
  },
  "required": ["name", "count", "total", "items"]
}"#;

const DATA: &str = r#"{"name": "a", "count": 1, "total": 0, "items": [{"qty": 1, "price": 2.0}]}"#;

const LOG_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {"log": {"type": "array", "items": {"type": "string"}}},
  "required": ["log"]
}"#;

const LONG: Duration = Duration::from_secs(60);

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn calc() -> Calc {
    let mut calc = Calc::new(std::sync::Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.add_sheet(
        "T",
        jsonschema::lower_str(LOG_SCHEMA).unwrap(),
        json::parse(r#"{"log": []}"#).unwrap().node,
    )
    .unwrap();
    calc
}

fn session_with(source: &str, settings: EvalSettings) -> Session {
    let mut calc = calc();
    calc.add_module("procs", source).unwrap();
    calc.clear_history();
    Session::new(
        calc,
        Evaluator::new(python::factory(PythonSettings::default()), settings),
    )
}

fn settings() -> EvalSettings {
    EvalSettings {
        formula_limit: Some(Duration::from_secs(20)),
        recalc_limit: Some(Duration::from_secs(60)),
        procedure_limit: Some(Duration::from_secs(60)),
        grace: Duration::from_secs(5),
        max_abandoned: 20,
        stack_size: 64 * 1024 * 1024,
    }
}

fn session(source: &str) -> Session {
    session_with(source, settings())
}

fn call(procedure: &str) -> Call {
    Call {
        procedure: procedure.to_owned(),
        sheet: "S".to_owned(),
        form: "Default".to_owned(),
        button: "Go".to_owned(),
        time_limit: None,
    }
}

/// Runs a procedure through the session and waits for its answer.
fn run(s: &mut Session, procedure: &str) -> Finished {
    s.run_procedure(call(procedure)).unwrap();
    assert!(s.wait_idle(LONG), "the workbook did not settle");
    let mut done = s.take_finished();
    assert_eq!(done.len(), 1);
    done.remove(0)
}

fn get(s: &Session, sheet: &str, path: &str) -> Option<Value> {
    s.calc()
        .data(sheet)
        .unwrap()
        .get(&p(path))
        .map(|n| n.value().clone())
}

#[test]
fn the_writes_of_a_procedure_are_one_undoable_edit() {
    let mut s = session(
        "def go(form, ctx):\n    form.name = 'changed'\n    form.count = form.count + 1\n    form.items.append({'qty': 2, 'price': 1.5})\n",
    );
    let done = run(&mut s, "go");
    assert_eq!(done.outcome, Ok(()));
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("changed".into())));
    assert_eq!(get(&s, "S", "count"), Some(Value::Int(2)));
    assert_eq!(get(&s, "S", "items[1].qty"), Some(Value::Int(2)));
    assert_eq!(s.calc().undo_label(), Some("Run Go"));
    assert_eq!(s.calc().history_len().0, 1, "all the writes are one entry");

    s.calc_mut().undo().unwrap();
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("a".into())));
    assert_eq!(get(&s, "S", "count"), Some(Value::Int(1)));
    assert_eq!(get(&s, "S", "items[1]"), None);
}

#[test]
fn a_procedure_reads_the_data_as_it_was_and_knows_its_form_and_button() {
    let mut s = session(
        "def go(form, ctx):\n    form.count = 10\n    form.name = ctx.form + '/' + ctx.button + '/' + str(form.count)\n",
    );
    run(&mut s, "go");
    // `form.count` was still 1 when the name was made: reads are of the data
    // the procedure started from.
    assert_eq!(
        get(&s, "S", "name"),
        Some(Value::Str("Default/Go/1".into()))
    );
    assert_eq!(get(&s, "S", "count"), Some(Value::Int(10)));
}

#[test]
fn a_procedure_writes_to_other_tabs_through_ctx() {
    let mut s = session("def go(form, ctx):\n    ctx.tab('T').log.append('hello')\n");
    run(&mut s, "go");
    assert_eq!(get(&s, "T", "log[0]"), Some(Value::Str("hello".into())));
}

#[test]
fn a_write_that_does_not_fit_the_schema_refuses_all_of_them() {
    let mut s = session("def go(form, ctx):\n    form.name = 'changed'\n    form.count = 'many'\n");
    let done = run(&mut s, "go");
    let message = done.outcome.unwrap_err();
    assert!(message.contains("count"), "{message}");
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("a".into())));
    assert_eq!(get(&s, "S", "count"), Some(Value::Int(1)));
    assert!(!s.calc().can_undo(), "a refused procedure leaves no entry");
    assert!(!s.calc().can_redo());
}

#[test]
fn an_exception_applies_none_of_the_writes_and_says_why() {
    let mut s = session(
        "def go(form, ctx):\n    form.name = 'changed'\n    raise ValueError('no stock left')\n",
    );
    let done = run(&mut s, "go");
    let message = done.outcome.unwrap_err();
    assert!(message.contains("no stock left"), "{message}");
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("a".into())));
    assert!(!s.calc().can_undo());
}

#[test]
fn writing_a_computed_node_is_refused() {
    let mut s = session("def go(form, ctx):\n    form.name = 'changed'\n    form.total = 5\n");
    s.calc_mut()
        .set_formula("S", &p("total"), "parent.count * 2")
        .unwrap();
    assert!(s.wait_idle(LONG));
    let done = run(&mut s, "go");
    let message = done.outcome.unwrap_err();
    assert!(message.contains("formula"), "{message}");
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("a".into())));
    assert_eq!(get(&s, "S", "total"), Some(Value::Int(2)));
}

#[test]
fn removing_an_optional_field_works_and_a_required_one_is_refused() {
    let mut s = session(
        "def clear_note(form, ctx):\n    del form.note\ndef set_note(form, ctx):\n    form.note = 'x'\ndef none_note(form, ctx):\n    form.note = None\ndef drop_name(form, ctx):\n    del form.name\n",
    );
    assert_eq!(run(&mut s, "set_note").outcome, Ok(()));
    assert_eq!(get(&s, "S", "note"), Some(Value::Str("x".into())));
    assert_eq!(run(&mut s, "clear_note").outcome, Ok(()));
    assert_eq!(get(&s, "S", "note"), None);
    assert_eq!(run(&mut s, "set_note").outcome, Ok(()));
    assert_eq!(run(&mut s, "none_note").outcome, Ok(()));
    assert_eq!(get(&s, "S", "note"), None, "None clears an optional field");
    let message = run(&mut s, "drop_name").outcome.unwrap_err();
    assert!(message.contains("required"), "{message}");
    assert!(get(&s, "S", "name").is_some());
}

#[test]
fn an_item_is_removed_and_replaced_by_index() {
    let mut s = session(
        "def go(form, ctx):\n    form.items.append({'qty': 5, 'price': 1.0})\n    form.items[0] = {'qty': 9, 'price': 3.0}\n    del form.items[0]\n",
    );
    // Index 0 is removed after being set, leaving the appended item.
    let done = run(&mut s, "go");
    assert_eq!(done.outcome, Ok(()));
    assert_eq!(get(&s, "S", "items[0].qty"), Some(Value::Int(5)));
    assert_eq!(get(&s, "S", "items[1]"), None);
}

#[test]
fn only_a_function_of_a_module_can_be_a_procedure() {
    let mut s = session("def go(form, ctx):\n    pass\ndef _hidden(form, ctx):\n    pass\n");
    for name in ["sum", "S", "procs", "_hidden", "missing", "procs.missing"] {
        let message = run(&mut s, name).outcome.unwrap_err();
        assert!(message.contains("no procedure"), "{name}: {message}");
    }
    assert_eq!(run(&mut s, "procs.go").outcome, Ok(()));
}

/// What the property page offers a button: the public functions of the
/// modules by bare name, none from a module that does not parse or that
/// failed to load. Each of them runs.
#[test]
fn the_names_a_button_can_run_are_listed() {
    let mut s = session("def go(form, ctx):\n    pass\ndef _hidden(form, ctx):\n    pass\n");
    let calc = s.calc_mut();
    calc.add_module("extra", "def ship(form, ctx):\n    pass\n")
        .unwrap();
    // `go` clashes with procs, so this module fails when it is loaded.
    calc.add_module(
        "more",
        "def go(form, ctx):\n    pass\ndef stop(form, ctx):\n    pass\n",
    )
    .unwrap();
    calc.add_module("broken", "def half(:\n").unwrap();
    assert_eq!(
        calc.procedure_names(),
        ["go", "ship", "stop"],
        "not loaded yet"
    );
    assert_eq!(run(&mut s, "go").outcome, Ok(()));
    let names = s.calc().procedure_names();
    assert_eq!(names, ["go", "ship"]);
    for name in names {
        assert_eq!(run(&mut s, &name).outcome, Ok(()), "{name}");
    }
}

#[test]
fn a_formula_cannot_change_data() {
    let mut s = session("def poke():\n    S.count = 99\n    return 1\n");
    s.calc_mut()
        .set_formula("S", &p("total"), "poke()")
        .unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "S", "count"), Some(Value::Int(1)));
    let cell = s.calc().cell("S", &p("total")).unwrap();
    assert!(cell.error.is_some(), "the formula failed");
    assert!(
        cell.error.unwrap().message.contains("button procedure"),
        "{:?}",
        cell.error
    );
}

#[test]
fn a_procedure_waits_for_the_workbook_to_be_up_to_date() {
    let mut s = session("def go(form, ctx):\n    form.name = str(form.total)\n");
    s.calc_mut()
        .set_formula("S", &p("total"), "parent.count * 2")
        .unwrap();
    assert!(s.wait_idle(LONG));
    // An edit makes the formula out of date; the click comes straight after.
    s.calc_mut()
        .set_value("S", &p("count"), Value::Int(21))
        .unwrap();
    s.run_procedure(call("go")).unwrap();
    assert!(s.wait_idle(LONG));
    let done = s.take_finished();
    assert_eq!(done[0].outcome, Ok(()));
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("42".into())));
}

#[test]
fn one_procedure_runs_at_a_time() {
    let mut s = session("def go(form, ctx):\n    pass\n");
    s.run_procedure(call("go")).unwrap();
    assert_eq!(s.status().procedure.as_deref(), Some("Go"));
    let message = s.run_procedure(call("go")).unwrap_err();
    assert!(message.contains("running"), "{message}");
    assert!(s.wait_idle(LONG));
    assert_eq!(s.status().procedure, None);
    assert!(s.run_procedure(call("go")).is_ok());
}

#[test]
fn an_untrusted_workbook_runs_no_procedure() {
    let mut s = session("def go(form, ctx):\n    form.name = 'changed'\n");
    s.calc_mut().set_trusted(false);
    let message = s.run_procedure(call("go")).unwrap_err();
    assert!(message.contains("trusted"), "{message}");
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("a".into())));
}

#[test]
fn a_procedure_that_runs_too_long_is_stopped_and_writes_nothing() {
    let mut limits = settings();
    limits.procedure_limit = Some(Duration::from_millis(300));
    let mut s = session_with(
        "def go(form, ctx):\n    form.name = 'changed'\n    while True:\n        pass\n",
        limits,
    );
    let done = run(&mut s, "go");
    let message = done.outcome.unwrap_err();
    assert!(message.contains("time"), "{message}");
    assert_eq!(get(&s, "S", "name"), Some(Value::Str("a".into())));
}

#[test]
fn a_node_edited_while_the_procedure_ran_refuses_the_writes() {
    let mut calc = calc();
    calc.add_module(
        "procs",
        "def go(form, ctx):\n    form.name = 'changed'\n    form.count = 5\n",
    )
    .unwrap();
    let engine = PythonEngine::new();
    let result = calc.snapshot().run_call(&engine, &call("go"));
    // The user edits a node the procedure wrote, after it started.
    calc.set_value("S", &p("count"), Value::Int(7)).unwrap();
    let message = calc.apply_procedure(result).unwrap_err();
    assert!(message.contains("count"), "{message}");
    assert!(message.contains("edited"), "{message}");
    assert_eq!(
        calc.data("S").unwrap().get(&p("name")).unwrap().value(),
        &Value::Str("a".into())
    );
}

#[test]
fn an_edit_to_another_node_does_not_refuse_the_writes() {
    let mut calc = calc();
    calc.add_module("procs", "def go(form, ctx):\n    form.name = 'changed'\n")
        .unwrap();
    let engine = PythonEngine::new();
    let result = calc.snapshot().run_call(&engine, &call("go"));
    calc.set_value("S", &p("count"), Value::Int(7)).unwrap();
    calc.apply_procedure(result).unwrap();
    assert_eq!(
        calc.data("S").unwrap().get(&p("name")).unwrap().value(),
        &Value::Str("changed".into())
    );
}
