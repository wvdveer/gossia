//! Copy and paste of cells as tab-separated text (DESIGN.md, "Tabular view of
//! arrays", "Editing in the table" and "Hiding"; the decisions for copy and
//! paste of ranges).
//!
//! In a table the selection can be a rectangle: the selected cell and an
//! anchor, the corner where Shift was first used. The rectangle is logical:
//! every row and every column between the two corners is in it, hidden or
//! filtered ones too. Elsewhere the selection is one cell.
//!
//! *Copy* puts the values on the clipboard, which is what another program
//! wants, and remembers the formulas behind them; *Paste* of that same text
//! in this window puts the formulas back. *Copy formulas* and *Paste values*
//! choose explicitly. None of this draws, so it is tested without a window.

use gossia_i18n::tr;
use std::ops::RangeInclusive;

use gossia_calc::entry;
use gossia_core::{ErrorKind, Node, Path, Value};

use crate::app::GossiaApp;
use crate::rows::display;
use crate::table::Column;

/// What a copy puts on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// The values, with the formulas remembered for a paste in this window.
    Cells,
    /// `=` and the formula of each cell that has one, the value otherwise.
    Formulas,
}

/// What a paste writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteMode {
    /// The formulas of this window's own copy, otherwise the text as values.
    Cells,
    /// The values, even of this window's own copy.
    Values,
}

/// This window's last *Copy*: the text it put on the clipboard and what each
/// cell held, so that a paste of the same text can put the formulas back.
#[derive(Debug, Clone)]
pub struct CopyRecord {
    pub text: String,
    pub cells: Vec<Vec<CopiedCell>>,
}

#[derive(Debug, Clone)]
pub struct CopiedCell {
    /// The value as it is typed (see `value_text`).
    pub value: String,
    /// The formula, without the `=`.
    pub formula: Option<String>,
    /// The value is an error.
    pub error: bool,
}

/// One cell to paste: the text to enter, and whether it is an error value,
/// which is left out.
#[derive(Debug, Clone)]
struct Pasted {
    text: String,
    error: bool,
}

/// The cells the selection covers in a table: rows by their index in the
/// array and columns by their place among all the columns, hidden or not.
#[derive(Debug, Clone, PartialEq)]
pub struct CellRange {
    pub array: Path,
    pub rows: RangeInclusive<usize>,
    pub columns: RangeInclusive<usize>,
}

impl CellRange {
    pub fn contains(&self, row: usize, column: usize) -> bool {
        self.rows.contains(&row) && self.columns.contains(&column)
    }

    /// Whether it is more than one cell.
    pub fn is_range(&self) -> bool {
        self.rows.start() != self.rows.end() || self.columns.start() != self.columns.end()
    }
}

// ----------------------------------------------------------- the text

/// Writes cells as tab-separated text, as a spreadsheet puts them on the
/// clipboard: a tab between cells, a line break after each row, and a cell
/// that holds a tab, a line break or starts with a quote in double quotes,
/// with its quotes doubled.
pub fn to_tsv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row.iter().map(|cell| quote(cell)).collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    out
}

fn quote(cell: &str) -> String {
    if cell.contains(['\t', '\n', '\r']) || cell.starts_with('"') {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_owned()
    }
}

/// Reads tab-separated text into rows of cells. A cell in double quotes may
/// hold tabs, line breaks and `""` for a quote; a quote inside a cell that
/// does not start with one is an ordinary character. One line break at the
/// end is dropped, and a blank line is a row with one empty cell, so that the
/// rows stay in step with what was copied. Cells are not trimmed.
pub fn from_tsv(text: &str) -> Vec<Vec<String>> {
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut chars = text.chars().peekable();
    let mut at_start = true;
    while let Some(c) = chars.next() {
        match c {
            '"' if at_start => {
                // A quoted cell, up to the quote that is not doubled.
                while let Some(q) = chars.next() {
                    if q == '"' {
                        if chars.peek() == Some(&'"') {
                            chars.next();
                            cell.push('"');
                        } else {
                            break;
                        }
                    } else {
                        cell.push(q);
                    }
                }
                at_start = false;
            }
            '\t' => {
                row.push(std::mem::take(&mut cell));
                at_start = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                at_start = true;
            }
            _ => {
                cell.push(c);
                at_start = false;
            }
        }
    }
    row.push(cell);
    rows.push(row);
    rows
}

// ------------------------------------------------------ the selection

impl GossiaApp {
    /// Where the selected cell is in the table in the pane: the row's index
    /// and the column's place among all the columns.
    fn table_place(&self, path: &Path) -> Option<(usize, usize)> {
        let layout = self.layout()?;
        let (array, index) = self.table_row_of(path)?;
        if array != layout.array {
            return None;
        }
        let column = layout
            .columns
            .iter()
            .position(|c| &c.cell_path(&array, index) == path)?;
        Some((index, column))
    }

    /// The cells the selection covers in the table in the pane, or `None`
    /// when the selection is not a cell of it.
    pub fn table_range(&self) -> Option<CellRange> {
        let selection = self.selection.as_ref()?;
        let (row, column) = self.table_place(selection)?;
        let (anchor_row, anchor_column) = self
            .tables
            .anchor
            .as_ref()
            .and_then(|anchor| self.table_place(anchor))
            .unwrap_or((row, column));
        Some(CellRange {
            array: self.layout()?.array.clone(),
            rows: row.min(anchor_row)..=row.max(anchor_row),
            columns: column.min(anchor_column)..=column.max(anchor_column),
        })
    }

    /// Whether the cell at `path` is the selected cell or in the range.
    pub fn is_in_selection(&self, path: &Path) -> bool {
        if self.selection.as_ref() == Some(path) {
            return true;
        }
        match (self.table_range(), self.table_place(path)) {
            (Some(range), Some((row, column))) => range.contains(row, column),
            _ => false,
        }
    }

    /// Selects `path` and keeps the corner the range started from, so that
    /// the selection becomes the rectangle between them (Shift and a click).
    /// Outside a table it is an ordinary selection.
    pub fn extend_selection(&mut self, path: &Path) -> bool {
        let anchor = self
            .tables
            .anchor
            .clone()
            .or_else(|| self.selection.clone());
        if !self.select(path) {
            return false;
        }
        let same_table = anchor.as_ref().is_some_and(|anchor| {
            self.table_place(anchor).is_some() && self.table_place(path).is_some()
        });
        if same_table {
            self.tables.anchor = anchor;
        }
        true
    }

    /// Moves the selected corner of the range among the cells that are shown
    /// (Shift and an arrow key).
    pub fn table_extend_selection(&mut self, dx: isize, dy: isize) {
        let anchor = self
            .tables
            .anchor
            .clone()
            .or_else(|| self.selection.clone());
        self.table_move_selection(dx, dy);
        if self.selection.as_ref() != anchor.as_ref() {
            self.tables.anchor = anchor;
        }
    }

    // ------------------------------------------------------------ copy

    /// The value of one cell as text, whatever computes it, as it is typed:
    /// text with an apostrophe in front when it starts with `=` or an
    /// apostrophe, numbers in their full form. An error, or a summary of an
    /// array or a map, is what the cell shows. The flag says it is an error.
    fn value_text(&self, path: &Path) -> (String, bool) {
        match self
            .calc()
            .data(&self.sheet)
            .and_then(|d| d.get(path))
            .map(Node::value)
        {
            None => (String::new(), false),
            Some(Value::Str(s)) if s.starts_with('=') || s.starts_with('\'') => {
                (format!("'{s}"), false)
            }
            Some(Value::Str(s)) => (s.clone(), false),
            Some(value @ Value::Error(_)) => (display(value), true),
            Some(value) => (display(value), false),
        }
    }

    fn copied_cell(&self, path: &Path) -> CopiedCell {
        let (value, error) = self.value_text(path);
        CopiedCell {
            value,
            formula: self.calc().formula_text(&self.sheet, path),
            error,
        }
    }

    /// The cells of the selection: the whole rectangle in a table, hidden
    /// rows and columns included, or the one selected cell.
    fn selected_cells(&self) -> Option<Vec<Vec<CopiedCell>>> {
        if let Some(range) = self.table_range() {
            let columns = &self.layout()?.columns;
            return Some(
                range
                    .rows
                    .clone()
                    .map(|index| {
                        range
                            .columns
                            .clone()
                            .map(|c| self.copied_cell(&columns[c].cell_path(&range.array, index)))
                            .collect()
                    })
                    .collect(),
            );
        }
        let path = self.selection.as_ref()?;
        Some(vec![vec![self.copied_cell(path)]])
    }

    /// Copies the selection and puts it on the clipboard. *Copy* gives the
    /// values, and remembers the formulas behind them for a paste in this
    /// window; *Copy formulas* gives `=` and the formula of each cell that has
    /// one. Returns the text.
    pub fn copy_cells(&mut self, mode: CopyMode) -> Option<String> {
        let cells = self.selected_cells()?;
        let rows: Vec<Vec<String>> = cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match (mode, &cell.formula) {
                        (CopyMode::Formulas, Some(formula)) => format!("={formula}"),
                        _ => cell.value.clone(),
                    })
                    .collect()
            })
            .collect();
        let text = to_tsv(&rows);
        self.tables.copied = (mode == CopyMode::Cells).then(|| CopyRecord {
            text: text.clone(),
            cells,
        });
        self.write_clipboard(&text);
        Some(text)
    }

    /// Puts text on the clipboard: on the next frame through egui, or on the
    /// stand-in clipboard when the tests set one. In a host, the clipboard
    /// is the application's own, in the host's storage, apart from the
    /// system's (as in the owner's other web projects).
    fn write_clipboard(&mut self, text: &str) {
        match &mut self.tables.stand_in {
            Some(stand_in) => *stand_in = text.to_owned(),
            #[cfg(target_family = "wasm")]
            None => {
                if let Some(host) = crate::host::current()
                    && let Err(e) = host.write_storage(crate::host::CLIPBOARD, text)
                {
                    self.notice = Some(tr!("the clipboard cannot be written: {e}", e = e));
                }
            }
            #[cfg(not(target_family = "wasm"))]
            None => self.tables.to_clipboard = Some(text.to_owned()),
        }
    }

    /// The text on the clipboard. egui gives it only with Ctrl+V, so the other
    /// commands read it themselves.
    pub fn read_clipboard(&self) -> Result<String, String> {
        if let Some(text) = &self.tables.stand_in {
            return Ok(text.clone());
        }
        #[cfg(not(target_family = "wasm"))]
        return arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map(|text| text.replace("\r\n", "\n"))
            .map_err(|e| tr!("the clipboard cannot be read: {e}", e = e));
        #[cfg(target_family = "wasm")]
        crate::host::current()
            .and_then(|host| host.read_storage(crate::host::CLIPBOARD))
            .ok_or_else(|| tr!("nothing has been copied").to_owned())
    }

    // ----------------------------------------------------------- paste

    /// Pastes what is on the clipboard (see [`GossiaApp::paste_clip`]).
    pub fn paste_from_clipboard(&mut self, mode: PasteMode) -> Result<(), String> {
        let text = match self.read_clipboard() {
            Ok(text) => text,
            Err(e) => {
                self.notice = Some(e.clone());
                return Err(e);
            }
        };
        self.paste_clip(&text, mode)
    }

    /// Pastes text at the selection, cell for cell from its top left corner,
    /// hidden cells included. If the text is this window's own last *Copy*,
    /// *Paste* puts back the formulas it came from, and *Paste values* the
    /// values; any other text is values. One cell fills the whole range, and
    /// rows past the end of the array are added. Every cell is entered as if
    /// typed, and the paste is one entry in the history: if a cell is refused,
    /// nothing is written and the reason names the cell. Error values are not
    /// pasted; the notice says how many were left as they were.
    pub fn paste_clip(&mut self, text: &str, mode: PasteMode) -> Result<(), String> {
        if self.editing.is_some() && !self.commit_editing() {
            return Err(self.notice.clone().unwrap_or_default());
        }
        let text = text.replace("\r\n", "\n");
        let cells: Vec<Vec<Pasted>> = match &self.tables.copied {
            Some(record) if record.text == text => record
                .cells
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| match (mode, &cell.formula) {
                            (PasteMode::Cells, Some(formula)) => Pasted {
                                text: format!("={formula}"),
                                error: false,
                            },
                            _ => Pasted {
                                text: cell.value.clone(),
                                error: cell.error,
                            },
                        })
                        .collect()
                })
                .collect(),
            _ => from_tsv(&text)
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|text| Pasted {
                            error: is_error_code(&text),
                            text,
                        })
                        .collect()
                })
                .collect(),
        };
        let result = match self.table_range() {
            Some(range) => self.paste_in_table(&range, &cells),
            None => self.paste_in_cell(&cells),
        };
        self.notice = match &result {
            Ok(0) => None,
            Ok(1) => Some("1 cell with an error was not pasted".to_owned()),
            Ok(n) => Some(format!("{n} cells with an error were not pasted")),
            Err(e) => Some(e.clone()),
        };
        self.session.tick();
        result.map(|_| ())
    }

    fn paste_in_cell(&mut self, cells: &[Vec<Pasted>]) -> Result<usize, String> {
        let path = self
            .selection
            .clone()
            .ok_or(tr!("nothing is selected to paste into"))?;
        let one = tr!("only one cell can be pasted here; paste a range into a table");
        let [row] = cells else {
            return Err(one.to_owned());
        };
        let [cell] = row.as_slice() else {
            return Err(one.to_owned());
        };
        self.paste_cells(tr!("Paste"), &path, &[], &[(path.clone(), cell.clone())])
    }

    fn paste_in_table(
        &mut self,
        range: &CellRange,
        cells: &[Vec<Pasted>],
    ) -> Result<usize, String> {
        let columns: Vec<Column> = self.layout().map(|l| l.columns.clone()).unwrap_or_default();
        let length = self.items_len(&range.array);
        let one = cells.len() == 1 && cells[0].len() == 1;
        // A single cell fills the range; anything else starts at its corner.
        let (height, width) = if one {
            (
                range.rows.end() - range.rows.start() + 1,
                range.columns.end() - range.columns.start() + 1,
            )
        } else {
            (cells.len(), cells.iter().map(Vec::len).max().unwrap_or(0))
        };
        let (top, left) = (*range.rows.start(), *range.columns.start());
        if left + width > columns.len() {
            return Err(tr!(
                "the clipboard is {width} columns wide and the table has {count} from {column}",
                width = width,
                count = columns.len() - left,
                column = columns[left].name
            ));
        }
        let mut writes = Vec::new();
        for r in 0..height {
            for c in 0..width {
                let cell = if one {
                    cells[0][0].clone()
                } else {
                    // A short row leaves the cells after it as they are.
                    match cells[r].get(c) {
                        Some(cell) => cell.clone(),
                        None => continue,
                    }
                };
                let column = &columns[left + c];
                if column.summary {
                    // A count such as `[3 items]` cannot be entered; its cell
                    // is left as it is.
                    continue;
                }
                writes.push((column.cell_path(&range.array, top + r), cell));
            }
        }
        let added = (top + height).saturating_sub(length);
        let label = if height * width == 1 {
            tr!("Paste").to_owned()
        } else {
            tr!(
                "Paste {height} x {width} cells",
                height = height,
                width = width
            )
        };
        let added: Vec<usize> = (length..length + added).collect();
        let skipped = self.paste_cells(&label, &range.array, &added, &writes)?;
        for index in top..top + height {
            if let Some(column) = columns.get(left) {
                self.pin_row_of(&column.cell_path(&range.array, index));
            }
        }
        Ok(skipped)
    }

    fn items_len(&self, array: &Path) -> usize {
        self.calc()
            .data(&self.sheet)
            .and_then(|d| d.get(array))
            .and_then(|n| n.value().as_array())
            .map_or(0, <[Node]>::len)
    }

    /// Adds the rows `added` to the array at `array` and enters every write,
    /// as one entry; all or nothing. Returns how many error cells were left
    /// out.
    fn paste_cells(
        &mut self,
        label: &str,
        array: &Path,
        added: &[usize],
        writes: &[(Path, Pasted)],
    ) -> Result<usize, String> {
        let skipped = writes.iter().filter(|(_, cell)| cell.error).count();
        // A cell that already says what is pasted is left alone, so that
        // copying rows within a table keeps a column formula whole.
        let writes: Vec<(Path, String)> = writes
            .iter()
            .filter(|(_, cell)| !cell.error)
            .filter(|(path, cell)| {
                added.contains(&row_of(array, path)) || self.cell_text(path) != cell.text
            })
            .map(|(path, cell)| (path.clone(), cell.text.clone()))
            .collect();
        if added.is_empty() && writes.is_empty() {
            return Ok(skipped);
        }
        let sheet = self.sheet.clone();
        let item = if added.is_empty() {
            None
        } else {
            let calc = self.calc();
            let spec = calc.spec(&sheet).ok_or(tr!("the tab has no schema"))?;
            match spec.resolve(array).map(|r| &spec.get(r.ty).kind) {
                Some(gossia_core::Kind::Array { items, .. }) => {
                    Some(GossiaApp::default_value(spec, *items))
                }
                _ => return Err(format!("{} is not an array", self.name_of(array))),
            }
        };
        let names: Vec<String> = writes.iter().map(|(path, _)| self.name_of(path)).collect();
        let calc = self.session.calc_mut();
        calc.begin_entry(label);
        let result = (|| {
            for &index in added {
                let value = item.clone().unwrap_or(Value::Null);
                calc.insert_item(&sheet, array, index, value)
                    .map_err(|e| e.to_string())?;
            }
            for ((path, text), name) in writes.iter().zip(&names) {
                entry::enter(calc, &sheet, path, text, false)
                    .map_err(|e| format!("{name}: {e}; nothing was pasted"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                calc.end_entry();
                if !added.is_empty() {
                    self.set_open(array, true);
                }
                Ok(skipped)
            }
            Err(e) => {
                calc.abort_entry();
                Err(e)
            }
        }
    }
}

/// The index of the item of `array` that `path` is in, or `usize::MAX`.
fn row_of(array: &Path, path: &Path) -> usize {
    match path.segments().get(array.len()) {
        Some(gossia_core::Segment::Index(i)) => *i,
        _ => usize::MAX,
    }
}

/// Whether the text is the code of an error value, such as `#DIV/0!`.
fn is_error_code(text: &str) -> bool {
    ErrorKind::ALL.iter().any(|kind| kind.code() == text)
}
