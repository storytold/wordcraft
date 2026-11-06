//! A document whose file another program changed while it was open (#5623).
//!
//! The desktop app looks at the active document's file every [`POLL_MS`] while the window has
//! the focus, and at once when the window gets the focus back: the usual moment, since the
//! person has just been in the editor, the terminal or the agent that wrote the file. A document
//! with no unsaved changes is read again from the file on the spot (`file.reload`), keeping its
//! id, zoom, scroll and caret. One with unsaved changes is left alone: a bar under the ribbon
//! says the file changed and offers Reload (the unsaved changes are discarded) or Keep mine (the
//! document stays as it is; the next change on disk asks again). Nothing is overwritten
//! silently either way. Headless sessions, tests and the web build don't watch files: only the
//! window's frame loop calls [`WordApp::disk_tick`].

use egui::{RichText, Ui};
use serde_json::json;

use crate::WordApp;
use crate::clock::now_ms;
use crate::theme::Tokens;

/// How often the file is looked at while the window has the focus, in milliseconds: one `stat`
/// call, cheap enough to repeat, and soon enough after a save made elsewhere.
pub(crate) const POLL_MS: f64 = 2000.0;

/// What one look at the file did ([`WordApp::check_disk_change`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskChange {
    /// The file is as it was read or written last, or the bar is already asking.
    None,
    /// The file had changed and the document had no unsaved changes: read again.
    Reloaded,
    /// The file had changed and the document has unsaved changes (or reading it failed): the
    /// bar asks.
    Asked,
}

impl WordApp {
    /// Once a frame, for the window's document: look at its file when its turn comes, or at
    /// once when the window has just got the focus back, and arrange the next look.
    pub(crate) fn disk_tick(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        let regained = focused && !self.window_focused;
        self.window_focused = focused;
        if self.session.path.is_none() {
            return;
        }
        let now = now_ms();
        if regained || now - self.disk_check_at >= POLL_MS {
            self.disk_check_at = now;
            self.check_disk_change();
        }
        if focused {
            ctx.request_repaint_after(std::time::Duration::from_millis(POLL_MS as u64));
        }
    }

    /// Compare the active document's file with what was read or written last. Changed, and no
    /// unsaved changes: read again now. Changed with unsaved changes, or a read that failed (the
    /// other program may still be writing): the bar asks. Returns what happened.
    pub(crate) fn check_disk_change(&mut self) -> DiskChange {
        let id = self.session.document_id();
        if self.disk_notice == Some(id) || !self.session.changed_on_disk() {
            return DiskChange::None;
        }
        if self.session.dirty {
            self.disk_notice = Some(id);
            return DiskChange::Asked;
        }
        match self.run("file.reload", json!({})) {
            Ok(_) => {
                self.status(tl!("The file was changed by another program and has been reloaded."));
                DiskChange::Reloaded
            }
            Err(e) => {
                log::warn!("reload from disk: {e}");
                self.status(e);
                self.disk_notice = Some(id);
                DiskChange::Asked
            }
        }
    }

    /// The bar's Reload: read the file again, the unsaved changes discarded.
    pub(crate) fn reload_from_disk(&mut self) {
        self.disk_notice = None;
        match self.run("file.reload", json!({"force": true})) {
            Ok(_) => self.status(tl!("Reloaded from disk.")),
            Err(e) => self.status(e),
        }
    }

    /// The bar's Keep mine: the document stays as it is, and the file as it is now is what the
    /// next change is measured against, so the bar only comes back when another program writes
    /// the file again. Saving writes the document over the file, as it always did.
    pub(crate) fn keep_mine(&mut self) {
        self.disk_notice = None;
        self.session.stamp_disk();
    }
}

/// The bar under the ribbon while the document on screen has unsaved changes and its file was
/// changed on disk: Reload or Keep mine.
pub(crate) fn bar(app: &mut WordApp, ui: &mut Ui) {
    let document = app.session.document_id();
    if app.disk_notice != Some(document) {
        return;
    }
    // Saved over since (the person's version is the file now), or the file is gone: nothing to ask.
    if !app.session.changed_on_disk() {
        app.disk_notice = None;
        return;
    }
    let name = app.session.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| app.title_stem(), |n| n.to_string_lossy().to_string());
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("disk_change_bar")
        .exact_size(36.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            let full = ui.max_rect();
            ui.painter().hline(full.x_range(), full.bottom() - 0.5, egui::Stroke::new(1.0, t.border));
            ui.horizontal_centered(|ui| {
                let message = crate::i18n::fmt(tl!("{name} was changed on disk by another program."), &[("name", &name)]);
                ui.add(egui::Label::new(RichText::new(message).color(t.text)).truncate());
                if ui
                    .add(crate::widgets::primary_button(ui, tl!("Reload")))
                    .on_hover_text(tl!("Read the file again; your unsaved changes are lost."))
                    .clicked()
                {
                    app.reload_from_disk();
                }
                if ui.button(tl!("Keep mine")).on_hover_text(tl!("Keep your version; Save writes it over the file.")).clicked() {
                    app.keep_mine();
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::StoryRef;

    fn app_with_file(name: &str, text: &str) -> (WordApp, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-disk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.txt");
        std::fs::write(&path, text).unwrap();
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        (app, path)
    }

    fn text(app: &WordApp) -> String {
        app.session.doc.plain_text(StoryRef::Body)
    }

    /// A document without unsaved changes follows its file: read again as soon as the change is
    /// seen, once.
    #[test]
    fn a_document_without_unsaved_changes_is_reloaded_when_its_file_changes() {
        let (mut app, path) = app_with_file("clean", "alpha");
        assert_eq!(app.check_disk_change(), DiskChange::None);
        std::fs::write(&path, "alpha beta").unwrap();
        assert_eq!(app.check_disk_change(), DiskChange::Reloaded);
        assert_eq!(text(&app), "alpha beta");
        assert_eq!(app.disk_notice, None, "nothing to ask");
        assert_eq!(app.check_disk_change(), DiskChange::None, "read once");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A document with unsaved changes is asked, and nothing is lost until the person chooses:
    /// Keep mine keeps the document (and asks again only after the next change on disk), Reload
    /// takes the file's version.
    #[test]
    fn a_document_with_unsaved_changes_is_asked_and_nothing_is_lost() {
        let (mut app, path) = app_with_file("dirty", "alpha");
        app.run("text.insert", json!({"text": "X"})).unwrap();
        std::fs::write(&path, "alpha beta").unwrap();
        assert_eq!(app.check_disk_change(), DiskChange::Asked);
        assert_eq!(text(&app), "Xalpha", "nothing was lost");
        assert_eq!(app.disk_notice, Some(app.session.document_id()));
        assert_eq!(app.check_disk_change(), DiskChange::None, "asked once");
        app.keep_mine();
        assert_eq!(app.disk_notice, None);
        assert!(!app.session.changed_on_disk(), "the file as it is now is the one we know");
        assert_eq!(text(&app), "Xalpha");
        std::fs::write(&path, "alpha beta gamma").unwrap();
        assert_eq!(app.check_disk_change(), DiskChange::Asked, "the next change on disk asks again");
        app.reload_from_disk();
        assert_eq!(text(&app), "alpha beta gamma");
        assert!(!app.session.dirty);
        assert_eq!(app.disk_notice, None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A document without a file (new, never saved) has nothing to watch.
    #[test]
    fn a_new_document_is_never_reloaded() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("text.insert", json!({"text": "X"})).unwrap();
        assert_eq!(app.check_disk_change(), DiskChange::None);
        assert_eq!(text(&app), "X");
    }
}
