//! A document whose file another program changed while it was open (#5623).
//!
//! The desktop app looks at every open document's file, the active one's and the parked ones',
//! at most every [`POLL_MS`] while the window has the focus and is painting (a blinking caret,
//! the mouse, a key), and at once when the window gets the focus back: the usual moment, since
//! the person has just been in the editor, the terminal or the agent that wrote the file. No
//! wake-up is asked for: an idle app sleeps, and looks when it next paints or gets the focus.
//!
//! A document with no unsaved changes is read again from its file on the spot (`file.reload`),
//! keeping its id, zoom, scroll, caret, undo history and versions; the content as it was is one
//! more undo step, so Ctrl+Z brings it back. One with unsaved changes is left alone: a bar under
//! the ribbon says the file changed and offers Reload (the file's content, the person's version
//! one Undo away) or Keep mine (the document stays as it is; the next change on disk asks
//! again). Save and AutoSave never write over a file that differs from the one read: Save is
//! refused until the person chooses, and AutoSave waits. A file that changed but can't be read
//! (another program may still be writing it) is tried again at the next look, and the bar says
//! so meanwhile. Headless sessions, tests and the web build don't watch files: only the
//! window's frame loop calls [`WordApp::disk_tick`].

use egui::{RichText, Ui};
use serde_json::json;

use crate::WordApp;
use crate::clock::now_ms;
use crate::theme::Tokens;

/// How often, at most, the files are looked at while the window has the focus, in
/// milliseconds: one `stat` call per open document with a file, cheap enough to repeat, and
/// soon enough after a save made elsewhere.
pub(crate) const POLL_MS: f64 = 2000.0;

/// Why the bar under the ribbon is up for a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiskNotice {
    /// The file changed and the document has unsaved changes: Reload or Keep mine.
    Unsaved,
    /// The file changed but reading it failed (another program may still be writing it): the
    /// next look tries again; Reload tries now, Keep mine stops asking.
    Unreadable(String),
}

/// What one look at the files did to a document ([`WordApp::check_disk_change`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskChange {
    /// The file had changed and the document had no unsaved changes: read again.
    Reloaded,
    /// The file had changed and the document has unsaved changes: the bar asks.
    Asked,
    /// The file had changed but could not be read: the bar says so, and the next look tries
    /// again.
    Failed,
}

impl WordApp {
    /// Once a frame: look at the open documents' files when their turn comes, or at once when
    /// the window has just got the focus back.
    pub(crate) fn disk_tick(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        let regained = focused && !self.window_focused;
        self.window_focused = focused;
        let now = now_ms();
        if regained || now - self.disk_check_at >= POLL_MS {
            self.disk_check_at = now;
            self.check_disk_change();
        }
    }

    /// Compare every open document's file with what was read or written last. Changed, and no
    /// unsaved changes: read again now, a parked document in its place. Changed with unsaved
    /// changes: the bar asks, once. A read that failed is tried again, the bar saying so
    /// meanwhile. A bar whose file is as we know it again (saved over, Keep mine, reloaded) or
    /// whose document closed comes down. Returns what happened to each document it touched.
    pub(crate) fn check_disk_change(&mut self) -> Vec<(u64, DiskChange)> {
        let changed = self.session.changed_on_disk_ids();
        self.disk_notices.retain(|(id, _)| changed.contains(id));
        let mut did = Vec::new();
        for id in changed {
            let notice = self.disk_notices.iter().position(|(d, _)| *d == id);
            if self.session.document_dirty(id) {
                if notice.is_none() {
                    self.disk_notices.push((id, DiskNotice::Unsaved));
                    did.push((id, DiskChange::Asked));
                }
                continue;
            }
            match self.run("file.reload", json!({"id": id})) {
                Ok(_) => {
                    if let Some(at) = notice {
                        self.disk_notices.remove(at);
                    }
                    let name = self.disk_name(id);
                    self.status(crate::i18n::fmt(tl!("{name} was changed by another program and has been reloaded."), &[("name", &name)]));
                    did.push((id, DiskChange::Reloaded));
                }
                Err(e) => {
                    log::warn!("reload from disk of document {id}: {e}");
                    match notice {
                        Some(at) => self.disk_notices[at].1 = DiskNotice::Unreadable(e),
                        None => self.disk_notices.push((id, DiskNotice::Unreadable(e))),
                    }
                    did.push((id, DiskChange::Failed));
                }
            }
        }
        did
    }

    /// Put the bar up for document `id` (unless it is up already): a Save or an AutoSave that
    /// found the file changed, before the next look would have.
    pub(crate) fn notice_disk_change(&mut self, id: u64, why: DiskNotice) {
        if !self.disk_notices.iter().any(|(d, _)| *d == id) {
            self.disk_notices.push((id, why));
        }
    }

    /// The bar's Reload for the document on screen: read the file again, the document's own
    /// version one Undo away.
    pub(crate) fn reload_from_disk(&mut self) {
        let id = self.session.document_id();
        match self.run("file.reload", json!({"force": true})) {
            Ok(_) => {
                self.disk_notices.retain(|(d, _)| *d != id);
                self.status(tl!("Reloaded from disk. Undo brings your version back."));
            }
            Err(e) => {
                log::warn!("reload from disk of document {id}: {e}");
                self.status(e.clone());
                match self.disk_notices.iter().position(|(d, _)| *d == id) {
                    Some(at) => self.disk_notices[at].1 = DiskNotice::Unreadable(e),
                    None => self.disk_notices.push((id, DiskNotice::Unreadable(e))),
                }
            }
        }
    }

    /// The bar's Keep mine: the document on screen stays as it is, and the file as it is now is
    /// what the next change is measured against, so the bar only comes back when another
    /// program writes the file again, and Save writes the document over the file.
    pub(crate) fn keep_mine(&mut self) {
        let id = self.session.document_id();
        self.disk_notices.retain(|(d, _)| *d != id);
        self.session.stamp_disk();
    }

    /// The bar's notice for the document on screen, if any.
    pub(crate) fn disk_notice(&self) -> Option<&DiskNotice> {
        let id = self.session.document_id();
        self.disk_notices.iter().find(|(d, _)| *d == id).map(|(_, why)| why)
    }

    /// What the status line calls document `id`: its file's name.
    fn disk_name(&self, id: u64) -> String {
        self.session
            .documents()
            .into_iter()
            .find(|d| d.id == id)
            .map(|d| d.path.as_ref().and_then(|p| p.file_name()).map_or(d.title, |n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| self.title_stem())
    }
}

/// The bar under the ribbon while the document on screen has a notice: its file changed on
/// disk while it had unsaved changes, or changed and could not be read. Reload or Keep mine.
/// The file itself is looked at by [`WordApp::check_disk_change`], not here.
pub(crate) fn bar(app: &mut WordApp, ui: &mut Ui) {
    let Some(why) = app.disk_notice().cloned() else { return };
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
                let (message, reload_hint, keep_hint) = match &why {
                    DiskNotice::Unsaved => (
                        crate::i18n::fmt(tl!("{name} was changed on disk by another program."), &[("name", &name)]),
                        tl!("Read the file again; Undo brings your version back."),
                        tl!("Keep your version; Save writes it over the file."),
                    ),
                    DiskNotice::Unreadable(error) => (
                        crate::i18n::fmt(
                            tl!("{name} was changed on disk by another program, but could not be read: {error}"),
                            &[("name", &name), ("error", error)],
                        ),
                        tl!("Try reading the file again now."),
                        tl!("Keep the document as it is; Save writes it over the file."),
                    ),
                };
                ui.add(egui::Label::new(RichText::new(message).color(t.text)).truncate());
                if ui.add(crate::widgets::primary_button(ui, tl!("Reload"))).on_hover_text(reload_hint).clicked() {
                    app.reload_from_disk();
                }
                if ui.button(tl!("Keep mine")).on_hover_text(keep_hint).clicked() {
                    app.keep_mine();
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::StoryRef;

    fn app() -> WordApp {
        WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default())
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-disk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app_with_file(name: &str, text: &str) -> (WordApp, std::path::PathBuf) {
        let path = temp_dir(name).join("notes.txt");
        std::fs::write(&path, text).unwrap();
        let mut app = app();
        app.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        (app, path)
    }

    fn text(app: &WordApp) -> String {
        app.session.doc.plain_text(StoryRef::Body)
    }

    fn notice_of(app: &WordApp, id: u64) -> Option<&DiskNotice> {
        app.disk_notices.iter().find(|(d, _)| *d == id).map(|(_, why)| why)
    }

    /// A document without unsaved changes follows its file: read again as soon as the change is
    /// seen, once, with its undo history kept and the content as it was one Undo away.
    #[test]
    fn a_document_without_unsaved_changes_is_reloaded_when_its_file_changes() {
        let (mut app, path) = app_with_file("clean", "alpha");
        let id = app.session.document_id();
        assert!(app.check_disk_change().is_empty());
        app.run("text.insert", json!({"text": "X"})).unwrap();
        app.run("file.save", json!({})).unwrap();
        std::fs::write(&path, "alpha beta").unwrap();
        assert_eq!(app.check_disk_change(), vec![(id, DiskChange::Reloaded)]);
        assert_eq!(text(&app), "alpha beta");
        assert!(app.disk_notices.is_empty(), "nothing to ask");
        assert!(app.check_disk_change().is_empty(), "read once");
        assert_eq!(app.session.undo_label(), Some("Reload from Disk"));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&app), "Xalpha", "Ctrl+Z still works after a reload");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&app), "alpha", "and the history before it is there");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A document with unsaved changes is asked, and nothing is lost until the person chooses:
    /// Keep mine keeps the document (and asks again only after the next change on disk), Reload
    /// takes the file's version, with the person's one Undo away.
    #[test]
    fn a_document_with_unsaved_changes_is_asked_and_nothing_is_lost() {
        let (mut app, path) = app_with_file("dirty", "alpha");
        let id = app.session.document_id();
        app.run("text.insert", json!({"text": "X"})).unwrap();
        std::fs::write(&path, "alpha beta").unwrap();
        assert_eq!(app.check_disk_change(), vec![(id, DiskChange::Asked)]);
        assert_eq!(text(&app), "Xalpha", "nothing was lost");
        assert_eq!(app.disk_notice(), Some(&DiskNotice::Unsaved));
        assert!(app.check_disk_change().is_empty(), "asked once");
        app.keep_mine();
        assert!(app.disk_notices.is_empty());
        assert!(!app.session.changed_on_disk(), "the file as it is now is the one we know");
        assert_eq!(text(&app), "Xalpha");
        std::fs::write(&path, "alpha beta gamma").unwrap();
        assert_eq!(app.check_disk_change(), vec![(id, DiskChange::Asked)], "the next change on disk asks again");
        app.reload_from_disk();
        assert_eq!(text(&app), "alpha beta gamma");
        assert!(!app.session.dirty);
        assert!(app.disk_notices.is_empty());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&app), "Xalpha", "the unsaved changes are one Undo away");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Save and AutoSave never write over the other program's changes without asking: Save is
    /// refused and puts the bar up, AutoSave waits (and stays on), until the person chooses.
    /// Keep mine then lets the save through.
    #[test]
    fn save_and_autosave_never_write_over_a_changed_file_unasked() {
        // A .docx saved from the app: AutoSave covers only a file saved here, in a format that
        // keeps everything.
        let dir = temp_dir("save");
        let path = dir.join("notes.docx");
        let mut app = app();
        app.run("text.insert", json!({"text": "alpha"})).unwrap();
        app.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        let id = app.session.document_id();
        assert!(app.autosaves(), "AutoSave covers a file saved here");
        app.run("text.insert", json!({"text": "X"})).unwrap();
        std::fs::write(&path, "their text").unwrap();
        app.autosave_tick(now_ms() + 100_000.0);
        assert_eq!(std::fs::read(&path).unwrap(), b"their text", "AutoSave waited");
        assert!(app.session.dirty);
        assert!(app.autosaves(), "and stays on");
        assert_eq!(notice_of(&app, id), Some(&DiskNotice::Unsaved), "the bar asks");
        app.disk_notices.clear();
        assert!(app.run("file.save", json!({})).is_err(), "Save is refused");
        assert_eq!(std::fs::read(&path).unwrap(), b"their text");
        assert_eq!(notice_of(&app, id), Some(&DiskNotice::Unsaved), "and puts the bar up");
        app.keep_mine();
        app.run("file.save", json!({})).unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(b"PK"), "chosen: the document is written over the file");
        assert!(!app.session.dirty && !app.session.changed_on_disk());
        assert!(app.disk_notices.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that changed but can't be read yet (another program still writing it, say) isn't
    /// given up on: the bar says so, and the next look reads it once it can.
    #[test]
    fn a_file_that_cannot_be_read_is_tried_again_at_the_next_look() {
        let dir = temp_dir("unreadable");
        let path = dir.join("notes.docx");
        let mut app = app();
        app.run("text.insert", json!({"text": "alpha"})).unwrap();
        app.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        let id = app.session.document_id();
        let good = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"not a document yet").unwrap();
        assert_eq!(app.check_disk_change(), vec![(id, DiskChange::Failed)]);
        assert!(matches!(app.disk_notice(), Some(DiskNotice::Unreadable(_))), "{:?}", app.disk_notices);
        assert_eq!(text(&app), "alpha", "the document is as it was");
        assert_eq!(app.check_disk_change(), vec![(id, DiskChange::Failed)], "tried again, not latched");
        std::fs::write(&path, &good).unwrap();
        assert_eq!(app.check_disk_change(), vec![(id, DiskChange::Reloaded)]);
        assert!(app.disk_notices.is_empty(), "read: the bar comes down");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every open document is watched, not only the one on screen: a parked document's file
    /// that changed is read again in its place, and the one on screen stays as it is.
    #[test]
    fn a_parked_document_is_reloaded_in_its_place() {
        let (mut app, a_path) = app_with_file("parked", "alpha");
        let a = app.session.document_id();
        let b_path = a_path.with_file_name("other.txt");
        std::fs::write(&b_path, "bravo").unwrap();
        app.run("file.open", json!({"path": b_path.to_string_lossy()})).unwrap();
        let b = app.session.document_id();
        app.run("text.insert", json!({"text": "X"})).unwrap();
        std::fs::write(&a_path, "alpha changed").unwrap();
        assert_eq!(app.check_disk_change(), vec![(a, DiskChange::Reloaded)]);
        assert_eq!(app.session.document_id(), b, "the document on screen stays");
        assert_eq!(text(&app), "Xbravo");
        app.session.activate(a).unwrap();
        assert_eq!(text(&app), "alpha changed");
        // A parked document with unsaved changes gets its bar when it comes on screen.
        app.session.activate(b).unwrap();
        std::fs::write(&b_path, "bravo changed").unwrap();
        app.session.activate(a).unwrap();
        assert_eq!(app.check_disk_change(), vec![(b, DiskChange::Asked)]);
        assert_eq!(app.disk_notice(), None, "not this document's");
        app.session.activate(b).unwrap();
        assert_eq!(app.disk_notice(), Some(&DiskNotice::Unsaved));
        // Closed: its bar goes with it.
        app.run("file.closeDocument", json!({"id": b, "force": true})).unwrap();
        app.check_disk_change();
        assert!(app.disk_notices.is_empty());
        let _ = std::fs::remove_dir_all(a_path.parent().unwrap());
    }

    /// A document without a file (new, never saved) has nothing to watch.
    #[test]
    fn a_new_document_is_never_reloaded() {
        let mut app = app();
        app.run("text.insert", json!({"text": "X"})).unwrap();
        assert!(app.check_disk_change().is_empty());
        assert_eq!(text(&app), "X");
    }
}
