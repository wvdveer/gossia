//! Tests of the Python engine, running formulas through the real calculation
//! layer. Each says which part of DESIGN.md it checks.

use std::sync::Arc;

use gossia_core::{ErrorKind, Path, Value, json, jsonschema};

use super::{PythonEngine, PythonSyntax};
use crate::{Calc, EditError};

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "a": {"type": "integer"},
    "b": {"type": "integer"},
    "c": {"type": "integer"},
    "x": {"type": "number"},
    "name": {"type": "string"},
    "flag": {"type": "boolean"},
    "note": {"type": "string"},
    "items": {"type": "array", "items": {"type": "object", "properties": {
        "qty": {"type": "integer"},
        "price": {"type": "number"},
        "total": {"type": "number"},
        "tag": {"type": "string"}
      }, "required": ["qty", "price"]}},
    "tags": {"type": "array", "items": {"type": "string"}},
    "nums": {"type": "array", "items": {"type": "integer"}},
    "copy": {"type": "array", "items": {"type": "integer"}},
    "by": {"type": "object", "additionalProperties": {"type": "integer"}},
    "sum": {"type": "number"},
    "text": {"type": "string"}
  },
  "required": ["a", "b", "c", "x", "name", "flag", "items", "tags", "nums", "copy", "by", "sum", "text"]
}"#;

const DATA: &str = r#"{
  "a": 1, "b": 2, "c": 0, "x": 1.5, "name": "widget", "flag": true,
  "items": [
    {"qty": 2, "price": 1.5},
    {"qty": 3, "price": 2.0},
    {"qty": 4, "price": 0.5, "tag": "sale"}
  ],
  "tags": ["x", "y"],
  "nums": [10, 20, 30],
  "copy": [],
  "by": {"one": 1, "two": 2},
  "sum": 0, "text": ""
}"#;

pub(super) fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

pub(super) struct Book {
    pub(super) calc: Calc,
    pub(super) engine: PythonEngine,
}

impl Book {
    pub(super) fn new() -> Book {
        let mut calc = Calc::new(Arc::new(PythonSyntax));
        let spec = jsonschema::lower_str(SCHEMA).unwrap();
        let data = json::parse(DATA).unwrap().node;
        calc.add_sheet("S", spec.clone(), data.clone()).unwrap();
        // A second tab with the same data and no formulas, to search without
        // being circular with the formulas on `S`.
        calc.add_sheet("D", spec, data).unwrap();
        Book {
            calc,
            engine: PythonEngine::new(),
        }
    }

    pub(super) fn formula(&mut self, path: &str, text: &str) {
        self.calc.set_formula("S", &p(path), text).unwrap();
    }

    pub(super) fn set(&mut self, path: &str, value: Value) {
        self.calc.set_value("S", &p(path), value).unwrap();
    }

    pub(super) fn run(&mut self) -> usize {
        self.calc.recalc(&self.engine)
    }

    pub(super) fn get(&self, path: &str) -> Option<Value> {
        self.calc
            .data("S")
            .unwrap()
            .get(&p(path))
            .map(|n| n.value().clone())
    }

    pub(super) fn error(&self, path: &str) -> Option<ErrorKind> {
        match self.get(path) {
            Some(Value::Error(e)) => Some(e.kind),
            _ => None,
        }
    }

    pub(super) fn message(&self, path: &str) -> String {
        match self.get(path) {
            Some(Value::Error(e)) => e.message,
            other => panic!("no error at {path}: {other:?}"),
        }
    }
}

#[test]
fn a_formula_is_python_and_reads_the_tab_by_name() {
    let mut b = Book::new();
    b.formula("c", "S.a + S.b * 10");
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(21)));
}

#[test]
fn syntax_errors_are_found_on_entry() {
    let mut b = Book::new();
    b.formula("c", "1 +");
    assert_eq!(b.error("c"), Some(ErrorKind::Syntax));
    let err = b.calc.set_formula("S", &p("nothing"), "1").unwrap_err();
    assert!(matches!(err, EditError::NotInSchema(_)));
}
