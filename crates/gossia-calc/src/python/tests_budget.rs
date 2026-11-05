//! The time limit on a formula (DESIGN.md, "Trust model", "Resource limits",
//! and "Python engine", "Cancelling a running formula").

use std::time::{Duration, Instant};

use gossia_core::{ErrorKind, Value};

use super::tests::Book;

/// A loop that never ends, made of Python code (so it can be interrupted).
const FOREVER: &str = "sum(1 for _ in iter(int, 1))";

#[test]
fn a_formula_that_runs_too_long_is_stopped_with_timeout() {
    let mut b = Book::new();
    b.engine.set_time_limit(Some(Duration::from_millis(300)));
    b.formula("a", FOREVER);
    let started = Instant::now();
    b.run();
    let took = started.elapsed();
    assert_eq!(b.error("a"), Some(ErrorKind::Timeout));
    assert!(took >= Duration::from_millis(250), "{took:?}");
    assert!(took < Duration::from_secs(3), "stopped promptly: {took:?}");
}

#[test]
fn the_interpreter_is_reusable_after_a_timeout() {
    let mut b = Book::new();
    b.engine.set_time_limit(Some(Duration::from_millis(200)));
    b.formula("a", FOREVER);
    b.formula("b", "S.nums[0] + 1");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Timeout));
    assert_eq!(b.get("b"), Some(Value::Int(11)));
    // And a later edit runs normally.
    b.formula("a", "42");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(42)));
}

/// A formula cannot catch its own timeout, and `IFERROR` does not hide it.
#[test]
fn a_timeout_cannot_be_caught_in_the_formula() {
    let mut b = Book::new();
    b.engine.set_time_limit(Some(Duration::from_millis(200)));
    b.formula("a", &format!("IFERROR(lambda: {FOREVER}, 5)"));
    b.formula("b", "S.a + 1");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Timeout));
    assert_eq!(
        b.error("b"),
        Some(ErrorKind::Timeout),
        "dependents see the error"
    );
}

#[test]
fn quick_formulas_are_not_disturbed_by_the_limit() {
    let mut b = Book::new();
    b.engine.set_time_limit(Some(Duration::from_millis(500)));
    for i in 0..50 {
        b.formula("a", &format!("S.b + {i}"));
        b.run();
        assert_eq!(b.get("a"), Some(Value::Int(2 + i)));
    }
}

#[test]
fn without_a_limit_a_formula_runs_as_long_as_it_takes() {
    let mut b = Book::new();
    b.engine.set_time_limit(None);
    b.formula("a", "sum(1 for _ in range(200000))");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(200_000)));
}
