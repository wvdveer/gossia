//! Disconnect from Data Sources (DESIGN.md, "Data sources", "Disconnect from
//! Data Sources"): every read of a source written out in a formula or in an
//! expression of a form or a report is replaced by a literal of what it gives
//! now, so the workbook can go to someone who does not have the sources.
//!
//! It goes in three steps. [`Calc::plan_disconnect`] finds the reads and puts
//! a probe for each in the queue of expressions: `_as_literal((read))`, run
//! where the formula or expression runs, once for each instance (each row of
//! a column, each item of a repeater). Once the workbook is idle,
//! [`Calc::finish_disconnect`] reads the probes back and works out the new
//! texts, and what cannot be replaced, without changing anything; and
//! [`Calc::apply_disconnect`] makes the changes.
//!
//! A report's expressions run only while it is shown, so its reads are
//! planned on their own ([`Calc::plan_disconnect_report`]) while it is.
//! [`find_disconnect`] goes through the formulas and forms and then each
//! report in turn, and [`apply_and_rerun`] makes the changes and calculates
//! the results cache of each report that changed again.
//!
//! A read is replaced together with what is done to its result in the same
//! chain, as far as that has a literal: `source('api').get('/x').json()`
//! becomes the data, though the response alone has no literal. Writes are
//! never replaced, nor is a read whose source is not named in the text, nor
//! anything in a module.

use gossia_core::form::{ColumnDesign, Control, FormDesign, template};
use gossia_core::report::ReportDesign;
use indexmap::IndexMap;

use super::control_exprs::Bindings;
use super::report_params::{mentions_params, param_key};
use super::*;

/// The most text one literal may have.
const MAX_LITERAL: usize = 1_000_000;

/// What is done when Disconnect from Data Sources is given: the probes that
/// are running, and where their answers go.
#[derive(Debug, Clone)]
pub struct DisconnectPlan {
    targets: Vec<Target>,
    /// The formulas and forms, not a report: the modules are looked at too.
    workbook: bool,
}

impl DisconnectPlan {
    /// How many reads were found.
    pub fn reads(&self) -> usize {
        self.targets.iter().map(|t| t.reads.len()).sum()
    }
}

/// What the command would change, and what it cannot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Disconnect {
    changes: Vec<(Place, String)>,
    /// Each where, and why its use of a source stays.
    pub problems: Vec<String>,
    /// How many reads are replaced.
    pub replaced: usize,
    /// The reports with a read replaced that uses their parameters, so is
    /// replaced by what it gives with the values chosen now.
    pub with_params: Vec<String>,
}

impl Disconnect {
    /// How many formulas (column formulas counted once) change.
    pub fn formulas(&self) -> usize {
        self.changes
            .iter()
            .filter(|(p, _)| matches!(p, Place::Formula { .. } | Place::Column { .. }))
            .count()
    }

    /// How many expressions of forms change.
    pub fn expressions(&self) -> usize {
        self.changes.len() - self.formulas() - self.report_expressions()
    }

    /// How many expressions of reports change.
    pub fn report_expressions(&self) -> usize {
        self.changes
            .iter()
            .filter(|(p, _)| p.report().is_some())
            .count()
    }

    /// The reports whose design changes, in the order found.
    pub fn reports(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (place, _) in &self.changes {
            if let Some(report) = place.report()
                && !out.iter().any(|r| r == report)
            {
                out.push(report.to_owned());
            }
        }
        out
    }

    /// Whether nothing would change.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The new texts, each with where it goes, for tests and for showing.
    pub fn texts(&self) -> Vec<(String, String)> {
        self.changes
            .iter()
            .map(|(p, text)| (p.to_string(), text.clone()))
            .collect()
    }

    /// Adds what another part of the workbook would change.
    pub fn merge(&mut self, other: Disconnect) {
        self.changes.extend(other.changes);
        self.problems.extend(other.problems);
        self.replaced += other.replaced;
        for report in other.with_params {
            if !self.with_params.contains(&report) {
                self.with_params.push(report);
            }
        }
    }
}

/// A text that may be rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Formula {
        sheet: String,
        path: Path,
    },
    /// A column formula: its rule and the formula of every item.
    Column {
        sheet: String,
        array: Path,
        field: String,
    },
    /// An expression of a control, or of a column of its table. For a
    /// report, `tab` is the report and `form` is [`REPORT_FORM`].
    Control {
        tab: String,
        form: String,
        at: Vec<usize>,
        column: Option<String>,
        kind: ExprKind,
    },
    /// The whole text of a template label; its parts are rewritten.
    Template {
        tab: String,
        form: String,
        at: Vec<usize>,
    },
    /// The expression of one parameter of a chart.
    Chart {
        tab: String,
        form: String,
        at: Vec<usize>,
        param: String,
    },
    /// The default (`Value`) or list (`Choices`) of a report's parameter.
    Param {
        report: String,
        name: String,
        kind: ExprKind,
    },
}

impl Place {
    /// The report the text is in, if it is in one.
    fn report(&self) -> Option<&str> {
        match self {
            Place::Control { tab, form, .. }
            | Place::Template { tab, form, .. }
            | Place::Chart { tab, form, .. }
                if form == REPORT_FORM =>
            {
                Some(tab)
            }
            Place::Param { report, .. } => Some(report),
            _ => None,
        }
    }
}

/// `form Main of Orders`, or `report Sales`.
fn owner(tab: &str, form: &str) -> String {
    if form == REPORT_FORM {
        format!("report {tab}")
    } else {
        format!("form {form} of {tab}")
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::Formula { sheet, path } => write!(f, "{}", Loc::new(sheet, path.clone())),
            Place::Column {
                sheet,
                array,
                field,
            } => {
                let rule = ColumnFormula {
                    array: array.clone(),
                    field: field.clone(),
                    text: String::new(),
                };
                write!(f, "{sheet}.{}", rule.label())
            }
            Place::Control {
                tab,
                form,
                at,
                column,
                kind,
            } => {
                write!(f, "{}, control {}", owner(tab, form), at_text(at))?;
                if let Some(column) = column {
                    write!(f, ", column {column}")?;
                }
                write!(f, ", {}", kind_text(*kind))
            }
            Place::Template { tab, form, at } => {
                write!(f, "{}, control {}, label", owner(tab, form), at_text(at))
            }
            Place::Chart {
                tab,
                form,
                at,
                param,
            } => write!(
                f,
                "{}, control {}, chart {param}",
                owner(tab, form),
                at_text(at)
            ),
            Place::Param { report, name, kind } => write!(
                f,
                "report {report}, parameter {name}, {}",
                if *kind == ExprKind::Choices {
                    "choices"
                } else {
                    "default"
                }
            ),
        }
    }
}

fn at_text(at: &[usize]) -> String {
    at.iter()
        .map(|i| (i + 1).to_string())
        .collect::<Vec<_>>()
        .join(".")
}

fn kind_text(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible => "visible when",
        ExprKind::Enabled => "enabled when",
        ExprKind::TextColour => "text colour",
        ExprKind::FillColour => "fill colour",
        ExprKind::Choices => "choices",
        ExprKind::Value => "value",
        ExprKind::Template => "label",
        ExprKind::Items => "items",
        ExprKind::Chart => "chart",
    }
}

/// One text with reads in it, or with a use of a source that stays.
#[derive(Debug, Clone)]
struct Target {
    place: Place,
    /// For a template, the expression of one part; else the whole text.
    text: String,
    /// The part of the template, for a template.
    part: Option<usize>,
    reads: Vec<crate::engine::SourceRead>,
    /// For each read, for each of its chains, the probe of each instance.
    probes: Vec<Vec<Vec<usize>>>,
    problems: Vec<String>,
}

/// Whether the text calls `source` at all (and so may need a look).
fn mentions_source(text: &str) -> bool {
    text.match_indices("source").any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + "source".len()..].trim_start();
        !before.is_some_and(|c| c == '_' || c == '.' || c.is_alphanumeric())
            && after.starts_with('(')
    })
}

/// The texts of a design that use a source: each with where it goes, the
/// part of a template, and the key its instances run under.
fn design_texts(
    tab: &str,
    form: &str,
    design: &FormDesign,
) -> Vec<(Place, String, Option<usize>, ExprKey)> {
    let mut texts = Vec::new();
    each_control(&design.controls, &mut Vec::new(), &mut |at, control| {
        let key = |kind, column: Option<&str>| ExprKey {
            tab: tab.to_owned(),
            form: form.to_owned(),
            at: at.to_vec(),
            column: column.map(str::to_owned),
            rows: Vec::new(),
            kind,
        };
        let place = |kind, column: Option<&str>| Place::Control {
            tab: tab.to_owned(),
            form: form.to_owned(),
            at: at.to_vec(),
            column: column.map(str::to_owned),
            kind,
        };
        for kind in ExprKind::ALL {
            if kind == ExprKind::Template {
                if !control.is_template() {
                    continue;
                }
                let text = control.text.as_deref().unwrap_or_default();
                if let Ok(parts) = template::parts(text) {
                    for (i, part) in parts.iter().enumerate() {
                        if let template::Part::Expr { text, .. } = part
                            && mentions_source(text)
                        {
                            let place = Place::Template {
                                tab: tab.to_owned(),
                                form: form.to_owned(),
                                at: at.to_vec(),
                            };
                            texts.push((place, text.clone(), Some(i), key(kind, None)));
                        }
                    }
                }
                continue;
            }
            if let Some(text) = control_text(control, kind).filter(|t| mentions_source(t)) {
                texts.push((place(kind, None), text.to_owned(), None, key(kind, None)));
            }
            for column in &control.table_columns {
                if let Some(text) = column_text(column, kind).filter(|t| mentions_source(t)) {
                    let name = Some(column.name.as_str());
                    texts.push((place(kind, name), text.to_owned(), None, key(kind, name)));
                }
            }
        }
        for (param, text) in &control.chart_args {
            if mentions_source(text) {
                let place = Place::Chart {
                    tab: tab.to_owned(),
                    form: form.to_owned(),
                    at: at.to_vec(),
                    param: param.clone(),
                };
                texts.push((place, text.clone(), None, key(ExprKind::Chart, None)));
            }
        }
    });
    texts
}

impl Calc {
    /// Finds the reads of data sources in the formulas and the expressions of
    /// the forms and starts a probe of each. Run the evaluation until idle,
    /// then call [`Calc::finish_disconnect`]. The workbook must be trusted, and
    /// should be idle, so that every row and item is known. The reports are
    /// planned one at a time, with [`Calc::plan_disconnect_report`].
    pub fn plan_disconnect(&mut self) -> Result<DisconnectPlan, String> {
        if !self.is_trusted() {
            return Err(
                "the workbook is not trusted, so what the sources give cannot be read".to_owned(),
            );
        }
        self.sync_expressions();
        let mut targets: Vec<Target> = Vec::new();
        let mut probes: Vec<(String, Loc, Bindings, bool)> = Vec::new();
        let none: Bindings = Vec::new();

        // Formulas, with the items of a column formula as the instances of
        // its rule.
        let sheets: Vec<String> = self.sheet_names().map(str::to_owned).collect();
        for sheet in &sheets {
            let mut columns: Vec<(ColumnFormula, Vec<Loc>)> = Vec::new();
            let mut plain: Vec<(Path, String)> = Vec::new();
            for (path, text) in self.formulas(sheet) {
                if !mentions_source(text) {
                    continue;
                }
                match self.column_of(sheet, path) {
                    Some(rule) => match columns.iter_mut().find(|(r, _)| r == rule) {
                        Some((_, locs)) => locs.push(Loc::new(sheet, path.clone())),
                        None => columns.push((rule.clone(), vec![Loc::new(sheet, path.clone())])),
                    },
                    None => plain.push((path.clone(), text.to_owned())),
                }
            }
            for (path, text) in plain {
                let loc = Loc::new(sheet, path.clone());
                let place = Place::Formula {
                    sheet: sheet.clone(),
                    path,
                };
                targets.push(self.target(
                    place,
                    text,
                    None,
                    &[(loc, none.clone())],
                    true,
                    &mut probes,
                ));
            }
            for (rule, locs) in columns {
                let instances: Vec<(Loc, Bindings)> =
                    locs.into_iter().map(|l| (l, none.clone())).collect();
                let place = Place::Column {
                    sheet: sheet.clone(),
                    array: rule.array.clone(),
                    field: rule.field.clone(),
                };
                targets.push(self.target(
                    place,
                    rule.text.clone(),
                    None,
                    &instances,
                    true,
                    &mut probes,
                ));
            }
        }

        // The expressions of the forms.
        let groups = self.exprs.instance_groups();
        let mut texts = Vec::new();
        for (tab, form, design) in self.forms.iter() {
            texts.extend(design_texts(tab, form, design));
        }
        for (place, text, part, key) in texts {
            let instances = instances_of(&groups, &key);
            targets.push(self.target(place, text, part, &instances, false, &mut probes));
        }

        self.exprs.set_probes(probes);
        Ok(DisconnectPlan {
            targets,
            workbook: true,
        })
    }

    /// Finds the reads of data sources in the expressions of the report
    /// `report` and starts a probe of each, as [`Calc::plan_disconnect`] does
    /// for the formulas and forms. The report must be shown, and the workbook
    /// should be idle, so that its expressions have run with the parameter
    /// values chosen now.
    pub fn plan_disconnect_report(&mut self, report: &str) -> Result<DisconnectPlan, String> {
        if !self.is_trusted() {
            return Err(
                "the workbook is not trusted, so what the sources give cannot be read".to_owned(),
            );
        }
        if self.shown_report() != Some(report) {
            return Err(format!("the report {report} is not shown"));
        }
        self.sync_expressions();
        let Some(design) = self.reports.get(report) else {
            return Err(format!("there is no report {report}"));
        };
        let groups = self.exprs.instance_groups();
        let mut texts = design_texts(report, REPORT_FORM, &design.layout);
        for p in &design.params {
            let wants = [
                (ExprKind::Value, p.default_expression()),
                (ExprKind::Choices, p.choices_expression()),
            ];
            for (kind, text) in wants {
                if let Some(text) = text.filter(|t| mentions_source(t)) {
                    let place = Place::Param {
                        report: report.to_owned(),
                        name: p.name.clone(),
                        kind,
                    };
                    texts.push((
                        place,
                        text.to_owned(),
                        None,
                        param_key(report, &p.name, kind),
                    ));
                }
            }
        }
        let mut targets: Vec<Target> = Vec::new();
        let mut probes: Vec<(String, Loc, Bindings, bool)> = Vec::new();
        for (place, text, part, key) in texts {
            let instances = instances_of(&groups, &key);
            targets.push(self.target(place, text, part, &instances, false, &mut probes));
        }
        self.exprs.set_probes(probes);
        Ok(DisconnectPlan {
            targets,
            workbook: false,
        })
    }

    /// Finds the reads in one text and adds a probe for each chain of each
    /// read in each instance.
    fn target(
        &self,
        place: Place,
        text: String,
        part: Option<usize>,
        instances: &[(Loc, Bindings)],
        formula: bool,
        probes: &mut Vec<(String, Loc, Bindings, bool)>,
    ) -> Target {
        let scan = self.syntax.source_reads(&text);
        let mut problems = scan.others;
        if scan.reads.is_empty() && problems.is_empty() {
            problems.push("it uses a data source but cannot be read".to_owned());
        }
        if !scan.reads.is_empty() && instances.is_empty() {
            problems.push(
                "it has not run (it is in a repeater with no items, or on a control not shown), \
                 so what it reads is not known"
                    .to_owned(),
            );
        }
        let mut target_probes = Vec::new();
        if !instances.is_empty() {
            for read in &scan.reads {
                let mut chains = Vec::new();
                for chain in &read.chains {
                    let probe = format!("_as_literal(({}))", &text[chain.clone()]);
                    let mut each = Vec::new();
                    for (loc, bindings) in instances {
                        each.push(probes.len());
                        probes.push((probe.clone(), loc.clone(), bindings.clone(), formula));
                    }
                    chains.push(each);
                }
                target_probes.push(chains);
            }
        }
        Target {
            place,
            text,
            part,
            reads: if instances.is_empty() {
                Vec::new()
            } else {
                scan.reads
            },
            probes: target_probes,
            problems,
        }
    }

    /// The controls of a form, or of a report when `form` is
    /// [`REPORT_FORM`].
    fn layout_of(&self, tab: &str, form: &str) -> Option<&FormDesign> {
        if form == REPORT_FORM {
            self.reports.get(tab).map(|r| &r.layout)
        } else {
            self.forms.get(tab, form)
        }
    }

    /// Reads the probes back and works out the new texts: `None` if some
    /// have not run. Nothing is changed, and the probes are gone afterwards
    /// either way.
    pub fn finish_disconnect(&mut self, plan: &DisconnectPlan) -> Option<Disconnect> {
        let results = self.exprs.probe_results();
        self.exprs.clear_probes();
        let results = results?;
        let mut out = Disconnect::default();
        // The new text of each part of a template, gathered per control.
        let mut templates: Vec<(Place, Vec<(usize, String)>)> = Vec::new();
        for target in &plan.targets {
            let mut problems: Vec<String> = target.problems.clone();
            let mut chosen: Vec<(std::ops::Range<usize>, String)> = Vec::new();
            for (read, chains) in target.reads.iter().zip(&target.probes) {
                let mut why: Option<String> = None;
                let mut found: Option<(std::ops::Range<usize>, String)> = None;
                for (range, each) in read.chains.iter().zip(chains).rev() {
                    match same_literal(each.iter().map(|&i| &results[i])) {
                        Ok(literal) => {
                            found = Some((range.clone(), literal));
                            break;
                        }
                        Err(e) => {
                            why.get_or_insert(e);
                        }
                    }
                }
                match found {
                    Some(found) => chosen.push(found),
                    None => {
                        let read_text = read.chains.first().map_or("", |r| &target.text[r.clone()]);
                        problems.push(format!(
                            "{read_text}: {}",
                            why.unwrap_or_else(|| "it did not run".to_owned())
                        ));
                    }
                }
            }
            // An outer read takes the place of one inside it.
            chosen.sort_by_key(|(r, _)| std::cmp::Reverse(r.len()));
            let mut kept: Vec<(std::ops::Range<usize>, String)> = Vec::new();
            for (range, literal) in chosen {
                if kept
                    .iter()
                    .all(|(k, _)| range.end <= k.start || k.end <= range.start)
                {
                    kept.push((range, literal));
                }
            }
            let reads = kept.len();
            let uses_params = target.place.report().is_some()
                && kept
                    .iter()
                    .any(|(range, _)| mentions_params(&target.text[range.clone()]));
            kept.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
            let mut text = target.text.clone();
            for (range, literal) in &kept {
                let next = text[range.end..].chars().next();
                let wrapped = if literal.starts_with('-')
                    || (literal.starts_with(|c: char| c.is_ascii_digit()) && next == Some('.'))
                {
                    format!("({literal})")
                } else {
                    literal.clone()
                };
                text.replace_range(range.clone(), &wrapped);
            }
            for problem in problems {
                out.problems.push(format!("{}: {problem}", target.place));
            }
            if reads == 0 {
                continue;
            }
            if let Err(e) = self.syntax.check(&text) {
                out.problems.push(format!(
                    "{}: the new text does not parse ({e})",
                    target.place
                ));
                continue;
            }
            out.replaced += reads;
            if uses_params
                && let Some(report) = target.place.report()
                && !out.with_params.iter().any(|r| r == report)
            {
                out.with_params.push(report.to_owned());
            }
            match target.part {
                Some(part) => match templates.iter_mut().find(|(p, _)| *p == target.place) {
                    Some((_, parts)) => parts.push((part, text)),
                    None => templates.push((target.place.clone(), vec![(part, text)])),
                },
                None => out.changes.push((target.place.clone(), text)),
            }
        }
        for (place, new_parts) in templates {
            let Place::Template { tab, form, at } = &place else {
                continue;
            };
            let Some(control) = self
                .layout_of(tab, form)
                .and_then(|d| control_at(&d.controls, at))
            else {
                continue;
            };
            let old = control.text.clone().unwrap_or_default();
            let Ok(mut parts) = template::parts(&old) else {
                continue;
            };
            for (i, text) in &new_parts {
                if let Some(template::Part::Expr { text: t, .. }) = parts.get_mut(*i) {
                    *t = text.clone();
                }
            }
            let rebuilt = template::rebuild(&parts);
            if template::parts(&rebuilt).as_ref() != Ok(&parts) {
                out.problems.push(format!(
                    "{place}: the new template does not read back the same"
                ));
                out.replaced -= new_parts.len().min(out.replaced);
                continue;
            }
            out.changes.push((place, rebuilt));
        }
        if plan.workbook {
            for name in self.module_names() {
                if self.module_source(name).is_some_and(mentions_source) {
                    out.problems.push(format!(
                        "module {name}: it uses a data source; modules are not changed"
                    ));
                }
            }
        }
        Some(out)
    }

    /// Makes the changes: the formulas in one step of the history, the forms
    /// and reports (which have no history) in place. The results caches of
    /// the reports that change no longer fit; [`apply_and_rerun`] also
    /// calculates them again.
    pub fn apply_disconnect(&mut self, disconnect: &Disconnect) {
        let mut ops: Vec<Op> = Vec::new();
        let mut forms: Vec<(String, String, FormDesign)> = Vec::new();
        let mut reports: Vec<(String, ReportDesign)> = Vec::new();
        for (place, text) in &disconnect.changes {
            match place {
                Place::Formula { sheet, path } => {
                    if self.formula_text(sheet, path).is_some() {
                        ops.push(self.formula_op(sheet, path, text));
                    }
                }
                Place::Column {
                    sheet,
                    array,
                    field,
                } => {
                    let Some(before) = self.column_formula(sheet, array, field).map(str::to_owned)
                    else {
                        continue;
                    };
                    let old = ColumnFormula {
                        array: array.clone(),
                        field: field.clone(),
                        text: before.clone(),
                    };
                    let places = self.column_places(sheet, &old);
                    ops.push(self.store_column(sheet, array, field, Some(text), Some(before)));
                    for path in places {
                        ops.push(self.formula_op(sheet, &path, text));
                    }
                }
                Place::Param { report, name, kind } => {
                    let Some(design) = self.report_copy(&mut reports, report) else {
                        continue;
                    };
                    if let Some(p) = design.params.iter_mut().find(|p| &p.name == name) {
                        match kind {
                            ExprKind::Choices => p.choices = Some(text.clone()),
                            _ => p.default = Some(text.clone()),
                        }
                    }
                }
                Place::Control { tab, form, at, .. }
                | Place::Template { tab, form, at }
                | Place::Chart { tab, form, at, .. } => {
                    let layout = if form == REPORT_FORM {
                        self.report_copy(&mut reports, tab).map(|r| &mut r.layout)
                    } else {
                        let index = match forms.iter().position(|(t, f, _)| t == tab && f == form) {
                            Some(i) => Some(i),
                            None => self.forms.get(tab, form).cloned().map(|design| {
                                forms.push((tab.clone(), form.clone(), design));
                                forms.len() - 1
                            }),
                        };
                        index.map(|i| &mut forms[i].2)
                    };
                    let Some(control) = layout.and_then(|l| control_at_mut(&mut l.controls, at))
                    else {
                        continue;
                    };
                    match place {
                        Place::Chart { param, .. } => {
                            control.chart_args.insert(param.clone(), text.clone());
                        }
                        Place::Template { .. } => control.text = Some(text.clone()),
                        Place::Control { column, kind, .. } => {
                            let slot = match column {
                                Some(name) => control
                                    .table_columns
                                    .iter_mut()
                                    .find(|c| &c.name == name)
                                    .and_then(|c| column_slot(c, *kind)),
                                None => control_slot(control, *kind),
                            };
                            if let Some(slot) = slot {
                                *slot = Some(text.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        self.history
            .record("Disconnect from data sources".to_owned(), ops, None);
        if !forms.is_empty() {
            let store = self.forms_mut();
            for (tab, form, design) in forms {
                store.set(&tab, &form, design);
            }
        }
        for (name, design) in reports {
            let _ = self.set_report(&name, design);
        }
    }

    /// The copy of a report's design in `reports`, taken from the workbook
    /// the first time.
    fn report_copy<'a>(
        &self,
        reports: &'a mut Vec<(String, ReportDesign)>,
        name: &str,
    ) -> Option<&'a mut ReportDesign> {
        let index = match reports.iter().position(|(n, _)| n == name) {
            Some(i) => i,
            None => {
                let design = self.reports.get(name)?.clone();
                reports.push((name.to_owned(), design));
                reports.len() - 1
            }
        };
        Some(&mut reports[index].1)
    }
}

/// Every expression of a control, with where each instance of it runs.
type InstanceGroups = IndexMap<ExprKey, Vec<(Vec<usize>, Loc, Bindings)>>;

/// Where each instance of the expressions under `key` runs.
fn instances_of(groups: &InstanceGroups, key: &ExprKey) -> Vec<(Loc, Bindings)> {
    groups
        .get(key)
        .map(|found| {
            found
                .iter()
                .map(|(_, l, b)| (l.clone(), b.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// What runs the workbook while it is disconnected: the window's or the
/// command line's session, or a test's.
pub trait DisconnectRunner {
    fn calc_mut(&mut self) -> &mut Calc;
    /// Runs the evaluation until the workbook is idle. Returns false if the
    /// time ran out first.
    fn settle(&mut self) -> bool;
}

/// Why nothing can be disconnected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectError {
    /// The workbook is not trusted, or another reason, said.
    Refused(String),
    /// The sources did not answer in time.
    Timeout,
}

/// Finds what Disconnect from Data Sources would change: the formulas and
/// forms, then each report in turn, shown so that its expressions run with
/// the parameter values chosen now. The report shown before is shown again
/// afterwards. Nothing is changed.
pub fn find_disconnect(runner: &mut impl DisconnectRunner) -> Result<Disconnect, DisconnectError> {
    let shown = runner.calc_mut().shown_report().map(str::to_owned);
    let found = find_each(runner);
    runner.calc_mut().show_report(shown.as_deref());
    found
}

fn find_each(runner: &mut impl DisconnectRunner) -> Result<Disconnect, DisconnectError> {
    // Every row and item has to be known before the reads are found.
    if !runner.settle() {
        return Err(DisconnectError::Timeout);
    }
    let plan = runner
        .calc_mut()
        .plan_disconnect()
        .map_err(DisconnectError::Refused)?;
    let mut out = probe(runner, &plan)?;
    for report in runner.calc_mut().report_names() {
        runner.calc_mut().show_report(Some(&report));
        if !runner.settle() {
            return Err(DisconnectError::Timeout);
        }
        let plan = runner
            .calc_mut()
            .plan_disconnect_report(&report)
            .map_err(DisconnectError::Refused)?;
        out.merge(probe(runner, &plan)?);
    }
    Ok(out)
}

/// Runs the probes of `plan` and reads them back.
fn probe(
    runner: &mut impl DisconnectRunner,
    plan: &DisconnectPlan,
) -> Result<Disconnect, DisconnectError> {
    let idle = runner.settle();
    match runner.calc_mut().finish_disconnect(plan) {
        Some(found) if idle => Ok(found),
        _ => Err(DisconnectError::Timeout),
    }
}

/// Makes the changes, then runs each report that changed (and `also`, if
/// given) so that its results cache is this run with the values chosen now.
/// The report shown before is shown again afterwards. Returns false if the
/// time ran out before every report had run.
pub fn apply_and_rerun(
    runner: &mut impl DisconnectRunner,
    disconnect: &Disconnect,
    also: Option<&str>,
) -> bool {
    let calc = runner.calc_mut();
    let shown = calc.shown_report().map(str::to_owned);
    calc.apply_disconnect(disconnect);
    let mut reports = disconnect.reports();
    if let Some(also) = also
        && !reports.iter().any(|r| r == also)
    {
        reports.push(also.to_owned());
    }
    let mut whole = true;
    for report in reports {
        runner.calc_mut().show_report(Some(&report));
        whole &= runner.settle() && runner.calc_mut().take_report_run();
    }
    runner.calc_mut().show_report(shown.as_deref());
    whole
}

/// The literal every instance gave, if they all gave the same one.
fn same_literal<'a>(
    mut results: impl Iterator<Item = &'a Result<Value, String>>,
) -> Result<String, String> {
    let mut first: Option<&str> = None;
    for result in results.by_ref() {
        let text = match result {
            Ok(Value::Str(text)) => text.as_str(),
            Ok(Value::Error(e)) => return Err(literal_problem(&e.message)),
            Ok(_) => return Err("it has no literal".to_owned()),
            Err(e) => return Err(literal_problem(e)),
        };
        match first {
            None => first = Some(text),
            Some(f) if f == text => {}
            Some(_) => {
                return Err(
                    "it gives something different for each row or item, so no one literal can \
                     take its place"
                        .to_owned(),
                );
            }
        }
    }
    let literal = first.ok_or_else(|| "it did not run".to_owned())?;
    if literal.len() > MAX_LITERAL {
        return Err(format!(
            "what it gives is too big to write out ({} characters; at most {MAX_LITERAL})",
            literal.len()
        ));
    }
    Ok(literal.to_owned())
}

fn literal_problem(message: &str) -> String {
    match message.find("has no literal") {
        Some(_) => format!("what it gives cannot be written out ({message})"),
        None => format!("it failed: {message}"),
    }
}

/// Calls `f` on every control, with where it is.
fn each_control<'a>(
    controls: &'a [Control],
    at: &mut Vec<usize>,
    f: &mut impl FnMut(&[usize], &'a Control),
) {
    for (i, control) in controls.iter().enumerate() {
        at.push(i);
        f(at, control);
        each_control(&control.children, at, f);
        at.pop();
    }
}

fn control_at<'a>(controls: &'a [Control], at: &[usize]) -> Option<&'a Control> {
    let (first, rest) = at.split_first()?;
    let control = controls.get(*first)?;
    if rest.is_empty() {
        Some(control)
    } else {
        control_at(&control.children, rest)
    }
}

fn control_at_mut<'a>(controls: &'a mut [Control], at: &[usize]) -> Option<&'a mut Control> {
    let (first, rest) = at.split_first()?;
    let control = controls.get_mut(*first)?;
    if rest.is_empty() {
        Some(control)
    } else {
        control_at_mut(&mut control.children, rest)
    }
}

fn control_text(control: &Control, kind: ExprKind) -> Option<&str> {
    match kind {
        ExprKind::Visible => control.visible_when.as_deref(),
        ExprKind::Enabled => control.enabled_when.as_deref(),
        ExprKind::TextColour => control.text_colour.as_deref(),
        ExprKind::FillColour => control.fill_colour.as_deref(),
        ExprKind::Choices => control.choices.as_deref(),
        ExprKind::Value => control.value.as_deref(),
        ExprKind::Items => control.items.as_deref(),
        ExprKind::Template | ExprKind::Chart => None,
    }
}

fn control_slot(control: &mut Control, kind: ExprKind) -> Option<&mut Option<String>> {
    Some(match kind {
        ExprKind::Visible => &mut control.visible_when,
        ExprKind::Enabled => &mut control.enabled_when,
        ExprKind::TextColour => &mut control.text_colour,
        ExprKind::FillColour => &mut control.fill_colour,
        ExprKind::Choices => &mut control.choices,
        ExprKind::Value => &mut control.value,
        ExprKind::Items => &mut control.items,
        ExprKind::Template => &mut control.text,
        ExprKind::Chart => return None,
    })
}

fn column_text(column: &ColumnDesign, kind: ExprKind) -> Option<&str> {
    match kind {
        ExprKind::Visible => column.visible_when.as_deref(),
        ExprKind::Enabled => column.enabled_when.as_deref(),
        ExprKind::TextColour => column.text_colour.as_deref(),
        ExprKind::FillColour => column.fill_colour.as_deref(),
        ExprKind::Choices => column.choices.as_deref(),
        ExprKind::Value => column.value.as_deref(),
        ExprKind::Template | ExprKind::Items | ExprKind::Chart => None,
    }
}

fn column_slot(column: &mut ColumnDesign, kind: ExprKind) -> Option<&mut Option<String>> {
    Some(match kind {
        ExprKind::Visible => &mut column.visible_when,
        ExprKind::Enabled => &mut column.enabled_when,
        ExprKind::TextColour => &mut column.text_colour,
        ExprKind::FillColour => &mut column.fill_colour,
        ExprKind::Choices => &mut column.choices,
        ExprKind::Value => &mut column.value,
        ExprKind::Template | ExprKind::Items | ExprKind::Chart => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_of_source_is_told_from_other_names() {
        assert!(mentions_source("source('a').query('x')"));
        assert!(mentions_source("1 + source ('a').get('/')"));
        assert!(!mentions_source("sources.define('a', {})"));
        assert!(!mentions_source("my_source('a')"));
        assert!(!mentions_source("x.source('a')"));
        assert!(!mentions_source("'source'"));
    }
}
