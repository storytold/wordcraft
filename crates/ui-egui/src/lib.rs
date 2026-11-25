//! WordCraft's egui front end: a Word-style window (title bar with Quick Access Toolbar, ribbon,
//! rulers, page canvas, panes, status bar, Backstage, dialogs) over `wordcraft-engine`.
//!
//! The UI is thin: it reads `Session` state and acts through [`WordApp::run`] (command ids), so
//! the menus, ribbon, shortcuts, control channel and MCP all reach the same behaviour.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// WordCraft as craft-update knows it, with its 128 px icon and accent colour: the update
/// prompt shows both, and an AppImage's menu entry gets the icon.
#[cfg(not(target_arch = "wasm32"))]
pub const UPDATE_APP: craft::update::app::App = craft::update::app::App {
    icon_png: Some(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.wordcraft.png")),
    accent: Some([0x3f, 0x5f, 0xe0]),
    ..craft::update::suite::WORDCRAFT
};

/// WordCraft as the host and the theme pins know it. [`UPDATE_APP`] stays beside it: the updater
/// takes that type, which carries the icon and accent this one has no place for.
pub const APP: craft::host::AppInfo = craft::host::AppInfo { id: "wordcraft", name: "WordCraft", version: env!("CARGO_PKG_VERSION") };

/// An English UI string in the current interface language ([`i18n::t`]).
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod a11y;
pub mod a11y_pane;
pub mod backstage;
pub mod canvas;
pub mod caret_glide;
pub mod chrome;
// PLAN: the `colors` module line stays because `more_colors` (ribbon-w11, rb.A4) names `colors::Target`; the sheet's removal list assumed the dialog went with it.
pub mod colors;
pub mod content_controls;
pub mod control;
pub mod credits;
pub mod crop;
pub mod dialogs;
pub mod documents;
pub mod equation_tab;
pub mod file_dialogs;
pub mod font_download;
pub mod format_shape;
pub mod i18n;
pub mod icons;
pub mod ink;
pub mod keys;
pub mod keytips;
pub(crate) mod language_packs;
mod local_time;
pub mod md_source;
pub mod mini_toolbar;
pub mod objects;
pub mod options;
/// The window identity of the channel this build runs as (stable or Nightly).
pub mod os_window;
pub mod panes;
pub mod paste_picture;
pub mod prefs;
pub mod previews;
pub mod print_page;
pub mod printing;
pub mod proofing_pane;
pub mod read_aloud;
mod restrict_pane;
pub mod ribbon;
mod save_types;
pub mod scroll;
pub use scroll::ScrollSpeed;
pub mod desktop_theme; // owed by ribbon batch 2 (S-L348)
pub mod system_clipboard;
pub mod table_mouse;
pub mod table_pen;
pub mod table_resize;
pub mod theme;
mod undo_marks;
pub mod updates_ui;
pub mod widgets;
pub mod window_geometry;
mod window_ui;
mod windows;
pub mod wordart_menus;
pub mod zotero;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::Session;

pub use dialogs::dialogs_bullet;
pub use dialogs::dialogs_chart;
pub use dialogs::dialogs_controls;
pub use dialogs::dialogs_insert;
pub use dialogs::dialogs_layout;
pub use dialogs::dialogs_lists;
pub use dialogs::dialogs_notes;
pub use dialogs::dialogs_para;
pub use dialogs::dialogs_refs;
pub use dialogs::dialogs_review;
pub use dialogs::dialogs_ribbon;
pub use dialogs::dialogs_smart_art;
pub use file_dialogs::open_filters;

mod announce;
mod app;
mod services;
mod state;

#[cfg(test)]
pub(crate) use app::works_without_document;
// PLAN: assumed shell-w13 leaves `HostSaveFn` and `Inbox` out of `services` (sh.A14 removes `host_save` and `inbox`).
pub use app::fonts::ui_fonts;
pub use services::Services;
pub use state::{AutoSaveBlock, keeps_everything};

/// A status-bar message and when it stops showing.
pub(crate) struct Status {
    /// The message.
    pub text: String,
    /// The [`craft::platform::clock::monotonic_ms`] reading from which the bar no longer shows it.
    pub until: f64,
    /// The tone it reads in; failures are `Bad`.
    pub tone: craft::theme::variant::Tone,
}

/// A save the browser's picker hasn't finished.
struct SaveInFlight {
    ticket: craft::host::PickId,
    /// The file name it saves as.
    name: String,
    /// The document it saved ([`Session::document_id`]).
    document: u64,
    /// Whether it marked the document saved (a Save in a format that keeps everything).
    marked_saved: bool,
    /// The session revision when the save started: a later edit isn't in that save.
    rev: u64,
    /// Puts back what a Save stamped (zoom, author, dates, revision) and the dirty flag, when the
    /// save doesn't reach its destination and nothing was edited since. `None` for an export.
    undo_stamp: Option<Box<dyn FnOnce(&mut wordcraft_engine::Session)>>,
}

/// File › Options › Advanced › Linux display server (#2184, `ui.displayServer`): how the desktop
/// app connects on Linux and the BSDs. It's read before the window opens, so a change applies at
/// the next launch; `WORDCRAFT_WAYLAND` overrides it. An unknown saved value reads as Automatic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum DisplayServer {
    /// The app's own choice: XWayland whenever there's an X server, for file drag-and-drop.
    #[default]
    Auto,
    Wayland,
    X11,
}

impl DisplayServer {
    /// In menu order.
    pub const ALL: [DisplayServer; 3] = [DisplayServer::Auto, DisplayServer::Wayland, DisplayServer::X11];

    /// The value used in `ui.json` and by the `ui.displayServer` command.
    pub fn code(self) -> &'static str {
        match self {
            DisplayServer::Auto => "auto",
            DisplayServer::Wayland => "wayland",
            DisplayServer::X11 => "x11",
        }
    }

    pub fn from_code(code: &str) -> Option<DisplayServer> {
        DisplayServer::ALL.into_iter().find(|d| d.code().eq_ignore_ascii_case(code.trim()))
    }

    /// The English label; only Automatic is translated, the others are names.
    pub fn label(self) -> &'static str {
        match self {
            DisplayServer::Auto => "Automatic",
            DisplayServer::Wayland => "Wayland",
            DisplayServer::X11 => "X11 (XWayland)",
        }
    }
}

/// What the status bar's live count shows (#2797): words (the default, as Word's status bar
/// counts), characters with spaces, or characters without spaces — the same three the Word Count
/// dialog reports, worded the same way. Right-click the count in the status bar, or File ›
/// Options › Display, to choose; kept between runs in `ui.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum StatusCount {
    #[default]
    Words,
    Characters,
    CharactersNoSpaces,
}

impl StatusCount {
    /// In menu order.
    pub const ALL: [StatusCount; 3] = [StatusCount::Words, StatusCount::Characters, StatusCount::CharactersNoSpaces];

    /// The value used in `ui.json` and by the `ui.statusCount` command.
    pub fn code(self) -> &'static str {
        match self {
            StatusCount::Words => "words",
            StatusCount::Characters => "characters",
            StatusCount::CharactersNoSpaces => "charactersNoSpaces",
        }
    }

    pub fn from_code(code: &str) -> Option<StatusCount> {
        StatusCount::ALL.into_iter().find(|c| c.code().eq_ignore_ascii_case(code.trim()))
    }

    /// The English label (translated with `tl!` where shown): the Word Count dialog's own words
    /// for these counts.
    pub fn label(self) -> &'static str {
        match self {
            StatusCount::Words => "Words",
            StatusCount::Characters => "Characters (with spaces)",
            StatusCount::CharactersNoSpaces => "Characters (no spaces)",
        }
    }
}

impl From<String> for DisplayServer {
    fn from(s: String) -> Self {
        DisplayServer::from_code(&s).unwrap_or_default()
    }
}

impl From<DisplayServer> for String {
    fn from(d: DisplayServer) -> Self {
        d.code().to_string()
    }
}

/// A code that is none of the three is an error, not the default, so a settings reader reports it
/// (the legacy `ui.json` import lists it by its key) and the field reads as the default.
impl TryFrom<String> for StatusCount {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        StatusCount::from_code(&s).ok_or_else(|| format!("{s:?} is not a status count"))
    }
}

impl From<StatusCount> for String {
    fn from(c: StatusCount) -> Self {
        c.code().to_string()
    }
}

/// Whole-document counts the status bar shows (#2797), computed together whenever the document
/// changes: words ([`wordcraft_engine::Session::word_count`]), characters and characters without
/// spaces (the Word Count dialog's [`wordcraft_doc::count_chars`] and [`wordcraft_doc::count_chars_no_spaces`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub words: usize,
    pub chars: usize,
    pub chars_no_spaces: usize,
}

impl StatusCounts {
    /// The total in the unit the status bar is showing.
    pub fn in_unit(self, unit: StatusCount) -> usize {
        match unit {
            StatusCount::Words => self.words,
            StatusCount::Characters => self.chars,
            StatusCount::CharactersNoSpaces => self.chars_no_spaces,
        }
    }
}

/// `s` (a selection, say) counted in the unit the status bar is showing.
pub fn count_in_unit(s: &str, unit: StatusCount) -> usize {
    match unit {
        StatusCount::Words => wordcraft_doc::count_words(s),
        StatusCount::Characters => wordcraft_doc::count_chars(s),
        StatusCount::CharactersNoSpaces => wordcraft_doc::count_chars_no_spaces(s),
    }
}

/// The ribbon tab the app opens on, and the one it falls back to when the tab that was showing
/// no longer applies (a contextual tab after the selection moved away, a tab hidden in Customize
/// Ribbon). Word opens on Home every launch; see [`prefs::Prefs::tab`].
pub const DEFAULT_TAB: &str = "Home";

/// Most recent documents kept (File › Options › Advanced offers 0–50, as Word does).
pub const MAX_RECENT: usize = 50;
/// Most custom dictionary words read back from `ui.json`.
pub const MAX_DICTIONARY: usize = 10_000;
/// Fonts kept under Recently Used Fonts, about as many as Word shows.
pub const MAX_RECENT_FONTS: usize = 10;
/// The longest family name Recently Used Fonts keeps, the longest `format.font` accepts.
pub const MAX_FONT_NAME_BYTES: usize = 128;

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
pub(crate) fn lenient_styles_pane_dock<'de, D: serde::Deserializer<'de>>(d: D) -> Result<wordcraft_engine::StylesPaneDock, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_str().and_then(wordcraft_engine::StylesPaneDock::parse).unwrap_or_default())
}

/// Anything but four finite numbers reads as no saved rect; `craft_ui::panes` brings a saved one
/// back inside the window.
pub(crate) fn lenient_float_rect<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<[f32; 4]>, D::Error> {
    Ok(panes::float_rect(&Value::deserialize(d)?))
}

/// `recentFonts` as saved, through [`recent_fonts_from`]: an entry that isn't a name is dropped,
/// not the whole list.
pub(crate) fn lenient_recent_fonts<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
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
pub(crate) fn migrate_units(v: &mut Value) {
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
    pub ui: prefs::Prefs,
    pub services: Services,
    /// The app's host: files, jobs, settings sections and the language, shared with the framework.
    pub(crate) host: std::sync::Arc<craft::host::Host>,
    /// The saved preferences' store, shared with the host.
    pub(crate) settings: Option<craft::settings::SharedSettings<prefs::Prefs>>,
    /// What the settings store reported at load and at each save, for the status line and the notices.
    pub(crate) settings_notices: craft::ui::notices::SettingsNoticeState,
    /// The app's commands as the keymap knows them, and the registry they were built from
    /// ([`WordApp::catalog`]).
    pub(crate) command_catalog: Option<(std::sync::Arc<wordcraft_engine::Registry>, craft::ui::runtime::commands::AppCatalog)>,
    /// Whether the window's operating system was macOS when the catalog was built: the interface
    /// chords differ per platform, so the catalog is rebuilt when it changes.
    pub(crate) command_catalog_mac: bool,
    /// The interface language the catalog's labels were translated into: rebuilt when it changes.
    pub(crate) command_catalog_lang: &'static str,
    /// The session's key overrides, kept as one `Arc` while they do not change: the runtime
    /// rebuilds its keymap when the `Arc` it is handed changes ([`WordApp::overrides`]).
    pub(crate) key_overrides: std::sync::Arc<craft::keymap::Overrides>,
    /// The app-scope command states handed to the runtime, edited only when they change: the runtime resolves
    /// commands again only when their revision moves ([`WordApp::command_input`]).
    pub(crate) command_states: craft::ui::contract::Tracked<craft::ui::runtime::commands::CommandStates>,
    /// Precomputed command metadata, translated hints, and last gate results for [`WordApp::command_input`],
    /// avoiding per-frame allocations and catalog reparsing while idle.
    pub(crate) command_states_cache: Option<app::command_board::CommandStatesCache>,
    /// Cached keytip control lists and reserved shortcut letters per registry and active ribbon tab.
    pub(crate) keytip_cache: Option<keytips::KeytipCache>,
    /// Frames drawn so far, counted to 2: the control channel's queued requests wait until the
    /// fonts are warm.
    pub(crate) fonts_frames: u8,
    /// Only a successfully published copy may reuse this window's private fragment.
    pub clipboard_published: bool,
    pub printing: printing::State,
    print_feedback_seen: Option<printing::Feedback>,
    /// Which job message is current: bumped when a job starts and whenever the message changes,
    /// so a window can tell a new result from one it already said even when the text is the same.
    print_gen: u64,
    pub canvas: canvas::CanvasState,
    /// The Restrict Editing pane (#412).
    pub(crate) restrict: restrict_pane::RestrictPane,
    /// The language-aware spelling and grammar task pane.
    pub spelling_review: Option<proofing_pane::SpellingForm>,
    /// The status bar's message, until its `until` passes.
    pub(crate) status: Option<Status>,
    /// The latest formatting shortcut announcement for the active window.
    pub(crate) announcement: Option<announce::Announcement>,
    pub previews: previews::Previews,
    /// Media key of the picture a pending Change Picture replaces (#147).
    pub change_picture_target: Option<String>,
    /// Web: a recipient list was asked for (Select Recipients › Use an Existing List…), so the
    /// next text file from the picker loads as recipients instead of opening (#240).
    pub recipient_list_pending: bool,
    /// The font database's [`generation`](wordcraft_fonts::FontDb::generation) last seen: when
    /// fonts are added or removed (by any route) the font lists and the layout are refreshed.
    font_generation: u64,
    /// Focus (#1678): the ribbon is dropped down from the Focus bar, over the page. The shell
    /// owns the flag while it draws; the app reads it back after each pass.
    pub(crate) ribbon_dropped: bool,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) updates_rt: updates_ui::Runtime,
    /// The documents (by id) the missing-fonts download box has been considered for
    /// ([`font_download::tick`]), in every window.
    pub(crate) font_notice_seen: Vec<u64>,
    /// Whether that box comes up by itself ([`WordApp::tell_about_font_downloads`]).
    pub(crate) font_notices: bool,
    /// Language pack installs, the missing-pack bar ([`language_packs`]).
    pub(crate) packs_rt: language_packs::Runtime,
    /// The control channel's requests and replies, once a host started it.
    pub(crate) control: Option<craft::automation::control::ControlChannel>,
    /// The last screenshot token handed out; tokens start at 1.
    pub(crate) capture_token: u64,
    /// The Navigation pane's search box takes the keyboard on its next frame (Ctrl+F, #712).
    pub(crate) nav_search_focus: bool,
    /// The display scale (native pixels per point) seen in the last frame, 1.0 before the first;
    /// `ui.interfaceSize` reports the size Auto draws at with it.
    native_scale: f32,
    /// The most pixels per point Auto may draw at on this screen ([`theme::auto_scale_limit`], #4030).
    auto_scale_limit: f32,
    /// A pre-#822 interface size (percent on top of the display scale) waiting for the first frame
    /// that knows the display's scale, to become the absolute size it drew at
    /// ([`theme::migrate_legacy_interface_size`]).
    legacy_interface_size: Option<f64>,
    /// The interface language and the editing language the session last got for it
    /// ([`i18n::Lang::editing_lang`], or for English the region's English).
    applied_editing: Option<(i18n::Lang, Option<String>)>,
    pub frame_ms: f64,
    /// Nothing else (a dialog, menu, keytips, the File tab) is open to take Escape this frame, so
    /// Escape may leave full screen. Read before anything handles the frame's keys.
    escape_free: bool,
    pub quit_requested: bool,
    pub autosave: bool,
    /// The document's fonts that aren't installed, for the status bar and the font box, and the
    /// layout they were read from.
    pub missing_fonts: crate::chrome::MissingFontsMemo,
    /// This window's refused or failed export, shown on its Export and Print pages; see [`WordApp::fail_export`].
    pub export_error: String,
    /// The document's [`StatusCounts`] and the key they were computed for; see [`WordApp::cached_status_counts`].
    pub status_counts: (u64, StatusCounts),
    last_autosave: f64,
    /// The tab shown before the Equation tab came up (restored when editing ends).
    pub(crate) equation_prev_tab: Option<String>,
    /// SmartArt Text Pane: the item last focused (Promote, Demote and Add Shape act on it).
    pub smart_art_item: Option<usize>,
    /// The tab shown before Outline view brought up the Outlining tab (restored when it closes).
    pub(crate) outline_prev_tab: Option<String>,
    /// Zotero commands in flight (`ui.zotero.*`).
    pub zotero: zotero::ZoteroLink,
    /// The egui context, once the first frame has run (background work wakes the UI with it).
    pub(crate) ctx: Option<egui::Context>,
    /// Whether the current command was entered by an input method's own transaction. The engine
    /// uses it to keep IME commits from setting the application-change signal.
    pub(crate) ime_origin: bool,
    /// Whether the current command is a user's ([`WordApp::run`]), not a script's or an agent's
    /// ([`WordApp::execute`]): only a user's command opens the dialog an engine request asks for.
    pub(crate) user_origin: bool,
    /// Read Aloud: the start of the sentence the caret was last moved to, and the last error shown.
    pub(crate) read_aloud_at: Option<wordcraft_engine::doc::Pos>,
    pub(crate) read_aloud_error: Option<String>,
    /// The file the document was last explicitly saved to in this session; AutoSave writes only
    /// there. A file that was merely opened isn't rewritten until the user saves it: saving drops
    /// whatever WordCraft can't represent (content controls, charts, macros…).
    autosave_path: Option<std::path::PathBuf>,
    /// Web: saves the browser's picker hasn't finished yet (#1315).
    saves_in_flight: Vec<SaveInFlight>,
    /// The Save Changes prompt this window is running and the command it holds back.
    pub(crate) closing: Option<app::Closing>,
    /// The file dialog the host is showing, and what its answer is for ([`file_dialogs`]).
    file_dialog: Option<file_dialogs::PendingDialog>,
    /// Whether the in-app file chooser is forced in place of the system one.
    pub(crate) fallback_chooser: bool,
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
    /// The open state of the Quick Access Toolbar menu, the undo list, the status menus and the missing-fonts box.
    pub(crate) chrome: chrome::ChromeOpen,
    /// The File page's state.
    pub(crate) backstage_state: backstage::BackstageState,
    /// File › Options' state.
    pub(crate) options_state: craft::ui::patterns::preferences::PreferencesState,
    /// The Markdown Source pane's last visit.
    pub(crate) md_source_visit: u64,
    /// Keytips are showing (set from the keytips' output each pass).
    pub(crate) keytips_active: bool,
    /// The dialogs this window has open.
    pub dialog_host: craft::ui::dialog::DialogHost,
    /// What the window's dialogs ask of the app, by dialog.
    pub(crate) dialog_hooks: Vec<(craft::ui::dialog::DialogId, dialogs::host::DialogHooks)>,
    /// A finished child form's answer for its parent form, by parent dialog and form key; the parent's pass takes it.
    pub(crate) dialog_answers: Vec<(craft::ui::dialog::DialogId, String, serde_json::Value)>,
    /// Field gestures as undo steps ([`undo_marks`]).
    pub(crate) edits: craft::ui::edit::GestureHost<WordApp>,
    /// The Equation tab's symbol set.
    pub(crate) equation_symbol_set: usize,
    /// The gradient variation memory of Format Shape.
    pub(crate) shape_variation: Option<format_shape::Variation>,
    /// The File › Print page's state.
    pub(crate) print: print_page::PrintPage,
    /// The target of each open More Colors dialog, by its dialog key.
    pub(crate) more_colors: std::collections::HashMap<String, colors::Target>,
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
mod test_harness;
#[cfg(test)]
mod tests;
