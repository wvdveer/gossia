//! The format a data tab is written in (DESIGN.md, "Import and export",
//! "Other file formats"). XML goes with an XSD; the others are JSON-shaped and
//! go with a JSON Schema, and this module reads and writes them.

use crate::json::{self, ParseError, Parsed, WriteError};
use crate::json5::{self, Style};
use crate::path::Path;
use crate::toml_format;
use crate::typespec::Kind;
use crate::value::{Node, Value};
use crate::yaml;
use gossia_i18n::tr;

/// A tab's file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DataFormat {
    Json,
    Xml,
    /// JSON5: JSON with comments, unquoted keys, single quotes and the rest.
    Json5,
    /// JSONC: JSON with comments, as VS Code's settings are.
    Jsonc,
    /// JSON Lines: one JSON value on each line.
    JsonLines,
    /// TOML: a table at the top, with comments.
    Toml,
    /// YAML of one document, with comments.
    Yaml,
    /// YAML of several documents, each after a `---` line: a list.
    YamlDocuments,
    /// Fixed-length COBOL records, `.blk`: goes with a copybook, and is bytes,
    /// not text (see `copybook::records`).
    Blk,
}

impl DataFormat {
    /// Every format, in the order menus list them.
    pub const ALL: [DataFormat; 9] = [
        DataFormat::Json,
        DataFormat::Json5,
        DataFormat::Jsonc,
        DataFormat::JsonLines,
        DataFormat::Toml,
        DataFormat::Yaml,
        DataFormat::YamlDocuments,
        DataFormat::Xml,
        DataFormat::Blk,
    ];

    /// The formats that go with a JSON Schema.
    pub const JSON_SHAPED: [DataFormat; 7] = [
        DataFormat::Json,
        DataFormat::Json5,
        DataFormat::Jsonc,
        DataFormat::JsonLines,
        DataFormat::Toml,
        DataFormat::Yaml,
        DataFormat::YamlDocuments,
    ];

    /// The name the manifest of a `.gossia` file gives it.
    pub fn code(self) -> &'static str {
        match self {
            DataFormat::Json => "json",
            DataFormat::Xml => "xml",
            DataFormat::Json5 => "json5",
            DataFormat::Jsonc => "jsonc",
            DataFormat::JsonLines => "jsonl",
            DataFormat::Toml => "toml",
            DataFormat::Yaml => "yaml",
            DataFormat::YamlDocuments => "yaml-documents",
            DataFormat::Blk => "blk",
        }
    }

    pub fn from_code(code: &str) -> Option<DataFormat> {
        DataFormat::ALL.into_iter().find(|f| f.code() == code)
    }

    /// What the format is called in the window.
    pub fn label(self) -> &'static str {
        match self {
            DataFormat::Json => "JSON",
            DataFormat::Xml => "XML",
            DataFormat::Json5 => "JSON5",
            DataFormat::Jsonc => tr!("JSON with comments"),
            DataFormat::JsonLines => tr!("JSON Lines"),
            DataFormat::Toml => "TOML",
            DataFormat::Yaml => "YAML",
            DataFormat::YamlDocuments => tr!("YAML documents"),
            DataFormat::Blk => tr!("COBOL records"),
        }
    }

    /// The usual extension of a file of this format.
    pub fn extension(self) -> &'static str {
        match self {
            DataFormat::YamlDocuments => "yaml",
            _ => self.code(),
        }
    }

    /// The format of a file with this extension (without its dot, in any
    /// case), if it is one of these.
    pub fn from_extension(extension: &str) -> Option<DataFormat> {
        Some(match extension.to_ascii_lowercase().as_str() {
            "json" => DataFormat::Json,
            "xml" => DataFormat::Xml,
            "json5" => DataFormat::Json5,
            "jsonc" => DataFormat::Jsonc,
            "jsonl" | "ndjson" => DataFormat::JsonLines,
            "toml" => DataFormat::Toml,
            "yaml" | "yml" => DataFormat::Yaml,
            "blk" => DataFormat::Blk,
            _ => return None,
        })
    }

    /// Whether its tabs can have comments, so that they have the comment
    /// commands: in the file, for a format that has comments, and in the
    /// workbook only for JSON and JSON Lines, which have none (owner,
    /// 2026-11-01). Only COBOL records have none.
    pub fn keeps_comments(self) -> bool {
        self != DataFormat::Blk
    }

    /// Whether the format's files can hold comments.
    pub fn has_comments(self) -> bool {
        matches!(
            self,
            DataFormat::Xml
                | DataFormat::Json5
                | DataFormat::Jsonc
                | DataFormat::Toml
                | DataFormat::Yaml
                | DataFormat::YamlDocuments
        )
    }

    /// Whether the format goes with a JSON Schema.
    pub fn json_shaped(self) -> bool {
        !matches!(self, DataFormat::Xml | DataFormat::Blk)
    }

    /// Why data whose root is of this kind cannot be written in the format,
    /// or None.
    pub fn root_problem(self, root: &Kind) -> Option<String> {
        match self {
            DataFormat::JsonLines if !matches!(root, Kind::Array { .. }) => {
                Some("JSON Lines holds a list of values, so the root must be an array".to_owned())
            }
            DataFormat::Toml if !matches!(root, Kind::Object { .. } | Kind::Map { .. }) => {
                Some("TOML holds a table at the top, so the root must be an object".to_owned())
            }
            DataFormat::YamlDocuments if !matches!(root, Kind::Array { .. }) => Some(
                "YAML documents are a list of documents, so the root must be an array".to_owned(),
            ),
            _ => None,
        }
    }

    /// The format a file of this format's extension is read as, given its
    /// text: YAML of several documents is read as YAML documents.
    pub fn refine(self, text: &str) -> DataFormat {
        match self {
            DataFormat::Yaml if yaml::document_count(text) > 1 => DataFormat::YamlDocuments,
            _ => self,
        }
    }

    /// Reads text of a JSON-shaped format.
    pub fn read(self, text: &str) -> Result<Parsed, ParseError> {
        match self {
            DataFormat::Json => json::parse(text),
            DataFormat::Json5 | DataFormat::Jsonc => json5::parse(text),
            DataFormat::JsonLines => read_lines(text),
            DataFormat::Toml => toml_format::parse(text),
            DataFormat::Yaml => yaml::parse(text),
            DataFormat::YamlDocuments => yaml::parse_documents(text),
            DataFormat::Xml => Err(ParseError {
                line: 1,
                column: 1,
                message: "XML is read with its schema".to_owned(),
            }),
            DataFormat::Blk => Err(ParseError {
                line: 1,
                column: 1,
                message: "COBOL records are bytes, read with their copybook".to_owned(),
            }),
        }
    }

    /// Writes a tree in a JSON-shaped format: indented by two spaces if
    /// `pretty`. Comments are written where the format has them. The text
    /// ends with a newline.
    pub fn write(self, node: &Node, pretty: bool) -> Result<String, WriteError> {
        let mut text = match self {
            DataFormat::Json => json::to_string(node, pretty)?,
            DataFormat::Json5 => json5::to_string(node, Style::Json5, pretty)?,
            DataFormat::Jsonc => json5::to_string(node, Style::Jsonc, pretty)?,
            DataFormat::JsonLines => write_lines(node)?,
            DataFormat::Toml => toml_format::to_string(node)?,
            DataFormat::Yaml => yaml::to_string(node)?,
            DataFormat::YamlDocuments => yaml::documents_to_string(node)?,
            DataFormat::Xml => {
                return Err(WriteError {
                    path: Path::root(),
                    message: "XML is written with its schema".to_owned(),
                });
            }
            DataFormat::Blk => {
                return Err(WriteError {
                    path: Path::root(),
                    message: "COBOL records are bytes, written with their copybook".to_owned(),
                });
            }
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }
}

/// JSON Lines: each line that is not blank is a JSON value, an item of the
/// array.
fn read_lines(text: &str) -> Result<Parsed, ParseError> {
    let text = text.trim_start_matches('\u{FEFF}');
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed = json::parse(line).map_err(|e| ParseError {
            line: i + 1,
            column: e.column,
            message: format!("this line is not a JSON value: {}", e.message),
        })?;
        let index = items.len();
        warnings.extend(parsed.warnings.into_iter().map(|mut w| {
            w.path = Path::root().index(index).join(&w.path);
            w
        }));
        items.push(parsed.node);
    }
    Ok(Parsed {
        node: Node::new(Value::Array(items)),
        warnings,
    })
}

fn write_lines(node: &Node) -> Result<String, WriteError> {
    let Value::Array(items) = node.value() else {
        return Err(WriteError {
            path: Path::root(),
            message: "JSON Lines holds a list of values, so the root must be an array".to_owned(),
        });
    };
    let mut text = String::new();
    for (i, item) in items.iter().enumerate() {
        if matches!(item.value(), Value::Error(_)) {
            continue;
        }
        let line = json::to_string(item, false).map_err(|mut e| {
            e.path = Path::root().index(i).join(&e.path);
            e
        })?;
        text.push_str(&line);
        text.push('\n');
    }
    Ok(text)
}

#[cfg(test)]
#[path = "dataformat_tests.rs"]
mod dataformat_tests;
