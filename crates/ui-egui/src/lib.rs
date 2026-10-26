//! WordCraft's egui front end: a Word-style window (title bar with Quick Access Toolbar, ribbon,
//! rulers, page canvas, panes, status bar, Backstage, dialogs) over `wordcraft-engine`.
//!
//! The UI is thin: it reads `Session` state and acts through [`WordApp::run`] (command ids), so
//! the menus, ribbon, shortcuts, control channel and MCP all reach the same behaviour.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// What the operating system says about light and dark. macOS and Windows answer through winit;
/// winit reports nothing on Linux, where the desktop portal does ([`desktop_theme`]). `None` means
/// nobody answered, not a preference.
fn system_theme(ctx: &egui::Context, desktop_dark: Option<bool>) -> Option<egui::Theme> {
    ctx.system_theme().or_else(|| desktop_dark.map(|dark| if dark { egui::Theme::Dark } else { egui::Theme::Light }))
}

/// An English UI string in the current interface language ([`i18n::t`]).
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod a11y;
pub mod a11y_pane;
pub mod async_saves;
pub mod autoscroll;
pub mod backstage;
pub mod canvas;
pub mod caret_glide;
pub mod chrome;
pub mod colors;
pub mod content_controls;
pub mod control;
pub mod credits;
pub mod crop;
pub mod desktop_theme;
pub mod dialogs;
pub mod dialogs_chart;
pub mod dialogs_controls;
pub mod dialogs_insert;
pub mod dialogs_keyboard;
pub mod dialogs_layout;
pub mod dialogs_lists;
pub mod dialogs_notes;
pub mod dialogs_para;
pub mod dialogs_refs;
pub mod dialogs_review;
pub mod dialogs_ribbon;
pub mod dialogs_smart_art;
pub mod dialogs_table_style;
pub mod dialogs_templates;
pub mod documents;
pub mod equation_tab;
/// The built-in file browser, for desktops without a system file dialog (#576).
#[cfg(not(target_arch = "wasm32"))]
pub mod file_browser;
pub mod file_dialogs;
pub mod format_shape;
pub mod frame;
pub mod guides;
pub mod i18n;
pub mod ibeam;
pub mod icons;
pub mod ink;
pub mod keys;
pub mod keytips;
mod local_time;
pub mod mini_toolbar;
pub mod objects;
pub mod options;
pub mod panes;
pub mod paste_picture;
pub mod previews;
pub mod print_page;
pub mod printing;
pub mod read_aloud;
pub mod ribbon;
pub mod ribbon_fit;
pub mod ruler_tabs;
pub mod scroll;
pub mod system_clipboard;
pub mod table_mouse;
pub mod table_pen;
pub mod table_resize;
pub mod table_ruler;
pub mod theme;
pub mod updates_ui;
pub mod widgets;
pub mod window_close;
pub mod window_geometry;
mod window_ui;
mod windows;
pub mod wordart_menus;
pub mod zotero;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::Session;

pub use control::ControlRequest;
pub use file_dialogs::{FileDialogAnswer, FileDialogRequest, open_filters};

/// Platform services injected by the host (file dialogs, file I/O).
#[derive(Default)]
pub struct Services {
    /// Desktop: discover configured printers on a worker.
    pub printers: Option<printing::Discover>,
    /// Desktop: submit the PDF with the chosen settings on a worker.
    pub print_native: Option<printing::Submit>,
    /// Pick a file to open; the argument says what for (`document`, `picture`).
    pub pick_open: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Pick a path to save to, given a suggested name.
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Desktop: show a native file dialog without blocking the UI thread (#94); used instead of
    /// `pick_open` / `pick_save` when set. The host sends the picked path (`Ok(None)` when
    /// cancelled) through the returned channel once the user answers, and wakes the UI; a dropped
    /// sender counts as cancelled. An `Err` says no system dialog could be shown, and the app shows
    /// its built-in file browser instead (#576). The app shows one dialog at a time ([`file_dialogs`]).
    pub file_dialog: Option<Box<dyn Fn(FileDialogRequest) -> std::sync::mpsc::Receiver<FileDialogAnswer>>>,
    /// Web: open a file picker; the file arrives later through `inbox`.
    pub open_async: Option<Box<dyn Fn(&str)>>,
    /// Web: files (name, bytes) delivered asynchronously (picker, drag and drop).
    pub inbox: Option<Inbox>,
    /// Web: pictures pasted from the browser's clipboard (the bytes; the name is ignored). Kept
    /// apart from `inbox` because a paste only lands in the document when the page is the edit
    /// target (#1349), where a dropped or opened file always lands.
    pub pasted: Option<Inbox>,
    /// Web: hand bytes to the browser as a download. An error means no download started (the
    /// browser can't tell the page whether the user then kept the file).
    pub download: Option<Box<dyn Fn(&str, &[u8]) -> Result<(), String>>>,
    /// Web: saves that `download` started and that finish later (the browser's save picker,
    /// #1315). The hook starts a ticket for such a save and reports how it ended; until then the
    /// document's save isn't confirmed, and a New/Open held back by the Save Changes prompt waits.
    pub async_saves: Option<async_saves::AsyncSaves>,
    /// Web, framed by a page that keeps the files (`?host=parent`): set alongside `download`,
    /// Save, Save As and Export hand the bytes to that page instead, with whether it was Save As
    /// ([`HostSaveFn`]: true for a `file.saveAs` in a format that keeps everything, which File ›
    /// Export to such a format runs too, and for a Save under a name the page never sent; false
    /// for Save and for copies: the PDF and PNG exports and Save As in a format that doesn't keep
    /// everything). Save then writes the opened file
    /// under its own name and format, as on the desktop, and a Save or Save As that keeps
    /// everything makes that name the document's file, so the next Save goes to it.
    pub host_save: Option<HostSaveFn>,
    /// Web: told whether the document has unsaved changes after each pass, for the browser's
    /// leave-page guard (`beforeunload` runs between frames and can't ask the app).
    pub on_dirty: Option<Box<dyn Fn(bool)>>,
    /// Web: hand PDF bytes to the browser's print dialog (no download or intermediate file).
    /// Desktop uses `print_native` instead. An error means nothing was opened.
    pub print: Option<Box<dyn Fn(&[u8]) -> Result<(), String>>>,
    /// Desktop: the picture on the system clipboard, if any — egui's paste only carries text
    /// (#45). Read when Paste finds no text; see [`paste_picture`].
    pub clipboard_picture: Option<Box<dyn Fn() -> Option<paste_picture::ClipboardPicture>>>,
    /// Whether the system has animation effects on (Windows: Animation effects; macOS: Reduce
    /// motion; Linux and the BSDs: the desktop's setting; browsers: `prefers-reduced-motion`), or
    /// `None` when it doesn't say. What "Use system setting" follows ([`theme::Animations`]).
    /// Asked every frame, so a change in the system's settings shows at once: the host answers
    /// from memory and does any slow asking elsewhere. Without the hook, animation effects stay on.
    pub animations_enabled: Option<Box<dyn Fn() -> Option<bool>>>,
    /// Desktop: the text on the system clipboard, if any. Ctrl+V gets it from egui's paste event;
    /// the ribbon and context-menu Paste read it here (#584).
    pub clipboard_text: Option<Box<dyn Fn() -> Option<String>>>,
    /// Desktop: the HTML on the system clipboard — another program's formatted copy (a Word
    /// document, a web page) — if any. Paste keeps its formatting (#687); see [`system_clipboard`].
    pub clipboard_html: Option<Box<dyn Fn() -> Option<String>>>,
    /// Desktop: put HTML on the system clipboard with its plain-text alternative (html, text, and
    /// for a copy of one equation alone its MathML, else empty), so other programs keep the
    /// formatting (#1523) and equations (#816). The MathML goes in the platform's MathML formats
    /// as well, which Word reads a pasted equation from (#1297). `true` when the HTML and text are
    /// there; otherwise the copy's text goes through egui as usual.
    pub set_clipboard_html: Option<Box<dyn Fn(&str, &str, &str) -> bool>>,
    /// Desktop: the RTF on the system clipboard, where the platform offers it (#687).
    pub clipboard_rtf: Option<Box<dyn Fn() -> Option<String>>>,
    /// Desktop: the names of the formats on the system clipboard (MIME types, Windows format
    /// names), where the platform lists them, so a paste that finds nothing it can use says what
    /// is there instead of "empty" (#833).
    pub clipboard_formats: Option<Box<dyn Fn() -> Vec<String>>>,
    /// Web: keep a font the user added (stored file name, original bytes) for the next visit
    /// (#582). Desktop fonts are stored by the font database itself, in the app's font folder.
    pub store_font: Option<Box<dyn Fn(&str, &[u8])>>,
    /// Web: forget a stored font (its stored file name) once the user removes it.
    pub forget_font: Option<Box<dyn Fn(&str)>>,
    /// Web: request browser to query and load local installed fonts (`window.queryLocalFonts()`).
    pub load_system_fonts: Option<Box<dyn Fn()>>,
    /// Desktop: File › Options › General › Make WordCraft the default for Word documents (#295),
    /// only from that button (never a programmatic command). Windows opens the Settings app's
    /// Default apps page (apps can't set defaults themselves); Linux and BSD set it with
    /// `xdg-mime`. `None` (web, macOS) hides the button; macOS shows how to do it in Finder
    /// instead. An error means nothing changed.
    pub make_default_app: Option<Box<dyn Fn() -> Result<DefaultApp, String>>>,
    /// Desktop: the app's own user templates folder (`Templates` in the settings folder, #2852),
    /// used while File › Options names none. File › New lists personal templates from it and Save
    /// as Word Template starts in it. `None` (web) hides personal templates.
    pub templates_dir: Option<std::path::PathBuf>,
}

/// How Save in the Save Changes prompt went.
enum PromptSave {
    /// Written, keeping everything: the held-back command may run.
    Saved,
    /// Not saved (a format that doesn't keep everything); the command is cancelled.
    NotSaved,
    /// Web: the browser's save picker finishes later; its ticket ([`async_saves`]).
    Later(u64),
}

/// Web: a save the browser's picker hasn't finished ([`async_saves`]).
struct SaveInFlight {
    ticket: u64,
    /// The file name it saves as.
    name: String,
    /// The document it saved ([`Session::document_id`]).
    document: u64,
    /// Whether it marked the document saved (a Save in a format that keeps everything).
    marked_saved: bool,
}

/// Web: the command a Save Changes prompt holds back until its picker save ends.
struct AwaitingSave {
    ticket: u64,
    then: String,
    params: Value,
    /// The document the prompt asked about.
    document: u64,
    /// Its revision once the save started: a later edit isn't in that save.
    rev: u64,
}

/// What [`Services::make_default_app`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultApp {
    /// The system's default-apps settings opened; the user picks WordCraft there (Windows).
    SettingsOpened,
    /// WordCraft is now the default app for Word documents (Linux, BSD).
    MadeDefault,
}

/// Files delivered asynchronously.
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// [`Services::host_save`]: `(file name, bytes, save_as)`. An error means the page didn't get
/// the file, and the save failed.
pub type HostSaveFn = Box<dyn Fn(&str, &[u8], bool) -> Result<(), String>>;

/// Whether a pasted picture belongs in the document: the page has keyboard focus, and no
/// Backstage or dialog is open (#1349). A focused text field (comment, Find box) means the page
/// does not have focus, so `canvas_focused` already covers it; egui's own "wants keyboard input"
/// is true for the focused page itself and can't be used.
fn paste_target(canvas_focused: bool, backstage: bool, dialog: bool) -> bool {
    canvas_focused && !backstage && !dialog
}

/// UI state that persists between runs (and that agents can read and set).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    /// The ribbon tab showing. It is written to `ui.json` but not restored at startup: Word opens
    /// on Home every launch, and only going File and back to the editor keeps the tab ([`WordApp::apply_prefs`]).
    pub tab: String,
    pub backstage: bool,
    pub backstage_page: String,
    /// File › Print settings (#641); not saved.
    #[serde(skip)]
    pub print: print_page::PrintSettings,
    pub ribbon_collapsed: bool,
    /// The Quick Access Toolbar shows in the title bar (#1945). Its menu's Hide Quick Access
    /// Toolbar takes it away; Show Quick Access Toolbar (the ribbon's right-click menu, Customize
    /// Ribbon › Quick Access Toolbar) brings it back. Kept between runs.
    pub quick_access_toolbar: bool,
    pub recent: Vec<String>,
    /// Symbols inserted from the Symbol dialog, most recent first (#321).
    pub recent_symbols: Vec<dialogs_insert::RecentSymbol>,
    /// Colours picked in the Colors dialog (More Colors…), most recent first (#606).
    pub recent_colors: Vec<String>,
    /// Fonts applied from the font box, the mini toolbar or the Font dialog, most recent first:
    /// the Recently Used Fonts section at the top of the font list (#2068).
    #[serde(deserialize_with = "lenient_recent_fonts")]
    pub recent_fonts: Vec<String>,
    /// Interface theme: light, dark, or follow the OS appearance (#115).
    pub theme: theme::Appearance,
    /// Interface size (#475, #822): Auto (the display scale the system reports) or an absolute
    /// size in percent. Junk saved values read as Auto; out-of-range ones are clamped to 50–300 %.
    #[serde(rename = "uiScale")]
    pub interface_size: theme::InterfaceSize,
    /// The interface size `ui.json` held before #822, on top of the display scale: read once to
    /// migrate ([`theme::legacy_interface_percent`]), never written. A valid `uiScale` wins.
    #[serde(rename = "interfaceSize", skip_serializing)]
    pub(crate) legacy_interface_size: Option<serde_json::Value>,
    /// Animation effects (smooth scrolling, fades): on, off, or follow the system's setting, which
    /// is the default. An unknown saved value reads as the default.
    pub animations: theme::Animations,
    /// The dark-mode switch `ui.json` held before `theme` (#115): read once to migrate, never written.
    #[serde(rename = "dark", skip_serializing)]
    pub(crate) legacy_dark: Option<bool>,
    pub nav_tab: String,
    /// View › Navigation Pane: open or closed as the person left it, kept between runs (#2526).
    /// The session owns it while running (see [`WordApp::prefs`]).
    pub nav_pane: bool,
    /// Where the Styles pane sits (#1756). The session owns it while running (see [`WordApp::prefs`]).
    #[serde(deserialize_with = "lenient_styles_pane_dock")]
    pub styles_pane_dock: wordcraft_engine::StylesPaneDock,
    /// The floating Styles pane's `[x, y, width, height]`, also kept while it is docked.
    #[serde(deserialize_with = "lenient_float_rect")]
    pub styles_pane_float: Option<[f32; 4]>,
    /// The docked Styles pane's width this run; like the other panes' widths, not kept between runs.
    #[serde(skip)]
    pub styles_pane_width: Option<f32>,
    /// Where each of the other task panes sits, by [`panes::Pane::key`] (#1756).
    #[serde(deserialize_with = "panes::lenient_places")]
    pub pane_places: std::collections::BTreeMap<String, panes::SavedPane>,
    /// The task panes in the order the user put them, by [`panes::Pane::key`]: each edge shows its
    /// panes in this order, the first outermost. Empty: the default order.
    #[serde(deserialize_with = "panes::lenient_order", skip_serializing_if = "Vec::is_empty")]
    pub pane_order: Vec<String>,
    /// Headings whose sub-headings the Navigation pane hides (#1997); per session, not saved.
    #[serde(skip)]
    pub nav_collapsed: std::collections::HashSet<wordcraft_doc::Path>,
    pub show_discord: bool,
    /// User name (File › Options) for comments and tracked changes; empty keeps the default.
    pub author: String,
    /// Desktop: the main window's size and position, restored at the next launch.
    pub window: Option<window_geometry::WindowGeometry>,
    /// Interface language: `auto` (follow the system) or a code from [`i18n::LANGUAGES`].
    pub language: String,
    /// Editing preferences; the session owns them while running (see [`WordApp::prefs`]).
    pub editing: wordcraft_engine::Prefs,
    /// Read Aloud speed (1 = normal) and whether it skips citations and bibliographies.
    pub read_aloud_rate: f32,
    pub read_aloud_skip_citations: bool,
    /// Show ribbon and Quick Access Toolbar KeyTips on Alt/F10 (#620).
    pub keytips_enabled: bool,
    /// Digits entered for a multi-key Quick Access Toolbar badge; never persisted.
    #[serde(skip)]
    pub keytip_prefix: String,
    /// Keytips (Alt) state; not persisted, it resets each session.
    #[serde(skip)]
    pub keytips: crate::keytips::Phase,
    /// Alt is held according to key or modifier events; never persisted.
    #[serde(skip)]
    pub alt_pressed: bool,
    /// Since the last Alt press, another key or a mouse button was pressed: the Alt release
    /// that follows ends a chord (Alt+click, Alt+drag column selection), not a keytip tap.
    #[serde(skip)]
    pub alt_chord_used: bool,
    /// An Alt key is down according to key events. Its modifier can go before the key does:
    /// AltGr on Linux reports the Right Alt key with Level 3 Shift, not Alt (#2388).
    #[serde(skip)]
    pub alt_key_down: bool,
    /// Since the last Alt press the Alt modifier was on, or the key was Left Alt: a Right Alt
    /// that never turns it on is AltGr, which types characters and never opens keytips (#2388).
    #[serde(skip)]
    pub alt_modifier_seen: bool,
    /// View › Switch Modes: show pages dark (white text on black), kept between runs. Only the
    /// pages: the interface follows [`UiState::theme`] (#312).
    pub dark_page: bool,
    /// File › Options › Proofing › Check spelling as you type, kept between runs (#1792).
    pub check_spelling: bool,
    /// File › Options › Advanced › Animation: the caret glides to its new place and its blink
    /// fades, as Word's does on Windows (on, the default), or it jumps and blinks on and off
    /// (off).
    pub caret_animation: bool,
    /// Custom keyboard shortcuts (Customize Keyboard, #368); the session owns them while running.
    pub keyboard: wordcraft_engine::KeyMap,
    /// Custom ribbon tabs and groups, hidden tabs and the Quick Access Toolbar (Customize Ribbon,
    /// #379); the session owns them while running.
    pub ribbon: wordcraft_engine::RibbonLayout,
    /// Caption labels the user added (References › Insert Caption › New Label), kept between
    /// runs like Word keeps them in its Normal template (#402).
    pub caption_labels: Vec<String>,
    /// The label of the last caption inserted, picked again next time (#3120).
    pub last_caption_label: String,
    /// Draw › Add Pen: the pens added to the pen gallery; the session owns them while running.
    /// Entries that aren't pens are dropped on reading.
    #[serde(deserialize_with = "wordcraft_engine::cmd::draw::lenient_pens")]
    pub ink_pens: Vec<wordcraft_engine::cmd::draw::CustomPen>,
    /// File › Options › General: initials for comments; empty = the user name's. The session
    /// owns them while running, like `author`.
    pub initials: String,
    /// The File › Options pane last shown ([`wordcraft_engine::cmd::options::PANES`]).
    pub options_pane: String,
    /// File › Options › Advanced: how many recent documents File › Home and Open list (0–50).
    pub recent_count: usize,
    /// File › Options › General: show the mini toolbar over a selection.
    pub mini_toolbar: bool,
    /// File › Options › Advanced › Editing: moving a picture or shape snaps it to alignment
    /// guides (on by default; Alt moves freely).
    pub alignment_guides: bool,
    /// File › Options › General: start on File › Home (Blank document, templates, Recent) when
    /// the app opens without a document (#766); on, as in Word. See [`WordApp::start_screen`].
    pub start_screen: bool,
    /// File › Options › Display: print the page colour (off, as in Word).
    pub print_backgrounds: bool,
    /// File › Options › Display: update fields (dates, page numbers, SEQ…) before printing.
    pub update_fields_before_print: bool,
    /// The custom dictionary (File › Options › Proofing, Add to Dictionary), kept between runs.
    pub dictionary: Vec<String>,
}

/// The ribbon tab the app opens on, and the one it falls back to when the tab that was showing
/// no longer applies (a contextual tab after the selection moved away, a tab hidden in Customize
/// Ribbon). Word opens on Home every launch; see [`UiState::tab`].
pub const DEFAULT_TAB: &str = "Home";

/// ⌘Q, which only the Mac sends: quit.
const QUIT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::MAC_CMD, egui::Key::Q);
/// Most recent documents kept (File › Options › Advanced offers 0–50, as Word does).
pub const MAX_RECENT: usize = 50;
/// Most custom dictionary words read back from `ui.json`.
pub const MAX_DICTIONARY: usize = 10_000;
/// Fonts kept under Recently Used Fonts, about as many as Word shows.
pub const MAX_RECENT_FONTS: usize = 10;
/// The longest family name Recently Used Fonts keeps, the longest `format.font` accepts.
pub const MAX_FONT_NAME_BYTES: usize = 128;

impl Default for UiState {
    fn default() -> Self {
        UiState {
            tab: DEFAULT_TAB.into(),
            backstage: false,
            backstage_page: "home".into(),
            print: print_page::PrintSettings::default(),
            ribbon_collapsed: false,
            quick_access_toolbar: true,
            recent: Vec::new(),
            recent_symbols: Vec::new(),
            recent_colors: Vec::new(),
            recent_fonts: Vec::new(),
            theme: theme::Appearance::default(),
            interface_size: theme::InterfaceSize::default(),
            legacy_interface_size: None,
            animations: theme::Animations::default(),
            legacy_dark: None,
            nav_tab: "headings".into(),
            nav_pane: false,
            styles_pane_dock: wordcraft_engine::StylesPaneDock::Right,
            styles_pane_float: None,
            styles_pane_width: None,
            pane_places: Default::default(),
            pane_order: Vec::new(),
            nav_collapsed: Default::default(),
            show_discord: true,
            author: String::new(),
            window: None,
            language: i18n::AUTO.into(),
            editing: wordcraft_engine::Prefs::default(),
            read_aloud_rate: 1.0,
            read_aloud_skip_citations: true,
            keytips_enabled: true,
            keytip_prefix: String::new(),
            keytips: crate::keytips::Phase::Off,
            alt_pressed: false,
            alt_chord_used: false,
            alt_key_down: false,
            alt_modifier_seen: false,
            dark_page: false,
            check_spelling: true,
            caret_animation: true,
            keyboard: wordcraft_engine::KeyMap::default(),
            ribbon: wordcraft_engine::RibbonLayout::default(),
            caption_labels: Vec::new(),
            last_caption_label: String::new(),
            ink_pens: Vec::new(),
            initials: String::new(),
            options_pane: "general".into(),
            recent_count: 12,
            mini_toolbar: true,
            alignment_guides: true,
            start_screen: true,
            print_backgrounds: false,
            update_fields_before_print: false,
            dictionary: Vec::new(),
        }
    }
}

impl UiState {
    /// Read `ui.json` leniently: a setting with a value of the wrong type (or a newer or older
    /// version's idea of it) falls back to its default instead of discarding every setting.
    /// `None` only when the file isn't a JSON object.
    pub fn from_json(bytes: &[u8]) -> Option<UiState> {
        let mut v: Value = serde_json::from_slice(bytes).ok()?;
        if !v.is_object() {
            return None;
        }
        migrate_units(&mut v);
        Some(options::lenient(&v))
    }

    /// How many recent documents to list.
    pub fn recent_shown(&self) -> usize {
        self.recent_count.min(MAX_RECENT)
    }
}

/// Put `name` first in the Recently Used Fonts list, dropping an earlier entry for the same
/// family (ignoring case); the list keeps [`MAX_RECENT_FONTS`] names. A blank name, or one longer
/// than [`MAX_FONT_NAME_BYTES`], is not added.
pub fn push_recent_font(list: &mut Vec<String>, name: &str) {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_FONT_NAME_BYTES {
        return;
    }
    let key = name.to_lowercase();
    list.retain(|f| f.to_lowercase() != key);
    list.insert(0, name.to_string());
    list.truncate(MAX_RECENT_FONTS);
}

/// A Recently Used Fonts list from saved `names`: trimmed, without blanks, names longer than
/// [`MAX_FONT_NAME_BYTES`] or repeats (ignoring case); the first [`MAX_RECENT_FONTS`] that remain.
pub fn recent_fonts_from<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for f in names {
        if out.len() >= MAX_RECENT_FONTS {
            break;
        }
        let f = f.trim();
        if f.is_empty() || f.len() > MAX_FONT_NAME_BYTES {
            continue;
        }
        let key = f.to_lowercase();
        if !keys.contains(&key) {
            keys.push(key);
            out.push(f.to_string());
        }
    }
    out
}

/// An unknown saved position reads as the default.
fn lenient_styles_pane_dock<'de, D: serde::Deserializer<'de>>(d: D) -> Result<wordcraft_engine::StylesPaneDock, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_str().and_then(wordcraft_engine::StylesPaneDock::parse).unwrap_or_default())
}

/// Anything but four finite numbers reads as no saved rect; `craft_ui::panes` brings a saved one
/// back inside the window.
fn lenient_float_rect<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<[f32; 4]>, D::Error> {
    Ok(panes::float_rect(&Value::deserialize(d)?))
}

/// `recentFonts` as saved, through [`recent_fonts_from`]: an entry that isn't a name is dropped,
/// not the whole list.
fn lenient_recent_fonts<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_array().map(|a| recent_fonts_from(a.iter().filter_map(Value::as_str))).unwrap_or_default())
}

/// The family a command applies: `format.font {"name"}` (the font box, the mini toolbar, command
/// search) or `format.set {"props": {"font"}}` (the Font dialog). It joins Recently Used Fonts
/// once the command succeeds.
fn applied_font(id: &str, params: &Value) -> Option<String> {
    let name = match id {
        "format.font" => params.get("name"),
        "format.set" => params.get("props").and_then(|p| p.get("font")),
        _ => None,
    }?;
    let name = name.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Settings saved before #838 kept the measurement unit as `editing.units`, which defaulted to
/// inches; it now lives in `editing.measurementUnits`, and no value there follows the region. A
/// unit other than inches under the old key was the user's own choice, so it carries over (an
/// inch value can't be told from the old default, so it follows the region).
fn migrate_units(v: &mut Value) {
    let Some(editing) = v.get_mut("editing").and_then(Value::as_object_mut) else { return };
    if editing.contains_key("measurementUnits") {
        return;
    }
    if let Some(old) = editing.get("units").and_then(Value::as_str).filter(|u| *u != "inches") {
        let old = Value::String(old.to_string());
        editing.insert("measurementUnits".into(), old);
    }
}

/// The application.
pub struct WordApp {
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    pub printing: printing::State,
    print_feedback_seen: Option<printing::Feedback>,
    pub canvas: canvas::CanvasState,
    /// Content control frames, check boxes and lists on the page.
    pub controls: content_controls::ControlUi,
    pub dialog: Option<dialogs::Dialog>,
    /// The Colors dialog (More Colors…), shown above any other dialog (#606).
    pub colors: Option<colors::ColorsDialog>,
    /// The Restrict Editing pane (#412).
    pub restrict: dialogs_review::RestrictPane,
    pub status_msg: Option<(String, f64)>,
    pub previews: previews::Previews,
    /// Media key of the picture a pending Change Picture replaces (#147).
    pub change_picture_target: Option<String>,
    /// Web: a recipient list was asked for (Select Recipients › Use an Existing List…), so the
    /// next text file from the picker loads as recipients instead of opening (#240).
    pub recipient_list_pending: bool,
    /// The font database's [`generation`](wordcraft_fonts::FontDb::generation) last seen: when
    /// fonts are added or removed (by any route) the font lists and the layout are refreshed.
    font_generation: u64,
    /// A file picked for the open dialog (Templates and Add-ins › Attach…, Organizer ›
    /// Browse…); the dialog takes it on its next frame.
    pub(crate) dialog_pick: Option<String>,
    /// macOS: the window has no title bar; leave room for the traffic lights.
    pub integrated_titlebar: bool,
    /// macOS: the title bar's height and where its content starts, around the traffic lights.
    pub mac_titlebar: chrome::MacTitleBar,
    /// Focus (#1678): the ribbon is dropped down from the Focus bar, over the page.
    pub(crate) focus_ribbon: bool,
    pub(crate) updates_rt: updates_ui::Runtime,
    control_rx: Option<std::sync::mpsc::Receiver<ControlRequest>>,
    /// Admitted `ui.screenshot` requests: count/byte budgets, retries and completion slots.
    shots: control::Shots,
    /// Monotonic geometry epoch; any size/DPI/zoom/surface/document change increments it,
    /// including a change back to an earlier value.
    capture_epoch: u64,
    /// The surface reported by the last `RawInput` hook, without a document identity.
    surface_geometry: Option<control::SurfaceGeometry>,
    /// The geometry `capture_epoch` describes; `None` before the first logic pass.
    capture_geometry: Option<control::CaptureGeometry>,
    /// Synthetic input not consumed by a real UI pass yet.
    pub(crate) synthetic: Vec<egui::Event>,
    /// The batch offered to eframe is already in its pending RawInput; do not append it again.
    synthetic_pending: bool,
    /// How many events of `synthetic` the pending batch contains.
    synthetic_injected: usize,
    /// macOS: the primary button went down with Control, so it is being read as the secondary.
    control_click: bool,
    /// Badge rects recorded during layout for the current keytip phase (cleared by `logic`, painted by `show`).
    pub(crate) keytip_rects: Vec<(egui::Rect, String)>,
    /// The Navigation pane's search box takes the keyboard on its next frame (Ctrl+F, #712).
    pub(crate) nav_search_focus: bool,
    styled: bool,
    /// The CJK face order the installed UI fonts use (Chinese first, or Japanese first).
    fonts_hans: bool,
    /// Whether the installed UI fonts use the Korean CJK face order.
    fonts_korean: bool,
    /// Whether the installed UI fonts include an installed CJK fallback font (#241).
    fonts_system_cjk: bool,
    /// CJK text is (about to be) on screen in a non-CJK interface, e.g. the language names in
    /// File ▸ Options: load the installed CJK fallback font if no embedded face covers it.
    pub(crate) want_system_cjk: bool,
    /// The document (id, revision) whose comment text [`WordApp::ensure_comment_fonts`] last
    /// looked at.
    comment_fonts_for: Option<(u64, u64)>,
    /// The Recover list's document names [`WordApp::ensure_recover_fonts`] last looked at.
    recover_fonts_for: Option<u64>,
    /// The list of recent document paths [`WordApp::ensure_recent_fonts`] last looked at.
    recent_fonts_for: Vec<String>,
    /// The Find query and result count [`WordApp::ensure_find_fonts`] last looked at.
    find_fonts_for: Option<(String, usize, u64)>,
    fonts_frames: u32,
    /// The effective interface theme last installed in egui ([`theme::apply`]).
    applied_theme: Option<theme::Appearance>,
    /// The display scale (native pixels per point) seen in the last frame, 1.0 before the first;
    /// `ui.interfaceSize` reports the size Auto draws at with it.
    native_scale: f32,
    /// A pre-#822 interface size (percent on top of the display scale) waiting for the first frame
    /// that knows the display's scale, to become the absolute size it drew at
    /// ([`theme::migrate_legacy_interface_size`]).
    legacy_interface_size: Option<f64>,
    /// The desktop's light/dark choice, watched while the app runs (Linux has no winit answer).
    desktop_theme: desktop_theme::DesktopTheme,
    /// The last answer [`desktop_theme`] gave, so a frame that asks costs nothing.
    desktop_dark: Option<bool>,
    /// The interface language and the editing language the session last got for it
    /// ([`i18n::Lang::editing_lang`], or for English the region's English).
    applied_editing: Option<(i18n::Lang, Option<String>)>,
    /// What the system last said about its animation effects ([`Services::animations_enabled`]).
    system_animations: Option<bool>,
    /// Whether egui's own animations were last switched on or off ([`theme::apply_animations`]).
    applied_animations: Option<bool>,
    pub frame_ms: f64,
    /// The window title last sent; a viewport command schedules a repaint, so only send changes.
    sent_title: String,
    /// Full screen as last asked of the window, and as the window last reported it (#980).
    sent_full_screen: bool,
    seen_full_screen: Option<bool>,
    /// Nothing else (a dialog, menu, keytips, the File tab) is open to take Escape this frame, so
    /// Escape may leave full screen. Read before anything handles the frame's keys.
    escape_free: bool,
    pub quit_requested: bool,
    pub autosave: bool,
    pub word_count: (u64, usize),
    last_autosave: f64,
    /// The tab shown before the Equation tab came up (restored when editing ends).
    pub(crate) equation_prev_tab: Option<String>,
    /// SmartArt Text Pane: the item last focused (Promote, Demote and Add Shape act on it) and
    /// the item being typed in (its keystrokes join one undo step).
    pub smart_art_item: Option<usize>,
    /// A Text Pane row has focus (`smart_art_item` remembers the last one, so Tab handling in
    /// [`WordApp::logic`] only fires while a row is focused, not from the canvas).
    pub smart_art_row_focused: bool,
    pub(crate) smart_art_typing: Option<usize>,
    /// The tab shown before Outline view brought up the Outlining tab (restored when it closes).
    pub(crate) outline_prev_tab: Option<String>,
    /// Zotero commands in flight (`ui.zotero.*`).
    pub zotero: zotero::ZoteroLink,
    /// The egui context, once the first frame has run (background work wakes the UI with it).
    pub(crate) ctx: Option<egui::Context>,
    /// Whether the current command was entered by an input method's own transaction. The engine
    /// uses it to keep IME commits from setting the application-change signal.
    pub(crate) ime_origin: bool,
    /// Read Aloud: the start of the sentence the caret was last moved to, and the last error shown.
    pub(crate) read_aloud_at: Option<wordcraft_engine::doc::Pos>,
    pub(crate) read_aloud_error: Option<String>,
    /// The file the document was last explicitly saved to in this session; AutoSave writes only
    /// there. A file that was merely opened isn't rewritten until the user saves it: saving drops
    /// whatever WordCraft can't represent (content controls, charts, macros…).
    autosave_path: Option<std::path::PathBuf>,
    /// Web: saves the browser's picker hasn't finished yet ([`async_saves`], #1315).
    saves_in_flight: Vec<SaveInFlight>,
    /// Web: the command a Save Changes prompt held back until its picker save ends (#1315).
    awaiting_save: Option<AwaitingSave>,
    /// The file dialog the host is showing, and what its answer is for ([`file_dialogs`]).
    file_dialog: Option<file_dialogs::PendingDialog>,
    /// The built-in file browser, when it is open, and the folder it showed last (#576).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) file_browser: file_browser::State,
    /// File › Info: the property field being typed in and the document revision after its last
    /// keystroke, so one visit's keystrokes make a single undo step (and anything else in
    /// between, such as an Undo, starts a new one).
    pub(crate) info_editing: Option<(&'static str, u64)>,
    recovery_seen: bool,
    /// Started on the Start screen with nothing to open (#1129): the id of the placeholder
    /// blank document the session holds until the user creates or opens one. While it is still
    /// the session's document, no document is open: the window shows no page and only the File
    /// tab, Help and settings work. See [`WordApp::has_document`].
    no_document: Option<u64>,
    /// Comment and reply text typed but not yet written to the document (#1348).
    pub(crate) comment_drafts: panes::CommentDrafts,
    /// The document the window's per-document state belongs to; when the session's active
    /// document is another, [`WordApp::sync_document`] puts that state aside and brings the
    /// other's.
    shown_document: u64,
    /// The per-document state of the open documents not shown.
    parked: std::collections::HashMap<u64, documents::Parked>,
    /// The open windows ([`windows`]).
    pub(crate) windows: windows::Windows,
    /// The document the last print job was for: its window reports how the job went.
    print_document: Option<u64>,
}

/// The answer to "Do you want to save changes?" (`ui.saveChanges`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveChoice {
    Save,
    DontSave,
    Cancel,
}

impl SaveChoice {
    pub fn parse(s: &str) -> Option<SaveChoice> {
        Some(match s {
            "save" => SaveChoice::Save,
            "dontSave" => SaveChoice::DontSave,
            "cancel" => SaveChoice::Cancel,
            _ => return None,
        })
    }
}

impl WordApp {
    /// Creates the app: adopts the system locale, then initializes the document session, UI state,
    /// services, printing, canvas, dialogs, previews, control and screenshot state, font and theme
    /// tracking.
    pub fn new(session: Session, services: Services) -> Self {
        // Numeral › System follows the system locale, found here rather than in layout.
        if let Some(tag) = sys_locale::get_locale() {
            wordcraft_engine::set_system_locale(&tag);
        }
        // What the window shows starts as a new window's.
        let window_ui::WindowUi {
            canvas,
            controls,
            dialog,
            colors,
            restrict,
            status_msg,
            previews,
            change_picture_target,
            recipient_list_pending,
            dialog_pick,
            focus_ribbon,
            control_click,
            keytip_rects,
            nav_search_focus,
            comment_fonts_for,
            find_fonts_for,
            sent_title,
            sent_full_screen,
            seen_full_screen,
            escape_free,
            word_count,
            last_autosave,
            equation_prev_tab,
            smart_art_item,
            smart_art_row_focused,
            smart_art_typing,
            outline_prev_tab,
            read_aloud_at,
            awaiting_save,
            file_dialog,
            info_editing,
            shown_document,
            #[cfg(not(target_arch = "wasm32"))]
            file_browser,
            tab,
            backstage,
            backstage_page,
            print,
            nav_collapsed,
            keytip_prefix,
            keytips,
            alt_pressed,
            alt_chord_used,
            alt_key_down,
            alt_modifier_seen,
        } = window_ui::WindowUi::new(session.document_id());
        WordApp {
            session,
            ui: UiState {
                tab,
                backstage,
                backstage_page,
                print,
                nav_collapsed,
                keytip_prefix,
                keytips,
                alt_pressed,
                alt_chord_used,
                alt_key_down,
                alt_modifier_seen,
                ..UiState::default()
            },
            services,
            printing: Default::default(),
            print_feedback_seen: None,
            integrated_titlebar: false,
            mac_titlebar: chrome::MacTitleBar::default(),
            updates_rt: updates_ui::Runtime::default(),
            control_rx: None,
            shots: control::Shots::default(),
            capture_epoch: 0,
            surface_geometry: None,
            capture_geometry: None,
            synthetic: Vec::new(),
            synthetic_pending: false,
            synthetic_injected: 0,
            styled: false,
            fonts_hans: false,
            fonts_korean: false,
            fonts_system_cjk: false,
            want_system_cjk: false,
            recover_fonts_for: None,
            recent_fonts_for: Vec::new(),
            fonts_frames: 0,
            applied_theme: None,
            native_scale: 1.0,
            legacy_interface_size: None,
            // Inert until the app binary calls `watch_desktop_theme`.
            desktop_theme: desktop_theme::DesktopTheme::default(),
            desktop_dark: None,
            applied_editing: None,
            system_animations: None,
            applied_animations: None,
            frame_ms: 0.0,
            quit_requested: false,
            autosave: true,
            zotero: zotero::ZoteroLink::default(),
            ctx: None,
            ime_origin: false,
            read_aloud_error: None,
            autosave_path: None,
            saves_in_flight: Vec::new(),
            font_generation: wordcraft_fonts::FontDb::global().generation(),
            recovery_seen: false,
            no_document: None,
            comment_drafts: Default::default(),
            parked: Default::default(),
            windows: windows::Windows::new(shown_document),
            print_document: None,
            canvas,
            controls,
            dialog,
            colors,
            restrict,
            status_msg,
            previews,
            change_picture_target,
            recipient_list_pending,
            dialog_pick,
            focus_ribbon,
            control_click,
            keytip_rects,
            nav_search_focus,
            comment_fonts_for,
            find_fonts_for,
            sent_title,
            sent_full_screen,
            seen_full_screen,
            escape_free,
            word_count,
            last_autosave,
            equation_prev_tab,
            smart_art_item,
            smart_art_row_focused,
            smart_art_typing,
            outline_prev_tab,
            read_aloud_at,
            awaiting_save,
            file_dialog,
            info_editing,
            shown_document,
            #[cfg(not(target_arch = "wasm32"))]
            file_browser,
        }
    }

    pub fn with_control(mut self, rx: std::sync::mpsc::Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Start-up with no document to open (#766): show File › Home (Blank document, the templates
    /// and Recent), as Word starts, when the Start screen setting is on (the default). No document
    /// is open until the user picks Blank document or a template or opens a file (#1129): Esc or
    /// the back arrow shows an empty window with the ribbon greyed out, as Word does. Hosts call
    /// this after [`WordApp::apply_prefs`] when they have nothing to open; [`WordApp::new`] never
    /// does, so tests, `ui_shot` and windows opened for a document start in the editor. Returns
    /// whether the Start screen shows.
    pub fn start_screen(&mut self) -> bool {
        if self.ui.start_screen {
            self.ui.backstage = true;
            self.ui.backstage_page = "home".into();
            self.no_document = Some(self.session.document_id());
            self.session.release_untitled_name();
        }
        self.ui.start_screen
    }

    /// Whether a document is open: false from a Start screen launch until the user creates or
    /// opens one (#1129).
    pub fn has_document(&self) -> bool {
        self.no_document != Some(self.session.document_id())
    }

    /// Preferences to save between runs: the UI state plus the current user name and editing
    /// preferences.
    pub fn prefs(&self) -> UiState {
        let mut ui = self.ui.clone();
        ui.author = self.session.author.clone();
        ui.initials = self.session.initials.clone();
        ui.dictionary = wordcraft_engine::proof::user_dictionary();
        ui.editing = self.session.prefs.clone();
        ui.read_aloud_rate = self.session.read_aloud.rate();
        ui.read_aloud_skip_citations = self.session.read_aloud.skip_citations;
        ui.dark_page = self.session.view.dark_mode;
        ui.nav_pane = self.session.view.nav_pane;
        ui.styles_pane_dock = self.session.view.styles_pane_dock;
        ui.check_spelling = self.session.view.proofing;
        ui.keyboard = self.session.keymap.clone();
        ui.ribbon = self.session.ribbon.clone();
        ui.ink_pens = self.session.view.draw.pens.clone();
        ui
    }

    /// Restore preferences saved by [`WordApp::prefs`].
    pub fn apply_prefs(&mut self, ui: UiState) {
        self.ui = ui;
        self.ui.backstage = false;
        // Word opens on Home every launch, so the tab `ui.json` holds is not restored. Going File
        // and back to the editor within the run still returns to the tab that was showing.
        self.ui.tab = DEFAULT_TAB.into();
        let recent_fonts = recent_fonts_from(self.ui.recent_fonts.iter().map(String::as_str));
        self.ui.recent_fonts = recent_fonts;
        // A `ui.json` from before the theme setting: its dark-mode switch picks Dark or Light.
        if let Some(dark) = self.ui.legacy_dark.take() {
            self.ui.theme = if dark { theme::Appearance::Dark } else { theme::Appearance::Light };
        }
        // A `ui.json` from before #822: its interface size was on top of the display scale, so it
        // becomes absolute on the first frame that knows that scale (`logic`). A chosen `uiScale`
        // (the current key) wins; "auto" can't be told from a missing key, so the old size does.
        self.legacy_interface_size =
            self.ui.legacy_interface_size.take().filter(|_| self.ui.interface_size.is_auto()).and_then(|old| theme::legacy_interface_percent(&old));
        // The session owns the name from here on; `prefs` copies it back when saving.
        let author = std::mem::take(&mut self.ui.author);
        if !author.trim().is_empty() {
            self.session.author = author;
        }
        self.session.initials = std::mem::take(&mut self.ui.initials).chars().take(wordcraft_engine::cmd::options::MAX_INITIALS_CHARS).collect();
        for w in std::mem::take(&mut self.ui.dictionary).iter().take(MAX_DICTIONARY) {
            let w = w.trim();
            if !w.is_empty() && w.chars().count() <= wordcraft_engine::cmd::options::MAX_WORD_CHARS {
                wordcraft_engine::proof::add_word(w);
            }
        }
        self.session.prefs = std::mem::take(&mut self.ui.editing);
        // Check spelling as you type follows the saved preference (#1792).
        self.session.view.proofing = self.session.prefs.check_spelling;
        // The saved dictionaries folder (#1063) is searched from the start.
        if let Some(f) = self.session.prefs.dictionaries_folder.as_deref() {
            wordcraft_engine::cmd::options::use_dictionaries_folder(Some(f));
        }
        self.session.read_aloud.set_rate(self.ui.read_aloud_rate);
        self.session.read_aloud.skip_citations = self.ui.read_aloud_skip_citations;
        self.session.view.dark_mode = self.ui.dark_page;
        self.session.view.nav_pane = self.ui.nav_pane;
        self.session.view.styles_pane_dock = self.ui.styles_pane_dock;
        self.session.view.proofing = self.ui.check_spelling;
        // Keys for commands this version doesn't have are dropped.
        self.session.keymap = std::mem::take(&mut self.ui.keyboard);
        let reg = self.session.registry.clone();
        self.session.keymap.retain_known(&reg);
        // So are ribbon and toolbar commands.
        self.session.ribbon = std::mem::take(&mut self.ui.ribbon);
        self.session.ribbon.retain_known(&reg);
        self.session.view.draw.pens = std::mem::take(&mut self.ui.ink_pens);
    }

    /// Select the picture or icon the person just inserted or pasted, so it shows its handles.
    /// `insert.picture` leaves the caret after it, which scripts and agents keep so their next
    /// typing doesn't replace it (#965).
    pub(crate) fn select_inserted_picture(&mut self, r: &Result<Value, String>) {
        if !r.as_ref().is_ok_and(|v| v.get("media").is_some() && v.get("canvasMember").is_none()) || !self.session.sel.is_collapsed() {
            return;
        }
        let end = self.session.sel.focus.clone();
        let Some(off) = end.off.checked_sub(wordcraft_doc::para::OBJ.len_utf8()) else { return };
        let start = wordcraft_doc::Pos { off, ..end.clone() };
        if self.session.doc.para_at(&start).and_then(|p| p.object_at(off)).is_some_and(|o| o.is_drawing()) {
            self.session.sel = wordcraft_engine::Selection { anchor: start, focus: end };
        }
    }

    /// `run` for a command a mouse drag on a table sends: the view stays where it is.
    pub(crate) fn run_keep_view(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.canvas.keep_view = true;
        let r = self.run(id, params);
        self.canvas.keep_view = false;
        r
    }

    /// Run a command as the input method's own transaction while preserving every normal
    /// [`Self::run`] side effect (save prompts, clipboard, body-epoch tracking, status, …).
    ///
    /// The origin is restored on every return, including errors and nested commands.
    pub(crate) fn run_ime(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let previous = std::mem::replace(&mut self.ime_origin, true);
        let result = self.run(id, params);
        self.ime_origin = previous;
        result
    }

    /// Run a command the user asked for (ribbon, shortcut, Backstage, file drop). New, Open,
    /// Close, Envelopes, Labels and Finish & Merge on a document with unsaved changes first ask
    /// Save / Don't Save / Cancel, and the command runs once that is answered (`ui.saveChanges`).
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // Comment text still being typed is written first, so a save or export has it and a
        // close, New or Open counts it as a change (#1348). A draft the document refuses is not
        // part of it: saving and exporting go on without it (the refusal stays on the status
        // line), and replacing the document asks Save / Don't Save / Cancel about it.
        let refused = if flushes_comment_drafts(id, &params) { panes::flush_comment_drafts(self).err() } else { None };
        if id == "file.recover" && params.as_object().is_none_or(|p| p.is_empty()) {
            self.ui.backstage = true;
            self.ui.backstage_page = "recover".into();
        }
        // A button whose command needs input the user hasn't given yet opens its dialog (#240).
        if let Some(name) = input_dialog(id, &params)
            && widgets::enabled(self, id)
        {
            self.dialog = dialogs::Dialog::open(name, self);
            return Ok(json!({"pending": name}));
        }
        if (self.session.dirty || self.save_unconfirmed() || refused.is_some())
            && discards_document(id, &params)
            && !self.closes_only_its_window(id, &params)
        {
            let name = self.prompt_name();
            self.dialog = Some(dialogs::Dialog::SaveChanges {
                name,
                then: id.to_string(),
                params,
                document: self.session.document_id(),
                message: String::new(),
            });
            return Ok(json!({"pending": "saveChanges"}));
        }
        // The ribbon and context-menu Paste commands send no text: read the system clipboard's, as
        // Ctrl+V's paste event carries it (#584), and its HTML and RTF, which egui's paste event
        // doesn't carry (#687).
        let mut params = params;
        if let Some(r) = system_clipboard::fill(self, id, &mut params) {
            return r;
        }
        // Paste without system clipboard text: a picture there is newer than our own copy (copying
        // here replaces it with text), so it wins (#45).
        if id == "edit.paste"
            && params.get("text").is_none()
            && let Some(r) = paste_picture::paste(self)
        {
            return r;
        }
        let applied = applied_font(id, &params);
        let r = self.execute_user(id, params);
        // A save error already on the status line is the reason to see; the refusal shows otherwise.
        if let (Some(e), true) = (refused, r.is_ok()) {
            self.status(e);
        }
        if let (Ok(_), Some(name)) = (&r, applied) {
            push_recent_font(&mut self.ui.recent_fonts, &name);
        }
        if matches!(id, "insert.picture" | "insert.icon") {
            self.select_inserted_picture(&r);
        }
        // review.spelling only moves the selection to the next issue; without this, the ribbon's
        // Spelling & Grammar and Editor buttons looked broken — just a selection with no way to
        // see or apply a suggestion (#3340). Reuse the context menu's suggestions, as Shift+F10
        // already does for the word at the caret.
        if id == "review.spelling"
            && let Ok(v) = &r
            && v.get("done").and_then(serde_json::Value::as_bool) != Some(true)
        {
            self.canvas.context_menu_from_keys = true;
        }
        let r = system_clipboard::explain(self, id, r);
        // Check Accessibility shows its findings in the Accessibility pane, kept up to date there.
        if id == "file.accessibility" && r.is_ok() {
            self.session.view.accessibility_pane = true;
        }
        // Match Fields and Check for Errors show what they found.
        if let Ok(v) = &r
            && let Some(d) = dialogs::Dialog::report(id, v)
        {
            self.dialog = Some(d);
        }
        r
    }

    /// Run a command for a script or an agent (control channel, MCP bridge): never asks first.
    /// UI-level commands (`ui.*`) are handled here, the rest by the engine.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // A pending Change Picture only survives until the next action (#147).
        if !matches!(id, "ui.changePicture" | "picture.change" | "insert.picture") {
            self.change_picture_target = None;
        }
        if !matches!(id, "ui.openRecipientList" | "mailings.recipients") {
            self.recipient_list_pending = false;
        }
        // No document open (#1129): only the commands that open or create one, Help, and the
        // settings work.
        if !self.has_document() && !works_without_document(id) {
            let e = tl!("No document is open. Create a document or open one first.").to_string();
            self.status(e.clone());
            return Err(e);
        }
        if let Some(r) = self.ui_command(id, &params) {
            return r;
        }
        if id == "file.close" {
            return self.close_document(&params);
        }
        if id == "file.autosave" {
            return self.set_autosave(&params);
        }
        if let Some(r) = table_pen::toggle(self, id, &params) {
            return r;
        }
        let ctx = self.ctx.clone();
        if let Some(r) = zotero::command(self, id, &params, ctx.as_ref()) {
            if let Err(e) = &r {
                self.status(e.clone());
            }
            return r;
        }
        // Web: saving and exporting become downloads.
        if self.services.download.is_some() && matches!(id, "file.save" | "file.saveAs" | "file.exportPdf" | "file.exportPng") {
            let name = params.get("path").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| self.web_save_name(id));
            let name = if id == "file.exportPdf" && !name.ends_with(".pdf") { format!("{name}.pdf") } else { name };
            // A save with a password encrypts this and later saves (`file.save`'s `password`).
            if matches!(id, "file.save" | "file.saveAs")
                && let Some(pw) = params.get("password").filter(|v| !v.is_null())
            {
                if !wordcraft_engine::io::can_encrypt(&name) || pw.as_str().is_none_or(str::is_empty) {
                    return Err("only Word documents (.docx, .docm, .dotx, .dotm) can be saved with a password, and it can't be empty".into());
                }
                self.session.run("file.encrypt", &json!({"password": pw})).map_err(|e| e.to_string())?;
            }
            // A save is a save, download or not: advance the revision and modified stamp (#262).
            if matches!(id, "file.save" | "file.saveAs") {
                self.session.stamp_save();
            }
            // Word documents are encrypted with the document's password, if it has one.
            let password =
                if matches!(id, "file.save" | "file.saveAs") { self.session.password.as_ref().map(wordcraft_engine::Password::as_str) } else { None };
            let opts = wordcraft_engine::cmd::file::export_options(&self.session, &params);
            let bytes = wordcraft_engine::io::save_bytes_with(&name, &self.session.doc, password, &opts)?;
            // A ticket left over from an earlier call isn't this save's.
            let saves = self.services.async_saves.clone();
            if let Some(saves) = &saves {
                let _ = saves.take_started();
            }
            // An export is a copy, and so is a save in a format that doesn't keep everything (a
            // page image, plain text): the document itself still has unsaved changes.
            let marked_saved = matches!(id, "file.save" | "file.saveAs") && keeps_everything(&name);
            // Save As tells the host page the document is now this file, which it only is when the
            // save marks it saved. A Save under a name the page never sent (an untitled document,
            // an imported .doc saved as .docx) is a new file too, never an overwrite.
            let opened_name = self.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string());
            let new_file = id == "file.saveAs" || opened_name.as_deref() != Some(name.as_str());
            let delivered = match (&self.services.host_save, &self.services.download) {
                (Some(host), _) => host(&name, &bytes, marked_saved && new_file),
                (None, Some(download)) => download(&name, &bytes),
                (None, None) => Ok(()),
            };
            if let Err(e) = delivered {
                let msg = i18n::fmt(tl!("Couldn't download {name}: {error}"), &[("name", &name), ("error", &e)]);
                log::error!("download of {name} failed: {e}");
                self.status(msg.clone());
                return Err(msg);
            }
            if marked_saved {
                self.session.dirty = false;
                // The host page keeps the file under this name now, as a desktop save would.
                if self.services.host_save.is_some() {
                    self.session.path = Some(std::path::PathBuf::from(&name));
                }
            }
            // The browser's picker writes the file later (#1315): until it says how that went,
            // the document's save isn't confirmed.
            if let Some(ticket) = saves.as_ref().and_then(async_saves::AsyncSaves::take_started) {
                let document = self.session.document_id();
                self.saves_in_flight.push(SaveInFlight { ticket, name: name.clone(), document, marked_saved });
                return Ok(json!({"downloaded": name, "bytes": bytes.len(), "pendingSave": ticket}));
            }
            return Ok(json!({"downloaded": name, "bytes": bytes.len()}));
        }
        let document = self.session.document_id();
        let body_epoch = self.session.ime_body_epoch();
        let math_session = self.session.math.as_ref().map(|m| m.session);
        let composing_before = !self.canvas.ime_preedit.is_empty();
        let old_path = self.session.path.clone();
        let r = if self.ime_origin {
            self.session.run_ime(id, &params).map_err(|e| e.to_string())
        } else {
            self.session.run(id, &params).map_err(|e| e.to_string())
        };
        if self.session.ime_body_epoch() != body_epoch {
            self.canvas.ime_interrupted_preedit |= composing_before;
            // The body (or the whole document) was replaced under the same id — setText, a
            // version restore, compare, Undo/Redo. A composition belongs to the old content:
            // drop its pre-edit and ask the platform to interrupt it, even when the new bounded
            // window happens to be identical.
            self.canvas.ime_preedit.clear();
            self.canvas.ime_preedit_range = None;
            self.canvas.ime_window_cache = None;
            self.canvas.ime_interrupt_now = true;
        }
        if self.session.math.as_ref().map(|m| m.session) != math_session {
            self.canvas.ime_interrupted_preedit |= composing_before;
            // Entering, leaving or replacing the edited equation changes the IME target's
            // replacement epoch under the same document id. The old composition belongs to a
            // tree this session no longer edits.
            self.canvas.ime_preedit.clear();
            self.canvas.ime_preedit_range = None;
            self.canvas.ime_interrupt_now = true;
        }
        // Copy and Cut put what they took on the system clipboard, whichever control ran them
        // (the ribbon, a context menu, the native menu, or the Cmd+C/X event). Only the key event
        // used to write it, so a Cut button deleted the words without copying them.
        if matches!(id, "edit.copy" | "edit.cut")
            && let Ok(v) = &r
            && let Some(ctx) = &ctx
        {
            // Its HTML too when it has some (an equation as MathML, #816).
            crate::system_clipboard::put_result(self, ctx, v);
        }
        // Another document is shown (not when Open only showed its picker): the window's state
        // for the one it showed is put aside.
        if self.session.document_id() != document {
            // A requested New/Open wins over the asynchronous startup recovery scan.
            // Older copies remain available through File > Recover.
            self.recovery_seen = true;
            self.sync_document();
        }
        // An explicit save to the document's own file lets AutoSave keep writing there.
        if let Ok(v) = &r
            && matches!(id, "file.save" | "file.saveAs")
            && v.get("saved").and_then(Value::as_bool) == Some(true)
            && v.get("path").and_then(Value::as_str).map(std::path::Path::new) == self.session.path.as_deref()
        {
            self.autosave_path = self.session.path.clone();
        }
        // A renamed file is the same file: AutoSave keeps covering it, and Recent drops the old name.
        if r.is_ok()
            && id == "file.rename"
            && let Some(old) = old_path.filter(|old| self.session.path.as_ref() != Some(old))
        {
            if self.autosave_path.as_ref() == Some(&old) {
                self.autosave_path = self.session.path.clone();
            }
            let old = old.to_string_lossy();
            self.ui.recent.retain(|r| *r != old);
        }
        if let Ok(v) = &r {
            self.keep_web_font(id, &params, v);
        }
        self.after_command(id);
        if let Err(e) = &r {
            self.status(e.clone());
        }
        r
    }

    /// Web: store a font added from bytes, or forget the removed ones ([`Services::store_font`]).
    fn keep_web_font(&self, id: &str, params: &Value, result: &Value) {
        match id {
            "font.add" => {
                if let (Some(store), Some(file), Some(data)) =
                    (&self.services.store_font, result.get("file").and_then(Value::as_str), params.get("data").and_then(Value::as_str))
                    && let Some(bytes) = wordcraft_engine::cmd::insert::base64_decode(data)
                {
                    store(file, &bytes);
                }
            }
            "font.remove" => {
                if let Some(forget) = &self.services.forget_font {
                    for f in result.get("removed").and_then(Value::as_array).into_iter().flatten() {
                        if let Some(file) = f.get("file").and_then(Value::as_str) {
                            forget(file);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Fonts were added or removed (a command, the web shell loading stored fonts): lay out
    /// again and refresh the font lists and their previews.
    fn watch_fonts(&mut self, ctx: &egui::Context) {
        let db = wordcraft_fonts::FontDb::global();
        let generation = db.generation();
        if generation == self.font_generation {
            return;
        }
        self.font_generation = generation;
        self.session.relayout();
        self.previews.fonts_changed(ctx, db.user_fonts().iter().flat_map(|f| f.families()));
        ctx.request_repaint();
    }

    /// Answer the Save Changes prompt: Save (through Save As for a new document) and carry on,
    /// carry on without saving, or cancel. A failed or cancelled save cancels too.
    fn answer_save_changes(&mut self, choice: SaveChoice) -> Result<Value, String> {
        let Some(dialogs::Dialog::SaveChanges { then, params, document, .. }) = self.dialog.take() else {
            return Err("no Save Changes prompt is open".into());
        };
        if document != self.session.document_id() {
            return Err("the document the prompt asked about has been replaced".into());
        }
        // Don't Save is the user's say-so to discard the changes, which Close otherwise refuses.
        let mut params = params;
        if choice == SaveChoice::DontSave && then == "file.close" {
            match params.as_object_mut() {
                Some(p) => {
                    p.insert("force".into(), json!(true));
                }
                None => params = json!({"force": true}),
            }
        }
        let go = match choice {
            SaveChoice::Cancel => false,
            SaveChoice::DontSave => true,
            // A new document or an import-only Word file: ask where through Save As. The command
            // runs once that has saved, on a later frame with the non-blocking dialog (#94).
            SaveChoice::Save if (self.session.path.is_none() || self.legacy_word_path()) && self.services.download.is_none() => {
                return match self.save_as(file_dialogs::AfterSave::Continue { then, params, document }) {
                    file_dialogs::Asked::Done(r) => r,
                    file_dialogs::Asked::Pending => Ok(json!({"pending": "saveAs"})),
                    file_dialogs::Asked::Busy => Ok(json!({"done": false})),
                };
            }
            SaveChoice::Save => match self.save_for_prompt() {
                Ok(PromptSave::Saved) => true,
                Ok(PromptSave::NotSaved) => false,
                // The browser's picker finishes later: the command waits for it (#1315).
                Ok(PromptSave::Later(ticket)) => {
                    let rev = self.session.rev();
                    self.awaiting_save = Some(AwaitingSave { ticket, then, params, document, rev });
                    self.status(tl!("Waiting for the save to finish…"));
                    return Ok(json!({"pending": "save"}));
                }
                Err(error) => return Err(self.save_failed(&error, then, params, document)),
            },
        };
        if !go {
            return Ok(json!({"done": false}));
        }
        self.execute_user(&then, params).map(|r| json!({"done": true, "result": r}))
    }

    /// User commands can ask for a supported Save As target or an opening password (#55).
    /// Scripts and agents use [`WordApp::execute`], which never opens these dialogs.
    pub(crate) fn execute_user(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // Legacy Word files are import-only (#2445). Only a user's implicit Save asks for
        // a supported target; scripts and explicitly supplied paths keep their normal errors.
        if id == "file.save"
            && params.get("path").and_then(Value::as_str).is_none()
            && self.has_document()
            && self.legacy_word_path()
            && self.services.download.is_none()
        {
            return match self.save_as(file_dialogs::AfterSave::Nothing) {
                file_dialogs::Asked::Done(r) => r,
                file_dialogs::Asked::Pending => Ok(json!({"pending": "saveAs"})),
                file_dialogs::Asked::Busy => Ok(json!({"saved": false})),
            };
        }
        let retry = (id == "file.open").then(|| params.clone());
        let r = self.execute(id, params);
        if let (Err(e), Some(params)) = (&r, retry)
            && wordcraft_engine::io::needs_password(e)
        {
            let path = params.get("path").and_then(Value::as_str).unwrap_or("");
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
            self.dialog = Some(dialogs::Dialog::Password { name, params, password: Default::default(), message: String::new() });
            return Ok(json!({"pending": "password"}));
        }
        r
    }

    /// Save before New/Open/Close (and the mailings that replace the document): true once the
    /// document is safely written. A save in a format that doesn't keep everything (a page
    /// image, plain text) writes a copy and leaves the document unsaved, so it doesn't count:
    /// the command is cancelled and the document stays. A save over the document's own file
    /// (a note.txt that was opened) is a save in any format (#2376). A save that fails (no permission, the
    /// file locked by another app, a missing folder) is the save's error ([`Self::save_failed`]
    /// tells the user).
    /// In the browser the save picker may write the file later ([`PromptSave::Later`], #1315).
    fn save_for_prompt(&mut self) -> Result<PromptSave, String> {
        let v = self.execute("file.save", json!({}))?;
        let saved = v.get("saved").and_then(Value::as_bool) == Some(true) || v.get("downloaded").is_some();
        if !self.safely_saved(saved) {
            return Ok(PromptSave::NotSaved);
        }
        Ok(match v.get("pendingSave").and_then(Value::as_u64) {
            Some(ticket) => PromptSave::Later(ticket),
            None => PromptSave::Saved,
        })
    }

    /// Web: a save of this document is still in the browser's picker (#1315). The document
    /// looks saved, but nothing is written yet, so New/Open ask first.
    fn save_unconfirmed(&self) -> bool {
        let document = self.session.document_id();
        self.saves_in_flight.iter().any(|s| s.marked_saved && s.document == document)
    }

    /// The document's name as the Save Changes prompt shows it: its file's name, else its title.
    fn prompt_name(&self) -> String {
        self.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.title_stem())
    }

    /// Save (or Save As) from the Save Changes prompt failed (#1445): say so plainly on the status
    /// bar and in the prompt, which comes back for Save again, Don't Save or Cancel, with the
    /// held-back command (`then`) still waiting. Nothing closes and nothing is discarded. Returns
    /// the message, the prompt answer's error.
    pub(crate) fn save_failed(&mut self, error: &str, then: String, params: Value, document: u64) -> String {
        let name = self.prompt_name();
        let message = i18n::fmt(tl!("Couldn't save {name}, so it is still open and nothing was lost: {error}"), &[("name", &name), ("error", error)]);
        log::error!("save from the Save Changes prompt failed: {error}");
        self.prompt_again(message, then, params, document)
    }

    /// The Save Changes prompt comes back with `message` (also on the status bar), the held-back
    /// command still waiting, if the document is still the one it asked about. Returns `message`.
    fn prompt_again(&mut self, message: String, then: String, params: Value, document: u64) -> String {
        self.status(message.clone());
        if document == self.session.document_id() {
            let name = self.prompt_name();
            self.dialog = Some(dialogs::Dialog::SaveChanges { name, then, params, document, message: message.clone() });
        }
        message
    }

    /// Read how the saves that finished later ended (#1315). Called every frame.
    /// A save that didn't write the file leaves the document it was for unsaved again (only that
    /// document: one that has replaced it since is left alone). The command a Save Changes prompt
    /// held back runs once its save is written and closed, if the document is still the same and
    /// unchanged since; a cancelled or failed save brings the prompt back with the reason.
    pub(crate) fn settle_async_saves(&mut self) {
        let Some(saves) = self.services.async_saves.clone() else { return };
        for (ticket, outcome) in saves.take_finished() {
            let current = self.session.document_id();
            if let Some(i) = self.saves_in_flight.iter().position(|s| s.ticket == ticket) {
                let save = self.saves_in_flight.swap_remove(i);
                match &outcome {
                    async_saves::SaveOutcome::Saved => log::info!("saved {} through the save picker", save.name),
                    async_saves::SaveOutcome::Cancelled | async_saves::SaveOutcome::Failed(_) => {
                        if save.marked_saved && save.document == current {
                            self.session.dirty = true;
                        } else if save.marked_saved {
                            // Another document is shown: the one saved is unsaved again where it waits.
                            let _ = self.session.with_document(save.document, |s| s.dirty = true);
                        }
                    }
                }
                if let async_saves::SaveOutcome::Failed(error) = &outcome {
                    let msg = i18n::fmt(tl!("Couldn't download {name}: {error}"), &[("name", &save.name), ("error", error)]);
                    log::error!("the save picker couldn't save {}: {error}", save.name);
                    self.status(msg);
                }
            }
            let Some(wait) = self.awaiting_save.take_if(|w| w.ticket == ticket) else { continue };
            if wait.document != current {
                log::info!("the document a held-back {} was for has been replaced; dropped", wait.then);
                continue;
            }
            match outcome {
                // Only when nothing changed since and no later save of it is still unconfirmed.
                async_saves::SaveOutcome::Saved if !self.session.dirty && self.session.rev() == wait.rev && !self.save_unconfirmed() => {
                    if let Err(e) = self.execute_user(&wait.then, wait.params) {
                        self.status(e);
                    }
                }
                // Edited while the picker was open (those edits may not be written yet): ask again.
                async_saves::SaveOutcome::Saved => {
                    let name = self.prompt_name();
                    self.dialog = Some(dialogs::Dialog::SaveChanges {
                        name,
                        then: wait.then,
                        params: wait.params,
                        document: wait.document,
                        message: String::new(),
                    });
                }
                async_saves::SaveOutcome::Cancelled => {
                    let name = self.prompt_name();
                    let message = i18n::fmt(tl!("Saving {name} was cancelled, so it is still open and nothing was lost."), &[("name", &name)]);
                    self.prompt_again(message, wait.then, wait.params, wait.document);
                }
                async_saves::SaveOutcome::Failed(error) => {
                    self.save_failed(&error, wait.then, wait.params, wait.document);
                }
            }
        }
    }

    /// Whether a save before New/Open/Close counts: it happened, and kept everything.
    fn safely_saved(&mut self, saved: bool) -> bool {
        if saved && self.session.dirty {
            self.status(tl!("That format doesn't keep everything, so the document is still open. Save it as a Word document (.docx) to go on."));
            return false;
        }
        saved
    }

    /// The window's close button (or the system) asked to quit. Returns true to let the window
    /// close; with unsaved changes it asks first and returns false.
    pub fn close_requested(&mut self) -> bool {
        if self.printing.busy() {
            self.status(tl!("Wait for the print job to finish submitting before closing."));
            return false;
        }
        if self.quit_requested {
            return true;
        }
        // Nothing to save, also with no document open in this window (#1129) and none elsewhere.
        if !self.has_unsaved_documents() {
            return true;
        }
        // A comment draft the document refuses is an unsaved change: the prompt can drop it.
        if self.has_document() {
            let _ = panes::flush_comment_drafts(self);
        }
        if !self.has_unsaved_documents() {
            return true;
        }
        // Each document with unsaved work is shown and asked about in turn; Cancel stops.
        self.ask_before_quit();
        self.quit_requested
    }

    /// File › Close (Mod+W, Mod+F4) closes the shown document, or only its window while other
    /// windows show the document (View › New Window), asking nothing then. Its window closes too while other
    /// windows are open, and the one focused before it comes forward (in the web app's one window,
    /// the next document shows). The last window stays, as Word's does (#1712), with no page, as
    /// after Esc on the Start screen (#1129), and File › New or Open goes on from there. `{"quit": true}`,
    /// from the window's close button, goes on quitting: the shown document's prompt has been
    /// answered, so it closes and the next document with unsaved changes is asked about
    /// ([`WordApp::ask_before_quit`]). With no document open there is nothing to close. Unsaved
    /// changes are never discarded unasked: a dirty document closes only with `{"force": true}`
    /// (Don't Save in the prompt passes it).
    fn close_document(&mut self, params: &Value) -> Result<Value, String> {
        if self.closes_only_its_window("file.close", params) && self.close_window_only() {
            return Ok(json!({"closed": true, "window": true}));
        }
        if let Some(e) = control::close_refused(self, "file.close", params) {
            return Err(e.into());
        }
        let last = self.session.documents().len() <= 1;
        if params.get("quit").and_then(Value::as_bool) == Some(true) {
            if last {
                // Don't Save: the document stays open until the app exits, but its changes,
                // and so its recovery copy, are discarded.
                if params.get("force").and_then(Value::as_bool) == Some(true) {
                    self.session.discard_recovery_copy();
                }
                self.quit_requested = true;
                return Ok(json!({"quit": true}));
            }
            self.close_shown_document()?;
            return Ok(self.ask_before_quit());
        }
        if !self.has_document() {
            return Ok(json!({"closed": false}));
        }
        self.close_shown_document()?;
        self.recovery_seen = true;
        if last {
            self.no_document = Some(self.session.document_id());
            self.session.release_untitled_name();
        }
        // The window that stays leaves the File page; another window that came forward keeps
        // its own.
        if last || !self.windows.native() {
            self.ui.backstage = false;
        }
        Ok(json!({"closed": true}))
    }

    fn after_command(&mut self, id: &str) {
        options::sync(self);
        self.canvas.caret_visible_since = now_ms();
        if id == "view.outline" {
            self.ui.nav_tab = "headings".into();
        }
        if matches!(id, "view.readColumnWidth" | "view.readTextSize" | "view.readLineSpacing") {
            self.canvas.read_force_reflow = true;
        }
        if id == "view.readMode" {
            self.canvas.read_first_visible = None;
        }
        // Drawing leaves the caret where it was: following it would pull the view off the ink.
        // A mouse drag on a table (`run_keep_view`) does the same (#1738). A caret in an empty
        // story (a footer just made) also spans the whole story: it isn't a selection of it, and
        // the view follows it. Saving, exporting, printing, the document's properties and File ›
        // Options move neither the caret nor the text, so the view stays where the user is
        // reading (#3864); AutoSave runs its save on the session and never gets here.
        let file_keeps_view = matches!(
            id,
            "file.save"
                | "file.saveAs"
                | "file.saveTemplate"
                | "file.exportPdf"
                | "file.exportPng"
                | "file.printPdf"
                | "file.print"
                | "file.printMarkup"
                | "file.properties"
                | "file.info"
                | "file.rename"
                | "file.privacy"
                | "file.privacyOptions"
                | "file.setAuthor"
                | "file.compatibility"
                | "file.encrypt"
                | "file.share"
                | "file.options"
        );
        if (self.session.selects_whole_story() && !self.session.sel.is_collapsed()) || id == "select.all" || self.canvas.keep_view {
            self.canvas.scroll_to_caret = false;
        } else if !file_keeps_view && !["ui.", "view.", "document.", "format.state", "draw."].iter().any(|p| id.starts_with(p)) {
            self.canvas.scroll_to_caret = true;
        }
        for req in std::mem::take(&mut self.session.ui_requests) {
            self.handle_request(&req);
        }
        if let Some(p) = self.session.path.as_ref().map(|p| p.to_string_lossy().to_string())
            && self.ui.recent.first() != Some(&p)
        {
            self.ui.recent.retain(|r| *r != p);
            self.ui.recent.insert(0, p);
            self.ui.recent.truncate(MAX_RECENT);
        }
        if !self.session.status.is_empty() {
            // Engine messages are English; the catalog translates the ones it knows.
            let s = std::mem::take(&mut self.session.status);
            let s = tl!(&s).to_string();
            self.status(s);
        }
        if matches!(id, "table.deleteTable")
            || (self.canvas.table_tool == Some(crate::table_pen::TableTool::Erase) && self.session.sel.focus.path.cell().is_none())
        {
            crate::table_pen::stop(self);
        }
    }

    /// Requests commands make of the UI (open a dialog…).
    pub(crate) fn handle_request(&mut self, req: &Value) {
        // Made for a document that isn't shown (an agent's command run there): its window handles
        // it when it draws.
        if let Some(document) = req.get("document").and_then(Value::as_u64)
            && document != self.session.document_id()
        {
            let app_wide = req.get("interfaceTheme").is_some() || req.get("toggle").and_then(Value::as_str) == Some("interfaceTheme");
            if app_wide || req.get("close").is_some() || !self.queue_for_window(document, req) {
                self.handle_request_for(document, req);
            }
            return;
        }
        // Ctrl+F: type straight into the Navigation pane's search box (#712).
        if req.get("focus").and_then(Value::as_str) == Some("navSearch") {
            self.nav_search_focus = true;
            self.ui.nav_tab = "results".into();
        }
        // #1861: the registered Interface Theme command drives the same setting as Options.
        if let Some(name) = req.get("interfaceTheme").and_then(Value::as_str)
            && let Some(a) = theme::Appearance::from_code(name)
        {
            self.ui.theme = a;
        }
        if req.get("toggle").and_then(Value::as_str) == Some("interfaceTheme") {
            self.ui.theme = if self.ui_is_dark() { theme::Appearance::Light } else { theme::Appearance::Dark };
        }
        if let Some(what) = req.get("open").and_then(Value::as_str) {
            match what {
                "openFile" => self.open_dialog(),
                "saveAs" => {
                    self.save_as_dialog();
                }
                "exportPdf" => self.export_dialog("pdf"),
                "exportPng" => self.export_dialog("png"),
                "insertPicture" => self.pick_picture(),
                "addFont" => self.pick_font(),
                "loadSystemFonts" => self.load_system_fonts(),
                "print" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "print".into();
                }
                "options" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "options".into();
                    if let Some(pane) = req.get("pane").and_then(Value::as_str) {
                        self.ui.options_pane = pane.into();
                    }
                }
                "pasteSpecial" => self.dialog = Some(dialogs::Dialog::paste_special(self, req)),
                "newWindow" => self.new_window(),
                other => self.dialog = dialogs::Dialog::open(other, self),
            }
        }
        if req.get("close").is_some() {
            self.quit_requested = true;
        }
    }

    /// A request made for document `document`, which isn't shown and which no other window
    /// handles: the interface theme is the app's and changes; Close closes that document if it
    /// has nothing unsaved; the rest (dialogs, panes, focus, scrolling) are about a window's page
    /// and are dropped, as no window shows the document.
    fn handle_request_for(&mut self, document: u64, req: &Value) {
        let theme_only = req.get("interfaceTheme").is_some() || req.get("toggle").and_then(Value::as_str) == Some("interfaceTheme");
        if theme_only {
            let mut req = req.clone();
            if let Some(o) = req.as_object_mut() {
                o.remove("document");
            }
            self.handle_request(&req);
            return;
        }
        if req.get("close").is_some() {
            let dirty = self.session.documents().iter().any(|d| d.id == document && d.dirty);
            if dirty {
                log::warn!("not closing document {document}: it has unsaved changes");
            } else if let Err(e) = self.session.close_document(document) {
                log::warn!("closing document {document}: {e}");
            }
            self.sync_document();
            return;
        }
        log::warn!("dropped a request for document {document}, which no window shows: {req}");
    }

    /// Follow the desktop's light/dark choice while the app runs (Linux, where winit reports no
    /// system theme; see [`desktop_theme`]). The desktop app calls this once at start-up; unit
    /// tests and `ui_shot` don't, so what they draw doesn't depend on the host's desktop theme.
    pub fn watch_desktop_theme(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.desktop_theme = desktop_theme::DesktopTheme::start(move || ctx.request_repaint());
    }

    /// Whether the interface theme setting currently resolves to dark (`System` asks the OS).
    pub fn ui_is_dark(&self) -> bool {
        self.ui.theme.is_dark(self.ctx.as_ref().and_then(|ctx| system_theme(ctx, self.desktop_dark)))
    }

    /// Whether animation effects are on: the Animation effects setting, where `System` follows
    /// what the system last reported (on when it reports nothing).
    pub fn animations_on(&self) -> bool {
        self.ui.animations.enabled(self.system_animations)
    }

    /// What the system last said about its animation effects (`None`: it doesn't say).
    pub fn system_animations(&self) -> Option<bool> {
        self.system_animations
    }

    /// Ask the host for the system's animation setting.
    fn read_system_animations(&mut self) {
        self.system_animations = self.services.animations_enabled.as_ref().and_then(|ask| ask());
    }

    /// Commands that live in the UI layer.
    fn ui_command(&mut self, id: &str, p: &Value) -> Option<Result<Value, String>> {
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        Some(Ok(match id {
            "ui.checkForUpdates" | "help.checkForUpdates" => {
                crate::updates_ui::check(self);
                Value::Null
            }
            "ui.tab" => {
                let tab = s("tab").unwrap_or("Home").to_string();
                if tab == "File" {
                    self.ui.backstage = true;
                } else {
                    self.ui.tab = tab;
                    self.ui.backstage = false;
                }
                json!({"tab": self.ui.tab})
            }
            "ui.backstage" => {
                self.ui.backstage = p.get("value").and_then(Value::as_bool).unwrap_or(!self.ui.backstage);
                if let Some(pg) = s("page") {
                    self.ui.backstage_page = pg.into();
                }
                if let Some(pane) = s("pane") {
                    self.ui.options_pane = pane.into();
                }
                json!({"backstage": self.ui.backstage})
            }
            "ui.dialog" if s("name") == Some("colors") => {
                // The font colour's More Colors… (#606); `tab`: standard or custom.
                let target = colors::Target::cmd("format.color").remember(colors::Remember::FontColor);
                let current =
                    self.session.run("format.state", &json!({})).ok().and_then(|st| st.get("color").and_then(Value::as_str).and_then(colors::parse));
                let mut d = colors::ColorsDialog::new(current, target);
                if s("tab") == Some("custom") {
                    d.tab = colors::ColorsTab::Custom;
                }
                self.colors = Some(d);
                json!({"dialog": "colors"})
            }
            "ui.dialog" => {
                self.dialog = dialogs::Dialog::open(s("name").unwrap_or(""), self);
                // New / Modify Table Style can open on a part (`"part": "firstRow"`…).
                if let Some(dialogs::Dialog::TableStyle { region, .. }) = &mut self.dialog
                    && let Some(part) = s("part").and_then(wordcraft_engine::cmd::table_style::part_named)
                {
                    *region = dialogs_table_style::part_index(part);
                }
                json!({"dialog": self.dialog.as_ref().map(|d| d.name())})
            }
            "ui.closeDialog" => {
                self.dialog = None;
                self.colors = None;
                json!({})
            }
            "ui.changePicture" => {
                let Some(media) = self.selected_picture_media() else {
                    return Some(Err("select a picture first".into()));
                };
                self.change_picture_target = Some(media);
                self.pick_picture();
                json!({"pending": self.change_picture_target.is_some()})
            }
            // The page's context menu at the caret, as Shift+F10 or the Menu key opens it (#821).
            "ui.contextMenu" => {
                self.canvas.context_menu_from_keys = true;
                self.canvas.want_focus = true;
                json!({"pending": "contextMenu"})
            }
            "ui.paneDock" => return Some(panes::dock_command(self, p)),
            "ui.collapseRibbon" => {
                self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed;
                json!({"collapsed": self.ui.ribbon_collapsed})
            }
            "ui.focusRibbon" => {
                // Focus's "⋯" (#1678): `value` true drops the ribbon down over the page, false
                // puts it away; no value toggles it. Outside Focus there is no ribbon to drop:
                // it stays put away.
                let focus = chrome::Screen::of(&self.session.view) == chrome::Screen::Focus;
                match p.get("value") {
                    None => self.focus_ribbon = focus && !self.focus_ribbon,
                    Some(Value::Bool(on)) => self.focus_ribbon = focus && *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"focusRibbon": self.focus_ribbon})
            }
            "ui.quickAccessToolbar" => {
                // Hide Quick Access Toolbar (its menu) and Show Quick Access Toolbar (the ribbon's
                // right-click menu, Customize Ribbon), #1945: `value` true shows the toolbar in
                // the title bar, false hides it; no value toggles it.
                match p.get("value") {
                    None => self.ui.quick_access_toolbar = !self.ui.quick_access_toolbar,
                    Some(Value::Bool(on)) => self.ui.quick_access_toolbar = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"quickAccessToolbar": self.ui.quick_access_toolbar})
            }
            "ui.keytips" => {
                if let Some(v) = p.get("value") {
                    let Some(enabled) = v.as_bool() else { return Some(Err("`value` must be a boolean".into())) };
                    self.ui.keytips_enabled = enabled;
                    if !enabled {
                        self.ui.keytips = keytips::Phase::Off;
                        self.ui.keytip_prefix.clear();
                    }
                }
                json!({"enabled": self.ui.keytips_enabled})
            }
            "ui.dark" => {
                // A manual Light/Dark switch (toggles what is shown when no value is given).
                let dark = p.get("value").and_then(Value::as_bool).unwrap_or(!self.ui_is_dark());
                self.ui.theme = if dark { theme::Appearance::Dark } else { theme::Appearance::Light };
                json!({"dark": dark, "theme": self.ui.theme.code()})
            }
            "ui.theme" => {
                // `light`, `dark` or `system` (follow the OS appearance); no value reads it.
                if let Some(v) = s("value") {
                    match theme::Appearance::from_code(v) {
                        Some(a) => self.ui.theme = a,
                        None => return Some(Err(format!("unknown theme {v:?}; use light, dark or system"))),
                    }
                }
                json!({"theme": self.ui.theme.code(), "dark": self.ui_is_dark()})
            }
            "ui.interfaceSize" => {
                // `auto` (the display scale the system reports) or an absolute size in percent,
                // 50–300 (clamped); no value reads it. `percent` in the result is the size the
                // interface is drawn at. Document zoom is `view.zoom` and stays as it is.
                if let Some(v) = p.get("percent").or_else(|| p.get("value")) {
                    let size = match v.as_str() {
                        Some(a) if a.trim().eq_ignore_ascii_case("auto") => Some(theme::InterfaceSize::AUTO),
                        Some(t) => t.trim().trim_end_matches('%').trim().parse::<f64>().ok().and_then(theme::InterfaceSize::from_percent),
                        None => v.as_f64().and_then(theme::InterfaceSize::from_percent),
                    };
                    match size {
                        Some(size) => {
                            self.ui.interface_size = size;
                            self.legacy_interface_size = None;
                        }
                        None => {
                            return Some(Err(format!(
                                "`percent` must be \"auto\" or a number from {} to {}",
                                theme::InterfaceSize::MIN,
                                theme::InterfaceSize::MAX
                            )));
                        }
                    }
                }
                let size = self.ui.interface_size;
                json!({"auto": size.is_auto(), "percent": size.drawn_percent(self.native_scale)})
            }
            "ui.animations" => {
                // `system` (follow the system's animation setting), `on` or `off`; no value reads
                // it. `enabled` is what that comes to now, `system` what the system reports.
                if let Some(v) = s("value") {
                    match theme::Animations::from_code(v) {
                        Some(a) => self.ui.animations = a,
                        None => return Some(Err(format!("unknown animations setting {v:?}; use system, on or off"))),
                    }
                }
                self.read_system_animations();
                json!({"animations": self.ui.animations.code(), "enabled": self.animations_on(), "system": self.system_animations})
            }
            "ui.language" => {
                // `auto` (follow the system) or a language code; anything else is an error.
                if let Some(v) = s("value") {
                    match i18n::normalize_pref(v) {
                        Some(code) => self.ui.language = code.to_string(),
                        None => {
                            let codes: Vec<&str> = i18n::Lang::all().map(i18n::Lang::code).collect();
                            return Some(Err(format!("unknown language `{v}`; use auto or one of {}", codes.join(", "))));
                        }
                    }
                }
                let lang = i18n::Lang::from_pref(&self.ui.language);
                json!({"language": self.ui.language, "effective": lang.code(), "available": i18n::Lang::all().map(|l| json!({"code": l.code(), "name": l.name()})).collect::<Vec<_>>()})
            }
            "ui.recoveryDiscard" => {
                if let Some(answer) = s("answer") {
                    let Some(dialogs::Dialog::DiscardRecovery { id, .. }) = self.dialog.clone() else {
                        return Some(Err("no recovery discard is pending".into()));
                    };
                    match answer {
                        "cancel" => {
                            self.dialog = None;
                            json!({"discarded": false})
                        }
                        "discard" => {
                            let result = self.execute("file.recover", json!({"discard": id}));
                            if result.is_ok() {
                                self.dialog = None;
                            }
                            return Some(result);
                        }
                        _ => return Some(Err("`answer` must be discard or cancel".into())),
                    }
                } else {
                    let Some(id) = s("id") else {
                        return Some(Err("`id` is required".into()));
                    };
                    let Some(info) = self.session.recovery_documents().into_iter().find(|d| d.id == id) else {
                        return Some(Err("no such recovery copy".into()));
                    };
                    self.dialog = Some(dialogs::Dialog::DiscardRecovery { id: info.id, name: info.name });
                    json!({"pending": "discardRecovery"})
                }
            }
            "ui.saveChanges" => {
                let Some(choice) = s("answer").and_then(SaveChoice::parse) else {
                    return Some(Err("`answer` must be save, dontSave or cancel".into()));
                };
                return Some(self.answer_save_changes(choice));
            }
            "ui.openFileDialog" => {
                self.open_dialog();
                json!({})
            }
            // File › Save As › Save as Word Template (#2852): the Save dialog for a .dotx, in the
            // user templates folder. Opens system UI, like `ui.openFileDialog`.
            "ui.saveAsTemplate" => {
                let dir = self.save_as_template_dialog();
                json!({"dir": dir})
            }
            // The built-in file browser (#576), for when the system's file dialog doesn't show up
            // (desktop only): `purpose` is document (default), picture, recipients or save. A system
            // dialog still waiting for an answer is given up as cancelled.
            #[cfg(not(target_arch = "wasm32"))]
            "ui.fileBrowser" => {
                self.browse_builtin(s("purpose").unwrap_or("document"));
                json!({"open": self.file_browser.open.is_some()})
            }
            // Mailings › Select Recipients › Use an Existing List…: pick a CSV/TSV/text file and
            // load it as the mail-merge recipients. Opens system UI, like `ui.openFileDialog`.
            "ui.openRecipientList" => {
                self.pick_recipient_list();
                json!({"pending": self.recipient_list_pending})
            }
            // Native printing submits the selected queue on a worker; the browser owns its dialog.
            "ui.printSettings" => return Some(printing::settings(&mut self.printing, p, self.services.printers.as_ref())),
            "ui.print" | "ui.printSubmit" => return Some(self.print_to_system(p)),
            // File › Options › General › Make WordCraft the default… changes the user's system
            // default apps (Linux) or opens system settings (Windows), so only a click on that
            // button does it ([`WordApp::make_default_app`]); scripts, the control channel and
            // MCP get an error.
            "ui.makeDefaultApp" => {
                return Some(Err(
                    "making WordCraft the default app needs a click in File › Options › General; scripts and agents can't change system defaults"
                        .into(),
                ));
            }
            "ui.discord" => {
                self.canvas.open_url = Some("https://discord.gg/artcraft".into());
                json!({})
            }
            "ui.caretAnimation" => {
                // File › Options › Advanced › Animation: `value` true or false turns the caret's
                // glide and fading blink on or off; no value reads it.
                match p.get("value") {
                    None => {}
                    Some(Value::Bool(on)) => self.ui.caret_animation = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"caretAnimation": self.ui.caret_animation})
            }
            "ui.alignmentGuides" => {
                // File › Options › Advanced › Editing: `value` true or false turns snapping to
                // alignment guides while moving pictures and shapes on or off; no value reads it.
                match p.get("value") {
                    None => {}
                    Some(Value::Bool(on)) => self.ui.alignment_guides = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"alignmentGuides": self.ui.alignment_guides})
            }
            "ui.startScreen" => {
                // File › Options › General › Start-up (#766): `value` true or false turns showing
                // File › Home at start-up on or off; no value reads it.
                match p.get("value") {
                    None => {}
                    Some(Value::Bool(on)) => self.ui.start_screen = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"startScreen": self.ui.start_screen})
            }
            // Shape Format › Text › Create Link: the next click on an empty text box links the
            // selected box to it (`shape.link` with that box as the target); Esc cancels.
            // Picture Format › Size › Crop: crop mode on the selected picture, or off (`crop`).
            "ui.crop" => match crop::toggle(self) {
                Ok(v) => v,
                Err(e) => return Some(Err(e)),
            },
            "ui.linkTextBox" => {
                let from = wordcraft_engine::cmd::objects::text_box_target(&self.session).filter(|_| widgets::enabled(self, "shape.link"));
                let Some((pos, _)) = from else { return Some(Err("select a text box that isn't linked yet first".into())) };
                self.canvas.link_from = Some(pos);
                self.status(tl!("Click the empty text box the text should continue in (Esc cancels)"));
                json!({"pending": "link"})
            }
            _ => return None,
        }))
    }

    /// Submit the selected print job; the implementation lives beside its localized controls.
    fn print_to_system(&mut self, p: &Value) -> Result<Value, String> {
        self.print_document = Some(self.session.document_id());
        printing::submit(self, p)
    }

    /// File › Options › General › Make WordCraft the default…: hand over to the host's
    /// [`Services::make_default_app`] and say in the status bar what happened. An error when the
    /// host has no hook (web, macOS). Only the button calls this: it is deliberately not a
    /// command, so nothing programmatic (control channel, MCP, macros) can change system defaults.
    pub(crate) fn make_default_app(&mut self) -> Result<Value, String> {
        let Some(hook) = self.services.make_default_app.as_ref() else {
            return Err("WordCraft can't change the default apps here; use your system's settings".into());
        };
        match hook() {
            Ok(DefaultApp::SettingsOpened) => {
                self.status(tl!("In Default apps, choose WordCraft for the document types you want it to open."));
                Ok(json!({"settingsOpened": true}))
            }
            Ok(DefaultApp::MadeDefault) => {
                self.status(tl!("WordCraft is now the default app for Word documents."));
                Ok(json!({"default": true}))
            }
            Err(e) => {
                log::warn!("make default app: {e}");
                let msg = i18n::fmt(tl!("Couldn't make WordCraft the default: {error}"), &[("error", &e)]);
                self.status(msg.clone());
                Err(msg)
            }
        }
    }

    pub fn status(&mut self, s: impl Into<String>) {
        self.status_msg = Some((s.into(), now_ms()));
    }

    /// Media key of the selected picture, if any.
    pub(crate) fn selected_picture_media(&self) -> Option<String> {
        match wordcraft_engine::cmd::objects::selected(&self.session) {
            Some((_, wordcraft_doc::para::InlineObject::Image { media, .. })) => Some(media),
            _ => None,
        }
    }

    /// Inserting a picture selects it, and the next insert would replace it. When several images
    /// arrive in one drop, put the caret after the picture placed by the previous file (#1592).
    fn caret_after_placed_picture(&mut self, placed: bool) {
        if placed {
            self.session.sel.anchor = self.session.sel.focus.clone();
        }
    }

    /// Whether `r` placed a picture in the text (and selected it). One taken into a selected
    /// Drawing Canvas leaves the canvas selected, so the next file of the drop joins it too.
    fn placed_in_text(r: &Result<Value, String>) -> bool {
        r.as_ref().is_ok_and(|v| v.get("canvasMember").is_none())
    }

    /// A picked image replaces the pending Change Picture target, else it is inserted.
    fn insert_or_change_picture(&mut self, params: Value) -> Result<Value, String> {
        let target = std::mem::take(&mut self.change_picture_target);
        match (target, self.selected_picture_media()) {
            (Some(t), Some(m)) if t == m => self.run("picture.change", params),
            _ => self.run("insert.picture", params),
        }
    }

    /// Drop a pending Change Picture once the selection moved to another picture.
    fn clear_stale_change_picture(&mut self) {
        let stale = match (&self.change_picture_target, self.selected_picture_media()) {
            (Some(t), Some(m)) => t != &m,
            (Some(_), None) => true,
            _ => false,
        };
        if stale {
            self.change_picture_target = None;
        }
    }

    /// Pick a recipient list (desktop: through the file dialog hook; web: it arrives through the
    /// inbox).
    fn pick_recipient_list(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("recipients");
            self.recipient_list_pending = true;
            return;
        }
        let _ = self.ask_file(file_dialogs::FileDialogRequest::Open { purpose: "recipients".into() }, file_dialogs::AfterPick::Recipients);
    }

    /// Load mail-merge recipients and say how many there are. A workbook with several sheets
    /// shows Select Table to choose one first (#334).
    pub(crate) fn load_recipients(&mut self, params: Value) -> Result<Value, String> {
        let r = self.run("mailings.recipients", params.clone());
        if let Ok(v) = &r
            && v.get("chooseSheet").and_then(Value::as_bool) == Some(true)
        {
            let sheets = v
                .get("sheets")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            self.dialog = Some(dialogs::Dialog::SelectTable { sheets, selected: 0, headers: true, message: String::new(), params });
        } else if let Ok(v) = &r {
            let records = v.get("records").and_then(Value::as_u64).unwrap_or(0).to_string();
            self.status(i18n::fmt(tl!("Recipients: {count}"), &[("count", &records)]));
        }
        r
    }

    /// These paths have readers but no writer, so an interactive save needs another format.
    fn legacy_word_path(&self) -> bool {
        self.session
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("doc") || e.eq_ignore_ascii_case("dot"))
    }

    /// The name a browser Save or Export without a name uses. An export is named after the
    /// document (`report.pdf`, not `report.docx.pdf`). A host page that keeps the files
    /// ([`Services::host_save`]) gets Save back under the opened file's own name and format, as
    /// the desktop app writes it, except an import-only Word 97–2003 file (#2445).
    fn web_save_name(&self, id: &str) -> String {
        match id {
            "file.exportPdf" => return format!("{}.pdf", self.title_stem()),
            "file.exportPng" => return format!("{}.png", self.title_stem()),
            _ => {}
        }
        if id == "file.save"
            && self.services.host_save.is_some()
            && !self.legacy_word_path()
            && let Some(name) = self.session.path.as_ref().and_then(|p| p.file_name())
        {
            return name.to_string_lossy().to_string();
        }
        self.default_save_name()
    }

    /// The name Save suggests: the open file's own Word format (so a .docm keeps its macros),
    /// otherwise .docx.
    fn default_save_name(&self) -> String {
        let ext = self.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_string());
        match ext {
            Some(e) if wordcraft_engine::io::is_word_package(&e) => format!("{}.{e}", self.title_stem()),
            _ => format!("{}.docx", self.title_stem()),
        }
    }

    /// The window caption is painted with the semibold interface face (Inter). A file name in a
    /// script those faces lack would otherwise be drawn as `?`. Borrow the document fonts'
    /// fallback faces, as IME composition does (#565). A CJK name uses the CJK UI font instead:
    /// those collections are too large to add here.
    fn ensure_caption_fonts(&mut self) {
        let stem = self.title_stem();
        if stem.chars().any(wordcraft_fonts::is_cjk) {
            self.want_system_cjk = true;
        }
        let new = theme::faces_for_preedit(&stem, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            // Fonts are already installed for this frame's language; load the new faces too.
            self.styled = false;
        }
    }

    /// Comment balloons and the Comments pane draw comment text and author names with the
    /// interface faces, which have no Arabic, Hebrew, Thai …: each such letter was drawn as a box.
    /// Borrow the document fonts' fallback faces for them, as the caption does. Looked at once per
    /// document revision; CJK text uses the CJK UI font instead.
    fn ensure_comment_fonts(&mut self) {
        let key = (self.session.document_id(), self.session.rev());
        if self.comment_fonts_for == Some(key) {
            return;
        }
        self.comment_fonts_for = Some(key);
        let doc = &self.session.doc;
        let mut chars = std::collections::BTreeSet::new();
        for c in doc.comments.values() {
            chars.extend(c.author.chars());
            chars.extend(doc.plain_text(wordcraft_doc::StoryRef::Part(c.part)).chars());
        }
        if chars.iter().any(|c| wordcraft_fonts::is_cjk(*c)) {
            self.want_system_cjk = true;
        }
        let text: String = chars.into_iter().collect();
        let new = theme::faces_for_preedit(&text, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            self.styled = false;
        }
    }

    /// File › Recover draws the recovered documents' names with the interface faces too: a
    /// Chinese name was a row of boxes, an Arabic or Hebrew one likewise. The same treatment as
    /// the caption (and as the Recent list, `ensure_recent_fonts`): CJK loads the CJK UI font,
    /// other scripts borrow the document fallback faces. Looked at again only when the names change.
    fn ensure_recover_fonts(&mut self) {
        use std::hash::{Hash, Hasher};
        let docs = self.session.recovery_documents();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        docs.iter().for_each(|d| d.name.hash(&mut hasher));
        let key = hasher.finish();
        if self.recover_fonts_for == Some(key) {
            return;
        }
        self.recover_fonts_for = Some(key);
        let chars: std::collections::BTreeSet<char> = docs.iter().flat_map(|d| d.name.chars()).collect();
        if chars.iter().any(|c| wordcraft_fonts::is_cjk(*c)) {
            self.want_system_cjk = true;
        }
        let text: String = chars.into_iter().collect();
        let new = theme::faces_for_preedit(&text, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            self.styled = false;
        }
    }

    /// Recent documents in Backstage (and the Start screen) show file names and paths with the
    /// interface faces. Borrow document fallback faces for characters in non-Latin scripts (#2067),
    /// as the caption and comments do.
    fn ensure_recent_fonts(&mut self) {
        if self.recent_fonts_for.as_slice() == self.ui.recent.as_slice() {
            return;
        }
        self.recent_fonts_for = self.ui.recent.clone();
        for p in &self.ui.recent {
            if p.chars().any(wordcraft_fonts::is_cjk) {
                self.want_system_cjk = true;
            }
            let new = theme::faces_for_preedit(p, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
                wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
            });
            if !new.is_empty() {
                for face in new {
                    if !self.canvas.ime_faces.iter().any(|f| f.id() == face.id()) {
                        self.canvas.ime_faces.push(face);
                    }
                }
                self.styled = false;
            }
        }
    }

    /// The Find box and the Navigation pane's Results list draw the typed query and document
    /// excerpts around each match with the interface faces, which have no Georgian, Armenian,
    /// Thai … (#3248): each such letter was drawn as a box. Borrow the document fonts' fallback
    /// faces for them, as comments and the caption do. Looked at once per query/result-count/
    /// revision change (an edit can change a result's surrounding text — adding Georgian beside
    /// an existing Latin match — without changing the query or how many matches there are, #3257
    /// review); CJK text uses the CJK UI font instead.
    fn ensure_find_fonts(&mut self) {
        let key = (self.session.find.query.clone(), self.session.find.results.len(), self.session.rev());
        if self.find_fonts_for.as_ref() == Some(&key) {
            return;
        }
        self.find_fonts_for = Some(key);
        let mut text = self.session.find.query.clone();
        for (a, b) in self.session.find.results.iter().take(500) {
            let Some(p) = self.session.doc.para_at(a) else { continue };
            let from = p.clamp(a.off.saturating_sub(30));
            let to = p.clamp((b.off + 40).min(p.len()));
            if let Some(s) = p.text.get(from..to) {
                text.push_str(s);
            }
        }
        if text.chars().any(wordcraft_fonts::is_cjk) {
            self.want_system_cjk = true;
        }
        let new = theme::faces_for_preedit(&text, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            self.styled = false;
        }
    }

    /// Document title for the title bar (and the suggested file name). An untitled document is
    /// named in the interface language, as `Document1` is in English.
    pub fn title_stem(&self) -> String {
        match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!("Document1").into()),
            None => match &self.session.file_name {
                Some(name) => name.clone(),
                None if self.session.doc.core.title.is_empty() => {
                    documents::untitled_title(&self.session.resolved_filenames().0).unwrap_or_else(|| tl!("Document1").into())
                }
                None => self.session.doc.core.title.clone(),
            },
        }
    }

    /// Terminal feedback also reaches the normal window when printing starts through control.
    pub fn poll_printing(&mut self, ctx: &egui::Context) {
        self.printing.poll(ctx);
        self.print_feedback();
    }

    /// Say how the print job went, once, in the window showing the printed document.
    fn print_feedback(&mut self) {
        if self.printing.message == self.print_feedback_seen {
            return;
        }
        self.print_feedback_seen = self.printing.message.clone();
        let Some(message @ (printing::Feedback::Submitted(_) | printing::Feedback::Failed(_) | printing::Feedback::WorkerStopped)) =
            self.printing.message.clone()
        else {
            return;
        };
        let text = printing::feedback(&message, |label| i18n::t(label).to_owned());
        self.status(text);
        // Word returns to the document once a print job is away, the same as after Save; a
        // failure stays on the Print page so the error (and the settings to fix it) are visible
        // (#3210: the Print page used to stay up after a successful submission).
        if matches!(message, printing::Feedback::Submitted(_)) && self.ui.backstage && self.ui.backstage_page == "print" {
            self.ui.backstage = false;
            self.canvas.want_focus = true;
        }
    }

    /// Per-frame logic before layout: control requests, screenshots, shortcuts, file drops.
    pub fn logic(&mut self, ctx: &egui::Context) {
        updates_ui::tick(self, ctx);
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        // New documents are written in the interface's language when WordCraft can proof it
        // (a Polish interface: Polish spelling, grammar and hyphenation from the first word).
        // An English interface writes the region's English (English (UK) on a British system,
        // #1045), a Spanish one the region's Spanish (Spanish (Uruguay) on a Uruguayan system,
        // Spain's elsewhere, #3360).
        let editing = match lang.editing_lang() {
            Some(t) => Some(t.to_string()),
            None if lang.code() == "en" => self.session.region_english(),
            None if lang.code() == "es" => Some(self.session.region_spanish()),
            None => None,
        };
        if self.applied_editing.as_ref().is_none_or(|(l, e)| *l != lang || *e != editing) {
            self.session.set_editing_language(editing.as_deref());
            self.applied_editing = Some((lang, editing));
        }
        options::sync(self);
        self.settle_async_saves();
        // Chinese text wants the Chinese face before the Japanese one (one glyph style per line).
        // An installed CJK font is read only once CJK text is shown (#241): a CJK interface, the
        // language names in Options, or a file name the caption is drawing. A file opened later
        // in this frame (dialog, drop, control channel) is picked up again at the end.
        self.ensure_caption_fonts();
        self.ensure_comment_fonts();
        self.ensure_recover_fonts();
        self.ensure_recent_fonts();
        self.ensure_find_fonts();
        self.install_ui_fonts(ctx, lang);
        if let Some(answer) = self.desktop_theme.take_change() {
            self.desktop_dark = Some(answer);
        }
        // Both palettes are installed and egui picks one: with `System` it follows an OS appearance
        // change live (winit reports it and repaints, #311). Where winit is silent (Linux) the
        // desktop portal's answer fills in instead. The interface follows the Interface theme
        // setting alone; Dark page only changes how pages are drawn (#312).
        let effective = match self.ui.theme {
            theme::Appearance::System if ctx.system_theme().is_none() => match self.desktop_dark {
                Some(true) => theme::Appearance::Dark,
                Some(false) => theme::Appearance::Light,
                None => theme::Appearance::System,
            },
            other => other,
        };
        if self.applied_theme != Some(effective) {
            theme::apply(ctx, effective);
            self.applied_theme = Some(effective);
        }
        // Interface size (#475, #822): egui's zoom factor multiplies the display scale the system
        // reports (native pixels per point). Auto keeps that scale; a chosen size divides it back
        // out, so "100%" is one screen pixel per point even where the system's scale is a guess.
        // Recomputed every frame, so a move to a monitor with another scale keeps a chosen size.
        // Keyboard zoom is off (above), so nothing else changes the factor; `set_zoom_factor`
        // repaints only on change.
        self.native_scale = theme::usable_scale(ctx.native_pixels_per_point().unwrap_or(1.0));
        if let Some(native) = ctx.native_pixels_per_point()
            && let Some(old) = self.legacy_interface_size.take()
        {
            self.ui.interface_size = theme::migrate_legacy_interface_size(old, native);
        }
        let zoom = self.ui.interface_size.zoom_factor(self.native_scale);
        if ctx.zoom_factor() != zoom {
            ctx.set_zoom_factor(zoom);
        }
        // Animation effects: the host is asked every frame what the system says (it answers from
        // memory), so a change made in the system's settings is followed without a restart.
        self.read_system_animations();
        let animate = self.animations_on();
        if self.applied_animations != Some(animate) {
            theme::apply_animations(ctx, animate);
            self.applied_animations = Some(animate);
        }
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
        // Capture geometry/window/document identity is current before control admission and
        // completion.
        self.refresh_capture_geometry(ctx);
        let native = self.windows.native();
        if native {
            self.printing.poll(ctx);
        } else {
            self.poll_printing(ctx);
        }
        self.drain_control(ctx);
        // The active window's own work; with native windows each window does it again as it
        // draws, for itself.
        self.window_work(ctx);
        self.drain_inbox();
        self.drain_pasted();
        self.watch_fonts(ctx);
        // A script, an agent or a later result may have made another document the active one.
        self.sync_document();
        let document = self.session.document_id();
        self.session.recovery_tick();
        if self.session.document_id() != document {
            self.ui.backstage = false;
            self.sync_document();
        }
        if !self.recovery_seen && self.session.recovery_ready() {
            self.recovery_seen = true;
            if !self.session.recovery_documents().is_empty() && !self.session.dirty {
                self.ui.backstage = true;
                self.ui.backstage_page = "recover".into();
            }
        }
        if let Some(wait) = self.session.recovery_wait() {
            ctx.request_repaint_after(wait);
        }
        if !native {
            self.window_input(ctx);
        }
        self.send_control_input(ctx);
        // Screenshot deadlines, geometry and completions belong to `logic`; issuing the one
        // callback command belongs to `ui` (`flush_shot_and_synthetic`). A queued/settling capture
        // needs a real UI pass, and so does an injected synthetic step, even while the window is
        // hidden.
        control::drain_screenshots(self, ctx);
        if self.shots.in_flight() || self.synthetic_pending || !self.synthetic.is_empty() {
            let viewport = self.active_viewport();
            if ctx.input_for(viewport, |i| i.viewport().visible()) != Some(true) {
                ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::RequestPaintWhileHidden);
            }
            ctx.request_repaint_of(viewport);
            ctx.request_repaint();
        }
        // The open above may have changed the file name the caption draws.
        self.ensure_caption_fonts();
        self.ensure_comment_fonts();
        self.ensure_recover_fonts();
        self.ensure_recent_fonts();
        self.ensure_find_fonts();
        self.install_ui_fonts(ctx, lang);
        self.report_dirty();
    }

    /// A window's own work, with its state in place: print feedback for its document, Zotero,
    /// Read Aloud, its file dialog, AutoSave of its document. Runs in [`WordApp::logic`] for the
    /// active window and, with native windows, as each window draws.
    fn window_work(&mut self, ctx: &egui::Context) {
        if self.windows.native() {
            let doc = self.session.document_id();
            if self.print_document.is_none_or(|d| d == doc || !self.session.is_open(d)) {
                self.print_feedback();
            }
        }
        zotero::poll(self, ctx);
        read_aloud::poll(self, ctx);
        self.clear_stale_change_picture();
        let _ = self.poll_file_dialog();
        self.autosave_tick(now_ms());
        if self.autosaves() && self.session.dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(1000));
        }
    }

    /// A window's own input before it draws: ⌘Q. Runs in [`WordApp::logic`] with one window,
    /// else as each window draws (each native window has its own input). The SmartArt Text
    /// Pane's Tab keys and dropped files are read in the first UI pass
    /// ([`WordApp::handle_ui_input_events`]).
    fn window_input(&mut self, ctx: &egui::Context) {
        if self.windows.native() && ctx.input_mut(|i| i.consume_shortcut(&QUIT)) {
            self.quit();
        }
        // The drawn window's caption, comments and Find results borrow faces it may not have.
        if self.windows.native() {
            self.ensure_caption_fonts();
            self.ensure_comment_fonts();
            self.ensure_find_fonts();
        }
    }

    /// Files dropped on the native window. Raw input is taken only in the first UI pass: a
    /// hidden `run_logic` tick leaves the previous shown frame's events stale, and replaying a
    /// drop would open or insert it again.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_dropped_files(&mut self, files: Vec<egui::DroppedFileHandle>) {
        let before = self.session.document_id();
        let mut placed_picture = false;
        for f in &files {
            let path = f.path().to_string_lossy().to_string();
            let lp = path.to_ascii_lowercase();
            let img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".wmf", ".emf"].iter().any(|e| lp.ends_with(e));
            let sheet = wordcraft_engine::cmd::mailings::is_spreadsheet_name(&lp);
            let _ = if img {
                self.caret_after_placed_picture(placed_picture);
                let r = self.run("insert.picture", json!({"path": path}));
                placed_picture = Self::placed_in_text(&r);
                r
            } else if sheet {
                // A spreadsheet isn't a document: it's a recipient list (#334).
                self.load_recipients(json!({"path": path}))
            } else if wordcraft_fonts::is_font_file_name(&lp) {
                self.run("font.add", json!({"path": path}))
            } else {
                self.open_file(&path, None)
            };
        }
        if self.session.document_id() != before {
            self.autosave_path = None;
            self.dialog = None;
            self.ui.backstage = false;
            self.canvas.want_focus = true;
        }
        if !files.is_empty() {
            self.ensure_caption_fonts();
            self.ensure_comment_fonts();
            self.ensure_recover_fonts();
            self.ensure_recent_fonts();
            self.ensure_find_fonts();
        }
    }

    /// Install the interface fonts when the language, the CJK fallback, or a caption fallback
    /// face has changed since the last install.
    fn install_ui_fonts(&mut self, ctx: &egui::Context, lang: i18n::Lang) {
        let system_cjk = self.fonts_system_cjk || self.want_system_cjk || lang.uses_cjk();
        if !self.styled || lang.prefers_hans() != self.fonts_hans || lang.prefers_korean() != self.fonts_korean || system_cjk != self.fonts_system_cjk
        {
            // Every window's borrowed faces: the fonts are the app's.
            let mut faces = self.canvas.ime_faces.clone();
            for w in self.other_windows() {
                for face in &w.canvas.ime_faces {
                    if !faces.iter().any(|f| f.id() == face.id()) {
                        faces.push(face.clone());
                    }
                }
            }
            theme::install_fonts_language_extra(ctx, lang.prefers_hans(), lang.prefers_korean(), system_cjk, &faces);
            self.fonts_hans = lang.prefers_hans();
            self.fonts_korean = lang.prefers_korean();
            self.fonts_system_cjk = system_cjk;
            // Mod with -, = and 0 are Word shortcuts (optional hyphen, subscript, paragraph spacing);
            // egui's keyboard zoom would also scale the whole window on them. Zoom is View › Zoom.
            // egui quits on Command-Q, which is Ctrl+Q off the Mac: Word's Reset Paragraph
            // Formatting there (#1218). Keep ⌘Q, which only the Mac sends. egui would close only the
            // window it was pressed in, so with native windows the app quits itself
            // ([`WordApp::window_logic_before`]).
            let quit = (!self.windows.native()).then_some(QUIT).into_iter().collect();
            ctx.options_mut(|o| {
                o.zoom_with_keyboard = false;
                o.quit_shortcuts = quit;
            });
            self.styled = true;
        }
    }

    /// Tell the host whether the document has unsaved changes ([`Services::on_dirty`]).
    fn report_dirty(&self) {
        if let Some(f) = &self.services.on_dirty {
            f(self.session.dirty || panes::has_comment_drafts(self));
        }
    }

    /// Whether the document's file is one the user saved to in this session.
    pub fn saved_here(&self) -> bool {
        self.session.path.is_some() && self.session.path == self.autosave_path
    }

    /// Why AutoSave can't cover the document, or `None` when it can: it writes only a file the
    /// user saved to in this session, in a format that keeps everything, and never in the browser
    /// (a page can't write to the user's files; saving there is a download, #176).
    pub fn autosave_block(&self) -> Option<AutoSaveBlock> {
        if self.services.download.is_some() {
            return Some(AutoSaveBlock::Browser);
        }
        let Some(path) = &self.session.path else { return Some(AutoSaveBlock::Unsaved) };
        if !keeps_everything(&path.to_string_lossy()) {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            return Some(AutoSaveBlock::Format { name });
        }
        if !self.saved_here() {
            return Some(AutoSaveBlock::NotSavedHere);
        }
        None
    }

    /// Whether AutoSave covers the document: it is on and nothing blocks it ([`Self::autosave_block`]).
    pub fn autosaves(&self) -> bool {
        self.autosave && self.autosave_block().is_none()
    }

    /// `file.autosave` (the AutoSave switch, and scripts): `value` turns AutoSave on or off;
    /// without it, it toggles what the switch shows. Turning it on for a document AutoSave can't
    /// cover is an error that says why, instead of a switch that is on but saves nothing (#196).
    fn set_autosave(&mut self, params: &Value) -> Result<Value, String> {
        let on = params.get("value").and_then(Value::as_bool).unwrap_or(!self.autosaves());
        if on && let Some(block) = self.autosave_block() {
            return Err(block.reason());
        }
        self.autosave = on;
        self.session.autosave = on;
        Ok(json!({"value": on}))
    }

    /// AutoSave: write a saved document a couple of seconds after the last change. A failure is
    /// shown in the status bar and turns AutoSave off for the file until the user saves it again.
    fn autosave_tick(&mut self, now: f64) {
        if self.autosaves() && self.session.dirty && now - self.last_autosave > 2500.0 && now - self.canvas.caret_visible_since > 1500.0 {
            self.last_autosave = now;
            if let Err(e) = self.session.run("file.save", &json!({})) {
                log::warn!("AutoSave failed: {e}");
                self.autosave_path = None;
                self.status(i18n::fmt(tl!("AutoSave failed: {error}. Save the document to turn AutoSave back on."), &[("error", &e.to_string())]));
            }
        }
    }

    /// Read a Control-click as the right-click on macOS, then offer at most one synthetic gesture
    /// step to the host. eframe carries an unconsumed RawInput into the next pass and into hidden
    /// logic ticks, so offering it again would duplicate input. The step stays queued until a
    /// final (not discarded) UI pass acknowledges it, and a press/release pair stays intact.
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Capture the surface from the raw input itself. `InputState::viewport_rect` and the
        // last painted screen rect are stale in a hidden `run_logic` tick; RawInput is the host's
        // current answer, including minimized/occluded and a Wayland `inner_rect: None`.
        if raw.viewport_id == egui::ViewportId::ROOT
            && let Some(geometry) = control::SurfaceGeometry::from_raw(raw, ctx)
        {
            // A hidden resize and change back still invalidate an older capture, even if
            // neither intermediate geometry reaches a drawing or logic pass.
            if self.surface_geometry != Some(geometry) {
                self.capture_epoch = self.capture_epoch.wrapping_add(1);
            }
            self.surface_geometry = Some(geometry);
        }
        if cfg!(target_os = "macos") {
            control_click_as_secondary(&mut raw.events, &mut self.control_click);
        }
        if self.synthetic_pending || self.synthetic.is_empty() || self.fonts_frames < 2 || !self.control_input_is_roots(ctx) {
            return;
        }
        let Some((events, n)) = self.peek_synthetic() else { return };
        if events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))) {
            // Control input belongs to this window's remembered widget even while another app
            // is active. Give only this input frame egui focus, without requesting OS activation.
            raw.focused = true;
            if let (Some(ctx), Some((id, _))) = (&self.ctx, self.canvas.widget_id) {
                // Canvas keys run before painting updates last frame's native focus flag.
                self.canvas.focused = ctx.memory(|m| m.has_focus(id));
            }
        }
        raw.events.extend(events);
        self.synthetic_pending = true;
        self.synthetic_injected = n;
    }

    /// The control channel's next input frame, taken off the queue.
    fn next_synthetic(&mut self) -> Option<Vec<egui::Event>> {
        let (events, n) = self.peek_synthetic()?;
        self.synthetic.drain(..n);
        Some(events)
    }

    /// The control channel's next input frame: one pointer event (after a move to it), or keys
    /// and text up to the next key release; with how many queued events it takes.
    fn peek_synthetic(&self) -> Option<(Vec<egui::Event>, usize)> {
        if self.synthetic.is_empty() {
            return None;
        }
        let n = match self.synthetic.first() {
            Some(egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }) => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        let mut events = Vec::new();
        if let Some(egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. }) = self.synthetic.first() {
            events.push(egui::Event::PointerMoved(*p));
        }
        let n = n.min(self.synthetic.len());
        events.extend(self.synthetic.iter().take(n).cloned());
        Some((events, n))
    }

    /// Lay out the windows: on the desktop each open document's in its own native window
    /// ([`windows`]), on the web the one window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if self.windows.native() {
            self.windows_ui(ui);
        } else {
            self.window_ui(ui);
        }
    }

    /// Lay out the drawn window. A command that brings another window's state in (New, Open,
    /// Switch Windows) ends this window's drawing for the frame: the rest would be the other
    /// window's.
    fn window_ui(&mut self, ui: &mut egui::Ui) {
        let window = self.windows.shown();
        let moved = |app: &WordApp| app.windows.shown() != window;
        let t0 = now_ms();
        let ctx = ui.ctx().clone();
        i18n::set_current(i18n::Lang::from_pref(&self.ui.language));
        // Animation effects off: egui's scroll areas get a wheel notch whole, not eased in.
        if !self.animations_on() {
            let opts = ctx.options(|o| o.input_options);
            ctx.input_mut(|i| i.smooth_scroll_delta = scroll::unsmoothed_scroll(i, &opts));
        }
        self.handle_ui_input_events(&ctx);
        // Fonts installed by `logic` take effect on the next frame.
        if self.fonts_frames < 2 {
            self.fonts_frames += 1;
            ctx.request_repaint();
            return;
        }
        let t = theme::Tokens::get(&ctx);
        self.escape_free = self.dialog.is_none() && !self.ui.backstage && self.ui.keytips == keytips::Phase::Off && !egui::Popup::is_any_open(&ctx);
        // Keytips are read before layout so the frame paints the new state; badges paint after
        // the ribbon so they sit on top of it.
        crate::keytips::logic(self, &ctx);
        // A failed Recent open, or a failed Save As, is news only until File closes (#2938).
        if !self.ui.backstage {
            backstage::forget_open_error(&ctx);
            backstage::forget_save_as_error(&ctx);
        }
        self.paint_window(ui, &ctx, t);
        if !moved(self) {
            self.show_overlays(ui, &ctx);
        }
        if !moved(self) {
            self.sync_title_and_frame(&ctx, t0);
        }
        // The synthetic step this frame took is done even when drawing ended early.
        self.flush_shot_and_synthetic(&ctx);
    }

    /// Raw input read in the known UI phase, first pass only: dropped files and the SmartArt Text
    /// Pane's Tab/Shift+Tab, before the font warm-up return. `logic` also runs hidden, where its
    /// input events are from the previous shown frame.
    fn handle_ui_input_events(&mut self, ctx: &egui::Context) {
        // eframe does not preserve dropped files for a later frame, so a drop in the first two
        // passes would otherwise disappear.
        #[cfg(not(target_arch = "wasm32"))]
        if ctx.current_pass_index() == 0 {
            let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
            if !dropped.is_empty() {
                self.open_dropped_files(dropped);
                let lang = i18n::Lang::from_pref(&self.ui.language);
                self.install_ui_fonts(ctx, lang);
            }
        }
        // SmartArt Text Pane (#2701): Tab demotes and Shift+Tab promote the focused item, as
        // Word's pane does. This is an input action, so it runs in the first UI pass only; hidden
        // `run_logic` ticks still see the previous shown frame's keys.
        if ctx.current_pass_index() == 0 && self.session.view.smart_art_pane && self.smart_art_row_focused && self.smart_art_item.is_some() {
            let shift_tab = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
            let tab = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
            if shift_tab || tab {
                ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                let _ = self.run(if shift_tab { "smartArt.promote" } else { "smartArt.demote" }, json!({"index": self.smart_art_item.unwrap_or(0)}));
            }
        }
    }

    /// Draw the File page, the empty-window chrome, or the normal document window.
    fn paint_window(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: theme::Tokens) {
        let window = self.windows.shown();
        let moved = |app: &WordApp| app.windows.shown() != window;
        if self.ui.backstage {
            backstage::show(self, ui);
        } else if !self.has_document() {
            // No document open (#1129): the ribbon greyed out but for Help, and no page.
            chrome::title_bar(self, ui);
            ribbon::show(self, ui);
            chrome::status_bar(self, ui);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |_| {});
        } else {
            let screen = chrome::Screen::of(&self.session.view);
            // Focus and Full Screen (#1678) take the whole screen, with only their slim bar.
            self.focus_ribbon &= screen == chrome::Screen::Focus;
            if screen != chrome::Screen::Normal {
                chrome::screen_bar(self, ui, screen);
            } else {
                chrome::title_bar(self, ui);
                if moved(self) {
                    return;
                }
                ribbon::show(self, ui);
                if moved(self) {
                    return;
                }
                chrome::status_bar(self, ui);
                panes::show(self, ui);
            }
            if moved(self) {
                return;
            }
            let fill = if screen == chrome::Screen::Focus { chrome::FOCUS_CANVAS } else { t.canvas };
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(fill)).show(ui, |ui| {
                canvas::show(self, ui);
            });
            if self.focus_ribbon && !moved(self) {
                chrome::focus_ribbon(self, ctx);
            }
        }
    }

    /// Dialogs, panes and other overlays drawn over the window. A command that brings another
    /// window's state in ends the drawing: the rest would be the other window's.
    fn show_overlays(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let window = self.windows.shown();
        let moved = |app: &WordApp| app.windows.shown() != window;
        content_controls::popup(self, ctx);
        dialogs::show(self, ctx);
        if moved(self) {
            return;
        }
        updates_ui::show(self, ctx);
        #[cfg(not(target_arch = "wasm32"))]
        file_browser::show(self, ctx);
        colors::show(self, ctx);
        zotero::show_alert(self, ctx);
        read_aloud::show(self, ctx);
        crate::keytips::show(self, ctx, ui);
        if moved(self) {
            return;
        }
        keys::global_shortcuts(self, ctx);
        if moved(self) {
            return;
        }
        self.refocus_after_chrome_click(ctx);
        if let Some(url) = self.canvas.open_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// Keep the window title and full-screen state in sync, then record this frame's time.
    fn sync_title_and_frame(&mut self, ctx: &egui::Context, t0: f64) {
        // With native windows the viewport driver gives the window its title.
        let title = self.window_title();
        if title != self.sent_title {
            if !self.windows.driven() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            }
            self.sent_title = title;
        }
        self.sync_full_screen(ctx);
        self.frame_ms = now_ms() - t0;
        // Typing and formatting land here, after `logic` has reported.
        self.report_dirty();
    }

    /// One callback command per admitted request, and the synthetic-input queue's final pass.
    fn flush_shot_and_synthetic(&mut self, ctx: &egui::Context) {
        // This is in `ui` so the captured frame is the layout just drawn; a discarded pass is not
        // a layout anyone will see. The shot's `issued` flag makes repeated final passes harmless.
        if !ctx.will_discard() {
            control::issue_screenshots(self, ctx);
        }
        // The raw input was consumed by this host frame's passes; only a final UI pass proves
        // the step is done. A discarded pass keeps the queue for the next UI frame.
        if self.synthetic_pending && !ctx.will_discard() {
            let n = self.synthetic_injected.min(self.synthetic.len());
            self.synthetic.drain(..n);
            self.synthetic_pending = false;
            self.synthetic_injected = 0;
            if !self.synthetic.is_empty() {
                ctx.request_repaint();
            }
        }
    }

    /// Carry View › Full Screen (`view.fullScreen`, F11), Focus and Full Screen View (#1678) to
    /// the window, and follow the window when the system takes it in or out of full screen (the
    /// macOS title bar button, the window manager): in is Full Screen, out ends all three. Leaving
    /// full screen gives the window back its earlier size and place: the system keeps them, and
    /// [`window_geometry`] never saves a full-screen size (#980).
    fn sync_full_screen(&mut self, ctx: &egui::Context) {
        let v = &self.session.view;
        let want = v.full_screen || v.focus_mode || v.full_screen_view;
        let seen = ctx.input(|i| i.viewport().fullscreen);
        if seen != self.seen_full_screen {
            self.seen_full_screen = seen;
            if let Some(on) = seen
                && on != want
            {
                let v = &mut self.session.view;
                v.full_screen = on;
                if !on {
                    v.focus_mode = false;
                    v.full_screen_view = false;
                }
                self.sent_full_screen = on;
            }
        }
        let v = &self.session.view;
        let want = v.full_screen || v.focus_mode || v.full_screen_view;
        if want != self.sent_full_screen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(want));
            self.sent_full_screen = want;
        }
    }

    /// A click on the window's chrome (ribbon tabs and empty ribbon space, galleries, check
    /// boxes, the status bar, panes) hands keyboard focus back to the document, as in Word, so the
    /// caret keeps blinking and typing continues where it was (#1093). egui drops the page's focus
    /// on any click elsewhere; only a click that left nothing focused counts, so a text field
    /// (font size, zoom, command search) keeps the focus it took, and clicks on a dialog, menu or
    /// popup, or while one is open, leave focus alone. Clicks on the page are the canvas's own.
    fn refocus_after_chrome_click(&mut self, ctx: &egui::Context) {
        if self.ui.backstage || self.dialog.is_some() || self.colors.is_some() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.file_browser.open.is_some() {
            return;
        }
        let Some(pos) = ctx.input(|i| if i.pointer.any_click() { i.pointer.interact_pos() } else { None }) else { return };
        // The ribbon dropped over the page in Focus (#1678) is chrome, though it lies over the page.
        let on_focus_ribbon = self.focus_ribbon && ctx.layer_id_at(pos).is_some_and(|l| l.id == egui::Id::unique("focus_ribbon"));
        // The page handles its own clicks (comment balloons there take the keyboard themselves).
        if !on_focus_ribbon && (self.canvas.balloon.is_some() || self.canvas.viewport.is_some_and(|v| v.contains(pos))) {
            return;
        }
        let on_chrome = on_focus_ribbon || ctx.layer_id_at(pos).is_none_or(|l| l.order == egui::Order::Background);
        if on_chrome && ctx.memory(|m| m.focused().is_none()) && !egui::Popup::is_any_open(ctx) {
            self.canvas.want_focus = true;
            // The page has already been drawn, so it only sees `want_focus` next frame, and its own
            // `focused` flag (which gates the keys) isn't recomputed until then either. Claim both
            // now, or the click swallows the first keystroke.
            self.canvas.focused = true;
            // Only a page this pass drew. The File page's back arrow closes the File page in the
            // pass that drew it instead of the page, and focus on a widget missing from the pass
            // names a node AccessKit doesn't have: with accessibility on, AccessKit ends the program
            // over that. `want_focus` hands the keyboard over next frame instead.
            if let Some((id, pass)) = self.canvas.widget_id
                && pass == ctx.cumulative_pass_nr()
            {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            // The canvas reads keys before it takes focus back (#1107), so it counts as focused
            // now: a keystroke in the very next frame reaches the page instead of being dropped.
            self.canvas.focused = true;
            ctx.request_repaint();
        }
    }

    /// Files that arrived asynchronously (web picker, drops): documents open, pictures insert,
    /// recipient lists (CSV/TSV, spreadsheets, or a text file asked for as one) load as recipients.
    fn drain_inbox(&mut self) {
        let Some(inbox) = self.services.inbox.clone() else { return };
        let files = std::mem::take(&mut *inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let mut placed_picture = false;
        for (name, bytes) in files {
            let lower = name.to_ascii_lowercase();
            let img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".wmf", ".emf"].iter().any(|e| lower.ends_with(e));
            // Any recipient file ends the pending picker request, so a later .txt drop opens normally.
            let asked = std::mem::take(&mut self.recipient_list_pending);
            let list = lower.ends_with(".csv") || lower.ends_with(".tsv") || (lower.ends_with(".txt") && asked);
            let sheet = wordcraft_engine::cmd::mailings::is_spreadsheet_name(&lower);
            self.recipient_list_pending = asked && !list && !sheet;
            let r = if sheet {
                self.load_recipients(json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            } else if wordcraft_fonts::is_font_file_name(&lower) {
                self.run("font.add", json!({"name": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            } else if list {
                self.load_recipients(json!({"csv": wordcraft_engine::io::decode_text(&bytes)}))
            } else if img {
                self.caret_after_placed_picture(placed_picture);
                let r = self.insert_or_change_picture(json!({"data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}));
                placed_picture = Self::placed_in_text(&r);
                r
            } else {
                self.run("file.open", json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            };
            if r.is_ok() {
                self.ui.backstage = false;
            }
        }
    }

    /// Pictures pasted in the browser: inserted once when the page is the edit target, dropped
    /// otherwise so the focused field's own paste applies (a text field ignores a picture).
    fn drain_pasted(&mut self) {
        let Some(pasted) = self.services.pasted.clone() else { return };
        let pictures = std::mem::take(&mut *pasted.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let target = paste_target(self.canvas.focused, self.ui.backstage, self.dialog.is_some() || self.file_dialog_open());
        for (_, bytes) in pictures {
            if !target {
                log::debug!("pasted picture ignored: the document is not the edit target");
                continue;
            }
            let _ = self.insert_or_change_picture(json!({"data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}));
        }
    }

    /// Make the capture geometry the active window's surface, viewport and document. The epoch is
    /// monotonic: resizing back to an earlier size is still a new epoch.
    pub(crate) fn refresh_capture_geometry(&mut self, ctx: &egui::Context) {
        let (viewport, surface) = self.active_surface_geometry(ctx);
        let geometry = surface.map(|surface| surface.with_target(viewport, self.session.document_id()));
        if self.capture_geometry != geometry {
            self.capture_geometry = geometry;
            self.capture_epoch = self.capture_epoch.wrapping_add(1);
        }
    }

    /// Drains queued control requests, dispatches each through `control::handle`, sends its reply or
    /// admits a screenshot, then puts the receiver back.
    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path } => control::admit_screenshot(self, ctx, path, reply),
            }
        }
        self.control_rx = Some(rx);
    }
}

/// Why AutoSave can't cover the document ([`WordApp::autosave_block`]); the switch is greyed out
/// and its tooltip gives [`AutoSaveBlock::reason`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutoSaveBlock {
    /// The web app: saving is a download, there is no file to keep writing to.
    Browser,
    /// A new document that has never been saved.
    Unsaved,
    /// A file in a format that can't keep everything (Markdown, plain text, HTML…), by file name.
    Format { name: String },
    /// A file that was opened but not yet saved in this session.
    NotSavedHere,
}

impl AutoSaveBlock {
    /// What the user (or a script) is told, in the interface language.
    pub fn reason(&self) -> String {
        match self {
            AutoSaveBlock::Browser => {
                tl!("AutoSave isn't available in the browser: it can't write to your files. Save downloads a copy.").to_string()
            }
            AutoSaveBlock::Unsaved => tl!("Save the document to turn on AutoSave").to_string(),
            AutoSaveBlock::Format { name } => i18n::fmt(
                tl!("AutoSave only keeps .docx, .odt and .rtf files; {name} can't keep all formatting. Save as .docx to turn on AutoSave."),
                &[("name", name)],
            ),
            AutoSaveBlock::NotSavedHere => {
                tl!("Save the document to turn on AutoSave (a file you have only opened isn't rewritten until you save it)").to_string()
            }
        }
    }

    /// Whether Save As can lift the block (everywhere but the browser).
    pub fn save_as_helps(&self) -> bool {
        !matches!(self, AutoSaveBlock::Browser)
    }
}

/// Whether saving to `name` keeps the whole document (the formats `file.save` treats as the
/// document's own file, including macro-enabled documents and templates); anything else is a
/// copy that leaves it unsaved, unless it is the file the document was opened from (#2376).
/// AutoSave writes only these formats.
pub fn keeps_everything(name: &str) -> bool {
    let ext = std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "docx" | "docm" | "dotx" | "dotm" | "odt" | "rtf" | "json" | "xml")
}

/// The dialog a user-run command opens when it lacks the input it needs (scripts and agents get
/// the command's own error or default instead): Select Recipients and Edit Recipient List without
/// data, Insert Merge Field without a field, Find Recipient without text, the If and Skip
/// Record If rules (or Rules with no rule at all) without a field, Finish & Merge and Print
/// Documents without records, Table Properties without settings, Symbol and Field without a
/// character or code, and Enclose Characters without a shape.
fn input_dialog(id: &str, params: &Value) -> Option<&'static str> {
    let has = |k: &str| params.get(k).is_some_and(|v| !v.is_null());
    let rule = params.get("rule").and_then(Value::as_str).map(str::to_ascii_uppercase);
    match id {
        "file.rename" if !has("name") => Some("renameDocument"),
        "mailings.rules" if !has("field") => match rule.as_deref() {
            None | Some("IF") => Some("ruleIf"),
            Some("SKIPIF") => Some("ruleSkipIf"),
            _ => None,
        },
        "mailings.recipients" if !["csv", "path", "data", "rows"].into_iter().any(has) => Some("recipientList"),
        "mailings.editRecipients" if !has("rows") => Some("recipientList"),
        "mailings.insertField" if !has("field") => Some("insertMergeField"),
        "mailings.findRecipient" if !has("text") => Some("findRecipient"),
        // Finish & Merge and Print Documents without records ask which (#3028).
        "mailings.finish" if !["path", "records", "from", "to"].into_iter().any(has) => Some("mergeRecords"),
        "mailings.printDocuments" if !["records", "from", "to"].into_iter().any(has) => Some("printRecords"),
        // Symbol without a character and Field without a code show their dialogs (#321).
        "insert.symbol" if !has("char") => Some("symbol"),
        "insert.field" if !has("instr") => Some("field"),
        // Edit Field Code shows the selected field's code (#339).
        "fields.setCode" if !has("code") => Some("editFieldCode"),
        // SmartArt without a layout or items shows the layout picker.
        "insert.smartArt" if !has("layout") && !has("items") => Some("insertSmartArt"),
        // Chart without a type shows the gallery; Change Chart Type and Edit Data their dialogs.
        "insert.chart" if !has("type") && !has("data") => Some("insertChart"),
        "chart.type" if !has("type") => Some("changeChartType"),
        "chart.editData" if !["data", "categories", "series"].into_iter().any(has) => Some("chartData"),
        // Table Properties without settings shows the dialog (with settings it applies them).
        "table.properties" if params.as_object().is_none_or(|m| m.is_empty()) => Some("tableProperties"),
        // Cell Margins without settings shows the dialog; values continue through the same command.
        "table.cellMargins" if params.as_object().is_none_or(|m| m.is_empty()) => Some("tableCellMargins"),
        // Size without a size (Picture Format › Size's launcher) shows the Size dialog (#2573).
        "picture.size" if params.as_object().is_none_or(|m| m.is_empty()) => Some("pictureSize"),
        // Formula without one asks for it, suggested from the numbers next to the cell (#1201).
        "table.formula" if !has("formula") => Some("tableFormula"),
        // Sort without settings asks for the column, order and header row (#1200).
        "table.sort" if params.as_object().is_none_or(|m| m.is_empty()) => Some("tableSort"),
        // Split Cells and Convert Text to Table without settings ask for them (#2592).
        "table.split" if params.as_object().is_none_or(|m| m.is_empty()) => Some("splitCells"),
        "table.fromText" if params.as_object().is_none_or(|m| m.is_empty()) => Some("textToTable"),
        // Convert to Text without a separator asks for one.
        "table.toText" if params.as_object().is_none_or(|m| m.is_empty()) => Some("tableToText"),
        // Tabs, Borders and Shading, and Page Borders without settings show their dialogs (#320).
        "para.tabs" if params.as_object().is_none_or(|m| m.is_empty()) => Some("tabs"),
        "para.borders" if params.as_object().is_none_or(|m| m.is_empty()) => Some("borders"),
        "design.pageBorders" if params.as_object().is_none_or(|m| m.is_empty()) => Some("pageBorders"),
        // `null` is an answer here: it removes the password.
        "file.encrypt" if params.get("password").is_none() => Some("encryptPassword"),
        // Define New Multilevel List and Track Changes Options without settings show their dialogs.
        "list.define" if params.get("levels").is_none() => Some("defineList"),
        "review.trackingOptions" if params.as_object().is_none_or(|m| m.is_empty()) => Some("trackChangesOptions"),
        // Compare / Combine without a revised document, AutoCorrect Options without settings and
        // Restrict Editing without settings show their dialogs or pane (#412).
        "review.compare" if !has("path") && !has("text") => Some("compare"),
        "review.combine" if !has("path") && !has("text") => Some("combine"),
        "tools.autocorrect" if params.as_object().is_none_or(|m| m.is_empty()) => Some("autoCorrect"),
        "review.restrict" | "file.protect" if params.as_object().is_none_or(|m| m.is_empty()) => Some("restrictEditing"),
        "insert.bookmark" | "insert.deleteBookmark" if !has("name") => Some("bookmark"),
        // Word Count from the keyboard or command search shows its dialog, as the ribbon does.
        "review.wordCount" if params.as_object().is_none_or(|m| m.is_empty()) => Some("wordCount"),
        // Content control Properties without settings shows the dialog.
        "developer.properties" if params.as_object().is_none_or(|m| m.is_empty()) => Some("controlProperties"),
        // Enclose Characters without a shape (null is one: it takes the enclosure away).
        "format.enclose" if params.get("shape").is_none() => Some("enclose"),
        // Footnote and Endnote without settings shows its dialog (#385).
        "references.noteOptions" if params.as_object().is_none_or(|m| m.is_empty()) => Some("noteOptions"),
        // References dialogs (#402): Insert Caption, Insert Index, Insert Table of Figures (#3013),
        // Mark Entry, Manage Sources and Insert Citation without a source.
        "references.caption" | "references.index" | "references.tableOfFigures" if params.as_object().is_none_or(|m| m.is_empty()) => {
            Some(match id {
                "references.caption" => "caption",
                "references.index" => "index",
                _ => "tableOfFigures",
            })
        }
        "references.markEntry" if !has("entry") => Some("markEntry"),
        // Cross-reference without a target shows its dialog (#916); scripts get the target list.
        "insert.crossReference" if !has("target") => Some("crossReference"),
        "references.sources" if params.as_object().is_none_or(|m| m.is_empty()) => Some("sourceManager"),
        "references.citation" if !has("tag") && !has("source") => Some("createSource"),
        // Templates and Add-ins, and the Organizer without a file to list (#377).
        "tools.templates" => Some("templates"),
        "styles.organizer" if !has("path") => Some("organizer"),
        // Language, Envelopes and Labels without settings show their dialogs (#407).
        "review.language" if params.as_object().is_none_or(|m| m.is_empty()) => Some("language"),
        "mailings.envelopes" if params.as_object().is_none_or(|m| m.is_empty()) => Some("envelopes"),
        "mailings.labels" if params.as_object().is_none_or(|m| m.is_empty()) => Some("labels"),
        _ => None,
    }
}

/// Commands that work while no document is open (#1129): opening or creating one, closing the
/// window, Help, adding fonts (a font file dropped on the empty window), and the settings (File ›
/// Options, the view and interface preferences it sets, Customize Ribbon and Keyboard).
/// Everything else needs a document, `ui.*` included: only the interface commands listed here
/// (tabs, Backstage, dialogs, appearance, opening files) work without one.
pub(crate) fn works_without_document(id: &str) -> bool {
    matches!(
        id,
        "file.new"
            | "file.newFromTemplate"
            | "file.listTemplates"
            | "file.open"
            | "file.close"
            | "file.options"
            | "file.setAuthor"
            | "font.add"
            | "font.list"
            | "font.remove"
            | "readAloud.speed"
            | "readAloud.skipCitations"
            | "tools.customizeKeyboard"
            | "tools.customizeRibbon"
            | "view.darkMode"
            | "view.marks"
            | "view.ruler"
            | "view.numeral"
            | "ui.tab"
            | "ui.backstage"
            | "ui.dialog"
            | "ui.closeDialog"
            | "ui.collapseRibbon"
            | "ui.quickAccessToolbar"
            | "ui.keytips"
            | "ui.dark"
            | "ui.theme"
            | "ui.interfaceSize"
            | "ui.animations"
            | "ui.language"
            | "ui.saveChanges"
            | "ui.openFileDialog"
            | "ui.fileBrowser"
            | "ui.makeDefaultApp"
            | "ui.discord"
            | "ui.caretAnimation"
            | "ui.startScreen"
            | "ui.checkForUpdates"
    ) || id.starts_with("help.")
}

/// Commands that save, export, print, send or replace the document: comment text still being
/// typed is written before them (#1348).
fn flushes_comment_drafts(id: &str, params: &Value) -> bool {
    matches!(
        id,
        "file.save"
            | "file.saveAs"
            | "file.saveTemplate"
            | "file.open"
            | "file.exportPdf"
            | "file.exportPng"
            | "file.print"
            | "file.printPdf"
            | "file.share"
            | "ui.print"
            | "ui.printSubmit"
    ) || discards_document(id, params)
}

/// User commands that close the shown document: Close. New, Open, restoring a recovered copy,
/// Envelopes, Labels, Finish & Merge and Print Documents open their document beside it (Open and restoring
/// replace only an untouched blank document, which has nothing to lose).
pub(crate) fn discards_document(id: &str, _params: &Value) -> bool {
    id == "file.close"
}

/// Monotonic milliseconds for deadlines, settling and animation. `web_time::Instant` is a real
/// monotonic clock on native (`Instant::now`) and in the browser (`performance.now()`), so a wall
/// clock or NTP adjustment cannot make a screenshot deadline or autosave fire early or late.
pub fn now_ms() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<web_time::Instant> = OnceLock::new();
    START.get_or_init(web_time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// macOS reads a Control-click as the right-click (winit reports the primary button with Control
/// held), so menus open on it as on a two-finger click. The release follows its press even if
/// Control was let go in between.
fn control_click_as_secondary(events: &mut [egui::Event], held: &mut bool) {
    for e in events {
        if let egui::Event::PointerButton { button: button @ egui::PointerButton::Primary, pressed, modifiers, .. } = e {
            if *pressed {
                *held = modifiers.ctrl && !modifiers.mac_cmd;
            }
            if *held {
                *button = egui::PointerButton::Secondary;
            }
            if !*pressed {
                *held = false;
            }
        }
    }
}

#[cfg(test)]
mod tests_perf;

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn mod_key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }
    }

    /// Issue #14: Mod+- (optional hyphen) also zoomed the whole window out, so it read as "zoom out".
    #[test]
    fn word_shortcuts_do_not_zoom_the_window() {
        let ctx = egui::Context::default();
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput { events, ..Default::default() };
            // No painter here: dropping the font atlas delta unapplied trips an epaint debug assertion.
            ctx.run_ui(input, |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
        };
        frame(Vec::new());
        frame(vec![mod_key(egui::Key::Minus)]);
        frame(vec![mod_key(egui::Key::Minus)]);
        frame(Vec::new());
        assert_eq!(ctx.zoom_factor(), 1.0);
    }

    /// Issue #117: a viewport command schedules a repaint, so resending the title every frame kept
    /// the app redrawing at the monitor's refresh rate while idle.
    #[test]
    fn window_title_is_sent_only_when_it_changes() {
        let ctx = egui::Context::default();
        let mut app = app();
        let titles = |app: &mut WordApp| {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            let n = out.viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, egui::ViewportCommand::Title(_))).count();
            out.drop_without_applying_deltas();
            n
        };
        // The first frames only install fonts; the title goes out once, on the first full frame.
        let first: usize = (0..4).map(|_| titles(&mut app)).sum();
        assert_eq!(first, 1);
        assert_eq!(titles(&mut app), 0);
        assert_eq!(titles(&mut app), 0);
        app.session.dirty = true;
        assert_eq!(titles(&mut app), 1);
    }

    /// #1678: Focus and Full Screen View put the window in full screen, asked of the system once
    /// per change (switching between the two keeps it), and the usual window comes back after.
    #[test]
    fn focus_and_full_screen_view_ask_for_full_screen_once_per_change() {
        let ctx = egui::Context::default();
        let mut app = app();
        let sent = |app: &mut WordApp| {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            let v: Vec<bool> = out
                .viewport_output
                .values()
                .flat_map(|v| v.commands.iter())
                .filter_map(|c| if let egui::ViewportCommand::Fullscreen(b) = c { Some(*b) } else { None })
                .collect();
            out.drop_without_applying_deltas();
            v
        };
        let start: Vec<bool> = (0..4).flat_map(|_| sent(&mut app)).collect();
        assert!(start.is_empty(), "the window starts as it was: {start:?}");
        app.run("view.focus", json!({})).unwrap();
        assert_eq!(sent(&mut app), [true]);
        assert!(sent(&mut app).is_empty());
        app.run("view.fullScreenView", json!({})).unwrap();
        assert!(sent(&mut app).is_empty(), "Focus to Full Screen View stays full screen");
        app.run("view.fullScreenView", json!({})).unwrap();
        assert_eq!(sent(&mut app), [false]);
    }

    /// #1678: Focus hides the title bar, ribbon and status bar; "⋯" drops the ribbon over the
    /// page without leaving Focus, and Escape leaves Focus (and Full Screen View).
    #[test]
    fn focus_hides_the_chrome_drops_the_ribbon_and_esc_leaves() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);
        assert!(h.query_by_label("Insert").is_some(), "the ribbon's tabs show in the usual window");
        // Focus on the status bar, as a user turns it on.
        h.get_by_label("Focus").click();
        h.run_steps(3);
        assert!(h.state().session.view.focus_mode, "the status bar's Focus turns Focus on");
        assert!(h.query_by_label("Insert").is_none(), "Focus hides the ribbon");
        assert!(h.query_by_label("Search commands and help").is_none(), "and the title bar");
        // "⋯" in the Focus bar, at the top centre.
        h.get_by_label("Show the ribbon").click();
        h.run_steps(3);
        assert!(h.query_by_label("Insert").is_some(), "the ribbon drops down");
        assert!(h.state().session.view.focus_mode, "still in Focus");
        // A click on the dropped ribbon works it and leaves it down; one on the page puts it away.
        h.get_by_label("Insert").click();
        h.run_steps(3);
        assert!(h.state().focus_ribbon && h.state().ui.tab == "Insert", "a click on the ribbon keeps it down");
        // The dropped ribbon lies over the page but is chrome: its click hands the keyboard back
        // to the page, as a ribbon click does in the usual window (#1093).
        assert!(h.state().canvas.focused, "a click on the dropped ribbon gives the page the keyboard");
        h.event(egui::Event::Text("x".into()));
        h.run_steps(2);
        assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Hellox"), "typing after it reaches the page");
        assert!(h.state().focus_ribbon, "typing leaves the dropped ribbon down");
        let page = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 600.0).unwrap();
        h.event(egui::Event::PointerMoved(page));
        h.event(egui::Event::PointerButton { pos: page, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: page, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        assert!(!h.state().focus_ribbon && h.state().session.view.focus_mode, "a click on the page puts it away, still in Focus");
        h.get_by_label("Show the ribbon").click();
        h.run_steps(3);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        let a = h.state_mut();
        assert!(!a.session.view.focus_mode && !a.focus_ribbon, "Escape leaves Focus");
        a.run("view.fullScreenView", json!({})).unwrap();
        h.run_steps(3);
        assert!(h.query_by_label("Insert").is_none(), "Full Screen View shows the pages alone");
        assert!(h.query_by_label("Search commands and help").is_some(), "with Search at the top");
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(!h.state().session.view.full_screen_view, "Escape leaves Full Screen View");
        // Full Screen (#980, F11) is the whole window, ribbon and all.
        h.state_mut().run("view.fullScreen", json!({})).unwrap();
        h.run_steps(3);
        assert!(h.query_by_label("Insert").is_some(), "Full Screen keeps the ribbon");
    }

    /// #1678: Focus's dropped ribbon is a command, `ui.focusRibbon`, which "⋯" and a click on the
    /// page run; outside Focus it does nothing and never fails.
    #[test]
    fn focus_ribbon_is_a_command() {
        let mut a = app();
        let off = json!({"focusRibbon": false});
        assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), off, "outside Focus there is no ribbon to drop");
        assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap(), off);
        assert!(!a.focus_ribbon);
        a.run("view.focus", json!({})).unwrap();
        assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), json!({"focusRibbon": true}), "no value toggles it");
        assert!(a.focus_ribbon && a.session.view.focus_mode);
        assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), off);
        assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap()["focusRibbon"], true);
        assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap()["focusRibbon"], true, "true keeps it down");
        for junk in [json!({"value": "yes"}), json!({"value": 1}), json!({"value": null}), json!({"value": [true]})] {
            assert!(a.run("ui.focusRibbon", junk).is_err());
        }
        assert!(a.focus_ribbon, "a refused value leaves it as it was");
        assert_eq!(a.run("ui.focusRibbon", json!({"value": false})).unwrap(), off);
        assert!(a.session.view.focus_mode, "putting the ribbon away stays in Focus");
    }

    /// A harness on a document with text whose page has the keyboard.
    fn canvas_harness() -> egui_kittest::Harness<'static, WordApp> {
        let mut a = app();
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);
        h.state_mut().canvas.want_focus = true;
        h.run_steps(3);
        assert!(h.state().canvas.focused, "the page has the keyboard");
        h
    }

    /// #1678: one Escape path. In Focus with text selected on the page, the first Escape only
    /// collapses the selection, as in a normal window, and the second leaves Focus. A handler that
    /// takes Escape for Focus before the page sees it would leave Focus with the selection kept.
    #[test]
    fn focus_esc_collapses_the_selection_first_then_leaves() {
        let mut h = canvas_harness();
        h.state_mut().run("view.focus", json!({})).unwrap();
        h.run_steps(3);
        h.state_mut().run("select.all", json!({})).unwrap();
        h.run_steps(2);
        assert!(h.state().canvas.focused && h.state().session.view.focus_mode);
        assert!(!h.state().session.sel.is_collapsed(), "text is selected");
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().session.sel.is_collapsed(), "the first Escape collapses the selection");
        assert!(h.state().session.view.focus_mode, "and keeps Focus");
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(!h.state().session.view.focus_mode, "the second Escape leaves Focus");
    }

    /// Ctrl+F1 pressed on the page, through the app's key handling.
    #[cfg(not(target_os = "macos"))]
    fn press_ctrl_f1(h: &mut egui_kittest::Harness<'static, WordApp>) {
        let modifiers = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        h.event(egui::Event::Key { key: egui::Key::F1, physical_key: None, pressed: true, repeat: false, modifiers });
        h.event(egui::Event::Key { key: egui::Key::F1, physical_key: None, pressed: false, repeat: false, modifiers });
        h.run_steps(3);
    }

    /// #1678: unbound, Ctrl+F1 collapses the ribbon and a second one brings it back (not on
    /// macOS, where Ctrl+F1 is the system's).
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ctrl_f1_through_the_app_collapses_the_ribbon() {
        let mut h = canvas_harness();
        assert!(!h.state().ui.ribbon_collapsed);
        press_ctrl_f1(&mut h);
        assert!(h.state().ui.ribbon_collapsed, "Ctrl+F1 collapses the ribbon");
        assert!(!h.state().session.view.focus_mode);
        press_ctrl_f1(&mut h);
        assert!(!h.state().ui.ribbon_collapsed, "and brings it back");
    }

    /// #1678: a Ctrl+F1 the user binds in Customize Keyboard runs their command through the app,
    /// and the ribbon stays as it was.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_user_bound_ctrl_f1_through_the_app_runs_the_command() {
        let mut h = canvas_harness();
        h.state_mut().session.keymap = wordcraft_engine::keymap::KeyMap::from_value(&json!({"assigned": {"Mod+F1": "view.focus"}}));
        press_ctrl_f1(&mut h);
        assert!(h.state().session.view.focus_mode, "the bound command ran");
        assert!(!h.state().ui.ribbon_collapsed, "the ribbon is unchanged");
    }

    /// The caption is Inter semibold, which has no Hebrew (or Arabic, Thai, …), so egui would
    /// paint each of those letters as `?`. Opening such a file borrows one document fallback
    /// face. A CJK name uses the CJK UI font instead of that list.
    #[test]
    fn non_latin_file_name_is_drawn_in_the_caption() {
        let ctx = egui::Context::default();
        let frame = |app: &mut WordApp| {
            ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
        };
        let mut latin = app();
        latin.session.path = Some("report.docx".into());
        frame(&mut latin);
        assert!(!latin.want_system_cjk);
        assert!(latin.canvas.ime_faces.is_empty(), "a Latin name needs no extra face");

        let mut hebrew = app();
        hebrew.session.path = Some("מסמך.docx".into());
        frame(&mut hebrew);
        let letter = 'מ';
        if wordcraft_fonts::FontDb::global().fallback_for(letter, u32::MAX).is_some_and(|f| f.data().len() <= 16 << 20) {
            assert!(hebrew.canvas.ime_faces.iter().any(|f| f.covers(letter)), "the caption borrows a face that covers the file name");
        }
        let n = hebrew.canvas.ime_faces.len();
        frame(&mut hebrew);
        assert_eq!(hebrew.canvas.ime_faces.len(), n, "the face is added once");

        let mut cjk = app();
        cjk.session.path = Some("文档.docx".into());
        frame(&mut cjk);
        assert!(cjk.want_system_cjk, "a CJK file name loads the CJK UI font");
        assert!(cjk.canvas.ime_faces.is_empty(), "CJK collections are not copied into the fallback list");
    }

    /// File › Recover draws the recovered documents' names with the interface faces: a Chinese
    /// name there was a row of boxes because nothing asked for the CJK UI font. (The Recent list
    /// has its own, `ensure_recent_fonts`.)
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn non_latin_names_in_the_recover_list_get_a_face() {
        let with_copy = |name: &str| {
            let root = std::env::temp_dir().join(format!(
                "wordcraft-recover-fonts-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            let id = "0123456789abcdef0123456789abcdef";
            std::fs::create_dir_all(root.join(id)).unwrap();
            let snapshot = json!({
                "version": 1, "generation": 1, "info": {"id": id, "name": name, "savedAt": "2026-10-11T10:42:00Z"},
                "document": wordcraft_doc::Document::from_text("Recovered work")
            });
            std::fs::write(root.join(id).join("snapshot-00000000000000000001.json"), serde_json::to_vec(&snapshot).unwrap()).unwrap();
            let mut a = app();
            a.session.enable_recovery(root.clone(), std::sync::Arc::new(|| {})).unwrap();
            let ctx = egui::Context::default();
            let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                ctx.run_ui(egui::RawInput::default(), |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
                if a.session.recovery_ready() && !a.session.recovery_documents().is_empty() {
                    // One more frame looks at the names it found.
                    ctx.run_ui(egui::RawInput::default(), |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
                    break;
                }
                assert!(std::time::Instant::now() < end, "the recovery copy was never found");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _ = std::fs::remove_dir_all(&root);
            a
        };
        assert!(!with_copy("Synthetic draft").want_system_cjk, "Latin names need no CJK font");
        assert!(with_copy("会议记录-草稿").want_system_cjk, "a CJK name in Recover loads the CJK UI font");
    }

    /// Comment balloons draw comment text and author names with the interface faces, which have
    /// no Arabic, so an Arabic comment was all boxes. A document with one borrows a document
    /// fallback face that covers it, once per document revision.
    #[test]
    fn arabic_comment_text_is_drawn_in_the_balloons() {
        let ctx = egui::Context::default();
        let frame = |app: &mut WordApp| {
            ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
        };
        let mut latin = app();
        latin.run("text.insert", json!({"text": "Scope of works"})).unwrap();
        latin.run("select.text", json!({"text": "Scope"})).unwrap();
        latin.run("review.newComment", json!({"text": "Please check"})).unwrap();
        frame(&mut latin);
        assert!(latin.canvas.ime_faces.is_empty(), "a Latin comment needs no extra face");

        let mut arabic = app();
        arabic.run("text.insert", json!({"text": "Scope of works"})).unwrap();
        arabic.run("select.text", json!({"text": "Scope"})).unwrap();
        arabic.run("review.newComment", json!({"text": "لتحديد نطاق الأعمال"})).unwrap();
        frame(&mut arabic);
        let letter = 'ن';
        if wordcraft_fonts::FontDb::global().fallback_for(letter, u32::MAX).is_some_and(|f| f.data().len() <= 16 << 20) {
            assert!(arabic.canvas.ime_faces.iter().any(|f| f.covers(letter)), "the balloons borrow a face that covers the comment");
        }
        let n = arabic.canvas.ime_faces.len();
        frame(&mut arabic);
        assert_eq!(arabic.canvas.ime_faces.len(), n, "the face is added once");
    }

    /// Recent documents in Backstage and the Start screen draw file names and paths with the
    /// interface faces, which have no Arabic, Hebrew, Thai …: a recent document borrows a
    /// document fallback face that covers it (#2067).
    #[test]
    fn recent_documents_borrow_fallback_faces() {
        let ctx = egui::Context::default();
        let frame = |app: &mut WordApp| {
            ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
        };
        let mut latin = app();
        latin.ui.recent.push("/home/ice/report.docx".into());
        frame(&mut latin);
        assert!(latin.canvas.ime_faces.is_empty(), "a Latin file name needs no extra face");

        let mut arabic = app();
        arabic.ui.recent.push("/home/ice/نواقص 2026.docx".into());
        frame(&mut arabic);
        let letter = 'ن';
        if wordcraft_fonts::FontDb::global().fallback_for(letter, u32::MAX).is_some_and(|f| f.data().len() <= 16 << 20) {
            assert!(arabic.canvas.ime_faces.iter().any(|f| f.covers(letter)), "the recent list borrows a face that covers the file name");
        }
        let n = arabic.canvas.ime_faces.len();
        frame(&mut arabic);
        assert_eq!(arabic.canvas.ime_faces.len(), n, "the face is added once");
    }

    /// #3248: the Find box and the Navigation pane's Results list draw the typed query and
    /// document excerpts with the interface faces, which have no Georgian (or Armenian, Thai,
    /// …), so a Georgian search was all boxes. Running a search that finds Georgian text borrows
    /// a document fallback face that covers it, as the caption and comments already do.
    #[test]
    fn georgian_find_text_is_drawn_in_the_navigation_pane() {
        let ctx = egui::Context::default();
        let frame = |app: &mut WordApp| {
            ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
        };
        let mut latin = app();
        latin.run("text.insert", json!({"text": "This is some text"})).unwrap();
        latin.run("edit.find", json!({"text": "some text"})).unwrap();
        frame(&mut latin);
        assert!(latin.canvas.ime_faces.is_empty(), "a Latin query and result need no extra face");

        let mut georgian = app();
        georgian.run("text.insert", json!({"text": "ეს არის ქართული ტექსტი"})).unwrap();
        georgian.run("edit.find", json!({"text": "ტექსტი"})).unwrap();
        assert!(!georgian.session.find.results.is_empty(), "the search actually found the Georgian word");
        frame(&mut georgian);
        let letter = 'ტ';
        if wordcraft_fonts::FontDb::global().fallback_for(letter, u32::MAX).is_some_and(|f| f.data().len() <= 16 << 20) {
            assert!(georgian.canvas.ime_faces.iter().any(|f| f.covers(letter)), "the Results list borrows a face that covers the match");
        }
        let n = georgian.canvas.ime_faces.len();
        frame(&mut georgian);
        assert_eq!(georgian.canvas.ime_faces.len(), n, "the face is added once, cached by query and result count");

        // The query box needs a fallback face even before any match is found (#3248's screenshot
        // shows the typed query itself as boxes, not just the results).
        let mut typing = app();
        typing.run("text.insert", json!({"text": "unrelated"})).unwrap();
        typing.session.find.query = "ტექსტი".into();
        frame(&mut typing);
        if wordcraft_fonts::FontDb::global().fallback_for(letter, u32::MAX).is_some_and(|f| f.data().len() <= 16 << 20) {
            assert!(typing.canvas.ime_faces.iter().any(|f| f.covers(letter)), "the typed query alone (no match yet) still gets a fallback face");
        }
    }

    /// #3257 review: the cache key didn't include the document revision, so editing text right
    /// beside an existing match — adding Georgian next to a Latin match already found — left the
    /// newly-visible Georgian in the Results list's context window without a fallback face, since
    /// neither the query nor the (stale, not recomputed by typing alone) result count changed.
    #[test]
    fn editing_beside_an_existing_match_still_gets_a_fallback_face() {
        let ctx = egui::Context::default();
        let frame = |app: &mut WordApp| {
            ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
        };
        let mut a = app();
        a.run("text.insert", json!({"text": "Before some text After"})).unwrap();
        a.run("edit.find", json!({"text": "some text"})).unwrap();
        frame(&mut a);
        assert!(a.canvas.ime_faces.is_empty(), "a Latin query and result need no extra face yet");

        let before = a.session.find.results.clone();
        a.run("caret.docEnd", json!({})).unwrap();
        a.run("text.insert", json!({"text": " ტექსტი"})).unwrap();
        assert_eq!(a.session.find.results, before, "typing alone doesn't recompute the stale results");
        frame(&mut a);
        let letter = 'ტ';
        if wordcraft_fonts::FontDb::global().fallback_for(letter, u32::MAX).is_some_and(|f| f.data().len() <= 16 << 20) {
            assert!(
                a.canvas.ime_faces.iter().any(|f| f.covers(letter)),
                "Georgian typed beside the match, inside its context window, gets a fallback face too"
            );
        }
    }

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    /// All title-bar inputs have accessible names. Clicking the title opens Rename Document, the
    /// Rename button saves a new name, Escape cancels, a long name stays clickable, and rename
    /// leaves the document clean.
    #[test]
    fn title_and_account_buttons_work() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        h.run_steps(6);
        assert!(h.unnamed_widgets().is_empty(), "chrome inputs have accessible names: {:?}", h.unnamed_widgets());
        h.check_accessibility();
        h.get_by_label("Document1 — Not saved").click();
        h.run_steps(2);
        assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("renameDocument"));
        assert!(h.unnamed_widgets().is_empty(), "the document-name field is labelled: {:?}", h.unnamed_widgets());
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename").click();
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
        assert!(h.state().session.file_name.is_none());
        assert!(!h.state().session.dirty);
        h.get_by_label("Document1 — Not saved").click();
        h.run_steps(2);
        h.event(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { command: true, ..Default::default() },
        });
        h.event(egui::Event::Text("Project notes".into()));
        h.run_steps(2);
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename").click();
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
        assert_eq!(h.state().title_stem(), "Project notes");
        assert_eq!(h.state().default_save_name(), "Project notes.docx");
        assert!(h.state().session.doc.core.title.is_empty());
        assert!(!h.state().session.dirty);
        // A long name remains clickable, and Escape leaves it unchanged.
        let long_name = "Notes ".repeat(25);
        h.state_mut().run("file.rename", json!({"name": long_name})).unwrap();
        h.run_steps(2);
        h.get_by_label(&format!("{} — Not saved", long_name.trim())).click();
        h.run_steps(2);
        assert!(h.state().dialog.is_some());
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
        assert_eq!(h.state().title_stem(), long_name.trim());
        h.get_by_label("Account").click();
        h.run_steps(2);
        assert!(h.state().ui.backstage);
        assert_eq!(h.state().ui.backstage_page, "options");
        assert_eq!(h.state().ui.options_pane, "general");
        assert!(h.query_by_label("User name:").is_some());
    }

    /// File › Options › Advanced › Editing: alignment guides are a UI command, on by default and
    /// kept between runs.
    #[test]
    fn alignment_guides_are_a_ui_command_and_survive_a_restart() {
        let mut a = app();
        assert_eq!(a.run("ui.alignmentGuides", json!({})).unwrap(), json!({"alignmentGuides": true}), "on by default");
        assert_eq!(a.run("ui.alignmentGuides", json!({"value": false})).unwrap(), json!({"alignmentGuides": false}));
        for junk in [json!({"value": "no"}), json!({"value": 0}), json!({"value": null})] {
            assert!(a.run("ui.alignmentGuides", junk).is_err());
        }
        assert!(!a.ui.alignment_guides, "a rejected value keeps the setting");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert!(!b.ui.alignment_guides);
        let old: UiState = serde_json::from_str(r#"{"tab":"Insert"}"#).unwrap();
        assert!(old.alignment_guides, "older settings leave it on");
    }

    /// File › Options › Advanced › Animation: the caret's animation is a UI command, kept between
    /// runs.
    #[test]
    fn caret_animation_is_a_ui_command_and_survives_a_restart() {
        let mut a = app();
        assert_eq!(a.run("ui.caretAnimation", json!({})).unwrap(), json!({"caretAnimation": true}), "on by default; no value reads it");
        assert_eq!(a.run("ui.caretAnimation", json!({"value": false})).unwrap(), json!({"caretAnimation": false}));
        for junk in [json!({"value": "yes"}), json!({"value": 1}), json!({"value": null}), json!({"value": [true]})] {
            assert!(a.run("ui.caretAnimation", junk).is_err());
        }
        assert!(!a.ui.caret_animation, "a rejected value keeps the setting");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""caretAnimation":false"#), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert!(!b.ui.caret_animation);
        // Settings saved before the option existed leave it on.
        let old: UiState = serde_json::from_str(r#"{"tab":"Insert"}"#).unwrap();
        assert!(old.caret_animation && old.tab == "Insert");
        assert_eq!(a.run("ui.caretAnimation", json!({"value": true})).unwrap()["caretAnimation"], true);
    }

    /// #1945: showing or hiding the Quick Access Toolbar is a UI command (no value toggles it),
    /// kept between runs; settings saved before it existed show the toolbar; it works with no
    /// document open, like the other interface settings.
    #[test]
    fn quick_access_toolbar_is_a_ui_command_and_survives_a_restart() {
        let mut a = app();
        assert!(a.ui.quick_access_toolbar, "shown by default");
        assert_eq!(a.run("ui.quickAccessToolbar", json!({})).unwrap(), json!({"quickAccessToolbar": false}), "no value toggles it");
        assert_eq!(a.run("ui.quickAccessToolbar", json!({})).unwrap(), json!({"quickAccessToolbar": true}));
        assert_eq!(a.run("ui.quickAccessToolbar", json!({"value": false})).unwrap(), json!({"quickAccessToolbar": false}));
        for junk in [json!({"value": "no"}), json!({"value": 0}), json!({"value": null}), json!({"value": [true]})] {
            assert!(a.run("ui.quickAccessToolbar", junk).is_err());
        }
        assert!(!a.ui.quick_access_toolbar, "a rejected value keeps the setting");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""quickAccessToolbar":false"#), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert!(!b.ui.quick_access_toolbar, "hidden after a restart");
        let old: UiState = serde_json::from_str(r#"{"tab":"Insert"}"#).unwrap();
        assert!(old.quick_access_toolbar && old.tab == "Insert", "older settings show the toolbar");
        assert_eq!(a.run("ui.quickAccessToolbar", json!({"value": true})).unwrap()["quickAccessToolbar"], true);
        let mut c = app();
        c.start_screen();
        assert!(!c.has_document());
        assert_eq!(c.run("ui.quickAccessToolbar", json!({"value": false})).unwrap()["quickAccessToolbar"], false, "works without a document");
    }

    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
    }

    /// #766: started with nothing to open, the app shows File › Home (the Start screen) as Word
    /// does. #1129: no document is open behind it, so Esc (or Back) shows an empty window, with
    /// no page, the ribbon greyed out but for Help, and nothing to save on close. With the setting
    /// off it starts in the editor, and `WordApp::new` alone (tests, ui_shot, windows opened for a
    /// document) never shows it.
    #[test]
    fn start_screen_shows_file_home_and_esc_leaves_no_document() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        assert!(!a.ui.backstage, "`new` starts in the editor");
        assert!(a.has_document());
        a.ui.backstage_page = "print".into();
        assert!(a.start_screen(), "on by default");
        assert!(a.ui.backstage && a.ui.backstage_page == "home");
        assert!(!a.has_document(), "no document behind the Start screen");
        let mut h = harness(a);
        h.run_steps(4);
        assert!(h.query_by_label("Blank document").is_some(), "Home lists Blank document and the templates");
        assert!(h.query_by_label("Recent").is_some(), "and the Recent list");
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        let a = h.state_mut();
        assert!(!a.ui.backstage, "Esc leaves the Start screen");
        assert!(!a.has_document(), "for an empty window, not a blank document");
        assert_eq!(a.sent_title, "WordCraft", "the window is titled with the app alone");
        // The ribbon's commands are off, but Help, the File tab and the settings work.
        for id in ["format.bold", "text.insert", "file.save", "file.saveAs", "file.print", "edit.paste", "insert.table", "ui.print"] {
            assert!(!widgets::enabled(a, id), "{id} is off with no document");
        }
        for id in ["file.new", "file.open", "ui.backstage", "ui.tab", "ui.discord", "file.options", "ui.theme"] {
            assert!(widgets::enabled(a, id), "{id} works with no document");
        }
        assert!(!chrome::qat_enabled(a, "file.save"));
        // Only the listed interface commands work: the ones that act on the document don't.
        for id in ["ui.contextMenu", "ui.render", "ui.linkTextBox", "ui.zotero.refresh", "ui.unlistedCommand"] {
            assert!(!works_without_document(id), "{id} needs a document");
        }
        let refused = a.run("ui.zotero.status", json!({})).unwrap_err();
        assert!(refused.contains("No document is open"), "refused before it runs: {refused}");
        assert!(works_without_document("font.add"), "a font file dropped on the empty window is added");
        // Neither keys nor commands reach the hidden placeholder.
        assert!(a.run("text.insert", json!({"text": "Hello"})).is_err());
        assert!(a.execute("format.bold", json!({})).is_err());
        h.event(egui::Event::Text("typed".into()));
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let a = h.state_mut();
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "");
        assert!(!a.session.dirty && !a.has_document());
        assert!(a.close_requested(), "closing asks nothing");
        assert!(a.dialog.is_none());

        let mut b = app();
        b.run("ui.startScreen", json!({"value": false})).unwrap();
        assert!(!b.start_screen(), "off: today's behaviour");
        assert!(!b.ui.backstage && b.has_document());
    }

    /// #3045: on the empty window (no document behind the Start screen) the ribbon is greyed
    /// out, but the tab strip works: File reaches the Start screen again, where a document can
    /// be made. Disabling the Share / Editing / Comments row disabled the whole line, the tab
    /// strip with it: the tabs took clicks that did nothing.
    #[test]
    fn empty_window_ribbon_tabs_reach_the_start_screen() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.start_screen();
        let mut h = harness(a);
        h.run_steps(4);
        // Esc (or the back arrow) leaves the Start screen for the empty window, as #1129 has it.
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "the empty window");
        assert!(!h.state().has_document(), "with no document");
        // File, the first tab, is live and reaches the Start screen.
        h.get_by_label("File").click();
        h.run_steps(4);
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "home", "File reaches the Start screen");
        assert!(h.query_by_label("Blank document").is_some(), "where a document can be made");
    }

    /// #1908: with no document open the Home ribbon's split buttons grey out like the plain
    /// buttons beside them, and their drop-down arrow opens no menu.
    #[test]
    fn split_buttons_are_disabled_with_no_document() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut a = app();
        a.start_screen();
        a.run("ui.backstage", json!({"value": false})).unwrap();
        assert!(!a.has_document());
        let mut h = harness(a);
        h.run_steps(4);
        for tip in ["Underline", "Text Highlight Color", "Font Color", "Bullets", "Numbering", "Shading", "Borders"] {
            let node = h.query_by_role_and_label(Role::Button, tip);
            assert!(node.is_some_and(|n| n.accesskit_node().is_disabled()), "{tip} is greyed out with no document");
        }
        let r = h.get_by_role_and_label(Role::Button, "Underline").rect();
        let arrow = egui::pos2(r.max.x + 6.5, r.center().y);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: arrow, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        }
        h.run_steps(3);
        assert!(!egui::Popup::is_any_open(&h.ctx), "the disabled split button's arrow opens no menu");
    }

    /// #1129: the empty window's File tab goes back to the Backstage with only Home, New, Open
    /// and Options; picking Blank document there opens one, ready for typing.
    #[test]
    fn blank_document_opens_a_document_from_the_start_screen() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.start_screen();
        a.run("ui.backstage", json!({"value": false})).unwrap();
        a.run("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
        let first = a.session.document_id();
        let mut h = harness(a);
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "home", "Info needs a document");
        h.get_by_label("Blank document").click();
        h.run_steps(4);
        let a = h.state_mut();
        assert!(a.has_document() && !a.ui.backstage, "Blank document opens one");
        assert_ne!(a.session.document_id(), first);
        assert!(widgets::enabled(a, "format.bold"));
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Hello");
        assert!(a.sent_title.starts_with(&a.title_stem()), "{}", a.sent_title);
    }

    /// #1129: a template or a file opened from the Start screen (or handed over later) is the
    /// document; a failed open leaves the window empty.
    #[test]
    fn templates_and_files_open_from_the_start_screen() {
        let mut a = app();
        a.start_screen();
        a.run("file.new", json!({"template": "letter"})).unwrap();
        assert!(a.has_document());

        let dir = scratch("start-screen-open");
        let path = docx_on_disk(&dir, "Signed copy");
        let mut b = app();
        b.start_screen();
        assert!(b.run("file.open", json!({"path": dir.join("missing.docx").to_string_lossy()})).is_err());
        assert!(!b.has_document(), "a failed open opens nothing");
        b.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(b.has_document());
        assert!(body_text(&b).contains("Signed copy"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The Start screen setting is a UI command, kept between runs; settings saved before it
    /// existed leave it on, and opening saved settings never reopens the Backstage by itself.
    #[test]
    fn start_screen_setting_is_a_ui_command_and_survives_a_restart() {
        let mut a = app();
        assert_eq!(a.run("ui.startScreen", json!({})).unwrap(), json!({"startScreen": true}), "on by default; no value reads it");
        assert_eq!(a.run("ui.startScreen", json!({"value": false})).unwrap(), json!({"startScreen": false}));
        for junk in [json!({"value": "yes"}), json!({"value": 1}), json!({"value": null}), json!({"value": [true]})] {
            assert!(a.run("ui.startScreen", junk).is_err());
        }
        assert!(!a.ui.start_screen, "a rejected value keeps the setting");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""startScreen":false"#), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert!(!b.ui.start_screen && !b.start_screen());
        let old: UiState = serde_json::from_str(r#"{"tab":"Insert"}"#).unwrap();
        assert!(old.start_screen, "settings from before the option leave it on");
        let mut c = app();
        c.apply_prefs(old);
        assert!(!c.ui.backstage, "applying settings alone stays in the editor");
        assert!(c.start_screen() && c.ui.backstage_page == "home");
    }

    /// Insert › Icons (#309) opens the icon picker from the ribbon; an id inserts without it.
    #[test]
    fn icons_button_opens_the_picker_and_ids_insert_directly() {
        let mut a = app();
        a.run("insert.icon", json!({})).unwrap();
        assert_eq!(a.dialog.as_ref().map(dialogs::Dialog::name), Some("icons"));
        a.dialog = None;
        let r = a.run("insert.icon", json!({"id": "sun", "size": 36})).unwrap();
        assert_eq!(r["icon"], "sun");
        assert!(a.dialog.is_none(), "an id inserts without the dialog");
        assert!(a.selected_picture_media().is_some());
    }

    /// Table Layout › Properties (#44) opens the dialog from the ribbon; OK sends only what
    /// changed, as one `table.properties` step. Scripts with settings never see the dialog.
    #[test]
    fn table_properties_button_opens_the_dialog() {
        let mut a = app();
        a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        a.run("table.properties", json!({})).unwrap();
        let Some(dialogs::Dialog::TableProperties { form, basis }) = a.dialog.clone() else { panic!("no dialog: {:?}", a.dialog) };
        assert_eq!(form.changes(&basis), json!({}), "untouched, nothing changes");
        let mut f = (*form).clone();
        f.align = "center".into();
        f.row_height_on = true;
        f.row_height = 0.5;
        f.row_exact = true;
        f.header_row = true;
        let changes = f.changes(&basis);
        assert_eq!(changes, json!({"align": "center", "rowHeight": 36.0, "rowHeightRule": "exact", "headerRow": true}));
        a.dialog = None;
        a.run("table.properties", changes).unwrap();
        assert!(a.dialog.is_none(), "settings apply without the dialog");
        let (tp, _, _) = a.session.sel.focus.path.cell().unwrap();
        let t = a.session.doc.table(a.session.sel.focus.story, &tp).unwrap();
        assert_eq!(t.props.align, Some(wordcraft_doc::props::Align::Center));
        assert_eq!((t.rows[0].props.height, t.rows[0].props.header), (Some(36.0), true));
        assert_eq!(dialogs::TableForm::read(&a).unwrap().changes(&f), json!({}), "the dialog reopens with the new values");
        // Table direction (#362) is one more setting of the same step.
        let mut g = dialogs::TableForm::read(&a).unwrap();
        let basis = g.clone();
        g.rtl = true;
        assert_eq!(g.changes(&basis), json!({"rtl": true}));
    }

    /// Table Layout › Cell Margins opens with the table defaults and does not replace imported
    /// values until the user changes a value and confirms.
    #[test]
    fn cell_margins_button_preserves_existing_values_until_confirmed() {
        let mut a = app();
        a.run("insert.table", json!({"rows": 1, "cols": 1})).unwrap();
        let (tp, _, _) = a.session.sel.focus.path.cell().unwrap();
        let table = a.session.doc.table_mut(a.session.sel.focus.story, &tp).unwrap();
        table.props.cell_margins = Some([3.0, 6.0, 9.0, 12.0]);
        table.rows[0].cells[0].props.margins = Some([1.0, 2.0, 3.0, 4.0]);

        a.run("table.cellMargins", json!({})).unwrap();
        let Some(dialogs::Dialog::CellMargins { form, basis }) = a.dialog.clone() else { panic!("no dialog: {:?}", a.dialog) };
        assert_eq!((form.top, form.left, form.bottom, form.right), (3.0 / 72.0, 6.0 / 72.0, 9.0 / 72.0, 12.0 / 72.0));
        assert_eq!(form.changes(&basis), json!({}), "an untouched dialog does not reset margins");

        let mut changed = (*form).clone();
        changed.right = 18.0 / 72.0;
        let params = changed.changes(&basis);
        assert_eq!(params, json!({"top": 3.0, "left": 6.0, "bottom": 9.0, "right": 18.0}));
        a.dialog = None;
        a.run("table.cellMargins", params).unwrap();
        let table = a.session.doc.table(a.session.sel.focus.story, &tp).unwrap();
        assert_eq!(table.props.cell_margins, Some([3.0, 6.0, 9.0, 18.0]));
        assert_eq!(table.rows[0].cells[0].props.margins, Some([1.0, 2.0, 3.0, 4.0]), "cell-specific margins remain intact");
    }

    /// Home › Font › Enclose Characters (#297): the button opens the dialog on the selected
    /// character; the dialog's OK is `format.enclose`, and it reopens showing the enclosure.
    #[test]
    fn enclose_characters_button_opens_the_dialog() {
        let mut a = app();
        a.run("text.insert", json!({"text": "a 字 b"})).unwrap();
        a.run("select.text", json!({"text": "字"})).unwrap();
        a.run("format.enclose", json!({})).unwrap();
        let Some(dialogs::Dialog::Enclose { style, text, shape }) = a.dialog.clone() else { panic!("no dialog: {:?}", a.dialog) };
        assert_eq!((style.as_str(), text.as_str(), shape.as_str()), ("shrink", "字", "circle"));
        a.dialog = None;
        a.run("format.enclose", json!({"shape": "square", "style": "enlarge", "text": "字"})).unwrap();
        assert!(a.dialog.is_none(), "a shape applies without the dialog");
        a.run("format.enclose", json!({})).unwrap();
        let Some(dialogs::Dialog::Enclose { style, shape, .. }) = a.dialog.clone() else { panic!("no dialog") };
        assert_eq!((style.as_str(), shape.as_str()), ("enlarge", "square"));
    }

    /// Issue #772: the Find and Replace dialog left focus on the document, so typing didn't go into
    /// Find what. (Ctrl/⌘+F now searches in the Navigation pane, as in Word (#712), so this opens
    /// the dialog with Ctrl/⌘+H.)
    #[test]
    fn find_shortcut_focuses_find_what() {
        let ctx = egui::Context::default();
        let mut a = app();
        let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { events, ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        // The first frames install fonts; the canvas appears after them.
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let key = egui::Event::Key { key: egui::Key::H, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        frame(&mut a, vec![key]);
        for _ in 0..2 {
            frame(&mut a, Vec::new());
        }
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique("find_what"))), "Find what has the focus");
        frame(&mut a, vec![egui::Event::Text("abc".into())]);
        let Some(dialogs::Dialog::Find { query, .. }) = &a.dialog else { panic!("no Find dialog") };
        assert!(query.ends_with("abc"), "typing went into Find what: {query:?}");
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "nothing was typed into the document");
    }

    /// #1610: opening a hyperlink must direct typing and paste into its address.
    #[test]
    fn hyperlink_shortcut_focuses_address_and_preserves_document() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "Keep this text"})).unwrap();
        a.run("select.text", json!({"text": "Keep this text"})).unwrap();
        let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        for (event, expected) in [
            (egui::Event::Paste("https://example.com".into()), "https://example.com"),
            (egui::Event::Text("https://example.org".into()), "https://example.org"),
        ] {
            frame(&mut a, vec![key(egui::Key::K, egui::Modifiers::COMMAND)]);
            for _ in 0..2 {
                frame(&mut a, Vec::new());
            }
            assert!(ctx.memory(|m| m.has_focus(egui::Id::unique("link_address"))), "Address has focus");
            frame(&mut a, vec![event]);
            let Some(dialogs::Dialog::Link { url, text, .. }) = &a.dialog else { panic!("no Link dialog") };
            assert_eq!(url, expected, "input replaces the initial address");
            assert_eq!(text, "Keep this text");
            assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Keep this text");
            frame(&mut a, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
            frame(&mut a, Vec::new());
            assert!(a.dialog.is_none(), "Cancel closes the dialog");
            assert_eq!(a.session.selected_text(), "Keep this text", "Cancel preserves selection");
        }
    }

    /// Issue #139: Ctrl+wheel over the page didn't zoom.
    #[test]
    fn ctrl_wheel_over_canvas_zooms() {
        let ctx = egui::Context::default();
        let mut a = app();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        // The first frames install fonts; the canvas appears after them.
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let over_page = a.canvas.canvas_rect.unwrap().center();
        frame(&mut a, vec![egui::Event::PointerMoved(over_page)]);
        let before = a.canvas.scale;
        let wheel = |y: f32| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::COMMAND,
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&mut a, vec![wheel(1.0)]);
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        assert!(a.canvas.scale > before * 1.05, "Ctrl+wheel up zooms in: {before} -> {}", a.canvas.scale);
        let zoomed = a.canvas.scale;
        frame(&mut a, vec![wheel(-1.0)]);
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        assert!(a.canvas.scale < zoomed, "Ctrl+wheel down zooms out");
    }

    /// Issue #1223: Ctrl+wheel zoomed around the top of the scroll range, not the pointer, so the
    /// page slid away under it while zooming; once the pages reflowed into rows the view glided
    /// back to the caret. The place under the pointer stays put, rows or not. The events are the
    /// ones egui-winit sends on Linux: Ctrl sets both `ctrl` and `command`.
    #[test]
    fn ctrl_wheel_zoom_keeps_the_place_under_the_pointer() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(4000)})).unwrap();
        a.run("caret.docStart", json!({})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let pointer = a.canvas.canvas_rect.unwrap().center();
        frame(&mut a, vec![egui::Event::PointerMoved(pointer)]);
        let wheel = |y: f32, modifiers| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers,
            source: egui::MouseWheelSource::Unknown,
        };
        // Away from the caret, several pages down.
        for _ in 0..60 {
            frame(&mut a, vec![wheel(-1.0, egui::Modifiers::NONE)]);
        }
        for _ in 0..60 {
            frame(&mut a, Vec::new());
        }
        assert!(a.canvas.scroll_offset.y > 500.0, "scrolled down: {:?}", a.canvas.scroll_offset);
        let page = canvas::nearest_page(&a.canvas.page_rects, pointer).unwrap();
        assert!(page >= 3, "a few pages down: page {page}");
        let under = |a: &WordApp| (pointer - a.canvas.page_rects[page].min) / a.canvas.scale;
        let at = under(&a);
        let linux_ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        frame(&mut a, vec![egui::Event::ModifiersChanged(linux_ctrl)]);
        let (cols, before) = (a.canvas.cols, a.canvas.scale);
        for n in 0..40 {
            let events = if n % 8 == 0 { vec![wheel(-1.0, linux_ctrl)] } else { Vec::new() };
            frame(&mut a, events);
            let now = under(&a);
            // Across, the place can only be kept while the pages stay one per row: in rows they
            // fit the window side by side and there is nothing to scroll sideways.
            // Within a couple of screen pixels.
            let off = (now - at) * a.canvas.scale;
            let across = a.canvas.cols > cols || off.x.abs() < 2.0;
            assert!(
                across && off.y.abs() < 2.0,
                "frame {n} at {:.0}%: {now:?} on page {page}, was {at:?}",
                a.canvas.scale / canvas::PX_PER_PT * 100.0
            );
        }
        assert!(a.canvas.scale < before * 0.6, "zoomed out: {before} -> {}", a.canvas.scale);
        assert!(a.canvas.cols > cols, "the pages reflowed into rows on the way");
    }

    /// Issue #67: View › Zoom In from a fit mode zoomed *out*: Page Width showed 163% but Zoom In
    /// stepped from the stale manual 100% to 110%.
    #[test]
    fn view_tab_zoom_steps_from_the_shown_zoom_in_fit_modes() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("ui.tab", json!({"tab": "View"})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
        let frame = |a: &mut WordApp| {
            for _ in 0..4 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                ctx.run_ui(input, |ui| {
                    a.logic(ui.ctx());
                    a.ui(ui);
                })
                .drop_without_applying_deltas();
            }
            a.canvas.scale / canvas::PX_PER_PT
        };
        let base = frame(&mut a);
        assert!((base - 1.0).abs() < 1e-3);
        for fit in ["view.pageWidth", "view.onePage", "view.multiplePages"] {
            a.run(fit, json!({})).unwrap();
            let fitted = frame(&mut a);
            assert!((fitted - base).abs() > 0.05, "{fit} changes the zoom");
            assert!((a.session.view.zoom - fitted).abs() < 1e-3, "{fit}: session zoom follows the shown zoom");
            a.run("view.zoomIn", json!({})).unwrap();
            let zin = frame(&mut a);
            assert!(zin > fitted, "{fit}: Zoom In zooms in: {fitted} -> {zin}");
            a.run(fit, json!({})).unwrap();
            frame(&mut a);
            a.run("view.zoomOut", json!({})).unwrap();
            let zout = frame(&mut a);
            assert!(zout < fitted, "{fit}: Zoom Out zooms out: {fitted} -> {zout}");
            // The Zoom dialog opens at the shown zoom.
            a.run(fit, json!({})).unwrap();
            frame(&mut a);
            a.run("ui.dialog", json!({"name": "zoom"})).unwrap();
            assert!(matches!(a.dialog, Some(dialogs::Dialog::Zoom { percent }) if (percent - (fitted * 100.0).round()).abs() < 1.0));
            a.dialog = None;
        }
        a.run("view.zoom100", json!({})).unwrap();
        assert!((frame(&mut a) - 1.0).abs() < 1e-3);
    }

    /// Issue #122: touchpad deltas scroll the page 1:1 at once; a wheel notch eases in to about
    /// three lines.
    #[test]
    fn touchpad_scrolls_one_to_one_and_wheel_notches_ease_in() {
        let ctx = egui::Context::default();
        let mut a = app();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let over_page = a.canvas.canvas_rect.unwrap().center();
        frame(&mut a, vec![egui::Event::PointerMoved(over_page)]);
        let wheel = |unit, y: f32, phase| egui::Event::MouseWheel {
            unit,
            delta: egui::vec2(0.0, y),
            phase,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        };
        let start = a.canvas.scroll_offset.y;
        frame(&mut a, vec![wheel(egui::MouseWheelUnit::Point, -40.0, egui::TouchPhase::Start)]);
        assert_eq!(a.canvas.scroll_offset.y, start + 40.0, "the first touchpad delta lands in the same frame");
        // Fingers lift; the coast after it is left to the unit tests (crate::scroll).
        frame(&mut a, vec![wheel(egui::MouseWheelUnit::Point, 0.0, egui::TouchPhase::End)]);
        a.canvas.wheel = Default::default();
        let at = a.canvas.scroll_offset.y;
        frame(&mut a, vec![wheel(egui::MouseWheelUnit::Line, -1.0, egui::TouchPhase::Move)]);
        let first = a.canvas.scroll_offset.y - at;
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let notch = crate::scroll::notch_px(a.canvas.scale / crate::canvas::PX_PER_PT);
        assert!(first > 0.0 && first < notch, "a notch eases in: {first}");
        assert!((a.canvas.scroll_offset.y - at - notch).abs() < 0.5, "a notch scrolls {notch}: {}", a.canvas.scroll_offset.y - at);
    }

    /// Web Layout (#1876): text wraps to the work area, re-wraps once a resize has settled (not on
    /// every frame of a drag) and at once on a zoom, and the width is clamped. Print Layout keeps its
    /// page width and Draft keeps its 800 pt column after a visit to Web Layout.
    #[test]
    fn web_layout_fills_the_window_width() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "A long paragraph that wraps across the work area. ".repeat(200)})).unwrap();
        let mut t = 0.0;
        // One frame `dt` seconds after the last one, in a window `w` wide.
        let mut frame = |a: &mut WordApp, w: f32, dt: f64| {
            t += dt;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 740.0));
            let input = egui::RawInput { events: vec![], time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        let lines = |a: &mut WordApp| {
            a.session
                .layout()
                .pages
                .iter()
                .flat_map(|p| p.items.iter())
                .map(|it| if let wordcraft_layout::Placed::Lines { l0, l1, .. } = it { l1 - l0 } else { 0 })
                .sum::<usize>()
        };
        let tick = 1.0 / 60.0;
        for _ in 0..3 {
            frame(&mut a, 1400.0, tick);
        }
        let print_lines = lines(&mut a);
        a.run("view.draft", json!({})).unwrap();
        frame(&mut a, 1400.0, tick);
        let draft_lines = lines(&mut a);
        a.run("view.webLayout", json!({})).unwrap();
        frame(&mut a, 1400.0, tick);
        let wide = a.session.view.web_width;
        assert!((900.0..1400.0).contains(&wide), "entering Web Layout fills the window at once, not 800: {wide}");
        let wide_lines = lines(&mut a);
        frame(&mut a, 1400.0, tick);
        assert_eq!(a.session.view.web_width, wide, "no change without a resize");
        let page = a.canvas.page_rects[0];
        assert!(page.left() < 4.0, "page starts at the left edge: {page:?}");
        // A window drag: the width holds while the size keeps changing, then follows once it settles.
        for w in [1300.0, 1100.0, 900.0, 700.0, 500.0] {
            frame(&mut a, w, tick);
            assert_eq!(a.session.view.web_width, wide, "no relayout mid-drag at {w}");
        }
        frame(&mut a, 500.0, 0.2);
        let narrow = a.session.view.web_width;
        assert!((144.0..500.0).contains(&narrow), "{narrow}");
        assert!(lines(&mut a) > wide_lines, "narrower window wraps more");
        // A zoom re-wraps at once: at 200% the window holds half as many points.
        frame(&mut a, 1400.0, 0.2);
        frame(&mut a, 1400.0, 0.2);
        let at_100 = a.session.view.web_width;
        a.run("view.zoom", json!({"value": 200})).unwrap();
        frame(&mut a, 1400.0, tick);
        let at_200 = a.session.view.web_width;
        assert!((at_200 - at_100 / 2.0).abs() < 2.0, "200% halves the width: {at_100} -> {at_200}");
        a.run("view.zoom", json!({"value": 100})).unwrap();
        frame(&mut a, 20.0, 0.2);
        frame(&mut a, 20.0, 0.2);
        assert!(a.session.view.web_width >= 144.0);
        frame(&mut a, 1400.0, 0.2);
        frame(&mut a, 1400.0, 0.2);
        a.run("view.draft", json!({})).unwrap();
        frame(&mut a, 1400.0, tick);
        assert_eq!(a.session.layout().pages[0].w, 800.0, "Draft keeps its column after Web Layout");
        assert_eq!(lines(&mut a), draft_lines, "Draft wraps as before Web Layout");
        a.run("view.printLayout", json!({})).unwrap();
        for _ in 0..3 {
            frame(&mut a, 1400.0, tick);
        }
        assert_eq!(lines(&mut a), print_lines, "Print Layout is unchanged");
    }

    /// Read Mode turns columns with the keyboard (#1772): Right/Page Down forward, Left/Page Up back.
    #[test]
    fn reading_mode_keys_turn_columns() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "One readable paragraph in many narrow columns. ".repeat(300)})).unwrap();
        a.run("caret.set", json!({"pos": wordcraft_doc::Pos::body(0, 0)})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 740.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, vec![]);
        }
        a.run("view.readMode", json!({})).unwrap();
        for _ in 0..3 {
            frame(&mut a, vec![]);
        }
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let step = a.session.view.read_width * a.canvas.scale + 18.0;
        assert!(a.canvas.focused, "the canvas has the keyboard");
        frame(&mut a, vec![key(egui::Key::PageDown)]);
        for _ in 0..30 {
            frame(&mut a, vec![]);
        }
        assert!((a.canvas.scroll_offset.x - step).abs() < 1.0, "Page Down turns one column: {}", a.canvas.scroll_offset.x);
        frame(&mut a, vec![key(egui::Key::ArrowRight)]);
        for _ in 0..30 {
            frame(&mut a, vec![]);
        }
        assert!((a.canvas.scroll_offset.x - 2.0 * step).abs() < 1.0, "Right turns one column: {}", a.canvas.scroll_offset.x);
        frame(&mut a, vec![key(egui::Key::ArrowLeft)]);
        for _ in 0..30 {
            frame(&mut a, vec![]);
        }
        assert!((a.canvas.scroll_offset.x - step).abs() < 1.0, "Left turns back: {}", a.canvas.scroll_offset.x);
        frame(&mut a, vec![key(egui::Key::Escape)]);
        assert!(!a.session.view.read_mode, "Escape leaves Read Mode");
    }

    /// Reading Mode: reflowed viewport-height columns, horizontal wheel/navigation, exit restores
    /// Print Layout without editing or changing the document's page setup.
    #[test]
    fn reading_mode_columns_wheel_turns_and_navigation() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "One readable paragraph in many narrow columns. ".repeat(300)})).unwrap();
        a.run("caret.set", json!({"pos": wordcraft_doc::Pos::body(0, 0)})).unwrap();
        let original = a.session.doc.settings.clone();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 740.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>, screen| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, vec![], screen);
        }
        a.run("view.readMode", json!({})).unwrap();
        for _ in 0..3 {
            frame(&mut a, vec![], screen);
        }
        let rects = &a.canvas.page_rects;
        assert!(rects.len() > 5, "reading should reflow: {}", rects.len());
        assert_eq!(a.canvas.cols, rects.len(), "one unwrapped row");
        assert!((rects[0].top() - rects[1].top()).abs() < 0.1);
        assert!(rects[1].left() > rects[0].right());
        // Full height, less the top gap and the page-turn bar's strip at the bottom (#1772).
        assert!((rects[0].height() - (a.canvas.canvas_rect.unwrap().height() - 62.0)).abs() < 2.0);
        let initial_width = a.session.view.read_width;
        // A simple mouse click in a later visible column must neither pull that column
        // to the leading edge nor select text under the stationary, held pointer.
        let click = a.canvas.page_rects[2].center();
        assert!(a.canvas.canvas_rect.unwrap().contains(click));
        let initial_scroll = a.canvas.scroll_offset.x;
        let button =
            |pressed| egui::Event::PointerButton { pos: click, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut a, vec![egui::Event::PointerMoved(click), button(true)], screen);
        assert_eq!(a.session.layout().caret_on(&a.session.sel.focus, a.session.page_hint).unwrap().page, 2);
        frame(&mut a, vec![], screen); // mouse is still held at the same coordinates
        assert!((a.canvas.scroll_offset.x - initial_scroll).abs() < 0.5, "click must not scroll a later column to the left");
        assert!(a.session.sel.is_collapsed(), "stationary click must not create a selection");
        frame(&mut a, vec![button(false)], screen);
        frame(&mut a, vec![], screen);
        assert!((a.canvas.scroll_offset.x - initial_scroll).abs() < 0.5);
        assert!(a.session.sel.is_collapsed());
        // An explicit caret move outside the visible columns must still reveal its destination.
        a.run("caret.set", json!({"pos": wordcraft_doc::Pos::body(0, 5000)})).unwrap();
        frame(&mut a, vec![], screen);
        assert!(a.canvas.scroll_offset.x > initial_scroll + a.session.view.read_width * a.canvas.scale);
        a.run("caret.set", json!({"pos": wordcraft_doc::Pos::body(0, 0)})).unwrap();
        frame(&mut a, vec![], screen);
        assert!((a.canvas.scroll_offset.x - initial_scroll).abs() < 0.5);
        let over = a.canvas.canvas_rect.unwrap().center();
        frame(&mut a, vec![egui::Event::PointerMoved(over)], screen);
        frame(
            &mut a,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -65.0),
                phase: egui::TouchPhase::Start,
                modifiers: egui::Modifiers::NONE,
                source: egui::MouseWheelSource::Unknown,
            }],
            screen,
        );
        let step = a.session.view.read_width * a.canvas.scale + 18.0;
        assert!(a.canvas.scroll_offset.x > 0.0 && a.canvas.scroll_offset.x < step, "a wheel turn animates instead of jumping");
        let first = a.canvas.scroll_offset.x;
        frame(&mut a, vec![], screen);
        assert!(a.canvas.scroll_offset.x > first, "smooth motion continues after the wheel event");
        for _ in 0..23 {
            frame(&mut a, vec![], screen);
        }
        assert!((a.canvas.scroll_offset.x - step).abs() < 1.0, "animation settles exactly on column boundary");
        assert_eq!(a.canvas.scroll_offset.y, 0.0);
        canvas::turn_read_page(&mut a, 1.0);
        for _ in 0..25 {
            frame(&mut a, vec![], screen);
        }
        assert!((a.canvas.scroll_offset.x - 2.0 * step).abs() < 1.0);
        let x = a.canvas.scroll_offset.x;
        // A heading navigation request must position its column at the leading edge,
        // not wherever egui's generic scroll_to_rect decides the caret is visible.
        let target = wordcraft_doc::Pos::body(0, 5000);
        a.run("view.navigationPane", json!({"value": true})).unwrap();
        a.run("caret.set", json!({"pos": target})).unwrap();
        a.canvas.nav_to_caret = true;
        a.canvas.scroll_to_caret = false;
        frame(&mut a, vec![], screen);
        let page = a.session.layout().caret_on(&a.session.sel.focus, a.session.page_hint).unwrap().page;
        let step = a.session.view.read_width * a.canvas.scale + 18.0;
        assert!((a.canvas.scroll_offset.x - (page as f32 * step)).abs() < 1.0, "heading must align its column beside the navigation pane");
        assert!(a.canvas.scroll_offset.x != x);
        // Resizing changes the content width (reflow), not the zoom or the print paper width.
        let narrow = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(850.0, 630.0));
        let before_resize = a.canvas.scroll_offset.x;
        let before_layout = a.session.layout();
        let anchor = a.canvas.read_first_visible.clone().unwrap();
        frame(&mut a, vec![], narrow);
        assert!(std::sync::Arc::ptr_eq(&before_layout, &a.session.layout()), "resize frames reuse cached pagination");
        assert!((a.canvas.scroll_offset.x - before_resize).abs() < 1.0, "no jump while resize is in progress");
        assert_eq!(a.session.view.read_width, initial_width, "expensive repagination is deferred during resizing");
        for _ in 0..15 {
            frame(&mut a, vec![], narrow);
        }
        assert_ne!(a.session.view.read_width, initial_width);
        assert!(!std::sync::Arc::ptr_eq(&before_layout, &a.session.layout()), "stable viewport commits one repagination");
        let target_page = a.session.layout().caret_on(&anchor, 0).unwrap().page;
        let new_step = a.session.view.read_width * a.canvas.scale + 18.0;
        assert!(
            (a.canvas.scroll_offset.x - target_page as f32 * new_step).abs() < 1.0,
            "resize must preserve visible text anchor, not jump to editing caret"
        );
        assert!((a.canvas.scale - canvas::PX_PER_PT).abs() < 0.01);
        a.run("view.readColumnWidth", json!({"value": "page"})).unwrap();
        frame(&mut a, vec![], narrow);
        assert!(a.session.view.read_width <= 612.0);
        a.run("view.readTextSize", json!({"value": 125})).unwrap();
        frame(&mut a, vec![], narrow);
        assert!((a.canvas.scale / canvas::PX_PER_PT - 1.25).abs() < 0.01);
        // The Edit Document button works even when the navigation pane owns focus.
        a.run("view.printLayout", json!({})).unwrap();
        frame(&mut a, vec![], narrow);
        assert!(!a.session.view.read_mode);
        assert_eq!(a.session.view.mode, wordcraft_layout::ViewMode::Print);
        assert_eq!(a.session.doc.settings, original);
    }

    /// An app whose host reports the system's animation setting as `system` holds it.
    fn app_with_system_animations(system: &std::rc::Rc<std::cell::Cell<Option<bool>>>) -> WordApp {
        let system = system.clone();
        let services = Services { animations_enabled: Some(Box::new(move || system.get())), ..Default::default() };
        WordApp::new(Session::new(wordcraft_doc::Document::new()), services)
    }

    /// Animation effects: "Use system setting" (the default) follows what the host reports and is
    /// on when it reports nothing; On and Off ignore the system. The choice is kept between runs.
    #[test]
    fn animation_effects_setting_follows_the_system_and_survives_a_restart() {
        use theme::Animations;
        for (setting, system, on) in [
            (Animations::System, None, true),
            (Animations::System, Some(true), true),
            (Animations::System, Some(false), false),
            (Animations::On, Some(false), true),
            (Animations::Off, Some(true), false),
            (Animations::Off, None, false),
        ] {
            assert_eq!(setting.enabled(system), on, "{setting:?} with the system at {system:?}");
        }
        // No host hook (tests, headless screenshots): nothing is reported, so they are on.
        let mut a = app();
        assert_eq!(a.run("ui.animations", json!({})).unwrap(), json!({"animations": "system", "enabled": true, "system": null}));
        // The system has them off, as Windows reports with Animation effects switched off.
        let system = std::rc::Rc::new(std::cell::Cell::new(Some(false)));
        let mut a = app_with_system_animations(&system);
        assert_eq!(a.run("ui.animations", json!({})).unwrap(), json!({"animations": "system", "enabled": false, "system": false}));
        assert_eq!(a.run("ui.animations", json!({"value": "On"})).unwrap()["enabled"], true);
        system.set(Some(true));
        assert_eq!(a.run("ui.animations", json!({"value": "off"})).unwrap(), json!({"animations": "off", "enabled": false, "system": true}));
        assert!(a.run("ui.animations", json!({"value": "sometimes"})).is_err());
        assert_eq!(a.ui.animations, Animations::Off, "a rejected value keeps the setting");
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""animations":"off""#), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.ui.animations, Animations::Off);
        assert!(!b.animations_on());
        // Preferences from before the setting, or with a value it doesn't know, follow the system.
        for raw in [r#"{"tab": "Insert"}"#, r#"{"animations": "wobbly", "tab": "Insert"}"#, r#"{"animations": 3, "tab": "Insert"}"#] {
            let ui = UiState::from_json(raw.as_bytes()).unwrap();
            assert_eq!((ui.animations, ui.tab.as_str()), (Animations::System, "Insert"), "{raw}");
        }
    }

    /// With the system's animation effects off (what Windows reports with Settings › Accessibility ›
    /// Visual effects › Animation effects off), a wheel notch scrolls the page its whole distance
    /// in one frame and egui's own fades and scroll glides are off. A change of the system's
    /// setting is followed from the next frame.
    #[test]
    fn wheel_notches_land_at_once_with_animation_effects_off() {
        let ctx = egui::Context::default();
        let system = std::rc::Rc::new(std::cell::Cell::new(Some(false)));
        let mut a = app_with_system_animations(&system);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        assert!(!a.animations_on());
        assert_eq!(ctx.global_style().animation_time, 0.0, "no fades");
        assert_eq!(ctx.global_style().scroll_animation, egui::style::ScrollAnimation::none(), "no glide to the caret");
        let over_page = a.canvas.canvas_rect.unwrap().center();
        frame(&mut a, vec![egui::Event::PointerMoved(over_page)]);
        let notch_down = || egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        };
        let notch = crate::scroll::notch_px(a.canvas.scale / crate::canvas::PX_PER_PT);
        let at = a.canvas.scroll_offset.y;
        frame(&mut a, vec![notch_down()]);
        let landed = a.canvas.scroll_offset.y;
        assert!((landed - at - notch).abs() < 0.5, "the whole notch ({notch}) lands in its frame: {}", landed - at);
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.canvas.scroll_offset.y, landed, "nothing is left to ease in");
        // Animation effects are switched back on in the system's settings: the next frame has it.
        system.set(Some(true));
        frame(&mut a, Vec::new());
        assert!(a.animations_on());
        assert_eq!(ctx.global_style().animation_time, egui::Style::default().animation_time);
        assert_eq!(ctx.global_style().scroll_animation, egui::Style::default().scroll_animation);
        frame(&mut a, vec![notch_down()]);
        let first = a.canvas.scroll_offset.y - landed;
        assert!(first > 0.0 && first < notch, "a notch eases in again: {first}");
    }

    /// With animation effects off a zoom step is finished in its own frame: Ctrl+wheel zooms the
    /// notch's whole step at once, the page is already placed and drawn as it will stay, and the
    /// status bar's zoom readout follows in the very next frame. Turns that arrive in small pieces
    /// (a touchpad pinch on Windows) add up to a step first rather than each zooming a little.
    #[test]
    fn zoom_steps_land_finished_with_animation_effects_off() {
        let ctx = egui::Context::default();
        let system = std::rc::Rc::new(std::cell::Cell::new(Some(false)));
        let mut a = app_with_system_animations(&system);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut t = 0.0;
        // One frame; returns how soon it asked to be drawn again.
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            let out = ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            });
            let again = out.viewport_output.values().map(|v| v.repaint_delay).min();
            out.drop_without_applying_deltas();
            again
        };
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let over_page = a.canvas.canvas_rect.unwrap().center();
        frame(&mut a, vec![egui::Event::PointerMoved(over_page)]);
        frame(&mut a, Vec::new());
        let ctrl_wheel = |notches: f32| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, notches),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::COMMAND,
            source: egui::MouseWheelSource::Unknown,
        };
        let opts = egui::InputOptions::default();
        let notch_zoom = (opts.scroll_zoom_speed * opts.line_scroll_speed).exp();
        let before = a.canvas.scale;
        frame(&mut a, vec![ctrl_wheel(1.0)]);
        let (scale, pages) = (a.canvas.scale, a.canvas.page_rects.clone());
        assert!((scale / before - notch_zoom).abs() < 1e-3, "the whole notch in its frame: {before} -> {scale}");
        for _ in 0..30 {
            frame(&mut a, Vec::new());
            assert_eq!(a.canvas.scale, scale, "nothing is left to ease in");
            assert_eq!(a.canvas.page_rects, pages, "the page was already where and how it stays");
        }
        // Quarter notches: a step of about 10% with every second one, nothing in between.
        frame(&mut a, vec![ctrl_wheel(-0.25)]);
        assert_eq!(a.canvas.scale, scale, "a quarter notch isn't a step yet");
        frame(&mut a, vec![ctrl_wheel(-0.25)]);
        let stepped = a.canvas.scale;
        assert!((stepped / scale - (-0.1_f32).exp()).abs() < 1e-3, "{scale} -> {stepped}");
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.canvas.scale, stepped);
        // Zoom Out (the status bar's minus, View › Zoom Out): the page changes in that frame, and
        // the frame asks for another at once so the readout beside the slider isn't left behind.
        assert_ne!(frame(&mut a, Vec::new()), Some(std::time::Duration::ZERO), "idle: nothing to redraw at once");
        a.run("view.zoomOut", json!({})).unwrap();
        assert_eq!(frame(&mut a, Vec::new()), Some(std::time::Duration::ZERO));
        let (scale, pages) = (a.canvas.scale, a.canvas.page_rects.clone());
        assert!(scale < stepped);
        frame(&mut a, Vec::new());
        assert_eq!((a.canvas.scale, &a.canvas.page_rects), (scale, &pages));
    }

    /// Issue #1743: Ctrl/⌘ + wheel only zooms. The one view movement is the anchor correction that
    /// keeps the page point under the pointer there; the touchpad coast still running when the
    /// wheel starts zooming (animation effects off) doesn't scroll on top of it.
    #[test]
    fn ctrl_wheel_zoom_does_not_scroll_the_document() {
        for modifiers in [egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
            let ctx = egui::Context::default();
            let system = std::rc::Rc::new(std::cell::Cell::new(Some(false)));
            let mut a = app_with_system_animations(&system);
            a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
            let mut t = 0.0;
            let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
                t += 1.0 / 60.0;
                let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
                ctx.run_ui(input, |ui| {
                    a.logic(ui.ctx());
                    a.ui(ui);
                })
                .drop_without_applying_deltas();
            };
            for _ in 0..3 {
                frame(&mut a, Vec::new());
            }
            let rect = a.canvas.canvas_rect.unwrap();
            let pointer = rect.center() + egui::vec2(30.0, 40.0);
            frame(&mut a, vec![egui::Event::PointerMoved(pointer)]);
            let wheel = |delta: f32, modifiers| egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, delta),
                phase: egui::TouchPhase::Move,
                modifiers,
                source: egui::MouseWheelSource::Unknown,
            };
            // Scroll down a little, so the pointer is well inside the document.
            frame(&mut a, vec![wheel(-3.0, egui::Modifiers::NONE)]);
            for _ in 0..40 {
                frame(&mut a, Vec::new());
            }
            let layout = a.session.layout();
            let under = |a: &WordApp| canvas::page_at(&a.canvas.page_rects, &layout, a.canvas.scale, pointer);
            let (page, x, y) = under(&a).unwrap();
            let scale = a.canvas.scale;
            frame(&mut a, vec![wheel(50.0, modifiers)]);
            let mut last = a.canvas.scroll_offset;
            for n in 0..60 {
                last = a.canvas.scroll_offset;
                if n > 0 {
                    frame(&mut a, Vec::new());
                }
                let (p, px, py) = under(&a).unwrap();
                assert_eq!(p, page);
                assert!(
                    ((px - x).abs() < 1.0 || a.canvas.scroll_offset.x == 0.0) && (py - y).abs() < 1.0,
                    "the page point stays under the pointer ({modifiers:?} frame {n}): ({x},{y}) -> ({px},{py})"
                );
            }
            assert_eq!(a.canvas.scroll_offset, last, "nothing scrolls once the zoom has settled ({modifiers:?})");
            assert!(a.canvas.scale > scale, "zoomed ({modifiers:?})");
        }
    }

    /// Issue #688: dragging a selection past the viewport's bottom edge scrolls on, and the
    /// selection keeps growing as the content moves under the stationary pointer.
    #[test]
    fn dragging_a_selection_past_the_viewport_edge_auto_scrolls() {
        let ctx = egui::Context::default();
        let mut a = app();
        // A few pages of text, so there is content below the fold.
        a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let (Some(vp), Some(page)) = (a.canvas.viewport, a.canvas.page_rects.first()) else { panic!("the canvas laid out") };
        let start = page.left_top() + egui::vec2(120.0, 150.0);
        frame(&mut a, vec![egui::Event::PointerMoved(start)]);
        frame(
            &mut a,
            vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
        );
        assert!(a.canvas.drag_selecting(), "the press started a drag-select");
        let base = a.session.sel.focus.clone();
        // Hold the pointer just past the viewport's bottom edge and let the frames run.
        let below = egui::pos2(vp.center().x, vp.max.y + 40.0);
        for _ in 0..30 {
            frame(&mut a, vec![egui::Event::PointerMoved(below)]);
        }
        assert!(a.canvas.scroll_offset.y > 0.0, "the view scrolled toward later content");
        assert!(a.session.sel.focus > base, "the selection followed the content");
        // The pointer never moves; the content keeps scrolling under it, so the selection must
        // keep growing.
        let grown = a.session.sel.focus.clone();
        for _ in 0..15 {
            frame(&mut a, vec![egui::Event::PointerMoved(below)]);
        }
        assert!(a.session.sel.focus > grown, "the selection keeps growing under a stationary pointer");
        frame(
            &mut a,
            vec![egui::Event::PointerButton { pos: below, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }],
        );
        assert!(!a.canvas.drag_selecting());
        assert!(!a.session.sel.is_collapsed(), "the drag left a selection");
    }

    /// Issue #1388: the mouse wheel did nothing while a selection was being dragged, so a passage
    /// longer than the screen couldn't be selected in one go.
    #[test]
    fn wheel_scrolls_the_view_while_drag_selecting() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
        // Start at the top: typing left the caret (and the view) at the end.
        a.run("caret.docStart", json!({})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a, Vec::new());
        }
        let (Some(vp), Some(page)) = (a.canvas.viewport, a.canvas.page_rects.first()) else { panic!("the canvas laid out") };
        let start = page.left_top() + egui::vec2(120.0, 150.0);
        frame(&mut a, vec![egui::Event::PointerMoved(start)]);
        frame(
            &mut a,
            vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
        );
        // Drag to the middle of the view, well away from the edges that auto-scroll.
        let mid = vp.center();
        for _ in 0..5 {
            frame(&mut a, vec![egui::Event::PointerMoved(mid)]);
        }
        assert!(a.canvas.drag_selecting(), "the press started a drag-select");
        assert_eq!(a.canvas.scroll_offset.y, 0.0, "nothing scrolled before the wheel");
        let base = a.session.sel.focus.clone();
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&mut a, vec![egui::Event::PointerMoved(mid), wheel]);
        for _ in 0..30 {
            frame(&mut a, vec![egui::Event::PointerMoved(mid)]);
        }
        assert!(a.canvas.scroll_offset.y > 0.0, "the wheel scrolled the view during the drag");
        assert!(a.session.sel.focus > base, "the selection followed the content under the pointer");
        frame(
            &mut a,
            vec![egui::Event::PointerButton { pos: mid, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }],
        );
        assert!(!a.session.sel.is_collapsed(), "the drag left a selection");
    }

    /// #1013: a press inside the selected comment's balloon (to move the caret in its text) left
    /// the balloon, because the canvas under it also "contained" the pointer.
    #[test]
    fn a_press_inside_the_selected_comment_keeps_it_selected() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "Some text to comment on. ".repeat(40)})).unwrap();
        a.run("select.word", json!({})).unwrap();
        a.run("review.newComment", json!({"text": "A comment with a few words in it"})).unwrap();
        a.session.view.comments_pane = false;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 700.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let (id, card) = *a.canvas.balloon_rects.first().expect("the comment has a balloon");
        frame(&mut a, vec![egui::Event::PointerMoved(card.center())]);
        frame(&mut a, vec![btn(card.center(), true)]);
        frame(&mut a, vec![btn(card.center(), false)]);
        for _ in 0..5 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.canvas.balloon, Some(id), "a click on the balloon selects it");
        let (_, card) = *a.canvas.balloon_rects.iter().find(|(i, _)| *i == id).unwrap();
        let (focus, scroll) = (a.session.sel.clone(), a.canvas.scroll_offset);
        // Press, drag a little and release inside the comment's text.
        let (from, to) = (card.left_top() + egui::vec2(30.0, 30.0), card.left_top() + egui::vec2(80.0, 30.0));
        frame(&mut a, vec![egui::Event::PointerMoved(from)]);
        frame(&mut a, vec![btn(from, true)]);
        assert_eq!(a.canvas.balloon, Some(id), "the press stays in the balloon");
        for _ in 0..3 {
            frame(&mut a, vec![egui::Event::PointerMoved(to)]);
        }
        assert!(!a.canvas.drag_selecting(), "the press didn't start a drag in the document");
        frame(&mut a, vec![btn(to, false)]);
        assert_eq!(a.canvas.balloon, Some(id), "still selected after the release");
        assert_eq!(a.session.sel, focus, "the document's caret didn't move");
        assert_eq!(a.canvas.scroll_offset, scroll);
        // A press on the page itself still leaves the balloon.
        let page = a.canvas.page_rects.first().copied().unwrap();
        let body = page.left_top() + egui::vec2(120.0, 160.0);
        frame(&mut a, vec![egui::Event::PointerMoved(body)]);
        frame(&mut a, vec![btn(body, true)]);
        frame(&mut a, vec![btn(body, false)]);
        assert_eq!(a.canvas.balloon, None, "a click on the page leaves the balloon");
    }

    /// #1055: with "only for the selected text" the margin column is gone and just the comment
    /// under the caret floats, beside the page in a wide window and over its edge in a narrow one.
    #[test]
    fn selected_comment_floats_and_keeps_its_clicks() {
        for width in [1700.0_f32, 1000.0] {
            let ctx = egui::Context::default();
            let mut a = app();
            a.run("text.insert", json!({"text": "one two three. ".repeat(60)})).unwrap();
            a.run("select.text", json!({"text": "two"})).unwrap();
            a.run("review.newComment", json!({"text": "A comment with a few words in it"})).unwrap();
            a.session.view.comments_pane = false;
            assert!(canvas::markup_width(&a) > 0.0, "the margin is the default, as in Word");
            a.run("review.trackingOptions", json!({"commentDisplay": "selected"})).unwrap();
            assert_eq!(canvas::markup_width(&a), 0.0, "no margin column any more");
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 700.0));
            let mut t = 0.0;
            let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
                t += 1.0 / 60.0;
                let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
                ctx.run_ui(input, |ui| {
                    a.logic(ui.ctx());
                    a.ui(ui);
                })
                .drop_without_applying_deltas();
            };
            let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            for _ in 0..30 {
                frame(&mut a, Vec::new());
            }
            // The caret is in the text after the first comment: not inside its range.
            a.run("caret.docStart", json!({})).unwrap();
            for _ in 0..5 {
                frame(&mut a, Vec::new());
            }
            assert!(a.canvas.balloon_rects.is_empty(), "width {width}: no comment under the caret, no card");
            // Inside the commented word.
            a.run("select.text", json!({"text": "two"})).unwrap();
            a.run("caret.left", json!({})).unwrap();
            for _ in 0..5 {
                frame(&mut a, Vec::new());
            }
            assert_eq!(a.canvas.balloon_rects.len(), 1, "width {width}: the comment under the caret shows");
            let (id, card) = a.canvas.balloon_rects[0];
            let page = a.canvas.page_rects.first().copied().unwrap();
            assert_eq!(
                card.left() < page.right(),
                width < 1100.0,
                "width {width}: beside the page when the window has room, else over its edge: {card:?} {page:?}"
            );
            if width < 1100.0 {
                let line = a.session.layout().caret_on(&a.session.sel.focus, a.session.page_hint).expect("caret laid out");
                let bottom = page.min.y + (line.top + line.height) * canvas::PX_PER_PT;
                assert!(card.top() >= bottom, "width {width}: over the page the card sits under the commented line: {card:?} vs {bottom}");
            }
            // The #1013 flow on the floating card: click, press inside, drag, release.
            let (caret, scroll) = (a.session.sel.clone(), a.canvas.scroll_offset);
            frame(&mut a, vec![egui::Event::PointerMoved(card.center())]);
            frame(&mut a, vec![btn(card.center(), true)]);
            frame(&mut a, vec![btn(card.center(), false)]);
            for _ in 0..5 {
                frame(&mut a, Vec::new());
            }
            assert_eq!(a.canvas.balloon, Some(id), "width {width}: a click on the card selects it");
            assert_eq!(a.session.sel.focus.path, caret.focus.path, "width {width}: the document's caret stays in its paragraph");
            let (_, card) = *a.canvas.balloon_rects.iter().find(|(i, _)| *i == id).unwrap();
            let (from, to) = (card.left_top() + egui::vec2(30.0, 30.0), card.left_top() + egui::vec2(80.0, 30.0));
            let caret = a.session.sel.clone();
            frame(&mut a, vec![egui::Event::PointerMoved(from)]);
            frame(&mut a, vec![btn(from, true)]);
            for _ in 0..3 {
                frame(&mut a, vec![egui::Event::PointerMoved(to)]);
            }
            assert!(!a.canvas.drag_selecting(), "width {width}: the press didn't start a drag in the document");
            frame(&mut a, vec![btn(to, false)]);
            assert_eq!(a.canvas.balloon, Some(id), "width {width}: still selected");
            assert_eq!(a.session.sel, caret, "width {width}: the document's caret didn't move");
            assert_eq!(a.canvas.scroll_offset, scroll, "width {width}: nothing scrolled");
            // A click on the page text leaves the card, and the comment goes away with the caret.
            let body = egui::pos2(page.left() + 120.0, page.top() + 40.0 + 150.0);
            frame(&mut a, vec![egui::Event::PointerMoved(body)]);
            frame(&mut a, vec![btn(body, true)]);
            frame(&mut a, vec![btn(body, false)]);
            for _ in 0..5 {
                frame(&mut a, Vec::new());
            }
            assert_eq!(a.canvas.balloon, None, "width {width}: a click on the page leaves the card");
        }
    }

    /// #1019: a double-click in the header area returned before the release ended the drag, so
    /// a later press held on the ribbon edge-scrolled the document up.
    #[test]
    fn a_press_on_the_ribbon_after_a_header_double_click_does_not_scroll() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("insert.editHeader", json!({})).unwrap();
        a.run("text.insert", json!({"text": "Header"})).unwrap();
        a.run("insert.closeHeader", json!({})).unwrap();
        a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        a.run("caret.docStart", json!({})).unwrap();
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let (Some(vp), Some(page)) = (a.canvas.viewport, a.canvas.page_rects.first().copied()) else { panic!("the canvas laid out") };
        let header = egui::pos2(page.left() + 150.0, page.top() + 70.0);
        assert!(vp.contains(header));
        frame(&mut a, vec![egui::Event::PointerMoved(header)]);
        for _ in 0..2 {
            frame(&mut a, vec![btn(header, true)]);
            frame(&mut a, vec![btn(header, false)]);
        }
        frame(&mut a, Vec::new());
        assert!(!a.canvas.drag_selecting(), "the double-click's release ended the drag");
        a.run("insert.closeHeader", json!({})).unwrap();
        a.run("caret.docEnd", json!({})).unwrap();
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let ribbon = egui::pos2(vp.center().x, vp.min.y - 40.0);
        let before = a.canvas.scroll_offset;
        frame(&mut a, vec![egui::Event::PointerMoved(ribbon), btn(ribbon, true)]);
        for _ in 0..30 {
            frame(&mut a, vec![egui::Event::PointerMoved(ribbon)]);
        }
        assert_eq!(a.canvas.scroll_offset, before, "holding a press on the ribbon leaves the document where it is");
        frame(&mut a, vec![btn(ribbon, false)]);
    }

    /// #3331: Esc out of a header on a later page scrolled back to page 1.
    #[test]
    fn escape_from_the_header_stays_on_the_page() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 600.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let before = (a.session.sel.clone(), a.canvas.scroll_offset);
        assert!(before.1.y > 1000.0, "the caret's last page is in view");
        a.run("insert.editHeader", json!({})).unwrap();
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&mut a, vec![esc]);
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.session.sel, before.0, "Esc left the header for where the caret was");
        // Entering an empty header scrolls to it (#3201) and Esc scrolls back to the caret, which
        // can land half a pixel from where the view started.
        let drift = a.canvas.scroll_offset - before.1;
        assert!(drift.x.abs() <= 1.0 && drift.y.abs() <= 1.0, "the view stays on that page: {:?} vs {:?}", a.canvas.scroll_offset, before.1);
    }

    /// While editing the header, a single click in the footer edits the footer there (and one in
    /// the next page's header goes back to the header) without scrolling. The click used to be
    /// hit-tested against the header, so the caret stayed in it and the view scrolled back up.
    #[test]
    fn a_click_in_the_footer_while_editing_the_header_moves_there() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("insert.header", json!({"text": "Head"})).unwrap();
        a.run("insert.footer", json!({"text": "Foot"})).unwrap();
        a.run("insert.closeHeader", json!({})).unwrap();
        a.run("text.insert", json!({"text": "lorem ipsum dolor sit amet consectetur ".repeat(1200)})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        a.run("caret.docStart", json!({})).unwrap();
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        a.run("insert.editHeader", json!({"page": 0})).unwrap();
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let page = a.session.layout().pages[0].clone();
        let (header, footer) = (page.header_story.expect("a header"), page.footer_story.expect("a footer"));
        assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(header));
        let (x, foot_y) = (page.body.x + 20.0, (page.body.bottom() + page.h) / 2.0);
        // Scroll down to page 1's footer a wheel notch at a time: page 1's header goes off screen.
        let vp = a.canvas.viewport.expect("the canvas laid out");
        for _ in 0..100 {
            let at = canvas::page_to_screen(&mut a, 0, x, foot_y).expect("footer laid out");
            if at.y < vp.center().y {
                break;
            }
            let wheel = egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
                source: egui::MouseWheelSource::Unknown,
            };
            frame(&mut a, vec![egui::Event::PointerMoved(vp.center()), wheel]);
            for _ in 0..30 {
                frame(&mut a, Vec::new());
            }
        }
        let at = canvas::page_to_screen(&mut a, 0, x, foot_y).expect("footer laid out");
        let head_at = canvas::page_to_screen(&mut a, 0, x, page.body.y / 2.0).expect("header laid out");
        assert!(vp.shrink(100.0).contains(at) && !vp.contains(head_at), "the footer is well on screen, the header isn't: {vp:?} {at:?} {head_at:?}");
        let before = a.canvas.scroll_offset;
        frame(&mut a, vec![egui::Event::PointerMoved(at)]);
        frame(&mut a, vec![btn(at, true)]);
        frame(&mut a, vec![btn(at, false)]);
        for _ in 0..40 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(footer), "editing the footer");
        assert_eq!(a.session.page_hint, 0, "on the clicked page");
        assert!((a.canvas.scroll_offset.y - before.y).abs() < 1.0, "the view stays put: {before:?} -> {:?}", a.canvas.scroll_offset);
        // The next page's header, just below: back in the header, still without scrolling.
        let next = a.session.layout().pages[1].clone();
        let at = canvas::page_to_screen(&mut a, 1, x, next.body.y / 2.0).expect("header laid out");
        assert!(vp.contains(at), "the next page's header is on screen");
        frame(&mut a, vec![egui::Event::PointerMoved(at)]);
        frame(&mut a, vec![btn(at, true)]);
        frame(&mut a, vec![btn(at, false)]);
        for _ in 0..40 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(header), "editing the header again");
        assert_eq!(a.session.page_hint, 1, "on the clicked page");
        assert!((a.canvas.scroll_offset.y - before.y).abs() < 1.0, "the view stays put: {before:?} -> {:?}", a.canvas.scroll_offset);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Part(footer)), "Foot");
    }

    /// Runs one 1280 by 800 frame through the raw input hook, logic and UI, dropping texture deltas.
    fn control_frame(a: &mut WordApp, ctx: &egui::Context, focused: bool) {
        let mut raw = egui::RawInput {
            focused,
            time: Some(ctx.input(|i| i.time) + 1.0 / 60.0),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            ..Default::default()
        };
        a.raw_input_hook(ctx, &mut raw);
        ctx.run_ui(raw, |ui| {
            a.logic(ui.ctx());
            a.ui(ui);
        })
        .drop_without_applying_deltas();
    }

    fn control_input(a: &mut WordApp, ctx: &egui::Context, method: &str, params: Value) {
        let (req, _reply) = ControlRequest::new(method, params);
        let control::Outcome::Done(v) = control::handle(a, ctx, &req) else { panic!("input is queued") };
        assert_eq!(v["result"]["queued"], true);
    }

    /// #2420: an inactive native window still delivers control input to its focused widget.
    #[test]
    fn control_keyboard_reaches_the_background_document_once() {
        for focused in [true, false] {
            let ctx = egui::Context::default();
            let mut a = app();
            a.run("document.setText", json!({"text": "Seed"})).unwrap();
            for _ in 0..4 {
                control_frame(&mut a, &ctx, true);
            }
            assert!(a.canvas.focused);
            for _ in 0..2 {
                control_frame(&mut a, &ctx, focused);
            }
            a.run("select.all", json!({})).unwrap();
            // Two queued requests are delivered in order across the key's release boundary.
            control_input(&mut a, &ctx, "ui.key", json!({"key": "b", "cmd": true}));
            control_input(&mut a, &ctx, "ui.text", json!({"text": "Written"}));
            control_frame(&mut a, &ctx, focused);
            assert_eq!(a.session.doc.para_at(&a.session.sel.focus).unwrap().props_of_char(0).bold, Some(true), "focused={focused}: Mod+B applied");
            assert_eq!(body_text(&a), "Seed", "text waits behind the key's release");
            control_frame(&mut a, &ctx, focused);
            assert_eq!(body_text(&a), "Written", "focused={focused}: text applied");
            assert!(a.synthetic.is_empty());
            let rev = a.session.rev();
            for _ in 0..3 {
                control_frame(&mut a, &ctx, true);
            }
            assert_eq!(a.session.rev(), rev, "foregrounding never replays an input");
            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(body_text(&a), "Seed", "one Undo reverses the one text input");
        }
    }

    /// Synthetic control text and keys target the focused background widget (the Find field), and
    /// Escape closes it. Closing a dirty document offers Save Changes, and Escape cancels without
    /// losing the document.
    #[test]
    fn control_keyboard_keeps_the_background_widget_target() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("document.setText", json!({"text": "Seed"})).unwrap();
        for _ in 0..4 {
            control_frame(&mut a, &ctx, true);
        }
        a.run("edit.advancedFind", json!({})).unwrap();
        for _ in 0..2 {
            control_frame(&mut a, &ctx, true);
        }
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique("find_what"))));
        for _ in 0..2 {
            control_frame(&mut a, &ctx, false);
        }
        control_input(&mut a, &ctx, "ui.text", json!({"text": "Needle"}));
        control_frame(&mut a, &ctx, false);
        assert!(matches!(&a.dialog, Some(dialogs::Dialog::Find { query, .. }) if query == "Needle"));
        control_input(&mut a, &ctx, "ui.key", json!({"key": "Backspace"}));
        control_frame(&mut a, &ctx, false);
        assert!(matches!(&a.dialog, Some(dialogs::Dialog::Find { query, .. }) if query == "Needl"));
        assert_eq!(body_text(&a), "Seed", "the Find field, not the document, owns the input");
        control_input(&mut a, &ctx, "ui.key", json!({"key": "Escape"}));
        control_frame(&mut a, &ctx, false);
        assert!(a.dialog.is_none());
        a.run("file.close", json!({})).unwrap();
        for focused in [true, true, false, false] {
            control_frame(&mut a, &ctx, focused);
        }
        assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveChanges { .. })));
        control_input(&mut a, &ctx, "ui.key", json!({"key": "Escape"}));
        control_frame(&mut a, &ctx, false);
        assert!(a.dialog.is_none());
        assert!(a.session.dirty && a.has_document() && !a.quit_requested);
        assert_eq!(body_text(&a), "Seed", "Cancel preserves the unsaved document");
    }

    /// Control input respects read-only protection, and a frame without control keys leaves the native
    /// focus state alone.
    #[test]
    fn control_keyboard_preserves_native_focus_and_document_protection() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("document.setText", json!({"text": "Seed"})).unwrap();
        for _ in 0..4 {
            control_frame(&mut a, &ctx, true);
        }
        a.run("select.all", json!({})).unwrap();
        a.run("review.restrict", json!({"mode": "readOnly"})).unwrap();
        let doc = a.session.doc.clone();
        for _ in 0..2 {
            control_frame(&mut a, &ctx, false);
        }
        control_input(&mut a, &ctx, "ui.key", json!({"key": "b", "cmd": true}));
        control_input(&mut a, &ctx, "ui.text", json!({"text": "Refused"}));
        for _ in 0..3 {
            control_frame(&mut a, &ctx, false);
        }
        assert_eq!(a.session.doc, doc, "control input respects the same protection as real keys");
        assert!(a.synthetic.is_empty());
        let mut native = egui::RawInput { focused: false, ..Default::default() };
        a.raw_input_hook(&ctx, &mut native);
        assert!(!native.focused, "a frame without control keys keeps the native focus state");
        assert!(!a.canvas.focused);
    }

    /// #2103: double-clicking the page number in a later page's footer opens the footer there;
    /// the view stays on that page instead of jumping to the top of the document.
    #[test]
    fn double_clicking_a_later_pages_footer_does_not_scroll_to_the_top() {
        let ctx = egui::Context::default();
        let mut a = app();
        // A header too, so opening the header instead of the footer can't pass.
        a.run("insert.header", json!({"text": "Head"})).unwrap();
        a.run("insert.closeHeader", json!({})).unwrap();
        a.run("insert.pageNumber", json!({"position": "bottom"})).unwrap();
        for _ in 0..4 {
            a.run("text.insert", json!({"text": "Some body text on this page."})).unwrap();
            a.run("insert.pageBreak", json!({})).unwrap();
        }
        a.run("text.insert", json!({"text": "The last page."})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { events, time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let last = a.session.layout().pages.len() - 1;
        assert!(last >= 3, "several pages");
        let page = a.session.layout().pages[last].clone();
        let (x, y) = (page.body.x + page.body.w / 2.0, (page.body.bottom() + page.h) / 2.0);
        // Scroll the reader to the last page's footer, as with the wheel.
        let at = canvas::page_to_screen(&mut a, last, x, y).expect("footer laid out");
        let vp = a.canvas.viewport.expect("the canvas laid out");
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, vp.center().y - at.y),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&mut a, vec![egui::Event::PointerMoved(vp.center()), wheel]);
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let at = canvas::page_to_screen(&mut a, last, x, y).expect("footer laid out");
        assert!(vp.contains(at), "the last page's footer is on screen");
        let before = a.canvas.scroll_offset;
        assert!(before.y > 1000.0, "scrolled down to the last page: {before:?}");
        frame(&mut a, vec![egui::Event::PointerMoved(at)]);
        // A single click there leaves the caret and the view alone (it used to scroll up to the
        // body text above).
        let caret = a.session.sel.clone();
        frame(&mut a, vec![btn(at, true)]);
        // Already at the press, before the release can decide anything.
        assert_eq!(a.session.sel, caret, "pressing in the footer area doesn't move the caret");
        assert!((a.canvas.scroll_offset.y - before.y).abs() < 1.0, "pressing doesn't scroll: {before:?} -> {:?}", a.canvas.scroll_offset);
        frame(&mut a, vec![btn(at, false)]);
        for _ in 0..40 {
            frame(&mut a, Vec::new());
        }
        assert_eq!(a.session.sel, caret, "a single click in the footer area doesn't move the caret");
        assert!((a.canvas.scroll_offset.y - before.y).abs() < 1.0, "a single click doesn't scroll: {before:?} -> {:?}", a.canvas.scroll_offset);
        for _ in 0..2 {
            frame(&mut a, vec![btn(at, true)]);
            frame(&mut a, vec![btn(at, false)]);
        }
        for _ in 0..30 {
            frame(&mut a, Vec::new());
        }
        let footer = page.footer_story.expect("the clicked page has a footer");
        assert!(page.header_story.is_some_and(|h| h != footer), "the page has a separate header: {:?}", page.header_story);
        assert_eq!(a.session.sel.focus.story, wordcraft_doc::StoryRef::Part(footer), "editing the clicked page's footer");
        assert_eq!(a.session.page_hint, last, "on the clicked page");
        assert!((a.canvas.scroll_offset.y - before.y).abs() < 1.0, "the view stays put: {before:?} -> {:?}", a.canvas.scroll_offset);
    }

    /// #3734: `ui.resize` refuses sizes that aren't positive and clamps the rest to a window the
    /// GPU can draw, replying with the size it asked for.
    #[test]
    fn control_resize_rejects_bad_sizes_and_clamps_huge_ones() {
        let ctx = egui::Context::default();
        let mut a = app();
        let mut resize = |params: Value| {
            let (req, _reply) = ControlRequest::new("ui.resize", params);
            let mut reply = Value::Null;
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let control::Outcome::Done(v) = control::handle(&mut a, ui.ctx(), &req) else { panic!("ui.resize replies at once") };
                reply = v;
            });
            let sent: Vec<egui::Vec2> = out
                .viewport_output
                .values()
                .flat_map(|v| v.commands.iter())
                .filter_map(|c| if let egui::ViewportCommand::InnerSize(s) = c { Some(*s) } else { None })
                .collect();
            out.drop_without_applying_deltas();
            (reply, sent)
        };
        // 1e300 is past f32: infinite once read, so refused too.
        for bad in [
            json!({"width": -5, "height": 600}),
            json!({"width": 0, "height": 0}),
            json!({"width": 1200, "height": -1}),
            json!({"width": 1e300, "height": 600}),
        ] {
            let (v, sent) = resize(bad.clone());
            assert_eq!(v["ok"], false, "{bad} is refused: {v}");
            assert!(sent.is_empty(), "{bad} sends no size: {sent:?}");
        }
        let max = ctx.input(|i| i.max_texture_side) as f32 / ctx.pixels_per_point();
        let (v, sent) = resize(json!({"width": 1e9, "height": 600}));
        assert_eq!(sent, [egui::vec2(max, 600.0)], "{v}");
        assert_eq!(v["result"], json!({"width": max, "height": 600.0}));
        let (v, sent) = resize(json!({"width": 10, "height": 10}));
        assert_eq!(sent, [window_geometry::MIN_SIZE], "{v}");
        let (v, sent) = resize(json!({"width": 1200, "height": 800}));
        assert_eq!(sent, [egui::vec2(1200.0, 800.0)]);
        assert_eq!(v["result"], json!({"width": 1200.0, "height": 800.0}));
    }

    /// The control channel's `ui.wheel` scrolls the canvas like a mouse wheel over it.
    #[test]
    fn control_wheel_scrolls_the_canvas() {
        let ctx = egui::Context::default();
        let mut a = app();
        for _ in 0..3 {
            a.run("insert.pageBreak", json!({})).unwrap();
        }
        a.run("caret.docStart", json!({})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp| {
            t += 1.0 / 60.0;
            let mut input = egui::RawInput { time: Some(t), screen_rect: Some(screen), ..Default::default() };
            a.raw_input_hook(&ctx, &mut input);
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a);
        }
        let top = a.canvas.page_rects[0].top();
        let at = a.canvas.canvas_rect.unwrap().center();
        let (req, _reply) = ControlRequest::new("ui.wheel", json!({"x": at.x, "y": at.y, "dy": -400.0}));
        control::handle(&mut a, &ctx, &req);
        for _ in 0..30 {
            frame(&mut a);
        }
        let moved = top - a.canvas.page_rects[0].top();
        assert!(moved > 100.0, "the first page moved up by {moved}");
        let (req, _reply) = ControlRequest::new("ui.wheel", json!({"x": at.x}));
        let control::Outcome::Done(v) = control::handle(&mut a, &ctx, &req) else { panic!("ui.wheel replies at once") };
        assert_eq!(v["ok"], false, "y is required: {v}");
    }

    /// Issue #123: zoomed out, pages sit side by side, and clicks map to the page under the pointer.
    #[test]
    fn zoomed_out_pages_sit_side_by_side_and_clicks_land_on_them() {
        let ctx = egui::Context::default();
        let mut a = app();
        for _ in 0..3 {
            a.run("insert.pageBreak", json!({})).unwrap();
        }
        a.run("text.insert", json!({"text": "Last page"})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut a);
        }
        let rects = a.canvas.page_rects.clone();
        assert_eq!(rects.len(), 4);
        assert!(rects[1].top() > rects[0].bottom(), "100%: one page per row");
        a.run("view.zoom", json!({"value": 30})).unwrap();
        for _ in 0..3 {
            frame(&mut a);
        }
        let rects = a.canvas.page_rects.clone();
        assert_eq!(a.canvas.cols, 4, "30%: all four pages fit across: {rects:?}");
        assert!((rects[3].top() - rects[0].top()).abs() < 1.0 && rects[3].left() > rects[2].right(), "{rects:?}");
        // A click inside the last page's text puts the caret on that page.
        let p = canvas::page_to_screen(&mut a, 3, 100.0, 80.0).unwrap();
        assert!(rects[3].contains(p), "{p:?} in {:?}", rects[3]);
        let pos = canvas::pos_from_screen(&mut a, p).unwrap();
        let caret = a.session.layout().caret_on(&pos, 3).unwrap();
        assert_eq!(caret.page, 3);
    }

    /// Issue #768: after One Page, zooming out (Zoom Out, the Zoom box, Ctrl + wheel) keeps one page per row.
    #[test]
    fn zoom_after_one_page_keeps_one_page_per_row() {
        let ctx = egui::Context::default();
        let mut a = app();
        for _ in 0..3 {
            a.run("insert.pageBreak", json!({})).unwrap();
        }
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut t = 0.0;
        let mut frame = |a: &mut WordApp| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { time: Some(t), screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            })
            .drop_without_applying_deltas();
        };
        a.run("view.onePage", json!({})).unwrap();
        for _ in 0..3 {
            frame(&mut a);
        }
        assert_eq!(a.canvas.cols, 1);
        a.run("view.zoom", json!({"value": 30})).unwrap();
        for _ in 0..3 {
            frame(&mut a);
        }
        assert_eq!(a.canvas.cols, 1, "30% after One Page: still one page per row");
        a.run("view.multiplePages", json!({})).unwrap();
        a.run("view.zoom", json!({"value": 30})).unwrap();
        for _ in 0..3 {
            frame(&mut a);
        }
        assert_eq!(a.canvas.cols, 4, "30% after Multiple Pages: all four pages fit across");
    }

    #[test]
    fn user_name_survives_restart() {
        let mut first = app();
        first.run("file.setAuthor", json!({"name": "Ada Lovelace"})).unwrap();
        let saved = serde_json::to_vec(&first.prefs()).unwrap();

        let mut second = app();
        second.apply_prefs(serde_json::from_slice(&saved).unwrap());
        assert_eq!(second.session.author, "Ada Lovelace");

        // A later rename is what gets saved next, not the name loaded at startup.
        second.run("file.setAuthor", json!({"name": "Grace Hopper"})).unwrap();
        assert_eq!(second.prefs().author, "Grace Hopper");
    }

    const UNSAVED: &str = "My unsaved novel chapter";

    /// An app whose new document has unsaved typing.
    fn typed() -> WordApp {
        let mut a = app();
        a.run("text.insert", json!({"text": UNSAVED})).unwrap();
        assert!(a.session.dirty);
        a
    }

    fn body_text(a: &WordApp) -> String {
        a.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
    }

    /// The reason the Save Changes prompt shows (a failed save), if that prompt is open.
    fn prompt_message(a: &WordApp) -> Option<String> {
        match &a.dialog {
            Some(dialogs::Dialog::SaveChanges { message, .. }) => Some(message.clone()),
            _ => None,
        }
    }

    fn prompt(a: &WordApp) -> Option<&'static str> {
        a.dialog.as_ref().map(|d| d.name())
    }

    /// Cut and Copy reach the system clipboard however they are run — the ribbon, a context menu,
    /// the native menu or Cmd+C/X — not only the key event, which used to be the sole writer.
    #[test]
    fn cut_and_copy_reach_the_system_clipboard() {
        let copied = |ctx: &egui::Context| -> Vec<String> {
            ctx.output(|o| {
                o.commands
                    .iter()
                    .filter_map(|c| match c {
                        egui::OutputCommand::CopyText(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect()
            })
        };
        let ctx = egui::Context::default();
        let mut a = typed();
        a.ctx = Some(ctx.clone());
        a.run("select.all", json!({})).unwrap();
        let text = a.session.selected_text();
        assert!(!text.is_empty());
        a.run("edit.cut", json!({})).unwrap();
        assert!(body_text(&a).is_empty(), "cut deleted the selection");
        assert!(copied(&ctx).contains(&text), "cut put {text:?} on the clipboard: {:?}", copied(&ctx));

        a.run("edit.undo", json!({})).unwrap();
        a.run("select.all", json!({})).unwrap();
        a.run("edit.copy", json!({})).unwrap();
        assert_eq!(body_text(&a), text, "copy kept the text");
        assert!(copied(&ctx).contains(&text), "copy put {text:?} on the clipboard: {:?}", copied(&ctx));
    }

    /// #2276: copying a selected picture, which has no text, still replaces what the system
    /// clipboard held, so Paste inserts the picture instead of the text copied before it.
    #[test]
    fn copying_a_picture_replaces_the_clipboards_old_text() {
        let last_copied = |ctx: &egui::Context| -> Option<String> {
            ctx.output(|o| {
                o.commands.iter().rev().find_map(|c| match c {
                    egui::OutputCommand::CopyText(t) => Some(t.clone()),
                    _ => None,
                })
            })
        };
        let ctx = egui::Context::default();
        let mut a = typed();
        a.ctx = Some(ctx.clone());
        a.run("select.all", json!({})).unwrap();
        a.run("edit.copy", json!({})).unwrap();
        assert_eq!(last_copied(&ctx).as_deref(), Some(UNSAVED));

        a.run("select.collapse", json!({"end": true})).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(&png_bytes([200, 30, 30, 255]));
        a.run("insert.picture", json!({"data": data})).unwrap();
        assert!(crate::objects::selected(&a).is_some());
        a.run("edit.copy", json!({})).unwrap();
        assert_eq!(last_copied(&ctx).as_deref(), Some(""), "the old text is gone from the system clipboard");

        // The system clipboard now holds no text, so Paste finds no text to send: our copy.
        a.run("select.collapse", json!({"end": true})).unwrap();
        a.run("edit.paste", json!({})).unwrap();
        assert_eq!(object_count(&mut a), 2, "Paste duplicated the picture");
        assert_eq!(body_text(&a).matches(UNSAVED).count(), 1, "and didn't paste the old text");
    }

    /// #1265: every user entry point publishes Cut to the one clipboard sink — the ribbon's button
    /// and a real Cmd/Ctrl+X key event, not only the generic command dispatch the sink sits behind.
    #[test]
    fn every_cut_entry_point_publishes_to_the_clipboard() {
        let copied = |ctx: &egui::Context| -> Vec<String> {
            ctx.output(|o| {
                o.commands
                    .iter()
                    .filter_map(|c| match c {
                        egui::OutputCommand::CopyText(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect()
            })
        };

        // The ribbon's Cut button goes through `run_from_ribbon`, like every ribbon control.
        let ctx = egui::Context::default();
        let mut a = typed();
        a.ctx = Some(ctx.clone());
        a.run("select.all", json!({})).unwrap();
        let text = a.session.selected_text();
        crate::widgets::run_from_ribbon(&mut a, "edit.cut", json!({}));
        assert!(body_text(&a).is_empty(), "ribbon Cut deleted the selection");
        assert!(copied(&ctx).contains(&text), "ribbon Cut published {text:?}: {:?}", copied(&ctx));

        // A real Cut key event through the app's key handling.
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            typed(),
        );
        for _ in 0..6 {
            h.step();
        }
        h.state_mut().canvas.focused = true;
        h.state_mut().run("select.all", json!({})).unwrap();
        h.step();
        h.event(egui::Event::Cut);
        h.step();
        let via_key: Vec<String> = h
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(via_key.iter().any(|t| t.contains("My unsaved novel")), "key event Cut: {via_key:?} body={:?}", body_text(h.state()));
    }

    /// A fresh scratch folder for one test.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A .docx on disk that the app didn't write.
    fn docx_on_disk(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
        let path = dir.join("contract.docx");
        let mut s = Session::new(wordcraft_doc::Document::from_text(text));
        s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        path
    }

    /// Mod+N (and the File › New tiles) open a new document beside unsaved work, asking nothing:
    /// nothing is replaced.
    #[test]
    fn new_opens_beside_unsaved_work() {
        let mut a = typed();
        let first = a.session.document_id();
        a.run("file.new", json!({"template": "letter"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(!body_text(&a).contains(UNSAVED));
        assert!(!a.session.dirty);
        assert!(!body_text(&a).trim().is_empty(), "the template the user picked was used");
        a.run("view.switchWindows", json!({"id": first})).unwrap();
        assert!(body_text(&a).contains(UNSAVED) && a.session.dirty, "the unsaved document is still open");
    }

    #[test]
    fn a_clean_document_is_replaced_without_asking() {
        let mut a = app();
        a.run("file.new", json!({"template": "letter"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(!body_text(&a).trim().is_empty());
    }

    /// #55: a password-protected document asks for its password (after Save Changes) instead of
    /// opening blank; scripts get the error instead of a dialog; Encrypt with Password asks for one.
    #[test]
    fn password_protected_documents_ask_for_the_password() {
        let dir = scratch("password");
        let path = dir.join("locked.docx");
        let mut s = Session::new(wordcraft_doc::Document::from_text("Locked away"));
        s.run("file.save", &json!({"path": path.to_string_lossy(), "password": "sesame"})).unwrap();

        let mut a = typed();
        assert!(a.execute("file.open", json!({"path": path.to_string_lossy()})).is_err(), "a script gets the error");
        assert_eq!(prompt(&a), None);
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(prompt(&a), Some("password"));
        assert!(body_text(&a).contains(UNSAVED), "nothing opens until the password fits");
        let state = serde_json::to_string(&a.dialog).unwrap();
        assert!(state.contains("locked.docx") && !state.contains("sesame"));

        let mut b = app();
        b.run("file.encrypt", json!({})).unwrap();
        assert_eq!(prompt(&b), Some("encryptPassword"));
        b.dialog = None;
        b.run("file.encrypt", json!({"password": null})).unwrap();
        assert_eq!(prompt(&b), None, "null removes the password without asking");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Open (after the file is picked), recent files and dropped files all go through `file.open`:
    /// the file opens beside unsaved work, asking nothing, and opening it again shows it.
    #[test]
    fn open_opens_beside_unsaved_work() {
        let dir = scratch("open");
        let path = docx_on_disk(&dir, "Signed contract");
        let mut a = typed();
        let first = a.session.document_id();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(body_text(&a).contains("Signed contract"));
        assert_eq!(a.session.path.as_deref(), Some(path.as_path()));
        let opened = a.session.document_id();
        a.run("view.switchWindows", json!({"id": first})).unwrap();
        assert!(body_text(&a).contains(UNSAVED) && a.session.dirty);
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(a.session.document_id(), opened, "an open file is shown, not opened twice");
        assert_eq!(a.session.documents().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Mod+O without a path only shows the file picker, so it doesn't ask yet.
    #[test]
    fn open_without_a_path_only_shows_the_picker() {
        let mut a = typed();
        a.run("file.open", json!({})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(body_text(&a).contains(UNSAVED));
    }

    /// Mod+W ran `file.close`, which quit at once; then it asked first, but still quit the whole
    /// app. #1712: as in Word, it closes the document and the window stays, showing no page, until
    /// File › New or Open.
    #[test]
    fn close_asks_then_closes_the_document_and_keeps_the_window() {
        let mut a = typed();
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(a.has_document() && body_text(&a).contains(UNSAVED), "Cancel keeps the document");

        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(!a.quit_requested, "the window stays");
        assert!(!a.has_document() && !a.session.dirty && !a.ui.backstage, "no document, no page");
        assert!(a.close_requested(), "closing the empty window asks nothing");
        a.run("file.close", json!({})).unwrap();
        assert!(!a.quit_requested && !a.has_document(), "nothing left to close");
        a.run("file.new", json!({})).unwrap();
        assert!(a.has_document(), "File › New goes on from there");

        // A clean document closes straight away.
        let mut clean = app();
        clean.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&clean), None);
        assert!(!clean.quit_requested && !clean.has_document());
    }

    /// A direct programmatic `file.close` (no prompt, no control channel) never discards unsaved
    /// changes either: it is refused unless `force` says so, as over the control channel.
    #[test]
    fn a_direct_close_keeps_unsaved_changes_unless_forced() {
        let mut a = typed();
        for params in [json!({}), json!({"quit": true}), json!({"force": "yes"})] {
            let e = a.execute("file.close", params.clone()).unwrap_err();
            assert!(e.contains("unsaved") && e.contains("force"), "{params}: {e}");
            assert!(a.has_document() && !a.quit_requested && body_text(&a).contains(UNSAVED), "{params}");
        }
        a.execute("file.close", json!({"force": true})).unwrap();
        assert!(!a.has_document() && !a.quit_requested);
    }

    /// The window's close button quit at once; now it asks (the host cancels the close on false).
    #[test]
    fn window_close_asks_first() {
        assert!(app().close_requested(), "a clean document closes");

        let mut a = typed();
        assert!(!a.close_requested());
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(!a.close_requested(), "asked again on the next click");

        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(a.quit_requested);
        assert!(a.close_requested(), "the close that follows goes ahead");
        assert!(body_text(&a).contains(UNSAVED), "nothing was replaced on the way out");
    }

    /// #1445: the window's X on an edited document with a file → Save writes the file and lets
    /// the window close.
    #[test]
    fn window_close_save_writes_the_file_and_quits() {
        let dir = scratch("close-save");
        let path = docx_on_disk(&dir, "Draft");
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        assert!(!a.close_requested());
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        let on_disk = wordcraft_engine::io::open_path(&path).unwrap().plain_text(wordcraft_doc::StoryRef::Body);
        assert!(on_disk.contains("Revised Draft"), "the edit is on disk: {on_disk:?}");
        assert!(a.quit_requested, "the app may close");
        assert!(a.close_requested(), "the close that follows goes ahead");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #2376: a text file opened, edited and saved still asked to save on close, and Save in that
    /// prompt wrote the file but left the window open; only Don't Save closed it.
    #[test]
    fn a_text_file_saved_over_itself_closes() {
        let dir = scratch("close-txt");
        let path = dir.join("note.txt");
        std::fs::write(&path, b"").unwrap();

        // Saved with Ctrl+S, then closed: nothing to ask.
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        a.run("file.save", json!({})).unwrap();
        assert!(a.close_requested(), "a saved text file closes");
        assert_eq!(prompt(&a), None);

        // Edited again, then closed: Save in the prompt writes the file and lets the window close.
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        assert!(!a.close_requested());
        assert_eq!(prompt(&a), Some("saveChanges"));
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["done"], true, "{r}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("Revised Hello"), "the edit is on disk");
        assert!(a.quit_requested, "the app may close");
        assert!(a.close_requested(), "the close that follows goes ahead");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #1445: when Save in the close prompt can't write the file, the prompt's answer is an error
    /// that says so plainly, the status bar shows it, and the document stays open, still edited.
    #[test]
    fn window_close_save_that_fails_says_so_and_keeps_the_document() {
        let dir = scratch("close-save-fails");
        let folder = dir.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let path = docx_on_disk(&folder, "Draft");
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        // The document's folder is gone and a file has its name: nothing can be written there.
        std::fs::remove_dir_all(&folder).unwrap();
        std::fs::write(&folder, b"not a folder").unwrap();
        assert!(!a.close_requested());
        a.status_msg = None;
        let err = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap_err();
        assert!(err.contains("Couldn't save contract.docx") && err.contains("still open"), "{err}");
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| *m == err), "the status bar says why: {:?}", a.status_msg);
        assert!(!a.quit_requested, "nothing closes");
        assert!(a.session.dirty && body_text(&a).contains("Revised Draft"), "nothing was discarded");
        assert_eq!(a.session.path.as_deref(), Some(path.as_path()));
        // The prompt comes back with the reason in it: Save again, Don't Save or Cancel.
        assert_eq!(prompt(&a), Some("saveChanges"), "the prompt is open again");
        assert_eq!(prompt_message(&a).as_deref(), Some(err.as_str()), "the prompt says why");
        assert!(a.run("ui.saveChanges", json!({"answer": "save"})).is_err(), "Save again fails again");
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(!a.close_requested(), "the next close asks again");
        assert_eq!(prompt_message(&a).as_deref(), Some(""), "a fresh prompt, no old reason");
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(a.quit_requested, "Don't Save still closes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #1445: a new document → close → Save → Save As to a place that can't be written: the
    /// same plain error, the prompt back with the reason, nothing closed or discarded.
    #[test]
    fn window_close_save_as_that_fails_says_so_and_keeps_the_document() {
        let dir = scratch("close-save-as-fails");
        let blocker = dir.join("folder");
        std::fs::write(&blocker, b"not a folder").unwrap();
        let picked = blocker.join("novel.docx").to_string_lossy().to_string();
        let mut a = typed();
        a.services.pick_save = Some(Box::new(move |_| Some(picked.clone())));
        assert!(!a.close_requested());
        a.status_msg = None;
        let err = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap_err();
        assert!(err.contains("Couldn't save") && err.contains("still open"), "{err}");
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| *m == err), "the status bar says why: {:?}", a.status_msg);
        assert!(!a.quit_requested, "nothing closes");
        assert!(a.session.dirty && body_text(&a).contains(UNSAVED), "nothing was discarded");
        assert_eq!(a.session.path, None);
        assert_eq!(prompt(&a), Some("saveChanges"), "the prompt is open again");
        assert_eq!(prompt_message(&a).as_deref(), Some(err.as_str()), "the prompt says why");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #1445: the prompt that comes back after a failed save shows the reason on screen.
    #[test]
    fn save_prompt_shows_why_the_save_failed() {
        use egui_kittest::kittest::Queryable;
        let mut a = typed();
        a.run("file.close", json!({})).unwrap();
        let reason = "Couldn't save Draft, so it is still open and nothing was lost: disk full";
        if let Some(dialogs::Dialog::SaveChanges { message, .. }) = &mut a.dialog {
            *message = reason.into();
        }
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().build_ui_state(
            move |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                if fonts {
                    dialogs::show(app, &ctx);
                } else {
                    theme::install_fonts_for(&ctx, false);
                    fonts = true;
                }
            },
            a,
        );
        for _ in 0..3 {
            h.step();
        }
        assert!(h.query_by_label(reason).is_some(), "the reason is in the prompt");
        assert!(h.query_by_label("Save").is_some(), "Save again is offered");
    }

    /// Enter activates the focused prompt button; it means Save only when no button has focus.
    /// (Enter on a focused Cancel or Don't Save used to be overridden by a blanket Enter → Save.)
    #[test]
    fn enter_answers_with_the_focused_button() {
        use egui_kittest::kittest::Queryable;
        for (focus, closes, saves) in [(Some("Cancel"), false, false), (Some("Don't Save"), true, false), (None, true, true)] {
            let dir = scratch(&format!("enter-{}", focus.map_or(0, str::len)));
            let path = docx_on_disk(&dir, "Draft");
            let before = std::fs::read(&path).unwrap();
            let mut a = app();
            a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
            a.run("text.insert", json!({"text": "Revised "})).unwrap();
            a.run("file.close", json!({})).unwrap();
            assert_eq!(prompt(&a), Some("saveChanges"));
            // The dialog's theme fonts are installed on the first frame and usable from the next.
            let mut fonts = false;
            let mut h = egui_kittest::Harness::builder().build_ui_state(
                move |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    if fonts {
                        dialogs::show(app, &ctx);
                    } else {
                        theme::install_fonts_for(&ctx, false);
                        fonts = true;
                    }
                },
                a,
            );
            for _ in 0..3 {
                h.step();
            }
            if let Some(label) = focus {
                h.get_by_label(label).focus();
                h.step();
                assert!(h.get_by_label(label).is_focused(), "{label} has focus");
            }
            h.key_press(egui::Key::Enter);
            for _ in 0..3 {
                h.step();
            }
            let a = h.state();
            assert_eq!(prompt(a), None, "{focus:?}: answered");
            assert_eq!(!a.has_document(), closes, "{focus:?}: closed");
            assert_eq!(std::fs::read(&path).unwrap() != before, saves, "{focus:?}: file written");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// #861: the save prompt answers from the keyboard alone. Save has focus when it opens, and
    /// Left/Right move the focus between Save, Don't Save and Cancel (stopping at the ends).
    #[test]
    fn arrow_keys_choose_the_save_prompt_answer() {
        use egui_kittest::kittest::Queryable;
        for (arrows, focused, closes, saves) in [
            (vec![egui::Key::ArrowLeft], "Save", true, true),
            (vec![egui::Key::ArrowRight], "Don't Save", true, false),
            (vec![egui::Key::ArrowRight, egui::Key::ArrowRight, egui::Key::ArrowRight], "Cancel", false, false),
            (vec![egui::Key::ArrowRight, egui::Key::ArrowRight, egui::Key::ArrowLeft], "Don't Save", true, false),
        ] {
            let dir = scratch(&format!("arrows-{}-{}", arrows.len(), focused.len()));
            let path = docx_on_disk(&dir, "Draft");
            let before = std::fs::read(&path).unwrap();
            let mut a = app();
            a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
            a.run("text.insert", json!({"text": "Revised "})).unwrap();
            a.run("file.close", json!({})).unwrap();
            assert_eq!(prompt(&a), Some("saveChanges"));
            let mut fonts = false;
            let mut h = egui_kittest::Harness::builder().build_ui_state(
                move |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    if fonts {
                        dialogs::show(app, &ctx);
                    } else {
                        theme::install_fonts_for(&ctx, false);
                        fonts = true;
                    }
                },
                a,
            );
            for _ in 0..3 {
                h.step();
            }
            assert!(h.get_by_label("Save").is_focused(), "Save has focus when the prompt opens");
            for key in &arrows {
                h.key_press(*key);
                h.step();
            }
            assert!(h.get_by_label(focused).is_focused(), "{arrows:?}: {focused} has focus");
            h.key_press(egui::Key::Enter);
            for _ in 0..3 {
                h.step();
            }
            let a = h.state();
            assert_eq!(prompt(a), None, "{arrows:?}: answered");
            assert_eq!(!a.has_document(), closes, "{arrows:?}: closed");
            assert_eq!(std::fs::read(&path).unwrap() != before, saves, "{arrows:?}: file written");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// #1597: the prompt takes the keyboard from the page. The canvas, focused while typing, keeps
    /// Tab and the arrows for itself; Save still has focus when the prompt opens over it, and the
    /// arrows move between the prompt's buttons instead of the caret.
    #[test]
    fn save_prompt_takes_focus_from_the_page() {
        use egui_kittest::kittest::Queryable;
        let dir = scratch("prompt-over-page");
        let path = docx_on_disk(&dir, "Draft");
        let before = std::fs::read(&path).unwrap();
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        a.canvas.want_focus = true;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        assert!(h.state().canvas.focused, "the page has keyboard focus");
        h.state_mut().run("file.close", json!({})).unwrap();
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(prompt(h.state()), Some("saveChanges"));
        let focused = |h: &egui_kittest::Harness<WordApp>, label: &str| h.query_all_by_label(label).any(|n| n.is_focused());
        assert!(focused(&h, "Save"), "Save has focus when the prompt opens");
        let caret = h.state().session.sel.focus.clone();
        h.key_press(egui::Key::ArrowRight);
        h.step();
        assert!(focused(&h, "Don't Save"), "the arrow moved to Don't Save");
        assert_eq!(h.state().session.sel.focus, caret, "the caret stayed where it was");
        h.key_press(egui::Key::Enter);
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(prompt(h.state()), None, "answered");
        assert!(!h.state().has_document() && !h.state().quit_requested, "Don't Save closes the document, not the window (#1712)");
        assert_eq!(std::fs::read(&path).unwrap(), before, "nothing was saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #3172: the font name box and the font size box say what they are, and the arrows beside
    /// them say whose options they open: without a name a screen reader has only "edit" or
    /// "button" to say.
    #[test]
    fn the_ribbon_boxes_name_themselves_for_a_screen_reader() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("file.new", json!({"template": "sample"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(6);
        for (role, name) in [
            (Role::TextInput, "Font"),
            (Role::TextInput, "Font Size"),
            (Role::Button, "Font options"),
            (Role::Button, "Font Size options"),
            (Role::Button, "Font Color options"),
        ] {
            assert!(h.query_by_role_and_label(role, name).is_some(), "nothing on the ribbon is called {name:?}");
        }
    }

    /// #1015: focusing Save is not enough: its dialog must expose the question and any failed
    /// save's explanation, so assistive technology has context for the three choices.
    #[test]
    fn save_prompt_exposes_its_context_to_accessibility() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{By, NodeT, Queryable};
        let dir = scratch("save-accessibility");
        let path = docx_on_disk(&dir, "Draft");
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        a.canvas.want_focus = true;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(6);
        assert!(h.state().canvas.focused);
        h.state_mut().run("file.close", json!({})).unwrap();
        h.run_steps(3);
        let save = h.query_all_by_role(Role::Button).find(|n| n.accesskit_node().label().as_deref() == Some("Save") && n.is_focused()).unwrap();
        let mut ancestor = save.parent();
        let mut roles = Vec::new();
        while let Some(node) = ancestor {
            roles.push(node.accesskit_node().role());
            ancestor = node.parent();
        }
        assert!(roles.contains(&Role::AlertDialog), "Save has no alert-dialog ancestor: {roles:?}");
        let dialog = h.get(By::new().predicate(|n| n.role() == Role::AlertDialog));
        let node = dialog.accesskit_node();
        assert!(node.label().is_some_and(|s| s.contains("contract.docx") && s.contains("save changes")));
        assert!(node.description().is_some_and(|s| s.contains("Your changes will be lost")));
        for label in ["Save", "Don't Save", "Cancel"] {
            assert!(dialog.query_by_role_and_label(Role::Button, label).is_some(), "missing {label}");
        }
        let reason = "Couldn't save contract.docx: disk full";
        if let Some(dialogs::Dialog::SaveChanges { message, .. }) = &mut h.state_mut().dialog {
            *message = reason.into();
        }
        h.run_steps(2);
        let dialog = h.get(By::new().predicate(|n| n.role() == Role::AlertDialog));
        assert!(dialog.accesskit_node().description().is_some_and(|s| s.contains(reason)));
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().dialog.is_none());
        assert!(h.state().canvas.focused && h.state().session.dirty);
        assert!(body_text(h.state()).contains("Revised Draft"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `ui.print` hands the document's PDF to the host's print hook, and is an error (with
    /// nothing printed) when the host has none.
    #[test]
    fn ui_print_needs_the_hosts_print_hook() {
        let mut a = typed();
        assert!(a.run("ui.print", json!({})).is_err(), "no print hook: an error");

        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = got.clone();
        a.services.print = Some(Box::new(move |bytes| {
            seen.borrow_mut().push(bytes.to_vec());
            Ok(())
        }));
        let r = a.run("ui.print", json!({})).unwrap();
        assert_eq!(r["printing"], true);
        assert_eq!(got.borrow().len(), 1, "printed once");
        assert!(got.borrow().first().is_some_and(|b| b.starts_with(b"%PDF")), "the hook got a PDF");
        assert!(a.session.dirty, "printing is not a save");

        a.services.print = Some(Box::new(|_| Err("blocked by the browser".into())));
        a.status_msg = None;
        let e = a.run("ui.print", json!({})).unwrap_err();
        assert!(e.contains("blocked by the browser"), "{e}");
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| m.contains("blocked by the browser")), "the status bar says why");
    }

    /// Make WordCraft the default… (#295) goes through the host's hook and reports the outcome in
    /// the status bar; without a hook (web, macOS) it's an error and nothing runs.
    #[test]
    fn make_default_app_needs_the_hosts_hook_and_reports_the_outcome() {
        let mut a = app();
        assert!(a.make_default_app().is_err(), "no hook: an error");

        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = calls.clone();
        a.services.make_default_app = Some(Box::new(move || {
            seen.set(seen.get() + 1);
            Ok(DefaultApp::MadeDefault)
        }));
        assert_eq!(a.make_default_app().unwrap()["default"], true);
        assert_eq!(calls.get(), 1);
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| m.contains("now the default")), "{:?}", a.status_msg);

        a.services.make_default_app = Some(Box::new(|| Ok(DefaultApp::SettingsOpened)));
        assert_eq!(a.make_default_app().unwrap()["settingsOpened"], true);
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| m.contains("Default apps")), "{:?}", a.status_msg);

        a.services.make_default_app = Some(Box::new(|| Err("xdg-mime isn't installed".into())));
        let e = a.make_default_app().unwrap_err();
        assert!(e.contains("xdg-mime isn't installed"), "{e}");
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| m.contains("xdg-mime isn't installed")), "the status bar says why");
        assert!(!a.session.dirty, "the document is untouched");
    }

    /// Changing the system's default apps needs the user's click: `ui.makeDefaultApp` through
    /// `run` (what the control channel, MCP and macros use) is refused and never reaches the hook.
    #[test]
    fn programmatic_calls_cannot_change_the_default_apps() {
        let mut a = app();
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = calls.clone();
        a.services.make_default_app = Some(Box::new(move || {
            seen.set(seen.get() + 1);
            Ok(DefaultApp::MadeDefault)
        }));
        let e = a.run("ui.makeDefaultApp", json!({})).unwrap_err();
        assert!(e.contains("click"), "{e}");
        assert_eq!(calls.get(), 0, "the hook never ran");
    }

    /// The print settings reach the hook: bad ones print nothing, `copies` repeats the pages.
    #[test]
    fn ui_print_honours_the_print_settings() {
        let mut a = typed();
        let sizes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = sizes.clone();
        a.services.print = Some(Box::new(move |bytes| {
            seen.borrow_mut().push(bytes.len());
            Ok(())
        }));
        assert!(a.run("ui.print", json!({"pages": "7-9"})).is_err(), "page 7 doesn't exist");
        assert!(a.run("ui.print", json!({"range": "nope"})).is_err());
        assert!(sizes.borrow().is_empty(), "nothing was sent");
        a.run("ui.print", json!({"range": "current", "copies": 3})).unwrap();
        assert_eq!(sizes.borrow().len(), 1);
    }

    /// The Print page shows the Print button only when the host can print, and the button
    /// prints through `ui.print`.
    #[test]
    fn the_print_button_shows_only_with_a_print_hook() {
        use egui_kittest::kittest::Queryable;
        const BLURB: &str = "Send the document straight to the system print dialog, or save a copy in another format below.";
        for hook in [false, true] {
            let printed = std::rc::Rc::new(std::cell::Cell::new(0));
            let mut a = app();
            if hook {
                let seen = printed.clone();
                a.services.print = Some(Box::new(move |_| {
                    seen.set(seen.get() + 1);
                    Ok(())
                }));
            }
            a.run("file.print", json!({})).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            );
            for _ in 0..4 {
                h.step();
            }
            assert!(h.state().ui.backstage && h.state().ui.backstage_page == "print", "the Print page is open");
            assert_eq!(h.query_by_label(BLURB).is_some(), hook, "hook {hook}: the Print button's blurb");
            assert!(h.query_by_label("PDF document (*.pdf)").is_some(), "hook {hook}: the page rendered its export choices");
            if hook {
                // The page's own Print button (the backstage list has a Print entry too): the last.
                let buttons: Vec<_> = h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button).label("Print")).collect();
                buttons.last().expect("a Print button").click();
                h.step();
                h.step();
                assert_eq!(printed.get(), 1, "the button printed");
            }
        }
    }

    /// #2958: the Print button leads the settings, above the printer and the settings it prints
    /// with (as in Word), and it wears the accent the app gives a dialog's primary button, so the
    /// page's one action is found first and reads as one rather than as another setting.
    #[test]
    fn the_print_button_leads_the_settings_and_is_the_primary_action() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{By, Queryable};
        const BLURB: &str = "Send the document straight to the system print dialog, or save a copy in another format below.";
        let mut a = app();
        a.services.print = Some(Box::new(|_| Ok(())));
        a.run("file.print", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..4 {
            h.step();
        }
        // The page's own Print button (the backstage list has a Print entry too): the last.
        let print = h.query_all(By::new().role(Role::Button).label("Print")).last().expect("a Print button").rect();
        let blurb = h.get_by_label(BLURB).rect();
        assert!(print.min.y < blurb.min.y, "the Print button leads the settings: {print:?} above {blurb:?}");
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        assert!(
            h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == print && r.fill == accent)),
            "the Print button is the primary (accent) button"
        );
    }

    /// #3210: a successful native print submission returns to the document, the same as Word —
    /// previously the Print page stayed up until the user clicked Back, even though the job had
    /// already gone to the spooler. A failed submission stays up instead, so the error (and the
    /// settings to fix it) are still visible.
    #[test]
    fn a_successful_native_print_returns_to_the_document() {
        use printing::Printer;
        for ok in [true, false] {
            let mut a = app();
            a.printing.printers = vec![Printer { name: "Test Printer".into(), label: String::new(), default: true }];
            a.printing.settings.printer = "Test Printer".into();
            a.services.print_native = Some(Box::new(move |_, _, _, _| {
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(if ok { Ok(String::new()) } else { Err("no paper".into()) });
                rx
            }));
            a.run("file.print", json!({})).unwrap();
            assert!(a.ui.backstage && a.ui.backstage_page == "print", "the Print page is open");
            a.canvas.want_focus = false;
            a.run("ui.print", json!({})).unwrap();
            let ctx = egui::Context::default();
            ctx.run_ui(egui::RawInput::default(), |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
            assert_eq!(a.ui.backstage, !ok, "ok {ok}: whether the Print page is still open");
            assert_eq!(a.canvas.want_focus, ok, "ok {ok}: the document should take keyboard focus back only once the Print page actually closes");
            if !ok {
                assert_eq!(a.ui.backstage_page, "print", "the Print page is still the one showing, not some other backstage page");
                assert!(
                    matches!(&a.printing.message, Some(printing::Feedback::Failed(error)) if error == "no paper"),
                    "the failure is still visible: {:?}",
                    a.printing.message
                );
            }
        }
    }

    /// #1156: the Print page's preview fills the pane beside the settings, zooms from its own
    /// bar, scrolls both ways once the page outgrows the pane, and One Page fits it again.
    #[test]
    fn the_print_preview_fills_the_pane_zooms_and_scrolls_both_ways() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{By, Queryable};
        let mut a = app();
        a.run("file.print", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let page = |h: &egui_kittest::Harness<'_, WordApp>| h.get(By::new().role(Role::Image)).rect();
        let fit = page(&h);
        assert!(fit.height() > 650.0, "the page fills the pane's height: {fit:?}");
        assert!(h.query(By::new().role(Role::Label).label("Page 1 of 1")).is_some(), "the page indicator stays");

        for _ in 0..5 {
            h.get_by_label("Zoom In").click();
            h.run_steps(2);
        }
        let zoomed = page(&h);
        assert!(zoomed.width() > fit.width() * 1.4 && zoomed.height() > 900.0, "zoomed in: {zoomed:?}");

        h.input_mut().events.push(egui::Event::PointerMoved(zoomed.center()));
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-120.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        });
        h.run_steps(20);
        let moved = page(&h);
        assert!(moved.min.x < zoomed.min.x - 50.0, "scrolls sideways: {zoomed:?} -> {moved:?}");
        assert!(moved.min.y < zoomed.min.y - 50.0, "scrolls down: {zoomed:?} -> {moved:?}");

        h.get_by_label("One Page").click();
        h.run_steps(4);
        let again = page(&h);
        assert!((again.height() - fit.height()).abs() < 1.0, "One Page fits the page again: {fit:?} vs {again:?}");
    }

    /// File › Options shows Make WordCraft the default… only when the host can do it (#295), and
    /// the button runs the host's hook.
    #[test]
    fn the_default_app_button_shows_only_with_the_hosts_hook() {
        use egui_kittest::kittest::Queryable;
        const LABEL: &str = "Make WordCraft the default for Word documents…";
        for hook in [false, true] {
            let calls = std::rc::Rc::new(std::cell::Cell::new(0));
            let mut a = app();
            if hook {
                let seen = calls.clone();
                a.services.make_default_app = Some(Box::new(move || {
                    seen.set(seen.get() + 1);
                    Ok(DefaultApp::MadeDefault)
                }));
            }
            a.run("ui.backstage", json!({"value": true, "page": "options"})).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            );
            h.run_steps(4);
            assert!(h.query_by_label("Show the community button in the title bar").is_some(), "hook {hook}: the Options page rendered");
            assert_eq!(h.query_by_label(LABEL).is_some(), hook, "hook {hook}: the button");
            if hook {
                h.get_by_label(LABEL).click();
                h.run_steps(2);
                assert_eq!(calls.get(), 1, "the button ran the hook");
            }
        }
    }

    /// A prompt about document A outlived an agent showing B instead, and Don't Save then ran
    /// the pending Close against B, throwing away B's edits under a question naming A.
    #[test]
    fn a_prompt_is_dropped_when_its_document_is_replaced() {
        let dir = scratch("stale");
        let path = docx_on_disk(&dir, "Agent document");
        let ctx = egui::Context::default();
        let mut a = typed();
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        let asked = a.dialog.clone();
        for command in [
            json!({"command": "file.open", "params": {"path": path.to_string_lossy()}}),
            json!({"command": "text.insert", "params": {"text": "Edited "}}),
        ] {
            let (req, _reply) = ControlRequest::new("engine.execute", command);
            control::handle(&mut a, &ctx, &req);
        }
        assert_eq!(prompt(&a), None, "the question was about a document that is gone");

        // An answer that arrives anyway (a click in that frame, an agent) doesn't touch B.
        a.dialog = asked;
        assert!(a.run("ui.saveChanges", json!({"answer": "dontSave"})).is_err());
        assert!(body_text(&a).contains("Edited Agent document"));
        assert_eq!(a.session.path.as_deref(), Some(path.as_path()));
        assert!(a.session.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The web shell's leave-page guard learned about unsaved changes only after `logic`, but
    /// typing lands in `ui`, so a tab closed right after the first keystroke left without asking.
    #[test]
    fn the_host_hears_about_edits_made_while_drawing() {
        let reported = std::rc::Rc::new(std::cell::Cell::new(None));
        let mut a = app();
        let seen = reported.clone();
        a.services.on_dirty = Some(Box::new(move |d| seen.set(Some(d))));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        assert!(h.state().canvas.focused, "the page has keyboard focus");
        assert_eq!(reported.get(), Some(false));
        h.event(egui::Event::Text("x".into()));
        h.step();
        assert!(h.state().session.dirty, "the keystroke reached the document");
        assert_eq!(reported.get(), Some(true), "the host was told before the frame ended");
    }

    #[test]
    fn paragraph_alignment_keeps_the_canvas_focused_for_typing() {
        use wordcraft_doc::Align;

        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            typed(),
        );
        for _ in 0..6 {
            h.step();
        }

        for (label, alignment, text) in
            [("Align Left", Align::Left, " left"), ("Center", Align::Center, " center"), ("Align Right", Align::Right, " right")]
        {
            h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
            h.step();

            assert!(h.state().canvas.focused, "{label}: the canvas keeps keyboard focus");
            let paragraph = h.state().session.doc.para_at(&h.state().session.sel.focus).expect("the caret remains in its paragraph");
            assert_eq!(h.state().session.doc.styles.resolve_para(&paragraph.props).align, alignment);

            h.event(egui::Event::Text(text.into()));
            h.step();
            assert!(body_text(h.state()).ends_with(text), "{label}: typing continues in the document");
        }

        // Every command button does the same, not only alignment: Bold, then type.
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Bold").click();
        h.step();
        assert!(h.state().canvas.focused, "Bold: the canvas keeps keyboard focus");
        h.event(egui::Event::Text(" bold".into()));
        h.step();
        assert!(body_text(h.state()).ends_with(" bold"), "Bold: typing continues in the document");
    }

    /// Types `text` after a click on `what` and checks it reached the document (#1093).
    fn type_after_click(h: &mut egui_kittest::Harness<'_, WordApp>, what: &str, text: &str) {
        h.step();
        h.step();
        h.event(egui::Event::Text(text.into()));
        h.step();
        assert!(h.state().canvas.focused, "{what}: the page keeps keyboard focus");
        assert!(body_text(h.state()).ends_with(text), "{what}: typing continues in the document");
    }

    /// #1093: clicks on ribbon and status-bar chrome that isn't a command button (gallery tiles,
    /// tab names, empty ribbon space, View check boxes, the status bar's view buttons and
    /// language menu) used to take keyboard focus from the page: the caret vanished and typing
    /// went nowhere until the page was clicked again.
    #[test]
    fn clicks_on_ribbon_and_status_bar_chrome_keep_typing_in_the_document() {
        use egui::accesskit::Role;

        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            typed(),
        );
        for _ in 0..6 {
            h.step();
        }
        assert!(h.state().canvas.focused, "the page starts with keyboard focus");

        // A Styles gallery tile.
        h.get_by_role_and_label(Role::Button, "Heading 1").click();
        type_after_click(&mut h, "Heading 1 tile", " gallery");
        let para = h.state().session.doc.para_at(&h.state().session.sel.focus).expect("caret paragraph");
        assert_eq!(para.props.style.as_deref(), Some("Heading1"), "the tile applied its style");

        // A ribbon tab name, then a View tab check box.
        h.get_by_role_and_label(Role::Button, "View").click();
        type_after_click(&mut h, "View tab", " tab");
        assert_eq!(h.state().ui.tab, "View");
        let ruler = h.state().session.view.ruler;
        h.get_by_role_and_label(Role::CheckBox, "Ruler").click();
        type_after_click(&mut h, "Ruler check box", " ruler");
        assert_ne!(h.state().session.view.ruler, ruler, "the check box toggled the ruler");

        // Empty space at the right end of the ribbon, under the tab strip.
        let tab = h.get_by_role_and_label(Role::Button, "View").rect();
        let blank = egui::pos2(1425.0, tab.max.y + 25.0);
        for pressed in [true, false] {
            h.hover_at(blank);
            h.event(egui::Event::PointerButton { pos: blank, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        }
        type_after_click(&mut h, "empty ribbon space", " blank");

        // The status bar's view buttons.
        h.get_by_role_and_label(Role::Button, "Web Layout").click();
        type_after_click(&mut h, "Web Layout", " web");
        assert_eq!(h.state().session.view.mode, wordcraft_layout::ViewMode::Web);
        h.get_by_role_and_label(Role::Button, "Print Layout").click();
        type_after_click(&mut h, "Print Layout", " print");
        assert_eq!(h.state().session.view.mode, wordcraft_layout::ViewMode::Print);

        // The language item opens its menu; a click elsewhere on the status bar cancels it.
        let lang_label = crate::ribbon::caret_language(h.state());
        let lang = h.get_by_label(&lang_label).rect();
        h.get_by_label(&lang_label).click();
        h.step();
        h.step();
        assert!(egui::Popup::is_any_open(&h.ctx), "the language menu opened");
        let elsewhere = egui::pos2(lang.max.x + 40.0, lang.center().y);
        for pressed in [true, false] {
            h.hover_at(elsewhere);
            h.event(egui::Event::PointerButton { pos: elsewhere, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        }
        h.step();
        assert!(!egui::Popup::is_any_open(&h.ctx), "the click closed the language menu");
        type_after_click(&mut h, "language menu cancelled", " lang");

        // Back on Home, a text field still takes the focus: the font size box.
        h.get_by_role_and_label(Role::Button, "Home").click();
        type_after_click(&mut h, "Home tab", " home");
        let before = body_text(h.state());
        let size_box = h
            .get_all_by_role(Role::TextInput)
            .find(|n| n.value().is_some_and(|v| v.trim().parse::<f64>().is_ok()))
            .expect("the font size box")
            .rect();
        for pressed in [true, false] {
            h.hover_at(size_box.center());
            h.event(egui::Event::PointerButton {
                pos: size_box.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        h.step();
        h.step();
        assert!(!h.state().canvas.focused, "the font size box took keyboard focus");
        h.event(egui::Event::Text("4".into()));
        h.step();
        assert!(!h.state().canvas.focused, "the font size box keeps focus while edited");
        assert_eq!(body_text(h.state()), before, "typing went into the font size box, not the page");
    }

    /// #2068: fonts applied from the font box or the mini toolbar (`format.font`) and from the
    /// Font dialog (`format.set`) join Recently Used Fonts, most recent first, once each, ten at
    /// most; the list is kept in `ui.json` and read back tidied.
    #[test]
    fn applied_fonts_join_recently_used_fonts() {
        let mut a = typed();
        assert!(a.ui.recent_fonts.is_empty());
        a.run("format.font", json!({"name": "Courier New"})).unwrap();
        a.run("format.font", json!({"name": "Georgia"})).unwrap();
        assert_eq!(a.ui.recent_fonts, ["Georgia", "Courier New"]);
        // Applied again: to the front, once, as typed.
        a.run("format.font", json!({"name": "courier new"})).unwrap();
        assert_eq!(a.ui.recent_fonts, ["courier new", "Georgia"]);
        // The Font dialog applies through format.set.
        a.run("format.set", json!({"props": {"font": "Verdana", "bold": true}})).unwrap();
        assert_eq!(a.ui.recent_fonts, ["Verdana", "courier new", "Georgia"]);
        // A refused command, or one applying no Latin font, adds nothing.
        assert!(a.run("format.font", json!({"name": "  "})).is_err());
        a.run("format.font", json!({"eastAsia": "SimSun"})).unwrap();
        a.run("format.set", json!({"props": {"italic": true}})).unwrap();
        assert_eq!(a.ui.recent_fonts.len(), 3);
        for i in 0..12 {
            a.run("format.font", json!({"name": format!("Font {i}")})).unwrap();
        }
        assert_eq!(a.ui.recent_fonts.len(), MAX_RECENT_FONTS);
        assert_eq!(a.ui.recent_fonts.first().map(String::as_str), Some("Font 11"));
        assert_eq!(a.ui.recent_fonts.last().map(String::as_str), Some("Font 2"));
        // Kept between runs.
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""recentFonts":["Font 11","Font 10""#), "{saved}");
        let mut b = app();
        b.apply_prefs(UiState::from_json(saved.as_bytes()).unwrap());
        assert_eq!(b.ui.recent_fonts, a.ui.recent_fonts);
        // A ui.json from before the list has nothing recent; junk in the list is dropped and the
        // rest kept once each, trimmed, ten at most; a value of the wrong type keeps the default.
        assert!(UiState::from_json(br#"{"tab": "Insert"}"#).unwrap().recent_fonts.is_empty());
        let mut list: Vec<String> = ["", "  Arial ", "ARIAL", "Georgia"].map(String::from).to_vec();
        list.extend((0..12).map(|i| format!("F{i}")));
        let raw = json!({ "recentFonts": list });
        let tidy = UiState::from_json(raw.to_string().as_bytes()).unwrap().recent_fonts;
        assert_eq!(tidy.len(), MAX_RECENT_FONTS);
        assert_eq!(&tidy[..3], ["Arial", "Georgia", "F0"]);
        assert!(UiState::from_json(br#"{"recentFonts": "Arial"}"#).unwrap().recent_fonts.is_empty());
        // One entry that isn't a name drops that entry, not the list.
        let mixed = UiState::from_json(br#"{"recentFonts": ["Arial", 3, null, {"a": 1}, "Georgia"]}"#).unwrap();
        assert_eq!(mixed.recent_fonts, ["Arial", "Georgia"]);
    }

    /// #2068 review: a hand-edited `ui.json` with a huge list or huge names read back slowly and
    /// kept a multi-megabyte name, and the Font dialog applies names of any length; neither is
    /// kept. Reading stops at ten names.
    #[test]
    fn recently_used_fonts_refuse_overlong_names_and_huge_lists() {
        let long = "N".repeat(MAX_FONT_NAME_BYTES + 1);
        let mut a = typed();
        a.run("format.font", json!({"name": "Georgia"})).unwrap();
        a.run("format.set", json!({"props": {"font": long}})).unwrap();
        assert_eq!(a.ui.recent_fonts, ["Georgia"], "the dialog's overlong name is applied but not kept");
        let mut list = vec!["Arial".to_string()];
        push_recent_font(&mut list, &long);
        push_recent_font(&mut list, &"M".repeat(MAX_FONT_NAME_BYTES));
        assert_eq!(list.len(), 2, "a name of the longest length is kept");
        // A huge saved list: overlong names skipped, the first ten others kept, quickly.
        let mut saved: Vec<String> = vec![long.clone(), "x".repeat(4 << 20)];
        saved.extend((0..200_000).map(|i| format!("Family {i}")));
        let raw = json!({ "recentFonts": saved }).to_string();
        let started = std::time::Instant::now();
        let ui = UiState::from_json(raw.as_bytes()).unwrap();
        assert_eq!(ui.recent_fonts.len(), MAX_RECENT_FONTS);
        assert_eq!(ui.recent_fonts.first().map(String::as_str), Some("Family 0"));
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "read in {:?}", started.elapsed());
        let mut b = app();
        b.apply_prefs(UiState { recent_fonts: vec![long, " Arial ".into(), "ARIAL".into()], ..UiState::default() });
        assert_eq!(b.ui.recent_fonts, ["Arial"]);
    }

    /// #2068: the font box lists the fonts applied lately under Recently Used Fonts, above All
    /// Fonts, while nothing is typed; a query lists its matches only; Down and Enter apply a
    /// recent one.
    #[test]
    fn the_font_box_lists_recently_used_fonts_first() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{By, Queryable};
        let mut a = typed();
        a.run("format.font", json!({"name": "Recent Font Alpha"})).unwrap();
        a.run("format.font", json!({"name": "Recent Font Beta"})).unwrap();
        let mut h = harness(a);
        h.run_steps(6);
        h.get(By::new().role(Role::TextInput).value("Recent Font Beta")).focus();
        h.step();
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        h.event(key(egui::Key::A, egui::Modifiers::COMMAND));
        h.step();
        h.event(egui::Event::Text("x".into()));
        h.run_steps(2);
        assert!(h.query(By::new().role(Role::TextInput).value("x")).is_some(), "the query replaced the font name");
        assert!(h.query_by_label("Recently Used Fonts").is_none(), "a query lists its matches only");
        assert_eq!(h.query_all_by_label("Recent Font Alpha").count(), 0);
        h.event(key(egui::Key::Backspace, egui::Modifiers::NONE));
        h.run_steps(2);
        assert!(h.query(By::new().role(Role::TextInput).value("")).is_some(), "Backspace cleared the query");
        assert!(h.query_by_label("Recently Used Fonts").is_some(), "the section shows while nothing is typed");
        assert!(h.query_by_label("All Fonts").is_some());
        assert_eq!(h.query_all_by_label("Recent Font Beta").count(), 1);
        assert_eq!(h.query_all_by_label("Recent Font Alpha").count(), 1);
        // Down moves to the second recent font; Enter applies it.
        h.event(key(egui::Key::ArrowDown, egui::Modifiers::NONE));
        h.step();
        h.event(key(egui::Key::Enter, egui::Modifiers::NONE));
        h.run_steps(2);
        let st = h.state_mut().session.run("format.state", &json!({})).unwrap();
        assert_eq!(st.get("font").and_then(Value::as_str), Some("Recent Font Alpha"));
        assert_eq!(h.state().ui.recent_fonts, ["Recent Font Alpha", "Recent Font Beta"]);
        // The arrow's list has the sections too; a click on a heading leaves it open.
        let field = h.get(By::new().role(Role::TextInput).value("Recent Font Alpha")).rect();
        let arrow = egui::pos2(field.max.x + 8.0, field.center().y);
        for pressed in [true, false] {
            h.hover_at(arrow);
            h.event(egui::Event::PointerButton { pos: arrow, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        }
        h.run_steps(3);
        assert!(egui::Popup::is_any_open(&h.ctx), "the arrow opened the list");
        h.get_by_label("All Fonts").click();
        h.run_steps(3);
        assert!(egui::Popup::is_any_open(&h.ctx), "a click on a heading keeps the list open");
        assert!(h.query_by_label("Recently Used Fonts").is_some());
    }

    /// #330: Word's shortcuts work from the keyboard — Ctrl+Alt+1 (⌘⌥1) applies Heading 1, and
    /// an Alt shortcut doesn't also type the character its key would produce.
    #[test]
    fn word_shortcuts_run_from_the_keyboard() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        h.event(egui::Event::Text("Kilns".into()));
        h.step();
        h.event(key(egui::Key::Num1, egui::Modifiers::COMMAND | egui::Modifiers::ALT));
        h.step();
        let style =
            |h: &egui_kittest::Harness<WordApp>| h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.style.clone());
        assert_eq!(style(&h).as_deref(), Some("Heading1"));
        // Alt+F7 (next misspelling) can come with key text on some platforms; none is typed.
        h.event(key(egui::Key::F7, egui::Modifiers::ALT));
        h.event(egui::Event::Text("x".into()));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns");
    }

    /// PR #4241: Ctrl/Cmd+Shift+C copies the selection's formatting and Ctrl/Cmd+Shift+V pastes it
    /// onto another selection, as in Word — not the text. egui 0.37 delivers those chords as keys
    /// (only Cmd/Ctrl without extra modifiers become copy/paste events), so the registry's
    /// `Mod+Shift+C`/`Mod+Shift+V` commands must run.
    #[test]
    fn ctrl_shift_c_and_v_copy_and_paste_formatting_not_text() {
        // A native event carries ⌘ (mac_cmd and command) on macOS, Ctrl (ctrl and command)
        // elsewhere; COMMAND+SHIFT is the logical chord both platforms resolve.
        let native = if cfg!(target_os = "macos") {
            egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::CTRL | egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
        };
        let copied = |h: &egui_kittest::Harness<WordApp>| -> Vec<String> {
            h.output()
                .platform_output
                .commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::CopyText(t) => Some(t.clone()),
                    _ => None,
                })
                .collect()
        };
        let bold = |h: &mut egui_kittest::Harness<WordApp>| -> Option<bool> {
            h.state_mut().run("format.state", json!({})).unwrap().get("bold").and_then(Value::as_bool)
        };
        for modifiers in [egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, native] {
            let mut a = app();
            a.run("text.insert", json!({"text": "Bold plain"})).unwrap();
            a.run("select.text", json!({"text": "Bold"})).unwrap();
            a.run("format.bold", json!({})).unwrap();
            a.run("select.text", json!({"text": "plain"})).unwrap();
            a.run("edit.copy", json!({})).unwrap();
            let clipboard_entries = a.session.clip_history.len();
            assert_eq!(a.session.clipboard_text, "plain");
            assert!(clipboard_entries > 0);
            a.run("select.text", json!({"text": "Bold"})).unwrap();
            let mut h = harness(a);
            h.run_steps(6);
            assert!(h.state().canvas.focused, "the canvas's key handler has the keyboard");
            assert_eq!(bold(&mut h), Some(true), "the source selection is bold");
            let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            h.event(key(egui::Key::C, modifiers));
            h.step();
            assert!(h.state().session.painter.is_some(), "{modifiers:?}: Copy Formatting picked up the selection's formatting");
            assert_eq!(h.state().session.clip_history.len(), clipboard_entries, "{modifiers:?}: Copy Formatting preserved clipboard history");
            assert_eq!(h.state().session.clipboard_text, "plain", "{modifiers:?}: the existing clipboard text was preserved");
            assert!(copied(&h).is_empty(), "{modifiers:?}: no text reached the system clipboard: {:?}", copied(&h));
            assert_eq!(body_text(h.state()), "Bold plain", "{modifiers:?}: copying formatting left the document text alone");

            h.state_mut().run("select.text", json!({"text": "plain"})).unwrap();
            assert_eq!(bold(&mut h), Some(false), "the target selection is plain before the paste");
            h.event(key(egui::Key::V, modifiers));
            h.step();
            assert_eq!(bold(&mut h), Some(true), "{modifiers:?}: Paste Formatting made the target bold");
            assert!(h.state().session.painter.is_none(), "{modifiers:?}: the non-sticky painter is used up");
            assert_eq!(body_text(h.state()), "Bold plain", "{modifiers:?}: pasting formatting left the document text alone");
            assert_eq!(h.state().session.clip_history.len(), clipboard_entries, "{modifiers:?}: Paste Formatting preserved clipboard history");
            assert_eq!(h.state().session.clipboard_text, "plain", "{modifiers:?}: Paste Formatting preserved the clipboard text");
            assert!(copied(&h).is_empty(), "{modifiers:?}: pasting formatting put no text on the system clipboard: {:?}", copied(&h));
        }
    }

    /// #1669: with Shift held, Backspace still erases (#707) and Delete cuts the selection, doing
    /// nothing without one, as in Word for Windows. (Windows sends Shift+Delete as a Cut; macOS and
    /// Linux send the key, which reached no command.)
    #[test]
    fn shift_delete_cuts_the_selection_and_shift_backspace_erases() {
        let mut h = harness(app());
        for _ in 0..6 {
            h.step();
        }
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let body = |h: &egui_kittest::Harness<WordApp>| h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        h.event(egui::Event::Text("Kilns fire clay".into()));
        h.step();
        h.event(key(egui::Key::Backspace, egui::Modifiers::SHIFT));
        h.step();
        assert_eq!(body(&h), "Kilns fire cla");
        h.event(key(egui::Key::Home, egui::Modifiers::NONE));
        h.event(key(egui::Key::Delete, egui::Modifiers::SHIFT));
        h.step();
        assert_eq!(body(&h), "Kilns fire cla", "no selection: nothing to cut");
        h.state_mut().run("select.text", json!({"text": "fire "})).unwrap();
        h.event(key(egui::Key::Delete, egui::Modifiers::SHIFT));
        h.step();
        assert_eq!(body(&h), "Kilns cla");
        assert_eq!(h.state().session.clipboard_text, "fire ", "cut, not just deleted");
    }

    /// #1538: inserting WordArt leaves its text selected, so Delete erased only the text and the
    /// empty box stayed. Escape now selects the box itself, Delete removes it, and a second Escape
    /// is back to a caret in the body.
    #[test]
    fn escape_selects_a_wordart_box_and_delete_removes_it() {
        let mut a = app();
        let id = a.run("insert.wordArt", json!({})).unwrap()["story"].as_u64().unwrap() as u32;
        assert!(canvas::in_text_box(&a), "the WordArt's text is selected after inserting it");
        let mut h = harness(a);
        h.run_steps(4);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_some(), "Escape selects the box");
        h.key_press(egui::Key::Delete);
        h.run_steps(2);
        assert_eq!(h.state().session.doc.text_box_anchor(id), None, "Delete removed the WordArt");
        assert!(h.state_mut().session.undo(), "one undo brings it back");
        assert!(h.state().session.doc.text_box_anchor(id).is_some());
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        let s = &h.state().session;
        assert!(s.sel.is_collapsed() && s.sel.focus.story == wordcraft_doc::StoryRef::Body, "a second Escape leaves a caret in the body");
    }

    /// #1218: Ctrl+Q resets the paragraph formatting and nothing else; it also closed the window
    /// (asking to save) on Windows and Linux. ⌘Q still quits on the Mac.
    #[test]
    fn ctrl_q_resets_the_paragraph_without_closing_the_window() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let closes = |h: &egui_kittest::Harness<WordApp>| {
            h.output().viewport_output.values().flat_map(|v| v.commands.iter()).any(|c| matches!(c, egui::ViewportCommand::Close))
        };
        let align = |h: &egui_kittest::Harness<WordApp>| h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align);
        h.event(egui::Event::Text("Kilns".into()));
        h.step();
        h.state_mut().run("para.alignCenter", json!({})).unwrap();
        h.step();
        assert!(align(&h).is_some());
        // Off the Mac, Ctrl is the command key: egui sets both `ctrl` and `command`.
        let ctrl_q = if cfg!(target_os = "macos") { egui::Modifiers::CTRL } else { egui::Modifiers::CTRL | egui::Modifiers::COMMAND };
        h.event(key(egui::Key::Q, ctrl_q));
        h.step();
        assert_eq!(align(&h), None, "Ctrl+Q resets the paragraph formatting");
        // Ctrl+Q as Windows and Linux send it, on any host.
        h.event(key(egui::Key::Q, egui::Modifiers::CTRL | egui::Modifiers::COMMAND));
        h.step();
        assert!(!closes(&h), "Ctrl+Q doesn't close the window");
        h.event(key(egui::Key::Q, egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND));
        h.step();
        // Quitting asks about the typed text first, in the document's window.
        assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("saveChanges"), "⌘Q still quits on the Mac");
    }

    /// #3424: Pressing Ctrl+Alt+Q (German shortcut for @, or AltGr+Q) types '@' and does not
    /// close the window.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ctrl_alt_q_types_at_symbol_without_closing() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let closes = |h: &egui_kittest::Harness<WordApp>| {
            h.output().viewport_output.values().flat_map(|v| v.commands.iter()).any(|c| matches!(c, egui::ViewportCommand::Close))
        };
        let german_mods = egui::Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: true };
        h.event(egui::Event::Key { key: egui::Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: german_mods });
        h.step();
        assert!(!closes(&h), "Ctrl+Alt+Q does not close the window");

        h.event(egui::Event::ModifiersChanged(german_mods));
        h.event(egui::Event::Text("@".into()));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "@", "Ctrl+Alt+Q types the @ character");
    }

    /// #819: Alt+Q opens Search with its box focused, as in Word: the Q isn't typed into the
    /// document, and what is typed next goes into the search box.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn alt_q_opens_search_and_typing_goes_there() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        h.event(egui::Event::Text("Kilns".into()));
        h.step();
        // Some platforms send the key's text with an Alt shortcut; it isn't typed.
        h.event(key(egui::Key::Q, egui::Modifiers::ALT));
        h.event(egui::Event::Text("q".into()));
        h.step();
        let query = |h: &egui_kittest::Harness<WordApp>| match &h.state().dialog {
            Some(dialogs::Dialog::Commands { query, .. }) => Some(query.clone()),
            _ => None,
        };
        assert_eq!(query(&h).as_deref(), Some(""), "Search is open and empty");
        h.step();
        {
            use egui_kittest::kittest::Queryable;
            assert!(h.query_by_label_contains("Word Count").is_some(), "an empty box suggests commands, as Word's does");
        }
        h.event(egui::Event::Text("bold".into()));
        h.step();
        assert_eq!(query(&h).as_deref(), Some("bold"), "typing goes into the search box");
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns");
        h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
        h.step();
        h.step();
        assert!(h.state().dialog.is_none(), "Escape closes Search");
        // Words that name no command: Enter finds them in the document, in the Navigation pane.
        h.event(key(egui::Key::Q, egui::Modifiers::ALT));
        h.step();
        h.step();
        // Typing sends each key's press before its text.
        h.event(key(egui::Key::K, egui::Modifiers::SHIFT));
        h.event(egui::Event::Text("Kilns".into()));
        h.step();
        h.event(key(egui::Key::Enter, egui::Modifiers::NONE));
        h.step();
        h.step();
        assert!(h.state().dialog.is_none(), "Enter ran the search");
        assert!(h.state().session.view.nav_pane, "the Navigation pane shows the results");
        assert_eq!(h.state().session.find.results.len(), 1);
    }

    /// #819: `tools.search` with a query lists matching commands for scripts and agents; every
    /// word has to match, so "insert table" finds the command that inserts a table.
    #[test]
    fn search_command_matches_every_word() {
        let mut a = app();
        let r = a.run("tools.search", json!({"query": "insert table"})).unwrap();
        let ids: Vec<&str> = r["matches"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
        assert!(ids.contains(&"insert.table"), "{ids:?}");
        assert!(r["matches"].as_array().unwrap().iter().all(|m| {
            let hay = format!("{} {} {}", m["label"], m["location"], m["id"]).to_lowercase();
            hay.contains("insert") && hay.contains("table")
        }));
        assert_eq!(a.run("tools.search", json!({"query": "   "})).unwrap()["matches"], json!([]));
        assert_eq!(a.run("tools.search", json!({"query": "zzqqxx"})).unwrap()["matches"], json!([]));
        assert!(a.dialog.is_none(), "a query never opens the box");
        a.run("tools.search", json!({})).unwrap();
        assert!(matches!(a.dialog, Some(dialogs::Dialog::Commands { .. })), "no query opens Search");
    }

    /// Save writes the document first, then carries on with what the user asked for.
    #[test]
    fn save_then_continue() {
        let dir = scratch("save");
        let path = docx_on_disk(&dir, "Draft");
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(!a.has_document(), "the saved document closed");
        let saved = wordcraft_engine::io::open_path(&path).unwrap();
        assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("Revised Draft"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save As to a format that doesn't keep everything (a .png of page 1, plain text) reports
    /// `saved` but leaves the document unsaved; the prompt took that as saved and replaced it.
    #[test]
    fn a_save_that_drops_content_does_not_continue() {
        for ext in ["png", "txt"] {
            let dir = scratch(&format!("lossy-{ext}"));
            let picked = dir.join(format!("novel.{ext}")).to_string_lossy().to_string();
            let mut a = typed();
            a.services.pick_save = Some(Box::new(move |_| Some(picked.clone())));
            a.run("file.close", json!({})).unwrap();
            a.status_msg = None;
            let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
            assert_eq!(r["done"], false, "{ext}: the Close was cancelled");
            assert!(dir.join(format!("novel.{ext}")).exists(), "{ext}: the copy was written");
            assert_eq!(prompt(&a), None);
            assert!(body_text(&a).contains(UNSAVED), "{ext}: the document is kept");
            assert!(a.session.dirty);
            assert!(a.status_msg.is_some(), "{ext}: the status bar says why");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Web: a download in a format that doesn't keep everything marked the document saved, so
    /// the prompt and the leave-page guard let it go.
    #[test]
    fn web_downloads_that_drop_content_leave_the_document_unsaved() {
        for name in ["novel.png", "novel.txt", "novel.md"] {
            let mut a = typed();
            a.services.download = Some(Box::new(|_, _| Ok(())));
            a.execute("file.saveAs", json!({"path": name})).unwrap();
            assert!(a.session.dirty, "{name}");
        }
        for name in ["novel.docx", "novel.docm", "novel.DOTX", "novel.dotm", "novel.odt", "novel.rtf"] {
            let mut a = typed();
            a.services.download = Some(Box::new(|_, _| Ok(())));
            a.execute("file.saveAs", json!({"path": name})).unwrap();
            assert!(!a.session.dirty, "{name}");
        }
    }

    /// Web inside a page that keeps the files: the app with `host_save` recording each file it
    /// is handed (name, Save As, size).
    fn host_app(fail: bool) -> (WordApp, std::rc::Rc<std::cell::RefCell<Vec<(String, bool, usize)>>>) {
        let mut a = typed();
        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = got.clone();
        a.services.download = Some(Box::new(|_, _| Err("downloads go to the host page".into())));
        a.services.host_save = Some(Box::new(move |name: &str, bytes: &[u8], save_as: bool| {
            if fail {
                return Err("the page is gone".into());
            }
            seen.borrow_mut().push((name.to_string(), save_as, bytes.len()));
            Ok(())
        }));
        (a, got)
    }

    /// Open `name` as the host page sends it (bytes of the typed document in that format).
    fn host_open(a: &mut WordApp, name: &str) {
        let bytes = wordcraft_engine::io::save_bytes_with(name, &typed().session.doc, None, &Default::default()).unwrap();
        a.execute("file.open", json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)})).unwrap();
        a.run("text.insert", json!({"text": "Edited "})).unwrap();
        assert!(a.session.dirty);
    }

    /// Save hands the opened file back under its own name and format; Save As says so and
    /// becomes the file the next Save writes.
    #[test]
    fn host_saves_go_back_to_the_opened_file() {
        let (mut a, got) = host_app(false);
        host_open(&mut a, "report.odt");
        a.execute("file.save", json!({})).unwrap();
        assert!(!a.session.dirty);
        a.run("text.insert", json!({"text": "More "})).unwrap();
        a.execute("file.saveAs", json!({"path": "report copy.docx"})).unwrap();
        assert!(!a.session.dirty);
        a.run("text.insert", json!({"text": "Again "})).unwrap();
        a.execute("file.save", json!({})).unwrap();
        let got = got.borrow();
        let calls: Vec<(&str, bool)> = got.iter().map(|(n, s, _)| (n.as_str(), *s)).collect();
        assert_eq!(calls, [("report.odt", false), ("report copy.docx", true), ("report copy.docx", false)]);
        assert!(got.iter().all(|(_, _, len)| *len > 0));
    }

    /// A Markdown file goes back as Markdown, which doesn't keep everything: the document stays
    /// unsaved, as on the desktop.
    #[test]
    fn host_save_of_markdown_keeps_its_format_and_stays_unsaved() {
        let (mut a, got) = host_app(false);
        host_open(&mut a, "notes.md");
        a.execute("file.save", json!({})).unwrap();
        assert_eq!(got.borrow().first().map(|(n, s, _)| (n.as_str(), *s)), Some(("notes.md", false)));
        assert!(a.session.dirty);
        assert_eq!(a.session.path.as_deref(), Some(std::path::Path::new("notes.md")));
    }

    /// An import-only Word 97–2003 file is saved as a .docx, never back over itself (#2445). The
    /// host page never had that name, so it's a new file: Save As.
    #[test]
    fn host_save_of_a_legacy_word_file_writes_a_docx() {
        let (mut a, got) = host_app(false);
        a.session.path = Some("old.doc".into());
        a.execute("file.save", json!({})).unwrap();
        assert_eq!(got.borrow().first().map(|(n, s, _)| (n.as_str(), *s)), Some(("old.docx", true)));
    }

    /// Saving a document the host page never sent is Save As, so the page doesn't overwrite a
    /// file of the same name; the next Save goes back to it.
    #[test]
    fn host_save_of_an_untitled_document_is_save_as() {
        let (mut a, got) = host_app(false);
        a.execute("file.save", json!({})).unwrap();
        a.run("text.insert", json!({"text": "More "})).unwrap();
        a.execute("file.save", json!({})).unwrap();
        let got = got.borrow();
        let calls: Vec<bool> = got.iter().map(|(_, s, _)| *s).collect();
        assert_eq!(calls, [true, false], "{got:?}");
        assert_eq!(got.first().map(|(n, ..)| n), got.get(1).map(|(n, ..)| n));
    }

    /// Exports without a name are named after the document, not after its file.
    #[test]
    fn web_exports_are_named_after_the_document() {
        let (mut a, got) = host_app(false);
        host_open(&mut a, "report.docx");
        a.execute("file.exportPdf", json!({})).unwrap();
        a.execute("file.exportPng", json!({})).unwrap();
        let names: Vec<(String, bool)> = got.borrow().iter().map(|(n, s, _)| (n.clone(), *s)).collect();
        assert_eq!(names, [("report.pdf".to_string(), false), ("report.png".to_string(), false)]);
        assert!(a.session.dirty, "an export is a copy");
    }

    /// Save As in a format that doesn't keep everything (File › Export to PDF or Markdown) is a
    /// copy: the host page isn't told the document is now that file.
    #[test]
    fn host_save_as_a_copy_format_is_not_save_as() {
        let (mut a, got) = host_app(false);
        host_open(&mut a, "report.docx");
        a.execute("file.saveAs", json!({"path": "report.pdf"})).unwrap();
        a.execute("file.saveAs", json!({"path": "report.md"})).unwrap();
        let calls: Vec<(String, bool)> = got.borrow().iter().map(|(n, s, _)| (n.clone(), *s)).collect();
        assert_eq!(calls, [("report.pdf".to_string(), false), ("report.md".to_string(), false)]);
        assert_eq!(a.session.path.as_deref(), Some(std::path::Path::new("report.docx")));
    }

    /// The page not getting the file is a failed save: the document keeps its changes.
    #[test]
    fn a_host_save_that_fails_leaves_the_document_unsaved() {
        let (mut a, _) = host_app(true);
        host_open(&mut a, "report.docx");
        assert!(a.execute("file.save", json!({})).is_err());
        assert!(a.session.dirty);
    }

    /// Macro-enabled documents and templates keep everything (#172), so once saved in this
    /// session AutoSave covers them like a .docx.
    #[test]
    fn autosave_covers_macro_enabled_documents_and_templates() {
        for ext in ["docm", "dotx", "dotm"] {
            let dir = scratch(&format!("autosave-{ext}"));
            let path = dir.join(format!("novel.{ext}"));
            let mut a = typed();
            a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
            assert!(a.autosaves(), "{ext}");
            a.run("text.insert", json!({"text": "More "})).unwrap();
            a.autosave_tick(now_ms() + 10_000.0);
            assert!(!a.session.dirty, "{ext}: AutoSave wrote the file");
            let saved = wordcraft_engine::io::open_path(&path).unwrap();
            assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("More "), "{ext}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn renaming_a_saved_file_keeps_autosave_and_replaces_its_recent_entry() {
        let dir = scratch("rename-autosave");
        let path = dir.join("notes.docx");
        let renamed = dir.join("Notes v2.docx");
        let mut a = typed();
        a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(a.autosaves());
        a.run("file.rename", json!({"name": "Notes v2"})).unwrap();
        assert_eq!(a.session.path.as_deref(), Some(renamed.as_path()));
        assert!(a.autosaves(), "AutoSave still covers the renamed file");
        assert_eq!(a.ui.recent.first(), Some(&renamed.to_string_lossy().to_string()));
        assert!(!a.ui.recent.contains(&path.to_string_lossy().to_string()), "{:?}", a.ui.recent);
        a.run("text.insert", json!({"text": "More "})).unwrap();
        a.autosave_tick(now_ms() + 10_000.0);
        assert!(!a.session.dirty, "AutoSave wrote the renamed file");
        let saved = wordcraft_engine::io::open_path(&renamed).unwrap();
        assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("More "));
        assert!(!path.exists());

        // A file opened but not saved in this session doesn't gain AutoSave by being renamed.
        let mut b = app();
        b.run("file.open", json!({"path": renamed.to_string_lossy()})).unwrap();
        assert!(!b.saved_here());
        b.run("file.rename", json!({"name": "Notes v3"})).unwrap();
        assert!(!b.saved_here() && !b.autosaves());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Web: the download callback couldn't report a failure, so Save marked the document saved
    /// and the prompt went ahead even when no download started.
    #[test]
    fn a_failed_download_is_not_a_save() {
        let mut a = typed();
        a.services.download = Some(Box::new(|_, _| Err("blocked by the browser".into())));
        assert!(a.execute("file.save", json!({})).is_err());
        assert!(a.session.dirty);

        a.run("file.close", json!({})).unwrap();
        a.status_msg = None;
        // A failed save is an error, not a quiet Cancel (#1445); the Close doesn't run.
        let err = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap_err();
        assert!(err.contains("still open") && err.contains("blocked by the browser"), "{err}");
        assert!(body_text(&a).contains(UNSAVED));
        assert!(a.session.dirty);
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(msg.contains("blocked by the browser"), "the status bar says why: {msg:?}");
    }

    /// #1315: a web app whose `download` hands the save to the browser's picker, which finishes
    /// later. Returns the app (with unsaved typing) and the saves its download starts, plus the
    /// last ticket started.
    fn web_picker_app() -> (WordApp, async_saves::AsyncSaves, std::rc::Rc<std::cell::Cell<u64>>) {
        let mut a = typed();
        let saves = async_saves::AsyncSaves::default();
        let last = std::rc::Rc::new(std::cell::Cell::new(0));
        let (hook, seen) = (saves.clone(), last.clone());
        a.services.download = Some(Box::new(move |_, _| {
            seen.set(hook.start());
            Ok(())
        }));
        a.services.async_saves = Some(saves.clone());
        (a, saves, last)
    }

    /// #1315: Save in the prompt before Close waits for the picker: Close runs only once the file
    /// is written and closed.
    #[test]
    fn web_save_before_close_waits_for_the_picker_to_finish() {
        let (mut a, saves, last) = web_picker_app();
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["pending"], "save", "{r}");
        assert_eq!(saves.pending(), 1);
        a.settle_async_saves();
        assert!(body_text(&a).contains(UNSAVED), "nothing closes while the picker is open");
        saves.finish(last.get(), async_saves::SaveOutcome::Saved);
        a.settle_async_saves();
        assert!(!body_text(&a).contains(UNSAVED), "the Close ran once the save finished");
        assert!(!a.session.dirty);
        assert_eq!(prompt(&a), None);
    }

    /// #1315: cancelling the picker, or a write/close that fails, keeps the document: its text,
    /// its undo, still unsaved, and the prompt back with the reason. Nothing is replaced.
    #[test]
    fn web_save_before_close_that_is_cancelled_or_fails_keeps_the_document() {
        for (outcome, reason) in
            [(async_saves::SaveOutcome::Cancelled, "cancel"), (async_saves::SaveOutcome::Failed("the disk is full".into()), "the disk is full")]
        {
            let (mut a, saves, last) = web_picker_app();
            a.run("file.close", json!({})).unwrap();
            a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
            a.status_msg = None;
            saves.finish(last.get(), outcome.clone());
            a.settle_async_saves();
            assert!(body_text(&a).contains(UNSAVED), "{outcome:?}: nothing was replaced");
            assert!(a.session.dirty, "{outcome:?}: still unsaved");
            assert_eq!(prompt(&a), Some("saveChanges"), "{outcome:?}: the prompt is back");
            let message = prompt_message(&a).unwrap_or_default();
            assert!(message.contains("still open") && message.contains(reason), "{outcome:?}: {message}");
            let status = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
            assert!(status.contains(reason), "{outcome:?}: the status bar says why: {status:?}");
            // The undo history is still there.
            a.execute("edit.undo", json!({})).unwrap();
            assert!(!body_text(&a).contains(UNSAVED), "{outcome:?}: the typing can be undone");
        }
    }

    /// #1315 review: the prompt's Save is in the picker (ticket 1), the user edits and saves
    /// again (ticket 2, which marks the document saved at once). Ticket 1 ending doesn't let the
    /// held-back Close close the document while ticket 2 hasn't written its edits.
    #[test]
    fn web_overlapping_saves_hold_the_replacement() {
        let (mut a, saves, last) = web_picker_app();
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        let first = last.get();
        a.run("text.insert", json!({"text": "Later edit "})).unwrap();
        a.execute("file.save", json!({})).unwrap();
        assert!(!a.session.dirty, "the second save marked the document saved");
        saves.finish(first, async_saves::SaveOutcome::Saved);
        a.settle_async_saves();
        assert!(body_text(&a).contains("Later edit") && body_text(&a).contains(UNSAVED), "nothing was replaced");
        assert_eq!(prompt(&a), Some("saveChanges"), "the prompt asks again");
    }

    /// #1315 review: edited while the picker was open, then the save ends: those edits aren't
    /// in the file, so the prompt asks again instead of running Close.
    #[test]
    fn web_save_that_ends_after_an_edit_asks_again() {
        let (mut a, saves, last) = web_picker_app();
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        a.run("text.insert", json!({"text": "Later edit "})).unwrap();
        saves.finish(last.get(), async_saves::SaveOutcome::Saved);
        a.settle_async_saves();
        assert!(body_text(&a).contains("Later edit"), "nothing was replaced");
        assert!(a.session.dirty);
        assert_eq!(prompt(&a), Some("saveChanges"));
        assert_eq!(prompt_message(&a).as_deref(), Some(""));
    }

    /// #1315 review: the status bar says Close is waiting for the save; a cancelled picker
    /// says it was cancelled, not that saving failed.
    #[test]
    fn web_save_prompt_says_it_waits_and_that_a_cancel_was_a_cancel() {
        let (mut a, saves, last) = web_picker_app();
        a.run("file.close", json!({})).unwrap();
        a.status_msg = None;
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        let status = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(status.contains("Waiting for the save"), "{status:?}");
        saves.finish(last.get(), async_saves::SaveOutcome::Cancelled);
        a.settle_async_saves();
        let message = prompt_message(&a).unwrap_or_default();
        assert!(message.contains("cancelled") && message.contains("still open"), "{message}");
        assert!(!message.contains("Couldn't save"), "a cancel isn't a failure: {message}");
    }

    /// #1315 review: a document dropped or picked on the web (`file.open` with its bytes) opens
    /// beside one whose save is still in the picker, asking nothing.
    #[test]
    fn web_open_opens_beside_a_save_in_flight() {
        let dir = scratch("web-open-after-save");
        let bytes = std::fs::read(docx_on_disk(&dir, "Picked document")).unwrap();
        let (mut a, _saves, _last) = web_picker_app();
        let first = a.session.document_id();
        a.execute("file.save", json!({})).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        a.run("file.open", json!({"path": "picked.docx", "data": data})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(body_text(&a).contains("Picked document"), "the picked file opened: {:?}", body_text(&a));
        a.run("view.switchWindows", json!({"id": first})).unwrap();
        assert!(body_text(&a).contains(UNSAVED), "the first document is still open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #1315: while the picker save is in flight the document's save isn't confirmed, so Close
    /// asks again instead of closing it.
    #[test]
    fn web_close_while_a_picker_save_is_in_flight_asks_first() {
        let (mut a, _saves, _last) = web_picker_app();
        a.execute("file.save", json!({})).unwrap();
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        assert!(body_text(&a).contains(UNSAVED), "nothing was replaced");
    }

    /// #1315: a save that ends after another document is shown neither runs the held-back Close
    /// on that document nor marks it unsaved.
    #[test]
    fn web_picker_save_ending_after_the_document_changed_leaves_the_new_one_alone() {
        for outcome in [async_saves::SaveOutcome::Saved, async_saves::SaveOutcome::Failed("gone".into())] {
            let (mut a, saves, last) = web_picker_app();
            a.run("file.close", json!({})).unwrap();
            a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
            // Another document is shown (an agent's or a script's command) while the picker is open.
            a.execute("file.new", json!({})).unwrap();
            a.execute("text.insert", json!({"text": "Second document"})).unwrap();
            a.session.dirty = false;
            saves.finish(last.get(), outcome.clone());
            a.settle_async_saves();
            assert!(body_text(&a).contains("Second document"), "{outcome:?}: the held-back Close didn't run on it");
            assert!(!a.session.dirty, "{outcome:?}: the new document isn't marked unsaved");
            assert_eq!(prompt(&a), None, "{outcome:?}");
        }
    }

    /// A new document has no path: Save goes through Save As, and cancelling that cancels the Close.
    #[test]
    fn save_as_cancelled_keeps_the_document() {
        let mut a = typed();
        a.services.pick_save = Some(Box::new(|_| None));
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(body_text(&a).contains(UNSAVED));
        assert!(a.session.dirty);
    }

    #[test]
    fn save_as_picked_saves_then_continues() {
        let dir = scratch("saveas");
        let path = dir.join("novel.docx");
        let picked = path.to_string_lossy().to_string();
        let mut a = typed();
        a.services.pick_save = Some(Box::new(move |_| Some(picked.clone())));
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert!(!a.has_document(), "closed once saved");
        let saved = wordcraft_engine::io::open_path(&path).unwrap();
        assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains(UNSAVED));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Scripts and agents (control channel, MCP bridge) are never asked anything.
    #[test]
    fn programmatic_commands_never_prompt() {
        let ctx = egui::Context::default();
        let mut a = typed();
        let (req, _reply) = ControlRequest::new("engine.execute", json!({"command": "file.new"}));
        control::handle(&mut a, &ctx, &req);
        assert_eq!(prompt(&a), None);
        assert!(!body_text(&a).contains(UNSAVED));
    }

    /// Send `engine.execute` the way the control channel does and return the reply (#1268).
    fn control_execute(a: &mut WordApp, params: Value) -> Value {
        let ctx = egui::Context::default();
        let (req, _reply) = ControlRequest::new("engine.execute", params);
        match control::handle(a, &ctx, &req) {
            control::Outcome::Done(v) => v,
            _ => json!({"ok": false, "error": "not a reply"}),
        }
    }

    /// A programmatic `file.close` on unsaved work doesn't quit and lose it (#1268).
    #[test]
    fn programmatic_close_keeps_unsaved_work() {
        let mut a = typed();
        let r = control_execute(&mut a, json!({"command": "file.close"}));
        assert_eq!(r["ok"], json!(false), "{r}");
        let e = r["error"].as_str().unwrap_or("");
        assert!(e.contains("unsaved") && e.contains("force"), "{e}");
        assert!(a.has_document() && !a.quit_requested);
        assert!(body_text(&a).contains(UNSAVED));

        let r = control_execute(&mut a, json!({"command": "file.close", "params": {"force": true}}));
        assert_eq!(r["ok"], json!(true), "{r}");
        // Closed: the document, not the window (#1712).
        assert!(!a.has_document() && !a.quit_requested);

        let mut clean = app();
        let r = control_execute(&mut clean, json!({"command": "file.close"}));
        assert_eq!(r["ok"], json!(true), "{r}");
        assert!(!clean.has_document() && !clean.quit_requested);
    }

    /// Every way to name `file.close` over the control channel is guarded (#1268).
    #[test]
    fn programmatic_close_guard_covers_every_form() {
        let ctx = egui::Context::default();
        let send = |a: &mut WordApp, method: &str, params: Value| match control::handle(a, &ctx, &ControlRequest::new(method, params).0) {
            control::Outcome::Done(v) => v,
            _ => json!({"ok": false, "error": "not a reply"}),
        };
        let refused = [
            ("file.close", json!({})),
            ("command", json!({"id": "file.close"})),
            ("command", json!({"command": "file.close", "params": {"force": "yes"}})),
            ("file.close", json!({"force": "yes"})),
            ("file.close", json!({"force": 1})),
        ];
        for (method, params) in refused {
            let mut a = typed();
            let r = send(&mut a, method, params.clone());
            assert_eq!(r["ok"], json!(false), "{method} {params}: {r}");
            assert!(a.has_document() && !a.quit_requested, "{method} {params}");
            assert!(body_text(&a).contains(UNSAVED));
        }
        let mut a = typed();
        let r = send(&mut a, "file.close", json!({"force": true}));
        assert_eq!(r["ok"], json!(true), "{r}");
        assert!(!a.has_document() && !a.quit_requested);
    }

    const MAILINGS: [&str; 4] = ["mailings.envelopes", "mailings.labels", "mailings.finish", "mailings.printDocuments"];

    /// What the user gave: Envelopes and Labels without anything open their dialogs (#407).
    fn mailing(id: &str) -> Value {
        match id {
            "mailings.envelopes" => json!({"delivery": "Ada"}),
            "mailings.labels" => json!({"text": "Ada"}),
            _ => json!({"records": "all"}),
        }
    }

    /// #3028: Finish & Merge and Print Documents from the ribbon ask which records first; with
    /// records given (scripts, the dialog's OK) they merge.
    #[test]
    fn finish_and_print_documents_ask_which_records() {
        let mut a = app();
        a.run("mailings.recipients", json!({"csv": "Name\nAda\nAlan"})).unwrap();
        for (id, dialog) in [("mailings.finish", "mergeRecords"), ("mailings.printDocuments", "printRecords")] {
            assert_eq!(a.run(id, json!({})).unwrap(), json!({"pending": dialog}));
            assert_eq!(prompt(&a), Some(dialog));
            assert!(matches!(&a.dialog, Some(dialogs::Dialog::MergeRecords { from, to, .. }) if from == "1" && to == "2"));
            a.dialog = None;
        }
    }

    /// Envelopes, Labels and Finish & Merge open their result as a new document beside the
    /// unsaved one, like New, asking nothing.
    #[test]
    fn mailings_open_beside_unsaved_work() {
        for id in MAILINGS {
            let mut a = typed();
            a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            let document = a.session.document_id();
            a.run(id, mailing(id)).unwrap();
            assert_eq!(prompt(&a), None, "{id}");
            assert_ne!(a.session.document_id(), document, "{id}: the result is a new document");
            assert_eq!(a.session.path, None);
            a.run("view.switchWindows", json!({"id": document})).unwrap();
            assert!(body_text(&a).contains(UNSAVED) && a.session.dirty, "{id}: the unsaved document is still open");
        }
        // A `path` that isn't text merges into a new document beside this one, like no path at all.
        let mut a = typed();
        a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        let document = a.session.document_id();
        a.run("mailings.finish", json!({"path": null, "records": "all"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert_ne!(a.session.document_id(), document);
    }

    /// A clean document is replaced straight away, like New; a merge written to a file
    /// (`mailings.finish` with `path`) doesn't replace anything, so it never asks.
    #[test]
    fn mailings_on_a_clean_document_or_into_a_file_do_not_ask() {
        let dir = scratch("merge-to-file");
        for id in MAILINGS {
            let mut a = app();
            a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            a.run("file.save", json!({"path": dir.join("saved.docx").to_string_lossy()})).unwrap();
            assert!(!a.session.dirty);
            let document = a.session.document_id();
            a.run(id, mailing(id)).unwrap();
            assert_eq!(prompt(&a), None, "{id}");
            assert_ne!(a.session.document_id(), document, "{id}");
        }
        let out = dir.join("merged.docx");
        let mut a = typed();
        a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        a.run("mailings.finish", json!({"path": out.to_string_lossy()})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(out.exists());
        assert!(body_text(&a).contains(UNSAVED));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The result is a new, untitled document: Save, AutoSave and the title never point at the
    /// file the original came from.
    #[test]
    fn mailings_results_never_save_over_the_original() {
        for id in MAILINGS {
            let dir = scratch(&format!("mailings-{}", id.len()));
            let path = docx_on_disk(&dir, "Signed contract");
            let mut a = app();
            a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
            a.run("text.insert", json!({"text": "Revised "})).unwrap();
            a.run("file.save", json!({})).unwrap();
            assert!(a.autosaves());
            a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            a.run("text.insert", json!({"text": "Again "})).unwrap();
            let saved = std::fs::read(&path).unwrap();
            a.run(id, mailing(id)).unwrap();
            assert_eq!(prompt(&a), None);

            assert_eq!(a.session.path, None, "{id}");
            assert!(!a.saved_here() && !a.autosaves(), "{id}: AutoSave doesn't cover the result");
            assert_ne!(a.title_stem(), "contract", "{id}: titled as a new document");
            a.run("text.insert", json!({"text": "Note "})).unwrap();
            a.autosave_tick(now_ms() + 10_000.0);
            a.run("file.save", json!({})).unwrap();
            a.run("edit.undo", json!({})).unwrap();
            a.run("edit.undo", json!({})).unwrap();
            a.run("file.save", json!({})).unwrap();
            a.autosave_tick(now_ms() + 20_000.0);
            assert_eq!(std::fs::read(&path).unwrap(), saved, "{id}: the original file is never overwritten");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Scripts and agents (control channel, MCP bridge) replace the document without a question.
    #[test]
    fn programmatic_mailings_never_prompt() {
        let ctx = egui::Context::default();
        for id in MAILINGS {
            let mut a = typed();
            a.execute("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            let document = a.session.document_id();
            let (req, _reply) = ControlRequest::new("engine.execute", json!({"command": id}));
            control::handle(&mut a, &ctx, &req);
            assert_eq!(prompt(&a), None, "{id}");
            assert_ne!(a.session.document_id(), document, "{id}");
            assert_eq!(a.session.path, None, "{id}");
        }
    }

    /// Review finding: an agent's Undo while a prompt about the mailing result B was up must not
    /// bring document A back under B, where Don't Save would throw A away. The result is its
    /// own document: Undo stays in B, and Don't Save closes B and shows A, unsaved work intact.
    #[test]
    fn undo_after_a_mailing_cannot_redirect_a_pending_prompt() {
        for id in MAILINGS {
            let mut a = typed();
            a.execute("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            let first = a.session.document_id();
            a.execute(id, json!({})).unwrap();
            let b = a.session.document_id();
            assert_ne!(b, first, "{id}");
            a.run("text.insert", json!({"text": "Edited result "})).unwrap();
            a.run("file.close", json!({})).unwrap();
            assert_eq!(prompt(&a), Some("saveChanges"), "{id}");

            a.execute("edit.undo", json!({})).unwrap();
            assert_eq!(a.session.document_id(), b, "{id}");
            a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
            assert!(!a.session.is_open(b), "{id}: Don't Save closed the document it asked about");
            assert_eq!(a.session.document_id(), first, "{id}");
            assert!(body_text(&a).contains(UNSAVED) && a.session.dirty, "{id}");
        }
    }

    /// Review finding: with undo kept across the swap, Undo → Save As → Redo put the mailing result
    /// under the saved file, and Save or AutoSave then wrote it there.
    #[test]
    fn redo_after_a_mailing_cannot_put_the_result_under_a_saved_file() {
        for id in MAILINGS {
            let dir = scratch(&format!("mailings-redo-{}", id.len()));
            let path = docx_on_disk(&dir, "Signed contract");
            let copy = dir.join("copy.docx");
            let on_file = |p: &std::path::Path| wordcraft_engine::io::open_path(p).unwrap().plain_text(wordcraft_doc::StoryRef::Body);
            let mut a = app();
            a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
            a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            a.run("file.save", json!({})).unwrap();
            a.run(id, mailing(id)).unwrap();
            assert_eq!(prompt(&a), None, "{id}: the document was saved");
            let result = body_text(&a);

            a.run("edit.undo", json!({})).unwrap();
            assert_eq!(body_text(&a), result, "{id}: history starts afresh, like New");
            a.run("file.saveAs", json!({"path": copy.to_string_lossy()})).unwrap();
            let written = on_file(&copy);
            a.run("edit.redo", json!({})).unwrap();
            assert_eq!(body_text(&a), result, "{id}: nothing to redo");
            a.run("file.save", json!({})).unwrap();
            a.autosave_tick(now_ms() + 10_000.0);
            assert_eq!(on_file(&copy), written, "{id}");
            assert!(on_file(&path).contains("Signed contract"), "{id}: the original is untouched");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Surfaces finding 3: AutoSave rewrote any opened .docx ~2.5 s after the first keystroke,
    /// deleting whatever the reader doesn't model (content controls, charts, macros…).
    #[test]
    fn opened_files_are_not_autosaved_until_the_user_saves() {
        let dir = scratch("autosave");
        let path = docx_on_disk(&dir, "Original");
        let before = std::fs::read(&path).unwrap();
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Typed "})).unwrap();
        a.autosave_tick(now_ms() + 10_000.0);
        assert!(std::fs::read(&path).unwrap() == before, "an opened file is never rewritten behind the user's back");
        assert!(a.session.dirty);

        // After an explicit save, later changes are saved automatically.
        a.run("file.save", json!({})).unwrap();
        a.run("text.insert", json!({"text": "More "})).unwrap();
        assert!(a.session.dirty);
        a.autosave_tick(now_ms() + 10_000.0);
        assert!(!a.session.dirty);
        let saved = wordcraft_engine::io::open_path(&path).unwrap();
        assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("Typed More Original"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #196: a saved .md file showed AutoSave on, but AutoSave writes only formats that keep
    /// everything, so edits stayed unsaved. The switch is now off and blocked with a reason, and
    /// scripts turning it on get that reason as an error; a .docx saved here is covered.
    #[test]
    fn autosave_is_blocked_with_a_reason_for_files_it_cannot_keep() {
        let dir = scratch("autosave-md");
        let md = dir.join("notes.md");
        std::fs::write(&md, "# Notes\n\nSome text\n").unwrap();
        let mut a = app();
        assert_eq!(a.autosave_block(), Some(AutoSaveBlock::Unsaved), "a new document has no file yet");
        a.run("file.open", json!({"path": md.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Typed "})).unwrap();
        a.run("file.save", json!({})).unwrap();
        assert!(a.autosave, "the preference stays on");
        assert_eq!(a.autosave_block(), Some(AutoSaveBlock::Format { name: "notes.md".into() }));
        assert!(!a.autosaves(), "the switch shows off");
        let why = a.autosave_block().map(|b| b.reason()).unwrap_or_default();
        assert!(why.contains("notes.md") && why.contains(".docx"), "{why}");
        let err = a.run("file.autosave", json!({"value": true})).unwrap_err();
        assert_eq!(err, why, "scripts are told why");
        assert!(a.run("file.autosave", json!({"value": false})).is_ok(), "turning it off always works");
        a.autosave = true;

        // Save As .docx (what the greyed switch offers) lifts the block.
        let docx = dir.join("notes.docx");
        a.run("file.saveAs", json!({"path": docx.to_string_lossy()})).unwrap();
        assert_eq!(a.autosave_block(), None);
        assert!(a.autosaves());
        a.run("text.insert", json!({"text": "More "})).unwrap();
        a.autosave_tick(now_ms() + 10_000.0);
        assert!(!a.session.dirty, "AutoSave wrote the .docx");
        assert_eq!(a.run("file.autosave", json!({})).unwrap(), json!({"value": false}), "toggling turns it off");
        assert!(!a.autosaves());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #176: in the browser saving is a download, so AutoSave can't keep a file up to date; the
    /// switch is blocked with a reason instead of being on while nothing is saved.
    #[test]
    fn autosave_is_unavailable_in_the_browser() {
        let services = Services { download: Some(Box::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        a.run("file.save", json!({"path": "draft.docx"})).unwrap();
        assert_eq!(a.autosave_block(), Some(AutoSaveBlock::Browser));
        assert!(!a.autosaves());
        let err = a.run("file.autosave", json!({"value": true})).unwrap_err();
        assert!(err.contains("browser"), "{err}");
    }

    /// Cancelling the Open picker (Mod+O) left the document as it was but turned AutoSave off.
    #[test]
    fn cancelling_open_keeps_autosave() {
        let dir = scratch("autosave-open");
        let path = dir.join("novel.docx");
        let mut a = typed();
        a.services.pick_open = Some(Box::new(|_| None));
        a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(a.autosaves());
        a.run("file.open", json!({})).unwrap();
        assert!(a.autosaves(), "the picker was cancelled; nothing was replaced");

        a.run("text.insert", json!({"text": " continues"})).unwrap();
        a.autosave_tick(now_ms() + 10_000.0);
        assert!(!a.session.dirty);
        let saved = wordcraft_engine::io::open_path(&path).unwrap();
        assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("chapter continues"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failing AutoSave used to be silent (`let _ =`) and retried every 2.5 s.
    #[test]
    fn autosave_failures_are_reported_once() {
        let dir = scratch("autosave-fail");
        let path = dir.join("gone").join("novel.docx");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut a = typed();
        a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        // The folder disappears (unplugged drive, network share gone).
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        a.run("text.insert", json!({"text": "More "})).unwrap();
        a.status_msg = None;
        a.autosave_tick(now_ms() + 10_000.0);
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(msg.contains("AutoSave"), "the failure shows in the status bar: {msg:?}");
        assert!(a.session.dirty);

        // Not retried until the user saves again.
        a.status_msg = None;
        a.autosave_tick(now_ms() + 20_000.0);
        assert!(a.status_msg.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Web Save of an opened .docm downloads a .docm, so its macros stay (Codex review, docm lane).
    #[test]
    fn web_save_keeps_the_opened_word_format() {
        let mut doc = wordcraft_doc::Document::new();
        doc.passthrough.insert("word/vbaProject.bin".into(), std::sync::Arc::new(vec![0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3]));
        doc.passthrough.insert("word/vbaData.xml".into(), std::sync::Arc::new(b"<wne:vbaSuppData/>".to_vec()));
        // The docx reader's list of parts the project relates to (wordcraft_docx VBA_RELATED).
        let related = "http://schemas.microsoft.com/office/2006/relationships/wordVbaData\tword/vbaData.xml\tapplication/vnd.ms-word.vbaData+xml\n";
        doc.passthrough.insert("wordcraft:vbaProject.related".into(), std::sync::Arc::new(related.as_bytes().to_vec()));
        let got: std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>> = Default::default();
        let sink = got.clone();
        let services = Services {
            download: Some(Box::new(move |n: &str, b: &[u8]| {
                sink.borrow_mut().push((n.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(Session::new(doc), services);
        for (opened, expect) in
            [("m.docm", "m.docm"), ("t.DOTM", "t.DOTM"), ("t.dotx", "t.dotx"), ("notes.odt", "notes.docx"), ("readme.txt", "readme.docx")]
        {
            a.session.path = Some(opened.into());
            a.run("file.save", json!({})).unwrap();
            let (name, bytes) = got.borrow_mut().pop().unwrap();
            assert_eq!(name, expect);
            let back = wordcraft_engine::io::open_bytes(&name, &bytes).unwrap();
            let macros = name.to_ascii_lowercase().ends_with('m');
            for part in ["word/vbaProject.bin", "word/vbaData.xml"] {
                assert_eq!(back.passthrough.contains_key(part), macros, "{opened}: {part}");
            }
        }
        // Save As with an explicit name is a conversion: a .docx can't hold macros.
        a.session.path = Some("m.docm".into());
        a.run("file.saveAs", json!({"path": "m.docx"})).unwrap();
        let (name, bytes) = got.borrow_mut().pop().unwrap();
        assert_eq!(name, "m.docx");
        assert!(!wordcraft_engine::io::open_bytes(&name, &bytes).unwrap().passthrough.contains_key("word/vbaProject.bin"));
    }

    #[test]
    fn dark_page_survives_restart() {
        let mut first = app();
        assert!(!first.session.view.dark_mode);
        first.run("view.darkMode", json!({"value": true})).unwrap();
        let saved = serde_json::to_vec(&first.prefs()).unwrap();

        let mut second = app();
        second.apply_prefs(serde_json::from_slice(&saved).unwrap());
        assert!(second.session.view.dark_mode);
    }

    /// #2526: the Navigation pane stays as the person left it at the next launch, and a
    /// `ui.json` from before the setting leaves it closed.
    #[test]
    fn navigation_pane_visibility_survives_restart() {
        let mut first = app();
        assert!(!first.session.view.nav_pane);
        first.session.view.nav_pane = true;
        let saved = serde_json::to_vec(&first.prefs()).unwrap();

        let mut second = app();
        second.apply_prefs(serde_json::from_slice(&saved).unwrap());
        assert!(second.session.view.nav_pane, "the pane is open again");

        let mut closed = app();
        closed.session.view.nav_pane = true;
        closed.session.view.nav_pane = false;
        let saved = serde_json::to_vec(&closed.prefs()).unwrap();
        let mut third = app();
        third.apply_prefs(serde_json::from_slice(&saved).unwrap());
        assert!(!third.session.view.nav_pane, "a closed pane stays closed");

        let mut old = app();
        old.apply_prefs(serde_json::from_str(r#"{"navTab": "headings"}"#).unwrap());
        assert!(!old.session.view.nav_pane, "no saved value: closed");
    }

    /// Task panes: two open panes on one edge sit side by side, the first outermost; a pane's ×
    /// runs its command; the Styles pane's place and float rect survive a restart (#1756).
    #[test]
    fn task_panes_share_an_edge_close_and_keep_the_styles_place() {
        use egui_kittest::kittest::Queryable;
        let bars = |h: &egui_kittest::Harness<'_, WordApp>| {
            let mut bars: Vec<egui::Rect> = h.get_all_by_label_contains("Move Pane: ").map(|n| n.rect()).collect();
            bars.sort_by(|a, b| a.left().total_cmp(&b.left()));
            bars
        };
        // A title bar's ▾ and × sit right of the bar, on its row.
        fn click_button(h: &egui_kittest::Harness<'_, WordApp>, bar: egui::Rect, label: &str) {
            let on_row = |r: egui::Rect| r.center().y > bar.top() && r.center().y < bar.bottom() && r.left() >= bar.right() - 1.0;
            let node = h.get_all_by_label(label).filter(|n| on_row(n.rect())).min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()));
            node.unwrap_or_else(|| panic!("no {label} beside the title bar")).click();
        }
        let mut a = app();
        a.run("view.stylesPane", json!({"value": true})).unwrap();
        a.run("view.commentsPane", json!({"value": true})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);
        let [comments, styles] = bars(&h)[..] else { panic!("two pane title bars") };
        assert!(comments.right() <= styles.left() + 1.0, "side by side");
        assert!(comments.left() > 720.0, "both on the right");
        // Default widths 290 and 250: Styles, opened first, is the outer one.
        assert!((comments.width() - styles.width() - 40.0).abs() < 2.0, "Styles is outermost: {comments:?} {styles:?}");

        click_button(&h, styles, "Close");
        h.run_steps(3);
        assert!(!h.state().session.view.styles_pane, "× ran view.stylesPane");
        assert!(h.state().session.view.comments_pane);
        assert_eq!(bars(&h).len(), 1);

        h.state_mut().run("view.stylesPane", json!({"value": true})).unwrap();
        h.run_steps(3);
        let styles = *bars(&h).last().unwrap();
        click_button(&h, styles, "Pane Options");
        h.run_steps(2);
        h.get_by_label("Float").click();
        h.run_steps(3);
        assert_eq!(h.state().session.view.styles_pane_dock, wordcraft_engine::StylesPaneDock::Floating);
        let rect = h.state().ui.styles_pane_float.expect("the float rect is kept");

        let saved = serde_json::to_vec(&h.state().prefs()).unwrap();
        let mut next = app();
        next.apply_prefs(serde_json::from_slice(&saved).unwrap());
        assert_eq!(next.session.view.styles_pane_dock, wordcraft_engine::StylesPaneDock::Floating);
        assert_eq!(next.ui.styles_pane_float, Some(rect));

        let mut junk = app();
        junk.apply_prefs(serde_json::from_str(r#"{"stylesPaneDock": "top", "stylesPaneFloat": [1, "x", 3, 4], "navPane": true}"#).unwrap());
        assert_eq!(junk.session.view.styles_pane_dock, wordcraft_engine::StylesPaneDock::Right, "junk reads as the default");
        assert_eq!(junk.ui.styles_pane_float, None);
        assert!(junk.session.view.nav_pane, "the rest of the file still loads");
    }

    /// A pointer drag on a pane's resize edge leaves no keyboard focus behind, so Ctrl and
    /// F-key shortcuts run right after it, docked or floating. A resize edge focused from the
    /// keyboard takes the arrow keys until Escape (#1756).
    #[test]
    fn shortcuts_run_after_a_pane_resize() {
        use egui_kittest::kittest::Queryable;
        type H = egui_kittest::Harness<'static, WordApp>;
        fn key(h: &mut H, key: egui::Key, modifiers: egui::Modifiers) {
            h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            h.run_steps(2);
        }
        fn drag(h: &mut H, from: egui::Pos2, by: egui::Vec2) {
            let button =
                |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            h.event(egui::Event::PointerMoved(from));
            h.step();
            h.event(button(from, true));
            h.step();
            for t in [0.5, 1.0] {
                h.event(egui::Event::PointerMoved(from + by * t));
                h.step();
            }
            h.event(button(from + by, false));
            h.run_steps(2);
        }
        // Ctrl+H opens Replace; Escape closes it again.
        fn replace_opens(h: &mut H) -> bool {
            key(h, egui::Key::H, egui::Modifiers::COMMAND);
            let opened = h.state().dialog.as_ref().map(|d| d.name()) == Some("replace");
            key(h, egui::Key::Escape, egui::Modifiers::NONE);
            opened && h.state().dialog.is_none()
        }
        let mut a = app();
        a.run("view.stylesPane", json!({"value": true})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);

        let edge = h.get_by_label("Resize Pane").rect();
        // On the pane's side of the edge: the document's scroll bar lies under its outer half.
        drag(&mut h, edge.center() + egui::vec2(1.5, 0.0), egui::vec2(-40.0, 0.0));
        let width = h.state().ui.styles_pane_width.unwrap_or_default();
        assert!((width - 290.0).abs() <= 2.0, "the drag widened the docked pane: {width}");
        assert!(replace_opens(&mut h), "Ctrl+H runs after the drag");

        h.get_by_label("Resize Pane").focus();
        h.run_steps(2);
        key(&mut h, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        assert!(h.state().ui.styles_pane_width.is_some_and(|w| w > width), "arrow keys move the focused edge");
        key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
        assert_eq!(h.ctx.memory(|m| m.focused()), None, "Escape leaves the edge");
        assert!(replace_opens(&mut h), "Ctrl+H runs after Escape");

        h.state_mut().run("view.stylesPaneDock", json!({"dock": "float"})).unwrap();
        h.run_steps(3);
        // The bottom-right grip.
        let corner = |r: &egui::Rect| r.max.x + r.max.y;
        let grip = h.get_all_by_label("Resize Pane").map(|n| n.rect()).max_by(|a, b| corner(a).total_cmp(&corner(b))).unwrap();
        let before = h.state().ui.styles_pane_float;
        drag(&mut h, grip.center(), egui::vec2(30.0, 30.0));
        assert_ne!(h.state().ui.styles_pane_float, before, "the drag resized the floating pane");
        assert!(replace_opens(&mut h), "Ctrl+H runs after the floating pane's resize");
    }

    #[test]
    fn check_spelling_survives_restart() {
        // #1792: unchecking Check spelling as you type was forgotten on restart.
        let mut first = app();
        assert!(first.session.view.proofing);
        first.run("file.options", json!({"checkSpelling": false})).unwrap();
        let saved = serde_json::to_vec(&first.prefs()).unwrap();

        let mut second = app();
        second.apply_prefs(serde_json::from_slice(&saved).unwrap());
        assert!(!second.session.view.proofing);
        // Settings saved before #1792 have no key: spelling stays on.
        second.apply_prefs(serde_json::from_str("{}").unwrap());
        assert!(second.session.view.proofing);
    }

    #[test]
    fn prefs_without_user_name_keep_default() {
        let mut a = app();
        let default = a.session.author.clone();
        a.apply_prefs(serde_json::from_str(r#"{"dark": true, "backstage": true}"#).unwrap());
        assert_eq!(a.session.author, default);
        assert_eq!(a.ui.theme, theme::Appearance::Dark);
        assert!(!a.ui.backstage);
    }

    /// #115: the old dark-mode switch migrates to Light/Dark; a saved System stays System, and the
    /// old key is never written back.
    #[test]
    fn interface_theme_migrates_and_survives_a_restart() {
        use theme::Appearance;
        for (saved, want) in [(r#"{"dark": false}"#, Appearance::Light), (r#"{"dark": true}"#, Appearance::Dark), ("{}", Appearance::System)] {
            let mut a = app();
            a.apply_prefs(serde_json::from_str(saved).unwrap());
            assert_eq!(a.ui.theme, want, "{saved}");
        }
        let mut a = app();
        a.run("ui.theme", json!({"value": "System"})).unwrap();
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""theme":"system""#) && !saved.contains(r#""dark":"#), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.ui.theme, Appearance::System);
        // An unknown saved value keeps the rest of the preferences.
        let odd: UiState = serde_json::from_str(r#"{"theme": "sepia", "tab": "Insert"}"#).unwrap();
        assert_eq!((odd.theme, odd.tab.as_str()), (Appearance::System, "Insert"));
    }

    /// #1861: the registered Interface Theme command drives the same setting as Options:
    /// searchable in the registry, settable, toggling without a value, and surviving a restart.
    #[test]
    fn interface_theme_command_sets_toggles_and_persists() {
        use theme::Appearance;
        let mut a = app();
        let spec = a.session.registry.get("view.interfaceTheme").expect("registered and searchable");
        assert_eq!((spec.label, spec.location), ("Interface Theme", "View › Dark Mode"));
        a.run("view.interfaceTheme", json!({"value": "dark"})).unwrap();
        assert_eq!(a.ui.theme, Appearance::Dark);
        a.run("view.interfaceTheme", json!({"value": "system"})).unwrap();
        assert_eq!(a.ui.theme, Appearance::System);
        a.run("view.interfaceTheme", json!({"value": "light"})).unwrap();
        a.run("view.interfaceTheme", json!({})).unwrap();
        assert_eq!(a.ui.theme, Appearance::Dark, "no value toggles light to dark");
        a.run("view.interfaceTheme", json!({})).unwrap();
        assert_eq!(a.ui.theme, Appearance::Light, "and back");
        assert!(a.run("view.interfaceTheme", json!({"value": "sepia"})).is_err());
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.ui.theme, Appearance::Light);
    }

    /// #475, #822: the interface size is kept between runs; junk saved values read as Auto and
    /// out-of-range ones are clamped, without losing the other preferences.
    #[test]
    fn interface_size_survives_a_restart_and_clamps_junk() {
        let mut a = app();
        assert_eq!(a.run("ui.interfaceSize", json!({})).unwrap(), json!({"auto": true, "percent": 100}));
        assert_eq!(a.run("ui.interfaceSize", json!({"percent": 150})).unwrap(), json!({"auto": false, "percent": 150}));
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""uiScale":150"#) && !saved.contains("interfaceSize"), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.ui.interface_size.percent(), Some(150));
        for (raw, want) in [
            ("1000", Some(300)),
            ("10", Some(50)),
            ("-5", Some(50)),
            ("124.6", Some(125)),
            (r#""auto""#, None),
            (r#""big""#, None),
            ("null", None),
            ("[1]", None),
        ] {
            let ui: UiState = serde_json::from_str(&format!(r#"{{"uiScale": {raw}, "tab": "Insert"}}"#)).unwrap();
            assert_eq!((ui.interface_size.percent(), ui.tab.as_str()), (want, "Insert"), "{raw}");
        }
        assert_eq!(a.run("ui.interfaceSize", json!({"percent": 500})).unwrap()["percent"], 300);
        assert!(a.run("ui.interfaceSize", json!({"percent": "huge"})).is_err());
        assert!(a.run("ui.interfaceSize", json!({"percent": null})).is_err());
        assert_eq!(a.ui.interface_size.percent(), Some(300), "a rejected value keeps the setting");
        assert_eq!(a.run("ui.interfaceSize", json!({"value": "125%"})).unwrap()["percent"], 125);
        assert_eq!(a.run("ui.interfaceSize", json!({"value": "Auto"})).unwrap()["auto"], true);
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""uiScale":"auto""#), "{saved}");
    }

    /// #822: a `ui.json` from before saved `interfaceSize` on top of the display scale. Its
    /// default, 100, becomes Auto (the same size as before); a size picked on purpose becomes the
    /// absolute size it drew at, once the first frame says the display's scale: 125 % on a 2×
    /// display drew at 250 % and stays there. A valid `uiScale` beside it wins.
    #[test]
    fn interface_size_from_before_822_migrates() {
        let frame = |a: &mut WordApp, native: f32| {
            let ctx = egui::Context::default();
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
            ctx.run_ui(input, |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
        };
        for (json, native, want) in [
            (r#"{"interfaceSize": 100}"#, 2.0, None),
            (r#"{"interfaceSize": 100.2}"#, 2.0, None),
            (r#"{"interfaceSize": "big"}"#, 2.0, None),
            (r#"{"interfaceSize": 125}"#, 2.0, Some(250)),
            (r#"{"interfaceSize": 150}"#, 1.0, Some(150)),
            (r#"{"interfaceSize": 80}"#, 1.5, Some(120)),
            (r#"{"interfaceSize": 200}"#, 2.0, Some(300)),
            (r#"{"interfaceSize": 80}"#, 0.5, Some(50)),
            (r#"{"interfaceSize": 150, "uiScale": 90}"#, 2.0, Some(90)),
            (r#"{"interfaceSize": 150, "uiScale": "junk"}"#, 2.0, Some(300)),
        ] {
            let mut a = app();
            a.apply_prefs(serde_json::from_str(json).unwrap());
            frame(&mut a, native);
            assert_eq!(a.ui.interface_size.percent(), want, "{json} on a {native}× display");
            assert!(!serde_json::to_string(&a.prefs()).unwrap().contains("interfaceSize"), "{json}");
        }
        // Converted once: a later move to another display keeps the absolute size.
        let mut a = app();
        a.apply_prefs(serde_json::from_str(r#"{"interfaceSize": 125}"#).unwrap());
        frame(&mut a, 2.0);
        frame(&mut a, 1.0);
        assert_eq!(a.ui.interface_size.percent(), Some(250));
        // A size the user picks before the first frame isn't overwritten by the old one.
        let mut a = app();
        a.apply_prefs(serde_json::from_str(r#"{"interfaceSize": 125}"#).unwrap());
        a.run("ui.interfaceSize", json!({"percent": 100})).unwrap();
        frame(&mut a, 2.0);
        assert_eq!(a.ui.interface_size.percent(), Some(100));
    }

    /// #822: on a display the system reports at 200 % (a 1080p screen under a bare window manager,
    /// where winit guesses the scale from the monitor's DPI), "100%" draws at a true 100 %: one
    /// screen pixel per point. Auto keeps the display's scale and says what it draws at. Document
    /// zoom is a separate setting.
    #[test]
    fn interface_size_is_absolute_and_auto_keeps_the_system_scale() {
        let ctx = egui::Context::default();
        let mut a = app();
        let doc_zoom = a.session.view.zoom;
        let frame = |a: &mut WordApp, native: f32| {
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
            ctx.run_ui(input, |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
        };
        let ppp = |a: &mut WordApp, native: f32| {
            // The new factor applies from the next pass.
            frame(a, native);
            frame(a, native);
            ctx.pixels_per_point()
        };
        assert_eq!(ppp(&mut a, 2.0), 2.0, "Auto keeps the display's scale");
        assert_eq!(a.run("ui.interfaceSize", json!({})).unwrap(), json!({"auto": true, "percent": 200}));
        a.run("ui.interfaceSize", json!({"percent": 100})).unwrap();
        assert_eq!(ppp(&mut a, 2.0), 1.0, "100% is one screen pixel per point on a 2× display");
        for native in [1.0, 1.25, 1.5, 2.0, 3.0] {
            for pct in [75u16, 100, 150, 200, 300] {
                a.run("ui.interfaceSize", json!({"percent": pct})).unwrap();
                let got = ppp(&mut a, native);
                assert!((got - f32::from(pct) / 100.0).abs() < 1e-4, "{pct}% on a {native}× display drew at {got}");
                assert_eq!(a.run("ui.interfaceSize", json!({})).unwrap()["percent"], pct, "{pct}% on a {native}× display");
            }
        }
        a.run("ui.interfaceSize", json!({"value": "auto"})).unwrap();
        assert_eq!(ppp(&mut a, 1.5), 1.5);
        assert_eq!(a.session.view.zoom, doc_zoom, "document zoom is a separate setting");
    }

    /// #822: a display scale egui doesn't know or that is nonsense counts as 1×.
    #[test]
    fn interface_size_survives_an_unusable_display_scale() {
        use theme::InterfaceSize;
        let size = InterfaceSize::from_percent(125.0).unwrap();
        for native in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -2.0] {
            assert_eq!((size.zoom_factor(native), size.drawn_percent(native)), (1.25, 125), "{native}");
            assert_eq!((InterfaceSize::AUTO.zoom_factor(native), InterfaceSize::AUTO.drawn_percent(native)), (1.0, 100), "{native}");
        }
        assert_eq!(InterfaceSize::AUTO.drawn_percent(f32::MAX), u16::MAX);
    }

    /// #115: System follows the OS appearance (light when unknown); the manual choices ignore it.
    #[test]
    fn interface_theme_resolves_against_the_os_appearance() {
        use egui::Theme::{Dark, Light};
        use theme::Appearance;
        let cases = [(Appearance::System, None, false), (Appearance::System, Some(Light), false), (Appearance::System, Some(Dark), true)];
        for (setting, os, dark) in cases.into_iter().chain([(Appearance::Light, Some(Dark), false), (Appearance::Dark, Some(Light), true)]) {
            assert_eq!(setting.is_dark(os), dark, "{setting:?} with the OS at {os:?}");
        }
        let mut a = app();
        assert!(a.run("ui.theme", json!({"value": "purple"})).is_err());
        assert_eq!(a.run("ui.theme", json!({"value": "dark"})).unwrap()["theme"], "dark");
        assert_eq!(a.run("ui.dark", json!({})).unwrap()["theme"], "light", "ui.dark toggles the manual choice");
        assert_eq!(a.run("ui.theme", json!({})).unwrap()["theme"], "light");
    }

    /// One app frame with the OS appearance egui reports; returns the window commands it sent.
    fn theme_frame(ctx: &egui::Context, a: &mut WordApp, os: Option<egui::Theme>) -> Vec<egui::ViewportCommand> {
        let input = egui::RawInput { system_theme: os, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.logic(ui.ctx()));
        let commands = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        out.drop_without_applying_deltas();
        commands
    }

    /// #980: F11 and View › Full Screen (`view.fullScreen`) take the window to full screen; a
    /// second F11 or Escape brings it back.
    #[test]
    fn f11_and_escape_toggle_full_screen() {
        use egui::ViewportCommand::Fullscreen;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        // Run frames and return the full-screen requests the window got.
        let sent = |h: &mut egui_kittest::Harness<WordApp>| {
            let mut out = Vec::new();
            for _ in 0..2 {
                h.step();
                out.extend(h.output().viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, Fullscreen(_))).cloned());
            }
            out
        };
        assert_eq!(sent(&mut h), vec![], "a normal window to start with");
        h.event(key(egui::Key::F11));
        assert_eq!(sent(&mut h), vec![Fullscreen(true)], "F11");
        assert!(h.state().session.view.full_screen);
        h.event(key(egui::Key::F11));
        assert_eq!(sent(&mut h), vec![Fullscreen(false)], "F11 again");
        assert!(!h.state().session.view.full_screen);
        // The command (ribbon, command search, agents), then Escape.
        assert_eq!(h.state_mut().run("view.fullScreen", json!({})).unwrap(), json!({"value": true}));
        assert_eq!(sent(&mut h), vec![Fullscreen(true)], "the command");
        h.event(key(egui::Key::Escape));
        assert_eq!(sent(&mut h), vec![Fullscreen(false)], "Escape");
        // Escape in a normal window doesn't touch the window.
        h.event(key(egui::Key::Escape));
        assert_eq!(sent(&mut h), vec![]);
        // Escape closing a dialog leaves full screen alone.
        h.state_mut().run("view.fullScreen", json!({"value": true})).unwrap();
        h.state_mut().run("ui.dialog", json!({"name": "zoom"})).unwrap();
        assert_eq!(sent(&mut h), vec![Fullscreen(true)]);
        h.event(key(egui::Key::Escape));
        assert_eq!(sent(&mut h), vec![]);
        assert!(h.state().dialog.is_none(), "Escape closed the dialog");
        assert!(h.state().session.view.full_screen, "and only that");
    }

    /// #2864: the View tab's Full Screen button toggles out of full screen as well as in.
    #[test]
    fn the_full_screen_button_toggles_out() {
        use egui::ViewportCommand::Fullscreen;
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        h.state_mut().run("ui.tab", json!({ "tab": "View" })).unwrap();
        for _ in 0..3 {
            h.step();
        }
        // The View tab's Window group folds into a drop-down when the window is narrow (#662),
        // and the button then lives in that menu: open the group when it is folded.
        fn full_screen<'a>(h: &'a egui_kittest::Harness<'a, WordApp>) -> Option<egui_kittest::Node<'a>> {
            h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button))
                .into_iter()
                .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.replace('\n', " ").contains("Full Screen")))
        }
        if full_screen(&h).is_none() {
            h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button))
                .into_iter()
                .find(|n| n.accesskit_node().label().as_deref() == Some("Window"))
                .unwrap_or_else(|| panic!("the View tab has a Window group"))
                .click();
            for _ in 0..2 {
                h.step();
            }
        }
        // The click's frames carry the viewport command, so collect while clicking.
        let click_full_screen = |h: &mut egui_kittest::Harness<WordApp>| {
            let mut out = Vec::new();
            full_screen(h).unwrap_or_else(|| panic!("the Full Screen button is on the View tab")).click();
            for _ in 0..2 {
                h.step();
                out.extend(h.output().viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, Fullscreen(_))).cloned());
            }
            out
        };
        let clicked = click_full_screen(&mut h);
        assert_eq!(clicked, vec![Fullscreen(true)], "the button enters full screen");
        assert!(h.state().session.view.full_screen);
        let clicked = click_full_screen(&mut h);
        assert_eq!(clicked, vec![Fullscreen(false)], "the button leaves full screen again");
        assert!(!h.state().session.view.full_screen);
    }

    /// The Full Screen button tells assistive technology what the command can do: it is enabled
    /// exactly when the command is, so the browser build (which keeps full screen to itself)
    /// does not announce a control that cannot be activated.
    #[test]
    fn the_full_screen_button_reports_the_commands_own_enabled_state() {
        use egui_kittest::kittest::{NodeT, Queryable};
        use wordcraft_engine::{CommandSpec, Registry};

        // The browser build's reason for refusing full screen, forced here so the test says
        /// something on every platform.
        fn unavailable(_: &wordcraft_engine::Session) -> Option<&'static str> {
            Some("Use the browser's full screen (F11)")
        }

        let mut a = app();
        let specs: Vec<CommandSpec> = a
            .session
            .registry
            .all()
            .iter()
            .map(|s| {
                if s.id == "view.fullScreen" {
                    CommandSpec {
                        id: s.id,
                        label: s.label,
                        location: s.location,
                        shortcut: s.shortcut,
                        params: s.params,
                        mutates: s.mutates,
                        enabled: unavailable,
                        run: s.run,
                    }
                } else {
                    s.clone()
                }
            })
            .collect();
        a.session.registry = std::sync::Arc::new(Registry::new(specs));

        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1920.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        h.state_mut().run("ui.tab", json!({ "tab": "View" })).unwrap();
        for _ in 0..3 {
            h.step();
        }
        let node = h
            .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button))
            .into_iter()
            .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.replace('\n', " ").contains("Full Screen")))
            .expect("the Full Screen button is on the View tab");
        assert!(!crate::widgets::enabled(h.state(), "view.fullScreen"), "the command is unavailable, as the test arranged it",);
        assert!(node.accesskit_node().is_disabled(), "a control the command refuses is announced as disabled",);
    }

    /// #980: in full screen, Escape with text selected collapses the selection (as in a normal
    /// window) and leaves full screen alone; the next Escape, with nothing selected, leaves it.
    #[test]
    fn escape_collapses_a_selection_before_leaving_full_screen() {
        use egui::ViewportCommand::Fullscreen;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let sent = |h: &mut egui_kittest::Harness<WordApp>| {
            let mut out = Vec::new();
            for _ in 0..2 {
                h.step();
                out.extend(h.output().viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, Fullscreen(_))).cloned());
            }
            out
        };
        h.state_mut().canvas.focused = true;
        h.state_mut().run("text.insert", json!({"text": "Hello world"})).unwrap();
        h.state_mut().run("select.all", json!({})).unwrap();
        h.state_mut().run("view.fullScreen", json!({"value": true})).unwrap();
        assert_eq!(sent(&mut h), vec![Fullscreen(true)]);
        assert!(!h.state().session.sel.is_collapsed());
        h.event(esc.clone());
        assert_eq!(sent(&mut h), vec![], "Escape with a selection stays in full screen");
        assert!(h.state().session.sel.is_collapsed(), "and collapses the selection");
        assert!(h.state().session.view.full_screen);
        h.event(esc);
        assert_eq!(sent(&mut h), vec![Fullscreen(false)], "Escape with nothing selected leaves");
        assert!(!h.state().session.view.full_screen);
    }

    /// #980: the window leaving full screen by other means (the window manager, the macOS title
    /// bar button) turns View › Full Screen off, and entering it turns it on.
    #[test]
    fn full_screen_follows_the_window() {
        let ctx = egui::Context::default();
        let mut a = app();
        let frame = |a: &mut WordApp, fullscreen: Option<bool>| {
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().fullscreen = fullscreen;
            let mut out = ctx.run_ui(input, |ui| a.sync_full_screen(ui.ctx()));
            let sent = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
            out.drop_without_applying_deltas();
            sent.into_iter().filter(|c| matches!(c, egui::ViewportCommand::Fullscreen(_))).collect::<Vec<_>>()
        };
        assert!(frame(&mut a, Some(false)).is_empty());
        let sent = frame(&mut a, Some(true));
        assert!(sent.is_empty(), "the window went full screen: nothing to ask: {sent:?}");
        assert!(a.session.view.full_screen);
        assert!(frame(&mut a, Some(false)).is_empty());
        assert!(!a.session.view.full_screen, "and came back");
        // Focus, then the system takes the window out of full screen: Focus ends with it.
        a.run("view.focus", json!({})).unwrap();
        assert_eq!(frame(&mut a, Some(false)), vec![egui::ViewportCommand::Fullscreen(true)]);
        assert!(frame(&mut a, Some(true)).is_empty());
        assert!(frame(&mut a, Some(false)).is_empty());
        assert!(!a.session.view.focus_mode, "Focus ended with the window's full screen");
        // Asked for: sent once; the window is slow to report it, which doesn't undo the request.
        a.run("view.fullScreen", json!({})).unwrap();
        assert_eq!(frame(&mut a, Some(false)), vec![egui::ViewportCommand::Fullscreen(true)]);
        assert!(frame(&mut a, Some(false)).is_empty());
        assert!(a.session.view.full_screen);
        assert!(frame(&mut a, Some(true)).is_empty());
        assert!(a.session.view.full_screen);
    }

    /// #311: with System, egui's theme preference stays System, so the native window is never
    /// pinned to a concrete appearance (macOS would stop reporting OS changes), and our palette
    /// follows each OS appearance egui reports.
    #[test]
    fn system_theme_follows_the_os_without_pinning_the_window() {
        use egui::{SystemTheme, Theme, ThemePreference, ViewportCommand};
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("ui.theme", json!({"value": "system"})).unwrap();
        let mut sent = Vec::new();
        for (os, dark) in [(Some(Theme::Light), false), (Some(Theme::Dark), true), (Some(Theme::Light), false), (None, false)] {
            sent.extend(theme_frame(&ctx, &mut a, os));
            assert_eq!(ctx.options(|o| o.theme_preference), ThemePreference::System, "OS at {os:?}");
            let want = if dark { theme::Tokens::dark() } else { theme::Tokens::light() };
            assert_eq!(theme::Tokens::get(&ctx).dark, dark, "OS at {os:?}");
            // Our palette, not egui's default style for that theme.
            assert_eq!(ctx.global_style().visuals.panel_fill, want.ribbon, "OS at {os:?}");
            assert_eq!(a.ui_is_dark(), dark, "OS at {os:?}");
        }
        assert!(sent.contains(&ViewportCommand::SetTheme(SystemTheme::SystemDefault)), "{sent:?}");
        assert!(!sent.iter().any(|c| matches!(c, ViewportCommand::SetTheme(SystemTheme::Light | SystemTheme::Dark))), "{sent:?}");
        // A manual choice pins the window, whatever the OS says.
        a.run("ui.theme", json!({"value": "dark"})).unwrap();
        let sent = theme_frame(&ctx, &mut a, Some(Theme::Light));
        assert!(sent.contains(&ViewportCommand::SetTheme(SystemTheme::Dark)), "{sent:?}");
        assert!(theme::Tokens::get(&ctx).dark);
    }

    /// #537: where winit reports no OS theme (Linux), System follows the desktop portal's answer
    /// and pins the window to it, so GNOME's client-side title bar matches the interface and
    /// follows a theme switch made while the app runs.
    #[test]
    fn system_theme_pins_the_title_bar_to_the_desktop_answer() {
        use egui::{SystemTheme, ViewportCommand};
        let ctx = egui::Context::default();
        let mut a = app();
        a.run("ui.theme", json!({"value": "system"})).unwrap();
        for dark in [false, true, false] {
            a.desktop_dark = Some(dark);
            let sent = theme_frame(&ctx, &mut a, None);
            let want = if dark { SystemTheme::Dark } else { SystemTheme::Light };
            assert!(sent.contains(&ViewportCommand::SetTheme(want)), "desktop dark {dark}: {sent:?}");
            assert_eq!(theme::Tokens::get(&ctx).dark, dark);
        }
    }

    /// #312: Dark page darkens only the pages; the interface keeps following its own setting.
    #[test]
    fn dark_page_leaves_the_interface_theme_alone() {
        use egui::Theme::{Dark, Light};
        for (setting, os, dark) in [
            ("system", Some(Light), false),
            ("system", None, false),
            ("light", Some(Dark), false),
            ("system", Some(Dark), true),
            ("dark", Some(Light), true),
        ] {
            let ctx = egui::Context::default();
            let mut a = app();
            a.run("ui.theme", json!({"value": setting})).unwrap();
            a.run("view.darkMode", json!({"value": true})).unwrap();
            theme_frame(&ctx, &mut a, os);
            assert!(a.session.view.dark_mode, "{setting} with the OS at {os:?}");
            assert_eq!(theme::Tokens::get(&ctx).dark, dark, "{setting} with the OS at {os:?}");
            assert_eq!(a.run("ui.theme", json!({})).unwrap()["dark"], dark, "{setting} with the OS at {os:?}");
        }
    }

    #[test]
    fn word_count_setting_survives_a_restart() {
        let mut a = app();
        a.session.run("review.wordCount", &json!({"includeTextBoxes": false})).unwrap();
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""editing":{"countNotes":false,"#), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert!(!b.session.prefs.count_notes);
    }

    #[test]
    fn prefs_without_editing_keep_its_defaults() {
        let mut a = app();
        a.apply_prefs(serde_json::from_str(r#"{"tab": "Insert", "dark": true}"#).unwrap());
        // The tab is not restored (Word opens on Home), but forcing it costs nothing else.
        assert_eq!((a.ui.tab.as_str(), a.ui.theme), (DEFAULT_TAB, theme::Appearance::Dark));
        assert!(a.session.prefs.count_notes, "text boxes and notes count by default");
    }

    /// Word opens on Home every launch, so the tab `ui.json` holds is not restored; the rest of the
    /// preferences come back as saved.
    #[test]
    fn startup_opens_on_home_not_the_saved_tab() {
        let mut a = app();
        a.ui.tab = "Review".into();
        a.ui.theme = theme::Appearance::Dark;
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        assert!(saved.contains(r#""tab":"Review""#), "{saved}");
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.ui.tab, DEFAULT_TAB, "the saved tab is not restored");
        assert_eq!(b.ui.theme, theme::Appearance::Dark, "the other preferences are");
    }

    /// Going File and back to the editor returns to the tab that was showing, contextual tabs
    /// included (Word behaves the same way).
    #[test]
    fn going_to_file_and_back_keeps_the_tab() {
        let mut a = app();
        a.ui_command("ui.tab", &json!({"tab": "Table Design"})).unwrap().unwrap();
        a.ui_command("ui.tab", &json!({"tab": "File"})).unwrap().unwrap();
        assert!(a.ui.backstage, "File opens Backstage, leaving the tab alone");
        a.ui_command("ui.tab", &json!({"tab": "Table Design"})).unwrap().unwrap();
        assert!(!a.ui.backstage && a.ui.tab == "Table Design");
    }

    fn png_bytes(c: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(20, 10, |_, _| image::Rgba(c));
        let mut b = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    }

    fn insert_picture(a: &mut WordApp, c: [u8; 4]) {
        let data = wordcraft_engine::cmd::insert::base64_encode(&png_bytes(c));
        a.session.run("insert.picture", &json!({"data": data})).unwrap();
    }

    fn object_count(a: &mut WordApp) -> usize {
        a.session.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(|x| x.len()).unwrap_or(0)
    }

    /// #965: a picture or icon the person inserts is selected, with handles, as in Word; one a
    /// script inserts (control channel, MCP bridge) leaves the caret after it, and typing keeps it.
    #[test]
    fn inserted_picture_is_selected_for_the_user_not_for_scripts() {
        let data = wordcraft_engine::cmd::insert::base64_encode(&png_bytes([200, 30, 30, 255]));
        for (id, params) in [("insert.picture", json!({"data": data})), ("insert.icon", json!({"id": "sun"}))] {
            let mut a = app();
            a.run(id, params.clone()).unwrap();
            assert!(crate::objects::selected(&a).is_some(), "{id}: the person's picture is selected");
            let mut a = app();
            a.execute(id, params).unwrap();
            assert!(a.session.sel.is_collapsed() && crate::objects::selected(&a).is_none(), "{id}: a script gets the caret after it");
            a.execute("text.newParagraph", json!({})).unwrap();
            a.execute("text.insert", json!({"text": "cap"})).unwrap();
            assert_eq!(object_count(&mut a), 1, "{id}");
        }
    }

    /// A picture pasted in the browser lands only when the page is the edit target (#1349);
    /// dropped or opened files always land.
    #[test]
    fn pasted_picture_inserts_only_when_the_page_is_the_edit_target() {
        let mut a = app();
        let q: Inbox = Inbox::default();
        a.services.pasted = Some(q.clone());
        let push = |a: &mut WordApp, c| {
            // Collapse after the last picture so an insertion adds one instead of replacing it.
            a.session.run("select.collapse", &json!({"end": true})).unwrap();
            q.lock().unwrap().push(("pasted.png".into(), png_bytes(c)));
        };
        // The page has focus: inserted once.
        a.canvas.focused = true;
        push(&mut a, [200, 30, 30, 255]);
        a.drain_pasted();
        assert_eq!(object_count(&mut a), 1);
        assert!(q.lock().unwrap().is_empty(), "the queue is drained, so no second insertion");
        a.drain_pasted();
        assert_eq!(object_count(&mut a), 1);
        // A dialog is open.
        a.run("ui.dialog", json!({"name": "find"})).unwrap();
        assert!(a.dialog.is_some());
        push(&mut a, [1, 2, 3, 255]);
        a.drain_pasted();
        assert_eq!(object_count(&mut a), 1);
        a.dialog = None;
        // The Backstage is open.
        a.ui.backstage = true;
        push(&mut a, [1, 2, 3, 255]);
        a.drain_pasted();
        assert_eq!(object_count(&mut a), 1);
        assert!(q.lock().unwrap().is_empty(), "ignored pictures are dropped, not kept for later");
        a.ui.backstage = false;
        // A dropped file still inserts with the page unfocused.
        a.canvas.focused = false;
        a.session.run("select.collapse", &json!({"end": true})).unwrap();
        a.services.inbox = Some(Inbox::default());
        a.services.inbox.as_ref().unwrap().lock().unwrap().push(("drop.png".into(), png_bytes([5, 5, 5, 255])));
        a.drain_inbox();
        assert_eq!(object_count(&mut a), 2);
    }

    fn paste_harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        h
    }

    /// Through the real frame loop (#1349): a paste onto the focused page inserts once; the
    /// focused page must not look like a text field to the guard.
    #[test]
    fn a_paste_onto_the_focused_page_inserts_one_picture() {
        let mut a = app();
        let q: Inbox = Inbox::default();
        a.services.pasted = Some(q.clone());
        let mut h = paste_harness(a);
        assert!(h.state().canvas.focused);
        q.lock().unwrap().push(("pasted.png".into(), png_bytes([9, 9, 9, 255])));
        h.step();
        assert_eq!(object_count(h.state_mut()), 1);
        h.step();
        assert_eq!(object_count(h.state_mut()), 1, "once");
    }

    /// A real text field with focus (the Find box) takes the input: the picture is dropped.
    #[test]
    fn a_paste_with_the_find_dialog_open_is_ignored() {
        let mut a = app();
        let q: Inbox = Inbox::default();
        a.services.pasted = Some(q.clone());
        a.run("ui.dialog", json!({"name": "find"})).unwrap();
        let mut h = paste_harness(a);
        h.ctx.memory_mut(|m| m.request_focus(egui::Id::unique("find_what")));
        h.step();
        h.step();
        assert!(!h.state().canvas.focused);
        q.lock().unwrap().push(("pasted.png".into(), png_bytes([9, 9, 9, 255])));
        h.step();
        assert_eq!(object_count(h.state_mut()), 0);
        assert!(q.lock().unwrap().is_empty());
    }

    /// A text field outside any dialog (the Navigation pane's search) with focus takes the input:
    /// the page is not focused, and the picture is dropped (#1349).
    #[test]
    fn a_paste_with_a_pane_text_field_focused_is_ignored() {
        let mut a = app();
        let q: Inbox = Inbox::default();
        a.services.pasted = Some(q.clone());
        a.run("view.navigationPane", json!({"value": true})).unwrap();
        let mut h = paste_harness(a);
        // Focus the search box for real (as Ctrl+F does), once the page has settled.
        h.state_mut().canvas.want_focus = false;
        h.state_mut().nav_search_focus = true;
        h.step();
        h.step();
        let app = h.state();
        assert!(app.dialog.is_none() && !app.ui.backstage, "no dialog, no Backstage");
        assert!(!app.canvas.focused, "the search box has the focus, not the page");
        q.lock().unwrap().push(("pasted.png".into(), png_bytes([9, 9, 9, 255])));
        h.step();
        assert_eq!(object_count(h.state_mut()), 0);
        assert!(q.lock().unwrap().is_empty());
    }

    /// A pending Change Picture replaces the selected image instead of inserting (#147).
    #[test]
    fn change_picture_replaces_instead_of_inserting() {
        let mut a = app();
        a.services.inbox = Some(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        insert_picture(&mut a, [200, 30, 30, 255]);
        assert_eq!(object_count(&mut a), 1);
        let before = a.selected_picture_media().unwrap();
        a.change_picture_target = a.selected_picture_media();
        a.services.inbox.as_ref().unwrap().lock().unwrap().push(("new.png".into(), png_bytes([30, 200, 30, 255])));
        a.drain_inbox();
        assert_eq!(object_count(&mut a), 1);
        assert_ne!(a.selected_picture_media().unwrap(), before);
        assert!(a.change_picture_target.is_none());
    }

    /// Several images dropped together all stay in the document (#1592).
    #[test]
    fn several_dropped_images_are_all_inserted() {
        let mut a = app();
        a.services.inbox = Some(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        {
            let mut q = a.services.inbox.as_ref().unwrap().lock().unwrap();
            q.push(("a.png".into(), png_bytes([200, 30, 30, 255])));
            q.push(("b.png".into(), png_bytes([30, 200, 30, 255])));
        }
        a.drain_inbox();
        assert_eq!(object_count(&mut a), 2);
    }

    /// With a Drawing Canvas selected, every image of the drop goes into it (#1592).
    #[test]
    fn several_dropped_images_all_join_a_selected_canvas() {
        let mut a = app();
        a.services.inbox = Some(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        a.session.run("insert.canvas", &json!({})).unwrap();
        {
            let mut q = a.services.inbox.as_ref().unwrap().lock().unwrap();
            q.push(("a.png".into(), png_bytes([200, 30, 30, 255])));
            q.push(("b.png".into(), png_bytes([30, 200, 30, 255])));
        }
        a.drain_inbox();
        let members = a.session.run("canvas.member", &json!({})).unwrap();
        assert_eq!(members["members"].as_array().map(|m| m.len()), Some(2));
    }

    /// Without a pending change, inbox images insert; stale targets clear on selection change.
    #[test]
    fn insert_without_pending_adds_and_stale_target_clears() {
        let mut a = app();
        a.services.inbox = Some(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        insert_picture(&mut a, [200, 30, 30, 255]);
        a.session.run("select.collapse", &json!({"end": true})).unwrap();
        a.services.inbox.as_ref().unwrap().lock().unwrap().push(("second.png".into(), png_bytes([30, 30, 200, 255])));
        a.drain_inbox();
        assert_eq!(object_count(&mut a), 2);
        // Stale target (another picture's key) clears instead of replacing the new selection.
        a.change_picture_target = Some("m0000-gone.png".into());
        a.clear_stale_change_picture();
        assert!(a.change_picture_target.is_none());
    }

    /// `ui.changePicture` needs a selected picture; cancelling the picker clears the target.
    #[test]
    fn change_picture_guards_and_cancel_clears() {
        let mut a = app();
        assert!(a.run("ui.changePicture", json!({})).is_err());
        assert!(a.change_picture_target.is_none());
        insert_picture(&mut a, [200, 30, 30, 255]);
        // No pickers in tests, so the picker "cancels" and the target clears.
        assert!(a.run("ui.changePicture", json!({})).is_ok());
        assert!(a.change_picture_target.is_none());
    }

    /// Any unrelated action cancels a pending Change Picture (#147 web-picker cancel case).
    #[test]
    fn unrelated_command_clears_change_picture_target() {
        let mut a = app();
        insert_picture(&mut a, [200, 30, 30, 255]);
        a.change_picture_target = a.selected_picture_media();
        assert!(a.change_picture_target.is_some());
        let _ = a.run("format.bold", json!({}));
        assert!(a.change_picture_target.is_none());
    }

    fn dialog_name(a: &WordApp) -> Option<&'static str> {
        a.dialog.as_ref().map(dialogs::Dialog::name)
    }

    /// #778: Edit Recipient List shows every record with the list's filter and sort, and its
    /// Filter and Sort section maps to `mailings.filterRecipients`.
    #[test]
    fn recipient_list_dialog_carries_the_filter() {
        let mut a = app();
        a.run("mailings.recipients", json!({"csv": "Name,City\nAda,London\nAlan,Wilmslow\nGrace,London"})).unwrap();
        a.run(
            "mailings.filterRecipients",
            json!({"rules": [{"field": "City", "op": "equal", "value": "London"}], "sort": [{"field": "Name", "descending": true}]}),
        )
        .unwrap();
        a.run("mailings.editRecipients", json!({})).unwrap();
        let Some(dialogs::Dialog::RecipientList { rows, filter, fields, .. }) = &a.dialog else { panic!() };
        assert_eq!(rows.len(), 3, "every record, not only the filtered ones");
        assert!(filter.open);
        let params = filter.params(fields);
        // A rule or sort key on a field renamed or removed in the table drops (#1287).
        assert_eq!(filter.params(&["Name".to_string()]), json!({"rules": [], "sort": [{"field": "Name", "descending": true}]}));
        assert_eq!(params["rules"], json!([{"field": "City", "op": "equal", "value": "London", "join": "and"}]));
        assert_eq!(params["sort"], json!([{"field": "Name", "descending": true}]));
        // Applying the dialog's filter to a reloaded list gives the same records.
        a.run("mailings.recipients", json!({"csv": "Name,City\nAda,London\nAlan,Wilmslow\nGrace,London"})).unwrap();
        a.run("mailings.filterRecipients", params).unwrap();
        let names: Vec<&str> = a.session.merge.rows.iter().map(|r| r[0].as_str()).collect();
        assert_eq!(names, ["Grace", "Ada"]);
        assert!(open_filters("recipients")[0].1.contains(&"xls"));
    }

    /// #1576: Find in list and "only records that will be merged" hide rows from the grid
    /// without touching the list; the shown rows keep their real indices for edit and remove.
    #[test]
    fn recipient_list_search_and_merged_view() {
        let fields: Vec<String> = vec!["Name".into(), "City".into()];
        let rows: Vec<Vec<String>> = [["Ada", "London"], ["Alan", "Wilmslow"], ["Grace", "LONDON"], ["Émile", "Paris"]]
            .iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect();
        let none = dialogs::visible_recipients(&rows, None, "", false);
        assert_eq!(none, [0, 1, 2, 3], "no search, no filter: every row");
        // Search: any cell, any case, including non-ASCII.
        assert_eq!(dialogs::visible_recipients(&rows, None, "london", false), [0, 2]);
        assert_eq!(dialogs::visible_recipients(&rows, None, "  AL ", false), [1], "trimmed, any case");
        assert_eq!(dialogs::visible_recipients(&rows, None, "émi", false), [3]);
        let greek = vec![vec!["ΟΣ".to_string()], vec!["ος".to_string()], vec!["Αλφα".to_string()]];
        assert_eq!(dialogs::visible_recipients(&greek, None, "ος", false), [0, 1], "final sigma matches capital sigma");
        assert_eq!(dialogs::visible_recipients(&rows, None, "ÉMI", false), [3]);
        assert!(dialogs::visible_recipients(&rows, None, "zzz", false).is_empty());
        // Filter only: City = London keeps Ada and Grace.
        let mut filter = dialogs::RecipientFilter::default();
        filter.rules.push(dialogs::FilterRow {
            field: "City".into(),
            op: wordcraft_engine::cmd::mailings::FilterOp::Equal,
            value: "London".into(),
            or: false,
        });
        let order = dialogs::merge_order(&fields, &rows, &filter).unwrap();
        assert_eq!(order, [Some(1), None, Some(2), None]);
        assert_eq!(dialogs::visible_recipients(&rows, Some(&order), "", false), [0, 1, 2, 3], "unchecked: excluded rows still shown");
        assert_eq!(dialogs::visible_recipients(&rows, Some(&order), "", true), [0, 2]);
        // Both: search within the merged records.
        assert_eq!(dialogs::visible_recipients(&rows, Some(&order), "grace", true), [2]);
        assert!(dialogs::visible_recipients(&rows, Some(&order), "alan", true).is_empty());
        // No filter: only_merged hides nothing.
        assert_eq!(dialogs::visible_recipients(&rows, None, "", true), [0, 1, 2, 3]);
        // Removing the first shown row removes the real record, not row 0.
        let shown = dialogs::visible_recipients(&rows, None, "grace", false);
        let mut edited = rows.clone();
        edited.remove(shown[0]);
        assert_eq!(edited.iter().map(|r| r[0].as_str()).collect::<Vec<_>>(), ["Ada", "Alan", "Émile"]);
        // Hostile order (shorter than the list) never panics; missing entries count as excluded.
        assert_eq!(dialogs::visible_recipients(&rows, Some(&[Some(1)]), "", true), [0]);
        // The view settings don't reach the merge.
        let mut viewed = filter.clone();
        viewed.find = "grace".into();
        viewed.only_merged = true;
        assert_eq!(viewed.params(&fields), filter.params(&fields));
    }

    /// #1576 review: while a cell is edited the grid keeps its rows, so a record the edit stops
    /// matching stays put instead of sliding another record under the cursor; and New Entry
    /// shows every row, so the new, empty entry can't be hidden.
    #[test]
    fn recipient_grid_holds_its_rows_while_a_cell_is_edited() {
        let mut held = None;
        // Not editing: whatever matches now.
        assert_eq!(dialogs::hold_rows(vec![0, 2], &mut held, false, 4), [0, 2]);
        assert_eq!(held, None);
        // Editing starts: the rows shown then are held…
        assert_eq!(dialogs::hold_rows(vec![0, 2], &mut held, true, 4), [0, 2]);
        // …even when the edit makes row 0 stop matching.
        assert_eq!(dialogs::hold_rows(vec![2], &mut held, true, 4), [0, 2]);
        // A held row past the end of a shrunk list is dropped, never indexed.
        assert_eq!(dialogs::hold_rows(vec![], &mut held, true, 1), [0]);
        // Editing ends: the view catches up.
        assert_eq!(dialogs::hold_rows(vec![2], &mut held, false, 4), [2]);
        assert_eq!(held, None);
        assert_eq!(dialogs::hold_rows(vec![1], &mut held, true, 4), [1], "a new edit holds the new rows");

        let mut filter = dialogs::RecipientFilter { find: "grace".into(), only_merged: true, held: Some(vec![1]), ..Default::default() };
        filter.show_all();
        assert!(filter.find.is_empty() && !filter.only_merged && filter.held.is_none());
    }

    /// #1576: the real Edit Recipient List grid in a harness (the dialog body, without the app
    /// around it): Find, merged-only, an edit while the search hides rows, remove, New Entry.
    #[test]
    fn recipient_grid_edits_the_right_record_through_the_ui() {
        use egui_kittest::kittest::Queryable;
        type State = (WordApp, Vec<String>, Vec<Vec<String>>, String, dialogs::RecipientFilter);
        let rows = [["Ada", "London"], ["Alan", "Wilmslow"], ["Grace", "London"], ["Émile", "Paris"]];
        let mut filter = dialogs::RecipientFilter::default();
        let rule =
            dialogs::FilterRow { field: "City".into(), op: wordcraft_engine::cmd::mailings::FilterOp::Equal, value: "London".into(), or: false };
        filter.rules.push(rule);
        let state: State = (
            app(),
            vec!["Name".into(), "City".into()],
            rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect(),
            String::new(),
            filter,
        );
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
            |ui, s: &mut State| {
                let (app, fields, rows, message, filter) = s;
                dialogs::recipient_list(app, ui, fields, rows, message, filter);
            },
            state,
        );
        h.run_steps(4);
        // Text boxes in order: Find, two field names, then the visible records' cells.
        let cells = |h: &egui_kittest::Harness<'_, State>| h.query_all_by_role(egui::accesskit::Role::TextInput).count().saturating_sub(3) / 2;
        let names = |h: &egui_kittest::Harness<'_, State>| h.state().2.iter().map(|r| r[0].clone()).collect::<Vec<_>>();
        // Find in list: the first text box. Grace is the only match.
        let find = h.query_all_by_role(egui::accesskit::Role::TextInput).next().unwrap();
        find.click();
        h.run_steps(2);
        h.query_all_by_role(egui::accesskit::Role::TextInput).next().unwrap().type_text("grace");
        h.run_steps(3);
        assert_eq!(h.state().4.find, "grace");
        assert!(cells(&h) == 1, "search hides the other rows");
        // Merged-only on top of it: still Grace (London is kept).
        h.get_by_label("Show only records that will be merged").click();
        h.run_steps(3);
        assert!(h.state().4.only_merged && cells(&h) == 1);
        // Edit Grace's cell to something the search no longer matches: the row stays, and the
        // edit lands on record 2, not on the first visible row's index.
        h.query_all_by_role(egui::accesskit::Role::TextInput).nth(3).unwrap().click();
        h.run_steps(2);
        h.event(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { command: true, ..Default::default() },
        });
        h.query_all_by_role(egui::accesskit::Role::TextInput).nth(3).unwrap().type_text("Zed");
        h.run_steps(3);
        assert_eq!(names(&h), ["Ada", "Alan", "Zed", "Émile"]);
        assert_eq!(h.state().4.held, Some(vec![2]), "rows are held while the cell has the keyboard");
        assert_eq!(cells(&h), 1);
        // Remove the one shown record: the third "×" (two are field headers).
        let xs: Vec<_> = h.get_all_by_label("×").collect();
        xs[2].click();
        h.run_steps(3);
        assert_eq!(names(&h), ["Ada", "Alan", "Émile"]);
        assert!(h.state().4.held.is_none());
        // New Entry clears the view and appends an empty record.
        h.get_by_label("New Entry").click();
        h.run_steps(3);
        assert_eq!(h.state().2, vec![vec!["Ada", "London"], vec!["Alan", "Wilmslow"], vec!["Émile", "Paris"], vec!["", ""]]);
        assert!(h.state().4.find.is_empty() && !h.state().4.only_merged);
    }

    /// #240: Select Recipients needs data, so clicking it opens the recipient list instead of
    /// reporting missing parameters; scripts and agents still get the error.
    #[test]
    fn select_recipients_opens_the_recipient_list() {
        let mut a = app();
        let r = a.run("mailings.recipients", json!({})).unwrap();
        assert_eq!(r, json!({"pending": "recipientList"}));
        assert_eq!(dialog_name(&a), Some("recipientList"));
        assert!(a.status_msg.is_none(), "no error in the status bar: {:?}", a.status_msg);
        let Some(dialogs::Dialog::RecipientList { fields, rows, .. }) = &a.dialog else { panic!() };
        assert_eq!(fields.len(), dialogs::NEW_LIST_FIELDS.len());
        assert_eq!(rows.len(), 1, "a new list starts with one empty entry");
        // Edit Recipient List opens the same dialog, showing the current list.
        a.dialog = None;
        a.run("mailings.recipients", json!({"csv": "Name,City\nAda,London"})).unwrap();
        a.run("mailings.editRecipients", json!({})).unwrap();
        let Some(dialogs::Dialog::RecipientList { fields, rows, .. }) = &a.dialog else { panic!() };
        assert_eq!((fields.clone(), rows.clone()), (vec!["Name".to_string(), "City".into()], vec![vec!["Ada".to_string(), "London".into()]]));
        // Type a New List… always starts fresh.
        a.run("ui.dialog", json!({"name": "newRecipientList"})).unwrap();
        let Some(dialogs::Dialog::RecipientList { fields, .. }) = &a.dialog else { panic!() };
        assert_eq!(fields[0], dialogs::NEW_LIST_FIELDS[0]);
        // Insert Merge Field without a field opens its dialog too.
        assert_eq!(a.run("mailings.insertField", json!({})).unwrap(), json!({"pending": "insertMergeField"}));
        // Programmatic calls never open dialogs.
        let mut b = app();
        assert!(b.execute("mailings.recipients", json!({})).is_err());
        assert!(b.dialog.is_none());
    }

    /// #339: Ctrl+F9 opens the Field dialog (#321) and Edit Field… shows the selected field's code;
    /// scripts and agents get an error instead of a dialog.
    #[test]
    fn field_code_dialogs_open_for_users_only() {
        let mut a = app();
        assert_eq!(a.run("insert.field", json!({})).unwrap(), json!({"pending": "field"}));
        a.dialog = None;
        a.run("insert.field", json!({"instr": "PAGE"})).unwrap();
        a.run("caret.docStart", json!({})).unwrap();
        assert_eq!(a.run("fields.setCode", json!({})).unwrap(), json!({"pending": "editFieldCode"}));
        let Some(dialogs::Dialog::FieldCode { code, edit: true, .. }) = &a.dialog else { panic!("{:?}", a.dialog) };
        assert_eq!(code, "PAGE");
        let mut b = app();
        assert!(b.execute("insert.field", json!({})).is_err());
        assert!(b.execute("fields.setCode", json!({})).is_err());
        assert!(b.dialog.is_none());
    }

    /// Rules › If…Then…Else… asks for its condition and inserts the IF field it describes; Next
    /// Record has nothing to ask. Match Fields and Check for Errors show what they found.
    #[test]
    fn merge_rules_and_reports_open_dialogs() {
        let mut a = app();
        a.run("mailings.recipients", json!({"csv": "First Name,City\nAda,London"})).unwrap();
        assert_eq!(a.run("mailings.rules", json!({"rule": "IF"})).unwrap(), json!({"pending": "ruleIf"}));
        let Some(mut d) = a.dialog.take() else { panic!("no dialog") };
        let dialogs::Dialog::MergeRule { field, value, then, els, .. } = &mut d else { panic!("{d:?}") };
        assert_eq!(field, "First Name", "the list's first field is picked");
        (*field, *value, *then, *els) = ("City".into(), "London".into(), "Local".into(), "Away".into());
        let params = dialogs::rule_params(&d).unwrap();
        a.run("mailings.rules", params).unwrap();
        assert!(a.dialog.is_none());
        let fields = |a: &WordApp| -> Vec<String> {
            let (doc, body) = (&a.session.doc, wordcraft_doc::StoryRef::Body);
            let paras = doc.para_paths(body).into_iter().filter_map(|p| doc.para(body, &p).cloned());
            paras
                .flat_map(|p| p.objects)
                .filter_map(|o| if let wordcraft_doc::para::InlineObject::Field { instr, .. } = o { Some(instr) } else { None })
                .collect()
        };
        assert_eq!(fields(&a), vec![r#"IF { MERGEFIELD City } = "London" "Local" "Away""#]);
        // Next Record inserts directly; Skip Record If asks.
        assert!(a.run("mailings.rules", json!({"rule": "NEXT"})).unwrap().get("pending").is_none());
        assert_eq!(fields(&a).last().map(String::as_str), Some("NEXT"));
        assert_eq!(a.run("mailings.rules", json!({"rule": "skipif"})).unwrap(), json!({"pending": "ruleSkipIf"}));
        a.dialog = None;
        a.run("mailings.matchFields", json!({})).unwrap();
        let Some(dialogs::Dialog::MatchFields { address, .. }) = &a.dialog else { panic!() };
        assert_eq!(address["firstName"], "First Name");
        a.run("mailings.checkErrors", json!({})).unwrap();
        let Some(dialogs::Dialog::CheckErrors { unknown, records }) = &a.dialog else { panic!() };
        assert_eq!((unknown.len(), *records), (0, 1));
        // Scripts and agents get the data or the default rule, never a dialog.
        let mut b = app();
        b.execute("mailings.checkErrors", json!({})).unwrap();
        b.execute("mailings.rules", json!({})).unwrap();
        assert!(b.dialog.is_none());
    }

    /// Desktop: Use an Existing List… picks a file and loads it as recipients, not as a document.
    #[test]
    fn an_existing_list_picked_on_the_desktop_loads_recipients() {
        let dir = scratch("recipients");
        let csv = dir.join("people.csv");
        std::fs::write(&csv, "First Name,City\nAda,London\nAlan,Wilmslow\n").unwrap();
        let mut a = typed();
        let asked = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let (seen, path) = (asked.clone(), csv.to_string_lossy().to_string());
        a.services.pick_open = Some(Box::new(move |purpose| {
            *seen.borrow_mut() = purpose.to_string();
            Some(path.clone())
        }));
        a.run("ui.openRecipientList", json!({})).unwrap();
        assert_eq!(*asked.borrow(), "recipients", "the host can offer CSV files");
        assert_eq!(a.session.merge.headers, vec!["First Name", "City"]);
        assert_eq!(a.session.merge.rows.len(), 2);
        assert!(body_text(&a).contains(UNSAVED), "the document stays open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Add Font… (#582), web: the picker asks for fonts, the picked file arrives through the
    /// inbox and is added, listed in the font menu and kept in the browser; removing it forgets it.
    #[test]
    fn a_font_picked_on_the_web_is_added_listed_and_kept() {
        const FAMILY: &str = "Uifont Sans 3";
        let mut font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        for (from, to) in [(utf16("Source Sans 3"), utf16(FAMILY)), (b"Source Sans 3".to_vec(), FAMILY.as_bytes().to_vec())] {
            let mut i = 0;
            while let Some(at) = font[i..].windows(from.len()).position(|w| w == from) {
                font[i + at..i + at + to.len()].copy_from_slice(&to);
                i += at + to.len();
            }
        }
        let mut a = typed();
        let asked = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let seen = asked.clone();
        a.services.open_async = Some(Box::new(move |purpose| *seen.borrow_mut() = purpose.to_string()));
        let kept = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(String, usize)>::new()));
        let (k1, k2) = (kept.clone(), kept.clone());
        a.services.store_font = Some(Box::new(move |name, bytes| k1.borrow_mut().push((name.to_string(), bytes.len()))));
        a.services.forget_font = Some(Box::new(move |name| k2.borrow_mut().retain(|(n, _)| n != name)));
        let inbox: Inbox = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        a.services.inbox = Some(inbox.clone());
        let ctx = egui::Context::default();
        assert!(!a.previews.families().iter().any(|f| f == FAMILY));
        a.run("font.add", json!({})).unwrap();
        assert_eq!(*asked.borrow(), "font");
        inbox.lock().unwrap().push(("Uifont-Regular.ttf".into(), font.clone()));
        a.drain_inbox();
        a.watch_fonts(&ctx);
        assert!(a.previews.families().iter().any(|f| f == FAMILY), "listed in the font menu at once");
        assert_eq!(*kept.borrow(), vec![("Uifont-Regular.ttf".to_string(), font.len())]);
        assert!(body_text(&a).contains(UNSAVED), "the document stays open");
        // Junk named like a font is refused, and nothing is kept.
        inbox.lock().unwrap().push(("junk.woff2".into(), b"wOF2junk".to_vec()));
        a.drain_inbox();
        assert_eq!(kept.borrow().len(), 1);
        a.run("font.remove", json!({"family": FAMILY})).unwrap();
        a.watch_fonts(&ctx);
        assert!(kept.borrow().is_empty());
        assert!(!a.previews.families().iter().any(|f| f == FAMILY));
    }

    /// Web: the picked file arrives through the inbox and loads as recipients, even as .txt; a
    /// CSV dropped on the page does too. Anything else afterwards opens as before.
    #[test]
    fn an_existing_list_picked_on_the_web_loads_recipients() {
        let mut a = typed();
        let asked = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let seen = asked.clone();
        a.services.open_async = Some(Box::new(move |purpose| *seen.borrow_mut() = purpose.to_string()));
        let inbox: Inbox = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        a.services.inbox = Some(inbox.clone());
        assert_eq!(a.run("ui.openRecipientList", json!({})).unwrap(), json!({"pending": true}));
        assert_eq!(*asked.borrow(), "recipients");
        inbox.lock().unwrap().push(("people.txt".into(), b"Name\tCity\nAda\tLondon\n".to_vec()));
        a.drain_inbox();
        assert_eq!(a.session.merge.headers, vec!["Name", "City"]);
        assert!(!a.recipient_list_pending);
        assert!(body_text(&a).contains(UNSAVED), "the document stays open");
        inbox.lock().unwrap().push(("more.csv".into(), b"Name\nBo\nCy\n".to_vec()));
        a.drain_inbox();
        assert_eq!(a.session.merge.rows.len(), 2);
        // A CSV picked through the picker must not turn the next .txt drop into recipients.
        a.run("ui.openRecipientList", json!({})).unwrap();
        inbox.lock().unwrap().push(("people.csv".into(), b"Name,City\nAda,London\n".to_vec()));
        a.drain_inbox();
        assert!(!a.recipient_list_pending);
        inbox.lock().unwrap().push(("notes.txt".into(), b"A plain text document.\nAnother line.\n".to_vec()));
        a.drain_inbox();
        assert_eq!(prompt(&a), None, "the TXT opens like any document, beside the first");
        assert!(body_text(&a).contains("A plain text document."), "the TXT opened as a document");
        let first = a.session.documents().first().map(|d| d.id).unwrap();
        a.run("view.switchWindows", json!({"id": first})).unwrap();
        assert_eq!(a.session.merge.headers, vec!["Name", "City"], "the recipients stay with their document");
        // A cancelled picker leaves nothing pending once the user does something else.
        a.run("ui.openRecipientList", json!({})).unwrap();
        a.run("format.bold", json!({})).unwrap();
        assert!(!a.recipient_list_pending);
    }

    /// Issue #859: lifting the pen jumped the view back to the caret, usually on page 1. Drawing
    /// leaves the caret where it was, so the view stays on the ink; typing still follows the caret.
    #[test]
    fn drawing_ink_keeps_the_view_where_it_is() {
        let mut a = typed();
        let keeps_view = |a: &mut WordApp, id: &str, params: Value| {
            a.canvas.scroll_to_caret = false;
            let r = a.run(id, params);
            assert!(r.is_ok(), "{id}: {r:?}");
            assert!(!a.canvas.scroll_to_caret, "{id} scrolled to the caret");
        };
        keeps_view(&mut a, "draw.stroke", json!({"page": 0, "points": [[100, 300], [200, 320]], "tool": "pen"}));
        keeps_view(&mut a, "draw.lasso", json!({"page": 0, "points": [[80, 280], [220, 280], [220, 340], [80, 340]]}));
        keeps_view(&mut a, "draw.lassoMove", json!({"dx": 10, "dy": 10}));
        keeps_view(&mut a, "draw.stroke", json!({"page": 0, "points": [[300, 500], [400, 500]], "tool": "pencil"}));
        keeps_view(&mut a, "draw.erase", json!({"page": 0, "x": 350, "y": 500}));
        a.canvas.scroll_to_caret = false;
        a.run("text.insert", json!({"text": "x"})).unwrap();
        assert!(a.canvas.scroll_to_caret, "typing still brings the caret into view");
    }

    /// #2054: clicking a ribbon tab while a footer is being edited scrolled the page a little.
    #[test]
    fn ribbon_tab_click_does_not_follow_the_caret() {
        let mut a = typed();
        a.canvas.scroll_to_caret = false;
        a.run("ui.tab", json!({"tab": "Design"})).unwrap();
        assert!(!a.canvas.scroll_to_caret, "switching tabs left the view alone");
        a.run("text.insert", json!({"text": "x"})).unwrap();
        assert!(a.canvas.scroll_to_caret, "typing still brings the caret into view");
    }

    /// #3864: Save (Ctrl+S, the Save button) jumped the view back to the caret, pages away from
    /// where the user was reading. Saving, exporting, printing and the document's properties move
    /// neither the caret nor the text, so the view stays, as in Word; opening another document
    /// still shows its caret.
    #[test]
    fn saving_and_exporting_keep_the_view_where_it_is() {
        let dir = scratch("save-keeps-view");
        let path = |name: &str| dir.join(name).to_string_lossy().to_string();
        let mut a = typed();
        let keeps_view = |a: &mut WordApp, id: &str, params: Value| {
            a.canvas.scroll_to_caret = false;
            let r = a.run(id, params);
            assert!(r.is_ok(), "{id}: {r:?}");
            assert!(!a.canvas.scroll_to_caret, "{id} scrolled to the caret");
        };
        keeps_view(&mut a, "file.saveAs", json!({"path": path("report.docx")}));
        a.run("text.insert", json!({"text": "x"})).unwrap();
        keeps_view(&mut a, "file.save", json!({}));
        keeps_view(&mut a, "file.encrypt", json!({"password": "hunter2"}));
        keeps_view(&mut a, "file.exportPdf", json!({"path": path("report.pdf")}));
        keeps_view(&mut a, "file.exportPng", json!({"path": path("report.png")}));
        keeps_view(&mut a, "file.printPdf", json!({"path": path("printed.pdf")}));
        keeps_view(&mut a, "file.saveTemplate", json!({"path": path("report.dotx")}));
        keeps_view(&mut a, "file.print", json!({}));
        keeps_view(&mut a, "file.printMarkup", json!({"value": true}));
        keeps_view(&mut a, "file.properties", json!({"title": "Quarterly report"}));
        keeps_view(&mut a, "file.info", json!({}));
        keeps_view(&mut a, "file.rename", json!({"name": "renamed"}));
        keeps_view(&mut a, "file.privacyOptions", json!({"removeDateTime": true}));
        keeps_view(&mut a, "file.privacy", json!({}));
        keeps_view(&mut a, "file.setAuthor", json!({"name": "Ada"}));
        keeps_view(&mut a, "file.compatibility", json!({}));
        keeps_view(&mut a, "file.share", json!({}));
        keeps_view(&mut a, "file.options", json!({"checkSpelling": false}));
        a.canvas.scroll_to_caret = false;
        a.execute("file.new", json!({})).unwrap();
        assert!(a.canvas.scroll_to_caret, "a new document still shows its caret");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Go to Footer into a footer just made — empty, so the caret at its start also "selects the
    /// whole story" — brings the footer into view (it stayed at the header).
    #[test]
    fn go_to_footer_into_an_empty_footer_scrolls_to_it() {
        let mut a = typed();
        a.run("insert.editHeader", json!({})).unwrap();
        a.run("text.insert", json!({"text": "Umowa nr 7/2026"})).unwrap();
        a.canvas.scroll_to_caret = false;
        a.run("insert.editFooter", json!({})).unwrap();
        assert!(a.session.sel.is_collapsed() && a.session.selects_whole_story(), "the footer is empty");
        assert!(a.canvas.scroll_to_caret, "the view follows the caret into the footer");
    }

    #[test]
    fn select_all_does_not_scroll_to_the_end() {
        let mut a = typed();
        a.canvas.scroll_to_caret = true;
        a.run("select.all", json!({})).unwrap();
        assert!(!a.canvas.scroll_to_caret);
        a.run("format.bold", json!({})).unwrap();
        assert!(!a.canvas.scroll_to_caret);
        a.run("edit.copy", json!({})).unwrap();
        assert!(!a.canvas.scroll_to_caret);
        a.run("text.insert", json!({"text": "x"})).unwrap();
        assert!(a.canvas.scroll_to_caret, "typing still brings the caret into view");
    }

    /// #1129: with no document open (the Start screen's empty window), `document.inspect` and
    /// `ui.render` on the control channel say so instead of reading the hidden placeholder.
    #[test]
    fn control_inspect_and_render_need_a_document() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.start_screen();
        assert!(!a.has_document());
        for (method, params) in [("document.inspect", json!({})), ("ui.render", json!({"page": 1, "scale": 0.5}))] {
            let (req, _reply) = ControlRequest::new(method, params);
            let control::Outcome::Done(v) = control::handle(&mut a, &ctx, &req) else { panic!("{method} replies at once") };
            assert_eq!(v["ok"], false, "{method}: {v}");
            assert!(v["error"].as_str().is_some_and(|e| e.contains("no document is open")), "{method}: {v}");
        }
        a.run("file.new", json!({})).unwrap();
        let (req, _reply) = ControlRequest::new("document.inspect", json!({}));
        let control::Outcome::Done(v) = control::handle(&mut a, &ctx, &req) else { panic!("document.inspect replies at once") };
        assert_eq!(v["ok"], true, "with a document it works: {v}");
    }

    /// `ui.render` and `ui.screenshot` without a `path` return the PNG in the reply instead of
    /// writing a fixed name into the shared temporary folder, where another user could have
    /// planted a link to one of this user's files.
    #[test]
    fn render_and_screenshot_without_a_path_reply_with_the_png() {
        let ctx = egui::Context::default();
        let mut a = typed();
        let (req, _reply) = ControlRequest::new("ui.render", json!({"page": 1, "scale": 0.5}));
        let control::Outcome::Done(v) = control::handle(&mut a, &ctx, &req) else { panic!("ui.render replies at once") };
        assert_eq!(v["ok"], true, "{v}");
        assert!(v["result"].get("path").is_none(), "no file: {v}");
        let png = wordcraft_engine::cmd::insert::base64_decode(v["result"]["png"].as_str().expect("png")).expect("base64");
        assert_eq!(&png[..4], b"\x89PNG", "{v}");
        assert!(v["result"]["width"].as_u64().unwrap() > 0 && v["result"]["pages"].as_u64().unwrap() >= 1, "{v}");

        let image = egui::ColorImage::from_rgba_unmultiplied([2, 1], &[255, 0, 0, 255, 0, 0, 255, 255]);
        let v = control::save_screenshot(&image, None, 41);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["request"], 41);
        assert!(v["result"].get("path").is_none(), "no file: {v}");
        let png = wordcraft_engine::cmd::insert::base64_decode(v["result"]["png"].as_str().expect("png")).expect("base64");
        assert_eq!(&png[..4], b"\x89PNG");
        assert_eq!(v["result"]["width"], 2, "{v}");
        assert_eq!(v["result"]["height"], 1, "{v}");
    }

    /// #821: Shift+F10 opens the page's context menu below the caret, like right-clicking there
    /// (Word's keyboard way to it; the Menu key isn't passed on by the windowing layer); Escape
    /// closes it and typing goes on.
    #[test]
    fn shift_f10_opens_the_context_menu_at_the_caret() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        h.event(egui::Event::Text("Kilns fire".into()));
        h.step();
        let pastes = |h: &egui_kittest::Harness<WordApp>| h.query_all_by_label_contains("Paste").count();
        let before = pastes(&h);
        h.event(key(egui::Key::F10, egui::Modifiers::SHIFT));
        h.step();
        h.step();
        assert!(h.state().canvas.context_menu_open, "Shift+F10 opened the menu");
        assert!(pastes(&h) > before, "the page's context menu, with Paste");
        let caret = canvas::caret_screen(h.state_mut()).unwrap();
        let at = h.state().canvas.context_menu_at.unwrap();
        assert!((at.x - caret.0.x).abs() < 1.0 && at.y > caret.0.y, "below the caret: {at:?} vs {caret:?}");
        assert_eq!(h.state().ui.keytips, keytips::Phase::Off, "Shift+F10 doesn't show keytips");
        h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
        h.step();
        h.step();
        assert!(!h.state().canvas.context_menu_open, "Escape closes it");
        // Scripts and agents open it too.
        h.state_mut().run("ui.contextMenu", json!({})).unwrap();
        h.step();
        h.step();
        assert!(h.state().canvas.context_menu_open);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns fire");
    }

    /// #712: Find, Replace and Go To open as in Word. Ctrl+F puts the keyboard in the Navigation
    /// pane's search box (with the selected text in it), not a dialog; Ctrl+H and Ctrl+G / F5 open
    /// the one Find and Replace dialog on its Replace and Go To tabs.
    #[test]
    fn find_replace_and_go_to_keys_like_word() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app(),
        );
        for _ in 0..6 {
            h.step();
        }
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        h.event(egui::Event::Text("Kilns fire clay. Kilns cool.".into()));
        h.step();
        let nav_query =
            |h: &egui_kittest::Harness<WordApp>| h.ctx.data(|d| d.get_temp::<String>(h.state().window_id("nav_query"))).unwrap_or_default();
        // Ctrl+F: the pane, and typing goes into its search box.
        h.event(key(egui::Key::F, egui::Modifiers::COMMAND));
        h.step();
        h.step();
        assert!(h.state().session.view.nav_pane, "the Navigation pane opened");
        assert_eq!(h.state().ui.nav_tab, "results", "Find exposes its results (#2096)");
        assert!(h.state().dialog.is_none(), "no dialog: {:?}", h.state().dialog.as_ref().map(|d| d.name()));
        h.event(key(egui::Key::C, egui::Modifiers::NONE));
        h.event(egui::Event::Text("cool".into()));
        h.step();
        h.step();
        assert_eq!(nav_query(&h), "cool", "typed into the pane's search box");
        assert_eq!(h.state().session.find.results.len(), 1);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns fire clay. Kilns cool.");
        // Manual navigation remains available until the next search edit.
        h.state_mut().ui.nav_tab = "headings".into();
        h.step();
        assert_eq!(h.state().ui.nav_tab, "headings");
        h.event(egui::Event::Text("missing".into()));
        h.step();
        h.step();
        assert_eq!(h.state().ui.nav_tab, "results", "editing the query reveals even an empty result list");
        assert!(h.state().session.find.results.is_empty());
        // With text selected, Ctrl+F searches for it.
        h.state_mut().ui.nav_tab = "pages".into();
        h.state_mut().run("select.text", json!({"text": "Kilns"})).unwrap();
        h.state_mut().canvas.want_focus = true;
        h.step();
        h.event(key(egui::Key::F, egui::Modifiers::COMMAND));
        h.step();
        h.step();
        assert_eq!(nav_query(&h), "Kilns");
        assert_eq!(h.state().ui.nav_tab, "results", "Find leaves Pages to show the selected-text matches");
        assert_eq!(h.state().session.find.results.len(), 2);
        // Ctrl+H: the Replace tab; Go To: the Go To tab of the same dialog.
        h.state_mut().canvas.want_focus = true;
        h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
        h.step();
        h.step();
        h.event(key(egui::Key::H, egui::Modifiers::COMMAND));
        h.step();
        h.step();
        assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("replace"));
        h.state_mut().run("edit.goto", json!({})).unwrap();
        h.step();
        assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("goto"));
        h.state_mut().run("edit.advancedFind", json!({})).unwrap();
        h.step();
        assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("find"));
    }

    /// Builds a root-viewport raw input at `size`, optionally marked minimized and occluded.
    fn shot_raw(size: egui::Vec2, hidden: bool) -> egui::RawInput {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let mut raw = egui::RawInput { screen_rect: Some(rect), max_texture_side: Some(16_384), ..Default::default() };
        raw.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                inner_rect: Some(rect),
                native_pixels_per_point: Some(1.0),
                minimized: Some(hidden),
                occluded: Some(hidden),
                ..Default::default()
            },
        );
        raw
    }

    /// Runs one screenshot frame through the raw hook, logic and UI, and clears its texture deltas.
    fn shot_frame(app: &mut WordApp, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        let mut raw = shot_raw(size, false);
        app.raw_input_hook(ctx, &mut raw);
        let mut output = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        });
        output.textures_delta.clear();
        output
    }

    /// Replaces each root screenshot command in the output with `ViewportCommand::Focus` and returns
    /// its callback, so the test frame runs no real screenshot.
    fn take_shot_callbacks(output: &mut egui::FullOutput) -> Vec<egui::ScreenshotCallback> {
        let mut callbacks = Vec::new();
        if let Some(viewport) = output.viewport_output.get_mut(&egui::ViewportId::ROOT) {
            for command in &mut viewport.commands {
                if matches!(command, egui::ViewportCommand::Screenshot(_)) {
                    let egui::ViewportCommand::Screenshot(callback) = std::mem::replace(command, egui::ViewportCommand::Focus) else {
                        continue;
                    };
                    callbacks.push(callback);
                }
            }
        }
        callbacks
    }

    /// Creates an app with text, warms four screenshot frames, and returns it with a fresh context.
    fn screenshot_app(size: egui::Vec2) -> (WordApp, egui::Context) {
        let mut app = app();
        app.run("document.setText", json!({"text": "Screenshot"})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..4 {
            shot_frame(&mut app, &ctx, size);
        }
        (app, ctx)
    }

    /// Admits a screenshot through `control::admit_screenshot` with a fresh reply channel and returns
    /// the receiver.
    fn admit(app: &mut WordApp, ctx: &egui::Context, path: Option<String>) -> std::sync::mpsc::Receiver<Value> {
        let (reply, replies) = std::sync::mpsc::channel();
        control::admit_screenshot(app, ctx, path, reply);
        replies
    }

    /// Forces queued screenshot requests ready and runs one frame to collect their callbacks.
    fn issue_ready(app: &mut WordApp, ctx: &egui::Context, size: egui::Vec2) -> Vec<egui::ScreenshotCallback> {
        for shot in &mut app.shots.requests {
            shot.ready_at = 0.0;
            shot.frames = 3;
        }
        let mut output = shot_frame(app, ctx, size);
        take_shot_callbacks(&mut output)
    }

    /// The deadline/animation clock is monotonic on native and in the browser.
    #[test]
    fn now_ms_never_moves_backwards() {
        let mut last = now_ms();
        for _ in 0..1000 {
            let now = now_ms();
            assert!(now >= last, "the clock went backwards: {last} -> {now}");
            last = now;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[derive(Debug)]
    struct DroppedText(std::path::PathBuf);

    #[cfg(not(target_arch = "wasm32"))]
    impl egui::DroppedFile for DroppedText {
        /// Exposes the dropped file's path.
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        /// Reads the dropped file's bytes, stringifying I/O errors.
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    /// A file dropped in the first two font-warm-up passes is taken before the early return;
    /// eframe does not preserve dropped files for a later frame.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_drop_before_font_warm_up_is_not_lost() {
        let (mut app, ctx) = (app(), egui::Context::default());
        let path = std::env::temp_dir().join(format!("wordcraft-drop-warmup-{}.txt", std::process::id()));
        std::fs::write(&path, b"dropped text").expect("write the dropped document");
        let mut raw = shot_raw(egui::vec2(640.0, 480.0), false);
        raw.dropped_files.push(std::sync::Arc::new(DroppedText(path.clone())));
        app.raw_input_hook(&ctx, &mut raw);
        ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        })
        .drop_without_applying_deltas();
        assert_eq!(app.fonts_frames, 1, "the UI returned for font warm-up");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end_matches(['\r', '\n']), "dropped text");
        let _ = std::fs::remove_file(&path);
    }

    /// Wayland may leave `ViewportInfo::inner_rect` unknown: capture geometry then uses the same
    /// RawInput frame's screen rect and rounds logical points to physical pixels.
    #[test]
    fn capture_geometry_falls_back_to_screen_rect_and_rounds_physical_pixels() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 200.0));
        let mut raw = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
        raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(1.5);
        let surface = control::SurfaceGeometry::from_raw(&raw, &ctx).expect("screen rect fallback");
        let geometry = surface.with_target(egui::ViewportId::ROOT, 7);
        assert_eq!(geometry.expected_size(), Some([480, 300]));
        assert_eq!(geometry.expected_bytes(), Some(480 * 300 * 4));
    }

    /// A completed capture reaches its own slot, the callback never writes a file, and the
    /// request is replied to exactly once from `logic`.
    #[test]
    fn screenshot_callback_completes_one_request_without_writing_from_the_callback() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let replies = admit(&mut app, &ctx, None);
        let callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert_eq!(callbacks.len(), 1);
        callbacks.into_iter().next().unwrap().complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        assert!(replies.try_recv().is_err(), "the callback itself sends no reply");
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let reply = replies.recv().unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["width"], 640);
        assert_eq!(reply["result"]["height"], 480);
        assert!(app.shots.requests.is_empty(), "the reservation is released");
    }

    /// A discarded first layout pass does not issue two callback commands: the shot's issued
    /// flag and first-pass settle counter keep the capture one-shot across the final pass.
    #[test]
    fn screenshot_issue_is_one_shot_across_a_multipass_frame() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let replies = admit(&mut app, &ctx, None);
        for shot in &mut app.shots.requests {
            shot.ready_at = 0.0;
            shot.frames = 3;
        }
        let mut passes = 0;
        let mut output = ctx.run_ui(shot_raw(egui::vec2(640.0, 480.0), false), |ui| {
            if passes == 0 {
                ui.request_discard("test capture multipass");
            }
            app.logic(ui.ctx());
            app.ui(ui);
            passes += 1;
        });
        assert_eq!(passes, 2);
        let callbacks = take_shot_callbacks(&mut output);
        assert_eq!(callbacks.len(), 1, "one callback for the whole host frame");
        callbacks.into_iter().next().unwrap().complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        output.drop_without_applying_deltas();
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert_eq!(replies.recv().unwrap()["ok"], true);
    }

    /// Two requests keep their own callbacks; a later completion does not consume the earlier
    /// request's image or reply.
    #[test]
    fn screenshot_callbacks_complete_out_of_order() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let first = admit(&mut app, &ctx, None);
        let second = admit(&mut app, &ctx, None);
        let mut callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert_eq!(callbacks.len(), 2);
        let first_callback = callbacks.remove(0);
        let second_callback = callbacks.remove(0);
        second_callback.complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let second_reply = second.recv().unwrap();
        assert_eq!(second_reply["ok"], true, "{second_reply}");
        assert!(first.try_recv().is_err(), "the first request is still pending");
        assert_eq!(app.shots.requests.len(), 1);
        first_callback.complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let first_reply = first.recv().unwrap();
        assert_eq!(first_reply["ok"], true, "{first_reply}");
        assert!(app.shots.requests.is_empty());
    }

    /// A deadline is checked before a completed image is written. A callback that arrives after
    /// the timeout finds a cancelled slot and gets no second reply.
    #[test]
    fn a_late_screenshot_callback_never_writes_or_replies() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let path = std::env::temp_dir().join(format!("wordcraft-late-shot-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let replies = admit(&mut app, &ctx, Some(path.to_string_lossy().to_string()));
        let callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let callback = callbacks.into_iter().next().unwrap();
        app.shots.requests[0].deadline = 0.0;
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert_eq!(replies.recv().unwrap()["ok"], false);
        callback.complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert!(replies.try_recv().is_err(), "one reply only");
        assert!(!path.exists(), "no late file was written");
        assert!(app.shots.requests.is_empty());
    }

    /// Resizing away and back is still a new geometry epoch, so old pixels are rejected and the
    /// request retries inside its original deadline.
    #[test]
    fn screenshot_epoch_changes_even_when_geometry_returns() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let replies = admit(&mut app, &ctx, None);
        let callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let callback = callbacks.into_iter().next().unwrap();
        shot_frame(&mut app, &ctx, egui::vec2(800.0, 600.0));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        callback.complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert!(replies.try_recv().is_err(), "stale pixels are dropped");
        assert_eq!(app.shots.requests.len(), 1, "the request retries");
        assert!(!app.shots.requests[0].issued);
        app.shots.requests[0].deadline = 0.0;
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert_eq!(replies.recv().unwrap()["ok"], false);
    }

    /// A geometry change and change back still invalidates issued pixels and retries the request.
    #[test]
    fn screenshot_epoch_observes_a_raw_change_and_change_back() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let replies = admit(&mut app, &ctx, None);
        let callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let callback = callbacks.into_iter().next().unwrap();
        let issued_epoch = app.shots.requests[0].epoch;
        let mut away = shot_raw(egui::vec2(800.0, 600.0), false);
        app.raw_input_hook(&ctx, &mut away);
        let mut back = shot_raw(egui::vec2(640.0, 480.0), false);
        app.raw_input_hook(&ctx, &mut back);
        assert!(app.capture_epoch > issued_epoch);
        callback.complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert!(replies.try_recv().is_err(), "pixels from before the hidden change are dropped");
        assert_eq!(app.shots.requests.len(), 1, "the request retries");
        assert!(!app.shots.requests[0].issued);
    }

    /// Wayland can report `inner_rect: None`; a resize is still seen through the raw screen rect
    /// and starts a new epoch instead of reusing the last painted size.
    #[test]
    fn screenshot_geometry_uses_the_current_raw_screen_rect_on_wayland() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let before = app.capture_epoch;
        let mut raw = shot_raw(egui::vec2(320.0, 200.0), false);
        raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().inner_rect = None;
        app.raw_input_hook(&ctx, &mut raw);
        let _ = ctx.run_logic(&raw, |ctx| app.logic(ctx));
        assert!(app.capture_epoch > before, "a resize starts a new geometry epoch");
        assert_eq!(app.capture_geometry.and_then(control::CaptureGeometry::expected_size), Some([320, 200]));
    }

    /// A successful capture with a path writes exactly that file, and its reply carries the
    /// request token used to correlate admission, timeout and completion.
    #[test]
    fn screenshot_success_writes_the_requested_path_and_correlates_the_token() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let path = std::env::temp_dir().join(format!("wordcraft-shot-ok-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let replies = admit(&mut app, &ctx, Some(path.to_string_lossy().to_string()));
        let token = app.shots.requests[0].token;
        let callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        callbacks.into_iter().next().unwrap().complete(std::sync::Arc::new(egui::ColorImage::filled([640, 480], egui::Color32::BLACK)));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let reply = replies.recv().unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["request"], token, "{reply}");
        let png = std::fs::read(&path).expect("the requested screenshot file");
        assert_eq!(&png[..4], b"\x89PNG", "a real PNG was written");
        let _ = std::fs::remove_file(&path);
    }

    /// A request admitted for document A is never rebound to document B. The slot is cancelled,
    /// the requested path is not written and the caller gets one document-changed error.
    #[test]
    fn screenshot_document_switch_cancels_instead_of_rebinding() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let path = std::env::temp_dir().join(format!("wordcraft-shot-doc-switch-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let replies = admit(&mut app, &ctx, Some(path.to_string_lossy().to_string()));
        let document = app.session.document_id();
        app.session.dirty = false; // A saved document can be replaced without the save prompt.
        app.run("file.new", json!({})).unwrap();
        assert_ne!(document, app.session.document_id(), "the test must actually replace the document");
        let callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        assert!(callbacks.is_empty(), "no callback is issued for the new document");
        let reply = replies.recv().unwrap();
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().unwrap_or_default().contains("document changed"), "{reply}");
        assert!(!path.exists(), "document B's pixels never reach document A's path");
        assert!(app.shots.requests.is_empty());
        assert!(replies.try_recv().is_err(), "one reply only");
    }

    /// A minimized (or fully occluded) window cannot be captured: admission/issuance cancels the
    /// request rather than accepting the last visible surface or retrying it.
    #[test]
    fn screenshot_minimized_window_is_cancelled_not_retried() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let replies = admit(&mut app, &ctx, None);
        for shot in &mut app.shots.requests {
            shot.ready_at = 0.0;
            shot.frames = 3;
        }
        let mut raw = shot_raw(egui::vec2(640.0, 480.0), true);
        app.raw_input_hook(&ctx, &mut raw);
        let mut output = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        let callbacks = take_shot_callbacks(&mut output);
        output.drop_without_applying_deltas();
        assert!(callbacks.is_empty(), "no capture is issued while minimized");
        let reply = replies.recv().unwrap();
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().unwrap_or_default().contains("minimized"), "{reply}");
        assert!(app.shots.requests.is_empty());
    }

    /// Admission also refuses a minimized surface: the last visible size must not be captured.
    #[test]
    fn screenshot_admission_rejects_a_minimized_window() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let mut raw = shot_raw(egui::vec2(640.0, 480.0), true);
        app.raw_input_hook(&ctx, &mut raw);
        let _ = ctx.run_logic(&raw, |ctx| app.logic(ctx));
        let replies = admit(&mut app, &ctx, None);
        let reply = replies.recv().unwrap();
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().unwrap_or_default().contains("minimized"), "{reply}");
        assert!(app.shots.requests.is_empty());
    }

    /// A malformed completion is reported at once; it is not treated as a resize and retried.
    #[test]
    fn screenshot_malformed_completion_fails_without_retrying() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let replies = admit(&mut app, &ctx, None);
        let callbacks = issue_ready(&mut app, &ctx, egui::vec2(640.0, 480.0));
        callbacks.into_iter().next().unwrap().complete(std::sync::Arc::new(egui::ColorImage {
            size: [640, 480],
            source_size: egui::vec2(640.0, 480.0),
            pixels: Vec::new(),
        }));
        shot_frame(&mut app, &ctx, egui::vec2(640.0, 480.0));
        let reply = replies.recv().unwrap();
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().unwrap_or_default().contains("pixels"), "{reply}");
        assert!(reply.get("request").is_some(), "completion errors carry the request token: {reply}");
        assert!(app.shots.requests.is_empty(), "a malformed image is not retried");
        assert!(replies.try_recv().is_err());
    }

    /// Admission enforces four requests, 128 MiB per image and 256 MiB aggregate, without
    /// evicting an earlier request.
    #[test]
    fn screenshot_admission_bounds_count_image_and_aggregate() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        for _ in 0..control::SCREENSHOT_MAX_REQUESTS {
            let _ = admit(&mut app, &ctx, None);
        }
        let (reply, replies) = std::sync::mpsc::channel();
        control::admit_screenshot(&mut app, &ctx, None, reply);
        assert_eq!(replies.recv().unwrap()["ok"], false);
        assert_eq!(app.shots.requests.len(), control::SCREENSHOT_MAX_REQUESTS);

        // 2300×2250 reserves about 90 MB including image, RGBA, PNG and base64 buffers:
        // two fit the 256 MB aggregate, a third does not.
        let mut app = self::app();
        let ctx = egui::Context::default();
        let mut raw = shot_raw(egui::vec2(2300.0, 2250.0), false);
        app.raw_input_hook(&ctx, &mut raw);
        let _ = ctx.run_logic(&raw, |ctx| app.logic(ctx));
        let _first = admit(&mut app, &ctx, None);
        let _second = admit(&mut app, &ctx, None);
        let (reply, replies) = std::sync::mpsc::channel();
        control::admit_screenshot(&mut app, &ctx, None, reply);
        let over = replies.recv().unwrap();
        assert!(over["error"].as_str().unwrap_or_default().contains("aggregate"), "{over}");

        // 3400×3000 exceeds the per-image working set on its own.
        let mut app = self::app();
        let ctx = egui::Context::default();
        let mut raw = shot_raw(egui::vec2(3400.0, 3000.0), false);
        app.raw_input_hook(&ctx, &mut raw);
        let _ = ctx.run_logic(&raw, |ctx| app.logic(ctx));
        let (reply, replies) = std::sync::mpsc::channel();
        control::admit_screenshot(&mut app, &ctx, None, reply);
        let too_large = replies.recv().unwrap();
        assert!(too_large["error"].as_str().unwrap_or_default().contains("image limit"), "{too_large}");
    }

    /// A queued capture on a hidden window asks for a real paint, because issuing the screenshot
    /// is UI work even though `logic` is the only callback a hidden tick gets. The forced paint
    /// pass must actually reach issuance: exactly one callback, and the request is marked issued.
    #[test]
    fn a_hidden_queued_capture_requests_a_paint_and_the_forced_pass_issues_it() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        let _replies = admit(&mut app, &ctx, None);
        for shot in &mut app.shots.requests {
            shot.ready_at = 0.0;
            shot.frames = 3;
        }
        let mut raw = shot_raw(egui::vec2(640.0, 480.0), true);
        app.raw_input_hook(&ctx, &mut raw);
        let output = ctx.run_logic(&raw, |ctx| app.logic(ctx));
        let commands = output.viewport_commands.get(&egui::ViewportId::ROOT).cloned().unwrap_or_default();
        assert!(commands.contains(&egui::ViewportCommand::RequestPaintWhileHidden), "{commands:?}");
        assert!(
            output.viewport_commands.values().flatten().all(|command| !matches!(command, egui::ViewportCommand::Screenshot(_))),
            "logic alone does not issue the capture"
        );
        assert!(app.shots.requests.first().is_some_and(|shot| !shot.issued), "the request is still queued");

        // The paint requested above is a full UI pass. Issuing happens there, not in `logic`.
        let mut paint = shot_raw(egui::vec2(640.0, 480.0), false);
        app.raw_input_hook(&ctx, &mut paint);
        let mut output = ctx.run_ui(paint, |ui| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        });
        let callbacks = take_shot_callbacks(&mut output);
        output.drop_without_applying_deltas();
        assert_eq!(callbacks.len(), 1, "the forced paint pass issues exactly one capture");
        assert!(app.shots.requests.first().is_some_and(|shot| shot.issued), "the queued request reached issuance");
    }

    /// Real RawInput is offered once, survives hidden ticks, and a multipass frame consumes it
    /// once. The step is not drained by a pass that will be discarded.
    #[test]
    fn synthetic_input_is_offered_once_through_hidden_and_multipass_frames() {
        let (mut app, ctx) = screenshot_app(egui::vec2(640.0, 480.0));
        app.run("document.setText", json!({"text": ""})).unwrap();
        app.synthetic = vec![egui::Event::Text("x".into())];
        let mut first = egui::RawInput::default();
        app.raw_input_hook(&ctx, &mut first);
        assert_eq!(first.events.len(), 1);
        assert!(app.synthetic_pending);
        assert_eq!(app.synthetic.len(), 1, "the step is not consumed by the hook");

        // eframe keeps the unconsumed RawInput for later; a hidden logic tick must not append it
        // again, nor offer the next step.
        let mut hidden = egui::RawInput::default();
        app.raw_input_hook(&ctx, &mut hidden);
        assert!(hidden.events.is_empty(), "the pending step is not appended twice");
        app.synthetic.push(egui::Event::Text("y".into()));
        let mut hidden_again = egui::RawInput::default();
        app.raw_input_hook(&ctx, &mut hidden_again);
        assert!(hidden_again.events.is_empty(), "a later synthetic step waits too");
        assert_eq!(app.synthetic.len(), 2);

        // One host frame, two passes. The first is discarded, so the UI ack waits for the final
        // pass; the input itself exists only in the first pass and must type exactly once.
        let mut passes = 0;
        ctx.run_ui(first, |ui| {
            ui.request_discard("test multipass");
            app.logic(ui.ctx());
            app.ui(ui);
            passes += 1;
        })
        .drop_without_applying_deltas();
        assert_eq!(passes, 2, "the discard forced a second pass");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "x");
        assert!(!app.synthetic_pending, "the final UI pass acknowledged the step");
        assert_eq!(app.synthetic.len(), 1, "only the offered step was removed");

        // The next host frame offers y, which was waiting behind the acknowledged step.
        let mut next = egui::RawInput::default();
        app.raw_input_hook(&ctx, &mut next);
        assert_eq!(next.events, vec![egui::Event::Text("y".into())]);
        ctx.run_ui(next, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        })
        .drop_without_applying_deltas();
        assert!(!app.synthetic_pending);
        assert!(app.synthetic.is_empty());
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "xy", "each step typed once");
    }

    /// A synthetic step waits for the font warm-up frames rather than being injected into a pass
    /// whose layout has no usable font atlas.
    #[test]
    fn synthetic_input_waits_for_font_warm_up() {
        let mut app = app();
        app.synthetic = vec![egui::Event::Text("x".into())];
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        app.raw_input_hook(&ctx, &mut raw);
        assert!(raw.events.is_empty(), "fonts are not ready yet");
        assert_eq!(app.synthetic.len(), 1, "the step is retained");
    }

    /// #991: the status bar's Focus button leaves only the page (no ribbon, status bar or panes);
    /// Escape brings them back.
    #[test]
    fn focus_hides_the_ribbon_and_escape_brings_it_back() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.ui.tab = "View".into();
        a.session.view.nav_pane = true;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let shown = |h: &egui_kittest::Harness<WordApp>, label| h.query_by_label(label).is_some();
        assert!(shown(&h, "Ruler"), "the View tab is showing");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Focus").click();
        h.step();
        h.step();
        assert!(h.state().session.view.focus_mode);
        assert!(!shown(&h, "Ruler") && !shown(&h, "Focus"), "the ribbon is hidden");
        // The pane's title bar: the only pane open is Navigation.
        assert!(!shown(&h, "Move Pane: Navigation"), "and so is the Navigation pane");
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        h.step();
        h.step();
        assert!(!h.state().session.view.focus_mode, "Escape leaves Focus");
        assert!(shown(&h, "Ruler"), "and the ribbon is back");
        assert!(shown(&h, "Move Pane: Navigation"), "with the pane");
        // Immersive Reader is Read Mode, not Focus: its status bar stays.
        h.state_mut().run("view.immersive", json!({})).unwrap();
        h.step();
        h.step();
        assert!(h.state().session.view.read_mode && shown(&h, "Focus"), "Immersive Reader keeps the status bar");
    }
}
