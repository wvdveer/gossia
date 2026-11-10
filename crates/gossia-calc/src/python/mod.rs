//! The Python engine: formulas run by RustPython (DESIGN.md, "Python engine").
//!
//! The interpreter is built without access to the operating system, so what is
//! compiled in, not what is hidden, is what keeps a formula away from files and
//! the network. Removing builtins and rejecting double-underscore access catch
//! mistakes; they are not described as security.

pub mod catalog;
pub mod completion;
mod convert;
mod jsonpath;
mod modules;
pub(crate) use modules::no_module;
mod node;
mod shared;
mod sources;
pub(crate) mod syntax;
mod watchdog;
mod xmlpath;

use gossia_i18n::tr;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gossia_core::{ErrorKind, Path, Value};
use rustpython::vm::builtins::{PyCode, PyDictRef};
use rustpython::vm::compiler::Mode;
use rustpython::vm::function::ArgMapping;
use rustpython::vm::scope::Scope;
use rustpython::vm::{
    AsObject, Interpreter, PyObjectRef, PyPayload, PyRef, PyResult, Settings, VirtualMachine,
};

use crate::engine::{Answer, Call, Context, Depth, Engine, Failure, Read, Write};
use shared::{Classes, Shared};
use watchdog::Watchdog;

pub use syntax::PythonSyntax;

/// A factory for the evaluation thread: each call makes an engine with its
/// own interpreter.
pub fn factory(settings: PythonSettings) -> crate::EngineFactory {
    std::sync::Arc::new(move || {
        Box::new(PythonEngine::with_settings(settings.clone())) as Box<dyn Engine>
    })
}

/// The Python source that defines `ErrorValue`, `IFERROR` and the rest.
const PRELUDE: &str = include_str!("prelude.py");

/// The built-in chart types (DESIGN.md, Reports, "Chart functions"), which
/// the code editor shows read-only.
pub const CHARTS: &str = include_str!("charts.py");

/// The names of the built-in chart types and the functions that draw them.
pub const CHART_FUNCTIONS: &[(&str, &str)] = &[
    ("Bar", "bar_chart"),
    ("Line", "line_chart"),
    ("Pie", "pie_chart"),
    ("Scatter", "scatter_chart"),
    ("Sankey", "sankey_chart"),
];

/// Standard library modules formulas can use by name, without importing them
/// (DESIGN.md, "Python engine", "Modules that work").
pub(crate) const MODULES: &[&str] = &[
    "math",
    "datetime",
    "re",
    "json",
    "itertools",
    "decimal",
    "fractions",
    "collections",
    "functools",
    "string",
    "textwrap",
    "statistics",
];

/// Modules the interpreter's own Rust code imports by name when a formula
/// calls it: `date.strftime` imports `time`, the `n` format of a number
/// `locale`, some codecs `_pycodecs`. The replacement `__import__` serves
/// them as Python's own does, since it is the one in force while a formula
/// runs. build.rs freezes those written in Python. (`datetime.strptime`
/// imports `_strptime`, which needs `os`, so it is not available.)
pub(crate) const INTERNAL_MODULES: &[&str] = &[
    "time",
    "locale",
    "_pycodecs",
    "typing",
    "warnings",
    "weakref",
    "copyreg",
    "codecs",
    "string.templatelib",
];

/// The standard library modules frozen into the engine, chosen by build.rs.
fn frozen_lib() -> &'static rustpython_vm::frozen::FrozenLib {
    let lib = include!(concat!(env!("OUT_DIR"), "/frozen_lib.rs"));
    rustpython_vm::frozen::FrozenLib::from_ref(&lib.bytes[..])
}

/// The names of the modules [`frozen_lib`] holds.
#[cfg(test)]
pub(crate) const FROZEN: &[&str] = include!(concat!(env!("OUT_DIR"), "/frozen_names.rs"));

/// Builtins a formula does not see. This catches mistakes; it is not a
/// boundary (DESIGN.md, "What the sandbox really is").
const REMOVED_BUILTINS: &[&str] = &[
    "open",
    "exec",
    "eval",
    "compile",
    "__import__",
    "__build_class__",
    "input",
    "breakpoint",
    "help",
    "exit",
    "quit",
    "copyright",
    "credits",
    "license",
    "globals",
    "locals",
    "vars",
    "setattr",
    "delattr",
];

/// Settings of the engine that the user can change (DESIGN.md, "Settings and
/// limits").
#[derive(Debug, Clone)]
pub struct PythonSettings {
    /// The longest sequence `range` will make.
    pub max_range: i64,
    /// The data source definitions of this user on this machine.
    pub sources: crate::sources::SourceStore,
    /// How long a call to a data source may wait.
    pub source_timeout: Duration,
    /// The largest response a data source may give, in bytes.
    pub max_response: u64,
}

impl Default for PythonSettings {
    fn default() -> Self {
        PythonSettings {
            max_range: 1_000_000,
            sources: crate::sources::SourceStore::in_memory(),
            source_timeout: Duration::from_secs(30),
            max_response: 16 << 20,
        }
    }
}

/// Runs formulas with RustPython. It owns an interpreter, which cannot move
/// between threads, so make it on the thread that will use it.
pub struct PythonEngine {
    shared: Rc<Shared>,
    /// The names a formula can see.
    globals: PyDictRef,
    /// The namespace of the prelude, where its settings live.
    prelude: PyDictRef,
    code: RefCell<HashMap<String, PyRef<PyCode>>>,
    sheets: RefCell<Vec<String>>,
    /// The workbook modules that are loaded, or have failed to.
    modules: RefCell<HashMap<String, modules::Loaded>>,
    /// Which module defines each name a module provides: its own name and its
    /// public functions, those of a failed module too.
    provided: RefCell<HashMap<String, String>>,
    /// The names of the tabs, as of the last time the modules were loaded.
    tabs: RefCell<Vec<String>>,
    evaluations: Cell<usize>,
    watchdog: Rc<Watchdog>,
    time_limit: Cell<Option<Duration>>,
    // Last, so that it is dropped after everything that lives in it.
    interp: Interpreter,
}

impl PythonEngine {
    /// HTTP requests go to the host from now on (the wasm version's worker):
    /// a call without an answer asks for one and stops what runs.
    pub fn ask_host_for_requests(&self) {
        let mut fetches = self.shared.fetches.borrow_mut();
        if fetches.is_none() {
            *fetches = Some(crate::fetches::Fetches::default());
        }
    }

    /// Works with the host's answers, if requests go to the host.
    pub fn with_fetches<T>(&self, f: impl FnOnce(&mut crate::fetches::Fetches) -> T) -> Option<T> {
        self.shared.fetches.borrow_mut().as_mut().map(f)
    }

    pub fn new() -> PythonEngine {
        PythonEngine::with_settings(PythonSettings::default())
    }

    pub fn with_settings(settings: PythonSettings) -> PythonEngine {
        let mut vm_settings = Settings::default();
        vm_settings.isolated = true;
        vm_settings.import_site = false;
        vm_settings.install_signal_handlers = false;
        vm_settings.write_bytecode = false;
        vm_settings.safe_path = true;
        let (signals, inbox) = rustpython::vm::signal::user_signal_channel();
        let builder = rustpython::Interpreter::builder(vm_settings);
        // What init_stdlib does, with only the Python modules formulas can
        // reach frozen in (build.rs) instead of all of `Lib`.
        let native = rustpython_stdlib::formula_module_defs(&builder.ctx);
        let builder = builder
            .add_native_modules(&native)
            .add_frozen_modules(frozen_lib())
            .init_hook(move |vm| vm.set_user_signal_channel(inbox));
        let module = node::_gossia::module_def(&builder.ctx);
        let interp = builder.add_native_module(module).build();
        let shared = Shared::new();
        *shared.sources.borrow_mut() = settings.sources.clone();
        shared.source_timeout.set(settings.source_timeout);
        shared.max_response.set(settings.max_response);
        let watchdog = Rc::new(Watchdog::new(signals));
        *shared.watchdog.borrow_mut() = Some(Rc::clone(&watchdog));
        let (globals, prelude) =
            interp.enter(|vm| set_up(vm, &shared, &settings).expect("the Python prelude runs"));
        PythonEngine {
            shared,
            globals,
            prelude,
            code: RefCell::new(HashMap::new()),
            sheets: RefCell::new(Vec::new()),
            modules: RefCell::new(HashMap::new()),
            provided: RefCell::new(HashMap::new()),
            tabs: RefCell::new(Vec::new()),
            evaluations: Cell::new(0),
            watchdog,
            time_limit: Cell::new(None),
            interp,
        }
    }

    /// How many formulas have been run.
    pub fn evaluations(&self) -> usize {
        self.evaluations.get()
    }

    /// Runs Python source directly, past the syntax check, as a formula, and
    /// gives the text of each result or of the exception it raised. For the
    /// tests of what is reachable from inside.
    #[cfg(test)]
    fn probe(&self, sources: &[&str]) -> Vec<String> {
        self.interp.enter(|vm| {
            sources
                .iter()
                .map(|source| {
                    let scope = Scope::new(None, self.globals.clone());
                    let outcome = vm
                        .compile(source, Mode::Eval, "<probe>".to_owned())
                        .map_err(|e| e.into_pyexception(vm, Some(source)))
                        .and_then(|code| vm.run_code_obj(code, scope));
                    match outcome {
                        Ok(value) => value
                            .repr(vm)
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| "<unprintable>".to_owned()),
                        Err(exc) => format!(
                            "{}: {}",
                            exc.class().name(),
                            exc.as_object()
                                .str(vm)
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        ),
                    }
                })
                .collect()
        })
    }

    /// Whether `import name` works for a formula's interpreter.
    #[cfg(test)]
    fn can_import(&self, name: &'static str) -> bool {
        self.interp.enter(|vm| vm.import(name, 0).is_ok())
    }

    /// How long one formula may run before it is stopped with `#TIMEOUT!`.
    /// `None` is no limit. This stops a formula that is running Python code; a
    /// single long call into the interpreter's own code is not interrupted.
    pub fn set_time_limit(&self, limit: Option<Duration>) {
        self.time_limit.set(limit);
    }

    /// Changes the longest sequence `range` will make.
    pub fn set_max_range(&self, max_range: i64) {
        self.interp.enter(|vm| {
            self.prelude
                .set_item("_max_range", vm.ctx.new_int(max_range).into(), vm)
                .expect("a dict takes a new item");
        });
    }

    /// Makes the tabs available by name.
    fn sync_sheets(&self, vm: &VirtualMachine, ctx: &dyn Context) {
        let mut names = ctx.sheets();
        names.sort();
        if *self.sheets.borrow() == names {
            return;
        }
        for old in self.sheets.borrow().iter() {
            let _ = self.globals.del_item(old.as_str(), vm);
        }
        for name in &names {
            // A tab never takes the place of a name the engine holds or a
            // module provides: nothing is shadowed.
            if crate::names::is_reserved(name) || self.provided.borrow().contains_key(name) {
                continue;
            }
            let sheet: Rc<str> = Rc::from(name.as_str());
            let proxy = node::lazy_proxy(&self.shared, vm, &sheet, Path::root());
            let _ = self.globals.set_item(name.as_str(), proxy, vm);
        }
        *self.sheets.borrow_mut() = names;
    }

    /// The compiled formula. Formulas are kept compiled; an expression of the
    /// Immediate window (`keep` false) is not, since it is typed once.
    fn compiled(
        &self,
        vm: &VirtualMachine,
        text: &str,
        keep: bool,
    ) -> Result<PyRef<PyCode>, Failure> {
        if let Some(code) = self.code.borrow().get(text) {
            return Ok(code.clone());
        }
        match vm.compile(text, Mode::Eval, "<formula>".to_owned()) {
            Ok(code) => {
                if keep {
                    self.code.borrow_mut().insert(text.to_owned(), code.clone());
                }
                Ok(code)
            }
            Err(error) => Err(Failure::new(ErrorKind::Syntax, error.to_string())),
        }
    }

    fn run(
        &self,
        vm: &VirtualMachine,
        text: &str,
        ctx: &Rc<dyn Context>,
    ) -> Result<Answer, Failure> {
        let result = self.run_object(vm, text, ctx, true)?;
        if let Some(bytes) = result.downcast_ref::<rustpython::vm::builtins::PyBytes>() {
            return Ok(Answer::Bytes(bytes.as_bytes().to_vec()));
        }
        match convert::to_value(&self.shared, vm, &result) {
            Ok(value) => Ok(Answer::Value(value)),
            Err(convert::Bad::Value(kind, message)) => Err(Failure::new(kind, message)),
            Err(convert::Bad::Raised(exc)) => Err(convert::failure(&self.shared, vm, &exc)),
        }
    }

    /// Runs the text as a formula on the node of `ctx`, and gives the Python
    /// object it came to.
    fn run_object(
        &self,
        vm: &VirtualMachine,
        text: &str,
        ctx: &Rc<dyn Context>,
        keep: bool,
    ) -> Result<PyObjectRef, Failure> {
        let code = self.compiled(vm, text, keep)?;
        self.record_module_use(text, ctx);
        let locals = vm.ctx.new_dict();
        let sheet: Rc<str> = Rc::from(ctx.sheet());
        let here = ctx.path();
        let this = match ctx.this_by_value() {
            true => self.this_value(vm, &sheet, here, ctx),
            false => node::lazy_proxy(&self.shared, vm, &sheet, here.clone()),
        };
        let parent = match here.parent() {
            Some(path) => node::lazy_proxy(&self.shared, vm, &sheet, path),
            None => vm.ctx.none(),
        };
        let root = node::lazy_proxy(&self.shared, vm, &sheet, Path::root());
        let _ = locals.set_item("this", this, vm);
        let _ = locals.set_item("parent", parent, vm);
        let _ = locals.set_item("root", root, vm);
        // The items of the repeaters around a control, by the names they are
        // given (and `this`, for an item with no node).
        for (name, value) in ctx.bindings() {
            let value = convert::value_to_py(&self.shared, vm, value)
                .map_err(|exc| convert::failure(&self.shared, vm, &exc))?;
            let _ = locals.set_item(name.as_str(), value, vm);
        }
        let scope = Scope::new(
            Some(ArgMapping::from_dict_exact(locals)),
            self.globals.clone(),
        );
        match vm.run_code_obj(code, scope) {
            Ok(result) => Ok(result),
            Err(exc) => {
                // A name that is not defined may be a tab that does not exist
                // yet. Looking for it is a dependency, so that adding the tab
                // runs the formula again.
                if exc.fast_isinstance(vm.ctx.exceptions.name_error)
                    && let Some(name) = missing_name(vm, &exc)
                {
                    ctx.record(&name, &Path::root(), Depth::Shallow);
                    if let Some(failure) = self.failed_module_name(&name) {
                        return Err(failure);
                    }
                }
                Err(convert::failure(&self.shared, vm, &exc))
            }
        }
    }
}

impl PythonEngine {
    /// `this` for an expression of a control: the value of a node that holds
    /// one, the node itself for an object or an array, and `None` for an
    /// optional field that is not there.
    fn this_value(
        &self,
        vm: &VirtualMachine,
        sheet: &Rc<str>,
        here: &Path,
        ctx: &Rc<dyn Context>,
    ) -> PyObjectRef {
        match ctx.read(sheet, here, Depth::Shallow) {
            Read::Node(node) => convert::node_to_py(&self.shared, vm, sheet, here.clone(), &node)
                .unwrap_or_else(|_| node::lazy_proxy(&self.shared, vm, sheet, here.clone())),
            _ if ctx.absent_ok(sheet, here) => vm.ctx.none(),
            _ => node::lazy_proxy(&self.shared, vm, sheet, here.clone()),
        }
    }

    /// The function a button names: one a workbook module defines, as `name`
    /// or `module.name`. Nothing else is run: not a built-in, not a tab.
    fn find_procedure(&self, vm: &VirtualMachine, name: &str) -> Result<PyObjectRef, Failure> {
        let missing = || {
            Failure::new(
                ErrorKind::Name,
                tr!(
                    "there is no procedure called {name}: it must be a function of a workbook module",
                    name = name
                ),
            )
        };
        let (module, function) = match name.split_once('.') {
            Some((module, function)) => (Some(module), function),
            None => (None, name),
        };
        if function.starts_with('_') {
            return Err(missing());
        }
        if let Some(failure) = self.failed_module_name(module.unwrap_or(function)) {
            return Err(failure);
        }
        let found = match module {
            Some(module) => {
                if !self.modules.borrow().contains_key(module) {
                    return Err(missing());
                }
                self.globals
                    .get_item(module, vm)
                    .and_then(|m| m.get_attr(&*vm.ctx.new_str(function), vm))
            }
            None => {
                let owner = self.provided.borrow().get(function).cloned();
                if owner.is_none_or(|owner| owner == function) {
                    return Err(missing());
                }
                self.globals.get_item(function, vm)
            }
        };
        match found {
            Ok(f) if f.is_callable() => Ok(f),
            _ => Err(missing()),
        }
    }

    fn run_procedure(
        &self,
        vm: &VirtualMachine,
        call: &Call,
        ctx: &Rc<dyn Context>,
    ) -> Result<(), Failure> {
        let function = self.find_procedure(vm, &call.procedure)?;
        let sheet: Rc<str> = Rc::from(call.sheet.as_str());
        let form = node::lazy_proxy(&self.shared, vm, &sheet, Path::root());
        let tabs = vm.ctx.new_dict();
        for name in ctx.sheets() {
            let tab: Rc<str> = Rc::from(name.as_str());
            let proxy = node::lazy_proxy(&self.shared, vm, &tab, Path::root());
            let _ = tabs.set_item(name.as_str(), proxy, vm);
        }
        let context = self
            .prelude
            .get_item("ProcedureContext", vm)
            .and_then(|class| {
                class.call(
                    (
                        vm.ctx.new_str(call.form.as_str()),
                        vm.ctx.new_str(call.button.as_str()),
                        tabs,
                        match call.rows.last() {
                            Some(&i) => vm.ctx.new_int(i).into(),
                            None => vm.ctx.none(),
                        },
                    ),
                    vm,
                )
            })
            .map_err(|exc| convert::failure(&self.shared, vm, &exc))?;
        let called = match &call.value {
            Some(text) => function.call((form, context, vm.ctx.new_str(text.as_str())), vm),
            None => function.call((form, context), vm),
        };
        called
            .map(|_| ())
            .map_err(|exc| convert::failure(&self.shared, vm, &exc))
    }
}

/// The name a `NameError` is about.
fn missing_name(
    vm: &VirtualMachine,
    exc: &rustpython::vm::builtins::PyBaseExceptionRef,
) -> Option<String> {
    let message = exc.as_object().str(vm).ok()?.to_string_lossy().into_owned();
    let start = message.find('\'')? + 1;
    let end = start + message[start..].find('\'')?;
    Some(message[start..end].to_owned())
}

impl Default for PythonEngine {
    fn default() -> Self {
        PythonEngine::new()
    }
}

impl Engine for PythonEngine {
    fn set_time_limit(&self, limit: Option<Duration>) {
        PythonEngine::set_time_limit(self, limit);
    }

    fn sync_modules(
        &self,
        specs: &[crate::ModuleSpec],
        tabs: &[String],
    ) -> Vec<crate::ModuleReport> {
        self.load_modules(specs, tabs)
    }

    fn begin_pass(&self) {
        self.shared.source_reads.borrow_mut().clear();
    }

    fn waiting(&self) -> bool {
        self.shared.waiting.get()
    }

    fn call(&self, call: &Call, ctx: Rc<dyn Context>) -> Result<Vec<Write>, Failure> {
        self.interp.enter(|vm| {
            self.sync_sheets(vm, &*ctx);
            self.shared.begin(ctx.clone());
            *self.shared.writes.borrow_mut() = Some(Vec::new());
            self.shared.side_effects.set(true);
            if let Some(limit) = call.time_limit {
                self.watchdog.start(limit);
            }
            let result = self.run_procedure(vm, call, &ctx);
            if call.time_limit.is_some() {
                self.watchdog.stop();
            }
            let writes = self.shared.writes.borrow_mut().take();
            self.shared.side_effects.set(false);
            self.shared.end();
            result.map(|()| writes.unwrap_or_default())
        })
    }

    fn immediate(
        &self,
        text: &str,
        ctx: Rc<dyn Context>,
        limit: Option<Duration>,
    ) -> Result<String, Failure> {
        let text = text.trim();
        self.interp.enter(|vm| {
            self.sync_sheets(vm, &*ctx);
            self.shared.begin(ctx.clone());
            self.shared.side_effects.set(true);
            if let Some(limit) = limit {
                self.watchdog.start(limit);
            }
            let result = self.run_object(vm, text, &ctx, false).and_then(|object| {
                // A node is shown as the plain lists, dicts and values it
                // holds, in Python's own form.
                let plain = self
                    .prelude
                    .get_item("PLAIN", vm)
                    .and_then(|plain| plain.call((object.clone(),), vm))
                    .unwrap_or(object);
                plain
                    .repr(vm)
                    .map(|text| text.to_string_lossy().into_owned())
                    .map_err(|exc| convert::failure(&self.shared, vm, &exc))
            });
            if limit.is_some() {
                self.watchdog.stop();
            }
            self.shared.side_effects.set(false);
            self.shared.end();
            result
        })
    }

    fn eval(&self, formula: &str, ctx: Rc<dyn Context>) -> Result<Value, Failure> {
        match self.eval_answer(formula, ctx)? {
            Answer::Value(value) => Ok(value),
            // Only a node's schema can read bytes (DESIGN.md, "Raw memory").
            Answer::Bytes(_) => Err(Failure::new(
                ErrorKind::Type,
                tr!(
                    "this expression gave bytes, which only a formula on a node can give: the node's schema reads them"
                ),
            )),
        }
    }

    fn eval_answer(&self, formula: &str, ctx: Rc<dyn Context>) -> Result<Answer, Failure> {
        self.evaluations.set(self.evaluations.get() + 1);
        let text = formula.trim();
        self.interp.enter(|vm| {
            self.sync_sheets(vm, &*ctx);
            self.shared.begin(ctx.clone());
            if let Some(limit) = self.time_limit.get() {
                self.watchdog.start(limit);
            }
            let result = self.run(vm, text, &ctx);
            if self.time_limit.get().is_some() {
                self.watchdog.stop();
            }
            self.shared.end();
            result
        })
    }
}

/// Builds the prelude and the names formulas see.
fn set_up(
    vm: &VirtualMachine,
    shared: &Rc<Shared>,
    settings: &PythonSettings,
) -> PyResult<(PyDictRef, PyDictRef)> {
    // The prelude runs in a namespace of its own.
    let prelude = vm.ctx.new_dict();
    prelude.set_item("_max_range", vm.ctx.new_int(settings.max_range).into(), vm)?;
    let scope = Scope::with_builtins(None, prelude.clone(), vm);
    vm.run_string(scope, PRELUDE, "<gossia prelude>")?;
    // The Excel-named functions run in the same namespace.
    let scope = Scope::with_builtins(None, prelude.clone(), vm);
    vm.run_string(scope, gossia_std::PYTHON, "<gossia std>")?;
    // And the built-in chart types, which use the prelude's `chart`.
    let scope = Scope::with_builtins(None, prelude.clone(), vm);
    vm.run_string(scope, CHARTS, "<gossia charts>")?;
    // After the prelude, which imports `_gossia` and so makes its types.
    // The prelude only reads `_source_access` when a source is used.
    let access = node::_gossia::SourceAccess {
        shared: shared.clone(),
    };
    prelude.set_item("_source_access", access.into_ref(&vm.ctx).into(), vm)?;
    let raw = node::_gossia::RawAccess {
        shared: shared.clone(),
    };
    prelude.set_item("_raw_access", raw.into_ref(&vm.ctx).into(), vm)?;

    let datetime = vm.import("datetime", 0)?;
    shared.set_classes(Classes {
        error_value: prelude.get_item("ErrorValue", vm)?,
        propagated: prelude.get_item("PropagatedError", vm)?,
        timeout: prelude.get_item("FormulaTimeout", vm)?,
        not_available: prelude.get_item(gossia_std::NOT_AVAILABLE, vm)?,
        date: datetime.get_attr("date", vm)?,
        datetime: datetime.get_attr("datetime", vm)?,
        timezone: datetime.get_attr("timezone", vm)?,
        timedelta: datetime.get_attr("timedelta", vm)?,
        source_error: prelude.get_item("SourceError", vm)?,
        record: prelude.get_item("Record", vm)?,
    });

    // The builtins a formula sees: all of them, less a few.
    let builtins = vm.ctx.new_dict();
    for (name, value) in vm.builtins.dict().items_vec() {
        let keep = match name.downcast_ref::<rustpython::vm::builtins::PyStr>() {
            Some(text) => !REMOVED_BUILTINS.contains(&text.expect_str()),
            None => false,
        };
        if keep {
            builtins.set_item(&*name, value, vm)?;
        }
    }

    // One flat namespace: it is the globals of every formula, and the builtins
    // of every module, so a module finds a tab or another module's function by
    // name when it is called, even if that was added after it was loaded.
    let globals = builtins;
    globals.set_item("__builtins__", globals.clone().into(), vm)?;
    // The replacement `__import__`, which serves only the workbook's modules
    // and the exposed standard library modules.
    let importer = node::_gossia::Importer {
        shared: shared.clone(),
    };
    globals.set_item("__import__", importer.into_ref(&vm.ctx).into(), vm)?;
    for name in [
        "ErrorValue",
        "PropagatedError",
        "ISERROR",
        "IFERROR",
        "VALID",
        "PLAIN",
        "JSONPATH",
        "XMLPATH",
        "raw",
        "range",
        "source",
        "sources",
        "SourceError",
        "_date",
        "_datetime",
        "_decimal",
        "_record",
        "_as_literal",
        "chart",
        "_chart_svg",
    ]
    .iter()
    .chain(gossia_std::NAMES)
    .chain(CHART_FUNCTIONS.iter().map(|(_, f)| f))
    {
        globals.set_item(*name, prelude.get_item(*name, vm)?, vm)?;
    }
    for name in MODULES {
        let module = vm.import(*name, 0)?;
        shared
            .stdlib
            .borrow_mut()
            .insert((*name).to_owned(), module.clone());
        globals.set_item(*name, module, vm)?;
    }
    Ok((globals, prelude))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_more.rs"]
mod tests_more;

#[cfg(test)]
#[path = "tests_budget.rs"]
mod tests_budget;

#[cfg(test)]
#[path = "tests_std.rs"]
mod tests_std;

#[cfg(test)]
#[path = "tests_modules.rs"]
mod tests_modules;

#[cfg(test)]
#[path = "tests_trust.rs"]
mod tests_trust;

#[cfg(test)]
#[path = "tests_frozen.rs"]
mod tests_frozen;

#[cfg(test)]
#[path = "tests_xml.rs"]
mod tests_xml;

#[cfg(test)]
#[path = "tests_sources.rs"]
mod tests_sources;

#[cfg(test)]
#[path = "tests_disconnect.rs"]
mod tests_disconnect;
