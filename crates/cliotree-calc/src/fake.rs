//! A small stand-in for the Python engine, so that the calculation layer can be
//! built and tested before RustPython is added (DESIGN.md, Build order, step 2).
//!
//! It reads a tiny expression language that looks like the start of a Python
//! expression: numbers, strings, `True`, `False`, `None`, references such as
//! `Orders.items[2].price`, the operators `+ - * /` and the comparisons, and
//! the functions `SUM`, `IF`, `IFERROR` and `ISERROR`. It follows the design's
//! rules for errors: reading an errored node passes the error along, and using
//! its value raises it again with the same kind and origin.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cliotree_core::{ErrorKind, Num, Path, Segment, Value};

use crate::engine::{Context, Engine, Failure, Read, SyntaxError};

/// The stand-in engine. Counts how many formulas it has run, so tests can tell
/// whether a recalculation did more work than it needed to.
#[derive(Debug, Default, Clone)]
pub struct FakeEngine {
    evaluations: Arc<AtomicUsize>,
}

impl FakeEngine {
    pub fn new() -> FakeEngine {
        FakeEngine::default()
    }

    /// How many times [`Engine::eval`] has been called.
    pub fn evaluations(&self) -> usize {
        self.evaluations.load(Ordering::SeqCst)
    }
}

impl Engine for FakeEngine {
    fn check(&self, formula: &str) -> Result<(), SyntaxError> {
        parse(formula).map(|_| ())
    }

    fn eval(&self, formula: &str, ctx: &mut dyn Context) -> Result<Value, Failure> {
        self.evaluations.fetch_add(1, Ordering::SeqCst);
        let expr = parse(formula).map_err(|e| Failure::new(ErrorKind::Syntax, e.to_string()))?;
        eval(&expr, ctx)
    }
}

// ---------------------------------------------------------------- syntax

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    Punct(&'static str),
}

const PUNCT: &[&str] = &[
    "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "(", ")", "[", "]", ".", ",",
];

fn error_at(column: usize, message: impl Into<String>) -> SyntaxError {
    SyntaxError {
        message: message.into(),
        line: 1,
        column,
    }
}

fn tokenize(text: &str) -> Result<Vec<(Tok, usize)>, SyntaxError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let column = i + 1;
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut float = false;
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                float = true;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let digits: String = chars[start..i].iter().collect();
            let tok = if float {
                Tok::Float(digits.parse().expect("digits with one point"))
            } else {
                match digits.parse() {
                    Ok(n) => Tok::Int(n),
                    Err(_) => return Err(error_at(column, "integer is too large")),
                }
            };
            out.push((tok, column));
        } else if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != quote {
                i += 1;
            }
            if i >= chars.len() {
                return Err(error_at(column, "unterminated string"));
            }
            out.push((Tok::Str(chars[start..i].iter().collect()), column));
            i += 1;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push((Tok::Ident(chars[start..i].iter().collect()), column));
        } else {
            let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
            match PUNCT.iter().find(|p| rest.starts_with(**p)) {
                Some(p) => {
                    out.push((Tok::Punct(p), column));
                    i += p.len();
                }
                None => return Err(error_at(column, format!("unexpected character {c:?}"))),
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum Expr {
    Lit(Value),
    Ref { sheet: String, path: Path },
    Neg(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    end: usize,
}

fn parse(text: &str) -> Result<Expr, SyntaxError> {
    let toks = tokenize(text)?;
    if toks.is_empty() {
        return Err(error_at(1, "the formula is empty"));
    }
    let mut parser = Parser {
        toks,
        pos: 0,
        end: text.chars().count() + 1,
    };
    let expr = parser.comparison()?;
    match parser.toks.get(parser.pos) {
        None => Ok(expr),
        Some((_, column)) => Err(error_at(*column, "unexpected text after the expression")),
    }
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }

    fn column(&self) -> usize {
        self.toks.get(self.pos).map_or(self.end, |(_, c)| *c)
    }

    fn eat(&mut self, punct: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Punct(p)) if *p == punct) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, punct: &str) -> Result<(), SyntaxError> {
        if self.eat(punct) {
            Ok(())
        } else {
            Err(error_at(self.column(), format!("expected `{punct}`")))
        }
    }

    fn comparison(&mut self) -> Result<Expr, SyntaxError> {
        let left = self.additive()?;
        for op in ["==", "!=", "<=", ">=", "<", ">"] {
            if self.eat(op) {
                let right = self.additive()?;
                return Ok(Expr::Bin(op, Box::new(left), Box::new(right)));
            }
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.multiplicative()?;
        loop {
            let op = if self.eat("+") {
                "+"
            } else if self.eat("-") {
                "-"
            } else {
                return Ok(left);
            };
            let right = self.multiplicative()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.unary()?;
        loop {
            let op = if self.eat("*") {
                "*"
            } else if self.eat("/") {
                "/"
            } else {
                return Ok(left);
            };
            let right = self.unary()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> Result<Expr, SyntaxError> {
        if self.eat("-") {
            Ok(Expr::Neg(Box::new(self.unary()?)))
        } else {
            self.primary()
        }
    }

    fn primary(&mut self) -> Result<Expr, SyntaxError> {
        let column = self.column();
        let Some(tok) = self.peek().cloned() else {
            return Err(error_at(column, "the expression ends too soon"));
        };
        self.pos += 1;
        match tok {
            Tok::Int(n) => Ok(Expr::Lit(Value::Int(n))),
            Tok::Float(x) => Ok(Expr::Lit(Value::Float(x))),
            Tok::Str(s) => Ok(Expr::Lit(Value::Str(s))),
            Tok::Punct("(") => {
                let inner = self.comparison()?;
                self.expect(")")?;
                Ok(inner)
            }
            Tok::Ident(name) => match name.as_str() {
                "True" => Ok(Expr::Lit(Value::Bool(true))),
                "False" => Ok(Expr::Lit(Value::Bool(false))),
                "None" => Ok(Expr::Lit(Value::Null)),
                _ if self.eat("(") => {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.comparison()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    Ok(Expr::Call(name, args))
                }
                _ => self.reference(name),
            },
            Tok::Punct(p) => Err(error_at(column, format!("unexpected `{p}`"))),
        }
    }

    /// `Sheet.field[2]["key"]`: the first name is the tab.
    fn reference(&mut self, sheet: String) -> Result<Expr, SyntaxError> {
        let mut path = Path::root();
        loop {
            if self.eat(".") {
                let column = self.column();
                match self.peek().cloned() {
                    Some(Tok::Ident(name)) => {
                        self.pos += 1;
                        path.push(Segment::Key(name));
                    }
                    _ => return Err(error_at(column, "expected a field name after `.`")),
                }
            } else if self.eat("[") {
                let column = self.column();
                match self.peek().cloned() {
                    Some(Tok::Int(n)) if n >= 0 => {
                        self.pos += 1;
                        path.push(Segment::Index(n as usize));
                    }
                    Some(Tok::Str(key)) => {
                        self.pos += 1;
                        path.push(Segment::Key(key));
                    }
                    _ => return Err(error_at(column, "expected an index or a quoted key")),
                }
                self.expect("]")?;
            } else {
                return Ok(Expr::Ref { sheet, path });
            }
        }
    }
}

// ------------------------------------------------------------ evaluation

fn name_of(sheet: &str, path: &Path) -> String {
    let text = path.to_string();
    if text.is_empty() {
        sheet.to_string()
    } else if text.starts_with('[') {
        format!("{sheet}{text}")
    } else {
        format!("{sheet}.{text}")
    }
}

fn eval(expr: &Expr, ctx: &mut dyn Context) -> Result<Value, Failure> {
    match expr {
        Expr::Lit(v) => Ok(v.clone()),
        Expr::Ref { sheet, path } => read(ctx, sheet, path),
        Expr::Neg(inner) => match used(eval(inner, ctx)?)? {
            Value::Int(n) => n
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| Failure::new(ErrorKind::Value, "integer overflow")),
            Value::Float(x) => Ok(Value::Float(-x)),
            other => Err(type_error("unary -", &other)),
        },
        Expr::Bin(op, left, right) => {
            let l = used(eval(left, ctx)?)?;
            let r = used(eval(right, ctx)?)?;
            binary(op, l, r)
        }
        Expr::Call(name, args) => call(name, args, ctx),
    }
}

/// Reading an errored node gives its error. Reading below one gives the same
/// error, as the design says navigation does.
fn read(ctx: &mut dyn Context, sheet: &str, path: &Path) -> Result<Value, Failure> {
    match ctx.read(sheet, path) {
        Read::Node(node) => Ok(node.value().clone()),
        Read::NoSheet => Err(Failure::new(
            ErrorKind::Name,
            format!("there is no tab called {sheet}"),
        )),
        Read::Missing => {
            // Reading the whole tab to look for an error would make the formula
            // depend on everything, so the search stops short of the root.
            let mut above = path.clone();
            while above.pop().is_some() && !above.is_root() {
                match ctx.read(sheet, &above) {
                    Read::Node(node) => {
                        if let Value::Error(e) = node.value() {
                            return Ok(Value::Error(e.clone()));
                        }
                        break;
                    }
                    _ => continue,
                }
            }
            Err(Failure::new(
                ErrorKind::Ref,
                format!("there is nothing at {}", name_of(sheet, path)),
            ))
        }
    }
}

/// Using a value raises the error it holds. Only passing it along does not.
fn used(value: Value) -> Result<Value, Failure> {
    match value {
        Value::Error(e) => Err(Failure::propagated(&e)),
        other => Ok(other),
    }
}

fn type_error(what: &str, got: &Value) -> Failure {
    Failure::new(
        ErrorKind::Type,
        format!("unsupported operand for {what}: {}", got.kind_name()),
    )
}

fn number(value: &Value) -> Option<Num> {
    match value {
        Value::Int(n) => Some(Num::Int(*n)),
        Value::Float(x) => Some(Num::Float(*x)),
        _ => None,
    }
}

fn as_f64(n: Num) -> f64 {
    match n {
        Num::Int(i) => i as f64,
        Num::Float(x) => x,
    }
}

fn binary(op: &str, l: Value, r: Value) -> Result<Value, Failure> {
    if let (Value::Str(a), Value::Str(b), "+") = (&l, &r, op) {
        return Ok(Value::Str(format!("{a}{b}")));
    }
    if matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=") {
        return compare(op, &l, &r);
    }
    let (Some(a), Some(b)) = (number(&l), number(&r)) else {
        return Err(Failure::new(
            ErrorKind::Type,
            format!(
                "unsupported operand types: {} {op} {}",
                l.kind_name(),
                r.kind_name()
            ),
        ));
    };
    match (op, a, b) {
        ("/", _, _) => {
            let divisor = as_f64(b);
            if divisor == 0.0 {
                Err(Failure::new(ErrorKind::DivZero, "division by zero"))
            } else {
                Ok(Value::Float(as_f64(a) / divisor))
            }
        }
        (_, Num::Int(x), Num::Int(y)) => {
            let result = match op {
                "+" => x.checked_add(y),
                "-" => x.checked_sub(y),
                _ => x.checked_mul(y),
            };
            result
                .map(Value::Int)
                .ok_or_else(|| Failure::new(ErrorKind::Value, "integer overflow"))
        }
        _ => {
            let (x, y) = (as_f64(a), as_f64(b));
            Ok(Value::Float(match op {
                "+" => x + y,
                "-" => x - y,
                _ => x * y,
            }))
        }
    }
}

fn compare(op: &str, l: &Value, r: &Value) -> Result<Value, Failure> {
    use std::cmp::Ordering::*;
    let ordering = match (l, r) {
        (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
        _ => match (number(l), number(r)) {
            (Some(a), Some(b)) => a.compare(b),
            _ => None,
        },
    };
    let Some(ordering) = ordering else {
        return match op {
            "==" => Ok(Value::Bool(false)),
            "!=" => Ok(Value::Bool(true)),
            _ => Err(Failure::new(
                ErrorKind::Type,
                format!("cannot order {} and {}", l.kind_name(), r.kind_name()),
            )),
        };
    };
    Ok(Value::Bool(match op {
        "==" => ordering == Equal,
        "!=" => ordering != Equal,
        "<" => ordering == Less,
        ">" => ordering == Greater,
        "<=" => ordering != Greater,
        _ => ordering != Less,
    }))
}

fn call(name: &str, args: &[Expr], ctx: &mut dyn Context) -> Result<Value, Failure> {
    match name {
        "SUM" => {
            let mut total = Value::Int(0);
            for arg in args {
                match used(eval(arg, ctx)?)? {
                    Value::Array(items) => {
                        for item in items {
                            total = binary("+", total, used(item.value().clone())?)?;
                        }
                    }
                    scalar => total = binary("+", total, scalar)?,
                }
            }
            Ok(total)
        }
        "IF" => {
            let [condition, then, otherwise] = args else {
                return Err(Failure::new(ErrorKind::Type, "IF takes three arguments"));
            };
            match used(eval(condition, ctx)?)? {
                Value::Bool(true) => eval(then, ctx),
                Value::Bool(false) => eval(otherwise, ctx),
                other => Err(type_error("IF", &other)),
            }
        }
        "IFERROR" => {
            let [value, fallback] = args else {
                return Err(Failure::new(ErrorKind::Type, "IFERROR takes two arguments"));
            };
            match eval(value, ctx) {
                Ok(Value::Error(_)) | Err(_) => eval(fallback, ctx),
                ok => ok,
            }
        }
        "ISERROR" => {
            let [value] = args else {
                return Err(Failure::new(ErrorKind::Type, "ISERROR takes one argument"));
            };
            Ok(Value::Bool(matches!(
                eval(value, ctx),
                Ok(Value::Error(_)) | Err(_)
            )))
        }
        _ => Err(Failure::new(
            ErrorKind::Name,
            format!("the name {name} is not defined"),
        )),
    }
}
