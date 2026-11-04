//! The close control of a floating window (dialogs, the Read Aloud player, the file browser),
//! as on a Word dialog on Windows (#1885): a box at the right end of the title bar, the full
//! title-bar height and about 1.6× that wide, all of it clickable. It fills red with a white ✕
//! while the pointer is over it and darker red while pressed. Escape closes dialogs as before.

use egui::{CornerRadius, Margin, Rect, Response, Sense, Stroke, TextStyle, Ui, UiBuilder, pos2, vec2};

use crate::theme::Tokens;

/// The box's width as a multiple of the title bar height.
const WIDTH: f32 = 1.6;
/// Half the ✕'s width and height.
const GLYPH: f32 = 5.0;

/// The title bar's measurements for the current style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    /// Title bar height inside the frame line: the heading row plus the window margin.
    pub title_h: f32,
    /// The box's width, a whole number so the title bar's margin can reserve it.
    pub width: f32,
}

pub(crate) fn geometry(ctx: &egui::Context) -> Geometry {
    let style = ctx.global_style();
    let heading = ctx.fonts_mut(|f| f.row_height(&TextStyle::Heading.resolve(&style)));
    let m = style.spacing.window_margin;
    let title_h = heading + f32::from(m.top) + f32::from(m.bottom);
    Geometry { title_h, width: (WIDTH * title_h).round().clamp(0.0, 127.0) }
}

/// Reserves the box's room at the right end of `w`'s title bar. Pair with [`show`] after
/// `Window::show`; the window must not use egui's own close button (`Window::open`).
pub(crate) fn window<'a>(ctx: &egui::Context, w: egui::Window<'a>) -> egui::Window<'a> {
    let style = ctx.global_style();
    let m = style.spacing.window_margin;
    let right = geometry(ctx).width as i8;
    w.title_frame(egui::Frame::window(&style).inner_margin(Margin { right, ..m }))
}

/// The box for a window whose outer rect is `window` (frame line included), with a title bar
/// `g.title_h` tall inside a `stroke` wide frame line.
pub(crate) fn close_rect(window: Rect, stroke: f32, g: Geometry) -> Rect {
    let top = window.top() + stroke;
    let right = window.right() - stroke;
    Rect::from_min_max(pos2(right - g.width, top), pos2(right, top + g.title_h))
}

/// Draws the box over `window` (the response `Window::show` returned) and handles the pointer.
/// True when the window should close: the box was clicked, or something inside asked for it.
pub(crate) fn show(ctx: &egui::Context, window: &Response, t: &Tokens) -> bool {
    let style = ctx.global_style();
    let stroke = style.visuals.window_stroke.width;
    let rect = close_rect(window.rect, stroke, geometry(ctx));
    let id = window.layer_id.id.with("close_box");
    let ui = Ui::new(ctx.clone(), id, UiBuilder::new().layer_id(window.layer_id).max_rect(rect));
    // Pointer-only, like a caption button: Tab never lands on it.
    let r = ui.interact(rect, id.with("hit"), Sense::CLICK | Sense::DRAG);
    let label = tl!("Close");
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let r = r.on_hover_text(label);
    let down = r.is_pointer_button_down_on() && r.contains_pointer();
    let hot = down || r.hovered();
    let painter = ctx.layer_painter(window.layer_id);
    if hot {
        let ne = style.visuals.window_corner_radius.ne.saturating_sub(stroke.round() as u8);
        painter.rect_filled(rect, CornerRadius { ne, ..CornerRadius::ZERO }, if down { t.close_pressed } else { t.close_hover });
    }
    let c = pos2(painter.round_to_pixel_center(rect.center().x), painter.round_to_pixel_center(rect.center().y));
    let s = Stroke::new(1.0, if hot { t.on_close } else { t.text });
    painter.line_segment([c + vec2(-GLYPH, -GLYPH), c + vec2(GLYPH, GLYPH)], s);
    painter.line_segment([c + vec2(-GLYPH, GLYPH), c + vec2(GLYPH, -GLYPH)], s);
    r.clicked() || window.should_close()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WordApp;
    use serde_json::json;

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

    /// Clicks the close box of the window area `id`: 6 px from its right edge, mid title bar,
    /// well outside the ✕.
    fn click_close_box(h: &mut egui_kittest::Harness<'_, WordApp>, id: egui::Id) {
        let window = h.ctx.memory(|m| m.area_rect(id)).expect("window is open");
        let at = pos2(window.right() - 6.0, close_rect(window, 1.0, geometry(&h.ctx)).center().y);
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        h.event(egui::Event::PointerMoved(at));
        h.step();
        h.event(button(true));
        h.step();
        h.event(button(false));
        for _ in 0..4 {
            h.step();
        }
    }

    /// The box fills the title bar's height inside the frame line and is as wide as the
    /// geometry says, flush with the window's right edge.
    #[test]
    fn the_box_is_the_title_bars_right_end() {
        let g = Geometry { title_h: 30.0, width: 48.0 };
        let r = close_rect(Rect::from_min_max(pos2(100.0, 50.0), pos2(400.0, 200.0)), 1.0, g);
        assert_eq!(r, Rect::from_min_max(pos2(351.0, 51.0), pos2(399.0, 81.0)));
    }

    /// With the app's style, the box is the heading row plus the window margin tall and about
    /// 1.6× as wide, a whole number of points.
    #[test]
    fn geometry_follows_the_style() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions(false, false));
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.begin_pass(Default::default());
        let g = geometry(&ctx);
        let m = ctx.global_style().spacing.window_margin;
        let font = TextStyle::Heading.resolve(&ctx.global_style());
        let heading = ctx.fonts_mut(|f| f.row_height(&font));
        ctx.end_pass().drop_without_applying_deltas();
        assert!(heading > 12.0, "a heading row: {heading}");
        assert_eq!(g.title_h, heading + f32::from(m.top) + f32::from(m.bottom));
        assert_eq!(g.width, g.width.round());
        assert!((g.width / g.title_h - WIDTH).abs() < 0.05, "{g:?}");
    }

    /// The Read Aloud player and the file browser use the same box, and close from it.
    #[test]
    fn the_player_and_the_file_browser_close_from_the_box() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.session.read_aloud.open = true;
        let mut h = harness(app);
        for _ in 0..4 {
            h.step();
        }
        click_close_box(&mut h, egui::Id::unique("read-aloud-player"));
        assert!(!h.state().session.read_aloud.open, "the player stops and closes");
        let _ = h.state_mut().run("ui.fileBrowser", json!({"purpose": "document"}));
        for _ in 0..4 {
            h.step();
        }
        assert!(h.state().file_browser.open.is_some(), "the browser opened");
        click_close_box(&mut h, egui::Id::unique("builtin_file_browser"));
        assert!(h.state().file_browser.open.is_none(), "the browser is cancelled");
    }

    /// The box is pointer-only, like a Windows caption button: Tab never lands on it, so Enter
    /// can't close a dialog by accident.
    #[test]
    fn the_box_is_not_in_the_tab_order() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let _ = app.run("ui.dialog", json!({"name": "wordCount"}));
        let mut h = harness(app);
        for _ in 0..4 {
            h.step();
        }
        let hit = egui::Id::unique(("dialog", "Word Count")).with("close_box").with("hit");
        let r = h.ctx.read_response(hit).expect("the close box is a widget");
        assert!(r.sense.senses_click() && !r.sense.is_focusable(), "pointer-only: {:?}", r.sense);
        for _ in 0..12 {
            h.key_press(egui::Key::Tab);
            h.step();
            assert_ne!(h.ctx.memory(|m| m.focused()), Some(hit), "Tab skips the close box");
        }
        assert!(h.state().dialog.is_some(), "the dialog is still open");
    }
}
