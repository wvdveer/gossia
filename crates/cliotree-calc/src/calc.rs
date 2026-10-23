//! The live workbook as the calculation layer sees it: tabs with their data
//! and formulas, the dependency index, and the list of formulas that are out
//! of date.
//!
//! Edits go through here. Each one bumps the version, is logged, and marks the
//! formulas that depend on it as out of date. Evaluating happens on a
//! [`Snapshot`], and [`Calc::apply`] takes the results back, dropping any that
//! an edit has overtaken (DESIGN.md, "Evaluation and scheduling",
//! "Consistency").

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;

use cliotree_core::{ErrorKind, ErrorValue, Node, Path, Segment, TypeSpec, Value};
use indexmap::IndexSet;

use crate::check::{Computed, store};
use crate::engine::{Engine, Syntax};
use crate::eval::{Evaluation, SheetSnap, Snapshot};
use crate::model::{Dep, Formula, Formulas, Loc, overlaps};
use crate::trie::PathTrie;

/// How many recent edits are remembered for judging whether a result is stale.
/// An evaluation older than the oldest one kept is dropped as a whole.
const LOG_LIMIT: usize = 10_000;

#[derive(Debug, Clone)]
struct Sheet {
    spec: Arc<TypeSpec>,
    data: Node,
    formulas: Arc<Formulas>,
}

/// Why an edit was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    /// There is no tab of that name.
    NoSheet(String),
    /// A tab of that name exists already, ignoring case.
    DuplicateSheet(String),
    /// The schema defines no node there, so no formula can go there.
    NotInSchema(Loc),
    /// The node is inside the result of a formula, which owns it.
    InsideComputed { node: Loc, formula: Path },
    /// The node has formulas inside it, and a formula on it would own them.
    ContainsFormulas { node: Loc, formula: Path },
    /// The edit does not fit the data.
    Path(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::NoSheet(name) => write!(f, "there is no tab called {name}"),
            EditError::DuplicateSheet(name) => {
                write!(f, "a tab called {name} exists already (names ignore case)")
            }
            EditError::NotInSchema(loc) => {
                write!(
                    f,
                    "the schema defines no node at {loc}, so it cannot hold a formula"
                )
            }
            EditError::InsideComputed { node, formula } => write!(
                f,
                "{node} is computed by the formula at {}; edit or clear that formula instead",
                Loc::new(node.sheet.clone(), formula.clone())
            ),
            EditError::ContainsFormulas { node, formula } => write!(
                f,
                "{node} cannot take a formula because the formula at {} is inside it",
                Loc::new(node.sheet.clone(), formula.clone())
            ),
            EditError::Path(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for EditError {}

/// What [`Calc::apply`] did with an evaluation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Formulas whose result was taken.
    pub applied: Vec<Loc>,
    /// Formulas whose result an edit had overtaken. They stay out of date.
    pub discarded: Vec<Loc>,
}

/// Everything known about one node for display.
#[derive(Debug, Clone)]
pub struct Cell<'a> {
    pub node: Option<&'a Node>,
    /// The formula text without the `=`, if the node has a formula.
    pub formula: Option<&'a str>,
    /// The error the formula ended in, which the node holds as well when it
    /// exists.
    pub error: Option<&'a ErrorValue>,
    pub traceback: Option<&'a str>,
    /// An evaluation is outstanding: the node shows its last value.
    pub pending: bool,
    /// The formula whose result contains this node, so the node is read-only.
    pub computed_by: Option<Path>,
}

pub struct Calc {
    syntax: Arc<dyn Syntax>,
    sheets: indexmap::IndexMap<String, Sheet>,
    /// Which formulas read which paths, by tab name (also for tabs that do not
    /// exist, so that adding the tab runs the formulas that were looking for it).
    deep_readers: HashMap<String, PathTrie<Loc>>,
    /// Formulas that read only the structure of a node (see [`Dep`]).
    shallow_readers: HashMap<String, PathTrie<Loc>>,
    dirty: IndexSet<Loc>,
    version: u64,
    log: VecDeque<(u64, Loc)>,
    /// The version of the newest edit dropped from the log.
    log_floor: u64,
}

impl Calc {
    /// `syntax` checks formula text as it is entered.
    pub fn new(syntax: Arc<dyn Syntax>) -> Calc {
        Calc {
            syntax,
            sheets: indexmap::IndexMap::new(),
            deep_readers: HashMap::new(),
            shallow_readers: HashMap::new(),
            dirty: IndexSet::new(),
            version: 0,
            log: VecDeque::new(),
            log_floor: 0,
        }
    }

    pub fn syntax(&self) -> &Arc<dyn Syntax> {
        &self.syntax
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    // ----------------------------------------------------------- tabs

    pub fn add_sheet(
        &mut self,
        name: impl Into<String>,
        spec: TypeSpec,
        data: Node,
    ) -> Result<(), EditError> {
        let name = name.into();
        if self
            .sheets
            .keys()
            .any(|existing| existing.to_lowercase() == name.to_lowercase())
        {
            return Err(EditError::DuplicateSheet(name));
        }
        self.sheets.insert(
            name.clone(),
            Sheet {
                spec: Arc::new(spec),
                data,
                formulas: Arc::default(),
            },
        );
        self.changed(&name, &Path::root());
        Ok(())
    }

    pub fn sheet_names(&self) -> impl Iterator<Item = &str> {
        self.sheets.keys().map(String::as_str)
    }

    pub fn data(&self, sheet: &str) -> Option<&Node> {
        self.sheets.get(sheet).map(|s| &s.data)
    }

    pub fn spec(&self, sheet: &str) -> Option<&TypeSpec> {
        self.sheets.get(sheet).map(|s| &*s.spec)
    }

    /// The formulas of a tab, as `(path, text)`, in the order they were added.
    pub fn formulas(&self, sheet: &str) -> Vec<(&Path, &str)> {
        self.sheets
            .get(sheet)
            .map(|s| {
                s.formulas
                    .by_path
                    .iter()
                    .map(|(p, f)| (p, f.text.as_str()))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---------------------------------------------------------- reading

    pub fn cell(&self, sheet: &str, path: &Path) -> Option<Cell<'_>> {
        let s = self.sheets.get(sheet)?;
        let formula = s.formulas.by_path.get(path);
        let computed_by = s.formulas.computed_by(path);
        let pending = self.dirty.contains(&Loc::new(sheet, path.clone()))
            || computed_by
                .as_ref()
                .is_some_and(|p| self.dirty.contains(&Loc::new(sheet, p.clone())));
        Some(Cell {
            node: s.data.get(path),
            formula: formula.map(|f| f.text.as_str()),
            error: formula.and_then(|f| f.error.as_ref()),
            traceback: formula.and_then(|f| f.traceback.as_deref()),
            pending,
            computed_by,
        })
    }

    /// How many formulas are out of date.
    pub fn pending(&self) -> usize {
        self.dirty.len()
    }

    // ---------------------------------------------------------- editing

    /// Sets a literal. A formula at exactly this node is replaced by it
    /// (DESIGN.md, "Entering formulas"); a node inside a formula's result is
    /// refused.
    pub fn set_value(&mut self, sheet: &str, path: &Path, value: Value) -> Result<(), EditError> {
        self.refuse_inside_computed(sheet, path)?;
        let s = self.sheet_mut(sheet)?;
        s.data
            .set(path, value)
            .map_err(|e| EditError::Path(e.to_string()))?;
        self.drop_formula(sheet, path);
        self.changed(sheet, path);
        Ok(())
    }

    /// Removes a node, shifting later array items down. Formulas stay where
    /// they are: they belong to a location, not to an item (DESIGN.md,
    /// "References are literal").
    pub fn remove(&mut self, sheet: &str, path: &Path) -> Result<(), EditError> {
        self.refuse_inside_computed(sheet, path)?;
        let s = self.sheet_mut(sheet)?;
        s.data
            .remove(path)
            .map_err(|e| EditError::Path(e.to_string()))?;
        self.drop_formula(sheet, path);
        // Removing an array item moves the items after it.
        let changed = match path.segments().last() {
            Some(Segment::Index(_)) => path.parent().unwrap_or_default(),
            _ => path.clone(),
        };
        self.changed(sheet, &changed);
        Ok(())
    }

    /// Inserts an item into an array at `index`, shifting later items up.
    pub fn insert_item(
        &mut self,
        sheet: &str,
        array: &Path,
        index: usize,
        value: Value,
    ) -> Result<(), EditError> {
        self.refuse_inside_computed(sheet, &array.index(index))?;
        let s = self.sheet_mut(sheet)?;
        let mut items = match s.data.get(array).map(Node::value) {
            Some(Value::Array(items)) => items.clone(),
            _ => return Err(EditError::Path(format!("there is no array at {array}"))),
        };
        if index > items.len() {
            return Err(EditError::Path(format!(
                "cannot insert at {index} in an array of {}",
                items.len()
            )));
        }
        items.insert(index, Node::new(value));
        s.data
            .set(array, Value::Array(items))
            .map_err(|e| EditError::Path(e.to_string()))?;
        self.changed(sheet, array);
        Ok(())
    }

    /// Attaches a formula to a node, or replaces the one there. `text` is the
    /// expression without the leading `=`.
    ///
    /// Text that does not parse is kept: the node holds `#SYNTAX!` and the text
    /// stays for the author to correct (DESIGN.md, "Syntax errors").
    pub fn set_formula(&mut self, sheet: &str, path: &Path, text: &str) -> Result<(), EditError> {
        let loc = Loc::new(sheet, path.clone());
        let s = self.sheet_mut(sheet)?;
        if s.spec.resolve(path).is_none() {
            return Err(EditError::NotInSchema(loc));
        }
        if let Some(formula) = s.formulas.computed_by(path) {
            return Err(EditError::InsideComputed { node: loc, formula });
        }
        if let Some(formula) = s.formulas.below(path).into_iter().next() {
            return Err(EditError::ContainsFormulas { node: loc, formula });
        }
        let syntax = self.syntax.check(text);
        let s = self.sheet_mut(sheet)?;
        let mut formula = Formula {
            text: text.to_string(),
            deps: Vec::new(),
            error: None,
            traceback: None,
            syntax_error: false,
        };
        if let Err(e) = &syntax {
            let error = ErrorValue {
                kind: ErrorKind::Syntax,
                message: e.to_string(),
                origin: Some(path.clone()),
            };
            let computed = Computed::Error(error.clone());
            let _ = store(&s.spec, &mut s.data, path, &computed);
            formula.syntax_error = true;
            formula.error = Some(error);
        }
        let old = Arc::make_mut(&mut s.formulas).insert(path.clone(), formula);
        if let Some(old) = old {
            self.forget_deps(&loc, &old.deps);
        }
        self.dirty.swap_remove(&loc);
        self.changed(sheet, path);
        Ok(())
    }

    /// Removes the formula at a node and leaves the node's value as it is.
    /// Returns whether there was one.
    pub fn remove_formula(&mut self, sheet: &str, path: &Path) -> Result<bool, EditError> {
        self.sheet_mut(sheet)?;
        let existed = self.drop_formula(sheet, path);
        if existed {
            self.changed(sheet, path);
        }
        Ok(existed)
    }

    /// Marks every formula as out of date, as F9 does and as opening a trusted
    /// workbook does. Formulas that do not parse are left alone.
    pub fn recalc_all(&mut self) {
        for (name, sheet) in &self.sheets {
            for (path, formula) in &sheet.formulas.by_path {
                if !formula.syntax_error {
                    self.dirty.insert(Loc::new(name.clone(), path.clone()));
                }
            }
        }
    }

    // ------------------------------------------------------- evaluating

    /// A consistent view to evaluate. Take it, evaluate it anywhere, and hand
    /// the result to [`Calc::apply`].
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: self.version,
            sheets: self
                .sheets
                .iter()
                .map(|(name, s)| {
                    (
                        name.clone(),
                        SheetSnap {
                            spec: Arc::clone(&s.spec),
                            data: s.data.clone(),
                            formulas: Arc::clone(&s.formulas),
                        },
                    )
                })
                .collect(),
            dirty: self.dirty.iter().cloned().collect(),
        }
    }

    /// Takes the results of an evaluation. A result is dropped if an edit made
    /// after the snapshot touched anything the formula read or its own node,
    /// or if it read the result of a formula that was dropped. Dropped
    /// formulas stay out of date, for the next evaluation.
    pub fn apply(&mut self, evaluation: Evaluation) -> Applied {
        let mut applied = Applied::default();
        let mut dropped: HashMap<String, PathTrie<Loc>> = HashMap::new();
        for outcome in evaluation.outcomes {
            let loc = outcome.loc.clone();
            let overtaken = self.edited_since(evaluation.base, &loc, false)
                || outcome.deps.iter().any(|dep| {
                    self.edited_since(evaluation.base, &dep.loc, dep.shallow)
                        || dropped.get(&dep.loc.sheet).is_some_and(|t| {
                            if dep.shallow {
                                // A dropped result above the node changes its structure.
                                t.any_at_or_above(dep.loc.path.segments())
                            } else {
                                t.any_related(dep.loc.path.segments())
                            }
                        })
                })
                || !self
                    .sheets
                    .get(&loc.sheet)
                    .is_some_and(|s| s.formulas.by_path.contains_key(&loc.path));
            if overtaken {
                dropped
                    .entry(loc.sheet.clone())
                    .or_default()
                    .insert(loc.path.segments(), loc.clone());
                applied.discarded.push(loc);
                continue;
            }
            let sheet = self.sheets.get_mut(&loc.sheet).expect("checked above");
            let _ = store(&sheet.spec, &mut sheet.data, &loc.path, &outcome.computed);
            let formulas = Arc::make_mut(&mut sheet.formulas);
            let formula = formulas.by_path.get_mut(&loc.path).expect("checked above");
            formula.error = match outcome.computed {
                Computed::Error(e) => Some(e),
                _ => None,
            };
            formula.traceback = outcome.traceback;
            let old = std::mem::replace(&mut formula.deps, outcome.deps.clone());
            self.forget_deps(&loc, &old);
            self.remember_deps(&loc, &outcome.deps);
            self.dirty.swap_remove(&loc);
            applied.applied.push(loc);
        }
        applied
    }

    /// Evaluates everything that is out of date and applies the results, on
    /// this thread. Returns how many formulas were run.
    pub fn recalc(&mut self, engine: &dyn Engine) -> usize {
        let mut run = 0;
        while !self.dirty.is_empty() {
            let evaluation = self.snapshot().evaluate(engine);
            run += evaluation.outcomes.len();
            if self.apply(evaluation).applied.is_empty() {
                break;
            }
        }
        run
    }

    // ------------------------------------------------------- internals

    fn sheet_mut(&mut self, name: &str) -> Result<&mut Sheet, EditError> {
        self.sheets
            .get_mut(name)
            .ok_or_else(|| EditError::NoSheet(name.to_string()))
    }

    fn refuse_inside_computed(&self, sheet: &str, path: &Path) -> Result<(), EditError> {
        let s = self
            .sheets
            .get(sheet)
            .ok_or_else(|| EditError::NoSheet(sheet.to_string()))?;
        match s.formulas.computed_by(path) {
            Some(formula) => Err(EditError::InsideComputed {
                node: Loc::new(sheet, path.clone()),
                formula,
            }),
            None => Ok(()),
        }
    }

    /// Removes a formula and what is known about what it read.
    fn drop_formula(&mut self, sheet: &str, path: &Path) -> bool {
        let Some(s) = self.sheets.get_mut(sheet) else {
            return false;
        };
        if !s.formulas.by_path.contains_key(path) {
            return false;
        }
        let removed = Arc::make_mut(&mut s.formulas).remove(path);
        let loc = Loc::new(sheet, path.clone());
        self.dirty.swap_remove(&loc);
        if let Some(formula) = removed {
            self.forget_deps(&loc, &formula.deps);
        }
        true
    }

    fn remember_deps(&mut self, formula: &Loc, deps: &[Dep]) {
        for dep in deps {
            let readers = if dep.shallow {
                &mut self.shallow_readers
            } else {
                &mut self.deep_readers
            };
            readers
                .entry(dep.loc.sheet.clone())
                .or_default()
                .insert(dep.loc.path.segments(), formula.clone());
        }
    }

    fn forget_deps(&mut self, formula: &Loc, deps: &[Dep]) {
        for dep in deps {
            let readers = if dep.shallow {
                &mut self.shallow_readers
            } else {
                &mut self.deep_readers
            };
            if let Some(trie) = readers.get_mut(&dep.loc.sheet) {
                trie.remove(dep.loc.path.segments(), formula);
            }
        }
    }

    /// Records an edit and marks what depends on it as out of date.
    fn changed(&mut self, sheet: &str, path: &Path) {
        self.version += 1;
        self.log
            .push_back((self.version, Loc::new(sheet, path.clone())));
        while self.log.len() > LOG_LIMIT {
            if let Some((version, _)) = self.log.pop_front() {
                self.log_floor = version;
            }
        }
        self.mark_dirty_from(sheet, path);
    }

    /// Whether an edit made after `base` concerns a read of `loc`: for a deep
    /// read, an edit at the node, above it, or below it; for a shallow read,
    /// an edit at the node or above it.
    fn edited_since(&self, base: u64, loc: &Loc, shallow: bool) -> bool {
        if base < self.log_floor {
            return true;
        }
        self.log
            .iter()
            .rev()
            .take_while(|(version, _)| *version > base)
            .any(|(_, edit)| {
                edit.sheet == loc.sheet
                    && if shallow {
                        loc.path.segments().starts_with(edit.path.segments())
                    } else {
                        overlaps(&edit.path, &loc.path)
                    }
            })
    }

    /// Marks as out of date every formula that read something at or around
    /// `path`, and every formula located there (their results are overwritten
    /// by the edit), and then, in turn, whatever read their results.
    fn mark_dirty_from(&mut self, sheet: &str, path: &Path) {
        let mut queue = vec![Loc::new(sheet, path.clone())];
        while let Some(at) = queue.pop() {
            let mut found: Vec<Loc> = Vec::new();
            if let Some(readers) = self.deep_readers.get(&at.sheet) {
                readers.related(at.path.segments(), &mut found);
            }
            if let Some(readers) = self.shallow_readers.get(&at.sheet) {
                readers.at_or_below(at.path.segments(), &mut found);
            }
            if let Some(s) = self.sheets.get(&at.sheet) {
                let mut located = Vec::new();
                s.formulas.index.related(at.path.segments(), &mut located);
                found.extend(located.into_iter().map(|p| Loc::new(at.sheet.clone(), p)));
            }
            for loc in found {
                let runnable = self
                    .sheets
                    .get(&loc.sheet)
                    .and_then(|s| s.formulas.by_path.get(&loc.path))
                    .is_some_and(|f| !f.syntax_error);
                if runnable && self.dirty.insert(loc.clone()) {
                    queue.push(loc);
                }
            }
        }
    }
}
