//! Core data model for Gossia: `Value`, `TypeSpec`, schema lowering,
//! validation, and the JSON and XML format adapters.
//!
//! This crate has no UI and no Python, so that it can be tested headlessly and
//! reused by a later web port (see DESIGN.md, Application structure).

pub mod binary;
pub mod copybook;
pub mod dataformat;
pub mod delimited;
#[cfg(test)]
mod delimited_tests;
pub mod form;
pub mod format;
pub mod infer;
#[cfg(test)]
mod infer_tests;
pub mod json;
pub mod json5;
pub mod jsonschema;
pub mod metaschema;
mod num;
mod path;
pub mod report;
#[cfg(test)]
mod report_tests;
pub mod spreadsheet;
pub mod toml_format;
mod typespec;
pub mod validate;
mod value;
pub mod xml;
pub mod xmltree;
pub mod xsd;
pub mod yaml;

pub use num::Num;
pub use path::{ParsePathError, Path, Segment};
pub use typespec::{
    Field, Kind, Pattern, Range, Resolved, StringConstraints, StringFormat, TypeDef, TypeId,
    TypeSpec, XmlInfo,
};
pub use value::{
    CommentPlace, DateTimeValue, ErrorKind, ErrorValue, Node, NodeId, PathError, Value,
};

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
