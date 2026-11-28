//! Define New Multilevel List (Home › Paragraph › Multilevel List) and Track Changes Options
//! (Review › Tracking). Each fills a form from the session and ends in one command
//! (`list.define`, `review.trackingOptions`), so agents get the same result without the dialog.

use craft::behavior::{EditEvents, Mixed, Selection, SelectionChange, keyed};
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft::ui::dialog::DialogId;
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::primitives::{ContentPainterCx, TextRun};
use craft::ui::runtime::env::{Env, Label, LengthUnits, UnitKind};
use craft::ui::sections::Section;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, TextField};
use egui::{Color32, Rect, Sense, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::section::NumFormat;
use wordcraft_engine::cmd::lists::{LEVELS, caret_list, level_json, levels_of};

use super::dialogs_para::{color_row, pick, radios, srgba, word};
use super::font::FontRequest;
use super::host::{Asks, ChildAnswer, WordForm, open_form};
use crate::WordApp;
use crate::colors::{AutoFace, ColorFieldEmpty, Palette};
use crate::options::lengths;

/// Number styles a level can have: OOXML name and a sample.
const STYLES: [(&str, &str); 10] = [
    ("decimal", "1, 2, 3, …"),
    ("lowerLetter", "a, b, c, …"),
    ("upperLetter", "A, B, C, …"),
    ("lowerRoman", "i, ii, iii, …"),
    ("upperRoman", "I, II, III, …"),
    ("decimalZero", "01, 02, 03, …"),
    ("ordinal", "1st, 2nd, 3rd, …"),
    ("cardinalText", "One, Two, Three, …"),
    ("bullet", "•"),
    ("none", ""),
];

/// What follows a number, in the order of the Follow number with list.
const FOLLOWS: [&str; 3] = ["tab", "space", "nothing"];

/// How a number sits at its aligned-at position, in the order of the Number alignment radio group.
const ALIGNS: [&str; 3] = ["left", "center", "right"];

/// The range of a level's positions, points.
const POSITION_RANGE: std::ops::RangeInclusive<f64> = -720.0..=1584.0;

/// Opens Define New Multilevel List (`defineList`), Define New Number Format (`defineNumber`), or
/// Track Changes Options (`trackChangesOptions`). `None` for another name.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "defineList" => Some(open_form(app, parent, ListForm::read(app))),
        "defineNumber" => Some(open_form(app, parent, NumberForm::default())),
        "trackChangesOptions" => Some(open_form(app, parent, TrackForm::read(app))),
        _ => None,
    }
}

/// The value a number field's events leave, `None` when they carry none.
fn edited(events: &EditEvents<Option<f64>>) -> Option<f64> {
    events.iter().filter_map(|e| e.value().copied().flatten()).next_back()
}

/// A length field of a level, points kept and shown in the interface unit; `enabled` false greys it.
fn length_field(ui: &mut Ui, salt: &'static str, label: Label, lengths: LengthUnits, points: &mut f32, enabled: bool) {
    let events = Field::new(label.clone())
        .id_salt(salt)
        .show(ui, |ui, _| {
            NumberField::length(label.clone(), Bind::given(Some(f64::from(*points))), lengths)
                .range(POSITION_RANGE)
                .enabled(enabled)
                .show(ui)
                .edit
                .events
        })
        .inner
        .unwrap_or_default();
    if let Some(v) = edited(&events) {
        *points = v as f32;
    }
}

/// A section of the dialog: a rule and `title` over a field group, so the section's fields share one label column.
fn section<R>(ui: &mut Ui, title: Label, mut body: impl FnMut(&mut Ui) -> R) -> Option<R> {
    Section::fixed(title.clone()).show(ui, |ui, _| FieldGroup::new(title.clone()).show(ui, |ui, _| body(ui)).inner).inner.flatten()
}

/// The points of one level kept and shown, stored points in the interface unit.
fn stored_points() -> LengthUnits {
    LengthUnits::new(UnitKind::Points, UnitKind::Points, 96.0)
}

/// One level of the Define New Multilevel List form. Positions are points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelForm {
    pub text: String,
    pub format: String,
    pub start: u32,
    pub restart: bool,
    /// 1-based level it restarts after.
    pub restart_after: u8,
    /// Linked paragraph style id, empty for none.
    pub style: String,
    pub align: String,
    pub aligned_at: f32,
    pub indent_at: f32,
    pub follow: String,
    pub tab_on: bool,
    pub tab_at: f32,
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    pub legal: bool,
}

impl LevelForm {
    fn read(i: usize, v: &Value) -> LevelForm {
        let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let f = |key: &str| v.get(key).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite());
        let restart_after = v.get("restartAfter").and_then(Value::as_u64);
        LevelForm {
            text: s("text"),
            format: s("format"),
            start: v.get("start").and_then(Value::as_u64).unwrap_or(1).min(32_767) as u32,
            restart: restart_after != Some(0),
            restart_after: restart_after.filter(|r| *r > 0).map(|r| r.min(8) as u8).unwrap_or(i as u8).max(1),
            style: s("style"),
            align: s("align"),
            aligned_at: f("alignedAt").unwrap_or(0.0),
            indent_at: f("indentAt").unwrap_or(0.0),
            follow: s("follow"),
            tab_on: f("tabAt").is_some(),
            tab_at: f("tabAt").or(f("indentAt")).unwrap_or(0.0),
            font: s("font"),
            bold: v.get("bold").and_then(Value::as_bool).unwrap_or(false),
            italic: v.get("italic").and_then(Value::as_bool).unwrap_or(false),
            legal: v.get("legal").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    /// The level as `list.define` takes it.
    pub fn params(&self, i: usize) -> Value {
        json!({
            "format": self.format,
            "text": self.text,
            "start": self.start,
            "restartAfter": if !self.restart { json!(0) } else if i > 0 && (self.restart_after as usize) < i { json!(self.restart_after) } else { Value::Null },
            "style": if self.style.is_empty() { Value::Null } else { json!(self.style) },
            "align": self.align,
            "alignedAt": self.aligned_at,
            "indentAt": self.indent_at,
            "follow": self.follow,
            "tabAt": if self.tab_on && self.follow == "tab" { json!(self.tab_at) } else { Value::Null },
            "font": if self.font.trim().is_empty() { Value::Null } else { json!(self.font.trim()) },
            "bold": self.bold,
            "italic": self.italic,
            "legal": self.legal,
        })
    }
}

/// The Define New Multilevel List form: nine levels and the one being edited.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListForm {
    pub levels: Vec<LevelForm>,
    pub level: usize,
    /// Paragraph styles a level can link to: (id, name).
    #[serde(skip)]
    pub styles: Vec<(String, String)>,
    /// The interface unit the positions are shown in, set by `read` because the body has no app.
    #[serde(skip, default = "stored_points")]
    lengths: LengthUnits,
}

impl Default for ListForm {
    fn default() -> Self {
        ListForm { levels: Vec::new(), level: 0, styles: Vec::new(), lengths: stored_points() }
    }
}

/// One level of the preview: what it shows and where.
#[derive(Clone, Debug)]
struct PreviewRow {
    sample: String,
    aligned_at: f32,
    indent_at: f32,
    align: String,
    follow: String,
}

impl ListForm {
    /// Starts from the caret's list, or from a new 1. a. i. list.
    pub fn read(app: &WordApp) -> ListForm {
        let s = &app.session;
        let levels =
            caret_list(s).and_then(|n| levels_of(s, n)).unwrap_or_else(|| wordcraft_doc::numbering::levels_for(wordcraft_doc::ListKind::Numbered));
        let levels = (0..LEVELS).map(|i| LevelForm::read(i, &levels.get(i).map(level_json).unwrap_or(Value::Null))).collect();
        let mut styles: Vec<(String, String)> = s
            .doc
            .styles
            .styles
            .iter()
            .filter(|st| st.kind == wordcraft_doc::StyleKind::Paragraph && !st.hidden)
            .map(|st| (st.id.clone(), st.name.clone()))
            .collect();
        styles.sort_by(|a, b| a.1.cmp(&b.1));
        ListForm { levels, level: 0, styles, lengths: lengths(s.units()) }
    }

    /// `list.define` parameters for the whole list.
    pub fn params(&self) -> Value {
        json!({"levels": self.levels.iter().enumerate().map(|(i, l)| l.params(i)).collect::<Vec<_>>()})
    }

    /// The number level `k` shows with every level at its start value, as in the preview.
    pub fn sample(&self, k: usize) -> String {
        let Some(l) = self.levels.get(k) else { return String::new() };
        match l.format.as_str() {
            "none" => return String::new(),
            "bullet" => return l.text.clone(),
            _ => {}
        }
        let mut out = String::new();
        let mut chars = l.text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%'
                && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
            {
                chars.next();
                let j = (d as usize).saturating_sub(1).min(LEVELS - 1);
                let lv = self.levels.get(j);
                let fmt = if l.legal && j != k { NumFormat::Decimal } else { lv.map(|x| NumFormat::from_ooxml(&x.format)).unwrap_or_default() };
                out.push_str(&fmt.format(lv.map(|x| x.start).unwrap_or(1).max(1)));
            } else {
                out.push(c);
            }
        }
        out
    }

    /// The level list on the left and the preview of all nine levels on the right.
    fn level_row(&mut self, ui: &mut Ui) {
        let rows: Vec<PreviewRow> = (0..LEVELS)
            .filter_map(|k| {
                let l = self.levels.get(k)?;
                Some(PreviewRow {
                    sample: self.sample(k),
                    aligned_at: l.aligned_at,
                    indent_at: l.indent_at,
                    align: l.align.clone(),
                    follow: l.follow.clone(),
                })
            })
            .collect();
        let level = self.level;
        Row::new().show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Text::new(word(tl!("Click level to modify:"))).show(ui);
                let numbers: Vec<usize> = (0..LEVELS).collect();
                let source = keyed(&numbers, |at: &usize| *at);
                let events = List::new(word(tl!("Click level to modify:")))
                    .select(SelectMode::Single)
                    .natural_height(true)
                    .selection(Bind::given(Selection::single(level)))
                    .show_items(ui, &source, &mut |at, _| ListRow::new((*at + 1).to_string()))
                    .events;
                for event in events {
                    if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                        && let Some(chosen) = keys.last()
                    {
                        self.level = *chosen;
                    }
                }
            });
            Stack::new().show(ui, |ui, _| {
                Text::new(word(tl!("Preview"))).show(ui);
                Picture::painter(word(tl!("Preview")), ThumbAspect::new(5, 3), preview(rows.clone(), level))
                    .extent(PictureSize::Thumb(ThumbSize::Large))
                    .show(ui);
            });
        });
    }
}

/// All nine levels, each number at its aligned-at position followed by a grey line of text at its text indent; the level
/// being edited at full strength and the others faded.
fn preview(rows: Vec<PreviewRow>, edited: usize) -> impl Fn(&ContentPainterCx, Rect) {
    move |cx: &ContentPainterCx, rect: Rect| {
        cx.data_shapes([
            egui::Shape::rect_filled(rect, 2.0, Color32::WHITE),
            egui::Shape::rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Color32::from_gray(160)), egui::StrokeKind::Inside),
        ]);
        let row = rect.height() / LEVELS as f32;
        // Positions are points, scaled so 4.5 inches fit.
        let scale = (rect.width() - 16.0).max(1.0) / 324.0;
        let x_of = |points: f32| rect.left() + 8.0 + (points * scale).clamp(0.0, (rect.width() - 40.0).max(0.0));
        for (i, r) in rows.iter().enumerate() {
            let top = rect.top() + row * i as f32;
            let paint = |cx: &ContentPainterCx| {
                let run = TextRun::new(r.sample.clone()).role(TypeRole::Caption);
                let layout = cx.with_draw(|draw| draw.layout(&run, f32::INFINITY));
                let width = layout.size().x;
                let x = x_of(r.aligned_at);
                let number_x = match r.align.as_str() {
                    "center" => x - width / 2.0,
                    "right" => x - width,
                    _ => x,
                };
                let spare = if layout.galley.rows.len() <= 1 { (row - layout.size().y).max(0.0) } else { 0.0 };
                cx.data_shapes([egui::Shape::galley_with_override_text_color(
                    pos2(number_x, top + spare / 2.0),
                    layout.galley.clone(),
                    Color32::BLACK,
                )]);
                let text_x = match r.follow.as_str() {
                    "space" => number_x + width + 3.0,
                    "nothing" => number_x + width,
                    _ => x_of(r.indent_at).max(number_x + width + 2.0),
                };
                let bar = Rect::from_min_max(pos2(text_x, top + row / 2.0 - 2.0), pos2(rect.right() - 8.0, top + row / 2.0 + 2.0));
                if bar.width() > 0.0 {
                    let draw_bar = |cx: &ContentPainterCx| cx.data_shapes([egui::Shape::rect_filled(bar, 1.0, Color32::BLACK)]);
                    cx.faded(0.45, draw_bar);
                }
            };
            if i == edited {
                paint(cx);
            } else {
                cx.faded(0.5, paint);
            }
        }
    }
}

impl WordForm for ListForm {
    fn key(&self) -> &'static str {
        "defineList"
    }

    fn title(&self) -> Label {
        word(tl!("Define New Multilevel List"))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        self.level_row(ui);
        let i = self.level.min(LEVELS - 1);
        let (styles, lengths) = (self.styles.clone(), self.lengths);
        let Some(l) = self.levels.get_mut(i) else { return true };
        section(ui, word(tl!("Number format")), |ui| number_format(ui, l, i, &styles));
        section(ui, word(tl!("Position")), |ui| position(ui, l, lengths));
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("list.define".to_string(), self.params())]
    }
}

/// The Number format part of a level: the number's text and style, what it restarts after, the linked style, the font
/// and legal numbering.
fn number_format(ui: &mut Ui, l: &mut LevelForm, i: usize, styles: &[(String, String)]) {
    Field::new(word(tl!("Enter formatting for number:"))).id_salt("number-text").show(ui, |ui, _| {
        TextField::new(word(tl!("Enter formatting for number:")), Bind::bound(&mut l.text)).show(ui);
    });
    let names: Vec<Label> = STYLES
        .iter()
        .map(|(_, sample)| match *sample {
            "" => word(tl!("None")),
            "•" => word(format!("• {}", tl!("Bullet"))),
            other => word(other),
        })
        .collect();
    let current = STYLES.iter().position(|(k, _)| *k == l.format);
    if let Some((k, _)) = pick(ui, "number-style", word(tl!("Number style for this level:")), &names, current, true).and_then(|at| STYLES.get(at)) {
        // A new style puts this level's number in the text if it had none (a bullet its character).
        if *k == "bullet" {
            l.text = "•".into();
        } else if l.format == "bullet" || !l.text.contains('%') {
            l.text = format!("%{}.", i + 1);
        }
        l.format = k.to_string();
    }
    let level_name = |n: usize| word(crate::i18n::fmt(tl!("Level {n}"), &[("n", &n.to_string())]));
    if i > 0 {
        let names: Vec<Label> = (1..=i).map(level_name).collect();
        if let Some(at) = pick(ui, "include-level", word(tl!("Include level number from:")), &names, None, true) {
            l.text.push_str(&format!("%{}", at + 1));
        }
    }
    let events = Field::new(word(tl!("Start at:")))
        .id_salt("start-at")
        .show(ui, |ui, _| NumberField::new(word(tl!("Start at:")), Bind::given(Some(f64::from(l.start)))).range(0.0..=32_767.0).show(ui).edit.events)
        .inner
        .unwrap_or_default();
    if let Some(v) = edited(&events) {
        l.start = v.round().clamp(0.0, 32_767.0) as u32;
    }
    if i > 0 {
        toggle(ui, word(tl!("Restart list after:")), &mut l.restart, true);
        let names: Vec<Label> = (1..=i).map(level_name).collect();
        let current = usize::from(l.restart_after).clamp(1, i) - 1;
        if let Some(at) = pick(ui, "restart-after", word(tl!("Restart list after:")), &names, Some(current), l.restart) {
            l.restart_after = (at + 1).min(8) as u8;
        }
    }
    let mut names: Vec<Label> = vec![word(tl!("(no style)"))];
    names.extend(styles.iter().map(|(_, name)| word(name)));
    let current = if l.style.is_empty() { Some(0) } else { styles.iter().position(|(id, _)| *id == l.style).map(|at| at + 1) };
    if let Some(at) = pick(ui, "link-style", word(tl!("Link level to style:")), &names, current, true) {
        l.style = at.checked_sub(1).and_then(|k| styles.get(k)).map(|(id, _)| id.clone()).unwrap_or_default();
    }
    Field::new(word(tl!("Font:"))).id_salt("font").show(ui, |ui, _| {
        TextField::new(word(tl!("Font:")), Bind::bound(&mut l.font)).show(ui);
    });
    toggle(ui, word(tl!("Bold")), &mut l.bold, true);
    toggle(ui, word(tl!("Italic")), &mut l.italic, true);
    toggle(ui, word(tl!("Legal style numbering")), &mut l.legal, true);
}

/// A check box over `on`.
fn toggle(ui: &mut Ui, label: Label, on: &mut bool, enabled: bool) {
    let mut now = *on;
    Checkbox::new(Bind::bound(&mut now)).label(label).enabled(enabled).show(ui);
    *on = now;
}

/// The Position part of a level: the number's alignment and position, the text indent and what follows the number.
fn position(ui: &mut Ui, l: &mut LevelForm, lengths: LengthUnits) {
    let names = [word(tl!("Left")), word(tl!("Center")), word(tl!("Right"))];
    let current = ALIGNS.iter().position(|a| *a == l.align).unwrap_or(0);
    if let Some(at) = radios(ui, word(tl!("Number alignment:")), &names, current) {
        l.align = ALIGNS.get(at).map(|a| a.to_string()).unwrap_or_default();
    }
    length_field(ui, "aligned-at", word(tl!("Aligned at:")), lengths, &mut l.aligned_at, true);
    length_field(ui, "indent-at", word(tl!("Text indent at:")), lengths, &mut l.indent_at, true);
    let names = [word(tl!("Tab character")), word(tl!("Space")), word(tl!("Nothing"))];
    let current = FOLLOWS.iter().position(|f| *f == l.follow).unwrap_or(0);
    if let Some(at) = pick(ui, "follow", word(tl!("Follow number with:")), &names, Some(current), true) {
        l.follow = FOLLOWS.get(at).map(|f| f.to_string()).unwrap_or_default();
    }
    let tab = l.follow == "tab";
    toggle(ui, word(tl!("Add tab stop at:")), &mut l.tab_on, tab);
    length_field(ui, "tab-at", word(tl!("Add tab stop at:")), lengths, &mut l.tab_at, tab && l.tab_on);
}

/// Define New Number Format (Home › Paragraph › Numbering, #2787): the number style, the number
/// format (`%1` = the number; none once the number is deleted), the number's font and its
/// alignment. OK runs one `para.defineNumber`, so agents get the same list without the dialog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NumberForm {
    pub format: String,
    pub text: String,
    pub align: String,
    /// The number's formatting from Font…, in `format.set`'s shape (font, size, bold, colour…).
    pub props: serde_json::Map<String, Value>,
    /// Where the number stood (a char index of `text`) when it was deleted, so a number style
    /// picked again puts it back there.
    #[serde(skip)]
    pub number_at: Option<usize>,
}

impl Default for NumberForm {
    fn default() -> NumberForm {
        NumberForm { format: "decimal".into(), text: "%1.".into(), align: "left".into(), props: serde_json::Map::new(), number_at: None }
    }
}

impl NumberForm {
    /// The `n`th label, as the list will show it (for tests).
    #[cfg(test)]
    fn sample(&self, n: u32) -> String {
        if self.format == "none" {
            return wordcraft_doc::numbering::none_label(&self.text);
        }
        self.text.replace("%1", &NumFormat::from_ooxml(&self.format).format(n))
    }

    /// Delete the number from the format (Word then shows the style as none).
    fn remove_number(&mut self) {
        if let Some((before, after)) = self.text.split_once("%1") {
            self.number_at = Some(before.chars().count());
            self.text = format!("{before}{after}");
        }
        self.format = "none".into();
    }

    /// Pick a style: a number style after none puts the number back where it stood (else at the
    /// end), none deletes it.
    fn set_style(&mut self, style: &str) {
        if style == "none" {
            self.remove_number();
            return;
        }
        if !self.text.contains("%1") {
            let at = self.number_at.take().unwrap_or(usize::MAX).min(self.text.chars().count());
            let byte = self.text.char_indices().nth(at).map_or(self.text.len(), |(i, _)| i);
            self.text.insert_str(byte, "%1");
        }
        self.format = style.to_string();
    }

    pub fn params(&self) -> Value {
        json!({"format": self.format, "text": self.text, "align": self.align, "props": self.props})
    }

    /// The number's bold, italic and colour from its properties (for tests).
    #[cfg(test)]
    fn look(&self) -> (bool, bool, Option<wordcraft_doc::Rgb>) {
        let b = |k: &str| self.props.get(k).and_then(Value::as_bool).unwrap_or(false);
        let rgb = self.props.get("color").and_then(|c| c.get("Rgb")).and_then(Value::as_array).and_then(|c| match c.as_slice() {
            [r, g, b] => Some(wordcraft_doc::Rgb(r.as_u64()? as u8, g.as_u64()? as u8, b.as_u64()? as u8)),
            _ => None,
        });
        (b("bold"), b("italic"), rgb)
    }

    /// The number's size in the preview: the Font dialog's size, 12 pt without one, clamped to
    /// `6..=36 pt` while the preview box grows vertically to hold all three rows.
    fn preview_size(&self) -> f32 {
        let size = self.props.get("size").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(12.0);
        size.clamp(6.0, 36.0) as f32
    }
}

/// Number styles Define New Number Format offers: the multilevel list's without bullet (none last).
fn number_styles() -> impl Iterator<Item = (&'static str, &'static str)> {
    STYLES.into_iter().filter(|(k, _)| *k != "bullet")
}

fn rgba_c32(c: craft::theme::Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

fn set_text_cursor(ctx: &egui::Context, id: egui::Id, char_idx: usize) {
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(char_idx))));
    state.store(ctx, id);
}

/// The number format box as Word shows it: the text before the number, the number itself (the
/// style's first value on a highlighted background, not typed over), the text after it. Clicking
/// before the number or moving left from the text after it moves focus before the number so prefix
/// text can be typed freely. Delete or Backspace on the number, Delete at the end of the text
/// before it or Backspace at the start of the text after it deletes the number, and the style
/// becomes none: then the box is plain text.
fn number_format_box(ui: &mut Ui, field_id: egui::Id, f: &mut NumberForm) {
    let tokens = Env::of(ui).tokens.clone();
    let min_width = tokens.size.length.field.md.max(180.0);
    let width = if ui.is_sizing_pass() { min_width } else { ui.available_width().max(min_width) };
    let height = tokens.size.control.md.max(ui.spacing().interact_size.y);
    if f.format == "none" || !f.text.contains("%1") {
        TextField::new(word(tl!("Number format:")), Bind::bound(&mut f.text)).sizing(Sizing::FillWidth).show(ui);
        if f.text.chars().count() > 64 {
            f.text = f.text.chars().take(64).collect();
        }
        return;
    }
    let (outer, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if ui.is_sizing_pass() {
        return;
    }
    let colors = tokens.color;
    let bg = rgba_c32(colors.surface.sunken);
    let border = rgba_c32(colors.line.control_edge);
    let text_color = rgba_c32(colors.text.normal);
    let chip_bg = rgba_c32(colors.state.selected);
    let chip_fg = rgba_c32(colors.text.on_selected);
    let focus_bg = rgba_c32(colors.accent.strong);
    let focus_fg = rgba_c32(colors.text.on_strong);
    let sel_bg = rgba_c32(colors.state.text_selection);
    let radius = tokens.shape.radius.control;
    ui.painter().rect_filled(outer, radius, bg);
    ui.painter().rect_stroke(outer, radius, egui::Stroke::new(tokens.stroke.thin, border), egui::StrokeKind::Inside);

    let (mut before, mut after) = f.text.split_once("%1").map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or_default();
    let font = egui::FontId::proportional(tokens.typography.body.size);
    let measure = |ui: &Ui, s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), text_color).size().x;
    let pressed = |ui: &Ui, k: egui::Key| ui.input(|i| i.key_pressed(k));
    let cursor =
        |ui: &Ui, id: egui::Id| egui::text_edit::TextEditState::load(ui.ctx(), id).and_then(|s| s.cursor.char_range()).map(|r| r.primary.index.0);
    let mut remove = false;
    let pad_x = tokens.space.pad.control;
    let inner = outer.shrink2(vec2(pad_x, 2.0));
    let click_pos = ui.input(|i| i.pointer.primary_clicked().then(|| i.pointer.interact_pos()).flatten());
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.visuals_mut().override_text_color = Some(text_color);
        ui.visuals_mut().text_cursor.stroke.color = text_color;
        ui.visuals_mut().selection.bg_fill = sel_bg;
        ui.visuals_mut().selection.stroke.color = text_color;
        ui.spacing_mut().item_spacing.x = 0.0;
        // Frameless pieces sit inside the painted field box, on one shared vertical centre line.
        ui.spacing_mut().interact_size.y = inner.height();
        let before_id = ui.scope_id().with("dnf_before");
        let after_id = ui.scope_id().with("dnf_after");
        let before_chars = before.chars().count();
        let at_before = cursor(ui, before_id);
        let at_after = cursor(ui, after_id);
        // Empty, leave a tiny caret slot so the number still sits near the box's start; clicking
        // anywhere in the left padding before the number also focuses this field (below).
        let w = if before.is_empty() { 2.0 } else { measure(ui, &before) + 2.0 };
        let before_resp = ui.add(
            egui::TextEdit::singleline(&mut before)
                .id(before_id)
                .frame(egui::Frame::NONE)
                .font(font.clone())
                .text_color(text_color)
                .align(egui::Align2::LEFT_CENTER)
                .char_limit(30)
                .desired_width(w)
                .margin(egui::Margin::ZERO),
        );
        ui.ctx().accesskit_node_builder(before_resp.id, |node| node.set_label(tl!("Text before the number")));
        if before_resp.has_focus() && pressed(ui, egui::Key::Delete) && at_before == Some(before_chars) {
            remove = true;
        }
        let sample = NumFormat::from_ooxml(&f.format).format(1);
        let sample_w = measure(ui, &sample);
        let (rect, num) = ui.allocate_exact_size(vec2(sample_w + 2.0, inner.height()), Sense::click());
        num.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Number")));
        if num.clicked() {
            num.request_focus();
        }
        // Clicking anywhere in the box to the left of the number focuses the prefix field so users
        // can type before `1.` without having to hit a 2 px sliver.
        if let Some(pos) = click_pos
            && outer.contains(pos)
            && pos.x < rect.left()
        {
            ui.memory_mut(|m| m.request_focus(before_id));
            set_text_cursor(ui.ctx(), before_id, before.chars().count());
        }
        // Left and Right are this piece's own (below), not egui's move-focus keys.
        ui.memory_mut(|m| m.set_focus_lock_filter(num.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
        let (fill, chip_text) = if num.has_focus() { (focus_bg, focus_fg) } else { (chip_bg, chip_fg) };
        let galley = ui.painter().layout_no_wrap(sample, font.clone(), chip_text);
        let text_y = rect.center().y - galley.size().y / 2.0;
        craft::ui::layout::record_control_baseline(ui, field_id, (text_y - outer.top()) + galley.size().y * 0.8);
        let highlight = Rect::from_min_size(pos2(rect.left(), text_y - 1.0), vec2(galley.size().x + 2.0, galley.size().y + 2.0));
        ui.painter().rect_filled(highlight, 1.5, fill);
        ui.painter().galley(pos2(rect.left() + 1.0, text_y), galley, chip_text);
        if num.has_focus() && (pressed(ui, egui::Key::Delete) || pressed(ui, egui::Key::Backspace)) {
            remove = true;
        }
        // Arrow/Home/End navigation across `before` <-> `Number` <-> `after`.
        let mut focus_num = false;
        if before_resp.has_focus() {
            if pressed(ui, egui::Key::ArrowRight) && at_before == Some(before_chars) {
                focus_num = true;
            } else if pressed(ui, egui::Key::End) {
                ui.memory_mut(|m| m.request_focus(after_id));
                set_text_cursor(ui.ctx(), after_id, after.chars().count());
            }
        } else if num.has_focus() {
            if pressed(ui, egui::Key::ArrowLeft) {
                ui.memory_mut(|m| m.request_focus(before_id));
                set_text_cursor(ui.ctx(), before_id, before.chars().count());
            } else if pressed(ui, egui::Key::Home) {
                ui.memory_mut(|m| m.request_focus(before_id));
                set_text_cursor(ui.ctx(), before_id, 0);
            } else if pressed(ui, egui::Key::ArrowRight) {
                ui.memory_mut(|m| m.request_focus(after_id));
                set_text_cursor(ui.ctx(), after_id, 0);
            } else if pressed(ui, egui::Key::End) {
                ui.memory_mut(|m| m.request_focus(after_id));
                set_text_cursor(ui.ctx(), after_id, after.chars().count());
            }
        }
        let after_resp = ui.add(
            egui::TextEdit::singleline(&mut after)
                .id(after_id)
                .frame(egui::Frame::NONE)
                .font(font.clone())
                .text_color(text_color)
                .align(egui::Align2::LEFT_CENTER)
                .char_limit(30)
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::ZERO),
        );
        ui.ctx().accesskit_node_builder(after_resp.id, |node| node.set_label(tl!("Text after the number")));
        if after_resp.has_focus() {
            if pressed(ui, egui::Key::Backspace) && at_after == Some(0) {
                remove = true;
            } else if pressed(ui, egui::Key::ArrowLeft) && at_after == Some(0) {
                focus_num = true;
            } else if pressed(ui, egui::Key::Home) {
                ui.memory_mut(|m| m.request_focus(before_id));
                set_text_cursor(ui.ctx(), before_id, 0);
            }
        }
        if focus_num {
            num.request_focus();
        }
    });
    f.text = format!("{before}%1{after}");
    if remove {
        f.remove_number();
    }
}

/// Preview height in logical points, growing with the chosen font size so all three real-engine
/// list rows and context bars have comfortable breathing room.
fn preview_height(f: &NumberForm) -> f32 {
    let row_pitch = (f.preview_size() * 1.45).max(20.0);
    (3.0 * row_pitch + 48.0).clamp(112.0, 216.0)
}

/// Real `CharProps` for the preview, parsed directly from `NumberForm.props` so font family,
/// size, colour, bold, italic, underline, strikethrough, caps and spacing render as in the
/// document.
fn preview_chr(f: &NumberForm) -> wordcraft_doc::props::CharProps {
    let mut chr: wordcraft_doc::props::CharProps = serde_json::from_value(Value::Object(f.props.clone())).unwrap_or_default();
    if chr.size.is_some() {
        chr.size = Some(f.preview_size());
    }
    chr.hidden = None;
    chr
}

/// Cache key for [`show_number_preview`].
fn number_preview_key(format: &str, text: &str, props: &serde_json::Map<String, Value>, align: &str, w: f32, h: f32, ppp: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "defineNumber-preview".hash(&mut hasher);
    format.hash(&mut hasher);
    text.hash(&mut hasher);
    serde_json::to_string(props).unwrap_or_default().hash(&mut hasher);
    align.hash(&mut hasher);
    w.round().to_bits().hash(&mut hasher);
    h.round().to_bits().hash(&mut hasher);
    ppp.to_bits().hash(&mut hasher);
    wordcraft_fonts::FontDb::global().generation().hash(&mut hasher);
    hasher.finish()
}

/// Renders three numbered paragraphs through the real layout/render engine (`previews::number_format_preview`)
/// at device pixel density, growing taller when a larger number font size is chosen.
fn show_number_preview(ui: &mut Ui, f: &NumberForm, target_w: f32) {
    let format = f.format.clone();
    let text = f.text.clone();
    let align = f.align.clone();
    let chr = preview_chr(f);
    let h = preview_height(f);
    let ppp = ui.ctx().pixels_per_point();
    let key = number_preview_key(&format, &text, &f.props, &align, target_w, h, ppp);
    let maker = move || crate::previews::number_format_preview(&format, &text, &chr, &align, (target_w, h, ppp));
    let aspect = ThumbAspect::new(target_w.round().clamp(1.0, f32::from(u16::MAX)) as u16, h.round().clamp(1.0, f32::from(u16::MAX)) as u16);
    Picture::new(word(tl!("Preview")), key, &maker).aspect(aspect).extent(PictureSize::DevicePixels).show(ui);
}

impl WordForm for NumberForm {
    fn key(&self) -> &'static str {
        "defineNumber"
    }

    fn title(&self) -> Label {
        word(tl!("Define New Number Format"))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let styles: Vec<(&'static str, &'static str)> = number_styles().collect();
        let style_entries: Vec<Entry<usize>> =
            styles.iter().enumerate().map(|(at, (_, s))| Entry::new(at, if s.is_empty() { word(tl!("(none)")) } else { word(tl!(s)) })).collect();
        let current = styles.iter().position(|(k, _)| *k == self.format);
        let shown = current.map_or(Mixed::Mixed, Mixed::Same);
        let top = FieldGroup::new(word(tl!("Number format"))).show(ui, |ui, _| {
            Field::new(word(tl!("Number style:"))).id_salt("dnf_style").show(ui, |ui, _| {
                Row::new().fill_width().show(ui, |ui, _| {
                    if let Some(at) = Select::new(word(tl!("Number style:")), Bind::given(shown), &style_entries)
                        .sizing(Sizing::FillWidth)
                        .show(ui)
                        .value()
                        .copied()
                        && let Some((k, _)) = styles.get(at)
                    {
                        self.set_style(k);
                    }
                    // Word's Font… beside the style: the app's own Font dialog formatting the number.
                    if clicked(&Button::new(word(tl!("Font…"))).show(ui)) {
                        asks.push(Asks::Font(FontRequest::number(self.clone())));
                    }
                });
            });
            Field::new(word(tl!("Number format:"))).id_salt("dnf_format").show(ui, |ui, cx| {
                number_format_box(ui, cx.resolved.id, self);
            });
            Field::new(word(tl!("Alignment:"))).id_salt("dnf_align").show(ui, |ui, _| {
                let group = RadioGroup::new(word(tl!("Alignment:")), Bind::given(self.align.clone())).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        for (v, s) in [("left", "Left"), ("center", "Center"), ("right", "Right")] {
                            Radio::new(v.to_string()).label(word(tl!(s))).show(ui);
                        }
                    });
                });
                if let Some(a) = group.output.value().cloned() {
                    self.align = a;
                }
            });
        });
        let width_key = ui.scope_id().with("dnf_preview_width");
        if !ui.is_sizing_pass() {
            let w = top.output.rect.width();
            if w > 100.0 {
                ui.ctx().data_mut(|d| d.insert_temp(width_key, w));
            }
        }
        section(ui, word(tl!("Preview")), |ui| {
            let least = Env::of(ui).tokens.size.dialog.least.max(240.0);
            let w = ui.ctx().data(|d| d.get_temp::<f32>(width_key)).unwrap_or(least).max(least);
            show_number_preview(ui, self, w);
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        // para.defineNumber checks the format: a refused one keeps the dialog open with its reason.
        vec![("para.defineNumber".into(), self.params())]
    }

    fn answer(&mut self, answer: ChildAnswer) {
        if let ChildAnswer::Font(font) = answer {
            font.apply_to_number_props(&mut self.props);
        }
    }
}

/// Track Changes Options (Review › Tracking): what markup shows and how revisions look.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackForm {
    pub comments: bool,
    pub ink: bool,
    pub insertions_deletions: bool,
    pub formatting: bool,
    pub balloons: String,
    pub comment_display: String,
    pub insert_mark: String,
    /// Hex, empty = by author.
    pub insert_color: String,
    pub delete_mark: String,
    pub delete_color: String,
    pub changed_lines: String,
    /// Hex, empty = automatic.
    pub changed_lines_color: String,
    pub track_formatting: bool,
    pub move_from_mark: String,
    /// Hex, empty = by author.
    pub move_from_color: String,
    pub move_to_mark: String,
    pub move_to_color: String,
    /// The colours the colour fields offer, filled every pass by `prepare`.
    // PLAN: assumed `Palette` derives Clone, Debug, Default and PartialEq (ribbon-w11, S-L284)
    #[serde(skip)]
    palette: Palette,
}

impl TrackForm {
    pub fn read(app: &WordApp) -> TrackForm {
        let v = wordcraft_engine::cmd::review::tracking_json(&app.session.prefs.markup);
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let c = |k: &str| wordcraft_doc::Rgb::parse(&s(k)).map(|c| c.hex()).unwrap_or_default();
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(true);
        TrackForm {
            comments: b("comments"),
            ink: b("ink"),
            insertions_deletions: b("insertionsDeletions"),
            formatting: b("formatting"),
            balloons: s("balloons"),
            comment_display: s("commentDisplay"),
            insert_mark: s("insertMark"),
            insert_color: c("insertColor"),
            delete_mark: s("deleteMark"),
            delete_color: c("deleteColor"),
            changed_lines: s("changedLines"),
            changed_lines_color: c("changedLinesColor"),
            track_formatting: b("trackFormatting"),
            move_from_mark: s("moveFromMark"),
            move_from_color: c("moveFromColor"),
            move_to_mark: s("moveToMark"),
            move_to_color: c("moveToColor"),
            palette: Palette::default(),
        }
    }

    /// `review.trackingOptions` parameters.
    pub fn params(&self) -> Value {
        let or = |c: &str, auto: &str| if c.is_empty() { auto.to_string() } else { c.to_string() };
        json!({
            "comments": self.comments,
            "ink": self.ink,
            "insertionsDeletions": self.insertions_deletions,
            "formatting": self.formatting,
            "balloons": self.balloons,
            "commentDisplay": self.comment_display,
            "insertMark": self.insert_mark,
            "insertColor": or(&self.insert_color, "byAuthor"),
            "deleteMark": self.delete_mark,
            "deleteColor": or(&self.delete_color, "byAuthor"),
            "changedLines": self.changed_lines,
            "changedLinesColor": or(&self.changed_lines_color, "auto"),
            "trackFormatting": self.track_formatting,
            "moveFromMark": self.move_from_mark,
            "moveFromColor": or(&self.move_from_color, "byAuthor"),
            "moveToMark": self.move_to_mark,
            "moveToColor": or(&self.move_to_color, "byAuthor"),
        })
    }
}

/// How insertions (and moved text where it went) can be marked: (value, label).
const INSERT_MARKS: &[(&str, &str)] = &[
    ("underline", "Underline"),
    ("doubleUnderline", "Double Underline"),
    ("bold", "Bold"),
    ("italic", "Italic"),
    ("strikethrough", "Strikethrough"),
    ("colorOnly", "Color only"),
    ("none", "None"),
];

/// How deletions (and the place moved text was taken from) can be marked, in Word for Mac's
/// order: (value, label).
const DELETE_MARKS: &[(&str, &str)] = &[
    ("none", "(none)"),
    ("colorOnly", "Color only"),
    ("bold", "Bold"),
    ("italic", "Italic"),
    ("underline", "Underline"),
    ("doubleUnderline", "Double Underline"),
    ("strikethrough", "Strikethrough"),
    ("hidden", "Hidden"),
    ("caret", "^"),
    ("hash", "#"),
    ("doubleStrikethrough", "Double Strikethrough"),
];

/// How moved text is marked where it went, in Word for Mac's order: (value, label).
const MOVE_TO_MARKS: &[(&str, &str)] = &[
    ("none", "(none)"),
    ("colorOnly", "Color only"),
    ("bold", "Bold"),
    ("italic", "Italic"),
    ("underline", "Underline"),
    ("doubleUnderline", "Double Underline"),
    ("strikethrough", "Strikethrough"),
];

/// The balloon choices, the comment display choices and the changed-line choices: (value, label).
const BALLOONS: &[(&str, &str)] = &[
    ("revisions", "Show revisions in balloons"),
    ("inline", "Show all revisions inline"),
    ("commentsAndFormatting", "Show only comments and formatting in balloons"),
];
const COMMENT_DISPLAYS: &[(&str, &str)] = &[("margin", "In the margin"), ("selected", "Only for the selected text")];
const CHANGED_LINES: &[(&str, &str)] = &[("outside", "Outside border"), ("left", "Left border"), ("right", "Right border"), ("none", "None")];

/// The choice `value` of `options` ((value, label)) as the index of a drop-down or radio group over their labels.
fn index_of(options: &[(&str, &str)], value: &str) -> Option<usize> {
    options.iter().position(|(v, _)| *v == value)
}

fn labels(options: &[(&str, &str)]) -> Vec<Label> {
    options.iter().map(|(_, text)| word(tl!(text))).collect()
}

/// A drop-down over `options` setting `value`.
fn choose(ui: &mut Ui, salt: &'static str, label: Label, options: &[(&str, &str)], value: &mut String) {
    if let Some((v, _)) = pick(ui, salt, label, &labels(options), index_of(options, value), true).and_then(|at| options.get(at)) {
        *value = v.to_string();
    }
}

/// A radio group over `options` setting `value`.
fn choose_radio(ui: &mut Ui, label: Label, options: &[(&str, &str)], value: &mut String) {
    let current = index_of(options, value).unwrap_or(0);
    if let Some((v, _)) = radios(ui, label, &labels(options), current).and_then(|at| options.get(at)) {
        *value = v.to_string();
    }
}

impl WordForm for TrackForm {
    fn key(&self) -> &'static str {
        "trackChangesOptions"
    }

    fn title(&self) -> Label {
        word(tl!("Track Changes Options"))
    }

    // PLAN: assumed `WordForm::prepare(&mut self, &WordApp)` and `Palette::of(app)` (dialogs-w13 S-L283, ribbon-w11 S-L284)
    fn prepare(&mut self, app: &WordApp) {
        self.palette = Palette::of(app);
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let palette = self.palette.clone();
        let by_author = |name: &str| ColorFieldEmpty::Automatic {
            label: word(name),
            value: "",
            face: AutoFace::Several((0..3).map(|k| srgba(wordcraft_layout::display::revision_color(k))).collect()),
        };
        let automatic = ColorFieldEmpty::Automatic {
            label: word(tl!("Automatic")),
            value: "",
            face: AutoFace::One(srgba(wordcraft_layout::display::CHANGE_BAR)),
        };
        section(ui, word(tl!("Show")), |ui| {
            toggle(ui, word(tl!("Comments")), &mut self.comments, true);
            toggle(ui, word(tl!("Ink")), &mut self.ink, true);
            toggle(ui, word(tl!("Insertions and Deletions")), &mut self.insertions_deletions, true);
            toggle(ui, word(tl!("Formatting")), &mut self.formatting, true);
        });
        section(ui, word(tl!("Balloons")), |ui| choose_radio(ui, word(tl!("Balloons")), BALLOONS, &mut self.balloons));
        section(ui, word(tl!("Comments")), |ui| choose_radio(ui, word(tl!("Comments")), COMMENT_DISPLAYS, &mut self.comment_display));
        section(ui, word(tl!("Advanced")), |ui| {
            choose(ui, "insert-mark", word(tl!("Insertions:")), INSERT_MARKS, &mut self.insert_mark);
            color_row(ui, "insert", &word(tl!("Color:")), &mut self.insert_color, &by_author(tl!("By author")), &palette, asks);
            choose(ui, "delete-mark", word(tl!("Deletions:")), DELETE_MARKS, &mut self.delete_mark);
            color_row(ui, "delete", &word(tl!("Color:")), &mut self.delete_color, &by_author(tl!("By author")), &palette, asks);
            choose(ui, "changed-lines", word(tl!("Changed lines:")), CHANGED_LINES, &mut self.changed_lines);
            color_row(ui, "lines", &word(tl!("Color:")), &mut self.changed_lines_color, &automatic, &palette, asks);
        });
        section(ui, word(tl!("Moves")), |ui| {
            choose(ui, "move-from-mark", word(tl!("Moved from:")), DELETE_MARKS, &mut self.move_from_mark);
            color_row(ui, "moveFrom", &word(tl!("Color:")), &mut self.move_from_color, &by_author(tl!("By author")), &palette, asks);
            choose(ui, "move-to-mark", word(tl!("Moved to:")), MOVE_TO_MARKS, &mut self.move_to_mark);
            color_row(ui, "moveTo", &word(tl!("Color:")), &mut self.move_to_color, &by_author(tl!("By author")), &palette, asks);
        });
        toggle(ui, word(tl!("Track formatting")), &mut self.track_formatting, true);
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("review.trackingOptions".to_string(), self.params())]
    }

    fn answer(&mut self, answer: ChildAnswer) {
        if let ChildAnswer::Color { field, hex } = answer {
            match field {
                "insert" => self.insert_color = hex,
                "delete" => self.delete_color = hex,
                "lines" => self.changed_lines_color = hex,
                "moveFrom" => self.move_from_color = hex,
                "moveTo" => self.move_to_color = hex,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::super::host::FormHost;
    use super::super::host::testing::{confirm, harness};
    use super::{DELETE_MARKS, INSERT_MARKS, ListForm, MOVE_TO_MARKS, TrackForm};
    use crate::{Services, WordApp};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    /// The id of the open dialog `key`: the one the command opened (a command that answers `pending` has
    /// handled its request, so the dialog is already open and a second `open` would stack another on it).
    fn opened(app: &WordApp, key: &str) -> craft::ui::dialog::DialogId {
        app.dialog_host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)).map(|d| d.id).expect("the command opened the dialog")
    }

    fn number_form(app: &WordApp, id: craft::ui::dialog::DialogId) -> &super::NumberForm {
        app.dialog_host.get_by_id::<FormHost<super::NumberForm>>(id).expect("Define New Number Format is not open").form()
    }

    fn number_form_mut(app: &mut WordApp, id: craft::ui::dialog::DialogId) -> &mut super::NumberForm {
        app.dialog_host.get_mut_by_id::<FormHost<super::NumberForm>>(id).expect("Define New Number Format is not open").form_mut()
    }

    /// #2787: Define New Number Format opens from the Numbering drop-down's command without
    /// settings, on `1.`, `2.`; the preview draws the number at the Font dialog's size, within
    /// what its rows hold.
    #[test]
    fn define_new_number_format_opens_on_a_decimal_number() {
        let mut h = harness(app());
        assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
        let id = opened(h.state(), "defineNumber");
        h.run_steps(2);
        let form = number_form(h.state(), id);
        assert_eq!((form.sample(1), form.sample(2)), ("1.".to_string(), "2.".to_string()));
        let mut sized = form.clone();
        assert_eq!(sized.preview_size(), 12.0);
        let base_h = super::preview_height(&sized);
        for (size, want) in [(json!(16.0), 16.0), (json!(24.0), 24.0), (json!(400.0), 36.0), (json!(1.0), 6.0), (json!("big"), 12.0)] {
            sized.props.insert("size".into(), size);
            assert_eq!(sized.preview_size(), want);
        }
        sized.props.insert("size".into(), json!(28.0));
        sized.props.insert("font".into(), json!("Georgia"));
        sized.props.insert("underline".into(), json!("single"));
        assert!(super::preview_height(&sized) > base_h, "preview grows taller for larger font sizes");
        let chr = super::preview_chr(&sized);
        assert_eq!(chr.font.as_deref(), Some("Georgia"));
        assert_eq!(chr.size, Some(28.0));
        assert!(chr.underline.is_some());
    }

    /// #2787: the number in the format box deletes as one piece (style none) and comes back
    /// where it stood with a number style; Font… swaps to the app's Font dialog for the number
    /// (OK keeps its formatting, Escape doesn't, neither formats text); OK numbers the paragraph
    /// in one step, a none format with its text; Cancel changes nothing.
    #[test]
    fn define_new_number_format_dialog_applies_only_a_valid_format() {
        use craft::ui::dialog::HeldOk;
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("text.insert", json!({"text": "Clause"})).unwrap();
        let doc = a.session.doc.clone();
        let mut h = harness(a);
        assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
        let id = opened(h.state(), "defineNumber");
        {
            let form = number_form_mut(h.state_mut(), id);
            form.text = "(%1)".into();
            form.format = "lowerLetter".into();
        }
        h.run_steps(4);
        assert_eq!(number_form(h.state(), id).sample(2), "(b)");
        // The number in the format box is one piece: Backspace on it deletes it, and the style is
        // none (Word's "(none)"), the box plain text.
        h.get_by_label("Number").click();
        h.run_steps(3);
        h.key_press(egui::Key::Backspace);
        h.run_steps(3);
        assert_eq!((number_form(h.state(), id).format.as_str(), number_form(h.state(), id).text.as_str()), ("none", "()"));
        assert!(h.query_by_label("Number").is_none(), "no number left in the box");
        // Picking a number style again puts the number back where it stood.
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Number style:").click();
        h.run_steps(3);
        h.get_by_label("a, b, c, …").click();
        h.run_steps(3);
        assert_eq!((number_form(h.state(), id).format.as_str(), number_form(h.state(), id).text.as_str()), ("lowerLetter", "(%1)"));
        // From the number, Left goes to the text before it: typing lands there.
        h.get_by_label("Number").click();
        h.run_steps(3);
        h.key_press(egui::Key::ArrowLeft);
        h.run_steps(3);
        h.input_mut().events.push(egui::Event::Text("x".into()));
        h.run_steps(3);
        assert_eq!(number_form(h.state(), id).text, "(x%1)");
        {
            let form = number_form_mut(h.state_mut(), id);
            form.text = "(%1)".into();
            form.align = "right".into();
        }
        h.run_steps(3);
        // Font… switches to the app's Font dialog; Enter returns to Define New Number Format with
        // the number's formatting, and no text is formatted.
        h.get_by_label("Font…").click();
        h.run_steps(6);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::SpinButton, "Size:").is_some(), "the Font dialog is open");
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Bold").click();
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Italic").click();
        h.run_steps(3);
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        assert!(h.state().dialog_host.get_by_id::<FormHost<super::NumberForm>>(id).is_some(), "Define New Number Format is back");
        assert_eq!(number_form(h.state(), id).look(), (true, true, None));
        assert_eq!(h.state().session.doc, doc, "the Font dialog formatted no text");
        // Font… again, opening on the number's formatting, then Escape: nothing changes.
        h.get_by_label("Font…").click();
        h.run_steps(6);
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Bold").click();
        h.run_steps(3);
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert!(h.state().dialog_host.get_by_id::<FormHost<super::NumberForm>>(id).is_some(), "Define New Number Format is back");
        assert_eq!(number_form(h.state(), id).look(), (true, true, None), "Escape keeps the formatting as it was");
        confirm(&mut h, id);
        let st = h.state_mut();
        assert!(st.dialog_host.is_empty(), "OK closes it");
        let got = st.session.run("list.get", &json!({})).unwrap();
        let l = &got["levels"][0];
        assert_eq!((l["format"].as_str(), l["text"].as_str(), l["align"].as_str()), (Some("lowerLetter"), Some("(%1)"), Some("right")));
        assert_eq!((l["bold"].as_bool(), l["italic"].as_bool()), (Some(true), Some(true)));
        st.run("edit.undo", json!({})).unwrap();
        assert_eq!(st.session.doc, doc, "one undo step");
        drop(h);
        // "(none)": every label is the format's text.
        let mut c = app();
        c.run("text.insert", json!({"text": "Clause"})).unwrap();
        let mut h = harness(c);
        assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
        let id = opened(h.state(), "defineNumber");
        *number_form_mut(h.state_mut(), id) = super::NumberForm { format: "none".into(), text: "Note:".into(), ..Default::default() };
        h.run_steps(4);
        assert_eq!(number_form(h.state(), id).sample(1), "Note:");
        confirm(&mut h, id);
        let got = h.state_mut().session.run("list.get", &json!({})).unwrap();
        assert_eq!((got["levels"][0]["format"].as_str(), got["levels"][0]["text"].as_str()), (Some("none"), Some("Note:")));
        drop(h);
        // OK on formats without `%1` or with `%1` repeated: the dialog stays open with its refusal.
        for invalid in ["Item", "%1-%1"] {
            let mut d = app();
            d.run("text.insert", json!({"text": "Clause"})).unwrap();
            let doc = d.session.doc.clone();
            let mut h = harness(d);
            assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
            let id = opened(h.state(), "defineNumber");
            *number_form_mut(h.state_mut(), id) = super::NumberForm { text: invalid.into(), ..Default::default() };
            h.run_steps(4);
            confirm(&mut h, id);
            assert!(h.state().dialog_host.get_by_id::<FormHost<super::NumberForm>>(id).is_some(), "{invalid}: still open");
            assert_eq!(h.state().session.doc, doc, "{invalid}: changes nothing");
            let held = h.state().dialog_host.held_state(id);
            assert!(matches!(held, Some(HeldOk::Refused(ref m)) if m.as_str().contains("%1")), "{invalid}: {held:?}");
        }
        // Cancel: nothing happens.
        let mut b = app();
        b.run("text.insert", json!({"text": "Clause"})).unwrap();
        let doc = b.session.doc.clone();
        let mut h = harness(b);
        assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
        h.run_steps(4);
        h.get_by_label("Cancel").click();
        h.run_steps(4);
        assert!(h.state().dialog_host.is_empty());
        assert_eq!(h.state().session.doc, doc);
    }

    /// #2787: from the text after the number, Home or ArrowLeft crosses the number into the prefix
    /// field so users can type text before `1.`.
    #[test]
    fn number_format_box_moves_across_number_into_prefix() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(app());
        assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
        let id = opened(h.state(), "defineNumber");
        h.run_steps(4);
        h.get_by_label("Text after the number").click();
        h.run_steps(2);
        h.key_press(egui::Key::Home);
        h.run_steps(2);
        h.input_mut().events.push(egui::Event::Text("Step ".into()));
        h.run_steps(3);
        assert_eq!(number_form(h.state(), id).text, "Step %1.");
    }

    /// #2787: the number's Font… opens on the number's kerning, not the selected text's; a size
    /// the number had and the user cleared there goes, so the number takes the paragraph's.
    #[test]
    fn the_number_font_dialog_shows_the_kerning_and_clears_a_size() {
        use crate::dialogs::font::{FontForm, FontRequest};
        let mut a = app();
        a.run("text.insert", json!({"text": "Clause"})).unwrap();
        let mut with_kern = super::NumberForm::default();
        with_kern.props.insert("kern".into(), json!(18.0));
        let d = FontRequest::number(with_kern).into_form(&mut a);
        assert!((d.kern, d.kern_at) == (Some(true), Some(18.0)));
        let d = FontRequest::number(super::NumberForm::default()).into_form(&mut a);
        assert_eq!(d.kern, Some(false));
        // Size 14, cleared, then OK: the number's size goes.
        let mut h = harness(a);
        assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
        let id = opened(h.state(), "defineNumber");
        number_form_mut(h.state_mut(), id).props.insert("size".into(), json!(14.0));
        h.run_steps(3);
        use egui_kittest::kittest::Queryable;
        h.get_by_label("Font…").click();
        h.run_steps(6);
        let font_id = opened(h.state(), "font");
        h.state_mut().dialog_host.get_mut_by_id::<FormHost<FontForm>>(font_id).unwrap().form_mut().size = None;
        h.run_steps(2);
        confirm(&mut h, font_id);
        let form = number_form(h.state(), id);
        assert_eq!(form.props.get("size"), None, "{:?}", form.props);
    }

    /// #2787: closing or cancelling the number's Font dialog returns to Define New Number Format
    /// with the form unchanged.
    #[test]
    fn the_number_font_dialog_returns_on_close_and_cancel() {
        use egui_kittest::kittest::{NodeT, Queryable};
        for button in ["Cancel", "Close"] {
            let mut a = app();
            a.run("text.insert", json!({"text": "Clause"})).unwrap();
            let mut h = harness(a);
            assert_eq!(h.state_mut().run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
            let id = opened(h.state(), "defineNumber");
            h.run_steps(4);
            h.get_by_label("Font…").click();
            h.run_steps(6);
            h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Bold").click();
            h.run_steps(3);
            let shown = h
                .query_all_by_role_and_label(egui::accesskit::Role::Button, button)
                .find(|n| !n.accesskit_node().is_hidden())
                .unwrap_or_else(|| panic!("no shown {button} button"));
            shown.click();
            h.run_steps(4);
            assert!(h.state().dialog_host.get_by_id::<FormHost<super::NumberForm>>(id).is_some(), "{button} goes back to Define New Number Format");
            assert_eq!(number_form(h.state(), id).look(), (false, false, None), "{button} discards Font edits");
        }
    }

    /// #328: the ribbon opens Define New Multilevel List; OK defines the list and numbers the
    /// paragraph. Scripts without levels get an error, never the dialog.
    #[test]
    fn define_new_multilevel_list_opens_from_the_ribbon_and_applies() {
        let mut h = harness(app());
        h.state_mut().run("text.insert", json!({"text": "Intro"})).unwrap();
        assert_eq!(h.state_mut().run("list.define", json!({})).unwrap(), json!({"pending": "defineList"}));
        let id = opened(h.state(), "defineList");
        h.run_steps(2);
        let form = h.state_mut().dialog_host.get_mut_by_id::<FormHost<ListForm>>(id).expect("the form is hosted").form_mut();
        assert_eq!((form.levels.len(), form.sample(0), form.sample(1)), (9, "1.".to_string(), "a.".to_string()));
        form.levels[0].text = "Part %1:".into();
        form.levels[0].format = "upperRoman".into();
        form.levels[1].text = "%1.%2".into();
        form.levels[1].legal = true;
        assert_eq!((form.sample(0), form.sample(1)), ("Part I:".to_string(), "1.a".to_string()), "legal shows levels above in digits");
        confirm(&mut h, id);
        let got = h.state_mut().session.run("list.get", &json!({})).unwrap();
        assert_eq!((got["levels"][0]["text"].as_str(), got["levels"][0]["format"].as_str()), (Some("Part %1:"), Some("upperRoman")));
        // Opening it again starts from the caret's list.
        assert_eq!(h.state_mut().run("list.define", json!({})).unwrap(), json!({"pending": "defineList"}));
        let id = opened(h.state(), "defineList");
        h.run_steps(2);
        assert_eq!(h.state_mut().dialog_host.get_mut_by_id::<FormHost<ListForm>>(id).expect("the form is hosted").form().sample(0), "Part I:");
        let mut b = app();
        assert!(b.execute("list.define", json!({})).is_err());
        assert!(b.dialog_host.is_empty());
    }

    /// #328: the Tracking group's launcher opens Track Changes Options; OK changes the per-user
    /// preference that is saved with the interface settings.
    #[test]
    fn track_changes_options_open_from_the_launcher_and_persist() {
        let mut h = harness(app());
        assert_eq!(h.state_mut().run("review.trackingOptions", json!({})).unwrap(), json!({"pending": "trackChangesOptions"}));
        let id = opened(h.state(), "trackChangesOptions");
        h.run_steps(2);
        let form = h.state_mut().dialog_host.get_mut_by_id::<FormHost<TrackForm>>(id).expect("the form is hosted").form_mut();
        assert_eq!((form.insert_mark.as_str(), form.balloons.as_str(), form.insert_color.as_str()), ("underline", "commentsAndFormatting", ""));
        assert_eq!(form.comment_display, "margin", "comments stay in the margin, as in Word, unless chosen otherwise (#1055)");
        form.comment_display = "selected".into();
        form.insert_mark = "doubleUnderline".into();
        form.changed_lines = "right".into();
        form.changed_lines_color = "FF0000".into();
        confirm(&mut h, id);
        let a = h.state_mut();
        let m = &a.session.prefs.markup;
        assert_eq!(m.insert_mark, wordcraft_layout::display::InsertMark::DoubleUnderline);
        assert_eq!(m.comment_display, wordcraft_layout::display::CommentDisplay::Selected);
        assert_eq!(m.changed_lines_color, Some(wordcraft_doc::Rgb(0xFF, 0, 0)));
        // Saved and restored with the preferences.
        // PLAN: the round trip reads `Prefs` once shell-w13 lands it; the lines stay `prefs()`/`apply_prefs` until then.
        let saved = a.prefs();
        let mut b = app();
        b.apply_prefs(saved);
        assert_eq!(b.session.prefs.markup.changed_lines, wordcraft_layout::display::ChangeBar::Right);
    }

    /// Gap #5: the Moves rows (Moved from / Moved to: mark and colour) start at green double
    /// strikethrough and double underline, and OK applies them. There is no Track moves
    /// checkbox (as in Word for Mac), so OK leaves the document's setting alone.
    #[test]
    fn track_changes_options_moves_rows_apply_marks_and_colours() {
        use wordcraft_layout::display::{DeleteMark, InsertMark, MOVE_COLOR};
        let mut h = harness(app());
        h.state_mut().session.run("review.trackingOptions", &json!({"trackMoves": false})).unwrap();
        let id = crate::dialogs::open(h.state_mut(), "trackChangesOptions", &json!({})).expect("the dialog opens");
        h.run_steps(2);
        let form = h.state_mut().dialog_host.get_mut_by_id::<FormHost<TrackForm>>(id).expect("the form is hosted").form_mut();
        assert_eq!(
            (form.move_from_mark.as_str(), form.move_from_color.clone(), form.move_to_mark.as_str(), form.move_to_color.clone()),
            ("doubleStrikethrough", MOVE_COLOR.hex(), "doubleUnderline", MOVE_COLOR.hex())
        );
        form.move_from_mark = "strikethrough".into();
        form.move_from_color = "FF0000".into();
        form.move_to_mark = "bold".into();
        form.move_to_color = String::new();
        assert!(form.params().get("trackMoves").is_none(), "{}", form.params());
        confirm(&mut h, id);
        let a = h.state_mut();
        let m = &a.session.prefs.markup;
        assert_eq!((m.move_from_mark, m.move_from_color), (DeleteMark::Strikethrough, Some(wordcraft_doc::Rgb(0xFF, 0, 0))));
        assert_eq!((m.move_to_mark, m.move_to_color), (InsertMark::Bold, None), "empty = by author");
        assert!(!a.session.doc.settings.track_moves, "the document setting is untouched");
    }

    /// Gap #5: every mark the Insertions, Deletions, Moved from and Moved to lists offer is one the engine
    /// parses and `review.trackingOptions` accepts, so no choice of the dialog fails on OK.
    #[test]
    fn track_changes_options_mark_lists_follow_word() {
        use wordcraft_layout::display::{DeleteMark, InsertMark};
        assert!(DELETE_MARKS.iter().all(|(v, _)| DeleteMark::parse(v).is_some()));
        assert!(MOVE_TO_MARKS.iter().chain(INSERT_MARKS).all(|(v, _)| InsertMark::parse(v).is_some()));
        let mut a = app();
        for (v, _) in DELETE_MARKS {
            a.run("review.trackingOptions", json!({"deleteMark": v, "moveFromMark": v})).unwrap();
        }
        for (v, _) in INSERT_MARKS {
            a.run("review.trackingOptions", json!({"insertMark": v})).unwrap();
        }
        for (v, _) in MOVE_TO_MARKS {
            a.run("review.trackingOptions", json!({"moveToMark": v})).unwrap();
        }
    }

    /// #7325: the Define New Multilevel List dialog preview renders an opaque white paper sheet
    /// with a crisp stroke outline across both Light and Dark themes, and renders level samples
    /// and text bars in fixed dark ink.
    #[test]
    fn define_new_multilevel_list_preview_opaque_paper_in_both_themes() {
        use egui::epaint::ClippedShape;

        fn collect_shapes<'a>(shapes: &'a [ClippedShape], out: &mut Vec<&'a egui::Shape>) {
            for clipped in shapes {
                fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
                    match shape {
                        egui::Shape::Vec(children) => {
                            for child in children {
                                walk(child, out);
                            }
                        }
                        other => out.push(other),
                    }
                }
                walk(&clipped.shape, out);
            }
        }

        for theme in ["light", "dark"] {
            let mut a = app();
            a.run("ui.theme", json!({"value": theme})).unwrap();
            let mut h = harness(a);
            assert_eq!(h.state_mut().run("list.define", json!({})).unwrap(), json!({"pending": "defineList"}));
            h.run_steps(2);

            let mut shapes = Vec::new();
            collect_shapes(&h.output().shapes, &mut shapes);

            // Verify the preview paper sheet is painted as an opaque white rect
            let white_rects: Vec<_> = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Rect(r)
                        if r.fill == egui::Color32::WHITE && (r.rect.width() - 80.0).abs() <= 5.0 && (r.rect.height() - 48.0).abs() <= 5.0 =>
                    {
                        Some(r)
                    }
                    _ => None,
                })
                .collect();
            assert!(!white_rects.is_empty(), "{theme}: preview sheet must have an opaque white paper background");

            // Verify crisp stroke border (StrokeKind::Inside with gray 160)
            let stroke_rects: Vec<_> = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Rect(r)
                        if r.stroke.color == egui::Color32::from_gray(160)
                            && (r.rect.width() - 80.0).abs() <= 5.0
                            && (r.rect.height() - 48.0).abs() <= 5.0 =>
                    {
                        Some(r)
                    }
                    _ => None,
                })
                .collect();
            assert!(!stroke_rects.is_empty(), "{theme}: preview sheet must have a crisp stroke border");

            // Verify sample text has black override text color in both themes
            let black_text_samples: Vec<_> = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::Text(t) if t.override_text_color == Some(egui::Color32::BLACK) => Some(t),
                    _ => None,
                })
                .collect();
            assert!(!black_text_samples.is_empty(), "{theme}: preview text samples must override text color with black");
        }
    }
}
