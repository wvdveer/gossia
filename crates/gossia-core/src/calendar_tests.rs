use super::*;
use crate::validate::validate;

const ICS: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//Test//EN\r
BEGIN:VTIMEZONE\r
TZID:Made Up Time\r
BEGIN:STANDARD\r
DTSTART:19700101T000000\r
TZOFFSETFROM:+0300\r
TZOFFSETTO:+0300\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
UID:weekly@example.com\r
DTSTAMP:20261101T000000Z\r
DTSTART;TZID=Australia/Brisbane:20261109T090000\r
DTEND;TZID=Australia/Brisbane:20261109T093000\r
SUMMARY:Stand-up\\, daily\r
DESCRIPTION:Line one\\nLine two\r
RRULE:FREQ=WEEKLY;BYDAY=MO\r
EXDATE;TZID=Australia/Brisbane:20261116T090000,20261123T090000\r
CATEGORIES:Work,Team\r
STATUS:CONFIRMED\r
ORGANIZER;CN=Ada Lovelace:mailto:ada@example.com\r
ATTENDEE;CN=Bob;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED:mailto:bob@example.com\r
X-MICROSOFT-CDO-BUSYSTATUS:BUSY\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT10M\r
END:VALARM\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:weekly@example.com\r
RECURRENCE-ID;TZID=Australia/Brisbane:20261130T090000\r
DTSTART;TZID=Australia/Brisbane:20261130T100000\r
DURATION:PT45M\r
SUMMARY:Stand-up (moved)\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:holiday@example.com\r
DTSTART;VALUE=DATE:20261225\r
DTEND;VALUE=DATE:20261227\r
SUMMARY:Holidays\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:outlook@example.com\r
DTSTART;TZID=AUS Eastern Standard Time:20260115T140000\r
DTEND:20260115T040000Z\r
SUMMARY:Outlook\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:described@example.com\r
DTSTART;TZID=Made Up Time:20260115T140000\r
SUMMARY:Described\r
END:VEVENT\r
BEGIN:VEVENT\r
SUMMARY:No UID\r
END:VEVENT\r
BEGIN:VTODO\r
UID:task@example.com\r
DUE;VALUE=DATE:20261201\r
PRIORITY:1\r
PERCENT-COMPLETE:150\r
STATUS:IN-PROCESS\r
SUMMARY:Write report\r
END:VTODO\r
BEGIN:VJOURNAL\r
UID:j@example.com\r
END:VJOURNAL\r
END:VCALENDAR\r
";

fn read_it() -> CalendarRead {
    read(ICS, &TimeZone::UTC).unwrap()
}

#[test]
fn events_are_read_with_their_times_converted() {
    let read = read_it();
    assert_eq!(read.events.len(), 5);
    let weekly = &read.events[0];
    assert_eq!(
        weekly,
        &json!({
            "uid": "weekly@example.com",
            "summary": "Stand-up, daily",
            "start": "2026-11-09T09:00:00+10:00",
            "end": "2026-11-09T09:30:00+10:00",
            "time_zone": "Australia/Brisbane",
            "description": "Line one\nLine two",
            "status": "confirmed",
            "categories": ["Work", "Team"],
            "organizer": {"name": "Ada Lovelace", "email": "ada@example.com"},
            "attendees": [{"name": "Bob", "email": "bob@example.com", "role": "req-participant", "status": "accepted"}],
            "recurrence": "FREQ=WEEKLY;BYDAY=MO",
            "excluded": ["2026-11-16T09:00:00+10:00", "2026-11-23T09:00:00+10:00"]
        })
    );
    let moved = &read.events[1];
    assert_eq!(moved["recurrence_id"], "2026-11-30T09:00:00+10:00");
    assert_eq!(moved["end"], "2026-11-30T10:45:00+10:00");
    let holiday = &read.events[2];
    assert_eq!(holiday["start_date"], "2026-12-25");
    // DTEND is the day after the last day.
    assert_eq!(holiday["end_date"], "2026-12-26");
    let outlook = &read.events[3];
    assert_eq!(outlook["start"], "2026-01-15T14:00:00+11:00");
    assert_eq!(outlook["time_zone"], "Australia/Sydney");
    assert_eq!(outlook["end"], "2026-01-15T04:00:00Z");
    let described = &read.events[4];
    assert_eq!(described["start"], "2026-01-15T14:00:00+03:00");
    assert!(described.get("time_zone").is_none());
}

#[test]
fn tasks_go_apart_and_are_clamped_to_the_schema() {
    let read = read_it();
    assert_eq!(
        read.tasks,
        [json!({
            "uid": "task@example.com",
            "summary": "Write report",
            "due_date": "2026-12-01",
            "percent_complete": 100,
            "priority": 1,
            "status": "in-process"
        })]
    );
}

#[test]
fn the_report_names_what_was_left_out() {
    let notes = read_it().notes.join("\n");
    assert!(notes.contains("VJOURNAL (1)"), "{notes}");
    assert!(notes.contains("VEVENT without a UID (1)"), "{notes}");
    assert!(notes.contains("Alarms are not kept (1)"), "{notes}");
    assert!(notes.contains("X-MICROSOFT-CDO-BUSYSTATUS (1)"), "{notes}");
    assert!(notes.contains("Made Up Time"), "{notes}");
    assert!(!notes.contains("DTSTAMP"), "{notes}");
}

#[test]
fn what_is_read_fits_the_schemas() {
    let read = read_it();
    for (kind, entries) in [
        (EntryKind::Events, &read.events),
        (EntryKind::Tasks, &read.tasks),
    ] {
        let schema = kind.schema();
        let spec = crate::jsonschema::lower(&schema).unwrap();
        let issues = validate(&spec, &data(entries));
        assert!(issues.is_empty(), "{kind:?}: {issues:?}");
    }
}

#[test]
fn the_same_entry_twice_keeps_the_later_in_the_earlier_place() {
    let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nSUMMARY:first\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nEND:VEVENT\nBEGIN:VEVENT\nUID:a\nSUMMARY:second\nEND:VEVENT\nEND:VCALENDAR\n";
    let read = read(ics, &TimeZone::UTC).unwrap();
    assert_eq!(read.events.len(), 2);
    assert_eq!(read.events[0]["summary"], "second");
    assert!(read.notes.iter().any(|n| n.contains("the later is kept")));
}

#[test]
fn floating_times_take_the_zone_given() {
    let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:f\nDTSTART:20260715T090000\nEND:VEVENT\nEND:VCALENDAR\n";
    let (_, perth) = crate::timezone::iana("Australia/Perth").unwrap();
    let read = read(ics, &perth).unwrap();
    assert_eq!(read.events[0]["start"], "2026-07-15T09:00:00+08:00");
    assert!(read.notes.iter().any(|n| n.contains("no time zone")));
}

#[test]
fn text_that_is_not_a_calendar_is_refused() {
    assert!(read("BEGIN:VCARD\nFN:x\nEND:VCARD\n", &TimeZone::UTC).is_err());
    assert!(read("hello", &TimeZone::UTC).is_err());
}

#[test]
fn kinds_are_told_by_their_titles_and_keys() {
    for kind in EntryKind::ALL {
        assert_eq!(EntryKind::from_title(kind.title()), Some(kind));
    }
    let e = json!({"uid": "a", "recurrence_id": "x"});
    assert_ne!(
        key_of(&e, EntryKind::Events),
        key_of(&json!({"uid": "a"}), EntryKind::Events)
    );
    assert_eq!(
        key_of(&json!({"full_name": "Ada"}), EntryKind::Contacts).as_deref(),
        Some("name:Ada")
    );
}
