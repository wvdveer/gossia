//! `COPY` statements in a copybook (DESIGN.md, "COBOL copybooks", "COPY"):
//! the member named is read in its place, with the words of `REPLACING`
//! changed.

use super::{Copybook, ParseError, Token, literal_words, parse_tokens, tokens};

/// How deep members may copy in members.
const DEPTH: usize = 10;

/// Reads the text of a copybook, copying in the member each `COPY`
/// statement names: `load` gives the text of a member by its name, or None
/// when there is none.
pub fn parse_with(
    text: &str,
    load: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<Copybook, ParseError> {
    let (tokens, end_comment) = tokens(text)?;
    let tokens = expand(tokens, load, 0)?;
    parse_tokens(tokens, end_comment)
}

/// One pair of `REPLACING`: the words looked for, and those put in.
struct Pair {
    from: Vec<String>,
    to: Vec<String>,
    mode: Mode,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Whole words, one or more in a row.
    Words,
    /// The start of a word (`LEADING`).
    Leading,
    /// The end of a word (`TRAILING`).
    Trailing,
}

fn expand(
    tokens: Vec<Token>,
    load: &mut dyn FnMut(&str) -> Option<String>,
    depth: usize,
) -> Result<Vec<Token>, ParseError> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    // The comment to put on the next token: one after a member's last.
    let mut pending: Option<String> = None;
    let mut i = 0;
    while i < tokens.len() {
        let starts = i == 0 || tokens[i - 1].ends;
        if !(starts && tokens[i].text.eq_ignore_ascii_case("COPY")) {
            let mut token = tokens[i].clone();
            token.comment = join(pending.take(), token.comment);
            out.push(token);
            i += 1;
            continue;
        }
        let line = tokens[i].line;
        let error = |message: String| ParseError { line, message };
        let end = (i..tokens.len())
            .find(|&k| tokens[k].ends)
            .ok_or_else(|| error("this COPY statement has no period at its end".into()))?;
        let statement = &tokens[i + 1..=end];
        let (name, pairs) = statement_of(statement).map_err(error)?;
        if depth >= DEPTH {
            return Err(error(format!(
                "COPY {name}: members copy in members more than {DEPTH} deep"
            )));
        }
        let text = load(&name)
            .ok_or_else(|| error(format!("COPY {name}: there is no member {name} to copy in")))?;
        let (member, end_comment) =
            super::tokens(&text).map_err(|e| error(format!("COPY {name}: {e}")))?;
        let member = expand(member, load, depth + 1)?;
        let mut member = replace(member, &pairs);
        for token in &mut member {
            token.line = line;
        }
        if let Some(first) = member.first_mut() {
            first.comment = join(
                join(pending.take(), tokens[i].comment.clone()),
                first.comment.take(),
            );
        } else {
            pending = join(pending.take(), tokens[i].comment.clone());
        }
        out.extend(member);
        pending = join(pending.take(), end_comment);
        i = end + 1;
    }
    if let (Some(text), Some(last)) = (pending, out.last_mut()) {
        last.comment = join(last.comment.take(), Some(text));
    }
    Ok(out)
}

fn join(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
        (a, b) => a.or(b),
    }
}

/// The member's name and the pairs of `REPLACING` of a COPY statement after
/// the word COPY.
fn statement_of(words: &[Token]) -> Result<(String, Vec<Pair>), String> {
    let (first, rest) = words
        .split_first()
        .ok_or("COPY names the member to copy in")?;
    let name = unquote(&first.text);
    let upper = |k: usize| rest.get(k).map(|t| t.text.to_ascii_uppercase());
    let mut k = 0;
    // A library (OF or IN) is not looked in: members are beside the file.
    if matches!(upper(k).as_deref(), Some("OF" | "IN")) {
        k += 2;
    }
    if upper(k).as_deref() == Some("SUPPRESS") {
        k += 1;
    }
    let mut pairs = Vec::new();
    match upper(k).as_deref() {
        None => {}
        Some("REPLACING") => {
            let text: Vec<&str> = rest[k + 1..].iter().map(|t| t.text.as_str()).collect();
            pairs = pairs_of(&text.join(" "))?;
        }
        Some(word) => return Err(format!("COPY {name}: {word} is not understood here")),
    }
    Ok((name, pairs))
}

fn unquote(text: &str) -> String {
    let t = text.trim_matches(['\'', '"']);
    t.to_owned()
}

/// The pairs of `REPLACING`: `==a== BY ==b==`, `A BY B`, and `LEADING` or
/// `TRAILING` before a pair.
fn pairs_of(text: &str) -> Result<Vec<Pair>, String> {
    let mut scan = Scan { text, at: 0 };
    let mut pairs = Vec::new();
    while !scan.done() {
        let mut mode = Mode::Words;
        let mut from = scan.operand()?;
        if from.1 == Some(false) {
            match from.0.first().map(|w| w.to_ascii_uppercase()).as_deref() {
                Some("LEADING") if from.0.len() == 1 => mode = Mode::Leading,
                Some("TRAILING") if from.0.len() == 1 => mode = Mode::Trailing,
                _ => {}
            }
            if mode != Mode::Words {
                from = scan.operand()?;
            }
        }
        let by = scan.operand()?;
        if by.1 != Some(false) || !by.0[0].eq_ignore_ascii_case("BY") {
            return Err("REPLACING is a list of pairs, such as ==A== BY ==B==".into());
        }
        let to = scan.operand()?;
        if from.0.is_empty() {
            return Err("REPLACING looks for at least one word".into());
        }
        if mode != Mode::Words && (from.0.len() != 1 || to.0.len() > 1) {
            return Err("LEADING and TRAILING replace part of one word".into());
        }
        pairs.push(Pair {
            from: from.0,
            to: to.0,
            mode,
        });
    }
    Ok(pairs)
}

struct Scan<'a> {
    text: &'a str,
    at: usize,
}

impl Scan<'_> {
    fn done(&mut self) -> bool {
        self.skip();
        self.at >= self.text.len()
    }

    fn skip(&mut self) {
        let rest = &self.text[self.at..];
        self.at += rest.len() - rest.trim_start().len();
    }

    /// The next operand: the words of a pseudo-text and Some(true), or one
    /// word and Some(false).
    fn operand(&mut self) -> Result<(Vec<String>, Option<bool>), String> {
        self.skip();
        let rest = &self.text[self.at..];
        if rest.is_empty() {
            return Err("REPLACING ends before its pair does".into());
        }
        if let Some(inner) = rest.strip_prefix("==") {
            let end = inner
                .find("==")
                .ok_or("a pseudo-text of REPLACING has no == at its end")?;
            self.at += 2 + end + 2;
            return Ok((literal_words(&inner[..end]), Some(true)));
        }
        let word = literal_words(rest)
            .into_iter()
            .next()
            .ok_or("REPLACING ends before its pair does")?;
        self.at += rest.find(&word).unwrap_or(0) + word.len();
        Ok((vec![word], Some(false)))
    }
}

/// The member's tokens with the pairs of `REPLACING` put in: each place is
/// looked at once, with the first pair that matches there.
fn replace(tokens: Vec<Token>, pairs: &[Pair]) -> Vec<Token> {
    if pairs.is_empty() {
        return tokens;
    }
    let same = |a: &str, b: &str| {
        if a.starts_with(['\'', '"']) {
            a == b
        } else {
            a.eq_ignore_ascii_case(b)
        }
    };
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    'tokens: while i < tokens.len() {
        for pair in pairs {
            match pair.mode {
                Mode::Words => {
                    let n = pair.from.len();
                    let matched = i + n <= tokens.len()
                        && pair
                            .from
                            .iter()
                            .zip(&tokens[i..i + n])
                            .enumerate()
                            // Only the last word may end the entry.
                            .all(|(k, (w, t))| same(w, &t.text) && (k + 1 == n || !t.ends));
                    if matched {
                        let first = &tokens[i];
                        let ends = tokens[i + n - 1].ends;
                        let count = pair.to.len();
                        for (k, word) in pair.to.iter().enumerate() {
                            out.push(Token {
                                text: word.clone(),
                                line: first.line,
                                comment: if k == 0 { first.comment.clone() } else { None },
                                ends: ends && k + 1 == count,
                            });
                        }
                        if count == 0
                            && ends
                            && let Some(last) = out.last_mut()
                        {
                            last.ends = true;
                        }
                        i += n;
                        continue 'tokens;
                    }
                    // A tag in colons (`:PFX:`) is replaced inside words too.
                    let tag = &pair.from[0];
                    if n == 1 && tag.len() > 2 && tag.starts_with(':') && tag.ends_with(':') {
                        let upper = tokens[i].text.to_ascii_uppercase();
                        let wanted = tag.to_ascii_uppercase();
                        if upper.contains(&wanted) {
                            let mut token = tokens[i].clone();
                            token.text =
                                replace_ignoring_case(&token.text, &wanted, &pair.to.join(" "));
                            out.push(token);
                            i += 1;
                            continue 'tokens;
                        }
                    }
                }
                Mode::Leading | Mode::Trailing => {
                    let text = &tokens[i].text;
                    let upper = text.to_ascii_uppercase();
                    let part = pair.from[0].to_ascii_uppercase();
                    let to = pair.to.first().cloned().unwrap_or_default();
                    let new = if pair.mode == Mode::Leading && upper.starts_with(&part) {
                        Some(format!("{to}{}", &text[part.len()..]))
                    } else if pair.mode == Mode::Trailing && upper.ends_with(&part) {
                        Some(format!("{}{to}", &text[..text.len() - part.len()]))
                    } else {
                        None
                    };
                    if let Some(new) = new {
                        let mut token = tokens[i].clone();
                        token.text = new;
                        out.push(token);
                        i += 1;
                        continue 'tokens;
                    }
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

/// `text` with each `wanted` (upper case) put in place of by `with`,
/// whatever its case in `text`.
fn replace_ignoring_case(text: &str, wanted: &str, with: &str) -> String {
    let upper = text.to_ascii_uppercase();
    let mut out = String::new();
    let mut at = 0;
    while let Some(found) = upper[at..].find(wanted) {
        out.push_str(&text[at..at + found]);
        out.push_str(with);
        at += found + wanted.len();
    }
    out.push_str(&text[at..]);
    out
}
