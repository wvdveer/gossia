//! The Raw view of a Data Tab: the tab's data as the pretty-printed document
//! it is stored as, in the tab's format (JSON, XML, JSON5, ...). A tab of
//! COBOL records shows a hexadecimal dump of its `.blk` bytes with the EBCDIC
//! text beside it, and a copybook schema tab its `.cpy` text. The text can be
//! selected and copied; *Copy* copies COBOL records in the form chosen in
//! Options (a dump, hex digits or Base64), and a paste of any of the three
//! is read back as records. A paste
//! is imported as a dropped file is (`import.rs`); the import dialog can also
//! put a document in place of the tab's data, which is checked against the
//! tab's schema first and refused with the reason if it does not conform:
//! nothing is changed then.

use gossia_calc::SchemaFormat;
use gossia_core::binary;
use gossia_core::copybook;
use gossia_core::dataformat::DataFormat;
use gossia_core::validate::validate;
use gossia_core::{Path, xml};
use gossia_i18n::tr;

use crate::app::GossiaApp;
use crate::view::error_color;

/// How many problems of a refused paste are listed.
const LISTED: usize = 6;

/// How many bytes of COBOL records the dump shows; the rest is counted.
pub const DUMPED: usize = 64 * 1024;

/// What the Raw view keeps between frames.
#[derive(Debug, Default)]
pub struct RawState {
    /// The text shown, and the tab and revision it was made for.
    pub text: String,
    pub key: Option<(String, u64)>,
    /// Why the last paste was refused.
    pub error: Option<String>,
}

impl GossiaApp {
    /// The format the tab is written in.
    fn raw_format(&self) -> DataFormat {
        self.calc().format(&self.sheet).unwrap_or(DataFormat::Json)
    }

    /// What the view's heading calls the document.
    fn raw_label(&self) -> String {
        let calc = self.calc();
        if calc.schema_format(&self.sheet) == Some(SchemaFormat::Copybook) {
            return "COBOL copybook".to_owned();
        }
        match calc.spec(&self.sheet).and_then(|s| s.copybook()) {
            Some(layout) => format!(
                "COBOL record {} ({} bytes{}), code page {}, read-only",
                layout.record,
                layout.length,
                if calc.is_variable(&self.sheet) {
                    ", variable length after a record descriptor word"
                } else {
                    ""
                },
                layout.code_page.label()
            ),
            None => self.raw_format().label().to_owned(),
        }
    }

    /// The text of the tab's document, made again only when the data changed.
    pub fn raw_text(&mut self) -> &str {
        let key = (self.sheet.clone(), self.calc().revision());
        if self.raw.key.as_ref() != Some(&key) {
            let calc = self.calc();
            let copybook_tab = calc.schema_format(&self.sheet) == Some(SchemaFormat::Copybook);
            let text = match (calc.spec(&self.sheet), calc.data(&self.sheet)) {
                (_, Some(data)) if copybook_tab => copybook::from_node(data)
                    .map(|(book, _)| copybook::to_text(&book).trim_end_matches('\n').to_owned()),
                (Some(spec), Some(_)) if spec.copybook().is_some() => {
                    let layout = spec.copybook().expect("checked above");
                    calc.records_bytes(&self.sheet)
                        .map(|bytes| binary::dump(&bytes, layout.length, layout.code_page, DUMPED))
                }
                (Some(spec), Some(data)) => match (spec.xml(), calc.xml_document(&self.sheet)) {
                    (Some(_), Some(document)) => {
                        xml::to_string(data, spec, document, true).map_err(|e| e.to_string())
                    }
                    (Some(_), None) => {
                        xml::to_string(data, spec, &xml::XmlDocument::default(), true)
                            .map_err(|e| e.to_string())
                    }
                    (None, _) => calc
                        .format(&self.sheet)
                        .unwrap_or(DataFormat::Json)
                        .write(data, true)
                        // A file ends with a line break; the view does not.
                        .map(|t| t.trim_end_matches('\n').to_owned())
                        .map_err(|e| e.to_string()),
                },
                _ => Ok(String::new()),
            };
            self.raw.text =
                text.unwrap_or_else(|e| tr!("The document cannot be written: {e}", e = e));
            self.raw.key = Some(key);
        }
        &self.raw.text
    }

    /// A tab of COBOL records as *Copy* copies it: all of its bytes, in the
    /// form chosen in Options. None for any other tab.
    pub fn records_text(&self) -> Option<Result<String, String>> {
        let calc = self.calc();
        let layout = calc.spec(&self.sheet)?.copybook()?;
        Some(calc.records_bytes(&self.sheet).map(|bytes| {
            self.user
                .prefs
                .binary_copy
                .write(&bytes, layout.length, layout.code_page)
        }))
    }

    /// Replaces the records of a tab of COBOL records with `bytes`, read in
    /// its layout: one entry in the history.
    pub fn replace_records(&mut self, sheet: &str, bytes: &[u8]) -> Result<(), String> {
        let parsed = self
            .calc()
            .read_records(sheet, bytes)
            .map_err(|e| tr!("Not pasted: {e}", e = e))?;
        self.session
            .calc_mut()
            .set_value(sheet, &Path::root(), parsed.node.value().clone())
            .map_err(|e| tr!("Not pasted: {e}", e = e))?;
        self.raw.error = None;
        self.notice = None;
        self.session.tick();
        Ok(())
    }

    /// Replaces the document of the tab with `text`, if it is a document that
    /// conforms to the schema. On refusal nothing changes and the reason is
    /// returned, and shown in the view.
    pub fn paste_document(&mut self, text: &str) -> Result<(), String> {
        let result = self.try_paste_document(text);
        self.raw.error = result.as_ref().err().cloned();
        result
    }

    fn try_paste_document(&mut self, text: &str) -> Result<(), String> {
        let sheet = self.sheet.clone();
        if self.calc().schema_format(&sheet) == Some(SchemaFormat::Copybook) {
            return self.paste_copybook(&sheet, text);
        }
        let (node, document) = {
            let Some(spec) = self.calc().spec(&sheet) else {
                return Err(tr!("there is no such tab").to_owned());
            };
            let (node, document) = if spec.xml().is_some() {
                let parsed = xml::parse(text, spec)
                    .map_err(|e| tr!("Not pasted: it is not XML of this schema: {e}", e = e))?;
                (parsed.node, Some(parsed.document))
            } else {
                let format = self.raw_format();
                let parsed = format.read(text).map_err(|e| {
                    tr!(
                        "Not pasted: it is not valid {format}: {e}",
                        format = format.label(),
                        e = e
                    )
                })?;
                (parsed.node, None)
            };
            let issues = validate(spec, &node);
            if !issues.is_empty() {
                let mut message = if issues.len() == 1 {
                    tr!(
                        "Not pasted: the document does not conform to the schema of {sheet} (1 problem):",
                        sheet = sheet
                    )
                } else {
                    tr!(
                        "Not pasted: the document does not conform to the schema of {sheet} ({n} problems):",
                        sheet = sheet,
                        n = issues.len()
                    )
                };
                for issue in issues.iter().take(LISTED) {
                    let at = if issue.path.is_root() {
                        sheet.clone()
                    } else {
                        format!("{sheet}.{}", issue.path)
                    };
                    message.push_str(&format!("\n{at}: {}", issue.message));
                }
                if issues.len() > LISTED {
                    message.push_str(&format!("\nand {} more", issues.len() - LISTED));
                }
                return Err(message);
            }
            (node, document)
        };
        self.session
            .calc_mut()
            .set_value(&sheet, &Path::root(), node.value().clone())
            .map_err(|e| tr!("Not pasted: {e}", e = e))?;
        if let Some(document) = document {
            let _ = self.session.calc_mut().set_xml_document(&sheet, document);
        }
        self.notice = None;
        self.session.tick();
        Ok(())
    }

    /// Puts copybook text in place of a copybook schema tab's, keeping its
    /// code page and floating point format, which the text does not say.
    fn paste_copybook(&mut self, sheet: &str, text: &str) -> Result<(), String> {
        let mut book = copybook::parse(text)
            .map_err(|e| tr!("Not pasted: it is not a copybook: {e}", e = e))?;
        let (old, code_page) = self
            .calc()
            .data(sheet)
            .and_then(|data| copybook::from_node(data).ok())
            .unwrap_or_default();
        book.float = old.float;
        let tree = copybook::to_node(&book, code_page);
        self.session
            .calc_mut()
            .set_value(sheet, &Path::root(), tree.value().clone())
            .map_err(|e| tr!("Not pasted: {e}", e = e))?;
        self.notice = None;
        self.session.tick();
        Ok(())
    }

    /// The Raw view, filling the central panel.
    pub(crate) fn raw_view(&mut self, ui: &mut egui::Ui) {
        // A paste over the view is a paste of the whole document, unless
        // another text box (the import dialog's, say) has the keyboard.
        let document = egui::Id::unique("raw_document");
        let elsewhere = ui
            .ctx()
            .memory(|m| m.focused())
            .is_some_and(|id| id != document);
        let pasted = (!elsewhere)
            .then(|| {
                ui.input_mut(|i| {
                    let mut found = None;
                    i.events.retain(|e| match e {
                        egui::Event::Paste(text) => {
                            found = Some(text.clone());
                            false
                        }
                        _ => true,
                    });
                    found
                })
            })
            .flatten();
        // A paste is imported as a dropped file is: the import dialog opens
        // for it, which can also put it in place of this tab's data.
        if let Some(text) = pasted {
            self.raw.error = self.begin_paste(&text).err();
        }
        let format = self.raw_label();
        let text = self.raw_text().to_owned();
        let records = self.records_text();
        ui.horizontal(|ui| {
            ui.strong(format);
            let hover = match &records {
                Some(_) => tr!(
                    "Copy the records as {kind} (Options, Files and import)",
                    kind = self.user.prefs.binary_copy.label().to_lowercase()
                ),
                None => tr!("Copy the whole document").to_owned(),
            };
            if ui.button(tr!("Copy")).on_hover_text(hover).clicked() {
                match records {
                    Some(Ok(records)) => ui.ctx().copy_text(records),
                    Some(Err(why)) => self.raw.error = Some(why),
                    None => ui.ctx().copy_text(text.clone()),
                }
            }
            ui.weak(
                tr!("Press Ctrl+V to import the clipboard: JSON, XML, YAML, TOML, CSV, TSV or a COBOL copybook."),
            );
        });
        if let Some(error) = &self.raw.error {
            let color = error_color(ui);
            ui.colored_label(color, error);
        }
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // A shared reference is a text buffer that cannot be edited,
                // which can still be selected and copied.
                let mut shown = text.as_str();
                let response = ui.add(
                    egui::TextEdit::multiline(&mut shown)
                        .id(document)
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
                let name = tr!("Raw document");
                response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, name));
            });
    }
}
