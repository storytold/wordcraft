use super::{ImeEditOutcome, WordApp, json};

/// The most pre-edit text a UI pass will hold and paint. A composition is a word or a phrase;
/// anything larger is a malformed or hostile event, and laying it out every frame would cost.
pub(super) const MAX_IME_PREEDIT_BYTES: usize = wordcraft_doc::ime_text::MAX_IME_TEXT_BYTES;

/// The largest IME commit inserted in one event. Real commits are short; a multi-megabyte one is
/// refused rather than turned into document text without a bound.
pub(super) const MAX_IME_COMMIT_BYTES: usize = 64 * 1024;

/// The bounded visible-logical window around the body selection, cached by `(revision, cursor,
/// anchor)` so an idle pass does not walk a long paragraph again. Every edit bumps `rev`.
fn ime_window(app: &mut WordApp) -> Option<wordcraft_doc::ime_text::ImeWindow> {
    let rev = app.session.rev();
    let cursor = app.session.sel.focus.clone();
    let anchor = app.session.sel.anchor.clone();
    if let Some((r, c, a, w)) = &app.canvas.ime_window_cache
        && *r == rev
        && *c == cursor
        && *a == anchor
    {
        return w.clone();
    }
    let window = app.session.doc.ime_surrounding(&cursor, &anchor, wordcraft_doc::ime_text::MAX_IME_TEXT_BYTES);
    app.canvas.ime_window_cache = Some((rev, cursor, anchor, window.clone()));
    window
}

/// Observe the settled editor at the end of a UI pass and publish its IME descriptor.
///
/// The tracker is scoped by native viewport. A closed viewport releases its state; a pass without
/// a visible focused editor reports an unfocused observation so a later focus starts afresh.
pub(crate) fn observe_ime(app: &mut WordApp, ctx: &egui::Context, geometry: Option<(egui::Rect, egui::Rect)>, interrupt: bool) {
    let viewport = ctx.viewport_id();
    // Drain an application-origin change before the final observation. The signal survives later
    // commands until here (or until a receipt publishes), so an edit after the receipt still
    // makes the frame `Other`.
    if app.session.take_pending_ime_application_change() {
        note_ime_application_change(app);
    }
    if app.canvas.ime_viewport != Some(viewport) {
        if let Some(old) = app.canvas.ime_viewport {
            app.canvas.ime_tracker.end_viewport(old);
        }
        app.canvas.ime_viewport = Some(viewport);
    }
    let frame = ctx.cumulative_frame_nr_for(viewport);
    let pass = ctx.cumulative_pass_nr_for(viewport);
    let focused = app.canvas.focused && geometry.is_some();
    let update = if focused {
        let target = ime_target(app);
        if app.session.math.is_some() {
            // An equation's structured tree has no reversible text mapping yet: report an
            // explicit Unavailable, never fabricated surroundings. Its stable session token is
            // the replacement epoch, so caret motion keeps one context. The engine revision lets
            // an unavailable receipt prove a committed edit.
            let mut observation = egui::ImeObservation::new(frame, pass, target);
            if let Some(rev) = app.session.ime_state_revision() {
                observation = observation.unavailable_revision(rev);
            }
            app.canvas.ime_tracker.observe(viewport, Some(observation))
        } else {
            let window = ime_window(app);
            if let Some(w) = &window {
                let committed = Some(egui::CommittedText::new(&w.text, w.cursor, w.anchor));
                app.canvas.ime_tracker.observe(viewport, Some(egui::ImeObservation::new(frame, pass, target).committed(committed)))
            } else {
                // A wide selection or cross-story caret has no single surrounding window. Its
                // engine revision orders unavailable observations; no text is fabricated.
                let mut observation = egui::ImeObservation::new(frame, pass, target);
                if let Some(rev) = app.session.ime_state_revision() {
                    observation = observation.unavailable_revision(rev);
                }
                app.canvas.ime_tracker.observe(viewport, Some(observation))
            }
        }
    } else {
        // No editor has the keyboard: a pre-edit painted from the old focus must not linger.
        app.canvas.ime_preedit.clear();
        app.canvas.ime_preedit_range = None;
        app.canvas.ime_tracker.observe(viewport, None)
    };
    if let (Some(update), Some((rect, cursor_rect))) = (update, geometry) {
        ctx.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                owner: update.owner,
                context_generation: update.context_generation,
                purpose: update.purpose,
                rect,
                cursor_rect,
                should_interrupt_composition: interrupt,
                reset_request: None,
                change_cause: update.change_cause,
                surrounding_text: update.surrounding_text,
            });
        });
    }
}

/// The immutable IME owner for the editor that currently has the keyboard.
///
/// The document identity and story are stable across edits; an equation adds its editing-session
/// token as the replacement epoch because its document position moves with the text.
fn ime_target(app: &WordApp) -> egui::ImeTarget {
    let document = app.session.document_id();
    if let Some(math) = &app.session.math {
        egui::ImeTarget::new(egui::Id::unique(("wordcraft-ime-equation", document)), math.session)
    } else {
        // A body replaced under the same document id (setText, version restore, compare,
        // Undo/Redo) advances the epoch, so the platform clears the old composition.
        egui::ImeTarget::new(egui::Id::unique(("wordcraft-ime-body", document, app.session.sel.focus.story)), app.session.ime_body_epoch())
    }
}

/// The bounded committed snapshot used by IME receipts.
///
/// An equation has no reversible tree-to-text mapping, and a selection across blocks has no
/// single visible paragraph, so both are honestly unavailable rather than fabricated.
pub(super) fn ime_stamp(app: &mut WordApp) -> egui::ImeMutationStamp {
    let unavailable = |app: &mut WordApp| match app.session.ime_state_revision() {
        Some(rev) => egui::ImeMutationStamp::unavailable_at_revision(rev),
        None => egui::ImeMutationStamp::unavailable(),
    };
    if app.session.math.is_some() {
        return unavailable(app);
    }
    match ime_window(app) {
        Some(w) => egui::ImeMutationStamp::from_committed(&w.text, w.cursor, w.anchor, None),
        None => unavailable(app),
    }
}

/// Record a successful accepted IME commit/delete for the current frame and viewport.
fn note_ime_receipt(app: &mut WordApp, before: egui::ImeMutationStamp, after: egui::ImeMutationStamp) {
    let Some(ctx) = app.ctx.clone() else { return };
    let viewport = ctx.viewport_id();
    let frame = ctx.cumulative_frame_nr_for(viewport);
    let target = ime_target(app);
    app.canvas.ime_tracker.note_input_method_apply(viewport, frame, target, before, after);
}

/// Record an application-origin edit in the current frame; `Other` dominates the frame.
pub(super) fn note_ime_application_change(app: &mut WordApp) {
    let Some(ctx) = app.ctx.clone() else { return };
    let viewport = ctx.viewport_id();
    let frame = ctx.cumulative_frame_nr_for(viewport);
    let target = ime_target(app);
    app.canvas.ime_tracker.note_application_change(viewport, frame, target);
}

/// Whether the stamp pair proves the transition an IME receipt needs.
/// The receipt decision, kept pure so tests can exercise every revision/availability case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImeProof {
    /// Classified no-op with an unchanged stamp: leave an earlier receipt as the frame's cause.
    NoMutation,
    /// A committed change the stamps cannot prove (or an automatic rewrite): `Other`.
    UnprovenMutation,
    /// A classified content change whose exact stamp transition is proven.
    Certified,
}

/// Wraps egui's unavailable-revision accessor for an IME mutation stamp.
fn unavailable_revision(stamp: &egui::ImeMutationStamp) -> Option<u64> {
    stamp.unavailable_revision()
}

/// Whether a pair of IME mutation stamps proves a transition: changed surrounding text, an
/// increase between unavailable revisions, or a mixed transition whose unavailable endpoint
/// carries a revision.
fn stamp_transition_proven(before: &egui::ImeMutationStamp, after: &egui::ImeMutationStamp) -> bool {
    use egui::output::IMESurroundingText;
    match (before.surrounding_text(), after.surrounding_text()) {
        (IMESurroundingText::Available(a), IMESurroundingText::Available(b)) => a != b,
        (IMESurroundingText::Unavailable, IMESurroundingText::Unavailable) => {
            matches!((unavailable_revision(before), unavailable_revision(after)), (Some(a), Some(b)) if a < b)
        }
        // A mixed transition has one unavailable endpoint; only that endpoint carries a revision.
        (IMESurroundingText::Unavailable, IMESurroundingText::Available(_)) | (IMESurroundingText::Available(_), IMESurroundingText::Unavailable) => {
            let unavailable = if matches!(before.surrounding_text(), IMESurroundingText::Unavailable) { before } else { after };
            unavailable_revision(unavailable).is_some()
        }
    }
}

/// Classifies an IME outcome and stamp pair as no mutation, unproven mutation or certified. Only a
/// classified, untransformed content change with a proven stamp transition is certified.
pub(super) fn classify_ime_proof(outcome: ImeEditOutcome, before: &egui::ImeMutationStamp, after: &egui::ImeMutationStamp) -> ImeProof {
    if !outcome.classified {
        return if outcome.state_changed { ImeProof::UnprovenMutation } else { ImeProof::NoMutation };
    }
    if outcome.transformed {
        return ImeProof::UnprovenMutation;
    }
    if outcome.state_changed && !outcome.content_changed {
        return ImeProof::UnprovenMutation;
    }
    if outcome.content_changed {
        return if stamp_transition_proven(before, after) { ImeProof::Certified } else { ImeProof::UnprovenMutation };
    }
    if before == after { ImeProof::NoMutation } else { ImeProof::UnprovenMutation }
}

/// Publish at most one receipt for a successful IME command, or force `Other` when the engine
/// outcome and the bounded stamps cannot prove a pure content change.
pub(super) fn publish_ime_receipt(app: &mut WordApp, before: egui::ImeMutationStamp, after: egui::ImeMutationStamp, outcome: ImeEditOutcome) {
    if app.ctx.is_none() {
        return;
    }
    // A successful application-origin command since the last observation poisons the frame, even
    // when the available text stamp is unchanged (formatting-only changes).
    if app.session.take_pending_ime_application_change() {
        note_ime_application_change(app);
        return;
    }
    match classify_ime_proof(outcome, &before, &after) {
        ImeProof::NoMutation => {}
        ImeProof::UnprovenMutation => note_ime_application_change(app),
        ImeProof::Certified => note_ime_receipt(app, before, after),
    }
}

/// AutoCorrect or AutoFormat just changed the text around the caret. The web's text agent still
/// holds the text as typed and counts its `DeleteSurrounding` from that, so it is asked to start
/// afresh (`canvas::ime_reset`). The caller passes the engine's per-command outcome; an undo label
/// from an earlier step cannot make an unrelated literal commit look transformed.
pub(super) fn note_automatic_change(app: &mut WordApp, outcome: ImeEditOutcome) {
    if outcome.transformed {
        app.canvas.ime_reset = true;
    }
}

/// An input method's `DeleteSurrounding`: erase `before` characters (Unicode scalar values, as
/// the input method counts them) before the caret and `after` after it, as Backspace does over a
/// selection. The web build's text agent (eframe's hidden `<input>`) sends it, then the new
/// text, whenever its value changes other than by an addition at its end (#1480): Backspace
/// and corrections from phone keyboards, and Chrome's caret jump. The desktop builds never get
/// it (egui-winit 0.36 doesn't send it).
///
/// The counts are over the visible logical text: tracked deletions are hidden and inline objects
/// contribute their plain-text result. They exclude the selection, which is preserved, and any
/// pre-edit (which never lives in the model here). Erasure joins an open typing group and the
/// text that follows joins it too ([`type_text`]), so one Undo takes back the typing with its
/// correction. Neither path runs inside an equation, whose text is not the document's.
pub(super) fn delete_surrounding(app: &mut WordApp, before: usize, after: usize) {
    delete_surrounding_common(app, before, after, wordcraft_doc::ime_text::DeleteUnit::Chars);
}

/// An input method's byte-counted `DeleteSurroundingBytes` (macOS and Wayland): erase the UTF-8
/// range it names, then the following `Commit` replaces it. Both counts round inward at a
/// character boundary (a start forward, an end backward), so a count that ends inside a
/// character leaves that character whole. The selection and paragraph marks stay outside the
/// range.
pub(super) fn delete_surrounding_bytes(app: &mut WordApp, before: usize, after: usize) {
    delete_surrounding_common(app, before, after, wordcraft_doc::ime_text::DeleteUnit::Bytes);
}

/// Refuse a body surrounding request and discard the composition it invalidates.
///
/// The platform has already changed the text it believed was around the caret, so its snapshot is
/// stale: interrupt it, unpaint the local pre-edit, and drop every replacement promise (body and
/// equation) that could otherwise join a later edit into an earlier deletion.
fn refuse_ime_request(app: &mut WordApp) {
    app.canvas.ime_preedit.clear();
    app.canvas.ime_preedit_range = None;
    app.canvas.ime_join_text = None;
    app.session.clear_ime_joins();
    app.canvas.ime_interrupt_now = true;
}

/// A signed stored-length change: negative when the paragraph shrank, positive only when a
/// restored content-control placeholder grew in place of the erased text.
fn signed_len_delta(before: usize, after: usize) -> Option<i64> {
    let after = i64::try_from(after).ok()?;
    let before = i64::try_from(before).ok()?;
    after.checked_sub(before)
}

/// Move a stored UTF-8 offset by a signed paragraph-length delta.
fn shift_offset(off: usize, shift: i64) -> Option<usize> {
    let off = i64::try_from(off).ok()?;
    usize::try_from(off.checked_add(shift)?).ok()
}

/// The end of a placeholder `restore_placeholder` just inserted at or before `deleted_start`.
///
/// `restore_placeholder` sets the caret to the control's content start before inserting. Finding
/// that exact state proves the growth came from a placeholder insertion at a position before the
/// erased range, so the stored suffix really moved by the measured signed delta.
fn restored_placeholder_end(s: &wordcraft_engine::Session, deleted_start: usize) -> Option<usize> {
    let at = &s.sel.focus;
    let r = s.doc.control_at(at)?;
    if !r.control.showing_placeholder || r.content_start() != *at || r.start.path != r.end.path {
        return None;
    }
    let start = r.content_start().off;
    if start > deleted_start {
        return None;
    }
    let placeholder = r.control.placeholder_shown();
    let end = start.checked_add(placeholder.len())?;
    // The caret alone is not proof: a control that was already showing its placeholder and had
    // that text deleted also leaves the caret at the content start. Require the restored
    // placeholder text itself to occupy the content from the start.
    let para = s.doc.para_at(at)?;
    (!placeholder.is_empty() && para.text.get(start..end) == Some(placeholder)).then_some(end)
}

/// The exact state a refused IME delete-surrounding transaction restores.
struct ImeRollback {
    snapshot: Option<wordcraft_engine::EditSnapshot>,
    original: wordcraft_engine::Selection,
    pending: Option<wordcraft_doc::props::CharProps>,
    goal_x: Option<f32>,
}

impl ImeRollback {
    fn apply(&mut self, app: &mut WordApp) {
        let Some(snapshot) = self.snapshot.take() else {
            return;
        };
        app.session.restore(snapshot);
        app.session.sel = self.original.clone();
        app.session.pending = self.pending.clone();
        app.session.goal_x = self.goal_x;
        refuse_ime_request(app);
    }
}

/// Whether the edit is a classified content and state change that is not a transform.
fn ime_edit_satisfied(app: &mut WordApp) -> bool {
    let outcome = app.session.ime_edit_outcome();
    outcome.classified && outcome.content_changed && outcome.state_changed && !outcome.transformed
}

/// Delete one IME-surrounding range. Returns the paragraph's signed length change and the end of
/// a placeholder the deletion restored, or `None` after rolling back the whole transaction.
fn ime_delete_side(
    app: &mut WordApp,
    rollback: &mut ImeRollback,
    cursor: &wordcraft_doc::Pos,
    start: usize,
    end: usize,
    join_undo: bool,
) -> Option<(i64, Option<usize>)> {
    if start >= end {
        rollback.apply(app);
        return None;
    }
    let Some(len_before) = app.session.doc.para_at(cursor).map(|p| p.len()) else {
        rollback.apply(app);
        return None;
    };
    // A control already showing its placeholder cannot have received a new one from this
    // deletion; its unchanged text must not be mistaken for a restored placeholder.
    let showing_before =
        app.session.doc.control_at(&wordcraft_doc::Pos { off: end, ..cursor.clone() }).is_some_and(|r| r.control.showing_placeholder);
    app.session.sel = wordcraft_engine::Selection {
        anchor: wordcraft_doc::Pos { off: start, ..cursor.clone() },
        focus: wordcraft_doc::Pos { off: end, ..cursor.clone() },
    };
    if join_undo {
        app.session.join_next_undo();
    }
    if app.run_ime("text.backspace", json!({})).is_err() || !ime_edit_satisfied(app) {
        rollback.apply(app);
        return None;
    }
    // A tracked delete of text this author inserted removes it from the stored paragraph;
    // other tracked deletions only mark it. A content control emptied by the deletion gains
    // its placeholder again, which can make the paragraph longer. Measure the signed change
    // and require proof for growth: only a placeholder restored at or before the erased
    // range can move the suffix right.
    let Some(len_after) = app.session.doc.para_at(cursor).map(|p| p.len()) else {
        rollback.apply(app);
        return None;
    };
    let Some(delta) = signed_len_delta(len_before, len_after) else {
        rollback.apply(app);
        return None;
    };
    let restored = restored_placeholder_end(&app.session, start).filter(|_| !showing_before);
    if delta > 0 && restored.is_none() {
        // Growth that cannot be attributed to a placeholder insertion at/before the erased
        // range has no provable suffix mapping: refuse the whole transaction.
        rollback.apply(app);
        return None;
    }
    // Keep the restored placeholder's end: for a caret it is the provable insertion point,
    // also when a tracked deletion keeps the erased bytes in the stored paragraph.
    Some((delta, restored))
}

/// The final caret/selection offsets after one or two surrounding deletions. A restored
/// placeholder end is the only provable position when one side inserted it; otherwise both
/// stored offsets move by the paragraph's signed length change.
fn ime_surrounding_offsets(
    del_anchor: usize,
    del_focus: usize,
    shift: i64,
    first_placeholder_end: Option<usize>,
    second_placeholder_end: Option<usize>,
) -> Option<(usize, usize)> {
    if let Some(end) = second_placeholder_end {
        // A selection spanning the emptied control cannot be remapped through a placeholder
        // insertion without guessing; only a caret has a provable position.
        return (del_anchor == del_focus).then_some((end, end));
    }
    if let (Some(end), true) = (first_placeholder_end, del_anchor == del_focus) {
        // The first side restored the placeholder; its end is the caret's provable position even
        // while an after-side deletion is applied beyond the control.
        return Some((end, end));
    }
    Some((shift_offset(del_anchor, shift)?, shift_offset(del_focus, shift)?))
}

/// Both delete-surrounding paths: one visible-logical mapping for the query and the deletion.
fn delete_surrounding_common(app: &mut WordApp, before: usize, after: usize, unit: wordcraft_doc::ime_text::DeleteUnit) {
    if before == 0 && after == 0 {
        return;
    }
    // A new surrounding request supersedes any replacement promised by an earlier one.
    app.session.clear_ime_joins();
    if app.session.math.is_some() {
        refuse_ime_request(app);
        return;
    }
    let cursor = app.session.sel.focus.clone();
    let anchor = app.session.sel.anchor.clone();
    if cursor.story != anchor.story || cursor.path != anchor.path {
        refuse_ime_request(app);
        return;
    }
    let Some(del) = app.session.doc.ime_delete_surrounding(&cursor, &anchor, before, after, unit) else {
        refuse_ime_request(app);
        return;
    };
    if del.before.is_none() && del.after.is_none() {
        refuse_ime_request(app);
        return;
    }
    let join = app.session.typing_open();
    let props = app.session.typing_props();
    let original = app.session.sel.clone();
    let pending_before = app.session.pending.clone();
    let goal_x_before = app.session.goal_x;
    let before_stamp = ime_stamp(app);
    // Both sides are one IME request: if the second half is refused (Restrict Editing, a
    // content control, an already-deleted tracked range, a classified no-op…), the first half
    // must go back too. The document, selection, undo stack, typing group, dirty flag, pending
    // formatting, `goal_x` and every pending join are restored or cleared. Deliberately not
    // restored: `last_command`, macro recording, Go Back marks, `also_selected`, `typing`/
    // `autocap` and canvas preedit (a refusal clears it).
    let snapshot = app.session.edit_snapshot();
    let mut rollback = ImeRollback { snapshot: Some(snapshot), original, pending: pending_before, goal_x: goal_x_before };
    // Stored offsets of the suffix (and of the original selection) move by the paragraph's
    // signed length change. A placeholder can make that positive; subtracting a removed-byte
    // count cannot describe that, and would leave the second range and the caret inside the
    // placeholder.
    let mut shift = 0i64;
    let mut deleted = false;
    let mut first_placeholder_end = None;
    if let Some(range) = del.before.clone() {
        let Some((delta, restored)) = ime_delete_side(app, &mut rollback, &cursor, range.start, range.end, join) else {
            return;
        };
        deleted = true;
        first_placeholder_end = restored;
        shift = delta;
    }
    let mut second_placeholder_end = None;
    if let Some(range) = del.after {
        let Some(start) = shift_offset(range.start, shift) else {
            rollback.apply(app);
            return;
        };
        let Some(end) = shift_offset(range.end, shift) else {
            rollback.apply(app);
            return;
        };
        let in_bounds = app
            .session
            .doc
            .para_at(&cursor)
            .is_some_and(|p| start < end && end <= p.len() && p.text.is_char_boundary(start) && p.text.is_char_boundary(end));
        if !in_bounds {
            rollback.apply(app);
            return;
        }
        // The mapped end is in the paragraph as the first side left it; a control already
        // showing its placeholder there is not one this side restores.
        let Some((_, restored)) = ime_delete_side(app, &mut rollback, &cursor, start, end, deleted || join) else {
            return;
        };
        deleted = true;
        second_placeholder_end = restored;
    }
    let Some((anchor_off, focus_off)) = ime_surrounding_offsets(del.anchor, del.focus, shift, first_placeholder_end, second_placeholder_end) else {
        rollback.apply(app);
        return;
    };
    let valid =
        app.session.doc.para_at(&cursor).is_some_and(|p| {
            anchor_off <= p.len() && focus_off <= p.len() && p.text.is_char_boundary(anchor_off) && p.text.is_char_boundary(focus_off)
        });
    if !valid {
        rollback.apply(app);
        return;
    }
    app.session.sel = wordcraft_engine::Selection {
        anchor: wordcraft_doc::Pos { off: anchor_off, ..cursor.clone() },
        focus: wordcraft_doc::Pos { off: focus_off, ..cursor },
    };
    if deleted {
        // Only the next `text.insert` at this exact selection may join the deletion; any
        // intervening ordinary command consumes and discards the promise and owns its own Undo
        // checkpoint.
        app.session.join_next_ime_text_replacement();
        app.canvas.ime_join_text = Some(props);
        let after_stamp = ime_stamp(app);
        let outcome = ImeEditOutcome { classified: true, content_changed: true, state_changed: true, transformed: false, selection_changed: true };
        publish_ime_receipt(app, before_stamp, after_stamp, outcome);
    }
}

/// An input method's `DeleteSurrounding` inside an equation: one bounded transactional command.
///
/// The structured tree has no surrounding snapshot; the engine revision on the unavailable stamp
/// proves the commit. Any refusal leaves the document untouched and interrupts the composition.
/// `Session::run` has already consumed the pending undo joins before dispatch and may have
/// recorded the attempted no-op command in the macro/Repeat; those generic side effects are not
/// rolled back.
pub(super) fn delete_in_equation(app: &mut WordApp, before: usize, after: usize) {
    if app.session.math.is_none() {
        return;
    }
    // A new surrounding request supersedes any earlier equation deletion's promised join,
    // whether or not this one can run. Clear the painted composition before every refusal,
    // too: it belongs to a snapshot the interrupt is about to discard.
    app.session.clear_ime_joins();
    app.canvas.ime_preedit.clear();
    app.canvas.ime_preedit_range = None;
    if before > wordcraft_engine::MAX_EQUATION_DELETE_UNITS || after > wordcraft_engine::MAX_EQUATION_DELETE_UNITS {
        app.canvas.ime_interrupt_now = true;
        return;
    }
    if before == 0 && after == 0 {
        return;
    }
    if app.session.typing_open() {
        app.session.join_next_undo();
    }
    let before_stamp = ime_stamp(app);
    match app.run_ime("equation.deleteSurrounding", json!({"before": before, "after": after})) {
        Ok(_) => {
            // The replacement the platform commits next belongs to the same edit as the
            // deletion, exactly like the body path's `ime_join_text`. This is an equation-only
            // promise: an unrelated command consumes it without joining, so it cannot merge
            // into this deletion's undo step.
            app.session.join_next_equation_edit();
            let outcome = app.session.ime_edit_outcome();
            let after_stamp = ime_stamp(app);
            publish_ime_receipt(app, before_stamp, after_stamp, outcome);
        }
        Err(_) => {
            app.canvas.ime_interrupt_now = true;
        }
    }
}
