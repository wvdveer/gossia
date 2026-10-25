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
use crate::history::{Entry, History, MergeKey, Op};
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
    /// How the tab is tied to a schema (see the `schema` module).
    schema: SchemaRole,
    /// What an XML tab says about its document beyond the tree: the declaration,
    /// the prefix and the namespaces of the root. None on a JSON tab.
    xml: Option<cliotree_core::xml::XmlDocument>,
    /// The formulas that are given to a field of every item of an array.
    columns: Vec<ColumnFormula>,
}

/// What a tab is to the schema machinery.
#[derive(Debug, Clone, Default)]
enum SchemaRole {
    /// An ordinary tab, bound to nothing: its type was handed over directly.
    #[default]
    Free,
    /// A tab whose data is a schema document, described by the bundled
    /// meta-schema.
    Document {
        /// The language the document is written in.
        format: SchemaFormat,
        /// Whether tabs that use it may change it (DESIGN.md, "Schema binding").
        auto_update: bool,
        /// Why the document cannot be lowered now, if it cannot.
        error: Option<String>,
    },
    /// A tab bound to the schema tab of that name.
    Bound(String),
}

#[path = "schema.rs"]
mod schema;
pub use schema::{FieldKind, FieldPlaces, SchemaFormat};

#[path = "package.rs"]
mod package;
pub use package::{
    ENGINE_NAME, ENGINE_REVISION, FORMAT_MAJOR, FORMAT_MINOR, Meta, Opened, Package, PackageError,
    RESERVED_NAMES_VERSION, safe_path,
};

#[path = "columns.rs"]
mod columns;
pub use columns::ColumnFormula;

#[path = "modules.rs"]
mod modules;
use modules::Module;
pub use modules::ModuleStatus;

#[path = "trust.rs"]
mod trust;
pub use trust::{Change, ChangeKind, Decision, Fingerprint, TrustStore};

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
    /// There is no module of that name.
    NoModule(String),
    /// A tab or module name that cannot be used; the message says why.
    BadName(String),
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
            EditError::Path(message) | EditError::BadName(message) => f.write_str(message),
            EditError::NoModule(name) => write!(f, "there is no module called {name}"),
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
    /// The node has a formula that is not evaluated because the workbook is
    /// not trusted: the node shows the value it was stored with.
    pub untrusted: bool,
    /// The formula is out of date and, because the workbook is not trusted,
    /// cannot be brought up to date.
    pub stale: bool,
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
    /// Counts every change to what the tabs hold: edits, and results that were
    /// taken. (`version` counts only edits, because a result is not an edit.)
    revision: u64,
    log: VecDeque<(u64, Loc)>,
    /// The version of the newest edit dropped from the log.
    log_floor: u64,
    history: History,
    /// The workbook's Python modules, in display order.
    modules: indexmap::IndexMap<String, Module>,
    /// Counts the changes to modules, so that every one has a generation of
    /// its own.
    module_generation: u64,
    /// Whether Python may run (see [`Calc::set_trusted`]).
    trusted: bool,
    /// The customised forms of the tabs.
    forms: crate::forms::FormStore,
    /// Counts the times the forms were changed, which are not edits of the
    /// data and so are not in the history, but are unsaved changes all the same.
    forms_version: u64,
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
            revision: 0,
            log: VecDeque::new(),
            log_floor: 0,
            history: History::default(),
            modules: indexmap::IndexMap::new(),
            module_generation: 0,
            trusted: true,
            forms: crate::forms::FormStore::default(),
            forms_version: 0,
        }
    }

    /// The customised forms of the tabs. A form that has not been customised
    /// is not stored: it is generated from the schema.
    pub fn forms(&self) -> &crate::forms::FormStore {
        &self.forms
    }

    /// Changes to forms are not edits of the workbook's data, so they are not
    /// in the undo history and do not recalculate anything.
    pub fn forms_mut(&mut self) -> &mut crate::forms::FormStore {
        self.forms_version += 1;
        &mut self.forms
    }

    /// Changes whenever the forms may have been changed.
    pub fn forms_version(&self) -> u64 {
        self.forms_version
    }

    pub fn syntax(&self) -> &Arc<dyn Syntax> {
        &self.syntax
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Changes whenever anything in the tabs changes, results included, so a
    /// view of the data knows when to draw it again.
    pub fn revision(&self) -> u64 {
        self.revision
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
                schema: SchemaRole::Free,
                xml: None,
                columns: Vec::new(),
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
        let waiting = self.dirty.contains(&Loc::new(sheet, path.clone()))
            || computed_by
                .as_ref()
                .is_some_and(|p| self.dirty.contains(&Loc::new(sheet, p.clone())));
        Some(Cell {
            node: s.data.get(path),
            formula: formula.map(|f| f.text.as_str()),
            error: formula.and_then(|f| f.error.as_ref()),
            traceback: formula.and_then(|f| f.traceback.as_deref()),
            pending: waiting && self.trusted,
            untrusted: !self.trusted && (formula.is_some() || computed_by.is_some()),
            stale: waiting && !self.trusted,
            computed_by,
        })
    }

    /// How many formulas are out of date and will be evaluated. None while the
    /// workbook is not trusted (see [`Calc::stale`]).
    pub fn pending(&self) -> usize {
        if self.trusted { self.dirty.len() } else { 0 }
    }

    // ---------------------------------------------------------- editing
    //
    // Every edit is an entry in the history unless several are being grouped
    // (see [`Calc::begin_entry`]).

    /// Sets a literal. A formula at exactly this node is replaced by it
    /// (DESIGN.md, "Entering formulas"); a node inside a formula's result is
    /// refused.
    pub fn set_value(&mut self, sheet: &str, path: &Path, value: Value) -> Result<(), EditError> {
        self.set_value_as(sheet, path, value, false)
    }

    /// Like [`Calc::set_value`], but an edit of this kind to the same cell
    /// straight after another joins the same entry, so that typing in a cell
    /// is one thing to undo. [`Calc::seal_history`] ends the run.
    pub fn set_value_merging(
        &mut self,
        sheet: &str,
        path: &Path,
        value: Value,
    ) -> Result<(), EditError> {
        self.set_value_as(sheet, path, value, true)
    }

    fn set_value_as(
        &mut self,
        sheet: &str,
        path: &Path,
        value: Value,
        merge: bool,
    ) -> Result<(), EditError> {
        self.refuse_inside_computed(sheet, path)?;
        let formula = self.formula_text(sheet, path);
        let s = self.sheet_mut(sheet)?;
        let before = s.data.get(path).cloned();
        s.data
            .set(path, value)
            .map_err(|e| EditError::Path(e.to_string()))?;
        let after = s.data.get(path).cloned().expect("it was just set");
        let position = s.data.position_of(path).unwrap_or(0);
        let mut ops = Vec::new();
        // A literal in one item of a column ends the column formula.
        ops.extend(self.release_column(sheet, path));
        if let Some(text) = formula {
            ops.push(Op::Formula {
                sheet: sheet.to_owned(),
                path: path.clone(),
                before: Some(text),
                after: None,
                value: None,
            });
        }
        ops.push(match before {
            Some(before) => Op::Replace {
                sheet: sheet.to_owned(),
                path: path.clone(),
                before,
                after,
            },
            None => Op::Add {
                sheet: sheet.to_owned(),
                path: path.clone(),
                position,
                node: after,
                moved: Vec::new(),
            },
        });
        self.drop_formula(sheet, path);
        self.changed(sheet, path);
        ops.extend(self.sync_columns(sheet, path));
        let key = merge.then(|| MergeKey {
            sheet: sheet.to_owned(),
            path: path.clone(),
        });
        self.history
            .record(format!("Set {}", Loc::new(sheet, path.clone())), ops, key);
        Ok(())
    }

    /// Removes a node, shifting later array items down. Formulas stay where
    /// they are: they belong to a location, not to an item (DESIGN.md,
    /// "References are literal").
    pub fn remove(&mut self, sheet: &str, path: &Path) -> Result<(), EditError> {
        self.refuse_inside_computed(sheet, path)?;
        let formula = self.formula_text(sheet, path);
        let s = self.sheet_mut(sheet)?;
        let position = s
            .data
            .position_of(path)
            .ok_or_else(|| EditError::Path(format!("there is no node at {path} to remove")))?;
        let node = s
            .data
            .remove(path)
            .map_err(|e| EditError::Path(e.to_string()))?;
        let mut ops = Vec::new();
        ops.extend(self.release_column(sheet, path));
        if let Some(text) = formula {
            ops.push(Op::Formula {
                sheet: sheet.to_owned(),
                path: path.clone(),
                before: Some(text),
                after: None,
                value: None,
            });
        }
        self.drop_formula(sheet, path);
        let moved = match path.segments().last() {
            Some(Segment::Index(i)) => {
                self.moved_by(sheet, &path.parent().unwrap_or_default(), *i, 1)
            }
            _ => Vec::new(),
        };
        ops.push(Op::Remove {
            sheet: sheet.to_owned(),
            path: path.clone(),
            position,
            node,
            moved,
        });
        self.changed_by_structure(sheet, path);
        ops.extend(self.sync_columns(sheet, path));
        self.history.record(
            format!("Remove {}", Loc::new(sheet, path.clone())),
            ops,
            None,
        );
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
        let node = Node::new(value);
        let s = self.sheet_mut(sheet)?;
        s.data
            .insert_child(array, index, &Segment::Index(index), node.clone())
            .map_err(|e| EditError::Path(e.to_string()))?;
        self.changed(sheet, array);
        let path = array.index(index);
        let moved = self.moved_by(sheet, array, index + 1, -1);
        let mut ops = vec![Op::Add {
            sheet: sheet.to_owned(),
            path: path.clone(),
            position: index,
            node,
            moved,
        }];
        ops.extend(self.sync_columns(sheet, &path));
        self.history
            .record(format!("Insert {}", Loc::new(sheet, path)), ops, None);
        Ok(())
    }

    /// Puts the items of the array at `array` in a new order: place `i` gets
    /// the item that was at `order[i]`. One entry in the history, labelled
    /// `what` (for example "Sort"). Formulas belong to a place, so they stay
    /// where they are and are calculated again against the item now there.
    pub fn reorder(
        &mut self,
        sheet: &str,
        array: &Path,
        order: &[usize],
        what: &str,
    ) -> Result<(), EditError> {
        self.refuse_inside_computed(sheet, array)?;
        if self.formula_text(sheet, array).is_some() {
            return Err(EditError::Path(format!(
                "{} is computed by a formula, so its order cannot be changed",
                Loc::new(sheet, array.clone())
            )));
        }
        let s = self.sheet_mut(sheet)?;
        let before = s
            .data
            .get(array)
            .cloned()
            .ok_or_else(|| EditError::Path(format!("there is no node at {array}")))?;
        let Value::Array(items) = before.value() else {
            return Err(EditError::Path(format!("{array} is not an array")));
        };
        let mut seen = vec![false; items.len()];
        let valid = order.len() == items.len()
            && order
                .iter()
                .all(|&i| i < seen.len() && !std::mem::replace(&mut seen[i], true));
        if !valid {
            return Err(EditError::Path(
                "the new order must name every item once".to_owned(),
            ));
        }
        if order.iter().enumerate().all(|(place, &i)| place == i) {
            return Ok(());
        }
        let moved: Vec<Node> = order.iter().map(|&i| items[i].clone()).collect();
        let mut after = before.clone();
        *after.value_mut() = Value::Array(moved);
        s.data
            .replace(array, after.clone())
            .map_err(|e| EditError::Path(e.to_string()))?;
        self.changed(sheet, array);
        self.history.record(
            format!("{what} {}", Loc::new(sheet, array.clone())),
            vec![Op::Replace {
                sheet: sheet.to_owned(),
                path: array.clone(),
                before,
                after,
            }],
            None,
        );
        Ok(())
    }

    /// Attaches a formula to a node, or replaces the one there. `text` is the
    /// expression without the leading `=`.
    ///
    /// Text that does not parse is kept: the node holds `#SYNTAX!` and the text
    /// stays for the author to correct (DESIGN.md, "Syntax errors").
    pub fn set_formula(&mut self, sheet: &str, path: &Path, text: &str) -> Result<(), EditError> {
        self.check_formula_place(sheet, path)?;
        let loc = Loc::new(sheet, path.clone());
        let mut ops = Vec::new();
        // A different formula on one item of a column ends the column formula.
        if self.column_of(sheet, path).is_some_and(|c| c.text != text) {
            ops.extend(self.release_column(sheet, path));
        }
        ops.push(self.formula_op(sheet, path, text));
        self.history
            .record(format!("Set formula at {loc}"), ops, None);
        Ok(())
    }

    /// Whether a formula can go on the node.
    fn check_formula_place(&self, sheet: &str, path: &Path) -> Result<(), EditError> {
        let loc = Loc::new(sheet, path.clone());
        let s = self
            .sheets
            .get(sheet)
            .ok_or_else(|| EditError::NoSheet(sheet.to_owned()))?;
        if s.spec.resolve(path).is_none() {
            return Err(EditError::NotInSchema(loc));
        }
        if let Some(formula) = s.formulas.computed_by(path) {
            return Err(EditError::InsideComputed { node: loc, formula });
        }
        if let Some(formula) = s.formulas.below(path).into_iter().next() {
            return Err(EditError::ContainsFormulas { node: loc, formula });
        }
        Ok(())
    }

    /// Puts a formula on a node and returns the operation that records it.
    fn formula_op(&mut self, sheet: &str, path: &Path, text: &str) -> Op {
        let before = self.formula_text(sheet, path);
        let value = self.sheets.get(sheet).and_then(|s| {
            s.data
                .get(path)
                .cloned()
                .map(|node| (node, s.data.position_of(path).unwrap_or(0)))
        });
        self.install_formula(sheet, path, text);
        Op::Formula {
            sheet: sheet.to_owned(),
            path: path.clone(),
            before,
            after: Some(text.to_owned()),
            value: Some(value),
        }
    }

    /// Puts a formula on a node (replacing one that is there) and marks it out
    /// of date. Nothing is checked and nothing is recorded: the callers do that.
    fn install_formula(&mut self, sheet: &str, path: &Path, text: &str) {
        let loc = Loc::new(sheet, path.clone());
        let syntax = self.syntax.check(text);
        let Some(s) = self.sheets.get_mut(sheet) else {
            return;
        };
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
    }

    /// The text of the formula at a node.
    pub fn formula_text(&self, sheet: &str, path: &Path) -> Option<String> {
        self.sheets
            .get(sheet)?
            .formulas
            .by_path
            .get(path)
            .map(|f| f.text.clone())
    }

    /// Removes the formula at a node and leaves the node's value as it is.
    /// Returns whether there was one.
    pub fn remove_formula(&mut self, sheet: &str, path: &Path) -> Result<bool, EditError> {
        self.sheet_mut(sheet)?;
        let Some(text) = self.formula_text(sheet, path) else {
            return Ok(false);
        };
        // Clearing one item of a column ends the column formula.
        let mut ops: Vec<Op> = self.release_column(sheet, path).into_iter().collect();
        self.drop_formula(sheet, path);
        self.changed(sheet, path);
        ops.push(Op::Formula {
            sheet: sheet.to_owned(),
            path: path.clone(),
            before: Some(text),
            after: None,
            value: None,
        });
        self.history.record(
            format!("Clear formula at {}", Loc::new(sheet, path.clone())),
            ops,
            None,
        );
        Ok(true)
    }

    // ------------------------------------------------- undo and redo

    /// Starts an entry that several edits make up: a paste, the writes of a
    /// button procedure. Calls nest; the entry ends with the matching
    /// [`Calc::end_entry`].
    pub fn begin_entry(&mut self, description: &str) {
        self.history.begin(description);
    }

    pub fn end_entry(&mut self) {
        self.history.end();
    }

    /// Ends the run of edits that [`Calc::set_value_merging`] joins together.
    pub fn seal_history(&mut self) {
        self.history.seal();
    }

    /// Undoes the last entry. Returns what it was called, or `None` if there
    /// is nothing to undo.
    pub fn undo(&mut self) -> Result<Option<String>, EditError> {
        self.refuse_history_while_grouping()?;
        let Some(entry) = self.history.pop_undo() else {
            return Ok(None);
        };
        self.run_entry(&entry, false)?;
        let description = entry.description.clone();
        self.history.finish_undo(entry);
        Ok(Some(description))
    }

    /// Does again what was undone.
    pub fn redo(&mut self) -> Result<Option<String>, EditError> {
        self.refuse_history_while_grouping()?;
        let Some(entry) = self.history.pop_redo() else {
            return Ok(None);
        };
        self.run_entry(&entry, true)?;
        let description = entry.description.clone();
        self.history.finish_redo(entry);
        Ok(Some(description))
    }

    fn refuse_history_while_grouping(&self) -> Result<(), EditError> {
        if self.history.is_grouping() {
            Err(EditError::Path(
                "an edit is in progress; finish it before undoing".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    /// Runs the operations of an entry forwards, or backwards in reverse order.
    /// If one fails the history can no longer be trusted and is cleared.
    fn run_entry(&mut self, entry: &Entry, forward: bool) -> Result<(), EditError> {
        let result = if forward {
            entry.ops.iter().try_for_each(|op| self.run_op(op, true))
        } else {
            entry
                .ops
                .iter()
                .rev()
                .try_for_each(|op| self.run_op(op, false))
        };
        if result.is_err() {
            self.history.clear();
        }
        result
    }

    fn run_op(&mut self, op: &Op, forward: bool) -> Result<(), EditError> {
        let path_error = |e: cliotree_core::PathError| EditError::Path(e.to_string());
        match op {
            Op::Replace {
                sheet,
                path,
                before,
                after,
            } => {
                let node = if forward { after } else { before }.clone();
                self.sheet_mut(sheet)?
                    .data
                    .replace(path, node)
                    .map_err(path_error)?;
                self.changed(sheet, path);
            }
            Op::Add {
                sheet,
                path,
                position,
                node,
                moved,
            }
            | Op::Remove {
                sheet,
                path,
                position,
                node,
                moved,
            } => {
                let adding = matches!(op, Op::Add { .. }) == forward;
                if adding {
                    self.insert_node(sheet, path, *position, node.clone())?;
                } else {
                    self.sheet_mut(sheet)?
                        .data
                        .remove(path)
                        .map_err(path_error)?;
                    self.changed_by_structure(sheet, path);
                }
                if !forward {
                    // Items that formulas overwrote after they moved.
                    for (at, original) in moved {
                        if self.sheet_mut(sheet)?.data.get(at).is_some() {
                            self.sheet_mut(sheet)?
                                .data
                                .replace(at, original.clone())
                                .map_err(path_error)?;
                            self.changed(sheet, at);
                        }
                    }
                }
            }
            Op::ModuleSource {
                name,
                before,
                after,
            } => {
                self.store_module_source(name, if forward { after } else { before })?;
            }
            Op::AutoUpdate {
                sheet,
                before,
                after,
            } => {
                self.store_auto_update(sheet, if forward { *after } else { *before })?;
            }
            Op::Column {
                sheet,
                array,
                field,
                before,
                after,
            } => {
                self.sheet_mut(sheet)?;
                let target = if forward { after } else { before };
                self.put_column(sheet, array, field, target.as_deref());
            }
            Op::Formula {
                sheet,
                path,
                before,
                after,
                value,
            } => {
                self.sheet_mut(sheet)?;
                let target = if forward { after } else { before };
                match target {
                    Some(text) => self.install_formula(sheet, path, text),
                    None => {
                        if self.drop_formula(sheet, path) {
                            self.changed(sheet, path);
                        }
                    }
                }
                if !forward && let Some(value) = value {
                    self.restore_value(sheet, path, value.clone())?;
                }
            }
        }
        Ok(())
    }

    fn insert_node(
        &mut self,
        sheet: &str,
        path: &Path,
        position: usize,
        node: Node,
    ) -> Result<(), EditError> {
        let Some(segment) = path.segments().last().cloned() else {
            return Err(EditError::Path("the root cannot be inserted".to_owned()));
        };
        let parent = path.parent().unwrap_or_default();
        self.sheet_mut(sheet)?
            .data
            .insert_child(&parent, position, &segment, node)
            .map_err(|e| EditError::Path(e.to_string()))?;
        self.changed_by_structure(sheet, path);
        Ok(())
    }

    /// Puts back what a node held before a formula was put on it.
    fn restore_value(
        &mut self,
        sheet: &str,
        path: &Path,
        value: Option<(Node, usize)>,
    ) -> Result<(), EditError> {
        let s = self.sheet_mut(sheet)?;
        match value {
            Some((node, position)) => {
                if s.data.get(path).is_some() {
                    s.data
                        .replace(path, node)
                        .map_err(|e| EditError::Path(e.to_string()))?;
                    self.changed(sheet, path);
                } else {
                    self.insert_node(sheet, path, position, node)?;
                }
            }
            None => {
                if s.data.remove(path).is_ok() {
                    self.changed_by_structure(sheet, path);
                }
            }
        }
        Ok(())
    }

    /// The items of `array` that, after an edit moved them, sit where a formula
    /// is, together with the place each had before the edit. A formula belongs
    /// to a location, so the item that moves there is overwritten by its
    /// result; undoing the edit has to put the item back as it was.
    /// `first` is the first index in the new layout that holds a moved item, and
    /// `before` is how far the old index of such an item is from the new one.
    fn moved_by(
        &self,
        sheet: &str,
        array: &Path,
        first: usize,
        before: isize,
    ) -> Vec<(Path, Node)> {
        let Some(s) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        let depth = array.len();
        let mut indexes: Vec<usize> = s
            .formulas
            .below(array)
            .iter()
            .filter_map(|p| match p.segments().get(depth) {
                Some(Segment::Index(k)) => Some(*k),
                _ => None,
            })
            .filter(|k| *k >= first)
            .collect();
        indexes.sort_unstable();
        indexes.dedup();
        indexes
            .into_iter()
            .filter_map(|k| {
                let node = s.data.get(&array.index(k))?.clone();
                let old = usize::try_from(k as isize + before).ok()?;
                Some((array.index(old), node))
            })
            .collect()
    }

    /// An edit that adds or removes a node: when it is an array item, the items
    /// after it move, so the array is what changed.
    fn changed_by_structure(&mut self, sheet: &str, path: &Path) {
        let changed = match path.segments().last() {
            Some(Segment::Index(_)) => path.parent().unwrap_or_default(),
            _ => path.clone(),
        };
        self.changed(sheet, &changed);
    }

    pub fn can_undo(&self) -> bool {
        self.history.undo_label().is_some()
    }

    pub fn can_redo(&self) -> bool {
        self.history.redo_label().is_some()
    }

    /// What `undo` would undo, for the Edit menu.
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    /// How many entries can be undone and redone.
    pub fn history_len(&self) -> (usize, usize) {
        self.history.len()
    }

    /// Bounds the history: the most entries, and a rough measure of how much
    /// they hold. The oldest entries are dropped first.
    pub fn set_history_limits(&mut self, max_entries: usize, max_weight: usize) {
        self.history.set_limits(max_entries, max_weight);
    }

    /// Forgets all history, for example when a workbook is closed.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Notes that the workbook has just been saved.
    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    /// Whether the workbook differs from the last saved state. Undoing back to
    /// that state clears it.
    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
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
        // Nothing is to be run in a workbook that is not trusted, and no module
        // loaded.
        let trusted = self.trusted;
        Snapshot {
            version: self.version,
            modules: if trusted {
                self.module_specs()
            } else {
                Arc::default()
            },
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
            dirty: if trusted {
                self.dirty.iter().cloned().collect()
            } else {
                Vec::new()
            },
        }
    }

    /// Takes the results of an evaluation. A result is dropped if an edit made
    /// after the snapshot touched anything the formula read or its own node,
    /// or if it read the result of a formula that was dropped. Dropped
    /// formulas stay out of date, for the next evaluation.
    pub fn apply(&mut self, evaluation: Evaluation) -> Applied {
        self.apply_module_reports(evaluation.modules);
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
        if !applied.applied.is_empty() {
            self.revision += 1;
        }
        applied
    }

    /// Evaluates everything that is out of date and applies the results, on
    /// this thread. Returns how many formulas were run.
    pub fn recalc(&mut self, engine: &dyn Engine) -> usize {
        let mut run = 0;
        while !self.dirty.is_empty() || self.pending_modules() > 0 {
            let evaluation = self.snapshot().evaluate(engine);
            run += evaluation.outcomes.len();
            let applied = self.apply(evaluation);
            if applied.applied.is_empty() && self.pending_modules() == 0 {
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
        self.relower_if_schema(sheet);
        self.version += 1;
        self.revision += 1;
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
