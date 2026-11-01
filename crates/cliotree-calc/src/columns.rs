//! Column formulas: one formula for a field of every item of an array
//! (DESIGN.md, "Array and object formulas", "Column formulas").
//!
//! A column formula is a rule that the tab keeps: every item of the array has
//! an ordinary formula on that field, with the rule's text. The rule is what
//! makes a new item get its formula, and what is saved (once, not once per
//! item). Entering something else in one item ends the rule and leaves the
//! other items with ordinary formulas.

use super::*;

/// The rule: this field of every item of this array has this formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnFormula {
    /// The array, which is not inside another array.
    pub array: Path,
    /// A field of its items.
    pub field: String,
    /// The expression, without the `=`.
    pub text: String,
}

impl ColumnFormula {
    /// `items[].total`, as the trust record names it.
    pub fn label(&self) -> String {
        let array = self.array.to_string();
        if array.is_empty() {
            format!("[].{}", self.field)
        } else {
            format!("{array}[].{}", self.field)
        }
    }

    /// The place of this field in item `index`.
    pub fn place(&self, index: usize) -> Path {
        self.array.index(index).key(self.field.clone())
    }

    /// Whether `path` is this field of some item.
    pub(crate) fn holds(&self, path: &Path) -> bool {
        let depth = self.array.len();
        let segments = path.segments();
        segments.len() == depth + 2
            && segments.starts_with(self.array.segments())
            && matches!(segments[depth], Segment::Index(_))
            && matches!(&segments[depth + 1], Segment::Key(k) if *k == self.field)
    }
}

impl Calc {
    /// The column formulas of a tab.
    pub fn column_formulas(&self, sheet: &str) -> &[ColumnFormula] {
        self.sheets.get(sheet).map_or(&[], |s| &s.columns)
    }

    /// The text of the column formula on `field` of the items of `array`.
    pub fn column_formula(&self, sheet: &str, array: &Path, field: &str) -> Option<&str> {
        self.column_formulas(sheet)
            .iter()
            .find(|c| c.array == *array && c.field == field)
            .map(|c| c.text.as_str())
    }

    /// The column formula that owns `path`, if `path` is that field of an item.
    pub fn column_of(&self, sheet: &str, path: &Path) -> Option<&ColumnFormula> {
        self.column_formulas(sheet).iter().find(|c| c.holds(path))
    }

    /// Gives the field of every item of `array` one formula, replacing what
    /// the field held (the results overwrite it) and any formulas there. Items
    /// added later get it too. One entry in the history.
    pub fn set_column_formula(
        &mut self,
        sheet: &str,
        array: &Path,
        field: &str,
        text: &str,
    ) -> Result<(), EditError> {
        self.put_column_formula(sheet, array, field, text, false)
    }

    /// As [`Calc::set_column_formula`], but formulas inside the field of the
    /// items are removed first instead of refusing the formula.
    pub fn set_column_formula_replacing(
        &mut self,
        sheet: &str,
        array: &Path,
        field: &str,
        text: &str,
    ) -> Result<(), EditError> {
        self.put_column_formula(sheet, array, field, text, true)
    }

    /// What a column formula on the field would erase: the values the items
    /// hold there and the formulas in and inside it, apart from those of a
    /// column formula already there, which it replaces as a whole.
    pub fn detail_inside_column(&self, sheet: &str, array: &Path, field: &str) -> Detail {
        let mut detail = Detail::default();
        let Some(s) = self.sheets.get(sheet) else {
            return detail;
        };
        let length = self.column_length(sheet, array, field).unwrap_or(0);
        let rule = ColumnFormula {
            array: array.clone(),
            field: field.to_owned(),
            text: String::new(),
        };
        for index in 0..length {
            let place = rule.place(index);
            if self.column_of(sheet, &place).is_some() {
                continue;
            }
            if let Some(node) = s.data.get(&place) {
                collect_detail(s, &place, node, &mut detail);
            }
        }
        detail
    }

    fn put_column_formula(
        &mut self,
        sheet: &str,
        array: &Path,
        field: &str,
        text: &str,
        replace: bool,
    ) -> Result<(), EditError> {
        let length = self.column_length(sheet, array, field)?;
        let rule = ColumnFormula {
            array: array.clone(),
            field: field.to_owned(),
            text: text.to_owned(),
        };
        for index in 0..length {
            self.check_formula_place(sheet, &rule.place(index), replace)?;
        }
        let before = self.column_formula(sheet, array, field).map(str::to_owned);
        let mut ops = vec![self.store_column(sheet, array, field, Some(text), before)];
        for index in 0..length {
            if replace {
                ops.extend(self.clear_formulas_below(sheet, &rule.place(index)));
            }
            ops.push(self.formula_op(sheet, &rule.place(index), text));
        }
        self.history.record(
            format!("Set column formula at {}", Loc::new(sheet, array.clone())),
            ops,
            None,
        );
        Ok(())
    }

    /// Removes the column formula and the formulas it gave the items, which
    /// keep the values they produced. Returns whether there was one.
    pub fn remove_column_formula(
        &mut self,
        sheet: &str,
        array: &Path,
        field: &str,
    ) -> Result<bool, EditError> {
        self.sheet_mut(sheet)?;
        let Some(text) = self.column_formula(sheet, array, field).map(str::to_owned) else {
            return Ok(false);
        };
        let rule = ColumnFormula {
            array: array.clone(),
            field: field.to_owned(),
            text: text.clone(),
        };
        let mut ops = vec![self.store_column(sheet, array, field, None, Some(text.clone()))];
        for path in self.column_places(sheet, &rule) {
            if self.drop_formula(sheet, &path) {
                self.changed(sheet, &path);
                ops.push(Op::Formula {
                    sheet: sheet.to_owned(),
                    path,
                    before: Some(text.clone()),
                    after: None,
                    value: None,
                });
            }
        }
        self.history.record(
            format!("Clear column formula at {}", Loc::new(sheet, array.clone())),
            ops,
            None,
        );
        Ok(true)
    }

    // -------------------------------------------------------- internals

    /// How many items the array has, after checking that the schema lets each
    /// have a formula on that field and that the column can be a column.
    pub(super) fn column_length(
        &self,
        sheet: &str,
        array: &Path,
        field: &str,
    ) -> Result<usize, EditError> {
        let s = self
            .sheets
            .get(sheet)
            .ok_or_else(|| EditError::NoSheet(sheet.to_owned()))?;
        let rule = ColumnFormula {
            array: array.clone(),
            field: field.to_owned(),
            text: String::new(),
        };
        if array
            .segments()
            .iter()
            .any(|s| matches!(s, Segment::Index(_)))
        {
            return Err(EditError::Path(
                "a column formula is for an array that is not inside another array".to_owned(),
            ));
        }
        if field.is_empty() || field.contains(['.', '[', ']']) {
            return Err(EditError::Path(format!(
                "{field:?} is not the name of a field of the items"
            )));
        }
        let place = rule.place(0);
        if s.spec.resolve(&place).is_none() {
            return Err(EditError::NotInSchema(Loc::new(sheet, place)));
        }
        if let Some(formula) = s.formulas.computed_by(&place) {
            return Err(EditError::InsideComputed {
                node: Loc::new(sheet, place),
                formula,
            });
        }
        match s.data.get(array).map(Node::value) {
            Some(Value::Array(items)) => Ok(items.len()),
            Some(_) => Err(EditError::Path(format!("{array} is not an array"))),
            None => Err(EditError::Path(format!("there is no array at {array}"))),
        }
    }

    /// Where the rule's formulas are now: the field of each item that has one.
    fn column_places(&self, sheet: &str, rule: &ColumnFormula) -> Vec<Path> {
        let Some(s) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        s.formulas
            .below(&rule.array)
            .into_iter()
            .filter(|p| rule.holds(p))
            .collect()
    }

    /// Puts the rule in the tab (`text`) or takes it out (`None`), returning
    /// the operation that does it.
    fn store_column(
        &mut self,
        sheet: &str,
        array: &Path,
        field: &str,
        text: Option<&str>,
        before: Option<String>,
    ) -> Op {
        self.put_column(sheet, array, field, text);
        Op::Column {
            sheet: sheet.to_owned(),
            array: array.clone(),
            field: field.to_owned(),
            before,
            after: text.map(str::to_owned),
        }
    }

    pub(super) fn put_column(
        &mut self,
        sheet: &str,
        array: &Path,
        field: &str,
        text: Option<&str>,
    ) {
        let Some(s) = self.sheets.get_mut(sheet) else {
            return;
        };
        s.columns
            .retain(|c| !(c.array == *array && c.field == field));
        if let Some(text) = text {
            s.columns.push(ColumnFormula {
                array: array.clone(),
                field: field.to_owned(),
                text: text.to_owned(),
            });
        }
    }

    /// Entering something else in one item of a column ends the column
    /// formula; the other items keep theirs as ordinary formulas.
    pub(super) fn release_column(&mut self, sheet: &str, path: &Path) -> Option<Op> {
        let rule = self.column_of(sheet, path)?.clone();
        Some(self.store_column(
            sheet,
            &rule.array,
            &rule.field,
            None,
            Some(rule.text.clone()),
        ))
    }

    /// Gives the items that lack the formula of a column their formula, and
    /// takes it from places past the end of the array, after an edit that
    /// may have added or removed items. `at` is where the edit was.
    pub(super) fn sync_columns(&mut self, sheet: &str, at: &Path) -> Vec<Op> {
        let rules: Vec<ColumnFormula> = self
            .column_formulas(sheet)
            .iter()
            .filter(|c| {
                // The edit is the array, above it, or one of its items.
                c.array.segments().starts_with(at.segments())
                    || at.parent().as_ref() == Some(&c.array)
            })
            .cloned()
            .collect();
        let mut ops = Vec::new();
        for rule in rules {
            let length = match self.data(sheet).and_then(|d| d.get(&rule.array)) {
                Some(node) => match node.value() {
                    Value::Array(items) => items.len(),
                    _ => 0,
                },
                None => 0,
            };
            let have: std::collections::HashSet<Path> =
                self.column_places(sheet, &rule).into_iter().collect();
            for index in 0..length {
                let place = rule.place(index);
                if !have.contains(&place) {
                    ops.push(self.formula_op(sheet, &place, &rule.text));
                }
            }
            for place in have {
                let past = matches!(
                    place.segments().get(rule.array.len()),
                    Some(Segment::Index(i)) if *i >= length
                );
                if past && self.drop_formula(sheet, &place) {
                    self.changed(sheet, &place);
                    ops.push(Op::Formula {
                        sheet: sheet.to_owned(),
                        path: place,
                        before: Some(rule.text.clone()),
                        after: None,
                        value: None,
                    });
                }
            }
        }
        ops
    }
}
