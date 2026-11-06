//! The file format of a tab (DESIGN.md, "Import and export", "Other file
//! formats"): which one it is, which it could be, and changing it.

use gossia_core::CommentPlace;
use gossia_core::dataformat::DataFormat;
use gossia_i18n::tr;

use super::*;

impl Calc {
    /// The format the tab is written in.
    pub fn format(&self, sheet: &str) -> Option<DataFormat> {
        self.sheets.get(sheet).map(|s| s.format)
    }

    /// Whether the tab can have comments, so that it has the comment
    /// commands: every tab but one of COBOL records. A JSON or JSON Lines
    /// tab keeps them in the workbook only.
    pub fn format_has_comments(&self, sheet: &str) -> bool {
        self.format(sheet).is_some_and(DataFormat::keeps_comments)
    }

    /// How many nodes with comments the tab has, which a file of `format`
    /// leaves out because it has no comments (export says so).
    pub fn comments_left_out(&self, sheet: &str, format: DataFormat) -> usize {
        if format.has_comments() {
            return 0;
        }
        self.sheets
            .get(sheet)
            .map_or(0, |s| commented(&s.data, &Path::root()).len())
    }

    /// Why the tab cannot be written in `format`, or None. A schema tab keeps
    /// its own (its documents are JSON or XSD), XML goes only with an XSD and
    /// the JSON-shaped formats only with a JSON Schema, and the format must
    /// be able to hold the tab's root.
    pub fn format_problem(&self, sheet: &str, format: DataFormat) -> Option<String> {
        let Some(s) = self.sheets.get(sheet) else {
            return Some(tr!("there is no tab called {sheet}", sheet = sheet));
        };
        if matches!(s.schema, SchemaRole::Document { .. }) {
            return Some(tr!(
                "{sheet} is a schema tab, which is written as its schema language is",
                sheet = sheet
            ));
        }
        let fits = match SchemaFormat::of(&s.spec) {
            SchemaFormat::Xsd => format == DataFormat::Xml,
            SchemaFormat::Copybook => format == DataFormat::Blk,
            SchemaFormat::Json => format.json_shaped(),
        };
        if !fits {
            return Some(tr!(
                "{sheet} holds {data}, so it cannot be {format}",
                sheet = sheet,
                data = SchemaFormat::of(&s.spec).data_label(),
                format = format.label()
            ));
        }
        format.root_problem(&s.spec.get(s.spec.root()).kind)
    }

    /// The formats the tab could be written in, its own first.
    pub fn formats_for(&self, sheet: &str) -> Vec<DataFormat> {
        let Some(own) = self.format(sheet) else {
            return Vec::new();
        };
        let mut formats = vec![own];
        formats.extend(
            DataFormat::ALL
                .into_iter()
                .filter(|f| *f != own && self.format_problem(sheet, *f).is_none()),
        );
        formats
    }

    /// How many comments changing the tab to `format` would take off, as the
    /// format has none.
    pub fn comments_lost(&self, sheet: &str, format: DataFormat) -> usize {
        if format.keeps_comments() {
            return 0;
        }
        self.sheets
            .get(sheet)
            .map_or(0, |s| commented(&s.data, &Path::root()).len())
    }

    /// Changes the format the tab is written in: one entry in the history.
    /// Comments the new format cannot hold are taken off (the window asks
    /// first; see `comments_lost`). The data is otherwise the same, so no
    /// formula is out of date.
    pub fn set_format(&mut self, sheet: &str, format: DataFormat) -> Result<(), EditError> {
        if let Some(why) = self.format_problem(sheet, format) {
            return Err(EditError::Path(why));
        }
        let before = self.format(sheet).expect("the tab is there");
        if before == format {
            return Ok(());
        }
        let mut ops = Vec::new();
        if !format.keeps_comments() {
            let data = self.sheets[sheet].data.clone();
            for path in commented(&data, &Path::root()) {
                let node = data.get(&path).expect("the path was just found");
                for place in CommentPlace::ALL {
                    if let Some(text) = node.comment_at(place) {
                        ops.push(Op::Comment {
                            sheet: sheet.to_owned(),
                            path: path.clone(),
                            place,
                            before: Some(text.to_owned()),
                            after: None,
                        });
                    }
                }
            }
        }
        for op in &ops {
            if let Op::Comment {
                path, place, after, ..
            } = op
            {
                self.put_comment(sheet, path, *place, after.clone())?;
            }
        }
        self.store_format(sheet, format)?;
        ops.push(Op::Format {
            sheet: sheet.to_owned(),
            before,
            after: format,
        });
        self.history.record(
            tr!(
                "Write {sheet} as {format}",
                sheet = sheet,
                format = format.label()
            ),
            ops,
            None,
        );
        Ok(())
    }

    pub(super) fn store_format(
        &mut self,
        sheet: &str,
        format: DataFormat,
    ) -> Result<(), EditError> {
        self.sheet_mut(sheet)?.format = format;
        self.revision += 1;
        Ok(())
    }
}

/// The paths of the nodes under (and at) `path` that have a comment.
pub(super) fn commented(node: &Node, path: &Path) -> Vec<Path> {
    let mut out = Vec::new();
    if node.has_comments() {
        out.push(path.clone());
    }
    match node.value() {
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                out.extend(commented(item, &path.index(i)));
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields {
                out.extend(commented(field, &path.key(key.clone())));
            }
        }
        _ => {}
    }
    out
}
