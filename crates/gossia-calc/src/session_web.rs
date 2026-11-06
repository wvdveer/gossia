//! The evaluator of the wasm version (DESIGN.md, Web port, 0.3.0): the same
//! methods as the evaluation thread of the desktop, but the jobs go to a Web
//! Worker as bytes (see [`crate::wire`]) and the results come back the same
//! way.
//!
//! The worker is reached through a [`WorkerLink`], which the page's glue
//! provides, so this crate needs no JavaScript. The worker says when each
//! formula starts ([`FromWorker::Started`]); a formula that runs past its
//! limit and the grace is given up by ending the worker and starting a fresh
//! one, which takes the place of abandoning the thread on the desktop. So
//! nothing is ever abandoned here, and evaluation is never suspended.
//!
//! HTTP requests are made by the host: the worker asks for them
//! ([`FromWorker::Fetch`]), whoever runs the page takes them
//! ([`Evaluator::take_fetches`]) and gives back the host's answers
//! ([`Evaluator::answer_fetch`]), and once all have come they go to the
//! worker together. Until then nothing else is started, and the time waited
//! does not count against a formula's limit.

use gossia_i18n::tr;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use gossia_core::{ErrorKind, ErrorValue};
use web_time::Instant;

use crate::check::Computed;
use crate::engine::{Call, Failure, Immediate, ImmediateResult};
use crate::eval::{Evaluation, Outcome, ProcedureResult, Snapshot};
use crate::http::{FetchAnswer, FetchRequest};
use crate::model::Loc;
use crate::python::PythonSettings;
use crate::session::EvalSettings;
use crate::wire::{self, Done, EngineConfig, FromWorker, Job, ToWorker};

/// The page's side of the Web Worker that runs Python.
pub trait WorkerLink {
    /// Ends the worker there is, if any, and starts a fresh one. Messages
    /// posted afterwards go to the new one, which takes them in order once it
    /// has loaded.
    fn start(&self);
    /// Sends a message to the worker.
    fn post(&self, message: Vec<u8>);
    /// The messages the worker has sent since the last call, in order.
    fn take(&self) -> Vec<Vec<u8>>;
}

struct Running {
    version: u64,
    call: Option<Call>,
    immediate: Option<Immediate>,
    /// The formula that is running, as the worker last said, and since when.
    current: Option<(Loc, Instant)>,
}

/// The handle the page uses to run evaluations.
pub struct Evaluator {
    link: Box<dyn WorkerLink>,
    python: PythonSettings,
    settings: EvalSettings,
    /// A worker has been started and given the engine's settings.
    started: bool,
    running: Option<Running>,
    done: VecDeque<Evaluation>,
    finished_calls: VecDeque<ProcedureResult>,
    finished_immediate: VecDeque<ImmediateResult>,
    /// The settings changed while something was running: the worker is given
    /// the new ones when it comes back.
    reconfigure: bool,
    /// Requests for the host, with ids of the page's own.
    to_host: Vec<FetchRequest>,
    /// The requests the host has still to answer: the page's id, and the
    /// worker's.
    outstanding: HashMap<u64, u64>,
    /// The answers that have come, by the worker's ids.
    answers: Vec<(u64, FetchAnswer)>,
    next_fetch: u64,
}

impl Evaluator {
    pub fn new(
        link: Box<dyn WorkerLink>,
        python: PythonSettings,
        settings: EvalSettings,
    ) -> Evaluator {
        Evaluator {
            link,
            python,
            settings,
            started: false,
            running: None,
            done: VecDeque::new(),
            finished_calls: VecDeque::new(),
            finished_immediate: VecDeque::new(),
            reconfigure: false,
            to_host: Vec::new(),
            outstanding: HashMap::new(),
            answers: Vec::new(),
            next_fetch: 0,
        }
    }

    /// The requests the worker wants the host to make, each with an id to
    /// give its answer back with.
    pub fn take_fetches(&mut self) -> Vec<FetchRequest> {
        std::mem::take(&mut self.to_host)
    }

    /// The host's answer to a request. Once every request has its answer,
    /// they go to the worker. An answer nobody waits for any more (the
    /// worker was started again since) is dropped.
    pub fn answer_fetch(&mut self, id: u64, answer: FetchAnswer) {
        let Some(worker_id) = self.outstanding.remove(&id) else {
            return;
        };
        self.answers.push((worker_id, answer));
        if self.outstanding.is_empty() {
            let answers = std::mem::take(&mut self.answers);
            self.link.post(wire::encode(&ToWorker::Answers(answers)));
        }
    }

    /// Forgets the requests of a worker that is gone or made again.
    fn forget_fetches(&mut self) {
        self.to_host.clear();
        self.outstanding.clear();
        self.answers.clear();
    }

    pub fn settings(&self) -> &EvalSettings {
        &self.settings
    }

    /// Changes the limits, for evaluations that start afterwards.
    pub fn set_settings(&mut self, settings: EvalSettings) {
        self.settings = settings;
    }

    /// Changes the engine's settings and the limits. The worker gets them
    /// at once if it is idle, otherwise when what it runs comes back; its
    /// engine is made again, so the modules load again with the next job.
    pub fn reconfigure(&mut self, python: PythonSettings, settings: EvalSettings) {
        self.python = python;
        self.settings = settings;
        if self.running.is_some() {
            self.reconfigure = true;
        } else if self.started {
            self.configure();
        }
    }

    /// Gives the worker the settings, which makes its engine again: the
    /// answers it waited for are no longer wanted (formulas that waited ask
    /// again).
    fn configure(&mut self) {
        self.forget_fetches();
        let config = EngineConfig::new(&self.python, self.settings.formula_limit);
        self.link.post(wire::encode(&ToWorker::Configure(config)));
    }

    /// Something is running, or waits for the host's answers.
    pub fn is_running(&self) -> bool {
        self.running.is_some() || !self.outstanding.is_empty()
    }

    /// Nothing is ever abandoned here: a stuck worker is ended.
    pub fn abandoned(&self) -> usize {
        0
    }

    /// Evaluation is never suspended here.
    pub fn suspended(&self) -> bool {
        false
    }

    fn send(&mut self, job: Job, call: Option<Call>, immediate: Option<Immediate>) {
        if !self.started {
            self.link.start();
            self.configure();
            self.started = true;
        }
        let version = job.version();
        self.link.post(wire::encode(&ToWorker::Job(job)));
        self.running = Some(Running {
            version,
            call,
            immediate,
            current: None,
        });
    }

    /// Starts an evaluation of the snapshot. Gives it back if one is already
    /// running.
    pub fn submit(&mut self, snapshot: Snapshot) -> Result<(), Snapshot> {
        if self.is_running() {
            return Err(snapshot);
        }
        let recalc_limit = self.settings.recalc_limit;
        self.send(
            Job::Evaluate {
                snapshot,
                recalc_limit,
            },
            None,
            None,
        );
        Ok(())
    }

    /// Starts a button procedure on the snapshot. Gives the snapshot back if
    /// something is running already.
    pub fn submit_call(&mut self, snapshot: Snapshot, call: Call) -> Result<(), Snapshot> {
        if self.is_running() {
            return Err(snapshot);
        }
        let job = Job::Call {
            snapshot,
            call: call.clone(),
        };
        self.send(job, Some(call), None);
        Ok(())
    }

    /// Starts an expression of the Immediate window on the snapshot. Gives the
    /// snapshot back if something is running already.
    pub fn submit_immediate(
        &mut self,
        snapshot: Snapshot,
        immediate: Immediate,
    ) -> Result<(), Snapshot> {
        if self.is_running() {
            return Err(snapshot);
        }
        let job = Job::Immediate {
            snapshot,
            immediate: immediate.clone(),
        };
        self.send(job, None, Some(immediate));
        Ok(())
    }

    /// The expression of the Immediate window that is running, if one is.
    pub fn running_immediate(&self) -> Option<&Immediate> {
        self.running.as_ref().and_then(|r| r.immediate.as_ref())
    }

    /// Takes the expressions of the Immediate window that have finished.
    pub fn take_immediate(&mut self) -> Vec<ImmediateResult> {
        self.finished_immediate.drain(..).collect()
    }

    /// The procedure that is running, if one is.
    pub fn running_call(&self) -> Option<&Call> {
        self.running.as_ref().and_then(|r| r.call.as_ref())
    }

    /// Takes the procedures that have finished.
    pub fn take_calls(&mut self) -> Vec<ProcedureResult> {
        self.finished_calls.drain(..).collect()
    }

    /// Takes the evaluations that have finished, and gives up on a formula
    /// that has run too long to be stopped.
    pub fn poll(&mut self) -> Vec<Evaluation> {
        for message in self.link.take() {
            match wire::decode::<FromWorker>(&message) {
                Ok(FromWorker::Started(loc)) => {
                    if let Some(running) = &mut self.running {
                        running.current = Some((loc, Instant::now()));
                    }
                }
                Ok(FromWorker::Done(done)) => self.take(done),
                // The page's store is the user's: whoever keeps it (the
                // host) sees that it changed.
                Ok(FromWorker::Sources(defs)) => {
                    let _ = self.python.sources.replace_all(defs);
                }
                Ok(FromWorker::Fetch(requests)) => {
                    // Waiting for the host is not running: no formula is
                    // stuck meanwhile.
                    if let Some(running) = &mut self.running {
                        running.current = None;
                    }
                    for mut request in requests {
                        self.next_fetch += 1;
                        self.outstanding.insert(self.next_fetch, request.id);
                        request.id = self.next_fetch;
                        self.to_host.push(request);
                    }
                }
                // A message that does not read is a bug in one side or the
                // other; the job it belonged to times out like a stuck one.
                Err(_) => {}
            }
        }
        self.give_up_if_stuck();
        self.done.drain(..).collect()
    }

    fn take(&mut self, done: Done) {
        self.running = None;
        if self.reconfigure {
            self.reconfigure = false;
            self.configure();
        }
        match done {
            Done::Evaluation(evaluation) => self.done.push_back(evaluation),
            Done::Procedure(result) => self.finished_calls.push_back(result),
            Done::Immediate(result) => self.finished_immediate.push_back(result),
        }
    }

    /// The page cannot block while the worker runs, so this only takes what
    /// has come back.
    pub fn wait(&mut self, _timeout: Duration) {
        let done = self.poll();
        self.done.extend(done);
    }

    fn give_up_if_stuck(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let limit = match &running.call {
            Some(_) => self.settings.procedure_limit,
            None => self.settings.formula_limit,
        };
        let patience = limit
            .unwrap_or(Duration::from_secs(u64::MAX / 4))
            .saturating_add(self.settings.grace);
        let stuck = running
            .current
            .as_ref()
            .filter(|(_, since)| since.elapsed() > patience)
            .map(|(loc, _)| loc.clone());
        let Some(loc) = stuck else {
            return;
        };
        let Some(running) = self.running.take() else {
            return;
        };
        // A fresh worker, with the settings as they are now.
        self.reconfigure = false;
        self.link.start();
        self.configure();
        if let Some(immediate) = running.immediate {
            self.finished_immediate.push_back(ImmediateResult {
                id: immediate.id,
                text: immediate.text,
                result: Err(tr!(
                    "#TIMEOUT!: the expression did not stop when its time ran out and was \
                     ended"
                )
                .to_owned()),
            });
            return;
        }
        if let Some(call) = running.call {
            let failure = Failure::new(
                ErrorKind::Timeout,
                tr!("the procedure did not stop when its time ran out and was ended"),
            );
            self.finished_calls
                .push_back(ProcedureResult::failed(running.version, call, failure));
            return;
        }
        let error = ErrorValue {
            kind: ErrorKind::Timeout,
            message: tr!("the formula did not stop when its time ran out and was ended").to_owned(),
            origin: Some(loc.path.clone()),
        };
        self.done.push_back(Evaluation {
            base: running.version,
            outcomes: vec![Outcome {
                loc,
                computed: Computed::Error(error),
                traceback: None,
                deps: Vec::new(),
                rejected: None,
            }],
            modules: Vec::new(),
            exprs: Vec::new(),
        });
    }
}

#[cfg(test)]
#[path = "session_web_tests.rs"]
mod session_web_tests;
