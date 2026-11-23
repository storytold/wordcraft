//! Paragraph dialogs (#320): Tabs, and Borders and Shading. Each ends by running commands
//! (`para.tabs`; `para.borders`, `para.shading`, `format.set`, `format.shading`,
//! `design.pageBorders`) as one undo step, so agents get the same result without the dialog.

use std::borrow::Cow;

use craft::behavior::{Mixed, SelectionChange, keyed};
use craft::color_ui::ui::ColorPickerTab;
use craft::pages::ui::table::{BorderEdges, EdgeKey, EdgeLook, EdgePicker, SampleArea};
use craft::text::ui::DocUnit;
use craft::text::ui::ruler::{TabAlign as UiAlign, TabLeader as UiLeader, TabStop as UiStop};
use craft::text::ui::tabs_dialog::{MAX_STOPS, TabStopsDialog, TabStopsResult};
use craft::theme::Srgba;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, CollectionEvent, Entry, FieldLayout, Props as _, Show as _, ShowCollection as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use wordcraft_doc::props::{Border, BorderStyle, Borders, Rgb, TabAlign, TabLeader, TabStop};

use super::host::{Asks, ChildAnswer, DialogHooks, Keyed, Settle, WordForm, open_form, open_with};
use crate::WordApp;
use crate::colors::{AutoFace, ColorFieldEmpty, Palette, color_field};
use crate::options::unit_kind;

/// The Tabs dialog as the dialog host holds it, under its dialog name.
pub(crate) type TabStopsHost = Keyed<TabStopsDialog>;

/// An interface string as a label.
pub(super) fn word(text: impl std::fmt::Display) -> Label {
    Label::from(text.to_string())
}

/// A colour as the framework's colour value.
pub(super) fn srgba(c: Rgb) -> Srgba {
    let unit = |v: u8| f32::from(v) / 255.0;
    Srgba { r: unit(c.0), g: unit(c.1), b: unit(c.2), a: 1.0 }
}

/// A colour row of a dialog form: the framework's colour field, and a request for the More Colors dialog when the
/// field asks for it. The answer comes back to the form's `answer` under `key`.
pub(super) fn color_row(
    ui: &mut Ui,
    key: &'static str,
    label: &Label,
    value: &mut String,
    empty: &ColorFieldEmpty,
    palette: &Palette,
    asks: &mut Vec<Asks>,
) {
    if color_field(ui, label, value, empty, palette).more_colors {
        asks.push(Asks::Color { field: key, current: Rgb::parse(value), tab: ColorPickerTab::Standard });
    }
}

/// A drop-down over `labels` under `label`, showing entry `current` (empty while it is `None`). The index of the entry
/// picked in this pass.
pub(super) fn pick(ui: &mut Ui, salt: &'static str, label: Label, labels: &[Label], current: Option<usize>, enabled: bool) -> Option<usize> {
    let entries: Vec<Entry<usize>> = labels.iter().enumerate().map(|(at, text)| Entry::new(at, text.clone())).collect();
    let shown = current.map_or(Mixed::Mixed, Mixed::Same);
    Field::new(label.clone())
        .id_salt(salt)
        .show(ui, |ui, _| Select::new(label.clone(), Bind::given(shown), &entries).enabled(enabled).show(ui).value().copied())
        .inner
        .flatten()
}

/// A radio group over `labels` under `label`, entry `current` chosen. The index chosen in this pass, when it is
/// another one.
pub(super) fn radios(ui: &mut Ui, label: Label, labels: &[Label], current: usize) -> Option<usize> {
    let mut at = current;
    RadioGroup::new(label, Bind::bound(&mut at)).show(ui, |ui, cx| {
        for (i, text) in labels.iter().enumerate() {
            cx.labelled(ui, i, text.clone(), SideLabel::new().layout(FieldLayout::After), Radio::new(i));
        }
    });
    (at != current).then_some(at)
}

/// Opens the dialog `name` (`tabs`, `borders`, `pageBorders`, `shading`) over the caret's paragraph. `None` for
/// another name, and while no paragraph holds the caret.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let tab = match name {
        "tabs" => {
            let dialog = tabs_dialog(app)?;
            return Some(open_with(app, parent, dialog, DialogHooks { prepare: None, settle: settle_tabs }));
        }
        "borders" => 0,
        "pageBorders" => 1,
        "shading" => 2,
        _ => return None,
    };
    let form = BordersForm::read(app, tab)?;
    Some(open_form(app, parent, form))
}

/// The caret paragraph's stops, those a dialog can edit (finite, not the `Clear` marks, at most [`MAX_STOPS`]), and
/// the document's default interval, points.
pub(super) fn caret_tabs(app: &WordApp) -> Option<(Vec<TabStop>, f32)> {
    let s = &app.session;
    let rp = s.doc.para_at(&s.sel.focus).map(|p| s.doc.styles.resolve_para(&p.props))?;
    let stops = rp.tabs.iter().copied().filter(|t| t.align != TabAlign::Clear && t.pos.is_finite()).take(MAX_STOPS).collect();
    Some((stops, s.doc.settings.default_tab))
}

/// The Tabs dialog over the caret paragraph's stops, its OK held until `para.tabs` has run. The alignment names
/// and the leader fills are the framework dialog's.
pub(crate) fn tabs_dialog(app: &WordApp) -> Option<TabStopsHost> {
    Some(Keyed::new("tabs", tab_stops(app)?.held_ok()))
}

fn ui_align(a: TabAlign) -> UiAlign {
    match a {
        TabAlign::Center => UiAlign::Center,
        TabAlign::Right => UiAlign::Right,
        TabAlign::Decimal => UiAlign::Decimal,
        TabAlign::Bar => UiAlign::Bar,
        TabAlign::Left | TabAlign::Clear => UiAlign::Left,
    }
}

fn ui_leader(l: TabLeader) -> UiLeader {
    match l {
        TabLeader::None => UiLeader::None,
        TabLeader::Dot => UiLeader::Dot,
        TabLeader::Hyphen => UiLeader::Hyphen,
        TabLeader::Underscore => UiLeader::Underscore,
        TabLeader::MiddleDot => UiLeader::MiddleDot,
    }
}

fn doc_align(a: UiAlign) -> TabAlign {
    match a {
        UiAlign::Center => TabAlign::Center,
        UiAlign::Right => TabAlign::Right,
        UiAlign::Decimal => TabAlign::Decimal,
        UiAlign::Bar => TabAlign::Bar,
        _ => TabAlign::Left,
    }
}

fn doc_leader(l: UiLeader) -> TabLeader {
    match l {
        UiLeader::Dot => TabLeader::Dot,
        UiLeader::Hyphen => TabLeader::Hyphen,
        UiLeader::Underscore => TabLeader::Underscore,
        UiLeader::MiddleDot => TabLeader::MiddleDot,
        _ => TabLeader::None,
    }
}

/// The Tabs dialog's content over the caret paragraph's stops: the Tabs dialog, or the Paragraph dialog's Tabs page.
pub(super) fn tab_stops(app: &WordApp) -> Option<TabStopsDialog> {
    let (stops, default) = caret_tabs(app)?;
    let stops = stops.iter().map(|t| UiStop::new(t.pos, ui_align(t.align), ui_leader(t.leader))).collect();
    // Only pixel units read the resolution, and WordCraft never shows pixels.
    let unit = DocUnit::new(unit_kind(app.session.units()), 96.0);
    Some(TabStopsDialog::new(stops, default).unit(unit))
}

/// `para.tabs` parameters for what the Tabs dialog changed (`None` when nothing did). `basis` and `default_basis`
/// are the stops and the default interval the dialog opened with, points; the framework dialog keeps its result in
/// points whatever unit it shows.
pub(crate) fn tabs_params(basis: &[TabStop], default_basis: f32, result: &TabStopsResult) -> Option<Value> {
    let stops: Vec<TabStop> =
        result.stops.iter().map(|s| TabStop { pos: s.position, align: doc_align(s.align), leader: doc_leader(s.leader) }).collect();
    let mut v = Map::new();
    if stops.is_empty() && !basis.is_empty() {
        // Clear All removes the inherited stops too.
        v.insert("clearAll".into(), json!(true));
    } else {
        let set: Vec<&TabStop> = stops.iter().filter(|t| !basis.contains(t)).collect();
        if !set.is_empty() {
            v.insert("set".into(), json!(set));
        }
        if !result.cleared.is_empty() {
            v.insert("clear".into(), json!(result.cleared));
        }
    }
    if (result.default_interval - default_basis).abs() > 1e-4 {
        v.insert("default".into(), json!(result.default_interval));
    }
    (!v.is_empty()).then_some(Value::Object(v))
}

/// The settle hook of the Tabs dialog: runs a filed OK as `para.tabs`. The dialog closes when the command ran and
/// stays, showing the reason, when it failed. The hook always keeps itself: a refused OK files its next answer here,
/// and the host drops the hook once the dialog closed.
fn settle_tabs(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(result) = app.dialog_host.get_mut_by_id::<TabStopsHost>(id).and_then(|host| host.inner_mut().take_ok()) else {
        return Settle::Keep;
    };
    let (basis, default_basis) = caret_tabs(app).unwrap_or_default();
    let ran = match tabs_params(&basis, default_basis, &result) {
        Some(params) => app.run("para.tabs", params).map(|_| ()),
        None => Ok(()),
    };
    if let Some(host) = app.dialog_host.get_mut_by_id::<TabStopsHost>(id) {
        host.inner_mut().answer_ok(ran.map_err(Label::from));
    }
    Settle::Keep
}

/// A border setting: the presets on the left of the Borders and Page Border tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Setting {
    #[default]
    None,
    Box,
    /// A box with heavier bottom and right lines.
    Shadow,
    /// A box with heavier top and left lines.
    ThreeD,
    Custom,
}

/// Border lines as the dialog edits them: one style, colour and width for the chosen sides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineForm {
    pub setting: Setting,
    pub style: BorderStyle,
    /// `RRGGBB`, empty for automatic.
    pub color: String,
    pub width: f32,
    /// Top, bottom, left, right, between.
    pub sides: [bool; 5],
}

const TOP: usize = 0;
const BOTTOM: usize = 1;
const LEFT: usize = 2;
const RIGHT: usize = 3;
const BETWEEN: usize = 4;

const STYLES: [(BorderStyle, &str); 8] = [
    (BorderStyle::Single, "Single"),
    (BorderStyle::Dotted, "Dotted"),
    (BorderStyle::Dashed, "Dashed"),
    (BorderStyle::DotDash, "Dot dash"),
    (BorderStyle::Double, "Double"),
    (BorderStyle::Triple, "Triple"),
    (BorderStyle::Thick, "Thick"),
    (BorderStyle::Wave, "Wave"),
];

const WIDTHS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];

/// The presets, in the order the Setting radio buttons list them.
const SETTINGS: [Setting; 5] = [Setting::None, Setting::Box, Setting::Shadow, Setting::ThreeD, Setting::Custom];

/// The picker's edge for each side, in [`LineForm::sides`] order. The picker's inside horizontal line is the line
/// between paragraphs.
const EDGES: [(usize, EdgeKey); 5] =
    [(TOP, EdgeKey::Top), (BOTTOM, EdgeKey::Bottom), (LEFT, EdgeKey::Left), (RIGHT, EdgeKey::Right), (BETWEEN, EdgeKey::InsideHorizontal)];

impl LineForm {
    fn read(b: Option<&Borders>) -> LineForm {
        let side = |x: Option<Border>| x.filter(Border::is_visible);
        let s = b.map(|b| [side(b.top), side(b.bottom), side(b.left), side(b.right), side(b.between)]).unwrap_or_default();
        let on = s.map(|x| x.is_some());
        let first = s.iter().flatten().next().copied();
        let w = |i: usize| s.get(i).copied().flatten().map(|x| x.width).unwrap_or(0.0);
        let four = on[TOP] && on[BOTTOM] && on[LEFT] && on[RIGHT] && !on[BETWEEN];
        let setting = if first.is_none() {
            Setting::None
        } else if four && w(TOP) == w(LEFT) && w(BOTTOM) == w(RIGHT) && w(BOTTOM) > w(TOP) {
            Setting::Shadow
        } else if four && w(TOP) == w(LEFT) && w(BOTTOM) == w(RIGHT) && w(TOP) > w(BOTTOM) {
            Setting::ThreeD
        } else if four && w(TOP) == w(BOTTOM) && w(LEFT) == w(RIGHT) && w(TOP) == w(LEFT) {
            Setting::Box
        } else {
            Setting::Custom
        };
        // The base width of a heavier-sided preset is the lighter side's.
        let width = match setting {
            Setting::Shadow => w(TOP),
            Setting::ThreeD => w(BOTTOM),
            _ => first.map(|f| f.width).unwrap_or(0.5),
        };
        LineForm {
            setting,
            style: first.map(|f| f.style).unwrap_or(BorderStyle::Single),
            color: first.and_then(|f| f.color).map(Rgb::hex).unwrap_or_default(),
            width: if width > 0.0 { width } else { 0.5 },
            sides: on,
        }
    }

    fn apply_setting(&mut self, s: Setting) {
        self.setting = s;
        match s {
            Setting::None => self.sides = [false; 5],
            Setting::Box | Setting::Shadow | Setting::ThreeD => self.sides = [true, true, true, true, false],
            Setting::Custom if !self.sides.iter().any(|x| *x) => self.sides = [true, true, true, true, false],
            Setting::Custom => {}
        }
    }

    fn toggle(&mut self, side: usize) {
        if let Some(x) = self.sides.get_mut(side) {
            *x = !*x;
        }
        self.setting = if self.sides.iter().any(|x| *x) { Setting::Custom } else { Setting::None };
    }

    /// The width of `side` under the setting (Shadow and 3-D double their heavier sides).
    fn side_width(&self, side: usize) -> f32 {
        let heavy = match self.setting {
            Setting::Shadow => side == BOTTOM || side == RIGHT,
            Setting::ThreeD => side == TOP || side == LEFT,
            _ => false,
        };
        let w = self.width.clamp(0.25, 6.0);
        if heavy { (w * 2.0).min(6.0) } else { w }
    }

    fn border(&self, side: usize) -> Border {
        Border { style: self.style, width: self.side_width(side), color: Rgb::parse(&self.color), space: 0.0 }
    }

    /// The `sides` parameter of `para.borders` / `design.pageBorders`.
    fn sides_param(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (i, k) in ["top", "bottom", "left", "right", "between"].into_iter().enumerate() {
            if self.sides.get(i).copied().unwrap_or(false) {
                let mut side = json!({"style": self.style.ooxml(), "width": self.side_width(i)});
                if Rgb::parse(&self.color).is_some() {
                    side["color"] = json!(self.color);
                }
                m.insert(k.into(), side);
            }
        }
        Value::Object(m)
    }

    /// The sides as the picker shows them; the line between paragraphs only where `between` allows it.
    fn edges(&self, between: bool) -> BorderEdges {
        let mut edges = BorderEdges::default();
        for (side, edge) in EDGES {
            edges.set(edge, self.sides.get(side).copied().unwrap_or(false) && (side != BETWEEN || between));
        }
        edges
    }

    /// Takes the picker's change: each side it flipped is toggled, so the setting follows (Custom, or None when
    /// no side is left).
    fn take_edges(&mut self, edges: BorderEdges, between: bool) {
        for (side, edge) in EDGES {
            if side == BETWEEN && !between {
                continue;
            }
            if edges.get(edge) != self.sides.get(side).copied().unwrap_or(false) {
                self.toggle(side);
            }
        }
    }
}

/// The Borders and Shading dialog's fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BordersForm {
    /// 0 Borders, 1 Page Border, 2 Shading.
    pub tab: u8,
    pub para: LineForm,
    /// Borders apply to the selected text (a character border) instead of the paragraphs.
    pub to_text: bool,
    pub page: LineForm,
    /// Page borders apply to the whole document instead of this section.
    pub whole_doc: bool,
    /// Fill `RRGGBB`, empty for none.
    pub fill: String,
    pub fill_to_text: bool,
    /// The colours the colour fields offer, filled every pass by `prepare`.
    // PLAN: assumed `Palette` derives Clone, Debug, Default and PartialEq (ribbon-w11, S-L284)
    #[serde(skip)]
    palette: Palette,
    /// The form as `read` made it, which [`BordersForm::commands`] compares with. It is constant while the dialog is
    /// open.
    #[serde(skip)]
    basis: Option<Box<BordersForm>>,
}

impl BordersForm {
    pub fn read(app: &WordApp, tab: u8) -> Option<BordersForm> {
        let s = &app.session;
        let rp = s.doc.para_at(&s.sel.focus).map(|p| s.doc.styles.resolve_para(&p.props))?;
        let sect = wordcraft_engine::cmd::page::sect(s);
        let mut f = BordersForm {
            tab: tab.min(2),
            para: LineForm::read(rp.borders.as_ref()),
            to_text: false,
            page: LineForm::read(sect.page_borders.as_ref()),
            whole_doc: true,
            fill: rp.shading.map(Rgb::hex).unwrap_or_default(),
            fill_to_text: false,
            palette: Palette::default(),
            basis: None,
        };
        f.page.sides[BETWEEN] = false;
        f.basis = Some(Box::new(f.clone()));
        Some(f)
    }

    /// The commands OK runs, in order: only the parts that changed.
    pub fn commands(&self) -> Vec<(&'static str, Value)> {
        let Some(b) = self.basis.as_deref() else { return Vec::new() };
        let mut out = Vec::new();
        if self.para != b.para || self.to_text != b.to_text {
            if self.to_text {
                let first = [TOP, LEFT, BOTTOM, RIGHT].into_iter().find(|i| self.para.sides.get(*i).copied().unwrap_or(false));
                let border = match first {
                    Some(i) => self.para.border(i),
                    None => Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 },
                };
                out.push(("format.set", json!({"props": {"border": border}})));
            } else {
                out.push(("para.borders", json!({"sides": self.para.sides_param()})));
            }
        }
        if self.page != b.page || self.whole_doc != b.whole_doc {
            let apply = if self.whole_doc { "document" } else { "section" };
            out.push(("design.pageBorders", json!({"sides": self.page.sides_param(), "applyTo": apply})));
        }
        if self.fill != b.fill || self.fill_to_text != b.fill_to_text {
            let color = Rgb::parse(&self.fill).map(|c| json!(c.hex())).unwrap_or(Value::Null);
            // Apply to: Paragraph shades the paragraphs even for text selected inside one.
            out.push(if self.fill_to_text {
                ("format.shading", json!({"color": color}))
            } else {
                ("para.shading", json!({"color": color, "paragraph": true}))
            });
        }
        out
    }

    fn tab_strip(&mut self, ui: &mut Ui) {
        let tabs = [(0u8, tl!("Borders").to_string()), (1, tl!("Page Border").to_string()), (2, tl!("Shading").to_string())];
        let source = keyed(&tabs[..], |tab: &(u8, String)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
        let strip = TabStrip::new(word(tl!("Borders and Shading"))).kind(TabKind::Text).shown(self.tab).natural_height(true);
        let out = strip
            .show_items(ui, &source, &mut |key, _| Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or_else(String::new, |tab| tab.1.clone())));
        for event in out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(chosen) = keys.last()
            {
                self.tab = *chosen;
            }
        }
    }

    /// One "Apply to" choice of two: `first` and `second` are the labels of `false` and `true`.
    fn apply_to(ui: &mut Ui, on: &mut bool, first: Label, second: Label) {
        if let Some(at) = radios(ui, word(tl!("Apply to:")), &[first, second], usize::from(*on)) {
            *on = at == 1;
        }
    }
}

/// Setting, style, colour and width on the left; the sample with its side toggles on the right. `area` is what the
/// borders go around, and the line between paragraphs exists only around a paragraph.
fn lines(ui: &mut Ui, key: &'static str, l: &mut LineForm, area: SampleArea, fill: Option<Rgb>, palette: &Palette, asks: &mut Vec<Asks>) {
    let between = area == SampleArea::Paragraph;
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            let names = [word(tl!("None")), word(tl!("Box")), word(tl!("Shadow")), word(tl!("3-D")), word(tl!("Custom"))];
            let current = SETTINGS.iter().position(|s| *s == l.setting).unwrap_or(0);
            if let Some(setting) = radios(ui, word(tl!("Setting:")), &names, current).and_then(|at| SETTINGS.get(at)) {
                l.apply_setting(*setting);
            }
        });
        Stack::new().show(ui, |ui, _| {
            let names: Vec<Label> = STYLES.iter().map(|(_, name)| word(tl!(name))).collect();
            let current = STYLES.iter().position(|(s, _)| *s == l.style);
            if let Some((style, _)) = pick(ui, "style", word(tl!("Style:")), &names, current, true).and_then(|at| STYLES.get(at)) {
                l.style = *style;
            }
            let automatic = ColorFieldEmpty::Automatic { label: word(tl!("Automatic")), value: "", face: AutoFace::One(srgba(Rgb(0, 0, 0))) };
            color_row(ui, key, &word(tl!("Color:")), &mut l.color, &automatic, palette, asks);
            let names: Vec<Label> = WIDTHS.iter().map(|w| word(crate::i18n::fmt(tl!("{n} pt"), &[("n", &w.to_string())]))).collect();
            let current = WIDTHS.iter().position(|w| *w == l.width);
            if let Some(width) = pick(ui, "width", word(tl!("Width:")), &names, current, true).and_then(|at| WIDTHS.get(at)) {
                l.width = *width;
            }
        });
        Stack::new().show(ui, |ui, _| {
            let mut edges = l.edges(between);
            let sample = EdgeLook::sample(area, fill.map(srgba));
            let picker = EdgePicker::new(word(tl!("Preview")), Bind::bound(&mut edges)).look(sample);
            let picker = if between { picker.paragraph() } else { picker };
            picker.show(ui);
            l.take_edges(edges, between);
            let mut sides = vec![(TOP, word(tl!("Top"))), (BOTTOM, word(tl!("Bottom"))), (LEFT, word(tl!("Left"))), (RIGHT, word(tl!("Right")))];
            if between {
                sides.push((BETWEEN, word(tl!("Between"))));
            }
            for (side, text) in sides {
                let shown = l.sides.get(side).copied().unwrap_or(false);
                let mut on = shown;
                Checkbox::new(Bind::bound(&mut on)).label(text).show(ui);
                if on != shown {
                    l.toggle(side);
                }
            }
        });
    });
}

impl WordForm for BordersForm {
    fn key(&self) -> &'static str {
        "borders"
    }

    fn title(&self) -> Label {
        word(tl!("Borders and Shading"))
    }

    // PLAN: assumed `WordForm::prepare(&mut self, &WordApp)` and `Palette::of(app)` (dialogs-w13 S-L283, ribbon-w11 S-L284)
    fn prepare(&mut self, app: &WordApp) {
        self.palette = Palette::of(app);
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        self.tab_strip(ui);
        let palette = self.palette.clone();
        let fill = Rgb::parse(&self.fill);
        match self.tab {
            0 => {
                let area = if self.to_text { SampleArea::Text } else { SampleArea::Paragraph };
                lines(ui, "para", &mut self.para, area, fill, &palette, asks);
                Self::apply_to(ui, &mut self.to_text, word(tl!("Paragraph")), word(tl!("Text")));
            }
            1 => {
                lines(ui, "page", &mut self.page, SampleArea::Page, None, &palette, asks);
                Self::apply_to(ui, &mut self.whole_doc, word(tl!("This section")), word(tl!("Whole document")));
            }
            _ => {
                color_row(ui, "fill", &word(tl!("Fill")), &mut self.fill, &ColorFieldEmpty::NoColor, &palette, asks);
                // The same sample as the Borders tab shows, without the lines and not editable.
                let mut none = BorderEdges::default();
                let area = if self.fill_to_text { SampleArea::Text } else { SampleArea::Paragraph };
                let sample = EdgeLook::sample(area, Rgb::parse(&self.fill).map(srgba));
                EdgePicker::new(word(tl!("Preview")), Bind::bound(&mut none)).look(sample).enabled(false).show(ui);
                Self::apply_to(ui, &mut self.fill_to_text, word(tl!("Paragraph")), word(tl!("Text")));
            }
        }
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        BordersForm::commands(self).into_iter().map(|(id, params)| (id.to_string(), params)).collect()
    }

    fn answer(&mut self, answer: ChildAnswer) {
        if let ChildAnswer::Color { field, hex } = answer {
            match field {
                "para" => self.para.color = hex,
                "page" => self.page.color = hex,
                "fill" => self.fill = hex,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::FormHost;
    use super::super::host::testing::{confirm, harness};
    use super::*;
    use wordcraft_engine::Session;

    fn open_tabs(h: &mut egui_kittest::Harness<'static, WordApp>) -> DialogId {
        let id = crate::dialogs::open(h.state_mut(), "tabs", &json!({})).expect("Tabs opens without params");
        h.run_steps(2);
        id
    }

    fn app_with(text: &str) -> WordApp {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let _ = app.run("document.setText", json!({"text": text}));
        app
    }

    fn caret_tabs_of(app: &WordApp) -> Option<Vec<TabStop>> {
        app.session.doc.para_at(&app.session.sel.focus).and_then(|p| p.props.tabs.clone())
    }

    /// #320: OK in Borders and Shading applies a box border with the chosen style, colour and
    /// width, and the page border and fill with it — one undo step for all three.
    #[test]
    fn borders_dialog_ok_applies_box_in_one_undo_step() {
        let mut h = harness(app_with("Boxed paragraph"));
        let id = crate::dialogs::open(h.state_mut(), "borders", &json!({})).expect("Borders and Shading opens from the menu item");
        h.run_steps(2);
        let f = h.state_mut().dialog_host.get_mut_by_id::<FormHost<BordersForm>>(id).expect("the form is hosted").form_mut();
        assert_eq!(f.para.setting, Setting::None);
        f.para.apply_setting(Setting::Box);
        f.para.style = BorderStyle::Double;
        f.para.width = 1.5;
        f.para.color = "1F4E79".into();
        f.page.apply_setting(Setting::Shadow);
        f.fill = "DDEEFF".into();
        let undo_before = h.state().session.undo_labels().len();
        confirm(&mut h, id);
        let app = h.state_mut();
        let props = app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        let b = props.borders.unwrap_or_default();
        for side in [b.top, b.bottom, b.left, b.right] {
            let side = side.unwrap_or_default();
            assert_eq!((side.style, side.width, side.color), (BorderStyle::Double, 1.5, Rgb::parse("1F4E79")));
        }
        assert!(b.between.is_none());
        assert_eq!(props.shading, Rgb::parse("DDEEFF"));
        let pb = app.session.doc.last_section.page_borders.unwrap_or_default();
        assert_eq!(pb.right.map(|x| x.width), Some(1.0), "Shadow doubles the right and bottom lines");
        assert_eq!(pb.top.map(|x| x.width), Some(0.5));
        assert_eq!(app.session.undo_labels().len(), undo_before + 1, "one undo step");
        let _ = app.run("edit.undo", json!({}));
        let props = app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default();
        assert!(props.borders.is_none() && props.shading.is_none());
        assert!(app.session.doc.last_section.page_borders.is_none());
        // Reading the result back gives the same setting.
        assert_eq!(LineForm::read(Some(&pb)).setting, Setting::Shadow);
        // Unchanged, OK runs nothing.
        let f = BordersForm::read(app, 0).unwrap_or_else(|| panic!("form"));
        assert!(f.commands().is_empty());
    }

    /// #320: what the Tabs dialog answers reaches `para.tabs`: a stop set, a stop cleared, Clear All, and the default
    /// interval; an unchanged OK runs nothing.
    #[test]
    fn tabs_table_builds_and_clears() {
        let mut h = harness(app_with("a\tb"));
        let id = open_tabs(&mut h);
        let dialog = h.state_mut().dialog_host.get_mut_by_id::<TabStopsHost>(id).expect("the dialog is hosted").inner_mut();
        // Set, Set: rows at one and two default intervals; the first is cleared again.
        dialog.new_stop();
        dialog.new_stop();
        dialog.select(Some(0));
        dialog.clear();
        confirm(&mut h, id);
        assert_eq!(caret_tabs_of(h.state()), Some(vec![TabStop { pos: 72.0, align: TabAlign::Left, leader: TabLeader::None }]));

        // Clearing the one stop of two clears just it.
        let _ = h.state_mut().run("para.tabs", json!({"set": [{"pos": 144.0, "align": "right"}]}));
        let id = open_tabs(&mut h);
        let dialog = h.state_mut().dialog_host.get_mut_by_id::<TabStopsHost>(id).expect("the dialog is hosted").inner_mut();
        dialog.select(Some(0));
        dialog.clear();
        confirm(&mut h, id);
        assert_eq!(caret_tabs_of(h.state()), Some(vec![TabStop { pos: 144.0, align: TabAlign::Right, leader: TabLeader::None }]));

        // Unchanged, OK runs nothing; Clear All clears.
        let undo = h.state().session.undo_labels().len();
        let id = open_tabs(&mut h);
        confirm(&mut h, id);
        assert_eq!(h.state().session.undo_labels().len(), undo, "an unchanged OK is no undo step");
        let id = open_tabs(&mut h);
        h.state_mut().dialog_host.get_mut_by_id::<TabStopsHost>(id).expect("the dialog is hosted").inner_mut().clear_all();
        confirm(&mut h, id);
        assert!(caret_tabs_of(h.state()).is_none_or(|t| t.is_empty()));

        // The default interval, in points whatever the dialog shows.
        let mut result = TabStopsResult::default();
        let basis: Vec<TabStop> = Vec::new();
        result.default_interval = 72.0;
        assert_eq!(tabs_params(&basis, 36.0, &result), Some(json!({"default": 72.0})));
        assert_eq!(tabs_params(&basis, 72.0, &result), None);
    }
}
