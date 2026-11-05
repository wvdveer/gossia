//! Tests of the workbook's Python modules (DESIGN.md, "Python modules" and
//! "Names and reserved words"), run through the real calculation layer and the
//! real engine.

use std::time::Duration;

use gossia_core::{ErrorKind, Value};
use rustpython::vm::builtins::PyStr;

use super::PythonEngine;
use super::tests::{Book, p};
use crate::{EditError, ModuleStatus, names};

const TAX: &str = "def vat(x, rate=0.21):\n    return round(x * rate, 2)\n";

fn book_with(modules: &[(&str, &str)]) -> Book {
    let mut b = Book::new();
    for (name, source) in modules {
        b.calc.add_module(name, source).unwrap();
    }
    b
}

fn failed(b: &Book, name: &str) -> String {
    match b.calc.module_status(name) {
        Some(ModuleStatus::Failed(why)) => why.clone(),
        other => panic!("{name} did not fail: {other:?}"),
    }
}

// ------------------------------------------------------------- availability

/// Python modules: a module is available by name, and its public functions by
/// bare name.
#[test]
fn a_module_is_available_by_name_and_its_functions_by_bare_name() {
    let mut b = book_with(&[("Tax", TAX)]);
    b.formula("x", "Tax.vat(100)");
    b.formula("sum", "vat(50)");
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(21.0)));
    assert_eq!(b.get("sum"), Some(Value::Float(10.5)));
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Loaded));
    assert_eq!(b.calc.module_functions("Tax"), ["vat"]);
}

#[test]
fn private_names_are_not_published() {
    let mut b = book_with(&[(
        "M",
        "_secret = 5\n\ndef _hidden():\n    return 1\n\ndef shown():\n    return _hidden() + _secret\n",
    )]);
    b.formula("a", "shown()");
    b.formula("b", "_hidden()");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(6)));
    assert_eq!(b.error("b"), Some(ErrorKind::Name));
    assert_eq!(b.calc.module_functions("M"), ["shown"]);
}

/// A module's functions see the workbook the way formulas do, looked up when
/// they are called, and the reads are dependencies of the running formula.
#[test]
fn a_module_function_reads_a_tab_when_it_is_called_and_the_formula_depends_on_it() {
    let mut b = book_with(&[("Sums", "def total():\n    return sum(S.nums)\n")]);
    b.formula("a", "total()");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(60)));
    b.set("nums[0]", Value::Int(11));
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(61)));
}

#[test]
fn a_tab_added_after_the_module_loaded_is_found_when_the_function_is_called() {
    let mut b = book_with(&[("Late", "def n():\n    return len(Extra.nums)\n")]);
    b.formula("a", "n()");
    b.run();
    assert_eq!(b.error("a"), Some(ErrorKind::Name));
    let spec = b.calc.spec("D").unwrap().clone();
    let data = b.calc.data("D").unwrap().clone();
    b.calc.add_sheet("Extra", spec, data).unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(3)));
}

#[test]
fn modules_can_use_the_exposed_standard_library_and_the_builtin_functions() {
    let mut b = book_with(&[(
        "M",
        "import math\nimport collections\nfrom itertools import chain\n\n\
         def f():\n    return math.floor(2.5) + len(list(chain([1], [2]))) + SUM([1, 2])\n",
    )]);
    b.formula("a", "f()");
    b.run();
    // 2 + 2 + 3
    assert_eq!(b.get("a"), Some(Value::Int(7)));
}

// ----------------------------------------------------------------- imports

#[test]
fn modules_import_each_other_in_dependency_order_whatever_the_order_added() {
    let mut b = book_with(&[
        (
            "Top",
            "from Base import helper\n\ndef f():\n    return helper() + 1\n",
        ),
        ("Base", "def helper():\n    return 1\n"),
    ]);
    b.formula("a", "f()");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(2)));
    assert_eq!(b.calc.module_status("Top"), Some(&ModuleStatus::Loaded));
}

/// Reloading: a reload also reloads the modules that import it, so that one
/// that copied a function (`from Base import helper`) sees the new code.
#[test]
fn editing_a_module_reloads_the_modules_that_import_it() {
    let mut b = book_with(&[
        ("Base", "def helper():\n    return 1\n"),
        (
            "Top",
            "from Base import helper\n\ndef f():\n    return helper() + 1\n",
        ),
    ]);
    b.formula("a", "f()");
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(2)));
    b.calc
        .set_module_source("Base", "def helper():\n    return 10\n")
        .unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(11)));
}

/// A cycle of imports fails every module on it, naming the cycle.
#[test]
fn a_cycle_of_imports_fails_every_module_on_it_and_names_the_cycle() {
    let mut b = book_with(&[
        ("A", "import B\n\ndef fa():\n    return 1\n"),
        ("B", "import A\n\ndef fb():\n    return 2\n"),
        ("C", "def fc():\n    return 3\n"),
    ]);
    b.formula("a", "fa()");
    b.formula("b", "fc()");
    b.run();
    assert!(
        failed(&b, "A").contains("import cycle: A -> B -> A"),
        "{}",
        failed(&b, "A")
    );
    assert!(
        failed(&b, "B").contains("import cycle: B -> A -> B"),
        "{}",
        failed(&b, "B")
    );
    assert_eq!(b.error("a"), Some(ErrorKind::Name));
    assert!(
        b.message("a").contains("failed to load"),
        "{}",
        b.message("a")
    );
    // Other modules are unaffected.
    assert_eq!(b.get("b"), Some(Value::Int(3)));
    assert_eq!(b.calc.module_issues().len(), 2);
}

#[test]
fn only_the_workbook_and_the_exposed_standard_library_can_be_imported() {
    let mut b = book_with(&[
        ("Os", "import os\n"),
        ("Sock", "import socket\n"),
        ("Dotted", "import collections.abc\n"),
        ("Fine", "import json\n"),
    ]);
    b.run();
    for name in ["Os", "Sock", "Dotted"] {
        let why = failed(&b, name);
        assert!(why.contains("ImportError"), "{name}: {why}");
        assert!(why.contains("is available"), "{name}: {why}");
    }
    assert_eq!(b.calc.module_status("Fine"), Some(&ModuleStatus::Loaded));
}

#[test]
fn importing_a_module_that_failed_says_so() {
    let mut b = book_with(&[
        ("Bad", "x = 1 / 0\n"),
        ("User", "import Bad\n\ndef f():\n    return 1\n"),
    ]);
    b.run();
    assert!(
        failed(&b, "User").contains("module Bad failed to load"),
        "{}",
        failed(&b, "User")
    );
}

// ---------------------------------------------------------------- failures

/// An exception at load marks the module failed, and formulas that use its
/// functions show #NAME! with a message that points at the module.
#[test]
fn a_module_that_raises_at_load_is_failed_and_its_functions_are_not_found() {
    let mut b = book_with(&[("Bad", "def f():\n    return 1\n\nx = 1 / 0\n")]);
    b.formula("a", "f()");
    b.run();
    let why = failed(&b, "Bad");
    assert!(
        why.contains("ZeroDivisionError") && why.contains("line 4"),
        "{why}"
    );
    assert_eq!(b.error("a"), Some(ErrorKind::Name));
    assert!(
        b.message("a")
            .contains("f comes from module Bad, which failed to load"),
        "{}",
        b.message("a")
    );
    // Fixing the module fixes the formula.
    b.calc
        .set_module_source("Bad", "def f():\n    return 1\n")
        .unwrap();
    b.run();
    assert_eq!(b.get("a"), Some(Value::Int(1)));
    assert_eq!(b.calc.module_status("Bad"), Some(&ModuleStatus::Loaded));
}

#[test]
fn a_module_with_a_syntax_error_is_failed_and_listed() {
    let mut b = book_with(&[("Syn", "def f(:\n    pass\n")]);
    b.formula("a", "f()");
    b.run();
    let issues = b.calc.module_issues();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].1.contains("syntax error"), "{issues:?}");
    assert_eq!(b.error("a"), Some(ErrorKind::Name));
}

#[test]
fn double_underscore_access_in_a_module_is_refused() {
    let mut b = book_with(&[("Sneaky", "def f(x):\n    return x.__class__\n")]);
    b.run();
    assert!(b.calc.module_issues()[0].1.contains("double-underscore"));
}

#[test]
fn a_module_that_never_finishes_loading_is_stopped_by_the_time_limit() {
    let mut b = book_with(&[("Loop", "while True:\n    pass\n")]);
    b.engine.set_time_limit(Some(Duration::from_millis(300)));
    b.run();
    assert!(
        failed(&b, "Loop").contains("ran out of time"),
        "{}",
        failed(&b, "Loop")
    );
}

// ------------------------------------------------------------------ clashes

/// Names: a function that would take the place of anything is an error at
/// load, and the message names both.
#[test]
fn a_function_that_takes_a_name_already_in_use_fails_its_module_naming_both() {
    let mut b = book_with(&[
        ("P", "def vat():\n    return 1\n"),
        ("Q", "def vat():\n    return 2\n"),
        ("R", "def SUM():\n    return 3\n"),
        ("T", "def S():\n    return 4\n"),
        ("U", "def P():\n    return 5\n"),
    ]);
    b.formula("a", "vat()");
    b.run();
    assert!(
        failed(&b, "Q").contains("function vat in module Q clashes with function vat in module P"),
        "{}",
        failed(&b, "Q")
    );
    assert!(failed(&b, "R").contains("clashes with a built-in name"));
    assert!(failed(&b, "T").contains("clashes with the tab S"));
    assert!(failed(&b, "U").contains("clashes with the module P"));
    // The first one keeps the name.
    assert_eq!(b.get("a"), Some(Value::Int(1)));
    assert_eq!(b.calc.module_status("P"), Some(&ModuleStatus::Loaded));
}

#[test]
fn module_names_are_checked_when_they_are_given() {
    let mut b = book_with(&[("Tax", TAX)]);
    let refused = |b: &mut Book, name: &str| match b.calc.add_module(name, "") {
        Err(EditError::BadName(why)) => why,
        other => panic!("{name}: {other:?}"),
    };
    assert!(refused(&mut b, "SUM").contains("built-in function"));
    assert!(refused(&mut b, "math").contains("standard library"));
    assert!(refused(&mut b, "class").contains("keyword"));
    assert!(refused(&mut b, "2x").contains("not a valid"));
    assert!(refused(&mut b, "_x").contains("internal use"));
    assert!(refused(&mut b, "tax").contains("exists already"));
    assert!(refused(&mut b, "S").contains("name of a tab"));
    assert!(refused(&mut b, "s").contains("name of a tab"));
}

/// The names the engine holds, all of them: every builtin of the interpreter
/// is reserved.
#[test]
fn every_builtin_of_the_interpreter_is_a_reserved_name() {
    let engine = PythonEngine::new();
    let found: Vec<String> = engine.interp.enter(|vm| {
        vm.builtins
            .dict()
            .items_vec()
            .into_iter()
            .filter_map(|(k, _)| k.downcast_ref::<PyStr>().map(|s| s.expect_str().to_owned()))
            .collect()
    });
    assert!(found.len() > 50);
    let missing: Vec<_> = found
        .iter()
        .filter(|n| !n.starts_with('_') && !names::is_reserved(n))
        .collect();
    assert!(missing.is_empty(), "not reserved: {missing:?}");
}

// ------------------------------------------------------ editing and recalc

/// Editing a module recalculates exactly the formulas that depend on it.
#[test]
fn editing_a_module_makes_exactly_the_formulas_that_used_it_out_of_date() {
    let mut b = book_with(&[("Tax", TAX)]);
    b.formula("x", "vat(100)");
    b.formula("sum", "S.x * 2");
    b.formula("a", "S.b + 1");
    b.run();
    assert_eq!(b.calc.pending(), 0);
    b.calc
        .set_module_source(
            "Tax",
            "def vat(x, rate=0.1):\n    return round(x * rate, 2)\n",
        )
        .unwrap();
    // `x` used it and `sum` reads `x`; `a` has nothing to do with it.
    assert_eq!(b.calc.pending(), 2);
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(10.0)));
    assert_eq!(b.get("sum"), Some(Value::Float(20.0)));
    assert_eq!(b.get("a"), Some(Value::Int(3)));
}

/// A result computed under an older generation of a module is dropped.
#[test]
fn a_result_computed_from_an_older_generation_is_discarded() {
    let mut b = book_with(&[("Tax", TAX)]);
    b.formula("x", "vat(100)");
    b.run();
    b.calc
        .set_module_source("Tax", "def vat(x):\n    return 1.0\n")
        .unwrap();
    let evaluation = b.calc.snapshot().evaluate(&b.engine);
    // Edited again while that was being evaluated.
    b.calc
        .set_module_source("Tax", "def vat(x):\n    return 2.0\n")
        .unwrap();
    let applied = b.calc.apply(evaluation);
    assert!(
        applied.discarded.iter().any(|l| l.path == p("x")),
        "{applied:?}"
    );
    assert_eq!(b.get("x"), Some(Value::Float(21.0)), "still the old result");
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(2.0)));
}

#[test]
fn a_module_nobody_uses_is_still_loaded_and_its_status_comes_back() {
    let mut b = book_with(&[("Tax", TAX)]);
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Pending));
    assert_eq!(b.calc.pending_modules(), 1);
    b.run();
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Loaded));
    assert_eq!(b.calc.pending_modules(), 0);
    assert!(b.calc.module_issues().is_empty());
}

#[test]
fn editing_a_module_is_one_undoable_entry() {
    let mut b = book_with(&[("Tax", TAX)]);
    b.formula("x", "vat(100)");
    b.run();
    b.calc
        .set_module_source("Tax", "def vat(x):\n    return 7.0\n")
        .unwrap();
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(7.0)));
    assert_eq!(b.calc.undo_label(), Some("Edit module Tax"));
    b.calc.undo().unwrap();
    b.run();
    assert_eq!(b.calc.module_source("Tax"), Some(TAX));
    assert_eq!(b.get("x"), Some(Value::Float(21.0)));
    b.calc.redo().unwrap();
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(7.0)));
}

#[test]
fn removing_and_renaming_a_module_leave_names_in_formulas_alone() {
    let mut b = book_with(&[("Tax", TAX)]);
    b.formula("x", "Tax.vat(100)");
    b.formula("sum", "vat(100)");
    b.run();
    b.calc.rename_module("Tax", "Vat").unwrap();
    b.run();
    // Names in formulas are not rewritten: they stop finding it.
    assert_eq!(b.error("x"), Some(ErrorKind::Name));
    assert_eq!(
        b.get("sum"),
        Some(Value::Float(21.0)),
        "the function is still there"
    );
    b.formula("x", "Vat.vat(100)");
    b.run();
    assert_eq!(b.get("x"), Some(Value::Float(21.0)));
    b.calc.remove_module("Vat").unwrap();
    b.run();
    assert_eq!(b.error("x"), Some(ErrorKind::Name));
    assert_eq!(b.error("sum"), Some(ErrorKind::Name));
    assert_eq!(b.calc.module_names().count(), 0);
}

#[test]
fn inserting_removing_and_renaming_a_module_are_undoable() {
    let mut b = book_with(&[("A", ""), ("Tax", TAX), ("C", "")]);
    b.formula("x", "Tax.vat(100)");
    b.calc.mark_saved();
    b.run();

    b.calc.remove_module("Tax").unwrap();
    assert!(
        b.calc.is_modified(),
        "removing a module is a change to save"
    );
    assert_eq!(b.calc.undo_label(), Some("Remove module Tax"));
    b.run();
    assert_eq!(b.error("x"), Some(ErrorKind::Name));
    b.calc.undo().unwrap();
    b.run();
    assert_eq!(b.calc.module_names().collect::<Vec<_>>(), ["A", "Tax", "C"]);
    assert_eq!(b.calc.module_source("Tax"), Some(TAX));
    assert_eq!(b.get("x"), Some(Value::Float(21.0)));
    assert!(!b.calc.is_modified());

    b.calc.rename_module("Tax", "Vat").unwrap();
    assert_eq!(b.calc.undo_label(), Some("Rename module Tax to Vat"));
    b.calc.undo().unwrap();
    assert_eq!(b.calc.module_names().collect::<Vec<_>>(), ["A", "Tax", "C"]);
    b.calc.redo().unwrap();
    assert_eq!(b.calc.module_names().collect::<Vec<_>>(), ["A", "Vat", "C"]);
    // The window follows renames, undone and redone ones included.
    assert_eq!(
        b.calc.take_module_renames(),
        [
            ("Tax".to_owned(), "Vat".to_owned()),
            ("Vat".to_owned(), "Tax".to_owned()),
            ("Tax".to_owned(), "Vat".to_owned()),
        ]
    );

    b.calc.add_module("Extra", "y = 1\n").unwrap();
    assert_eq!(b.calc.undo_label(), Some("Insert module Extra"));
    b.calc.undo().unwrap();
    assert_eq!(b.calc.module_names().collect::<Vec<_>>(), ["A", "Vat", "C"]);
}

#[test]
fn a_new_module_takes_the_first_free_number_and_names_are_checked() {
    let mut b = book_with(&[("Module1", ""), ("module3", "")]);
    assert_eq!(b.calc.new_module_name(), "Module2");
    b.calc.add_module("Module2", "").unwrap();
    // Names ignore case, so Module3 is taken by module3.
    assert_eq!(b.calc.new_module_name(), "Module4");
    assert!(b.calc.module_name_problem("Module1", None).is_some());
    assert_eq!(b.calc.module_name_problem("Module1", Some("Module1")), None);
    assert!(b.calc.module_name_problem("not a name", None).is_some());
    // Renaming to the same name is no edit.
    let label = b.calc.undo_label().map(str::to_owned);
    b.calc.rename_module("Module1", "Module1").unwrap();
    assert_eq!(b.calc.undo_label().map(str::to_owned), label);
}

#[test]
fn modules_keep_their_order_when_one_is_renamed() {
    let mut b = book_with(&[("A", ""), ("B", ""), ("C", "")]);
    b.calc.rename_module("B", "Bb").unwrap();
    assert_eq!(b.calc.module_names().collect::<Vec<_>>(), ["A", "Bb", "C"]);
}

// ------------------------------------------------------------- completion

/// Name completion offers the workbook's modules and their functions.
#[test]
fn completion_offers_the_workbook_modules_and_their_functions() {
    use super::completion::{CompletionKind, complete};
    let b = book_with(&[("Tax", TAX)]);
    let root = gossia_core::Path::root();
    let at = |text: &str| complete(&b.calc, "S", &root, text, text.len());

    let c = at("Ta");
    let tax = c.items.iter().find(|i| i.label == "Tax").unwrap();
    assert_eq!(
        (tax.insert.as_str(), tax.kind),
        ("Tax.", CompletionKind::Module)
    );

    let c = at("va");
    let vat = c.items.iter().find(|i| i.label == "vat").unwrap();
    assert_eq!(
        (vat.insert.as_str(), vat.kind),
        ("vat(", CompletionKind::Function)
    );
    assert!(vat.detail.contains("module Tax"));

    let c = at("Tax.");
    assert_eq!(c.start, 4);
    let labels: Vec<_> = c.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["vat"]);
}

// ------------------------------------------------------------------ compile

fn problems(b: &Book, name: &str) -> Vec<(Option<usize>, Option<usize>, String)> {
    b.calc
        .module_problems(name)
        .unwrap()
        .into_iter()
        .map(|p| (p.line, p.column, p.message))
        .collect()
}

/// *Compile* lists each problem of a module with where it is.
#[test]
fn compile_lists_the_problems_of_a_module_with_their_line_and_column() {
    let mut b = book_with(&[
        ("Good", TAX),
        ("A", "import math\nimport B\n"),
        ("B", "x = 1\nfrom A import y\n"),
        ("Odd", "import math, os\n\ndef f():\n    return 1\n"),
        ("Bad", "def f():\n    return 1\n\nx = 1 / 0\n"),
        ("Broken", "def f(:\n    pass\n"),
    ]);
    b.run();
    assert!(problems(&b, "Good").is_empty());
    assert!(b.calc.module_problems("Nothing").is_none());
    // The import that closes the cycle, in each module on it.
    let a = problems(&b, "A");
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!((a[0].0, a[0].1), (Some(2), Some(8)));
    assert!(a[0].2.contains("A -> B -> A"), "{}", a[0].2);
    let at_b = problems(&b, "B");
    assert_eq!((at_b[0].0, at_b[0].1), (Some(2), Some(6)));
    // An import that cannot be served: once, not again as the load failure.
    let odd = problems(&b, "Odd");
    assert_eq!(odd.len(), 1, "{odd:?}");
    assert_eq!((odd[0].0, odd[0].1), (Some(1), Some(14)));
    assert!(odd[0].2.contains("no module named os"), "{}", odd[0].2);
    // A failure at load, with the line it names.
    let bad = problems(&b, "Bad");
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert_eq!((bad[0].0, bad[0].1), (Some(4), None));
    assert!(bad[0].2.contains("ZeroDivisionError"), "{}", bad[0].2);
    // A syntax error is the only problem.
    let broken = problems(&b, "Broken");
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert_eq!(broken[0].0, Some(1));
    assert!(broken[0].2.starts_with("syntax error"), "{}", broken[0].2);
}
