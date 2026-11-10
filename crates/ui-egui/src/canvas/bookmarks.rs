//! View › Show › Bookmarks: the marks Word puts at the start and end of every bookmark (screen
//! only — the layout, the print markup and every export ignore them). Names a program made for
//! itself (`_Toc…`, `_Ref…`, `_GoBack`) are hidden, as in Word's own list. Resting the pointer on
//! a mark names the bookmark, the way Word's own marks do.

use egui::{Pos2, Rect, Stroke, pos2, vec2};
use wordcraft_doc::BookmarkRange;

use super::{DocLayout, WordApp};
use crate::theme::Tokens;

/// Most marks drawn on one walk (a template-heavy or hostile file).
const MAX_MARKS: usize = 400;

/// One mark's screen rect: the page it is on mapped onto the canvas.
fn spot(app: &WordApp, layout: &DocLayout, rects: &[Rect], scale: f32, r: &BookmarkRange, at_end: bool) -> Option<Rect> {
    let at = if at_end { &r.end } else { &r.start };
    let c = layout.caret_on(at, app.session.page_hint)?;
    let pr = rects.get(c.page)?;
    Some(Rect::from_min_size(pos2(pr.min.x + c.x * scale, pr.min.y + c.top * scale), vec2(0.0, c.height.max(1.0) * scale)))
}

/// The caret sits inside this bookmark's range. A walk never closes a bookmark before it opens
/// one, so `start` is always at or before `end`.
fn holds(r: &BookmarkRange, caret: &wordcraft_doc::Pos) -> bool {
    r.start <= *caret && *caret <= r.end
}

/// The strip a mark's name is read from: the rule is two pixels wide, so the pointer gets the
/// cap's own overhang to find it.
fn zone(s: &Rect) -> Rect {
    s.expand2(vec2(3.0, 0.0))
}

/// Name the bookmark the pointer rests on. The area is deliberately not interactive: an
/// `ui.interact` hover would take the pointer off the canvas's own response, and the canvas needs
/// it to choose the caret cursor and to draw the I-beam (#1746).
fn tip(ui: &egui::Ui, at: Pos2, name: &str) {
    egui::Area::new(egui::Id::unique("bookmark_mark_tip"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(at + vec2(6.0, -2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                // Unwrapped: the area keeps last frame's width, too narrow for a longer name.
                ui.add(egui::Label::new(name).extend());
            });
        });
}

/// Draw the marks, inside the canvas viewport after the selection and the content controls.
/// `hover` is the pointer, when the canvas owns it.
pub(super) fn paint(
    app: &mut WordApp,
    ui: &egui::Ui,
    painter: &egui::Painter,
    t: &Tokens,
    layout: &DocLayout,
    rects: &[Rect],
    scale: f32,
    hover: Option<Pos2>,
) {
    if !app.session.view.show_bookmarks || app.session.view.read_mode {
        return;
    }
    let caret = app.session.sel.focus.clone();
    let mut named = false;
    for r in app.session.doc.bookmark_ranges().into_iter().filter(|r| r.visible()).take(MAX_MARKS) {
        // The one the caret is in stands out; the rest stay quiet so a form of 60 fields is
        // still readable.
        let ink = if holds(&r, &caret) { t.accent } else { t.accent.gamma_multiply(0.55) };
        for at_end in [false, true] {
            let Some(s) = spot(app, layout, rects, scale, &r, at_end) else { continue };
            // Word's mark is an I-beam: a vertical rule with a short serif at each end.
            let stroke = Stroke::new(1.2, ink);
            let cap = 3.5_f32.min(s.height() / 3.0);
            painter.line_segment([pos2(s.min.x, s.min.y), pos2(s.min.x, s.max.y)], stroke);
            for y in [s.min.y, s.max.y] {
                painter.line_segment([pos2(s.min.x - cap, y), pos2(s.min.x + cap, y)], stroke);
            }
            // One name a frame, the one nearest the start when two marks share a spot.
            if !named && hover.is_some_and(|p| zone(&s).contains(p)) {
                named = true;
                tip(ui, pos2(s.min.x, s.min.y), &r.name);
            }
        }
    }
}
