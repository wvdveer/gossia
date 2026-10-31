//! Adding and renaming the entries of a map in the data tree (DESIGN.md,
//! "Maps"): *Add entry...* on a map's row and *Rename key...* on an entry's
//! row, each asking for the key in a small dialog.

use cliotree_core::{Kind, Node, Path, Segment, Value};

use crate::app::ClioApp;
use crate::tab_rename::select_all;
use crate::view::{error_color, named};

/// The dialog of *Add entry...* or *Rename key...*, while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDialog {
    pub sheet: String,
    /// The map.
    pub map: Path,
    /// The key being renamed; None when adding an entry.
    pub from: Option<String>,
    /// What is typed.
    pub key: String,
    /// Why the last attempt was refused.
    pub error: Option<String>,
    focused: bool,
}

impl ClioApp {
    /// Whether the node at `path` of the tab shown is a map.
    pub fn is_map(&self, path: &Path) -> bool {
        let Some(spec) = self.calc().spec(&self.sheet) else {
            return false;
        };
        spec.resolve(path)
            .is_some_and(|r| matches!(spec.get(r.ty).kind, Kind::Map { .. }))
    }

    /// Opens *Add entry...* for the map at `map`.
    pub fn begin_add_entry(&mut self, map: &Path) {
        self.key_dialog = Some(KeyDialog {
            sheet: self.sheet.clone(),
            map: map.clone(),
            from: None,
            key: String::new(),
            error: None,
            focused: false,
        });
    }

    /// Opens *Rename key...* for the entry at `entry`.
    pub fn begin_rename_key(&mut self, entry: &Path) {
        let (Some(map), Some(Segment::Key(key))) = (entry.parent(), entry.segments().last()) else {
            return;
        };
        self.key_dialog = Some(KeyDialog {
            sheet: self.sheet.clone(),
            map,
            from: Some(key.clone()),
            key: key.clone(),
            error: None,
            focused: false,
        });
    }

    /// Why `key` cannot be the key of a new or renamed entry, or None.
    pub fn key_problem(&self, map: &Path, from: Option<&str>, key: &str) -> Option<String> {
        if key.is_empty() {
            return Some("Type the key.".to_owned());
        }
        if Some(key) == from {
            return None;
        }
        let exists = self
            .calc()
            .data(&self.sheet)
            .and_then(|d| d.get(&map.key(key)))
            .is_some();
        exists.then(|| format!("{} already has an entry {key}.", self.name_of(map)))
    }

    /// Adds the entry `key` to the map at `map`, holding what a new value of
    /// the map's type holds. A map that is absent is made. One entry in the
    /// history.
    pub fn add_map_entry(&mut self, map: &Path, key: &str) -> Result<(), String> {
        if let Some(why) = self.key_problem(map, None, key) {
            return Err(why);
        }
        let value = self.calc().spec(&self.sheet).and_then(|spec| {
            match &spec.get(spec.resolve(map)?.ty).kind {
                Kind::Map { value } => Some(ClioApp::default_value(spec, *value)),
                _ => None,
            }
        });
        let value = value.ok_or_else(|| format!("{} is not a map", self.name_of(map)))?;
        let present = self
            .calc()
            .data(&self.sheet)
            .is_some_and(|d| d.get(map).is_some());
        let sheet = self.sheet.clone();
        let (path, value) = if present {
            (map.key(key), value)
        } else {
            let fields = std::iter::once((key.to_owned(), Node::new(value))).collect();
            (map.clone(), Value::Object(fields))
        };
        self.session
            .calc_mut()
            .set_value(&sheet, &path, value)
            .map_err(|e| e.to_string())?;
        self.set_open(map, true);
        self.notice = None;
        self.session.tick();
        Ok(())
    }

    /// Renames the entry `from` of the map at `map` to `to`; its formulas go
    /// with it. One entry in the history.
    pub fn rename_map_key(&mut self, map: &Path, from: &str, to: &str) -> Result<(), String> {
        if let Some(why) = self.key_problem(map, Some(from), to) {
            return Err(why);
        }
        let sheet = self.sheet.clone();
        self.session
            .calc_mut()
            .rename_key(&sheet, map, from, to)
            .map_err(|e| e.to_string())?;
        // The selection follows the entry.
        if self.selection.as_ref() == Some(&map.key(from)) {
            self.select(&map.key(to));
        }
        self.notice = None;
        self.session.tick();
        Ok(())
    }

    /// The dialog of *Add entry...* or *Rename key...*, when it is open.
    pub(crate) fn key_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.key_dialog.take() else {
            return;
        };
        // A dialog for another tab closes when that tab is no longer shown.
        if dialog.sheet != self.sheet {
            return;
        }
        let key = dialog.key.trim().to_owned();
        let problem = self.key_problem(&dialog.map, dialog.from.as_deref(), &key);
        let title = match &dialog.from {
            Some(from) => format!("Rename key {from}"),
            None => format!("Add entry to {}", self.name_of(&dialog.map)),
        };
        let (mut apply, mut cancel, mut open) = (false, false, true);
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(if dialog.from.is_some() {
                    "New key"
                } else {
                    "Key"
                });
                let response = ui.text_edit_singleline(&mut dialog.key);
                named(&response, "Map key");
                if !dialog.focused {
                    response.request_focus();
                    select_all(ui, &response, &dialog.key);
                    dialog.focused = true;
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
                if let Some(why) = problem.as_ref().or(dialog.error.as_ref()) {
                    ui.colored_label(error_color(ui), why);
                }
                ui.horizontal(|ui| {
                    let label = if dialog.from.is_some() {
                        "Rename"
                    } else {
                        "Add"
                    };
                    apply |= ui
                        .add_enabled(problem.is_none(), egui::Button::new(label))
                        .clicked();
                    cancel |= ui.button("Cancel").clicked();
                });
            });
        if apply && problem.is_none() {
            let result = match dialog.from.clone() {
                Some(from) => self.rename_map_key(&dialog.map, &from, &key),
                None => self.add_map_entry(&dialog.map, &key),
            };
            if let Err(why) = result {
                dialog.error = Some(why);
                self.key_dialog = Some(dialog);
            }
        } else if open && !cancel {
            self.key_dialog = Some(dialog);
        }
    }
}
