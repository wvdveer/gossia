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
//! - `backupWorkbook(bytes)` and `bytesFailed(kind, message)`: the answers to
//!   [`request_bytes`], for a host that saves and backs up the file itself
//!   (VS Code). A save asked for that way is answered with `saveWorkbook`.
//!
//! Such a host also hands the page a workbook while it runs
//! ([`open_workbook`], for Revert).

use gossia_i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gossia_app::GossiaApp;
use gossia_app::host::{Host, ModuleEdit};
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
    /// What the host has asked of the application, until the next frame.
    static REQUESTS: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
}

/// Something the host asks of the application, done in the next frame.
enum Request {
    /// Show this workbook in place of the one there is (empty bytes: a blank
    /// workbook, as a new empty file is).
    Open { name: String, bytes: Vec<u8> },
    /// The workbook's bytes: `save` (counted as saved, given with
    /// `saveWorkbook`) or `backup` (given with `backupWorkbook`).
    Bytes(String),
    /// One of Gossia's commands, by name, from the host's command list.
    Command(String),
}

/// Runs one of Gossia's commands by its name (`gossia_app::commands`), as
/// the host's command list (VS Code's Command Palette) asks. A command that
/// is refused says why in the window.
#[wasm_bindgen]
pub fn run_command(name: String) {
    REQUESTS.with(|r| r.borrow_mut().push(Request::Command(name)));
    repaint();
}

/// The names and titles of Gossia's commands, as a flat list.
#[wasm_bindgen]
pub fn command_list() -> Vec<String> {
    gossia_app::commands::COMMANDS
        .iter()
        .flat_map(|(name, title)| [(*name).to_owned(), (*title).to_owned()])
        .collect()
}

/// A module's file saved in the host's editor: its source, or a new module.
/// Answered with `moduleAnswer(id, error)`.
#[wasm_bindgen]
pub fn module_written(id: u32, name: String, text: String) {
    gossia_app::host_code::module_edited(ModuleEdit::Write { id, name, text });
    repaint();
}

/// A module's file renamed in the host's editor.
#[wasm_bindgen]
pub fn module_renamed(id: u32, from: String, to: String) {
    gossia_app::host_code::module_edited(ModuleEdit::Rename { id, from, to });
    repaint();
}

/// A module's file deleted in the host's editor.
#[wasm_bindgen]
pub fn module_removed(id: u32, name: String) {
    gossia_app::host_code::module_edited(ModuleEdit::Remove { id, name });
    repaint();
}

/// Opens this workbook in place of the one there is, as the host asks when
/// it reverts the file. Empty bytes give a blank workbook.
#[wasm_bindgen]
pub fn open_workbook(name: String, bytes: Vec<u8>) {
    REQUESTS.with(|r| r.borrow_mut().push(Request::Open { name, bytes }));
    repaint();
}

/// Asks for the workbook's bytes, once nothing is being calculated: `kind`
/// is `save` or `backup`. The answer is `saveWorkbook(name, bytes)`,
/// `backupWorkbook(bytes)`, or `bytesFailed(kind, message)`.
#[wasm_bindgen]
pub fn request_bytes(kind: String) {
    REQUESTS.with(|r| r.borrow_mut().push(Request::Bytes(kind)));
    repaint();
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
    /// The host has an editor of its own for modules (`openModule`).
    edits_modules: bool,
}

impl JsHost {
    /// Calls one of the object's functions. A missing function, or one that
    /// throws, is reported in the console: there is nobody else to tell.
    fn call(&self, name: &str, args: &[JsValue]) -> Result<JsValue, String> {
        let f = Reflect::get(&self.object, &JsValue::from_str(name))
            .ok()
            .and_then(|f| f.dyn_into::<Function>().ok())
            .ok_or_else(|| tr!("the host has no {name}()", name = name))?;
        let args: js_sys::Array = args.iter().collect();
        f.apply(&self.object, &args).map_err(|e| {
            tr!(
                "the host's {name}() failed: {e}",
                name = name,
                e = format!("{e:?}")
            )
        })
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
                edits_modules: false,
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

    fn edits_modules(&self) -> bool {
        self.edits_modules
    }

    fn open_module(&self, name: &str, line: usize, column: usize) {
        self.call_logged(
            "openModule",
            &[
                JsValue::from_str(name),
                JsValue::from_f64(line as f64),
                JsValue::from_f64(column as f64),
            ],
        );
    }

    fn modules_changed(&self, modules: &[(String, String)]) {
        let list: js_sys::Array = modules
            .iter()
            .map(|(name, source)| {
                let pair: js_sys::Array = [JsValue::from_str(name), JsValue::from_str(source)]
                    .into_iter()
                    .collect();
                JsValue::from(pair)
            })
            .collect();
        self.call_logged("modulesChanged", &[list.into()]);
    }

    fn module_answer(&self, id: u32, error: Option<&str>) {
        let error = error.map_or(JsValue::NULL, JsValue::from_str);
        self.call_logged("moduleAnswer", &[JsValue::from_f64(f64::from(id)), error]);
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

/// The application, with what the host asks of it done before each frame.
struct PageApp {
    app: GossiaApp,
    host: Rc<JsHost>,
}

impl PageApp {
    fn requests(&mut self, ctx: &egui::Context) {
        let requests = REQUESTS.with(|r| std::mem::take(&mut *r.borrow_mut()));
        let mut later = Vec::new();
        for request in requests {
            match request {
                Request::Open { name, bytes } => {
                    let opened = if bytes.is_empty() {
                        self.app.new_workbook();
                        Ok(())
                    } else {
                        self.app.open_bytes(&name, &bytes)
                    };
                    if let Err(e) = opened {
                        self.app.notice = Some(e);
                    }
                }
                // The bytes wait for the calculation, which the page cannot
                // wait for within a frame.
                Request::Bytes(kind) if !self.app.session.wait_idle(Duration::ZERO) => {
                    later.push(Request::Bytes(kind));
                }
                Request::Command(name) => {
                    if let Err(e) = self.app.run_command(&name) {
                        self.app.notice = Some(format!("{e}."));
                    }
                }
                Request::Bytes(kind) => {
                    let result = match kind.as_str() {
                        "save" => self.app.save_to_host(&*self.host),
                        "backup" => self.app.backup_bytes().map(|bytes| {
                            let bytes = Uint8Array::from(bytes.as_slice());
                            self.host.call_logged("backupWorkbook", &[bytes.into()]);
                        }),
                        other => Err(tr!(
                            "{other} is not a kind of bytes to ask for",
                            other = other
                        )),
                    };
                    if let Err(e) = result {
                        self.host.call_logged(
                            "bytesFailed",
                            &[JsValue::from_str(&kind), JsValue::from_str(&e)],
                        );
                    }
                }
            }
        }
        if !later.is_empty() {
            REQUESTS.with(|r| {
                let mut r = r.borrow_mut();
                later.append(&mut r);
                *r = later;
            });
            ctx.request_repaint();
        }
    }
}

impl eframe::App for PageApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.requests(&ctx);
        #[cfg(feature = "test-control")]
        crate::control::answer(&mut self.app, &ctx);
        eframe::App::ui(&mut self.app, ui, frame);
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
    // A host with an editor of its own for modules says so by having
    // `openModule`.
    let edits_modules =
        Reflect::get(&host, &JsValue::from_str("openModule")).is_ok_and(|f| f.is_function());
    let host = Rc::new(JsHost {
        object: host,
        edits_modules,
    });
    gossia_app::host::set(host.clone());
    let page_host = host.clone();
    let (user, mut warnings) = User::load(Config::in_storage(Rc::new(HostStorage(host.clone()))));
    gossia_app::host::load_sources(&*host, &user.sources, &mut warnings);
    let mut app = if sample {
        GossiaApp::sample()
    } else {
        GossiaApp::new(gossia_app::workbook_file::blank_workbook())
    }
    .with_user(user);
    if let (Some(name), Some(bytes)) = (name, bytes)
        && !bytes.is_empty()
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
                Ok(Box::new(PageApp {
                    app,
                    host: page_host,
                }))
            }),
        )
        .await;
    RUNNER.with(|r| *r.borrow_mut() = Some(runner));
    started
}
