//! The Font dialog (Home › Font): the character formatting of the selection, and the font of Define
//! New Bullet's bullet. The form is a [`WordForm`]; OK runs one `format.set` with only what the user
//! changed, or hands the form back to Define New Bullet when it was opened from there.

use craft::behavior::ColumnWidth;
use craft::font_manager::picker::FontPickerSettings;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Pad, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::sections::GroupBox;
use craft::ui::style::SurfaceKind;
use craft::ui::text::NumberField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::Rgb;
use wordcraft_doc::props::{CharProps, TextColor, Underline, VertAlign};

use super::dialogs_bullet::DefineBulletForm;
use super::host::{Asks, ChildAnswer, WordForm, open_form};
use crate::WordApp;
use crate::colors::{AutoFace, ColorFieldEmpty, Palette, color_field};
use crate::widgets::font_box::{CatalogRef, font_picker};

/// The width of the sample; fixed, because sizing it from the dialog would grow the dialog frame by frame.
const SAMPLE_WIDTH: f32 = 300.0;
const SAMPLE_HEIGHT: f32 = 56.0;

/// The Font dialog's fields.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct FontForm {
    /// The Latin text font as the box shows it.
    pub font: String,
    /// The Asian text font as the box shows it.
    pub font_ea: String,
    /// Whether the Asian text font box is shown.
    pub asian: bool,
    /// The size in points; `None` leaves it as it is.
    pub size: Option<f64>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub sup: bool,
    pub sub: bool,
    pub small_caps: bool,
    pub caps: bool,
    pub hidden: bool,
    /// `RRGGBB`, `auto`, or empty when the selection's colours differ.
    pub color: String,
    pub spacing: Option<f64>,
    /// Kerning for fonts: `None` when the selection differs, shown neither on nor off.
    pub kern: Option<bool>,
    /// The Points-and-above size of kerning.
    pub kern_at: Option<f64>,
    /// Formatting Define New Bullet's bullet: OK hands the form back and formats no text.
    pub for_bullet: bool,
    /// The fonts as they stood when the dialog opened, to tell a changed font box.
    #[serde(skip)]
    font_was: String,
    /// The properties the fields stood for at opening: OK sends only what the user changed.
    #[serde(skip)]
    start: serde_json::Map<String, Value>,
    /// Shown name to stored (English) name, for the fonts whose two differ.
    #[serde(skip)]
    names: Vec<(String, String)>,
    #[serde(skip)]
    paper: Option<Rgb>,
    #[serde(skip)]
    dark: bool,
    /// The colour field's theme and recent colours, filled by `prepare`.
    // PLAN: assumed `Palette: Clone + Default + PartialEq` (ribbon-w11, dg.A5)
    #[serde(skip)]
    palette: Palette,
    #[serde(skip)]
    catalog: CatalogRef,
    #[serde(skip)]
    settings: FontPickerSettings,
    /// The missing-font notes beside the two boxes, as of opening.
    #[serde(skip)]
    shown_as: Option<String>,
    #[serde(skip)]
    shown_as_ea: Option<String>,
}

/// What opens the Font dialog over another dialog.
pub(crate) struct FontRequest {
    bullet: Option<DefineBulletForm>,
}

impl FontRequest {
    /// Font… of Define New Bullet: the dialog opens on the bullet's font, size, bold, italic and colour.
    pub(crate) fn bullet(form: DefineBulletForm) -> FontRequest {
        FontRequest { bullet: Some(form) }
    }

    /// The form the request opens with.
    // PLAN: dialogs-w13's `Asks::Font` opening calls `req.into_form(app)`: a bullet's form cannot be built without the app.
    pub(crate) fn into_form(self, app: &mut WordApp) -> FontForm {
        let mut form = FontForm::read(app);
        if let Some(bullet) = self.bullet {
            if !bullet.font.is_empty() {
                form.font = app.previews.font_shown(&bullet.font);
                form.font_was = form.font.clone();
            }
            form.size = bullet.size.trim().parse::<f64>().ok();
            form.bold = bullet.bold;
            form.italic = bullet.italic;
            form.color = bullet.color;
            form.for_bullet = true;
            form.start = form.props();
        }
        form
    }
}

/// The preview's size: the Size box's points between 6 and 72, else 12.
fn preview_size(size: Option<f64>) -> f32 {
    size.map(|pt| pt as f32).filter(|pt| pt.is_finite()).unwrap_or(12.0).clamp(6.0, 72.0)
}

/// Whether automatic text on `paper` is white.
fn dark_paper(paper: Rgb) -> bool {
    u32::from(paper.0) * 299 + u32::from(paper.1) * 587 + u32::from(paper.2) * 114 < 128_000
}

/// The sample's character properties: the dialog's fields as the document reads them.
fn font_sample_props(form: &FontForm, paper: Rgb) -> CharProps {
    let font = form.stored(form.font.trim());
    let (size, color, spacing) = (form.size, form.color.as_str(), form.spacing.unwrap_or(0.0) as f32);
    let (bold, italic, underline, strike) = (form.bold, form.italic, form.underline, form.strike);
    let (small_caps, caps, hidden) = (form.small_caps, form.caps, form.hidden);
    // Automatic text is black, or white on a dark page.
    let auto = if dark_paper(paper) { 255 } else { 0 };
    let ink = Rgb::parse(color).unwrap_or(Rgb(auto, auto, auto));
    // A hidden run is dotted, as Word draws hidden text when it is shown.
    let line = if underline {
        Underline::Single
    } else if hidden {
        Underline::Dotted
    } else {
        Underline::None
    };
    CharProps {
        font: Some(font.to_string()),
        size: Some(preview_size(size).min(40.0)),
        bold: Some(bold),
        italic: Some(italic),
        underline: Some(line),
        strike: Some(strike),
        color: Some(TextColor::Rgb(ink)),
        caps: Some(caps),
        small_caps: Some(small_caps),
        spacing: Some(spacing),
        ..Default::default()
    }
}

/// Is `c` Chinese, Japanese or Korean text (ideographs, kana, Hangul, CJK punctuation, fullwidth
/// forms)?
pub(super) fn east_asian_char(c: char) -> bool {
    matches!(u32::from(c), 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

impl FontForm {
    /// The fields as the selection stands: its formatting, the fonts as the boxes show them, and what the
    /// body needs of the app (catalog, stored names, paper), so the body never reads the app.
    pub(crate) fn read(app: &mut WordApp) -> FontForm {
        let st = app.session.run("format.state", &json!({})).unwrap_or_default();
        let s = |k: &str| st.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let b = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
        let font = app.previews.font_shown(&s("font"));
        let font_ea = app.previews.font_shown(&s("fontEastAsia"));
        let lang = crate::i18n::current().code();
        let text = app.session.selected_text();
        let text = if text.is_empty() {
            app.session.doc.para_at(&app.session.sel.focus).map(|p| p.plain_text().to_string()).unwrap_or_default()
        } else {
            text
        };
        let asian = ["zh", "ja", "ko"].iter().any(|l| lang.starts_with(l)) || !font_ea.is_empty() || text.chars().any(east_asian_char);
        let kern = st.get("kern").and_then(Value::as_f64);
        let mut names = Vec::new();
        let listed: Vec<String> = app.previews.families().into_iter().chain(app.previews.east_asian_families()).collect();
        for shown in listed {
            let stored = app.previews.font_stored(&shown);
            if stored != shown {
                names.push((shown, stored));
            }
        }
        let stored_font = app.previews.font_stored(&font);
        let stored_ea = app.previews.font_stored(&font_ea);
        let latin_note = app.font_dialog_warning(&stored_font);
        let ea_note = app.east_asian_font_box_warning(&stored_ea, &stored_font);
        let mut form = FontForm {
            font_was: font.clone(),
            font,
            font_ea,
            asian,
            size: st.get("size").and_then(Value::as_f64),
            bold: b("bold"),
            italic: b("italic"),
            underline: b("underline"),
            strike: b("strike"),
            sup: b("superscript"),
            sub: b("subscript"),
            small_caps: b("smallCaps"),
            caps: b("caps"),
            hidden: b("hidden"),
            color: s("color"),
            spacing: Some(st.get("spacing").and_then(Value::as_f64).unwrap_or(0.0)),
            kern: kern.map(|k| k > 0.0),
            kern_at: Some(kern.filter(|k| *k > 0.0).unwrap_or(12.0)),
            for_bullet: false,
            start: serde_json::Map::new(),
            names,
            paper: Some(app.session.doc.settings.page_color.unwrap_or(Rgb(255, 255, 255))),
            dark: app.session.view.dark_mode,
            shown_as: latin_note,
            shown_as_ea: ea_note,
            ..FontForm::default()
        };
        form.start = form.props();
        form
    }

    /// The stored (English) name of a font as the box shows it.
    fn stored(&self, shown: &str) -> String {
        self.names.iter().find(|(s, _)| s == shown).map_or_else(|| shown.to_string(), |(_, stored)| stored.clone())
    }

    /// The font to hand back to Define New Bullet: its stored name, and only when the user changed the box,
    /// so a bullet in the text's font keeps following the text.
    pub(crate) fn bullet_font(&self) -> Option<String> {
        (self.font.trim() != self.font_was.trim()).then(|| self.stored(self.font.trim()))
    }

    /// The character properties the fields stand for, fonts by their stored names.
    fn props(&self) -> serde_json::Map<String, Value> {
        let mut props = serde_json::Map::new();
        props.insert("bold".into(), json!(self.bold));
        props.insert("italic".into(), json!(self.italic));
        props.insert("strike".into(), json!(self.strike));
        props.insert("underline".into(), json!(if self.underline { "single" } else { "none" }));
        let vert = if self.sup {
            "superscript"
        } else if self.sub {
            "subscript"
        } else {
            "baseline"
        };
        props.insert("vertAlign".into(), json!(vert));
        props.insert("smallCaps".into(), json!(self.small_caps));
        props.insert("caps".into(), json!(self.caps));
        props.insert("hidden".into(), json!(self.hidden));
        props.insert("spacing".into(), json!(self.spacing.unwrap_or(0.0)));
        match self.kern {
            Some(true) => {
                props.insert("kern".into(), json!(self.kern_at.unwrap_or(12.0).clamp(1.0, 1638.0)));
            }
            Some(false) => {
                props.insert("kern".into(), json!(0.0));
            }
            None => {}
        }
        if !self.font.trim().is_empty() {
            props.insert("font".into(), json!(self.stored(self.font.trim())));
        }
        if !self.font_ea.trim().is_empty() {
            props.insert("fontEa".into(), json!(self.stored(self.font_ea.trim())));
        }
        if let Some(s) = self.size {
            props.insert("size".into(), json!(s));
        }
        if let Some(c) = Rgb::parse(&self.color) {
            props.insert("color".into(), json!({"Rgb": [c.0, c.1, c.2]}));
        } else if self.color.eq_ignore_ascii_case("auto") {
            // Automatic, from the colour field's drop-down (#606).
            props.insert("color".into(), json!("Auto"));
        }
        props
    }
}

/// The sample picture's key, naming everything the picture depends on.
fn sample_key(props: &CharProps, mark: Option<VertAlign>, paper: Rgb, dark: bool, ppp: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("fontsample:{props:?}:{mark:?}:{paper:?}:{dark}:{ppp}").hash(&mut h);
    h.finish()
}

/// The columns of the font rows: the labels, then the controls filling the rest.
const FONT_COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::fill(1.0)];

/// The Effects check boxes, two to a line, each column as wide as its widest box and label (the old dialog's
/// table: Italic starts just past Superscript, not halfway across the box).
const EFFECT_COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Content];

/// One font box over `family` as two grid cells, its label and the box; the action rows (Add Font…, Load System
/// Installed Fonts…) file a run.
fn font_row(
    ui: &mut Ui,
    label: Label,
    family: &mut String,
    catalog: &CatalogRef,
    settings: &mut FontPickerSettings,
    shown_as: Option<&str>,
    asks: &mut Vec<Asks>,
) {
    // Before the catalog check, so the grid's columns stay in step without one.
    Text::new(label.clone()).show(ui);
    let Some(catalog) = catalog.get() else {
        Text::new(Label::from(family.clone())).show(ui);
        return;
    };
    let actions = [Label::from(tl!("Add Font…")), Label::from(tl!("Load System Installed Fonts…"))];
    let out = font_picker(label, Bind::bound(family), catalog, Bind::bound(settings), &actions, shown_as).show(ui);
    match out.action {
        Some(0) => asks.push(Asks::Run("font.add".into(), json!({}))),
        Some(1) => asks.push(Asks::Run("font.loadSystem".into(), json!({}))),
        _ => {}
    }
}

/// A check box over `value`; true when this pass ticked or cleared it.
fn tick(ui: &mut Ui, value: &mut bool, label: &str) -> bool {
    match Checkbox::new(Bind::given(*value)).label(Label::from(label.to_owned())).show(ui).value().cloned() {
        Some(now) => {
            *value = now;
            true
        }
        None => false,
    }
}

impl WordForm for FontForm {
    fn key(&self) -> &'static str {
        "font"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Font"))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        // The old dialog's three framed boxes: an untitled one of label and control rows (labels at the start, controls
        // in the second column), Effects as two columns of check boxes, and Preview.
        Stack::new().surface(SurfaceKind::Frame).pad(Pad::Regular).fill_width().show(ui, |ui, _| {
            Grid::new(&FONT_COLUMNS).fill_width().show(ui, |ui, _| {
                // Word's two font boxes when East Asian text is in use: the Asian text font, then the Latin one.
                if self.asian {
                    font_row(
                        ui,
                        Label::from(tl!("Asian text font:")),
                        &mut self.font_ea,
                        &self.catalog,
                        &mut self.settings,
                        self.shown_as_ea.as_deref(),
                        asks,
                    );
                }
                let latin = if self.asian { tl!("Latin text font:") } else { tl!("Font:") };
                font_row(ui, Label::from(latin), &mut self.font, &self.catalog, &mut self.settings, self.shown_as.as_deref(), asks);
                Text::new(Label::from(tl!("Size:"))).show(ui);
                NumberField::new(Label::from(tl!("Size:")), Bind::bound(&mut self.size)).unit_kind(UnitKind::Points).range(1.0..=1638.0).show(ui);
                Text::new(Label::from(tl!("Font color:"))).show(ui);
                let face = if dark_paper(self.paper.unwrap_or(Rgb(255, 255, 255))) {
                    craft::theme::Srgba::rgb(1.0, 1.0, 1.0)
                } else {
                    craft::theme::Srgba::rgb(0.0, 0.0, 0.0)
                };
                let empty = ColorFieldEmpty::Automatic { label: Label::from(tl!("Automatic")), value: "auto", face: AutoFace::One(face) };
                let out = color_field(ui, &Label::from(tl!("Font color:")), &mut self.color, &empty, &self.palette);
                if out.more_colors {
                    asks.push(Asks::Color { field: "color", current: Rgb::parse(&self.color), tab: craft::color_ui::ui::ColorPickerTab::Standard });
                }
                Text::new(Label::from(tl!("Spacing (pt):"))).show(ui);
                NumberField::new(Label::from(tl!("Spacing (pt):")), Bind::bound(&mut self.spacing))
                    .unit_kind(UnitKind::Points)
                    .range(-20.0..=20.0)
                    .show(ui);
                // The check box takes the label column; its size and "Points and above" share the control column.
                // Text that differs shows the box off until it is ticked; OK then sets kerning for every run.
                let on = self.kern == Some(true);
                if let Some(now) = Checkbox::new(Bind::given(on)).label(Label::from(tl!("Kerning for fonts:"))).show(ui).value().cloned() {
                    self.kern = Some(now);
                }
                Row::new().show(ui, |ui, _| {
                    NumberField::new(Label::from(tl!("Points and above")), Bind::bound(&mut self.kern_at))
                        .unit_kind(UnitKind::Points)
                        .range(1.0..=1638.0)
                        .enabled(on)
                        .show(ui);
                    Text::new(Label::from(tl!("Points and above"))).enabled(on).show(ui);
                });
            });
        });
        GroupBox::new(Label::from(tl!("Effects")), None).show(ui, |ui, _| {
            Grid::new(&EFFECT_COLUMNS).fill_width().show(ui, |ui, _| {
                tick(ui, &mut self.bold, tl!("Bold"));
                tick(ui, &mut self.italic, tl!("Italic"));
                tick(ui, &mut self.underline, tl!("Underline"));
                tick(ui, &mut self.strike, tl!("Strikethrough"));
                // Superscript and Subscript exclude each other: the one just ticked wins.
                if tick(ui, &mut self.sup, tl!("Superscript")) && self.sup {
                    self.sub = false;
                }
                if tick(ui, &mut self.sub, tl!("Subscript")) && self.sub {
                    self.sup = false;
                }
                tick(ui, &mut self.small_caps, tl!("Small caps"));
                tick(ui, &mut self.caps, tl!("All caps"));
                tick(ui, &mut self.hidden, tl!("Hidden"));
            });
        });
        GroupBox::new(Label::from(tl!("Preview")), None).show(ui, |ui, _| {
            let paper = self.paper.unwrap_or(Rgb(255, 255, 255));
            let props = font_sample_props(self, paper);
            // A raised or lowered "2" follows the text when Superscript or Subscript is on.
            let mark = if self.sup {
                Some(VertAlign::Superscript)
            } else if self.sub {
                Some(VertAlign::Subscript)
            } else {
                None
            };
            let ppp = ui.ctx().pixels_per_point();
            let dark = self.dark;
            let key = sample_key(&props, mark, paper, dark, ppp);
            let maker = move || {
                crate::previews::font_sample(tl!("AaBbYyZz"), &props, mark, (SAMPLE_WIDTH, SAMPLE_HEIGHT, ppp), paper, dark)
                    .unwrap_or_else(|| egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]))
            };
            // The sample at its own size (rendered for the screen's density), in a frame, as the old dialog's page box.
            Stack::new().surface(SurfaceKind::Frame).pad(Pad::None).show(ui, |ui, _| {
                Picture::new(Label::from(tl!("Preview")), key, &maker)
                    .aspect(ThumbAspect::new(SAMPLE_WIDTH as u16, SAMPLE_HEIGHT as u16))
                    .extent(PictureSize::DevicePixels)
                    .show(ui);
            });
            if self.hidden {
                Text::new(Label::from(tl!("Hidden text is not shown unless you display it."))).show(ui);
            }
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        if self.for_bullet {
            return Vec::new();
        }
        let mut changed: serde_json::Map<String, Value> = self.props().into_iter().filter(|(k, v)| self.start.get(k) != Some(v)).collect();
        // A changed Latin font also sets the complex-script one (Persian and Arabic text).
        if let Some(f) = changed.get("font").cloned() {
            changed.insert("fontCs".into(), f);
        }
        if changed.is_empty() { Vec::new() } else { vec![("format.set".into(), json!({"props": changed}))] }
    }

    fn answer(&mut self, answer: ChildAnswer) {
        if let ChildAnswer::Color { field: "color", hex } = answer {
            self.color = hex;
        }
    }

    fn prepare(&mut self, app: &WordApp) {
        // PLAN: assumed `WordApp::font_catalog`, `WordApp::font_picker_settings`, `Palette::of`, `CatalogRef` (ribbon-w11, S-L284)
        self.catalog = CatalogRef(Some(app.font_catalog()));
        self.settings = app.font_picker_settings();
        self.palette = Palette::of(app);
    }

    fn write_back(&self, app: &mut WordApp) {
        // PLAN: assumed `WordApp::set_font_picker_settings(&mut self, FontPickerSettings)` (ribbon-w11)
        app.set_font_picker_settings(self.settings.clone());
    }
}

/// Opens the Font dialog (name `font`).
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let form = FontForm::read(app);
    Some(open_form(app, parent, form))
}
