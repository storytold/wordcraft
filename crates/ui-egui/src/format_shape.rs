//! The Format Shape pane (Shape Fill › More Gradients…, a shape's Format Shape…): the selected
//! shapes' fill, as Word's Fill section: no fill, a solid colour with its transparency, or a
//! gradient (preset, type, direction, angle, stops, rotate with shape). Every change runs
//! `shape.fill` with the whole fill; a drag applies as it goes and undoes in one step.

use std::sync::Arc;

use egui::{Color32, Id, Mesh, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use wordcraft_doc::Rgb;
use wordcraft_doc::fill::{Fill, Gradient, GradientKind, PathShape};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::wordart::GradStop;
use wordcraft_engine::cmd::shape_fill::{self, LINEAR_DIRECTIONS, PATH_DIRECTIONS, stops};
use wordcraft_layout::display::paint_for;

use crate::WordApp;
use crate::theme::Tokens;

fn mark_id() -> Id {
    Id::new("format_shape.drag_mark")
}

fn stop_id() -> Id {
    Id::new("format_shape.stop")
}

/// Forgets a drag's undo mark: called while the pane is closed, so a drag cut short by closing
/// it never merges later undo steps.
pub fn forget_drag(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<u64>(mark_id()));
}

/// The gradient a variation applied to the object at a position, and the colour it was built
/// from.
#[derive(Clone)]
struct Variation(wordcraft_doc::Pos, Fill, Rgb);

fn variation_id() -> Id {
    Id::new("format_shape.variation")
}

/// The colour Shape Fill › Gradient builds from for the object at `pos` filled with `fill`:
/// the colour the variation it shows was made from (so picking variations in turn, or No
/// Gradient, doesn't drift), else the fill's main colour.
// ponytail: remembered for the last variation only, in this session; a gradient from a file
// uses its middle stop (Word keeps the scheme colour it was built from).
pub fn variation_base(ctx: &egui::Context, pos: &wordcraft_doc::Pos, fill: &Fill) -> Rgb {
    match ctx.data(|d| d.get_temp::<Variation>(variation_id())) {
        Some(Variation(p, f, base)) if p == *pos && f == *fill => base,
        _ => fill.main_color(),
    }
}

/// Notes that `fill`, made from `base`, was applied to the object at `pos`.
pub fn remember_variation(ctx: &egui::Context, pos: wordcraft_doc::Pos, fill: Fill, base: Rgb) {
    ctx.data_mut(|d| d.insert_temp(variation_id(), Variation(pos, fill, base)));
}

/// The first selected shape's or canvas's fill: `None` with no shape selected.
pub(crate) fn current(app: &WordApp) -> Option<Option<Fill>> {
    let (_, o) = wordcraft_engine::cmd::objects::selected(&app.session)?;
    match o {
        InlineObject::Shape { fill, .. } => Some(fill),
        o => o.canvas_style().map(|c| c.fill.clone()),
    }
}

/// Sets the selected shapes' fill. While `resp` is being dragged each change is applied and
/// the drag becomes one undo step when the pointer is released.
fn apply(app: &mut WordApp, ui: &Ui, fill: Option<&Fill>, resp: Option<&Response>) {
    if resp.is_some_and(Response::dragged) && ui.data(|d| d.get_temp::<u64>(mark_id())).is_none() {
        let mark = app.session.undo_mark();
        ui.data_mut(|d| d.insert_temp(mark_id(), mark));
    }
    let _ = app.run("shape.fill", shape_fill::params(fill));
}

/// The pane, while open.
pub fn pane(app: &mut WordApp, ui: &mut Ui) {
    if crate::panes::header(ui, "Format Shape") {
        let _ = app.run("view.formatShapePane", serde_json::json!({"value": false}));
        return;
    }
    // A drag that applied as it went ends as one undo step.
    if !ui.input(|i| i.pointer.any_down())
        && let Some(mark) = ui.data_mut(|d| {
            let m = d.get_temp::<u64>(mark_id());
            d.remove::<u64>(mark_id());
            m
        })
    {
        app.session.merge_undo_since(mark);
    }
    let Some(fill) = current(app) else {
        ui.label(egui::RichText::new(tl!("Select a shape, text box or drawing canvas to format its fill.")).weak());
        return;
    };
    let theme = app.session.doc.settings.theme_colors.clone();
    ui.label(egui::RichText::new(tl!("Fill")).strong());
    ui.add_space(2.0);
    let kind = match &fill {
        None => 0,
        Some(Fill::Solid { .. }) => 1,
        Some(Fill::Gradient(_)) => 2,
    };
    let mut pick = kind;
    ui.radio_value(&mut pick, 0, tl!("No fill"));
    ui.radio_value(&mut pick, 1, tl!("Solid fill"));
    ui.radio_value(&mut pick, 2, tl!("Gradient fill"));
    if pick != kind {
        let accent1 = theme.get(4).copied().unwrap_or(Rgb(68, 114, 196));
        let main = fill.as_ref().map_or(accent1, Fill::main_color);
        let new = match pick {
            1 => Some(Fill::solid(main)),
            2 => Some(shape_fill::default_gradient(accent1)),
            _ => None,
        };
        apply(app, ui, new.as_ref(), None);
        return;
    }
    ui.add_space(6.0);
    match fill {
        Some(Fill::Solid { color, transparency }) => solid(app, ui, &theme, color, transparency),
        Some(Fill::Gradient(g)) => gradient(app, ui, &theme, &g),
        None => {}
    }
}

fn solid(app: &mut WordApp, ui: &mut Ui, theme: &[Rgb], color: Rgb, transparency: f32) {
    let mut c = color;
    if color_field(ui, theme, Id::new("format_shape.solid_color"), &mut c) {
        apply(app, ui, Some(&Fill::Solid { color: c, transparency }), None);
    }
    let mut t = transparency;
    if let Some(r) = percent(ui, tl!("Transparency"), &mut t) {
        apply(app, ui, Some(&Fill::Solid { color, transparency: t }), Some(&r));
    }
}

/// A labelled 0–100% slider with its value box; the response when it changed.
fn percent(ui: &mut Ui, label: &str, v: &mut f32) -> Option<Response> {
    ui.horizontal(|ui| {
        ui.label(label);
        let s = ui.add(egui::Slider::new(v, 0.0..=100.0).suffix("%").fixed_decimals(0));
        (s.changed()).then_some(s)
    })
    .inner
}

/// A colour button with the theme colour drop-down; true when a colour was picked.
fn color_field(ui: &mut Ui, theme: &[Rgb], id: Id, c: &mut Rgb) -> bool {
    let mut hex = c.hex();
    crate::colors::take_field(ui.ctx(), id, &mut hex);
    ui.horizontal(|ui| {
        ui.label(tl!("Color"));
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 14.0), Sense::hover());
        ui.painter().rect_filled(r, 2.0, crate::theme::c32(*c));
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, Tokens::get(ui.ctx()).border), egui::StrokeKind::Inside);
        ui.menu_button("▾", |ui| crate::colors::field_menu(ui, theme, id, &mut hex));
    });
    match Rgb::parse(&hex) {
        Some(n) if n != *c => {
            *c = n;
            true
        }
        _ => false,
    }
}

/// Sets the gradient's kind, with the tile rectangle Word gives a gradient from a corner.
fn set_kind(g: &mut Gradient, kind: GradientKind) {
    g.tile = match kind {
        GradientKind::Path { focus, .. } => shape_fill::corner_tile(focus),
        GradientKind::Linear { .. } => None,
    };
    g.kind = kind;
}

fn with(g: &Gradient, f: impl FnOnce(&mut Gradient)) -> Fill {
    let mut g = g.clone();
    f(&mut g);
    Fill::Gradient(Arc::new(g))
}

const TYPES: [(&str, &str); 4] = [("Linear", "linear"), ("Radial", "radial"), ("Rectangular", "rectangular"), ("Path", "path")];

fn type_of(k: &GradientKind) -> usize {
    match k {
        GradientKind::Linear { .. } => 0,
        GradientKind::Path { shape: PathShape::Circle, .. } => 1,
        GradientKind::Path { shape: PathShape::Rect, .. } => 2,
        GradientKind::Path { shape: PathShape::Shape, .. } => 3,
    }
}

/// Whether a linear gradient made from `k` scales with the shape: as `k` does, and as Word's
/// new linear gradients do otherwise.
fn scaled_of(k: &GradientKind) -> bool {
    match k {
        GradientKind::Linear { scaled, .. } => *scaled,
        GradientKind::Path { .. } => true,
    }
}

fn kind_of(t: usize, focus: [f32; 4], angle: f32, scaled: bool) -> GradientKind {
    match t {
        1 => GradientKind::Path { shape: PathShape::Circle, focus },
        2 => GradientKind::Path { shape: PathShape::Rect, focus },
        3 => GradientKind::Path { shape: PathShape::Shape, focus: [50.0; 4] },
        _ => GradientKind::Linear { angle, scaled },
    }
}

fn gradient(app: &mut WordApp, ui: &mut Ui, theme: &[Rgb], g: &Gradient) {
    egui::Grid::new("format_shape.gradient").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Preset gradients"));
        ui.menu_button("▾", |ui| {
            if let Some(f) = presets_gallery(ui, theme) {
                apply(app, ui, Some(&f), None);
                ui.close();
            }
        });
        ui.end_row();

        ui.label(tl!("Type"));
        let t = type_of(&g.kind);
        let mut pick = t;
        egui::ComboBox::from_id_salt("format_shape.type").selected_text(tl!(TYPES[t].0)).show_ui(ui, |ui| {
            for (i, (label, _)) in TYPES.iter().enumerate() {
                ui.selectable_value(&mut pick, i, tl!(label));
            }
        });
        if pick != t {
            let (angle, focus) = match g.kind {
                GradientKind::Linear { angle, .. } => (angle, [50.0; 4]),
                GradientKind::Path { focus, .. } => (90.0, focus),
            };
            // The tile rectangle stays: only a Direction pick sets a corner's.
            let kind = kind_of(pick, focus, angle, scaled_of(&g.kind));
            apply(app, ui, Some(&with(g, |g| g.kind = kind)), None);
        }
        ui.end_row();

        if t != 3 {
            ui.label(tl!("Direction"));
            ui.menu_button("▾", |ui| {
                if let Some(kind) = directions(ui, g, t) {
                    apply(app, ui, Some(&with(g, |g| set_kind(g, kind))), None);
                    ui.close();
                }
            });
            ui.end_row();
        }

        if let GradientKind::Linear { angle, scaled } = g.kind {
            ui.label(tl!("Angle"));
            let mut a = angle;
            let r = ui.add(egui::DragValue::new(&mut a).range(0.0..=359.9).suffix("°").speed(1.0));
            if r.changed() {
                apply(app, ui, Some(&with(g, |g| g.kind = GradientKind::Linear { angle: a, scaled })), Some(&r));
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.label(tl!("Gradient stops"));
    let sel = ui.data(|d| d.get_temp::<usize>(stop_id())).unwrap_or(0).min(g.stops.len().saturating_sub(1));
    ui.horizontal(|ui| {
        stops_bar(app, ui, g, sel);
        let sel = ui.data(|d| d.get_temp::<usize>(stop_id())).unwrap_or(0).min(g.stops.len().saturating_sub(1));
        if crate::widgets::icon_button(ui, "plus", tl!("Add gradient stop")).clicked() {
            let next = g.stops.get(sel + 1).map_or(100.0, |s| s.pos);
            let here = g.stops.get(sel).map_or(0.0, |s| s.pos);
            let mut st = g.stops.clone();
            if let Some(i) = stops::add(&mut st, (here + next) / 2.0) {
                ui.data_mut(|d| d.insert_temp(stop_id(), i));
                apply(app, ui, Some(&with(g, |g| g.stops = st)), None);
            }
        }
        if crate::widgets::icon_button(ui, "minus", tl!("Remove gradient stop")).clicked() {
            let mut st = g.stops.clone();
            if stops::remove(&mut st, sel) {
                ui.data_mut(|d| d.insert_temp(stop_id(), sel.saturating_sub(1)));
                apply(app, ui, Some(&with(g, |g| g.stops = st)), None);
            }
        }
    });
    let sel = ui.data(|d| d.get_temp::<usize>(stop_id())).unwrap_or(0).min(g.stops.len().saturating_sub(1));
    let Some(stop) = g.stops.get(sel).copied() else { return };
    let set = |s: GradStop| with(g, |g| g.stops[sel] = s);
    let mut c = stop.color;
    if color_field(ui, theme, Id::new("format_shape.stop_color"), &mut c) {
        apply(app, ui, Some(&set(GradStop { color: c, ..stop })), None);
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Position"));
        let mut p = stop.pos;
        let r = ui.add(egui::DragValue::new(&mut p).range(0.0..=100.0).suffix("%").speed(0.5).fixed_decimals(0));
        if r.changed() {
            let mut st = g.stops.clone();
            let i = stops::move_to(&mut st, sel, p);
            ui.data_mut(|d| d.insert_temp(stop_id(), i));
            apply(app, ui, Some(&with(g, |g| g.stops = st)), Some(&r));
        }
    });
    let mut t = stop.transparency;
    if let Some(r) = percent(ui, tl!("Transparency"), &mut t) {
        apply(app, ui, Some(&set(GradStop { transparency: t, ..stop })), Some(&r));
    }
    let mut rot = g.rot_with_shape;
    if ui.checkbox(&mut rot, tl!("Rotate with shape")).changed() {
        apply(app, ui, Some(&with(g, |g| g.rot_with_shape = rot)), None);
    }
}

/// The gradient stops bar: the gradient with a handle per stop. Click a handle to pick it, drag
/// it to move it, click elsewhere to add a stop there; with the bar focused, the arrow keys pick
/// the previous or next stop.
fn stops_bar(app: &mut WordApp, ui: &mut Ui, g: &Gradient, sel: usize) {
    let t = Tokens::get(ui.ctx());
    let w = (ui.available_width() - 56.0).clamp(120.0, 400.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click_and_drag());
    let bar = Rect::from_min_size(r.min + vec2(6.0, 0.0), vec2(w - 12.0, 16.0));
    let linear = Fill::Gradient(Arc::new(Gradient { kind: GradientKind::Linear { angle: 0.0, scaled: false }, ..g.clone() }));
    paint_fill(ui.painter(), bar, &linear);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
    let x_of = |pos: f32| bar.min.x + bar.width() * pos / 100.0;
    let pos_of = |x: f32| ((x - bar.min.x) / bar.width() * 100.0).clamp(0.0, 100.0);
    for (i, s) in g.stops.iter().enumerate() {
        let x = x_of(s.pos);
        let h = Rect::from_center_size(pos2(x, bar.max.y + 7.0), vec2(9.0, 11.0));
        ui.painter().rect_filled(h, 1.0, crate::theme::c32(s.color));
        let ring = if i == sel { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.border_strong) };
        ui.painter().rect_stroke(h, 1.0, ring, egui::StrokeKind::Outside);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        // The arrows pick stops here: not focus moves, nor nudges of the shape.
        let arrows = egui::EventFilter { horizontal_arrows: true, ..Default::default() };
        ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, arrows));
        let (left, right) = ui
            .input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)));
        let next = if left {
            sel.saturating_sub(1)
        } else if right {
            (sel + 1).min(g.stops.len().saturating_sub(1))
        } else {
            sel
        };
        ui.data_mut(|d| d.insert_temp(stop_id(), next));
    }
    let near = |p: Pos2| {
        g.stops
            .iter()
            .enumerate()
            .map(|(i, s)| (i, (x_of(s.pos) - p.x).abs()))
            .filter(|(_, d)| *d <= 6.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let Some(p) = resp.interact_pointer_pos() else { return };
    // Whether the drag in progress began on a handle: only then does it move a stop.
    let grab_id = Id::new("format_shape.grabbed");
    if resp.drag_started() || resp.clicked() {
        resp.request_focus();
        let hit = near(p);
        ui.data_mut(|d| d.insert_temp(grab_id, hit.is_some()));
        match hit {
            Some(i) => {
                ui.data_mut(|d| d.insert_temp(stop_id(), i));
            }
            None if resp.clicked() => {
                let mut st = g.stops.clone();
                if let Some(i) = stops::add(&mut st, pos_of(p.x)) {
                    ui.data_mut(|d| d.insert_temp(stop_id(), i));
                    apply(app, ui, Some(&with(g, |g| g.stops = st)), None);
                }
            }
            None => {}
        }
    } else if resp.dragged() && resp.drag_delta().x != 0.0 && ui.data(|d| d.get_temp::<bool>(grab_id)).unwrap_or(false) {
        let mut st = g.stops.clone();
        let i = stops::move_to(&mut st, sel, pos_of(p.x));
        ui.data_mut(|d| d.insert_temp(stop_id(), i));
        apply(app, ui, Some(&with(g, |g| g.stops = st)), Some(&resp));
    }
}

/// Word's 30 preset gradients from the theme's colours; the one picked.
fn presets_gallery(ui: &mut Ui, theme: &[Rgb]) -> Option<Fill> {
    let presets = shape_fill::presets(theme);
    let mut out = None;
    egui::Grid::new("format_shape.presets").spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        for (i, f) in presets.iter().enumerate() {
            let style = shape_fill::PRESET_STYLES.get(i / 6).copied().unwrap_or("");
            let name = format!("{}, {} {}", tl!(style), tl!("Accent"), i % 6 + 1);
            if swatch(ui, f, &name) {
                out = Some(f.clone());
            }
            if i % 6 == 5 {
                ui.end_row();
            }
        }
    });
    out
}

/// The direction tiles for gradient type `t` (linear, radial or rectangular); the kind picked.
fn directions(ui: &mut Ui, g: &Gradient, t: usize) -> Option<GradientKind> {
    let kinds: Vec<(GradientKind, String)> = if t == 0 {
        let scaled = scaled_of(&g.kind);
        LINEAR_DIRECTIONS.iter().map(|&a| (GradientKind::Linear { angle: a, scaled }, shape_fill::linear_direction_name(a))).collect()
    } else {
        PATH_DIRECTIONS.iter().map(|&f| (kind_of(t, f, 90.0, true), shape_fill::path_direction_name(f))).collect()
    };
    let mut out = None;
    egui::Grid::new("format_shape.directions").spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        for (i, (k, name)) in kinds.into_iter().enumerate() {
            let f = with(g, |g| set_kind(g, k));
            if swatch(ui, &f, &name) {
                out = Some(k);
            }
            if i % 5 == 4 {
                ui.end_row();
            }
        }
    });
    out
}

/// A clickable square showing `fill`; true when clicked.
pub fn swatch(ui: &mut Ui, fill: &Fill, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
    paint_fill(ui.painter(), r.shrink(2.0), fill);
    let ring = if resp.hovered() { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.border) };
    ui.painter().rect_stroke(r.shrink(2.0), 0.0, ring, egui::StrokeKind::Outside);
    let name = tl!(tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    resp.on_hover_text(name).clicked()
}

/// Paints `fill` over `r`: a grid of vertex colours, fine enough for previews.
pub fn paint_fill(p: &egui::Painter, r: Rect, fill: &Fill) {
    const N: u32 = 16;
    let frame = wordcraft_geom::Rect { x: r.min.x, y: r.min.y, w: r.width(), h: r.height() };
    let Some(paint) = paint_for(fill, frame) else { return };
    let at = |q: Pos2| {
        let (c, a) = paint.at(q.x, q.y);
        rgba(c, a)
    };
    let mut mesh = Mesh::default();
    for j in 0..=N {
        for i in 0..=N {
            let q = pos2(r.min.x + r.width() * i as f32 / N as f32, r.min.y + r.height() * j as f32 / N as f32);
            mesh.colored_vertex(q, at(q));
        }
    }
    for j in 0..N {
        for i in 0..N {
            let k = j * (N + 1) + i;
            mesh.add_triangle(k, k + 1, k + N + 1);
            mesh.add_triangle(k + 1, k + N + 2, k + N + 1);
        }
    }
    p.add(mesh);
}

fn rgba(c: Rgb, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use crate::WordApp;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_doc::fill::Fill;
    use wordcraft_doc::para::InlineObject;

    fn fill(app: &WordApp) -> Option<Fill> {
        match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((_, InlineObject::Shape { fill, .. })) => fill,
            o => panic!("{o:?}"),
        }
    }

    /// Shape Fill › Gradient applies a variation; the Format Shape pane switches the fill to
    /// solid and back to no fill, each one undo step.
    #[test]
    fn gradient_menu_and_format_shape_pane_set_the_fill() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        // Shape Fill › Gradient's body (ribbon buttons are drawn without accessible names).
        let mut m = egui_kittest::Harness::builder().build_ui_state(
            |ui, app: &mut WordApp| {
                let f = fill(app);
                crate::ribbon::gradient_menu(ui, app, f.as_ref());
            },
            a,
        );
        m.run_steps(2);
        m.get_all_by_label("Linear Down").next().expect("a light variation").click();
        m.run_steps(2);
        let a = std::mem::replace(m.state_mut(), WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default()));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(3);
        assert!(matches!(fill(h.state()), Some(Fill::Gradient(_))), "{:?}", fill(h.state()));
        h.state_mut().run("view.formatShapePane", json!({"value": true})).unwrap();
        h.run_steps(3);
        let depth = h.state().session.undo_depth();
        h.get_by_label("Solid fill").click();
        h.run_steps(3);
        assert!(matches!(fill(h.state()), Some(Fill::Solid { .. })));
        h.get_by_label("No fill").click();
        h.run_steps(3);
        assert_eq!(fill(h.state()), None);
        assert_eq!(h.state().session.undo_depth(), depth + 2);
    }
}

#[cfg(test)]
mod drift_tests {
    use crate::WordApp;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_doc::fill::Fill;

    /// Picking variations in turn builds each from the shape's colour, not from the last
    /// variation's, and No Gradient brings that colour back.
    #[test]
    fn variations_dont_drift_and_no_gradient_restores_the_colour() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.fill", json!({"color": "156082"})).unwrap();
        let mut m = egui_kittest::Harness::builder().build_ui_state(
            |ui, app: &mut WordApp| {
                let f = super::current(app).flatten();
                crate::ribbon::gradient_menu(ui, app, f.as_ref());
            },
            a,
        );
        m.run_steps(2);
        let pick = |m: &mut egui_kittest::Harness<WordApp>, name: &str| {
            // The first tile of that name (No Gradient: its swatch, after the heading).
            let n = if name == "No Gradient" { 1 } else { 0 };
            m.get_all_by_label(name).nth(n).unwrap_or_else(|| panic!("{name}")).click();
            m.run_steps(2);
            super::current(m.state()).flatten()
        };
        let first = pick(&mut m, "Linear Down");
        let again = pick(&mut m, "Linear Down");
        assert!(matches!(first, Some(Fill::Gradient(_))));
        assert_eq!(first, again, "the same variation twice gives the same gradient");
        let back = pick(&mut m, "No Gradient");
        assert_eq!(back, Some(Fill::solid(wordcraft_doc::Rgb(0x15, 0x60, 0x82))));
    }
}
