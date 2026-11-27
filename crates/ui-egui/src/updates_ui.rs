//! Software updates, background check at startup, and the update dialogs on the dialog host.

use crate::WordApp;

#[cfg(not(target_arch = "wasm32"))]
use craft::ui::dialog::DialogResult;
#[cfg(not(target_arch = "wasm32"))]
use craft::update::stack_dialogs::{CheckChoice, UpdateCheckDialog};
#[cfg(not(target_arch = "wasm32"))]
use craft::update::updater::{Action, Status, Updater};

/// What the update flow keeps between passes: the updater and whether the check at start is on.
pub(crate) struct Runtime {
    #[cfg(not(target_arch = "wasm32"))]
    updater: Updater,
    /// The desktop app turns the check at start on; tests and the web build leave it off.
    #[cfg(not(target_arch = "wasm32"))]
    startup_check: bool,
    /// How many times the test stub answered a check (see `stub_update_check`).
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) stub_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            updater: Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).with_translator(translate),
            #[cfg(not(target_arch = "wasm32"))]
            startup_check: false,
            #[cfg(all(test, not(target_arch = "wasm32")))]
            stub_calls: Default::default(),
        }
    }
}

/// The update prompt's words in the interface language: craft-update passes its English text and
/// the values of its `{name}` placeholders each time it draws.
#[cfg(not(target_arch = "wasm32"))]
fn translate(text: &str, args: &[(&str, &str)]) -> String {
    crate::i18n::fmt(crate::i18n::t(text), args)
}

impl WordApp {
    /// Whether the app asks for a newer release when it starts (off until the desktop app turns it on).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_startup_update_check(&mut self, on: bool) {
        self.updates_rt.startup_check = on;
    }

    /// Turns the self-update off for the rest of this process: no update is offered, and Help ▸
    /// Check for Updates says why. For a process that still carries another program's AppImage
    /// variables, which the update would take for this copy's file and replace (#5653).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn disable_self_update(&mut self) {
        let rt = &mut self.updates_rt;
        rt.startup_check = false;
        rt.updater = Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).with_translator(translate).with_source(refused_check());
    }

    /// The Nightly channel's rule: ArtCraft Launcher installs and replaces the nightly build, so
    /// this window never checks for, offers or starts one. Nightly-only text is English: it is not
    /// one of the stable catalogs' strings.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn use_launcher_updates(&mut self) {
        let rt = &mut self.updates_rt;
        rt.startup_check = false;
        rt.updater = Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).with_translator(translate).with_source(launcher_check());
    }

    /// Answers every update check with "up to date" instead of asking GitHub, so a test's result
    /// doesn't depend on the network or its rate limit (#7463).
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn stub_update_check(&mut self) {
        let rt = &mut self.updates_rt;
        rt.startup_check = false;
        let calls = rt.stub_calls.clone();
        rt.updater =
            Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).with_translator(translate).with_source(std::sync::Arc::new(move || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(None)
            }));
    }

    /// Starts the helper of an update that is ready, so that it installs once the process has
    /// ended. Call it once the exit is certain, before a path that ends the process without
    /// dropping the app. Later calls do nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_pending_update(&mut self) {
        self.updates_rt.updater.start_pending_update();
    }
}

/// A check for updates that never finds one and gives the reason as its error, which Help ▸ Check
/// for Updates shows.
#[cfg(not(target_arch = "wasm32"))]
fn refused_check() -> craft::update::updater::CheckSource {
    let why = tl!("Updates are off in this window: WordCraft was started with another program's AppImage settings. Start WordCraft from its own menu entry to update it.")
        .to_string();
    std::sync::Arc::new(move || Err(why.clone().into()))
}

/// A check whose only answer is that the Launcher owns this build's updates.
#[cfg(not(target_arch = "wasm32"))]
fn launcher_check() -> craft::update::updater::CheckSource {
    let why = "Updates for WordCraft Nightly are managed by ArtCraft Launcher. Open ArtCraft Launcher to update it.".to_string();
    std::sync::Arc::new(move || Err(why.clone().into()))
}

/// Help ▸ Check for Updates. The result dialog opens from [`run`] while the updater has a status.
pub(crate) fn check(app: &mut WordApp) {
    #[cfg(not(target_arch = "wasm32"))]
    app.updates_rt.updater.check_now();
    #[cfg(target_arch = "wasm32")]
    {
        let alert =
            craft::ui::dialog::Alert::new(tl!("Check for Updates").to_string(), tl!("Update check is not supported in the web browser.").to_string());
        app.dialog_host.open(craft::ui::dialog::HostedAlert::new(alert));
    }
}

/// Once per frame, before the UI: the check at start, and the updater's own work.
pub(crate) fn tick(app: &mut WordApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let rt = &mut app.updates_rt;
        if rt.startup_check {
            rt.updater.check_at_start();
        }
        rt.updater.tick(ctx);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

/// Once per frame, before the dialog host runs: the updater's prompt and the check-result dialog.
pub(crate) fn run(app: &mut WordApp) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // An update starts, and a ready one asks the app to close, only while no work is unsaved.
        let can_start = !(app.session.dirty || app.save_unconfirmed() || app.printing.busy() || crate::panes::has_comment_drafts(app));
        let action = app.updates_rt.updater.run(&mut app.dialog_host, can_start);
        match action {
            Action::OpenUrl(url) => {
                if let Some(ctx) = &app.ctx {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
            }
            // The whole app quits, every window included, and first asks about each document with
            // unsaved changes.
            Action::Close => {
                let _ = app.ask_before_quit();
            }
            _ => {}
        }
        sync_check_dialog(app, can_start);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = app;
}

/// Shows the check-result dialog while the updater has a status (a manual check, or what the last
/// update came to), acts on its answer, and withdraws it when an update was found: the prompt
/// replaces it.
#[cfg(not(target_arch = "wasm32"))]
fn sync_check_dialog(app: &mut WordApp, can_start: bool) {
    let ctx = app.ctx.clone();
    let rt = &mut app.updates_rt;
    let host = &mut app.dialog_host;
    if let Some(answer) = host.take::<UpdateCheckDialog>() {
        match answer {
            DialogResult::Cancel | DialogResult::Ok(CheckChoice::Close) => rt.updater.dismiss_status(),
            DialogResult::Ok(CheckChoice::CheckNow | CheckChoice::CheckAgain) => rt.updater.check_now(),
            // The dialog closes, and the next pass's `run` asks the app to quit once nothing is unsaved.
            DialogResult::Ok(CheckChoice::RestartToUpdate) => {
                rt.updater.restart_to_update();
                rt.updater.dismiss_status();
            }
            DialogResult::Ok(CheckChoice::Update) => {
                rt.updater.request_install(can_start);
            }
            // Download is only offered with a release set on the dialog, which the app never sets: the prompt shows releases.
            _ => {}
        }
    }
    let status = rt.updater.status().clone();
    if matches!(status, Status::Idle) {
        if host.is_open::<UpdateCheckDialog>()
            && let Some(ctx) = &ctx
        {
            host.cancel::<UpdateCheckDialog>(ctx);
        }
        return;
    }
    if !host.is_open::<UpdateCheckDialog>() {
        host.open(UpdateCheckDialog::new(env!("CARGO_PKG_VERSION")));
    }
    if let Some(dialog) = host.get_mut::<UpdateCheckDialog>() {
        dialog.set_status(status);
        dialog.set_installing(rt.updater.is_installing());
        dialog.set_message(rt.updater.status_message());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use craft::update::stack_dialogs::UpdateCheckDialog;
    use craft::update::updater::Status;

    /// With the self-update off, a startup check is never made, and Help ▸ Check for Updates (the
    /// command's own path) ends in the reason instead of a release to install (#5653).
    #[test]
    fn with_the_self_update_off_a_check_finds_nothing_and_says_why() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.set_startup_update_check(true);
        app.disable_self_update();
        assert!(!app.updates_rt.startup_check, "no check at start");
        let ctx = egui::Context::default();
        super::check(&mut app);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !matches!(app.updates_rt.updater.status(), Status::Failed(_)) {
            assert!(std::time::Instant::now() < deadline, "the check never ended: {:?}", app.updates_rt.updater.status());
            super::tick(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let status = app.updates_rt.updater.status();
        assert!(matches!(status, Status::Failed(why) if why.contains("another program's AppImage")), "{status:?}");
        assert!(!app.updates_rt.updater.prompt_open(), "no update offered");
    }

    /// The first start after an update reports what it came to without a menu click: the check at
    /// start reads the record, the result dialog opens on the dialog host by itself, and Esc closes
    /// it and clears the status.
    #[test]
    fn the_last_update_opens_the_result_window_until_it_is_closed() {
        let version = env!("CARGO_PKG_VERSION");
        // The files craft-update writes for an installed update and for a failed one.
        let cases: [(&str, &str, fn(&Status) -> bool); 2] = [
            ("data/self-update-done.txt", version, |s| matches!(s, Status::Updated(v) if v == env!("CARGO_PKG_VERSION"))),
            ("data/logs/self-update-error.txt", "disk full", |s| matches!(s, Status::InstallFailed(why) if why == "disk full")),
        ];
        for (n, (file, text, expected)) in cases.into_iter().enumerate() {
            let root = std::env::temp_dir().join(format!("wordcraft-last-update-{n}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let record = root.join(file);
            std::fs::create_dir_all(record.parent().unwrap()).unwrap();
            std::fs::write(&record, text).unwrap();
            let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            app.updates_rt.updater = craft::update::updater::Updater::new(&crate::UPDATE_APP, version)
                .with_source(std::sync::Arc::new(|| Ok(None)))
                .with_paths(craft::update::Paths::under(&root))
                .with_pins_path(root.join("no-pins.toml"));
            app.set_startup_update_check(true);
            // The dialog host and its Escape need the framework runtime, installed before the first frame.
            let ctx = egui::Context::default();
            craft::ui::shell::embed::install(&ctx, crate::test_harness::install_spec(), &crate::ui_fonts(crate::i18n::Lang::EN)).unwrap();
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            let frame = |app: &mut crate::WordApp, events: Vec<egui::Event>| {
                let time = ctx.input(|i| i.time) + 0.05;
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
                let input = egui::RawInput { events, focused: true, time: Some(time), screen_rect: Some(screen), ..Default::default() };
                craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
                let mut out = ctx.run_ui(input, |ui| {
                    super::tick(app, ui.ctx());
                    super::run(app);
                    app.dialog_host.run(ui);
                });
                out.textures_delta.clear();
            };
            frame(&mut app, Vec::new());
            let status = app.updates_rt.updater.status();
            assert!(expected(status), "{file}: {status:?}");
            assert!(app.dialog_host.is_open::<UpdateCheckDialog>(), "{file}: the result dialog opens by itself");
            assert!(!record.exists(), "{file}: the record is read once");
            frame(&mut app, Vec::new());
            assert!(app.dialog_host.is_open::<UpdateCheckDialog>(), "{file}: the dialog stays open");
            let escape =
                egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            frame(&mut app, vec![escape]);
            // The pass after the answer reads it.
            frame(&mut app, Vec::new());
            assert!(!app.dialog_host.is_open::<UpdateCheckDialog>(), "{file}: Esc closes the dialog");
            assert_eq!(app.updates_rt.updater.status(), &Status::Idle, "{file}: closing clears the status");
            frame(&mut app, Vec::new());
            assert!(!app.dialog_host.is_open::<UpdateCheckDialog>(), "{file}: the dialog stays closed");
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

/// The Nightly channel's rule: the Launcher owns its updates, so a check never reaches the network
/// and says why.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod launcher_updates {
    #[test]
    fn a_nightly_check_never_reaches_the_network_and_names_the_launcher() {
        let error = super::launcher_check()().unwrap_err();
        assert!(error.to_string().contains("ArtCraft Launcher"), "{error}");
    }
}
