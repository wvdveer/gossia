//! Tabs of COBOL records (DESIGN.md, "Import and export", "COBOL
//! copybooks"): the bytes of their one record, which layout of each group
//! of `REDEFINES` they show, which record of the copybook they hold, and
//! whether their file is a variable-length record.

use gossia_core::Value;
use gossia_core::copybook::{Layout, records};
use gossia_i18n::tr;

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
            .ok_or_else(|| {
                EditError::Path(tr!("{sheet} does not hold COBOL records", sheet = sheet))
            })
    }

    /// The tab's data as a `.blk` file: its record, in its copybook's
    /// layout, after a record descriptor word when the tab is a
    /// variable-length record. Refused, naming the field, for a value that
    /// cannot be written.
    pub fn records_bytes(&self, sheet: &str) -> Result<Vec<u8>, String> {
        let layout = self.layout(sheet).map_err(|e| e.to_string())?;
        let s = &self.sheets[sheet];
        let written = if s.variable {
            records::write_variable(&s.data, layout)
        } else {
            records::write(&s.data, layout)
        };
        written.map_err(|e| format!("tab {sheet}: {e}"))
    }

    /// Whether the tab of COBOL records is a variable-length record
    /// (`RECFM=VB`), whose file starts with a record descriptor word.
    pub fn is_variable(&self, sheet: &str) -> bool {
        self.sheets.get(sheet).is_some_and(|s| s.variable)
    }

    /// Writes the tab as a variable-length record, or as a fixed-length one.
    /// One entry in the history.
    pub fn set_variable(&mut self, sheet: &str, on: bool) -> Result<(), EditError> {
        self.layout(sheet)?;
        let before = self.is_variable(sheet);
        if before == on {
            return Ok(());
        }
        self.store_variable(sheet, on)?;
        self.history.record(
            if on {
                tr!("Write {sheet} as a variable-length record", sheet = sheet)
            } else {
                tr!("Write {sheet} as a fixed-length record", sheet = sheet)
            },
            vec![Op::Variable {
                sheet: sheet.to_owned(),
                before,
                after: on,
            }],
            None,
        );
        Ok(())
    }

    pub(super) fn store_variable(&mut self, sheet: &str, on: bool) -> Result<(), EditError> {
        self.sheet_mut(sheet)?.variable = on;
        self.revision += 1;
        Ok(())
    }

    /// Why the tab cannot hold the copybook's record `record`, or None when
    /// it can: the record must be as long as the one it holds now.
    pub fn record_problem(&self, sheet: &str, record: &str) -> Option<String> {
        let layout = match self.layout(sheet) {
            Ok(layout) => layout,
            Err(e) => return Some(e.to_string()),
        };
        let schema = self.schema_of(sheet)?;
        match self.lowered_for(schema, Some(record)) {
            Err(e) => Some(e.to_string()),
            Ok(spec) => match spec.copybook() {
                Some(new) if new.length != layout.length => Some(format!(
                    "{record} has {} bytes, and {} has {}",
                    new.length, layout.record, layout.length
                )),
                Some(_) => None,
                None => Some(tr!("{schema} is not a copybook", schema = schema)),
            },
        }
    }

    /// Makes the tab hold another 01 record of its copybook, of the same
    /// length: its bytes are read with that record's layout, showing the
    /// first layout of each group of `REDEFINES`. Its formulas are deleted
    /// (the window asks first). One entry in the history.
    pub fn set_record(&mut self, sheet: &str, record: &str) -> Result<(), EditError> {
        if let Some(why) = self.record_problem(sheet, record) {
            return Err(EditError::Path(why));
        }
        let old = self.layout(sheet)?.clone();
        if old.record.eq_ignore_ascii_case(record) {
            return Ok(());
        }
        let schema = self.schema_of(sheet).unwrap_or_default().to_owned();
        let spec = self.lowered_for(&schema, Some(record))?;
        let new = spec.copybook().expect("checked above").clone();
        let data = self.sheets[sheet].data.clone();
        let bytes = records::write_record(&data, &old, &Path::root())
            .map_err(|e| EditError::Path(tr!("the record cannot be converted: {e}", e = e)))?;
        let read = records::read_record(&bytes, &new, &[]);
        let mut ops = Vec::new();
        if let Some(text) = self.formula_text(sheet, &Path::root()) {
            ops.extend(self.release_column(sheet, &Path::root()));
            if self.drop_formula(sheet, &Path::root()) {
                ops.push(Op::Formula {
                    sheet: sheet.to_owned(),
                    path: Path::root(),
                    before: Some(text),
                    after: None,
                    value: None,
                });
            }
        }
        ops.extend(self.clear_formulas_below(sheet, &Path::root()));
        let choices = self.sheets[sheet].redefines.clone();
        for (set, member) in choices.iter().enumerate().rev() {
            if *member != 0 {
                self.store_redefine(sheet, set, 0)?;
                ops.push(Op::Redefine {
                    sheet: sheet.to_owned(),
                    set,
                    before: *member,
                    after: 0,
                });
            }
        }
        let before = self.sheets[sheet].record.clone();
        let after = Some(new.record.clone());
        self.store_record(sheet, after.clone())?;
        ops.push(Op::Record {
            sheet: sheet.to_owned(),
            before,
            after,
        });
        self.sheet_mut(sheet)?
            .data
            .replace(&Path::root(), read.clone())
            .map_err(|e| EditError::Path(e.to_string()))?;
        self.changed(sheet, &Path::root());
        ops.push(Op::Replace {
            sheet: sheet.to_owned(),
            path: Path::root(),
            before: data,
            after: read,
        });
        self.history.record(
            tr!(
                "Hold {record} in {sheet}",
                record = new.record,
                sheet = sheet
            ),
            ops,
            None,
        );
        Ok(())
    }

    /// Sets the record a tab holds and gives it that record's type.
    pub(super) fn store_record(
        &mut self,
        sheet: &str,
        record: Option<String>,
    ) -> Result<(), EditError> {
        let schema = self
            .schema_of(sheet)
            .ok_or_else(|| {
                EditError::Path(tr!("{sheet} is not bound to a copybook", sheet = sheet))
            })?
            .to_owned();
        let spec = self.lowered_for(&schema, record.as_deref())?;
        let s = self.sheet_mut(sheet)?;
        s.record = record;
        s.spec = Arc::new(spec);
        self.changed(sheet, &Path::root());
        self.revision += 1;
        Ok(())
    }

    /// Reads a `.blk` file in the layout of the tab's copybook, showing the
    /// layouts it shows. The tab is not changed.
    pub fn read_records(
        &self,
        sheet: &str,
        bytes: &[u8],
    ) -> Result<gossia_core::json::Parsed, String> {
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
                filler: set.keys.iter().map(Vec::is_empty).collect(),
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
        let mut places = Vec::new();
        for key in group.keys.get(shown).into_iter().flatten() {
            find_key(&s.data, &Path::root(), key, &mut places);
        }
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
        let group = layout.redefines.get(set).ok_or_else(|| {
            EditError::Path(tr!("{sheet} has no such group of layouts", sheet = sheet))
        })?;
        let new_keys = match group.keys.get(member) {
            Some(keys) if !keys.is_empty() => keys.clone(),
            Some(_) => {
                return Err(EditError::Path(
                    tr!("that layout is FILLER, which has no fields to show").into(),
                ));
            }
            None => return Err(EditError::Path(tr!("there is no such layout").into())),
        };
        let mut choices = self.sheets[sheet].redefines.clone();
        if choices.len() <= set {
            choices.resize(set + 1, 0);
        }
        let before = choices[set];
        if before == member {
            return Ok(());
        }
        let old_keys = group.keys.get(before).cloned().unwrap_or_default();
        choices[set] = member;
        // The new record, worked out before anything changes.
        let record = self.sheets[sheet].data.clone();
        let bytes = records::write_record(&record, &layout, &Path::root())
            .map_err(|e| EditError::Path(tr!("the record cannot be converted: {e}", e = e)))?;
        let read = records::read_record(&bytes, &layout, &choices);
        let after = if old_keys.is_empty() {
            read
        } else {
            swap(&record, &read, &old_keys, &new_keys)
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
            tr!(
                "Show {name} in {sheet}",
                name = group.names[member],
                sheet = sheet
            ),
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

/// The record `old` with the fields `old_keys` put in place of by
/// `new_keys` as `read` has them, wherever `read` has them in the same
/// place; the rest of `old` is kept as it is.
fn swap(old: &Node, read: &Node, old_keys: &[String], new_keys: &[String]) -> Node {
    let mut node = old.clone();
    match (node.value_mut(), read.value()) {
        (Value::Object(fields), Value::Object(read_fields)) => {
            let position = old_keys.iter().filter_map(|k| fields.get_index_of(k)).min();
            if let Some(position) = position
                && new_keys.iter().any(|k| read_fields.contains_key(k))
            {
                for key in old_keys {
                    fields.shift_remove(key);
                }
                let mut at = position.min(fields.len());
                for key in new_keys {
                    if let Some(new) = read_fields.get(key) {
                        fields.shift_insert(at, key.clone(), new.clone());
                        at += 1;
                    }
                }
            } else {
                for (k, child) in fields.iter_mut() {
                    if let Some(r) = read_fields.get(k) {
                        *child = swap(child, r, old_keys, new_keys);
                    }
                }
            }
        }
        (Value::Array(items), Value::Array(read_items)) => {
            for (item, r) in items.iter_mut().zip(read_items) {
                *item = swap(item, r, old_keys, new_keys);
            }
        }
        _ => {}
    }
    node
}
