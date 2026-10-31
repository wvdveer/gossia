//! `.blk` files: one record with no record descriptor word, laid out by a
//! copybook (DESIGN.md, "COBOL copybooks"). Text is EBCDIC in the
//! copybook's code page; numbers are zoned, packed or binary.
//!
//! Bytes that are not a valid number are read as an error value whose
//! message ends with the bytes (`X'4040'`), and such a value is written back
//! as those bytes, so a record goes in and comes out unchanged.

use indexmap::IndexMap;

use super::lower::{Counter, Element, Layout, NumUsage, Numeric, SignPlace, Slot};
use crate::json::{Parsed, Warning, WriteError};
use crate::path::Path;
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
/// one not given is the first. A file that is not exactly one record long
/// is refused.
pub fn read(bytes: &[u8], layout: &Layout, choices: &[usize]) -> Result<Parsed, ReadError> {
    if bytes.len() != layout.length {
        return Err(ReadError {
            message: format!(
                "the file has {} bytes, and a record of {} has {}",
                bytes.len(),
                layout.record,
                layout.length
            ),
        });
    }
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
                message: format!(
                    "{} says {value} items, but the table holds {min} to {max}, so {count} were read",
                    counter.name
                ),
            });
        }
        count
    }
}

/// The error value of bytes that are not a number.
fn bad_bytes(name: &str, bytes: &[u8]) -> Value {
    Value::Error(ErrorValue {
        kind: ErrorKind::Value,
        message: format!(
            "{name} holds bytes that are not a number: X'{}'",
            hex(bytes)
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
            message: format!("a {} record is a table of its fields", layout.record),
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
                    .position(|k| k.as_ref().is_some_and(|k| fields.contains_key(k)))
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
                                message: format!("{} is a table, a list of items", slot.name),
                            });
                        }
                    };
                    if items.len() > occurs.max {
                        return Err(WriteError {
                            path: here,
                            message: format!(
                                "{} has {} items, more than the {} it has room for",
                                slot.name,
                                items.len(),
                                occurs.max
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
                None => Err(refuse(format!(
                    "it holds an error ({}), which cannot be written",
                    e.kind.code()
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
                Some(_) => return Err(refuse("it is a group, a table of fields".into())),
            },
            Element::Text => {
                let text = match value {
                    None | Some(Value::Null) => String::new(),
                    Some(Value::Str(s)) => s.clone(),
                    Some(Value::Int(n)) => n.to_string(),
                    Some(Value::Float(x)) => x.to_string(),
                    Some(Value::Bool(b)) => b.to_string(),
                    Some(Value::Date(d)) => d.format("%Y-%m-%d").to_string(),
                    Some(_) => return Err(refuse("it is text".into())),
                };
                let encoded = self.layout.code_page.encode(&text).map_err(|c| {
                    refuse(format!(
                        "code page {} has no {c:?}",
                        self.layout.code_page.code()
                    ))
                })?;
                if encoded.len() > field.len() {
                    return Err(refuse(format!(
                        "the text has {} characters, more than its {}",
                        encoded.len(),
                        field.len()
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
                            return Err(refuse("the number is too large".into()));
                        }
                        v as i128
                    }
                    Some(_) => return Err(refuse("it is a number".into())),
                };
                encode(scaled, n, field).map_err(refuse)?;
            }
        }
        Ok(())
    }
}

/// Writes a number, already scaled to a whole number by its decimals, into
/// the bytes of its field.
fn encode(value: i128, n: &Numeric, field: &mut [u8]) -> Result<(), String> {
    if value < 0 && !n.signed {
        return Err("it has no sign, so it cannot be negative".into());
    }
    let limit = 10_i128.pow(n.digits);
    if n.usage != NumUsage::Native && value.abs() >= limit {
        return Err(format!(
            "{} is too large for its {} digits",
            value_text(value, n.scale),
            n.digits
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
                return Err(format!(
                    "{} does not fit in its {} bytes",
                    value_text(value, n.scale),
                    field.len()
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
