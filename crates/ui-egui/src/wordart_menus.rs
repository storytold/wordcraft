//! WordArt menus: Text Fill, Text Outline and Text Effects (Shadow, Glow, Reflection,
//! Transform). Previews are drawn in code from our own styles and the warp geometry itself;
//! the style gallery is `ribbon::galleries::wordart`.

use craft::behavior::keyed;
use craft::draw::ui::GradientSwatch;
use craft::menu::CommandRef;
use craft::ui::contract::{ActivateOn, BodyCx, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::layout_components::Row;
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::ContentPainterCx;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::{Align2, Color32, Pos2, Rect, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::Rgb;
use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind};
use wordcraft_doc::wordart::{DRAWN_WARPS, GradStop, TextFill, TextWarp, art_style};

use crate::WordApp;
use crate::colors::Target;

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// A letter "A" drawn in WordArt style `id` (its fill, outline, shadow and glow, roughly), in the content painter's ink.
pub(crate) fn paint_style(cx: &ContentPainterCx<'_>, r: Rect, id: &str, theme: &[Rgb]) {
    let p = &cx.painter;
    let Some(fx) = art_style(id, theme) else { return };
    let ink = cx.ink();
    let font = egui::FontId::proportional(r.height() * 0.8);
    let at = r.center();
    let text = |dx: f32, dy: f32, c: Color32| {
        p.text(at + vec2(dx, dy), Align2::CENTER_CENTER, "A", font.clone(), c);
    };
    if let Some(g) = fx.glow {
        let c = c32(g.color).gamma_multiply(0.35);
        for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0), (-1.5, -1.5), (1.5, 1.5), (-1.5, 1.5), (1.5, -1.5)] {
            text(dx, dy, c);
        }
    }
    if fx.shadow.is_some() {
        text(1.5, 1.5, Color32::from_black_alpha(110));
    }
    if fx.reflection.is_some() {
        p.rect_filled(
            Rect::from_min_max(pos2(r.center().x - r.width() * 0.25, r.max.y - 3.0), pos2(r.center().x + r.width() * 0.25, r.max.y)),
            1.0,
            ink.gamma_multiply(0.5),
        );
    }
    if let Some(o) = fx.outline.and_then(|o| o.color) {
        for (dx, dy) in [(-0.8, 0.0), (0.8, 0.0), (0.0, -0.8), (0.0, 0.8)] {
            text(dx, dy, c32(o));
        }
    }
    match &fx.fill {
        Some(TextFill::None) => text(0.0, 0.0, ink.gamma_multiply(0.2)),
        Some(f) => {
            let c = f.main_color().map_or(ink, c32);
            text(0.0, 0.0, c);
            if let TextFill::Gradient { stops, .. } = f
                && let Some(first) = stops.first()
            {
                // The gradient's first colour over the top half.
                let top = Rect::from_min_max(r.min, pos2(r.max.x, r.center().y));
                p.with_clip_rect(top).text(at, Align2::CENTER_CENTER, "A", font.clone(), c32(first.color));
            }
        }
        None => text(0.0, 0.0, ink),
    }
}

/// Text Fill: automatic (the text's colour), a colour, no fill (hollow), or a gradient of
/// theme colours. `cx` is the body of the menu the rows sit in, closed by a gradient pick.
pub fn fill_menu(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp, theme: &[Rgb]) {
    MenuItem::command(CommandRef::with_params("format.textEffects", json!({"fill": null}))).label(crate::tl!("Automatic").to_string()).show(ui);
    MenuItem::command(CommandRef::with_params("format.textEffects", json!({"fill": "none"}))).label(crate::tl!("No Fill").to_string()).show(ui);
    let target = Target::at("format.textEffects", json!({"fill": null}), "/fill");
    if crate::colors::palette(ui, app, target).picked.is_some() {
        cx.request_close();
    }
    let slot = |i: usize| theme.get(i).copied().unwrap_or(Rgb::BLACK);
    let mut picked = None;
    Row::new().show(ui, |ui, _| {
        for (a, b) in [(slot(4), slot(7)), (slot(5), slot(8)), (slot(1), slot(4)), (slot(2), slot(9))] {
            let stop = |pos: f32, color: Rgb| GradStop { pos, color, transparency: 0.0 };
            let fill = Fill::Gradient(std::sync::Arc::new(Gradient {
                stops: vec![stop(0.0, a), stop(1.0, b)],
                kind: GradientKind::Linear { angle: 90.0, scaled: false },
                tile: None,
                rot_with_shape: false,
                flip: GradientFlip::None,
            }));
            // PLAN: assumed crate::format_shape::fill_tile(&Fill) -> (GradientStops, GradientTile) (chrome-w14)
            let (stops, tile) = crate::format_shape::fill_tile(&fill);
            if GradientSwatch::new(crate::tl!("Text Fill").to_string(), &stops).tile(tile).show(ui).clicked {
                picked = Some((a, b));
            }
        }
    });
    if let Some((a, b)) = picked {
        let _ = app.run("format.textEffects", json!({"fill": {"gradient": [a.hex(), b.hex()], "angle": 90}}));
        cx.request_close();
    }
}

/// Text Outline: automatic (none set), no outline, a colour, a weight.
pub fn outline_menu(ui: &mut Ui, app: &mut WordApp, current: Option<(Rgb, f32)>) {
    MenuItem::command(CommandRef::with_params("format.textEffects", json!({"outline": null}))).label(crate::tl!("Automatic").to_string()).show(ui);
    MenuItem::command(CommandRef::with_params("format.textEffects", json!({"outline": "none"}))).label(crate::tl!("No Outline").to_string()).show(ui);
    let width = current.map_or(0.75, |c| c.1);
    let target = Target::at("format.textEffects", json!({"outline": {"color": null, "width": width}}), "/outline/color");
    let _ = crate::colors::palette(ui, app, target);
    Divider::horizontal().show(ui);
    Menu::submenu_row(ui, crate::tl!("Weight").to_string(), |ui, _| {
        let color = current.map_or(Rgb::BLACK, |c| c.0).hex();
        crate::ribbon::weight_menu(ui, app, "format.textEffects", current.map(|c| c.1), |w| json!({"outline": {"color": color, "width": w}}));
    });
}

/// A warp's preview: its top, middle and bottom lines and a few verticals, bent as the text is.
fn paint_warp(p: &egui::Painter, r: Rect, preset: &str, c: Color32) {
    let Some(tw) = TextWarp::new(preset) else { return };
    let frame = wordcraft_geom::Rect::new(r.min.x, r.min.y, r.width(), r.height());
    let Some(w) = wordcraft_layout::wordart::Warp::new(&tw, frame, 0.3) else { return };
    let pt = |u: f32, v: f32| {
        let (x, y) = w.map(u, v);
        Pos2::new(x, y)
    };
    for v in [0.0, 1.0] {
        p.add(egui::Shape::line((0..=24).map(|i| pt(i as f32 / 24.0, v)).collect(), Stroke::new(1.0, c)));
    }
    p.add(egui::Shape::line((0..=24).map(|i| pt(i as f32 / 24.0, 0.5)).collect(), Stroke::new(1.0, c.gamma_multiply(0.5))));
    for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
        p.line_segment([pt(u, 0.0), pt(u, 1.0)], Stroke::new(1.0, c.gamma_multiply(0.6)));
    }
}

/// Text Effects: Shadow, Glow and Reflection (and Transform, for a shape's text).
pub fn effects_menu(ui: &mut Ui, app: &mut WordApp, transform: bool) {
    let effect = |ui: &mut Ui, label: String, params: serde_json::Value| {
        MenuItem::command(CommandRef::with_params("format.textEffects", params)).label(label).show(ui);
    };
    Menu::submenu_row(ui, crate::tl!("Shadow").to_string(), |ui, _| {
        effect(ui, crate::tl!("No Shadow").to_string(), json!({"shadow": null}));
        Divider::horizontal().show(ui);
        for (id, label) in wordcraft_doc::effects::SHADOW_PRESETS {
            effect(ui, crate::tl!(label).to_string(), json!({"shadow": id}));
        }
    });
    Menu::submenu_row(ui, crate::tl!("Glow").to_string(), |ui, _| {
        effect(ui, crate::tl!("No Glow").to_string(), json!({"glow": null}));
        Divider::horizontal().show(ui);
        for size in [3.0, 5.0, 8.0, 11.0] {
            effect(ui, format!("{size} pt"), json!({"glow": size}));
        }
        Divider::horizontal().show(ui);
        Menu::submenu_row(ui, crate::tl!("Glow Colors").to_string(), |ui, _| {
            let target = Target::at("format.textEffects", json!({"glow": {"color": null, "size": 5.0}}), "/glow/color");
            let _ = crate::colors::palette(ui, app, target);
        });
    });
    Menu::submenu_row(ui, crate::tl!("Reflection").to_string(), |ui, _| {
        effect(ui, crate::tl!("No Reflection").to_string(), json!({"reflection": null}));
        Divider::horizontal().show(ui);
        for size in [25.0, 50.0, 90.0] {
            effect(ui, format!("{size:.0}%"), json!({"reflection": {"size": size}}));
        }
    });
    if transform {
        Menu::submenu_row(ui, crate::tl!("Transform").to_string(), |ui, cx| transform_menu(ui, cx, app));
    }
}

/// Text Effects › Transform: the warps WordCraft bends text along, as previews.
pub fn transform_menu(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut WordApp) {
    MenuItem::command(CommandRef::with_params("wordArt.transform", json!({"preset": "none"}))).label(crate::tl!("No Transform").to_string()).show(ui);
    let source = keyed(&DRAWN_WARPS, |(id, _)| *id);
    let out = TileGrid::new(crate::tl!("Transform").to_string())
        .caption(CaptionPlace::None)
        .select(SelectMode::None)
        .activate(ActivateOn::Click)
        .card_columns(4)
        .thumb_size(ThumbSize::Small)
        // Every warp shows; the menu's body is the one scroll.
        .natural_height(true)
        .show_items(ui, &source, &mut |id, _| {
            let id: &'static str = id;
            let label = DRAWN_WARPS.iter().find(|(warp, _)| *warp == id).map_or(id, |(_, label)| *label);
            Tile::new(crate::tl!(label).to_string())
                .thumb(ThumbAspect::new(46, 32), move |cx, r| paint_warp(&cx.painter, r.shrink(2.0), id, cx.ink()))
        });
    for event in &out.response.events {
        if let CollectionEvent::Activate(id) = event {
            // The command is fired by the app, not bound to the tile: a tile has no command of its own.
            let _ = app.run("wordArt.transform", json!({"preset": id}));
            cx.request_close();
        }
    }
}

/// The text outline (colour, width) Text Outline would change, for its Weight menu: read where
/// `format.textEffects` applies, so choosing a weight keeps that text's colour. A selected shape,
/// or a caret in a text box, formats the box's whole text, read from its start; otherwise the
/// selection, read as typing reads it (the first selected character, or the caret's).
pub fn current_outline(app: &WordApp) -> Option<(Rgb, f32)> {
    let s = &app.session;
    let props = match wordcraft_engine::cmd::wordart::box_story(s) {
        Some(id) => s.doc.para_at(&s.doc.start_of(wordcraft_doc::StoryRef::Part(id)))?.props_at(0).clone(),
        None => s.typing_props(),
    };
    let fx = props.text_effects.as_deref()?;
    fx.outline.and_then(|o| o.color.map(|c| (c, o.width)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_engine::Session;

    fn outline_at(app: &WordApp, para: usize) -> Option<(Rgb, f32)> {
        let p = app.session.doc.para_at(&wordcraft_doc::Pos::body(para, 0))?;
        let fx = p.props_of_char(0).text_effects.as_deref()?;
        fx.outline.and_then(|o| o.color.map(|c| (c, o.width)))
    }

    /// #3917 review: the Weight menu reads the outline where Text Outline applies, the selection,
    /// not the first character of the story. Its highlight is the selected text's weight, and a
    /// new weight keeps that text's colour and leaves other paragraphs alone.
    #[test]
    fn the_weight_menu_reads_and_keeps_the_selected_texts_outline() {
        use egui::accesskit::Toggled;
        use egui_kittest::kittest::{NodeT, Queryable};
        let rgb = |h: &str| Rgb::parse(h);
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Red line\nBlue line")), crate::Services::default());
        a.run("select.text", json!({"text": "Red line"})).unwrap();
        a.run("format.textEffects", json!({"outline": {"color": "C00000", "width": 1.0}})).unwrap();
        a.run("select.text", json!({"text": "Blue line"})).unwrap();
        a.run("format.textEffects", json!({"outline": {"color": "1F4E79", "width": 3.0}})).unwrap();
        assert_eq!(current_outline(&a), rgb("1F4E79").map(|c| (c, 3.0)), "the selection's outline, not the first paragraph's");
        let mut h = crate::test_harness::builder().with_size(egui::vec2(500.0, 700.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let current = current_outline(app);
                Menu::inline("Text Outline").show(ui, |ui, _| outline_menu(ui, app, current));
                // Kit command rows fire through the command board; the app runs what a pass fired.
                let fired = craft::ui::runtime::frame::take_output(ui.ctx());
                app.run_fired(ui.ctx(), &fired);
            },
            a,
        );
        h.run_steps(2);
        h.get_by_label("Weight").hover();
        h.run();
        // Hovering opened the submenu, whose panel is named for its row too: click the row, the
        // first of the two in tree order.
        h.query_all_by_label("Weight").next().expect("the Weight row").click();
        h.run();
        assert_eq!(h.get_by_label("3 pt").accesskit_node().toggled(), Some(Toggled::True), "the selected text's weight");
        assert_eq!(h.get_by_label("1 pt").accesskit_node().toggled(), Some(Toggled::False));
        h.get_by_label("6 pt").click();
        h.run_steps(2);
        assert_eq!(outline_at(h.state(), 1), rgb("1F4E79").map(|c| (c, 6.0)), "the new weight keeps the selected text's colour");
        assert_eq!(outline_at(h.state(), 0), rgb("C00000").map(|c| (c, 1.0)), "the other paragraph is untouched");
    }

    /// A caret in a text box formats the box's whole text, so its outline is read from the box's
    /// start; back in the body, the body text's (none here).
    #[test]
    fn a_caret_in_a_text_box_reads_the_boxs_outline() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Body")), crate::Services::default());
        a.run("insert.textBox", json!({"text": "Boxed words"})).unwrap();
        a.run("format.textEffects", json!({"outline": {"color": "00B050", "width": 2.25}})).unwrap();
        assert_eq!(current_outline(&a), Rgb::parse("00B050").map(|c| (c, 2.25)));
        a.run("caret.set", json!({"pos": wordcraft_doc::Pos::body(0, 2)})).unwrap();
        assert_eq!(current_outline(&a), None, "the body text has no outline");
    }

    /// #7449: the Transform menu renders compact preview tiles (~46×32 pt)
    /// instead of expanding each warp thumbnail to default card dimensions (~132 pt).
    #[test]
    fn transform_menu_renders_compact_tiles() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;

        let a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Transform")), crate::Services::default());
        let mut h = crate::test_harness::builder().with_size(egui::vec2(600.0, 700.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                Menu::inline("Transform").show(ui, |ui, cx| transform_menu(ui, cx, app));
            },
            a,
        );
        h.run_steps(2);

        let tile = h.get_by_role_and_label(Role::ListItem, "Arch Up").rect();
        assert!(tile.width() <= 60.0 && tile.height() <= 45.0, "transform tile is compact (~46×32 pt), got {:?}", tile.size());
        let grid = h.get_by_role_and_label(Role::ListBox, "Transform").rect();
        assert!(grid.width() <= 260.0, "4-column transform menu grid width stays compact (~200 pt), got {:?}", grid.size());
    }
}
