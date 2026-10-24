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
mod history;
mod model;
pub mod names;
pub mod python;
mod session;
mod trie;

pub use calc::{
    Applied, Calc, Cell, Change, ChangeKind, Decision, EditError, FieldKind, FieldPlaces,
    Fingerprint, ModuleStatus, SchemaFormat, TrustStore,
};
pub use check::Computed;
pub use engine::{
    Context, Depth, Engine, Failure, ModuleInfo, ModuleReport, ModuleSpec, Read, Syntax,
    SyntaxError,
};
pub use eval::{Evaluation, Hooks, Outcome, Snapshot};
pub use model::{Dep, Loc};
pub use session::{EngineFactory, EvalSettings, Evaluator, Session, Status};

/// The version of this crate, as set in the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
#[path = "calc_tests.rs"]
mod calc_tests;

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
