use super::*;
use crate::calendar::data;
use crate::validate::validate;

const VCF: &str = "BEGIN:VCARD\r
VERSION:3.0\r
UID:urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1\r
FN:Dr. Ada Lovelace\r
N:Lovelace;Ada;Augusta;Dr.;\r
NICKNAME:Ada,Countess\r
ORG:Analytical Engines Ltd;Research;Notes\r
TITLE:Analyst\r
BDAY:1815-12-10\r
EMAIL;TYPE=INTERNET,WORK,pref:ada@example.com\r
EMAIL;TYPE=HOME:ada@home.example\r
TEL;TYPE=CELL:+61 400 000 000\r
ADR;TYPE=WORK:;Suite 1;12 Engine St;Brisbane;QLD;4000;Australia\r
URL:https://example.com/ada\r
NOTE:Wrote the first\\, published program.\r
CATEGORIES:Friends,Work\r
REV:2026-11-07T09:00:00Z\r
PHOTO;ENCODING=b;TYPE=JPEG:AAAA\r
X-SOCIALPROFILE:ada\r
END:VCARD\r
BEGIN:VCARD\r
VERSION:2.1\r
N:Babbage;Charles\r
TEL;WORK;VOICE:123\r
BDAY:--1226\r
END:VCARD\r
BEGIN:VCARD\r
VERSION:4.0\r
EMAIL:only@example.com\r
TEL;VALUE=uri;TYPE=home:tel:+61-7-0000-0000\r
END:VCARD\r
BEGIN:VCARD\r
VERSION:3.0\r
NOTE:nothing to name it by\r
END:VCARD\r
";

#[test]
fn contacts_are_read_into_their_fields() {
    let read = read(VCF).unwrap();
    assert_eq!(read.contacts.len(), 3);
    assert_eq!(
        read.contacts[0],
        json!({
            "uid": "urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1",
            "full_name": "Dr. Ada Lovelace",
            "name": {"family": "Lovelace", "given": "Ada", "additional": "Augusta", "prefix": "Dr."},
            "nickname": "Ada, Countess",
            "organization": "Analytical Engines Ltd",
            "department": "Research, Notes",
            "title": "Analyst",
            "birthday": "1815-12-10",
            "emails": [
                {"address": "ada@example.com", "type": "work"},
                {"address": "ada@home.example", "type": "home"}
            ],
            "phones": [{"number": "+61 400 000 000", "type": "cell"}],
            "addresses": [{
                "type": "work", "extended": "Suite 1", "street": "12 Engine St",
                "locality": "Brisbane", "region": "QLD", "postal_code": "4000", "country": "Australia"
            }],
            "urls": ["https://example.com/ada"],
            "note": "Wrote the first, published program.",
            "categories": ["Friends", "Work"],
            "revision": "2026-11-07T09:00:00Z"
        })
    );
}

#[test]
fn older_and_newer_cards_and_names_made_up() {
    let read = read(VCF).unwrap();
    let babbage = &read.contacts[1];
    assert_eq!(babbage["full_name"], "Charles Babbage");
    assert_eq!(
        babbage["phones"],
        json!([{"number": "123", "type": "work,voice"}])
    );
    assert!(babbage.get("birthday").is_none());
    let only = &read.contacts[2];
    assert_eq!(only["full_name"], "only@example.com");
    assert_eq!(
        only["phones"],
        json!([{"number": "+61-7-0000-0000", "type": "home"}])
    );
}

#[test]
fn the_report_names_what_was_left_out() {
    let notes = read(VCF).unwrap().notes.join("\n");
    assert!(notes.contains("without any name"), "{notes}");
    assert!(notes.contains("Pictures"), "{notes}");
    assert!(
        notes.contains("without a year cannot be kept (1)"),
        "{notes}"
    );
    assert!(notes.contains("X-SOCIALPROFILE (1)"), "{notes}");
    assert!(!notes.contains("VERSION"), "{notes}");
}

#[test]
fn what_is_read_fits_the_schema() {
    let read = read(VCF).unwrap();
    let spec = crate::jsonschema::lower(&EntryKind::Contacts.schema()).unwrap();
    let issues = validate(&spec, &data(&read.contacts));
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn text_that_is_not_vcard_is_refused() {
    assert!(read("BEGIN:VCALENDAR\nEND:VCALENDAR\n").is_err());
}
