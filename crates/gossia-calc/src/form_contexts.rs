//! The contexts of canvas forms (DESIGN.md, Later versions, "Form canvases,
//! 0.7.0", "Context fields"): the values of the context fields of a form's
//! header and the current row of each of its repeaters. They belong to the
//! form, not to the tab: they are not data, not in the history and not in
//! the trust digest. The workbook remembers the values chosen, as it
//! remembers the view a tab shows.
//!
//! A context field not chosen has its default; a row number not chosen is
//! 0, kept within the items of its array, and `None` while the array is
//! empty or its path cannot be bound.

use gossia_core::form::FormDesign;
use gossia_core::form::canvas::ContextPath;
use gossia_core::{Node, Path, Value};
use indexmap::IndexMap;

use super::*;

/// The contexts of a form by name, in the order of its context fields then
/// its repeaters.
pub type Contexts = IndexMap<String, Value>;

/// The values of every context of `design`, from the values `chosen` and
/// the tab's `data`. The repeaters are bound in their order, so a
/// repeater's path can use the row of one before it (`orders[row].lines`).
pub fn contexts_of(
    design: &FormDesign,
    data: Option<&Node>,
    chosen: Option<&Contexts>,
) -> Contexts {
    let mut out = Contexts::new();
    if !design.canvas {
        return out;
    }
    let chosen_value = |name: &str| chosen.and_then(|c| c.get(name)).cloned();
    for (_, field) in design.context_fields() {
        let Some(name) = field.context.clone() else {
            continue;
        };
        let value =
            chosen_value(&name).unwrap_or_else(|| field.context_start().unwrap_or(Value::Null));
        out.entry(name).or_insert(value);
    }
    for index in design.repeater_sections() {
        let Some(name) = design.row_name_of(index) else {
            continue;
        };
        let len = design.controls[index]
            .path
            .as_deref()
            .and_then(|t| t.parse::<ContextPath>().ok())
            .and_then(|p| p.bind(|n| out.get(n).cloned()).ok())
            .and_then(|path| data?.get(&path))
            .and_then(|n| n.value().as_array().map(<[Node]>::len))
            .unwrap_or(0);
        let value = if len == 0 {
            Value::Null
        } else {
            let at = match chosen_value(&name) {
                Some(Value::Int(i)) => i.clamp(0, len as i64 - 1),
                _ => 0,
            };
            Value::Int(at)
        };
        out.entry(name).or_insert(value);
    }
    out
}

/// `context` as expressions see it: an object of the contexts by name.
pub fn context_object(contexts: &Contexts) -> Value {
    Value::Object(
        contexts
            .iter()
            .map(|(k, v)| (k.clone(), Node::new(v.clone())))
            .collect(),
    )
}

/// The path of a control's `path` text bound with `contexts`, or None when
/// it cannot be read or bound.
pub fn bind_path(text: &str, contexts: &Contexts) -> Option<Path> {
    let parsed: ContextPath = text.parse().ok()?;
    parsed.bind(|n| contexts.get(n).cloned()).ok()
}

impl Calc {
    /// The values chosen for the contexts of a form, by name. A context not
    /// here has its default.
    pub fn chosen_contexts(&self, tab: &str, form: &str) -> Option<&Contexts> {
        self.contexts.get(&(tab.to_owned(), form.to_owned()))
    }

    /// Every form of `tab` with contexts chosen, for saving.
    pub fn chosen_contexts_of_tab(&self, tab: &str) -> Vec<(String, Contexts)> {
        let mut out: Vec<(String, Contexts)> = self
            .contexts
            .iter()
            .filter(|((t, _), c)| t == tab && !c.is_empty())
            .map(|((_, f), c)| (f.clone(), c.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The contexts of the form `form` of `tab` drawn from `design` (the
    /// stored design, or the generated one).
    pub fn form_contexts(&self, tab: &str, form: &str, design: &FormDesign) -> Contexts {
        contexts_of(design, self.data(tab), self.chosen_contexts(tab, form))
    }

    /// Chooses a value for a context of a form: a context field's value, or
    /// a repeater's current row. This is not an edit: it is not in the
    /// history, and runs again only the expressions that use contexts.
    /// Returns whether it changed.
    pub fn set_form_context(&mut self, tab: &str, form: &str, name: &str, value: Value) -> bool {
        let values = self
            .contexts
            .entry((tab.to_owned(), form.to_owned()))
            .or_default();
        let changed = values.insert(name.to_owned(), value.clone()) != Some(value);
        if changed {
            self.contexts_version += 1;
            self.exprs.resync = true;
        }
        changed
    }

    /// Puts back the chosen contexts of a form, as a workbook remembers them.
    pub fn set_chosen_contexts(&mut self, tab: &str, form: &str, values: Contexts) {
        let key = (tab.to_owned(), form.to_owned());
        if values.is_empty() {
            self.contexts.remove(&key);
        } else {
            self.contexts.insert(key, values);
        }
        self.contexts_version += 1;
        self.exprs.resync = true;
    }

    /// Changes whenever a context's chosen value changes.
    pub fn contexts_version(&self) -> u64 {
        self.contexts_version
    }
}
