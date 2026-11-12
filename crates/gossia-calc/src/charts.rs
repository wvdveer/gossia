//! Chart types (DESIGN.md, Reports, "Chart functions"): the built-in ones and
//! those the workbook's modules mark with `@chart`, all read from the source
//! without running it, so the designer offers them in a workbook that is not
//! trusted too. A chart on a report becomes one expression: its chart
//! function called with the expression of each parameter.

use gossia_i18n::tr;
use std::sync::OnceLock;

use gossia_core::form::{CHART_SIZE, Control, ControlKind};

use super::Calc;
use crate::engine::{ChartParam, Syntax};
use crate::python::syntax::PythonSyntax;
use crate::python::{CHART_FUNCTIONS, CHARTS};

/// A chart type the designer offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartType {
    /// The name `@chart` gives it ("Bar", "Funnel").
    pub name: String,
    /// The module that has it; `None` for a built-in one.
    pub module: Option<String>,
    pub function: String,
    /// One box on the property page for each.
    pub params: Vec<ChartParam>,
}

impl ChartType {
    /// What a formula calls to draw it.
    fn callee(&self) -> String {
        match &self.module {
            Some(module) => format!("{module}.{}", self.function),
            None => self.function.clone(),
        }
    }
}

/// The built-in chart types, read once from their module.
fn builtin() -> &'static [ChartType] {
    static TYPES: OnceLock<Vec<ChartType>> = OnceLock::new();
    TYPES.get_or_init(|| {
        let info = PythonSyntax
            .module_info(CHARTS)
            .expect("the built-in charts module parses");
        CHART_FUNCTIONS
            .iter()
            .map(|(name, function)| {
                let found = info
                    .charts
                    .iter()
                    .find(|c| c.function == *function)
                    .expect("each built-in chart type is marked with @chart");
                debug_assert_eq!(found.name, *name);
                ChartType {
                    name: found.name.clone(),
                    module: None,
                    function: found.function.clone(),
                    params: found.params.clone(),
                }
            })
            .collect()
    })
}

impl Calc {
    /// Sets the limits of reports in force: the most detail rows of a
    /// multipage report and the largest SVG of a chart, in bytes.
    pub fn set_report_limits(&mut self, detail_rows: usize, svg_bytes: usize) {
        if self.report_limits != (detail_rows, svg_bytes) {
            self.report_limits = (detail_rows, svg_bytes);
            self.exprs.resync = true;
        }
    }

    /// The most detail rows a multipage report shows.
    pub fn detail_limit(&self) -> usize {
        self.report_limits.0
    }

    /// The largest SVG a chart function may return, in bytes.
    pub fn svg_limit(&self) -> usize {
        self.report_limits.1
    }

    /// The chart types: the built-in ones, then those of the modules in
    /// module order. A name is used once, ignoring case: a module with a
    /// chart type whose name is taken is not loaded (DESIGN.md, "Chart
    /// functions"), and its chart types are left out.
    pub fn chart_types(&self) -> Vec<ChartType> {
        self.chart_types_and_clashes().0
    }

    /// The chart types, and the chart types of each module whose name is
    /// taken: by a built-in one, one of an earlier module, or one earlier
    /// in the same module.
    #[allow(clippy::type_complexity)]
    fn chart_types_and_clashes(&self) -> (Vec<ChartType>, Vec<(String, usize, String)>) {
        let mut types: Vec<ChartType> = builtin().to_vec();
        let mut clashes = Vec::new();
        for (module, m) in &self.modules {
            let Ok(info) = &m.info else { continue };
            let mut own: Vec<ChartType> = Vec::new();
            let mut clashed = false;
            for (i, c) in info.charts.iter().enumerate() {
                let taken = types
                    .iter()
                    .chain(&own)
                    .find(|t| t.name.eq_ignore_ascii_case(&c.name));
                if let Some(t) = taken {
                    let by = match &t.module {
                        _ if t.module.as_deref() == Some(module) => {
                            tr!("{function} in this module", function = t.function)
                        }
                        Some(other) => tr!(
                            "{function} of module {other}",
                            function = t.function,
                            other = other
                        ),
                        None => tr!("a built-in chart type").to_owned(),
                    };
                    clashes.push((module.clone(), i, by));
                    clashed = true;
                    continue;
                }
                own.push(ChartType {
                    name: c.name.clone(),
                    module: Some(module.clone()),
                    function: c.function.clone(),
                    params: c.params.clone(),
                });
            }
            if !clashed {
                types.extend(own);
            }
        }
        (types, clashes)
    }

    /// The chart type of this name, ignoring case.
    pub fn chart_type(&self, name: &str) -> Option<ChartType> {
        self.chart_types()
            .into_iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// Each chart type of a module whose name is taken, with its line,
    /// column and why: the module is not loaded.
    pub(super) fn chart_problems(&self, module: &str) -> Vec<(usize, usize, String)> {
        let Some(Ok(info)) = self.modules.get(module).map(|m| &m.info) else {
            return Vec::new();
        };
        self.chart_types_and_clashes()
            .1
            .into_iter()
            .filter(|(m, _, _)| m == module)
            .map(|(_, i, by)| {
                let c = &info.charts[i];
                (
                    c.at.0,
                    c.at.1,
                    tr!(
                        "the chart type name {name} is taken by {by}, so the module is not loaded",
                        name = c.name,
                        by = by
                    ),
                )
            })
            .collect()
    }

    /// The expression of a chart: its function called with the expression
    /// of each parameter that is filled in and the chart's size, the result
    /// checked to be SVG. `None` while nothing is chosen or filled in; why
    /// it cannot be drawn when a required parameter is empty, the chart type
    /// is not there, or an expression does not parse.
    pub(super) fn chart_text(&self, control: &Control) -> Option<Result<String, String>> {
        if control.kind != ControlKind::Chart {
            return None;
        }
        let name = control.chart.as_deref()?;
        let Some(chart) = self.chart_type(name) else {
            return Some(Err(tr!("there is no chart type named {name}", name = name)));
        };
        let mut args: Vec<String> = Vec::new();
        let mut missing: Vec<&str> = Vec::new();
        for p in &chart.params {
            let text = control
                .chart_args
                .get(&p.name)
                .map(|t| t.trim())
                .filter(|t| !t.is_empty());
            match text {
                Some(text) => {
                    if let Err(e) = self.syntax.check(text) {
                        return Some(Err(tr!(
                            "syntax error in {name}: {e}",
                            name = p.name,
                            e = e
                        )));
                    }
                    // On lines of their own, so a comment at the end of one
                    // does not swallow the rest.
                    args.push(format!("{}=(\n{text}\n)", p.name));
                }
                None if p.required => missing.push(&p.name),
                None => {}
            }
        }
        if args.is_empty() {
            return None;
        }
        if !missing.is_empty() {
            return Some(Err(tr!("fill in {missing}", missing = missing.join(", "))));
        }
        let width = control.width.unwrap_or(CHART_SIZE.0);
        let height = control.height.unwrap_or(CHART_SIZE.1);
        args.push(format!("width={width}, height={height}"));
        // `_chart_size` tells Gossia pyplot the size first, for a figure
        // made without one.
        Some(Ok(format!(
            "_chart_svg(_chart_size({width}, {height}) or {}({}))",
            chart.callee(),
            args.join(", ")
        )))
    }
}
