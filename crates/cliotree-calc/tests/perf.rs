//! Performance of the proxy layer against the budgets in DESIGN.md ("Python
//! engine", "Speed"). Not run by default, because the figures only mean
//! something in a release build on a quiet machine:
//!
//! ```text
//! cargo test --release -p cliotree-calc --test perf -- --ignored --nocapture
//! ```
//!
//! The nightly job runs this and fails when a budget is exceeded.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cliotree_calc::Calc;
use cliotree_calc::python::{PythonEngine, PythonSyntax};
use cliotree_core::{Path, Value, json, jsonschema};

const N: usize = 10_000;

/// The budgets (DESIGN.md, "Python engine", "Speed").
const ATTRIBUTE_READ: Duration = Duration::from_micros(5);
const TEN_THOUSAND_FORMULAS: Duration = Duration::from_millis(250);
/// The first calculation also compiles 10,000 different formulas, which takes
/// about 20 microseconds each.
const TEN_THOUSAND_FORMULAS_FIRST: Duration = Duration::from_millis(500);
const ONE_AGGREGATE: Duration = Duration::from_millis(50);
/// An aggregate that goes into each of ten thousand objects for two fields:
/// an object proxy and three proxy reads per item, so about 7 microseconds.
/// A projection, `S.items.price`, makes an object proxy and a read per item.
const AGGREGATE_PROJECTION: Duration = Duration::from_millis(75);
const AGGREGATE_TWO_FIELDS: Duration = Duration::from_millis(100);

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn book(schema: &str, data: &str) -> (Calc, PythonEngine) {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(schema).unwrap(),
        json::parse(data).unwrap().node,
    )
    .unwrap();
    (calc, PythonEngine::new())
}

fn time<T>(what: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    let took = start.elapsed();
    println!("{what:<58} {took:>12.3?}");
    (out, took)
}

/// Runs one formula many times over to get a figure per operation.
fn per_operation(calc: &mut Calc, engine: &PythonEngine, path: &str, text: &str) -> Duration {
    calc.set_formula("S", &p(path), text).unwrap();
    let (_, took) = time(text, || calc.recalc(engine));
    took
}

#[test]
#[ignore = "a measurement, for release builds"]
fn proxy_layer_against_the_budgets() {
    let items: Vec<String> = (0..N)
        .map(|i| format!(r#"{{"qty": {}, "price": {}.5}}"#, i % 7 + 1, i % 13))
        .collect();
    let numbers: Vec<String> = (0..N).map(|i| i.to_string()).collect();
    let data = format!(
        r#"{{"n": 0, "x": 0.0, "nums": [{}], "out": [{}], "items": [{}], "one": {{"qty": 3, "price": 2.5}}}}"#,
        numbers.join(","),
        vec!["0"; N].join(","),
        items.join(",")
    );
    let schema = r#"{"type": "object", "properties": {
        "n": {"type": "integer"}, "x": {"type": "number"},
        "nums": {"type": "array", "items": {"type": "integer"}},
        "out": {"type": "array", "items": {"type": "integer"}},
        "items": {"type": "array", "items": {"type": "object", "properties": {
            "qty": {"type": "integer"}, "price": {"type": "number"}}, "required": ["qty", "price"]}},
        "one": {"type": "object", "properties": {
            "qty": {"type": "integer"}, "price": {"type": "number"}}, "required": ["qty", "price"]}
      }, "required": ["n", "x", "nums", "out", "items", "one"]}"#;
    let (mut calc, engine) = book(schema, &data);

    println!("\n--- one proxy read (budget {ATTRIBUTE_READ:?} or less)");
    const LOOP: usize = 200_000;
    let base = per_operation(
        &mut calc,
        &engine,
        "n",
        &format!("sum(1 for _ in range({LOOP})) * 0"),
    );
    let attribute = per_operation(
        &mut calc,
        &engine,
        "n",
        &format!("sum(S.one.qty for _ in range({LOOP})) * 0"),
    );
    let index = per_operation(
        &mut calc,
        &engine,
        "n",
        &format!("sum(S.nums[5] for _ in range({LOOP})) * 0"),
    );
    let per_attribute = (attribute.saturating_sub(base)) / (2 * LOOP as u32);
    let per_index = (index.saturating_sub(base)) / (2 * LOOP as u32);
    println!("per attribute read (S.one.qty is two reads): {per_attribute:?}");
    println!("per index read (S.nums[5] is two reads):     {per_index:?}");

    // Back to a literal, so that the loops above do not run again below.
    calc.set_value("S", &p("n"), Value::Int(0)).unwrap();

    println!(
        "\n--- ten thousand formulas, each reading a few nodes (budget {TEN_THOUSAND_FORMULAS:?} or less, {TEN_THOUSAND_FORMULAS_FIRST:?} for the first)"
    );
    for i in 0..N {
        calc.set_formula(
            "S",
            &p(&format!("out[{i}]")),
            &format!("S.nums[{i}] * 2 + S.nums[{}]", (i + 1) % N),
        )
        .unwrap();
    }
    let (_, many) = time("10,000 formulas, first calculation", || {
        calc.recalc(&engine)
    });
    assert_eq!(
        calc.data("S")
            .unwrap()
            .get(&p("out[3]"))
            .unwrap()
            .value()
            .clone(),
        Value::Int(10)
    );
    calc.recalc_all();
    let snapshot = calc.snapshot();
    let (evaluation, evaluating) = time("  evaluating", || snapshot.evaluate(&engine));
    let (_, applying) = time("  applying", || calc.apply(evaluation));
    let again = evaluating + applying;
    println!("10,000 formulas, recalculation (dependencies known)  {again:?}");

    println!("\n--- one aggregate over ten thousand items (budget {ONE_AGGREGATE:?} or less)");
    let aggregate_ints = per_operation(&mut calc, &engine, "n", "sum(S.nums) * 0");
    let aggregate_fields = per_operation(
        &mut calc,
        &engine,
        "x",
        "sum(i.qty * i.price for i in S.items) * 0.0",
    );
    let projected = per_operation(&mut calc, &engine, "x", "sum(S.items.price) * 0.0");

    println!("\n--- summary");
    println!("one attribute read   {per_attribute:>10.2?}  (budget {ATTRIBUTE_READ:?})");
    println!(
        "10,000 formulas      {many:>10.2?}  first, {again:.2?} again (budget {TEN_THOUSAND_FORMULAS:?})"
    );
    println!("aggregate, integers  {aggregate_ints:>10.2?}  (budget {ONE_AGGREGATE:?})");
    println!("aggregate, fields    {aggregate_fields:>10.2?}  (budget {AGGREGATE_TWO_FIELDS:?})");
    println!("aggregate, projected {projected:>10.2?}  (budget {AGGREGATE_PROJECTION:?})");

    let mut over = Vec::new();
    if per_attribute > ATTRIBUTE_READ {
        over.push(format!("attribute read {per_attribute:?}"));
    }
    if many > TEN_THOUSAND_FORMULAS_FIRST {
        over.push(format!("10,000 formulas, first calculation {many:?}"));
    }
    if again > TEN_THOUSAND_FORMULAS {
        over.push(format!("10,000 formulas, recalculation {again:?}"));
    }
    if aggregate_ints > ONE_AGGREGATE {
        over.push(format!("aggregate over integers {aggregate_ints:?}"));
    }
    if projected > AGGREGATE_PROJECTION {
        over.push(format!("aggregate over a projection {projected:?}"));
    }
    if aggregate_fields > AGGREGATE_TWO_FIELDS {
        over.push(format!("aggregate over two fields {aggregate_fields:?}"));
    }
    assert!(over.is_empty(), "over budget: {}", over.join("; "));
}

/// The same ten thousand formulas with the stand-in engine, to tell the cost
/// of the calculation layer from the cost of Python.
#[test]
#[ignore = "a measurement, for release builds"]
fn calculation_layer_alone() {
    use cliotree_calc::fake::FakeEngine;
    let numbers: Vec<String> = (0..N).map(|i| i.to_string()).collect();
    let data = format!(
        r#"{{"nums": [{}], "out": [{}]}}"#,
        numbers.join(","),
        vec!["0"; N].join(",")
    );
    let schema = r#"{"type": "object", "properties": {
        "nums": {"type": "array", "items": {"type": "integer"}},
        "out": {"type": "array", "items": {"type": "integer"}}
      }, "required": ["nums", "out"]}"#;
    let engine = FakeEngine::new();
    let mut calc = Calc::new(Arc::new(engine.clone()));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(schema).unwrap(),
        json::parse(&data).unwrap().node,
    )
    .unwrap();
    let (_, entering) = time("entering 10,000 formulas", || {
        for i in 0..N {
            calc.set_formula(
                "S",
                &p(&format!("out[{i}]")),
                &format!("S.nums[{i}] * 2 + S.nums[{}]", (i + 1) % N),
            )
            .unwrap();
        }
    });
    let (_, first) = time("10,000 formulas, first calculation", || {
        calc.recalc(&engine)
    });
    calc.recalc_all();
    let (_, again) = time("10,000 formulas, recalculation", || calc.recalc(&engine));
    println!("entering {entering:?}, first {first:?}, again {again:?}");
}

/// Where the time of one formula goes, by calling the engine directly.
#[test]
#[ignore = "a measurement, for release builds"]
fn cost_of_one_evaluation() {
    use cliotree_calc::{Context, Depth, Engine, Read};
    use cliotree_core::Node;
    use std::collections::HashMap;
    use std::rc::Rc;

    struct Fixed {
        path: Path,
        data: HashMap<String, Node>,
    }
    impl Context for Fixed {
        fn sheet(&self) -> &str {
            "S"
        }
        fn path(&self) -> &Path {
            &self.path
        }
        fn read(&self, sheet: &str, path: &Path, _: Depth) -> Read {
            match self.data.get(sheet).and_then(|n| n.get(path)) {
                Some(n) => Read::Node(n.clone()),
                None => Read::Missing,
            }
        }
        fn record(&self, _: &str, _: &Path, _: Depth) {}
        fn absent_ok(&self, _: &str, _: &Path) -> bool {
            false
        }
        fn sheets(&self) -> Vec<String> {
            vec!["S".to_owned()]
        }
    }
    let numbers: Vec<String> = (0..100).map(|i| i.to_string()).collect();
    let root = json::parse(&format!(r#"{{"nums": [{}]}}"#, numbers.join(",")))
        .unwrap()
        .node;
    let ctx: Rc<dyn Context> = Rc::new(Fixed {
        path: p("out[1]"),
        data: HashMap::from([("S".to_owned(), root)]),
    });
    let engine = PythonEngine::new();
    for text in ["1", "S", "S.nums", "S.nums[1]", "S.nums[1] * 2 + S.nums[2]"] {
        let _ = engine.eval(text, ctx.clone());
        let (_, took) = time(&format!("10,000 x {text}"), || {
            for _ in 0..N {
                engine.eval(text, ctx.clone()).unwrap();
            }
        });
        println!("    {:?} each", took / N as u32);
    }
}

/// Ten thousand formulas of different shapes through the whole stack.
#[test]
#[ignore = "a measurement, for release builds"]
fn ten_thousand_formulas_by_shape() {
    let numbers: Vec<String> = (0..N).map(|i| i.to_string()).collect();
    let data = format!(
        r#"{{"nums": [{}], "out": [{}]}}"#,
        numbers.join(","),
        vec!["0"; N].join(",")
    );
    let schema = r#"{"type": "object", "properties": {
        "nums": {"type": "array", "items": {"type": "integer"}},
        "out": {"type": "array", "items": {"type": "integer"}}
      }, "required": ["nums", "out"]}"#;
    for (name, make) in [
        (
            "1",
            Box::new(|_: usize| "1".to_owned()) as Box<dyn Fn(usize) -> String>,
        ),
        ("S.nums[0]", Box::new(|_| "S.nums[0]".to_owned())),
        ("S.nums[i]", Box::new(|i| format!("S.nums[{i}]"))),
        ("1 + i (distinct text)", Box::new(|i| format!("1 + {i}"))),
    ] {
        let (mut calc, engine) = book(schema, &data);
        for i in 0..N {
            calc.set_formula("S", &p(&format!("out[{i}]")), &make(i))
                .unwrap();
        }
        let (_, first) = time(&format!("{name}: first calculation"), || {
            calc.recalc(&engine)
        });
        calc.recalc_all();
        let (_, again) = time(&format!("{name}: again"), || calc.recalc(&engine));
        println!(
            "    {:?} / {:?} per formula",
            first / N as u32,
            again / N as u32
        );
    }
}
