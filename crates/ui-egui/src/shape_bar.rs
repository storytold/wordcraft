//! The shape's right-click bar: Style, Fill and Outline, the three buttons Word shows with the
//! context menu of a shape, just above the menu (below it when there is no room). It comes with
//! the menu and stays while one of its own drop-downs is open; picking from one, a click
//! elsewhere or Escape puts it away. Every button runs a command (`shape.style`, `shape.fill`,
//! `shape.outline`), as the ribbon's Shape Format tab does.

use egui::{Align2, Color32, CornerRadius, Id, Order, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use serde_json::json;
use wordcraft_doc::Rgb;
use wordcraft_doc::para::InlineObject;
use wordcraft_engine::cmd::objects::{SHAPE_STYLE_COLORS, SHAPE_STYLES, shape_style};

use crate::colors::Target;
use crate::ribbon::mi;
use crate::theme::{Tokens, c32, regular};
use crate::{WordApp, icons};

/// One button: icon over its label, with the drop-down arrow beside the label.
const BTN_W: f32 = 56.0;
const BTN_H: f32 = 50.0;
const PAD: f32 = 4.0;
const SPACING: f32 = 2.0;
/// Between the bar and the menu.
const GAP: f32 = 6.0;
pub const BAR_W: f32 = PAD * 2.0 + BTN_W * 3.0 + SPACING * 2.0;
pub const BAR_H: f32 = PAD * 2.0 + BTN_H;

/// The bar's buttons: icon and English label (the `shape` context words it for the bar).
pub const BUTTONS: [(&str, &str); 3] = [("shapeStyle", "Style"), ("shading", "Fill"), ("pen", "Outline")];

/// The theme colours' names, in the order of [`SHAPE_STYLE_COLORS`].
const COLOR_NAMES: [&str; 7] = ["Dark 1", "Accent 1", "Accent 2", "Accent 3", "Accent 4", "Accent 5", "Accent 6"];

/// Whether the selection is something the bar is for: a shape, text box or drawing canvas.
pub fn wanted(app: &WordApp) -> bool {
    !app.session.view.read_mode && app.dialog.is_none() && app.colors.is_none() && crate::ribbon::has_shape_selected(&app.session)
}

/// Where the bar sits for a context menu at `menu`: above it, left edges aligned; below it when
/// there is no room above; inside `viewport` either way. `None` for degenerate input. Pure
/// geometry, unit tested without a window.
pub fn place(menu: Rect, viewport: Rect) -> Option<Rect> {
    if !menu.is_finite() || !viewport.is_finite() || !viewport.is_positive() {
        return None;
    }
    let x = menu.min.x.clamp(viewport.left(), (viewport.right() - BAR_W).max(viewport.left()));
    let above = menu.min.y - GAP - BAR_H;
    let below = menu.max.y + GAP;
    let y = if above >= viewport.top() {
        above
    } else if below + BAR_H <= viewport.bottom() {
        below
    } else {
        viewport.top()
    };
    Some(Rect::from_min_size(pos2(x, y), vec2(BAR_W, BAR_H)))
}

/// Run each frame after the context menu: `menu` is where the menu was drawn this frame (`None`
/// once it is closed), `viewport` the document area. Places, keeps, draws or puts away the bar.
pub fn update(app: &mut WordApp, ctx: &egui::Context, menu: Option<Rect>, viewport: Rect) {
    if !wanted(app) {
        app.canvas.shape_bar = None;
        app.canvas.shape_bar_popup = false;
        return;
    }
    if let Some(m) = menu {
        app.canvas.shape_bar = place(m, viewport);
    } else if let Some(bar) = app.canvas.shape_bar {
        // A press on the bar closes the menu in the frame it opens one of the bar's drop-downs:
        // keep the bar through it.
        let on_bar = ctx.input(|i| (i.pointer.any_down() || i.pointer.any_released()) && i.pointer.interact_pos().is_some_and(|p| bar.contains(p)));
        if !app.canvas.shape_bar_popup && !on_bar {
            app.canvas.shape_bar = None;
        }
    }
    let Some(bar) = app.canvas.shape_bar else {
        app.canvas.shape_bar_popup = false;
        return;
    };
    show(app, ctx, bar);
}

fn show(app: &mut WordApp, ctx: &egui::Context, bar: Rect) {
    let t = Tokens::get(ctx);
    let area = egui::Area::new(Id::unique("wc_shape_bar")).order(Order::Foreground).interactable(true).fixed_pos(bar.min);
    let mut popup = false;
    area.show(ctx, |ui| {
        let painter = ui.painter();
        painter.rect_filled(bar.translate(Vec2::new(0.0, 2.0)), CornerRadius::same(6), t.page_shadow);
        painter.rect_filled(bar, CornerRadius::same(6), t.menu);
        painter.rect_stroke(bar, CornerRadius::same(6), Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
        let (fill, stroke) = current(app);
        for (i, (icon, label)) in BUTTONS.iter().enumerate() {
            let r = Rect::from_min_size(pos2(bar.min.x + PAD + i as f32 * (BTN_W + SPACING), bar.min.y + PAD), vec2(BTN_W, BTN_H));
            let swatch = match i {
                1 => Some(fill),
                2 => Some(stroke),
                _ => None,
            };
            let resp = button(ui, &t, r, i, icon, label, swatch);
            let menu = egui::Popup::menu(&resp).show(|ui| {
                ui.set_min_width(200.0);
                match i {
                    0 => style_menu(ui, app),
                    1 => fill_menu(ui, app),
                    _ => outline_menu(ui, app),
                }
            });
            popup |= menu.is_some() || egui::Popup::is_id_open(ctx, egui::Popup::default_response_id(&resp));
        }
        ctx.data_mut(|d| d.insert_temp(shown_id(), bar));
    });
    app.canvas.shape_bar_popup = popup;
}

/// Where the bar was drawn last frame, for tests.
pub fn shown_id() -> Id {
    Id::unique("wc_shape_bar").with("shown")
}

/// The selected shape's fill and outline, for the swatches under Fill and Outline.
fn current(app: &WordApp) -> (Option<Rgb>, Option<Rgb>) {
    match wordcraft_engine::cmd::objects::selected(&app.session) {
        Some((_, InlineObject::Shape { fill, stroke, .. })) => (fill.map(|f| f.main_color()), stroke),
        _ => (None, None),
    }
}

/// One of the bar's buttons; `swatch` (Fill, Outline) shows the colour it has now under the icon.
fn button(ui: &mut Ui, t: &Tokens, r: Rect, i: usize, icon: &str, label: &str, swatch: Option<Option<Rgb>>) -> egui::Response {
    let text = crate::tl!(label);
    let resp = ui.interact(r, Id::unique(("wc_shape_bar", i)), Sense::click());
    let p = ui.painter();
    if resp.hovered() || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp)) {
        p.rect_filled(r, CornerRadius::same(4), t.hover);
    }
    let icon_r = Rect::from_center_size(pos2(r.center().x, r.min.y + 15.0), vec2(22.0, 22.0));
    icons::paint(p, icon_r, icon, t.icon, t.accent);
    if let Some(c) = swatch {
        let bar = Rect::from_min_max(pos2(icon_r.min.x + 1.0, icon_r.max.y + 1.0), pos2(icon_r.max.x - 1.0, icon_r.max.y + 4.0));
        match c {
            Some(c) => {
                p.rect_filled(bar, 0.0, c32(c));
            }
            None => {
                p.rect_stroke(bar, 0.0, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
            }
        }
    }
    let font = regular(11.0);
    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), t.text).size().x);
    let arrow = 7.0;
    let left = r.center().x - (w + arrow + 2.0) / 2.0;
    let y = r.max.y - 9.0;
    p.text(pos2(left, y), Align2::LEFT_CENTER, text, font, t.text);
    icons::paint(p, Rect::from_center_size(pos2(left + w + 2.0 + arrow / 2.0, y), vec2(arrow, arrow)), "dropdown", t.icon, t.icon);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_text(text)
}

/// Style: our shape styles in the theme's colours, a row per style and a column per colour.
fn style_menu(ui: &mut Ui, app: &mut WordApp) {
    let theme = app.session.doc.settings.theme_colors.clone();
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(crate::tl!("Theme Styles")).small().weak());
    egui::Grid::new("wc_shape_styles").spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        for (style, style_name) in SHAPE_STYLES {
            for ((color, _), color_name) in SHAPE_STYLE_COLORS.iter().zip(COLOR_NAMES) {
                let Some(st) = shape_style(style, color, &theme) else { continue };
                let (r, resp) = ui.allocate_exact_size(vec2(34.0, 26.0), Sense::click());
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(r, 3.0, t.hover);
                }
                let box_r = r.shrink2(vec2(5.0, 4.0));
                if st.shadow {
                    p.rect_filled(box_r.translate(vec2(2.0, 2.0)), 2.0, Color32::from_black_alpha(90));
                }
                p.rect_filled(box_r, 2.0, st.fill.map_or(Color32::TRANSPARENT, c32));
                if let Some(s) = st.stroke {
                    p.rect_stroke(box_r, 2.0, Stroke::new(st.width.clamp(1.0, 2.0), c32(s)), egui::StrokeKind::Inside);
                }
                let tip = format!("{}, {}", crate::tl!(style_name), crate::tl!(color_name));
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
                if resp.on_hover_text(&tip).clicked() {
                    let _ = app.run("shape.style", json!({"style": style, "color": color}));
                    ui.close();
                }
            }
            ui.end_row();
        }
    });
}

/// Fill: no fill, the colour grid, Recent Colors and More Colors….
fn fill_menu(ui: &mut Ui, app: &mut WordApp) {
    let theme = app.session.doc.settings.theme_colors.clone();
    mi(ui, app, "No Fill", "shape.fill", json!({"color": null}));
    crate::colors::menu(ui, app, &theme, Target::cmd("shape.fill"));
}

/// Outline: no outline, the colour grid, and the line's weight.
fn outline_menu(ui: &mut Ui, app: &mut WordApp) {
    let theme = app.session.doc.settings.theme_colors.clone();
    mi(ui, app, "No Outline", "shape.outline", json!({"color": null}));
    crate::colors::menu(ui, app, &theme, Target::cmd("shape.outline"));
    ui.separator();
    let color = current(app).1.unwrap_or(Rgb::BLACK).hex();
    ui.menu_button(crate::tl!("Weight"), |ui| {
        for w in [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0] {
            mi(ui, app, &format!("{w} pt"), "shape.outline", json!({"color": color, "width": w}));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Rect {
        Rect::from_min_size(pos2(100.0, 100.0), vec2(1000.0, 700.0))
    }

    #[test]
    fn sits_above_the_menu_when_there_is_room_and_below_when_not() {
        let menu = Rect::from_min_size(pos2(400.0, 400.0), vec2(220.0, 300.0));
        let bar = place(menu, vp()).unwrap();
        assert_eq!(bar.size(), vec2(BAR_W, BAR_H));
        assert!((bar.max.y - (menu.min.y - GAP)).abs() < 1e-3 && bar.min.x == menu.min.x, "above, left edges aligned: {bar:?}");
        let high = Rect::from_min_size(pos2(400.0, 110.0), vec2(220.0, 300.0));
        let bar = place(high, vp()).unwrap();
        assert!((bar.min.y - (high.max.y + GAP)).abs() < 1e-3, "below a menu at the top: {bar:?}");
    }

    #[test]
    fn stays_inside_the_viewport_and_refuses_nonsense() {
        for x in [-500.0, 90.0, 1090.0, 5000.0] {
            let bar = place(Rect::from_min_size(pos2(x, 500.0), vec2(220.0, 200.0)), vp()).unwrap();
            assert!(bar.min.x >= vp().min.x - 1e-3 && bar.max.x <= vp().max.x + 1e-3, "x {x}: {bar:?}");
        }
        let tall = Rect::from_min_size(pos2(300.0, 105.0), vec2(220.0, 690.0));
        assert!(vp().contains_rect(place(tall, vp()).unwrap()), "no room either side: still on screen");
        assert!(place(Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(1.0, 1.0)), vp()).is_none());
        assert!(place(Rect::from_min_size(pos2(0.0, 0.0), vec2(1.0, 1.0)), Rect::NOTHING).is_none());
    }
}
