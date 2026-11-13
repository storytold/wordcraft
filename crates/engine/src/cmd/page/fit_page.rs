//! Fit One Page command implementation (`layout.fitOnePage`).
//!
//! Automatically condenses a multi-page document by one page using speculative
//! layout evaluation, minimal typographical perturbation, and strict readability
//! safeguards (line spacing floor and font size floor).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;
use wordcraft_doc::section::SectionStart;
use wordcraft_doc::shrink::fit_document_typography;
use wordcraft_doc::{CharProps, Document, ParaProps, StoryRef};
use wordcraft_layout::{HideDeleted, LayoutCache, LayoutOptions, Page, Placed, ViewMode};

use crate::cmd::sel_result;
use crate::{CmdError, CmdResult, Session, p};

/// Minimum comfortable body font size before refusing further reduction (Anti-Compound Guard).
pub const MIN_COMFORTABLE_BODY_FONT_SIZE: f32 = 8.5;

/// Maximum number of fine-grained trial steps allowed when `stepPt` is specified.
pub const MAX_FINE_STEPS: usize = 20;

/// Pre-calculated candidate steps `(delta_font_pt, space_factor)`.
///
/// Evaluated in ascending order of aesthetic perturbation:
/// 1. Tries paragraph spacing adjustment first with zero font size perturbation.
/// 2. Then tests micro font adjustments with minimal spacing changes.
/// 3. Respects a line-spacing floor (>= 0.94) and paragraph spacing floor (>= 0.85)
///    to guarantee that lines and paragraphs never crowd or collide.
const CANDIDATE_STEPS: [(f32, f32); 10] = [
    (0.0, 0.94), // 1. Spacing-only mild (font sizes untouched)
    (0.0, 0.92), // 2. Spacing-only moderate (font sizes untouched)
    (0.0, 0.90), // 3. Spacing-only tighter paragraph spacing (font sizes untouched)
    (0.0, 0.88), // 4. Spacing-only compact (font sizes untouched)
    (0.5, 0.96), // 5. Micro font reduction (-0.5 pt) with near-normal spacing
    (0.5, 0.90), // 6. Micro font reduction (-0.5 pt) with gentle spacing reduction
    (1.0, 0.96), // 7. Moderate font reduction (-1.0 pt) with near-normal spacing
    (1.0, 0.90), // 8. Moderate font reduction (-1.0 pt) with gentle spacing reduction
    (1.0, 0.85), // 9. Moderate font reduction (-1.0 pt) with tighter paragraph spacing
    (2.0, 0.88), // 10. Maximum safe font reduction (-2.0 pt)
];

/// Calculates the minimum effective font size in points across a single paragraph.
///
/// Resolves character formatting using `resolve_char_in` against `table_chr` (when inside a table cell)
/// and inspects both standard and complex-script text properties while ignoring superscript/subscript characters.
/// When `hides` hides a deletion, runs carrying it are skipped because Print layout omits them.
/// When `show_hidden` is false, runs with hidden text formatting are skipped because they are omitted from layout.
fn min_para_font_size(
    doc: &Document,
    p: &wordcraft_doc::para::Paragraph,
    table_chr: Option<&wordcraft_doc::CharProps>,
    hides: HideDeleted,
    show_hidden: bool,
) -> f32 {
    let style_id = p.props.style.as_deref();
    let mut min_size = if p.runs.is_empty() {
        let mark_resolved = doc.styles.resolve_char_in(style_id, table_chr, &p.mark);
        if (mark_resolved.hidden && !show_hidden) || hides.hides(doc, p.mark.del) { f32::INFINITY } else { mark_resolved.size }
    } else {
        f32::INFINITY
    };
    let mut run_start: usize = 0;
    for r in &p.runs {
        let run_end = run_start.saturating_add(r.len).min(p.text.len());
        let run_text = p.text.get(run_start..run_end).unwrap_or("");
        run_start = run_end;
        if hides.hides(doc, r.props.del) {
            continue;
        }

        let resolved = doc.styles.resolve_char_in(style_id, table_chr, &r.props);
        if resolved.hidden && !show_hidden {
            continue;
        }
        if matches!(resolved.vert_align, wordcraft_doc::props::VertAlign::Superscript | wordcraft_doc::props::VertAlign::Subscript) {
            continue;
        }
        let uses_cs = resolved.cs || resolved.rtl || run_text.chars().any(|c| resolved.uses_complex(c));
        if uses_cs {
            min_size = min_size.min(resolved.size_cs);
        }
        let has_standard = run_text.is_empty() || run_text.chars().any(|c| !resolved.uses_complex(c));
        if has_standard {
            min_size = min_size.min(resolved.size);
        }
    }
    min_size
}

/// Recursively calculates the minimum effective font size in points across blocks, resolving table-style conditional formatting.
///
/// When `hides` hides a deletion, deleted text runs are skipped.
/// When `show_hidden` is false, hidden text runs omitted from Print view are skipped.
fn min_blocks_font_size(
    doc: &Document,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_chr: Option<&wordcraft_doc::CharProps>,
    hides: HideDeleted,
    show_hidden: bool,
) -> f32 {
    use wordcraft_doc::styles::PartSet;

    let mut min_sz = f32::INFINITY;
    for b in blocks {
        if let Some(p) = b.as_para() {
            min_sz = min_sz.min(min_para_font_size(doc, p, table_chr, hides, show_hidden));
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let nrows = tbl.rows.len();
            let ncols = tbl.cols();
            for (ri, row) in tbl.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    let mut cell_chr = wordcraft_doc::CharProps::default();
                    if let Some(st) = &style {
                        let sizes = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
                        let g = tbl.grid_col(ri, ci);
                        let span = cell.span();
                        let region = PartSet::of_cell(&tbl.props.look, sizes, (ri, nrows), (g, span, ncols));
                        cell_chr.overlay(&st.chr);
                        for part in region.iter() {
                            let f = st.part(part);
                            cell_chr.overlay(&f.chr);
                        }
                    }
                    min_sz = min_sz.min(min_blocks_font_size(doc, &cell.blocks, Some(&cell_chr), hides, show_hidden));
                }
            }
        }
    }
    min_sz
}

/// True when a paragraph has a page-break character in a run that Print layout draws.
fn paragraph_has_visible_page_break(
    doc: &Document,
    p: &wordcraft_doc::para::Paragraph,
    table_chr: Option<&CharProps>,
    hides: HideDeleted,
    show_hidden: bool,
) -> bool {
    let style_id = p.props.style.as_deref();
    if p.runs.is_empty() {
        let mark = doc.styles.resolve_char_in(style_id, table_chr, &p.mark);
        let visible = !(mark.hidden && !show_hidden) && !hides.hides(doc, p.mark.del);
        return visible && p.text.contains(wordcraft_doc::para::PAGE_BREAK);
    }
    let mut run_start = 0usize;
    for r in &p.runs {
        let run_end = run_start.saturating_add(r.len).min(p.text.len());
        let run_text = p.text.get(run_start..run_end).unwrap_or("");
        run_start = run_end;
        if hides.hides(doc, r.props.del) {
            continue;
        }
        let resolved = doc.styles.resolve_char_in(style_id, table_chr, &r.props);
        if resolved.hidden && !show_hidden {
            continue;
        }
        if run_text.contains(wordcraft_doc::para::PAGE_BREAK) {
            return true;
        }
    }
    false
}

/// Returns true when these blocks contain a visible explicit page break or a resolved
/// `page_break_before`, including inside table cells (and nested tables).
fn blocks_have_explicit_breaks(
    doc: &Document,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_para: Option<&ParaProps>,
    table_chr: Option<&CharProps>,
    hides: HideDeleted,
    show_hidden: bool,
) -> bool {
    for b in blocks {
        if let Some(p) = b.as_para() {
            if doc.styles.resolve_para_in(&p.props, table_para).page_break_before
                || paragraph_has_visible_page_break(doc, p, table_chr, hides, show_hidden)
            {
                return true;
            }
        } else if let Some(tbl) = b.as_table() {
            use wordcraft_doc::styles::PartSet;

            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let cell_para = style.as_ref().map(|st| &st.para);
            let nrows = tbl.rows.len();
            let ncols = tbl.cols();
            for (ri, row) in tbl.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    let mut cell_chr = CharProps::default();
                    if let Some(st) = &style {
                        let sizes = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
                        let region = PartSet::of_cell(&tbl.props.look, sizes, (ri, nrows), (tbl.grid_col(ri, ci), cell.span(), ncols));
                        cell_chr.overlay(&st.chr);
                        for part in region.iter() {
                            cell_chr.overlay(&st.part(part).chr);
                        }
                    }
                    if blocks_have_explicit_breaks(doc, &cell.blocks, cell_para, Some(&cell_chr), hides, show_hidden) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Returns true if the document contains explicit page or section breaks that
/// physically force content onto additional pages.
fn has_explicit_breaks(doc: &Document, hides: HideDeleted, show_hidden: bool) -> bool {
    let sections = doc.sections();
    if sections.len() > 1 {
        for (_, sec) in &sections[1..] {
            if matches!(sec.start, SectionStart::NextPage | SectionStart::EvenPage | SectionStart::OddPage) {
                return true;
            }
        }
    }
    blocks_have_explicit_breaks(doc, &doc.body, None, None, hides, show_hidden)
}

/// True when `item` is a line-number decoration: a `DECOR_STORY` line with no block path.
///
/// Note separators and continuation notices also use `DECOR_STORY`, but carry their block path.
/// Line numbers sit outside the body column and must not count as page content.
fn is_line_number_decoration(item: &Placed) -> bool {
    matches!(item, Placed::Lines { story, path, .. } if *story == wordcraft_layout::DECOR_STORY && path.0.is_empty())
}

/// True when `story` is a footnote part, whose text sits at the bottom of its page.
fn is_footnote_story(doc: &Document, story: &wordcraft_doc::StoryRef) -> bool {
    match story {
        wordcraft_doc::StoryRef::Part(id) => doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::Footnote),
        _ => false,
    }
}

/// The height covered by the union of `spans`, so overlapping items (side-by-side table cells)
/// count once.
fn union_height(spans: &[(f32, f32)]) -> f32 {
    let mut spans: Vec<(f32, f32)> = spans.iter().copied().filter(|(a, b)| a.is_finite() && b.is_finite() && *b > *a).collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut total = 0.0;
    let mut iter = spans.into_iter();
    if let Some((mut start, mut end)) = iter.next() {
        for (a, b) in iter {
            if a <= end + 0.01 {
                end = end.max(b);
            } else {
                total += end - start;
                start = a;
                end = b;
            }
        }
        total += end - start;
    }
    total
}

/// The story an item belongs to, when it carries one.
fn item_story(item: &Placed) -> Option<&StoryRef> {
    match item {
        Placed::Lines { story, .. }
        | Placed::Cell { story, .. }
        | Placed::Image { story, .. }
        | Placed::Graphic { story, .. }
        | Placed::Object { story, .. } => Some(story),
        Placed::Shape { object, .. } => object.as_ref().map(|pos| &pos.story),
        Placed::Fill { .. } | Placed::Rule { .. } | Placed::Clip { .. } => None,
    }
}

/// Body line count and occupied body height on the final page, for the pre-flight volume limit.
///
/// Line-number decorations sit outside the body column and note content sits at the page's foot, so
/// neither counts as body spill. The remaining items contribute the union of their vertical spans:
/// a tall image or an image-only table cell counts at its real height, and side-by-side cells
/// count once.
fn final_page_volume(doc: &Document, page: &Page) -> (usize, f32) {
    let mut last_page_lines = 0usize;
    let mut body_spans: Vec<(f32, f32)> = Vec::new();
    let mut note_spans: Vec<(f32, f32)> = Vec::new();
    for item in &page.items {
        if is_line_number_decoration(item) {
            continue;
        }
        let is_note_separator = matches!(item, Placed::Lines { story, .. } if *story == wordcraft_layout::DECOR_STORY);
        let is_note = item_story(item).is_some_and(|story| is_footnote_story(doc, story));
        if is_note || is_note_separator {
            note_spans.push(item.vertical_span());
            continue;
        }
        if let Placed::Lines { l0, l1, .. } = item {
            last_page_lines = last_page_lines.saturating_add(l1.saturating_sub(*l0));
        }
        if !matches!(item, Placed::Fill { .. } | Placed::Rule { .. } | Placed::Clip { .. }) {
            body_spans.push(item.vertical_span());
        }
    }
    let body_spill_h = union_height(&body_spans);
    let note_h = union_height(&note_spans);
    let body_area_h = (page.body.h - note_h).max(0.0);
    let fill_ratio = if body_area_h > 0.0 {
        body_spill_h / body_area_h
    } else if body_spill_h > 0.0 {
        1.0
    } else {
        0.0
    };
    (last_page_lines, fill_ratio)
}

/// Calculates the minimum effective body font size in points across the document body and editable text boxes.
///
/// Uses the same deletion visibility as the session's Print layout for each story.
fn min_body_font_size(doc: &Document, opts: &LayoutOptions) -> f32 {
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    let mut min_sz = min_blocks_font_size(doc, &doc.body, None, opts.hides_deleted(StoryRef::Body), opts.show_hidden);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            min_sz = min_sz.min(min_blocks_font_size(doc, &part.blocks, None, opts.hides_deleted(StoryRef::Part(*id)), opts.show_hidden));
        }
    }
    if min_sz.is_finite() { min_sz } else { wordcraft_doc::props::DEFAULT_FONT_SIZE }
}

/// Checks whether effective body font sizes are at or below the minimum readable font size (8.5 pt).
fn doc_has_unreadable_body_font(doc: &Document, opts: &LayoutOptions) -> bool {
    min_body_font_size(doc, opts) < MIN_COMFORTABLE_BODY_FONT_SIZE
}

/// Collects all effective font sizes in points across a slice of blocks.
fn collect_blocks_font_sizes(doc: &Document, blocks: &[Arc<wordcraft_doc::Block>], table_chr: Option<&wordcraft_doc::CharProps>, out: &mut Vec<f32>) {
    use wordcraft_doc::styles::PartSet;

    for b in blocks {
        if let Some(p) = b.as_para() {
            let style_id = p.props.style.as_deref();
            if p.runs.is_empty() {
                let mark_resolved = doc.styles.resolve_char_in(style_id, table_chr, &p.mark);
                out.push(mark_resolved.size);
                out.push(mark_resolved.size_cs);
            }
            for r in &p.runs {
                let resolved = doc.styles.resolve_char_in(style_id, table_chr, &r.props);
                if !matches!(resolved.vert_align, wordcraft_doc::props::VertAlign::Superscript | wordcraft_doc::props::VertAlign::Subscript) {
                    out.push(resolved.size);
                    out.push(resolved.size_cs);
                }
            }
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let nrows = tbl.rows.len();
            let ncols = tbl.cols();
            for (ri, row) in tbl.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    let mut cell_chr = wordcraft_doc::CharProps::default();
                    if let Some(st) = &style {
                        let sizes = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
                        let g = tbl.grid_col(ri, ci);
                        let span = cell.span();
                        let region = PartSet::of_cell(&tbl.props.look, sizes, (ri, nrows), (g, span, ncols));
                        cell_chr.overlay(&st.chr);
                        for part in region.iter() {
                            let f = st.part(part);
                            cell_chr.overlay(&f.chr);
                        }
                    }
                    collect_blocks_font_sizes(doc, &cell.blocks, Some(&cell_chr), out);
                }
            }
        }
    }
}

/// Collects every effective body font size across the document body and editable parts, with duplicates.
fn collect_body_font_sizes(doc: &Document) -> Vec<f32> {
    let mut sizes = Vec::new();
    collect_blocks_font_sizes(doc, &doc.body, None, &mut sizes);
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            collect_blocks_font_sizes(doc, &part.blocks, None, &mut sizes);
        }
    }
    sizes
}

/// Collects all effective line spacings across a slice of blocks while preserving table-style context.
fn collect_blocks_line_spacings(
    doc: &Document,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_para: Option<&wordcraft_doc::ParaProps>,
    out: &mut Vec<wordcraft_doc::props::LineSpacing>,
) {
    for b in blocks {
        if let Some(p) = b.as_para() {
            let resolved = doc.styles.resolve_para_in(&p.props, table_para);
            out.push(resolved.line_spacing);
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let cell_para = style.as_ref().map(|st| &st.para);
            for row in &tbl.rows {
                for cell in &row.cells {
                    collect_blocks_line_spacings(doc, &cell.blocks, cell_para, out);
                }
            }
        }
    }
}

/// Collects every effective line spacing across body paragraphs and editable parts, with duplicates.
fn collect_body_line_spacings(doc: &Document) -> Vec<wordcraft_doc::props::LineSpacing> {
    let mut spacings = Vec::new();
    collect_blocks_line_spacings(doc, &doc.body, None, &mut spacings);
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            collect_blocks_line_spacings(doc, &part.blocks, None, &mut spacings);
        }
    }
    spacings
}

/// Collects all effective paragraph spacing values across a slice of blocks, preserving table context.
fn collect_blocks_paragraph_spacings(
    doc: &Document,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_para: Option<&wordcraft_doc::ParaProps>,
    out: &mut Vec<f32>,
) {
    for b in blocks {
        if let Some(p) = b.as_para() {
            let resolved = doc.styles.resolve_para_in(&p.props, table_para);
            out.push(resolved.space_before);
            out.push(resolved.space_after);
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let cell_para = style.as_ref().map(|st| &st.para);
            for row in &tbl.rows {
                for cell in &row.cells {
                    collect_blocks_paragraph_spacings(doc, &cell.blocks, cell_para, out);
                }
            }
        }
    }
}

/// Collects every effective paragraph spacing value across body paragraphs and editable parts.
fn collect_body_paragraph_spacings(doc: &Document) -> Vec<f32> {
    let mut spacings = Vec::new();
    collect_blocks_paragraph_spacings(doc, &doc.body, None, &mut spacings);
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            collect_blocks_paragraph_spacings(doc, &part.blocks, None, &mut spacings);
        }
    }
    spacings
}

/// Recursively collects effective font sizes by formatting location key across a slice of blocks.
fn collect_blocks_font_locations(
    doc: &Document,
    prefix: &str,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_chr: Option<&wordcraft_doc::CharProps>,
    out: &mut BTreeMap<String, f32>,
) {
    use wordcraft_doc::styles::PartSet;

    for (bi, b) in blocks.iter().enumerate() {
        let block_prefix = format!("{prefix}/{bi}");
        if let Some(p) = b.as_para() {
            let style_id = p.props.style.as_deref();
            if p.runs.is_empty() {
                let mark_resolved = doc.styles.resolve_char_in(style_id, table_chr, &p.mark);
                out.insert(format!("{block_prefix}/mark/size"), mark_resolved.size);
                out.insert(format!("{block_prefix}/mark/size_cs"), mark_resolved.size_cs);
            }
            for (ri, r) in p.runs.iter().enumerate() {
                let resolved = doc.styles.resolve_char_in(style_id, table_chr, &r.props);
                if !matches!(resolved.vert_align, wordcraft_doc::props::VertAlign::Superscript | wordcraft_doc::props::VertAlign::Subscript) {
                    out.insert(format!("{block_prefix}/r{ri}/size"), resolved.size);
                    out.insert(format!("{block_prefix}/r{ri}/size_cs"), resolved.size_cs);
                }
            }
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let nrows = tbl.rows.len();
            let ncols = tbl.cols();
            for (ri, row) in tbl.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    let mut cell_chr = wordcraft_doc::CharProps::default();
                    if let Some(st) = &style {
                        let sizes = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
                        let g = tbl.grid_col(ri, ci);
                        let span = cell.span();
                        let region = PartSet::of_cell(&tbl.props.look, sizes, (ri, nrows), (g, span, ncols));
                        cell_chr.overlay(&st.chr);
                        for part in region.iter() {
                            let f = st.part(part);
                            cell_chr.overlay(&f.chr);
                        }
                    }
                    let cell_prefix = format!("{block_prefix}/tbl/r{ri}/c{ci}");
                    collect_blocks_font_locations(doc, &cell_prefix, &cell.blocks, Some(&cell_chr), out);
                }
            }
        }
    }
}

/// Collects all effective font sizes by formatting location key across body paragraphs and editable parts.
fn collect_body_font_locations(doc: &Document) -> BTreeMap<String, f32> {
    let mut out = BTreeMap::new();
    collect_blocks_font_locations(doc, "body", &doc.body, None, &mut out);
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            let prefix = format!("part:{id}");
            collect_blocks_font_locations(doc, &prefix, &part.blocks, None, &mut out);
        }
    }
    out
}

/// Recursively collects effective line spacing by formatting location key across a slice of blocks.
fn collect_blocks_line_spacing_locations(
    doc: &Document,
    prefix: &str,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_para: Option<&wordcraft_doc::ParaProps>,
    out: &mut BTreeMap<String, wordcraft_doc::props::LineSpacing>,
) {
    for (bi, b) in blocks.iter().enumerate() {
        let block_prefix = format!("{prefix}/{bi}");
        if let Some(p) = b.as_para() {
            let resolved = doc.styles.resolve_para_in(&p.props, table_para);
            out.insert(format!("{block_prefix}/line_spacing"), resolved.line_spacing);
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let cell_para = style.as_ref().map(|st| &st.para);
            for (ri, row) in tbl.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    let cell_prefix = format!("{block_prefix}/tbl/r{ri}/c{ci}");
                    collect_blocks_line_spacing_locations(doc, &cell_prefix, &cell.blocks, cell_para, out);
                }
            }
        }
    }
}

/// Collects all effective line spacing values by formatting location key across body paragraphs and editable parts.
fn collect_body_line_spacing_locations(doc: &Document) -> BTreeMap<String, wordcraft_doc::props::LineSpacing> {
    let mut out = BTreeMap::new();
    collect_blocks_line_spacing_locations(doc, "body", &doc.body, None, &mut out);
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            let prefix = format!("part:{id}");
            collect_blocks_line_spacing_locations(doc, &prefix, &part.blocks, None, &mut out);
        }
    }
    out
}

/// Recursively collects effective paragraph spacing by formatting location key across a slice of blocks.
fn collect_blocks_paragraph_spacing_locations(
    doc: &Document,
    prefix: &str,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_para: Option<&wordcraft_doc::ParaProps>,
    out: &mut BTreeMap<String, f32>,
) {
    for (bi, b) in blocks.iter().enumerate() {
        let block_prefix = format!("{prefix}/{bi}");
        if let Some(p) = b.as_para() {
            let resolved = doc.styles.resolve_para_in(&p.props, table_para);
            out.insert(format!("{block_prefix}/space_before"), resolved.space_before);
            out.insert(format!("{block_prefix}/space_after"), resolved.space_after);
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let cell_para = style.as_ref().map(|st| &st.para);
            for (ri, row) in tbl.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    let cell_prefix = format!("{block_prefix}/tbl/r{ri}/c{ci}");
                    collect_blocks_paragraph_spacing_locations(doc, &cell_prefix, &cell.blocks, cell_para, out);
                }
            }
        }
    }
}

/// Collects all effective paragraph spacing values by formatting location key across body paragraphs and editable parts.
fn collect_body_paragraph_spacing_locations(doc: &Document) -> BTreeMap<String, f32> {
    let mut out = BTreeMap::new();
    collect_blocks_paragraph_spacing_locations(doc, "body", &doc.body, None, &mut out);
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            let prefix = format!("part:{id}");
            collect_blocks_paragraph_spacing_locations(doc, &prefix, &part.blocks, None, &mut out);
        }
    }
    out
}

/// Checks whether previously reduced text from an earlier fit is still present in the document.
///
/// Compares each formatting location against its recorded original value before the first fit,
/// within the cumulative reduction that fit applied.
#[cfg(test)]
fn has_previously_reduced_text(doc: &Document, fit: &wordcraft_doc::shrink::FitState) -> bool {
    (fit.cum_delta > 0.0 && font_reduction_present(doc, fit)) || (fit.cum_space < 1.0 - 1e-4 && spacing_reduction_present(doc, fit))
}

/// Checks whether any font reduction from an earlier fit remains active in `doc`.
///
/// Compares each formatting location against its recorded original value before the fit.
/// Treats locations without a recorded key as unknown and preserves the active budget when
/// counts or keys do not match; clears it only when matching locations establish that
/// reductions are fully restored.
fn font_reduction_present(doc: &Document, fit: &wordcraft_doc::shrink::FitState) -> bool {
    if fit.original_font_locations.is_empty() {
        if fit.original_font_sizes.is_empty() {
            return false;
        }
        let current_sizes = collect_body_font_sizes(doc);
        return current_sizes.iter().any(|&cur| {
            fit.original_font_sizes.iter().any(|&orig| {
                let floor = orig - fit.cum_delta - 1e-4;
                cur >= floor && cur < orig - 1e-4
            })
        });
    }

    let current = collect_body_font_locations(doc);
    let mut matching_count = 0;
    for (key, &orig) in &fit.original_font_locations {
        if let Some(&cur) = current.get(key) {
            matching_count += 1;
            let floor = orig - fit.cum_delta - 1e-4;
            if cur >= floor && cur < orig - 1e-4 {
                return true;
            }
        }
    }

    if matching_count != fit.original_font_locations.len() || current.len() != fit.original_font_locations.len() {
        return true;
    }

    let all_restored = fit.original_font_locations.iter().all(|(key, &orig)| current.get(key).map(|&cur| cur >= orig - 1e-4).unwrap_or(false));

    !all_restored
}

/// Checks whether any line spacing reduction from an earlier fit remains active in `doc`.
///
/// Compares each formatting location against its recorded original line spacing before the fit.
/// Treats locations without a recorded key as unknown and preserves the active budget when
/// counts or keys do not match; clears it only when matching locations establish that
/// reductions are fully restored.
fn line_spacing_reduction_present(doc: &Document, fit: &wordcraft_doc::shrink::FitState) -> bool {
    if fit.original_line_spacing_locations.is_empty() {
        if fit.original_line_spacings.is_empty() {
            return false;
        }
        let current_spacings = collect_body_line_spacings(doc);
        return current_spacings.iter().any(|cur| {
            fit.original_line_spacings.iter().any(|orig| match (cur, orig) {
                (wordcraft_doc::props::LineSpacing::Multiple(c), wordcraft_doc::props::LineSpacing::Multiple(o))
                | (wordcraft_doc::props::LineSpacing::AtLeast(c), wordcraft_doc::props::LineSpacing::AtLeast(o))
                | (wordcraft_doc::props::LineSpacing::Exactly(c), wordcraft_doc::props::LineSpacing::Exactly(o)) => {
                    let floor = o * fit.cum_space - 1e-4;
                    *c >= floor && *c < *o - 1e-4
                }
                _ => false,
            })
        });
    }

    let current = collect_body_line_spacing_locations(doc);
    let mut matching_count = 0;
    for (key, orig) in &fit.original_line_spacing_locations {
        if let Some(cur) = current.get(key) {
            matching_count += 1;
            let is_reduced = match (cur, orig) {
                (wordcraft_doc::props::LineSpacing::Multiple(c), wordcraft_doc::props::LineSpacing::Multiple(o))
                | (wordcraft_doc::props::LineSpacing::AtLeast(c), wordcraft_doc::props::LineSpacing::AtLeast(o))
                | (wordcraft_doc::props::LineSpacing::Exactly(c), wordcraft_doc::props::LineSpacing::Exactly(o)) => {
                    let floor = o * fit.cum_space - 1e-4;
                    *c >= floor && *c < *o - 1e-4
                }
                _ => false,
            };
            if is_reduced {
                return true;
            }
        }
    }

    if matching_count != fit.original_line_spacing_locations.len() || current.len() != fit.original_line_spacing_locations.len() {
        return true;
    }

    let all_restored = fit.original_line_spacing_locations.iter().all(|(key, orig)| {
        current
            .get(key)
            .map(|cur| match (cur, orig) {
                (wordcraft_doc::props::LineSpacing::Multiple(c), wordcraft_doc::props::LineSpacing::Multiple(o))
                | (wordcraft_doc::props::LineSpacing::AtLeast(c), wordcraft_doc::props::LineSpacing::AtLeast(o))
                | (wordcraft_doc::props::LineSpacing::Exactly(c), wordcraft_doc::props::LineSpacing::Exactly(o)) => *c >= *o - 1e-4,
                _ => false,
            })
            .unwrap_or(false)
    });

    !all_restored
}

/// Checks whether any paragraph spacing reduction from an earlier fit remains active in `doc`.
///
/// Compares each formatting location against its recorded original paragraph spacing before the fit.
/// Treats locations without a recorded key as unknown and preserves the active budget when
/// counts or keys do not match; clears it only when matching locations establish that
/// reductions are fully restored.
fn paragraph_spacing_reduction_present(doc: &Document, fit: &wordcraft_doc::shrink::FitState) -> bool {
    if fit.original_paragraph_spacing_locations.is_empty() {
        if fit.original_paragraph_spacings.is_empty() {
            return false;
        }
        let current_spacings = collect_body_paragraph_spacings(doc);
        return current_spacings.iter().any(|&cur| {
            fit.original_paragraph_spacings.iter().any(|&orig| {
                let floor = orig * fit.cum_space - 1e-4;
                cur >= floor && cur < orig - 1e-4
            })
        });
    }

    let current = collect_body_paragraph_spacing_locations(doc);
    let mut matching_count = 0;
    for (key, &orig) in &fit.original_paragraph_spacing_locations {
        if let Some(&cur) = current.get(key) {
            matching_count += 1;
            let floor = orig * fit.cum_space - 1e-4;
            if cur >= floor && cur < orig - 1e-4 {
                return true;
            }
        }
    }

    if matching_count != fit.original_paragraph_spacing_locations.len() || current.len() != fit.original_paragraph_spacing_locations.len() {
        return true;
    }

    let all_restored =
        fit.original_paragraph_spacing_locations.iter().all(|(key, &orig)| current.get(key).map(|&cur| cur >= orig - 1e-4).unwrap_or(false));

    !all_restored
}

/// Checks whether line or paragraph spacing reductions from an earlier fit remain active in `doc`.
fn spacing_reduction_present(doc: &Document, fit: &wordcraft_doc::shrink::FitState) -> bool {
    line_spacing_reduction_present(doc, fit) || paragraph_spacing_reduction_present(doc, fit)
}

/// The reduction budget still active in `doc`, with font and spacing resolved independently.
#[derive(Clone, Copy, Debug)]
struct ActiveBudget {
    delta: f32,
    space: f32,
    font_active: bool,
    spacing_active: bool,
}

/// Resolves the active cumulative budget for font and spacing components in `doc`.
fn active_budget(doc: &Document) -> ActiveBudget {
    let Some(fit) = doc.fit_state.as_ref() else {
        return ActiveBudget { delta: 0.0, space: 1.0, font_active: false, spacing_active: false };
    };
    let font_active = fit.cum_delta > 0.0 && font_reduction_present(doc, fit);
    let spacing_active = fit.cum_space < 1.0 - 1e-4 && spacing_reduction_present(doc, fit);
    ActiveBudget {
        delta: if font_active { fit.cum_delta } else { 0.0 },
        space: if spacing_active { fit.cum_space } else { 1.0 },
        font_active,
        spacing_active,
    }
}

/// The original typography baseline recorded before the first fit.
#[derive(Clone, Debug, Default)]
struct OriginalTypography {
    font_sizes: Vec<f32>,
    line_spacings: Vec<wordcraft_doc::props::LineSpacing>,
    paragraph_spacings: Vec<f32>,
    font_locations: BTreeMap<String, f32>,
    line_spacing_locations: BTreeMap<String, wordcraft_doc::props::LineSpacing>,
    paragraph_spacing_locations: BTreeMap<String, f32>,
}

impl OriginalTypography {
    /// Collects the typography baseline from the body and editable parts of `doc`.
    fn from_doc(doc: &Document) -> Self {
        Self {
            font_sizes: collect_body_font_sizes(doc),
            line_spacings: collect_body_line_spacings(doc),
            paragraph_spacings: collect_body_paragraph_spacings(doc),
            font_locations: collect_body_font_locations(doc),
            line_spacing_locations: collect_body_line_spacing_locations(doc),
            paragraph_spacing_locations: collect_body_paragraph_spacing_locations(doc),
        }
    }
}

/// Records the cumulative Fit One Page reduction state into the document's internal fit tracking.
fn set_cumulative_shrink_state(doc: &mut Document, delta: f32, space: f32, orig: OriginalTypography) {
    let sizes = collect_body_font_sizes(doc);
    let spacings = collect_body_line_spacings(doc);
    let paragraph_spacings = collect_body_paragraph_spacings(doc);
    doc.fit_state = Some(wordcraft_doc::FitState {
        cum_delta: delta,
        cum_space: space,
        reduced_font_sizes: sizes,
        reduced_line_spacings: spacings,
        reduced_paragraph_spacings: paragraph_spacings,
        original_font_sizes: orig.font_sizes,
        original_line_spacings: orig.line_spacings,
        original_paragraph_spacings: orig.paragraph_spacings,
        original_font_locations: orig.font_locations,
        original_line_spacing_locations: orig.line_spacing_locations,
        original_paragraph_spacing_locations: orig.paragraph_spacing_locations,
    });
}

/// Generates candidate font-reduction delta and line-spacing alpha pairs respecting parameter bounds.
fn generate_candidate_steps(max_delta: f32, step_pt_opt: Option<f32>, adjust_spacing: bool) -> Vec<(f32, f32)> {
    let mut candidates: Vec<(f32, f32)> = Vec::new();
    if let Some(step_pt) = step_pt_opt {
        if step_pt > 0.0 && step_pt.is_finite() {
            if adjust_spacing {
                candidates.push((0.0, 0.94));
            }
            let count = if max_delta <= 0.0 { 0 } else { ((max_delta / step_pt).floor()).min(MAX_FINE_STEPS as f32).max(0.0) as usize };
            let mut last_d = 0.0f32;
            for i in 1..=count {
                let d = i as f32 * step_pt;
                if d > max_delta {
                    break;
                }
                if d <= 0.0 || (d - last_d).abs() < 1e-4 {
                    continue;
                }
                last_d = d;
                if adjust_spacing {
                    candidates.push((d, 0.95));
                    candidates.push((d, 0.94));
                } else {
                    candidates.push((d, 1.0));
                }
            }
            let mut coarse_d = ((last_d / 0.5).floor() + 1.0) * 0.5;
            while coarse_d <= max_delta {
                if adjust_spacing {
                    candidates.push((coarse_d, 0.95));
                    candidates.push((coarse_d, 0.94));
                } else {
                    candidates.push((coarse_d, 1.0));
                }
                coarse_d += 0.5;
            }
        }
    } else {
        for &(delta, alpha) in &CANDIDATE_STEPS {
            if delta <= max_delta {
                let alpha = if adjust_spacing { alpha } else { 1.0 };
                if delta > 0.0 || (alpha - 1.0).abs() >= f32::EPSILON {
                    candidates.push((delta, alpha));
                }
            }
        }
        // If maxDeltaPt was explicitly configured above 2.0 pt, test higher reductions up to max_delta
        if max_delta > 2.0 {
            let mut extra_delta = 2.5;
            while extra_delta <= max_delta {
                if adjust_spacing {
                    candidates.push((extra_delta, 0.95));
                    candidates.push((extra_delta, 0.94));
                } else {
                    candidates.push((extra_delta, 1.0));
                }
                extra_delta += 0.5;
            }
        }
    }
    candidates
}

/// The Print options Fit One Page uses: the session's pagination-affecting visibility and numeral
/// settings, with proofing disabled because it does not change pagination.
fn fit_layout_options(s: &Session) -> LayoutOptions {
    let (fn_name, fn_path) = s.resolved_filenames();
    LayoutOptions {
        view: ViewMode::Print,
        field_codes: s.view.field_codes,
        show_hidden: s.view.marks || s.prefs.marks.hidden,
        hide_deleted: !s.view.show_markup,
        hide_marked: s.prefs.markup.hidden_deletions(),
        deletions_in_balloons: s.revisions_in_balloons(),
        proofing: false,
        proof: wordcraft_layout::ProofOptions { grammar: false, spelling: Default::default() },
        numeral: s.view.numeral.resolve(crate::locale::system_locale()),
        filename: Some(Arc::from(fn_name.as_str())),
        filepath: Some(Arc::from(fn_path.as_str())),
        ..LayoutOptions::default()
    }
}

/// Executes the `layout.fitOnePage` command.
///
/// Condenses the document by one page if feasible, preserving typographical
/// hierarchies, protecting line spacing, and clamping to readability bounds.
/// Refuses while Track Changes and Track Formatting are on, because the style
/// reductions it needs cannot be recorded as formatting revisions.
pub fn fit_one_page(s: &mut Session, v: &Value) -> CmdResult {
    // 0. Parse parameters safely
    let max_delta = p::f32(v, "maxDeltaPt").unwrap_or(2.0).clamp(0.0, 10.0);
    let step_pt_opt = p::f32(v, "stepPt");
    if let Some(step) = step_pt_opt
        && !(step > 0.0 && step.is_finite())
    {
        return Err(CmdError::Params("`stepPt` must be a positive number of points".into()));
    }
    let adjust_spacing = p::bool(v, "adjustSpacing").unwrap_or(true);

    // 1. Refuse while formatting changes must be tracked: fitting rescales styles, and the
    //    formatting-recording path compares direct run/paragraph properties only, so the
    //    inherited reductions could not be represented as revisions.
    if s.doc.settings.track_changes && s.prefs.markup.track_formatting {
        return Err(CmdError::Disabled(
            "the document tracks formatting changes: Fit One Page cannot record its style changes as formatting revisions".into(),
        ));
    }

    // 2. Resolve the cumulative budget. Each component has its own, so a restored font or
    //    spacing budget is not blocked by the other component's remaining reduction.
    let prior_fit = s.doc.fit_state.clone();
    let budget = active_budget(&s.doc);
    let (cum_delta, cum_space) = (budget.delta, budget.space);
    let (font_active, spacing_active) = (budget.font_active, budget.spacing_active);
    let font_exhausted = cum_delta >= 1.0 - 1e-4;
    let spacing_exhausted = cum_space <= 0.94 + 1e-4;

    // 3. Build equivalent Print layout options
    let print_opts = fit_layout_options(s);
    let shown_hidden = print_opts.show_hidden;
    let pre_typography = OriginalTypography::from_doc(&s.doc);

    // 4. Initial page count evaluation
    let mut initial_cache = LayoutCache::new();
    let initial_layout = wordcraft_layout::layout(&s.doc, &mut initial_cache, &print_opts);
    let p0 = initial_layout.pages.len();

    if p0 <= 1 {
        return Err(CmdError::Failed("The document already fits on one page.".into()));
    }
    if font_exhausted && spacing_exhausted {
        return Err(CmdError::Failed("Unable to shrink document further without reducing text beyond readability limits.".into()));
    }

    // Explicit breaks force pages regardless of typography; refuse before any trial can appear to
    // succeed by moving text around them.
    if has_explicit_breaks(&s.doc, print_opts.hides_deleted(StoryRef::Body), shown_hidden) {
        return Err(CmdError::Failed("Unable to fit document to one page: explicit page or section breaks prevent page reduction.".into()));
    }

    // 5. Pre-flight check on final page content volume
    let last_page = &initial_layout.pages[p0 - 1];
    let (last_page_lines, fill_ratio) = final_page_volume(&s.doc, last_page);
    if last_page_lines > 12 || fill_ratio > 0.30 {
        return Err(CmdError::Failed(
            "Unable to fit document to one page: content on the final page exceeds limits for aesthetic page reduction.".into(),
        ));
    }
    let target_pages = p0.saturating_sub(1);

    // 6. Generate candidate steps: either bounded fine-grained stepPt or pre-calculated steps
    let candidates = generate_candidate_steps(max_delta, step_pt_opt, adjust_spacing);

    // 7. Speculative search loop using ephemeral cache and equivalent print options
    let mut successful_doc = None;
    let mut applied_step = None;
    let mut final_pages = target_pages;

    for (delta, alpha) in candidates {
        // Enforce strict upper bound on delta
        if delta > max_delta {
            continue;
        }
        // A component whose budget is exhausted is skipped, leaving the other component usable.
        if font_exhausted && delta > 0.0 {
            continue;
        }
        if spacing_exhausted && alpha < 1.0 - 1e-4 && delta <= 0.0 {
            continue;
        }
        // Anti-compound budget across runs: cap cumulative font reduction at 1.0 pt
        if cum_delta > 0.0 && cum_delta + delta > 1.0 + 1e-4 {
            continue;
        }
        // Anti-compound budget across runs: cap cumulative line spacing compression at 0.94 floor.
        // If spacing has no room left, a font candidate still runs without spacing compression.
        let mut alpha = if spacing_exhausted { 1.0 } else { alpha };
        if alpha < 1.0 - 1e-4 && cum_space < 1.0 - 1e-4 {
            let cand_line_space = alpha.max(0.94);
            let prior_line_space = cum_space.max(0.94);
            if prior_line_space * cand_line_space < 0.94 - 1e-4 {
                if delta > 0.0 {
                    alpha = 1.0;
                } else {
                    continue;
                }
            }
        }
        if delta <= 0.0 && alpha >= 1.0 - 1e-4 {
            continue;
        }
        // Anti-compound guard: refuse font reduction if body font is already at or below the readability floor,
        // while allowing spacing-only candidates (delta == 0.0) to run.
        if delta > 0.0 && doc_has_unreadable_body_font(&s.doc, &print_opts) {
            continue;
        }

        let mut trial_doc = s.doc.clone();
        fit_document_typography(&mut trial_doc, delta, alpha);

        // Validate the trial document's effective body font sizes against 8.5 pt minimum before accepting
        if delta > 0.0 && doc_has_unreadable_body_font(&trial_doc, &print_opts) {
            continue;
        }

        let mut trial_cache = LayoutCache::new();
        let trial_layout = wordcraft_layout::layout(&trial_doc, &mut trial_cache, &print_opts);

        if trial_layout.pages.len() <= target_pages {
            final_pages = trial_layout.pages.len();
            successful_doc = Some(trial_doc);
            applied_step = Some((delta, alpha));
            break;
        }
    }

    // 8. Success commit or clean rollback
    match (successful_doc, applied_step) {
        (Some(mut new_doc), Some((d, a))) => {
            let prior_original_fonts =
                || prior_fit.as_ref().filter(|_| font_active).map(|fit| fit.original_font_sizes.clone()).filter(|sizes| !sizes.is_empty());
            let prior_original_lines = || {
                prior_fit.as_ref().filter(|_| spacing_active).map(|fit| fit.original_line_spacings.clone()).filter(|spacings| !spacings.is_empty())
            };
            let prior_original_paragraphs = || {
                prior_fit
                    .as_ref()
                    .filter(|_| spacing_active)
                    .map(|fit| fit.original_paragraph_spacings.clone())
                    .filter(|spacings| !spacings.is_empty())
            };
            let prior_original_font_locs =
                || prior_fit.as_ref().filter(|_| font_active).map(|fit| fit.original_font_locations.clone()).filter(|locs| !locs.is_empty());
            let prior_original_line_locs = || {
                prior_fit.as_ref().filter(|_| spacing_active).map(|fit| fit.original_line_spacing_locations.clone()).filter(|locs| !locs.is_empty())
            };
            let prior_original_para_locs = || {
                prior_fit
                    .as_ref()
                    .filter(|_| spacing_active)
                    .map(|fit| fit.original_paragraph_spacing_locations.clone())
                    .filter(|locs| !locs.is_empty())
            };
            let original_typography = OriginalTypography {
                font_sizes: prior_original_fonts().unwrap_or(pre_typography.font_sizes),
                line_spacings: prior_original_lines().unwrap_or(pre_typography.line_spacings),
                paragraph_spacings: prior_original_paragraphs().unwrap_or(pre_typography.paragraph_spacings),
                font_locations: prior_original_font_locs().unwrap_or(pre_typography.font_locations),
                line_spacing_locations: prior_original_line_locs().unwrap_or(pre_typography.line_spacing_locations),
                paragraph_spacing_locations: prior_original_para_locs().unwrap_or(pre_typography.paragraph_spacing_locations),
            };
            set_cumulative_shrink_state(&mut new_doc, cum_delta + d, cum_space * a, original_typography);
            s.doc = new_doc;
            s.clamp_selection();
            let page_word = if final_pages == 1 { "page" } else { "pages" };
            s.status = if d > 0.0 {
                format!("Document fitted to {} {} (font: -{:.1} pt, spacing: {:.0}%).", final_pages, page_word, d, a * 100.0)
            } else {
                format!("Document fitted to {} {} (spacing: {:.0}%).", final_pages, page_word, a * 100.0)
            };
            sel_result(s)
        }
        _ => {
            if min_body_font_size(&s.doc, &print_opts) <= MIN_COMFORTABLE_BODY_FONT_SIZE {
                Err(CmdError::Failed("Unable to shrink document further without reducing text beyond readability limits.".into()))
            } else {
                Err(CmdError::Failed("Unable to fit document to one page: no permitted typography adjustment could remove a page.".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_steps_bounded_by_max_fine_steps() {
        // step_pt is 0.001 (above the 1e-4 deduplication threshold).
        // Ratio 2.0 / 0.001 = 2000 requests 2000 steps, which must be clamped by MAX_FINE_STEPS (20).
        let candidates = generate_candidate_steps(2.0, Some(0.001), false);
        let fine_steps_count = candidates.iter().filter(|(d, _)| *d < 0.5).count();
        assert_eq!(fine_steps_count, MAX_FINE_STEPS);
    }

    #[test]
    fn union_height_counts_overlaps_and_stacked_spans_once() {
        assert_eq!(union_height(&[]), 0.0);
        // Side-by-side cells overlap; the stacked span below adds its own height.
        assert_eq!(union_height(&[(0.0, 20.0), (5.0, 20.0), (20.0, 30.0)]), 30.0);
        // Malformed input (reversed, NaN, infinite) is ignored rather than poisoning the total.
        assert_eq!(union_height(&[(10.0, 5.0), (f32::NAN, 1.0), (0.0, f32::INFINITY), (2.0, 4.0)]), 2.0);
    }

    #[test]
    fn final_page_volume_counts_non_text_body_items() {
        let page = Page {
            body: wordcraft_geom::Rect::new(0.0, 0.0, 500.0, 700.0),
            items: vec![Placed::Cell {
                rect: wordcraft_geom::Rect::new(0.0, 0.0, 500.0, 300.0),
                covered: false,
                table: wordcraft_doc::Path::top(0),
                row: 0,
                cell: 0,
                story: StoryRef::Body,
                clip: false,
            }],
            ..Default::default()
        };
        let (lines, ratio) = final_page_volume(&Document::new(), &page);
        assert_eq!(lines, 0);
        assert!(ratio > 0.30, "a 300 pt image-only cell on a 700 pt body must count: {ratio}");
    }

    #[test]
    fn explicit_breaks_include_resolved_page_break_before_and_nested_tables() {
        let mut doc = Document::new();
        doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("body", Default::default()))];
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.props.page_break_before = Some(true);
        }
        assert!(has_explicit_breaks(&doc, HideDeleted::default(), false), "a direct page_break_before is an explicit break");

        let mut doc = Document::new();
        doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("body", Default::default()))];
        let style = wordcraft_doc::Style {
            id: "BreakBefore".into(),
            name: "Break Before".into(),
            kind: wordcraft_doc::StyleKind::Paragraph,
            para: ParaProps { page_break_before: Some(true), ..Default::default() },
            ..Default::default()
        };
        Arc::make_mut(&mut doc.styles).upsert(style);
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.props.style = Some("BreakBefore".into());
        }
        assert!(has_explicit_breaks(&doc, HideDeleted::default(), false), "a style-inherited page_break_before is an explicit break");

        let mut doc = Document::new();
        let mut table = wordcraft_doc::Table::new(1, 1, 100.0);
        let mut cell_p = wordcraft_doc::Paragraph::with_text("cell", Default::default());
        cell_p.props.page_break_before = Some(true);
        table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cell_p)];
        doc.body = vec![Arc::new(wordcraft_doc::Block::Table(table))];
        assert!(has_explicit_breaks(&doc, HideDeleted::default(), false), "a page_break_before inside a table cell is an explicit break");
    }

    #[test]
    fn hidden_and_deleted_page_breaks_follow_print_visibility() {
        let text = format!("before{}after", wordcraft_doc::para::PAGE_BREAK);
        let mut doc = Document::new();
        doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.runs[0].props.hidden = Some(true);
        }
        assert!(!has_explicit_breaks(&doc, HideDeleted::default(), false), "hidden break is omitted in Print view");
        assert!(has_explicit_breaks(&doc, HideDeleted::default(), true), "hidden break counts when hidden marks are shown");

        let mut doc = Document::new();
        doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.runs[0].props.del = Some(0);
        }
        assert!(!has_explicit_breaks(&doc, HideDeleted::ALL, false), "deleted break is omitted while markup is hidden");
        assert!(has_explicit_breaks(&doc, HideDeleted::default(), false), "deleted break counts when markup is shown");
    }

    #[test]
    fn conditional_table_style_hidden_override_hides_page_break() {
        let text = format!("before{}after", wordcraft_doc::para::PAGE_BREAK);
        let mut table = wordcraft_doc::Table::new(1, 1, 100.0);
        table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
        table.props.style = Some("HiddenHeader".into());

        let mut doc = Document::new();
        doc.body = vec![Arc::new(wordcraft_doc::Block::Table(table))];
        let style = wordcraft_doc::Style {
            id: "HiddenHeader".into(),
            name: "Hidden Header".into(),
            kind: wordcraft_doc::StyleKind::Table,
            table: Some(wordcraft_doc::styles::TableStyleParts {
                header_chr: CharProps { hidden: Some(true), ..Default::default() },
                ..Default::default()
            }),
            ..Default::default()
        };
        Arc::make_mut(&mut doc.styles).upsert(style);

        assert!(!has_explicit_breaks(&doc, HideDeleted::default(), false), "a first-row hidden override must hide the break");
        assert!(has_explicit_breaks(&doc, HideDeleted::default(), true), "the hidden break counts when hidden marks are shown");
    }

    #[test]
    fn provenance_resets_when_body_and_heading_are_restored() {
        let mut doc = Document::from_text("Alpha\nBeta\nGamma\nDelta");
        for (i, b) in doc.body.iter_mut().enumerate() {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(b) {
                let size = if i == 0 { 24.0 } else { 12.0 };
                for r in &mut p.runs {
                    r.props.size = Some(size);
                }
            }
        }
        let orig = OriginalTypography::from_doc(&doc);
        wordcraft_doc::shrink::fit_document_typography(&mut doc, 1.0, 1.0);
        set_cumulative_shrink_state(&mut doc, 1.0, 1.0, orig);

        assert!(has_previously_reduced_text(&doc, doc.fit_state.as_ref().unwrap()));
        // Restoring only the heading leaves the body reduced.
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            for r in &mut p.runs {
                r.props.size = Some(24.0);
            }
        }
        assert!(has_previously_reduced_text(&doc, doc.fit_state.as_ref().unwrap()));
        // Restoring the body too leaves no reduced text.
        for b in doc.body.iter_mut().skip(1) {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(b) {
                for r in &mut p.runs {
                    r.props.size = Some(12.0);
                }
            }
        }
        assert!(!has_previously_reduced_text(&doc, doc.fit_state.as_ref().unwrap()));
    }

    #[test]
    fn provenance_resets_when_mixed_line_spacings_are_restored() {
        use wordcraft_doc::props::LineSpacing;

        let mut doc = Document::from_text("Alpha\nBeta");
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
        }
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
            p.props.line_spacing = Some(LineSpacing::Multiple(2.0));
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
        }
        let orig = OriginalTypography::from_doc(&doc);
        wordcraft_doc::shrink::fit_document_typography(&mut doc, 0.0, 0.9);
        set_cumulative_shrink_state(&mut doc, 0.0, 0.9, orig);

        assert!(has_previously_reduced_text(&doc, doc.fit_state.as_ref().unwrap()));
        // Restoring the 2.0 spacing but leaving the 1.0 one reduced keeps the budget.
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
            p.props.line_spacing = Some(LineSpacing::Multiple(2.0));
        }
        assert!(has_previously_reduced_text(&doc, doc.fit_state.as_ref().unwrap()));
        // Restoring both resets it.
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        }
        assert!(!has_previously_reduced_text(&doc, doc.fit_state.as_ref().unwrap()));
    }

    #[test]
    fn provenance_resets_when_neighbouring_sizes_are_restored() {
        let mut doc = Document::from_text("Alpha\nBeta");
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            for r in &mut p.runs {
                r.props.size = Some(12.0);
            }
        }
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
            for r in &mut p.runs {
                r.props.size = Some(11.0);
            }
        }
        let orig = OriginalTypography::from_doc(&doc);
        wordcraft_doc::shrink::fit_document_typography(&mut doc, 1.0, 1.0);
        set_cumulative_shrink_state(&mut doc, 1.0, 1.0, orig);
        assert!(font_reduction_present(&doc, doc.fit_state.as_ref().unwrap()));

        // Restoring 12 pt and 11 pt makes 11 pt look like another size's reduction interval; the
        // exact original multiset must clear the budget.
        for (i, size) in [(0, 12.0), (1, 11.0)] {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[i]) {
                for r in &mut p.runs {
                    r.props.size = Some(size);
                }
            }
        }
        assert!(!font_reduction_present(&doc, doc.fit_state.as_ref().unwrap()));
        assert_eq!(active_budget(&doc).delta, 0.0);
    }

    #[test]
    fn paragraph_spacing_keeps_the_budget_when_line_spacing_rounds_back() {
        use wordcraft_doc::props::LineSpacing;

        let mut doc = Document::from_text("Alpha\nBeta");
        for p in doc.body.iter_mut() {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(p) {
                p.props.line_spacing = Some(LineSpacing::Exactly(8.0));
                p.props.space_before = Some(20.0);
                p.props.space_after = Some(0.0);
            }
        }
        let orig = OriginalTypography::from_doc(&doc);
        wordcraft_doc::shrink::fit_document_typography(&mut doc, 0.0, 0.94);
        set_cumulative_shrink_state(&mut doc, 0.0, 0.94, orig);

        // Exactly(8.0) rounds back to 8.0, but 20 pt paragraph spacing becomes 18.8 pt.
        assert!(!line_spacing_reduction_present(&doc, doc.fit_state.as_ref().unwrap()));
        assert!(paragraph_spacing_reduction_present(&doc, doc.fit_state.as_ref().unwrap()));
        assert!(spacing_reduction_present(&doc, doc.fit_state.as_ref().unwrap()));

        for p in doc.body.iter_mut() {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(p) {
                p.props.space_before = Some(20.0);
            }
        }
        assert!(!spacing_reduction_present(&doc, doc.fit_state.as_ref().unwrap()));
    }

    #[test]
    fn font_and_spacing_budgets_reset_independently() {
        use wordcraft_doc::props::LineSpacing;

        let mut doc = Document::from_text("Alpha\nBeta");
        for p in doc.body.iter_mut() {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(p) {
                p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
                p.props.space_before = Some(10.0);
                p.props.space_after = Some(0.0);
                for r in &mut p.runs {
                    r.props.size = Some(12.0);
                }
            }
        }
        let orig = OriginalTypography::from_doc(&doc);
        wordcraft_doc::shrink::fit_document_typography(&mut doc, 1.0, 0.9);
        set_cumulative_shrink_state(&mut doc, 1.0, 0.9, orig);

        let both = active_budget(&doc);
        assert_eq!(both.delta, 1.0);
        assert!((both.space - 0.9).abs() < 1e-4);
        assert!(both.font_active && both.spacing_active);

        // Restore only the spacing: its budget resets while the font budget remains.
        for p in doc.body.iter_mut() {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(p) {
                p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
                p.props.space_before = Some(10.0);
                p.props.space_after = Some(0.0);
            }
        }
        let spacing_restored = active_budget(&doc);
        assert_eq!(spacing_restored.delta, 1.0);
        assert_eq!(spacing_restored.space, 1.0);
        assert!(spacing_restored.font_active);
        assert!(!spacing_restored.spacing_active);

        // Restoring the fonts too clears the other budget.
        for p in doc.body.iter_mut() {
            if let wordcraft_doc::Block::Para(p) = Arc::make_mut(p) {
                for r in &mut p.runs {
                    r.props.size = Some(12.0);
                }
            }
        }
        let restored = active_budget(&doc);
        assert_eq!(restored.delta, 0.0);
        assert_eq!(restored.space, 1.0);
        assert!(!restored.font_active && !restored.spacing_active);
    }

    #[test]
    fn fit_layout_options_match_session_visibility_and_numeral() {
        let mut s = Session::new(Document::from_text("Alpha\nBeta"));
        s.view.mode = ViewMode::Print;
        s.view.show_markup = true;
        s.prefs.markup.balloons = wordcraft_layout::display::BalloonMode::Revisions;
        s.prefs.markup.insertions_deletions = true;
        s.view.numeral = wordcraft_layout::NumeralMode::Hindi;
        s.view.proofing = false;
        s.prefs.mark_grammar = false;

        let opts = fit_layout_options(&s);
        assert_eq!(opts.hide_deleted, !s.view.show_markup);
        assert_eq!(opts.hide_marked, s.prefs.markup.hidden_deletions());
        assert_eq!(opts.deletions_in_balloons, s.revisions_in_balloons());
        assert_eq!(opts.numeral, s.view.numeral.resolve(crate::locale::system_locale()));

        let mut cache = LayoutCache::new();
        let fitted = wordcraft_layout::layout(&s.doc, &mut cache, &opts);
        assert_eq!(fitted.pages.len(), s.layout().pages.len());
    }

    #[test]
    fn provenance_tracks_reductions_per_location_not_sorted_multisets() {
        let mut doc = Document::from_text("Alpha\nBeta");
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            for r in &mut p.runs {
                r.props.size = Some(12.0);
            }
        }
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
            for r in &mut p.runs {
                r.props.size = Some(11.0);
            }
        }
        let orig = OriginalTypography::from_doc(&doc);
        wordcraft_doc::shrink::fit_document_typography(&mut doc, 1.0, 1.0);
        set_cumulative_shrink_state(&mut doc, 1.0, 1.0, orig);

        // Setting the second run (reduced to 10 pt) to 12 pt yields sorted sizes [11.0, 12.0],
        // matching the sorted originals [11.0, 12.0]. However, the first run (Slot 0) is still reduced
        // from 12 pt to 11 pt, so the active budget must not be cleared.
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
            for r in &mut p.runs {
                r.props.size = Some(12.0);
            }
        }
        assert!(font_reduction_present(&doc, doc.fit_state.as_ref().unwrap()), "Slot 0 is still reduced");
        assert_eq!(active_budget(&doc).delta, 1.0);

        // Restoring both runs to their respective original locations clears the budget.
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            for r in &mut p.runs {
                r.props.size = Some(12.0);
            }
        }
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
            for r in &mut p.runs {
                r.props.size = Some(11.0);
            }
        }
        assert!(!font_reduction_present(&doc, doc.fit_state.as_ref().unwrap()));
        assert_eq!(active_budget(&doc).delta, 0.0);
    }

    #[test]
    fn paragraph_swap_preserves_active_budget() {
        let mut doc = Document::from_text("Alpha\nBeta");
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            for r in &mut p.runs {
                r.props.size = Some(12.0);
            }
        }
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
            for r in &mut p.runs {
                r.props.size = Some(11.0);
            }
        }
        let orig = OriginalTypography::from_doc(&doc);
        wordcraft_doc::shrink::fit_document_typography(&mut doc, 1.0, 1.0);
        set_cumulative_shrink_state(&mut doc, 1.0, 1.0, orig);

        // Swap the two paragraphs: doc.body[0] is now Beta (10 pt), doc.body[1] is now Alpha (11 pt).
        doc.body.swap(0, 1);
        assert!(font_reduction_present(&doc, doc.fit_state.as_ref().unwrap()), "reordered paragraphs must preserve active budget");
        assert_eq!(active_budget(&doc).delta, 1.0);
    }

    #[test]
    fn step_pt_must_be_positive_and_finite() {
        let mut s = Session::new(Document::from_text("Alpha\nBeta"));
        let err_zero = s.run("layout.fitOnePage", &serde_json::json!({"stepPt": 0.0}));
        assert!(matches!(err_zero, Err(CmdError::Params(_))));
        let err_neg = s.run("layout.fitOnePage", &serde_json::json!({"stepPt": -0.5}));
        assert!(matches!(err_neg, Err(CmdError::Params(_))));
    }
}
