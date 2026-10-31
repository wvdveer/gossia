//! Built-in functions available to formulas.
//!
//! Most built-ins are one-line Python over the standard library; only the
//! tree-aware pieces need Rust (see DESIGN.md, Semantics). This crate holds the
//! Python source of the Excel-named functions and the description of each, for
//! the Insert Function dialog and for name completion. The engine in
//! `cliotree-calc` runs the source.

/// The version of this crate, as set in the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Python source of the Excel-named functions. It runs in the namespace
/// of the engine's prelude and defines every name in [`NAMES`].
pub const PYTHON: &str = include_str!("functions.py");

/// The functions the source defines, which the engine puts in the names a
/// formula sees.
pub const NAMES: &[&str] = &[
    "SUM", "AVERAGE", "MIN", "MAX", "COUNT", "IF", "LOOKUP", "FILTER", "SORT", "UNIQUE", "CONCAT",
    "TEXTJOIN", "TEXT", "NA",
];

/// The exception class `NA()` raises, which the engine turns into `#N/A`.
pub const NOT_AVAILABLE: &str = "NotAvailable";

/// One name a formula can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Function {
    pub name: &'static str,
    /// What is written into the formula bar: the name with its brackets, or
    /// the module name and a dot.
    pub insert: &'static str,
    pub signature: &'static str,
    /// One line.
    pub description: &'static str,
    pub group: Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Math,
    Logic,
    Lookup,
    Text,
    Errors,
    Data,
    Python,
    Modules,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Math => "Math",
            Group::Logic => "Logic",
            Group::Lookup => "Lookup and lists",
            Group::Text => "Text",
            Group::Errors => "Errors",
            Group::Data => "Data",
            Group::Python => "Python",
            Group::Modules => "Standard library modules",
        }
    }

    pub const ALL: [Group; 8] = [
        Group::Math,
        Group::Logic,
        Group::Lookup,
        Group::Text,
        Group::Errors,
        Group::Data,
        Group::Python,
        Group::Modules,
    ];
}

const fn f(
    name: &'static str,
    insert: &'static str,
    signature: &'static str,
    description: &'static str,
    group: Group,
) -> Function {
    Function {
        name,
        insert,
        signature,
        description,
        group,
    }
}

/// The Excel-named functions, described.
const FUNCTIONS: &[Function] = &[
    f(
        "SUM",
        "SUM(",
        "SUM(*values)",
        "The sum of the numbers in the arguments; lists and arrays are opened one level.",
        Group::Math,
    ),
    f(
        "AVERAGE",
        "AVERAGE(",
        "AVERAGE(*values)",
        "The mean of the numbers in the arguments; none gives #DIV/0!.",
        Group::Math,
    ),
    f(
        "MIN",
        "MIN(",
        "MIN(*values, default=...)",
        "The smallest element: numbers, text and dates all compare.",
        Group::Math,
    ),
    f(
        "MAX",
        "MAX(",
        "MAX(*values, default=...)",
        "The largest element.",
        Group::Math,
    ),
    f(
        "COUNT",
        "COUNT(",
        "COUNT(*values)",
        "How many elements are not None. An error among them is raised.",
        Group::Math,
    ),
    f(
        "IF",
        "IF(",
        "IF(condition, then, otherwise=None)",
        "then if condition is true, else otherwise. Both are worked out first.",
        Group::Logic,
    ),
    f(
        "LOOKUP",
        "LOOKUP(",
        "LOOKUP(key, items, key_field, result_field)",
        "A field of the first item whose key field equals key; none gives #REF!.",
        Group::Lookup,
    ),
    f(
        "FILTER",
        "FILTER(",
        "FILTER(items, condition)",
        "The items for which a function of one item, or a list of flags, is true.",
        Group::Lookup,
    ),
    f(
        "SORT",
        "SORT(",
        "SORT(items, key=None, reverse=False)",
        "The items in order, as a list. key is a function or a field name.",
        Group::Lookup,
    ),
    f(
        "UNIQUE",
        "UNIQUE(",
        "UNIQUE(items)",
        "The items without repeats, each at its first place.",
        Group::Lookup,
    ),
    f(
        "CONCAT",
        "CONCAT(",
        "CONCAT(*arrays)",
        "One list with the items of each array in turn; not text joining.",
        Group::Lookup,
    ),
    f(
        "TEXTJOIN",
        "TEXTJOIN(",
        "TEXTJOIN(delimiter, *values)",
        "The text elements joined with the delimiter.",
        Group::Text,
    ),
    f(
        "TEXT",
        "TEXT(",
        "TEXT(value, spec='')",
        "A value as text by a Python format spec, as in TEXT(3.14159, '.2f').",
        Group::Text,
    ),
    f(
        "NA",
        "NA()",
        "NA()",
        "Gives #N/A, for a value that is not available.",
        Group::Errors,
    ),
];

/// Every Excel-named function, with its description.
pub fn functions() -> &'static [Function] {
    FUNCTIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_match_the_core_crate() {
        assert_eq!(VERSION, cliotree_core::VERSION);
    }

    #[test]
    fn every_name_is_described_once_and_defined_in_the_source() {
        let mut described: Vec<_> = FUNCTIONS.iter().map(|d| d.name).collect();
        let mut names: Vec<_> = NAMES.to_vec();
        described.sort();
        names.sort();
        assert_eq!(described, names);
        for name in NAMES {
            assert!(
                PYTHON.contains(&format!("def {name}(")),
                "{name} is not defined"
            );
        }
        assert!(PYTHON.contains(&format!("class {NOT_AVAILABLE}")));
    }

    #[test]
    fn insert_text_has_the_bracket() {
        let sum = functions().iter().find(|f| f.name == "SUM").unwrap();
        assert_eq!(sum.insert, "SUM(");
        let na = functions().iter().find(|f| f.name == "NA").unwrap();
        assert_eq!(na.insert, "NA()");
    }
}
