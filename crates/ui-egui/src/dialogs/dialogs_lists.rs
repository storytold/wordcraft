//! Define New Multilevel List (Home › Paragraph › Multilevel List) and Track Changes Options
//! (Review › Tracking). Each fills a form from the session and ends in one command
//! (`list.define`, `review.trackingOptions`), so agents get the same result without the dialog.

use craft::behavior::{EditEvents, Selection, SelectionChange, keyed};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::primitives::{ContentPainterCx, TextRun};
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::sections::Section;
use craft::ui::style::{Corners, SurfaceKind, TypeRole};
use craft::ui::text::{NumberField, TextField};
use egui::{Rect, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::section::NumFormat;
use wordcraft_engine::cmd::lists::{LEVELS, caret_list, level_json, levels_of};

use super::dialogs_para::{color_row, pick, radios, srgba, word};
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

/// Opens Define New Multilevel List (`defineList`) or Track Changes Options (`trackChangesOptions`). `None` for
/// another name.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "defineList" => Some(open_form(app, parent, ListForm::read(app))),
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
                    if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
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
        cx.surface(rect, SurfaceKind::Sunken, Corners::All);
        let row = rect.height() / LEVELS as f32;
        // Positions are points, scaled so 4.5 inches fit.
        let scale = (rect.width() - 16.0).max(1.0) / 324.0;
        let x_of = |points: f32| rect.left() + 8.0 + (points * scale).clamp(0.0, (rect.width() - 40.0).max(0.0));
        for (i, r) in rows.iter().enumerate() {
            let top = rect.top() + row * i as f32;
            let paint = |cx: &ContentPainterCx| {
                let run = TextRun::new(r.sample.clone()).role(TypeRole::Caption);
                let width = cx.with_draw(|draw| draw.layout(&run, f32::INFINITY).size().x);
                let x = x_of(r.aligned_at);
                let number_x = match r.align.as_str() {
                    "center" => x - width / 2.0,
                    "right" => x - width,
                    _ => x,
                };
                cx.with_draw(|draw| draw.text(Rect::from_min_size(pos2(number_x, top), vec2(width + 1.0, row)), &run));
                let text_x = match r.follow.as_str() {
                    "space" => number_x + width + 3.0,
                    "nothing" => number_x + width,
                    _ => x_of(r.indent_at).max(number_x + width + 2.0),
                };
                let bar = Rect::from_min_max(pos2(text_x, top + row / 2.0 - 2.0), pos2(rect.right() - 8.0, top + row / 2.0 + 2.0));
                if bar.width() > 0.0 {
                    let draw_bar = |cx: &ContentPainterCx| cx.data_shapes([egui::Shape::rect_filled(bar, 1.0, cx.ink())]);
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
}
