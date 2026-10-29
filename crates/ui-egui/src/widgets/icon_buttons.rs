use super::buttons::{DROPDOWN, focus_ring, focus_ring_with};
use super::combo::drawn_state;
use super::{Color32, CornerRadius, Rect, Response, Sense, Stroke, Tokens, Ui, icons, pos2, regular, vec2};

/// A dialog's primary button (OK, Save, Close): the accent fill with `on_accent` text, per theme.
pub fn primary_button(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    let t = Tokens::get(ui.ctx());
    egui::Button::new(egui::RichText::new(text.into()).color(t.on_accent)).fill(t.accent)
}

/// A frameless 22 px icon button (pane close buttons): ribbon-style hover and press feedback.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let st = drawn_state(ui, &resp, false);
    ui.painter().rect_filled(r, CornerRadius::same(4), st.fill);
    focus_ring(ui, r, &resp, &t);
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, t.icon, t.accent);
    resp.on_hover_text(tip)
}

/// A text button with a drawn icon in front. Symbols such as ✎ or 💬 aren't in the interface
/// fonts and would draw as empty boxes, so icons are painted (`icons.rs`) rather than typed.
/// It looks and reacts like `ui.button` (see [`drawn_state`]).
pub fn icon_text_button(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId) -> Response {
    icon_text_button_with(ui, icon, text, font, false, true)
}

/// [`icon_text_button`] that opens a menu: the drop-down marker follows the text.
pub fn icon_text_menu_button(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId) -> Response {
    icon_text_button_with(ui, icon, text, font, true, true)
}

/// The icon-only form of [`icon_text_button`] / [`icon_text_menu_button`] for a crowded strip
/// (#2026): `label` is the accessible name and the tooltip, not drawn.
pub fn icon_only_button(ui: &mut Ui, icon: &str, label: &str, font: egui::FontId, marker: bool) -> Response {
    icon_text_button_with(ui, icon, label, font, marker, false).on_hover_text(label)
}

/// Width [`icon_text_button`] (or its menu / icon-only forms) takes.
pub fn icon_text_button_width(ui: &Ui, text: &str, font: egui::FontId, marker: bool, show_text: bool) -> f32 {
    let text_w = if show_text { ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER).size().x) } else { 0.0 };
    icon_text_button_size(ui, text_w, marker, show_text)
}

fn icon_text_button_size(ui: &Ui, text_w: f32, marker: bool, show_text: bool) -> f32 {
    let pad = ui.spacing().button_padding;
    let gap = if show_text { 4.0 } else { 0.0 };
    let marker_w = if marker { DROPDOWN + 4.0 } else { 0.0 };
    pad.x + 16.0 + gap + text_w + marker_w + pad.x
}

/// Allocates and paints a themed icon button with optional text, marker and accessible label, and
/// returns its response.
fn icon_text_button_with(ui: &mut Ui, icon: &str, text: &str, font: egui::FontId, marker: bool, show_text: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let pad = ui.spacing().button_padding;
    let icon_w = 16.0;
    let text_w = if show_text { galley.size().x } else { 0.0 };
    let size = vec2(icon_text_button_size(ui, text_w, marker, show_text), galley.size().y.max(icon_w) + 2.0 * pad.y);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let st = drawn_state(ui, &resp, true);
    let r = r.expand(st.expansion);
    ui.painter().rect(r, ui.visuals().widgets.inactive.corner_radius, st.fill, st.stroke, egui::StrokeKind::Inside);
    focus_ring_with(ui, r, &resp, ui.visuals().widgets.inactive.corner_radius, t.focus);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(r.min.x + pad.x + icon_w / 2.0, r.center().y), vec2(icon_w, icon_w)),
        icon,
        t.icon,
        t.accent,
    );
    let text_x = r.min.x + pad.x + icon_w + if show_text { 4.0 } else { 0.0 };
    if show_text {
        ui.painter().galley(pos2(text_x, r.center().y - galley.size().y / 2.0), galley, st.text);
    }
    if marker {
        let c = pos2(text_x + text_w + 4.0 + DROPDOWN / 2.0, r.center().y);
        icons::paint(ui.painter(), Rect::from_center_size(c, vec2(DROPDOWN, DROPDOWN)), "dropdown", st.text, st.text);
    }
    resp
}

/// A drop-down menu item with `icon` before `text`, at least `min_w` wide.
pub fn icon_menu_item(ui: &mut Ui, icon: &str, text: &str, shortcut: &str, min_w: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::PLACEHOLDER);
    let sc = (!shortcut.is_empty()).then(|| ui.painter().layout_no_wrap(shortcut.to_string(), font, Color32::PLACEHOLDER));
    let pad = ui.spacing().button_padding;
    let icon_w = 16.0;
    let sc_w = sc.as_ref().map_or(0.0, |g| g.size().x + 3.0 * pad.x);
    let w = (pad.x + icon_w + 6.0 + galley.size().x + sc_w + pad.x).max(min_w).max(ui.available_width());
    let (r, resp) = ui.allocate_exact_size(vec2(w, galley.size().y.max(icon_w) + 2.0 * pad.y), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let st = drawn_state(ui, &resp, false);
    ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, st.fill);
    focus_ring_with(ui, r, &resp, ui.visuals().widgets.inactive.corner_radius, t.focus);
    let (line, accent) = if ui.is_enabled() { (t.icon, t.accent) } else { (t.text_disabled, t.text_disabled) };
    let text_c = if ui.is_enabled() { st.text } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + pad.x + icon_w / 2.0, r.center().y), vec2(icon_w, icon_w)), icon, line, accent);
    ui.painter().galley(pos2(r.min.x + pad.x + icon_w + 6.0, r.center().y - galley.size().y / 2.0), galley, text_c);
    if let Some(g) = sc {
        ui.painter().galley(pos2(r.max.x - pad.x - g.size().x, r.center().y - g.size().y / 2.0), g, t.text_disabled);
    }
    resp
}

/// A strip of dialog tabs. The active tab joins the page below it.
/// Returns the index of the tab the user clicked.
pub fn dialog_tabs(ui: &mut Ui, labels: &[String], selected: usize) -> Option<usize> {
    const H: f32 = 26.0;
    let t = Tokens::get(ui.ctx());
    let page = ui.visuals().window_fill;
    let need: f32 = labels.iter().map(|l| ui.painter().layout_no_wrap(l.clone(), regular(12.5), t.text).size().x + 27.0).sum::<f32>() + 1.0;
    ui.set_min_width(need);
    let origin = ui.cursor().min;
    let width = ui.available_width();
    let base_y = origin.y + H;
    let base = ui.painter().add(egui::Shape::Noop);
    let top = CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 };
    let mut x = origin.x;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let on = i == selected;
        let galley = ui.painter().layout_no_wrap(label.clone(), regular(12.5), t.text);
        let w = galley.size().x + 28.0;
        let r = Rect::from_min_max(pos2(x, origin.y + 2.0), pos2(x + w, base_y));
        let resp = ui.interact(r, ui.scope_id().with(("dialog_tab", i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
        let hover = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered());
        let fill = if on { page } else { page.lerp_to_gamma(t.border, 0.14 + 0.1 * hover) };
        let p = ui.painter();
        let shown = if on { r } else { Rect::from_min_max(pos2(r.min.x, r.min.y + 2.0), r.max) };
        p.rect(shown, top, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        if on {
            p.rect_filled(Rect::from_min_max(pos2(r.min.x + 1.0, base_y - 1.0), pos2(r.max.x - 1.0, base_y + 1.0)), 0.0, page);
        }
        focus_ring_with(ui, shown, &resp, top, t.focus);
        let color = if on { t.text } else { t.text.gamma_multiply(0.8) };
        p.galley(pos2(r.center().x - galley.size().x / 2.0, r.center().y - galley.size().y / 2.0), galley, color);
        if resp.clicked() && !on {
            clicked = Some(i);
        }
        x += w - 1.0;
    }
    ui.allocate_space(vec2(width, H + 1.0));
    ui.painter().set(base, egui::Shape::line_segment([pos2(origin.x, base_y), pos2(origin.x + width, base_y)], Stroke::new(1.0, t.border)));
    clicked
}
