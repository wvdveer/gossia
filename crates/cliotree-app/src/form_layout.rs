//! Named forms and customising them (DESIGN.md, "Forms"): the commands that
//! make, copy, rename and delete forms, layout mode where the form being shown
//! is edited on the form itself, and the property page of the selected control.
//!
//! A form that has not been customised is generated from the schema and is not
//! stored. The first change to it **materialises** it: the generated design is
//! stored, and edited from then on. Edits to a design are not in the
//! workbook's undo history, like module edits and tab operations; layout mode
//! has its own undo and redo, which last while it is on.

use cliotree_calc::{ExprKind, entry};
use cliotree_core::form::{
    self as design, Address, Alignment, ColumnDesign, Control, ControlKind, Editor, FormDesign,
    RepeaterMode,
};
use cliotree_core::{Kind, Path};

use crate::app::ClioApp;
use crate::form::{DEFAULT_FORM, View, control_name, last_name};
use crate::icons;
use crate::view::{error_color, named};

/// What layout mode keeps while it is on.
#[derive(Debug, Clone)]
pub struct LayoutState {
    /// The tab and the form being edited.
    pub sheet: String,
    pub form: String,
    pub selected: Option<Address>,
    /// A column of the table of the selected control, when one is selected
    /// rather than the control.
    pub column: Option<String>,
    /// What the form was before each change: `None` is a form that was not
    /// stored, a generated one.
    pub undo: Vec<Option<FormDesign>>,
    pub redo: Vec<Option<FormDesign>>,
    /// The property last typed into, so that typing is one change and not one
    /// for each letter.
    merge: Option<String>,
    /// The formula being typed for the node of the selected field.
    formula: Option<(Path, String)>,
}

impl LayoutState {
    fn new(sheet: &str, form: &str) -> LayoutState {
        LayoutState {
            sheet: sheet.to_owned(),
            form: form.to_owned(),
            selected: None,
            column: None,
            undo: Vec::new(),
            redo: Vec::new(),
            merge: None,
            formula: None,
        }
    }
}

/// A command on a control, from its menu or the property page.
#[derive(Debug, Clone)]
pub(crate) enum ControlAction {
    Select(Address),
    Shift(Address, bool),
    /// Into the group above (`true`), or out of the group.
    Nest(Address, bool),
    Delete(Address),
    /// Select a column of the table of a repeater, by its name.
    SelectColumn(Address, String),
    /// Move or show a column of the table of a repeater.
    Column(Address, String, ColumnEdit),
}

/// A change to the place of a column, or to whether it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnEdit {
    /// One place earlier (`true`) or later in the list of the table.
    Step(bool),
    /// To the left (`true`) or the right of the next column that is shown.
    Move(bool),
    Show(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormCommand {
    New,
    Duplicate,
    Rename,
    /// Delete a form that is not Default.
    Delete,
    /// Discard the design of Default, so that it is generated again.
    Reset,
}

/// A command on forms that waits for a name, or for a yes.
#[derive(Debug, Clone)]
pub struct FormDialog {
    pub command: FormCommand,
    pub name: String,
    pub error: Option<String>,
}

impl ClioApp {
    // ----------------------------------------------------------- designs

    /// The design of a form of the current tab: the one that is stored, or the
    /// one generated from the schema.
    pub fn design_of(&self, form: &str) -> FormDesign {
        if let Some(stored) = self.calc().forms().get(&self.sheet, form) {
            return stored.clone();
        }
        self.calc()
            .spec(&self.sheet)
            .map(design::generate)
            .unwrap_or_default()
    }

    /// Whether the form has been customised (stored), as opposed to generated.
    pub fn is_customised(&self, form: &str) -> bool {
        self.calc().forms().get(&self.sheet, form).is_some()
    }

    fn put_design(&mut self, form: &str, design: Option<FormDesign>) {
        let sheet = self.sheet.clone();
        let forms = self.session.calc_mut().forms_mut();
        match design {
            Some(design) => forms.set(&sheet, form, design),
            None => {
                forms.remove(&sheet, form);
            }
        }
    }

    /// Changes the design of the form being shown. `change` returns what to
    /// select afterwards, or None if it changed nothing. Returns whether a
    /// change was made. The first change to a generated form stores it.
    pub fn edit_design(
        &mut self,
        change: impl FnOnce(&mut FormDesign) -> Option<Option<Address>>,
    ) -> bool {
        self.edit_design_merging(None, change)
    }

    fn edit_design_merging(
        &mut self,
        merge: Option<String>,
        change: impl FnOnce(&mut FormDesign) -> Option<Option<Address>>,
    ) -> bool {
        let form = self.form_name();
        let before = self.calc().forms().get(&self.sheet, &form).cloned();
        let mut design = before.clone().unwrap_or_else(|| self.design_of(&form));
        let Some(selection) = change(&mut design) else {
            return false;
        };
        if before.as_ref() == Some(&design) {
            return false;
        }
        self.put_design(&form, Some(design));
        let sheet = self.sheet.clone();
        if let Some(layout) = self
            .form
            .layout
            .as_mut()
            .filter(|l| l.sheet == sheet && l.form == form)
        {
            let joins = merge.is_some() && layout.merge == merge;
            if !joins {
                layout.undo.push(before);
            }
            layout.redo.clear();
            layout.merge = merge;
            if layout.selected != selection {
                layout.column = None;
            }
            layout.selected = selection;
        }
        true
    }

    // ------------------------------------------------------- layout mode

    /// In layout mode on the form being shown: the selected control, if any.
    pub(crate) fn layout_selection(&self, form: &str) -> Option<Option<Address>> {
        self.form
            .layout
            .as_ref()
            .filter(|l| l.sheet == self.sheet && l.form == form)
            .map(|l| l.selected.clone())
    }

    /// Whether the form being shown is in layout mode.
    pub fn layout_active(&self) -> bool {
        self.in_form() && self.layout_selection(&self.form_name()).is_some()
    }

    /// Turns layout mode on or off for the form being shown.
    pub fn set_layout(&mut self, on: bool) {
        if on && self.in_form() {
            let form = self.form_name();
            self.form.layout = Some(LayoutState::new(&self.sheet, &form));
        } else {
            self.form.layout = None;
        }
    }

    /// The control selected in layout mode.
    pub fn selected_control(&self) -> Option<Address> {
        self.layout_selection(&self.form_name()).flatten()
    }

    /// Selects a control in layout mode.
    pub fn select_control(&mut self, address: Option<Address>) {
        if let Some(layout) = self.form.layout.as_mut() {
            layout.selected = address;
            layout.column = None;
            layout.merge = None;
        }
    }

    /// The column selected in layout mode: the repeater, and the column's name.
    pub fn selected_column(&self) -> Option<(Address, String)> {
        let at = self.selected_control()?;
        let column = self.form.layout.as_ref()?.column.clone()?;
        Some((at, column))
    }

    /// Selects a column of the table of a repeater in layout mode.
    pub fn select_table_column(&mut self, at: Address, name: &str) {
        if let Some(layout) = self.form.layout.as_mut() {
            layout.selected = Some(at);
            layout.column = Some(name.to_owned());
            layout.merge = None;
        }
    }

    pub fn can_layout_undo(&self) -> bool {
        self.form
            .layout
            .as_ref()
            .is_some_and(|l| !l.undo.is_empty())
    }

    pub fn can_layout_redo(&self) -> bool {
        self.form
            .layout
            .as_ref()
            .is_some_and(|l| !l.redo.is_empty())
    }

    /// Takes back the last change to the design.
    pub fn layout_undo(&mut self) {
        self.step_layout(true);
    }

    pub fn layout_redo(&mut self) {
        self.step_layout(false);
    }

    fn step_layout(&mut self, undo: bool) {
        let Some(layout) = self.form.layout.as_mut() else {
            return;
        };
        let target = if undo {
            layout.undo.pop()
        } else {
            layout.redo.pop()
        };
        let Some(target) = target else {
            return;
        };
        layout.selected = None;
        layout.column = None;
        layout.merge = None;
        let form = layout.form.clone();
        let current = self.calc().forms().get(&self.sheet, &form).cloned();
        if let Some(layout) = self.form.layout.as_mut() {
            if undo {
                layout.redo.push(current);
            } else {
                layout.undo.push(current);
            }
        }
        self.put_design(&form, target);
    }

    pub(crate) fn apply_control(&mut self, action: ControlAction) {
        match action {
            ControlAction::Select(at) => self.select_control(Some(at)),
            ControlAction::Shift(at, up) => {
                self.edit_design(|d| d.shift(&at, up).map(Some));
            }
            ControlAction::Nest(at, into) => {
                self.edit_design(|d| d.nest(&at, into).map(Some));
            }
            ControlAction::Delete(at) => {
                self.edit_design(|d| d.remove(&at).map(|_| None));
            }
            ControlAction::SelectColumn(at, name) => self.select_table_column(at, &name),
            ControlAction::Column(at, name, edit) => {
                self.edit_column_place(&at, &name, edit);
            }
        }
    }

    /// Adds a control to the form: into the selected group, or after the
    /// selected control, or at the end. The new control is selected.
    pub fn add_control(&mut self, control: Control) -> bool {
        let selected = self.selected_control();
        self.edit_design(|d| {
            let at = match selected {
                Some(at) if d.get(&at).is_some_and(|c| c.kind == ControlKind::Group) => {
                    d.insert(&at, None, control)
                }
                Some(at) => {
                    let (last, parent) = at.split_last()?;
                    d.insert(parent, Some(last + 1), control)
                }
                None => d.insert(&[], None, control),
            };
            at.map(Some)
        })
    }

    /// Adds the control the generated form has for a field that is not on the
    /// form.
    pub fn add_field_to_form(&mut self, path: &Path) -> bool {
        let control = self
            .calc()
            .spec(&self.sheet)
            .and_then(|spec| design::control_for(spec, path));
        match control {
            Some(control) => self.add_control(control),
            None => false,
        }
    }

    // ------------------------------------------------------- the columns

    /// The names of the columns of the table of the repeater at `at`, in the
    /// order of the schema, if it is a table.
    fn column_names(&self, design: &FormDesign, at: &[usize]) -> Option<Vec<String>> {
        let spec = self.calc().spec(&self.sheet)?;
        let columns = crate::form::columns_of(spec, design.get(at)?)?;
        Some(columns.into_iter().map(|c| c.name).collect())
    }

    /// Changes the columns of the table of the repeater at `at`, as the
    /// control arranges them. `change` returns whether it changed anything.
    fn edit_columns(
        &mut self,
        merge: Option<String>,
        at: &Address,
        change: impl FnOnce(&mut Vec<ColumnDesign>) -> bool,
    ) -> bool {
        let design = self.design_of(&self.form_name());
        let Some(names) = self.column_names(&design, at) else {
            return false;
        };
        let at = at.clone();
        self.edit_design_merging(merge, |d| {
            let control = d.get_mut(&at)?;
            let mut columns = control.arrange_columns(&names);
            if !change(&mut columns) {
                return None;
            }
            control.set_columns(columns, &names);
            Some(Some(at))
        })
    }

    /// Moves a column, or shows or hides it. The last column of the items
    /// that is shown cannot be hidden.
    pub(crate) fn edit_column_place(&mut self, at: &Address, name: &str, edit: ColumnEdit) -> bool {
        self.edit_columns(None, at, |columns| {
            let Some(i) = columns.iter().position(|c| c.name == name) else {
                return false;
            };
            match edit {
                ColumnEdit::Show(on) => {
                    let shown = columns
                        .iter()
                        .filter(|c| !c.hidden && !c.is_row_number())
                        .count();
                    let last = !columns[i].is_row_number() && !columns[i].hidden && shown == 1;
                    if columns[i].hidden != on || (!on && last) {
                        return false;
                    }
                    columns[i].hidden = !on;
                    true
                }
                ColumnEdit::Step(up) | ColumnEdit::Move(up) => {
                    let past_hidden = matches!(edit, ColumnEdit::Move(_));
                    let target = if up {
                        (0..i).rev().find(|&j| !past_hidden || !columns[j].hidden)
                    } else {
                        (i + 1..columns.len()).find(|&j| !past_hidden || !columns[j].hidden)
                    };
                    let Some(j) = target else {
                        return false;
                    };
                    if up {
                        columns[j..=i].rotate_right(1);
                    } else {
                        columns[i..=j].rotate_left(1);
                    }
                    true
                }
            }
        })
    }

    /// Changes the properties of one column of the table at `at`. A column
    /// that is selected stays selected.
    pub fn set_column(
        &mut self,
        merge: Option<String>,
        at: &Address,
        column: ColumnDesign,
    ) -> bool {
        self.edit_columns(merge, at, |columns| {
            match columns.iter_mut().find(|c| c.name == column.name) {
                Some(slot) if *slot != column => {
                    *slot = column;
                    true
                }
                _ => false,
            }
        })
    }

    // ------------------------------------------------------- the commands

    fn check_form_name(&self, name: &str, renaming: Option<&str>) -> Result<(), String> {
        if name.eq_ignore_ascii_case("Raw") {
            return Err("Raw is the name of a view of every tab".to_owned());
        }
        match self
            .calc()
            .forms()
            .name_problem(&self.sheet, name, renaming)
        {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }

    /// Makes a form, starting from the generated one, and shows it.
    pub fn new_form(&mut self, name: &str) -> Result<(), String> {
        self.check_form_name(name, None)?;
        let design = self
            .calc()
            .spec(&self.sheet)
            .map(design::generate)
            .ok_or("there is no such tab")?;
        self.put_design(name, Some(design));
        self.show_form(name);
        Ok(())
    }

    /// Makes a copy of the form being shown, and shows the copy.
    pub fn duplicate_form(&mut self, name: &str) -> Result<(), String> {
        self.check_form_name(name, None)?;
        let design = self.design_of(&self.form_name());
        self.put_design(name, Some(design));
        self.show_form(name);
        Ok(())
    }

    /// Renames the form being shown. Default cannot be renamed.
    pub fn rename_form(&mut self, to: &str) -> Result<(), String> {
        let from = self.form_name();
        self.check_form_name(to, Some(&from))?;
        let sheet = self.sheet.clone();
        self.session
            .calc_mut()
            .forms_mut()
            .rename(&sheet, &from, to)?;
        self.views.insert(sheet, View::Form(to.to_owned()));
        self.form.layout = None;
        Ok(())
    }

    /// Deletes the form being shown, and shows Default. Default is not
    /// deleted: see [`ClioApp::reset_form`].
    pub fn delete_form(&mut self) -> Result<(), String> {
        let form = self.form_name();
        if form == DEFAULT_FORM {
            return Err("the Default form cannot be deleted; reset it instead".to_owned());
        }
        self.put_design(&form, None);
        self.show_form(DEFAULT_FORM);
        Ok(())
    }

    /// Discards the design of Default, so that it is generated again.
    pub fn reset_form(&mut self) -> Result<(), String> {
        if self.form_name() != DEFAULT_FORM {
            return Err("only the Default form is reset; delete the others".to_owned());
        }
        self.put_design(DEFAULT_FORM, None);
        self.form.layout = None;
        Ok(())
    }

    fn show_form(&mut self, name: &str) {
        self.form.layout = None;
        self.raw.error = None;
        self.views
            .insert(self.sheet.clone(), View::Form(name.to_owned()));
        self.notice = None;
    }

    /// Opens the dialog of a command on forms.
    pub fn begin_form_command(&mut self, command: FormCommand) {
        let form = self.form_name();
        let name = match command {
            FormCommand::New => String::new(),
            FormCommand::Duplicate => format!("{form}_copy"),
            FormCommand::Rename => form,
            FormCommand::Delete | FormCommand::Reset => String::new(),
        };
        self.form.dialog = Some(FormDialog {
            command,
            name,
            error: None,
        });
    }

    /// Does what the open dialog says. If it is refused, the dialog stays open
    /// with the reason.
    pub fn apply_form_dialog(&mut self) {
        let Some(mut dialog) = self.form.dialog.take() else {
            return;
        };
        let name = dialog.name.trim().to_owned();
        let result = match dialog.command {
            FormCommand::New => self.new_form(&name),
            FormCommand::Duplicate => self.duplicate_form(&name),
            FormCommand::Rename => self.rename_form(&name),
            FormCommand::Delete => self.delete_form(),
            FormCommand::Reset => self.reset_form(),
        };
        if let Err(message) = result {
            dialog.error = Some(message);
            self.form.dialog = Some(dialog);
        }
    }

    /// The dialog of a command on forms, when one is waiting.
    pub(crate) fn form_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.form.dialog.take() else {
            return;
        };
        let form = self.form_name();
        let title = match dialog.command {
            FormCommand::New => "New form".to_owned(),
            FormCommand::Duplicate => format!("Duplicate form {form}"),
            FormCommand::Rename => format!("Rename form {form}"),
            FormCommand::Delete => format!("Delete form {form}"),
            FormCommand::Reset => "Reset the Default form".to_owned(),
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                match dialog.command {
                    FormCommand::New | FormCommand::Duplicate | FormCommand::Rename => {
                        ui.label(match dialog.command {
                            FormCommand::New => "Name of the new form",
                            FormCommand::Duplicate => "Name of the copy",
                            _ => "New name",
                        });
                        let response = ui.text_edit_singleline(&mut dialog.name);
                        named(&response, "Form name");
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            apply = true;
                        }
                        ui.label(
                            "A form name follows the rules of a tab name, and is not used \
                             by another form of this tab.",
                        );
                    }
                    FormCommand::Delete => {
                        ui.label(format!("Delete the form {form}? Its design is lost."));
                    }
                    FormCommand::Reset => {
                        ui.label(
                            "Discard the design of the Default form? It is generated from \
                             the schema again.",
                        );
                    }
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(error_color(ui), error);
                }
                ui.horizontal(|ui| {
                    let label = match dialog.command {
                        FormCommand::New => "Create",
                        FormCommand::Duplicate => "Duplicate",
                        FormCommand::Rename => "Rename",
                        FormCommand::Delete => "Delete",
                        FormCommand::Reset => "Reset",
                    };
                    apply |= ui.button(label).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if apply {
            self.form.dialog = Some(dialog);
            self.apply_form_dialog();
        } else if open && !cancel {
            self.form.dialog = Some(dialog);
        }
    }

    // --------------------------------------------------------- the menu

    /// The Forms submenu of the View menu.
    pub(crate) fn forms_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Forms", |ui| {
            let in_form = self.in_form();
            let form = self.form_name();
            let is_default = form == DEFAULT_FORM;
            let mut command = None;
            let mut item = |ui: &mut egui::Ui, label: &str, enabled: bool, c: FormCommand| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    command = Some(c);
                    ui.close();
                }
            };
            item(ui, "New form...", true, FormCommand::New);
            item(ui, "Duplicate form...", in_form, FormCommand::Duplicate);
            item(
                ui,
                "Rename form...",
                in_form && !is_default,
                FormCommand::Rename,
            );
            item(
                ui,
                "Delete form...",
                in_form && !is_default,
                FormCommand::Delete,
            );
            item(
                ui,
                "Reset Default form...",
                in_form && is_default && self.is_customised(&form),
                FormCommand::Reset,
            );
            ui.separator();
            let mut on = self.layout_active();
            if ui
                .add_enabled(in_form, egui::Checkbox::new(&mut on, "Edit form"))
                .clicked()
            {
                self.set_layout(on);
                ui.close();
            }
            if let Some(command) = command {
                self.begin_form_command(command);
            }
        });
    }

    // ------------------------------------------------------- the strip

    /// The strip above a form: the switch to layout mode, and in layout mode
    /// the palette, undo and redo, and discarding the design.
    pub(crate) fn form_bar(&mut self, ui: &mut egui::Ui, form: &str) {
        ui.horizontal_wrapped(|ui| {
            let mut on = self.layout_active();
            if ui.checkbox(&mut on, "Edit form").changed() {
                self.set_layout(on);
            }
            if !on {
                return;
            }
            ui.separator();
            for kind in ControlKind::ALL {
                let label = format!("Add {}", kind.label().to_lowercase());
                if ui.button(label).clicked() {
                    let control = match kind {
                        ControlKind::Label => {
                            let mut c = Control::new(kind);
                            c.text = Some("Label".to_owned());
                            c
                        }
                        ControlKind::Button => {
                            let mut c = Control::new(kind);
                            c.label = Some("Button".to_owned());
                            c
                        }
                        _ => Control::new(kind),
                    };
                    self.add_control(control);
                }
            }
            ui.separator();
            if ui
                .add_enabled(self.can_layout_undo(), egui::Button::new("Undo layout"))
                .clicked()
            {
                self.layout_undo();
            }
            if ui
                .add_enabled(self.can_layout_redo(), egui::Button::new("Redo layout"))
                .clicked()
            {
                self.layout_redo();
            }
            ui.separator();
            if form == DEFAULT_FORM {
                let stored = self.is_customised(form);
                if ui
                    .add_enabled(stored, egui::Button::new("Reset to default"))
                    .clicked()
                {
                    self.begin_form_command(FormCommand::Reset);
                }
            } else if ui.button("Delete form").clicked() {
                self.begin_form_command(FormCommand::Delete);
            }
        });
    }

    // --------------------------------------------------- the property page

    /// The property page of layout mode: the selected control, the fields
    /// that are not on the form, and the controls that are broken.
    pub(crate) fn property_page(&mut self, ui: &mut egui::Ui) {
        let form = self.form_name();
        let design = self.design_of(&form);
        let selected = self.selected_control();
        let column = self.selected_column();
        ui.heading("Properties");
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let column = column.and_then(|(at, name)| {
                    let control = design.get(&at)?;
                    let names = self.column_names(&design, &at)?;
                    let column = control
                        .arrange_columns(&names)
                        .into_iter()
                        .find(|c| c.name == name)?;
                    Some((at, control.clone(), column))
                });
                if let Some((at, control, column)) = column {
                    self.column_properties(ui, &at, &control, column);
                } else {
                    match selected
                        .as_ref()
                        .and_then(|at| design.get(at).map(|c| (at, c)))
                    {
                        Some((at, control)) => self.control_properties(ui, &design, at, control),
                        None => {
                            ui.weak("Select a control on the form to change it.");
                        }
                    }
                }
                ui.separator();
                self.unplaced_list(ui, &design);
                self.broken_list(ui, &design);
            });
    }

    fn control_properties(
        &mut self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        at: &Address,
        control: &Control,
    ) {
        let mut edited = control.clone();
        let spec = self.calc().spec(&self.sheet).cloned();
        ui.strong(control_name(control));
        let key = |property: &str| format!("{at:?}:{property}");
        let mut merge = None;
        let mut column_edit = None;
        let kind = control.kind;
        let bound = matches!(
            kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater
        );

        // What the control is bound to.
        if bound && let Some(spec) = &spec {
            let options = design::bindable(spec, kind);
            let current = edited.path.clone();
            let mut chosen = current.clone();
            ui.horizontal(|ui| {
                ui.label("Path");
                let text = current.clone().unwrap_or_else(|| "(none)".to_owned());
                let response = egui::ComboBox::from_id_salt("property_path")
                    .selected_text(text)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        if kind == ControlKind::Group {
                            ui.selectable_value(&mut chosen, None, "(none)");
                        }
                        for path in &options {
                            ui.selectable_value(
                                &mut chosen,
                                Some(path.to_string()),
                                path.to_string(),
                            );
                        }
                    })
                    .response;
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Path of the control")
                });
            });
            if chosen != current {
                edited.path = chosen;
            }
        }
        if matches!(
            kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater | ControlKind::Button
        ) {
            let hint = edited
                .bound_path()
                .and_then(|p| last_name(&p))
                .unwrap_or_default();
            if option_text(ui, "Label", &mut edited.label, &hint, false) {
                merge = Some(key("label"));
            }
        }
        if bound && option_text(ui, "Help", &mut edited.help, "", true) {
            merge = Some(key("help"));
        }
        match kind {
            ControlKind::Field => {
                ui.horizontal(|ui| {
                    ui.label("Editor");
                    let mut editor = edited.editor.unwrap_or(Editor::Auto);
                    let response = egui::ComboBox::from_id_salt("property_editor")
                        .selected_text(editor.label())
                        .show_ui(ui, |ui| {
                            for e in Editor::ALL {
                                ui.selectable_value(&mut editor, e, e.label());
                            }
                        })
                        .response;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Editor of the field")
                    });
                    edited.editor = (editor != Editor::Auto).then_some(editor);
                });
                ui.checkbox(&mut edited.read_only, "Read only");
                let kind = spec.as_ref().and_then(|spec| {
                    let path = edited.bound_path()?;
                    Some(spec.get(spec.resolve(&path)?.ty).kind.clone())
                });
                if let Some(kind) = kind
                    && format_properties(
                        ui,
                        &kind,
                        &mut edited.thousands,
                        &mut edited.decimals,
                        &mut edited.date_format,
                        "field",
                    )
                {
                    merge = Some(key("date_format"));
                }
            }
            ControlKind::Group => {
                ui.horizontal(|ui| {
                    ui.label("Columns");
                    let mut columns = edited.columns.unwrap_or(1);
                    let response = ui.add(egui::DragValue::new(&mut columns).range(1..=6));
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::SpinButton,
                            true,
                            "Columns of the group",
                        )
                    });
                    edited.columns = (columns > 1).then_some(columns);
                });
            }
            ControlKind::Repeater => {
                ui.horizontal(|ui| {
                    ui.label("Shown as");
                    let mut mode = edited.mode;
                    let text = mode.map_or("Automatic", RepeaterMode::label);
                    let response = egui::ComboBox::from_id_salt("property_mode")
                        .selected_text(text)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut mode, None, "Automatic");
                            ui.selectable_value(
                                &mut mode,
                                Some(RepeaterMode::Table),
                                RepeaterMode::Table.label(),
                            );
                            ui.selectable_value(
                                &mut mode,
                                Some(RepeaterMode::Stacked),
                                RepeaterMode::Stacked.label(),
                            );
                        })
                        .response;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::ComboBox,
                            true,
                            "How the array is shown",
                        )
                    });
                    edited.mode = mode;
                });
                column_edit = self.column_list(ui, design, at);
                font_properties(
                    ui,
                    &mut edited.title_font,
                    &mut edited.title_font_size,
                    "Title ",
                    "table title",
                );
                font_properties(
                    ui,
                    &mut edited.header_font,
                    &mut edited.header_font_size,
                    "Column title ",
                    "column titles",
                );
            }
            ControlKind::Label => {
                if option_text(ui, "Text", &mut edited.text, "", true) {
                    merge = Some(key("text"));
                }
            }
            ControlKind::Button => {
                let names = self.calc().procedure_names();
                procedure_choice(ui, &mut edited.procedure, &names);
                if names.is_empty() {
                    ui.weak(
                        "No module has a function yet. Write one, def name(form, ctx):, \
                         in the code editor (Alt+F11).",
                    );
                } else {
                    ui.weak("A function of a module, called as name(form, ctx).");
                }
            }
            ControlKind::Separator => {}
        }
        if kind != ControlKind::Separator {
            font_properties(ui, &mut edited.font, &mut edited.font_size, "", "control");
        }
        let coloured = kind != ControlKind::Separator;
        let fields = [
            (ExprKind::Visible, &mut edited.visible_when, true),
            (ExprKind::Enabled, &mut edited.enabled_when, true),
            (ExprKind::TextColour, &mut edited.text_colour, coloured),
            (ExprKind::FillColour, &mut edited.fill_colour, coloured),
        ];
        for (expr, value, shown) in fields {
            if !shown {
                continue;
            }
            if option_text(ui, expr_label(expr), value, expr_hint(expr), false) {
                merge = Some(key(expr_property(expr)));
            }
            self.expression_error(ui, at, None, expr);
        }
        ui.weak(
            "Python expressions, run where the control is bound. A colour is a name, \
             \"#rrggbb\" or (r, g, b); None is the usual colour.",
        );

        if kind == ControlKind::Field {
            self.formula_editor(ui, &edited);
        }

        // Where the control is.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let index = at.last().copied().unwrap_or(0);
            let siblings = sibling_count(design, at);
            let above_group = index > 0
                && at.split_last().is_some_and(|(_, parent)| {
                    let mut above = parent.to_vec();
                    above.push(index - 1);
                    design
                        .get(&above)
                        .is_some_and(|c| c.kind == ControlKind::Group)
                });
            let mut act = |ui: &mut egui::Ui, label: &str, enabled: bool, action: ControlAction| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    self.apply_control(action);
                }
            };
            act(
                ui,
                "Move up",
                index > 0,
                ControlAction::Shift(at.clone(), true),
            );
            act(
                ui,
                "Move down",
                index + 1 < siblings,
                ControlAction::Shift(at.clone(), false),
            );
            act(
                ui,
                "Move into the group above",
                above_group,
                ControlAction::Nest(at.clone(), true),
            );
            act(
                ui,
                "Move out of the group",
                at.len() > 1,
                ControlAction::Nest(at.clone(), false),
            );
            act(
                ui,
                "Delete control",
                true,
                ControlAction::Delete(at.clone()),
            );
        });

        if &edited != control {
            let to = edited;
            let at = at.clone();
            self.edit_design_merging(merge, |d| {
                *d.get_mut(&at)? = to;
                Some(Some(at))
            });
        } else if let Some(action) = column_edit {
            self.apply_control(action);
        }
    }

    /// The columns of the table of a repeater, the row numbers among them, in
    /// their order: whether each is shown, buttons that move it, and one that
    /// opens its properties. Returns what was asked for.
    fn column_list(
        &self,
        ui: &mut egui::Ui,
        design: &FormDesign,
        at: &Address,
    ) -> Option<ControlAction> {
        let control = design.get(at)?;
        let names = self.column_names(design, at)?;
        let columns = control.arrange_columns(&names);
        let shown = columns
            .iter()
            .filter(|c| !c.hidden && !c.is_row_number())
            .count();
        let mut action = None;
        ui.label("Columns of the table");
        let last = columns.len().saturating_sub(1);
        egui::Grid::new("table_columns")
            .num_columns(3)
            .show(ui, |ui| {
                for (i, column) in columns.iter().enumerate() {
                    let name = column.name.clone();
                    let title = if column.is_row_number() {
                        "# (row numbers)".to_owned()
                    } else {
                        name.clone()
                    };
                    let mut on = !column.hidden;
                    // The last column of the items that is shown stays shown.
                    let can = column.hidden || column.is_row_number() || shown > 1;
                    let response = ui.add_enabled(can, egui::Checkbox::new(&mut on, &title));
                    let show_name = format!("Show column {name}");
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::CheckBox, can, on, show_name.clone())
                    });
                    if response.changed() {
                        action = Some(ControlAction::Column(
                            at.clone(),
                            name.clone(),
                            ColumnEdit::Show(on),
                        ));
                    }
                    ui.horizontal(|ui| {
                        let up = format!("Move column {name} up");
                        if icons::button(ui, i > 0, icons::ARROW_UP, "^", &up).clicked() {
                            action = Some(ControlAction::Column(
                                at.clone(),
                                name.clone(),
                                ColumnEdit::Step(true),
                            ));
                        }
                        let down = format!("Move column {name} down");
                        if icons::button(ui, i < last, icons::ARROW_DOWN, "v", &down).clicked() {
                            action = Some(ControlAction::Column(
                                at.clone(),
                                name.clone(),
                                ColumnEdit::Step(false),
                            ));
                        }
                    });
                    let open = ui.button("Properties");
                    let open_name = format!("Properties of column {name}");
                    open.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, open_name.clone())
                    });
                    if open.clicked() {
                        action = Some(ControlAction::SelectColumn(at.clone(), name.clone()));
                    }
                    ui.end_row();
                }
            });
        ui.weak("A click on the title of a column on the form also selects it.");
        action
    }

    /// The property page of a column of a table.
    fn column_properties(
        &mut self,
        ui: &mut egui::Ui,
        at: &Address,
        control: &Control,
        column: ColumnDesign,
    ) {
        let mut edited = column.clone();
        let row_number = column.is_row_number();
        ui.strong(crate::form::column_label(&column.name, control));
        if ui.button("Back to the table").clicked() {
            self.select_control(Some(at.clone()));
            return;
        }
        let key = |property: &str| format!("{at:?}:{}:{property}", column.name);
        let mut merge = None;
        if option_text(ui, "Label", &mut edited.label, &column.name, false) {
            merge = Some(key("label"));
        }
        if option_text(ui, "Help", &mut edited.help, "", true) {
            merge = Some(key("help"));
        }
        ui.horizontal(|ui| {
            ui.label("Width");
            // 0 is automatic; anything else is at least the narrowest width.
            let mut width = edited.width.unwrap_or(0);
            let response = ui.add(
                egui::DragValue::new(&mut width)
                    .range(0..=*design::COLUMN_WIDTHS.end())
                    .custom_formatter(|v, _| {
                        if v < 1.0 {
                            "Automatic".to_owned()
                        } else {
                            format!("{v:.0} pt")
                        }
                    }),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Width of the column")
            });
            edited.width =
                (width > 0).then(|| width.clamp(*design::COLUMN_WIDTHS.start(), u32::MAX));
        });
        alignment_choice(ui, &mut edited.title_align, "Title alignment", "the title");
        alignment_choice(ui, &mut edited.align, "Cell alignment", "the cells");
        if row_number {
            row_numbering(ui, &mut edited);
        }
        let kind = (!row_number)
            .then(|| {
                let spec = self.calc().spec(&self.sheet)?;
                let columns = crate::form::columns_of(spec, control)?;
                let ty = columns.iter().find(|c| c.name == column.name)?.ty;
                Some(spec.get(ty).kind.clone())
            })
            .flatten();
        if !row_number {
            ui.horizontal(|ui| {
                ui.label("Editor");
                let mut text = edited.editor.is_some_and(|e| e != Editor::Auto);
                let response = egui::ComboBox::from_id_salt("property_column_editor")
                    .selected_text(if text {
                        Editor::Text.label()
                    } else {
                        Editor::Auto.label()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut text, false, Editor::Auto.label());
                        ui.selectable_value(&mut text, true, Editor::Text.label());
                    })
                    .response;
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Editor of the column")
                });
                edited.editor = text.then_some(Editor::Text);
            });
            ui.checkbox(&mut edited.read_only, "Read only");
        }
        if let Some(kind) = &kind
            && format_properties(
                ui,
                kind,
                &mut edited.thousands,
                &mut edited.decimals,
                &mut edited.date_format,
                "column",
            )
        {
            merge = Some(key("date_format"));
        }
        font_properties(
            ui,
            &mut edited.font,
            &mut edited.font_size,
            "Cell ",
            "cells",
        );
        if !row_number {
            let fields = [
                (ExprKind::Visible, &mut edited.visible_when),
                (ExprKind::Enabled, &mut edited.enabled_when),
                (ExprKind::TextColour, &mut edited.text_colour),
                (ExprKind::FillColour, &mut edited.fill_colour),
            ];
            for (expr, value) in fields {
                if option_text(ui, expr_label(expr), value, expr_hint(expr), false) {
                    merge = Some(key(expr_property(expr)));
                }
                self.expression_error(ui, at, Some(&column.name), expr);
            }
            ui.weak(
                "Visible when runs once, where the table is bound, and hides the whole \
                 column. The others run for each row, with this as the cell and parent \
                 as the row.",
            );
        }
        ui.weak("Where the column is, and whether it is shown, are properties of the table.");
        if edited != column {
            self.set_column(merge, at, edited);
        }
    }

    /// Under the field of an expression of the control at `at`, or of its
    /// column `column`: why it failed, the first line of the message, with
    /// the full message on hover. For one that runs for each row, the first
    /// row that failed. Nothing before it has run, when it worked, or in a
    /// workbook that is not trusted.
    fn expression_error(
        &self,
        ui: &mut egui::Ui,
        at: &Address,
        column: Option<&str>,
        kind: ExprKind,
    ) {
        let calc = self.calc();
        let form = self.form_name();
        let sheet = &self.sheet;
        let failed = |state: Option<cliotree_calc::ExprState>| state?.result?.err();
        let error = match column {
            None => failed(calc.control_expression(sheet, &form, at, kind)).map(|e| (None, e)),
            Some(name) if kind == ExprKind::Visible => {
                failed(calc.column_expression(sheet, &form, at, name, None, kind))
                    .map(|e| (None, e))
            }
            Some(name) => (0..)
                .map_while(|row| {
                    calc.column_expression(sheet, &form, at, name, Some(row), kind)
                        .map(|state| (row, state))
                })
                .find_map(|(row, state)| Some((Some(row), failed(Some(state))?))),
        };
        let Some((row, message)) = error else {
            return;
        };
        let first = message.lines().next().unwrap_or_default();
        let text = match row {
            Some(row) => format!("Row [{row}]: {first}"),
            None => first.to_owned(),
        };
        let color = error_color(ui);
        let response = ui.add(egui::Label::new(
            egui::RichText::new(text).small().color(color),
        ));
        response.on_hover_text(message);
    }

    /// The formula of the node a field is bound to, edited here, since a form
    /// has no formula bar. It is the same entry as in the formula bar of the
    /// data tree, an undoable edit of the tab.
    fn formula_editor(&mut self, ui: &mut egui::Ui, control: &Control) {
        let Some(path) = control.bound_path().filter(|_| control.binds()) else {
            return;
        };
        let sheet = self.sheet.clone();
        let current = self.calc().formula_text(&sheet, &path).unwrap_or_default();
        let draft = self
            .form
            .layout
            .as_ref()
            .and_then(|l| l.formula.as_ref())
            .filter(|(p, _)| p == &path)
            .map(|(_, t)| t.clone());
        let mut text = draft.clone().unwrap_or_else(|| current.clone());
        ui.horizontal(|ui| {
            ui.label("Formula =");
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("no formula")
                    .desired_width(f32::INFINITY),
            );
            named(&response, "Formula of the bound node");
            if response.changed()
                && let Some(layout) = self.form.layout.as_mut()
            {
                layout.formula = Some((path.clone(), text.clone()));
            }
            if response.lost_focus() && text != current {
                self.commit_formula(&path, &text);
            }
        });
        if let Some(notice) = &self.notice {
            let color = error_color(ui);
            ui.colored_label(color, notice);
        }
    }

    /// Enters the formula of a node, or removes it when the text is empty.
    pub fn commit_formula(&mut self, path: &Path, text: &str) {
        let sheet = self.sheet.clone();
        let formula = format!("={}", text.trim());
        if !text.trim().is_empty()
            && self.ask_before_erasing(
                &sheet,
                crate::replace_inside::Target::Node(path.clone()),
                &formula,
                true,
            )
        {
            return;
        }
        let result = if text.trim().is_empty() {
            self.session
                .calc_mut()
                .remove_formula(&sheet, path)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            entry::enter(self.session.calc_mut(), &sheet, path, &formula, false)
                .map_err(|e| e.to_string())
        };
        match result {
            Ok(()) => {
                self.notice = None;
                self.clear_form_formula();
                self.session.tick();
            }
            Err(message) => self.notice = Some(message),
        }
    }

    /// Lets the formula box of the form designer show the node's formula
    /// again, instead of what was typed.
    pub(crate) fn clear_form_formula(&mut self) {
        if let Some(layout) = self.form.layout.as_mut() {
            layout.formula = None;
        }
    }

    fn unplaced_list(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        let Some(spec) = self.calc().spec(&self.sheet) else {
            return;
        };
        let unplaced = design.unplaced(spec);
        egui::CollapsingHeader::new(format!("Fields not on the form ({})", unplaced.len()))
            .id_salt("unplaced")
            .default_open(true)
            .show(ui, |ui| {
                if unplaced.is_empty() {
                    ui.weak("Every field is on the form.");
                }
                for path in &unplaced {
                    ui.horizontal(|ui| {
                        ui.label(path.to_string());
                        let response = ui.button("Add");
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Add {path} to the form"),
                            )
                        });
                        if response.clicked() {
                            self.add_field_to_form(path);
                        }
                    });
                }
            });
    }

    fn broken_list(&mut self, ui: &mut egui::Ui, design: &FormDesign) {
        let Some(spec) = self.calc().spec(&self.sheet) else {
            return;
        };
        let broken = design.broken(spec);
        egui::CollapsingHeader::new(format!("Broken controls ({})", broken.len()))
            .id_salt("broken")
            .default_open(true)
            .show(ui, |ui| {
                if broken.is_empty() {
                    ui.weak("No control is broken.");
                }
                for (at, problem) in &broken {
                    let name = design.get(at).map(control_name).unwrap_or_default();
                    ui.label(format!("{name}: {problem}"));
                    ui.horizontal(|ui| {
                        let rebind = ui.button("Re-bind");
                        rebind.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Re-bind {name}"),
                            )
                        });
                        if rebind.clicked() {
                            self.select_control(Some(at.clone()));
                        }
                        let remove = ui.button("Remove");
                        remove.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                format!("Remove {name}"),
                            )
                        });
                        if remove.clicked() {
                            self.apply_control(ControlAction::Delete(at.clone()));
                        }
                    });
                }
            });
    }
}

/// How many controls are in the list `at` is in.
fn sibling_count(design: &FormDesign, at: &[usize]) -> usize {
    match at.split_last() {
        Some((_, parent)) if !parent.is_empty() => {
            design.get(parent).map_or(0, |c| c.children.len())
        }
        Some(_) => design.controls.len(),
        None => 0,
    }
}

/// The sizes the property page offers, in points.
const FONT_SIZE_CHOICES: [u32; 16] = [8, 9, 10, 11, 12, 13, 14, 16, 18, 20, 24, 28, 32, 36, 48, 72];

/// The font of a control or a column: a typeface and a size, each *Inherited*
/// (from the group or table it is in, or the interface font) unless chosen.
/// `what` names the two for a screen reader: "Font of the control".
fn font_properties(
    ui: &mut egui::Ui,
    font: &mut Option<cliotree_core::form::FontFamily>,
    font_size: &mut Option<u32>,
    prefix: &str,
    what: &str,
) {
    use crate::style::{Typeface, family, typeface_label};
    let (font_label, size_label) = if prefix.is_empty() {
        ("Font".to_owned(), "Size".to_owned())
    } else {
        (format!("{prefix}font"), format!("{prefix}size"))
    };
    ui.horizontal(|ui| {
        ui.label(font_label);
        let mut chosen = *font;
        let response = egui::ComboBox::from_id_salt(("property_font", what))
            .selected_text(chosen.map_or_else(|| "Inherited".to_owned(), typeface_label))
            .width(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "Inherited");
                for typeface in Typeface::ALL {
                    let text =
                        egui::RichText::new(typeface_label(typeface)).family(family(typeface));
                    ui.selectable_value(&mut chosen, Some(typeface), text);
                }
            })
            .response;
        let name = format!("Font of the {what}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        *font = chosen;
    });
    ui.horizontal(|ui| {
        ui.label(size_label);
        let mut size = *font_size;
        let response = egui::ComboBox::from_id_salt(("property_font_size", what))
            .selected_text(size.map_or_else(|| "Inherited".to_owned(), |s| format!("{s} pt")))
            // Tall enough for every size, with no scrolling.
            .height(480.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut size, None, "Inherited");
                for choice in FONT_SIZE_CHOICES {
                    ui.selectable_value(&mut size, Some(choice), format!("{choice} pt"));
                }
            })
            .response;
        let name = format!("Font size of the {what}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        *font_size = size;
    });
}

/// Where the text of a column's title or cells sits: automatic, left, centre
/// or right. `what` names it for a screen reader: "Alignment of the title".
fn alignment_choice(ui: &mut egui::Ui, align: &mut Option<Alignment>, label: &str, what: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut chosen = *align;
        let response = egui::ComboBox::from_id_salt(("property_align", what))
            .selected_text(chosen.map_or("Automatic", Alignment::label))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "Automatic");
                for a in Alignment::ALL {
                    ui.selectable_value(&mut chosen, Some(a), a.label());
                }
            })
            .response;
        let name = format!("Alignment of {what}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
        *align = chosen;
    });
}

/// How the row numbers are written, and the number of the first row. A start
/// that is the numbering's own is stored as nothing.
fn row_numbering(ui: &mut egui::Ui, edited: &mut ColumnDesign) {
    use cliotree_core::form::Numbering;
    let mut numbering = edited.numbering.unwrap_or(Numbering::Index);
    ui.horizontal(|ui| {
        ui.label("Numbering");
        let response = egui::ComboBox::from_id_salt("property_numbering")
            .selected_text(numbering.label())
            .show_ui(ui, |ui| {
                for n in Numbering::ALL {
                    ui.selectable_value(&mut numbering, n, n.label());
                }
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Numbering of the rows")
        });
    });
    let mut start = edited
        .start
        .unwrap_or_else(|| edited.numbering.unwrap_or(Numbering::Index).default_start());
    ui.horizontal(|ui| {
        ui.label("Start at");
        let response = ui.add(egui::DragValue::new(&mut start).speed(0.1));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Number of the first row")
        });
    });
    // A start that was not given follows the numbering; one that was is kept.
    let start = if edited.start.is_none()
        && start == edited.numbering.unwrap_or(Numbering::Index).default_start()
    {
        numbering.default_start()
    } else {
        start
    };
    edited.numbering = (numbering != Numbering::Index).then_some(numbering);
    edited.start = (start != numbering.default_start()).then_some(start);
    let first: Vec<String> = (0..3).map(|i| edited.row_label(i)).collect();
    ui.weak(format!("The rows are numbered {}, ...", first.join(", ")));
}

/// The format of a number (a thousands separator, decimal places) or of a
/// date (a pattern), for a field or column of that type; nothing for another
/// type. Returns whether the pattern was typed into.
fn format_properties(
    ui: &mut egui::Ui,
    kind: &Kind,
    thousands: &mut bool,
    decimals: &mut Option<u32>,
    date_format: &mut Option<String>,
    what: &str,
) -> bool {
    use cliotree_core::format::{DATE_PRESETS, MAX_DECIMALS, date, is_date, is_number};
    if is_number(kind) {
        let response = ui.checkbox(thousands, "Thousands separator");
        let name = format!("Thousands separator of the {what}");
        let on = *thousands;
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, &name));
        ui.horizontal(|ui| {
            ui.label("Decimal places");
            let mut chosen = *decimals;
            let response = egui::ComboBox::from_id_salt("property_decimals")
                .selected_text(chosen.map_or_else(|| "Automatic".to_owned(), |d| d.to_string()))
                .height(400.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, "Automatic");
                    for d in 0..=MAX_DECIMALS {
                        ui.selectable_value(&mut chosen, Some(d), d.to_string());
                    }
                })
                .response;
            let name = format!("Decimal places of the {what}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
            *decimals = chosen;
        });
        false
    } else if is_date(kind) {
        let sample = chrono::NaiveDate::from_ymd_opt(2026, 3, 7).expect("a valid date");
        ui.horizontal(|ui| {
            ui.label("Date format");
            let mut chosen = date_format.clone();
            let text = chosen.as_deref().map_or_else(
                || "Automatic (yyyy-mm-dd)".to_owned(),
                |p| format!("{p} ({})", date(sample, p)),
            );
            let response = egui::ComboBox::from_id_salt("property_date_format")
                .selected_text(text)
                .width(200.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, "Automatic (yyyy-mm-dd)");
                    for preset in DATE_PRESETS {
                        let label = format!("{preset} ({})", date(sample, preset));
                        ui.selectable_value(&mut chosen, Some(preset.to_owned()), label);
                    }
                })
                .response;
            let name = format!("Date format of the {what}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
            *date_format = chosen;
        });
        let typed = option_text(ui, "Pattern", date_format, "yyyy-mm-dd", false);
        ui.weak("d dd ddd dddd: the day; m mm mmm mmmm: the month; yy yyyy: the year.");
        typed
    } else {
        false
    }
}

/// A labelled text field for an optional property: empty is absent. Returns
/// whether it was typed into.
/// The name of an expression's field on the property page.
fn expr_label(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible => "Visible when",
        ExprKind::Enabled => "Enabled when",
        ExprKind::TextColour => "Text colour",
        ExprKind::FillColour => "Fill colour",
    }
}

fn expr_hint(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible | ExprKind::Enabled => "expression",
        ExprKind::TextColour | ExprKind::FillColour => "expression giving a colour",
    }
}

/// The property an expression is stored in, for merging the typing in its
/// field into one change of the design.
fn expr_property(kind: ExprKind) -> &'static str {
    match kind {
        ExprKind::Visible => "visible_when",
        ExprKind::Enabled => "enabled_when",
        ExprKind::TextColour => "text_colour",
        ExprKind::FillColour => "fill_colour",
    }
}

/// The procedure of a button, chosen from the functions of the modules. A
/// name that no module has any more stays chosen, marked, until another is
/// chosen, so that fixing the module brings the button back.
fn procedure_choice(ui: &mut egui::Ui, procedure: &mut Option<String>, names: &[String]) {
    let missing = procedure
        .as_ref()
        .filter(|name| {
            // `module.name`, written by hand, runs too.
            let function = name.rsplit_once('.').map_or(name.as_str(), |(_, f)| f);
            !names.iter().any(|n| n == function)
        })
        .cloned();
    let shown = match (&*procedure, &missing) {
        (_, Some(name)) => format!("{name} (not in any module)"),
        (Some(name), None) => name.clone(),
        (None, _) => "(none)".to_owned(),
    };
    ui.horizontal(|ui| {
        ui.label("Procedure");
        let response = egui::ComboBox::from_id_salt("property_procedure")
            .selected_text(shown)
            .width(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(procedure, None, "(none)");
                if let Some(name) = &missing {
                    ui.selectable_value(
                        procedure,
                        Some(name.clone()),
                        format!("{name} (not in any module)"),
                    );
                }
                for name in names {
                    ui.selectable_value(procedure, Some(name.clone()), name);
                }
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Procedure of the button")
        });
    });
}

fn option_text(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<String>,
    hint: &str,
    multiline: bool,
) -> bool {
    let mut text = value.clone().unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let edit = if multiline {
            egui::TextEdit::multiline(&mut text).desired_rows(2)
        } else {
            egui::TextEdit::singleline(&mut text)
        };
        let response = ui.add(edit.hint_text(hint).desired_width(f32::INFINITY));
        let name = label.to_owned();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &name));
        changed = response.changed();
    });
    if changed {
        *value = (!text.is_empty()).then_some(text);
    }
    changed
}
