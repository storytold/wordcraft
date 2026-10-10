//! Columns, Symbol and Field dialogs (#321): Layout › Columns › More Columns…, Insert › Symbol ›
//! More Symbols… and Insert › Quick Parts › Field…. Each ends by running one command
//! (`layout.columns`, `insert.symbol`, `insert.field`), so scripts and agents get the same result
//! without the dialog. Shown through [`crate::dialogs::Dialog::Insert`].

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum InsertDialog {
    Columns(ColumnsForm),
    Symbol(SymbolForm),
    Field(FieldForm),
}

impl InsertDialog {
    pub fn name(&self) -> &'static str {
        match self {
            InsertDialog::Columns(_) => "columns",
            InsertDialog::Symbol(_) => "symbol",
            InsertDialog::Field(_) => "field",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            InsertDialog::Columns(_) => "Columns",
            InsertDialog::Symbol(_) => "Symbol",
            InsertDialog::Field(_) => "Field",
        }
    }
}

/// The dialog `ui.dialog` opens by `name`: `columns`, `symbol`, `specialCharacters` or `field`.
pub fn open(name: &str, app: &WordApp) -> Option<InsertDialog> {
    Some(match name {
        "columns" => InsertDialog::Columns(ColumnsForm::read(app)),
        "symbol" => InsertDialog::Symbol(SymbolForm::new(app, 0)),
        "specialCharacters" => InsertDialog::Symbol(SymbolForm::new(app, 1)),
        "field" => InsertDialog::Field(FieldForm::new(app)),
        _ => return None,
    })
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut InsertDialog) -> bool {
    match d {
        InsertDialog::Columns(f) => columns_ui(app, ui, f),
        InsertDialog::Symbol(f) => symbol_ui(app, ui, f),
        InsertDialog::Field(f) => field_ui(app, ui, f),
    }
}

/// OK / Cancel (or another OK label); returns (ok, cancel). `ok_enabled` greys OK out.
fn buttons(ui: &mut Ui, ok: &str, cancel: &str, ok_enabled: bool) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!(cancel)).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                r.1 = true;
            }
            let okb = ui.add_enabled(ok_enabled, egui::Button::new(egui::RichText::new(tl!(ok)).color(Color32::WHITE)).fill(crate::theme::APP_COLOR));
            if okb.clicked() || (ok_enabled && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                r.0 = true;
            }
        });
    });
    r
}

// ---------------------------------------------------------------------------------------------
// Columns

/// Narrowest column, points (the engine refuses narrower ones).
const MIN_COL_PT: f32 = 18.0;

/// The Columns dialog's fields. Lengths are in the interface unit ([`wordcraft_geom::Unit`]).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnsForm {
    pub count: u32,
    pub equal: bool,
    /// Width and the spacing after it, per column (the last column's spacing is unused).
    pub cols: Vec<(f32, f32)>,
    /// Line between columns.
    pub separator: bool,
    /// `section`, `document` or `forward` (this point forward).
    pub apply: String,
    /// The section's text width, in the interface unit.
    pub text_width: f32,
    /// The document has more than one section (offer "This section").
    pub sections: bool,
}

impl ColumnsForm {
    fn k() -> f32 {
        wordcraft_geom::Unit::default().pt_per_unit()
    }

    pub fn read(app: &WordApp) -> ColumnsForm {
        let k = Self::k();
        let sp = wordcraft_engine::cmd::page::sect(&app.session);
        let c = &sp.columns;
        let count = c.count.clamp(1, 12);
        let sections = app.session.doc.sections().len() > 1;
        let mut f = ColumnsForm {
            count,
            equal: true,
            cols: Vec::new(),
            separator: c.separator,
            apply: if sections { "section" } else { "document" }.into(),
            text_width: sp.text_width() / k,
            sections,
        };
        if c.widths.len() == count as usize && count > 1 {
            f.equal = false;
            f.cols = c.widths.iter().map(|(w, s)| (w / k, s / k)).collect();
        } else {
            f.cols = vec![(0.0, c.space.max(0.0) / k); count as usize];
            f.even_out();
        }
        f
    }

    fn min_w() -> f32 {
        MIN_COL_PT / Self::k()
    }

    /// Spacing the first column has (or the default 0.5").
    fn spacing(&self) -> f32 {
        self.cols.first().map(|c| c.1).unwrap_or(36.0 / Self::k())
    }

    /// Equal widths for `count` columns sharing the first column's spacing.
    fn even_out(&mut self) {
        let n = self.count.clamp(1, 12) as usize;
        let room = (self.text_width - Self::min_w() * n as f32).max(0.0);
        let sp = if n > 1 { self.spacing().clamp(0.0, room / (n - 1) as f32) } else { self.spacing() };
        let w = (self.text_width - sp * (n as f32 - 1.0)) / n as f32;
        self.cols = (0..n).map(|i| (w, if i + 1 < n { sp } else { 0.0 })).collect();
        if n == 1
            && let Some(c) = self.cols.first_mut()
        {
            c.1 = sp;
        }
    }

    pub fn set_count(&mut self, n: u32) {
        self.count = n.clamp(1, 12);
        self.even_out();
    }

    /// One, Two, Three, Left or Right.
    pub fn preset(&mut self, p: &str) {
        let k = Self::k();
        let sp = 36.0 / k;
        match p {
            "left" | "right" => {
                self.count = 2;
                self.equal = false;
                let narrow = (self.text_width - sp) / 3.0;
                let wide = (self.text_width - sp) * 2.0 / 3.0;
                self.cols = if p == "left" { vec![(narrow, sp), (wide, 0.0)] } else { vec![(wide, sp), (narrow, 0.0)] };
            }
            _ => {
                self.count = match p {
                    "two" => 2,
                    "three" => 3,
                    _ => 1,
                };
                self.equal = true;
                if let Some(c) = self.cols.first_mut() {
                    c.1 = sp;
                }
                self.even_out();
            }
        }
    }

    /// The preset the current settings match, if any.
    pub fn current_preset(&self) -> Option<&'static str> {
        let near = |a: f32, b: f32| (a - b).abs() < 0.02;
        if self.equal || self.count == 1 {
            return match self.count {
                1 => Some("one"),
                2 => Some("two"),
                3 => Some("three"),
                _ => None,
            };
        }
        let (a, b) = (self.cols.first()?.0, self.cols.get(1)?.0);
        if self.count == 2 && near(a * 2.0, b) {
            Some("left")
        } else if self.count == 2 && near(b * 2.0, a) {
            Some("right")
        } else {
            None
        }
    }

    /// After column `i`'s width or spacing changed: the next column (the one before, for the
    /// last) takes up the difference so the columns fill the text width.
    pub fn rebalance(&mut self, i: usize) {
        let n = self.cols.len();
        if self.equal {
            if let (Some(&(w, sp)), true) = (self.cols.get(i), n > 1) {
                // Equal columns: a changed width sets the spacing, a changed spacing the width.
                let sp = if i == 0 && (w - self.cols.get(1).map(|c| c.0).unwrap_or(w)).abs() > 1e-4 {
                    (self.text_width - w * n as f32) / (n - 1) as f32
                } else {
                    sp
                };
                if let Some(c) = self.cols.first_mut() {
                    c.1 = sp.max(0.0);
                }
            }
            self.even_out();
            return;
        }
        if n < 2 {
            return;
        }
        if let Some(last) = self.cols.last_mut() {
            last.1 = 0.0;
        }
        let j = if i + 1 < n { i + 1 } else { i.saturating_sub(1) };
        let total: f32 = self.cols.iter().map(|(w, s)| w + s).sum();
        let excess = total - self.text_width;
        let min = Self::min_w();
        let short = match self.cols.get_mut(j) {
            Some(c) => {
                c.0 -= excess;
                let short = (min - c.0).max(0.0);
                c.0 = c.0.max(min);
                short
            }
            None => 0.0,
        };
        // The neighbour can't shrink further: take the rest back from the edited column.
        if short > 0.0
            && let Some(c) = self.cols.get_mut(i)
        {
            let from_w = short.min((c.0 - min).max(0.0));
            c.0 -= from_w;
            c.1 = (c.1 - (short - from_w)).max(0.0);
        }
    }

    /// The `layout.columns` parameters.
    pub fn params(&self) -> Value {
        let k = Self::k();
        let r = |x: f32| (x * k * 100.0).round() / 100.0;
        if self.equal || self.count <= 1 {
            json!({"count": self.count, "space": r(self.spacing()), "separator": self.separator, "apply": self.apply})
        } else {
            let widths: Vec<[f32; 2]> = self.cols.iter().map(|(w, s)| [r(*w), r(*s)]).collect();
            json!({"widths": widths, "separator": self.separator, "apply": self.apply})
        }
    }
}

/// A preset tile: a small page with its columns.
fn preset_tile(ui: &mut Ui, label: &str, widths: &[f32], on: bool, t: &Tokens) -> bool {
    let resp = ui
        .vertical(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(44.0, 52.0), Sense::click());
            let fill = if on {
                t.checked
            } else if resp.hovered() {
                t.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect(r, 3.0, fill, Stroke::new(1.0, if on { t.accent } else { t.border }), egui::StrokeKind::Inside);
            let page = Rect::from_min_size(r.min + vec2(9.0, 7.0), vec2(26.0, 38.0));
            ui.painter().rect(page, 0.0, t.input, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
            let inner = page.shrink(4.0);
            let gap = 3.5;
            let total: f32 = widths.iter().sum::<f32>().max(1.0);
            let avail = inner.width() - gap * (widths.len() as f32 - 1.0);
            let mut x = inner.min.x;
            for w in widths {
                let cw = avail * w / total;
                let mut y = inner.min.y;
                while y + 1.5 <= inner.max.y {
                    ui.painter().line_segment([pos2(x, y), pos2(x + cw, y)], Stroke::new(1.0, t.text_dim));
                    y += 3.0;
                }
                x += cw + gap;
            }
            ui.label(egui::RichText::new(tl!(label)).small());
            resp
        })
        .inner;
    resp.clicked()
}

/// The page preview: the columns (and the line between them) on a page.
fn columns_preview(ui: &mut Ui, f: &ColumnsForm, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(96.0, 124.0), Sense::hover());
    let page = Rect::from_center_size(r.center(), vec2(84.0, 110.0));
    ui.painter().rect(page, 0.0, t.input, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let inner = page.shrink(9.0);
    let scale = inner.width() / f.text_width.max(0.1);
    let mut x = inner.min.x;
    let n = f.cols.len();
    for (i, (w, sp)) in f.cols.iter().enumerate() {
        let cw = (w * scale).max(2.0);
        let mut y = inner.min.y;
        while y + 2.0 <= inner.max.y {
            ui.painter().line_segment([pos2(x, y), pos2((x + cw).min(inner.max.x), y)], Stroke::new(1.2, t.text_dim));
            y += 4.0;
        }
        x += cw;
        if i + 1 < n {
            let gap = sp * scale;
            if f.separator {
                let mx = x + gap / 2.0;
                ui.painter().line_segment([pos2(mx, inner.min.y), pos2(mx, inner.max.y)], Stroke::new(1.0, t.text));
            }
            x += gap;
        }
    }
}

fn columns_ui(app: &mut WordApp, ui: &mut Ui, f: &mut ColumnsForm) -> bool {
    let t = Tokens::get(ui.ctx());
    let unit = wordcraft_geom::Unit::default();
    ui.set_width(420.0);
    ui.label(egui::RichText::new(tl!("Presets")).font(semibold(12.5)));
    let current = f.current_preset();
    ui.horizontal(|ui| {
        for (key, label, widths) in [
            ("one", "One", &[1.0][..]),
            ("two", "Two", &[1.0, 1.0][..]),
            ("three", "Three", &[1.0, 1.0, 1.0][..]),
            ("left", "Left", &[1.0, 2.0][..]),
            ("right", "Right", &[2.0, 1.0][..]),
        ] {
            if preset_tile(ui, label, widths, current == Some(key), &t) {
                f.preset(key);
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Number of columns:"));
        let mut n = f.count;
        if ui.add(egui::DragValue::new(&mut n).range(1..=12)).changed() {
            f.set_count(n);
        }
        ui.add_space(16.0);
        ui.checkbox(&mut f.separator, tl!("Line between"));
    });
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Width and spacing")).font(semibold(12.5)));
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            let mut changed = None;
            egui::ScrollArea::vertical().id_salt("columns_widths").max_height(130.0).show(ui, |ui| {
                egui::Grid::new("columns_grid").num_columns(3).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                    ui.label(tl!("Col #:"));
                    ui.label(tl!("Width:"));
                    ui.label(tl!("Spacing:"));
                    ui.end_row();
                    let n = f.cols.len();
                    let max = f.text_width.max(0.1);
                    for i in 0..n {
                        let editable = i == 0 || !f.equal;
                        ui.label(format!("{}:", i + 1));
                        if let Some(c) = f.cols.get_mut(i) {
                            let w =
                                egui::DragValue::new(&mut c.0).speed(0.01).range(ColumnsForm::min_w()..=max).suffix(unit.suffix()).max_decimals(2);
                            if ui.add_enabled(editable, w).changed() {
                                changed = Some(i);
                            }
                            if i + 1 < n {
                                let s = egui::DragValue::new(&mut c.1).speed(0.01).range(0.0..=max).suffix(unit.suffix()).max_decimals(2);
                                if ui.add_enabled(editable, s).changed() {
                                    changed = Some(i);
                                }
                            } else {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            if let Some(i) = changed {
                f.rebalance(i);
            }
            let mut equal = f.equal;
            if ui.add_enabled(f.count > 1, egui::Checkbox::new(&mut equal, tl!("Equal column width"))).changed() {
                f.equal = equal;
                if equal {
                    f.even_out();
                }
            }
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(tl!("Preview"));
            columns_preview(ui, f, &t);
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Apply to:"));
        let mut options = vec![("document", "Whole document"), ("forward", "This point forward")];
        if f.sections {
            options.insert(0, ("section", "This section"));
        }
        let shown = options.iter().find(|(k, _)| *k == f.apply).map(|(_, l)| *l).unwrap_or("This section");
        egui::ComboBox::from_id_salt("columns_apply").selected_text(tl!(shown)).width(180.0).show_ui(ui, |ui| {
            for (k, l) in &options {
                ui.selectable_value(&mut f.apply, (*k).to_string(), tl!(l));
            }
        });
    });
    let (ok, cancel) = buttons(ui, "OK", "Cancel", true);
    if ok {
        let _ = app.run("layout.columns", f.params());
    }
    ok || cancel
}

// ---------------------------------------------------------------------------------------------
// Symbol

/// A symbol inserted from the Symbol dialog, kept for its Recently used row and the ribbon's
/// Symbol gallery (`font` empty = the text's own font).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RecentSymbol {
    pub ch: String,
    pub font: String,
}

/// How many recent symbols are kept.
pub const RECENT_SYMBOLS: usize = 20;

/// Remember an inserted symbol (most recent first).
pub fn remember_symbol(app: &mut WordApp, ch: &str, font: &str) {
    let r = RecentSymbol { ch: ch.to_string(), font: font.to_string() };
    let list = &mut app.ui.recent_symbols;
    list.retain(|x| *x != r);
    list.insert(0, r);
    list.truncate(RECENT_SYMBOLS);
}

/// Unicode blocks the subset list offers (name, first, last code point). The names are the
/// Unicode Standard's block names and stay as they are in every language.
pub const SUBSETS: &[(&str, u32, u32)] = &[
    ("Basic Latin", 0x0020, 0x007F),
    ("Latin-1 Supplement", 0x0080, 0x00FF),
    ("Latin Extended-A", 0x0100, 0x017F),
    ("Latin Extended-B", 0x0180, 0x024F),
    ("IPA Extensions", 0x0250, 0x02AF),
    ("Spacing Modifier Letters", 0x02B0, 0x02FF),
    ("Combining Diacritical Marks", 0x0300, 0x036F),
    ("Greek and Coptic", 0x0370, 0x03FF),
    ("Cyrillic", 0x0400, 0x052F),
    ("Armenian", 0x0530, 0x058F),
    ("Hebrew", 0x0590, 0x05FF),
    ("Arabic", 0x0600, 0x06FF),
    ("Devanagari", 0x0900, 0x097F),
    ("Thai", 0x0E00, 0x0E7F),
    ("Georgian", 0x10A0, 0x10FF),
    ("Hangul Jamo", 0x1100, 0x11FF),
    ("Phonetic Extensions", 0x1D00, 0x1DBF),
    ("Latin Extended Additional", 0x1E00, 0x1EFF),
    ("Greek Extended", 0x1F00, 0x1FFF),
    ("General Punctuation", 0x2000, 0x206F),
    ("Superscripts and Subscripts", 0x2070, 0x209F),
    ("Currency Symbols", 0x20A0, 0x20CF),
    ("Letterlike Symbols", 0x2100, 0x214F),
    ("Number Forms", 0x2150, 0x218F),
    ("Arrows", 0x2190, 0x21FF),
    ("Mathematical Operators", 0x2200, 0x22FF),
    ("Miscellaneous Technical", 0x2300, 0x23FF),
    ("Enclosed Alphanumerics", 0x2460, 0x24FF),
    ("Box Drawing", 0x2500, 0x257F),
    ("Block Elements", 0x2580, 0x259F),
    ("Geometric Shapes", 0x25A0, 0x25FF),
    ("Miscellaneous Symbols", 0x2600, 0x26FF),
    ("Dingbats", 0x2700, 0x27BF),
    ("Supplemental Arrows", 0x27F0, 0x27FF),
    ("Braille Patterns", 0x2800, 0x28FF),
    ("Supplemental Mathematical Operators", 0x2A00, 0x2AFF),
    ("CJK Symbols and Punctuation", 0x3000, 0x303F),
    ("Hiragana", 0x3040, 0x309F),
    ("Katakana", 0x30A0, 0x30FF),
    ("CJK Unified Ideographs", 0x4E00, 0x9FFF),
    ("Hangul Syllables", 0xAC00, 0xD7AF),
    ("Private Use Area", 0xE000, 0xF8FF),
    ("Alphabetic Presentation Forms", 0xFB00, 0xFB4F),
    ("Halfwidth and Fullwidth Forms", 0xFF00, 0xFFEF),
    ("Specials", 0xFFF0, 0xFFFF),
    ("Mathematical Alphanumeric Symbols", 0x1D400, 0x1D7FF),
    ("Emoji and Pictographs", 0x1F300, 0x1FAFF),
];

/// The subset a character belongs to.
pub fn subset_of(c: char) -> Option<&'static str> {
    let cp = c as u32;
    SUBSETS.iter().find(|(_, a, b)| (*a..=*b).contains(&cp)).map(|(n, _, _)| *n)
}

/// Special Characters: (name, character, the command that inserts it, when there is one).
pub const SPECIAL: &[(&str, char, Option<&str>)] = &[
    ("Em Dash", '\u{2014}', None),
    ("En Dash", '\u{2013}', None),
    ("Nonbreaking Hyphen", '\u{2011}', Some("text.nbHyphen")),
    ("Optional Hyphen", '\u{00AD}', Some("text.optionalHyphen")),
    ("Em Space", '\u{2003}', None),
    ("En Space", '\u{2002}', None),
    ("1/4 Em Space", '\u{2005}', None),
    ("Nonbreaking Space", '\u{00A0}', Some("text.nbsp")),
    ("Copyright", '\u{00A9}', None),
    ("Registered", '\u{00AE}', None),
    ("Trademark", '\u{2122}', None),
    ("Section Mark", '\u{00A7}', None),
    ("Paragraph Mark", '\u{00B6}', None),
    ("Ellipsis", '\u{2026}', None),
    ("Single Opening Quote", '\u{2018}', None),
    ("Single Closing Quote", '\u{2019}', None),
    ("Double Opening Quote", '\u{201C}', None),
    ("Double Closing Quote", '\u{201D}', None),
    ("No-Width Optional Break", '\u{200B}', None),
    ("No-Width Non Break", '\u{2060}', None),
];

/// The Symbol dialog: the Symbols tab (a font's characters by Unicode subset) and the Special
/// Characters tab.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolForm {
    /// 0 Symbols, 1 Special Characters.
    pub tab: u8,
    /// The chosen font; empty = (normal text), the text's own font.
    pub font: String,
    pub selected: Option<char>,
    /// Character code, hexadecimal Unicode.
    pub code: String,
    /// Selected row on the Special Characters tab.
    pub special: usize,
    /// The caret's font: what (normal text) shows.
    pub text_font: String,
    /// The characters the shown font has, sorted.
    #[serde(skip)]
    chars: Vec<char>,
    /// The font `chars` was read from.
    #[serde(skip)]
    chars_of: Option<String>,
    /// Scroll the grid to this row on the next frame.
    #[serde(skip)]
    scroll_to: Option<usize>,
}

/// Grid geometry.
const GRID_COLS: usize = 16;
const CELL: f32 = 30.0;
const GRID_ROWS: usize = 8;

impl SymbolForm {
    pub fn new(app: &WordApp, tab: u8) -> SymbolForm {
        let text_font = app
            .session
            .doc
            .para_at(&app.session.sel.focus)
            .map(|p| app.session.doc.styles.resolve_char(p.props.style.as_deref(), &app.session.typing_props()).font)
            .unwrap_or_else(|| wordcraft_doc::styles::BODY_FONT.to_string());
        let recent = app.ui.recent_symbols.first().cloned();
        let font = recent.as_ref().map(|r| r.font.clone()).unwrap_or_default();
        let selected = recent.and_then(|r| r.ch.chars().next());
        let mut f =
            SymbolForm { tab, font, selected, code: String::new(), special: 0, text_font, chars: Vec::new(), chars_of: None, scroll_to: None };
        f.sync_code();
        f
    }

    /// The font the grid shows: the chosen one, or the text's.
    pub fn shown_font(&self) -> &str {
        if self.font.is_empty() { &self.text_font } else { &self.font }
    }

    /// Read the shown font's characters when it changed.
    fn load(&mut self) {
        let family = self.shown_font().to_string();
        if self.chars_of.as_deref() == Some(family.as_str()) {
            return;
        }
        // The face the document's text would use (with Word's font substitutes).
        let face = wordcraft_fonts::word::resolve(&family, false, false).face.get();
        self.chars = face.chars().into_iter().map(|(c, _)| c).filter(|c| !c.is_control() && !(0xD800..=0xDFFF).contains(&(*c as u32))).collect();
        self.chars_of = Some(family);
        // Keep the selection when the new font has it; otherwise its first character.
        if !self.selected.is_some_and(|c| self.chars.binary_search(&c).is_ok()) {
            self.selected = self.chars.iter().copied().find(|c| *c > ' ').or(self.chars.first().copied());
        }
        self.sync_code();
        self.scroll_to = self.selected.and_then(|c| self.row_of(c));
    }

    fn row_of(&self, c: char) -> Option<usize> {
        self.chars.binary_search(&c).ok().map(|i| i / GRID_COLS)
    }

    fn sync_code(&mut self) {
        self.code = self.selected.map(|c| format!("{:04X}", c as u32)).unwrap_or_default();
    }

    /// Select by typed character code (hex): the character, when the font has it.
    pub fn select_code(&mut self, code: &str) -> bool {
        let cp = u32::from_str_radix(code.trim().trim_start_matches("U+").trim_start_matches("u+"), 16).ok();
        let Some(c) = cp.and_then(char::from_u32) else { return false };
        if self.chars.binary_search(&c).is_err() {
            return false;
        }
        self.selected = Some(c);
        self.scroll_to = self.row_of(c);
        true
    }

    /// Jump to a Unicode subset: its first character in this font.
    pub fn jump_to(&mut self, subset: &str) {
        let Some((_, a, b)) = SUBSETS.iter().find(|(n, _, _)| *n == subset) else { return };
        let i = self.chars.partition_point(|c| (*c as u32) < *a);
        if let Some(c) = self.chars.get(i).filter(|c| (**c as u32) <= *b) {
            self.selected = Some(*c);
            self.sync_code();
            self.scroll_to = Some(i / GRID_COLS);
        }
    }

    /// The `insert.symbol` parameters for the selection.
    pub fn params(&self) -> Option<Value> {
        let c = self.selected?;
        Some(if self.font.is_empty() { json!({"char": c.to_string()}) } else { json!({"char": c.to_string(), "font": self.font}) })
    }
}

/// A character rendered in `font` as a coverage mask (tinted when painted), cached; `None`
/// while waiting for this frame's rendering budget.
fn glyph_texture(app: &mut WordApp, ctx: &egui::Context, font: &str, c: char) -> Option<egui::TextureHandle> {
    let ppp = ctx.pixels_per_point();
    let key = format!("symbol:{font}:{:x}:{ppp}", c as u32);
    let font = font.to_string();
    app.previews.get_or(ctx, &key, || glyph_image(&font, c, ppp))
}

fn glyph_image(font: &str, c: char, ppp: f32) -> Option<egui::ColorImage> {
    use wordcraft_doc::props::{Align, CharProps, LineSpacing};
    let size = 16.0;
    let mut p =
        wordcraft_doc::Paragraph::with_text(&c.to_string(), CharProps { font: Some(font.to_string()), size: Some(size), ..Default::default() });
    p.props.align = Some(Align::Center);
    p.props.space_before = Some(((CELL - size * 1.3) / 2.0).max(0.0));
    p.props.space_after = Some(0.0);
    p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
    let mut img = crate::previews::snippet(&wordcraft_doc::Document::new(), p, CELL - 2.0, CELL - 2.0, ppp, 0.0)?;
    // Ink against the paper (the first pixel) becomes coverage.
    let lum = |p: &Color32| (p.r() as u16 + p.g() as u16 + p.b() as u16) / 3;
    let bg = img.pixels.first().map(lum).unwrap_or(255);
    for px in img.pixels.iter_mut() {
        let a = lum(&*px).abs_diff(bg).min(255) as u8;
        *px = Color32::from_rgba_premultiplied(a, a, a, a);
    }
    Some(img)
}

/// One grid cell; returns the click response.
fn symbol_cell(app: &mut WordApp, ui: &mut Ui, font: &str, c: char, selected: bool, t: &Tokens) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
    let fill = if selected {
        t.accent
    } else if resp.hovered() {
        t.hover
    } else {
        t.input
    };
    ui.painter().rect(r, 0.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    if let Some(tex) = glyph_texture(app, ui.ctx(), font, c) {
        let sz = tex.size_vec2() / ui.ctx().pixels_per_point();
        let tint = if selected { t.on_accent } else { t.text };
        ui.painter().image(tex.id(), Rect::from_center_size(r.center(), sz), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    }
    resp
}

fn symbol_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SymbolForm) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.set_width(GRID_COLS as f32 * CELL + 24.0);
    ui.horizontal(|ui| {
        for (i, l) in ["Symbols", "Special Characters"].into_iter().enumerate() {
            if ui.selectable_label(f.tab == i as u8, tl!(l)).clicked() {
                f.tab = i as u8;
            }
        }
    });
    ui.separator();
    let mut insert: Option<(Value, String, String)> = None;
    if f.tab == 1 {
        if special_ui(app, ui, f, &mut insert, &t) {
            return true;
        }
    } else {
        f.load();
        ui.horizontal(|ui| {
            ui.label(tl!("Font:"));
            let normal = tl!("(normal text)").to_string();
            let mut items = vec![normal.clone()];
            items.extend(app.previews.families());
            let shown = if f.font.is_empty() { normal.clone() } else { f.font.clone() };
            let pf = app.previews.font_preview_fn();
            let normal_row = normal.clone();
            let preview = move |ui: &mut Ui, name: &str| -> egui::Response {
                if name == normal_row {
                    let (r, resp) = ui.allocate_exact_size(vec2(260.0, crate::widgets::COMBO_PREVIEW_ROW_H), Sense::click());
                    let t = Tokens::get(ui.ctx());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 3.0, t.hover);
                    }
                    ui.painter().text(pos2(r.min.x + 8.0, r.center().y), egui::Align2::LEFT_CENTER, name, regular(12.0), t.text);
                    resp
                } else {
                    pf(ui, name)
                }
            };
            if let Some(v) = crate::widgets::combo(ui, "symbol_font", 190.0, &shown, &items, Some(&preview)) {
                f.font = if v == normal || v.trim().is_empty() { String::new() } else { v.trim().to_string() };
                f.load();
            }
            ui.add_space(8.0);
            ui.label(tl!("Subset:"));
            let current = f.selected.and_then(subset_of).unwrap_or("");
            let mut chosen = None;
            egui::ComboBox::from_id_salt("symbol_subset").selected_text(current).width(170.0).show_ui(ui, |ui| {
                for (name, a, b) in SUBSETS {
                    let i = f.chars.partition_point(|c| (*c as u32) < *a);
                    if f.chars.get(i).is_some_and(|c| (*c as u32) <= *b) && ui.selectable_label(*name == current, *name).clicked() {
                        chosen = Some(*name);
                    }
                }
            });
            if let Some(s) = chosen {
                f.jump_to(s);
            }
        });
        ui.add_space(4.0);
        let rows = f.chars.len().div_ceil(GRID_COLS);
        let mut area = egui::ScrollArea::vertical().id_salt("symbol_grid").max_height(CELL * GRID_ROWS as f32).auto_shrink([false, false]);
        if let Some(row) = f.scroll_to.take() {
            // Keep the row in view: it lands on the grid's second row when there is room.
            area = area.vertical_scroll_offset(row.saturating_sub(1) as f32 * CELL);
        }
        let font = f.shown_font().to_string();
        let chars = f.chars.clone();
        let mut picked = None;
        let mut double = false;
        area.show_rows(ui, CELL, rows, |ui, range| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for row in range {
                ui.horizontal(|ui| {
                    for c in chars.iter().skip(row * GRID_COLS).take(GRID_COLS) {
                        let resp = symbol_cell(app, ui, &font, *c, f.selected == Some(*c), &t);
                        if resp.clicked() {
                            picked = Some(*c);
                        }
                        if resp.double_clicked() {
                            picked = Some(*c);
                            double = true;
                        }
                    }
                });
            }
        });
        if chars.is_empty() {
            ui.label(egui::RichText::new(tl!("This font has no characters to show.")).weak());
        }
        if let Some(c) = picked {
            f.selected = Some(c);
            f.sync_code();
        }
        ui.add_space(6.0);
        ui.label(tl!("Recently used symbols:"));
        let recent = app.ui.recent_symbols.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for r in recent.iter().take(GRID_COLS) {
                let Some(c) = r.ch.chars().next() else { continue };
                let rfont = if r.font.is_empty() { f.text_font.clone() } else { r.font.clone() };
                let resp = symbol_cell(app, ui, &rfont, c, false, &t);
                if resp.clicked() || resp.double_clicked() {
                    f.font = r.font.clone();
                    f.selected = Some(c);
                    f.load();
                    f.sync_code();
                    f.scroll_to = f.row_of(c);
                }
                if resp.double_clicked() {
                    double = true;
                }
            }
            if recent.is_empty() {
                let _ = ui.allocate_exact_size(vec2(CELL, CELL), Sense::hover());
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let name = f.selected.map(|c| format!("U+{:04X}", c as u32)).unwrap_or_default();
            ui.label(egui::RichText::new(name).font(semibold(12.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(tl!("from Unicode (hex)"));
                let resp = ui.add(egui::TextEdit::singleline(&mut f.code).desired_width(70.0).font(egui::TextStyle::Monospace));
                if resp.changed() {
                    let code = f.code.clone();
                    f.select_code(&code);
                }
                ui.label(tl!("Character code:"));
            });
        });
        if double && let Some(p) = f.params() {
            insert = Some((p, f.selected.map(String::from).unwrap_or_default(), f.font.clone()));
        }
        // The Insert button.
        let (ok, close) = buttons(ui, "Insert", "Close", f.selected.is_some());
        if ok && let Some(p) = f.params() {
            insert = Some((p, f.selected.map(String::from).unwrap_or_default(), f.font.clone()));
        }
        if close {
            return true;
        }
    }
    if let Some((params, ch, font)) = insert {
        // Special characters with their own command insert through it (non-breaking hyphen…).
        let r = match params.get("command").and_then(Value::as_str) {
            Some(id) => app.run(id, json!({})),
            None => app.run("insert.symbol", params),
        };
        if r.is_ok() {
            remember_symbol(app, &ch, &font);
        }
    }
    false
}

/// The Special Characters tab; returns true to close.
fn special_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SymbolForm, insert: &mut Option<(Value, String, String)>, t: &Tokens) -> bool {
    let mut double = false;
    egui::ScrollArea::vertical().id_salt("special_chars").max_height(CELL * GRID_ROWS as f32 + 60.0).auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("special_grid").num_columns(3).striped(true).spacing(vec2(14.0, 4.0)).show(ui, |ui| {
            ui.label(egui::RichText::new(tl!("Character")).font(semibold(12.0)));
            ui.label("");
            ui.label(egui::RichText::new(tl!("Shortcut key:")).font(semibold(12.0)));
            ui.end_row();
            for (i, (name, c, cmd)) in SPECIAL.iter().enumerate() {
                let shown = if c.is_whitespace() || matches!(*c, '\u{00AD}' | '\u{200B}' | '\u{2060}') { String::new() } else { c.to_string() };
                ui.label(egui::RichText::new(shown).font(regular(15.0)).color(t.text));
                let resp = ui.selectable_label(f.special == i, tl!(name));
                if resp.clicked() {
                    f.special = i;
                }
                if resp.double_clicked() {
                    f.special = i;
                    double = true;
                }
                ui.label(cmd.map(|id| crate::widgets::shortcut_text(app, id)).unwrap_or_default());
                ui.end_row();
            }
        });
    });
    let (ok, close) = buttons(ui, "Insert", "Close", true);
    if (ok || double)
        && let Some((_, c, cmd)) = SPECIAL.get(f.special)
    {
        let params = match cmd {
            Some(id) => json!({"command": id}),
            None => json!({"char": c.to_string()}),
        };
        *insert = Some((params, c.to_string(), String::new()));
    }
    close
}

// ---------------------------------------------------------------------------------------------
// Field

/// What a field's options are about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Date,
    Number,
    Bookmark,
    Sequence,
    Merge,
    Toc,
    Plain,
}

/// The fields WordCraft computes, by category: (name, category, description).
pub const FIELDS: &[(&str, &str, &str)] = &[
    ("CREATEDATE", "Date and Time", "The date the document was created"),
    ("DATE", "Date and Time", "Today's date"),
    ("PRINTDATE", "Date and Time", "The date the document was last printed"),
    ("SAVEDATE", "Date and Time", "The date the document was last saved"),
    ("TIME", "Date and Time", "The current time"),
    ("AUTHOR", "Document Information", "The document's author"),
    ("FILENAME", "Document Information", "The document's file name"),
    ("NUMPAGES", "Document Information", "The number of pages in the document"),
    ("NUMWORDS", "Document Information", "The number of words in the document"),
    ("TITLE", "Document Information", "The document's title"),
    ("PAGE", "Numbering", "The number of the current page"),
    ("SECTION", "Numbering", "The number of the current section"),
    ("SECTIONPAGES", "Numbering", "The number of pages in the section"),
    ("SEQ", "Numbering", "A numbered sequence (figures, tables…)"),
    ("PAGEREF", "Links and References", "The page number of a bookmark"),
    ("REF", "Links and References", "The text of a bookmark"),
    ("TOC", "Index and Tables", "A table of contents"),
    ("MERGEFIELD", "Mail Merge", "A mail merge field"),
];

/// The categories, in list order ("(All)" first).
pub const FIELD_CATEGORIES: &[&str] =
    &["(All)", "Date and Time", "Document Information", "Links and References", "Index and Tables", "Mail Merge", "Numbering"];

/// Date and time pictures for the `\@` switch.
pub const DATE_FORMATS: &[&str] = &[
    "M/d/yyyy",
    "dddd, MMMM d, yyyy",
    "MMMM d, yyyy",
    "M/d/yy",
    "yyyy-MM-dd",
    "d-MMM-yy",
    "M.d.yyyy",
    "d MMMM yyyy",
    "MMMM yy",
    "MMM-yy",
    "M/d/yyyy h:mm am/pm",
    "M/d/yyyy h:mm:ss am/pm",
    "h:mm am/pm",
    "h:mm:ss am/pm",
    "HH:mm",
    "HH:mm:ss",
];

/// Number formats for the `\*` switch: (switch value, example).
pub const NUMBER_FORMATS: &[(&str, &str)] = &[
    ("", "(none)"),
    ("Arabic", "1, 2, 3, …"),
    ("alphabetic", "a, b, c, …"),
    ("ALPHABETIC", "A, B, C, …"),
    ("roman", "i, ii, iii, …"),
    ("ROMAN", "I, II, III, …"),
    ("Ordinal", "1st, 2nd, 3rd, …"),
    ("CardText", "One, Two, Three, …"),
    ("OrdText", "First, Second, Third, …"),
];

pub fn field_kind(name: &str) -> FieldKind {
    match name {
        "DATE" | "TIME" | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => FieldKind::Date,
        "PAGE" | "NUMPAGES" | "SECTION" | "SECTIONPAGES" | "NUMWORDS" => FieldKind::Number,
        "REF" | "PAGEREF" => FieldKind::Bookmark,
        "SEQ" => FieldKind::Sequence,
        "MERGEFIELD" => FieldKind::Merge,
        "TOC" => FieldKind::Toc,
        _ => FieldKind::Plain,
    }
}

/// The Field dialog: a field by category, its options, and the field code it makes (or the code
/// typed directly with Field Codes on).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldForm {
    pub category: String,
    pub name: String,
    /// `\@` picture for date fields.
    pub date_format: String,
    /// `\*` value for number fields (empty = none).
    pub number_format: String,
    /// Bookmark (REF, PAGEREF), sequence name (SEQ) or merge field (MERGEFIELD).
    pub target: String,
    /// Keep the formatting applied to the result when it updates (`\* MERGEFORMAT`).
    pub preserve: bool,
    /// Field Codes: type the code directly.
    pub codes: bool,
    pub code: String,
    #[serde(skip)]
    bookmarks: Vec<String>,
    #[serde(skip)]
    merge_fields: Vec<String>,
}

impl FieldForm {
    pub fn new(app: &WordApp) -> FieldForm {
        let mut bookmarks: Vec<String> = app.session.doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| !n.starts_with('_')).collect();
        bookmarks.sort();
        bookmarks.dedup();
        let mut f = FieldForm {
            category: FIELD_CATEGORIES.first().copied().unwrap_or("(All)").to_string(),
            name: "DATE".into(),
            date_format: DATE_FORMATS.first().copied().unwrap_or("M/d/yyyy").to_string(),
            number_format: String::new(),
            target: String::new(),
            preserve: true,
            codes: false,
            code: String::new(),
            bookmarks,
            merge_fields: app.session.merge.headers.clone(),
        };
        f.select("DATE");
        f
    }

    /// Choose a field: its options start from sensible defaults.
    pub fn select(&mut self, name: &str) {
        self.name = name.to_string();
        self.number_format.clear();
        self.target = match field_kind(name) {
            FieldKind::Bookmark => self.bookmarks.first().cloned().unwrap_or_default(),
            FieldKind::Sequence => "Figure".into(),
            FieldKind::Merge => self.merge_fields.first().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        if field_kind(name) == FieldKind::Date {
            self.date_format = if name == "TIME" { "h:mm am/pm" } else { "M/d/yyyy" }.into();
        }
        self.code = self.instruction();
    }

    /// The field code the options make.
    pub fn instruction(&self) -> String {
        let mut s = self.name.clone();
        let target = self.target.trim();
        match field_kind(&self.name) {
            FieldKind::Date if !self.date_format.trim().is_empty() => s.push_str(&format!(" \\@ \"{}\"", self.date_format.trim().replace('"', ""))),
            FieldKind::Bookmark if !target.is_empty() => s.push_str(&format!(" {} \\h", target.split_whitespace().collect::<Vec<_>>().join("_"))),
            FieldKind::Sequence => s.push_str(&format!(
                " {}",
                if target.is_empty() { "Figure".to_string() } else { target.split_whitespace().collect::<Vec<_>>().join("_") }
            )),
            FieldKind::Merge if !target.is_empty() => {
                if target.contains(char::is_whitespace) {
                    s.push_str(&format!(" \"{}\"", target.replace('"', "")));
                } else {
                    s.push_str(&format!(" {target}"));
                }
            }
            FieldKind::Toc => s.push_str(" \\o \"1-3\" \\h \\z \\u"),
            _ => {}
        }
        if matches!(field_kind(&self.name), FieldKind::Number | FieldKind::Sequence) && !self.number_format.is_empty() {
            s.push_str(&format!(" \\* {}", self.number_format));
        }
        if self.preserve && field_kind(&self.name) != FieldKind::Toc {
            s.push_str(" \\* MERGEFORMAT");
        }
        s
    }

    /// The code to insert: typed (Field Codes) or built from the options.
    pub fn final_code(&self) -> String {
        if self.codes { self.code.trim().to_string() } else { self.instruction() }
    }

    /// The `insert.field` parameters (a merge field shows «name» until merged).
    pub fn params(&self) -> Option<Value> {
        let code = self.final_code();
        if code.is_empty() || code.len() > 4096 {
            return None;
        }
        let name = wordcraft_layout::fields::field_name(&code);
        Some(if name == "MERGEFIELD" { json!({"instr": code, "result": format!("«{}»", merge_field_name(&code))}) } else { json!({"instr": code}) })
    }
}

/// The merge field a `MERGEFIELD` code names (quoted names may hold spaces).
fn merge_field_name(code: &str) -> String {
    let rest = code.trim_start().split_once(char::is_whitespace).map(|x| x.1.trim_start()).unwrap_or("");
    match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or("").to_string(),
        None => rest.split_whitespace().next().unwrap_or("").to_string(),
    }
}

fn field_ui(app: &mut WordApp, ui: &mut Ui, f: &mut FieldForm) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.set_width(620.0);
    ui.horizontal_top(|ui| {
        // Left: category and field names.
        ui.vertical(|ui| {
            ui.set_width(200.0);
            ui.label(egui::RichText::new(tl!("Please choose a field")).font(semibold(12.5)));
            ui.label(tl!("Categories:"));
            let shown = tl!(f.category.as_str()).to_string();
            egui::ComboBox::from_id_salt("field_category").selected_text(shown).width(190.0).show_ui(ui, |ui| {
                for c in FIELD_CATEGORIES {
                    ui.selectable_value(&mut f.category, (*c).to_string(), tl!(c));
                }
            });
            ui.label(tl!("Field names:"));
            let mut picked = None;
            egui::Frame::new().stroke(Stroke::new(1.0, t.border)).inner_margin(2.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("field_names").max_height(230.0).auto_shrink([false, false]).show(ui, |ui| {
                    for (name, cat, _) in FIELDS {
                        if (f.category == "(All)" || f.category == *cat) && ui.selectable_label(f.name == *name, *name).clicked() {
                            picked = Some(*name);
                        }
                    }
                });
            });
            if let Some(n) = picked {
                f.select(n);
            }
        });
        ui.add_space(14.0);
        // Right: the field's options, or its code.
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Field properties")).font(semibold(12.5)));
            if let Some((_, _, desc)) = FIELDS.iter().find(|(n, _, _)| *n == f.name) {
                ui.label(egui::RichText::new(format!("{} {}", tl!("Description:"), tl!(desc))).small().color(t.text_dim));
            }
            ui.add_space(4.0);
            if f.codes {
                ui.label(tl!("Field codes:"));
                ui.add(egui::TextEdit::multiline(&mut f.code).desired_rows(3).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                ui.label(egui::RichText::new(tl!("Type the field's name and its switches, for example PAGE \\* roman.")).small().color(t.text_dim));
            } else {
                field_options(ui, f, &t);
                ui.add_space(4.0);
                ui.checkbox(&mut f.preserve, tl!("Preserve formatting during updates"));
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Field code:")).font(semibold(12.0)));
        ui.label(egui::RichText::new(f.final_code()).monospace());
    });
    ui.horizontal(|ui| {
        let label = if f.codes { "Hide Codes" } else { "Field Codes" };
        if ui.button(tl!(label)).clicked() {
            if !f.codes {
                f.code = f.instruction();
            }
            f.codes = !f.codes;
        }
    });
    let params = f.params();
    let (ok, cancel) = buttons(ui, "OK", "Cancel", params.is_some());
    if ok && let Some(p) = params {
        let _ = app.run("insert.field", p);
    }
    ok || cancel
}

fn field_options(ui: &mut Ui, f: &mut FieldForm, t: &Tokens) {
    match field_kind(&f.name) {
        FieldKind::Date => {
            ui.label(tl!("Date formats:"));
            ui.add(egui::TextEdit::singleline(&mut f.date_format).desired_width(f32::INFINITY));
            egui::Frame::new().stroke(Stroke::new(1.0, t.border)).inner_margin(2.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("field_dates").max_height(170.0).auto_shrink([false, false]).show(ui, |ui| {
                    for pic in DATE_FORMATS {
                        let sample = wordcraft_engine::cmd::insert::format_date(pic);
                        if ui.selectable_label(f.date_format == *pic, sample).on_hover_text(*pic).clicked() {
                            f.date_format = (*pic).to_string();
                        }
                    }
                });
            });
        }
        FieldKind::Number | FieldKind::Sequence => {
            if field_kind(&f.name) == FieldKind::Sequence {
                ui.label(tl!("Sequence name:"));
                ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(200.0));
            }
            ui.label(tl!("Format:"));
            let shown = NUMBER_FORMATS.iter().find(|(v, _)| *v == f.number_format).map(|(_, l)| *l).unwrap_or("(none)");
            egui::ComboBox::from_id_salt("field_number").selected_text(if shown == "(none)" { tl!(shown) } else { shown }).width(200.0).show_ui(
                ui,
                |ui| {
                    for (v, l) in NUMBER_FORMATS {
                        ui.selectable_value(&mut f.number_format, (*v).to_string(), if *l == "(none)" { tl!(l) } else { l });
                    }
                },
            );
        }
        FieldKind::Bookmark => {
            ui.label(tl!("Bookmark name:"));
            if f.bookmarks.is_empty() {
                ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(200.0));
                ui.label(egui::RichText::new(tl!("The document has no bookmarks yet.")).small().color(t.text_dim));
            } else {
                egui::ComboBox::from_id_salt("field_bookmark").selected_text(f.target.clone()).width(200.0).show_ui(ui, |ui| {
                    for b in &f.bookmarks {
                        ui.selectable_value(&mut f.target, b.clone(), b);
                    }
                });
            }
        }
        FieldKind::Merge => {
            ui.label(tl!("Field name:"));
            ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(200.0));
            if !f.merge_fields.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for m in f.merge_fields.clone() {
                        if ui.selectable_label(f.target == m, &m).clicked() {
                            f.target = m;
                        }
                    }
                });
            }
        }
        FieldKind::Toc | FieldKind::Plain => {
            ui.label(egui::RichText::new(tl!("This field has no options.")).small().color(t.text_dim));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> WordApp {
        WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Default::default())
    }

    /// More Columns: two unequal columns with a line between apply as one undo step.
    #[test]
    fn columns_dialog_applies_unequal_columns_in_one_step() {
        let mut a = app();
        a.run("ui.dialog", json!({"name": "columns"})).unwrap();
        let Some(crate::dialogs::Dialog::Insert(d)) = a.dialog.take() else { panic!("no dialog") };
        let InsertDialog::Columns(mut f) = *d else { panic!("not columns") };
        assert_eq!((f.count, f.equal), (1, true));
        f.preset("left");
        assert_eq!(f.current_preset(), Some("left"));
        // Widen the first column: the second gives up the room, the total stays the text width.
        if let Some(c) = f.cols.first_mut() {
            c.0 += 0.5;
        }
        f.rebalance(0);
        let total: f32 = f.cols.iter().map(|(w, s)| w + s).sum();
        assert!((total - f.text_width).abs() < 0.01, "{total} vs {}", f.text_width);
        f.separator = true;
        let undo_before = a.session.undo_depth();
        a.run("layout.columns", f.params()).unwrap();
        let c = wordcraft_engine::cmd::page::sect(&a.session).columns;
        assert_eq!((c.count, c.separator, c.widths.len()), (2, true, 2));
        assert_eq!(a.session.undo_depth(), undo_before + 1, "one undo step");
        // The dialog reopens with what was applied.
        let again = ColumnsForm::read(&a);
        assert!(!again.equal && again.separator && again.cols.len() == 2);
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(wordcraft_engine::cmd::page::sect(&a.session).columns.count, 1);
    }

    /// The Symbol dialog inserts in the chosen font and remembers the symbol.
    #[test]
    fn symbol_dialog_inserts_with_its_font() {
        let mut a = app();
        let mut f = SymbolForm::new(&a, 0);
        f.font = "DejaVu Sans".into();
        f.selected = Some('\u{2665}');
        let p = f.params().unwrap();
        assert_eq!(p, json!({"char": "\u{2665}", "font": "DejaVu Sans"}));
        a.run("insert.symbol", p).unwrap();
        remember_symbol(&mut a, "\u{2665}", "DejaVu Sans");
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).starts_with('\u{2665}'));
        assert_eq!(a.ui.recent_symbols.first().map(|r| r.font.as_str()), Some("DejaVu Sans"));
        assert_eq!(subset_of('\u{2665}'), Some("Miscellaneous Symbols"));
        // (normal text) inserts without a font.
        f.font.clear();
        assert_eq!(f.params().unwrap(), json!({"char": "\u{2665}"}));
    }

    /// The Field dialog builds the field code from its options.
    #[test]
    fn field_dialog_builds_the_instruction() {
        let a = app();
        let mut f = FieldForm::new(&a);
        assert_eq!(f.instruction(), r#"DATE \@ "M/d/yyyy" \* MERGEFORMAT"#);
        f.date_format = "yyyy-MM-dd".into();
        f.preserve = false;
        assert_eq!(f.instruction(), r#"DATE \@ "yyyy-MM-dd""#);
        f.select("PAGE");
        f.number_format = "roman".into();
        assert_eq!(f.instruction(), r"PAGE \* roman");
        f.select("SEQ");
        f.target = "Table".into();
        assert_eq!(f.instruction(), r"SEQ Table");
        f.select("REF");
        f.target = "intro".into();
        assert_eq!(f.instruction(), r"REF intro \h");
        f.select("MERGEFIELD");
        f.target = "First Name".into();
        assert_eq!(f.params().unwrap(), json!({"instr": r#"MERGEFIELD "First Name""#, "result": "«First Name»"}));
        // Field Codes: the typed code wins.
        f.codes = true;
        f.code = "NUMPAGES \\* Arabic".into();
        assert_eq!(f.final_code(), "NUMPAGES \\* Arabic");
        f.code = "  ".into();
        assert!(f.params().is_none());
    }
}
