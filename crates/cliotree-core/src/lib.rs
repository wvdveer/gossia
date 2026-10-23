//! Core data model for ClioTree: `Value`, `TypeSpec`, schema lowering,
//! validation, and the JSON and XML format adapters.
//!
//! This crate has no UI and no Python, so that it can be tested headlessly and
//! reused by a later web port (see DESIGN.md, Application structure).

/// The version of this crate, as set in the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!VERSION.is_empty());
    }
}
