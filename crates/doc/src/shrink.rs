//! Fit One Page typography helper.
//!
//! Adjusts font sizes and paragraph spacing across styles and editable stories
//! while preserving visual hierarchy, clamping to safe bounds, and excluding
//! fixed framing stories like headers and footers.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::edit::{each_para, each_para_mut};
use crate::para::Paragraph;
use crate::parts::PartKind;
use crate::props::{CharProps, LineSpacing, ParaProps};
use crate::styles::StyleSheet;
use crate::{Block, Document};

/// Minimum font size allowed in points (WordCraft floor).
const MIN_SHRINK_FONT_SIZE: f32 = 4.0;

/// Maximum font size allowed in points.
const MAX_SHRINK_FONT_SIZE: f32 = 1638.0;

/// Safe line spacing floor factor (preventing line collision / crowding).
const MIN_LINE_SPACING_FACTOR: f32 = 0.94;

/// Cumulative fit-to-page typography reduction state.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FitState {
    /// Cumulative font reduction delta in points.
    pub cum_delta: f32,
    /// Cumulative line spacing scale factor.
    pub cum_space: f32,
    /// Unique effective body font sizes present immediately after the reduction was applied.
    pub reduced_font_sizes: Vec<f32>,
    /// Unique effective line spacing values present immediately after the reduction was applied.
    pub reduced_line_spacings: Vec<LineSpacing>,
    /// Unique effective paragraph spacing values present immediately after the reduction was applied.
    #[serde(default)]
    pub reduced_paragraph_spacings: Vec<f32>,
    /// Unique effective body font sizes before the first Fit One Page reduction was applied.
    #[serde(default)]
    pub original_font_sizes: Vec<f32>,
    /// Unique effective line spacing values before the first Fit One Page reduction was applied.
    #[serde(default)]
    pub original_line_spacings: Vec<LineSpacing>,
    /// Unique effective paragraph spacing values before the first Fit One Page reduction was applied.
    #[serde(default)]
    pub original_paragraph_spacings: Vec<f32>,
    /// Original effective body font sizes by formatting location key before the first fit.
    #[serde(default)]
    pub original_font_locations: std::collections::BTreeMap<String, f32>,
    /// Original effective line spacing values by formatting location key before the first fit.
    #[serde(default)]
    pub original_line_spacing_locations: std::collections::BTreeMap<String, LineSpacing>,
    /// Original effective paragraph spacing values by formatting location key before the first fit.
    #[serde(default)]
    pub original_paragraph_spacing_locations: std::collections::BTreeMap<String, f32>,
}

/// Scale a font size down by `delta_pt`.
#[inline]
fn scale_size(size: f32, delta_pt: f32) -> f32 {
    if delta_pt == 0.0 {
        return size;
    }
    // Round up so the reduction never exceeds `delta_pt`, and never enlarge.
    let target = (size - delta_pt).clamp(MIN_SHRINK_FONT_SIZE, MAX_SHRINK_FONT_SIZE);
    ((target * 2.0).ceil() / 2.0).min(size)
}

/// Scale a spacing value (e.g. `space_before`, `space_after`) by `space_factor`.
///
/// Caps the result at the original nonnegative spacing value and rounds up to hundredths (with a
/// tiny epsilon for floating-point noise), so a trial never removes more spacing than its factor
/// permits and never increases spacing. Rounding down would over-compress very small values.
#[inline]
fn scale_spacing(val: f32, space_factor: f32) -> f32 {
    let non_neg = val.max(0.0);
    let scaled = (non_neg * space_factor).clamp(0.0, 1584.0);
    ((scaled * 100.0 - 1e-4).ceil() / 100.0).min(non_neg)
}

/// Scale fixed line spacing in points respecting the safe readability floor and rounding upward to half-points.
///
/// Rounds upward to half-points so the result never falls below the 0.94 floor, and caps at `pts` so
/// reduction never increases fixed line spacing.
#[inline]
fn scale_fixed_line_spacing(pts: f32, space_factor: f32) -> f32 {
    let floor = pts * MIN_LINE_SPACING_FACTOR;
    let scaled = (pts * space_factor).max(floor).clamp(0.0, 1584.0);
    // Round upward to half-points (0.5 pt) so the resulting spacing never falls below the 0.94 floor.
    ((scaled * 2.0).ceil() / 2.0).min(pts)
}

/// Scale line spacing by `space_factor` respecting the safe readability floor.
#[inline]
fn scale_line_spacing(ls: LineSpacing, space_factor: f32) -> LineSpacing {
    match ls {
        LineSpacing::Multiple(m) => {
            // Guard against ascender/descender collisions: keep line spacing at or above
            // safe readability floor of 0.94 factor (~6% max compression).
            let floor = m * MIN_LINE_SPACING_FACTOR;
            let new_m = (m * space_factor).max(floor).clamp(0.5, 132.0);
            // Round upward to a hundredth so the resulting multiple never falls below the 0.94 floor.
            LineSpacing::Multiple(((new_m * 100.0).ceil() / 100.0).min(m))
        }
        LineSpacing::AtLeast(pts) => LineSpacing::AtLeast(scale_fixed_line_spacing(pts, space_factor)),
        LineSpacing::Exactly(pts) => LineSpacing::Exactly(scale_fixed_line_spacing(pts, space_factor)),
    }
}

/// Scale explicit font sizes on a `CharProps`.
#[inline]
fn scale_char_props(c: &mut CharProps, delta_pt: f32) {
    if delta_pt > 0.0 {
        if let Some(s) = c.size.as_mut() {
            *s = scale_size(*s, delta_pt);
        }
        if let Some(s) = c.size_cs.as_mut() {
            *s = scale_size(*s, delta_pt);
        }
    }
}

/// Scale explicit paragraph spacing on a `ParaProps`.
#[inline]
fn scale_para_props(p: &mut ParaProps, space_factor: f32) {
    if space_factor < 1.0 - f32::EPSILON {
        if let Some(sb) = p.space_before.as_mut() {
            *sb = scale_spacing(*sb, space_factor);
        }
        if let Some(sa) = p.space_after.as_mut() {
            *sa = scale_spacing(*sa, space_factor);
        }
        if let Some(ls) = p.line_spacing.as_mut() {
            *ls = scale_line_spacing(*ls, space_factor);
        }
    }
}

/// Finds all text box IDs transitively reachable from Header and Footer parts
/// (including linked text box chains and nested text boxes).
pub fn header_footer_text_boxes(doc: &Document) -> BTreeSet<u32> {
    let mut hf_boxes: BTreeSet<u32> = BTreeSet::new();
    let mut queue: Vec<u32> = Vec::new();

    for part in doc.parts.values() {
        if matches!(part.kind, PartKind::Header | PartKind::Footer) {
            for b in &part.blocks {
                each_para(b, 0, &mut |p| {
                    for obj in &p.objects {
                        for box_id in obj.text_boxes() {
                            if hf_boxes.insert(box_id) {
                                queue.push(box_id);
                            }
                        }
                    }
                });
            }
        }
    }

    while let Some(box_id) = queue.pop() {
        if let Some(part) = doc.parts.get(&box_id) {
            if let Some(next_id) = part.body.next
                && hf_boxes.insert(next_id)
            {
                queue.push(next_id);
            }
            for b in &part.blocks {
                each_para(b, 0, &mut |p| {
                    for obj in &p.objects {
                        for nested_id in obj.text_boxes() {
                            if hf_boxes.insert(nested_id) {
                                queue.push(nested_id);
                            }
                        }
                    }
                });
            }
        }
    }
    hf_boxes
}

/// Freezes resolved character and paragraph formatting on a single paragraph using the provided stylesheet and optional table-style context.
fn freeze_para_formatting(
    p: &mut Paragraph,
    styles: &StyleSheet,
    table_chr: Option<&CharProps>,
    table_para: Option<&ParaProps>,
    delta_pt: f32,
    space_factor: f32,
) {
    let style = p.props.style.clone();
    if delta_pt > 0.0 {
        for r in &mut p.runs {
            if r.props.size.is_none() {
                let resolved = styles.resolve_char_in(style.as_deref(), table_chr, &r.props).size;
                r.props.size = Some(resolved);
            }
            if r.props.size_cs.is_none() {
                let resolved_cs = styles.resolve_char_in(style.as_deref(), table_chr, &r.props).size_cs;
                r.props.size_cs = Some(resolved_cs);
            }
        }
        if p.mark.size.is_none() {
            let resolved = styles.resolve_char_in(style.as_deref(), table_chr, &p.mark).size;
            p.mark.size = Some(resolved);
        }
        if p.mark.size_cs.is_none() {
            let resolved_cs = styles.resolve_char_in(style.as_deref(), table_chr, &p.mark).size_cs;
            p.mark.size_cs = Some(resolved_cs);
        }
    }
    if space_factor < 1.0 - f32::EPSILON {
        let resolved_p = styles.resolve_para_in(&p.props, table_para);
        if p.props.space_before.is_none() {
            p.props.space_before = Some(resolved_p.space_before);
        }
        if p.props.space_after.is_none() {
            p.props.space_after = Some(resolved_p.space_after);
        }
        if p.props.line_spacing.is_none() {
            p.props.line_spacing = Some(resolved_p.line_spacing);
        }
    }
}

/// Recursively freezes resolved formatting on blocks while preserving table-style context.
fn freeze_blocks_formatting(
    blocks: &mut [Arc<Block>],
    styles: &StyleSheet,
    table_chr: Option<&CharProps>,
    table_para: Option<&ParaProps>,
    delta_pt: f32,
    space_factor: f32,
) {
    use crate::styles::PartSet;

    for b in blocks {
        let b = Arc::make_mut(b);
        match b {
            Block::Para(p) => {
                freeze_para_formatting(p, styles, table_chr, table_para, delta_pt, space_factor);
            }
            Block::Table(tbl) => {
                let style = tbl.props.style.as_deref().and_then(|id| styles.table_style(id));
                let cell_para = style.as_ref().map(|st| &st.para);
                let nrows = tbl.rows.len();
                let ncols = tbl.cols();
                let cell_grids: Vec<Vec<usize>> =
                    (0..nrows).map(|ri| (0..tbl.rows[ri].cells.len()).map(|ci| tbl.grid_col(ri, ci)).collect()).collect();
                for (ri, row) in tbl.rows.iter_mut().enumerate() {
                    for (ci, cell) in row.cells.iter_mut().enumerate() {
                        let mut cell_chr = CharProps::default();
                        if let Some(st) = &style {
                            let sizes = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
                            let g = cell_grids.get(ri).and_then(|r| r.get(ci)).copied().unwrap_or(0);
                            let span = cell.span();
                            let region = PartSet::of_cell(&tbl.props.look, sizes, (ri, nrows), (g, span, ncols));
                            cell_chr.overlay(&st.chr);
                            for part in region.iter() {
                                let f = st.part(part);
                                cell_chr.overlay(&f.chr);
                            }
                        }
                        freeze_blocks_formatting(&mut cell.blocks, styles, Some(&cell_chr), cell_para, delta_pt, space_factor);
                    }
                }
            }
        }
    }
}

/// Fits font sizes across the document by `delta_pt` points and scales paragraph
/// spacing by `space_factor`.
///
/// Clean-room typographical helper for Fit One Page. Preserves relative typography
/// hierarchies between headings, body text, and direct run formatting.
/// Clamps font sizes to `[4.0, 1638.0]` pt and rounds to 0.5 pt (half-points).
/// Headers, footers, and comments are explicitly excluded and retain their exact original sizes.
/// Footnotes are included and shrink alongside body text.
pub fn fit_document_typography(doc: &mut Document, delta_pt: f32, space_factor: f32) {
    if !delta_pt.is_finite() || !space_factor.is_finite() {
        return;
    }
    let delta_pt = delta_pt.max(0.0);
    let space_factor = space_factor.clamp(0.1, 1.0);

    if delta_pt == 0.0 && (space_factor - 1.0).abs() < f32::EPSILON {
        return;
    }

    // 1. Find all text boxes reachable from Header and Footer parts (including nested text boxes and chains).
    let hf_boxes = header_footer_text_boxes(doc);

    // Freeze header, footer, and comment font sizes and spacing (and reachable text boxes) before modifying styles,
    // preserving table-style context so header/footer table cells retain their conditional table formatting,
    // and comment stories retain their inherited formatting from shared styles.
    let old_styles = doc.styles.clone();
    for (id, part) in doc.parts.iter_mut() {
        if matches!(part.kind, PartKind::Header | PartKind::Footer | PartKind::Comment) || hf_boxes.contains(id) {
            freeze_blocks_formatting(&mut part.blocks, &old_styles, None, None, delta_pt, space_factor);
        }
    }

    // 2. Adjust styles: document defaults, character styles, paragraph styles, table style regions.
    let st = Arc::make_mut(&mut doc.styles);

    if delta_pt > 0.0 {
        // Default character formatting
        let default_size = st.default_chr.size.unwrap_or(crate::props::DEFAULT_FONT_SIZE);
        st.default_chr.size = Some(scale_size(default_size, delta_pt));
        if let Some(cs) = st.default_chr.size_cs.as_mut() {
            *cs = scale_size(*cs, delta_pt);
        }
    }

    // Default paragraph formatting
    scale_para_props(&mut st.default_para, space_factor);

    // All named styles (excluding header/footer styles if any)
    for style in &mut st.styles {
        let is_hf = style.id.eq_ignore_ascii_case("header")
            || style.id.eq_ignore_ascii_case("footer")
            || style.name.eq_ignore_ascii_case("header")
            || style.name.eq_ignore_ascii_case("footer");
        if is_hf {
            continue;
        }

        if delta_pt > 0.0 {
            scale_char_props(&mut style.chr, delta_pt);
        }
        scale_para_props(&mut style.para, space_factor);

        if let Some(t) = style.table.as_mut()
            && delta_pt > 0.0
        {
            for c in [&mut t.header_chr, &mut t.band_chr, &mut t.first_col_chr, &mut t.last_col_chr, &mut t.total_chr, &mut t.col_band_chr] {
                scale_char_props(c, delta_pt);
            }
            for part in t.regions.values_mut() {
                scale_char_props(&mut part.chr, delta_pt);
            }
        }
    }

    // 3. Helper to shrink editable paragraph content (runs, mark, spacing).
    let mut shrink_paragraph = |p: &mut Paragraph| {
        scale_para_props(&mut p.props, space_factor);

        if delta_pt > 0.0 {
            // Scale explicit paragraph mark size
            scale_char_props(&mut p.mark, delta_pt);

            // Scale explicit run sizes
            for r in &mut p.runs {
                scale_char_props(&mut r.props, delta_pt);
            }
        }

        p.touch();
    };

    // 4. Traverse main body story.
    for b in &mut doc.body {
        each_para_mut(Arc::make_mut(b), 0, &mut shrink_paragraph);
    }

    // 5. Traverse secondary editable stories (footnotes, body text boxes), excluding headers, footers,
    // comments, and text boxes reachable from headers or footers.
    for (id, part) in doc.parts.iter_mut() {
        if matches!(part.kind, PartKind::Header | PartKind::Footer | PartKind::Comment) || hf_boxes.contains(id) {
            continue;
        }
        for b in &mut part.blocks {
            each_para_mut(Arc::make_mut(b), 0, &mut shrink_paragraph);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::para::Paragraph;
    use crate::props::{CharProps, LineSpacing};
    use crate::table::Table;
    use crate::{Document, para_block};

    #[test]
    fn test_shrink_preserves_hierarchy() {
        let mut doc = Document::new();
        // Setup Heading 1 paragraph and Normal paragraph
        let h1_para = Paragraph::with_text("Heading 1 Title", CharProps::default()).styled("Heading1");
        let normal_para = Paragraph::with_text("Body text in Normal style", CharProps::default()).styled("Normal");
        let mut custom_run_para = Paragraph::with_text("Normal prefix ", CharProps::default()).styled("Normal");
        custom_run_para.insert_text(custom_run_para.len(), "Special large text", &CharProps { size: Some(24.0), ..Default::default() }).unwrap();

        doc.body = vec![para_block(h1_para), para_block(normal_para), para_block(custom_run_para)];

        let h1_before = doc.styles.resolve_char(Some("Heading1"), &CharProps::default()).size;
        let normal_before = doc.styles.resolve_char(Some("Normal"), &CharProps::default()).size;

        fit_document_typography(&mut doc, 0.5, 1.0);

        let h1_after = doc.styles.resolve_char(Some("Heading1"), &CharProps::default()).size;
        let normal_after = doc.styles.resolve_char(Some("Normal"), &CharProps::default()).size;

        // Both styles scaled down by exactly 0.5 pt
        assert_eq!(h1_after, h1_before - 0.5);
        assert_eq!(normal_after, normal_before - 0.5);
        // Relative hierarchy preserved
        assert_eq!(h1_after - normal_after, h1_before - normal_before);

        // Explicit run in paragraph 3 was 24.0 pt, now 23.5 pt
        let p3 = doc.body[2].as_para().unwrap();
        assert_eq!(p3.runs[1].props.size, Some(23.5));
    }

    #[test]
    fn test_shrink_excludes_headers_and_footers() {
        let mut doc = Document::new();
        let body_para = Paragraph::with_text("Main story body text", CharProps::default());
        doc.body = vec![para_block(body_para)];

        let header_para = Paragraph::with_text("Header Text", CharProps { size: Some(10.0), ..Default::default() });
        let footer_para = Paragraph::with_text("Footer Text Page 1", CharProps::default()); // inherits

        let h_id = doc.add_part(PartKind::Header, vec![para_block(header_para)]);
        let f_id = doc.add_part(PartKind::Footer, vec![para_block(footer_para)]);

        let footer_before_size = doc.styles.resolve_char(None, &CharProps::default()).size;

        fit_document_typography(&mut doc, 0.5, 1.0);

        // Body default style shrank
        let body_after_size = doc.styles.resolve_char(None, &CharProps::default()).size;
        assert_eq!(body_after_size, footer_before_size - 0.5);

        // Header retained its explicit 10.0 pt size
        let h_part = doc.parts.get(&h_id).unwrap();
        let h_p = h_part.blocks[0].as_para().unwrap();
        assert_eq!(h_p.runs[0].props.size, Some(10.0));

        // Footer retained its original resolved size
        let f_part = doc.parts.get(&f_id).unwrap();
        let f_p = f_part.blocks[0].as_para().unwrap();
        assert_eq!(f_p.runs[0].props.size, Some(footer_before_size));
    }

    #[test]
    fn test_shrink_scales_footnotes() {
        let mut doc = Document::new();
        let body_para = Paragraph::with_text("Main body with note", CharProps::default());
        doc.body = vec![para_block(body_para)];

        let fn_para = Paragraph::with_text("Footnote content", CharProps { size: Some(10.0), ..Default::default() });
        let fn_id = doc.add_part(PartKind::Footnote, vec![para_block(fn_para)]);

        fit_document_typography(&mut doc, 0.5, 1.0);

        let fn_part = doc.parts.get(&fn_id).unwrap();
        let fn_p = fn_part.blocks[0].as_para().unwrap();
        assert_eq!(fn_p.runs[0].props.size, Some(9.5));
    }

    #[test]
    fn test_shrink_clamps_to_min_floor() {
        let mut doc = Document::new();
        let mut p = Paragraph::with_text("Very tiny text", CharProps { size: Some(4.2), ..Default::default() });
        p.insert_text(p.len(), "Floor text", &CharProps { size: Some(4.0), ..Default::default() }).unwrap();
        doc.body = vec![para_block(p)];

        fit_document_typography(&mut doc, 1.0, 1.0);

        let p_after = doc.body[0].as_para().unwrap();
        assert_eq!(p_after.runs[0].props.size, Some(4.0));
        assert_eq!(p_after.runs[1].props.size, Some(4.0));
    }

    #[test]
    fn test_shrink_zero_delta_identity() {
        let mut doc = Document::new();
        let h1_para = Paragraph::with_text("Heading 1 Title", CharProps::default()).styled("Heading1");
        let normal_para = Paragraph::with_text("Body text", CharProps::default());
        doc.body = vec![para_block(h1_para), para_block(normal_para)];
        doc.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Header", CharProps::default()))]);
        doc.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Note", CharProps::default()))]);

        let before = doc.clone();
        fit_document_typography(&mut doc, 0.0, 1.0);
        assert_eq!(doc, before);
    }

    #[test]
    fn test_shrink_paragraph_spacing() {
        let mut doc = Document::new();
        let mut p = Paragraph::with_text("Spaced paragraph", CharProps::default());
        p.props.space_before = Some(18.0);
        p.props.space_after = Some(12.0);
        p.props.line_spacing = Some(LineSpacing::Multiple(1.15));
        doc.body = vec![para_block(p)];

        fit_document_typography(&mut doc, 0.5, 0.9);

        let p_after = doc.body[0].as_para().unwrap();
        // 18.0 * 0.9 = 16.2, capped at the requested reduction
        assert_eq!(p_after.props.space_before, Some(16.2));
        // 12.0 * 0.9 = 10.8, capped at the requested reduction
        assert_eq!(p_after.props.space_after, Some(10.8));
        // 1.15 * 0.9 = 1.035 -> clamped to safe line spacing floor (1.15 * 0.94 = 1.081) -> ceil hundredth 1.09
        assert_eq!(p_after.props.line_spacing, Some(LineSpacing::Multiple(1.09)));
    }

    #[test]
    fn spacing_scaling_never_exceeds_requested_compression() {
        // 0.26 * 0.94 = 0.2444 pt: it must not round down to zero and remove all spacing.
        let tiny = scale_spacing(0.26, 0.94);
        assert!(tiny > 0.0 && tiny <= 0.26, "tiny spacing must shrink without vanishing: {tiny}");
        // 0.019 * 0.94 = 0.01786 pt: rounding down to hundredths would remove 47%, so round up.
        assert_eq!(scale_spacing(0.019, 0.94), 0.019);
        // 18.0 * 0.9 = 16.2 pt: the reduction stays at the requested 1.8 pt.
        assert_eq!(scale_spacing(18.0, 0.9), 16.2);
        assert_eq!(scale_spacing(0.0, 0.9), 0.0);
    }

    #[test]
    fn test_line_spacing_floor_prevents_overlap() {
        let mut doc = Document::new();
        let mut p = Paragraph::with_text("Single spaced text", CharProps::default());
        p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        doc.body = vec![para_block(p)];

        // Even with aggressive space_factor = 0.5, line spacing multiple must not fall below safe floor (0.94)
        fit_document_typography(&mut doc, 0.5, 0.5);

        let p_after = doc.body[0].as_para().unwrap();
        assert_eq!(p_after.props.line_spacing, Some(LineSpacing::Multiple(0.94)));
    }

    #[test]
    fn test_header_text_boxes_are_frozen_and_do_not_shrink() {
        let mut doc = Document::new();
        let body_p = Paragraph::with_text("Body text", CharProps { size: Some(12.0), ..Default::default() });
        doc.body = vec![para_block(body_p)];

        let header_box_id = doc
            .add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Header box", CharProps { size: Some(14.0), ..Default::default() }))]);
        let mut header_p = Paragraph::with_text("Header prefix ", CharProps { size: Some(10.0), ..Default::default() });
        let shape = crate::para::InlineObject::Shape {
            kind: crate::para::ShapeKind::TextBox,
            w: 144.0,
            h: 72.0,
            fill: None,
            stroke: None,
            stroke_width: 0.75,
            float: Default::default(),
            story: Some(header_box_id),
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        header_p.objects.push(shape);
        doc.add_part(PartKind::Header, vec![para_block(header_p)]);

        let body_box_id =
            doc.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Body box", CharProps { size: Some(16.0), ..Default::default() }))]);

        fit_document_typography(&mut doc, 2.0, 1.0);

        let b_p = doc.body[0].as_para().unwrap();
        assert_eq!(b_p.runs[0].props.size, Some(10.0));

        let bb_part = doc.parts.get(&body_box_id).unwrap();
        let bb_p = bb_part.blocks[0].as_para().unwrap();
        assert_eq!(bb_p.runs[0].props.size, Some(14.0));

        let hb_part = doc.parts.get(&header_box_id).unwrap();
        let hb_p = hb_part.blocks[0].as_para().unwrap();
        assert_eq!(hb_p.runs[0].props.size, Some(14.0));
    }

    #[test]
    fn test_shrink_table_cells() {
        let mut doc = Document::new();
        let mut table = Table::new(1, 1, 100.0);
        let cell_p = Paragraph::with_text("Cell text", CharProps { size: Some(12.0), ..Default::default() });
        table.rows[0].cells[0].blocks = vec![para_block(cell_p)];
        doc.body = vec![Arc::new(Block::Table(table))];

        fit_document_typography(&mut doc, 0.5, 1.0);

        let tbl = doc.body[0].as_table().unwrap();
        let cell_p_after = tbl.rows[0].cells[0].blocks[0].as_para().unwrap();
        assert_eq!(cell_p_after.runs[0].props.size, Some(11.5));
    }

    #[test]
    fn test_exact_line_spacing_preserves_floor_upward() {
        // LineSpacing::Exactly(7.5) with space_factor = 0.90 (below MIN_LINE_SPACING_FACTOR 0.94)
        // Floor = 7.5 * 0.94 = 7.05 pt. Rounding upward to half-points yields 7.5 pt (>= 7.05 pt).
        let ls = scale_line_spacing(LineSpacing::Exactly(7.5), 0.90);
        assert_eq!(ls, LineSpacing::Exactly(7.5));

        // LineSpacing::Exactly(10.0) with space_factor = 0.90:
        // Floor = 10.0 * 0.94 = 9.4 pt. Rounding upward to half-points yields 9.5 pt (>= 9.4 pt).
        let ls2 = scale_line_spacing(LineSpacing::Exactly(10.0), 0.90);
        assert_eq!(ls2, LineSpacing::Exactly(9.5));
    }

    #[test]
    fn test_spacing_only_leaves_unrounded_font_sizes_unchanged() {
        let mut doc = Document::new();
        // A run with fractional 9.3 pt font
        let p = Paragraph::with_text("Unrounded size text", CharProps { size: Some(9.3), ..Default::default() });
        doc.body = vec![para_block(p)];

        // Spacing-only trial (delta_pt = 0.0, space_factor = 0.94)
        fit_document_typography(&mut doc, 0.0, 0.94);

        let p_after = doc.body[0].as_para().unwrap();
        assert_eq!(p_after.runs[0].props.size, Some(9.3), "spacing-only trial must not round or scale font size");
    }

    #[test]
    fn test_scale_size_never_enlarges_sub_floor_font() {
        // 2.0 pt run (below 4.0 pt floor) with delta 0.5 pt must not be enlarged to 4.0 pt
        assert_eq!(scale_size(2.0, 0.5), 2.0);
        // 3.0 pt run with delta 0.5 pt must not be enlarged to 4.0 pt
        assert_eq!(scale_size(3.0, 0.5), 3.0);
        // 5.0 pt run with delta 0.5 pt shrinks to 4.5 pt
        assert_eq!(scale_size(5.0, 0.5), 4.5);
        // 12.0 pt run with delta 0.3 pt rounds up to 12.0 pt so reduction (0.0 pt) never exceeds delta (0.3 pt)
        assert_eq!(scale_size(12.0, 0.3), 12.0);
        // Zero delta leaves size unchanged
        assert_eq!(scale_size(2.0, 0.0), 2.0);
    }

    #[test]
    fn test_shrink_excludes_comment_stories() {
        let mut doc = Document::new();
        let comment_p = Paragraph::with_text("Comment text", CharProps { size: Some(8.0), ..Default::default() });
        let comment_id = doc.add_part(PartKind::Comment, vec![para_block(comment_p)]);

        // Shrink with 1.0 pt delta
        fit_document_typography(&mut doc, 1.0, 0.94);

        let comment_part = doc.parts.get(&comment_id).unwrap();
        let p_after = comment_part.blocks[0].as_para().unwrap();
        assert_eq!(p_after.runs[0].props.size, Some(8.0), "comments must be excluded from typography shrink pass");
    }

    #[test]
    fn test_shrink_preserves_comment_inherited_formatting() {
        let mut doc = Document::new();
        // Setup a named style "CommentText" with 12.0 pt font and 6.0 pt spacing after.
        let comment_style = crate::styles::Style {
            id: "CommentText".into(),
            name: "Comment Text".into(),
            kind: crate::styles::StyleKind::Paragraph,
            chr: CharProps { size: Some(12.0), ..Default::default() },
            para: ParaProps { space_after: Some(6.0), line_spacing: Some(LineSpacing::Multiple(1.2)), ..Default::default() },
            ..Default::default()
        };
        Arc::make_mut(&mut doc.styles).upsert(comment_style);

        // Comment paragraph inheriting from "CommentText" without explicit run size or paragraph spacing
        let mut comment_p = Paragraph::with_text("Inherited comment text", CharProps::default());
        comment_p.props.style = Some("CommentText".to_string());
        let comment_id = doc.add_part(PartKind::Comment, vec![para_block(comment_p)]);

        // Shrink with 1.0 pt delta and 0.94 space factor
        fit_document_typography(&mut doc, 1.0, 0.94);

        // Shared style "CommentText" is scaled (12.0 -> 11.0 pt)
        let scaled_style = doc.styles.get("CommentText").unwrap();
        assert_eq!(scaled_style.chr.size, Some(11.0));

        // The comment story must have frozen its original inherited formatting before styles were modified
        let comment_part = doc.parts.get(&comment_id).unwrap();
        let p_after = comment_part.blocks[0].as_para().unwrap();
        assert_eq!(p_after.runs[0].props.size, Some(12.0), "comment run size must remain 12.0 pt from old styles");
        assert_eq!(p_after.props.space_after, Some(6.0), "comment paragraph space_after must remain 6.0 pt from old styles");
        assert_eq!(p_after.props.line_spacing, Some(LineSpacing::Multiple(1.2)), "comment paragraph line_spacing must remain 1.2 from old styles");
    }

    #[test]
    fn test_header_table_text_inherits_size_from_table_style() {
        let mut doc = Document::new();
        // Setup a custom table style with 9.0 pt font
        let st = crate::styles::Style {
            id: "HeaderTableStyle".into(),
            name: "HeaderTableStyle".into(),
            kind: crate::styles::StyleKind::Table,
            chr: CharProps { size: Some(9.0), ..Default::default() },
            table: Some(crate::styles::TableStyleParts::default()),
            ..Default::default()
        };
        Arc::make_mut(&mut doc.styles).upsert(st);

        // Create a table in a Header part that uses HeaderTableStyle
        let mut table = crate::Table::new(1, 1, 100.0);
        table.props.style = Some("HeaderTableStyle".to_string());
        // Cell paragraph with no explicit font size on run (inherits from table style)
        let cell_p = Paragraph::with_text("Header cell text", CharProps::default());
        table.rows[0].cells[0].blocks = vec![crate::para_block(cell_p)];

        let header_id = doc.add_part(PartKind::Header, vec![Arc::new(Block::Table(table))]);

        // Run shrink with delta_pt = 1.0 (body font would shrink)
        fit_document_typography(&mut doc, 1.0, 0.95);

        // Verify that the header table cell's run preserved the 9.0 pt size from the table style,
        // and was not frozen to the document default (11.0 pt)
        let header_part = doc.parts.get(&header_id).unwrap();
        if let Block::Table(tbl) = header_part.blocks[0].as_ref() {
            let p = tbl.rows[0].cells[0].blocks[0].as_para().unwrap();
            assert_eq!(p.runs[0].props.size, Some(9.0), "header table text must inherit and freeze table style font size");
        } else {
            panic!("expected table in header part");
        }
    }

    #[test]
    fn test_scale_line_spacing_never_increases_fixed_or_multiple() {
        // LineSpacing::Exactly(7.45) with space_factor = 0.94:
        // 7.45 * 0.94 = 7.003 -> ceil to half-point would yield 7.5 pt, but capping at 7.45 ensures no enlargement
        assert_eq!(scale_fixed_line_spacing(7.45, 0.94), 7.45);
        // Regular scaling: 10.0 pt with 0.94 -> 9.4 -> ceil to half-point = 9.5 pt <= 10.0 pt
        assert_eq!(scale_fixed_line_spacing(10.0, 0.94), 9.5);

        // LineSpacing::Multiple with small factor never enlarges
        let ls = LineSpacing::Multiple(0.942);
        let scaled = scale_line_spacing(ls, 0.94);
        if let LineSpacing::Multiple(m) = scaled {
            assert!(m <= 0.942);
        } else {
            panic!("expected Multiple");
        }
    }

    #[test]
    fn test_scale_spacing_never_increases_value() {
        // Spacing 0.3 pt scaled by 0.94 yields 0.282 pt, rounded up to 0.29 and never above 0.3 pt.
        assert_eq!(scale_spacing(0.3, 0.94), 0.29);
        // 12.0 pt * 0.94 = 11.28 pt, capped at the requested reduction.
        assert_eq!(scale_spacing(12.0, 0.94), 11.28);
        // Zero spacing remains zero.
        assert_eq!(scale_spacing(0.0, 0.94), 0.0);
    }
}
