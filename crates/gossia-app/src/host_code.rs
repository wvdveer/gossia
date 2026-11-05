//! Modules edited in the host's own editor (DESIGN.md, Web port, 0.3.0, In
//! VS Code): in VS Code a module opens as a file of VS Code's (`gossia:`),
//! with its highlighting, search and Python extensions, and saving that file
//! puts the module in the workbook. The code editor stays in the workbook's
//! window for the project explorer, *Compile* and the Immediate window, but
//! opening a module there opens it in the host's editor.
//!
//! The host is told the modules (names and sources) at the start and each
//! time they change, so that it can show them and refresh what it shows after
//! an undo; what is saved there comes back as a [`ModuleEdit`], made in the
//! workbook as one entry in its history and answered with whether it was.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};

use crate::GossiaApp;
use crate::host::{self, Host, ModuleEdit};

thread_local! {
    /// The edits the host has sent, until the next frame.
    static EDITS: RefCell<Vec<ModuleEdit>> = const { RefCell::new(Vec::new()) };
    /// What the modules were when the host was last told them.
    static TOLD: RefCell<Option<u64>> = const { RefCell::new(None) };
}

/// A module was written, renamed or removed in the host's editor.
pub fn module_edited(edit: ModuleEdit) {
    EDITS.with(|e| e.borrow_mut().push(edit));
}

/// The host that edits modules, if there is one.
fn editing_host() -> Option<std::rc::Rc<dyn Host>> {
    host::current().filter(|h| h.edits_modules())
}

impl GossiaApp {
    /// Makes the edits the host has sent, and tells it the modules when they
    /// have changed. Each frame.
    pub(crate) fn sync_host_modules(&mut self) {
        let Some(host) = editing_host() else {
            return;
        };
        for edit in EDITS.with(|e| std::mem::take(&mut *e.borrow_mut())) {
            let (id, done) = match edit {
                ModuleEdit::Write { id, name, text } => (id, self.write_module(&name, &text)),
                ModuleEdit::Rename { id, from, to } => (id, self.rename_module(&from, &to)),
                ModuleEdit::Remove { id, name } => (id, self.remove_module(&name)),
            };
            host.module_answer(id, done.err().as_deref());
        }
        let modules: Vec<(String, String)> = {
            let calc = self.calc();
            calc.module_names()
                .map(|name| {
                    let source = calc.module_source(name).unwrap_or_default();
                    (name.to_owned(), source.to_owned())
                })
                .collect()
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        modules.hash(&mut hasher);
        let now = hasher.finish();
        if TOLD.with(|t| t.replace(Some(now))) != Some(now) {
            host.modules_changed(&modules);
        }
    }

    /// A module's file saved in the host's editor: its new source, or a new
    /// module when there is none of that name. One entry in the history.
    pub fn write_module(&mut self, name: &str, text: &str) -> Result<(), String> {
        self.commit_code();
        let calc = self.session.calc_mut();
        let done = if calc.module_source(name).is_some() {
            calc.set_module_source(name, text)
        } else {
            calc.add_module(name, text)
        };
        done.map_err(|e| e.to_string())
    }

    /// Opens a module in the host's editor at a line and column (0-based),
    /// when modules are edited there. Whether it was.
    pub(crate) fn open_module_outside(&self, name: &str, line: usize, column: usize) -> bool {
        let Some(host) = editing_host() else {
            return false;
        };
        if self.calc().module_source(name).is_none() {
            return false;
        }
        host.open_module(name, line + 1, column + 1);
        true
    }

    /// Whether modules open in the host's editor rather than in a tab here.
    pub fn modules_open_outside(&self) -> bool {
        editing_host().is_some()
    }
}
