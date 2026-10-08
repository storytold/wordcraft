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
            quit_requested: false,
            autosave: true,
            word_count: (0, 0),
            last_autosave: 0.0,
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
    }

    /// Run a command; UI-level commands (`ui.*`) are handled here, the rest by the engine.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if let Some(r) = self.ui_command(id, &params) {
            return r;
        }
        // Web: saving and exporting become downloads.
        if self.services.download.is_some() && matches!(id, "file.save" | "file.saveAs" | "file.exportPdf" | "file.exportPng") {
            let name = params.get("path").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{}.docx", self.title_stem()));
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
        let name = self.title_stem() + ".docx";
        let picked = self.services.pick_save.as_ref().and_then(|f| f(&name));
        if let Some(path) = picked {
            let _ = self.run("file.save", json!({"path": path}));
        }
    }

    fn pick_picture(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("picture");
            return;
        }
        let picked = self.services.pick_open.as_ref().and_then(|f| f("picture"));
        if let Some(path) = picked {
            let _ = self.run("insert.picture", json!({"path": path}));
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
        let title = format!("{}{} - WordCraft", self.title_stem(), if self.session.dirty { " •" } else { "" });
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
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
            let r = if img { self.run("insert.picture", json!({"data": data})) } else { self.run("file.open", json!({"path": name, "data": data})) };
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

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
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

    #[test]
    fn prefs_without_user_name_keep_default() {
        let mut a = app();
        let default = a.session.author.clone();
        a.apply_prefs(serde_json::from_str(r#"{"dark": true, "backstage": true}"#).unwrap());
        assert_eq!(a.session.author, default);
        assert!(a.ui.dark);
        assert!(!a.ui.backstage);
    }
}
