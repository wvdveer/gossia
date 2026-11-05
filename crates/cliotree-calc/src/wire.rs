//! The worker protocol (DESIGN.md, Web port, 0.3.0): the jobs and results of
//! the evaluation thread, encoded as bytes so that the wasm version can send
//! them to the Web Worker that runs Python and back.
//!
//! A job carries a whole [`Snapshot`], and a result is what the evaluation
//! thread gives back on the desktop, so the page applies it with the same
//! code ([`crate::Calc::apply`] and the rest). Nodes that are read back get
//! fresh ids (see `cliotree_core::Node`). The encoding is MessagePack with
//! field names, so that fields a type leaves out when it is saved as JSON do
//! not throw the reading out of step.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use web_time::Instant;

use crate::engine::{Call, Engine, Immediate, ImmediateResult};
use crate::eval::{Evaluation, Hooks, ProcedureResult, Snapshot};
use crate::model::Loc;
use crate::python::{PythonEngine, PythonSettings};
use crate::sources::{SourceDef, SourceStore};

/// A message from the page to the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToWorker {
    /// Make the engine with these settings. Sent first to every new worker,
    /// and again when the settings change; the modules load again with the
    /// next job.
    Configure(EngineConfig),
    Job(Job),
}

/// A message from the worker to the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FromWorker {
    /// A formula, or a procedure or Immediate expression on its tab, is about
    /// to run. Sent before it runs, so it reaches the page even while the
    /// worker is busy; the page uses it to see that a formula is stuck.
    Started(Loc),
    Done(Done),
}

/// The settings an engine is made with, as the worker receives them: the
/// user's settings of the engine and the formula time limit. The data source
/// definitions travel as a list and are kept in memory in the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub max_range: i64,
    pub sources: Vec<SourceDef>,
    pub source_timeout: Duration,
    pub max_response: u64,
    pub formula_limit: Option<Duration>,
}

impl EngineConfig {
    pub fn new(settings: &PythonSettings, formula_limit: Option<Duration>) -> EngineConfig {
        EngineConfig {
            max_range: settings.max_range,
            sources: settings.sources.list().unwrap_or_default(),
            source_timeout: settings.source_timeout,
            max_response: settings.max_response,
            formula_limit,
        }
    }

    /// The engine these settings describe.
    pub fn engine(&self) -> PythonEngine {
        let sources = SourceStore::in_memory();
        for def in &self.sources {
            // Definitions that were saved once are valid.
            let _ = sources.save(def.clone(), None, true);
        }
        let engine = PythonEngine::with_settings(PythonSettings {
            max_range: self.max_range,
            sources,
            source_timeout: self.source_timeout,
            max_response: self.max_response,
        });
        engine.set_time_limit(self.formula_limit);
        engine
    }
}

/// What the page asks the worker to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Job {
    /// Bring the formulas and the expressions of the snapshot up to date,
    /// within `recalc_limit` for the whole pass.
    Evaluate {
        snapshot: Snapshot,
        recalc_limit: Option<Duration>,
    },
    /// Run a button procedure.
    Call { snapshot: Snapshot, call: Call },
    /// Run an expression of the Immediate window.
    Immediate {
        snapshot: Snapshot,
        immediate: Immediate,
    },
}

impl Job {
    /// The version of the workbook the job was made from.
    pub fn version(&self) -> u64 {
        match self {
            Job::Evaluate { snapshot, .. }
            | Job::Call { snapshot, .. }
            | Job::Immediate { snapshot, .. } => snapshot.version(),
        }
    }
}

/// What the worker gives back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Done {
    Evaluation(Evaluation),
    Procedure(ProcedureResult),
    Immediate(ImmediateResult),
}

/// Runs a job, as the evaluation thread does on the desktop. The formulas'
/// time limit is the engine's, set by whoever made it.
pub fn run(engine: &dyn Engine, job: Job) -> Done {
    run_reporting(engine, job, &mut |_| {})
}

/// Runs a job and calls `started` before each formula, and before a
/// procedure or an Immediate expression, with where it runs.
pub fn run_reporting(engine: &dyn Engine, job: Job, started: &mut dyn FnMut(&Loc)) -> Done {
    match job {
        Job::Evaluate {
            snapshot,
            recalc_limit,
        } => {
            let mut hooks = Pass {
                deadline: recalc_limit.map(|l| Instant::now() + l),
                started,
            };
            Done::Evaluation(snapshot.evaluate_with(engine, &mut hooks))
        }
        Job::Call { snapshot, call } => {
            started(&Loc::new(call.sheet.clone(), cliotree_core::Path::root()));
            Done::Procedure(snapshot.run_call(engine, &call))
        }
        Job::Immediate {
            snapshot,
            immediate,
        } => {
            started(&Loc::new(immediate.sheet.clone(), immediate.path.clone()));
            let done = snapshot.run_immediate(engine, &immediate);
            Done::Immediate(done)
        }
    }
}

/// Says when each formula starts, and stops a pass that has run past its
/// limit: what is left gets `#TIMEOUT!`.
struct Pass<'a> {
    deadline: Option<Instant>,
    started: &'a mut dyn FnMut(&Loc),
}

impl Hooks for Pass<'_> {
    fn before(&mut self, loc: &Loc) -> bool {
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return false;
        }
        (self.started)(loc);
        true
    }
}

/// Encodes a job or a result.
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    rmp_serde::to_vec_named(value).expect("the jobs and results of the evaluation can be encoded")
}

/// Decodes a job or a result.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    rmp_serde::from_slice(bytes)
        .map_err(|e| format!("a message to or from the worker did not read: {e}"))
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod wire_tests;
