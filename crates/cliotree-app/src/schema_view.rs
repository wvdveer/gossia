//! Drawing what belongs to editing the schema: the bar above a schema tab, the
//! menu items that change the fields from a data tab, and their dialog. What the
//! commands do is in `schema.rs`.

use cliotree_calc::FieldKind;
use cliotree_core::Path;

use crate::app::ClioApp;
use crate::schema::SchemaCommand;
use crate::view::{Action, error_color, named};

/// A column header being dragged to a new place among the fields.
#[derive(Debug, Clone)]
pub(crate) struct ColumnDrag {
    /// The record the column's field belongs to.
    pub object: Path,
    pub field: String,
}

impl ClioApp {
    /// The bar above a schema tab: whether the tab that uses it may change it,
    /// and which tab that is.
    pub(crate) fn schema_bar(&mut self, ui: &mut egui::Ui) {
        let sheet = self.sheet.clone();
        let users = self.calc().schema_users(&sheet);
        let mut on = self.calc().auto_update(&sheet);
        let blocker = if on {
            None
        } else {
            self.calc().auto_update_blocker(&sheet)
        };
        ui.horizontal(|ui| {
            let checkbox = ui.add_enabled(
                blocker.is_none(),
                egui::Checkbox::new(&mut on, "Auto-update"),
            );
            let checkbox = match &blocker {
                Some(why) => checkbox.on_disabled_hover_text(format!("Cannot be switched on: {why}")),
                None => checkbox.on_hover_text(
                    "Lets the one tab that uses this schema add, rename, delete and reorder                      its fields from its own grid",
                ),
            };
            if checkbox.changed() {
                self.set_auto_update(&sheet, on);
            }
            ui.separator();
            if users.is_empty() {
                ui.label("Used by: no tab");
            } else {
                ui.label(format!("Used by: {}", users.join(", ")));
            }
            if let Some(error) = self.calc().schema_error(&sheet) {
                ui.separator();
                ui.colored_label(error_color(ui), format!("Schema error: {error}"));
            }
        });
    }

    /// The items of a context menu that change the fields: adding one to the
    /// record `add_to`, and renaming or deleting the field `field` of a record.
    /// They are there on every tab that has a schema tab, and unavailable, with
    /// the reason as a tooltip, when the change cannot be made.
    pub(crate) fn field_menu_items(
        &self,
        ui: &mut egui::Ui,
        add_to: Option<&Path>,
        field: Option<&(Path, String)>,
        actions: &mut Vec<Action>,
    ) {
        if self.calc().schema_of(&self.sheet).is_none() {
            return;
        }
        let any = add_to.or(field.map(|(object, _)| object));
        let Some(any) = any else {
            return;
        };
        let reason = self.schema_edit_allowed(any).err();
        ui.separator();
        let mut item = |ui: &mut egui::Ui, label: &str, action: Action| {
            let response = ui.add_enabled(reason.is_none(), egui::Button::new(label));
            let response = match &reason {
                Some(why) => response.on_disabled_hover_text(why),
                None => response,
            };
            if response.clicked() {
                actions.push(action);
                ui.close();
            }
        };
        if let Some(object) = add_to {
            item(ui, "Add field...", Action::AddField(object.clone()));
        }
        if let Some((object, name)) = field {
            item(
                ui,
                "Rename field...",
                Action::RenameField(object.clone(), name.clone()),
            );
            item(
                ui,
                "Delete field...",
                Action::DeleteField(object.clone(), name.clone()),
            );
        }
    }

    /// *Force object definition*, on a field whose type is a record with no
    /// fields (or an array of them) and whose formula returned records with
    /// fields. Unavailable, with the reason, when the schema cannot be
    /// changed from here.
    pub(crate) fn force_type_item(
        &self,
        ui: &mut egui::Ui,
        path: &Path,
        actions: &mut Vec<Action>,
    ) {
        if !self.calc().force_type_offered(&self.sheet, path) {
            return;
        }
        let parent = path.parent().unwrap_or_default();
        let reason = self.schema_edit_allowed(&parent).err().or_else(|| {
            self.calc()
                .field_places(&self.sheet, &parent)
                .ok()
                .filter(|p| p.xsd)
                .map(|p| {
                    format!(
                        "the type of an XML element is defined on its XSD tab, {}",
                        p.schema_sheet
                    )
                })
        });
        ui.separator();
        let response = ui.add_enabled(
            reason.is_none(),
            egui::Button::new("Force object definition"),
        );
        let response = match &reason {
            Some(why) => response.on_disabled_hover_text(why),
            None => response
                .on_hover_text("Defines the fields of this node from what its formula returned"),
        };
        if response.clicked() {
            actions.push(Action::ForceType(path.clone()));
            ui.close();
        }
    }

    /// Does *Force object definition* on the node at `path`.
    pub(crate) fn force_type(&mut self, path: &Path) {
        let sheet = self.sheet.clone();
        match self.session.calc_mut().force_type(&sheet, path) {
            Ok(()) => {
                self.notice = None;
                self.session.tick();
                self.refresh_rows();
            }
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    /// The dialog of a field command, when one is waiting.
    pub(crate) fn schema_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.schema_dialog.take() else {
            return;
        };
        let title = match &dialog.command {
            SchemaCommand::Add { .. } => "Add field".to_owned(),
            SchemaCommand::Rename { from, .. } => format!("Rename field {from}"),
            SchemaCommand::Delete { name } => format!("Delete field {name}"),
            SchemaCommand::Reorder { .. } => "Reorder fields".to_owned(),
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        let schema = self
            .calc()
            .schema_of(&self.sheet)
            .unwrap_or_default()
            .to_owned();
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                match &mut dialog.command {
                    SchemaCommand::Add {
                        name,
                        kind,
                        required,
                        attribute,
                    } => {
                        if dialog.xml {
                            ui.horizontal(|ui| {
                                ui.radio_value(attribute, false, "Element");
                                ui.radio_value(attribute, true, "Attribute");
                            });
                        }
                        ui.label("Name");
                        let response = ui.text_edit_singleline(name);
                        named(&response, "Field name");
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            apply = true;
                        }
                        ui.horizontal(|ui| {
                            let label = ui.label("Type");
                            egui::ComboBox::from_id_salt("field type")
                                .selected_text(kind.label())
                                // Every type in sight, without scrolling.
                                .height(f32::INFINITY)
                                .show_ui(ui, |ui| {
                                    for k in FieldKind::ALL {
                                        ui.selectable_value(kind, k, k.label());
                                    }
                                })
                                .response
                                .labelled_by(label.id);
                        });
                        if let Some(hint) = kind.hint() {
                            ui.weak(hint);
                        }
                        ui.checkbox(required, "Required");
                    }
                    SchemaCommand::Rename { from, to } => {
                        ui.label(format!("New name for {from}"));
                        let response = ui.text_edit_singleline(to);
                        named(&response, "New field name");
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            apply = true;
                        }
                        ui.label(
                            "Formulas that read the field by its old name are not rewritten \
                             and will show #REF!.",
                        );
                    }
                    SchemaCommand::Delete { name } => {
                        ui.label(format!(
                            "Delete {name}? {} will be removed from the data{}.",
                            plural(dialog.values, "value"),
                            if dialog.formulas > 0 {
                                format!(", with {}", plural(dialog.formulas, "formula"))
                            } else {
                                String::new()
                            }
                        ));
                        ui.label("Formulas that read it will show #REF!.");
                    }
                    SchemaCommand::Reorder { order } => {
                        ui.label(format!("New order: {}", order.join(", ")));
                    }
                }
                if !dialog.also_used_at.is_empty() {
                    ui.separator();
                    ui.label(format!(
                        "This type is also used at {}. The change applies there too.",
                        dialog.also_used_at.join(", ")
                    ));
                }
                ui.label(format!("This changes the schema in {schema}."));
                if let Some(error) = &dialog.error {
                    ui.colored_label(error_color(ui), error);
                }
                ui.horizontal(|ui| {
                    let label = match &dialog.command {
                        SchemaCommand::Add { .. } => "Add",
                        SchemaCommand::Rename { .. } => "Rename",
                        SchemaCommand::Delete { .. } => "Delete",
                        SchemaCommand::Reorder { .. } => "Reorder",
                    };
                    apply |= ui.button(label).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if apply {
            self.schema_dialog = Some(dialog);
            self.apply_schema_dialog();
        } else if open && !cancel {
            self.schema_dialog = Some(dialog);
        }
    }

    /// Begins a command from a menu item.
    pub(crate) fn begin_field_command(&mut self, action: Action) {
        match action {
            Action::AddField(object) => self.start_schema_command(
                &object,
                SchemaCommand::Add {
                    name: String::new(),
                    kind: FieldKind::Text,
                    required: false,
                    attribute: false,
                },
            ),
            Action::RenameField(object, name) => self.start_schema_command(
                &object,
                SchemaCommand::Rename {
                    // An attribute's new name is typed without its `@`.
                    to: name.strip_prefix('@').unwrap_or(&name).to_owned(),
                    from: name,
                },
            ),
            Action::DeleteField(object, name) => {
                self.start_schema_command(&object, SchemaCommand::Delete { name });
            }
            Action::ReorderFields(object, moved, target) => {
                self.move_field(&object, &moved, &target);
            }
            _ => {}
        }
    }
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
