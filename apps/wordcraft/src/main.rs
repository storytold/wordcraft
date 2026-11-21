//! WordCraft desktop app.
//!
//! Usage: `wordcraft [--control <port>] [--control-token <hex>] [--control-token-file <path>]
//! [--sample] [files…]`; `--help` prints `USAGE`.
//!
//! `--control <port>` (or `WORDCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server.
//! A client authenticates once, with its first line, using the control token: the one given by
//! `--control-token` / `--control-token-file` (or `WORDCRAFT_CONTROL_TOKEN` /
//! `WORDCRAFT_CONTROL_TOKEN_FILE`), else one the app generates and writes to the token file
//! (`docs/control-protocol.md`, Token).
//! Requests after that carry no key. See `docs/control-protocol.md`.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "linux")]
mod appimage_env;
mod control_server;
mod default_apps;
mod prefs;
mod region;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use craft::automation::args::{ControlOptions, EnvNames, FlagNames, Source, control_port};
use craft::automation::control::ControlChannel;
use craft::dialogs::service::{FilePanels, FileService};
use craft::host::target::{Backend, Os, Runtime, ShellInfo, WindowKind};
use craft::host::{Host, Wake};
use craft::platform::appearance::SystemReader;
use craft::platform::clipboard::{Clipboard, ClipboardImage, SystemClipboard};
use craft::platform::default_app::{DefaultApp, DefaultAppError};
use craft::platform::logs::{self, LogNames, crash};
use craft::platform::open::{DropTarget, DroppedFile, FileIntake, IntakeSender};
use craft::print::spool::Spool;
use craft::settings::SharedSettings;
use craft::ui::InstallSpec;
use craft::ui::runtime::windows::{WindowAction, WindowHost};
use craft::ui::shell::embed;
use craft::window::policy::{Decision, Desktop, DesktopKind, TitleBarPref, apply, decide};
use craft::window::restore::{self, Store};
use craft::window::system::SystemWindows;
use wordcraft_engine::Session;
// PLAN: assumed `wordcraft_ui_egui::prefs` is `pub mod prefs` (sh.A12, shell-w13); app-hub's list names the module without its visibility.
use wordcraft_ui_egui::{Services, WordApp, prefs::Prefs};

/// The app, the file dialogs it asked for, what the OS asked of it, the settings it flushes at
/// exit, the print spool it waits for at exit and the control server it stops at exit.
struct App {
    app: WordApp,
    /// The native panels behind the file service the app reaches through `Host::files`.
    panels: FilePanels,
    /// Feeds the files dropped on the window into the app's intake (`Services::intake`), the one
    /// queue the OS's requests are read from.
    drops: IntakeSender,
    /// The settings store the app saves through; flushed once the app is done.
    settings: SharedSettings<Prefs>,
    /// The print spool, a handle on the one in the app's services: jobs being sent finish first.
    spool: Spool,
    /// The control server, when `--control` started one; it removes the token file it made.
    control: Option<control_server::Control>,
}

impl App {
    /// One frame's logic: what the OS asked, the app, its file dialogs, then the root window's close
    /// path. Without `frame` (a test over no window) the dialogs have no parent window.
    fn logic_pass(&mut self, ctx: &egui::Context, frame: Option<&eframe::Frame>) {
        self.app.logic(ctx);
        self.panels.pump_with(self.app.dialog_host_mut(), frame);
        // A quit that waits for nothing (the documents are answered for) closes the root window.
        if self.app.quit_requested && !self.app.printing.busy() {
            WindowHost::of(ctx).act(egui::ViewportId::ROOT, WindowAction::Close);
        }
        // The decision holds the close while a Save Changes prompt is open; a closure over
        // `close_requested` would ask again on every frame of the prompt.
        embed::logic(ctx, || self.app.close_decision());
    }

    /// One pass of the root window's UI.
    fn ui_pass(&mut self, ui: &mut egui::Ui, frame: Option<&eframe::Frame>) {
        if let Some(frame) = frame {
            embed::before_ui(ui.ctx(), frame);
        }
        self.app.ui(ui);
        self.panels.pump_with(self.app.dialog_host_mut(), frame);
    }

    /// Everything that must happen before the process ends, whichever way the window closes.
    fn finish(&mut self) {
        // Jobs being sent finish first, so the spooler has them before the process exits.
        self.spool.shutdown();
        self.app.session.finish_recovery();
        if let Err(e) = self.settings.lock().flush() {
            log::warn!("preferences: {e}");
        }
        if let Some(control) = self.control.take() {
            control.shut_down();
        }
        self.app.start_pending_update();
    }
}

impl eframe::App for App {
    /// Hands the app what the runtime gathered since the last frame (fired commands) and the
    /// runtime the input the app gives back, before any pass of the frame runs.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Once per frame, before any pass; the files stay in `raw` for the runtime's own reading.
        let files = dropped_files(raw);
        if !files.is_empty() {
            self.drops.dropped(files, DropTarget::Elsewhere);
        }
        embed::before_input(ctx, |out| self.app.frame_input(out));
    }
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.logic_pass(ctx, Some(frame));
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ui_pass(ui, Some(frame));
    }
    fn on_exit(&mut self) {
        self.finish();
    }
}

/// The files dropped in this frame's input, as the intake carries them: a file with a path by its
/// path, a file without one by its bytes.
fn dropped_files(raw: &egui::RawInput) -> Vec<DroppedFile> {
    raw.dropped_files
        .iter()
        .map(|file| {
            let path = Some(file.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty());
            let name = path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // A file without a path (the web) carries its bytes; one that cannot be read is dropped as empty.
            let bytes = if path.is_none() { file.bytes().ok().map(std::sync::Arc::from) } else { None };
            DroppedFile { path, name, bytes }
        })
        .collect()
}

/// Whether the window runs through XWayland (X11) rather than native Wayland, and the log line
/// saying why. `WORDCRAFT_WAYLAND` wins (`0`, `false`, `no` or `off` asks for X11, any other value
/// for Wayland), then File › Options › Advanced › Linux display server (#2184). Automatic is X11
/// whenever there's an X server: winit has no file drag-and-drop on Wayland, so dropping files from
/// the file manager showed a "no" cursor. X11 is only forced with an X server (`DISPLAY`), and
/// unforced winit picks Wayland when it's there and X11 otherwise, so a choice that isn't
/// available falls back to the other instead of failing to start. The X11 libraries are only
/// recommended by the packages, so when `x11_libraries` reports some missing (it's asked only when
/// X11 would be forced) the app stays on Wayland: forced X11 without them failed to start, and
/// winit panicked without libxkbcommon-x11.
#[cfg(all(unix, not(target_os = "macos")))]
fn uses_x11(
    setting: wordcraft_ui_egui::DisplayServer,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
    x11_libraries: impl FnOnce() -> Result<(), String>,
) -> (bool, String) {
    use wordcraft_ui_egui::DisplayServer;
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty());
    let (wanted, by) = match env("WORDCRAFT_WAYLAND") {
        Some(v) if matches!(v.to_string_lossy().trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off") => {
            (DisplayServer::X11, "WORDCRAFT_WAYLAND")
        }
        Some(_) => (DisplayServer::Wayland, "WORDCRAFT_WAYLAND"),
        None => (setting, "the setting"),
    };
    let missing = if wanted != DisplayServer::Wayland && set("DISPLAY") { x11_libraries().err() } else { None };
    let x11 = wanted != DisplayServer::Wayland && set("DISPLAY") && missing.is_none();
    let wayland = set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET");
    let runs = if x11 {
        "X11"
    } else if wayland {
        "native Wayland"
    } else if set("DISPLAY") {
        "X11 (no Wayland)"
    } else {
        "neither (no Wayland or X server found)"
    };
    let mut why = format!("display: {runs}; {} asks for {}", by, wanted.code());
    if let Some(missing) = missing {
        why += &format!("; not X11 because its libraries are missing: {missing}");
    }
    (x11, why)
}

/// `Err` naming the libraries winit's X11 backend opens at startup that can't be loaded: libX11,
/// libXcursor, libX11-xcb (which brings in libxcb), libXi, and libxkbcommon-x11 for the keyboard.
#[cfg(all(unix, not(target_os = "macos")))]
fn x11_libraries() -> Result<(), String> {
    use x11_dl::{xcursor::Xcursor, xinput2::XInput2, xlib::Xlib, xlib_xcb::Xlib_xcb};
    let mut missing: Vec<String> = [Xlib::open().err(), Xcursor::open().err(), Xlib_xcb::open().err(), XInput2::open().err()]
        .into_iter()
        .flatten()
        .map(|e| e.to_string())
        .collect();
    if xkbcommon_dl::x11::xkbcommon_x11_option().is_none() {
        missing.push("libxkbcommon-x11.so.0 could not be loaded".into());
    }
    if missing.is_empty() { Ok(()) } else { Err(missing.join("; ")) }
}

/// The type of the "make WordCraft the default app" button's action.
type MakeDefault = Box<dyn Fn() -> Result<DefaultApp, DefaultAppError>>;

/// What the app takes from its host beyond the runtime: File › Print through the system spooler,
/// personal templates (#2852, made on first save in `Templates` in the settings folder), and making
/// WordCraft the default app for Word documents (#295; macOS has no way to, so no button there).
fn services(spool: Spool, intake: FileIntake) -> Services {
    #[cfg(any(unix, windows))]
    let make_default_app =
        (!cfg!(target_os = "macos")).then(|| -> MakeDefault { Box::new(|| craft::platform::default_app::make_default(&default_apps::identity())) });
    #[cfg(not(any(unix, windows)))]
    let make_default_app: Option<MakeDefault> = None;
    Services {
        print: Some(spool),
        templates_dir: wordcraft_control_key::settings_dir().map(|d| d.join("Templates")),
        make_default_app,
        intake: Some(intake),
        ..Default::default()
    }
}

/// A PNG as the pixels the system clipboard takes, `None` for anything that doesn't decode within
/// the picture decoder's limits.
fn decode_png(bytes: &[u8]) -> Option<ClipboardImage> {
    let rgba = wordcraft_geom::picture::decode_memory(bytes).ok()?.to_rgba8();
    Some(ClipboardImage { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
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
  wordcraft [--sample] [--control PORT] [--control-token TOKEN | --control-token-file PATH] [FILE…]

OPTIONS:
  --sample                     open the sample document
  --control PORT               start the localhost control server on PORT, for agents and scripts
                               (or set WORDCRAFT_CONTROL_PORT); see docs/control-protocol.md
  --control-token TOKEN        the 64 hex digits clients authenticate with
                               (or set WORDCRAFT_CONTROL_TOKEN); without one a token is generated
  --control-token-file PATH    read the token from a file instead
                               (or set WORDCRAFT_CONTROL_TOKEN_FILE)
  --version                    print the version
  -h, --help                   print this help
";

/// The commit the build came from (short), or `dev`.
fn build_sha() -> &'static str {
    option_env!("WORDCRAFT_BUILD_SHA").map(|s| s.get(..8).unwrap_or(s)).unwrap_or("dev")
}

/// The arguments as text, and apart the files whose names aren't valid Unicode, such as a Latin-1
/// one (#1440). A `--control` port, a token or a token file that isn't is an error.
fn text_args() -> (Vec<String>, Vec<std::path::PathBuf>) {
    let (mut args, mut files) = (Vec::new(), Vec::new());
    for (i, a) in std::env::args_os().skip(1).enumerate() {
        match a.into_string() {
            Ok(a) => args.push(a),
            Err(a) if args.last().is_some_and(|p| matches!(p.as_str(), "--control" | "--control-token" | "--control-token-file")) => {
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

/// The arguments without the ones macOS adds when it launches an app (#1103): `-psn_0_12345`, and
/// the `-NS…` / `-Apple…` options (`-AppleLanguages (en)`, `-NSDocumentRevisionsDebugMode YES`) with
/// the value after each. They are not files, and opening them as files fails.
fn without_launch_args(args: Vec<String>) -> Vec<String> {
    let mut kept = Vec::with_capacity(args.len());
    let mut args = args.into_iter().peekable();
    while let Some(a) = args.next() {
        if a.starts_with("-psn_") {
            continue;
        }
        if a.starts_with("-NS") || a.starts_with("-Apple") {
            // Its value, unless the next argument is another flag.
            args.next_if(|v| !v.starts_with('-'));
            continue;
        }
        kept.push(a);
    }
    kept
}

/// Open a file whose name isn't valid Unicode (#1440). `file.open` takes its path as text, so it
/// gets the file's bytes under the lossy name, and the document then keeps the real path to save to.
/// The bytes are read as `file.open` reads a path: only a regular file, and at most 2 GB. A Word
/// template starts a new document instead ([`WordApp::open_from_platform`]), which has no path.
fn open_non_unicode(app: &mut WordApp, path: &std::path::Path) {
    let name = path.to_string_lossy();
    let opened = wordcraft_engine::io::read_document(path)
        .and_then(|bytes| app.open_from_platform(&name, Some(&wordcraft_engine::cmd::insert::base64_encode(&bytes))));
    match opened {
        // Not when it waits for a password: that open names the document itself.
        Ok(r) if r.get("pending").is_none() && !wordcraft_ui_egui::documents::is_word_template(&name) => app.session.path = Some(path.into()),
        Ok(_) => {}
        Err(e) => {
            app.report_open_failure(&name, &e);
        }
    }
}

/// Writes `message` and a newline to standard error. Unlike `eprintln!`, never panics.
fn complain(message: std::fmt::Arguments<'_>) {
    let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("wordcraft: {message}\n"));
}

/// Parses the command line, installs logging and the panic hook, restores preferences and window
/// geometry, opens requested documents or the Start screen, starts the control server when
/// asked, and runs the eframe event loop.
fn main() -> eframe::Result {
    // Windows: the print helper is a copy of this program that runs one job in its own process.
    #[cfg(windows)]
    if let Some(code) = craft::print::spool::isolated::helper_main() {
        std::process::exit(code);
    }
    // Windows: the update helper is a copy of this program started with `--finish-update`.
    #[cfg(windows)]
    if let Some(result) = craft::update::self_update::finish(&wordcraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        std::process::exit(i32::from(result.is_err()));
    }
    // Linux: before anything reads another program's AppImage variables, or a thread starts (#5653).
    #[cfg(target_os = "linux")]
    let start_over_failed = appimage_env::drop_inherited();
    // Still carrying them, this process must never update itself: the update would replace the
    // other program's file.
    #[cfg(target_os = "linux")]
    let updates_unsafe = start_over_failed.is_some();
    #[cfg(not(target_os = "linux"))]
    let updates_unsafe = false;
    // First, so the panic hook (set once, by this call; the later one only gives it the crash-log
    // folder) and every start-up warning are recorded.
    let logger = logs::install(LogNames::new("wordcraft"));
    crash::install("WordCraft", env!("CARGO_PKG_VERSION"), None);
    #[cfg(target_os = "linux")]
    if let Some(e) = start_over_failed {
        log::error!("couldn't start again without another program's AppImage variables, so the self-update is off in this window: {e}");
    }
    let mut flags = ControlOptions::default();
    let mut files = Vec::new();
    let mut sample = false;
    let (args, other_files) = text_args();
    // Only macOS (Finder / LaunchServices) adds these; elsewhere `-NS…` could be a real file name.
    let args = if cfg!(target_os = "macos") { without_launch_args(args) } else { args };
    let mut bad_control: Option<Option<String>> = None;
    let mut missing_value: Option<&'static str> = None;
    let mut args = args.into_iter().peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            // A flag after `--control` stays a flag; a bad or missing port is an error (#1426).
            // Port 0 is refused after the variables are read: the port names the control server's files.
            "--control" => {
                let port = args.next_if(|p| p.strip_prefix('-').is_none_or(|n| n.starts_with(|c: char| c.is_ascii_digit())));
                match port.as_deref().and_then(|p| control_port(p).ok()) {
                    Some(p) => {
                        flags.port = Some(p);
                        flags.sources.port = Some(Source::Flag);
                    }
                    None => bad_control = Some(port),
                }
            }
            "--control-token" => match args.next() {
                Some(token) => {
                    flags.token = Some(token);
                    flags.sources.token = Some(Source::Flag);
                }
                None => missing_value = Some("--control-token"),
            },
            "--control-token-file" => match args.next() {
                Some(path) => {
                    flags.token_file = Some(PathBuf::from(path));
                    flags.sources.token_file = Some(Source::Flag);
                }
                None => missing_value = Some("--control-token-file"),
            },
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
    // After the loop, so `--version` later on the line still wins.
    if let Some(port) = bad_control {
        let got = port.map(|p| format!(", got {p:?}")).unwrap_or_default();
        complain(format_args!("--control needs a port number from 1 to 65535{got}"));
        std::process::exit(2);
    }
    if let Some(flag) = missing_value {
        complain(format_args!("{flag} needs a value"));
        std::process::exit(2);
    }
    // The flags over the variables; a bad variable names itself, never the token.
    let names = EnvNames::for_app("WORDCRAFT");
    let control = match ControlOptions::resolve(flags, &names).and_then(|o| o.check(&FlagNames::default(), &names).map(|()| o)) {
        Ok(options) => options,
        Err(e) => {
            complain(format_args!("error: {e}"));
            std::process::exit(2);
        }
    };
    if control.port == Some(0) {
        complain(format_args!("--control needs a port number from 1 to 65535, got \"0\""));
        std::process::exit(2);
    }
    let settings_dir = wordcraft_control_key::settings_dir();
    // The log file lives in the settings directory, next to the preferences; opened after the
    // arguments, so `--version` leaves no file behind. Records logged until now are written to it
    // first. Runs without preferences (agents' test runs) log to standard error only, so they
    // don't rotate away the user's own logs.
    if let Some(logger) = logger {
        match settings_dir.as_ref().filter(|_| prefs::enabled()).map(|d| d.join("logs")) {
            Some(dir) => match logger.attach_dir(&dir) {
                Ok(path) => {
                    crash::install("WordCraft", env!("CARGO_PKG_VERSION"), Some(dir));
                    log::info!("WordCraft {} ({}), log file {}", env!("CARGO_PKG_VERSION"), build_sha(), path.display());
                }
                // Standard error only by now (`attach_dir` gave up on the file).
                Err(e) => log::warn!("no log file: {e}"),
            },
            None => logger.stderr_only(),
        }
    }
    let (viewport, frame) = root_viewport(Os::current());
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    if let Some(icon) = app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    // Before eframe creates the wgpu instance: the backends, power preference, shader compiler and
    // the GPU that drives the screen (#603, #2652, #5199).
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        craft::gpu::configure(setup);
    }
    prefs::load_user_fonts();
    // Where the root window was last, kept beside the settings; in memory when preferences are off.
    // The settings open with it: a first run imports the old `ui.json`'s window place into it.
    let mut placement = match settings_dir.as_ref().filter(|_| prefs::enabled()) {
        Some(dir) => Store::at(&dir.join(restore::FILE_NAME)),
        None => Store::in_memory(),
    };
    let (settings, report) = prefs::open(prefs::enabled(), &mut placement);
    if report.has_problems() {
        log::warn!("preferences: {report:?}");
    }
    let placement = Arc::new(Mutex::new(placement));
    #[cfg(all(unix, not(target_os = "macos")))]
    let x11 = {
        let display_server = settings.lock().get().display_server;
        let (x11, why) = uses_x11(display_server, |name| std::env::var_os(name), x11_libraries);
        log::info!("{why}");
        if x11 {
            options.event_loop_builder = Some(Box::new(|b| {
                use winit::platform::x11::EventLoopBuilderExtX11;
                b.with_x11();
            }));
        }
        x11
    };
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let x11 = false;
    // What the OS asks of a running app arrives here; its wake repaints once the window exists.
    let repaint: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let (intake, sender) = FileIntake::new(Wake::new({
        let repaint = repaint.clone();
        move || {
            if let Some(ctx) = repaint.get() {
                ctx.request_repaint();
            }
        }
    }));
    // Registered before the event loop starts, so it catches the Finder event that launched us as
    // well as later ones. Lives until the event loop returns.
    let drops = intake.sender();
    #[cfg(target_os = "macos")]
    let _apple_events = craft::platform::open::apple_events::install(sender);
    #[cfg(not(target_os = "macos"))]
    drop(sender);
    let exit_settings = settings.clone();
    let result = eframe::run_native(
        "WordCraft",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let _ = repaint.set(ctx.clone());
            let wake = {
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            };
            // The interface language: the saved choice, else the system's.
            let lang = wordcraft_ui_egui::i18n::Lang::from_pref(&settings.lock().get().language);
            let (file_service, panels) = FileService::new(Wake::new(wake.clone()));
            let host = Arc::new(
                Host::builder(wordcraft_ui_egui::APP)
                    .i18n(wordcraft_ui_egui::i18n::catalogs())
                    .files(Arc::new(file_service))
                    .settings(Arc::new(settings.clone()))
                    .lang(lang.craft_lang())
                    .build(),
            );
            let shell = ShellInfo::current().backend(if x11 { Backend::X11 } else { Backend::from_env() });
            let mut spec = InstallSpec::new(shell, Arc::new(SystemWindows))
                .app_id("ai.storyteller.wordcraft")
                .system(SystemReader::start(wake))
                .clipboard(Clipboard::new(SystemClipboard::new().decoder(decode_png)))
                .placement(placement);
            spec.frame = frame;
            // PLAN: assumed `wordcraft_ui_egui::ui_fonts` re-exports sh.A10's `app::fonts::ui_fonts` (`mod app` is private; app-hub/shell-w11).
            embed::install(&ctx, spec, &wordcraft_ui_egui::ui_fonts(lang)).map_err(|e| format!("the runtime could not be installed: {e}"))?;
            // The system region picks the new document's paper, the units and the decimal
            // separator (#838).
            let region = region::system();
            let doc = if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_engine::blank_document(&region) };
            let mut session = Session::new(doc);
            session.system_region = region;
            // The custom dictionary's words (#40) count as loaded settings.
            if prefs::enabled()
                && let Some(path) = prefs::user_dict_path()
            {
                prefs::load_user_dictionary(&mut session, &settings, path);
            }
            let spool = Spool::new(host.jobs_shared().clone());
            let mut app = WordApp::new(session, services(spool.clone(), intake)).with_host(host).with_settings(settings.clone());
            // PLAN: writer note 9 puts `with_settings` after `with_control`; it comes first here because
            // the region check below reads the saved region the settings apply.
            if std::env::var("WORDCRAFT_FILE_DIALOG").is_ok_and(|v| v.trim().eq_ignore_ascii_case("builtin")) {
                app = app.with_fallback_chooser(true);
            }
            app.set_startup_update_check(true);
            // After the preferences, so the reason it gives is in the interface language.
            if updates_unsafe {
                app.disable_self_update();
            }
            // Language packs from the suite's font store; an Install click is the person's own.
            wordcraft_engine::packs::use_font_store(true);
            // A region chosen in File › Options replaces the system's for the first blank
            // document too (#838).
            if !sample && app.session.prefs.region.is_some() {
                let doc = app.session.blank_document();
                app.replace_startup_document(doc);
            }
            app.tell_about_font_downloads();
            if prefs::enabled()
                && let Some(dir) = wordcraft_control_key::settings_dir()
            {
                let ctx = ctx.clone();
                if let Err(e) = app.session.enable_recovery(dir.join("recovery"), Arc::new(move || ctx.request_repaint())) {
                    log::warn!("local recovery unavailable: {e}");
                }
            }
            let mut control_server = None;
            if let Some(port) = control.port
                && let Some((rx, started)) =
                    control_server::start(&control, port, WindowHost::of(&ctx), wordcraft_control_key::settings_dir().as_deref())
            {
                app = app.with_control(ControlChannel::new(rx));
                control_server = Some(started);
            }
            // Nothing to open (no files, no sample, no control channel driving the editor): start
            // on File › Home, as Word does (#766), unless the Start screen option is off. Files
            // the OS hands over later open on top of it and close it.
            if files.is_empty() && other_files.is_empty() && !sample && control.port.is_none() {
                app.start_screen();
            }
            // A Word template named here starts a new document based on it (#3433).
            for f in files {
                if let Err(e) = app.open_from_platform(&f, None) {
                    app.report_open_failure(&f, &e);
                }
            }
            for f in other_files {
                open_non_unicode(&mut app, &f);
            }
            Ok(Box::new(App { app, panels, drops, settings, spool, control: control_server }))
        }),
    );
    // The app is dropped by now: whatever it saved on the way out is in the store.
    if let Err(e) = exit_settings.lock().flush() {
        log::warn!("preferences: {e}");
    }
    result
}

/// The root window's viewport, framed for `os`, and the frame decision the install spec passes to the shell.
fn root_viewport(os: Os) -> (egui::ViewportBuilder, Option<Decision>) {
    let viewport = egui::ViewportBuilder::default()
        .with_title("WordCraft")
        .with_app_id("ai.storyteller.wordcraft")
        .with_inner_size([1440.0, 920.0])
        .with_min_inner_size(wordcraft_ui_egui::window_geometry::MIN_SIZE)
        .with_drag_and_drop(true);
    let frame = frame_decision(os);
    let viewport = match &frame {
        Some(decision) => apply(viewport, &decision.frame),
        None => viewport,
    };
    (viewport, frame)
}

/// The root window's frame, decided for the OS. macOS draws the app's own bar over a full-size content view; the shell
/// must be told (the install spec's frame), or it treats the window as OS-framed: the title bar has no move band to drag
/// the window by and no room for the traffic lights (#7149 item 1). Elsewhere the OS keeps its own frame.
fn frame_decision(os: Os) -> Option<Decision> {
    // lean: no CRAFT_TITLEBAR override or "system title bar" setting on macOS yet; pass them to decide when asked.
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    (os == Os::Mac).then(|| decide(os, Runtime::Native, Backend::from_env(), &desktop, WindowKind::Main, TitleBarPref::default(), None))
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use craft::window::policy::Frame;

    #[test]
    fn only_macos_gets_the_inset_frame() {
        assert!(matches!(frame_decision(Os::Mac).map(|d| d.frame), Some(Frame::Inset(_))));
        assert!(frame_decision(Os::Linux).is_none());
        assert!(frame_decision(Os::Windows).is_none());
    }

    /// The macOS window draws the app's bar over a full-size content view with the system bar hidden, and tells the
    /// shell so; Windows and Linux keep the OS frame. Every window takes dropped files.
    #[test]
    fn the_root_window_is_framed_for_its_os() {
        let (mac, frame) = root_viewport(Os::Mac);
        assert!(frame.is_some(), "the shell is told the macOS frame");
        assert_eq!((mac.fullsize_content_view, mac.titlebar_shown, mac.title_shown), (Some(true), Some(false), Some(false)));
        for os in [Os::Windows, Os::Linux] {
            let (viewport, frame) = root_viewport(os);
            assert!(frame.is_none(), "{os:?} keeps the OS frame");
            assert_eq!((viewport.fullsize_content_view, viewport.titlebar_shown), (None, None), "{os:?} has the system bar");
        }
        assert_eq!(mac.drag_and_drop, Some(true));
    }
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

    /// #2184: `WORDCRAFT_WAYLAND` beats the setting, Automatic keeps XWayland where there's an X
    /// server, and a choice that isn't available falls back to the other.
    #[test]
    fn the_display_server_follows_the_variable_then_the_setting() {
        use wordcraft_ui_egui::DisplayServer::{Auto, Wayland, X11};
        let x11 = |setting, vars: &[(&str, &str)]| {
            uses_x11(setting, |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| std::ffi::OsString::from(v)), || Ok(())).0
        };
        let both = [("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")];
        assert!(x11(Auto, &both), "Automatic keeps XWayland, for file drag-and-drop");
        assert!(x11(X11, &both));
        assert!(!x11(Wayland, &both));
        for (value, wants_x11) in [("1", false), ("", false), ("yes", false), ("0", true), (" Off ", true), ("FALSE", true), ("no", true)] {
            let vars = [both[0], both[1], ("WORDCRAFT_WAYLAND", value)];
            for setting in [Auto, Wayland, X11] {
                assert_eq!(x11(setting, &vars), wants_x11, "WORDCRAFT_WAYLAND={value:?} beats {setting:?}");
            }
        }
        // Without an X server X11 isn't forced: winit connects to Wayland instead of failing.
        let wayland_only = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", "")];
        assert!(!x11(X11, &wayland_only) && !x11(Auto, &wayland_only));
        // Without Wayland, Wayland isn't forced either: unforced winit picks X11.
        let (forced, why) = uses_x11(Wayland, |name| (name == "DISPLAY").then(|| ":0".into()), || Ok(()));
        assert!(!forced && why.contains("X11 (no Wayland)"), "{why}");
    }

    /// Without the X11 libraries (the packages only recommend them) X11 isn't forced, whatever asks
    /// for it: forced X11 failed to start, and winit panicked without libxkbcommon-x11. The check
    /// runs only when X11 would be forced.
    #[test]
    fn missing_x11_libraries_leave_the_app_on_wayland() {
        use wordcraft_ui_egui::DisplayServer::{Auto, Wayland, X11};
        let both = [("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")];
        for (setting, variable) in [(Auto, None), (X11, None), (Wayland, Some("0"))] {
            let env = |name: &str| {
                both.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).into())
                    .or_else(|| (name == "WORDCRAFT_WAYLAND").then_some(variable).flatten().map(Into::into))
            };
            let (x11, why) = uses_x11(setting, env, || Err("libxkbcommon-x11.so.0 could not be loaded".into()));
            assert!(!x11, "{setting:?} with WORDCRAFT_WAYLAND={variable:?}: {why}");
            assert!(why.contains("native Wayland") && why.contains("libxkbcommon-x11"), "{why}");
        }
        let checked = std::cell::Cell::new(false);
        let (x11, _) = uses_x11(
            Wayland,
            |name| both.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).into()),
            || {
                checked.set(true);
                Ok(())
            },
        );
        assert!(!x11 && !checked.get(), "asked for Wayland, the libraries aren't checked");
    }

    /// A named pipe or a device under such a name is a warning, not a read: the pipe blocked
    /// start-up until something wrote to it, and `/dev/zero` read until memory ran out.
    #[test]
    fn a_pipe_or_device_whose_name_isnt_unicode_is_not_read() {
        let dir = std::env::temp_dir().join(format!("wc-app-non-utf8-dev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join(std::ffi::OsString::from_vec(b"fif\xe9.txt".to_vec()));
        if !std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_dir_all(&dir);
            eprintln!("mkfifo unavailable; skipping");
            return;
        }
        let zero = dir.join(std::ffi::OsString::from_vec(b"z\xe9ro.txt".to_vec()));
        std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
        // If the open blocks on the pipe, a writer comes after 10 s, so the test fails rather than hangs.
        let unblock = fifo.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let _ = std::fs::OpenOptions::new().write(true).open(unblock);
        });

        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        app.run("text.insert", serde_json::json!({"text": "kept"})).unwrap();
        for p in [&fifo, &zero] {
            let started = std::time::Instant::now();
            open_non_unicode(&mut app, p);
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "{p:?} took {:?}", started.elapsed());
            assert_eq!(app.session.path, None, "{p:?}");
            assert!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("kept"), "{p:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod launch_arg_tests {
    use super::without_launch_args;

    fn strip(args: &[&str]) -> Vec<String> {
        without_launch_args(args.iter().map(|a| a.to_string()).collect())
    }

    #[test]
    fn macos_launch_arguments_are_not_files() {
        assert_eq!(strip(&["-psn_0_12345", "a.docx"]), ["a.docx"]);
        assert_eq!(strip(&["-AppleLanguages", "(en)", "a.docx"]), ["a.docx"]);
        assert_eq!(strip(&["a.docx", "-NSDocumentRevisionsDebugMode", "YES", "b.docx"]), ["a.docx", "b.docx"]);
        // A flag with no value after it, or followed by another flag, takes nothing with it.
        assert_eq!(strip(&["a.docx", "-AppleLocale"]), ["a.docx"]);
        assert_eq!(strip(&["-NSFoo", "--sample", "a.docx"]), ["--sample", "a.docx"]);
    }

    #[test]
    fn real_arguments_are_kept() {
        assert_eq!(strip(&["--control", "7981", "--sample", "-h"]), ["--control", "7981", "--sample", "-h"]);
        // Files whose names merely start like a launch argument keep their place when they are paths.
        assert_eq!(strip(&["./-psn_x.docx", "Apple.docx", "-", "-nsfoo"]), ["./-psn_x.docx", "Apple.docx", "-", "-nsfoo"]);
        assert!(strip(&[]).is_empty());
    }
}

/// The app over the framework's frame driver, installed as `main` installs it, with in-memory
/// settings, at a desktop size.
#[cfg(test)]
mod smoke {
    use super::*;
    use craft::ui::testing::Frames;
    use serde_json::json;

    /// The app, its frame driver and the sender of its intake.
    struct Run {
        app: App,
        frames: Frames,
        ctx: egui::Context,
        sender: IntakeSender,
    }

    impl Run {
        /// The app as `main` builds it, minus the window: the runtime installed before the first
        /// frame, a desktop host with a file service and a print spool, one blank document.
        fn new() -> Run {
            let ctx = egui::Context::default();
            // The test finds the prompt's button in the AccessKit tree, which egui builds only once AccessKit is on
            // (a screen reader turns it on in the app; `Frames::over` keeps the context's own options).
            ctx.enable_accesskit();
            let (files, panels) = FileService::new(Wake::new(|| {}));
            let (settings, _) = prefs::open(false, &mut Store::in_memory());
            let lang = wordcraft_ui_egui::i18n::Lang::from_pref(&settings.lock().get().language);
            let host = Arc::new(
                Host::builder(wordcraft_ui_egui::APP)
                    .i18n(wordcraft_ui_egui::i18n::catalogs())
                    .files(Arc::new(files))
                    .settings(Arc::new(settings.clone()))
                    .lang(lang.craft_lang())
                    .build(),
            );
            let spec = InstallSpec::new(ShellInfo::current(), Arc::new(SystemWindows))
                .app_id("ai.storyteller.wordcraft")
                .placement(Arc::new(Mutex::new(Store::in_memory())));
            embed::install(&ctx, spec, &wordcraft_ui_egui::ui_fonts(lang)).map_err(|e| e.to_string()).unwrap();
            let mut frames = Frames::over(ctx.clone());
            frames.screen(egui::vec2(1440.0, 920.0));
            let spool = Spool::new(host.jobs_shared().clone());
            let (intake, sender) = FileIntake::new(Wake::new(|| {}));
            let drops = intake.sender();
            let session = Session::new(wordcraft_doc::Document::new());
            let app = WordApp::new(session, services(spool.clone(), intake)).with_host(host).with_settings(settings.clone());
            Run { app: App { app, panels, drops, settings, spool, control: None }, frames, ctx, sender }
        }

        /// One frame as eframe runs it: the input hook once, the logic once, the UI per pass.
        /// Returns whether the frame sent a close command.
        fn step(&mut self, events: Vec<egui::Event>) -> bool {
            let Run { app, frames, ctx, .. } = self;
            embed::before_input(ctx, |out| app.app.frame_input(out));
            let mut first = true;
            frames.frame_without_input(events, |ui| {
                if std::mem::take(&mut first) {
                    app.logic_pass(ctx, None);
                }
                app.ui_pass(ui, None);
            })
        }

        /// Frames with no input.
        fn settle(&mut self) {
            for _ in 0..3 {
                self.step(vec![]);
            }
        }

        /// One frame in which the OS asks the root window to close (its close button: a `Close` event in the
        /// root's window facts, which the app's close decision answers; `WindowAction::Close` is the app's own
        /// close command and asks nobody). Returns whether the app let the close through: it holds it by
        /// cancelling the close.
        fn close_request_is_let_through(&mut self) -> bool {
            let Run { app, ctx, .. } = self;
            embed::before_input(ctx, |out| app.app.frame_input(out));
            let mut input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 920.0))), ..Default::default() };
            input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() });
            let mut first = true;
            let mut output = ctx.run_ui(input, |ui| {
                if std::mem::take(&mut first) {
                    app.logic_pass(ui.ctx(), None);
                }
                app.ui_pass(ui, None);
            });
            // Nothing paints this frame, so its texture uploads are dropped on purpose (egui panics on an unhandled drop).
            output.textures_delta.clear();
            let held = output
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|viewport| viewport.commands.contains(&egui::ViewportCommand::CancelClose));
            !held
        }
    }

    /// A file dropped on the window opens through the intake; a close of the root window with a
    /// modified document is held and the Save Changes prompt shows; Don't Save, pressed by its
    /// name, lets the next close request through. Guards the install order, the input hook and
    /// the close closure, which a debug assert on the first frame would otherwise hide.
    #[test]
    fn smoke() {
        let mut run = Run::new();
        run.settle();

        let dir = std::env::temp_dir().join(format!("wc-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dropped.txt");
        std::fs::write(&path, "Dropped words").unwrap();
        run.sender.dropped(vec![DroppedFile { path: Some(path), name: "dropped.txt".into(), bytes: None }], DropTarget::Elsewhere);
        run.settle();
        assert!(run.app.app.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Dropped words"), "the dropped file opened");
        let _ = std::fs::remove_dir_all(dir);

        run.app.app.run("text.insert", json!({"text": "unsaved"})).unwrap();
        run.settle();
        assert!(!run.close_request_is_let_through(), "a modified document holds the close");
        run.settle();
        assert!(run.app.app.dialog_host_mut().shows_modal(), "the Save Changes prompt shows");

        let tree = run.frames.tree();
        // The framework words the answer as the platform does: "Don't Save" on macOS, "No" elsewhere.
        let dont_save = if cfg!(target_os = "macos") { "Don't Save" } else { "No" };
        // A button is named by the label inside it (`labelled_by`) as often as by its own name, and the label's own
        // node is no button: the testing module's `name` and `is_interactive` read it as the fixture's `by_label` does.
        let button = tree
            .keys()
            .into_iter()
            .find(|key| craft::ui::testing::access::name(&tree, *key) == Some(dont_save) && craft::ui::testing::access::is_interactive(&tree, *key))
            .expect("the prompt has a button for Don't Save");
        let press = egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: egui::accesskit::NodeId::from(button),
            data: None,
        });
        run.step(vec![press]);
        run.settle();
        assert!(!run.app.app.dialog_host_mut().shows_modal(), "the prompt is answered");
        let closed = (0..3).any(|_| run.close_request_is_let_through());
        if cfg!(target_os = "macos") {
            // On macOS the root close goes to the platform quit (`craft_quit`), which keeps the window open for the quit
            // to close, so the request always comes back held: what shows is that no prompt asks again.
            run.settle();
            assert!(!run.app.app.dialog_host_mut().shows_modal(), "Don't Save lets the close through to the quit");
        } else {
            assert!(closed, "Don't Save lets the close through");
        }
    }
}
