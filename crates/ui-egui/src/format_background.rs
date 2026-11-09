//! The Format Background pane (Draw › Edit › Format Background): the page's fill, as Word's
//! Fill Effects pane — no fill, a solid colour, a gradient, a picture or texture, or a pattern,
//! with transparency and a live preview of the page. Changes are a draft until Apply, which runs
//! `design.pageFill` as one undo step; Reset takes the document's fill back and Close leaves it.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind};
use wordcraft_doc::wordart::GradStop;
use wordcraft_doc::{PagePattern, Rgb};

use crate::WordApp;
use crate::theme::Tokens;

/// What the pane will apply, before Apply.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum DraftFill {
    #[default]
    None,
    Solid {
        color: Rgb,
    },
    Gradient {
        start: Rgb,
        end: Rgb,
        angle: f32,
        transparency: f32,
        /// The document's own stops, so Apply keeps interior colours and per-stop transparencies.
        stops: Vec<GradStop>,
    },
    Picture {
        path: Option<String>,
        media: Option<String>,
        data: Option<String>,
        tile: bool,
        transparency: f32,
    },
    Pattern {
        preset: String,
        fore: Rgb,
        back: Rgb,
        transparency: f32,
    },
}

/// The pane's state: the document it was read from and that document's fill at the time, so an
/// undo or another window's change is reflected instead of being overwritten by a stale draft.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub loaded: bool,
    pub doc: u64,
    pub base: u64,
    pub fill: DraftFill,
}

impl Draft {
    /// The document's current fill.
    pub fn from_doc(doc: &wordcraft_doc::Document) -> Draft {
        let s = &doc.settings;
        let fill = if let Some(p) = &s.page_picture {
            DraftFill::Picture {
                path: None,
                media: Some(p.media.clone()),
                data: None,
                tile: p.fill_type.eq_ignore_ascii_case("tile"),
                transparency: p.transparency,
            }
        } else if let Some(g) = &s.page_gradient {
            let stops = g.stops.clone();
            let start = stops.first().map_or(Rgb::WHITE, |s| s.color);
            let end = stops.last().map_or(Rgb::WHITE, |s| s.color);
            let angle = match g.kind {
                GradientKind::Linear { angle, .. } if angle.is_finite() => angle,
                _ => 90.0,
            };
            let transparency = stops.iter().map(|s| s.transparency).sum::<f32>() / stops.len().max(1) as f32;
            DraftFill::Gradient { start, end, angle, transparency, stops }
        } else if let Some(p) = &s.page_pattern {
            DraftFill::Pattern { preset: p.preset.clone(), fore: p.fore, back: p.back, transparency: p.transparency }
        } else if let Some(c) = s.page_color {
            DraftFill::Solid { color: c }
        } else {
            DraftFill::None
        };
        Draft { loaded: true, doc: 0, base: fill_fingerprint(doc), fill }
    }

    /// The command params that make this fill (a picture keeps the path or data the picker gave,
    /// else the media key already in the document).
    pub fn params(&self) -> serde_json::Value {
        use serde_json::json;
        match &self.fill {
            DraftFill::None => json!({"fill": "none"}),
            DraftFill::Solid { color } => json!({"fill": "solid", "color": color.hex()}),
            DraftFill::Gradient { start, end, angle, transparency, stops } => {
                let mut list: Vec<serde_json::Value> =
                    stops.iter().map(|s| json!({"pos": s.pos, "color": s.color.hex(), "transparency": s.transparency})).collect();
                if !(2..=8).contains(&list.len()) {
                    list = vec![
                        json!({"pos": 0.0, "color": start.hex(), "transparency": transparency}),
                        json!({"pos": 100.0, "color": end.hex(), "transparency": transparency}),
                    ];
                } else {
                    let mean = stops.iter().map(|s| s.transparency).sum::<f32>() / list.len() as f32;
                    let last = list.len() - 1;
                    list[0]["color"] = json!(start.hex());
                    list[last]["color"] = json!(end.hex());
                    // The slider is one transparency for the fill: move every stop only when the
                    // user changed it, so an untouched multi-stop gradient keeps its own values.
                    if (transparency - mean).abs() > 0.01 {
                        for s in &mut list {
                            s["transparency"] = json!(transparency);
                        }
                    }
                }
                json!({"fill": "gradient", "angle": angle, "stops": list})
            }
            DraftFill::Picture { path, media, data, tile, transparency } => {
                let mut p = json!({"fill": "picture", "tile": tile, "transparency": transparency});
                if let Some(o) = p.as_object_mut() {
                    if let Some(path) = path {
                        o.insert("path".into(), json!(path));
                    } else if let Some(data) = data {
                        o.insert("data".into(), json!(data));
                    } else if let Some(media) = media {
                        o.insert("media".into(), json!(media));
                    }
                }
                p
            }
            DraftFill::Pattern { preset, fore, back, transparency } => json!({
                "fill": "pattern", "preset": preset, "fore": fore.hex(), "back": back.hex(), "transparency": transparency,
            }),
        }
    }
}

/// A hash of the page fill settings alone: typing elsewhere in the document doesn't disturb the
/// pane's draft, but an undo of the fill (or another window's change) does.
fn fill_fingerprint(doc: &wordcraft_doc::Document) -> u64 {
    let s = &doc.settings;
    let key = format!("{:?}{:?}{:?}{:?}", s.page_color, s.page_picture, s.page_gradient, s.page_pattern);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// The pane was closed: read the document's fill afresh next time it opens, and forget a picture
/// picker it asked for (its result goes nowhere now).
pub fn forget(app: &mut WordApp) {
    app.background.loaded = false;
    app.background_picture_pending = false;
}

/// The pane, while open.
pub fn pane(app: &mut WordApp, ui: &mut Ui) {
    let doc_id = app.session.document_id();
    let fp = fill_fingerprint(&app.session.doc);
    if !app.background.loaded || app.background.doc != doc_id || app.background.base != fp {
        app.background = Draft::from_doc(&app.session.doc);
        app.background.doc = doc_id;
    }
    let theme = app.session.doc.settings.theme_colors.clone();
    let accent = theme.get(4).copied().unwrap_or(Rgb(0x15, 0x60, 0x82));

    ui.label(egui::RichText::new(tl!("Fill")).strong());
    ui.add_space(2.0);
    let kind = lang_kind(&app.background.fill);
    let mut pick = kind;
    ui.radio_value(&mut pick, 0, tl!("No fill"));
    ui.radio_value(&mut pick, 1, tl!("Solid"));
    ui.radio_value(&mut pick, 2, tl!("Gradient"));
    ui.radio_value(&mut pick, 3, tl!("Picture or texture"));
    ui.radio_value(&mut pick, 4, tl!("Pattern"));
    if pick != kind {
        app.background.fill = match pick {
            1 => DraftFill::Solid { color: accent },
            2 => DraftFill::Gradient { start: accent, end: Rgb::WHITE, angle: 90.0, transparency: 0.0, stops: Vec::new() },
            3 => DraftFill::Picture { path: None, media: None, data: None, tile: false, transparency: 0.0 },
            4 => DraftFill::Pattern { preset: "ltDnDiag".into(), fore: Rgb::BLACK, back: Rgb::WHITE, transparency: 0.0 },
            _ => DraftFill::None,
        };
    }
    ui.add_space(6.0);

    let mut picking = false;
    match &mut app.background.fill {
        DraftFill::None => {
            ui.label(egui::RichText::new(tl!("No page background.")).weak());
        }
        DraftFill::Solid { color } => {
            if let Some(c) = color_button(ui, &theme, Id::unique("formatBackground.solid"), tl!("Color"), *color) {
                *color = c;
            }
        }
        DraftFill::Gradient { start, end, angle, transparency, .. } => {
            if let Some(c) = color_button(ui, &theme, Id::unique("formatBackground.start"), tl!("Start"), *start) {
                *start = c;
            }
            if let Some(c) = color_button(ui, &theme, Id::unique("formatBackground.end"), tl!("End"), *end) {
                *end = c;
            }
            ui.horizontal(|ui| {
                ui.label(tl!("Direction"));
                let mut dir = direction_of(*angle);
                craft_ui::ComboBox::from_id_salt("formatBackground.direction").selected_text(tl!(DIRECTIONS[dir].0)).show_ui(ui, |ui| {
                    for (i, (label, _)) in DIRECTIONS.iter().enumerate() {
                        ui.selectable_value(&mut dir, i, tl!(label));
                    }
                });
                if dir != direction_of(*angle) {
                    *angle = DIRECTIONS[dir].1;
                }
            });
            let mut a = *angle;
            let r = ui.add(egui::DragValue::new(&mut a).range(0.0..=359.9).clamp_existing_to_range(false).suffix("°").speed(1.0));
            if r.changed() {
                *angle = a;
            }
            let mut t = *transparency;
            if percent(ui, tl!("Transparency"), &mut t).is_some() {
                *transparency = t;
            }
        }
        DraftFill::Picture { path, media, data, tile, transparency } => {
            let chosen = path.is_some() || media.is_some() || data.is_some();
            ui.label(egui::RichText::new(if chosen { tl!("Picture") } else { tl!("No picture chosen") }).weak());
            if ui.button(tl!("Select Picture…")).clicked() {
                picking = true;
            }
            let mut t = *tile;
            if ui.checkbox(&mut t, tl!("Tile picture as texture")).changed() {
                *tile = t;
            }
            let mut tr = *transparency;
            if percent(ui, tl!("Transparency"), &mut tr).is_some() {
                *transparency = tr;
            }
        }
        DraftFill::Pattern { preset, fore, back, transparency } => {
            ui.label(tl!("Pattern"));
            let mut chosen = preset.clone();
            egui::ScrollArea::vertical().max_height(96.0).show(ui, |ui| {
                crate::rtl::grid(ui, egui::Grid::new("formatBackground.patterns").num_columns(4).spacing(vec2(4.0, 4.0)), |ui| {
                    for (i, p) in wordcraft_doc::PATTERN_PRESETS.iter().enumerate() {
                        let on = *preset == p.name;
                        if pattern_swatch(ui, p.name, *fore, *back, on).clicked() {
                            chosen = p.name.to_string();
                        }
                        if i % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
            });
            if chosen != *preset {
                *preset = chosen;
            }
            if let Some(c) = color_button(ui, &theme, Id::unique("formatBackground.fore"), tl!("Foreground"), *fore) {
                *fore = c;
            }
            if let Some(c) = color_button(ui, &theme, Id::unique("formatBackground.back"), tl!("Background"), *back) {
                *back = c;
            }
            let mut tr = *transparency;
            if percent(ui, tl!("Transparency"), &mut tr).is_some() {
                *transparency = tr;
            }
        }
    }
    if picking {
        app.pick_background_picture();
    }

    ui.add_space(8.0);
    preview(app, ui);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(tl!("Apply")).clicked() {
            let params = app.background.params();
            if app.run("design.pageFill", params).is_ok() {
                app.background = Draft::from_doc(&app.session.doc);
                app.background.doc = app.session.document_id();
            }
        }
        if ui.button(tl!("Reset")).clicked() {
            app.background = Draft::from_doc(&app.session.doc);
            app.background.doc = app.session.document_id();
        }
        if ui.button(tl!("Close")).clicked() {
            let _ = app.run("view.formatBackgroundPane", serde_json::json!({"value": false}));
        }
    });
}

/// The radio the draft's fill is.
fn lang_kind(f: &DraftFill) -> usize {
    match f {
        DraftFill::None => 0,
        DraftFill::Solid { .. } => 1,
        DraftFill::Gradient { .. } => 2,
        DraftFill::Picture { .. } => 3,
        DraftFill::Pattern { .. } => 4,
    }
}

/// Word's gradient directions, as the angle they run at (0° left to right, 90° top to bottom).
const DIRECTIONS: [(&str, f32); 8] = [
    ("Linear Down", 90.0),
    ("Linear Up", 270.0),
    ("Linear Right", 0.0),
    ("Linear Left", 180.0),
    ("Diagonal down-right", 45.0),
    ("Diagonal up-right", 315.0),
    ("Diagonal down-left", 135.0),
    ("Diagonal up-left", 225.0),
];

fn direction_of(angle: f32) -> usize {
    DIRECTIONS.iter().position(|(_, a)| (a - angle.rem_euclid(360.0)).abs() < 0.01).unwrap_or(0)
}

/// A labelled 0–100% slider.
fn percent(ui: &mut Ui, label: &str, v: &mut f32) -> Option<egui::Response> {
    ui.horizontal(|ui| {
        let l = ui.label(label);
        let s = ui.add(egui::Slider::new(v, 0.0..=100.0).suffix("%").fixed_decimals(0)).labelled_by(l.id);
        s.changed().then_some(s)
    })
    .inner
}

/// A colour swatch with the theme colours' drop-down; the colour picked.
fn color_button(ui: &mut Ui, theme: &[Rgb], id: Id, label: &str, c: Rgb) -> Option<Rgb> {
    let mut hex = c.hex();
    crate::colors::take_field(ui.ctx(), id, &mut hex);
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label(label);
        let t = Tokens::get(ui.ctx());
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 14.0), Sense::hover());
        ui.painter().rect_filled(r, 2.0, crate::theme::c32(c));
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        ui.menu_button("▾", |ui| crate::colors::field_menu(ui, theme, id, &mut hex));
    });
    if let Some(n) = Rgb::parse(&hex)
        && n != c
    {
        picked = Some(n);
    }
    picked
}

/// A pattern preset's marks, repeated over `rect` at `size` points and clipped to it: some presets
/// draw strokes past their tile (`dkDnDiag` to y = 2, `dkUpDiag` to y = -1), and the last tile
/// column and row run past its edge, so without the clip a mark lands over the page's margin or
/// the gallery beside its swatch.
fn paint_pattern_marks(p: &egui::Painter, rect: Rect, tile: &wordcraft_doc::PatternTile, size: f32, ink: Color32) {
    let p = &p.with_clip_rect(rect.intersect(p.clip_rect()));
    let mut x = rect.min.x;
    while x < rect.max.x {
        let mut y = rect.min.y;
        while y < rect.max.y {
            for [rx, ry, rw, rh] in &tile.rects {
                p.rect_filled(Rect::from_min_size(pos2(x + rx * size, y + ry * size), vec2(rw * size, rh * size)).intersect(rect), 0.0, ink);
            }
            for [cx, cy, rad] in &tile.dots {
                let c = pos2(x + cx * size, y + cy * size);
                if rect.contains(c) {
                    p.circle_filled(c, (rad * size).max(0.6), ink);
                }
            }
            for [x0, y0, x1, y1, w] in &tile.strokes {
                p.line_segment([pos2(x + x0 * size, y + y0 * size), pos2(x + x1 * size, y + y1 * size)], Stroke::new((w * size).max(0.5), ink));
            }
            y += size;
        }
        x += size;
    }
}

/// A preset's own marks in a small square (the same pattern geometry the page draws), a click
/// selecting it.
fn pattern_swatch(ui: &mut Ui, preset: &str, fore: Rgb, back: Rgb, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(34.0, 30.0), Sense::click());
    let tile = PagePattern { preset: preset.into(), fore, back, transparency: 0.0 };
    ui.painter().rect_filled(r.shrink(2.0), 2.0, crate::theme::c32(back));
    let inner = r.shrink(3.0);
    paint_pattern_marks(ui.painter(), inner, &tile.tile(), 10.0, crate::theme::c32(fore));
    ui.painter().rect_stroke(
        r.shrink(2.0),
        2.0,
        Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.border }),
        egui::StrokeKind::Inside,
    );
    let name = wordcraft_doc::PATTERN_PRESETS.iter().find(|p| p.name == preset).map_or(preset, |p| p.label);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, name));
    resp.on_hover_text(name)
}

/// A small page showing the draft fill behind a few lines of text.
fn preview(app: &WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (pw, ph) = app.session.doc.sections().first().map(|(_, s)| (s.page_w, s.page_h)).unwrap_or((612.0, 792.0));
    let w = (ui.available_width() - 8.0).clamp(120.0, 240.0);
    let h = (w * ph.max(1.0) / pw.max(1.0)).clamp(80.0, 180.0);
    let (page, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(page.translate(vec2(0.0, 2.0)).expand(1.5), 1.0, t.page_shadow);
    painter.rect_filled(page, 0.0, Color32::WHITE);
    let body = page.shrink2(vec2(page.width() * 0.09, page.height() * 0.08));
    paint_draft_fill(app, ui, painter, body);
    // A few lines of text, so the fill is seen behind writing.
    let ink = if is_dark(&app.background.fill) { Color32::from_gray(230) } else { Color32::from_gray(90) };
    let mut y = body.min.y + 6.0;
    let mut i = 0;
    while y < body.max.y - 4.0 {
        let len = if i % 5 == 4 {
            0.55
        } else if i % 3 == 1 {
            0.92
        } else {
            0.78
        };
        let width = body.width() * len;
        painter.rect_filled(Rect::from_min_size(pos2(body.min.x, y), vec2(width, 2.0)), 0.5, ink.gamma_multiply(0.55));
        y += 8.0;
        i += 1;
    }
    painter.rect_stroke(page, 0.0, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    resp.on_hover_text(tl!("Preview"));
}

/// Paint the draft fill over `rect`, exactly as the page draws it (solid, gradient, pattern,
/// picture). The pattern's presets draw strokes past their own tile (`dkDnDiag` to y = 2), so the
/// painter is clipped to `rect` as the page's tile is clipped by the page.
fn paint_draft_fill(app: &WordApp, ui: &Ui, p: &egui::Painter, rect: Rect) {
    let p = &p.with_clip_rect(rect.intersect(p.clip_rect()));
    match &app.background.fill {
        DraftFill::None => {}
        DraftFill::Solid { color } => {
            p.rect_filled(rect, 0.0, crate::theme::c32(*color));
        }
        DraftFill::Gradient { start, end, angle, transparency, .. } => {
            let g = Gradient {
                stops: vec![
                    GradStop { pos: 0.0, color: *start, transparency: *transparency },
                    GradStop { pos: 100.0, color: *end, transparency: *transparency },
                ],
                kind: GradientKind::Linear { angle: *angle, scaled: false },
                tile: None,
                rot_with_shape: false,
                flip: GradientFlip::None,
            };
            crate::format_shape::paint_fill(p, rect, &Fill::Gradient(Arc::new(g)));
        }
        DraftFill::Pattern { preset, fore, back, transparency } => {
            let alpha = 1.0 - transparency.clamp(0.0, 100.0) / 100.0;
            p.rect_filled(rect, 0.0, crate::theme::c32(*back).gamma_multiply(alpha));
            let src = PagePattern { preset: preset.clone(), fore: *fore, back: *back, transparency: 0.0 }.tile();
            let ink = crate::theme::c32(*fore).gamma_multiply(alpha);
            paint_pattern_marks(p, rect, &src, 10.0, ink);
        }
        DraftFill::Picture { .. } => paint_picture_preview(app, ui, p, rect),
    }
}

/// The cache key of the chosen picture: its path, a fingerprint of its `data`, or its media key.
/// The data fingerprint samples the head, middle and tail: a PNG's head is its signature and
/// header, the same for every picture of a given length, so the head alone can't tell two apart.
pub(crate) fn picture_cache_key(path: &Option<String>, data: &Option<String>, media: &Option<String>) -> Option<String> {
    if let Some(path) = path {
        return Some(path.clone());
    }
    if let Some(d) = data {
        // Hash the whole string: sampled windows can collide, and two pictures of one length
        // would then share a cached texture (or a cached failure).
        let mut h = std::collections::hash_map::DefaultHasher::new();
        d.hash(&mut h);
        return Some(format!("data:{}:{:016x}", d.len(), h.finish()));
    }
    media.as_ref().map(|m| format!("media:{m}"))
}

/// The chosen picture's thumbnail, its texture cached by path, data or media key; a dashed frame
/// when none is chosen or it can't be read.
fn paint_picture_preview(app: &WordApp, ui: &Ui, p: &egui::Painter, rect: Rect) {
    let DraftFill::Picture { path, media, data, tile, transparency } = &app.background.fill else {
        return;
    };
    let key = picture_cache_key(path, data, media);
    let Some(key) = key else {
        dashed_frame(p, rect, Tokens::get(ui.ctx()).border_strong);
        return;
    };
    let tex_id = Id::unique(("formatBackground.picture", &key));
    // Only the picture now chosen keeps a texture: a user trying several in the pane would
    // otherwise leave each one's texture, and its pixels, in the context.
    let shown_id = Id::unique("formatBackground.picture.shown");
    if ui.ctx().data(|d| d.get_temp::<String>(shown_id)).as_deref() != Some(key.as_str()) {
        ui.ctx().data_mut(|d| {
            if let Some(previous) = d.get_temp::<String>(shown_id) {
                let _ = d.remove_temp::<Option<egui::TextureHandle>>(Id::unique(("formatBackground.picture", &previous)));
            }
            d.insert_temp(shown_id, key.clone());
        });
    }
    // A load that fails (an unreadable file, a metafile, a picture past the size limit) is cached
    // too: the pane reads and decodes the file once, not on every frame it is open.
    let tex = match ui.ctx().data(|d| d.get_temp::<Option<egui::TextureHandle>>(tex_id)) {
        Some(cached) => cached,
        None => {
            let loaded = (|| {
                let bytes = if let Some(path) = path {
                    wordcraft_engine::io::read_file(path).ok()
                } else if let Some(media) = media {
                    app.session.doc.media.get(media).map(|b| b.to_vec())
                } else {
                    data.as_deref().and_then(wordcraft_engine::cmd::insert::base64_decode)
                }?;
                let img = thumbnail(&bytes)?;
                Some(ui.ctx().load_texture(format!("formatBackground.picture::{key}"), img, egui::TextureOptions::LINEAR))
            })();
            ui.ctx().data_mut(|d| d.insert_temp(tex_id, loaded.clone()));
            loaded
        }
    };
    let Some(tex) = tex else {
        dashed_frame(p, rect, Tokens::get(ui.ctx()).border_strong);
        return;
    };
    p.rect_filled(rect, 0.0, Color32::WHITE);
    let a = (1.0 - transparency.clamp(0.0, 100.0) / 100.0).clamp(0.0, 1.0);
    let tint = Color32::WHITE.gamma_multiply(a);
    if *tile {
        let size = 28.0_f32;
        let mut x = rect.min.x;
        while x < rect.max.x {
            let mut y = rect.min.y;
            while y < rect.max.y {
                let r = Rect::from_min_size(pos2(x, y), vec2(size, size)).intersect(rect);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(r.width() / size, r.height() / size));
                p.image(tex.id(), r, uv, tint);
                y += size;
            }
            x += size;
        }
    } else {
        p.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    }
}

/// A dashed rectangle, for a picture that isn't there.
fn dashed_frame(p: &egui::Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let step = 8.0;
    let mut y = rect.min.y;
    while y < rect.max.y {
        p.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
        y += step;
    }
    let mut x = rect.min.x;
    while x < rect.max.x {
        p.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], stroke);
        x += step;
    }
}

/// Decode `bytes` to a thumbnail the pane can show, bounded (hostile files).
fn thumbnail(bytes: &[u8]) -> Option<egui::ColorImage> {
    if bytes.len() > 64 << 20 {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 << 20);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    let img = reader.decode().ok()?.thumbnail(320, 320);
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()))
}

/// Whether the fill is dark enough that preview text should be light.
fn is_dark(f: &DraftFill) -> bool {
    let lum = |c: Rgb| 0.2126 * f32::from(c.0) + 0.7152 * f32::from(c.1) + 0.0722 * f32::from(c.2);
    match f {
        DraftFill::None => false,
        DraftFill::Solid { color } => lum(*color) < 128.0,
        DraftFill::Gradient { start, end, .. } => (lum(*start) + lum(*end)) / 2.0 < 128.0,
        DraftFill::Pattern { .. } => false,
        DraftFill::Picture { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pattern's marks stop at the rectangle they are painted in: `dkDnDiag`'s strokes run to
    /// y = 2 and `dkUpDiag`'s to y = -1, so without the clip they land over the page's margin in
    /// the preview and over the gallery beside a swatch.
    #[test]
    fn pattern_marks_are_clipped_to_their_rectangle() {
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(34.0, 30.0));
        let ctx = egui::Context::default();
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            for preset in ["dkDnDiag", "dkUpDiag", "smCheck", "pct90"] {
                let tile = PagePattern { preset: preset.into(), fore: Rgb::BLACK, back: Rgb::WHITE, transparency: 0.0 }.tile();
                paint_pattern_marks(&p, rect, &tile, 10.0, Color32::BLACK);
            }
        });
        let shapes = std::mem::take(&mut out.shapes);
        out.drop_without_applying_deltas();
        assert!(shapes.iter().any(|s| !rect.contains_rect(s.shape.visual_bounding_rect())), "a preset's marks do leave the rectangle");
        for s in &shapes {
            assert!(rect.expand(0.1).contains_rect(s.clip_rect), "a mark was not clipped to {rect:?}: {:?}", s.clip_rect);
        }
    }

    /// A picture that can't be decoded is tried once: its failure is cached under the picture's
    /// key, so a repaint doesn't read and decode the file again. Choosing another picture
    /// releases the one before it, so trying several doesn't collect them all.
    #[test]
    fn picture_textures_are_cached_once_and_released() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 200, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("x")), Default::default());
        a.session.doc.media.insert("good.png".into(), Arc::new(png));
        a.session.doc.media.insert("junk.png".into(), Arc::new(b"not a picture".to_vec()));
        let entry =
            |ctx: &egui::Context, key: &str| ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(Id::unique(("formatBackground.picture", key))));
        let frame = |ctx: &egui::Context, a: &WordApp| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                paint_picture_preview(a, ui, ui.painter(), Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 30.0)));
            })
            .drop_without_applying_deltas();
        };
        let ctx = egui::Context::default();
        a.background.fill = DraftFill::Picture { path: None, media: Some("good.png".into()), data: None, tile: false, transparency: 0.0 };
        frame(&ctx, &a);
        assert!(matches!(entry(&ctx, "media:good.png"), Some(Some(_))), "the decoded picture is cached");
        // An unreadable one caches its failure, so it isn't decoded every frame.
        a.background.fill = DraftFill::Picture { path: None, media: Some("junk.png".into()), data: None, tile: false, transparency: 0.0 };
        frame(&ctx, &a);
        assert!(matches!(entry(&ctx, "media:junk.png"), Some(None)), "the failed load is cached");
        frame(&ctx, &a);
        assert!(matches!(entry(&ctx, "media:junk.png"), Some(None)), "and still is");
        // The picture before it is released.
        assert!(entry(&ctx, "media:good.png").is_none(), "the previous picture's texture is gone");
    }
}
