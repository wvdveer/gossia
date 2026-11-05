//! Bytes as text (DESIGN.md, "COBOL copybooks"): a hexadecimal dump with the
//! text beside it, a stream of hex digits, and Base64. Used for copying and
//! pasting COBOL records, and by the bytes value.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::copybook::CodePage;

/// How bytes are written as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BinaryText {
    /// Sixteen bytes a line with the offset first and the text in the code
    /// page after, each record headed by its number when there are several:
    /// what the Raw view shows.
    #[default]
    Dump,
    /// Two hex digits a byte, with no spaces: `F0F1C1`.
    Hex,
    /// Base64, as RFC 4648 has it, with padding.
    Base64,
}

impl BinaryText {
    pub const ALL: [BinaryText; 3] = [BinaryText::Dump, BinaryText::Hex, BinaryText::Base64];

    pub fn key(self) -> &'static str {
        match self {
            BinaryText::Dump => "hex_dump",
            BinaryText::Hex => "hex",
            BinaryText::Base64 => "base64",
        }
    }

    pub fn from_key(key: &str) -> Option<BinaryText> {
        BinaryText::ALL.into_iter().find(|b| b.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            BinaryText::Dump => "Hex dump, as the Raw view shows it",
            BinaryText::Hex => "Hex digits",
            BinaryText::Base64 => "Base64",
        }
    }

    /// The bytes as text of this kind; a dump is of records `record` bytes
    /// long, with their text in `page`.
    pub fn write(self, bytes: &[u8], record: usize, page: CodePage) -> String {
        match self {
            BinaryText::Dump => dump(bytes, record, page, usize::MAX),
            BinaryText::Hex => to_hex(bytes),
            BinaryText::Base64 => STANDARD.encode(bytes),
        }
    }

    /// Reads text of this kind back into bytes, or None when it is not.
    pub fn read(self, text: &str) -> Option<Vec<u8>> {
        match self {
            BinaryText::Dump => read_dump(text),
            BinaryText::Hex => from_hex(text),
            BinaryText::Base64 => from_base64(text),
        }
    }

    /// Reads text that is one of the kinds, with the kind it is. A dump is
    /// tried first, then hex digits, then Base64: hex digits are Base64 too,
    /// but Base64 written from bytes is almost never only hex digits.
    pub fn read_any(text: &str) -> Option<(BinaryText, Vec<u8>)> {
        BinaryText::ALL
            .into_iter()
            .find_map(|kind| kind.read(text).map(|bytes| (kind, bytes)))
    }
}

/// A dump of at most `limit` bytes, the rest counted at the end.
pub fn dump(bytes: &[u8], record: usize, page: CodePage, limit: usize) -> String {
    let record = record.max(1);
    let mut out = String::new();
    let shown = &bytes[..bytes.len().min(limit)];
    let several = bytes.len() > record;
    for (n, chunk) in shown.chunks(record).enumerate() {
        if several {
            if n > 0 {
                out.push('\n');
            }
            out.push_str(&format!("Record {n}\n"));
        }
        for (i, line) in chunk.chunks(16).enumerate() {
            let offset = n * record + i * 16;
            out.push_str(&format!("{offset:08X}  "));
            for k in 0..16 {
                match line.get(k) {
                    Some(b) => out.push_str(&format!("{b:02X} ")),
                    None => out.push_str("   "),
                }
                if k == 7 {
                    out.push(' ');
                }
            }
            out.push(' ');
            out.extend(line.iter().map(|&b| {
                let c = page.decode_byte(b);
                if c.is_control() { '.' } else { c }
            }));
            out.push('\n');
        }
    }
    if bytes.len() > shown.len() {
        out.push_str(&format!(
            "\n... and {} more bytes, not shown\n",
            bytes.len() - shown.len()
        ));
    }
    out.trim_end_matches('\n').to_owned()
}

/// The bytes of a dump: the hex columns of its lines that start with an
/// offset. None when a line is neither a record heading, blank, nor a line
/// of bytes, or the dump was cut short.
fn read_dump(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut lines = 0;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with("Record ") {
            continue;
        }
        let (offset, rest) = (line.get(..8)?, line.get(8..)?);
        if !offset.chars().all(|c| c.is_ascii_hexdigit()) || !rest.starts_with("  ") {
            return None;
        }
        let start = usize::from_str_radix(offset, 16).ok()?;
        if start != bytes.len() {
            return None;
        }
        // The hex columns: 16 of "XX ", with one more space after the eighth.
        let hex = rest.get(2..).unwrap_or_default();
        let hex = hex.get(..hex.len().min(49)).unwrap_or(hex);
        for word in hex.split_whitespace() {
            if word.len() != 2 {
                return None;
            }
            bytes.push(u8::from_str_radix(word, 16).ok()?);
        }
        lines += 1;
    }
    (lines > 0).then_some(bytes)
}

/// Two upper-case hex digits a byte.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// The bytes of hex digits, white space between them passed over. None
/// for anything else, or an odd number of digits.
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// Base64 text.
pub fn to_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// The bytes of Base64 text, white space (line breaks) passed over.
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.is_empty() {
        return None;
    }
    STANDARD.decode(clean).ok()
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod binary_tests;
