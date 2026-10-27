use super::*;

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
        if self.selected.is_none_or(|c| self.chars.binary_search(&c).is_err()) {
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

pub(super) fn symbol_ui(app: &mut WordApp, ui: &mut Ui, f: &mut SymbolForm) -> bool {
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
            craft_ui::ComboBox::from_id_salt("symbol_subset").selected_text(current).width(170.0).show_ui(ui, |ui| {
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
