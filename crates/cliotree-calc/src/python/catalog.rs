//! The names a formula can use, for the Insert Function dialog and for name
//! completion (DESIGN.md, "Formulas", "Insert Function" and "Name
//! completion"). The Excel-named functions arrive with step 5; the workbook's
//! own modules with them.

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
    Errors,
    Data,
    Python,
    Modules,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Errors => "Errors",
            Group::Data => "Data",
            Group::Python => "Python",
            Group::Modules => "Standard library modules",
        }
    }

    pub const ALL: [Group; 4] = [Group::Errors, Group::Data, Group::Python, Group::Modules];
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

/// Everything a formula can call or use by name, without a workbook module.
pub fn catalog() -> &'static [Function] {
    use Group::*;
    const ALL: &[Function] = &[
        f(
            "IFERROR",
            "IFERROR(",
            "IFERROR(value, fallback)",
            "value, or fallback if value is an error. A function is called inside a try.",
            Errors,
        ),
        f(
            "ISERROR",
            "ISERROR(",
            "ISERROR(value)",
            "Whether value is an error. A function is called inside a try.",
            Errors,
        ),
        f(
            "VALID",
            "VALID(",
            "VALID(iterable)",
            "The elements that are not errors, in order.",
            Errors,
        ),
        f(
            "PLAIN",
            "PLAIN(",
            "PLAIN(node)",
            "A copy of a node as plain lists, dicts and values; reads all of it.",
            Data,
        ),
        f(
            "JSONPATH",
            "JSONPATH(",
            "JSONPATH(query, node)",
            "The nodes of a tab or node that match a JSONPath query, as a list.",
            Data,
        ),
        f(
            "abs",
            "abs(",
            "abs(x)",
            "The absolute value of a number.",
            Python,
        ),
        f(
            "all",
            "all(",
            "all(iterable)",
            "True if every element is true.",
            Python,
        ),
        f(
            "any",
            "any(",
            "any(iterable)",
            "True if some element is true.",
            Python,
        ),
        f(
            "bool",
            "bool(",
            "bool(x)",
            "Whether x is true in Python's sense.",
            Python,
        ),
        f(
            "divmod",
            "divmod(",
            "divmod(a, b)",
            "The quotient and remainder of a division.",
            Python,
        ),
        f(
            "enumerate",
            "enumerate(",
            "enumerate(iterable, start=0)",
            "Pairs of a count and an element.",
            Python,
        ),
        f(
            "filter",
            "filter(",
            "filter(function, iterable)",
            "The elements for which function is true.",
            Python,
        ),
        f(
            "float",
            "float(",
            "float(x)",
            "x as a floating-point number.",
            Python,
        ),
        f(
            "int",
            "int(",
            "int(x)",
            "x as a whole number, cutting off any fraction.",
            Python,
        ),
        f(
            "len",
            "len(",
            "len(x)",
            "The number of items of an array, fields of an object, or characters of text.",
            Python,
        ),
        f(
            "list",
            "list(",
            "list(iterable)",
            "A list of the elements.",
            Python,
        ),
        f(
            "map",
            "map(",
            "map(function, iterable)",
            "function applied to each element.",
            Python,
        ),
        f(
            "max",
            "max(",
            "max(iterable) or max(a, b, ...)",
            "The largest value.",
            Python,
        ),
        f(
            "min",
            "min(",
            "min(iterable) or min(a, b, ...)",
            "The smallest value.",
            Python,
        ),
        f(
            "pow",
            "pow(",
            "pow(base, exp)",
            "base to the power exp.",
            Python,
        ),
        f(
            "range",
            "range(",
            "range(stop) or range(start, stop, step)",
            "The whole numbers from start up to stop; refuses a very long range.",
            Python,
        ),
        f(
            "reversed",
            "reversed(",
            "reversed(sequence)",
            "The elements in reverse order.",
            Python,
        ),
        f(
            "round",
            "round(",
            "round(x, digits=None)",
            "x rounded to a number of digits, to the nearest even on ties.",
            Python,
        ),
        f(
            "set",
            "set(",
            "set(iterable)",
            "The distinct elements.",
            Python,
        ),
        f(
            "sorted",
            "sorted(",
            "sorted(iterable, key=None, reverse=False)",
            "The elements in order, as a new list.",
            Python,
        ),
        f("str", "str(", "str(x)", "x as text.", Python),
        f(
            "sum",
            "sum(",
            "sum(iterable, start=0)",
            "The sum of the elements.",
            Python,
        ),
        f(
            "tuple",
            "tuple(",
            "tuple(iterable)",
            "A tuple of the elements.",
            Python,
        ),
        f(
            "zip",
            "zip(",
            "zip(a, b, ...)",
            "Tuples of the elements taken together.",
            Python,
        ),
        f(
            "math",
            "math.",
            "math",
            "Mathematical functions: sqrt, floor, ceil, log, sin, pi, and more.",
            Modules,
        ),
        f(
            "datetime",
            "datetime.",
            "datetime",
            "Dates and times: date, datetime, timedelta, timezone.",
            Modules,
        ),
        f(
            "re",
            "re.",
            "re",
            "Regular expressions: search, match, sub, findall.",
            Modules,
        ),
        f(
            "json",
            "json.",
            "json",
            "Reading and writing JSON text: loads, dumps.",
            Modules,
        ),
        f(
            "itertools",
            "itertools.",
            "itertools",
            "Iterator building blocks: chain, islice, groupby, product.",
            Modules,
        ),
        f(
            "decimal",
            "decimal.",
            "decimal",
            "Decimal arithmetic that is exact: Decimal.",
            Modules,
        ),
        f(
            "fractions",
            "fractions.",
            "fractions",
            "Exact fractions: Fraction.",
            Modules,
        ),
        f(
            "collections",
            "collections.",
            "collections",
            "Counter, defaultdict, deque, OrderedDict.",
            Modules,
        ),
        f(
            "functools",
            "functools.",
            "functools",
            "reduce, partial, lru_cache.",
            Modules,
        ),
        f(
            "string",
            "string.",
            "string",
            "Text constants and Template.",
            Modules,
        ),
        f(
            "textwrap",
            "textwrap.",
            "textwrap",
            "Wrapping and shortening text: wrap, shorten, dedent.",
            Modules,
        ),
    ];
    ALL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_every_group_has_entries() {
        let all = catalog();
        let mut names: Vec<_> = all.iter().map(|f| f.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len());
        for group in Group::ALL {
            assert!(all.iter().any(|f| f.group == group), "{group:?}");
        }
        assert!(
            all.iter()
                .all(|f| !f.description.is_empty() && !f.signature.is_empty())
        );
    }

    /// The exposed modules are the ones the engine puts in a formula's names.
    #[test]
    fn the_modules_listed_are_the_ones_the_engine_exposes() {
        let listed: Vec<_> = catalog()
            .iter()
            .filter(|f| f.group == Group::Modules)
            .map(|f| f.name)
            .collect();
        assert_eq!(listed, super::super::MODULES);
    }
}
