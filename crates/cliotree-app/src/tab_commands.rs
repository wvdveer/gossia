//! Adding, deleting and rebinding tabs from the window (DESIGN.md, "Managing
//! tabs"): the dialogs of *New tab...*, *Delete tab...* and *Change
//! schema...*, in the Edit menu and the context menu of a sheet tab.

use cliotree_calc::{Mentions, SchemaFormat};
use cliotree_core::dataformat::DataFormat;
use cliotree_core::validate::validate;
use cliotree_core::xml::XmlDocument;
use cliotree_core::{Node, json, metaschema, xml, xsd};

use crate::app::ClioApp;
use crate::import::unique;
use crate::view::{error_color, named};

/// What a new tab is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewSchema {
    /// A schema tab of its own, empty, with auto-update on, so that its
    /// fields are added from the data tab (as File > New does): a JSON
    /// Schema, or an XSD, which makes the tab an XML tab.
    Own(SchemaFormat),
    /// A schema tab that is there already.
    Existing(String),
}

/// The dialog of *New tab...*.
#[derive(Debug, Clone)]
pub struct NewTab {
    pub name: String,
    pub schema_name: String,
    pub schema: NewSchema,
    pub error: Option<String>,
    focused: bool,
}

/// The dialog of *Delete tab...*.
#[derive(Debug, Clone)]
pub struct DeleteTab {
    pub name: String,
    pub error: Option<String>,
}

/// The dialog of *Change schema...*.
#[derive(Debug, Clone)]
pub struct BindTab {
    pub sheet: String,
    pub schema: String,
    pub error: Option<String>,
    /// How many problems the data has under `schema`, made when it was chosen.
    problems: Option<(String, usize)>,
}

/// The tab dialogs that are open.
#[derive(Debug, Clone, Default)]
pub struct TabDialogs {
    pub new: Option<NewTab>,
    pub delete: Option<DeleteTab>,
    pub bind: Option<BindTab>,
    /// A change of format waiting for the answer to whether its comments may
    /// go.
    pub format: Option<FormatChange>,
}

/// *Format* on a tab's menu, waiting for the answer to whether the comments
/// the new format cannot hold may be taken off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatChange {
    pub sheet: String,
    pub format: DataFormat,
    /// How many comments would go.
    pub lost: usize,
}

impl TabDialogs {
    /// Closes what was open for a tab that is gone.
    pub fn forget(&mut self, name: &str) {
        if self.format.as_ref().is_some_and(|f| f.sheet == name) {
            self.format = None;
        }
        if self.delete.as_ref().is_some_and(|d| d.name == name) {
            self.delete = None;
        }
        if self
            .bind
            .as_ref()
            .is_some_and(|d| d.sheet == name || d.schema == name)
        {
            self.bind = None;
        }
        if let Some(NewTab {
            schema: NewSchema::Existing(schema),
            ..
        }) = &self.new
            && schema == name
        {
            self.new = None;
        }
    }
}

/// The schema of a tab made with its own schema, before any field is added.
const EMPTY_SCHEMA: &str = r#"{"type": "object", "properties": {}}"#;

/// A blank XSD: its root element, named `root`, with no attributes or
/// elements yet.
pub fn blank_xsd(root: &str) -> String {
    format!(
        "<xs:schema xmlns:xs=\"{}\">\n  <xs:element name=\"{root}\">\n    \
         <xs:complexType>\n      <xs:sequence/>\n    </xs:complexType>\n  \
         </xs:element>\n</xs:schema>\n",
        xsd::XSD_NS
    )
}

impl ClioApp {
    fn can_change_tabs(&mut self) -> bool {
        self.session.running_procedure().is_none() && self.commit_editing()
    }

    // ---------------------------------------------------------- new tab

    pub fn begin_new_tab(&mut self) {
        if !self.can_change_tabs() {
            return;
        }
        let calc = self.calc();
        let taken = |n: &str| calc.tab_name_problem(n, None).is_some();
        let name = unique("Data", taken);
        let schema_name = unique(&format!("{name}Schema"), |n| {
            taken(n) || n.eq_ignore_ascii_case(&name)
        });
        self.tab_dialogs.new = Some(NewTab {
            name,
            schema_name,
            schema: NewSchema::Own(SchemaFormat::Json),
            error: None,
            focused: false,
        });
    }

    /// Why the new tab cannot be made as the dialog stands, or None.
    pub fn new_tab_problem(&self, dialog: &NewTab) -> Option<String> {
        let name = dialog.name.trim();
        if let Some(why) = self.calc().tab_name_problem(name, None) {
            return Some(format!("The tab name: {why}."));
        }
        match &dialog.schema {
            NewSchema::Own(_) => {
                let schema = dialog.schema_name.trim();
                if let Some(why) = self.calc().tab_name_problem(schema, None) {
                    return Some(format!("The schema tab's name: {why}."));
                }
                if schema.eq_ignore_ascii_case(name) {
                    return Some("The two tabs need different names.".to_owned());
                }
            }
            NewSchema::Existing(schema) => {
                if let Err(e) = self.calc().schema_spec(schema) {
                    return Some(e.to_string());
                }
                if self.calc().auto_update(schema) && !self.calc().schema_users(schema).is_empty() {
                    return Some(format!(
                        "{schema} has auto-update switched on, so only one tab may use it."
                    ));
                }
            }
        }
        None
    }

    /// Makes the tab of the dialog, at the end of the tabs, and shows it. Its
    /// data is what a new item of the schema's type would hold.
    pub fn create_tab(&mut self, dialog: &NewTab) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        if let Some(why) = self.new_tab_problem(dialog) {
            return Err(why);
        }
        let name = dialog.name.trim();
        let calc = self.session.calc_mut();
        let done = match &dialog.schema {
            NewSchema::Own(format) => {
                let schema_name = dialog.schema_name.trim();
                let empty = Node::new(cliotree_core::Value::Object(Default::default()));
                calc.begin_entry(&format!("New tab {name}"));
                let done = match format {
                    SchemaFormat::Json => {
                        let schema = json::parse(EMPTY_SCHEMA).expect("the schema is JSON").node;
                        calc.create_json_schema_tab(schema_name, schema)
                            .and_then(|()| calc.create_data_tab(name, schema_name, empty, None))
                    }
                    // The root element is named after the tab.
                    SchemaFormat::Xsd => {
                        let schema = xml::parse(&blank_xsd(name), metaschema::xsd_meta())
                            .expect("the blank XSD is read");
                        let document = XmlDocument {
                            declaration: Some(cliotree_core::xmltree::Declaration::default()),
                            ..XmlDocument::default()
                        };
                        calc.create_xsd_schema_tab(schema_name, schema.node, schema.document)
                            .and_then(|()| {
                                calc.create_data_tab(name, schema_name, empty, Some(document))
                            })
                    }
                }
                .and_then(|()| calc.set_auto_update(schema_name, true));
                calc.end_entry();
                done
            }
            NewSchema::Existing(schema) => calc.schema_spec(schema).and_then(|spec| {
                let data = Node::new(ClioApp::default_value(&spec, spec.root()));
                let xml = spec.xml().map(|_| XmlDocument {
                    declaration: Some(cliotree_core::xmltree::Declaration::default()),
                    ..XmlDocument::default()
                });
                calc.create_data_tab(name, schema, data, xml)
            }),
        };
        done.map_err(|e| e.to_string())?;
        self.follow_tabs();
        self.show_tab(name);
        self.session.tick();
        Ok(())
    }

    fn new_tab_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.tab_dialogs.new.take() else {
            return;
        };
        let problem = self.new_tab_problem(&dialog);
        let schemas: Vec<String> = self
            .calc()
            .sheet_names()
            .filter(|s| self.calc().is_schema_sheet(s))
            .map(str::to_owned)
            .collect();
        let (mut apply, mut cancel, mut open) = (false, false, true);
        egui::Window::new("New tab")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Tab name");
                    let response = ui.text_edit_singleline(&mut dialog.name);
                    named(&response, "New tab name");
                    if !dialog.focused {
                        response.request_focus();
                        crate::tab_rename::select_all(ui, &response, &dialog.name);
                        dialog.focused = true;
                    }
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        apply = true;
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let own = matches!(dialog.schema, NewSchema::Own(_));
                    if ui.radio(own, "Its own schema, called").clicked() && !own {
                        dialog.schema = NewSchema::Own(SchemaFormat::Json);
                    }
                    let response = ui.add_enabled(
                        own,
                        egui::TextEdit::singleline(&mut dialog.schema_name).desired_width(160.0),
                    );
                    named(&response, "New schema tab name");
                });
                // The language of its own schema: an XSD makes an XML tab.
                ui.horizontal(|ui| {
                    let own = match dialog.schema {
                        NewSchema::Own(format) => Some(format),
                        NewSchema::Existing(_) => None,
                    };
                    ui.add_space(24.0);
                    ui.add_enabled_ui(own.is_some(), |ui| {
                        for (format, text) in [
                            (SchemaFormat::Json, "JSON Schema (JSON data)"),
                            (SchemaFormat::Xsd, "XSD (XML data)"),
                        ] {
                            if ui.radio(own == Some(format), text).clicked() {
                                dialog.schema = NewSchema::Own(format);
                            }
                        }
                    });
                });
                ui.add_enabled_ui(!schemas.is_empty(), |ui| {
                    ui.horizontal(|ui| {
                        let existing = matches!(dialog.schema, NewSchema::Existing(_));
                        if ui.radio(existing, "The schema of").clicked()
                            && let Some(first) = schemas.first()
                        {
                            dialog.schema = NewSchema::Existing(first.clone());
                        }
                        let current = match &dialog.schema {
                            NewSchema::Existing(s) => s.clone(),
                            NewSchema::Own(_) => String::new(),
                        };
                        egui::ComboBox::from_id_salt("new_tab_schema")
                            .selected_text(current.clone())
                            .show_ui(ui, |ui| {
                                for schema in &schemas {
                                    if ui.selectable_label(&current == schema, schema).clicked() {
                                        dialog.schema = NewSchema::Existing(schema.clone());
                                    }
                                }
                            })
                            .response
                            .widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::ComboBox,
                                    true,
                                    "Schema of the new tab",
                                )
                            });
                    });
                });
                if let Some(why) = problem.as_ref().or(dialog.error.as_ref()) {
                    ui.colored_label(error_color(ui), why);
                }
                ui.horizontal(|ui| {
                    apply |= ui
                        .add_enabled(problem.is_none(), egui::Button::new("Create"))
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if apply && problem.is_none() {
            if let Err(why) = self.create_tab(&dialog) {
                dialog.error = Some(why);
                self.tab_dialogs.new = Some(dialog);
            }
        } else if open && !cancel {
            self.tab_dialogs.new = Some(dialog);
        }
    }

    /// Moves a tab to place `to` in the order, as dragging it along the bar
    /// does. Typing in progress is entered first.
    pub fn move_tab(&mut self, name: &str, to: usize) {
        if !self.commit_editing() {
            return;
        }
        if let Err(e) = self.session.calc_mut().move_sheet(name, to) {
            self.notice = Some(e.to_string());
        }
    }

    // ------------------------------------------------------- delete tab

    pub fn begin_delete_tab(&mut self, name: &str) {
        if !self.can_change_tabs() {
            return;
        }
        self.tab_dialogs.delete = Some(DeleteTab {
            name: name.to_owned(),
            error: None,
        });
    }

    /// Deletes a tab and shows the one after it (or before it, for the last).
    pub fn delete_tab(&mut self, name: &str) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        let names: Vec<String> = self.calc().sheet_names().map(str::to_owned).collect();
        let at = names.iter().position(|n| n == name);
        self.session
            .calc_mut()
            .delete_sheet(name)
            .map_err(|e| e.to_string())?;
        let next = at.and_then(|i| {
            names
                .get(i + 1)
                .or_else(|| i.checked_sub(1).and_then(|j| names.get(j)))
        });
        if self.sheet == name
            && let Some(next) = next.cloned()
        {
            self.show_tab(&next);
        }
        self.follow_tabs();
        self.session.tick();
        Ok(())
    }

    fn delete_tab_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.tab_dialogs.delete.take() else {
            return;
        };
        let problem = self.calc().delete_problem(&dialog.name);
        let mentions = self.calc().mentions(&dialog.name);
        let (mut apply, mut cancel, mut open) = (false, false, true);
        egui::Window::new(format!("Delete tab {}", dialog.name))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                match &problem {
                    Some(why) => {
                        ui.colored_label(error_color(ui), format!("It cannot be deleted: {why}."));
                    }
                    None => {
                        ui.label(format!(
                            "Delete {} with its data, formulas and forms? Undo puts it back.",
                            dialog.name
                        ));
                        if let Some(warning) = delete_mentions_text(&dialog.name, mentions) {
                            ui.label(warning);
                        }
                    }
                }
                if let Some(why) = &dialog.error {
                    ui.colored_label(error_color(ui), why);
                }
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(problem.is_none(), egui::Button::new("Delete"))
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if apply && problem.is_none() {
            let name = dialog.name.clone();
            if let Err(why) = self.delete_tab(&name) {
                dialog.error = Some(why);
                self.tab_dialogs.delete = Some(dialog);
            }
        } else if open && !cancel {
            self.tab_dialogs.delete = Some(dialog);
        }
    }

    // ---------------------------------------------------- change schema

    /// Opens *Change schema...* for a data tab. A schema tab has none to
    /// change.
    pub fn begin_bind_tab(&mut self, sheet: &str) {
        if self.calc().is_schema_sheet(sheet) || !self.can_change_tabs() {
            return;
        }
        let schema = self.calc().schema_of(sheet).unwrap_or_default().to_owned();
        self.tab_dialogs.bind = Some(BindTab {
            sheet: sheet.to_owned(),
            schema,
            error: None,
            problems: None,
        });
    }

    /// Binds a tab to another schema tab, as the dialog does.
    pub fn bind_tab(&mut self, sheet: &str, schema: &str) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .rebind_sheet(sheet, schema)
            .map_err(|e| e.to_string())?;
        // What was made for the old type is made again.
        self.rows_key = None;
        self.summary.key = None;
        self.raw.key = None;
        self.tables.layout = None;
        self.tables.rev += 1;
        self.session.tick();
        Ok(())
    }

    fn bind_tab_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.tab_dialogs.bind.take() else {
            return;
        };
        let calc = self.calc();
        let current = calc.schema_of(&dialog.sheet).map(str::to_owned);
        let xml = calc.is_xml(&dialog.sheet);
        let schemas: Vec<String> = calc
            .sheet_names()
            .filter(|s| {
                calc.schema_format(s)
                    .is_some_and(|f| (f == cliotree_calc::SchemaFormat::Xsd) == xml)
            })
            .map(str::to_owned)
            .collect();
        let problem = calc.rebind_problem(&dialog.sheet, &dialog.schema);
        if problem.is_none()
            && dialog
                .problems
                .as_ref()
                .is_none_or(|(s, _)| s != &dialog.schema)
            && let (Ok(spec), Some(data)) =
                (calc.schema_spec(&dialog.schema), calc.data(&dialog.sheet))
        {
            dialog.problems = Some((dialog.schema.clone(), validate(&spec, data).len()));
        }
        let (mut apply, mut cancel, mut open) = (false, false, true);
        egui::Window::new(format!("Schema of {}", dialog.sheet))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Schema tab");
                    egui::ComboBox::from_id_salt("bind_tab_schema")
                        .selected_text(dialog.schema.clone())
                        .show_ui(ui, |ui| {
                            for schema in &schemas {
                                let label = if current.as_deref() == Some(schema.as_str()) {
                                    format!("{schema} (now)")
                                } else {
                                    schema.clone()
                                };
                                if ui
                                    .selectable_label(&dialog.schema == schema, label)
                                    .clicked()
                                {
                                    dialog.schema = schema.clone();
                                    dialog.error = None;
                                }
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Schema tab")
                        });
                });
                match (&problem, &dialog.problems) {
                    (Some(why), _) => {
                        ui.colored_label(error_color(ui), why);
                    }
                    (None, Some((_, 0))) => {
                        ui.label("The data fits it.");
                    }
                    (None, Some((_, n))) => {
                        ui.label(format!(
                            "{n} {} in the data would not fit it; the data is kept, and they are flagged.",
                            if *n == 1 { "value" } else { "values" }
                        ));
                    }
                    (None, None) => {}
                }
                if let Some(why) = &dialog.error {
                    ui.colored_label(error_color(ui), why);
                }
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(problem.is_none(), egui::Button::new("Apply"))
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        if apply && problem.is_none() {
            let (sheet, schema) = (dialog.sheet.clone(), dialog.schema.clone());
            if let Err(why) = self.bind_tab(&sheet, &schema) {
                dialog.error = Some(why);
                self.tab_dialogs.bind = Some(dialog);
            }
        } else if open && !cancel {
            self.tab_dialogs.bind = Some(dialog);
        }
    }

    /// *Format* on a tab's menu: writes the tab in another format. When that
    /// format has no comments and the tab has some, it asks first (the
    /// change waits in `tab_dialogs.format`).
    pub fn begin_set_format(&mut self, sheet: &str, format: DataFormat) -> Result<(), String> {
        let lost = self.calc().comments_lost(sheet, format);
        if lost > 0 {
            self.tab_dialogs.format = Some(FormatChange {
                sheet: sheet.to_owned(),
                format,
                lost,
            });
            return Ok(());
        }
        self.set_format(sheet, format)
    }

    /// Writes the tab in `format`, taking off the comments it cannot hold.
    pub fn set_format(&mut self, sheet: &str, format: DataFormat) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .set_format(sheet, format)
            .map_err(|e| e.to_string())?;
        self.session.tick();
        Ok(())
    }

    fn format_window(&mut self, ctx: &egui::Context) {
        let Some(change) = self.tab_dialogs.format.take() else {
            return;
        };
        let (mut yes, mut no, mut open) = (false, false, true);
        egui::Window::new(format!(
            "Write {} as {}",
            change.sheet,
            change.format.label()
        ))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "{} has no comments, so the {} on {} will be taken off. Undo puts them back.",
                change.format.label(),
                if change.lost == 1 {
                    "comment".to_owned()
                } else {
                    format!("{} comments", change.lost)
                },
                change.sheet
            ));
            ui.horizontal(|ui| {
                yes = ui.button("Yes").clicked();
                no = ui.button("No").clicked();
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                no = true;
            }
        });
        if yes {
            if let Err(why) = self.set_format(&change.sheet, change.format) {
                self.notice = Some(why);
            }
        } else if open && !no {
            self.tab_dialogs.format = Some(change);
        }
    }

    /// The dialogs of the tab commands, when they are open.
    pub(crate) fn tab_windows(&mut self, ctx: &egui::Context) {
        self.format_window(ctx);
        self.new_tab_window(ctx);
        self.delete_tab_window(ctx);
        self.bind_tab_window(ctx);
        self.import_window(ctx);
        self.export_window(ctx);
    }

    /// The items for tabs, in the Edit menu and a sheet tab's context menu.
    /// `tab` is the tab they act on.
    pub(crate) fn tab_menu_items(&mut self, ui: &mut egui::Ui, tab: &str) {
        let can_change = self.editing.is_none() && self.session.running_procedure().is_none();
        ui.add_enabled_ui(can_change, |ui| {
            if ui.button("New tab...").clicked() {
                self.begin_new_tab();
                ui.close();
            }
            if ui.button("Import...").clicked() {
                self.begin_import();
                ui.close();
            }
            if ui.button("Export...").clicked() {
                self.begin_export(tab);
                ui.close();
            }
            ui.separator();
            if ui.button("Rename tab...").clicked() {
                self.begin_rename_tab(tab);
                ui.close();
            }
            if ui.button("Delete tab...").clicked() {
                self.begin_delete_tab(tab);
                ui.close();
            }
            let data_tab = !self.calc().is_schema_sheet(tab);
            if ui
                .add_enabled(data_tab, egui::Button::new("Change schema..."))
                .clicked()
            {
                self.begin_bind_tab(tab);
                ui.close();
            }
            // The formats the tab could be written in; one alone is no choice.
            let formats = self.calc().formats_for(tab);
            let own = self.calc().format(tab);
            ui.add_enabled_ui(formats.len() > 1, |ui| {
                ui.menu_button("Format", |ui| {
                    for format in formats {
                        if ui.radio(own == Some(format), format.label()).clicked() {
                            if own != Some(format)
                                && let Err(why) = self.begin_set_format(tab, format)
                            {
                                self.notice = Some(why);
                            }
                            ui.close();
                        }
                    }
                });
            });
        });
    }
}

/// What *Delete tab* says about the formulas and modules that use the name.
pub(crate) fn delete_mentions_text(name: &str, mentions: Mentions) -> Option<String> {
    let text = crate::tab_rename::mentions_text(name, mentions)?;
    // "3 formulas use Orders. They are ..." becomes a warning about deleting.
    let (count, _) = text.split_once(". ")?;
    let one = mentions.formulas + mentions.modules == 1;
    Some(format!(
        "{count}; {} will show #NAME! until the tab is back.",
        if one { "it" } else { "they" }
    ))
}
