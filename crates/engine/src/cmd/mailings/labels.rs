use super::merge::address_block;
use super::{
    Align, Block, CharProps, CmdError, CmdResult, Document, Paragraph, Pos, SectionProps, Selection, Session, StoryRef, Table, Value, json, p,
    para_block,
};

/// Common envelope sizes: name, width and height in points (long side first).
pub const ENVELOPE_SIZES: [(&str, f32, f32); 8] = [
    ("Envelope #10", 684.0, 297.0),
    ("Envelope #9", 639.0, 279.0),
    ("Envelope Monarch", 540.0, 279.0),
    ("Envelope DL", 623.6, 311.8),
    ("Envelope C4", 918.4, 649.1),
    ("Envelope C5", 649.1, 459.2),
    ("Envelope C6", 459.2, 323.1),
    ("Envelope B5", 708.7, 498.9),
];

/// Envelope size by name (`Envelope DL`, or just `DL`, any case), or `width`/`height` in points.
fn envelope_size(v: &Value) -> Result<(f32, f32), CmdError> {
    if let (Some(w), Some(h)) = (p::f32(v, "width"), p::f32(v, "height")) {
        let (w, h) = (w.clamp(144.0, 1584.0), h.clamp(144.0, 1584.0));
        return Ok((w.max(h), w.min(h)));
    }
    let name = p::str(v, "size").unwrap_or("Envelope #10").trim();
    ENVELOPE_SIZES
        .iter()
        .find(|(n, ..)| n.eq_ignore_ascii_case(name) || n.strip_prefix("Envelope ").is_some_and(|short| short.eq_ignore_ascii_case(name)))
        .map(|(_, w, h)| (*w, *h))
        .ok_or_else(|| CmdError::Params(format!("unknown envelope size `{name}`")))
}

/// Longest label or envelope address text accepted.
const MAX_LABEL_TEXT: usize = 2_000;

/// An optional label/envelope text parameter, rejected when longer than `MAX_LABEL_TEXT` characters.
fn label_text(v: &Value, key: &str) -> Result<Option<String>, CmdError> {
    match p::str(v, key) {
        Some(t) if t.chars().count() > MAX_LABEL_TEXT => Err(CmdError::Params(format!("`{key}` is longer than {MAX_LABEL_TEXT} characters"))),
        t => Ok(t.map(str::to_string)),
    }
}

/// Envelopes: a landscape page with the return address at the top left and the delivery address
/// in the middle. As a new document, or (`add`) as a section of its own at the start of this one.
pub(super) fn envelopes(s: &mut Session, v: &Value) -> CmdResult {
    let delivery = label_text(v, "delivery")?.unwrap_or_else(|| {
        if s.merge.rows.is_empty() { "Recipient Name\nStreet Address\nCity, ST 00000".into() } else { address_block(s, s.merge.record) }
    });
    let ret = label_text(v, "return")?.unwrap_or_default();
    let (w, h) = envelope_size(v)?;
    let sect = SectionProps {
        page_w: w,
        page_h: h,
        landscape: true,
        margin_top: 22.0,
        margin_left: 22.0,
        margin_right: 22.0,
        margin_bottom: 22.0,
        ..Default::default()
    };
    // Addresses are capped: a pasted novel is not an address.
    let lines = |t: &str| t.split('\n').take(20).map(str::to_string).collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let ret_lines = if ret.is_empty() { Vec::new() } else { lines(&ret) };
    for l in &ret_lines {
        blocks.push(para_block(Paragraph::with_text(l, CharProps { size: Some(10.0), ..Default::default() }).styled("NoSpacing")));
    }
    let spacer = (6 - ret_lines.len().min(6)).max(1);
    for _ in 0..spacer + 2 {
        blocks.push(para_block(Paragraph::new().styled("NoSpacing")));
    }
    let mut last = None;
    for l in lines(&delivery) {
        let mut p = Paragraph::with_text(&l, CharProps::default()).styled("NoSpacing");
        p.props.indent_left = Some(w * 0.42);
        if let Some(prev) = last.replace(p) {
            blocks.push(para_block(prev));
        }
    }
    if p::bool(v, "add") == Some(true) {
        // The envelope's own section ends with its last address line.
        let mut end = last.unwrap_or_else(|| Paragraph::new().styled("NoSpacing"));
        end.section = Some(Box::new(sect));
        blocks.push(para_block(end));
        let n = blocks.len();
        s.doc.body.splice(0..0, blocks);
        s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
        return Ok(json!({"width": w, "height": h, "added": n}));
    }
    blocks.extend(last.map(para_block));
    let mut d = Document::new();
    d.last_section = sect;
    d.body = blocks;
    s.add_document(d);
    Ok(json!({"width": w, "height": h}))
}

/// A sheet of labels: the label size, how many across and down, and where the first one sits.
/// Lengths in points; the pitch is from one label's edge to the next one's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelProduct {
    pub id: &'static str,
    /// What the labels are for: `address`, `shipping` or `return`.
    pub kind: &'static str,
    /// Letter or A4.
    pub paper: &'static str,
    pub cols: usize,
    pub rows: usize,
    pub width: f32,
    pub height: f32,
    pub top: f32,
    pub side: f32,
    pub h_pitch: f32,
    pub v_pitch: f32,
}

const MM: f32 = 72.0 / 25.4;

/// Common label sheets (the sizes are the industry's; the names are ours).
pub const LABEL_PRODUCTS: [LabelProduct; 9] = [
    LabelProduct {
        id: "letter-3x10",
        kind: "address",
        paper: "Letter",
        cols: 3,
        rows: 10,
        width: 189.0,
        height: 72.0,
        top: 36.0,
        side: 13.5,
        h_pitch: 198.0,
        v_pitch: 72.0,
    },
    LabelProduct {
        id: "letter-2x10",
        kind: "address",
        paper: "Letter",
        cols: 2,
        rows: 10,
        width: 288.0,
        height: 72.0,
        top: 36.0,
        side: 11.25,
        h_pitch: 301.5,
        v_pitch: 72.0,
    },
    LabelProduct {
        id: "letter-2x7",
        kind: "address",
        paper: "Letter",
        cols: 2,
        rows: 7,
        width: 288.0,
        height: 96.0,
        top: 60.0,
        side: 11.25,
        h_pitch: 301.5,
        v_pitch: 96.0,
    },
    LabelProduct {
        id: "letter-2x5",
        kind: "shipping",
        paper: "Letter",
        cols: 2,
        rows: 5,
        width: 288.0,
        height: 144.0,
        top: 36.0,
        side: 11.25,
        h_pitch: 301.5,
        v_pitch: 144.0,
    },
    LabelProduct {
        id: "letter-4x20",
        kind: "return",
        paper: "Letter",
        cols: 4,
        rows: 20,
        width: 126.0,
        height: 36.0,
        top: 36.0,
        side: 21.6,
        h_pitch: 147.6,
        v_pitch: 36.0,
    },
    LabelProduct {
        id: "a4-3x7",
        kind: "address",
        paper: "A4",
        cols: 3,
        rows: 7,
        width: 63.5 * MM,
        height: 38.1 * MM,
        top: 15.15 * MM,
        side: 7.21 * MM,
        h_pitch: 66.04 * MM,
        v_pitch: 38.1 * MM,
    },
    LabelProduct {
        id: "a4-2x7",
        kind: "address",
        paper: "A4",
        cols: 2,
        rows: 7,
        width: 99.1 * MM,
        height: 38.1 * MM,
        top: 15.15 * MM,
        side: 4.65 * MM,
        h_pitch: 101.6 * MM,
        v_pitch: 38.1 * MM,
    },
    LabelProduct {
        id: "a4-3x8",
        kind: "address",
        paper: "A4",
        cols: 3,
        rows: 8,
        width: 63.5 * MM,
        height: 33.9 * MM,
        top: 12.9 * MM,
        side: 7.21 * MM,
        h_pitch: 66.04 * MM,
        v_pitch: 33.9 * MM,
    },
    LabelProduct {
        id: "a4-2x4",
        kind: "shipping",
        paper: "A4",
        cols: 2,
        rows: 4,
        width: 99.1 * MM,
        height: 67.7 * MM,
        top: 13.1 * MM,
        side: 4.65 * MM,
        h_pitch: 101.6 * MM,
        v_pitch: 67.7 * MM,
    },
];

impl LabelProduct {
    pub fn by_id(id: &str) -> Option<&'static LabelProduct> {
        LABEL_PRODUCTS.iter().find(|p| p.id == id)
    }
    /// Label size as people say it: inches on Letter, millimetres on A4.
    pub fn size_text(&self) -> String {
        if self.paper == "A4" {
            format!("{:.1} × {:.1} mm", self.width / MM, self.height / MM)
        } else {
            let i = |pt: f32| format!("{}", ((pt / 72.0) * 1000.0).round() / 1000.0);
            format!("{}\" × {}\"", i(self.width), i(self.height))
        }
    }
}

/// Labels: a new document with a table of labels laid out like the sheet (`product`; spacer
/// columns and rows stand for the gaps). A full page of the same `text` (or one recipient per
/// label), or (`single`: `{"row", "col"}`, from 1) one label at that place on the sheet.
pub(super) fn labels(s: &mut Session, v: &Value) -> CmdResult {
    let prod = match p::str(v, "product") {
        Some(id) => *LabelProduct::by_id(id).ok_or_else(|| CmdError::Params(format!("unknown label product `{id}`")))?,
        None => LABEL_PRODUCTS[0],
    };
    let rows = p::u64(v, "rows").map_or(prod.rows, |n| n.clamp(1, 40) as usize);
    let cols = p::u64(v, "cols").map_or(prod.cols, |n| n.clamp(1, 10) as usize);
    let single = v.get("single").filter(|x| !x.is_null()).map(|x| {
        let at = |k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
        (at("row").min(rows), at("col").min(cols))
    });
    let from_rec = single.is_none() && p::bool(v, "fromRecipients").unwrap_or(!s.merge.rows.is_empty() && p::str(v, "text").is_none());
    let text = label_text(v, "text")?.unwrap_or_else(|| "Name\nAddress\nCity, ST 00000".into());
    let (page_w, page_h) = if prod.paper == "A4" { (595.3, 841.9) } else { (612.0, 792.0) };
    let h_gap = (prod.h_pitch - prod.width).max(0.0);
    let v_gap = (prod.v_pitch - prod.height).max(0.0);
    let mut d = Document::new();
    d.last_section.page_w = page_w;
    d.last_section.page_h = page_h;
    d.last_section.margin_top = prod.top;
    d.last_section.margin_bottom = 0.0;
    d.last_section.margin_left = prod.side;
    d.last_section.margin_right = (page_w - prod.side - prod.h_pitch * (cols - 1) as f32 - prod.width).max(0.0);
    d.last_section.header = 0.0;
    d.last_section.footer = 0.0;
    // Grid columns: label, gap, label, …; rows likewise.
    let grid_cols: Vec<Option<usize>> = (0..cols).flat_map(|c| [Some(c)].into_iter().chain((c + 1 < cols && h_gap > 0.5).then_some(None))).collect();
    let grid_rows: Vec<Option<usize>> = (0..rows).flat_map(|r| [Some(r)].into_iter().chain((r + 1 < rows && v_gap > 0.5).then_some(None))).collect();
    let width: f32 = grid_cols.iter().map(|c| if c.is_some() { prod.width } else { h_gap }).sum();
    let mut t = Table::new(grid_rows.len(), grid_cols.len(), width);
    t.props.style = None;
    t.props.borders = Some(wordcraft_doc::props::Borders::default());
    t.props.fixed = true;
    t.grid = grid_cols.iter().map(|c| if c.is_some() { prod.width } else { h_gap }).collect();
    let mut k = 0usize;
    let mut made = 0usize;
    for (row, r) in t.rows.iter_mut().zip(&grid_rows) {
        row.props.height = Some(if r.is_some() { prod.height } else { v_gap });
        row.props.height_rule = wordcraft_doc::props::HeightRule::Exact;
        for (cell, c) in row.cells.iter_mut().zip(&grid_cols) {
            cell.props.width = Some(if c.is_some() { prod.width } else { h_gap });
            let (Some(r), Some(c)) = (r, c) else { continue };
            let content = match single {
                Some((sr, sc)) if (sr, sc) != (r + 1, c + 1) => String::new(),
                _ if from_rec => {
                    let rec = if k < s.merge.rows.len() { address_block(s, k) } else { String::new() };
                    k += 1;
                    rec
                }
                _ => text.clone(),
            };
            if !content.is_empty() {
                made += 1;
            }
            cell.props.valign = Some(wordcraft_doc::props::VAlign::Center);
            cell.blocks = content
                .split('\n')
                .take(20)
                .map(|l| {
                    let mut p = Paragraph::with_text(l, CharProps::default()).styled("NoSpacing");
                    p.props.align = Some(Align::Left);
                    p.props.indent_left = Some(9.0);
                    para_block(p)
                })
                .collect();
        }
    }
    d.body = vec![std::sync::Arc::new(Block::Table(t)), para_block(Paragraph::new())];
    s.add_document(d);
    let _ = Pos::body(0, 0);
    Ok(json!({"labels": if single.is_some() { 1 } else { rows * cols }, "filled": made, "product": prod.id}))
}
