//! Tests of tabs of calendar entries and contacts (DESIGN.md, Calendars and
//! contacts, 0.4.0).

use std::sync::Arc;

use gossia_core::calendar::{EntryKind, data};
use gossia_core::{Node, json};
use serde_json::json;

use crate::Calc;
use crate::fake::FakeEngine;

fn items(list: serde_json::Value) -> Vec<Node> {
    let serde_json::Value::Array(list) = list else {
        panic!("not a list")
    };
    data(&list).value().as_array().unwrap().to_vec()
}

/// "Events" of two events, bound to "EventsSchema", and a tab of
/// contacts, plus a tab of something else.
fn book() -> Calc {
    let mut calc = Calc::new(Arc::new(FakeEngine::new()));
    calc.create_json_schema_tab("EventsSchema", EntryKind::Events.schema())
        .unwrap();
    let events = json!([
        {"uid": "a", "summary": "one"},
        {"uid": "a", "recurrence_id": "2026-11-30T09:00:00+10:00", "summary": "moved"}
    ]);
    calc.create_data_tab(
        "Events",
        "EventsSchema",
        data(events.as_array().unwrap()),
        None,
    )
    .unwrap();
    calc.create_json_schema_tab("ContactsSchema", EntryKind::Contacts.schema())
        .unwrap();
    calc.create_data_tab("People", "ContactsSchema", data(&[]), None)
        .unwrap();
    calc.create_json_schema_tab(
        "OtherSchema",
        json::parse(r#"{"type": "array", "items": {"type": "integer"}}"#)
            .unwrap()
            .node,
    )
    .unwrap();
    calc.create_data_tab(
        "Other",
        "OtherSchema",
        json::parse("[1]").unwrap().node,
        None,
    )
    .unwrap();
    calc.clear_history();
    calc
}

#[test]
fn tabs_are_told_by_their_schema_title() {
    let calc = book();
    assert_eq!(calc.entry_kind("Events"), Some(EntryKind::Events));
    assert_eq!(calc.entry_kind("People"), Some(EntryKind::Contacts));
    assert_eq!(calc.entry_kind("Other"), None);
    assert_eq!(calc.entry_kind("EventsSchema"), None);
    assert_eq!(calc.tabs_of_kind(EntryKind::Events), ["Events"]);
    assert!(calc.tabs_of_kind(EntryKind::Tasks).is_empty());
}

#[test]
fn the_same_uid_replaces_in_place_and_the_rest_go_at_the_end() {
    let mut calc = book();
    let new = items(json!([
        {"uid": "b", "summary": "new"},
        {"uid": "a", "summary": "changed"},
        {"uid": "b", "summary": "newer"}
    ]));
    let (replaced, added) = calc
        .add_entries("Events", EntryKind::Events, &new, "Import")
        .unwrap();
    assert_eq!((replaced, added), (2, 1));
    let list = calc.data("Events").unwrap().value().as_array().unwrap();
    let summaries: Vec<_> = list
        .iter()
        .map(|n| {
            n.value()
                .as_object()
                .unwrap()
                .get("summary")
                .unwrap()
                .value()
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(summaries, ["changed", "moved", "newer"]);
    // One thing to undo.
    assert_eq!(calc.history_len().0, 1);
    calc.undo().unwrap();
    assert_eq!(
        calc.data("Events")
            .unwrap()
            .value()
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn contacts_without_a_uid_match_by_name() {
    let mut calc = book();
    let first = items(json!([{"full_name": "Ada"}, {"uid": "x", "full_name": "Bob"}]));
    assert_eq!(
        calc.add_entries("People", EntryKind::Contacts, &first, "Import")
            .unwrap(),
        (0, 2)
    );
    let again =
        items(json!([{"full_name": "Ada", "note": "hi"}, {"uid": "y", "full_name": "Bob"}]));
    assert_eq!(
        calc.add_entries("People", EntryKind::Contacts, &again, "Import")
            .unwrap(),
        (1, 1)
    );
}

#[test]
fn a_missing_tab_is_refused() {
    let mut calc = book();
    assert!(
        calc.add_entries("Nope", EntryKind::Events, &[], "Import")
            .is_err()
    );
}
