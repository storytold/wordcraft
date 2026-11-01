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
/// The compact geometry shared by the caption glyphs.
const GLYPH: f32 = 5.0;
const ICON_STROKE: f32 = 1.35;
/// The thickness of the invisible resize strips along the window's edges.
const EDGE: f32 = 6.0;
/// The height of the title bar and of the band the Backstage reserves for it.
pub(crate) const BAND_HEIGHT: f32 = 38.0;
pub(crate) const BUTTONS_WIDTH: f32 = WIDTH * 2.0 + CLOSE_WIDTH;

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
    // Right-to-left layout puts the first button at the right edge. This produces the standard
    // left-to-right order: Minimize, Maximize/Restore, Close.
    for kind in [Kind::Close, Kind::Maximize, Kind::Minimize] {
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
            ICON_STROKE,
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
                ui.painter().line_segment([pos2(c.x - GLYPH, c.y + 2.5), pos2(c.x + GLYPH, c.y + 2.5)], stroke);
            }
            Kind::Maximize if !maximized => {
                ui.painter().rect_stroke(centre_box(c, 0.0), 1.0, stroke, StrokeKind::Inside);
            }
            Kind::Maximize => {
                // Restore: two offset windows with only the visible edges of the rear one.
                let front = centre_box(c, 0.0).translate(vec2(-1.0, 1.0));
                let back = centre_box(c, 0.0).translate(vec2(2.0, -2.0));
                ui.painter().line_segment([back.left_top(), back.right_top()], stroke);
                ui.painter().line_segment([back.right_top(), back.right_bottom()], stroke);
                ui.painter().rect_stroke(front, 1.0, stroke, StrokeKind::Inside);
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
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
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
    let s = GLYPH * 2.0 + grow - 1.0;
    Rect::from_center_size(c, vec2(s, s))
}

/// The top band of a full-window surface that covers the title bar (Backstage): it drags the
/// window (double-click maximizes, as the title bar does) and carries the caption buttons at its
/// right end. The caller leaves `band` clear of content; the buttons are drawn over it here.
pub(crate) fn band(app: &WordApp, ctx: &Context, band: Rect) {
    if !app.frameless {
        return;
    }
    let controls_rect = Rect::from_min_max(pos2(band.max.x - BUTTONS_WIDTH, band.min.y), band.max);
    let drag_rect = Rect::from_min_max(band.min, pos2(controls_rect.min.x, band.max.y));
    let drag_id = egui::Id::unique("window_band_drag");
    let drag = egui::Area::new(drag_id)
        .order(Order::Foreground)
        .fixed_pos(drag_rect.min)
        .show(ctx, |ui| {
            ui.allocate_rect(drag_rect, Sense::click_and_drag())
        })
        .inner;
    if drag.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
    egui::Area::new(egui::Id::unique("window_band_buttons"))
        .order(Order::Foreground)
        .fixed_pos(controls_rect.min)
        .show(ctx, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(controls_rect).layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    draw(ui, controls_rect.height());
                },
            );
        });
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
        let id = egui::Id::unique(("window_resize", n));
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
    /// one doesn't (the system buttons do that work). Close sits at the right end, as a Windows
    /// caption bar has it.
    #[test]
    fn caption_buttons_only_on_a_frameless_window() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(frameless_app());
        for _ in 0..4 {
            h.step();
        }
        let close = h.get_by_label_contains("Close");
        let max = h.get_by_label_contains("Maximize");
        let min = h.get_by_label_contains("Minimize");
        assert!(
            min.rect().left() < max.rect().left() && max.rect().left() < close.rect().left(),
            "Minimize, Maximize, Close left to right, Close at the edge: min={}, max={}, close={}",
            min.rect().left(),
            max.rect().left(),
            close.rect().left()
        );
        let h = harness(WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default()));
        for name in ["Minimize", "Maximize", "Close"] {
            assert!(h.query_by_label_contains(name).is_none(), "{name} belongs to the system's title bar");
        }
    }

    /// The Backstage covers the title bar: its top band carries the caption buttons and a drag
    /// region, so the frameless window can still be moved (and closed) from the File pages.
    #[test]
    fn the_backstage_band_carries_the_caption_buttons() {
        use egui_kittest::kittest::Queryable;
        let mut app = frameless_app();
        app.run("ui.backstage", json!({"value": true})).unwrap();
        let mut h = harness(app);
        for _ in 0..4 {
            h.step();
        }
        // The Backstage's own Close page shares the label; the caption button is the one in the
        // window's top band.
        let close = h
            .get_all_by_label_contains("Close")
            .into_iter()
            .find(|n| n.rect().top() < crate::caption_buttons::BAND_HEIGHT)
            .expect("the caption Close sits in the band");
        assert!(close.rect().top() < crate::caption_buttons::BAND_HEIGHT, "the buttons sit in the Backstage's top band");
        assert!(h.ctx.read_response(egui::Id::unique("window_band_drag")).is_some(), "the band drags the window");
        // Click back button closes backstage:
        h.event(egui::Event::PointerMoved(pos2(23.0, 19.0)));
        h.event(egui::Event::PointerButton { pos: pos2(23.0, 19.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos2(23.0, 19.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
        assert!(!h.state().ui.backstage, "clicking backstage back should close backstage");
        // Drag in the window band:
        let mut app2 = frameless_app();
        app2.run("ui.backstage", json!({"value": true})).unwrap();
        let mut h2 = harness(app2);
        for _ in 0..4 {
            h2.step();
        }
        h2.event(egui::Event::PointerMoved(pos2(300.0, 19.0)));
        h2.event(egui::Event::PointerButton { pos: pos2(300.0, 19.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h2.event(egui::Event::PointerMoved(pos2(350.0, 19.0)));
        h2.step();
        let layer = h2.ctx.layer_id_at(pos2(300.0, 19.0));
        assert_eq!(layer, Some(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("window_band_drag"))));
        // The sidebar's top band also drags the window:
        h2.event(egui::Event::PointerMoved(pos2(100.0, 19.0)));
        h2.event(egui::Event::PointerButton { pos: pos2(100.0, 19.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h2.event(egui::Event::PointerMoved(pos2(120.0, 19.0)));
        h2.step();
        assert!(h2.state().ui.backstage, "dragging sidebar top band leaves backstage open");
        let mut h = harness(WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default()));
        let _ = h.state_mut().run("ui.backstage", json!({"value": true}));
        for _ in 0..4 {
            h.step();
        }
        // The Backstage's own Close page keeps its label on every window; what must differ is
        // the caption button in the window's top band.
        let in_band = h.get_all_by_label_contains("Close").into_iter().any(|n| n.rect().top() < crate::caption_buttons::BAND_HEIGHT);
        assert!(!in_band, "a native title bar already has the buttons");
    }

    #[test]
    fn frameless_start_screen_back_button_and_drag() {
        let mut app = frameless_app();
        app.start_screen();
        assert!(app.ui.backstage);
        assert!(!app.has_document());
        let mut h = harness(app);
        h.run_steps(4);
        // Drag in central panel top band
        h.event(egui::Event::PointerMoved(pos2(300.0, 19.0)));
        h.event(egui::Event::PointerButton { pos: pos2(300.0, 19.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerMoved(pos2(350.0, 19.0)));
        h.step();
        assert_eq!(h.ctx.layer_id_at(pos2(300.0, 19.0)), Some(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("window_band_drag"))));

        // Click Back button leaves start screen to empty window
        h.event(egui::Event::PointerMoved(pos2(23.0, 19.0)));
        h.event(egui::Event::PointerButton { pos: pos2(23.0, 19.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos2(23.0, 19.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "clicking back leaves the Start screen");
        assert!(!h.state().has_document(), "for the empty window");
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
            let id = egui::Id::unique(("window_resize", n));
            assert!(h.ctx.read_response(id).is_some(), "edge {n} is a widget");
        }
        let h = harness(WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default()));
        let id = egui::Id::unique(("window_resize", 0u8));
        assert!(h.ctx.read_response(id).is_none(), "no edges on a decorated window");
    }
}
