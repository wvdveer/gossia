//! Calendar times and their zones (DESIGN.md, Calendars and contacts,
//! 0.4.0): a zone named in a file, found in the IANA database bundled with
//! Gossia, through the Windows names of CLDR, or from the file's own
//! `VTIMEZONE`; and the civil times of iCalendar (`20261107T090000`) turned
//! into RFC 3339 date-times with an offset.

use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::tz::{Offset, TimeZone};

use crate::contentline::Component;
use crate::windows_zones::WINDOWS_ZONES;

/// A zone as a file names it, found.
#[derive(Debug, Clone)]
pub enum Zone {
    /// A zone of the IANA database, with its name.
    Iana(String, TimeZone),
    /// A zone only the file's `VTIMEZONE` describes.
    Described(Described),
}

impl Zone {
    /// The IANA name, which `time_zone` keeps; None for a described zone.
    pub fn name(&self) -> Option<&str> {
        match self {
            Zone::Iana(name, _) => Some(name),
            Zone::Described(_) => None,
        }
    }

    /// The offset in force at a clock time of the zone.
    pub fn offset_at(&self, local: DateTime) -> Offset {
        match self {
            Zone::Iana(_, tz) => tz
                .to_ambiguous_zoned(local)
                .compatible()
                .map(|z| z.offset())
                .unwrap_or(Offset::UTC),
            Zone::Described(d) => d.offset_at(local),
        }
    }
}

/// The IANA zone a name stands for: the name itself, its IANA part after a
/// prefix (`/mozilla.org/20050126_1/Europe/Berlin`, `/citadel.org/.../Europe/Berlin`),
/// or a Windows name (`AUS Eastern Standard Time`).
pub fn iana(name: &str) -> Option<(String, TimeZone)> {
    let name = name.trim().trim_matches('"');
    if let Ok(tz) = jiff::tz::db().get(name) {
        return Some((tz.iana_name().unwrap_or(name).to_owned(), tz));
    }
    if let Ok(i) = WINDOWS_ZONES.binary_search_by(|(w, _)| (*w).cmp(name)) {
        let iana = WINDOWS_ZONES[i].1;
        return jiff::tz::db()
            .get(iana)
            .ok()
            .map(|tz| (iana.to_owned(), tz));
    }
    // The longest tail of the path that is a zone's name.
    let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
    for start in 1..parts.len() {
        let tail = parts[start..].join("/");
        if let Ok(tz) = jiff::tz::db().get(&tail) {
            return Some((tail, tz));
        }
    }
    None
}

/// A zone a file's `VTIMEZONE` describes: its observances, each starting at
/// a clock time and repeating yearly when it has a rule.
#[derive(Debug, Clone, Default)]
pub struct Described {
    observances: Vec<Observance>,
}

#[derive(Debug, Clone)]
struct Observance {
    start: DateTime,
    offset: Offset,
    /// `FREQ=YEARLY;BYMONTH=10;BYDAY=1SU`: the month and which weekday of it.
    yearly: Option<(i8, i8, Weekday)>,
}

impl Described {
    /// The zone a `VTIMEZONE` describes, from its `STANDARD` and `DAYLIGHT`
    /// parts; None when it has none that can be read.
    pub fn from_component(vtimezone: &Component) -> Option<Described> {
        let mut observances = Vec::new();
        for part in &vtimezone.children {
            if part.name != "STANDARD" && part.name != "DAYLIGHT" {
                continue;
            }
            let start = part.get("DTSTART").and_then(|p| civil(&p.value))?;
            let offset = part.get("TZOFFSETTO").and_then(|p| utc_offset(&p.value))?;
            let yearly = part.get("RRULE").and_then(|p| yearly_rule(&p.value));
            observances.push(Observance {
                start: start.0,
                offset,
                yearly,
            });
        }
        (!observances.is_empty()).then_some(Described { observances })
    }

    /// The offset of the observance that began last before `local`.
    pub fn offset_at(&self, local: DateTime) -> Offset {
        let mut best: Option<(DateTime, Offset)> = None;
        for o in &self.observances {
            let began = match o.yearly {
                None => Some(o.start).filter(|s| *s <= local),
                Some((month, nth, day)) => {
                    // This year's onset, or last year's if it is still to come.
                    [local.year(), local.year() - 1]
                        .into_iter()
                        .filter_map(|y| nth_weekday(y, month, nth, day))
                        .map(|d| d.to_datetime(o.start.time()))
                        .find(|t| *t <= local && *t >= o.start)
                }
            };
            if let Some(t) = began
                && best.is_none_or(|(b, _)| t > b)
            {
                best = Some((t, o.offset));
            }
        }
        best.map(|(_, o)| o)
            .or_else(|| self.observances.first().map(|o| o.offset))
            .unwrap_or(Offset::UTC)
    }
}

/// `FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU` as month, which one (1 to 5, or -1 for
/// the last) and weekday.
fn yearly_rule(rule: &str) -> Option<(i8, i8, Weekday)> {
    let mut month = None;
    let mut by_day = None;
    for part in rule.split(';') {
        let (k, v) = part.split_once('=')?;
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" if !v.eq_ignore_ascii_case("YEARLY") => return None,
            "BYMONTH" => month = v.trim().parse::<i8>().ok(),
            "BYDAY" => by_day = Some(v.trim().to_ascii_uppercase()),
            _ => {}
        }
    }
    let by_day = by_day?;
    let (n, day) = by_day.split_at(by_day.len().checked_sub(2)?);
    let nth = if n.is_empty() {
        1
    } else {
        n.parse::<i8>().ok()?
    };
    Some((month?, nth, weekday(day)?))
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

/// The `nth` given weekday of a month (`-1` the last).
fn nth_weekday(year: i16, month: i8, nth: i8, day: Weekday) -> Option<Date> {
    let first = Date::new(year, month, 1).ok()?;
    first.nth_weekday_of_month(nth, day).ok()
}

/// `+1000` or `-0530` (`+103000` with seconds) as an offset.
fn utc_offset(s: &str) -> Option<Offset> {
    let s = s.trim();
    let (sign, digits) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    if digits.len() != 4 && digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    let sec: i32 = digits.get(4..6).map_or(Some(0), |x| x.parse().ok())?;
    Offset::from_seconds(sign * (h * 3600 + m * 60 + sec)).ok()
}

/// An iCalendar `DATE-TIME` (`20261107T090000`, with `Z` for UTC) as a clock
/// time and whether it is UTC; None for anything else.
pub fn civil(s: &str) -> Option<(DateTime, bool)> {
    let s = s.trim();
    let (body, utc) = match s.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (s, false),
    };
    let (d, t) = body.split_once(['T', 't'])?;
    let date = date(d)?;
    if t.len() < 6 || !t.is_ascii() {
        return None;
    }
    let num = |r: std::ops::Range<usize>| t.get(r)?.parse::<i8>().ok();
    // A leap second (60) is read as 59.
    let time = Time::new(num(0..2)?, num(2..4)?, num(4..6)?.min(59), 0).ok()?;
    Some((date.to_datetime(time), utc))
}

/// An iCalendar `DATE` (`20261107`), or the same with dashes (vCard 4.0
/// allows `2026-11-07`).
pub fn date(s: &str) -> Option<Date> {
    let digits: String = s.trim().chars().filter(|c| *c != '-').collect();
    if digits.len() != 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i16 = digits[0..4].parse().ok()?;
    let m: i8 = digits[4..6].parse().ok()?;
    let d: i8 = digits[6..8].parse().ok()?;
    Date::new(y, m, d).ok()
}

/// A clock time with an offset as RFC 3339, `2026-11-07T09:00:00+10:00`, or
/// `...Z` for UTC.
pub fn rfc3339(local: DateTime, offset: Offset) -> String {
    let base = local.strftime("%Y-%m-%dT%H:%M:%S").to_string();
    if offset == Offset::UTC {
        return format!("{base}Z");
    }
    let secs = offset.seconds();
    let sign = if secs < 0 { '-' } else { '+' };
    let secs = secs.unsigned_abs();
    format!("{base}{sign}{:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

/// The computer's zone, for floating times.
pub fn system() -> TimeZone {
    TimeZone::system()
}

#[cfg(test)]
#[path = "timezone_tests.rs"]
mod tests;
