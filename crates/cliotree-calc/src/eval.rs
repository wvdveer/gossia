//! Evaluating the formulas that are out of date, against a snapshot.
//!
//! An evaluation reads only its [`Snapshot`], never the live workbook, so it
//! can run on another thread while the user keeps editing (DESIGN.md,
//! "Evaluation and scheduling"). It produces an [`Evaluation`] tagged with the
//! version it started from, and [`crate::Calc::apply`] decides what is still
//! valid.
//!
//! **Order.** Dependencies are found as formulas run, so there is no order to
//! compute in advance. A formula that reads a node another out-of-date formula
//! is about to compute is run again after that one. A formula's first run is
//! made on the old values and thrown away; to make that rare, formulas are
//! first visited in an order guessed from what they read last time.
//!
//! **Stack.** The chain of formulas waiting for each other is kept in a vector,
//! not on the call stack, so a chain of ten thousand formulas does not
//! overflow it.
//!
//! **Cycles.** A formula that reads a node it is still waiting to compute is
//! in a cycle. Every formula on the cycle gets `#CIRCULAR!`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use cliotree_core::{ErrorKind, ErrorValue, Node, Path, TypeSpec};
use indexmap::IndexSet;

use crate::calc::{ExprJob, ExprOutcome};
use crate::check::{Computed, finish, store};
use crate::engine::{
    Answer, Call, Context, Depth, Engine, Failure, Immediate, ImmediateResult, ModuleReport,
    ModuleSpec, Read, Write,
};
use crate::model::{Dep, Formulas, Loc};
use crate::node_bytes::{self, TabInfo};

/// One tab as the evaluator sees it.
#[derive(Debug, Clone)]
pub(crate) struct SheetSnap {
    pub spec: Arc<TypeSpec>,
    pub data: Node,
    pub formulas: Arc<Formulas>,
    /// What bytes are to the tab: its format and the rest.
    pub info: Arc<TabInfo>,
}

/// A consistent view of the workbook and the list of formulas to run. Cheap to
/// make: it shares the tree and the formulas with the live workbook.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(crate) version: u64,
    pub(crate) modules: Arc<Vec<ModuleSpec>>,
    pub(crate) sheets: HashMap<String, SheetSnap>,
    pub(crate) dirty: Vec<Loc>,
    /// The expressions on controls that are out of date, run after the
    /// formulas so that they read fresh values.
    pub(crate) exprs: Vec<ExprJob>,
}

/// What one formula came to.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub loc: Loc,
    pub computed: Computed,
    pub traceback: Option<String>,
    /// Everything the formula read, found or not.
    pub deps: Vec<Dep>,
}

/// The results of one evaluation, in the order they were finished: a formula
/// comes after those it read.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub(crate) base: u64,
    pub(crate) outcomes: Vec<Outcome>,
    /// What loading the modules came to, if the evaluation loaded any.
    pub(crate) modules: Vec<ModuleReport>,
    pub(crate) exprs: Vec<ExprOutcome>,
}

/// What running a button procedure came to: the writes it made, not yet
/// applied (see [`crate::Calc::apply_procedure`]), or why it failed.
#[derive(Debug, Clone)]
pub struct ProcedureResult {
    pub(crate) base: u64,
    pub(crate) call: Call,
    pub(crate) result: Result<Vec<Write>, Failure>,
}

impl ProcedureResult {
    pub fn call(&self) -> &Call {
        &self.call
    }

    /// A result for a procedure that was given up on.
    pub(crate) fn failed(base: u64, call: Call, failure: Failure) -> ProcedureResult {
        ProcedureResult {
            base,
            call,
            result: Err(failure),
        }
    }
}

impl Evaluation {
    /// The workbook version the evaluation started from.
    pub fn base_version(&self) -> u64 {
        self.base
    }

    pub fn outcomes(&self) -> &[Outcome] {
        &self.outcomes
    }
}

impl Snapshot {
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Runs a button procedure against this snapshot. Nothing it reads is
    /// recorded, and what it writes is returned, not applied.
    pub fn run_call(&self, engine: &dyn Engine, call: &Call) -> ProcedureResult {
        let ctx = self.loose_context(engine, Loc::new(call.sheet.clone(), Path::root()));
        let result = engine.call(call, ctx);
        ProcedureResult {
            base: self.version,
            call: call.clone(),
            result,
        }
    }

    /// Runs an expression of the Immediate window against this snapshot, as a
    /// formula on its node would run. Nothing it reads is recorded.
    pub fn run_immediate(&self, engine: &dyn Engine, immediate: &Immediate) -> ImmediateResult {
        let loc = Loc::new(immediate.sheet.clone(), immediate.path.clone());
        let ctx = self.loose_context(engine, loc);
        let result = engine
            .immediate(&immediate.text, ctx, immediate.time_limit)
            .map_err(|failure| failure_text(&failure));
        ImmediateResult {
            id: immediate.id,
            text: immediate.text.clone(),
            result,
        }
    }

    /// A context at `loc` whose reads are not dependencies of anything, with
    /// the modules loaded first: a fresh engine (after one was abandoned) has
    /// none yet.
    fn loose_context(&self, engine: &dyn Engine, loc: Loc) -> Rc<EvalContext> {
        let tabs: Vec<String> = self.sheets.keys().cloned().collect();
        let _ = engine.sync_modules(&self.modules, &tabs);
        Rc::new(EvalContext {
            loc,
            this_by_value: false,
            data: self
                .sheets
                .iter()
                .map(|(name, s)| (name.clone(), s.data.clone()))
                .collect(),
            specs: self
                .sheets
                .iter()
                .map(|(name, s)| (name.clone(), Arc::clone(&s.spec)))
                .collect(),
            infos: self
                .sheets
                .iter()
                .map(|(name, s)| (name.clone(), Arc::clone(&s.info)))
                .collect(),
            names: self
                .sheets
                .keys()
                .map(|name| (name.clone(), Rc::from(name.as_str())))
                .collect(),
            reads: RefCell::new(Vec::new()),
        })
    }

    /// How many formulas are to be run.
    pub fn pending(&self) -> usize {
        self.dirty.len()
    }

    /// Whether there is anything to run: formulas or expressions on controls.
    pub fn has_work(&self) -> bool {
        !self.dirty.is_empty() || !self.exprs.is_empty()
    }

    /// Runs every formula in the list, and those that depend on them.
    pub fn evaluate(&self, engine: &dyn Engine) -> Evaluation {
        self.evaluate_with(engine, &mut NoHooks)
    }

    /// As [`Snapshot::evaluate`], telling `hooks` before each formula is run
    /// and letting it stop the pass.
    pub fn evaluate_with(&self, engine: &dyn Engine, hooks: &mut dyn Hooks) -> Evaluation {
        // Modules are loaded between evaluations, never during one.
        let tabs: Vec<String> = self.sheets.keys().cloned().collect();
        let modules = engine.sync_modules(&self.modules, &tabs);
        engine.begin_pass();
        let mut run = Run {
            engine,
            hooks,
            sheets: self.sheets.clone(),
            state: HashMap::new(),
            open: 0,
            last_deps: HashMap::new(),
            outcomes: Vec::new(),
        };
        let mut dirty: Vec<Loc> = Vec::new();
        for loc in &self.dirty {
            if run.formula_exists(loc) && run.set(loc, St::Pending) {
                dirty.push(loc.clone());
            }
        }
        for loc in run.visiting_order(&dirty) {
            run.drive(loc);
        }
        let mut exprs = Vec::new();
        for job in &self.exprs {
            // Out of time: the rest stay out of date, for the next pass.
            if !run.hooks.before(&job.loc) {
                break;
            }
            exprs.push(run.expression(job));
        }
        Evaluation {
            base: self.version,
            outcomes: run.outcomes,
            modules,
            exprs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    /// Out of date, not started.
    Pending,
    /// Started, and waiting on something or running. Always on the stack.
    InProgress,
    Done,
}

/// What an evaluation tells whoever is running it, and asks of them.
pub trait Hooks {
    /// A formula is about to be run. Returning false stops the pass: that
    /// formula, and every one after it, gets `#TIMEOUT!` instead of being run.
    fn before(&mut self, loc: &Loc) -> bool;
}

struct NoHooks;

impl Hooks for NoHooks {
    fn before(&mut self, _: &Loc) -> bool {
        true
    }
}

struct Run<'a> {
    engine: &'a dyn Engine,
    hooks: &'a mut dyn Hooks,
    /// The tabs with the results so far written into their trees.
    sheets: HashMap<String, SheetSnap>,
    /// By tab, then path. A formula that is not here is up to date.
    state: HashMap<String, HashMap<Path, St>>,
    /// How many formulas are not yet `Done`.
    open: usize,
    /// What the latest, unfinished run of a formula read.
    last_deps: HashMap<Loc, Vec<Dep>>,
    outcomes: Vec<Outcome>,
}

/// A formula that a run found it has to wait for.
///
/// `certain` says whether the read that found it came before the run had seen
/// any value that is still out of date. Such a read is on the path the formula
/// will really take. A later one is a guess made on old values (the formula
/// may read something else once they are fresh), so it may be ordered but
/// must not by itself make a cycle.
#[derive(Debug, Clone)]
struct Blocker {
    loc: Loc,
    st: St,
    certain: bool,
}

/// A formula on the stack of formulas waiting for each other.
struct Frame {
    loc: Loc,
    /// Whether the formula that pushed it is certain to need it.
    certain: bool,
    /// It has been run at least once and is in the search (see `drive`).
    started: bool,
}

enum Attempt {
    Finished,
    Blocked(Vec<Blocker>),
}

impl Run<'_> {
    fn formula_exists(&self, loc: &Loc) -> bool {
        self.sheets
            .get(&loc.sheet)
            .is_some_and(|s| s.formulas.by_path.contains_key(&loc.path))
    }

    fn st(&self, loc: &Loc) -> Option<St> {
        self.state.get(&loc.sheet)?.get(&loc.path).copied()
    }

    /// Records a state. Returns false if the formula was already known.
    fn set(&mut self, loc: &Loc, st: St) -> bool {
        let tab = self.state.entry(loc.sheet.clone()).or_default();
        let before = tab.insert(loc.path.clone(), st);
        match (before, st) {
            (None, St::Done) | (Some(St::Done), St::Done) => {}
            (None, _) => self.open += 1,
            (Some(St::Done), _) => self.open += 1,
            (Some(_), St::Done) => self.open -= 1,
            _ => {}
        }
        before.is_none()
    }

    /// Formulas that are not yet done and whose result a read of `path` would
    /// include. A deep read includes those at the path, above it, or below it;
    /// a shallow read only those at the path or above it, since what is below
    /// does not change the node's structure.
    fn open_related(&self, sheet: &str, path: &Path, depth: Depth, out: &mut Vec<(Loc, St)>) {
        if self.open == 0 {
            return;
        }
        let (Some(snap), Some(states)) = (self.sheets.get(sheet), self.state.get(sheet)) else {
            return;
        };
        let mut paths = Vec::new();
        match depth {
            Depth::Deep => snap.formulas.index.related(path.segments(), &mut paths),
            Depth::Shallow => snap.formulas.index.at_or_above(path.segments(), &mut paths),
        }
        for p in paths {
            if let Some(st) = states.get(&p).copied().filter(|s| *s != St::Done) {
                out.push((Loc::new(sheet, p), st));
            }
        }
    }

    /// An order to start formulas in, so that most of them find what they read
    /// already computed: each formula after the out-of-date formulas it read
    /// last time. Only a hint; correctness does not depend on it.
    fn visiting_order(&self, dirty: &[Loc]) -> Vec<Loc> {
        let mut seen: HashSet<Loc> = HashSet::new();
        let mut order = Vec::with_capacity(dirty.len());
        for root in dirty {
            if !seen.insert(root.clone()) {
                continue;
            }
            let mut stack = vec![(root.clone(), self.hint_children(root), 0usize)];
            while let Some((loc, kids, next)) = stack.last_mut() {
                if let Some(kid) = kids.get(*next).cloned() {
                    *next += 1;
                    if seen.insert(kid.clone()) {
                        let grandkids = self.hint_children(&kid);
                        stack.push((kid, grandkids, 0));
                    }
                } else {
                    order.push(loc.clone());
                    stack.pop();
                }
            }
        }
        order
    }

    fn hint_children(&self, loc: &Loc) -> Vec<Loc> {
        let Some(formula) = self
            .sheets
            .get(&loc.sheet)
            .and_then(|s| s.formulas.by_path.get(&loc.path))
        else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for dep in &formula.deps {
            let depth = if dep.shallow {
                Depth::Shallow
            } else {
                Depth::Deep
            };
            self.open_related(&dep.loc.sheet, &dep.loc.path, depth, &mut found);
        }
        let mut seen = HashSet::new();
        found
            .into_iter()
            .map(|(l, _)| l)
            .filter(|l| l != loc && seen.insert(l.clone()))
            .collect()
    }

    /// Runs `root` and everything it has to wait for.
    ///
    /// This is a depth-first search over "reads", in the manner of Tarjan's
    /// algorithm for strongly connected components, so that every formula on a
    /// cycle is found whichever formula the search happens to enter it by:
    ///
    /// - A formula that finds everything it read ready finishes.
    /// - One that reads something out of date waits, and the search goes there.
    /// - One that reads a formula that has been entered but not finished is on
    ///   a cycle with it. It cannot finish; the formula at the start of the
    ///   cycle, once everything it waits for has been seen, closes the whole
    ///   group with `#CIRCULAR!`.
    ///
    /// Only reads made before the formula saw any value that is still out of
    /// date are certain (see [`Blocker`]). A formula the search reached only on
    /// a guess is given up again, instead of joining a cycle, if it turns out to
    /// read something that is waiting.
    fn drive(&mut self, root: Loc) {
        if self.st(&root) != Some(St::Pending) {
            return;
        }
        let mut stack = vec![Frame {
            loc: root,
            certain: true,
            started: false,
        }];
        // Discovery number and lowest number reachable, for formulas entered
        // and not yet finished.
        let mut numbers: HashMap<Loc, (usize, usize)> = HashMap::new();
        let mut entered: Vec<Loc> = Vec::new();
        let mut next_number = 0;
        while let Some(top) = stack.last().map(|f| f.loc.clone()) {
            let frame = stack.last_mut().expect("just looked at it");
            if !frame.started {
                if self.st(&top) != Some(St::Pending) {
                    // Finished, or already part of a group waiting to close.
                    stack.pop();
                    continue;
                }
                frame.started = true;
                let number = next_number;
                next_number += 1;
                numbers.insert(top.clone(), (number, number));
                entered.push(top.clone());
                self.set(&top, St::InProgress);
            }
            let blockers = match self.attempt(&top) {
                Attempt::Finished => {
                    // A formula that finishes is nearly always the last one entered.
                    match entered.iter().rposition(|l| l == &top) {
                        Some(at) => {
                            entered.remove(at);
                        }
                        None => unreachable!("a started formula is on the list"),
                    }
                    numbers.remove(&top);
                    stack.pop();
                    continue;
                }
                Attempt::Blocked(blockers) => blockers,
            };
            let (number, mut low) = numbers[&top];
            let mut certain: Vec<Frame> = Vec::new();
            let mut guesses: Vec<Frame> = Vec::new();
            for blocker in blockers {
                match (blocker.st, blocker.certain) {
                    (St::InProgress, true) => {
                        low = low.min(numbers[&blocker.loc].0);
                    }
                    (St::Pending, true) => certain.push(Frame {
                        loc: blocker.loc,
                        certain: true,
                        started: false,
                    }),
                    (St::Pending, false) => guesses.push(Frame {
                        loc: blocker.loc,
                        certain: false,
                        started: false,
                    }),
                    _ => {}
                }
            }
            numbers.insert(top.clone(), (number, low));
            if !certain.is_empty() {
                // Go and see to what it waits for, certain reads first.
                stack.extend(guesses.into_iter().rev());
                stack.extend(certain.into_iter().rev());
                continue;
            }
            let at = entered
                .iter()
                .rposition(|l| l == &top)
                .expect("an entered formula is on the list");
            if low == number {
                // Nothing it waits for is earlier in the search: everything
                // entered since is on a cycle with it.
                let members = entered.split_off(at);
                for member in &members {
                    numbers.remove(member);
                }
                stack.pop();
                self.close_cycle(&members);
            } else if stack.last().is_some_and(|f| f.certain) {
                // It waits for a formula that was entered before it, and is
                // certainly needed by the one that sent the search here.
                stack.pop();
                if let Some(parent) = stack.iter().rev().find(|f| f.started)
                    && let Some(entry) = numbers.get_mut(&parent.loc)
                {
                    entry.1 = entry.1.min(low);
                }
            } else {
                // It was only a guess that this formula is needed. Give it and
                // what was entered after it up, to be run again if it is.
                for member in entered.split_off(at) {
                    numbers.remove(&member);
                    self.set(&member, St::Pending);
                }
                stack.pop();
            }
        }
    }

    /// Runs one formula against the current values. If it read a node that an
    /// out-of-date formula is still to compute, the run is thrown away and the
    /// formulas to wait for are returned.
    fn attempt(&mut self, loc: &Loc) -> Attempt {
        if !self.hooks.before(loc) {
            let error = ErrorValue {
                kind: ErrorKind::Timeout,
                message: "the recalculation ran out of time before this formula was run".to_owned(),
                origin: Some(loc.path.clone()),
            };
            self.record(loc, Computed::Error(error), None, Vec::new());
            return Attempt::Finished;
        }
        let Some(snap) = self.sheets.get(&loc.sheet) else {
            self.set(loc, St::Done);
            return Attempt::Finished;
        };
        let Some(formula) = snap.formulas.by_path.get(&loc.path) else {
            self.set(loc, St::Done);
            return Attempt::Finished;
        };
        let text = formula.text.clone();
        let spec = Arc::clone(&snap.spec);
        let info = Arc::clone(&snap.info);
        let ctx = self.context(loc, false);
        let result = self.engine.eval_answer(&text, ctx.clone());
        let reads = ctx.reads.take();
        drop(ctx);

        // What it read, in order, and which of that is still out of date. A
        // read that finds something out of date is the last that is certain.
        let mut deps: IndexSet<Dep> = IndexSet::new();
        let mut blockers: Vec<Blocker> = Vec::new();
        let mut stale_seen = false;
        let mut seen: HashSet<(&str, &Path, Depth)> = HashSet::with_capacity(reads.len().min(64));
        for (sheet, path, depth) in &reads {
            if !seen.insert((sheet, path, *depth)) {
                continue;
            }
            let dep = Dep {
                loc: Loc::new(&**sheet, path.clone()),
                shallow: *depth == Depth::Shallow,
            };
            let depth = *depth;
            let mut waiting = Vec::new();
            self.open_related(&dep.loc.sheet, &dep.loc.path, depth, &mut waiting);
            if !waiting.is_empty() {
                let certain = !stale_seen;
                blockers.extend(
                    waiting
                        .into_iter()
                        .map(|(loc, st)| Blocker { loc, st, certain }),
                );
                stale_seen = true;
            }
            deps.insert(dep);
        }
        let deps: Vec<Dep> = deps.into_iter().collect();
        if !blockers.is_empty() {
            self.last_deps.insert(loc.clone(), deps);
            let mut seen = HashSet::new();
            let blockers = blockers
                .into_iter()
                .filter(|b| seen.insert(b.loc.clone()))
                .collect();
            return Attempt::Blocked(blockers);
        }
        let result = match result {
            Ok(Answer::Value(value)) => Ok(value),
            Ok(Answer::Bytes(_)) if info.columns.iter().any(|c| c.holds(&loc.path)) => {
                Err(Failure::new(
                    ErrorKind::Type,
                    "a column formula cannot give bytes: give the field its own formula",
                ))
            }
            Ok(Answer::Bytes(bytes)) => {
                let data = &self.sheets[&loc.sheet].data;
                node_bytes::read(&spec, &info, data, &loc.path, &bytes).map_err(|error| Failure {
                    error,
                    traceback: None,
                })
            }
            Err(failure) => Err(failure),
        };
        let (computed, traceback) = finish(&spec, loc, result);
        self.record(loc, computed, traceback, deps);
        Attempt::Finished
    }

    /// A context at `loc` that sees the results so far. `this_by_value` for
    /// an expression of a control.
    fn context(&self, loc: &Loc, this_by_value: bool) -> Rc<EvalContext> {
        Rc::new(EvalContext {
            loc: loc.clone(),
            this_by_value,
            data: self
                .sheets
                .iter()
                .map(|(name, s)| (name.clone(), s.data.clone()))
                .collect(),
            specs: self
                .sheets
                .iter()
                .map(|(name, s)| (name.clone(), Arc::clone(&s.spec)))
                .collect(),
            infos: self
                .sheets
                .iter()
                .map(|(name, s)| (name.clone(), Arc::clone(&s.info)))
                .collect(),
            names: self
                .sheets
                .keys()
                .map(|name| (name.clone(), Rc::from(name.as_str())))
                .collect(),
            reads: RefCell::new(Vec::new()),
        })
    }

    /// Runs an expression of a control against the values as they are once
    /// the formulas have run, and records what it read.
    fn expression(&self, job: &ExprJob) -> ExprOutcome {
        let ctx = self.context(&job.loc, true);
        let result = self.engine.eval(&job.text, ctx.clone());
        let reads = ctx.reads.take();
        drop(ctx);
        let mut deps: IndexSet<Dep> = IndexSet::new();
        for (sheet, path, depth) in reads {
            deps.insert(Dep {
                loc: Loc::new(&*sheet, path),
                shallow: depth == Depth::Shallow,
            });
        }
        let result = match result {
            Ok(cliotree_core::Value::Error(error)) => {
                Err(format!("{}: {}", error.kind, error.message))
            }
            Ok(value) => Ok(value),
            Err(failure) => Err(failure_text(&failure)),
        };
        ExprOutcome {
            key: job.key.clone(),
            text: job.text.clone(),
            result,
            deps: deps.into_iter().collect(),
        }
    }

    /// Stores a finished result in the working tree, so that formulas that read
    /// it later in this run see it, and keeps it for [`crate::Calc::apply`].
    fn record(
        &mut self,
        loc: &Loc,
        mut computed: Computed,
        traceback: Option<String>,
        deps: Vec<Dep>,
    ) {
        if let Some(snap) = self.sheets.get_mut(&loc.sheet)
            && let Err(message) = store(&snap.spec, &mut snap.data, &loc.path, &computed)
            && !matches!(computed, Computed::Error(_))
        {
            // The node cannot exist, for example an item far past the end of
            // its array.
            computed = Computed::Error(ErrorValue {
                kind: ErrorKind::Ref,
                message,
                origin: Some(loc.path.clone()),
            });
            let _ = store(&snap.spec, &mut snap.data, &loc.path, &computed);
        }
        self.set(loc, St::Done);
        self.last_deps.remove(loc);
        self.outcomes.push(Outcome {
            loc: loc.clone(),
            computed,
            traceback,
            deps,
        });
    }

    fn close_cycle(&mut self, members: &[Loc]) {
        let mut names: Vec<String> = members.iter().map(Loc::to_string).collect();
        names.push(names[0].clone());
        let message = format!("circular reference: {}", names.join(" -> "));
        for member in members {
            // Keep what it read, so that editing anything on the cycle, or a
            // value that decides whether it is one, runs it again.
            let deps = self.last_deps.remove(member).unwrap_or_else(|| {
                self.sheets
                    .get(&member.sheet)
                    .and_then(|s| s.formulas.by_path.get(&member.path))
                    .map(|f| f.deps.clone())
                    .unwrap_or_default()
            });
            let error = ErrorValue {
                kind: ErrorKind::Circular,
                message: message.clone(),
                origin: Some(member.path.clone()),
            };
            self.record(member, Computed::Error(error), None, deps);
        }
    }
}

/// What one run of one formula sees: the trees as they were when it started,
/// and a log of what it read. The log is looked at afterwards, so the formula
/// is not held up by the order of evaluation.
struct EvalContext {
    loc: Loc,
    data: HashMap<String, Node>,
    specs: HashMap<String, Arc<TypeSpec>>,
    infos: HashMap<String, Arc<TabInfo>>,
    /// Tab names, shared, so that a read does not have to make a string.
    names: HashMap<String, Rc<str>>,
    reads: RefCell<Vec<(Rc<str>, Path, Depth)>>,
    this_by_value: bool,
}

impl EvalContext {
    fn log(&self, sheet: &str, path: &Path, depth: Depth) {
        let name = match self.names.get(sheet) {
            Some(name) => Rc::clone(name),
            None => Rc::from(sheet),
        };
        self.reads.borrow_mut().push((name, path.clone(), depth));
    }
}

impl Context for EvalContext {
    fn sheet(&self) -> &str {
        &self.loc.sheet
    }

    fn this_by_value(&self) -> bool {
        self.this_by_value
    }

    fn path(&self) -> &Path {
        &self.loc.path
    }

    fn read(&self, sheet: &str, path: &Path, depth: Depth) -> Read {
        self.log(sheet, path, depth);
        match self.data.get(sheet) {
            None => Read::NoSheet,
            Some(root) => match root.get(path) {
                Some(node) => Read::Node(node.clone()),
                None => Read::Missing,
            },
        }
    }

    fn record(&self, sheet: &str, path: &Path, depth: Depth) {
        self.log(sheet, path, depth);
    }

    fn absent_ok(&self, sheet: &str, path: &Path) -> bool {
        self.specs
            .get(sheet)
            .is_some_and(|spec| spec.is_optional_field(path))
    }

    fn sheets(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    fn raw(&self, sheet: &str, path: &Path) -> Result<Vec<u8>, ErrorValue> {
        self.log(sheet, path, Depth::Deep);
        let (Some(data), Some(spec), Some(info)) = (
            self.data.get(sheet),
            self.specs.get(sheet),
            self.infos.get(sheet),
        ) else {
            return Err(ErrorValue {
                kind: ErrorKind::Ref,
                message: format!("there is no tab {sheet}"),
                origin: None,
            });
        };
        node_bytes::raw(spec, info, data, path)
    }
}

/// A failure in words: the error code and the message, and the engine's own
/// account (a traceback) below, if it gave one.
fn failure_text(failure: &Failure) -> String {
    let mut text = format!("{}: {}", failure.error.kind, failure.error.message);
    if let Some(traceback) = failure.traceback.as_deref().filter(|t| !t.is_empty()) {
        text.push('\n');
        text.push_str(traceback.trim_end());
    }
    text
}
