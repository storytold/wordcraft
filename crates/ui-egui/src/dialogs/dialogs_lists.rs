//! Define New Multilevel List (Home › Paragraph › Multilevel List) and Track Changes Options
//! (Review › Tracking). Each fills a form from the session and ends in one command
//! (`list.define`, `review.trackingOptions`), so agents get the same result without the dialog.

use egui::{Sense, Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::section::NumFormat;
use wordcraft_engine::cmd::lists::{LEVELS, caret_list, level_json, levels_of};

use crate::WordApp;
use crate::dialogs::buttons;
use crate::options::MeasureExt;
use crate::theme::{Tokens, semibold};

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

/// One level of the Define New Multilevel List form. Positions are in the interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
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
        let k = crate::options::unit().pt_per_unit();
        let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let f = |key: &str| v.get(key).and_then(Value::as_f64).map(|x| x as f32 / k).filter(|x| x.is_finite());
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
        let k = crate::options::unit().pt_per_unit();
        json!({
            "format": self.format,
            "text": self.text,
            "start": self.start,
            "restartAfter": if !self.restart { json!(0) } else if i > 0 && (self.restart_after as usize) < i { json!(self.restart_after) } else { Value::Null },
            "style": if self.style.is_empty() { Value::Null } else { json!(self.style) },
            "align": self.align,
            "alignedAt": self.aligned_at * k,
            "indentAt": self.indent_at * k,
            "follow": self.follow,
            "tabAt": if self.tab_on && self.follow == "tab" { json!(self.tab_at * k) } else { Value::Null },
            "font": if self.font.trim().is_empty() { Value::Null } else { json!(self.font.trim()) },
            "bold": self.bold,
            "italic": self.italic,
            "legal": self.legal,
        })
    }
}

/// The Define New Multilevel List form: nine levels and the one being edited.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListForm {
    pub levels: Vec<LevelForm>,
    pub level: usize,
    /// Paragraph styles a level can link to: (id, name).
    #[serde(skip)]
    pub styles: Vec<(String, String)>,
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
        ListForm { levels, level: 0, styles }
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
}

/// Define New Multilevel List. Returns true to close.
pub fn define_list(app: &mut WordApp, ui: &mut Ui, f: &mut ListForm) -> bool {
    let unit = crate::options::unit();
    fn len(v: &mut f32, unit: wordcraft_geom::Unit) -> egui::DragValue<'_> {
        egui::DragValue::new(v).speed(0.01).range(-10.0..=22.0).measure(unit).max_decimals(2)
    }
    let heading = |ui: &mut Ui, s: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Click level to modify:"));
            for i in 0..LEVELS {
                if ui.selectable_label(f.level == i, format!("{}", i + 1)).clicked() {
                    f.level = i;
                }
            }
        });
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Preview")).small().weak());
            preview(ui, f);
        });
    });
    let i = f.level.min(LEVELS - 1);
    let styles = f.styles.clone();
    let Some(l) = f.levels.get_mut(i) else { return true };
    heading(ui, "Number format");
    egui::Grid::new("mll_number").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Enter formatting for number:"));
        ui.text_edit_singleline(&mut l.text);
        ui.end_row();
        ui.label(tl!("Number style for this level:"));
        let current = STYLES.iter().find(|(k, _)| *k == l.format).map(|(_, s)| *s).unwrap_or("");
        craft_ui::ComboBox::from_id_salt("mll_style").selected_text(style_label(current)).show_ui(ui, |ui| {
            for (k, s) in STYLES {
                if ui.selectable_label(l.format == k, style_label(s)).clicked() {
                    // A new style puts this level's number in the text if it had none (a bullet
                    // its character).
                    if k == "bullet" {
                        l.text = "•".into();
                    } else if l.format == "bullet" || !l.text.contains('%') {
                        l.text = format!("%{}.", i + 1);
                    }
                    l.format = k.to_string();
                }
            }
        });
        ui.end_row();
        if i > 0 {
            ui.label(tl!("Include level number from:"));
            craft_ui::ComboBox::from_id_salt("mll_include").selected_text("").show_ui(ui, |ui| {
                for k in 0..i {
                    if ui.selectable_label(false, crate::i18n::fmt(tl!("Level {n}"), &[("n", &(k + 1).to_string())])).clicked() {
                        l.text.push_str(&format!("%{}", k + 1));
                    }
                }
            });
            ui.end_row();
        }
        ui.label(tl!("Start at:"));
        ui.add(egui::DragValue::new(&mut l.start).range(0..=32_767));
        ui.end_row();
        if i > 0 {
            ui.checkbox(&mut l.restart, tl!("Restart list after:"));
            ui.add_enabled_ui(l.restart, |ui| {
                let shown = crate::i18n::fmt(tl!("Level {n}"), &[("n", &l.restart_after.clamp(1, i as u8).to_string())]);
                craft_ui::ComboBox::from_id_salt("mll_restart").selected_text(shown).show_ui(ui, |ui| {
                    for k in 1..=i as u8 {
                        ui.selectable_value(&mut l.restart_after, k, crate::i18n::fmt(tl!("Level {n}"), &[("n", &k.to_string())]));
                    }
                });
            });
            ui.end_row();
        }
        ui.label(tl!("Link level to style:"));
        let shown = styles.iter().find(|(id, _)| *id == l.style).map(|(_, n)| n.clone()).unwrap_or_else(|| tl!("(no style)").to_string());
        craft_ui::ComboBox::from_id_salt("mll_link").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut l.style, String::new(), tl!("(no style)"));
            for (id, name) in &styles {
                ui.selectable_value(&mut l.style, id.clone(), name);
            }
        });
        ui.end_row();
        ui.label(tl!("Font:"));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut l.font).desired_width(110.0));
            ui.checkbox(&mut l.bold, tl!("Bold"));
            ui.checkbox(&mut l.italic, tl!("Italic"));
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut l.legal, tl!("Legal style numbering"));
        ui.end_row();
    });
    heading(ui, "Position");
    egui::Grid::new("mll_position").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Number alignment:"));
        ui.horizontal(|ui| {
            for (v, s) in [("left", "Left"), ("center", "Center"), ("right", "Right")] {
                ui.radio_value(&mut l.align, v.to_string(), tl!(s));
            }
        });
        ui.end_row();
        ui.label(tl!("Aligned at:"));
        ui.add(len(&mut l.aligned_at, unit));
        ui.end_row();
        ui.label(tl!("Text indent at:"));
        ui.add(len(&mut l.indent_at, unit));
        ui.end_row();
        ui.label(tl!("Follow number with:"));
        let follow = |k: &str| match k {
            "space" => tl!("Space"),
            "nothing" => tl!("Nothing"),
            _ => tl!("Tab character"),
        };
        craft_ui::ComboBox::from_id_salt("mll_follow").selected_text(follow(&l.follow)).show_ui(ui, |ui| {
            for k in ["tab", "space", "nothing"] {
                ui.selectable_value(&mut l.follow, k.to_string(), follow(k));
            }
        });
        ui.end_row();
        ui.add_enabled(l.follow == "tab", egui::Checkbox::new(&mut l.tab_on, tl!("Add tab stop at:")));
        ui.add_enabled(l.follow == "tab" && l.tab_on, len(&mut l.tab_at, unit));
        ui.end_row();
    });
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Err(e) = app.run("list.define", f.params()) {
        app.status(e);
    }
    ok || cancel
}

fn style_label(s: &str) -> String {
    match s {
        "" => tl!("None").to_string(),
        "•" => format!("• {}", tl!("Bullet")),
        s => s.to_string(),
    }
}

/// All nine levels, each number at its aligned-at position followed by a grey line of text at
/// its text indent; the level being edited in the text colour.
fn preview(ui: &mut Ui, f: &ListForm) {
    let t = Tokens::get(ui.ctx());
    let row = 19.0;
    let (rect, _) = ui.allocate_exact_size(vec2(300.0, row * LEVELS as f32 + 8.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, egui::Color32::WHITE);
    p.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let k = crate::options::unit().pt_per_unit();
    // Positions in points, scaled so 4.5 inches fit.
    let scale = (rect.width() - 16.0) / 324.0;
    let x_of = |v: f32| rect.left() + 8.0 + (v * k * scale).clamp(0.0, rect.width() - 40.0);
    for (i, l) in f.levels.iter().enumerate() {
        let y = rect.top() + 4.0 + row * i as f32 + row / 2.0;
        let on = i == f.level;
        let ink = if on { egui::Color32::BLACK } else { egui::Color32::from_gray(150) };
        let label = f.sample(i);
        let font = egui::FontId::proportional(10.5);
        let galley = p.layout_no_wrap(label, font, ink);
        let w = galley.size().x;
        let x = x_of(l.aligned_at);
        let nx = match l.align.as_str() {
            "center" => x - w / 2.0,
            "right" => x - w,
            _ => x,
        };
        p.galley(egui::pos2(nx, y - galley.size().y / 2.0), galley, ink);
        let tx = match l.follow.as_str() {
            "space" => nx + w + 3.0,
            "nothing" => nx + w,
            _ => x_of(l.indent_at).max(nx + w + 2.0),
        };
        let bar = egui::Rect::from_min_max(egui::pos2(tx, y - 2.0), egui::pos2(rect.right() - 8.0, y + 2.0));
        if bar.width() > 0.0 {
            p.rect_filled(bar, 1.0, if on { egui::Color32::from_gray(120) } else { egui::Color32::from_gray(205) });
        }
    }
}

/// Define New Number Format (Home › Paragraph › Numbering, #2787): the number style, the number
/// format (`%1` = the number; none once the number is deleted), the number's font and its
/// alignment. OK runs one
/// `para.defineNumber`, so agents get the same list without the dialog.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberForm {
    pub format: String,
    pub text: String,
    pub align: String,
    /// The number's formatting from Font…, in `format.set`'s shape (font, size, bold, colour…).
    pub props: serde_json::Map<String, Value>,
    /// The app's Font dialog while Font… has it open over this one.
    #[serde(skip)]
    pub font_dialog: Option<Box<crate::dialogs::Dialog>>,
    /// Where the number stood (a char index of `text`) when it was deleted, so a number style
    /// picked again puts it back there.
    #[serde(skip)]
    pub number_at: Option<usize>,
}

impl Default for NumberForm {
    fn default() -> NumberForm {
        NumberForm {
            format: "decimal".into(),
            text: "%1.".into(),
            align: "left".into(),
            props: serde_json::Map::new(),
            font_dialog: None,
            number_at: None,
        }
    }
}

impl NumberForm {
    /// What `para.defineNumber` accepts: a number style's format holds the number (`%1`), a
    /// "none" one holds none, neither holds another level's, and it fits.
    /// The `n`th label, as the list will show it.
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

    /// The number's bold, italic and colour from its properties (for the preview).
    fn look(&self) -> (bool, bool, Option<wordcraft_doc::Rgb>) {
        let b = |k: &str| self.props.get(k).and_then(Value::as_bool).unwrap_or(false);
        let rgb = self.props.get("color").and_then(|c| c.get("Rgb")).and_then(Value::as_array).and_then(|c| match c.as_slice() {
            [r, g, b] => Some(wordcraft_doc::Rgb(r.as_u64()? as u8, g.as_u64()? as u8, b.as_u64()? as u8)),
            _ => None,
        });
        (b("bold"), b("italic"), rgb)
    }

    /// The number's size in the preview: the Font dialog's size, 12 pt without one, kept to what
    /// the preview's rows have room for.
    fn preview_size(&self) -> f32 {
        let size = self.props.get("size").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(12.0);
        size.clamp(6.0, 20.0) as f32
    }
}

/// Number styles Define New Number Format offers: the multilevel list's without bullet (none last).
fn number_styles() -> impl Iterator<Item = (&'static str, &'static str)> {
    STYLES.into_iter().filter(|(k, _)| *k != "bullet")
}

/// The number format box as Word shows it: the text before the number, the number itself (the
/// style's first value on grey, not typed over), the text after it. Delete or Backspace on the
/// number, Delete at the end of the text before it or Backspace at the start of the text after it
/// deletes the number, and the style becomes none: then the box is plain text.
fn number_format_box(ui: &mut Ui, f: &mut NumberForm) {
    let t = Tokens::get(ui.ctx());
    let width = 230.0;
    if f.format == "none" || !f.text.contains("%1") {
        let h = ui.spacing().interact_size.y;
        ui.add(egui::TextEdit::singleline(&mut f.text).char_limit(64).desired_width(width).min_size(vec2(0.0, h)))
            .accessible_name(tl!("Number format:"));
        return;
    }
    let (mut before, mut after) = f.text.split_once("%1").map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or_default();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let measure = |ui: &Ui, s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), t.text).size().x;
    let pressed = |ui: &Ui, k: egui::Key| ui.input(|i| i.key_pressed(k));
    let cursor =
        |ui: &Ui, id: egui::Id| egui::text_edit::TextEditState::load(ui.ctx(), id).and_then(|s| s.cursor.char_range()).map(|r| r.primary.index.0);
    let mut remove = false;
    // As tall as the Number style box beside it: a field's height less the frame's margin and border.
    let row = (ui.spacing().interact_size.y - 6.0).max(12.0);
    egui::Frame::new()
        .fill(t.input)
        .stroke(egui::Stroke::new(1.0, t.input_border))
        .corner_radius(3.0)
        .inner_margin(egui::Margin { left: 2, right: 4, top: 2, bottom: 2 })
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(vec2(width - 8.0, row), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_size(vec2(width - 8.0, row));
                ui.spacing_mut().item_spacing.x = 1.0;
                // A text field is at least a field's height (egui's theme floor): these frameless
                // pieces sit inside the box's frame, so they take the row's.
                ui.spacing_mut().interact_size.y = row;
                let id = ui.scope_id().with("dnf_before");
                let before_id = id;
                let at = cursor(ui, id);
                // Empty, it takes no room: the number starts the box, as in Word.
                let w = if before.is_empty() { 1.0 } else { measure(ui, &before) + 2.0 };
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut before)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .char_limit(30)
                            .desired_width(w)
                            .margin(egui::Margin::ZERO),
                    )
                    .accessible_name(tl!("Text before the number"));
                if r.has_focus() && pressed(ui, egui::Key::Delete) && at == Some(before.chars().count()) {
                    remove = true;
                }
                let sample = NumFormat::from_ooxml(&f.format).format(1);
                let (rect, num) = ui.allocate_exact_size(vec2(measure(ui, &sample) + 8.0, row), Sense::click());
                num.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Number")));
                if num.clicked() {
                    num.request_focus();
                }
                // Left and Right are this piece's own (below), not egui's move-focus keys.
                ui.memory_mut(|m| m.set_focus_lock_filter(num.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
                let fill = if num.has_focus() { t.selection } else { t.ruler_margin };
                ui.painter().rect_filled(rect, 2.0, fill);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &sample, font.clone(), t.text);
                if num.has_focus() && (pressed(ui, egui::Key::Delete) || pressed(ui, egui::Key::Backspace)) {
                    remove = true;
                }
                let id = ui.scope_id().with("dnf_after");
                // From the number, Left or Home goes to the text before it, Right or End to after.
                if num.has_focus() {
                    if pressed(ui, egui::Key::ArrowLeft) || pressed(ui, egui::Key::Home) {
                        ui.memory_mut(|m| m.request_focus(before_id));
                    } else if pressed(ui, egui::Key::ArrowRight) || pressed(ui, egui::Key::End) {
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                }
                let at = cursor(ui, id);
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut after)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .char_limit(30)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::ZERO),
                    )
                    .accessible_name(tl!("Text after the number"));
                if r.has_focus() && pressed(ui, egui::Key::Backspace) && at == Some(0) {
                    remove = true;
                }
            });
        });
    f.text = format!("{before}%1{after}");
    if remove {
        f.remove_number();
    }
}

pub fn define_number(app: &mut WordApp, ui: &mut Ui, f: &mut NumberForm) -> bool {
    // Font… open over this dialog: this one waits, as in Word.
    if f.font_dialog.is_some() {
        ui.disable();
    }
    ui.label(egui::RichText::new(tl!("Number format")).font(semibold(12.5)));
    egui::Grid::new("dnf_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let label = ui.label(tl!("Number style:"));
        ui.horizontal(|ui| {
            let current = number_styles().find(|(k, _)| *k == f.format).map(|(_, s)| s).unwrap_or("");
            let combo = craft_ui::ComboBox::from_id_salt("dnf_style").selected_text(style_label(current)).show_ui(ui, |ui| {
                for (k, s) in number_styles() {
                    if ui.selectable_label(f.format == k, style_label(s)).clicked() {
                        f.set_style(k);
                    }
                }
            });
            combo.response.labelled_by(label.id);
            // Word's Font… beside the style: the number's own font, in a dialog over this one.
            if ui.button(tl!("Font…")).clicked() {
                f.font_dialog = crate::dialogs::number_font_dialog(app, &f.props).map(Box::new);
            }
        });
        ui.end_row();
        ui.label(tl!("Number format:"));
        number_format_box(ui, f);
        ui.end_row();
        ui.label(tl!("Alignment:"));
        ui.horizontal(|ui| {
            for (v, s) in [("right", "Right"), ("center", "Center"), ("left", "Left")] {
                ui.radio_value(&mut f.align, v.to_string(), tl!(s));
            }
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Preview")).small().weak());
    number_preview(ui, f);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    // While Font… is open (the app's Font dialog, over this one), Enter and Escape are its own.
    if let Some(mut d) = f.font_dialog.take() {
        match crate::dialogs::show_number_font(app, ui.ctx(), &mut d) {
            None => f.font_dialog = Some(d),
            Some(true) => {
                if let Some(changed) = crate::dialogs::number_font_result(app, &d) {
                    f.props.extend(changed);
                }
            }
            Some(false) => {}
        }
        return false;
    }
    if ok {
        // para.defineNumber checks the format: a refused one keeps the dialog open, with the
        // reason in the status bar.
        if let Err(e) = app.run("para.defineNumber", f.params()) {
            app.status(e);
            return false;
        }
        return true;
    }
    cancel
}

/// A page with three numbered paragraphs in the form's style, format, font and alignment
/// (numbers aligned at one point, text in grey bars after them), between grey lines of text.
fn number_preview(ui: &mut Ui, f: &NumberForm) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(300.0, 132.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, egui::Color32::WHITE);
    p.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let grey = |y: f32| {
        let r = egui::Rect::from_min_max(egui::pos2(rect.left() + 14.0, y - 2.0), egui::pos2(rect.right() - 14.0, y + 2.0));
        p.rect_filled(r, 1.0, egui::Color32::from_gray(205));
    };
    grey(rect.top() + 14.0);
    grey(rect.bottom() - 14.0);
    let at = rect.left() + 66.0;
    let text_x = rect.left() + 104.0;
    let (bold, italic, rgb) = f.look();
    let ink = rgb.map_or(egui::Color32::BLACK, crate::theme::c32);
    let size = f.preview_size();
    let font = if bold { semibold(size) } else { egui::FontId::proportional(size) };
    for i in 0..3u32 {
        let y = rect.top() + 40.0 + 26.0 * i as f32;
        let mut job = egui::text::LayoutJob::default();
        let format = egui::TextFormat { font_id: font.clone(), color: ink, italics: italic, ..Default::default() };
        job.append(&f.sample(i + 1), 0.0, format);
        let galley = p.layout_job(job);
        let w = galley.size().x;
        let x = match f.align.as_str() {
            "center" => at - w / 2.0,
            "right" => at - w,
            _ => at,
        };
        p.galley(egui::pos2(x, y - galley.size().y / 2.0), galley, ink);
        let bar = egui::Rect::from_min_max(egui::pos2(text_x.max(x + w + 4.0), y - 2.5), egui::pos2(rect.right() - 14.0, y + 2.5));
        p.rect_filled(bar, 1.0, egui::Color32::from_gray(90));
    }
}

/// Track Changes Options (Review › Tracking): what markup shows and how revisions look.
#[derive(Clone, Debug, PartialEq, Serialize)]
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

/// A choice from `options` ((value, label)) in a drop-down.
fn choice(ui: &mut Ui, id: &str, value: &mut String, options: &[(&str, &str)]) {
    let label = |k: &str| options.iter().find(|(v, _)| *v == k).map(|(_, l)| tl!(l).to_string()).unwrap_or_default();
    craft_ui::ComboBox::from_id_salt(id).selected_text(label(value)).width(150.0).show_ui(ui, |ui| {
        for (v, l) in options {
            ui.selectable_value(value, v.to_string(), tl!(l));
        }
    });
}

/// A colour: `none_label` (by author, or `auto` when given) or a colour from the grid. `value` is
/// hex.
fn color(ui: &mut Ui, theme: &[wordcraft_doc::Rgb], value: &mut String, none_label: &str, auto: Option<wordcraft_doc::Rgb>) {
    ui.horizontal(|ui| {
        let id = ui.scope_id().with("colorField");
        crate::colors::take_field(ui.ctx(), id, value);
        let c = wordcraft_doc::Rgb::parse(value);
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        let t = Tokens::get(ui.ctx());
        match c.or(auto) {
            Some(c) => {
                ui.painter().rect_filled(r, 2.0, crate::theme::c32(c));
            }
            // By author: the first author colours, side by side.
            None => {
                for (k, a) in [0u32, 1, 2].iter().enumerate() {
                    let w = r.width() / 3.0;
                    let part = egui::Rect::from_min_size(r.min + vec2(w * k as f32, 0.0), vec2(w, r.height()));
                    ui.painter().rect_filled(part, 0.0, crate::theme::c32(wordcraft_layout::display::revision_color(*a)));
                }
            }
        }
        ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
        let label = if c.is_some() { value.clone() } else { tl!(none_label).to_string() };
        ui.menu_button(label, |ui| {
            if ui.button(tl!(none_label)).clicked() {
                value.clear();
                ui.close();
            }
            crate::colors::field_menu(ui, theme, id, value);
        });
    });
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

/// Track Changes Options. Returns true to close.
pub fn track_options(app: &mut WordApp, ui: &mut Ui, f: &mut TrackForm) -> bool {
    let theme = app.session.doc.settings.theme_colors.clone();
    let heading = |ui: &mut Ui, s: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    };
    heading(ui, "Show");
    egui::Grid::new("tco_show").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
        ui.checkbox(&mut f.comments, tl!("Comments"));
        ui.checkbox(&mut f.ink, tl!("Ink"));
        ui.end_row();
        ui.checkbox(&mut f.insertions_deletions, tl!("Insertions and Deletions"));
        ui.checkbox(&mut f.formatting, tl!("Formatting"));
        ui.end_row();
    });
    heading(ui, "Balloons");
    for (v, l) in [
        ("revisions", "Show revisions in balloons"),
        ("inline", "Show all revisions inline"),
        ("commentsAndFormatting", "Show only comments and formatting in balloons"),
    ] {
        ui.radio_value(&mut f.balloons, v.to_string(), tl!(l));
    }
    heading(ui, "Comments");
    for (v, l) in [("margin", "In the margin"), ("selected", "Only for the selected text")] {
        ui.radio_value(&mut f.comment_display, v.to_string(), tl!(l));
    }
    heading(ui, "Advanced");
    egui::Grid::new("tco_advanced").num_columns(3).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Insertions:"));
        choice(ui, "tco_ins", &mut f.insert_mark, INSERT_MARKS);
        color(ui, &theme, &mut f.insert_color, "By author", None);
        ui.end_row();
        ui.label(tl!("Deletions:"));
        choice(ui, "tco_del", &mut f.delete_mark, DELETE_MARKS);
        color(ui, &theme, &mut f.delete_color, "By author", None);
        ui.end_row();
        ui.label(tl!("Changed lines:"));
        choice(
            ui,
            "tco_bar",
            &mut f.changed_lines,
            &[("outside", "Outside border"), ("left", "Left border"), ("right", "Right border"), ("none", "None")],
        );
        color(ui, &theme, &mut f.changed_lines_color, "Automatic", Some(wordcraft_layout::display::CHANGE_BAR));
        ui.end_row();
    });
    heading(ui, "Moves");
    egui::Grid::new("tco_moves").num_columns(3).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Moved from:"));
        choice(ui, "tco_move_from", &mut f.move_from_mark, DELETE_MARKS);
        ui.push_id("tco_move_from_color", |ui| color(ui, &theme, &mut f.move_from_color, "By author", None));
        ui.end_row();
        ui.label(tl!("Moved to:"));
        choice(ui, "tco_move_to", &mut f.move_to_mark, MOVE_TO_MARKS);
        ui.push_id("tco_move_to_color", |ui| color(ui, &theme, &mut f.move_to_color, "By author", None));
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.checkbox(&mut f.track_formatting, tl!("Track formatting"));
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Err(e) = app.run("review.trackingOptions", f.params()) {
        app.status(e);
    }
    ok || cancel
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use crate::dialogs::Dialog;
    use crate::{Services, WordApp};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    /// The Define New Number Format dialog on its own: state = (app, form, whether it closed, once
    /// drawn). The app's fonts go in on the first frame, which draws nothing.
    fn number_harness(a: WordApp, form: super::NumberForm) -> egui_kittest::Harness<'static, (WordApp, super::NumberForm, Option<bool>)> {
        egui_kittest::Harness::builder().with_size(egui::vec2(480.0, 520.0)).build_ui_state(
            |ui, st: &mut (WordApp, super::NumberForm, Option<bool>)| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    crate::theme::install_fonts(ui.ctx());
                    ui.ctx().request_repaint();
                    return;
                }
                // Once closed it stays closed, as in the app.
                if st.2 != Some(true) {
                    st.2 = Some(super::define_number(&mut st.0, ui, &mut st.1));
                }
            },
            (a, form, None),
        )
    }

    /// #2787: Define New Number Format opens from the Numbering drop-down's command without
    /// settings, on `1.`, `2.`; the preview draws the number at the Font dialog's size, within
    /// what its rows hold.
    #[test]
    fn define_new_number_format_opens_on_a_decimal_number() {
        let mut a = app();
        assert_eq!(a.run("para.defineNumber", json!({})).unwrap(), json!({"pending": "defineNumber"}));
        let Some(Dialog::DefineNumber { form }) = &a.dialog else { panic!("dialog") };
        assert_eq!((form.sample(1), form.sample(2)), ("1.".to_string(), "2.".to_string()));
        let mut sized = (**form).clone();
        assert_eq!(sized.preview_size(), 12.0);
        for (size, want) in [(json!(16.0), 16.0), (json!(400.0), 20.0), (json!(1.0), 6.0), (json!("big"), 12.0)] {
            sized.props.insert("size".into(), size);
            assert_eq!(sized.preview_size(), want);
        }
    }

    /// #2787: the number in the format box deletes as one piece (style none) and comes back
    /// where it stood with a number style; Font… opens the app's Font dialog for the number (OK
    /// keeps its formatting, Escape doesn't, neither formats text); OK numbers the paragraph in
    /// one step, a none format with its text; Cancel changes nothing.
    #[test]
    fn define_new_number_format_dialog_applies_only_a_valid_format() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("text.insert", json!({"text": "Clause"})).unwrap();
        let doc = a.session.doc.clone();
        // The dialog itself, clicked: state = (app, form, what the dialog returned).
        let mut form = super::NumberForm { text: "(%1)".into(), ..Default::default() };
        form.format = "lowerLetter".into();
        let mut h = number_harness(a, form);
        h.run();
        assert_eq!(h.state().1.sample(2), "(b)");
        // The number in the format box is one piece: Backspace on it deletes it, and the style is
        // none (Word's "(none)"), the box plain text.
        h.get_by_label("Number").click();
        h.run();
        h.key_press(egui::Key::Backspace);
        h.run();
        assert_eq!((h.state().1.format.as_str(), h.state().1.text.as_str()), ("none", "()"));
        assert!(h.query_by_label("Number").is_none(), "no number left in the box");
        assert_eq!(h.state().2, Some(false), "still open");
        // Picking a number style again puts the number back where it stood.
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Number style:").click();
        h.run();
        h.get_by_label("a, b, c, …").click();
        h.run();
        assert_eq!((h.state().1.format.as_str(), h.state().1.text.as_str()), ("lowerLetter", "(%1)"));
        // From the number, Left goes to the text before it: typing lands there.
        h.get_by_label("Number").click();
        h.run();
        h.key_press(egui::Key::ArrowLeft);
        h.run();
        h.input_mut().events.push(egui::Event::Text("x".into()));
        h.run();
        assert_eq!(h.state().1.text, "(x%1)");
        h.state_mut().1.text = "(%1)".into();
        h.state_mut().1.align = "right".into();
        h.run();
        // Font… opens the app's Font dialog over this one; Enter there is its OK, the number takes
        // its formatting and no text is formatted, while this dialog stays open.
        h.get_by_label("Font…").click();
        h.run();
        assert!(h.query_by_label("Size:").is_some(), "the Font dialog is open");
        h.get_by_label("Bold").click();
        h.get_by_label("Italic").click();
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(h.state().1.font_dialog.is_none(), "Font closed");
        assert_eq!(h.state().2, Some(false), "Define New Number Format still open");
        assert_eq!(h.state().1.look(), (true, true, None));
        assert_eq!(h.state().0.session.doc, doc, "the Font dialog formatted no text");
        // Font… again, opening on the number's formatting, then Escape: nothing changes.
        h.get_by_label("Font…").click();
        h.run();
        h.get_by_label("Bold").click();
        h.run();
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(h.state().1.font_dialog.is_none(), "Font closed");
        assert_eq!(h.state().1.look(), (true, true, None), "Escape keeps the formatting as it was");
        assert_eq!(h.state().2, Some(false));
        h.get_by_label("OK").click();
        h.run();
        let st = h.state_mut();
        assert_eq!(st.2, Some(true), "OK closes it");
        let got = st.0.session.run("list.get", &json!({})).unwrap();
        let l = &got["levels"][0];
        assert_eq!((l["format"].as_str(), l["text"].as_str(), l["align"].as_str()), (Some("lowerLetter"), Some("(%1)"), Some("right")));
        assert_eq!((l["bold"].as_bool(), l["italic"].as_bool()), (Some(true), Some(true)));
        st.0.run("edit.undo", json!({})).unwrap();
        assert_eq!(st.0.session.doc, doc, "one undo step");
        drop(h);
        // "(none)": every label is the format's text.
        let mut c = app();
        c.run("text.insert", json!({"text": "Clause"})).unwrap();
        let mut h = number_harness(c, super::NumberForm { format: "none".into(), text: "Note:".into(), ..Default::default() });
        h.run();
        assert_eq!(h.state().1.sample(1), "Note:");
        h.get_by_label("OK").click();
        h.run();
        let got = h.state_mut().0.session.run("list.get", &json!({})).unwrap();
        assert_eq!((got["levels"][0]["format"].as_str(), got["levels"][0]["text"].as_str()), (Some("none"), Some("Note:")));
        drop(h);
        // OK on a format without the number: the dialog stays open, the status bar says why.
        let mut d = app();
        d.run("text.insert", json!({"text": "Clause"})).unwrap();
        let doc = d.session.doc.clone();
        let mut h = number_harness(d, super::NumberForm { text: "Item".into(), ..Default::default() });
        h.run();
        h.get_by_label("OK").click();
        h.run();
        assert_eq!(h.state().2, Some(false), "still open");
        assert_eq!(h.state().0.session.doc, doc);
        assert!(h.state().0.status_msg.as_ref().is_some_and(|(m, _)| m.contains("%1")), "{:?}", h.state().0.status_msg);
        drop(h);
        // Cancel: nothing happens.
        let mut b = app();
        b.run("text.insert", json!({"text": "Clause"})).unwrap();
        let doc = b.session.doc.clone();
        let mut h = number_harness(b, super::NumberForm::default());
        h.run();
        h.get_by_label("Cancel").click();
        h.run();
        assert_eq!(h.state().2, Some(true));
        assert_eq!(h.state().0.session.doc, doc);
    }

    /// #328: the ribbon opens Define New Multilevel List; OK defines the list and numbers the
    /// paragraph. Scripts without levels get an error, never the dialog.
    #[test]
    fn define_new_multilevel_list_opens_from_the_ribbon_and_applies() {
        let mut a = app();
        a.run("text.insert", json!({"text": "Intro"})).unwrap();
        assert_eq!(a.run("list.define", json!({})).unwrap(), json!({"pending": "defineList"}));
        let Some(Dialog::DefineList { form }) = &mut a.dialog else { panic!("dialog") };
        assert_eq!((form.levels.len(), form.sample(0), form.sample(1)), (9, "1.".to_string(), "a.".to_string()));
        form.levels[0].text = "Part %1:".into();
        form.levels[0].format = "upperRoman".into();
        form.levels[1].text = "%1.%2".into();
        form.levels[1].legal = true;
        assert_eq!((form.sample(0), form.sample(1)), ("Part I:".to_string(), "1.a".to_string()), "legal shows levels above in digits");
        let params = form.params();
        a.dialog = None;
        a.run("list.define", params).unwrap();
        let got = a.session.run("list.get", &json!({})).unwrap();
        assert_eq!((got["levels"][0]["text"].as_str(), got["levels"][0]["format"].as_str()), (Some("Part %1:"), Some("upperRoman")));
        // Opening it again starts from the caret's list.
        a.run("ui.dialog", json!({"name": "defineList"})).unwrap();
        assert!(matches!(&a.dialog, Some(Dialog::DefineList { form }) if form.sample(0) == "Part I:"));
        let mut b = app();
        assert!(b.execute("list.define", json!({})).is_err());
        assert!(b.dialog.is_none());
    }

    /// #328: the Tracking group's launcher opens Track Changes Options; OK changes the per-user
    /// preference that is saved with the interface settings.
    #[test]
    fn track_changes_options_open_from_the_launcher_and_persist() {
        let mut a = app();
        assert_eq!(a.run("review.trackingOptions", json!({})).unwrap(), json!({"pending": "trackChangesOptions"}));
        let Some(Dialog::TrackOptions { form }) = &mut a.dialog else { panic!("dialog") };
        assert_eq!((form.insert_mark.as_str(), form.balloons.as_str(), form.insert_color.as_str()), ("underline", "commentsAndFormatting", ""));
        assert_eq!(form.comment_display, "margin", "comments stay in the margin, as in Word, unless chosen otherwise (#1055)");
        form.comment_display = "selected".into();
        form.insert_mark = "doubleUnderline".into();
        form.changed_lines = "right".into();
        form.changed_lines_color = "FF0000".into();
        let params = form.params();
        a.dialog = None;
        a.run("review.trackingOptions", params).unwrap();
        let m = &a.session.prefs.markup;
        assert_eq!(m.insert_mark, wordcraft_layout::display::InsertMark::DoubleUnderline);
        assert_eq!(m.comment_display, wordcraft_layout::display::CommentDisplay::Selected);
        assert_eq!(m.changed_lines_color, Some(wordcraft_doc::Rgb(0xFF, 0, 0)));
        // Saved and restored with the preferences.
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
        let mut a = app();
        a.session.run("review.trackingOptions", &json!({"trackMoves": false})).unwrap();
        a.run("review.trackingOptions", json!({})).unwrap();
        let Some(Dialog::TrackOptions { form }) = &mut a.dialog else { panic!("dialog") };
        assert_eq!(
            (form.move_from_mark.as_str(), form.move_from_color.clone(), form.move_to_mark.as_str(), form.move_to_color.clone()),
            ("doubleStrikethrough", MOVE_COLOR.hex(), "doubleUnderline", MOVE_COLOR.hex())
        );
        form.move_from_mark = "strikethrough".into();
        form.move_from_color = "FF0000".into();
        form.move_to_mark = "bold".into();
        form.move_to_color = String::new();
        let params = form.params();
        assert!(params.get("trackMoves").is_none(), "{params}");
        a.dialog = None;
        a.run("review.trackingOptions", params).unwrap();
        let m = &a.session.prefs.markup;
        assert_eq!((m.move_from_mark, m.move_from_color), (DeleteMark::Strikethrough, Some(wordcraft_doc::Rgb(0xFF, 0, 0))));
        assert_eq!((m.move_to_mark, m.move_to_color), (InsertMark::Bold, None), "empty = by author");
        assert!(!a.session.doc.settings.track_moves, "the document setting is untouched");
    }

    /// Gap #5, Word for Mac 16.113.4 (2026-10-15): the Deletions and Moved from lists, and the
    /// Moved to list, in Word's order; every choice is one `review.trackingOptions` takes.
    #[test]
    fn track_changes_options_mark_lists_follow_word() {
        use wordcraft_layout::display::{DeleteMark, InsertMark};
        let values = |l: &[(&str, &str)]| l.iter().map(|(v, _)| v.to_string()).collect::<Vec<_>>();
        assert_eq!(
            values(super::DELETE_MARKS),
            [
                "none",
                "colorOnly",
                "bold",
                "italic",
                "underline",
                "doubleUnderline",
                "strikethrough",
                "hidden",
                "caret",
                "hash",
                "doubleStrikethrough"
            ]
        );
        assert_eq!(values(super::MOVE_TO_MARKS), ["none", "colorOnly", "bold", "italic", "underline", "doubleUnderline", "strikethrough"]);
        assert!(super::DELETE_MARKS.iter().all(|(v, _)| DeleteMark::parse(v).is_some()));
        assert!(super::MOVE_TO_MARKS.iter().chain(super::INSERT_MARKS).all(|(v, _)| InsertMark::parse(v).is_some()));
        let mut a = app();
        for (v, _) in super::DELETE_MARKS {
            a.run("review.trackingOptions", json!({"deleteMark": v, "moveFromMark": v})).unwrap();
        }
    }
}
