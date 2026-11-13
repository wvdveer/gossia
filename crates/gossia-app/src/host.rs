//! The host interface (DESIGN.md, Web port, 0.3.0): what the surroundings
//! of the wasm version provide, with an implementation for each host (VS
//! Code, the plain browser page of the tests, and later others). The
//! desktop application has no host; it keeps its own files and settings.
//!
//! A page shows one application, so the host is set once, by the page's
//! entry point, and found here by whatever needs it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gossia_calc::http::{FetchAnswer, FetchRequest};
use gossia_calc::python::PythonSettings;
use gossia_calc::sources::{SOURCES_FILE, SOURCES_VERSION, SourceDef, SourceStore};
use gossia_calc::{EvalSettings, Evaluator};
use gossia_i18n::tr;

/// What the host does for the application. Calls that need an answer from
/// the user or the network come back later through the application's
/// polling; these are the ones that do not.
pub trait Host {
    /// A link to a new Python worker (the evaluation thread's place).
    #[cfg(target_family = "wasm")]
    fn worker(&self) -> Box<dyn gossia_calc::WorkerLink>;

    /// Saves the workbook file. `name` is the file's name, for a host that
    /// asks where to save a workbook that has none.
    fn save_workbook(&self, name: &str, bytes: Vec<u8>);

    /// Whether the workbook has changes that are not saved, each time that
    /// changes, for the host's unsaved-changes mark.
    fn set_dirty(&self, dirty: bool);

    /// Reads one entry of the host's storage: the preferences, the trust
    /// record and the Immediate window's history, each a JSON text under
    /// the name of its file on the desktop.
    fn read_storage(&self, name: &str) -> Option<String>;

    /// Writes one entry of the host's storage.
    fn write_storage(&self, name: &str, text: &str) -> Result<(), String>;

    /// Reads a secret: a data source's password and token, kept apart from
    /// the rest (VS Code's `SecretStorage`). A host whose store answers
    /// later reads its secrets before the application starts.
    fn read_secret(&self, name: &str) -> Option<String>;

    /// Writes a secret, or removes it with `None`.
    fn write_secret(&self, name: &str, text: Option<&str>);

    /// Asks the user for a file to read, with the host's own dialog. The
    /// file comes back later, through [`picked`], with `purpose`; nothing
    /// comes back if the user cancels. `extensions` (without the dot) narrow
    /// the choice; none means any file.
    fn pick_file(&self, purpose: &str, extensions: &[&str]);

    /// Asks the user where to save a file, with the host's own dialog,
    /// offering `name`, and writes `bytes` there. Whether it was written is
    /// the host's to say.
    fn save_file(&self, name: &str, bytes: Vec<u8>);

    /// Opens a file or an address with the system's program for it.
    fn open_outside(&self, target: &str);

    /// Whether the host's theme is dark, if it says.
    fn dark(&self) -> Option<bool>;

    /// The host's language, as a BCP 47 tag (VS Code's display language),
    /// if it says: the interface's language when the preferences follow
    /// the system's.
    fn language(&self) -> Option<String> {
        None
    }

    /// Starts loading the font the host ships for Japanese or Chinese (by
    /// the language's tag), which a web view needs since it cannot read the
    /// system's fonts. Whether the host has one: its bytes, or why they
    /// could not be read, come later through [`font_loaded`].
    fn load_font(&self, _tag: &str) -> bool {
        false
    }

    /// Makes an HTTP request for a data source, without following a
    /// redirect and within its timeout and size. The answer is handed to
    /// [`fetched`] with the request's id, later.
    fn fetch(&self, request: &FetchRequest);

    /// Whether modules are edited in the host's own editor (VS Code's, as
    /// files): opening one there opens it as the host's file, and the host
    /// is told the modules each time they change ([`Host::modules_changed`]).
    fn edits_modules(&self) -> bool {
        false
    }

    /// Opens a module in the host's editor, at a line and column (1-based).
    fn open_module(&self, _name: &str, _line: usize, _column: usize) {}

    /// The workbook's modules, each a name and its source, in order: at the
    /// start and each time one is added, changed, renamed or removed.
    fn modules_changed(&self, _modules: &[(String, String)]) {}

    /// The answer to a module written, renamed or removed in the host's
    /// editor ([`ModuleEdit`]): `None` when it was done, or why not.
    fn module_answer(&self, _id: u32, _error: Option<&str>) {}

    /// Whether the application's own File menu opens and saves workbooks
    /// through the host (the browser page: *New*, *Open*, *Open Recent* and
    /// *Save As* are Gossia's), rather than the host having its own (VS
    /// Code's). Such a host answers a save later, with [`saved`] or
    /// [`not_saved`], and gives a workbook to open through [`picked`] with
    /// [`purpose::OPEN`].
    fn keeps_files(&self) -> bool {
        false
    }

    /// Whether the host's pointer is a finger (a phone's or a tablet's
    /// browser), so that the window starts with the larger targets for
    /// touch (DESIGN.md, 17d). A touch turns them on anyway.
    fn coarse_pointer(&self) -> bool {
        false
    }

    /// Asks the user where to save the workbook, always, offering `name`.
    fn save_workbook_as(&self, name: &str, bytes: Vec<u8>) {
        self.save_workbook(name, bytes);
    }

    /// A workbook was opened: the one last given with [`purpose::OPEN`]
    /// (`picked`), whose file it is now, or one dropped, which has no file
    /// to save into. `bytes` are its file, for the copy kept with the recent
    /// workbooks.
    fn workbook_opened(&self, _name: &str, _bytes: &[u8], _picked: bool) {}

    /// The workbook shown is a new one, without a file.
    fn new_workbook(&self) {}

    /// The recent workbooks, the latest first: each a key and its name.
    fn recent_workbooks(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Opens a recent workbook, by its key: it comes through [`picked`] with
    /// [`purpose::OPEN`].
    fn open_recent(&self, _key: &str) {}

    /// Takes a recent workbook off the list, with its copy; `None` takes
    /// them all.
    fn forget_recent(&self, _key: Option<&str>) {}

    /// Whether secrets are kept for the session only unless the user asks
    /// to remember one (the browser page).
    fn secrets_per_session(&self) -> bool {
        false
    }

    /// Whether this secret is remembered beyond the session.
    fn secret_remembered(&self, _name: &str) -> bool {
        true
    }

    /// Remembers a secret beyond the session, or keeps it for the session
    /// only.
    fn remember_secret(&self, _name: &str, _remember: bool) {}
}

/// A change to a module made in the host's editor, to be made in the
/// workbook, answered through [`Host::module_answer`] with its `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleEdit {
    /// A module's file was saved: its new source, or a new module.
    Write {
        id: u32,
        name: String,
        text: String,
    },
    Rename {
        id: u32,
        from: String,
        to: String,
    },
    Remove {
        id: u32,
        name: String,
    },
}

/// The storage entry of the application's own clipboard in a host: what
/// Copy puts there, Paste takes, apart from the system's clipboard.
pub const CLIPBOARD: &str = "clipboard";

/// What a file read through the host is for.
pub mod purpose {
    /// File > Import: a file to import as tabs.
    pub const IMPORT: &str = "import";
    /// A file dropped on the window.
    pub const DROP: &str = "drop";
    /// The code editor's File > Import module.
    pub const IMPORT_MODULE: &str = "import-module";
    /// File > Open and File > Open Recent, in a host that keeps files: a
    /// workbook to open.
    pub const OPEN: &str = "open";
}

/// What became of a save given to a host that keeps files: the file's name
/// when it was written (the user may have chosen another), or why not
/// (`None`: the user cancelled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveAnswer {
    Saved(String),
    NotSaved(Option<String>),
}

/// A file read through the host: picked in its dialog, or dropped.
#[derive(Debug, Clone)]
pub struct Picked {
    pub purpose: String,
    /// The file's name, without its folder.
    pub name: String,
    pub bytes: Vec<u8>,
}

/// What to do with the answer to a request the host is making.
type Then = Box<dyn FnOnce(FetchAnswer)>;

thread_local! {
    static HOST: RefCell<Option<Rc<dyn Host>>> = const { RefCell::new(None) };
    static PICKED: RefCell<Vec<Picked>> = const { RefCell::new(Vec::new()) };
    static SAVE_ANSWERS: RefCell<Vec<SaveAnswer>> = const { RefCell::new(Vec::new()) };
    /// What to do with the answers to the requests the host is making, by
    /// id, and the last id given.
    static FETCHES: RefCell<(u64, HashMap<u64, Then>)> = RefCell::new((0, HashMap::new()));
    /// The evaluator's requests that the host has answered: the evaluator's
    /// id and the answer.
    static FETCHED: Rc<RefCell<Vec<(u64, FetchAnswer)>>> = Rc::new(RefCell::new(Vec::new()));
    /// The version of the data source definitions last kept in the host,
    /// and their names then.
    static SOURCES_KEPT: RefCell<(u64, Vec<String>)> = const { RefCell::new((0, Vec::new())) };
    /// The fonts asked of the host, by tag, and what came: kept, so that
    /// choosing the language again does not load the font again.
    static FONTS: RefCell<HashMap<String, HostFont>> = RefCell::new(HashMap::new());
}

/// The host's font for a language, as far as it has come.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostFont {
    /// The host ships none: the system's is looked for instead.
    None,
    /// Asked for, not come yet.
    Waiting,
    Loaded(Vec<u8>),
    /// It could not be read: why.
    Failed(String),
}

/// The font `host` ships for `language`, asking for it the first time.
pub fn font(host: &dyn Host, language: gossia_i18n::Language) -> HostFont {
    let tag = language.tag();
    if let Some(known) = FONTS.with(|f| f.borrow().get(tag).cloned()) {
        return known;
    }
    if !host.load_font(tag) {
        return HostFont::None;
    }
    FONTS.with(|f| f.borrow_mut().insert(tag.to_owned(), HostFont::Waiting));
    HostFont::Waiting
}

/// The host's answer to [`Host::load_font`]: the font's bytes, or why
/// they could not be read.
pub fn font_loaded(tag: &str, font: Result<Vec<u8>, String>) {
    let font = match font {
        Ok(bytes) => HostFont::Loaded(bytes),
        Err(why) => HostFont::Failed(why),
    };
    FONTS.with(|f| f.borrow_mut().insert(tag.to_owned(), font));
}

/// The secret of a data source is kept under this name.
pub(crate) fn secret_name(source: &str) -> String {
    format!("source:{source}")
}

/// Reads the data source definitions the host keeps into `store`: the
/// definitions from storage, as `data-sources.json` holds them on the desktop
/// without their secrets, and each one's password and token from the
/// secrets. What cannot be read is said in the warnings.
pub fn load_sources(host: &dyn Host, store: &SourceStore, warnings: &mut Vec<String>) {
    let Some(text) = host.read_storage(SOURCES_FILE) else {
        return;
    };
    let list = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|mut doc| doc.get_mut("sources").map(serde_json::Value::take))
        .map(serde_json::from_value::<Vec<SourceDef>>);
    let mut defs = match list {
        Some(Ok(defs)) => defs,
        _ => {
            warnings
                .push("The stored data sources cannot be read, so none are defined.".to_owned());
            return;
        }
    };
    for def in &mut defs {
        let secret = host
            .read_secret(&secret_name(&def.name))
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        if let Some(secret) = secret {
            def.password = secret["password"].as_str().map(str::to_owned);
            def.token = secret["token"].as_str().map(str::to_owned);
        }
    }
    let names = defs.iter().map(|d| d.name.clone()).collect();
    if let Err(e) = store.replace_all(defs) {
        warnings.push(e);
    }
    SOURCES_KEPT.with(|kept| *kept.borrow_mut() = (store.version(), names));
}

/// Keeps the definitions in the host, when they have changed since they were
/// last kept. Says whether they had.
pub fn keep_sources(host: &dyn Host, store: &SourceStore) -> Result<bool, String> {
    let version = store.version();
    let old_names = SOURCES_KEPT.with(|kept| {
        let kept = kept.borrow();
        (kept.0 != version).then(|| kept.1.clone())
    });
    let Some(old_names) = old_names else {
        return Ok(false);
    };
    let defs = store.list()?;
    let plain: Vec<SourceDef> = defs.iter().map(SourceDef::without_secrets).collect();
    let doc = serde_json::json!({ "version": SOURCES_VERSION, "sources": plain });
    host.write_storage(SOURCES_FILE, &doc.to_string())?;
    for def in &defs {
        let secret = def.has_secret().then(|| {
            serde_json::json!({ "password": def.password, "token": def.token }).to_string()
        });
        host.write_secret(&secret_name(&def.name), secret.as_deref());
    }
    for gone in old_names
        .iter()
        .filter(|n| !defs.iter().any(|d| &&d.name == n))
    {
        host.write_secret(&secret_name(gone), None);
    }
    let names = defs.iter().map(|d| d.name.clone()).collect();
    SOURCES_KEPT.with(|kept| *kept.borrow_mut() = (version, names));
    Ok(true)
}

impl crate::GossiaApp {
    /// In a host, keeps the data source definitions there when they change,
    /// and gives the worker the new ones (it has a copy).
    pub(crate) fn sync_sources(&mut self) {
        let Some(host) = current() else {
            return;
        };
        match keep_sources(&*host, &self.user.sources) {
            Ok(false) => {}
            Ok(true) => {
                let effective = self.effective_settings();
                reconfigure(
                    self.session.evaluator_mut(),
                    self.user.python_settings(&effective),
                    effective.eval_settings(),
                );
            }
            Err(e) => {
                self.notice = Some(tr!("The data sources could not be kept: {e}", e = e));
            }
        }
    }
}

/// A file has come: the host's answer to [`Host::pick_file`], or a file
/// dropped on the window once the browser has read it.
pub fn picked(file: Picked) {
    PICKED.with(|p| p.borrow_mut().push(file));
}

/// The files that have come since the last call.
pub fn take_picked() -> Vec<Picked> {
    PICKED.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// A host that keeps files wrote the workbook, under this name.
pub fn saved(name: String) {
    SAVE_ANSWERS.with(|a| a.borrow_mut().push(SaveAnswer::Saved(name)));
}

/// A host that keeps files did not write the workbook: why, or `None` when
/// the user cancelled.
pub fn not_saved(why: Option<String>) {
    SAVE_ANSWERS.with(|a| a.borrow_mut().push(SaveAnswer::NotSaved(why)));
}

/// The answers to saves that have come since the last call.
pub fn take_save_answers() -> Vec<SaveAnswer> {
    SAVE_ANSWERS.with(|a| std::mem::take(&mut *a.borrow_mut()))
}

/// Asks the host to make `request`; `then` is given the answer when it
/// comes. The request's id is replaced by one of the host's.
pub fn fetch(host: &dyn Host, mut request: FetchRequest, then: impl FnOnce(FetchAnswer) + 'static) {
    let id = FETCHES.with(|f| {
        let mut f = f.borrow_mut();
        f.0 += 1;
        let id = f.0;
        f.1.insert(id, Box::new(then));
        id
    });
    request.id = id;
    host.fetch(&request);
}

/// The host's answer to the request with this id.
pub fn fetched(id: u64, answer: FetchAnswer) {
    let then = FETCHES.with(|f| f.borrow_mut().1.remove(&id));
    if let Some(then) = then {
        then(answer);
    }
}

impl crate::GossiaApp {
    /// In a host, gives it the HTTP requests the worker asks for, and gives
    /// the worker the answers that have come.
    pub(crate) fn forward_fetches(&mut self) {
        #[cfg(target_family = "wasm")]
        {
            let Some(host) = current() else {
                return;
            };
            let evaluator = self.session.evaluator_mut();
            for request in evaluator.take_fetches() {
                let id = request.id;
                let inbox = FETCHED.with(Rc::clone);
                fetch(&*host, request, move |answer| {
                    inbox.borrow_mut().push((id, answer));
                });
            }
            for (id, answer) in FETCHED.with(|f| std::mem::take(&mut *f.borrow_mut())) {
                evaluator.answer_fetch(id, answer);
            }
        }
    }
}

/// Sets the host, once, before the application is made.
pub fn set(host: Rc<dyn Host>) {
    HOST.with(|h| *h.borrow_mut() = Some(host));
}

/// The host, if the application has one.
pub fn current() -> Option<Rc<dyn Host>> {
    HOST.with(|h| h.borrow().clone())
}

/// An evaluator with these settings: on the desktop a thread with its own
/// engine, in a host a Web Worker that the host starts.
pub fn evaluator(python: PythonSettings, settings: EvalSettings) -> Evaluator {
    #[cfg(not(target_family = "wasm"))]
    {
        Evaluator::new(gossia_calc::python::factory(python), settings)
    }
    #[cfg(target_family = "wasm")]
    {
        let host = current().expect("the page sets the host before the application is made");
        Evaluator::new(host.worker(), python, settings)
    }
}

/// Gives an evaluator new settings.
pub fn reconfigure(evaluator: &mut Evaluator, python: PythonSettings, settings: EvalSettings) {
    #[cfg(not(target_family = "wasm"))]
    evaluator.reconfigure(gossia_calc::python::factory(python), settings);
    #[cfg(target_family = "wasm")]
    evaluator.reconfigure(python, settings);
}
