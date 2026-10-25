//! Colour menus and the Colors dialog (More Colors…, #606).
//!
//! Every colour drop-down (Font Color, Shading, Page Color, Shape Fill/Outline, pen colours, and
//! the colour fields inside dialogs) shows the theme and standard colours, a Recent Colors row and
//! a More Colors… item. More Colors… opens the Colors dialog: a Standard tab (a honeycomb of
//! colours and a row of greys) and a Custom tab (a hue × saturation field with a lightness
//! slider, RGB or HSL numbers and a hex box). Everything is drawn in code.
//!
//! What a pick does is a [`Target`]: run a command with the colour put into its params, or hand
//! the colour back to a dialog's field. The colour a command takes is any `RRGGBB`, so a custom
//! colour needs nothing new below the UI.

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_doc::Rgb;

use crate::WordApp;
use crate::theme::{Tokens, c32};

/// How many colours Recent Colors keeps.
pub const RECENT_MAX: usize = 10;

// ---------------------------------------------------------------------------------------------
// Pure helpers

/// A colour typed by a person: `RRGGBB`, `#RRGGBB`, `#RGB`, `0xRRGGBB`, `rgb(r, g, b)` or three
/// numbers 0–255 separated by commas or spaces. Anything else is `None`.
pub fn parse(s: &str) -> Option<Rgb> {
    let s = s.trim();
    let hex = s.trim_start_matches('#');
    let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    if hex.len() == 6 {
        if let Some(c) = Rgb::parse(hex) {
            return Some(c);
        }
    } else if hex.len() == 3 && hex.is_ascii() {
        let d = |i: usize| hex.get(i..i + 1).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|v| v * 17);
        if let (Some(r), Some(g), Some(b)) = (d(0), d(1), d(2)) {
            return Some(Rgb(r, g, b));
        }
    }
    let lower = s.to_ascii_lowercase();
    let inner = lower.strip_prefix("rgb").map(|r| r.trim().trim_start_matches('(').trim_end_matches(')')).unwrap_or(&lower);
    let parts: Vec<&str> = inner.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    if parts.len() != 3 {
        return None;
    }
    let n = |i: usize| parts.get(i).and_then(|p| p.parse::<u8>().ok());
    Some(Rgb(n(0)?, n(1)?, n(2)?))
}

/// Hue (degrees, 0–360), saturation and lightness (0–1) of a colour.
pub fn to_hsl(c: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    ((h * 60.0).rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
}

/// The colour with hue `h` (degrees, any value wraps), saturation `s` and lightness `l` (0–1,
/// clamped). Non-finite input counts as 0.
pub fn from_hsl(h: f32, s: f32, l: f32) -> Rgb {
    let fin = |v: f32| if v.is_finite() { v } else { 0.0 };
    let h = fin(h).rem_euclid(360.0);
    let s = fin(s).clamp(0.0, 1.0);
    let l = fin(l).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let q = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb(q(r), q(g), q(b))
}

/// Puts `hex` first in a Recent Colors list (no repeats, at most [`RECENT_MAX`]). Junk is ignored.
pub fn push_recent(list: &mut Vec<String>, hex: &str) {
    let Some(c) = parse(hex) else { return };
    let hex = c.hex();
    list.retain(|h| parse(h).map(Rgb::hex).as_deref() != Some(hex.as_str()));
    list.insert(0, hex);
    list.truncate(RECENT_MAX);
}

/// The Recent Colors to show from a saved list: valid colours only, no repeats, at most
/// [`RECENT_MAX`].
pub fn recent(list: &[String]) -> Vec<Rgb> {
    let mut out: Vec<Rgb> = Vec::new();
    for c in list.iter().filter_map(|h| parse(h)) {
        if !out.contains(&c) {
            out.push(c);
        }
        if out.len() >= RECENT_MAX {
            break;
        }
    }
    out
}

/// Rings of the Standard tab's honeycomb around its white centre.
pub const HONEYCOMB_RINGS: i32 = 6;

/// The Standard tab's honeycomb: axial cell coordinates `(q, r)` and their colours. The centre is
/// white; going out, colours get more saturated and then darker; the hue follows the angle.
pub fn honeycomb() -> Vec<(i32, i32, Rgb)> {
    let n = HONEYCOMB_RINGS;
    let mut cells = Vec::new();
    for r in -n..=n {
        for q in -n..=n {
            let d = q.abs().max(r.abs()).max((q + r).abs());
            if d > n {
                continue;
            }
            let (x, y) = cell_xy(q, r);
            let hue = (-y).atan2(x).to_degrees();
            let t = d as f32 / n as f32;
            cells.push((q, r, from_hsl(hue, 1.0, 1.0 - 0.75 * t)));
        }
    }
    cells
}

/// The greys under the honeycomb, white to black.
pub fn greys() -> Vec<Rgb> {
    (0..=14u32)
        .map(|i| {
            let v = (255 - (i * 255 / 14).min(255)) as u8;
            Rgb(v, v, v)
        })
        .collect()
}

/// A cell's centre in units of the cell radius (pointy-top hexagons).
fn cell_xy(q: i32, r: i32) -> (f32, f32) {
    (3f32.sqrt() * (q as f32 + r as f32 / 2.0), 1.5 * r as f32)
}

// ---------------------------------------------------------------------------------------------
// Targets

/// What a menu remembers as the split button's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remember {
    Nothing,
    FontColor,
    Shading,
}

/// Where a picked colour goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Run `cmd` with the colour (`RRGGBB`) put into `params` at the JSON pointer `at`.
    Run { cmd: String, params: Value, at: String, remember: Remember },
    /// A dialog's colour field (see [`field_menu`]).
    Field(Id),
}

impl Target {
    /// `cmd` with params `{"color": RRGGBB}`.
    pub fn cmd(cmd: &str) -> Target {
        Target::Run { cmd: cmd.into(), params: json!({"color": null}), at: "/color".into(), remember: Remember::Nothing }
    }

    /// `cmd` with the colour at the JSON pointer `at` inside `params`.
    pub fn at(cmd: &str, params: Value, at: &str) -> Target {
        Target::Run { cmd: cmd.into(), params, at: at.into(), remember: Remember::Nothing }
    }

    pub fn remember(self, r: Remember) -> Target {
        match self {
            Target::Run { cmd, params, at, .. } => Target::Run { cmd, params, at, remember: r },
            t => t,
        }
    }

    /// The command and params a pick of `hex` runs (`None` for a field).
    pub fn command(&self, hex: &str) -> Option<(String, Value)> {
        let Target::Run { cmd, params, at, .. } = self else { return None };
        let mut p = params.clone();
        match p.pointer_mut(at) {
            Some(slot) => *slot = json!(hex),
            None => p["color"] = json!(hex),
        }
        Some((cmd.clone(), p))
    }
}

/// Apply a picked colour to its target.
pub fn apply(app: &mut WordApp, ctx: &egui::Context, target: &Target, hex: &str) {
    match target {
        Target::Run { remember, .. } => {
            match remember {
                Remember::FontColor => app.canvas.last_font_color = hex.to_string(),
                Remember::Shading => app.canvas.last_shading = hex.to_string(),
                Remember::Nothing => {}
            }
            if let Some((cmd, p)) = target.command(hex) {
                let _ = app.run(&cmd, p);
            }
        }
        Target::Field(id) => {
            ctx.data_mut(|d| d.insert_temp(*id, FieldResult(hex.to_string())));
            ctx.request_repaint();
        }
    }
}

/// The colour a target has now (the dialog's Current swatch).
fn current_of(app: &mut WordApp, target: &Target) -> Option<Rgb> {
    let Target::Run { cmd, .. } = target else { return None };
    match cmd.as_str() {
        "format.color" => {
            let st = app.session.run("format.state", &json!({})).ok()?;
            st.get("color").and_then(Value::as_str).and_then(parse)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The drop-down

/// A choice in a colour drop-down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Color(String),
    More,
}

/// Stable storage id for the Recent Colors list.
fn recent_id() -> Id {
    Id::unique("wordcraft.recentColors")
}

/// Stable storage id for the pending More Colors request.
fn request_id() -> Id {
    Id::unique("wordcraft.moreColorsRequest")
}

/// Stable storage id for the Colors dialog's open flag.
fn open_flag_id() -> Id {
    Id::unique("wordcraft.colorsDialogOpen")
}

/// Removes and returns a temporary value.
fn take<T: Clone + Send + Sync + 'static>(d: &mut egui::util::IdTypeMap, id: Id) -> Option<T> {
    let v = d.get_temp::<T>(id);
    d.remove::<T>(id);
    v
}

#[derive(Clone)]
struct FieldRequest(Id, Option<Rgb>);

#[derive(Clone)]
struct FieldResult(String);

/// The drop-down's body: theme and standard colours, Recent Colors, More Colors….
pub fn palette(ui: &mut Ui, theme: &[Rgb]) -> Option<Pick> {
    let mut picked = crate::widgets::color_grid(ui, theme).map(Pick::Color);
    let recent: Vec<String> = ui.data(|d| d.get_temp::<Vec<String>>(recent_id())).unwrap_or_default();
    let recent = self::recent(&recent);
    if !recent.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!("Recent Colors")).small().weak());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
            for c in recent {
                if crate::widgets::swatch(ui, c) {
                    picked = Some(Pick::Color(c.hex()));
                }
            }
        });
    }
    ui.add_space(2.0);
    ui.separator();
    if more_item(ui).clicked() {
        picked = Some(Pick::More);
    }
    picked
}

/// The More Colors… row: a small rainbow wheel and the label.
fn more_item(ui: &mut Ui) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(180.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 2.0, t.hover);
    }
    let c = pos2(r.min.x + 10.0, r.center().y);
    for i in 0..12 {
        let a0 = (i as f32) * std::f32::consts::TAU / 12.0;
        let a1 = a0 + std::f32::consts::TAU / 12.0;
        let p = |a: f32| c + vec2(a.cos(), a.sin()) * 6.5;
        let col = c32(from_hsl(i as f32 * 30.0, 1.0, 0.5));
        ui.painter().add(egui::Shape::convex_polygon(vec![c, p(a0), p(a1)], col, Stroke::NONE));
    }
    ui.painter().circle_stroke(c, 6.5, Stroke::new(0.5, t.border_strong));
    ui.painter().text(
        pos2(r.min.x + 22.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        tl!("More Colors…"),
        egui::TextStyle::Button.resolve(ui.style()),
        t.text,
    );
    resp
}

/// A command colour drop-down's body: picks run `target`, More Colors… opens the Colors dialog.
pub fn menu(ui: &mut Ui, app: &mut WordApp, theme: &[Rgb], target: Target) {
    match palette(ui, theme) {
        Some(Pick::Color(hex)) => {
            let ctx = ui.ctx().clone();
            apply(app, &ctx, &target, &hex);
            ui.close();
        }
        Some(Pick::More) => {
            let current = current_of(app, &target);
            app.colors = Some(ColorsDialog::new(current, target));
            ui.close();
        }
        None => {}
    }
}

/// A dialog field's drop-down body (the dialog has no access to the app): a pick sets `value`;
/// More Colors… asks for the Colors dialog, whose answer [`take_field`] collects. `id` must be
/// the same each frame.
pub fn field_menu(ui: &mut Ui, theme: &[Rgb], id: Id, value: &mut String) {
    match palette(ui, theme) {
        Some(Pick::Color(hex)) => {
            *value = hex;
            ui.close();
        }
        Some(Pick::More) => {
            let current = parse(value);
            ui.data_mut(|d| d.insert_temp(request_id(), FieldRequest(id, current)));
            ui.close();
        }
        None => {}
    }
}

/// The Colors dialog's answer for the field `id`, if it just closed with OK.
pub fn take_field(ctx: &egui::Context, id: Id, value: &mut String) {
    if let Some(FieldResult(hex)) = ctx.data_mut(|d| take::<FieldResult>(d, id)) {
        *value = hex;
    }
}

/// Whether the Colors dialog was open last frame: dialogs under it leave Enter and Escape to it.
pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(open_flag_id())).unwrap_or(false)
}

// ---------------------------------------------------------------------------------------------
// The Colors dialog

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorsTab {
    Standard,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Rgb,
    Hsl,
}

/// The Colors dialog's state.
#[derive(Clone, Debug)]
pub struct ColorsDialog {
    pub tab: ColorsTab,
    /// The new colour.
    pub color: Rgb,
    /// Hue, saturation, lightness of `color`; the hue is kept through greys so dragging the
    /// lightness back out of black or white returns to it.
    pub hsl: (f32, f32, f32),
    /// The colour the target had when the dialog opened.
    pub current: Option<Rgb>,
    pub model: Model,
    /// The hex box's text (may be mid-edit).
    pub hex: String,
    pub target: Target,
}

impl ColorsDialog {
    pub fn new(current: Option<Rgb>, target: Target) -> ColorsDialog {
        let c = current.unwrap_or(Rgb::BLACK);
        let mut d = ColorsDialog { tab: ColorsTab::Standard, color: c, hsl: (0.0, 0.0, 0.0), current, model: Model::Rgb, hex: String::new(), target };
        d.set_rgb(c);
        d
    }

    pub fn set_rgb(&mut self, c: Rgb) {
        let (h, s, l) = to_hsl(c);
        let h = if s > 0.0 && l > 0.0 && l < 1.0 { h } else { self.hsl.0 };
        self.color = c;
        self.hsl = (h, s, l);
        self.hex = c.hex();
    }

    pub fn set_hsl(&mut self, h: f32, s: f32, l: f32) {
        self.hsl = (h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        self.color = from_hsl(self.hsl.0, self.hsl.1, self.hsl.2);
        self.hex = self.color.hex();
    }

    /// Sets the colour from typed text; returns whether it was a colour.
    pub fn set_text(&mut self, s: &str) -> bool {
        match parse(s) {
            Some(c) => {
                let (h, sat, l) = to_hsl(c);
                let h = if sat > 0.0 { h } else { self.hsl.0 };
                self.color = c;
                self.hsl = (h, sat, l);
                true
            }
            None => false,
        }
    }
}

/// Draws the Colors dialog if it is open (and opens it for a dialog field that asked).
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(recent_id(), app.ui.recent_colors.clone()));
    if let Some(FieldRequest(id, current)) = ctx.data_mut(|d| take::<FieldRequest>(d, request_id())) {
        app.colors = Some(ColorsDialog::new(current, Target::Field(id)));
    }
    let Some(mut d) = app.colors.take() else {
        ctx.data_mut(|m| m.insert_temp(open_flag_id(), false));
        return;
    };
    let resp = egui::Modal::new(Id::unique("wordcraft.colorsDialog")).show(ctx, |ui| body(ui, &mut d));
    let mut done = resp.inner;
    if done.is_none() && resp.should_close() {
        done = Some(false);
    }
    match done {
        Some(ok) => {
            ctx.data_mut(|m| m.insert_temp(open_flag_id(), false));
            if ok {
                finish(app, ctx, &d);
            }
            if matches!(d.target, Target::Run { .. }) && app.dialog.is_none() {
                app.canvas.want_focus = true;
            }
        }
        None => {
            ctx.data_mut(|m| m.insert_temp(open_flag_id(), true));
            app.colors = Some(d);
        }
    }
}

/// OK: the colour joins Recent Colors and goes to the target.
pub fn finish(app: &mut WordApp, ctx: &egui::Context, d: &ColorsDialog) {
    let hex = d.color.hex();
    push_recent(&mut app.ui.recent_colors, &hex);
    ctx.data_mut(|m| m.insert_temp(recent_id(), app.ui.recent_colors.clone()));
    apply(app, ctx, &d.target, &hex);
}

/// The dialog's contents. `Some(true)` for OK, `Some(false)` for Cancel.
fn body(ui: &mut Ui, d: &mut ColorsDialog) -> Option<bool> {
    let t = Tokens::get(ui.ctx());
    ui.set_width(430.0);
    ui.label(egui::RichText::new(tl!("Colors")).font(crate::theme::semibold(14.0)));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for (tab, label) in [(ColorsTab::Standard, "Standard"), (ColorsTab::Custom, "Custom")] {
            if ui.selectable_label(d.tab == tab, tl!(label)).clicked() {
                d.tab = tab;
            }
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(330.0);
            match d.tab {
                ColorsTab::Standard => standard(ui, d),
                ColorsTab::Custom => custom(ui, d),
            }
        });
        ui.vertical(|ui| {
            ui.add_space(110.0);
            preview(ui, d, &t);
        });
    });
    let mut out = None;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (okb, cancelb) = crate::dialogs::ok_cancel_responses(ui, |ui| ui.add(crate::widgets::primary_button(ui, tl!("OK"))), tl!("Cancel"));
            if cancelb.clicked() {
                out = Some(false);
            }
            let typing = ui.ctx().memory(|m| m.focused().is_some());
            if okb.clicked() || (!typing && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                out = Some(true);
            }
        });
    });
    out
}

/// New over Current, as two halves of one swatch.
fn preview(ui: &mut Ui, d: &ColorsDialog, t: &Tokens) {
    ui.label(egui::RichText::new(tl!("New")).small());
    let (r, _) = ui.allocate_exact_size(vec2(64.0, 72.0), Sense::hover());
    let top = Rect::from_min_max(r.min, pos2(r.max.x, r.center().y));
    let bottom = Rect::from_min_max(pos2(r.min.x, r.center().y), r.max);
    ui.painter().rect_filled(top, 0.0, c32(d.color));
    match d.current {
        Some(c) => {
            ui.painter().rect_filled(bottom, 0.0, c32(c));
        }
        None => {
            ui.painter().rect_filled(bottom, 0.0, t.panel);
            ui.painter().line_segment([bottom.left_bottom(), bottom.right_top()], Stroke::new(1.0, t.text_dim));
        }
    }
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    ui.label(egui::RichText::new(tl!("Current")).small());
}

/// Pointy-top hexagon corners around `c` with radius `s`.
fn hexagon(c: Pos2, s: f32) -> Vec<Pos2> {
    (0..6)
        .map(|i| {
            let a = (60.0 * i as f32 - 30.0).to_radians();
            c + vec2(a.cos(), a.sin()) * s
        })
        .collect()
}

fn standard(ui: &mut Ui, d: &mut ColorsDialog) {
    let t = Tokens::get(ui.ctx());
    ui.label(tl!("Colors:"));
    let s = 11.0;
    let n = HONEYCOMB_RINGS as f32;
    let size = vec2(3f32.sqrt() * s * (2.0 * n + 1.0) + 2.0, s * (3.0 * n + 2.0) + 2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), size.y), Sense::click());
    let centre = rect.center();
    let cells = honeycomb();
    let at = |q: i32, r: i32| {
        let (x, y) = cell_xy(q, r);
        centre + vec2(x, y) * s
    };
    let hit = |p: Pos2, centres: Vec<(Pos2, Rgb)>| centres.into_iter().find(|(c, _)| c.distance(p) < s * 0.9).map(|(_, col)| col);
    let mut selected = None;
    for &(q, r, col) in &cells {
        let c = at(q, r);
        ui.painter().add(egui::Shape::convex_polygon(hexagon(c, s), c32(col), Stroke::new(0.5, t.border_strong)));
        if col == d.color {
            selected = Some(c);
        }
    }
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.clicked())
        && let Some(col) = hit(p, cells.iter().map(|&(q, r, col)| (at(q, r), col)).collect())
    {
        d.set_rgb(col);
    }
    // Greys: one row of hexagons under the honeycomb.
    ui.add_space(6.0);
    let greys = greys();
    let w = 3f32.sqrt() * s;
    let (row, gresp) = ui.allocate_exact_size(vec2(ui.available_width(), s * 2.0 + 2.0), Sense::click());
    let x0 = row.center().x - w * (greys.len() as f32 - 1.0) / 2.0;
    let gat = |i: usize| pos2(x0 + w * i as f32, row.center().y);
    for (i, col) in greys.iter().enumerate() {
        let c = gat(i);
        ui.painter().add(egui::Shape::convex_polygon(hexagon(c, s), c32(*col), Stroke::new(0.5, t.border_strong)));
        if *col == d.color {
            selected = Some(c);
        }
    }
    if let Some(p) = gresp.interact_pointer_pos().filter(|_| gresp.clicked())
        && let Some(col) = hit(p, greys.iter().enumerate().map(|(i, col)| (gat(i), *col)).collect())
    {
        d.set_rgb(col);
    }
    if let Some(c) = selected {
        ui.painter().add(egui::Shape::closed_line(hexagon(c, s), Stroke::new(2.5, Color32::WHITE)));
        ui.painter().add(egui::Shape::closed_line(hexagon(c, s + 1.5), Stroke::new(1.0, Color32::BLACK)));
    }
}

fn custom(ui: &mut Ui, d: &mut ColorsDialog) {
    let t = Tokens::get(ui.ctx());
    ui.label(tl!("Colors:"));
    ui.horizontal(|ui| {
        // Hue across, saturation up; drawn at the middle lightness.
        let (field, fresp) = ui.allocate_exact_size(vec2(280.0, 170.0), Sense::click_and_drag());
        let mut mesh = egui::Mesh::default();
        let (cols, rows) = (36u32, 6u32);
        for j in 0..=rows {
            for i in 0..=cols {
                let (u, v) = (i as f32 / cols as f32, j as f32 / rows as f32);
                let p = pos2(field.min.x + u * field.width(), field.min.y + v * field.height());
                mesh.colored_vertex(p, c32(from_hsl(u * 360.0, 1.0 - v, 0.5)));
            }
        }
        for j in 0..rows {
            for i in 0..cols {
                let a = j * (cols + 1) + i;
                mesh.add_triangle(a, a + 1, a + cols + 1);
                mesh.add_triangle(a + 1, a + cols + 2, a + cols + 1);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Outside);
        if let Some(p) = fresp.interact_pointer_pos().filter(|_| fresp.clicked() || fresp.dragged()) {
            let u = ((p.x - field.min.x) / field.width()).clamp(0.0, 1.0);
            let v = ((p.y - field.min.y) / field.height()).clamp(0.0, 1.0);
            let l = if d.hsl.2 <= 0.0 || d.hsl.2 >= 1.0 { 0.5 } else { d.hsl.2 };
            d.set_hsl(u * 359.9, 1.0 - v, l);
        }
        let m = pos2(field.min.x + d.hsl.0 / 360.0 * field.width(), field.min.y + (1.0 - d.hsl.1) * field.height());
        ui.painter().circle_stroke(m, 5.0, Stroke::new(2.0, Color32::BLACK));
        ui.painter().circle_stroke(m, 4.0, Stroke::new(1.0, Color32::WHITE));

        // Lightness: white at the top, black at the bottom, through the picked hue and saturation.
        let (bar, bresp) = ui.allocate_exact_size(vec2(16.0, 170.0), Sense::click_and_drag());
        let mut mesh = egui::Mesh::default();
        let steps = 16u32;
        for k in 0..=steps {
            let v = k as f32 / steps as f32;
            let col = c32(from_hsl(d.hsl.0, d.hsl.1, 1.0 - v));
            let y = bar.min.y + v * bar.height();
            mesh.colored_vertex(pos2(bar.min.x, y), col);
            mesh.colored_vertex(pos2(bar.max.x, y), col);
        }
        for k in 0..steps {
            let a = k * 2;
            mesh.add_triangle(a, a + 1, a + 2);
            mesh.add_triangle(a + 1, a + 3, a + 2);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Outside);
        if let Some(p) = bresp.interact_pointer_pos().filter(|_| bresp.clicked() || bresp.dragged()) {
            let v = ((p.y - bar.min.y) / bar.height()).clamp(0.0, 1.0);
            d.set_hsl(d.hsl.0, d.hsl.1, 1.0 - v);
        }
        let y = bar.min.y + (1.0 - d.hsl.2) * bar.height();
        let tip = pos2(bar.max.x + 2.0, y);
        ui.painter().add(egui::Shape::convex_polygon(vec![tip, tip + vec2(7.0, -5.0), tip + vec2(7.0, 5.0)], t.text, Stroke::NONE));
        ui.add_space(8.0);
    });
    ui.add_space(8.0);
    egui::Grid::new("colors_custom").num_columns(2).spacing(vec2(10.0, 5.0)).show(ui, |ui| {
        ui.label(tl!("Color model:"));
        egui::ComboBox::from_id_salt("colors_model")
            .selected_text(match d.model {
                Model::Rgb => "RGB",
                Model::Hsl => "HSL",
            })
            .width(90.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut d.model, Model::Rgb, "RGB");
                ui.selectable_value(&mut d.model, Model::Hsl, "HSL");
            });
        ui.end_row();
        match d.model {
            Model::Rgb => {
                let mut c = [d.color.0, d.color.1, d.color.2];
                let mut changed = false;
                for (i, label) in ["Red:", "Green:", "Blue:"].into_iter().enumerate() {
                    ui.label(tl!(label));
                    if let Some(v) = c.get_mut(i) {
                        changed |= ui.add(egui::DragValue::new(v).range(0..=255).speed(1.0)).changed();
                    }
                    ui.end_row();
                }
                if changed {
                    d.set_rgb(Rgb(c[0], c[1], c[2]));
                }
            }
            Model::Hsl => {
                // Word-style 0–255 scales for all three.
                let to255 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                let mut v = [to255(d.hsl.0 / 360.0), to255(d.hsl.1), to255(d.hsl.2)];
                let mut changed = false;
                for (i, label) in ["Hue:", "Sat:", "Lum:"].into_iter().enumerate() {
                    ui.label(tl!(label));
                    if let Some(x) = v.get_mut(i) {
                        changed |= ui.add(egui::DragValue::new(x).range(0..=255).speed(1.0)).changed();
                    }
                    ui.end_row();
                }
                if changed {
                    d.set_hsl(v[0] as f32 / 255.0 * 360.0, v[1] as f32 / 255.0, v[2] as f32 / 255.0);
                }
            }
        }
        ui.label(tl!("Hex:"));
        ui.horizontal(|ui| {
            ui.label("#");
            let mut text = d.hex.clone();
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0).char_limit(16));
            if r.changed() {
                d.hex = text.clone();
                d.set_text(&text);
            }
            if r.lost_focus() {
                d.hex = d.color.hex();
            }
        });
        ui.end_row();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_engine::Session;

    #[test]
    fn parse_accepts_hex_short_hex_and_rgb_triples() {
        assert_eq!(parse("1A2B3C"), Some(Rgb(0x1A, 0x2B, 0x3C)));
        assert_eq!(parse("  #1a2b3c "), Some(Rgb(0x1A, 0x2B, 0x3C)));
        assert_eq!(parse("0xFF8800"), Some(Rgb(0xFF, 0x88, 0x00)));
        assert_eq!(parse("#f80"), Some(Rgb(0xFF, 0x88, 0x00)));
        assert_eq!(parse("rgb(12, 34, 56)"), Some(Rgb(12, 34, 56)));
        assert_eq!(parse("12 34 56"), Some(Rgb(12, 34, 56)));
        for junk in ["", "#", "#12345", "GGGGGG", "rgb(1,2)", "256,0,0", "-1,0,0", "ééé", "#ééé", "1,2,3,4", "\u{0}"] {
            assert_eq!(parse(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn hsl_round_trips_every_colour_channel_step() {
        for r in (0..=255u8).step_by(15) {
            for g in (0..=255u8).step_by(17) {
                for b in (0..=255u8).step_by(51) {
                    let c = Rgb(r, g, b);
                    let (h, s, l) = to_hsl(c);
                    assert!((0.0..360.0).contains(&h) && (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&l));
                    assert_eq!(from_hsl(h, s, l), c);
                }
            }
        }
        assert_eq!(from_hsl(0.0, 1.0, 0.5), Rgb(255, 0, 0));
        assert_eq!(from_hsl(120.0, 1.0, 0.5), Rgb(0, 255, 0));
        assert_eq!(from_hsl(240.0, 1.0, 0.5), Rgb(0, 0, 255));
        assert_eq!(from_hsl(-120.0, 1.0, 0.5), Rgb(0, 0, 255), "hue wraps");
        assert_eq!(from_hsl(f32::NAN, f32::INFINITY, -3.0), Rgb(0, 0, 0), "junk input is clamped, never a panic");
    }

    #[test]
    fn recent_colors_dedupe_and_cap() {
        let mut list = vec!["junk".to_string()];
        for i in 0..15u8 {
            push_recent(&mut list, &Rgb(i, i, i).hex());
        }
        push_recent(&mut list, "#0e0e0e");
        push_recent(&mut list, "not a colour");
        assert_eq!(list.len(), RECENT_MAX);
        assert_eq!(list.first().map(String::as_str), Some("0E0E0E"));
        assert_eq!(list.iter().filter(|h| *h == "0E0E0E").count(), 1);
        let saved = vec!["FF0000".to_string(), "ff0000".into(), "nope".into(), "00FF00".into()];
        assert_eq!(recent(&saved), vec![Rgb(255, 0, 0), Rgb(0, 255, 0)]);
    }

    #[test]
    fn honeycomb_is_white_in_the_middle_and_has_every_hue() {
        let cells = honeycomb();
        assert_eq!(cells.len(), 127, "6 rings around the centre");
        assert_eq!(cells.iter().find(|(q, r, _)| (*q, *r) == (0, 0)).map(|c| c.2), Some(Rgb::WHITE));
        let mid: Vec<Rgb> = cells.iter().filter(|(q, r, _)| q.abs().max(r.abs()).max((q + r).abs()) == 3).map(|c| c.2).collect();
        assert!(mid.iter().any(|c| c.0 > 200 && c.1 < 100 && c.2 < 100), "a red");
        assert!(mid.iter().any(|c| c.1 > 200 && c.0 < 100 && c.2 < 100), "a green");
        assert!(mid.iter().any(|c| c.2 > 200 && c.0 < 100 && c.1 < 100), "a blue");
        let g = greys();
        assert_eq!((g.first().copied(), g.last().copied()), (Some(Rgb::WHITE), Some(Rgb::BLACK)));
    }

    /// Assert that colour targets build the expected `format.color` or JSON-path command payload, and
    /// that a field target has no command.
    #[test]
    fn targets_put_the_colour_where_the_command_wants_it() {
        assert_eq!(Target::cmd("format.color").command("1A2B3C"), Some(("format.color".into(), json!({"color": "1A2B3C"}))));
        let glow = Target::at("shape.effects", json!({"glow": {"color": null, "size": 8.0}}), "/glow/color");
        assert_eq!(glow.command("00FF00"), Some(("shape.effects".into(), json!({"glow": {"color": "00FF00", "size": 8.0}}))));
        assert_eq!(Target::Field(Id::unique("x")).command("00FF00"), None);
    }

    #[test]
    fn dialog_keeps_hue_through_grey_and_reads_typed_colours() {
        let mut d = ColorsDialog::new(Some(Rgb(255, 0, 0)), Target::cmd("format.color"));
        d.set_hsl(200.0, 0.0, 0.5);
        assert_eq!(d.hsl.0, 200.0, "grey keeps the hue");
        d.set_hsl(d.hsl.0, 1.0, 0.5);
        assert_eq!(d.color, from_hsl(200.0, 1.0, 0.5));
        assert!(d.set_text("#123456"));
        assert_eq!(d.color, Rgb(0x12, 0x34, 0x56));
        assert!(!d.set_text("#12"), "an unfinished hex keeps the colour");
        assert_eq!(d.color, Rgb(0x12, 0x34, 0x56));
    }

    /// #606: More Colors… for the font colour opens the Colors dialog with the selection's colour
    /// as Current; OK applies the custom colour, remembers it on the split button and puts it
    /// first in Recent Colors.
    #[test]
    fn custom_font_color_applies_and_joins_recent_colors() {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let _ = app.run("document.setText", json!({"text": "Custom colour"}));
        let _ = app.run("select.all", json!({}));
        let _ = app.run("format.color", json!({"color": "112233"}));
        let r = app.run("ui.dialog", json!({"name": "colors"}));
        assert_eq!(r.ok().and_then(|v| v.get("dialog").cloned()), Some(json!("colors")));
        assert!(!crate::mini_toolbar::visible(&app), "the mini toolbar stays out of the Colors dialog's way");
        let mut d = app.colors.take().unwrap_or_else(|| panic!("the Colors dialog opens"));
        assert_eq!(d.current, Some(Rgb(0x11, 0x22, 0x33)));
        assert!(d.set_text("#7A3EC8"));
        let ctx = egui::Context::default();
        finish(&mut app, &ctx, &d);
        let st = app.session.run("format.state", &json!({})).unwrap_or_default();
        assert_eq!(st.get("color"), Some(&json!("7A3EC8")));
        assert_eq!(app.canvas.last_font_color, "7A3EC8");
        assert_eq!(app.ui.recent_colors.first().map(String::as_str), Some("7A3EC8"));
        // A dialog field gets the colour back instead.
        let id = Id::unique("field");
        d.target = Target::Field(id);
        finish(&mut app, &ctx, &d);
        let mut value = String::new();
        take_field(&ctx, id, &mut value);
        assert_eq!(value, "7A3EC8");
    }
}
