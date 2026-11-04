//! A report of a workbook, read from what the workbook calculated: the
//! [`Source`] its layout takes its values from, and the whole report as a
//! PDF, as the application and `cliotree-cli` write it.

use std::cell::RefCell;

use cliotree_calc::{Calc, ExprKind, ExprState};
use cliotree_core::Value;
use cliotree_core::form::{Control, ControlKind};
use cliotree_layout::report::{self, Got, Prop, Source};

/// The values of a report's expressions, as the workbook has them.
pub struct CalcSource<'a> {
    pub calc: &'a Calc,
    pub report: &'a str,
}

fn kind(prop: Prop) -> ExprKind {
    match prop {
        Prop::Visible => ExprKind::Visible,
        Prop::TextColour => ExprKind::TextColour,
        Prop::FillColour => ExprKind::FillColour,
        Prop::Value => ExprKind::Value,
        Prop::Template => ExprKind::Template,
        Prop::Items => ExprKind::Items,
        Prop::Chart => ExprKind::Chart,
    }
}

/// What an expression came to, as the report shows it. Nothing runs in a
/// workbook that is not trusted: it shows what the results cache has, and
/// `#UNTRUSTED!` for what it has not. In a trusted one, an expression that
/// has not run yet shows its cached result as pending until it has.
fn got(trusted: bool, state: Option<ExprState>, cached: Option<&Result<Value, String>>) -> Got {
    let from_cache = |pending: bool| match cached {
        Some(Ok(value)) => Some(Got::Value(value.clone(), pending)),
        Some(Err(error)) => Some(Got::Error(error.clone())),
        None => None,
    };
    if !trusted {
        return from_cache(false).unwrap_or(if state.is_some() {
            Got::Untrusted
        } else {
            Got::None
        });
    }
    let Some(state) = state else {
        return from_cache(true).unwrap_or(Got::None);
    };
    match state.result {
        Some(Ok(value)) => Got::Value(value, state.pending),
        Some(Err(error)) => Got::Error(error),
        None => from_cache(true).unwrap_or(Got::Waiting),
    }
}

impl Source for CalcSource<'_> {
    fn svg_limit(&self) -> usize {
        self.calc.svg_limit()
    }

    fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got {
        got(
            self.calc.is_trusted(),
            self.calc
                .report_expression(self.report, at, rows, kind(prop)),
            self.calc
                .cached_expression(self.report, at, None, rows, kind(prop)),
        )
    }

    fn column(
        &self,
        at: &[usize],
        rows: &[usize],
        column: &str,
        row: Option<usize>,
        prop: Prop,
    ) -> Got {
        let kind = kind(prop);
        let mut cached_rows = rows.to_vec();
        if kind.per_row() {
            cached_rows.extend(row);
        }
        got(
            self.calc.is_trusted(),
            self.calc
                .report_column_expression(self.report, at, rows, column, row, kind),
            self.calc
                .cached_expression(self.report, at, Some(column), &cached_rows, kind),
        )
    }
}

/// A report run to the end, as `cliotree-cli` writes it.
pub struct Run {
    pub pdf: Vec<u8>,
    /// Each control that shows an error value, by band and control, with
    /// the error: `Detail band, Field order.total: ZeroDivisionError ...`.
    pub errors: Vec<String>,
    /// A multipage report stopped at the limit of detail rows.
    pub limited: bool,
}

/// The report `name` of `calc` as a PDF, laid out as its tab draws it: a
/// multipage report page by page, a single page report as one page.
pub fn pdf(calc: &Calc, name: &str) -> Result<Vec<u8>, String> {
    run(calc, name).map(|r| r.pdf)
}

/// The report `name` of `calc` laid out and written as a PDF, with what it
/// shows that went wrong.
pub fn run(calc: &Calc, name: &str) -> Result<Run, String> {
    let design = calc
        .report(name)
        .ok_or_else(|| format!("there is no report {name}"))?;
    let source = Recording {
        inner: CalcSource { calc, report: name },
        errors: RefCell::new(Vec::new()),
    };
    let (pdf, limited) = if design.bands().is_some() {
        let paged = report::pages(design, &source, calc.detail_limit());
        (crate::pages(&paged, name)?, paged.limited)
    } else {
        let laid = if design.canvas {
            report::canvas(design, &source)
        } else {
            report::single_page(design, design.page.content_width(), &source)
        };
        (crate::single_page(&laid, name)?, false)
    };
    let multipage = design.bands().is_some();
    let mut errors = Vec::new();
    for (at, column, error) in source.errors.into_inner() {
        let mut place = Vec::new();
        if multipage && let Some(&band) = at.first() {
            place.push(design.band_name(band));
        }
        let inside = if multipage { 1 } else { 0 };
        if at.len() > inside
            && let Some(control) = design.layout.get(&at)
        {
            place.push(control_name(control));
        }
        if let Some(column) = column {
            place.push(format!("column {column}"));
        }
        // The first line: a traceback follows it.
        let error = error.lines().next().unwrap_or_default().trim();
        let line = format!("{}: {error}", place.join(", "));
        if !errors.contains(&line) {
            errors.push(line);
        }
    }
    Ok(Run {
        pdf,
        errors,
        limited,
    })
}

/// What a control is called in a list of errors: its kind and what it shows.
pub fn control_name(control: &Control) -> String {
    let detail = match control.kind {
        ControlKind::Label => control.text.clone(),
        ControlKind::Chart => control.chart.clone(),
        ControlKind::Separator => None,
        _ => control
            .path
            .clone()
            .or_else(|| control.value.clone())
            .or_else(|| control.label.clone()),
    }
    .map(|d| d.trim().to_owned())
    .filter(|d| !d.is_empty());
    match detail {
        Some(detail) => format!("{} {detail}", control.kind.label()),
        None => control.kind.label().to_owned(),
    }
}

/// A source that notes each error value the layout is given.
struct Recording<'a> {
    inner: CalcSource<'a>,
    errors: RefCell<Vec<Noted>>,
}

/// An error value: the control's address, its column, and the error.
type Noted = (Vec<usize>, Option<String>, String);

impl Recording<'_> {
    fn note(&self, at: &[usize], column: Option<&str>, got: Got) -> Got {
        let error = match &got {
            Got::Error(e) => Some(e.clone()),
            Got::Value(Value::Error(e), _) if e.message.is_empty() => {
                Some(e.kind.code().to_owned())
            }
            Got::Value(Value::Error(e), _) => Some(format!("{} {}", e.kind.code(), e.message)),
            _ => None,
        };
        if let Some(error) = error {
            self.errors
                .borrow_mut()
                .push((at.to_vec(), column.map(str::to_owned), error));
        }
        got
    }
}

impl Source for Recording<'_> {
    fn svg_limit(&self) -> usize {
        self.inner.svg_limit()
    }

    fn control(&self, at: &[usize], rows: &[usize], prop: Prop) -> Got {
        self.note(at, None, self.inner.control(at, rows, prop))
    }

    fn column(
        &self,
        at: &[usize],
        rows: &[usize],
        column: &str,
        row: Option<usize>,
        prop: Prop,
    ) -> Got {
        self.note(
            at,
            Some(column),
            self.inner.column(at, rows, column, row, prop),
        )
    }
}
