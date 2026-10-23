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

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cliotree_core::{ErrorKind, ErrorValue};

use crate::calc::Calc;
use crate::check::Computed;
use crate::engine::Engine;
use crate::eval::{Evaluation, Hooks, Outcome, Snapshot};
use crate::model::Loc;

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
            grace: Duration::from_millis(500),
            max_abandoned: 20,
            stack_size: 64 * 1024 * 1024,
        }
    }
}

struct Job {
    snapshot: Snapshot,
    deadline: Option<Instant>,
}

/// What the evaluation thread is doing, for the thread that watches it.
#[derive(Default)]
struct Progress {
    current: Mutex<Option<(Loc, Instant)>>,
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
            *current = Some((loc.clone(), Instant::now()));
        }
        true
    }
}

struct Worker {
    jobs: Sender<Job>,
    results: Receiver<Evaluation>,
    progress: Arc<Progress>,
    /// Set when the worker is given up, so that its thread, when it ends,
    /// counts itself out of the abandoned threads.
    abandoned: Arc<AtomicBool>,
}

struct Running {
    version: u64,
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
        match worker.jobs.send(Job { snapshot, deadline }) {
            Ok(()) => {
                self.running = Some(Running { version });
                Ok(())
            }
            Err(mpsc::SendError(job)) => {
                // The thread is gone; the next try makes a new one.
                self.worker = None;
                Err(job.snapshot)
            }
        }
    }

    /// Takes the evaluations that have finished, and gives up on a formula
    /// that has run too long to be stopped.
    pub fn poll(&mut self) -> Vec<Evaluation> {
        if let Some(worker) = &self.worker {
            while let Ok(evaluation) = worker.results.try_recv() {
                self.running = None;
                self.done.push_back(evaluation);
            }
        }
        self.abandon_if_stuck();
        self.done.drain(..).collect()
    }

    /// Waits up to `timeout` for the running evaluation to finish.
    pub fn wait(&mut self, timeout: Duration) {
        if let Some(worker) = &self.worker
            && self.running.is_some()
        {
            match worker.results.recv_timeout(timeout) {
                Ok(evaluation) => {
                    self.running = None;
                    self.done.push_back(evaluation);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => self.running = None,
            }
        }
    }

    fn abandon_if_stuck(&mut self) {
        let (Some(running), Some(worker)) = (&self.running, &self.worker) else {
            return;
        };
        let patience = self
            .settings
            .formula_limit
            .unwrap_or(Duration::from_secs(u64::MAX / 4))
            .saturating_add(self.settings.grace);
        let stuck = match worker.progress.current.lock() {
            Ok(current) => current
                .as_ref()
                .filter(|(_, since)| since.elapsed() > patience)
                .map(|(loc, _)| loc.clone()),
            Err(_) => None,
        };
        let Some(loc) = stuck else {
            return;
        };
        let version = running.version;
        if let Some(worker) = self.worker.take() {
            // The thread ends when it finishes what it is doing and finds that
            // nobody wants the answer.
            worker.abandoned.store(true, Ordering::SeqCst);
            self.abandoned.fetch_add(1, Ordering::SeqCst);
        }
        self.running = None;
        let error = ErrorValue {
            kind: ErrorKind::Timeout,
            message: "the formula did not stop when its time ran out and was abandoned".to_owned(),
            origin: Some(loc.path.clone()),
        };
        self.done.push_back(Evaluation {
            base: version,
            outcomes: vec![Outcome {
                loc,
                computed: Computed::Error(error),
                traceback: None,
                deps: Vec::new(),
            }],
        });
    }

    fn spawn_worker(&self) -> Worker {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (results_tx, results_rx) = mpsc::channel::<Evaluation>();
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
                let engine = factory();
                engine.set_time_limit(settings.formula_limit);
                while let Ok(job) = jobs_rx.recv() {
                    let mut hooks = PassHooks {
                        progress: Arc::clone(&watched),
                        deadline: job.deadline,
                    };
                    let evaluation = job.snapshot.evaluate_with(&*engine, &mut hooks);
                    if let Ok(mut current) = watched.current.lock() {
                        *current = None;
                    }
                    if results_tx.send(evaluation).is_err() {
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
}

/// A workbook with its evaluation thread. The interface thread calls
/// [`Session::tick`] once per frame: that takes finished results and starts
/// the next evaluation if formulas are out of date.
pub struct Session {
    calc: Calc,
    evaluator: Evaluator,
}

impl Session {
    pub fn new(calc: Calc, evaluator: Evaluator) -> Session {
        Session { calc, evaluator }
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
        for evaluation in self.evaluator.poll() {
            let applied = self.calc.apply(evaluation);
            changed |= !applied.applied.is_empty() || !applied.discarded.is_empty();
        }
        if !self.evaluator.is_running() && self.calc.pending() > 0 && !self.evaluator.suspended() {
            let _ = self.evaluator.submit(self.calc.snapshot());
        }
        changed
    }

    pub fn status(&self) -> Status {
        Status {
            pending: self.calc.pending(),
            running: self.evaluator.is_running(),
            abandoned: self.evaluator.abandoned(),
            suspended: self.evaluator.suspended(),
        }
    }

    /// Nothing is out of date and nothing is running.
    pub fn is_idle(&self) -> bool {
        self.calc.pending() == 0 && !self.evaluator.is_running()
    }

    /// Waits until the workbook is idle, for tests and for saving. Returns
    /// whether it is idle, which it is not if the time ran out first. While
    /// evaluation is suspended it waits for the abandoned threads to end.
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
