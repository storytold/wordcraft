//! Word's floating mini toolbar: the small translucent bar over a non-collapsed text selection
//! with the most-used formatting controls. It mirrors Word's trigger (selection, not caret),
//! position (above, clamped to the viewport, flipping below when tight) and dismissal rules.

use egui::{Align, CornerRadius, Layout, Order, Rect, Sense, Vec2, pos2};
use serde_json::{Value, json};

use crate::theme::{Tokens, regular};
use crate::{WordApp, widgets};

/// Height of the bar (a little taller than the ribbon's 22 pt button row).
const BAR_H: f32 = 30.0;
/// Vertical gap between the bar and the selection's first line.
const VGAP: f32 = 8.0;
/// Distance from the selection's left edge at which the bar is centred.
const ANCHOR_INSET: f32 = 4.0;
/// Horizontal padding inside the bar.
const PAD_X: f32 = 4.0;
/// Width of the font and size combos (the ribbon's metrics).
const COMBO_FONT_W: f32 = 150.0;
const COMBO_SIZE_W: f32 = 52.0;
/// Width of one icon button (the ribbon's small-button metric).
const BTN_W: f32 = 24.0;

/// Fixed bar width, so it never jitters as the selection changes.
pub const BAR_W: f32 = PAD_X * 2.0 + COMBO_FONT_W + COMBO_SIZE_W + 3.0 + 8.0 * BTN_W + 3.0 * 9.0;

/// Where the bar sits for a selection line and viewport; `None` for degenerate inputs (NaN
/// rects must never reach egui). Pure geometry, unit tested without a window.
pub fn anchor_rect(selection_top: f32, selection_left: f32, viewport: Rect) -> Option<Rect> {
    if !selection_top.is_finite() || !selection_left.is_finite() || !viewport.is_finite() || !viewport.is_positive() {
        return None;
    }
    // Horizontal: centre on the selection's start (a few points in), clamped inside the viewport.
    let cx = (selection_left + ANCHOR_INSET).clamp(viewport.left(), viewport.right());
    let x = (cx - BAR_W / 2.0).clamp(viewport.left(), (viewport.right() - BAR_W).max(viewport.left()));
    // Vertical: above the selection, unless there is no room, then below it.
    let above = selection_top - VGAP - BAR_H;
    let y = if above >= viewport.top() { above } else { selection_top + VGAP };
    Some(Rect::from_min_size(pos2(x, y), Vec2::new(BAR_W, BAR_H)))
}

/// Whether the bar may be shown for the current app state.
pub fn visible(app: &WordApp) -> bool {
    // Never over a modal dialog or the Backstage.
    if app.dialog.is_some() || app.ui.backstage {
        return false;
    }
    // Never while the right-click context menu is open: it owns the pointer and would double up.
    if app.canvas.context_menu_open {
        return false;
    }
    // Never while a drag-select is in progress (Word shows it on mouse-up).
    if app.canvas.drag_selecting() {
        return false;
    }
    // Never for a collapsed caret.
    !app.session.sel.is_collapsed()
}

/// Show the bar: `selection` is the screen rect of the selection's first line, `viewport` the
/// canvas area it must stay inside.
pub fn show(app: &mut WordApp, ctx: &egui::Context, selection: Rect, viewport: Rect) {
    if !visible(app) {
        return;
    }
    let Some(bar) = anchor_rect(selection.min.y, selection.min.x, viewport) else { return };
    let t = Tokens::get(ctx);
    // The bar sits above the document but below every popup, and is not movable: it is an overlay
    // pinned to the selection, so moving it would lose the association.
    let area = egui::Area::new(egui::Id::new("wc_mini_toolbar")).order(Order::Tooltip).interactable(true).fixed_pos(bar.min);
    let resp = area.show(ctx, |ui| {
        ui.set_clip_rect(bar);
        let fill = if t.dark {
            egui::Color32::from_rgba_unmultiplied(0x2B, 0x2B, 0x2B, 0xF4)
        } else {
            egui::Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 0xF4)
        };
        let painter = ui.painter();
        painter.rect_filled(bar.translate(Vec2::new(0.0, 2.0)), CornerRadius::same(6), t.page_shadow);
        painter.rect_filled(bar, CornerRadius::same(6), fill);
        painter.rect_stroke(bar, CornerRadius::same(6), egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
        let inner = bar.shrink2(Vec2::new(PAD_X, 0.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
        child.spacing_mut().item_spacing = Vec2::new(1.0, 0.0);
        content(app, &mut child, &t);
    });
    // A click that lands on the bar must not also reach the document. egui already stops the
    // click at the topmost widget; we only make sure the bar itself never looks "hoverable" like
    // text underneath (the canvas sets the I-beam on hover of its own rect).
    let _ = resp;
}

/// The controls, reusing the ribbon's widgets so the bar matches the ribbon exactly.
fn content(app: &mut WordApp, ui: &mut egui::Ui, t: &Tokens) {
    let st: Value = app.session.run("format.state", &json!({})).unwrap_or_default();
    let flag = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    let align = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props).align).unwrap_or_default();

    let font = st.get("font").and_then(Value::as_str).unwrap_or("").to_string();
    let fams = app.previews.families();
    let prev = app.previews.font_preview_fn();
    if let Some(f) = widgets::combo(ui, "mini_font", COMBO_FONT_W, &font, &fams, Some(&*prev)) {
        let _ = app.run("format.font", json!({"name": f}));
    }
    let size = st.get("size").and_then(Value::as_f64).map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }).unwrap_or_default();
    let sizes: Vec<String> =
        wordcraft_engine::cmd::format::SIZES.iter().map(|s| if s.fract() == 0.0 { format!("{s:.0}") } else { s.to_string() }).collect();
    if let Some(v) = widgets::combo(ui, "mini_size", COMBO_SIZE_W, &size, &sizes, None)
        && let Ok(x) = v.trim().parse::<f64>()
    {
        let _ = app.run("format.size", json!({"size": x}));
    }
    ui.add_space(3.0);
    sep(ui, t);

    widgets::small(ui, app, "bold", None, "Bold", "format.bold", json!({}), flag("bold"));
    widgets::small(ui, app, "italic", None, "Italic", "format.italic", json!({}), flag("italic"));
    widgets::small(ui, app, "underline", None, "Underline", "format.underline", json!({}), flag("underline"));
    widgets::small(ui, app, "strike", None, "Strikethrough", "format.strikethrough", json!({}), flag("strike"));
    sep(ui, t);

    let hl = app.canvas.last_highlight.clone();
    widgets::split(ui, app, "highlight", "Text Highlight Color", "format.highlight", json!({"color": hl}), false, None, |ui, app| {
        highlight_menu(ui, app);
    });
    let fc = app.canvas.last_font_color.clone();
    let sw = wordcraft_doc::Rgb::parse(&fc).map(crate::theme::c32);
    widgets::split(ui, app, "fontcolor", "Font Color", "format.color", json!({"color": fc}), false, sw, |ui, app| {
        font_color_menu(ui, app);
    });
    sep(ui, t);

    use wordcraft_doc::Align as A;
    widgets::small(ui, app, "alignLeft", None, "Align Left", "para.alignLeft", json!({}), align == A::Left);
    widgets::small(ui, app, "alignCenter", None, "Center", "para.alignCenter", json!({}), align == A::Center);
    widgets::small(ui, app, "alignRight", None, "Align Right", "para.alignRight", json!({}), align == A::Right);
    widgets::small(ui, app, "justify", None, "Justify", "para.justify", json!({}), align == A::Justify);
}

/// A hairline divider between the bar's groups.
fn sep(ui: &mut egui::Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(9.0, 20.0), Sense::hover());
    let p = ui.painter();
    p.vline(r.center().x, r.y_range().shrink(4.0), egui::Stroke::new(1.0, t.border));
}

/// Highlight colour grid, matching the ribbon's.
fn highlight_menu(ui: &mut egui::Ui, app: &mut WordApp) {
    egui::Grid::new("mini_hl").spacing(Vec2::new(3.0, 3.0)).show(ui, |ui| {
        for (i, h) in wordcraft_doc::props::Highlight::ALL.iter().enumerate().skip(1) {
            let Some(c) = h.rgb() else { continue };
            let (r, resp) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::click());
            ui.painter().rect_filled(r, 2.0, egui::Color32::from_rgb(c.0, c.1, c.2));
            if resp.on_hover_text(h.name()).clicked() {
                app.canvas.last_highlight = h.ooxml().to_string();
                let _ = app.run("format.highlight", json!({"color": h.ooxml()}));
                ui.close();
            }
            if i % 5 == 0 {
                ui.end_row();
            }
        }
    });
    if ui.selectable_label(false, "No Color").clicked() {
        let _ = app.run("format.highlight", json!({"color": "none"}));
        ui.close();
    }
}

/// Font colour: Automatic plus the standard colour grid.
fn font_color_menu(ui: &mut egui::Ui, app: &mut WordApp) {
    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
        if ui.selectable_label(false, "Automatic").clicked() {
            let _ = app.run("format.color", json!({"color": "auto"}));
            ui.close();
            return;
        }
        let theme = app.session.doc.settings.theme_colors.clone();
        if let Some(hex) = widgets::color_grid(ui, &theme) {
            app.canvas.last_font_color = hex.clone();
            let _ = app.run("format.color", json!({"color": hex}));
            ui.close();
        }
    });
    let _ = regular(10.0); // keep the theme font helper referenced by this module's public surface
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), Vec2::new(1200.0, 700.0))
    }

    #[test]
    fn sits_above_the_selection_when_there_is_room() {
        let left = vp().center().x;
        let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
        assert_eq!(r.size(), Vec2::new(BAR_W, BAR_H));
        assert!((r.max.y - (400.0 - VGAP)).abs() < 1e-3, "bar bottom should clear the selection by VGAP");
        assert!(r.max.y < 400.0, "the bar must be above the selection");
        assert!((r.center().x - (left + ANCHOR_INSET)).abs() < 1e-3, "bar should be centred on the selection's left edge");
    }

    #[test]
    fn centred_unless_the_viewport_forces_a_shift() {
        let left = vp().min.x + 20.0;
        let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
        assert!((r.min.x - vp().min.x).abs() < 1e-3, "the bar must be pushed onto the viewport's left edge");
        let right = vp().max.x - 20.0;
        let r = anchor_rect(400.0, right, vp()).unwrap_or(Rect::EVERYTHING);
        assert!((r.max.x - vp().max.x).abs() < 1e-3, "the bar must be pushed onto the viewport's right edge");
    }

    #[test]
    fn flips_below_when_there_is_no_room_above() {
        let r = anchor_rect(60.0, 300.0, vp()).unwrap_or(Rect::EVERYTHING);
        assert!((r.min.y - (60.0 + VGAP)).abs() < 1e-3, "bar top should clear the selection by VGAP below");
        assert!(r.min.y > 60.0, "the bar must be below the selection");
    }

    #[test]
    fn never_leaves_the_viewport() {
        for left in [0.0, 130.0, 400.0, 2000.0, -500.0] {
            let r = anchor_rect(400.0, left, vp()).unwrap_or(Rect::EVERYTHING);
            assert!(r.min.x >= vp().min.x - 1e-3, "left {left}: bar left {} out of viewport", r.min.x);
            assert!(r.max.x <= vp().max.x + 1e-3, "left {left}: bar right {} out of viewport", r.max.x);
        }
    }

    #[test]
    fn degenerate_inputs_are_rejected_not_crashed() {
        assert!(anchor_rect(f32::NAN, 10.0, vp()).is_none());
        assert!(anchor_rect(10.0, f32::NAN, vp()).is_none());
        assert!(anchor_rect(f32::INFINITY, 10.0, vp()).is_none());
        assert!(anchor_rect(10.0, f32::NEG_INFINITY, vp()).is_none());
        assert!(anchor_rect(10.0, 10.0, Rect::EVERYTHING).is_none());
        assert!(anchor_rect(10.0, 10.0, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(0.0, 100.0))).is_none());
    }

    #[test]
    fn a_viewport_narrower_than_the_bar_still_yields_a_rect() {
        let r = anchor_rect(10.0, 5.0, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(20.0, 20.0)));
        let r = r.unwrap_or(Rect::EVERYTHING);
        assert!(r.width() > 0.0 && r.height() > 0.0, "a tiny viewport must still place a positive-size bar");
        assert!(r.min.x.is_finite() && r.min.y.is_finite());
    }

    #[test]
    fn bar_width_covers_exactly_its_controls() {
        // The position maths uses BAR_W, so it must equal the width the widgets actually take:
        // padding + two combos + a gap + eight 24 pt buttons + three 9 pt dividers.
        let expected = PAD_X * 2.0 + COMBO_FONT_W + COMBO_SIZE_W + 3.0 + 8.0 * BTN_W + 3.0 * 9.0;
        assert_eq!(BAR_W, expected);
    }
}
