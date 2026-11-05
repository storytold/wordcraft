//! Paragraph text frames ([MS-DOC] §2.6.13): the frame sprms a paragraph mark (or its style)
//! can carry, and the grouping of consecutive framed paragraphs into floating text-box parts.
//!
//! Word positions a framed paragraph group absolutely on the page; the body flow only keeps an
//! anchor. [`extract`] moves each group into a `PartKind::TextBox` part and replaces it in the
//! body with one paragraph holding a story-carrying [`InlineObject::Shape`] — the same shape
//! the EScher reader (`drawing.rs`) builds for drawn text boxes — with the frame's geometry as
//! its [`Float`]. The mapping is an approximation by design ([spec §F]): sizes come from the
//! sprms when present, else from sensible defaults; values are clamped, never errors.

use wordcraft_doc::para::{Anchor, Float, FloatAlign, InlineObject, Run, ShapeKind, Wrap};
use wordcraft_doc::props::{CharProps, LineSpacing};
use wordcraft_doc::styles::StyleSheet;
use wordcraft_doc::{Document, PartKind};

use crate::sections::Section;
use crate::sprm;
use crate::table::{self, ParaOut};

/// `sprmPDxaAbs`: horizontal position, `XAS_plusOne` (stored value − 1 = twips), with negative
/// alignment sentinels ([MS-DOC] §2.6.13, ECMA-376 ST_XAlign).
const P_DXA_ABS: u16 = 0x8418;
/// `sprmPDyaAbs`: downward vertical position, `YAS_plusOne`, with alignment sentinels
/// (ST_YAlign).
const P_DYA_ABS: u16 = 0x8419;
/// `sprmPDxaWidth`: frame width in twips; 0 = automatic.
const P_DXA_WIDTH: u16 = 0x841A;
/// `sprmPPc`: `PositionCodeOperand` — which edges the offsets are relative to.
const P_PC: u16 = 0x261B;
/// `sprmPWr`: how body text wraps around the frame (ST_Wrap).
const P_WR: u16 = 0x2423;
/// `sprmPWHeightAbs`: `WHeightAbs` — frame height, bit 15 = minimum-height flag.
const P_W_HEIGHT_ABS: u16 = 0x442B;
/// `sprmPDyaFromText` / `sprmPDxaFromText`: distances wrapping text keeps.
const P_DYA_FROM_TEXT: u16 = 0x842E;
const P_DXA_FROM_TEXT: u16 = 0x842F;

/// Largest offset or size kept (points; 22 inches, Word's own limit).
const MAX_PT: f32 = 1584.0;
/// Box insets the layout gives a text-box story (`layout`'s `BOX_INSET_X`/`BOX_INSET_Y`, Word's
/// 0.1"/0.05" defaults). A paragraph frame has no insets of its own, so the box is grown by
/// them to keep the frame's own geometry for the text inside.
const BOX_INSET_X: f32 = 7.2;
const BOX_INSET_Y: f32 = 3.6;
/// Most paragraphs whose height feeds the automatic-frame estimate.
const MAX_EST_PARAS: usize = 256;
/// Most table rows the estimate counts.
const MAX_EST_ROWS: usize = 512;

/// The frame properties of one paragraph: the effective sprm values after the style chain and
/// the paragraph's own grpprl are merged (per sprm, the later wins). Equality on this drives
/// grouping: Word merges consecutive framed paragraphs with equal properties into one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FrameInfo {
    /// Raw `sprmPPc` operand.
    pc: Option<u8>,
    /// Raw `sprmPDxaAbs`, sentinels intact.
    dxa_abs: Option<i16>,
    dya_abs: Option<i16>,
    /// Raw `sprmPDxaWidth`; negative values are junk and read as automatic.
    width: Option<i16>,
    /// Raw `sprmPWHeightAbs`.
    height: Option<u16>,
    wrap: Option<u8>,
    dxa_from_text: Option<i16>,
    dya_from_text: Option<i16>,
}

impl FrameInfo {
    /// Apply a grpprl's frame sprms over what earlier grpprls set (style chain, then direct).
    fn apply(&mut self, papx: &[u8]) {
        for prl in sprm::iter(papx) {
            let s16 = |p: &sprm::Prl| -> Option<i16> {
                match p.operand {
                    [b0, b1, ..] => Some(i16::from_le_bytes([*b0, *b1])),
                    _ => None,
                }
            };
            match prl.op {
                P_PC => self.pc = prl.operand.first().copied(),
                P_DXA_ABS => self.dxa_abs = s16(&prl),
                P_DYA_ABS => self.dya_abs = s16(&prl),
                P_DXA_WIDTH => self.width = s16(&prl).filter(|v| *v >= 0),
                P_W_HEIGHT_ABS => {
                    self.height = match prl.operand {
                        [b0, b1, ..] => Some(u16::from_le_bytes([*b0, *b1])),
                        _ => None,
                    };
                }
                P_WR => self.wrap = prl.operand.first().copied(),
                P_DYA_FROM_TEXT => self.dya_from_text = s16(&prl),
                P_DXA_FROM_TEXT => self.dxa_from_text = s16(&prl),
                _ => {}
            }
        }
    }

    /// Whether the paragraph is framed: a `sprmPPc` whose two position codes are not both
    /// "none" (3), or any positioning sprm when `sprmPPc` is absent (its default codes anchor
    /// to the margin/column).
    fn framed(&self) -> bool {
        match self.pc {
            Some(p) => (p >> 4) & 3 != 3 || (p >> 6) & 3 != 3,
            None => self.dxa_abs.is_some() || self.dya_abs.is_some() || self.width.is_some() || self.height.is_some(),
        }
    }

    /// The frame's placement and its explicit size, if the sprms give one (points).
    fn geometry(&self) -> (Float, Option<f32>, Option<f32>) {
        // PositionCodeOperand: bits 4-5 name the vertical anchor (0 top page margin, 1 page
        // top, 2 the preceding paragraph's bottom), bits 6-7 the horizontal one (0 column
        // left, 1 left page margin, 2 page left). Code 3 is "not positioned"; such a
        // paragraph is not framed, so it cannot reach here through `framed`.
        let pc = self.pc.unwrap_or(0);
        let v_rel = match (pc >> 4) & 3 {
            1 => Anchor::Page,
            2 => Anchor::Paragraph,
            _ => Anchor::Margin,
        };
        let h_rel = match (pc >> 6) & 3 {
            1 => Anchor::Margin,
            2 => Anchor::Page,
            _ => Anchor::Column,
        };
        // XAS_plusOne/YAS_plusOne: the stored value minus one is the twip distance, and the
        // values around -4 are alignment sentinels instead (ST_XAlign/ST_YAlign), which take
        // precedence over any offset.
        let (x, h_align) = match self.dxa_abs {
            Some(-4) => (0.0, Some(FloatAlign::Center)),
            Some(-8) => (0.0, Some(FloatAlign::End)),
            Some(-12) => (0.0, Some(FloatAlign::Inside)),
            Some(-16) => (0.0, Some(FloatAlign::Outside)),
            Some(v) => (((v as f32) - 1.0) / 20.0, None),
            None => (0.0, None),
        };
        let (y, v_align) = match self.dya_abs {
            Some(-8) => (0.0, Some(FloatAlign::Center)),
            Some(-12) => (0.0, Some(FloatAlign::End)),
            Some(-16) => (0.0, Some(FloatAlign::Inside)),
            Some(-20) => (0.0, Some(FloatAlign::Outside)),
            // 0xFFFC ("top") and 0x0000 ("inline") both land at the anchor's top.
            Some(v) => (((v as f32) - 1.0) / 20.0, None),
            None => (0.0, None),
        };
        // sprmPWr → ST_Wrap; unknown values fall back to wrapped-around, the default.
        let wrap = match self.wrap {
            Some(1) => Wrap::TopAndBottom,
            Some(3) => Wrap::InFrontOfText,
            Some(4) => Wrap::Tight,
            Some(5) => Wrap::Through,
            _ => Wrap::Square,
        };
        let tw = |v: f32| v.clamp(-MAX_PT, MAX_PT);
        let nonneg = |v: f32| v.clamp(0.0, MAX_PT);
        let float = Float {
            wrap,
            h_rel,
            v_rel,
            x: tw(x),
            y: tw(y),
            h_align,
            v_align,
            dist: self.dxa_from_text.map_or(0.0, |v| nonneg(v as f32 / 20.0)),
            dist_top: self.dya_from_text.map_or(0.0, |v| nonneg(v as f32 / 20.0)),
            dist_bottom: self.dya_from_text.map_or(0.0, |v| nonneg(v as f32 / 20.0)),
            ..Float::default()
        };
        let w = self.width.map(|v| nonneg(v as f32 / 20.0));
        // WHeightAbs: 15 bits of twips (0 = automatic), bit 15 = minimum height. A shape box
        // is fixed, so a minimum is taken as the size and growth-with-content is lost.
        let h = self.height.filter(|v| v & 0x7FFF != 0).map(|v| nonneg((v & 0x7FFF) as f32 / 20.0));
        (float, w, h)
    }
}

/// The frame sprms of a paragraph mark: its own PAPX over the frame sprms of its style chain
/// (root first, the paragraph's style last), per sprm the later grpprl wins.
pub(crate) fn decode(raw_styles: &[crate::stsh::RawStyle], istd: u16, direct: &[u8]) -> Option<FrameInfo> {
    let mut info = FrameInfo::default();
    for i in crate::fmt::style_chain(raw_styles, istd as usize) {
        if let Some(st) = raw_styles.get(i) {
            info.apply(&st.papx);
        }
    }
    info.apply(direct);
    info.framed().then_some(info)
}

/// Group consecutive framed paragraphs into floating parts. A table whose marks are framed
/// frames as a whole (Word frames a table through its row paragraphs), so an unframed cell
/// paragraph between framed row marks joins its table's frame. Groups of empty paragraphs
/// alone stay in the body: an invisible frame is noise, and skipping it keeps trailing
/// style-carried marks (the repro's cover has one) from shadowing real frames.
pub(crate) fn extract(paras: Vec<ParaOut>, sects: &[Section], sheet: &StyleSheet, doc: &mut Document) -> Vec<ParaOut> {
    // Effective frame per paragraph, tables stamped whole.
    let mut frames: Vec<Option<FrameInfo>> = paras.iter().map(|p| p.frame.clone()).collect();
    let in_table = |p: &ParaOut| p.row.in_table || p.row.ttp || p.row.cell_mark;
    let mut i = 0;
    while i < paras.len() {
        if !in_table(&paras[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < paras.len() && in_table(&paras[i]) {
            i += 1;
        }
        if let Some(f) = frames[start..i].iter().find_map(|f| f.clone()) {
            for slot in &mut frames[start..i] {
                *slot = Some(f.clone());
            }
        }
    }

    let mut out: Vec<ParaOut> = Vec::with_capacity(paras.len());
    let mut group: Vec<ParaOut> = Vec::new();
    let mut cur: Option<FrameInfo> = None;
    for (idx, mut p) in paras.into_iter().enumerate() {
        let f = frames[idx].clone();
        if f.as_ref() != cur.as_ref() {
            flush_group(&mut group, cur.take(), &mut out, sects, sheet, doc);
            cur = f;
        }
        // A section break ends the group: the boundary belongs to the body flow, and the
        // paragraph that carries it into a text-box story would put the same break in both.
        if p.para.section.is_some() && !group.is_empty() {
            let sect = p.para.section.take();
            flush_group(&mut group, cur.take(), &mut out, sects, sheet, doc);
            p.para.section = sect;
            out.push(p);
            continue;
        }
        if cur.is_some() {
            group.push(p);
        } else {
            out.push(p);
        }
    }
    flush_group(&mut group, cur.take(), &mut out, sects, sheet, doc);
    out
}

/// Finish the group gathered so far: into a floating part with an anchor paragraph, or back
/// into the body unchanged when it has nothing to show (empty paragraphs only) or the part
/// cap is reached.
fn flush_group(
    group: &mut Vec<ParaOut>,
    frame: Option<FrameInfo>,
    out: &mut Vec<ParaOut>,
    sects: &[Section],
    sheet: &StyleSheet,
    doc: &mut Document,
) {
    let Some(f) = frame else {
        out.append(group);
        return;
    };
    let in_table = |p: &ParaOut| p.row.in_table || p.row.ttp || p.row.cell_mark;
    if group.is_empty() || doc.parts.len() >= crate::MAX_PARTS {
        out.append(group);
        return;
    }
    if !group.iter().any(|p| !p.para.text.is_empty() || !p.para.objects.is_empty() || in_table(p)) {
        out.append(group);
        return;
    }
    // The first paragraph's CP and a section break anywhere in the group, before the group
    // is assembled into the part's story.
    let end_cp = group[0].end_cp;
    let keeps: Vec<bool> = group[0]
        .para
        .objects
        .iter()
        .map(|o| {
            matches!(
                o,
                InlineObject::BookmarkStart { .. }
                    | InlineObject::BookmarkEnd { .. }
                    | InlineObject::FieldStart { .. }
                    | InlineObject::FieldEnd
                    | InlineObject::NoteRef { .. }
                    | InlineObject::CommentStart { .. }
                    | InlineObject::CommentEnd { .. }
                    | InlineObject::PermStart { .. }
                    | InlineObject::PermEnd { .. }
            ) || matches!(o, InlineObject::Field { result, .. } if result.is_empty())
        })
        .collect();
    // The kept objects move to the anchor (below), out of the story's first paragraph: left
    // in place they would appear in both — a NoteRef referenced twice, a bookmark pair opened
    // on the anchor and inside the story. The story keeps what shows.
    let mut first_objects: Vec<InlineObject> = Vec::new();
    let mut story_objects: Vec<InlineObject> = Vec::new();
    for (k, o) in group[0].para.objects.iter().enumerate() {
        if keeps[k] {
            first_objects.push(o.clone());
        } else {
            story_objects.push(o.clone());
        }
    }
    group[0].para.objects = story_objects;
    let section = group.iter().rev().find_map(|p| p.para.section.clone());
    // The section break belongs to the anchor alone now: a `section` left on a grouped
    // paragraph would ride into the text-box story as a second copy of the same break.
    for p in group.iter_mut() {
        p.para.section = None;
    }
    let blocks = table::assemble(std::mem::take(group));
    let story = doc.add_part(PartKind::TextBox, blocks);
    let (float, w, h) = f.geometry();
    // Default box: the story's own estimated width when the file leaves the frame's width
    // automatic ([MS-DOC] sprmPDxaWidth: content-fit) — but only for a frame aligned
    // centre/right/etc., whose text position depends on the box hugging its content (the
    // repro's one-line header). A left-offset frame's text starts at the box's left edge
    // whatever its width, so those keep the whole column and cannot newly wrap. The box
    // insets the layout gives a text-box story are added on so the frame's own geometry —
    // the size its text lays out in — survives inside the box.
    let column = column_width(sects, end_cp);
    let w = w.map(|v| v + 2.0 * BOX_INSET_X).unwrap_or_else(|| {
        let auto = if float.h_align.is_some() { estimate_width(&doc.parts[&story].blocks, column, sheet) } else { column };
        auto + 2.0 * BOX_INSET_X
    });
    let h = h.map(|v| v + 2.0 * BOX_INSET_Y).unwrap_or_else(|| estimate_height(&doc.parts[&story].blocks, w, sheet) + 2.0 * BOX_INSET_Y);
    let obj = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: w.clamp(12.0, MAX_PT),
        h: h.clamp(12.0, MAX_PT),
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float,
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    // The anchor keeps the group's first paragraph's invisible inline objects (bookmarks,
    // note references, a field whose cached result is empty, like a TOC range anchor);
    // anything that shows — pictures, shapes, a field rendering its result text — stays in
    // the story, where copying it onto the anchor would duplicate it.
    let mut objects: Vec<InlineObject> = first_objects;
    let n = objects.len() + 1;
    objects.push(obj);
    // The anchor line is as small as the model allows: Word shows nothing for a frame's
    // anchor, and a full line of the framed paragraph's style both stacks the anchors up
    // (pushing later ones into their own frames' wrap bands, off the page) and leaves stray
    // empty space in the flow.
    let mut anchor = wordcraft_doc::para::Paragraph::new();
    for _ in 0..n {
        anchor.text.push('\u{FFFC}');
    }
    anchor.runs = vec![Run { len: 3 * n, props: CharProps { size: Some(1.0), ..CharProps::default() } }];
    anchor.props.line_spacing = Some(LineSpacing::Exactly(2.0));
    anchor.props.space_before = Some(0.0);
    anchor.props.space_after = Some(0.0);
    anchor.mark.size = Some(1.0);
    anchor.objects = objects;
    anchor.section = section;
    out.push(ParaOut { para: anchor, end_cp, row: table::RowInfo::default(), frame: None });
}

/// The width of the text column of the section covering CP `end_cp` (points). Sections are
/// ordered by end CP; the first reaching past the paragraph is its own.
fn column_width(sects: &[Section], end_cp: u32) -> f32 {
    let props = sects.iter().find(|s| s.end_cp >= end_cp).or_else(|| sects.last()).map(|s| &s.props).cloned().unwrap_or_default();
    (props.page_w - props.margin_left - props.margin_right).clamp(72.0, MAX_PT)
}

/// A rough width the story's text needs (points): the widest paragraph's estimated extent —
/// runs at half the font size per byte, a field's cached result counting as text — clamped
/// to the column. The estimate is trusted only when clearly narrower than the column: it
/// can miss a real line by a few percent, and a box narrower than its text wraps it, while
/// a too-wide box only leaves space.
fn estimate_width(blocks: &wordcraft_doc::Blocks, max_w: f32, sheet: &StyleSheet) -> f32 {
    use wordcraft_doc::Block;
    let mut w = 12.0f32;
    for b in blocks.iter().take(MAX_EST_PARAS) {
        match &**b {
            Block::Para(p) => {
                let style_size = p.props.style.as_deref().and_then(|id| sheet.get(id)).and_then(|s| s.chr.size);
                let size_of = |r: &Run| r.props.size.or(style_size).unwrap_or(10.0).max(1.0);
                let mut est: f32 = p.runs.iter().map(|r| r.len as f32 * size_of(r) * 0.5).sum();
                // A field renders its cached result in place of the anchor glyph.
                let field_chars: usize = p
                    .objects
                    .iter()
                    .filter_map(|o| match o {
                        InlineObject::Field { result, .. } => Some(result.chars().count()),
                        _ => None,
                    })
                    .sum();
                let size = p.runs.iter().map(size_of).fold(0.0f32, f32::max).max(10.0);
                est += field_chars as f32 * size * 0.5;
                w = w.max(est.min(max_w));
            }
            // A table's own autofit decides its width; give it the whole column.
            Block::Table(_) => return max_w,
        }
    }
    if w < 0.6 * max_w { w } else { max_w }
}

/// A generous estimate of the height the story needs in a box `w` wide (points), for frames
/// whose height the file leaves automatic. Direct run sizes win; a paragraph's style carries
/// the size when the runs do not (the repro's cover titles); everything else is a crude
/// over-estimate on purpose — a too-tall box leaves space, a too-short one hides text.
fn estimate_height(blocks: &wordcraft_doc::Blocks, w: f32, sheet: &StyleSheet) -> f32 {
    use wordcraft_doc::Block;
    let inner = (w - 2.0 * BOX_INSET_X).max(12.0);
    let mut h = 0.0f32;
    let mut seen = 0usize;
    for b in blocks.iter().take(MAX_EST_PARAS) {
        match &**b {
            Block::Para(p) => h += para_height(p, inner, sheet),
            Block::Table(t) => {
                // Rows at Word's minimum height; a cell's paragraphs keep their own line
                // count, the tallest cell sizing the row.
                let cells = (t.grid.len().max(1) as f32).max(1.0);
                let col = (inner / cells).max(12.0);
                for row in t.rows.iter().take(MAX_EST_ROWS) {
                    let lines: f32 = row
                        .cells
                        .iter()
                        .map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| para_height(p, col, sheet)).sum::<f32>())
                        .fold(0.0f32, f32::max);
                    h += lines.max(14.2);
                    seen += 1;
                }
            }
        }
        seen += 1;
        if seen >= MAX_EST_PARAS {
            break;
        }
    }
    h.clamp(12.0, MAX_PT)
}

/// One paragraph's estimated height at line width `w`: lines (half the font size per
/// character is a wide guess, on purpose) times the line height, plus the paragraph spacing.
fn para_height(p: &wordcraft_doc::para::Paragraph, w: f32, sheet: &StyleSheet) -> f32 {
    let style_size = p.props.style.as_deref().and_then(|id| sheet.get(id)).and_then(|s| s.chr.size);
    let size_of = |r: &wordcraft_doc::para::Run| r.props.size.or(style_size).unwrap_or(10.0).max(1.0);
    let size = p.runs.iter().map(size_of).fold(0.0f32, f32::max).max(10.0);
    let line = match p.props.line_spacing {
        Some(LineSpacing::Exactly(v)) | Some(LineSpacing::AtLeast(v)) if v.is_finite() => v.max(size),
        Some(LineSpacing::Multiple(m)) if m.is_finite() => size * 1.15 * m.max(0.1),
        _ => size * 1.35,
    };
    let est_w: f32 = p.runs.iter().map(|r| r.len as f32 * size_of(r) * 0.5).sum();
    let lines = if p.runs.is_empty() { 1.0 } else { (est_w / w).ceil().clamp(1.0, 100.0) };
    lines * line + p.props.space_before.unwrap_or(0.0).max(0.0) + p.props.space_after.unwrap_or(0.0).max(0.0)
}
