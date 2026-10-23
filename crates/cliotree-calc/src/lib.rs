//! Formula storage, the dependency graph, recalculation, error values, proxy
//! objects, and the `PythonEngine` trait with its RustPython implementation.
//!
//! The RustPython dependency arrives with build step 3 (see DESIGN.md, Build
//! order). This crate must stay free of UI and filesystem code.

/// The version of this crate, as set in the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_match_the_core_crate() {
        assert_eq!(VERSION, cliotree_core::VERSION);
    }
}
