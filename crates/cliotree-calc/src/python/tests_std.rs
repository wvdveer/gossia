//! Tests of the Excel-named functions (DESIGN.md, "Semantics: Python first,
//! Excel second"), run as formulas through the real calculation layer.

use cliotree_core::{ErrorKind, Value};

use super::tests::{Book, p};

/// The integers of an array result.
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

/// Runs `text` as the formula of `path` and returns the book.
fn eval(path: &str, text: &str) -> Book {
    let mut b = Book::new();
    b.formula(path, text);
    b.run();
    b
}

#[test]
fn every_name_the_crate_lists_is_a_function_a_formula_can_call() {
    for name in cliotree_std::NAMES {
        let b = eval("flag", &format!("callable({name})"));
        assert_eq!(b.get("flag"), Some(Value::Bool(true)), "{name}");
    }
}

// ------------------------------------------------------------- aggregates

#[test]
fn sum_opens_lists_and_arrays_one_level_and_takes_plain_numbers() {
    assert_eq!(eval("a", "SUM(S.nums)").get("a"), Some(Value::Int(60)));
    assert_eq!(
        eval("a", "SUM(S.nums, 5, [1, 2])").get("a"),
        Some(Value::Int(68))
    );
    assert_eq!(eval("a", "SUM([])").get("a"), Some(Value::Int(0)));
    assert_eq!(
        eval("x", "SUM(S.items.price)").get("x"),
        Some(Value::Float(4.0))
    );
}

#[test]
fn nothing_is_turned_into_another_kind_so_text_in_a_sum_is_a_type_error() {
    assert_eq!(eval("a", "SUM(S.tags)").error("a"), Some(ErrorKind::Type));
    assert_eq!(eval("a", "SUM('12', 1)").error("a"), Some(ErrorKind::Type));
}

#[test]
fn average_of_nothing_is_a_division_by_zero() {
    assert_eq!(
        eval("x", "AVERAGE(S.nums)").get("x"),
        Some(Value::Float(20.0))
    );
    assert_eq!(
        eval("x", "AVERAGE([])").error("x"),
        Some(ErrorKind::DivZero)
    );
}

#[test]
fn min_and_max_compare_numbers_and_text_and_take_a_default() {
    assert_eq!(eval("a", "MIN(S.nums)").get("a"), Some(Value::Int(10)));
    assert_eq!(eval("a", "MAX(S.nums, 99)").get("a"), Some(Value::Int(99)));
    assert_eq!(
        eval("name", "MAX(S.tags)").get("name"),
        Some(Value::Str("y".into()))
    );
    assert_eq!(
        eval("a", "MIN([], default=7)").get("a"),
        Some(Value::Int(7))
    );
    assert_eq!(eval("a", "MIN([])").error("a"), Some(ErrorKind::Value));
}

#[test]
fn count_counts_the_elements_that_are_not_none() {
    assert_eq!(eval("a", "COUNT(S.nums)").get("a"), Some(Value::Int(3)));
    // Only the third item has a tag.
    assert_eq!(
        eval("a", "COUNT(S.items.tag)").get("a"),
        Some(Value::Int(1))
    );
}

/// DESIGN.md, "Errors inside Python": default aggregations propagate an error,
/// they never skip a bad item.
#[test]
fn an_error_among_the_items_is_raised_by_every_aggregate_and_keeps_its_kind() {
    for call in [
        "SUM(S.items.price)",
        "AVERAGE(S.items.price)",
        "MIN(S.items.price)",
        "MAX(S.items.price)",
        "COUNT(S.items.price)",
    ] {
        let mut b = Book::new();
        b.formula("items[1].price", "1 / 0");
        b.formula("x", call);
        b.run();
        assert_eq!(b.error("x"), Some(ErrorKind::DivZero), "{call}");
    }
}

// ----------------------------------------------------------------- logic

#[test]
fn if_is_a_conditional_with_none_as_the_default() {
    assert_eq!(
        eval("name", "IF(S.flag, 'yes', 'no')").get("name"),
        Some(Value::Str("yes".into()))
    );
    assert_eq!(
        eval("name", "IF(not S.flag, 'yes', 'no')").get("name"),
        Some(Value::Str("no".into()))
    );
    // `note` is required, so a None result is refused with the usual reason.
    assert_eq!(
        eval("name", "IF(not S.flag, 'yes')").error("name"),
        Some(ErrorKind::Type)
    );
}

// ---------------------------------------------------------------- lookups

#[test]
fn vlookup_finds_a_field_of_the_first_match_and_raises_when_there_is_none() {
    assert_eq!(
        eval("x", "VLOOKUP(3, S.items, 'qty', 'price')").get("x"),
        Some(Value::Float(2.0))
    );
    assert_eq!(
        eval("x", "VLOOKUP(9, S.items, 'qty', 'price')").error("x"),
        Some(ErrorKind::Ref)
    );
}

#[test]
fn filter_takes_a_function_or_a_list_of_flags() {
    assert_eq!(
        ints(
            eval(
                "copy",
                "[i.qty for i in FILTER(S.items, lambda i: i.qty > 2)]"
            )
            .get("copy")
        ),
        [3, 4]
    );
    assert_eq!(
        ints(eval("copy", "FILTER(S.nums, [True, False, True])").get("copy")),
        [10, 30]
    );
    assert_eq!(
        eval("copy", "FILTER(S.nums, [True])").error("copy"),
        Some(ErrorKind::Value)
    );
}

#[test]
fn sort_orders_by_a_function_a_field_name_or_the_items_themselves() {
    assert_eq!(
        ints(eval("copy", "SORT(S.nums, reverse=True)").get("copy")),
        [30, 20, 10]
    );
    // By price: 0.5, 1.5, 2.0.
    assert_eq!(
        ints(eval("copy", "[i.qty for i in SORT(S.items, 'price')]").get("copy")),
        [4, 2, 3]
    );
    assert_eq!(
        ints(eval("copy", "[i.qty for i in SORT(S.items, lambda i: -i.qty)]").get("copy")),
        [4, 3, 2]
    );
}

#[test]
fn unique_keeps_the_first_of_each_and_compares_objects_by_what_is_in_them() {
    assert_eq!(
        ints(eval("copy", "UNIQUE([1, 2, 1, 3, 2])").get("copy")),
        [1, 2, 3]
    );
    assert_eq!(
        eval("a", "len(UNIQUE(S.items))").get("a"),
        Some(Value::Int(3))
    );
    assert_eq!(
        eval("a", "len(UNIQUE([S.items[0], S.items[0]]))").get("a"),
        Some(Value::Int(1))
    );
}

#[test]
fn concat_joins_arrays_and_lists_and_refuses_anything_else() {
    assert_eq!(
        ints(eval("copy", "CONCAT(S.nums, [1], D.copy)").get("copy")),
        [10, 20, 30, 1]
    );
    assert_eq!(
        eval("copy", "CONCAT(S.nums, 5)").error("copy"),
        Some(ErrorKind::Type)
    );
    assert_eq!(
        eval("copy", "CONCAT(S.by)").error("copy"),
        Some(ErrorKind::Type)
    );
}

// ------------------------------------------------------------------- text

#[test]
fn textjoin_joins_text_and_text_formats_by_a_python_spec() {
    assert_eq!(
        eval("name", "TEXTJOIN('-', S.tags)").get("name"),
        Some(Value::Str("x-y".into()))
    );
    assert_eq!(
        eval("name", "TEXTJOIN(',', S.nums)").error("name"),
        Some(ErrorKind::Type)
    );
    assert_eq!(
        eval("text", "TEXT(3.14159, '.2f')").get("text"),
        Some(Value::Str("3.14".into()))
    );
}

// --------------------------------------------------------------------- NA

#[test]
fn na_is_the_only_source_of_n_a_and_it_propagates() {
    let mut b = Book::new();
    b.formula("x", "NA()");
    b.formula("sum", "S.x + 1");
    b.run();
    assert_eq!(b.error("x"), Some(ErrorKind::Na));
    assert_eq!(b.error("sum"), Some(ErrorKind::Na));
    // And IFERROR catches it like any other error.
    b.formula("sum", "IFERROR(S.x, 0)");
    b.run();
    assert_eq!(b.get("sum"), Some(Value::Int(0)));
    let _ = p("x");
}
