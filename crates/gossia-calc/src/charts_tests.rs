//! Charts (DESIGN.md, Reports, "Chart functions"): chart types read from
//! modules, the built-in ones, and a chart on a report as one expression.

use std::sync::Arc;

use gossia_core::form::{Control, ControlKind};
use gossia_core::report::{ReportDesign, ReportKind};
use gossia_core::{Value, json, jsonschema};

use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, ChartParam, ExprKind, ModuleStatus, Syntax};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "regions": {"type": "array", "items": {"type": "string"}},
    "sales": {"type": "array", "items": {"type": "number"}}
  }
}"#;

const FUNNEL: &str = r#"
@chart("Funnel")
def funnel(stages, counts, title=None, *, width, height):
    return '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d">%s %d</svg>' % (
        width, height, title or '', len(stages))

def helper():
    return 1
"#;

struct Book {
    calc: Calc,
    engine: PythonEngine,
}

fn chart(name: &str, args: &[(&str, &str)]) -> Control {
    let mut c = Control::new(ControlKind::Chart);
    c.chart = Some(name.to_owned());
    for (k, v) in args {
        c.chart_args.insert((*k).to_owned(), (*v).to_owned());
    }
    c
}

fn book(modules: &[(&str, &str)], controls: Vec<Control>) -> Book {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "Shop",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(r#"{"regions": ["North", "South", "East"], "sales": [12, 30.5, 7]}"#)
            .unwrap()
            .node,
    )
    .unwrap();
    for (name, source) in modules {
        calc.add_module(name, source).unwrap();
    }
    let mut design = ReportDesign::new(ReportKind::SinglePage);
    design.layout.controls = controls;
    calc.create_report("Sales", design).unwrap();
    let mut b = Book {
        calc,
        engine: PythonEngine::new(),
    };
    b.calc.show_report(Some("Sales"));
    b.run();
    b
}

impl Book {
    fn run(&mut self) {
        for _ in 0..3 {
            self.calc.recalc(&self.engine);
        }
    }

    fn svg(&self, at: usize) -> Option<Result<Value, String>> {
        self.calc
            .report_expression("Sales", &[at], &[], ExprKind::Chart)
            .and_then(|s| s.result)
    }

    fn text(&self, at: usize) -> String {
        match self.svg(at) {
            Some(Ok(Value::Str(s))) => s,
            other => panic!("not SVG: {other:?}"),
        }
    }
}

#[test]
fn a_chart_type_is_read_from_its_decorator_without_running_the_module() {
    let info = PythonSyntax.module_info(FUNNEL).unwrap();
    assert_eq!(info.charts.len(), 1);
    let c = &info.charts[0];
    assert_eq!((c.name.as_str(), c.function.as_str()), ("Funnel", "funnel"));
    assert_eq!(
        c.params,
        vec![
            ChartParam {
                name: "stages".into(),
                required: true
            },
            ChartParam {
                name: "counts".into(),
                required: true
            },
            ChartParam {
                name: "title".into(),
                required: false
            },
        ]
    );
    assert_eq!(c.at, (2, 1));
    // Only a literal name counts: the module is not run to find it.
    let named = "N = 'X'\n@chart(N)\ndef f(a, *, width, height):\n    pass\n";
    assert!(PythonSyntax.module_info(named).unwrap().charts.is_empty());
}

#[test]
fn the_built_in_chart_types_come_first_then_those_of_the_modules() {
    let b = book(&[("Shapes", FUNNEL)], Vec::new());
    let names: Vec<String> = b.calc.chart_types().into_iter().map(|t| t.name).collect();
    assert_eq!(
        names,
        ["Bar", "Line", "Pie", "Scatter", "Sankey", "Map", "Funnel"]
    );
    let funnel = b.calc.chart_type("funnel").unwrap();
    assert_eq!(funnel.module.as_deref(), Some("Shapes"));
    let bar = b.calc.chart_type("Bar").unwrap();
    assert_eq!(bar.module, None);
    let required: Vec<&str> = bar
        .params
        .iter()
        .filter(|p| p.required)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(required, ["labels", "values"]);
}

#[test]
fn a_chart_calls_its_function_with_each_expression_and_its_size() {
    let mut c = chart(
        "Funnel",
        &[("stages", "Shop.regions"), ("counts", "Shop.sales")],
    );
    c.width = Some(200);
    c.height = Some(100);
    let b = book(&[("Shapes", FUNNEL)], vec![c]);
    let svg = b.text(0);
    assert!(svg.contains(r#"width="200" height="100""#), "{svg}");
    assert!(svg.contains(" 3</svg>"), "{svg}");
}

#[test]
fn each_built_in_chart_draws_svg_that_parses() {
    let controls = vec![
        chart(
            "Bar",
            &[("labels", "Shop.regions"), ("values", "Shop.sales")],
        ),
        chart(
            "Bar",
            &[
                ("labels", "Shop.regions"),
                ("values", "[Shop.sales, [1, 2, 3]]"),
                ("series", "['This year', 'Last year']"),
                ("horizontal", "True"),
                ("title", "'Sales & more'"),
            ],
        ),
        chart(
            "Line",
            &[("labels", "Shop.regions"), ("values", "Shop.sales")],
        ),
        chart(
            "Pie",
            &[("labels", "Shop.regions"), ("values", "Shop.sales")],
        ),
        chart(
            "Scatter",
            &[
                ("x", "[1, 2, 3]"),
                ("y", "Shop.sales"),
                ("labels", "Shop.regions"),
            ],
        ),
        chart(
            "Sankey",
            &[
                (
                    "sources",
                    "['Pay', 'Pay', 'Gifts', 'Budget', 'Budget', 'Budget']",
                ),
                (
                    "targets",
                    "['Budget', 'Tax', 'Budget', 'Rent', 'Food', 'Saved']",
                ),
                ("values", "[3000, 900, 200, 1400, 600, 1200]"),
                ("title", "'Where it goes'"),
            ],
        ),
        chart(
            "Sankey",
            &[("sources", "[]"), ("targets", "[]"), ("values", "[]")],
        ),
        chart("Bar", &[("labels", "[]"), ("values", "[]")]),
        chart("Line", &[("labels", "['a']"), ("values", "[-5]")]),
        chart(
            "Map",
            &[
                ("longitudes", "[4.9, -74.0, 151.2]"),
                ("latitudes", "[52.4, 40.7, -33.9]"),
                ("labels", "['Amsterdam', 'New York', 'Sydney']"),
                ("title", "'Offices'"),
            ],
        ),
        chart("Map", &[("longitudes", "[]"), ("latitudes", "[]")]),
    ];
    let count = controls.len();
    let b = book(&[], controls);
    for at in 0..count {
        let svg = b.text(at);
        gossia_layout_check(&svg);
    }
    assert!(b.text(1).contains("Sales &amp; more"));
}

/// The built-in map draws the land and the borders behind the places, shows
/// the places with room around them or the whole world, and refuses a
/// latitude that is not one.
#[test]
fn a_map_draws_the_world_behind_its_places() {
    let b = book(
        &[],
        vec![
            chart(
                "Map",
                &[
                    ("longitudes", "[4.9, 2.35]"),
                    ("latitudes", "[52.4, 48.9]"),
                    ("labels", "['Amsterdam', 'Paris']"),
                ],
            ),
            chart("Map", &[("longitudes", "[]"), ("latitudes", "[]")]),
            chart("Map", &[("longitudes", "[0]"), ("latitudes", "[95]")]),
        ],
    );
    let europe = b.text(0);
    assert!(europe.contains("Amsterdam") && europe.contains("Paris"));
    // The land is filled faintly in the text colour, and the borders drawn.
    assert!(
        europe.contains(r#"fill="currentColor" fill-opacity="0.08""#),
        "{europe}"
    );
    assert!(europe.contains(r#"stroke-opacity="0.3""#), "{europe}");
    let world = b.text(1);
    // Only what is in view is written: Europe alone is far smaller.
    assert!(
        europe.len() * 2 < world.len(),
        "{} {}",
        europe.len(),
        world.len()
    );
    let Some(Err(e)) = b.svg(2) else { panic!() };
    assert!(e.contains("a latitude is from -90 to 90 degrees"), "{e}");
}

/// The SVG is well formed XML with an svg root. (The layout crate parses it
/// with usvg; calc does not depend on it.)
fn gossia_layout_check(svg: &str) {
    let doc = gossia_core::xmltree::parse(svg);
    assert!(doc.is_ok(), "{svg}");
    assert!(svg.starts_with("<svg "), "{svg}");
}

#[test]
fn a_missing_required_parameter_is_said_and_nothing_runs() {
    let b = book(
        &[],
        vec![
            chart("Bar", &[("labels", "Shop.regions")]),
            chart("Bar", &[]),
            chart("Bar", &[("labels", "Shop.regions"), ("values", "1 +")]),
            chart("Nope", &[("labels", "1")]),
        ],
    );
    assert_eq!(b.svg(0), Some(Err("fill in values".to_owned())));
    // Nothing filled in yet: no expression.
    assert!(
        b.calc
            .report_expression("Sales", &[1], &[], ExprKind::Chart)
            .is_none()
    );
    let Some(Err(e)) = b.svg(2) else { panic!() };
    assert!(e.starts_with("syntax error in values"), "{e}");
    assert_eq!(
        b.svg(3),
        Some(Err("there is no chart type named Nope".to_owned()))
    );
}

/// A chart whose parameters are all optional, or that has none but the
/// size, draws without anything filled in.
#[test]
fn a_chart_that_needs_no_values_draws_at_once() {
    let source = r#"
@chart("Fixed")
def fixed(width=400, height=300):
    return '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d"/>' % (width, height)

@chart("Titled")
def titled(title='Sales', *, width, height):
    return '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d"><title>%s</title></svg>' % (width, height, title)
"#;
    let mut c = chart("Fixed", &[]);
    c.width = Some(200);
    c.height = Some(100);
    let b = book(
        &[("Own", source)],
        vec![
            c,
            chart("Titled", &[]),
            chart("Titled", &[("title", "'Q3'")]),
        ],
    );
    assert!(b.text(0).contains(r#"width="200" height="100""#));
    assert!(b.text(1).contains("<title>Sales</title>"));
    assert!(b.text(2).contains("<title>Q3</title>"));
}

#[test]
fn what_is_not_svg_is_a_type_error_and_an_exception_its_error() {
    let source = "@chart('Odd')\ndef odd(v, *, width, height):\n    return v\n";
    let b = book(
        &[("Odd", source)],
        vec![
            chart("Odd", &[("v", "42")]),
            chart(
                "Odd",
                &[("v", "b'<svg xmlns=\"http://www.w3.org/2000/svg\"/>'")],
            ),
            chart("Bar", &[("labels", "['a']"), ("values", "['x']")]),
        ],
    );
    let Some(Err(e)) = b.svg(0) else {
        panic!("{:?}", b.svg(0))
    };
    assert!(e.starts_with("#TYPE!"), "{e}");
    assert!(b.text(1).starts_with("<svg"));
    let Some(Err(e)) = b.svg(2) else {
        panic!("{:?}", b.svg(2))
    };
    assert!(e.contains("values must be numbers"), "{e}");
}

#[test]
fn a_module_whose_chart_type_name_is_taken_is_not_loaded() {
    let other = "@chart('FUNNEL')\ndef wide(a, *, width, height):\n    return ''\n";
    let builtin = "@chart('bar')\ndef mine(a, *, width, height):\n    return ''\n";
    let b = book(
        &[("Shapes", FUNNEL), ("Other", other), ("Mine", builtin)],
        Vec::new(),
    );
    let names: Vec<String> = b.calc.chart_types().into_iter().map(|t| t.name).collect();
    assert_eq!(
        names,
        ["Bar", "Line", "Pie", "Scatter", "Sankey", "Map", "Funnel"]
    );
    assert!(matches!(
        b.calc.module_status("Other"),
        Some(ModuleStatus::Failed(_))
    ));
    let problems = b.calc.module_problems("Other").unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line, Some(1));
    assert!(
        problems[0].message.contains("funnel of module Shapes"),
        "{problems:?}"
    );
    let problems = b.calc.module_problems("Mine").unwrap();
    assert!(
        problems[0].message.contains("a built-in chart type"),
        "{problems:?}"
    );
    assert_eq!(b.calc.module_status("Shapes"), Some(&ModuleStatus::Loaded));
}

#[test]
fn editing_the_module_draws_the_chart_again() {
    let mut b = book(
        &[("Shapes", FUNNEL)],
        vec![chart(
            "Funnel",
            &[("stages", "Shop.regions"), ("counts", "Shop.sales")],
        )],
    );
    assert!(b.text(0).contains(" 3</svg>"));
    let changed = FUNNEL.replace("len(stages)", "len(stages) * 10");
    b.calc.set_module_source("Shapes", &changed).unwrap();
    b.run();
    assert!(b.text(0).contains(" 30</svg>"), "{}", b.text(0));
}

#[test]
fn the_detail_and_svg_limits_are_set_by_the_application() {
    let mut b = book(&[], Vec::new());
    assert_eq!(b.calc.detail_limit(), 100_000);
    assert_eq!(b.calc.svg_limit(), 5 << 20);
    b.calc.set_report_limits(10, 1000);
    assert_eq!((b.calc.detail_limit(), b.calc.svg_limit()), (10, 1000));
}

#[test]
fn a_sankey_diagram_lays_out_its_flows_and_refuses_a_circle() {
    let flows = |s: &str, t: &str, v: &str| {
        chart("Sankey", &[("sources", s), ("targets", t), ("values", v)])
    };
    let b = book(
        &[],
        vec![
            flows("['A', 'A', 'B']", "['B', 'C', 'C']", "[5, 2, 5]"),
            flows("['A', 'B']", "['B', 'A']", "[1, 1]"),
            flows("['A']", "['A']", "[1]"),
            flows("['A', 'B']", "['B']", "[1, 2]"),
            flows("['A']", "['B']", "[-1]"),
        ],
    );
    let svg = b.text(0);
    // A band for each flow and a node for each name, each labelled.
    assert_eq!(svg.matches("<path ").count(), 3, "{svg}");
    assert_eq!(svg.matches("<rect ").count(), 1 + 3, "{svg}");
    for name in ["A ", "B ", "C "] {
        assert!(svg.contains(&format!(">{name}<tspan")), "{svg}");
    }
    let error = |at: usize| match b.svg(at) {
        Some(Err(e)) => e,
        other => panic!("{other:?}"),
    };
    assert!(error(1).contains("round in a circle"), "{}", error(1));
    assert!(error(2).contains("A flows to itself"), "{}", error(2));
    assert!(error(3).contains("2 sources, 1 targets"), "{}", error(3));
    assert!(error(4).contains("cannot be negative"), "{}", error(4));
}

#[test]
fn a_pie_with_more_labels_than_colours_folds_the_smallest_into_other() {
    let labels: Vec<String> = (1..=10).map(|i| format!("'S{i}'")).collect();
    let values = "[5, 90, 4, 80, 70, 3, 60, 50, 40, 30]";
    let b = book(
        &[],
        vec![chart(
            "Pie",
            &[
                ("labels", &format!("[{}]", labels.join(", "))),
                ("values", values),
            ],
        )],
    );
    let svg = b.text(0);
    // The seven largest keep their place in the order given; the three
    // smallest (5 + 4 + 3) are Other, in a grey of the text colour.
    for kept in ["S2", "S4", "S5", "S7", "S8", "S9", "S10", "Other"] {
        assert!(svg.contains(&format!(">{kept}<")), "{kept}: {svg}");
    }
    for folded in [">S1<", ">S3<", ">S6<"] {
        assert!(!svg.contains(folded), "{folded}: {svg}");
    }
    assert!(
        svg.contains(r#"fill="currentColor" fill-opacity="0.35""#),
        "{svg}"
    );
    // 12 of 432 is 3%.
    assert!(svg.contains(">3%<"), "{svg}");
}
