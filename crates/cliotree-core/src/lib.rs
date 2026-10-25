//! Core data model for ClioTree: `Value`, `TypeSpec`, schema lowering,
//! validation, and the JSON and XML format adapters.
//!
//! This crate has no UI and no Python, so that it can be tested headlessly and
//! reused by a later web port (see DESIGN.md, Application structure).

pub mod form;
pub mod json;
pub mod jsonschema;
pub mod metaschema;
mod num;
mod path;
mod typespec;
pub mod validate;
mod value;
pub mod xml;
pub mod xmltree;
pub mod xsd;

pub use num::Num;
pub use path::{ParsePathError, Path, Segment};
pub use typespec::{
    Field, Kind, Pattern, Range, Resolved, StringConstraints, StringFormat, TypeDef, TypeId,
    TypeSpec, XmlInfo,
};
pub use value::{DateTimeValue, ErrorKind, ErrorValue, Node, NodeId, PathError, Value};

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
