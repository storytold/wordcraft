//! WordArt menus: the style gallery (Insert › WordArt, Home › Text Effects, Shape Format ›
//! WordArt Styles), Text Fill, Text Outline and Text Effects (Shadow, Glow, Reflection,
//! Transform). Previews are drawn in code from our own styles and the warp geometry itself.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::Rgb;
use wordcraft_doc::wordart::{ART_STYLES, DRAWN_WARPS, TextFill, TextWarp, art_style};

use crate::WordApp;
use crate::ribbon::mi;
use crate::theme::{Tokens, semibold};
use crate::widgets::color_grid;

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// A clickable preview cell `w` × `h`; true when clicked.
fn cell(ui: &mut Ui, w: f32, h: f32, tip: &str, paint: impl FnOnce(&egui::Painter, Rect)) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let bg = if resp.hovered() { t.hover } else { t.input };
    ui.painter().rect_filled(r, 3.0, bg);
    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    paint(ui.painter(), r.shrink(3.0));
    resp.on_hover_text(crate::tl!(tip)).clicked()
}

/// A letter "A" drawn in WordArt style `id` (its fill, outline, shadow and glow, roughly).
fn paint_style(p: &egui::Painter, r: Rect, id: &str, theme: &[Rgb]) {
    let Some(fx) = art_style(id, theme) else { return };
    let font = semibold(r.height() * 0.8);
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
            Tokens::get(p.ctx()).text_dim,
        );
    }
    if let Some(o) = fx.outline.and_then(|o| o.color) {
        for (dx, dy) in [(-0.8, 0.0), (0.8, 0.0), (0.0, -0.8), (0.0, 0.8)] {
            text(dx, dy, c32(o));
        }
    }
    match &fx.fill {
        Some(TextFill::None) => text(0.0, 0.0, Tokens::get(p.ctx()).input),
        Some(f) => {
            let c = f.main_color().map_or(Color32::BLACK, c32);
            text(0.0, 0.0, c);
            if let TextFill::Gradient { stops, .. } = f
                && let Some(first) = stops.first()
            {
                // The gradient's first colour over the top half.
                let top = Rect::from_min_max(r.min, pos2(r.max.x, r.center().y));
                p.with_clip_rect(top).text(at, Align2::CENTER_CENTER, "A", font.clone(), c32(first.color));
            }
        }
        None => text(0.0, 0.0, Color32::BLACK),
    }
}

/// Our WordArt styles as a grid; a click runs `cmd` with `{"style": id}`.
pub fn style_gallery(ui: &mut Ui, app: &mut WordApp, cmd: &str) {
    let theme = app.session.doc.settings.theme_colors.clone();
    egui::Grid::new(("wordart-styles", cmd)).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        for (i, (id, label)) in ART_STYLES.iter().enumerate() {
            if cell(ui, 40.0, 36.0, label, |p, r| paint_style(p, r, id, &theme)) {
                let _ = app.run(cmd, json!({"style": id}));
                ui.close();
            }
            if i % 4 == 3 {
                ui.end_row();
            }
        }
    });
}

/// Text Fill: automatic (the text's colour), a colour, no fill (hollow), or a gradient of
/// theme colours.
pub fn fill_menu(ui: &mut Ui, app: &mut WordApp, theme: &[Rgb]) {
    mi(ui, app, "Automatic", "format.textEffects", json!({"fill": null}));
    mi(ui, app, "No Fill", "format.textEffects", json!({"fill": "none"}));
    if let Some(hex) = color_grid(ui, theme) {
        let _ = app.run("format.textEffects", json!({"fill": hex}));
        ui.close();
    }
    ui.add_space(4.0);
    let slot = |i: usize| theme.get(i).copied().unwrap_or(Rgb::BLACK);
    ui.horizontal(|ui| {
        for (a, b) in [(slot(4), slot(7)), (slot(5), slot(8)), (slot(1), slot(4)), (slot(2), slot(9))] {
            let clicked = cell(ui, 40.0, 18.0, "Text Fill", |p, r| {
                let n = 12;
                for k in 0..n {
                    let f = k as f32 / (n - 1) as f32;
                    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) as u8;
                    let c = Color32::from_rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2));
                    let y0 = r.min.y + r.height() * k as f32 / n as f32;
                    p.rect_filled(Rect::from_min_max(pos2(r.min.x, y0), pos2(r.max.x, y0 + r.height() / n as f32 + 0.5)), 0.0, c);
                }
            });
            if clicked {
                let _ = app.run("format.textEffects", json!({"fill": {"gradient": [a.hex(), b.hex()], "angle": 90}}));
                ui.close();
            }
        }
    });
}

/// Text Outline: automatic (none set), no outline, a colour, a weight.
pub fn outline_menu(ui: &mut Ui, app: &mut WordApp, theme: &[Rgb], current: Option<(Rgb, f32)>) {
    mi(ui, app, "Automatic", "format.textEffects", json!({"outline": null}));
    mi(ui, app, "No Outline", "format.textEffects", json!({"outline": "none"}));
    let width = current.map_or(0.75, |c| c.1);
    if let Some(hex) = color_grid(ui, theme) {
        let _ = app.run("format.textEffects", json!({"outline": {"color": hex, "width": width}}));
        ui.close();
    }
    ui.separator();
    ui.menu_button(crate::tl!("Weight"), |ui| {
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
pub fn effects_menu(ui: &mut Ui, app: &mut WordApp, theme: &[Rgb], transform: bool) {
    ui.menu_button(crate::tl!("Shadow"), |ui| {
        mi(ui, app, "No Shadow", "format.textEffects", json!({"shadow": null}));
        ui.separator();
        for (id, label) in wordcraft_doc::effects::SHADOW_PRESETS {
            mi(ui, app, label, "format.textEffects", json!({"shadow": id}));
        }
    });
    ui.menu_button(crate::tl!("Glow"), |ui| {
        mi(ui, app, "No Glow", "format.textEffects", json!({"glow": null}));
        ui.separator();
        for size in [3.0, 5.0, 8.0, 11.0] {
            mi(ui, app, &format!("{size} pt"), "format.textEffects", json!({"glow": size}));
        }
        ui.separator();
        ui.menu_button(crate::tl!("Glow Colors"), |ui| {
            if let Some(hex) = color_grid(ui, theme) {
                let _ = app.run("format.textEffects", json!({"glow": {"color": hex, "size": 5.0}}));
                ui.close();
            }
        });
    });
    ui.menu_button(crate::tl!("Reflection"), |ui| {
        mi(ui, app, "No Reflection", "format.textEffects", json!({"reflection": null}));
        ui.separator();
        for size in [25.0, 50.0, 90.0] {
            mi(ui, app, &format!("{size:.0}%"), "format.textEffects", json!({"reflection": {"size": size}}));
        }
    });
    if transform {
        ui.menu_button(crate::tl!("Transform"), |ui| transform_menu(ui, app));
    }
}

/// Text Effects › Transform: the warps WordCraft bends text along, as previews.
pub fn transform_menu(ui: &mut Ui, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    mi(ui, app, "No Transform", "wordArt.transform", json!({"preset": "none"}));
    ui.add_space(4.0);
    egui::Grid::new("wordart-warps").spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        for (i, (id, label)) in DRAWN_WARPS.iter().enumerate() {
            if cell(ui, 46.0, 32.0, label, |p, r| paint_warp(p, r.shrink(2.0), id, t.accent)) {
                let _ = app.run("wordArt.transform", json!({"preset": id}));
                ui.close();
            }
            if i % 4 == 3 {
                ui.end_row();
            }
        }
    });
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
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 700.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let theme = app.session.doc.settings.theme_colors.clone();
                let current = current_outline(app);
                outline_menu(ui, app, &theme, current);
            },
            a,
        );
        h.run_steps(2);
        h.get_by_label("Weight").hover();
        h.run();
        h.get_by_label("Weight").click();
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
}
