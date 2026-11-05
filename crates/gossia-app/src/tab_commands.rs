//! Adding, deleting and rebinding tabs from the window (DESIGN.md, "Managing
//! tabs"): the dialogs of *New tab...*, *Delete tab...* and *Change
//! schema...*, in the Edit menu and the context menu of a sheet tab.

use gossia_calc::{Mentions, SchemaFormat};
use gossia_core::copybook::{CodePage, FloatFormat};
use gossia_core::dataformat::DataFormat;
use gossia_core::report::ReportKind;
use gossia_core::validate::validate;
use gossia_core::xml::XmlDocument;
use gossia_core::{Node, Path, json, metaschema, xml, xsd};

use crate::app::GossiaApp;
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

/// What kind of tab *New tab...* makes (DESIGN.md, "Managing tabs").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    Data,
    Report,
}

/// The dialog of *New tab...*.
#[derive(Debug, Clone)]
pub struct NewTab {
    /// A data tab or a report.
    pub kind: NewKind,
    pub name: String,
    pub schema_name: String,
    pub schema: NewSchema,
    /// The format the data tab is written in, for a JSON Schema: JSON,
    /// JSON5, YAML, TOML and the rest (DESIGN.md, "Other file formats").
    pub format: DataFormat,
    /// For a report: single page or multipage.
    pub report_kind: ReportKind,
    /// For a report: the array of a tab it starts from, if any.
    pub start: Option<(String, Path)>,
    pub error: Option<String>,
    focused: bool,
    /// The name the dialog gave, which choosing the other kind replaces;
    /// a name typed by the user stays.
    given_name: String,
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
    /// Another layout of a group of `REDEFINES`, waiting for the answer to
    /// whether the formulas in the one shown may be deleted.
    pub redefine: Option<RedefineChange>,
    /// Another record of the copybook, waiting for the answer to whether
    /// the tab's formulas may be deleted.
    pub record: Option<RecordChange>,
}

/// *Record* on the menu of a tab of COBOL records, waiting for the answer
/// to whether the tab's formulas may be deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordChange {
    pub sheet: String,
    pub record: String,
    /// How many formulas would be deleted.
    pub formulas: usize,
}

/// *Layouts* on the menu of a tab of COBOL records, waiting for the answer
/// to whether the formulas in the layout shown may be deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedefineChange {
    pub sheet: String,
    pub set: usize,
    pub member: usize,
    /// The COBOL names of the layout shown and the one picked.
    pub from: String,
    pub to: String,
    /// How many formulas would be deleted.
    pub formulas: usize,
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
        if self.redefine.as_ref().is_some_and(|r| r.sheet == name) {
            self.redefine = None;
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

/// The same, for a format that holds a list (JSON Lines, YAML documents):
/// a list of objects.
const EMPTY_LIST_SCHEMA: &str =
    r#"{"type": "array", "items": {"type": "object", "properties": {}}}"#;

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

impl GossiaApp {
    pub(crate) fn can_change_tabs(&mut self) -> bool {
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
            kind: NewKind::Data,
            given_name: name.clone(),
            name,
            schema_name,
            schema: NewSchema::Own(SchemaFormat::Json),
            format: DataFormat::Json,
            report_kind: ReportKind::SinglePage,
            start: None,
            error: None,
            focused: false,
        });
    }

    /// *New report...*: the dialog of *New tab...* with Report chosen.
    pub fn begin_new_report(&mut self) {
        self.begin_new_tab();
        let name = self.report_name();
        if let Some(dialog) = self.tab_dialogs.new.as_mut() {
            dialog.kind = NewKind::Report;
            dialog.name = name.clone();
            dialog.given_name = name;
        }
    }

    /// The name a new report is offered: `Report`, or the next number free.
    fn report_name(&self) -> String {
        let calc = self.calc();
        unique("Report", |n| calc.tab_name_problem(n, None).is_some())
    }

    /// The name a new data tab is offered.
    fn data_name(&self) -> String {
        let calc = self.calc();
        unique("Data", |n| calc.tab_name_problem(n, None).is_some())
    }

    /// Why the new tab cannot be made as the dialog stands, or None.
    pub fn new_tab_problem(&self, dialog: &NewTab) -> Option<String> {
        let name = dialog.name.trim();
        if let Some(why) = self.calc().tab_name_problem(name, None) {
            return Some(format!("The tab name: {why}."));
        }
        if dialog.kind == NewKind::Report {
            return None;
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
                let spec = match self.calc().schema_spec(schema) {
                    Ok(spec) => spec,
                    Err(e) => return Some(e.to_string()),
                };
                if self.calc().schema_format(schema) == Some(SchemaFormat::Json)
                    && let Some(why) = dialog.format.root_problem(&spec.get(spec.root()).kind)
                {
                    return Some(format!(
                        "{schema} cannot be written as {}: {why}.",
                        dialog.format.label()
                    ));
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
        if dialog.kind == NewKind::Report {
            return self.create_report_tab(&dialog.name, dialog.report_kind, dialog.start.as_ref());
        }
        let name = dialog.name.trim();
        let json_data = match &dialog.schema {
            NewSchema::Own(format) => *format == SchemaFormat::Json,
            NewSchema::Existing(schema) => {
                self.calc().schema_format(schema) == Some(SchemaFormat::Json)
            }
        };
        // Only JSON-shaped data has a choice of format.
        let data_format = if json_data {
            dialog.format
        } else {
            DataFormat::Json
        };
        let before = self.calc().history_len().0;
        let calc = self.session.calc_mut();
        calc.begin_entry(&format!("New tab {name}"));
        let done = match &dialog.schema {
            NewSchema::Own(format) => {
                let schema_name = dialog.schema_name.trim();
                let empty = Node::new(gossia_core::Value::Object(Default::default()));
                match format {
                    SchemaFormat::Json => {
                        let list = matches!(
                            data_format,
                            DataFormat::JsonLines | DataFormat::YamlDocuments
                        );
                        let (text, data) = if list {
                            (
                                EMPTY_LIST_SCHEMA,
                                Node::new(gossia_core::Value::Array(Vec::new())),
                            )
                        } else {
                            (EMPTY_SCHEMA, empty)
                        };
                        let schema = json::parse(text).expect("the schema is JSON").node;
                        calc.create_json_schema_tab(schema_name, schema)
                            .and_then(|()| calc.create_data_tab(name, schema_name, data, None))
                    }
                    // A record named after the tab, with one field.
                    // Its fields are added on the copybook tab, so auto-update
                    // stays off.
                    SchemaFormat::Copybook => {
                        let copybook = gossia_core::copybook::parse(&blank_copybook(name))
                            .expect("the blank copybook is read");
                        let tree = gossia_core::copybook::to_node(&copybook, Default::default());
                        let records = Node::new(gossia_core::Value::Object(Default::default()));
                        calc.create_copybook_schema_tab(schema_name, tree)
                            .and_then(|()| calc.create_data_tab(name, schema_name, records, None))
                    }
                    // The root element is named after the tab.
                    SchemaFormat::Xsd => {
                        let schema = xml::parse(&blank_xsd(name), metaschema::xsd_meta())
                            .expect("the blank XSD is read");
                        let document = XmlDocument {
                            declaration: Some(gossia_core::xmltree::Declaration::default()),
                            ..XmlDocument::default()
                        };
                        calc.create_xsd_schema_tab(schema_name, schema.node, schema.document)
                            .and_then(|()| {
                                calc.create_data_tab(name, schema_name, empty, Some(document))
                            })
                    }
                }
                .and_then(|()| match format {
                    SchemaFormat::Copybook => Ok(()),
                    _ => calc.set_auto_update(schema_name, true),
                })
            }
            NewSchema::Existing(schema) => calc.schema_spec(schema).and_then(|spec| {
                let data = Node::new(GossiaApp::default_value(&spec, spec.root()));
                let xml = spec.xml().map(|_| XmlDocument {
                    declaration: Some(gossia_core::xmltree::Declaration::default()),
                    ..XmlDocument::default()
                });
                calc.create_data_tab(name, schema, data, xml)
            }),
        }
        // The format is set in the same entry.
        .and_then(|()| match data_format {
            DataFormat::Json => Ok(()),
            format => calc.set_format(name, format),
        });
        calc.end_entry();
        // A tab made before the failure is taken out again.
        if done.is_err() && self.calc().history_len().0 > before {
            let _ = self.session.calc_mut().undo();
            let _ = self.session.calc_mut().take_removed();
        }
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
        let starts = if dialog.kind == NewKind::Report {
            self.starting_points()
        } else {
            Vec::new()
        };
        let (data_name, report_name) = (self.data_name(), self.report_name());
        let (mut apply, mut cancel, mut open) = (false, false, true);
        egui::Window::new("New tab")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Make a");
                    for (kind, text) in [(NewKind::Data, "Data tab"), (NewKind::Report, "Report")] {
                        if ui.radio(dialog.kind == kind, text).clicked() && dialog.kind != kind {
                            dialog.kind = kind;
                            // The name offered for the other kind, unless one
                            // was typed.
                            if dialog.name == dialog.given_name {
                                dialog.name = match kind {
                                    NewKind::Data => data_name.clone(),
                                    NewKind::Report => report_name.clone(),
                                };
                                dialog.given_name = dialog.name.clone();
                            }
                        }
                    }
                });
                ui.add_space(4.0);
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
                if dialog.kind == NewKind::Report {
                    report_choices(ui, &mut dialog, &starts);
                } else {
                    ui.horizontal(|ui| {
                        let own = matches!(dialog.schema, NewSchema::Own(_));
                        if ui.radio(own, "Its own schema, called").clicked() && !own {
                            dialog.schema = NewSchema::Own(SchemaFormat::Json);
                        }
                        let response = ui.add_enabled(
                            own,
                            egui::TextEdit::singleline(&mut dialog.schema_name)
                                .desired_width(160.0),
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
                                (SchemaFormat::Copybook, "COBOL copybook (records)"),
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
                                        if ui.selectable_label(&current == schema, schema).clicked()
                                        {
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
                    // The format of JSON-shaped data: what the Raw view shows,
                    // and what the tab is saved and exported as.
                    let json_data = match &dialog.schema {
                        NewSchema::Own(format) => *format == SchemaFormat::Json,
                        NewSchema::Existing(schema) => {
                            self.calc().schema_format(schema) == Some(SchemaFormat::Json)
                        }
                    };
                    ui.add_enabled_ui(json_data, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Data format");
                            let shown = if json_data {
                                dialog.format.label()
                            } else {
                                "set by the schema"
                            };
                            egui::ComboBox::from_id_salt("new_tab_format")
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    for format in DataFormat::JSON_SHAPED {
                                        if ui
                                            .selectable_label(
                                                dialog.format == format,
                                                format.label(),
                                            )
                                            .clicked()
                                        {
                                            dialog.format = format;
                                        }
                                    }
                                })
                                .response
                                .widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::Role::ComboBox,
                                        true,
                                        "Data format of the new tab",
                                    )
                                });
                        });
                    });
                }
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
        let names = self.calc().tab_names();
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
        let report = self.calc().is_report(&dialog.name);
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
                    None if report => {
                        ui.label(format!(
                            "Delete the report {} with its design? Undo puts it back.",
                            dialog.name
                        ));
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
        // Schema tabs of the language the tab's data is in.
        let family = calc.spec(&dialog.sheet).map(SchemaFormat::of);
        let schemas: Vec<String> = calc
            .sheet_names()
            .filter(|s| family.is_some() && calc.schema_format(s) == family)
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

    /// *Layouts* on a tab of COBOL records: shows another layout of a group
    /// of `REDEFINES`. When the layout shown has formulas, it asks first (the
    /// change waits in `tab_dialogs.redefine`).
    pub fn begin_set_redefine(
        &mut self,
        sheet: &str,
        set: usize,
        member: usize,
    ) -> Result<(), String> {
        let formulas = self.calc().redefine_formulas(sheet, set).len();
        if formulas > 0 {
            let group = self
                .calc()
                .redefine_groups(sheet)
                .into_iter()
                .nth(set)
                .ok_or("there is no such group of layouts")?;
            let name = |i: usize| group.names.get(i).cloned().unwrap_or_default();
            self.tab_dialogs.redefine = Some(RedefineChange {
                sheet: sheet.to_owned(),
                set,
                member,
                from: name(group.shown),
                to: name(member),
                formulas,
            });
            return Ok(());
        }
        self.set_redefine(sheet, set, member)
    }

    /// Shows another layout of a group of `REDEFINES`, deleting the formulas
    /// in the one shown.
    pub fn set_redefine(&mut self, sheet: &str, set: usize, member: usize) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .set_redefine(sheet, set, member)
            .map_err(|e| e.to_string())?;
        self.rows_key = None;
        self.summary.key = None;
        self.raw.key = None;
        self.tables.layout = None;
        self.tables.rev += 1;
        self.session.tick();
        Ok(())
    }

    fn redefine_window(&mut self, ctx: &egui::Context) {
        let Some(change) = self.tab_dialogs.redefine.take() else {
            return;
        };
        let (mut yes, mut no, mut open) = (false, false, true);
        egui::Window::new(format!("Show {} in {}", change.to, change.sheet))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} has {} on its fields, which showing {} deletes. Delete {}? Undo puts {} back.",
                    change.from,
                    if change.formulas == 1 {
                        "a formula".to_owned()
                    } else {
                        format!("{} formulas", change.formulas)
                    },
                    change.to,
                    if change.formulas == 1 { "it" } else { "them" },
                    if change.formulas == 1 { "it" } else { "them" },
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
            if let Err(why) = self.set_redefine(&change.sheet, change.set, change.member) {
                self.notice = Some(why);
            }
        } else if open && !no {
            self.tab_dialogs.redefine = Some(change);
        }
    }

    /// *Record* on a tab of COBOL records: makes it hold another record of
    /// its copybook, of the same length. When the tab has formulas, it asks
    /// first (the change waits in `tab_dialogs.record`).
    pub fn begin_set_record(&mut self, sheet: &str, record: &str) -> Result<(), String> {
        if let Some(why) = self.calc().record_problem(sheet, record) {
            return Err(why);
        }
        let root = gossia_core::Path::root();
        let formulas = self.calc().formulas_inside(sheet, &root).len()
            + usize::from(self.calc().formula_text(sheet, &root).is_some());
        if formulas > 0 {
            self.tab_dialogs.record = Some(RecordChange {
                sheet: sheet.to_owned(),
                record: record.to_owned(),
                formulas,
            });
            return Ok(());
        }
        self.set_record(sheet, record)
    }

    /// Makes a tab of COBOL records hold another record of its copybook,
    /// deleting its formulas.
    pub fn set_record(&mut self, sheet: &str, record: &str) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .set_record(sheet, record)
            .map_err(|e| e.to_string())?;
        self.rows_key = None;
        self.summary.key = None;
        self.raw.key = None;
        self.tables.layout = None;
        self.tables.rev += 1;
        self.session.tick();
        Ok(())
    }

    /// *Variable length (VB)* on a tab of COBOL records: whether its file is
    /// a variable-length record after a record descriptor word.
    pub fn set_variable(&mut self, sheet: &str, on: bool) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .set_variable(sheet, on)
            .map_err(|e| e.to_string())?;
        self.raw.key = None;
        self.session.tick();
        Ok(())
    }

    fn record_window(&mut self, ctx: &egui::Context) {
        let Some(change) = self.tab_dialogs.record.take() else {
            return;
        };
        let (mut yes, mut no, mut open) = (false, false, true);
        let one = change.formulas == 1;
        egui::Window::new(format!("Hold {} in {}", change.record, change.sheet))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} has {}, which holding {} deletes: its fields are others. Delete {}? Undo puts {} back.",
                    change.sheet,
                    if one {
                        "a formula".to_owned()
                    } else {
                        format!("{} formulas", change.formulas)
                    },
                    change.record,
                    if one { "it" } else { "them" },
                    if one { "it" } else { "them" },
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
            if let Err(why) = self.set_record(&change.sheet, &change.record) {
                self.notice = Some(why);
            }
        } else if open && !no {
            self.tab_dialogs.record = Some(change);
        }
    }

    /// *Floating point* on a copybook schema tab: how its `COMP-1` and
    /// `COMP-2` items hold their numbers, one entry in the history.
    pub fn set_float_format(&mut self, sheet: &str, format: FloatFormat) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .set_value(
                sheet,
                &gossia_core::Path::root().key("floatFormat"),
                gossia_core::Value::Str(format.code().to_owned()),
            )
            .map_err(|e| e.to_string())?;
        self.raw.key = None;
        self.session.tick();
        Ok(())
    }

    /// How a copybook schema tab's floating point items hold their numbers.
    fn float_format_of(&self, sheet: &str) -> FloatFormat {
        let node = self
            .calc()
            .data(sheet)
            .and_then(|d| d.get(&gossia_core::Path::root().key("floatFormat")));
        match node.map(|n| n.value()) {
            Some(gossia_core::Value::Str(code)) => FloatFormat::from_code(code).unwrap_or_default(),
            _ => FloatFormat::default(),
        }
    }

    /// *Code page* on a copybook schema tab: the EBCDIC code page of its
    /// records' text, one entry in the history.
    pub fn set_code_page(&mut self, sheet: &str, page: CodePage) -> Result<(), String> {
        if !self.can_change_tabs() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        self.session
            .calc_mut()
            .set_value(
                sheet,
                &gossia_core::Path::root().key("codePage"),
                gossia_core::Value::Str(page.code().to_owned()),
            )
            .map_err(|e| e.to_string())?;
        self.raw.key = None;
        self.session.tick();
        Ok(())
    }

    /// The code page a copybook schema tab gives its records.
    fn code_page_of(&self, sheet: &str) -> Option<CodePage> {
        let data = self.calc().data(sheet)?;
        match data
            .get(&gossia_core::Path::root().key("codePage"))?
            .value()
        {
            gossia_core::Value::Str(code) => CodePage::from_code(code),
            _ => None,
        }
    }

    /// The dialogs of the tab commands, when they are open.
    pub(crate) fn tab_windows(&mut self, ctx: &egui::Context) {
        self.format_window(ctx);
        self.redefine_window(ctx);
        self.record_window(ctx);
        self.new_tab_window(ctx);
        self.delete_tab_window(ctx);
        self.bind_tab_window(ctx);
        self.import_window(ctx);
        self.export_window(ctx);
        self.pdf_window(ctx);
    }

    /// The items for tabs, in the Edit menu and a sheet tab's context menu.
    /// `tab` is the tab they act on.
    pub(crate) fn tab_menu_items(&mut self, ui: &mut egui::Ui, tab: &str) {
        if self.calc().is_report(tab) {
            self.report_menu_items(ui, tab);
            return;
        }
        let can_change = self.editing.is_none() && self.session.running_procedure().is_none();
        ui.add_enabled_ui(can_change, |ui| {
            if ui.button("New tab...").clicked() {
                self.begin_new_tab();
                ui.close();
            }
            if ui.button("New report...").clicked() {
                self.begin_new_report();
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
            // A copybook schema tab: the code page of its records' text.
            if self.calc().schema_format(tab) == Some(SchemaFormat::Copybook) {
                let own = self.code_page_of(tab);
                ui.menu_button("Code page", |ui| {
                    for page in CodePage::ALL {
                        if ui.radio(own == Some(page), page.label()).clicked() {
                            if own != Some(page)
                                && let Err(why) = self.set_code_page(tab, page)
                            {
                                self.notice = Some(why);
                            }
                            ui.close();
                        }
                    }
                });
                let own = self.float_format_of(tab);
                ui.menu_button("Floating point", |ui| {
                    for format in FloatFormat::ALL {
                        if ui.radio(own == format, format.label()).clicked() {
                            if own != format
                                && let Err(why) = self.set_float_format(tab, format)
                            {
                                self.notice = Some(why);
                            }
                            ui.close();
                        }
                    }
                });
            }
            // A tab of COBOL records: which record of its copybook it holds,
            // and whether its file is a variable-length record.
            if let (Some(own), Some(schema)) = (
                self.calc().record_of(tab),
                self.calc().schema_of(tab).map(str::to_owned),
            ) {
                let records = self.calc().copybook_records(&schema);
                let variable = self.calc().is_variable(tab);
                ui.menu_button("Record", |ui| {
                    for record in &records {
                        let problem = self.calc().record_problem(tab, record);
                        let mine = record.eq_ignore_ascii_case(&own);
                        let response = ui.add_enabled(
                            mine || problem.is_none(),
                            egui::RadioButton::new(mine, record),
                        );
                        let response = match &problem {
                            Some(why) if !mine => response.on_disabled_hover_text(why),
                            _ => response,
                        };
                        if response.clicked() {
                            if !mine && let Err(why) = self.begin_set_record(tab, record) {
                                self.notice = Some(why);
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    let mut on = variable;
                    if ui
                        .checkbox(
                            &mut on,
                            "Variable length (VB, with a record descriptor word)",
                        )
                        .clicked()
                    {
                        if let Err(why) = self.set_variable(tab, on) {
                            self.notice = Some(why);
                        }
                        ui.close();
                    }
                });
            }
            // A tab of COBOL records: which layout of each group of
            // REDEFINES it shows. FILLER has no fields to show.
            let groups = self.calc().redefine_groups(tab);
            if !groups.is_empty() {
                ui.menu_button("Layouts", |ui| {
                    for (set, group) in groups.iter().enumerate() {
                        ui.menu_button(group.names.join(" / "), |ui| {
                            for (member, name) in group.names.iter().enumerate() {
                                let filler = group.filler.get(member).copied().unwrap_or(false);
                                let response = ui.add_enabled(
                                    !filler,
                                    egui::RadioButton::new(group.shown == member, name),
                                );
                                if response.clicked() {
                                    if group.shown != member
                                        && let Err(why) = self.begin_set_redefine(tab, set, member)
                                    {
                                        self.notice = Some(why);
                                    }
                                    ui.close();
                                }
                            }
                        });
                    }
                });
            }
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

/// The copybook a new copybook schema tab starts with: a record named after
/// the tab, with one field of text.
fn blank_copybook(tab: &str) -> String {
    let name: String = tab
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-');
    let name = if name.is_empty() { "RECORD" } else { name };
    format!("       01  {name}-REC.\n           05  {name}-ID  PIC X(10).\n")
}

/// What *New tab...* asks for a report: its kind, and the array it starts
/// from, if any.
fn report_choices(ui: &mut egui::Ui, dialog: &mut NewTab, starts: &[(String, Path, String)]) {
    ui.horizontal(|ui| {
        ui.label("Kind");
        for kind in ReportKind::ALL {
            // Starting from an array makes a multipage report.
            let allowed = dialog.start.is_none() || kind == ReportKind::Multipage;
            let response = ui
                .add_enabled(
                    allowed,
                    egui::RadioButton::new(dialog.report_kind == kind, kind.label()),
                )
                .on_disabled_hover_text(
                    "A report that starts from an array is multipage: its detail band repeats \
                     for each item.",
                );
            if response.clicked() {
                dialog.report_kind = kind;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Start from");
        let shown = dialog
            .start
            .as_ref()
            .and_then(|(tab, path)| {
                starts
                    .iter()
                    .find(|(t, p, _)| t == tab && p == path)
                    .map(|(.., shown)| shown.clone())
            })
            .unwrap_or_else(|| "nothing (an empty report)".to_owned());
        egui::ComboBox::from_id_salt("new_report_start")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(dialog.start.is_none(), "nothing (an empty report)")
                    .clicked()
                {
                    dialog.start = None;
                }
                for (tab, path, shown) in starts {
                    let chosen = dialog
                        .start
                        .as_ref()
                        .is_some_and(|(t, p)| t == tab && p == path);
                    if ui.selectable_label(chosen, shown).clicked() {
                        dialog.start = Some((tab.clone(), path.clone()));
                        dialog.report_kind = ReportKind::Multipage;
                    }
                }
            })
            .response
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::ComboBox,
                    true,
                    "Array the report starts from",
                )
            });
    });
    ui.weak(
        "Empty, a report's controls are added in Edit report. From an array, it is a multipage \
         report whose detail band shows each item's fields.",
    );
}
