//! The Format Shape pane (Shape Fill › More Gradients…, a shape's Format Shape…): the selected
//! shapes' fill, outline, and layout settings (size, rotation, text direction, and alignment).
//! Every change runs the corresponding shape command; a drag applies as it goes and undoes in one step.

use std::sync::Arc;

use egui::{Color32, Id, Mesh, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::Rgb;
use wordcraft_doc::fill::{Fill, Gradient, GradientKind, PathShape};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::wordart::GradStop;
use wordcraft_engine::cmd::shape_fill::{self, LINEAR_DIRECTIONS, PATH_DIRECTIONS, stops};
use wordcraft_layout::display::paint_for;

use crate::WordApp;
use crate::theme::Tokens;

/// Stable storage id for a fill drag's undo mark.
fn mark_id() -> Id {
    Id::unique("format_shape.drag_mark")
}

/// Stable storage id for a line drag's undo mark.
fn line_mark_id() -> Id {
    Id::unique("format_shape.line_drag_mark")
}

/// Stable storage id for a layout (width, height, rotation) drag's undo mark.
fn layout_mark_id() -> Id {
    Id::unique("format_shape.layout_drag_mark")
}

/// Stable storage id for the gradient stop being edited.
fn stop_id() -> Id {
    Id::unique("format_shape.stop")
}

/// Active pointer-drag bookmark so subsequent frames join the drag's first checkpoint.
#[derive(Clone, Copy, PartialEq, Eq)]
struct DragMark {
    mark: u64,
    widget: Id,
}

/// Forgets a drag's undo mark: called while the pane is closed, so a drag cut short by closing
/// it never merges later undo steps.
pub fn forget_drag(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<DragMark>(mark_id());
        d.remove::<DragMark>(line_mark_id());
        d.remove::<DragMark>(layout_mark_id());
    });
}

/// The gradient a variation applied to the object at a position, and the colour it was built
/// from.
#[derive(Clone)]
struct Variation(wordcraft_doc::Pos, Fill, Rgb);

/// Stable storage id for the gradient variation remembered for the last object.
fn variation_id() -> Id {
    Id::unique("format_shape.variation")
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

/// All selected shapes and drawing canvases (or the active text box containing the caret).
fn target_shapes(app: &WordApp) -> Vec<(wordcraft_doc::Pos, InlineObject)> {
    wordcraft_engine::cmd::objects::target_objects(&app.session, |o| matches!(o, InlineObject::Shape { .. }) || o.is_canvas())
}

/// The first selected shape's or canvas's object: `None` with no shape or canvas active.
pub(crate) fn current_object(app: &WordApp) -> Option<(wordcraft_doc::Pos, InlineObject)> {
    target_shapes(app).into_iter().next()
}

fn fill_of(o: &InlineObject) -> Option<Fill> {
    match o {
        InlineObject::Shape { fill, .. } => fill.clone(),
        o => o.canvas_style().and_then(|c| c.fill.clone()),
    }
}

fn fill_kind_of(fill: &Option<Fill>) -> u8 {
    match fill {
        None => 0,
        Some(Fill::Solid { .. }) => 1,
        Some(Fill::Gradient(_)) => 2,
    }
}

/// The first selected shape's or canvas's fill: `None` with no shape selected.
pub(crate) fn current(app: &WordApp) -> Option<Option<Fill>> {
    let (_, o) = current_object(app)?;
    match o {
        InlineObject::Shape { fill, .. } => Some(fill),
        o => o.canvas_style().map(|c| c.fill.clone()),
    }
}

/// The selected shape's outline properties (summarized across all selected shapes/canvases).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CurrentLine {
    /// Whether all targets have a line (`Some(true)`), none do (`Some(false)`), or mixed (`None`).
    pub has_line: Option<bool>,
    /// The representative stroke colour (`None` when no target has an outline).
    pub stroke: Option<Rgb>,
    /// The representative stroke width in points.
    pub width: f32,
    /// The representative dash style.
    pub dash: wordcraft_doc::line_dash::LineDash,
    /// Whether selected non-canvas shapes disagree on dash style.
    pub dash_mixed: bool,
    /// The representative line cap style.
    pub cap: wordcraft_doc::connector::LineCap,
    /// Whether selected non-canvas shapes disagree on cap style.
    pub cap_mixed: bool,
    /// Whether all targets are drawing canvases (canvas outlines do not support dash or cap styles).
    pub is_canvas: bool,
}

fn line_item(o: &InlineObject) -> Option<(Option<Rgb>, f32, wordcraft_doc::line_dash::LineDash, wordcraft_doc::connector::LineCap, bool)> {
    match o {
        InlineObject::Shape { stroke, stroke_width, extra, .. } => Some((*stroke, *stroke_width, extra.dash.clone(), extra.cap, false)),
        o => o
            .canvas_style()
            .map(|c| (c.stroke, c.stroke_width, wordcraft_doc::line_dash::LineDash::Solid, wordcraft_doc::connector::LineCap::Flat, true)),
    }
}

/// The selected shapes' or canvases' outline summary: `None` when no shape or canvas is selected.
pub(crate) fn current_line(app: &WordApp) -> Option<CurrentLine> {
    let shapes = target_shapes(app);
    let items: Vec<_> = shapes.iter().filter_map(|(_, o)| line_item(o)).collect();
    let first = items.first()?;
    let any_line = items.iter().any(|x| x.0.is_some());
    let all_line = items.iter().all(|x| x.0.is_some());
    let has_line = (all_line || !any_line).then_some(all_line);
    let rep = items.iter().find(|x| x.0.is_some()).unwrap_or(first);
    let shape_items: Vec<_> = items.iter().filter(|x| !x.4).collect();
    let dash_mixed = shape_items.windows(2).any(|w| w[0].2 != w[1].2);
    let cap_mixed = shape_items.windows(2).any(|w| w[0].3 != w[1].3);
    Some(CurrentLine {
        has_line,
        stroke: rep.0,
        width: rep.1,
        dash: rep.2.clone(),
        dash_mixed,
        cap: rep.3,
        cap_mixed,
        is_canvas: items.iter().all(|x| x.4),
    })
}

/// The selected shape's or text box's layout properties.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CurrentLayout {
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
    /// Rotation in degrees.
    pub rotation: f32,
    /// Whether any target is a text box.
    pub is_text_box: bool,
    /// The primary target text box's body properties.
    pub text_body: Option<wordcraft_doc::TextBody>,
    /// Whether selected text boxes disagree on vertical alignment.
    pub anchor_mixed: bool,
    /// Whether selected text boxes disagree on text direction.
    pub vert_mixed: bool,
}

/// The selected shapes' or text boxes' layout: size, rotation, and text body settings.
pub(crate) fn current_layout(app: &WordApp) -> Option<CurrentLayout> {
    let (_, o) = current_object(app)?;
    let (width, height, float) = o.frame()?;
    let bodies: Vec<wordcraft_doc::TextBody> = wordcraft_engine::cmd::objects::text_box_targets(&app.session)
        .into_iter()
        .filter_map(|(_, id)| app.session.doc.parts.get(&id).map(|p| p.body))
        .collect();
    let is_text_box = !bodies.is_empty();
    let text_body = bodies.first().copied();
    let anchor_mixed = bodies.windows(2).any(|w| w[0].anchor != w[1].anchor);
    let vert_mixed = bodies.windows(2).any(|w| w[0].vert != w[1].vert);
    Some(CurrentLayout { width, height, rotation: float.spin().deg, is_text_box, text_body, anchor_mixed, vert_mixed })
}

/// Runs `cmd` with `params`. While `resp` is being dragged, subsequent frames join the drag's
/// initial undo checkpoint via `Session::join_next_undo` so long drags never evict older history.
pub(crate) fn run_response_command(app: &mut WordApp, ui: &Ui, drag_id: Id, resp: Option<&Response>, cmd: &str, params: serde_json::Value) {
    let Some(r) = resp.filter(|r| r.dragged()) else {
        let _ = app.run(cmd, params);
        return;
    };
    if r.drag_started() {
        ui.data_mut(|d| d.remove::<DragMark>(drag_id));
    }
    let current = DragMark { mark: app.session.undo_mark(), widget: r.id };
    if ui.data(|d| d.get_temp::<DragMark>(drag_id)) == Some(current) {
        app.session.join_next_undo();
        let _ = app.run(cmd, params);
    } else {
        let before = app.session.undo_mark();
        let _ = app.run(cmd, params);
        let after = app.session.undo_mark();
        if after > before {
            ui.data_mut(|d| d.insert_temp(drag_id, DragMark { mark: after, widget: r.id }));
        }
    }
}

/// Sets the selected shapes' fill. While `resp` is being dragged each change is applied and
/// joined into one undo step.
fn apply(app: &mut WordApp, ui: &Ui, fill: Option<&Fill>, resp: Option<&Response>) {
    run_response_command(app, ui, mark_id(), resp, "shape.fill", shape_fill::params(fill));
}

/// The pane, while open.
pub fn pane(app: &mut WordApp, ui: &mut Ui) {
    // End any in-flight drag when no pointer button is down so the next drag gets its own undo step.
    if !ui.input(|i| i.pointer.any_down()) {
        forget_drag(ui.ctx());
    }
    let shapes = target_shapes(app);
    let Some((_, first)) = shapes.first() else {
        ui.label(egui::RichText::new(tl!("Select a shape, text box or drawing canvas to format it.")).weak());
        return;
    };
    let fill = fill_of(first);
    let theme = app.session.doc.settings.theme_colors.clone();
    ui.label(egui::RichText::new(tl!("Fill")).strong());
    ui.add_space(2.0);
    let first_kind = fill_kind_of(&fill);
    let fill_mixed = shapes.iter().skip(1).any(|(_, o)| fill_kind_of(&fill_of(o)) != first_kind);
    let fill_kind = (!fill_mixed).then_some(first_kind);
    let mut picked_fill = None;
    for (value, label) in [(0u8, tl!("No fill")), (1, tl!("Solid fill")), (2, tl!("Gradient fill"))] {
        if ui.radio(fill_kind == Some(value), label).clicked() && fill_kind != Some(value) {
            picked_fill = Some(value);
        }
    }
    if let Some(pick) = picked_fill {
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

    if let Some(line) = current_line(app) {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        line_section(app, ui, &theme, &line);
    }

    if let Some(layout) = current_layout(app) {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        layout_section(app, ui, &layout);
    }
}

/// Draws a labelled preset ComboBox for an optional current choice (`None` when mixed or custom)
/// and returns the newly picked preset parameter when the selection changes.
fn preset_combo<'a>(
    ui: &mut Ui,
    label_text: &str,
    id_salt: &str,
    options: &[(&str, &'a str)],
    orig_idx: Option<usize>,
    custom_label: Option<&str>,
) -> Option<&'a str> {
    let mut pick = orig_idx;
    let selected_text = custom_label.unwrap_or_else(|| orig_idx.map(|i| tl!(options[i].0)).unwrap_or(""));
    ui.horizontal(|ui| {
        let label = ui.label(label_text);
        let combo = craft_ui::ComboBox::from_id_salt(id_salt).selected_text(selected_text).show_ui(ui, |ui| {
            if let Some(custom) = custom_label {
                ui.add_enabled_ui(false, |ui| {
                    let _ = ui.selectable_label(true, custom);
                });
            }
            for (i, (opt_label, _)) in options.iter().enumerate() {
                ui.selectable_value(&mut pick, Some(i), tl!(opt_label));
            }
        });
        combo.response.labelled_by(label.id);
    });
    pick.filter(|&i| Some(i) != orig_idx).map(|i| options[i].1)
}

/// Draws the Line section: No line or Solid line (color, width, dash type, cap type).
fn line_section(app: &mut WordApp, ui: &mut Ui, theme: &[Rgb], line: &CurrentLine) {
    ui.label(egui::RichText::new(tl!("Line")).strong());
    ui.add_space(2.0);
    let mut picked_line = None;
    for (target, label) in [(false, tl!("No line")), (true, tl!("Solid line"))] {
        if ui.radio(line.has_line == Some(target), label).clicked() && line.has_line != Some(target) {
            picked_line = Some(target);
        }
    }
    if let Some(solid_line) = picked_line {
        if !solid_line {
            let _ = app.run("shape.outline", json!({"color": null}));
        } else {
            let accent1 = theme.get(4).copied().unwrap_or(Rgb(68, 114, 196));
            let w = if line.width > 0.0 { line.width } else { 0.75 };
            let _ = app.run("shape.outline", json!({"color": accent1.hex(), "width": w}));
        }
        return;
    }
    if let Some(color) = line.stroke {
        ui.add_space(6.0);
        let mut c = color;
        if color_field(ui, theme, Id::unique("format_shape.line_color"), &mut c) {
            let _ = app.run("shape.outline", json!({"color": c.hex()}));
        }
        ui.horizontal(|ui| {
            let label = ui.label(tl!("Width"));
            let mut w = line.width;
            let r = ui
                .add(egui::DragValue::new(&mut w).range(0.25..=72.0).clamp_existing_to_range(false).suffix(" pt").speed(0.25).fixed_decimals(2))
                .labelled_by(label.id);
            if r.changed() {
                run_response_command(app, ui, line_mark_id(), Some(&r), "shape.outline", json!({"width": w}));
            }
        });

        if !line.is_canvas {
            const DASHES: [(&str, &str); 11] = [
                ("Solid", "solid"),
                ("Round Dot", "dot"),
                ("Square Dot", "sysDot"),
                ("Dash", "dash"),
                ("Dash Dot", "dashDot"),
                ("Long Dash", "lgDash"),
                ("Long Dash Dot", "lgDashDot"),
                ("Long Dash Dot Dot", "lgDashDotDot"),
                ("System Dash", "sysDash"),
                ("System Dash Dot", "sysDashDot"),
                ("System Dash Dot Dot", "sysDashDotDot"),
            ];
            let is_custom = !line.dash_mixed && matches!(line.dash, wordcraft_doc::line_dash::LineDash::Custom(_));
            let orig_dash_idx = if line.dash_mixed || is_custom {
                None
            } else {
                let current_dash_str = line.dash.ooxml().unwrap_or("solid");
                DASHES.iter().position(|(_, k)| *k == current_dash_str).or(Some(0))
            };
            if let Some(dash) = preset_combo(ui, tl!("Dash type"), "format_shape.dash_type", &DASHES, orig_dash_idx, is_custom.then(|| tl!("Custom")))
            {
                let _ = app.run("shape.outline", json!({"dash": dash}));
            }

            const CAPS: [(&str, &str); 3] = [("Flat", "flat"), ("Round", "rnd"), ("Square", "sq")];
            let orig_cap_idx = if line.cap_mixed {
                None
            } else {
                let current_cap_str = line.cap.ooxml();
                CAPS.iter().position(|(_, k)| *k == current_cap_str).or(Some(0))
            };
            if let Some(cap) = preset_combo(ui, tl!("Cap type"), "format_shape.cap_type", &CAPS, orig_cap_idx, None) {
                let _ = app.run("shape.outline", json!({"cap": cap}));
            }
        }
    }
}

/// Draws the Size & Properties / Text Box layout section.
fn layout_section(app: &mut WordApp, ui: &mut Ui, layout: &CurrentLayout) {
    ui.label(egui::RichText::new(tl!("Layout & Properties")).strong());
    ui.add_space(2.0);

    ui.horizontal(|ui| {
        let label = ui.label(tl!("Width"));
        let mut w = layout.width;
        let r = ui
            .add(egui::DragValue::new(&mut w).range(1.0..=4000.0).clamp_existing_to_range(false).suffix(" pt").speed(1.0).fixed_decimals(1))
            .labelled_by(label.id);
        if r.changed() {
            run_response_command(app, ui, layout_mark_id(), Some(&r), "picture.size", json!({"width": w, "lockAspect": false}));
        }
    });
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Height"));
        let mut h = layout.height;
        let r = ui
            .add(egui::DragValue::new(&mut h).range(1.0..=4000.0).clamp_existing_to_range(false).suffix(" pt").speed(1.0).fixed_decimals(1))
            .labelled_by(label.id);
        if r.changed() {
            run_response_command(app, ui, layout_mark_id(), Some(&r), "picture.size", json!({"height": h, "lockAspect": false}));
        }
    });
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Rotation"));
        let mut rot = layout.rotation;
        let r = ui
            .add(egui::DragValue::new(&mut rot).range(0.0..=360.0).clamp_existing_to_range(false).suffix("°").speed(1.0).fixed_decimals(1))
            .labelled_by(label.id);
        if r.changed() {
            run_response_command(app, ui, layout_mark_id(), Some(&r), "arrange.rotation", json!({"degrees": rot}));
        }
    });

    if layout.is_text_box || layout.text_body.is_some() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tl!("Text Box")).strong());
        ui.add_space(2.0);

        let body = layout.text_body.unwrap_or_default();

        const VALIGNS: [(wordcraft_doc::props::VAlign, &str, &str); 3] = [
            (wordcraft_doc::props::VAlign::Top, "Top", "t"),
            (wordcraft_doc::props::VAlign::Center, "Middle", "ctr"),
            (wordcraft_doc::props::VAlign::Bottom, "Bottom", "b"),
        ];
        const VALIGN_OPTS: [(&str, &str); 3] = [("Top", "t"), ("Middle", "ctr"), ("Bottom", "b")];
        let orig_valign_idx = (!layout.anchor_mixed).then(|| VALIGNS.iter().position(|(a, _, _)| *a == body.anchor).unwrap_or(0));
        if let Some(anchor) = preset_combo(ui, tl!("Vertical alignment"), "format_shape.valign", &VALIGN_OPTS, orig_valign_idx, None) {
            let _ = app.run("shape.alignText", json!({"anchor": anchor}));
        }

        const DIRS: [(wordcraft_doc::props::TextVert, &str, &str); 4] = [
            (wordcraft_doc::props::TextVert::Horz, "Horizontal", "horz"),
            (wordcraft_doc::props::TextVert::Vert, "Rotate all text 90°", "vert"),
            (wordcraft_doc::props::TextVert::Vert270, "Rotate all text 270°", "vert270"),
            (wordcraft_doc::props::TextVert::Stacked, "Stacked", "wordArtVert"),
        ];
        const DIR_OPTS: [(&str, &str); 4] =
            [("Horizontal", "horz"), ("Rotate all text 90°", "vert"), ("Rotate all text 270°", "vert270"), ("Stacked", "wordArtVert")];
        let orig_dir_idx = (!layout.vert_mixed).then(|| DIRS.iter().position(|(d, _, _)| *d == body.vert).unwrap_or(0));
        if let Some(dir) = preset_combo(ui, tl!("Text direction"), "format_shape.text_direction", &DIR_OPTS, orig_dir_idx, None) {
            let _ = app.run("shape.textDirection", json!({"dir": dir}));
        }
    }
}

/// Draws a solid fill's colour field and transparency slider, applying each changed fill as a
/// whole.
fn solid(app: &mut WordApp, ui: &mut Ui, theme: &[Rgb], color: Rgb, transparency: f32) {
    let mut c = color;
    if color_field(ui, theme, Id::unique("format_shape.solid_color"), &mut c) {
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
        let label = ui.label(label);
        let s = ui.add(egui::Slider::new(v, 0.0..=100.0).suffix("%").fixed_decimals(0)).labelled_by(label.id);
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

/// Draws the gradient preset gallery and type, direction and angle controls, the stop bar with
/// add and remove, and the selected stop's colour, position, transparency and rotate-with-shape
/// controls.
fn gradient(app: &mut WordApp, ui: &mut Ui, theme: &[Rgb], g: &Gradient) {
    crate::rtl::grid(ui, egui::Grid::new("format_shape.gradient").num_columns(2).spacing(vec2(8.0, 6.0)), |ui| {
        ui.label(tl!("Preset gradients"));
        ui.menu_button("▾", |ui| {
            if let Some(f) = presets_gallery(ui, theme) {
                apply(app, ui, Some(&f), None);
                ui.close();
            }
        });
        ui.end_row();

        let type_label = ui.label(tl!("Type"));
        let t = type_of(&g.kind);
        let mut pick = t;
        let combo = craft_ui::ComboBox::from_id_salt("format_shape.type").selected_text(tl!(TYPES[t].0)).show_ui(ui, |ui| {
            for (i, (label, _)) in TYPES.iter().enumerate() {
                ui.selectable_value(&mut pick, i, tl!(label));
            }
        });
        combo.response.labelled_by(type_label.id);
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
            // Not `clamp_existing_to_range`: egui would clamp a document's 359.95° to 359.9° and
            // report it as a change, rewriting the fill unasked.
            let mut a = angle;
            let r = ui.add(egui::DragValue::new(&mut a).range(0.0..=359.9).clamp_existing_to_range(false).suffix("°").speed(1.0));
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
    if color_field(ui, theme, Id::unique("format_shape.stop_color"), &mut c) {
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
    let grab_id = Id::unique("format_shape.grabbed");
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
    crate::rtl::grid(ui, egui::Grid::new("format_shape.presets").spacing(vec2(4.0, 4.0)), |ui| {
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
    crate::rtl::grid(ui, egui::Grid::new("format_shape.directions").spacing(vec2(4.0, 4.0)), |ui| {
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
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

    /// Showing the pane leaves a linear angle the Angle field can't type (359.95°) alone: no
    /// edit, no undo step.
    #[test]
    fn format_shape_pane_leaves_an_angle_outside_the_field_alone() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        let stops = json!([{"pos": 0, "color": "FFFFFF"}, {"pos": 100, "color": "156082"}]);
        a.run("shape.fill", json!({"gradient": {"stops": stops, "angle": 359.95}})).unwrap();
        let (doc, depth) = (a.session.doc.clone(), a.session.undo_depth());
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(6);
        assert!(h.query_by_label("Angle").is_some(), "the pane shows the gradient's fields");
        let Some(Fill::Gradient(g)) = fill(h.state()) else { panic!("{:?}", fill(h.state())) };
        assert_eq!(g.kind, wordcraft_doc::fill::GradientKind::Linear { angle: 359.95, scaled: false });
        assert!(h.state().session.doc == doc);
        assert_eq!(h.state().session.undo_depth(), depth);
    }

    fn stroke(app: &WordApp) -> Option<wordcraft_doc::Rgb> {
        match super::current_object(app) {
            Some((_, InlineObject::Shape { stroke, .. })) => stroke,
            o => panic!("{o:?}"),
        }
    }

    /// Issue #5896: Format Shape pane provides Line settings: "No line" removes the frame,
    /// and "Solid line" restores it with color and width.
    #[test]
    fn format_shape_pane_removes_and_restores_line() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.textBox", json!({"text": "Hello"})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert_eq!(stroke(h.state()), Some(wordcraft_doc::Rgb::BLACK), "default textbox has black frame");

        // Click "No line" in the Format Shape pane:
        h.get_by_label("No line").click();
        h.run_steps(3);
        assert_eq!(stroke(h.state()), None, "No line removes the frame");

        // Click "Solid line" in the Format Shape pane:
        h.get_by_label("Solid line").click();
        h.run_steps(3);
        assert!(stroke(h.state()).is_some(), "Solid line restores the frame");
    }

    /// Format Shape pane displays layout and text box sections, allowing layout adjustments.
    #[test]
    fn format_shape_pane_exposes_layout_and_text_box() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.textBox", json!({"text": "Box content"})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Layout & Properties").is_some());
        assert!(h.query_by_label("Text Box").is_some());
        assert!(h.query_by_label("Vertical alignment").is_some());
        assert!(h.query_by_label("Text direction").is_some());
    }

    /// Showing the pane leaves a shape with dimensions outside the DragValue range alone.
    #[test]
    fn format_shape_pane_leaves_out_of_range_layout_alone() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("picture.size", json!({"width": 2500.0, "lockAspect": false})).unwrap();
        let (doc, depth) = (a.session.doc.clone(), a.session.undo_depth());
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert_eq!(h.state().session.undo_depth(), depth, "opening pane does not mutate size");
        assert!(h.state().session.doc == doc);
    }

    /// A drawing canvas outline hides dash and cap type controls.
    #[test]
    fn format_shape_pane_hides_dash_and_cap_for_canvas() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.canvas", json!({"width": 300, "height": 200})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Line").is_some());
        assert!(h.query_by_label("Solid line").is_some());
        assert!(h.query_by_label("Dash type").is_none(), "canvas does not show dash type");
        assert!(h.query_by_label("Cap type").is_none(), "canvas does not show cap type");
    }

    /// System dash presets and custom dashes are preserved and displayed correctly.
    #[test]
    fn format_shape_pane_preserves_custom_and_system_dashes() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.outline", json!({"dash": "sysDash"})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let line = super::current_line(h.state()).expect("shape has line");
        assert_eq!(line.dash, wordcraft_doc::line_dash::LineDash::SystemDash);

        // Custom dashes show as "Custom" and are preserved:
        let (pos, _) = wordcraft_engine::cmd::objects::selected(&h.state().session).unwrap();
        let p = h.state_mut().session.doc.para_mut(pos.story, &pos.path).unwrap();
        let InlineObject::Shape { extra, .. } = p.object_at_mut(pos.off).unwrap() else { panic!("expected shape") };
        extra.dash = wordcraft_doc::line_dash::LineDash::Custom(vec![(100_000, 100_000)]);
        h.run_steps(4);

        let line = super::current_line(h.state()).expect("shape has line");
        assert!(matches!(line.dash, wordcraft_doc::line_dash::LineDash::Custom(_)), "custom dash is preserved");
    }

    /// Issue #5896 P2: when typing inside a text box inside a drawing canvas, Format Shape shows
    /// the member's controls and "No line" removes the member's border without clearing the canvas's.
    #[test]
    fn format_shape_pane_targets_canvas_text_box_member_while_typing() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.canvas", json!({"width": 300, "height": 200, "outline": "1F4E79"})).unwrap();
        a.run("insert.textBox", json!({"text": "Canvas text"})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Dash type").is_some(), "member text box shows Dash type");
        assert!(h.query_by_label("Cap type").is_some(), "member text box shows Cap type");
        assert!(h.query_by_label("Text Box").is_some(), "member text box shows Text Box section");

        h.get_by_label("No line").click();
        h.run_steps(3);
        let canvas = h.state().session.doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.objects.first()).unwrap();
        assert_eq!(canvas.canvas_style().and_then(|st| st.stroke), Some(wordcraft_doc::Rgb(0x1F, 0x4E, 0x79)));
        let InlineObject::Group { children, .. } = canvas else { panic!("{canvas:?}") };
        assert!(matches!(&children[0].obj, InlineObject::Shape { stroke: None, .. }));
    }

    /// Issue #5896: on a mixed selection where the primary shape has no line and another shape
    /// has an outline (or where a text box is added after a rectangle), clicking "No line" applies
    /// to every selected shape and the Text Box section remains accessible.
    #[test]
    fn format_shape_pane_applies_line_choice_to_mixed_selection() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.outline", json!({"color": null})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(a.session.doc.end_of(wordcraft_doc::StoryRef::Body));
        a.run("insert.textBox", json!({"text": "Mixed"})).unwrap();
        a.run("select.objects", json!({"index": 0})).unwrap();
        a.run("select.addObject", json!({"index": 1})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();

        let line = super::current_line(&a).expect("mixed selection has line info");
        assert_eq!(line.has_line, None, "mixed No line / Solid line state");
        let layout = super::current_layout(&a).expect("mixed selection has layout info");
        assert!(layout.is_text_box, "added text box exposes Text Box section");

        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Text Box").is_some(), "Text Box controls shown for added text box");

        // Primary shape has no line, second shape has an outline: clicking "No line" clears both.
        h.get_by_label("No line").click();
        h.run_steps(3);
        let para = h.state().session.doc.body.first().and_then(|b| b.as_para()).unwrap();
        for (i, off) in para.object_offsets().into_iter().enumerate() {
            let InlineObject::Shape { stroke, .. } = para.object_at(off).unwrap() else { panic!("expected shape {i}") };
            assert_eq!(*stroke, None, "shape {i} outline cleared");
        }
    }

    /// Issue #5896: dragging a Format Shape slider/DragValue joins updates into its initial
    /// undo checkpoint on the fly so drags past MAX_UNDO (500) never evict earlier undo steps.
    #[test]
    fn format_shape_pane_drag_coalesces_into_single_undo_step() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.fill", json!({"color": "156082"})).unwrap();
        a.run("arrange.rotation", json!({"degrees": 180.0})).unwrap();
        let depth_before = a.session.undo_depth();

        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(320.0, 600.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                super::pane(app, ui);
            },
            a,
        );
        h.run_steps(2);

        let rot_rect = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Rotation").rect();
        let mut pos = rot_rect.center();
        h.event(egui::Event::PointerMoved(pos));
        h.step();
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();

        // 510 changed frames (> MAX_UNDO = 500): sweep within 80..=280 so no frame clamps to a no-op.
        for i in 0..510 {
            let dx = if (i / 100) % 2 == 0 { 1.0 } else { -1.0 };
            pos.x += dx;
            h.event(egui::Event::PointerMoved(pos));
            h.step();
        }
        assert_eq!(h.state().session.undo_depth(), depth_before + 1, "all 510 drag frames share one checkpoint without eviction");

        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);
        assert_eq!(h.state().session.undo_depth(), depth_before + 1);
        assert_ne!(super::current_layout(h.state()).unwrap().rotation, 180.0, "drag changed rotation");

        assert!(h.state_mut().session.undo());
        assert_eq!(super::current_layout(h.state()).unwrap().rotation, 180.0, "single Undo restores pre-drag angle");
        assert_eq!(h.state().session.undo_depth(), depth_before, "earlier undo history stayed intact");
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
