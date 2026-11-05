//! The Python side of the wasm version: the engine and the calculation layer,
//! run in a Web Worker so that the page never runs Python (DESIGN.md, Web
//! port, 0.3.0).
//!
//! [`WorkerSide`] is the worker: it takes the messages of the worker protocol
//! (`gossia_calc::wire`), runs the jobs and posts the results. [`PageProbe`]
//! is the page's side for the probe page: a workbook in a real
//! `gossia_calc::Session`, whose evaluator reaches the worker through
//! `JsLink`. The application takes its place in 13b-2.
//!
//! Everything here is for wasm; on other targets the crate is empty.

#![cfg(target_family = "wasm")]

#[cfg(feature = "test-control")]
pub mod control;
pub mod page;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gossia_calc::python::{PythonSettings, PythonSyntax};
use gossia_calc::wire::{self, ToWorker};
use gossia_calc::{Calc, EvalSettings, Evaluator, Session, WorkerLink};
use gossia_core::{Path, Value, json, jsonschema};
use js_sys::{Function, Uint8Array};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

thread_local! {
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Keeps the message of a panic, which wasm otherwise reports only as
/// "unreachable", for [`last_panic`].
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        let text = info.to_string();
        LAST_PANIC.with(|last| *last.borrow_mut() = Some(text));
    }));
}

/// The message of the last panic, if there was one.
#[wasm_bindgen]
pub fn last_panic() -> Option<String> {
    LAST_PANIC.with(|last| last.borrow().clone())
}

fn call(f: &Function, bytes: &[u8]) {
    // A failure in the glue shows in the console; there is nobody to tell here.
    let _ = f.call1(&JsValue::NULL, &Uint8Array::from(bytes));
}

/// The worker: the protocol's worker side (`wire::Worker`), posting its
/// messages to the page.
#[wasm_bindgen]
pub struct WorkerSide {
    post: Function,
    worker: wire::Worker,
}

#[wasm_bindgen]
impl WorkerSide {
    /// `post` is called with the bytes of each message for the page.
    #[wasm_bindgen(constructor)]
    pub fn new(post: Function) -> WorkerSide {
        WorkerSide {
            post,
            worker: wire::Worker::default(),
        }
    }

    /// Takes one message from the page. A job runs to the end before this
    /// returns; the page learns of each formula as it starts.
    pub fn handle(&mut self, message: &[u8]) -> Result<(), String> {
        let message = wire::decode::<ToWorker>(message)?;
        let post = &self.post;
        self.worker
            .handle(message, &mut |reply| call(post, &wire::encode(&reply)));
        Ok(())
    }
}

/// The page's link to the worker, through two JavaScript functions: `start`
/// ends the worker there is and starts a new one, `post` sends it bytes. The
/// page hands what the worker sends to [`PageProbe::receive`].
struct JsLink {
    start: Function,
    post: Function,
    inbox: Rc<RefCell<Vec<Vec<u8>>>>,
}

impl WorkerLink for JsLink {
    fn start(&self) {
        let _ = self.start.call0(&JsValue::NULL);
    }

    fn post(&self, message: Vec<u8>) {
        call(&self.post, &message);
    }

    fn take(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.inbox.borrow_mut())
    }
}

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "name": {"type": "string"},
    "total": {"type": "number"},
    "label": {"type": "string"},
    "bad": {"type": "number"},
    "spin": {"type": "number"},
    "items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {"qty": {"type": "integer"}, "price": {"type": "number"}},
        "required": ["qty", "price"]
      }
    }
  },
  "required": ["name", "items"]
}"#;

const DATA: &str =
    r#"{"name": "a", "items": [{"qty": 2, "price": 1.5}, {"qty": 3, "price": 2.0}]}"#;

const RATES_SCHEMA: &str =
    r#"{"type": "object", "properties": {"vat": {"type": "number"}}, "required": ["vat"]}"#;

const MODULE: &str = "def gross(net):\n    return round(net * (1 + Rates.vat), 2)\n";

fn text<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

/// A workbook for the probe page: tab `S` with formulas that use tab `Rates`
/// and a module, evaluated in the worker.
#[wasm_bindgen]
pub struct PageProbe {
    session: Session,
    inbox: Rc<RefCell<Vec<Vec<u8>>>>,
}

#[wasm_bindgen]
impl PageProbe {
    /// `formula_ms` is the formula time limit; the grace is a quarter of it.
    #[wasm_bindgen(constructor)]
    pub fn new(start: Function, post: Function, formula_ms: u32) -> Result<PageProbe, String> {
        let mut calc = Calc::new(Arc::new(PythonSyntax));
        let spec = jsonschema::lower_str(SCHEMA).map_err(text)?;
        let data = json::parse(DATA).map_err(text)?.node;
        calc.add_sheet("S", spec, data).map_err(text)?;
        let spec = jsonschema::lower_str(RATES_SCHEMA).map_err(text)?;
        let data = json::parse(r#"{"vat": 0.21}"#).map_err(text)?.node;
        calc.add_sheet("Rates", spec, data).map_err(text)?;
        calc.add_module("book", MODULE).map_err(text)?;
        let inbox = Rc::new(RefCell::new(Vec::new()));
        let link = JsLink {
            start,
            post,
            inbox: Rc::clone(&inbox),
        };
        let limit = Duration::from_millis(u64::from(formula_ms));
        let settings = EvalSettings {
            formula_limit: Some(limit),
            grace: limit / 4,
            ..EvalSettings::default()
        };
        let evaluator = Evaluator::new(Box::new(link), PythonSettings::default(), settings);
        Ok(PageProbe {
            session: Session::new(calc, evaluator),
            inbox,
        })
    }

    /// A message from the worker.
    pub fn receive(&self, message: &[u8]) {
        self.inbox.borrow_mut().push(message.to_vec());
    }

    /// Takes what has come back and starts what is due, as the window does
    /// once per frame. Says whether the workbook is idle.
    pub fn tick(&mut self) -> bool {
        self.session.tick();
        self.session.is_idle()
    }

    pub fn set_formula(&mut self, path: &str, formula: &str) -> Result<(), String> {
        let path: Path = path.parse().map_err(text)?;
        self.session
            .calc_mut()
            .set_formula("S", &path, formula)
            .map_err(text)
    }

    pub fn set_vat(&mut self, vat: f64) -> Result<(), String> {
        let path: Path = "vat".parse().map_err(text)?;
        self.session
            .calc_mut()
            .set_value("Rates", &path, Value::Float(vat))
            .map_err(text)
    }

    /// What a cell of `S` shows: its value as JSON text, or its error.
    pub fn cell(&self, path: &str) -> String {
        let Ok(path) = path.parse::<Path>() else {
            return "bad path".to_owned();
        };
        let Some(cell) = self.session.calc().cell("S", &path) else {
            return "absent".to_owned();
        };
        if let Some(error) = cell.error {
            return format!("{:?}: {}", error.kind, error.message);
        }
        match cell.node {
            Some(node) => json::to_string(node, false).unwrap_or_else(text),
            None => "absent".to_owned(),
        }
    }
}
