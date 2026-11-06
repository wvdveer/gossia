//! `.blk` files: one record laid out by a copybook (DESIGN.md, "COBOL
//! copybooks"), fixed length, or variable (VB) after a record descriptor
//! word. Text is EBCDIC in the copybook's code page; numbers are zoned,
//! packed, binary or floating point.
//!
//! Bytes that are not a valid number are read as an error value whose
//! message ends with the bytes (`X'4040'`), and such a value is written back
//! as those bytes, so a record goes in and comes out unchanged.

use gossia_i18n::tr;
use indexmap::IndexMap;

use super::FloatFormat;
use super::lower::{Counter, Element, Layout, NumUsage, Numeric, SignPlace, Slot};
use crate::json::{Parsed, Warning, WriteError};
use crate::path::{Path, Segment};
use crate::value::{ErrorKind, ErrorValue, Node, Value};

/// The `.blk` file is not records of the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError {
    pub message: String,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ReadError {}

/// Reads a `.blk` file as its one record. `choices` says, for each group of
/// layouts over the same bytes ([`Layout::redefines`]), which one is read;
/// one not given is the first. The file is the record's bytes, or a
/// variable-length record ([`has_rdw`]); any other is refused.
pub fn read(bytes: &[u8], layout: &Layout, choices: &[usize]) -> Result<Parsed, ReadError> {
    let padded;
    let bytes = if has_rdw(bytes, layout) {
        padded = variable_record(&bytes[4..], layout)?;
        &padded[..]
    } else if bytes.len() != layout.length {
        return Err(ReadError {
            message: tr!(
                "the file has {bytes} bytes, and a record of {record} has {length}",
                bytes = bytes.len(),
                record = layout.record,
                length = layout.length
            ),
        });
    } else {
        bytes
    };
    let mut reader = Reader {
        layout,
        choices,
        warnings: Vec::new(),
    };
    let mut fields = IndexMap::new();
    reader.group(&layout.slots, bytes, 0, bytes, &Path::root(), &mut fields);
    Ok(Parsed {
        node: unwrap_table(Node::new(Value::Object(fields)), layout),
        warnings: reader.warnings,
    })
}

/// Whether a file is one variable-length record (`RECFM=VB`): a record
/// descriptor word, whose first two bytes are the file's length and whose
/// other two are zero, and the record, which is no longer than the layout.
/// A file of exactly the layout's length is a fixed-length record, unless
/// its record also ends exactly where its items do: then the word cannot be
/// chance.
pub fn has_rdw(bytes: &[u8], layout: &Layout) -> bool {
    let word = bytes.len() >= 4
        && bytes.len() <= layout.length + 4
        && usize::from(u16::from_be_bytes([bytes[0], bytes[1]])) == bytes.len()
        && bytes[2] == 0
        && bytes[3] == 0;
    if !word {
        return false;
    }
    if bytes.len() != layout.length {
        return true;
    }
    let mut padded = bytes[4..].to_vec();
    padded.resize(layout.length, 0);
    used_length(&padded, layout) == bytes.len() - 4
}

/// The bytes of a variable-length record after its descriptor word, made
/// the layout's length: a record shorter than the layout ends inside its
/// last table (`OCCURS DEPENDING ON`), which holds fewer items than it has
/// room for, and the room after them is low-values.
fn variable_record(record: &[u8], layout: &Layout) -> Result<Vec<u8>, ReadError> {
    let mut padded = record.to_vec();
    padded.resize(layout.length, 0);
    let used = used_length(&padded, layout);
    if record.len() != used && record.len() != layout.length {
        return Err(ReadError {
            message: tr!(
                "the record descriptor word says the record has {count} bytes, and a record of {record} \
                 with the items its tables say they hold has {used}",
                count = record.len(),
                record = layout.record,
                used = used
            ),
        });
    }
    Ok(padded)
}

/// The bytes a record takes as a variable-length record: up to the end of
/// its last item, a table (`OCCURS DEPENDING ON`) taking only the items its
/// counter says it holds.
pub fn used_length(record: &[u8], layout: &Layout) -> usize {
    fn end(slots: &[Slot], base: usize, record: &[u8]) -> usize {
        slots
            .iter()
            .map(|slot| {
                let start = base + slot.offset;
                match (&slot.occurs, &slot.element) {
                    (Some(o), _) => match &o.counter {
                        Some(counter) => {
                            let field = &record[counter.offset..counter.offset + counter.size];
                            let count = match decode(field, &counter.numeric) {
                                Some(Value::Int(n)) => usize::try_from(n).unwrap_or(0),
                                _ => 0,
                            };
                            start + count.clamp(o.min, o.max) * slot.size
                        }
                        None => start + slot.total(),
                    },
                    (None, Element::Group(children)) => end(children, start, record).max(start),
                    (None, _) => start + slot.size,
                }
            })
            .max()
            .unwrap_or(base)
    }
    end(&layout.slots, 0, record).min(layout.length)
}

/// Writes the record of a tab as a variable-length `.blk` file: a record
/// descriptor word, then the record up to the end of its last item.
pub fn write_variable(node: &Node, layout: &Layout) -> Result<Vec<u8>, WriteError> {
    let record = write(node, layout)?;
    Ok(with_rdw(&record, layout))
}

/// A record's bytes as a variable-length record.
pub fn with_rdw(record: &[u8], layout: &Layout) -> Vec<u8> {
    let used = used_length(record, layout).min(record.len());
    let length = u16::try_from(used + 4).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(used + 4);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&record[..used]);
    out
}

/// A record whose top item has `OCCURS` is its table: the one field of the
/// object read.
fn unwrap_table(node: Node, layout: &Layout) -> Node {
    match (&layout.table, node.value()) {
        (Some(key), Value::Object(fields)) => fields
            .get(key)
            .cloned()
            .unwrap_or_else(|| Node::new(Value::Array(Vec::new()))),
        _ => node,
    }
}

/// Where the node at `path` of a record's data is in its bytes: the first byte
/// and how many. The root is the whole record; a table not indexed is all of
/// its occurrences, whatever its counter says. None if the layout has no such
/// node.
pub fn place(layout: &Layout, path: &Path) -> Option<(usize, usize)> {
    let mut segments: Vec<Segment> = path.segments().to_vec();
    if segments.is_empty() {
        return Some((0, layout.length));
    }
    if let Some(key) = &layout.table {
        segments.insert(0, Segment::Key(key.clone()));
    }
    place_in(&layout.slots, 0, &segments)
}

fn place_in(slots: &[Slot], base: usize, segments: &[Segment]) -> Option<(usize, usize)> {
    let (Segment::Key(wanted), rest) = segments.split_first()? else {
        return None;
    };
    for slot in slots {
        let start = base + slot.offset;
        let Some(key) = &slot.key else {
            // A FILLER group's items are fields of the object it is in.
            if let Element::Group(children) = &slot.element
                && let Some(found) = place_in(children, start, segments)
            {
                return Some(found);
            }
            continue;
        };
        if key != wanted {
            continue;
        }
        let (start, rest) = match (&slot.occurs, rest.split_first()) {
            (None, _) => (start, rest),
            (Some(_), None) => return Some((start, slot.total())),
            (Some(occurs), Some((Segment::Index(i), rest))) if *i < occurs.max => {
                (start + i * slot.size, rest)
            }
            (Some(_), Some(_)) => return None,
        };
        if rest.is_empty() {
            return Some((start, slot.size));
        }
        return match &slot.element {
            Element::Group(children) => place_in(children, start, rest),
            _ => None,
        };
    }
    None
}

/// Reads one record's bytes as a record object.
pub fn read_record(record: &[u8], layout: &Layout, choices: &[usize]) -> Node {
    let mut reader = Reader {
        layout,
        choices,
        warnings: Vec::new(),
    };
    let mut fields = IndexMap::new();
    reader.group(&layout.slots, record, 0, record, &Path::root(), &mut fields);
    unwrap_table(Node::new(Value::Object(fields)), layout)
}

struct Reader<'a> {
    layout: &'a Layout,
    choices: &'a [usize],
    warnings: Vec<Warning>,
}

impl Reader<'_> {
    /// Reads the slots of a group starting at `base` into `fields`.
    fn group(
        &mut self,
        slots: &[Slot],
        bytes: &[u8],
        base: usize,
        record: &[u8],
        path: &Path,
        fields: &mut IndexMap<String, Node>,
    ) {
        for slot in slots {
            if let Some((set, member)) = slot.redefines
                && self.choices.get(set).copied().unwrap_or(0) != member
            {
                continue;
            }
            let start = base + slot.offset;
            let Some(key) = &slot.key else {
                // A FILLER group's items are fields of this object.
                if let Element::Group(children) = &slot.element {
                    self.group(children, bytes, start, record, path, fields);
                }
                continue;
            };
            let here = path.key(key.clone());
            let node = match &slot.occurs {
                None => self.element(slot, bytes, start, record, &here),
                Some(occurs) => {
                    let count = match &occurs.counter {
                        None => occurs.max,
                        Some(counter) => self.count(counter, record, occurs.min, occurs.max, &here),
                    };
                    let items = (0..count)
                        .map(|i| {
                            self.element(slot, bytes, start + i * slot.size, record, &here.index(i))
                        })
                        .collect();
                    Node::new(Value::Array(items))
                }
            };
            fields.insert(key.clone(), node);
        }
    }

    fn element(
        &mut self,
        slot: &Slot,
        bytes: &[u8],
        start: usize,
        record: &[u8],
        path: &Path,
    ) -> Node {
        let field = &bytes[start..start + slot.size];
        match &slot.element {
            Element::Group(children) => {
                let mut fields = IndexMap::new();
                self.group(children, bytes, start, record, path, &mut fields);
                Node::new(Value::Object(fields))
            }
            Element::Text => Node::new(Value::Str(self.layout.code_page.decode(field))),
            Element::Number(n) => Node::new(match decode(field, n) {
                Some(value) => value,
                None => bad_bytes(&slot.name, field),
            }),
            Element::Float => Node::new(match decode_float(field, self.layout.float) {
                Some(value) => value,
                None => bad_bytes(&slot.name, field),
            }),
        }
    }

    /// How many items a table has, by its counter.
    fn count(
        &mut self,
        counter: &Counter,
        record: &[u8],
        min: usize,
        max: usize,
        path: &Path,
    ) -> usize {
        let bytes = &record[counter.offset..counter.offset + counter.size];
        let value = match decode(bytes, &counter.numeric) {
            Some(Value::Int(n)) => n,
            Some(Value::Float(x)) => x as i64,
            _ => -1,
        };
        let count = usize::try_from(value).unwrap_or(0).clamp(min, max);
        if usize::try_from(value).ok() != Some(count) {
            self.warnings.push(Warning {
                path: path.clone(),
                message: tr!("{name} says {value} items, but the table holds {min} to {max}, so {count} were read", name = counter.name, value = value, min = min, max = max, count = count),
            });
        }
        count
    }
}

/// The error value of bytes that are not a number.
fn bad_bytes(name: &str, bytes: &[u8]) -> Value {
    Value::Error(ErrorValue {
        kind: ErrorKind::Value,
        message: tr!(
            "{name} holds bytes that are not a number: X'{bytes}'",
            name = name,
            bytes = hex(bytes)
        ),
        origin: None,
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// The bytes an error value of [`bad_bytes`] keeps.
fn kept_bytes(message: &str) -> Option<Vec<u8>> {
    let hex = message.rsplit_once("X'")?.1.strip_suffix('\'')?;
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// A number from its bytes, or None if they are not one.
pub fn decode(bytes: &[u8], n: &Numeric) -> Option<Value> {
    let (negative, magnitude) = match n.usage {
        NumUsage::Zoned => zoned(bytes, n)?,
        NumUsage::Packed => packed(bytes)?,
        NumUsage::Binary | NumUsage::Native => {
            let mut value: i128 = 0;
            for b in bytes {
                value = (value << 8) | i128::from(*b);
            }
            if n.signed && bytes.first().is_some_and(|b| b & 0x80 != 0) {
                value -= 1_i128 << (8 * bytes.len());
            }
            (value < 0, value.unsigned_abs())
        }
    };
    let value = if negative {
        -(i128::try_from(magnitude).ok()?)
    } else {
        i128::try_from(magnitude).ok()?
    };
    if n.scale == 0 {
        Some(match i64::try_from(value) {
            Ok(v) => Value::Int(v),
            Err(_) => Value::Float(value as f64),
        })
    } else {
        Some(Value::Float(value as f64 / 10_f64.powi(n.scale as i32)))
    }
}

/// A floating point number from its 4 or 8 bytes, or None if they are not
/// a finite number. A `COMP-1` is given with the fewest digits that are
/// still those bytes (`0.1`, not `0.100000001`).
pub fn decode_float(bytes: &[u8], format: FloatFormat) -> Option<Value> {
    let exact = match (format, bytes.len()) {
        (FloatFormat::Ieee, 4) => f64::from(f32::from_be_bytes(bytes.try_into().ok()?)),
        (FloatFormat::Ieee, 8) => f64::from_be_bytes(bytes.try_into().ok()?),
        (FloatFormat::Hex, 4 | 8) => {
            let negative = bytes[0] & 0x80 != 0;
            let exponent = i32::from(bytes[0] & 0x7F) - 64;
            let mut mantissa: u64 = 0;
            for b in &bytes[1..] {
                mantissa = (mantissa << 8) | u64::from(*b);
            }
            let bits = 8 * (bytes.len() as i32 - 1);
            let value = mantissa as f64 * 2_f64.powi(-bits) * 16_f64.powi(exponent);
            if negative { -value } else { value }
        }
        _ => return None,
    };
    if !exact.is_finite() {
        return None;
    }
    if bytes.len() == 4 {
        for digits in 1..=9 {
            let shorter: f64 = format!("{exact:.prec$e}", prec = digits - 1).parse().ok()?;
            let mut again = [0; 4];
            if encode_float(shorter, format, &mut again).is_ok() && again == bytes {
                return Some(Value::Float(shorter));
            }
        }
    }
    Some(Value::Float(exact))
}

/// Writes a floating point number into its 4 or 8 bytes.
pub fn encode_float(value: f64, format: FloatFormat, field: &mut [u8]) -> Result<(), String> {
    if !value.is_finite() {
        return Err(tr!("it is not a finite number").into());
    }
    match (format, field.len()) {
        (FloatFormat::Ieee, 4) => {
            let single = value as f32;
            if !single.is_finite() {
                return Err(tr!("{value} is too large for a COMP-1", value = value));
            }
            field.copy_from_slice(&single.to_be_bytes());
        }
        (FloatFormat::Ieee, 8) => field.copy_from_slice(&value.to_be_bytes()),
        (FloatFormat::Hex, 4 | 8) => {
            field.fill(0);
            if value == 0.0 {
                return Ok(());
            }
            let bits = 8 * (field.len() as i32 - 1);
            let magnitude = value.abs();
            // magnitude = fraction * 16^exponent, fraction from 1/16 to 1.
            let mut exponent = (magnitude.log2() / 4.0).floor() as i32 + 1;
            let mut fraction = magnitude / 16_f64.powi(exponent);
            while fraction >= 1.0 {
                exponent += 1;
                fraction /= 16.0;
            }
            while fraction < 1.0 / 16.0 {
                exponent -= 1;
                fraction *= 16.0;
            }
            let mut mantissa = (fraction * 2_f64.powi(bits)).round() as u64;
            if mantissa >> bits != 0 {
                mantissa >>= 4;
                exponent += 1;
            }
            if exponent + 64 > 127 {
                return Err(tr!(
                    "{value} is too large for its floating point bytes",
                    value = value
                ));
            }
            if exponent + 64 < 0 {
                // Too small to hold: zero.
                return Ok(());
            }
            field[0] =
                u8::try_from(exponent + 64).unwrap_or(0) | if value < 0.0 { 0x80 } else { 0 };
            for (i, b) in field[1..].iter_mut().rev().enumerate() {
                *b = ((mantissa >> (8 * i)) & 0xFF) as u8;
            }
        }
        _ => return Err(tr!("a floating point item has 4 or 8 bytes").into()),
    }
    Ok(())
}

/// A sign nibble: true for minus, None if it is not a sign.
fn sign_nibble(nibble: u8) -> Option<bool> {
    match nibble {
        0xC | 0xA | 0xE | 0xF => Some(false),
        0xD | 0xB => Some(true),
        _ => None,
    }
}

fn zoned(bytes: &[u8], n: &Numeric) -> Option<(bool, u128)> {
    let (digits, negative) = match (n.signed, n.sign) {
        (true, SignPlace::LeadingSeparate) => {
            let (sign, rest) = bytes.split_first()?;
            (rest, separate_sign(*sign)?)
        }
        (true, SignPlace::TrailingSeparate) => {
            let (sign, rest) = bytes.split_last()?;
            (rest, separate_sign(*sign)?)
        }
        (true, SignPlace::LeadingEmbedded) => (bytes, sign_nibble(bytes.first()? >> 4)?),
        _ => (bytes, sign_nibble(bytes.last()? >> 4)?),
    };
    let sign_at = match (n.signed, n.sign) {
        (true, SignPlace::LeadingEmbedded) => Some(0),
        (true, SignPlace::TrailingEmbedded) | (false, _) => Some(digits.len() - 1),
        _ => None,
    };
    let mut value: u128 = 0;
    for (i, b) in digits.iter().enumerate() {
        let (zone, digit) = (b >> 4, b & 0x0F);
        if digit > 9 || (Some(i) != sign_at && zone != 0xF) {
            return None;
        }
        value = value * 10 + u128::from(digit);
    }
    Some((negative && value != 0, value))
}

fn separate_sign(byte: u8) -> Option<bool> {
    match byte {
        0x4E => Some(false),
        0x60 => Some(true),
        _ => None,
    }
}

fn packed(bytes: &[u8]) -> Option<(bool, u128)> {
    let mut value: u128 = 0;
    let last = bytes.len().checked_sub(1)?;
    for (i, b) in bytes.iter().enumerate() {
        let (high, low) = (b >> 4, b & 0x0F);
        if high > 9 {
            return None;
        }
        value = value * 10 + u128::from(high);
        if i == last {
            let negative = sign_nibble(low)?;
            return Some((negative && value != 0, value));
        }
        if low > 9 {
            return None;
        }
        value = value * 10 + u128::from(low);
    }
    None
}

/// Writes the record of a tab as a `.blk` file. A field that is absent is
/// written as spaces or zero; `FILLER` as spaces; the room after the last
/// item of an `OCCURS DEPENDING ON` table as low-values. Refused, naming the
/// field, for text too long or with a character the code page lacks, a
/// number too large for its picture, and an error value.
pub fn write(node: &Node, layout: &Layout) -> Result<Vec<u8>, WriteError> {
    write_record(node, layout, &Path::root())
}

/// Writes one record.
pub fn write_record(record: &Node, layout: &Layout, path: &Path) -> Result<Vec<u8>, WriteError> {
    // A record that is a table is written as the object holding it.
    let wrapped;
    let record = match &layout.table {
        Some(key) => {
            wrapped = Node::new(Value::Object(
                std::iter::once((key.clone(), record.clone())).collect(),
            ));
            &wrapped
        }
        None => record,
    };
    let mut bytes = vec![layout.code_page.space(); layout.length];
    let Value::Object(fields) = record.value() else {
        return Err(WriteError {
            path: path.clone(),
            message: tr!(
                "a {record} record is a table of its fields",
                record = layout.record
            ),
        });
    };
    let writer = Writer { layout };
    writer.group(&layout.slots, fields, &mut bytes, 0, path)?;
    Ok(bytes)
}

struct Writer<'a> {
    layout: &'a Layout,
}

impl Writer<'_> {
    fn group(
        &self,
        slots: &[Slot],
        fields: &IndexMap<String, Node>,
        bytes: &mut [u8],
        base: usize,
        path: &Path,
    ) -> Result<(), WriteError> {
        for slot in slots {
            let start = base + slot.offset;
            if let Some((set, member)) = slot.redefines {
                // The first layout of the group that the record holds is
                // written; the others are passed over.
                let keys = &self.layout.redefines[set].keys;
                let shown = keys
                    .iter()
                    .position(|k| k.iter().any(|k| fields.contains_key(k)))
                    .unwrap_or(0);
                if shown != member {
                    continue;
                }
            }
            let Some(key) = &slot.key else {
                if let Element::Group(children) = &slot.element {
                    self.group(children, fields, bytes, start, path)?;
                }
                continue;
            };
            let here = path.key(key.clone());
            let node = fields.get(key);
            match &slot.occurs {
                None => self.element(slot, node, bytes, start, &here)?,
                Some(occurs) => {
                    let items: &[Node] = match node.map(Node::value) {
                        Some(Value::Array(items)) => items,
                        None => &[],
                        Some(_) => {
                            return Err(WriteError {
                                path: here,
                                message: tr!(
                                    "{name} is a table, a list of items",
                                    name = slot.name
                                ),
                            });
                        }
                    };
                    if items.len() > occurs.max {
                        return Err(WriteError {
                            path: here,
                            message: tr!(
                                "{name} has {items} items, more than the {max} it has room for",
                                name = slot.name,
                                items = items.len(),
                                max = occurs.max
                            ),
                        });
                    }
                    let count = if occurs.counter.is_some() {
                        items.len()
                    } else {
                        occurs.max
                    };
                    for i in 0..count {
                        let at = start + i * slot.size;
                        self.element(slot, items.get(i), bytes, at, &here.index(i))?;
                    }
                    if let Some(counter) = &occurs.counter {
                        bytes[start + count * slot.size..start + slot.total()].fill(0);
                        let field = &mut bytes[counter.offset..counter.offset + counter.size];
                        encode(count as i128, &counter.numeric, field).map_err(|message| {
                            WriteError {
                                path: here.clone(),
                                message: format!("{}: {message}", counter.name),
                            }
                        })?;
                    }
                }
            }
        }
        Ok(())
    }

    fn element(
        &self,
        slot: &Slot,
        node: Option<&Node>,
        bytes: &mut [u8],
        start: usize,
        path: &Path,
    ) -> Result<(), WriteError> {
        let refuse = |message: String| WriteError {
            path: path.clone(),
            message: format!("{}: {message}", slot.name),
        };
        let field = &mut bytes[start..start + slot.size];
        let value = node.map(Node::value);
        if let Some(Value::Error(e)) = value {
            return match kept_bytes(&e.message).filter(|b| b.len() == field.len()) {
                Some(kept) => {
                    field.copy_from_slice(&kept);
                    Ok(())
                }
                None => Err(refuse(tr!(
                    "it holds an error ({kind}), which cannot be written",
                    kind = e.kind.code()
                ))),
            };
        }
        match &slot.element {
            Element::Group(children) => match value {
                Some(Value::Object(fields)) => {
                    self.group(children, fields, bytes, start, path)?;
                }
                None | Some(Value::Null) => {
                    self.group(children, &IndexMap::new(), bytes, start, path)?;
                }
                Some(_) => return Err(refuse(tr!("it is a group, a table of fields").into())),
            },
            Element::Text => {
                let text = match value {
                    None | Some(Value::Null) => String::new(),
                    Some(Value::Str(s)) => s.clone(),
                    Some(Value::Int(n)) => n.to_string(),
                    Some(Value::Float(x)) => x.to_string(),
                    Some(Value::Bool(b)) => b.to_string(),
                    Some(Value::Date(d)) => d.format("%Y-%m-%d").to_string(),
                    Some(_) => return Err(refuse(tr!("it is text").into())),
                };
                let encoded = self.layout.code_page.encode(&text).map_err(|c| {
                    refuse(tr!(
                        "code page {code_page} has no {c}",
                        code_page = self.layout.code_page.code(),
                        c = format!("{c:?}")
                    ))
                })?;
                if encoded.len() > field.len() {
                    return Err(refuse(tr!(
                        "the text has {encoded} characters, more than its {field}",
                        encoded = encoded.len(),
                        field = field.len()
                    )));
                }
                field.fill(self.layout.code_page.space());
                field[..encoded.len()].copy_from_slice(&encoded);
            }
            Element::Number(n) => {
                let unit = 10_f64.powi(n.scale as i32);
                let scaled: i128 = match value {
                    None | Some(Value::Null) => 0,
                    Some(Value::Int(v)) => i128::from(*v) * 10_i128.pow(n.scale),
                    Some(Value::Float(x)) => {
                        let v = (x * unit).round();
                        if !v.is_finite() || v.abs() >= 1e36 {
                            return Err(refuse(tr!("the number is too large").into()));
                        }
                        v as i128
                    }
                    Some(_) => return Err(refuse(tr!("it is a number").into())),
                };
                encode(scaled, n, field).map_err(refuse)?;
            }
            Element::Float => {
                let x = match value {
                    None | Some(Value::Null) => 0.0,
                    Some(Value::Int(v)) => *v as f64,
                    Some(Value::Float(x)) => *x,
                    Some(_) => return Err(refuse(tr!("it is a number").into())),
                };
                encode_float(x, self.layout.float, field).map_err(refuse)?;
            }
        }
        Ok(())
    }
}

/// Writes a number, already scaled to a whole number by its decimals, into
/// the bytes of its field.
fn encode(value: i128, n: &Numeric, field: &mut [u8]) -> Result<(), String> {
    if value < 0 && !n.signed {
        return Err(tr!("it has no sign, so it cannot be negative").into());
    }
    let limit = 10_i128.pow(n.digits);
    if n.usage != NumUsage::Native && value.abs() >= limit {
        return Err(tr!(
            "{value} is too large for its {digits} digits",
            value = value_text(value, n.scale),
            digits = n.digits
        ));
    }
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    match n.usage {
        NumUsage::Zoned => {
            let separate = n.signed
                && matches!(
                    n.sign,
                    SignPlace::LeadingSeparate | SignPlace::TrailingSeparate
                );
            let digits = format!("{magnitude:0width$}", width = n.digits as usize);
            let (sign_byte, digit_bytes) = match (separate, n.sign) {
                (true, SignPlace::LeadingSeparate) => {
                    let (s, d) = field.split_at_mut(1);
                    (Some(&mut s[0]), d)
                }
                (true, _) => {
                    let len = field.len();
                    let (d, s) = field.split_at_mut(len - 1);
                    (Some(&mut s[0]), d)
                }
                _ => (None, field),
            };
            for (b, d) in digit_bytes.iter_mut().zip(digits.bytes()) {
                *b = 0xF0 | (d - b'0');
            }
            match sign_byte {
                Some(s) => *s = if negative { 0x60 } else { 0x4E },
                None if n.signed => {
                    let at = if n.sign == SignPlace::LeadingEmbedded {
                        0
                    } else {
                        digit_bytes.len() - 1
                    };
                    let zone = if negative { 0xD0 } else { 0xC0 };
                    digit_bytes[at] = zone | (digit_bytes[at] & 0x0F);
                }
                None => {}
            }
        }
        NumUsage::Packed => {
            let nibbles = field.len() * 2;
            let digits = format!("{magnitude:0width$}", width = nibbles - 1);
            let mut all: Vec<u8> = digits.bytes().map(|d| d - b'0').collect();
            all.push(match (n.signed, negative) {
                (false, _) => 0xF,
                (true, false) => 0xC,
                (true, true) => 0xD,
            });
            for (i, b) in field.iter_mut().enumerate() {
                *b = (all[2 * i] << 4) | all[2 * i + 1];
            }
        }
        NumUsage::Binary | NumUsage::Native => {
            let bits = 8 * field.len() as u32;
            let (low, high) = if n.signed {
                (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
            } else {
                (0, (1_i128 << bits) - 1)
            };
            if value < low || value > high {
                return Err(tr!(
                    "{value} does not fit in its {count} bytes",
                    value = value_text(value, n.scale),
                    count = field.len()
                ));
            }
            let mut v = value;
            for b in field.iter_mut().rev() {
                *b = (v & 0xFF) as u8;
                v >>= 8;
            }
        }
    }
    Ok(())
}

fn value_text(value: i128, scale: u32) -> String {
    if scale == 0 {
        value.to_string()
    } else {
        (value as f64 / 10_f64.powi(scale as i32)).to_string()
    }
}

#[cfg(test)]
#[path = "records_tests.rs"]
mod records_tests;
