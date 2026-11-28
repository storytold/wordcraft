//! The saved preferences: [`Prefs`] is the settings type of the `wordcraft` store, and
//! [`legacy_ui_json`] and [`import_legacy_window`] read the `ui.json` older builds wrote, once.

// PLAN: Cargo.toml of ui-egui needs craft-settings, craft-keymap, craft-font-manager, craft-platform,
// craft-print, toml_edit and craft-window's restore store; the app-hub owns the manifest.

use craft::settings::{MergeRule, SectionError, Settings};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::scroll::ScrollSpeed;
use crate::{DEFAULT_TAB, DisplayServer, MAX_RECENT, MAX_RECENT_FONTS, StatusCount, dialogs_insert, i18n, panes, print_page, theme};

/// The user's custom shortcuts. A wrapper because `craft::keymap::Overrides` has no `PartialEq`, which
/// [`Settings`] needs to see whether a save is due.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Shortcuts(pub craft::keymap::Overrides);

impl PartialEq for Shortcuts {
    fn eq(&self, other: &Self) -> bool {
        self.0.iter().eq(other.0.iter())
    }
}

/// UI state that persists between runs (and that agents can read and set).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// The ribbon tab showing. It is saved but not restored at startup: Word opens on Home every
    /// launch, and only going File and back to the editor keeps the tab ([`crate::WordApp::apply_prefs`]).
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
    /// Saved choices in Insert > Date & Time: format picture.
    pub date_time_format: String,
    /// Saved language tag in Insert > Date & Time.
    pub date_time_language: String,
    /// Saved preference for updating inserted date/time fields automatically (defaults to false).
    pub date_time_update: bool,
    /// Colours picked in the Colors dialog (More Colors…), most recent first (#606).
    pub recent_colors: Vec<String>,
    /// The font pickers' stars and Recently Used Fonts (the section at the top of the font list,
    /// #2068), shared by every font box.
    pub font_picker: craft::font_manager::picker::FontPickerSettings,
    /// Interface theme: light, dark, or follow the OS appearance (#115).
    pub theme: theme::Appearance,
    /// Interface size (#475, #822): Auto (the display scale the system reports) or an absolute
    /// size in percent. Junk saved values read as Auto; out-of-range ones are clamped to 50–300 %.
    #[serde(rename = "uiScale")]
    pub interface_size: theme::InterfaceSize,
    /// The interface size `ui.json` held before #822, on top of the display scale: read once to
    /// migrate ([`theme::legacy_interface_percent`]), never written. A valid `uiScale` wins.
    #[serde(rename = "interfaceSize", skip_serializing)]
    pub(crate) legacy_interface_size: Option<Value>,
    /// Animation effects (smooth scrolling, fades): on, off, or follow the system's setting, which
    /// is the default. An unknown saved value reads as the default.
    pub animations: theme::Animations,
    /// The dark-mode switch `ui.json` held before `theme` (#115): read once to migrate, never written.
    #[serde(rename = "dark", skip_serializing)]
    pub(crate) legacy_dark: Option<bool>,
    pub nav_tab: String,
    /// View › Navigation Pane: open or closed as the person left it, kept between runs (#2526).
    /// The session owns it while running (see [`crate::WordApp::prefs`]).
    pub nav_pane: bool,
    /// Where the Styles pane sits (#1756). The session owns it while running (see [`crate::WordApp::prefs`]).
    #[serde(deserialize_with = "crate::lenient_styles_pane_dock")]
    pub styles_pane_dock: wordcraft_engine::StylesPaneDock,
    /// The floating Styles pane's `[x, y, width, height]`, also kept while it is docked.
    #[serde(deserialize_with = "crate::lenient_float_rect", skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Interface language: `auto` (follow the system) or a code from [`i18n::LANGUAGES`].
    pub language: String,
    /// Editing preferences; the session owns them while running (see [`crate::WordApp::prefs`]).
    pub editing: wordcraft_engine::Prefs,
    /// Read Aloud speed (1 = normal) and whether it skips citations and bibliographies.
    pub read_aloud_rate: f32,
    pub read_aloud_skip_citations: bool,
    /// Show ribbon and Quick Access Toolbar KeyTips on Alt/F10 (#620).
    pub keytips_enabled: bool,
    /// View › Switch Modes: show pages dark (white text on black), kept between runs. Only the
    /// pages: the interface follows [`Prefs::theme`] (#312).
    pub dark_page: bool,
    /// File › Options › Proofing › Check spelling as you type, kept between runs (#1792).
    pub check_spelling: bool,
    /// File › Options › Advanced › Animation: the caret glides to its new place and its blink
    /// fades, as Word's does on Windows (on, the default), or it jumps and blinks on and off
    /// (off).
    pub caret_animation: bool,
    /// Custom keyboard shortcuts (Customize Keyboard, #368); the session owns them while running.
    pub keyboard: Shortcuts,
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
    /// the app opens without a document (#766); on, as in Word. See [`crate::WordApp::start_screen`].
    pub start_screen: bool,
    /// File › Options › Display: print the page colour (off, as in Word).
    pub print_backgrounds: bool,
    /// File › Options › Display: update fields (dates, page numbers, SEQ…) before printing.
    pub update_fields_before_print: bool,
    /// The custom dictionary (File › Options › Proofing, Add to Dictionary), kept between runs.
    pub dictionary: Vec<String>,
    /// File › Options › Advanced › Linux display server (#2184): read by the desktop app before
    /// the window opens.
    pub display_server: DisplayServer,
    /// What the status bar's live count shows (#2797): words (the default), characters with
    /// spaces, or characters without spaces. Right-click the count in the status bar, or File ›
    /// Options › Display. Junk saved values read as words.
    pub status_count: StatusCount,
    /// File › Options › General: tell about a document's missing fonts that can be downloaded
    /// ([`crate::font_download`]); the box's "Don't show this again" turns it off.
    pub font_download_notice: bool,
    /// How far a mouse-wheel notch scrolls (#6146).
    #[serde(default, alias = "scrollSpeed")]
    pub scroll_speed: ScrollSpeed,
    /// How far a touchpad swipe the system reports in points (Wayland, macOS) scrolls (#6146).
    #[serde(default, alias = "touchpadScrollSpeed")]
    pub touchpad_scroll_speed: ScrollSpeed,
}

impl Default for Prefs {
    /// Default user interface preferences matching Word defaults.
    fn default() -> Self {
        Prefs {
            tab: DEFAULT_TAB.into(),
            backstage: false,
            backstage_page: "home".into(),
            print: print_page::PrintSettings::default(),
            ribbon_collapsed: false,
            quick_access_toolbar: true,
            recent: Vec::new(),
            recent_symbols: Vec::new(),
            date_time_format: String::new(),
            date_time_language: String::new(),
            date_time_update: false,
            recent_colors: Vec::new(),
            font_picker: craft::font_manager::picker::FontPickerSettings::default(),
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
            language: i18n::AUTO.into(),
            editing: wordcraft_engine::Prefs::default(),
            read_aloud_rate: 1.0,
            read_aloud_skip_citations: true,
            keytips_enabled: true,
            dark_page: false,
            check_spelling: true,
            caret_animation: true,
            keyboard: Shortcuts::default(),
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
            display_server: DisplayServer::Auto,
            status_count: StatusCount::default(),
            font_download_notice: true,
            scroll_speed: ScrollSpeed::default(),
            touchpad_scroll_speed: ScrollSpeed::default(),
        }
    }
}

impl Prefs {
    /// How many recent documents to list.
    pub fn recent_shown(&self) -> usize {
        self.recent_count.min(MAX_RECENT)
    }
}

impl Settings for Prefs {
    const APP_ID: &'static str = "wordcraft";
    const SCHEMA_VERSION: u32 = 1;

    /// Recent lists merge as most-recently-used lists, user entry lists entry by entry, the
    /// groups key by key, and every other setting as a whole: this process's value wins.
    fn merge_rule(path: &[&str]) -> MergeRule {
        match path {
            [] | ["editing" | "keyboard" | "ribbon" | "panePlaces"] | ["editing", "markup" | "marks" | "autocorrect"] => MergeRule::ByKey,
            ["recent"] => MergeRule::Recent { cap: MAX_RECENT, key: text_key },
            ["recentSymbols"] => MergeRule::Recent { cap: dialogs_insert::RECENT_SYMBOLS, key: text_key },
            ["recentColors"] => MergeRule::Recent { cap: crate::colors::RECENT_MAX, key: text_key },
            ["fontPicker", "recent"] => MergeRule::Recent { cap: MAX_RECENT_FONTS, key: font_key },
            ["editing", "autocorrect", "entries"] => MergeRule::Entries { key: replaced_text },
            ["ribbon", "tabs"] => MergeRule::Entries { key: tab_name },
            ["editing", "autocorrect", "removed" | "exceptions" | "exceptionsRemoved" | "initialCapsExceptions"]
            | ["fontPicker", "favorites"]
            | ["ribbon", "qat"]
            | ["captionLabels" | "inkPens" | "dictionary"] => MergeRule::Entries { key: whole_entry },
            _ => MergeRule::Whole,
        }
    }
}

/// An entry that is its own identity.
fn whole_entry(v: &toml_edit::Value) -> Option<String> {
    Some(v.to_string().trim().to_string())
}

/// A recent-list entry by its text (a path, a colour) or, for a table, its whole text.
fn text_key(v: &toml_edit::Value) -> Option<String> {
    Some(v.as_str().map_or_else(|| v.to_string().trim().to_string(), str::to_string))
}

/// A recent font by its name, ignoring case as the list does.
fn font_key(v: &toml_edit::Value) -> Option<String> {
    v.as_str().map(|s| s.trim().to_lowercase())
}

/// An AutoCorrect replacement by the text it replaces.
fn replaced_text(v: &toml_edit::Value) -> Option<String> {
    let first = v.as_array().and_then(|a| a.get(0));
    Some(first.unwrap_or(v).to_string().trim().to_string())
}

/// A ribbon tab by its name, ignoring case as the ribbon does.
fn tab_name(v: &toml_edit::Value) -> Option<String> {
    let name = v.as_inline_table().and_then(|t| t.get("name")).and_then(|n| n.as_str());
    Some(name.map_or_else(|| v.to_string().trim().to_string(), str::to_lowercase))
}

/// Reads the `ui.json` older builds wrote: the [`LegacyFile::parse`](craft::settings::LegacyFile) of the
/// one-time import. A setting with a value of the wrong type (or a newer or older version's idea of
/// it) falls back to its default and is reported as a [`SectionError`] with its path; unknown keys are
/// dropped. `Err` only when the file isn't a JSON object.
pub fn legacy_ui_json(bytes: &[u8]) -> Result<(Prefs, Vec<SectionError>), String> {
    let mut v: Value = serde_json::from_slice(bytes).map_err(|e| format!("ui.json is not JSON: {e}"))?;
    let Some(map) = v.as_object_mut() else {
        return Err("ui.json is not a JSON object".into());
    };
    let keyboard = map.remove("keyboard");
    let recent_fonts = map.remove("recentFonts");
    migrate_units_of(&mut v);
    let (mut prefs, mut errors) = lenient(&v);
    if let Some(saved) = keyboard {
        match legacy_keyboard(&saved) {
            Some(overrides) => {
                // The framework's overrides never fail to load: a value that is no shortcut map loads as no
                // overrides and says what it dropped (`Overrides::load_notes`), so each note is reported.
                errors.extend(overrides.0.load_notes().iter().map(|note| SectionError { section: "keyboard".into(), message: format!("{note:?}") }));
                prefs.keyboard = overrides;
            }
            None => errors.push(SectionError { section: "keyboard".into(), message: "not a shortcut map".into() }),
        }
    }
    if let Some(list) = recent_fonts {
        // The one recent-fonts list is the font picker's; the old list joins it once.
        prefs.font_picker.recent = crate::lenient_recent_fonts(list).unwrap_or_default();
    }
    Ok((prefs, errors))
}

fn migrate_units_of(v: &mut Value) {
    crate::migrate_units(v);
}

/// The saved shortcuts in either shape: the old `{"assigned", "removed"}` map becomes overrides
/// through the command registry; a value already in the new shape reads as it is.
fn legacy_keyboard(saved: &Value) -> Option<Shortcuts> {
    // Only an object is a shortcut map; anything else is junk, whatever the new shape would take of it.
    if !saved.is_object() {
        return None;
    }
    if saved.get("assigned").is_some() || saved.get("removed").is_some() {
        let reg = wordcraft_engine::cmd::registry();
        return Some(Shortcuts(wordcraft_engine::keymap::overrides_from_legacy(&reg, saved)));
    }
    serde_json::from_value(saved.clone()).ok()
}

/// Deserializes `input` over the defaults one setting at a time: a value of the wrong type is
/// dropped (its default stays) and reported, instead of failing the whole read. Objects nest
/// (`editing`).
fn lenient(input: &Value) -> (Prefs, Vec<SectionError>) {
    let mut errors = Vec::new();
    let mut base = serde_json::to_value(Prefs::default()).unwrap_or(Value::Null);
    if base.is_object() {
        let mut extra = Vec::new();
        overlay(&mut base, &mut Vec::new(), input, 0, &mut extra, &mut errors);
        // Keys with no slot in the defaults (write-only legacy settings, or unknown ones) are
        // kept only after the loop, so `base` stays the size of `Prefs` while each key is checked.
        for (path, k, v) in extra {
            if let Some(parent) = object_at(&mut base, &path) {
                parent.insert(k, v);
            }
        }
    }
    (serde_json::from_value(base).unwrap_or_default(), errors)
}

/// The object at `path` inside `root`.
fn object_at<'a>(root: &'a mut Value, path: &[String]) -> Option<&'a mut Map<String, Value>> {
    let mut v = root;
    for k in path {
        v = v.as_object_mut()?.get_mut(k)?;
    }
    v.as_object_mut()
}

/// The report of the value at `path`/`key` that was reset.
fn reset(path: &[String], key: &str, error: &serde_json::Error) -> SectionError {
    let section = path.iter().map(String::as_str).chain([key]).collect::<Vec<_>>().join(".");
    SectionError { section, message: error.to_string() }
}

fn overlay(
    root: &mut Value,
    path: &mut Vec<String>,
    input: &Value,
    depth: usize,
    extra: &mut Vec<(Vec<String>, String, Value)>,
    errors: &mut Vec<SectionError>,
) {
    let Some(map) = input.as_object() else { return };
    for (k, v) in map {
        let Some(parent) = object_at(root, path) else { return };
        let old = parent.get(k).cloned();
        if depth < 4 && v.is_object() && old.as_ref().is_some_and(Value::is_object) {
            path.push(k.clone());
            overlay(root, path, v, depth + 1, extra, errors);
            path.pop();
            continue;
        }
        parent.insert(k.clone(), v.clone());
        let verdict = serde_json::from_value::<Prefs>(root.clone());
        match old {
            None => {
                if let Some(parent) = object_at(root, path) {
                    parent.remove(k);
                }
                match verdict {
                    Ok(_) => extra.push((path.clone(), k.clone(), v.clone())),
                    Err(e) => errors.push(reset(path, k, &e)),
                }
            }
            Some(o) => {
                if let Err(e) = verdict {
                    errors.push(reset(path, k, &e));
                    if let Some(parent) = object_at(root, path) {
                        parent.insert(k.clone(), o);
                    }
                }
            }
        }
    }
}

/// The `window` object of an old `ui.json`.
#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct LegacyWindow {
    position: Option<[f32; 2]>,
    width: f32,
    height: f32,
    maximized: bool,
}

impl Default for LegacyWindow {
    fn default() -> Self {
        LegacyWindow { position: None, width: 0.0, height: 0.0, maximized: false }
    }
}

/// Records the main window of an old `ui.json` in the window store, once. The old file knew no
/// monitor scale, so 1 is assumed; the store's own repair drops a size that is no size, and a
/// position a minimized window parked far off screen is dropped.
pub fn import_legacy_window(bytes: &[u8], store: &mut craft::window::restore::Store) {
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else { return };
    let Some(window) = v.get("window") else { return };
    let Ok(old) = serde_json::from_value::<LegacyWindow>(window.clone()) else { return };
    let position = old.position.filter(|p| p.iter().all(|c| c.is_finite() && *c > -30000.0 && *c < 1.0e6));
    let mut placement = craft::behavior::placement::Placement::new([old.width, old.height], 1.0);
    placement.normal.pos = position.unwrap_or([0.0; 2]);
    placement.position_px = position.map(|p| p.map(|c| c.round() as i32));
    placement.maximized = old.maximized;
    if let Some(repaired) = placement.repaired() {
        store.set(craft::window::restore::MAIN, &repaired);
    }
}

#[cfg(test)]
mod tests;
