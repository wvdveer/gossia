//! The Python engine: formulas run by RustPython (DESIGN.md, "Python engine").
//!
//! The interpreter is built without access to the operating system, so what is
//! compiled in, not what is hidden, is what keeps a formula away from files and
//! the network. Removing builtins and rejecting double-underscore access catch
//! mistakes; they are not described as security.

mod convert;
mod jsonpath;
mod node;
mod shared;
mod syntax;
mod watchdog;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use cliotree_core::{ErrorKind, Path, Value};
use rustpython::InterpreterBuilderExt;
use rustpython::vm::builtins::{PyCode, PyDictRef};
use rustpython::vm::compiler::Mode;
use rustpython::vm::function::ArgMapping;
use rustpython::vm::scope::Scope;
use rustpython::vm::{AsObject, Interpreter, PyRef, PyResult, Settings, VirtualMachine};

use crate::engine::{Context, Depth, Engine, Failure};
use shared::{Classes, Shared};
use watchdog::Watchdog;

pub use syntax::PythonSyntax;

/// The Python source that defines `ErrorValue`, `IFERROR` and the rest.
const PRELUDE: &str = include_str!("prelude.py");

/// Standard library modules formulas can use by name, without importing them
/// (DESIGN.md, "Python engine", "Modules that work").
const MODULES: &[&str] = &[
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
];

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
}

impl Default for PythonSettings {
    fn default() -> Self {
        PythonSettings {
            max_range: 10_000_000,
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
    evaluations: Cell<usize>,
    watchdog: Watchdog,
    time_limit: Cell<Option<Duration>>,
    // Last, so that it is dropped after everything that lives in it.
    interp: Interpreter,
}

impl PythonEngine {
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
        let builder = rustpython::Interpreter::builder(vm_settings)
            .init_stdlib()
            .init_hook(move |vm| vm.set_user_signal_channel(inbox));
        let module = node::_cliotree::module_def(&builder.ctx);
        let interp = builder.add_native_module(module).build();
        let shared = Shared::new();
        let (globals, prelude) =
            interp.enter(|vm| set_up(vm, &shared, &settings).expect("the Python prelude runs"));
        PythonEngine {
            shared,
            globals,
            prelude,
            code: RefCell::new(HashMap::new()),
            sheets: RefCell::new(Vec::new()),
            evaluations: Cell::new(0),
            watchdog: Watchdog::new(signals),
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
            let sheet: Rc<str> = Rc::from(name.as_str());
            let proxy = node::lazy_proxy(&self.shared, vm, &sheet, Path::root());
            let _ = self.globals.set_item(name.as_str(), proxy, vm);
        }
        *self.sheets.borrow_mut() = names;
    }

    fn compiled(&self, vm: &VirtualMachine, text: &str) -> Result<PyRef<PyCode>, Failure> {
        if let Some(code) = self.code.borrow().get(text) {
            return Ok(code.clone());
        }
        match vm.compile(text, Mode::Eval, "<formula>".to_owned()) {
            Ok(code) => {
                self.code.borrow_mut().insert(text.to_owned(), code.clone());
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
    ) -> Result<Value, Failure> {
        let code = self.compiled(vm, text)?;
        let locals = vm.ctx.new_dict();
        let sheet: Rc<str> = Rc::from(ctx.sheet());
        let here = ctx.path();
        let this = node::lazy_proxy(&self.shared, vm, &sheet, here.clone());
        let parent = match here.parent() {
            Some(path) => node::lazy_proxy(&self.shared, vm, &sheet, path),
            None => vm.ctx.none(),
        };
        let root = node::lazy_proxy(&self.shared, vm, &sheet, Path::root());
        let _ = locals.set_item("this", this, vm);
        let _ = locals.set_item("parent", parent, vm);
        let _ = locals.set_item("root", root, vm);
        let scope = Scope::new(
            Some(ArgMapping::from_dict_exact(locals)),
            self.globals.clone(),
        );
        match vm.run_code_obj(code, scope) {
            Ok(result) => match convert::to_value(&self.shared, vm, &result) {
                Ok(value) => Ok(value),
                Err(convert::Bad::Value(kind, message)) => Err(Failure::new(kind, message)),
                Err(convert::Bad::Raised(exc)) => Err(convert::failure(&self.shared, vm, &exc)),
            },
            Err(exc) => {
                // A name that is not defined may be a tab that does not exist
                // yet. Looking for it is a dependency, so that adding the tab
                // runs the formula again.
                if exc.fast_isinstance(vm.ctx.exceptions.name_error)
                    && let Some(name) = missing_name(vm, &exc)
                {
                    ctx.record(&name, &Path::root(), Depth::Shallow);
                }
                Err(convert::failure(&self.shared, vm, &exc))
            }
        }
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
    fn eval(&self, formula: &str, ctx: Rc<dyn Context>) -> Result<Value, Failure> {
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
    vm.run_string(scope, PRELUDE, "<cliotree prelude>")?;

    let datetime = vm.import("datetime", 0)?;
    shared.set_classes(Classes {
        error_value: prelude.get_item("ErrorValue", vm)?,
        propagated: prelude.get_item("PropagatedError", vm)?,
        timeout: prelude.get_item("FormulaTimeout", vm)?,
        date: datetime.get_attr("date", vm)?,
        datetime: datetime.get_attr("datetime", vm)?,
        timezone: datetime.get_attr("timezone", vm)?,
        timedelta: datetime.get_attr("timedelta", vm)?,
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

    let globals = vm.ctx.new_dict();
    globals.set_item("__builtins__", builtins.into(), vm)?;
    for name in [
        "ErrorValue",
        "PropagatedError",
        "ISERROR",
        "IFERROR",
        "VALID",
        "PLAIN",
        "JSONPATH",
        "range",
    ] {
        globals.set_item(name, prelude.get_item(name, vm)?, vm)?;
    }
    for name in MODULES {
        globals.set_item(*name, vm.import(*name, 0)?, vm)?;
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
