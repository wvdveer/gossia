//! The expressions on the controls of forms (DESIGN.md, "Forms", "Expressions
//! on controls"): *visible when*, *enabled when*, the text and fill
//! colours, and the *choices* of a dropdown or typeahead. They run as formulas do, on the evaluation thread, at the node the
//! control is bound to (the root for a control that is not bound), and what
//! they read is recorded so that an edit there runs them again. Their results
//! are kept here, in memory: they are not data, and they are not saved.
//!
//! The columns of a repeater's table have them too. A column's *visible
//! when* runs once, at the array; the others run for each row, at the cell,
//! so their list follows the number of items. So do the controls in the
//! design of a stacked repeater's items: each of their expressions runs for
//! each item, at the control's node in that item (at the item, for a control
//! that is not bound).

use cliotree_core::form::{ColumnDesign, Control, ControlKind, RepeaterMode};
use cliotree_core::{Kind, Path, Value};
use indexmap::IndexMap;

use super::*;
use crate::model::Dep;

/// Which expression of a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExprKind {
    Visible,
    Enabled,
    TextColour,
    FillColour,
    /// The acceptable values of a field (DESIGN.md, "Forms", "Choices").
    Choices,
}

impl ExprKind {
    pub const ALL: [ExprKind; 5] = [
        ExprKind::Visible,
        ExprKind::Enabled,
        ExprKind::TextColour,
        ExprKind::FillColour,
        ExprKind::Choices,
    ];

    fn text(self, control: &Control) -> Option<&str> {
        match self {
            ExprKind::Visible => control.visible_when.as_deref(),
            ExprKind::Enabled => control.enabled_when.as_deref(),
            ExprKind::TextColour => control.text_colour.as_deref(),
            ExprKind::FillColour => control.fill_colour.as_deref(),
            ExprKind::Choices => control.choices.as_deref(),
        }
        .map(str::trim)
        .filter(|t| !t.is_empty())
    }

    fn column_text(self, column: &ColumnDesign) -> Option<&str> {
        match self {
            ExprKind::Visible => column.visible_when.as_deref(),
            ExprKind::Enabled => column.enabled_when.as_deref(),
            ExprKind::TextColour => column.text_colour.as_deref(),
            ExprKind::FillColour => column.fill_colour.as_deref(),
            ExprKind::Choices => column.choices.as_deref(),
        }
        .map(str::trim)
        .filter(|t| !t.is_empty())
    }

    /// Whether, on a column, it runs for each row rather than once.
    fn per_row(self) -> bool {
        self != ExprKind::Visible
    }
}

/// One expression of one control of one form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprKey {
    pub tab: String,
    pub form: String,
    /// Where the control is in the design.
    pub at: Vec<usize>,
    /// For a column of the control's table: its name, and for an expression
    /// that runs for each row, the row.
    pub column: Option<String>,
    pub row: Option<usize>,
    pub kind: ExprKind,
}

/// What an expression came to, as the form draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprState {
    /// The latest result, or why there is none: the error in words. None
    /// before it has first run.
    pub result: Option<Result<Value, String>>,
    /// Something it read has changed since: the result is the last one.
    pub pending: bool,
}

#[derive(Debug, Clone)]
struct ControlExpr {
    text: String,
    loc: Loc,
    result: Option<Result<Value, String>>,
    deps: Vec<Dep>,
    stale: bool,
    /// It does not parse, so it never runs.
    syntax_error: bool,
}

/// An expression to run, as a snapshot carries it.
#[derive(Debug, Clone)]
pub(crate) struct ExprJob {
    pub key: ExprKey,
    pub text: String,
    pub loc: Loc,
}

/// What running one came to.
#[derive(Debug, Clone)]
pub(crate) struct ExprOutcome {
    pub key: ExprKey,
    pub text: String,
    pub result: Result<Value, String>,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ControlExprs {
    by_key: IndexMap<ExprKey, ControlExpr>,
    /// The designs may have changed since the list was made.
    pub resync: bool,
    /// The generation of the modules when they were last loaded.
    pub modules_seen: u64,
    /// Some column has an expression for each row, so a change to the data
    /// may change the list.
    pub has_rows: bool,
}

impl ControlExprs {
    pub fn stale_jobs(&self) -> Vec<ExprJob> {
        self.by_key
            .iter()
            .filter(|(_, e)| e.stale)
            .map(|(key, e)| ExprJob {
                key: key.clone(),
                text: e.text.clone(),
                loc: e.loc.clone(),
            })
            .collect()
    }

    /// Marks out of date every expression that read something an edit at
    /// `path` changes: for a full read, anything at, above or below it; for a
    /// read of the structure only, a change at or above the node.
    pub fn mark_stale(&mut self, sheet: &str, path: &Path) {
        for e in self.by_key.values_mut() {
            if e.stale {
                continue;
            }
            let touched = e.deps.iter().any(|dep| {
                dep.loc.sheet == sheet
                    && if dep.shallow {
                        dep.loc.path.segments().starts_with(path.segments())
                    } else {
                        overlaps(path, &dep.loc.path)
                    }
            });
            if touched {
                e.stale = true;
            }
        }
    }

    /// The expressions at `loc` that were waiting to run fail with
    /// `message`: one of them did not stop and its thread was given up.
    pub fn fail_stale_at(&mut self, loc: &Loc, message: &str) {
        for e in self.by_key.values_mut() {
            if e.stale && &e.loc == loc {
                e.result = Some(Err(format!("#TIMEOUT!: {message}")));
                e.stale = false;
            }
        }
    }

    pub fn mark_all_stale(&mut self) {
        for e in self.by_key.values_mut() {
            e.stale = !e.syntax_error;
        }
    }
}

impl Calc {
    /// Makes the list of expressions match the stored designs. An expression
    /// that is new, or whose text or node changed, is out of date; one that
    /// does not parse has its syntax error as its result and does not run.
    pub fn sync_expressions(&mut self) {
        if !self.exprs.resync {
            return;
        }
        self.exprs.resync = false;
        let mut wanted: Vec<(ExprKey, String, Loc)> = Vec::new();
        let mut has_rows = false;
        for (tab, form, design) in self.forms.iter() {
            // Each control, with the array of the stacked repeater whose
            // items it is designed in.
            let mut stack: Vec<(Vec<usize>, &Control, Option<Path>)> = design
                .controls
                .iter()
                .enumerate()
                .map(|(i, c)| (vec![i], c, None))
                .collect();
            while let Some((at, control, scope)) = stack.pop() {
                let key = |kind, column: Option<&str>, row| ExprKey {
                    tab: tab.to_owned(),
                    form: form.to_owned(),
                    at: at.clone(),
                    column: column.map(str::to_owned),
                    row,
                    kind,
                };
                let texts: Vec<(ExprKind, &str)> = ExprKind::ALL
                    .into_iter()
                    .filter_map(|k| Some((k, k.text(control)?)))
                    .collect();
                match &scope {
                    None => {
                        let path = control.bound_path().unwrap_or_default();
                        for (kind, text) in &texts {
                            wanted.push((
                                key(*kind, None, None),
                                (*text).to_owned(),
                                Loc::new(tab, path.clone()),
                            ));
                        }
                    }
                    // In the items of a stacked repeater: for each item.
                    Some(array) if !texts.is_empty() => {
                        has_rows = true;
                        let len = self
                            .data(tab)
                            .and_then(|d| d.get(array))
                            .and_then(|n| n.value().as_array())
                            .map_or(0, <[_]>::len);
                        for row in 0..len {
                            let item = array.index(row);
                            let path = match control.bound_path() {
                                Some(rest) => item.join(&rest),
                                None => item,
                            };
                            for (kind, text) in &texts {
                                wanted.push((
                                    key(*kind, None, Some(row)),
                                    (*text).to_owned(),
                                    Loc::new(tab, path.clone()),
                                ));
                            }
                        }
                    }
                    Some(_) => {}
                }
                // The columns of its table. (Not of a table in an item.)
                if scope.is_none()
                    && control.kind == ControlKind::Repeater
                    && control.mode != Some(RepeaterMode::Stacked)
                    && let Some(array) = control.bound_path()
                {
                    for column in &control.table_columns {
                        if column.hidden || column.is_row_number() {
                            continue;
                        }
                        let name = Some(column.name.as_str());
                        if let Some(text) = ExprKind::Visible.column_text(column) {
                            wanted.push((
                                key(ExprKind::Visible, name, None),
                                text.to_owned(),
                                Loc::new(tab, array.clone()),
                            ));
                        }
                        let each: Vec<(ExprKind, &str)> = ExprKind::ALL
                            .into_iter()
                            .filter(|k| k.per_row())
                            .filter_map(|k| Some((k, k.column_text(column)?)))
                            .collect();
                        if each.is_empty() {
                            continue;
                        }
                        has_rows = true;
                        let Some(cells) = self.column_cells(tab, &array, &column.name) else {
                            continue;
                        };
                        for (row, cell) in cells.into_iter().enumerate() {
                            for (kind, text) in &each {
                                wanted.push((
                                    key(*kind, name, Some(row)),
                                    (*text).to_owned(),
                                    Loc::new(tab, cell.clone()),
                                ));
                            }
                        }
                    }
                }
                // The controls in it: a group's are where the group is; a
                // stacked repeater's are the design of its items. Those of a
                // table, and of a repeater in an item, are not used.
                let inner = match (&scope, control.kind) {
                    (Some(_), ControlKind::Repeater) => continue,
                    (Some(_), _) => scope.clone(),
                    (None, ControlKind::Repeater) if control.is_stacked() => {
                        match control.bound_path() {
                            Some(array) => Some(array),
                            None => continue,
                        }
                    }
                    (None, ControlKind::Repeater) => continue,
                    (None, _) => None,
                };
                for (i, child) in control.children.iter().enumerate() {
                    let mut child_at = at.clone();
                    child_at.push(i);
                    stack.push((child_at, child, inner.clone()));
                }
            }
        }
        let mut old = std::mem::take(&mut self.exprs.by_key);
        for (key, text, loc) in wanted {
            let kept = old
                .swap_remove(&key)
                .filter(|e| e.text == text && e.loc == loc);
            let expr = kept.unwrap_or_else(|| match self.syntax.check(&text) {
                Ok(()) => ControlExpr {
                    text,
                    loc,
                    result: None,
                    deps: Vec::new(),
                    stale: true,
                    syntax_error: false,
                },
                Err(e) => ControlExpr {
                    text,
                    loc,
                    result: Some(Err(format!("syntax error: {e}"))),
                    deps: Vec::new(),
                    stale: false,
                    syntax_error: true,
                },
            });
            self.exprs.by_key.insert(key, expr);
        }
        self.exprs.has_rows = has_rows;
    }

    /// The cells of a column of the table of the array at `array`, one for
    /// each item, as the form's table finds them: a column of items that are
    /// objects is a field, dotted for a field of a field; the one column of
    /// items that are not is the item. None if the column is not one of the
    /// items' type.
    fn column_cells(&self, tab: &str, array: &Path, name: &str) -> Option<Vec<Path>> {
        let spec = self.spec(tab)?;
        let Kind::Array { items, .. } = &spec.get(spec.resolve(array)?.ty).kind else {
            return None;
        };
        let fields: Vec<&str> = match &spec.get(*items).kind {
            Kind::Object { .. } => name.split('.').collect(),
            Kind::Array { .. } | Kind::Map { .. } => return None,
            _ if name == "value" => Vec::new(),
            _ => return None,
        };
        let cell = |i: usize| fields.iter().fold(array.index(i), |p, f| p.key(*f));
        spec.resolve(&cell(0))?;
        let len = self
            .data(tab)?
            .get(array)
            .and_then(|n| n.value().as_array())
            .map_or(0, |items| items.len());
        Some((0..len).map(cell).collect())
    }

    /// How many expressions on controls are out of date and will be run. None
    /// while the workbook is not trusted.
    pub fn pending_expressions(&self) -> usize {
        if !self.trusted {
            return 0;
        }
        self.exprs.by_key.values().filter(|e| e.stale).count()
    }

    /// What an expression of a control came to. None if the control has no
    /// such expression (or the designs changed and have not been looked at
    /// again yet: [`Calc::sync_expressions`]).
    pub fn control_expression(
        &self,
        tab: &str,
        form: &str,
        at: &[usize],
        kind: ExprKind,
    ) -> Option<ExprState> {
        self.control_expression_in(tab, form, at, None, kind)
    }

    /// As [`Calc::control_expression`], for a control in the design of a
    /// stacked repeater's items: what it came to for item `item`.
    pub fn control_expression_in(
        &self,
        tab: &str,
        form: &str,
        at: &[usize],
        item: Option<usize>,
        kind: ExprKind,
    ) -> Option<ExprState> {
        self.expression_state(&ExprKey {
            tab: tab.to_owned(),
            form: form.to_owned(),
            at: at.to_vec(),
            column: None,
            row: item,
            kind,
        })
    }

    /// What an expression of a column of the table of the control at `at`
    /// came to: *visible when* with no row, the others for the row.
    pub fn column_expression(
        &self,
        tab: &str,
        form: &str,
        at: &[usize],
        column: &str,
        row: Option<usize>,
        kind: ExprKind,
    ) -> Option<ExprState> {
        self.expression_state(&ExprKey {
            tab: tab.to_owned(),
            form: form.to_owned(),
            at: at.to_vec(),
            column: Some(column.to_owned()),
            row: if kind.per_row() { row } else { None },
            kind,
        })
    }

    fn expression_state(&self, key: &ExprKey) -> Option<ExprState> {
        let e = self.exprs.by_key.get(key)?;
        Some(ExprState {
            result: e.result.clone(),
            pending: e.stale && self.trusted,
        })
    }

    /// Takes what the expressions of an evaluation came to. One is dropped,
    /// and stays out of date, if an edit made after the snapshot touched what
    /// it read, or it read the result of a formula that was dropped or is
    /// still out of date. Returns whether any result was taken.
    pub(super) fn apply_expr_outcomes(
        &mut self,
        base: u64,
        outcomes: Vec<ExprOutcome>,
        dropped: &HashMap<String, PathTrie<Loc>>,
    ) -> bool {
        let mut any = false;
        for outcome in outcomes {
            let overtaken = outcome.deps.iter().any(|dep| {
                self.edited_since(base, &dep.loc, dep.shallow)
                    || dropped.get(&dep.loc.sheet).is_some_and(|t| {
                        if dep.shallow {
                            t.any_at_or_above(dep.loc.path.segments())
                        } else {
                            t.any_related(dep.loc.path.segments())
                        }
                    })
                    || self
                        .dirty
                        .iter()
                        .any(|d| d.sheet == dep.loc.sheet && overlaps(&d.path, &dep.loc.path))
            });
            let Some(e) = self.exprs.by_key.get_mut(&outcome.key) else {
                continue;
            };
            if overtaken || e.text != outcome.text {
                continue;
            }
            e.result = Some(outcome.result);
            e.deps = outcome.deps;
            e.stale = false;
            any = true;
        }
        any
    }
}
