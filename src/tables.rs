//! Editable document tables. Owned grids are marked vector content; detection
//! requires complete drawn borders and never changes a document by itself.
use anyhow::{Context, Result, ensure};
use lopdf::{
    Dictionary, Object, Stream,
    content::{Content, Operation},
    dictionary,
};
use serde::{Deserialize, Serialize};

const MARK: &[u8] = b"MyPDFTable";
/// Properties applied to a cell, or copied to its row/column/table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CellStyle {
    pub size: f32,
    pub color: [f32; 3],
    pub fill: Option<[f32; 3]>,
    pub border: [f32; 3],
    pub border_width: f32,
    /// Left, top, right, bottom, in points.
    pub insets: [f32; 4],
    pub alignment: u8,
    pub vertical: u8,
    pub bold: bool,
    pub italic: bool,
}
impl Default for CellStyle {
    fn default() -> Self {
        Self {
            size: 11.,
            color: [0.12; 3],
            fill: None,
            border: [0.35; 3],
            border_width: 0.6,
            insets: [4.; 4],
            alignment: 0,
            vertical: 0,
            bold: false,
            italic: false,
        }
    }
}
pub const TEMPLATES: [&str; 5] = ["Plain", "Business blue", "Sage", "Warm", "Ledger"];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Cell,
    Row,
    Column,
    Table,
}
/// A merged rectangle, identified by its top-left physical cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellSpan {
    pub anchor: usize,
    pub rows: usize,
    pub columns: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub id: String,
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    pub cells: Vec<String>,
    pub size: f32,
    pub padding: f32,
    /// 0 left, 1 center, 2 right.
    pub alignment: u8,
    pub color: [f32; 3],
    pub header: bool,
    #[serde(default)]
    pub styles: Vec<CellStyle>,
    #[serde(default)]
    pub template: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<CellSpan>,
    #[serde(default)]
    pub auto_height: bool,
}
impl Table {
    pub fn new(rows: usize, columns: usize, rect: [f32; 4]) -> Self {
        let [l, t, r, b] = rect;
        Self {
            id: format!(
                "{:x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            xs: (0..=columns)
                .map(|i| l + (r - l) * i as f32 / columns.max(1) as f32)
                .collect(),
            ys: (0..=rows)
                .map(|i| t + (b - t) * i as f32 / rows.max(1) as f32)
                .collect(),
            cells: vec![String::new(); rows.saturating_mul(columns).min(720)],
            size: 11.,
            padding: 3.,
            alignment: 0,
            color: [0.12; 3],
            header: false,
            styles: vec![],
            template: String::new(),
            spans: vec![],
            auto_height: false,
        }
    }
    pub fn style(&self, index: usize) -> CellStyle {
        let index = self.anchor(index);
        self.styles
            .get(index)
            .cloned()
            .unwrap_or_else(|| CellStyle {
                size: self.size,
                color: self.color,
                insets: [self.padding; 4],
                alignment: self.alignment,
                fill: if self.header && index < self.columns() {
                    Some([0.92, 0.94, 0.97])
                } else {
                    None
                },
                ..Default::default()
            })
    }
    pub fn set_style(&mut self, index: usize, scope: Scope, style: CellStyle) {
        if self.styles.len() != self.cells.len() {
            self.styles = (0..self.cells.len()).map(|i| self.style(i)).collect();
        }
        let columns = self.columns();
        for i in 0..self.cells.len() {
            if match scope {
                Scope::Cell => i == index,
                Scope::Row => i / columns == index / columns,
                Scope::Column => i % columns == index % columns,
                Scope::Table => true,
            } {
                self.styles[i] = style.clone();
            }
        }
    }
    pub fn apply_template(&mut self, which: usize) {
        let which = which.min(TEMPLATES.len() - 1);
        let accents = [
            [0.2; 3],
            [0.12, 0.27, 0.46],
            [0.17, 0.38, 0.3],
            [0.61, 0.26, 0.17],
            [0.28, 0.31, 0.35],
        ];
        self.styles = (0..self.cells.len())
            .map(|i| {
                let row = i / self.columns();
                let mut style = CellStyle {
                    border: if which == 0 {
                        [0.55; 3]
                    } else {
                        accents[which].map(|v| v * 0.45 + 0.5)
                    },
                    vertical: 1,
                    ..Default::default()
                };
                if row == 0 {
                    style.bold = true;
                    style.fill = Some(if which == 0 {
                        [0.94; 3]
                    } else {
                        accents[which]
                    });
                    if which != 0 {
                        style.color = [1.; 3];
                    }
                } else if row.is_multiple_of(2) && which != 0 {
                    style.fill = Some(accents[which].map(|v| 0.92 + v * 0.07));
                }
                style
            })
            .collect();
        self.template = TEMPLATES[which].into();
        self.header = false;
    }
    pub fn translate(&mut self, dx: f32, dy: f32) {
        for x in &mut self.xs {
            *x += dx;
        }
        for y in &mut self.ys {
            *y += dy;
        }
    }
    pub fn fit_rect(&mut self, rect: [f32; 4]) {
        let [l, t, r, b] = self.rect();
        for x in &mut self.xs {
            *x = rect[0] + (*x - l) / (r - l) * (rect[2] - rect[0]);
        }
        for y in &mut self.ys {
            *y = rect[1] + (*y - t) / (b - t) * (rect[3] - rect[1]);
        }
    }
    pub fn rows(&self) -> usize {
        self.ys.len().saturating_sub(1)
    }
    pub fn columns(&self) -> usize {
        self.xs.len().saturating_sub(1)
    }
    pub fn rect(&self) -> [f32; 4] {
        [
            self.xs[0],
            self.ys[0],
            *self.xs.last().unwrap(),
            *self.ys.last().unwrap(),
        ]
    }
    pub fn cell(&self, index: usize) -> [f32; 4] {
        let index = self.anchor(index);
        let c = index % self.columns();
        let r = index / self.columns();
        let [rows, columns] = self.span(index);
        [
            self.xs[c],
            self.ys[r],
            self.xs[c + columns],
            self.ys[r + rows],
        ]
    }
    pub fn raw_cell(&self, index: usize) -> [f32; 4] {
        let c = index % self.columns();
        let r = index / self.columns();
        [self.xs[c], self.ys[r], self.xs[c + 1], self.ys[r + 1]]
    }
    pub fn anchor(&self, index: usize) -> usize {
        let columns = self.columns();
        if columns == 0 {
            return index;
        }
        let (r, c) = (index / columns, index % columns);
        self.spans
            .iter()
            .find(|s| {
                let (top, left) = (s.anchor / columns, s.anchor % columns);
                r >= top && r < top + s.rows && c >= left && c < left + s.columns
            })
            .map_or(index, |s| s.anchor)
    }
    pub fn is_covered(&self, index: usize) -> bool {
        self.anchor(index) != index
    }
    pub fn span(&self, index: usize) -> [usize; 2] {
        let anchor = self.anchor(index);
        self.spans
            .iter()
            .find(|s| s.anchor == anchor)
            .map_or([1, 1], |s| [s.rows, s.columns])
    }
    pub fn anchors(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.cells.len()).filter(|i| !self.is_covered(*i))
    }
    /// Merge a rectangular selection without dropping any cell's text.
    /// Splitting later keeps the combined text in the top-left cell; Undo can
    /// restore the former individual values.
    pub fn merge_range(&mut self, first: usize, last: usize) -> Result<usize> {
        self.validate()?;
        ensure!(
            first < self.cells.len() && last < self.cells.len(),
            "Cell not found"
        );
        let cols = self.columns();
        let (top, bottom) = (
            (first / cols).min(last / cols),
            (first / cols).max(last / cols),
        );
        let (left, right) = (
            (first % cols).min(last % cols),
            (first % cols).max(last % cols),
        );
        ensure!(
            top != bottom || left != right,
            "Select at least two cells to merge"
        );
        for s in &self.spans {
            let (r, c) = (s.anchor / cols, s.anchor % cols);
            let overlaps = r <= bottom && r + s.rows > top && c <= right && c + s.columns > left;
            ensure!(
                !overlaps
                    || (r >= top
                        && r + s.rows <= bottom + 1
                        && c >= left
                        && c + s.columns <= right + 1),
                "Split the existing merged cell before merging part of it"
            );
        }
        let mut updated = self.clone();
        let anchor = top * cols + left;
        let mut parts = vec![];
        for r in top..=bottom {
            for c in left..=right {
                let i = r * cols + c;
                if !self.cells[i].trim().is_empty() {
                    parts.push(self.cells[i].as_str());
                }
                updated.cells[i].clear();
            }
        }
        updated.cells[anchor] = parts.join("\n");
        updated.spans.retain(|s| {
            let (r, c) = (s.anchor / cols, s.anchor % cols);
            !(r >= top && r <= bottom && c >= left && c <= right)
        });
        updated.spans.push(CellSpan {
            anchor,
            rows: bottom - top + 1,
            columns: right - left + 1,
        });
        updated.spans.sort_by_key(|s| s.anchor);
        updated.validate()?;
        *self = updated;
        Ok(anchor)
    }
    pub fn split_cell(&mut self, index: usize) -> Result<bool> {
        self.validate()?;
        ensure!(index < self.cells.len(), "Cell not found");
        let anchor = self.anchor(index);
        let old = self.spans.len();
        self.spans.retain(|s| s.anchor != anchor);
        Ok(old != self.spans.len())
    }
    /// Paste an Excel/TSV rectangle atomically, optionally appending rows and
    /// columns. Existing merges must be split before pasting into their cells.
    pub fn paste_grid(&mut self, start: usize, text: &str, grow: bool) -> Result<usize> {
        self.validate()?;
        ensure!(start < self.cells.len(), "Cell not found");
        let grid = parse_grid(text)?;
        let (row, column) = (start / self.columns(), start % self.columns());
        let rows = row + grid.len();
        let columns = column + grid[0].len();
        ensure!(
            rows <= 60 && columns <= 12,
            "Pasted cells exceed the 60 row / 12 column limit"
        );
        ensure!(
            grow || (rows <= self.rows() && columns <= self.columns()),
            "The pasted cells do not fit. Allow table growth or choose a smaller range"
        );
        let mut updated = self.clone();
        updated.grow_to(rows.max(self.rows()), columns.max(self.columns()))?;
        for (r, values) in grid.iter().enumerate() {
            for (c, value) in values.iter().enumerate() {
                let i = (row + r) * updated.columns() + column + c;
                ensure!(
                    updated.span(i) == [1, 1],
                    "Split merged cells before pasting into them"
                );
                updated.cells[i] = value.clone();
            }
        }
        updated.validate()?;
        *self = updated;
        Ok(grid.len() * grid[0].len())
    }
    fn grow_to(&mut self, rows: usize, columns: usize) -> Result<()> {
        let (old_rows, old_columns) = (self.rows(), self.columns());
        if rows == old_rows && columns == old_columns {
            return Ok(());
        }
        let mut cells = vec![String::new(); rows * columns];
        let mut styles = vec![CellStyle::default(); rows * columns];
        for r in 0..rows {
            for c in 0..columns {
                let previous = r.min(old_rows - 1) * old_columns + c.min(old_columns - 1);
                styles[r * columns + c] = self
                    .styles
                    .get(previous)
                    .cloned()
                    .unwrap_or_else(|| self.style(previous));
                if r < old_rows && c < old_columns {
                    cells[r * columns + c] = self.cells[r * old_columns + c].clone();
                }
            }
        }
        let width = self.xs[old_columns] - self.xs[old_columns - 1];
        let height = self.ys[old_rows] - self.ys[old_rows - 1];
        for _ in old_columns..columns {
            self.xs.push(self.xs.last().unwrap() + width);
        }
        for _ in old_rows..rows {
            self.ys.push(self.ys.last().unwrap() + height);
        }
        for span in &mut self.spans {
            span.anchor = span.anchor / old_columns * columns + span.anchor % old_columns;
        }
        self.cells = cells;
        self.styles = styles;
        self.validate()
    }
    /// Fit every row to the same wrapped/font-metric layout used for PDF and
    /// vector rendering. No geometry changes are kept when it cannot fit.
    pub fn fit_row_heights(&mut self, max_bottom: f32) -> Result<()> {
        fit_heights(self, max_bottom)
    }
    pub fn hit(&self, p: [f32; 2]) -> Option<usize> {
        self.anchors().find(|i| {
            let [l, t, r, b] = self.cell(*i);
            p[0] >= l && p[0] <= r && p[1] >= t && p[1] <= b
        })
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=60).contains(&self.rows())
                && (1..=12).contains(&self.columns())
                && self.cells.len() == self.rows() * self.columns(),
            "Choose 1–60 rows and 1–12 columns"
        );
        ensure!(
            self.xs
                .windows(2)
                .all(|w| w[0].is_finite() && w[1].is_finite() && w[1] - w[0] >= 12.)
                && self
                    .ys
                    .windows(2)
                    .all(|w| w[0].is_finite() && w[1].is_finite() && w[1] - w[0] >= 12.),
            "Cells must be at least 12 pt wide and tall"
        );
        ensure!(
            (4. ..=72.).contains(&self.size)
                && (0. ..=20.).contains(&self.padding)
                && self.alignment <= 2
                && self.color.iter().all(|v| (0. ..=1.).contains(v)),
            "Invalid table text appearance"
        );
        ensure!(
            self.cells.iter().all(|s| s.len() <= 32768)
                && self.cells.iter().map(String::len).sum::<usize>() <= 512 * 1024,
            "Table text is too long"
        );
        ensure!(
            self.styles.is_empty() || self.styles.len() == self.cells.len(),
            "Invalid cell properties"
        );
        let mut covered = vec![false; self.cells.len()];
        for span in &self.spans {
            ensure!(
                span.anchor < self.cells.len()
                    && span.rows > 0
                    && span.columns > 0
                    && (span.rows > 1 || span.columns > 1),
                "Invalid merged cell"
            );
            let (r, c) = (span.anchor / self.columns(), span.anchor % self.columns());
            ensure!(
                span.rows <= self.rows() - r && span.columns <= self.columns() - c,
                "Merged cell extends outside the table"
            );
            for row in r..r + span.rows {
                for col in c..c + span.columns {
                    let i = row * self.columns() + col;
                    ensure!(!covered[i], "Merged cells overlap");
                    ensure!(
                        i == span.anchor || self.cells[i].is_empty(),
                        "Covered cells cannot contain hidden text"
                    );
                    covered[i] = true;
                }
            }
        }
        for s in &self.styles {
            ensure!(
                (4. ..=72.).contains(&s.size)
                    && s.alignment <= 2
                    && s.vertical <= 2
                    && s.insets.iter().all(|v| (0. ..=36.).contains(v))
                    && (0. ..=8.).contains(&s.border_width)
                    && s.color
                        .iter()
                        .chain(s.border.iter())
                        .chain(s.fill.iter().flatten())
                        .all(|v| (0. ..=1.).contains(v)),
                "Invalid cell properties"
            );
        }
        Ok(())
    }
    pub fn resize_structure(&mut self, row: bool, at: usize, insert: bool) -> Result<()> {
        self.validate()?;
        ensure!(
            self.spans.is_empty(),
            "Split merged cells before adding or removing rows and columns"
        );
        let (rows, cols) = (self.rows(), self.columns());
        let count = if row { rows } else { cols };
        ensure!(
            at < count && (insert || count > 1),
            "Keep at least one row and column"
        );
        ensure!(
            !insert || count < if row { 60 } else { 12 },
            "Table size limit reached"
        );
        let mut edges = if row {
            self.ys.clone()
        } else {
            self.xs.clone()
        };
        if insert {
            let span = edges[at + 1] - edges[at];
            edges.insert(at + 1, edges[at + 1]);
            for e in &mut edges[at + 2..] {
                *e += span;
            }
        } else {
            let span = edges[at + 1] - edges[at];
            edges.remove(at + 1);
            for e in &mut edges[at + 1..] {
                *e -= span;
            }
        }
        let (nr, nc) = if row {
            (if insert { rows + 1 } else { rows - 1 }, cols)
        } else {
            (rows, if insert { cols + 1 } else { cols - 1 })
        };
        let old = self.cells.clone();
        let old_styles: Vec<_> = (0..self.cells.len()).map(|i| self.style(i)).collect();
        let mut styles = vec![];
        let mut cells = Vec::new();
        for r in 0..nr {
            for c in 0..nc {
                let v = if row { r } else { c };
                if insert && v == at {
                    cells.push(String::new());
                    styles.push(old_styles[(r.min(rows - 1)) * cols + c.min(cols - 1)].clone());
                    continue;
                }
                let orig = if insert {
                    if v > at { v - 1 } else { v }
                } else if v >= at {
                    v + 1
                } else {
                    v
                };
                let (rr, cc) = if row { (orig, c) } else { (r, orig) };
                cells.push(old[rr * cols + cc].clone());
                styles.push(old_styles[rr * cols + cc].clone());
            }
        }
        self.cells = cells;
        self.styles = styles;
        if row {
            self.ys = edges;
        } else {
            self.xs = edges;
        }
        Ok(())
    }
}
/// Excel's text clipboard format uses tabs, quoted fields and doubled quotes.
/// Parsing is bounded before it can change a table; a terminal line ending is
/// a terminator rather than an extra blank row.
pub fn parse_grid(text: &str) -> Result<Vec<Vec<String>>> {
    ensure!(text.len() <= 1024 * 1024, "Pasted table text is too long");
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut row_ended = false;
    let mut chars = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .chars()
        .peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else if c == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                field.push('\n');
            } else {
                field.push(c);
            }
            ensure!(field.len() <= 32768, "A pasted cell exceeds 32 KB of text");
            row_ended = false;
            continue;
        }
        ensure!(
            !closed || matches!(c, '\t' | '\r' | '\n'),
            "Unexpected text after a quoted cell"
        );
        match c {
            '"' if field.is_empty() && !closed => {
                quoted = true;
                row_ended = false;
            }
            '\t' | '\r' | '\n' => {
                ensure!(row.len() < 12, "Pasted cells exceed the 12 column limit");
                row.push(std::mem::take(&mut field));
                closed = false;
                row_ended = c != '\t';
                if row_ended {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    ensure!(rows.len() < 60, "Pasted cells exceed the 60 row limit");
                    rows.push(std::mem::take(&mut row));
                }
            }
            _ => {
                field.push(c);
                row_ended = false;
            }
        }
        ensure!(field.len() <= 32768, "A pasted cell exceeds 32 KB of text");
    }
    ensure!(
        !quoted,
        "The pasted table contains an unfinished quoted cell"
    );
    if !row_ended || !row.is_empty() || !field.is_empty() {
        ensure!(
            row.len() < 12 && rows.len() < 60,
            "Pasted cells exceed the 60 row / 12 column limit"
        );
        row.push(field);
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(vec![String::new()]);
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1);
    for row in &mut rows {
        row.resize(columns, String::new());
    }
    ensure!(
        rows.iter().flatten().map(String::len).sum::<usize>() <= 512 * 1024,
        "Pasted table text exceeds 512 KB"
    );
    Ok(rows)
}
fn table_writers() -> Result<Vec<crate::unicode_text::Writer>> {
    let fallback = crate::document::default_font()?;
    let fonts = crate::text_style::fonts();
    (0..4)
        .map(|i| {
            let font = fonts
                .iter()
                .find(|f| f.family == "Arial" && f.bold == (i & 1 != 0) && f.italic == (i & 2 != 0))
                .map(|f| f.path.clone())
                .unwrap_or_else(|| fallback.clone());
            crate::unicode_text::Writer::new(&crate::unicode_text::Preferred::File(font))
        })
        .collect()
}
struct LayoutLine {
    line: crate::unicode_text::Line,
    text: String,
    x: f32,
    y: f32,
    style: CellStyle,
    writer: usize,
}
struct MeasuredLine {
    line: crate::unicode_text::Line,
    text: String,
    width: f32,
    baseline: f32,
    height: f32,
}
fn measured_lines(
    text: &str,
    style: &CellStyle,
    width: f32,
    writer: &mut crate::unicode_text::Writer,
) -> Result<Vec<MeasuredLine>> {
    ensure!(width > 0. && width.is_finite(), "Reduce cell edge spacing");
    writer
        .wrap(text, style.size, width)?
        .into_iter()
        .map(|(text, _)| {
            let line = writer.shape(&text)?;
            let advance = line.width(style.size);
            ensure!(advance <= width + 0.1, "A character is wider than the cell");
            let [ascent, descent, gap] = writer.line_metrics(&line, style.size);
            let height = (ascent + descent + gap).max(style.size * 1.1);
            Ok(MeasuredLine {
                line,
                text,
                width: advance,
                baseline: ascent + (height - ascent - descent) / 2.,
                height,
            })
        })
        .collect()
}
fn fit_heights(table: &mut Table, max_bottom: f32) -> Result<()> {
    table.validate()?;
    ensure!(
        max_bottom.is_finite() && max_bottom > table.ys[0],
        "Invalid page bottom"
    );
    let mut writers = table_writers()?;
    let mut heights = vec![12_f32; table.rows()];
    let mut spanning = vec![];
    for i in table.anchors() {
        let style = table.style(i);
        let [l, _, r, _] = table.cell(i);
        let writer = usize::from(style.bold) + usize::from(style.italic) * 2;
        let lines = measured_lines(
            &table.cells[i],
            &style,
            r - l - style.insets[0] - style.insets[2],
            &mut writers[writer],
        )?;
        let need =
            lines.iter().map(|line| line.height).sum::<f32>() + style.insets[1] + style.insets[3];
        let row = i / table.columns();
        let rows = table.span(i)[0];
        if rows == 1 {
            heights[row] = heights[row].max(need);
        } else {
            spanning.push((row, rows, need));
        }
    }
    spanning.sort_by_key(|(_, rows, _)| *rows);
    for (row, rows, need) in spanning {
        let spare = (need - heights[row..row + rows].iter().sum::<f32>()).max(0.);
        for height in &mut heights[row..row + rows] {
            *height += spare / rows as f32;
        }
    }
    let bottom = table.ys[0] + heights.iter().sum::<f32>();
    ensure!(
        bottom <= max_bottom + 0.01,
        "The fitted table would extend beyond the page. Shorten the text, widen columns or reduce font size"
    );
    let mut updated = table.clone();
    updated.ys.truncate(1);
    for height in heights {
        updated.ys.push(updated.ys.last().unwrap() + height);
    }
    updated.validate()?;
    layout_lines(&updated, &mut writers)?;
    *table = updated;
    Ok(())
}
fn layout_lines(
    table: &Table,
    writers: &mut [crate::unicode_text::Writer],
) -> Result<Vec<LayoutLine>> {
    let mut out = vec![];
    for i in table.anchors() {
        let text = &table.cells[i];
        let [l, t, r, b] = table.cell(i);
        let style = table.style(i);
        let [left, top, right, bottom] = style.insets;
        let width = r - l - left - right;
        let height = b - t - top - bottom;
        ensure!(width > 0. && height > 0., "Reduce cell edge spacing");
        let index = usize::from(style.bold) + usize::from(style.italic) * 2;
        let writer = &mut writers[index];
        let lines = measured_lines(text, &style, width, writer)?;
        let used = lines.iter().map(|line| line.height).sum::<f32>();
        ensure!(
            text.trim().is_empty() || used <= height + 0.1,
            "Text in row {}, column {} does not fit. Enlarge the row or reduce font size.",
            i / table.columns() + 1,
            i % table.columns() + 1
        );
        let mut y = t
            + top
            + match style.vertical {
                1 => (height - used).max(0.) / 2.,
                2 => (height - used).max(0.),
                _ => 0.,
            };
        for line in lines {
            let x = match style.alignment {
                1 => l + left + (width - line.width) / 2.,
                2 => r - right - line.width,
                _ => l + left,
            };
            out.push(LayoutLine {
                line: line.line,
                text: line.text,
                x,
                y: y + line.baseline,
                style: style.clone(),
                writer: index,
            });
            y += line.height;
        }
    }
    Ok(out)
}
/// A structured table remains a separate movable vector annotation.
pub fn layer(
    table: &Table,
    page: usize,
    previous: Option<&crate::annotations::Annotation>,
) -> Result<crate::annotations::Annotation> {
    table.validate()?;
    let [l, t, r, b] = table.rect();
    let mut writers = table_writers()?;
    let lines = layout_lines(table, &mut writers)?;
    let hex = |c: [f32; 3]| {
        format!(
            "#{:02X}{:02X}{:02X}",
            (c[0] * 255.) as u8,
            (c[1] * 255.) as u8,
            (c[2] * 255.) as u8
        )
    };
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        r - l,
        b - t,
        r - l,
        b - t
    );
    for i in table.anchors() {
        let [x, y, right, bottom] = table.cell(i);
        let s = table.style(i);
        // Keep the outside stroke inside the SVG viewport and annotation bounds.
        let half = s.border_width / 2.;
        let x0 = x - l + if (x - l).abs() < 0.01 { half } else { 0. };
        let y0 = y - t + if (y - t).abs() < 0.01 { half } else { 0. };
        let x1 = right - l - if (right - r).abs() < 0.01 { half } else { 0. };
        let y1 = bottom - t - if (bottom - b).abs() < 0.01 { half } else { 0. };
        svg.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",x0,y0,x1-x0,y1-y0,s.fill.map(hex).unwrap_or_else(||"none".into()),hex(s.border),s.border_width));
    }
    for line in lines {
        svg.push_str(&format!("<text x=\"{}\" y=\"{}\" font-family=\"Arial\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" fill=\"{}\">{}</text>",line.x-l,line.y-t,line.style.size,if line.style.bold{"bold"}else{"normal"},if line.style.italic{"italic"}else{"normal"},hex(line.style.color),quick_xml::escape::escape(&line.text)));
    }
    svg.push_str("</svg>");
    crate::svg::tree(&svg, None)?;
    let mut mark = previous.cloned().unwrap_or_else(|| {
        crate::annotations::Annotation::new(
            page,
            table.rect(),
            crate::annotations::Kind::Svg {
                source: String::new(),
                tint: false,
            },
            Default::default(),
        )
    });
    mark.rect = table.rect();
    mark.kind = crate::annotations::Kind::Svg {
        source: svg,
        tint: false,
    };
    mark.object.table = Some(table.clone());
    mark.object.name = if table.template.is_empty() {
        "Table".into()
    } else {
        format!("Table · {}", table.template)
    };
    mark.style.color = [0.2; 3];
    mark.validate()?;
    Ok(mark)
}
pub fn from_layer(mark: &crate::annotations::Annotation) -> Option<Table> {
    let mut table = mark.object.table.clone()?;
    table.validate().ok()?;
    if mark.rotation.abs() > 0.01 || mark.flip_x || mark.flip_y {
        return None;
    }
    table.fit_rect(mark.rect);
    table.validate().ok()?;
    Some(table)
}
fn marked(op: &Operation) -> Option<Table> {
    if op.operator != "BDC" {
        return None;
    }
    let d = op.operands.get(1)?.as_dict().ok()?;
    let bytes = d.get(MARK).ok()?.as_str().ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let table: Table = serde_json::from_slice(bytes).ok()?;
    table.validate().ok()?;
    Some(table)
}
pub fn owned(bytes: &[u8], page: u32) -> Result<Vec<Table>> {
    let doc = lopdf::Document::load_mem(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let tables: Vec<Table> = doc
        .get_and_decode_page_content(id)?
        .operations
        .iter()
        .filter_map(marked)
        .take(100)
        .collect();
    let runs = crate::text_geometry::runs(bytes, page)?;
    let normalized = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    // Other editors may alter cell content without updating our table marker.
    // Treat those as ordinary detected grids rather than restoring stale words.
    Ok(tables
        .into_iter()
        .filter(|table| {
            table.anchors().all(|i| {
                let text = &table.cells[i];
                text_from_runs(&runs, table.cell(i))
                    .is_ok_and(|(actual, _)| normalized(&actual) == normalized(text))
            })
        })
        .collect())
}
pub fn write(bytes: &[u8], page: u32, table: &Table) -> Result<Vec<u8>> {
    table.validate()?;
    let geometry = crate::annotations::page_geometry(bytes)?;
    let (size, transform) = geometry
        .get(page.saturating_sub(1) as usize)
        .context("Page not found")?;
    let [l, t, r, b] = table.rect();
    ensure!(
        l >= 0. && t >= 0. && r <= size[2] + 0.1 && b <= size[3] + 0.1,
        "The table extends beyond the page"
    );
    let mut doc = crate::content::editable(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let original = doc.get_and_decode_page_content(id)?;
    let mut kept = Vec::new();
    let mut skip = 0usize;
    for op in original.operations {
        if skip > 0 {
            if op.operator == "BDC" || op.operator == "BMC" {
                skip += 1;
            } else if op.operator == "EMC" {
                skip -= 1;
            }
            continue;
        }
        if marked(&op).is_some_and(|old| old.id == table.id) {
            skip = 1;
            continue;
        }
        kept.push(op);
    }
    ensure!(skip == 0, "Unbalanced table content");
    let mut writers = table_writers()?;
    let shaped = layout_lines(table, &mut writers)?;
    let mut resources = crate::content::resources(&doc, id)?;
    let mut font_dict = resources
        .get(b"Font")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let mut n = 0;
    let mut names = Vec::new();
    for writer in &mut writers {
        let fonts = writer.embed(&mut doc)?;
        let group: Vec<_> = fonts
            .into_iter()
            .map(|f| {
                f.map(|f| {
                    while font_dict.has(format!("MyPDFTbl{n}").as_bytes()) {
                        n += 1;
                    }
                    let name = format!("MyPDFTbl{n}");
                    font_dict.set(name.as_str(), f);
                    n += 1;
                    name
                })
            })
            .collect();
        names.push(group);
    }
    resources.set("Font", font_dict);
    let nums = |a: &[f32]| a.iter().copied().map(Object::Real).collect();
    let mut ops = vec![
        Operation::new(
            "BDC",
            vec![
                Object::Name(b"Table".to_vec()),
                Object::Dictionary(
                    dictionary! {MARK=>Object::string_literal(serde_json::to_vec(table)?)},
                ),
            ],
        ),
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            transform.iter().map(|v| Object::Real(*v as f32)).collect(),
        ),
    ];
    for i in table.anchors() {
        let [l, t, r, b] = table.cell(i);
        let style = table.style(i);
        if let Some(fill) = style.fill {
            ops.extend([
                Operation::new("rg", nums(&fill)),
                Operation::new("re", nums(&[l, t, r - l, b - t])),
                Operation::new("f", vec![]),
            ]);
        }
        if style.border_width > 0. {
            ops.extend([
                Operation::new("RG", nums(&style.border)),
                Operation::new("w", nums(&[style.border_width])),
                Operation::new("re", nums(&[l, t, r - l, b - t])),
                Operation::new("S", vec![]),
            ]);
        }
    }
    for item in shaped {
        ops.extend([
            Operation::new("BT", vec![]),
            Operation::new("rg", nums(&item.style.color)),
            Operation::new("Tm", nums(&[1., 0., 0., -1., item.x, item.y])),
        ]);
        ops.extend(writers[item.writer].operations(
            &item.line,
            &names[item.writer],
            item.style.size,
            0.,
            0.,
        )?);
        ops.push(Operation::new("ET", vec![]));
    }
    ops.extend([Operation::new("Q", vec![]), Operation::new("EMC", vec![])]);
    let (before, after) = crate::header_footer::isolation(&doc, id)?;
    let mut data = before;
    data.extend(Content { operations: kept }.encode()?);
    data.extend(after);
    data.extend(Content { operations: ops }.encode()?);
    let stream = doc.add_object(Stream::new(Dictionary::new(), data));
    let p = doc.get_dictionary_mut(id)?;
    p.set("Contents", stream);
    p.set("Resources", resources);
    crate::content::serialize(&mut doc)
}

fn contains(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] - 0.5 && p[0] <= r[2] + 0.5 && p[1] >= r[1] - 0.5 && p[1] <= r[3] + 0.5
}
/// Text wholly contained in a cell, ordered by baseline and horizontal position.
pub fn cell_text(
    bytes: &[u8],
    page: u32,
    rect: [f32; 4],
) -> Result<(String, crate::text_block::Selection)> {
    let runs = crate::text_geometry::runs(bytes, page)?;
    text_from_runs(&runs, rect)
}
fn text_from_runs(
    runs: &[crate::document::TextRun],
    rect: [f32; 4],
) -> Result<(String, crate::text_block::Selection)> {
    let mut selected: Vec<_> = runs
        .iter()
        .enumerate()
        .filter(|(_, run)| {
            run.rect
                .is_some_and(|r| contains(rect, [(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.]))
        })
        .filter(|(_, r)| !r.text.trim().is_empty())
        .collect();
    for (_, run) in &selected {
        let r = run.rect.unwrap();
        ensure!(
            contains(rect, [r[0], r[1]]) && contains(rect, [r[2], r[3]]),
            "Text crosses a cell boundary; adjust the detected grid first"
        );
    }
    selected.sort_by(|a, b| {
        let a = a.1.rect.unwrap();
        let b = b.1.rect.unwrap();
        if (a[1] - b[1]).abs() > 3. {
            a[1].total_cmp(&b[1])
        } else {
            a[0].total_cmp(&b[0])
        }
    });
    let mut text = String::new();
    let mut prev: Option<[f32; 4]> = None;
    for (_, run) in &selected {
        let r = run.rect.unwrap();
        if let Some(p) = prev {
            if (r[1] - p[1]).abs() > 3. {
                text.push('\n');
            } else if !text.ends_with(char::is_whitespace)
                && !run.text.starts_with(char::is_whitespace)
                && r[0] - p[2] > run.size * 0.15
            {
                text.push(' ');
            }
        }
        text.push_str(&run.text);
        prev = Some(r);
    }
    Ok((
        text,
        crate::text_block::Selection {
            runs: selected
                .into_iter()
                .map(|(i, r)| (i, r.text.clone()))
                .collect(),
            annotations: vec![],
        },
    ))
}
pub fn edit_cell(
    bytes: &[u8],
    page: u32,
    table: &Table,
    index: usize,
    text: &str,
) -> Result<Vec<u8>> {
    table.validate()?;
    ensure!(index < table.cells.len(), "Cell not found");
    let index = table.anchor(index);
    if owned(bytes, page)?.iter().any(|t| t.id == table.id) {
        let mut updated = table.clone();
        updated.cells[index] = text.into();
        return write(bytes, page, &updated);
    }
    let [l, t, r, b] = table.cell(index);
    let (_, selection) = cell_text(bytes, page, [l, t, r, b])?;
    let style = table.style(index);
    let [left, top, right, bottom] = style.insets;
    let rect = [
        l + left,
        t + top,
        r - l - left - right,
        b - t - top - bottom,
    ];
    let font = crate::text_style::fonts()
        .iter()
        .find(|f| f.family == "Arial" && f.bold == style.bold && f.italic == style.italic)
        .map(|f| f.path.clone());
    let format = crate::text_style::TextFormat {
        font,
        alignment: match style.alignment {
            1 => crate::text_style::Alignment::Center,
            2 => crate::text_style::Alignment::Right,
            _ => crate::text_style::Alignment::Left,
        },
    };
    let mut stripped = if selection.runs.is_empty() {
        bytes.to_vec()
    } else {
        crate::text_block::replace(
            bytes,
            page,
            &crate::text_block::Replacement {
                selection,
                spans: vec![],
                layout: vec![],
                text: String::new(),
                rect,
                size: style.size,
                color: style.color,
                format: format.clone(),
            },
        )?
    };
    stripped = paint_cell_style(&stripped, page, table.cell(index), &style)?;
    if text.trim().is_empty() {
        return Ok(stripped);
    }
    let mut writer = crate::unicode_text::Writer::new(&crate::unicode_text::Preferred::File(
        format
            .font
            .clone()
            .unwrap_or(crate::document::default_font()?),
    ))?;
    let lines = writer.wrap(text, style.size, rect[2])?;
    let used = lines.len() as f32 * style.size * 1.25;
    let y = rect[1]
        + match style.vertical {
            1 => (rect[3] - used).max(0.) / 2.,
            2 => (rect[3] - used).max(0.),
            _ => 0.,
        };
    crate::content::add_text_styled(
        &stripped,
        page,
        text,
        [rect[0], y, rect[2], (rect[1] + rect[3] - y).max(1.)],
        style.size,
        style.color,
        &format,
    )
}
const CELL_STYLE: &[u8] = b"MyPDFCellStyle";
fn style_marks(bytes: &[u8], page: u32) -> Result<Vec<([f32; 4], CellStyle)>> {
    let doc = lopdf::Document::load_mem(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    Ok(doc
        .get_and_decode_page_content(id)?
        .operations
        .iter()
        .filter_map(|op| {
            if op.operator != "BDC" {
                return None;
            }
            let s = op
                .operands
                .get(1)?
                .as_dict()
                .ok()?
                .get(CELL_STYLE)
                .ok()?
                .as_str()
                .ok()?;
            serde_json::from_slice(s).ok()
        })
        .collect())
}
fn paint_cell_style(bytes: &[u8], page: u32, rect: [f32; 4], style: &CellStyle) -> Result<Vec<u8>> {
    let mut doc = crate::content::editable(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let content = doc.get_and_decode_page_content(id)?;
    let mut kept = vec![];
    let mut skip = 0usize;
    for op in content.operations {
        if skip > 0 {
            if op.operator == "BDC" || op.operator == "BMC" {
                skip += 1;
            } else if op.operator == "EMC" {
                skip -= 1;
            }
            continue;
        }
        let same = op
            .operands
            .get(1)
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(CELL_STYLE).ok())
            .and_then(|o| o.as_str().ok())
            .and_then(|v| serde_json::from_slice::<([f32; 4], CellStyle)>(v).ok())
            .is_some_and(|(r, _)| r.iter().zip(rect).all(|(a, b)| (*a - b).abs() < 0.1));
        if op.operator == "BDC" && same {
            skip = 1;
        } else {
            kept.push(op);
        }
    }
    let geometry = crate::annotations::page_geometry(bytes)?;
    let transform = geometry[page as usize - 1].1;
    let nums = |v: &[f32]| v.iter().copied().map(Object::Real).collect();
    let [l, t, r, b] = rect;
    let inset = style.border_width / 2.;
    let mut ops = vec![
        Operation::new(
            "BDC",
            vec![
                Object::Name(b"Span".to_vec()),
                Object::Dictionary(
                    dictionary! {CELL_STYLE=>Object::string_literal(serde_json::to_vec(&(rect,style))?)},
                ),
            ],
        ),
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            transform.iter().map(|v| Object::Real(*v as f32)).collect(),
        ),
    ];
    if let Some(fill) = style.fill {
        ops.extend([
            Operation::new("rg", nums(&fill)),
            Operation::new(
                "re",
                nums(&[l + inset, t + inset, r - l - 2. * inset, b - t - 2. * inset]),
            ),
            Operation::new("f", vec![]),
        ]);
    }
    if style.border_width > 0. {
        ops.extend([
            Operation::new("RG", nums(&style.border)),
            Operation::new("w", nums(&[style.border_width])),
            Operation::new("re", nums(&[l, t, r - l, b - t])),
            Operation::new("S", vec![]),
        ]);
    }
    ops.extend([Operation::new("Q", vec![]), Operation::new("EMC", vec![])]);
    let (before, after) = crate::header_footer::isolation(&doc, id)?;
    let mut data = before;
    data.extend(Content { operations: kept }.encode()?);
    data.extend(after);
    data.extend(Content { operations: ops }.encode()?);
    let stream = doc.add_object(Stream::new(Dictionary::new(), data));
    doc.get_dictionary_mut(id)?.set("Contents", stream);
    crate::content::serialize(&mut doc)
}
/// Apply cell/row/column styling to a detected grid in one document transaction.
pub fn edit_detected(bytes: &[u8], page: u32, table: &Table, base: &Table) -> Result<Vec<u8>> {
    table.validate()?;
    base.validate()?;
    ensure!(
        table.xs == base.xs && table.ys == base.ys && table.spans == base.spans,
        "Detach the detected grid before resizing it"
    );
    let mut current = bytes.to_vec();
    for i in table.anchors() {
        if table.cells[i] != base.cells[i] || table.style(i) != base.style(i) {
            current = edit_cell(&current, page, table, i, &table.cells[i])?;
        }
    }
    Ok(current)
}

struct GridPaint {
    rect: [f32; 4],
    fill: Option<[f32; 3]>,
    stroke: Option<([f32; 3], f32)>,
}
fn grid_paints(bytes: &[u8], page: u32) -> Result<Vec<GridPaint>> {
    use hayro::kurbo::{Affine, Point};
    let doc = lopdf::Document::load_mem(bytes)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let display =
        Affine::new(crate::annotations::page_geometry(bytes)?[page as usize - 1].1).inverse();
    let mut state = (Affine::IDENTITY, [0.; 3], [0.; 3], 1.);
    let mut stack = vec![];
    let mut points: Vec<Point> = vec![];
    let mut straight = true;
    let mut out = vec![];
    for op in doc.get_and_decode_page_content(id)?.operations {
        let n: Vec<_> = op
            .operands
            .iter()
            .filter_map(|v| v.as_float().ok())
            .collect();
        match op.operator.as_str() {
            "q" => stack.push(state),
            "Q" => state = stack.pop().unwrap_or(state),
            "cm" if n.len() == 6 => {
                state.0 *= Affine::new(
                    n.iter()
                        .map(|v| *v as f64)
                        .collect::<Vec<_>>()
                        .try_into()
                        .unwrap(),
                )
            }
            "rg" if n.len() == 3 => state.1 = [n[0], n[1], n[2]],
            "RG" if n.len() == 3 => state.2 = [n[0], n[1], n[2]],
            "g" if n.len() == 1 => state.1 = [n[0]; 3],
            "G" if n.len() == 1 => state.2 = [n[0]; 3],
            "w" if n.len() == 1 => state.3 = n[0],
            "m" | "l" if n.len() == 2 => {
                points.push(display * state.0 * Point::new(n[0] as f64, n[1] as f64))
            }
            "re" if n.len() == 4 => {
                for [x, y] in [
                    [n[0], n[1]],
                    [n[0] + n[2], n[1]],
                    [n[0] + n[2], n[1] + n[3]],
                    [n[0], n[1] + n[3]],
                    [n[0], n[1]],
                ] {
                    points.push(display * state.0 * Point::new(x as f64, y as f64));
                }
            }
            "c" | "v" | "y" | "W" | "W*" => straight = false,
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                if straight
                    && points.len() >= 2
                    && points
                        .windows(2)
                        .all(|p| (p[0].x - p[1].x).abs() < 0.5 || (p[0].y - p[1].y).abs() < 0.5)
                {
                    let rect = [
                        points
                            .iter()
                            .map(|p| p.x as f32)
                            .fold(f32::INFINITY, f32::min),
                        points
                            .iter()
                            .map(|p| p.y as f32)
                            .fold(f32::INFINITY, f32::min),
                        points
                            .iter()
                            .map(|p| p.x as f32)
                            .fold(f32::NEG_INFINITY, f32::max),
                        points
                            .iter()
                            .map(|p| p.y as f32)
                            .fold(f32::NEG_INFINITY, f32::max),
                    ];
                    let fill = matches!(
                        op.operator.as_str(),
                        "f" | "F" | "f*" | "B" | "B*" | "b" | "b*"
                    )
                    .then_some(state.1);
                    let stroke =
                        matches!(op.operator.as_str(), "S" | "s" | "B" | "B*" | "b" | "b*")
                            .then_some((state.2, state.3));
                    out.push(GridPaint { rect, fill, stroke });
                }
                points.clear();
                straight = true;
            }
            _ => {}
        }
    }
    Ok(out)
}
fn detected_style(
    cell: [f32; 4],
    runs: &[crate::document::TextRun],
    paints: &[GridPaint],
) -> CellStyle {
    let mut style = CellStyle {
        insets: [3.; 4],
        ..Default::default()
    };
    for paint in paints {
        let r = paint.rect;
        if r[0] <= cell[0] + 0.5
            && r[1] <= cell[1] + 0.5
            && r[2] >= cell[2] - 0.5
            && r[3] >= cell[3] - 0.5
            && let Some(fill) = paint.fill
        {
            style.fill = Some(fill);
        }
        if let Some((color, width)) = paint.stroke
            && r.iter().zip(cell).all(|(a, b)| (*a - b).abs() < 1.)
        {
            style.border = color;
            style.border_width = width.clamp(0., 8.);
        }
    }
    if let Some(run) = runs.iter().find(|r| {
        r.rect
            .is_some_and(|r| contains(cell, [(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.]))
    }) {
        style.size = run.size.clamp(4., 72.);
        style.color = run.color;
        let name = run.base_font.to_lowercase();
        style.bold = name.contains("bold");
        style.italic = name.contains("italic") || name.contains("oblique");
        if let Some(rect) = run.rect {
            let left = rect[0] - cell[0];
            let right = cell[2] - rect[2];
            style.alignment = if (left - right).abs() < 4. {
                1
            } else if right < left && right < 12. {
                2
            } else {
                0
            };
            if ((rect[1] + rect[3] - cell[1] - cell[3]) / 2.).abs() < style.size * 0.4 {
                style.vertical = 1;
            }
        }
    }
    style
}

fn infer_spans(table: &mut Table, hs: &[[f32; 3]], vs: &[[f32; 3]]) -> Result<()> {
    let covers = |edges: &[[f32; 3]], line: f32, start: f32, end: f32| {
        edges
            .iter()
            .any(|e| (e[0] - line).abs() < 1.5 && e[1] <= start + 1.5 && e[2] >= end - 1.5)
    };
    let complete_or_empty = |edges: &[[f32; 3]], line: f32, start: f32, end: f32| -> Result<bool> {
        let complete = covers(edges, line, start, end);
        ensure!(
            complete
                || !edges
                    .iter()
                    .any(|e| (e[0] - line).abs() < 1.5 && e[1] < end - 1.5 && e[2] > start + 1.5),
            "An incomplete border is not a rectangular merged cell"
        );
        Ok(complete)
    };
    let [l, t, r, b] = table.rect();
    ensure!(
        covers(hs, t, l, r) && covers(hs, b, l, r) && covers(vs, l, t, b) && covers(vs, r, t, b),
        "The table's outer border is incomplete"
    );
    let columns = table.columns();
    let mut parents: Vec<_> = (0..table.cells.len()).collect();
    fn root(parents: &[usize], mut i: usize) -> usize {
        while parents[i] != i {
            i = parents[i];
        }
        i
    }
    let mut present = vec![];
    for row in 0..table.rows() {
        for col in 1..columns {
            let a = row * columns + col - 1;
            let b = a + 1;
            if complete_or_empty(vs, table.xs[col], table.ys[row], table.ys[row + 1])? {
                present.push((a, b));
            } else {
                let a = root(&parents, a);
                let b = root(&parents, b);
                parents[b] = a;
            }
        }
    }
    for row in 1..table.rows() {
        for col in 0..columns {
            let a = (row - 1) * columns + col;
            let b = row * columns + col;
            if complete_or_empty(hs, table.ys[row], table.xs[col], table.xs[col + 1])? {
                present.push((a, b));
            } else {
                let a = root(&parents, a);
                let b = root(&parents, b);
                parents[b] = a;
            }
        }
    }
    ensure!(
        present
            .iter()
            .all(|(a, b)| root(&parents, *a) != root(&parents, *b)),
        "Borders inside a merged cell are ambiguous"
    );
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..parents.len() {
        groups.entry(root(&parents, i)).or_default().push(i);
    }
    for cells in groups.values().filter(|cells| cells.len() > 1) {
        let top = cells.iter().map(|i| i / columns).min().unwrap();
        let bottom = cells.iter().map(|i| i / columns).max().unwrap();
        let left = cells.iter().map(|i| i % columns).min().unwrap();
        let right = cells.iter().map(|i| i % columns).max().unwrap();
        ensure!(
            cells.len() == (bottom - top + 1) * (right - left + 1),
            "A merged cell must be rectangular"
        );
        table.spans.push(CellSpan {
            anchor: top * columns + left,
            rows: bottom - top + 1,
            columns: right - left + 1,
        });
    }
    table.spans.sort_by_key(|s| s.anchor);
    table.validate()
}

pub fn detect(bytes: &[u8], page: u32) -> Result<Vec<Table>> {
    let mut doc = lopdf::Document::load_mem(bytes)?;
    let clean = crate::annotations::clean_render_copy(bytes, &mut doc)?;
    let bytes = clean.as_slice();
    let own = owned(bytes, page)?;
    let runs = crate::text_geometry::runs(bytes, page)?;
    let painted = style_marks(bytes, page)?;
    let paints = grid_paints(bytes, page)?;
    let segments = crate::form_detect::border_segments(bytes, page.saturating_sub(1) as usize)?;
    // Merge touching collinear segments, including borders drawn cell by cell.
    let merge = |vertical: bool| {
        let mut edges: Vec<[f32; 3]> = segments
            .iter()
            .filter_map(|s| {
                if vertical && (s[0] - s[2]).abs() < 1. {
                    Some([s[0], s[1].min(s[3]), s[1].max(s[3])])
                } else if !vertical && (s[1] - s[3]).abs() < 1. {
                    Some([s[1], s[0].min(s[2]), s[0].max(s[2])])
                } else {
                    None
                }
            })
            .collect();
        edges.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        let mut merged: Vec<[f32; 3]> = vec![];
        for e in edges {
            if let Some(m) = merged.last_mut()
                && (m[0] - e[0]).abs() < 1.5
                && e[1] <= m[2] + 1.5
            {
                m[2] = m[2].max(e[2]);
            } else {
                merged.push(e);
            }
        }
        merged
    };
    let hs = merge(false);
    let vs = merge(true);
    let mut groups: Vec<Vec<[f32; 3]>> = vec![];
    for h in hs.iter().copied() {
        if let Some(g) = groups
            .iter_mut()
            .find(|g| (g[0][1] - h[1]).abs() < 1.5 && (g[0][2] - h[2]).abs() < 1.5)
        {
            g.push(h);
        } else {
            groups.push(vec![h]);
        }
    }
    let mut candidates = vec![];
    for mut group in groups {
        group.sort_by(|a, b| a[0].total_cmp(&b[0]));
        group.truncate(61);
        for span in (1..group.len()).rev() {
            for start in 0..group.len() - span {
                let end = start + span;
                let (l, r, t, b) = (
                    group[start][1],
                    group[start][2],
                    group[start][0],
                    group[end][0],
                );
                let mut xs: Vec<_> = vs
                    .iter()
                    .filter(|v| {
                        v[0] >= l - 1.5 && v[0] <= r + 1.5 && v[1] < b - 1.5 && v[2] > t + 1.5
                    })
                    .map(|v| v[0])
                    .collect();
                xs.sort_by(f32::total_cmp);
                xs.dedup_by(|a, b| (*a - *b).abs() < 1.5);
                if xs.len() < 2
                    || xs.len() > 13
                    || (xs[0] - l).abs() > 1.5
                    || (*xs.last().unwrap() - r).abs() > 1.5
                {
                    continue;
                }
                let mut ys: Vec<_> = hs
                    .iter()
                    .filter(|h| {
                        h[0] >= t - 1.5 && h[0] <= b + 1.5 && h[1] < r - 1.5 && h[2] > l + 1.5
                    })
                    .map(|h| h[0])
                    .collect();
                ys.sort_by(f32::total_cmp);
                ys.dedup_by(|a, b| (*a - *b).abs() < 1.5);
                if ys.len() < 2 || ys.len() > 61 || (ys.len() - 1) * (xs.len() - 1) < 2 {
                    continue;
                }
                let mut table = Table::new(ys.len() - 1, xs.len() - 1, [l, t, r, b]);
                table.id.clear();
                table.xs = xs;
                table.ys = ys;
                if table.validate().is_err()
                    || infer_spans(&mut table, &hs, &vs).is_err()
                    || own.iter().any(|o| {
                        let a = o.rect();
                        contains(a, [l, t]) && contains(a, [r, b])
                    })
                    || candidates.iter().any(|o: &Table| {
                        let a = o.rect();
                        contains(a, [l, t]) && contains(a, [r, b])
                    })
                {
                    continue;
                }
                for i in table.anchors().collect::<Vec<_>>() {
                    table.cells[i] = text_from_runs(&runs, table.cell(i))
                        .map(|v| v.0)
                        .unwrap_or_default();
                }
                {
                    table.styles = (0..table.cells.len())
                        .map(|i| {
                            let rect = table.cell(i);
                            painted
                                .iter()
                                .rev()
                                .find(|(r, _)| r.iter().zip(rect).all(|(a, b)| (*a - b).abs() < 1.))
                                .map(|(_, s)| s.clone())
                                .unwrap_or_else(|| detected_style(rect, &runs, &paints))
                        })
                        .collect();
                }
                candidates.push(table);
                if candidates.len() >= 30 {
                    break;
                }
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.ys[0]
            .total_cmp(&b.ys[0])
            .then(a.xs[0].total_cmp(&b.xs[0]))
    });
    Ok(candidates)
}

/// Remove the original embedded grid and text, then create its separate vector layer.
/// Shared drawing operations are refused so neighboring page artwork cannot move.
pub fn detach(bytes: &[u8], page: u32, table: &Table) -> Result<Vec<u8>> {
    table.validate()?;
    let mark = layer(table, page.saturating_sub(1) as usize, None)?;
    let out = remove_embedded(bytes, page, table)?;
    let mut annotations = crate::annotations::load(&lopdf::Document::load_mem(&out)?);
    annotations.push(mark);
    crate::annotations::export(&out, &annotations)
}

/// Delete an embedded table's text, borders and fills without creating a layer.
/// Unrelated page content and annotations remain unchanged. Shared drawing
/// operations are refused before the caller receives any replacement bytes.
pub fn delete(bytes: &[u8], page: u32, table: &Table) -> Result<Vec<u8>> {
    table.validate()?;
    remove_embedded(bytes, page, table)
}

fn remove_embedded(bytes: &[u8], page: u32, table: &Table) -> Result<Vec<u8>> {
    let mut source = bytes.to_vec();
    let own = owned(bytes, page)?.iter().any(|t| t.id == table.id);
    if !own {
        let mut selection = crate::text_block::Selection {
            runs: vec![],
            annotations: vec![],
        };
        for i in table.anchors() {
            selection
                .runs
                .extend(cell_text(bytes, page, table.cell(i))?.1.runs);
        }
        selection.runs.sort_by_key(|r| r.0);
        selection.runs.dedup_by_key(|r| r.0);
        if !selection.runs.is_empty() {
            source = crate::text_block::replace(
                &source,
                page,
                &crate::text_block::Replacement {
                    selection,
                    spans: vec![],
                    layout: vec![],
                    text: String::new(),
                    rect: [0., 0., 1., 1.],
                    size: 11.,
                    color: [0.; 3],
                    format: Default::default(),
                },
            )?;
        }
    }
    let mut doc = crate::content::editable(&source)?;
    let id = *doc.get_pages().get(&page).context("Page not found")?;
    let content = doc.get_and_decode_page_content(id)?;
    let mut kept = vec![];
    let mut skip = 0usize;
    if own {
        for op in content.operations {
            if skip > 0 {
                if op.operator == "BDC" || op.operator == "BMC" {
                    skip += 1;
                } else if op.operator == "EMC" {
                    skip -= 1;
                }
                continue;
            }
            if marked(&op).is_some_and(|t| t.id == table.id) {
                skip = 1;
                continue;
            }
            kept.push(op);
        }
    } else {
        use hayro::kurbo::{Affine, Point};
        let geometry = crate::annotations::page_geometry(&source)?;
        let display = Affine::new(geometry[page as usize - 1].1).inverse();
        let mut transform = Affine::IDENTITY;
        let mut stack = vec![];
        let mut path_indices = vec![];
        let mut points: Vec<Point> = vec![];
        let mut complex = false;
        let mut removed = std::collections::HashSet::new();
        let mut paths = 0;
        let bounds = table.rect();
        let inside = |p: Point| {
            p.x >= bounds[0] as f64 - 1.5
                && p.x <= bounds[2] as f64 + 1.5
                && p.y >= bounds[1] as f64 - 1.5
                && p.y <= bounds[3] as f64 + 1.5
        };
        for (index, op) in content.operations.iter().enumerate() {
            let values: Vec<_> = op
                .operands
                .iter()
                .filter_map(|v| v.as_float().ok())
                .collect();
            match op.operator.as_str() {
                "q" => stack.push(transform),
                "Q" => transform = stack.pop().context("Unbalanced graphics state")?,
                "cm" => {
                    ensure!(values.len() == 6, "Invalid transformation");
                    transform *= Affine::new(
                        values
                            .iter()
                            .map(|v| *v as f64)
                            .collect::<Vec<_>>()
                            .try_into()
                            .unwrap(),
                    );
                }
                "m" | "l" => {
                    ensure!(values.len() == 2, "Invalid border path");
                    points
                        .push(display * transform * Point::new(values[0] as f64, values[1] as f64));
                    path_indices.push(index);
                }
                "re" => {
                    ensure!(values.len() == 4, "Invalid border rectangle");
                    let [x, y, w, h] = [values[0], values[1], values[2], values[3]];
                    for p in [[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]] {
                        points.push(display * transform * Point::new(p[0] as f64, p[1] as f64));
                    }
                    path_indices.push(index);
                }
                "h" => path_indices.push(index),
                "c" | "v" | "y" => {
                    complex = true;
                    path_indices.push(index);
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                    let on_grid = |p: &Point| {
                        table.xs.iter().any(|x| (p.x - *x as f64).abs() < 1.5)
                            || table.ys.iter().any(|y| (p.y - *y as f64).abs() < 1.5)
                    };
                    let touches = points.iter().any(|p| inside(*p) && on_grid(p));
                    if touches && op.operator != "n" {
                        ensure!(
                            !complex && points.iter().all(|p| inside(*p) && on_grid(p)),
                            "These table borders share a drawing operation with other artwork and cannot be removed independently."
                        );
                        removed.extend(path_indices.iter().copied());
                        removed.insert(index);
                        paths += 1;
                    }
                    points.clear();
                    path_indices.clear();
                    complex = false;
                }
                "W" | "W*" => complex = true,
                _ => {}
            }
        }
        ensure!(
            paths > 0,
            "The table borders are inside grouped artwork and cannot be removed independently"
        );
        kept = content
            .operations
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !removed.contains(i))
            .map(|(_, o)| o)
            .collect();
    }
    ensure!(skip == 0, "Unbalanced table content");
    let stream = doc.add_object(Stream::new(
        Dictionary::new(),
        Content { operations: kept }.encode()?,
    ));
    doc.get_dictionary_mut(id)?.set("Contents", stream);
    crate::content::serialize(&mut doc)
}
