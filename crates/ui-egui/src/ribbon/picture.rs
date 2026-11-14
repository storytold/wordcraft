use std::collections::HashMap;
use std::sync::Arc;

use super::arrange::{align_menu, group_menu, rotate_menu, rotation_field, wrap_menu};
use super::common::{mi, size_field, stack};
use super::shape::alt_text_group;
use super::{Target, Ui, WordApp, big, group, json, menu_button, small};
use serde_json::Value;
use wordcraft_engine::cmd::objects::look;

/// Draws the Picture Format tab's Adjust, Picture Styles, Accessibility (Alt Text), Arrange and
/// Size groups.
pub(super) fn picture_format(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Adjust", None, app, |ui, app| {
        // Background Removal mode, with its own tab (#3344).
        big(ui, app, "removeBackground", "Remove\nBackground", "ui.removeBackground", json!({}), false);
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new(
                "corrections",
                Some("Corrections"),
                "Sharpen, soften, brightness and contrast",
                true,
                "picture.corrections",
            ),
            corrections_menu,
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("pictureColor", Some("Color"), "Saturation, tone and recolor", true, "picture.color"),
            color_menu,
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new(
                "artisticEffects",
                Some("Artistic\nEffects"),
                "Make the picture look like a sketch or a painting",
                true,
                "picture.effects",
            ),
            |ui, app| {
                let items: Vec<Item> =
                    std::iter::once(("None", "none")).chain(EFFECTS).map(|(l, e)| Item::new(l, "picture.effects", json!({"effect": e}))).collect();
                gallery(ui, app, "picture_effects", &items, 5);
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("transparency", Some("Transparency"), "Picture transparency", true, "picture.transparency"),
            |ui, app| {
                let items: Vec<Item> = [0.0, 15.0, 30.0, 50.0, 65.0, 80.0, 95.0]
                    .iter()
                    .map(|v| {
                        Item::text(
                            crate::i18n::fmt(tl!("Transparency: {percent}%"), &[("percent", &format!("{v:.0}"))]),
                            "picture.transparency",
                            json!({"percent": v}),
                        )
                    })
                    .collect();
                gallery(ui, app, "picture_transparency", &items, 7);
                ui.separator();
                // An exact value, dragged with a live preview: absolute from the original
                // pixels, one undo step for the whole drag (same pattern as Size › W/H).
                let now = look::current(&app.session).map_or(0.0, |l| f64::from(l.transparency));
                slider(ui, app, "Transparency", now, 0.0..=100.0, "%", "picture.transparency", "percent");
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new(
                "icons",
                Some("Icon Color"),
                "Recolor a one-color picture, such as an icon",
                true,
                "picture.iconColor",
            ),
            |ui, app| {
                let theme = app.session.doc.settings.theme_colors.clone();
                crate::colors::menu(ui, app, &theme, Target::cmd("picture.iconColor"));
            },
        );
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "compressPictures",
                    Some("Compress Pictures"),
                    "Make the picture smaller for printing, the web or e-mail",
                    false,
                    "picture.compress",
                ),
                compress_menu,
            );
            small(
                ui,
                app,
                "picture",
                Some("Change Picture"),
                "Change Picture",
                crate::widgets::ButtonAction::new("ui.changePicture", json!({}), false),
            );
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "resetGraphic",
                    Some("Reset Picture"),
                    "Discard the formatting changes made to this picture",
                    false,
                    "picture.reset",
                ),
                |ui, app| {
                    mi(ui, app, "Reset Picture", "picture.reset", json!({}));
                    mi(ui, app, "Reset Picture & Size", "picture.reset", json!({"size": true}));
                },
            );
        });
    });
    group(ui, "Picture Styles", None, app, |ui, app| {
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("picture", Some("Styles"), "Frames and effects", true, "picture.style"),
            |ui, app| {
                let items: Vec<Item> = [
                    ("No Style", "none"),
                    ("Simple Frame", "simpleFrame"),
                    ("Thick Frame", "thickFrame"),
                    ("Rounded", "rounded"),
                    ("Soft Edge", "softEdge"),
                    ("Shadow", "shadow"),
                ]
                .into_iter()
                .map(|(l, st)| Item::new(l, "picture.style", json!({"style": st})))
                .collect();
                gallery(ui, app, "picture_styles", &items, 6);
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("borders", Some("Border"), "Picture border: its color and weight", true, "picture.border"),
            |ui, app| {
                let theme = app.session.doc.settings.theme_colors.clone();
                crate::colors::menu(ui, app, &theme, Target::cmd("picture.border"));
                ui.separator();
                mi(ui, app, "No Outline", "picture.border", json!({"color": null}));
                ui.menu_button(tl!("Weight"), |ui| {
                    for (l, w) in [("Thin", 2), ("Medium", 4), ("Thick", 8), ("Extra Thick", 16)] {
                        mi(ui, app, l, "picture.border", json!({"width": w}));
                    }
                });
            },
        );
    });
    alt_text_group(ui, app);
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        wrap_menu(ui, app);
        stack(ui, |ui| {
            rotate_menu(ui, app, Some("Rotate"));
            align_menu(ui, app, Some("Align"));
            group_menu(ui, app, Some("Group"));
        });
    });
    // The launcher opens the Size dialog: scale, Lock aspect ratio, the original size (#2573).
    group(ui, "Size", Some("picture.size"), app, |ui, app| {
        let (media, off, w0, h0, crop0, alt0) = match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((pos, wordcraft_doc::para::InlineObject::Image { media, w, h, alt, crop, .. })) => (media, pos.off, w, h, crop, alt),
            _ => (String::new(), 0, 0.0, 0.0, [0.0; 4], String::new()),
        };
        // `picture.size` locks the aspect ratio unless told otherwise, so one dimension suffices.
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                for (label, key, pt) in [("W:", "width", w0), ("H:", "height", h0)] {
                    let hint = tl!("Type a size in any unit, such as 5 cm or 300 px. The other side follows, so the picture keeps its proportions.");
                    if let Some(v) = size_field(ui, app, label, pt, egui::Id::unique(("picture_size_field", key)), hint) {
                        let _ = app.run("picture.size", json!({key: v}));
                    }
                }
                rotation_field(ui, app);
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                let cropping = crate::crop::active(app).is_some();
                small(ui, app, "crop", Some("Crop"), "Crop", crate::widgets::ButtonAction::new("ui.crop", json!({}), cropping));
                for (i, side) in ["L", "T", "R", "B"].iter().enumerate() {
                    ui.label(egui::RichText::new(tl!(side)).small());
                    let mut v = crop0[i] * 100.0;
                    let r = ui.add(egui::DragValue::new(&mut v).speed(0.5).range(0.0..=95.0).suffix("%"));
                    if r.changed() {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let mut c = crop0;
                        c[i] = (v / 100.0).clamp(0.0, 0.95);
                        // The two sides of an axis keep 5% of the source between them.
                        let other = c[if i < 2 { i + 2 } else { i - 2 }];
                        c[i] = c[i].min((0.95 - other).max(0.0));
                        let _ = app.run("picture.crop", json!({"left": c[0], "top": c[1], "right": c[2], "bottom": c[3]}));
                    }
                }
                if ui.button(egui::RichText::new(tl!("Reset")).small()).on_hover_text(tl!("Reset Crop")).clicked() {
                    let _ = app.run("picture.crop", json!({"left": 0.0, "top": 0.0, "right": 0.0, "bottom": 0.0}));
                }
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                ui.label(egui::RichText::new(tl!("Alt:")).small());
                // Frame-local buffers lose keystrokes; keep the draft in egui temp memory keyed
                // by picture (same pattern as the comment editor in panes.rs).
                let key = egui::Id::unique(("picture_alt", media.clone(), off));
                let mut alt = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| alt0.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut alt).desired_width(120.0).hint_text(tl!("Alt text")));
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(key, alt.clone()));
                }
                if r.lost_focus() {
                    ui.data_mut(|d| d.remove::<String>(key));
                    // The click that took focus may have selected another picture; never commit
                    // one picture's draft onto another.
                    if alt != alt0 && app.selected_picture_media().as_deref() == Some(media.as_str()) {
                        let _ = app.run("picture.altText", json!({"text": alt}));
                    }
                }
            });
        });
    });
}

/// The Background Removal tab, while its mode is on (#3344): Mark Areas to Keep and Mark Areas to
/// Remove (pens), Discard All Changes and Keep Changes.
pub(super) fn background_removal(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Refine", None, app, |ui, app| {
        let pen = crate::background::pen(app);
        crate::widgets::big_toggle(ui, app, "markKeep", "Mark Areas\nto Keep", "ui.markAreasToKeep", pen == Some(true));
        crate::widgets::big_toggle(ui, app, "markRemove", "Mark Areas\nto Remove", "ui.markAreasToRemove", pen == Some(false));
    });
    group(ui, "Close", None, app, |ui, app| {
        big(ui, app, "close", "Discard All\nChanges", "ui.discardBackgroundChanges", json!({}), false);
        big(ui, app, "check", "Keep\nChanges", "ui.keepBackgroundChanges", json!({}), false);
    });
}

/// Artistic Effects, as Word names them, after None.
const EFFECTS: [(&str, &str); 22] = [
    ("Marker", "marker"),
    ("Pencil Grayscale", "pencilGrayscale"),
    ("Pencil Sketch", "pencilSketch"),
    ("Line Drawing", "lineDrawing"),
    ("Chalk Sketch", "chalkSketch"),
    ("Paint Strokes", "paintStrokes"),
    ("Paint Brush", "paintBrush"),
    ("Glow Diffused", "glowDiffused"),
    ("Blur", "blur"),
    ("Light Screen", "lightScreen"),
    ("Watercolor Sponge", "watercolorSponge"),
    ("Film Grain", "filmGrain"),
    ("Mosaic Bubbles", "mosaicBubbles"),
    ("Glass", "glass"),
    ("Cement", "cement"),
    ("Texturizer", "texturizer"),
    ("Crisscross Etching", "crisscrossEtching"),
    ("Pastels Smooth", "pastelsSmooth"),
    ("Plastic Wrap", "plasticWrap"),
    ("Cutout", "cutout"),
    ("Photocopy", "photocopy"),
    ("Glow Edges", "glowEdges"),
];

/// Corrections: Sharpen/Soften and Brightness/Contrast as previews, then exact values.
fn corrections_menu(ui: &mut Ui, app: &mut WordApp) {
    heading(ui, "Sharpen/Soften");
    let sharpen: Vec<Item> = [("Soften: 50%", -50.0), ("Soften: 25%", -25.0), ("Sharpen: 0%", 0.0), ("Sharpen: 25%", 25.0), ("Sharpen: 50%", 50.0)]
        .into_iter()
        .map(|(l, v)| Item::new(l, "picture.corrections", json!({"sharpen": v})))
        .collect();
    gallery(ui, app, "picture_sharpen", &sharpen, 5);
    heading(ui, "Brightness/Contrast");
    let steps = [-40.0f64, -20.0, 0.0, 20.0, 40.0];
    let mut bc = Vec::new();
    for c in steps {
        for b in steps {
            let label = crate::i18n::fmt(tl!("Brightness: {b}% Contrast: {c}%"), &[("b", &format!("{b:+.0}")), ("c", &format!("{c:+.0}"))]);
            bc.push(Item::text(label, "picture.corrections", json!({"brightness": b, "contrast": c})));
        }
    }
    gallery(ui, app, "picture_brightness", &bc, 5);
    ui.separator();
    heading(ui, "Picture Corrections Options");
    let now = look::current(&app.session).map(|l| l.corrections).unwrap_or_default();
    slider(ui, app, "Sharpness", f64::from(now.sharpen), -100.0..=100.0, "%", "picture.corrections", "sharpen");
    slider(ui, app, "Brightness", f64::from(now.brightness), -100.0..=100.0, "%", "picture.corrections", "brightness");
    slider(ui, app, "Contrast", f64::from(now.contrast), -100.0..=100.0, "%", "picture.corrections", "contrast");
}

/// Color: Saturation, Color Tone and Recolor as previews, More Variations, Set Transparent Color,
/// then exact values.
fn color_menu(ui: &mut Ui, app: &mut WordApp) {
    // As wide as its galleries: the submenu and the item below would stretch it.
    ui.set_max_width(7.0 * (TILE + 4.0));
    heading(ui, "Color Saturation");
    let sat: Vec<Item> = [0.0, 33.0, 66.0, 100.0, 200.0, 300.0, 400.0]
        .iter()
        .map(|v| {
            Item::text(crate::i18n::fmt(tl!("Saturation: {percent}%"), &[("percent", &format!("{v:.0}"))]), "picture.color", json!({"saturation": v}))
        })
        .collect();
    gallery(ui, app, "picture_saturation", &sat, 7);
    heading(ui, "Color Tone");
    let tone: Vec<Item> = [4700.0, 5300.0, 5900.0, 6500.0, 7200.0, 8800.0, 11200.0]
        .iter()
        .map(|k| {
            Item::text(
                crate::i18n::fmt(tl!("Temperature: {kelvin} K"), &[("kelvin", &format!("{k:.0}"))]),
                "picture.color",
                json!({"temperature": k}),
            )
        })
        .collect();
    gallery(ui, app, "picture_tone", &tone, 7);
    heading(ui, "Recolor");
    let recolor: Vec<Item> = [
        ("No Recolor", json!({"recolor": "none"})),
        ("Grayscale", json!({"recolor": "grayscale"})),
        ("Sepia", json!({"recolor": "sepia"})),
        ("Washout", json!({"recolor": "washout"})),
        ("Black and White: 25%", json!({"recolor": "blackAndWhite", "threshold": 25})),
        ("Black and White: 50%", json!({"recolor": "blackAndWhite", "threshold": 50})),
        ("Black and White: 75%", json!({"recolor": "blackAndWhite", "threshold": 75})),
    ]
    .into_iter()
    .map(|(l, v)| Item::new(l, "picture.color", v))
    .collect();
    gallery(ui, app, "picture_recolor", &recolor, 7);
    // The theme's accents (Accent 1 is the theme's fifth colour): a row dark, a row light.
    let theme = app.session.doc.settings.theme_colors.clone();
    for (kind, name) in [("dark", "Accent {n}, Dark"), ("light", "Accent {n}, Light")] {
        let row: Vec<Item> = theme
            .iter()
            .skip(4)
            .take(6)
            .enumerate()
            .map(|(i, c)| {
                let label = crate::i18n::fmt(tl!(name), &[("n", &(i + 1).to_string())]);
                Item::text(label, "picture.color", json!({"recolor": kind, "color": c.hex()}))
            })
            .collect();
        gallery(ui, app, &format!("picture_recolor_{kind}"), &row, 7);
    }
    ui.separator();
    ui.menu_button(tl!("More Variations"), |ui| {
        crate::colors::menu(ui, app, &theme, Target::at("picture.color", json!({"recolor": "dark", "color": null}), "/color"));
    });
    super::common::mi_icon(ui, app, "transparentColor", "Set Transparent Color", "ui.setTransparentColor", json!({}));
    ui.separator();
    heading(ui, "Picture Color Options");
    let now = look::current(&app.session).map(|l| l.color).unwrap_or_default();
    slider(ui, app, "Saturation", f64::from(now.saturation), 0.0..=400.0, "%", "picture.color", "saturation");
    slider(ui, app, "Temperature", f64::from(now.temperature), 1500.0..=11500.0, " K", "picture.color", "temperature");
}

/// Compress Pictures: the picture's pixels down to what a resolution needs at its size.
fn compress_menu(ui: &mut Ui, app: &mut WordApp) {
    let long = match wordcraft_engine::cmd::objects::selected(&app.session) {
        Some((_, wordcraft_doc::para::InlineObject::Image { w, h, .. })) => w.max(h),
        _ => 0.0,
    };
    for (label, ppi) in [("High fidelity: 330 ppi", 330.0), ("Print: 220 ppi", 220.0), ("Web: 150 ppi", 150.0), ("E-mail: 96 ppi", 96.0)] {
        // Points are 1/72 in: the longer side's pixels at that resolution.
        let px = (long / 72.0 * ppi).ceil().clamp(32.0, 20000.0);
        mi(ui, app, label, "picture.compress", json!({"maxPixels": px}));
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(tl!(text)).small().strong());
}

/// One exact value for `cmd`'s `key`, dragged with a live preview: one undo step for the drag.
#[allow(clippy::too_many_arguments)]
fn slider(ui: &mut Ui, app: &mut WordApp, label: &str, now: f64, range: std::ops::RangeInclusive<f64>, suffix: &str, cmd: &str, key: &str) {
    crate::widgets::row(ui, |ui| {
        ui.label(egui::RichText::new(tl!(label)).small());
        let mut v = now;
        let r = ui.add(egui::DragValue::new(&mut v).speed(1.0).range(range).suffix(suffix));
        if r.changed() {
            if r.dragged() && !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run(cmd, json!({key: v}));
        }
    });
}

/// A gallery tile: what it is called and the command it runs.
struct Item {
    label: String,
    id: &'static str,
    params: Value,
}

impl Item {
    fn new(label: &str, id: &'static str, params: Value) -> Item {
        Item { label: tl!(label).to_string(), id, params }
    }

    /// With a label already in the interface language.
    fn text(label: String, id: &'static str, params: Value) -> Item {
        Item { label, id, params }
    }
}

/// The side of a gallery tile (points).
const TILE: f32 = 52.0;
/// The longest side of the copy the tiles are drawn from (pixels).
const TILE_PX: u32 = 72;

/// The tiles drawn for one picture look, kept in egui's memory: (media, tiles by gallery and item).
type Tiles = (String, HashMap<(String, usize), Option<egui::TextureHandle>>);

/// A grid of the selected picture as each item would leave it; a click runs that item. The one
/// matching the picture's setting now is outlined. Without a picture to draw, plain items.
fn gallery(ui: &mut Ui, app: &mut WordApp, key: &str, items: &[Item], cols: usize) {
    let current = look::current(&app.session);
    let media = match wordcraft_engine::cmd::objects::selected(&app.session) {
        Some((_, wordcraft_doc::para::InlineObject::Image { media, .. })) => media,
        _ => String::new(),
    };
    let thumb = current.as_ref().and_then(|l| tile_source(app, ui.ctx(), &l.base));
    let t = crate::theme::Tokens::get(ui.ctx());
    let store = egui::Id::unique("picture_gallery_tiles");
    let mut tiles: Tiles =
        ui.ctx().data(|d| d.get_temp::<Tiles>(store)).filter(|(m, _)| *m == media).unwrap_or_else(|| (media.clone(), HashMap::new()));
    let mut clicked = None;
    egui::Grid::new(("picture_gallery", key)).spacing(egui::vec2(4.0, 4.0)).show(ui, |ui| {
        for (i, it) in items.iter().enumerate() {
            let tex = match &thumb {
                Some(thumb) => tiles
                    .1
                    .entry((key.to_string(), i))
                    .or_insert_with(|| {
                        look::preview(&app.session, thumb, it.id, &it.params).map(|img| {
                            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                            ui.ctx().load_texture(format!("picture_gallery_{key}_{i}"), ci, egui::TextureOptions::LINEAR)
                        })
                    })
                    .clone(),
                None => None,
            };
            let on = current.as_ref().is_some_and(|l| is_current(l, it.id, &it.params));
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(TILE, TILE), egui::Sense::click());
            let painter = ui.painter();
            // A checkerboard shows what an item makes see-through.
            painter.rect_filled(rect, 2.0, egui::Color32::WHITE);
            if let Some(tex) = &tex {
                let size = tex.size_vec2();
                let k = (TILE - 6.0) / size.x.max(size.y).max(1.0);
                painter.image(
                    tex.id(),
                    egui::Rect::from_center_size(rect.center(), size * k),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, &it.label, egui::FontId::proportional(9.0), t.text);
            }
            let edge = if on {
                egui::Stroke::new(2.0, t.accent)
            } else if resp.hovered() {
                egui::Stroke::new(1.5, t.accent)
            } else {
                egui::Stroke::new(1.0, t.border)
            };
            painter.rect_stroke(rect, 2.0, edge, egui::StrokeKind::Inside);
            let resp = resp.on_hover_text(&it.label);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &it.label));
            if resp.clicked() {
                clicked = Some(i);
            }
            if (i + 1) % cols.max(1) == 0 {
                ui.end_row();
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(store, tiles));
    if let Some(it) = clicked.and_then(|i| items.get(i)) {
        crate::widgets::run_from_ribbon(app, it.id, it.params.clone());
        ui.close();
    }
}

/// Whether `id` with `v` is what the picture has now (running it would change nothing of its own).
fn is_current(cur: &look::PictureLook, id: &str, v: &Value) -> bool {
    let mut after = cur.clone();
    if look::change(id, v, &mut after).is_err() {
        return false;
    }
    match id {
        "picture.corrections" => after.corrections == cur.corrections && v.as_object().is_some_and(|o| o.keys().all(|k| corrections_part(cur, k, v))),
        "picture.color" => after.color == cur.color && v.as_object().is_some_and(|o| o.keys().all(|k| color_part(cur, k))),
        "picture.effects" => after.effect == cur.effect,
        "picture.transparency" => after.transparency == cur.transparency,
        "picture.style" => after.style == cur.style && cur.border.is_none(),
        _ => false,
    }
}

/// A Brightness/Contrast tile is on only when both match; a Sharpen tile when the sharpness does.
fn corrections_part(cur: &look::PictureLook, k: &str, v: &Value) -> bool {
    let want = v.get(k).and_then(Value::as_f64).map(|x| x as f32);
    match k {
        "brightness" => want == Some(cur.corrections.brightness),
        "contrast" => want == Some(cur.corrections.contrast),
        "sharpen" => want == Some(cur.corrections.sharpen),
        _ => false,
    }
}

/// A Recolor tile isn't on just because the saturation is at 100%.
fn color_part(cur: &look::PictureLook, k: &str) -> bool {
    match k {
        "saturation" | "temperature" => cur.color.recolor.is_none(),
        _ => true,
    }
}

/// The copy of `base` the tiles are drawn from, kept in egui's memory per media item.
fn tile_source(app: &WordApp, ctx: &egui::Context, base: &str) -> Option<Arc<image::RgbaImage>> {
    let bytes = app.session.doc.media.get(base)?;
    let id = egui::Id::unique(("picture_gallery_source", base, bytes.len()));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<Arc<image::RgbaImage>>>(id)) {
        return t;
    }
    let img = wordcraft_geom::picture::decode(bytes, wordcraft_geom::MAX_DECODED_BYTES)
        .ok()
        .map(|img| Arc::new(img.thumbnail(TILE_PX, TILE_PX).into_rgba8()));
    ctx.data_mut(|d| d.insert_temp(id, img.clone()));
    img
}
