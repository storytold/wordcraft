//! WordCraft desktop app.
//!
//! Usage: `wordcraft [--control <port>] [--sample] [files…]`; `--help` prints `USAGE`.
//!
//! `--control <port>` (or `WORDCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"key":"…","method":"engine.execute","params":{"command":"text.insert","params":{"text":"Hi"}}}`.
//! Every request needs the key the app writes to `<settings>/control-key.<instance>` (mode 0600,
//! removed on exit). See `docs/control-protocol.md`.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod animations;
#[cfg(target_os = "macos")]
mod apple_events;
mod clipboard;
mod control_server;
mod default_apps;
mod file_dialogs;
#[cfg(any(target_os = "windows", test))]
mod graphics;
mod logging;
mod print;
mod region;
#[cfg(target_os = "macos")]
mod traffic_lights;

use wordcraft_engine::Session;
use wordcraft_ui_egui::{
    Services, UiState, WordApp,
    window_geometry::{WindowGeometry, take_rescue},
};

/// The app, the restored window geometry (or, with none, the default: no position) until the
/// first frame that knows the monitor has checked it, the control server's key file (removed on
/// exit), the file dialogs the app asked for and, on macOS, the documents opened from Finder.
struct App(
    WordApp,
    Option<WindowGeometry>,
    Option<control_server::KeyFile>,
    file_dialogs::Launcher,
    #[cfg(target_os = "macos")] fmv_macos_events::Inbox,
);

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        apple_events::poll(&self.4, &mut self.0, ctx);
        self.0.logic(ctx);
        self.3.show(frame);
        // A window that doesn't fit on its screen (the default size on a laptop) shrinks to fit
        // there, so all of the scroll bar can be dragged (#1215). Checked on the frame that first
        // knows the monitor: the off-screen check below takes the pending geometry then.
        if let Some((pos, size)) = ctx.input(|i| self.1.and_then(|g| g.fit_on_monitor(i.viewport()))) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        let prev = self.0.ui.window;
        self.0.ui.window = ctx.input(|i| WindowGeometry::track(prev, i.viewport(), i.viewport_rect().size()));
        // The first frame, once the monitor is known (as `take_rescue` waits for): maximize a
        // window that opened at its normal size so it never flashed on screen half-opened.
        let maximize = self.1.and_then(|g| g.maximize_after_open()).filter(|_| ctx.input(|i| i.viewport().monitor_size.is_some()));
        if let Some(pos) = ctx.input(|i| take_rescue(&mut self.1, i.viewport())) {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
        if let Some(command) = maximize {
            ctx.send_viewport_cmd(command);
        }
        // The window's close button: with unsaved changes, ask first (`WordApp::close_requested`).
        let closing = ctx.input(|i| i.viewport().close_requested());
        if closing && !self.0.close_requested() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if (closing || self.0.quit_requested) && !self.0.printing.busy() {
            // On macOS, letting eframe drop the window inside `-[NSApplication run]` aborts in
            // `-[_NSTouchBarFinderObservation invalidate]`. Command-Q calls `terminate:`. Once
            // the close is allowed, take that path instead of dropping the window on this turn.
            #[cfg(target_os = "macos")]
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.0.quit_requested = false;
                schedule_macos_terminate(&self.0, self.2.as_ref());
            }
            #[cfg(not(target_os = "macos"))]
            if self.0.quit_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.0.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.0.ui(ui);
        self.3.show(frame);
    }
    fn on_exit(&mut self) {
        print::finish_pending_jobs();
        save_prefs(&self.0);
        if let Some(key_file) = &self.2 {
            key_file.remove();
        }
    }
}

/// Quit the way Command-Q does, on the next turn of the main queue.
///
/// Called when the window is closing. Doing it on the next turn lets the close
/// button's display cycle finish while the window still exists; `terminate:`
/// then ends the process. Preferences are written and the control key file is
/// removed first, as `on_exit` does, because a fallback [`std::process::exit`]
/// skips `on_exit`. A second close while a quit is already scheduled is ignored.
#[cfg(target_os = "macos")]
fn schedule_macos_terminate(app: &WordApp, key_file: Option<&control_server::KeyFile>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    if app.printing.busy() {
        return;
    }
    static QUIT: AtomicBool = AtomicBool::new(false);
    if QUIT.swap(true, Ordering::Relaxed) {
        return;
    }
    log::info!("window close: quitting");
    // This path can bypass on_exit. Join the owned workers before termination and cleanup.
    print::finish_pending_jobs();
    save_prefs(app);
    if let Some(key_file) = key_file {
        key_file.remove();
    }
    dispatch2::DispatchQueue::main().exec_async(|| {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(mtm).terminate(None);
        }
        // `terminate:` does not return when it succeeds. If it did, leave anyway:
        // staying up after the close was cancelled would trap the user in the window.
        std::process::exit(0);
    });
}

/// `ui.json` in the settings folder, which the MCP bridge also uses to find the control key.
fn prefs_path() -> Option<std::path::PathBuf> {
    wordcraft_control_key::settings_dir().map(|b| b.join("ui.json"))
}

/// Where the log files live: `logs` in the preferences folder (see `logging`).
fn log_dir() -> Option<std::path::PathBuf> {
    Some(prefs_path()?.parent()?.join("logs"))
}

/// `custom-dictionary.txt` in the settings folder: the words Add to Dictionary kept (#40).
fn user_dict_path() -> Option<std::path::PathBuf> {
    wordcraft_control_key::settings_dir().map(|b| b.join(wordcraft_engine::Session::USER_DICT_FILE))
}

fn load_user_dictionary(app: &mut WordApp) {
    if !prefs_enabled() {
        return;
    }
    if let Some(p) = user_dict_path() {
        wordcraft_proof::load_user_dictionary(&p);
        app.session.user_dict_path = Some(p);
    }
}

/// Runs without preferences (`WORDCRAFT_NO_PREFS`, agents' test runs) neither read nor write them.
fn prefs_enabled() -> bool {
    std::env::var_os("WORDCRAFT_NO_PREFS").is_none()
}

fn load_prefs(app: &mut WordApp) {
    if !prefs_enabled() {
        return;
    }
    if let Some(p) = prefs_path()
        && let Ok(bytes) = std::fs::read(&p)
        && let Some(ui) = UiState::from_json(&bytes)
    {
        app.apply_prefs(ui);
    }
}

/// Fonts the user added (#582) live in `fonts` in the settings folder: load them, and store fonts
/// added later there. Runs without preferences neither read nor keep them.
fn load_user_fonts() {
    if !prefs_enabled() {
        return;
    }
    let Some(dir) = wordcraft_control_key::settings_dir().map(|d| d.join("fonts")) else { return };
    let (loaded, skipped) = wordcraft_fonts::FontDb::global().load_user_fonts(&dir);
    if !loaded.is_empty() || !skipped.is_empty() {
        log::info!("user fonts: {} loaded, {} skipped from {}", loaded.len(), skipped.len(), dir.display());
    }
}

/// The saved window geometry, read before the window opens (`load_prefs` runs after).
fn saved_window() -> Option<WindowGeometry> {
    if !prefs_enabled() {
        return None;
    }
    let bytes = std::fs::read(prefs_path()?).ok()?;
    UiState::from_json(&bytes)?.window?.sanitized()
}

fn save_prefs(app: &WordApp) {
    if !prefs_enabled() {
        return;
    }
    if let Some(p) = prefs_path() {
        if let Some(dir) = p.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!("preferences: cannot create {}: {e}", dir.display());
            return;
        }
        match serde_json::to_vec_pretty(&app.prefs()) {
            Ok(bytes) => {
                if let Err(e) = write_prefs(&p, &bytes) {
                    log::warn!("preferences: cannot save {}: {e}", p.display());
                }
            }
            Err(e) => log::warn!("preferences: cannot serialize: {e}"),
        }
    }
}

/// Writes `bytes` to a sibling temporary file and renames it over `path`, so a failed or
/// interrupted write leaves the previous preferences in place.
fn write_prefs(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "ui.json".into());
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Native file dialogs, shown without blocking the window (`file_dialogs`, #94), File › Print
/// through the system spooler (`print`), the system clipboard's text, formatted copies,
/// pictures and format names (`clipboard`, #45, #584, #687, #833), the system's animation setting
/// (`animations`), and making WordCraft the default app for Word documents (`default_apps`, #295).
fn services(file_dialog: file_dialogs::Hook, ctx: &egui::Context) -> Services {
    Services {
        file_dialog: Some(file_dialog),
        printers: Some(print::discover()),
        print_native: Some(print::submit()),
        clipboard_picture: Some(Box::new(clipboard::picture)),
        make_default_app: default_apps::hook(),
        animations_enabled: Some(animations::hook(ctx)),
        clipboard_text: Some(Box::new(clipboard::text)),
        clipboard_html: Some(Box::new(clipboard::html)),
        set_clipboard_html: Some(Box::new(clipboard::set_html)),
        clipboard_rtf: Some(Box::new(clipboard::rtf)),
        clipboard_formats: Some(Box::new(clipboard::formats)),
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

const USAGE: &str = "\
wordcraft — the WordCraft desktop app

USAGE:
  wordcraft [--sample] [--control PORT] [FILE…]

OPTIONS:
  --sample            open the sample document
  --control PORT      start the localhost control server on PORT, for agents and scripts
                      (or set WORDCRAFT_CONTROL_PORT); see docs/control-protocol.md
  --version           print the version
  -h, --help          print this help
";

/// The commit the build came from (short), or `dev`.
fn build_sha() -> &'static str {
    option_env!("WORDCRAFT_BUILD_SHA").map(|s| s.get(..8).unwrap_or(s)).unwrap_or("dev")
}

/// The arguments as text, and apart the files whose names aren't valid Unicode, such as a Latin-1
/// one (#1440). A `--control` port that isn't is an error.
fn text_args() -> (Vec<String>, Vec<std::path::PathBuf>) {
    let (mut args, mut files) = (Vec::new(), Vec::new());
    for (i, a) in std::env::args_os().skip(1).enumerate() {
        match a.into_string() {
            Ok(a) => args.push(a),
            Err(a) if args.last().is_some_and(|p| p == "--control") => {
                // Unlike `eprintln!`, never panics.
                let (n, lossy) = (i + 1, a.to_string_lossy());
                let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("wordcraft: argument {n} is not valid Unicode: {lossy}\n"));
                std::process::exit(2);
            }
            Err(a) => files.push(a.into()),
        }
    }
    (args, files)
}

/// Open a file whose name isn't valid Unicode (#1440). `file.open` takes its path as text, so it
/// gets the file's bytes under the lossy name, and the document then keeps the real path to save to.
fn open_non_unicode(app: &mut WordApp, path: &std::path::Path) {
    let name = path.to_string_lossy();
    let opened = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| app.run("file.open", serde_json::json!({"path": name, "data": wordcraft_engine::cmd::insert::base64_encode(&bytes)})));
    match opened {
        // Not when it waits for a password: that open names the document itself.
        Ok(r) if r.get("pending").is_none() => app.session.path = Some(path.into()),
        Ok(_) => {}
        Err(e) => log::warn!("{name}: {e}"),
    }
}

fn main() -> eframe::Result {
    // First, so the panic hook and every start-up warning are recorded (`logging`).
    let logger = logging::install();
    logging::install_panic_hook();
    let mut control_port: Option<u16> = std::env::var("WORDCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok());
    let mut files = Vec::new();
    let mut sample = false;
    let (args, other_files) = text_args();
    let mut bad_control: Option<Option<String>> = None;
    let mut args = args.into_iter().peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            // A flag after `--control` stays a flag; a bad or missing port is an error (#1426).
            "--control" => {
                let port = args.next_if(|p| p.strip_prefix('-').is_none_or(|n| n.starts_with(|c: char| c.is_ascii_digit())));
                match port.as_deref().and_then(|p| p.parse::<std::num::NonZeroU16>().ok()) {
                    Some(p) => control_port = Some(p.get()),
                    None => bad_control = Some(port),
                }
            }
            "--sample" => sample = true,
            "--version" => {
                println!("wordcraft {} ({})", env!("CARGO_PKG_VERSION"), build_sha());
                return Ok(());
            }
            // Before the window opens (#968). Unlike `print!`, a closed stdout can't panic.
            "--help" | "-h" => {
                let _ = std::io::Write::write_all(&mut std::io::stdout(), USAGE.as_bytes());
                return Ok(());
            }
            _ => files.push(a),
        }
    }
    // After the loop, so `--version` later on the line still wins. Unlike `eprintln!`, never panics.
    if let Some(port) = bad_control {
        let got = port.map(|p| format!(", got {p:?}")).unwrap_or_default();
        let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("wordcraft: --control needs a port number from 1 to 65535{got}\n"));
        std::process::exit(2);
    }
    // The log file lives in the settings directory, next to the preferences; opened after the
    // arguments, so `--version` leaves no file behind. Records logged until now are written to it
    // first. Runs without preferences (agents' test runs) log to standard error only, so they
    // don't rotate away the user's own logs.
    if let Some(logger) = logger {
        match log_dir().filter(|_| prefs_enabled()) {
            Some(dir) => match logger.attach_dir(&dir) {
                Ok(path) => log::info!("WordCraft {} ({}), log file {}", env!("CARGO_PKG_VERSION"), build_sha(), path.display()),
                // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => logger.no_file(),
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
    // Before eframe creates the wgpu instance: default Windows to DirectX 12 only, or OpenGL when
    // DirectX 12 has no GPU driver or this is a 32-bit build, and compile DirectX 12 shaders with
    // FXC, which needs no DLL of ours (#603; see graphics.rs).
    #[cfg(target_os = "windows")]
    graphics::configure(&mut options, eframe::wgpu::Backends::from_env());
    load_user_fonts();
    let restored = saved_window();
    if let Some(window) = restored {
        options.viewport = window.apply(options.viewport);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    // winit has no file drag-and-drop on Wayland (only on X11), so dropping files from the file
    // manager showed a "no" cursor. Run through XWayland when it's there; WORDCRAFT_WAYLAND=1
    // keeps the native Wayland backend.
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("WORDCRAFT_WAYLAND").is_none() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    // Registered before the event loop starts, so it catches the Finder event that launched us as
    // well as later ones. Lives until the event loop returns; the app creator only borrows it.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;
    eframe::run_native(
        "WordCraft",
        options,
        Box::new(move |cc| {
            // The system region picks the new document's paper, the units and the decimal
            // separator (#838).
            let region = region::system();
            let doc = if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_engine::blank_document(&region) };
            let (file_dialog, dialogs) = file_dialogs::hook(&cc.egui_ctx);
            let mut session = Session::new(doc);
            session.system_region = region;
            let mut app = WordApp::new(session, services(file_dialog, &cc.egui_ctx));
            load_prefs(&mut app);
            load_user_dictionary(&mut app);
            // LawCraft, an add-in off until the user turns it on (File › Options › Add-ins): its
            // profiles live in the settings folder and its commands come with it.
            wordcraft_law::set_profiles_dir(wordcraft_law::settings_dir());
            app.add_addin(Box::new(wordcraft_law_ui::LawAddIn::default()));
            // A region chosen in File › Options replaces the system's for the first blank
            // document too (#838).
            if !sample && app.session.prefs.region.is_some() {
                let doc = app.session.blank_document();
                app.session.set_document(doc);
            }
            app.watch_desktop_theme(&cc.egui_ctx);
            if prefs_enabled()
                && let Some(dir) = wordcraft_control_key::settings_dir()
            {
                let ctx = cc.egui_ctx.clone();
                if let Err(e) = app.session.enable_recovery(dir.join("recovery"), std::sync::Arc::new(move || ctx.request_repaint())) {
                    log::warn!("local recovery unavailable: {e}");
                }
            }
            app.ui.window = restored;
            app.integrated_titlebar = cfg!(target_os = "macos");
            #[cfg(target_os = "macos")]
            if let Some(bar) = traffic_lights::title_bar() {
                log::info!("title bar: {bar:?}");
                app.mac_titlebar = bar;
            }
            let mut key_file = None;
            if let Some(port) = control_port
                && let Some((rx, file)) = control_server::start(port, cc.egui_ctx.clone(), wordcraft_control_key::settings_dir().as_deref())
            {
                app = app.with_control(rx);
                key_file = file;
            }
            // Nothing to open (no files, no sample, no control channel driving the editor): start
            // on File › Home, as Word does (#766), unless the Start screen option is off. Files
            // macOS hands over later open on top of it and close it (`apple_events`).
            if files.is_empty() && other_files.is_empty() && !sample && control_port.is_none() {
                app.start_screen();
            }
            for f in files {
                if let Err(e) = app.run("file.open", serde_json::json!({"path": f})) {
                    log::warn!("{f}: {e}");
                }
            }
            for f in other_files {
                open_non_unicode(&mut app, &f);
            }
            Ok(Box::new(App(
                app,
                // With nothing restored, the window the system placed is checked all the same.
                Some(restored.unwrap_or_default()),
                key_file,
                dialogs,
                #[cfg(target_os = "macos")]
                apple_events.connect(&cc.egui_ctx),
            )))
        }),
    )
}

// Not on macOS: APFS refuses file names that aren't valid UTF-8, so no such file can exist there.
#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn a_file_whose_name_isnt_unicode_opens_and_saves_back_to_itself() {
        let dir = std::env::temp_dir().join(format!("wc-app-non-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(std::ffi::OsString::from_vec(b"caf\xe9.txt".to_vec()));
        std::fs::write(&path, "Bonjour").unwrap();

        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        open_non_unicode(&mut app, &path);
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Bonjour");
        assert_eq!(app.session.path.as_deref(), Some(path.as_path()));

        app.run("caret.docEnd", serde_json::json!({})).unwrap();
        app.run("text.insert", serde_json::json!({"text": " encore"})).unwrap();
        app.run("file.save", serde_json::json!({})).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("Bonjour encore"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "a second file was written");

        // A file that can't be read is a warning, as one that fails to open by name is.
        open_non_unicode(&mut app, &dir.join(std::ffi::OsString::from_vec(b"gon\xe9.txt".to_vec())));
        assert_eq!(app.session.path.as_deref(), Some(path.as_path()));
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod prefs_tests {
    use super::write_prefs;

    #[test]
    fn failed_preferences_write_keeps_the_old_file_and_reports_the_error() {
        let dir = std::env::temp_dir().join(format!("wordcraft-prefs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ui.json");
        std::fs::write(&path, b"old").unwrap();
        write_prefs(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        // A directory squatting on the temporary name makes the preparation step fail.
        std::fs::create_dir(dir.join(format!(".ui.json.{}.tmp", std::process::id()))).unwrap();
        assert!(write_prefs(&path, b"newer").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
