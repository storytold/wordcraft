//! The caret section's text columns on the horizontal ruler (#5197): the space between each two
//! columns is shaded like the margins, and its edges are dragged to change the columns' widths
//! and spacing, through `layout.columns`. As in Word for Mac, the space has no grip of its own
//! in the middle: only its edges drag.

use serde_json::Value;
use wordcraft_doc::section::SectionProps;
use wordcraft_doc::{Pos, StoryRef};

use super::{DocLayout, Rect, Sense, Tokens, Ui, WordApp, json, pos2};

/// Which edge of the space between two columns a drag grabs, in reading order: the end of the
/// column before it or the start of the column after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GapGrip {
    End,
    Start,
}

/// The narrowest column a drag leaves: a quarter inch, as the Columns dialog allows.
const MIN_COLUMN: f32 = 18.0;

/// The widest space a drag leaves between two columns, as the Columns dialog allows.
const MAX_SPACE: f32 = 300.0;

/// The caret section's columns on a page, for the ruler.
pub(super) struct ColumnGaps {
    /// The caret's section.
    sect: SectionProps,
    /// The page x of the section's text area's left edge.
    origin: f32,
    /// Each space between two neighbouring columns in reading order: its (left, right) page x.
    gaps: Vec<(f32, f32)>,
}

/// The spaces between the caret section's columns on page `pi`: none outside the main text or
/// Print Layout (where text runs in one column), in a table (whose own markers take the ruler),
/// in a section of one column, for a selection across sections (a drag would set the columns of
/// each) or when the caret isn't on that page (the ruler stays put while the whole document is
/// selected).
pub(super) fn column_gaps(app: &WordApp, layout: &DocLayout, pi: usize) -> Option<ColumnGaps> {
    let sel = &app.session.sel;
    if sel.focus.story != StoryRef::Body || sel.anchor.story != StoryRef::Body {
        return None;
    }
    if app.session.view.mode != wordcraft_layout::ViewMode::Print || app.session.view.read_mode {
        return None;
    }
    if sel.focus.path.cell().is_some() || crate::table_ruler::text_area(app, layout, pi).is_some() {
        return None;
    }
    let doc = &app.session.doc;
    let section = |p: &Pos| doc.section_index_of(p.path.0.first().copied().unwrap_or(0) as usize);
    if section(&sel.anchor) != section(&sel.focus) {
        return None;
    }
    if layout.caret_on(&sel.focus, pi).map(|c| c.page) != Some(pi) {
        return None;
    }
    let sect = wordcraft_engine::cmd::page::sect(&app.session);
    if sect.columns.count <= 1 {
        return None;
    }
    let page = layout.pages.get(pi)?;
    // A continuous section can change the margins on the page: its columns start at its own
    // margin, moved as the page's text area is (on an even page of a mirrored layout, #2914).
    let sections = app.session.doc.sections();
    let (_, page_sect) = sections.get(page.section)?;
    let origin = page.body.x + sect.margin_left + sect.gutter - page_sect.margin_left - page_sect.gutter;
    let gaps = gaps_of(&sect).into_iter().map(|(l, r)| (origin + l, origin + r)).collect();
    Some(ColumnGaps { sect, origin, gaps })
}

/// Each space between two neighbouring columns of `s` in reading order, as (left, right) points
/// from the text area's left edge.
fn gaps_of(s: &SectionProps) -> Vec<(f32, f32)> {
    s.column_boxes()
        .windows(2)
        .filter_map(|w| match w {
            // Leftwards in a right-to-left section.
            [(ax, aw), (bx, bw)] => Some(if bx >= ax { (ax + aw, *bx) } else { (bx + bw, *ax) }),
            _ => None,
        })
        .collect()
}

/// Shades the spaces between the columns on the ruler `bar`, over the white text area.
pub(super) fn draw(painter: &egui::Painter, cg: &ColumnGaps, bar: Rect, x0: f32, scale: f32, t: &Tokens) {
    for (l, r) in &cg.gaps {
        let (sl, sr) = (x0 + l * scale, x0 + r * scale);
        if sr > sl {
            painter.rect_filled(Rect::from_min_max(pos2(sl, bar.min.y), pos2(sr, bar.max.y)), 0.0, t.ruler_margin);
        }
    }
}

/// The drag zones of the spaces between the columns: each of their edges, none over a tab
/// stop's marker (`tabs`), which keeps its own drag. Registered after the tab stops (whose
/// set-a-stop zone spans the text area) and before the indent markers, which keep the pointer
/// where they overlap. Columns the command wouldn't take back get no zones.
pub(super) fn handles(app: &mut WordApp, ui: &Ui, cg: &ColumnGaps, bar: Rect, x0: f32, scale: f32, tabs: &[Rect]) {
    if !draggable(&cg.sect) {
        return;
    }
    let rtl = cg.sect.rtl;
    for (gap, (l, r)) in cg.gaps.iter().enumerate() {
        let (sl, sr) = (x0 + l * scale, x0 + r * scale);
        // Left to right, the space's left edge ends the column before it; right to left, it
        // starts the column after it.
        let (left, right) = if rtl { (GapGrip::Start, GapGrip::End) } else { (GapGrip::End, GapGrip::Start) };
        // A narrow space is split at its middle, so both edges can be grabbed.
        let mid = (sl + sr) / 2.0;
        let zones = [
            (left, Rect::from_min_max(pos2(sl - 3.0, bar.min.y), pos2((sl + 3.0).min(mid), bar.max.y))),
            (right, Rect::from_min_max(pos2((sr - 3.0).max(mid), bar.min.y), pos2(sr + 3.0, bar.max.y))),
        ];
        for (grip, zone) in zones {
            if tabs.iter().any(|t| t.intersects(zone)) {
                continue;
            }
            let r =
                ui.interact(zone, ui.scope_id().with(("ruler_column", gap, grip)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            let r = if r.dragged() { r } else { r.on_hover_text(tl!("Column Edge")) };
            if r.dragged()
                && let Some(pp) = r.interact_pointer_pos()
            {
                let v = (pp.x - x0) / scale - cg.origin;
                let at = if rtl { cg.sect.text_width() - v } else { v };
                let Some(params) = column_drag(&cg.sect, gap, grip, at) else { continue };
                if !r.drag_started() {
                    app.session.join_next_undo();
                }
                let _ = app.run_keep_view("layout.columns", params);
            }
        }
    }
}

/// The `layout.columns` parameters after dragging `grip` of the space after column `gap` (in
/// reading order) to `at` points from the text area's start edge (its right edge in a
/// right-to-left section). Equal columns stay equal, so every space changes, as in Word for Mac;
/// between unequal columns only the column at that edge and the space change. Columns stay at
/// least a quarter inch wide and the space 0–300 pt, snapped to 1/16 inch. `None` when there is
/// no such space or `at` is junk.
pub(crate) fn column_drag(s: &SectionProps, gap: usize, grip: GapGrip, at: f32) -> Option<Value> {
    let n = s.columns.count as usize;
    if !at.is_finite() || !draggable(s) || gap + 1 >= n {
        return None;
    }
    let tw = s.text_width();
    let sep = s.columns.separator;
    if s.columns.widths.len() != n {
        // Column k is w wide and starts at k·(w + space), w = (tw − (n − 1)·space) / n: solved
        // for the space that puts the edge at `at`.
        let (i, nf) = (gap as f32, n as f32);
        let space = match grip {
            GapGrip::End => ((i + 1.0) * tw - at * nf) / (nf - 1.0 - i),
            _ => at * nf / (i + 1.0) - tw,
        };
        let most = ((tw - MIN_COLUMN * nf) / (nf - 1.0)).clamp(0.0, MAX_SPACE);
        // Snapped, and at the limit the last 1/16 inch below it.
        let space = (space / 4.5).round() * 4.5;
        let space = if space > most { (most / 4.5).floor() * 4.5 } else { space.max(0.0) };
        return Some(json!({"count": n, "space": space, "separator": sep, "apply": "section"}));
    }
    // Unequal columns: (width, space after) each.
    let mut cols = s.columns.widths.clone();
    let start: f32 = cols.iter().take(gap).map(|(w, sp)| w + sp).sum();
    let (&(w0, sp), &(w1, sp1)) = (cols.get(gap)?, cols.get(gap + 1)?);
    let at = (at / 4.5).round() * 4.5;
    let (end0, start1, end1) = (start + w0, start + w0 + sp, start + w0 + sp + w1);
    let (w0, sp, w1) = match grip {
        GapGrip::End => {
            // `max` and `min` rather than `clamp`, which panics on bounds from a hostile file.
            let e = at.min(start1.max(start + MIN_COLUMN)).max((start + MIN_COLUMN).max(start1 - MAX_SPACE));
            (e - start, start1 - e, w1)
        }
        GapGrip::Start => {
            let b = at.min((end1 - MIN_COLUMN).min(end0 + MAX_SPACE)).max(end0);
            (w0, b - end0, end1 - b)
        }
    };
    if let Some(c) = cols.get_mut(gap) {
        *c = (w0, sp);
    }
    if let Some(c) = cols.get_mut(gap + 1) {
        *c = (w1, sp1);
    }
    Some(json!({"widths": cols.iter().map(|(w, sp)| [*w, *sp]).collect::<Vec<_>>(), "separator": sep, "apply": "section"}))
}

/// Whether `layout.columns` takes the section's columns back after a drag: 2–12 columns, unequal
/// ones each at least a quarter inch wide with 0–300 pt after it and no wider together than the
/// text. A file can hold others (up to 45 columns, any widths); their spaces are shown, not
/// dragged.
fn draggable(s: &SectionProps) -> bool {
    let n = s.columns.count as usize;
    let widths = &s.columns.widths;
    if !(2..=12).contains(&n) {
        return false;
    }
    if widths.len() != n {
        return true;
    }
    // The last column has no space after it (the command drops one).
    let space = |i: usize, sp: f32| if i + 1 == n { 0.0 } else { sp };
    let total: f32 = widths.iter().enumerate().map(|(i, (w, sp))| w + space(i, *sp)).sum();
    total <= s.text_width() + 1.0 && widths.iter().enumerate().all(|(i, (w, sp))| *w >= MIN_COLUMN && (0.0..=MAX_SPACE).contains(&space(i, *sp)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::section::Columns;

    /// Letter paper with inch margins: 468 pt of text.
    fn letter(columns: Columns) -> SectionProps {
        SectionProps { page_w: 612.0, page_h: 792.0, margin_left: 72.0, margin_right: 72.0, gutter: 0.0, columns, ..Default::default() }
    }

    fn equal(count: u32, space: f32) -> SectionProps {
        letter(Columns { count, space, ..Default::default() })
    }

    fn widths(v: &Value) -> Vec<(f32, f32)> {
        v["widths"].as_array().into_iter().flatten().map(|c| (c[0].as_f64().unwrap_or(0.0) as f32, c[1].as_f64().unwrap_or(0.0) as f32)).collect()
    }

    /// #5197: two equal columns half an inch apart leave a space from 216 to 252 pt.
    #[test]
    fn the_spaces_between_columns_are_where_the_columns_are_not() {
        assert_eq!(gaps_of(&equal(2, 36.0)), vec![(216.0, 252.0)]);
        assert_eq!(gaps_of(&equal(3, 36.0)), vec![(132.0, 168.0), (300.0, 336.0)]);
        assert!(gaps_of(&equal(1, 36.0)).is_empty(), "one column, no space");
        // Right to left, the first column is on the right: the same spaces, read leftwards.
        let rtl = SectionProps { rtl: true, ..equal(3, 36.0) };
        assert_eq!(gaps_of(&rtl), vec![(300.0, 336.0), (132.0, 168.0)]);
        let unequal = letter(Columns { count: 2, space: 18.0, widths: vec![(100.0, 18.0), (350.0, 0.0)], ..Default::default() });
        assert_eq!(gaps_of(&unequal), vec![(100.0, 118.0)]);
    }

    /// #5197: dragging an edge of the space between equal columns keeps them equal and changes
    /// the spacing, so the dragged edge lands at the pointer.
    #[test]
    fn dragging_an_edge_between_equal_columns_changes_the_spacing() {
        let s = equal(2, 36.0);
        // The first column's end, from 216 to 207: 9 pt more space each side, 18 in all.
        let v = column_drag(&s, 0, GapGrip::End, 207.0).unwrap_or_default();
        assert_eq!(v, json!({"count": 2, "space": 54.0, "separator": false, "apply": "section"}));
        // The second column's start, from 252 to 261: the same.
        assert_eq!(column_drag(&s, 0, GapGrip::Start, 261.0).unwrap_or_default()["space"], 54.0);
        // Past the middle the space closes; it never goes negative.
        assert_eq!(column_drag(&s, 0, GapGrip::End, 400.0).unwrap_or_default()["space"], 0.0);
        // At most 300 pt: 297, the last 1/16 inch below it.
        assert_eq!(column_drag(&s, 0, GapGrip::End, 0.0).unwrap_or_default()["space"], 297.0);
        // And it never leaves a column narrower than a quarter inch.
        let narrow = equal(12, 0.0);
        let most = (468.0 - 18.0 * 12.0) / 11.0;
        let space = column_drag(&narrow, 0, GapGrip::End, 0.0).unwrap_or_default()["space"].as_f64().unwrap_or(0.0) as f32;
        assert!(space <= most, "{space} leaves every column a quarter inch ({most})");
        assert_eq!(space % 4.5, 0.0, "{space} is on the 1/16 inch grid at the limit too");
        // Three columns: the second space's start edge, from 336 to 345.
        let s = equal(3, 36.0);
        assert_eq!(column_drag(&s, 1, GapGrip::Start, 345.0).unwrap_or_default()["space"], 49.5);
    }

    /// #5197: between unequal columns only the column at the dragged edge and the space change.
    #[test]
    fn dragging_between_unequal_columns_changes_its_neighbours() {
        let s = letter(Columns { count: 3, space: 18.0, widths: vec![(100.0, 18.0), (200.0, 18.0), (132.0, 0.0)], ..Default::default() });
        // The first column's end, 100 → 90: it narrows and the space after it widens.
        assert_eq!(widths(&column_drag(&s, 0, GapGrip::End, 90.0).unwrap_or_default()), vec![(90.0, 28.0), (200.0, 18.0), (132.0, 0.0)]);
        // The third column's start, 336 → 300, back into the second column: it stops at the
        // second column's end (318), the space closes and the third column takes it.
        assert_eq!(widths(&column_drag(&s, 1, GapGrip::Start, 300.0).unwrap_or_default()), vec![(100.0, 18.0), (200.0, 0.0), (150.0, 0.0)]);
        // The second column's start, 118 → 1000: the column never gets narrower than a quarter
        // inch.
        assert_eq!(widths(&column_drag(&s, 0, GapGrip::Start, 1000.0).unwrap_or_default()), vec![(100.0, 200.0), (18.0, 18.0), (132.0, 0.0)]);
    }

    /// Junk positions and spaces that aren't there change nothing.
    #[test]
    fn column_drags_ignore_junk() {
        let s = equal(2, 36.0);
        assert!(column_drag(&s, 0, GapGrip::End, f32::NAN).is_none());
        assert!(column_drag(&s, 1, GapGrip::End, 100.0).is_none(), "two columns have one space");
        assert!(column_drag(&equal(1, 36.0), 0, GapGrip::Start, 100.0).is_none());
        assert!(column_drag(&equal(13, 0.0), 0, GapGrip::End, 10.0).is_none(), "more columns than the command takes");
        // Unequal widths from a file that the command wouldn't take back aren't dragged.
        let narrow = letter(Columns { count: 2, widths: vec![(5.0, 0.0), (463.0, 0.0)], ..Default::default() });
        assert!(column_drag(&narrow, 0, GapGrip::End, 30.0).is_none(), "a column under a quarter inch");
        let wide = letter(Columns { count: 2, widths: vec![(100.0, 400.0), (50.0, 0.0)], ..Default::default() });
        assert!(column_drag(&wide, 0, GapGrip::End, 90.0).is_none(), "a space over 300 pt");
        let over = letter(Columns { count: 2, widths: vec![(300.0, 18.0), (300.0, 0.0)], ..Default::default() });
        assert!(column_drag(&over, 0, GapGrip::End, 290.0).is_none(), "wider than the text");
        let last = letter(Columns { count: 2, widths: vec![(200.0, 18.0), (250.0, 999.0)], ..Default::default() });
        assert!(column_drag(&last, 0, GapGrip::End, 190.0).is_some(), "the last column's space after it doesn't count");
    }

    /// Through the command: a drag's result is applied to the caret's section, keeps the line
    /// between columns, and is one undo step.
    #[test]
    fn a_column_drag_sets_the_sections_columns() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one two three")), Default::default());
        app.run("layout.columns", json!({"count": 2, "separator": true})).unwrap();
        let before = wordcraft_engine::cmd::page::sect(&app.session);
        // The first column's end, 9 pt to the left: 18 pt more space, the columns still equal.
        let end = before.column_boxes()[0].0 + before.column_boxes()[0].1;
        let params = column_drag(&before, 0, GapGrip::End, end - 9.0).unwrap();
        app.run("layout.columns", params).unwrap();
        let after = wordcraft_engine::cmd::page::sect(&app.session);
        assert_eq!(after.columns.count, 2);
        assert!(after.columns.widths.is_empty(), "the columns stay equal");
        assert_eq!(after.columns.space, before.columns.space + 18.0);
        assert!(after.columns.separator, "the line between the columns stays");
        let boxes = after.column_boxes();
        assert!((boxes[0].1 - (end - 9.0)).abs() < 0.01, "the edge is at the pointer: {boxes:?}");
        let laid = app.session.layout();
        let page = laid.pages.first().unwrap();
        let cg = column_gaps(&app, &laid, 0).unwrap();
        assert_eq!(cg.gaps.len(), 1);
        assert!((cg.gaps[0].0 - (page.body.x + boxes[0].1)).abs() < 0.01, "the ruler's space starts where the first column ends");
        assert!(app.session.undo());
        assert_eq!(wordcraft_engine::cmd::page::sect(&app.session).columns, before.columns);
    }

    /// A drag sets the columns of every section the selection touches, so a selection across
    /// sections gets no spaces on the ruler; nor does a page the caret isn't on.
    #[test]
    fn no_column_spaces_for_a_selection_across_sections_or_another_page() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Default::default());
        app.run("caret.docEnd", json!({})).unwrap();
        app.run("layout.break", json!({"kind": "nextPage"})).unwrap();
        app.run("layout.columns", json!({"count": 2, "apply": "section"})).unwrap();
        let laid = app.session.layout();
        assert_eq!(laid.pages.len(), 2);
        assert!(column_gaps(&app, &laid, 1).is_some(), "the caret's page shows the second section's columns");
        assert!(column_gaps(&app, &laid, 0).is_none(), "the first page has one column");
        app.run("select.all", json!({})).unwrap();
        let laid = app.session.layout();
        assert!(column_gaps(&app, &laid, 1).is_none(), "the selection runs into a section of one column");
    }
}
