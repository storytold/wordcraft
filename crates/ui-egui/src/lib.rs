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
pub mod i18n;
pub mod icons;
pub mod keys;
pub mod keytips;
pub mod mini_toolbar;
pub mod panes;
pub mod previews;
pub mod ribbon;
pub mod theme;
pub mod widgets;
pub mod window_geometry;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::Session;

pub use control::ControlRequest;

/// Platform services injected by the host (file dialogs, file I/O).
#[derive(Default)]
pub struct Services {
    /// Pick a file to open; the argument says what for (`document`, `picture`).
    pub pick_open: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Pick a path to save to, given a suggested name.
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Web: open a file picker; the file arrives later through `inbox`.
    pub open_async: Option<Box<dyn Fn(&str)>>,
    /// Web: files (name, bytes) delivered asynchronously (picker, drag and drop).
    pub inbox: Option<Inbox>,
    /// Web: hand bytes to the browser as a download.
    pub download: Option<Box<dyn Fn(&str, &[u8])>>,
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
    pub dark: bool,
    pub nav_tab: String,
    pub show_discord: bool,
    /// User name (File › Options) for comments and tracked changes; empty keeps the default.
    pub author: String,
    /// Desktop: the main window's size and position, restored at the next launch.
    pub window: Option<window_geometry::WindowGeometry>,
    /// Interface language: `auto` (follow the system) or a code from [`i18n::LANGUAGES`].
    pub language: String,
    /// Keytips (Alt) state; not persisted, it resets each session.
    #[serde(skip)]
    pub keytips: crate::keytips::Phase,
    /// View › Switch Modes: show pages dark (white text on black), kept between runs.
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
            dark: false,
            nav_tab: "headings".into(),
            show_discord: true,
            author: String::new(),
            window: None,
            language: i18n::AUTO.into(),
            keytips: crate::keytips::Phase::Off,
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
    fonts_frames: u32,
    applied_dark: Option<bool>,
    pub frame_ms: f64,
    /// The window title last sent; a viewport command schedules a repaint, so only send changes.
    sent_title: String,
    pub quit_requested: bool,
    pub autosave: bool,
    pub word_count: (u64, usize),
    last_autosave: f64,
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
            fonts_frames: 0,
            applied_dark: None,
            frame_ms: 0.0,
            sent_title: String::new(),
            quit_requested: false,
            autosave: true,
            word_count: (0, 0),
            last_autosave: 0.0,
            change_picture_target: None,
        }
    }

    pub fn with_control(mut self, rx: std::sync::mpsc::Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Preferences to save between runs: the UI state plus the current user name.
    pub fn prefs(&self) -> UiState {
        let mut ui = self.ui.clone();
        ui.author = self.session.author.clone();
        ui.dark_page = self.session.view.dark_mode;
        ui
    }

    /// Restore preferences saved by [`WordApp::prefs`].
    pub fn apply_prefs(&mut self, ui: UiState) {
        self.ui = ui;
        self.ui.backstage = false;
        // The session owns the name from here on; `prefs` copies it back when saving.
        let author = std::mem::take(&mut self.ui.author);
        if !author.trim().is_empty() {
            self.session.author = author;
        }
        self.session.view.dark_mode = self.ui.dark_page;
    }

    /// Run a command; UI-level commands (`ui.*`) are handled here, the rest by the engine.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // A pending Change Picture only survives until the next action (#147).
        if !matches!(id, "ui.changePicture" | "picture.change" | "insert.picture") {
            self.change_picture_target = None;
        }
        if let Some(r) = self.ui_command(id, &params) {
            return r;
        }
        // Web: saving and exporting become downloads.
        if self.services.download.is_some() && matches!(id, "file.save" | "file.saveAs" | "file.exportPdf" | "file.exportPng") {
            let name = params.get("path").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| self.default_save_name());
            let name = if id == "file.exportPdf" && !name.ends_with(".pdf") { format!("{name}.pdf") } else { name };
            let bytes = wordcraft_engine::io::save_bytes(&name, &self.session.doc)?;
            if let Some(d) = &self.services.download {
                d(&name, &bytes);
            }
            self.session.dirty = false;
            return Ok(json!({"downloaded": name, "bytes": bytes.len()}));
        }
        let r = self.session.run(id, &params).map_err(|e| e.to_string());
        self.after_command(id);
        if let Err(e) = &r {
            self.status(e.clone());
        }
        r
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
                "saveAs" => self.save_as_dialog(),
                "insertPicture" => self.pick_picture(),
                "print" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "print".into();
                }
                "options" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "options".into();
                }
                other => self.dialog = dialogs::Dialog::open(other, self),
            }
        }
        if req.get("close").is_some() {
            self.quit_requested = true;
        }
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
                self.ui.dark = p.get("value").and_then(Value::as_bool).unwrap_or(!self.ui.dark);
                json!({"dark": self.ui.dark})
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
            "ui.openFileDialog" => {
                self.open_dialog();
                json!({})
            }
            "ui.discord" => {
                self.canvas.open_url = Some("https://discord.gg/artcraft".into());
                json!({})
            }
            _ => return None,
        }))
    }

    pub fn status(&mut self, s: impl Into<String>) {
        self.status_msg = Some((s.into(), now_ms()));
    }

    fn open_dialog(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("document");
            return;
        }
        let picked = self.services.pick_open.as_ref().and_then(|f| f("document"));
        if let Some(path) = picked {
            let _ = self.run("file.open", json!({"path": path}));
            self.ui.backstage = false;
        }
    }

    pub fn save_as_dialog(&mut self) {
        let name = self.default_save_name();
        let picked = self.services.pick_save.as_ref().and_then(|f| f(&name));
        if let Some(path) = picked {
            let _ = self.run("file.save", json!({"path": path}));
        }
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

    fn pick_picture(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("picture");
            return;
        }
        let picked = self.services.pick_open.as_ref().and_then(|f| f("picture"));
        if let Some(path) = picked {
            let _ = self.insert_or_change_picture(json!({"path": path}));
        } else {
            self.change_picture_target = None;
        }
    }

    /// The name Save suggests: the open file's own Word format (so a .docm keeps its macros),
    /// otherwise .docx.
    fn default_save_name(&self) -> String {
        let ext = self.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_string());
        match ext {
            Some(e) if wordcraft_engine::io::is_word_package(&e) => format!("{}.{e}", self.display_title()),
            _ => format!("{}.docx", self.display_title()),
        }
    }

    /// [`title_stem`](Self::title_stem) as people see it (title bar, suggested file names): an
    /// untitled document is named in the interface language, e.g. `Dokument1`.
    pub fn display_title(&self) -> String {
        let stem = self.title_stem();
        if stem == "Document1" { tl!("Document1").to_string() } else { stem }
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

    /// New documents follow the interface language (a German interface makes German, A4
    /// documents). The first blank document exists before the language is known, so it is made
    /// again in the new language while nobody has touched it.
    fn follow_language(&mut self, lang: i18n::Lang) {
        if self.session.template_language == lang.code() {
            return;
        }
        self.session.template_language = lang.code().to_string();
        let s = &self.session;
        if s.path.is_none() && !s.dirty && !s.can_undo() && s.doc.word_count() == 0 {
            let _ = self.session.run("file.new", &json!({}));
        }
    }

    /// Per-frame logic before layout: control requests, screenshots, shortcuts, file drops.
    pub fn logic(&mut self, ctx: &egui::Context) {
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        self.follow_language(lang);
        // Chinese text wants the Chinese face before the Japanese one (one glyph style per line).
        if !self.styled || lang.prefers_hans() != self.fonts_hans {
            theme::install_fonts_for(ctx, lang.prefers_hans());
            self.fonts_hans = lang.prefers_hans();
            // Mod with -, = and 0 are Word shortcuts (optional hyphen, subscript, paragraph spacing);
            // egui's keyboard zoom would also scale the whole window on them. Zoom is View › Zoom.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            self.styled = true;
        }
        let dark = self.ui.dark || self.session.view.dark_mode;
        if self.applied_dark != Some(dark) {
            theme::apply(ctx, &if dark { theme::Tokens::dark() } else { theme::Tokens::light() });
            self.applied_dark = Some(dark);
        }
        self.drain_control(ctx);
        self.clear_stale_change_picture();
        self.drain_inbox();
        // AutoSave: write a saved document a couple of seconds after the last change.
        let now = now_ms();
        if self.autosave
            && self.session.dirty
            && self.session.path.is_some()
            && now - self.last_autosave > 2500.0
            && now - self.canvas.caret_visible_since > 1500.0
        {
            self.last_autosave = now;
            let ext = self.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if ext == "docx" || ext == "json" || ext == "odt" || ext == "rtf" {
                let _ = self.session.run("file.save", &json!({}));
            }
        }
        if self.autosave && self.session.dirty && self.session.path.is_some() {
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
        crate::keytips::show(self, &ctx, ui);
        keys::global_shortcuts(self, &ctx);
        if let Some(url) = self.canvas.open_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        let title = format!("{}{} - WordCraft", self.display_title(), if self.session.dirty { " •" } else { "" });
        if title != self.sent_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.sent_title = title;
        }
        self.frame_ms = now_ms() - t0;
    }

    /// Files that arrived asynchronously (web picker, drops): documents open, pictures insert.
    fn drain_inbox(&mut self) {
        let Some(inbox) = self.services.inbox.clone() else { return };
        let files = std::mem::take(&mut *inbox.lock().unwrap_or_else(|e| e.into_inner()));
        for (name, bytes) in files {
            let lower = name.to_ascii_lowercase();
            let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
            let img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"].iter().any(|e| lower.ends_with(e));
            let r =
                if img { self.insert_or_change_picture(json!({"data": data})) } else { self.run("file.open", json!({"path": name, "data": data})) };
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

pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
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
        let services =
            Services { download: Some(Box::new(move |n: &str, b: &[u8]| sink.borrow_mut().push((n.to_string(), b.to_vec())))), ..Default::default() };
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
        assert!(a.ui.dark);
        assert!(!a.ui.backstage);
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
}
