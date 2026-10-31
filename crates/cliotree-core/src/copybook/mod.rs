//! COBOL copybooks (DESIGN.md, "Import and export", "COBOL copybooks"): the
//! third schema language. A `.cpy` file is read into [`Copybook`], which a
//! copybook schema tab holds as a tree under the built-in meta-schema
//! (`builtin:copybook`, see [`to_node`] and [`from_node`]) and which is
//! written back as `.cpy` text by [`to_text`]. [`lower`] turns a copybook
//! into the type of its data tabs and the byte [`Layout`] of a record, and
//! [`records`] reads and writes `.blk` files of fixed-length records.

pub mod ebcdic;
mod lower;
pub mod records;
mod tables;

use std::fmt;

use indexmap::IndexMap;

use crate::value::{CommentPlace, Node, Value};

pub use ebcdic::CodePage;
pub use lower::{
    Counter, Element, Layout, NumUsage, Numeric, RedefineSet, SignPlace, Slot, SlotOccurs, lower,
    lower_node, lower_node_record, lower_record, record_names,
};

/// A copybook: its items nested by level, and the comment after the last.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Copybook {
    pub items: Vec<Item>,
    pub end_comment: Option<String>,
}

/// One data description entry, with the entries below it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub level: u8,
    /// The name as written; `FILLER` for an item without one.
    pub name: String,
    pub picture: Option<String>,
    pub usage: Option<Usage>,
    pub sign: Option<Sign>,
    pub occurs: Option<Occurs>,
    pub redefines: Option<String>,
    /// The `VALUE` clause's literal, as written.
    pub value: Option<String>,
    /// Clauses kept as written that do not change the layout (`JUSTIFIED`,
    /// `BLANK WHEN ZERO`, `INDEXED BY`, `KEY IS`).
    pub other: Option<String>,
    /// The comment lines above it, and condition names (level 88) below it.
    pub comment: Option<String>,
    pub items: Vec<Item>,
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
}

impl Usage {
    pub const ALL: [Usage; 7] = [
        Usage::Display,
        Usage::Comp,
        Usage::Comp3,
        Usage::Comp4,
        Usage::Comp5,
        Usage::Binary,
        Usage::PackedDecimal,
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

/// The text is not a copybook ClioTree can read.
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
            message: "a continuation line has nothing before it to go on with".into(),
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

/// Reads the text of a copybook.
pub fn parse(text: &str) -> Result<Copybook, ParseError> {
    let (tokens, end_comment) = tokens(text)?;
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
                message: "this entry has no period at its end".into(),
            });
        }
        let sentence = &tokens[start..=i];
        i += 1;
        let line = sentence[0].line;
        let level: u8 = sentence[0].text.parse().map_err(|_| ParseError {
            line,
            message: format!(
                "an entry starts with its level number, not {}",
                sentence[0].text
            ),
        })?;
        let comment = sentence[0].comment.clone();
        match level {
            88 | 66 => {
                // Condition names and RENAMES are kept as comments of the
                // item they follow.
                let text: Vec<&str> = sentence.iter().map(|t| t.text.as_str()).collect();
                let text = format!("{}.", text.join(" "));
                let Some((last, _)) = entries.last_mut() else {
                    return Err(ParseError {
                        line,
                        message: format!("a level {level} entry must follow an item"),
                    });
                };
                let joined = [last.comment.take(), comment, Some(text)]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n");
                last.comment = Some(joined);
                continue;
            }
            1..=49 | 77 => {}
            _ => {
                return Err(ParseError {
                    line,
                    message: format!("{level} is not a level number"),
                });
            }
        }
        let mut item = entry(&sentence[1..], line)?;
        item.level = level;
        item.comment = comment;
        entries.push((item, line));
    }
    let items = nest(entries)?;
    Ok(Copybook { items, end_comment })
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
                return Err(error(format!(
                    "{} is not supported: ClioTree does not align binary items",
                    words[k - 1].text
                )));
            }
            "COMP-1" | "COMP-2" | "COMPUTATIONAL-1" | "COMPUTATIONAL-2" => {
                return Err(error(format!(
                    "{} (floating point) is not supported",
                    words[k - 1].text
                )));
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
    Usage::from_word(word).ok_or_else(|| match word {
        "COMP-1" | "COMP-2" | "COMPUTATIONAL-1" | "COMPUTATIONAL-2" => {
            format!("{word} (floating point) is not supported")
        }
        _ => format!("{word} is not a usage ClioTree reads"),
    })
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
    for child in &item.items {
        write_item(out, child, depth + 1);
    }
}

// ------------------------------------------------- the schema tab's tree

/// The tree of a copybook schema tab, which the meta-schema types.
pub fn to_node(copybook: &Copybook, code_page: CodePage) -> Node {
    let mut fields = IndexMap::new();
    fields.insert(
        "codePage".to_owned(),
        Node::new(Value::Str(code_page.code().to_owned())),
    );
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
    if !item.items.is_empty() {
        f.insert("items".to_owned(), items_node(&item.items));
    }
    let mut node = Node::new(Value::Object(f));
    node.set_comment(item.comment.clone());
    node
}

/// The copybook a schema tab's tree holds, and its code page.
pub fn from_node(node: &Node) -> Result<(Copybook, CodePage), String> {
    let Value::Object(fields) = node.value() else {
        return Err("a copybook schema is a table of codePage and items".into());
    };
    let code_page = match fields.get("codePage").map(Node::value) {
        None => CodePage::default(),
        Some(Value::Str(code)) => CodePage::from_code(code)
            .ok_or_else(|| format!("{code} is not a code page ClioTree knows"))?,
        Some(_) => return Err("codePage is text, such as 037".into()),
    };
    let items = match fields.get("items") {
        Some(items) => items_of(items, "items")?,
        None => Vec::new(),
    };
    Ok((
        Copybook {
            items,
            end_comment: node.comment_at(CommentPlace::End).map(str::to_owned),
        },
        code_page,
    ))
}

fn items_of(node: &Node, at: &str) -> Result<Vec<Item>, String> {
    let Value::Array(items) = node.value() else {
        return Err(format!("{at} is a list of items"));
    };
    items
        .iter()
        .enumerate()
        .map(|(i, n)| item_of(n, &format!("{at}[{i}]")))
        .collect()
}

fn item_of(node: &Node, at: &str) -> Result<Item, String> {
    let Value::Object(f) = node.value() else {
        return Err(format!("{at} is a table of the item's clauses"));
    };
    let text = |key: &str| -> Result<Option<String>, String> {
        match f.get(key).map(Node::value) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Str(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::Str(s)) => Ok(Some(s.trim().to_owned())),
            Some(_) => Err(format!("{at}.{key} is text")),
        }
    };
    let number = |key: &str| -> Result<Option<u32>, String> {
        match f.get(key).map(Node::value) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Int(n)) => u32::try_from(*n)
                .map(Some)
                .map_err(|_| format!("{at}.{key} is a whole number from 0")),
            Some(_) => Err(format!("{at}.{key} is a whole number")),
        }
    };
    let level = number("level")?.ok_or_else(|| format!("{at} has no level"))?;
    let level = u8::try_from(level)
        .ok()
        .filter(|l| (1..=49).contains(l) || *l == 77)
        .ok_or_else(|| format!("{at}.level is 1 to 49, or 77"))?;
    let name = text("name")?.unwrap_or_else(|| "FILLER".to_owned());
    let usage = match text("usage")? {
        None => None,
        Some(u) => Some(usage_of(&u.to_ascii_uppercase()).map_err(|e| format!("{at}.usage: {e}"))?),
    };
    let sign = match text("sign")? {
        None => None,
        Some(s) => Some(
            Sign::from_code(&s)
                .ok_or_else(|| format!("{at}.sign is one of {}", Sign::CODES.join(", ")))?,
        ),
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
    Ok(Item {
        level,
        name,
        picture: text("picture")?.map(|p| p.to_ascii_uppercase()),
        usage,
        sign,
        occurs,
        redefines: text("redefines")?,
        value: text("value")?,
        other: text("other")?,
        comment: node.comment().map(str::to_owned),
        items,
    })
}

#[cfg(test)]
#[path = "copybook_tests.rs"]
mod copybook_tests;
