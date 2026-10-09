//! WordCraft desktop app.
//!
//! Usage: `wordcraft [--control <port>] [--sample] [files…]`
//!
//! `--control <port>` (or `WORDCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"engine.execute","params":{"command":"text.insert","params":{"text":"Hi"}}}`.
//! Every request needs the key the app writes to `<config>/wordcraft/control-key.<instance>`
//! (mode 0600, removed on exit) as top-level `"key"`; see `docs/control-protocol.md`.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod control_server;

use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, UiState, WordApp};

struct App(WordApp, Option<std::path::PathBuf>);

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.0.logic(ctx);
        if self.0.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.0.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
    fn on_exit(&mut self) {
        save_prefs(&self.0);
        if let Some(p) = &self.1 {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn prefs_path() -> Option<std::path::PathBuf> {
    wordcraft_chat::config_dir().map(|b| b.join("ui.json"))
}

/// `<config>/wordcraft/control-key.<instance>` (mode 0600): the host tools' key for this instance
/// of the control channel. The MCP bridge computes the same path with `wordcraft_chat::control_key_path`.
fn control_key_path(port: u16) -> Option<std::path::PathBuf> {
    prefs_path().and_then(|p| p.parent().map(|d| wordcraft_chat::control_key_path(d, port)))
}

/// Write the key file (create_new, 0600). `Err` carries a message for the user.
fn write_control_key(p: &std::path::Path, key: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let _ = std::fs::remove_file(p);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(p)
            .map_err(|e| format!("cannot create {}: {e}", p.display()))?;
        f.write_all(key.as_bytes()).map_err(|e| format!("cannot write {}: {e}", p.display()))
    }
    #[cfg(not(unix))]
    std::fs::write(p, key).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

fn load_prefs(app: &mut WordApp) {
    if std::env::var_os("WORDCRAFT_NO_PREFS").is_some() {
        return;
    }
    if let Some(p) = prefs_path()
        && let Ok(bytes) = std::fs::read(&p)
        && let Ok(ui) = serde_json::from_slice::<UiState>(&bytes)
    {
        app.ui = ui;
        app.ui.backstage = false;
        if wordcraft_ui_egui::chat_pane::valid_owner_name(&app.ui.owner_name) {
            app.session.author = app.ui.owner_name.trim().to_string();
        }
    }
}

fn save_prefs(app: &WordApp) {
    if std::env::var_os("WORDCRAFT_NO_PREFS").is_some() {
        return;
    }
    if let Some(p) = prefs_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&app.ui) {
            let _ = std::fs::write(&p, bytes);
        }
    }
}

fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|purpose: &str| {
            let d = rfd::FileDialog::new();
            let d = if purpose == "picture" {
                d.add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
            } else {
                d.add_filter("Documents", &["docx", "docm", "dotx", "odt", "rtf", "txt", "md", "html", "htm", "json"])
                    .add_filter("Word document", &["docx"])
                    .add_filter("All files", &["*"])
            };
            d.pick_file().map(|p| p.to_string_lossy().to_string())
        })),
        pick_save: Some(Box::new(|name: &str| rfd::FileDialog::new().set_file_name(name).save_file().map(|p| p.to_string_lossy().to_string()))),
        ..Default::default()
    }
}

/// Window, Dock and taskbar icon.
fn app_icon() -> Option<egui::IconData> {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/wordcraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.wordcraft.png");
    eframe::icon_data::from_png_bytes(png).map_err(|e| log::warn!("app icon: {e}")).ok()
}

fn main() -> eframe::Result {
    let mut control_port: Option<u16> = std::env::var("WORDCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok());
    let mut files = Vec::new();
    let mut sample = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--control" => control_port = args.next().and_then(|p| p.parse().ok()),
            "--sample" => sample = true,
            "--version" => {
                println!(
                    "wordcraft {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    option_env!("WORDCRAFT_BUILD_SHA").map(|s| s.get(..8).unwrap_or(s)).unwrap_or("dev")
                );
                return Ok(());
            }
            _ => files.push(a),
        }
    }
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WordCraft")
            .with_app_id(option_env!("WORDCRAFT_APP_ID").unwrap_or("ai.storyteller.wordcraft"))
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    if let Some(icon) = app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    eframe::run_native(
        "WordCraft",
        options,
        Box::new(move |cc| {
            let doc = if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_doc::Document::new() };
            let mut app = WordApp::new(Session::new(doc), services());
            load_prefs(&mut app);
            app.integrated_titlebar = cfg!(target_os = "macos");
            let mut key_file = None;
            if let Some(port) = control_port {
                // Bind first: a failed bind must not touch any key file.
                match std::net::TcpListener::bind(("127.0.0.1", port)) {
                    Err(e) => eprintln!("wordcraft: control server failed to bind 127.0.0.1:{port}: {e}"),
                    Ok(listener) => match wordcraft_chat::keys::random_hex(32) {
                        Err(e) => eprintln!("wordcraft: no random key ({e}); control channel disabled"),
                        Ok(key) => {
                            match control_key_path(port) {
                                Some(p) => match write_control_key(&p, &key) {
                                    Ok(()) => key_file = Some(p),
                                    Err(e) => eprintln!("wordcraft: control key file not written ({e}); host tools cannot connect"),
                                },
                                None => eprintln!("wordcraft: no config directory; control key file not written, host tools cannot connect"),
                            }
                            // Chat language: WORDCRAFT_CHAT_LANG (en | pt), English by default.
                            let hub = wordcraft_chat::Hub::with_lang(key, wordcraft_chat::Lang::from_env());
                            let ctx = cc.egui_ctx.clone();
                            hub.set_notify(Box::new(move || ctx.request_repaint()));
                            eprintln!("wordcraft: control server listening on 127.0.0.1:{port}");
                            let rx = control_server::start_with(listener, cc.egui_ctx.clone(), hub.clone());
                            app = app.with_control(rx).with_chat(hub);
                            app.chat_logs_dir = wordcraft_chat::config_dir().map(|d| d.join("chat-logs"));
                        }
                    },
                }
            }
            for f in files {
                if let Err(e) = app.run("file.open", serde_json::json!({"path": f})) {
                    eprintln!("wordcraft: {f}: {e}");
                }
            }
            Ok(Box::new(App(app, key_file)))
        }),
    )
}
