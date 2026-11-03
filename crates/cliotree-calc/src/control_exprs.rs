//! The expressions on the controls of forms (DESIGN.md, "Forms", "Expressions
//! on controls"): *visible when*, *enabled when*, the text and fill colours,
//! the *choices* of a dropdown or typeahead, and (from 0.2.0, "Additions in
//! 0.2.0, shared with reports") the value a field shows, the template of a
//! label and the items of a repeater. They run as formulas do, on the
//! evaluation thread, at the node the control is bound to (the root for a
//! control that is not bound), and what they read is recorded so that an
//! edit there runs them again. Their results are kept here, in memory: they
//! are not data, and they are not saved.
//!
//! The columns of a repeater's table have them too. A column's *visible
//! when* runs once, at the array; the others run for each row, at the cell,
//! so their list follows the number of items. So do the controls in the
//! design of a stacked repeater's items: each of their expressions runs for
//! each item, at the control's node in that item (at the item, for a control
//! that is not bound). Designed items nest: a control inside the items of a
//! repeater inside the items of another runs for each pair of items, and so on.
//!
//! A repeater's items can come from an expression instead of an array of the
//! tab. The controls inside it then have no node: their expressions are given
//! the item as `this`, and under the repeater's item name and list name if it
//! has them (as `order` and `rows`), and run for each item. The items are
//! known only once the expression has run, so the expressions inside follow
//! its result.
//!
//! Reports (DESIGN.md, "Reports") have the same expressions on the same
//! controls, keyed by the report's name and [`REPORT_FORM`]. They have no node:
//! they run at the root of a tab that does not exist, with `this`, `parent`
//! and `root` bound to `None` outside the items of a repeater. A report's
//! expressions run only while it is shown ([`Calc::show_report`]).
//!
//! The bands of a multipage report (DESIGN.md, Reports, "Multipage") run
//! once for each place they are drawn: a section's header and footer for
//! each item of the section, keyed by the item of each section around and
//! its own; the detail band for each detail item, with the detail item last;
//! the page header and footer for each page, keyed by the page, with `page`
//! (`page.number`, `page.count`); the report header and footer once. The
//! items of a section are an expression on its header band, run in each
//! occurrence of the section around it; the bands inside see each item by
//! the section's item name and its list by its list name, and so do the
//! header and footer of the section around. The pages come from
//! [`cliotree_core::report::paginate`], as the layout's do.

use cliotree_core::form::{ColumnDesign, Control, ControlKind, fixed_colour, template, truthy};
use cliotree_core::report::{BandKind, ReportDesign, paginate};
use cliotree_core::{Kind, Node, Path, Value};
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
    /// What a field, or the cells of a column, show instead of a node.
    Value,
    /// The text of a label that is a template.
    Template,
    /// The items of a repeater that come from an expression.
    Items,
    /// The SVG of a chart: its chart function called with the expression of
    /// each parameter (DESIGN.md, Reports, "Chart functions").
    Chart,
}

impl ExprKind {
    pub const ALL: [ExprKind; 9] = [
        ExprKind::Visible,
        ExprKind::Enabled,
        ExprKind::TextColour,
        ExprKind::FillColour,
        ExprKind::Choices,
        ExprKind::Value,
        ExprKind::Template,
        ExprKind::Items,
        ExprKind::Chart,
    ];

    /// The expression of this kind on a control: its Python text, or why
    /// there is none that can run (a template that cannot be read).
    fn text(self, control: &Control) -> Option<Result<String, String>> {
        let plain = match self {
            ExprKind::Visible => control.visible_when.as_deref(),
            ExprKind::Enabled => control.enabled_when.as_deref(),
            ExprKind::TextColour => control
                .text_colour
                .as_deref()
                .filter(|t| fixed_colour(t).is_none()),
            ExprKind::FillColour => control
                .fill_colour
                .as_deref()
                .filter(|t| fixed_colour(t).is_none()),
            ExprKind::Choices => control.choices.as_deref(),
            ExprKind::Value => control
                .shows_value()
                .then_some(control.value.as_deref())
                .flatten(),
            ExprKind::Items => control
                .lists_items()
                .then_some(control.items.as_deref())
                .flatten(),
            ExprKind::Template => {
                if !control.is_template() {
                    return None;
                }
                let text = control.text.as_deref().unwrap_or_default();
                // A template with no expression in it is fixed text.
                return match template::parts(text) {
                    Ok(parts) if parts.iter().all(|p| matches!(p, template::Part::Text(_))) => None,
                    Ok(_) => Some(
                        template::expression_of(text)
                            .map_err(|e| format!("the template cannot be read: {e}")),
                    ),
                    Err(e) => Some(Err(format!("the template cannot be read: {e}"))),
                };
            }
            // Made by `Calc::chart_text`, which knows the chart types.
            ExprKind::Chart => None,
        };
        plain
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| Ok(t.to_owned()))
    }

    fn column_text(self, column: &ColumnDesign) -> Option<&str> {
        match self {
            ExprKind::Visible => column.visible_when.as_deref(),
            ExprKind::Enabled => column.enabled_when.as_deref(),
            ExprKind::TextColour => column
                .text_colour
                .as_deref()
                .filter(|t| fixed_colour(t).is_none()),
            ExprKind::FillColour => column
                .fill_colour
                .as_deref()
                .filter(|t| fixed_colour(t).is_none()),
            ExprKind::Choices => column.choices.as_deref(),
            ExprKind::Value => column.value.as_deref(),
            ExprKind::Template | ExprKind::Items | ExprKind::Chart => None,
        }
        .map(str::trim)
        .filter(|t| !t.is_empty())
    }

    /// Whether, on a column, it runs for each row rather than once.
    pub fn per_row(self) -> bool {
        self != ExprKind::Visible
    }

    /// Whether it means anything on a read-only table, whose items an
    /// expression gave.
    fn on_listed_column(self) -> bool {
        matches!(
            self,
            ExprKind::Visible | ExprKind::TextColour | ExprKind::FillColour | ExprKind::Value
        )
    }
}

/// One expression of one control of one form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprKey {
    pub tab: String,
    pub form: String,
    /// Where the control is in the design.
    pub at: Vec<usize>,
    /// For a column of the control's table: its name.
    pub column: Option<String>,
    /// The item of each repeater around the control whose items are designed,
    /// outermost first, and for an expression of a column that runs for each
    /// row, the row last.
    pub rows: Vec<usize>,
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

/// Names an expression is given besides `this`, `parent` and `root`, with
/// their values.
pub(crate) type Bindings = Vec<(String, Value)>;

#[derive(Debug, Clone)]
struct ControlExpr {
    text: String,
    loc: Loc,
    bindings: Bindings,
    result: Option<Result<Value, String>>,
    deps: Vec<Dep>,
    stale: bool,
    /// It does not parse, so it never runs.
    syntax_error: bool,
}

/// The form name of the keys of probes, which no form can have.
const PROBE_FORM: &str = "\u{1}probe";

/// The form name of the keys of a report's expressions, which no form can
/// have; their tab is the report's name.
pub const REPORT_FORM: &str = "\u{1}report";

/// An expression to run, as a snapshot carries it.
#[derive(Debug, Clone)]
pub(crate) struct ExprJob {
    pub key: ExprKey,
    pub text: String,
    pub loc: Loc,
    pub bindings: Bindings,
    /// It runs as a formula on its node would (a probe of a formula, for
    /// Disconnect from Data Sources), not as an expression of a control.
    pub formula: bool,
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
    /// Expressions run once for Disconnect from Data Sources, each with
    /// whether it runs as a formula. They run with the next pass and are
    /// read back by the command.
    probes: IndexMap<ExprKey, (ControlExpr, bool)>,
    /// The report being shown, whose expressions run.
    pub shown_report: Option<String>,
}

impl ControlExprs {
    pub fn stale_jobs(&self) -> Vec<ExprJob> {
        let job = |key: &ExprKey, e: &ControlExpr, formula: bool| ExprJob {
            key: key.clone(),
            text: e.text.clone(),
            loc: e.loc.clone(),
            bindings: e.bindings.clone(),
            formula,
        };
        self.by_key
            .iter()
            .filter(|(_, e)| e.stale)
            .map(|(key, e)| job(key, e, false))
            .chain(
                self.probes
                    .iter()
                    .filter(|(_, (e, _))| e.stale)
                    .map(|(key, (e, formula))| job(key, e, *formula)),
            )
            .collect()
    }

    /// Puts the probes of Disconnect from Data Sources in place of any there
    /// were: each an expression, where it runs, the names it sees, and
    /// whether it runs as a formula.
    pub(crate) fn set_probes(&mut self, probes: Vec<(String, Loc, Bindings, bool)>) {
        self.probes = probes
            .into_iter()
            .enumerate()
            .map(|(i, (text, loc, bindings, formula))| {
                let key = ExprKey {
                    tab: String::new(),
                    form: PROBE_FORM.to_owned(),
                    at: vec![i],
                    column: None,
                    rows: Vec::new(),
                    kind: ExprKind::Value,
                };
                let expr = ControlExpr {
                    text,
                    loc,
                    bindings,
                    result: None,
                    deps: Vec::new(),
                    stale: true,
                    syntax_error: false,
                };
                (key, (expr, formula))
            })
            .collect();
    }

    /// What the probes came to, in their order, once every one has run.
    pub(crate) fn probe_results(&self) -> Option<Vec<Result<Value, String>>> {
        self.probes
            .values()
            .map(|(e, _)| e.result.clone().filter(|_| !e.stale))
            .collect()
    }

    pub(crate) fn clear_probes(&mut self) {
        self.probes.clear();
    }

    /// Every expression of a control (or of a column of its table), keyed
    /// with no rows, with where each instance of it runs: its rows, its node
    /// and its names, in the order of their items.
    pub(crate) fn instance_groups(&self) -> IndexMap<ExprKey, Vec<(Vec<usize>, Loc, Bindings)>> {
        let mut groups: IndexMap<ExprKey, Vec<(Vec<usize>, Loc, Bindings)>> = IndexMap::new();
        for (key, e) in &self.by_key {
            let mut general = key.clone();
            let rows = std::mem::take(&mut general.rows);
            groups
                .entry(general)
                .or_default()
                .push((rows, e.loc.clone(), e.bindings.clone()));
        }
        for instances in groups.values_mut() {
            instances.sort_by(|a, b| a.0.cmp(&b.0));
        }
        groups
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

    /// Every expression with its result, None before it has first run.
    pub(crate) fn entries(
        &self,
    ) -> impl Iterator<Item = (&ExprKey, Option<Result<Value, String>>)> {
        self.by_key.iter().map(|(k, e)| {
            let result = if e.stale && !e.syntax_error {
                None
            } else {
                e.result.clone()
            };
            (k, result)
        })
    }

    pub fn mark_all_stale(&mut self) {
        for e in self.by_key.values_mut() {
            e.stale = !e.syntax_error;
        }
    }
}

/// Where a control's expressions run: once at the top of a form, or once for
/// each item of each repeater around it whose items are designed.
#[derive(Debug, Clone, Default)]
struct Instance {
    /// The item of each repeater around, outermost first.
    rows: Vec<usize>,
    /// The node of the innermost item, when it is an item of an array of the
    /// tab; the paths of the controls in it are relative to it.
    base: Option<Path>,
    /// The names the repeaters around give their items and lists, and `this`
    /// for an item that an expression gave.
    bindings: Bindings,
}

/// Makes the key of an expression of the control being synced: its kind,
/// its column, and the items it is for.
type KeyOf<'a> = dyn Fn(ExprKind, Option<&str>, Vec<usize>) -> ExprKey + 'a;

/// Controls whose expressions are synced, each with its address and the
/// instances it runs in.
type Seeds<'a> = Vec<(Vec<usize>, &'a Control, Vec<Instance>)>;

/// The controls at the top level of a design, each run once in `top`.
fn seed<'a>(controls: &'a [Control], top: &Instance) -> Seeds<'a> {
    controls
        .iter()
        .enumerate()
        .map(|(i, c)| (vec![i], c, vec![top.clone()]))
        .collect()
}

/// What working out the occurrences of the bands of a multipage report
/// gathers: the instances of each band, how many items each section has in
/// each occurrence (by its depth and the occurrence), and the detail items
/// so far.
struct BandWalk {
    per_band: Vec<Vec<Instance>>,
    counts: HashMap<(usize, Vec<usize>), usize>,
    details: usize,
}

/// The value of `page` in a page header or footer.
fn page_value(index: usize, count: usize) -> Value {
    let mut fields = IndexMap::new();
    fields.insert("number".to_owned(), Node::new(Value::Int(index as i64 + 1)));
    fields.insert("count".to_owned(), Node::new(Value::Int(count as i64)));
    Value::Object(fields)
}

fn name_of(n: &Option<String>) -> Option<&str> {
    n.as_deref().map(str::trim).filter(|n| !n.is_empty())
}

/// An expression the designs ask for.
struct Wanted {
    key: ExprKey,
    text: Result<String, String>,
    loc: Loc,
    bindings: Bindings,
}

/// The bindings of an instance with more names, a later one of the same name
/// in place of an earlier one.
fn bind(base: &Bindings, more: impl IntoIterator<Item = (String, Value)>) -> Bindings {
    let mut out = base.clone();
    for (name, value) in more {
        out.retain(|(n, _)| *n != name);
        out.push((name, value));
    }
    out
}

/// The names an expression is given: all of them, but `params` only if it
/// names it, so that a change to a parameter runs only those that use one.
fn fit(bindings: &Bindings, text: &str) -> Bindings {
    if bindings.iter().any(|(n, _)| n == "params") && !mentions_params(text) {
        bindings
            .iter()
            .filter(|(n, _)| n != "params")
            .cloned()
            .collect()
    } else {
        bindings.clone()
    }
}

/// The names an item of a repeater whose items come from an expression is
/// given: `this`, and the repeater's item and list names.
fn item_names(control: &Control, item: &Value, list: &Value) -> Vec<(String, Value)> {
    let mut names = vec![("this".to_owned(), item.clone())];
    fn name(n: &Option<String>) -> Option<&str> {
        n.as_deref().map(str::trim).filter(|n| !n.is_empty())
    }
    if let Some(n) = name(&control.item_name) {
        names.push((n.to_owned(), item.clone()));
    }
    if let Some(n) = name(&control.list_name) {
        names.push((n.to_owned(), list.clone()));
    }
    names
}

/// What a cell of a column of an item shows when it has no expression: the
/// field of that name (dotted for a field of a field), or the item itself for
/// the one column of items that are not objects.
pub fn cell_of(item: &Value, column: &str) -> Option<Value> {
    match item {
        Value::Object(_) => {
            let mut at = item.clone();
            for part in column.split('.') {
                let Value::Object(fields) = &at else {
                    return None;
                };
                at = fields.get(part)?.value().clone();
            }
            Some(at)
        }
        _ if column == "value" => Some(item.clone()),
        _ => None,
    }
}

impl Calc {
    /// Makes the list of expressions match the stored designs. An expression
    /// that is new, or whose text, node or names changed, is out of date; one
    /// that does not parse has its syntax error as its result and does not run.
    pub fn sync_expressions(&mut self) {
        if !self.exprs.resync {
            return;
        }
        self.exprs.resync = false;
        let mut wanted: Vec<Wanted> = Vec::new();
        let mut has_rows = false;
        // The forms, then the report being shown, whose expressions have no
        // node: `this`, `parent` and `root` are None.
        let mut designs: Vec<(&str, &str, Seeds)> = self
            .forms
            .iter()
            .map(|(tab, form, design)| (tab, form, seed(&design.controls, &Instance::default())))
            .collect();
        if let Some(name) = &self.exprs.shown_report
            && let Some(report) = self.reports.get(name)
        {
            let mut top = Instance {
                bindings: ["this", "parent", "root"]
                    .into_iter()
                    .map(|n| (n.to_owned(), Value::Null))
                    .collect(),
                ..Instance::default()
            };
            // The defaults and lists of its parameters, which do not see
            // `params`.
            for p in &report.params {
                let wants = [
                    (ExprKind::Value, p.default_expression()),
                    (ExprKind::Choices, p.choices_expression()),
                ];
                for (kind, text) in wants {
                    if let Some(text) = text {
                        wanted.push(Wanted {
                            key: param_key(name, &p.name, kind),
                            text: Ok(text.to_owned()),
                            loc: Loc::new(name, Path::root()),
                            bindings: top.bindings.clone(),
                        });
                    }
                }
            }
            if !report.params.is_empty() {
                top.bindings
                    .push(("params".to_owned(), self.params_value(name)));
            }
            // While a required parameter is empty nothing else runs.
            let missing = !self.missing_params(name).is_empty();
            let seeds = match report.bands() {
                _ if missing => Vec::new(),
                Some(sections) => {
                    has_rows = true;
                    self.band_seeds(name, report, sections, top, &mut wanted)
                }
                None => seed(&report.layout.controls, &top),
            };
            designs.push((name.as_str(), REPORT_FORM, seeds));
        }
        for (tab, form, seeds) in designs {
            let mut stack = seeds;
            while let Some((at, control, instances)) = stack.pop() {
                let key = |kind, column: Option<&str>, rows: Vec<usize>| ExprKey {
                    tab: tab.to_owned(),
                    form: form.to_owned(),
                    at: at.clone(),
                    column: column.map(str::to_owned),
                    rows,
                    kind,
                };
                let texts: Vec<(ExprKind, Result<String, String>)> = ExprKind::ALL
                    .into_iter()
                    .filter_map(|k| Some((k, k.text(control)?)))
                    .chain(self.chart_text(control).map(|t| (ExprKind::Chart, t)))
                    .collect();
                let own_path = (!control.shows_value() && !control.lists_items())
                    .then(|| control.bound_path())
                    .flatten();
                let mut inner: Vec<Instance> = Vec::new();
                for inst in &instances {
                    let path = match (&own_path, &inst.base) {
                        (Some(rest), Some(base)) => base.join(rest),
                        (Some(rest), None) => rest.clone(),
                        (None, Some(base)) => base.clone(),
                        (None, None) => Path::root(),
                    };
                    let loc = Loc::new(tab, path.clone());
                    has_rows |= !inst.rows.is_empty();
                    for (kind, text) in &texts {
                        wanted.push(Wanted {
                            key: key(*kind, None, inst.rows.clone()),
                            text: text.clone(),
                            loc: loc.clone(),
                            bindings: inst.bindings.clone(),
                        });
                    }
                    let at_top = inst.rows.is_empty() && inst.base.is_none();
                    match control.kind {
                        ControlKind::Repeater if control.lists_items() => {
                            let Some(Ok(text)) = ExprKind::Items.text(control) else {
                                continue;
                            };
                            let items_key = key(ExprKind::Items, None, inst.rows.clone());
                            let Some(list) = self.listed(&items_key, &text, &loc, &inst.bindings)
                            else {
                                continue;
                            };
                            let Value::Array(items) = &list else {
                                continue;
                            };
                            if control.is_stacked() {
                                for (i, item) in items.iter().enumerate() {
                                    let mut rows = inst.rows.clone();
                                    rows.push(i);
                                    inner.push(Instance {
                                        rows,
                                        base: None,
                                        bindings: bind(
                                            &inst.bindings,
                                            item_names(control, item.value(), &list),
                                        ),
                                    });
                                }
                            } else {
                                self.listed_columns(
                                    control,
                                    &key,
                                    inst,
                                    &loc,
                                    items,
                                    &list,
                                    &mut wanted,
                                );
                            }
                        }
                        ControlKind::Repeater if control.is_stacked() => {
                            let Some(array) = own_path.as_ref().map(|_| path.clone()) else {
                                continue;
                            };
                            let len = self.array_len(tab, &array);
                            has_rows = true;
                            for i in 0..len {
                                let mut rows = inst.rows.clone();
                                rows.push(i);
                                inner.push(Instance {
                                    rows,
                                    base: Some(array.index(i)),
                                    bindings: inst.bindings.clone(),
                                });
                            }
                        }
                        // The columns of a table bound to an array. (Not of
                        // a table in an item.)
                        ControlKind::Repeater if at_top => {
                            if let Some(array) = &own_path {
                                has_rows |=
                                    self.path_columns(control, &key, tab, array, &mut wanted);
                            }
                        }
                        ControlKind::Repeater => {}
                        _ => inner.push(inst.clone()),
                    }
                }
                // The controls in it: a group's are where the group is; a
                // stacked repeater's are the design of its items, for each
                // item. Those of a table are not used.
                if inner.is_empty() {
                    continue;
                }
                for (i, child) in control.children.iter().enumerate() {
                    let mut child_at = at.clone();
                    child_at.push(i);
                    stack.push((child_at, child, inner.clone()));
                }
            }
        }
        let mut old = std::mem::take(&mut self.exprs.by_key);
        for mut w in wanted {
            let text_now = match &w.text {
                Ok(t) => t.clone(),
                Err(e) => e.clone(),
            };
            w.bindings = fit(&w.bindings, &text_now);
            let kept = old
                .swap_remove(&w.key)
                .filter(|e| e.text == text_now && e.loc == w.loc && e.bindings == w.bindings);
            let expr = kept.unwrap_or_else(|| {
                let checked = match &w.text {
                    Ok(text) => self
                        .syntax
                        .check(text)
                        .map_err(|e| format!("syntax error: {e}")),
                    Err(problem) => Err(problem.clone()),
                };
                match checked {
                    Ok(()) => ControlExpr {
                        text: text_now,
                        loc: w.loc,
                        bindings: w.bindings,
                        result: None,
                        deps: Vec::new(),
                        stale: true,
                        syntax_error: false,
                    },
                    Err(message) => ControlExpr {
                        text: text_now,
                        loc: w.loc,
                        bindings: w.bindings,
                        result: Some(Err(message)),
                        deps: Vec::new(),
                        stale: false,
                        syntax_error: true,
                    },
                }
            });
            self.exprs.by_key.insert(w.key, expr);
        }
        self.exprs.has_rows = has_rows;
    }

    /// The bands of a multipage report with `sections` sections, each with
    /// the instances it runs in (see the module's notes), and the items
    /// expressions of its sections and detail band in `wanted`.
    fn band_seeds<'a>(
        &self,
        name: &str,
        report: &'a ReportDesign,
        sections: usize,
        top: Instance,
        wanted: &mut Vec<Wanted>,
    ) -> Seeds<'a> {
        let layout = &report.layout;
        let mut walk = BandWalk {
            per_band: vec![Vec::new(); layout.controls.len()],
            counts: HashMap::new(),
            details: 0,
        };
        let top = self.band_level(name, report, sections, 0, &top, &mut walk, wanted);
        for band in [BandKind::ReportHeader, BandKind::ReportFooter] {
            walk.per_band[band.index(sections)].push(top.clone());
        }
        // The pages, from the items and the bands that are not visible.
        let visible = |band: usize, rows: &[usize]| {
            let key = ExprKey {
                tab: name.to_owned(),
                form: REPORT_FORM.to_owned(),
                at: vec![band],
                column: None,
                rows: rows.to_vec(),
                kind: ExprKind::Visible,
            };
            match self.exprs.by_key.get(&key).and_then(|e| e.result.as_ref()) {
                Some(Ok(value)) => truthy(value),
                _ => true,
            }
        };
        let items = |level: usize, rows: &[usize]| {
            walk.counts
                .get(&(level, rows.to_vec()))
                .copied()
                .unwrap_or(0)
        };
        let pages = paginate(report, &items, &visible, self.detail_limit())
            .pages
            .len();
        for p in 0..pages {
            let instance = Instance {
                rows: vec![p],
                base: None,
                bindings: bind(&top.bindings, [("page".to_owned(), page_value(p, pages))]),
            };
            for band in [BandKind::PageHeader, BandKind::PageFooter] {
                walk.per_band[band.index(sections)].push(instance.clone());
            }
        }
        layout
            .controls
            .iter()
            .zip(walk.per_band)
            .enumerate()
            .map(|(i, (c, instances))| (vec![i], c, instances))
            .collect()
    }

    /// The occurrences of the section at depth `level` (the detail band,
    /// after the last section) in the occurrence `outer` of the sections
    /// around it. Returns `outer` with the list of this level under its list
    /// name, which the header and footer around it see.
    #[allow(clippy::too_many_arguments)]
    fn band_level(
        &self,
        name: &str,
        report: &ReportDesign,
        sections: usize,
        level: usize,
        outer: &Instance,
        walk: &mut BandWalk,
        wanted: &mut Vec<Wanted>,
    ) -> Instance {
        let band = if level < sections {
            BandKind::SectionHeader(level).index(sections)
        } else {
            BandKind::Detail.index(sections)
        };
        let control = &report.layout.controls[band];
        let Some(text) = name_of(&control.items) else {
            return outer.clone();
        };
        let loc = Loc::new(name, Path::root());
        let key = ExprKey {
            tab: name.to_owned(),
            form: REPORT_FORM.to_owned(),
            at: vec![band],
            column: None,
            rows: outer.rows.clone(),
            kind: ExprKind::Items,
        };
        wanted.push(Wanted {
            key: key.clone(),
            text: Ok(text.to_owned()),
            loc: loc.clone(),
            bindings: outer.bindings.clone(),
        });
        let Some(list) = self.listed(&key, text, &loc, &outer.bindings) else {
            return outer.clone();
        };
        let Value::Array(items) = &list else {
            return outer.clone();
        };
        let listed = name_of(&control.list_name).map(|n| (n.to_owned(), list.clone()));
        let around = Instance {
            bindings: bind(&outer.bindings, listed),
            ..outer.clone()
        };
        let mut count = 0;
        for (i, item) in items.iter().enumerate() {
            if walk.details >= self.detail_limit() {
                break;
            }
            count += 1;
            let mut rows = outer.rows.clone();
            rows.push(i);
            let child = Instance {
                rows,
                base: None,
                bindings: bind(&around.bindings, item_names(control, item.value(), &list)),
            };
            if level == sections {
                walk.details += 1;
                walk.per_band[band].push(child);
            } else {
                let child =
                    self.band_level(name, report, sections, level + 1, &child, walk, wanted);
                let footer = BandKind::SectionFooter(level).index(sections);
                walk.per_band[band].push(child.clone());
                walk.per_band[footer].push(child);
            }
        }
        walk.counts.insert((level, outer.rows.clone()), count);
        around
    }

    /// The list a repeater's items expression came to, if it has run with
    /// this text, at this node and with these names.
    fn listed(&self, key: &ExprKey, text: &str, loc: &Loc, bindings: &Bindings) -> Option<Value> {
        let e = self.exprs.by_key.get(key)?;
        if e.text != text || &e.loc != loc || e.bindings != fit(bindings, text) {
            return None;
        }
        match &e.result {
            Some(Ok(list @ Value::Array(_))) => Some(list.clone()),
            _ => None,
        }
    }

    fn array_len(&self, tab: &str, array: &Path) -> usize {
        self.data(tab)
            .and_then(|d| d.get(array))
            .and_then(|n| n.value().as_array())
            .map_or(0, <[_]>::len)
    }

    /// The expressions of the columns of a table bound to an array: *visible
    /// when* once at the array, the others for each row at the cell (at the
    /// item, for a column that shows an expression). Returns whether any runs
    /// for each row.
    fn path_columns(
        &self,
        control: &Control,
        key: &KeyOf,
        tab: &str,
        array: &Path,
        wanted: &mut Vec<Wanted>,
    ) -> bool {
        let mut each_row = false;
        for column in &control.table_columns {
            if column.hidden || column.is_row_number() {
                continue;
            }
            let name = Some(column.name.as_str());
            if let Some(text) = ExprKind::Visible.column_text(column) {
                wanted.push(Wanted {
                    key: key(ExprKind::Visible, name, Vec::new()),
                    text: Ok(text.to_owned()),
                    loc: Loc::new(tab, array.clone()),
                    bindings: Vec::new(),
                });
            }
            let each: Vec<(ExprKind, &str)> = ExprKind::ALL
                .into_iter()
                .filter(|k| k.per_row())
                .filter_map(|k| Some((k, k.column_text(column)?)))
                .collect();
            if each.is_empty() {
                continue;
            }
            let cells = self.column_cells(tab, array, &column.name);
            // A column that is not a field of the items runs nothing, unless
            // it shows an expression.
            if cells.is_none() && column.value.is_none() {
                continue;
            }
            each_row = true;
            let len = self.array_len(tab, array);
            for row in 0..len {
                let cell = cells
                    .as_ref()
                    .and_then(|c| c.get(row).cloned())
                    .unwrap_or_else(|| array.index(row));
                for (kind, text) in &each {
                    // A column that shows an expression runs at the item.
                    let at = if column.value.is_some() {
                        array.index(row)
                    } else {
                        cell.clone()
                    };
                    wanted.push(Wanted {
                        key: key(*kind, name, vec![row]),
                        text: Ok((*text).to_owned()),
                        loc: Loc::new(tab, at),
                        bindings: Vec::new(),
                    });
                }
            }
        }
        each_row
    }

    /// The expressions of the columns of a table whose items an expression
    /// gave: *visible when* once, the others for each row, given the item
    /// under the repeater's names and the cell as `this`.
    #[allow(clippy::too_many_arguments)]
    fn listed_columns(
        &self,
        control: &Control,
        key: &KeyOf,
        inst: &Instance,
        loc: &Loc,
        items: &[Node],
        list: &Value,
        wanted: &mut Vec<Wanted>,
    ) {
        for column in &control.table_columns {
            if column.hidden || column.is_row_number() {
                continue;
            }
            let name = Some(column.name.as_str());
            if let Some(text) = ExprKind::Visible.column_text(column) {
                let mut names = Vec::new();
                if let Some(n) = control
                    .list_name
                    .as_deref()
                    .filter(|n| !n.trim().is_empty())
                {
                    names.push((n.trim().to_owned(), list.clone()));
                }
                wanted.push(Wanted {
                    key: key(ExprKind::Visible, name, inst.rows.clone()),
                    text: Ok(text.to_owned()),
                    loc: loc.clone(),
                    bindings: bind(&inst.bindings, names),
                });
            }
            let each: Vec<(ExprKind, &str)> = ExprKind::ALL
                .into_iter()
                .filter(|k| k.per_row() && k.on_listed_column())
                .filter_map(|k| Some((k, k.column_text(column)?)))
                .collect();
            if each.is_empty() {
                continue;
            }
            for (row, item) in items.iter().enumerate() {
                let item = item.value();
                let mut names = item_names(control, item, list);
                // In a cell, `this` is the cell when the item has one.
                if column.value.is_none()
                    && let Some(cell) = cell_of(item, &column.name)
                {
                    names[0].1 = cell;
                }
                let bindings = bind(&inst.bindings, names);
                let mut rows = inst.rows.clone();
                rows.push(row);
                for (kind, text) in &each {
                    wanted.push(Wanted {
                        key: key(*kind, name, rows.clone()),
                        text: Ok((*text).to_owned()),
                        loc: loc.clone(),
                        bindings: bindings.clone(),
                    });
                }
            }
        }
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
        let len = self.array_len(tab, array);
        Some((0..len).map(cell).collect())
    }

    /// How many expressions on controls are out of date and will be run. None
    /// while the workbook is not trusted.
    pub fn pending_expressions(&self) -> usize {
        if !self.trusted {
            return 0;
        }
        self.exprs.by_key.values().filter(|e| e.stale).count()
            + self.exprs.probes.values().filter(|(e, _)| e.stale).count()
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
        self.control_expression_at(tab, form, at, &[], kind)
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
        let rows: Vec<usize> = item.into_iter().collect();
        self.control_expression_at(tab, form, at, &rows, kind)
    }

    /// As [`Calc::control_expression`], for a control inside the designed
    /// items of repeaters: `rows` is the item of each, outermost first.
    pub fn control_expression_at(
        &self,
        tab: &str,
        form: &str,
        at: &[usize],
        rows: &[usize],
        kind: ExprKind,
    ) -> Option<ExprState> {
        self.expression_state(&ExprKey {
            tab: tab.to_owned(),
            form: form.to_owned(),
            at: at.to_vec(),
            column: None,
            rows: rows.to_vec(),
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
        self.column_expression_at(tab, form, at, &[], column, row, kind)
    }

    /// As [`Calc::column_expression`], for a table inside the designed items
    /// of repeaters, `outer` being the item of each.
    #[allow(clippy::too_many_arguments)]
    pub fn column_expression_at(
        &self,
        tab: &str,
        form: &str,
        at: &[usize],
        outer: &[usize],
        column: &str,
        row: Option<usize>,
        kind: ExprKind,
    ) -> Option<ExprState> {
        let mut rows = outer.to_vec();
        if kind.per_row() {
            rows.extend(row);
        }
        self.expression_state(&ExprKey {
            tab: tab.to_owned(),
            form: form.to_owned(),
            at: at.to_vec(),
            column: Some(column.to_owned()),
            rows,
            kind,
        })
    }

    /// The first failure of an expression of the control at `at` (or of its
    /// column `column`) in any of the items it runs for: the item of each
    /// repeater around (and the row, for a column), and the message.
    pub fn first_expression_error(
        &self,
        tab: &str,
        form: &str,
        at: &[usize],
        column: Option<&str>,
        kind: ExprKind,
    ) -> Option<(Vec<usize>, String)> {
        self.exprs
            .by_key
            .iter()
            .filter(|(k, _)| {
                k.tab == tab
                    && k.form == form
                    && k.at == at
                    && k.column.as_deref() == column
                    && k.kind == kind
            })
            .filter_map(|(k, e)| match &e.result {
                Some(Err(message)) => Some((k.rows.clone(), message.clone())),
                _ => None,
            })
            .min_by(|a, b| a.0.cmp(&b.0))
    }

    /// Shows a report, so that its expressions run, or none. The report
    /// shown before stops running; its results are dropped.
    pub fn show_report(&mut self, name: Option<&str>) {
        let name = name
            .filter(|n| self.reports.contains_key(*n))
            .map(str::to_owned);
        if self.exprs.shown_report != name {
            // The report shown until now keeps its run, if it is whole.
            self.take_report_run();
            self.exprs.shown_report = name;
            self.exprs.resync = true;
        }
    }

    /// The report whose expressions run.
    pub fn shown_report(&self) -> Option<&str> {
        self.exprs.shown_report.as_deref()
    }

    /// What an expression of a control of a report came to, inside the
    /// designed items of the repeaters around it (`rows`, outermost first).
    pub fn report_expression(
        &self,
        report: &str,
        at: &[usize],
        rows: &[usize],
        kind: ExprKind,
    ) -> Option<ExprState> {
        self.control_expression_at(report, REPORT_FORM, at, rows, kind)
    }

    /// What an expression of a column of a table of a report came to.
    pub fn report_column_expression(
        &self,
        report: &str,
        at: &[usize],
        outer: &[usize],
        column: &str,
        row: Option<usize>,
        kind: ExprKind,
    ) -> Option<ExprState> {
        self.column_expression_at(report, REPORT_FORM, at, outer, column, row, kind)
    }

    pub(crate) fn expression_state(&self, key: &ExprKey) -> Option<ExprState> {
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
            if let Some((e, _)) = self.exprs.probes.get_mut(&outcome.key) {
                // A probe runs once; an edit since does not matter to it.
                if e.text == outcome.text {
                    e.result = Some(outcome.result);
                    e.stale = false;
                    any = true;
                }
                continue;
            }
            let Some(e) = self.exprs.by_key.get_mut(&outcome.key) else {
                continue;
            };
            if overtaken || e.text != outcome.text {
                continue;
            }
            // The items of a repeater decide which expressions run inside it,
            // and the items and visible bands of a multipage report its pages.
            // So do the defaults and lists of a report's parameters, which
            // give `params`.
            let decides = outcome.key.kind == ExprKind::Items
                || (outcome.key.form == REPORT_FORM
                    && outcome.key.at.len() == 1
                    && outcome.key.kind == ExprKind::Visible)
                || (outcome.key.form == REPORT_FORM && outcome.key.at.is_empty());
            if decides && e.result.as_ref() != Some(&outcome.result) {
                self.exprs.resync = true;
            }
            e.result = Some(outcome.result);
            e.deps = outcome.deps;
            e.stale = false;
            any = true;
        }
        any
    }
}
