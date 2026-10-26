//! The customised forms of the tabs (DESIGN.md, "Forms"). A form that has not
//! been customised is not stored: the Default form of a tab is generated from
//! its schema until the first change to it, and the other forms exist only
//! because they were made.
//!
//! Forms are not data: editing one is not an edit of the workbook, is not in
//! its undo history and recalculates nothing.

use cliotree_core::form::FormDesign;
use indexmap::IndexMap;

use crate::names::{self, Subject};

/// The name of the form every tab has.
pub const DEFAULT_FORM: &str = "Default";

#[derive(Debug, Clone, Default)]
pub struct FormStore {
    /// By tab, then by form name, in display order.
    by_tab: IndexMap<String, IndexMap<String, FormDesign>>,
}

impl FormStore {
    /// The names of the stored forms of a tab, in display order. Default is
    /// among them only once it has been customised.
    pub fn names(&self, tab: &str) -> Vec<&str> {
        self.by_tab
            .get(tab)
            .map(|forms| forms.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// The names of the forms a tab offers: Default first, then the others.
    pub fn all_names(&self, tab: &str) -> Vec<String> {
        let mut names = vec![DEFAULT_FORM.to_owned()];
        names.extend(
            self.names(tab)
                .into_iter()
                .filter(|n| *n != DEFAULT_FORM)
                .map(str::to_owned),
        );
        names
    }

    pub fn get(&self, tab: &str, name: &str) -> Option<&FormDesign> {
        self.by_tab.get(tab)?.get(name)
    }

    /// Stores a design under a name that is already free or is the form's own.
    /// A new form goes to the end of the order.
    pub fn set(&mut self, tab: &str, name: &str, design: FormDesign) {
        self.by_tab
            .entry(tab.to_owned())
            .or_default()
            .insert(name.to_owned(), design);
    }

    /// Takes away every form of a tab, in display order, for a tab that is
    /// deleted.
    pub fn take_tab(&mut self, tab: &str) -> IndexMap<String, FormDesign> {
        self.by_tab.shift_remove(tab).unwrap_or_default()
    }

    /// Gives a tab back the forms [`FormStore::take_tab`] took.
    pub fn put_tab(&mut self, tab: &str, forms: IndexMap<String, FormDesign>) {
        if !forms.is_empty() {
            self.by_tab.insert(tab.to_owned(), forms);
        }
    }

    /// Discards a design. Default is then generated again; any other form is
    /// gone. Returns whether there was one.
    pub fn remove(&mut self, tab: &str, name: &str) -> bool {
        let Some(forms) = self.by_tab.get_mut(tab) else {
            return false;
        };
        let removed = forms.shift_remove(name).is_some();
        if forms.is_empty() {
            self.by_tab.shift_remove(tab);
        }
        removed
    }

    /// Why `name` cannot be the name of a new form of `tab`, or of a form
    /// renamed to it, or None. `renaming` is the form's own name, which it may
    /// keep in other letters.
    pub fn name_problem(&self, tab: &str, name: &str, renaming: Option<&str>) -> Option<String> {
        if let Some(why) = names::problem(name, Subject::Form) {
            return Some(why);
        }
        let taken = self.all_names(tab).into_iter().any(|n| {
            n.eq_ignore_ascii_case(name) && renaming.is_none_or(|own| !n.eq_ignore_ascii_case(own))
        });
        taken.then(|| format!("the tab {tab} has a form called {name} already"))
    }

    /// Moves the forms of a tab to its new name, keeping their order and the
    /// place of the tab among the others.
    pub fn rename_tab(&mut self, from: &str, to: &str) {
        if let Some(at) = self.by_tab.get_index_of(from) {
            let (_, forms) = self
                .by_tab
                .shift_remove_index(at)
                .expect("the index was found");
            self.by_tab.insert(to.to_owned(), forms);
            let last = self.by_tab.len() - 1;
            self.by_tab.move_index(last, at);
        }
    }

    /// Renames a stored form, which keeps its place. Default cannot be renamed.
    pub fn rename(&mut self, tab: &str, from: &str, to: &str) -> Result<(), String> {
        if from == DEFAULT_FORM {
            return Err("the Default form cannot be renamed".to_owned());
        }
        if let Some(why) = self.name_problem(tab, to, Some(from)) {
            return Err(why);
        }
        let Some(forms) = self.by_tab.get_mut(tab) else {
            return Err(format!("{tab} has no form {from}"));
        };
        let Some(at) = forms.get_index_of(from) else {
            return Err(format!("{tab} has no form {from}"));
        };
        let (_, design) = forms.shift_remove_index(at).expect("the index was found");
        forms.insert(to.to_owned(), design);
        let last = forms.len() - 1;
        forms.move_index(last, at);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design(title: &str) -> FormDesign {
        FormDesign {
            title: Some(title.to_owned()),
            ..FormDesign::default()
        }
    }

    #[test]
    fn default_is_always_offered_first_and_only_stored_once_customised() {
        let mut store = FormStore::default();
        assert_eq!(store.all_names("Orders"), ["Default"]);
        assert!(store.names("Orders").is_empty());
        store.set("Orders", "Packing", design("Packing"));
        store.set("Orders", "Default", design("Mine"));
        assert_eq!(store.all_names("Orders"), ["Default", "Packing"]);
        assert!(store.remove("Orders", "Default"));
        assert_eq!(store.all_names("Orders"), ["Default", "Packing"]);
        assert!(store.get("Orders", "Default").is_none());
        assert!(store.remove("Orders", "Packing"));
        assert!(!store.remove("Orders", "Packing"));
        assert_eq!(store.all_names("Orders"), ["Default"]);
    }

    #[test]
    fn names_follow_the_rules_of_tab_names_and_are_unique_ignoring_case() {
        let mut store = FormStore::default();
        store.set("Orders", "Packing", design("a"));
        assert_eq!(store.name_problem("Orders", "Invoice", None), None);
        assert!(store.name_problem("Orders", "my form", None).is_some());
        assert!(store.name_problem("Orders", "", None).is_some());
        assert!(
            store
                .name_problem("Orders", "packing", None)
                .unwrap()
                .contains("already")
        );
        assert!(
            store
                .name_problem("Orders", "DEFAULT", None)
                .unwrap()
                .contains("already")
        );
        // Another tab may have the same name; the form's own name may change case.
        assert_eq!(store.name_problem("Prices", "Packing", None), None);
        assert_eq!(
            store.name_problem("Orders", "PACKING", Some("Packing")),
            None
        );
    }

    #[test]
    fn a_rename_keeps_the_place_and_default_stays() {
        let mut store = FormStore::default();
        store.set("Orders", "A", design("a"));
        store.set("Orders", "B", design("b"));
        store.set("Orders", "C", design("c"));
        store.rename("Orders", "B", "Beta").unwrap();
        assert_eq!(store.names("Orders"), ["A", "Beta", "C"]);
        assert_eq!(
            store.get("Orders", "Beta").unwrap().title.as_deref(),
            Some("b")
        );
        assert!(store.rename("Orders", "A", "c").is_err());
        assert!(store.rename("Orders", "Default", "Main").is_err());
        assert!(store.rename("Orders", "Missing", "Other").is_err());
    }
}
