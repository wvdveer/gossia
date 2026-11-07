//! Contact files (`.vcf`: vCard 2.1, 3.0 and 4.0) read into a list of
//! contacts with the schema Gossia knows for them (DESIGN.md, Calendars and
//! contacts, 0.4.0).

use std::collections::BTreeMap;

use gossia_i18n::tr;
use serde_json::{Map, Value as J, json};

use crate::calendar::{EntryKind, later_wins};
use crate::contentline::{self, Component, Property};
use crate::timezone;

/// What reading a contacts file gave, and what was not read, for the report.
#[derive(Debug, Clone)]
pub struct ContactsRead {
    pub contacts: Vec<J>,
    pub notes: Vec<String>,
}

/// Reads a `.vcf` file. Refused when the text is not vCard or holds no card.
pub fn read(text: &str) -> Result<ContactsRead, String> {
    let top = contentline::parse(text)?;
    let cards: Vec<&Component> = top.iter().filter(|c| c.name == "VCARD").collect();
    if cards.is_empty() {
        return Err(tr!("there is no contact in it (no BEGIN:VCARD)").to_owned());
    }
    let mut not_kept: BTreeMap<String, usize> = BTreeMap::new();
    let mut pictures = 0;
    let mut partial_dates = 0;
    let mut unnamed = 0;
    let mut contacts = Vec::new();
    for card in cards {
        match contact(card, &mut not_kept, &mut pictures, &mut partial_dates) {
            Some(c) => contacts.push(c),
            None => unnamed += 1,
        }
    }
    let mut notes = Vec::new();
    if unnamed > 0 {
        notes.push(tr!(
            "Not read: contacts without any name, organisation or email address ({n}).",
            n = unnamed
        ));
    }
    if pictures > 0 {
        notes.push(tr!(
            "Pictures, logos, sounds and keys are not kept ({n}).",
            n = pictures
        ));
    }
    if partial_dates > 0 {
        notes.push(tr!(
            "Dates without a year cannot be kept ({n}).",
            n = partial_dates
        ));
    }
    if !not_kept.is_empty() {
        let list: Vec<String> = not_kept.iter().map(|(n, k)| format!("{n} ({k})")).collect();
        notes.push(tr!("Properties not kept: {list}.", list = list.join(", ")));
    }
    notes.extend(later_wins(&mut contacts, EntryKind::Contacts));
    Ok(ContactsRead { contacts, notes })
}

fn contact(
    card: &Component,
    not_kept: &mut BTreeMap<String, usize>,
    pictures: &mut usize,
    partial_dates: &mut usize,
) -> Option<J> {
    let mut c = Map::new();
    let text = |name: &str| {
        card.get(name)
            .map(Property::text)
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    if let Some(uid) = text("UID") {
        c.insert("uid".into(), json!(uid));
    }
    let mut name = Map::new();
    if let Some(n) = card.get("N") {
        let parts = n.list(';');
        for (i, field) in ["family", "given", "additional", "prefix", "suffix"]
            .iter()
            .enumerate()
        {
            if let Some(part) = parts.get(i).map(|p| p.replace(',', " ").trim().to_owned())
                && !part.is_empty()
            {
                name.insert((*field).into(), json!(part));
            }
        }
    }
    let org = card.get("ORG").map(|o| o.list(';'));
    let emails = entries(card, "EMAIL", "address", &[]);
    let full_name = text("FN")
        .or_else(|| {
            let given = name.get("given").and_then(J::as_str).unwrap_or("");
            let family = name.get("family").and_then(J::as_str).unwrap_or("");
            let both = format!("{given} {family}").trim().to_owned();
            (!both.is_empty()).then_some(both)
        })
        .or_else(|| {
            org.as_ref()
                .and_then(|o| o.first())
                .map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .or_else(|| {
            emails
                .first()
                .and_then(|e| e.get("address"))
                .and_then(J::as_str)
                .map(str::to_owned)
        })?;
    c.insert("full_name".into(), json!(full_name));
    if !name.is_empty() {
        c.insert("name".into(), J::Object(name));
    }
    if let Some(nick) = card.get("NICKNAME") {
        let all: Vec<String> = nick
            .list(',')
            .into_iter()
            .map(|s| s.trim().to_owned())
            .collect();
        let joined = all.join(", ");
        if !joined.is_empty() {
            c.insert("nickname".into(), json!(joined));
        }
    }
    if let Some(org) = org {
        let mut parts = org.into_iter().map(|s| s.trim().to_owned());
        if let Some(first) = parts.next().filter(|s| !s.is_empty()) {
            c.insert("organization".into(), json!(first));
        }
        let rest: Vec<String> = parts.filter(|s| !s.is_empty()).collect();
        if !rest.is_empty() {
            c.insert("department".into(), json!(rest.join(", ")));
        }
    }
    for (prop, field) in [("TITLE", "title"), ("ROLE", "role")] {
        if let Some(v) = text(prop) {
            c.insert(field.into(), json!(v));
        }
    }
    // vCard 3.0 writers put the anniversary in an X- property.
    let anniversary = if card.get("ANNIVERSARY").is_some() {
        "ANNIVERSARY"
    } else {
        "X-ANNIVERSARY"
    };
    for (prop, field) in [("BDAY", "birthday"), (anniversary, "anniversary")] {
        if let Some(p) = card.get(prop) {
            let value = p.text();
            let day = value.trim().split(['T', 't']).next().unwrap_or("");
            match timezone::date(day) {
                Some(d) => {
                    c.insert(field.into(), json!(d.to_string()));
                }
                None => *partial_dates += 1,
            }
        }
    }
    if !emails.is_empty() {
        c.insert("emails".into(), J::Array(emails));
    }
    let phones = entries(card, "TEL", "number", &["tel:"]);
    if !phones.is_empty() {
        c.insert("phones".into(), J::Array(phones));
    }
    let addresses: Vec<J> = card
        .all("ADR")
        .map(|a| {
            let mut out = Map::new();
            if let Some(t) = types(a) {
                out.insert("type".into(), json!(t));
            }
            let parts = a.list(';');
            for (i, field) in [
                "po_box",
                "extended",
                "street",
                "locality",
                "region",
                "postal_code",
                "country",
            ]
            .iter()
            .enumerate()
            {
                if let Some(part) = parts.get(i).map(|p| p.trim().to_owned())
                    && !part.is_empty()
                {
                    out.insert((*field).into(), json!(part));
                }
            }
            J::Object(out)
        })
        .collect();
    if !addresses.is_empty() {
        c.insert("addresses".into(), J::Array(addresses));
    }
    let urls: Vec<J> = card
        .all("URL")
        .map(|u| u.text().trim().to_owned())
        .filter(|u| !u.is_empty())
        .map(J::String)
        .collect();
    if !urls.is_empty() {
        c.insert("urls".into(), J::Array(urls));
    }
    if let Some(note) = text("NOTE") {
        c.insert("note".into(), json!(note));
    }
    let categories: Vec<J> = card
        .all("CATEGORIES")
        .flat_map(|p| p.list(','))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(J::String)
        .collect();
    if !categories.is_empty() {
        c.insert("categories".into(), J::Array(categories));
    }
    if let Some(rev) = text("REV") {
        // `20261107T090000Z` or `2026-11-07T09:00:00Z`.
        let compact: String = rev.chars().filter(|ch| *ch != '-' && *ch != ':').collect();
        if let Some((local, utc)) = timezone::civil(&compact)
            && utc
        {
            c.insert(
                "revision".into(),
                json!(timezone::rfc3339(local, jiff::tz::Offset::UTC)),
            );
        }
    }
    for p in &card.properties {
        match p.name.as_str() {
            "PHOTO" | "LOGO" | "SOUND" | "KEY" => *pictures += 1,
            n if DONE.contains(&n) => {}
            n => *not_kept.entry(n.to_owned()).or_default() += 1,
        }
    }
    Some(J::Object(order(c)))
}

/// The `EMAIL` or `TEL` lines as a list of objects: the value in `field`,
/// less a prefix (`tel:`), and the types.
fn entries(card: &Component, prop: &str, field: &str, prefixes: &[&str]) -> Vec<J> {
    card.all(prop)
        .filter_map(|p| {
            let mut value = p.text().trim().to_owned();
            for prefix in prefixes {
                if value.to_ascii_lowercase().starts_with(prefix) {
                    value = value[prefix.len()..].to_owned();
                }
            }
            if value.is_empty() {
                return None;
            }
            let mut out = Map::new();
            out.insert(field.into(), json!(value));
            if let Some(t) = types(p) {
                out.insert("type".into(), json!(t));
            }
            Some(J::Object(out))
        })
        .collect()
}

/// A line's types, lower case and joined with commas (`work,voice`), less
/// those that say nothing to a reader (`internet`, `pref`).
fn types(p: &Property) -> Option<String> {
    let mut all: Vec<String> = Vec::new();
    for t in p.params_all("TYPE") {
        for part in t.split(',') {
            let part = part.trim().to_ascii_lowercase();
            if part.is_empty() || part == "internet" || part == "pref" || part == "x400" {
                continue;
            }
            if !all.contains(&part) {
                all.push(part);
            }
        }
    }
    (!all.is_empty()).then(|| all.join(","))
}

fn order(mut map: Map<String, J>) -> Map<String, J> {
    let mut out = Map::new();
    for f in FIELDS {
        if let Some(v) = map.remove(*f) {
            out.insert((*f).to_owned(), v);
        }
    }
    out
}

/// The properties the fields come from, and those passed over without a
/// word.
const DONE: &[&str] = &[
    "VERSION",
    "PRODID",
    "UID",
    "FN",
    "N",
    "NICKNAME",
    "ORG",
    "TITLE",
    "ROLE",
    "BDAY",
    "ANNIVERSARY",
    "X-ANNIVERSARY",
    "EMAIL",
    "TEL",
    "ADR",
    "URL",
    "NOTE",
    "CATEGORIES",
    "REV",
];

pub(crate) const FIELDS: &[&str] = &[
    "uid",
    "full_name",
    "name",
    "nickname",
    "organization",
    "department",
    "title",
    "role",
    "birthday",
    "anniversary",
    "emails",
    "phones",
    "addresses",
    "urls",
    "note",
    "categories",
    "revision",
];

/// The schema of a list of contacts.
pub(crate) fn schema() -> J {
    let text = || json!({"type": "string"});
    let date = || json!({"type": "string", "format": "date"});
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": EntryKind::Contacts.title(),
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "uid": text(),
                "full_name": text(),
                "name": {
                    "type": "object",
                    "properties": {
                        "family": text(),
                        "given": text(),
                        "additional": text(),
                        "prefix": text(),
                        "suffix": text()
                    },
                    "additionalProperties": false
                },
                "nickname": text(),
                "organization": text(),
                "department": text(),
                "title": text(),
                "role": text(),
                "birthday": date(),
                "anniversary": date(),
                "emails": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"address": text(), "type": text()},
                        "required": ["address"],
                        "additionalProperties": false
                    }
                },
                "phones": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"number": text(), "type": text()},
                        "required": ["number"],
                        "additionalProperties": false
                    }
                },
                "addresses": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": text(),
                            "po_box": text(),
                            "extended": text(),
                            "street": text(),
                            "locality": text(),
                            "region": text(),
                            "postal_code": text(),
                            "country": text()
                        },
                        "additionalProperties": false
                    }
                },
                "urls": {"type": "array", "items": {"type": "string"}},
                "note": text(),
                "categories": {"type": "array", "items": {"type": "string"}},
                "revision": {"type": "string", "format": "date-time"}
            },
            "required": ["full_name"],
            "additionalProperties": false
        }
    })
}

#[cfg(test)]
#[path = "vcard_tests.rs"]
mod tests;
