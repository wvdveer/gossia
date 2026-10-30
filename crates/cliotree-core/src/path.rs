//! Paths into a data tree.
//!
//! A path is the address of a node within one tab's tree, for example
//! `items[2].price`. The tab name that precedes it in a formula is not part of
//! the path. Paths are literal: they mean exactly what they say, whatever the
//! data does (see DESIGN.md, "References are literal").

use std::fmt;
use std::str::FromStr;

/// One step of a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Segment {
    /// A field of an object or a key of a map.
    Key(String),
    /// A zero-based position in an array.
    Index(usize),
}

/// A sequence of steps from the root of a tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Path(Vec<Segment>);

/// The text given to [`Path::from_str`] is not a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePathError {
    pub position: usize,
    pub message: &'static str,
}

impl fmt::Display for ParsePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid path at position {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for ParsePathError {}

impl Path {
    /// The empty path, which addresses the root.
    pub fn root() -> Path {
        Path(Vec::new())
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn segments(&self) -> &[Segment] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, segment: Segment) {
        self.0.push(segment);
    }

    pub fn pop(&mut self) -> Option<Segment> {
        self.0.pop()
    }

    /// This path extended by a step.
    pub fn child(&self, segment: Segment) -> Path {
        let mut p = self.clone();
        p.push(segment);
        p
    }

    /// This path extended by a key.
    pub fn key(&self, key: impl Into<String>) -> Path {
        let mut p = self.clone();
        p.push(Segment::Key(key.into()));
        p
    }

    /// This path extended by an index.
    pub fn index(&self, index: usize) -> Path {
        let mut p = self.clone();
        p.push(Segment::Index(index));
        p
    }

    /// This path followed by the steps of `rest`, which is relative to it.
    pub fn join(&self, rest: &Path) -> Path {
        let mut p = self.clone();
        p.0.extend(rest.0.iter().cloned());
        p
    }

    /// The path of the containing node, or `None` for the root.
    pub fn parent(&self) -> Option<Path> {
        let (_, rest) = self.0.split_last()?;
        Some(Path(rest.to_vec()))
    }
}

impl From<Vec<Segment>> for Path {
    fn from(segments: Vec<Segment>) -> Path {
        Path(segments)
    }
}

/// Whether a key can be written as `.name` (a Python-style identifier).
fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

impl fmt::Display for Path {
    /// Writes `a.b[2]["odd key"]`. Keys that are not identifiers are quoted.
    /// The root is written as the empty string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, segment) in self.0.iter().enumerate() {
            match segment {
                Segment::Index(i) => write!(f, "[{i}]")?,
                Segment::Key(k) if is_identifier(k) => {
                    if n > 0 {
                        f.write_str(".")?;
                    }
                    f.write_str(k)?;
                }
                Segment::Key(k) => {
                    f.write_str("[\"")?;
                    for c in k.chars() {
                        if c == '"' || c == '\\' {
                            f.write_str("\\")?;
                        }
                        write!(f, "{c}")?;
                    }
                    f.write_str("\"]")?;
                }
            }
        }
        Ok(())
    }
}

impl FromStr for Path {
    type Err = ParsePathError;

    /// Parses the text written by `Display`.
    fn from_str(text: &str) -> Result<Path, ParsePathError> {
        let err = |position, message| ParsePathError { position, message };
        let chars: Vec<char> = text.chars().collect();
        let mut segments = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '[' => {
                    i += 1;
                    if chars.get(i) == Some(&'"') {
                        i += 1;
                        let mut key = String::new();
                        loop {
                            match chars.get(i) {
                                None => return Err(err(i, "unterminated quoted key")),
                                Some('\\') => match chars.get(i + 1) {
                                    Some(&c @ ('"' | '\\')) => {
                                        key.push(c);
                                        i += 2;
                                    }
                                    _ => {
                                        return Err(err(
                                            i,
                                            "only a quote or a backslash may be escaped",
                                        ));
                                    }
                                },
                                Some('"') => {
                                    i += 1;
                                    break;
                                }
                                Some(&c) => {
                                    key.push(c);
                                    i += 1;
                                }
                            }
                        }
                        if chars.get(i) != Some(&']') {
                            return Err(err(i, "expected ]"));
                        }
                        i += 1;
                        segments.push(Segment::Key(key));
                    } else {
                        let start = i;
                        while chars.get(i).is_some_and(char::is_ascii_digit) {
                            i += 1;
                        }
                        if i == start {
                            return Err(err(i, "expected an index or a quoted key"));
                        }
                        if chars.get(i) != Some(&']') {
                            return Err(err(i, "expected ]"));
                        }
                        let digits: String = chars[start..i].iter().collect();
                        let index = digits.parse().map_err(|_| err(start, "index too large"))?;
                        i += 1;
                        segments.push(Segment::Index(index));
                    }
                }
                '.' if !segments.is_empty() => {
                    i += 1;
                    let start = i;
                    i = identifier_end(&chars, i);
                    if i == start {
                        return Err(err(i, "expected a name after ."));
                    }
                    segments.push(Segment::Key(chars[start..i].iter().collect()));
                }
                c if segments.is_empty() && (c == '_' || c.is_alphabetic()) => {
                    let start = i;
                    i = identifier_end(&chars, i);
                    segments.push(Segment::Key(chars[start..i].iter().collect()));
                }
                _ => return Err(err(i, "unexpected character")),
            }
        }
        Ok(Path(segments))
    }
}

fn identifier_end(chars: &[char], mut i: usize) -> usize {
    while chars
        .get(i)
        .is_some_and(|c| *c == '_' || c.is_alphanumeric())
    {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn parse(text: &str) -> Path {
        text.parse().unwrap()
    }

    #[test]
    fn displays_like_python_member_syntax() {
        let p = Path::root().key("items").index(2).key("price");
        assert_eq!(p.to_string(), "items[2].price");
        assert_eq!(Path::root().to_string(), "");
        assert_eq!(Path::root().index(0).key("a").to_string(), "[0].a");
    }

    #[test]
    fn quotes_keys_that_are_not_identifiers() {
        let p = Path::root().key("@id").key("a b").key("q\"\\");
        assert_eq!(p.to_string(), r#"["@id"]["a b"]["q\"\\"]"#);
        assert_eq!(parse(&p.to_string()), p);
    }

    #[test]
    fn parses_what_it_displays() {
        assert_eq!(
            parse("a.b[3].c"),
            Path::root().key("a").key("b").index(3).key("c")
        );
        assert_eq!(parse(""), Path::root());
        assert_eq!(parse("[1][2]"), Path::root().index(1).index(2));
    }

    #[test]
    fn rejects_malformed_paths() {
        for bad in [
            "a.",
            "a..b",
            ".a",
            "a[",
            "a[x]",
            "a[1",
            "a[\"x",
            "a[\"x\"",
            "1a",
            "a b",
            "a[\"\\n\"]",
        ] {
            assert!(bad.parse::<Path>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn parent_of_root_is_none() {
        assert_eq!(Path::root().parent(), None);
        assert_eq!(parse("a.b").parent(), Some(parse("a")));
    }

    fn segment() -> impl Strategy<Value = Segment> {
        prop_oneof![
            any::<u32>().prop_map(|i| Segment::Index(i as usize)),
            ".{0,6}".prop_map(Segment::Key),
        ]
    }

    proptest! {
        #[test]
        fn display_then_parse_round_trips(segs in prop::collection::vec(segment(), 0..6)) {
            let path = Path::from(segs);
            prop_assert_eq!(path.to_string().parse::<Path>().unwrap(), path);
        }
    }
}
