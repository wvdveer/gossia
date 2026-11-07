use super::*;
use crate::{calendar, vcard};
use serde_json::json;

fn stamp() -> Timestamp {
    "2026-11-07T00:00:00Z".parse().unwrap()
}

fn events() -> Vec<J> {
    vec![
        json!({
            "uid": "weekly@example.com",
            "summary": "Stand-up, daily; short",
            "start": "2026-11-09T09:00:00+11:00",
            "end": "2026-11-09T09:30:00+11:00",
            "time_zone": "Australia/Sydney",
            "description": "Line one\nLine two",
            "status": "confirmed",
            "categories": ["Work", "Team, core"],
            "organizer": {"name": "Lovelace, Ada", "email": "ada@example.com"},
            "attendees": [{"name": "Bob", "email": "bob@example.com", "role": "req-participant", "status": "accepted"}],
            "recurrence": "FREQ=WEEKLY;BYDAY=MO",
            "excluded": ["2026-11-16T09:00:00+11:00"],
            "sequence": 2,
            "created": "2026-11-01T00:00:00Z"
        }),
        json!({
            "uid": "weekly@example.com",
            "recurrence_id": "2026-11-30T09:00:00+11:00",
            "summary": "Moved",
            "start": "2026-11-30T10:00:00+11:00",
            "time_zone": "Australia/Sydney"
        }),
        json!({"uid": "holiday@example.com", "summary": "Holidays", "start_date": "2026-12-25", "end_date": "2026-12-26"}),
        json!({"uid": "utc@example.com", "start": "2026-01-15T04:00:00Z"}),
        json!({"summary": "no uid, left out"}),
    ]
}

#[test]
fn events_come_back_as_they_went_out() {
    let text = write_calendar(EntryKind::Events, &events(), stamp());
    assert!(text.split("\r\n").all(|l| l.len() <= 75), "{text}");
    assert!(
        text.contains("DTSTART;TZID=Australia/Sydney:20261109T090000"),
        "{text}"
    );
    assert!(
        text.contains("BEGIN:VTIMEZONE\r\nTZID:Australia/Sydney"),
        "{text}"
    );
    assert!(text.contains("DTEND;VALUE=DATE:20261227"), "{text}");
    assert!(text.contains("DTSTAMP:20261107T000000Z"), "{text}");
    let read = calendar::read(&text, &TimeZone::UTC).unwrap();
    assert_eq!(read.events, events()[..4]);
    assert!(read.notes.is_empty(), "{:?}", read.notes);
}

#[test]
fn a_zone_is_described_across_its_changes() {
    let text = write_calendar(EntryKind::Events, &events(), stamp());
    // Sydney changes in April and October; ten years of RDATEs follow.
    assert!(text.contains("BEGIN:DAYLIGHT"), "{text}");
    assert!(text.contains("BEGIN:STANDARD"), "{text}");
    assert!(text.contains("TZOFFSETTO:+1100"), "{text}");
    assert!(text.matches("RDATE:").count() >= 18, "{text}");
    // Another reader with only the file's description gets the same times.
    let described = text.replace("Australia/Sydney", "Somewhere Made Up");
    let read = calendar::read(&described, &TimeZone::UTC).unwrap();
    assert_eq!(read.events[0]["start"], "2026-11-09T09:00:00+11:00");
}

#[test]
fn tasks_come_back_as_they_went_out() {
    let tasks = vec![json!({
        "uid": "task@example.com",
        "summary": "Write report",
        "due_date": "2026-12-01",
        "completed": "2026-11-20T03:00:00Z",
        "percent_complete": 100,
        "priority": 1,
        "status": "completed"
    })];
    let text = write_calendar(EntryKind::Tasks, &tasks, stamp());
    assert!(text.contains("BEGIN:VTODO"));
    assert_eq!(calendar::read(&text, &TimeZone::UTC).unwrap().tasks, tasks);
}

#[test]
fn contacts_come_back_as_they_went_out() {
    let contacts = vec![
        json!({
            "uid": "urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1",
            "full_name": "Dr. Ada Lovelace",
            "name": {"family": "Lovelace", "given": "Ada", "additional": "Augusta", "prefix": "Dr."},
            "nickname": "Ada, Countess",
            "organization": "Analytical Engines Ltd",
            "department": "Research, Notes",
            "title": "Analyst",
            "birthday": "1815-12-10",
            "anniversary": "1835-07-08",
            "emails": [{"address": "ada@example.com", "type": "work"}],
            "phones": [{"number": "+61 400 000 000", "type": "cell,voice"}],
            "addresses": [{"type": "work", "street": "12 Engine St; rear", "locality": "Brisbane", "country": "Australia"}],
            "urls": ["https://example.com/ada"],
            "note": "Wrote the first, published program.\nSecond line.",
            "categories": ["Friends", "Work"],
            "revision": "2026-11-07T09:00:00Z"
        }),
        json!({"full_name": "Charles Babbage"}),
        json!({"uid": "nameless"}),
    ];
    let text = write_contacts(&contacts);
    assert!(text.contains("VERSION:3.0"));
    let read = vcard::read(&text).unwrap();
    assert_eq!(read.contacts, contacts[..2]);
    assert!(read.notes.is_empty(), "{:?}", read.notes);
}
