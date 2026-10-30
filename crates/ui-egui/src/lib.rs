//! WordCraft's egui front end: a Word-style window (title bar with Quick Access Toolbar, ribbon,
//! rulers, page canvas, panes, status bar, Backstage, dialogs) over `wordcraft-engine`.
//!
//! The UI is thin: it reads `Session` state and acts through [`WordApp::run`] (command ids), so
//! the menus, ribbon, shortcuts, control channel and MCP all reach the same behaviour.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

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
pub use dialogs::dialogs_chart;
pub use dialogs::dialogs_controls;
pub use dialogs::dialogs_insert;
pub use dialogs::dialogs_keyboard;
pub use dialogs::dialogs_layout;
pub use dialogs::dialogs_lists;
pub use dialogs::dialogs_notes;
pub use dialogs::dialogs_para;
pub use dialogs::dialogs_refs;
pub use dialogs::dialogs_review;
pub use dialogs::dialogs_ribbon;
pub use dialogs::dialogs_smart_art;
pub use dialogs::dialogs_table_style;
pub use dialogs::dialogs_templates;
pub use file_dialogs::{FileDialogAnswer, FileDialogRequest, open_filters};

mod app;
mod clock;
mod services;
mod state;

use app::control_click_as_secondary;
pub(crate) use app::{discards_document, works_without_document};
pub use clock::now_ms;
pub use services::{HostSaveFn, Inbox, Services};
pub use state::{AutoSaveBlock, DefaultApp, SaveChoice, keeps_everything};

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
    /// User name (File › Options) for comments and tracked changes. `None` (no saved name) keeps
    /// the default; a saved empty name stays empty (#4368).
    pub author: Option<String>,
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
            author: None,
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
    fonts_hant: bool,
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

/// Every `.rs` file under `dir`, subdirectories included, in a fixed order: tests that scan the
/// sources must keep seeing code that moves into a module directory.
#[cfg(test)]
pub(crate) fn rust_sources(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = vec![];
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests;
