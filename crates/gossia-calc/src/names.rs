//! The names a workbook may give to its tabs, modules and module functions
//! (DESIGN.md, "Names and reserved words"). Formulas, form button procedures
//! and report expressions share one flat namespace, so every name in it has to
//! be unique and mean the same thing everywhere. Nothing may shadow anything.

use gossia_i18n::tr;
use std::collections::HashSet;
use std::sync::OnceLock;

/// What a name is for, which changes what is checked and how it is worded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Tab,
    /// A form of a tab: the rules of a tab name, which is all that is
    /// checked here.
    Form,
    Module,
    /// A public function of a module.
    Function,
}

impl Subject {
    /// The kind of name, as a whole phrase ("tab name"), so that each
    /// language words it its own way.
    fn kind_of_name(self) -> &'static str {
        match self {
            Subject::Tab => tr!("tab name"),
            Subject::Form => tr!("form name"),
            Subject::Module => tr!("module name"),
            Subject::Function => tr!("function name"),
        }
    }

    fn needs_a_name(self) -> &'static str {
        match self {
            Subject::Tab => tr!("a tab needs a name"),
            Subject::Form => tr!("a form needs a name"),
            Subject::Module => tr!("a module needs a name"),
            Subject::Function => tr!("a function needs a name"),
        }
    }
}

/// Python's keywords. `class` passes `isidentifier()` but cannot be used.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Python's builtins, including those a formula does not see: they are
/// reserved all the same, so that a name never means one thing in a formula and
/// another in a module.
const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "copyright",
    "credits",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "exit",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "license",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "quit",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "Ellipsis",
    "NotImplemented",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BaseExceptionGroup",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "BytesWarning",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "EncodingWarning",
    "EnvironmentError",
    "Exception",
    "ExceptionGroup",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "IOError",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "PythonFinalizationError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "WindowsError",
    "ZeroDivisionError",
    // Newer than the Python this list was first written from; a test that lists
    // what the interpreter has found them.
    "frozendict",
    "sentinel",
    "ImportCycleError",
];

/// The special names of a formula, and the functions of the language of
/// errors and of data that are part of the engine, not of `gossia-std`.
const SPECIAL: &[&str] = &[
    "this",
    "parent",
    "root",
    // JSON's literals, the same as `True`, `False` and `None` (owner, 2026-11-12).
    "true",
    "false",
    "null",
    "ErrorValue",
    "PropagatedError",
    "IFERROR",
    "ISERROR",
    "VALID",
    "PLAIN",
    "JSONPATH",
    // Reserved ahead of the XML adapter (build step 6).
    "XMLPATH",
    // The data sources (build step 6b).
    "source",
    "sources",
    "SourceError",
    // Charts (step 12e): the decorator and the built-in chart types.
    "chart",
    "bar_chart",
    "line_chart",
    "pie_chart",
    "scatter_chart",
    "sankey_chart",
    "map_chart",
    // Gossia pyplot (0.6.0), there without an import.
    "plt",
];

/// The names reserved in 0.6.0 (reserved-names version 2), which a workbook
/// made before may use for a tab: it opens with a warning to rename the tab,
/// and until then its formulas see the tab in place of the engine's value.
pub(crate) const YIELD_TO_TABS: &[&str] = &["true", "false", "null", "plt", "map_chart"];

fn reserved_set() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set: HashSet<&'static str> = HashSet::new();
        set.extend(KEYWORDS);
        set.extend(BUILTINS);
        set.extend(SPECIAL);
        set.extend(gossia_std::NAMES);
        set.extend(crate::python::MODULES);
        set
    })
}

/// Whether the name is one the engine holds: a keyword, a builtin, a special
/// name, a built-in function or an exposed standard library module.
pub fn is_reserved(name: &str) -> bool {
    reserved_set().contains(name)
}

/// Why `name` cannot be used for `subject`, or `None` if it can. Only the name
/// is judged: whether it clashes with another tab, module or function is for
/// the workbook to say, since it knows the others.
pub fn problem(name: &str, subject: Subject) -> Option<String> {
    let kind = subject.kind_of_name();
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Some(subject.needs_a_name().to_owned());
    };
    if !(first == '_' || first.is_alphabetic()) || !chars.all(|c| c == '_' || c.is_alphanumeric()) {
        return Some(tr!(
            "{name} is not a valid {kind}: use letters, digits and underscores, \
             and do not start with a digit",
            name = name,
            kind = kind
        ));
    }
    if reserved_set().contains(name) {
        let what = if KEYWORDS.contains(&name) {
            tr!("a Python keyword")
        } else if BUILTINS.contains(&name) {
            tr!("a Python builtin")
        } else if gossia_std::NAMES.contains(&name) || SPECIAL.contains(&name) {
            tr!("the name of a built-in function or of a special name in formulas")
        } else {
            tr!("the name of a standard library module that formulas can use")
        };
        return Some(tr!(
            "{name} cannot be a {kind}: it is {what}",
            name = name,
            kind = kind,
            what = what
        ));
    }
    if name.starts_with('_') || name.starts_with("gossia") {
        return Some(tr!(
            "{name} cannot be a {kind}: names starting with an underscore or with gossia \
             are kept for internal use",
            name = name,
            kind = kind
        ));
    }
    None
}

/// The identifiers that appear in `source`, strings and comments included. It
/// finds too many, which only means a module is thought to use something it
/// does not, and recalculates a little more than it needs to.
pub(crate) fn identifiers(source: &str) -> HashSet<&str> {
    let mut found = HashSet::new();
    let mut start: Option<usize> = None;
    for (at, c) in source.char_indices() {
        let part = c == '_' || c.is_alphanumeric();
        match (start, part) {
            (None, true) => start = Some(at),
            (Some(from), false) => {
                found.insert(&source[from..at]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        found.insert(&source[from..]);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_found_wherever_they_are() {
        let found = identifiers("vat(Orders.total) + 'x_1' # note");
        for name in ["vat", "Orders", "total", "x_1", "note"] {
            assert!(found.contains(name), "{name}");
        }
        assert!(!found.contains("+"));
    }

    #[test]
    fn ordinary_names_pass_and_the_reasons_are_said() {
        assert_eq!(problem("Orders", Subject::Tab), None);
        assert_eq!(problem("Tax2", Subject::Module), None);
        assert_eq!(problem("vat", Subject::Function), None);
        for (name, word) in [
            ("", "needs a name"),
            ("2fast", "not a valid"),
            ("has space", "not a valid"),
            ("class", "keyword"),
            ("list", "builtin"),
            ("SUM", "built-in function"),
            ("this", "special name"),
            ("true", "special name"),
            ("false", "special name"),
            ("null", "special name"),
            ("plt", "special name"),
            ("math", "standard library module"),
            ("_hidden", "internal use"),
            ("gossia_x", "internal use"),
        ] {
            let why = problem(name, Subject::Tab).unwrap();
            assert!(why.contains(word), "{name}: {why}");
        }
    }

    #[test]
    fn every_function_of_the_standard_crate_is_reserved() {
        for name in gossia_std::NAMES {
            assert!(is_reserved(name), "{name}");
        }
    }

    #[test]
    fn the_reasons_are_in_the_interface_language() {
        use gossia_i18n::{Language, set_thread_language};
        // This thread's own language, so that the other tests stay in English.
        set_thread_language(Some(Language::French));
        assert_eq!(
            problem("", Subject::Module).as_deref(),
            Some("un module a besoin d'un nom")
        );
        assert_eq!(
            problem("SUM", Subject::Tab).as_deref(),
            Some(
                "SUM ne peut pas être un nom d'onglet : c'est le nom d'une fonction intégrée \
                 ou d'un nom spécial des formules"
            )
        );
        set_thread_language(Some(Language::Japanese));
        assert_eq!(
            problem("1x", Subject::Function).as_deref(),
            Some(
                "1x は関数名として無効です。文字、数字、アンダースコアを使い、数字で始めないでください"
            )
        );
        set_thread_language(None);
    }
}
