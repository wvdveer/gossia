//! Formula storage, the dependency graph, recalculation, error values, proxy
//! objects, and the `PythonEngine` trait with its RustPython implementation.
//!
//! This is build step 2 (see DESIGN.md, Build order): the graph, the evaluator
//! and error values, run against [`fake::FakeEngine`]. The RustPython engine
//! and the proxy objects arrive with step 3. This crate must stay free of UI
//! and filesystem code.

mod calc;
mod check;
mod engine;
pub mod entry;
mod eval;
pub mod fake;
mod forms;
mod history;
mod model;
pub mod names;
pub mod python;
mod session;
pub mod sources;
mod trie;

pub use calc::{
    Applied, Calc, Ceilings, Cell, Change, ChangeKind, ColumnFormula, Decision, Detail, EditError,
    Effective, ExportFormat, ExprKey, ExprKind, ExprState, FieldKind, FieldPlaces, Fingerprint,
    Mentions, ModuleProblem, ModuleStatus, SchemaFormat, Setting, TrustStore, Unit,
    WorkbookSettings, comment_text_problem,
};
pub use calc::{
    ENGINE_NAME, ENGINE_REVISION, FORMAT_MAJOR, FORMAT_MINOR, Meta, Opened, Package, PackageError,
    RESERVED_NAMES_VERSION, safe_path,
};
pub use check::Computed;
pub use engine::{
    Call, Context, Depth, Engine, Failure, Immediate, ImmediateResult, ModuleInfo, ModuleReport,
    ModuleSpec, Read, Syntax, SyntaxError, Write, WriteOp,
};
pub use eval::{Evaluation, Hooks, Outcome, ProcedureResult, Snapshot};
pub use forms::{DEFAULT_FORM, FormStore};
pub use history::{DEFAULT_MAX_ENTRIES, DEFAULT_MAX_WEIGHT};
pub use model::{Dep, Loc};
pub use session::{EngineFactory, EvalSettings, Evaluator, Finished, Session, Status};

/// The version of this crate, as set in the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
#[path = "calc_tests.rs"]
mod calc_tests;
#[cfg(test)]
#[path = "comments_tests.rs"]
mod comments_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_match_the_core_crate() {
        assert_eq!(VERSION, cliotree_core::VERSION);
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;

#[cfg(test)]
#[path = "schema_tests.rs"]
mod schema_tests;

#[cfg(test)]
#[path = "package_tests.rs"]
mod package_tests;

#[cfg(test)]
#[path = "xsd_fields_tests.rs"]
mod xsd_fields_tests;

#[cfg(test)]
#[path = "control_exprs_tests.rs"]
mod control_exprs_tests;

#[cfg(test)]
#[path = "columns_tests.rs"]
mod columns_tests;

#[cfg(test)]
#[path = "procedure_tests.rs"]
mod procedure_tests;

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;

#[cfg(test)]
#[path = "tabs_tests.rs"]
mod tabs_tests;
