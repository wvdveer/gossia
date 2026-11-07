//! Writing tabs of calendar entries as `.ics` (RFC 5545) and of contacts as
//! `.vcf` (vCard 3.0), the other way from [`crate::calendar`] and
//! [`crate::vcard`] (DESIGN.md, Calendars and contacts, 0.4.0).
//!
//! A time with a `time_zone` the database knows is written in that zone,
//! with a `VTIMEZONE` describing it, so that a repeating entry keeps its
//! hour across daylight saving; any other time is written in UTC. Fields of
//! the wrong type (a tab can be edited into anything) are left out.

use std::collections::BTreeMap;

use jiff::civil::Date;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, ToSpan};
use serde_json::{Map, Value as J};

use crate::calendar::EntryKind;
use crate::contentline::{escape, fold};
use crate::timezone;

/// Events or tasks as an iCalendar file; `stamp` is the `DTSTAMP` of each.
pub fn write_calendar(kind: EntryKind, entries: &[J], stamp: Timestamp) -> String {
    let mut writer = CalendarWriter::default();
    let mut body = String::new();
    let component = if kind == EntryKind::Tasks {
        "VTODO"
    } else {
        "VEVENT"
    };
    for entry in entries {
        if let Some(e) = entry.as_object() {
            writer.entry(&mut body, component, e, stamp);
        }
    }
    let mut out = String::new();
    out.push_str(&fold("BEGIN:VCALENDAR"));
    out.push_str(&fold("VERSION:2.0"));
    out.push_str(&fold("PRODID:-//Gossia//Gossia//EN"));
    out.push_str(&fold("CALSCALE:GREGORIAN"));
    for (name, (tz, first, last)) in &writer.zones {
        out.push_str(&vtimezone(name, tz, *first, *last));
    }
    out.push_str(&body);
    out.push_str(&fold("END:VCALENDAR"));
    out
}

/// The same with now as the stamp.
pub fn write_calendar_now(kind: EntryKind, entries: &[J]) -> String {
    write_calendar(kind, entries, Timestamp::now())
}

#[derive(Default)]
struct CalendarWriter {
    /// The zones written in, with the first and last time written in each.
    zones: BTreeMap<String, (TimeZone, Timestamp, Timestamp)>,
}

impl CalendarWriter {
    fn entry(&mut self, out: &mut String, component: &str, e: &Map<String, J>, stamp: Timestamp) {
        let text = |f: &str| e.get(f).and_then(J::as_str).filter(|s| !s.is_empty());
        let Some(uid) = text("uid") else {
            return;
        };
        let zone = text("time_zone").and_then(timezone::iana);
        let mut line = |s: String| out.push_str(&fold(&s));
        line(format!("BEGIN:{component}"));
        line(format!("UID:{}", escape(uid)));
        line(format!("DTSTAMP:{}", utc(stamp)));
        let mut when = |prop: &str, value: &str, line: &mut dyn FnMut(String)| {
            if let Some(s) = self.when(prop, value, zone.as_ref()) {
                line(s);
            }
        };
        if let Some(id) = text("recurrence_id") {
            when("RECURRENCE-ID", id, &mut line);
        }
        for (field, prop) in [
            ("summary", "SUMMARY"),
            ("location", "LOCATION"),
            ("description", "DESCRIPTION"),
        ] {
            if let Some(v) = text(field) {
                line(format!("{prop}:{}", escape(v)));
            }
        }
        if let Some(v) = text("start") {
            when("DTSTART", v, &mut line);
        } else if let Some(d) = text("start_date").and_then(date) {
            line(format!("DTSTART;VALUE=DATE:{}", compact_date(d)));
        }
        if component == "VEVENT" {
            if let Some(v) = text("end") {
                when("DTEND", v, &mut line);
            } else if let Some(d) = text("end_date").and_then(date) {
                // DTEND is the day after the last day.
                let after = d.tomorrow().unwrap_or(d);
                line(format!("DTEND;VALUE=DATE:{}", compact_date(after)));
            }
        } else {
            if let Some(v) = text("due") {
                when("DUE", v, &mut line);
            } else if let Some(d) = text("due_date").and_then(date) {
                line(format!("DUE;VALUE=DATE:{}", compact_date(d)));
            }
            if let Some(ts) = text("completed").and_then(|v| v.parse::<Timestamp>().ok()) {
                line(format!("COMPLETED:{}", utc(ts)));
            }
            for (field, prop, max) in [
                ("percent_complete", "PERCENT-COMPLETE", 100),
                ("priority", "PRIORITY", 9),
            ] {
                if let Some(n) = e.get(field).and_then(J::as_i64) {
                    line(format!("{prop}:{}", n.clamp(0, max)));
                }
            }
        }
        if let Some(s) = text("status") {
            line(format!("STATUS:{}", s.to_ascii_uppercase()));
        }
        if let Some(list) = texts(e.get("categories")) {
            line(format!(
                "CATEGORIES:{}",
                list.iter().map(|c| escape(c)).collect::<Vec<_>>().join(",")
            ));
        }
        if let Some(o) = e.get("organizer").and_then(J::as_object)
            && let Some(s) = person("ORGANIZER", o, &[])
        {
            line(s);
        }
        for a in e
            .get("attendees")
            .and_then(J::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(a) = a.as_object() {
                let extra = [("ROLE", "role"), ("PARTSTAT", "status")];
                if let Some(s) = person("ATTENDEE", a, &extra) {
                    line(s);
                }
            }
        }
        if let Some(v) = text("url") {
            line(format!("URL:{v}"));
        }
        if let Some(v) = text("recurrence") {
            line(format!("RRULE:{}", v.trim().trim_start_matches("RRULE:")));
        }
        for v in texts(e.get("excluded")).unwrap_or_default() {
            when("EXDATE", &v, &mut line);
        }
        if let Some(n) = e.get("sequence").and_then(J::as_i64) {
            line(format!("SEQUENCE:{}", n.max(0)));
        }
        for (field, prop) in [("created", "CREATED"), ("last_modified", "LAST-MODIFIED")] {
            if let Some(ts) = text(field).and_then(|v| v.parse::<Timestamp>().ok()) {
                line(format!("{prop}:{}", utc(ts)));
            }
        }
        line(format!("END:{component}"));
    }

    /// A date or date-time property: a date as such, a time in the entry's
    /// zone when it has one the database knows, and otherwise in UTC.
    fn when(
        &mut self,
        prop: &str,
        value: &str,
        zone: Option<&(String, TimeZone)>,
    ) -> Option<String> {
        if let Some(d) = date(value) {
            return Some(format!("{prop};VALUE=DATE:{}", compact_date(d)));
        }
        let ts = value.parse::<Timestamp>().ok()?;
        match zone {
            Some((name, tz)) => {
                let local = tz.to_datetime(ts);
                let seen = self
                    .zones
                    .entry(name.clone())
                    .or_insert((tz.clone(), ts, ts));
                seen.1 = seen.1.min(ts);
                seen.2 = seen.2.max(ts);
                Some(format!(
                    "{prop};TZID={}:{}",
                    param(name),
                    local.strftime("%Y%m%dT%H%M%S")
                ))
            }
            None => Some(format!("{prop}:{}", utc(ts))),
        }
    }
}

/// An `ORGANIZER` or `ATTENDEE`: `name` as its `CN`, `email` as its address,
/// and the other parameters from their fields, in capitals.
fn person(prop: &str, p: &Map<String, J>, extra: &[(&str, &str)]) -> Option<String> {
    let email = p
        .get("email")
        .and_then(J::as_str)
        .filter(|s| !s.is_empty())?;
    let mut s = prop.to_owned();
    if let Some(name) = p.get("name").and_then(J::as_str).filter(|s| !s.is_empty()) {
        s.push_str(&format!(";CN={}", param(name)));
    }
    for (name, field) in extra {
        if let Some(v) = p.get(*field).and_then(J::as_str).filter(|s| !s.is_empty()) {
            s.push_str(&format!(";{name}={}", param(&v.to_ascii_uppercase())));
        }
    }
    s.push_str(&format!(":mailto:{email}"));
    Some(s)
}

/// A parameter value, quoted when it holds `:`, `;` or `,`; a double quote,
/// which a parameter cannot hold, becomes a single one.
fn param(v: &str) -> String {
    let v: String = v
        .chars()
        .map(|c| if c == '"' { '\'' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    if v.contains([':', ';', ',']) {
        format!("\"{v}\"")
    } else {
        v
    }
}

/// A date alone (`2026-12-25`); jiff would also take the date of a time.
fn date(s: &str) -> Option<Date> {
    (s.len() == 10).then(|| s.parse().ok()).flatten()
}

fn texts(v: Option<&J>) -> Option<Vec<String>> {
    let list: Vec<String> = v?
        .as_array()?
        .iter()
        .filter_map(J::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    (!list.is_empty()).then_some(list)
}

fn utc(ts: Timestamp) -> String {
    TimeZone::UTC
        .to_datetime(ts)
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn compact_date(d: Date) -> String {
    d.strftime("%Y%m%d").to_string()
}

fn offset_text(o: Offset) -> String {
    let secs = o.seconds();
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.unsigned_abs();
    let (h, m, s) = (a / 3600, a / 60 % 60, a % 60);
    if s == 0 {
        format!("{sign}{h:02}{m:02}")
    } else {
        format!("{sign}{h:02}{m:02}{s:02}")
    }
}

/// A kind of change of offset: from, to, whether to daylight saving time,
/// and the abbreviation after it.
type Observance = (Offset, Offset, bool, String);

/// A `VTIMEZONE` for a zone: its changes of offset from the last before
/// `first` to ten years after `last` (repeating entries go on), each kind
/// of change one observance with its dates.
fn vtimezone(name: &str, tz: &TimeZone, first: Timestamp, last: Timestamp) -> String {
    let end = last.checked_add(87_660.hours()).unwrap_or(Timestamp::MAX);
    let mut changes: Vec<(Timestamp, Offset, Offset, bool, String)> = Vec::new();
    let prior = tz.preceding(first).next();
    let mut before = match &prior {
        Some(t) => tz.to_offset(
            t.timestamp()
                .checked_sub(1.second())
                .unwrap_or(t.timestamp()),
        ),
        None => tz.to_offset(first),
    };
    let following = tz.following(first).take_while(|t| t.timestamp() <= end);
    for t in prior.into_iter().chain(following) {
        changes.push((
            t.timestamp(),
            before,
            t.offset(),
            t.dst().is_dst(),
            t.abbreviation().to_owned(),
        ));
        before = t.offset();
    }
    let mut out = String::new();
    out.push_str(&fold("BEGIN:VTIMEZONE"));
    out.push_str(&fold(&format!("TZID:{name}")));
    if changes.is_empty() {
        // A zone without changes: one observance from long ago.
        let info = tz.to_offset_info(first);
        let o = offset_text(info.offset());
        out.push_str(&fold("BEGIN:STANDARD"));
        out.push_str(&fold("DTSTART:16010101T000000"));
        out.push_str(&fold(&format!("TZOFFSETFROM:{o}")));
        out.push_str(&fold(&format!("TZOFFSETTO:{o}")));
        out.push_str(&fold(&format!("TZNAME:{}", escape(info.abbreviation()))));
        out.push_str(&fold("END:STANDARD"));
    }
    // Changes of the same kind share an observance, each date an RDATE.
    let mut groups: Vec<(Observance, Vec<String>)> = Vec::new();
    for (at, from, to, dst, abbr) in changes {
        // The onset in the local time before it.
        let local = from.to_datetime(at).strftime("%Y%m%dT%H%M%S").to_string();
        let key = (from, to, dst, abbr);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, dates)) => dates.push(local),
            None => groups.push((key, vec![local])),
        }
    }
    for ((from, to, dst, abbr), dates) in groups {
        let kind = if dst { "DAYLIGHT" } else { "STANDARD" };
        out.push_str(&fold(&format!("BEGIN:{kind}")));
        out.push_str(&fold(&format!("DTSTART:{}", dates[0])));
        out.push_str(&fold(&format!("TZOFFSETFROM:{}", offset_text(from))));
        out.push_str(&fold(&format!("TZOFFSETTO:{}", offset_text(to))));
        if !abbr.is_empty() {
            out.push_str(&fold(&format!("TZNAME:{}", escape(&abbr))));
        }
        for date in &dates[1..] {
            out.push_str(&fold(&format!("RDATE:{date}")));
        }
        out.push_str(&fold(&format!("END:{kind}")));
    }
    out.push_str(&fold("END:VTIMEZONE"));
    out
}

/// Contacts as a vCard 3.0 file. A contact without a full name is left out,
/// since a card must have one.
pub fn write_contacts(contacts: &[J]) -> String {
    let mut out = String::new();
    for c in contacts.iter().filter_map(J::as_object) {
        let text = |f: &str| c.get(f).and_then(J::as_str).filter(|s| !s.is_empty());
        let Some(full_name) = text("full_name") else {
            continue;
        };
        let mut line = |s: String| out.push_str(&fold(&s));
        line("BEGIN:VCARD".into());
        line("VERSION:3.0".into());
        if let Some(uid) = text("uid") {
            line(format!("UID:{}", escape(uid)));
        }
        line(format!("FN:{}", escape(full_name)));
        let name = c.get("name").and_then(J::as_object);
        let parts: Vec<String> = ["family", "given", "additional", "prefix", "suffix"]
            .iter()
            .map(|f| {
                name.and_then(|n| n.get(*f))
                    .and_then(J::as_str)
                    .map(escape)
                    .unwrap_or_default()
            })
            .collect();
        line(format!("N:{}", parts.join(";")));
        if let Some(nick) = text("nickname") {
            let all: Vec<String> = nick.split(',').map(|s| escape(s.trim())).collect();
            line(format!("NICKNAME:{}", all.join(",")));
        }
        if let Some(org) = text("organization") {
            match text("department") {
                Some(dept) => line(format!("ORG:{};{}", escape(org), escape(dept))),
                None => line(format!("ORG:{}", escape(org))),
            }
        }
        for (field, prop) in [("title", "TITLE"), ("role", "ROLE")] {
            if let Some(v) = text(field) {
                line(format!("{prop}:{}", escape(v)));
            }
        }
        // vCard 3.0 has no anniversary; the X- property is what most
        // programs read.
        for (field, prop) in [("birthday", "BDAY"), ("anniversary", "X-ANNIVERSARY")] {
            if let Some(d) = text(field).and_then(date) {
                line(format!("{prop}:{d}"));
            }
        }
        for (list, prop, field) in [("emails", "EMAIL", "address"), ("phones", "TEL", "number")] {
            for item in c.get(list).and_then(J::as_array).into_iter().flatten() {
                let Some(o) = item.as_object() else { continue };
                let Some(value) = o.get(field).and_then(J::as_str).filter(|s| !s.is_empty()) else {
                    continue;
                };
                line(format!("{prop}{}:{}", types(o), escape(value)));
            }
        }
        for item in c
            .get("addresses")
            .and_then(J::as_array)
            .into_iter()
            .flatten()
        {
            let Some(o) = item.as_object() else { continue };
            let parts: Vec<String> = [
                "po_box",
                "extended",
                "street",
                "locality",
                "region",
                "postal_code",
                "country",
            ]
            .iter()
            .map(|f| {
                o.get(*f)
                    .and_then(J::as_str)
                    .map(escape)
                    .unwrap_or_default()
            })
            .collect();
            if parts.iter().all(String::is_empty) {
                continue;
            }
            line(format!("ADR{}:{}", types(o), parts.join(";")));
        }
        for url in texts(c.get("urls")).unwrap_or_default() {
            line(format!("URL:{url}"));
        }
        if let Some(note) = text("note") {
            line(format!("NOTE:{}", escape(note)));
        }
        if let Some(list) = texts(c.get("categories")) {
            line(format!(
                "CATEGORIES:{}",
                list.iter().map(|s| escape(s)).collect::<Vec<_>>().join(",")
            ));
        }
        if let Some(ts) = text("revision").and_then(|v| v.parse::<Timestamp>().ok()) {
            line(format!("REV:{}", utc(ts)));
        }
        line("END:VCARD".into());
    }
    out
}

/// `;TYPE=work,voice` from a `type` field, or nothing.
fn types(o: &Map<String, J>) -> String {
    match o.get("type").and_then(J::as_str) {
        Some(t) if !t.trim().is_empty() => {
            let list: Vec<String> = t
                .split(',')
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .map(param)
                .collect();
            format!(";TYPE={}", list.join(","))
        }
        _ => String::new(),
    }
}

#[cfg(test)]
#[path = "entries_write_tests.rs"]
mod tests;
