//! The self-hosted update notice (feature `self-hosted-update`, off by default).
//!
//! For a WordCraft web build served from someone's own server: a strip above the document tells
//! every visitor that a newer release exists, and the administrator can sign in and start the
//! update. The update itself is done on the server by `craft-updated` (craft-libs'
//! `craft-update` crate), reached through the web server at `SERVICE_PATH` next to this page;
//! it checks the administrator's sign-in on every request. Setup: docs/self-hosted-updates.md.

use craft_update::client::Backend;
use craft_update::ui::{Strings, UpdateBanner};
use wordcraft_ui_egui::i18n;

/// Where the web server proxies `craft-updated`, relative to the page.
#[cfg(target_arch = "wasm32")]
pub const SERVICE_PATH: &str = "craft-update/";

pub struct Updates<B: Backend> {
    banner: UpdateBanner<B>,
    strings: Strings,
    language: Option<&'static str>,
}

impl<B: Backend> Updates<B> {
    pub fn new(backend: B) -> Self {
        Updates { banner: UpdateBanner::new(backend, env!("CARGO_PKG_VERSION")), strings: Strings::english(), language: None }
    }

    /// Draws the notice (when there is one) at the top of `ui`, in the interface language (set
    /// by `WordApp::logic` each frame); call before the app draws so the document sits below it.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let lang = i18n::current();
        if self.language != Some(lang.code()) {
            self.strings = Strings::translated(|english| i18n::tr(lang, english).to_owned());
            self.language = Some(lang.code());
        }
        self.banner.show(ui, &self.strings);
    }

    #[cfg(test)]
    pub fn banner(&mut self) -> &mut UpdateBanner<B> {
        &mut self.banner
    }
}

/// The browser client, waking egui when an answer arrives.
#[cfg(target_arch = "wasm32")]
pub fn browser(ctx: egui::Context) -> Updates<craft_update::web::Client> {
    Updates::new(craft_update::web::Client::new(SERVICE_PATH, move || ctx.request_repaint()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_update::client::{ApiError, Reply};
    use craft_update::protocol::{Job, LoginReply, Status};
    use craft_update::ui::Notice;
    use egui_kittest::kittest::{NodeT, Queryable};
    use wordcraft_engine::Session;
    use wordcraft_ui_egui::{Services, WordApp};

    #[derive(Default)]
    struct Fake {
        replies: Vec<Reply>,
        logins: Vec<String>,
        updates: Vec<String>,
        reloads: usize,
    }

    impl Backend for Fake {
        fn request_status(&mut self) {}
        fn request_login(&mut self, username: &str, password: &str) {
            self.logins.push(format!("{username}:{password}"));
        }
        fn request_update(&mut self, token: &str) {
            self.updates.push(token.into());
        }
        fn request_logout(&mut self, _: &str) {}
        fn poll(&mut self) -> Vec<Reply> {
            std::mem::take(&mut self.replies)
        }
        fn reload_page(&mut self) {
            self.reloads += 1;
        }
    }

    struct Shell {
        app: WordApp,
        updates: Updates<Fake>,
    }

    /// The web shell's drawing order: the notice, then the app below it.
    fn shell() -> egui_kittest::Harness<'static, Shell> {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let shell = Shell { app, updates: Updates::new(Fake::default()) };
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
            |ui, shell: &mut Shell| {
                shell.app.logic(&ui.ctx().clone());
                shell.updates.show(ui);
                shell.app.ui(ui);
            },
            shell,
        );
        h.run_steps(6);
        h
    }

    fn fake<'h>(h: &'h mut egui_kittest::Harness<'static, Shell>) -> &'h mut Fake {
        h.state_mut().updates.banner().backend_mut()
    }

    fn answer(h: &mut egui_kittest::Harness<'static, Shell>, reply: Reply) {
        fake(h).replies.push(reply);
        h.run_steps(6);
    }

    fn type_into(h: &mut egui_kittest::Harness<'static, Shell>, label: &str, text: &str) {
        h.get_by_label(label).click();
        h.run_steps(1);
        h.get_by_label(label).type_text(text);
        h.run_steps(1);
    }

    fn available() -> Status {
        Status {
            current: env!("CARGO_PKG_VERSION").into(),
            latest: Some("99.0.0".into()),
            release_url: Some("https://github.com/storytold/wordcraft/releases/tag/v99.0.0".into()),
            update_available: true,
            checked_at: None,
            job: Job::Idle,
        }
    }

    #[test]
    fn visitors_see_the_notice_above_the_document_and_only_the_administrator_can_update() {
        let mut h = shell();
        let ribbon_before = h.get_by_label("Bold").rect().top();
        assert!(h.query_by_label_contains("is available").is_none());

        // A visitor: the notice, above the ribbon, with Update now disabled.
        answer(&mut h, Reply::Status(Ok(available())));
        let notice = format!("A new version (99.0.0) is available. This site runs {}.", env!("CARGO_PKG_VERSION"));
        let strip = h.get_by_label(&notice).rect();
        let ribbon = h.get_by_label("Bold").rect().top();
        assert!(
            ribbon > ribbon_before && ribbon >= strip.bottom(),
            "the ribbon moves below the notice ({ribbon_before} -> {ribbon}, strip ends {})",
            strip.bottom()
        );
        assert!(h.get_by_label("Update now").accesskit_node().is_disabled());
        h.get_by_label("Update now").click();
        h.run_steps(3);
        assert!(fake(&mut h).updates.is_empty());

        // A wrong password keeps it disabled.
        h.get_by_label("Administrator sign-in…").click();
        h.run_steps(4);
        type_into(&mut h, "Username", "admin");
        type_into(&mut h, "Password", "guess");
        h.get_by_label("Sign in").click();
        h.run_steps(3);
        assert_eq!(fake(&mut h).logins, ["admin:guess"]);
        answer(&mut h, Reply::Login(Err(ApiError::Unauthorized)));
        h.get_by_label("Wrong username or password.");
        assert!(h.get_by_label("Update now").accesskit_node().is_disabled());

        // The administrator: enabled, and the click sends the session's token.
        answer(&mut h, Reply::Login(Ok(LoginReply { token: "session".into(), expires_in: 900 })));
        let update = h.get_by_label("Update now");
        assert!(!update.accesskit_node().is_disabled());
        update.click();
        h.run_steps(4);
        assert_eq!(fake(&mut h).updates, ["session"]);
    }

    #[test]
    fn everyone_sees_the_update_run_and_then_a_reload_prompt() {
        let mut h = shell();
        answer(&mut h, Reply::Status(Ok(Status { job: Job::Running { version: "99.0.0".into(), started_at: 1 }, ..available() })));
        h.get_by_label("Updating to 99.0.0… The site keeps working meanwhile.");
        assert!(h.query_by_label("Update now").is_none());
        let done = Status {
            current: "99.0.0".into(),
            update_available: false,
            job: Job::Succeeded { version: "99.0.0".into(), finished_at: 2 },
            ..available()
        };
        answer(&mut h, Reply::Status(Ok(done)));
        assert_eq!(h.state_mut().updates.banner().notice(), Some(Notice::Installed { latest: "99.0.0".into() }));
        h.get_by_label("Reload").click();
        h.run_steps(3);
        assert_eq!(fake(&mut h).reloads, 1);
    }
}
