use craft::behavior::{EditEvent, IconId, Selection};
use craft::review::ThreadId;
use craft::review::ui::CommentsPanelState;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props, Show, ShowContainer};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::sections::GroupBox;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};
use wordcraft_doc::StoryRef;

use super::WordApp;

/// Draws the Comments pane: New Comment and comment and reply cards with edit, go-to, reply, resolve or
/// reopen, and delete actions.
pub(super) fn comments(app: &mut WordApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if Button::new(tl!("New Comment").to_owned()).icon(IconSpec::svg(IconId::new("newComment"))).show(ui).clicked() {
        let _ = app.run("review.newComment", json!({"text": ""}));
    }
    let list = app.session.run("review.comments", &json!({})).unwrap_or_default();
    // Comment ids are the document's own; another window's document has the same ones.
    let document = app.session.document_id();
    Stack::new().fill_width().show(ui, |ui, _| {
        for c in list.as_array().cloned().unwrap_or_default() {
            let id = c.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()).unwrap_or(0);
            let author = c.get("author").and_then(Value::as_str).unwrap_or("").to_owned();
            GroupBox::new(author, None).show(ui, |ui, _| card(app, ui, &ctx, document, id, &c));
        }
    });
}

/// One comment's card: its time and state, its text, and its actions.
fn card(app: &mut WordApp, ui: &mut Ui, ctx: &egui::Context, document: u64, id: u32, c: &Value) {
    let when = app.session.doc.comments.get(&id).map(crate::local_time::comment_time).unwrap_or_default();
    let resolved = c.get("resolved").and_then(Value::as_bool).unwrap_or(false);
    Row::new().wrap(true).show(ui, |ui, _| {
        Text::new(when.clone()).show(ui);
        if resolved {
            Text::new(tl!("Resolved").to_owned()).show(ui);
        }
    });
    // Editable comment text (one undo step per edit, like the balloons).
    comment_field(app, ui, document, id);
    let open_key = egui::Id::unique(("comment_reply_open", document, id));
    let mut replying = ui.data(|d| d.get_temp::<bool>(open_key)).unwrap_or(false);
    Row::new().wrap(true).show(ui, |ui, _| {
        if Button::new(tl!("Go to").to_owned()).show(ui).clicked()
            && let Some(a) = c.get("anchor").filter(|a| !a.is_null())
        {
            let _ = app.run("caret.set", json!({"pos": a}));
        }
        if Button::new(tl!("Reply").to_owned()).show(ui).clicked() {
            replying = true;
            ui.data_mut(|d| d.insert_temp(open_key, true));
            // The field is shown later in this pass, where the focus service applies the request.
            focus::request(ctx, FocusTarget::Id(reply_id(document, id)));
        }
        if Button::new(if resolved { tl!("Reopen") } else { tl!("Resolve") }.to_owned()).show(ui).clicked() {
            let _ = app.run("review.resolveComment", json!({"id": id}));
        }
        if Button::new(tl!("Delete").to_owned()).show(ui).clicked() {
            let _ = app.run("review.deleteComment", json!({"id": id}));
        }
    });
    if replying && reply_field(app, ui, document, id) {
        ui.data_mut(|d| d.remove::<bool>(open_key));
    }
}

/// What has been typed in the pane's comment and reply fields but not yet written to the document (#1348).
/// The state is the canvas's: the balloons draft in it too, and a document switch parks it with the document.
fn drafts(app: &mut WordApp) -> &mut CommentsPanelState {
    &mut app.canvas.comments
}

/// The comment thread a message of `id` belongs to, as the panel state names it.
fn thread_of(app: &WordApp, id: u32) -> ThreadId {
    ThreadId(u64::from(thread_root(app, id)))
}

/// The place of comment `id` in its thread: 0 for the comment, then its replies in id order.
fn message_index(app: &WordApp, id: u32) -> usize {
    let root = thread_root(app, id);
    if root == id {
        return 0;
    }
    replies(app, root).iter().position(|r| *r == id).map_or(0, |at| at + 1)
}

/// The comment at message `index` of the thread rooted at `root`.
fn message_id(app: &WordApp, root: u32, index: usize) -> Option<u32> {
    match index {
        0 => Some(root),
        _ => replies(app, root).get(index - 1).copied(),
    }
}

/// The ids of the replies to `root`, in id order.
fn replies(app: &WordApp, root: u32) -> Vec<u32> {
    let mut replies: Vec<u32> = app.session.doc.comments.iter().filter(|(_, c)| c.parent == Some(root)).map(|(id, _)| *id).collect();
    replies.sort_unstable();
    replies
}

/// The pending draft of comment `id`'s text in the open document, if any.
fn draft_of(app: &WordApp, id: u32) -> Option<String> {
    let key = (thread_of(app, id).0, message_index(app, id));
    app.canvas.comments.editing.as_ref().filter(|(t, i, _)| (t.0, *i) == key).map(|(_, _, text)| text.clone())
}

/// Whether the open document has a comment or reply draft not yet written to it (typed, or
/// kept after the document refused it): closing the page would lose it.
pub(crate) fn has_comment_drafts(app: &WordApp) -> bool {
    state_has_drafts(&app.canvas.comments)
}

/// Whether `state` holds a comment or reply draft: the open document's, or a parked document's.
pub(crate) fn state_has_drafts(state: &CommentsPanelState) -> bool {
    state.editing.is_some() || !state.reply.is_empty()
}

/// Lines of a comment's text that its card or balloon shows; longer text scrolls in its field and
/// is cut off in a balloon that isn't selected (#3382).
pub(crate) const COMMENT_ROWS: usize = 10;

/// A comment's text, editable in place. Typing stays in a draft until the field's value lands (focus leaves it);
/// then [`commit_comment`] writes it back with one `review.editComment`, so each editing session is one undo step.
/// Escape gives back the text from before, so the draft goes.
fn comment_field(app: &mut WordApp, ui: &mut Ui, document: u64, id: u32) {
    let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part) else { return };
    let (thread, index) = (thread_of(app, id), message_index(app, id));
    let shown = draft_of(app, id).unwrap_or_else(|| app.session.doc.plain_text(StoryRef::Part(part)));
    // Read-only where comments can't change (a protected document), so nothing is typed in vain.
    let editable = crate::app::command_board::command_enabled(app, "review.editComment");
    let field = ui.make_persistent_id(("comment_edit", document, id));
    let out = TextField::new(tl!("Comment").to_owned(), Bind::given(shown))
        .id(field)
        .lines(1)
        .max_lines(COMMENT_ROWS)
        .placeholder(tl!("Add a comment").to_owned())
        .enabled(editable)
        .show(ui);
    for event in out.events.iter() {
        match event {
            EditEvent::Update { value, .. } => drafts(app).editing = Some((thread, index, value.clone())),
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => {
                drafts(app).editing = Some((thread, index, after.clone()));
                // A refusal is on the status line, and the draft stays.
                let _ = commit_comment(app, id);
            }
            EditEvent::Cancel { .. } if draft_of(app, id).is_some() => {
                drafts(app).editing = None;
            }
            _ => {}
        }
    }
}

/// Write a comment's draft text back, when it has one that differs from the comment. The draft
/// is dropped once written (or when there is nothing to write); a refused edit keeps it.
pub(crate) fn commit_comment(app: &mut WordApp, id: u32) -> Result<(), String> {
    let Some(text) = draft_of(app, id) else { return Ok(()) };
    if let Some(part) = app.session.doc.comments.get(&id).map(|c| c.part)
        && text != app.session.doc.plain_text(StoryRef::Part(part))
    {
        app.run("review.editComment", json!({"id": id, "text": text}))?;
    }
    drafts(app).editing = None;
    Ok(())
}

/// The comment a reply to `id` answers: replies to a reply join its thread.
fn thread_root(app: &WordApp, id: u32) -> u32 {
    match app.session.doc.comments.get(&id).and_then(|c| c.parent) {
        Some(p) if app.session.doc.comments.contains_key(&p) => p,
        _ => id,
    }
}

/// The id of comment `id`'s reply field, so the Reply button can ask for its focus.
fn reply_id(document: u64, id: u32) -> egui::Id {
    egui::Id::unique(("comment_reply", document, id))
}

/// A field for replying to a comment. When its value lands a non-empty reply is posted with
/// `review.reply`; returns true when the field is done (posted, left empty or abandoned). A refused reply
/// keeps the field open with its text.
fn reply_field(app: &mut WordApp, ui: &mut Ui, document: u64, id: u32) -> bool {
    let thread = thread_of(app, id);
    let selected = app.canvas.comments.selected.lead.as_ref().is_some_and(|t| t.0 == thread.0);
    let shown = if selected { app.canvas.comments.reply.clone() } else { String::new() };
    let editable = crate::app::command_board::command_enabled(app, "review.reply");
    let field = reply_id(document, id);
    let out = TextField::new(tl!("Reply").to_owned(), Bind::given(shown))
        .id(field)
        .lines(1)
        .max_lines(COMMENT_ROWS)
        .placeholder(tl!("Write a reply").to_owned())
        .enabled(editable)
        .show(ui);
    let mut done = false;
    for event in out.events.iter() {
        match event {
            EditEvent::Update { value, .. } => {
                let d = drafts(app);
                d.selected = Selection::single(thread);
                d.reply = value.clone();
            }
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => {
                let d = drafts(app);
                d.selected = Selection::single(thread);
                d.reply = after.clone();
                done = commit_reply(app, id).is_ok();
            }
            EditEvent::Cancel { .. } => {
                // Escape throws the reply away and closes the field.
                if selected {
                    drafts(app).reply.clear();
                }
                done = true;
            }
            _ => {}
        }
    }
    done
}

/// Post a pending reply draft to comment `id` (nothing when there is none or it is blank). The
/// draft is dropped once posted; a refused reply keeps it.
pub(crate) fn commit_reply(app: &mut WordApp, id: u32) -> Result<(), String> {
    let thread = thread_of(app, id);
    let d = drafts(app);
    if !d.selected.lead.as_ref().is_some_and(|t| t.0 == thread.0) {
        return Ok(());
    }
    let text = d.reply.clone();
    if !text.trim().is_empty() && app.session.doc.comments.contains_key(&id) {
        let root = thread_root(app, id);
        app.run("review.reply", json!({"id": root, "text": text.trim_end()}))?;
    }
    drafts(app).reply.clear();
    Ok(())
}

/// Write every pending comment and reply draft to the document, so a save has them and a close,
/// New or Open sees them as unsaved changes (#1348). Every draft is tried; the first refusal is
/// returned (and is on the status line), and refused drafts stay.
pub(crate) fn flush_comment_drafts(app: &mut WordApp) -> Result<(), String> {
    let mut first = Ok(());
    if let Some((thread, index, _)) = drafts(app).editing.clone() {
        let id = u32::try_from(thread.0).ok().and_then(|root| message_id(app, root, index));
        let r = match id {
            Some(id) => commit_comment(app, id),
            // The comment is gone: its draft has nowhere to go.
            None => {
                drafts(app).editing = None;
                Ok(())
            }
        };
        if first.is_ok() {
            first = r;
        }
    }
    let selected = drafts(app).selected.lead.as_ref().and_then(|t| u32::try_from(t.0).ok());
    if let Some(id) = selected {
        let r = commit_reply(app, id);
        if first.is_ok() {
            first = r;
        }
    }
    first
}
