//! Loading the workbook's Python modules into the engine (DESIGN.md, "Python
//! modules", "How the namespaces work").
//!
//! Every module is a real module object with globals of its own, so its
//! top-level names are private to it. Anything it does not define is found in
//! its builtins, which is the live workbook namespace: the tabs, the other
//! modules, the built-in functions. After a module loads, the module object is
//! put in that namespace under its name, and so is each public function it
//! defines. A function that would take the place of anything else makes the
//! module fail, with a message that names both. Loading happens between
//! evaluations (`Engine::sync_modules`), never during one.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rustpython::vm::builtins::PyStr;
use rustpython::vm::compiler::Mode;
use rustpython::vm::function::FuncArgs;
use rustpython::vm::scope::Scope;
use rustpython::vm::{AsObject, PyObjectRef, PyResult, VirtualMachine};

use super::PythonEngine;
use super::shared::{ModuleSlot, Shared};
use crate::engine::{Context, Depth, Failure};
use crate::names::{identifiers, is_reserved};
use crate::{ModuleReport, ModuleSpec};
use cliotree_core::{ErrorKind, Path};

/// A module the engine has loaded, or has tried to.
pub(super) struct Loaded {
    pub generation: u64,
    /// The names put in the namespace for it: its own, and its functions.
    pub published: Vec<String>,
    pub result: Result<(), String>,
}

impl PythonEngine {
    /// Makes the modules of the engine the ones given. Each is reported, so the
    /// workbook always knows what became of the generation it has.
    pub(super) fn load_modules(&self, specs: &[ModuleSpec], tabs: &[String]) -> Vec<ModuleReport> {
        self.tabs.replace(tabs.to_vec());
        self.interp.enter(|vm| self.load_modules_in(vm, specs))
    }

    fn load_modules_in(&self, vm: &VirtualMachine, specs: &[ModuleSpec]) -> Vec<ModuleReport> {
        let names: HashSet<&str> = specs.iter().map(|s| s.name.as_str()).collect();

        // Modules that are gone.
        let gone: Vec<String> = self
            .modules
            .borrow()
            .keys()
            .filter(|k| !names.contains(k.as_str()))
            .cloned()
            .collect();
        for name in gone {
            self.unpublish(vm, &name);
            self.modules.borrow_mut().remove(&name);
            self.shared.modules.borrow_mut().remove(&name);
        }

        // Who provides what, for the modules that load and those that do not.
        {
            let mut provided = self.provided.borrow_mut();
            provided.clear();
            for spec in specs {
                provided.insert(spec.name.clone(), spec.name.clone());
                for function in public(spec) {
                    provided
                        .entry(function.to_owned())
                        .or_insert_with(|| spec.name.clone());
                }
            }
        }

        let mut reloaded: HashSet<String> = HashSet::new();
        let mut reports = Vec::with_capacity(specs.len());
        for index in import_order(specs) {
            let spec = &specs[index];
            let up_to_date = self
                .modules
                .borrow()
                .get(&spec.name)
                .is_some_and(|m| m.generation == spec.generation)
                && !spec
                    .info
                    .imports
                    .iter()
                    .any(|import| reloaded.contains(import));
            if !up_to_date {
                let result = self.load_one(vm, spec);
                reloaded.insert(spec.name.clone());
                let published = self
                    .modules
                    .borrow()
                    .get(&spec.name)
                    .map(|m| m.published.clone())
                    .unwrap_or_default();
                self.modules.borrow_mut().insert(
                    spec.name.clone(),
                    Loaded {
                        generation: spec.generation,
                        published,
                        result,
                    },
                );
            }
            let result = self.modules.borrow()[&spec.name].result.clone();
            reports.push(ModuleReport {
                name: spec.name.clone(),
                generation: spec.generation,
                result,
            });
        }
        reports
    }

    /// Takes a module's names out of the namespace.
    fn unpublish(&self, vm: &VirtualMachine, name: &str) {
        let published = self
            .modules
            .borrow()
            .get(name)
            .map(|m| m.published.clone())
            .unwrap_or_default();
        for published in published {
            let _ = self.globals.del_item(published.as_str(), vm);
        }
        if let Some(loaded) = self.modules.borrow_mut().get_mut(name) {
            loaded.published.clear();
        }
        self.shared.modules.borrow_mut().remove(name);
    }

    /// Runs the source of one module and publishes it.
    fn load_one(&self, vm: &VirtualMachine, spec: &ModuleSpec) -> Result<(), String> {
        self.unpublish(vm, &spec.name);
        let fail = |why: String| {
            self.shared.modules.borrow_mut().insert(
                spec.name.clone(),
                ModuleSlot {
                    object: None,
                    failed: Some(why.clone()),
                },
            );
            Err(why)
        };
        if let Some(why) = &spec.refused {
            return fail(why.clone());
        }
        let code = match vm.compile(&spec.source, Mode::Exec, format!("<module {}>", spec.name)) {
            Ok(code) => code,
            Err(e) => return fail(format!("syntax error: {e}")),
        };
        let globals = vm.ctx.new_dict();
        let set = |key: &str, value: PyObjectRef| globals.set_item(key, value, vm);
        if set("__name__", vm.ctx.new_str(spec.name.as_str()).into()).is_err()
            || set("__builtins__", self.globals.clone().into()).is_err()
        {
            return fail("the module could not be set up".to_owned());
        }
        let module = vm.new_module(&spec.name, globals.clone(), None);

        if let Some(limit) = self.time_limit.get() {
            self.watchdog.start(limit);
        }
        let scope = Scope::with_builtins(None, globals.clone(), vm);
        let outcome = vm.run_code_obj(code, scope);
        if self.time_limit.get().is_some() {
            self.watchdog.stop();
        }
        if let Err(exc) = outcome {
            return fail(describe(vm, &exc));
        }

        // The public functions it defines.
        let mut functions: Vec<(String, PyObjectRef)> = Vec::new();
        for (key, value) in globals.items_vec() {
            let Some(name) = key
                .downcast_ref::<PyStr>()
                .map(|s| s.expect_str().to_owned())
            else {
                continue;
            };
            if name.starts_with('_') || value.class().name().to_string() != "function" {
                continue;
            }
            let defined_here = value
                .get_attr("__module__", vm)
                .ok()
                .and_then(|m| {
                    m.downcast_ref::<PyStr>()
                        .map(|s| s.expect_str() == spec.name)
                })
                .unwrap_or(false);
            if defined_here {
                functions.push((name, value));
            }
        }

        // Nothing may be taken over.
        for (name, _) in &functions {
            if let Some(clash) = self.clash(&spec.name, name) {
                return fail(clash);
            }
        }
        let mut published = vec![spec.name.clone()];
        let _ = self
            .globals
            .set_item(spec.name.as_str(), module.clone().into(), vm);
        for (name, value) in functions {
            let _ = self.globals.set_item(name.as_str(), value, vm);
            published.push(name);
        }
        if let Some(loaded) = self.modules.borrow_mut().get_mut(&spec.name) {
            loaded.published = published.clone();
        } else {
            self.modules.borrow_mut().insert(
                spec.name.clone(),
                Loaded {
                    generation: spec.generation,
                    published: published.clone(),
                    result: Ok(()),
                },
            );
        }
        self.shared.modules.borrow_mut().insert(
            spec.name.clone(),
            ModuleSlot {
                object: Some(module.into()),
                failed: None,
            },
        );
        Ok(())
    }

    /// Why `function` of `module` cannot be published, if it cannot: it would
    /// take the place of something.
    fn clash(&self, module: &str, function: &str) -> Option<String> {
        if is_reserved(function) {
            return Some(format!(
                "function {function} in module {module} clashes with a built-in name"
            ));
        }
        if self.tabs.borrow().iter().any(|s| s == function) {
            return Some(format!(
                "function {function} in module {module} clashes with the tab {function}"
            ));
        }
        for (other, loaded) in self.modules.borrow().iter() {
            if other != module && loaded.published.iter().any(|p| p == function) {
                return Some(format!(
                    "function {function} in module {module} clashes with {} in module {other}",
                    if other == function {
                        format!("the module {function}")
                    } else {
                        format!("function {function}")
                    }
                ));
            }
        }
        None
    }

    /// A formula that uses a module, or a function it defines, depends on the
    /// module: editing it makes the formula out of date.
    pub(super) fn record_module_use(&self, formula: &str, ctx: &Rc<dyn Context>) {
        let provided = self.provided.borrow();
        if provided.is_empty() {
            return;
        }
        let mut owners: Vec<&String> = identifiers(formula)
            .into_iter()
            .filter_map(|name| provided.get(name))
            .collect();
        owners.sort();
        owners.dedup();
        for owner in owners {
            ctx.record(owner, &Path::root(), Depth::Shallow);
        }
    }

    /// For a name that is not defined: if a module that failed to load is where
    /// it would come from, the failure to report, which points at the module.
    pub(super) fn failed_module_name(&self, name: &str) -> Option<Failure> {
        let owner = self.provided.borrow().get(name)?.clone();
        let why = match &self.modules.borrow().get(&owner)?.result {
            Err(why) => why.clone(),
            Ok(()) => return None,
        };
        Some(Failure::new(
            ErrorKind::Name,
            format!("{name} comes from module {owner}, which failed to load: {why}"),
        ))
    }
}

/// The public functions a module declares.
fn public(spec: &ModuleSpec) -> impl Iterator<Item = &str> {
    spec.info
        .functions
        .iter()
        .map(String::as_str)
        .filter(|f| !f.starts_with('_'))
}

/// The indexes of the modules in an order in which each comes after the
/// modules it imports. Modules on a cycle are refused by the workbook before
/// they get here; they are simply placed where they are found.
fn import_order(specs: &[ModuleSpec]) -> Vec<usize> {
    let index: HashMap<&str, usize> = specs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.as_str(), i))
        .collect();
    let mut order = Vec::with_capacity(specs.len());
    let mut state = vec![0u8; specs.len()]; // 0 new, 1 on the way, 2 done
    fn visit(
        at: usize,
        specs: &[ModuleSpec],
        index: &HashMap<&str, usize>,
        state: &mut [u8],
        order: &mut Vec<usize>,
    ) {
        if state[at] != 0 {
            return;
        }
        state[at] = 1;
        for import in &specs[at].info.imports {
            if let Some(&next) = index.get(import.as_str()) {
                visit(next, specs, index, state, order);
            }
        }
        state[at] = 2;
        order.push(at);
    }
    for at in 0..specs.len() {
        visit(at, specs, &index, &mut state, &mut order);
    }
    order
}

/// A short account of an exception raised while loading: its class and message,
/// and the line of the module it came from.
fn describe(vm: &VirtualMachine, exc: &rustpython::vm::builtins::PyBaseExceptionRef) -> String {
    if exc.fast_isinstance(vm.ctx.exceptions.keyboard_interrupt) {
        return "the module ran out of time while loading".to_owned();
    }
    let class = exc.class().name().to_string();
    let detail = exc
        .as_object()
        .str(vm)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut traceback = String::new();
    let _ = vm.write_exception(&mut traceback, exc);
    let line = traceback
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("File \"<module "))
        .and_then(|l| l.split(", line ").nth(1))
        .and_then(|l| l.split(',').next())
        .map(|n| format!(" (line {n})"))
        .unwrap_or_default();
    if detail.is_empty() {
        format!("{class}{line}")
    } else {
        format!("{class}: {detail}{line}")
    }
}

/// `import name` inside a module: the workbook's modules and the exposed
/// standard library modules, and nothing else.
pub(super) fn import(shared: &Rc<Shared>, args: &FuncArgs, vm: &VirtualMachine) -> PyResult {
    let name = args
        .args
        .first()
        .and_then(|a| a.downcast_ref::<PyStr>())
        .map(|s| s.expect_str().to_owned())
        .ok_or_else(|| vm.new_type_error("the name of a module is text"))?;
    if let Some(slot) = shared.modules.borrow().get(&name) {
        return match (&slot.object, &slot.failed) {
            (Some(module), _) => Ok(module.clone()),
            (None, Some(why)) => Err(vm.new_import_error(
                format!("the module {name} failed to load: {why}"),
                vm.ctx.new_str(name.as_str()),
            )),
            (None, None) => Err(vm.new_import_error(
                format!("the module {name} is not loaded"),
                vm.ctx.new_str(name.as_str()),
            )),
        };
    }
    if let Some(module) = shared.stdlib.borrow().get(&name) {
        return Ok(module.clone());
    }
    Err(vm.new_import_error(
        format!("no module named {name} is available: only the workbook's modules and the standard library modules formulas can use can be imported"),
        vm.ctx.new_str(name.as_str()),
    ))
}
