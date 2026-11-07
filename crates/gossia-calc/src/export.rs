//! Exporting a tab as a standalone file (DESIGN.md, "Import and export",
//! "Exporting"): the text of the file, in the tab's own format or as lines of
//! text. Writing it to disk is the application's job.

use gossia_core::calendar::EntryKind;
use gossia_core::dataformat::DataFormat;
use gossia_core::{Value, entries_write, tabular, xml};
use gossia_i18n::tr;

use super::{Calc, SchemaRole};

/// What an exported file is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// A file format a tab can be written in.
    Data(DataFormat),
    /// One line for each string of an array of strings, as text import reads
    /// them.
    Text,
    /// A copybook schema tab as `.cpy` text.
    Copybook,
    /// An array of objects as rows of comma-separated values, nested objects
    /// as dotted columns (DESIGN.md, "Excel and ODS files, 0.4.0").
    Csv,
    /// The same, separated by tabs.
    Tsv,
    /// The same as an Excel workbook of one worksheet.
    Xlsx,
    /// The same as an OpenDocument spreadsheet of one sheet.
    Ods,
    /// A tab of calendar events or tasks as an iCalendar file (DESIGN.md,
    /// "Calendars and contacts, 0.4.0").
    Calendar,
    /// A tab of contacts as vCard.
    Contacts,
}

impl ExportFormat {
    pub const JSON: ExportFormat = ExportFormat::Data(DataFormat::Json);
    pub const XML: ExportFormat = ExportFormat::Data(DataFormat::Xml);

    /// The usual extension of a file of this format.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Data(format) => format.extension(),
            ExportFormat::Text => "txt",
            ExportFormat::Copybook => "cpy",
            ExportFormat::Csv => "csv",
            ExportFormat::Tsv => "tsv",
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Ods => "ods",
            ExportFormat::Calendar => "ics",
            ExportFormat::Contacts => "vcf",
        }
    }

    /// What the export dialog calls it.
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Data(format) => format.label(),
            ExportFormat::Text => tr!("Text, one line for each string"),
            ExportFormat::Copybook => tr!("COBOL copybook"),
            ExportFormat::Csv => tr!("CSV, a row for each item"),
            ExportFormat::Tsv => tr!("TSV, a row for each item"),
            ExportFormat::Xlsx => tr!("Excel workbook, a row for each item"),
            ExportFormat::Ods => tr!("ODS spreadsheet, a row for each item"),
            ExportFormat::Calendar => tr!("iCalendar file"),
            ExportFormat::Contacts => tr!("vCard contacts"),
        }
    }

    /// Whether it writes calendar entries or contacts, in their own format.
    pub fn is_entries(self) -> bool {
        matches!(self, ExportFormat::Calendar | ExportFormat::Contacts)
    }

    /// Whether it writes rows of an array of objects.
    pub fn is_rows(self) -> bool {
        matches!(
            self,
            ExportFormat::Csv | ExportFormat::Tsv | ExportFormat::Xlsx | ExportFormat::Ods
        )
    }
}

impl Calc {
    /// The format a tab is written in: its own.
    pub fn export_format(&self, tab: &str) -> Option<ExportFormat> {
        self.export_formats(tab).into_iter().next()
    }

    /// The formats a tab can be exported as: its own first, the others it
    /// could be written in (DESIGN.md, "Other file formats"), and text if its
    /// root is an array of strings (errored items are not counted, as they
    /// are not written). A schema tab is written only in its own.
    pub fn export_formats(&self, tab: &str) -> Vec<ExportFormat> {
        let Some(own) = self.format(tab) else {
            return Vec::new();
        };
        if self.schema_format(tab) == Some(super::SchemaFormat::Copybook) {
            return vec![ExportFormat::Copybook];
        }
        let mut formats: Vec<ExportFormat> = if self.is_schema_sheet(tab) {
            vec![ExportFormat::Data(own)]
        } else {
            self.formats_for(tab)
                .into_iter()
                .map(ExportFormat::Data)
                .collect()
        };
        if own.json_shaped() && self.text_lines(tab).is_ok() {
            formats.push(ExportFormat::Text);
        }
        if !self.is_schema_sheet(tab) && !self.export_row_arrays(tab).is_empty() {
            formats.extend([
                ExportFormat::Csv,
                ExportFormat::Tsv,
                ExportFormat::Xlsx,
                ExportFormat::Ods,
            ]);
        }
        match self.entry_kind(tab) {
            Some(EntryKind::Contacts) => formats.push(ExportFormat::Contacts),
            Some(_) => formats.push(ExportFormat::Calendar),
            None => {}
        }
        formats
    }

    /// The arrays of objects of a data tab that can be written as rows: the
    /// root, if it is one, and those reached through the fields of objects.
    pub fn export_row_arrays(&self, tab: &str) -> Vec<gossia_core::Path> {
        match self.sheets.get(tab) {
            Some(sheet) if !self.is_schema_sheet(tab) => tabular::row_arrays(&sheet.spec),
            _ => Vec::new(),
        }
    }

    /// The columns the array of objects at `at` would be written with, and
    /// the fields left out because they are lists or maps.
    pub fn export_row_columns(
        &self,
        tab: &str,
        at: &gossia_core::Path,
    ) -> Option<(Vec<String>, Vec<String>)> {
        tabular::columns_of(&self.sheets.get(tab)?.spec, at)
    }

    /// The array of objects at `at` in a tab as rows of CSV, TSV or an Excel
    /// workbook, with the fields left out because they are lists or maps.
    /// The worksheet is named after the tab, or after the array if it is not
    /// the tab's root.
    pub fn export_rows(
        &self,
        tab: &str,
        at: &gossia_core::Path,
        format: ExportFormat,
    ) -> Result<(Vec<u8>, Vec<String>), String> {
        let sheet = self
            .sheets
            .get(tab)
            .ok_or_else(|| tr!("there is no tab {tab}", tab = tab))?;
        if !format.is_rows() {
            return Err(tr!("{format} does not hold rows", format = format.label()));
        }
        let table = tabular::table(&sheet.spec, &sheet.data, at).ok_or_else(|| {
            tr!(
                "{at} of tab {tab} is not a list of records, so it cannot be written as rows",
                at = at,
                tab = tab
            )
        })?;
        let bytes = match format {
            ExportFormat::Csv => tabular::to_delimited(&table, ',').into_bytes(),
            ExportFormat::Tsv => tabular::to_delimited(&table, '\t').into_bytes(),
            _ => {
                let name = if at.is_root() {
                    tab.to_owned()
                } else {
                    at.to_string()
                };
                if format == ExportFormat::Ods {
                    gossia_core::ods::write(&table, &name)?
                } else {
                    tabular::to_xlsx(&table, &name)?
                }
            }
        };
        Ok((bytes, table.left_out))
    }

    /// The first array of objects of a tab as rows: what is written when no
    /// array is chosen.
    fn export_first_rows(&self, tab: &str, format: ExportFormat) -> Result<Vec<u8>, String> {
        let at = self
            .export_row_arrays(tab)
            .into_iter()
            .next()
            .ok_or_else(|| tr!("tab {tab} has no list of records", tab = tab))?;
        self.export_rows(tab, &at, format).map(|(bytes, _)| bytes)
    }

    /// The schema tab whose file can be written beside the tab's: the one it
    /// is bound to. A schema tab has none, since its schema is built in.
    pub fn export_schema(&self, tab: &str) -> Option<&str> {
        match &self.sheets.get(tab)?.schema {
            SchemaRole::Bound(schema) => Some(schema),
            _ => None,
        }
    }

    /// The bytes of the tab as a standalone file: its records for COBOL
    /// records, and otherwise its text (see [`Calc::export_text`]).
    pub fn export_bytes(
        &self,
        tab: &str,
        format: ExportFormat,
        pretty: bool,
    ) -> Result<Vec<u8>, String> {
        if format == ExportFormat::Data(DataFormat::Blk) {
            if !self.export_formats(tab).contains(&format) {
                return Err(tr!("tab {tab} does not hold COBOL records", tab = tab));
            }
            return self.records_bytes(tab);
        }
        if matches!(format, ExportFormat::Xlsx | ExportFormat::Ods) {
            return self.export_first_rows(tab, format);
        }
        self.export_text(tab, format, pretty)
            .map(String::into_bytes)
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
            .ok_or_else(|| tr!("there is no tab {tab}", tab = tab))?;
        // Text says itself why a tab cannot be lines (a line break, say).
        if format != ExportFormat::Text && !self.export_formats(tab).contains(&format) {
            return Err(tr!(
                "tab {tab} is {format} data and cannot be written as {format2}",
                tab = tab,
                format = sheet.format.label(),
                format2 = format.label()
            ));
        }
        let mut text = match format {
            ExportFormat::Csv | ExportFormat::Tsv => {
                let bytes = self.export_first_rows(tab, format)?;
                return Ok(String::from_utf8(bytes).expect("CSV is written as UTF-8"));
            }
            ExportFormat::Xlsx | ExportFormat::Ods => {
                return Err(tr!(
                    "tab {tab} as a spreadsheet file is bytes, not text",
                    tab = tab
                ));
            }
            ExportFormat::Calendar | ExportFormat::Contacts => {
                return self.entries_text(tab);
            }
            ExportFormat::Text => {
                let lines = self.text_lines(tab)?;
                let mut text = String::new();
                for line in lines {
                    text.push_str(line);
                    text.push('\n');
                }
                return Ok(text);
            }
            ExportFormat::Copybook => {
                let (copybook, _) = gossia_core::copybook::from_node(&sheet.data)
                    .map_err(|e| format!("tab {tab}: {e}"))?;
                return Ok(gossia_core::copybook::to_text(&copybook));
            }
            ExportFormat::Data(DataFormat::Blk) => {
                return Err(tr!(
                    "tab {tab} holds COBOL records, which are bytes, not text",
                    tab = tab
                ));
            }
            ExportFormat::Data(DataFormat::Xml) => {
                let document = sheet.xml.clone().unwrap_or_default();
                xml::to_string(&sheet.data, &sheet.spec, &document, pretty)
            }
            ExportFormat::Data(format) => format.write(&sheet.data, pretty),
        }
        .map_err(|e| format!("tab {tab}: {e}"))?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    /// A tab of calendar entries as iCalendar, or of contacts as vCard.
    fn entries_text(&self, tab: &str) -> Result<String, String> {
        let kind = self
            .entry_kind(tab)
            .ok_or_else(|| tr!("tab {tab} holds no calendar entries or contacts", tab = tab))?;
        let data = self
            .data(tab)
            .ok_or_else(|| tr!("there is no tab {tab}", tab = tab))?;
        let text =
            gossia_core::json::to_string(data, false).map_err(|e| format!("tab {tab}: {e}"))?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("tab {tab}: {e}"))?;
        let entries = value.as_array().cloned().unwrap_or_default();
        Ok(match kind {
            EntryKind::Contacts => entries_write::write_contacts(&entries),
            _ => entries_write::write_calendar_now(kind, &entries),
        })
    }

    /// The strings of a tab whose root is an array of strings, without its
    /// errored items, or why it is not one. A string with a line break in it
    /// is refused, since it would come back as two lines.
    fn text_lines(&self, tab: &str) -> Result<Vec<&str>, String> {
        let sheet = self
            .sheets
            .get(tab)
            .ok_or_else(|| tr!("there is no tab {tab}", tab = tab))?;
        let not_text = || tr!("tab {tab} is not an array of strings", tab = tab);
        if sheet.spec.xml().is_some() {
            return Err(not_text());
        }
        let items = sheet.data.value().as_array().ok_or_else(not_text)?;
        let mut lines = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            match item.value() {
                Value::Error(_) => {}
                Value::Str(s) if s.contains(['\n', '\r']) => {
                    return Err(tr!(
                        "item {index} of tab {tab} has a line break, so the tab cannot be \
                         written as lines of text",
                        index = index,
                        tab = tab
                    ));
                }
                Value::Str(s) => lines.push(s.as_str()),
                _ => return Err(not_text()),
            }
        }
        Ok(lines)
    }
}
