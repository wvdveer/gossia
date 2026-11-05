//! The application in the page: `gossia-app` drawn by egui on a canvas,
//! with a host that is a JavaScript object (DESIGN.md, Web port, 0.3.0, The
//! host interface). The plain browser page of the tests gives one, the VS
//! Code extension's panel another.
//!
//! The host object has these functions:
//! - `startWorker()` ends the Python worker there is and starts a new one;
//!   `postWorker(bytes)` sends it a message. The page hands what the worker
//!   sends to [`worker_message`].
//! - `saveWorkbook(name, bytes)` and `setDirty(dirty)`, for the file.
//! - `pickFile(purpose, extensions)` asks for a file to read; the page hands
//!   it to [`file_picked`] with the same purpose. `saveFile(name, bytes)`
//!   asks where to save a file and writes it.
//! - `readStorage(name)` (text or `null`) and `writeStorage(name, text)`.
//! - `readSecret(name)` (text or `null`) and `writeSecret(name, text)`
//!   (`null` removes it): data source passwords and tokens. A host whose
//!   secret store answers later (VS Code's) reads them before the start.
//! - `openOutside(target)`.
//! - `dark()`: `true`, `false`, or `null` when the host does not say.
//! - `fetch(id, method, url, headers, body, timeoutMs, maxBytes)` makes an
//!   HTTP request for a data source (`headers` a flat list of names and
//!   values, `body` bytes or `null`), without following a redirect; the
//!   answer goes to [`fetch_answered`] or [`fetch_failed`] with the same id.

use std::cell::RefCell;
use std::rc::Rc;

use gossia_app::GossiaApp;
use gossia_app::host::Host;
use gossia_app::prefs::{Config, Storage, User};
use gossia_calc::WorkerLink;
use js_sys::{Function, Reflect, Uint8Array};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};

thread_local! {
    /// What the worker has sent, until the evaluator takes it.
    static INBOX: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    /// The window's context, to draw again when the worker answers.
    static CONTEXT: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    /// The runner, kept for the life of the page.
    static RUNNER: RefCell<Option<eframe::WebRunner>> = const { RefCell::new(None) };
}

/// A message from the Python worker.
#[wasm_bindgen]
pub fn worker_message(message: &[u8]) {
    INBOX.with(|inbox| inbox.borrow_mut().push(message.to_vec()));
    repaint();
}

/// A file the user picked in the host's dialog, for `purpose`.
#[wasm_bindgen]
pub fn file_picked(purpose: String, name: String, bytes: Vec<u8>) {
    gossia_app::host::picked(gossia_app::host::Picked {
        purpose,
        name,
        bytes,
    });
    repaint();
}

/// The host's answer to a request (`fetch`): `headers` a flat list of names
/// and values.
#[wasm_bindgen]
pub fn fetch_answered(id: f64, status: u16, reason: String, headers: Vec<String>, body: Vec<u8>) {
    let headers = headers
        .chunks(2)
        .filter_map(|pair| Some((pair.first()?.clone(), pair.get(1)?.clone())))
        .collect();
    let response = gossia_calc::http::HttpResponse {
        status,
        reason,
        headers,
        body,
    };
    gossia_app::host::fetched(id as u64, Ok(response));
    repaint();
}

/// A request the host could not make, and why, in words for the user.
#[wasm_bindgen]
pub fn fetch_failed(id: f64, message: String) {
    gossia_app::host::fetched(id as u64, Err(message));
    repaint();
}

pub(crate) fn repaint() {
    CONTEXT.with(|ctx| {
        if let Some(ctx) = &*ctx.borrow() {
            ctx.request_repaint();
        }
    });
}

/// The host, through its JavaScript object.
struct JsHost {
    object: JsValue,
}

impl JsHost {
    /// Calls one of the object's functions. A missing function, or one that
    /// throws, is reported in the console: there is nobody else to tell.
    fn call(&self, name: &str, args: &[JsValue]) -> Result<JsValue, String> {
        let f = Reflect::get(&self.object, &JsValue::from_str(name))
            .ok()
            .and_then(|f| f.dyn_into::<Function>().ok())
            .ok_or_else(|| format!("the host has no {name}()"))?;
        let args: js_sys::Array = args.iter().collect();
        f.apply(&self.object, &args)
            .map_err(|e| format!("the host's {name}() failed: {e:?}"))
    }

    fn call_logged(&self, name: &str, args: &[JsValue]) -> JsValue {
        self.call(name, args).unwrap_or_else(|e| {
            web_sys::console::error_1(&JsValue::from_str(&e));
            JsValue::UNDEFINED
        })
    }
}

impl Host for JsHost {
    fn worker(&self) -> Box<dyn WorkerLink> {
        Box::new(HostLink {
            host: JsHost {
                object: self.object.clone(),
            },
        })
    }

    fn save_workbook(&self, name: &str, bytes: Vec<u8>) {
        let bytes = Uint8Array::from(bytes.as_slice());
        self.call_logged("saveWorkbook", &[JsValue::from_str(name), bytes.into()]);
    }

    fn set_dirty(&self, dirty: bool) {
        self.call_logged("setDirty", &[JsValue::from_bool(dirty)]);
    }

    fn read_storage(&self, name: &str) -> Option<String> {
        self.call_logged("readStorage", &[JsValue::from_str(name)])
            .as_string()
    }

    fn write_storage(&self, name: &str, text: &str) -> Result<(), String> {
        self.call(
            "writeStorage",
            &[JsValue::from_str(name), JsValue::from_str(text)],
        )
        .map(|_| ())
    }

    fn read_secret(&self, name: &str) -> Option<String> {
        self.call_logged("readSecret", &[JsValue::from_str(name)])
            .as_string()
    }

    fn write_secret(&self, name: &str, text: Option<&str>) {
        let text = text.map_or(JsValue::NULL, JsValue::from_str);
        self.call_logged("writeSecret", &[JsValue::from_str(name), text]);
    }

    fn pick_file(&self, purpose: &str, extensions: &[&str]) {
        let extensions: js_sys::Array = extensions.iter().map(|e| JsValue::from_str(e)).collect();
        self.call_logged("pickFile", &[JsValue::from_str(purpose), extensions.into()]);
    }

    fn save_file(&self, name: &str, bytes: Vec<u8>) {
        let bytes = Uint8Array::from(bytes.as_slice());
        self.call_logged("saveFile", &[JsValue::from_str(name), bytes.into()]);
    }

    fn open_outside(&self, target: &str) {
        self.call_logged("openOutside", &[JsValue::from_str(target)]);
    }

    fn dark(&self) -> Option<bool> {
        self.call_logged("dark", &[]).as_bool()
    }

    fn fetch(&self, request: &gossia_calc::http::FetchRequest) {
        let headers: js_sys::Array = request
            .headers
            .iter()
            .flat_map(|(name, value)| [JsValue::from_str(name), JsValue::from_str(value)])
            .collect();
        let body = match &request.body {
            Some(body) => Uint8Array::from(body.as_slice()).into(),
            None => JsValue::NULL,
        };
        self.call_logged(
            "fetch",
            &[
                JsValue::from_f64(request.id as f64),
                JsValue::from_str(&request.method),
                JsValue::from_str(&request.url),
                headers.into(),
                body,
                JsValue::from_f64(request.timeout.as_millis() as f64),
                JsValue::from_f64(request.max_bytes as f64),
            ],
        );
    }
}

/// The preferences, the trust record and the Immediate window's history in
/// the host's storage.
struct HostStorage(Rc<JsHost>);

impl Storage for HostStorage {
    fn read(&self, name: &str) -> Option<String> {
        self.0.read_storage(name)
    }

    fn write(&self, name: &str, text: &str) -> Result<(), String> {
        self.0.write_storage(name, text)
    }
}

/// The evaluator's link to the worker, through the host.
struct HostLink {
    host: JsHost,
}

impl WorkerLink for HostLink {
    fn start(&self) {
        INBOX.with(|inbox| inbox.borrow_mut().clear());
        self.host.call_logged("startWorker", &[]);
    }

    fn post(&self, message: Vec<u8>) {
        let bytes = Uint8Array::from(message.as_slice());
        self.host.call_logged("postWorker", &[bytes.into()]);
    }

    fn take(&self) -> Vec<Vec<u8>> {
        INBOX.with(|inbox| std::mem::take(&mut *inbox.borrow_mut()))
    }
}

/// Starts the application on `canvas`, with `host`. `name` and `bytes` are
/// the workbook to open; without them a blank workbook is shown, or with
/// `sample` the sample workbook (for the test page).
#[wasm_bindgen]
pub async fn start_app(
    canvas: web_sys::HtmlCanvasElement,
    host: JsValue,
    name: Option<String>,
    bytes: Option<Vec<u8>>,
    sample: bool,
) -> Result<(), JsValue> {
    let host = Rc::new(JsHost { object: host });
    gossia_app::host::set(host.clone());
    let (user, mut warnings) = User::load(Config::in_storage(Rc::new(HostStorage(host.clone()))));
    gossia_app::host::load_sources(&*host, &user.sources, &mut warnings);
    let mut app = if sample {
        GossiaApp::sample()
    } else {
        GossiaApp::new(gossia_app::workbook_file::blank_workbook())
    }
    .with_user(user);
    if let (Some(name), Some(bytes)) = (name, bytes)
        && let Err(e) = app.open_bytes(&name, &bytes)
    {
        warnings.push(e);
    }
    if !warnings.is_empty() {
        app.notice = Some(warnings.join(" "));
    }
    let runner = eframe::WebRunner::new();
    let started = runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                CONTEXT.with(|ctx| *ctx.borrow_mut() = Some(cc.egui_ctx.clone()));
                #[cfg(feature = "test-control")]
                let app = crate::control::Controlled::new(app);
                Ok(Box::new(app))
            }),
        )
        .await;
    RUNNER.with(|r| *r.borrow_mut() = Some(runner));
    started
}
