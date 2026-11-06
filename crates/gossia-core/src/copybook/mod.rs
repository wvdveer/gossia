//! COBOL copybooks (DESIGN.md, "Import and export", "COBOL copybooks"): the
//! third schema language. A `.cpy` file is read into [`Copybook`], which a
//! copybook schema tab holds as a tree under the built-in meta-schema
//! (`builtin:copybook`, see [`to_node`] and [`from_node`]) and which is
//! written back as `.cpy` text by [`to_text`]. [`lower`] turns a copybook
//! into the type of its data tabs and the byte [`Layout`] of a record, and
//! [`records`] reads and writes `.blk` files of fixed-length records.

mod copies;
pub mod ebcdic;
mod lower;
pub mod records;
mod tables;

use gossia_i18n::tr;
use std::fmt;

use indexmap::IndexMap;

use crate::value::{CommentPlace, Node, Value};

pub use copies::parse_with;
pub use ebcdic::CodePage;
pub use lower::{
    Condition, Counter, Element, Layout, NumUsage, Numeric, RedefineSet, SignPlace, Slot,
    SlotOccurs, lower, lower_node, lower_node_record, lower_record, record_names,
};

/// A copybook: its items nested by level, and the comment after the last.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Copybook {
    pub items: Vec<Item>,
    pub end_comment: Option<String>,
    /// How its `COMP-1` and `COMP-2` items hold their numbers: a setting of
    /// the schema tab, as the code page is, which the text does not say.
    pub float: FloatFormat,
}

/// How floating point items (`COMP-1`, `COMP-2`) are held (owner,
/// 2026-11-01: IBM hexadecimal unless the copybook tab says IEEE 754).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum FloatFormat {
    /// IBM hexadecimal floating point (HFP), as z/OS COBOL holds them.
    #[default]
    Hex,
    /// IEEE 754 binary floating point, big-endian (`FLOAT-BINARY`, and
    /// `ARITH(EXTEND)` with `FLOAT(NATIVE)` off z/OS).
    Ieee,
}

impl FloatFormat {
    pub const ALL: [FloatFormat; 2] = [FloatFormat::Hex, FloatFormat::Ieee];

    pub fn code(self) -> &'static str {
        match self {
            FloatFormat::Hex => "hex",
            FloatFormat::Ieee => "ieee",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FloatFormat::Hex => tr!("IBM hexadecimal"),
            FloatFormat::Ieee => "IEEE 754",
        }
    }

    pub fn from_code(code: &str) -> Option<FloatFormat> {
        FloatFormat::ALL.into_iter().find(|f| f.code() == code)
    }
}

/// One data description entry, with the entries below it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub level: u8,
    /// The name as written; `FILLER` for an item without one.
    pub name: String,
    pub picture: Option<String>,
    pub usage: Option<Usage>,
    /// `SYNC`: a binary or floating point item starts at a multiple of its
    /// size from the start of the record.
    pub sync: bool,
    pub sign: Option<Sign>,
    pub occurs: Option<Occurs>,
    pub redefines: Option<String>,
    /// The `VALUE` clause's literal, as written.
    pub value: Option<String>,
    /// Clauses kept as written that do not change the layout (`JUSTIFIED`,
    /// `BLANK WHEN ZERO`, `INDEXED BY`, `KEY IS`).
    pub other: Option<String>,
    /// The comment lines above it.
    pub comment: Option<String>,
    /// Its condition names (level 88).
    pub conditions: Vec<ConditionName>,
    pub items: Vec<Item>,
    /// For a record (level 01), its RENAMES entries (level 66), which come
    /// after its last item.
    pub renames: Vec<Renames>,
}

/// A RENAMES entry (level 66): another name for an item of the record, or
/// for the run of items from one to another.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Renames {
    pub name: String,
    pub from: String,
    pub thru: Option<String>,
    /// The comment lines above it.
    pub comment: Option<String>,
}

/// A condition name (level 88): a name for some values of the item above.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConditionName {
    pub name: String,
    /// The literals after `VALUE`, as written: `'A'`, `'C' 'X'`,
    /// `1 THRU 9`.
    pub values: String,
    /// The comment lines above it.
    pub comment: Option<String>,
}

/// How an item's value is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    Display,
    Comp,
    Comp3,
    Comp4,
    Comp5,
    Binary,
    PackedDecimal,
    /// Floating point, 4 bytes, with no picture.
    Comp1,
    /// Floating point, 8 bytes, with no picture.
    Comp2,
}

impl Usage {
    pub const ALL: [Usage; 9] = [
        Usage::Display,
        Usage::Comp,
        Usage::Comp3,
        Usage::Comp4,
        Usage::Comp5,
        Usage::Binary,
        Usage::PackedDecimal,
        Usage::Comp1,
        Usage::Comp2,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Usage::Display => "DISPLAY",
            Usage::Comp => "COMP",
            Usage::Comp3 => "COMP-3",
            Usage::Comp4 => "COMP-4",
            Usage::Comp5 => "COMP-5",
            Usage::Binary => "BINARY",
            Usage::PackedDecimal => "PACKED-DECIMAL",
            Usage::Comp1 => "COMP-1",
            Usage::Comp2 => "COMP-2",
        }
    }

    /// The usage a word names, with its long spellings.
    pub fn from_word(word: &str) -> Option<Usage> {
        Some(match word {
            "DISPLAY" => Usage::Display,
            "COMP" | "COMPUTATIONAL" => Usage::Comp,
            "COMP-3" | "COMPUTATIONAL-3" => Usage::Comp3,
            "COMP-4" | "COMPUTATIONAL-4" => Usage::Comp4,
            "COMP-5" | "COMPUTATIONAL-5" => Usage::Comp5,
            "BINARY" => Usage::Binary,
            "PACKED-DECIMAL" => Usage::PackedDecimal,
            "COMP-1" | "COMPUTATIONAL-1" => Usage::Comp1,
            "COMP-2" | "COMPUTATIONAL-2" => Usage::Comp2,
            _ => return None,
        })
    }
}

/// A `SIGN` clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sign {
    pub leading: bool,
    pub separate: bool,
}

impl Sign {
    pub const CODES: [&str; 4] = [
        "leading",
        "trailing",
        "leading separate",
        "trailing separate",
    ];

    pub fn code(self) -> &'static str {
        match (self.leading, self.separate) {
            (true, false) => "leading",
            (false, false) => "trailing",
            (true, true) => "leading separate",
            (false, true) => "trailing separate",
        }
    }

    pub fn from_code(code: &str) -> Option<Sign> {
        let n = Sign::CODES.iter().position(|c| *c == code)?;
        Some(Sign {
            leading: n % 2 == 0,
            separate: n >= 2,
        })
    }
}

/// An `OCCURS` clause: `OCCURS max`, or `OCCURS min TO max DEPENDING ON x`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurs {
    pub min: Option<u32>,
    pub max: u32,
    pub depending_on: Option<String>,
}

/// The text is not a copybook Gossia can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

// ------------------------------------------------------------ reading text

/// A word of the code, or a literal in quotes, with its line.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    line: usize,
    /// The comment lines just above it.
    comment: Option<String>,
    /// A period ends the entry after it.
    ends: bool,
}

/// Whether the copybook is in fixed form: columns 1 to 6 hold only a
/// sequence number or spaces, column 7 marks comments and continuations, and
/// the code is in columns 8 to 72.
fn is_fixed(lines: &[&str]) -> bool {
    lines.iter().filter(|l| !l.trim().is_empty()).all(|l| {
        let chars: Vec<char> = l.chars().collect();
        chars
            .iter()
            .take(6)
            .all(|c| c.is_ascii_digit() || *c == ' ')
            && chars
                .get(6)
                .is_none_or(|c| matches!(c, ' ' | '*' | '/' | '-' | 'D' | 'd'))
    })
}

/// Splits the text into tokens, keeping the comment lines with the token
/// after them.
fn tokens(text: &str) -> Result<(Vec<Token>, Option<String>), ParseError> {
    let lines: Vec<&str> = text
        .trim_start_matches('\u{FEFF}')
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .collect();
    let fixed = is_fixed(&lines);
    let mut out: Vec<Token> = Vec::new();
    let mut comments: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let number = i + 1;
        let chars: Vec<char> = line.chars().collect();
        let (indicator, code): (char, String) = if fixed {
            let indicator = chars.get(6).copied().unwrap_or(' ');
            let code = chars.iter().skip(7).take(65).collect();
            (indicator, code)
        } else {
            let trimmed = line.trim_start();
            if trimmed.starts_with('*') && !trimmed.starts_with("*>") {
                ('*', trimmed[1..].to_owned())
            } else {
                (' ', (*line).to_owned())
            }
        };
        if matches!(indicator, '*' | '/') {
            let text = code.trim_end();
            // A line of stars (`**   Text`) is a comment as one star is.
            let text = match text.strip_prefix('*') {
                Some(rest) => rest.trim_start_matches('*').trim_start(),
                None => text.strip_prefix(' ').unwrap_or(text),
            };
            comments.push(text.to_owned());
            continue;
        }
        // An inline comment runs to the end of the line.
        let code = match code.find("*>") {
            Some(at) => {
                let text = code[at + 2..].trim();
                if !text.is_empty() {
                    comments.push(text.to_owned());
                }
                code[..at].to_owned()
            }
            None => code,
        };
        if indicator == '-' {
            continue_literal(&mut out, &code, number)?;
            continue;
        }
        let mut rest = code.as_str();
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            let (text, after) = if rest.starts_with(['\'', '"']) {
                let quote = rest.chars().next().unwrap_or('\'');
                let mut end = None;
                let bytes: Vec<(usize, char)> = rest.char_indices().collect();
                let mut k = 1;
                while k < bytes.len() {
                    if bytes[k].1 == quote {
                        // Two quotes stand for one inside the literal.
                        if bytes.get(k + 1).is_some_and(|b| b.1 == quote) {
                            k += 2;
                            continue;
                        }
                        end = Some(bytes[k].0 + quote.len_utf8());
                        break;
                    }
                    k += 1;
                }
                // A literal not closed on its line goes on on the next one.
                let end = end.unwrap_or(rest.len());
                (&rest[..end], &rest[end..])
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..end], &rest[end..])
            };
            rest = after;
            // A period at the end of a word ends the entry; after a literal
            // it comes as a word of its own.
            let (word, ends) = match text.strip_suffix('.') {
                Some(word) if !text.starts_with(['\'', '"']) => (word, true),
                _ => (text, false),
            };
            if !word.is_empty() {
                out.push(Token {
                    text: word.to_owned(),
                    line: number,
                    comment: (!comments.is_empty()).then(|| comments.join("\n")),
                    ends,
                });
                comments.clear();
            } else if ends && let Some(last) = out.last_mut() {
                last.ends = true;
            }
        }
    }
    let end = (!comments.is_empty()).then(|| comments.join("\n"));
    Ok((out, end))
}

/// A continuation line (`-` in column 7) goes on with the literal of the
/// line before, from just after its first quote.
fn continue_literal(out: &mut [Token], code: &str, line: usize) -> Result<(), ParseError> {
    let Some(last) = out.last_mut() else {
        return Err(ParseError {
            line,
            message: tr!("a continuation line has nothing before it to go on with").into(),
        });
    };
    let code = code.trim();
    let rest = code.strip_prefix(['\'', '"']).unwrap_or(code);
    let (rest, ends) = match rest.strip_suffix('.') {
        Some(r) if r.ends_with(['\'', '"']) => (r, true),
        _ => (rest, false),
    };
    last.text.push_str(rest);
    last.ends |= ends;
    Ok(())
}

/// Reads the text of a copybook. A `COPY` statement in it is refused: see
/// [`parse_with`] for one that copies members in.
pub fn parse(text: &str) -> Result<Copybook, ParseError> {
    parse_with(text, &mut |_| None)
}

/// Reads a copybook from its tokens.
fn parse_tokens(tokens: Vec<Token>, end_comment: Option<String>) -> Result<Copybook, ParseError> {
    let mut entries: Vec<(Item, usize)> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let start = i;
        while i < tokens.len() && !tokens[i].ends {
            i += 1;
        }
        if i == tokens.len() {
            return Err(ParseError {
                line: tokens[start].line,
                message: tr!("this entry has no period at its end").into(),
            });
        }
        let sentence = &tokens[start..=i];
        i += 1;
        let line = sentence[0].line;
        let level: u8 = sentence[0].text.parse().map_err(|_| ParseError {
            line,
            message: tr!(
                "an entry starts with its level number, not {text}",
                text = sentence[0].text
            ),
        })?;
        let comment = sentence[0].comment.clone();
        match level {
            88 => {
                let Some((last, _)) = entries.last_mut() else {
                    return Err(ParseError {
                        line,
                        message: tr!("a level 88 entry must follow an item").into(),
                    });
                };
                last.conditions
                    .push(condition(&sentence[1..], comment, line)?);
                continue;
            }
            66 => {
                // A RENAMES entry belongs to the record before it.
                let renames = renames(&sentence[1..], comment, line)?;
                let at = entries
                    .iter()
                    .rposition(|(e, _)| e.level == 1)
                    .or((!entries.is_empty()).then_some(0))
                    .ok_or_else(|| ParseError {
                        line,
                        message: tr!("a level 66 entry must follow a record").into(),
                    })?;
                entries[at].0.renames.push(renames);
                continue;
            }
            1..=49 | 77 => {}
            _ => {
                return Err(ParseError {
                    line,
                    message: tr!("{level} is not a level number", level = level),
                });
            }
        }
        let mut item = entry(&sentence[1..], line)?;
        item.level = level;
        item.comment = comment;
        entries.push((item, line));
    }
    let items = nest(entries)?;
    Ok(Copybook {
        items,
        end_comment,
        float: FloatFormat::default(),
    })
}

/// A RENAMES entry after its level number: `NAME RENAMES A` or
/// `NAME RENAMES A THRU B`.
fn renames(words: &[Token], comment: Option<String>, line: usize) -> Result<Renames, ParseError> {
    let upper: Vec<String> = words.iter().map(|w| w.text.to_ascii_uppercase()).collect();
    let error = || ParseError {
        line,
        message: tr!("a level 66 entry is NAME RENAMES item, or NAME RENAMES item THRU item")
            .into(),
    };
    let text = |k: usize| words.get(k).map(|t| t.text.clone()).ok_or_else(error);
    if upper.get(1).map(String::as_str) != Some("RENAMES") {
        return Err(error());
    }
    let thru = match upper.get(3).map(String::as_str) {
        None => None,
        Some("THRU" | "THROUGH") if words.len() == 5 => Some(text(4)?),
        Some(_) => return Err(error()),
    };
    Ok(Renames {
        name: text(0)?,
        from: text(2)?,
        thru,
        comment,
    })
}

/// A condition name entry after its level number: the name, `VALUE` (or
/// `VALUES`, with `IS` or `ARE`) and the literals.
fn condition(
    words: &[Token],
    comment: Option<String>,
    line: usize,
) -> Result<ConditionName, ParseError> {
    let (name, rest) = words.split_first().ok_or_else(|| ParseError {
        line,
        message: tr!("a level 88 entry names a condition").into(),
    })?;
    let mut rest = rest;
    for skip in [["VALUE", "VALUES"], ["IS", "ARE"]] {
        if rest
            .first()
            .is_some_and(|w| skip.contains(&w.text.to_ascii_uppercase().as_str()))
        {
            rest = &rest[1..];
        }
    }
    if rest.is_empty() {
        return Err(ParseError {
            line,
            message: tr!("the condition {text} has no VALUE", text = name.text),
        });
    }
    let values: Vec<&str> = rest.iter().map(|t| t.text.as_str()).collect();
    Ok(ConditionName {
        name: name.text.clone(),
        values: values.join(" "),
        comment,
    })
}

/// The clauses of an entry after its level number.
fn entry(words: &[Token], line: usize) -> Result<Item, ParseError> {
    let error = |message: String| ParseError { line, message };
    let upper: Vec<String> = words.iter().map(|w| w.text.to_ascii_uppercase()).collect();
    let mut item = Item::default();
    let mut k = 0;
    let clause_word = |w: &str| {
        matches!(
            w,
            "PIC" | "PICTURE" | "USAGE" | "VALUE" | "VALUES" | "OCCURS" | "REDEFINES" | "SIGN"
        ) || Usage::from_word(w).is_some()
    };
    item.name = match upper.first() {
        Some(w) if !clause_word(w) => {
            k = 1;
            words[0].text.clone()
        }
        _ => "FILLER".to_owned(),
    };
    let mut other: Vec<String> = Vec::new();
    let is = |k: &mut usize, word: &str| {
        if upper.get(*k).is_some_and(|w| w == word) {
            *k += 1;
        }
    };
    while k < upper.len() {
        let word = upper[k].as_str();
        k += 1;
        match word {
            "PIC" | "PICTURE" => {
                is(&mut k, "IS");
                let picture = words
                    .get(k)
                    .ok_or_else(|| error("PIC has no picture after it".into()))?;
                item.picture = Some(picture.text.to_ascii_uppercase());
                k += 1;
            }
            "USAGE" => {
                is(&mut k, "IS");
                let usage = upper
                    .get(k)
                    .ok_or_else(|| error("USAGE has no usage after it".into()))?;
                item.usage = Some(usage_of(usage).map_err(error)?);
                k += 1;
            }
            "REDEFINES" => {
                let name = words
                    .get(k)
                    .ok_or_else(|| error("REDEFINES has no name after it".into()))?;
                item.redefines = Some(name.text.clone());
                k += 1;
            }
            "VALUE" | "VALUES" => {
                is(&mut k, "IS");
                is(&mut k, "ARE");
                let value = words
                    .get(k)
                    .ok_or_else(|| error("VALUE has no literal after it".into()))?;
                let mut text = value.text.clone();
                k += 1;
                // ALL 'x' and figurative constants are words of their own.
                if value.text.eq_ignore_ascii_case("ALL")
                    && let Some(next) = words.get(k)
                {
                    text = format!("{text} {}", next.text);
                    k += 1;
                }
                item.value = Some(text);
            }
            "OCCURS" => {
                let number = |k: usize| -> Result<u32, ParseError> {
                    upper
                        .get(k)
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| error("OCCURS needs a number of times".into()))
                };
                let first = number(k)?;
                k += 1;
                let mut occurs = Occurs {
                    min: None,
                    max: first,
                    depending_on: None,
                };
                if upper.get(k).is_some_and(|w| w == "TO") {
                    occurs.min = Some(first);
                    occurs.max = number(k + 1)?;
                    k += 2;
                }
                is(&mut k, "TIMES");
                if upper.get(k).is_some_and(|w| w == "DEPENDING") {
                    k += 1;
                    is(&mut k, "ON");
                    let name = words
                        .get(k)
                        .ok_or_else(|| error("DEPENDING ON has no name after it".into()))?;
                    occurs.depending_on = Some(name.text.clone());
                    k += 1;
                }
                item.occurs = Some(occurs);
            }
            "SIGN" | "LEADING" | "TRAILING" => {
                let mut word = word;
                if word == "SIGN" {
                    is(&mut k, "IS");
                    word = upper.get(k).map_or("", String::as_str);
                    k += 1;
                }
                let leading = match word {
                    "LEADING" => true,
                    "TRAILING" => false,
                    _ => return Err(error("SIGN is LEADING or TRAILING".into())),
                };
                let separate = upper.get(k).is_some_and(|w| w == "SEPARATE");
                if separate {
                    k += 1;
                    is(&mut k, "CHARACTER");
                }
                item.sign = Some(Sign { leading, separate });
            }
            "SYNC" | "SYNCHRONIZED" => {
                item.sync = true;
                if upper.get(k).is_some_and(|w| w == "LEFT" || w == "RIGHT") {
                    k += 1;
                }
            }
            "EXTERNAL" | "GLOBAL" | "IS" => {}
            _ => {
                if let Some(usage) = Usage::from_word(word) {
                    item.usage = Some(usage);
                } else {
                    other.push(words[k - 1].text.clone());
                }
            }
        }
    }
    item.other = (!other.is_empty()).then(|| other.join(" "));
    Ok(item)
}

fn usage_of(word: &str) -> Result<Usage, String> {
    Usage::from_word(word).ok_or_else(|| tr!("{word} is not a usage Gossia reads", word = word))
}

/// Nests the entries by their level numbers.
fn nest(entries: Vec<(Item, usize)>) -> Result<Vec<Item>, ParseError> {
    let mut top: Vec<Item> = Vec::new();
    // The open groups, outermost first.
    let mut stack: Vec<Item> = Vec::new();
    fn close(stack: &mut Vec<Item>, top: &mut Vec<Item>) {
        if let Some(item) = stack.pop() {
            match stack.last_mut() {
                Some(parent) => parent.items.push(item),
                None => top.push(item),
            }
        }
    }
    for (item, _) in entries {
        if item.level == 1 || item.level == 77 {
            while !stack.is_empty() {
                close(&mut stack, &mut top);
            }
        } else {
            while stack.last().is_some_and(|open| open.level >= item.level) {
                close(&mut stack, &mut top);
            }
            // When every open group is closed, the item is a record of its
            // own: a copybook copied in under an 01 of the program (IBM MQ's
            // start at level 10) has its records at that level.
        }
        stack.push(item);
    }
    while !stack.is_empty() {
        close(&mut stack, &mut top);
    }
    Ok(top)
}

// -------------------------------------------------------- writing text

/// Writes a copybook in fixed form: comments with `*` in column 7, entries
/// from column 8, each level four columns further in, nothing past
/// column 72.
pub fn to_text(copybook: &Copybook) -> String {
    let mut out = String::new();
    for item in &copybook.items {
        write_item(&mut out, item, 0);
    }
    write_comment(&mut out, copybook.end_comment.as_deref());
    out
}

fn write_comment(out: &mut String, comment: Option<&str>) {
    for line in comment.into_iter().flat_map(str::lines) {
        let text: String = line.chars().take(64).collect();
        let line = if text.is_empty() {
            "      *".to_owned()
        } else {
            format!("      * {text}")
        };
        out.push_str(line.trim_end());
        out.push('\n');
    }
}

fn write_item(out: &mut String, item: &Item, depth: usize) {
    write_comment(out, item.comment.as_deref());
    let mut words: Vec<String> = vec![format!("{:02}", item.level), item.name.clone()];
    if let Some(redefines) = &item.redefines {
        words.push("REDEFINES".into());
        words.push(redefines.clone());
    }
    if let Some(picture) = &item.picture {
        words.push("PIC".into());
        words.push(picture.clone());
    }
    if let Some(usage) = item.usage {
        words.push(usage.code().into());
    }
    if item.sync {
        words.push("SYNC".into());
    }
    if let Some(sign) = item.sign {
        words.push("SIGN".into());
        words.push(if sign.leading { "LEADING" } else { "TRAILING" }.into());
        if sign.separate {
            words.push("SEPARATE".into());
        }
    }
    if let Some(occurs) = &item.occurs {
        words.push("OCCURS".into());
        if let Some(min) = occurs.min {
            words.push(min.to_string());
            words.push("TO".into());
        }
        words.push(occurs.max.to_string());
        words.push("TIMES".into());
        if let Some(on) = &occurs.depending_on {
            words.push("DEPENDING".into());
            words.push("ON".into());
            words.push(on.clone());
        }
    }
    if let Some(other) = &item.other {
        words.extend(other.split_whitespace().map(str::to_owned));
    }
    if let Some(value) = &item.value {
        words.push("VALUE".into());
        words.push(value.clone());
    }
    write_entry(out, words, depth);
    for condition in &item.conditions {
        write_comment(out, condition.comment.as_deref());
        let mut words = vec!["88".to_owned(), condition.name.clone(), "VALUE".to_owned()];
        words.extend(literal_words(&condition.values));
        write_entry(out, words, depth + 1);
    }
    for child in &item.items {
        write_item(out, child, depth + 1);
    }
    for renames in &item.renames {
        write_comment(out, renames.comment.as_deref());
        let mut words = vec![
            "66".to_owned(),
            renames.name.clone(),
            "RENAMES".to_owned(),
            renames.from.clone(),
        ];
        if let Some(thru) = &renames.thru {
            words.push("THRU".into());
            words.push(thru.clone());
        }
        write_entry(out, words, depth);
    }
}

/// The literals of a VALUE clause as words, a quoted literal one word.
pub(crate) fn literal_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) => {
                word.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == '\'' || c == '"' => {
                word.push(c);
                quote = Some(c);
            }
            None if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            None => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Writes one entry, its level first, at the indentation of `depth`.
fn write_entry(out: &mut String, mut words: Vec<String>, depth: usize) {
    if let Some(last) = words.last_mut() {
        last.push('.');
    }
    let start = 7 + depth.min(10) * 4;
    let mut line = " ".repeat(start);
    let mut fresh = true;
    for (n, word) in words.iter().enumerate() {
        // The level and the name are apart by two spaces, as is usual.
        let gap = if fresh {
            ""
        } else if n == 1 {
            "  "
        } else {
            " "
        };
        if line.chars().count() + gap.len() + word.chars().count() <= 72 {
            line.push_str(gap);
            line.push_str(word);
            fresh = false;
            continue;
        }
        if !fresh {
            out.push_str(&line);
            out.push('\n');
            line = " ".repeat(start + 4);
        }
        // A literal too long for a line goes on on continuation lines.
        let mut rest: Vec<char> = word.chars().collect();
        loop {
            let room = 72 - line.chars().count();
            if rest.len() <= room {
                line.extend(rest.iter());
                break;
            }
            line.extend(rest[..room].iter());
            out.push_str(&line);
            out.push('\n');
            rest.drain(..room);
            let quote = word.chars().next().filter(|c| matches!(c, '\'' | '"'));
            line = format!("      -{}", " ".repeat(4));
            if let Some(quote) = quote {
                line.push(quote);
            }
        }
        fresh = false;
    }
    out.push_str(&line);
    out.push('\n');
}

// ------------------------------------------------- the schema tab's tree

/// The tree of a copybook schema tab, which the meta-schema types.
pub fn to_node(copybook: &Copybook, code_page: CodePage) -> Node {
    let mut fields = IndexMap::new();
    fields.insert(
        "codePage".to_owned(),
        Node::new(Value::Str(code_page.code().to_owned())),
    );
    if copybook.float != FloatFormat::Hex {
        fields.insert(
            "floatFormat".to_owned(),
            Node::new(Value::Str(copybook.float.code().to_owned())),
        );
    }
    fields.insert("items".to_owned(), items_node(&copybook.items));
    let mut node = Node::new(Value::Object(fields));
    node.set_comment_at(CommentPlace::End, copybook.end_comment.clone());
    node
}

fn items_node(items: &[Item]) -> Node {
    Node::new(Value::Array(items.iter().map(item_node).collect()))
}

fn item_node(item: &Item) -> Node {
    let text = |s: &str| Node::new(Value::Str(s.to_owned()));
    let mut f = IndexMap::new();
    f.insert(
        "level".to_owned(),
        Node::new(Value::Int(i64::from(item.level))),
    );
    f.insert("name".to_owned(), text(&item.name));
    if let Some(p) = &item.picture {
        f.insert("picture".to_owned(), text(p));
    }
    if let Some(u) = item.usage {
        f.insert("usage".to_owned(), text(u.code()));
    }
    if item.sync {
        f.insert("sync".to_owned(), Node::new(Value::Bool(true)));
    }
    if let Some(s) = item.sign {
        f.insert("sign".to_owned(), text(s.code()));
    }
    if let Some(o) = &item.occurs {
        if let Some(min) = o.min {
            f.insert(
                "occursFrom".to_owned(),
                Node::new(Value::Int(i64::from(min))),
            );
        }
        f.insert("occurs".to_owned(), Node::new(Value::Int(i64::from(o.max))));
        if let Some(on) = &o.depending_on {
            f.insert("dependingOn".to_owned(), text(on));
        }
    }
    if let Some(r) = &item.redefines {
        f.insert("redefines".to_owned(), text(r));
    }
    if let Some(v) = &item.value {
        f.insert("value".to_owned(), text(v));
    }
    if let Some(o) = &item.other {
        f.insert("other".to_owned(), text(o));
    }
    if !item.conditions.is_empty() {
        let conditions = item
            .conditions
            .iter()
            .map(|c| {
                let mut f = IndexMap::new();
                f.insert("name".to_owned(), text(&c.name));
                f.insert("values".to_owned(), text(&c.values));
                let mut node = Node::new(Value::Object(f));
                node.set_comment(c.comment.clone());
                node
            })
            .collect();
        f.insert("conditions".to_owned(), Node::new(Value::Array(conditions)));
    }
    if !item.items.is_empty() {
        f.insert("items".to_owned(), items_node(&item.items));
    }
    if !item.renames.is_empty() {
        let renames = item
            .renames
            .iter()
            .map(|r| {
                let mut f = IndexMap::new();
                f.insert("name".to_owned(), text(&r.name));
                f.insert("from".to_owned(), text(&r.from));
                if let Some(thru) = &r.thru {
                    f.insert("thru".to_owned(), text(thru));
                }
                let mut node = Node::new(Value::Object(f));
                node.set_comment(r.comment.clone());
                node
            })
            .collect();
        f.insert("renames".to_owned(), Node::new(Value::Array(renames)));
    }
    let mut node = Node::new(Value::Object(f));
    node.set_comment(item.comment.clone());
    node
}

/// The condition names of an item's tree. Before they had a place of their
/// own they were kept as comment lines (`88 ACTIVE VALUE 'A'.`), which are
/// read as condition names too.
type Entries = (Vec<ConditionName>, Vec<Renames>, Option<String>);

fn conditions_of(
    f: &IndexMap<String, Node>,
    comment: Option<&str>,
    at: &str,
) -> Result<Entries, String> {
    let mut conditions = Vec::new();
    if let Some(list) = f.get("conditions") {
        let Value::Array(list) = list.value() else {
            return Err(tr!("{at}.conditions is a list of condition names", at = at));
        };
        for (i, node) in list.iter().enumerate() {
            let here = format!("{at}.conditions[{i}]");
            let Value::Object(c) = node.value() else {
                return Err(tr!("{here} is a table of name and values", here = here));
            };
            let text = |key: &str| match c.get(key).map(Node::value) {
                Some(Value::Str(s)) if !s.trim().is_empty() => Ok(s.trim().to_owned()),
                _ => Err(tr!("{here}.{key} is text", here = here, key = key)),
            };
            conditions.push(ConditionName {
                name: text("name")?,
                values: text("values")?,
                comment: node.comment().map(str::to_owned),
            });
        }
    }
    let mut renames = Vec::new();
    if let Some(list) = f.get("renames") {
        let Value::Array(list) = list.value() else {
            return Err(tr!("{at}.renames is a list of RENAMES entries", at = at));
        };
        for (i, node) in list.iter().enumerate() {
            let here = format!("{at}.renames[{i}]");
            let Value::Object(r) = node.value() else {
                return Err(tr!("{here} is a table of name, from and thru", here = here));
            };
            let text = |key: &str| match r.get(key).map(Node::value) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Str(s)) if s.trim().is_empty() => Ok(None),
                Some(Value::Str(s)) => Ok(Some(s.trim().to_owned())),
                _ => Err(tr!("{here}.{key} is text", here = here, key = key)),
            };
            renames.push(Renames {
                name: text("name")?.ok_or_else(|| tr!("{here}.name is text", here = here))?,
                from: text("from")?.ok_or_else(|| tr!("{here}.from is text", here = here))?,
                thru: text("thru")?,
                comment: node.comment().map(str::to_owned),
            });
        }
    }
    let mut kept = Vec::new();
    for line in comment.into_iter().flat_map(str::lines) {
        let words = literal_words(line.trim().trim_end_matches('.'));
        let upper: Vec<String> = words.iter().map(|w| w.to_ascii_uppercase()).collect();
        // A RENAMES entry was kept as a comment line too.
        if upper.first().map(String::as_str) == Some("66")
            && upper.get(2).map(String::as_str) == Some("RENAMES")
        {
            let tokens: Vec<Token> = words[1..]
                .iter()
                .map(|w| Token {
                    text: w.clone(),
                    line: 0,
                    comment: None,
                    ends: false,
                })
                .collect();
            if let Ok(r) = self::renames(&tokens, None, 0) {
                renames.push(r);
                continue;
            }
        }
        if upper.first().map(String::as_str) == Some("88") && words.len() >= 3 {
            let mut k = 2;
            for skip in [["VALUE", "VALUES"], ["IS", "ARE"]] {
                if upper.get(k).is_some_and(|w| skip.contains(&w.as_str())) {
                    k += 1;
                }
            }
            if k < words.len() {
                conditions.push(ConditionName {
                    name: words[1].clone(),
                    values: words[k..].join(" "),
                    comment: None,
                });
                continue;
            }
        }
        kept.push(line);
    }
    let comment = (!kept.is_empty()).then(|| kept.join("\n"));
    Ok((conditions, renames, comment))
}

/// The copybook a schema tab's tree holds, and its code page.
pub fn from_node(node: &Node) -> Result<(Copybook, CodePage), String> {
    let Value::Object(fields) = node.value() else {
        return Err(tr!("a copybook schema is a table of codePage and items").into());
    };
    let code_page = match fields.get("codePage").map(Node::value) {
        None => CodePage::default(),
        Some(Value::Str(code)) => CodePage::from_code(code)
            .ok_or_else(|| tr!("{code} is not a code page Gossia knows", code = code))?,
        Some(_) => return Err(tr!("codePage is text, such as 037").into()),
    };
    let float = match fields.get("floatFormat").map(Node::value) {
        None | Some(Value::Null) => FloatFormat::default(),
        Some(Value::Str(code)) => FloatFormat::from_code(code).ok_or_else(|| {
            tr!(
                "{code} is not a floating point format: hex or ieee",
                code = code
            )
        })?,
        Some(_) => return Err(tr!("floatFormat is text: hex or ieee").into()),
    };
    let mut items = match fields.get("items") {
        Some(items) => items_of(items, "items")?,
        None => Vec::new(),
    };
    // RENAMES entries kept as comment lines of an item inside a record
    // belong to the record.
    for item in &mut items {
        let mut found = Vec::new();
        for child in &mut item.items {
            take_renames(child, &mut found);
        }
        item.renames.extend(found);
    }
    Ok((
        Copybook {
            items,
            end_comment: node.comment_at(CommentPlace::End).map(str::to_owned),
            float,
        },
        code_page,
    ))
}

fn take_renames(item: &mut Item, out: &mut Vec<Renames>) {
    out.append(&mut item.renames);
    for child in &mut item.items {
        take_renames(child, out);
    }
}

fn items_of(node: &Node, at: &str) -> Result<Vec<Item>, String> {
    let Value::Array(items) = node.value() else {
        return Err(tr!("{at} is a list of items", at = at));
    };
    items
        .iter()
        .enumerate()
        .map(|(i, n)| item_of(n, &format!("{at}[{i}]")))
        .collect()
}

fn item_of(node: &Node, at: &str) -> Result<Item, String> {
    let Value::Object(f) = node.value() else {
        return Err(tr!("{at} is a table of the item's clauses", at = at));
    };
    let text = |key: &str| -> Result<Option<String>, String> {
        match f.get(key).map(Node::value) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Str(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::Str(s)) => Ok(Some(s.trim().to_owned())),
            Some(_) => Err(tr!("{at}.{key} is text", at = at, key = key)),
        }
    };
    let number = |key: &str| -> Result<Option<u32>, String> {
        match f.get(key).map(Node::value) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Int(n)) => u32::try_from(*n)
                .map(Some)
                .map_err(|_| tr!("{at}.{key} is a whole number from 0", at = at, key = key)),
            Some(_) => Err(tr!("{at}.{key} is a whole number", at = at, key = key)),
        }
    };
    let level = number("level")?.ok_or_else(|| tr!("{at} has no level", at = at))?;
    let level = u8::try_from(level)
        .ok()
        .filter(|l| (1..=49).contains(l) || *l == 77)
        .ok_or_else(|| tr!("{at}.level is 1 to 49, or 77", at = at))?;
    let name = text("name")?.unwrap_or_else(|| "FILLER".to_owned());
    let usage = match text("usage")? {
        None => None,
        Some(u) => Some(usage_of(&u.to_ascii_uppercase()).map_err(|e| format!("{at}.usage: {e}"))?),
    };
    let sign = match text("sign")? {
        None => None,
        Some(s) => Some(Sign::from_code(&s).ok_or_else(|| {
            tr!(
                "{at}.sign is one of {codes}",
                at = at,
                codes = Sign::CODES.join(", ")
            )
        })?),
    };
    let occurs = match number("occurs")? {
        None => None,
        Some(max) => Some(Occurs {
            min: number("occursFrom")?,
            max,
            depending_on: text("dependingOn")?,
        }),
    };
    let items = match f.get("items") {
        Some(items) => items_of(items, &format!("{at}.items"))?,
        None => Vec::new(),
    };
    let (conditions, renames, comment) = conditions_of(f, node.comment(), at)?;
    let sync = match f.get("sync").map(Node::value) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(tr!("{at}.sync is true or false", at = at)),
    };
    Ok(Item {
        level,
        name,
        picture: text("picture")?.map(|p| p.to_ascii_uppercase()),
        usage,
        sync,
        sign,
        occurs,
        redefines: text("redefines")?,
        value: text("value")?,
        other: text("other")?,
        comment,
        conditions,
        items,
        renames,
    })
}

#[cfg(test)]
#[path = "copybook_tests.rs"]
mod copybook_tests;
