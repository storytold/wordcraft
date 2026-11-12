//! Fit One Page command implementation (`layout.fitOnePage`).
//!
//! Automatically condenses a multi-page document by one page using speculative
//! layout evaluation, minimal typographical perturbation, and strict readability
//! safeguards (line spacing floor and font size floor).

use std::sync::Arc;

use serde_json::Value;
use wordcraft_doc::section::SectionStart;
use wordcraft_doc::shrink::fit_document_typography;
use wordcraft_doc::{CharProps, Document, ParaProps, StoryRef};
use wordcraft_layout::{LayoutCache, LayoutOptions, Page, Placed, ViewMode};

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
/// When `hide_deleted` is true, runs with deletion markup (`props.del.is_some()`) are skipped because they
/// are not rendered in Print view when revision markup is hidden.
/// When `show_hidden` is false, runs with hidden text formatting are skipped because they are omitted from layout.
fn min_para_font_size(
    doc: &Document,
    p: &wordcraft_doc::para::Paragraph,
    table_chr: Option<&wordcraft_doc::CharProps>,
    hide_deleted: bool,
    show_hidden: bool,
) -> f32 {
    let style_id = p.props.style.as_deref();
    let mut min_size = if p.runs.is_empty() {
        let mark_resolved = doc.styles.resolve_char_in(style_id, table_chr, &p.mark);
        if (mark_resolved.hidden && !show_hidden) || (hide_deleted && p.mark.del.is_some()) { f32::INFINITY } else { mark_resolved.size }
    } else {
        f32::INFINITY
    };
    let mut run_start: usize = 0;
    for r in &p.runs {
        let run_end = run_start.saturating_add(r.len).min(p.text.len());
        let run_text = p.text.get(run_start..run_end).unwrap_or("");
        run_start = run_end;
        if hide_deleted && r.props.del.is_some() {
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
/// When `hide_deleted` is true, deleted text runs hidden in Print view are skipped.
/// When `show_hidden` is false, hidden text runs omitted from Print view are skipped.
fn min_blocks_font_size(
    doc: &Document,
    blocks: &[Arc<wordcraft_doc::Block>],
    table_chr: Option<&wordcraft_doc::CharProps>,
    hide_deleted: bool,
    show_hidden: bool,
) -> f32 {
    use wordcraft_doc::styles::PartSet;

    let mut min_sz = f32::INFINITY;
    for b in blocks {
        if let Some(p) = b.as_para() {
            min_sz = min_sz.min(min_para_font_size(doc, p, table_chr, hide_deleted, show_hidden));
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
                    min_sz = min_sz.min(min_blocks_font_size(doc, &cell.blocks, Some(&cell_chr), hide_deleted, show_hidden));
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
    hide_deleted: bool,
    show_hidden: bool,
) -> bool {
    let style_id = p.props.style.as_deref();
    if p.runs.is_empty() {
        let mark = doc.styles.resolve_char_in(style_id, table_chr, &p.mark);
        let visible = !(mark.hidden && !show_hidden) && !(hide_deleted && p.mark.del.is_some());
        return visible && p.text.contains(wordcraft_doc::para::PAGE_BREAK);
    }
    let mut run_start = 0usize;
    for r in &p.runs {
        let run_end = run_start.saturating_add(r.len).min(p.text.len());
        let run_text = p.text.get(run_start..run_end).unwrap_or("");
        run_start = run_end;
        if hide_deleted && r.props.del.is_some() {
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
    hide_deleted: bool,
    show_hidden: bool,
) -> bool {
    for b in blocks {
        if let Some(p) = b.as_para() {
            if doc.styles.resolve_para_in(&p.props, table_para).page_break_before
                || paragraph_has_visible_page_break(doc, p, table_chr, hide_deleted, show_hidden)
            {
                return true;
            }
        } else if let Some(tbl) = b.as_table() {
            let style = tbl.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
            let cell_para = style.as_ref().map(|st| &st.para);
            let cell_chr = style.as_ref().map(|st| &st.chr);
            for row in &tbl.rows {
                for cell in &row.cells {
                    if blocks_have_explicit_breaks(doc, &cell.blocks, cell_para, cell_chr, hide_deleted, show_hidden) {
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
fn has_explicit_breaks(doc: &Document, hide_deleted: bool, show_hidden: bool) -> bool {
    let sections = doc.sections();
    if sections.len() > 1 {
        for (_, sec) in &sections[1..] {
            if matches!(sec.start, SectionStart::NextPage | SectionStart::EvenPage | SectionStart::OddPage) {
                return true;
            }
        }
    }
    blocks_have_explicit_breaks(doc, &doc.body, None, None, hide_deleted, show_hidden)
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
/// When `hide_deleted` is true, deleted text runs hidden in Print view are skipped.
/// When `show_hidden` is false, hidden text runs omitted from Print view are skipped.
fn min_body_font_size(doc: &Document, hide_deleted: bool, show_hidden: bool) -> f32 {
    let hf_boxes = wordcraft_doc::shrink::header_footer_text_boxes(doc);
    let mut min_sz = min_blocks_font_size(doc, &doc.body, None, hide_deleted, show_hidden);
    for (id, part) in &doc.parts {
        if !matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer | wordcraft_doc::PartKind::Comment)
            && !hf_boxes.contains(id)
        {
            min_sz = min_sz.min(min_blocks_font_size(doc, &part.blocks, None, hide_deleted, show_hidden));
        }
    }
    if min_sz.is_finite() { min_sz } else { wordcraft_doc::props::DEFAULT_FONT_SIZE }
}

/// Checks whether effective body font sizes are at or below the minimum readable font size (8.5 pt).
///
/// When `hide_deleted` is true, deleted text runs hidden in Print view are skipped.
/// When `show_hidden` is false, hidden text runs omitted from Print view are skipped.
fn doc_has_unreadable_body_font(doc: &Document, hide_deleted: bool, show_hidden: bool) -> bool {
    min_body_font_size(doc, hide_deleted, show_hidden) < MIN_COMFORTABLE_BODY_FONT_SIZE
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

/// Collects unique effective body font sizes across the document body and editable parts.
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
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sizes.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
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

/// Collects unique effective line spacings across body paragraphs and editable parts.
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
    spacings.sort_by(|a, b| match (a, b) {
        (wordcraft_doc::props::LineSpacing::Multiple(x), wordcraft_doc::props::LineSpacing::Multiple(y))
        | (wordcraft_doc::props::LineSpacing::AtLeast(x), wordcraft_doc::props::LineSpacing::AtLeast(y))
        | (wordcraft_doc::props::LineSpacing::Exactly(x), wordcraft_doc::props::LineSpacing::Exactly(y)) => {
            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
        }
        (wordcraft_doc::props::LineSpacing::Multiple(_), _) => std::cmp::Ordering::Less,
        (_, wordcraft_doc::props::LineSpacing::Multiple(_)) => std::cmp::Ordering::Greater,
        (wordcraft_doc::props::LineSpacing::AtLeast(_), _) => std::cmp::Ordering::Less,
        (_, wordcraft_doc::props::LineSpacing::AtLeast(_)) => std::cmp::Ordering::Greater,
    });
    spacings.dedup_by(|a, b| match (a, b) {
        (wordcraft_doc::props::LineSpacing::Multiple(x), wordcraft_doc::props::LineSpacing::Multiple(y))
        | (wordcraft_doc::props::LineSpacing::AtLeast(x), wordcraft_doc::props::LineSpacing::AtLeast(y))
        | (wordcraft_doc::props::LineSpacing::Exactly(x), wordcraft_doc::props::LineSpacing::Exactly(y)) => (*x - *y).abs() < 1e-4,
        _ => false,
    });
    spacings
}

/// Checks whether previously reduced text from an earlier fit is still present in the document.
fn has_previously_reduced_text(doc: &Document, fit: &wordcraft_doc::shrink::FitState) -> bool {
    let mut font_reduced_present = false;
    if fit.cum_delta > 0.0 {
        let current_sizes = collect_body_font_sizes(doc);
        for &cur in &current_sizes {
            if fit.reduced_font_sizes.iter().any(|&red| cur <= red + 1e-4) {
                font_reduced_present = true;
                break;
            }
        }
    }

    let mut space_reduced_present = false;
    if fit.cum_space < 1.0 - 1e-4 {
        let current_spacings = collect_body_line_spacings(doc);
        for cur in &current_spacings {
            if fit.reduced_line_spacings.iter().any(|red| match (cur, red) {
                (wordcraft_doc::props::LineSpacing::Multiple(c), wordcraft_doc::props::LineSpacing::Multiple(r))
                | (wordcraft_doc::props::LineSpacing::AtLeast(c), wordcraft_doc::props::LineSpacing::AtLeast(r))
                | (wordcraft_doc::props::LineSpacing::Exactly(c), wordcraft_doc::props::LineSpacing::Exactly(r)) => *c <= *r + 1e-4,
                _ => false,
            }) {
                space_reduced_present = true;
                break;
            }
        }
    }

    if fit.cum_delta > 0.0 && fit.cum_space < 1.0 - 1e-4 {
        font_reduced_present || space_reduced_present
    } else if fit.cum_delta > 0.0 {
        font_reduced_present
    } else if fit.cum_space < 1.0 - 1e-4 {
        space_reduced_present
    } else {
        false
    }
}

/// Reads the cumulative Fit One Page reduction state from the document's internal fit tracking.
///
/// Preserves the reduction budget when reopening a fitted document where `doc.fit_state` was serialized,
/// while maintaining the full budget for documents with no prior reductions.
fn cumulative_shrink_state(doc: &Document) -> (f32, f32) {
    if let Some(fit) = &doc.fit_state
        && has_previously_reduced_text(doc, fit)
    {
        return (fit.cum_delta, fit.cum_space);
    }
    (0.0, 1.0)
}

/// Records the cumulative Fit One Page reduction state into the document's internal fit tracking.
fn set_cumulative_shrink_state(doc: &mut Document, delta: f32, space: f32) {
    let sizes = collect_body_font_sizes(doc);
    let spacings = collect_body_line_spacings(doc);
    doc.fit_state = Some(wordcraft_doc::FitState { cum_delta: delta, cum_space: space, reduced_font_sizes: sizes, reduced_line_spacings: spacings });
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

/// Executes the `layout.fitOnePage` command.
///
/// Condenses the document by one page if feasible, preserving typographical
/// hierarchies, protecting line spacing, and clamping to readability bounds.
/// Refuses while Track Changes and Track Formatting are on, because the style
/// reductions it needs cannot be recorded as formatting revisions.
pub fn fit_one_page(s: &mut Session, v: &Value) -> CmdResult {
    // 0. Refuse while formatting changes must be tracked: fitting rescales styles, and the
    //    formatting-recording path compares direct run/paragraph properties only, so the
    //    inherited reductions could not be represented as revisions.
    if s.doc.settings.track_changes && s.prefs.markup.track_formatting {
        return Err(CmdError::Disabled(
            "the document tracks formatting changes: Fit One Page cannot record its style changes as formatting revisions".into(),
        ));
    }

    // 1. Anti-compound guard: refuse if cumulative reduction already reached 1.0 pt or spacing factor <= 0.94
    let (cum_delta, cum_space) = cumulative_shrink_state(&s.doc);
    if cum_delta >= 1.0 - 1e-4 || cum_space <= 0.94 + 1e-4 {
        return Err(CmdError::Failed("Unable to shrink document further without reducing text beyond readability limits.".into()));
    }

    // 2. Build equivalent Print layout options
    let shown_hidden = s.view.marks || s.prefs.marks.hidden;
    let hide_deleted = !s.view.show_markup;
    let (fn_name, fn_path) = s.resolved_filenames();
    let print_opts = LayoutOptions {
        view: ViewMode::Print,
        field_codes: s.view.field_codes,
        show_hidden: shown_hidden,
        hide_deleted,
        proofing: false,
        proof: wordcraft_layout::ProofOptions { grammar: false, spelling: Default::default() },
        filename: Some(Arc::from(fn_name.as_str())),
        filepath: Some(Arc::from(fn_path.as_str())),
        ..LayoutOptions::default()
    };

    // 3. Initial page count evaluation
    let mut initial_cache = LayoutCache::new();
    let initial_layout = wordcraft_layout::layout(&s.doc, &mut initial_cache, &print_opts);
    let p0 = initial_layout.pages.len();

    if p0 <= 1 {
        return Err(CmdError::Failed("The document already fits on one page.".into()));
    }

    // Explicit breaks force pages regardless of typography; refuse before any trial can appear to
    // succeed by moving text around them.
    if has_explicit_breaks(&s.doc, hide_deleted, shown_hidden) {
        return Err(CmdError::Failed("Unable to fit document to one page: explicit page or section breaks prevent page reduction.".into()));
    }

    // 4. Pre-flight check on final page content volume
    let last_page = &initial_layout.pages[p0 - 1];
    let (last_page_lines, fill_ratio) = final_page_volume(&s.doc, last_page);
    if last_page_lines > 12 || fill_ratio > 0.30 {
        return Err(CmdError::Failed(
            "Unable to fit document to one page: content on the final page exceeds limits for aesthetic page reduction.".into(),
        ));
    }
    let target_pages = p0.saturating_sub(1);

    // 5. Parse parameters safely
    let max_delta = p::f32(v, "maxDeltaPt").unwrap_or(2.0).clamp(0.0, 10.0);
    let step_pt_opt = p::f32(v, "stepPt");
    let adjust_spacing = p::bool(v, "adjustSpacing").unwrap_or(true);

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
        // Anti-compound budget across runs: cap cumulative font reduction at 1.0 pt
        if cum_delta > 0.0 && cum_delta + delta > 1.0 + 1e-4 {
            continue;
        }
        // Anti-compound budget across runs: cap cumulative line spacing compression at 0.94 floor
        let cand_line_space = alpha.max(0.94);
        let prior_line_space = cum_space.max(0.94);
        if cum_space < 1.0 - 1e-4 && prior_line_space * cand_line_space < 0.94 - 1e-4 {
            continue;
        }
        // Anti-compound guard: refuse font reduction if body font is already at or below the readability floor,
        // while allowing spacing-only candidates (delta == 0.0) to run.
        if delta > 0.0 && doc_has_unreadable_body_font(&s.doc, hide_deleted, shown_hidden) {
            continue;
        }

        let mut trial_doc = s.doc.clone();
        fit_document_typography(&mut trial_doc, delta, alpha);

        // Validate the trial document's effective body font sizes against 8.5 pt minimum before accepting
        if delta > 0.0 && doc_has_unreadable_body_font(&trial_doc, hide_deleted, shown_hidden) {
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
            set_cumulative_shrink_state(&mut new_doc, cum_delta + d, cum_space * a);
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
            if min_body_font_size(&s.doc, hide_deleted, shown_hidden) <= MIN_COMFORTABLE_BODY_FONT_SIZE {
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
        assert!(has_explicit_breaks(&doc, false, false), "a direct page_break_before is an explicit break");

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
        assert!(has_explicit_breaks(&doc, false, false), "a style-inherited page_break_before is an explicit break");

        let mut doc = Document::new();
        let mut table = wordcraft_doc::Table::new(1, 1, 100.0);
        let mut cell_p = wordcraft_doc::Paragraph::with_text("cell", Default::default());
        cell_p.props.page_break_before = Some(true);
        table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(cell_p)];
        doc.body = vec![Arc::new(wordcraft_doc::Block::Table(table))];
        assert!(has_explicit_breaks(&doc, false, false), "a page_break_before inside a table cell is an explicit break");
    }

    #[test]
    fn hidden_and_deleted_page_breaks_follow_print_visibility() {
        let text = format!("before{}after", wordcraft_doc::para::PAGE_BREAK);
        let mut doc = Document::new();
        doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.runs[0].props.hidden = Some(true);
        }
        assert!(!has_explicit_breaks(&doc, false, false), "hidden break is omitted in Print view");
        assert!(has_explicit_breaks(&doc, false, true), "hidden break counts when hidden marks are shown");

        let mut doc = Document::new();
        doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&text, Default::default()))];
        if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[0]) {
            p.runs[0].props.del = Some(0);
        }
        assert!(!has_explicit_breaks(&doc, true, false), "deleted break is omitted while markup is hidden");
        assert!(has_explicit_breaks(&doc, false, false), "deleted break counts when markup is shown");
    }
}
