use super::margins::{MarginEdge, margin_handle};
use super::pages::page_screen_scale;
use super::{DocLayout, Pos2, Rect, Sense, Stroke, Tokens, Ui, WordApp, json, pos2, regular, vec2};

/// Horizontal and vertical rulers for the caret's page and paragraph. Each spans that page's
/// rect on screen at the page's own scale and is clipped to its strip, whatever the layout
/// (one column, One Page, Multiple Pages), and follows the caret to another page (#1878).
pub(super) fn rulers(app: &mut WordApp, ui: &mut Ui, h: Rect, v: Rect, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let corner = Rect::from_min_max(pos2(v.min.x, h.min.y), pos2(v.max.x, h.max.y));
    p.rect_filled(corner, 0.0, t.canvas);
    p.rect_filled(h, 0.0, t.canvas);
    p.rect_filled(v, 0.0, t.canvas);
    crate::ruler_tabs::selector(app, ui, corner);
    let pi = app.session.page_hint.min(layout.pages.len().saturating_sub(1));
    let (Some(page), Some(pr)) = (layout.pages.get(pi), rects.get(pi)) else { return };
    let scale = page_screen_scale(*pr, page, scale);
    let sect = app.session.doc.sections().get(page.section).map(|(_, s)| (*s).clone()).unwrap_or_default();
    // Horizontal.
    let hp = ui.painter_at(h);
    let x0 = pr.min.x;
    let bar = Rect::from_min_max(pos2(x0, h.min.y + 4.0), pos2(x0 + page.w * scale, h.max.y - 3.0));
    hp.rect_filled(bar, 0.0, t.ruler_margin);
    let text = Rect::from_min_max(pos2(x0 + page.body.x * scale, bar.min.y), pos2(x0 + page.body.right() * scale, bar.max.y));
    hp.rect_filled(text, 0.0, t.ruler);
    let rp = app.session.doc.para_at(&app.session.sel.focus).map(|para| app.session.doc.styles.resolve_para(&para.props));
    // Graduations in the File › Options unit.
    let (unit, div, step) = ruler_scale(app.session.units());
    // The caret paragraph's reading direction anchors the whole scale (#692): for a right-to-left
    // paragraph it runs from the page's right edge and the numbers increase right-to-left, so the
    // ruler matches the indent and tab markers, which sit at the paragraph's start edge.
    let rtl = rp.as_ref().is_some_and(|rp| rp.bidi);
    let origin = if rtl { page.w - page.body.right() } else { page.body.x };
    // A distance along the reading direction (points from the page's start edge) and back.
    let sx = |d: f32| ruler_x(d, x0, scale, page.w, rtl);
    let s_at = |x: f32| ruler_d(x, x0, scale, page.w, rtl);
    // The caret paragraph's column (in a table, the cell's text) and where it starts, from the
    // page's start edge (the right edge when right-to-left).
    let (col_x, w) = crate::table_ruler::text_area(app, layout, pi).unwrap_or((page.body.x, page.body.w));
    let start = if rtl { page.w - (col_x + w) } else { col_x };
    // Indents and tabs are measured from the paragraph's start edge: the right margin of a
    // right-to-left paragraph, where the start-indent markers sit.
    let px = |s: f32| sx(start + s);
    let dist_at = |x: f32| s_at(x) - start;
    // The markers the numbers keep clear of (#1878): the indents, the tab stops, a table's columns.
    let mut keep_clear: Vec<Rect> = Vec::new();
    if let Some(rp) = &rp {
        let [first, left, right] = [rp.indent_left + rp.indent_first, rp.indent_left, w - rp.indent_right].map(px);
        keep_clear.extend(indent_marker_rects(first, left, right, bar));
        keep_clear.extend(crate::ruler_tabs::marker_rects(app, bar, &px, &rp.tabs));
    }
    keep_clear.extend(crate::table_ruler::column_markers(app, ui.ctx(), layout, pi, bar, x0, scale));
    for (d, mark) in ruler_marks(origin, page.body.w, page.w, (unit, div, step), scale) {
        ruler_mark(&hp, pos2(sx(d), bar.center().y), mark, true, &keep_clear, &t);
    }
    // Margins: the edges of the white text area drag the section's margins. Registered before
    // the indent markers, so a marker sitting on an edge keeps the pointer where they overlap.
    let (le, re) = (x0 + page.body.x * scale, x0 + page.body.right() * scale);
    // An even page of a mirrored layout is the odd page reflected: its left edge is the stored right margin.
    let mirrored = app.session.doc.settings.mirror_margins && page.number.is_multiple_of(2);
    for (edge, zone) in [
        (MarginEdge::Left, Rect::from_min_max(pos2(le - 10.0, bar.min.y), pos2(le + 3.0, bar.max.y))),
        (MarginEdge::Right, Rect::from_min_max(pos2(re - 3.0, bar.min.y), pos2(re + 10.0, bar.max.y))),
    ] {
        margin_handle(app, ui, edge, zone, |p| (p.x - x0) / scale, &sect, mirrored);
    }
    // Indent markers for the caret's paragraph.
    if let Some(rp) = rp {
        let first = px(rp.indent_left + rp.indent_first);
        let left = px(rp.indent_left);
        let right = px(w - rp.indent_right);
        let c = t.text_dim;
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(first - 4.5, bar.min.y), pos2(first + 4.5, bar.min.y), pos2(first, bar.min.y + 5.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(left - 4.5, bar.max.y - 3.0), pos2(left + 4.5, bar.max.y - 3.0), pos2(left, bar.max.y - 8.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        hp.rect(
            Rect::from_min_max(pos2(left - 4.5, bar.max.y - 3.0), pos2(left + 4.5, bar.max.y + 1.0)),
            0.0,
            t.ruler,
            Stroke::new(1.0, c),
            egui::StrokeKind::Inside,
        );
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(right - 4.5, bar.max.y - 1.0), pos2(right + 4.5, bar.max.y - 1.0), pos2(right, bar.max.y - 6.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        // Tab stops: drawn and registered before the indent markers' drags, which win where they overlap.
        let step = unit / div as f32 / 2.0;
        let ruler = crate::ruler_tabs::Ruler { bar, text, width: w, step, rtl, x_of: &px, dist_at: &dist_at };
        crate::ruler_tabs::stops(app, ui, &hp, &ruler, &rp.tabs);
        // In a table, its column borders: registered after the tab stops (whose set-a-stop zone
        // spans the text area, so a border marker keeps its drag) and before the indent markers,
        // which keep the pointer where they overlap.
        crate::table_ruler::columns(app, ui, layout, pi, bar, x0, scale);
        // Dragging the left-indent marker.
        let id = ui.scope_id().with("ruler_left");
        let mr = Rect::from_center_size(pos2(left, bar.max.y - 3.0), vec2(12.0, 12.0));
        let r = ui.interact(mr, id, Sense::click_and_drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = dist_at(pp.x).clamp(-col_x, (w - 18.0).max(-col_x));
            let snapped = (pt / 4.5).round() * 4.5;
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"left": snapped}));
        }
        let fr = Rect::from_center_size(pos2(first, bar.min.y + 3.0), vec2(12.0, 10.0));
        let r = ui.interact(fr, ui.scope_id().with("ruler_first"), Sense::click_and_drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = dist_at(pp.x) - rp.indent_left;
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"firstLine": (pt / 4.5).round() * 4.5}));
        }
        let rr = Rect::from_center_size(pos2(right, bar.max.y - 3.0), vec2(12.0, 12.0));
        let r = ui.interact(rr, ui.scope_id().with("ruler_right"), Sense::click_and_drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = w - dist_at(pp.x);
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"right": (pt / 4.5).round() * 4.5}));
        }
    }
    // Vertical.
    let vp = ui.painter_at(v);
    let y0 = pr.min.y;
    let vbar = Rect::from_min_max(pos2(v.min.x + 4.0, y0), pos2(v.max.x - 3.0, pr.max.y));
    vp.rect_filled(vbar, 0.0, t.ruler_margin);
    let (te, be) = (y0 + page.body.y * scale, y0 + page.body.bottom() * scale);
    vp.rect_filled(Rect::from_min_max(pos2(vbar.min.x, te), pos2(vbar.max.x, be)), 0.0, t.ruler);
    for (edge, ey) in [(MarginEdge::Top, te), (MarginEdge::Bottom, be)] {
        let zone = Rect::from_min_max(pos2(vbar.min.x, ey - 4.0), pos2(vbar.max.x, ey + 4.0));
        margin_handle(app, ui, edge, zone, |p| (p.y - y0) / scale, &sect, false);
    }
    // The same graduations down the page; the numbers keep clear of a table's row markers.
    let keep_clear = crate::table_ruler::row_markers(app, ui.ctx(), layout, pi, vbar, y0, scale);
    for (d, mark) in ruler_marks(page.body.y, page.body.h, page.h.min(20_000.0), (unit, div, step), scale) {
        ruler_mark(&vp, pos2(vbar.center().x, y0 + d * scale), mark, false, &keep_clear, &t);
    }
    // In a table, its rows' bottom borders.
    crate::table_ruler::rows(app, ui, layout, pi, vbar, y0, scale);
}

/// The indent markers' outlines on the horizontal ruler `bar`, each centred on its screen x:
/// first line (a flag at the top), left and right (at the bottom).
fn indent_marker_rects(first: f32, left: f32, right: f32, bar: Rect) -> [Rect; 3] {
    [
        Rect::from_min_max(pos2(first - 4.5, bar.min.y), pos2(first + 4.5, bar.min.y + 5.0)),
        Rect::from_min_max(pos2(left - 4.5, bar.max.y - 8.0), pos2(left + 4.5, bar.max.y + 1.0)),
        Rect::from_min_max(pos2(right - 4.5, bar.max.y - 6.0), pos2(right + 4.5, bar.max.y - 1.0)),
    ]
}

/// One mark drawn along a ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RulerMark {
    /// A number, `n` units from the start margin.
    Label(i32),
    /// A tick line, `len` screen points long.
    Tick(f32),
}

/// A whole unit's tick where it has no number.
pub(super) const UNIT_TICK: f32 = 7.0;

/// The least room between two numbers on a ruler (screen points).
pub(super) const RULER_LABEL_PX: f32 = 30.0;

/// The least room between two ticks on a ruler (screen points).
const RULER_TICK_PX: f32 = 4.0;

/// The sparsest a ruler gets: past this it is left blank.
pub(super) const RULER_SPARSEST: i32 = 1 << 20;

/// Most marks on one ruler (page sizes come from files).
pub(super) const RULER_MAX_MARKS: usize = 2000;

/// How a ruler thins out as the page shrinks on screen (#1878).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RulerDensity {
    /// A number every this many units: 1, 2, 4, 8…
    pub(super) label_every: i32,
    /// A tick every this many of the unit's subdivisions: 1, 2, 4…
    pub(super) tick_every: i32,
}

/// The density of a ruler whose unit takes `px_per_unit` screen points and has `div`
/// subdivisions: the fewest units between numbers (doubling from 1) that keep them at least
/// `RULER_LABEL_PX` apart, and the fewest subdivisions between ticks that keep those at least
/// `RULER_TICK_PX` apart. The numbers' text keeps its size.
pub(super) fn ruler_density(px_per_unit: f32, div: i32) -> RulerDensity {
    let px = if px_per_unit.is_finite() { px_per_unit.max(0.0) } else { 0.0 };
    let every = |px: f32, room: f32| {
        let mut n = 1;
        while px * (n as f32) < room && n < RULER_SPARSEST {
            n *= 2;
        }
        n
    };
    RulerDensity { label_every: every(px, RULER_LABEL_PX), tick_every: every(px / div.max(1) as f32, RULER_TICK_PX) }
}

/// The marks of a ruler with their distance from the page's start edge (points along the
/// reading direction): ticks from the page edge out to `width` on the grid of the start margin
/// `origin`, numbered in whole units from that margin across the `text` area only (the margins
/// get ticks, #1878). Whole units, halves, quarters and the rest get shorter ticks in turn, and
/// zoomed out the numbers and ticks thin out (`ruler_density`).
pub(super) fn ruler_marks(origin: f32, text: f32, width: f32, (unit, div, step): (f32, i32, i32), scale: f32) -> Vec<(f32, RulerMark)> {
    let div = div.max(2);
    let mut marks = Vec::new();
    if !(unit.is_finite() && unit > 0.0 && origin.is_finite() && text.is_finite() && width.is_finite() && scale.is_finite()) {
        return marks;
    }
    let density = ruler_density(scale * unit, div);
    let per_label = div.saturating_mul(density.label_every);
    // The first subdivision at or before the page edge, on the ticks' grid.
    let edge = ((origin / unit).ceil().clamp(-1e6, 1e6) as i32).saturating_mul(div).saturating_neg();
    let mut k = edge.div_euclid(density.tick_every) * density.tick_every;
    for _ in 0..RULER_MAX_MARKS {
        let d = origin + k as f32 * unit / div as f32;
        if d > width + 1e-3 {
            break;
        }
        if d >= -1e-3 && k != 0 {
            let in_text = k > 0 && d <= origin + text + 1e-3;
            let mark = if k % per_label == 0 && in_text {
                RulerMark::Label(k / div * step)
            } else if k % div == 0 {
                RulerMark::Tick(UNIT_TICK)
            } else if k % (div / 2) == 0 {
                RulerMark::Tick(5.0)
            } else if div >= 8 && k % (div / 4) == 0 {
                RulerMark::Tick(3.0)
            } else {
                RulerMark::Tick(1.5)
            };
            marks.push((d, mark));
        }
        k = k.saturating_add(density.tick_every);
    }
    marks
}

/// Draw one graduation at `at` on a ruler, across it when `horizontal`: a tick, or a number,
/// unless its text would sit on a marker in `keep_clear`, where a whole unit's tick stands in.
fn ruler_mark(p: &egui::Painter, at: Pos2, mark: RulerMark, horizontal: bool, keep_clear: &[Rect], t: &Tokens) {
    let len = match mark {
        RulerMark::Label(n) => {
            let galley = p.layout_no_wrap(n.to_string(), regular(9.5), t.ruler_tick);
            let r = Rect::from_center_size(at, galley.size());
            if !keep_clear.iter().any(|o| o.intersects(r)) {
                p.galley(r.min, galley, t.ruler_tick);
                return;
            }
            UNIT_TICK
        }
        RulerMark::Tick(len) => len,
    };
    let (a, b) = if horizontal {
        (pos2(at.x, at.y - len / 2.0), pos2(at.x, at.y + len / 2.0))
    } else {
        (pos2(at.x - len / 2.0, at.y), pos2(at.x + len / 2.0, at.y))
    };
    p.line_segment([a, b], Stroke::new(1.0, t.ruler_tick));
}

/// Screen x of a point `d` points from the page's start edge, on a page whose left edge sits at
/// `x0` drawn at `scale`. A right-to-left paragraph reads from the right, so its scale mirrors.
pub(super) fn ruler_x(d: f32, x0: f32, scale: f32, page_w: f32, rtl: bool) -> f32 {
    if rtl { x0 + (page_w - d) * scale } else { x0 + d * scale }
}

/// The inverse of [`ruler_x`]: the distance from the page's start edge under screen `x`.
pub(super) fn ruler_d(x: f32, x0: f32, scale: f32, page_w: f32, rtl: bool) -> f32 {
    if rtl { page_w - (x - x0) / scale } else { (x - x0) / scale }
}

/// A ruler's graduation for a unit: points per numbered step, divisions per step, and how much
/// each step counts (millimetre rulers number every centimetre as 10, 20…).
fn ruler_scale(u: wordcraft_geom::Unit) -> (f32, i32, i32) {
    use wordcraft_geom::Unit;
    match u {
        Unit::Centimeters => (wordcraft_geom::PT_PER_CM, 4, 1),
        Unit::Millimeters => (wordcraft_geom::PT_PER_CM, 4, 10),
        Unit::Points => (72.0, 8, 72),
        Unit::Picas => (72.0, 8, 6),
        Unit::Inches => (72.0, 8, 1),
    }
}
