//! Tests of the worker protocol (DESIGN.md, Web port, 0.3.0): a job that goes
//! through the encoding, is run, and whose result comes back through it,
//! gives what running it directly gives. They run the real interpreter.

use std::sync::Arc;

use cliotree_core::{Path, Value, json, jsonschema};

use super::{Done, Job, decode, encode, run};
use crate::python::{PythonEngine, PythonSyntax};
use crate::{Calc, Call, Immediate};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "name": {"type": "string"},
    "count": {"type": "integer"},
    "total": {"type": "number"},
    "label": {"type": "string"},
    "bad": {"type": "number"},
    "today": {"type": "string"},
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {"qty": {"type": "integer"}, "price": {"type": "number"}},
        "required": ["qty", "price"]
      }
    }
  },
  "required": ["name", "count", "items"]
}"#;

const DATA: &str =
    r#"{"name": "a", "count": 1, "items": [{"qty": 2, "price": 1.5}, {"qty": 3, "price": 2.0}]}"#;

const RATES_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {"vat": {"type": "number"}},
  "required": ["vat"]
}"#;

const MODULE: &str = "\
def gross(net):
    return round(net * (1 + Rates.vat), 2)

def go(form, ctx):
    form.name = 'changed'
    form.count = form.count + 1
    form.items.append({'qty': 1, 'price': 9.0})
";

/// The formulas, and the paths whose cells are compared.
const FORMULAS: &[(&str, &str)] = &[
    ("total", "gross(sum(i.qty * i.price for i in S.items))"),
    ("label", "f'{S.name}: {len(S.items)} lines'"),
    ("bad", "1 / 0"),
];

fn p(text: &str) -> Path {
    text.parse().unwrap()
}

fn workbook() -> Calc {
    let mut calc = Calc::new(Arc::new(PythonSyntax));
    calc.add_sheet(
        "S",
        jsonschema::lower_str(SCHEMA).unwrap(),
        json::parse(DATA).unwrap().node,
    )
    .unwrap();
    calc.add_sheet(
        "Rates",
        jsonschema::lower_str(RATES_SCHEMA).unwrap(),
        json::parse(r#"{"vat": 0.21}"#).unwrap().node,
    )
    .unwrap();
    calc.add_module("book", MODULE).unwrap();
    for (path, formula) in FORMULAS {
        calc.set_formula("S", &p(path), formula).unwrap();
    }
    calc
}

/// Sends a job through the encoding both ways, as the page and the worker
/// do, and runs it on the other side.
fn through_the_wire(engine: &PythonEngine, job: &Job) -> Done {
    let bytes = encode(job);
    let job: Job = decode(&bytes).unwrap();
    let done = run(engine, job);
    decode(&encode(&done)).unwrap()
}

/// What a cell shows: its value as JSON text, or its error.
fn shown(calc: &Calc, path: &str) -> String {
    let cell = calc.cell("S", &p(path)).unwrap();
    if let Some(error) = cell.error {
        return format!("{:?}: {}", error.kind, error.message);
    }
    match cell.node {
        Some(node) => json::to_string(node, false).unwrap(),
        None => "absent".to_owned(),
    }
}

/// Brings the workbook up to date through the wire, pass by pass, as the
/// page's session does.
fn recalc_through_the_wire(calc: &mut Calc, engine: &PythonEngine) {
    for _ in 0..10 {
        let snapshot = calc.snapshot();
        if snapshot.dirty.is_empty() && snapshot.exprs.is_empty() {
            return;
        }
        let job = Job::Evaluate {
            snapshot,
            recalc_limit: None,
        };
        match through_the_wire(engine, &job) {
            Done::Evaluation(evaluation) => {
                calc.apply(evaluation);
            }
            other => panic!("an evaluation came back as {other:?}"),
        }
    }
    panic!("the workbook did not settle in ten passes");
}

#[test]
fn formulas_calculated_through_the_wire_match_those_calculated_directly() {
    let engine = PythonEngine::new();
    let mut direct = workbook();
    direct.recalc(&engine);
    let mut wired = workbook();
    recalc_through_the_wire(&mut wired, &engine);
    for (path, _) in FORMULAS {
        assert_eq!(shown(&wired, path), shown(&direct, path), "at {path}");
    }
    assert_eq!(shown(&wired, "total"), "10.89");
    assert!(shown(&wired, "bad").starts_with("DivZero"));

    // An edit afterwards: only the formulas that read it run again.
    for calc in [&mut direct, &mut wired] {
        calc.set_value("Rates", &p("vat"), Value::Float(0.09))
            .unwrap();
    }
    direct.recalc(&engine);
    recalc_through_the_wire(&mut wired, &engine);
    assert_eq!(shown(&wired, "total"), shown(&direct, "total"));
    assert_eq!(shown(&wired, "total"), "9.81");
}

#[test]
fn a_procedure_run_through_the_wire_makes_its_writes() {
    let engine = PythonEngine::new();
    let mut calc = workbook();
    recalc_through_the_wire(&mut calc, &engine);
    let call = Call {
        procedure: "go".to_owned(),
        sheet: "S".to_owned(),
        form: "Default".to_owned(),
        button: "Go".to_owned(),
        value: None,
        rows: Vec::new(),
        time_limit: None,
    };
    let job = Job::Call {
        snapshot: calc.snapshot(),
        call,
    };
    let Done::Procedure(result) = through_the_wire(&engine, &job) else {
        panic!("a procedure did not come back as one");
    };
    calc.apply_procedure(result).unwrap();
    recalc_through_the_wire(&mut calc, &engine);
    assert_eq!(shown(&calc, "name"), r#""changed""#);
    assert_eq!(shown(&calc, "count"), "2");
    assert_eq!(shown(&calc, "label"), r#""changed: 3 lines""#);
}

#[test]
fn an_immediate_expression_runs_through_the_wire() {
    let engine = PythonEngine::new();
    let calc = workbook();
    let immediate = Immediate {
        id: 7,
        text: "gross(100)".to_owned(),
        sheet: "S".to_owned(),
        path: Path::root(),
        time_limit: None,
    };
    let job = Job::Immediate {
        snapshot: calc.snapshot(),
        immediate,
    };
    let Done::Immediate(result) = through_the_wire(&engine, &job) else {
        panic!("an Immediate expression did not come back as one");
    };
    assert_eq!(result.id, 7);
    assert_eq!(result.result, Ok("121.0".to_owned()));
}

#[test]
fn a_decoded_snapshot_has_the_same_data_with_fresh_node_ids() {
    let calc = workbook();
    let snapshot = calc.snapshot();
    let decoded: crate::Snapshot = decode(&encode(&snapshot)).unwrap();
    let before = &snapshot.sheets["S"].data;
    let after = &decoded.sheets["S"].data;
    assert_eq!(
        json::to_string(after, false).unwrap(),
        json::to_string(before, false).unwrap()
    );
    assert_ne!(after.id(), before.id());
    assert_eq!(decoded.version(), snapshot.version());
}

#[test]
fn bytes_that_are_not_a_message_are_refused() {
    let refused = decode::<Job>(b"not a job");
    assert!(refused.unwrap_err().contains("did not read"));
}
