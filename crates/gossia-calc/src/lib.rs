//! Formula storage, the dependency graph, recalculation, error values, proxy
//! objects, and the `PythonEngine` trait with its RustPython implementation.
//!
//! This is build step 2 (see DESIGN.md, Build order): the graph, the evaluator
//! and error values, run against [`fake::FakeEngine`]. The RustPython engine
//! and the proxy objects arrive with step 3. This crate must stay free of UI
//! and filesystem code.

mod calc;
mod check;
#[cfg(not(target_family = "wasm"))]
pub mod database;
mod engine;
pub mod entry;
mod eval;
pub mod fake;
pub mod fetches;
mod forms;
mod history;
pub mod http;
#[cfg(not(target_family = "wasm"))]
pub mod journal;
mod model;
pub mod names;
mod node_bytes;
pub mod python;
mod session;
// The evaluator of the wasm version. It is compiled everywhere so that its
// tests run with the others, and is the `Evaluator` only on wasm.
#[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
mod session_web;
pub mod sources;
#[cfg(not(target_family = "wasm"))]
pub mod sqlite;
mod trie;
#[cfg(target_family = "wasm")]
mod unavailable;
pub mod wire;
#[cfg(target_family = "wasm")]
pub use unavailable::{database, journal, sqlite};

pub use calc::{
    Applied, Calc, Ceilings, Cell, Change, ChangeKind, ChartType, ColumnFormula, Decision, Detail,
    Disconnect, DisconnectError, DisconnectPlan, DisconnectRunner, EditError, Effective,
    ExportFormat, ExprKey, ExprKind, ExprState, FieldKind, FieldPlaces, Fingerprint,
    KEEP_REPORT_RESULTS, Mentions, ModuleFix, ModuleProblem, ModuleStatus, ParamState, REPORT_FORM,
    RedefineGroup, ReportCache, SchemaFormat, Setting, TabKind, TrustStore, Unit, WorkbookSettings,
    apply_and_rerun, cell_of, comment_text_problem, design_digest, find_disconnect, matplotlib_fix,
    today,
};
pub use calc::{
    ENGINE_NAME, ENGINE_REVISION, FORMAT_MAJOR, FORMAT_MINOR, Meta, Opened, Package, PackageError,
    RESERVED_NAMES_VERSION, safe_path,
};
pub use check::Computed;
pub use engine::{
    Answer, Call, ChartInfo, ChartParam, Context, Depth, Engine, Failure, Immediate,
    ImmediateResult, ModuleInfo, ModuleReport, ModuleSpec, Read, SourceRead, SourceScan, Syntax,
    SyntaxError, Write, WriteOp,
};
pub use eval::{Evaluation, Hooks, Outcome, ProcedureResult, Snapshot};
pub use forms::{DEFAULT_FORM, FormStore};
pub use history::{DEFAULT_MAX_ENTRIES, DEFAULT_MAX_WEIGHT};
pub use model::{Dep, Loc};
#[cfg(target_family = "wasm")]
pub use session::WorkerLink;
pub use session::{
    EngineFactory, EvalSettings, Evaluator, Finished, Session, SessionRunner, Status,
};

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
        assert_eq!(VERSION, gossia_core::VERSION);
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
#[path = "xsd_children_tests.rs"]
mod xsd_children_tests;
#[cfg(test)]
#[path = "xsd_fields_tests.rs"]
mod xsd_fields_tests;
#[cfg(test)]
#[path = "xsd_imports_tests.rs"]
mod xsd_imports_tests;

#[cfg(test)]
#[path = "control_exprs_tests.rs"]
mod control_exprs_tests;

#[cfg(test)]
#[path = "control_exprs_additions_tests.rs"]
mod control_exprs_additions_tests;

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

#[cfg(test)]
#[path = "reports_tests.rs"]
mod reports_tests;

#[cfg(test)]
#[path = "report_exprs_tests.rs"]
mod report_exprs_tests;

#[cfg(test)]
#[path = "report_bands_tests.rs"]
mod report_bands_tests;

#[cfg(test)]
#[path = "report_params_tests.rs"]
mod report_params_tests;

#[cfg(test)]
#[path = "charts_tests.rs"]
mod charts_tests;
#[cfg(test)]
#[path = "report_cache_tests.rs"]
mod report_cache_tests;

#[cfg(test)]
#[path = "formats_tests.rs"]
mod formats_tests;

#[cfg(test)]
#[path = "copybooks_tests.rs"]
mod copybooks_tests;

#[cfg(test)]
#[path = "entries_tests.rs"]
mod entries_tests;

#[cfg(test)]
#[path = "node_bytes_tests.rs"]
mod node_bytes_tests;
