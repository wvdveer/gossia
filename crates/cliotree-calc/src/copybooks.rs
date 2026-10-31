//! Tabs of COBOL records (DESIGN.md, "Import and export", "COBOL
//! copybooks"): the bytes of their one record, and which layout of each
//! group of `REDEFINES` they show.

use cliotree_core::Value;
use cliotree_core::copybook::{Layout, records};

use super::*;

/// A group of layouts over the same bytes, as a tab shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedefineGroup {
    /// The COBOL names of the layouts, the redefined one first.
    pub names: Vec<String>,
    /// Which a `FILLER` is: it has no fields to show.
    pub filler: Vec<bool>,
    /// The one shown.
    pub shown: usize,
}

impl Calc {
    fn layout(&self, sheet: &str) -> Result<&Layout, EditError> {
        self.sheets
            .get(sheet)
            .ok_or_else(|| EditError::NoSheet(sheet.to_owned()))?
            .spec
            .copybook()
            .ok_or_else(|| EditError::Path(format!("{sheet} does not hold COBOL records")))
    }

    /// The tab's data as a `.blk` file: its record, in its copybook's layout.
    /// Refused, naming the field, for a value that cannot be written.
    pub fn records_bytes(&self, sheet: &str) -> Result<Vec<u8>, String> {
        let layout = self.layout(sheet).map_err(|e| e.to_string())?;
        let data = &self.sheets[sheet].data;
        records::write(data, layout).map_err(|e| format!("tab {sheet}: {e}"))
    }

    /// Reads a `.blk` file in the layout of the tab's copybook, showing the
    /// layouts it shows. The tab is not changed.
    pub fn read_records(
        &self,
        sheet: &str,
        bytes: &[u8],
    ) -> Result<cliotree_core::json::Parsed, String> {
        let layout = self.layout(sheet).map_err(|e| e.to_string())?;
        records::read(bytes, layout, &self.sheets[sheet].redefines).map_err(|e| e.to_string())
    }

    /// The groups of `REDEFINES` of a tab of COBOL records, with the layout
    /// each shows.
    pub fn redefine_groups(&self, sheet: &str) -> Vec<RedefineGroup> {
        let Ok(layout) = self.layout(sheet) else {
            return Vec::new();
        };
        let chosen = &self.sheets[sheet].redefines;
        layout
            .redefines
            .iter()
            .enumerate()
            .map(|(i, set)| RedefineGroup {
                names: set.names.clone(),
                filler: set.keys.iter().map(Option::is_none).collect(),
                shown: chosen.get(i).copied().unwrap_or(0),
            })
            .collect()
    }

    /// The layouts the tab shows, one for each group of `REDEFINES`.
    pub fn redefine_choices(&self, sheet: &str) -> Vec<usize> {
        self.sheets
            .get(sheet)
            .map(|s| s.redefines.clone())
            .unwrap_or_default()
    }

    /// Where the layout shown of a group is in the data: one place, or each
    /// item of a table it is in.
    fn redefine_places(&self, sheet: &str, set: usize) -> Vec<Path> {
        let (Ok(layout), Some(s)) = (self.layout(sheet), self.sheets.get(sheet)) else {
            return Vec::new();
        };
        let Some(group) = layout.redefines.get(set) else {
            return Vec::new();
        };
        let shown = s.redefines.get(set).copied().unwrap_or(0);
        let Some(Some(key)) = group.keys.get(shown) else {
            return Vec::new();
        };
        let mut places = Vec::new();
        find_key(&s.data, &Path::root(), key, &mut places);
        places
    }

    /// The formulas in the layout shown of a group of `REDEFINES`, which
    /// showing another deletes (the window asks first).
    pub fn redefine_formulas(&self, sheet: &str, set: usize) -> Vec<Path> {
        let Some(s) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for place in self.redefine_places(sheet, set) {
            if s.formulas.by_path.contains_key(&place) {
                found.push(place.clone());
            }
            found.extend(s.formulas.below(&place));
        }
        found
    }

    /// Shows another layout of a group of `REDEFINES`: the record's bytes for
    /// it are written from the layout shown and read with the new one. The
    /// formulas in the layout shown are deleted. One entry in the history.
    pub fn set_redefine(
        &mut self,
        sheet: &str,
        set: usize,
        member: usize,
    ) -> Result<(), EditError> {
        let layout = self.layout(sheet)?.clone();
        let group = layout
            .redefines
            .get(set)
            .ok_or_else(|| EditError::Path(format!("{sheet} has no such group of layouts")))?;
        let new_key = match group.keys.get(member) {
            Some(Some(key)) => key.clone(),
            Some(None) => {
                return Err(EditError::Path(
                    "that layout is FILLER, which has no fields to show".into(),
                ));
            }
            None => return Err(EditError::Path("there is no such layout".into())),
        };
        let mut choices = self.sheets[sheet].redefines.clone();
        if choices.len() <= set {
            choices.resize(set + 1, 0);
        }
        let before = choices[set];
        if before == member {
            return Ok(());
        }
        let old_key = group.keys[before].clone();
        choices[set] = member;
        // The new record, worked out before anything changes.
        let record = self.sheets[sheet].data.clone();
        let bytes = records::write_record(&record, &layout, &Path::root())
            .map_err(|e| EditError::Path(format!("the record cannot be converted: {e}")))?;
        let read = records::read_record(&bytes, &layout, &choices);
        let after = match &old_key {
            Some(old_key) => swap(&record, &read, old_key, &new_key),
            None => read,
        };
        let replaced = (after != record).then(|| (Path::root(), record.clone(), after));
        let mut ops = Vec::new();
        for place in self.redefine_places(sheet, set) {
            if let Some(text) = self.formula_text(sheet, &place) {
                ops.extend(self.release_column(sheet, &place));
                if self.drop_formula(sheet, &place) {
                    ops.push(Op::Formula {
                        sheet: sheet.to_owned(),
                        path: place.clone(),
                        before: Some(text),
                        after: None,
                        value: None,
                    });
                }
            }
            ops.extend(self.clear_formulas_below(sheet, &place));
        }
        if let Some((path, before, after)) = replaced {
            self.sheet_mut(sheet)?
                .data
                .replace(&path, after.clone())
                .map_err(|e| EditError::Path(e.to_string()))?;
            self.changed(sheet, &path);
            ops.push(Op::Replace {
                sheet: sheet.to_owned(),
                path,
                before,
                after,
            });
        }
        self.store_redefine(sheet, set, member)?;
        ops.push(Op::Redefine {
            sheet: sheet.to_owned(),
            set,
            before,
            after: member,
        });
        self.history.record(
            format!("Show {} in {sheet}", group.names[member]),
            ops,
            None,
        );
        Ok(())
    }

    pub(super) fn store_redefine(
        &mut self,
        sheet: &str,
        set: usize,
        member: usize,
    ) -> Result<(), EditError> {
        let choices = &mut self.sheet_mut(sheet)?.redefines;
        if choices.len() <= set {
            choices.resize(set + 1, 0);
        }
        choices[set] = member;
        while choices.last() == Some(&0) {
            choices.pop();
        }
        self.revision += 1;
        Ok(())
    }
}

/// The places of the field `key` in a tree.
fn find_key(node: &Node, path: &Path, key: &str, out: &mut Vec<Path>) {
    match node.value() {
        Value::Object(fields) => {
            for (k, child) in fields {
                let here = path.key(k.clone());
                if k == key {
                    out.push(here);
                } else {
                    find_key(child, &here, key, out);
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                find_key(item, &path.index(i), key, out);
            }
        }
        _ => {}
    }
}

/// The record `old` with the field `old_key` put in place of by `new_key`
/// as `read` has it, wherever `read` has it in the same place; the rest of
/// `old` is kept as it is.
fn swap(old: &Node, read: &Node, old_key: &str, new_key: &str) -> Node {
    let mut node = old.clone();
    match (node.value_mut(), read.value()) {
        (Value::Object(fields), Value::Object(read_fields)) => {
            if fields.contains_key(old_key)
                && let Some(new) = read_fields.get(new_key)
            {
                let position = fields.get_index_of(old_key).unwrap_or(fields.len());
                fields.shift_remove(old_key);
                fields.shift_insert(position, new_key.to_owned(), new.clone());
            } else {
                for (k, child) in fields.iter_mut() {
                    if let Some(r) = read_fields.get(k) {
                        *child = swap(child, r, old_key, new_key);
                    }
                }
            }
        }
        (Value::Array(items), Value::Array(read_items)) => {
            for (item, r) in items.iter_mut().zip(read_items) {
                *item = swap(item, r, old_key, new_key);
            }
        }
        _ => {}
    }
    node
}
