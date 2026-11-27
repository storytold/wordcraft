//! Comment and revision balloons beside the pages: this file turns the document's comments and tracked changes into
//! the framework's `BalloonItem`s, shows `CommentBalloons` and applies the edits its cards report. The selected card
//! and its drafts are `CanvasState::comments`, the one copy the Comments pane shares.

use craft::review::ui::{BalloonItem, BalloonPlace, CommentBalloons, CommentEdit};
use craft::review::{CommentAnchor, CommentMessage, CommentThread, Revision, RevisionKind, ThreadId};

use super::geometry::{FLOAT_WIDTH, floating_comments, markup_width, shown_comment};
use super::{Color32, CommentDisplay, DocLayout, Env, Pos, Pos2, StoryRef, Ui, WordApp, json};

/// The most revision cards of each kind shown.
const MAX_REVISIONS: usize = 500;

/// What a card shows.
enum What {
    Thread(CommentThread),
    Revision(Revision),
}

/// A card with the page, anchor (page points) and document position it hangs from.
struct Card {
    page: usize,
    anchor: Pos2,
    pos: Pos,
    what: What,
}

struct BalloonItems {
    cards: Vec<Card>,
}

/// Comment balloons in the markup area right of each page, joined to their anchors, with each comment's replies in
/// its card. Clicking a card selects it, like Word: its text becomes editable in place, with Reply, Resolve and
/// Delete (the same commands as the Comments pane).
pub(crate) fn balloons(app: &mut WordApp, ui: &mut Ui, out: &craft::pages::ui::PageViewOutput, layout: &DocLayout) {
    app.canvas.balloon_rects.clear();
    let mw = markup_width(app);
    let float = floating_comments(app);
    if app.canvas.selected_balloon().is_some_and(|id| (mw <= 0.0 && !float) || !app.session.doc.comments.contains_key(&id)) {
        deselect_balloon(app);
    }
    if mw <= 0.0 && !float {
        return;
    }
    let only_shown = app.session.prefs.markup.comment_display == CommentDisplay::Selected;
    let shown = shown_comment(app);
    // Each author takes the next of the theme's categorical data colours.
    let palette: Vec<Color32> = Env::of(ui).tokens.color.data.series.iter().map(|c| Color32::from_rgb(c.0[0], c.0[1], c.0[2])).collect();
    let BalloonItems { cards } = collect_items(app, layout, only_shown, shown, &palette);
    let items: Vec<BalloonItem<'_>> = cards
        .iter()
        .map(|c| match &c.what {
            What::Thread(thread) => BalloonItem::thread(thread, c.page, c.anchor),
            What::Revision(revision) => BalloonItem::revision(revision, c.page, c.anchor),
        })
        .collect();
    let scale = out.pages.first().map_or(1.0, |p| p.scale as f32);
    let (place, width) = if mw > 0.0 { (BalloonPlace::Column, mw * scale) } else { (BalloonPlace::Floating, FLOAT_WIDTH * scale) };
    let editable = crate::app::command_board::command_enabled(app, "review.editComment");
    let shown =
        CommentBalloons::new(tl!("Comments"), &out.pages, &items, &mut app.canvas.comments).place(place).width(width).read_only(!editable).show(ui);
    app.canvas.balloon_rects = shown.cards.iter().filter_map(|(t, r)| Some((app_id(*t)?, *r))).collect();
    for edit in shown.edits {
        apply(app, &cards, edit);
    }
}

/// Applies what a card reported through the commands the Comments pane uses.
fn apply(app: &mut WordApp, cards: &[Card], edit: CommentEdit) {
    match edit {
        CommentEdit::Select(threads) => {
            let Some(thread) = threads.first().copied() else { return };
            let Some(id) = app_id(thread) else { return };
            focus_balloon(app, id);
            let pos = cards.iter().find_map(|c| match &c.what {
                What::Thread(t) if t.id == thread => Some(c.pos.clone()),
                _ => None,
            });
            if let Some(pos) = pos {
                app.session.set_selection(wordcraft_engine::Selection::caret(pos));
            }
        }
        CommentEdit::Edit { thread, index, text, .. } => {
            let Some(id) = app_id(thread).and_then(|t| message_id(app, t, index)) else { return };
            // An empty comment is deleted, as Word does.
            let _ = if text.trim().is_empty() {
                app.run("review.deleteComment", json!({"id": id}))
            } else {
                app.run("review.editComment", json!({"id": id, "text": text}))
            };
        }
        CommentEdit::Reply { thread, text, .. } => {
            let Some(id) = app_id(thread) else { return };
            if !text.trim().is_empty() {
                let _ = app.run("review.reply", json!({"id": id, "text": text.trim_end()}));
            }
        }
        CommentEdit::Resolve { thread, .. } => {
            if let Some(id) = app_id(thread) {
                let _ = app.run("review.resolveComment", json!({"id": id}));
            }
        }
        CommentEdit::Delete { thread, index, .. } => {
            let Some(id) = app_id(thread).and_then(|t| message_id(app, t, index)) else { return };
            if index == 0 {
                deselect_balloon(app);
            }
            let _ = app.run("review.deleteComment", json!({"id": id}));
        }
        // The balloon cards report no other edit.
        _ => {}
    }
}

fn thread_id(id: u32) -> ThreadId {
    ThreadId(u64::from(id))
}

fn app_id(thread: ThreadId) -> Option<u32> {
    u32::try_from(thread.0).ok()
}

/// The ids of a thread's messages in card order: the comment, then its replies by id.
fn message_ids(app: &WordApp, id: u32) -> Vec<u32> {
    let mut replies: Vec<u32> = app.session.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k).collect();
    replies.sort_unstable();
    std::iter::once(id).chain(replies).collect()
}

/// The comment id of message `index` of thread `id`.
fn message_id(app: &WordApp, id: u32, index: usize) -> Option<u32> {
    message_ids(app, id).get(index).copied()
}

/// Seconds since the epoch of a comment's ISO 8601 date; a date without a zone is read as UTC, an unreadable one is 0.
fn unix_seconds(date: &str) -> i64 {
    use chrono::{DateTime, NaiveDateTime};
    DateTime::parse_from_rfc3339(date)
        .map(|d| d.timestamp())
        .or_else(|_| NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S").map(|d| d.and_utc().timestamp()))
        .unwrap_or(0)
}

/// The thread of comment `id`: the comment and its replies.
fn thread_of(app: &WordApp, id: u32, page: usize, anchor: Pos2, color: Color32) -> Option<CommentThread> {
    let root = app.session.doc.comments.get(&id)?;
    let messages = message_ids(app, id)
        .into_iter()
        .filter_map(|cid| {
            let c = app.session.doc.comments.get(&cid)?;
            let created = c.date_utc.as_deref().map(unix_seconds).filter(|t| *t > 0).unwrap_or_else(|| unix_seconds(&c.date));
            let text = app.session.doc.plain_text(StoryRef::Part(c.part)).trim().to_string();
            Some(CommentMessage { author: author_name(&c.author), created, modified: None, text, reactions: Vec::new(), read: true, deletable: true })
        })
        .collect();
    Some(CommentThread {
        id: thread_id(id),
        anchor: CommentAnchor::Page { page, at: [f64::from(anchor.x), f64::from(anchor.y)] },
        kind: None,
        color: Some([color.r(), color.g(), color.b()]),
        messages,
        statuses: Vec::new(),
        reply_statuses: Vec::new(),
        marked: false,
        locked: false,
        resolved: root.resolved,
    })
}

/// The cards of the comments and tracked changes the markup preferences show: replies hang under their parent,
/// `Selected` shows only the shown comment, and formatting, moves and deletions follow the markup preferences.
fn collect_items(app: &WordApp, layout: &DocLayout, only_shown: bool, shown: Option<u32>, palette: &[Color32]) -> BalloonItems {
    let comments_by_id = &app.session.doc.comments;
    // Replies hang under the comment they answer rather than getting balloons of their own.
    let is_reply = |comment: &wordcraft_doc::Comment| comment.parent.is_some_and(|parent| comments_by_id.contains_key(&parent));
    let mut cards: Vec<Card> = Vec::new();
    let mut authors: Vec<String> = Vec::new();
    let mut color_of = |author: &str| {
        let ai = authors.iter().position(|a| a == author).unwrap_or_else(|| {
            authors.push(author.to_string());
            authors.len() - 1
        });
        palette.get(ai % palette.len().max(1)).copied().unwrap_or(Color32::GRAY)
    };
    let markup = &app.session.prefs.markup;
    let comments = if markup.comments { wordcraft_engine::cmd::review::comment_list(&app.session) } else { Vec::new() };
    for (id, pos) in comments {
        let Some(pos) = pos else { continue };
        let Some(comment) = comments_by_id.get(&id) else { continue };
        if is_reply(comment) || (only_shown && shown != Some(id)) {
            continue;
        }
        let Some(anchor) = anchor_of(layout, app.session.page_hint, &pos) else { continue };
        let color = color_of(&author_name(&comment.author));
        if let Some(thread) = thread_of(app, id, anchor.0, anchor.1, color) {
            cards.push(Card { page: anchor.0, anchor: anchor.1, pos, what: What::Thread(thread) });
        }
    }
    let revision = |cards: &mut Vec<Card>, pos: Pos, kind: RevisionKind, author: String, text: String| {
        if let Some((page, anchor)) = anchor_of(layout, app.session.page_hint, &pos) {
            let revision = Revision { kind, heading: author_name(&author), details: vec![text] };
            cards.push(Card { page, anchor, pos, what: What::Revision(revision) });
        }
    };
    if markup.formatting {
        for (pos, author, props) in wordcraft_engine::cmd::review::format_changes(&app.session).into_iter().take(MAX_REVISIONS) {
            let what = crate::panes::describe_props(&props).join(", ");
            let text = if what.is_empty() { tl!("Formatted").to_string() } else { format!("{}: {what}", tl!("Formatted")) };
            revision(&mut cards, pos, RevisionKind::Changed, author, text);
        }
    }
    if markup.insertions_deletions {
        for (pos, author, from) in wordcraft_engine::cmd::review::move_changes(&app.session).into_iter().take(MAX_REVISIONS) {
            let (kind, text) = if from { (RevisionKind::Deleted, tl!("Moved from")) } else { (RevisionKind::Inserted, tl!("Moved to")) };
            revision(&mut cards, pos, kind, author, text.to_string());
        }
    }
    if app.session.revisions_in_balloons() {
        for deletion in wordcraft_engine::cmd::review::deletion_balloons(&app.session) {
            let text = format!("{}: {}", tl!("Deleted"), deletion.text);
            revision(&mut cards, deletion.at, RevisionKind::Deleted, deletion.author, text);
        }
    }
    BalloonItems { cards }
}

/// The page and the point (page points) a card at `pos` hangs from: the bottom of the commented line.
fn anchor_of(layout: &DocLayout, page_hint: usize, pos: &Pos) -> Option<(usize, Pos2)> {
    let caret = layout.caret_on(pos, page_hint)?;
    Some((caret.page, Pos2::new(caret.x, caret.top + caret.height)))
}

fn author_name(a: &str) -> String {
    if a.is_empty() { tl!("Author").to_string() } else { a.to_string() }
}

/// Writes back the drafts of the thread of comment `id`: each message's edit and the reply.
fn write_back(app: &mut WordApp, id: u32) {
    for c in message_ids(app, id) {
        // A refusal is on the status line; the draft stays for when the balloon is selected again.
        let _ = crate::panes::commit_comment(app, c);
    }
    let _ = crate::panes::commit_reply(app, id);
}

/// Makes comment `id` the selected card with its own text open for editing, writing back the draft of the one edited
/// before. The card selects itself on a click; this opens its edit so the field has the keyboard.
fn focus_balloon(app: &mut WordApp, id: u32) {
    let before = app.canvas.comments.editing.as_ref().and_then(|(t, ..)| app_id(*t)).filter(|b| *b != id);
    if let Some(before) = before {
        write_back(app, before);
    }
    // Where comments can't change (a read-only document) the card is selected with no edit open: nothing is typed.
    let editable = crate::app::command_board::command_enabled(app, "review.editComment");
    let text = app.session.doc.comments.get(&id).filter(|_| editable).map(|c| app.session.doc.plain_text(StoryRef::Part(c.part)));
    let thread = thread_id(id);
    let comments = &mut app.canvas.comments;
    comments.selected = craft::behavior::Selection::single(thread);
    if comments.editing.as_ref().is_none_or(|(t, ..)| *t != thread) {
        comments.editing = text.map(|t| (thread, 0, t));
    }
}

/// Select a comment's balloon (its text gets the keyboard), writing back any edit in the one selected before.
pub fn select_balloon(app: &mut WordApp, id: u32) {
    if app.canvas.selected_balloon() == Some(id) {
        return;
    }
    deselect_balloon(app);
    focus_balloon(app, id);
}

/// Leave the selected balloon, writing back what was typed in it (its fields may not be drawn again to notice they
/// lost focus).
pub fn deselect_balloon(app: &mut WordApp) {
    let Some(id) = app.canvas.selected_balloon() else { return };
    // What was written is cleared by the write; a draft the document refused (read-only protection) stays, with its
    // reason on the status line, for when the balloon is selected again or the document is saved or closed (#1348).
    write_back(app, id);
    app.canvas.comments.selected = craft::behavior::Selection::default();
}
