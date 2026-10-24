//! Tests of the trust model (DESIGN.md, "Trust model"): the digest of the
//! executable content, the record of what was trusted, and untrusted mode,
//! run through the real calculation layer and the real engine.

use cliotree_core::Value;

use super::tests::{Book, p};
use crate::{ChangeKind, Decision, ModuleStatus, TrustStore};

const TAX: &str = "def vat(x, rate=0.21):\n    return round(x * rate, 2)\n";

fn trusted_store(b: &Book) -> TrustStore {
    let mut store = TrustStore::default();
    store.trust("id-1", b.calc.fingerprint());
    store
}

// ------------------------------------------------------------ the digest

/// Trust model: the digest covers the executable content and nothing else.
#[test]
fn data_does_not_change_the_digest_but_executable_content_does() {
    let mut b = Book::new();
    b.formula("c", "S.a + 1");
    let before = b.calc.fingerprint();

    b.set("a", Value::Int(5));
    b.set("name", Value::Str("other".into()));
    assert_eq!(b.calc.fingerprint(), before, "a data edit is not content");

    b.formula("c", "S.a + 2");
    let edited = b.calc.fingerprint();
    assert_ne!(edited.hash, before.hash, "a formula is content");

    b.calc.add_module("Tax", TAX).unwrap();
    let with_module = b.calc.fingerprint();
    assert_ne!(with_module.hash, edited.hash, "a module is content");

    b.calc.set_module_source("Tax", "x = 1\n").unwrap();
    assert_ne!(b.calc.fingerprint().hash, with_module.hash);
}

#[test]
fn the_digest_does_not_depend_on_the_order_things_were_added() {
    let mut one = Book::new();
    one.formula("a", "1");
    one.formula("b", "2");
    let mut two = Book::new();
    two.formula("b", "2");
    two.formula("a", "1");
    assert_eq!(one.calc.fingerprint(), two.calc.fingerprint());
}

#[test]
fn the_order_of_the_modules_is_content_because_it_decides_who_keeps_a_name() {
    let mut one = Book::new();
    one.calc.add_module("A", "x = 1\n").unwrap();
    one.calc.add_module("B", "y = 1\n").unwrap();
    let mut two = Book::new();
    two.calc.add_module("B", "y = 1\n").unwrap();
    two.calc.add_module("A", "x = 1\n").unwrap();
    assert_ne!(one.calc.fingerprint().hash, two.calc.fingerprint().hash);
}

#[test]
fn undo_returns_to_the_digest_it_had() {
    let mut b = Book::new();
    b.formula("c", "S.a + 1");
    let before = b.calc.fingerprint();
    b.calc.seal_history();
    b.formula("c", "S.a + 2");
    assert_ne!(b.calc.fingerprint(), before);
    b.calc.undo().unwrap();
    assert_eq!(b.calc.fingerprint(), before);
}

// ------------------------------------------------------------ the record

/// Trust model, "Recording trust": a matching id and digest opens trusted.
#[test]
fn a_matching_id_and_digest_is_trusted() {
    let mut b = Book::new();
    b.formula("c", "S.a");
    let store = trusted_store(&b);
    assert_eq!(
        store.decide("id-1", &b.calc.fingerprint()),
        Decision::Trusted
    );
}

#[test]
fn an_unknown_id_is_unknown_and_so_is_an_id_copied_into_a_hostile_file() {
    let mut good = Book::new();
    good.formula("c", "S.a");
    let store = trusted_store(&good);
    assert_eq!(
        store.decide("nobody", &good.calc.fingerprint()),
        Decision::Unknown
    );

    // The id is known, but the file is not what was trusted.
    let mut hostile = Book::new();
    hostile.formula("c", "S.a");
    hostile.formula("b", "S.a");
    assert!(matches!(
        store.decide("id-1", &hostile.calc.fingerprint()),
        Decision::Changed(_)
    ));
}

/// Trust model: when the file changed outside the application, the banner lists
/// what changed.
#[test]
fn a_changed_file_says_what_changed() {
    let mut b = Book::new();
    b.formula("a", "1");
    b.formula("b", "2");
    b.calc.add_module("Tax", TAX).unwrap();
    let store = trusted_store(&b);

    b.formula("a", "100"); // edited
    b.calc.remove_formula("S", &p("b")).unwrap(); // removed
    b.formula("c", "3"); // added
    b.calc.set_module_source("Tax", "x = 1\n").unwrap(); // edited

    let Decision::Changed(changes) = store.decide("id-1", &b.calc.fingerprint()) else {
        panic!("not changed");
    };
    let find = |item: &str| {
        changes
            .iter()
            .find(|c| c.item == item)
            .unwrap_or_else(|| panic!("{item} not listed in {changes:?}"))
            .kind
    };
    assert_eq!(find("formula S!a"), ChangeKind::Edited);
    assert_eq!(find("formula S!b"), ChangeKind::Removed);
    assert_eq!(find("formula S!c"), ChangeKind::Added);
    assert_eq!(find("module Tax"), ChangeKind::Edited);
    assert_eq!(changes.len(), 4);
}

/// Trust model: edits made inside the application update the record, with no
/// question; an unrecorded workbook cannot become trusted by being edited.
#[test]
fn edits_inside_the_application_keep_the_record_up_to_date() {
    let mut b = Book::new();
    b.formula("c", "S.a");
    let mut store = trusted_store(&b);
    b.formula("c", "S.a + 1");
    assert!(store.update("id-1", b.calc.fingerprint()));
    assert_eq!(
        store.decide("id-1", &b.calc.fingerprint()),
        Decision::Trusted
    );
    assert!(!store.update("other", b.calc.fingerprint()));
    assert!(!store.is_recorded("other"));
}

#[test]
fn save_as_carries_the_trust_to_the_copys_new_id_and_revoking_forgets_it() {
    let mut b = Book::new();
    b.formula("c", "S.a");
    let mut store = trusted_store(&b);
    assert!(store.carry("id-1", "id-2"));
    assert_eq!(
        store.decide("id-2", &b.calc.fingerprint()),
        Decision::Trusted
    );
    assert!(!store.carry("missing", "id-3"));
    assert!(store.revoke("id-2"));
    assert!(!store.revoke("id-2"));
    assert_eq!(
        store.decide("id-2", &b.calc.fingerprint()),
        Decision::Unknown
    );
    assert_eq!(store.ids().collect::<Vec<_>>(), ["id-1"]);
}

// ------------------------------------------------------ untrusted mode

/// Trust model, "Untrusted mode": nothing is evaluated, the stored value stays.
#[test]
fn an_untrusted_workbook_runs_nothing_and_keeps_the_stored_values() {
    let mut b = Book::new();
    b.formula("c", "S.a + 10");
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(11)));

    b.calc.set_trusted(false);
    assert!(!b.calc.is_trusted());
    b.set("a", Value::Int(5));
    assert_eq!(b.run(), 0, "nothing is evaluated");
    assert_eq!(b.get("c"), Some(Value::Int(11)), "the stored value stays");

    let cell = b.calc.cell("S", &p("c")).unwrap();
    assert!(cell.untrusted);
    assert!(cell.stale, "its input changed and it cannot recalculate");
    assert!(!cell.pending);
    assert_eq!(b.calc.pending(), 0);
    assert_eq!(b.calc.stale(), 1);

    // A cell with no formula is not touched by any of this.
    let plain = b.calc.cell("S", &p("a")).unwrap();
    assert!(!plain.untrusted && !plain.stale);
}

#[test]
fn a_formula_can_be_entered_in_an_untrusted_workbook_and_waits_unevaluated() {
    let mut b = Book::new();
    b.calc.set_trusted(false);
    b.formula("c", "S.a + 10");
    assert_eq!(
        b.calc.formula_text("S", &p("c")).as_deref(),
        Some("S.a + 10")
    );
    b.run();
    assert_eq!(
        b.get("c"),
        Some(Value::Int(0)),
        "the node keeps what it had"
    );
    assert!(b.calc.cell("S", &p("c")).unwrap().stale);
}

/// Trust model: trusting recalculates everything, as opening a trusted workbook
/// does.
#[test]
fn trusting_calculates_what_waited() {
    let mut b = Book::new();
    b.calc.set_trusted(false);
    b.formula("c", "S.a + 10");
    b.run();
    b.calc.set_trusted(true);
    assert_eq!(b.calc.stale(), 0);
    assert_eq!(b.calc.pending(), 1);
    b.run();
    assert_eq!(b.get("c"), Some(Value::Int(11)));
    let cell = b.calc.cell("S", &p("c")).unwrap();
    assert!(!cell.untrusted && !cell.stale && !cell.pending);
}

/// Trust model: module sources are executable content, so in an untrusted
/// workbook they are not loaded.
#[test]
fn modules_are_not_loaded_until_the_workbook_is_trusted() {
    let mut b = Book::new();
    b.calc.set_trusted(false);
    b.calc
        .add_module("Tax", "raise RuntimeError('ran')\n")
        .unwrap();
    b.formula("x", "vat(100)");
    b.run();
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Untrusted));
    assert_eq!(b.calc.pending_modules(), 0);
    assert!(b.calc.module_issues().is_empty(), "not a failure");
    assert_eq!(b.get("x"), Some(Value::Float(1.5)));

    // The source can be edited; it is shown and kept.
    b.calc.set_module_source("Tax", TAX).unwrap();
    assert_eq!(b.calc.module_source("Tax"), Some(TAX));
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Untrusted));

    b.calc.set_trusted(true);
    b.run();
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Loaded));
    assert_eq!(b.get("x"), Some(Value::Float(21.0)));
}

#[test]
fn trusting_again_loads_the_modules_again() {
    let mut b = Book::new();
    b.calc.add_module("Tax", TAX).unwrap();
    b.formula("x", "vat(100)");
    b.run();
    let loaded = b.calc.module_generation("Tax").unwrap();

    b.calc.set_trusted(false);
    b.calc.set_trusted(true);
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Pending));
    assert!(b.calc.module_generation("Tax").unwrap() > loaded);
    b.run();
    assert_eq!(b.calc.module_status("Tax"), Some(&ModuleStatus::Loaded));
    assert_eq!(b.get("x"), Some(Value::Float(21.0)));
}

#[test]
fn setting_the_state_it_already_has_changes_nothing() {
    let mut b = Book::new();
    b.formula("c", "S.a");
    b.run();
    let revision = b.calc.revision();
    b.calc.set_trusted(true);
    assert_eq!(b.calc.revision(), revision);
    assert_eq!(b.calc.pending(), 0);
}
