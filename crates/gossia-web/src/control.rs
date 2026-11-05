//! The test build's way in (DESIGN.md, Web port, 0.3.0, Testing): the page
//! draws on a canvas and has no elements to query, so the `test-control`
//! feature gives it [`control`], which runs the commands of the test control
//! protocol (`gossia_app::control`) on the application, as the desktop's
//! loopback channel does. Release builds never have it.
//!
//! A command waits for the next frame, where the application is; the
//! answer is a promise. The page cannot block, so `wait_idle` is asked again
//! each frame until the workbook is idle or the time is up.

use std::cell::RefCell;

use gossia_app::{GossiaApp, control};
use js_sys::{Function, Promise};
use serde_json::Value as Json;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

/// The longest `wait_idle` waits, as on the desktop.
const MAX_WAIT_MS: f64 = 120_000.0;

/// A command on its way to the application, with its promise.
struct Job {
    cmd: String,
    args: Json,
    resolve: Function,
    reject: Function,
    /// For `wait_idle`, when it gives up (`Date.now()`), once it has started.
    until: Option<f64>,
}

thread_local! {
    static JOBS: RefCell<Vec<Job>> = const { RefCell::new(Vec::new()) };
}

/// Runs a command of the test control protocol: `args` is its `args` object
/// as JSON text (empty for none). The promise gives the result as JSON text,
/// or fails with the error.
#[wasm_bindgen]
pub fn control(cmd: String, args: String) -> Promise {
    let promise = Promise::new(&mut |resolve, reject| {
        let text = if args.trim().is_empty() {
            "null"
        } else {
            &args
        };
        match serde_json::from_str::<Json>(text) {
            Ok(args) => JOBS.with(|jobs| {
                jobs.borrow_mut().push(Job {
                    cmd: cmd.clone(),
                    args,
                    resolve,
                    reject,
                    until: None,
                });
            }),
            Err(e) => {
                let error = JsValue::from_str(&format!("the args are not JSON: {e}"));
                let _ = reject.call1(&JsValue::NULL, &error);
            }
        }
    });
    crate::page::repaint();
    promise
}

/// Runs the commands that have come for the application, and keeps the
/// waits that are not over for the next frame. Called by the page each frame.
pub(crate) fn answer(app: &mut GossiaApp, ctx: &egui::Context) {
    let jobs = JOBS.with(|jobs| std::mem::take(&mut *jobs.borrow_mut()));
    if jobs.is_empty() {
        return;
    }
    let mut waiting = Vec::new();
    for mut job in jobs {
        match job.cmd.as_str() {
            "wait_idle" => {
                let now = js_sys::Date::now();
                let asked = job
                    .args
                    .get("timeout_ms")
                    .and_then(Json::as_f64)
                    .unwrap_or(10_000.0);
                let until = *job.until.get_or_insert(now + asked.min(MAX_WAIT_MS));
                match control::run(app, "wait_idle", &job.args) {
                    Err(_) if now < until => waiting.push(job),
                    result => settle(&job, result),
                }
            }
            "quit" | "screenshot" => {
                let error = format!(
                    "{} is not done in the page: it has no window of its own, and the                      Playwright script takes the screenshots",
                    job.cmd
                );
                settle(&job, Err(error));
            }
            cmd => {
                let result = control::run(app, cmd, &job.args);
                settle(&job, result);
            }
        }
    }
    if !waiting.is_empty() {
        // Commands that came meanwhile go after the waits.
        JOBS.with(|jobs| {
            let mut jobs = jobs.borrow_mut();
            waiting.append(&mut jobs);
            *jobs = waiting;
        });
    }
    ctx.request_repaint();
}

fn settle(job: &Job, result: Result<Json, String>) {
    let _ = match result {
        Ok(value) => job
            .resolve
            .call1(&JsValue::NULL, &JsValue::from_str(&value.to_string())),
        Err(error) => job.reject.call1(&JsValue::NULL, &JsValue::from_str(&error)),
    };
}
