//! The evaluation thread, and a workbook session that uses it (DESIGN.md,
//! "Evaluation and scheduling").
//!
//! All Python runs on a thread of its own, never on the thread that draws the
//! user interface. The interface thread owns a [`Calc`]; when formulas are out
//! of date it hands the evaluation thread a [`Snapshot`], goes on, and takes the
//! results when they arrive with [`Calc::apply`], which drops any that an edit
//! has overtaken. So the interface never waits, and shows the last value of a
//! cell with a pending marker until a new one comes.
//!
//! **Limits.** Each formula has a time limit that the engine enforces, and a
//! whole pass has one too: when it runs out, formulas not yet run get
//! `#TIMEOUT!`. A formula that does not stop (one long call into the
//! interpreter's own code cannot be interrupted) is **abandoned** after a grace
//! period: its node gets `#TIMEOUT!`, the thread is left to finish whenever it
//! does, and a fresh thread with a fresh interpreter takes over. The number of
//! abandoned threads still running is limited; at the limit, evaluation is
//! suspended.

use std::sync::Arc;
use std::time::Duration;

use cliotree_core::Path;
use web_time::Instant;

use crate::calc::Calc;
use crate::engine::{Call, Engine, Immediate, ImmediateResult};

#[cfg(target_family = "wasm")]
pub use crate::session_web::{Evaluator, WorkerLink};
#[cfg(not(target_family = "wasm"))]
pub use thread::Evaluator;

/// Makes an engine. Called on the evaluation thread, because an engine usually
/// owns an interpreter that cannot move between threads.
pub type EngineFactory = Arc<dyn Fn() -> Box<dyn Engine> + Send + Sync>;

/// The limits (DESIGN.md, "Settings and limits").
#[derive(Debug, Clone)]
pub struct EvalSettings {
    /// The longest one formula may run.
    pub formula_limit: Option<Duration>,
    /// The longest one pass may run.
    pub recalc_limit: Option<Duration>,
    /// The longest one button procedure may run.
    pub procedure_limit: Option<Duration>,
    /// How long to wait for a formula to stop after its limit before it is
    /// abandoned.
    pub grace: Duration,
    /// How many abandoned threads may still be running.
    pub max_abandoned: usize,
    /// The stack of an evaluation thread. The interpreter needs a lot.
    pub stack_size: usize,
}

impl Default for EvalSettings {
    fn default() -> Self {
        EvalSettings {
            formula_limit: Some(Duration::from_secs(1)),
            recalc_limit: Some(Duration::from_secs(10)),
            procedure_limit: Some(Duration::from_secs(10)),
            grace: Duration::from_millis(500),
            max_abandoned: 20,
            stack_size: 64 * 1024 * 1024,
        }
    }
}

/// The evaluation thread of the desktop. The wasm version has a Web Worker
/// instead (`session_web.rs`).
#[cfg(not(target_family = "wasm"))]
mod thread {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use cliotree_core::{ErrorKind, ErrorValue};
    use web_time::Instant;

    use super::{EngineFactory, EvalSettings};
    use crate::check::Computed;
    use crate::engine::{Call, Failure, Immediate, ImmediateResult};
    use crate::eval::{Evaluation, Hooks, Outcome, ProcedureResult, Snapshot};
    use crate::model::Loc;

    enum Job {
        Evaluate {
            snapshot: Snapshot,
            deadline: Option<Instant>,
        },
        Call {
            snapshot: Snapshot,
            call: Call,
        },
        Immediate {
            snapshot: Snapshot,
            immediate: Immediate,
        },
    }

    impl Job {
        fn into_snapshot(self) -> Snapshot {
            match self {
                Job::Evaluate { snapshot, .. }
                | Job::Call { snapshot, .. }
                | Job::Immediate { snapshot, .. } => snapshot,
            }
        }
    }

    /// What the evaluation thread sends back.
    enum Done {
        Evaluation(Evaluation),
        Procedure(ProcedureResult),
        Immediate(ImmediateResult),
    }

    /// What the evaluation thread is doing, for the thread that watches it.
    #[derive(Default)]
    struct Progress {
        current: Mutex<Option<(Loc, Instant)>>,
        /// Time the current formula spent waiting for data sources, which does
        /// not count against its limit.
        io: Arc<crate::sources::IoClock>,
    }

    struct PassHooks {
        progress: Arc<Progress>,
        deadline: Option<Instant>,
    }

    impl Hooks for PassHooks {
        fn before(&mut self, loc: &Loc) -> bool {
            if self.deadline.is_some_and(|d| Instant::now() >= d) {
                return false;
            }
            if let Ok(mut current) = self.progress.current.lock() {
                self.progress.io.reset();
                *current = Some((loc.clone(), Instant::now()));
            }
            true
        }
    }

    struct Worker {
        jobs: Sender<Job>,
        results: Receiver<Done>,
        progress: Arc<Progress>,
        /// Set when the worker is given up, so that its thread, when it ends,
        /// counts itself out of the abandoned threads.
        abandoned: Arc<AtomicBool>,
    }

    struct Running {
        version: u64,
        /// The procedure that is running, if it is one.
        call: Option<Call>,
        /// The expression of the Immediate window that is running, if it is one.
        immediate: Option<Immediate>,
    }

    /// The handle the interface thread uses to run evaluations.
    pub struct Evaluator {
        factory: EngineFactory,
        settings: EvalSettings,
        worker: Option<Worker>,
        running: Option<Running>,
        /// How many abandoned threads have not ended yet.
        abandoned: Arc<AtomicUsize>,
        done: VecDeque<Evaluation>,
        finished_calls: VecDeque<ProcedureResult>,
        finished_immediate: VecDeque<ImmediateResult>,
        /// The worker was made with settings that have changed since: it is let
        /// go when what it is running comes back.
        retire: bool,
    }

    impl Evaluator {
        pub fn new(factory: EngineFactory, settings: EvalSettings) -> Evaluator {
            Evaluator {
                factory,
                settings,
                worker: None,
                running: None,
                abandoned: Arc::new(AtomicUsize::new(0)),
                done: VecDeque::new(),
                finished_calls: VecDeque::new(),
                finished_immediate: VecDeque::new(),
                retire: false,
            }
        }

        /// The thread's answer came back: a worker that is to be retired goes.
        fn finished_one(&mut self) {
            self.running = None;
            if self.retire {
                self.retire = false;
                self.worker = None;
            }
        }

        pub fn settings(&self) -> &EvalSettings {
            &self.settings
        }

        /// Changes the limits. They apply to evaluations that start afterwards; the
        /// engine of a running thread keeps the formula limit it has.
        pub fn set_settings(&mut self, settings: EvalSettings) {
            self.settings = settings;
        }

        /// Changes how engines are made and the limits, for everything that
        /// starts afterwards. The thread's engine was made with the old ones, so
        /// the thread is retired: at once if it is idle, otherwise when what it is
        /// running comes back. The next evaluation starts a fresh one, which loads
        /// the modules again.
        pub fn reconfigure(&mut self, factory: EngineFactory, settings: EvalSettings) {
            self.factory = factory;
            self.settings = settings;
            if self.running.is_some() {
                self.retire = true;
            } else {
                self.worker = None;
            }
        }

        /// An evaluation is running.
        pub fn is_running(&self) -> bool {
            self.running.is_some()
        }

        /// Abandoned threads that have not ended.
        pub fn abandoned(&self) -> usize {
            self.abandoned.load(Ordering::SeqCst)
        }

        /// Too many abandoned threads: no new evaluation starts until some end.
        pub fn suspended(&self) -> bool {
            self.abandoned() >= self.settings.max_abandoned
        }

        /// Starts an evaluation of the snapshot. Gives it back if one is already
        /// running or evaluation is suspended.
        pub fn submit(&mut self, snapshot: Snapshot) -> Result<(), Snapshot> {
            if self.running.is_some() || self.suspended() {
                return Err(snapshot);
            }
            if self.worker.is_none() {
                self.worker = Some(self.spawn_worker());
            }
            let worker = self.worker.as_ref().expect("a worker was just made");
            let version = snapshot.version();
            let deadline = self.settings.recalc_limit.map(|l| Instant::now() + l);
            match worker.jobs.send(Job::Evaluate { snapshot, deadline }) {
                Ok(()) => {
                    self.running = Some(Running {
                        version,
                        call: None,
                        immediate: None,
                    });
                    Ok(())
                }
                Err(mpsc::SendError(job)) => {
                    // The thread is gone; the next try makes a new one.
                    self.worker = None;
                    Err(job.into_snapshot())
                }
            }
        }

        /// Starts a button procedure on the snapshot. Gives the snapshot back if
        /// something is running already or evaluation is suspended.
        pub fn submit_call(&mut self, snapshot: Snapshot, call: Call) -> Result<(), Snapshot> {
            if self.running.is_some() || self.suspended() {
                return Err(snapshot);
            }
            if self.worker.is_none() {
                self.worker = Some(self.spawn_worker());
            }
            let worker = self.worker.as_ref().expect("a worker was just made");
            let version = snapshot.version();
            match worker.jobs.send(Job::Call {
                snapshot,
                call: call.clone(),
            }) {
                Ok(()) => {
                    self.running = Some(Running {
                        version,
                        call: Some(call),
                        immediate: None,
                    });
                    Ok(())
                }
                Err(mpsc::SendError(job)) => {
                    self.worker = None;
                    Err(job.into_snapshot())
                }
            }
        }

        /// Starts an expression of the Immediate window on the snapshot. Gives the
        /// snapshot back if something is running already or evaluation is
        /// suspended.
        pub fn submit_immediate(
            &mut self,
            snapshot: Snapshot,
            immediate: Immediate,
        ) -> Result<(), Snapshot> {
            if self.running.is_some() || self.suspended() {
                return Err(snapshot);
            }
            if self.worker.is_none() {
                self.worker = Some(self.spawn_worker());
            }
            let worker = self.worker.as_ref().expect("a worker was just made");
            let version = snapshot.version();
            match worker.jobs.send(Job::Immediate {
                snapshot,
                immediate: immediate.clone(),
            }) {
                Ok(()) => {
                    self.running = Some(Running {
                        version,
                        call: None,
                        immediate: Some(immediate),
                    });
                    Ok(())
                }
                Err(mpsc::SendError(job)) => {
                    self.worker = None;
                    Err(job.into_snapshot())
                }
            }
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
            let mut received = Vec::new();
            if let Some(worker) = &self.worker {
                while let Ok(done) = worker.results.try_recv() {
                    received.push(done);
                }
            }
            for done in received {
                self.take(done);
            }
            self.abandon_if_stuck();
            self.done.drain(..).collect()
        }

        fn take(&mut self, done: Done) {
            self.finished_one();
            match done {
                Done::Evaluation(evaluation) => self.done.push_back(evaluation),
                Done::Procedure(result) => self.finished_calls.push_back(result),
                Done::Immediate(result) => self.finished_immediate.push_back(result),
            }
        }

        /// Waits up to `timeout` for the running evaluation to finish.
        pub fn wait(&mut self, timeout: Duration) {
            if let Some(worker) = &self.worker
                && self.running.is_some()
            {
                match worker.results.recv_timeout(timeout) {
                    Ok(done) => self.take(done),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => self.finished_one(),
                }
            }
        }

        fn abandon_if_stuck(&mut self) {
            let (Some(running), Some(worker)) = (&self.running, &self.worker) else {
                return;
            };
            let limit = match &running.call {
                Some(_) => self.settings.procedure_limit,
                None => self.settings.formula_limit,
            };
            let patience = limit
                .unwrap_or(Duration::from_secs(u64::MAX / 4))
                .saturating_add(self.settings.grace);
            let stuck = match worker.progress.current.lock() {
                Ok(current) => current
                    .as_ref()
                    .filter(|(_, since)| {
                        since.elapsed().saturating_sub(worker.progress.io.waited()) > patience
                    })
                    .map(|(loc, _)| loc.clone()),
                Err(_) => None,
            };
            let Some(loc) = stuck else {
                return;
            };
            let version = running.version;
            let call = running.call.clone();
            let immediate = running.immediate.clone();
            if let Some(worker) = self.worker.take() {
                // The thread ends when it finishes what it is doing and finds that
                // nobody wants the answer.
                worker.abandoned.store(true, Ordering::SeqCst);
                self.abandoned.fetch_add(1, Ordering::SeqCst);
            }
            self.running = None;
            // The next worker is made with the settings as they are now.
            self.retire = false;
            if let Some(immediate) = immediate {
                self.finished_immediate.push_back(ImmediateResult {
                    id: immediate.id,
                    text: immediate.text,
                    result: Err(
                        "#TIMEOUT!: the expression did not stop when its time ran out and was \
                         abandoned"
                            .to_owned(),
                    ),
                });
                return;
            }
            if let Some(call) = call {
                let failure = Failure::new(
                    ErrorKind::Timeout,
                    "the procedure did not stop when its time ran out and was abandoned",
                );
                self.finished_calls
                    .push_back(ProcedureResult::failed(version, call, failure));
                return;
            }
            let error = ErrorValue {
                kind: ErrorKind::Timeout,
                message: "the formula did not stop when its time ran out and was abandoned"
                    .to_owned(),
                origin: Some(loc.path.clone()),
            };
            self.done.push_back(Evaluation {
                base: version,
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

        fn spawn_worker(&self) -> Worker {
            let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
            let (results_tx, results_rx) = mpsc::channel::<Done>();
            let progress = Arc::new(Progress::default());
            let abandoned = Arc::new(AtomicBool::new(false));
            let factory = Arc::clone(&self.factory);
            let settings = self.settings.clone();
            let watched = Arc::clone(&progress);
            let flag = Arc::clone(&abandoned);
            let counter = Arc::clone(&self.abandoned);
            std::thread::Builder::new()
                .name("cliotree-evaluation".to_owned())
                .stack_size(settings.stack_size)
                .spawn(move || {
                    crate::sources::set_io_clock(Arc::clone(&watched.io));
                    let engine = factory();
                    engine.set_time_limit(settings.formula_limit);
                    while let Ok(job) = jobs_rx.recv() {
                        let done = match job {
                            Job::Evaluate { snapshot, deadline } => {
                                let mut hooks = PassHooks {
                                    progress: Arc::clone(&watched),
                                    deadline,
                                };
                                Done::Evaluation(snapshot.evaluate_with(&*engine, &mut hooks))
                            }
                            Job::Call { snapshot, call } => {
                                if let Ok(mut current) = watched.current.lock() {
                                    watched.io.reset();
                                    *current = Some((
                                        Loc::new(call.sheet.clone(), cliotree_core::Path::root()),
                                        Instant::now(),
                                    ));
                                }
                                Done::Procedure(snapshot.run_call(&*engine, &call))
                            }
                            Job::Immediate {
                                snapshot,
                                immediate,
                            } => {
                                if let Ok(mut current) = watched.current.lock() {
                                    watched.io.reset();
                                    *current = Some((
                                        Loc::new(immediate.sheet.clone(), immediate.path.clone()),
                                        Instant::now(),
                                    ));
                                }
                                let done = snapshot.run_immediate(&*engine, &immediate);
                                // The formulas' own limit again.
                                engine.set_time_limit(settings.formula_limit);
                                Done::Immediate(done)
                            }
                        };
                        if let Ok(mut current) = watched.current.lock() {
                            *current = None;
                        }
                        if results_tx.send(done).is_err() {
                            break;
                        }
                    }
                    if flag.load(Ordering::SeqCst) {
                        counter.fetch_sub(1, Ordering::SeqCst);
                    }
                })
                .expect("a thread can be started");
            Worker {
                jobs: jobs_tx,
                results: results_rx,
                progress,
                abandoned,
            }
        }
    }
}

/// What the status bar shows about evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Formulas that are out of date.
    pub pending: usize,
    /// An evaluation is running.
    pub running: bool,
    /// Abandoned threads that have not ended.
    pub abandoned: usize,
    /// Evaluation is suspended because of too many abandoned threads.
    pub suspended: bool,
    /// The label of the button whose procedure is waiting or running.
    pub procedure: Option<String>,
}

/// A button procedure that has finished, and what became of its writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub call: Call,
    /// `Err` is the message to show: the exception, or why a write was
    /// refused. Nothing was changed then.
    pub outcome: Result<(), String>,
}

/// A workbook with its evaluation thread. The interface thread calls
/// [`Session::tick`] once per frame: that takes finished results and starts
/// the next evaluation if formulas are out of date.
pub struct Session {
    calc: Calc,
    evaluator: Evaluator,
    /// A procedure waiting for the workbook to be up to date, so that it
    /// reads what is on the screen.
    queued: Option<Call>,
    finished: Vec<Finished>,
    /// An expression of the Immediate window waiting its turn, as a procedure
    /// does.
    queued_immediate: Option<Immediate>,
    immediate_results: Vec<ImmediateResult>,
    next_immediate: u64,
}

impl Session {
    pub fn new(calc: Calc, evaluator: Evaluator) -> Session {
        Session {
            calc,
            evaluator,
            queued: None,
            finished: Vec::new(),
            queued_immediate: None,
            immediate_results: Vec::new(),
            next_immediate: 0,
        }
    }

    /// Asks for an expression of the Immediate window to run, as a formula on
    /// the node `path` of `sheet` would, under the formula time limit. It
    /// waits its turn as a procedure does; the answer is in
    /// [`Session::take_immediate`]. Gives the number that tells the answer.
    pub fn run_immediate(&mut self, text: &str, sheet: &str, path: &Path) -> Result<u64, String> {
        if !self.calc.is_trusted() {
            return Err(
                "the Immediate window runs Python, so the workbook has to be trusted first"
                    .to_owned(),
            );
        }
        if self.running_immediate().is_some() {
            return Err("an expression is running; wait for it to finish".to_owned());
        }
        self.calc
            .syntax()
            .check(text)
            .map_err(|e| format!("syntax error: {e}"))?;
        self.next_immediate += 1;
        self.queued_immediate = Some(Immediate {
            id: self.next_immediate,
            text: text.trim().to_owned(),
            sheet: sheet.to_owned(),
            path: path.clone(),
            time_limit: self.evaluator.settings().formula_limit,
        });
        Ok(self.next_immediate)
    }

    /// The expression of the Immediate window that is waiting or running.
    pub fn running_immediate(&self) -> Option<&Immediate> {
        self.queued_immediate
            .as_ref()
            .or_else(|| self.evaluator.running_immediate())
    }

    /// Takes the answers of the Immediate window since the last call.
    pub fn take_immediate(&mut self) -> Vec<ImmediateResult> {
        std::mem::take(&mut self.immediate_results)
    }

    /// Asks for a button procedure to run. It waits its turn: evaluation that
    /// is out of date is finished first, then the procedure reads the tabs as
    /// they are. One runs at a time; the answer is in [`Session::take_finished`].
    pub fn run_procedure(&mut self, mut call: Call) -> Result<(), String> {
        if !self.calc.is_trusted() {
            return Err("buttons run Python, so the workbook has to be trusted first".to_owned());
        }
        if self.running_procedure().is_some() {
            return Err("a button procedure is running; wait for it to finish".to_owned());
        }
        call.time_limit = self.evaluator.settings().procedure_limit;
        self.queued = Some(call);
        Ok(())
    }

    /// The procedure that is waiting or running.
    pub fn running_procedure(&self) -> Option<&Call> {
        self.queued
            .as_ref()
            .or_else(|| self.evaluator.running_call())
    }

    /// Takes the procedures that have finished since the last call.
    pub fn take_finished(&mut self) -> Vec<Finished> {
        std::mem::take(&mut self.finished)
    }

    pub fn calc(&self) -> &Calc {
        &self.calc
    }

    /// For edits. Call [`Session::tick`] afterwards to start recalculating.
    pub fn calc_mut(&mut self) -> &mut Calc {
        &mut self.calc
    }

    pub fn evaluator(&self) -> &Evaluator {
        &self.evaluator
    }

    pub fn evaluator_mut(&mut self) -> &mut Evaluator {
        &mut self.evaluator
    }

    /// Takes finished results and starts an evaluation if there is something
    /// to do. Returns whether any result changed the workbook, so that the
    /// interface knows to redraw.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        self.calc.sync_expressions();
        for evaluation in self.evaluator.poll() {
            let applied = self.calc.apply(evaluation);
            changed |= !applied.applied.is_empty() || !applied.discarded.is_empty();
        }
        for result in self.evaluator.take_calls() {
            let call = result.call().clone();
            let outcome = self.calc.apply_procedure(result);
            self.finished.push(Finished { call, outcome });
            changed = true;
        }
        let answers = self.evaluator.take_immediate();
        changed |= !answers.is_empty();
        self.immediate_results.extend(answers);
        let work = self.calc.pending() > 0
            || self.calc.pending_modules() > 0
            || self.calc.pending_expressions() > 0;
        if !self.evaluator.is_running() && !self.evaluator.suspended() {
            if work {
                let _ = self.evaluator.submit(self.calc.snapshot());
            } else if let Some(call) = self.queued.take() {
                if !self.calc.is_trusted() {
                    // Trust was taken back while the procedure waited.
                    let outcome = Err(
                        "the workbook is no longer trusted, so the button did not run".to_owned(),
                    );
                    self.finished.push(Finished { call, outcome });
                    changed = true;
                } else if let Err(_snapshot) = self
                    .evaluator
                    .submit_call(self.calc.snapshot(), call.clone())
                {
                    self.queued = Some(call);
                }
            } else if let Some(immediate) = self.queued_immediate.take() {
                if !self.calc.is_trusted() {
                    self.immediate_results.push(ImmediateResult {
                        id: immediate.id,
                        text: immediate.text,
                        result: Err(
                            "the workbook is no longer trusted, so it did not run".to_owned()
                        ),
                    });
                    changed = true;
                } else if let Err(_snapshot) = self
                    .evaluator
                    .submit_immediate(self.calc.snapshot(), immediate.clone())
                {
                    self.queued_immediate = Some(immediate);
                }
            }
        }
        changed
    }

    pub fn status(&self) -> Status {
        Status {
            pending: self.calc.pending(),
            running: self.evaluator.is_running(),
            abandoned: self.evaluator.abandoned(),
            suspended: self.evaluator.suspended(),
            procedure: self.running_procedure().map(|call| call.button.clone()),
        }
    }

    /// Nothing is out of date and nothing is running.
    pub fn is_idle(&self) -> bool {
        self.calc.pending() == 0
            && self.calc.pending_modules() == 0
            && self.calc.pending_expressions() == 0
            && !self.evaluator.is_running()
            && self.queued.is_none()
            && self.queued_immediate.is_none()
    }

    /// Waits until the workbook is idle, for tests and for saving. Returns
    /// whether it is idle, which it is not if the time ran out first. While
    /// evaluation is suspended it waits for the abandoned threads to end.
    ///
    /// In the wasm version the page cannot wait: the worker's answers arrive
    /// only between frames. There it takes what has come back and says
    /// whether the workbook is idle now.
    #[cfg(target_family = "wasm")]
    pub fn wait_idle(&mut self, _timeout: Duration) -> bool {
        self.tick();
        self.is_idle()
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn wait_idle(&mut self, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            self.tick();
            if self.is_idle() {
                return true;
            }
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                return false;
            }
            self.evaluator.wait(left.min(Duration::from_millis(10)));
            if !self.evaluator.is_running() {
                // Nothing to wait on; give a thread that is ending a moment.
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// Takes the workbook and the evaluation thread apart.
    pub fn into_parts(self) -> (Calc, Evaluator) {
        (self.calc, self.evaluator)
    }
}

/// A session run until a deadline, for Disconnect from Data Sources.
pub struct SessionRunner<'a> {
    pub session: &'a mut Session,
    pub deadline: Instant,
}

impl crate::DisconnectRunner for SessionRunner<'_> {
    fn calc_mut(&mut self) -> &mut Calc {
        self.session.calc_mut()
    }

    fn settle(&mut self) -> bool {
        let left = self.deadline.saturating_duration_since(Instant::now());
        self.session.wait_idle(left)
    }
}
