//! Guards for the IME fixes this branch carries on top of winit 0.30.13.
//!
//! Most of these fixes can only be exercised with a real input method on the platform, so each
//! test checks that the fix is still in the source and fails if a rebase drops it. When a winit
//! release carries an equivalent fix, drop the patch together with its test.

use std::path::Path;

fn source(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform_impl").join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// The text after `start`, up to the first `end`.
fn section<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let (_, rest) = text.split_once(start).unwrap_or_else(|| panic!("`{start}` not found"));
    rest.split_once(end).map_or(rest, |(body, _)| body)
}

#[test]
fn wayland_text_input_lives_until_the_seat_goes() {
    // winit 0.30.13 destroys a seat's text input whenever the seat loses any capability (a touch
    // screen or tablet unplugged, a remote-desktop session ending) and recreates it only when a
    // capability is added, so IME stops working until the app restarts. The text input belongs to
    // the seat instead (rust-windowing/winit#4747).
    let seat = source("linux/wayland/seat/mod.rs");
    let remove_capability = section(&seat, "fn remove_capability(", "\n    }\n");
    let remove_seat = section(&seat, "fn remove_seat(", "\n    }\n");
    assert!(
        !remove_capability.contains("text_input.destroy()"),
        "losing a capability destroys the seat's text input"
    );
    assert!(
        remove_seat.contains("text_input.destroy()"),
        "removing the seat doesn't destroy its text input"
    );
}

#[test]
fn windows_ime_cursor_is_read_only_when_reported() {
    // winit 0.30.13 reads `GCS_CURSORPOS` for every composition without a target clause, whether
    // or not `WM_IME_COMPOSITION` says the IME set it. The Microsoft Korean IME never sets it, so
    // the cursor read as 0, before the syllable being composed. Without the flag the cursor is at
    // the end of the composition (rust-windowing/winit#4746).
    let ime = source("windows/ime.rs");
    assert!(
        ime.contains("if composition_flags & GCS_CURSORPOS != 0 {"),
        "GCS_CURSORPOS is read unconditionally"
    );
    let event_loop = source("windows/event_loop.rs");
    assert!(
        event_loop.contains("get_composing_text_and_cursor(lparam as u32)"),
        "WM_IME_COMPOSITION doesn't pass its flags on"
    );
}

#[test]
fn x11_input_context_outlives_disallowing_ime() {
    // Toolkits allow IME when a text field gains focus and disallow it when the field loses it
    // (egui does). winit 0.30.13 destroys the window's input context each time and creates a new
    // one, which resets the input method's state for the window (Vietnamese Telex turns off) and
    // stays unfocused until the next focus event, which never comes while the window keeps the
    // focus. The window keeps one context: allowing IME focuses it, disallowing resets and
    // unfocuses it. This also covers rust-windowing/winit#4727.
    let ime = source("linux/x11/ime/mod.rs");
    let set_ime_allowed = section(&ime, "pub fn set_ime_allowed(", "\n    }\n");
    assert!(
        !set_ime_allowed.contains("remove_context"),
        "set_ime_allowed destroys the input context"
    );
    let context = source("linux/x11/ime/context.rs");
    assert!(
        !context.contains("none_style"),
        "a context with IME disallowed is created with another input style"
    );
    let events = source("linux/x11/event_processor.rs");
    let allow =
        section(&events, "ImeRequest::Allow(window_id, allowed) => {", "\n                },\n");
    for call in ["ime.focus(window_id)", "ime.reset(window_id)", "ime.unfocus(window_id)"] {
        assert!(allow.contains(call), "an IME allow request doesn't call `{call}`");
    }
}

#[test]
fn x11_ime_spot_waits_for_the_input_method() {
    // Setting the IME spot is a synchronous XIM request. Keys an input method forwards back while
    // the client waits for the reply are filtered again by Xlib and reach the input method as new
    // key presses; Vietnamese engines that fix words with forwarded BackSpaces garble the text
    // (rust-windowing/winit#4777, storytold/wordcraft#2932). Spots are queued and sent once no
    // event is queued and the input method has been quiet for a while.
    let events = source("linux/x11/event_processor.rs");
    let position = section(&events, "ImeRequest::Position(window_id, x, y) => {", "},");
    assert!(!position.contains("send_xim_spot"), "an IME spot is sent as soon as it's requested");
    let send = section(&events, "pub fn send_ime_spots(&mut self) {", "\n    }\n");
    assert!(send.contains("self.ime_busy_until"), "IME spots don't wait for the input method");
    assert!(send.contains("self.poll()"), "IME spots are sent while events are queued");
    let filtered = section(&events, "// Don't process event if it was filtered.", "return;");
    assert!(
        filtered.contains("self.ime_busy_until = Some("),
        "events taken by the input method don't hold IME spots back"
    );
    let event_loop = source("linux/x11/mod.rs");
    assert!(
        event_loop.contains("self.event_processor.send_ime_spots();"),
        "the event loop doesn't send IME spots held back"
    );
}

#[allow(dead_code)]
fn reset_ime_is_public(window: &winit::window::Window) {
    window.reset_ime();
    window.reset_ime_with_request(u64::MAX);
}

#[test]
fn linux_reset_ime_discards_the_composition() {
    // `Window::reset_ime` resets the input method when focus moves between fields. X11 resets the
    // input context before the next key reaches the input method; Wayland disables and re-enables
    // text input in two commits, so the compositor deactivates the input method, and sends the
    // state that enabling resets again.
    let events = source("linux/x11/event_processor.rs");
    let process_event = section(&events, "pub fn process_event<", "\n    }\n");
    let (before, after) = process_event
        .split_once("self.process_xevent(xev, &mut callback);")
        .expect("process_event doesn't process the event");
    assert!(
        before.contains("self.handle_ime_requests();")
            && after.contains("self.handle_ime_requests();"),
        "IME requests aren't handled before and after each event"
    );
    assert!(
        before.contains("self.drain_ime_events(&mut callback);")
            && after.contains("self.drain_ime_events(&mut callback);"),
        "a queued reset fence can be overtaken by the next native event"
    );
    let context = source("linux/x11/ime/context.rs");
    assert!(context.contains("(xconn.xlib.XmbResetIC)(self.ic)"), "X11 doesn't reset the context");

    let state = source("linux/wayland/window/state.rs");
    let reset =
        section(&state, "pub(crate) fn reset_ime(&mut self) -> ImeStateUpdate {", "\n    }\n");
    assert!(
        reset.contains("text_input.reset_with_state(&state);"),
        "Wayland reset doesn't disable, enable and restore the IME state"
    );
    // The wire order itself (`disable`/`commit`, `enable`, state, `commit`) is checked by the
    // recording test in `text_input::tests::request_order_*`.
    let text_input = source("linux/wayland/seat/text_input/mod.rs");
    let reset_generation =
        section(&text_input, "fn reset_generation<T: TextInputRequests>(", "\n}\n");
    let disable_step = reset_generation
        .find("disable_generation(input, data)")
        .expect("reset doesn't disable first");
    let enable_step = reset_generation
        .find("enable_generation(input, data, state)")
        .expect("reset doesn't enable after the disable");
    assert!(disable_step < enable_step, "reset doesn't disable before it enables");
    let enable_generation =
        section(&text_input, "fn enable_generation<T: TextInputRequests>(", "\n}\n");
    let enable = enable_generation.find("input.enable();").expect("enable isn't sent");
    let state_step = enable_generation
        .find("data.send_enabled_commit(input, state, Some(ImeTextChangeCause::Other))")
        .expect("the reset state isn't applied and committed after enable");
    assert!(
        enable < state_step,
        "text-input-v3 enable doesn't precede the state it resets and its commit"
    );
    let send_state = section(&text_input, "fn send_enabled_commit<", "\n    }\n");
    let requests =
        send_state.find("send_state_requests(").expect("the enabled state setters aren't sent");
    let commit = send_state.find("input.commit();").expect("the enabled state isn't committed");
    assert!(requests < commit, "the state setters don't precede their commit");
    let snapshot = section(&state, "pub(crate) fn outgoing_ime_state(", "\n    }\n");
    assert!(
        snapshot.contains("OutgoingImeState::new(")
            && snapshot.contains("self.ime_purpose")
            && snapshot.contains("self.ime_cursor_area")
            && snapshot.contains("self.ime_surrounding_text"),
        "enabling text input doesn't rebuild the full outgoing snapshot"
    );
    let apply = section(&text_input, "fn send_state_requests<", "\n}\n");
    for request in ["set_content_type", "set_cursor_rectangle", "set_surrounding_text"] {
        assert!(apply.contains(request), "enabling text input doesn't restore `{request}`");
    }
}

#[test]
fn wayland_text_input_does_not_take_a_window_lock_under_the_data_lock() {
    // `Window` is Send + Sync and `set_ime_allowed`/`reset_ime` may run on another thread. The
    // data lock must be released before dispatch waits for the window-state lock; the reverse
    // order (window lock held while taking the data lock) is the only permitted nesting.
    let text_input = source("linux/wayland/seat/text_input/mod.rs");
    let event = section(&text_input, "impl Dispatch<ZwpTextInputV3", "\n    }\n}\n");
    let enter = section(event, "TextInputEvent::Enter", "TextInputEvent::Leave");
    let data_lock = enter.find("data.inner.lock()").expect("enter doesn't set the owner");
    let window_lock = enter.find("window.lock()").expect("enter doesn't lock the window");
    assert!(data_lock < window_lock, "enter takes the window lock before the owner setup");
    assert!(
        enter[data_lock..window_lock].contains(";\n"),
        "enter waits for a window lock while still holding the data lock"
    );
    for method in ["fn preedit_string(", "fn commit_string(", "fn delete_surrounding_text("] {
        let body = section(&text_input, method, "\n    }\n");
        assert!(
            !body.contains("window.lock()") && !body.contains("windows.get"),
            "{method} takes a window lock while handling incoming data"
        );
    }
}

#[test]
fn wayland_ime_lifecycle_fences_late_incoming_events() {
    // A disable or reset must not just clear the buffered fields: a `done`/`commit_string`
    // already in flight would otherwise be applied after `Ime::Disabled`. Incoming changes are
    // still applied for a `done` whose serial differs from the latest commit within the same
    // enabled generation, as text-input-v3 requires.
    let text_input = source("linux/wayland/seat/text_input/mod.rs");
    let finish = section(&text_input, "fn finish_done_with_preedit_policy(", "\n    }\n");
    assert!(
        finish.contains("let Some(incoming_from) = self.incoming_from else {"),
        "a late done is applied without a lifecycle generation"
    );
    assert!(
        finish.contains("serial_at_least(done_serial, incoming_from)"),
        "a done is gated on an exact serial match instead of the lifecycle fence"
    );
    assert!(
        text_input.contains("fn serial_at_least(serial: u32, floor: u32) -> bool {")
            && text_input.contains("serial.wrapping_sub(floor)"),
        "the commit serial fence doesn't wrap"
    );
    let disable = section(&text_input, "fn disable_generation<T: TextInputRequests>(", "\n}\n");
    let enable = section(&text_input, "fn enable_generation<T: TextInputRequests>(", "\n}\n");
    assert!(
        disable.contains("data.request_commit()"),
        "disable doesn't account for its commit serial"
    );
    assert!(
        enable.contains("data.send_enabled_commit(input, state, Some(ImeTextChangeCause::Other))"),
        "enable doesn't account for its commit serial"
    );
    assert!(
        disable.contains("end_incoming_generation()"),
        "disabling leaves the old incoming generation active"
    );
    // Queued events are tagged with the generation and checked again at delivery.
    assert!(
        finish.contains("events.push_ime_event("),
        "a delivered IME event isn't tagged with its lifecycle generation"
    );
    let sink = source("linux/wayland/event_loop/sink.rs");
    let into_deliverable = section(&sink, "fn into_deliverable(", "\n    }\n");
    assert!(
        into_deliverable.contains("if !meta.token.allows(meta.committed_input)"),
        "a stale queued IME event isn't checked at delivery"
    );
    assert!(
        sink.contains("committed_input: bool") && sink.contains("fn push_ime_commit("),
        "delivery doesn't distinguish accepted commit transactions from retractable composition \
         state"
    );
    let enter = section(&text_input, "TextInputEvent::Enter", "TextInputEvent::Leave");
    assert!(
        enter.contains("previous_id != window_id")
            && enter.contains("detach_input_from_window(&mut previous_window, text_input)"),
        "enter without a prior leave can leave a stale window entry or preedit owner"
    );
    assert!(
        enter.contains("push_window_event_at(") && enter.contains("WindowEvent::Ime(Ime::Enabled)"),
        "enter's Enabled is still tied to the retractable generation token"
    );

    let event_loop = source("linux/wayland/event_loop/mod.rs");
    assert!(
        event_loop.contains("buffer_sink.append_ordered(&mut window_events);")
            && event_loop.contains("buffer_sink.append_ordered(&mut state.events_sink);"),
        "synthetic and compositor IME events aren't merged into one push order"
    );
    assert!(
        event_loop.contains("drain_deliverable_for(|window_id|")
            && event_loop.contains("requests.is_closed()"),
        "queued IME events aren't checked against the window tombstone before delivery"
    );
}

#[test]
fn wayland_ime_teardown_is_per_window_and_cleans_a_removed_seat() {
    // `enter` goes to every seat's text input on a window, so a leave from one seat must not
    // report IME off for the others. The last leave, and a removed seat, still must clear the
    // window's IME state instead of leaving a destroyed proxy in `text_inputs`.
    let state = source("linux/wayland/window/state.rs");
    let text_input_left = section(&state, "pub(crate) fn text_input_left(", "\n    }\n");
    assert!(
        text_input_left.contains("TextInputLeft::Last")
            && text_input_left.contains("TextInputLeft::Remaining")
            && text_input_left.contains("self.text_inputs.is_empty()"),
        "leaving one of several seats can't be told apart from leaving the last one"
    );

    let text_input = source("linux/wayland/seat/text_input/mod.rs");
    let leave = section(&text_input, "TextInputEvent::Leave", "TextInputEvent::PreeditString");
    assert!(
        leave.contains("push_ime_teardown(")
            && leave.contains("detach_input_from_window(&mut window, text_input)"),
        "leave emits a window-wide Disabled regardless of the remaining seats"
    );
    assert!(
        leave.contains("data.inner.lock().unwrap().surface.clone()")
            && leave.contains("stored_id")
            && leave.contains("or_else"),
        "leave trusts a nil/event surface instead of the stored owner or a fallback"
    );
    let teardown = section(&text_input, "pub(crate) fn push_ime_teardown(", "\n}\n");
    assert!(
        teardown.contains("if clear_preedit {")
            && teardown.contains("left == TextInputLeft::Last && was_active"),
        "the preedit clear and Disabled boundary aren't tracked separately"
    );

    let seat = source("linux/wayland/seat/mod.rs");
    let remove_seat = section(&seat, "fn remove_seat(", "\n    }\n");
    for cleanup in [
        "data.end_owner()",
        "window.text_input_left(&text_input)",
        "window.ime_delete_surrounding = None",
        "text_input.destroy()",
        "push_ime_teardown(",
    ] {
        assert!(remove_seat.contains(cleanup), "removing a seat doesn't `{cleanup}`");
    }
    let detach = remove_seat
        .find("window.text_input_left(&text_input)")
        .expect("remove_seat never detaches the proxy");
    let sample =
        remove_seat.find("data.is_active()").expect("remove_seat never samples the old generation");
    assert!(
        detach < sample,
        "remove_seat samples is_active before detaching under the window lock"
    );
}

#[test]
fn wayland_surrounding_text_deletion_goes_with_a_commit() {
    // text-input-v3 applies a deletion after removing the preedit and before inserting the
    // commit. winit 0.30's `Ime` has no deletion event, so the deletion is handed to the window
    // as its commit is delivered, and a deletion without a commit gets an empty one.
    let text_input = source("linux/wayland/seat/text_input/mod.rs");
    let done = section(&text_input, "TextInputEvent::Done { serial } => {", "\n            },\n");
    assert!(
        done.contains("finish_done_with_preedit_policy("),
        "done doesn't apply the pending batch"
    );
    let batch = section(&text_input, "fn finish_done_with_preedit_policy(", "\n    }\n");
    assert!(
        batch.contains("self.batch.commit = Some(String::new());"),
        "a deletion without a commit isn't delivered"
    );
    assert!(
        batch.contains("events.push_ime_commit(text, owner, &token, delete);"),
        "a commit doesn't carry its deletion or preedit-owner policy"
    );
    let event_loop = source("linux/wayland/event_loop/mod.rs");
    assert!(
        event_loop.contains("drain_deliverable_for(|window_id|")
            && event_loop.contains("for (event, delete) in deliverable"),
        "delivering doesn't use the lifecycle- and tombstone-aware queue entry"
    );
    assert!(
        event_loop.contains("window.lock().unwrap().ime_delete_surrounding = delete;"),
        "delivering a commit doesn't hand its deletion to the window"
    );
}

#[test]
fn wayland_unchanged_surrounding_text_sends_nothing() {
    // Each text-input commit makes the compositor send `done`, and input-method-v2 compositors
    // ignore input-method commits against an older serial, so an app that sets the surrounding
    // text every frame would lose typed text.
    let state = source("linux/wayland/window/state.rs");
    let set =
        section(&state, "pub(crate) fn set_ime_surrounding_text_with_change_cause(", "\n    }\n");
    assert!(
        set.contains("if self.ime_purpose == ImePurpose::Password {"),
        "a password field can publish surrounding text"
    );
    let early_return = set
        .find("if self.ime_surrounding_text.as_ref().is_some_and(unchanged)")
        .expect("unchanged surrounding text is sent again");
    let commit = set
        .find("text_input.sync_ime_state(&state, Some(cause))")
        .expect("surrounding text isn't sent");
    assert!(early_return < commit, "unchanged surrounding text is sent again");
}

#[test]
fn wayland_surrounding_text_clear_is_an_explicit_unavailable_context() {
    // An available empty document and "unavailable" are different IME contexts. Only a committed
    // disable/enable can make an already-published surrounding text unavailable again, and a
    // disabled window must not be re-enabled by clearing.
    let text_input = source("linux/wayland/seat/text_input/mod.rs");
    assert!(
        text_input.contains("ZwpTextInputV3::set_text_change_cause(self, cause);")
            && text_input.contains("ImeTextChangeCause::Other => ChangeCause::Other"),
        "the surrounding-text change cause isn't sent through text-input-v3"
    );
    let reset_needed = section(&text_input, "pub fn context_reset_needed(", "\n}\n");
    assert!(
        reset_needed.contains("ime_allowed")
            && reset_needed.contains("has_entered_input")
            && reset_needed.contains("has_pending_or_active_incoming")
            && reset_needed.contains("had_surrounding_text")
            && reset_needed.contains("has_visible_composition"),
        "clearing ignores an active/buffered generation or resets a disabled context"
    );
    let state = source("linux/wayland/window/state.rs");
    let needs = section(&state, "fn needs_context_reset(", "\n    }\n");
    assert!(
        needs.contains("context_reset_needed(")
            && needs.contains("has_pending_or_active")
            && needs.contains("TextInputData::has_pending_or_active")
            && needs.contains("self.ime_preedit_owner.has_owner()"),
        "clearing ignores pending incoming input or the visible composition"
    );
    let clear = section(&state, "fn clear_ime_context(", "\n    }\n");
    assert!(clear.contains("self.needs_context_reset("), "clearing has no guard");
    assert!(
        clear.contains("text_input.reset_with_state(&state);"),
        "unavailable surrounding text isn't invalidated with a committed reset"
    );
    assert!(
        clear.contains("self.outgoing_ime_state()"),
        "the reset doesn't resend the remaining IME state"
    );
    let public = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/wayland.rs"),
    )
    .expect("platform/wayland.rs is readable");
    assert!(
        public.contains("fn clear_ime_surrounding_text(&self);"),
        "WindowExtWayland has no explicit unavailable-context API"
    );
    let external_default = section(
        &public,
        "fn set_ime_surrounding_text(&self, text: &str, cursor: usize, anchor: usize) {",
        "\n    }\n",
    );
    assert!(
        external_default.contains("ImeTextChangeCause::Other")
            && public.contains("fn set_ime_surrounding_text_with_change_cause(")
            && public.contains("best-effort on the wire"),
        "the public setter doesn't report an external cause or qualify available-empty"
    );
    let window = source("linux/wayland/window/mod.rs");
    let cursor = section(&window, "pub fn set_ime_cursor_area(", "\n    }\n");
    assert!(
        !cursor.contains("if window_state.ime_allowed()"),
        "a cursor area set while IME is disabled is discarded"
    );
}

#[test]
fn windows_reset_ime_cancels_the_composition() {
    // `CPS_CANCEL` discards the composition string; `CPS_COMPLETE` would commit it instead. The
    // window leaves `Preedit` before cancelling, so `WM_IME_ENDCOMPOSITION` doesn't commit the
    // result string, and clears the preedit itself, since IMEs differ in what they send.
    let ime = source("windows/ime.rs");
    assert!(
        ime.contains("ImmNotifyIME(self.himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0)"),
        "the composition isn't cancelled"
    );
    let window = source("windows/window.rs");
    let reset = section(&window, "pub fn reset_ime(&self) {", "\n    }\n");
    assert!(reset.contains("RESET_IME_MSG_ID"), "reset_ime doesn't reach the window procedure");
    let event_loop = source("windows/event_loop.rs");
    let handler = section(&event_loop, "msg == RESET_IME_MSG_ID.get()", "result = ProcResult");
    let leave_preedit = handler.find("w.ime_state = ImeState::Enabled;");
    let cancel = handler.find(".cancel_composition()");
    assert!(
        leave_preedit.is_some() && cancel.is_some() && leave_preedit < cancel,
        "reset_ime cancels while still in Preedit, so ENDCOMPOSITION can commit"
    );
    assert!(
        handler.contains("Ime::Preedit(String::new(), None)"),
        "reset_ime doesn't clear the preedit"
    );
}

/// A method in `WinitView`'s `define_class!` block.
fn view_method<'a>(view: &'a str, signature: &str) -> &'a str {
    section(view, signature, "\n        }\n")
}

#[test]
fn macos_composition_commits_after_marked_text_is_cleared() {
    // winit 0.30.13 treats `insertText` as an IME commit only while marked text is left. Input
    // methods that end a composition with `unmarkText` first lost the committed text, and Apple
    // Korean, which clears its marked text before inserting the key typed after a syllable (`한`
    // then `5`), lost that key. `insertText` commits when a composition was started
    // (rust-windowing/winit#4650, the 0.30 backport of #4651), and a commit outside a key press
    // resets the state itself, since no key follows to do it (#4748).
    let view = source("macos/view.rs");
    let set_marked_text = view_method(&view, "fn set_marked_text(");
    assert!(
        set_marked_text.contains("self.ivars().pending_commit.set(true);"),
        "setMarkedText doesn't start a pending commit"
    );
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("if pending_commit && ime_enabled && !is_control {"),
        "insertText commits only while marked text is left"
    );
    assert!(
        insert_text.contains("if self.ivars().in_key_down.get() {")
            && insert_text.contains("self.ivars().ime_state.set(ImeState::Ground);"),
        "a commit outside a key press swallows the next key"
    );
    let attributes = view_method(&view, "fn valid_attributes_for_marked_text(");
    assert!(
        attributes.contains("NSString::from_str(\"NSUnderline\")"),
        "validAttributesForMarkedText lost the underline attribute"
    );
}

#[test]
fn macos_disallowing_ime_discards_the_composition() {
    // Toolkits disallow IME when a text field loses focus. winit 0.30.13 then forgets the marked
    // text but doesn't tell the input method, which continues the composition once IME is allowed
    // again: the abandoned syllable comes back in the next field (`하`, then `가` in the next
    // field, became `학`). The input method is told to discard it (rust-windowing/winit#4745).
    let view = source("macos/view.rs");
    let set_ime_allowed = section(&view, "pub(super) fn set_ime_allowed(", "\n    }\n");
    assert!(
        set_ime_allowed.contains("input_context.discardMarkedText();"),
        "disallowing IME leaves the input method's composition"
    );
}

#[test]
fn macos_text_inserted_outside_a_key_press_is_committed() {
    // The emoji picker inserts its text with `insertText` outside any key press and without a
    // composition. winit 0.30.13 reports `insertText` only as part of a key press or a
    // composition, so the emoji was dropped. Such text is committed (rust-windowing/winit#4749,
    // the 0.30 backport of #4748).
    let view = source("macos/view.rs");
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("match plain_insert(&string, key_characters.as_deref()) {")
            && insert_text.contains("PlainInsert::Commit if self.ivars().ime_allowed.get() => {"),
        "text inserted outside a key press is dropped"
    );
}

#[test]
fn macos_text_typed_without_a_composition_is_committed() {
    // Chinese input methods type full-width punctuation (`，` for `,`) and Japanese ones an
    // ideographic space for Space with `insertText` and no composition. winit 0.30.13 sends the
    // key instead, with the key's own text. Text other than the key's is committed and the key's
    // text isn't sent (`plain_insert`).
    let view = source("macos/view.rs");
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains("self.ivars().key_characters.replace(Some(characters));"),
        "keyDown doesn't tell insertText which characters the key types"
    );
}

#[test]
fn macos_input_methods_see_a_caret_and_replace_text() {
    // winit 0.30.13 reports no selection, so dictation inserts nothing (rust-windowing/winit#4666),
    // and ignores replacement ranges, so a press-and-hold accent is typed after the letter instead
    // of replacing it (#3617). The view reports a caret after the text typed since it last moved
    // (`ImeDocument`) and has the app delete replaced text with the commit that replaces it.
    let view = source("macos/view.rs");
    let selected_range = view_method(&view, "fn selected_range(&self) -> NSRange {");
    assert!(selected_range.contains("ime_document"), "selectedRange reports no caret");
    // With a caret, AppKit's press-and-hold takes the repeats of a held letter; they aren't typed.
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains("&& !held_for_accents {"),
        "a held letter repeats under the accent menu"
    );
    for method in ["fn set_marked_text(", "fn insert_text("] {
        assert!(
            view_method(&view, method).contains("delete = self.replaced(replacement_range);"),
            "{method} ignores its replacement range"
        );
    }
    // Text `insertText` drops (emoji picker or dictation text while IME isn't allowed) leaves the
    // document alone, or its caret falls behind the app's.
    let insert_text = view_method(&view, "fn insert_text(");
    let decided = insert_text.find("if pending_commit && ime_enabled").expect("no commit branch");
    assert!(
        !insert_text[..decided].contains("self.replaced("),
        "insertText replaces text before deciding whether to deliver it"
    );
}

#[test]
fn macos_replaced_text_is_deleted_by_byte_count() {
    // Toolkits delete a whole grapheme cluster per Backspace, so one Backspace per replaced
    // character deleted text before the replaced range after a combining accent, a flag or a ZWJ
    // emoji. The deletion goes with a commit (`WindowExtMacOS::take_ime_delete_surrounding`), as
    // on Wayland.
    let view = source("macos/view.rs");
    assert!(!view.contains("NamedKey::Backspace"), "replaced text is deleted with Backspace");
    let queue_commit = section(&view, "fn queue_commit(", "\n    }\n");
    assert!(
        queue_commit.contains("view.ivars().ime_delete_surrounding.set(Some(delete));"),
        "the app can't take the deletion while it handles the commit"
    );
    let set_marked_text = view_method(&view, "fn set_marked_text(");
    assert!(
        set_marked_text.contains("self.queue_commit(String::new(), delete);"),
        "a deletion before a preedit isn't delivered"
    );
}

#[test]
fn macos_input_methods_can_call_a_view_without_a_window() {
    // An input method can ask for the caret rectangle or send text while the window is torn down.
    // winit 0.30.13 then panics on the missing window inside an Objective-C callback, which aborts
    // (rust-windowing/winit#4333, fixed on master by d6f7a28).
    let view = source("macos/view.rs");
    let first_rect = view_method(&view, "fn first_rect_for_character_range(");
    assert!(
        first_rect.contains("let Some(window) = (**self).window() else {"),
        "firstRectForCharacterRange needs a window"
    );
    let queue_event = section(&view, "fn queue_event(", "\n    }\n");
    assert!(
        queue_event.contains("let Some(window) = self.ivars()._ns_window.load() else {"),
        "queuing an event needs a window"
    );
    assert!(!view.contains(".expect(\"input context\")"), "a missing input context panics");
}

#[test]
fn macos_key_committed_through_ime_is_sent_once() {
    // Apple Korean answers Space with no syllable in progress with `setMarkedText(" ")`,
    // `insertText(" ")` and `setMarkedText("")`. The last call resets winit 0.30.13's IME state,
    // so the key arrived both as `Ime::Commit(" ")` and as `KeyboardInput` with text " ": two
    // spaces. A key whose text was committed isn't sent as `KeyboardInput` unless AppKit forwards
    // it as a command (Enter); the goal of rust-windowing/winit#4478, whose changes don't reach
    // this call order.
    let view = source("macos/view.rs");
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("self.ivars().committed_in_key_down.set(true);"),
        "a commit in keyDown isn't recorded"
    );
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains(
            "let had_ime_input = self.ivars().committed_in_key_down.replace(false) || \
             had_ime_input;"
        ),
        "a key committed through IME is sent again as KeyboardInput"
    );
}

#[test]
fn macos_reset_ime_discards_the_composition() {
    // `Window::reset_ime` ends a composition when the caret moves to other text with IME still
    // allowed. On macOS it has the input method discard its marked text, so the abandoned text
    // isn't continued or committed in the next field.
    let view = source("macos/view.rs");
    let reset_ime = section(&view, "pub(super) fn reset_ime(", "\n    }\n");
    assert!(
        reset_ime.contains("input_context.discardMarkedText();"),
        "reset_ime leaves the input method's composition"
    );
    let delegate = source("macos/window_delegate.rs");
    assert!(delegate.contains("self.view().reset_ime();"), "reset_ime isn't passed to the view");
}

#[test]
fn wayland_window_level_composition_and_delivery_seams_are_guarded() {
    // The public IME stream identifies a window, not a seat. Every accepted commit must emit the
    // documented empty Preedit before the commit and release the old visible owner; a non-owner
    // empty preedit must not clear or steal it. The event loop must recheck the closed-window
    // tombstone immediately before each callback, and all protocol state requests must wake the
    // loop even when no visible preedit produces a callback event.
    let text_input = source("linux/wayland/seat/text_input/mod.rs");
    let sink = source("linux/wayland/event_loop/sink.rs");
    let commit = section(&sink, "pub(crate) fn push_ime_commit(", "\n    }\n");
    assert!(
        commit.contains("Ime::Preedit(String::new(), None)") && !commit.contains("clear_preedit"),
        "push_ime_commit can omit the public clear before a non-owner commit"
    );
    let apply = section(&text_input, "fn finish_done_with_preedit_policy(", "\n    }\n");
    assert!(
        apply.contains("events.push_ime_commit(text, owner, &token, delete);")
            && apply.contains("if !outcome.committed && may_clear_preedit"),
        "the done reducer no longer separates commit clears from non-owner empty preedits"
    );
    let state = source("linux/wayland/window/state.rs");
    assert!(
        state.contains("pub(crate) fn after_done(")
            && state.contains("if outcome.committed {\n            self.owner = None;"),
        "a commit does not release the visible preedit owner"
    );

    let event_loop = source("linux/wayland/event_loop/mod.rs");
    let dispatch = section(&event_loop, "fn dispatch_ime_events<", "\n}\n");
    let production = section(&event_loop, "dispatch_ime_events(", "// Collect the window ids");
    assert!(
        dispatch.contains("sink::ime_event_window(&event)")
            && dispatch.contains("ime_window.is_some_and")
            && dispatch.contains("continue;")
            && production.contains("requests.is_closed()"),
        "the tombstone is checked only while draining, not immediately before each callback"
    );
    assert!(
        sink.contains("pub(crate) fn ime_event_window<")
            && sink.contains("WindowEvent::Ime(_) | WindowEvent::ImeResetCompleted { .. }"),
        "completion and native input use different closed-window classifications"
    );

    let purpose = section(&state, "pub(crate) fn set_ime_purpose(", "\n    }\n");
    assert!(
        purpose.contains("if !changed {") && purpose.contains("return ImeStateUpdate::default();"),
        "reasserting the same purpose can reset a password context every frame"
    );

    let window = source("linux/wayland/window/mod.rs");
    assert!(
        window.contains("fn wake_for_ime_state(")
            && window.contains("if update.sent {")
            && window.contains("self.wake_for_ime_state(update);"),
        "a background-thread IME protocol request can remain unflushed"
    );
    assert!(
        !text_input.contains(
            "push_ime_event_at(\n                        sequence,\n                        \
             Ime::Enabled"
        ),
        "Enabled is still tagged as retractable composition"
    );
}

#[test]
fn wayland_window_close_keeps_ime_and_selection_cleanup() {
    // W27's closed-window cleanup and W15P's per-window selection cleanup meet on the same close
    // branch, while W27 removed the global IME deletion deque where W15P stores its raw PRIMARY
    // offer. Keep both close calls, per-turn reconciliation and the field split: dropping either
    // leaves stale IME batches or selection reads bound to a dead window.
    let event_loop = source("linux/wayland/event_loop/mod.rs");
    let close = section(
        &event_loop,
        "if window_requests.get(window_id).unwrap().take_closed() {",
        "return Some(WindowEvent::Destroyed);",
    );
    assert!(
        close.contains("clear_ime_batch_for_window(&state.seats, *window_id);"),
        "closing a window drops its queued IME batch"
    );
    assert!(
        close.contains("state.clipboard.remove_window(crate::window::WindowId(*window_id));"),
        "closing a window leaves clipboard reads attached to it"
    );
    assert!(
        event_loop.contains("self.with_state(|state| state.clipboard.reconcile_selections());"),
        "the event-loop turn doesn't reconcile cleared selections"
    );

    let state = source("linux/wayland/state.rs");
    assert!(
        state.contains("pub pending_primary_offer: Option<ZwpPrimarySelectionOfferV1>"),
        "the PRIMARY raw-offer side channel is missing"
    );
    assert!(!state.contains("ime_deletions"), "the old global IME deletion deque was reintroduced");
}

// `ImeStartup` and `ImeDocument` are plain Rust, so their tests run on every platform.
#[allow(dead_code)]
#[path = "../src/platform_impl/macos/ime_startup.rs"]
mod ime_startup;

#[allow(dead_code)]
#[path = "../src/platform_impl/macos/ime_document.rs"]
mod ime_document;

/// `winit::platform::macos::ImeDeleteSurrounding`, which only macOS builds have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ImeDeleteSurrounding {
    pub before_bytes: usize,
    pub after_bytes: usize,
}

#[test]
fn macos_first_apple_korean_key_is_retried_once() {
    // On the first key in a freshly started app, Apple Korean can answer with `insertText` and the
    // raw compatibility jamo instead of starting a composition, so g k s typed ㅎㅏㄴ instead of
    // 한. That one key is interpreted again before it is sent to the app, under the conditions
    // `ImeStartup` checks (rust-windowing/winit#4744).
    let view = source("macos/view.rs");
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains(
            "let retry = self.ivars().ime_startup.borrow_mut().finish(serial, same_context);"
        ),
        "keyDown doesn't retry the first key"
    );
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("self.ivars().ime_startup.borrow_mut().insert(&string);"),
        "insertText doesn't report to the retry policy"
    );
}

// `SystemCaret` is plain Rust, so its policy tests run on every platform.
#[allow(dead_code)]
#[path = "../src/platform_impl/windows/ime_caret.rs"]
mod ime_caret;

#[test]
fn windows_system_caret_follows_the_ime_cursor_area() {
    // Some IMEs place their windows at the system caret and ignore `ImmSetCandidateWindow`
    // (Sogou, some Japanese IMEs). winit 0.30.13 creates no caret, so they couldn't find the
    // text being composed. A focused window that allows IME keeps a hidden caret over the IME
    // cursor area, under the rules `SystemCaret` tests.
    let window = source("windows/window.rs");
    let cursor_area = section(&window, "pub fn set_ime_cursor_area(", "\n    }\n");
    assert!(
        cursor_area.contains("state.system_caret.set_area(")
            && cursor_area.contains("ImeContext::update_system_caret(window, caret);"),
        "the caret doesn't follow the IME cursor area"
    );
    let allowed = section(&window, "pub fn set_ime_allowed(", "\n    }\n");
    assert!(
        allowed.contains("state.system_caret.set_ime_allowed(allowed)")
            && allowed.contains("ImeContext::update_system_caret(window, caret);"),
        "allowing IME doesn't create or destroy the caret"
    );
    let event_loop = source("windows/event_loop.rs");
    for (message, focused) in [("WM_SETFOCUS => {", "true"), ("WM_KILLFOCUS => {", "false")] {
        let handler = section(&event_loop, message, "\n        },\n");
        assert!(
            handler.contains(&format!("w.system_caret.set_focused({focused})"))
                && handler.contains("ImeContext::update_system_caret(window, caret)"),
            "{message} doesn't update the caret"
        );
    }
}

#[test]
fn windows_keys_taken_by_the_ime_are_not_sent() {
    // While an IME uses a key, `WM_KEYDOWN` carries `VK_PROCESSKEY`. winit 0.30.13 sent it as a
    // `Process` key press with the key's physical code, which apps that fall back to the physical
    // key (egui-winit) took as that key: Backspace or Enter in a composition also acted in the
    // app. Such a key-down is dropped unless the IME passes text on with it.
    let keyboard = source("windows/keyboard.rs");
    let key_down = section(&keyboard, "WM_KEYDOWN | WM_SYSKEYDOWN => {", "WM_DEADCHAR");
    assert!(
        key_down.contains(
            "if event_info.vkey == VK_PROCESSKEY {\n                            return \
             MatchResult::TokenToRemove(pending_token);"
        ),
        "a key the IME took is sent as a key press"
    );
}
