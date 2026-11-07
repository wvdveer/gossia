//! Tests of importing calendars and contacts (DESIGN.md, "Calendars and
//! contacts, 0.4.0").

use std::time::Duration;

use gossia_core::calendar::EntryKind;
use gossia_core::validate::validate;

use crate::GossiaApp;
use crate::import::{ImportFormat, SchemaChoice, format_of, format_of_text};

fn sample() -> GossiaApp {
    let mut app = GossiaApp::sample();
    app.files.browse = false;
    assert!(app.session.wait_idle(Duration::from_secs(60)));
    app
}

fn valid(app: &GossiaApp, tab: &str) -> bool {
    let calc = app.calc();
    validate(calc.spec(tab).unwrap(), calc.data(tab).unwrap()).is_empty()
}

fn summaries(app: &GossiaApp, tab: &str) -> Vec<String> {
    app.calc()
        .data(tab)
        .unwrap()
        .value()
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let o = n.value().as_object().unwrap();
            let field = o.get("summary").or_else(|| o.get("full_name")).unwrap();
            field.value().as_str().unwrap().to_owned()
        })
        .collect()
}

const ICS: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
BEGIN:VEVENT\r
UID:a@example.com\r
DTSTART:20261109T090000Z\r
SUMMARY:Stand-up\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:b@example.com\r
DTSTART;VALUE=DATE:20261225\r
SUMMARY:Holiday\r
END:VEVENT\r
BEGIN:VTODO\r
UID:t@example.com\r
SUMMARY:Write report\r
END:VTODO\r
END:VCALENDAR\r
";

const MORE: &str = "BEGIN:VCALENDAR\r
BEGIN:VEVENT\r
UID:b@example.com\r
DTSTART;VALUE=DATE:20261226\r
SUMMARY:Holiday (moved)\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:c@example.com\r
DTSTART:20261110T090000Z\r
SUMMARY:Review\r
END:VEVENT\r
END:VCALENDAR\r
";

#[test]
fn calendars_and_contacts_are_told_by_name_and_text() {
    assert_eq!(format_of("Work.ICS"), ImportFormat::Calendar);
    assert_eq!(format_of("people.vcf"), ImportFormat::Contacts);
    assert_eq!(format_of_text(ICS), ImportFormat::Calendar);
    assert_eq!(
        format_of_text("begin:vcard\nFN:x\nend:vcard\n"),
        ImportFormat::Contacts
    );
}

#[test]
fn a_calendar_becomes_a_tab_of_events_and_one_of_tasks() {
    let mut app = sample();
    let plan = app.plan_import("Work.ics", ICS.as_bytes()).unwrap();
    let parts: Vec<(&str, bool, usize, &str)> = plan
        .sheets
        .iter()
        .map(|s| (s.title.as_str(), s.ticked, s.rows, s.plan.name.as_str()))
        .collect();
    assert_eq!(
        parts,
        [
            ("Events", true, 2, "Work_Events"),
            ("Tasks", true, 1, "Work_Tasks")
        ]
    );
    assert_eq!(plan.sheets[0].plan.choice, SchemaChoice::Inferred);
    app.import(&plan).unwrap();
    assert_eq!(app.sheet, "Work_Events");
    assert_eq!(
        app.calc().entry_kind("Work_Events"),
        Some(EntryKind::Events)
    );
    assert_eq!(app.calc().entry_kind("Work_Tasks"), Some(EntryKind::Tasks));
    assert!(valid(&app, "Work_Events"));
    assert!(valid(&app, "Work_Tasks"));
    assert_eq!(summaries(&app, "Work_Events"), ["Stand-up", "Holiday"]);
    assert_eq!(app.calc().undo_label(), Some("Import Work.ics"));
}

#[test]
fn a_second_calendar_is_added_to_the_tab_there_by_uid() {
    let mut app = sample();
    let plan = app.plan_import("Work.ics", ICS.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let tabs = app.calc().sheet_names().count();
    let plan = app.plan_import("More.ics", MORE.as_bytes()).unwrap();
    // No tasks in this one: that part is not ticked.
    assert!(!plan.sheets[1].ticked);
    let events = &plan.sheets[0].plan;
    assert_eq!(events.add_to, ["Work_Events"]);
    assert_eq!(events.choice, SchemaChoice::AddTo("Work_Events".into()));
    assert_eq!(app.import_problem(&plan), None);
    app.import(&plan).unwrap();
    assert_eq!(app.calc().sheet_names().count(), tabs);
    assert_eq!(app.sheet, "Work_Events");
    assert_eq!(
        summaries(&app, "Work_Events"),
        ["Stand-up", "Holiday (moved)", "Review"]
    );
    assert!(valid(&app, "Work_Events"));
    // One undo takes the whole file back out.
    app.session.calc_mut().undo().unwrap();
    assert_eq!(summaries(&app, "Work_Events"), ["Stand-up", "Holiday"]);
}

#[test]
fn the_same_calendar_can_still_go_to_a_new_tab_on_the_schema_there() {
    let mut app = sample();
    let plan = app.plan_import("Work.ics", ICS.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let mut plan = app.plan_import("More.ics", MORE.as_bytes()).unwrap();
    let events = &mut plan.sheets[0].plan;
    assert!(
        events
            .candidates
            .iter()
            .any(|c| c.schema == "Work_EventsSchema" && c.problems == 0)
    );
    events.choice = SchemaChoice::Existing("Work_EventsSchema".into());
    app.import(&plan).unwrap();
    assert_eq!(
        app.calc().tabs_of_kind(EntryKind::Events),
        ["Work_Events", "More_Events"]
    );
}

#[test]
fn contacts_are_one_tab_and_added_to_by_uid_or_name() {
    let mut app = sample();
    let vcf = "BEGIN:VCARD\nVERSION:3.0\nFN:Ada Lovelace\nEMAIL:ada@example.com\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nUID:x\nFN:Charles Babbage\nEND:VCARD\n";
    let plan = app.plan_import("people.vcf", vcf.as_bytes()).unwrap();
    assert!(plan.sheets.is_empty());
    assert_eq!(plan.name, "people");
    app.import(&plan).unwrap();
    assert!(valid(&app, "people"));
    let more = "BEGIN:VCARD\nVERSION:3.0\nFN:Ada Lovelace\nNOTE:changed\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nFN:Grace Hopper\nEND:VCARD\n";
    let plan = app.plan_paste(more).unwrap();
    assert_eq!(plan.format, ImportFormat::Contacts);
    assert_eq!(plan.choice, SchemaChoice::AddTo("people".into()));
    assert!(plan.replace.is_none());
    app.import(&plan).unwrap();
    assert_eq!(
        summaries(&app, "people"),
        ["Ada Lovelace", "Charles Babbage", "Grace Hopper"]
    );
}

#[test]
fn a_file_with_nothing_to_import_is_refused() {
    let app = sample();
    let why = app
        .plan_import("empty.ics", b"BEGIN:VCALENDAR\nEND:VCALENDAR\n")
        .unwrap_err();
    assert!(why.contains("no events or tasks"), "{why}");
    let why = app.plan_import("x.vcf", b"hello").unwrap_err();
    assert!(why.contains("cannot be read as contacts"), "{why}");
}

#[test]
fn a_tab_of_entries_exports_as_a_file_that_imports_back_onto_it() {
    let mut app = sample();
    let plan = app.plan_import("Work.ics", ICS.as_bytes()).unwrap();
    app.import(&plan).unwrap();
    let calc = app.calc();
    assert!(
        calc.export_formats("Work_Events")
            .contains(&gossia_calc::ExportFormat::Calendar)
    );
    assert!(
        !calc
            .export_formats("Work_Events")
            .contains(&gossia_calc::ExportFormat::Contacts)
    );
    let text = calc
        .export_text("Work_Events", gossia_calc::ExportFormat::Calendar, true)
        .unwrap();
    assert!(text.starts_with("BEGIN:VCALENDAR\r\n"), "{text}");
    let before = summaries(&app, "Work_Events");
    let plan = app.plan_import("Again.ics", text.as_bytes()).unwrap();
    assert_eq!(plan.choice, SchemaChoice::AddTo("Work_Events".into()));
    app.import(&plan).unwrap();
    // Every entry matched its own UID: nothing was added.
    assert_eq!(summaries(&app, "Work_Events"), before);
}
