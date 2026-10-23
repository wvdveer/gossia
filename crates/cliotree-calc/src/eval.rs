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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cliotree_core::{ErrorKind, ErrorValue, Node, Path, TypeSpec};
use indexmap::IndexSet;

use crate::check::{Computed, finish, store};
use crate::engine::{Context, Engine, Read};
use crate::model::{Formulas, Loc};

/// One tab as the evaluator sees it.
#[derive(Debug, Clone)]
pub(crate) struct SheetSnap {
    pub spec: Arc<TypeSpec>,
    pub data: Node,
    pub formulas: Arc<Formulas>,
}

/// A consistent view of the workbook and the list of formulas to run. Cheap to
/// make: it shares the tree and the formulas with the live workbook.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(crate) version: u64,
    pub(crate) sheets: HashMap<String, SheetSnap>,
    pub(crate) dirty: Vec<Loc>,
}

/// What one formula came to.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub loc: Loc,
    pub computed: Computed,
    pub traceback: Option<String>,
    /// Everything the formula read, found or not.
    pub deps: Vec<Loc>,
}

/// The results of one evaluation, in the order they were finished: a formula
/// comes after those it read.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub(crate) base: u64,
    pub(crate) outcomes: Vec<Outcome>,
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

    /// How many formulas are to be run.
    pub fn pending(&self) -> usize {
        self.dirty.len()
    }

    /// Runs every formula in the list, and those that depend on them.
    pub fn evaluate(&self, engine: &dyn Engine) -> Evaluation {
        let mut run = Run {
            engine,
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
        Evaluation {
            base: self.version,
            outcomes: run.outcomes,
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

struct Run<'a> {
    engine: &'a dyn Engine,
    /// The tabs with the results so far written into their trees.
    sheets: HashMap<String, SheetSnap>,
    /// By tab, then path. A formula that is not here is up to date.
    state: HashMap<String, HashMap<Path, St>>,
    /// How many formulas are not yet `Done`.
    open: usize,
    /// What the latest, unfinished run of a formula read.
    last_deps: HashMap<Loc, Vec<Loc>>,
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
    /// include: those at the path, above it, or below it.
    fn open_related(&self, sheet: &str, path: &Path, out: &mut Vec<(Loc, St)>) {
        if self.open == 0 {
            return;
        }
        let (Some(snap), Some(states)) = (self.sheets.get(sheet), self.state.get(sheet)) else {
            return;
        };
        let mut paths = Vec::new();
        snap.formulas.index.related(path.segments(), &mut paths);
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
            self.open_related(&dep.sheet, &dep.path, &mut found);
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
        let engine = self.engine;
        let mut ctx = EvalContext {
            run: &*self,
            loc,
            deps: IndexSet::new(),
            blockers: Vec::new(),
            stale_seen: false,
        };
        let result = engine.eval(&text, &mut ctx);
        let EvalContext { deps, blockers, .. } = ctx;
        let deps: Vec<Loc> = deps.into_iter().collect();
        if !blockers.is_empty() {
            self.last_deps.insert(loc.clone(), deps);
            let mut seen = HashSet::new();
            let blockers = blockers
                .into_iter()
                .filter(|b| seen.insert(b.loc.clone()))
                .collect();
            return Attempt::Blocked(blockers);
        }
        let (computed, traceback) = finish(&spec, loc, result);
        self.record(loc, computed, traceback, deps);
        Attempt::Finished
    }

    /// Stores a finished result in the working tree, so that formulas that read
    /// it later in this run see it, and keeps it for [`crate::Calc::apply`].
    fn record(
        &mut self,
        loc: &Loc,
        mut computed: Computed,
        traceback: Option<String>,
        deps: Vec<Loc>,
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

struct EvalContext<'r, 'e> {
    run: &'r Run<'e>,
    loc: &'r Loc,
    deps: IndexSet<Loc>,
    blockers: Vec<Blocker>,
    /// A value that is still out of date has been returned to the formula.
    stale_seen: bool,
}

impl Context for EvalContext<'_, '_> {
    fn sheet(&self) -> &str {
        &self.loc.sheet
    }

    fn path(&self) -> &Path {
        &self.loc.path
    }

    fn read(&mut self, sheet: &str, path: &Path) -> Read {
        self.deps.insert(Loc::new(sheet, path.clone()));
        let Some(snap) = self.run.sheets.get(sheet) else {
            return Read::NoSheet;
        };
        // The old value is still returned, so that the formula runs on and
        // reports everything else it needs; the run is discarded afterwards.
        let mut waiting = Vec::new();
        self.run.open_related(sheet, path, &mut waiting);
        if !waiting.is_empty() {
            let certain = !self.stale_seen;
            self.blockers.extend(
                waiting
                    .into_iter()
                    .map(|(loc, st)| Blocker { loc, st, certain }),
            );
            self.stale_seen = true;
        }
        match snap.data.get(path) {
            Some(node) => Read::Node(node.clone()),
            None => Read::Missing,
        }
    }
}
