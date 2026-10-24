//! The window's caption buttons and resize edges for frameless windows (Linux, where the app's
//! own title bar replaces the native one, as Office's does on Windows): Minimize, Maximize (or
//! Restore) and Close at the title bar's right end, and thin strips along the window's edges
//! that resize it. Close sends `ViewportCommand::Close`, so the app's save prompt still runs
//! (the same path the native close button takes).

use egui::viewport::ResizeDirection;
use egui::{Context, CursorIcon, Order, Rect, Sense, StrokeKind, Ui, ViewportCommand, pos2, vec2};

use crate::WordApp;
use crate::theme::Tokens;

/// A caption button's width; the close box is a little wider, as on Windows.
const WIDTH: f32 = 40.0;
const CLOSE_WIDTH: f32 = 44.0;
/// How far a glyph reaches from its centre (the ✕, the line, the box).
const GLYPH: f32 = 4.5;
/// The thickness of the invisible resize strips along the window's edges.
const EDGE: f32 = 6.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Minimize,
    Maximize,
    Close,
}

/// The caption buttons of a frameless window's title bar, drawn flush to its right edge; the
/// caller places them first in the bar's right-to-left row.
pub(crate) fn draw(ui: &mut Ui, bar_h: f32) {
    let t = Tokens::get(ui.ctx());
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    for kind in [Kind::Minimize, Kind::Maximize, Kind::Close] {
        let w = if kind == Kind::Close { CLOSE_WIDTH } else { WIDTH };
        let (r, resp) = ui.allocate_exact_size(vec2(w, bar_h), Sense::click());
        let down = resp.is_pointer_button_down_on();
        let hot = down || resp.hovered();
        if hot {
            let fill = match kind {
                Kind::Close if down => t.close_pressed,
                Kind::Close => t.close_hover,
                _ if down => t.pressed,
                _ => t.hover,
            };
            ui.painter().rect_filled(r, 0.0, fill);
        }
        let c = r.center();
        let stroke = egui::Stroke::new(
            1.0,
            if kind == Kind::Close && hot {
                t.on_close
            } else if hot {
                t.text
            } else {
                t.text_dim
            },
        );
        match kind {
            Kind::Minimize => {
                ui.painter().line_segment([pos2(c.x - GLYPH, c.y + GLYPH), pos2(c.x + GLYPH, c.y + GLYPH)], stroke);
            }
            Kind::Maximize if !maximized => {
                ui.painter().rect_stroke(centre_box(c, 0.0), 1.5, stroke, StrokeKind::Inside);
            }
            Kind::Maximize => {
                // Restore: the front box, then the back one behind its top-right corner.
                ui.painter().rect_stroke(centre_box(c, 0.0), 1.5, stroke, StrokeKind::Inside);
                let back = centre_box(c, 0.0).translate(vec2(2.0, -2.0));
                ui.painter().line_segment([pos2(back.min.x, back.min.y), pos2(back.max.x, back.min.y)], stroke);
                ui.painter().line_segment([pos2(back.max.x, back.min.y), pos2(back.max.x, back.max.y - 3.0)], stroke);
            }
            Kind::Close => {
                ui.painter().line_segment([pos2(c.x - GLYPH, c.y - GLYPH), pos2(c.x + GLYPH, c.y + GLYPH)], stroke);
                ui.painter().line_segment([pos2(c.x - GLYPH, c.y + GLYPH), pos2(c.x + GLYPH, c.y - GLYPH)], stroke);
            }
        }
        let label = match kind {
            Kind::Minimize => tl!("Minimize"),
            Kind::Maximize if maximized => tl!("Restore"),
            Kind::Maximize => tl!("Maximize"),
            Kind::Close => tl!("Close"),
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let resp = resp.on_hover_text(label);
        if resp.clicked() {
            match kind {
                Kind::Minimize => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
                Kind::Maximize => ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                // The next frame reads this as `close_requested`, so the save prompt runs.
                Kind::Close => ui.ctx().send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
}

/// A `size`-square box centred on `c`.
fn centre_box(c: egui::Pos2, grow: f32) -> Rect {
    let s = GLYPH * 2.0 + grow;
    Rect::from_center_size(c, vec2(s, s))
}

/// Invisible strips along the window's edges that resize a frameless window (`BeginResize`).
/// Skipped while maximized or fullscreen; a maximized window doesn't resize.
pub(crate) fn resize_edges(app: &WordApp, ctx: &Context) {
    if !app.frameless {
        return;
    }
    let (maximized, fullscreen) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if maximized || fullscreen {
        return;
    }
    let full = ctx.input(|i| i.viewport_rect());
    let (l, t, r, b) = (full.min.x, full.min.y, full.max.x, full.max.y);
    let dirs: [(ResizeDirection, Rect, CursorIcon, u8); 8] = [
        (ResizeDirection::North, Rect::from_min_max(pos2(l, t), pos2(r, t + EDGE)), CursorIcon::ResizeNorth, 0),
        (ResizeDirection::South, Rect::from_min_max(pos2(l, b - EDGE), pos2(r, b)), CursorIcon::ResizeSouth, 1),
        (ResizeDirection::West, Rect::from_min_max(pos2(l, t), pos2(l + EDGE, b)), CursorIcon::ResizeWest, 2),
        (ResizeDirection::East, Rect::from_min_max(pos2(r - EDGE, t), pos2(r, b)), CursorIcon::ResizeEast, 3),
        (ResizeDirection::NorthWest, Rect::from_min_max(pos2(l, t), pos2(l + EDGE, t + EDGE)), CursorIcon::ResizeNorthWest, 4),
        (ResizeDirection::NorthEast, Rect::from_min_max(pos2(r - EDGE, t), pos2(r, t + EDGE)), CursorIcon::ResizeNorthEast, 5),
        (ResizeDirection::SouthWest, Rect::from_min_max(pos2(l, b - EDGE), pos2(l + EDGE, b)), CursorIcon::ResizeSouthWest, 6),
        (ResizeDirection::SouthEast, Rect::from_min_max(pos2(r - EDGE, b - EDGE), pos2(r, b)), CursorIcon::ResizeSouthEast, 7),
    ];
    for (dir, rect, cursor, n) in dirs {
        let id = egui::Id::new(("window_resize", n));
        let resp = egui::Area::new(id)
            .order(Order::Middle)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.interact(rect, id, Sense::click_and_drag())
            })
            .inner;
        let resp = resp.on_hover_cursor(cursor);
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::WordApp;

    fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        )
    }

    fn frameless_app() -> WordApp {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.frameless = true;
        app
    }

    /// A frameless window's title bar carries Minimize, Maximize and Close; a native-decorated
    /// one doesn't (the system buttons do that work).
    #[test]
    fn caption_buttons_only_on_a_frameless_window() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(frameless_app());
        for _ in 0..4 {
            h.step();
        }
        for name in ["Minimize", "Maximize", "Close"] {
            assert!(h.query_by_label_contains(name).is_some(), "{name} is drawn");
        }
        let h = harness(WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default()));
        for name in ["Minimize", "Maximize", "Close"] {
            assert!(h.query_by_label_contains(name).is_none(), "{name} belongs to the system's title bar");
        }
    }

    /// Close asks the viewport to close; egui-winit turns that into the close request, and the
    /// desktop app's wrapper runs it through `WordApp::close_requested` — which asks before
    /// dropping unsaved changes, as it does for the native close button.
    #[test]
    fn close_goes_through_the_apps_close_flow() {
        let mut app = frameless_app();
        let _ = app.run("text.insert", json!({"text": "Unsaved"}));
        assert!(!app.close_requested(), "a dirty document is not dropped unasked");
        assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })), "the save prompt shows");
        // A clean document (or no document at all) closes without a prompt.
        let mut app = frameless_app();
        assert!(app.close_requested(), "nothing to save");
    }

    /// The resize edges cover the window's edges but leave the middle of the title bar a drag
    /// region, and none of them are present without `frameless`.
    #[test]
    fn resize_edges_follow_the_frameless_flag() {
        let h = harness(frameless_app());
        for n in [0u8, 1, 3, 7] {
            let id = egui::Id::new(("window_resize", n));
            assert!(h.ctx.read_response(id).is_some(), "edge {n} is a widget");
        }
        let h = harness(WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default()));
        let id = egui::Id::new(("window_resize", 0u8));
        assert!(h.ctx.read_response(id).is_none(), "no edges on a decorated window");
    }
}
