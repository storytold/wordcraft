//! The Format Shape pane (Shape Fill › More Gradients…, a shape's Format Shape…): the selected
//! shapes' fill, outline, and layout settings (size, rotation, text direction, and alignment).
//! Every change runs the corresponding shape command; a drag applies as it goes and undoes in one step.

use std::sync::Arc;

use craft::behavior::{EditEvents, Mixed, Value};
use craft::color_ui::gradient::{ColorStop, GradientStops, StopRows, StopRules};
use craft::color_ui::ui::gradient::{GradientEditor, StopRef};
// PLAN: assumed craft::draw::ui exports these (ch.C15)
use craft::draw::ui::{FocusOutline, GradientGroup, GradientItem, GradientPalette, GradientRef, GradientStyle, GradientTile};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Outcome as _, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::form::{Field, SliderField};
use craft::ui::layout_components::Stack;
use craft::ui::menu::Menu;
use craft::ui::notices::Notice;
use craft::ui::patterns::presets::PresetState;
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft::ui::sections::{HeaderStrength, Section};
use craft::ui::text::NumberField;
use egui::{Ui, pos2};
use serde_json::json;
use wordcraft_doc::Rgb;
use wordcraft_doc::fill::{Fill, Gradient, GradientKind, PathShape};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::wordart::GradStop;
use wordcraft_engine::cmd::shape_fill::{self, LINEAR_DIRECTIONS, PATH_DIRECTIONS};

use crate::WordApp;
use crate::colors::{self, ColorFieldEmpty, Palette, Target, color_field};

/// The gradient a variation applied to the object at a position, and the colour it was built
/// from. The window keeps the last one (`WindowUi.shape_variation`).
#[derive(Clone)]
pub(crate) struct Variation(pub(crate) wordcraft_doc::Pos, pub(crate) Fill, pub(crate) Rgb);

/// The colour Shape Fill › Gradient builds from for the object at `pos` filled with `fill`:
/// the colour the variation it shows was made from (so picking variations in turn, or No
/// Gradient, doesn't drift), else the fill's main colour.
// ponytail: remembered for the last variation only, in this window; a gradient from a file
// uses its middle stop (Word keeps the scheme colour it was built from).
pub(crate) fn variation_base(app: &WordApp, pos: &wordcraft_doc::Pos, fill: &Fill) -> Rgb {
    // PLAN: assumed WindowUi.shape_variation: Option<Variation>, reached as `app.shape_variation` like `app.spelling_review` (app-hub)
    match &app.shape_variation {
        Some(Variation(p, f, base)) if p == pos && f == fill => *base,
        _ => fill.main_color(),
    }
}

/// Notes that `fill`, made from `base`, was applied to the object at `pos`.
pub(crate) fn remember_variation(app: &mut WordApp, pos: wordcraft_doc::Pos, fill: Fill, base: Rgb) {
    app.shape_variation = Some(Variation(pos, fill, base));
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
    let shape_rep = shape_items.first().copied().unwrap_or(rep);
    let dash_mixed = shape_items.windows(2).any(|w| w[0].2 != w[1].2);
    let cap_mixed = shape_items.windows(2).any(|w| w[0].3 != w[1].3);
    Some(CurrentLine {
        has_line,
        stroke: rep.0,
        width: rep.1,
        dash: shape_rep.2.clone(),
        dash_mixed,
        cap: shape_rep.3,
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

/// Sets the selected shapes' fill.
fn apply(app: &mut WordApp, fill: Option<&Fill>) {
    let _ = app.run("shape.fill", shape_fill::params(fill));
}

/// Applies a field's edits: a drag is one undo step through the one gesture path.
fn apply_fill_edits<T: Value>(app: &mut WordApp, events: &EditEvents<T>, fill: impl Fn(&T) -> Option<Fill>) {
    // PLAN: assumed crate::undo_marks::apply_edits<T: Value>(app: &mut WordApp, events: EditEvents<T>, apply: impl Fn(&mut WordApp, T)) (rb.A1, ribbon sets)
    crate::undo_marks::apply_edits(app, events.clone(), |app, value| {
        if let Some(f) = fill(&value) {
            apply(app, Some(&f));
        }
    });
}

fn srgba(c: Rgb, alpha: f32) -> Srgba {
    Srgba::new(f32::from(c.0) / 255.0, f32::from(c.1) / 255.0, f32::from(c.2) / 255.0, alpha)
}

fn rgb_of(c: Srgba) -> Rgb {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb(byte(c.r), byte(c.g), byte(c.b))
}

/// The document's stops as the editor's: one list, the opacity in each colour's alpha.
fn stops_of(g: &Gradient) -> GradientStops {
    GradientStops::new(g.stops.iter().map(|s| ColorStop::new(s.pos / 100.0, srgba(s.color, 1.0 - s.transparency / 100.0))).collect())
}

/// The gradient `g` with the editor's stops, in the editor's order.
fn with_stops(g: &Gradient, stops: &GradientStops) -> Fill {
    with(g, |g| {
        g.stops = stops
            .color
            .iter()
            .map(|s| GradStop {
                pos: (s.location * 100.0).clamp(0.0, 100.0),
                color: rgb_of(s.color),
                transparency: ((1.0 - s.color.a) * 100.0).clamp(0.0, 100.0),
            })
            .collect();
    })
}

/// The stops and the tile a swatch or palette cell draws `fill` with. The document's angle runs
/// clockwise and a tile's counter-clockwise, so the angle is negated here, the one conversion.
pub(crate) fn fill_tile(fill: &Fill) -> (GradientStops, GradientTile) {
    match fill {
        Fill::Solid { color, transparency } => {
            let c = srgba(*color, 1.0 - transparency / 100.0);
            (GradientStops::new(vec![ColorStop::new(0.0, c), ColorStop::new(1.0, c)]), GradientTile::Ramp)
        }
        Fill::Gradient(g) => {
            let tile = match g.kind {
                GradientKind::Linear { angle, .. } => GradientTile::style(GradientStyle::Linear, -angle),
                GradientKind::Path { shape, focus } => {
                    let [l, t, r, b] = focus;
                    GradientTile::focus(
                        if shape == PathShape::Circle { FocusOutline::Circle } else { FocusOutline::Rect },
                        egui::Rect::from_min_max(pos2(l / 100.0, t / 100.0), pos2(1.0 - r / 100.0, 1.0 - b / 100.0)),
                    )
                }
            };
            (stops_of(g), tile)
        }
    }
}

/// The pane, while open.
pub fn pane(app: &mut WordApp, ui: &mut Ui) {
    let shapes = target_shapes(app);
    let Some((_, first)) = shapes.first() else {
        Notice::new(Tone::Neutral, tl!("Select a shape, text box or drawing canvas to format it.")).show(ui);
        return;
    };
    let fill = fill_of(first);
    let theme = app.session.doc.settings.theme_colors.clone();
    let first_kind = fill_kind_of(&fill);
    let fill_mixed = shapes.iter().skip(1).any(|(_, o)| fill_kind_of(&fill_of(o)) != first_kind);
    let shown_kind = if fill_mixed { Mixed::Mixed } else { Mixed::Same(first_kind) };
    let shown = RadioGroup::new(tl!("Fill"), Bind::given(shown_kind)).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Radio::new(0u8).label(tl!("No fill")).show(ui);
            Radio::new(1u8).label(tl!("Solid fill")).show(ui);
            Radio::new(2u8).label(tl!("Gradient fill")).show(ui);
        });
    });
    if let Some(pick) = shown.output.events.committed().copied()
        && (fill_mixed || pick != first_kind)
    {
        let accent1 = theme.get(4).copied().unwrap_or(Rgb(68, 114, 196));
        let main = fill.as_ref().map_or(accent1, Fill::main_color);
        let new = match pick {
            1 => Some(Fill::solid(main)),
            2 => Some(shape_fill::default_gradient(accent1)),
            _ => None,
        };
        apply(app, new.as_ref());
        return;
    }
    match fill {
        Some(Fill::Solid { color, transparency }) => solid(app, ui, &theme, color, transparency),
        Some(Fill::Gradient(g)) => gradient(app, ui, &g),
        None => {}
    }

    if let Some(line) = current_line(app) {
        Section::fixed(tl!("Line")).show(ui, |ui, _| line_section(app, ui, &theme, &line));
    }

    if let Some(layout) = current_layout(app) {
        Section::fixed(tl!("Layout & Properties")).show(ui, |ui, _| layout_section(app, ui, &layout));
    }
}

/// Draws a labelled preset Select for an optional current choice (`None` when mixed or custom)
/// and returns the newly picked preset parameter when the selection changes.
fn preset_select<'a>(
    ui: &mut Ui,
    label_text: &str,
    id_salt: &str,
    options: &[(&str, &'a str)],
    orig_idx: Option<usize>,
    mixed: bool,
    custom_label: Option<&str>,
) -> Option<&'a str> {
    let entries: Vec<Entry<usize>> = options.iter().enumerate().map(|(i, (opt_label, _))| Entry::new(i, tl!(opt_label).to_owned())).collect();
    let shown = if mixed { Mixed::Mixed } else { Mixed::Same(orig_idx.unwrap_or(usize::MAX)) };
    let label = Label::from(label_text.to_owned());
    let custom = custom_label.map(|c| Label::from(c.to_owned()));
    let picked = Field::new(label.clone())
        .id_salt(id_salt)
        .show(ui, |ui, _| {
            let mut select = Select::new(label.clone(), Bind::given(shown), &entries);
            if let Some(custom) = custom.clone() {
                select = select.placeholder(custom);
            }
            select.show(ui).events.committed().copied()
        })
        .inner
        .flatten();
    picked.filter(|&i| Some(i) != orig_idx).and_then(|i| options.get(i).map(|o| o.1))
}

/// Draws the Line section: No line or Solid line (color, width, dash type, cap type).
fn line_section(app: &mut WordApp, ui: &mut Ui, theme: &[Rgb], line: &CurrentLine) {
    let has_line = line.has_line.map_or(Mixed::Mixed, Mixed::Same);
    let shown = RadioGroup::new(tl!("Line"), Bind::given(has_line)).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Radio::new(false).label(tl!("No line")).show(ui);
            Radio::new(true).label(tl!("Solid line")).show(ui);
        });
    });
    if let Some(solid_line) = shown.output.events.committed().copied()
        && line.has_line != Some(solid_line)
    {
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
        let palette = Palette { theme: theme.to_vec(), recent: Vec::new() };
        let mut hex = color.hex();
        let label: Label = tl!("Color").into();
        let picked = ui.push_id("line_color", |ui| color_field(ui, &label, &mut hex, &ColorFieldEmpty::NoColor, &palette)).inner;
        if picked.more_colors {
            colors::open_for(app, &Target::cmd("shape.outline"), Some(color), craft::color_ui::ui::ColorPickerTab::Standard);
            return;
        }
        if picked.changed {
            if hex.is_empty() {
                let _ = app.run("shape.outline", json!({"color": null}));
                return;
            }
            if let Some(n) = Rgb::parse(&hex) {
                let _ = app.run("shape.outline", json!({"color": n.hex()}));
            }
        }
        let mut width_events = EditEvents::new();
        Field::new(tl!("Width")).id_salt("format_shape.line_width").show(ui, |ui, _| {
            width_events = NumberField::new(tl!("Width"), Bind::given(Some(f64::from(line.width))))
                .unit_kind(UnitKind::Points)
                .range(0.25..=72.0)
                .step(0.25)
                .fixed_decimals(2)
                .show(ui)
                .edit
                .events;
        });
        crate::undo_marks::apply_edits(app, width_events, |app, v: Option<f64>| {
            if let Some(w) = v.filter(|w| (0.25..=72.0).contains(w)) {
                let _ = app.run("shape.outline", json!({"width": w}));
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
            if let Some(dash) = preset_select(
                ui,
                tl!("Dash type"),
                "format_shape.dash_type",
                &DASHES,
                orig_dash_idx,
                line.dash_mixed,
                is_custom.then(|| tl!("Custom")),
            ) {
                let _ = app.run("shape.outline", json!({"dash": dash}));
            }

            const CAPS: [(&str, &str); 3] = [("Flat", "flat"), ("Round", "rnd"), ("Square", "sq")];
            let orig_cap_idx = if line.cap_mixed {
                None
            } else {
                let current_cap_str = line.cap.ooxml();
                CAPS.iter().position(|(_, k)| *k == current_cap_str).or(Some(0))
            };
            if let Some(cap) = preset_select(ui, tl!("Cap type"), "format_shape.cap_type", &CAPS, orig_cap_idx, line.cap_mixed, None) {
                let _ = app.run("shape.outline", json!({"cap": cap}));
            }
        }
    }
}

/// Draws the Size & Properties / Text Box layout section.
fn layout_section(app: &mut WordApp, ui: &mut Ui, layout: &CurrentLayout) {
    for (label, id_salt, key, value) in
        [(tl!("Width"), "format_shape.layout_width", "width", layout.width), (tl!("Height"), "format_shape.layout_height", "height", layout.height)]
    {
        let mut events = EditEvents::new();
        Field::new(label).id_salt(id_salt).show(ui, |ui, _| {
            events = NumberField::new(label, Bind::given(Some(f64::from(value))))
                .unit_kind(UnitKind::Points)
                .range(1.0..=4000.0)
                .commit_decimals(1)
                .show(ui)
                .edit
                .events;
        });
        crate::undo_marks::apply_edits(app, events, move |app, v: Option<f64>| {
            if let Some(val) = v.filter(|val| (1.0..=4000.0).contains(val)) {
                let _ = app.run("shape.size", json!({ key: val, "lockAspect": false }));
            }
        });
    }

    let mut rot_events = EditEvents::new();
    Field::new(tl!("Rotation")).id_salt("format_shape.rotation").show(ui, |ui, _| {
        rot_events = NumberField::new(tl!("Rotation"), Bind::given(Some(f64::from(layout.rotation))))
            .unit_kind(UnitKind::Degrees)
            .range(0.0..=360.0)
            .commit_decimals(1)
            .show(ui)
            .edit
            .events;
    });
    crate::undo_marks::apply_edits(app, rot_events, |app, v: Option<f64>| {
        if let Some(degrees) = v.filter(|d| (0.0..=360.0).contains(d)) {
            let _ = app.run("shape.rotation", json!({"degrees": degrees}));
        }
    });

    if layout.is_text_box || layout.text_body.is_some() {
        let body = layout.text_body.unwrap_or_default();
        Section::fixed(tl!("Text Box")).strength(HeaderStrength::Caption).show(ui, |ui, _| {
            const VALIGNS: [(wordcraft_doc::props::VAlign, &str, &str); 3] = [
                (wordcraft_doc::props::VAlign::Top, "Top", "t"),
                (wordcraft_doc::props::VAlign::Center, "Middle", "ctr"),
                (wordcraft_doc::props::VAlign::Bottom, "Bottom", "b"),
            ];
            const VALIGN_OPTS: [(&str, &str); 3] = [("Top", "t"), ("Middle", "ctr"), ("Bottom", "b")];
            let orig_valign_idx = (!layout.anchor_mixed).then(|| VALIGNS.iter().position(|(a, _, _)| *a == body.anchor).unwrap_or(0));
            if let Some(anchor) =
                preset_select(ui, tl!("Vertical alignment"), "format_shape.valign", &VALIGN_OPTS, orig_valign_idx, layout.anchor_mixed, None)
            {
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
            if let Some(dir) =
                preset_select(ui, tl!("Text direction"), "format_shape.text_direction", &DIR_OPTS, orig_dir_idx, layout.vert_mixed, None)
            {
                let _ = app.run("shape.textDirection", json!({"dir": dir}));
            }
        });
    }
}

/// Draws a solid fill's colour field and transparency slider, applying each changed fill as a
/// whole.
fn solid(app: &mut WordApp, ui: &mut Ui, theme: &[Rgb], color: Rgb, transparency: f32) {
    // PLAN: the recent colours are the colour drop-downs' (rb.A4); none are offered here
    let palette = Palette { theme: theme.to_vec(), recent: Vec::new() };
    let mut hex = color.hex();
    let label: Label = tl!("Color").into();
    let picked = color_field(ui, &label, &mut hex, &ColorFieldEmpty::NoColor, &palette);
    if picked.changed
        && let Some(n) = Rgb::parse(&hex)
    {
        let _ = app.run("shape.fill", json!({"color": n.hex()}));
        return;
    }
    let out = SliderField::new(tl!("Transparency"), Bind::given(f64::from(transparency)), 0.0..=100.0).unit_kind(UnitKind::Percent).show(ui);
    apply_fill_edits(app, &out.events, |v| Some(Fill::Solid { color, transparency: *v as f32 }));
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

/// A button that opens a menu of gradient tiles, a `GradientPalette` of square cells; the index of
/// the tile picked, which also closes the menu. Each tile's name is its accessible name. `key` names
/// the menu's open flag, which the framework's menu writes through the binding.
fn tile_menu(ui: &mut Ui, label: &str, tiles: &[(Fill, String)], key: &'static str) -> Option<usize> {
    let items: Vec<GradientItem> = tiles
        .iter()
        .map(|(f, name)| {
            let (stops, tile) = fill_tile(f);
            GradientItem::new(name.clone(), stops).tile(tile)
        })
        .collect();
    let groups = [GradientGroup::new(label.to_string(), items)];
    let opener = Button::new(label.to_string()).show(ui);
    let ctx = ui.ctx().clone();
    let key = SaveKey::named(key);
    // PLAN: the framework has no public kept-value write; the open flag goes through the saved value
    let was: bool = saved_value(&ctx, &key);
    let mut open = was;
    let mut picked = None;
    Menu::new(label.to_string()).opened_by(&opener.response, Trigger::Click).open(Bind::bound(&mut open)).show(ui, |ui, _| {
        let out = GradientPalette::new(label.to_string(), &groups, Bind::<PresetState<GradientRef>>::kept()).rows(StopRows::Combined).show(ui);
        if let Some(GradientRef::Item { item, .. }) = out.picked {
            picked = Some(item);
        }
    });
    if picked.is_some() {
        open = false;
    }
    if open != was {
        set_saved_value(&ctx, &key, open);
    }
    picked
}

/// Draws the gradient preset menu and type, direction and angle controls, the stop editor and
/// the rotate-with-shape check.
fn gradient(app: &mut WordApp, ui: &mut Ui, g: &Gradient) {
    let theme = app.session.doc.settings.theme_colors.clone();
    let t = type_of(&g.kind);
    let presets = shape_fill::presets(&theme);
    let named: Vec<(Fill, String)> = presets
        .into_iter()
        .enumerate()
        .map(|(i, f)| {
            let style = shape_fill::PRESET_STYLES.get(i / 6).copied().unwrap_or("");
            (f, format!("{}, {} {}", tl!(style), tl!("Accent"), i % 6 + 1))
        })
        .collect();
    if let Some(i) = tile_menu(ui, tl!("Preset gradients"), &named, "format_shape.presets_open")
        && let Some((f, _)) = named.get(i)
    {
        apply(app, Some(f));
        return;
    }

    let entries: Vec<Entry<u8>> = TYPES.iter().enumerate().map(|(i, (label, _))| Entry::new(i as u8, tl!(label))).collect();
    let mut type_pick = None;
    Field::new(tl!("Type")).show(ui, |ui, _| {
        type_pick = Select::new(tl!("Type"), Bind::given(t as u8), &entries).show(ui).events.committed().copied();
    });
    if let Some(pick) = type_pick
        && usize::from(pick) != t
    {
        let (angle, focus) = match g.kind {
            GradientKind::Linear { angle, .. } => (angle, [50.0; 4]),
            GradientKind::Path { focus, .. } => (90.0, focus),
        };
        // The tile rectangle stays: only a Direction pick sets a corner's.
        let kind = kind_of(usize::from(pick), focus, angle, scaled_of(&g.kind));
        apply(app, Some(&with(g, |g| g.kind = kind)));
        return;
    }

    if t != 3 {
        let kinds: Vec<(GradientKind, String)> = if t == 0 {
            let scaled = scaled_of(&g.kind);
            LINEAR_DIRECTIONS.iter().map(|&a| (GradientKind::Linear { angle: a, scaled }, shape_fill::linear_direction_name(a))).collect()
        } else {
            PATH_DIRECTIONS.iter().map(|&f| (kind_of(t, f, 90.0, true), shape_fill::path_direction_name(f))).collect()
        };
        let tiles: Vec<(Fill, String)> = kinds.iter().map(|(k, name)| (with(g, |g| set_kind(g, *k)), tl!(name).to_string())).collect();
        if let Some(i) = tile_menu(ui, tl!("Direction"), &tiles, "format_shape.directions_open")
            && let Some((kind, _)) = kinds.get(i)
        {
            apply(app, Some(&with(g, |g| set_kind(g, *kind))));
            return;
        }
    }

    if let GradientKind::Linear { angle, scaled } = g.kind {
        // The field shows a document's 359.95° as it is: a given value writes nothing until it is edited.
        let mut out = None;
        Field::new(tl!("Angle")).show(ui, |ui, _| {
            out = Some(
                NumberField::new(tl!("Angle"), Bind::given(Some(f64::from(angle)))).range(0.0..=359.9).unit_kind(UnitKind::Degrees).show(ui).edit,
            );
        });
        if let Some(edit) = out {
            apply_fill_edits(app, &edit.events, |v| v.map(|a| with(g, |g| g.kind = GradientKind::Linear { angle: a as f32, scaled })));
        }
    }

    // PLAN: add and remove stop are the editor's own (click the bar, Delete); the sheet's IconButtons would be a second editor of the selection
    let rules = StopRules::new(StopRows::Combined).stops(2, 10);
    let editor = GradientEditor::new(tl!("Gradient stops"), Bind::given(stops_of(g))).rules(rules).color_popover(false).show(ui);
    apply_fill_edits(app, &editor.edit.events, |stops| Some(with_stops(g, stops)));

    // The editor asks for a stop's colour; the app's one colour palette answers in a popover hung from the editor.
    // The stop is the editor's selected colour stop, which is the one it asked about.
    let ctx = ui.ctx().clone();
    let key = SaveKey::named("format_shape.stop_color_open");
    // PLAN: the framework has no public kept-value write; the open flag goes through the saved value
    let was: bool = saved_value(&ctx, &key);
    let mut open = was || editor.color_requested.is_some();
    let mut chosen = None;
    Popover::new(tl!("Stop color"), Bind::bound(&mut open)).opened_by(editor.response()).show(ui, |ui, _| {
        // PLAN: assumed colors::palette(ui, app, Target::Pick) -> ColorPick (ribbon-w11, rb.A4)
        chosen = colors::palette(ui, app, Target::Pick).picked;
    });
    if let (Some(color), Some(StopRef::Color(at))) = (chosen, editor.selected) {
        let mut stops = stops_of(g);
        if let Some(stop) = stops.color.get_mut(at) {
            stop.color = srgba(color, stop.color.a);
            apply(app, Some(&with_stops(g, &stops)));
        }
        open = false;
    }
    if open != was {
        set_saved_value(&ctx, &key, open);
    }

    let rot = Checkbox::new(Bind::given(g.rot_with_shape)).label(tl!("Rotate with shape")).show(ui);
    if let Some(on) = rot.events.committed().copied() {
        apply(app, Some(&with(g, |g| g.rot_with_shape = on)));
    }
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

    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
    }

    /// The Format Shape pane switches a gradient fill to solid and back to no fill, each one undo
    /// step.
    #[test]
    fn gradient_menu_and_format_shape_pane_set_the_fill() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        let stops = json!([{"pos": 0, "color": "FFFFFF"}, {"pos": 100, "color": "156082"}]);
        a.run("shape.fill", json!({"gradient": {"stops": stops, "angle": 90}})).unwrap();
        let mut h = harness(a);
        h.run_steps(3);
        assert!(matches!(fill(h.state()), Some(Fill::Gradient(_))), "{:?}", fill(h.state()));
        h.state_mut().run("view.formatShapePane", json!({"value": true})).unwrap();
        h.run_steps(3);
        let depth = h.state().session.undo_depth();
        h.get_by_label("Solid fill").click();
        h.run_steps(3);
        assert!(matches!(fill(h.state()), Some(Fill::Solid { .. })));
        // One drag of the Transparency slider across several frames is one undo step.
        let before_drag = h.state().session.undo_depth();
        let rect = h.get_by_role_and_label(egui::accesskit::Role::Slider, "Transparency").rect();
        let at = |f: f32| egui::pos2(rect.left() + rect.width() * f, rect.center().y);
        h.drag_at(at(0.2));
        h.run_steps(1);
        for f in [0.35, 0.5, 0.65] {
            h.hover_at(at(f));
            h.run_steps(1);
        }
        h.drop_at(at(0.8));
        h.run_steps(3);
        assert_eq!(h.state().session.undo_depth(), before_drag + 1, "a drag is one undo step");
        h.get_by_label("No fill").click();
        h.run_steps(3);
        assert_eq!(fill(h.state()), None);
        assert_eq!(h.state().session.undo_depth(), depth + 4 - 1, "two picks and one drag");
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
        let mut h = harness(a);
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
        let mut h = harness(a);
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
        let mut h = harness(a);
        h.run_steps(4);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Heading, "Layout & Properties").is_some());
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Heading, "Text Box").is_some());
        assert!(h.query_by_label("Vertical alignment").is_some());
        assert!(h.query_by_label("Text direction").is_some());
    }

    /// Showing the pane leaves a shape with dimensions outside the NumberField range alone.
    #[test]
    fn format_shape_pane_leaves_out_of_range_layout_alone() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("picture.size", json!({"width": 2500.0, "lockAspect": false})).unwrap();
        let (doc, depth) = (a.session.doc.clone(), a.session.undo_depth());
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = harness(a);
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
        let mut h = harness(a);
        h.run_steps(4);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Heading, "Line").is_some());
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
        let mut h = harness(a);
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
        let mut h = harness(a);
        h.run_steps(4);
        assert!(h.query_by_label("Dash type").is_some(), "member text box shows Dash type");
        assert!(h.query_by_label("Cap type").is_some(), "member text box shows Cap type");
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Heading, "Text Box").is_some(), "member text box shows Text Box section");

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

        let mut h = harness(a);
        h.run_steps(4);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Heading, "Text Box").is_some(), "Text Box controls shown for added text box");

        // Primary shape has no line, second shape has an outline: clicking "No line" clears both.
        h.get_by_label("No line").click();
        h.run_steps(3);
        let para = h.state().session.doc.body.first().and_then(|b| b.as_para()).unwrap();
        for (i, off) in para.object_offsets().into_iter().enumerate() {
            let InlineObject::Shape { stroke, .. } = para.object_at(off).unwrap() else { panic!("expected shape {i}") };
            assert_eq!(*stroke, None, "shape {i} outline cleared");
        }
    }

    /// Issue #5896: dragging a Format Shape slider/NumberField joins updates into its initial
    /// undo checkpoint on the fly so drags past MAX_UNDO (500) never evict earlier undo steps.
    #[test]
    fn format_shape_pane_drag_coalesces_into_single_undo_step() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.fill", json!({"color": "156082"})).unwrap();
        a.run("arrange.rotation", json!({"degrees": 180.0})).unwrap();
        let depth_before = a.session.undo_depth();

        let mut h = crate::test_harness::builder().with_size(egui::vec2(600.0, 800.0)).build_ui_state(
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

    /// Calls `variation_base` and `remember_variation` directly: the menu tiles that drive them are
    /// `ribbon::gradient_menu`'s (ribbon-w18).
    ///
    /// A variation's base colour is remembered per window for the object and fill it was applied
    /// to, so picking variations in turn doesn't drift; any other fill falls back to its colour.
    #[test]
    fn variations_dont_drift_and_no_gradient_restores_the_colour() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.fill", json!({"color": "156082"})).unwrap();
        let (pos, _) = wordcraft_engine::cmd::objects::selected(&a.session).unwrap();
        let base = wordcraft_doc::Rgb(0x15, 0x60, 0x82);
        let solid = super::current(&a).flatten().unwrap();
        assert_eq!(super::variation_base(&a, &pos, &solid), base);
        let tiles = wordcraft_engine::cmd::shape_fill::variations(base);
        let (variation, _) = tiles[0][0].clone();
        a.run("shape.fill", wordcraft_engine::cmd::shape_fill::params(Some(&variation))).unwrap();
        let now = super::current(&a).flatten().unwrap();
        super::remember_variation(&mut a, pos.clone(), now.clone(), base);
        assert_eq!(super::variation_base(&a, &pos, &now), base, "the variation shown keeps the colour it was made from");
        assert_eq!(super::variation_base(&a, &pos, &solid), solid.main_color(), "another fill does not");
    }

    /// Mixed canvas + dashed/round-capped shape selection reads dash and cap from the shape,
    /// not from the canvas placeholder.
    #[test]
    fn current_line_reads_dash_and_cap_from_non_canvas_shape() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.canvas", json!({"width": 300, "height": 200, "outline": "1F4E79"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(a.session.doc.end_of(wordcraft_doc::StoryRef::Body));
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.outline", json!({"dash": "dash", "cap": "rnd"})).unwrap();
        a.run("select.objects", json!({"index": 0})).unwrap();
        a.run("select.addObject", json!({"index": 1})).unwrap();

        let line = super::current_line(&a).expect("mixed selection has line info");
        assert!(!line.dash_mixed);
        assert!(!line.cap_mixed);
        assert_eq!(line.dash, wordcraft_doc::line_dash::LineDash::Dash);
        assert_eq!(line.cap, wordcraft_doc::connector::LineCap::Round);
    }

    /// Mixed fill selection (first shape has no fill, second has solid fill) lets clicking
    /// "No fill" clear the second shape's fill.
    #[test]
    fn format_shape_pane_no_fill_clears_mixed_selection() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.fill", json!({"color": null})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(a.session.doc.end_of(wordcraft_doc::StoryRef::Body));
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("shape.fill", json!({"color": "156082"})).unwrap();
        a.run("select.objects", json!({"index": 0})).unwrap();
        a.run("select.addObject", json!({"index": 1})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();

        let mut h = harness(a);
        h.run_steps(4);
        h.get_by_label("No fill").click();
        h.run_steps(3);
        let para = h.state().session.doc.body.first().and_then(|b| b.as_para()).unwrap();
        for (i, off) in para.object_offsets().into_iter().enumerate() {
            let InlineObject::Shape { fill, .. } = para.object_at(off).unwrap() else { panic!("expected shape {i}") };
            assert_eq!(*fill, None, "shape {i} fill cleared");
        }
    }

    /// Selecting More Colors… in the Line color picker opens the Colors dialog and applies
    /// the chosen color through `shape.outline`.
    #[test]
    fn format_shape_pane_line_more_colors_applies_outline() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.textBox", json!({"text": "Hello"})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);

        h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Color #000000").last().expect("line color field").click();
        h.run_steps(3);
        h.get_by_label("More Colors…").click();
        h.run_steps(3);

        let dialog = h.state_mut().dialog_host.get_mut::<craft::color_ui::ui::ColorPickerDialog>().expect("Colors dialog opened");
        dialog.set_color(craft::color_ui::ui::PickedColor::rgb(craft::theme::Srgba::rgb(1.0, 0.0, 0.0)));
        let ctx = h.ctx.clone();
        assert!(h.state_mut().dialog_host.confirm::<craft::color_ui::ui::ColorPickerDialog>(&ctx));
        crate::dialogs::host::settle(h.state_mut(), &ctx);
        assert_eq!(stroke(h.state()), Some(wordcraft_doc::Rgb(255, 0, 0)));
    }

    fn png_data() -> String {
        let img = image::RgbaImage::from_fn(20, 10, |x, y| {
            if x > 2 && x < 17 && y > 2 && y < 7 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let b = wordcraft_geom::picture::encode_format(&image::DynamicImage::ImageRgba8(img), image::ImageFormat::Png).unwrap();
        wordcraft_engine::cmd::insert::base64_encode(&b)
    }

    fn body_objects(app: &WordApp) -> Vec<&InlineObject> {
        let para = app.session.doc.body.first().and_then(|b| b.as_para()).unwrap();
        para.object_offsets().into_iter().filter_map(|off| para.object_at(off)).collect()
    }

    fn edit_layout_spin(h: &mut egui_kittest::Harness<'static, WordApp>, label: &str, text: &str) {
        let mut matches: Vec<_> = h.query_all_by_role_and_label(egui::accesskit::Role::SpinButton, label).collect();
        let node = matches.pop().expect("layout spin button");
        drop(matches);
        node.click();
        crate::test_harness::frames(h, 2);
        let mut matches: Vec<_> = h.query_all_by_role_and_label(egui::accesskit::Role::SpinButton, label).collect();
        matches.pop().expect("layout spin button").type_text(text);
        crate::test_harness::frames(h, 2);
        h.key_press(egui::Key::Enter);
        crate::test_harness::frames(h, 3);
    }

    /// Issue #5896: with a Picture and a Shape selected (distinct dimensions and rotation),
    /// Format Shape shows the shape's layout and Width/Height/Rotation only edit the shape.
    #[test]
    fn format_shape_pane_mixed_picture_and_shape_shows_and_updates_shape_only() {
        use egui_kittest::kittest::NodeT;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.picture", json!({"data": png_data()})).unwrap();
        a.run("picture.size", json!({"width": 60.0, "height": 40.0, "lockAspect": false})).unwrap();
        a.run("arrange.rotation", json!({"degrees": 15.0})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(a.session.doc.end_of(wordcraft_doc::StoryRef::Body));
        a.run("insert.shape", json!({"kind": "rectangle", "width": 150.0, "height": 90.0})).unwrap();
        a.run("select.objects", json!({"index": 0})).unwrap();
        a.run("select.addObject", json!({"index": 1})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();

        let shown_layout = super::current_layout(&a).expect("mixed selection has shape layout");
        assert_eq!((shown_layout.width, shown_layout.height, shown_layout.rotation), (150.0, 90.0, 0.0));

        let mut h = harness(a);
        h.run_steps(4);
        let width_val = h.query_all_by_role_and_label(egui::accesskit::Role::SpinButton, "Width").last().unwrap().accesskit_node().numeric_value();
        let height_val = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Height").accesskit_node().numeric_value();
        let rot_val = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Rotation").accesskit_node().numeric_value();
        assert_eq!(width_val, Some(150.0));
        assert_eq!(height_val, Some(90.0));
        assert_eq!(rot_val, Some(0.0));

        edit_layout_spin(&mut h, "Width", "210");
        edit_layout_spin(&mut h, "Height", "115");
        edit_layout_spin(&mut h, "Rotation", "35");

        let objs = body_objects(h.state());
        assert_eq!(objs.len(), 2);
        let (pw, ph, pf) = objs[0].frame().unwrap();
        assert_eq!((pw, ph, pf.spin().deg), (60.0, 40.0, 15.0), "picture dimensions and rotation stay untouched");
        let (sw, sh, sf) = objs[1].frame().unwrap();
        assert_eq!((sw, sh, sf.spin().deg), (210.0, 115.0, 35.0), "shape updated by Format Shape layout controls");
        assert_eq!(wordcraft_engine::cmd::objects::picked_objects(&h.state().session).len(), 2, "multi-selection preserved");
    }

    /// Issue #5896: with an editable Chart and a Shape selected (distinct dimensions),
    /// Format Shape shows the shape's layout and Width/Height only edit the shape.
    #[test]
    fn format_shape_pane_mixed_chart_and_shape_shows_and_updates_shape_only() {
        use egui_kittest::kittest::NodeT;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.chart", json!({"type": "column"})).unwrap();
        let (chart_w0, chart_h0, _) = body_objects(&a)[0].frame().unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(a.session.doc.end_of(wordcraft_doc::StoryRef::Body));
        a.run("insert.shape", json!({"kind": "rectangle", "width": 140.0, "height": 70.0})).unwrap();
        assert_ne!((chart_w0, chart_h0), (140.0, 70.0), "distinct initial dimensions");
        a.run("select.objects", json!({"index": 0})).unwrap();
        a.run("select.addObject", json!({"index": 1})).unwrap();
        a.run("view.formatShapePane", json!({"value": true})).unwrap();

        let mut h = harness(a);
        h.run_steps(4);
        let width_val = h.query_all_by_role_and_label(egui::accesskit::Role::SpinButton, "Width").last().unwrap().accesskit_node().numeric_value();
        let height_val = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Height").accesskit_node().numeric_value();
        assert_eq!(width_val, Some(140.0));
        assert_eq!(height_val, Some(70.0));

        edit_layout_spin(&mut h, "Width", "220");
        edit_layout_spin(&mut h, "Height", "105");

        let objs = body_objects(h.state());
        assert_eq!(objs.len(), 2);
        let (cw, ch, _) = objs[0].frame().unwrap();
        assert_eq!((cw, ch), (chart_w0, chart_h0), "chart dimensions stay untouched");
        let (sw, sh, _) = objs[1].frame().unwrap();
        assert_eq!((sw, sh), (220.0, 105.0), "shape updated by Format Shape layout controls");
    }
}
