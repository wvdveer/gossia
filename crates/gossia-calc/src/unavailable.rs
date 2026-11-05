//! The data source kinds the wasm version does not have (DESIGN.md, "Data
//! sources": the kinds available depend on the target). A workbook that names
//! one opens and works; a call to it fails with `#SOURCE!` saying the kind is
//! not available here, and its Test says the same.

use crate::sources::{SourceDef, TestOutcome};

fn unavailable(def: &SourceDef) -> String {
    format!(
        "{}: {} sources are not available here",
        def.name,
        def.kind.label()
    )
}

/// SQLite files.
pub mod sqlite {
    use std::time::Duration;

    use serde_json::Value as Json;

    use super::{SourceDef, TestOutcome, unavailable};
    use crate::http::Limits;

    pub fn query(
        def: &SourceDef,
        _sql: &str,
        _params: &Json,
        _limits: Limits,
        _writable: bool,
    ) -> Result<Json, String> {
        Err(unavailable(def))
    }

    pub fn execute(
        def: &SourceDef,
        _sql: &str,
        _params: &Json,
        _limits: Limits,
    ) -> Result<u64, String> {
        Err(unavailable(def))
    }

    pub fn test(def: &SourceDef, _timeout: Duration) -> TestOutcome {
        TestOutcome::failed(unavailable(def))
    }
}

/// PostgreSQL, MariaDB and ODBC.
pub mod database {
    use std::time::Duration;

    use serde_json::Value as Json;

    use super::{SourceDef, TestOutcome, unavailable};
    use crate::http::Limits;

    pub fn query(
        def: &SourceDef,
        _sql: &str,
        _params: &Json,
        _limits: Limits,
    ) -> Result<Json, String> {
        Err(unavailable(def))
    }

    pub fn execute(
        def: &SourceDef,
        _sql: &str,
        _params: &Json,
        _limits: Limits,
    ) -> Result<u64, String> {
        Err(unavailable(def))
    }

    pub fn test(def: &SourceDef, _timeout: Duration) -> TestOutcome {
        TestOutcome::failed(unavailable(def))
    }
}

/// Beancount journals.
pub mod journal {
    use serde_json::Value as Json;

    use super::{SourceDef, TestOutcome, unavailable};

    pub fn read_call(def: &SourceDef, _method: &str, _args: &Json) -> Result<Json, String> {
        Err(unavailable(def))
    }

    pub fn append(def: &SourceDef, _request: &Json) -> Result<Json, String> {
        Err(unavailable(def))
    }

    pub fn test(def: &SourceDef) -> TestOutcome {
        TestOutcome::failed(unavailable(def))
    }

    /// A journal that cannot be read never changes.
    pub fn changed(_def: &SourceDef) -> bool {
        false
    }
}
