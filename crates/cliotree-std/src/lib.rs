//! Built-in functions available to formulas.
//!
//! Most built-ins are one-line Python over the standard library; only the
//! tree-aware pieces need Rust (see DESIGN.md, Semantics).

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
