//! The Format Shape pane (Shape Fill › More Gradients…, a shape's Format Shape…): the selected
//! shapes' fill, as Word's Fill section: no fill, a solid colour with its transparency, or a
//! gradient (preset, type, direction, angle, stops, rotate with shape). Every change runs
//! `shape.fill` with the whole fill; a drag applies as it goes and undoes in one step.

use std::sync::Arc;

use craft::behavior::{EditEvents, Value};
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
use craft::ui::text::NumberField;
use egui::{Ui, pos2};
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

/// The first selected shape's or canvas's fill: `None` with no shape selected.
pub(crate) fn current(app: &WordApp) -> Option<Option<Fill>> {
    let (_, o) = wordcraft_engine::cmd::objects::selected(&app.session)?;
    match o {
        InlineObject::Shape { fill, .. } => Some(fill),
        o => o.canvas_style().map(|c| c.fill.clone()),
    }
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
    let Some(fill) = current(app) else {
        Notice::new(Tone::Neutral, tl!("Select a shape, text box or drawing canvas to format its fill.")).show(ui);
        return;
    };
    let theme = app.session.doc.settings.theme_colors.clone();
    let kind: u8 = match &fill {
        None => 0,
        Some(Fill::Solid { .. }) => 1,
        Some(Fill::Gradient(_)) => 2,
    };
    let shown = RadioGroup::new(tl!("Fill"), Bind::given(kind)).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Radio::new(0u8).label(tl!("No fill")).show(ui);
            Radio::new(1u8).label(tl!("Solid fill")).show(ui);
            Radio::new(2u8).label(tl!("Gradient fill")).show(ui);
        });
    });
    if let Some(pick) = shown.output.events.committed().copied()
        && pick != kind
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
        && n != color
    {
        apply(app, Some(&Fill::Solid { color: n, transparency }));
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
}
