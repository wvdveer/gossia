//! `JSONPATH`: queries that do not fit member syntax (DESIGN.md, "Name
//! resolution").
//!
//! A subset of RFC 9535 over the data tree: `$`, member names (`.name`,
//! `['name']`), indexes (`[2]`, `[-1]`), slices (`[1:3]`, `[::2]`), wildcards
//! (`.*`, `[*]`), the descendant operator (`..name`, `..*`, `..[?...]`), unions
//! of selectors (`['a','b']`, `[0,2]`), and filters with comparison, `&&`,
//! `||`, `!`, parentheses and tests for existence: `[?@.id == 7]`,
//! `[?(@.price > 1 && @.tag)]`. Functions (`length()`, `match()`...) and `$`
//! inside a filter are not supported, and the query is refused with a message
//! if it uses them.

use cliotree_core::{Node, Num, Value};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Query {
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
struct Segment {
    descendant: bool,
    selectors: Vec<Selector>,
}

#[derive(Debug, Clone, PartialEq)]
enum Selector {
    Name(String),
    Index(i64),
    Wildcard,
    Slice(Option<i64>, Option<i64>, Option<i64>),
    Filter(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Compare(Operand, Cmp, Operand),
    Exists(Query),
}

#[derive(Debug, Clone, PartialEq)]
enum Operand {
    Literal(Literal),
    Relative(Query),
}

#[derive(Debug, Clone, PartialEq)]
enum Literal {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

// ---------------------------------------------------------------- parsing

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
}

pub(super) fn parse(text: &str) -> Result<Query, String> {
    let mut p = Parser {
        text: text.as_bytes(),
        pos: 0,
    };
    p.skip_space();
    if !p.eat(b'$') {
        return Err("a JSONPath query starts with `$`".to_owned());
    }
    let query = p.segments()?;
    p.skip_space();
    if p.pos < p.text.len() {
        return Err(p.error("unexpected text after the query"));
    }
    Ok(query)
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("{message} (at position {})", self.pos + 1)
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, s: &str) -> bool {
        if self.text[self.pos..].starts_with(s.as_bytes()) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn segments(&mut self) -> Result<Query, String> {
        let mut segments = Vec::new();
        loop {
            self.skip_space_before_segment();
            let descendant = self.eat_str("..");
            if descendant {
                match self.peek() {
                    Some(b'[') => {}
                    Some(b'*') => {
                        self.pos += 1;
                        segments.push(Segment {
                            descendant,
                            selectors: vec![Selector::Wildcard],
                        });
                        continue;
                    }
                    _ => {
                        let name = self.name()?;
                        segments.push(Segment {
                            descendant,
                            selectors: vec![Selector::Name(name)],
                        });
                        continue;
                    }
                }
            } else if self.eat(b'.') {
                if self.eat(b'*') {
                    segments.push(Segment {
                        descendant,
                        selectors: vec![Selector::Wildcard],
                    });
                } else {
                    let name = self.name()?;
                    segments.push(Segment {
                        descendant,
                        selectors: vec![Selector::Name(name)],
                    });
                }
                continue;
            } else if self.peek() != Some(b'[') {
                return Ok(Query { segments });
            }
            // A bracketed segment.
            self.pos += 1;
            let mut selectors = vec![self.selector()?];
            loop {
                self.skip_space();
                if self.eat(b',') {
                    selectors.push(self.selector()?);
                } else {
                    break;
                }
            }
            self.skip_space();
            if !self.eat(b']') {
                return Err(self.error("expected `]`"));
            }
            segments.push(Segment {
                descendant,
                selectors,
            });
        }
    }

    /// Inside a filter, a segment ends at the first space.
    fn skip_space_before_segment(&mut self) {}

    fn name(&mut self) -> Result<String, String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80)
        {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(self.error("expected a name"));
        }
        Ok(String::from_utf8_lossy(&self.text[start..self.pos]).into_owned())
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.peek().ok_or_else(|| self.error("expected a string"))?;
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let c = self
                .peek()
                .ok_or_else(|| self.error("the string is not closed"))?;
            self.pos += 1;
            if c == quote {
                break;
            }
            if c == b'\\' {
                let e = self
                    .peek()
                    .ok_or_else(|| self.error("the string is not closed"))?;
                self.pos += 1;
                out.push(match e {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    other => other,
                });
            } else {
                out.push(c);
            }
        }
        String::from_utf8(out).map_err(|_| self.error("the string is not valid text"))
    }

    fn integer(&mut self) -> Result<Option<i64>, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if start == self.pos {
            return Ok(None);
        }
        std::str::from_utf8(&self.text[start..self.pos])
            .ok()
            .and_then(|t| t.parse().ok())
            .map(Some)
            .ok_or_else(|| self.error("expected an integer"))
    }

    fn selector(&mut self) -> Result<Selector, String> {
        self.skip_space();
        match self.peek() {
            Some(b'*') => {
                self.pos += 1;
                Ok(Selector::Wildcard)
            }
            Some(b'\'' | b'"') => Ok(Selector::Name(self.string()?)),
            Some(b'?') => {
                self.pos += 1;
                Ok(Selector::Filter(Box::new(self.or()?)))
            }
            _ => {
                let first = self.integer()?;
                self.skip_space();
                if self.eat(b':') {
                    self.skip_space();
                    let end = self.integer()?;
                    self.skip_space();
                    let step = if self.eat(b':') {
                        self.skip_space();
                        self.integer()?
                    } else {
                        None
                    };
                    Ok(Selector::Slice(first, end, step))
                } else {
                    match first {
                        Some(n) => Ok(Selector::Index(n)),
                        None => Err(self.error("expected a selector")),
                    }
                }
            }
        }
    }

    // Filter expressions: or > and > not > comparison.

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        loop {
            self.skip_space();
            if self.eat_str("||") {
                let right = self.and()?;
                left = Expr::Or(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.not()?;
        loop {
            self.skip_space();
            if self.eat_str("&&") {
                let right = self.not()?;
                left = Expr::And(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn not(&mut self) -> Result<Expr, String> {
        self.skip_space();
        if self.peek() == Some(b'!') && self.text.get(self.pos + 1) != Some(&b'=') {
            self.pos += 1;
            return Ok(Expr::Not(Box::new(self.not()?)));
        }
        if self.eat(b'(') {
            let inner = self.or()?;
            self.skip_space();
            if !self.eat(b')') {
                return Err(self.error("expected `)`"));
            }
            return Ok(inner);
        }
        let left = self.operand()?;
        self.skip_space();
        let op = if self.eat_str("==") {
            Cmp::Eq
        } else if self.eat_str("!=") {
            Cmp::Ne
        } else if self.eat_str("<=") {
            Cmp::Le
        } else if self.eat_str(">=") {
            Cmp::Ge
        } else if self.eat(b'<') {
            Cmp::Lt
        } else if self.eat(b'>') {
            Cmp::Gt
        } else {
            return match left {
                Operand::Relative(query) => Ok(Expr::Exists(query)),
                Operand::Literal(_) => Err(self.error("a filter needs a comparison here")),
            };
        };
        let right = self.operand()?;
        Ok(Expr::Compare(left, op, right))
    }

    fn operand(&mut self) -> Result<Operand, String> {
        self.skip_space();
        match self.peek() {
            Some(b'@') => {
                self.pos += 1;
                Ok(Operand::Relative(self.segments()?))
            }
            Some(b'$') => Err(self.error("`$` inside a filter is not supported")),
            Some(b'\'' | b'"') => Ok(Operand::Literal(Literal::Str(self.string()?))),
            Some(c) if c.is_ascii_digit() || c == b'-' => {
                let start = self.pos;
                self.pos += 1;
                while self.peek().is_some_and(|c| {
                    c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')
                }) {
                    self.pos += 1;
                }
                let text = std::str::from_utf8(&self.text[start..self.pos]).unwrap_or("");
                if let Ok(n) = text.parse::<i64>() {
                    Ok(Operand::Literal(Literal::Int(n)))
                } else if let Ok(x) = text.parse::<f64>() {
                    Ok(Operand::Literal(Literal::Float(x)))
                } else {
                    Err(self.error("expected a number"))
                }
            }
            _ => {
                if self.eat_str("true") {
                    Ok(Operand::Literal(Literal::Bool(true)))
                } else if self.eat_str("false") {
                    Ok(Operand::Literal(Literal::Bool(false)))
                } else if self.eat_str("null") {
                    Ok(Operand::Literal(Literal::Null))
                } else {
                    Err(self.error("expected `@`, a number, a string, true, false or null"))
                }
            }
        }
    }
}

// ------------------------------------------------------------- evaluating

/// A matched node and the steps that lead to it from where the query started.
pub(super) type Match<'a> = (Vec<cliotree_core::Segment>, &'a Node);

pub(super) fn select<'a>(query: &Query, root: &'a Node) -> Vec<Match<'a>> {
    let mut current: Vec<Match<'a>> = vec![(Vec::new(), root)];
    for segment in &query.segments {
        let mut next = Vec::new();
        for (path, node) in &current {
            let mut targets: Vec<Match<'a>> = Vec::new();
            if segment.descendant {
                descendants(path, node, &mut targets);
            } else {
                targets.push((path.clone(), node));
            }
            for (path, node) in targets {
                for selector in &segment.selectors {
                    apply(selector, &path, node, &mut next);
                }
            }
        }
        current = next;
    }
    current
}

fn descendants<'a>(path: &[cliotree_core::Segment], node: &'a Node, out: &mut Vec<Match<'a>>) {
    out.push((path.to_vec(), node));
    for (step, child) in children(node) {
        let mut inner = path.to_vec();
        inner.push(step);
        descendants(&inner, child, out);
    }
}

fn children(node: &Node) -> Vec<(cliotree_core::Segment, &Node)> {
    use cliotree_core::Segment;
    match node.value() {
        Value::Object(fields) => fields
            .iter()
            .map(|(k, n)| (Segment::Key(k.clone()), n))
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, n)| (Segment::Index(i), n))
            .collect(),
        _ => Vec::new(),
    }
}

fn apply<'a>(
    selector: &Selector,
    path: &[cliotree_core::Segment],
    node: &'a Node,
    out: &mut Vec<Match<'a>>,
) {
    use cliotree_core::Segment;
    let step = |s: Segment, n: &'a Node, out: &mut Vec<Match<'a>>| {
        let mut p = path.to_vec();
        p.push(s);
        out.push((p, n));
    };
    match selector {
        Selector::Name(name) => {
            if let Value::Object(fields) = node.value()
                && let Some(n) = fields.get(name)
            {
                step(Segment::Key(name.clone()), n, out);
            }
        }
        Selector::Index(index) => {
            if let Value::Array(items) = node.value() {
                let len = items.len() as i64;
                let at = if *index < 0 { index + len } else { *index };
                if (0..len).contains(&at) {
                    step(Segment::Index(at as usize), &items[at as usize], out);
                }
            }
        }
        Selector::Wildcard => {
            for (s, n) in children(node) {
                step(s, n, out);
            }
        }
        Selector::Slice(start, end, stride) => {
            if let Value::Array(items) = node.value() {
                for i in slice_indexes(items.len(), *start, *end, *stride) {
                    step(Segment::Index(i), &items[i], out);
                }
            }
        }
        Selector::Filter(expr) => {
            for (s, n) in children(node) {
                if truth(expr, n) {
                    step(s, n, out);
                }
            }
        }
    }
}

fn slice_indexes(
    len: usize,
    start: Option<i64>,
    end: Option<i64>,
    step: Option<i64>,
) -> Vec<usize> {
    let len = len as i64;
    let step = step.unwrap_or(1);
    if step == 0 {
        return Vec::new();
    }
    let norm = |i: i64| if i >= 0 { i } else { len + i };
    let mut out = Vec::new();
    if step > 0 {
        let lower = norm(start.unwrap_or(0)).clamp(0, len);
        let upper = norm(end.unwrap_or(len)).clamp(0, len);
        let mut i = lower;
        while i < upper {
            out.push(i as usize);
            i += step;
        }
    } else {
        let upper = norm(start.unwrap_or(len - 1)).clamp(-1, len - 1);
        let lower = norm(end.unwrap_or(-len - 1)).clamp(-1, len - 1);
        let mut i = upper;
        while lower < i {
            out.push(i as usize);
            i += step;
        }
    }
    out
}

fn truth(expr: &Expr, node: &Node) -> bool {
    match expr {
        Expr::Or(a, b) => truth(a, node) || truth(b, node),
        Expr::And(a, b) => truth(a, node) && truth(b, node),
        Expr::Not(a) => !truth(a, node),
        Expr::Exists(query) => !select(query, node).is_empty(),
        Expr::Compare(a, op, b) => compare(operand(a, node), *op, operand(b, node)),
    }
}

/// What an operand is for the node being tested. `None` is "nothing", which
/// is what a path that finds no node gives.
fn operand(op: &Operand, node: &Node) -> Option<Value> {
    match op {
        Operand::Literal(l) => Some(match l {
            Literal::Int(n) => Value::Int(*n),
            Literal::Float(x) => Value::Float(*x),
            Literal::Str(s) => Value::Str(s.clone()),
            Literal::Bool(b) => Value::Bool(*b),
            Literal::Null => Value::Null,
        }),
        Operand::Relative(query) => {
            let found = select(query, node);
            match found.as_slice() {
                [(_, n)] => Some(n.value().clone()),
                _ => None,
            }
        }
    }
}

fn number(v: &Value) -> Option<Num> {
    match v {
        Value::Int(n) => Some(Num::Int(*n)),
        Value::Float(x) => Some(Num::Float(*x)),
        _ => None,
    }
}

fn compare(a: Option<Value>, op: Cmp, b: Option<Value>) -> bool {
    use std::cmp::Ordering;
    let ordering = match (&a, &b) {
        (None, None) => Some(Ordering::Equal),
        (None, _) | (_, None) => None,
        (Some(Value::Error(_)), _) | (_, Some(Value::Error(_))) => None,
        (Some(x), Some(y)) => match (number(x), number(y)) {
            (Some(p), Some(q)) => p.compare(q),
            _ => match (x, y) {
                (Value::Str(p), Value::Str(q)) => Some(p.cmp(q)),
                (Value::Bool(p), Value::Bool(q)) => Some(p.cmp(q)),
                (Value::Null, Value::Null) => Some(Ordering::Equal),
                _ => None,
            },
        },
    };
    match (op, ordering) {
        (Cmp::Eq, o) => o == Some(Ordering::Equal),
        (Cmp::Ne, o) => o != Some(Ordering::Equal),
        (Cmp::Lt, Some(o)) => o == Ordering::Less,
        (Cmp::Le, Some(o)) => o != Ordering::Greater,
        (Cmp::Gt, Some(o)) => o == Ordering::Greater,
        (Cmp::Ge, Some(o)) => o != Ordering::Less,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cliotree_core::json;

    const DATA: &str = r#"{
      "store": {
        "book": [
          {"id": 1, "title": "A", "price": 8.95, "tag": "x"},
          {"id": 7, "title": "B", "price": 12.99},
          {"id": 9, "title": "C", "price": 8.99, "tag": "y"}
        ],
        "bicycle": {"colour": "red", "price": 19.95}
      },
      "n": [0, 1, 2, 3, 4, 5]
    }"#;

    fn run(query: &str) -> Vec<String> {
        let root = json::parse(DATA).unwrap().node;
        let q = parse(query).unwrap_or_else(|e| panic!("{query}: {e}"));
        select(&q, &root)
            .into_iter()
            .map(|(path, node)| {
                let steps: String = path
                    .iter()
                    .map(|s| match s {
                        cliotree_core::Segment::Key(k) => format!(".{k}"),
                        cliotree_core::Segment::Index(i) => format!("[{i}]"),
                    })
                    .collect();
                format!("{steps}={}", json::to_string(node, false).unwrap())
            })
            .collect()
    }

    #[test]
    fn members_indexes_and_wildcards() {
        assert_eq!(
            run("$.store.bicycle.colour"),
            [".store.bicycle.colour=\"red\""]
        );
        assert_eq!(
            run("$['store']['bicycle']['price']"),
            [".store.bicycle.price=19.95"]
        );
        assert_eq!(run("$.store.book[1].title"), [".store.book[1].title=\"B\""]);
        assert_eq!(run("$.store.book[-1].id"), [".store.book[2].id=9"]);
        assert_eq!(run("$.store.book[*].id").len(), 3);
        assert_eq!(run("$.n.*").len(), 6);
        assert_eq!(run("$.nothing"), Vec::<String>::new());
        assert_eq!(run("$.n[9]"), Vec::<String>::new());
    }

    #[test]
    fn slices_and_unions() {
        assert_eq!(run("$.n[1:3]"), [".n[1]=1", ".n[2]=2"]);
        assert_eq!(run("$.n[::2]"), [".n[0]=0", ".n[2]=2", ".n[4]=4"]);
        assert_eq!(run("$.n[-2:]"), [".n[4]=4", ".n[5]=5"]);
        assert_eq!(run("$.n[::-2]"), [".n[5]=5", ".n[3]=3", ".n[1]=1"]);
        assert_eq!(run("$.n[0,2]"), [".n[0]=0", ".n[2]=2"]);
        assert_eq!(
            run("$.store.bicycle['colour','price']"),
            [
                ".store.bicycle.colour=\"red\"",
                ".store.bicycle.price=19.95"
            ]
        );
    }

    #[test]
    fn the_descendant_operator_finds_at_any_depth() {
        let prices = run("$..price");
        assert_eq!(prices.len(), 4);
        assert_eq!(run("$..book[0].id"), [".store.book[0].id=1"]);
        assert_eq!(run("$..n[0]"), [".n[0]=0"]);
        assert!(run("$..*").len() > 10);
    }

    #[test]
    fn filters_compare_combine_and_test_for_existence() {
        assert_eq!(
            run("$.store.book[?@.id == 7].title"),
            [".store.book[1].title=\"B\""]
        );
        assert_eq!(run("$.store.book[?(@.price < 9)].id").len(), 2);
        assert_eq!(
            run("$.store.book[?@.price < 9 && @.id > 1].id"),
            [".store.book[2].id=9"]
        );
        assert_eq!(run("$.store.book[?@.id == 1 || @.id == 9].id").len(), 2);
        assert_eq!(run("$.store.book[?@.tag].id").len(), 2, "existence");
        assert_eq!(run("$.store.book[?!@.tag].id"), [".store.book[1].id=7"]);
        assert_eq!(
            run("$.store.book[?@.title == 'C'].id"),
            [".store.book[2].id=9"]
        );
        assert_eq!(run("$.store.book[?@.id != 1].id").len(), 2);
        assert_eq!(
            run("$.store.book[?@.price >= 12.99].id"),
            [".store.book[1].id=7"]
        );
        // A different type never compares equal, and a missing field is not 0.
        assert_eq!(run("$.store.book[?@.id == '7'].id"), Vec::<String>::new());
        assert_eq!(run("$.store.book[?@.tag < 5].id"), Vec::<String>::new());
    }

    #[test]
    fn bad_queries_say_what_is_wrong() {
        for (query, fragment) in [
            ("store", "starts with `$`"),
            ("$.", "expected a name"),
            ("$[", "expected a selector"),
            ("$[?@.a == ]", "expected"),
            ("$[?length(@.a) > 1]", "expected"),
            ("$[?@.a == $.b]", "`$` inside a filter"),
            ("$.a b", "unexpected text"),
            ("$['a", "not closed"),
        ] {
            let e = parse(query).unwrap_err();
            assert!(e.contains(fragment), "{query}: {e}");
        }
    }
}
