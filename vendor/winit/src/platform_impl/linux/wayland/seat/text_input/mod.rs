//! Wayland text-input-v3 state.
//!
//! Two protocol directions are kept apart:
//!
//! * Incoming records (`preedit_string`, `commit_string`, `delete_surrounding_text`) are
//!   accumulated per text input and applied on `done`. The incoming lifecycle fence
//!   (`incoming_from`) only rejects a `done` from a previous enable/disable/reset generation. A
//!   `done` whose serial differs from the latest outgoing commit still applies the batch, as
//!   text-input-v3 requires; it just does not acknowledge the outgoing state.
//! * Outgoing state (`set_content_type`, `set_cursor_rectangle`, `set_surrounding_text`) is a
//!   desired snapshot per window and an acknowledged/pending snapshot per seat. A setter while a
//!   commit is unacknowledged only marks the seat's desired state dirty; it is coalesced. The
//!   matching `done` acknowledges the pending snapshot and flushes the newest desired snapshot with
//!   exactly one fresh `commit`. The serial is the wrapping count of all commits issued on the
//!   object, including the mandatory lifecycle commits.
//!
//! text-input-v3 says a `done` whose serial differs from the number of commits issued must still
//! be applied normally, but the client must not change the protocol's current state; pending
//! state requests must be sent and committed after a matching `done`. The reducer below follows
//! that rule: `acknowledge_done` is separate from `finish_done`.
//!
//! text-input-v3 also invalidates state on `enter` and on a committed disable/enable, so the full
//! snapshot is resent after enable and reset with `enable` -> state setters -> `commit`. The only
//! way to make an already-published surrounding text unavailable again is that committed reset;
//! the window's clear operation performs it only when text was published or a generation is
//! active, and never from a disabled window. An available empty document stays distinct: it is
//! sent as
//! `set_surrounding_text("", 0, 0)`, while unavailable sends no surrounding-text request.
//!
//! Lock order: a window-state lock may be held while taking the text input's inner data lock,
//! but that data lock must never be held while taking a window-state lock. Dispatch does the
//! window lookup and lock first; the data-only event paths never touch the window map.
//!
//! Every queued compositor IME event additionally carries the lifecycle token of the generation
//! that produced it (see `EventSink`). Disabling, resetting, leaving or destroying the seat
//! retires that token: already queued `Enabled`/`Preedit` composition state is dropped at
//! delivery, while a `Commit` (and the deletion attached to it) that was already accepted from a
//! protocol `done` is committed application input and is delivered before the boundary's
//! clear-preedit/`Disabled`. The event loop rejects every queued IME event, including those of
//! retired generations and untagged synthetic boundaries, when the window's closed tombstone is
//! set; this survives a seat whose text input was destroyed before the window.
//!
//! A window has one visible preedit owner even when several seats have entered text inputs. The
//! public `Ime` events identify the window, not the seat, so this is window-level composition:
//! every accepted commit from any seat emits `Preedit("")` immediately before its commit and
//! releases the old visible owner. A new nonempty preedit then claims ownership. A non-owner's
//! empty-preedit-only `done` must not clear another seat's visible text or claim ownership. The
//! owner's leave/reset/empty `done` clears the composition. This policy preserves
//! `Ime::Commit`'s clear-before-commit contract; it does not claim simultaneous per-seat editing.
//!
//! `preedit_string`, `commit_string` and `delete_surrounding_text` are independent records. The
//! order in which they arrive before `done` does not change the protocol's apply order: clear the
//! old preedit, delete, insert the commit, then insert the new preedit.
//!
//! The application's surrounding-text setter reports `text_change_cause:other` by default. An
//! explicit `InputMethod` cause is available when the change comes from applying IME input; the
//! protocol resets the cause to `input_method` after each commit. Every state commit that carries
//! surrounding text names its cause explicitly: initial enable, reset, purpose and cursor-only
//! commits use `Other`, while a coalesced IME receipt keeps `InputMethod` across a pending
//! geometry-only update.
//!
//! The wire has no record-generation tag. A compositor that coalesces an old record and sends
//! only a new `done` cannot be distinguished from a genuine new commit by the client. We apply
//! current-generation records and do not silently discard a valid new commit to claim a guarantee
//! the protocol does not provide; this remains a native qualification gate.
//!
//! The synthetic lifecycle events live in the window's sink and compositor events in
//! `WinitState::events_sink`; both carry a process-wide push sequence and the event loop merges
//! them before delivery. That ordering is required because a boundary is often queued after a
//! commit but in the other sink.
//!
//! Enable is sent before the state that enabling resets and committed after it:
//! `enable` -> state setters -> `commit`. `text-input-v3` resets the content type, cursor
//! rectangle and surrounding text on `enable`, so committing the state first would lose it.

use std::ops::Deref;

use sctk::globals::GlobalData;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};

use sctk::reexports::client::globals::{BindError, GlobalList};
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{delegate_dispatch, Dispatch};
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ChangeCause, ContentHint, ContentPurpose, Event as TextInputEvent, ZwpTextInputV3,
};

use crate::event::{Ime, WindowEvent};
use crate::platform::wayland::{ImeDeleteSurrounding, ImeTextChangeCause};
use crate::platform_impl::wayland;
use crate::platform_impl::wayland::event_loop::sink::{
    next_sequence, EventSink, ImeLifecycleToken,
};
use crate::platform_impl::wayland::state::WinitState;
use crate::platform_impl::wayland::window::state::{ImeBatchOutcome, TextInputLeft, WindowState};
use crate::platform_impl::wayland::WindowId;
use crate::window::ImePurpose;

pub struct TextInputState {
    text_input_manager: ZwpTextInputManagerV3,
}

impl TextInputState {
    pub fn new(
        globals: &GlobalList,
        queue_handle: &QueueHandle<WinitState>,
    ) -> Result<Self, BindError> {
        let text_input_manager = globals.bind(queue_handle, 1..=1, GlobalData)?;
        Ok(Self { text_input_manager })
    }
}

impl Deref for TextInputState {
    type Target = ZwpTextInputManagerV3;

    fn deref(&self) -> &Self::Target {
        &self.text_input_manager
    }
}

impl Dispatch<ZwpTextInputManagerV3, GlobalData, WinitState> for TextInputState {
    fn event(
        _state: &mut WinitState,
        _proxy: &ZwpTextInputManagerV3,
        _event: <ZwpTextInputManagerV3 as Proxy>::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
    }
}

impl Dispatch<ZwpTextInputV3, TextInputData, WinitState> for TextInputState {
    fn event(
        state: &mut WinitState,
        text_input: &ZwpTextInputV3,
        event: <ZwpTextInputV3 as Proxy>::Event,
        data: &TextInputData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
        let windows = state.windows.get_mut();
        match event {
            TextInputEvent::Enter { surface } => {
                let window_id = wayland::make_wid(&surface);

                // An `enter` without a matching `leave` (a missed leave or a seat re-enter) must
                // not leave this proxy registered in, or owning the preedit of, the previous
                // surface. Resolve the stored surface first, then detach it under the window
                // lock before the new owner is installed.
                let previous_surface = data.inner.lock().unwrap().surface.clone();
                if let Some(previous_id) = previous_surface.as_ref().map(wayland::make_wid) {
                    if previous_id != window_id {
                        if let Some(previous_window) = windows.get(&previous_id).cloned() {
                            let mut previous_window = previous_window.lock().unwrap();
                            let (clear_preedit, left) =
                                detach_input_from_window(&mut previous_window, text_input);
                            let (_, was_active) = {
                                let mut data = data.inner.lock().unwrap();
                                let was_active = data.is_active();
                                (data.end_owner(), was_active)
                            };
                            push_ime_teardown(
                                &mut state.events_sink,
                                previous_id,
                                left,
                                clear_preedit,
                                was_active,
                            );
                        }
                    }
                }

                // Prepare the owner before taking any window lock. The batch is not accepting
                // input until the enable commit below, so a stale `done` cannot be applied.
                {
                    let mut data = data.inner.lock().unwrap();
                    data.enter_generation(window_id);
                    data.surface = Some(surface);
                }

                let Some(window) = windows.get(&window_id).cloned() else {
                    data.inner.lock().unwrap().end_owner();
                    return;
                };
                let mut window = window.lock().unwrap();

                if window.ime_allowed() {
                    // Wire order is `enable` -> state -> `commit`: enabling resets the state
                    // setters that follow it. Keep the window-state lock while arming so a
                    // concurrent `set_ime_allowed(false)` cannot slip between the enable commit
                    // and the generation fence.
                    let ime_state = window.outgoing_ime_state();
                    let (_, sequence) = text_input.enable_with_state(&ime_state);
                    // `Enabled` is a window-level lifecycle notice, not composition owned by
                    // this generation. If a reset or clear arrives before delivery, the token
                    // may already be retired, but the application still must learn that the
                    // window allows IME before it receives a commit or preedit.
                    state.events_sink.push_window_event_at(
                        sequence,
                        WindowEvent::Ime(Ime::Enabled),
                        window_id,
                    );
                }

                window.text_input_entered(text_input);
            },
            TextInputEvent::Leave { surface } => {
                // The stored surface is the authoritative owner. A compositor may send `leave`
                // for a nil or unmapped surface; using the event surface would then skip all
                // window cleanup and leave a stale proxy/owner behind.
                let stored_surface = data.inner.lock().unwrap().surface.clone();
                let stored_id = stored_surface.as_ref().map(wayland::make_wid);
                let event_id = wayland::make_wid(&surface);
                let resolved = stored_id
                    .and_then(|id| windows.get(&id).cloned().map(|window| (id, window)))
                    .or_else(|| windows.get(&event_id).cloned().map(|window| (event_id, window)));

                let Some((window_id, window)) = resolved else {
                    // No surface we ever entered is still known. There is no window callback to
                    // deliver, but the input must stop accepting records for it.
                    data.inner.lock().unwrap().end_owner();
                    return;
                };
                let mut window = window.lock().unwrap();

                // Drop the incoming batch and the old surface before the text input can enter
                // another surface. The owner may still have a preedit on screen, so the window
                // clears it at the IME boundary below. A `Commit` already queued by an earlier
                // `done` keeps its token as committed input; only the queued composition is
                // retired. text-input-v3 says the compositor ignores requests after `leave`, so
                // no `disable`/`commit` is sent here; an ignored commit would also make the next
                // `done` serial fall behind winit's own commit count.
                let (clear_preedit, left) = detach_input_from_window(&mut window, text_input);
                let (_, was_active) = {
                    let mut data = data.inner.lock().unwrap();
                    let was_active = data.is_active();
                    (data.end_owner(), was_active)
                };

                // A window can have one text input per seat. Leave from one seat must not report
                // IME off for the others, so `Disabled` is only emitted when this was the last
                // entered text input. A preedit clear is not window-wide: it belongs to the seat
                // that owns the window's visible preedit and must still be emitted when another
                // seat remains entered.
                push_ime_teardown(
                    &mut state.events_sink,
                    window_id,
                    left,
                    clear_preedit,
                    was_active,
                );
            },
            TextInputEvent::PreeditString { text, cursor_begin, cursor_end } => {
                data.inner.lock().unwrap().preedit_string(text, cursor_begin, cursor_end);
            },
            TextInputEvent::CommitString { text } => {
                data.inner.lock().unwrap().commit_string(text);
            },
            TextInputEvent::Done { serial } => {
                // Resolve the owner first and release the data lock. Handling `done` also needs
                // the window's desired outgoing state, and the permitted lock order is window
                // first, then the text-input data. Never take the window lock while holding the
                // data lock.
                let current_owner =
                    { data.inner.lock().unwrap().surface.as_ref().map(wayland::make_wid) };
                let window = current_owner
                    .and_then(|window_id| windows.get(&window_id).cloned())
                    .map(|window| (window, current_owner));

                if let Some((window, current_owner)) = window {
                    let mut window = window.lock().unwrap();
                    let desired = window.outgoing_ime_state();
                    let mut text_input_data = data.inner.lock().unwrap();

                    // Incoming first: text-input-v3 applies the batch even when the serial does
                    // not match. The outgoing reducer then separately decides whether this
                    // `done` may acknowledge or flush client state.
                    let generation = text_input_data.generation.clone();
                    let may_clear_preedit = window.may_clear_ime_preedit(text_input);
                    let outcome = text_input_data.finish_done_with_preedit_policy(
                        serial,
                        current_owner,
                        &mut state.events_sink,
                        may_clear_preedit,
                    );
                    text_input_data.acknowledge_done(serial, Some(&desired), text_input);

                    // The public IME stream has no seat identity, so only one text input can own
                    // the window's visible composition. An accepted commit ends that composition
                    // for the whole window even when another seat produced it; a non-empty new
                    // preedit then claims it. An empty preedit from a non-owner neither clears nor
                    // steals the owner's composition. The window lock is still held, so no
                    // enable/reset can replace the generation before ownership is updated.
                    let current = generation.as_ref().is_some_and(|generation| {
                        text_input_data
                            .generation
                            .as_ref()
                            .is_some_and(|current| current.is_same_generation(generation))
                    });
                    if current {
                        window.note_ime_done(text_input, outcome);
                    }
                    drop(text_input_data);
                } else {
                    let mut text_input_data = data.inner.lock().unwrap();
                    text_input_data.finish_done(serial, current_owner, &mut state.events_sink);
                    text_input_data.acknowledge_done(serial, None, text_input);
                }
            },
            TextInputEvent::DeleteSurroundingText { before_length, after_length } => {
                data.inner.lock().unwrap().delete_surrounding_text(before_length, after_length);
            },
            _ => {},
        }
    }
}

/// The protocol requests this module issues, so tests can record the wire order.
///
/// Production implements this for [`ZwpTextInputV3`]. The lifecycle and outgoing-state ordering
/// lives in the free functions and the reducer below, which both paths use.
pub(crate) trait TextInputRequests {
    fn enable(&self);
    fn disable(&self);
    fn set_content_type(&self, purpose: ImePurpose);
    fn set_cursor_rectangle(&self, x: i32, y: i32, width: i32, height: i32);
    fn set_text_change_cause(&self, cause: ImeTextChangeCause);
    fn set_surrounding_text(&self, text: String, cursor: i32, anchor: i32);
    fn commit(&self);
}

fn content_type(purpose: ImePurpose) -> (ContentHint, ContentPurpose) {
    match purpose {
        ImePurpose::Normal => (ContentHint::None, ContentPurpose::Normal),
        ImePurpose::Password => (ContentHint::SensitiveData, ContentPurpose::Password),
        ImePurpose::Terminal => (ContentHint::None, ContentPurpose::Terminal),
    }
}

impl TextInputRequests for ZwpTextInputV3 {
    fn enable(&self) {
        ZwpTextInputV3::enable(self);
    }

    fn disable(&self) {
        ZwpTextInputV3::disable(self);
    }

    fn set_content_type(&self, purpose: ImePurpose) {
        let (hint, purpose) = content_type(purpose);
        ZwpTextInputV3::set_content_type(self, hint, purpose);
    }

    fn set_cursor_rectangle(&self, x: i32, y: i32, width: i32, height: i32) {
        ZwpTextInputV3::set_cursor_rectangle(self, x, y, width, height);
    }

    fn set_text_change_cause(&self, cause: ImeTextChangeCause) {
        let cause = match cause {
            ImeTextChangeCause::InputMethod => ChangeCause::InputMethod,
            ImeTextChangeCause::Other => ChangeCause::Other,
        };
        ZwpTextInputV3::set_text_change_cause(self, cause);
    }

    fn set_surrounding_text(&self, text: String, cursor: i32, anchor: i32) {
        ZwpTextInputV3::set_surrounding_text(self, text, cursor, anchor);
    }

    fn commit(&self) {
        ZwpTextInputV3::commit(self);
    }
}

/// What the compositor should currently know about one enabled text input.
///
/// `surrounding_text` is `None` for "unavailable": no `set_surrounding_text` request is sent, so
/// an input method does not treat the empty string as the document. `Some(("", 0, 0))` is the
/// internal representation of an available empty document and is sent as an empty request. The
/// v1 wire protocol has no separate availability bit and warns that empty initial values may be
/// treated as unsupported, so retaining that distinction is a compositor-dependent best effort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingImeState {
    pub(crate) purpose: ImePurpose,
    pub(crate) cursor_area: Option<(i32, i32, i32, i32)>,
    pub(crate) surrounding_text: Option<(String, i32, i32)>,
}

impl OutgoingImeState {
    /// Builds the snapshot for one window, refusing to carry text for a password purpose.
    ///
    /// The password check is repeated here, at the protocol boundary, so no caller path can
    /// publish private text through a stale cached snapshot.
    pub fn new(
        purpose: ImePurpose,
        cursor_area: Option<(i32, i32, i32, i32)>,
        surrounding_text: Option<(String, i32, i32)>,
    ) -> Self {
        let surrounding_text =
            if purpose == ImePurpose::Password { None } else { surrounding_text };
        Self { purpose, cursor_area, surrounding_text }
    }
}

/// The state a text input's newest commit sent, or the state an acknowledged commit left behind.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
enum SentContext {
    /// A committed disable (or no commit yet): there is no current enabled state.
    #[default]
    Disabled,
    /// A committed enable and the state it carried.
    Enabled(OutgoingImeState),
}

/// The result of a setter or acknowledgement, for tests and diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutgoingCommit {
    /// The compositor already has this state.
    Quiet,
    /// A commit is outstanding; the newest desired state is coalesced until its matching `done`.
    Deferred,
    /// A matching `done` acknowledged the pending commit and this state was sent.
    Acknowledged,
    /// The `done` serial did not match the newest commit; incoming edits apply but outgoing
    /// state waits.
    Unmatched,
    /// One fresh commit carrying this serial was sent.
    Sent(u32),
}

fn send_state_requests<T: TextInputRequests>(
    input: &T,
    state: &OutgoingImeState,
    change_cause: ImeTextChangeCause,
) {
    // text-input-v3 applies the requests in arrival order; all of them are part of the same
    // commit. A password snapshot never contains surrounding text. Every commit that carries
    // surrounding text names its cause explicitly instead of relying on the protocol default
    // (`input_method`), which would mislabel ordinary application edits and cursor moves.
    input.set_content_type(state.purpose);
    if let Some((x, y, width, height)) = state.cursor_area {
        input.set_cursor_rectangle(x, y, width, height);
    }
    if let Some((text, cursor, anchor)) = &state.surrounding_text {
        input.set_text_change_cause(change_cause);
        input.set_surrounding_text(text.clone(), *cursor, *anchor);
    }
}

/// Ends the current incoming generation with `disable`, then `commit`.
///
/// Returns whether a preedit was shown before the boundary and the boundary's ordering ticket.
///
/// The ticket is reserved while the data lock is held, after any commit already accepted by this
/// text input has been queued. The synthetic `Disabled`/clear is pushed later from the window
/// side, but uses this ticket so it cannot sort before those commits.
fn disable_generation<T: TextInputRequests>(
    input: &T,
    data: &mut TextInputDataInner,
) -> (bool, u64) {
    let sequence = next_sequence();
    let preedit_shown = data.end_incoming_generation();
    input.disable();
    data.request_commit();
    input.commit();
    // The disable commit is itself unacknowledged, but it is not an enabled context to restore.
    data.awaiting = Some(SentContext::Disabled);
    data.pending_change_cause = None;
    (preedit_shown, sequence)
}

/// Starts a new incoming generation in wire order: `enable`, the state setters, then `commit`.
///
/// text-input-v3 resets the content type, cursor rectangle and surrounding text on `enable`, so
/// sending the state before the enable would lose it. The generation starts accepting `done` at
/// the commit serial. Returns the enable commit serial and the boundary's ordering ticket.
fn enable_generation<T: TextInputRequests>(
    input: &T,
    data: &mut TextInputDataInner,
    state: &OutgoingImeState,
) -> (u32, u64) {
    // The ticket is earlier than every protocol event the new generation can produce, because
    // such an event can only arrive after the enable commit below.
    let sequence = next_sequence();
    let arm = data.begin_enable();
    input.enable();
    let serial = data.send_enabled_commit(input, state, Some(ImeTextChangeCause::Other));
    data.finish_enable(arm, serial);
    (serial, sequence)
}

/// Re-enables text input after a disable: `disable`/`commit`, then `enable`/state/`commit`.
/// Returns whether a preedit was shown, the enable commit serial and the disable boundary's
/// ordering ticket (used by the synthetic clear).
fn reset_generation<T: TextInputRequests>(
    input: &T,
    data: &mut TextInputDataInner,
    state: &OutgoingImeState,
) -> (bool, u32, u64) {
    let (preedit_shown, sequence) = disable_generation(input, data);
    let (serial, _) = enable_generation(input, data, state);
    (preedit_shown, serial, sequence)
}

/// Reset a window's entered seats as one input fence. The caller holds the window lock.
/// Hold every data lock until the completion ticket is reserved: otherwise the first seat's
/// new generation could accept input while a later seat still belongs to the old context.
fn reset_inputs_with_fence<T: TextInputRequests>(
    inputs: &[(&T, Option<&std::sync::Mutex<TextInputDataInner>>)],
    state: &OutgoingImeState,
) -> (Vec<(bool, u64)>, u64) {
    let mut locked: Vec<_> =
        inputs.iter().map(|(_, data)| data.map(|data| data.lock().unwrap())).collect();
    let mut resets = Vec::with_capacity(inputs.len());
    for ((input, _), data) in inputs.iter().zip(&mut locked) {
        let reset = if let Some(data) = data {
            let (shown, _, sequence) = reset_generation(*input, data, state);
            (shown, sequence)
        } else {
            let sequence = next_sequence();
            input.disable();
            input.commit();
            input.enable();
            send_state_requests(*input, state, ImeTextChangeCause::Other);
            input.commit();
            (false, sequence)
        };
        resets.push(reset);
    }
    // Also reserve a real fence when there are no entered objects. Old accepted input keeps
    // its earlier tickets and a future enter cannot publish new input before this ticket.
    let completion = next_sequence();
    (resets, completion)
}

pub(crate) fn reset_ime_with_fence(
    text_inputs: &[ZwpTextInputV3],
    state: &OutgoingImeState,
) -> (Vec<(bool, u64)>, u64) {
    let inputs: Vec<_> = text_inputs
        .iter()
        .map(|input| (input, input.data::<TextInputData>().map(|data| &data.inner)))
        .collect();
    reset_inputs_with_fence(&inputs, state)
}

/// Whether clearing an IME context needs the only request that can make surrounding text
/// unavailable again: a committed disable/enable reset.
///
/// A disabled window is never reset (that would re-enable it). Published surrounding text and a
/// visible composition both need invalidation, and so does every active or buffered incoming
/// generation: a commit/deletion in flight from before the context change must not become input
/// for the new field or a password field. That last condition is independent of the cached text
/// and of whether a preedit is currently visible.
pub fn context_reset_needed(
    had_surrounding_text: bool,
    ime_allowed: bool,
    has_entered_input: bool,
    has_visible_composition: bool,
    has_pending_or_active_incoming: bool,
) -> bool {
    ime_allowed
        && has_entered_input
        && (had_surrounding_text || has_visible_composition || has_pending_or_active_incoming)
}

/// Removes one text input from a window and reports what the teardown must emit.
///
/// Callers hold the window lock, so a concurrent `set_ime_allowed` cannot re-enable the proxy
/// between detaching it and sampling whether it was active.
fn detach_input_from_window(
    window: &mut WindowState,
    text_input: &ZwpTextInputV3,
) -> (bool, TextInputLeft) {
    let clear_preedit = window.release_ime_preedit(text_input);
    let left = window.text_input_left(text_input);
    if left == TextInputLeft::Last {
        // No other seat remains in this window, so this seat's leftover deletion belongs to no
        // pending commit.
        window.ime_delete_surrounding = None;
    }
    (clear_preedit, left)
}

/// Queues the window-level end of IME after a seat's text input left, was removed with its seat,
/// or its window was destroyed.
///
/// The window holds one text input per seat; only when the last one leaves, and only if the
/// application had actually been told IME was enabled, does leave mean `Disabled`. This keeps a
/// seat disconnecting from switching IME off for the other seats sharing the window.
pub(crate) fn push_ime_teardown(
    events: &mut EventSink,
    window_id: WindowId,
    left: TextInputLeft,
    clear_preedit: bool,
    was_active: bool,
) {
    if clear_preedit {
        events.push_window_event(WindowEvent::Ime(Ime::Preedit(String::new(), None)), window_id);
    }
    if left == TextInputLeft::Last && was_active {
        events.push_window_event(WindowEvent::Ime(Ime::Disabled), window_id);
    }
}

pub trait ZwpTextInputV3Ext {
    /// Disables text input and drops the incoming generation.
    ///
    /// Returns whether a preedit was shown before the boundary and its ordering ticket. No new
    /// `Preedit`/`Commit` record from the old generation is accepted after this, including a
    /// `done` that was already in flight. A commit that was already queued by an earlier `done`
    /// stays deliverable.
    fn disable_and_commit(&self) -> (bool, u64);

    /// Enables text input with `enable` -> state -> `commit` in that wire order and starts
    /// accepting the generation that begins with the commit.
    fn enable_with_state(&self, state: &OutgoingImeState) -> (u32, u64);

    /// Disables, then enables with the state applied in between, in protocol order.
    ///
    /// Returns whether a preedit was shown before the boundary, the enable commit serial and the
    /// disable boundary's ordering ticket.
    fn reset_with_state(&self, state: &OutgoingImeState) -> (bool, u32, u64);

    /// Publishes a changed desired snapshot, or coalesces it behind an outstanding commit.
    ///
    /// Never sends a lifecycle request: an ordinary edit keeps the current composition.
    fn sync_ime_state(
        &self,
        state: &OutgoingImeState,
        change_cause: Option<ImeTextChangeCause>,
    ) -> OutgoingCommit;
}

impl ZwpTextInputV3Ext for ZwpTextInputV3 {
    fn disable_and_commit(&self) -> (bool, u64) {
        let Some(data) = self.data::<TextInputData>() else {
            let sequence = next_sequence();
            self.disable();
            self.commit();
            return (false, sequence);
        };
        let mut data = data.inner.lock().unwrap();
        disable_generation(self, &mut data)
    }

    fn enable_with_state(&self, state: &OutgoingImeState) -> (u32, u64) {
        let Some(data) = self.data::<TextInputData>() else {
            let sequence = next_sequence();
            self.enable();
            send_state_requests(self, state, ImeTextChangeCause::Other);
            self.commit();
            return (0, sequence);
        };
        // The data lock is held across the whole sequence so a boundary from another thread
        // cannot interleave an enable and a disable.
        let mut data = data.inner.lock().unwrap();
        enable_generation(self, &mut data, state)
    }

    fn reset_with_state(&self, state: &OutgoingImeState) -> (bool, u32, u64) {
        let Some(data) = self.data::<TextInputData>() else {
            let sequence = next_sequence();
            self.disable();
            self.commit();
            self.enable();
            send_state_requests(self, state, ImeTextChangeCause::Other);
            self.commit();
            return (false, 0, sequence);
        };
        let mut data = data.inner.lock().unwrap();
        reset_generation(self, &mut data, state)
    }

    fn sync_ime_state(
        &self,
        state: &OutgoingImeState,
        change_cause: Option<ImeTextChangeCause>,
    ) -> OutgoingCommit {
        let Some(data) = self.data::<TextInputData>() else { return OutgoingCommit::Deferred };
        let mut data = data.inner.lock().unwrap();
        data.sync_outgoing(self, state, change_cause)
    }
}

/// The Data associated with the text input.
#[derive(Default)]
pub struct TextInputData {
    inner: std::sync::Mutex<TextInputDataInner>,
}

impl TextInputData {
    /// The window had an enabled incoming generation, so the application was told IME is on.
    pub(crate) fn is_active(&self) -> bool {
        self.inner.lock().unwrap().is_active()
    }

    /// Ends the owner and drops its composition. Accepted queued commits stay deliverable.
    pub(crate) fn end_owner(&self) -> bool {
        self.inner.lock().unwrap().end_owner()
    }

    /// Whether this text input has an active generation or records buffered before `done`.
    pub(crate) fn has_pending_or_active(&self) -> bool {
        self.inner.lock().unwrap().has_pending_or_active()
    }

    /// Drops the incoming state if the batch belongs to `owner`.
    ///
    /// Used for a window that is being destroyed. Queued events are rejected separately by the
    /// event loop's pre-delivery window tombstone, which also covers a seat whose text input was
    /// already destroyed before the window closed.
    pub(crate) fn clear_if_owner(&self, owner: WindowId) {
        self.inner.lock().unwrap().clear_if_owner(owner);
    }
}

#[derive(Default)]
pub struct TextInputDataInner {
    /// The `WlSurface` we're performing input to.
    surface: Option<WlSurface>,

    /// The incoming text-input-v3 state accumulated since the last `done`.
    batch: PendingImeBatch,

    /// Whether the window was last sent a preedit rather than its end.
    preedit_shown: bool,

    /// The number of `commit` requests issued on this text input.
    ///
    /// text-input-v3 counts commits per object and uses the count as the `done` serial. Keeping
    /// the same wrapping count lets a lifecycle boundary fence off events from the previous
    /// generation without treating every mismatched serial as stale.
    request_serial: u32,

    /// The first `done` serial that belongs to the current incoming generation.
    ///
    /// `None` means incoming state is not accepted, either because no surface entered or because
    /// an enable/disable/reset boundary is waiting for the matching enable commit.
    incoming_from: Option<u32>,

    /// The token shared with the queued events of the current incoming generation.
    ///
    /// A boundary replaces or drops it and retires the old token, so `EventSink` drops queued
    /// composition state from that generation while keeping commits it already accepted.
    generation: Option<ImeLifecycleToken>,

    /// The arm recorded by `begin_enable`, until the matching `finish_enable`.
    armed: Option<u64>,

    /// Counts enable arms. A commit is only accepted while its arm is still current, so a
    /// boundary racing an enable cannot leave the generation accepting after the boundary.
    arm_epoch: u64,

    /// The protocol state the newest commit acknowledged.
    ///
    /// `Disabled` means the compositor has no current enabled state (or no commit was
    /// acknowledged yet). A matching `done` moves `awaiting` here.
    acknowledged: SentContext,

    /// The state carried by the newest commit while it is unacknowledged.
    ///
    /// `Some` blocks new outgoing commits: text-input-v3 asks the client to wait for a matching
    /// `done` before sending pending state.
    awaiting: Option<SentContext>,

    /// The application's cause for a surrounding-text change coalesced behind `awaiting`.
    ///
    /// The protocol resets the cause after every commit, so it is kept only until the commit
    /// that carries the matching snapshot is sent.
    pending_change_cause: Option<ImeTextChangeCause>,
}

/// The text-input-v3 records accumulated between `done` events.
///
/// `owner` is the surface the records arrived for. It is set on `enter` and only cleared by a
/// lifecycle boundary, so a batch cannot be dispatched against a different surface later.
#[derive(Default)]
struct PendingImeBatch {
    owner: Option<WindowId>,

    /// The commit to submit on `done`. `None` means no `commit_string` arrived, while
    /// `Some("")` is an explicitly empty commit.
    commit: Option<String>,

    /// The preedit to submit on `done`.
    preedit: Option<Preedit>,

    /// The surrounding text to delete on `done`.
    delete: Option<ImeDeleteSurrounding>,
}

/// Whether a wrapping protocol serial is at or after `floor`.
///
/// Serials count commits and wrap at `u32::MAX`. Treating the smaller modular distance as the
/// positive direction gives the same ordering across a wrap.
fn serial_at_least(serial: u32, floor: u32) -> bool {
    serial.wrapping_sub(floor) < (1u32 << 31)
}

impl TextInputDataInner {
    /// Starts a fresh batch for a new owner, dropping anything left by the previous one.
    fn enter_generation(&mut self, owner: WindowId) {
        self.clear_pending();
        self.invalidate_generation();
        // text-input-v3 invalidates all state on `enter`: nothing an outgoing acknowledgment
        // carries may be flushed against the new surface before a fresh `enable`/state/`commit`.
        self.reset_outgoing_context();
        self.batch.owner = Some(owner);
    }

    /// Ends the current owner and returns whether it was showing a preedit.
    fn end_owner(&mut self) -> bool {
        let preedit_shown = self.end_incoming_generation();
        self.batch.owner = None;
        self.surface = None;
        // `leave` invalidates the protocol state as well.
        self.reset_outgoing_context();
        preedit_shown
    }

    /// Forgets outgoing state that a new enter/leave invalidated.
    fn reset_outgoing_context(&mut self) {
        self.awaiting = None;
        self.acknowledged = SentContext::Disabled;
        self.pending_change_cause = None;
    }

    /// Stops accepting incoming state and drops pending records.
    ///
    /// The owner and surface stay known so that enabling the same focused surface again does not
    /// require another `enter`. The generation token is invalidated, so events already queued
    /// for it are dropped at delivery.
    fn end_incoming_generation(&mut self) -> bool {
        let preedit_shown = self.clear_pending();
        self.invalidate_generation();
        preedit_shown
    }

    /// Whether the application has been told this text input is enabled.
    fn is_active(&self) -> bool {
        self.incoming_from.is_some()
    }

    /// Prepares a new enable generation; it becomes accepting only after the commit serial is
    /// known. Returns the arm that the matching commit must present.
    fn begin_enable(&mut self) -> u64 {
        self.clear_pending();
        self.invalidate_generation();
        self.arm_epoch = self.arm_epoch.wrapping_add(1);
        self.armed = Some(self.arm_epoch);
        self.arm_epoch
    }

    /// Accepts the enable commit only if no boundary happened since its arm.
    fn finish_enable(&mut self, arm: u64, serial: u32) {
        if self.armed == Some(arm) {
            self.armed = None;
            self.start_incoming_generation(serial);
        }
    }

    /// Starts accepting `done` events at or after the enable commit's serial.
    fn start_incoming_generation(&mut self, serial: u32) {
        self.clear_pending();
        self.invalidate_generation();
        // A fresh token per generation: never compare equal to a previous one.
        self.generation = Some(ImeLifecycleToken::new());
        self.incoming_from = Some(serial);
    }

    /// Stops accepting incoming state and retires queued composition of the generation.
    fn invalidate_generation(&mut self) {
        self.incoming_from = None;
        self.armed = None;
        if let Some(token) = self.generation.take() {
            token.invalidate();
        }
    }

    /// Records an outgoing commit and returns its protocol serial.
    fn request_commit(&mut self) -> u32 {
        self.request_serial = self.request_serial.wrapping_add(1);
        self.request_serial
    }

    /// Sends all state requested by an enable and its single commit.
    ///
    /// Callers must have sent `enable` first: text-input-v3 resets exactly this state on enable.
    fn send_enabled_commit<T: TextInputRequests>(
        &mut self,
        input: &T,
        state: &OutgoingImeState,
        change_cause: Option<ImeTextChangeCause>,
    ) -> u32 {
        // Initial enable, reset, purpose and cursor-only snapshots are application-originated
        // state changes. A coalesced IME receipt keeps its explicit `InputMethod` cause, but
        // every other commit names `Other` instead of inheriting the protocol default.
        send_state_requests(input, state, change_cause.unwrap_or(ImeTextChangeCause::Other));
        let serial = self.request_commit();
        input.commit();
        self.awaiting = Some(SentContext::Enabled(state.clone()));
        self.pending_change_cause = None;
        serial
    }

    /// Publishes a changed desired snapshot or coalesces it behind an unacknowledged commit.
    ///
    /// While `awaiting` is `Some`, the protocol state must not be changed; the newest desired
    /// snapshot is remembered on the window and flushed by the matching `done`. Two setters in
    /// that window therefore produce at most one commit, not one each.
    fn sync_outgoing<T: TextInputRequests>(
        &mut self,
        input: &T,
        desired: &OutgoingImeState,
        change_cause: Option<ImeTextChangeCause>,
    ) -> OutgoingCommit {
        if self.awaiting.is_some() {
            // Keep the newest application cause with the coalesced snapshot. A later cursor-area
            // or purpose update passes `None` and therefore does not erase a surrounding-text
            // receipt already waiting for its `done`. Only a new surrounding-text setter replaces
            // the pending cause.
            if change_cause.is_some() {
                self.pending_change_cause = change_cause;
            }
            return OutgoingCommit::Deferred;
        }

        if self.acknowledged == SentContext::Enabled(desired.clone()) {
            return OutgoingCommit::Quiet;
        }

        match self.acknowledged {
            // An ordinary edit in the same enabled context: one commit, no disable/enable, so a
            // composition in progress is preserved.
            SentContext::Enabled(_) => {
                OutgoingCommit::Sent(self.send_enabled_commit(input, desired, change_cause))
            },
            // Never start a generation from a setter. Only an explicit enable/enter/reset does
            // that; a disabled window must not be re-enabled by a surrounding-text update.
            SentContext::Disabled => OutgoingCommit::Deferred,
        }
    }

    /// Applies the outgoing half of a `done`.
    ///
    /// A mismatched serial is the protocol's "apply incoming changes but do not change current
    /// state": outgoing state stays pending and is not sent. A matching serial acknowledges the
    /// pending commit; if a setter coalesced newer state meanwhile, that state is sent with one
    /// fresh commit.
    fn acknowledge_done<T: TextInputRequests>(
        &mut self,
        done_serial: u32,
        desired: Option<&OutgoingImeState>,
        input: &T,
    ) -> OutgoingCommit {
        let Some(awaiting) = self.awaiting.clone() else { return OutgoingCommit::Quiet };
        if done_serial != self.request_serial {
            return OutgoingCommit::Unmatched;
        }

        self.awaiting = None;
        self.acknowledged = awaiting;
        let Some(desired) = desired else {
            // The window is gone or disabled: acknowledge without sending, and never enable.
            self.pending_change_cause = None;
            return OutgoingCommit::Acknowledged;
        };

        match self.acknowledged.clone() {
            SentContext::Disabled => {
                // The acknowledged commit disabled the input. Do not turn that into an enable.
                self.pending_change_cause = None;
                OutgoingCommit::Acknowledged
            },
            SentContext::Enabled(acknowledged) => {
                if &acknowledged != desired {
                    let change_cause = self.pending_change_cause.take();
                    OutgoingCommit::Sent(self.send_enabled_commit(input, desired, change_cause))
                } else {
                    self.pending_change_cause = None;
                    OutgoingCommit::Acknowledged
                }
            },
        }
    }

    /// Drops pending records but keeps the owner and the generation.
    fn clear_pending(&mut self) -> bool {
        let preedit_shown = std::mem::take(&mut self.preedit_shown);
        self.batch.commit = None;
        self.batch.preedit = None;
        self.batch.delete = None;
        preedit_shown
    }

    fn clear_if_owner(&mut self, owner: WindowId) {
        if self.batch.owner == Some(owner) {
            // Window destruction is not a lifecycle boundary the application can handle: drop
            // the queued events as well, unlike a disable/leave/reset that keeps accepted commits.
            let _ = self.clear_pending();
            self.batch.owner = None;
            self.surface = None;
            self.incoming_from = None;
            self.armed = None;
            self.reset_outgoing_context();
            if let Some(token) = self.generation.take() {
                token.discard();
            }
        }
    }

    fn accepts_incoming(&self) -> bool {
        self.incoming_from.is_some() && self.batch.owner.is_some()
    }

    /// Applies a `preedit_string` record. Ignored unless the current generation is enabled.
    fn preedit_string(&mut self, text: Option<String>, cursor_begin: i32, cursor_end: i32) {
        if !self.accepts_incoming() {
            return;
        }

        let text = text.unwrap_or_default();
        let cursor_begin = usize::try_from(cursor_begin)
            .ok()
            .and_then(|idx| text.is_char_boundary(idx).then_some(idx));
        let cursor_end = usize::try_from(cursor_end)
            .ok()
            .and_then(|idx| text.is_char_boundary(idx).then_some(idx));

        self.batch.preedit = Some(Preedit { text, cursor_begin, cursor_end });
    }

    /// Applies a `commit_string` record. A null string is no commit; an empty string still is.
    ///
    /// `preedit_string` and `commit_string` are independent double-buffered records. Their event
    /// arrival order says nothing about the order in `done`; in particular a commit must not
    /// discard a preedit that arrived before it in the same batch.
    fn commit_string(&mut self, text: Option<String>) {
        if !self.accepts_incoming() {
            return;
        }
        self.batch.commit = text;
    }

    /// Applies a `delete_surrounding_text` record.
    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        if !self.accepts_incoming() {
            return;
        }
        self.batch.delete = Some(ImeDeleteSurrounding {
            before_bytes: before_length as usize,
            after_bytes: after_length as usize,
        });
    }

    /// Whether the text input already accepts `done` or has records for the next one.
    fn has_pending_or_active(&self) -> bool {
        self.accepts_incoming()
            || self.batch.commit.is_some()
            || self.batch.preedit.is_some()
            || self.batch.delete.is_some()
    }

    /// Applies the pending batch in text-input-v3 order.
    ///
    /// `incoming_from` is a lifecycle fence, not an exact-match gate: a `done` serial that is at
    /// or after the enable commit still applies when it differs from the latest commit serial,
    /// as required by text-input-v3. Only a `done` from before the current generation is
    /// discarded.
    fn finish_done(
        &mut self,
        done_serial: u32,
        current_owner: Option<WindowId>,
        events: &mut EventSink,
    ) -> ImeBatchOutcome {
        self.finish_done_with_preedit_policy(done_serial, current_owner, events, true)
    }

    /// Applies `done` while telling the delivery whether this seat owns the visible preedit.
    ///
    /// The public `Ime` stream has one preedit slot for the window. An accepted commit from any
    /// seat ends that window-level composition: it always emits the documented `Preedit("")`
    /// immediately before the commit, and the caller releases the old visible owner. An empty
    /// preedit-only record is different: only the visible owner may clear, so a non-owner's empty
    /// `done` neither clears another seat's newer composition nor claims ownership.
    fn finish_done_with_preedit_policy(
        &mut self,
        done_serial: u32,
        current_owner: Option<WindowId>,
        events: &mut EventSink,
        may_clear_preedit: bool,
    ) -> ImeBatchOutcome {
        let Some(incoming_from) = self.incoming_from else {
            // Disabled/reset: a late `done` must not revive the batch it terminated.
            self.clear_pending();
            return ImeBatchOutcome::default();
        };
        if !serial_at_least(done_serial, incoming_from) {
            // A `done` from before this lifecycle generation. Drop both the records it
            // terminated and the event itself; a later generation's events are still accepted.
            self.clear_pending();
            return ImeBatchOutcome::default();
        }
        let Some(token) = self.generation.clone() else {
            // No generation token means the boundary already ended this batch.
            self.clear_pending();
            return ImeBatchOutcome::default();
        };

        let owner = match self.batch.owner {
            Some(owner) if current_owner == Some(owner) => owner,
            _ => {
                self.end_owner();
                return ImeBatchOutcome::default();
            },
        };

        // The deletion goes out with a commit, so the application applies it after
        // removing the preedit and before inserting the text, as text-input-v3 orders
        // them.
        let delete = self.batch.delete.take();
        if delete.is_some() && self.batch.commit.is_none() {
            self.batch.commit = Some(String::new());
        }

        let commit = self.batch.commit.take();
        let preedit = self.batch.preedit.take();
        let mut outcome = ImeBatchOutcome::default();

        // The clear preceding a commit is part of that accepted transaction, not retractable
        // composition. Keep it paired through a later leave/reset/disable. The public contract is
        // window-level: even a commit from a seat that does not own the visible preedit clears the
        // old composition and releases its owner before the commit is delivered.
        if let Some(text) = commit {
            self.preedit_shown = false;
            outcome.committed = true;
            outcome.composition_ended = true;
            events.push_ime_commit(text, owner, &token, delete);
        }

        match preedit {
            Some(preedit) if !preedit.text.is_empty() => {
                self.preedit_shown = true;
                outcome.preedit_visible = true;
                let cursor_range =
                    preedit.cursor_begin.map(|b| (b, preedit.cursor_end.unwrap_or(b)));
                events.push_ime_event(
                    Ime::Preedit(preedit.text, cursor_range),
                    owner,
                    &token,
                    None,
                );
            },
            Some(_empty) => {
                // `preedit_string("")` or a null string is a clear, not a visible composition.
                // Only the owner may clear the window's one public preedit slot; suppressing the
                // record also suppresses duplicate clears after a commit.
                self.preedit_shown = false;
                if !outcome.committed && may_clear_preedit {
                    outcome.composition_ended = true;
                    events.push_ime_event(Ime::Preedit(String::new(), None), owner, &token, None);
                }
            },
            None if !outcome.committed => {
                self.preedit_shown = false;
                if may_clear_preedit {
                    outcome.composition_ended = true;
                    events.push_ime_event(Ime::Preedit(String::new(), None), owner, &token, None);
                }
            },
            None => {},
        }

        outcome
    }
}

/// The state of the preedit.
struct Preedit {
    text: String,
    cursor_begin: Option<usize>,
    cursor_end: Option<usize>,
}

delegate_dispatch!(WinitState: [ZwpTextInputManagerV3: GlobalData] => TextInputState);
delegate_dispatch!(WinitState: [ZwpTextInputV3: TextInputData] => TextInputState);

/// The longest surrounding text text-input-v3 can carry.
const MAX_SURROUNDING_TEXT_BYTES: usize = 4000;

/// Clamps `cursor` and `anchor` to `text` and moves them back to character boundaries, then cuts
/// text longer than [`MAX_SURROUNDING_TEXT_BYTES`] to a window around the cursor and the
/// selection, or around the cursor alone when the selection doesn't fit. Returns the text and the
/// cursor and anchor offsets in it.
pub fn surrounding_text_excerpt(text: &str, cursor: usize, anchor: usize) -> (&str, usize, usize) {
    let cursor = floor_char_boundary(text, cursor);
    let anchor = floor_char_boundary(text, anchor);
    if text.len() <= MAX_SURROUNDING_TEXT_BYTES {
        return (text, cursor, anchor);
    }

    let (low, high) = (cursor.min(anchor), cursor.max(anchor));
    let (low, high) =
        if high - low <= MAX_SURROUNDING_TEXT_BYTES { (low, high) } else { (cursor, cursor) };
    // Split the spare bytes evenly on both sides, giving one side's unused share to the other.
    let spare = MAX_SURROUNDING_TEXT_BYTES - (high - low);
    let end = (low.saturating_sub(spare / 2) + MAX_SURROUNDING_TEXT_BYTES).min(text.len());
    let start = ceil_char_boundary(text, end - MAX_SURROUNDING_TEXT_BYTES);
    let end = floor_char_boundary(text, end);

    let anchor = anchor.clamp(start, end);
    (&text[start..end], cursor - start, anchor - start)
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{
        context_reset_needed, disable_generation, enable_generation, next_sequence,
        push_ime_teardown, reset_generation, reset_inputs_with_fence, serial_at_least,
        surrounding_text_excerpt, OutgoingCommit, OutgoingImeState, SentContext,
        TextInputDataInner, TextInputRequests, MAX_SURROUNDING_TEXT_BYTES as MAX,
    };
    use crate::event::{Event, Ime, WindowEvent};
    use crate::platform::wayland::{ImeDeleteSurrounding, ImeTextChangeCause};
    use crate::platform_impl::wayland::event_loop::sink::EventSink;
    use crate::platform_impl::wayland::window::state::{
        ImeBatchOutcome, ImePreeditOwnership, TextInputLeft,
    };
    use crate::platform_impl::wayland::WindowId;
    use crate::window::ImePurpose;

    fn owner(id: u64) -> WindowId {
        WindowId::from(id)
    }

    /// Delivers the queue through the same validity check the event loop uses.
    fn deliveries(sink: &mut EventSink) -> Vec<(WindowId, Ime, Option<ImeDeleteSurrounding>)> {
        sink.drain_deliverable()
            .into_iter()
            .filter_map(|(event, delete)| match event {
                Event::WindowEvent { window_id, event: WindowEvent::Ime(ime) } => {
                    Some((window_id.0, ime, delete))
                },
                _ => None,
            })
            .collect()
    }

    fn ime_events(sink: &mut EventSink) -> Vec<(WindowId, Ime)> {
        deliveries(sink).into_iter().map(|(window, ime, _)| (window, ime)).collect()
    }

    /// Delivers the two production sinks through the event loop's ordered merge.
    fn merged_deliveries(
        protocol_events: &mut EventSink,
        window_events: &mut EventSink,
    ) -> Vec<(WindowId, Ime, Option<ImeDeleteSurrounding>)> {
        let mut merged = EventSink::new();
        merged.append_ordered(protocol_events);
        merged.append_ordered(window_events);
        deliveries(&mut merged)
    }

    #[test]
    fn short_text_is_kept_and_offsets_are_clamped() {
        assert_eq!(surrounding_text_excerpt("foobar", 3, 3), ("foobar", 3, 3));
        assert_eq!(surrounding_text_excerpt("foobar", 1, 4), ("foobar", 1, 4));
        assert_eq!(surrounding_text_excerpt("foo", 10, 99), ("foo", 3, 3));
        // `и` is two bytes; an offset inside it moves back to its start.
        assert_eq!(surrounding_text_excerpt("граница", 1, 5), ("граница", 0, 4));
    }

    #[test]
    fn long_text_is_cut_around_the_cursor() {
        let text = "a".repeat(10_000);
        let (excerpt, cursor, anchor) = surrounding_text_excerpt(&text, 5_000, 5_000);
        assert_eq!(excerpt.len(), MAX);
        assert_eq!((cursor, anchor), (MAX / 2, MAX / 2));

        // Near either end, the unused share goes to the other side.
        assert_eq!(surrounding_text_excerpt(&text, 100, 100), (&text[..MAX], 100, 100));
        let (excerpt, cursor, _) = surrounding_text_excerpt(&text, 9_900, 9_900);
        assert_eq!((excerpt.len(), cursor), (MAX, MAX - 100));
    }

    #[test]
    fn long_text_keeps_the_selection() {
        let text = "a".repeat(10_000);
        let (excerpt, cursor, anchor) = surrounding_text_excerpt(&text, 6_000, 3_000);
        assert_eq!(excerpt.len(), MAX);
        assert_eq!(cursor - anchor, 3_000);
        assert_eq!(anchor, 500);

        // A selection longer than the limit keeps the cursor; the anchor stops at the cut.
        let (excerpt, cursor, anchor) = surrounding_text_excerpt(&text, 9_000, 1_000);
        assert_eq!(excerpt.len(), MAX);
        assert_eq!((cursor, anchor), (3_000, 0));
    }

    #[test]
    fn long_text_is_cut_on_char_boundaries() {
        // Three-byte characters: 4000 isn't a multiple of 3, so the cut must move inward.
        let text = "ấ".repeat(3_000);
        for cursor in [0, 4_500, 4_501, 9_000] {
            let (excerpt, cursor_in, anchor_in) = surrounding_text_excerpt(&text, cursor, cursor);
            assert!(excerpt.len() <= MAX && excerpt.len() > MAX - 6, "{cursor}");
            assert!(excerpt.chars().all(|c| c == 'ấ'), "{cursor}: cut inside a character");
            assert_eq!(cursor_in % 3, 0, "{cursor}");
            assert_eq!(cursor_in, anchor_in);
        }
    }

    /// The data state after a production enable: owner, enable arm and its commit.
    fn enabled(owner: WindowId) -> TextInputDataInner {
        let mut data = TextInputDataInner::default();
        data.enter_generation(owner);
        let arm = data.begin_enable();
        let serial = data.request_commit();
        data.finish_enable(arm, serial);
        data
    }

    /// The production `set_ime_allowed(true)` path for the same focused surface.
    fn re_enable(data: &mut TextInputDataInner) {
        let arm = data.begin_enable();
        let serial = data.request_commit();
        data.finish_enable(arm, serial);
    }

    fn done_serial(data: &TextInputDataInner) -> u32 {
        data.incoming_from.expect("the test data has an enabled incoming generation")
    }

    #[test]
    fn done_applies_delete_commit_and_new_preedit_in_protocol_order() {
        let window = owner(1);
        let delete = ImeDeleteSurrounding { before_bytes: 2, after_bytes: 1 };
        let mut data = enabled(window);
        data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
        data.commit_string(Some("界".to_owned()));
        data.preedit_string(Some("next".to_owned()), 1, 4);
        let mut sink = EventSink::new();

        data.finish_done(done_serial(&data), Some(window), &mut sink);

        // The deletion travels on the commit event itself.
        assert_eq!(deliveries(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None), None),
            (window, Ime::Commit("界".to_owned()), Some(delete)),
            (window, Ime::Preedit("next".to_owned(), Some((1, 4))), None),
        ]);
    }

    #[test]
    fn deletion_without_commit_becomes_an_empty_commit() {
        let window = owner(1);
        let delete = ImeDeleteSurrounding { before_bytes: 3, after_bytes: 0 };
        let mut data = enabled(window);
        data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
        let mut sink = EventSink::new();

        data.finish_done(done_serial(&data), Some(window), &mut sink);

        assert_eq!(deliveries(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None), None),
            (window, Ime::Commit(String::new()), Some(delete)),
        ]);
    }

    #[test]
    fn preedit_only_update_is_sent_without_a_clear() {
        let window = owner(1);
        let mut data = enabled(window);
        data.preedit_string(Some("a".to_owned()), 1, 1);
        let mut sink = EventSink::new();

        data.finish_done(done_serial(&data), Some(window), &mut sink);

        assert_eq!(deliveries(&mut sink), vec![(
            window,
            Ime::Preedit("a".to_owned(), Some((1, 1))),
            None
        )]);
        assert!(data.preedit_shown);
    }

    #[test]
    fn null_commit_is_not_inserted_but_an_empty_commit_is() {
        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.commit_string(None);
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert!(
            !ime_events(&mut sink).iter().any(|(_, event)| matches!(event, Ime::Commit(_))),
            "a null commit_string was delivered as text"
        );

        data.commit_string(Some(String::new()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert!(
            ime_events(&mut sink).contains(&(window, Ime::Commit(String::new()))),
            "an explicitly empty commit was lost"
        );
    }

    #[test]
    fn commit_and_preedit_pending_in_either_order_are_both_applied() {
        // `preedit_string` and `commit_string` are independent double-buffered records. A
        // `commit_string` must not discard a preedit recorded before it, and a null commit is
        // merely "no commit", not an instruction to clear the preedit.
        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.preedit_string(Some("first".to_owned()), 1, 2);
        data.commit_string(Some("x".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(deliveries(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None), None),
            (window, Ime::Commit("x".to_owned()), None),
            (window, Ime::Preedit("first".to_owned(), Some((1, 2))), None),
        ]);

        data.commit_string(Some("y".to_owned()));
        data.preedit_string(Some("second".to_owned()), 0, 1);
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(deliveries(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None), None),
            (window, Ime::Commit("y".to_owned()), None),
            (window, Ime::Preedit("second".to_owned(), Some((0, 1))), None),
        ]);

        data.commit_string(None);
        data.preedit_string(Some("third".to_owned()), 0, 0);
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(
            deliveries(&mut sink),
            vec![(window, Ime::Preedit("third".to_owned(), Some((0, 0))), None)],
            "a null commit discarded an independently buffered preedit"
        );
    }

    #[test]
    fn mismatched_done_serial_still_applies_within_the_same_generation() {
        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.commit_string(Some("first".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);

        // The app sends more outgoing commits (for example cursor area or surrounding text)
        // after the input method produced this batch. The `done` serial is then older than the
        // latest commit but still belongs to this generation, so it must apply.
        let older_but_current = data.request_commit();
        let latest = data.request_commit();
        assert_ne!(older_but_current, latest);
        data.commit_string(Some("second".to_owned()));
        data.finish_done(older_but_current, Some(window), &mut sink);

        assert_eq!(ime_events(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None)),
            (window, Ime::Commit("first".to_owned())),
            (window, Ime::Preedit(String::new(), None)),
            (window, Ime::Commit("second".to_owned())),
        ]);
    }

    #[test]
    fn disable_drops_the_batch_and_late_events_until_the_next_enable() {
        let window = owner(1);
        let delete = ImeDeleteSurrounding { before_bytes: 2, after_bytes: 1 };
        let mut data = enabled(window);

        // A composition is visible when the window disables IME.
        data.preedit_string(Some("old".to_owned()), 0, 3);
        let mut sink = EventSink::new();
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(ime_events(&mut sink).len(), 1);

        // The disable boundary is what `set_ime_allowed(false)` uses. Buffered commit,
        // preedit and deletion records must not survive it.
        data.commit_string(Some("stale".to_owned()));
        data.preedit_string(Some("late".to_owned()), 0, 4);
        data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
        assert!(data.end_incoming_generation());

        // In-flight records and their `done` arrive after the Disabled boundary.
        data.commit_string(Some("in flight".to_owned()));
        data.preedit_string(Some("in flight".to_owned()), 0, 9);
        data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
        let disable_serial = data.request_commit();
        data.finish_done(disable_serial, Some(window), &mut sink);
        data.finish_done(disable_serial.wrapping_add(1), Some(window), &mut sink);

        assert!(ime_events(&mut sink).is_empty(), "late IME events followed Disabled");

        // The next enable starts a fresh generation on the same focused owner.
        re_enable(&mut data);
        data.commit_string(Some("new".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);

        assert_eq!(ime_events(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None)),
            (window, Ime::Commit("new".to_owned())),
        ]);
    }

    #[test]
    fn disable_retires_composition_but_delivers_an_accepted_commit() {
        // A `Commit` that was accepted from `done` before the disable is committed input. The
        // clear-preedit belongs to that accepted transaction, so it survives with the commit
        // and its deletion. A later ordinary composition does not.
        let window = owner(1);
        let delete = ImeDeleteSurrounding { before_bytes: 1, after_bytes: 0 };
        let mut data = enabled(window);
        let mut sink = EventSink::new();
        data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
        data.commit_string(Some("typed".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(sink.queued_len(), 2);

        // `set_ime_allowed(false)`.
        data.end_incoming_generation();

        assert_eq!(
            deliveries(&mut sink),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("typed".to_owned()), Some(delete))
            ],
            "an accepted commit was dropped or duplicated by the disable boundary"
        );
    }

    #[test]
    fn reset_delivers_an_accepted_commit_but_drops_its_composition() {
        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.commit_string(Some("old".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);

        // `reset_ime`: disable, then enable again on the same surface.
        data.end_incoming_generation();
        let _ = data.request_commit();
        re_enable(&mut data);
        data.commit_string(Some("new".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);

        assert_eq!(ime_events(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None)),
            (window, Ime::Commit("old".to_owned())),
            (window, Ime::Preedit(String::new(), None)),
            (window, Ime::Commit("new".to_owned())),
        ]);
    }

    #[test]
    fn reset_drops_old_generation_but_keeps_the_new_one() {
        let window = owner(1);
        let mut data = enabled(window);
        let old_floor = done_serial(&data);
        let mut sink = EventSink::new();

        data.preedit_string(Some("compose".to_owned()), 0, 7);
        data.finish_done(old_floor, Some(window), &mut sink);
        assert_eq!(ime_events(&mut sink), vec![(
            window,
            Ime::Preedit("compose".to_owned(), Some((0, 7)))
        )]);

        // This is `reset_ime`: disable, commit, enable, commit.
        data.commit_string(Some("old generation".to_owned()));
        assert!(data.end_incoming_generation());
        let _disable_serial = data.request_commit();
        re_enable(&mut data);

        // A done from the disabled generation is older than the enable commit and must be
        // ignored even though the done serial does not equal the latest commit serial.
        data.finish_done(old_floor, Some(window), &mut sink);
        assert!(ime_events(&mut sink).is_empty());
        assert_eq!(sink.queued_len(), 0);

        // The same generation accepts a done that is newer than the enable commit but still
        // mismatched against a later outgoing commit.
        let later_commit = data.request_commit();
        data.commit_string(Some("new generation".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(ime_events(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None)),
            (window, Ime::Commit("new generation".to_owned())),
        ]);
        assert_ne!(done_serial(&data), later_commit);
    }

    #[test]
    fn lifecycle_fence_compares_wrapping_commit_serials() {
        assert!(serial_at_least(0, u32::MAX - 1));
        assert!(serial_at_least(u32::MAX, u32::MAX - 1));
        assert!(!serial_at_least(u32::MAX - 5, u32::MAX - 1));

        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        // An enable commit near the wrap point; the first `done` after it wraps to 0.
        let floor = u32::MAX - 1;
        data.start_incoming_generation(floor);
        data.commit_string(Some("wrapped".to_owned()));
        data.finish_done(0, Some(window), &mut sink);
        assert_eq!(ime_events(&mut sink), vec![
            (window, Ime::Preedit(String::new(), None)),
            (window, Ime::Commit("wrapped".to_owned())),
        ]);

        data.commit_string(Some("before the fence".to_owned()));
        data.finish_done(u32::MAX - 3, Some(window), &mut sink);
        assert!(ime_events(&mut sink).is_empty());
    }

    #[test]
    fn disable_keeps_the_focused_owner_for_the_next_enable() {
        let window = owner(1);
        let mut data = enabled(window);
        data.end_incoming_generation();

        // The same surface stays focused; `set_ime_allowed(true)` can send a new enable commit
        // without waiting for another `enter`.
        assert_eq!(data.batch.owner, Some(window));
        assert!(!data.accepts_incoming());

        let mut sink = EventSink::new();
        data.commit_string(Some("ignored".to_owned()));
        data.finish_done(7, Some(window), &mut sink);
        assert!(ime_events(&mut sink).is_empty());

        re_enable(&mut data);
        assert!(data.accepts_incoming());
    }

    #[test]
    fn leave_keeps_the_accepted_commit_but_drops_composition_and_owner() {
        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.commit_string(Some("old".to_owned()));
        data.preedit_string(Some("old".to_owned()), 0, 3);
        data.delete_surrounding_text(1, 1);
        data.preedit_shown = true;
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(sink.queued_len(), 3);

        let done_serial = done_serial(&data);
        assert!(data.end_owner());
        data.finish_done(done_serial, Some(window), &mut sink);

        assert_eq!(
            deliveries(&mut sink),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (
                    window,
                    Ime::Commit("old".to_owned()),
                    Some(ImeDeleteSurrounding { before_bytes: 1, after_bytes: 1 })
                )
            ],
            "the committed text vanished with the composition"
        );
        assert_eq!(data.batch.owner, None);
        assert!(!data.accepts_incoming());
    }

    #[test]
    fn enter_drops_the_previous_owner_batch() {
        let old = owner(1);
        let new = owner(2);
        let mut data = enabled(old);
        data.commit_string(Some("old".to_owned()));
        data.delete_surrounding_text(1, 0);

        data.enter_generation(new);
        assert_eq!(data.batch.owner, Some(new));
        assert!(data.batch.commit.is_none());
        assert!(data.batch.delete.is_none());
        assert!(!data.accepts_incoming());

        // Events cannot be applied between `enter` and the enable commit.
        let mut sink = EventSink::new();
        data.commit_string(Some("too early".to_owned()));
        data.finish_done(1, Some(new), &mut sink);
        assert!(ime_events(&mut sink).is_empty());
    }

    #[test]
    fn owner_mismatch_drops_a_done_even_with_a_current_serial() {
        let old = owner(1);
        let new = owner(2);
        let mut data = enabled(old);
        data.commit_string(Some("stale".to_owned()));
        let mut sink = EventSink::new();

        data.finish_done(done_serial(&data), Some(new), &mut sink);

        assert!(ime_events(&mut sink).is_empty());
        assert_eq!(data.batch.owner, None);
    }

    #[test]
    fn deletions_are_routed_by_their_commit_event() {
        let first = owner(1);
        let second = owner(2);
        let first_delete = ImeDeleteSurrounding { before_bytes: 1, after_bytes: 0 };
        let second_delete = ImeDeleteSurrounding { before_bytes: 0, after_bytes: 2 };
        let mut sink = EventSink::new();

        let mut first_input = enabled(first);
        first_input.commit_string(Some("a".to_owned()));
        first_input.delete_surrounding_text(
            first_delete.before_bytes as u32,
            first_delete.after_bytes as u32,
        );
        first_input.finish_done(done_serial(&first_input), Some(first), &mut sink);

        let mut second_input = enabled(second);
        second_input.commit_string(Some("b".to_owned()));
        second_input.delete_surrounding_text(
            second_delete.before_bytes as u32,
            second_delete.after_bytes as u32,
        );
        second_input.finish_done(done_serial(&second_input), Some(second), &mut sink);

        // Interleaved commits carry their own deletion; neither can consume the other's.
        let commits: Vec<_> = deliveries(&mut sink)
            .into_iter()
            .filter(|(_, event, _)| matches!(event, Ime::Commit(_)))
            .collect();
        assert_eq!(commits, vec![
            (first, Ime::Commit("a".to_owned()), Some(first_delete)),
            (second, Ime::Commit("b".to_owned()), Some(second_delete)),
        ]);
    }

    #[test]
    fn one_seats_disable_does_not_drop_another_seats_events_or_its_own_commit() {
        // Each seat's text input has its own generation token, so a bound seat does not silence
        // the seat that still has IME. The bound seat's accepted commit is still committed text.
        let first = owner(1);
        let second = owner(2);
        let mut first_input = enabled(first);
        let mut second_input = enabled(second);
        let mut sink = EventSink::new();

        first_input.commit_string(Some("first".to_owned()));
        first_input.finish_done(done_serial(&first_input), Some(first), &mut sink);
        second_input.commit_string(Some("second".to_owned()));
        second_input.finish_done(done_serial(&second_input), Some(second), &mut sink);

        // `set_ime_allowed(false)` for the first window.
        first_input.end_incoming_generation();

        assert_eq!(ime_events(&mut sink), vec![
            (first, Ime::Preedit(String::new(), None)),
            (first, Ime::Commit("first".to_owned())),
            (second, Ime::Preedit(String::new(), None)),
            (second, Ime::Commit("second".to_owned())),
        ]);
    }

    #[test]
    fn window_destruction_clears_only_the_owning_batch() {
        let window = owner(1);
        let other = owner(2);
        let mut data = enabled(window);
        data.commit_string(Some("old".to_owned()));
        data.preedit_shown = true;
        let mut sink = EventSink::new();
        data.finish_done(done_serial(&data), Some(window), &mut sink);

        data.clear_if_owner(other);
        assert_eq!(data.batch.owner, Some(window));
        assert!(data.accepts_incoming());
        assert_eq!(sink.queued_len(), 2, "another window's destruction dropped the event");

        data.clear_if_owner(window);
        assert_eq!(data.batch.owner, None);
        assert!(!data.accepts_incoming());
        assert!(
            deliveries(&mut sink).is_empty(),
            "the destroyed window's queued IME events were delivered"
        );
    }

    #[test]
    fn closed_window_tombstone_drops_queued_ime_before_delivery() {
        // The queue is checked immediately before delivery. This covers the accepted commit
        // whose text input has already been destroyed with its seat (the token no longer has a
        // registry to clear it), and the untagged synthetic Enabled/clear events as well.
        let closed = owner(1);
        let live = owner(2);
        let mut data = enabled(closed);
        let mut protocol_events = EventSink::new();
        data.commit_string(Some("accepted".to_owned()));
        data.finish_done(done_serial(&data), Some(closed), &mut protocol_events);
        data.end_owner();
        drop(data); // The seat was removed before the window closed.

        let mut window_events = EventSink::new();
        window_events.push_window_event_at(next_sequence(), WindowEvent::Ime(Ime::Enabled), closed);
        window_events.push_window_event_at(
            next_sequence(),
            WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            closed,
        );

        let mut live_data = enabled(live);
        live_data.commit_string(Some("live".to_owned()));
        live_data.finish_done(done_serial(&live_data), Some(live), &mut protocol_events);

        let mut merged = EventSink::new();
        merged.append_ordered(&mut protocol_events);
        merged.append_ordered(&mut window_events);
        let delivered = merged.drain_deliverable_for(|window| window == closed);

        assert_eq!(
            delivered
                .into_iter()
                .filter_map(|(event, delete)| match event {
                    Event::WindowEvent { window_id, event: WindowEvent::Ime(ime) } => {
                        Some((window_id.0, ime, delete))
                    },
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![
                (live, Ime::Preedit(String::new(), None), None),
                (live, Ime::Commit("live".to_owned()), None),
            ],
            "a closed window's queued IME events survived the pre-delivery tombstone"
        );
    }

    #[test]
    fn seat_removal_keeps_the_accepted_commit_for_teardown_delivery() {
        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();
        data.commit_string(Some("seat".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);

        // `remove_seat` detaches the proxy under the window lock and then ends the owner before
        // destroying the object.
        data.end_owner();

        assert_eq!(
            deliveries(&mut sink),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("seat".to_owned()), None)
            ],
            "a removed seat's accepted commit was dropped"
        );
    }

    #[test]
    fn distinct_generations_never_share_a_token() {
        let window = owner(1);
        let mut data = enabled(window);
        let first = data.generation.clone().unwrap();
        data.end_incoming_generation();
        assert!(!first.is_valid());

        re_enable(&mut data);
        let second = data.generation.clone().unwrap();
        assert!(second.is_valid());
        assert!(!first.is_same_generation(&second), "a generation token was reused");
    }

    #[test]
    fn a_stale_enable_commit_cannot_restart_a_generation_after_a_boundary() {
        let window = owner(1);
        let mut data = enabled(window);

        // An enable is armed, then a concurrent disable/reset ends the generation before the
        // enable's commit is accounted for.
        let arm = data.begin_enable();
        data.end_incoming_generation();
        let serial = data.request_commit();
        data.finish_enable(arm, serial);

        assert!(!data.accepts_incoming(), "a raced enable commit restarted acceptance");
        assert!(data.generation.is_none());
    }

    #[test]
    fn reset_and_dispatch_workload_emits_a_stable_operation_count() {
        // Deterministic replayable workload for the production batch state: buffer all three
        // incoming records, dispatch, then reset before the next batch. Each iteration must
        // queue exactly three IME events and deliver exactly one commit with its deletion.
        let window = owner(1);
        let delete = ImeDeleteSurrounding { before_bytes: 1, after_bytes: 0 };
        let iterations: usize = 1_000;
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        for serial in 0..iterations {
            data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
            data.commit_string(Some(serial.to_string()));
            data.preedit_string(Some("p".to_owned()), 0, 1);
            data.finish_done(done_serial(&data), Some(window), &mut sink);
            assert_eq!(sink.queued_len(), 3);

            let delivered = deliveries(&mut sink);
            assert_eq!(delivered.len(), 3);
            assert_eq!(
                delivered
                    .iter()
                    .filter(|(_, event, deletion)| matches!(event, Ime::Commit(_))
                        && *deletion == Some(delete))
                    .count(),
                1
            );

            // reset_ime: disable commit, then an enable commit starts the next generation.
            assert!(data.end_incoming_generation());
            let _ = data.request_commit();
            re_enable(&mut data);
        }

        assert_eq!(data.batch.owner, Some(window));
    }

    #[test]
    fn deletion_is_consumed_by_its_own_commit() {
        let window = owner(1);
        let delete = ImeDeleteSurrounding { before_bytes: 2, after_bytes: 0 };
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
        data.commit_string(Some("a".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        data.commit_string(Some("b".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut sink);

        let deletions: Vec<_> = deliveries(&mut sink)
            .into_iter()
            .filter(|(_, event, _)| matches!(event, Ime::Commit(_)))
            .map(|(_, _, deletion)| deletion)
            .collect();
        assert_eq!(deletions, vec![Some(delete), None]);
    }

    #[test]
    fn same_batch_commit_then_leave_delivers_commit_once_before_disabled() {
        // `commit_string`+`done` are accepted, then `leave` arrives in the same dispatch batch.
        // The accepted commit is application input and must not be retracted by the leave. The
        // clear/`Disabled` boundary is queued in the other production sink, so this also checks
        // the ordered merge rather than only the incoming batch.
        let window = owner(1);
        let delete = ImeDeleteSurrounding { before_bytes: 2, after_bytes: 1 };
        let mut data = enabled(window);
        let mut protocol_events = EventSink::new();
        let mut window_events = EventSink::new();

        data.delete_surrounding_text(delete.before_bytes as u32, delete.after_bytes as u32);
        data.commit_string(Some("typed".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut protocol_events);

        let (preedit_shown, was_active) = {
            let was_active = data.is_active();
            (data.end_owner(), was_active)
        };
        assert!(was_active);
        push_ime_teardown(
            &mut window_events,
            window,
            TextInputLeft::Last,
            preedit_shown,
            was_active,
        );

        assert_eq!(
            merged_deliveries(&mut protocol_events, &mut window_events),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("typed".to_owned()), Some(delete)),
                (window, Ime::Disabled, None),
            ],
            "leave lost or duplicated committed text, or put Disabled before it"
        );
    }

    #[test]
    fn disable_delivers_a_queued_commit_and_rejects_later_protocol_records() {
        let window = owner(1);
        let mut data = enabled(window);
        let old_serial = done_serial(&data);
        let mut protocol_events = EventSink::new();
        let mut window_events = EventSink::new();

        data.commit_string(Some("kept".to_owned()));
        data.finish_done(old_serial, Some(window), &mut protocol_events);

        // `set_ime_allowed(false)`: retire the generation and queue the synthetic boundary in the
        // window sink.
        let _ = data.end_incoming_generation();
        window_events.push_window_event(WindowEvent::Ime(Ime::Disabled), window);

        // Records and `done` that were still in flight are from the disabled generation.
        data.commit_string(Some("late".to_owned()));
        data.delete_surrounding_text(1, 0);
        data.preedit_string(Some("late".to_owned()), 0, 4);
        data.finish_done(old_serial, Some(window), &mut protocol_events);
        data.finish_done(old_serial.wrapping_add(1), Some(window), &mut protocol_events);

        assert_eq!(
            merged_deliveries(&mut protocol_events, &mut window_events),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("kept".to_owned()), None),
                (window, Ime::Disabled, None),
            ],
            "disable dropped an accepted commit or delivered a record from the old generation"
        );
    }

    #[test]
    fn reset_delivers_accepted_commits_before_its_clear_and_fences_old_serials() {
        let window = owner(1);
        let mut data = enabled(window);
        let old_serial = done_serial(&data);
        let mut protocol_events = EventSink::new();
        let mut window_events = EventSink::new();

        // One accepted commit, then a preedit the reset will cancel.
        data.commit_string(Some("committed".to_owned()));
        data.finish_done(old_serial, Some(window), &mut protocol_events);
        data.preedit_string(Some("composing".to_owned()), 0, 9);
        data.finish_done(old_serial, Some(window), &mut protocol_events);
        assert!(data.preedit_shown);

        // Native reset: disable/commit then enable/state/commit.
        let recorder = Recorder::default();
        let state = OutgoingImeState::new(ImePurpose::Normal, None, None);
        let (was_shown, _, sequence) = reset_generation(&recorder, &mut data, &state);
        assert!(was_shown);
        // `Window::reset_ime` queues the synthetic clear in the window sink.
        window_events.push_window_event_at(
            sequence,
            WindowEvent::Ime(Ime::Preedit(String::new(), None)),
            window,
        );

        // A `done` from the disabled generation is older than the re-enable commit.
        data.commit_string(Some("late old".to_owned()));
        data.finish_done(old_serial, Some(window), &mut protocol_events);

        // The new generation is accepted.
        data.commit_string(Some("after".to_owned()));
        data.finish_done(done_serial(&data), Some(window), &mut protocol_events);

        assert_eq!(
            merged_deliveries(&mut protocol_events, &mut window_events),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("committed".to_owned()), None),
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("after".to_owned()), None),
            ],
            "reset lost an accepted commit, delivered stale composition, or reordered its clear"
        );
    }

    #[test]
    fn leave_from_one_of_two_seats_does_not_disable_the_window() {
        let window = owner(1);
        let mut events = EventSink::new();

        // A non-owner leaves while another seat's newer preedit owns the window.
        push_ime_teardown(&mut events, window, TextInputLeft::Remaining, false, true);
        assert!(events.is_empty(), "one seat's leave disabled IME for the other seat");

        // The owner leaves while another seat remains entered: clear only its preedit.
        push_ime_teardown(&mut events, window, TextInputLeft::Remaining, true, true);
        assert_eq!(
            ime_events(&mut events),
            vec![(window, Ime::Preedit(String::new(), None))],
            "the preedit owner's leave did not clear its composition"
        );

        // A proxy the window never tracked can't produce a lifecycle boundary.
        push_ime_teardown(&mut events, window, TextInputLeft::NotTracked, false, true);
        assert!(events.is_empty(), "leave emitted Disabled for an untracked text input");

        // The last input was never enabled, so the application was not told IME is on.
        push_ime_teardown(&mut events, window, TextInputLeft::Last, false, false);
        assert!(events.is_empty(), "Disabled was emitted without an Enabled");

        push_ime_teardown(&mut events, window, TextInputLeft::Last, true, true);
        assert_eq!(
            ime_events(&mut events),
            vec![(window, Ime::Preedit(String::new(), None)), (window, Ime::Disabled),],
            "the last seat's leave didn't clear the preedit before Disabled"
        );
    }

    #[test]
    fn two_seat_preedit_owner_controls_the_window_clear() {
        // Seat 1 shows a preedit, then seat 2's newer preedit takes ownership. Seat 1 leaving
        // must not emit a clear that would erase seat 2's composition.
        let window = owner(1);
        let mut owners = ImePreeditOwnership::<u32>::default();
        owners.claim(&1);
        owners.claim(&2);
        let mut events = EventSink::new();

        let clear_seat_one = owners.release(&1);
        push_ime_teardown(&mut events, window, TextInputLeft::Remaining, clear_seat_one, true);
        assert!(events.is_empty(), "a leaving non-owner cleared the new owner's preedit");
        assert!(owners.owns(&2));

        // The owner's leave clears even though another seat is still entered.
        let clear_seat_two = owners.release(&2);
        push_ime_teardown(&mut events, window, TextInputLeft::Remaining, clear_seat_two, true);
        assert_eq!(
            ime_events(&mut events),
            vec![(window, Ime::Preedit(String::new(), None))],
            "the preedit owner's leave did not clear its composition"
        );
    }

    #[test]
    fn non_owner_empty_preedit_does_not_clear_or_steal() {
        // Seat 2 owns the window's visible preedit. A non-owner's null or empty preedit record
        // must not emit the window-wide clear or claim ownership, because `Ime::Preedit` has one
        // slot for the window rather than per-seat identity.
        let window = owner(1);
        let mut owners = ImePreeditOwnership::<u32>::default();
        owners.claim(&2);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.preedit_string(None, -1, -1);
        let outcome = data.finish_done_with_preedit_policy(
            done_serial(&data),
            Some(window),
            &mut sink,
            owners.may_clear(&1),
        );
        owners.after_done(&1, outcome);
        assert!(sink.is_empty(), "a non-owner null preedit cleared the newer preedit");
        assert_eq!(outcome, ImeBatchOutcome::default());
        assert!(owners.owns(&2));

        data.preedit_string(Some(String::new()), 0, 0);
        let outcome = data.finish_done_with_preedit_policy(
            done_serial(&data),
            Some(window),
            &mut sink,
            owners.may_clear(&1),
        );
        owners.after_done(&1, outcome);
        assert!(sink.is_empty(), "a non-owner empty preedit cleared the newer preedit");
        assert!(owners.owns(&2), "a non-owner empty preedit stole ownership");
    }

    #[test]
    fn owner_empty_preedit_emits_one_clear_and_releases() {
        let window = owner(1);
        let mut owners = ImePreeditOwnership::<u32>::default();
        owners.claim(&1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.preedit_string(Some(String::new()), 0, 0);
        let outcome = data.finish_done_with_preedit_policy(
            done_serial(&data),
            Some(window),
            &mut sink,
            owners.may_clear(&1),
        );
        owners.after_done(&1, outcome);

        assert_eq!(
            deliveries(&mut sink),
            vec![(window, Ime::Preedit(String::new(), None), None)],
            "the owner's empty preedit did not emit exactly one clear"
        );
        assert!(!owners.has_owner(), "the owner's clear did not release ownership");
        assert!(outcome.composition_ended && !outcome.committed);
    }

    #[test]
    fn non_owner_commit_always_clears_and_releases_the_visible_owner() {
        // Public composition is window-level: an accepted commit from any seat ends the old
        // visible composition and keeps `Ime::Commit`'s clear-before-commit contract. It must not
        // leave the old seat listed as the owner of text that is no longer visible.
        let window = owner(1);
        let mut owners = ImePreeditOwnership::<u32>::default();
        owners.claim(&2);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.commit_string(Some("from seat one".to_owned()));
        let outcome = data.finish_done_with_preedit_policy(
            done_serial(&data),
            Some(window),
            &mut sink,
            owners.may_clear(&1),
        );
        owners.after_done(&1, outcome);

        assert_eq!(
            deliveries(&mut sink),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("from seat one".to_owned()), None),
            ],
            "a non-owner commit omitted the public clear-before-commit contract"
        );
        assert!(outcome.committed && outcome.composition_ended);
        assert!(!owners.has_owner(), "a non-owner commit left the old preedit owner");
    }

    #[test]
    fn commit_then_new_preedit_clears_then_claims_for_the_committing_seat() {
        let window = owner(1);
        let mut owners = ImePreeditOwnership::<u32>::default();
        owners.claim(&2);
        let mut data = enabled(window);
        let mut sink = EventSink::new();

        data.commit_string(Some("typed".to_owned()));
        data.preedit_string(Some("next".to_owned()), 0, 4);
        let outcome = data.finish_done_with_preedit_policy(
            done_serial(&data),
            Some(window),
            &mut sink,
            owners.may_clear(&1),
        );
        owners.after_done(&1, outcome);

        assert_eq!(
            deliveries(&mut sink),
            vec![
                (window, Ime::Preedit(String::new(), None), None),
                (window, Ime::Commit("typed".to_owned()), None),
                (window, Ime::Preedit("next".to_owned(), Some((0, 4))), None),
            ],
            "commit and its new preedit were not clear, commit, preedit"
        );
        assert!(owners.owns(&1), "the new preedit did not claim ownership");
    }

    /// Records outgoing requests so the wire order can be checked without a compositor.
    #[derive(Default)]
    struct Recorder {
        requests: RefCell<Vec<String>>,
        on_commit: Option<Box<dyn Fn()>>,
    }

    impl Recorder {
        fn push(&self, request: impl Into<String>) {
            self.requests.borrow_mut().push(request.into());
        }

        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.requests.borrow_mut())
        }
    }

    impl TextInputRequests for Recorder {
        fn enable(&self) {
            self.push("enable");
        }

        fn disable(&self) {
            self.push("disable");
        }

        fn set_content_type(&self, purpose: ImePurpose) {
            self.push(format!("content_type:{purpose:?}"));
        }

        fn set_cursor_rectangle(&self, x: i32, y: i32, width: i32, height: i32) {
            self.push(format!("cursor:{x},{y},{width},{height}"));
        }

        fn set_text_change_cause(&self, cause: ImeTextChangeCause) {
            self.push(format!("text_change_cause:{cause:?}"));
        }

        fn set_surrounding_text(&self, text: String, cursor: i32, anchor: i32) {
            self.push(format!("surrounding:{text}:{cursor}:{anchor}"));
        }

        fn commit(&self) {
            self.push("commit");
            if let Some(probe) = &self.on_commit {
                probe();
            }
        }
    }

    fn state(
        surrounding: Option<(&str, i32, i32)>,
        cursor_area: Option<(i32, i32, i32, i32)>,
    ) -> OutgoingImeState {
        OutgoingImeState::new(
            ImePurpose::Normal,
            cursor_area,
            surrounding.map(|(text, cursor, anchor)| (text.to_owned(), cursor, anchor)),
        )
    }

    #[test]
    fn request_order_enables_before_state_and_commits_after() {
        // text-input-v3 `enable` resets the content type, cursor rectangle and surrounding
        // text, so the state setters must follow it and share its commit.
        let recorder = Recorder::default();
        let window = owner(1);
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let state = state(Some(("abc", 1, 1)), Some((1, 2, 3, 4)));

        let (serial, _) = enable_generation(&recorder, &mut data, &state);

        assert_eq!(recorder.take(), vec![
            "enable",
            "content_type:Normal",
            "cursor:1,2,3,4",
            "text_change_cause:Other",
            "surrounding:abc:1:1",
            "commit",
        ]);
        assert_eq!(data.incoming_from, Some(serial));
        assert!(data.accepts_incoming());
        assert_eq!(data.awaiting, Some(SentContext::Enabled(state)));
    }

    #[test]
    fn request_order_resets_with_disable_then_enable_state_commit() {
        let recorder = Recorder::default();
        let window = owner(1);
        let mut data = enabled(window);
        let state = state(Some(("next", 2, 2)), Some((4, 3, 2, 1)));

        let (was_shown, serial, _) = reset_generation(&recorder, &mut data, &state);

        assert!(!was_shown);
        assert_eq!(recorder.take(), vec![
            "disable",
            "commit",
            "enable",
            "content_type:Normal",
            "cursor:4,3,2,1",
            "text_change_cause:Other",
            "surrounding:next:2:2",
            "commit",
        ]);
        assert_eq!(data.incoming_from, Some(serial));
        assert!(data.accepts_incoming());
    }

    #[test]
    fn outgoing_setters_coalesce_until_the_matching_done_and_flush_once() {
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let first = state(Some(("abc", 3, 3)), None);
        let (enable_serial, _) = enable_generation(&recorder, &mut data, &first);
        let _ = recorder.take();

        // Two setters while the enable commit is unacknowledged must not each commit.
        let second = state(Some(("abcd", 4, 4)), None);
        assert_eq!(data.sync_outgoing(&recorder, &second, None), OutgoingCommit::Deferred);
        let third = state(Some(("abcde", 5, 5)), None);
        assert_eq!(data.sync_outgoing(&recorder, &third, None), OutgoingCommit::Deferred);
        assert!(recorder.take().is_empty(), "a setter committed before its done");

        // A mismatched done applies incoming edits but does not flush outgoing state.
        assert_eq!(
            data.acknowledge_done(enable_serial.wrapping_add(1), Some(&third), &recorder),
            OutgoingCommit::Unmatched
        );
        assert!(recorder.take().is_empty(), "a mismatched done committed pending state");

        // The matching done acknowledges the enable and flushes the newest snapshot once.
        assert_eq!(
            data.acknowledge_done(enable_serial, Some(&third), &recorder),
            OutgoingCommit::Sent(enable_serial.wrapping_add(1))
        );
        let flushed = recorder.take();
        assert_eq!(
            flushed,
            vec![
                "content_type:Normal",
                "text_change_cause:Other",
                "surrounding:abcde:5:5",
                "commit",
            ],
            "coalesced setters did not share one follow-up commit"
        );
        assert!(
            !flushed.iter().any(|request| request == "disable" || request == "enable"),
            "a same-context edit reset the composition"
        );

        // The flush's own done is quiet: the compositor already has this state.
        assert_eq!(
            data.acknowledge_done(enable_serial.wrapping_add(1), Some(&third), &recorder),
            OutgoingCommit::Acknowledged
        );
        assert!(recorder.take().is_empty(), "an acknowledged snapshot was sent again");
    }

    fn enabled_recorder_state() -> (Recorder, TextInputDataInner, OutgoingImeState, u32) {
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(owner(1));
        let first = state(Some(("abc", 3, 3)), None);
        let (serial, _) = enable_generation(&recorder, &mut data, &first);
        let _ = recorder.take();
        (recorder, data, first, serial)
    }

    #[test]
    fn surrounding_text_setter_reports_external_other_by_default() {
        let (recorder, mut data, first, serial) = enabled_recorder_state();
        // The protocol requires state setters to wait for the `done` that acknowledges the enable
        // commit; otherwise they are only coalesced and this test would assert on an empty queue.
        assert_eq!(
            data.acknowledge_done(serial, Some(&first), &recorder),
            OutgoingCommit::Acknowledged
        );
        let _ = recorder.take();
        let changed = state(Some(("abcd", 4, 4)), None);

        assert!(matches!(
            data.sync_outgoing(&recorder, &changed, Some(ImeTextChangeCause::Other)),
            OutgoingCommit::Sent(_)
        ));
        assert_eq!(
            recorder.take(),
            vec![
                "content_type:Normal",
                "text_change_cause:Other",
                "surrounding:abcd:4:4",
                "commit",
            ],
            "the application-originated surrounding setter didn't report an external change"
        );
    }

    #[test]
    fn ime_origin_setter_reports_input_method_cause() {
        let (recorder, mut data, first, serial) = enabled_recorder_state();
        assert_eq!(
            data.acknowledge_done(serial, Some(&first), &recorder),
            OutgoingCommit::Acknowledged
        );
        let _ = recorder.take();
        let changed = state(Some(("after commit", 12, 12)), None);

        assert!(matches!(
            data.sync_outgoing(&recorder, &changed, Some(ImeTextChangeCause::InputMethod)),
            OutgoingCommit::Sent(_)
        ));
        assert_eq!(recorder.take(), vec![
            "content_type:Normal",
            "text_change_cause:InputMethod",
            "surrounding:after commit:12:12",
            "commit",
        ]);
    }

    #[test]
    fn coalesced_surrounding_cause_is_flushed_with_its_snapshot() {
        let (recorder, mut data, _first, serial) = enabled_recorder_state();
        let changed = state(Some(("abcd", 4, 4)), None);
        let cursor = OutgoingImeState::new(
            ImePurpose::Normal,
            Some((9, 8, 7, 6)),
            changed.surrounding_text.clone(),
        );

        assert_eq!(
            data.sync_outgoing(&recorder, &changed, Some(ImeTextChangeCause::Other)),
            OutgoingCommit::Deferred
        );
        // A later cursor-only update must not drop the cause waiting behind the first commit.
        assert_eq!(data.sync_outgoing(&recorder, &cursor, None), OutgoingCommit::Deferred);
        assert!(recorder.take().is_empty());

        assert_eq!(
            data.acknowledge_done(serial, Some(&cursor), &recorder),
            OutgoingCommit::Sent(serial.wrapping_add(1))
        );
        assert_eq!(
            recorder.take(),
            vec![
                "content_type:Normal",
                "cursor:9,8,7,6",
                "text_change_cause:Other",
                "surrounding:abcd:4:4",
                "commit",
            ],
            "the coalesced commit lost the external change cause"
        );
    }

    #[test]
    fn cursor_only_commit_reports_other_after_acknowledgement() {
        // A geometry-only update still re-sends the cached surrounding text in protocol order.
        // The compositor would otherwise apply the default `input_method` cause to app-origin
        // text, making the IME think it changed the document.
        let (recorder, mut data, first, serial) = enabled_recorder_state();
        assert_eq!(
            data.acknowledge_done(serial, Some(&first), &recorder),
            OutgoingCommit::Acknowledged
        );
        let _ = recorder.take();

        let cursor = OutgoingImeState::new(
            ImePurpose::Normal,
            Some((9, 8, 7, 6)),
            first.surrounding_text.clone(),
        );
        assert!(matches!(data.sync_outgoing(&recorder, &cursor, None), OutgoingCommit::Sent(_)));
        assert_eq!(
            recorder.take(),
            vec![
                "content_type:Normal",
                "cursor:9,8,7,6",
                "text_change_cause:Other",
                "surrounding:abc:3:3",
                "commit",
            ],
            "a cursor-only commit inherited the protocol's input_method cause"
        );
    }

    #[test]
    fn purpose_only_commit_reports_other() {
        let (recorder, mut data, first, serial) = enabled_recorder_state();
        assert_eq!(
            data.acknowledge_done(serial, Some(&first), &recorder),
            OutgoingCommit::Acknowledged
        );
        let _ = recorder.take();

        let changed =
            OutgoingImeState::new(ImePurpose::Terminal, None, first.surrounding_text.clone());
        assert!(matches!(data.sync_outgoing(&recorder, &changed, None), OutgoingCommit::Sent(_)));
        assert_eq!(
            recorder.take(),
            vec![
                "content_type:Terminal",
                "text_change_cause:Other",
                "surrounding:abc:3:3",
                "commit",
            ],
            "a purpose-only commit relabeled application text as input-method origin"
        );
    }

    #[test]
    fn pending_input_method_receipt_survives_a_geometry_update() {
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let initial = state(Some(("abc", 3, 3)), None);
        let (serial, _) = enable_generation(&recorder, &mut data, &initial);
        let _ = recorder.take();

        let after_commit = state(Some(("abcdef", 6, 6)), None);
        assert_eq!(
            data.sync_outgoing(&recorder, &after_commit, Some(ImeTextChangeCause::InputMethod)),
            OutgoingCommit::Deferred
        );
        let moved = OutgoingImeState::new(
            ImePurpose::Normal,
            Some((10, 20, 30, 4)),
            after_commit.surrounding_text.clone(),
        );
        // Cursor geometry is not a new surrounding-text cause. It must not replace the receipt
        // that is still waiting for the first commit's matching `done`.
        assert_eq!(data.sync_outgoing(&recorder, &moved, None), OutgoingCommit::Deferred);
        assert!(recorder.take().is_empty());

        assert!(matches!(
            data.acknowledge_done(serial, Some(&moved), &recorder),
            OutgoingCommit::Sent(_)
        ));
        assert_eq!(
            recorder.take(),
            vec![
                "content_type:Normal",
                "cursor:10,20,30,4",
                "text_change_cause:InputMethod",
                "surrounding:abcdef:6:6",
                "commit",
            ],
            "a geometry update erased the coalesced InputMethod receipt cause"
        );
    }

    #[test]
    fn mismatched_done_still_applies_incoming_edits_while_outgoing_waits() {
        // The protocol's two halves: a done whose serial is not the newest commit must be
        // evaluated normally for incoming text, but the client must not change outgoing state
        // until a matching done.
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let first = state(Some(("abc", 3, 3)), None);
        let (enable_serial, _) = enable_generation(&recorder, &mut data, &first);
        let _ = recorder.take();
        let mut sink = EventSink::new();

        data.commit_string(Some("typed".to_owned()));
        let stale = enable_serial.wrapping_add(1);
        data.finish_done(stale, Some(window), &mut sink);
        assert_eq!(
            ime_events(&mut sink),
            vec![
                (window, Ime::Preedit(String::new(), None)),
                (window, Ime::Commit("typed".to_owned())),
            ],
            "a current-generation serial mismatch dropped incoming text"
        );

        let changed = state(Some(("abcd", 4, 4)), None);
        assert_eq!(data.sync_outgoing(&recorder, &changed, None), OutgoingCommit::Deferred);
        assert_eq!(
            data.acknowledge_done(stale, Some(&changed), &recorder),
            OutgoingCommit::Unmatched
        );
        assert!(recorder.take().is_empty());

        assert_eq!(
            data.acknowledge_done(enable_serial, Some(&changed), &recorder),
            OutgoingCommit::Sent(enable_serial.wrapping_add(1))
        );
        assert_eq!(recorder.take(), vec![
            "content_type:Normal",
            "text_change_cause:Other",
            "surrounding:abcd:4:4",
            "commit",
        ]);
    }

    #[test]
    fn coalesced_old_record_with_only_a_new_done_is_applied_not_silently_dropped() {
        // text-input-v3 has no per-record generation. If a compositor coalesces an old record
        // with a new commit and sends only the new `done`, this client cannot prove the record is
        // stale. Dropping it would silently lose a possibly valid new commit, so it applies the
        // current-generation record. Native traces must qualify whether a given compositor can
        // produce this sequence.
        let window = owner(1);
        let mut data = enabled(window);
        data.end_incoming_generation();
        let _disable_serial = data.request_commit();
        re_enable(&mut data);
        let mut sink = EventSink::new();

        data.commit_string(Some("coalesced".to_owned()));
        let outcome = data.finish_done(done_serial(&data), Some(window), &mut sink);

        assert!(outcome.committed);
        assert_eq!(
            ime_events(&mut sink),
            vec![
                (window, Ime::Preedit(String::new(), None)),
                (window, Ime::Commit("coalesced".to_owned())),
            ],
            "a current-generation commit was discarded to hide a wire ambiguity"
        );
    }

    #[test]
    fn enter_invalidates_pending_outgoing_state_instead_of_flushing_to_the_new_owner() {
        // `enter` invalidates all protocol state, so a late done for the previous surface must
        // not flush the new surface's state without an enable.
        let first = owner(1);
        let second = owner(2);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(first);
        let (serial, _) = enable_generation(&recorder, &mut data, &state(None, None));
        let _ = recorder.take();

        let changed = state(Some(("new", 3, 3)), None);
        assert_eq!(data.sync_outgoing(&recorder, &changed, None), OutgoingCommit::Deferred);

        data.enter_generation(second);
        assert_eq!(data.acknowledge_done(serial, Some(&changed), &recorder), OutgoingCommit::Quiet);
        assert!(
            recorder.take().is_empty(),
            "a previous surface's done flushed state for the new owner"
        );
    }

    #[test]
    fn commit_serials_wrap_and_acknowledgement_uses_the_issued_count() {
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        // The count is the protocol's serial; it wraps at u32::MAX like the compositor's.
        data.request_serial = u32::MAX;
        let first = state(Some(("wrap", 4, 4)), None);
        let (enable_serial, _) = enable_generation(&recorder, &mut data, &first);
        assert_eq!(enable_serial, 0, "the commit count did not wrap");
        let _ = recorder.take();

        let changed = state(Some(("wrapped", 7, 7)), None);
        assert_eq!(data.sync_outgoing(&recorder, &changed, None), OutgoingCommit::Deferred);
        // A done for the pre-wrap commit is stale now: the newest issued serial is 0.
        assert_eq!(
            data.acknowledge_done(u32::MAX, Some(&changed), &recorder),
            OutgoingCommit::Unmatched
        );
        assert_eq!(data.acknowledge_done(0, Some(&changed), &recorder), OutgoingCommit::Sent(1));
        assert_eq!(recorder.take(), vec![
            "content_type:Normal",
            "text_change_cause:Other",
            "surrounding:wrapped:7:7",
            "commit",
        ]);
    }

    #[test]
    fn password_state_never_carries_text() {
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let password =
            OutgoingImeState::new(ImePurpose::Password, None, Some(("hunter2".to_owned(), 7, 7)));

        enable_generation(&recorder, &mut data, &password);

        let requests = recorder.take();
        assert!(requests.iter().any(|request| request == "content_type:Password"));
        assert!(
            !requests
                .iter()
                .any(|request| request.contains("hunter2") || request.starts_with("surrounding:")),
            "password text reached the protocol: {requests:?}"
        );
    }

    #[test]
    fn password_reset_drops_an_active_buffered_commit_without_cached_text_or_preedit() {
        // The password transition decides that a reset is needed from the active incoming
        // generation alone. It must not depend on cached surrounding text or a visible preedit.
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = enabled(window);
        data.commit_string(Some("old field".to_owned()));
        data.delete_surrounding_text(3, 0);
        assert!(data.has_pending_or_active());
        assert!(context_reset_needed(false, true, true, false, data.has_pending_or_active()));

        let password = OutgoingImeState::new(ImePurpose::Password, None, None);
        let old_serial = done_serial(&data);
        let (_was_shown, _serial, _sequence) = reset_generation(&recorder, &mut data, &password);
        assert_eq!(
            recorder.take(),
            vec!["disable", "commit", "enable", "content_type:Password", "commit",],
            "the password transition did not send a committed disable/enable reset"
        );

        assert!(
            data.batch.commit.is_none()
                && data.batch.delete.is_none()
                && data.batch.preedit.is_none(),
            "buffered pre-password records survived the reset"
        );
        let mut sink = EventSink::new();
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        // A bare done still sends the upstream-compatible empty preedit, but no old edits.
        assert_eq!(deliveries(&mut sink), vec![(window, Ime::Preedit(String::new(), None), None)]);
        data.finish_done(old_serial, Some(window), &mut sink);
        assert!(ime_events(&mut sink).is_empty(), "a fenced done revived the old batch");
    }

    #[test]
    fn explicit_context_clear_drops_an_active_buffered_commit_without_cached_text() {
        // The explicit unavailable operation has the same lifecycle requirement as the password
        // transition: an active/buffered generation is invalidated even with no cached text and
        // no visible preedit.
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = enabled(window);
        data.commit_string(Some("old field".to_owned()));
        data.delete_surrounding_text(2, 1);
        assert!(context_reset_needed(false, true, true, false, data.has_pending_or_active()));

        let unavailable = OutgoingImeState::new(ImePurpose::Normal, None, None);
        let old_serial = done_serial(&data);
        let (_was_shown, _serial, _sequence) = reset_generation(&recorder, &mut data, &unavailable);
        assert_eq!(
            recorder.take(),
            vec!["disable", "commit", "enable", "content_type:Normal", "commit",],
            "the explicit clear did not send a committed disable/enable reset"
        );
        assert!(
            data.batch.commit.is_none()
                && data.batch.delete.is_none()
                && data.batch.preedit.is_none(),
            "buffered pre-clear records survived the reset"
        );
        let mut sink = EventSink::new();
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        // A bare done still sends the upstream-compatible empty preedit, but no old edits.
        assert_eq!(deliveries(&mut sink), vec![(window, Ime::Preedit(String::new(), None), None)]);
        data.finish_done(old_serial, Some(window), &mut sink);
        assert!(ime_events(&mut sink).is_empty(), "a fenced done revived the old batch");
    }

    #[test]
    fn bare_done_publishes_one_empty_preedit_and_no_deletion() {
        let window = owner(1);
        let mut data = enabled(window);
        let mut sink = EventSink::new();
        data.finish_done(done_serial(&data), Some(window), &mut sink);
        assert_eq!(deliveries(&mut sink), vec![(window, Ime::Preedit(String::new(), None), None)]);
    }

    #[test]
    fn resetting_a_non_owner_seat_preserves_the_other_seats_preedit() {
        let window = owner(1);
        let mut owners = ImePreeditOwnership::<u32>::default();
        owners.claim(&2);
        let mut data = enabled(window);
        data.commit_string(Some("discarded".into()));
        let recorder = Recorder::default();
        reset_generation(&recorder, &mut data, &state(None, None));
        let mut sink = EventSink::new();
        let outcome = data.finish_done_with_preedit_policy(
            done_serial(&data),
            Some(window),
            &mut sink,
            owners.may_clear(&1),
        );
        owners.after_done(&1, outcome);
        assert!(sink.is_empty());
        assert!(owners.owns(&2));
        assert_eq!(outcome, ImeBatchOutcome::default());
    }

    #[test]
    fn unavailable_omits_surrounding_text_while_an_empty_document_sends_it() {
        assert!(state(None, None).surrounding_text.is_none());
        assert_eq!(state(Some(("", 0, 0)), None).surrounding_text, Some((String::new(), 0, 0)));

        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        enable_generation(&recorder, &mut data, &state(None, None));
        assert_eq!(
            recorder.take(),
            vec!["enable", "content_type:Normal", "commit"],
            "unavailable sent a surrounding-text request"
        );

        let available_empty = state(Some(("", 0, 0)), None);
        reset_generation(&recorder, &mut data, &available_empty);
        assert_eq!(
            recorder.take(),
            vec![
                "disable",
                "commit",
                "enable",
                "content_type:Normal",
                "text_change_cause:Other",
                "surrounding::0:0",
                "commit",
            ],
            "an available empty document was treated as unavailable"
        );
    }

    #[test]
    fn cursor_area_cached_while_disabled_is_sent_by_the_next_enable() {
        // The window keeps its desired cursor area while IME is disabled. The next enable uses
        // that current area, not the one cached before focus moved.
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let new_area = (40, 50, 60, 20);
        let desired = OutgoingImeState::new(
            ImePurpose::Normal,
            Some(new_area),
            Some(("same field".to_owned(), 4, 4)),
        );

        enable_generation(&recorder, &mut data, &desired);

        assert_eq!(
            recorder.take(),
            vec![
                "enable",
                "content_type:Normal",
                "cursor:40,50,60,20",
                "text_change_cause:Other",
                "surrounding:same field:4:4",
                "commit",
            ],
            "the enable republished a stale cursor area"
        );
    }

    #[test]
    fn disable_acknowledgement_never_reenables_or_publishes_private_text() {
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let private = state(Some(("secret", 6, 6)), None);
        enable_generation(&recorder, &mut data, &private);
        let _ = recorder.take();

        let (_was_shown, _sequence) = disable_generation(&recorder, &mut data);
        let disable_serial = data.request_serial;
        let _ = recorder.take();

        // The disable is acknowledged while the window has been disabled: no state flush and no
        // implicit enable may follow.
        assert_eq!(
            data.acknowledge_done(disable_serial, Some(&private), &recorder),
            OutgoingCommit::Acknowledged
        );
        assert!(recorder.take().is_empty(), "a disabled window was re-enabled by a done");

        // Setters while disabled are deferred, not committed, and a later enable without
        // surrounding text publishes only the content type.
        assert_eq!(data.sync_outgoing(&recorder, &private, None), OutgoingCommit::Deferred);
        assert!(recorder.take().is_empty());
        let unavailable = state(None, None);
        enable_generation(&recorder, &mut data, &unavailable);
        let requests = recorder.take();
        assert_eq!(
            requests,
            vec!["enable", "content_type:Normal", "commit"],
            "disabling/clearing republished private text"
        );
    }

    #[test]
    fn context_clear_needs_a_committed_reset_only_for_a_published_or_active_context() {
        assert!(
            !context_reset_needed(true, false, true, true, true),
            "a disabled window was reset"
        );
        assert!(
            !context_reset_needed(true, true, false, true, true),
            "no entered text input to reset"
        );
        assert!(
            !context_reset_needed(false, true, true, false, false),
            "an untouched context was reset"
        );
        assert!(context_reset_needed(true, true, true, false, false));
        assert!(context_reset_needed(false, true, true, true, false));
        assert!(
            context_reset_needed(false, true, true, false, true),
            "an active generation without cached text or a visible preedit was not invalidated"
        );
    }

    #[test]
    fn two_seats_acknowledge_independently_with_one_commit_each() {
        let first_window = owner(1);
        let second_window = owner(2);
        let first_recorder = Recorder::default();
        let second_recorder = Recorder::default();
        let mut first = TextInputDataInner::default();
        let mut second = TextInputDataInner::default();
        first.enter_generation(first_window);
        second.enter_generation(second_window);
        let old = state(Some(("old", 3, 3)), None);
        let (first_serial, _) = enable_generation(&first_recorder, &mut first, &old);
        let (second_serial, _) = enable_generation(&second_recorder, &mut second, &old);
        let _ = first_recorder.take();
        let _ = second_recorder.take();

        let changed = state(Some(("new", 3, 3)), None);
        assert_eq!(first.sync_outgoing(&first_recorder, &changed, None), OutgoingCommit::Deferred);
        assert_eq!(
            second.sync_outgoing(&second_recorder, &changed, None),
            OutgoingCommit::Deferred
        );

        // Seat 1's done acknowledges only seat 1.
        assert_eq!(
            first.acknowledge_done(first_serial, Some(&changed), &first_recorder),
            OutgoingCommit::Sent(first_serial.wrapping_add(1))
        );
        assert_eq!(first_recorder.take(), vec![
            "content_type:Normal",
            "text_change_cause:Other",
            "surrounding:new:3:3",
            "commit",
        ]);
        assert!(second_recorder.take().is_empty(), "one seat's done flushed another seat");

        assert_eq!(
            second.acknowledge_done(second_serial, Some(&changed), &second_recorder),
            OutgoingCommit::Sent(second_serial.wrapping_add(1))
        );
        assert_eq!(second_recorder.take(), vec![
            "content_type:Normal",
            "text_change_cause:Other",
            "surrounding:new:3:3",
            "commit",
        ]);

        // The next commit is still outstanding. A stale done must not acknowledge it.
        assert_eq!(
            first.acknowledge_done(first_serial, Some(&changed), &first_recorder),
            OutgoingCommit::Unmatched
        );
        assert!(first_recorder.take().is_empty());
        assert_eq!(
            first.acknowledge_done(first_serial.wrapping_add(1), Some(&changed), &first_recorder),
            OutgoingCommit::Acknowledged
        );
        assert_eq!(
            first.acknowledge_done(first_serial, Some(&changed), &first_recorder),
            OutgoingCommit::Quiet
        );
        assert!(first_recorder.take().is_empty(), "an acknowledged seat sent another commit");
    }

    #[test]
    fn lifecycle_boundary_ticket_orders_the_synthetic_event_first() {
        // A synthetic `Enabled` queued from a non-event-loop thread can reach its sink after a
        // compositor event from the new generation. The boundary ticket reserved under the data
        // lock must still sort the synthetic event first.
        let window = owner(1);
        let recorder = Recorder::default();
        let mut data = TextInputDataInner::default();
        data.enter_generation(window);
        let state = state(None, None);
        let (serial, sequence) = enable_generation(&recorder, &mut data, &state);

        let mut protocol_events = EventSink::new();
        let mut window_events = EventSink::new();
        // This mimics `Window::set_ime_allowed` pushing late, after the protocol event arrived.
        data.preedit_string(Some("new".to_owned()), 0, 3);
        data.finish_done(serial, Some(window), &mut protocol_events);
        window_events.push_window_event_at(sequence, WindowEvent::Ime(Ime::Enabled), window);

        assert_eq!(
            merged_deliveries(&mut protocol_events, &mut window_events),
            vec![
                (window, Ime::Enabled, None),
                (window, Ime::Preedit("new".to_owned(), Some((0, 3))), None),
            ],
            "a new generation's preedit overtook its synthetic Enabled event"
        );
    }

    #[test]
    fn requested_reset_orders_old_commit_fence_and_new_suffix_without_visible_preedit() {
        for visible in [false, true] {
            let window = owner(1);
            let data = std::sync::Mutex::new(enabled(window));
            let recorder = Recorder::default();
            let mut protocol_events = EventSink::new();
            let mut window_events = EventSink::new();
            {
                let mut data = data.lock().unwrap();
                data.commit_string(Some("old commit".into()));
                let serial = done_serial(&data);
                data.finish_done(serial, Some(window), &mut protocol_events);
                if visible {
                    data.preedit_string(Some("old preview".into()), 0, 11);
                    data.finish_done(serial, Some(window), &mut protocol_events);
                }
            }
            let (resets, fence) =
                reset_inputs_with_fence(&[(&recorder, Some(&data))], &state(None, None));
            assert_eq!(resets[0].0, visible);
            {
                let mut data = data.lock().unwrap();
                let serial = done_serial(&data);
                data.preedit_string(Some("new preview".into()), 0, 11);
                data.finish_done(serial, Some(window), &mut protocol_events);
                data.commit_string(Some("new commit".into()));
                data.finish_done(serial, Some(window), &mut protocol_events);
            }
            // The new suffix arrived at its sink before the window pushed its fence. The
            // reserved sequence must still place the completion before that suffix.
            if visible {
                window_events.push_window_event_at(
                    resets[0].1,
                    WindowEvent::Ime(Ime::Preedit(String::new(), None)),
                    window,
                );
            }
            window_events.push_window_event_at(
                fence,
                WindowEvent::ImeResetCompleted { request_id: u64::MAX },
                window,
            );
            protocol_events.append_ordered(&mut window_events);
            let actual: Vec<_> =
                protocol_events.drain_deliverable().into_iter().map(|(e, _)| e).collect();
            let mut expected = vec![
                WindowEvent::Ime(Ime::Preedit(String::new(), None)),
                WindowEvent::Ime(Ime::Commit("old commit".into())),
            ];
            if visible {
                expected.push(WindowEvent::Ime(Ime::Preedit(String::new(), None)));
            }
            expected.extend([
                WindowEvent::ImeResetCompleted { request_id: u64::MAX },
                WindowEvent::Ime(Ime::Preedit("new preview".into(), Some((0, 11)))),
                WindowEvent::Ime(Ime::Preedit(String::new(), None)),
                WindowEvent::Ime(Ime::Commit("new commit".into())),
            ]);
            assert_eq!(
                actual,
                expected
                    .into_iter()
                    .map(|event| Event::WindowEvent {
                        window_id: crate::window::WindowId(window),
                        event,
                    })
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn requested_reset_holds_all_entered_seats_until_the_fence_is_reserved() {
        use std::cell::Cell;
        use std::rc::Rc;
        use std::sync::{Mutex, TryLockError};

        let first = Rc::new(Mutex::new(enabled(owner(1))));
        let second = Rc::new(Mutex::new(enabled(owner(1))));
        let probes = Rc::new(Cell::new(0));
        let make_recorder = || {
            let first = Rc::clone(&first);
            let second = Rc::clone(&second);
            let probes = Rc::clone(&probes);
            Recorder {
                on_commit: Some(Box::new(move || {
                    assert!(matches!(first.try_lock(), Err(TryLockError::WouldBlock)));
                    assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));
                    probes.set(probes.get() + 1);
                })),
                ..Recorder::default()
            }
        };
        let first_recorder = make_recorder();
        let second_recorder = make_recorder();
        let (resets, fence) = reset_inputs_with_fence(
            &[(&first_recorder, Some(&first)), (&second_recorder, Some(&second))],
            &state(None, None),
        );
        assert_eq!(probes.get(), 4);
        assert!(resets.iter().all(|(_, sequence)| *sequence < fence));
        assert!(first.try_lock().is_ok() && second.try_lock().is_ok());
        let (_, empty_fence) = reset_inputs_with_fence::<Recorder>(&[], &state(None, None));
        assert!(fence < empty_fence);
    }
}
