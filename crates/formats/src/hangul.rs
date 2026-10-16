//! What the two Hangul (Hancom Office) readers share — [`crate::hwp`] (HWP 5.0 binary) and
//! [`crate::hwpx`] (HWPX / OWPML, KS X 6101): both formats keep the same concepts (character
//! shapes, paragraph shapes and border fills referenced by id from runs, paragraphs and cells;
//! tables as cells with an address and spans), so each reader decodes its own syntax into the
//! small "raw" shapes below and this module turns them into the document model.

use std::sync::Arc;

use wordcraft_doc::para::{InlineObject, OBJ, Run};
use wordcraft_doc::props::{
    Border, BorderStyle, Borders, CellProps, CharProps, HeightRule, LineSpacing, ParaProps, RowProps, TabAlign, TabLeader, TabStop, TableProps,
    TextColor, Underline, VMerge, VertAlign,
};
use wordcraft_doc::table::{Cell, MAX_COLS, MAX_ROWS, Row};
use wordcraft_doc::{Align, Block, Blocks, Paragraph, Rgb, SectionProps, Table, para_block};

/// Nesting limit for tables in cells, notes, headers and text boxes (hostile input).
pub(crate) const MAX_DEPTH: usize = 16;
/// Most header, footer and note parts one document creates.
pub(crate) const MAX_PARTS: usize = 20_000;
/// Most picture bytes one document loads.
pub(crate) const MAX_MEDIA: u64 = 1 << 30;

/// HWPUNIT (1/7200 inch) to points, clamped to a sane range.
pub(crate) fn pt(v: f64) -> f32 {
    ((v / 100.0) as f32).clamp(-20_000.0, 20_000.0)
}

/// Hangul's border widths (HWP 5.0 line-width codes), millimetres.
const WIDTHS_MM: [f32; 16] = [0.1, 0.12, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.7, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0];

/// A border width code to points.
pub(crate) fn width_code_pt(code: u8) -> f32 {
    WIDTHS_MM.get(usize::from(code)).copied().unwrap_or(0.12) * 72.0 / 25.4
}

/// An OWPML width (`"0.12 mm"`) to points.
pub(crate) fn width_mm_pt(s: &str) -> f32 {
    let n: f32 = s.trim().trim_end_matches("mm").trim().parse().unwrap_or(0.12);
    (if n.is_finite() { n.clamp(0.0, 10.0) } else { 0.12 }) * 72.0 / 25.4
}

/// A border line type (HWP 5.0 code: 0 none, 1 solid, 2 dash, 3 dot, 4 dash-dot, 5 dash-dot-dot,
/// 6 long dash, 7 circle, 8 double, 9/10 thin-thick doubles, 11 triple, 12/13 waves, 14+ 3D).
pub(crate) fn line_style(code: u8) -> BorderStyle {
    match code {
        0 => BorderStyle::None,
        2 | 6 => BorderStyle::Dashed,
        3 | 7 => BorderStyle::Dotted,
        4 | 5 => BorderStyle::DotDash,
        8..=10 => BorderStyle::Double,
        11 => BorderStyle::Triple,
        12 | 13 => BorderStyle::Wave,
        _ => BorderStyle::Single,
    }
}

/// The HWP 5.0 line type code of an OWPML line type name.
pub(crate) fn line_code(name: &str) -> u8 {
    match name {
        "NONE" => 0,
        "SOLID" => 1,
        "DASH" => 2,
        "DOT" => 3,
        "DASH_DOT" => 4,
        "DASH_DOT_DOT" => 5,
        "LONG_DASH" => 6,
        "CIRCLE" => 7,
        "DOUBLE_SLIM" => 8,
        "SLIM_THICK" => 9,
        "THICK_SLIM" => 10,
        "SLIM_THICK_SLIM" => 11,
        "WAVE" => 12,
        "DOUBLE_WAVE" => 13,
        _ => 1,
    }
}

/// A HWP colour (`0x00BBGGRR`); `None` for "no colour" (`0xFFFFFFFF`).
pub(crate) fn colorref(v: u32) -> Option<Rgb> {
    if v >> 24 != 0 {
        return None;
    }
    let [r, g, b, _] = v.to_le_bytes();
    Some(Rgb(r, g, b))
}

/// An OWPML colour (`#RRGGBB`, `#AARRGGBB` or `none`).
pub(crate) fn hex_color(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    let h = if h.len() == 8 { h.get(2..)? } else { h };
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let [b, g, r, _] = v.to_le_bytes();
    Some(Rgb(r, g, b))
}

/// A border fill: borders on the four sides and a solid background.
#[derive(Clone, Debug, Default)]
pub(crate) struct BorderFill {
    pub borders: Borders,
    pub fill: Option<Rgb>,
}

impl BorderFill {
    /// A side from its line type code, width (points) and colour.
    pub fn side(code: u8, width: f32, color: Option<Rgb>) -> Border {
        Border { style: line_style(code), width: if code == 0 { 0.0 } else { width.max(0.25) }, color, space: 0.0 }
    }
    /// Any visible border.
    fn any_border(&self) -> bool {
        [self.borders.top, self.borders.left, self.borders.bottom, self.borders.right].iter().flatten().any(Border::is_visible)
    }
}

/// A character shape, as both formats give it.
#[derive(Clone, Debug, Default)]
pub(crate) struct RawChar {
    /// Hangul (East Asian) and Latin faces.
    pub hangul: Option<String>,
    pub latin: Option<String>,
    /// Size, HWPUNIT (1000 = 10 pt).
    pub height: i32,
    /// Width ratio (percent) and letter spacing (percent of the size) of Hangul text.
    pub ratio: i32,
    pub spacing: i32,
    pub bold: bool,
    pub italic: bool,
    /// Underline line shape (HWP 5.0 underline shape code: 0 solid, 1 dash, 2 dot…).
    pub underline: Option<u8>,
    pub strike: bool,
    pub sup: bool,
    pub sub: bool,
    pub color: Option<Rgb>,
    pub shade: Option<Rgb>,
    pub outline: bool,
    pub shadow: bool,
    pub emboss: bool,
    pub engrave: bool,
}

impl RawChar {
    pub fn props(&self) -> CharProps {
        let size = if self.height > 0 { (self.height as f32 / 100.0).clamp(1.0, 1638.0) } else { 10.0 };
        let underline = self.underline.map(|s| match s {
            1 | 5 => Underline::Dash,
            2 | 6 => Underline::Dotted,
            3 => Underline::DotDash,
            4 => Underline::DotDotDash,
            7..=10 => Underline::Double,
            11 => Underline::Wave,
            12 => Underline::DoubleWave,
            _ => Underline::Single,
        });
        let on = |b: bool| b.then_some(true);
        CharProps {
            font: self.latin.clone().filter(|s| !s.is_empty()),
            font_ea: self.hangul.clone().filter(|s| !s.is_empty()),
            size: Some(size),
            bold: Some(self.bold),
            italic: Some(self.italic),
            underline,
            strike: on(self.strike),
            vert_align: if self.sup {
                Some(VertAlign::Superscript)
            } else if self.sub {
                Some(VertAlign::Subscript)
            } else {
                None
            },
            color: self.color.map(TextColor::Rgb),
            shading: self.shade.filter(|c| *c != Rgb(255, 255, 255)),
            spacing: (self.spacing != 0).then(|| size * self.spacing.clamp(-50, 50) as f32 / 100.0),
            scale: (self.ratio > 0 && self.ratio != 100).then(|| self.ratio.clamp(50, 200) as f32),
            outline: on(self.outline),
            shadow: on(self.shadow),
            emboss: on(self.emboss),
            engrave: on(self.engrave),
            ..Default::default()
        }
    }
}

/// How a paragraph spaces its lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RawLine {
    /// Percent of the font size (Hangul's default, 160 %).
    Percent(f32),
    /// Exactly, at least, or this much space between lines, points.
    Fixed(f32),
    AtLeast(f32),
    Between(f32),
}

/// A paragraph shape, as both formats give it; lengths in points.
#[derive(Clone, Debug)]
pub(crate) struct RawPara {
    /// 0 justify, 1 left, 2 right, 3 center, 4 distribute, 5 split.
    pub align: u8,
    pub left: f32,
    pub right: f32,
    /// First-line indent; negative = hanging (Hangul keeps the first line at the left margin).
    pub indent: f32,
    pub before: f32,
    pub after: f32,
    pub line: RawLine,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub break_before: bool,
    pub widow: bool,
    pub border: Option<BorderFill>,
    /// Tab stops, positions from the paragraph's left margin.
    pub tabs: Vec<TabStop>,
}

/// A tab stop at `pos` points of kind `kind` (0 left, 1 right, 2 center, 3 decimal) with the
/// leader of line type `leader` (HWP 5.0 line code; 0 none).
pub(crate) fn tab(pos: f32, kind: u8, leader: u8) -> TabStop {
    TabStop {
        pos: pos.clamp(0.0, 2000.0),
        align: match kind {
            1 => TabAlign::Right,
            2 => TabAlign::Center,
            3 => TabAlign::Decimal,
            _ => TabAlign::Left,
        },
        leader: match leader {
            0 => TabLeader::None,
            1 | 8..=11 => TabLeader::Underscore,
            _ => TabLeader::Dot,
        },
    }
}

impl Default for RawPara {
    fn default() -> Self {
        RawPara {
            align: 0,
            left: 0.0,
            right: 0.0,
            indent: 0.0,
            before: 0.0,
            after: 0.0,
            line: RawLine::Percent(160.0),
            keep_next: false,
            keep_lines: false,
            break_before: false,
            widow: false,
            border: None,
            tabs: Vec::new(),
        }
    }
}

/// A paragraph's formatting and its line spacing as a percentage of its font size, which
/// [`ParaBuf::finish`] turns into points once the paragraph's sizes are known.
#[derive(Clone, Debug, Default)]
pub(crate) struct ParaShape {
    pub props: ParaProps,
    pub pct: Option<f32>,
}

impl RawPara {
    pub fn shape(&self) -> ParaShape {
        let len = |v: f32| v.clamp(-2000.0, 2000.0);
        // Hangul hangs the lines after the first; Word's first line hangs out to the left.
        let (left, first) = if self.indent < 0.0 { (self.left - self.indent, self.indent) } else { (self.left, self.indent) };
        let (line, pct) = match self.line {
            RawLine::Percent(p) => (None, Some(p.clamp(10.0, 1000.0))),
            RawLine::Fixed(v) => (Some(LineSpacing::Exactly(v.clamp(1.0, 1600.0))), None),
            RawLine::AtLeast(v) => (Some(LineSpacing::AtLeast(v.clamp(1.0, 1600.0))), None),
            RawLine::Between(_) => (Some(LineSpacing::Multiple(1.0)), None),
        };
        let mut props = ParaProps {
            align: Some(match self.align {
                1 => Align::Left,
                2 => Align::Right,
                3 => Align::Center,
                4 | 5 => Align::Distribute,
                _ => Align::Justify,
            }),
            indent_left: Some(len(left)),
            indent_right: Some(len(self.right)),
            indent_first: Some(len(first)),
            space_before: Some(self.before.clamp(0.0, 1584.0)),
            space_after: Some(self.after.clamp(0.0, 1584.0)),
            line_spacing: line,
            keep_next: Some(self.keep_next),
            keep_lines: Some(self.keep_lines),
            page_break_before: self.break_before.then_some(true),
            widow_control: Some(self.widow),
            tabs: (!self.tabs.is_empty()).then(|| self.tabs.iter().map(|t| TabStop { pos: (self.left + t.pos).clamp(0.0, 2000.0), ..*t }).collect()),
            ..Default::default()
        };
        props.cancel_char_indents(true, true, true);
        if let Some(b) = &self.border {
            if b.any_border() {
                props.borders = Some(Borders { between: None, inside_v: None, ..b.borders });
            }
            props.shading = b.fill;
        }
        ParaShape { props, pct }
    }
}

/// One paragraph being built from text and objects.
#[derive(Default)]
pub(crate) struct ParaBuf {
    text: String,
    runs: Vec<Run>,
    objects: Vec<InlineObject>,
    shape: ParaShape,
    mark: CharProps,
    /// Largest font size in the paragraph so far.
    max_size: f32,
}

impl ParaBuf {
    pub fn new(shape: ParaShape) -> ParaBuf {
        ParaBuf { shape, ..Default::default() }
    }
    pub fn set_mark(&mut self, mark: &CharProps) {
        self.mark = mark.clone();
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn push(&mut self, s: &str, props: &CharProps) {
        if s.is_empty() {
            return;
        }
        self.max_size = self.max_size.max(props.size.unwrap_or(10.0));
        match self.runs.last_mut() {
            Some(r) if r.props == *props => r.len += s.len(),
            _ => self.runs.push(Run { len: s.len(), props: props.clone() }),
        }
        self.text.push_str(s);
    }
    pub fn push_char(&mut self, c: char, props: &CharProps) {
        let mut b = [0u8; 4];
        self.push(c.encode_utf8(&mut b), props);
    }
    pub fn object(&mut self, obj: InlineObject, props: &CharProps) {
        self.push_char(OBJ, props);
        self.objects.push(obj);
    }
    /// The paragraph so far; the builder keeps its formatting (but not a page break before) for
    /// the text that follows, after a table has split the paragraph.
    pub fn finish(&mut self) -> Paragraph {
        let mut p = Paragraph::new();
        p.text = std::mem::take(&mut self.text);
        p.runs = std::mem::take(&mut self.runs);
        p.objects = std::mem::take(&mut self.objects);
        p.props = self.shape.props.clone();
        if let Some(pct) = self.shape.pct {
            let size = if self.max_size > 0.0 { self.max_size } else { self.mark.size.unwrap_or(10.0) };
            p.props.line_spacing = Some(LineSpacing::AtLeast((size * pct / 100.0).clamp(1.0, 1600.0)));
        }
        p.mark = self.mark.clone();
        self.shape.props.page_break_before = None;
        self.max_size = 0.0;
        p
    }
}

/// The blocks of one story (body, cell, header, note) as they are read. A paragraph that holds
/// only a table leaves an empty paragraph after it, kept only where a paragraph must follow:
/// before another table (which would otherwise merge with it) and at the story's end.
#[derive(Default)]
pub(crate) struct Story {
    items: Vec<(Arc<Block>, bool)>,
}

impl Story {
    pub fn para(&mut self, p: Paragraph) {
        self.items.push((para_block(p), false));
    }
    /// An empty paragraph left by a paragraph that held only tables.
    pub fn tail(&mut self, p: Paragraph) {
        self.items.push((para_block(p), true));
    }
    pub fn table(&mut self, t: Table) {
        self.items.push((Arc::new(Block::Table(t)), false));
    }
    pub fn extend(&mut self, blocks: Blocks) {
        self.items.extend(blocks.into_iter().map(|b| (b, false)));
    }
    pub fn finish(self) -> Blocks {
        let mut out: Blocks = Vec::with_capacity(self.items.len());
        let mut it = self.items.into_iter().peekable();
        while let Some((b, tail)) = it.next() {
            let keep = !tail || it.peek().is_none_or(|(n, _)| matches!(**n, Block::Table(_)));
            if keep {
                out.push(b);
            }
        }
        if out.last().is_none_or(|b| matches!(**b, Block::Table(_))) {
            out.push(para_block(Paragraph::new()));
        }
        out
    }
}

/// A table cell as both formats give it: its address, spans and size (points), its formatting
/// and its content.
pub(crate) struct RawCell {
    pub row: usize,
    pub col: usize,
    pub rows: usize,
    pub cols: usize,
    pub w: f32,
    pub h: f32,
    pub props: CellProps,
    pub blocks: Blocks,
}

/// Lay out cells given by address on a grid: column widths from the cells' widths, horizontal
/// spans as cell spans, vertical spans as merged cells, holes as empty cells. `None` without
/// cells.
pub(crate) fn table(mut cells: Vec<RawCell>, props: TableProps) -> Option<Table> {
    // Clip cells to the grid's limits; the grid is as big as its cells say.
    cells.retain(|c| c.row < MAX_ROWS && c.col < MAX_COLS);
    for c in &mut cells {
        c.rows = c.rows.clamp(1, MAX_ROWS - c.row);
        c.cols = c.cols.clamp(1, MAX_COLS - c.col);
    }
    let rows = cells.iter().map(|c| c.row + c.rows).max()?;
    let cols = cells.iter().map(|c| c.col + c.cols).max()?;
    // Which cell covers each grid slot (the first one read wins an overlap).
    let mut owner: Vec<Option<usize>> = vec![None; rows.checked_mul(cols)?];
    for (i, c) in cells.iter().enumerate() {
        for r in c.row..c.row + c.rows {
            for g in c.col..c.col + c.cols {
                if let Some(slot) = owner.get_mut(r * cols + g)
                    && slot.is_none()
                {
                    *slot = Some(i);
                }
            }
        }
    }
    let grid = grid_widths(&cells, cols);
    let mut out = Table { props, grid, rows: Vec::with_capacity(rows), ..Default::default() };
    out.props.width = Some(out.grid.iter().sum());
    for r in 0..rows {
        let mut row = Row::default();
        let mut height = 0.0f32;
        let mut g = 0;
        while g < cols {
            let width = |span: usize| out.grid.iter().skip(g).take(span).sum::<f32>();
            match owner.get(r * cols + g).copied().flatten().and_then(|i| cells.get_mut(i).map(|c| (i, c))) {
                Some((i, c)) if c.col == g && owner.get(c.row * cols + c.col).copied().flatten() == Some(i) => {
                    let mut props = c.props.clone();
                    props.span = c.cols as u32;
                    props.width = Some(width(c.cols));
                    if c.row == r {
                        props.vmerge = if c.rows > 1 { VMerge::Restart } else { VMerge::None };
                        if c.rows == 1 {
                            height = height.max(c.h);
                        }
                        let blocks = std::mem::take(&mut c.blocks);
                        row.cells.push(Cell {
                            props,
                            blocks: if blocks.is_empty() { vec![para_block(Paragraph::new())] } else { blocks },
                            ..Default::default()
                        });
                    } else {
                        props.vmerge = VMerge::Continue;
                        row.cells.push(Cell { props, blocks: vec![para_block(Paragraph::new())], ..Default::default() });
                    }
                    g += c.cols;
                }
                _ => {
                    let mut cell = Cell::empty();
                    cell.props.width = Some(width(1));
                    row.cells.push(cell);
                    g += 1;
                }
            }
        }
        if height > 0.0 {
            row.props = RowProps { height: Some(height.min(1584.0)), height_rule: HeightRule::AtLeast, ..Default::default() };
        }
        out.rows.push(row);
    }
    Some(out)
}

/// Column widths (points) from the cells' widths: the column edges follow from each cell's left
/// edge and width, narrow cells first; edges no cell fixes are spread evenly.
fn grid_widths(cells: &[RawCell], cols: usize) -> Vec<f32> {
    let mut x: Vec<Option<f32>> = vec![None; cols + 1];
    if let Some(first) = x.first_mut() {
        *first = Some(0.0);
    }
    let mut order: Vec<&RawCell> = cells.iter().filter(|c| c.w.is_finite() && c.w > 0.0).collect();
    order.sort_by_key(|c| c.cols);
    // Each pass fixes at least one edge or stops, so `cols + 1` passes are enough.
    for _ in 0..=cols {
        let mut changed = false;
        for c in &order {
            let (a, b) = (c.col, c.col + c.cols);
            match (x.get(a).copied().flatten(), x.get(b).copied().flatten()) {
                (Some(xa), None) => {
                    if let Some(s) = x.get_mut(b) {
                        *s = Some(xa + c.w);
                        changed = true;
                    }
                }
                (None, Some(xb)) => {
                    if let Some(s) = x.get_mut(a) {
                        *s = Some(xb - c.w);
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    // Fill the edges still unknown between known neighbours (or at the average width).
    let known: Vec<(usize, f32)> = x.iter().enumerate().filter_map(|(i, v)| v.map(|v| (i, v))).collect();
    let avg = match (known.first(), known.last()) {
        (Some(&(i0, v0)), Some(&(i1, v1))) if i1 > i0 && v1 > v0 => (v1 - v0) / (i1 - i0) as f32,
        _ => 72.0,
    };
    let mut prev: Option<(usize, f32)> = None;
    for i in 0..=cols {
        if let Some(v) = x.get(i).copied().flatten() {
            prev = Some((i, v));
            continue;
        }
        let next = known.iter().find(|(k, _)| *k > i).copied();
        let v = match (prev, next) {
            (Some((pi, pv)), Some((ni, nv))) => pv + (nv - pv) * (i - pi) as f32 / (ni - pi) as f32,
            (Some((pi, pv)), None) => pv + avg * (i - pi) as f32,
            (None, Some((ni, nv))) => nv - avg * (ni - i) as f32,
            (None, None) => avg * i as f32,
        };
        if let Some(s) = x.get_mut(i) {
            *s = Some(v);
        }
        prev = Some((i, v));
    }
    x.windows(2)
        .map(|w| match w {
            [Some(a), Some(b)] => (b - a).clamp(2.0, 2000.0),
            _ => 72.0,
        })
        .collect()
}

/// A cell's formatting from its border fill, vertical alignment and margins (points; `None` =
/// the table's).
pub(crate) fn cell_props(fill: Option<&BorderFill>, valign: wordcraft_doc::props::VAlign, margins: Option<[f32; 4]>) -> CellProps {
    CellProps {
        span: 1,
        borders: fill.map(|f| Borders { between: None, inside_v: None, ..f.borders }),
        shading: fill.and_then(|f| f.fill),
        valign: Some(valign),
        margins,
        ..Default::default()
    }
}

/// Hangul's default page: A4 portrait with its usual margins.
pub(crate) fn default_page() -> SectionProps {
    page(59528.0, 84188.0, false, [8504.0, 8504.0, 5668.0, 4252.0, 4252.0, 4252.0, 0.0])
}

/// A page from Hangul's page definition (HWPUNIT): width, height, landscape and the margins
/// left, right, top, bottom, header, footer, gutter. Hangul measures the header from the top
/// margin and the body from below the header, so the body starts at top + header.
pub(crate) fn page(w: f64, h: f64, landscape: bool, m: [f64; 7]) -> SectionProps {
    let [left, right, top, bottom, header, footer, gutter] = m.map(|v| pt(v).clamp(0.0, 1584.0));
    let (mut pw, mut ph) = (pt(w), pt(h));
    if !(pw >= 72.0 && ph >= 72.0) {
        (pw, ph) = (595.3, 841.9);
    }
    let (pw, ph) = if landscape { (pw.max(ph), pw.min(ph)) } else { (pw, ph) };
    let mut s = SectionProps {
        page_w: pw,
        page_h: ph,
        landscape,
        margin_left: left,
        margin_right: right,
        margin_top: (top + header).min(ph / 2.0),
        margin_bottom: (bottom + footer).min(ph / 2.0),
        header: top,
        footer: bottom,
        gutter,
        ..Default::default()
    };
    if s.margin_left + s.margin_right > pw - 36.0 {
        s.margin_left = 72.0;
        s.margin_right = 72.0;
    }
    s
}

/// A page number Hangul puts on every page without a header or footer part (`pgnp`,
/// `hp:pageNum`): its place (1 top left … 6 bottom right; 0 none) and the dash around it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PageNum {
    pub pos: u8,
    pub side: Option<char>,
}

impl PageNum {
    /// The header (`false`) or footer (`true`) paragraph that shows the number.
    pub fn part(&self) -> Option<(bool, Paragraph)> {
        let (footer, align) = match self.pos {
            1 | 7 | 9 => (false, Align::Left),
            2 => (false, Align::Center),
            3 => (false, Align::Right),
            4 | 8 | 10 => (true, Align::Left),
            5 => (true, Align::Center),
            6 => (true, Align::Right),
            _ => return None,
        };
        let mut b = ParaBuf::new(ParaShape { props: ParaProps { align: Some(align), ..Default::default() }, pct: None });
        let props = CharProps::default();
        let side = self.side.filter(|c| !c.is_whitespace() && !c.is_control());
        if let Some(c) = side {
            b.push(&format!("{c} "), &props);
        }
        b.object(page_field(), &props);
        if let Some(c) = side {
            b.push(&format!(" {c}"), &props);
        }
        Some((footer, b.finish()))
    }
}

/// A `PAGE` field.
pub(crate) fn page_field() -> InlineObject {
    InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: true, code: false, form: String::new() }
}

/// Add a header or footer part to `sec` unless the document already has too many parts.
pub(crate) fn add_header(doc: &mut wordcraft_doc::Document, sec: &mut SectionProps, footer: bool, even: bool, blocks: Blocks) {
    if doc.parts.len() >= MAX_PARTS {
        return;
    }
    let kind = if footer { wordcraft_doc::PartKind::Footer } else { wordcraft_doc::PartKind::Header };
    let id = doc.add_part(kind, blocks);
    let set = if footer { &mut sec.footers } else { &mut sec.headers };
    if even {
        set.even = Some(id);
    } else {
        set.default = Some(id);
    }
}

/// Finish a section: a page number without a header or footer becomes one.
pub(crate) fn finish_section(doc: &mut wordcraft_doc::Document, sec: &mut SectionProps, num: Option<PageNum>) {
    if let Some((footer, p)) = num.and_then(|n| n.part()) {
        let set = if footer { &sec.footers } else { &sec.headers };
        if set.default.is_none() {
            add_header(doc, sec, footer, false, vec![para_block(p)]);
        }
    }
}

/// Attach each section's page setup to the body: every section but the last ends with its last
/// paragraph; the last becomes the document's last section.
pub(crate) fn assemble(doc: &mut wordcraft_doc::Document, sections: Vec<(Blocks, SectionProps)>) {
    let n = sections.len();
    let mut body: Blocks = Vec::new();
    for (i, (mut blocks, props)) in sections.into_iter().enumerate() {
        if i + 1 == n {
            doc.last_section = props;
        } else if let Some(Block::Para(p)) = blocks.last_mut().map(Arc::make_mut) {
            p.section = Some(Box::new(props));
        }
        body.append(&mut blocks);
    }
    doc.body = body;
}
