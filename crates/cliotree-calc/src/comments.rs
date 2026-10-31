//! Comments on the nodes of a tab whose format has comments: XML, JSON5 and
//! JSONC (DESIGN.md, "Comments and structure fidelity", "Comments in the UI",
//! "Other file formats"). A comment is part of its node and is written before
//! it, or inside it, at its start or its end ([`CommentPlace`]); in XML as an
//! XML comment, in JSON5 and JSONC as `//` lines. Formulas cannot see it, so changing one
//! recalculates nothing; it is still an edit of the document, an entry in the
//! history that marks the workbook as modified.

use cliotree_core::CommentPlace;

use super::*;

/// Why `text` cannot be the text of an XML comment, if it cannot.
pub fn comment_text_problem(text: &str) -> Option<&'static str> {
    if text.contains("--") {
        Some("A comment cannot contain two hyphens in a row (--), because XML does not allow it.")
    } else if text.ends_with('-') {
        Some("A comment cannot end with a hyphen, because XML does not allow it.")
    } else {
        None
    }
}

impl Calc {
    /// Why the node at `path` cannot carry a comment, or `None` if it can.
    /// Only a tab whose format has comments can. On an XML tab only elements
    /// can: not an attribute, which XML gives no place for a comment, not the
    /// text of an element, and not an array, whose items are the elements.
    /// The node must be there, and not inside the result of a formula, which
    /// replaces what is inside on every run.
    pub fn comment_refusal(&self, sheet: &str, path: &Path) -> Option<String> {
        let Some(s) = self.sheets.get(sheet) else {
            return Some(format!("there is no tab called {sheet}"));
        };
        if !s.format.has_comments() {
            return Some(format!(
                "{} has no comments; a tab written as XML, JSON5 or JSON with comments can                  have them.",
                s.format.label()
            ));
        }
        let xml = s.spec.xml().is_some();
        if let Some(Segment::Key(key)) = path.segments().last().filter(|_| xml) {
            if key.starts_with('@') {
                return Some(
                    "An attribute cannot have a comment: XML has no place for one inside a tag."
                        .to_owned(),
                );
            }
            if key == "#text" {
                return Some(
                    "The text of an element cannot have a comment of its own; the element can."
                        .to_owned(),
                );
            }
        }
        let Some(node) = s.data.get(path) else {
            return Some("The node is not there, so it cannot have a comment.".to_owned());
        };
        if xml && matches!(node.value(), Value::Array(_)) {
            return Some("An array cannot have a comment; each of its items can.".to_owned());
        }
        if let Some(formula) = s.formulas.computed_by(path) {
            return Some(format!(
                "The node is computed by the formula on {}, which replaces it.",
                Loc::new(sheet, formula)
            ));
        }
        None
    }

    /// The comment on the node at `path` (the one before its element), if it
    /// has one.
    pub fn comment(&self, sheet: &str, path: &Path) -> Option<&str> {
        self.comment_at(sheet, path, CommentPlace::Before)
    }

    /// The comment on the node at `path` at `place`, if it has one.
    pub fn comment_at(&self, sheet: &str, path: &Path, place: CommentPlace) -> Option<&str> {
        self.sheets.get(sheet)?.data.get(path)?.comment_at(place)
    }

    /// Whether the node at `path` has a comment at any place.
    pub fn has_comments(&self, sheet: &str, path: &Path) -> bool {
        self.sheets
            .get(sheet)
            .and_then(|s| s.data.get(path))
            .is_some_and(Node::has_comments)
    }

    /// Inserts, edits or (with `None`, or nothing but white space) deletes
    /// the comment on the node at `path`, before its element.
    pub fn set_comment(
        &mut self,
        sheet: &str,
        path: &Path,
        text: Option<&str>,
    ) -> Result<(), EditError> {
        self.set_comments(sheet, path, &[(CommentPlace::Before, text)])
    }

    /// Sets the comments of the node at `path` at the places given (`None`,
    /// or nothing but white space, deletes one). One entry in the history;
    /// nothing is recalculated. Text that XML cannot hold in a comment is
    /// refused, and then nothing changes.
    pub fn set_comments(
        &mut self,
        sheet: &str,
        path: &Path,
        texts: &[(CommentPlace, Option<&str>)],
    ) -> Result<(), EditError> {
        if let Some(why) = self.comment_refusal(sheet, path) {
            return Err(EditError::Path(why));
        }
        let mut ops = Vec::new();
        for &(place, text) in texts {
            let after = text
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.replace("\r\n", "\n"));
            let xml = self.is_xml(sheet);
            if let Some(why) = after
                .as_deref()
                .filter(|_| xml)
                .and_then(comment_text_problem)
            {
                return Err(EditError::Path(why.to_owned()));
            }
            let before = self.comment_at(sheet, path, place).map(str::to_owned);
            if before != after {
                ops.push((place, before, after));
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        let what = if ops.iter().all(|(_, before, _)| before.is_none()) {
            "Insert comment on"
        } else if ops.iter().all(|(_, _, after)| after.is_none()) {
            "Delete comment on"
        } else {
            "Edit comment on"
        };
        for (place, _, after) in &ops {
            self.put_comment(sheet, path, *place, after.clone())?;
        }
        self.history.record(
            format!("{what} {}", Loc::new(sheet, path.clone())),
            ops.into_iter()
                .map(|(place, before, after)| Op::Comment {
                    sheet: sheet.to_owned(),
                    path: path.clone(),
                    place,
                    before,
                    after,
                })
                .collect(),
            None,
        );
        Ok(())
    }

    /// Puts a comment on a node, as an edit and its undo do. The window is
    /// drawn again; no formula is out of date.
    pub(super) fn put_comment(
        &mut self,
        sheet: &str,
        path: &Path,
        place: CommentPlace,
        comment: Option<String>,
    ) -> Result<(), EditError> {
        let node = self
            .sheet_mut(sheet)?
            .data
            .get_mut(path)
            .ok_or_else(|| EditError::Path(format!("there is no node at {path}")))?;
        node.set_comment_at(place, comment);
        self.revision += 1;
        Ok(())
    }
}
