//! The rulers around the caret's page: the framework's `PageRulers` shows them, and this file feeds them the caret
//! paragraph, the page's margins, the caret table's borders and the section's column gaps, and turns what they report
//! into commands. Margins, indents and column gaps run on every step of a drag, joined into one undo step; tab stops
//! and table borders run once, on the commit. A cancelled drag takes back what it ran.

use craft::behavior::{EditEvents, GestureId, GesturePhase};
use craft::pages::Margins;
use craft::pages::ui::{PageEdges, PageRulers};
use craft::text::ui::DocUnit;
use craft::text::ui::ruler::{RulerValues, TabAlign as RulerAlign, TabLeader as RulerLeader, TabStop as RulerStop};
use craft::ui::contract::Bind;
use craft::ui::runtime::env::UnitKind;
use serde_json::{Map, Value};
use wordcraft_doc::props::{TAB_POS_EPS, TabAlign, TabLeader, TabStop};
use wordcraft_engine::cmd::table_resize::Edge;

use super::{DocLayout, Placed, Ui, WordApp, json};
use crate::table_resize::ruler_params;

/// The id a column gap drag runs under: the gap drag reports a phase and no gesture id, and one pointer drives one
/// drag at a time, so a single id keeps its steps in one undo step.
const GAP_GESTURE: GestureId = GestureId::new(u64::MAX);

/// The snap step of indents, points: 1/16 inch.
const INDENT_SNAP: f32 = 4.5;

/// The narrowest text column an indent drag leaves, points.
const MIN_INDENT_ROOM: f32 = 18.0;

/// Shows the rulers of the caret's page and applies what they report. The rulers follow the caret to another page;
/// a new paragraph is a new ruler, so an open gesture ends with the paragraph it began on.
pub(crate) fn page_rulers(app: &mut WordApp, ui: &mut Ui, out: &craft::pages::ui::PageViewOutput, layout: &DocLayout) {
    let Some(canvas_rect) = app.canvas.canvas_rect() else { return };
    let pi = app.session.page_hint.min(layout.pages.len().saturating_sub(1));
    let (Some(page), Some(placed)) = (layout.pages.get(pi), out.pages.iter().find(|p| p.page == pi)) else { return };
    let sect = app.session.doc.sections().get(page.section).map(|(_, s)| (*s).clone()).unwrap_or_default();
    let focus = app.session.sel.focus.clone();
    let rp = app.session.doc.para_at(&focus).map(|para| app.session.doc.styles.resolve_para(&para.props));
    let rtl = rp.as_ref().is_some_and(|rp| rp.bidi);
    // The caret paragraph's column: in a table the cell's text, else the body.
    let (col_x, w) = text_area(app, layout, pi).unwrap_or((page.body.x, page.body.w));
    let col_end = col_x + w;
    let to_start = |x: f32| if rtl { col_end - x } else { x - col_x };
    let to_page = |d: f32| if rtl { col_end - d } else { col_x + d };

    let mut values = RulerValues::default();
    let mut doc_tabs: Vec<TabStop> = Vec::new();
    if let Some(rp) = &rp {
        values.stops = rp.tabs.iter().filter_map(|t| Some(RulerStop::new(t.pos, ruler_align(t.align)?, ruler_leader(t.leader)))).collect();
        values.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        values.left_indent = rp.indent_left;
        values.first_line_indent = rp.indent_first;
        values.right_indent = rp.indent_right;
        doc_tabs = rp.tabs.iter().filter(|t| t.align != TabAlign::Clear).copied().collect();
    }

    let cg = super::column_ruler::column_gaps(app, layout, pi);
    let gaps: Vec<(f32, f32)> = cg
        .as_ref()
        .map(|cg| cg.gaps.iter().map(|&(l, r)| if rtl { (col_end - r, col_end - l) } else { (l - col_x, r - col_x) }).collect())
        .unwrap_or_default();

    let row_cells = caret_row(app, layout, pi);
    let borders_page: Vec<f32> =
        row_cells.first().map(|f| std::iter::once(f.x).chain(row_cells.iter().map(|b| b.right())).collect()).unwrap_or_default();
    // While a column marker is dragged the markers show where the preview puts their borders (the ruler draws them
    // where it is told), so the grabbed marker follows the pointer and the ones right of it move along.
    let mods = crate::table_resize::mods_now(ui.ctx());
    let borders: Vec<f32> = borders_page
        .iter()
        .enumerate()
        .map(|(line, &x)| to_start(app.canvas.table_resize.as_ref().and_then(|d| d.ruler_line_x(mods, line)).unwrap_or(x)))
        .collect();
    let row_y = row_cells.first().map_or(0.0, |b| b.y + b.h / 2.0);
    let bottoms = row_bottoms(app, layout, pi);
    let rows: Vec<f32> = bottoms.iter().map(|(_, b)| b.bottom()).collect();
    // A section whose columns `column_drag` refuses shows its gaps without grips.
    let gaps_draggable = cg.as_ref().is_none_or(|cg| super::column_ruler::draggable(&cg.sect));

    let mut margins = Margins::uniform(0.0);
    margins.top = f64::from(sect.margin_top);
    margins.bottom = f64::from(sect.margin_bottom);
    margins.left = f64::from(sect.margin_left);
    margins.right = f64::from(sect.margin_right);
    let edges = PageEdges::new(margins);
    let mirrored = app.session.doc.settings.mirror_margins && page.number.is_multiple_of(2);
    let unit = DocUnit::new(unit_kind(app.session.units()), 96.0);
    let kind = ruler_align(app.canvas.tab_kind).unwrap_or_default();

    let output = PageRulers::new(tl!("Rulers"), placed, Bind::given(edges), Bind::given(values.clone()))
        .id_salt(("page_rulers", format!("{:?}{:?}", focus.story, focus.path)))
        .column((col_x, col_end))
        .unit(unit)
        .kind(Some(kind))
        .right_to_left(Some(rtl))
        .columns(&borders)
        .gaps(&gaps)
        .gaps_draggable(gaps_draggable)
        .rows(&rows)
        .gutter(f64::from(sect.gutter))
        .mirrored_even(mirrored)
        .show_at(ui, canvas_rect);

    if let Some(k) = output.ruler.kind {
        app.canvas.tab_kind = doc_align(k);
    }
    if output.ruler.open_tabs_dialog {
        let _ = app.run("para.tabs", json!({}));
    }
    drive(app, &output.ruler.edit.events, |app, id, v, last| {
        if let Some(p) = indent_params(&values, v, col_x, w) {
            live(app, id, |a| {
                let _ = a.run("para.indents", p);
            });
        }
        if last && let Some(p) = tab_commands(&doc_tabs, &doc_stops(&v.stops)) {
            let _ = app.run("para.tabs", p);
        }
    });
    drive(app, &output.margins.edit.events, |app, id, v, _| {
        if let Some(p) = margin_params(&edges, v) {
            live(app, id, |a| {
                let _ = a.run_keep_view("layout.margins", p);
            });
        }
    });

    if let Some(g) = output.ruler.gap
        && let Some(cg) = &cg
    {
        match g.phase {
            GesturePhase::Begin => settle(app, false),
            GesturePhase::Cancel => settle(app, true),
            phase => {
                let from_text_start = to_page(g.position) - cg.origin;
                let at = if cg.sect.rtl { cg.sect.text_width() - from_text_start } else { from_text_start };
                if let Some(p) = super::column_ruler::column_drag(&cg.sect, g.index, g.edge.into(), at) {
                    live(app, Some(GAP_GESTURE), |a| {
                        let _ = a.run_keep_view("layout.columns", p);
                    });
                }
                if phase == GesturePhase::Commit {
                    settle(app, false);
                }
            }
        }
    }

    if let Some(d) = output.ruler.column {
        let at = borders_page.get(d.border.index).map(|&x| (x, row_y));
        let to = (to_page(d.border.position), row_y);
        table_drag(app, layout, pi, BorderDrag { phase: d.phase, edge: Edge::Column(d.border.index), at, to, mods: d.mods });
    }
    if let Some(d) = output.row {
        let at = bottoms.get(d.index).map(|(_, b)| (b.x + b.w / 2.0, b.bottom()));
        let edge = bottoms.get(d.index).map(|(r, _)| Edge::Row(*r));
        if let (Some(edge), Some(at)) = (edge, at) {
            table_drag(app, layout, pi, BorderDrag { phase: d.phase, edge, at: Some(at), to: (at.0, d.position), mods: d.mods });
        }
    }
}

/// One step of a table border drag as the ruler reports it: the border's edge, where it was grabbed (`None` when the
/// border is no longer in the table) and where the pointer puts it.
struct BorderDrag {
    phase: GesturePhase,
    edge: Edge,
    at: Option<(f32, f32)>,
    to: (f32, f32),
    mods: craft::keymap::Mods,
}

/// One table border drag: the preview follows each step, and the commit runs `table.resizeDrag` once. A press and
/// release without a move changes nothing, and a cancel drops the preview.
fn table_drag(app: &mut WordApp, layout: &DocLayout, page: usize, drag: BorderDrag) {
    let BorderDrag { phase, edge, at, to, mods } = drag;
    let Some(at) = at else { return };
    match phase {
        GesturePhase::Begin => {}
        GesturePhase::Cancel => app.canvas.table_resize = None,
        phase => {
            // PLAN: assumed `ruler_params(app: &mut WordApp, layout: &DocLayout, page: usize, edge: Edge, at: (f32, f32), to: (f32, f32), mods: Mods) -> Option<Value>` (canvas-w13)
            let params = ruler_params(app, layout, page, edge, at, to, mods);
            if phase == GesturePhase::Commit {
                app.canvas.table_resize = None;
                let moved = (to.0 - at.0).abs() >= 0.5 || (to.1 - at.1).abs() >= 0.5;
                if moved
                    && let Some(p) = params
                    && let Err(e) = app.run_keep_view("table.resizeDrag", p)
                {
                    app.status(e);
                }
            }
        }
    }
}

/// Plays the events of a ruler's edit: a begin or cancel settles the drag (a cancel takes back what ran), any other
/// event is a step with its value and gesture, and `last` marks the one that ends the gesture (a commit or a single
/// set).
fn drive<T>(app: &mut WordApp, events: &EditEvents<T>, mut step: impl FnMut(&mut WordApp, Option<GestureId>, &T, bool)) {
    for ev in events.iter() {
        match ev.phase() {
            Some(GesturePhase::Begin) => settle(app, false),
            Some(GesturePhase::Cancel) => settle(app, true),
            phase => {
                let Some(v) = ev.value() else { continue };
                let last = phase != Some(GesturePhase::Update);
                step(app, ev.id(), v, last);
                if last {
                    settle(app, false);
                }
            }
        }
    }
}

/// Runs one step of a drag; a step of the gesture whose first step already ran joins its undo step, and a step of
/// another gesture starts afresh.
fn live(app: &mut WordApp, id: Option<GestureId>, run: impl FnOnce(&mut WordApp)) {
    if let Some(id) = id {
        if app.canvas.ruler_gesture == Some(id) {
            app.session.join_next_undo();
        }
        app.canvas.ruler_gesture = Some(id);
    }
    run(app);
}

/// Ends a drag; `undo` takes back the steps it ran.
fn settle(app: &mut WordApp, undo: bool) {
    if app.canvas.ruler_gesture.take().is_some() && undo {
        app.session.undo();
    }
}

/// `para.indents` params for the indents `v` moved from `shown`, snapped to 1/16 inch; the left indent stays within
/// the column and leaves room for text.
fn indent_params(shown: &RulerValues, v: &RulerValues, col_x: f32, w: f32) -> Option<Value> {
    let moved = |a: f32, b: f32| (a - b).abs() > 1e-3;
    let snap = |x: f32| (x / INDENT_SNAP).round() * INDENT_SNAP;
    let mut p = Map::new();
    if moved(shown.left_indent, v.left_indent) {
        let most = (w - MIN_INDENT_ROOM).max(-col_x);
        p.insert("left".into(), Value::from(snap(v.left_indent.min(most).max(-col_x))));
    }
    if moved(shown.first_line_indent, v.first_line_indent) {
        p.insert("firstLine".into(), Value::from(snap(v.first_line_indent)));
    }
    if moved(shown.right_indent, v.right_indent) {
        p.insert("right".into(), Value::from(snap(v.right_indent)));
    }
    (!p.is_empty()).then_some(Value::Object(p))
}

/// `layout.margins` params for the margins `v` moved from `shown`.
fn margin_params(shown: &PageEdges, v: &PageEdges) -> Option<Value> {
    let (a, b) = (&shown.margins, &v.margins);
    let mut p = Map::new();
    for (key, was, now) in [("left", a.left, b.left), ("right", a.right, b.right), ("top", a.top, b.top), ("bottom", a.bottom, b.bottom)] {
        if now.is_finite() && (was - now).abs() > 1e-3 {
            p.insert(key.into(), Value::from(now));
        }
    }
    (!p.is_empty()).then_some(Value::Object(p))
}

/// The `para.tabs` params that turn the stops `before` into `after`: the stops that are gone are cleared and the new
/// ones set, so a moved stop is a clear and a set that keep its kind and leader. `None` when nothing changed.
pub(crate) fn tab_commands(before: &[TabStop], after: &[TabStop]) -> Option<Value> {
    let same = |a: &TabStop, b: &TabStop| (a.pos - b.pos).abs() < TAB_POS_EPS && a.align == b.align && a.leader == b.leader;
    let cleared: Vec<f32> = before.iter().filter(|b| !after.iter().any(|a| same(a, b))).map(|b| b.pos).collect();
    let set: Vec<TabStop> = after.iter().filter(|a| !before.iter().any(|b| same(a, b))).copied().collect();
    let mut p = Map::new();
    if !cleared.is_empty() {
        p.insert("clear".into(), json!(cleared));
    }
    if !set.is_empty() {
        p.insert("set".into(), json!(set));
    }
    (!p.is_empty()).then_some(Value::Object(p))
}

/// The document's stops for the ruler's.
fn doc_stops(stops: &[RulerStop]) -> Vec<TabStop> {
    stops.iter().map(|s| TabStop { pos: s.position, align: doc_align(s.align), leader: doc_leader(s.leader) }).collect()
}

/// The ruler's kind of stop for the document's; a clearing stop is not shown.
fn ruler_align(a: TabAlign) -> Option<RulerAlign> {
    match a {
        TabAlign::Left => Some(RulerAlign::Left),
        TabAlign::Center => Some(RulerAlign::Center),
        TabAlign::Right => Some(RulerAlign::Right),
        TabAlign::Decimal => Some(RulerAlign::Decimal),
        TabAlign::Bar => Some(RulerAlign::Bar),
        TabAlign::Clear => None,
    }
}

/// The document's kind of stop for the ruler's.
fn doc_align(a: RulerAlign) -> TabAlign {
    match a {
        RulerAlign::Left => TabAlign::Left,
        RulerAlign::Center => TabAlign::Center,
        RulerAlign::Right => TabAlign::Right,
        RulerAlign::Decimal => TabAlign::Decimal,
        RulerAlign::Bar => TabAlign::Bar,
        _ => TabAlign::Left,
    }
}

/// The ruler's leader for the document's.
fn ruler_leader(l: TabLeader) -> RulerLeader {
    match l {
        TabLeader::None => RulerLeader::None,
        TabLeader::Dot => RulerLeader::Dot,
        TabLeader::Hyphen => RulerLeader::Hyphen,
        TabLeader::Underscore => RulerLeader::Underscore,
        TabLeader::MiddleDot => RulerLeader::MiddleDot,
    }
}

/// The document's leader for the ruler's.
fn doc_leader(l: RulerLeader) -> TabLeader {
    match l {
        RulerLeader::None => TabLeader::None,
        RulerLeader::Dot => TabLeader::Dot,
        RulerLeader::Hyphen => TabLeader::Hyphen,
        RulerLeader::Underscore => TabLeader::Underscore,
        RulerLeader::MiddleDot => TabLeader::MiddleDot,
        _ => TabLeader::None,
    }
}

/// The unit the rulers number in, from the File › Options unit.
fn unit_kind(u: wordcraft_geom::Unit) -> UnitKind {
    use wordcraft_geom::Unit;
    match u {
        Unit::Inches => UnitKind::Inches,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Points => UnitKind::Points,
        Unit::Picas => UnitKind::Picas,
    }
}

/// The cells of the caret's table drawn on `page`: (row, cell, box in page points).
fn cells(app: &WordApp, layout: &DocLayout, page: usize) -> Vec<(usize, usize, wordcraft_geom::Rect)> {
    let story = app.session.sel.focus.story;
    let (Some((tp, _, _)), Some(pg)) = (app.session.sel.focus.path.cell(), layout.pages.get(page)) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Cell { rect, table, row, cell, story: st, .. } if *table == tp && *st == story => Some((*row, *cell, *rect)),
            _ => None,
        })
        .collect()
}

/// The caret cell's text area on `page`, as (left edge, width) in page points: its box less its left and right
/// margins. `None` when the caret isn't in a table cell drawn there.
pub(crate) fn text_area(app: &WordApp, layout: &DocLayout, page: usize) -> Option<(f32, f32)> {
    let story = app.session.sel.focus.story;
    let (tp, r, c) = app.session.sel.focus.path.cell()?;
    let (.., b) = cells(app, layout, page).into_iter().find(|(row, cell, _)| *row == r && *cell == c)?;
    let m = wordcraft_layout::cell_margins(&app.session.doc, app.session.doc.table(story, &tp)?, r, c);
    let (left, right) = (m[1].max(0.0), m[3].max(0.0));
    Some((b.x + left, (b.w - left - right).max(0.0)))
}

/// The caret row's cells on `page`, left to right.
fn caret_row(app: &WordApp, layout: &DocLayout, page: usize) -> Vec<wordcraft_geom::Rect> {
    let Some((_, r, _)) = app.session.sel.focus.path.cell() else { return Vec::new() };
    let mut row: Vec<wordcraft_geom::Rect> = cells(app, layout, page).into_iter().filter(|(row, ..)| *row == r).map(|(.., b)| b).collect();
    row.sort_by(|a, b| a.x.total_cmp(&b.x));
    row
}

/// Each row of the caret's table on `page` with the cell its bottom is taken from: its shortest box, as a vertically
/// merged cell's box spans all its rows, and of those the leftmost.
fn row_bottoms(app: &WordApp, layout: &DocLayout, page: usize) -> Vec<(usize, wordcraft_geom::Rect)> {
    let mut rows: Vec<(usize, wordcraft_geom::Rect)> = Vec::new();
    for (r, _, b) in cells(app, layout, page) {
        match rows.iter_mut().find(|(row, _)| *row == r) {
            Some((_, first)) if b.bottom() < first.bottom() - 0.01 || (b.bottom() <= first.bottom() + 0.01 && b.x < first.x) => *first = b,
            Some(_) => {}
            None => rows.push((r, b)),
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(pos: f32, align: TabAlign, leader: TabLeader) -> TabStop {
        TabStop { pos, align, leader }
    }

    /// A finished ruler gesture is one `para.tabs` call: set, move (keeping kind and leader), dropped where it was,
    /// and dragged off.
    #[test]
    fn gestures_become_one_para_tabs_call() {
        let left = stop(36.0, TabAlign::Left, TabLeader::None);
        let right = stop(36.0, TabAlign::Right, TabLeader::Dot);
        // Set.
        assert_eq!(tab_commands(&[], &[left]), Some(json!({ "set": [left] })));
        // Moved: cleared where it was and set again with its kind and leader.
        let moved = stop(72.0, TabAlign::Right, TabLeader::Dot);
        assert_eq!(tab_commands(&[right], &[moved]), Some(json!({ "clear": [36.0], "set": [moved] })));
        // Dropped where it was: nothing.
        assert_eq!(tab_commands(&[right], &[right]), None);
        assert_eq!(tab_commands(&[right], &[stop(36.2, TabAlign::Right, TabLeader::Dot)]), None);
        // Dragged off the ruler: cleared.
        assert_eq!(tab_commands(&[right], &[]), Some(json!({ "clear": [36.0] })));
        // A stop placed and taken off again in one gesture: nothing.
        assert_eq!(tab_commands(&[], &[]), None);
    }
}
