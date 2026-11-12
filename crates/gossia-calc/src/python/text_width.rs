//! How wide a text is in Inter, the charts' font, for Gossia pyplot's
//! measuring of labels and legends (DESIGN.md, Gossia pyplot). The widths are
//! read from the font when building (build.rs, `text_widths`); there is no
//! shaping or kerning, which is close enough to keep labels apart.

/// The width of each character of a range, from its first, in thousandths
/// of an em; 0 where the font has no such character.
const WIDTHS: &[(u32, &[u16])] = include!(concat!(env!("OUT_DIR"), "/text_widths.rs"));

/// Characters Inter lacks, and those past the ranges: wide for the
/// ideographs and syllables of East Asian scripts, an average Latin width
/// for the rest.
fn fallback(c: char) -> f64 {
    let code = u32::from(c);
    let wide = matches!(code,
        0x1100..=0x115f | 0x2e80..=0xa4cf | 0xac00..=0xd7a3 | 0xf900..=0xfaff
        | 0xfe30..=0xfe4f | 0xff00..=0xff60 | 0xffe0..=0xffe6 | 0x20000..=0x3fffd);
    if wide { 1.0 } else { 0.6 }
}

/// The width of `text` in ems of the font size.
pub(crate) fn em(text: &str) -> f64 {
    text.chars()
        .map(|c| {
            let code = u32::from(c);
            WIDTHS
                .iter()
                .find_map(|&(first, widths)| {
                    let i = code.checked_sub(first)? as usize;
                    widths.get(i).copied()
                })
                .filter(|&w| w > 0)
                .map_or_else(|| fallback(c), |w| f64::from(w) / 1000.0)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::em;

    #[test]
    fn widths_come_from_the_font() {
        // About half an em for a digit, a space narrower, and widths add up.
        let digit = em("0");
        assert!((0.5..0.7).contains(&digit), "{digit}");
        assert!((em("00") - 2.0 * digit).abs() < 1e-9);
        assert!(em(" ") < digit);
        assert!(em("W") > em("i"));
        assert_eq!(em(""), 0.0);
        // The minus sign is in the table; an ideograph is a whole em.
        assert!(em("\u{2212}") > 0.0);
        assert_eq!(em("\u{6708}"), 1.0);
    }
}
