//! Label templates (DESIGN.md, Forms, "Additions in 0.2.0, shared with
//! reports"): text in which each `{expression}` is filled in as in a Python
//! f-string, `{total:,.2f}` and `{name!r}` included, with `{{` and `}}` for the
//! braces themselves.
//!
//! A template is turned into one Python expression, which the engine runs as
//! it runs any expression of a control, and is split into its parts so that
//! an untrusted workbook can show the fixed text with a mark in place of each
//! expression.

/// One part of a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// Text shown as it is.
    Text(String),
    /// An expression, with its conversion (`r`, `s` or `a`) and its format
    /// specification, as an f-string writes them.
    Expr {
        text: String,
        conversion: Option<char>,
        spec: String,
    },
}

/// Splits a template into its parts, or says in words why it cannot be.
pub fn parts(template: &str) -> Result<Vec<Part>, String> {
    let chars: Vec<char> = template.chars().collect();
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                text.push('{');
                i += 2;
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                text.push('}');
                i += 2;
            }
            '}' => {
                return Err(
                    "a } with nothing opened before it: write }} for the brace itself".to_owned(),
                );
            }
            '{' => {
                let (part, next) = expression(&chars, i + 1)?;
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(part);
                i = next;
            }
            c => {
                text.push(c);
                i += 1;
            }
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    Ok(parts)
}

/// Reads the expression that starts at `start` (just after its `{`): the
/// part, and where the template goes on after its `}`.
fn expression(chars: &[char], start: usize) -> Result<(Part, usize), String> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut i = start;
    let mut end_of_text: Option<usize> = None;
    let mut conversion = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '}' if depth > 0 => depth -= 1,
            '}' => {
                let text_end = end_of_text.unwrap_or(i);
                let spec_start = end_of_text.map(|e| e + 1 + 2 * usize::from(conversion.is_some()));
                return finish(chars, start, text_end, conversion, spec_start, i);
            }
            '!' if depth == 0
                && end_of_text.is_none()
                && matches!(chars.get(i + 1), Some('r' | 's' | 'a'))
                && matches!(chars.get(i + 2), Some(':' | '}')) =>
            {
                conversion = chars.get(i + 1).copied();
                end_of_text = Some(i);
                i += 2;
                if chars.get(i) == Some(&':') {
                    // The specification runs to the closing brace.
                    let close = chars[i..].iter().position(|&c| c == '}').map(|p| p + i);
                    let Some(close) = close else {
                        return Err(unclosed());
                    };
                    return finish(
                        chars,
                        start,
                        end_of_text.unwrap_or(i),
                        conversion,
                        Some(i + 1),
                        close,
                    );
                }
                continue;
            }
            ':' if depth == 0 && end_of_text.is_none() && chars.get(i + 1) != Some(&'=') => {
                let close = chars[i..].iter().position(|&c| c == '}').map(|p| p + i);
                let Some(close) = close else {
                    return Err(unclosed());
                };
                return finish(chars, start, i, None, Some(i + 1), close);
            }
            _ => {}
        }
        i += 1;
    }
    Err(unclosed())
}

fn unclosed() -> String {
    "a { that is not closed: write {{ for the brace itself".to_owned()
}

fn finish(
    chars: &[char],
    start: usize,
    text_end: usize,
    conversion: Option<char>,
    spec_start: Option<usize>,
    close: usize,
) -> Result<(Part, usize), String> {
    let text: String = chars[start..text_end].iter().collect();
    if text.trim().is_empty() {
        return Err("an empty {} in the template: put an expression in it".to_owned());
    }
    let spec = match spec_start {
        Some(s) if s <= close => chars[s..close].iter().collect(),
        _ => String::new(),
    };
    Ok((
        Part::Expr {
            text: text.trim().to_owned(),
            conversion,
            spec,
        },
        close + 1,
    ))
}

/// The template as one Python expression that gives its text.
pub fn expression_of(template: &str) -> Result<String, String> {
    let parts = parts(template)?;
    let pieces: Vec<String> = parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => literal(text),
            Part::Expr {
                text,
                conversion,
                spec,
            } => {
                let value = match conversion {
                    Some('r') => format!("repr(({text}))"),
                    Some('a') => format!("ascii(({text}))"),
                    Some(_) => format!("str(({text}))"),
                    None => format!("({text})"),
                };
                format!("format({value}, {})", literal(spec))
            }
        })
        .collect();
    Ok(match pieces.len() {
        0 => "''".to_owned(),
        1 if matches!(parts[0], Part::Text(_)) => pieces[0].clone(),
        _ => format!("''.join(({},))", pieces.join(", ")),
    })
}

/// The template the parts make: the parts of [`parts`] give it back. A
/// brace in text is doubled, and an expression that starts with a brace gets
/// a space before it, so that `{{` does not read as a brace in text.
pub fn rebuild(parts: &[Part]) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            Part::Text(text) => out.push_str(&text.replace('{', "{{").replace('}', "}}")),
            Part::Expr {
                text,
                conversion,
                spec,
            } => {
                out.push('{');
                if text.starts_with('{') {
                    out.push(' ');
                }
                out.push_str(text);
                if let Some(c) = conversion {
                    out.push('!');
                    out.push(*c);
                }
                if !spec.is_empty() {
                    out.push(':');
                    out.push_str(spec);
                }
                out.push('}');
            }
        }
    }
    out
}

/// The template with each expression shown as `mark`, for a workbook whose
/// expressions do not run. A template that cannot be read is shown as it is.
pub fn with_marks(template: &str, mark: &str) -> String {
    match parts(template) {
        Ok(parts) => parts
            .iter()
            .map(|part| match part {
                Part::Text(text) => text.as_str(),
                Part::Expr { .. } => mark,
            })
            .collect(),
        Err(_) => template.to_owned(),
    }
}

/// Text as a Python string literal.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(text: &str, conversion: Option<char>, spec: &str) -> Part {
        Part::Expr {
            text: text.to_owned(),
            conversion,
            spec: spec.to_owned(),
        }
    }

    #[test]
    fn rebuilt_parts_read_back_the_same() {
        for template in [
            "Total: {SUM(r.qty for r in rows):,.2f} kg",
            "{name!r} and {x!s:>8}",
            "{{braces}} { {'a': 1}['a'] } {'}'}",
            "{[1, {2: 3}]} end",
        ] {
            let read = parts(template).unwrap();
            assert_eq!(parts(&rebuild(&read)).unwrap(), read, "{template}");
        }
        let read = vec![expr("{'a': 1}", None, ""), Part::Text("{x}".into())];
        assert_eq!(parts(&rebuild(&read)).unwrap(), read);
    }

    #[test]
    fn a_template_is_split_into_text_and_expressions() {
        assert_eq!(parts("").unwrap(), []);
        assert_eq!(parts("plain").unwrap(), [Part::Text("plain".into())]);
        assert_eq!(
            parts("Total: {SUM(r.qty for r in rows):,.2f} kg").unwrap(),
            [
                Part::Text("Total: ".into()),
                expr("SUM(r.qty for r in rows)", None, ",.2f"),
                Part::Text(" kg".into()),
            ]
        );
        assert_eq!(parts("{name!r}").unwrap(), [expr("name", Some('r'), "")]);
        assert_eq!(parts("{x!s:>8}").unwrap(), [expr("x", Some('s'), ">8")]);
        // Braces, colons and quotes inside the expression are its own.
        assert_eq!(
            parts("{ {'a': 1}['a'] } {d[1:2]} {'}'} {a != b} {(y := 3)}").unwrap(),
            [
                expr("{'a': 1}['a']", None, ""),
                Part::Text(" ".into()),
                expr("d[1:2]", None, ""),
                Part::Text(" ".into()),
                expr("'}'", None, ""),
                Part::Text(" ".into()),
                expr("a != b", None, ""),
                Part::Text(" ".into()),
                expr("(y := 3)", None, ""),
            ]
        );
        assert_eq!(
            parts("{{literal}} }}").unwrap(),
            [Part::Text("{literal} }".into())]
        );
    }

    #[test]
    fn a_template_that_cannot_be_read_says_why() {
        assert!(parts("open {x").unwrap_err().contains("not closed"));
        assert!(parts("close }").unwrap_err().contains("nothing opened"));
        assert!(parts("empty {} here").unwrap_err().contains("empty"));
        assert!(parts("{ }").unwrap_err().contains("empty"));
    }

    #[test]
    fn a_template_becomes_one_python_expression() {
        assert_eq!(expression_of("").unwrap(), "''");
        assert_eq!(expression_of("it's\n").unwrap(), r"'it\'s\n'");
        assert_eq!(
            expression_of("{n} orders").unwrap(),
            "''.join((format((n), ''), ' orders',))"
        );
        assert_eq!(
            expression_of("{x!r:>5}").unwrap(),
            "''.join((format(repr((x)), '>5'),))"
        );
        assert!(expression_of("{").is_err());
    }

    #[test]
    fn an_untrusted_template_shows_its_text_and_marks() {
        assert_eq!(with_marks("Total: {t:,.2f} kg", "#U"), "Total: #U kg");
        assert_eq!(with_marks("{{x}}", "#U"), "{x}");
        assert_eq!(with_marks("bad {", "#U"), "bad {");
    }
}
