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

pub mod backstage;
pub mod canvas;
pub mod chrome;
pub mod control;
pub mod credits;
pub mod dialogs;
pub mod dialogs_insert;
pub mod dialogs_lists;
pub mod dialogs_para;
pub mod equation_tab;
pub mod file_dialogs;
pub mod frame;
pub mod i18n;
pub mod icons;
pub mod keys;
pub mod keytips;
pub mod mini_toolbar;
pub mod objects;
pub mod panes;
pub mod paste_picture;
pub mod previews;
pub mod read_aloud;
pub mod ribbon;
pub mod scroll;
pub mod theme;
pub mod widgets;
pub mod window_geometry;
pub mod zotero;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::Session;

pub use control::ControlRequest;
pub use file_dialogs::FileDialogRequest;

/// Platform services injected by the host (file dialogs, file I/O).
#[derive(Default)]
pub struct Services {
    /// Pick a file to open; the argument says what for (`document`, `picture`).
    pub pick_open: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Pick a path to save to, given a suggested name.
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Desktop: show a native file dialog without blocking the UI thread (#94); used instead of
    /// `pick_open` / `pick_save` when set. The host sends the picked path (`None` when cancelled)
    /// through the returned channel once the user answers, and wakes the UI; a dropped sender
    /// counts as cancelled. The app shows one dialog at a time ([`file_dialogs`]).
    pub file_dialog: Option<Box<dyn Fn(FileDialogRequest) -> std::sync::mpsc::Receiver<Option<String>>>>,
    /// Web: open a file picker; the file arrives later through `inbox`.
    pub open_async: Option<Box<dyn Fn(&str)>>,
    /// Web: files (name, bytes) delivered asynchronously (picker, drag and drop).
    pub inbox: Option<Inbox>,
    /// Web: hand bytes to the browser as a download. An error means no download started (the
    /// browser can't tell the page whether the user then kept the file).
    pub download: Option<Box<dyn Fn(&str, &[u8]) -> Result<(), String>>>,
    /// Web: told whether the document has unsaved changes after each pass, for the browser's
    /// leave-page guard (`beforeunload` runs between frames and can't ask the app).
    pub on_dirty: Option<Box<dyn Fn(bool)>>,
    /// Hand PDF bytes to the system's print flow. Web: the browser's print dialog on that PDF (no
    /// download, no intermediate file). Desktop: the PDF opens in the system's viewer (a temporary
    /// file), where the user prints (#15). An error means nothing was opened.
    pub print: Option<Box<dyn Fn(&[u8]) -> Result<(), String>>>,
    /// Desktop: the picture on the system clipboard, if any — egui's paste only carries text
    /// (#45). Read when Paste finds no text; see [`paste_picture`].
    pub clipboard_picture: Option<Box<dyn Fn() -> Option<paste_picture::ClipboardPicture>>>,
}

/// Files delivered asynchronously.
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// UI state that persists between runs (and that agents can read and set).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    pub tab: String,
    pub backstage: bool,
    pub backstage_page: String,
    pub ribbon_collapsed: bool,
    pub recent: Vec<String>,
    /// Symbols inserted from the Symbol dialog, most recent first (#321).
    pub recent_symbols: Vec<dialogs_insert::RecentSymbol>,
    /// Interface theme: light, dark, or follow the OS appearance (#115).
    pub theme: theme::Appearance,
    /// The dark-mode switch `ui.json` held before `theme` (#115): read once to migrate, never written.
    #[serde(rename = "dark", skip_serializing)]
    pub(crate) legacy_dark: Option<bool>,
    pub nav_tab: String,
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
    /// Keytips (Alt) state; not persisted, it resets each session.
    #[serde(skip)]
    pub keytips: crate::keytips::Phase,
    /// Since the last Alt press, another key or a mouse button was pressed: the Alt release
    /// that follows ends a chord (Alt+click, Alt+drag column selection), not a keytip tap.
    #[serde(skip)]
    pub alt_chord_used: bool,
    /// View › Switch Modes: show pages dark (white text on black), kept between runs. Only the
    /// pages: the interface follows [`UiState::theme`] (#312).
    pub dark_page: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            tab: "Home".into(),
            backstage: false,
            backstage_page: "home".into(),
            ribbon_collapsed: false,
            recent: Vec::new(),
            recent_symbols: Vec::new(),
            theme: theme::Appearance::default(),
            legacy_dark: None,
            nav_tab: "headings".into(),
            show_discord: true,
            author: String::new(),
            window: None,
            language: i18n::AUTO.into(),
            editing: wordcraft_engine::Prefs::default(),
            read_aloud_rate: 1.0,
            read_aloud_skip_citations: true,
            keytips: crate::keytips::Phase::Off,
            alt_chord_used: false,
            dark_page: false,
        }
    }
}

/// The application.
pub struct WordApp {
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    pub canvas: canvas::CanvasState,
    pub dialog: Option<dialogs::Dialog>,
    pub status_msg: Option<(String, f64)>,
    pub previews: previews::Previews,
    /// Media key of the picture a pending Change Picture replaces (#147).
    pub change_picture_target: Option<String>,
    /// Web: a recipient list was asked for (Select Recipients › Use an Existing List…), so the
    /// next text file from the picker loads as recipients instead of opening (#240).
    pub recipient_list_pending: bool,
    /// macOS: the window has no title bar; leave room for the traffic lights.
    pub integrated_titlebar: bool,
    control_rx: Option<std::sync::mpsc::Receiver<ControlRequest>>,
    shot_token: u64,
    queued_shots: Vec<(u64, f64, u32)>,
    pending_shots: Vec<(u64, Option<String>, std::sync::mpsc::Sender<Value>, f64)>,
    pub(crate) synthetic: Vec<egui::Event>,
    /// Badge rects recorded during layout for the current keytip phase (cleared by `logic`, painted by `show`).
    pub(crate) keytip_rects: Vec<(egui::Rect, String)>,
    styled: bool,
    /// The CJK face order the installed UI fonts use (Chinese first, or Japanese first).
    fonts_hans: bool,
    /// Whether the installed UI fonts include an installed CJK fallback font (#241).
    fonts_system_cjk: bool,
    /// CJK text is (about to be) on screen in a non-CJK interface, e.g. the language names in
    /// File ▸ Options: load the installed CJK fallback font if no embedded face covers it.
    pub(crate) want_system_cjk: bool,
    fonts_frames: u32,
    /// The interface theme setting last installed in egui ([`theme::apply`]).
    applied_theme: Option<theme::Appearance>,
    pub frame_ms: f64,
    /// The window title last sent; a viewport command schedules a repaint, so only send changes.
    sent_title: String,
    pub quit_requested: bool,
    pub autosave: bool,
    pub word_count: (u64, usize),
    last_autosave: f64,
    /// The tab shown before the Equation tab came up (restored when editing ends).
    pub(crate) equation_prev_tab: Option<String>,
    /// Zotero commands in flight (`ui.zotero.*`).
    pub zotero: zotero::ZoteroLink,
    /// The egui context, once the first frame has run (background work wakes the UI with it).
    pub(crate) ctx: Option<egui::Context>,
    /// Read Aloud: the start of the sentence the caret was last moved to, and the last error shown.
    pub(crate) read_aloud_at: Option<wordcraft_engine::doc::Pos>,
    pub(crate) read_aloud_error: Option<String>,
    /// The file the document was last explicitly saved to in this session; AutoSave writes only
    /// there. A file that was merely opened isn't rewritten until the user saves it: saving drops
    /// whatever WordCraft can't represent (content controls, charts, macros…).
    autosave_path: Option<std::path::PathBuf>,
    /// The file dialog the host is showing, and what its answer is for ([`file_dialogs`]).
    file_dialog: Option<file_dialogs::PendingDialog>,
    /// File › Info: the property field being typed in and the document revision after its last
    /// keystroke, so one visit's keystrokes make a single undo step (and anything else in
    /// between, such as an Undo, starts a new one).
    pub(crate) info_editing: Option<(&'static str, u64)>,
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
    pub fn new(session: Session, services: Services) -> Self {
        WordApp {
            session,
            ui: UiState::default(),
            services,
            canvas: canvas::CanvasState::default(),
            dialog: None,
            status_msg: None,
            previews: previews::Previews::default(),
            integrated_titlebar: false,
            control_rx: None,
            shot_token: 0,
            queued_shots: Vec::new(),
            pending_shots: Vec::new(),
            synthetic: Vec::new(),
            keytip_rects: Vec::new(),
            styled: false,
            fonts_hans: false,
            fonts_system_cjk: false,
            want_system_cjk: false,
            fonts_frames: 0,
            applied_theme: None,
            frame_ms: 0.0,
            sent_title: String::new(),
            quit_requested: false,
            autosave: true,
            word_count: (0, 0),
            last_autosave: 0.0,
            equation_prev_tab: None,
            zotero: zotero::ZoteroLink::default(),
            ctx: None,
            read_aloud_at: None,
            read_aloud_error: None,
            autosave_path: None,
            change_picture_target: None,
            recipient_list_pending: false,
            file_dialog: None,
            info_editing: None,
        }
    }

    pub fn with_control(mut self, rx: std::sync::mpsc::Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Preferences to save between runs: the UI state plus the current user name and editing
    /// preferences.
    pub fn prefs(&self) -> UiState {
        let mut ui = self.ui.clone();
        ui.author = self.session.author.clone();
        ui.editing = self.session.prefs.clone();
        ui.read_aloud_rate = self.session.read_aloud.rate();
        ui.read_aloud_skip_citations = self.session.read_aloud.skip_citations;
        ui.dark_page = self.session.view.dark_mode;
        ui
    }

    /// Restore preferences saved by [`WordApp::prefs`].
    pub fn apply_prefs(&mut self, ui: UiState) {
        self.ui = ui;
        self.ui.backstage = false;
        // A `ui.json` from before the theme setting: its dark-mode switch picks Dark or Light.
        if let Some(dark) = self.ui.legacy_dark.take() {
            self.ui.theme = if dark { theme::Appearance::Dark } else { theme::Appearance::Light };
        }
        // The session owns the name from here on; `prefs` copies it back when saving.
        let author = std::mem::take(&mut self.ui.author);
        if !author.trim().is_empty() {
            self.session.author = author;
        }
        self.session.prefs = std::mem::take(&mut self.ui.editing);
        self.session.read_aloud.set_rate(self.ui.read_aloud_rate);
        self.session.read_aloud.skip_citations = self.ui.read_aloud_skip_citations;
        self.session.view.dark_mode = self.ui.dark_page;
    }

    /// Run a command the user asked for (ribbon, shortcut, Backstage, file drop). New, Open,
    /// Close, Envelopes, Labels and Finish & Merge on a document with unsaved changes first ask
    /// Save / Don't Save / Cancel, and the command runs once that is answered (`ui.saveChanges`).
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // A button whose command needs input the user hasn't given yet opens its dialog (#240).
        if let Some(name) = input_dialog(id, &params)
            && widgets::enabled(self, id)
        {
            self.dialog = dialogs::Dialog::open(name, self);
            return Ok(json!({"pending": name}));
        }
        if self.session.dirty && discards_document(id, &params) {
            let name =
                self.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.title_stem());
            self.dialog = Some(dialogs::Dialog::SaveChanges { name, then: id.to_string(), params, document: self.session.document_id() });
            return Ok(json!({"pending": "saveChanges"}));
        }
        // Paste without system clipboard text: a picture there is newer than our own copy (copying
        // here replaces it with text), so it wins (#45).
        if id == "edit.paste"
            && params.get("text").is_none()
            && let Some(r) = paste_picture::paste(self)
        {
            return r;
        }
        let r = self.execute_user(id, params);
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
        if let Some(r) = self.ui_command(id, &params) {
            return r;
        }
        if id == "file.autosave" {
            return self.set_autosave(&params);
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
            let name = params.get("path").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| self.default_save_name());
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
            let bytes = wordcraft_engine::io::save_bytes_with(&name, &self.session.doc, password)?;
            if let Some(d) = &self.services.download
                && let Err(e) = d(&name, &bytes)
            {
                let msg = i18n::fmt(tl!("Couldn't download {name}: {error}"), &[("name", &name), ("error", &e)]);
                log::error!("download of {name} failed: {e}");
                self.status(msg.clone());
                return Err(msg);
            }
            // An export is a copy, and so is a save in a format that doesn't keep everything (a
            // page image, plain text): the document itself still has unsaved changes.
            if matches!(id, "file.save" | "file.saveAs") && keeps_everything(&name) {
                self.session.dirty = false;
            }
            return Ok(json!({"downloaded": name, "bytes": bytes.len()}));
        }
        let document = self.session.document_id();
        let r = self.session.run(id, &params).map_err(|e| e.to_string());
        // When the document has been replaced (not when Open only showed its picker), a pending
        // "Save changes?" and AutoSave's go-ahead were both about the old one.
        if self.session.document_id() != document {
            self.autosave_path = None;
            if matches!(self.dialog, Some(dialogs::Dialog::SaveChanges { .. })) {
                self.dialog = None;
            }
        }
        // An explicit save to the document's own file lets AutoSave keep writing there.
        if let Ok(v) = &r
            && matches!(id, "file.save" | "file.saveAs")
            && v.get("saved").and_then(Value::as_bool) == Some(true)
            && v.get("path").and_then(Value::as_str).map(std::path::Path::new) == self.session.path.as_deref()
        {
            self.autosave_path = self.session.path.clone();
        }
        self.after_command(id);
        if let Err(e) = &r {
            self.status(e.clone());
        }
        r
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
        let go = match choice {
            SaveChoice::Cancel => false,
            SaveChoice::DontSave => true,
            // A new document: ask where through Save As; the command runs once that has saved,
            // which with the desktop's non-blocking dialog is on a later frame (#94).
            SaveChoice::Save if self.session.path.is_none() && self.services.download.is_none() => {
                return match self.save_as(file_dialogs::AfterSave::Continue { then, params, document }) {
                    file_dialogs::Asked::Done(r) => r,
                    file_dialogs::Asked::Pending => Ok(json!({"pending": "saveAs"})),
                    file_dialogs::Asked::Busy => Ok(json!({"done": false})),
                };
            }
            SaveChoice::Save => self.save_for_prompt(),
        };
        if !go {
            return Ok(json!({"done": false}));
        }
        self.execute_user(&then, params).map(|r| json!({"done": true, "result": r}))
    }

    /// [`WordApp::execute`] for something the user did: a document that turns out to be
    /// password-protected asks for the password (and opens with it) instead of failing (#55).
    pub(crate) fn execute_user(&mut self, id: &str, params: Value) -> Result<Value, String> {
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
    /// the command is cancelled and the document stays.
    fn save_for_prompt(&mut self) -> bool {
        let saved = match self.execute("file.save", json!({})) {
            Ok(v) => v.get("saved").and_then(Value::as_bool) == Some(true) || v.get("downloaded").is_some(),
            Err(_) => false,
        };
        self.safely_saved(saved)
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
        if self.quit_requested || !self.session.dirty {
            return true;
        }
        let _ = self.run("file.close", json!({}));
        false
    }

    fn after_command(&mut self, id: &str) {
        self.canvas.caret_visible_since = now_ms();
        if !id.starts_with("view.") && !id.starts_with("document.") && !id.starts_with("format.state") {
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
            self.ui.recent.truncate(12);
        }
        if !self.session.status.is_empty() {
            let s = std::mem::take(&mut self.session.status);
            self.status(s);
        }
    }

    /// Requests commands make of the UI (open a dialog…).
    fn handle_request(&mut self, req: &Value) {
        if let Some(what) = req.get("open").and_then(Value::as_str) {
            match what {
                "openFile" => self.open_dialog(),
                "saveAs" => {
                    self.save_as_dialog();
                }
                "insertPicture" => self.pick_picture(),
                "print" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "print".into();
                }
                "options" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "options".into();
                }
                "pasteSpecial" => self.dialog = Some(dialogs::Dialog::paste_special(self, req)),
                other => self.dialog = dialogs::Dialog::open(other, self),
            }
        }
        if req.get("close").is_some() {
            self.quit_requested = true;
        }
    }

    /// Whether the interface theme setting currently resolves to dark (`System` asks the OS).
    pub fn ui_is_dark(&self) -> bool {
        self.ui.theme.is_dark(self.ctx.as_ref().and_then(egui::Context::system_theme))
    }

    /// Commands that live in the UI layer.
    fn ui_command(&mut self, id: &str, p: &Value) -> Option<Result<Value, String>> {
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        Some(Ok(match id {
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
                json!({"backstage": self.ui.backstage})
            }
            "ui.dialog" => {
                self.dialog = dialogs::Dialog::open(s("name").unwrap_or(""), self);
                json!({"dialog": self.dialog.as_ref().map(|d| d.name())})
            }
            "ui.closeDialog" => {
                self.dialog = None;
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
            "ui.collapseRibbon" => {
                self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed;
                json!({"collapsed": self.ui.ribbon_collapsed})
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
            // Mailings › Select Recipients › Use an Existing List…: pick a CSV/TSV/text file and
            // load it as the mail-merge recipients. Opens system UI, like `ui.openFileDialog`.
            "ui.openRecipientList" => {
                self.pick_recipient_list();
                json!({"pending": self.recipient_list_pending})
            }
            // Send the document to the system print flow (web, desktop). `file.print` opens the Print
            // page; this is the button on it, and the one programmatic call that opens system UI
            // here, like `ui.openFileDialog` — only when the host can print.
            "ui.print" => return Some(self.print_to_system()),
            "ui.discord" => {
                self.canvas.open_url = Some("https://discord.gg/artcraft".into());
                json!({})
            }
            _ => return None,
        }))
    }

    /// `ui.print`: export the document to PDF in memory and hand it to the host's print hook.
    /// An error when the host can't print (File › Print then only saves a PDF).
    fn print_to_system(&mut self) -> Result<Value, String> {
        if self.services.print.is_none() {
            return Err("printing to the system print dialog isn't available here; export a PDF instead".into());
        }
        let result = wordcraft_engine::io::save_bytes("document.pdf", &self.session.doc)
            .and_then(|bytes| self.services.print.as_ref().map_or(Ok(()), |print| print(&bytes)).map(|()| bytes.len()));
        match result {
            Ok(len) => {
                self.status(tl!("Opening print dialog…"));
                Ok(json!({"printing": true, "bytes": len}))
            }
            Err(e) => {
                log::error!("print failed: {e}");
                let msg = i18n::fmt(tl!("Print failed: {error}"), &[("error", &e)]);
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

    /// Load mail-merge recipients and say how many there are.
    pub(crate) fn load_recipients(&mut self, params: Value) -> Result<Value, String> {
        let r = self.run("mailings.recipients", params);
        if let Ok(v) = &r {
            let records = v.get("records").and_then(Value::as_u64).unwrap_or(0).to_string();
            self.status(i18n::fmt(tl!("Recipients: {count}"), &[("count", &records)]));
        }
        r
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

    /// Document title for the title bar.
    pub fn title_stem(&self) -> String {
        match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Document1".into()),
            None => {
                if self.session.doc.core.title.is_empty() {
                    "Document1".into()
                } else {
                    self.session.doc.core.title.clone()
                }
            }
        }
    }

    /// Per-frame logic before layout: control requests, screenshots, shortcuts, file drops.
    pub fn logic(&mut self, ctx: &egui::Context) {
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        // Chinese text wants the Chinese face before the Japanese one (one glyph style per line).
        // An installed CJK font is read only once CJK text is shown (#241): never for an English
        // interface that doesn't open the language list.
        let system_cjk = self.fonts_system_cjk || self.want_system_cjk || lang.uses_cjk();
        if !self.styled || lang.prefers_hans() != self.fonts_hans || system_cjk != self.fonts_system_cjk {
            theme::install_fonts_with(ctx, lang.prefers_hans(), system_cjk);
            self.fonts_hans = lang.prefers_hans();
            self.fonts_system_cjk = system_cjk;
            // Mod with -, = and 0 are Word shortcuts (optional hyphen, subscript, paragraph spacing);
            // egui's keyboard zoom would also scale the whole window on them. Zoom is View › Zoom.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            self.styled = true;
        }
        // Both palettes are installed and egui picks one: with `System` it follows an OS appearance
        // change live (it reports it and repaints, #311). The interface follows the Interface theme
        // setting alone; Dark page only changes how pages are drawn (#312).
        if self.applied_theme != Some(self.ui.theme) {
            theme::apply(ctx, self.ui.theme);
            self.applied_theme = Some(self.ui.theme);
        }
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
        self.drain_control(ctx);
        zotero::poll(self, ctx);
        read_aloud::poll(self, ctx);
        self.clear_stale_change_picture();
        let _ = self.poll_file_dialog();
        self.drain_inbox();
        self.autosave_tick(now_ms());
        if self.autosaves() && self.session.dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(1000));
        }
        self.collect_screenshots(ctx);
        self.issue_screenshots(ctx);
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        }
        #[cfg(not(target_arch = "wasm32"))]
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            {
                let path = f.path().to_string_lossy().to_string();
                let lp = path.to_ascii_lowercase();
                let img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"].iter().any(|e| lp.ends_with(e));
                let _ = if img { self.run("insert.picture", json!({"path": path})) } else { self.run("file.open", json!({"path": path})) };
            }
        }
        self.report_dirty();
    }

    /// Tell the host whether the document has unsaved changes ([`Services::on_dirty`]).
    fn report_dirty(&self) {
        if let Some(f) = &self.services.on_dirty {
            f(self.session.dirty);
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

    /// Inject synthetic input events from the control channel (one pointer event per frame).
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        if self.synthetic.is_empty() {
            return;
        }
        let n = match self.synthetic.first() {
            Some(egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }) => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        if let Some(egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. }) = self.synthetic.first() {
            raw.events.push(egui::Event::PointerMoved(*p));
        }
        let n = n.min(self.synthetic.len());
        raw.events.extend(self.synthetic.drain(..n));
    }

    /// Lay out the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let t0 = now_ms();
        let ctx = ui.ctx().clone();
        i18n::set_current(i18n::Lang::from_pref(&self.ui.language));
        // Fonts installed by `logic` take effect on the next frame.
        if self.fonts_frames < 2 {
            self.fonts_frames += 1;
            ctx.request_repaint();
            return;
        }
        let t = theme::Tokens::get(&ctx);
        // Keytips are read before layout so the frame paints the new state; badges paint after
        // the ribbon so they sit on top of it.
        crate::keytips::logic(self, &ctx);
        if self.ui.backstage {
            backstage::show(self, ui);
        } else {
            chrome::title_bar(self, ui);
            ribbon::show(self, ui);
            chrome::status_bar(self, ui);
            panes::show(self, ui);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
                canvas::show(self, ui);
            });
        }
        dialogs::show(self, &ctx);
        zotero::show_alert(self, &ctx);
        read_aloud::show(self, &ctx);
        crate::keytips::show(self, &ctx, ui);
        keys::global_shortcuts(self, &ctx);
        if let Some(url) = self.canvas.open_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        let title = format!("{}{} - WordCraft", self.title_stem(), if self.session.dirty { " •" } else { "" });
        if title != self.sent_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.sent_title = title;
        }
        self.frame_ms = now_ms() - t0;
        // Typing and formatting land here, after `logic` has reported.
        self.report_dirty();
    }

    /// Files that arrived asynchronously (web picker, drops): documents open, pictures insert,
    /// recipient lists (CSV/TSV, or a text file asked for as one) load as recipients.
    fn drain_inbox(&mut self) {
        let Some(inbox) = self.services.inbox.clone() else { return };
        let files = std::mem::take(&mut *inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for (name, bytes) in files {
            let lower = name.to_ascii_lowercase();
            let img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"].iter().any(|e| lower.ends_with(e));
            let list =
                lower.ends_with(".csv") || lower.ends_with(".tsv") || (lower.ends_with(".txt") && std::mem::take(&mut self.recipient_list_pending));
            let r = if list {
                self.load_recipients(json!({"csv": wordcraft_engine::io::decode_text(&bytes)}))
            } else if img {
                self.insert_or_change_picture(json!({"data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            } else {
                self.run("file.open", json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}))
            };
            if r.is_ok() {
                self.ui.backstage = false;
            }
        }
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path } => {
                    self.shot_token += 1;
                    let token = self.shot_token;
                    self.queued_shots.push((token, now_ms() + 120.0, 0));
                    self.pending_shots.push((token, path, reply, now_ms() + 8000.0));
                }
            }
        }
        self.control_rx = Some(rx);
    }

    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        let now = now_ms();
        self.queued_shots.retain_mut(|(token, at, frames)| {
            *frames += 1;
            if now >= *at && *frames >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
                false
            } else {
                true
            }
        });
        if !self.queued_shots.is_empty() || !self.pending_shots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_shots.is_empty() {
            return;
        }
        let events: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                        Some((token, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in events {
            if let Some(i) = self.pending_shots.iter().position(|(t, ..)| *t == token) {
                let (_, path, reply, _) = self.pending_shots.remove(i);
                let _ = reply.send(control::save_screenshot(&image, path.as_deref()));
            }
        }
        let now = now_ms();
        self.pending_shots.retain(|(_, _, reply, deadline)| {
            if now < *deadline {
                return true;
            }
            let _ = reply.send(json!({"ok": false, "error": "no frame was presented (window hidden?); use ui.render"}));
            false
        });
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
/// copy that leaves it unsaved. AutoSave writes only these formats.
pub(crate) fn keeps_everything(name: &str) -> bool {
    let ext = std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "docx" | "docm" | "dotx" | "dotm" | "odt" | "rtf" | "json")
}

/// The dialog a user-run command opens when it lacks the input it needs (scripts and agents get
/// the command's own error or default instead): Select Recipients and Edit Recipient List without
/// data, Insert Merge Field without a field, Find Recipient without text, the If and Skip
/// Record If rules (or Rules with no rule at all) without a field, Table Properties without
/// settings, and Symbol and Field without a character or code.
fn input_dialog(id: &str, params: &Value) -> Option<&'static str> {
    let has = |k: &str| params.get(k).is_some_and(|v| !v.is_null());
    let rule = params.get("rule").and_then(Value::as_str).map(str::to_ascii_uppercase);
    match id {
        "mailings.rules" if !has("field") => match rule.as_deref() {
            None | Some("IF") => Some("ruleIf"),
            Some("SKIPIF") => Some("ruleSkipIf"),
            _ => None,
        },
        "mailings.recipients" if !["csv", "path", "rows"].into_iter().any(has) => Some("recipientList"),
        "mailings.editRecipients" if !has("rows") => Some("recipientList"),
        "mailings.insertField" if !has("field") => Some("insertMergeField"),
        "mailings.findRecipient" if !has("text") => Some("findRecipient"),
        // Symbol without a character and Field without a code show their dialogs (#321).
        "insert.symbol" if !has("char") => Some("symbol"),
        "insert.field" if !has("instr") => Some("field"),
        // Table Properties without settings shows the dialog (with settings it applies them).
        "table.properties" if params.as_object().is_none_or(|m| m.is_empty()) => Some("tableProperties"),
        // Tabs, Borders and Shading, and Page Borders without settings show their dialogs (#320).
        "para.tabs" if params.as_object().is_none_or(|m| m.is_empty()) => Some("tabs"),
        "para.borders" if params.as_object().is_none_or(|m| m.is_empty()) => Some("borders"),
        "design.pageBorders" if params.as_object().is_none_or(|m| m.is_empty()) => Some("pageBorders"),
        // `null` is an answer here: it removes the password.
        "file.encrypt" if params.get("password").is_none() => Some("encryptPassword"),
        // Define New Multilevel List and Track Changes Options without settings show their dialogs.
        "list.define" if params.get("levels").is_none() => Some("defineList"),
        "review.trackingOptions" if params.as_object().is_none_or(|m| m.is_empty()) => Some("trackChangesOptions"),
        _ => None,
    }
}

/// User commands that replace or close the document (`file.open` without a path only shows the
/// file picker; the open that follows is checked). Envelopes, Labels and Finish & Merge make a
/// new document in its place; a merge written to a file (`path`) leaves it alone.
fn discards_document(id: &str, params: &Value) -> bool {
    match id {
        "file.new" | "file.close" | "mailings.envelopes" | "mailings.labels" => true,
        "file.open" => params.get("path").is_some(),
        "mailings.finish" => params.get("path").and_then(Value::as_str).is_none(),
        _ => false,
    }
}

/// Wall-clock milliseconds since the Unix epoch: the system clock, or the browser's on the web.
pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        if ms.is_finite() && ms > 0.0 { ms } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
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
        let wheel = |unit, y: f32, phase| egui::Event::MouseWheel { unit, delta: egui::vec2(0.0, y), phase, modifiers: egui::Modifiers::NONE };
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

    fn prompt(a: &WordApp) -> Option<&'static str> {
        a.dialog.as_ref().map(|d| d.name())
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

    /// Surfaces finding 1: Mod+N (and the File › New tiles) replaced unsaved work without asking.
    #[test]
    fn new_asks_before_discarding_unsaved_work() {
        let mut a = typed();
        a.run("file.new", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        assert!(body_text(&a).contains(UNSAVED), "the document is untouched while the prompt is up");
        assert!(a.session.dirty);

        // Cancel: nothing happens.
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(body_text(&a).contains(UNSAVED));

        // Don't Save: the new document replaces it.
        a.run("file.new", json!({"template": "letter"})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert!(!body_text(&a).contains(UNSAVED));
        assert!(!a.session.dirty);
        assert!(!body_text(&a).trim().is_empty(), "the template the user picked was used");
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
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert_eq!(prompt(&a), Some("password"));
        assert!(body_text(&a).contains(UNSAVED), "nothing is replaced until the password fits");
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

    /// Open (after the file is picked), recent files and dropped files all go through `file.open`.
    #[test]
    fn open_asks_before_discarding_unsaved_work() {
        let dir = scratch("open");
        let path = docx_on_disk(&dir, "Signed contract");
        let mut a = typed();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        assert!(body_text(&a).contains(UNSAVED));
        assert_eq!(a.session.path, None);

        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(body_text(&a).contains("Signed contract"));
        assert_eq!(a.session.path.as_deref(), Some(path.as_path()));
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

    /// Mod+W ran `file.close`, which quit at once.
    #[test]
    fn close_asks_before_quitting() {
        let mut a = typed();
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        assert!(!a.quit_requested);
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(!a.quit_requested);

        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(a.quit_requested);

        // A clean document closes straight away.
        let mut clean = app();
        clean.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&clean), None);
        assert!(clean.quit_requested);
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

    /// Enter activates the focused prompt button; it means Save only when no button has focus.
    /// (Enter on a focused Cancel or Don't Save used to be overridden by a blanket Enter → Save.)
    #[test]
    fn enter_answers_with_the_focused_button() {
        use egui_kittest::kittest::Queryable;
        for (focus, quits, saves) in [(Some("Cancel"), false, false), (Some("Don't Save"), true, false), (None, true, true)] {
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
            assert_eq!(a.quit_requested, quits, "{focus:?}: quit");
            assert_eq!(std::fs::read(&path).unwrap() != before, saves, "{focus:?}: file written");
            let _ = std::fs::remove_dir_all(&dir);
        }
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
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Print").click();
                h.step();
                h.step();
                assert_eq!(printed.get(), 1, "the button printed");
            }
        }
    }

    /// A prompt about document A outlived an agent replacing A with B, and Don't Save then ran
    /// the pending New against B, throwing away B's edits under a question naming A.
    #[test]
    fn a_prompt_is_dropped_when_its_document_is_replaced() {
        let dir = scratch("stale");
        let path = docx_on_disk(&dir, "Agent document");
        let ctx = egui::Context::default();
        let mut a = typed();
        a.run("file.new", json!({})).unwrap();
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

    /// Save writes the document first, then carries on with what the user asked for.
    #[test]
    fn save_then_continue() {
        let dir = scratch("save");
        let path = docx_on_disk(&dir, "Draft");
        let mut a = app();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        a.run("file.new", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(prompt(&a), None);
        assert_eq!(a.session.path, None, "the new document replaced the saved one");
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
            a.run("file.new", json!({})).unwrap();
            a.status_msg = None;
            let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
            assert_eq!(r["done"], false, "{ext}: the New was cancelled");
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
        for (cmd, name) in [
            ("file.saveAs", "novel.png"),
            ("file.saveAs", "novel.txt"),
            ("file.saveAs", "novel.md"),
            ("file.exportPdf", "novel.pdf"),
            ("file.exportPng", "novel.png"),
        ] {
            let mut a = typed();
            a.services.download = Some(Box::new(|_, _| Ok(())));
            a.execute(cmd, json!({"path": name})).unwrap();
            assert!(a.session.dirty, "{cmd} {name}");
        }
        for name in ["novel.docx", "novel.docm", "novel.DOTX", "novel.dotm", "novel.odt", "novel.rtf"] {
            let mut a = typed();
            a.services.download = Some(Box::new(|_, _| Ok(())));
            a.execute("file.saveAs", json!({"path": name})).unwrap();
            assert!(!a.session.dirty, "{name}");
        }
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

    /// Web: the download callback couldn't report a failure, so Save marked the document saved
    /// and the prompt went ahead even when no download started.
    #[test]
    fn a_failed_download_is_not_a_save() {
        let mut a = typed();
        a.services.download = Some(Box::new(|_, _| Err("blocked by the browser".into())));
        assert!(a.execute("file.save", json!({})).is_err());
        assert!(a.session.dirty);

        a.run("file.new", json!({})).unwrap();
        a.status_msg = None;
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["done"], false, "the New was cancelled");
        assert!(body_text(&a).contains(UNSAVED));
        assert!(a.session.dirty);
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(msg.contains("blocked by the browser"), "the status bar says why: {msg:?}");
    }

    /// A new document has no path: Save goes through Save As, and cancelling that cancels the New.
    #[test]
    fn save_as_cancelled_keeps_the_document() {
        let mut a = typed();
        a.services.pick_save = Some(Box::new(|_| None));
        a.run("file.new", json!({})).unwrap();
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
        assert!(a.quit_requested);
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

    const MAILINGS: [&str; 3] = ["mailings.envelopes", "mailings.labels", "mailings.finish"];

    /// Envelopes, Labels and Finish & Merge replace the document with a new one, like New, but
    /// the ribbon ran them at once and the unsaved document was gone without a question.
    #[test]
    fn mailings_ask_before_replacing_unsaved_work() {
        for id in MAILINGS {
            let mut a = typed();
            a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            let document = a.session.document_id();
            a.run(id, json!({})).unwrap();
            assert_eq!(prompt(&a), Some("saveChanges"), "{id}");
            assert_eq!(a.session.document_id(), document, "{id}: untouched while the prompt is up");

            a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
            assert_eq!(prompt(&a), None);
            assert_eq!(a.session.document_id(), document, "{id}: Cancel keeps the document");
            assert!(body_text(&a).contains(UNSAVED));
            assert!(a.session.dirty);

            a.run(id, json!({})).unwrap();
            a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
            assert_eq!(prompt(&a), None);
            assert_ne!(a.session.document_id(), document, "{id}: Don't Save makes the new document");
            assert_eq!(a.session.path, None);
        }
        // A `path` that isn't text merges into a new document, like no path at all.
        let mut a = typed();
        a.run("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
        a.run("mailings.finish", json!({"path": null})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
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
            a.run(id, json!({})).unwrap();
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
    /// file the original came from, even after Save in the prompt wrote that file.
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
            a.run(id, json!({})).unwrap();
            a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
            assert_eq!(prompt(&a), None);
            let saved = std::fs::read(&path).unwrap();
            assert!(
                wordcraft_engine::io::open_path(&path).unwrap().plain_text(wordcraft_doc::StoryRef::Body).contains("Revised Again Signed"),
                "{id}: Save wrote the document first"
            );

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

    /// Review finding: with undo kept across the swap, an agent's Undo brought document A back
    /// under the id of the mailing result B while a prompt about B was up, and Don't Save then
    /// threw away A. The history starts afresh instead, so the prompt only ever discards B.
    #[test]
    fn undo_after_a_mailing_cannot_redirect_a_pending_prompt() {
        for id in MAILINGS {
            let mut a = typed();
            a.execute("mailings.recipients", json!({"csv": "Name\nAda"})).unwrap();
            a.execute(id, json!({})).unwrap();
            let result = body_text(&a);
            let b = a.session.document_id();
            assert!(a.session.dirty, "{id}: the result is unsaved");
            a.run("file.new", json!({})).unwrap();
            assert_eq!(prompt(&a), Some("saveChanges"), "{id}");

            a.execute("edit.undo", json!({})).unwrap();
            assert_eq!(a.session.document_id(), b, "{id}");
            assert_eq!(body_text(&a), result, "{id}: nothing from before the mailing comes back");
            a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
            assert_ne!(a.session.document_id(), b, "{id}: Don't Save discarded the document it asked about");
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
            a.run(id, json!({})).unwrap();
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
        for (saved, want) in [(r#"{"dark": false}"#, Appearance::Light), (r#"{"dark": true}"#, Appearance::Dark), ("{}", Appearance::Light)] {
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
        assert_eq!((odd.theme, odd.tab.as_str()), (Appearance::Light, "Insert"));
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
        assert_eq!((a.ui.tab.as_str(), a.ui.theme), ("Insert", theme::Appearance::Dark));
        assert!(a.session.prefs.count_notes, "text boxes and notes count by default");
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
        // A cancelled picker leaves nothing pending once the user does something else.
        a.run("ui.openRecipientList", json!({})).unwrap();
        a.run("format.bold", json!({})).unwrap();
        assert!(!a.recipient_list_pending);
    }
}
