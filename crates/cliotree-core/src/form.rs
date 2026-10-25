//! Form designs (DESIGN.md, "Forms"): the tree of controls that a form is
//! drawn from, the built-in schema it is stored under (`builtin:form`), the
//! generated form, and the checks of a design against the schema of its tab.
//!
//! A design holds no data and no pixel positions. It is a list of controls laid
//! out one under the other; a group arranges its children in one or more
//! columns. A control is addressed by its place in the tree, a list of
//! indices ([`Address`]).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::typespec::{Kind, TypeSpec};
use crate::validate::validate;
use crate::{Path, json, jsonschema};

/// The version of the design document this build writes.
pub const DESIGN_VERSION: u32 = 1;

/// The place of a control in a design: the index in the list of the design,
/// then in the children of that control, and so on.
pub type Address = Vec<usize>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlKind {
    /// The control for the node at a path: an editor for a scalar, a list of
    /// entries for a map.
    Field,
    /// Controls arranged in columns, with a title; bound to an object, it can
    /// make the object when it is absent.
    Group,
    /// An array, as a table or one item at a time.
    Repeater,
    Label,
    /// Runs a procedure of a module (step 7c).
    Button,
    Separator,
}

impl ControlKind {
    pub const ALL: [ControlKind; 6] = [
        ControlKind::Field,
        ControlKind::Group,
        ControlKind::Repeater,
        ControlKind::Label,
        ControlKind::Button,
        ControlKind::Separator,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ControlKind::Field => "Field",
            ControlKind::Group => "Group",
            ControlKind::Repeater => "Repeater",
            ControlKind::Label => "Label",
            ControlKind::Button => "Button",
            ControlKind::Separator => "Separator",
        }
    }
}

/// The editor a field uses, when the one its type calls for is not wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Editor {
    /// What the type calls for: a toggle, a dropdown, a text field.
    Auto,
    /// One line of text, for any scalar.
    Text,
    /// Several lines of text, for a string.
    Multiline,
}

impl Editor {
    pub const ALL: [Editor; 3] = [Editor::Auto, Editor::Text, Editor::Multiline];

    pub fn label(self) -> &'static str {
        match self {
            Editor::Auto => "Automatic",
            Editor::Text => "One line of text",
            Editor::Multiline => "Several lines of text",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeaterMode {
    /// A table with a row for each item.
    Table,
    /// One item at a time, with a stepper.
    Stacked,
}

impl RepeaterMode {
    pub fn label(self) -> &'static str {
        match self {
            RepeaterMode::Table => "Table",
            RepeaterMode::Stacked => "One at a time",
        }
    }
}

/// One control. All kinds share the one record, so that the stored document
/// can be described in the schema subset, which has no unions; the checks say
/// which properties a kind uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    #[serde(rename = "type")]
    pub kind: ControlKind,
    /// The node a field, group or repeater is bound to, as a formula writes
    /// it without the tab name: `customer.name`, `items`. An empty path is the
    /// root. No index appears in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The label of a field or repeater, or the title of a group, when it is
    /// not the name of the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Help text, when it is not the description in the schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// The text of a label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<Editor>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    /// For a group: the number of columns its children are arranged in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    /// For a repeater: a table or one at a time. Absent: a table when the
    /// items are flat, one at a time otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<RepeaterMode>,
    /// For a repeater drawn as a table: the fields that are its columns, in
    /// order. Absent: all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<Vec<String>>,
    /// For a button: the procedure it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub procedure: Option<String>,
    /// Python expressions, without the `=`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_when: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Control>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Control {
    pub fn new(kind: ControlKind) -> Control {
        Control {
            kind,
            path: None,
            label: None,
            help: None,
            text: None,
            editor: None,
            read_only: false,
            columns: None,
            mode: None,
            show: None,
            procedure: None,
            visible_when: None,
            enabled_when: None,
            children: Vec::new(),
        }
    }

    pub fn bound(kind: ControlKind, path: &Path) -> Control {
        Control {
            path: Some(path.to_string()),
            ..Control::new(kind)
        }
    }

    /// The path this control is bound to, if it has one that can be read.
    pub fn bound_path(&self) -> Option<Path> {
        self.path.as_deref().and_then(|p| p.parse().ok())
    }

    /// Whether the kind binds to a node.
    pub fn binds(&self) -> bool {
        matches!(
            self.kind,
            ControlKind::Field | ControlKind::Group | ControlKind::Repeater
        ) && self.path.is_some()
    }
}

/// A form: the controls under its title, laid out one under the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormDesign {
    pub version: u32,
    /// Shown above the controls. Absent: the title of the schema, or the name
    /// of the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub controls: Vec<Control>,
}

impl Default for FormDesign {
    fn default() -> Self {
        FormDesign {
            version: DESIGN_VERSION,
            title: None,
            controls: Vec::new(),
        }
    }
}

// ----------------------------------------------------------- the schema

/// The text of the schema of design documents (`builtin:form`).
pub fn form_schema_source() -> &'static str {
    r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$ref": "#/$defs/Form",
  "$defs": {
    "Form": {
      "type": "object",
      "title": "Form design",
      "properties": {
        "version": {"type": "integer", "minimum": 1},
        "title": {"type": "string"},
        "controls": {"type": "array", "items": {"$ref": "#/$defs/Control"}}
      },
      "required": ["version", "controls"]
    },
    "Control": {
      "type": "object",
      "properties": {
        "type": {"type": "string", "enum": ["field", "group", "repeater", "label", "button", "separator"]},
        "path": {"type": "string"},
        "label": {"type": "string"},
        "help": {"type": "string"},
        "text": {"type": "string"},
        "editor": {"type": "string", "enum": ["auto", "text", "multiline"]},
        "read_only": {"type": "boolean"},
        "columns": {"type": "integer", "minimum": 1, "maximum": 6},
        "mode": {"type": "string", "enum": ["table", "stacked"]},
        "show": {"type": "array", "items": {"type": "string"}},
        "procedure": {"type": "string"},
        "visible_when": {"type": "string"},
        "enabled_when": {"type": "string"},
        "children": {"type": "array", "items": {"$ref": "#/$defs/Control"}}
      },
      "required": ["type"]
    }
  }
}"##
}

/// The lowered schema of design documents, made once.
pub fn form_schema() -> &'static TypeSpec {
    static SPEC: OnceLock<TypeSpec> = OnceLock::new();
    SPEC.get_or_init(|| {
        jsonschema::lower_str(form_schema_source()).expect("the bundled form schema is valid")
    })
}

impl FormDesign {
    /// The design as the JSON document it is stored as, pretty-printed.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a design always serializes")
    }

    /// Reads a design document. It must be JSON that conforms to the form
    /// schema; the reason is given in words if it does not. Whether its paths
    /// exist in a tab is a different question, see [`FormDesign::broken`].
    pub fn from_json(text: &str) -> Result<FormDesign, String> {
        let parsed = json::parse(text).map_err(|e| format!("not valid JSON: {e}"))?;
        let issues = validate(form_schema(), &parsed.node);
        if let Some(issue) = issues.first() {
            let at = if issue.path.is_root() {
                "the document".to_owned()
            } else {
                issue.path.to_string()
            };
            return Err(format!("{at}: {}", issue.message));
        }
        let design: FormDesign = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if design.version > DESIGN_VERSION {
            return Err(format!(
                "this form was made by a newer version (design version {}, this build reads up to {DESIGN_VERSION})",
                design.version
            ));
        }
        Ok(design)
    }
}

// ----------------------------------------------------------- the tree

impl FormDesign {
    fn list(&self, parent: &[usize]) -> Option<&Vec<Control>> {
        let mut list = &self.controls;
        for &i in parent {
            list = &list.get(i)?.children;
        }
        Some(list)
    }

    fn list_mut(&mut self, parent: &[usize]) -> Option<&mut Vec<Control>> {
        let mut list = &mut self.controls;
        for &i in parent {
            list = &mut list.get_mut(i)?.children;
        }
        Some(list)
    }

    pub fn get(&self, address: &[usize]) -> Option<&Control> {
        let (last, parent) = address.split_last()?;
        self.list(parent)?.get(*last)
    }

    pub fn get_mut(&mut self, address: &[usize]) -> Option<&mut Control> {
        let (last, parent) = address.split_last()?;
        self.list_mut(parent)?.get_mut(*last)
    }

    /// Adds `control` as the child of `parent` (the top level for an empty
    /// address) at `index`, or at the end. Returns where it is.
    pub fn insert(
        &mut self,
        parent: &[usize],
        index: Option<usize>,
        control: Control,
    ) -> Option<Address> {
        let list = self.list_mut(parent)?;
        let index = index.unwrap_or(list.len()).min(list.len());
        list.insert(index, control);
        let mut at = parent.to_vec();
        at.push(index);
        Some(at)
    }

    pub fn remove(&mut self, address: &[usize]) -> Option<Control> {
        let (last, parent) = address.split_last()?;
        let list = self.list_mut(parent)?;
        (*last < list.len()).then(|| list.remove(*last))
    }

    /// Moves a control one place up or down among its siblings. Returns the
    /// new address, or None if it is at the end already.
    pub fn shift(&mut self, address: &[usize], up: bool) -> Option<Address> {
        let (last, parent) = address.split_last()?;
        let list = self.list_mut(parent)?;
        let other = if up {
            last.checked_sub(1)?
        } else {
            (*last + 1 < list.len()).then_some(*last + 1)?
        };
        list.swap(*last, other);
        let mut at = parent.to_vec();
        at.push(other);
        Some(at)
    }

    /// Moves a control into the list that ends before it (the group above it
    /// takes it as its last child), or out of its group, to just after it.
    /// `into` chooses which. Returns the new address.
    pub fn nest(&mut self, address: &[usize], into: bool) -> Option<Address> {
        let (last, parent) = address.split_last()?;
        if into {
            let above = last.checked_sub(1)?;
            let mut target = parent.to_vec();
            target.push(above);
            if self.get(&target)?.kind != ControlKind::Group {
                return None;
            }
            let control = self.remove(address)?;
            let list = self.list_mut(&target)?;
            list.push(control);
            let mut at = target;
            at.push(list.len() - 1);
            Some(at)
        } else {
            let (group, grand) = parent.split_last()?;
            let control = self.remove(address)?;
            let index = group + 1;
            let mut at = grand.to_vec();
            self.list_mut(&at)?.insert(index, control);
            at.push(index);
            Some(at)
        }
    }

    /// Every control with its address, in the order of the form.
    pub fn walk(&self) -> Vec<(Address, &Control)> {
        fn visit<'a>(list: &'a [Control], at: &mut Address, out: &mut Vec<(Address, &'a Control)>) {
            for (i, control) in list.iter().enumerate() {
                at.push(i);
                out.push((at.clone(), control));
                visit(&control.children, at, out);
                at.pop();
            }
        }
        let mut out = Vec::new();
        visit(&self.controls, &mut Vec::new(), &mut out);
        out
    }
}

// ------------------------------------------------------ the generated form

/// The form drawn from a schema when none has been made: an object becomes a
/// group, an array a repeater, anything else a field.
pub fn generate(spec: &TypeSpec) -> FormDesign {
    let root = spec.root();
    let controls = match &spec.get(root).kind {
        Kind::Object { .. } => generated_children(spec, root, &Path::root()),
        _ => vec![generated(spec, root, &Path::root(), "")],
    };
    FormDesign {
        version: DESIGN_VERSION,
        title: None,
        controls,
    }
}

/// The control the generated form has for the node at `path`: what *Add* puts
/// on a form for a field that is not on it. None if the schema has no such node.
pub fn control_for(spec: &TypeSpec, path: &Path) -> Option<Control> {
    let resolved = spec.resolve(path)?;
    let name = match path.segments().last() {
        Some(crate::Segment::Key(key)) => key.clone(),
        _ => String::new(),
    };
    Some(generated(spec, resolved.ty, path, &name))
}

fn generated_children(spec: &TypeSpec, ty: crate::TypeId, path: &Path) -> Vec<Control> {
    let Kind::Object { fields } = &spec.get(ty).kind else {
        return Vec::new();
    };
    fields
        .iter()
        .map(|f| generated(spec, f.ty, &path.key(f.name.as_str()), &f.name))
        .collect()
}

fn generated(spec: &TypeSpec, ty: crate::TypeId, path: &Path, name: &str) -> Control {
    let label = (!name.is_empty()).then(|| name.to_owned());
    let kind = match &spec.get(ty).kind {
        Kind::Object { .. } => ControlKind::Group,
        Kind::Array { .. } => ControlKind::Repeater,
        _ => ControlKind::Field,
    };
    let mut control = Control::bound(kind, path);
    control.label = label;
    if kind == ControlKind::Group {
        control.children = generated_children(spec, ty, path);
    }
    control
}

/// The paths a control of `kind` can be bound to: a field takes a scalar or a
/// map, a group an object, a repeater an array. Only nodes that exist
/// without an index are offered, and the root is not.
pub fn bindable(spec: &TypeSpec, kind: ControlKind) -> Vec<Path> {
    fn visit(
        spec: &TypeSpec,
        ty: crate::TypeId,
        path: &Path,
        kind: ControlKind,
        out: &mut Vec<Path>,
    ) {
        let Kind::Object { fields } = &spec.get(ty).kind else {
            return;
        };
        for field in fields {
            let child = path.key(field.name.as_str());
            let wanted = match (&spec.get(field.ty).kind, kind) {
                (Kind::Object { .. }, ControlKind::Group) => true,
                (Kind::Array { .. }, ControlKind::Repeater) => true,
                (Kind::Object { .. } | Kind::Array { .. }, _) => false,
                (_, ControlKind::Field) => true,
                _ => false,
            };
            if wanted {
                out.push(child.clone());
            }
            visit(spec, field.ty, &child, kind, out);
        }
    }
    let mut out = Vec::new();
    visit(spec, spec.root(), &Path::root(), kind, &mut out);
    out
}

// ------------------------------------------------- a design and its schema

impl FormDesign {
    /// Why the control at `address` cannot be drawn against `spec`, or None.
    /// Paths are literal: one the schema does not define is broken, whatever
    /// the schema was before.
    pub fn problem(&self, spec: &TypeSpec, address: &[usize]) -> Option<String> {
        let control = self.get(address)?;
        control_problem(spec, control)
    }

    /// The controls that are broken, with the reason.
    pub fn broken(&self, spec: &TypeSpec) -> Vec<(Address, String)> {
        self.walk()
            .into_iter()
            .filter_map(|(at, c)| control_problem(spec, c).map(|p| (at, p)))
            .collect()
    }

    /// The nodes of the schema that no control shows, outermost first: a node
    /// that is shown, or lies in one that is, is not listed, and an object
    /// that is partly shown is listed by the fields it lacks. Paths in arrays
    /// are not listed, since a repeater shows its items.
    pub fn unplaced(&self, spec: &TypeSpec) -> Vec<Path> {
        // A field or a repeater shows its node and all that is in it. A group
        // shows only its children, so it only marks its object as partly shown.
        let mut bound = Vec::new();
        let mut groups = Vec::new();
        for (_, control) in self.walk() {
            if let Some(path) = control.bound_path().filter(|_| control.binds()) {
                if control.kind == ControlKind::Group {
                    groups.push(path);
                } else {
                    bound.push(path);
                }
            }
        }
        let shown = Shown { bound, groups };
        let mut out = Vec::new();
        visit_unplaced(spec, spec.root(), &Path::root(), &shown, &mut out);
        out
    }
}

/// Why a control cannot be drawn against a schema, or None.
pub fn control_problem(spec: &TypeSpec, control: &Control) -> Option<String> {
    let Some(text) = control.path.as_deref() else {
        return match control.kind {
            ControlKind::Field | ControlKind::Repeater => Some(format!(
                "a {} needs a path",
                control.kind.label().to_lowercase()
            )),
            _ => None,
        };
    };
    if !matches!(
        control.kind,
        ControlKind::Field | ControlKind::Group | ControlKind::Repeater
    ) {
        return None;
    }
    let path: Path = match text.parse() {
        Ok(path) => path,
        Err(e) => return Some(format!("the path {text:?} is not a path: {e}")),
    };
    let Some(resolved) = spec.resolve(&path) else {
        return Some(format!(
            "the schema has nothing at {}",
            if text.is_empty() { "the root" } else { text }
        ));
    };
    let kind = &spec.get(resolved.ty).kind;
    match (control.kind, kind) {
        (ControlKind::Repeater, Kind::Array { .. }) => None,
        (ControlKind::Repeater, _) => Some(format!("{text} is not an array")),
        (ControlKind::Group, Kind::Object { .. }) => None,
        (ControlKind::Group, _) => Some(format!("{text} is not an object")),
        _ => None,
    }
}

struct Shown {
    bound: Vec<Path>,
    groups: Vec<Path>,
}

fn visit_unplaced(
    spec: &TypeSpec,
    ty: crate::TypeId,
    path: &Path,
    shown: &Shown,
    out: &mut Vec<Path>,
) {
    let Kind::Object { fields } = &spec.get(ty).kind else {
        return;
    };
    for field in fields {
        let child = path.key(field.name.as_str());
        if shown.bound.iter().any(|b| inside(&child, b)) {
            continue;
        }
        let partly = shown.bound.iter().any(|b| inside(b, &child))
            || shown.groups.iter().any(|g| inside(g, &child));
        if !partly {
            out.push(child);
        } else if matches!(spec.get(field.ty).kind, Kind::Object { .. }) {
            visit_unplaced(spec, field.ty, &child, shown, out);
        }
    }
}

/// Whether `inner` is `outer` or lies in it.
fn inside(inner: &Path, outer: &Path) -> bool {
    inner.segments().starts_with(outer.segments())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TypeSpec {
        jsonschema::lower_str(
            r##"{
              "type": "object",
              "properties": {
                "name": {"type": "string"},
                "customer": {"type": "object", "properties": {
                    "city": {"type": "string"}, "vip": {"type": "boolean"}}},
                "items": {"type": "array", "items": {"type": "object",
                    "properties": {"sku": {"type": "string"}, "qty": {"type": "integer"}}}},
                "tags": {"type": "object", "additionalProperties": {"type": "string"}}
              }
            }"##,
        )
        .unwrap()
    }

    #[test]
    fn the_form_schema_lowers_and_is_not_a_meta_schema() {
        assert!(form_schema().xml().is_none());
    }

    #[test]
    fn the_generated_form_has_a_control_for_each_field() {
        let design = generate(&spec());
        let kinds: Vec<_> = design.controls.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                ControlKind::Field,
                ControlKind::Group,
                ControlKind::Repeater,
                ControlKind::Field
            ]
        );
        assert_eq!(design.controls[1].children.len(), 2);
        assert_eq!(design.controls[1].path.as_deref(), Some("customer"));
        assert_eq!(
            design.controls[1].children[0].path.as_deref(),
            Some("customer.city")
        );
        assert!(design.broken(&spec()).is_empty());
        assert!(design.unplaced(&spec()).is_empty());
    }

    #[test]
    fn a_design_round_trips_through_its_document() {
        let mut design = generate(&spec());
        design.title = Some("Orders".into());
        design.controls[0].read_only = true;
        design.controls[0].editor = Some(Editor::Multiline);
        design.controls[2].mode = Some(RepeaterMode::Stacked);
        design.controls[2].show = Some(vec!["sku".into()]);
        design.controls.push(Control::new(ControlKind::Separator));
        let text = design.to_json();
        assert_eq!(FormDesign::from_json(&text).unwrap(), design);
        // Properties that are not set are not written.
        assert!(!text.contains("null"));
    }

    #[test]
    fn a_document_that_does_not_conform_is_refused_with_the_reason() {
        let wrong = FormDesign::from_json(r#"{"version":1,"controls":[{"type":"widget"}]}"#);
        assert!(wrong.unwrap_err().contains("controls[0].type"));
        let missing = FormDesign::from_json(r#"{"controls":[]}"#).unwrap_err();
        assert!(missing.contains("version"), "{missing}");
        let extra =
            FormDesign::from_json(r#"{"version":1,"controls":[{"type":"label","colour":"red"}]}"#);
        assert!(extra.is_err());
        let not_json = FormDesign::from_json("{").unwrap_err();
        assert!(not_json.starts_with("not valid JSON"));
        let newer = FormDesign::from_json(r#"{"version":99,"controls":[]}"#).unwrap_err();
        assert!(newer.contains("newer version"));
    }

    #[test]
    fn controls_are_found_moved_and_removed_by_address() {
        let mut design = generate(&spec());
        assert_eq!(
            design.get(&[1, 1]).and_then(|c| c.path.as_deref()),
            Some("customer.vip")
        );
        assert_eq!(design.shift(&[1, 1], true), Some(vec![1, 0]));
        assert_eq!(
            design.get(&[1, 0]).and_then(|c| c.path.as_deref()),
            Some("customer.vip")
        );
        assert_eq!(design.shift(&[1, 0], true), None);
        assert_eq!(design.shift(&[3], false), None);
        let removed = design.remove(&[0]).unwrap();
        assert_eq!(removed.path.as_deref(), Some("name"));
        assert_eq!(design.controls.len(), 3);
    }

    #[test]
    fn nesting_moves_a_control_into_the_group_above_and_out_again() {
        let mut design = generate(&spec());
        // [1] is the customer group, [2] the items repeater.
        let inside = design.nest(&[2], true).unwrap();
        assert_eq!(inside, vec![1, 2]);
        assert_eq!(design.controls.len(), 3);
        let outside = design.nest(&inside, false).unwrap();
        assert_eq!(outside, vec![2]);
        assert_eq!(design.controls.len(), 4);
        assert_eq!(design.nest(&[0], true), None, "nothing is above the first");
        assert_eq!(
            design.nest(&[0], false),
            None,
            "the first is not in a group"
        );
        // Only a group takes a control.
        assert_eq!(design.nest(&[3], true), None);
    }

    #[test]
    fn the_paths_a_control_can_bind_to_depend_on_its_kind() {
        let names = |kind| -> Vec<String> {
            bindable(&spec(), kind)
                .iter()
                .map(Path::to_string)
                .collect()
        };
        assert_eq!(
            names(ControlKind::Field),
            ["name", "customer.city", "customer.vip", "tags"]
        );
        assert_eq!(names(ControlKind::Group), ["customer"]);
        assert_eq!(names(ControlKind::Repeater), ["items"]);
        assert!(names(ControlKind::Label).is_empty());
    }

    #[test]
    fn a_path_the_schema_lacks_is_a_broken_control() {
        let mut design = generate(&spec());
        design.controls[0].path = Some("nickname".into());
        design.controls[2].path = Some("name".into());
        let broken = design.broken(&spec());
        assert_eq!(broken.len(), 2);
        assert!(broken[0].1.contains("nothing at nickname"));
        assert!(broken[1].1.contains("not an array"));
        design.controls[1].children[0].path = Some("customer[".into());
        assert!(
            design
                .problem(&spec(), &[1, 0])
                .unwrap()
                .contains("not a path")
        );
        // A field needs a path; a label does not.
        let mut bare = FormDesign::default();
        bare.controls.push(Control::new(ControlKind::Field));
        bare.controls.push(Control::new(ControlKind::Label));
        assert_eq!(bare.broken(&spec()).len(), 1);
    }

    #[test]
    fn fields_that_are_not_on_the_form_are_listed_outermost_first() {
        let mut design = generate(&spec());
        design.remove(&[3]);
        design.remove(&[1, 0]);
        design.remove(&[0]);
        let unplaced: Vec<String> = design
            .unplaced(&spec())
            .iter()
            .map(Path::to_string)
            .collect();
        assert_eq!(unplaced, ["name", "customer.city", "tags"]);
        // A group bound to an object shows all of it, whatever it holds.
        let mut group = FormDesign::default();
        group.controls.push(Control::bound(
            ControlKind::Group,
            &"customer".parse().unwrap(),
        ));
        let rest: Vec<String> = group
            .unplaced(&spec())
            .iter()
            .map(Path::to_string)
            .collect();
        assert_eq!(
            rest,
            ["name", "customer.city", "customer.vip", "items", "tags"]
        );
    }
}
