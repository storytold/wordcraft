//! Tab stops on the horizontal ruler, edited like Word's: the selector in the rulers' corner picks
//! the kind of stop, a click on the ruler sets one there, dragging a stop moves it, dragging it off
//! the ruler clears it, and double-clicking one opens the Tabs dialog. A gesture runs `para.tabs`
//! once when the button is released, so it is one undo step.
use egui::{Painter, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::props::{TAB_POS_EPS, TabAlign, TabLeader, TabStop};

use crate::WordApp;
use crate::theme::Tokens;

/// How far (screen points) past the ruler's edge the pointer takes a dragged stop off it.
const DROP_OFF: f32 = 18.0;

/// The kinds of stop the corner selector cycles through (and the Tabs dialog lists), in Word's order.
pub(crate) const KINDS: [TabAlign; 5] = [TabAlign::Left, TabAlign::Center, TabAlign::Right, TabAlign::Decimal, TabAlign::Bar];

/// A tab stop being placed or dragged on the ruler.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabDrag {
    /// Where the stop was before the drag (points from the paragraph's start edge); `None` for a
    /// stop being placed.
    pub from: Option<f32>,
    /// The caret paragraph the gesture started on: it is abandoned if the caret leaves it
    /// (the keyboard still works mid-drag), rather than applied to another paragraph.
    pub para: Pos,
    pub align: TabAlign,
    pub leader: TabLeader,
    /// Where it lands if the button is released now.
    pub at: f32,
    /// The pointer has left the ruler: releasing clears the stop.
    pub off: bool,
}

/// The selector's next kind (`back`: the previous one).
pub(crate) fn next_kind(k: TabAlign, back: bool) -> TabAlign {
    let n = KINDS.len();
    let i = KINDS.iter().position(|x| *x == k).unwrap_or(0);
    let j = if back { (i + n - 1) % n } else { (i + 1) % n };
    KINDS.get(j).copied().unwrap_or(TabAlign::Left)
}

/// The kind's name, as the Tabs dialog labels it (already translated there).
pub(crate) fn kind_label(k: TabAlign) -> &'static str {
    match k {
        TabAlign::Left => "Left",
        TabAlign::Center => "Center",
        TabAlign::Right => "Right",
        TabAlign::Decimal => "Decimal",
        TabAlign::Bar => "Bar",
        TabAlign::Clear => "Clear",
    }
}

/// A ruler position for a stop: snapped to `step` (unless `free`, Alt held), within the column.
pub(crate) fn snap_tab(pt: f32, step: f32, width: f32, free: bool) -> f32 {
    let pt = if pt.is_finite() { pt } else { 0.0 };
    let snapped = if free || !step.is_finite() || step <= 0.0 { pt } else { (pt / step).round() * step };
    snapped.clamp(0.0, if width.is_finite() { width.max(0.0) } else { 0.0 })
}

/// Whether a pointer at `y` is far enough above or below the ruler `strip` to take a stop off.
pub(crate) fn dropped_off(y: f32, strip: Rect) -> bool {
    y > strip.max.y + DROP_OFF || y < strip.min.y - DROP_OFF
}

/// The finished gesture as `para.tabs` params, or `None` when it changes nothing.
pub(crate) fn release_params(d: &TabDrag) -> Option<Value> {
    let stop = json!([TabStop { pos: d.at, align: d.align, leader: d.leader }]);
    match (d.from, d.off) {
        (None, true) => None,
        (None, false) => Some(json!({ "set": stop })),
        (Some(from), true) => Some(json!({ "clear": [from] })),
        (Some(from), false) if (from - d.at).abs() < TAB_POS_EPS => None,
        (Some(from), false) => Some(json!({ "clear": [from], "set": stop })),
    }
}

/// Draw a stop's marker standing on `base` at `x`; right-to-left paragraphs mirror its foot.
pub(crate) fn draw_tab_glyph(p: &Painter, x: f32, base: f32, kind: TabAlign, rtl: bool, stroke: Stroke) {
    let foot = if rtl { -4.0 } else { 4.0 };
    let stem = |top: f32| {
        p.line_segment([pos2(x, top), pos2(x, base)], stroke);
    };
    let along = |a: f32, b: f32| {
        p.line_segment([pos2(x + a, base), pos2(x + b, base)], stroke);
    };
    match kind {
        TabAlign::Left => {
            stem(base - 5.0);
            along(0.0, foot);
        }
        TabAlign::Right => {
            stem(base - 5.0);
            along(-foot, 0.0);
        }
        TabAlign::Center => {
            stem(base - 5.0);
            along(-4.0, 4.0);
        }
        TabAlign::Decimal => {
            stem(base - 5.0);
            along(-4.0, 4.0);
            p.circle_filled(pos2(x + foot * 0.7, base - 3.5), 1.1, stroke.color);
        }
        TabAlign::Bar => {
            stem(base - 7.0);
        }
        TabAlign::Clear => {}
    }
}

/// The tab selector in the corner where the rulers meet: click for the next kind of stop,
/// right-click for the previous one.
pub(crate) fn selector(app: &mut WordApp, ui: &Ui, corner: Rect) {
    if corner.width() < 12.0 || corner.height() < 12.0 {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let r = ui.interact(corner, ui.scope_id().with("ruler_tab_kind"), Sense::click());
    if r.clicked() {
        app.canvas.tab_kind = next_kind(app.canvas.tab_kind, false);
    } else if r.secondary_clicked() {
        app.canvas.tab_kind = next_kind(app.canvas.tab_kind, true);
    }
    let kind = app.canvas.tab_kind;
    let p = ui.painter_at(corner);
    let b = Rect::from_center_size(corner.center(), vec2(14.0, 14.0));
    p.rect(b, 2.0, if r.hovered() { t.hover } else { t.ruler }, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
    // Centre the glyph on its box: Left's foot runs right of its stem, Right's left.
    let nudge = match kind {
        TabAlign::Left => -2.0,
        TabAlign::Right => 2.0,
        _ => 0.0,
    };
    draw_tab_glyph(&p, b.center().x + nudge, b.max.y - 4.0, kind, false, Stroke::new(1.5, t.text));
    r.on_hover_text(format!("{}: {}", tl!("Tabs"), tl!(kind_label(kind))));
}

/// Where the stops' glyphs sit on the ruler `bar`, for its numbers to keep clear of (#1878):
/// each of `tabs`, and a stop being dragged where it would land instead.
pub(crate) fn marker_rects(app: &WordApp, bar: Rect, x_of: &dyn Fn(f32) -> f32, tabs: &[TabStop]) -> Vec<Rect> {
    let base = bar.max.y - 1.0;
    let glyph = |x: f32| Rect::from_min_max(pos2(x - 4.5, base - 7.5), pos2(x + 4.5, base + 0.5));
    let drag = app.canvas.tab_drag.as_ref();
    let moving = drag.and_then(|d| d.from);
    let mut rects: Vec<Rect> =
        tabs.iter().filter(|t| moving.is_none_or(|from| (from - t.pos).abs() > TAB_POS_EPS)).map(|t| glyph(x_of(t.pos))).collect();
    if let Some(d) = drag.filter(|d| !d.off) {
        rects.push(glyph(x_of(d.at)));
    }
    rects
}

/// The ruler geometry the stops are drawn on.
pub(crate) struct Ruler<'a> {
    /// The ruler's strip.
    pub bar: Rect,
    /// The part of the strip over the text column, where a click sets a stop.
    pub text: Rect,
    /// The column's width (points): stops stay within it.
    pub width: f32,
    /// Snap step (points).
    pub step: f32,
    pub rtl: bool,
    /// Screen x of a distance (points) from the paragraph's start edge, and back.
    pub x_of: &'a dyn Fn(f32) -> f32,
    pub dist_at: &'a dyn Fn(f32) -> f32,
}

/// Draw the caret paragraph's stops and handle placing, moving, clearing and double-clicking them.
/// Register before the indent markers, so those keep the pointer where they overlap a stop.
pub(crate) fn stops(app: &mut WordApp, ui: &Ui, hp: &Painter, g: &Ruler<'_>, tabs: &[TabStop]) {
    let t = Tokens::get(ui.ctx());
    let stroke = Stroke::new(1.5, t.text);
    let base = g.bar.max.y - 1.0;
    let free = ui.input(|i| i.modifiers.alt);
    let snap = |x: f32| snap_tab((g.dist_at)(x), g.step, g.width, free);
    let units = app.session.units();
    let decimal = crate::options::decimal();

    // The paragraph the ruler is editing; a drag that outlives it is abandoned.
    let para = app.session.sel.focus.clone();

    // A click on an empty stretch over the text sets a stop of the selector's kind there; pressing
    // and dragging places it where the button is released. The edges stay with the margin handles
    // and indent markers, and only the primary button places stops.
    let add = ui.interact(g.text.shrink2(vec2(4.0, 0.0)), ui.scope_id().with("ruler_tab_add"), Sense::click_and_drag());
    let kind = app.canvas.tab_kind;
    let placing = |x: f32| TabDrag { from: None, para: para.clone(), align: kind, leader: TabLeader::None, at: snap(x), off: false };
    if add.clicked()
        && let Some(pp) = add.interact_pointer_pos()
        && let Some(params) = release_params(&placing(pp.x))
    {
        let _ = app.run("para.tabs", params);
    } else if add.drag_started_by(egui::PointerButton::Primary)
        && let Some(pp) = add.interact_pointer_pos()
    {
        app.canvas.tab_drag = Some(placing(pp.x));
    }

    let dragging = app.canvas.tab_drag.as_ref().and_then(|d| d.from);
    let mut open_dialog = false;
    for (i, tab) in tabs.iter().enumerate() {
        let x = (g.x_of)(tab.pos);
        let r = ui.interact(
            Rect::from_center_size(pos2(x, g.bar.max.y - 4.0), vec2(9.0, 10.0)),
            ui.scope_id().with(("ruler_tab", i)),
            Sense::click_and_drag(),
        );
        let starts = r.drag_started_by(egui::PointerButton::Primary);
        if starts {
            app.canvas.tab_drag =
                Some(TabDrag { from: Some(tab.pos), para: para.clone(), align: tab.align, leader: tab.leader, at: tab.pos, off: false });
        }
        // The stop being dragged is drawn where it would land instead (from this frame on).
        if !starts && dragging.is_none_or(|from| (from - tab.pos).abs() > TAB_POS_EPS) {
            draw_tab_glyph(hp, x, base, tab.align, g.rtl, stroke);
        }
        open_dialog |= r.double_clicked();
        if !starts && app.canvas.tab_drag.is_none() {
            r.on_hover_text(format!("{} {}", tl!(kind_label(tab.align)), units.format_with(tab.pos, decimal)));
        }
    }

    if let Some(mut d) = app.canvas.tab_drag.clone() {
        let (esc, down, released) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.pointer.primary_down(), i.pointer.primary_released()));
        if esc || d.para.story != para.story || d.para.path != para.path {
            // Escape cancels (like a table-border drag, #217), and a drag whose paragraph the
            // caret has left is abandoned, never applied to the new one.
            app.canvas.tab_drag = None;
        } else if down {
            if let Some(pp) = ui.input(|i| i.pointer.interact_pos()) {
                d.at = snap(pp.x);
                d.off = dropped_off(pp.y, g.bar);
            }
            app.canvas.tab_drag = Some(d.clone());
            let label = if d.off {
                tl!("Clear").to_string()
            } else {
                let x = (g.x_of)(d.at);
                draw_tab_glyph(hp, x, base, d.align, g.rtl, stroke);
                // A guide down the page shows where text will line up.
                if let Some(area) = app.canvas.canvas_rect {
                    let guide = ui
                        .ctx()
                        .layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("ruler_tab_guide")))
                        .with_clip_rect(area);
                    guide.extend(Shape::dashed_line(&[pos2(x, area.min.y), pos2(x, area.max.y)], Stroke::new(1.0, t.text_dim), 3.0, 3.0));
                }
                format!("{} {}", tl!(kind_label(d.align)), units.format_with(d.at, decimal))
            };
            egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), ui.scope_id().with("ruler_tab_tip"), egui::PopupAnchor::Pointer).show(|ui| {
                ui.label(label);
            });
        } else {
            app.canvas.tab_drag = None;
            // Only a release seen on this frame applies; one missed while the ruler wasn't
            // drawn (a view change mid-drag) is dropped rather than fired late.
            if released && let Some(params) = release_params(&d) {
                let _ = app.run("para.tabs", params);
            }
        }
    }
    if open_dialog {
        let _ = app.run("para.tabs", json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selector_cycles_through_the_kinds_both_ways() {
        let mut k = TabAlign::Left;
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(k);
            k = next_kind(k, false);
        }
        assert_eq!(seen, KINDS);
        assert_eq!(k, TabAlign::Left, "it wraps around");
        assert_eq!(next_kind(TabAlign::Left, true), TabAlign::Bar);
        assert_eq!(next_kind(TabAlign::Clear, false), TabAlign::Center, "a stray kind restarts the cycle");
    }

    #[test]
    fn stops_snap_and_stay_in_the_column() {
        assert_eq!(snap_tab(100.0, 4.5, 468.0, false), 99.0);
        assert_eq!(snap_tab(100.0, 4.5, 468.0, true), 100.0, "Alt places freely");
        assert_eq!(snap_tab(-20.0, 4.5, 468.0, false), 0.0);
        assert_eq!(snap_tab(900.0, 4.5, 468.0, false), 468.0);
        assert_eq!(snap_tab(f32::NAN, 4.5, 468.0, false), 0.0, "junk lands at the start");
        assert_eq!(snap_tab(50.0, 0.0, 468.0, false), 50.0, "no step, no snapping");
    }

    #[test]
    fn a_stop_comes_off_only_well_away_from_the_ruler() {
        let strip = Rect::from_min_max(pos2(0.0, 100.0), pos2(800.0, 115.0));
        assert!(!dropped_off(110.0, strip));
        assert!(!dropped_off(115.0 + DROP_OFF, strip), "just below still keeps it");
        assert!(dropped_off(116.0 + DROP_OFF, strip));
        assert!(dropped_off(100.0 - DROP_OFF - 1.0, strip));
    }

    #[test]
    fn gestures_become_one_para_tabs_call() {
        let d = |from, at, off| TabDrag { from, para: Pos::body(0, 0), align: TabAlign::Center, leader: TabLeader::Dot, at, off };
        assert_eq!(release_params(&d(None, 72.0, false)), Some(json!({"set": [{"pos": 72.0, "align": "center", "leader": "dot"}]})));
        assert_eq!(release_params(&d(None, 72.0, true)), None, "a new stop dragged off is never set");
        assert_eq!(release_params(&d(Some(72.0), 72.2, false)), None, "a stop dropped where it was");
        assert_eq!(release_params(&d(Some(72.0), 144.0, true)), Some(json!({"clear": [72.0]})));
        assert_eq!(
            release_params(&d(Some(72.0), 144.0, false)),
            Some(json!({"clear": [72.0], "set": [{"pos": 144.0, "align": "center", "leader": "dot"}]})),
            "moving keeps the stop's kind and leader"
        );
    }

    /// #1878: the ruler's numbers keep clear of each stop's glyph, and of a dragged stop where it
    /// lands rather than where it was; a stop dragged off the ruler frees its number.
    #[test]
    fn marker_rects_cover_each_stop_and_a_dragged_one_where_it_lands() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        let bar = Rect::from_min_max(pos2(0.0, 100.0), pos2(800.0, 115.0));
        let x_of = |pos: f32| 100.0 + pos;
        let stop = |pos: f32| TabStop { pos, align: TabAlign::Left, leader: TabLeader::None };
        let tabs = [stop(72.0), stop(144.0)];
        let centres = |rects: &[Rect]| rects.iter().map(|r| r.center().x).collect::<Vec<f32>>();
        let rects = marker_rects(&a, bar, &x_of, &tabs);
        assert_eq!(centres(&rects), [172.0, 244.0]);
        assert!(rects.iter().all(|r| r.width() == 9.0 && r.min.y == bar.max.y - 8.5 && r.max.y == bar.max.y - 0.5), "{rects:?}");
        let para = a.session.sel.focus.clone();
        let drag = |from, at, off| Some(TabDrag { from, para: para.clone(), align: TabAlign::Left, leader: TabLeader::None, at, off });
        a.canvas.tab_drag = drag(Some(72.0), 216.0, false);
        assert_eq!(centres(&marker_rects(&a, bar, &x_of, &tabs)), [244.0, 316.0], "a dragged stop counts where it lands");
        a.canvas.tab_drag = drag(Some(72.0), 216.0, true);
        assert_eq!(centres(&marker_rects(&a, bar, &x_of, &tabs)), [244.0], "off the ruler it counts nowhere");
        a.canvas.tab_drag = drag(None, 216.0, false);
        assert_eq!(centres(&marker_rects(&a, bar, &x_of, &tabs)), [172.0, 244.0, 316.0], "a stop being placed counts too");
    }
}
