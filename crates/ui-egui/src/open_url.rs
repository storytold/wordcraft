//! Every link the interface opens (the Discord button, Ctrl+click on a document's hyperlink, the
//! links in About) is asked for with `ctx.open_url`. Where the host sets
//! [`Services::open_url`](crate::Services::open_url), [`route`] takes each of them out of the
//! frame's output and hands it to the host, which opens it with the system (#2349: on SteamOS
//! egui's own opener finds no browser). Without the hook (the web, macOS, Windows) egui opens
//! links itself. A link the host couldn't open shows a notice with the address and a Copy button.

use std::sync::mpsc::{Receiver, TryRecvError};

use crate::{WordApp, i18n};

/// The [`Services::open_url`](crate::Services::open_url) hook: opens a web or email link with the
/// system, off the UI thread, and reports through the receiver how it went (an `Err` says no way
/// of opening it worked). A dropped sender counts as a failure.
pub type Hook = Box<dyn Fn(&str) -> Receiver<Result<(), String>>>;

/// Links handed to the host and not answered yet, and the last one that failed.
#[derive(Default)]
pub struct State {
    pending: Vec<(String, Receiver<Result<(), String>>)>,
    failed: Option<String>,
}

/// Hand the links asked for this frame to the host's hook. Only web and email links leave the app.
pub(crate) fn route(app: &mut WordApp, ctx: &egui::Context) {
    let Some(hook) = &app.services.open_url else { return };
    let urls: Vec<String> = ctx.output_mut(|o| {
        let mut urls = Vec::new();
        o.commands.retain(|c| match c {
            egui::OutputCommand::OpenUrl(u) => {
                urls.push(u.url.clone());
                false
            }
            _ => true,
        });
        urls
    });
    for url in urls {
        if wordcraft_doc::link::is_web_or_mail_link(&url) {
            let answer = hook(&url);
            app.open_urls.pending.push((url, answer));
        } else {
            log::warn!("refused to open {url:?}: only http, https and mailto links are opened");
        }
    }
}

/// Collect the host's answers, and show the notice for a link that couldn't be opened.
pub(crate) fn show(app: &mut WordApp, ctx: &egui::Context) {
    let state = &mut app.open_urls;
    state.pending.retain(|(url, answer)| match answer.try_recv() {
        Err(TryRecvError::Empty) => true,
        Ok(Ok(())) => false,
        Ok(Err(e)) => {
            log::warn!("couldn't open {url}: {e}");
            state.failed = Some(url.clone());
            false
        }
        Err(TryRecvError::Disconnected) => {
            state.failed = Some(url.clone());
            false
        }
    });
    let Some(url) = state.failed.clone() else { return };
    let mut close = false;
    let mut copied = false;
    egui::Window::new(tl!("Open link"))
        .id(egui::Id::new("open-url-failed"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -48.0))
        .show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.label(tl!("No browser could be opened for this link. Copy it and paste it into a browser:"));
            ui.add(egui::Label::new(egui::RichText::new(&url).monospace()).selectable(true).wrap());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Copy link")).clicked() {
                    ctx.copy_text(url.clone());
                    copied = true;
                }
                close = ui.button(tl!("Close")).clicked();
            });
        });
    if copied || close {
        app.open_urls.failed = None;
    }
    if copied {
        app.status(i18n::fmt(tl!("Link copied: {url}"), &[("url", &url)]));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_engine::Session;

    use crate::{Services, WordApp};

    const DISCORD: &str = "https://discord.gg/artcraft";

    /// An app whose host records each link it is asked to open and answers `answer`.
    fn app_answering(answer: Result<(), String>) -> (WordApp, Rc<RefCell<Vec<String>>>) {
        let asked = Rc::new(RefCell::new(Vec::new()));
        let seen = asked.clone();
        let services = Services {
            open_url: Some(Box::new(move |url: &str| {
                seen.borrow_mut().push(url.to_string());
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(answer.clone());
                rx
            })),
            ..Default::default()
        };
        (WordApp::new(Session::new(wordcraft_doc::Document::new()), services), asked)
    }

    fn harness(app: WordApp) -> Harness<'static, WordApp> {
        Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        )
    }

    fn opened_by_egui(h: &Harness<'_, WordApp>) -> Vec<String> {
        h.output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::OpenUrl(u) => Some(u.url.clone()),
                _ => None,
            })
            .collect()
    }

    fn click_discord_in_backstage(h: &mut Harness<'_, WordApp>) {
        let _ = h.state_mut().run("ui.backstage", json!({"value": true}));
        h.run_steps(4);
        h.get_by_label("Discord community").click();
        h.run_steps(3);
    }

    #[test]
    fn backstage_discord_goes_to_the_host() {
        let (app, asked) = app_answering(Ok(()));
        let mut h = harness(app);
        click_discord_in_backstage(&mut h);
        assert_eq!(*asked.borrow(), [DISCORD]);
        assert!(opened_by_egui(&h).is_empty(), "egui's own opener isn't asked");
        assert!(h.query_by_label(DISCORD).is_none(), "no notice when it opened");
    }

    /// Ctrl+click on a document link and the About links reach egui as `ctx.open_url`; all of them
    /// go to the host, and only web and email links leave the app.
    #[test]
    fn every_link_asked_of_egui_goes_to_the_host() {
        let (app, asked) = app_answering(Ok(()));
        let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                for url in ["https://example.org/a", "mailto:someone@example.org", "file:///etc/passwd", "smb://host/share"] {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        assert!(opened_by_egui(&h).is_empty(), "nothing is left for egui's opener");
        let asked = asked.borrow();
        assert!(asked.contains(&"https://example.org/a".into()) && asked.contains(&"mailto:someone@example.org".into()), "{asked:?}");
        assert!(asked.iter().all(|u| u == "https://example.org/a" || u == "mailto:someone@example.org"), "{asked:?}");
    }

    #[test]
    fn without_a_hook_egui_opens_links_as_before() {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut h = harness(app);
        h.run_steps(3);
        let _ = h.state_mut().run("ui.discord", json!({}));
        h.step();
        assert_eq!(opened_by_egui(&h), [DISCORD]);
    }

    /// The issue's second ask: a link that can't be opened is shown, and Copy puts it on the
    /// clipboard. From Backstage, which hides the status bar.
    #[test]
    fn a_link_that_cant_be_opened_is_shown_with_copy() {
        let (app, asked) = app_answering(Err("no browser".into()));
        let mut h = harness(app);
        click_discord_in_backstage(&mut h);
        assert_eq!(*asked.borrow(), [DISCORD]);
        h.get_by_label(DISCORD);
        h.get_by_label("Copy link").click();
        h.step();
        let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == DISCORD));
        assert!(copied, "{:?}", h.output().platform_output.commands);
        h.run_steps(2);
        assert!(h.query_by_label("Copy link").is_none(), "the notice closes");
    }

    #[test]
    fn a_host_that_drops_the_answer_counts_as_a_failure() {
        let services = Services { open_url: Some(Box::new(|_: &str| std::sync::mpsc::channel().1)), ..Default::default() };
        let mut h = harness(WordApp::new(Session::new(wordcraft_doc::Document::new()), services));
        click_discord_in_backstage(&mut h);
        h.get_by_label(DISCORD);
        let notice = h.ctx.memory(|m| m.area_rect(egui::Id::new("open-url-failed"))).expect("the notice is open");
        h.query_all_by_label("Close").find(|n| notice.contains(n.rect().center())).expect("its Close button").click();
        h.run_steps(2);
        assert!(h.query_by_label(DISCORD).is_none());
    }
}
