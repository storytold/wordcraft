use std::sync::Arc;

use craft::behavior::{ColumnWidth, Selection, SelectionChange, keyed};
use craft::font_manager::Catalog;
use craft::font_manager::picker::FontPickerSettings;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, ColumnKey, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::{Kbd, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Stack;
use craft::ui::list::{ListRow, ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::{FormCx, FormDialog};
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::env::Label;
use craft::ui::table::Table;
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{select_index, settled, text_field, word};
use crate::WordApp;
use crate::dialogs::host::{DialogHooks, FormHost, Settle, open_with};
use crate::widgets::font_box::font_picker;

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

/// What a form asks the Symbol dialog to pick: the character and font it opens on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SymbolRequest {
    /// The character the dialog opens on.
    pub ch: char,
    /// The font the dialog opens on; empty is the text's own.
    pub font: String,
    /// Whether the dialog blocks the window while it picks (a bullet for Define New Bullet).
    pub modal: bool,
}

/// The character and font a picking Symbol dialog ended with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SymbolPick {
    /// The picked character.
    pub ch: char,
    /// Its font; empty is the text's own.
    pub font: String,
}

/// What OK inserts: the command that does it, and what to remember.
struct Insertion {
    command: String,
    params: Value,
    ch: String,
    font: String,
}

/// The Symbol dialog's "from:" list: how the Character code box reads and which characters the
/// grid shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolFrom {
    /// Every character of the font; codes are Unicode code points in hex.
    #[default]
    UnicodeHex,
    /// The Windows-1252 code page in byte order; codes are decimal bytes.
    AsciiDecimal,
    /// The Windows-1252 code page in byte order; codes are hex bytes.
    AsciiHex,
}

impl SymbolFrom {
    pub const ALL: [SymbolFrom; 3] = [SymbolFrom::UnicodeHex, SymbolFrom::AsciiDecimal, SymbolFrom::AsciiHex];

    pub fn label(self) -> &'static str {
        match self {
            SymbolFrom::UnicodeHex => "Unicode (hex)",
            SymbolFrom::AsciiDecimal => "ASCII (decimal)",
            SymbolFrom::AsciiHex => "ASCII (hex)",
        }
    }
}

/// Windows-1252 bytes 0x80-0x9F (`None` = undefined in the code page).
const CP1252_HIGH: [Option<char>; 32] = [
    Some('\u{20AC}'),
    None,
    Some('\u{201A}'),
    Some('\u{0192}'),
    Some('\u{201E}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02C6}'),
    Some('\u{2030}'),
    Some('\u{0160}'),
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    Some('\u{017D}'),
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02DC}'),
    Some('\u{2122}'),
    Some('\u{0161}'),
    Some('\u{203A}'),
    Some('\u{0153}'),
    None,
    Some('\u{017E}'),
    Some('\u{0178}'),
];

/// The character a Windows-1252 byte (0x20..=0xFF) stands for.
fn cp1252_char(b: u8) -> Option<char> {
    match b {
        0x20..=0x7F | 0xA0..=0xFF => Some(char::from(b)),
        0x80..=0x9F => CP1252_HIGH.get(usize::from(b - 0x80)).copied().flatten(),
        _ => None,
    }
}

/// The Windows-1252 byte of `c`, if it has one (0x20..=0xFF).
pub fn cp1252_byte(c: char) -> Option<u8> {
    let cp = c as u32;
    if (0x20..=0x7F).contains(&cp) || (0xA0..=0xFF).contains(&cp) {
        return u8::try_from(cp).ok();
    }
    CP1252_HIGH.iter().position(|x| *x == Some(c)).and_then(|i| u8::try_from(i).ok()).map(|i| 0x80 + i)
}

/// The Character code box text for `c` in mode `from`; None when `c` has no code there.
pub fn code_text(c: char, from: SymbolFrom) -> Option<String> {
    match from {
        SymbolFrom::UnicodeHex => Some(format!("{:04X}", c as u32)),
        SymbolFrom::AsciiDecimal => cp1252_byte(c).map(|b| b.to_string()),
        SymbolFrom::AsciiHex => cp1252_byte(c).map(|b| format!("{b:02X}")),
    }
}

/// The character a typed code names in mode `from` (inverse of [`code_text`]; trims spaces).
pub fn char_from_code(code: &str, from: SymbolFrom) -> Option<char> {
    let code = code.trim();
    match from {
        SymbolFrom::UnicodeHex => {
            let hex = code.strip_prefix("U+").or_else(|| code.strip_prefix("u+")).unwrap_or(code);
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        }
        SymbolFrom::AsciiDecimal => code.parse::<u8>().ok().and_then(cp1252_char),
        SymbolFrom::AsciiHex => u8::from_str_radix(code, 16).ok().and_then(cp1252_char),
    }
}

/// The Shortcut key line: `bound` (a command's shortcut) wins; else the mode's typing method
/// (`2665, ` and `toggle_key`, the Toggle Unicode key, or `Alt+0189`); "" if none.
pub fn symbol_shortcut(c: char, from: SymbolFrom, bound: Option<&str>, toggle_key: Option<&str>) -> String {
    if let Some(k) = bound.filter(|k| !k.is_empty()) {
        return k.to_string();
    }
    match from {
        SymbolFrom::UnicodeHex => match (code_text(c, from), toggle_key.filter(|k| !k.is_empty())) {
            (Some(code), Some(key)) => format!("{code}, {key}"),
            _ => String::new(),
        },
        SymbolFrom::AsciiDecimal | SymbolFrom::AsciiHex => cp1252_byte(c).map(|b| format!("Alt+0{b:03}")).unwrap_or_default(),
    }
}

/// The from: mode that shows `c`: `from` when `c` has a code there, else Unicode (hex).
pub fn from_showing(c: char, from: SymbolFrom) -> SymbolFrom {
    if code_text(c, from).is_some() { from } else { SymbolFrom::UnicodeHex }
}

/// The characters the grid shows in mode `from`, from the font's sorted `chars`: all of them in
/// Unicode (hex); the code page's, in byte order, in the ASCII modes.
pub fn grid_chars(chars: &[char], from: SymbolFrom) -> Vec<char> {
    match from {
        SymbolFrom::UnicodeHex => chars.to_vec(),
        SymbolFrom::AsciiDecimal | SymbolFrom::AsciiHex => {
            let mut v: Vec<(u8, char)> = chars.iter().filter_map(|c| cp1252_byte(*c).map(|b| (b, *c))).collect();
            v.sort_unstable();
            v.dedup();
            v.into_iter().map(|(_, c)| c).collect()
        }
    }
}

/// AutoCorrect…: open the AutoCorrect dialog (AutoCorrect tab) with With = `c` and Replace
/// empty, as a dialog of its own (it replaces the Symbol dialog).
pub(crate) fn symbol_autocorrect(app: &mut WordApp, c: char) -> Option<DialogId> {
    let id = crate::dialogs_review::open(app, "autoCorrect", &json!({"tab": 0}), None)?;
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<crate::dialogs_review::AutoCorrectForm>>(id) {
        let form = host.form_mut();
        form.from = String::new();
        form.to = c.to_string();
    }
    Some(id)
}

/// The command (from the Special Characters table) that inserts `c`, when there is one.
fn special_command(c: char) -> Option<&'static str> {
    SPECIAL.iter().find(|(_, ch, _)| *ch == c).and_then(|(_, _, cmd)| *cmd)
}

/// The Symbol dialog: the Symbols tab (a font's characters by Unicode subset) and the Special
/// Characters tab.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SymbolForm {
    /// 0 Symbols, 1 Special Characters.
    pub tab: u8,
    /// The chosen font; empty = (normal text), the text's own font.
    pub font: String,
    pub selected: Option<char>,
    /// Character code, as the from: list reads it.
    pub code: String,
    /// The from: list: Unicode (hex), ASCII (decimal) or ASCII (hex).
    pub from: SymbolFrom,
    /// Selected row on the Special Characters tab.
    pub special: usize,
    /// The caret's font: what (normal text) shows.
    pub text_font: String,
    /// The characters the shown font has, sorted.
    #[serde(skip)]
    chars: Vec<char>,
    /// The characters the grid shows for `from` (byte order in the ASCII modes).
    #[serde(skip)]
    grid: Vec<char>,
    /// The font `chars` was read from.
    #[serde(skip)]
    chars_of: Option<String>,
    /// Scroll the grid to this character index once.
    #[serde(skip)]
    reveal: Option<usize>,
    /// The recently inserted symbols the Recently used list offers.
    #[serde(skip)]
    recent: Vec<RecentSymbol>,
}

impl SymbolForm {
    pub fn new(app: &WordApp, tab: u8) -> SymbolForm {
        let recent = app.ui.recent_symbols.first().cloned();
        let font = recent.as_ref().map(|r| r.font.clone()).unwrap_or_default();
        let selected = recent.and_then(|r| r.ch.chars().next());
        let mut f =
            SymbolForm { tab, font, selected, text_font: Self::text_font_of(app), recent: app.ui.recent_symbols.clone(), ..SymbolForm::default() };
        f.sync_code();
        f
    }

    /// The font of the text at the caret.
    fn text_font_of(app: &WordApp) -> String {
        app.session
            .doc
            .para_at(&app.session.sel.focus)
            .map(|p| app.session.doc.styles.resolve_char(p.props.style.as_deref(), &app.session.typing_props()).font)
            .unwrap_or_else(|| wordcraft_doc::styles::BODY_FONT.to_string())
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
        self.regrid();
    }

    /// Rebuild the shown grid for the font and the from: mode; keep the selection when the grid
    /// has it, otherwise select its first character.
    fn regrid(&mut self) {
        self.grid = grid_chars(&self.chars, self.from);
        if self.selected.is_none_or(|c| !self.grid.contains(&c)) {
            self.selected = self.grid.iter().copied().find(|c| *c > ' ').or(self.grid.first().copied());
        }
        self.sync_code();
        self.reveal = self.selected.and_then(|c| self.index_of(c));
    }

    /// Pick a recently used symbol `c`, inserted in `font` ("" = the text's): show its font, and
    /// switch to Unicode (hex) first when the from: mode has no code for it, so it stays selected.
    pub fn pick_recent(&mut self, font: &str, c: char) {
        let from = from_showing(c, self.from);
        let regrid = from != self.from;
        self.from = from;
        self.font = font.to_string();
        self.selected = Some(c);
        let before = self.chars_of.clone();
        self.load();
        if regrid && before == self.chars_of {
            self.regrid();
        }
        self.sync_code();
        self.reveal = self.index_of(c);
    }

    /// Change the from: mode.
    pub fn set_from(&mut self, from: SymbolFrom) {
        if self.from != from {
            self.from = from;
            self.regrid();
        }
    }

    /// Where `c` is in the shown grid.
    fn index_of(&self, c: char) -> Option<usize> {
        self.grid.iter().position(|x| *x == c)
    }

    fn sync_code(&mut self) {
        self.code = self.selected.and_then(|c| code_text(c, self.from)).unwrap_or_default();
    }

    /// Select by typed character code (read by the from: mode): the character, when the grid
    /// shows it.
    pub fn select_code(&mut self, code: &str) -> bool {
        let Some(c) = char_from_code(code, self.from) else { return false };
        if !self.grid.contains(&c) {
            return false;
        }
        self.selected = Some(c);
        self.reveal = self.index_of(c);
        true
    }

    /// Jump to a Unicode subset: its first character in this font.
    pub fn jump_to(&mut self, subset: &str) {
        let Some((_, a, b)) = SUBSETS.iter().find(|(n, _, _)| *n == subset) else { return };
        if let Some(i) = self.grid.iter().position(|c| (*a..=*b).contains(&(*c as u32))) {
            self.selected = self.grid.get(i).copied();
            self.sync_code();
            self.reveal = Some(i);
        }
    }

    /// The `insert.symbol` parameters for the selection.
    pub fn params(&self) -> Option<Value> {
        let c = self.selected?;
        Some(if self.font.is_empty() { json!({"char": c.to_string()}) } else { json!({"char": c.to_string(), "font": self.font}) })
    }

    /// What OK inserts for the shown tab; `None` while nothing is selected.
    fn insertion(&self) -> Option<Insertion> {
        if self.tab == 1 {
            // Special characters with their own command insert through it (non-breaking hyphen…).
            let (_, c, command) = SPECIAL.get(self.special)?;
            let (command, params) = match command {
                Some(id) => ((*id).to_string(), json!({})),
                None => ("insert.symbol".to_string(), json!({"char": c.to_string()})),
            };
            return Some(Insertion { command, params, ch: c.to_string(), font: String::new() });
        }
        let params = self.params()?;
        Some(Insertion { command: "insert.symbol".into(), params, ch: self.selected.map(String::from)?, font: self.font.clone() })
    }
}

/// The Symbol dialog on the dialog host: it inserts (insert mode, modeless, Insert and Close) or
/// picks a character for another dialog (`for_pick`, OK and Cancel).
pub(crate) struct SymbolDialog {
    form: SymbolForm,
    request: Option<SymbolRequest>,
    catalog: Option<Arc<Catalog>>,
    settings: FontPickerSettings,
    /// The font box changed `settings` since the app last saved them.
    settings_changed: bool,
    /// Insert mode: OK files the form here for the settle hook to insert.
    held: HeldAnswer<SymbolForm>,
    /// AutoCorrect… was pressed on this character: the settle hook opens AutoCorrect for it.
    autocorrect: Option<char>,
}

impl SymbolDialog {
    /// The hooks every Symbol dialog opens with.
    pub(crate) fn hooks() -> DialogHooks {
        DialogHooks { prepare: Some(prepare), settle }
    }

    /// Opens the inserting dialog on `page` (0 Symbols, 1 Special Characters), as a child of `parent`
    /// when one asks.
    pub(crate) fn open(app: &mut WordApp, page: u8, parent: Option<DialogId>) -> DialogId {
        let dialog = SymbolDialog {
            form: SymbolForm::new(app, page),
            request: None,
            catalog: None,
            settings: FontPickerSettings::default(),
            settings_changed: false,
            held: HeldAnswer::new(),
            autocorrect: None,
        };
        open_with(app, parent, dialog, Self::hooks())
    }

    /// The dialog picking the character `req` names; it finishes with `Some(SymbolPick)`. The
    /// app's state (fonts, recent symbols) arrives through the prepare hook.
    pub(crate) fn for_pick(req: SymbolRequest) -> SymbolDialog {
        let mut form = SymbolForm { font: req.font.clone(), selected: Some(req.ch), ..SymbolForm::default() };
        form.sync_code();
        SymbolDialog {
            form,
            request: Some(req),
            catalog: None,
            settings: FontPickerSettings::default(),
            settings_changed: false,
            held: HeldAnswer::new(),
            autocorrect: None,
        }
    }

    /// Takes the app's current state in before each pass.
    fn refresh(&mut self, catalog: Arc<Catalog>, settings: FontPickerSettings, text_font: String, recent: Vec<RecentSymbol>) {
        self.catalog = Some(catalog);
        if !self.settings_changed {
            self.settings = settings;
        }
        self.form.text_font = text_font;
        self.form.recent = recent;
        self.form.load();
    }

    /// The form OK filed, once.
    fn take_ok(&mut self) -> Option<SymbolForm> {
        self.held.take()
    }

    /// The app's answer to the filed form.
    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }
}

fn prepare(app: &mut WordApp, id: DialogId) {
    // PLAN: assumed `WordApp::{font_catalog, font_picker_settings}` (ribbon-w11, S-L284, S-L316).
    let catalog = app.font_catalog();
    let settings = app.font_picker_settings();
    let text_font = SymbolForm::text_font_of(app);
    let recent = app.ui.recent_symbols.clone();
    if let Some(d) = app.dialog_host.get_mut_by_id::<SymbolDialog>(id) {
        d.refresh(catalog, settings, text_font, recent);
    }
}

/// Inserts what OK filed, remembers it, and keeps the dialog open; writes back the font box's
/// remembered fonts.
fn settle(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let (filed, changed) = match app.dialog_host.get_mut_by_id::<SymbolDialog>(id) {
        Some(d) => {
            let changed = d.settings_changed.then(|| d.settings.clone());
            d.settings_changed = false;
            if let Some(c) = d.autocorrect.take() {
                // AutoCorrect… replaces the Symbol dialog.
                app.dialog_host.close_by_id(id);
                symbol_autocorrect(app, c);
                return Settle::Done;
            }
            (d.take_ok(), changed)
        }
        None => return Settle::Done,
    };
    if let Some(settings) = changed {
        // PLAN: assumed `WordApp::set_font_picker_settings` (ribbon-w11, S-L284, S-L316).
        app.set_font_picker_settings(settings);
    }
    let Some(form) = filed else { return settled(app, id) };
    let answer = match form.insertion() {
        None => Ok(()),
        Some(i) => match app.run(&i.command, i.params) {
            Ok(_) => {
                remember_symbol(app, &i.ch, &i.font);
                Ok(())
            }
            Err(e) => Err(Label::from(e.to_string())),
        },
    };
    if let Some(d) = app.dialog_host.get_mut_by_id::<SymbolDialog>(id) {
        d.answer_ok(answer);
    }
    settled(app, id)
}

impl StackDialog for SymbolDialog {
    type Result = Option<SymbolPick>;

    fn id(&self) -> DialogId {
        dialog_id("symbol")
    }

    fn key(&self) -> Option<&str> {
        Some("symbol")
    }

    fn modeless(&self) -> bool {
        self.request.as_ref().is_none_or(|r| !r.modal)
    }

    /// OK as a click would: a picking dialog answers with the selected character (`None` while nothing
    /// is selected); the Insert dialog holds its OK like a form's and has no programmatic one here.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Option<SymbolPick>> {
        self.request.as_ref()?;
        let ch = self.form.selected?;
        Some(Some(SymbolPick { ch, font: self.form.font.clone() }))
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        serde_json::to_value(&self.form).ok()
    }

    /// Automation sets the form's own fields (`selected`, `font`, `tab`, `code`, `special`).
    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let mut fields = serde_json::to_value(&self.form).map_err(|e| e.to_string())?;
        let map = fields.as_object_mut().ok_or_else(|| "the dialog has no fields".to_string())?;
        if !map.contains_key(field) {
            return Err(format!("no field `{field}` in this dialog"));
        }
        map.insert(field.to_string(), value);
        let mut form: SymbolForm = serde_json::from_value(fields).map_err(|e| e.to_string())?;
        // The skipped (derived) state is the dialog's own: keep it across the round trip.
        form.chars = std::mem::take(&mut self.form.chars);
        form.grid = std::mem::take(&mut self.form.grid);
        form.chars_of = self.form.chars_of.take();
        form.reveal = self.form.reveal.take();
        form.recent = std::mem::take(&mut self.form.recent);
        self.form = form;
        if field == "from" {
            self.form.regrid();
        } else if field == "selected" {
            self.form.sync_code();
        }
        Ok(())
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Option<SymbolPick>>> {
        let picking = self.request.is_some();
        let SymbolDialog { form, catalog, settings, settings_changed, held, autocorrect, .. } = self;
        if !picking {
            // An accepted insert leaves the dialog open: drop the accepted answer.
            let _ = held.finish();
        }
        let before = settings.clone();
        let mut dialog = FormDialog::new(word("Symbol"), Bind::bound(&mut *form));
        dialog = if picking {
            dialog.ok_label(word("OK"))
        } else {
            dialog.ok_label(word("Insert")).cancel_label(word("Close")).accept_stays_open(true).held(&mut *held)
        };
        let out = dialog.show(ui, |ui, cx, form| symbol_body(ui, cx, form, catalog.as_ref(), settings, picking, autocorrect));
        *settings_changed |= *settings != before;
        if picking {
            let pick = |form: &SymbolForm| form.selected.map(|ch| SymbolPick { ch, font: form.font.clone() });
            if let Some(accepted) = &out.accepted {
                return Some(match pick(accepted) {
                    Some(p) => DialogResult::Ok(Some(p)),
                    None => DialogResult::Cancel,
                });
            }
            return out.closed.map(|_| DialogResult::Cancel);
        }
        out.closed.map(|_| DialogResult::Ok(None))
    }
}

/// The dialog's body; whether OK is enabled. A double click or Enter on a cell is the OK press
/// (`FormCx::accept`).
fn symbol_body(
    ui: &mut Ui,
    cx: &mut FormCx<'_, '_>,
    form: &mut SymbolForm,
    catalog: Option<&Arc<Catalog>>,
    settings: &mut FontPickerSettings,
    picking: bool,
    autocorrect: &mut Option<char>,
) -> bool {
    Stack::new().show(ui, |ui, _| {
        if picking {
            // A bullet is one character in a font: the Special Characters tab has nothing for it.
            form.tab = 0;
        } else {
            let pages = [0u8, 1u8];
            let out = TabStrip::new(word("Symbol pages")).kind(TabKind::Text).shown(form.tab).natural_height(true).show_items(
                ui,
                &keyed(&pages, |t: &u8| *t),
                &mut |t, _| Tab::new(word(if *t == 0 { "Symbols" } else { "Special Characters" })),
            );
            for event in out.response.events {
                if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                    && let Some(t) = keys.last()
                {
                    form.tab = *t;
                }
            }
        }
        let activated = if form.tab == 1 { special_table(ui, form) } else { symbol_page(ui, form, catalog, settings, picking, autocorrect) };
        if activated {
            // PLAN: assumed `FormCx::accept` (dialogs-w5, S-L285).
            cx.accept();
        }
    });
    form.insertion().is_some()
}

/// Paints `ch` in `font` as the tile's picture. The image comes from `previews::glyph_image`.
fn paint_glyph(cx: &ContentPainterCx<'_>, rect: egui::Rect, ch: char, font: &str, dark: bool) {
    let ctx = cx.painter.ctx();
    let chr = wordcraft_doc::props::CharProps { font: Some(font.to_string()).filter(|f| !f.is_empty()), ..Default::default() };
    let ppp = ctx.pixels_per_point();
    // The texture is kept by what makes the picture, so a settled grid paints without rendering again.
    let key = egui::Id::unique(("symbol-glyph", ch, font, dark, rect.width().round() as i32, rect.height().round() as i32, (ppp * 100.0) as i32));
    let texture = match ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        Some(texture) => texture,
        None => {
            // PLAN: assumed `previews::glyph_image(..)` (ribbon-w12, dg.A6); its image goes to a texture
            // here until the tile picture hook (ribbon-w4, S-L213) takes it.
            let Some(image) = crate::previews::glyph_image(&ch.to_string(), &chr, (rect.width(), rect.height(), ppp), dark) else { return };
            let texture = ctx.load_texture("symbol-glyph", image, egui::TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(key, texture.clone()));
            texture
        }
    };
    cx.picture(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), 0.0);
}

/// The Special Characters table: the character, its name and the shortcut that inserts it. Whether a
/// row was activated.
fn special_table(ui: &mut Ui, form: &mut SymbolForm) -> bool {
    // PLAN: assumed `ColumnKey::new(&'static str)` (contract collection columns).
    let (glyph, name, keys) = (ColumnKey::new("glyph"), ColumnKey::new("name"), ColumnKey::new("keys"));
    let columns = vec![
        ColumnSpec::new(glyph.clone(), Label::Empty, ColumnWidth::fill(0.5)),
        ColumnSpec::new(name.clone(), word("Special Characters"), ColumnWidth::fill(2.0)),
        ColumnSpec::new(keys.clone(), word("Shortcut key:"), ColumnWidth::fill(1.0)),
    ];
    let positions: Vec<usize> = (0..SPECIAL.len()).collect();
    let source = keyed(&positions, |at: &usize| *at);
    let mut selection = Selection::single(form.special);
    let ctx = ui.ctx().clone();
    let events = Table::<usize>::new(word("Special Characters"), columns)
        .select(SelectMode::Single)
        .max_rows(10)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &source, &mut |at, icx| {
            let Some((label, c, command)) = SPECIAL.get(*at) else { return ListRow::new(Label::Empty) };
            let shown = if c.is_whitespace() || matches!(*c, '\u{00AD}' | '\u{200B}' | '\u{2060}') { String::new() } else { c.to_string() };
            let row = ListRow::new(shown).cells(icx.cells()).cell(name.clone(), word(label));
            // The chord comes from the framework's command chords, as the ribbon's.
            match command.and_then(|id| craft::ui::runtime::commands::chord(&ctx, &craft::ui::runtime::commands::CommandRef::from(id))) {
                Some(chord) => row.cell_field(keys.clone(), Kbd::chord(chord), |_| ()),
                None => row,
            }
        })
        .events;
    let mut activated = false;
    for event in events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(set)) => {
                if let Some(at) = set.last() {
                    form.special = *at;
                }
            }
            CollectionEvent::Activate(row) => {
                form.special = row;
                activated = true;
            }
            _ => {}
        }
    }
    activated
}

/// The Symbols tab: font, subset, the characters, recent symbols and the code. Whether a character
/// was activated.
fn symbol_page(
    ui: &mut Ui,
    form: &mut SymbolForm,
    catalog: Option<&Arc<Catalog>>,
    settings: &mut FontPickerSettings,
    picking: bool,
    autocorrect: &mut Option<char>,
) -> bool {
    form.load();
    if let Some(catalog) = catalog {
        let mut family = form.shown_font().to_string();
        let shown = family.clone();
        Field::new(word("Font:")).show(ui, |ui, _| {
            font_picker(word("Font:"), Bind::bound(&mut family), catalog, Bind::bound(settings), &[], None).show(ui);
        });
        if family != shown {
            form.font = if family == form.text_font { String::new() } else { family.trim().to_string() };
            form.load();
        }
    }
    // The ASCII modes show the code page in byte order: the subsets have no place there.
    let available: Vec<usize> = (0..if form.from == SymbolFrom::UnicodeHex { SUBSETS.len() } else { 0 })
        .filter(|i| {
            SUBSETS
                .get(*i)
                .is_some_and(|(_, a, b)| form.chars.get(form.chars.partition_point(|c| (*c as u32) < *a)).is_some_and(|c| (*c as u32) <= *b))
        })
        .collect();
    let subset_rows: Vec<Label> = available.iter().filter_map(|i| SUBSETS.get(*i)).map(|(name, _, _)| Label::from(*name)).collect();
    let current = form.selected.and_then(subset_of).and_then(|s| available.iter().position(|i| SUBSETS.get(*i).is_some_and(|(n, _, _)| *n == s)));
    if !available.is_empty()
        && let Some(at) = select_index(ui, &word("Subset:"), &subset_rows, current)
        && let Some((name, _, _)) = available.get(at).and_then(|i| SUBSETS.get(*i))
    {
        form.jump_to(name);
    }
    let dark = ui.visuals().dark_mode;
    let activated = glyph_grid(ui, form, dark);
    recent_row(ui, form, dark);
    if let Some(c) = form.selected {
        Text::new(Label::from(format!("U+{:04X}", c as u32))).show(ui);
    }
    let before = form.code.clone();
    text_field(ui, "symbol_code", &word("Character code:"), &mut form.code);
    if form.code != before {
        let code = form.code.clone();
        form.select_code(&code);
    }
    let modes: Vec<Label> = SymbolFrom::ALL.iter().map(|m| word(m.label())).collect();
    let shown = SymbolFrom::ALL.iter().position(|m| *m == form.from);
    if let Some(m) = select_index(ui, &word("from:"), &modes, shown).and_then(|at| SymbolFrom::ALL.get(at)) {
        form.set_from(*m);
    }
    // The Shortcut key line: a special character's command chord, else the mode's typing method.
    let ctx = ui.ctx().clone();
    let chord =
        |id: &'static str| craft::ui::runtime::commands::chord(&ctx, &craft::ui::runtime::commands::CommandRef::from(id)).map(|c| c.to_string());
    let shortcut = form
        .selected
        .map(|c| {
            let bound = special_command(c).and_then(chord);
            let toggle = chord("text.toggleUnicode");
            symbol_shortcut(c, form.from, bound.as_deref(), toggle.as_deref())
        })
        .unwrap_or_default();
    Field::new(word("Shortcut key:")).show(ui, |ui, _| {
        Text::new(Label::from(shortcut.clone())).show(ui);
    });
    if !picking && Button::new(word("AutoCorrect…")).id_salt("symbol_autocorrect").enabled(form.selected.is_some()).show(ui).clicked() {
        *autocorrect = form.selected;
    }
    activated
}

/// The recently inserted symbols as one row of glyphs, each in its own font; a click picks one.
fn recent_row(ui: &mut Ui, form: &mut SymbolForm, dark: bool) {
    if form.recent.is_empty() {
        return;
    }
    let positions: Vec<usize> = (0..form.recent.len()).collect();
    let source = keyed(&positions, |at: &usize| *at);
    let events = {
        let recent = &form.recent;
        TileGrid::new(word("Recently used symbols:"))
            .thumb_size(ThumbSize::Medium)
            .select(SelectMode::Single)
            .caption(CaptionPlace::None)
            .card_columns(RECENT_SYMBOLS)
            .natural_height(true)
            .selection(Bind::given(Selection::default()))
            .show_items(ui, &source, &mut |at, _| {
                let r = recent.get(*at).cloned().unwrap_or_default();
                let ch = r.ch.chars().next().unwrap_or(' ');
                let font = r.font;
                Tile::new(r.ch).thumb(ThumbAspect::SQUARE, move |cx, rect| paint_glyph(cx, rect, ch, &font, dark))
            })
            .response
            .events
    };
    for event in events {
        if let CollectionEvent::Selection(SelectionChange::Set(rows)) = event
            && let Some(r) = rows.last().and_then(|i| form.recent.get(*i)).cloned()
            && let Some(c) = r.ch.chars().next()
        {
            form.pick_recent(&r.font, c);
        }
    }
}

/// The characters of the shown font as a single-selection grid, each painted in the font; whether
/// one was activated.
fn glyph_grid(ui: &mut Ui, form: &mut SymbolForm, dark: bool) -> bool {
    if form.grid.is_empty() {
        Text::new(word("This font has no characters to show.")).show(ui);
        return false;
    }
    let positions: Vec<usize> = (0..form.grid.len()).collect();
    let source = keyed(&positions, |at: &usize| *at);
    let mut selection = form.selected.and_then(|c| form.index_of(c)).map_or_else(Selection::default, Selection::single);
    let reveal = form.reveal.take();
    let font = form.shown_font().to_string();
    let events = {
        let chars = &form.grid;
        let mut grid = TileGrid::new(word("Symbols"))
            .thumb_size(ThumbSize::Medium)
            .select(SelectMode::Single)
            .max_rows(8)
            .caption(CaptionPlace::None)
            .selection(Bind::bound(&mut selection));
        if let Some(at) = reveal {
            grid = grid.reveal(at);
        }
        grid.show_items(ui, &source, &mut |at, _| {
            let ch = chars.get(*at).copied().unwrap_or(' ');
            let font = font.clone();
            Tile::new(ch.to_string()).thumb(ThumbAspect::SQUARE, move |cx, rect| paint_glyph(cx, rect, ch, &font, dark))
        })
        .response
        .events
    };
    let (mut picked, mut activated) = (None, false);
    for event in events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(rows)) => picked = rows.last().copied().or(picked),
            CollectionEvent::Activate(row) => {
                picked = Some(row);
                activated = true;
            }
            _ => {}
        }
    }
    if let Some(c) = picked.and_then(|i| form.grid.get(i).copied()) {
        form.selected = Some(c);
        form.sync_code();
    }
    activated
}
