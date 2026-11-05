//! The EBCDIC code pages a copybook's text can be in (DESIGN.md, "COBOL
//! copybooks"): 037 by default.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::tables;

/// An EBCDIC code page.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub enum CodePage {
    /// USA, Canada and others.
    #[default]
    Cp037,
    /// 037 with the euro sign.
    Cp1140,
    /// International.
    Cp500,
    /// Germany and Austria.
    Cp273,
    /// 273 with the euro sign.
    Cp1141,
    /// United Kingdom.
    Cp285,
    /// France.
    Cp297,
}

impl CodePage {
    pub const ALL: [CodePage; 7] = [
        CodePage::Cp037,
        CodePage::Cp1140,
        CodePage::Cp500,
        CodePage::Cp273,
        CodePage::Cp1141,
        CodePage::Cp285,
        CodePage::Cp297,
    ];

    /// Its number, as the schema tab holds it.
    pub fn code(self) -> &'static str {
        match self {
            CodePage::Cp037 => "037",
            CodePage::Cp1140 => "1140",
            CodePage::Cp500 => "500",
            CodePage::Cp273 => "273",
            CodePage::Cp1141 => "1141",
            CodePage::Cp285 => "285",
            CodePage::Cp297 => "297",
        }
    }

    pub fn from_code(code: &str) -> Option<CodePage> {
        let code = code
            .trim()
            .trim_start_matches("CP")
            .trim_start_matches("cp");
        let code = if code == "37" { "037" } else { code };
        CodePage::ALL.into_iter().find(|p| p.code() == code)
    }

    /// What it is called in the window.
    pub fn label(self) -> &'static str {
        match self {
            CodePage::Cp037 => "037 (USA, Canada)",
            CodePage::Cp1140 => "1140 (USA, Canada, with euro)",
            CodePage::Cp500 => "500 (International)",
            CodePage::Cp273 => "273 (Germany, Austria)",
            CodePage::Cp1141 => "1141 (Germany, Austria, with euro)",
            CodePage::Cp285 => "285 (United Kingdom)",
            CodePage::Cp297 => "297 (France)",
        }
    }

    fn table(self) -> &'static [u16; 256] {
        match self {
            CodePage::Cp037 => &tables::CP037,
            CodePage::Cp1140 => &tables::CP1140,
            CodePage::Cp500 => &tables::CP500,
            CodePage::Cp273 => &tables::CP273,
            CodePage::Cp1141 => &tables::CP1141,
            CodePage::Cp285 => &tables::CP285,
            CodePage::Cp297 => &tables::CP297,
        }
    }

    /// The character of a byte.
    pub fn decode_byte(self, byte: u8) -> char {
        char::from_u32(u32::from(self.table()[usize::from(byte)])).unwrap_or('\u{FFFD}')
    }

    pub fn decode(self, bytes: &[u8]) -> String {
        bytes.iter().map(|b| self.decode_byte(*b)).collect()
    }

    /// The byte of a character, if the code page has it.
    pub fn encode_char(self, c: char) -> Option<u8> {
        static REVERSE: OnceLock<Vec<HashMap<char, u8>>> = OnceLock::new();
        let maps = REVERSE.get_or_init(|| {
            CodePage::ALL
                .iter()
                .map(|page| {
                    (0..=255u8)
                        .map(|b| (page.decode_byte(b), b))
                        .collect::<HashMap<char, u8>>()
                })
                .collect()
        });
        let index = CodePage::ALL.iter().position(|p| *p == self)?;
        maps[index].get(&c).copied()
    }

    /// The bytes of a text, or the first character the code page lacks.
    pub fn encode(self, text: &str) -> Result<Vec<u8>, char> {
        text.chars().map(|c| self.encode_char(c).ok_or(c)).collect()
    }

    /// The byte of a space.
    pub fn space(self) -> u8 {
        0x40
    }
}
