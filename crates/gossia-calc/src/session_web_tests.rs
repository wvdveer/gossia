//! Tests of the evaluator of the wasm version, run natively: the worker is a
//! fake link that runs each job at once with the real engine, through the
//! encoding, as the Web Worker does.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gossia_core::{ErrorKind, Path, json, jsonschema};

use super::{Evaluator, WorkerLink};
use crate::http::{FetchAnswer, FetchRequest, HttpResponse};
use crate::python::{PythonSettings, PythonSyntax};
use crate::wire::{self, FromWorker, ToWorker};
use crate::{Calc, EvalSettings, Loc};

#[derive(Default)]
struct State {
    starts: usize,
    worker: Option<wire::Worker>,
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
        state.worker = None;
    }

    fn post(&self, message: Vec<u8>) {
        let mut state = self.0.borrow_mut();
        match wire::decode::<ToWorker>(&message).unwrap() {
            ToWorker::Configure(config) => {
                let mut worker = wire::Worker::default();
                worker.handle(ToWorker::Configure(config), &mut |_| {});
                state.worker = Some(worker);
                state.configured += 1;
            }
            message => {
                if state.stuck {
                    let loc = Loc::new("S", "total".parse().unwrap());
                    state.inbox.push(wire::encode(&FromWorker::Started(loc)));
                    return;
                }
                let mut worker = state
                    .worker
                    .take()
                    .expect("configured before the first job");
                let mut sent = Vec::new();
                worker.handle(message, &mut |reply| {
                    sent.push(wire::encode(&reply));
                });
                state.inbox.extend(sent);
                state.worker = Some(worker);
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

#[test]
fn a_source_defined_in_the_worker_comes_back_to_the_pages_store() {
    let link = FakeLink::default();
    let store = crate::sources::SourceStore::in_memory();
    let mut evaluator = Evaluator::new(
        Box::new(link.clone()),
        PythonSettings {
            sources: store.clone(),
            ..PythonSettings::default()
        },
        settings(Duration::from_secs(10)),
    );
    let calc = calc();
    let immediate = crate::engine::Immediate {
        id: 1,
        text: "sources.define('Api', 'http', url='https://example.com/v1')".to_owned(),
        sheet: "S".to_owned(),
        path: Path::default(),
        time_limit: None,
    };
    evaluator
        .submit_immediate(calc.snapshot(), immediate)
        .unwrap();
    evaluator.poll();
    let results = evaluator.take_immediate();
    assert_eq!(results.len(), 1);
    assert!(results[0].result.is_ok(), "{:?}", results[0].result);
    let defs = store.list().unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "Api");
    assert_eq!(store.version(), 1, "one change, for the host to keep");
}

/// A store with one HTTP source, `Api`, that allows writes.
fn api_store() -> crate::sources::SourceStore {
    let store = crate::sources::SourceStore::in_memory();
    let settings = serde_json::json!({"url": "https://example.invalid/v1", "writes": true});
    let def =
        crate::sources::SourceDef::from_settings("Api", "http", settings.as_object().unwrap())
            .unwrap();
    store.save(def, None, false).unwrap();
    store
}

fn evaluator_with(store: &crate::sources::SourceStore, link: &FakeLink) -> Evaluator {
    Evaluator::new(
        Box::new(link.clone()),
        PythonSettings {
            sources: store.clone(),
            ..PythonSettings::default()
        },
        settings(Duration::from_secs(10)),
    )
}

/// The host: answers `GET /v1/<name>` with `{"<name>": <number>}` and a
/// `POST` with what it was sent, and keeps every request it was asked.
fn host(asked: &mut Vec<FetchRequest>, request: FetchRequest) -> FetchAnswer {
    let path = request
        .url
        .trim_start_matches("https://example.invalid/v1/");
    let body = match request.method.as_str() {
        "GET" => match path {
            "n" => r#"{"n": 30}"#.to_owned(),
            "m" => r#"{"m": 12}"#.to_owned(),
            _ => return Err(format!("{} cannot be reached", request.url)),
        },
        _ => String::from_utf8(request.body.clone().unwrap_or_default()).unwrap(),
    };
    asked.push(request);
    Ok(HttpResponse {
        status: 200,
        reason: "OK".to_owned(),
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body: body.into_bytes(),
    })
}

/// As `settle`, with the host answering the requests the worker asks for.
fn settle_with_host(
    calc: &mut Calc,
    evaluator: &mut Evaluator,
    asked: &mut Vec<FetchRequest>,
) -> usize {
    for round in 0..20 {
        for evaluation in evaluator.poll() {
            calc.apply(evaluation);
        }
        for request in evaluator.take_fetches() {
            let id = request.id;
            let answer = host(asked, request);
            evaluator.answer_fetch(id, answer);
        }
        if !evaluator.is_running() {
            let snapshot = calc.snapshot();
            if snapshot.dirty.is_empty() && snapshot.exprs.is_empty() {
                return round;
            }
            evaluator.submit(snapshot).unwrap();
        }
    }
    panic!("the workbook did not settle");
}

#[test]
fn a_formula_waits_for_the_hosts_answers_and_runs_again_with_them() {
    let link = FakeLink::default();
    let store = api_store();
    let mut evaluator = evaluator_with(&store, &link);
    let mut calc = calc();
    calc.set_formula(
        "S",
        &p("total"),
        "source('Api').get('n').json()['n'] + source('Api').get('m').json()['m']",
    )
    .unwrap();
    // A formula that reads it waits with it.
    calc.set_formula("S", &p("a"), "S.total - 1").unwrap();
    let mut asked = Vec::new();
    settle_with_host(&mut calc, &mut evaluator, &mut asked);
    let total = calc.cell("S", &p("total")).unwrap();
    assert_eq!(json::to_string(total.node.unwrap(), false).unwrap(), "42");
    let a = calc.cell("S", &p("a")).unwrap();
    assert_eq!(json::to_string(a.node.unwrap(), false).unwrap(), "41");
    let urls: Vec<&str> = asked.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://example.invalid/v1/n",
            "https://example.invalid/v1/m"
        ],
        "each read asked once, one after the other"
    );
    // The next pass reads afresh.
    calc.recalc_all();
    settle_with_host(&mut calc, &mut evaluator, &mut asked);
    assert_eq!(asked.len(), 4);
}

#[test]
fn a_failed_request_is_source_error_in_the_formula() {
    let link = FakeLink::default();
    let store = api_store();
    let mut evaluator = evaluator_with(&store, &link);
    let mut calc = calc();
    calc.set_formula("S", &p("total"), "source('Api').get('gone').json()['n']")
        .unwrap();
    settle_with_host(&mut calc, &mut evaluator, &mut Vec::new());
    let total = calc.cell("S", &p("total")).unwrap();
    let error = total.error.unwrap();
    assert_eq!(error.kind, ErrorKind::Source);
    assert!(
        error.message.contains("cannot be reached"),
        "{}",
        error.message
    );
}

#[test]
fn an_immediate_expression_waits_in_the_worker_and_sends_a_write_once() {
    let link = FakeLink::default();
    let store = api_store();
    let mut evaluator = evaluator_with(&store, &link);
    let calc = calc();
    let immediate = crate::engine::Immediate {
        id: 7,
        text: "(source('Api').post('orders', json={'id': 1}).json(), \
               source('Api').get('n').json())"
            .to_owned(),
        sheet: "S".to_owned(),
        path: Path::default(),
        time_limit: None,
    };
    evaluator
        .submit_immediate(calc.snapshot(), immediate)
        .unwrap();
    let mut asked = Vec::new();
    for _ in 0..10 {
        evaluator.poll();
        for request in evaluator.take_fetches() {
            let id = request.id;
            let answer = host(&mut asked, request);
            evaluator.answer_fetch(id, answer);
        }
        if !evaluator.is_running() {
            break;
        }
    }
    let results = evaluator.take_immediate();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].result.as_deref(),
        Ok("[{'id': 1}, {'n': 30}]"),
        "{:?}",
        results[0].result
    );
    let methods: Vec<&str> = asked.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["POST", "GET"], "the POST was sent once");
}
