//! Renaming a tab (DESIGN.md, "Renaming a tab").
//!
//! What belongs to the tab goes with it: its data, formulas, column formulas,
//! forms, its place in the order, and the bindings of the tabs that use it as
//! their schema. What only names it does not: formula text and module source
//! are never rewritten (DESIGN.md, "Renames do not rewrite formulas"), so they
//! are calculated again and find the tab, or not, by the name they say.

use std::sync::Arc;

use cliotree_core::form::FormDesign;
use cliotree_core::xml::XmlDocument;
use cliotree_core::{Node, Path, TypeSpec};
use indexmap::IndexMap;

use super::{Calc, EditError, SchemaFormat, SchemaRole, Sheet};
use crate::history::Op;
use crate::model::Loc;
use crate::names::{self, Subject};

/// How much of the workbook mentions a name, for the warning of the rename
/// dialog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mentions {
    /// Formulas, a column formula counting once.
    pub formulas: usize,
    pub modules: usize,
}

impl Mentions {
    pub fn is_empty(&self) -> bool {
        self.formulas == 0 && self.modules == 0
    }
}

/// A tab as the history keeps it: everything it holds and where it was, so
/// that adding it again, or putting a deleted tab back, gives the same tab.
#[derive(Debug, Clone)]
pub(crate) struct SavedTab {
    pub index: usize,
    pub name: String,
    sheet: Sheet,
    forms: IndexMap<String, FormDesign>,
}

impl SavedTab {
    /// A rough size, for the bound on how much history is kept.
    pub fn weight(&self) -> usize {
        fn size(node: &Node) -> usize {
            1 + match node.value() {
                cliotree_core::Value::Array(items) => items.iter().map(size).sum(),
                cliotree_core::Value::Object(fields) => fields.values().map(size).sum(),
                _ => 0,
            }
        }
        size(&self.sheet.data) + self.sheet.formulas.by_path.len()
    }
}

impl Calc {
    /// Why `name` cannot be the name of a new tab, or of the tab `renaming`
    /// renamed to it, or None. A tab may keep its own name in other letters.
    pub fn tab_name_problem(&self, name: &str, renaming: Option<&str>) -> Option<String> {
        if let Some(why) = names::problem(name, Subject::Tab) {
            return Some(why);
        }
        let lower = name.to_lowercase();
        if let Some(other) = self
            .sheets
            .keys()
            .find(|s| Some(s.as_str()) != renaming && s.to_lowercase() == lower)
        {
            return Some(format!(
                "a tab called {other} exists already (names ignore case)"
            ));
        }
        if let Some(module) = self.modules.keys().find(|m| m.to_lowercase() == lower) {
            return Some(format!(
                "{module} is the name of a module, and a tab cannot share it"
            ));
        }
        None
    }

    /// Gives a tab another name: one entry in the history. A name equal to the
    /// current one is no edit.
    pub fn rename_sheet(&mut self, from: &str, to: &str) -> Result<(), EditError> {
        if !self.sheets.contains_key(from) {
            return Err(EditError::NoSheet(from.to_owned()));
        }
        if from == to {
            return Ok(());
        }
        if let Some(why) = self.tab_name_problem(to, Some(from)) {
            return Err(EditError::BadName(why));
        }
        self.store_rename(from, to);
        self.history.record(
            format!("Rename tab {from} to {to}"),
            vec![Op::RenameSheet {
                from: from.to_owned(),
                to: to.to_owned(),
            }],
            None,
        );
        Ok(())
    }

    /// The renames done since the last call, oldest first, undoing and redoing
    /// included, so that the window can move what it keeps for a tab.
    pub fn take_renames(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.renames)
    }

    /// How many formulas and modules mention `name` as an identifier. It may
    /// count too many (a field of that name, a word in a string), never too
    /// few.
    pub fn mentions(&self, name: &str) -> Mentions {
        let mut mentions = Mentions::default();
        for sheet in self.sheets.values() {
            for (path, formula) in &sheet.formulas.by_path {
                if sheet.columns.iter().any(|c| c.holds(path)) {
                    continue;
                }
                if names::identifiers(&formula.text).contains(name) {
                    mentions.formulas += 1;
                }
            }
            mentions.formulas += sheet
                .columns
                .iter()
                .filter(|c| names::identifiers(&c.text).contains(name))
                .count();
        }
        mentions.modules = self
            .modules
            .values()
            .filter(|m| names::identifiers(&m.source).contains(name))
            .count();
        mentions
    }

    /// Renames without checking or recording, for an edit and for undoing and
    /// redoing one.
    pub(super) fn store_rename(&mut self, from: &str, to: &str) {
        let Some(at) = self.sheets.get_index_of(from) else {
            return;
        };
        let (_, sheet) = self
            .sheets
            .shift_remove_index(at)
            .expect("the index was found");
        // The formulas of the tab are known by their place, which has the tab's
        // name in it: what they read and whether they are out of date.
        let formulas = Arc::clone(&sheet.formulas);
        for (path, formula) in &formulas.by_path {
            let old = Loc::new(from, path.clone());
            let new = Loc::new(to, path.clone());
            self.forget_deps(&old, &formula.deps);
            self.remember_deps(&new, &formula.deps);
            if self.dirty.swap_remove(&old) {
                self.dirty.insert(new);
            }
        }
        self.sheets.insert(to.to_owned(), sheet);
        let last = self.sheets.len() - 1;
        self.sheets.move_index(last, at);
        for s in self.sheets.values_mut() {
            if let SchemaRole::Bound(schema) = &mut s.schema
                && schema == from
            {
                *schema = to.to_owned();
            }
        }
        self.forms.rename_tab(from, to);
        self.exprs.resync = true;
        self.forms_version += 1;
        self.renames.push((from.to_owned(), to.to_owned()));
        // What read the old name now finds nothing, what looked for the new
        // one finds the tab, and the tab's own formulas may name it.
        self.changed(from, &Path::root());
        self.changed(to, &Path::root());
    }

    // ------------------------------------------------------ new tabs

    /// Adds a data tab bound to a schema tab, holding `data`, at the end of the
    /// tabs. `xml` says how an XML tab's document is written; a tab bound to
    /// an XSD that is given none is written with the usual declaration.
    pub fn create_data_tab(
        &mut self,
        name: &str,
        schema_sheet: &str,
        data: Node,
        xml: Option<XmlDocument>,
    ) -> Result<(), EditError> {
        self.create_tab_of(name, schema_sheet, None, data, xml)
    }

    /// Adds a tab of COBOL records bound to a copybook schema tab, holding
    /// its 01 record `record` (None is the first).
    pub fn create_records_tab(
        &mut self,
        name: &str,
        schema_sheet: &str,
        record: Option<&str>,
        data: Node,
    ) -> Result<(), EditError> {
        self.create_tab_of(name, schema_sheet, record, data, None)
    }

    fn create_tab_of(
        &mut self,
        name: &str,
        schema_sheet: &str,
        record: Option<&str>,
        data: Node,
        xml: Option<XmlDocument>,
    ) -> Result<(), EditError> {
        self.refuse_tab_name(name)?;
        if !self.is_schema_sheet(schema_sheet) {
            return Err(EditError::Path(format!(
                "{schema_sheet} is not a schema tab"
            )));
        }
        self.refuse_auto_update_in_use(schema_sheet, name)?;
        let spec = self.lowered_for(schema_sheet, record)?;
        let xml_data = spec.xml().is_some();
        self.add_sheet(name, spec, data)?;
        let sheet = self.sheet_mut(name)?;
        sheet.schema = SchemaRole::Bound(schema_sheet.to_owned());
        sheet.record = record.map(str::to_owned);
        if xml_data {
            sheet.xml = Some(xml.unwrap_or_else(|| XmlDocument {
                declaration: Some(cliotree_core::xmltree::Declaration::default()),
                ..XmlDocument::default()
            }));
        }
        self.record_added(name);
        Ok(())
    }

    /// Adds a schema tab holding a JSON Schema, at the end of the tabs.
    pub fn create_json_schema_tab(&mut self, name: &str, document: Node) -> Result<(), EditError> {
        self.refuse_tab_name(name)?;
        self.add_schema_sheet(name, document)?;
        self.record_added(name);
        Ok(())
    }

    /// Adds a schema tab holding an XSD, as the XSD meta-schema types it.
    pub fn create_xsd_schema_tab(
        &mut self,
        name: &str,
        tree: Node,
        document: XmlDocument,
    ) -> Result<(), EditError> {
        self.refuse_tab_name(name)?;
        self.add_xsd_sheet(name, tree, document)?;
        self.record_added(name);
        Ok(())
    }

    /// Adds a copybook schema tab, as one entry in the history.
    pub fn create_copybook_schema_tab(&mut self, name: &str, tree: Node) -> Result<(), EditError> {
        self.refuse_tab_name(name)?;
        self.add_copybook_sheet(name, tree)?;
        self.record_added(name);
        Ok(())
    }

    fn refuse_tab_name(&self, name: &str) -> Result<(), EditError> {
        match self.tab_name_problem(name, None) {
            Some(why) => Err(EditError::BadName(why)),
            None => Ok(()),
        }
    }

    /// A schema with auto-update on takes one tab only.
    fn refuse_auto_update_in_use(&self, schema_sheet: &str, sheet: &str) -> Result<(), EditError> {
        if self.auto_update(schema_sheet)
            && self
                .schema_users(schema_sheet)
                .iter()
                .any(|user| user != sheet)
        {
            return Err(EditError::Path(format!(
                "{schema_sheet} has auto-update switched on, so only one tab may use it; \
                 switch that off first"
            )));
        }
        Ok(())
    }

    /// Moves the tab `name` to place `to` in the order of the tabs (0 is
    /// the first; past the end is the last). One entry in the history;
    /// nothing is recalculated, since formulas name tabs, not places.
    pub fn move_sheet(&mut self, name: &str, to: usize) -> Result<(), EditError> {
        let from = self
            .sheets
            .get_index_of(name)
            .ok_or_else(|| EditError::Path(format!("there is no tab called {name}")))?;
        let to = to.min(self.sheets.len() - 1);
        if from == to {
            return Ok(());
        }
        self.put_sheet_move(from, to);
        self.history.record(
            format!("Move tab {name}"),
            vec![Op::MoveSheet { from, to }],
            None,
        );
        Ok(())
    }

    /// Moves a tab in the order, as an edit and its undo do.
    pub(super) fn put_sheet_move(&mut self, from: usize, to: usize) {
        self.sheets.move_index(from, to);
        self.revision += 1;
    }

    fn record_added(&mut self, name: &str) {
        let Some(saved) = self.saved_tab(name) else {
            return;
        };
        self.history.record(
            format!("New tab {name}"),
            vec![Op::AddSheet(Box::new(saved))],
            None,
        );
    }

    fn saved_tab(&self, name: &str) -> Option<SavedTab> {
        let (index, _, sheet) = self.sheets.get_full(name)?;
        Some(SavedTab {
            index,
            name: name.to_owned(),
            sheet: sheet.clone(),
            forms: self
                .forms
                .names(name)
                .into_iter()
                .filter_map(|form| Some((form.to_owned(), self.forms.get(name, form)?.clone())))
                .collect(),
        })
    }

    // -------------------------------------------------------- deleting

    /// Why the tab cannot be deleted, or None.
    pub fn delete_problem(&self, name: &str) -> Option<String> {
        if !self.sheets.contains_key(name) {
            return Some(format!("there is no tab called {name}"));
        }
        if self.sheets.len() == 1 {
            return Some("a workbook keeps at least one tab".to_owned());
        }
        let users = self.schema_users(name);
        if !users.is_empty() {
            let (verb, them, it) = if users.len() == 1 {
                ("uses", "that tab", "it")
            } else {
                ("use", "those tabs", "them")
            };
            return Some(format!(
                "{} {verb} this schema; delete {them} or give {it} another schema first",
                users.join(", "),
            ));
        }
        None
    }

    /// Deletes a tab: one entry in the history, which puts the tab back with
    /// its data, formulas, forms and place. Formulas elsewhere that name it
    /// give `#NAME!` until it is back.
    pub fn delete_sheet(&mut self, name: &str) -> Result<(), EditError> {
        if !self.sheets.contains_key(name) {
            return Err(EditError::NoSheet(name.to_owned()));
        }
        if let Some(why) = self.delete_problem(name) {
            return Err(EditError::Path(why));
        }
        let saved = self.store_remove_tab(name).expect("the tab exists");
        self.history.record(
            format!("Delete tab {name}"),
            vec![Op::RemoveSheet(Box::new(saved))],
            None,
        );
        Ok(())
    }

    /// The tabs deleted since the last call, undoing and redoing included, so
    /// that the window can forget what it kept for them.
    pub fn take_removed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.removed)
    }

    /// Takes a tab out without checking or recording.
    pub(super) fn store_remove_tab(&mut self, name: &str) -> Option<SavedTab> {
        let saved = self.saved_tab(name)?;
        self.sheets.shift_remove(name);
        for (path, formula) in &saved.sheet.formulas.by_path {
            let loc = Loc::new(name, path.clone());
            self.forget_deps(&loc, &formula.deps);
            self.dirty.swap_remove(&loc);
        }
        self.forms.take_tab(name);
        self.exprs.resync = true;
        self.forms_version += 1;
        self.removed.push(name.to_owned());
        // What read the tab now finds nothing.
        self.changed(name, &Path::root());
        Some(saved)
    }

    /// Puts a tab in, as it was saved, without checking or recording.
    pub(super) fn store_insert_tab(&mut self, saved: &SavedTab) {
        let SavedTab {
            index,
            name,
            sheet,
            forms,
        } = saved.clone();
        let formulas = Arc::clone(&sheet.formulas);
        self.sheets.insert(name.clone(), sheet);
        let last = self.sheets.len() - 1;
        self.sheets.move_index(last, index.min(last));
        // Its formulas are calculated again: what they read may have changed.
        for (path, formula) in &formulas.by_path {
            let loc = Loc::new(name.as_str(), path.clone());
            self.remember_deps(&loc, &formula.deps);
            self.dirty.insert(loc);
        }
        self.forms.put_tab(&name, forms);
        self.exprs.resync = true;
        self.forms_version += 1;
        self.changed(&name, &Path::root());
    }

    // ------------------------------------------------------- rebinding

    /// Gives a data tab another schema tab: one entry in the history. The data
    /// stays as it is, and what does not fit the new schema is flagged. A tab
    /// keeps its format, so JSON data takes a JSON Schema and XML data an XSD.
    pub fn rebind_sheet(&mut self, sheet: &str, schema_sheet: &str) -> Result<(), EditError> {
        if let Some(why) = self.rebind_problem(sheet, schema_sheet) {
            return Err(EditError::Path(why));
        }
        let before = self.schema_of(sheet).map(str::to_owned);
        if before.as_deref() == Some(schema_sheet) {
            return Ok(());
        }
        let before_spec = Arc::clone(&self.sheet_mut(sheet)?.spec);
        let record = self.sheets[sheet].record.clone();
        let spec = self.lowered_for(schema_sheet, record.as_deref())?;
        self.store_binding(sheet, Some(schema_sheet.to_owned()), Arc::new(spec))?;
        self.history.record(
            format!("Bind {sheet} to {schema_sheet}"),
            vec![Op::Bind {
                sheet: sheet.to_owned(),
                before,
                before_spec,
                after: schema_sheet.to_owned(),
            }],
            None,
        );
        Ok(())
    }

    /// Why `sheet` cannot be bound to `schema_sheet`, or None.
    pub fn rebind_problem(&self, sheet: &str, schema_sheet: &str) -> Option<String> {
        let Some(s) = self.sheets.get(sheet) else {
            return Some(format!("there is no tab called {sheet}"));
        };
        if self.is_schema_sheet(sheet) {
            return Some(format!(
                "{sheet} is a schema tab, which is described by the built-in schema"
            ));
        }
        let Some(format) = self.schema_format(schema_sheet) else {
            return Some(format!("{schema_sheet} is not a schema tab"));
        };
        let own = SchemaFormat::of(&s.spec);
        if own != format {
            return Some(format!(
                "{sheet} holds {}, so it cannot be bound to {schema_sheet}",
                own.data_label()
            ));
        }
        if let Err(e) = self.refuse_auto_update_in_use(schema_sheet, sheet) {
            return Some(e.to_string());
        }
        let spec = match self.lowered_for(schema_sheet, s.record.as_deref()) {
            Ok(spec) => spec,
            Err(e) => return Some(e.to_string()),
        };
        if let Some(why) = s.format.root_problem(&spec.get(spec.root()).kind) {
            return Some(format!("{sheet} is written as {}: {why}", s.format.label()));
        }
        None
    }

    /// The type of a schema tab's document, for checking data against it
    /// before it is in a tab.
    pub fn schema_spec(&self, schema_sheet: &str) -> Result<TypeSpec, EditError> {
        self.lowered(schema_sheet)
    }

    /// The type of a copybook schema tab for its 01 record `record`.
    pub fn record_spec(&self, schema_sheet: &str, record: &str) -> Result<TypeSpec, EditError> {
        self.lowered_for(schema_sheet, Some(record))
    }

    /// The names of the 01 records of a copybook schema tab, in order; none
    /// for any other tab.
    pub fn copybook_records(&self, schema_sheet: &str) -> Vec<String> {
        if self.schema_format(schema_sheet) != Some(SchemaFormat::Copybook) {
            return Vec::new();
        }
        self.sheets
            .get(schema_sheet)
            .and_then(|s| cliotree_core::copybook::from_node(&s.data).ok())
            .map(|(copybook, _)| cliotree_core::copybook::record_names(&copybook))
            .unwrap_or_default()
    }

    /// The 01 record a tab of COBOL records holds: the one its layout is of.
    pub fn record_of(&self, sheet: &str) -> Option<String> {
        let s = self.sheets.get(sheet)?;
        s.spec.copybook().map(|layout| layout.record.clone())
    }

    pub(super) fn store_binding(
        &mut self,
        sheet: &str,
        schema: Option<String>,
        spec: Arc<TypeSpec>,
    ) -> Result<(), EditError> {
        let s = self.sheet_mut(sheet)?;
        s.spec = spec;
        s.schema = match schema {
            Some(schema) => SchemaRole::Bound(schema),
            None => SchemaRole::Free,
        };
        self.changed(sheet, &Path::root());
        Ok(())
    }
}
