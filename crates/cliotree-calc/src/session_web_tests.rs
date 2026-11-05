//! Tests of the evaluator of the wasm version, run natively: the worker is a
//! fake link that runs each job at once with the real engine, through the
//! encoding, as the Web Worker does.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cliotree_core::{ErrorKind, Path, json, jsonschema};

use super::{Evaluator, WorkerLink};
use crate::python::{PythonEngine, PythonSettings, PythonSyntax};
use crate::wire::{self, FromWorker, ToWorker};
use crate::{Calc, EvalSettings, Loc};

#[derive(Default)]
struct State {
    starts: usize,
    engine: Option<PythonEngine>,
    configured: usize,
    /// What the worker has sent.
    inbox: Vec<Vec<u8>>,
    /// A worker that takes jobs and never answers, as a stuck one.
    stuck: bool,
}

/// A worker that runs each message as it is posted.
#[derive(Clone, Default)]
struct FakeLink(Rc<RefCell<State>>);

impl WorkerLink for FakeLink {
    fn start(&self) {
        let mut state = self.0.borrow_mut();
        state.starts += 1;
        state.engine = None;
    }

    fn post(&self, message: Vec<u8>) {
        let mut state = self.0.borrow_mut();
        match wire::decode::<ToWorker>(&message).unwrap() {
            ToWorker::Configure(config) => {
                state.engine = Some(config.engine());
                state.configured += 1;
            }
            ToWorker::Job(job) => {
                if state.stuck {
                    let loc = Loc::new("S", "total".parse().unwrap());
                    state.inbox.push(wire::encode(&FromWorker::Started(loc)));
                    return;
                }
                let engine = state
                    .engine
                    .take()
                    .expect("configured before the first job");
                let mut sent = Vec::new();
                let done = wire::run_reporting(&engine, job, &mut |loc| {
                    sent.push(wire::encode(&FromWorker::Started(loc.clone())));
                });
                sent.push(wire::encode(&FromWorker::Done(done)));
                state.inbox.extend(sent);
                state.engine = Some(engine);
            }
        }
    }

    fn take(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.0.borrow_mut().inbox)
    }
}

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {"a": {"type": "integer"}, "total": {"type": "integer"}},
  "required": ["a"]
}"#;

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn calc() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"a": 20}"#).unwrap().node,
    )
    .unwrap();
    calc.add_module("m", "def double(x):\n    return 2 * x\n")
        .unwrap();
    calc.set_formula("S", &p("total"), "double(S.a) + 2")
        .unwrap();
    calc
}

fn settings(limit: Duration) -> EvalSettings {
    EvalSettings {
        formula_limit: Some(limit),
        grace: limit / 4,
        ..EvalSettings::default()
    }
}

/// Runs passes until nothing is out of date, as the session's ticks do.
fn settle(calc: &mut Calc, evaluator: &mut Evaluator) {
    for _ in 0..20 {
        for evaluation in evaluator.poll() {
            calc.apply(evaluation);
        }
        if !evaluator.is_running() {
            let snapshot = calc.snapshot();
            if snapshot.dirty.is_empty() && snapshot.exprs.is_empty() {
                return;
            }
            evaluator.submit(snapshot).unwrap();
        }
    }
    panic!("the workbook did not settle");
}

#[test]
fn the_first_job_starts_and_configures_a_worker_and_the_results_come_back() {
    let link = FakeLink::default();
    let mut evaluator = Evaluator::new(
        Box::new(link.clone()),
        PythonSettings::default(),
        settings(Duration::from_secs(10)),
    );
    let mut calc = calc();
    assert_eq!(link.0.borrow().starts, 0, "no worker before there is work");
    settle(&mut calc, &mut evaluator);
    let cell = calc.cell("S", &p("total")).unwrap();
    assert_eq!(json::to_string(cell.node.unwrap(), false).unwrap(), "42");
    let state = link.0.borrow();
    assert_eq!((state.starts, state.configured), (1, 1));
}

#[test]
fn a_second_job_waits_for_the_first() {
    let link = FakeLink::default();
    link.0.borrow_mut().stuck = true;
    let mut evaluator = Evaluator::new(
        Box::new(link),
        PythonSettings::default(),
        settings(Duration::from_secs(10)),
    );
    let calc = calc();
    evaluator.submit(calc.snapshot()).unwrap();
    assert!(evaluator.submit(calc.snapshot()).is_err());
    assert!(evaluator.is_running());
}

#[test]
fn a_stuck_formula_is_given_up_with_a_fresh_worker() {
    let link = FakeLink::default();
    link.0.borrow_mut().stuck = true;
    let limit = Duration::from_millis(40);
    let mut evaluator = Evaluator::new(
        Box::new(link.clone()),
        PythonSettings::default(),
        settings(limit),
    );
    let mut calc = calc();
    evaluator.submit(calc.snapshot()).unwrap();
    // The worker said the formula started; nothing yet.
    assert!(evaluator.poll().is_empty());
    std::thread::sleep(limit * 2);
    let evaluations = evaluator.poll();
    assert!(!evaluator.is_running());
    assert_eq!(evaluations.len(), 1);
    for evaluation in evaluations {
        calc.apply(evaluation);
    }
    let cell = calc.cell("S", &p("total")).unwrap();
    assert_eq!(cell.error.map(|e| e.kind), Some(ErrorKind::Timeout));
    let state = link.0.borrow();
    assert_eq!(state.starts, 2, "the stuck worker was replaced");
    assert_eq!(state.configured, 2, "each new worker is given the settings");
}

#[test]
fn settings_that_change_while_idle_reach_the_worker_at_once() {
    let link = FakeLink::default();
    let mut evaluator = Evaluator::new(
        Box::new(link.clone()),
        PythonSettings::default(),
        settings(Duration::from_secs(10)),
    );
    let mut calc = calc();
    settle(&mut calc, &mut evaluator);
    evaluator.reconfigure(
        PythonSettings {
            max_range: 10,
            ..PythonSettings::default()
        },
        settings(Duration::from_secs(10)),
    );
    assert_eq!(link.0.borrow().configured, 2);
    calc.set_formula("S", &p("total"), "len(range(100))")
        .unwrap();
    settle(&mut calc, &mut evaluator);
    let cell = calc.cell("S", &p("total")).unwrap();
    assert!(cell.error.is_some(), "the new range limit applies");
}
