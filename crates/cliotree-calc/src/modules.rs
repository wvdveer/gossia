//! The workbook's Python modules, as the calculation layer keeps them
//! (DESIGN.md, "Python modules"): their sources, the number of each reload (its
//! generation), what loading each came to, and how a change to one reaches the
//! formulas that use it. Loading itself is the engine's, on the evaluation
//! thread, between evaluations.
//!
//! **How a change reaches formulas.** A module is treated as a tab that has no
//! data: a formula that uses the module, or a function it defines, has read the
//! "tab" of the module's name, and editing the module is an edit of it. So the
//! edit log and the index of readers do the work: exactly the formulas that
//! used the module are out of date, and a result computed from an older
//! generation is dropped like one that an edit overtook. A module and a tab
//! never share a name, so the names cannot be confused.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cliotree_core::Path;

use super::{Calc, EditError};
use crate::engine::{ModuleInfo, ModuleReport, ModuleSpec, SyntaxError};
use crate::history::Op;
use crate::names::{self, Subject, identifiers};

/// What loading a module has come to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleStatus {
    /// The latest source has not been loaded yet.
    Pending,
    Loaded,
    /// It could not be loaded; the message says why.
    Failed(String),
    /// It is not loaded because the workbook is not trusted.
    Untrusted,
}

/// A problem that *Compile* finds in a module (DESIGN.md, "Python modules").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleProblem {
    /// Where it is, 1-based, if that is known.
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

impl ModuleProblem {
    fn at(line: usize, column: usize, message: String) -> ModuleProblem {
        ModuleProblem {
            line: Some(line),
            column: Some(column),
            message,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Module {
    pub source: Arc<str>,
    /// Goes up with every change of the source, and is part of what a result
    /// depends on.
    pub generation: u64,
    /// What the source says about itself, or why it does not parse.
    pub info: Result<ModuleInfo, SyntaxError>,
    pub status: ModuleStatus,
}

impl Module {
    fn new(source: &str, generation: u64, calc: &Calc) -> Module {
        Module {
            source: Arc::from(source),
            generation,
            info: calc.syntax.module_info(source),
            status: ModuleStatus::Pending,
        }
    }

    /// The functions a formula can call by bare name: top-level, not private.
    pub fn public_functions(&self) -> Vec<String> {
        match &self.info {
            Ok(info) => info
                .functions
                .iter()
                .filter(|f| !f.starts_with('_'))
                .cloned()
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl Calc {
    // ------------------------------------------------------------ reading

    pub fn module_names(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }

    pub fn module_source(&self, name: &str) -> Option<&str> {
        self.modules.get(name).map(|m| &*m.source)
    }

    pub fn module_status(&self, name: &str) -> Option<&ModuleStatus> {
        static UNTRUSTED: ModuleStatus = ModuleStatus::Untrusted;
        let module = self.modules.get(name)?;
        Some(if self.trusted {
            &module.status
        } else {
            &UNTRUSTED
        })
    }

    /// The generation of a module: one more for each change of its source.
    pub fn module_generation(&self, name: &str) -> Option<u64> {
        self.modules.get(name).map(|m| m.generation)
    }

    /// The functions a module defines that formulas can call by bare name.
    pub fn module_functions(&self, name: &str) -> Vec<String> {
        self.modules
            .get(name)
            .map(Module::public_functions)
            .unwrap_or_default()
    }

    /// The names a button can run, for the property page's list: the public
    /// functions of the modules, in module order, by bare name. Read from
    /// the source, so it needs neither trust nor a loaded module. A module
    /// that does not parse, or that failed to load (a clash of names among
    /// them), gives none; a name is listed once.
    pub fn procedure_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for m in self.modules.values() {
            if matches!(m.status, ModuleStatus::Failed(_)) {
                continue;
            }
            for function in m.public_functions() {
                if !names.contains(&function) {
                    names.push(function);
                }
            }
        }
        names
    }

    /// Each module that has failed, and why, for the list of problems.
    pub fn module_issues(&self) -> Vec<(String, String)> {
        self.modules
            .iter()
            .filter(|_| self.trusted)
            .filter_map(|(name, m)| match (&m.info, &m.status) {
                (Err(e), _) => Some((name.clone(), format!("syntax error: {e}"))),
                (_, ModuleStatus::Failed(why)) => Some((name.clone(), why.clone())),
                _ => None,
            })
            .collect()
    }

    /// What *Compile* finds in a module: a syntax error (then nothing more),
    /// an import of a module that is on a cycle of imports, an import of
    /// something that is neither a workbook module nor a standard library
    /// module formulas may use, and what loading it came to if it failed.
    /// Each has its line and column where they are known. `None` if there is
    /// no such module.
    pub fn module_problems(&self, name: &str) -> Option<Vec<ModuleProblem>> {
        let module = self.modules.get(name)?;
        let info = match &module.info {
            Ok(info) => info,
            Err(e) => {
                return Some(vec![ModuleProblem::at(
                    e.line,
                    e.column,
                    format!("syntax error: {}", e.message),
                )]);
            }
        };
        let mut problems = Vec::new();
        let cycle = self.import_cycles().remove(name);
        // The module this one imports on the way round the cycle.
        let next = cycle.as_ref().and_then(|message| {
            message
                .trim_start_matches("import cycle: ")
                .split(" -> ")
                .nth(1)
                .map(str::to_owned)
        });
        let standard = self.syntax.standard_modules();
        let mut unknown = false;
        for (i, import) in info.imports.iter().enumerate() {
            let (line, column) = info.import_at.get(i).copied().unwrap_or((1, 1));
            if self.modules.contains_key(import) {
                if next.as_deref() == Some(import.as_str())
                    && let Some(message) = &cycle
                {
                    problems.push(ModuleProblem::at(line, column, message.clone()));
                }
            } else if standard.is_some_and(|known| !known.contains(&import.as_str())) {
                unknown = true;
                problems.push(ModuleProblem::at(
                    line,
                    column,
                    format!(
                        "no module named {import} is available: only the workbook's modules \
                         and the standard library modules formulas can use can be imported"
                    ),
                ));
            }
        }
        let charts = self.chart_problems(name);
        let clash = !charts.is_empty();
        for (line, column, message) in charts {
            problems.push(ModuleProblem::at(line, column, message));
        }
        if let ModuleStatus::Failed(why) = &module.status
            && cycle.is_none()
            && !clash
            && !(unknown && why.contains("no module named"))
        {
            problems.push(ModuleProblem {
                line: line_in(why),
                column: None,
                message: why.clone(),
            });
        }
        Some(problems)
    }

    /// How many modules have a source that is not loaded yet.
    pub fn pending_modules(&self) -> usize {
        if !self.trusted {
            return 0;
        }
        self.modules
            .values()
            .filter(|m| m.status == ModuleStatus::Pending)
            .count()
    }

    // ------------------------------------------------------------ editing

    /// Adds a module after the others. Its name must be valid, not reserved,
    /// and different from every module and tab, ignoring case. One entry in
    /// the history.
    pub fn add_module(&mut self, name: &str, source: &str) -> Result<(), EditError> {
        self.check_module_name(name, None)?;
        let position = self.modules.len();
        self.store_add_module(name, source, position);
        self.history.record(
            format!("Insert module {name}"),
            vec![Op::AddModule {
                name: name.to_owned(),
                source: source.to_owned(),
                position,
            }],
            None,
        );
        Ok(())
    }

    /// The first free name of the form `Module1`, `Module2`, ..., for
    /// *Insert > Module*, as Excel names them.
    pub fn new_module_name(&self) -> String {
        (1..)
            .map(|n| format!("Module{n}"))
            .find(|name| self.check_module_name(name, None).is_ok())
            .expect("some number is free")
    }

    /// Why `name` cannot be given to a module (`renaming` is the module being
    /// renamed, if any), or `None` if it can.
    pub fn module_name_problem(&self, name: &str, renaming: Option<&str>) -> Option<String> {
        match self.check_module_name(name, renaming) {
            Ok(()) => None,
            Err(EditError::BadName(why)) => Some(why),
            Err(other) => Some(other.to_string()),
        }
    }

    /// Replaces the source of a module. One entry in the history.
    pub fn set_module_source(&mut self, name: &str, source: &str) -> Result<(), EditError> {
        let before = self
            .module_source(name)
            .ok_or_else(|| EditError::NoModule(name.to_owned()))?
            .to_owned();
        if before == source {
            return Ok(());
        }
        self.store_module_source(name, source)?;
        self.history.record(
            format!("Edit module {name}"),
            vec![Op::ModuleSource {
                name: name.to_owned(),
                before,
                after: source.to_owned(),
            }],
            None,
        );
        Ok(())
    }

    /// Takes a module away. Formulas that used it show `#NAME!`. One entry in
    /// the history, which puts it back in its place.
    pub fn remove_module(&mut self, name: &str) -> Result<(), EditError> {
        let Some(position) = self.modules.get_index_of(name) else {
            return Err(EditError::NoModule(name.to_owned()));
        };
        let source = self.modules[position].source.to_string();
        self.store_remove_module(name)?;
        self.history.record(
            format!("Remove module {name}"),
            vec![Op::RemoveModule {
                name: name.to_owned(),
                source,
                position,
            }],
            None,
        );
        Ok(())
    }

    /// Gives a module another name. Formulas that used the old name show
    /// `#NAME!`; names in formulas are never rewritten. One entry in the
    /// history; a name equal to the current one is no edit.
    pub fn rename_module(&mut self, from: &str, to: &str) -> Result<(), EditError> {
        if !self.modules.contains_key(from) {
            return Err(EditError::NoModule(from.to_owned()));
        }
        if from == to {
            return Ok(());
        }
        self.check_module_name(to, Some(from))?;
        self.store_rename_module(from, to)?;
        self.history.record(
            format!("Rename module {from} to {to}"),
            vec![Op::RenameModule {
                from: from.to_owned(),
                to: to.to_owned(),
            }],
            None,
        );
        Ok(())
    }

    /// The module renames done since the last call, oldest first, undoing and
    /// redoing included, so that the window can follow them.
    pub fn take_module_renames(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.module_renames)
    }

    /// Puts a module in at `position`, for adding one and for undoing a
    /// removal.
    pub(super) fn store_add_module(&mut self, name: &str, source: &str, position: usize) {
        self.module_generation += 1;
        let module = Module::new(source, self.module_generation, self);
        self.modules.insert(name.to_owned(), module);
        let last = self.modules.len() - 1;
        self.modules.move_index(last, position.min(last));
        self.module_changed(name, &[]);
    }

    pub(super) fn store_remove_module(&mut self, name: &str) -> Result<(), EditError> {
        let Some(module) = self.modules.shift_remove(name) else {
            return Err(EditError::NoModule(name.to_owned()));
        };
        let old = module.public_functions();
        self.module_changed(name, &old);
        Ok(())
    }

    pub(super) fn store_rename_module(&mut self, from: &str, to: &str) -> Result<(), EditError> {
        if !self.modules.contains_key(from) {
            return Err(EditError::NoModule(from.to_owned()));
        }
        let position = self.modules.get_index_of(from).unwrap_or(0);
        let mut module = self.modules.shift_remove(from).expect("checked above");
        let old = module.public_functions();
        self.module_changed(from, &old);
        self.module_generation += 1;
        module.generation = self.module_generation;
        module.status = ModuleStatus::Pending;
        self.modules.insert(to.to_owned(), module);
        let last = self.modules.len() - 1;
        self.modules.move_index(last, position);
        self.module_changed(to, &[]);
        self.module_renames.push((from.to_owned(), to.to_owned()));
        Ok(())
    }

    fn check_module_name(&self, name: &str, renaming: Option<&str>) -> Result<(), EditError> {
        if let Some(why) = names::problem(name, Subject::Module) {
            return Err(EditError::BadName(why));
        }
        let lower = name.to_lowercase();
        if self
            .modules
            .keys()
            .any(|m| Some(m.as_str()) != renaming && m.to_lowercase() == lower)
        {
            return Err(EditError::BadName(format!(
                "a module called {name} exists already (names ignore case)"
            )));
        }
        if self.sheets.keys().any(|s| s.to_lowercase() == lower) {
            return Err(EditError::BadName(format!(
                "{name} is the name of a tab, and a module cannot share it"
            )));
        }
        Ok(())
    }

    /// Changes the source of a module, for an edit and for undoing and
    /// redoing one.
    pub(super) fn store_module_source(
        &mut self,
        name: &str,
        source: &str,
    ) -> Result<(), EditError> {
        let old_functions = self
            .modules
            .get(name)
            .ok_or_else(|| EditError::NoModule(name.to_owned()))?
            .public_functions();
        self.module_generation += 1;
        let module = Module::new(source, self.module_generation, self);
        self.modules.insert(name.to_owned(), module);
        self.module_changed(name, &old_functions);
        Ok(())
    }

    // ------------------------------------------------------- consequences

    /// Records the change of a module, so that the formulas that used it, and
    /// the modules that use it, and what uses those, are out of date.
    fn module_changed(&mut self, name: &str, old_functions: &[String]) {
        let mut touched: Vec<String> = vec![name.to_owned()];
        touched.extend(old_functions.iter().cloned());
        if let Some(module) = self.modules.get(name) {
            touched.extend(module.public_functions());
        }
        for user in self.module_users(name) {
            // A module that uses it has to be loaded again too.
            if let Some(module) = self.modules.get_mut(&user) {
                self.module_generation += 1;
                module.generation = self.module_generation;
                module.status = ModuleStatus::Pending;
                touched.push(user.clone());
                touched.extend(module.public_functions());
            }
        }
        touched.sort();
        touched.dedup();
        for name in touched {
            self.changed(&name, &Path::root());
        }
    }

    /// The modules that use `name`, directly or through others: they import
    /// it, or mention it or one of its functions.
    fn module_users(&self, name: &str) -> Vec<String> {
        let mut provides: HashMap<&str, Vec<String>> = HashMap::new();
        for (m, module) in &self.modules {
            let mut names = module.public_functions();
            names.push(m.clone());
            provides.insert(m.as_str(), names);
        }
        let uses = |user: &str, used: &str| -> bool {
            let Some(module) = self.modules.get(user) else {
                return false;
            };
            let idents = identifiers(&module.source);
            let imports = module
                .info
                .as_ref()
                .is_ok_and(|i| i.imports.iter().any(|x| x == used));
            imports
                || provides
                    .get(used)
                    .is_some_and(|names| names.iter().any(|n| idents.contains(n.as_str())))
        };
        let mut found: Vec<String> = Vec::new();
        let mut frontier = vec![name.to_owned()];
        while let Some(used) = frontier.pop() {
            for user in self.modules.keys() {
                if *user != used && user != name && !found.contains(user) && uses(user, &used) {
                    found.push(user.clone());
                    frontier.push(user.clone());
                }
            }
        }
        found
    }

    // ------------------------------------------------- for the evaluation

    /// The modules as the engine is asked to have them loaded.
    pub(super) fn module_specs(&self) -> Arc<Vec<ModuleSpec>> {
        let cycles = self.import_cycles();
        Arc::new(
            self.modules
                .iter()
                .map(|(name, m)| {
                    let refused = match &m.info {
                        Err(e) => Some(format!("syntax error: {e}")),
                        Ok(_) => cycles.get(name).cloned().or_else(|| {
                            self.chart_problems(name)
                                .into_iter()
                                .next()
                                .map(|(line, _, message)| format!("line {line}: {message}"))
                        }),
                    };
                    ModuleSpec {
                        name: name.clone(),
                        source: Arc::clone(&m.source),
                        generation: m.generation,
                        info: m.info.clone().unwrap_or_default(),
                        refused,
                    }
                })
                .collect(),
        )
    }

    /// For each module on a cycle of imports, the message that names the
    /// cycle (`A -> B -> A`). None of the modules on it is loaded.
    fn import_cycles(&self) -> HashMap<String, String> {
        let edges = |name: &str| -> Vec<&str> {
            self.modules
                .get(name)
                .and_then(|m| m.info.as_ref().ok())
                .map(|i| {
                    i.imports
                        .iter()
                        .filter(|x| self.modules.contains_key(x.as_str()))
                        .map(String::as_str)
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut found: HashMap<String, String> = HashMap::new();
        for start in self.modules.keys() {
            // A path that leads back to `start`: depth first, remembering the way.
            let mut stack: Vec<(&str, Vec<&str>)> = vec![(start.as_str(), vec![start.as_str()])];
            let mut seen: HashSet<&str> = HashSet::new();
            while let Some((at, way)) = stack.pop() {
                for next in edges(at) {
                    if next == start {
                        let mut named = way.clone();
                        named.push(next);
                        found
                            .entry(start.clone())
                            .or_insert_with(|| format!("import cycle: {}", named.join(" -> ")));
                    } else if seen.insert(next) {
                        let mut longer = way.clone();
                        longer.push(next);
                        stack.push((next, longer));
                    }
                }
            }
        }
        found
    }

    /// Takes what the engine found out about loading the modules. A report for
    /// a generation that has been replaced is ignored.
    pub(super) fn apply_module_reports(&mut self, reports: Vec<ModuleReport>) {
        // An expression on a control may call a function of a module: once
        // the modules as they are now have been loaded, each runs again.
        if !reports.is_empty() && self.exprs.modules_seen != self.module_generation {
            self.exprs.modules_seen = self.module_generation;
            self.exprs.mark_all_stale();
        }
        let mut any = false;
        for report in reports {
            let Some(module) = self.modules.get_mut(&report.name) else {
                continue;
            };
            if module.generation != report.generation {
                continue;
            }
            let status = match report.result {
                Ok(()) => ModuleStatus::Loaded,
                Err(why) => ModuleStatus::Failed(why),
            };
            if module.status != status {
                module.status = status;
                any = true;
            }
        }
        if any {
            self.revision += 1;
        }
    }
}

/// The line a load error names at its end, as `... (line 3)`.
fn line_in(message: &str) -> Option<usize> {
    let rest = message.rsplit_once("(line ")?.1;
    rest.strip_suffix(')')?.trim().parse().ok()
}
