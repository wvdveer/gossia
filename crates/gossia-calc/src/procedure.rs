//! Applying the writes of a button procedure (DESIGN.md, "Button procedures").
//!
//! A procedure runs on the evaluation thread against a snapshot and makes a
//! list of writes. This is where the list is applied to the live workbook, on
//! the thread that owns it, all or nothing: every written value is checked
//! against the schema of its tab, a computed node is refused, and a node the
//! user edited after the procedure started refuses the lot. What is applied is
//! one entry in the history, and recalculation follows as it does after any
//! edit.

use super::*;
use crate::engine::{Write, WriteOp};
use crate::eval::ProcedureResult;
use gossia_core::validate::validate_at;
use gossia_i18n::tr;

impl Calc {
    /// Applies the writes of a procedure that started from workbook version
    /// `base`, as one undoable entry called `label`. If any write is refused
    /// nothing is changed, and the error says which and why.
    pub fn apply_writes(
        &mut self,
        base: u64,
        label: &str,
        writes: &[Write],
    ) -> Result<(), EditError> {
        if writes.is_empty() {
            return Ok(());
        }
        // The rule for an evaluation that an edit overtook: a node the user
        // changed since the procedure started, and the procedure wrote.
        for write in writes {
            if self.edited_since(base, &write.loc, false) {
                return Err(EditError::Path(tr!(
                    "{loc} was edited while the procedure ran, so none of its writes were applied",
                    loc = write.loc
                )));
            }
        }
        self.begin_entry(label);
        let result = writes.iter().try_for_each(|write| self.apply_write(write));
        match result {
            Ok(()) => {
                self.end_entry();
                Ok(())
            }
            Err(error) => {
                self.abort_entry();
                Err(error)
            }
        }
    }

    /// Takes the result of a procedure: its writes are applied if it ran
    /// without failing, all or nothing. The message of a failure, or of a
    /// refused write, is what the user is shown.
    pub fn apply_procedure(&mut self, result: ProcedureResult) -> Result<(), String> {
        let label = tr!("Run {button}", button = result.call.button);
        match result.result {
            Ok(writes) => self
                .apply_writes(result.base, &label, &writes)
                .map_err(|e| e.to_string()),
            Err(failure) => Err(failure.error.message),
        }
    }

    /// Ends the entry being built and takes back what it did, leaving nothing
    /// in the history, not even something to redo.
    pub fn abort_entry(&mut self) {
        if let Some(entry) = self.history.abort() {
            let _ = self.run_entry(&entry, false);
        }
    }

    fn apply_write(&mut self, write: &Write) -> Result<(), EditError> {
        let sheet = write.loc.sheet.as_str();
        let path = &write.loc.path;
        if !self.sheets.contains_key(sheet) {
            return Err(EditError::NoSheet(sheet.to_owned()));
        }
        match &write.op {
            WriteOp::Set(Value::Null) if self.is_optional(sheet, path) => {
                self.remove_node(sheet, path)
            }
            WriteOp::Set(value) => {
                self.check_value(sheet, path, value)?;
                self.refuse_formula_at(sheet, path)?;
                self.set_value(sheet, path, value.clone())
            }
            WriteOp::Remove => {
                let in_array = matches!(path.segments().last(), Some(Segment::Index(_)));
                if !in_array && !self.is_optional(sheet, path) {
                    return Err(EditError::Path(tr!(
                        "{loc} is required, so it cannot be removed",
                        loc = write.loc
                    )));
                }
                self.remove_node(sheet, path)
            }
            WriteOp::Append(value) => {
                let len = match self.data(sheet).and_then(|d| d.get(path)) {
                    Some(node) => match node.value() {
                        Value::Array(items) => items.len(),
                        _ => {
                            return Err(EditError::Path(tr!(
                                "{loc} is not an array",
                                loc = write.loc
                            )));
                        }
                    },
                    None => {
                        return Err(EditError::Path(tr!(
                            "there is nothing at {loc}",
                            loc = write.loc
                        )));
                    }
                };
                self.check_value(sheet, &path.index(len), value)?;
                self.refuse_inside_computed(sheet, path)?;
                self.insert_item(sheet, path, len, value.clone())
            }
        }
    }

    fn remove_node(&mut self, sheet: &str, path: &Path) -> Result<(), EditError> {
        self.refuse_formula_at(sheet, path)?;
        self.remove(sheet, path)
    }

    fn is_optional(&self, sheet: &str, path: &Path) -> bool {
        self.spec(sheet)
            .is_some_and(|spec| spec.is_optional_field(path))
    }

    /// A formula at the node itself owns it, as one above it does.
    fn refuse_formula_at(&self, sheet: &str, path: &Path) -> Result<(), EditError> {
        match self.sheets.get(sheet) {
            Some(s) if s.formulas.by_path.contains_key(path) => Err(EditError::InsideComputed {
                node: Loc::new(sheet, path.clone()),
                formula: path.clone(),
            }),
            _ => self.refuse_inside_computed(sheet, path),
        }
    }

    /// Checks a value against the schema of the node it is written to.
    fn check_value(&self, sheet: &str, path: &Path, value: &Value) -> Result<(), EditError> {
        let loc = Loc::new(sheet, path.clone());
        let spec = self
            .spec(sheet)
            .ok_or_else(|| EditError::NoSheet(sheet.to_owned()))?;
        let here = spec
            .resolve(path)
            .ok_or(EditError::NotInSchema(loc.clone()))?;
        match validate_at(spec, here.ty, &Node::new(value.clone()), path).first() {
            Some(issue) => Err(EditError::Path(format!("{loc}: {}", issue.message))),
            None => Ok(()),
        }
    }
}
