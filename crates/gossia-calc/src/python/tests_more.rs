//! More tests of the Python engine: language, navigation, dependencies,
//! errors, conversion, and what the interpreter can reach.

use gossia_core::{ErrorKind, Value, json, jsonschema};

use super::PythonEngine;
use super::tests::{Book, p};

fn ints(value: Option<Value>) -> Vec<i64> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .map(|n| match n.value() {
                Value::Int(i) => *i,
                other => panic!("{other:?}"),
            })
            .collect(),
        other => panic!("not an array: {other:?}"),
    }
}

// ------------------------------------------------------------ the language

/// Python engine: formulas are Python expressions, with lambdas, comprehensions,
/// f-strings and the standard library modules that work.
#[test]
fn formulas_use_python_expressions_and_the_exposed_modules() {
    let mut b = Book::new();
    b.formula("text", "f\"{S.tags[0].upper()}-{S.a + S.b:03d}\"");
    b.formula("x", "math.sqrt(16) + len(json.dumps([1, 2]))");
    b.formula("c", "sum(n * 2 for n in range(5) if n % 2 == 0)");
    b.formula("name", "datetime.date(2026, 10, 23).isoformat()");
    b.formula("note", "re.sub(r'[aeiou]', '', 'education')");
    b.run();
    assert_eq!(b.get("text"), Some(Value::Str("X-003".into())));
    assert_eq!(b.get("x"), Some(Value::Float(10.0)));
    assert_eq!(b.get("c"), Some(Value::Int(12)));
    assert_eq!(b.get("name"), Some(Value::Str("2026-10-23".into())));
    assert_eq!(b.get("note"), Some(Value::Str("dctn".into())));
}

/// `true`, `false` and `null` are `True`, `False` and `None` (owner, 2026-11-12), so JSON
/// pasted into a formula works.
#[test]
fn json_true_false_and_null_are_python_values() {
    let mut b = Book::new();
    b.formula(
        "text",
        "json.dumps({\"on\": true, \"off\": false, \"n\": [1, null]})",
    );
    b.formula(
        "c",
        "(1 if true is True else 0) + (2 if false is False else 0) + (4 if null is None else 0)",
    );
    b.run();
    assert_eq!(
        b.get("text"),
        Some(Value::Str(
            "{\"on\": true, \"off\": false, \"n\": [1, null]}".into()
        ))
    );
    assert_eq!(b.get("c"), Some(Value::Int(7)));
}

#[test]
fn the_other_exposed_modules_work() {
    let mut b = Book::new();
    b.formula(
        "text",
        "str(decimal.Decimal('1.10') + decimal.Decimal('2.20')) + ',' \
         + str(fractions.Fraction(1, 3) * 3) + ',' \
         + str(collections.Counter('aab')['a']) + ',' \
         + str(list(itertools.islice(itertools.count(5), 3))) + ',' \
         + str(functools.reduce(lambda p, q: p * q, [1, 2, 3, 4])) + ',' \
         + textwrap.shorten('a b c d', width=5)",
    );
    b.run();
    assert_eq!(
        b.get("text"),
        Some(Value::Str("3.30,1,2,[5, 6, 7],24,[...]".into())),
        "{:?}",
        b.get("text")
    );
}

/// `statistics` (owner, 2026-11-10): the vendored `random` is patched to
/// import without `os`, since `statistics` imports it.
#[test]
fn the_statistics_module_works() {
    let mut b = Book::new();
    b.formula(
        "text",
        "str(statistics.median([5, 1, 3])) + ',' \
         + str(statistics.mean([1, 2, 3, 4])) + ',' \
         + str(round(statistics.stdev([2, 4, 4, 4, 5, 5, 7, 9]), 4)) + ',' \
         + str(statistics.quantiles(range(1, 9), n=4)) + ',' \
         + str(statistics.linear_regression([1, 2, 3], [3, 5, 7]).slope) + ',' \
         + str(round(statistics.NormalDist(100, 15).cdf(115), 4))",
    );
    b.formula("name", "statistics.mean([])");
    b.run();
    assert_eq!(
        b.get("text"),
        Some(Value::Str(
            "3,2.5,2.1381,[2.25, 4.5, 6.75],2.0,0.8413".into()
        )),
        "{}",
        b.message("text")
    );
    assert!(
        b.message("name").contains("StatisticsError"),
        "{}",
        b.message("name")
    );
}

// ------------------------------------------------------------- navigation

/// Formulas, "Name resolution": `Orders.items[2].price`.
#[test]
fn proxies_move_through_objects_arrays_and_maps() {
    let mut b = Book::new();
    b.formula("x", "S.items[2].price + S.items[-3].price");
    b.formula("a", "S.by['two'] + S.nums[1] + len(S.items) + len(S.by)");
    b.formula("name", "S.tags[0] + S.tags[-1] + S.items[2].tag");
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(2.0)));
    assert_eq!(b.get("a"), Some(Value::Int(2 + 20 + 3 + 2)));
    assert_eq!(b.get("name"), Some(Value::Str("xysale".into())));
}

/// Formulas: `Orders.items.price` is the prices of all the items.
#[test]
fn an_attribute_of_an_array_is_that_field_of_every_item() {
    let mut b = Book::new();
    b.formula("x", "sum(S.items.price)");
    b.formula("a", "sum(S.items.qty)");
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(4.0)));
    assert_eq!(b.get("a"), Some(Value::Int(9)));
}

#[test]
fn iteration_slices_and_keys() {
    let mut b = Book::new();
    b.formula("a", "sum(i.qty for i in S.items if i.price > 1)");
    b.formula(
        "name",
        "','.join(S.tags[:1]) + ','.join(k for k in S.by) + str(len(S.nums[1:]))",
    );
    b.formula("b", "len([n for n in S.nums if n > 10])");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(5)));
    assert_eq!(b.get("name"), Some(Value::Str("xone,two2".into())));
    assert_eq!(b.get("b"), Some(Value::Int(2)));
}

/// Formulas: a field that is declared but absent reads as `None`; one that is
/// not defined, or an index past the end, is `#REF!`.
#[test]
fn an_absent_optional_field_is_none_and_anything_else_missing_is_a_ref_error() {
    let mut b = Book::new();
    b.formula("text", "'none' if S.note is None else S.note");
    b.formula("a", "S.nothing");
    b.formula("b", "S.nums[7]");
    b.formula("c", "S.by['three']");
    b.run();
    assert_eq!(b.get("text"), Some(Value::Str("none".into())));
    assert_eq!(b.error("a"), Some(ErrorKind::Ref));
    assert_eq!(b.error("b"), Some(ErrorKind::Ref));
    assert_eq!(b.error("c"), Some(ErrorKind::Ref));
    assert!(b.message("a").contains("S.nothing"), "{}", b.message("a"));
}

#[test]
fn formulas_read_other_tabs() {
    let mut b = Book::new();
    let spec = jsonschema::lower_str(
        r#"{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}"#,
    )
    .unwrap();
    let data = json::parse(r#"{"n": 40}"#).unwrap().node;
    b.calc.add_sheet("Other", spec, data).unwrap();
    b.formula("a", "Other.n + 2");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(42)));
    b.calc.set_value("Other", &p("n"), Value::Int(1)).unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(3)));
}

#[test]
fn a_tab_that_does_not_exist_is_a_name_error_until_it_does() {
    let mut b = Book::new();
    b.formula("a", "Later.n");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Name));
    let spec = jsonschema::lower_str(
        r#"{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}"#,
    )
    .unwrap();
    b.calc
        .add_sheet("Later", spec, json::parse(r#"{"n": 5}"#).unwrap().node)
        .unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(5)));
}

/// Formulas, "Relative addressing": `parent` and `root`.
#[test]
fn parent_and_root_give_relative_addressing() {
    let mut b = Book::new();
    b.formula("items[0].total", "parent.qty * parent.price");
    b.formula("items[1].total", "parent.qty * parent.price");
    b.formula("a", "root.nums[2] + len(root.tags)");
    b.run();
    assert_eq!(b.get("items[0].total"), Some(Value::Float(3.0)));
    assert_eq!(b.get("items[1].total"), Some(Value::Float(6.0)));
    assert_eq!(b.get("a"), Some(Value::Int(32)));
}

// ------------------------------------------- what a formula depends on

/// A row formula can read its siblings and an aggregate can read the rows,
/// without anything being circular, and an edit runs only what it touches.
#[test]
fn rows_and_totals_calculate_in_order_without_cycles() {
    let mut b = Book::new();
    b.formula("sum", "sum(i.total for i in S.items)");
    for i in 0..3 {
        b.formula(
            &format!("items[{i}].total"),
            &format!("S.items[{i}].qty * S.items[{i}].price"),
        );
    }
    b.run();
    assert_eq!(b.get("sum"), Some(Value::Float(3.0 + 6.0 + 2.0)));
    assert_eq!(b.get("items[2].total"), Some(Value::Float(2.0)));

    let before = b.engine.evaluations();
    b.set("items[1].qty", Value::Int(10));
    b.run();
    assert_eq!(b.get("sum"), Some(Value::Float(3.0 + 20.0 + 2.0)));
    assert_eq!(b.engine.evaluations() - before, 2, "one row and the total");
}

#[test]
fn a_formula_that_counts_items_does_not_depend_on_their_values() {
    let mut b = Book::new();
    b.formula("a", "len(S.nums)");
    b.run();
    let before = b.engine.evaluations();
    b.set("nums[1]", Value::Int(99));
    assert_eq!(b.calc.pending(), 0);
    b.calc
        .insert_item("S", &p("nums"), 0, Value::Int(1))
        .unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(4)));
    assert_eq!(b.engine.evaluations() - before, 1);
}

#[test]
fn reading_one_field_does_not_depend_on_the_others() {
    let mut b = Book::new();
    b.formula("a", "S.items[0].qty + 1");
    b.run();
    b.set("items[0].price", Value::Float(9.0));
    b.set("items[2].qty", Value::Int(9));
    assert_eq!(b.calc.pending(), 0);
    b.set("items[0].qty", Value::Int(7));
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(8)));
}

/// Returning a whole array reads all of it.
#[test]
fn returning_a_proxy_reads_all_of_it() {
    let mut b = Book::new();
    b.formula("copy", "S.nums");
    b.run();
    assert_eq!(ints(b.get("copy")), [10, 20, 30]);
    b.set("nums[1]", Value::Int(21));
    b.run();
    assert_eq!(ints(b.get("copy")), [10, 21, 30]);
}

// ----------------------------------------------------------------- errors

/// Semantics table, "Errors": each Python exception maps to an error kind.
#[test]
fn python_exceptions_map_to_error_kinds() {
    let mut b = Book::new();
    for (path, text) in [
        ("a", "1 // 0"),
        ("b", "{}['k']"),
        ("c", "[][0]"),
        ("x", "float('x') + 1.5"),
        ("name", "1 + 'a'"),
        ("text", "undefined_name"),
        ("note", "(1).foo"),
        ("sum", "sorted([1, 'a'])[0]"),
    ] {
        b.formula(path, text);
    }
    b.run();
    for (path, kind) in [
        ("a", ErrorKind::DivZero),
        ("b", ErrorKind::Ref),
        ("c", ErrorKind::Ref),
        ("x", ErrorKind::Value),
        ("name", ErrorKind::Type),
        ("text", ErrorKind::Name),
        ("note", ErrorKind::Ref),
        ("sum", ErrorKind::Type),
    ] {
        assert_eq!(b.error(path), Some(kind), "{path}: {:?}", b.get(path));
    }
}

#[test]
fn an_error_says_what_went_wrong_and_keeps_the_traceback() {
    let mut b = Book::new();
    b.formula("a", "1 // 0");
    b.run();
    assert!(
        b.message("a").contains("ZeroDivisionError"),
        "{}",
        b.message("a")
    );
    let cell = b.calc.cell("S", &p("a")).unwrap();
    let trace = cell.traceback.expect("a traceback");
    assert!(trace.contains("Traceback"), "{trace}");
}

#[test]
fn an_unusual_exception_is_a_python_error() {
    let mut b = Book::new();
    b.formula("a", "next(iter([]))");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::PyErr));
}

/// Errors inside Python: using an error raises it again with the same kind,
/// and the origin names the node where it started.
#[test]
fn using_an_error_raises_it_with_the_same_kind() {
    let mut b = Book::new();
    b.formula("a", "1 // 0");
    b.formula("b", "S.a + 1");
    b.formula("c", "abs(S.b)");
    b.run();
    assert_eq!(b.error("b"), Some(ErrorKind::DivZero));
    assert_eq!(b.error("c"), Some(ErrorKind::DivZero));
    let Some(Value::Error(e)) = b.get("c") else {
        panic!("no error");
    };
    assert_eq!(e.origin, Some(p("a")), "the origin is where it started");
}

#[test]
fn moving_through_an_error_gives_the_same_error_and_it_has_attributes() {
    let mut b = Book::new();
    b.formula("items[1].total", "1 // 0");
    b.formula(
        "text",
        "S.items[1].total.kind + ' ' + S.items[1].total.path",
    );
    b.formula("x", "S.items[1].total.anything[0].more()");
    b.run();
    assert_eq!(
        b.get("text"),
        Some(Value::Str("#DIV/0! S.items[1].total".into()))
    );
    assert_eq!(
        b.error("x"),
        Some(ErrorKind::DivZero),
        "returning it puts it in the node"
    );
}

#[test]
fn every_way_of_using_an_error_raises() {
    let mut b = Book::new();
    b.formula("a", "1 // 0");
    for use_it in [
        "S.a * 2",
        "-S.a",
        "S.a < 1",
        "bool(S.a)",
        "len(S.a)",
        "str(S.a)",
        "f'{S.a}'",
        "int(S.a)",
        "float(S.a)",
        "list(S.a)",
        "sum([S.a])",
        "hash(S.a)",
    ] {
        b.formula("x", &format!("1.0 if ({use_it}) is None else 2.0"));
        b.run();
        assert_eq!(b.error("x"), Some(ErrorKind::DivZero), "{use_it}");
    }
}

/// Errors inside Python: `IFERROR`, `ISERROR`, and `VALID`.
#[test]
fn iferror_iserror_and_valid_handle_errors() {
    let mut b = Book::new();
    b.formula("items[0].total", "1 // 0");
    b.formula("items[1].total", "2.5");
    b.formula("items[2].total", "4.0");
    b.formula("a", "IFERROR(S.items[0].total, 7)");
    b.formula("b", "IFERROR(lambda: S.items[0].total / 2, 8)");
    b.formula("c", "IFERROR(lambda: 1 // 0, 9)");
    b.formula("flag", "ISERROR(S.items[0].total)");
    b.formula("x", "sum(VALID(S.items.total))");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(7)));
    assert_eq!(b.get("b"), Some(Value::Int(8)));
    assert_eq!(b.get("c"), Some(Value::Int(9)));
    assert_eq!(b.get("flag"), Some(Value::Bool(true)));
    assert_eq!(b.get("x"), Some(Value::Float(6.5)));
    b.formula("sum", "sum(S.items.total)");
    b.run();
    assert_eq!(
        b.error("sum"),
        Some(ErrorKind::DivZero),
        "a default aggregation propagates"
    );
    b.formula("flag", "ISERROR(lambda: S.items[1].total + 1)");
    b.run();
    assert_eq!(b.get("flag"), Some(Value::Bool(false)));
}

#[test]
fn an_error_value_has_kind_message_and_path() {
    let mut b = Book::new();
    b.formula("a", "{}['k']");
    b.formula("text", "S.a.message + '|' + S.a.kind");
    b.run();
    let Some(Value::Str(text)) = b.get("text") else {
        panic!("no text: {:?}", b.get("text"));
    };
    assert!(
        text.contains("KeyError") && text.ends_with("|#REF!"),
        "{text}"
    );
}

#[test]
fn a_formula_beside_a_cycle_that_handles_the_error_is_not_in_it() {
    let mut b = Book::new();
    b.formula("a", "S.b + 1");
    b.formula("b", "S.a + 1");
    b.formula("c", "IFERROR(S.a, 5)");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Circular));
    assert_eq!(b.error("b"), Some(ErrorKind::Circular));
    assert_eq!(b.get("c"), Some(Value::Int(5)));
}

// ------------------------------------------------------ result conversion

/// Scalar types: conversion is strict.
#[test]
fn results_are_converted_strictly() {
    let mut b = Book::new();
    b.formula("a", "True");
    b.formula("b", "3.0");
    b.formula("c", "10 ** 30");
    b.formula("x", "float('nan')");
    b.formula("name", "b'bytes'");
    b.formula("flag", "1");
    b.formula("sum", "2");
    b.formula("text", "None");
    b.run();
    assert_eq!(
        b.error("a"),
        Some(ErrorKind::Type),
        "a bool is not an integer"
    );
    assert_eq!(
        b.error("b"),
        Some(ErrorKind::Type),
        "a float is not an integer"
    );
    assert_eq!(b.error("c"), Some(ErrorKind::Value), "beyond 64 bits");
    assert_eq!(b.error("x"), Some(ErrorKind::Value), "NaN");
    // Bytes are read as the node: text, as UTF-8 (DESIGN.md, "Raw memory").
    assert_eq!(b.get("name"), Some(Value::Str("bytes".into())));
    assert_eq!(
        b.error("flag"),
        Some(ErrorKind::Type),
        "an int is not a boolean"
    );
    assert_eq!(b.get("sum"), Some(Value::Int(2)), "an int is a number");
    assert_eq!(
        b.error("text"),
        Some(ErrorKind::Type),
        "None for a required node"
    );
}

#[test]
fn lists_tuples_and_dicts_become_arrays_and_objects() {
    let mut b = Book::new();
    b.formula("nums", "[n * 10 for n in range(1, 4)]");
    b.formula("tags", "('a', 'b')");
    b.formula("items", "[{'qty': i, 'price': i * 1.5} for i in range(2)]");
    b.formula("by", "{'p': 1, 'q': 2}");
    b.run();
    assert_eq!(b.get("nums[2]"), Some(Value::Int(30)));
    assert_eq!(b.get("tags[1]"), Some(Value::Str("b".into())));
    assert_eq!(b.get("items[1].price"), Some(Value::Float(1.5)));
    assert_eq!(b.get("by.q"), Some(Value::Int(2)));
}

#[test]
fn an_object_result_must_match_the_schema_exactly() {
    let mut b = Book::new();
    b.formula("items", "[{'qty': 1, 'price': 1.0, 'colour': 'red'}]");
    b.run();
    assert_eq!(b.error("items"), Some(ErrorKind::Type));
    assert!(
        b.message("items").contains("colour"),
        "{}",
        b.message("items")
    );
    b.formula("items", "[{'qty': 1}]");
    b.run();
    assert_eq!(
        b.error("items"),
        Some(ErrorKind::Type),
        "a required field is missing"
    );
}

#[test]
fn none_in_an_object_leaves_the_field_out() {
    let mut b = Book::new();
    b.formula("items", "[{'qty': 1, 'price': 1.0, 'tag': None}]");
    b.run();
    assert_eq!(b.get("items[0].qty"), Some(Value::Int(1)));
    assert_eq!(b.get("items[0].tag"), None);
}

#[test]
fn an_optional_node_takes_none_as_no_value() {
    let mut b = Book::new();
    b.set("note", Value::Str("old".into()));
    b.formula("note", "None");
    b.run();
    assert_eq!(b.get("note"), None);
    b.formula("note", "'new'");
    b.run();
    assert_eq!(b.get("note"), Some(Value::Str("new".into())));
}

/// Scalar types: a date node reads in Python as a date, and a date is not a
/// string for a string node.
#[test]
fn dates_are_python_dates_when_the_data_has_them() {
    let mut b = Book::new();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
    b.set("text", Value::Date(date));
    b.formula("name", "S.text.isoformat() + str(S.text.weekday())");
    b.run();
    assert_eq!(b.get("name"), Some(Value::Str("2026-10-234".into())));
    b.formula("text", "datetime.date(2026, 10, 23)");
    b.run();
    assert_eq!(b.error("text"), Some(ErrorKind::Type));
}

#[test]
fn datetimes_keep_their_offset_and_naive_ones_stay_naive() {
    use chrono::{FixedOffset, NaiveDate, TimeZone};
    use gossia_core::DateTimeValue;
    let mut b = Book::new();
    let naive = NaiveDate::from_ymd_opt(2026, 10, 23)
        .unwrap()
        .and_hms_micro_opt(14, 30, 5, 250)
        .unwrap();
    let zoned = FixedOffset::east_opt(2 * 3600)
        .unwrap()
        .from_local_datetime(&naive)
        .single()
        .unwrap();
    b.set("text", Value::DateTime(DateTimeValue::Naive(naive)));
    b.set("name", Value::DateTime(DateTimeValue::Zoned(zoned)));
    b.formula(
        "note",
        "S.text.isoformat() + '|' + S.name.isoformat() + '|' + str(S.name.utcoffset().total_seconds())",
    );
    b.run();
    assert_eq!(
        b.get("note"),
        Some(Value::Str(
            "2026-10-23T14:30:05.000250|2026-10-23T14:30:05.000250+02:00|7200.0".into()
        ))
    );
}

// --------------------------------------------------------------- sandbox

/// Python engine: the builtins that reach outside are not there, and
/// double-underscore access is rejected when a formula is entered.
#[test]
fn builtins_that_reach_outside_are_gone() {
    let mut b = Book::new();
    for text in [
        "open('x')",
        "exec('1')",
        "eval('1')",
        "compile('1', 'x', 'eval')",
        "input()",
        "globals()",
        "setattr(S, 'x', 1)",
    ] {
        b.formula("a", text);
        b.run();
        assert_eq!(b.error("a"), Some(ErrorKind::Name), "{text}");
    }
    b.formula("text", "().__class__.__name__");
    assert_eq!(b.error("text"), Some(ErrorKind::Syntax));
    assert!(b.message("text").contains("double-underscore"));
}

/// Python engine: the probes from the spike, kept as permanent tests. Built by
/// hand, past the syntax check, to show what the compiled-in features rule
/// out and not what the check rejects.
#[test]
fn the_interpreter_has_no_way_to_the_machine_even_through_getattr() {
    let engine = PythonEngine::new();
    let importer = "[c for c in getattr(getattr(getattr((), '__class__'), '__base__'), \
                    '__subclasses__')() if c.__name__ == 'BuiltinImporter'][0]";
    let probes: Vec<String> = ["os", "posix", "_socket", "subprocess", "nt"]
        .iter()
        .map(|m| format!("{importer}.find_spec('{m}')"))
        .collect();
    let refs: Vec<&str> = probes.iter().map(String::as_str).collect();
    for (module, outcome) in ["os", "posix", "_socket", "subprocess", "nt"]
        .iter()
        .zip(engine.probe(&refs))
    {
        assert_eq!(outcome, "None", "{module} can be found");
    }
    for name in [
        "os",
        "posix",
        "nt",
        "socket",
        "_socket",
        "subprocess",
        "ctypes",
        "sqlite3",
        "threading",
        "pathlib",
        "shutil",
        "urllib.request",
        "http.client",
        "select",
        "tempfile",
        "glob",
        "signal",
    ] {
        assert!(!engine.can_import(name), "{name} must not import");
    }
    let opened = engine.probe(&["getattr(__import__('io'), 'open')('Cargo.toml')"]);
    assert!(opened[0].contains("Error"), "{opened:?}");
}

#[test]
fn range_refuses_a_huge_sequence_before_it_starts() {
    let mut b = Book::new();
    b.formula("a", "sum(range(10 ** 10))");
    b.formula("b", "sum(range(10 ** 30))");
    b.formula("c", "sum(range(5))");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Value));
    assert!(
        b.message("a").contains("Maximum range length"),
        "{}",
        b.message("a")
    );
    assert_eq!(b.error("b"), Some(ErrorKind::Value));
    assert_eq!(b.get("c"), Some(Value::Int(10)));
    b.engine.set_max_range(3);
    b.calc.recalc_all();
    b.run();
    assert_eq!(b.error("c"), Some(ErrorKind::Value));
}

/// `PLAIN` turns a node into ordinary lists and dicts.
#[test]
fn plain_turns_nodes_into_lists_and_dicts() {
    let mut b = Book::new();
    b.formula("text", "json.dumps(PLAIN(S.items[2]), sort_keys=True)");
    b.formula("name", "json.dumps(PLAIN(S.by), sort_keys=True)");
    b.run();
    assert_eq!(
        b.get("text"),
        Some(Value::Str(
            r#"{"price": 0.5, "qty": 4, "tag": "sale"}"#.into()
        ))
    );
    assert_eq!(
        b.get("name"),
        Some(Value::Str(r#"{"one": 1, "two": 2}"#.into()))
    );
}

// --------------------------------------------------------------- JSONPATH

/// Formulas, "Name resolution": `JSONPATH` for queries that do not fit member
/// syntax. Matches come back as the values and proxies a formula would get
/// by walking there.
#[test]
fn jsonpath_finds_nodes_by_query() {
    let mut b = Book::new();
    b.formula("x", "sum(JSONPATH('$.items[?@.qty > 2].price', D))");
    b.formula("a", "len(JSONPATH('$..price', D))");
    b.formula("name", "JSONPATH('$.items[?@.tag == \"sale\"]', D)[0].tag");
    b.formula(
        "b",
        "JSONPATH('$.nums[-1]', D)[0] + JSONPATH('$.by.two', D)[0]",
    );
    b.formula("copy", "JSONPATH('$.nums[0:2]', D)");
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(2.5)));
    assert_eq!(b.get("a"), Some(Value::Int(3)));
    assert_eq!(b.get("name"), Some(Value::Str("sale".into())));
    assert_eq!(b.get("b"), Some(Value::Int(32)));
    assert_eq!(ints(b.get("copy")), [10, 20]);
}

#[test]
fn jsonpath_depends_on_everything_it_searched() {
    let mut b = Book::new();
    b.formula("a", "len(JSONPATH('$.items[?@.qty > 2]', D))");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(2)));
    b.calc
        .set_value("D", &p("items[0].qty"), Value::Int(5))
        .unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(3)));
}

#[test]
fn jsonpath_queries_can_be_wrong() {
    let mut b = Book::new();
    b.formula("a", "len(JSONPATH('items', D))");
    b.formula("b", "len(JSONPATH('$.nums', 5))");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Value));
    assert!(b.message("a").contains("not valid"), "{}", b.message("a"));
    assert_eq!(b.error("b"), Some(ErrorKind::Type));
}
