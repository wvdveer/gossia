//! Breaking a multipage report into pages (DESIGN.md, Reports, "Breaking
//! into pages"). Every band has a fixed height, so where the pages break
//! depends only on the design, on how many items each section and the detail
//! band have, and on which bands are visible: no text is measured. The
//! evaluation uses it for `page.number` and `page.count`, and the layout to
//! draw the pages, so both break in the same places.

use super::{BandKind, ReportDesign};

/// The number of detail bands a report stops after when nothing else is
/// set (DESIGN.md, Reports, "Limit").
pub const DETAIL_LIMIT: usize = 100_000;

/// A band placed on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// Its place in the stack of bands.
    pub band: usize,
    /// The item of each section around it, outermost first, and for the
    /// detail band the detail item last. The page header and footer have the
    /// page's index.
    pub rows: Vec<usize>,
    /// Its top, in points from the top of the page inside the margins.
    pub y: f32,
    /// The height it is drawn: less than its own when it is cut at the end
    /// of the page.
    pub height: f32,
}

/// A report broken into pages.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paging {
    /// The bands of each page, in the order they are drawn.
    pub pages: Vec<Vec<Placed>>,
    /// The bands taller than the space between the page header and footer,
    /// which are cut at the end of the page, each once.
    pub cut: Vec<usize>,
    /// The report stopped at the limit of detail bands.
    pub limited: bool,
    /// Where the last band on the last page ends, inside the margins.
    pub end: f32,
}

/// Breaks `design` into pages. `items(level, rows)` is how many items the
/// section at depth `level` has in the occurrence `rows` of the sections
/// around it (the detail band is the level after the last section);
/// `visible(band, rows)` says whether a band in the body is drawn there. The
/// report stops after `limit` detail bands.
pub fn paginate(
    design: &ReportDesign,
    items: &dyn Fn(usize, &[usize]) -> usize,
    visible: &dyn Fn(usize, &[usize]) -> bool,
    limit: usize,
) -> Paging {
    let Some(sections) = design.bands() else {
        return Paging::default();
    };
    let (_, page_h) = design.page.size();
    let margins = design.page.margins;
    let content = (page_h - (margins.top + margins.bottom) as f32).max(0.0);
    let mut pager = Pager {
        design,
        sections,
        items,
        visible,
        limit,
        content,
        out: Paging::default(),
        cursor: 0.0,
        body_top: 0.0,
        body_bottom: 0.0,
        open: Vec::new(),
        details: 0,
    };
    pager.start_page();
    let mut rows = Vec::new();
    if pager.level(0, &mut rows) {
        pager.place(BandKind::ReportFooter.index(sections), &[]);
    }
    pager.finish_page();
    pager.out
}

struct Pager<'a> {
    design: &'a ReportDesign,
    sections: usize,
    items: &'a dyn Fn(usize, &[usize]) -> usize,
    visible: &'a dyn Fn(usize, &[usize]) -> bool,
    limit: usize,
    /// The height inside the margins.
    content: f32,
    out: Paging,
    cursor: f32,
    body_top: f32,
    body_bottom: f32,
    /// The sections being drawn: their depth and occurrence.
    open: Vec<(usize, Vec<usize>)>,
    details: usize,
}

impl Pager<'_> {
    fn height(&self, band: usize) -> f32 {
        self.design
            .layout
            .controls
            .get(band)
            .and_then(|c| c.height)
            .unwrap_or(0) as f32
    }

    fn index(&self, kind: BandKind) -> usize {
        kind.index(self.sections)
    }

    fn page(&self) -> usize {
        self.out.pages.len().saturating_sub(1)
    }

    /// Puts a band at the cursor, cut at the end of the body if it is
    /// taller than what is left.
    fn put(&mut self, band: usize, rows: &[usize]) {
        let h = self.height(band);
        let room = (self.body_bottom - self.cursor).max(0.0);
        let drawn = if h > room + 0.01 {
            if !self.out.cut.contains(&band) {
                self.out.cut.push(band);
            }
            room
        } else {
            h
        };
        let page = self.page();
        self.out.pages[page].push(Placed {
            band,
            rows: rows.to_vec(),
            y: self.cursor,
            height: drawn,
        });
        self.cursor += drawn;
    }

    fn at_top(&self) -> bool {
        self.cursor <= self.body_top + 0.01
    }

    /// Starts a page: on the first the report header, then the page header,
    /// then again the headers of the sections that repeat theirs.
    fn start_page(&mut self) {
        let first = self.out.pages.is_empty();
        self.out.pages.push(Vec::new());
        let page = self.page();
        self.cursor = 0.0;
        let header = self.index(BandKind::PageHeader);
        let footer = self.index(BandKind::PageFooter);
        let (ph, pf) = (self.height(header), self.height(footer));
        self.body_bottom = (self.content - pf).max(0.0);
        if first {
            let report_header = self.index(BandKind::ReportHeader);
            if self.height(report_header) > 0.0 && (self.visible)(report_header, &[]) {
                // The page header still has its room under it.
                self.body_bottom = (self.content - pf - ph).max(0.0);
                self.put(report_header, &[]);
                self.body_bottom = (self.content - pf).max(0.0);
            }
        }
        if ph > 0.0 {
            let y = self.cursor;
            self.out.pages[page].push(Placed {
                band: header,
                rows: vec![page],
                y,
                height: ph.min((self.content - y).max(0.0)),
            });
            self.cursor += ph;
        }
        self.body_top = self.cursor;
        for (level, rows) in self.open.clone() {
            if self.design.section_options(level).repeat_header {
                let band = self.index(BandKind::SectionHeader(level));
                if self.height(band) > 0.0 && (self.visible)(band, &rows) {
                    self.put(band, &rows);
                }
            }
        }
    }

    /// Ends a page with its page footer.
    fn finish_page(&mut self) {
        let footer = self.index(BandKind::PageFooter);
        let pf = self.height(footer);
        self.out.end = self.cursor;
        if pf > 0.0 {
            let page = self.page();
            self.out.pages[page].push(Placed {
                band: footer,
                rows: vec![page],
                y: (self.content - pf).max(0.0),
                height: pf.min(self.content),
            });
        }
    }

    fn new_page(&mut self) {
        self.finish_page();
        self.start_page();
    }

    /// Places a band in the body, on the next page if it does not fit in
    /// what is left of this one.
    fn place(&mut self, band: usize, rows: &[usize]) {
        let h = self.height(band);
        if h <= 0.0 || !(self.visible)(band, rows) {
            return;
        }
        if self.cursor + h > self.body_bottom + 0.01 && !self.at_top() {
            self.new_page();
        }
        self.put(band, rows);
    }

    /// The height of the first band after the header of the section at
    /// `level` in occurrence `rows`: what keeping the header with it needs.
    fn first_after(&self, level: usize, rows: &[usize]) -> f32 {
        let inner = (self.items)(level + 1, rows);
        let next = if inner == 0 {
            BandKind::SectionFooter(level)
        } else if level + 1 < self.sections {
            BandKind::SectionHeader(level + 1)
        } else {
            BandKind::Detail
        };
        self.height(self.index(next))
    }

    /// Draws the items of the section at `level` (or of the detail band,
    /// after the last section) in the occurrence `rows`. Returns false once
    /// the limit is reached.
    fn level(&mut self, level: usize, rows: &mut Vec<usize>) -> bool {
        let count = (self.items)(level, rows);
        for i in 0..count {
            rows.push(i);
            if level == self.sections {
                if self.details >= self.limit {
                    self.out.limited = true;
                    rows.pop();
                    return false;
                }
                self.details += 1;
                self.place(self.index(BandKind::Detail), rows);
            } else {
                let options = self.design.section_options(level);
                let header = self.index(BandKind::SectionHeader(level));
                if options.new_page && !self.at_top() {
                    self.new_page();
                }
                let h = self.height(header);
                if options.keep_with_first
                    && h > 0.0
                    && (self.visible)(header, rows)
                    && self.cursor + h + self.first_after(level, rows) > self.body_bottom + 0.01
                    && !self.at_top()
                {
                    self.new_page();
                }
                self.place(header, rows);
                self.open.push((level, rows.clone()));
                let going = self.level(level + 1, rows);
                if going {
                    self.place(self.index(BandKind::SectionFooter(level)), rows);
                }
                self.open.pop();
                if !going {
                    rows.pop();
                    return false;
                }
            }
            rows.pop();
        }
        true
    }
}
