//! Reading and writing JSON5 and JSONC (DESIGN.md, "Import and export",
//! "Other file formats"): JSON with comments, which become the comments of
//! the nodes they stand above, and for JSON5 also unquoted keys, single
//! quotes, hexadecimal numbers and the rest of what JSON5 adds.
//!
//! One reader takes both. A comment on the lines above a key or an item is
//! that node's comment; one after it on the same line is added to it; one
//! after the last entry of an object or array is the container's end
//! comment; one in an empty container is its start comment.

use crate::json::{ParseError, Parsed, Warning, WriteError};
use crate::path::Path;
use crate::value::{CommentPlace, DateTimeValue, Node, Value};
use gossia_i18n::tr;

/// How deep objects and arrays may nest, as for JSON.
const MAX_DEPTH: usize = 128;

/// Which of the two a text is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// JSON5: keys unquoted where they can be.
    Json5,
    /// JSONC: strict JSON with comments, as VS Code's settings are.
    Jsonc,
}

/// Reads JSON5 or JSONC text.
pub fn parse(text: &str) -> Result<Parsed, ParseError> {
    let mut reader = Reader {
        chars: text.trim_start_matches('\u{FEFF}').chars().collect(),
        at: 0,
        line: 1,
        column: 1,
        warnings: Vec::new(),
        last_line: 1,
    };
    let lead = reader.space()?;
    if reader.peek().is_none() {
        return Err(reader.error(tr!("the text holds no value")));
    }
    let mut node = reader.value(&Path::root(), 0)?;
    let tail = reader.space()?;
    if reader.peek().is_some() {
        return Err(reader.error(tr!("only one value may be at the top")));
    }
    if let Some(text) = join(lead.into_iter().map(|c| c.text)) {
        node.set_comment(Some(text));
    }
    if let Some(text) = join(tail.into_iter().map(|c| c.text)) {
        add_comment(&mut node, CommentPlace::End, text);
    }
    Ok(Parsed {
        node,
        warnings: reader.warnings,
    })
}

/// A comment as read, and whether it began on the line where the value
/// before it ended.
struct Comment {
    text: String,
    trailing: bool,
}

struct Reader {
    chars: Vec<char>,
    at: usize,
    line: usize,
    column: usize,
    warnings: Vec<Warning>,
    /// The line on which the last value or punctuation ended.
    last_line: usize,
}

impl Reader {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            line: self.line,
            column: self.column,
            message: message.into(),
        }
    }

    fn expect(&mut self, wanted: char) -> Result<(), ParseError> {
        match self.peek() {
            Some(c) if c == wanted => {
                self.bump();
                self.last_line = self.line;
                Ok(())
            }
            Some(c) => Err(self.error(tr!(
                "expected `{wanted}`, found `{c}`",
                wanted = wanted,
                c = c
            ))),
            None => Err(self.error(tr!(
                "expected `{wanted}`, but the text ended",
                wanted = wanted
            ))),
        }
    }

    /// Passes over white space and comments, and gives the comments.
    fn space(&mut self) -> Result<Vec<Comment>, ParseError> {
        let mut comments = Vec::new();
        loop {
            match self.peek() {
                Some(c) if is_space(c) => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    let trailing = self.line == self.last_line;
                    self.bump();
                    self.bump();
                    let mut text = String::new();
                    while let Some(c) = self.peek() {
                        if c == '\n' || c == '\r' {
                            break;
                        }
                        text.push(c);
                        self.bump();
                    }
                    let text = text.strip_prefix(' ').unwrap_or(&text).trim_end();
                    comments.push(Comment {
                        text: text.to_owned(),
                        trailing,
                    });
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    let trailing = self.line == self.last_line;
                    let (line, column) = (self.line, self.column);
                    self.bump();
                    self.bump();
                    let mut text = String::new();
                    loop {
                        match self.peek() {
                            None => {
                                return Err(ParseError {
                                    line,
                                    column,
                                    message: tr!("a comment begun with /* is not ended with */")
                                        .into(),
                                });
                            }
                            Some('*') if self.peek_at(1) == Some('/') => {
                                self.bump();
                                self.bump();
                                break;
                            }
                            Some(c) => {
                                text.push(c);
                                self.bump();
                            }
                        }
                    }
                    comments.push(Comment {
                        text: block_text(&text),
                        trailing,
                    });
                }
                _ => return Ok(comments),
            }
        }
    }

    fn value(&mut self, path: &Path, depth: usize) -> Result<Node, ParseError> {
        let value = match self.peek() {
            Some('{') => return self.object(path, depth),
            Some('[') => return self.array(path, depth),
            Some('"' | '\'') => Value::Str(self.string()?),
            Some(c) if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() => {
                self.number(path)?
            }
            Some(c) if is_identifier_start(c) => {
                let (line, column) = (self.line, self.column);
                let word = self.identifier()?;
                match word.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    "null" => Value::Null,
                    "Infinity" | "NaN" => {
                        return Err(ParseError {
                            line,
                            column,
                            message: tr!(
                                "{word} cannot be read: Gossia holds only finite numbers",
                                word = word
                            ),
                        });
                    }
                    _ => {
                        return Err(ParseError {
                            line,
                            column,
                            message: tr!("`{word}` is not a value", word = word),
                        });
                    }
                }
            }
            Some(c) => return Err(self.error(tr!("`{c}` cannot begin a value", c = c))),
            None => return Err(self.error(tr!("a value was expected, but the text ended"))),
        };
        self.last_line = self.line;
        Ok(Node::new(value))
    }

    fn object(&mut self, path: &Path, depth: usize) -> Result<Node, ParseError> {
        if depth >= MAX_DEPTH {
            return Err(self.error(tr!("the data nests too deep")));
        }
        self.expect('{')?;
        let mut fields: indexmap::IndexMap<String, Node> = indexmap::IndexMap::new();
        let mut pending = Vec::new();
        let mut last: Option<String> = None;
        loop {
            let comments = self.space()?;
            // A comment on the line of the entry before it belongs to it.
            for comment in comments {
                match (&last, comment.trailing) {
                    (Some(key), true) => add_comment(
                        fields.get_mut(key).expect("the last key is there"),
                        CommentPlace::Before,
                        comment.text,
                    ),
                    _ => pending.push(comment.text),
                }
            }
            if self.peek() == Some('}') {
                self.bump();
                self.last_line = self.line;
                break;
            }
            let key_line = (self.line, self.column);
            let key = match self.peek() {
                Some('"' | '\'') => self.string()?,
                Some(c) if is_identifier_start(c) || c == '\\' => self.identifier()?,
                Some(c) => return Err(self.error(tr!("`{c}` cannot begin a key", c = c))),
                None => return Err(self.error(tr!("the object is not closed with `}`"))),
            };
            let more = self.space()?;
            pending.extend(more.into_iter().map(|c| c.text));
            self.expect(':')?;
            let more = self.space()?;
            pending.extend(more.into_iter().map(|c| c.text));
            let mut node = self.value(&path.key(key.clone()), depth + 1)?;
            if let Some(text) = join(pending.drain(..)) {
                node.set_comment(Some(text));
            }
            if fields.contains_key(&key) {
                self.warnings.push(Warning {
                    path: path.key(key.clone()),
                    message: tr!(
                        "the key {key} is there twice (line {line}); the last is kept",
                        key = format!("{key:?}"),
                        line = key_line.0
                    ),
                });
            }
            fields.insert(key.clone(), node);
            if !self.after_entry(
                '}',
                fields.get_mut(&key).expect("just put there"),
                &mut pending,
            )? {
                break;
            }
            last = Some(key);
        }
        let mut node = Node::new(Value::Object(fields));
        self.finish_container(&mut node, pending);
        Ok(node)
    }

    fn array(&mut self, path: &Path, depth: usize) -> Result<Node, ParseError> {
        if depth >= MAX_DEPTH {
            return Err(self.error(tr!("the data nests too deep")));
        }
        self.expect('[')?;
        let mut items: Vec<Node> = Vec::new();
        let mut pending = Vec::new();
        loop {
            let comments = self.space()?;
            for comment in comments {
                match items.last_mut() {
                    Some(item) if comment.trailing => {
                        add_comment(item, CommentPlace::Before, comment.text)
                    }
                    _ => pending.push(comment.text),
                }
            }
            if self.peek() == Some(']') {
                self.bump();
                self.last_line = self.line;
                break;
            }
            let mut node = self.value(&path.index(items.len()), depth + 1)?;
            if let Some(text) = join(pending.drain(..)) {
                node.set_comment(Some(text));
            }
            items.push(node);
            let item = items.last_mut().expect("just pushed");
            if !self.after_entry(']', item, &mut pending)? {
                break;
            }
        }
        let mut node = Node::new(Value::Array(items));
        self.finish_container(&mut node, pending);
        Ok(node)
    }

    /// After an entry: comments on its line, then a comma or the closing
    /// bracket. True when another entry (or a comment, or the bracket after a
    /// trailing comma) may follow; false when the bracket was read.
    fn after_entry(
        &mut self,
        close: char,
        entry: &mut Node,
        pending: &mut Vec<String>,
    ) -> Result<bool, ParseError> {
        let comments = self.space()?;
        for comment in comments {
            if comment.trailing {
                add_comment(entry, CommentPlace::Before, comment.text);
            } else {
                pending.push(comment.text);
            }
        }
        match self.peek() {
            Some(',') => {
                self.bump();
                self.last_line = self.line;
                Ok(true)
            }
            Some(c) if c == close => {
                self.bump();
                self.last_line = self.line;
                Ok(false)
            }
            Some(c) => Err(self.error(tr!(
                "expected `,` or `{close}`, found `{c}`",
                close = close,
                c = c
            ))),
            None => Err(self.error(tr!("the text ended before `{close}`", close = close))),
        }
    }

    /// Comments left over in a container: its end comment, or its start
    /// comment when it is empty.
    fn finish_container(&mut self, node: &mut Node, pending: Vec<String>) {
        if let Some(text) = join(pending.into_iter()) {
            let empty = match node.value() {
                Value::Object(f) => f.is_empty(),
                Value::Array(a) => a.is_empty(),
                _ => false,
            };
            let place = if empty {
                CommentPlace::Start
            } else {
                CommentPlace::End
            };
            add_comment(node, place, text);
        }
        self.last_line = self.line;
    }

    fn string(&mut self) -> Result<String, ParseError> {
        let quote = self.bump().expect("a quote is there");
        let (line, column) = (self.line, self.column - 1);
        let mut out = String::new();
        loop {
            match self.bump() {
                None => {
                    return Err(ParseError {
                        line,
                        column,
                        message: tr!("the string is not closed").into(),
                    });
                }
                Some(c) if c == quote => break,
                Some('\n' | '\r') => {
                    return Err(self.error(tr!(
                        "a string cannot hold a line break; write \\n, or end the line with \\"
                    )));
                }
                Some('\\') => self.escape(&mut out)?,
                Some(c) => out.push(c),
            }
        }
        self.last_line = self.line;
        Ok(out)
    }

    fn escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        match self.bump() {
            None => Err(self.error(tr!("the text ended inside an escape"))),
            Some('b') => {
                out.push('\u{8}');
                Ok(())
            }
            Some('f') => {
                out.push('\u{C}');
                Ok(())
            }
            Some('n') => {
                out.push('\n');
                Ok(())
            }
            Some('r') => {
                out.push('\r');
                Ok(())
            }
            Some('t') => {
                out.push('\t');
                Ok(())
            }
            Some('v') => {
                out.push('\u{B}');
                Ok(())
            }
            Some('0') if !self.peek().is_some_and(|c| c.is_ascii_digit()) => {
                out.push('\0');
                Ok(())
            }
            Some('x') => {
                let n = self.hex_digits(2)?;
                out.push(char::from_u32(n).expect("two hex digits are a char"));
                Ok(())
            }
            Some('u') => {
                let n = self.hex_digits(4)?;
                if (0xD800..0xDC00).contains(&n)
                    && self.peek() == Some('\\')
                    && self.peek_at(1) == Some('u')
                {
                    self.bump();
                    self.bump();
                    let low = self.hex_digits(4)?;
                    if (0xDC00..0xE000).contains(&low) {
                        let c = 0x10000 + ((n - 0xD800) << 10) + (low - 0xDC00);
                        out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                        return Ok(());
                    }
                    out.push('\u{FFFD}');
                    out.push(char::from_u32(low).unwrap_or('\u{FFFD}'));
                    return Ok(());
                }
                out.push(char::from_u32(n).unwrap_or('\u{FFFD}'));
                Ok(())
            }
            // A line continuation: the escaped line break is left out.
            Some('\r') => {
                if self.peek() == Some('\n') {
                    self.bump();
                }
                Ok(())
            }
            Some('\n' | '\u{2028}' | '\u{2029}') => Ok(()),
            Some(c) if c.is_ascii_digit() => Err(self.error(tr!(
                "a digit cannot be escaped (octal escapes are not JSON5)"
            ))),
            Some(c) => {
                out.push(c);
                Ok(())
            }
        }
    }

    fn hex_digits(&mut self, count: usize) -> Result<u32, ParseError> {
        let mut n = 0;
        for _ in 0..count {
            let d = self.peek().and_then(|c| c.to_digit(16)).ok_or_else(|| {
                self.error(tr!(
                    "{count} hexadecimal digits were expected",
                    count = count
                ))
            })?;
            self.bump();
            n = n * 16 + d;
        }
        Ok(n)
    }

    fn identifier(&mut self) -> Result<String, ParseError> {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == '\\' {
                self.bump();
                if self.bump() != Some('u') {
                    return Err(self.error(tr!("only \\u escapes may be in a key")));
                }
                let n = self.hex_digits(4)?;
                out.push(char::from_u32(n).unwrap_or('\u{FFFD}'));
            } else if (out.is_empty() && is_identifier_start(c))
                || (!out.is_empty() && is_identifier_part(c))
            {
                out.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if out.is_empty() {
            return Err(self.error(tr!("a key was expected")));
        }
        self.last_line = self.line;
        Ok(out)
    }

    fn number(&mut self, path: &Path) -> Result<Value, ParseError> {
        let (line, column) = (self.line, self.column);
        let mut text = String::new();
        let negative = match self.peek() {
            Some('-') => {
                self.bump();
                true
            }
            Some('+') => {
                self.bump();
                false
            }
            _ => false,
        };
        if self.peek().is_some_and(is_identifier_start) {
            let word = self.identifier()?;
            return Err(ParseError {
                line,
                column,
                message: if word == "Infinity" || word == "NaN" {
                    tr!(
                        "{word} cannot be read: Gossia holds only finite numbers",
                        word = word
                    )
                } else {
                    tr!("`{word}` is not a number", word = word)
                },
            });
        }
        if self.peek() == Some('0') && matches!(self.peek_at(1), Some('x' | 'X')) {
            self.bump();
            self.bump();
            let mut digits = String::new();
            while let Some(c) = self.peek().filter(char::is_ascii_hexdigit) {
                digits.push(c);
                self.bump();
            }
            if digits.is_empty() {
                return Err(self.error(tr!("hexadecimal digits were expected after 0x")));
            }
            self.last_line = self.line;
            return Ok(match i64::from_str_radix(&digits, 16) {
                Ok(n) => Value::Int(if negative { -n } else { n }),
                Err(_) => {
                    self.warnings.push(Warning {
                        path: path.clone(),
                        message: tr!("integer beyond the signed 64-bit range was read as a \
                                  floating-point number")
                        .into(),
                    });
                    let x = digits.chars().fold(0f64, |n, c| {
                        n * 16.0 + f64::from(c.to_digit(16).unwrap_or(0))
                    });
                    Value::Float(if negative { -x } else { x })
                }
            });
        }
        if negative {
            text.push('-');
        }
        while let Some(c) = self.peek() {
            // A sign is part of the number only right after its exponent.
            let exponent_sign = matches!(c, '+' | '-') && text.ends_with(['e', 'E']);
            if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E') || exponent_sign {
                text.push(c);
                self.bump();
            } else {
                break;
            }
        }
        self.last_line = self.line;
        let bad = || ParseError {
            line,
            column,
            message: tr!("`{text}` is not a number", text = text),
        };
        let digits = text.trim_start_matches('-');
        if digits.is_empty() || digits == "." {
            return Err(bad());
        }
        let integral = !text.contains(['.', 'e', 'E']);
        if integral {
            if digits.len() > 1 && digits.starts_with('0') {
                return Err(ParseError {
                    line,
                    column,
                    message: tr!(
                        "`{text}` has a leading zero, which JSON5 does not allow",
                        text = text
                    ),
                });
            }
            if let Ok(n) = text.parse::<i64>() {
                return Ok(Value::Int(n));
            }
            self.warnings.push(Warning {
                path: path.clone(),
                message: tr!(
                    "integer beyond the signed 64-bit range was read as a floating-point \
                          number"
                )
                .into(),
            });
        }
        // Rust reads `.5` and `5.` as JSON5 does.
        let x: f64 = text.parse().map_err(|_| bad())?;
        if !x.is_finite() {
            return Err(ParseError {
                line,
                column,
                message: tr!("`{text}` is too large to be a number", text = text),
            });
        }
        Ok(Value::Float(x))
    }
}

fn is_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\u{B}'
            | '\u{C}'
            | '\u{A0}'
            | '\u{FEFF}'
            | '\u{2028}'
            | '\u{2029}'
    ) || (c.is_whitespace() && !c.is_ascii())
}

fn is_identifier_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_alphabetic()
}

fn is_identifier_part(c: char) -> bool {
    is_identifier_start(c) || c.is_alphanumeric() || c == '\u{200C}' || c == '\u{200D}'
}

/// The text of a `/* */` comment: trimmed, and with the `*` that begins each
/// line of a boxed comment taken off.
fn block_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let boxed = lines.len() > 1 && lines.iter().skip(1).all(|l| l.starts_with('*'));
    let lines: Vec<String> = lines
        .iter()
        .map(|l| {
            let l = if boxed { l.trim_start_matches('*') } else { l };
            l.strip_prefix(' ').unwrap_or(l).trim_end().to_owned()
        })
        .collect();
    let start = lines
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(start, |i| i + 1);
    lines[start..end].join("\n")
}

fn join(texts: impl Iterator<Item = String>) -> Option<String> {
    let texts: Vec<String> = texts.collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

fn add_comment(node: &mut Node, place: CommentPlace, text: String) {
    let joined = match node.comment_at(place) {
        Some(before) => format!("{before}\n{text}"),
        None => text,
    };
    node.set_comment_at(place, Some(joined));
}

/// Writes a tree as JSON5 or JSONC, with two-space indentation if `pretty`,
/// and its comments as `//` lines (as `/* */` when compact). As for JSON, a
/// node that holds an error value is written absent.
pub fn to_string(node: &Node, style: Style, pretty: bool) -> Result<String, WriteError> {
    let mut out = String::new();
    let writer = Writer { style, pretty };
    if matches!(node.value(), Value::Error(_)) {
        return Err(WriteError {
            path: Path::root(),
            message: tr!("the root holds an error value").into(),
        });
    }
    writer.comment(&mut out, node.comment(), 0);
    writer.value(&mut out, node, &Path::root(), 0)?;
    // An object or array writes its end comment inside its brackets.
    let container = matches!(node.value(), Value::Object(_) | Value::Array(_));
    if let Some(end) = node.comment_at(CommentPlace::End).filter(|_| !container) {
        out.push(if pretty { '\n' } else { ' ' });
        writer.comment(&mut out, Some(end), 0);
        out.truncate(out.trim_end().len());
    }
    if pretty {
        out.push('\n');
    }
    Ok(out)
}

struct Writer {
    style: Style,
    pretty: bool,
}

impl Writer {
    fn indent(&self, out: &mut String, depth: usize) {
        if self.pretty {
            for _ in 0..depth {
                out.push_str("  ");
            }
        }
    }

    /// The comment, on lines of its own before what follows.
    fn comment(&self, out: &mut String, comment: Option<&str>, depth: usize) {
        let Some(comment) = comment else {
            return;
        };
        if self.pretty {
            for line in comment.lines() {
                self.indent(out, depth);
                if line.is_empty() {
                    out.push_str("//\n");
                } else {
                    out.push_str("// ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
        } else {
            out.push_str("/* ");
            out.push_str(&comment.replace("*/", "* /"));
            out.push_str(" */ ");
        }
    }

    fn value(
        &self,
        out: &mut String,
        node: &Node,
        path: &Path,
        depth: usize,
    ) -> Result<(), WriteError> {
        match node.value() {
            Value::Error(_) => unreachable!("errored nodes are left out by the caller"),
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(i) => out.push_str(&i.to_string()),
            Value::Float(x) => {
                if !x.is_finite() {
                    return Err(WriteError {
                        path: path.clone(),
                        message: tr!("NaN and infinity cannot be written").into(),
                    });
                }
                out.push_str(&serde_json::to_string(x).expect("a finite float serializes"));
            }
            Value::Str(s) => quote(out, s),
            Value::Date(d) => quote(out, &d.format("%Y-%m-%d").to_string()),
            Value::DateTime(DateTimeValue::Naive(dt)) => {
                quote(out, &dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
            }
            Value::DateTime(DateTimeValue::Zoned(dt)) => quote(out, &dt.to_rfc3339()),
            Value::Array(items) => {
                let items: Vec<(usize, &Node)> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| !matches!(n.value(), Value::Error(_)))
                    .collect();
                self.container(out, node, '[', ']', items.len(), depth, |out, i| {
                    let (index, item) = items[i];
                    self.comment(out, item.comment(), depth + 1);
                    self.indent(out, depth + 1);
                    self.value(out, item, &path.index(index), depth + 1)
                })?;
            }
            Value::Object(fields) => {
                let fields: Vec<(&String, &Node)> = fields
                    .iter()
                    .filter(|(_, n)| !matches!(n.value(), Value::Error(_)))
                    .collect();
                self.container(out, node, '{', '}', fields.len(), depth, |out, i| {
                    let (key, field) = fields[i];
                    self.comment(out, field.comment(), depth + 1);
                    self.indent(out, depth + 1);
                    self.key(out, key);
                    out.push(':');
                    if self.pretty {
                        out.push(' ');
                    }
                    self.value(out, field, &path.key(key.clone()), depth + 1)
                })?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn container(
        &self,
        out: &mut String,
        node: &Node,
        open: char,
        close: char,
        count: usize,
        depth: usize,
        mut entry: impl FnMut(&mut String, usize) -> Result<(), WriteError>,
    ) -> Result<(), WriteError> {
        out.push(open);
        let start = node.comment_at(CommentPlace::Start);
        let end = node.comment_at(CommentPlace::End);
        if count == 0 && start.is_none() && end.is_none() {
            out.push(close);
            return Ok(());
        }
        if self.pretty {
            out.push('\n');
        }
        self.comment(out, start, depth + 1);
        for i in 0..count {
            entry(out, i)?;
            if i + 1 < count {
                out.push(',');
            }
            if self.pretty {
                out.push('\n');
            }
        }
        self.comment(out, end, depth + 1);
        self.indent(out, depth);
        out.push(close);
        Ok(())
    }

    fn key(&self, out: &mut String, key: &str) {
        let bare = self.style == Style::Json5
            && key.chars().next().is_some_and(is_identifier_start)
            && key
                .chars()
                .all(|c| is_identifier_part(c) && c != '\u{200C}' && c != '\u{200D}');
        if bare {
            out.push_str(key);
        } else {
            quote(out, key);
        }
    }
}

fn quote(out: &mut String, text: &str) {
    out.push_str(&serde_json::to_string(text).expect("a string serializes"));
}

#[cfg(test)]
#[path = "json5_tests.rs"]
mod json5_tests;
