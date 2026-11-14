//! Schema tabs, binding a tab to one, auto-update, and the field commands that
//! a data tab can issue (DESIGN.md, "Schema binding" and "Editing the schema
//! from a data tab").
//!
//! A schema tab holds a schema document as ordinary data, described by the
//! bundled meta-schema. Whenever it changes, by an edit, an undo or a command,
//! the tabs bound to it take the newly lowered type ([`Calc::relower_if_schema`]),
//! so a command is only ever an edit of the document together with the edits of
//! the data and formulas that go with it, recorded as one entry.

use gossia_core::xml::XmlDocument;
use gossia_core::{Kind, TypeId, jsonschema, metaschema, xsd};
use gossia_i18n::tr;
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
            FieldKind::Object => Some(tr!(
                "An object with no fields yet: add them to it in the same way."
            )),
            FieldKind::Array => Some(tr!(
                "A list of objects with no fields yet: add an item, then add fields to it."
            )),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Text => tr!("text"),
            FieldKind::Integer => tr!("whole number"),
            FieldKind::Number => tr!("number"),
            FieldKind::Boolean => tr!("yes or no"),
            FieldKind::Date => tr!("date"),
            FieldKind::DateTime => tr!("date and time"),
            FieldKind::Object => tr!("object"),
            FieldKind::Array => tr!("array"),
        }
    }

    /// The types an array's items can have: any but an array.
    pub const ITEMS: [FieldKind; 7] = [
        FieldKind::Text,
        FieldKind::Integer,
        FieldKind::Number,
        FieldKind::Boolean,
        FieldKind::Date,
        FieldKind::DateTime,
        FieldKind::Object,
    ];

    /// The schema of an array whose items are of type `items`.
    fn array_schema(items: FieldKind) -> Node {
        let mut fields = IndexMap::new();
        fields.insert("type".to_owned(), text("array"));
        fields.insert("items".to_owned(), items.schema());
        Node::new(Value::Object(fields))
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
            FieldKind::Array => return FieldKind::array_schema(FieldKind::Object),
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
    /// The type is an item of an ordered list (`#children`): its fields are
    /// the elements of an `xs:choice`, where the document path leads.
    pub choice: bool,
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
    /// types (an XML document: see `gossia_core::xml`). `document` says how
    /// the XML was written; [`gossia_core::xsd::default_document`] is the one
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
    /// copybook meta-schema types (see `gossia_core::copybook::to_node`).
    /// It is kept as JSON5, which holds the comments of its items.
    pub fn add_copybook_sheet(
        &mut self,
        name: impl Into<String>,
        tree: Node,
    ) -> Result<(), EditError> {
        let name = name.into();
        self.add_sheet(name.clone(), metaschema::copybook_meta().clone(), tree)?;
        let sheet = self.sheet_mut(&name)?;
        sheet.format = gossia_core::dataformat::DataFormat::Json5;
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
            return Err(EditError::Path(tr!(
                "{schema_sheet} is not a schema tab",
                schema_sheet = schema_sheet
            )));
        }
        if self.is_schema_sheet(sheet) {
            return Err(EditError::Path(tr!(
                "{sheet} is a schema tab, which is described by the built-in schema",
                sheet = sheet
            )));
        }
        if self.auto_update(schema_sheet)
            && self
                .schema_users(schema_sheet)
                .iter()
                .any(|user| user != sheet)
        {
            return Err(EditError::Path(tr!(
                "{schema_sheet} has auto-update switched on, so only one tab may use it; \
                 switch that off first",
                schema_sheet = schema_sheet
            )));
        }
        let record = self.binding_record(sheet, schema_sheet);
        let spec = self.lowered_for(schema_sheet, record.as_deref())?;
        let s = self.sheet_mut(sheet)?;
        s.spec = Arc::new(spec);
        s.record = record;
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

    /// Where the node at `path` of `sheet` is defined: the schema tab the
    /// tab is bound to, and the place in that tab's tree (*Go to schema
    /// definition*). A field is its own entry in its record (JSON Schema's
    /// `properties`, an XSD `element` or `attribute`, a copybook item); a node
    /// whose place cannot be told is its nearest container that can, and then
    /// the top of the schema tab. None for a tab not bound to a schema tab.
    pub fn definition_of(&self, sheet: &str, path: &Path) -> Option<(String, Path)> {
        let s = self.sheets.get(sheet)?;
        let SchemaRole::Bound(schema_sheet) = &s.schema else {
            return None;
        };
        let document = &self.sheets.get(schema_sheet)?.data;
        let format = self.schema_format(schema_sheet)?;
        let mut at = path.clone();
        loop {
            if let Some(place) = place_in_schema(format, document, &s.spec, &at) {
                return Some((schema_sheet.clone(), place));
            }
            if at.pop().is_none() {
                return Some((schema_sheet.clone(), Path::root()));
            }
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
            return Some(tr!(
                "{schema_sheet} is not a schema tab",
                schema_sheet = schema_sheet
            ));
        }
        match self.schema_users(schema_sheet).len() {
            1 => None,
            0 => Some(tr!("no tab uses this schema, so there is nothing to update").to_owned()),
            n => Some(tr!(
                "{n} tabs use this schema; auto-update needs exactly one, so that a change \
                 can never reach another tab",
                n = n
            )),
        }
    }

    /// Switches auto-update of a schema tab on or off, as an entry that can be
    /// undone.
    pub fn set_auto_update(&mut self, schema_sheet: &str, on: bool) -> Result<(), EditError> {
        if !self.is_schema_sheet(schema_sheet) {
            return Err(EditError::Path(tr!(
                "{schema_sheet} is not a schema tab",
                schema_sheet = schema_sheet
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
            if on {
                tr!(
                    "Switch auto-update on for {schema_sheet}",
                    schema_sheet = schema_sheet
                )
            } else {
                tr!(
                    "Switch auto-update off for {schema_sheet}",
                    schema_sheet = schema_sheet
                )
            },
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
            _ => {
                return Err(EditError::Path(tr!(
                    "{sheet} is not a schema tab",
                    sheet = sheet
                )));
            }
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
                let main = self.xsd_element(schema_sheet).expect("the tab is there");
                // An `xs:import` names the schema tab of its namespace.
                let imports = |location: &str| {
                    let tab = self.xsd_import_tab(location)?;
                    self.xsd_element(&tab)
                };
                // `record` is the global element a tab holds, if not the first.
                xsd::lower_with(&main, &imports, record).map_err(|e| e.to_string())
            }
            SchemaRole::Document {
                format: SchemaFormat::Copybook,
                ..
            } => gossia_core::copybook::lower_node_record(&s.data, record),
            _ => jsonschema::lower(&s.data).map_err(|e| e.to_string()),
        };
        lowered.map_err(|e| {
            EditError::Path(tr!(
                "the schema in {schema_sheet} cannot be used: {e}",
                schema_sheet = schema_sheet,
                e = e
            ))
        })
    }

    pub(super) fn relower_if_schema(&mut self, sheet: &str) {
        if self.is_schema_sheet(sheet) {
            self.relower_schema(sheet);
        }
    }

    /// What a tab holds of `schema_sheet` when bound to it: for a copybook
    /// its 01 record, as before; for an XSD the global element of the name of
    /// the tab's root now, when that is not the first (None is the first).
    pub(super) fn binding_record(&self, sheet: &str, schema_sheet: &str) -> Option<String> {
        let s = self.sheets.get(sheet)?;
        if self.schema_format(schema_sheet) != Some(SchemaFormat::Xsd) {
            return s.record.clone();
        }
        let root = &s.spec.xml()?.root;
        let roots = self.xsd_roots(schema_sheet);
        roots
            .iter()
            .position(|r| r == root)
            .is_some_and(|i| i > 0)
            .then(|| root.clone())
    }

    /// The schema document of an XSD tab, as elements.
    pub fn xsd_element(&self, schema_sheet: &str) -> Option<gossia_core::xmltree::Element> {
        let s = self.sheets.get(schema_sheet)?;
        let declarations = s
            .xml
            .as_ref()
            .map(|d| d.declarations.clone())
            .unwrap_or_else(|| xsd::default_document().declarations);
        Some(xsd::schema_element(&s.data, &declarations))
    }

    /// The XSD tab an `xs:import`'s `schemaLocation` names: the tab of that
    /// name, or of the file's name without `.xsd` (`dc.xsd`, `schemas/dc.xsd`
    /// for a tab `dc`). Tab names ignore case.
    pub fn xsd_import_tab(&self, location: &str) -> Option<String> {
        let file = location.rsplit(['/', '\\']).next().unwrap_or(location);
        let stem = match file.len().checked_sub(4) {
            Some(at) if file.is_char_boundary(at) && file[at..].eq_ignore_ascii_case(".xsd") => {
                &file[..at]
            }
            _ => file,
        };
        [location, stem].into_iter().find_map(|wanted| {
            self.sheets
                .iter()
                .find(|(name, s)| {
                    name.eq_ignore_ascii_case(wanted)
                        && matches!(
                            s.schema,
                            SchemaRole::Document {
                                format: SchemaFormat::Xsd,
                                ..
                            }
                        )
                })
                .map(|(name, _)| name.clone())
        })
    }

    /// The `schemaLocation`s of an XSD tab's imports, with where each is in
    /// its tree.
    pub(super) fn xsd_imports(&self, schema_sheet: &str) -> Vec<(Path, String)> {
        let Some(s) = self.sheets.get(schema_sheet) else {
            return Vec::new();
        };
        if self.schema_format(schema_sheet) != Some(SchemaFormat::Xsd) {
            return Vec::new();
        }
        let list = Path::root().key("import");
        let items = match s.data.get(&list).map(Node::value) {
            Some(Value::Array(items)) => items.clone(),
            _ => return Vec::new(),
        };
        items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let at = list.index(i).key("@schemaLocation");
                let location = item.get(&Path::root().key("@schemaLocation"))?;
                Some((at, location.value().as_str()?.to_owned()))
            })
            .collect()
    }

    /// The XSD tabs that import `schema_sheet`, directly.
    pub fn xsd_importers(&self, schema_sheet: &str) -> Vec<String> {
        self.sheet_names()
            .filter(|tab| *tab != schema_sheet)
            .filter(|tab| {
                self.xsd_imports(tab).iter().any(|(_, location)| {
                    self.xsd_import_tab(location)
                        .is_some_and(|t| t.eq_ignore_ascii_case(schema_sheet))
                })
            })
            .map(str::to_owned)
            .collect()
    }

    /// The `targetNamespace` of an XSD tab: None for one in no namespace, or
    /// for any other tab.
    pub fn xsd_namespace(&self, schema_sheet: &str) -> Option<String> {
        if self.schema_format(schema_sheet) != Some(SchemaFormat::Xsd) {
            return None;
        }
        let s = self.sheets.get(schema_sheet)?;
        s.data
            .get(&Path::root().key("@targetNamespace"))
            .and_then(|n| n.value().as_str().map(str::to_owned))
    }

    /// The names of the global elements of an XSD tab, in order: the roots its
    /// documents may have. None for any other tab.
    pub fn xsd_roots(&self, schema_sheet: &str) -> Vec<String> {
        if self.schema_format(schema_sheet) != Some(SchemaFormat::Xsd) {
            return Vec::new();
        }
        let Some(s) = self.sheets.get(schema_sheet) else {
            return Vec::new();
        };
        let name_of = |item: &Node| {
            item.get(&Path::root().key("@name"))
                .and_then(|n| n.value().as_str().map(str::to_owned))
        };
        match s.data.get(&Path::root().key("element")).map(Node::value) {
            Some(Value::Array(items)) => items.iter().filter_map(name_of).collect(),
            Some(Value::Object(_)) => s
                .data
                .get(&Path::root().key("element"))
                .and_then(name_of)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Lowers the document again and gives the result to the tabs bound to
    /// it, and does the same for the XSD tabs that import it (and those that
    /// import them), since their types may hold its types.
    fn relower_schema(&mut self, schema_sheet: &str) {
        self.relower_one(schema_sheet);
        if self.schema_format(schema_sheet) != Some(SchemaFormat::Xsd) {
            return;
        }
        let mut done = vec![schema_sheet.to_lowercase()];
        let mut todo = self.xsd_importers(schema_sheet);
        while let Some(tab) = todo.pop() {
            if done.contains(&tab.to_lowercase()) {
                continue;
            }
            done.push(tab.to_lowercase());
            self.relower_one(&tab);
            todo.extend(self.xsd_importers(&tab));
        }
    }

    /// Lowers the document again and gives the result to the tabs bound to it.
    /// A document that does not lower leaves them as they were.
    fn relower_one(&mut self, schema_sheet: &str) {
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
            // An XSD of global attributes only is there to be imported
            // (an `android:` namespace), not bound: its problems show on
            // the tabs that import it.
            Err(_)
                if self.schema_format(schema_sheet) == Some(SchemaFormat::Xsd)
                    && self.xsd_roots(schema_sheet).is_empty() =>
            {
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
            return Err(EditError::Path(tr!(
                "the type of {sheet} is not defined by a schema tab, so its fields cannot be \
                 changed from here",
                sheet = sheet
            )));
        };
        let is_xsd = self.schema_format(schema_sheet) == Some(SchemaFormat::Xsd);
        if self.schema_format(schema_sheet) == Some(SchemaFormat::Copybook) {
            return Err(EditError::Path(tr!(
                "the fields of {sheet} are items of the copybook in {schema_sheet}, which are \
                 changed there",
                sheet = sheet,
                schema_sheet = schema_sheet
            )));
        }
        if !self.auto_update(schema_sheet) {
            return Err(EditError::Path(tr!(
                "switch on auto-update on {schema_sheet} to change the schema from {sheet}",
                schema_sheet = schema_sheet,
                sheet = sheet
            )));
        }
        if let Some(error) = self.schema_error(schema_sheet) {
            return Err(EditError::Path(tr!(
                "the schema in {schema_sheet} has an error: {error}",
                schema_sheet = schema_sheet,
                error = error
            )));
        }
        let spec = &s.spec;
        let at = Loc::new(sheet, object.clone());
        let ty = spec
            .resolve(object)
            .ok_or_else(|| EditError::NotInSchema(at.clone()))?
            .ty;
        let (fields, choice) = match &spec.get(ty).kind {
            Kind::Object { fields } => (fields, false),
            Kind::Choice { fields } if is_xsd => (fields, true),
            _ => {
                return Err(EditError::Path(tr!(
                    "{at} is not a record, so it has no fields to change",
                    at = at
                )));
            }
        };
        let source = &spec.get(ty).source;
        if is_xsd && let Some(place) = imported_from(source) {
            let tab = self
                .xsd_import_tab(place)
                .unwrap_or_else(|| place.to_owned());
            return Err(EditError::Path(tr!(
                "the type of {at} is written in {tab}, which {schema_sheet} imports; change it there",
                at = at,
                tab = tab,
                schema_sheet = schema_sheet
            )));
        }
        let document_path = if is_xsd {
            self.sheets
                .get(schema_sheet)
                .and_then(|d| xsd_fields::xsd_type_path(&d.data, source))
        } else {
            pointer_to_path(source)
        }
        .ok_or_else(|| {
            EditError::Path(tr!(
                "cannot tell where {at} is written in the schema",
                at = at
            ))
        })?;
        let document_ok = self
            .sheets
            .get(schema_sheet)
            .and_then(|d| d.data.get(&document_path))
            .is_some_and(|n| n.value().as_object().is_some());
        if !document_ok {
            return Err(EditError::Path(tr!(
                "cannot find the type of {at} in {schema_sheet}",
                at = at,
                schema_sheet = schema_sheet
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
            choice,
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
                return Err(EditError::Path(tr!(
                    "{place} is computed by a formula, so its fields cannot be changed",
                    place = Loc::new(sheet, place.clone())
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
                tr!("the new order must name every field once").to_owned(),
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
        let what = tr!(
            "Reorder fields of {place}",
            place = format!("{sheet}{}", location_of(self.spec_of(sheet)?, object))
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

    /// Whether *Force type definition* is offered on the node at `path`: it
    /// is a field (or the root) whose type is a record with no fields, or a
    /// field that is an array of such records, and its formula returned a
    /// record with fields, or an array of records with fields or of simple
    /// values (text, numbers), which the type refused.
    pub fn force_type_offered(&self, sheet: &str, path: &Path) -> bool {
        self.force_type_source(sheet, path).is_some()
            || self.force_any_target(sheet, path).is_some()
    }

    /// Whether *Force type definition* on the node at `path` defines a type
    /// of any value from what the node holds (rather than a record with no
    /// fields from a refused result).
    pub fn force_type_of_any(&self, sheet: &str, path: &Path) -> bool {
        self.force_type_source(sheet, path).is_none()
            && self.force_any_target(sheet, path).is_some()
    }

    /// The schema tab *Force type definition* of an `any` would change from
    /// `sheet`, or why it cannot be changed from here.
    pub fn force_any_schema_tab(&self, sheet: &str) -> Result<String, EditError> {
        self.json_schema_tab_to_edit(sheet)
    }

    /// The schema tab of `sheet`, when a command may change it from there: a
    /// JSON Schema tab with auto-update on and no error.
    fn json_schema_tab_to_edit(&self, sheet: &str) -> Result<String, EditError> {
        let s = self
            .sheets
            .get(sheet)
            .ok_or_else(|| EditError::NoSheet(sheet.to_owned()))?;
        let SchemaRole::Bound(schema_sheet) = &s.schema else {
            return Err(EditError::Path(tr!(
                "the type of {sheet} is not defined by a schema tab, so its fields cannot be \
                 changed from here",
                sheet = sheet
            )));
        };
        if self.schema_format(schema_sheet) != Some(SchemaFormat::Json) {
            return Err(EditError::Path(tr!(
                "the type of an XML element is defined on its XSD tab, {schema_sheet}",
                schema_sheet = schema_sheet
            )));
        }
        if !self.auto_update(schema_sheet) {
            return Err(EditError::Path(tr!(
                "switch on auto-update on {schema_sheet} to change the schema from {sheet}",
                schema_sheet = schema_sheet,
                sheet = sheet
            )));
        }
        if let Some(error) = self.schema_error(schema_sheet) {
            return Err(EditError::Path(tr!(
                "the schema in {schema_sheet} has an error: {error}",
                schema_sheet = schema_sheet,
                error = error
            )));
        }
        Ok(schema_sheet.clone())
    }

    /// For *Force type definition* on a node of any value (`{}`, or an array
    /// without `items`): where the `any` is written in the schema document,
    /// and the schema inferred from what the node holds, to put there. On an
    /// array of any values (or one of its items) that is the items' schema,
    /// from all of them. None when the node holds nothing to define it from.
    fn force_any_target(&self, sheet: &str, path: &Path) -> Option<(Path, Node)> {
        let s = self.sheets.get(sheet)?;
        let spec = &s.spec;
        let is_any = |ty: TypeId| matches!(spec.get(ty).kind, Kind::Any);
        let mut at = path.clone();
        let mut ty = spec.resolve(&at)?.ty;
        // An item of an array of any values: the array, from all its items.
        if is_any(ty)
            && matches!(at.segments().last(), Some(Segment::Index(_)))
            && let Some(parent) = at.parent()
            && let Some(r) = spec.resolve(&parent)
            && matches!(spec.get(r.ty).kind, Kind::Array { items, .. } if is_any(items))
        {
            at = parent;
            ty = r.ty;
        }
        let value = s.data.get(&at)?.value();
        let (target, from_items) = match &spec.get(ty).kind {
            Kind::Any => (ty, false),
            Kind::Array { items, .. } if is_any(*items) => (*items, true),
            _ => return None,
        };
        let holds_something = match value {
            Value::Null | Value::Error(_) => false,
            Value::Array(items) => items
                .iter()
                .any(|i| !matches!(i.value(), Value::Null | Value::Error(_))),
            Value::Object(fields) => !fields.is_empty(),
            _ => true,
        };
        if !holds_something || (from_items && !matches!(value, Value::Array(_))) {
            return None;
        }
        let document_path = pointer_to_path(&spec.get(target).source)?;
        let mut schema = gossia_core::infer::infer_json(&Node::new(value.clone())).schema;
        if from_items {
            schema = schema.value().as_object()?.get("items")?.clone();
        }
        if let Value::Object(map) = schema.value_mut() {
            map.shift_remove("$schema");
            map.shift_remove("title");
        }
        all_optional(&mut schema);
        Some((document_path, schema))
    }

    /// *Force type definition* on a node of any value: see
    /// [`Calc::force_any_target`]. What was written beside the `any` that
    /// is not part of a type (a title, a description, and at the root
    /// `$schema`, `$id` and `$defs`) is kept. One entry in the history.
    fn force_any(&mut self, sheet: &str, path: &Path) -> Result<(), EditError> {
        let (document_path, schema) = self.force_any_target(sheet, path).ok_or_else(|| {
            EditError::Path(tr!(
                "{place} is not a record with no fields, or a list of them, whose formula returned what could define it",
                place = Loc::new(sheet, path.clone())
            ))
        })?;
        let schema_sheet = self.json_schema_tab_to_edit(sheet)?;
        let Value::Object(mut map) = schema.value().clone() else {
            return Err(EditError::Path(
                tr!("the type is not a schema object").to_owned(),
            ));
        };
        let before = self
            .sheets
            .get(&schema_sheet)
            .and_then(|d| d.data.get(&document_path))
            .and_then(|n| n.value().as_object().cloned());
        if let Some(before) = before {
            let mut kept: IndexMap<String, Node> = before
                .into_iter()
                .filter(|(k, _)| {
                    matches!(
                        k.as_str(),
                        "$schema" | "$id" | "$defs" | "title" | "description"
                    )
                })
                .collect();
            kept.extend(map);
            map = kept;
        }
        let what = tr!(
            "Force type definition of {place}",
            place = Loc::new(sheet, path.clone())
        );
        self.entry(&what, |calc| {
            calc.set_value(&schema_sheet, &document_path, Value::Object(map))
        })
    }

    /// What a formula returned for the node at `path`, when the node is of a
    /// simple type (text, a number, a date, a choice of values) and refused
    /// it (`#TYPE!`): the whole result of the node's own formula, or, inside
    /// another formula's result, the part of it at this node that did not
    /// fit (randomuser's text `postcode` in an integer field). None otherwise.
    pub fn refused_simple_value(&self, sheet: &str, path: &Path) -> Option<&Value> {
        let s = self.sheets.get(sheet)?;
        let rejected = match s.formulas.by_path.get(path) {
            Some(formula) => formula.rejected.as_deref()?,
            None => {
                let owner = s.formulas.computed_by(path)?;
                let formula = s.formulas.by_path.get(&owner)?;
                if !formula.misfits.contains(path) {
                    return None;
                }
                let mut part = formula.rejected.as_deref()?;
                for segment in &path.segments()[owner.len()..] {
                    part = match (segment, part) {
                        (Segment::Key(k), Value::Object(fields)) => fields.get(k)?.value(),
                        (Segment::Index(i), Value::Array(items)) => items.get(*i)?.value(),
                        _ => return None,
                    };
                }
                part
            }
        };
        let ty = s.spec.resolve(path)?.ty;
        matches!(
            s.spec.get(ty).kind,
            Kind::String(_)
                | Kind::Integer(_)
                | Kind::Number(_)
                | Kind::Bool
                | Kind::Date
                | Kind::DateTime
                | Kind::Enum { .. }
        )
        .then_some(rejected)
    }

    /// The result *Force type definition* defines the type from.
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
            // Records with fields, or values that are not records at all
            // (text, numbers): either way the items' type can be defined.
            Kind::Array { items, .. } if !root && empty(*items) => match rejected {
                Value::Array(items) => {
                    let records = items.iter().all(|i| matches!(i.value(), Value::Object(_)));
                    let scalars = items.iter().all(|i| {
                        !matches!(i.value(), Value::Object(_) | Value::Array(_) | Value::Null)
                    });
                    !items.is_empty()
                        && ((records && items.iter().any(|i| has_fields(i.value()))) || scalars)
                }
                _ => false,
            },
            _ => false,
        };
        fits.then_some(rejected)
    }

    /// *Force type definition*: gives the field at `path` the type
    /// inferred from what its formula returned, as an import infers one, in
    /// place of its record with no fields. Whether it is required stays as
    /// it was. On the root, the root's record takes the inferred fields. One
    /// entry in the history.
    pub fn force_type(&mut self, sheet: &str, path: &Path) -> Result<(), EditError> {
        if self.force_type_of_any(sheet, path) {
            return self.force_any(sheet, path);
        }
        let value = self
            .force_type_source(sheet, path)
            .cloned()
            .ok_or_else(|| {
                EditError::Path(tr!(
                    "{place} is not a record with no fields, or a list of them, whose formula returned what could define it",
                    place = Loc::new(sheet, path.clone())
                ))
            })?;
        let name = match path.segments().last().cloned() {
            Some(Segment::Key(name)) => Some(name),
            None => None,
            Some(_) => {
                return Err(EditError::Path(
                    tr!("only a field can be defined").to_owned(),
                ));
            }
        };
        let parent = if name.is_some() {
            path.parent().unwrap_or_default()
        } else {
            path.clone()
        };
        let places = self.field_places(sheet, &parent)?;
        if places.xsd {
            return Err(EditError::Path(tr!(
                "the type of an XML element is defined on its XSD tab, {schema_sheet}",
                schema_sheet = places.schema_sheet
            )));
        }
        let mut schema = gossia_core::infer::infer_json(&Node::new(value)).schema;
        if let Value::Object(map) = schema.value_mut() {
            map.shift_remove("$schema");
            map.shift_remove("title");
        }
        // One result does not show which fields every result has (a source
        // may answer differently each time), so every field found is optional.
        all_optional(&mut schema);
        let what = tr!(
            "Force type definition of {place}",
            place = Loc::new(sheet, path.clone())
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
                    required.clear();
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
        self.add_field_of(sheet, object, name, kind, FieldKind::Object, required)
    }

    /// Adds an array field whose items are of type `items` (an object with no
    /// fields yet, or a simple type), as [`Calc::add_field`] adds a field.
    /// On an XSD tab an array is a repeated element of any type, as before.
    pub fn add_array_field(
        &mut self,
        sheet: &str,
        object: &Path,
        name: &str,
        items: FieldKind,
        required: bool,
    ) -> Result<(), EditError> {
        if items == FieldKind::Array {
            return Err(EditError::Path(
                tr!("the items of an array cannot be arrays themselves").to_owned(),
            ));
        }
        self.add_field_of(sheet, object, name, FieldKind::Array, items, required)
    }

    fn add_field_of(
        &mut self,
        sheet: &str,
        object: &Path,
        name: &str,
        kind: FieldKind,
        items: FieldKind,
        required: bool,
    ) -> Result<(), EditError> {
        // An element added to one whose children are an ordered list goes
        // into the list's choice.
        let list = (!name.starts_with('@'))
            .then(|| self.spec_of(sheet).ok())
            .flatten()
            .and_then(|spec| {
                let list = object.key("#children");
                spec.list_choices(spec.resolve(&list)?.ty)
                    .map(|_| list.index(0))
            });
        let object = list.as_ref().unwrap_or(object);
        let places = self.field_places(sheet, object)?;
        check_name(name)?;
        if places.fields.iter().any(|f| f == name) {
            return Err(EditError::Path(tr!(
                "there is a field called {name} already",
                name = name
            )));
        }
        let what = tr!("Add field \"{name}\"", name = name);
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
                    let schema = match kind {
                        FieldKind::Array => FieldKind::array_schema(items),
                        _ => kind.schema(),
                    };
                    props.insert(name.to_owned(), schema);
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
            return Err(EditError::Path(tr!(
                "there is no field called {from}",
                from = from
            )));
        }
        check_name(to)?;
        if from == to {
            return Ok(());
        }
        if places.fields.iter().any(|f| f == to) {
            return Err(EditError::Path(tr!(
                "there is a field called {to} already",
                to = to
            )));
        }
        let xsd_document = if places.xsd {
            Some(self.xsd_edited_type(&places, &XsdChange::Rename { from, to })?)
        } else {
            None
        };
        self.check_objects_free(sheet, &places)?;
        let what = tr!("Rename field \"{from}\" to \"{to}\"", from = from, to = to);
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
            return Err(EditError::Path(tr!(
                "there is no field called {name}",
                name = name
            )));
        }
        let xsd_document = if places.xsd {
            Some(self.xsd_edited_type(&places, &XsdChange::Delete { name })?)
        } else {
            None
        };
        self.check_objects_free(sheet, &places)?;
        let what = tr!("Delete field \"{name}\"", name = name);
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
            if places.choice {
                // An item of an ordered list that held the element goes; the
                // last first, so that the places before it stay where they are.
                for place in places.objects.iter().rev() {
                    let holds = matches!(
                        calc.sheets[sheet].data.get(place).map(Node::value),
                        Some(Value::Object(map)) if map.contains_key(name)
                    );
                    if holds {
                        calc.remove(sheet, place)?;
                    }
                }
            } else {
                for place in &places.objects {
                    calc.rewrite_object(sheet, place, |map| {
                        map.shift_remove(name);
                    })?;
                }
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
            .ok_or_else(|| {
                EditError::Path(tr!("the type is not in the schema any more").to_owned())
            })?;
        let Value::Object(map) = node.value() else {
            return Err(EditError::Path(
                tr!("the type is not a schema object").to_owned(),
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
            .ok_or_else(|| EditError::Path(tr!("there is no node at {path}", path = path)))?;
        s.data
            .set(path, value)
            .map_err(|e| EditError::Path(e.to_string()))?;
        let after = s.data.get(path).cloned().expect("it was just set");
        self.changed(sheet, path);
        self.history.record(
            tr!("Edit").to_owned(),
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
        return Err(EditError::Path(tr!("a field needs a name").to_owned()));
    }
    if name != name.trim() || name.chars().any(char::is_control) {
        return Err(EditError::Path(
            tr!("a field name cannot start or end with a space or hold control characters")
                .to_owned(),
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
        (Kind::Object { fields } | Kind::Choice { fields }, Value::Object(map)) => {
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
            (Kind::Object { fields } | Kind::Choice { fields }, Segment::Key(name)) => {
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
        Kind::Object { fields } | Kind::Choice { fields } => {
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

/// A JSON Schema with no `required` list left in any of its records, at any
/// depth (for *Force type definition*). Only schema keywords are followed,
/// so a field that happens to be called `required` stays.
fn all_optional(schema: &mut Node) {
    let Value::Object(map) = schema.value_mut() else {
        return;
    };
    map.shift_remove("required");
    for (key, node) in map.iter_mut() {
        match key.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" => {
                if let Value::Object(fields) = node.value_mut() {
                    fields.values_mut().for_each(all_optional);
                }
            }
            "items" | "additionalProperties" => all_optional(node),
            "anyOf" | "oneOf" | "allOf" | "prefixItems" => {
                if let Value::Array(items) = node.value_mut() {
                    items.iter_mut().for_each(all_optional);
                }
            }
            _ => {}
        }
    }
}

/// The `schemaLocation` of the imported schema a type's lowering `source`
/// is in (`import[dc]/schema/…`), if it is in one. The built-in schemas
/// (`builtin:xml/…`) count as imported too.
fn imported_from(source: &str) -> Option<&str> {
    if let Some(rest) = source.strip_prefix("import[") {
        return rest.split_once("]/").map(|(place, _)| place);
    }
    source
        .strip_prefix("builtin:")
        .and_then(|rest| rest.split_once('/'))
        .map(|(name, _)| name)
}

/// Where the node at `path` is written in a schema tab's tree, if that can
/// be told.
fn place_in_schema(
    format: SchemaFormat,
    document: &Node,
    spec: &TypeSpec,
    path: &Path,
) -> Option<Path> {
    let here = spec.resolve(path)?;
    let source = &spec.get(here.ty).source;
    let place = match format {
        SchemaFormat::Xsd => {
            // A field: its `element` or `attribute` in its record's type,
            // rather than the type it refers to.
            let field = match (path.segments().last(), path.parent()) {
                (Some(Segment::Key(name)), Some(parent)) => spec
                    .resolve(&parent)
                    .filter(|p| matches!(spec.get(p.ty).kind, Kind::Object { .. }))
                    .and_then(|p| xsd_fields::xsd_type_path(document, &spec.get(p.ty).source))
                    .and_then(|record| xsd_declaration(document, &record, name)),
                _ => None,
            };
            field.or_else(|| xsd_fields::xsd_type_path(document, source))
        }
        SchemaFormat::Copybook => copybook_item_path(document, source),
        SchemaFormat::Json => {
            // A field: its entry under its record's `properties`, rather than
            // the type it refers to.
            let field = match (path.segments().last(), path.parent()) {
                (Some(Segment::Key(name)), Some(parent)) => spec
                    .resolve(&parent)
                    .filter(|p| matches!(spec.get(p.ty).kind, Kind::Object { .. }))
                    .and_then(|p| pointer_to_path(&spec.get(p.ty).source))
                    .map(|record| record.key("properties").key(name.clone()))
                    .filter(|place| document.get(place).is_some()),
                _ => None,
            };
            field.or_else(|| pointer_to_path(source))
        }
    };
    place.filter(|place| document.get(place).is_some())
}

/// The declaration of the field `name` (`@name` for an attribute) inside the
/// XSD type at `record`: the nearest `element` or `attribute` with that name,
/// looked for level by level.
fn xsd_declaration(document: &Node, record: &Path, name: &str) -> Option<Path> {
    let (key, name) = match name.strip_prefix('@') {
        Some(attribute) => ("attribute", attribute),
        None => ("element", name),
    };
    let named = |node: &Node| {
        let value = |key: &str| {
            node.get(&Path::root().key(key))
                .and_then(|n| n.value().as_str().map(str::to_owned))
        };
        value("@name").as_deref() == Some(name)
            || value("@ref").is_some_and(|r| {
                r == name || (!name.contains(':') && r.rsplit(':').next() == Some(name))
            })
    };
    let mut level = vec![record.clone()];
    while !level.is_empty() {
        let mut next = Vec::new();
        for at in level {
            let Some(Value::Object(map)) = document.get(&at).map(Node::value) else {
                continue;
            };
            for (k, child) in map {
                let here = at.key(k.clone());
                match child.value() {
                    Value::Array(items) => {
                        for (i, item) in items.iter().enumerate() {
                            if k == key && named(item) {
                                return Some(here.index(i));
                            }
                            next.push(here.index(i));
                        }
                    }
                    Value::Object(_) => {
                        if k == key && named(child) {
                            return Some(here);
                        }
                        next.push(here);
                    }
                    _ => {}
                }
            }
        }
        level = next;
    }
    None
}

/// The item of a copybook schema tab that a type's `source` names
/// (`RECORD.CUSTOMER.NAME`): each name is looked for among the items below
/// the last one found, at any depth (a group without a name of its own is
/// not in the source). A leading name that is no item, the record's, is
/// passed over.
fn copybook_item_path(document: &Node, source: &str) -> Option<Path> {
    fn find(node: &Node, at: &Path, name: &str) -> Option<Path> {
        let Value::Array(items) = node.get(&Path::root().key("items"))?.value() else {
            return None;
        };
        let list = at.key("items");
        for (i, item) in items.iter().enumerate() {
            let here = list.index(i);
            let named = item
                .get(&Path::root().key("name"))
                .and_then(|n| n.value().as_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name));
            if named {
                return Some(here);
            }
            if let Some(found) = find(item, &here, name) {
                return Some(found);
            }
        }
        None
    }
    let mut at = Path::root();
    let mut found = false;
    for name in source.split('.').filter(|n| !n.is_empty()) {
        let node = document.get(&at)?;
        match find(node, &at, name) {
            Some(place) => {
                at = place;
                found = true;
            }
            None if !found => {}
            None => return None,
        }
    }
    found.then_some(at)
}
