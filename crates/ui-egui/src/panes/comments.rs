use super::*;

/// Draws the Comments pane: New Comment and scrollable comment and reply cards with edit, go-to,
/// reply, resolve or reopen, and delete actions.
pub(super) fn comments(app: &mut WordApp, ui: &mut Ui) {
    if crate::widgets::icon_text_button(ui, "newComment", tl!("New Comment"), regular(12.5)).clicked() {
        let _ = app.run("review.newComment", json!({"text": ""}));
    }
    ui.separator();
    let list = app.session.run("review.comments", &json!({})).unwrap_or_default();
    let t = Tokens::get(ui.ctx());
    // Comment ids are the document's own; another window's document has the same ones.
    let document = app.session.document_id();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for c in list.as_array().cloned().unwrap_or_default() {
            let id = c.get("id").and_then(Value::as_u64).unwrap_or(0);
            let reply = c.get("parent").is_some_and(|p| !p.is_null());
            let when = u32::try_from(id).ok().and_then(|i| app.session.doc.comments.get(&i)).map(crate::local_time::comment_time).unwrap_or_default();
            egui::Frame::NONE.fill(t.input).stroke(Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if reply {
                    ui.add_space(0.0);
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(c.get("author").and_then(Value::as_str).unwrap_or("")).font(semibold(12.0)));
                    ui.label(egui::RichText::new(when).small().weak());
                    if c.get("resolved").and_then(Value::as_bool).unwrap_or(false) {
                        ui.label(egui::RichText::new(tl!("Resolved")).small().color(t.green));
                    }
                });
                // Editable comment text (one undo step per edit, like the balloons).
                comment_editor(app, ui, id as u32, None, false);
                let open_key = egui::Id::unique(("comment_reply_open", document, id));
                let mut replying = ui.data(|d| d.get_temp::<bool>(open_key)).unwrap_or(false);
                ui.horizontal(|ui| {
                    if ui.small_button(tl!("Go to")).clicked()
                        && let Some(a) = c.get("anchor").filter(|a| !a.is_null())
                    {
                        let _ = app.run("caret.set", json!({"pos": a}));
                    }
                    if ui.small_button(tl!("Reply")).clicked() {
                        replying = true;
                        ui.data_mut(|d| d.insert_temp(open_key, true));
                        ui.data_mut(|d| d.insert_temp(egui::Id::unique(("comment_reply_focus", document, id)), true));
                    }
                    let resolved = c.get("resolved").and_then(Value::as_bool).unwrap_or(false);
                    if ui.small_button(if resolved { tl!("Reopen") } else { tl!("Resolve") }).clicked() {
                        let _ = app.run("review.resolveComment", json!({"id": id}));
                    }
                    if ui.small_button(tl!("Delete")).clicked() {
                        let _ = app.run("review.deleteComment", json!({"id": id}));
                    }
                });
                if replying {
                    let focus = ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(("comment_reply_focus", document, id)))).unwrap_or(false);
                    if reply_editor(app, ui, id as u32, None, focus) {
                        ui.data_mut(|d| d.remove::<bool>(open_key));
                    }
                }
            });
            ui.add_space(6.0);
        }
    });
}

/// What has been typed in comment and reply fields but not yet written to the document (#1348).
/// Drafts belong to one document ([`Session::document_id`](wordcraft_engine::Session::document_id)):
/// when another document replaces it they are dropped, so a comment there with the same id never
/// shows them. A draft is removed only once its command succeeds; a refused edit keeps it.
#[derive(Default)]
pub(crate) struct CommentDrafts {
    document: u64,
    edits: BTreeMap<u32, String>,
    replies: BTreeMap<u32, String>,
}

impl CommentDrafts {
    /// The drafts for `document`, dropping any left from another one.
    fn of(&mut self, document: u64) -> &mut Self {
        if self.document != document {
            self.edits.clear();
            self.replies.clear();
            self.document = document;
        }
        self
    }

    /// Nothing typed and not yet written.
    pub(crate) fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.replies.is_empty()
    }
}

/// The pending draft of comment `id`'s text in the open document, if any.
#[cfg(test)]
pub(crate) fn comment_draft(app: &WordApp, id: u32) -> Option<String> {
    let d = &app.comment_drafts;
    if d.document == app.session.document_id() { d.edits.get(&id).cloned() } else { None }
}

/// Whether the open document has a comment or reply draft not yet written to it (typed, or
/// kept after the document refused it): closing the page would lose it.
pub(crate) fn has_comment_drafts(app: &WordApp) -> bool {
    let d = &app.comment_drafts;
    d.document == app.session.document_id() && !(d.edits.is_empty() && d.replies.is_empty())
}

fn drafts(app: &mut WordApp) -> &mut CommentDrafts {
    let document = app.session.document_id();
    app.comment_drafts.of(document)
}

/// A comment's text, editable in place (the Comments pane and the balloons). Typing stays in a
/// draft until the field loses focus; then [`commit_comment`] writes it back with one
/// `review.editComment`, so each editing session is one undo step.
pub(crate) fn comment_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) {
    let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part) else { return };
    let draft = drafts(app).edits.get(&id).cloned();
    let mut text = draft.unwrap_or_else(|| app.session.doc.plain_text(StoryRef::Part(part)));
    // Read-only where comments can't change (a protected document), so nothing is typed in vain.
    let editable = crate::widgets::enabled(app, "review.editComment");
    let mut edit = egui::TextEdit::multiline(&mut text)
        .id(field_id(app, ui, "comment_edit", id))
        .interactive(editable)
        .desired_rows(1)
        .desired_width(f32::INFINITY)
        .hint_text(tl!("Add a comment"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        drafts(app).edits.insert(id, text);
    }
    if r.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Escape throws the draft away: the field shows the comment's text again.
            drafts(app).edits.remove(&id);
        } else {
            // A refusal is on the status line, and the draft stays in the field.
            let _ = commit_comment(app, id);
        }
    }
}

/// A comment field's widget id, per document: egui's own state for the field (its cursor and
/// undo history) never carries over to another document's comment with the same id.
fn field_id(app: &WordApp, ui: &Ui, kind: &str, id: u32) -> egui::Id {
    ui.scope_id().with((kind, app.session.document_id(), id))
}

/// Write a comment's draft text back, when it has one that differs from the comment. The draft
/// is dropped once written (or when there is nothing to write); a refused edit keeps it.
pub(crate) fn commit_comment(app: &mut WordApp, id: u32) -> Result<(), String> {
    let Some(text) = drafts(app).edits.get(&id).cloned() else { return Ok(()) };
    if let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part)
        && text != app.session.doc.plain_text(StoryRef::Part(part))
    {
        app.run("review.editComment", json!({"id": id, "text": text}))?;
    }
    drafts(app).edits.remove(&id);
    Ok(())
}

/// The comment a reply to `id` answers: replies to a reply join its thread.
fn thread_root(app: &WordApp, id: u32) -> u32 {
    match app.session.doc.comments.get(&id).and_then(|c| c.parent) {
        Some(p) if app.session.doc.comments.contains_key(&p) => p,
        _ => id,
    }
}

/// A field for replying to a comment. When it loses focus a non-empty reply is posted with
/// `review.reply`; returns true when the field is done (posted or left empty). A refused reply
/// keeps the field open with its text.
pub(crate) fn reply_editor(app: &mut WordApp, ui: &mut Ui, id: u32, font: Option<egui::FontId>, focus: bool) -> bool {
    let mut text = drafts(app).replies.get(&id).cloned().unwrap_or_default();
    let editable = crate::widgets::enabled(app, "review.reply");
    let mut edit = egui::TextEdit::multiline(&mut text)
        .id(field_id(app, ui, "comment_reply", id))
        .interactive(editable)
        .desired_rows(1)
        .desired_width(f32::INFINITY)
        .hint_text(tl!("Write a reply"));
    if let Some(f) = font {
        edit = edit.font(f);
    }
    let r = ui.add(edit);
    if focus {
        r.request_focus();
    }
    if r.changed() {
        drafts(app).replies.insert(id, text);
    }
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        // Escape throws the reply away and closes the field.
        drafts(app).replies.remove(&id);
        return true;
    }
    r.lost_focus() && commit_reply(app, id).is_ok()
}

/// Post a pending reply draft to comment `id` (nothing when there is none or it is blank). The
/// draft is dropped once posted; a refused reply keeps it.
pub(crate) fn commit_reply(app: &mut WordApp, id: u32) -> Result<(), String> {
    let Some(text) = drafts(app).replies.get(&id).cloned() else { return Ok(()) };
    if !text.trim().is_empty() && app.session.doc.comments.contains_key(&id) {
        let root = thread_root(app, id);
        app.run("review.reply", json!({"id": root, "text": text.trim_end()}))?;
    }
    drafts(app).replies.remove(&id);
    Ok(())
}

/// Write every pending comment and reply draft to the document, so a save has them and a close,
/// New or Open sees them as unsaved changes (#1348). Every draft is tried; the first refusal is
/// returned (and is on the status line), and refused drafts stay.
pub(crate) fn flush_comment_drafts(app: &mut WordApp) -> Result<(), String> {
    let (edits, replies): (Vec<u32>, Vec<u32>) = {
        let d = drafts(app);
        (d.edits.keys().copied().collect(), d.replies.keys().copied().collect())
    };
    let mut first = Ok(());
    for id in edits {
        let r = commit_comment(app, id);
        if first.is_ok() {
            first = r;
        }
    }
    for id in replies {
        let r = commit_reply(app, id);
        if first.is_ok() {
            first = r;
        }
    }
    first
}
