//! The Python side of the wasm version: the engine and the calculation layer,
//! run in a Web Worker so that the page never runs Python (DESIGN.md, Web
//! port, 0.3.0).
//!
//! Build step 13a has only a probe: it makes a workbook of one tab, sets one
//! formula, recalculates with the real Python engine and gives the result.
//! The worker's message protocol, the same as the desktop evaluation
//! thread's, comes with the host interface in 13b.

use std::cell::RefCell;
use std::sync::Arc;

use cliotree_calc::Calc;
use cliotree_calc::python::{PythonEngine, PythonSyntax};
use cliotree_core::{Path, json, jsonschema};
use serde_json::json;
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

/// A Python engine kept for the life of the worker, as the evaluation thread
/// keeps one.
#[wasm_bindgen]
pub struct Probe {
    engine: PythonEngine,
}

#[wasm_bindgen]
impl Probe {
    /// Starts the interpreter, which is the slow part.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Probe {
        Probe {
            engine: PythonEngine::new(),
        }
    }

    /// Makes a tab `S` from a JSON Schema and JSON data, sets `formula` (no
    /// leading `=`) at `path`, recalculates, and gives
    /// `{"value": ..., "error": ...}` as JSON text.
    pub fn run(&self, schema: &str, data: &str, path: &str, formula: &str) -> String {
        match self.try_run(schema, data, path, formula) {
            Ok(reply) => reply,
            Err(why) => json!({ "value": null, "error": why }).to_string(),
        }
    }
}

impl Default for Probe {
    fn default() -> Probe {
        Probe::new()
    }
}

impl Probe {
    fn try_run(
        &self,
        schema: &str,
        data: &str,
        path: &str,
        formula: &str,
    ) -> Result<String, String> {
        let spec = jsonschema::lower_str(schema).map_err(|e| format!("schema: {e:?}"))?;
        let data = json::parse(data).map_err(|e| format!("data: {e:?}"))?.node;
        let path: Path = path.parse().map_err(|e| format!("path: {e:?}"))?;
        let mut calc = Calc::new(Arc::new(PythonSyntax));
        calc.add_sheet("S", spec, data)
            .map_err(|e| format!("tab: {e:?}"))?;
        calc.set_formula("S", &path, formula)
            .map_err(|e| format!("formula: {e:?}"))?;
        calc.recalc(&self.engine);
        let cell = calc.cell("S", &path).ok_or("the node is not there")?;
        if let Some(error) = cell.error {
            return Ok(json!({
                "value": null,
                "error": format!("{:?}: {}", error.kind, error.message),
            })
            .to_string());
        }
        let value = match cell.node {
            Some(node) => {
                let text = json::to_string(node, false).map_err(|e| format!("{e:?}"))?;
                serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())?
            }
            None => serde_json::Value::Null,
        };
        Ok(json!({ "value": value, "error": null }).to_string())
    }
}
