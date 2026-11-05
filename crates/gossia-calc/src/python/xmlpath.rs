//! `XMLPATH`: queries over XML data that do not fit member syntax (DESIGN.md,
//! "Name resolution").
//!
//! A subset of XPath 1.0 over the data tree, where an element is an object (or
//! its value, if it is simple), a child element is a field named after it, a
//! repeated element is an array whose items are the occurrences, an attribute
//! is the field `@name`, and the text of an element that also has attributes is
//! the field `#text`:
//!
//! - steps `name`, `*`, `@name`, `@*`, `text()` and `.`, separated by `/`, and
//!   `//` for any depth;
//! - predicates in `[...]`: a position (`[2]`), `last()`, comparisons
//!   (`= != < <= > >=`) between a path, a number or a string, `and`, `or`,
//!   `not(...)`, parentheses, a path alone (it exists), and the functions
//!   `position()`, `last()`, `count(path)`, `contains(a, b)` and
//!   `starts-with(a, b)`.
//!
//! The node the query is given is the context: a path starts there, and a
//! leading `/` means the same (so `/line/sku` is `line/sku`, with the tab as
//! the root element). Axes other than child, attribute and descendant, unions,
//! and arithmetic are refused with a message.

use gossia_core::{Node, Segment, Value};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Query {
    steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
struct Step {
    /// `//` before the step: at any depth below the context.
    anywhere: bool,
    test: Test,
    predicates: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
enum Test {
    /// A child element.
    Name(String),
    /// Any child element.
    Any,
    /// An attribute.
    Attribute(String),
    AnyAttribute,
    Text,
    /// The context itself.
    This,
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Compare(Box<Expr>, Cmp, Box<Expr>),
    /// A relative path: true if it selects something; as an operand, its first
    /// match.
    Path(Query),
    Number(f64),
    Text(String),
    Position,
    Last,
    Count(Query),
    Contains(Box<Expr>, Box<Expr>),
    StartsWith(Box<Expr>, Box<Expr>),
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
    p.blanks();
    let query = p.query(true)?;
    p.blanks();
    if p.pos < p.text.len() {
        return Err(p.fail("unexpected text"));
    }
    Ok(query)
}

fn is_name_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic() || c >= 0x80
}

fn is_name_char(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == b'-' || c == b'.'
}

impl Parser<'_> {
    fn fail(&self, message: &str) -> String {
        format!("{message} at position {}", self.pos)
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn blanks(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.text[self.pos..].starts_with(s.as_bytes()) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.pos;
        if !self.peek().is_some_and(is_name_start) {
            return None;
        }
        while self.peek().is_some_and(is_name_char) {
            self.pos += 1;
        }
        // A name does not end in `.` or `-`; those belong to what follows.
        Some(String::from_utf8_lossy(&self.text[start..self.pos]).into_owned())
    }

    /// A path. `top` is the outermost one, which may start with `/` or `//`.
    fn query(&mut self, top: bool) -> Result<Query, String> {
        let mut steps = Vec::new();
        let mut anywhere = false;
        if self.eat("//") {
            anywhere = true;
        } else if top && self.peek() == Some(b'/') {
            self.pos += 1;
            self.blanks();
            if self.pos >= self.text.len() {
                // A lone `/` is the context itself.
                return Ok(Query {
                    steps: vec![Step {
                        anywhere: false,
                        test: Test::This,
                        predicates: Vec::new(),
                    }],
                });
            }
        }
        loop {
            steps.push(self.step(anywhere)?);
            if self.eat("//") {
                anywhere = true;
            } else if self.peek() == Some(b'/') {
                self.pos += 1;
                anywhere = false;
            } else {
                break;
            }
        }
        Ok(Query { steps })
    }

    fn step(&mut self, anywhere: bool) -> Result<Step, String> {
        let test = if self.eat("..") {
            return Err(self.fail("the parent axis `..` is not supported"));
        } else if self.eat(".") {
            Test::This
        } else if self.eat("@*") {
            Test::AnyAttribute
        } else if self.eat("@") {
            match self.name() {
                Some(n) => Test::Attribute(n),
                None => return Err(self.fail("expected an attribute name after @")),
            }
        } else if self.eat("*") {
            Test::Any
        } else if let Some(n) = self.name() {
            if self.peek() == Some(b'(') {
                if n == "text" && self.eat("()") {
                    Test::Text
                } else {
                    return Err(self.fail(&format!("the node test `{n}()` is not supported")));
                }
            } else if self.text[self.pos..].starts_with(b"::") {
                return Err(self.fail(&format!("the axis `{n}::` is not supported")));
            } else {
                Test::Name(n)
            }
        } else {
            return Err(self.fail("expected a name, `*`, `@name`, `text()` or `.`"));
        };
        let mut predicates = Vec::new();
        while self.peek() == Some(b'[') {
            self.pos += 1;
            self.blanks();
            let expr = self.or()?;
            self.blanks();
            if !self.eat("]") {
                return Err(self.fail("expected ]"));
            }
            predicates.push(expr);
        }
        Ok(Step {
            anywhere,
            test,
            predicates,
        })
    }

    fn keyword(&mut self, word: &str) -> bool {
        self.blanks();
        let at = self.pos;
        if self.eat(word) && !self.peek().is_some_and(is_name_char) {
            return true;
        }
        self.pos = at;
        false
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.keyword("or") {
            let right = self.and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.comparison()?;
        while self.keyword("and") {
            let right = self.comparison()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let left = self.operand()?;
        self.blanks();
        let op = if self.eat("!=") {
            Cmp::Ne
        } else if self.eat("<=") {
            Cmp::Le
        } else if self.eat(">=") {
            Cmp::Ge
        } else if self.eat("=") {
            Cmp::Eq
        } else if self.eat("<") {
            Cmp::Lt
        } else if self.eat(">") {
            Cmp::Gt
        } else {
            return Ok(left);
        };
        let right = self.operand()?;
        Ok(Expr::Compare(Box::new(left), op, Box::new(right)))
    }

    fn operand(&mut self) -> Result<Expr, String> {
        self.blanks();
        let Some(c) = self.peek() else {
            return Err(self.fail("the query ends early"));
        };
        match c {
            b'(' => {
                self.pos += 1;
                let inner = self.or()?;
                self.blanks();
                if !self.eat(")") {
                    return Err(self.fail("expected )"));
                }
                Ok(inner)
            }
            b'\'' | b'"' => {
                self.pos += 1;
                let start = self.pos;
                while self.peek().is_some_and(|d| d != c) {
                    self.pos += 1;
                }
                if self.peek() != Some(c) {
                    return Err(self.fail("a string is not closed"));
                }
                let s = String::from_utf8_lossy(&self.text[start..self.pos]).into_owned();
                self.pos += 1;
                Ok(Expr::Text(s))
            }
            b'0'..=b'9' | b'-' => {
                let start = self.pos;
                self.pos += 1;
                while self
                    .peek()
                    .is_some_and(|d| d.is_ascii_digit() || d == b'.' || d == b'e' || d == b'E')
                {
                    self.pos += 1;
                }
                let s = std::str::from_utf8(&self.text[start..self.pos]).unwrap_or_default();
                s.parse::<f64>()
                    .map(Expr::Number)
                    .map_err(|_| self.fail("not a number"))
            }
            _ => {
                // A function call, or a path.
                let at = self.pos;
                if let Some(name) = self.name()
                    && self.peek() == Some(b'(')
                    && !matches!(name.as_str(), "text")
                {
                    self.pos += 1;
                    return self.call(&name);
                }
                self.pos = at;
                Ok(Expr::Path(self.query(false)?))
            }
        }
    }

    fn call(&mut self, name: &str) -> Result<Expr, String> {
        self.blanks();
        let result = match name {
            "last" => Expr::Last,
            "position" => Expr::Position,
            "not" => Expr::Not(Box::new(self.or()?)),
            "count" => Expr::Count(self.query(false)?),
            "contains" | "starts-with" => {
                let a = self.or()?;
                self.blanks();
                if !self.eat(",") {
                    return Err(self.fail("expected a comma between the arguments"));
                }
                let b = self.or()?;
                if name == "contains" {
                    Expr::Contains(Box::new(a), Box::new(b))
                } else {
                    Expr::StartsWith(Box::new(a), Box::new(b))
                }
            }
            other => {
                return Err(self.fail(&format!("the function `{other}()` is not supported")));
            }
        };
        self.blanks();
        if !self.eat(")") {
            return Err(self.fail("expected )"));
        }
        Ok(result)
    }
}

// -------------------------------------------------------------- evaluating

/// A node that was selected, with the way to it from the context.
pub(super) type Match = (Vec<Segment>, Node);

/// The nodes `query` selects, from `context`, in document order, once each.
pub(super) fn select(query: &Query, context: &Node) -> Vec<Match> {
    // A sequence of repeated elements is each of its occurrences.
    let start: Vec<Match> = match context.value() {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, n)| (vec![Segment::Index(i)], n.clone()))
            .collect(),
        _ => vec![(Vec::new(), context.clone())],
    };
    walk(query, start)
}

fn walk(query: &Query, mut current: Vec<Match>) -> Vec<Match> {
    for step in &query.steps {
        let mut next: Vec<Match> = Vec::new();
        for context in &current {
            let from: Vec<Match> = if step.anywhere {
                let mut all = Vec::new();
                descendants_or_self(context, &mut all);
                all
            } else {
                vec![context.clone()]
            };
            for origin in &from {
                let mut found = apply_test(&step.test, origin);
                for predicate in &step.predicates {
                    let total = found.len();
                    found = found
                        .into_iter()
                        .enumerate()
                        .filter(|(i, m)| keep(predicate, m, i + 1, total))
                        .map(|(_, m)| m)
                        .collect();
                }
                next.extend(found);
            }
        }
        // Once each, in the order first met.
        let mut seen = std::collections::HashSet::new();
        next.retain(|(path, _)| seen.insert(format!("{path:?}")));
        current = next;
    }
    current
}

fn descendants_or_self(at: &Match, out: &mut Vec<Match>) {
    // Without recursion, so depth is bounded by the data, not the stack.
    let mut stack = vec![at.clone()];
    while let Some(m) = stack.pop() {
        out.push(m.clone());
        let mut kids = apply_test(&Test::Any, &m);
        kids.reverse();
        stack.extend(kids);
    }
}

fn child(path: &[Segment], key: &str, node: &Node, out: &mut Vec<Match>) {
    let mut at = path.to_vec();
    at.push(Segment::Key(key.to_string()));
    match node.value() {
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let mut with = at.clone();
                with.push(Segment::Index(i));
                out.push((with, item.clone()));
            }
        }
        _ => out.push((at, node.clone())),
    }
}

fn apply_test(test: &Test, (path, node): &Match) -> Vec<Match> {
    let mut out = Vec::new();
    let Value::Object(fields) = node.value() else {
        // A simple element has no children or attributes; its text is itself.
        if matches!(test, Test::Text | Test::This) {
            out.push((path.clone(), node.clone()));
        }
        return out;
    };
    match test {
        Test::This => out.push((path.clone(), node.clone())),
        Test::Name(name) => {
            if let Some(n) = fields.get(name) {
                child(path, name, n, &mut out);
            }
        }
        Test::Any => {
            for (key, n) in fields {
                if !key.starts_with('@') && key != "#text" {
                    child(path, key, n, &mut out);
                }
            }
        }
        Test::Attribute(name) => {
            let key = format!("@{name}");
            if let Some(n) = fields.get(&key) {
                child(path, &key, n, &mut out);
            }
        }
        Test::AnyAttribute => {
            for (key, n) in fields {
                if key.starts_with('@') {
                    child(path, key, n, &mut out);
                }
            }
        }
        Test::Text => {
            if let Some(n) = fields.get("#text") {
                child(path, "#text", n, &mut out);
            }
        }
    }
    out
}

fn keep(predicate: &Expr, at: &Match, position: usize, last: usize) -> bool {
    match evaluate(predicate, at, position, last) {
        // A number is a position: `[2]`, `[last()]`.
        Out::Number(n) => n == position as f64,
        other => truth(&other),
    }
}

/// What an expression came to.
#[derive(Debug, Clone)]
enum Out {
    Bool(bool),
    Number(f64),
    Text(String),
    /// The values a path selected.
    Nodes(Vec<Node>),
}

fn truth(out: &Out) -> bool {
    match out {
        Out::Bool(b) => *b,
        Out::Number(n) => *n != 0.0,
        Out::Text(t) => !t.is_empty(),
        Out::Nodes(n) => !n.is_empty(),
    }
}

fn evaluate(expr: &Expr, at: &Match, position: usize, last: usize) -> Out {
    match expr {
        Expr::Or(a, b) => Out::Bool(
            truth(&evaluate(a, at, position, last)) || truth(&evaluate(b, at, position, last)),
        ),
        Expr::And(a, b) => Out::Bool(
            truth(&evaluate(a, at, position, last)) && truth(&evaluate(b, at, position, last)),
        ),
        Expr::Not(a) => Out::Bool(!truth(&evaluate(a, at, position, last))),
        Expr::Number(n) => Out::Number(*n),
        Expr::Text(t) => Out::Text(t.clone()),
        Expr::Position => Out::Number(position as f64),
        Expr::Last => Out::Number(last as f64),
        Expr::Path(q) => Out::Nodes(
            walk(q, vec![at.clone()])
                .into_iter()
                .map(|(_, n)| n)
                .collect(),
        ),
        Expr::Count(q) => Out::Number(walk(q, vec![at.clone()]).len() as f64),
        Expr::Contains(a, b) => {
            let (a, b) = (
                text_of(&evaluate(a, at, position, last)),
                text_of(&evaluate(b, at, position, last)),
            );
            Out::Bool(a.contains(&b))
        }
        Expr::StartsWith(a, b) => {
            let (a, b) = (
                text_of(&evaluate(a, at, position, last)),
                text_of(&evaluate(b, at, position, last)),
            );
            Out::Bool(a.starts_with(&b))
        }
        Expr::Compare(a, op, b) => Out::Bool(compare(
            &evaluate(a, at, position, last),
            *op,
            &evaluate(b, at, position, last),
        )),
    }
}

/// The text of a value, as XPath's `string()` gives it.
fn text_of(out: &Out) -> String {
    match out {
        Out::Bool(b) => b.to_string(),
        Out::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        Out::Text(t) => t.clone(),
        Out::Nodes(nodes) => nodes.first().map(node_text).unwrap_or_default(),
    }
}

/// The text of a node: the text it holds, or its `#text`.
fn node_text(node: &Node) -> String {
    match node.value() {
        Value::Object(fields) => fields.get("#text").map(node_text).unwrap_or_default(),
        other => gossia_core::xml::scalar_text(other).unwrap_or_default(),
    }
}

fn number_of(out: &Out) -> Option<f64> {
    match out {
        Out::Number(n) => Some(*n),
        Out::Bool(b) => Some(f64::from(u8::from(*b))),
        Out::Text(t) => t.trim().parse().ok(),
        Out::Nodes(nodes) => nodes.first().and_then(|n| match n.value() {
            Value::Int(i) => Some(*i as f64),
            Value::Float(x) => Some(*x),
            other => gossia_core::xml::scalar_text(other).and_then(|t| t.trim().parse().ok()),
        }),
    }
}

/// One selected node as a plain value, to compare.
fn single(node: &Node) -> Out {
    match node.value() {
        Value::Int(i) => Out::Number(*i as f64),
        Value::Float(x) => Out::Number(*x),
        _ => Out::Text(node_text(node)),
    }
}

fn compare(a: &Out, op: Cmp, b: &Out) -> bool {
    // A set of nodes is true if any of its members makes the comparison true.
    if let Out::Nodes(nodes) = a {
        return nodes.iter().any(|n| compare(&single(n), op, b));
    }
    if let Out::Nodes(nodes) = b {
        return nodes.iter().any(|n| compare(a, op, &single(n)));
    }
    let numeric = matches!(a, Out::Number(_)) || matches!(b, Out::Number(_));
    if numeric || matches!(op, Cmp::Lt | Cmp::Le | Cmp::Gt | Cmp::Ge) {
        let (Some(x), Some(y)) = (number_of(a), number_of(b)) else {
            return matches!(op, Cmp::Ne);
        };
        return match op {
            Cmp::Eq => x == y,
            Cmp::Ne => x != y,
            Cmp::Lt => x < y,
            Cmp::Le => x <= y,
            Cmp::Gt => x > y,
            Cmp::Ge => x >= y,
        };
    }
    let (x, y) = (text_of(a), text_of(b));
    match op {
        Cmp::Eq => x == y,
        Cmp::Ne => x != y,
        _ => unreachable!("ordering is numeric"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
      <xs:element name="order"><xs:complexType>
        <xs:sequence>
          <xs:element name="customer" type="xs:string"/>
          <xs:element name="line" maxOccurs="unbounded"><xs:complexType>
            <xs:sequence>
              <xs:element name="sku" type="xs:string"/>
              <xs:element name="qty" type="xs:integer"/>
              <xs:element name="price" type="xs:decimal"/>
            </xs:sequence>
            <xs:attribute name="id" type="xs:integer" use="required"/>
          </xs:complexType></xs:element>
          <xs:element name="note" type="xs:string" minOccurs="0"/>
        </xs:sequence>
        <xs:attribute name="status" type="xs:string"/>
      </xs:complexType></xs:element>
    </xs:schema>"#;

    fn data() -> Node {
        let spec = gossia_core::xsd::lower_str(ORDER).unwrap();
        gossia_core::xml::parse(
            r#"<order status="open"><customer>Acme</customer>
              <line id="1"><sku>A</sku><qty>2</qty><price>9.5</price></line>
              <line id="2"><sku>B</sku><qty>1</qty><price>24</price></line>
              <line id="3"><sku>C</sku><qty>12</qty><price>0.75</price></line>
              <note>hi</note></order>"#,
            &spec,
        )
        .unwrap()
        .node
    }

    fn run(query: &str) -> Vec<String> {
        let q = parse(query).unwrap_or_else(|e| panic!("{query}: {e}"));
        select(&q, &data())
            .into_iter()
            .map(|(_, n)| node_text(&n))
            .collect()
    }

    fn paths(query: &str) -> Vec<String> {
        let q = parse(query).unwrap();
        select(&q, &data())
            .into_iter()
            .map(|(p, _)| gossia_core::Path::from(p).to_string())
            .collect()
    }

    #[test]
    fn child_steps_walk_elements_and_repeated_elements_flatten() {
        assert_eq!(run("customer"), ["Acme"]);
        assert_eq!(run("line/sku"), ["A", "B", "C"]);
        assert_eq!(
            run("/line/sku"),
            ["A", "B", "C"],
            "a leading slash is the context"
        );
        assert_eq!(
            paths("line/sku"),
            ["line[0].sku", "line[1].sku", "line[2].sku"]
        );
        assert_eq!(run("*/sku"), ["A", "B", "C"]);
        assert!(run("nothing").is_empty());
    }

    #[test]
    fn attributes_and_text_are_steps() {
        assert_eq!(run("@status"), ["open"]);
        assert_eq!(run("line/@id"), ["1", "2", "3"]);
        assert_eq!(run("line[1]/@*"), ["1"]);
        assert_eq!(run("customer/text()"), ["Acme"]);
        assert_eq!(paths("line/@id")[0], r#"line[0]["@id"]"#);
    }

    #[test]
    fn two_slashes_search_at_any_depth() {
        assert_eq!(run("//sku"), ["A", "B", "C"]);
        assert_eq!(run("line//qty"), ["2", "1", "12"]);
        assert_eq!(run("//@id"), ["1", "2", "3"]);
        // Once each, even when the descent can reach a node two ways.
        assert_eq!(run("//line//sku").len(), 3);
    }

    #[test]
    fn positions_and_last() {
        assert_eq!(run("line[1]/sku"), ["A"]);
        assert_eq!(run("line[2]/sku"), ["B"]);
        assert_eq!(run("line[last()]/sku"), ["C"]);
        assert_eq!(run("line[position() > 1]/sku"), ["B", "C"]);
        assert!(run("line[9]").is_empty());
    }

    #[test]
    fn predicates_compare_paths_numbers_and_strings() {
        assert_eq!(run("line[qty > 1]/sku"), ["A", "C"]);
        assert_eq!(run("line[@id = '2']/sku"), ["B"]);
        assert_eq!(run("line[@id = 2]/sku"), ["B"]);
        assert_eq!(run("line[sku != 'B']/@id"), ["1", "3"]);
        assert_eq!(run("line[price <= 9.5 and qty >= 2]/sku"), ["A", "C"]);
        assert_eq!(run("line[qty = 1 or qty = 12]/sku"), ["B", "C"]);
        assert_eq!(run("line[not(qty > 1)]/sku"), ["B"]);
        assert_eq!(
            run("line[(qty = 1 or qty = 2) and price > 9]/sku"),
            ["A", "B"]
        );
    }

    #[test]
    fn a_path_alone_tests_that_it_exists_and_functions_work() {
        assert_eq!(run("*[@id]/sku"), ["A", "B", "C"]);
        assert_eq!(run("line[note]").len(), 0);
        assert_eq!(run("line[contains(sku, 'B')]/@id"), ["2"]);
        assert_eq!(run("line[starts-with(sku, 'A')]/@id"), ["1"]);
        assert_eq!(run("line[count(sku) = 1]/@id"), ["1", "2", "3"]);
    }

    #[test]
    fn what_is_not_supported_is_refused_with_a_message() {
        for (query, word) in [
            ("..", "parent"),
            ("a/..", "parent"),
            ("ancestor::a", "axis"),
            ("a[sum(b) > 1]", "sum"),
            ("a[", "ends early"),
            ("a[b", "expected ]"),
            ("a|b", "unexpected"),
            ("a[b + 1]", "expected"),
            ("", "expected"),
            ("a[contains(b)]", "comma"),
            ("node()", "node test"),
        ] {
            let e = parse(query).unwrap_err();
            assert!(e.contains(word), "{query:?}: {e}");
        }
    }

    #[test]
    fn a_sequence_of_occurrences_is_queried_item_by_item() {
        let node = data();
        let lines = node.get(&"line".parse().unwrap()).unwrap().clone();
        let q = parse("sku").unwrap();
        let texts: Vec<_> = select(&q, &lines)
            .iter()
            .map(|(_, n)| node_text(n))
            .collect();
        assert_eq!(texts, ["A", "B", "C"]);
        // The paths are from the sequence.
        assert_eq!(
            select(&q, &lines)
                .into_iter()
                .map(|(p, _)| gossia_core::Path::from(p).to_string())
                .collect::<Vec<_>>(),
            ["[0].sku", "[1].sku", "[2].sku"]
        );
    }

    #[test]
    fn deep_data_does_not_overflow_the_stack() {
        // A chain of nested elements, searched at any depth.
        let mut node = Node::new(Value::Str("leaf".into()));
        for _ in 0..100 {
            let mut map = indexmap::IndexMap::new();
            map.insert("a".to_string(), node);
            node = Node::new(Value::Object(map));
        }
        let q = parse("//a").unwrap();
        assert_eq!(select(&q, &node).len(), 100);
    }
}
