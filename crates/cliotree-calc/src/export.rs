//! Exporting a tab as a standalone file (DESIGN.md, "Import and export",
//! "Exporting"): the text of the file, in the tab's own format or as lines of
//! text. Writing it to disk is the application's job.

use cliotree_core::{Value, json, xml};

use super::{Calc, SchemaRole};

/// What an exported file is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Xml,
    /// One line for each string of an array of strings, as text import reads
    /// them.
    Text,
}

impl ExportFormat {
    /// The usual extension of a file of this format.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Json => "json",
            ExportFormat::Xml => "xml",
            ExportFormat::Text => "txt",
        }
    }
}

impl Calc {
    /// The format a tab is written in: XML for a tab whose schema is an XSD
    /// (an XSD schema tab included), JSON otherwise.
    pub fn export_format(&self, tab: &str) -> Option<ExportFormat> {
        let sheet = self.sheets.get(tab)?;
        Some(if sheet.spec.xml().is_some() {
            ExportFormat::Xml
        } else {
            ExportFormat::Json
        })
    }

    /// The formats a tab can be exported as: its own, and text if its root is
    /// an array of strings (errored items are not counted, as they are not
    /// written).
    pub fn export_formats(&self, tab: &str) -> Vec<ExportFormat> {
        let Some(own) = self.export_format(tab) else {
            return Vec::new();
        };
        let mut formats = vec![own];
        if own == ExportFormat::Json && self.text_lines(tab).is_ok() {
            formats.push(ExportFormat::Text);
        }
        formats
    }

    /// The schema tab whose file can be written beside the tab's: the one it
    /// is bound to. A schema tab has none, since its schema is built in.
    pub fn export_schema(&self, tab: &str) -> Option<&str> {
        match &self.sheets.get(tab)?.schema {
            SchemaRole::Bound(schema) => Some(schema),
            _ => None,
        }
    }

    /// The text of the tab as a standalone file: what the tab shows, with the
    /// latest computed values and without errored nodes. `pretty` indents by
    /// two spaces; otherwise the file is compact. The text ends with a newline.
    pub fn export_text(
        &self,
        tab: &str,
        format: ExportFormat,
        pretty: bool,
    ) -> Result<String, String> {
        let sheet = self
            .sheets
            .get(tab)
            .ok_or_else(|| format!("there is no tab {tab}"))?;
        let own = self.export_format(tab).expect("the tab is there");
        let mut text = match format {
            ExportFormat::Text => {
                let lines = self.text_lines(tab)?;
                let mut text = String::new();
                for line in lines {
                    text.push_str(line);
                    text.push('\n');
                }
                return Ok(text);
            }
            _ if format != own => {
                return Err(format!(
                    "tab {tab} is {} data and cannot be written as {}",
                    name(own),
                    name(format)
                ));
            }
            ExportFormat::Xml => {
                let document = sheet.xml.clone().unwrap_or_default();
                xml::to_string(&sheet.data, &sheet.spec, &document, pretty)
            }
            ExportFormat::Json => json::to_string(&sheet.data, pretty),
        }
        .map_err(|e| format!("tab {tab}: {e}"))?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    /// The strings of a tab whose root is an array of strings, without its
    /// errored items, or why it is not one. A string with a line break in it
    /// is refused, since it would come back as two lines.
    fn text_lines(&self, tab: &str) -> Result<Vec<&str>, String> {
        let sheet = self
            .sheets
            .get(tab)
            .ok_or_else(|| format!("there is no tab {tab}"))?;
        let not_text = || format!("tab {tab} is not an array of strings");
        if sheet.spec.xml().is_some() {
            return Err(not_text());
        }
        let items = sheet.data.value().as_array().ok_or_else(not_text)?;
        let mut lines = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            match item.value() {
                Value::Error(_) => {}
                Value::Str(s) if s.contains(['\n', '\r']) => {
                    return Err(format!(
                        "item {index} of tab {tab} has a line break, so the tab cannot be \
                         written as lines of text"
                    ));
                }
                Value::Str(s) => lines.push(s.as_str()),
                _ => return Err(not_text()),
            }
        }
        Ok(lines)
    }
}

fn name(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Json => "JSON",
        ExportFormat::Xml => "XML",
        ExportFormat::Text => "text",
    }
}
