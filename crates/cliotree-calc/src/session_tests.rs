//! Tests of the evaluation thread and the session (DESIGN.md, "Evaluation and
//! scheduling"). They run the real interpreter on the real evaluation thread.

use std::time::{Duration, Instant};

use cliotree_core::{ErrorKind, Path, Value, json, jsonschema};

use crate::python::{self, PythonSettings, PythonSyntax};
use crate::{Calc, EvalSettings, Evaluator, Session};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "a": {"type": "integer"},
    "b": {"type": "integer"},
    "c": {"type": "integer"},
    "list": {"type": "array", "items": {"type": "integer"}}
  },
  "required": ["a", "b", "c", "list"]
}"#;

const DATA: &str = r#"{"a": 1, "b": 2, "c": 0, "list": [0, 0, 0, 0, 0, 0]}"#;

const LONG: Duration = Duration::from_secs(60);

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn session(settings: EvalSettings) -> Session {
    session_with(settings, PythonSettings::default())
}

/// A session whose `range` allows `LONG_CALL`, which is above the default
/// maximum range length.
fn long_call_session(settings: EvalSettings) -> Session {
    session_with(
        settings,
        PythonSettings {
            max_range: 10_i64.pow(7),
        },
    )
}

/// One long call into the interpreter's own code: nothing looks at signals
/// while it runs.
const LONG_CALL: &str = "sum(range(10 ** 7)) + sum(range(10 ** 7)) + sum(range(10 ** 7))";

fn session_with(settings: EvalSettings, python: PythonSettings) -> Session {
    let mut calc = Calc::new(std::sync::Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    Session::new(calc, Evaluator::new(python::factory(python), settings))
}

fn quick() -> EvalSettings {
    EvalSettings {
        formula_limit: Some(Duration::from_secs(20)),
        recalc_limit: Some(Duration::from_secs(60)),
        procedure_limit: Some(Duration::from_secs(60)),
        grace: Duration::from_secs(5),
        max_abandoned: 20,
        stack_size: 64 * 1024 * 1024,
    }
}

fn get(s: &Session, path: &str) -> Option<Value> {
    s.calc()
        .data("S")
        .unwrap()
        .get(&p(path))
        .map(|n| n.value().clone())
}

fn kind(s: &Session, path: &str) -> Option<ErrorKind> {
    match get(s, path) {
        Some(Value::Error(e)) => Some(e.kind),
        _ => None,
    }
}

/// The interface never waits: the cell keeps its last value and is marked
/// pending until the result arrives.
#[test]
fn a_formula_is_calculated_on_the_evaluation_thread_while_the_cell_shows_its_last_value() {
    let mut s = session(quick());
    s.calc_mut().set_formula("S", &p("c"), "S.a + S.b").unwrap();
    let started = Instant::now();
    s.tick();
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "tick does not wait for the interpreter"
    );
    let cell = s.calc().cell("S", &p("c")).unwrap();
    assert!(cell.pending, "marked pending");
    assert_eq!(get(&s, "c"), Some(Value::Int(0)), "last value");
    assert!(s.status().pending >= 1);
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "c"), Some(Value::Int(3)));
    assert!(!s.calc().cell("S", &p("c")).unwrap().pending);
    assert!(s.is_idle());
}

/// A result computed from inputs that were edited in the meantime is dropped,
/// and the formula is run again on the new inputs.
#[test]
fn an_edit_during_an_evaluation_discards_the_stale_result_and_recalculates() {
    let mut s = session(quick());
    s.calc_mut()
        .set_formula("S", &p("c"), "sum(S.a for _ in range(400000)) // 400000")
        .unwrap();
    s.tick();
    assert!(s.evaluator().is_running());
    s.calc_mut().set_value("S", &p("a"), Value::Int(7)).unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(
        get(&s, "c"),
        Some(Value::Int(7)),
        "never the result for a = 1"
    );
}

#[test]
fn undo_and_redo_recalculate_through_the_session() {
    let mut s = session(quick());
    s.calc_mut().set_formula("S", &p("c"), "S.a * 10").unwrap();
    assert!(s.wait_idle(LONG));
    s.calc_mut().set_value("S", &p("a"), Value::Int(5)).unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "c"), Some(Value::Int(50)));
    s.calc_mut().undo().unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "c"), Some(Value::Int(10)));
    s.calc_mut().redo().unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "c"), Some(Value::Int(50)));
}

#[test]
fn many_formulas_are_calculated_in_dependency_order_on_the_thread() {
    let mut s = session(quick());
    for i in (1..6).rev() {
        s.calc_mut()
            .set_formula(
                "S",
                &p(&format!("list[{i}]")),
                &format!("S.list[{}] + 1", i - 1),
            )
            .unwrap();
    }
    s.calc_mut()
        .set_value("S", &p("list[0]"), Value::Int(100))
        .unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "list[5]"), Some(Value::Int(105)));
}

/// A formula that runs past its limit and can be stopped gets `#TIMEOUT!`, and
/// the others are calculated.
#[test]
fn a_formula_past_its_time_limit_is_stopped_and_the_rest_carry_on() {
    let mut s = session(EvalSettings {
        formula_limit: Some(Duration::from_millis(200)),
        grace: Duration::from_secs(10),
        ..quick()
    });
    s.calc_mut()
        .set_formula("S", &p("a"), "sum(1 for _ in iter(int, 1))")
        .unwrap();
    s.calc_mut().set_formula("S", &p("b"), "40 + 2").unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(kind(&s, "a"), Some(ErrorKind::Timeout));
    assert_eq!(get(&s, "b"), Some(Value::Int(42)));
    assert_eq!(
        s.status().abandoned,
        0,
        "it stopped, so nothing was abandoned"
    );
}

/// The recalculation limit: formulas not yet run when it passes get
/// `#TIMEOUT!`.
#[test]
fn a_pass_that_runs_out_of_time_gives_timeout_to_the_formulas_it_did_not_reach() {
    let mut s = session(EvalSettings {
        formula_limit: Some(Duration::from_millis(150)),
        recalc_limit: Some(Duration::from_millis(250)),
        grace: Duration::from_secs(10),
        ..quick()
    });
    for i in 0..6 {
        s.calc_mut()
            .set_formula(
                "S",
                &p(&format!("list[{i}]")),
                "sum(1 for _ in iter(int, 1))",
            )
            .unwrap();
    }
    assert!(s.wait_idle(LONG));
    for i in 0..6 {
        assert_eq!(
            kind(&s, &format!("list[{i}]")),
            Some(ErrorKind::Timeout),
            "{i}"
        );
    }
}

/// A formula that cannot be interrupted is abandoned after the grace period.
/// Its node gets `#TIMEOUT!`, a fresh interpreter takes over, and the old
/// thread is counted until it ends.
#[test]
fn a_formula_that_cannot_be_stopped_is_abandoned_and_a_new_thread_takes_over() {
    let mut s = long_call_session(EvalSettings {
        formula_limit: Some(Duration::from_millis(50)),
        grace: Duration::from_millis(50),
        ..quick()
    });
    s.calc_mut().set_formula("S", &p("a"), LONG_CALL).unwrap();
    s.calc_mut().set_formula("S", &p("b"), "40 + 2").unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(kind(&s, "a"), Some(ErrorKind::Timeout));
    assert_eq!(
        get(&s, "b"),
        Some(Value::Int(42)),
        "the new interpreter ran it"
    );
    // The abandoned thread is still running or has just ended; it ends soon.
    let waited = Instant::now();
    while s.status().abandoned > 0 && waited.elapsed() < LONG {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        s.status().abandoned,
        0,
        "the thread ended and is not counted"
    );
    // And evaluation goes on as usual.
    s.calc_mut().set_formula("S", &p("c"), "S.b + 1").unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "c"), Some(Value::Int(43)));
}

/// At the limit of abandoned threads, evaluation is suspended, and it resumes
/// by itself when they end.
#[test]
fn too_many_abandoned_threads_suspend_evaluation_until_they_end() {
    let mut s = long_call_session(EvalSettings {
        formula_limit: Some(Duration::from_millis(50)),
        grace: Duration::from_millis(50),
        max_abandoned: 1,
        ..quick()
    });
    s.calc_mut().set_formula("S", &p("a"), LONG_CALL).unwrap();
    s.tick();
    let waited = Instant::now();
    while !s.evaluator().suspended() && waited.elapsed() < LONG {
        s.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(s.evaluator().suspended() || s.status().abandoned == 0);
    // Whatever happened, more work is done once the thread has ended.
    s.calc_mut().set_formula("S", &p("b"), "6 * 7").unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "b"), Some(Value::Int(42)));
    assert_eq!(s.status().abandoned, 0);
}

#[test]
fn the_status_describes_what_is_going_on() {
    let mut s = session(quick());
    assert!(s.is_idle());
    s.calc_mut().set_formula("S", &p("c"), "1 + 1").unwrap();
    assert_eq!(s.status().pending, 1);
    assert!(!s.status().running);
    s.tick();
    assert!(s.status().running || s.status().pending == 0);
    assert!(s.wait_idle(LONG));
    let status = s.status();
    assert_eq!(
        (status.pending, status.running, status.abandoned),
        (0, false, 0)
    );
    assert!(!status.suspended);
}

/// New settings reach the engine: the thread that was made with the old ones
/// is let go, and the next evaluation runs on a fresh one.
#[test]
fn a_new_range_limit_takes_effect_on_the_next_evaluation() {
    let mut s = session(quick());
    s.calc_mut()
        .set_formula("S", &p("c"), "len(range(1000))")
        .unwrap();
    assert!(s.wait_idle(LONG));
    assert_eq!(get(&s, "c"), Some(Value::Int(1000)));

    s.evaluator_mut()
        .reconfigure(python::factory(PythonSettings { max_range: 10 }), quick());
    s.calc_mut().recalc_all();
    assert!(s.wait_idle(LONG));
    assert_eq!(kind(&s, "c"), Some(ErrorKind::Value));
}

/// Settings changed while an evaluation runs apply to the one after it.
#[test]
fn settings_changed_while_running_apply_after_the_evaluation_comes_back() {
    let mut s = session(quick());
    s.calc_mut()
        .set_formula(
            "S",
            &p("c"),
            "sum(1 for _ in range(300000)) and len(range(1000))",
        )
        .unwrap();
    s.tick();
    assert!(s.evaluator().is_running());
    s.evaluator_mut()
        .reconfigure(python::factory(PythonSettings { max_range: 10 }), quick());
    assert!(s.wait_idle(LONG));
    // That evaluation was made with the old engine.
    assert_eq!(get(&s, "c"), Some(Value::Int(1000)));
    s.calc_mut().recalc_all();
    assert!(s.wait_idle(LONG));
    assert_eq!(kind(&s, "c"), Some(ErrorKind::Value));
}
