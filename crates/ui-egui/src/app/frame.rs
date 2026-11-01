#[cfg(not(target_arch = "wasm32"))]
use super::file_browser;
use super::{
    WordApp, backstage, canvas, chrome, colors, content_controls, dialogs, documents, keys, now_ms, panes, read_aloud, ribbon, theme, updates_ui,
    zotero,
};

impl WordApp {
    /// A file that wouldn't open (#1103): log it, and say on the status bar which file and why,
    /// whatever handed it over (the command line, a macOS open event, a drop). Returns the message.
    pub fn report_open_failure(&mut self, path: &str, error: &str) -> String {
        log::warn!("{path}: {error}");
        let msg = crate::i18n::fmt(crate::tl!("Couldn't open {path}: {error}"), &[("path", path), ("error", error)]);
        self.status(msg.clone());
        msg
    }

    pub fn status(&mut self, s: impl Into<String>) {
        self.status_msg = Some((s.into(), now_ms()));
    }

    /// Document title for the title bar (and the suggested file name). An untitled document is
    /// named in the interface language, as `Document1` is in English.
    pub fn title_stem(&self) -> String {
        let mut title = match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!("Document1").into()),
            None => match &self.session.file_name {
                Some(name) => name.clone(),
                None if self.session.doc.core.title.is_empty() => {
                    documents::untitled_title(&self.session.resolved_filenames().0).unwrap_or_else(|| tl!("Document1").into())
                }
                None => self.session.doc.core.title.clone(),
            },
        };
        // X11 converts window titles to C strings, which cannot contain NUL (#3634).
        title.retain(|c| c != '\0');
        if title.is_empty() { tl!("Document1").into() } else { title }
    }

    /// Tell the host whether the document has unsaved changes ([`Services::on_dirty`]).
    pub(super) fn report_dirty(&self) {
        if let Some(f) = &self.services.on_dirty {
            f(self.session.dirty || panes::has_comment_drafts(self));
        }
    }

    /// Draw the File page, the empty-window chrome, or the normal document window.
    pub(super) fn paint_window(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: theme::Tokens) {
        let window = self.windows.shown();
        let moved = |app: &WordApp| app.windows.shown() != window;
        if self.ui.backstage {
            backstage::show(self, ui);
        } else if !self.has_document() {
            // No document open (#1129): the ribbon greyed out but for Help, and no page.
            chrome::title_bar(self, ui);
            ribbon::show(self, ui);
            chrome::status_bar(self, ui);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |_| {});
        } else {
            let screen = chrome::Screen::of(&self.session.view);
            // Focus and Full Screen (#1678) take the whole screen, with only their slim bar.
            self.focus_ribbon &= screen == chrome::Screen::Focus;
            if screen != chrome::Screen::Normal {
                chrome::screen_bar(self, ui, screen);
            } else {
                chrome::title_bar(self, ui);
                if moved(self) {
                    return;
                }
                ribbon::show(self, ui);
                if moved(self) {
                    return;
                }
                chrome::status_bar(self, ui);
                panes::show(self, ui);
            }
            if moved(self) {
                return;
            }
            let fill = if screen == chrome::Screen::Focus { chrome::FOCUS_CANVAS } else { t.canvas };
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(fill)).show(ui, |ui| {
                canvas::show(self, ui);
            });
            if self.focus_ribbon && !moved(self) {
                chrome::focus_ribbon(self, ctx);
            }
        }
    }

    /// Dialogs, panes and other overlays drawn over the window. A command that brings another
    /// window's state in ends the drawing: the rest would be the other window's.
    pub(super) fn show_overlays(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let window = self.windows.shown();
        let moved = |app: &WordApp| app.windows.shown() != window;
        content_controls::popup(self, ctx);
        dialogs::show(self, ctx);
        if moved(self) {
            return;
        }
        updates_ui::show(self, ctx);
        #[cfg(not(target_arch = "wasm32"))]
        file_browser::show(self, ctx);
        colors::show(self, ctx);
        zotero::show_alert(self, ctx);
        read_aloud::show(self, ctx);
        crate::keytips::show(self, ctx, ui);
        if moved(self) {
            return;
        }
        keys::global_shortcuts(self, ctx);
        if moved(self) {
            return;
        }
        self.refocus_after_chrome_click(ctx);
        if let Some(url) = self.canvas.open_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// Keep the window title and full-screen state in sync, then record this frame's time.
    pub(super) fn sync_title_and_frame(&mut self, ctx: &egui::Context, t0: f64) {
        // With native windows the viewport driver gives the window its title.
        let title = self.window_title();
        if title != self.sent_title {
            if !self.windows.driven() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            }
            self.sent_title = title;
        }
        self.sync_full_screen(ctx);
        self.frame_ms = now_ms() - t0;
        // Typing and formatting land here, after `logic` has reported.
        self.report_dirty();
    }
}
