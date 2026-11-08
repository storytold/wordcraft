//! Software updates, background check at startup, and startup prompt modal.

use crate::WordApp;

#[cfg(not(target_arch = "wasm32"))]
use craft_update::dialog::{CheckResultDialog, CheckResultResponse};
#[cfg(not(target_arch = "wasm32"))]
use craft_update::updater::{Action, Status, Updater};

pub(crate) struct Runtime {
    #[cfg(not(target_arch = "wasm32"))]
    updater: Updater,
    #[cfg(not(target_arch = "wasm32"))]
    result_dialog: CheckResultDialog,
    /// The desktop app turns the check at start on; tests and the web build leave it off.
    #[cfg(not(target_arch = "wasm32"))]
    startup_check: bool,
    #[cfg(not(target_arch = "wasm32"))]
    check_requested: bool,
    /// The context of the last frame, to wake the UI when a check is queued from a menu callback.
    #[cfg(not(target_arch = "wasm32"))]
    repaint: Option<egui::Context>,
    pub(crate) manual_window_open: bool,
    #[cfg(target_arch = "wasm32")]
    pub(crate) status_message: Option<String>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            updater: Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")),
            #[cfg(not(target_arch = "wasm32"))]
            result_dialog: CheckResultDialog::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")),
            #[cfg(not(target_arch = "wasm32"))]
            startup_check: false,
            #[cfg(not(target_arch = "wasm32"))]
            check_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            repaint: None,
            manual_window_open: false,
            #[cfg(target_arch = "wasm32")]
            status_message: None,
        }
    }
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
        rt.updater = Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).with_source(refused_check());
    }
}

/// A check for updates that never finds one and gives the reason as its error, which Help ▸ Check
/// for Updates shows.
#[cfg(not(target_arch = "wasm32"))]
fn refused_check() -> craft_update::updater::CheckSource {
    let why = tl!("Updates are off in this window: WordCraft was started with another program's AppImage settings. Start WordCraft from its own menu entry to update it.")
        .to_string();
    std::sync::Arc::new(move || Err(why.clone()))
}

pub(crate) fn check(app: &mut WordApp) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.updates_rt.check_requested = true;
        app.updates_rt.manual_window_open = true;
        if let Some(ctx) = &app.updates_rt.repaint {
            ctx.request_repaint();
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        app.updates_rt.manual_window_open = true;
        app.updates_rt.status_message = Some(tl!("Update check is not supported in the web browser.").to_string());
    }
}

pub(crate) fn tick(app: &mut WordApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let rt = &mut app.updates_rt;
        rt.repaint = Some(ctx.clone());
        if std::mem::take(&mut rt.check_requested) {
            rt.updater.check_now(ctx);
        } else if rt.startup_check {
            rt.updater.check_at_start(ctx);
        }
        rt.updater.tick(ctx);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

pub(crate) fn show(app: &mut WordApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The restart helper is scheduled before the app closes, so no update starts over unsaved work.
        let clean = !(app.session.dirty || app.save_unconfirmed() || app.printing.busy() || crate::panes::has_comment_drafts(app));
        match app.updates_rt.updater.show_if(ctx, clean) {
            Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::None => {}
        }
        // The dialog answers Help ▸ Check for Updates; a found update leaves the status idle and the prompt above takes over.
        let rt = &mut app.updates_rt;
        if rt.manual_window_open {
            match rt.result_dialog.show(ctx, rt.updater.status()) {
                CheckResultResponse::None => {}
                CheckResultResponse::Close => {
                    rt.manual_window_open = false;
                    rt.updater.dismiss_status();
                }
                CheckResultResponse::CheckAgain => rt.updater.check_now(ctx),
            }
            // An update was found: the prompt replaces the dialog.
            if matches!(rt.updater.status(), Status::Idle) {
                rt.manual_window_open = false;
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    if app.updates_rt.manual_window_open {
        let mut open = true;
        egui::Window::new(tl!("Check for Updates"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                crate::rtl::root(ui, |ui| {
                    if let Some(msg) = &app.updates_rt.status_message {
                        ui.label(msg);
                    }
                })
            });
        if !open {
            app.updates_rt.manual_window_open = false;
            app.updates_rt.status_message = None;
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use craft_update::updater::Status;

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
}
