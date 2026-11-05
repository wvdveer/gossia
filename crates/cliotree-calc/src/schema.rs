//! Schema tabs, binding a tab to one, auto-update, and the field commands that
//! a data tab can issue (DESIGN.md, "Schema binding" and "Editing the schema
//! from a data tab").
//!
//! A schema tab holds a schema document as ordinary data, described by the
//! bundled meta-schema. Whenever it changes, by an edit, an undo or a command,
//! the tabs bound to it take the newly lowered type ([`Calc::relower_if_schema`]),
//! so a command is only ever an edit of the document together with the edits of
//! the data and formulas that go with it, recorded as one entry.

use cliotree_core::xml::XmlDocument;
use cliotree_core::{Kind, TypeId, jsonschema, metaschema, xsd};
use indexmap::IndexMap;

use super::xsd_fields::XsdChange;
use super::*;

/// The type of a field that a command adds: a simple type, an object with
/// no fields yet, or an array of such objects (whose fields are added the
/// same way, on an item). Anything more involved is written on the schema
/// tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Integer,
    Number,
    Boolean,
    Date,
    DateTime,
    Object,
    Array,
}

impl FieldKind {
    pub const ALL: [FieldKind; 8] = [
        FieldKind::Text,
        FieldKind::Integer,
        FieldKind::Number,
        FieldKind::Boolean,
        FieldKind::Date,
        FieldKind::DateTime,
        FieldKind::Object,
        FieldKind::Array,
    ];

    /// An object or an array: it holds fields rather than a value.
    pub fn is_structured(self) -> bool {
        matches!(self, FieldKind::Object | FieldKind::Array)
    }

    /// What the dialog says about a type that needs saying.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            FieldKind::Object => {
                Some("An object with no fields yet: add them to it in the same way.")
            }
            FieldKind::Array => {
                Some("A list of objects with no fields yet: add an item, then add fields to it.")
            }
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Integer => "whole number",
            FieldKind::Number => "number",
            FieldKind::Boolean => "yes or no",
            FieldKind::Date => "date",
            FieldKind::DateTime => "date and time",
            FieldKind::Object => "object",
            FieldKind::Array => "array",
        }
    }

    /// The schema of a field of this type, as it is written in the document.
    fn schema(self) -> Node {
        let record = || {
            let mut fields = IndexMap::new();
            fields.insert("type".to_owned(), text("object"));
            fields.insert(
                "properties".to_owned(),
                Node::new(Value::Object(IndexMap::new())),
            );
            Node::new(Value::Object(fields))
        };
        match self {
            FieldKind::Object => return record(),
            FieldKind::Array => {
                let mut fields = IndexMap::new();
                fields.insert("type".to_owned(), text("array"));
                fields.insert("items".to_owned(), record());
                return Node::new(Value::Object(fields));
            }
            _ => {}
        }
        let (ty, format) = match self {
            FieldKind::Text => ("string", None),
            FieldKind::Integer => ("integer", None),
            FieldKind::Number => ("number", None),
            FieldKind::Boolean => ("boolean", None),
            FieldKind::Date => ("string", Some("date")),
            FieldKind::DateTime => ("string", Some("date-time")),
            FieldKind::Object | FieldKind::Array => unreachable!("written above"),
        };
        let mut fields = IndexMap::new();
        fields.insert("type".to_owned(), text(ty));
        if let Some(format) = format {
            fields.insert("format".to_owned(), text(format));
        }
        Node::new(Value::Object(fields))
    }
}

fn text(s: &str) -> Node {
    Node::new(Value::Str(s.to_owned()))
}

/// The language a schema tab is written in. It fixes the format of the data of
/// the tabs bound to it: JSON Schema gives JSON data (or JSON-shaped), XSD
/// gives XML data, and a COBOL copybook gives `.blk` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaFormat {
    Json,
    Xsd,
    Copybook,
}

impl SchemaFormat {
    /// The schema language of a lowered type: the one its data goes with.
    pub fn of(spec: &TypeSpec) -> SchemaFormat {
        if spec.xml().is_some() {
            SchemaFormat::Xsd
        } else if spec.copybook().is_some() {
            SchemaFormat::Copybook
        } else {
            SchemaFormat::Json
        }
    }

    /// What its data is called, for messages.
    pub fn data_label(self) -> &'static str {
        match self {
            SchemaFormat::Json => "JSON data, described by a JSON Schema",
            SchemaFormat::Xsd => "XML data, described by an XSD",
            SchemaFormat::Copybook => "COBOL records, described by a copybook",
        }
    }
}

/// Where an object type is used, for a command that changes it.
#[derive(Debug, Clone)]
pub struct FieldPlaces {
    /// The schema tab that is edited.
    pub schema_sheet: String,
    /// Where the object type is written in the schema document.
    pub document_path: Path,
    /// The fields now, in declared order.
    pub fields: Vec<String>,
    /// Every object of this type in the tab: where the command changes data.
    pub objects: Vec<Path>,
    /// The other places in the tab's schema that use the same type, written
    /// like `Orders.billing` or `Orders.items[]` (`[]` is each item), because a
    /// command on a shared type changes all of them.
    pub also_used_at: Vec<String>,
    /// The schema tab is an XSD: the type is a `complexType`, and attribute
    /// fields are named with a leading `@`.
    pub xsd: bool,
}

impl Calc {
    // ------------------------------------------------------ schema tabs

    /// Adds a tab that holds a schema document. Its content is checked against
    /// the bundled meta-schema by the usual validation, and must lower for
    /// tabs to be bound to it; if it does not, [`Calc::schema_error`] says why.
    pub fn add_schema_sheet(
        &mut self,
        name: impl Into<String>,
        document: Node,
    ) -> Result<(), EditError> {
        let name = name.into();
        self.add_sheet(
            name.clone(),
            metaschema::jsonschema_meta().clone(),
            document,
        )?;
        self.sheet_mut(&name)?.schema = SchemaRole::Document {
            format: SchemaFormat::Json,
            auto_update: false,
            error: None,
        };
        self.relower_schema(&name);
        Ok(())
    }

    /// Adds a tab that holds an XSD, as the tree the bundled XSD meta-schema
    /// types (an XML document: see `cliotree_core::xml`). `document` says how
    /// the XML was written; [`cliotree_core::xsd::default_document`] is the one
    /// a new schema starts with.
    pub fn add_xsd_sheet(
        &mut self,
        name: impl Into<String>,
        tree: Node,
        document: XmlDocument,
    ) -> Result<(), EditError> {
        let name = name.into();
        self.add_sheet(name.clone(), metaschema::xsd_meta().clone(), tree)?;
        let sheet = self.sheet_mut(&name)?;
        sheet.xml = Some(document);
        sheet.schema = SchemaRole::Document {
            format: SchemaFormat::Xsd,
            auto_update: false,
            error: None,
        };
        self.relower_schema(&name);
        Ok(())
    }

    /// Adds a tab that holds a COBOL copybook, as the tree the bundled
    /// copybook meta-schema types (see `cliotree_core::copybook::to_node`).
    /// It is kept as JSON5, which holds the comments of its items.
    pub fn add_copybook_sheet(
        &mut self,
        name: impl Into<String>,
        tree: Node,
    ) -> Result<(), EditError> {
        let name = name.into();
        self.add_sheet(name.clone(), metaschema::copybook_meta().clone(), tree)?;
        let sheet = self.sheet_mut(&name)?;
        sheet.format = cliotree_core::dataformat::DataFormat::Json5;
        sheet.schema = SchemaRole::Document {
            format: SchemaFormat::Copybook,
            auto_update: false,
            error: None,
        };
        self.relower_schema(&name);
        Ok(())
    }

    /// The language of a schema tab, if the tab is one.
    pub fn schema_format(&self, sheet: &str) -> Option<SchemaFormat> {
        match &self.sheets.get(sheet)?.schema {
            SchemaRole::Document { format, .. } => Some(*format),
            _ => None,
        }
    }

    /// What an XML tab says about its document beyond its tree. None for a
    /// tab that is not XML.
    pub fn xml_document(&self, sheet: &str) -> Option<&XmlDocument> {
        self.sheets.get(sheet)?.xml.as_ref()
    }

    /// Records how the XML of a tab was written, so it is written the same way.
    pub fn set_xml_document(
        &mut self,
        sheet: &str,
        document: XmlDocument,
    ) -> Result<(), EditError> {
        self.sheet_mut(sheet)?.xml = Some(document);
        Ok(())
    }

    /// Whether the tab holds XML data, as its schema is an XSD.
    pub fn is_xml(&self, sheet: &str) -> bool {
        self.sheets
            .get(sheet)
            .is_some_and(|s| s.spec.xml().is_some())
    }

    /// Binds a tab to a schema tab: the tab takes the type the document
    /// describes. Refused while auto-update is on and another tab uses the
    /// schema, and when the document cannot be lowered.
    pub fn bind_schema(&mut self, sheet: &str, schema_sheet: &str) -> Result<(), EditError> {
        self.sheet_mut(sheet)?;
        if !self.is_schema_sheet(schema_sheet) {
            return Err(EditError::Path(format!(
                "{schema_sheet} is not a schema tab"
            )));
        }
        if self.is_schema_sheet(sheet) {
            return Err(EditError::Path(format!(
                "{sheet} is a schema tab, which is described by the built-in schema"
            )));
        }
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
        let record = self.sheets.get(sheet).and_then(|s| s.record.clone());
        let spec = self.lowered_for(schema_sheet, record.as_deref())?;
        let s = self.sheet_mut(sheet)?;
        s.spec = Arc::new(spec);
        s.schema = SchemaRole::Bound(schema_sheet.to_owned());
        self.changed(sheet, &Path::root());
        Ok(())
    }

    pub fn is_schema_sheet(&self, sheet: &str) -> bool {
        matches!(
            self.sheets.get(sheet).map(|s| &s.schema),
            Some(SchemaRole::Document { .. })
        )
    }

    /// The schema tab a tab is bound to.
    pub fn schema_of(&self, sheet: &str) -> Option<&str> {
        match &self.sheets.get(sheet)?.schema {
            SchemaRole::Bound(schema) => Some(schema),
            _ => None,
        }
    }

    /// The tabs bound to a schema tab, in tab order.
    pub fn schema_users(&self, schema_sheet: &str) -> Vec<String> {
        self.sheets
            .iter()
            .filter(|(_, s)| matches!(&s.schema, SchemaRole::Bound(b) if b == schema_sheet))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Whether the tab that uses this schema may change it.
    pub fn auto_update(&self, schema_sheet: &str) -> bool {
        matches!(
            self.sheets.get(schema_sheet).map(|s| &s.schema),
            Some(SchemaRole::Document {
                auto_update: true,
                ..
            })
        )
    }

    /// Why the document in a schema tab cannot be used as a type now.
    pub fn schema_error(&self, schema_sheet: &str) -> Option<&str> {
        match &self.sheets.get(schema_sheet)?.schema {
            SchemaRole::Document { error, .. } => error.as_deref(),
            _ => None,
        }
    }

    /// Why auto-update cannot be switched on, or `None` when it can: it needs
    /// exactly one tab that uses the schema, so that a change can only ever
    /// reach that one tab.
    pub fn auto_update_blocker(&self, schema_sheet: &str) -> Option<String> {
        if !self.is_schema_sheet(schema_sheet) {
            return Some(format!("{schema_sheet} is not a schema tab"));
        }
        match self.schema_users(schema_sheet).len() {
            1 => None,
            0 => Some("no tab uses this schema, so there is nothing to update".to_owned()),
            n => Some(format!(
                "{n} tabs use this schema; auto-update needs exactly one, so that a change \
                 can never reach another tab"
            )),
        }
    }

    /// Switches auto-update of a schema tab on or off, as an entry that can be
    /// undone.
    pub fn set_auto_update(&mut self, schema_sheet: &str, on: bool) -> Result<(), EditError> {
        if !self.is_schema_sheet(schema_sheet) {
            return Err(EditError::Path(format!(
                "{schema_sheet} is not a schema tab"
            )));
        }
        let before = self.auto_update(schema_sheet);
        if before == on {
            return Ok(());
        }
        if on && let Some(why) = self.auto_update_blocker(schema_sheet) {
            return Err(EditError::Path(why));
        }
        self.store_auto_update(schema_sheet, on)?;
        self.history.record(
            format!(
                "Switch auto-update {} for {schema_sheet}",
                if on { "on" } else { "off" }
            ),
            vec![Op::AutoUpdate {
                sheet: schema_sheet.to_owned(),
                before,
                after: on,
            }],
            None,
        );
        Ok(())
    }

    pub(super) fn store_auto_update(&mut self, sheet: &str, on: bool) -> Result<(), EditError> {
        match &mut self.sheet_mut(sheet)?.schema {
            SchemaRole::Document { auto_update, .. } => *auto_update = on,
            _ => return Err(EditError::Path(format!("{sheet} is not a schema tab"))),
        }
        self.revision += 1;
        Ok(())
    }

    // --------------------------------------------------------- lowering

    pub(super) fn lowered(&self, schema_sheet: &str) -> Result<TypeSpec, EditError> {
        self.lowered_for(schema_sheet, None)
    }

    /// The type of a schema tab's document for a tab that holds `record`,
    /// which only a copybook has a choice of.
    pub(super) fn lowered_for(
        &self,
        schema_sheet: &str,
        record: Option<&str>,
    ) -> Result<TypeSpec, EditError> {
        let s = self
            .sheets
            .get(schema_sheet)
            .ok_or_else(|| EditError::NoSheet(schema_sheet.to_owned()))?;
        let lowered = match &s.schema {
            SchemaRole::Document {
                format: SchemaFormat::Xsd,
                ..
            } => {
                let declarations = s
                    .xml
                    .as_ref()
                    .map(|d| d.declarations.clone())
                    .unwrap_or_else(|| xsd::default_document().declarations);
                xsd::lower_node(&s.data, &declarations).map_err(|e| e.to_string())
            }
            SchemaRole::Document {
                format: SchemaFormat::Copybook,
                ..
            } => cliotree_core::copybook::lower_node_record(&s.data, record),
            _ => jsonschema::lower(&s.data).map_err(|e| e.to_string()),
        };
        lowered.map_err(|e| {
            EditError::Path(format!("the schema in {schema_sheet} cannot be used: {e}"))
        })
    }

    pub(super) fn relower_if_schema(&mut self, sheet: &str) {
        if self.is_schema_sheet(sheet) {
            self.relower_schema(sheet);
        }
    }

    /// Lowers the document again and gives the result to the tabs bound to it.
    /// A document that does not lower leaves them as they were.
    fn relower_schema(&mut self, schema_sheet: &str) {
        let error = match self.lowered(schema_sheet) {
            Ok(spec) => {
                let spec = Arc::new(spec);
                // A tab of COBOL records may hold another 01 record.
                let records: Vec<(String, Option<String>)> = self
                    .sheets
                    .iter()
                    .filter(|(_, s)| s.record.is_some())
                    .map(|(name, s)| (name.clone(), s.record.clone()))
                    .collect();
                let mut own = std::collections::HashMap::new();
                for (name, record) in records {
                    if let Ok(spec) = self.lowered_for(schema_sheet, record.as_deref()) {
                        own.insert(name, Arc::new(spec));
                    }
                }
                for (name, s) in self.sheets.iter_mut() {
                    if matches!(&s.schema, SchemaRole::Bound(b) if b == schema_sheet) {
                        s.spec = own.remove(name).unwrap_or_else(|| Arc::clone(&spec));
                        // Their results were checked against the old type:
                        // a #TYPE! may fit now, and a value may not.
                        for (path, formula) in &s.formulas.by_path {
                            if !formula.syntax_error {
                                self.dirty.insert(Loc::new(name.clone(), path.clone()));
                            }
                        }
                    }
                }
                None
            }
            Err(e) => Some(e.to_string()),
        };
        if let Some(SchemaRole::Document { error: slot, .. }) =
            self.sheets.get_mut(schema_sheet).map(|s| &mut s.schema)
        {
            *slot = error;
        }
    }

    // --------------------------------------------------- finding places

    /// Checks that a field command may be issued from `sheet` on the object at
    /// `object` (any item of an array will do, even one that does not exist),
    /// and finds everywhere it applies.
    pub fn field_places(&self, sheet: &str, object: &Path) -> Result<FieldPlaces, EditError> {
        let s = self
            .sheets
            .get(sheet)
            .ok_or_else(|| EditError::NoSheet(sheet.to_owned()))?;
        let SchemaRole::Bound(schema_sheet) = &s.schema else {
            return Err(EditError::Path(format!(
                "the type of {sheet} is not defined by a schema tab, so its fields cannot be \
                 changed from here"
            )));
        };
        let is_xsd = self.schema_format(schema_sheet) == Some(SchemaFormat::Xsd);
        if self.schema_format(schema_sheet) == Some(SchemaFormat::Copybook) {
            return Err(EditError::Path(format!(
                "the fields of {sheet} are items of the copybook in {schema_sheet}, which are \
                 changed there"
            )));
        }
        if !self.auto_update(schema_sheet) {
            return Err(EditError::Path(format!(
                "switch on auto-update on {schema_sheet} to change the schema from {sheet}"
            )));
        }
        if let Some(error) = self.schema_error(schema_sheet) {
            return Err(EditError::Path(format!(
                "the schema in {schema_sheet} has an error: {error}"
            )));
        }
        let spec = &s.spec;
        let at = Loc::new(sheet, object.clone());
        let ty = spec
            .resolve(object)
            .ok_or_else(|| EditError::NotInSchema(at.clone()))?
            .ty;
        let Kind::Object { fields } = &spec.get(ty).kind else {
            return Err(EditError::Path(format!(
                "{at} is not a record, so it has no fields to change"
            )));
        };
        let source = &spec.get(ty).source;
        let document_path = if is_xsd {
            self.sheets
                .get(schema_sheet)
                .and_then(|d| xsd_fields::xsd_type_path(&d.data, source))
        } else {
            pointer_to_path(source)
        }
        .ok_or_else(|| {
            EditError::Path(format!("cannot tell where {at} is written in the schema"))
        })?;
        let document_ok = self
            .sheets
            .get(schema_sheet)
            .and_then(|d| d.data.get(&document_path))
            .is_some_and(|n| n.value().as_object().is_some());
        if !document_ok {
            return Err(EditError::Path(format!(
                "cannot find the type of {at} in {schema_sheet}"
            )));
        }
        let mut objects = Vec::new();
        collect_objects(spec, spec.root(), &s.data, Path::root(), ty, &mut objects);
        let mine = location_of(spec, object);
        let mut all = Vec::new();
        collect_locations(
            spec,
            spec.root(),
            ty,
            &mut String::new(),
            &mut Vec::new(),
            &mut all,
        );
        let also_used_at = all
            .into_iter()
            .filter(|l| *l != mine)
            .map(|l| format!("{sheet}{l}"))
            .collect();
        Ok(FieldPlaces {
            schema_sheet: schema_sheet.clone(),
            document_path,
            fields: fields.iter().map(|f| f.name.clone()).collect(),
            objects,
            also_used_at,
            xsd: is_xsd,
        })
    }

    /// How many values and formulas deleting the field would remove.
    pub fn delete_field_impact(
        &self,
        sheet: &str,
        object: &Path,
        name: &str,
    ) -> Result<(usize, usize), EditError> {
        let places = self.field_places(sheet, object)?;
        let s = &self.sheets[sheet];
        let mut values = 0;
        let mut formulas = 0;
        for place in &places.objects {
            let field = place.key(name);
            if s.data.get(&field).is_some() {
                values += 1;
            }
            formulas += formulas_at_or_below(s, &field).len();
        }
        Ok((values, formulas))
    }

    /// Everything a command must be able to change, checked before it changes
    /// anything, so that it does not stop half way.
    fn check_objects_free(&self, sheet: &str, places: &FieldPlaces) -> Result<(), EditError> {
        let s = &self.sheets[sheet];
        for place in &places.objects {
            if let Some(formula) = s.formulas.computed_by(place) {
                return Err(EditError::InsideComputed {
                    node: Loc::new(sheet, place.clone()),
                    formula,
                });
            }
            if s.formulas.by_path.contains_key(place) {
                return Err(EditError::Path(format!(
                    "{} is computed by a formula, so its fields cannot be changed",
                    Loc::new(sheet, place.clone())
                )));
            }
        }
        Ok(())
    }

    // ----------------------------------------------------------- commands

    /// Puts the fields of the record in the given order, in the schema and in
    /// every item. No value changes.
    pub fn reorder_fields(
        &mut self,
        sheet: &str,
        object: &Path,
        order: &[String],
    ) -> Result<(), EditError> {
        let places = self.field_places(sheet, object)?;
        let mut sorted_new: Vec<&String> = order.iter().collect();
        let mut sorted_old: Vec<&String> = places.fields.iter().collect();
        sorted_new.sort();
        sorted_old.sort();
        if sorted_new != sorted_old {
            return Err(EditError::Path(
                "the new order must name every field once".to_owned(),
            ));
        }
        if order == places.fields.as_slice() {
            return Ok(());
        }
        if places.xsd {
            // Checked before anything changes.
            self.xsd_edited_type(&places, &XsdChange::Reorder { order })?;
        }
        self.check_objects_free(sheet, &places)?;
        let what = format!(
            "Reorder fields of {sheet}{}",
            location_of(self.spec_of(sheet)?, object)
        );
        self.entry(&what, |calc| {
            let document = if places.xsd {
                calc.xsd_edited_type(&places, &XsdChange::Reorder { order })?
            } else {
                calc.edited_type(&places, |props, _| {
                    let mut ordered = IndexMap::new();
                    for name in order {
                        if let Some(node) = props.get(name) {
                            ordered.insert(name.clone(), node.clone());
                        }
                    }
                    *props = ordered;
                })?
            };
            calc.put(&places.schema_sheet, &places.document_path, document)?;
            for place in &places.objects {
                calc.rewrite_object(sheet, place, |map| {
                    let mut ordered = IndexMap::new();
                    for name in order {
                        if let Some(node) = map.get(name) {
                            ordered.insert(name.clone(), node.clone());
                        }
                    }
                    for (key, node) in map.iter() {
                        ordered.entry(key.clone()).or_insert_with(|| node.clone());
                    }
                    *map = ordered;
                })?;
            }
            Ok(())
        })
    }

    /// Whether *Force object definition* is offered on the node at `path`:
    /// it is a field (or the root) whose type is a record with no fields, or
    /// a field that is an array of such records, and its formula returned a
    /// record (or an array of records) with fields, which the type refused.
    pub fn force_type_offered(&self, sheet: &str, path: &Path) -> bool {
        self.force_type_source(sheet, path).is_some()
    }

    /// The result *Force object definition* defines the type from.
    fn force_type_source(&self, sheet: &str, path: &Path) -> Option<&Value> {
        let root = path.is_root();
        if !root && !matches!(path.segments().last(), Some(Segment::Key(_))) {
            return None;
        }
        let s = self.sheets.get(sheet)?;
        let rejected = s.formulas.by_path.get(path)?.rejected.as_deref()?;
        let ty = s.spec.resolve(path)?.ty;
        let empty = |ty: TypeId| matches!(&s.spec.get(ty).kind, Kind::Object { fields } if fields.is_empty());
        let has_fields = |v: &Value| matches!(v, Value::Object(map) if !map.is_empty());
        let fits = match &s.spec.get(ty).kind {
            Kind::Object { fields } if fields.is_empty() => has_fields(rejected),
            Kind::Array { items, .. } if !root && empty(*items) => match rejected {
                Value::Array(items) => {
                    items.iter().all(|i| matches!(i.value(), Value::Object(_)))
                        && items.iter().any(|i| has_fields(i.value()))
                }
                _ => false,
            },
            _ => false,
        };
        fits.then_some(rejected)
    }

    /// *Force object definition*: gives the field at `path` the type
    /// inferred from what its formula returned, as an import infers one, in
    /// place of its record with no fields. Whether it is required stays as
    /// it was. On the root, the root's record takes the inferred fields. One
    /// entry in the history.
    pub fn force_type(&mut self, sheet: &str, path: &Path) -> Result<(), EditError> {
        let value = self
            .force_type_source(sheet, path)
            .cloned()
            .ok_or_else(|| {
                EditError::Path(format!(
                    "{} is not a record with no fields whose formula returned records",
                    Loc::new(sheet, path.clone())
                ))
            })?;
        let name = match path.segments().last().cloned() {
            Some(Segment::Key(name)) => Some(name),
            None => None,
            Some(_) => {
                return Err(EditError::Path("only a field can be defined".to_owned()));
            }
        };
        let parent = if name.is_some() {
            path.parent().unwrap_or_default()
        } else {
            path.clone()
        };
        let places = self.field_places(sheet, &parent)?;
        if places.xsd {
            return Err(EditError::Path(format!(
                "the type of an XML element is defined on its XSD tab, {}",
                places.schema_sheet
            )));
        }
        let mut schema = cliotree_core::infer::infer_json(&Node::new(value)).schema;
        if let Value::Object(map) = schema.value_mut() {
            map.shift_remove("$schema");
            map.shift_remove("title");
        }
        let what = format!(
            "Force object definition of {}",
            Loc::new(sheet, path.clone())
        );
        self.entry(&what, |calc| {
            let document = calc.edited_type(&places, |props, required| match &name {
                Some(name) => {
                    props.insert(name.clone(), schema);
                }
                // The root: its own record takes the inferred fields.
                None => {
                    if let Some(Value::Object(fields)) = schema
                        .value()
                        .as_object()
                        .and_then(|m| m.get("properties"))
                        .map(Node::value)
                    {
                        *props = fields.clone();
                    }
                    if let Some(Value::Array(names)) = schema
                        .value()
                        .as_object()
                        .and_then(|m| m.get("required"))
                        .map(Node::value)
                    {
                        *required = names.clone();
                    }
                }
            })?;
            calc.put(&places.schema_sheet, &places.document_path, document)
        })
    }

    /// Adds an optional or required field of a simple type to the end of the
    /// record. Existing items get no value for it.
    pub fn add_field(
        &mut self,
        sheet: &str,
        object: &Path,
        name: &str,
        kind: FieldKind,
        required: bool,
    ) -> Result<(), EditError> {
        let places = self.field_places(sheet, object)?;
        check_name(name)?;
        if places.fields.iter().any(|f| f == name) {
            return Err(EditError::Path(format!(
                "there is a field called {name} already"
            )));
        }
        let what = format!("Add field \"{name}\"");
        let change = XsdChange::Add {
            name,
            kind,
            required,
        };
        let xsd_document = if places.xsd {
            Some(self.xsd_edited_type(&places, &change)?)
        } else {
            None
        };
        self.entry(&what, |calc| {
            let document = match xsd_document {
                Some(document) => document,
                None => calc.edited_type(&places, |props, required_list| {
                    props.insert(name.to_owned(), kind.schema());
                    if required {
                        required_list.push(text(name));
                    }
                })?,
            };
            calc.put(&places.schema_sheet, &places.document_path, document)
        })
    }

    /// Gives a field a new name in the schema and in every item. A formula on
    /// the field goes with it; formulas that read it by the old name are not
    /// rewritten and show `#REF!`.
    pub fn rename_field(
        &mut self,
        sheet: &str,
        object: &Path,
        from: &str,
        to: &str,
    ) -> Result<(), EditError> {
        let places = self.field_places(sheet, object)?;
        if !places.fields.iter().any(|f| f == from) {
            return Err(EditError::Path(format!("there is no field called {from}")));
        }
        check_name(to)?;
        if from == to {
            return Ok(());
        }
        if places.fields.iter().any(|f| f == to) {
            return Err(EditError::Path(format!(
                "there is a field called {to} already"
            )));
        }
        let xsd_document = if places.xsd {
            Some(self.xsd_edited_type(&places, &XsdChange::Rename { from, to })?)
        } else {
            None
        };
        self.check_objects_free(sheet, &places)?;
        let what = format!("Rename field \"{from}\" to \"{to}\"");
        let renamed = |key: &String| {
            if key == from {
                to.to_owned()
            } else {
                key.clone()
            }
        };
        self.entry(&what, |calc| {
            let document = match xsd_document {
                Some(document) => document,
                None => calc.edited_type(&places, |props, required_list| {
                    *props = props.iter().map(|(k, v)| (renamed(k), v.clone())).collect();
                    for entry in required_list.iter_mut() {
                        if entry.value().as_str() == Some(from) {
                            *entry.value_mut() = Value::Str(to.to_owned());
                        }
                    }
                })?,
            };
            // The formulas on the field, found before the data changes.
            let mut moves = Vec::new();
            for place in &places.objects {
                let old = place.key(from);
                for path in formulas_at_or_below(&calc.sheets[sheet], &old) {
                    let mut moved = place.key(to).segments().to_vec();
                    moved.extend_from_slice(&path.segments()[old.len()..]);
                    let text = calc.formula_text(sheet, &path).unwrap_or_default();
                    moves.push((path, Path::from(moved), text));
                }
            }
            calc.put(&places.schema_sheet, &places.document_path, document)?;
            for place in &places.objects {
                calc.rewrite_object(sheet, place, |map| {
                    *map = map.iter().map(|(k, v)| (renamed(k), v.clone())).collect();
                })?;
            }
            for (old, _, _) in &moves {
                calc.remove_formula(sheet, old)?;
            }
            moves.sort_by_key(|(_, new, _)| new.len());
            for (_, new, text) in &moves {
                calc.set_formula(sheet, new, text)?;
            }
            Ok(())
        })
    }

    /// Removes a field from the schema and its values from every item, with the
    /// formulas on it. Formulas that read it show `#REF!`.
    pub fn delete_field(
        &mut self,
        sheet: &str,
        object: &Path,
        name: &str,
    ) -> Result<(), EditError> {
        let places = self.field_places(sheet, object)?;
        if !places.fields.iter().any(|f| f == name) {
            return Err(EditError::Path(format!("there is no field called {name}")));
        }
        let xsd_document = if places.xsd {
            Some(self.xsd_edited_type(&places, &XsdChange::Delete { name })?)
        } else {
            None
        };
        self.check_objects_free(sheet, &places)?;
        let what = format!("Delete field \"{name}\"");
        self.entry(&what, |calc| {
            let document = match xsd_document {
                Some(document) => document,
                None => calc.edited_type(&places, |props, required_list| {
                    props.shift_remove(name);
                    required_list.retain(|n| n.value().as_str() != Some(name));
                })?,
            };
            for place in &places.objects {
                let field = place.key(name);
                for path in formulas_at_or_below(&calc.sheets[sheet], &field) {
                    calc.remove_formula(sheet, &path)?;
                }
            }
            for place in &places.objects {
                calc.rewrite_object(sheet, place, |map| {
                    map.shift_remove(name);
                })?;
            }
            calc.put(&places.schema_sheet, &places.document_path, document)
        })
    }

    // ---------------------------------------------------------- helpers

    fn spec_of(&self, sheet: &str) -> Result<&TypeSpec, EditError> {
        self.sheets
            .get(sheet)
            .map(|s| &*s.spec)
            .ok_or_else(|| EditError::NoSheet(sheet.to_owned()))
    }

    /// Runs `body` as one entry, which ends however it ends.
    fn entry<T>(
        &mut self,
        description: &str,
        body: impl FnOnce(&mut Calc) -> Result<T, EditError>,
    ) -> Result<T, EditError> {
        self.begin_entry(description);
        let result = body(self);
        self.end_entry();
        result
    }

    /// The value of the object type's schema, with `change` applied to its
    /// `properties` and to its `required` list (empty if there is none).
    fn edited_type(
        &self,
        places: &FieldPlaces,
        change: impl FnOnce(&mut IndexMap<String, Node>, &mut Vec<Node>),
    ) -> Result<Value, EditError> {
        let node = self
            .sheets
            .get(&places.schema_sheet)
            .and_then(|d| d.data.get(&places.document_path))
            .ok_or_else(|| EditError::Path("the type is not in the schema any more".to_owned()))?;
        let Value::Object(map) = node.value() else {
            return Err(EditError::Path(
                "the type is not a schema object".to_owned(),
            ));
        };
        let mut map = map.clone();
        let mut props = match map.get("properties").map(Node::value) {
            Some(Value::Object(props)) => props.clone(),
            _ => IndexMap::new(),
        };
        let mut required = match map.get("required").map(Node::value) {
            Some(Value::Array(items)) => items.clone(),
            _ => Vec::new(),
        };
        change(&mut props, &mut required);
        replace_value(&mut map, "properties", Value::Object(props));
        // A list that was not there stays out unless something is in it.
        if !required.is_empty() || map.contains_key("required") {
            replace_value(&mut map, "required", Value::Array(required));
        }
        Ok(Value::Object(map))
    }

    /// Replaces the value of an existing node, keeping its identity, as one
    /// recorded edit.
    fn put(&mut self, sheet: &str, path: &Path, value: Value) -> Result<(), EditError> {
        let s = self.sheet_mut(sheet)?;
        let before = s
            .data
            .get(path)
            .cloned()
            .ok_or_else(|| EditError::Path(format!("there is no node at {path}")))?;
        s.data
            .set(path, value)
            .map_err(|e| EditError::Path(e.to_string()))?;
        let after = s.data.get(path).cloned().expect("it was just set");
        self.changed(sheet, path);
        self.history.record(
            "Edit".to_owned(),
            vec![Op::Replace {
                sheet: sheet.to_owned(),
                path: path.clone(),
                before,
                after,
            }],
            None,
        );
        Ok(())
    }

    /// Changes the fields of one object in the data, keeping the nodes that
    /// stay.
    fn rewrite_object(
        &mut self,
        sheet: &str,
        path: &Path,
        change: impl FnOnce(&mut IndexMap<String, Node>),
    ) -> Result<(), EditError> {
        let Some(Value::Object(map)) = self.sheets[sheet].data.get(path).map(Node::value) else {
            return Ok(());
        };
        let mut map = map.clone();
        change(&mut map);
        self.put(sheet, path, Value::Object(map))
    }
}

/// Sets a key of a schema object, keeping its place if it has one.
fn replace_value(map: &mut IndexMap<String, Node>, key: &str, value: Value) {
    match map.get_mut(key) {
        Some(node) => *node.value_mut() = value,
        None => {
            map.insert(key.to_owned(), Node::new(value));
        }
    }
}

fn check_name(name: &str) -> Result<(), EditError> {
    if name.trim().is_empty() {
        return Err(EditError::Path("a field needs a name".to_owned()));
    }
    if name != name.trim() || name.chars().any(char::is_control) {
        return Err(EditError::Path(
            "a field name cannot start or end with a space or hold control characters".to_owned(),
        ));
    }
    Ok(())
}

/// The formulas at `path` and below it.
fn formulas_at_or_below(sheet: &Sheet, path: &Path) -> Vec<Path> {
    let mut out = sheet.formulas.below(path);
    if sheet.formulas.by_path.contains_key(path) {
        out.push(path.clone());
    }
    out
}

/// `#/properties/items/items` as the path `properties.items.items`.
fn pointer_to_path(pointer: &str) -> Option<Path> {
    let rest = pointer.strip_prefix('#')?;
    let mut path = Path::root();
    for part in rest.split('/').skip(1) {
        path = path.key(part.replace("~1", "/").replace("~0", "~"));
    }
    Some(path)
}

/// The objects of type `target` in the data.
fn collect_objects(
    spec: &TypeSpec,
    ty: TypeId,
    node: &Node,
    path: Path,
    target: TypeId,
    out: &mut Vec<Path>,
) {
    match (&spec.get(ty).kind, node.value()) {
        (Kind::Object { fields }, Value::Object(map)) => {
            if ty == target {
                out.push(path.clone());
            }
            for field in fields {
                if let Some(child) = map.get(&field.name) {
                    collect_objects(spec, field.ty, child, path.key(&field.name), target, out);
                }
            }
        }
        (Kind::Array { items, .. }, Value::Array(children)) => {
            for (i, child) in children.iter().enumerate() {
                collect_objects(spec, *items, child, path.index(i), target, out);
            }
        }
        (Kind::Map { value }, Value::Object(map)) => {
            for (key, child) in map {
                collect_objects(spec, *value, child, path.key(key), target, out);
            }
        }
        _ => {}
    }
}

/// Where in the schema a data path leads, as `.items[].sku`: each array item is
/// `[]` and each map entry `.*`.
fn location_of(spec: &TypeSpec, path: &Path) -> String {
    let mut out = String::new();
    let mut ty = spec.root();
    for segment in path.segments() {
        match (&spec.get(ty).kind, segment) {
            (Kind::Object { fields }, Segment::Key(name)) => {
                out.push('.');
                out.push_str(name);
                match fields.iter().find(|f| &f.name == name) {
                    Some(f) => ty = f.ty,
                    None => break,
                }
            }
            (Kind::Map { value }, Segment::Key(_)) => {
                out.push_str(".*");
                ty = *value;
            }
            (Kind::Array { items, .. }, Segment::Index(_)) => {
                out.push_str("[]");
                ty = *items;
            }
            _ => break,
        }
    }
    out
}

/// Every place in the schema that has type `target`, written as
/// [`location_of`] writes them.
fn collect_locations(
    spec: &TypeSpec,
    ty: TypeId,
    target: TypeId,
    here: &mut String,
    stack: &mut Vec<TypeId>,
    out: &mut Vec<String>,
) {
    if ty == target {
        out.push(here.clone());
    }
    if stack.contains(&ty) {
        return;
    }
    stack.push(ty);
    let length = here.len();
    match &spec.get(ty).kind {
        Kind::Object { fields } => {
            for field in fields {
                here.push('.');
                here.push_str(&field.name);
                collect_locations(spec, field.ty, target, here, stack, out);
                here.truncate(length);
            }
        }
        Kind::Array { items, .. } => {
            here.push_str("[]");
            collect_locations(spec, *items, target, here, stack, out);
            here.truncate(length);
        }
        Kind::Map { value } => {
            here.push_str(".*");
            collect_locations(spec, *value, target, here, stack, out);
            here.truncate(length);
        }
        _ => {}
    }
    stack.pop();
}
