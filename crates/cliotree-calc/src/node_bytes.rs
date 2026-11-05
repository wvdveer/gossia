//! Bytes and nodes (DESIGN.md, "COBOL copybooks", "Raw memory"). `RAW`
//! gives a node's bytes as its tab holds them, and the bytes a node's formula
//! gives are read as that node: the one is the reverse of the other.
//!
//! In a tab of COBOL records, a field's bytes are where its copybook puts
//! them, and the root's are the record. In any other tab, the root is the
//! tab's file; a node that holds other values is written in the tab's format
//! (an element for XML); a single value is its text, as UTF-8.

use cliotree_core::copybook::records;
use cliotree_core::dataformat::DataFormat;
use cliotree_core::xml::{self, XmlDocument};
use cliotree_core::{ErrorKind, ErrorValue, Kind, Node, Path, Segment, TypeSpec, Value};

use crate::ColumnFormula;

/// What a tab is besides its data and its formulas, for its bytes.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct TabInfo {
    pub format: Option<DataFormat>,
    /// What an XML tab says about its document beyond the tree.
    pub xml: Option<XmlDocument>,
    /// For a tab of COBOL records, the layout of each group of `REDEFINES`
    /// it shows.
    pub redefines: Vec<usize>,
    /// A tab of COBOL records whose file is a variable-length record.
    pub variable: bool,
    /// Its column formulas, whose formulas cannot give bytes.
    pub columns: Vec<ColumnFormula>,
}

fn value_error(message: impl Into<String>) -> ErrorValue {
    ErrorValue {
        kind: ErrorKind::Value,
        message: message.into(),
        origin: None,
    }
}

/// The bytes of the node at `path` of a tab whose data is `data`.
pub(crate) fn raw(
    spec: &TypeSpec,
    info: &TabInfo,
    data: &Node,
    path: &Path,
) -> Result<Vec<u8>, ErrorValue> {
    let node = data
        .get(path)
        .ok_or_else(|| value_error("there is no node there"))?;
    if let Value::Error(error) = node.value() {
        return Err(error.clone());
    }
    if let Some(layout) = spec.copybook() {
        let record = records::write(data, layout).map_err(|e| value_error(e.to_string()))?;
        // The root of a variable-length record is its file, after its
        // record descriptor word.
        if path.is_empty() && info.variable {
            return Ok(records::with_rdw(&record, layout));
        }
        let (start, length) = records::place(layout, path)
            .ok_or_else(|| value_error("the copybook has no field there"))?;
        return Ok(record[start..start + length].to_vec());
    }
    let format = info.format.unwrap_or(DataFormat::Json);
    let ty = spec
        .resolve(path)
        .ok_or_else(|| value_error("the schema defines no node there"))?
        .ty;
    let kind = &spec.get(ty).kind;
    let text = if !is_container(kind) {
        match node.value() {
            Value::Str(s) => s.clone(),
            other => xml::scalar_text(other).unwrap_or_default(),
        }
    } else if format == DataFormat::Xml {
        let (sub, document) = if path.is_empty() {
            (spec.clone(), info.xml.clone().unwrap_or_default())
        } else {
            (
                spec.rooted_at(ty, element_name(path)),
                XmlDocument::default(),
            )
        };
        xml::to_string(node, &sub, &document, true).map_err(|e| value_error(e.to_string()))?
    } else {
        piece_format(format, kind, path)
            .write(node, true)
            .map_err(|e| value_error(e.to_string()))?
    };
    Ok(text.into_bytes())
}

/// The value the node at `path` takes from `bytes`, read as that node. What
/// is read is not checked against the node's type here: the formula's result
/// is checked as any other.
pub(crate) fn read(
    spec: &TypeSpec,
    info: &TabInfo,
    data: &Node,
    path: &Path,
    bytes: &[u8],
) -> Result<Value, ErrorValue> {
    if let Some(layout) = spec.copybook() {
        if path.is_empty() {
            // A record, or a variable-length record.
            return records::read(bytes, layout, &info.redefines)
                .map(|parsed| parsed.node.value().clone())
                .map_err(|e| value_error(format!("the formula gave {e}")));
        }
        let (start, length) = records::place(layout, path)
            .ok_or_else(|| value_error("the copybook has no field there"))?;
        if bytes.len() != length {
            return Err(value_error(format!(
                "the formula gave {} bytes, and this field takes {length}",
                bytes.len()
            )));
        }
        // The field's bytes, put in the record as the rest of it is now.
        let mut rest = data.clone();
        let _ = rest.remove(path);
        let mut record = records::write(&rest, layout).map_err(|e| {
            value_error(format!(
                "the record around this field cannot be written: {e}"
            ))
        })?;
        record[start..start + length].copy_from_slice(bytes);
        let node = records::read_record(&record, layout, &info.redefines);
        return node
            .get(path)
            .map(|n| n.value().clone())
            .ok_or_else(|| value_error("the bytes hold no value for this field"));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|e| value_error(format!("the bytes are not UTF-8 text: {e}")))?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let format = info.format.unwrap_or(DataFormat::Json);
    let ty = spec
        .resolve(path)
        .ok_or_else(|| value_error("the schema defines no node there"))?
        .ty;
    let kind = &spec.get(ty).kind;
    if !is_container(kind) {
        return crate::entry::parse_literal(kind, text).map_err(|e| value_error(e.to_string()));
    }
    let node = if format == DataFormat::Xml {
        let sub = if path.is_empty() {
            spec.clone()
        } else {
            spec.rooted_at(ty, element_name(path))
        };
        xml::parse(text, &sub)
            .map_err(|e| value_error(e.to_string()))?
            .node
    } else {
        let format = piece_format(format, kind, path);
        format
            .read(text)
            .map_err(|e| value_error(format!("the bytes are not {}: {e}", format.label())))?
            .node
    };
    Ok(node.value().clone())
}

fn is_container(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Array { .. } | Kind::Object { .. } | Kind::Map { .. }
    )
}

/// The format a node other than the root is written in: one line of JSON
/// Lines is JSON, one of several YAML documents is YAML, and a TOML document
/// is a table, so what is not one is written as JSON.
fn piece_format(format: DataFormat, kind: &Kind, path: &Path) -> DataFormat {
    if path.is_empty() {
        return format;
    }
    match format {
        DataFormat::JsonLines => DataFormat::Json,
        DataFormat::YamlDocuments => DataFormat::Yaml,
        DataFormat::Toml if !matches!(kind, Kind::Object { .. } | Kind::Map { .. }) => {
            DataFormat::Json
        }
        other => other,
    }
}

/// The element a node of an XML tab is: the last field on its path (an item
/// of an array is another of its array's elements).
fn element_name(path: &Path) -> Option<&str> {
    path.segments().iter().rev().find_map(|s| match s {
        Segment::Key(key) => Some(key.as_str()),
        Segment::Index(_) => None,
    })
}
