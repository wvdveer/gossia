//! Comments on the nodes of an XML tab (DESIGN.md, "Comments and structure
//! fidelity", "Comments in the UI"). A comment is part of its node and is
//! written as an XML comment before its element. Formulas cannot see it, so
//! changing one recalculates nothing; it is still an edit of the document, an
//! entry in the history that marks the workbook as modified.

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
    /// Only the elements of an XML tab can: not an attribute, which XML gives
    /// no place for a comment, not the text of an element, and not an array,
    /// whose items are the elements. The node must be there, and not inside
    /// the result of a formula, which replaces what is inside on every run.
    pub fn comment_refusal(&self, sheet: &str, path: &Path) -> Option<String> {
        let Some(s) = self.sheets.get(sheet) else {
            return Some(format!("there is no tab called {sheet}"));
        };
        if s.spec.xml().is_none() {
            return Some("Comments are only on XML tabs, since JSON has none.".to_owned());
        }
        if let Some(Segment::Key(key)) = path.segments().last() {
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
        if matches!(node.value(), Value::Array(_)) {
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

    /// The comment on the node at `path`, if it has one.
    pub fn comment(&self, sheet: &str, path: &Path) -> Option<&str> {
        self.sheets.get(sheet)?.data.get(path)?.comment()
    }

    /// Inserts, edits or (with `None`, or nothing but white space) deletes
    /// the comment on the node at `path`. One entry in the history; nothing is
    /// recalculated. Text that XML cannot hold in a comment is refused.
    pub fn set_comment(
        &mut self,
        sheet: &str,
        path: &Path,
        text: Option<&str>,
    ) -> Result<(), EditError> {
        if let Some(why) = self.comment_refusal(sheet, path) {
            return Err(EditError::Path(why));
        }
        let after = text
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.replace("\r\n", "\n"));
        if let Some(why) = after.as_deref().and_then(comment_text_problem) {
            return Err(EditError::Path(why.to_owned()));
        }
        let before = self.comment(sheet, path).map(str::to_owned);
        if before == after {
            return Ok(());
        }
        let what = match (&before, &after) {
            (None, _) => "Insert comment on",
            (_, None) => "Delete comment on",
            _ => "Edit comment on",
        };
        self.put_comment(sheet, path, after.clone())?;
        self.history.record(
            format!("{what} {}", Loc::new(sheet, path.clone())),
            vec![Op::Comment {
                sheet: sheet.to_owned(),
                path: path.clone(),
                before,
                after,
            }],
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
        comment: Option<String>,
    ) -> Result<(), EditError> {
        let node = self
            .sheet_mut(sheet)?
            .data
            .get_mut(path)
            .ok_or_else(|| EditError::Path(format!("there is no node at {path}")))?;
        node.set_comment(comment);
        self.revision += 1;
        Ok(())
    }
}
