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
            // A comment or reply still being typed is unsaved work too (#1348): the bar asks.
            if self.session.document_dirty(id) || self.document_has_comment_drafts(id) {
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
    /// version one Undo away. A comment or reply still being typed is written into that version
    /// first (`file.reload` flushes drafts in [`WordApp::execute`], #1348), so Undo brings it back
    /// too; a draft the document refused goes with the content it was typed against.
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

    /// Before `file.reload` (#5623), from the bar or from an agent: a comment or reply still
    /// being typed is written into the content as it was (#1348), which the reload keeps one
    /// Undo away: the shown document's drafts, or the parked ones of the document the reload
    /// names. Returns that document's id, for [`WordApp::drafts_after_reload`]; `None` for any
    /// other command.
    pub(crate) fn drafts_before_reload(&mut self, id: &str, params: &serde_json::Value) -> Option<u64> {
        if id != "file.reload" {
            return None;
        }
        let target = params.get("id").and_then(serde_json::Value::as_u64).unwrap_or(self.session.document_id());
        let _ = if target == self.session.document_id() {
            crate::panes::flush_comment_drafts(self)
        } else {
            crate::panes::flush_parked_comment_drafts(self, target)
        };
        Some(target)
    }

    /// After a reload of document `target`: what is left of its drafts (one the document refused)
    /// went with the content as it was; none waits for the new content, where a comment number
    /// may be another one.
    pub(crate) fn drafts_after_reload(&mut self, target: u64) {
        if target == self.session.document_id() {
            crate::panes::discard_comment_drafts(self);
        } else {
            crate::panes::discard_parked_comment_drafts(self, target);
        }
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

    /// A reload keeps the view where the person is reading (#5623): neither the automatic
    /// reload nor the bar's Reload scrolls to the caret.
    #[test]
    fn a_reload_does_not_scroll_the_view_to_the_caret() {
        let (mut app, path) = app_with_file("view", "alpha");
        app.canvas.scroll_to_caret = false;
        std::fs::write(&path, "alpha beta").unwrap();
        assert_eq!(app.check_disk_change().len(), 1);
        assert!(!app.canvas.scroll_to_caret, "the view stays");
        app.run("text.insert", json!({"text": "X"})).unwrap();
        app.canvas.scroll_to_caret = false;
        std::fs::write(&path, "alpha beta gamma").unwrap();
        app.check_disk_change();
        app.reload_from_disk();
        assert_eq!(text(&app), "alpha beta gamma");
        assert!(!app.canvas.scroll_to_caret, "the bar's Reload keeps it too");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A comment or reply still being typed is unsaved work (#1348): a document with one is
    /// asked, not reloaded under it.
    #[test]
    fn a_document_with_a_comment_draft_is_asked_not_reloaded() {
        let dir = temp_dir("draft");
        let path = dir.join("notes.docx");
        let mut app = app();
        app.run("text.insert", json!({"text": "alpha"})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let comment = app.run("review.newComment", json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
        app.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        let id = app.session.document_id();
        assert!(!app.session.dirty);
        crate::panes::draft_reply(&mut app, comment, "typing a reply");
        // Another program writes the file: a headless session saving its own document there.
        let mut other = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        other.run("text.insert", &json!({"text": "theirs"})).unwrap();
        other.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(app.check_disk_change(), vec![(id, DiskChange::Asked)], "the draft is unsaved work");
        assert_eq!(app.disk_notice(), Some(&DiskNotice::Unsaved));
        assert_eq!(text(&app), "alpha", "nothing was lost");
        // Reload: the draft is written into the version one Undo away, not left for the new
        // content, where a comment with the same number may be another one.
        app.reload_from_disk();
        assert_eq!(text(&app), "theirs", "Reload takes the file's content");
        assert!(!crate::panes::has_comment_drafts(&app), "no draft waits for the new content");
        assert!(app.disk_notices.is_empty());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&app), "alpha", "Undo brings the old content back");
        assert!(app.session.doc.comments.values().any(|c| c.parent.is_some()), "with the typed reply in it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A parked document's reload leaves the shown document's comment draft alone: the person
    /// is still typing it.
    #[test]
    fn a_parked_documents_reload_leaves_the_shown_documents_draft_alone() {
        let (mut app, a_path) = app_with_file("parked-draft", "alpha");
        let a = app.session.document_id();
        let b_path = a_path.with_file_name("other.txt");
        std::fs::write(&b_path, "bravo").unwrap();
        app.run("file.open", json!({"path": b_path.to_string_lossy()})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let comment = app.run("review.newComment", json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
        crate::panes::draft_reply(&mut app, comment, "still typing");
        std::fs::write(&a_path, "alpha changed").unwrap();
        assert_eq!(app.check_disk_change(), vec![(a, DiskChange::Reloaded)]);
        assert!(crate::panes::has_comment_drafts(&app), "the draft is still being typed");
        assert_eq!(text(&app), "bravo");
        let _ = std::fs::remove_dir_all(a_path.parent().unwrap());
    }

    /// An agent's `file.reload` (the control channel runs commands with `execute`, not `run`)
    /// writes a comment draft into the content as it was first, like the bar's Reload: a reload
    /// without `force` then sees unsaved work, and a forced one keeps the reply one Undo away.
    #[test]
    fn an_agents_reload_writes_the_draft_into_the_version_one_undo_away() {
        let dir = temp_dir("agent-draft");
        let path = dir.join("notes.docx");
        let mut app = app();
        app.run("text.insert", json!({"text": "alpha"})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let comment = app.run("review.newComment", json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
        app.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        crate::panes::draft_reply(&mut app, comment, "typing a reply");
        let mut other = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        other.run("text.insert", &json!({"text": "theirs"})).unwrap();
        other.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        assert!(app.execute("file.reload", json!({})).is_err(), "the reply written in is unsaved work");
        assert_eq!(text(&app), "alpha");
        assert!(
            !crate::panes::has_comment_drafts(&app) && app.session.doc.comments.values().any(|c| c.parent.is_some()),
            "the draft is in the document"
        );
        app.execute("file.reload", json!({"force": true})).unwrap();
        assert_eq!(text(&app), "theirs");
        assert!(!crate::panes::has_comment_drafts(&app));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&app), "alpha");
        assert!(app.session.doc.comments.values().any(|c| c.parent.is_some()), "with the reply in it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Save refused because the file changed on disk: a document with nothing to lose is read
    /// again on the spot, not asked; one with unsaved work gets the bar. A save elsewhere is a
    /// copy, written as ever, and asks nothing.
    #[test]
    fn a_refused_save_reloads_a_clean_document_and_asks_about_a_dirty_one() {
        let (mut app, path) = app_with_file("refused", "alpha");
        std::fs::write(&path, "alpha beta").unwrap();
        assert!(app.execute_user("file.save", json!({})).is_err(), "refused: the file changed");
        assert!(app.disk_notices.is_empty(), "nothing to lose: no question");
        assert_eq!(text(&app), "alpha beta", "read again instead");
        app.run("text.insert", json!({"text": "X"})).unwrap();
        std::fs::write(&path, "alpha beta gamma").unwrap();
        let copy = path.with_file_name("copy.txt");
        app.execute_user("file.save", json!({"path": copy.to_string_lossy()})).unwrap();
        assert!(app.disk_notices.is_empty(), "a copy elsewhere asks nothing");
        assert!(app.execute_user("file.save", json!({})).is_err());
        assert_eq!(app.disk_notice(), Some(&DiskNotice::Unsaved), "unsaved work: the bar asks");
        assert_eq!(text(&app), "Xalpha beta", "nothing was lost");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// An agent's reload of a parked document (#5623) writes that document's own drafts into its
    /// content first, where the reload keeps them one Undo away: a reload without `force` then
    /// sees unsaved work, and none waits to be applied to the new content when the document is
    /// shown again.
    #[test]
    fn an_agents_reload_of_a_parked_document_handles_its_own_drafts() {
        let (mut app, a_path) = app_with_file("parked-agent", "alpha");
        let a = app.session.document_id();
        // Open flushes drafts (#1348): the second document opens first, and the draft is typed
        // after switching back.
        let b_path = a_path.with_file_name("other.txt");
        std::fs::write(&b_path, "bravo").unwrap();
        app.run("file.open", json!({"path": b_path.to_string_lossy()})).unwrap();
        let b = app.session.document_id();
        app.run("view.switchWindows", json!({"id": a})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let comment = app.run("review.newComment", json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
        app.run("file.save", json!({})).unwrap();
        crate::panes::draft_reply(&mut app, comment, "typing a reply");
        app.run("view.switchWindows", json!({"id": b})).unwrap();
        assert!(app.document_has_comment_drafts(a), "the draft is parked with its document");
        std::fs::write(&a_path, "alpha changed").unwrap();
        assert!(app.execute("file.reload", json!({"id": a})).is_err(), "the reply written in is unsaved work");
        assert!(!app.document_has_comment_drafts(a), "the draft is in the document now");
        app.execute("file.reload", json!({"id": a, "force": true})).unwrap();
        assert!(!app.document_has_comment_drafts(a));
        app.run("view.switchWindows", json!({"id": a})).unwrap();
        assert_eq!(text(&app), "alpha changed");
        assert!(!crate::panes::has_comment_drafts(&app), "nothing waits for the new content");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&app), "alpha");
        assert!(app.session.doc.comments.values().any(|c| c.parent.is_some()), "with the reply in it");
        let _ = std::fs::remove_dir_all(a_path.parent().unwrap());
    }
}
