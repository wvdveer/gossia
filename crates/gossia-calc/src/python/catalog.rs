//! The names a formula can use, for the Insert Function dialog and for name
//! completion (DESIGN.md, "Formulas", "Insert Function" and "Name
//! completion"). The Excel-named functions are described in `gossia-std`;
//! the rest are here.

use gossia_i18n::tr_noop;
pub use gossia_std::{Function, Group};

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

/// Everything a formula can call or use by name, without a workbook module:
/// the Excel-named functions first, then the rest.
pub fn catalog() -> &'static [Function] {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<Function>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = gossia_std::functions().to_vec();
        all.extend_from_slice(others());
        all
    })
}

fn others() -> &'static [Function] {
    use Group::*;
    const ALL: &[Function] = &[
        f(
            "IFERROR",
            "IFERROR(",
            "IFERROR(value, fallback)",
            tr_noop!("value, or fallback if value is an error. A function is called inside a try."),
            Errors,
        ),
        f(
            "ISERROR",
            "ISERROR(",
            "ISERROR(value)",
            tr_noop!("Whether value is an error. A function is called inside a try."),
            Errors,
        ),
        f(
            "VALID",
            "VALID(",
            "VALID(iterable)",
            tr_noop!("The elements that are not errors, in order."),
            Errors,
        ),
        f(
            "PLAIN",
            "PLAIN(",
            "PLAIN(node)",
            tr_noop!("A copy of a node as plain lists, dicts and values; reads all of it."),
            Data,
        ),
        f(
            "JSONPATH",
            "JSONPATH(",
            "JSONPATH(query, node)",
            tr_noop!("The nodes of a tab or node that match a JSONPath query, as a list."),
            Data,
        ),
        f(
            "XMLPATH",
            "XMLPATH(",
            "XMLPATH(query, node)",
            tr_noop!("The nodes of a tab or node that match an XPath query, as a list."),
            Data,
        ),
        f(
            "raw",
            "raw(",
            "raw(node)",
            tr_noop!("The bytes of a node, as its tab holds them: RAW."),
            Data,
        ),
        f(
            "abs",
            "abs(",
            "abs(x)",
            tr_noop!("The absolute value of a number."),
            Python,
        ),
        f(
            "all",
            "all(",
            "all(iterable)",
            tr_noop!("True if every element is true."),
            Python,
        ),
        f(
            "any",
            "any(",
            "any(iterable)",
            tr_noop!("True if some element is true."),
            Python,
        ),
        f(
            "bool",
            "bool(",
            "bool(x)",
            tr_noop!("Whether x is true in Python's sense."),
            Python,
        ),
        f(
            "divmod",
            "divmod(",
            "divmod(a, b)",
            tr_noop!("The quotient and remainder of a division."),
            Python,
        ),
        f(
            "enumerate",
            "enumerate(",
            "enumerate(iterable, start=0)",
            tr_noop!("Pairs of a count and an element."),
            Python,
        ),
        f(
            "filter",
            "filter(",
            "filter(function, iterable)",
            tr_noop!("The elements for which function is true."),
            Python,
        ),
        f(
            "float",
            "float(",
            "float(x)",
            tr_noop!("x as a floating-point number."),
            Python,
        ),
        f(
            "int",
            "int(",
            "int(x)",
            tr_noop!("x as a whole number, cutting off any fraction."),
            Python,
        ),
        f(
            "len",
            "len(",
            "len(x)",
            tr_noop!(
                "The number of items of an array, fields of an object, or characters of text."
            ),
            Python,
        ),
        f(
            "list",
            "list(",
            "list(iterable)",
            tr_noop!("A list of the elements."),
            Python,
        ),
        f(
            "map",
            "map(",
            "map(function, iterable)",
            tr_noop!("function applied to each element."),
            Python,
        ),
        f(
            "max",
            "max(",
            "max(iterable) or max(a, b, ...)",
            tr_noop!("The largest value."),
            Python,
        ),
        f(
            "min",
            "min(",
            "min(iterable) or min(a, b, ...)",
            tr_noop!("The smallest value."),
            Python,
        ),
        f(
            "pow",
            "pow(",
            "pow(base, exp)",
            tr_noop!("base to the power exp."),
            Python,
        ),
        f(
            "range",
            "range(",
            "range(stop) or range(start, stop, step)",
            tr_noop!("The whole numbers from start up to stop; refuses a very long range."),
            Python,
        ),
        f(
            "reversed",
            "reversed(",
            "reversed(sequence)",
            tr_noop!("The elements in reverse order."),
            Python,
        ),
        f(
            "round",
            "round(",
            "round(x, digits=None)",
            tr_noop!("x rounded to a number of digits, to the nearest even on ties."),
            Python,
        ),
        f(
            "set",
            "set(",
            "set(iterable)",
            tr_noop!("The distinct elements."),
            Python,
        ),
        f(
            "sorted",
            "sorted(",
            "sorted(iterable, key=None, reverse=False)",
            tr_noop!("The elements in order, as a new list."),
            Python,
        ),
        f("str", "str(", "str(x)", tr_noop!("x as text."), Python),
        f(
            "sum",
            "sum(",
            "sum(iterable, start=0)",
            tr_noop!("The sum of the elements."),
            Python,
        ),
        f(
            "tuple",
            "tuple(",
            "tuple(iterable)",
            tr_noop!("A tuple of the elements."),
            Python,
        ),
        f(
            "zip",
            "zip(",
            "zip(a, b, ...)",
            tr_noop!("Tuples of the elements taken together."),
            Python,
        ),
        f(
            "math",
            "math.",
            "math",
            tr_noop!("Mathematical functions: sqrt, floor, ceil, log, sin, pi, and more."),
            Modules,
        ),
        f(
            "datetime",
            "datetime.",
            "datetime",
            tr_noop!("Dates and times: date, datetime, timedelta, timezone."),
            Modules,
        ),
        f(
            "re",
            "re.",
            "re",
            tr_noop!("Regular expressions: search, match, sub, findall."),
            Modules,
        ),
        f(
            "json",
            "json.",
            "json",
            tr_noop!("Reading and writing JSON text: loads, dumps."),
            Modules,
        ),
        f(
            "itertools",
            "itertools.",
            "itertools",
            tr_noop!("Iterator building blocks: chain, islice, groupby, product."),
            Modules,
        ),
        f(
            "decimal",
            "decimal.",
            "decimal",
            tr_noop!("Decimal arithmetic that is exact: Decimal."),
            Modules,
        ),
        f(
            "fractions",
            "fractions.",
            "fractions",
            tr_noop!("Exact fractions: Fraction."),
            Modules,
        ),
        f(
            "collections",
            "collections.",
            "collections",
            tr_noop!("Counter, defaultdict, deque, OrderedDict."),
            Modules,
        ),
        f(
            "functools",
            "functools.",
            "functools",
            tr_noop!("reduce, partial, lru_cache."),
            Modules,
        ),
        f(
            "string",
            "string.",
            "string",
            tr_noop!("Text constants and Template."),
            Modules,
        ),
        f(
            "textwrap",
            "textwrap.",
            "textwrap",
            tr_noop!("Wrapping and shortening text: wrap, shorten, dedent."),
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
