//! Reading and writing YAML (DESIGN.md, "Other file formats", "YAML"). The
//! text is parsed with `saphyr-parser` into events, which carry where they
//! are in the text but not the comments; the comments are found in the text
//! between the events and given to the nodes around them. Values are read by
//! the YAML 1.2 core schema: only `true` and `false` are booleans, and only
//! `null`, `~` or nothing is null. Anchors and aliases are copied out, merge
//! keys (`<<`) are applied, and tags outside the core schema are refused.
//! Writing is ClioTree's own: block style, two spaces of indentation.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use saphyr_parser::{Event, Parser, ScalarStyle, Span, Tag};

use crate::json::{ParseError, Parsed, Warning, WriteError};
use crate::path::Path;
use crate::value::{CommentPlace, DateTimeValue, Node, Value};

/// How deeply lists and tables may nest.
const MAX_DEPTH: usize = 512;

/// How many nodes aliases may copy in all, against "billion laughs" files.
const MAX_COPIED: usize = 1_000_000;

/// The tags of the core schema, by their suffix.
const CORE_TAGS: [&str; 7] = ["str", "int", "float", "bool", "null", "map", "seq"];

static FLOAT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$").expect("valid")
});
static INFINITE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([-+]?\.(inf|Inf|INF)|\.(nan|NaN|NAN))$").expect("valid"));

/// Reads YAML text of one document. A file of several documents is read as
/// a list of them; [`document_count`] tells the two apart.
pub fn parse(text: &str) -> Result<Parsed, ParseError> {
    let (mut documents, warnings) = read(text)?;
    let node = match documents.len() {
        0 => Node::new(Value::Null),
        1 => documents.remove(0),
        _ => Node::new(Value::Array(documents)),
    };
    Ok(Parsed { node, warnings })
}

/// Reads YAML text as a list of its documents.
pub fn parse_documents(text: &str) -> Result<Parsed, ParseError> {
    let (documents, warnings) = read(text)?;
    Ok(Parsed {
        node: Node::new(Value::Array(documents)),
        warnings,
    })
}

/// How many documents the text holds, or 0 if it is not YAML.
pub fn document_count(text: &str) -> usize {
    let mut parser = Parser::new_from_str(text.trim_start_matches('\u{FEFF}'));
    let mut count = 0;
    while let Some(Ok((event, _))) = parser.next_event() {
        match event {
            Event::DocumentStart(_) => count += 1,
            Event::StreamEnd => return count,
            _ => {}
        }
    }
    0
}

/// A `#` comment in the text.
#[derive(Debug)]
struct Comment {
    /// Where the `#` is, in characters from the start.
    index: usize,
    /// Its line from 1 and column from 0.
    line: usize,
    column: usize,
    /// Whether something other than white space comes before it on its line.
    trailing: bool,
    text: String,
}

fn read(text: &str) -> Result<(Vec<Node>, Vec<Warning>), ParseError> {
    let text = text.trim_start_matches('\u{FEFF}');
    let mut parser = Parser::new_from_str(text);
    let mut events = Vec::new();
    while let Some(next) = parser.next_event() {
        let (event, span) = next.map_err(|e| ParseError {
            line: e.marker().line(),
            column: e.marker().col() + 1,
            message: e.info().to_owned(),
        })?;
        events.push((event, span));
    }
    let comments = find_comments(text, &events);
    let mut builder = Builder {
        events,
        at: 0,
        comments,
        next: 0,
        anchors: HashMap::new(),
        copied: 0,
        warnings: Vec::new(),
    };
    let documents = builder.stream()?;
    Ok((documents, builder.warnings))
}

/// The comments of the text: a `#` at the start of a line or after white
/// space, outside every scalar, up to the end of its line.
fn find_comments(text: &str, events: &[(Event<'_>, Span)]) -> Vec<Comment> {
    let mut scalars: Vec<(usize, usize)> = events
        .iter()
        .filter(|(e, _)| matches!(e, Event::Scalar(..)))
        .map(|(_, s)| (s.start.index(), s.end.index()))
        .collect();
    scalars.sort_unstable();
    let mut scalar = 0;
    let mut comments = Vec::new();
    let mut index = 0;
    for (line_number, line) in text.split('\n').enumerate() {
        let chars: Vec<char> = line.trim_end_matches('\r').chars().collect();
        let mut column = 0;
        while column < chars.len() {
            let here = index + column;
            while scalar < scalars.len() && scalars[scalar].1 <= here {
                scalar += 1;
            }
            let inside = scalars
                .get(scalar)
                .is_some_and(|&(start, end)| start <= here && here < end);
            if chars[column] == '#' && !inside && (column == 0 || chars[column - 1].is_whitespace())
            {
                let rest: String = chars[column + 1..].iter().collect();
                comments.push(Comment {
                    index: here,
                    line: line_number + 1,
                    column,
                    trailing: chars[..column].iter().any(|c| !c.is_whitespace()),
                    text: rest
                        .strip_prefix(' ')
                        .unwrap_or(&rest)
                        .trim_end()
                        .to_owned(),
                });
                break;
            }
            column += 1;
        }
        index += line.chars().count() + 1;
    }
    comments
}

/// Two comments as one: the first, then the second.
fn joined(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
        (a, b) => a.or(b),
    }
}

fn join_texts<'c>(comments: impl Iterator<Item = &'c Comment>) -> Option<String> {
    let texts: Vec<&str> = comments.map(|c| c.text.as_str()).collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

/// Adds a comment after what a node has at `place`.
fn add(node: &mut Node, place: CommentPlace, comment: Option<String>) {
    let comment = joined(node.comment_at(place).map(str::to_owned), comment);
    node.set_comment_at(place, comment);
}

fn size(node: &Node) -> usize {
    1 + match node.value() {
        Value::Array(items) => items.iter().map(size).sum(),
        Value::Object(fields) => fields.values().map(size).sum(),
        _ => 0,
    }
}

struct Builder<'t> {
    events: Vec<(Event<'t>, Span)>,
    at: usize,
    comments: Vec<Comment>,
    /// The first comment not yet given to a node; those before it are.
    next: usize,
    anchors: HashMap<usize, Node>,
    copied: usize,
    warnings: Vec<Warning>,
}

impl Builder<'_> {
    fn event(&self) -> &Event<'_> {
        self.events
            .get(self.at)
            .map_or(&Event::StreamEnd, |(e, _)| e)
    }

    fn span(&self) -> Option<&Span> {
        self.events.get(self.at).map(|(_, s)| s)
    }

    /// Where the current event starts, in characters.
    fn position(&self) -> usize {
        self.span().map_or(usize::MAX, |s| s.start.index())
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        let (line, column) = self
            .span()
            .map_or((1, 1), |s| (s.start.line(), s.start.col() + 1));
        ParseError {
            line,
            column,
            message: message.into(),
        }
    }

    /// The comments not yet given out that come before the current event.
    fn waiting(&self) -> std::ops::Range<usize> {
        let position = self.position();
        let end = self.comments[self.next..]
            .iter()
            .position(|c| c.index >= position)
            .map_or(self.comments.len(), |n| self.next + n);
        self.next..end
    }

    /// All the comments before the current event, given out.
    fn above(&mut self) -> Option<String> {
        let range = self.waiting();
        self.next = range.end;
        join_texts(self.comments[range].iter())
    }

    /// A comment on the line where `node` ends, after it: one more line of
    /// its comment.
    fn trailing(&mut self, node: &mut Node) {
        let range = self.waiting();
        if let Some(c) = self.comments[range].first().filter(|c| c.trailing) {
            let text = c.text.clone();
            self.next += 1;
            add(node, CommentPlace::Before, Some(text));
        }
    }

    /// The comments before the end of a container indented at least as far
    /// as its entries: those at its end. The rest are left for what follows.
    fn ending(&mut self, column: usize) -> Option<String> {
        let range = self.waiting();
        let count = self.comments[range.clone()]
            .iter()
            .take_while(|c| c.trailing || c.column >= column)
            .count();
        self.next += count;
        join_texts(self.comments[range.start..range.start + count].iter())
    }

    fn stream(&mut self) -> Result<Vec<Node>, ParseError> {
        let mut documents = Vec::new();
        loop {
            match self.event() {
                Event::StreamStart => self.at += 1,
                Event::DocumentStart(_) => {
                    self.at += 1;
                    let path = Path::root().index(documents.len());
                    documents.push(self.document(&path)?);
                }
                Event::StreamEnd => break,
                _ => return Err(self.error("this is not where a YAML document can start")),
            }
        }
        // What is left after the last document is the end of it.
        let rest = join_texts(self.comments[self.next..].iter());
        if let Some(last) = documents.last_mut() {
            add(last, CommentPlace::End, rest);
        }
        Ok(documents)
    }

    fn document(&mut self, path: &Path) -> Result<Node, ParseError> {
        let container = matches!(
            self.event(),
            Event::MappingStart(..) | Event::SequenceStart(..)
        );
        // The comments above the first entry that are apart from it by an
        // empty line are the document's; those right above it are its.
        let range = self.waiting();
        let start_line = self.span().map_or(usize::MAX, |s| s.start.line());
        let own = if container {
            let mut own = range.start;
            for i in range.clone() {
                let following = self
                    .comments
                    .get(i + 1)
                    .filter(|_| i + 1 < range.end)
                    .map_or(start_line, |c| c.line);
                if following > self.comments[i].line + 1 {
                    own = i + 1;
                }
            }
            own
        } else {
            range.end
        };
        let before = join_texts(self.comments[range.start..own].iter());
        self.next = own;
        let mut node = self.node(path, 0)?;
        self.trailing(&mut node);
        node.set_comment(joined(before, node.comment().map(str::to_owned)));
        if !matches!(self.event(), Event::DocumentEnd) {
            return Err(self.error("a YAML document holds one value"));
        }
        let end = self.ending(0);
        add(&mut node, CommentPlace::End, end);
        self.at += 1;
        Ok(node)
    }

    /// The tag's core-schema name, or None if it has none. Any other tag
    /// refuses the file.
    fn tag(&self, tag: Option<&Tag>) -> Result<Option<String>, ParseError> {
        let Some(tag) = tag else {
            return Ok(None);
        };
        // A lone `!` makes the value text.
        if tag.handle.is_empty() && tag.suffix == "!" {
            return Ok(Some("str".to_owned()));
        }
        if tag.is_yaml_core_schema() && CORE_TAGS.contains(&tag.suffix.as_str()) {
            return Ok(Some(tag.suffix.clone()));
        }
        let name = if tag.is_yaml_core_schema() {
            format!("!!{}", tag.suffix)
        } else {
            format!("{}{}", tag.handle, tag.suffix)
        };
        Err(self.error(format!(
            "the tag {name} is not one ClioTree reads: only plain YAML values are read, without custom tags"
        )))
    }

    fn node(&mut self, path: &Path, depth: usize) -> Result<Node, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("the lists and tables are nested too deeply"));
        }
        let (event, span) = self
            .events
            .get(self.at)
            .cloned()
            .ok_or_else(|| self.error("the text ends too soon"))?;
        match event {
            Event::Scalar(text, style, anchor, tag) => {
                let tag = self.tag(tag.as_deref())?;
                let value = self.scalar(&text, style, tag.as_deref(), path)?;
                self.at += 1;
                let node = Node::new(value);
                self.anchor(anchor, &node);
                Ok(node)
            }
            Event::Alias(anchor) => {
                let Some(mut node) = self.anchors.get(&anchor).cloned() else {
                    return Err(self.error("this alias names no anchor before it"));
                };
                self.copied += size(&node);
                if self.copied > MAX_COPIED {
                    return Err(self.error("the aliases copy in too much data"));
                }
                node.set_comment(None);
                self.warnings.push(Warning {
                    path: path.clone(),
                    message: "a copy of an anchored value (a YAML alias) was read; it is written out in full when saved".into(),
                });
                self.at += 1;
                Ok(node)
            }
            Event::SequenceStart(anchor, tag) => {
                if !matches!(self.tag(tag.as_deref())?.as_deref(), None | Some("seq")) {
                    return Err(self.error("this tag does not go with a list"));
                }
                self.at += 1;
                let node = self.sequence(path, depth, span.start.col())?;
                self.anchor(anchor, &node);
                Ok(node)
            }
            Event::MappingStart(anchor, tag) => {
                if !matches!(self.tag(tag.as_deref())?.as_deref(), None | Some("map")) {
                    return Err(self.error("this tag does not go with a table"));
                }
                self.at += 1;
                let node = self.mapping(path, depth, span.start.col())?;
                self.anchor(anchor, &node);
                Ok(node)
            }
            _ => Err(self.error("a value was expected here")),
        }
    }

    fn anchor(&mut self, anchor: usize, node: &Node) {
        if anchor != 0 {
            self.anchors.insert(anchor, node.clone());
        }
    }

    fn sequence(&mut self, path: &Path, depth: usize, column: usize) -> Result<Node, ParseError> {
        let mut items = Vec::new();
        while !matches!(self.event(), Event::SequenceEnd) {
            let before = self.above();
            let mut item = self.node(&path.index(items.len()), depth + 1)?;
            item.set_comment(joined(before, item.comment().map(str::to_owned)));
            self.trailing(&mut item);
            items.push(item);
        }
        let end = self.ending(column);
        self.at += 1;
        let place = if items.is_empty() {
            CommentPlace::Start
        } else {
            CommentPlace::End
        };
        let mut node = Node::new(Value::Array(items));
        node.set_comment_at(place, end);
        Ok(node)
    }

    fn mapping(&mut self, path: &Path, depth: usize, column: usize) -> Result<Node, ParseError> {
        let mut fields = indexmap::IndexMap::new();
        let mut merges = Vec::new();
        while !matches!(self.event(), Event::MappingEnd) {
            let before = self.above();
            let (key, merge) = match self.event() {
                Event::Scalar(text, style, _, tag) => {
                    let tag = self.tag(tag.as_deref())?;
                    let merge = text == "<<" && *style == ScalarStyle::Plain && tag.is_none();
                    (text.to_string(), merge)
                }
                _ => {
                    return Err(self.error(
                        "a key that is a list, a table or an alias cannot be read: keys are text",
                    ));
                }
            };
            if fields.contains_key(&key) {
                return Err(self.error(format!("the key {key} is in this table twice")));
            }
            self.at += 1;
            let mut after_key = Node::new(Value::Null);
            self.trailing(&mut after_key);
            let mut value = self.node(&path.key(key.clone()), depth + 1)?;
            let comment = joined(before, after_key.comment().map(str::to_owned));
            value.set_comment(joined(comment, value.comment().map(str::to_owned)));
            self.trailing(&mut value);
            if merge {
                merges.push(value);
            } else {
                fields.insert(key, value);
            }
        }
        let end = self.ending(column);
        self.at += 1;
        for merged in merges {
            let sources: Vec<&Node> = match merged.value() {
                Value::Array(items) => items.iter().collect(),
                _ => vec![&merged],
            };
            for source in sources {
                if let Value::Object(source) = source.value() {
                    for (key, value) in source {
                        fields.entry(key.clone()).or_insert_with(|| value.clone());
                    }
                }
            }
            self.warnings.push(Warning {
                path: path.clone(),
                message: "the fields of a merge key (<<) were copied in; they are written out in full when saved".into(),
            });
        }
        let place = if fields.is_empty() {
            CommentPlace::Start
        } else {
            CommentPlace::End
        };
        let mut node = Node::new(Value::Object(fields));
        node.set_comment_at(place, end);
        Ok(node)
    }

    fn scalar(
        &mut self,
        text: &str,
        style: ScalarStyle,
        tag: Option<&str>,
        path: &Path,
    ) -> Result<Value, ParseError> {
        let wrong = |b: &Self, what: &str| b.error(format!("{text:?} is not {what}"));
        match tag {
            Some("str") => return Ok(Value::Str(text.to_owned())),
            Some("null") => return Ok(Value::Null),
            Some("bool") => {
                return bool_of(text)
                    .map(Value::Bool)
                    .ok_or_else(|| wrong(self, "a boolean"));
            }
            Some("int") => {
                return match self.int(text, path)? {
                    Some(n) => Ok(n),
                    None => Err(wrong(self, "an integer")),
                };
            }
            Some("float") => {
                return match self.int(text, path)? {
                    Some(Value::Int(n)) => Ok(Value::Float(n as f64)),
                    Some(v) => Ok(v),
                    None => match self.float(text)? {
                        Some(x) => Ok(x),
                        None => Err(wrong(self, "a number")),
                    },
                };
            }
            Some(_) => return Err(self.error("this tag does not go with a single value")),
            None => {}
        }
        if style != ScalarStyle::Plain {
            return Ok(Value::Str(text.to_owned()));
        }
        if is_null(text) {
            return Ok(Value::Null);
        }
        if let Some(b) = bool_of(text) {
            return Ok(Value::Bool(b));
        }
        if let Some(n) = self.int(text, path)? {
            return Ok(n);
        }
        if let Some(x) = self.float(text)? {
            return Ok(x);
        }
        Ok(Value::Str(text.to_owned()))
    }

    /// An integer of the core schema: decimal, `0o` octal or `0x` hex. One
    /// beyond 64 bits is read as a float, with a warning.
    fn int(&mut self, text: &str, path: &Path) -> Result<Option<Value>, ParseError> {
        let parsed = if let Some(octal) = text.strip_prefix("0o") {
            is_digits(octal, 8).then(|| i64::from_str_radix(octal, 8))
        } else if let Some(hex) = text.strip_prefix("0x") {
            is_digits(hex, 16).then(|| i64::from_str_radix(hex, 16))
        } else {
            let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
            is_digits(digits, 10).then(|| text.parse::<i64>())
        };
        Ok(match parsed {
            None => None,
            Some(Ok(n)) => Some(Value::Int(n)),
            Some(Err(_)) => {
                self.warnings.push(Warning {
                    path: path.clone(),
                    message:
                        "integer beyond the signed 64-bit range was read as a floating-point number"
                            .into(),
                });
                let x = text.parse::<f64>().unwrap_or(f64::MAX);
                Some(Value::Float(x))
            }
        })
    }

    fn float(&self, text: &str) -> Result<Option<Value>, ParseError> {
        if INFINITE.is_match(text) {
            return Err(self.error(format!(
                "{text} cannot be read: ClioTree holds only finite numbers"
            )));
        }
        Ok(FLOAT
            .is_match(text)
            .then(|| text.parse::<f64>().ok())
            .flatten()
            .map(Value::Float))
    }
}

fn is_digits(text: &str, radix: u32) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_digit(radix))
}

fn is_null(text: &str) -> bool {
    matches!(text, "" | "~" | "null" | "Null" | "NULL")
}

fn bool_of(text: &str) -> Option<bool> {
    match text {
        "true" | "True" | "TRUE" => Some(true),
        "false" | "False" | "FALSE" => Some(false),
        _ => None,
    }
}

/// Writes a tree as YAML of one document. A node that holds an error value
/// is left out, as in JSON.
pub fn to_string(node: &Node) -> Result<String, WriteError> {
    if matches!(node.value(), Value::Error(_)) {
        return Err(WriteError {
            path: Path::root(),
            message: "the root holds an error value".into(),
        });
    }
    let mut out = String::new();
    document(&mut out, node, &Path::root())?;
    Ok(out)
}

/// Writes a list as YAML documents, each after a `---` line.
pub fn documents_to_string(node: &Node) -> Result<String, WriteError> {
    let Value::Array(items) = node.value() else {
        return Err(WriteError {
            path: Path::root(),
            message: "YAML documents are a list of documents, so the root must be an array".into(),
        });
    };
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if matches!(item.value(), Value::Error(_)) {
            continue;
        }
        out.push_str("---\n");
        document(&mut out, item, &Path::root().index(i))?;
    }
    Ok(out)
}

fn document(out: &mut String, node: &Node, path: &Path) -> Result<(), WriteError> {
    if let Some(before) = own_comment(node) {
        comment(out, Some(&before), 0);
        out.push('\n');
    }
    if is_block(node) {
        block(out, node, path, 0)?;
    } else {
        out.push_str(&scalar(node, path, 2)?);
        out.push('\n');
    }
    if let Some(end) = node
        .comment_at(CommentPlace::End)
        .filter(|_| !is_block(node))
    {
        out.push('\n');
        comment(out, Some(end), 0);
    }
    Ok(())
}

/// Whether a node is written as indented lines below its key: a list or
/// table with something in it.
fn is_block(node: &Node) -> bool {
    match node.value() {
        Value::Array(items) => items.iter().any(|i| !skipped(i)),
        Value::Object(fields) => fields.values().any(|v| !skipped(v)),
        _ => false,
    }
}

fn skipped(node: &Node) -> bool {
    matches!(node.value(), Value::Error(_))
}

/// The comment written above a node: its own, and for something not written
/// as a block those inside it, which have no other place.
fn own_comment(node: &Node) -> Option<String> {
    let own = node.comment().map(str::to_owned);
    if is_block(node) {
        return own;
    }
    let inside = joined(
        node.comment_at(CommentPlace::Start).map(str::to_owned),
        node.comment_at(CommentPlace::End).map(str::to_owned),
    );
    match node.value() {
        Value::Array(_) | Value::Object(_) => joined(own, inside),
        _ => own,
    }
}

fn comment(out: &mut String, comment: Option<&str>, indent: usize) {
    for line in comment.into_iter().flat_map(str::lines) {
        push_indent(out, indent);
        out.push('#');
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
        out.push('\n');
    }
}

fn push_indent(out: &mut String, indent: usize) {
    out.extend(std::iter::repeat_n(' ', indent));
}

/// The entries of a list or table with something in it, each line indented
/// by `indent`, then the comment at its end.
fn block(out: &mut String, node: &Node, path: &Path, indent: usize) -> Result<(), WriteError> {
    comment(out, node.comment_at(CommentPlace::Start), indent);
    match node.value() {
        Value::Object(fields) => {
            for (key, value) in fields.iter().filter(|(_, v)| !skipped(v)) {
                comment(out, own_comment(value).as_deref(), indent);
                push_indent(out, indent);
                out.push_str(&key_text(key));
                out.push(':');
                after(out, value, &path.key(key.clone()), indent)?;
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate().filter(|(_, i)| !skipped(i)) {
                comment(out, own_comment(item).as_deref(), indent);
                let path = path.index(i);
                if is_block(item) {
                    // The first entry goes on the line of the dash, unless
                    // it has a comment above it.
                    let mut inner = String::new();
                    block(&mut inner, item, &path, indent + 2)?;
                    push_indent(out, indent);
                    if inner[indent + 2..].starts_with('#') {
                        out.push_str("-\n");
                        out.push_str(&inner);
                    } else {
                        out.push_str("- ");
                        out.push_str(&inner[indent + 2..]);
                    }
                } else {
                    push_indent(out, indent);
                    out.push('-');
                    after(out, item, &path, indent)?;
                }
            }
        }
        _ => {}
    }
    comment(out, node.comment_at(CommentPlace::End), indent);
    Ok(())
}

/// The rest of a line that has a key or a dash on it: the value, or the
/// indented lines of a block below it.
fn after(out: &mut String, node: &Node, path: &Path, indent: usize) -> Result<(), WriteError> {
    if is_block(node) {
        out.push('\n');
        return block(out, node, path, indent + 2);
    }
    out.push(' ');
    out.push_str(&scalar(node, path, indent + 2)?);
    out.push('\n');
    Ok(())
}

/// A value that is not a block. Text of several lines becomes a literal
/// block, indented by `indent`.
fn scalar(node: &Node, path: &Path, indent: usize) -> Result<String, WriteError> {
    Ok(match node.value() {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(x) => {
            if !x.is_finite() {
                return Err(WriteError {
                    path: path.clone(),
                    message: "NaN and infinity are not written".into(),
                });
            }
            format!("{x:?}")
        }
        Value::Str(s) => match literal(s, indent) {
            Some(block) => block,
            None => text(s),
        },
        Value::Date(d) => text(&d.format("%Y-%m-%d").to_string()),
        Value::DateTime(DateTimeValue::Naive(dt)) => {
            text(&dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
        }
        Value::DateTime(DateTimeValue::Zoned(dt)) => text(&dt.to_rfc3339()),
        Value::Array(_) => "[]".to_owned(),
        Value::Object(_) => "{}".to_owned(),
        Value::Error(_) => "null".to_owned(),
    })
}

/// Text of several lines as a literal block (`|`), if it can be one: its
/// lines are written as they are, below the key and indented.
fn literal(s: &str, indent: usize) -> Option<String> {
    let body = s.trim_end_matches('\n');
    let breaks = s.len() - body.len();
    if !body.contains('\n') && breaks == 0 {
        return None;
    }
    let first = body.lines().find(|l| !l.is_empty())?;
    if first.starts_with([' ', '\t'])
        || body
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return None;
    }
    let mut out = String::from(match breaks {
        0 => "|-",
        1 => "|",
        _ => "|+",
    });
    for line in body.split('\n') {
        out.push('\n');
        if !line.is_empty() {
            push_indent(&mut out, indent);
            out.push_str(line);
        }
    }
    for _ in 1..breaks {
        out.push('\n');
    }
    Some(out)
}

fn key_text(key: &str) -> String {
    text(key)
}

/// Text plain if it would be read back as the same text, else in double
/// quotes.
fn text(s: &str) -> String {
    if plain(s) { s.to_owned() } else { quoted(s) }
}

fn plain(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    if s.starts_with(char::is_whitespace)
        || s.ends_with(char::is_whitespace)
        || s.ends_with(':')
        || s.contains(": ")
        || s.contains(" #")
        || s.chars().any(char::is_control)
        || "-?:,[]{}#&*!|>'\"%@`".contains(first)
    {
        return false;
    }
    // Words other YAML readers (YAML 1.1) take for booleans or null, and
    // anything that starts like a number, are quoted.
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "null" | "~" | "true" | "false" | "yes" | "no" | "on" | "off" | "y" | "n"
    ) {
        return false;
    }
    let unsigned = s.strip_prefix(['-', '+']).unwrap_or(s);
    !unsigned.starts_with(|c: char| c.is_ascii_digit() || c == '.')
}

fn quoted(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
#[path = "yaml_tests.rs"]
mod yaml_tests;
