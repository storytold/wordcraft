//! The Format Background pane (Draw › Edit › Format Background): the page's fill, as Word's
//! Fill Effects pane — no fill, a solid colour, a gradient, a picture or texture, or a pattern,
//! with transparency and a live preview of the page. Changes are a draft until Apply, which runs
//! `design.pageFill` as one undo step; Reset takes the document's fill back and Close leaves it.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::color_ui::ui::ColorPickerDialog;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::display::Text;
use craft::ui::form::{Field, SliderField};
use craft::ui::layout_components::Stack;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::text::NumberField;
use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use wordcraft_doc::fill::{Fill, Gradient, GradientFlip, GradientKind};
use wordcraft_doc::wordart::GradStop;
use wordcraft_doc::{PagePattern, Rgb};

use crate::WordApp;
use crate::colors::{ColorFieldEmpty, Palette, color_field};
use crate::dialogs::host::{DialogHooks, Settle};
use crate::theme::Tokens;
/// The most picture bytes the preview reads: past it the pane shows a dashed frame, like the
/// engine's thumbnail of an oversized picture. The read stops here, so a huge selection is
/// refused before it is held whole.
const MAX_PREVIEW_BYTES: u64 = 64 << 20;

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
        /// The cache key of `data`, hashed once when the picker hands the picture over: the pane
        /// must not hash a large base64 string on every repaint.
        data_key: Option<String>,
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
                data_key: None,
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
    /// The gradient's stops as the page draws them: the document's own stops (2 to 8), with the
    /// endpoint colours from the start/end pickers; fewer or more stops fall back to the two
    /// endpoints. The one transparency slider moves every stop only once the user changes it, so
    /// an untouched multi-stop gradient keeps its own values.
    pub(crate) fn gradient_stops(stops: &[GradStop], start: Rgb, end: Rgb, transparency: f32) -> Vec<GradStop> {
        if (2..=8).contains(&stops.len()) {
            let mean = stops.iter().map(|s| s.transparency).sum::<f32>() / stops.len() as f32;
            let mut kept: Vec<GradStop> = stops.to_vec();
            kept[0].color = start;
            if let Some(last) = kept.last_mut() {
                last.color = end;
            }
            if (transparency - mean).abs() > 0.01 {
                for s in &mut kept {
                    s.transparency = transparency;
                }
            }
            return kept;
        }
        vec![GradStop { pos: 0.0, color: start, transparency }, GradStop { pos: 100.0, color: end, transparency }]
    }

    pub fn params(&self) -> serde_json::Value {
        use serde_json::json;
        match &self.fill {
            DraftFill::None => json!({"fill": "none"}),
            DraftFill::Solid { color } => json!({"fill": "solid", "color": color.hex()}),
            DraftFill::Gradient { start, end, angle, transparency, stops } => {
                let list: Vec<serde_json::Value> = Self::gradient_stops(stops, *start, *end, *transparency)
                    .iter()
                    .map(|s| json!({"pos": s.pos, "color": s.color.hex(), "transparency": s.transparency}))
                    .collect();
                json!({"fill": "gradient", "angle": angle, "stops": list})
            }
            DraftFill::Picture { path, media, data, tile, transparency, .. } => {
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

/// The pane was closed: read the document's fill afresh next time it opens.
pub fn forget(app: &mut WordApp) {
    app.background.loaded = false;
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

    Text::new(Label::from(tl!("Fill"))).show(ui);
    ui.add_space(2.0);
    let kind = lang_kind(&app.background.fill);
    let mut pick = kind as u8;
    RadioGroup::new(Label::from(tl!("Fill type")), Bind::bound(&mut pick)).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, cx| {
            for (i, label) in ["No fill", "Solid", "Gradient", "Picture or texture", "Pattern"].into_iter().enumerate() {
                let side = SideLabel::new().layout(FieldLayout::After);
                cx.labelled(ui, i, Label::from(tl!(label)), side, Radio::new(i as u8));
            }
        });
    });
    if usize::from(pick) != kind {
        app.background.fill = match pick {
            1 => DraftFill::Solid { color: accent },
            2 => DraftFill::Gradient { start: accent, end: Rgb::WHITE, angle: 90.0, transparency: 0.0, stops: Vec::new() },
            3 => DraftFill::Picture { path: None, media: None, data: None, data_key: None, tile: false, transparency: 0.0 },
            4 => DraftFill::Pattern { preset: "ltDnDiag".into(), fore: Rgb::BLACK, back: Rgb::WHITE, transparency: 0.0 },
            _ => DraftFill::None,
        };
    }
    ui.add_space(6.0);

    let palette = Palette::of(app);
    let mut picking = false;
    let mut more: Option<DraftColor> = None;
    match &mut app.background.fill {
        DraftFill::None => {
            Text::new(Label::from(tl!("No page background."))).show(ui);
        }
        DraftFill::Solid { color } => {
            let out = color_button(ui, &palette, tl!("Color"), *color);
            if let Some(c) = out.picked {
                *color = c;
            }
            more = out.more.then_some(DraftColor::Solid);
        }
        DraftFill::Gradient { start, end, angle, transparency, .. } => {
            let out = color_button(ui, &palette, tl!("Start"), *start);
            if let Some(c) = out.picked {
                *start = c;
            }
            if out.more {
                more = Some(DraftColor::Start);
            }
            let out = color_button(ui, &palette, tl!("End"), *end);
            if let Some(c) = out.picked {
                *end = c;
            }
            if out.more {
                more = Some(DraftColor::End);
            }
            let entries: Vec<Entry<u8>> =
                DIRECTIONS.iter().enumerate().map(|(i, (label, _))| Entry::new(i as u8, tl!(label))).collect();
            let mut picked = None;
            Field::new(Label::from(tl!("Direction"))).show(ui, |ui, _| {
                picked = Select::new(tl!("Direction"), Bind::given(direction_of(*angle) as u8), &entries).show(ui).events.committed().copied();
            });
            if let Some(i) = picked {
                *angle = DIRECTIONS[usize::from(i)].1;
            }
            NumberField::new(Label::from(tl!("Angle")), Bind::bound(angle)).unit_kind(UnitKind::Degrees).range(0.0..=359.9).show(ui);
            SliderField::new(tl!("Transparency"), Bind::bound(transparency), 0.0..=100.0).unit_kind(UnitKind::Percent).show(ui);
        }
        DraftFill::Picture { path, media, data, tile, transparency, .. } => {
            let chosen = path.is_some() || media.is_some() || data.is_some();
            Text::new(Label::from(if chosen { tl!("Picture") } else { tl!("No picture chosen") })).show(ui);
            if Button::new(tl!("Select Picture…").to_string()).show(ui).clicked() {
                picking = true;
            }
            Checkbox::new(Bind::bound(tile)).label(Label::from(tl!("Tile picture as texture"))).show(ui);
            SliderField::new(tl!("Transparency"), Bind::bound(transparency), 0.0..=100.0).unit_kind(UnitKind::Percent).show(ui);
        }
        DraftFill::Pattern { preset, fore, back, transparency } => {
            Text::new(Label::from(tl!("Pattern"))).show(ui);
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
            let out = color_button(ui, &palette, tl!("Foreground"), *fore);
            if let Some(c) = out.picked {
                *fore = c;
            }
            if out.more {
                more = Some(DraftColor::Fore);
            }
            let out = color_button(ui, &palette, tl!("Background"), *back);
            if let Some(c) = out.picked {
                *back = c;
            }
            if out.more {
                more = Some(DraftColor::Back);
            }
            SliderField::new(tl!("Transparency"), Bind::bound(transparency), 0.0..=100.0).unit_kind(UnitKind::Percent).show(ui);
        }
    }
    if let Some(field) = more {
        open_more_colors(app, field, field.current(&app.background.fill));
    }
    if picking {
        app.pick_background_picture();
    }

    ui.add_space(8.0);
    preview(app, ui);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if Button::new(tl!("Apply").to_string()).show(ui).clicked() {
            let params = app.background.params();
            if app.run("design.pageFill", params).is_ok() {
                app.background = Draft::from_doc(&app.session.doc);
                app.background.doc = app.session.document_id();
            }
        }
        if Button::new(tl!("Reset").to_string()).show(ui).clicked() {
            app.background = Draft::from_doc(&app.session.doc);
            app.background.doc = app.session.document_id();
        }
        if Button::new(tl!("Close").to_string()).show(ui).clicked() {
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

/// One colour field of the pane: where More Colors… puts its answer.
#[derive(Clone, Copy, Debug)]
enum DraftColor {
    Solid,
    Start,
    End,
    Fore,
    Back,
}

impl DraftColor {
    /// The dialog key that names the field, and what it maps back to.
    fn key(self) -> &'static str {
        match self {
            Self::Solid => "formatBackground.solid",
            Self::Start => "formatBackground.start",
            Self::End => "formatBackground.end",
            Self::Fore => "formatBackground.fore",
            Self::Back => "formatBackground.back",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        [Self::Solid, Self::Start, Self::End, Self::Fore, Self::Back].into_iter().find(|f| f.key() == key)
    }

    /// The colour the field holds now, if the draft is still the fill it belonged to.
    fn current(self, fill: &DraftFill) -> Rgb {
        match (self, fill) {
            (Self::Solid, DraftFill::Solid { color }) => *color,
            (Self::Start, DraftFill::Gradient { start, .. }) => *start,
            (Self::End, DraftFill::Gradient { end, .. }) => *end,
            (Self::Fore, DraftFill::Pattern { fore, .. }) => *fore,
            (Self::Back, DraftFill::Pattern { back, .. }) => *back,
            _ => Rgb::WHITE,
        }
    }

    /// Puts the answer in the draft, if it is still the fill the field belonged to.
    fn set(self, fill: &mut DraftFill, c: Rgb) {
        match (self, fill) {
            (Self::Solid, DraftFill::Solid { color }) => *color = c,
            (Self::Start, DraftFill::Gradient { start, .. }) => *start = c,
            (Self::End, DraftFill::Gradient { end, .. }) => *end = c,
            (Self::Fore, DraftFill::Pattern { fore, .. }) => *fore = c,
            (Self::Back, DraftFill::Pattern { back, .. }) => *back = c,
            _ => {}
        }
    }
}

/// What a pane colour field did in this pass.
struct ColorButtonOut {
    /// The colour picked from the palette.
    picked: Option<Rgb>,
    /// More Colors… was chosen: the pane opens the Colors dialog for the field.
    more: bool,
}

/// The pane's colour field: a swatch and drop-down holding the theme colours and More Colors….
fn color_button(ui: &mut Ui, palette: &Palette, label: &str, c: Rgb) -> ColorButtonOut {
    let mut value = c.hex();
    let out = color_field(ui, &Label::from(label.to_string()), &mut value, &ColorFieldEmpty::NoColor, palette);
    if out.more_colors {
        return ColorButtonOut { picked: None, more: true };
    }
    ColorButtonOut { picked: out.changed.then(|| Rgb::parse(&value)).flatten().filter(|n| *n != c), more: false }
}

/// Opens the Colors dialog for a pane colour field; its answer goes into the draft.
fn open_more_colors(app: &mut WordApp, field: DraftColor, current: Rgb) {
    let dialog = crate::colors::more_colors(Some(current), field.key().to_string(), Default::default());
    crate::dialogs::host::open_with(app, None, dialog, DialogHooks { prepare: None, settle: settle_more_colors });
}

/// The pane's More Colors… answer, into the draft field it was opened for; true once the dialog is gone.
fn settle_more_colors(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    match app.dialog_host.take_by_id::<ColorPickerDialog>(id) {
        Some(DialogResult::Ok(result)) => {
            let c = result.picked.color;
            let channel = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
            if let Some(field) = DraftColor::from_key(&result.key) {
                field.set(&mut app.background.fill, Rgb(channel(c.r), channel(c.g), channel(c.b)));
            }
            Settle::Done
        }
        Some(_) => Settle::Done,
        None if app.dialog_host.get_by_id::<ColorPickerDialog>(id).is_none() => Settle::Done,
        None => Settle::Keep,
    }
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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, name));
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
        DraftFill::Gradient { start, end, angle, transparency, stops } => {
            // The preview draws the same stops Apply sends: endpoint pickers over the document's
            // own stops, not just the two endpoints.
            paint_gradient(p, rect, &Draft::gradient_stops(stops, *start, *end, *transparency), *angle);
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

/// Paints a linear gradient over `rect`: `stops` (position 0–100, colour, transparency) run at
/// `angle` (0° left to right, 90° top to bottom, clockwise), each stop's colour composited over the
/// white page at its transparency. Bands perpendicular to the gradient are drawn past `rect` and
/// clipped to it, so the preview matches the page's own shading.
fn paint_gradient(p: &egui::Painter, rect: Rect, stops: &[GradStop], angle: f32) {
    let Some(first) = stops.first() else { return };
    if !(angle.is_finite() && rect.width() > 0.0 && rect.height() > 0.0) {
        p.rect_filled(rect, 0.0, over_paper(*first));
        return;
    }
    let a = angle.to_radians();
    let d = vec2(a.cos(), a.sin());
    let n = vec2(-d.y, d.x);
    let c = rect.center();
    let proj = |q: Pos2| (q.x - c.x) * d.x + (q.y - c.y) * d.y;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for q in [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()] {
        let t = proj(q);
        lo = lo.min(t);
        hi = hi.max(t);
    }
    let span = hi - lo;
    if !(span.is_finite() && span > 0.5) {
        p.rect_filled(rect, 0.0, over_paper(*first));
        return;
    }
    let reach = rect.width().hypot(rect.height());
    let steps = 64;
    let colour_at = |t: f32| {
        let at = ((t - lo) / span).clamp(0.0, 1.0) * 100.0;
        let (mut left, mut right) = (stops[0], *stops.last().unwrap_or(&stops[0]));
        for pair in stops.windows(2) {
            if pair[0].pos <= at && at <= pair[1].pos {
                (left, right) = (pair[0], pair[1]);
                break;
            }
        }
        let f = if right.pos > left.pos { (at - left.pos) / (right.pos - left.pos) } else { 1.0 };
        let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f).round().clamp(0.0, 255.0) as u8;
        let alpha = (1.0 - (left.transparency + (right.transparency - left.transparency) * f).clamp(0.0, 100.0) / 100.0).clamp(0.0, 1.0);
        let mix = |over: u8, under: u8| (f32::from(over) * alpha + f32::from(under) * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8;
        let (r, g, b) = (lerp(left.color.0, right.color.0), lerp(left.color.1, right.color.1), lerp(left.color.2, right.color.2));
        Color32::from_rgb(mix(r, 255), mix(g, 255), mix(b, 255))
    };
    let mut mesh = egui::Mesh::default();
    for k in 0..=steps {
        let t = lo + span * k as f32 / steps as f32;
        let colour = colour_at(t);
        let centre = c + d * t;
        mesh.colored_vertex(centre + n * reach, colour);
        mesh.colored_vertex(centre - n * reach, colour);
    }
    for k in 0..steps {
        let (a, b, d, e) = (k * 2, k * 2 + 1, k * 2 + 2, k * 2 + 3);
        mesh.add_triangle(a, b, d);
        mesh.add_triangle(b, d, e);
    }
    p.with_clip_rect(rect.intersect(p.clip_rect())).add(egui::Shape::mesh(mesh));
}

/// A gradient stop's colour as the white page shows it at its transparency.
fn over_paper(stop: GradStop) -> Color32 {
    let alpha = (1.0 - stop.transparency.clamp(0.0, 100.0) / 100.0).clamp(0.0, 1.0);
    let mix = |over: u8| (f32::from(over) * alpha + 255.0 * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(mix(stop.color.0), mix(stop.color.1), mix(stop.color.2))
}

/// The cache key of the chosen picture: its path, a fingerprint of its `data`, or its media key.
/// The data fingerprint hashes the whole string: sampled windows can collide, and two pictures of
/// one length would then share a cached texture (or a cached failure). The media fallback is
/// scoped to the open document and the media's byte length, so a key reused by another document
/// (or replaced bytes under one key) can't serve the wrong cached texture.
pub(crate) fn picture_cache_key(
    path: &Option<String>,
    data: &Option<String>,
    media: &Option<String>,
    doc_id: u64,
    media_len: impl Fn(&str) -> usize,
) -> Option<String> {
    if let Some(path) = path {
        return Some(path.clone());
    }
    if let Some(d) = data {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        d.hash(&mut h);
        return Some(format!("data:{}:{:016x}", d.len(), h.finish()));
    }
    media.as_ref().map(|m| format!("media:{doc_id}:{m}:{}", media_len(m)))
}

/// The chosen picture's thumbnail, its texture cached by path, data or media key; a dashed frame
/// when none is chosen or it can't be read.
fn paint_picture_preview(app: &WordApp, ui: &Ui, p: &egui::Painter, rect: Rect) {
    let DraftFill::Picture { path, media, data, data_key, tile, transparency } = &app.background.fill else {
        return;
    };
    // `data_key` was hashed when the picture arrived; hashing the whole base64 string here would
    // run on every repaint. The media fallback carries the document and the media's byte length,
    // so a key reused by another document can't serve the wrong cached texture.
    let doc_id = app.session.document_id();
    let key = path
        .clone()
        .or_else(|| data_key.clone())
        .or_else(|| media.as_ref().map(|m| format!("media:{doc_id}:{m}:{}", app.session.doc.media.get(m).map_or(0, |b| b.len()))));
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
                // Past the thumbnail limit the pane shows a dashed frame, not the picture:
                // a 2 GiB selection is refused after a 64 MiB capped read, not held whole.
                let bytes = if let Some(path) = path {
                    read_preview_file(path)
                } else if let Some(media) = media {
                    app.session.doc.media.get(media).map(|b| b.to_vec())
                } else {
                    data.as_deref()
                        .and_then(|d| wordcraft_engine::cmd::insert::base64_decode_bounded(d, MAX_PREVIEW_BYTES as usize)?.or(Some(Vec::new())))
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

/// The bytes of the preview picture picked by path: a capped file read on native, where files
/// open from paths; `None` on the web, where files arrive through the browser as `data`/`media`.
#[cfg(not(target_arch = "wasm32"))]
fn read_preview_file(path: &str) -> Option<Vec<u8>> {
    wordcraft_engine::io::read_file_capped(path, MAX_PREVIEW_BYTES, "image").ok()
}

/// The bytes of the preview picture picked by path: a capped file read on native, where files
/// open from paths; `None` on the web, where files arrive through the browser as `data`/`media`.
#[cfg(target_arch = "wasm32")]
#[allow(clippy::needless_pass_by_value)]
fn read_preview_file(_path: &str) -> Option<Vec<u8>> {
    None
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

    /// A new picture pick drops the cached thumbnail for its path — including a cached failed
    /// load — so the pane reloads the file instead of repainting the stale entry.
    #[test]
    fn a_new_picture_pick_drops_the_stale_preview_cache_entry() {
        let key = "picked.png".to_string();
        let tex_id = Id::unique(("formatBackground.picture", &key));
        // A fill pointing at a missing file caches its failed load. The write lands at the end
        // of one pass and is visible from the next, as the pane's own repaints do.
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("x")), Default::default());
        app.background.fill = DraftFill::Picture { path: Some(key.clone()), media: None, data: None, data_key: None, tile: false, transparency: 0.0 };
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_picture_preview(&app, ui, ui.painter(), Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 30.0)));
        })
        .drop_without_applying_deltas();
        ctx.run_ui(egui::RawInput::default(), |_| {
            assert!(matches!(ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(tex_id)), Some(None)), "the failed load is cached");
        })
        .drop_without_applying_deltas();
        // A new pick of the same path clears the entry, so the next repaint reloads the file.
        crate::file_dialogs::clear_background_preview_cache(&ctx, &key);
        assert!(ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(tex_id)).is_none(), "the stale entry is gone");
    }

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
        let doc_id = a.session.document_id();
        let good_len = a.session.doc.media.get("good.png").map_or(0, |b| b.len());
        let junk_len = a.session.doc.media.get("junk.png").map_or(0, |b| b.len());
        let good = format!("media:{doc_id}:good.png:{good_len}");
        let junk = format!("media:{doc_id}:junk.png:{junk_len}");
        a.background.fill =
            DraftFill::Picture { path: None, media: Some("good.png".into()), data: None, data_key: None, tile: false, transparency: 0.0 };
        frame(&ctx, &a);
        assert!(matches!(entry(&ctx, &good), Some(Some(_))), "the decoded picture is cached");
        // An unreadable one caches its failure, so it isn't decoded every frame.
        a.background.fill =
            DraftFill::Picture { path: None, media: Some("junk.png".into()), data: None, data_key: None, tile: false, transparency: 0.0 };
        frame(&ctx, &a);
        assert!(matches!(entry(&ctx, &junk), Some(None)), "the failed load is cached");
        frame(&ctx, &a);
        assert!(matches!(entry(&ctx, &junk), Some(None)), "and still is");
        // The picture before it is released.
        assert!(entry(&ctx, &good).is_none(), "the previous picture's texture is gone");
    }

    /// A data picture's key is hashed once, when the picker hands it over: the preview uses the
    /// stored key, not a fresh hash of the whole base64 string on every repaint.
    #[test]
    fn picture_data_uses_its_stored_key() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("x")), Default::default());
        a.background.fill = DraftFill::Picture {
            path: None,
            media: None,
            data: Some("bm90IGFuIGltYWdl".into()), // "not an image": the load fails and caches that
            data_key: Some("data:12:sentinel".into()),
            tile: false,
            transparency: 0.0,
        };
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_picture_preview(&a, ui, ui.painter(), Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 30.0)));
        })
        .drop_without_applying_deltas();
        let entry = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(Id::unique(("formatBackground.picture", "data:12:sentinel"))));
        assert!(matches!(entry, Some(None)), "the stored key is the one cached");
    }
}
