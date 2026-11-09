//! Changing the schema from a data tab: the commands on fields, the dialog
//! state, and what the window keeps in step (DESIGN.md, "Editing the schema
//! from a data tab"). The calculation layer does the change, as one entry;
//! this module decides when it may be asked for, asks for it, and moves the
//! selection, the open nodes and the view state of tables to the new names.
//! Nothing here draws.

use gossia_calc::FieldKind;
use gossia_core::{Kind, Path, Segment};
use gossia_i18n::tr;

use crate::app::GossiaApp;
use crate::table::item_type;

/// What the user asked a field command to do.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaCommand {
    Add {
        name: String,
        kind: FieldKind,
        required: bool,
        /// On an XML tab: an attribute rather than an element. Its name is
        /// typed without the `@`.
        attribute: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        name: String,
    },
    /// All the fields, in the order they are to have.
    Reorder {
        order: Vec<String>,
    },
}

/// A command waiting for the user's word: a name to type, or a confirmation.
#[derive(Debug, Clone)]
pub struct SchemaDialog {
    /// Any object of the type that is changed (an item of an array will do).
    pub object: Path,
    pub command: SchemaCommand,
    /// Other places that use the type, which change as well.
    pub also_used_at: Vec<String>,
    /// For a delete: what will be removed.
    pub values: usize,
    pub formulas: usize,
    pub error: Option<String>,
    /// The schema tab is an XSD, so a new field is an element or an attribute.
    pub xml: bool,
}

impl SchemaCommand {
    /// The name the field is to have, as the calculation layer writes it: an
    /// attribute's starts with `@`, whether or not it was typed.
    pub fn new_name(&self) -> Option<String> {
        let at = |attribute: bool, name: &str| {
            let name = name.trim();
            if attribute && !name.starts_with('@') {
                format!("@{name}")
            } else {
                name.to_owned()
            }
        };
        match self {
            SchemaCommand::Add {
                name, attribute, ..
            } => Some(at(*attribute, name)),
            SchemaCommand::Rename { from, to } => Some(at(from.starts_with('@'), to)),
            _ => None,
        }
    }
}

impl GossiaApp {
    // -------------------------------------------------------- the switch

    /// Switches auto-update of the schema tab on or off.
    pub fn set_auto_update(&mut self, schema_sheet: &str, on: bool) {
        match self.session.calc_mut().set_auto_update(schema_sheet, on) {
            Ok(()) => self.notice = None,
            Err(e) => self.notice = Some(e.to_string()),
        }
    }

    // ------------------------------------------------------------ asking

    /// Whether a command on the fields of the object at `object` may be given
    /// now, and if not, why (for the tooltip of the menu item).
    pub fn schema_edit_allowed(&self, object: &Path) -> Result<(), String> {
        let places = self
            .calc()
            .field_places(&self.sheet, object)
            .map_err(|e| e.to_string())?;
        if let Some(editing) = &self.editing
            && editing.sheet == places.schema_sheet
        {
            return Err(tr!(
                "finish or cancel the edit in progress on {schema_sheet} first",
                schema_sheet = places.schema_sheet
            ));
        }
        Ok(())
    }

    /// The record type that a row stands for: the object itself, or the items
    /// of an array of objects. A path to any one of them will do.
    pub fn object_of_row(&self, path: &Path) -> Option<Path> {
        let spec = self.calc().spec(&self.sheet)?;
        match &spec.get(spec.resolve(path)?.ty).kind {
            Kind::Object { .. } => Some(path.clone()),
            Kind::Array { .. } => item_type(spec, path).map(|_| path.index(0)),
            _ => None,
        }
    }

    /// The record and the field name when `path` is a field of a record.
    pub fn field_of(&self, path: &Path) -> Option<(Path, String)> {
        let spec = self.calc().spec(&self.sheet)?;
        let (Segment::Key(name), parent) = path.segments().split_last()? else {
            return None;
        };
        let parent = Path::from(parent.to_vec());
        match &spec.get(spec.resolve(&parent)?.ty).kind {
            Kind::Object { fields } | Kind::Choice { fields }
                if fields.iter().any(|f| &f.name == name) =>
            {
                Some((parent, name.clone()))
            }
            _ => None,
        }
    }

    /// The record a table column's field belongs to, and the field's name.
    pub(crate) fn column_owner(
        &self,
        array: &Path,
        column: &crate::table::Column,
    ) -> Option<(Path, String)> {
        let (name, parents) = column.fields.split_last()?;
        let object = parents
            .iter()
            .fold(array.index(0), |path, field| path.key(field));
        Some((object, name.clone()))
    }

    /// Starts a command. One that needs a name or a confirmation opens its
    /// dialog; moving fields about asks only when the type is shared.
    pub fn start_schema_command(&mut self, object: &Path, command: SchemaCommand) {
        if let Err(why) = self.schema_edit_allowed(object) {
            self.notice = Some(why);
            return;
        }
        let calc = self.calc();
        let Ok(places) = calc.field_places(&self.sheet, object) else {
            return;
        };
        let (values, formulas) = match &command {
            SchemaCommand::Delete { name } => calc
                .delete_field_impact(&self.sheet, object, name)
                .unwrap_or_default(),
            _ => (0, 0),
        };
        let asks =
            !matches!(command, SchemaCommand::Reorder { .. }) || !places.also_used_at.is_empty();
        let dialog = SchemaDialog {
            object: object.clone(),
            command,
            also_used_at: places.also_used_at,
            values,
            formulas,
            error: None,
            xml: places.xsd,
        };
        self.schema_dialog = Some(dialog);
        if !asks {
            self.apply_schema_dialog();
        }
    }

    /// Moves `moved` to where `target` is among the fields of the record.
    pub fn move_field(&mut self, object: &Path, moved: &str, target: &str) {
        let Ok(places) = self.calc().field_places(&self.sheet, object) else {
            return;
        };
        let mut order = places.fields;
        let (Some(from), Some(to)) = (
            order.iter().position(|f| f == moved),
            order.iter().position(|f| f == target),
        ) else {
            return;
        };
        if from == to {
            return;
        }
        let field = order.remove(from);
        order.insert(to, field);
        self.start_schema_command(object, SchemaCommand::Reorder { order });
    }

    // ----------------------------------------------------------- running

    /// Does what the open dialog says. If the calculation layer refuses, the
    /// dialog stays open with the reason.
    pub fn apply_schema_dialog(&mut self) {
        let Some(mut dialog) = self.schema_dialog.take() else {
            return;
        };
        match self.run_schema_command(&dialog.object, &dialog.command) {
            Ok(()) => self.notice = None,
            Err(message) => {
                dialog.error = Some(message);
                self.schema_dialog = Some(dialog);
            }
        }
    }

    fn run_schema_command(&mut self, object: &Path, command: &SchemaCommand) -> Result<(), String> {
        self.schema_edit_allowed(object)?;
        let sheet = self.sheet.clone();
        let places = self
            .calc()
            .field_places(&sheet, object)
            .map_err(|e| e.to_string())?;
        // The names of the columns of every table before, so that what was
        // set on a column can follow it to its new name.
        let before = self.table_column_names();
        let new_name = command.new_name().unwrap_or_default();
        let calc = self.session.calc_mut();
        match command {
            SchemaCommand::Add { kind, required, .. } => {
                calc.add_field(&sheet, object, &new_name, *kind, *required)
            }
            SchemaCommand::Rename { from, .. } => {
                calc.rename_field(&sheet, object, from, &new_name)
            }
            SchemaCommand::Delete { name } => calc.delete_field(&sheet, object, name),
            SchemaCommand::Reorder { order } => calc.reorder_fields(&sheet, object, order),
        }
        .map_err(|e| e.to_string())?;
        match command {
            SchemaCommand::Rename { from, .. } => {
                for place in &places.objects {
                    self.move_view_of_node(&place.key(from), Some(&place.key(&new_name)));
                }
            }
            SchemaCommand::Delete { name } => {
                for place in &places.objects {
                    self.move_view_of_node(&place.key(name), None);
                }
            }
            _ => {}
        }
        self.follow_columns(&before);
        self.session.tick();
        Ok(())
    }

    /// The selection and the open nodes under `old` go to `new`, or to the
    /// node's parent if it is gone.
    fn move_view_of_node(&mut self, old: &Path, new: Option<&Path>) {
        let remap = |path: &Path| -> Option<Path> {
            if !path.segments().starts_with(old.segments()) {
                return Some(path.clone());
            }
            let new = new?;
            let mut moved = new.segments().to_vec();
            moved.extend_from_slice(&path.segments()[old.len()..]);
            Some(Path::from(moved))
        };
        if let Some(selection) = self.selection.clone() {
            self.selection = remap(&selection).or_else(|| old.parent());
        }
        if let Some(open) = self.expanded.get_mut(&self.sheet) {
            *open = open.iter().filter_map(remap).collect();
        }
        self.expansion_rev += 1;
        self.rows_key = None;
    }

    /// The column names of the table of each array of this tab that has view
    /// state.
    fn table_column_names(&self) -> Vec<(Path, Vec<String>)> {
        self.tables
            .views
            .keys()
            .filter(|(sheet, _)| sheet == &self.sheet)
            .map(|(_, array)| array.clone())
            .map(|array| {
                let names = self
                    .columns_of(&array)
                    .into_iter()
                    .map(|c| c.name)
                    .collect();
                (array, names)
            })
            .collect()
    }

    /// After a change of the fields: what was set on a renamed column is set on
    /// it under the new name, and what was set on a column that is gone is
    /// dropped.
    fn follow_columns(&mut self, before: &[(Path, Vec<String>)]) {
        let sheet = self.sheet.clone();
        for (array, old) in before {
            let new: Vec<String> = self.columns_of(array).into_iter().map(|c| c.name).collect();
            // A rename keeps the number and the order of the columns, so the
            // names that differ can be paired off by position.
            let renames: Vec<(&String, &String)> = if old.len() == new.len() {
                old.iter().zip(&new).filter(|(a, b)| a != b).collect()
            } else {
                Vec::new()
            };
            let Some(view) = self.tables.views.get_mut(&(sheet.clone(), array.clone())) else {
                continue;
            };
            for (a, b) in &renames {
                if view.hidden_columns.remove(*a) {
                    view.hidden_columns.insert((*b).clone());
                }
                if let Some(filter) = view.filters.remove(*a) {
                    view.filters.insert((*b).clone(), filter);
                }
            }
            view.hidden_columns.retain(|c| new.contains(c));
            view.filters.retain(|c, _| new.contains(c));
            if let Some((s, a, column, _)) = self.tables.last_sort.clone()
                && s == sheet
                && &a == array
            {
                if let Some((_, b)) = renames.iter().find(|(x, _)| **x == column) {
                    if let Some(last) = &mut self.tables.last_sort {
                        last.2 = (*b).clone();
                    }
                } else if !new.contains(&column) {
                    self.tables.last_sort = None;
                }
            }
        }
        self.tables.rev += 1;
    }
}
