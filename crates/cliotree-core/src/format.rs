//! How a form shows a number or a date (DESIGN.md, "Forms", "Number and date
//! formats"). A format changes only what is shown: the value in the tab, and
//! what is typed to change it, are as before.

use chrono::{Datelike, NaiveDate};

use crate::form::Numbering;
use crate::{DateTimeValue, Kind, StringFormat, Value};

/// The most decimal places a format may ask for.
pub const MAX_DECIMALS: u32 = 10;

/// Date patterns the property page offers. Any other pattern can be typed.
pub const DATE_PRESETS: [&str; 7] = [
    "yyyy-mm-dd",
    "dd/mm/yyyy",
    "mm/dd/yyyy",
    "d mmm yyyy",
    "d mmmm yyyy",
    "ddd d mmm yyyy",
    "dddd d mmmm yyyy",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// A number with a thousands separator, a fixed number of decimal places, or
/// both. None if the value is not a number, or the format asks for nothing.
pub fn number(value: &Value, thousands: bool, decimals: Option<u32>) -> Option<String> {
    if !thousands && decimals.is_none() {
        return None;
    }
    let decimals = decimals.map(|d| d.min(MAX_DECIMALS) as usize);
    let text = match (value, decimals) {
        (Value::Int(n), None) => n.to_string(),
        (Value::Int(n), Some(0)) => n.to_string(),
        (Value::Int(n), Some(d)) => format!("{n}.{}", "0".repeat(d)),
        (Value::Float(x), _) if !x.is_finite() => return None,
        (Value::Float(x), Some(d)) => format!("{x:.d$}"),
        (Value::Float(x), None) => python_float(*x),
        _ => return None,
    };
    Some(if thousands { grouped(&text) } else { text })
}

/// A float as Python writes it, as the grid shows it: `2.0`, `0.21`, `1e-07`.
fn python_float(x: f64) -> String {
    if x == x.trunc() && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

/// Commas between the thousands of the whole part. A number in exponent form
/// is left as it is.
fn grouped(text: &str) -> String {
    if text.contains(['e', 'E']) {
        return text.to_owned();
    }
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text),
    };
    let (whole, fraction) = match rest.find('.') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let mut out = String::with_capacity(text.len() + whole.len() / 3);
    out.push_str(sign);
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out.push_str(fraction);
    out
}

/// A date in a pattern. Runs of `d`, `m` and `y` (in either case) are the
/// day, the month and the year; anything else is copied.
/// - `d` 7, `dd` 07, `ddd` Mon, `dddd` Monday
/// - `m` 3, `mm` 03, `mmm` Mar, `mmmm` March
/// - `yy` 26, `yyyy` 2026
pub fn date(date: NaiveDate, pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].to_ascii_lowercase();
        let mut run = 1;
        while i + run < chars.len() && chars[i + run].to_ascii_lowercase() == c {
            run += 1;
        }
        match c {
            'd' => match run {
                1 => out.push_str(&date.day().to_string()),
                2 => out.push_str(&format!("{:02}", date.day())),
                3 => out.push_str(&DAYS[date.weekday().num_days_from_monday() as usize][..3]),
                _ => out.push_str(DAYS[date.weekday().num_days_from_monday() as usize]),
            },
            'm' => {
                let month = date.month0() as usize;
                match run {
                    1 => out.push_str(&(month + 1).to_string()),
                    2 => out.push_str(&format!("{:02}", month + 1)),
                    3 => out.push_str(&MONTHS[month][..3]),
                    _ => out.push_str(MONTHS[month]),
                }
            }
            'y' => match run {
                1 | 2 => out.push_str(&format!("{:02}", date.year().rem_euclid(100))),
                _ => out.push_str(&date.year().to_string()),
            },
            _ => {
                out.extend(&chars[i..i + run]);
            }
        }
        i += run;
    }
    out
}

/// A date and time: the date in the pattern, then the time as it is stored
/// (with its offset, if it has one).
pub fn date_time(value: &DateTimeValue, pattern: &str) -> String {
    match value {
        DateTimeValue::Naive(n) => {
            format!("{} {}", date(n.date(), pattern), n.format("%H:%M:%S"))
        }
        DateTimeValue::Zoned(z) => {
            format!(
                "{} {}",
                date(z.date_naive(), pattern),
                z.format("%H:%M:%S%:z")
            )
        }
    }
}

/// What a value looks like in the format, or None if the format does not
/// apply to it (and it is shown as it is).
pub fn formatted(
    value: &Value,
    thousands: bool,
    decimals: Option<u32>,
    date_format: Option<&str>,
) -> Option<String> {
    match value {
        Value::Int(_) | Value::Float(_) => number(value, thousands, decimals),
        Value::Date(d) => date_format.map(|p| date(*d, p)),
        Value::DateTime(dt) => date_format.map(|p| date_time(dt, p)),
        // A JSON Schema date is a string in ISO 8601.
        Value::Str(s) => {
            let pattern = date_format?;
            if s.len() == 10
                && let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d")
            {
                return Some(date(d, pattern));
            }
            let zoned = chrono::DateTime::parse_from_rfc3339(s).ok()?;
            Some(date_time(&DateTimeValue::Zoned(zoned), pattern))
        }
        _ => None,
    }
}

/// A row number in a numbering. A number the numbering cannot write (a
/// letter or a Roman numeral below 1, a Roman numeral over 3999) is written in
/// digits.
pub fn sequence(n: i64, numbering: Numbering) -> String {
    match numbering {
        Numbering::Index => format!("[{n}]"),
        Numbering::Decimal => n.to_string(),
        Numbering::LowerAlpha | Numbering::UpperAlpha if n >= 1 => {
            // Bijective base 26: a ... z, aa ... az, ba ...
            let mut n = n;
            let mut letters = Vec::new();
            while n > 0 {
                n -= 1;
                letters.push(b'a' + (n % 26) as u8);
                n /= 26;
            }
            letters.reverse();
            let text = String::from_utf8(letters).expect("ASCII letters");
            if numbering == Numbering::UpperAlpha {
                text.to_ascii_uppercase()
            } else {
                text
            }
        }
        Numbering::LowerRoman | Numbering::UpperRoman if (1..=3999).contains(&n) => {
            const NUMERALS: [(i64, &str); 13] = [
                (1000, "M"),
                (900, "CM"),
                (500, "D"),
                (400, "CD"),
                (100, "C"),
                (90, "XC"),
                (50, "L"),
                (40, "XL"),
                (10, "X"),
                (9, "IX"),
                (5, "V"),
                (4, "IV"),
                (1, "I"),
            ];
            let mut n = n;
            let mut text = String::new();
            for (value, numeral) in NUMERALS {
                while n >= value {
                    text.push_str(numeral);
                    n -= value;
                }
            }
            if numbering == Numbering::LowerRoman {
                text.to_ascii_lowercase()
            } else {
                text
            }
        }
        _ => n.to_string(),
    }
}

/// Whether values of this type are numbers, which take a number format.
pub fn is_number(kind: &Kind) -> bool {
    matches!(kind, Kind::Integer(_) | Kind::Number(_))
}

/// Whether values of this type are dates or date-times, which take a date
/// format: an XSD date, or a JSON Schema string of `format: date`.
pub fn is_date(kind: &Kind) -> bool {
    match kind {
        Kind::Date | Kind::DateTime => true,
        Kind::String(c) => matches!(c.format, Some(StringFormat::Date | StringFormat::DateTime)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_takes_a_separator_and_decimal_places() {
        let n = |v: Value, t, d| number(&v, t, d);
        assert_eq!(
            n(Value::Int(1234567), true, None).as_deref(),
            Some("1,234,567")
        );
        assert_eq!(
            n(Value::Int(-1234), true, Some(2)).as_deref(),
            Some("-1,234.00")
        );
        assert_eq!(n(Value::Int(999), true, None).as_deref(), Some("999"));
        assert_eq!(
            n(Value::Float(1234.5), false, Some(2)).as_deref(),
            Some("1234.50")
        );
        assert_eq!(
            n(Value::Float(62.925), false, Some(0)).as_deref(),
            Some("63")
        );
        assert_eq!(
            n(Value::Float(1234.5), true, None).as_deref(),
            Some("1,234.5")
        );
        assert_eq!(
            n(Value::Float(-0.5), true, Some(1)).as_deref(),
            Some("-0.5")
        );
        assert_eq!(
            n(Value::Float(1e20), true, None).as_deref(),
            Some("100,000,000,000,000,000,000")
        );
        // Nothing asked for, or not a number: no format.
        assert_eq!(n(Value::Int(5), false, None), None);
        assert_eq!(n(Value::Str("5".into()), true, None), None);
        assert_eq!(n(Value::Float(f64::NAN), true, Some(2)), None);
    }

    #[test]
    fn row_numbers_are_written_in_their_numbering() {
        let s = sequence;
        assert_eq!(s(0, Numbering::Index), "[0]");
        assert_eq!(s(-3, Numbering::Decimal), "-3");
        assert_eq!(s(1, Numbering::LowerAlpha), "a");
        assert_eq!(s(26, Numbering::LowerAlpha), "z");
        assert_eq!(s(27, Numbering::UpperAlpha), "AA");
        assert_eq!(s(703, Numbering::LowerAlpha), "aaa");
        assert_eq!(s(4, Numbering::LowerRoman), "iv");
        assert_eq!(s(1994, Numbering::UpperRoman), "MCMXCIV");
        assert_eq!(s(3999, Numbering::UpperRoman), "MMMCMXCIX");
        // What a numbering cannot write is in digits.
        assert_eq!(s(0, Numbering::LowerAlpha), "0");
        assert_eq!(s(0, Numbering::UpperRoman), "0");
        assert_eq!(s(4000, Numbering::UpperRoman), "4000");
    }

    #[test]
    fn a_date_follows_its_pattern() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 7).unwrap();
        assert_eq!(date(d, "yyyy-mm-dd"), "2026-03-07");
        assert_eq!(date(d, "dd/mm/yyyy"), "07/03/2026");
        assert_eq!(date(d, "m/d/yy"), "3/7/26");
        assert_eq!(date(d, "d mmm yyyy"), "7 Mar 2026");
        assert_eq!(date(d, "dddd d MMMM yyyy"), "Saturday 7 March 2026");
        assert_eq!(date(d, "ddd, d.m."), "Sat, 7.3.");
        for preset in DATE_PRESETS {
            assert!(!date(d, preset).is_empty());
        }
    }

    #[test]
    fn a_date_time_keeps_its_time() {
        let n = NaiveDate::from_ymd_opt(2026, 3, 7)
            .unwrap()
            .and_hms_opt(9, 5, 0)
            .unwrap();
        assert_eq!(
            date_time(&DateTimeValue::Naive(n), "d mmm yyyy"),
            "7 Mar 2026 09:05:00"
        );
        let value = Value::DateTime(DateTimeValue::Naive(n));
        assert_eq!(formatted(&value, false, None, None), None);
        // A JSON date is a string.
        let text = |s: &str| formatted(&Value::Str(s.into()), false, None, Some("dd/mm/yyyy"));
        assert_eq!(text("2026-03-07").as_deref(), Some("07/03/2026"));
        assert_eq!(
            text("2026-03-07T09:05:00Z").as_deref(),
            Some("07/03/2026 09:05:00+00:00")
        );
        assert_eq!(text("next week"), None);
    }
}
