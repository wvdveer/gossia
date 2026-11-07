//! The line syntax iCalendar (RFC 5545, §3.1) and vCard (RFC 6350, §3.3;
//! RFC 2425 and vCard 2.1 before it) share: folded lines,
//! `NAME;PARAM=value:value`, and `BEGIN:` / `END:` around components
//! (DESIGN.md, Calendars and contacts, 0.4.0). The calendar and contact
//! readers take what this gives them.

use gossia_i18n::tr;

/// One property line, unfolded: its name in capitals without a vCard group
/// (`item1.EMAIL` is `EMAIL`), its parameters, and its value as written (the
/// escapes are left for the reader, which knows whether the value is a list).
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    /// Each parameter's name in capitals and its values, unquoted. A vCard 2.1
    /// parameter written without a name (`TEL;HOME:`) is a `TYPE`.
    pub params: Vec<(String, Vec<String>)>,
    pub value: String,
}

impl Property {
    /// The first value of a parameter.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.first())
            .map(String::as_str)
    }

    /// Every value of a parameter, across repeats of it (`TYPE=work;TYPE=voice`).
    pub fn params_all(&self, name: &str) -> Vec<&str> {
        self.params
            .iter()
            .filter(|(n, _)| n == name)
            .flat_map(|(_, v)| v.iter().map(String::as_str))
            .collect()
    }

    /// The value as text: unescaped, quoted-printable decoded.
    pub fn text(&self) -> String {
        unescape(&self.value)
    }

    /// The value as a list separated by `sep`, each part unescaped. An
    /// escaped separator stays inside its part.
    pub fn list(&self, sep: char) -> Vec<String> {
        split_escaped(&self.value, sep)
            .iter()
            .map(|p| unescape(p))
            .collect()
    }
}

/// A component (`VCALENDAR`, `VEVENT`, `VCARD`, ...) with its properties and
/// the components inside it, in the file's order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Component {
    pub name: String,
    pub properties: Vec<Property>,
    pub children: Vec<Component>,
}

impl Component {
    /// The first property of a name.
    pub fn get(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// Every property of a name.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> + 'a {
        self.properties.iter().filter(move |p| p.name == name)
    }
}

/// The components at the top of a file. Lines outside any component and
/// empty lines are passed over; an `END` that does not close the open
/// component, a component left open, or a line without a `:` is refused,
/// naming its line.
pub fn parse(text: &str) -> Result<Vec<Component>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut stack: Vec<Component> = Vec::new();
    let mut top = Vec::new();
    for (number, line) in unfold(text) {
        if line.trim().is_empty() {
            continue;
        }
        let property = parse_line(&line).ok_or_else(|| {
            tr!(
                "line {number} is not a property: it has no colon",
                number = number
            )
        })?;
        match property.name.as_str() {
            "BEGIN" => stack.push(Component {
                name: property.value.trim().to_ascii_uppercase(),
                ..Component::default()
            }),
            "END" => {
                let name = property.value.trim().to_ascii_uppercase();
                let Some(done) = stack.pop() else {
                    return Err(tr!(
                        "line {number} ends {name}, which was not begun",
                        number = number,
                        name = name
                    ));
                };
                if done.name != name {
                    return Err(tr!(
                        "line {number} ends {name}, but {open} is the one open",
                        number = number,
                        name = name,
                        open = done.name
                    ));
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => top.push(done),
                }
            }
            _ => {
                if let Some(open) = stack.last_mut() {
                    open.properties.push(property);
                }
            }
        }
    }
    if let Some(open) = stack.last() {
        return Err(tr!(
            "{name} is begun but never ended",
            name = open.name.clone()
        ));
    }
    Ok(top)
}

/// The logical lines, each with the number of the line it starts on: a line
/// that starts with a space or a tab continues the one before (the space is
/// dropped), and a quoted-printable value that ends in `=` goes on to the
/// next line (vCard 2.1).
fn unfold(text: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut soft_break = false;
    for (index, raw) in text.split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(last) = out.last_mut() {
            if let Some(rest) = raw.strip_prefix([' ', '\t']) {
                last.1.push_str(rest);
                continue;
            }
            if soft_break {
                last.1.pop();
                last.1.push_str(raw);
                soft_break = quoted_printable_open(&last.1);
                continue;
            }
        }
        out.push((index + 1, raw.to_owned()));
        soft_break = quoted_printable_open(raw);
    }
    out
}

/// Whether a line is a quoted-printable property whose value goes on to the
/// next line.
fn quoted_printable_open(line: &str) -> bool {
    line.ends_with('=') && {
        let head = line.split(':').next().unwrap_or("").to_ascii_uppercase();
        head.contains("QUOTED-PRINTABLE")
    }
}

/// One unfolded line as a property, or None when it has no `:` outside
/// quotes.
fn parse_line(line: &str) -> Option<Property> {
    // The name, then `;` parameters, then `:` and the value; a quoted
    // parameter value may hold `;`, `:` and `,`.
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?.trim().to_ascii_uppercase();
    // A vCard group: `item1.EMAIL`.
    let name = match name.rsplit_once('.') {
        Some((_, n)) => n.to_owned(),
        None => name,
    };
    let mut params = Vec::new();
    for part in parts {
        match part.split_once('=') {
            Some((n, v)) => {
                let values = split_unquoted(v, ',')
                    .into_iter()
                    .map(|v| v.trim().trim_matches('"').to_owned())
                    .collect();
                params.push((n.trim().to_ascii_uppercase(), values));
            }
            None if !part.trim().is_empty() => {
                params.push(("TYPE".to_owned(), vec![part.trim().to_owned()]));
            }
            None => {}
        }
    }
    let mut property = Property {
        name,
        params,
        value: value.to_owned(),
    };
    decode_quoted_printable(&mut property);
    Some(property)
}

/// Parts of `s` between `sep`s that are not inside double quotes.
fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (i, c) in s.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == sep && !in_quotes {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

/// A vCard 2.1 value in quoted-printable (`ENCODING=QUOTED-PRINTABLE`, or a
/// bare `QUOTED-PRINTABLE` parameter), decoded in its `CHARSET` (UTF-8 when
/// none is given; ISO-8859-1 and Windows-1252 byte for byte otherwise).
fn decode_quoted_printable(property: &mut Property) {
    let qp = property.params.iter().any(|(n, v)| {
        (n == "ENCODING" || n == "TYPE")
            && v.iter().any(|v| v.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
    });
    if !qp {
        return;
    }
    let mut bytes = Vec::with_capacity(property.value.len());
    let raw = property.value.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'=' {
            let hex = raw
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok());
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                bytes.push(byte);
                i += 3;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let utf8 = property
        .param("CHARSET")
        .is_none_or(|c| c.eq_ignore_ascii_case("UTF-8"));
    property.value = match String::from_utf8(bytes) {
        Ok(s) if utf8 => s,
        Ok(s) if s.is_ascii() => s,
        Ok(s) => s.bytes().map(char::from).collect(),
        Err(e) => e.into_bytes().into_iter().map(char::from).collect(),
    };
    property.params.retain(|(n, v)| {
        !(n == "ENCODING"
            || (n == "TYPE" && v.iter().any(|v| v.eq_ignore_ascii_case("QUOTED-PRINTABLE"))))
    });
}

/// A text value with its escapes taken out: `\n` and `\N` are line breaks,
/// `\,`, `\;`, `\:` and `\\` the characters themselves.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A value split at each `sep` that is not escaped, the escapes kept for
/// [`unescape`].
pub fn split_escaped(s: &str, sep: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let last = out.last_mut().expect("never empty");
            last.push(c);
            if let Some(next) = chars.next() {
                last.push(next);
            }
        } else if c == sep {
            out.push(String::new());
        } else {
            out.last_mut().expect("never empty").push(c);
        }
    }
    out
}

/// Text escaped for a value: `\`, `;`, `,` and line breaks (iCalendar
/// `TEXT`, vCard 3.0).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.replace("\r\n", "\n").chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' | '\r' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// A property line folded at 75 bytes, as both formats require, each piece
/// after the first starting with a space, never splitting a character; ends
/// in `\r\n`.
pub fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3 + 2);
    let mut width = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
#[path = "contentline_tests.rs"]
mod tests;
