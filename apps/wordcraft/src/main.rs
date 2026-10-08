//! WordCraft desktop app.
//!
//! Usage: `wordcraft [--control <port>] [--sample] [files…]`
//!
//! `--control <port>` (or `WORDCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"engine.execute","params":{"command":"text.insert","params":{"text":"Hi"}}}`.
//! See `docs/control-protocol.md`.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod control_server;

use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, UiState, WordApp};

struct App(WordApp);

/// Files macOS delivered through `application:openURLs:` (Finder "Open With", `open -a`).
#[cfg(target_os = "macos")]
static PENDING_OPENS: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());
/// Wakes the UI when files arrive while the app is running.
#[cfg(target_os = "macos")]
static OPEN_CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.0.logic(ctx);
        if self.0.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.0.raw_input_hook(raw);
        // Files opened from Finder open the same way as files given on the command line.
        #[cfg(target_os = "macos")]
        for p in PENDING_OPENS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default() {
            let path = p.to_string_lossy().to_string();
            if let Err(e) = self.0.run("file.open", serde_json::json!({"path": path})) {
                eprintln!("wordcraft: {path}: {e}");
            }
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
    fn on_exit(&mut self) {
        save_prefs(&self.0);
    }
}

fn prefs_path() -> Option<std::path::PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support/WordCraft"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("WordCraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
            .map(|c| c.join("wordcraft"))
    };
    base.map(|b| b.join("ui.json"))
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
            .with_app_id("ai.storyteller.wordcraft")
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
    // Before the event loop runs, so the files that launch the app are not missed.
    #[cfg(target_os = "macos")]
    wordcraft_macos_open::install(|paths| {
        if let Ok(mut pending) = PENDING_OPENS.lock() {
            pending.extend(paths);
        }
        if let Some(ctx) = OPEN_CTX.get() {
            ctx.request_repaint();
        }
    });
    eframe::run_native(
        "WordCraft",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            let _ = OPEN_CTX.set(cc.egui_ctx.clone());
            let doc = if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_doc::Document::new() };
            let mut app = WordApp::new(Session::new(doc), services());
            load_prefs(&mut app);
            app.integrated_titlebar = cfg!(target_os = "macos");
            if let Some(port) = control_port {
                let rx = control_server::start(port, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            for f in files {
                if let Err(e) = app.run("file.open", serde_json::json!({"path": f})) {
                    eprintln!("wordcraft: {f}: {e}");
                }
            }
            Ok(Box::new(App(app)))
        }),
    )
}
