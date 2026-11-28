use super::{CommentDisplay, WordApp};

/// Width of the card that floats beside a page when comments show only for the selected text (points).
pub(super) const FLOAT_WIDTH: f32 = 216.0;

/// Comments show as a floating card rather than in a margin column (#1055): the option is on,
/// balloons are shown at all, and no other markup needs the column.
pub(super) fn floating_comments(app: &WordApp) -> bool {
    let s = &app.session;
    let m = &s.prefs.markup;
    s.balloons_shown() && m.comments && m.comment_display == CommentDisplay::Selected && !s.doc.comments.is_empty() && markup_width(app) <= 0.0
}

/// The comment shown when only the selected one is: the clicked balloon, else the comment whose
/// range holds the caret.
pub(super) fn shown_comment(app: &WordApp) -> Option<u32> {
    app.canvas.selected_balloon().or_else(|| wordcraft_engine::cmd::review::comment_range_at_caret(&app.session))
}

/// Width of the markup area beside each page for comment and revision balloons (points), or 0.
pub fn markup_width(app: &WordApp) -> f32 {
    // Word shows comments either in balloons (contextual) or in the Comments pane (list); Track
    // Changes Options can put every revision inline (`Session::balloons_shown`).
    let s = &app.session;
    if !s.balloons_shown() {
        return 0.0;
    }
    let m = &s.prefs.markup;
    let comments = m.comments && !s.doc.comments.is_empty() && m.comment_display == CommentDisplay::Margin;
    let [has_formats, has_moves, has_deletions] = match app.canvas.markup_facts.get() {
        Some((rev, facts)) if rev == (s.document_id(), s.rev()) => facts,
        _ => {
            use wordcraft_engine::cmd::{moves, review};
            let facts = [review::has_format_changes(&s.doc), moves::has_moves(&s.doc), review::has_deletions(&s.doc)];
            app.canvas.markup_facts.set(Some(((s.document_id(), s.rev()), facts)));
            facts
        }
    };
    let formats = m.formatting && has_formats;
    let moves = m.insertions_deletions && has_moves;
    let deletions = s.revisions_in_balloons() && has_deletions;
    if s.balloons_shown() && (comments || formats || moves || deletions) { 216.0 } else { 0.0 }
}

/// While a fit mode (Page Width, One Page, Multiple Pages) sizes the page, keep the session's zoom
/// equal to what is shown, as Word does. Zoom In/Out, the Zoom dialog and `view.state` then start
/// from the visible zoom; before, they stepped from the stale manual zoom, so Zoom In from a 163%
/// Page Width jumped to 110% (issue #67). `zoom` is the page view's shown zoom (1.0 = 100%).
pub fn sync_fit_zoom(app: &mut WordApp, zoom: f64) {
    let v = &mut app.session.view;
    if v.fit.is_empty() || v.read_mode || v.mode != wordcraft_layout::ViewMode::Print || !zoom.is_finite() {
        return;
    }
    v.zoom = zoom.clamp(0.1, 5.0) as f32;
}
