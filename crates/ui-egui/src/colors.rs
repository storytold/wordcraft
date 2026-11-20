//! Colour drop-downs and the Colors dialog (More Colors…, #606).
//!
//! Every colour drop-down (Font Color, Shading, Page Color, Shape Fill/Outline, pen colours, and
//! the colour fields inside dialogs) shows the theme colours, a Recent Colors row and a More
//! Colors… row. More Colors… opens the framework's tabbed [`ColorPickerDialog`]: a Standard tab
//! (a honeycomb of colours and a row of greys) and a Custom tab (the compact picker).
//!
//! What a pick does is a [`Target`]: run a command with the colour put into its params, or only
//! report it. The colour a command takes is any `RRGGBB`, so a custom colour needs nothing new
//! below the UI.

use craft::color_ui::ui::{ColorPickerDialog, ColorPickerDialogLayout, ColorPickerResult, ColorPickerTab};
use craft::keymap::CommandRef;
use craft::theme::Srgba;
use craft::ui::buttons::{SplitButton, SplitMenu};
use craft::ui::choice::SwatchTable;
use craft::ui::contract::{BodyCx, Hover, NoValue, Props as _, Show as _, clicked};
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::menu::{MenuItem, MenuItemOutput};
use craft::ui::primitives::{IconMark, IconSpec};
use craft::ui::runtime::env::Label;
use craft::ui::style::DataColor;
use egui::{Color32, Ui};
use serde_json::{Value, json};
use wordcraft_doc::Rgb;

use crate::WordApp;
use crate::dialogs::host::{DialogHooks, Settle};

/// How many colours Recent Colors keeps.
pub const RECENT_MAX: usize = 10;

/// The dialog key More Colors… opens its dialog under, and the key of its entry in
/// `WordApp::more_colors`. One More Colors dialog is open at a time, so one key serves every
/// drop-down.
const MORE_KEY: &str = "wordcraft.moreColors";

/// Cells to a row of the theme colours.
const THEME_PER_ROW: usize = 10;

/// Rings of the Standard tab's honeycomb around its white centre.
const HONEYCOMB_RINGS: i32 = 6;

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

/// The framework colour of a document colour.
fn srgba(c: Rgb) -> Srgba {
    Srgba::rgb(f32::from(c.0) / 255.0, f32::from(c.1) / 255.0, f32::from(c.2) / 255.0)
}

/// The document colour nearest a framework colour.
fn rgb_of(c: Srgba) -> Rgb {
    let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let [r, g, b] = c.to_rgb_array();
    Rgb(q(r), q(g), q(b))
}

fn color32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// The Standard tab's honeycomb: axial cell coordinates `(q, r)` and their colours, rows top first
/// and each row from its start (`r` outer, `q` inner), so a row has 7, 8, … 13, … 8, 7 cells. The
/// centre is white; going out, colours get more saturated and then darker; the hue follows the angle.
fn honeycomb() -> Vec<(i32, i32, Rgb)> {
    let n = HONEYCOMB_RINGS;
    let mut cells = Vec::new();
    for r in -n..=n {
        for q in -n..=n {
            let d = q.abs().max(r.abs()).max((q + r).abs());
            if d > n {
                continue;
            }
            let (x, y) = cell_xy(q, r);
            let hue = (-y).atan2(x).to_degrees().rem_euclid(360.0) / 360.0;
            let t = d as f32 / n as f32;
            let [cr, cg, cb] = craft::color::hsl_to_rgb(hue, 1.0, 1.0 - 0.75 * t);
            cells.push((q, r, rgb_of(Srgba::rgb(cr, cg, cb))));
        }
    }
    cells
}

/// The greys under the honeycomb, white to black.
fn greys() -> Vec<Rgb> {
    (0..=14u32)
        .map(|i| {
            let v = (255 - (i * 255 / 14).min(255)) as u8;
            Rgb(v, v, v)
        })
        .collect()
}

/// A cell's centre in units of the cell radius (pointy-top hexagons); the hue angle comes from it.
fn cell_xy(q: i32, r: i32) -> (f32, f32) {
    (3f32.sqrt() * (q as f32 + r as f32 / 2.0), 1.5 * r as f32)
}

/// The Standard tab's cells and greys, each named by its `#RRGGBB` text.
fn standard_cells() -> (Vec<(Label, Srgba)>, Vec<(Label, Srgba)>) {
    let named = |c: Rgb| (Label::from(format!("#{}", c.hex())), srgba(c));
    (honeycomb().into_iter().map(|(_, _, c)| named(c)).collect(), greys().into_iter().map(named).collect())
}

// ---------------------------------------------------------------------------------------------
// Targets

/// What a menu remembers as the split button's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remember {
    /// Nothing.
    Nothing,
    /// The font colour.
    FontColor,
    /// The shading colour.
    Shading,
}

/// Where a picked colour goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Run `cmd` with the colour (`RRGGBB`) put into `params` at the JSON pointer `at`.
    Run {
        /// The command id.
        cmd: String,
        /// The command's params, with a slot for the colour.
        params: Value,
        /// The JSON pointer of the colour's slot in `params`.
        at: String,
        /// What the pick is remembered as.
        remember: Remember,
    },
    /// Run nothing: the pick is only reported in [`ColorPick::picked`] (a gradient stop's colour).
    Pick,
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

    /// The same target remembering its picks as `r`.
    pub fn remember(self, r: Remember) -> Target {
        match self {
            Target::Run { cmd, params, at, .. } => Target::Run { cmd, params, at, remember: r },
            t => t,
        }
    }

    /// The command and params a pick of `hex` runs (`None` for a target that runs nothing).
    pub fn command(&self, hex: &str) -> Option<(String, Value)> {
        let Target::Run { cmd, params, at, .. } = self else { return None };
        let mut p = params.clone();
        match p.pointer_mut(at) {
            Some(slot) => *slot = json!(hex),
            None => p["color"] = json!(hex),
        }
        Some((cmd.clone(), p))
    }

    /// The command id the target runs.
    fn cmd_id(&self) -> Option<&str> {
        match self {
            Target::Run { cmd, .. } => Some(cmd),
            Target::Pick => None,
        }
    }
}

/// Apply a picked colour to its target.
pub fn apply(app: &mut WordApp, target: &Target, hex: &str) {
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
        Target::Pick => {}
    }
}

/// The colour a target has now (the dialog's Current swatch).
fn current_of(app: &mut WordApp, target: &Target) -> Option<Rgb> {
    let Target::Run { cmd, .. } = target else { return None };
    match cmd.as_str() {
        "format.color" => {
            let st = wordcraft_engine::cmd::format::state(&app.session);
            st.get("color").and_then(Value::as_str).and_then(parse)
        }
        _ => None,
    }
}

/// The colour a split button's main part applies: what its target remembers, else what is in effect.
fn remembered(app: &mut WordApp, target: &Target) -> Option<Rgb> {
    match target {
        Target::Run { remember: Remember::FontColor, .. } => parse(&app.canvas.last_font_color),
        Target::Run { remember: Remember::Shading, .. } => parse(&app.canvas.last_shading),
        _ => current_of(app, target),
    }
}

// ---------------------------------------------------------------------------------------------
// The palette

/// The colours a palette offers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Palette {
    /// The theme colours, in the document's order.
    pub theme: Vec<Rgb>,
    /// Recent Colors, newest first.
    pub recent: Vec<Rgb>,
}

impl Palette {
    /// The document's theme colours and the app's Recent Colors.
    pub fn of(app: &WordApp) -> Palette {
        Palette { theme: app.session.doc.settings.theme_colors.clone(), recent: recent(&app.ui.recent_colors) }
    }
}

/// What a drop-down reports for the pass.
pub struct ColorPick {
    /// The drop-down's response.
    pub response: egui::Response,
    /// The colour picked in this pass.
    pub picked: Option<Rgb>,
    /// What the pointer over a swatch did to the live preview: it shows while the pointer is over a
    /// swatch and ends when it leaves.
    pub preview: craft::behavior::Preview<Rgb>,
    /// The main part's stay of a split button; `Hover::default()` for a body without one.
    pub hover: Hover,
}

/// One swatch table's picks: a click's colour and the pass's preview.
fn swatch_table(
    ui: &mut Ui,
    name: &str,
    colors: &[Rgb],
    per_row: usize,
    current: Option<Rgb>,
) -> (Option<Rgb>, craft::behavior::Preview<Rgb>, egui::Response) {
    use craft::behavior::Preview;
    let swatches: Vec<(Label, Srgba)> = colors.iter().map(|c| (Label::from(format!("#{}", c.hex())), srgba(*c))).collect();
    let out = SwatchTable::new(name.to_string(), &swatches)
        .per_row(per_row)
        .tips(true)
        .marked(|i: &usize| colors.get(*i).copied() == current && current.is_some())
        .show_all(ui);
    let picked = out.selected.and_then(|i| colors.get(i).copied());
    let preview = match out.preview {
        Preview::Shows(i) => colors.get(i).map_or(Preview::Unchanged, |c| Preview::Shows(*c)),
        Preview::Ended => Preview::Ended,
        _ => Preview::Unchanged,
    };
    (picked, preview, out.response)
}

/// Two previews of one pass: a start wins over an end, as the pointer moving from one table to
/// the next ends one and starts the other.
fn merge_preview(a: craft::behavior::Preview<Rgb>, b: craft::behavior::Preview<Rgb>) -> craft::behavior::Preview<Rgb> {
    use craft::behavior::Preview;
    match (a, b) {
        (Preview::Shows(c), _) | (_, Preview::Shows(c)) => Preview::Shows(c),
        (Preview::Ended, _) | (_, Preview::Ended) => Preview::Ended,
        _ => Preview::Unchanged,
    }
}

/// The theme and recent swatches of `palette`; the colour in effect carries the selection mark.
fn swatches(ui: &mut Ui, palette: &Palette, current: Option<Rgb>) -> (Option<Rgb>, craft::behavior::Preview<Rgb>) {
    let (mut picked, mut preview, _) = swatch_table(ui, "Theme Colors", &palette.theme, THEME_PER_ROW, current);
    if !palette.recent.is_empty() {
        let (p, v, _) = swatch_table(ui, "Recent Colors", &palette.recent, RECENT_MAX, current);
        picked = picked.or(p);
        preview = merge_preview(preview, v);
    }
    (picked, preview)
}

/// The More Colors… row.
fn more_row(ui: &mut Ui) -> MenuItemOutput {
    MenuItem::new(tl!("More Colors…").to_string()).show(ui)
}

/// The drop-down's body: theme colours, Recent Colors and More Colors…. A pick runs `target` and
/// is remembered in Recent Colors; More Colors… opens the Colors dialog for `target`. The
/// returned `response` is the More Colors… row's. The body's owner closes the drop-down when
/// `picked` is set or the response is clicked (`BodyCx::request_close`).
pub fn palette(ui: &mut Ui, app: &mut WordApp, target: Target) -> ColorPick {
    let current = current_of(app, &target);
    let theme = Palette::of(app);
    let (picked, preview) = swatches(ui, &theme, current);
    let more = more_row(ui);
    if let Some(c) = picked {
        apply(app, &target, &c.hex());
    }
    if more.ran {
        open_for(app, &target, current, ColorPickerTab::Standard);
    }
    ColorPick { response: more.response, picked, preview, hover: Hover::default() }
}

/// A command colour drop-down's body: [`palette`]; the caller closes the drop-down.
pub fn menu(ui: &mut Ui, app: &mut WordApp, target: Target) -> ColorPick {
    palette(ui, app, target)
}

/// Closes the drop-down through its body after a pick or More Colors….
fn close_after(pick: &ColorPick, cx: &mut BodyCx<'_>) {
    if pick.picked.is_some() || clicked(&pick.response) {
        cx.request_close();
    }
}

// ---------------------------------------------------------------------------------------------
// Split buttons

/// What a split button's menu holds above the palette.
enum Above {
    /// The palette alone.
    Nothing,
    /// An Automatic row that runs `format.color {color: "auto"}`.
    Automatic,
    /// A No Color row that runs the target's own command with `{"color": null}`; it leaves the
    /// remembered colour alone.
    NoColor,
}

/// A split button whose main part runs `main` and whose arrow opens `body`.
fn split_with(
    ui: &mut Ui,
    app: &mut WordApp,
    label: Label,
    icon: IconSpec<'static>,
    main: CommandRef,
    mut body: impl FnMut(&mut Ui, &mut WordApp, &mut BodyCx<'_>) -> ColorPick,
) -> ColorPick {
    let mut inner: Option<(Option<Rgb>, craft::behavior::Preview<Rgb>)> = None;
    // The arrow is its own button, named "<label> options" so it is told from the main part (#3172).
    let arrow_name = crate::i18n::fmt(tl!("{name} options"), &[("name", label.as_str())]);
    let out = SplitButton::<NoValue>::command(
        main,
        SplitMenu::Body(Box::new(|ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let pick = body(ui, app, cx);
            inner = Some((pick.picked, pick.preview));
        })),
    )
    .label(label)
    .arrow_label(arrow_name)
    .icon(icon)
    .show(ui);
    let (picked, preview) = inner.unwrap_or((None, craft::behavior::Preview::Unchanged));
    ColorPick { response: out.response, picked, preview, hover: out.hover }
}

/// The split button of a colour `target`: the icon of its command with a bar of the remembered
/// colour, and the palette under the arrow.
fn color_split(ui: &mut Ui, app: &mut WordApp, label: Label, target: Target, above: Above) -> ColorPick {
    let cmd = target.cmd_id().unwrap_or_default().to_string();
    let colour = remembered(app, &target);
    let mut icon = crate::icons::for_command(&cmd);
    if let Some(c) = colour {
        icon = icon.with_mark(IconMark::bar(DataColor::new(color32(c))));
    }
    let main = match (&target, colour) {
        (Target::Run { .. }, Some(c)) => {
            let (id, params) = target.command(&c.hex()).unwrap_or_else(|| (cmd.clone(), json!({})));
            CommandRef::with_params(id, params)
        }
        _ => CommandRef::new(cmd.clone()),
    };
    split_with(ui, app, label, icon, main, |ui, app, cx| {
        match above {
            Above::Nothing => {}
            Above::Automatic => {
                let auto = CommandRef::with_params("format.color", json!({"color": "auto"}));
                if MenuItem::new(tl!("Automatic").to_string()).command(auto).show(ui).ran {
                    cx.request_close();
                }
            }
            Above::NoColor => {
                let none = CommandRef::with_params(cmd.clone(), json!({"color": null}));
                if MenuItem::new(tl!("No Color").to_string()).command(none).show(ui).ran {
                    cx.request_close();
                }
            }
        }
        let pick = menu(ui, app, target.clone());
        close_after(&pick, cx);
        pick
    })
}

/// Font Color: Automatic, the palette and More Colors…; picks are remembered as the button's colour.
pub fn font_color_split(ui: &mut Ui, app: &mut WordApp, label: impl Into<Label>, target: Target) -> ColorPick {
    color_split(ui, app, label.into(), target, Above::Automatic)
}

/// Shading: its own No Color row, then the palette.
pub fn shading_split(ui: &mut Ui, app: &mut WordApp, label: impl Into<Label>, target: Target) -> ColorPick {
    color_split(ui, app, label.into(), target, Above::NoColor)
}

/// A fill or outline colour split button: the palette alone.
pub fn fill_split(ui: &mut Ui, app: &mut WordApp, label: impl Into<Label>, target: Target) -> ColorPick {
    color_split(ui, app, label.into(), target, Above::Nothing)
}

/// The split-button icon and colour bar for the remembered highlight choice. A remembered name
/// that is not a highlight shows as No Color.
pub(crate) fn highlight_button_appearance(name: &str) -> (&'static str, Option<Color32>, &'static str) {
    let highlight = wordcraft_doc::props::Highlight::from_ooxml(name);
    let icon = if highlight == wordcraft_doc::props::Highlight::None { "highlightNoColor" } else { "highlight" };
    (icon, highlight.rgb().map(color32), highlight.ooxml())
}

/// The highlight drop-down: the highlight colours five to a row, then No Color. A pick, No Color
/// too, becomes the split button's colour, as in Word: after No Color a click on the button
/// removes highlighting (#2105).
fn highlight_body(ui: &mut Ui, app: &mut WordApp, cx: &mut BodyCx<'_>) -> ColorPick {
    use wordcraft_doc::props::Highlight;
    let in_effect = Some(wordcraft_engine::cmd::format::state(&app.session))
        .and_then(|st| st.get("highlight").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "none".into());
    let shown: Vec<(&Highlight, Rgb)> = Highlight::ALL.iter().skip(1).filter_map(|h| h.rgb().map(|c| (h, c))).collect();
    let swatches: Vec<(Label, Srgba)> = shown.iter().map(|(h, c)| (Label::from(tl!(h.name()).to_string()), srgba(*c))).collect();
    let out = SwatchTable::new("Highlight Colors".to_string(), &swatches)
        .per_row(5)
        .tips(true)
        .marked(|i: &usize| shown.get(*i).is_some_and(|(h, _)| h.ooxml() == in_effect))
        .show_all(ui);
    let mut picked = None;
    if let Some((h, c)) = out.selected.and_then(|i| shown.get(i)) {
        app.canvas.last_highlight = h.ooxml().to_string();
        let _ = app.run("format.highlight", json!({"color": h.ooxml()}));
        picked = Some(*c);
        cx.request_close();
    }
    let preview = match out.preview {
        craft::behavior::Preview::Shows(i) => shown.get(i).map_or(craft::behavior::Preview::Unchanged, |(_, c)| craft::behavior::Preview::Shows(*c)),
        craft::behavior::Preview::Ended => craft::behavior::Preview::Ended,
        _ => craft::behavior::Preview::Unchanged,
    };
    let none = CommandRef::with_params("format.highlight", json!({"color": "none"}));
    if MenuItem::new(tl!("No Color").to_string()).command(none).show(ui).ran {
        app.canvas.last_highlight = "none".into();
        cx.request_close();
    }
    ColorPick { response: out.response, picked, preview, hover: Hover::default() }
}

/// Text Highlight Color: the highlight grid under the arrow; the main part applies the remembered
/// highlight.
pub fn highlight_split(ui: &mut Ui, app: &mut WordApp, label: impl Into<Label>) -> ColorPick {
    let (icon_name, bar, ooxml) = highlight_button_appearance(&app.canvas.last_highlight);
    // PLAN: assumed `for_command("format.highlightNoColor")` names the No Color icon (ribbon-w13 icons).
    let mut icon = crate::icons::for_command(if icon_name == "highlightNoColor" { "format.highlightNoColor" } else { "format.highlight" });
    if let Some(c) = bar {
        icon = icon.with_mark(IconMark::bar(DataColor::new(c)));
    }
    let main = CommandRef::with_params("format.highlight", json!({"color": ooxml}));
    split_with(ui, app, label.into(), icon, main, highlight_body)
}

// ---------------------------------------------------------------------------------------------
// More Colors…

/// The Colors dialog: Standard honeycomb and greys, Custom compact picker. `current` is the colour
/// the target has now; `key` comes back in the result.
pub fn more_colors(current: Option<Rgb>, key: String, tab: ColorPickerTab) -> ColorPickerDialog {
    let (cells, greys) = standard_cells();
    ColorPickerDialog::new(key, tl!("Colors").to_string(), current.map_or(Srgba::rgb(0.0, 0.0, 0.0), srgba))
        .layout(ColorPickerDialogLayout::Tabbed)
        .standard(cells, greys)
        .compare(current.is_some())
        .tab(tab)
}

/// Opens the Colors dialog for `target`, remembering where its answer goes.
fn open_for(app: &mut WordApp, target: &Target, current: Option<Rgb>, tab: ColorPickerTab) -> DialogId {
    app.more_colors.insert(MORE_KEY.to_string(), target.clone());
    crate::dialogs::host::open_with(app, None, more_colors(current, MORE_KEY.to_string(), tab), DialogHooks { prepare: None, settle: settle_more })
}

/// The `ui.dialog colors` route: the font colour's More Colors…; `params.tab` is `standard` or
/// `custom`. `None` for any other dialog name.
pub fn open(app: &mut WordApp, name: &str, params: &Value) -> Option<DialogId> {
    if name != "colors" {
        return None;
    }
    let target = Target::cmd("format.color").remember(Remember::FontColor);
    let current = current_of(app, &target);
    let tab = if params.get("tab").and_then(Value::as_str) == Some("custom") { ColorPickerTab::Custom } else { ColorPickerTab::Standard };
    Some(open_for(app, &target, current, tab))
}

/// The settle hook of a More Colors dialog: `Done` once it took the finished result.
fn settle_more(app: &mut WordApp, ctx: &egui::Context, id: DialogId) -> Settle {
    match app.dialog_host.take_by_id::<ColorPickerDialog>(id) {
        Some(DialogResult::Ok(result)) => {
            picked(app, ctx, result);
            Settle::Done
        }
        Some(DialogResult::Cancel) => {
            app.more_colors.remove(MORE_KEY);
            Settle::Done
        }
        Some(_) | None => Settle::Keep,
    }
}

/// OK: the colour joins Recent Colors and goes to the target the dialog was opened for. A key
/// with no entry in `WordApp::more_colors` is not ours (a dialog field answers through its form)
/// and does nothing.
pub fn picked(app: &mut WordApp, ctx: &egui::Context, result: ColorPickerResult) {
    let Some(target) = app.more_colors.remove(&result.key) else { return };
    let hex = rgb_of(result.picked.color).hex();
    push_recent(&mut app.ui.recent_colors, &hex);
    apply(app, &target, &hex);
    if matches!(target, Target::Run { .. }) && app.dialog_host.is_empty() {
        app.canvas.focus_page(ctx);
    }
}

// ---------------------------------------------------------------------------------------------
// Dialog colour fields

/// What a colour field's empty choice is.
#[derive(Clone, Debug)]
pub enum ColorFieldEmpty {
    /// "No Color": the field holds the empty string.
    NoColor,
    /// An automatic choice: `value` is what the field holds, `face` what its swatch shows.
    Automatic {
        /// The row's label.
        label: Label,
        /// What the field holds for it.
        value: &'static str,
        /// What its swatch shows.
        face: AutoFace,
    },
}

/// The face of an Automatic choice's swatch.
#[derive(Clone, Debug)]
pub enum AutoFace {
    /// One colour.
    One(Srgba),
    /// Several colours, side by side (By author).
    Several(Vec<Srgba>),
}

/// What a colour field did in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorFieldResponse {
    /// The value changed.
    pub changed: bool,
    /// More Colors… was chosen: the owner opens [`more_colors`] and puts the answer in the value.
    pub more_colors: bool,
}

/// The one colour field of dialogs and panes: a swatch and drop-down holding the empty choice, the
/// palette and More Colors…. `value` is hex, or the empty choice's value.
pub fn color_field(ui: &mut Ui, label: &Label, value: &mut String, empty: &ColorFieldEmpty, palette: &Palette) -> ColorFieldResponse {
    let mut resp = ColorFieldResponse::default();
    let current = parse(value);
    let (shown, text) = match (current, empty) {
        (Some(c), _) => (Some(srgba(c)), Label::from(c.hex())),
        (None, ColorFieldEmpty::NoColor) => (None, Label::from(tl!("No Color").to_string())),
        // PLAN: framework gap: `IconSpec` has no multi-share swatch face, so `AutoFace::Several` shows its first colour until `IconSource` can paint `SwatchFace` shares (ribbon-w2).
        (None, ColorFieldEmpty::Automatic { label, face, .. }) => (
            match face {
                AutoFace::One(c) => Some(*c),
                AutoFace::Several(v) => v.first().copied(),
            },
            label.clone(),
        ),
    };
    let icon = shown.map_or_else(IconSpec::empty_swatch, IconSpec::swatch);
    let spoken_value = current.map_or_else(|| text.as_str().to_string(), |c| format!("#{}", c.hex()));
    let spoken = format!("{} {spoken_value}", label.as_str());
    let field = SplitButton::<NoValue>::body(text, |ui: &mut Ui, cx: &mut BodyCx<'_>| {
        let empty_label = match empty {
            ColorFieldEmpty::NoColor => Label::from(tl!("No Color").to_string()),
            ColorFieldEmpty::Automatic { label, .. } => label.clone(),
        };
        if MenuItem::new(empty_label).show(ui).ran {
            *value = match empty {
                ColorFieldEmpty::NoColor => String::new(),
                ColorFieldEmpty::Automatic { value, .. } => (*value).to_string(),
            };
            resp.changed = true;
            cx.request_close();
        }
        let (picked, _) = swatches(ui, palette, current);
        if let Some(c) = picked {
            *value = c.hex();
            resp.changed = true;
            cx.request_close();
        }
        if more_row(ui).ran {
            resp.more_colors = true;
            cx.request_close();
        }
    })
    .arrow_label(spoken.clone())
    .icon(icon)
    .show(ui);
    // The framework keeps the short visible value; both parts say the field and its value.
    ui.ctx().accesskit_node_builder(field.response.id, |node| node.set_label(spoken));
    resp
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

    /// Assert that colour targets build the expected `format.color` or JSON-path command payload, and
    /// that a pick-only target has no command.
    #[test]
    fn targets_put_the_colour_where_the_command_wants_it() {
        assert_eq!(Target::cmd("format.color").command("1A2B3C"), Some(("format.color".into(), json!({"color": "1A2B3C"}))));
        let glow = Target::at("shape.effects", json!({"glow": {"color": null, "size": 8.0}}), "/glow/color");
        assert_eq!(glow.command("00FF00"), Some(("shape.effects".into(), json!({"glow": {"color": "00FF00", "size": 8.0}}))));
        assert_eq!(Target::Pick.command("00FF00"), None);
    }

    /// The Standard tab's cells are the generator's, in the order the framework honeycomb places
    /// them: 127 cells in rows of 7…13…7, white in the middle, every hue on the outer ring; the
    /// greys run white to black.
    #[test]
    fn honeycomb_is_white_in_the_middle_and_has_every_hue() {
        let cells = honeycomb();
        assert_eq!(cells.len(), 127);
        let rows: Vec<usize> = (-6..=6).map(|r| cells.iter().filter(|c| c.1 == r).count()).collect();
        assert_eq!(rows, [7, 8, 9, 10, 11, 12, 13, 12, 11, 10, 9, 8, 7]);
        assert!(cells.windows(2).all(|w| (w[0].1, w[0].0) < (w[1].1, w[1].0)), "rows top first, each from its start");
        let centre = cells.iter().find(|c| c.0 == 0 && c.1 == 0).map(|c| c.2);
        assert_eq!(centre, Some(Rgb(255, 255, 255)));
        let sector = |c: Rgb| {
            let (r, g, b) = (f32::from(c.0), f32::from(c.1), f32::from(c.2));
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let d = max - min;
            let hue = if d == 0.0 {
                0.0
            } else if max == r {
                ((g - b) / d).rem_euclid(6.0)
            } else if max == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            };
            (hue.floor() as usize).min(5)
        };
        let mut seen = [false; 6];
        for (q, r, c) in &cells {
            if (*q).abs().max(r.abs()).max((q + r).abs()) == 6 {
                seen[sector(*c)] = true;
            }
        }
        assert_eq!(seen, [true; 6], "every hue sector appears on the outer ring");
        let greys = greys();
        assert_eq!(greys.len(), 15);
        assert_eq!(greys.first(), Some(&Rgb(255, 255, 255)));
        assert_eq!(greys.last(), Some(&Rgb(0, 0, 0)));
        assert!(greys.windows(2).all(|w| w[0].0 > w[1].0));
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
        let id = open(&mut app, "colors", &json!({}));
        assert!(id.is_some(), "the Colors dialog opens");
        let dialog = app.dialog_host.get_mut::<ColorPickerDialog>().unwrap();
        assert_eq!(dialog.opening().map(|p| rgb_of(p.color)), Some(Rgb(0x11, 0x22, 0x33)));
        dialog.set_color(craft::color_ui::ui::PickedColor::rgb(srgba(Rgb(0x7A, 0x3E, 0xC8))));
        let ctx = egui::Context::default();
        assert!(app.dialog_host.confirm::<ColorPickerDialog>(&ctx));
        crate::dialogs::host::settle(&mut app, &ctx);
        let st = app.session.run("format.state", &json!({})).unwrap_or_default();
        assert_eq!(st.get("color"), Some(&json!("7A3EC8")));
        assert_eq!(app.canvas.last_font_color, "7A3EC8");
        assert_eq!(app.ui.recent_colors.first().map(String::as_str), Some("7A3EC8"));
        assert!(app.more_colors.is_empty(), "no entry outlives its dialog");
    }
}
