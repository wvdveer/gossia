//! Lowering a copybook (DESIGN.md, "COBOL copybooks", "Lowering"): its
//! first 01 record becomes the type of an item of a data tab, which is an
//! array of records, and the [`Layout`] says where each field's bytes are.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::{CodePage, Copybook, FloatFormat, Item, Renames, Usage, from_node};
use crate::num::Num;
use crate::typespec::{Field, Kind, Range, StringConstraints, TypeDef, TypeId, TypeSpec};
use crate::value::{Node, Value};

/// Where the fields of a record are in its bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub code_page: CodePage,
    /// How `COMP-1` and `COMP-2` items hold their numbers.
    pub float: FloatFormat,
    /// The COBOL name of the record (its 01 item).
    pub record: String,
    /// The bytes of each record.
    pub length: usize,
    pub slots: Vec<Slot>,
    /// Each group of items over the same bytes: the redefined one first.
    pub redefines: Vec<RedefineSet>,
    /// For a record whose top item has `OCCURS`, the key of its one slot:
    /// the tab's data is that table, not an object holding it.
    pub table: Option<String>,
}

/// An item placed in a record.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    /// The field's key: the COBOL name with `-` as `_`. None for `FILLER`,
    /// whose bytes are passed over; a `FILLER` group's items are fields of
    /// the object the group is in.
    pub key: Option<String>,
    pub name: String,
    /// From the start of the group it is in (for an item of a table, of one
    /// occurrence of it).
    pub offset: usize,
    /// The bytes of one occurrence.
    pub size: usize,
    pub occurs: Option<SlotOccurs>,
    /// The group of layouts it is one of, and which one: see [`Layout::redefines`].
    pub redefines: Option<(usize, usize)>,
    pub element: Element,
    /// Its condition names (level 88), each with the literals it names.
    pub conditions: Vec<Condition>,
}

/// A condition name (level 88) of an item.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub name: String,
    /// The literals, as written: `'A'`, `1`, `SPACES`.
    pub values: Vec<String>,
    /// The ranges (`1 THRU 9`), each end as written.
    pub ranges: Vec<(String, String)>,
}

impl Slot {
    /// The bytes it takes, every occurrence.
    pub fn total(&self) -> usize {
        self.size * self.occurs.as_ref().map_or(1, |o| o.max)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlotOccurs {
    pub min: usize,
    pub max: usize,
    /// The field that holds how many items there are (`DEPENDING ON`).
    pub counter: Option<Counter>,
}

/// The field an `OCCURS DEPENDING ON` table counts its items by: where it is
/// in the record.
#[derive(Debug, Clone, PartialEq)]
pub struct Counter {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub numeric: Numeric,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    Group(Vec<Slot>),
    /// Characters: `PIC X`, `PIC A`, and numbers with editing (`PIC Z,ZZ9.99`).
    Text,
    Number(Numeric),
    /// Floating point: `COMP-1` (4 bytes) or `COMP-2` (8 bytes).
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Numeric {
    pub usage: NumUsage,
    pub digits: u32,
    /// The digits after the implied decimal point (`V`).
    pub scale: u32,
    pub signed: bool,
    pub sign: SignPlace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumUsage {
    /// `DISPLAY`: zoned decimal, a digit a byte.
    Zoned,
    /// `COMP-3`, `PACKED-DECIMAL`.
    Packed,
    /// `COMP`, `COMP-4`, `BINARY`: held to the digits of the picture.
    Binary,
    /// `COMP-5`: the whole range of the bytes.
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignPlace {
    TrailingEmbedded,
    LeadingEmbedded,
    LeadingSeparate,
    TrailingSeparate,
}

/// Items over the same bytes (`REDEFINES`, and `RENAMES`), of which a tab
/// shows one.
#[derive(Debug, Clone, PartialEq)]
pub struct RedefineSet {
    /// The COBOL names, the redefined item first. For a `RENAMES`, the
    /// first is the run of items renamed (`A THRU B`), the second its name.
    pub names: Vec<String>,
    /// The keys of the fields each shows: none for a `FILLER`, one for an
    /// item, the keys of the run of items for the first of a `RENAMES`.
    pub keys: Vec<Vec<String>>,
}

/// What a picture says.
struct Picture {
    numeric: bool,
    digits: u32,
    scale: u32,
    signed: bool,
    /// Its characters, without `S` and `V`.
    length: usize,
}

fn picture(text: &str) -> Result<Picture, String> {
    let mut expanded = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if chars.peek() == Some(&'(') {
            chars.next();
            let count: String = chars.by_ref().take_while(|c| *c != ')').collect();
            let count: usize = count
                .trim()
                .parse()
                .map_err(|_| format!("PIC {text} has a count that is not a number"))?;
            if count > 100_000 {
                return Err(format!("PIC {text} is too long"));
            }
            expanded.extend(std::iter::repeat_n(c, count));
        } else {
            expanded.push(c);
        }
    }
    let upper = expanded.to_ascii_uppercase();
    if upper.contains('P') {
        return Err(format!(
            "PIC {text}: scaling positions (P) are not supported"
        ));
    }
    let signed = upper.starts_with('S');
    let mut digits = 0;
    let mut scale = 0;
    let mut after_point = false;
    let mut numeric = true;
    let mut length = 0;
    for (i, c) in upper.chars().enumerate() {
        match c {
            'S' if i == 0 => {}
            'V' => after_point = true,
            '9' => {
                digits += 1;
                length += 1;
                if after_point {
                    scale += 1;
                }
            }
            _ => {
                numeric = false;
                length += 1;
            }
        }
    }
    if numeric && digits == 0 {
        return Err(format!("PIC {text} has no digits or characters"));
    }
    Ok(Picture {
        numeric,
        digits,
        scale,
        signed,
        length,
    })
}

/// The key of a COBOL name.
fn key_of(name: &str) -> Option<String> {
    (!name.eq_ignore_ascii_case("FILLER")).then(|| name.replace('-', "_"))
}

/// A numeric field others may count by, by its name.
#[derive(Clone)]
struct Named {
    offset: usize,
    size: usize,
    numeric: Option<Numeric>,
    in_table: bool,
}

struct Builder {
    redefines: Vec<RedefineSet>,
    /// Every item by its upper-case name; None once a name is used twice.
    names: HashMap<String, Option<Named>>,
}

impl Builder {
    #[allow(clippy::too_many_arguments)]
    fn group(
        &mut self,
        items: &[Item],
        base: usize,
        in_table: bool,
        usage: Option<Usage>,
        sign: Option<super::Sign>,
        sync: bool,
        path: &str,
    ) -> Result<(Vec<Slot>, usize), String> {
        let mut slots: Vec<Slot> = Vec::new();
        let mut cursor = 0;
        for item in items {
            let at = format!("{path}.{}", item.name);
            let usage = item.usage.or(usage);
            let sync = item.sync || sync;
            // A SYNC item starts at a multiple of its alignment from the
            // start of the record, after slack bytes.
            let align = alignment(item, usage, sync);
            let (offset, redefines) = match &item.redefines {
                None => ((base + cursor).next_multiple_of(align) - base, None),
                Some(target) => {
                    let Some(t) = slots
                        .iter()
                        .position(|s| s.name.eq_ignore_ascii_case(target))
                    else {
                        return Err(format!(
                            "{} redefines {target}, which is not an item before it at its level",
                            item.name
                        ));
                    };
                    let set = match slots[t].redefines {
                        Some((set, _)) => set,
                        None => {
                            self.redefines.push(RedefineSet {
                                names: vec![slots[t].name.clone()],
                                keys: vec![slots[t].key.clone().into_iter().collect()],
                            });
                            let set = self.redefines.len() - 1;
                            slots[t].redefines = Some((set, 0));
                            set
                        }
                    };
                    let members = &mut self.redefines[set];
                    members.names.push(item.name.clone());
                    members.keys.push(key_of(&item.name).into_iter().collect());
                    (slots[t].offset, Some((set, members.names.len() - 1)))
                }
            };
            let sign = item.sign.or(sign);
            let table = item.occurs.is_some();
            let (element, size) = if item.items.is_empty() {
                match (usage, &item.picture) {
                    (Some(Usage::Comp1 | Usage::Comp2), Some(_)) => {
                        return Err(format!(
                            "{}: a floating point item (COMP-1, COMP-2) has no PIC",
                            item.name
                        ));
                    }
                    (Some(Usage::Comp1), None) => (Element::Float, 4),
                    (Some(Usage::Comp2), None) => (Element::Float, 8),
                    (_, None) => {
                        return Err(format!("{} has no PIC and no items below it", item.name));
                    }
                    (_, Some(text)) => {
                        elementary(text, usage, sign).map_err(|e| format!("{}: {e}", item.name))?
                    }
                }
            } else {
                if item.picture.is_some() {
                    return Err(format!(
                        "{} is a group, with items below it, so it has no PIC",
                        item.name
                    ));
                }
                let (slots, size) = self.group(
                    &item.items,
                    base + offset,
                    in_table || table,
                    usage,
                    sign,
                    sync,
                    &at,
                )?;
                // Each occurrence of a table starts aligned as the first.
                let size = if table {
                    size.next_multiple_of(align)
                } else {
                    size
                };
                (Element::Group(slots), size)
            };
            let occurs = match &item.occurs {
                None => None,
                Some(o) => {
                    if o.max == 0 || o.min.is_some_and(|m| m > o.max) {
                        return Err(format!("{}: OCCURS has no room for any items", item.name));
                    }
                    let max = o.max as usize;
                    let min = o
                        .min
                        .map_or(if o.depending_on.is_some() { 0 } else { max }, |m| {
                            m as usize
                        });
                    Some(SlotOccurs {
                        min,
                        max,
                        counter: o.depending_on.as_ref().map(|name| Counter {
                            // Found once the whole record is placed.
                            name: name.clone(),
                            offset: 0,
                            size: 0,
                            numeric: ZERO,
                        }),
                    })
                }
            };
            let named = Named {
                offset: base + offset,
                size,
                numeric: match &element {
                    Element::Number(n) => Some(*n),
                    _ => None,
                },
                in_table: in_table || table,
            };
            if key_of(&item.name).is_some() {
                self.names
                    .entry(item.name.to_ascii_uppercase())
                    .and_modify(|e| *e = None)
                    .or_insert(Some(named));
            }
            let slot = Slot {
                key: key_of(&item.name),
                name: item.name.clone(),
                offset,
                size,
                occurs,
                redefines,
                element,
                conditions: item.conditions.iter().map(condition).collect(),
            };
            cursor = cursor.max(offset + slot.total());
            slots.push(slot);
        }
        check_keys(&slots, path)?;
        Ok((slots, cursor))
    }

    /// Finds the counter of each `OCCURS DEPENDING ON`.
    fn counters(&self, slots: &mut [Slot]) -> Result<(), String> {
        for slot in slots {
            if let Some(counter) = slot.occurs.as_mut().and_then(|o| o.counter.as_mut()) {
                let found = self
                    .names
                    .get(&counter.name.to_ascii_uppercase())
                    .ok_or_else(|| {
                        format!(
                            "{} depends on {}, which is not in the record",
                            slot.name, counter.name
                        )
                    })?
                    .clone()
                    .ok_or_else(|| {
                        format!(
                            "{} depends on {}, a name used twice in the record",
                            slot.name, counter.name
                        )
                    })?;
                let numeric = found.numeric.ok_or_else(|| {
                    format!(
                        "{} depends on {}, which is not a number",
                        slot.name, counter.name
                    )
                })?;
                if found.in_table {
                    return Err(format!(
                        "{} depends on {}, which is inside a table: a counter must be in the same place in every record",
                        slot.name, counter.name
                    ));
                }
                counter.offset = found.offset;
                counter.size = found.size;
                counter.numeric = numeric;
            }
            if let Element::Group(children) = &mut slot.element {
                self.counters(children)?;
            }
        }
        Ok(())
    }
}

/// The boundary a SYNC item starts on: binary items on a halfword (2) or
/// fullword (4), `COMP-1` on a fullword and `COMP-2` on a doubleword (8), a
/// group on the largest of its items'. 1 for the rest.
fn alignment(item: &Item, usage: Option<Usage>, sync: bool) -> usize {
    if !item.items.is_empty() {
        return item
            .items
            .iter()
            .map(|child| alignment(child, child.usage.or(usage), sync || child.sync))
            .max()
            .unwrap_or(1);
    }
    if !sync {
        return 1;
    }
    match usage {
        Some(Usage::Comp1) => 4,
        Some(Usage::Comp2) => 8,
        Some(Usage::Comp | Usage::Comp4 | Usage::Comp5 | Usage::Binary) => {
            match item.picture.as_deref().map(picture) {
                Some(Ok(p)) if p.numeric && p.digits <= 4 => 2,
                Some(Ok(p)) if p.numeric => 4,
                _ => 1,
            }
        }
        _ => 1,
    }
}

/// Puts each RENAMES entry of a record in its slots: a layout over the
/// bytes of the items it renames, which with them is a group of layouts as
/// a REDEFINES is (owner, 2026-11-01).
fn apply_renames(
    builder: &mut Builder,
    slots: &mut Vec<Slot>,
    renames: &[Renames],
    record: &str,
) -> Result<(), String> {
    for r in renames {
        let error = |why: &str| format!("{} renames {}: {why}", r.name, run_name(r));
        let list = siblings_of(slots, &r.from, false)
            .map_err(|e| error(&e))?
            .ok_or_else(|| error(&format!("{} is not an item of {record}", r.from)))?;
        let first = list
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(&r.from))
            .expect("found above");
        let last = match &r.thru {
            None => first,
            Some(thru) => list
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(thru))
                .ok_or_else(|| {
                    error(&format!(
                        "{thru} is not an item beside {} at its level",
                        r.from
                    ))
                })?,
        };
        if last < first {
            return Err(error("the last item comes before the first"));
        }
        let run = &list[first..=last];
        if run.iter().any(|s| s.occurs.is_some()) {
            return Err(error("an item renamed has OCCURS"));
        }
        if run.iter().any(|s| s.redefines.is_some()) {
            return Err(error(
                "an item renamed is in a REDEFINES, or renamed already",
            ));
        }
        let start = run[0].offset;
        let end = run
            .iter()
            .map(|s| s.offset + s.total())
            .max()
            .unwrap_or(start);
        let element = match run {
            [one] if !matches!(one.element, Element::Group(_)) => one.element.clone(),
            _ => Element::Text,
        };
        builder.redefines.push(RedefineSet {
            names: vec![run_name(r), r.name.clone()],
            keys: vec![
                run.iter().filter_map(|s| s.key.clone()).collect(),
                key_of(&r.name).into_iter().collect(),
            ],
        });
        let set = builder.redefines.len() - 1;
        for slot in &mut list[first..=last] {
            slot.redefines = Some((set, 0));
        }
        list.insert(
            last + 1,
            Slot {
                key: key_of(&r.name),
                name: r.name.clone(),
                offset: start,
                size: end - start,
                occurs: None,
                redefines: Some((set, 1)),
                element,
                conditions: Vec::new(),
            },
        );
        check_keys(list, record)?;
    }
    Ok(())
}

/// How a RENAMES names its run of items: `A`, or `A THRU B`.
fn run_name(r: &Renames) -> String {
    match &r.thru {
        Some(thru) => format!("{} THRU {thru}", r.from),
        None => r.from.clone(),
    }
}

/// The list of slots that has the item `name` in it, if any. Refused for an
/// item inside a table.
fn siblings_of<'a>(
    slots: &'a mut Vec<Slot>,
    name: &str,
    in_table: bool,
) -> Result<Option<&'a mut Vec<Slot>>, String> {
    if slots.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
        if in_table {
            return Err(format!("{name} is inside a table"));
        }
        return Ok(Some(slots));
    }
    for slot in slots.iter_mut() {
        let table = in_table || slot.occurs.is_some();
        if let Element::Group(children) = &mut slot.element
            && let Some(found) = siblings_of(children, name, table)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

const ZERO: Numeric = Numeric {
    usage: NumUsage::Zoned,
    digits: 1,
    scale: 0,
    signed: false,
    sign: SignPlace::TrailingEmbedded,
};

/// The keys of an object's fields, with those of `FILLER` groups in it,
/// must differ.
fn check_keys(slots: &[Slot], path: &str) -> Result<(), String> {
    fn keys<'a>(slots: &'a [Slot], out: &mut Vec<&'a str>) {
        for slot in slots {
            match (&slot.key, &slot.element) {
                (Some(key), _) => out.push(key),
                (None, Element::Group(children)) => keys(children, out),
                _ => {}
            }
        }
    }
    let mut all = Vec::new();
    keys(slots, &mut all);
    let mut seen = std::collections::HashSet::new();
    for key in all {
        if !seen.insert(key.to_ascii_uppercase()) {
            return Err(format!(
                "{path} has two items named {key} (a - in a name counts as _)"
            ));
        }
    }
    Ok(())
}

fn elementary(
    text: &str,
    usage: Option<Usage>,
    sign: Option<super::Sign>,
) -> Result<(Element, usize), String> {
    let picture = picture(text)?;
    let usage = usage.unwrap_or(Usage::Display);
    if !picture.numeric {
        if usage != Usage::Display {
            return Err(format!(
                "PIC {text} is characters, so it cannot be {}",
                usage.code()
            ));
        }
        return Ok((Element::Text, picture.length));
    }
    if picture.digits > 18 {
        return Err(format!(
            "PIC {text} has more than 18 digits, which ClioTree cannot hold"
        ));
    }
    let sign_place = match sign {
        Some(s) if s.leading && s.separate => SignPlace::LeadingSeparate,
        Some(s) if s.separate => SignPlace::TrailingSeparate,
        Some(s) if s.leading => SignPlace::LeadingEmbedded,
        _ => SignPlace::TrailingEmbedded,
    };
    let (num_usage, size) = match usage {
        Usage::Display => {
            let separate = picture.signed
                && matches!(
                    sign_place,
                    SignPlace::LeadingSeparate | SignPlace::TrailingSeparate
                );
            (
                NumUsage::Zoned,
                picture.digits as usize + usize::from(separate),
            )
        }
        Usage::Comp3 | Usage::PackedDecimal => (NumUsage::Packed, picture.digits as usize / 2 + 1),
        Usage::Comp | Usage::Comp4 | Usage::Binary | Usage::Comp5 => {
            let size = match picture.digits {
                1..=4 => 2,
                5..=9 => 4,
                _ => 8,
            };
            let usage = if usage == Usage::Comp5 {
                NumUsage::Native
            } else {
                NumUsage::Binary
            };
            (usage, size)
        }
        Usage::Comp1 | Usage::Comp2 => {
            return Err(format!(
                "a floating point item ({}) has no PIC",
                usage.code()
            ));
        }
    };
    Ok((
        Element::Number(Numeric {
            usage: num_usage,
            digits: picture.digits,
            scale: picture.scale,
            signed: picture.signed,
            sign: sign_place,
        }),
        size,
    ))
}

/// Lowers a copybook: the type of a data tab bound to it (its first record,
/// one instance) and the layout of the record.
pub fn lower(copybook: &Copybook, code_page: CodePage) -> Result<TypeSpec, String> {
    lower_record(copybook, code_page, None)
}

/// The names of a copybook's records, in order: its items at the top, which
/// are at level 01 unless the copybook is copied in under one (IBM MQ's
/// start at level 10).
pub fn record_names(copybook: &Copybook) -> Vec<String> {
    copybook.items.iter().map(|i| i.name.clone()).collect()
}

/// Lowers a copybook for its 01 record called `record`, or its first when
/// that is None or the copybook has no record of that name.
pub fn lower_record(
    copybook: &Copybook,
    code_page: CodePage,
    record: Option<&str>,
) -> Result<TypeSpec, String> {
    let ones = || copybook.items.iter();
    let record = record
        .and_then(|name| ones().find(|i| i.name.eq_ignore_ascii_case(name)))
        .or_else(|| ones().next())
        .ok_or("the copybook has no record")?;
    if record.redefines.is_some() {
        return Err(format!(
            "{} is a record, which has no REDEFINES",
            record.name
        ));
    }
    let mut builder = Builder {
        redefines: Vec::new(),
        names: HashMap::new(),
    };
    // A record that is one elementary item is a record of that one field,
    // and one whose top item has OCCURS a record of that one table.
    let (mut slots, length) = if record.items.is_empty() || record.occurs.is_some() {
        let mut alone = record.clone();
        alone.level = 2;
        builder.group(&[alone], 0, false, None, None, false, &record.name)?
    } else {
        builder.group(
            &record.items,
            0,
            false,
            record.usage,
            record.sign,
            record.sync,
            &record.name,
        )?
    };
    builder.counters(&mut slots)?;
    apply_renames(&mut builder, &mut slots, &record.renames, &record.name)?;
    if length == 0 {
        return Err(format!("{} takes no bytes", record.name));
    }
    let table = record
        .occurs
        .is_some()
        .then(|| slots.first().and_then(|s| s.key.clone()))
        .flatten();
    let layout = Layout {
        code_page,
        float: copybook.float,
        record: record.name.clone(),
        length,
        slots,
        redefines: builder.redefines,
        table,
    };
    // A tab of COBOL records holds one record (owner, 2026-11-01); a record
    // whose top item has OCCURS is that table.
    let mut types = Vec::new();
    let object = object_type(&layout.slots, &record.name, code_page, &mut types);
    let root = match (&layout.table, &types[object.0 as usize].kind) {
        (Some(_), Kind::Object { fields }) if fields.len() == 1 => fields[0].ty,
        _ => object,
    };
    Ok(TypeSpec::new(types, root).with_copybook(layout))
}

/// Lowers the tree of a copybook schema tab.
pub fn lower_node(node: &Node) -> Result<TypeSpec, String> {
    lower_node_record(node, None)
}

/// Lowers the tree of a copybook schema tab for one of its 01 records.
pub fn lower_node_record(node: &Node, record: Option<&str>) -> Result<TypeSpec, String> {
    let (copybook, code_page) = from_node(node)?;
    lower_record(&copybook, code_page, record)
}

fn push(types: &mut Vec<TypeDef>, kind: Kind, source: String) -> TypeId {
    types.push(TypeDef {
        kind,
        source,
        name: None,
        title: None,
        description: None,
        choices: Vec::new(),
    });
    TypeId(u32::try_from(types.len() - 1).unwrap_or(u32::MAX))
}

fn object_type(
    slots: &[Slot],
    path: &str,
    code_page: CodePage,
    types: &mut Vec<TypeDef>,
) -> TypeId {
    let mut fields: IndexMap<String, Field> = IndexMap::new();
    add_fields(slots, path, code_page, types, &mut fields);
    push(
        types,
        Kind::Object {
            fields: fields.into_values().collect(),
        },
        path.to_owned(),
    )
}

fn add_fields(
    slots: &[Slot],
    path: &str,
    code_page: CodePage,
    types: &mut Vec<TypeDef>,
    fields: &mut IndexMap<String, Field>,
) {
    for slot in slots {
        let at = format!("{path}.{}", slot.name);
        let Some(key) = &slot.key else {
            if let Element::Group(children) = &slot.element {
                add_fields(children, path, code_page, types, fields);
            }
            continue;
        };
        // A field of two layouts (a RENAMES of one item) is one field.
        if fields.contains_key(key) {
            continue;
        }
        let mut ty = match &slot.element {
            Element::Group(children) => object_type(children, &at, code_page, types),
            Element::Text => push(
                types,
                Kind::String(StringConstraints {
                    max_length: Some(slot.size),
                    ..StringConstraints::default()
                }),
                at.clone(),
            ),
            Element::Number(n) => push(types, number_kind(n, slot.size), at.clone()),
            Element::Float => push(
                types,
                Kind::Number(Range {
                    min: None,
                    max: None,
                }),
                at.clone(),
            ),
        };
        // Condition names are offered as choices (owner, 2026-11-01): a
        // field may still hold any other value, as in COBOL.
        types[ty.0 as usize].choices = slot
            .conditions
            .iter()
            .flat_map(|c| {
                c.values
                    .iter()
                    .filter_map(|literal| condition_value(literal, slot))
                    .chain(
                        c.ranges
                            .iter()
                            .flat_map(|(low, high)| range_values(low, high, slot, code_page)),
                    )
                    .map(|value| (value, c.name.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        if let Some(o) = &slot.occurs {
            ty = push(
                types,
                Kind::Array {
                    items: ty,
                    min_items: Some(o.min),
                    max_items: Some(o.max),
                },
                at.clone(),
            );
        }
        fields.insert(
            key.clone(),
            Field {
                name: key.clone(),
                ty,
                required: slot.redefines.is_none(),
            },
        );
    }
}

/// The literals and ranges a condition name names; `WHEN SET TO FALSE` is
/// left out.
fn condition(c: &super::ConditionName) -> Condition {
    let mut values: Vec<String> = Vec::new();
    let mut ranges = Vec::new();
    let words = super::literal_words(&c.values);
    let mut rest = words.iter();
    while let Some(word) = rest.next() {
        match word.to_ascii_uppercase().as_str() {
            "THRU" | "THROUGH" => {
                if let (Some(low), Some(high)) = (values.pop(), rest.next()) {
                    ranges.push((low, high.clone()));
                }
            }
            "WHEN" | "FALSE" => break,
            _ => values.push(word.clone()),
        }
    }
    Condition {
        name: c.name.clone(),
        values,
        ranges,
    }
}

/// Most values a range of a condition name is offered as choices with.
const RANGE_CHOICES: usize = 100;

/// The values of a range of a condition name, each one: whole numbers, or
/// single characters in the order of their bytes in the code page (as
/// COBOL compares them), printable ones only. None for a range of more than
/// [`RANGE_CHOICES`] values, or one that is neither.
fn range_values(low: &str, high: &str, slot: &Slot, code_page: CodePage) -> Vec<Value> {
    let (Some(Value::Int(a)), Some(Value::Int(b))) = (
        matches!(slot.element, Element::Number(_))
            .then(|| condition_value(low, slot))
            .flatten(),
        matches!(slot.element, Element::Number(_))
            .then(|| condition_value(high, slot))
            .flatten(),
    ) else {
        let one = |literal: &str| {
            let inner = literal
                .strip_prefix(['\'', '"'])
                .and_then(|l| l.strip_suffix(['\'', '"']))?;
            let mut chars = inner.chars();
            let c = chars.next()?;
            chars.next().is_none().then_some(c)
        };
        if !matches!(slot.element, Element::Text) {
            return Vec::new();
        }
        let (Some(a), Some(b)) = (
            one(low).and_then(|c| code_page.encode_char(c)),
            one(high).and_then(|c| code_page.encode_char(c)),
        ) else {
            return Vec::new();
        };
        let values: Vec<Value> = (a..=b)
            .map(|byte| code_page.decode_byte(byte))
            .filter(|c| !c.is_control() && *c != '\u{FFFD}')
            .map(|c| Value::Str(format!("{c:<width$}", width = slot.size)))
            .collect();
        return if values.len() <= RANGE_CHOICES {
            values
        } else {
            Vec::new()
        };
    };
    if b < a || (b - a) as u64 >= RANGE_CHOICES as u64 {
        return Vec::new();
    }
    (a..=b).map(Value::Int).collect()
}

/// The value a literal of a condition name stands for in a field, as the
/// field is read: text padded with spaces to its length, a number as its
/// picture gives it. None for what the field cannot hold.
fn condition_value(literal: &str, slot: &Slot) -> Option<Value> {
    let figurative = literal.to_ascii_uppercase();
    let quoted = ['\'', '"']
        .into_iter()
        .find(|q| literal.len() >= 2 && literal.starts_with(*q) && literal.ends_with(*q))
        .map(|q| {
            let doubled: String = [q, q].iter().collect();
            literal[1..literal.len() - 1].replace(&doubled, &q.to_string())
        });
    match &slot.element {
        Element::Text => {
            let text = match (quoted, figurative.as_str()) {
                (Some(text), _) => text,
                (None, "SPACE" | "SPACES") => String::new(),
                (None, "ZERO" | "ZEROS" | "ZEROES") => "0".repeat(slot.size),
                (None, _) if literal.chars().all(|c| c.is_ascii_digit()) => literal.to_owned(),
                _ => return None,
            };
            let length = text.chars().count();
            (length <= slot.size).then(|| Value::Str(format!("{text:<width$}", width = slot.size)))
        }
        Element::Number(n) => {
            let text = match figurative.as_str() {
                "ZERO" | "ZEROS" | "ZEROES" => "0",
                _ if quoted.is_some() => return None,
                _ => literal,
            };
            if n.scale == 0 {
                text.trim_start_matches('+')
                    .parse::<i64>()
                    .ok()
                    .map(Value::Int)
            } else {
                text.parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite())
                    .map(Value::Float)
            }
        }
        Element::Float => {
            let text = match figurative.as_str() {
                "ZERO" | "ZEROS" | "ZEROES" => "0",
                _ if quoted.is_some() => return None,
                _ => literal,
            };
            text.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .map(Value::Float)
        }
        Element::Group(_) => None,
    }
}

fn number_kind(n: &Numeric, size: usize) -> Kind {
    let max: i64 = if n.usage == NumUsage::Native {
        match (size, n.signed) {
            (2, true) => i64::from(i16::MAX),
            (2, false) => i64::from(u16::MAX),
            (4, true) => i64::from(i32::MAX),
            (4, false) => i64::from(u32::MAX),
            _ => i64::MAX,
        }
    } else {
        10_i64.pow(n.digits) - 1
    };
    let min: i64 = match (n.signed, n.usage == NumUsage::Native, size) {
        (false, _, _) => 0,
        (true, true, 2) => i64::from(i16::MIN),
        (true, true, 4) => i64::from(i32::MIN),
        (true, true, _) => i64::MIN,
        (true, false, _) => -max,
    };
    if n.scale == 0 {
        Kind::Integer(Range {
            min: Some(Num::Int(min)),
            max: Some(Num::Int(max)),
        })
    } else {
        let unit = 10_f64.powi(n.scale as i32);
        Kind::Number(Range {
            min: Some(Num::Float(min as f64 / unit)),
            max: Some(Num::Float(max as f64 / unit)),
        })
    }
}
