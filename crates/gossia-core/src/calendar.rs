//! Calendar files (`.ics`, RFC 5545) read into lists of events and of tasks,
//! each with a schema Gossia knows (DESIGN.md, Calendars and contacts,
//! 0.4.0). The contacts of `.vcf` files are in [`crate::vcard`]; both kinds
//! are named by [`EntryKind`].

use std::collections::{BTreeMap, HashMap};

use gossia_i18n::tr;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde_json::{Map, Value as J, json};

use crate::contentline::{self, Component, Property};
use crate::timezone::{self, Described, Zone};
use crate::value::Node;

/// The kinds of entry Gossia knows a schema for. A tab is of a kind when its
/// schema's root `title` is the kind's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Events,
    Tasks,
    Contacts,
}

impl EntryKind {
    pub const ALL: [EntryKind; 3] = [EntryKind::Events, EntryKind::Tasks, EntryKind::Contacts];

    /// The schema's root title, which tells a tab of the kind.
    pub fn title(self) -> &'static str {
        match self {
            EntryKind::Events => "Calendar events",
            EntryKind::Tasks => "Calendar tasks",
            EntryKind::Contacts => "Contacts",
        }
    }

    /// The kind whose title a schema has.
    pub fn from_title(title: &str) -> Option<EntryKind> {
        EntryKind::ALL.into_iter().find(|k| k.title() == title)
    }

    /// The fields that together tell one entry from another: an entry of the
    /// file replaces one in the tab with the same values of these.
    pub fn key(self) -> &'static [&'static str] {
        match self {
            EntryKind::Events | EntryKind::Tasks => &["uid", "recurrence_id"],
            EntryKind::Contacts => &["uid"],
        }
    }

    /// The schema Gossia writes for the kind.
    pub fn schema(self) -> Node {
        let value = match self {
            EntryKind::Events => events_schema(),
            EntryKind::Tasks => tasks_schema(),
            EntryKind::Contacts => crate::vcard::schema(),
        };
        node(&value)
    }
}

/// What reading a calendar gave: its events and its tasks, and what was not
/// read, for the report.
#[derive(Debug, Clone)]
pub struct CalendarRead {
    pub events: Vec<J>,
    pub tasks: Vec<J>,
    pub notes: Vec<String>,
}

/// A list of entries as a data node.
pub fn data(entries: &[J]) -> Node {
    node(&J::Array(entries.to_vec()))
}

/// A JSON value Gossia built as a node.
pub(crate) fn node(value: &J) -> Node {
    crate::json::parse(&value.to_string())
        .expect("Gossia writes valid JSON")
        .node
}

/// Reads an `.ics` file. `local` is the zone of floating times. Refused when
/// the text is not iCalendar or holds no calendar.
pub fn read(text: &str, local: &TimeZone) -> Result<CalendarRead, String> {
    let top = contentline::parse(text)?;
    let calendars: Vec<&Component> = top.iter().filter(|c| c.name == "VCALENDAR").collect();
    if calendars.is_empty() {
        return Err(tr!("there is no calendar in it (no BEGIN:VCALENDAR)").to_owned());
    }
    let mut reader = Reader {
        local: local.clone(),
        zones: HashMap::new(),
        not_kept: BTreeMap::new(),
        unknown_zones: Vec::new(),
        floating: 0,
        alarms: 0,
        skipped: BTreeMap::new(),
    };
    let mut events = Vec::new();
    let mut tasks = Vec::new();
    for calendar in &calendars {
        for part in &calendar.children {
            if part.name == "VTIMEZONE"
                && let Some(id) = part.get("TZID")
            {
                reader.zones.insert(id.text(), part.clone());
            }
        }
        for part in &calendar.children {
            match part.name.as_str() {
                "VEVENT" => events.extend(reader.event(part)),
                "VTODO" => tasks.extend(reader.task(part)),
                "VTIMEZONE" => {}
                other => *reader.skipped.entry(other.to_owned()).or_default() += 1,
            }
        }
    }
    let mut notes = reader.notes();
    notes.extend(later_wins(&mut events, EntryKind::Events));
    notes.extend(later_wins(&mut tasks, EntryKind::Tasks));
    Ok(CalendarRead {
        events,
        tasks,
        notes,
    })
}

/// Entries of the same key in one file: the later is kept, in the earlier's
/// place.
pub(crate) fn later_wins(entries: &mut Vec<J>, kind: EntryKind) -> Option<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<J> = Vec::with_capacity(entries.len());
    let mut repeats = 0;
    for entry in entries.drain(..) {
        match key_of(&entry, kind) {
            Some(key) => match seen.get(&key) {
                Some(&at) => {
                    out[at] = entry;
                    repeats += 1;
                }
                None => {
                    seen.insert(key, out.len());
                    out.push(entry);
                }
            },
            None => out.push(entry),
        }
    }
    *entries = out;
    (repeats > 0).then(|| match repeats {
        1 => tr!("1 entry repeats an earlier one in the file; the later is kept.").to_owned(),
        n => tr!(
            "{n} entries repeat earlier ones in the file; the later are kept.",
            n = n
        ),
    })
}

/// An entry's key as text, or None when it has none (no `uid`, and for a
/// contact no full name either).
pub fn key_of(entry: &J, kind: EntryKind) -> Option<String> {
    key_with(kind, |name| entry.get(name).and_then(J::as_str))
}

/// The same for an entry already in a tab.
pub fn node_key(entry: &Node, kind: EntryKind) -> Option<String> {
    key_with(kind, |name| {
        entry.value().as_object()?.get(name)?.value().as_str()
    })
}

fn key_with<'a>(kind: EntryKind, field: impl Fn(&str) -> Option<&'a str>) -> Option<String> {
    if kind == EntryKind::Contacts {
        return field("uid")
            .map(|u| format!("uid:{u}"))
            .or_else(|| field("full_name").map(|n| format!("name:{n}")));
    }
    let uid = field("uid")?;
    Some(format!(
        "{uid}\u{1f}{}",
        field("recurrence_id").unwrap_or("")
    ))
}

struct Reader {
    local: TimeZone,
    zones: HashMap<String, Component>,
    /// Properties not kept, by name, with how many.
    not_kept: BTreeMap<String, usize>,
    unknown_zones: Vec<String>,
    floating: usize,
    alarms: usize,
    /// Components not read (journal entries, free/busy), by name.
    skipped: BTreeMap<String, usize>,
}

/// A time as read: a date for all-day entries, or a date-time with its zone.
enum When {
    Date(jiff::civil::Date),
    Time {
        text: String,
        local: DateTime,
        zone: Option<Zone>,
    },
}

impl Reader {
    fn event(&mut self, c: &Component) -> Option<J> {
        let mut e = Map::new();
        self.common(c, &mut e, EVENT_DONE)?;
        let start = c.get("DTSTART").and_then(|p| self.when(p));
        let mut zone_name = None;
        match &start {
            Some(When::Date(d)) => {
                e.insert("start_date".into(), json!(d.to_string()));
            }
            Some(When::Time { text, zone, .. }) => {
                e.insert("start".into(), json!(text));
                zone_name = zone.as_ref().and_then(|z| z.name().map(str::to_owned));
            }
            None => {}
        }
        match (c.get("DTEND").and_then(|p| self.when(p)), &start) {
            (Some(When::Date(end)), Some(When::Date(begin))) => {
                // DTEND is the day after the last day.
                let last = end.yesterday().unwrap_or(end);
                e.insert("end_date".into(), json!(last.max(*begin).to_string()));
            }
            (Some(When::Date(end)), _) => {
                e.insert("end_date".into(), json!(end.to_string()));
            }
            (Some(When::Time { text, .. }), _) => {
                e.insert("end".into(), json!(text));
            }
            (None, Some(begin)) => {
                if let Some(end) = c.get("DURATION").and_then(|p| self.after(begin, &p.value)) {
                    e.insert(end.0.into(), json!(end.1));
                } else if let When::Date(d) = begin {
                    e.insert("end_date".into(), json!(d.to_string()));
                }
            }
            (None, None) => {}
        }
        if let Some(name) = zone_name {
            e.insert("time_zone".into(), json!(name));
        }
        if let Some(status) = c.get("STATUS").map(|p| p.text().to_ascii_lowercase())
            && ["tentative", "confirmed", "cancelled"].contains(&status.as_str())
        {
            e.insert("status".into(), json!(status));
        }
        if let Some(org) = c.get("ORGANIZER") {
            e.insert("organizer".into(), J::Object(person(org)));
        }
        let attendees: Vec<J> = c
            .all("ATTENDEE")
            .map(|a| {
                let mut p = person(a);
                if let Some(role) = a.param("ROLE") {
                    p.insert("role".into(), json!(role.to_ascii_lowercase()));
                }
                if let Some(status) = a.param("PARTSTAT") {
                    p.insert("status".into(), json!(status.to_ascii_lowercase()));
                }
                J::Object(p)
            })
            .collect();
        if !attendees.is_empty() {
            e.insert("attendees".into(), J::Array(attendees));
        }
        let excluded: Vec<J> = c
            .all("EXDATE")
            .flat_map(|p| {
                p.list(',')
                    .into_iter()
                    .filter_map(|v| self.when_value(p, &v))
                    .map(|w| json!(w.text()))
                    .collect::<Vec<_>>()
            })
            .collect();
        if !excluded.is_empty() {
            e.insert("excluded".into(), J::Array(excluded));
        }
        Some(J::Object(order(e, EVENT_FIELDS)))
    }

    fn task(&mut self, c: &Component) -> Option<J> {
        let mut t = Map::new();
        self.common(c, &mut t, TASK_DONE)?;
        let mut zone_name = None;
        for (prop, timed, dated) in [
            ("DTSTART", "start", "start_date"),
            ("DUE", "due", "due_date"),
        ] {
            match c.get(prop).and_then(|p| self.when(p)) {
                Some(When::Date(d)) => {
                    t.insert(dated.into(), json!(d.to_string()));
                }
                Some(When::Time { text, zone, .. }) => {
                    t.insert(timed.into(), json!(text));
                    if zone_name.is_none() {
                        zone_name = zone.and_then(|z| z.name().map(str::to_owned));
                    }
                }
                None => {}
            }
        }
        if let Some(name) = zone_name {
            t.insert("time_zone".into(), json!(name));
        }
        if let Some(When::Time { text, .. }) = c.get("COMPLETED").and_then(|p| self.when(p)) {
            t.insert("completed".into(), json!(text));
        }
        if let Some(n) = c
            .get("PERCENT-COMPLETE")
            .and_then(|p| p.text().trim().parse::<i64>().ok())
        {
            t.insert("percent_complete".into(), json!(n.clamp(0, 100)));
        }
        if let Some(n) = c
            .get("PRIORITY")
            .and_then(|p| p.text().trim().parse::<i64>().ok())
        {
            t.insert("priority".into(), json!(n.clamp(0, 9)));
        }
        if let Some(status) = c.get("STATUS").map(|p| p.text().to_ascii_lowercase())
            && ["needs-action", "in-process", "completed", "cancelled"].contains(&status.as_str())
        {
            t.insert("status".into(), json!(status));
        }
        Some(J::Object(order(t, TASK_FIELDS)))
    }

    /// The fields events and tasks share; None when there is no `UID`, which
    /// the formats require (the entry is then left out and counted).
    fn common(&mut self, c: &Component, out: &mut Map<String, J>, done: &[&str]) -> Option<()> {
        let Some(uid) = c
            .get("UID")
            .map(Property::text)
            .filter(|u| !u.trim().is_empty())
        else {
            *self
                .skipped
                .entry(format!("{} without a UID", c.name))
                .or_default() += 1;
            return None;
        };
        out.insert("uid".into(), json!(uid));
        if let Some(id) = c.get("RECURRENCE-ID").and_then(|p| self.when(p)) {
            out.insert("recurrence_id".into(), json!(id.text()));
        }
        for (prop, field) in [
            ("SUMMARY", "summary"),
            ("LOCATION", "location"),
            ("DESCRIPTION", "description"),
            ("URL", "url"),
        ] {
            if let Some(p) = c.get(prop) {
                out.insert(field.into(), json!(p.text()));
            }
        }
        if let Some(rule) = c.get("RRULE") {
            out.insert("recurrence".into(), json!(rule.value.trim()));
        }
        let categories: Vec<J> = c
            .all("CATEGORIES")
            .flat_map(|p| p.list(','))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .map(J::String)
            .collect();
        if !categories.is_empty() {
            out.insert("categories".into(), J::Array(categories));
        }
        if let Some(n) = c
            .get("SEQUENCE")
            .and_then(|p| p.text().trim().parse::<i64>().ok())
        {
            out.insert("sequence".into(), json!(n));
        }
        for (prop, field) in [("CREATED", "created"), ("LAST-MODIFIED", "last_modified")] {
            if let Some(When::Time { text, .. }) = c.get(prop).and_then(|p| self.when(p)) {
                out.insert(field.into(), json!(text));
            }
        }
        for p in &c.properties {
            if !done.contains(&p.name.as_str()) {
                *self.not_kept.entry(p.name.clone()).or_default() += 1;
            }
        }
        self.alarms += c.children.iter().filter(|k| k.name == "VALARM").count();
        Some(())
    }

    /// A date or date-time property, converted.
    fn when(&mut self, p: &Property) -> Option<When> {
        let value = p.value.trim().to_owned();
        self.when_value(p, &value)
    }

    fn when_value(&mut self, p: &Property, value: &str) -> Option<When> {
        let dated = p
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
            || (value.len() == 8 && !value.contains(['T', 't']));
        if dated {
            return timezone::date(value).map(When::Date);
        }
        let (local, utc) = timezone::civil(value)?;
        if utc {
            return Some(When::Time {
                text: timezone::rfc3339(local, jiff::tz::Offset::UTC),
                local,
                zone: None,
            });
        }
        let zone = match p.param("TZID") {
            Some(id) => self.zone(id),
            None => {
                self.floating += 1;
                None
            }
        };
        let offset = match &zone {
            Some(z) => z.offset_at(local),
            None => self
                .local
                .to_ambiguous_zoned(local)
                .compatible()
                .map(|z| z.offset())
                .unwrap_or(jiff::tz::Offset::UTC),
        };
        Some(When::Time {
            text: timezone::rfc3339(local, offset),
            local,
            zone,
        })
    }

    /// The zone a `TZID` names, from the database, or from the file's own
    /// description of it.
    fn zone(&mut self, id: &str) -> Option<Zone> {
        if let Some((name, tz)) = timezone::iana(id) {
            return Some(Zone::Iana(name, tz));
        }
        if !self.unknown_zones.iter().any(|z| z == id) {
            self.unknown_zones.push(id.to_owned());
        }
        self.zones
            .get(id)
            .and_then(Described::from_component)
            .map(Zone::Described)
    }

    /// The end of an entry that starts at `begin` and lasts `duration`
    /// (`P1D`, `PT1H30M`), as the field it goes in and its text.
    fn after(&self, begin: &When, duration: &str) -> Option<(&'static str, String)> {
        let span: jiff::Span = duration.trim().parse().ok()?;
        match begin {
            When::Date(d) => {
                let end = d.checked_add(span).ok()?;
                // The day the entry ends on, not the day after.
                let last = end.yesterday().ok().filter(|l| l >= d).unwrap_or(*d);
                Some(("end_date", last.to_string()))
            }
            When::Time { local, zone, text } => {
                let end = local.checked_add(span).ok()?;
                let offset = match zone {
                    Some(z) => z.offset_at(end),
                    None if text.ends_with('Z') => jiff::tz::Offset::UTC,
                    None => self
                        .local
                        .to_ambiguous_zoned(end)
                        .compatible()
                        .map(|z| z.offset())
                        .unwrap_or(jiff::tz::Offset::UTC),
                };
                Some(("end", timezone::rfc3339(end, offset)))
            }
        }
    }

    fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        for (name, n) in &self.skipped {
            notes.push(tr!("Not read: {name} ({n}).", name = name, n = n));
        }
        if self.alarms > 0 {
            notes.push(tr!("Alarms are not kept ({n}).", n = self.alarms));
        }
        if !self.not_kept.is_empty() {
            let list: Vec<String> = self
                .not_kept
                .iter()
                .map(|(name, n)| format!("{name} ({n})"))
                .collect();
            notes.push(tr!("Properties not kept: {list}.", list = list.join(", ")));
        }
        for zone in &self.unknown_zones {
            notes.push(tr!(
                "The time zone {zone} is not in the time zone database; its times are converted with the file's own description of it.",
                zone = zone
            ));
        }
        if self.floating > 0 {
            notes.push(tr!(
                "{n} times have no time zone; they are taken in this computer's.",
                n = self.floating
            ));
        }
        notes
    }
}

impl When {
    fn text(&self) -> String {
        match self {
            When::Date(d) => d.to_string(),
            When::Time { text, .. } => text.clone(),
        }
    }
}

/// An `ORGANIZER` or `ATTENDEE`: its `CN` as `name`, its address as `email`.
fn person(p: &Property) -> Map<String, J> {
    let mut out = Map::new();
    if let Some(cn) = p.param("CN") {
        out.insert("name".into(), json!(cn));
    }
    let value = p.text();
    let address = value
        .trim()
        .strip_prefix("mailto:")
        .or_else(|| value.trim().strip_prefix("MAILTO:"))
        .unwrap_or(value.trim());
    if !address.is_empty() {
        out.insert("email".into(), json!(address));
    }
    out
}

/// The fields in the schema's order.
fn order(mut map: Map<String, J>, fields: &[&str]) -> Map<String, J> {
    let mut out = Map::new();
    for f in fields {
        if let Some(v) = map.remove(*f) {
            out.insert((*f).to_owned(), v);
        }
    }
    out
}

/// The properties an event's or a task's fields come from, and those passed
/// over without a word (`DTSTAMP`, which a writer sets to when it wrote).
const EVENT_DONE: &[&str] = &[
    "UID",
    "RECURRENCE-ID",
    "SUMMARY",
    "LOCATION",
    "DESCRIPTION",
    "URL",
    "RRULE",
    "CATEGORIES",
    "SEQUENCE",
    "CREATED",
    "LAST-MODIFIED",
    "DTSTAMP",
    "DTSTART",
    "DTEND",
    "DURATION",
    "STATUS",
    "ORGANIZER",
    "ATTENDEE",
    "EXDATE",
];
const TASK_DONE: &[&str] = &[
    "UID",
    "RECURRENCE-ID",
    "SUMMARY",
    "LOCATION",
    "DESCRIPTION",
    "URL",
    "RRULE",
    "CATEGORIES",
    "SEQUENCE",
    "CREATED",
    "LAST-MODIFIED",
    "DTSTAMP",
    "DTSTART",
    "DUE",
    "COMPLETED",
    "PERCENT-COMPLETE",
    "PRIORITY",
    "STATUS",
];

pub(crate) const EVENT_FIELDS: &[&str] = &[
    "uid",
    "recurrence_id",
    "summary",
    "start",
    "end",
    "start_date",
    "end_date",
    "time_zone",
    "location",
    "description",
    "status",
    "categories",
    "organizer",
    "attendees",
    "url",
    "recurrence",
    "excluded",
    "sequence",
    "created",
    "last_modified",
];
pub(crate) const TASK_FIELDS: &[&str] = &[
    "uid",
    "recurrence_id",
    "summary",
    "start",
    "start_date",
    "due",
    "due_date",
    "completed",
    "percent_complete",
    "priority",
    "status",
    "time_zone",
    "location",
    "description",
    "categories",
    "url",
    "recurrence",
    "sequence",
    "created",
    "last_modified",
];

fn text() -> J {
    json!({"type": "string"})
}
fn date_time() -> J {
    json!({"type": "string", "format": "date-time"})
}
fn date() -> J {
    json!({"type": "string", "format": "date"})
}
fn texts() -> J {
    json!({"type": "array", "items": {"type": "string"}})
}

fn events_schema() -> J {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": EntryKind::Events.title(),
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "uid": text(),
                "recurrence_id": text(),
                "summary": text(),
                "start": date_time(),
                "end": date_time(),
                "start_date": date(),
                "end_date": date(),
                "time_zone": text(),
                "location": text(),
                "description": text(),
                "status": {"type": "string", "enum": ["tentative", "confirmed", "cancelled"]},
                "categories": texts(),
                "organizer": {
                    "type": "object",
                    "properties": {"name": text(), "email": text()},
                    "additionalProperties": false
                },
                "attendees": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": text(),
                            "email": text(),
                            "role": text(),
                            "status": text()
                        },
                        "additionalProperties": false
                    }
                },
                "url": text(),
                "recurrence": text(),
                "excluded": texts(),
                "sequence": {"type": "integer", "minimum": 0},
                "created": date_time(),
                "last_modified": date_time()
            },
            "required": ["uid"],
            "additionalProperties": false
        }
    })
}

fn tasks_schema() -> J {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": EntryKind::Tasks.title(),
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "uid": text(),
                "recurrence_id": text(),
                "summary": text(),
                "start": date_time(),
                "start_date": date(),
                "due": date_time(),
                "due_date": date(),
                "completed": date_time(),
                "percent_complete": {"type": "integer", "minimum": 0, "maximum": 100},
                "priority": {"type": "integer", "minimum": 0, "maximum": 9},
                "status": {
                    "type": "string",
                    "enum": ["needs-action", "in-process", "completed", "cancelled"]
                },
                "time_zone": text(),
                "location": text(),
                "description": text(),
                "categories": texts(),
                "url": text(),
                "recurrence": text(),
                "sequence": {"type": "integer", "minimum": 0},
                "created": date_time(),
                "last_modified": date_time()
            },
            "required": ["uid"],
            "additionalProperties": false
        }
    })
}

#[cfg(test)]
#[path = "calendar_tests.rs"]
mod tests;
