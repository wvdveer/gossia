use super::*;

fn at(s: &str) -> DateTime {
    civil(s).unwrap().0
}

#[test]
fn zone_names_are_found_three_ways() {
    assert_eq!(iana("Australia/Brisbane").unwrap().0, "Australia/Brisbane");
    assert_eq!(
        iana("AUS Eastern Standard Time").unwrap().0,
        "Australia/Sydney"
    );
    assert_eq!(
        iana("/mozilla.org/20050126_1/Europe/Berlin").unwrap().0,
        "Europe/Berlin"
    );
    assert!(iana("Nowhere Standard Time").is_none());
}

#[test]
fn the_windows_table_is_sorted_for_its_search() {
    assert!(WINDOWS_ZONES.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn times_take_the_offset_of_their_date() {
    let (_, sydney) = iana("Australia/Sydney").unwrap();
    let zone = Zone::Iana("Australia/Sydney".into(), sydney);
    // Daylight saving in January, not in July.
    let summer = at("20260115T090000");
    let winter = at("20260715T090000");
    assert_eq!(
        rfc3339(summer, zone.offset_at(summer)),
        "2026-01-15T09:00:00+11:00"
    );
    assert_eq!(
        rfc3339(winter, zone.offset_at(winter)),
        "2026-07-15T09:00:00+10:00"
    );
    assert_eq!(rfc3339(winter, Offset::UTC), "2026-07-15T09:00:00Z");
    let india = Offset::from_seconds(5 * 3600 + 1800).unwrap();
    assert_eq!(rfc3339(winter, india), "2026-07-15T09:00:00+05:30");
}

#[test]
fn a_zone_the_file_describes_follows_its_rules() {
    let text = "BEGIN:VTIMEZONE\nTZID:Made Up\nBEGIN:STANDARD\nDTSTART:19701025T030000\nTZOFFSETFROM:+0200\nTZOFFSETTO:+0100\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\nEND:STANDARD\nBEGIN:DAYLIGHT\nDTSTART:19700329T020000\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0200\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\nEND:DAYLIGHT\nEND:VTIMEZONE\n";
    let parsed = crate::contentline::parse(text).unwrap();
    let zone = Described::from_component(&parsed[0]).unwrap();
    assert_eq!(zone.offset_at(at("20260715T120000")).seconds(), 7200);
    assert_eq!(zone.offset_at(at("20260115T120000")).seconds(), 3600);
    // 2026: last Sunday of March is the 29th, of October the 25th.
    assert_eq!(zone.offset_at(at("20260329T010000")).seconds(), 3600);
    assert_eq!(zone.offset_at(at("20260329T030000")).seconds(), 7200);
    assert_eq!(zone.offset_at(at("20261025T040000")).seconds(), 3600);
}

#[test]
fn dates_and_times_are_read_strictly() {
    assert_eq!(date("20261107").unwrap().to_string(), "2026-11-07");
    assert_eq!(date("2026-11-07").unwrap().to_string(), "2026-11-07");
    assert!(date("--1107").is_none());
    assert!(date("20261340").is_none());
    assert!(civil("20261107T090000Z").unwrap().1);
    assert!(civil("20261107").is_none());
}
