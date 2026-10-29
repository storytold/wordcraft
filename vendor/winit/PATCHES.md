# Craft suite patches on winit 0.30.13 (`craft/0.30.13`)

Base: upstream tag `v0.30.13` (`e9809ef5`). One commit per patch, original authorship kept. Drop a
patch when a winit release used by the suite contains it. Feature work does not belong here (pen
input lives in `craft-tablet`).

| # | Commit | Change | Upstream | Remove when | Source in the suite |
|---|---|---|---|---|---|
| W1 | `b357a332` | Windows 11: apply the suggested `WM_DPICHANGED` rect (window growth on mixed-DPI drags) | #4341, merged on master | a 0.30.x/0.31 release in use has it | pdfcraft vendor, wordcraft#787, cadcraft#668, photocraft#4007, gridcraft#761, storytold/winit PR #2 |
| W2 | `13ca693b` | Wayland: destroy a seat's text input only when the seat is removed | #4747, merged on master | same | pdfcraft vendor, wordcraft#787, cadcraft#668 |
| W3 | `fd104a4c` | Windows: IME caret from `GCS_CURSORPOS` only when reported (Korean caret) | #4746, open | upstream release | same |
| W4 | `3fffb7ce` | Windows: hidden maximized windows stay hidden until shown | #4587, open (#4575) | upstream release | same |
| W5 | `283370ce` | X11: focus the replacement IME context | #4727, open | upstream release | same |
| W6 | `a82b90fb` | macOS: NSTextInputClient commit flow | #4650 (0.30), open; #4651 merged on master | upstream release | same |
| W7 | `9b90d5bd` | macOS: discard marked text when IME is disabled | #4745, open | upstream release | same |
| W8 | `6a87addd` | macOS: emoji-picker text as `Ime::Commit` (dictation part of #4748 excluded) | #4749 (0.30), open | upstream release | same |
| W9 | `9500f25d` | macOS: no duplicate keyboard text for a Korean IME space | related #4478; to be proposed | upstream fix | same; makes designcraft#1197's workaround unnecessary |
| W10 | `4a7dd791` | macOS: retry the first Apple Korean key | #4744, open | upstream release | same; photocraft#606 |
| W11 | `4df8554b` | X11: scale factor from the server DPI before the panel's physical DPI | none; to be proposed | upstream fix | pdfcraft#1150; photocraft#3724, gridcraft#389, deckcraft#170, wordcraft#721 |
| W12 | `7facf001` | macOS: the view answers `mouseDownCanMoveWindow` from `movable_by_window_background` (title-bar drags reach the app) | none; to be proposed | upstream fix | pdfcraft vendor patch (12), pdfcraft#2347 |
| W13 | `5506b1a9` | Wayland: pixel-only wheel, wheel-tilt and continuous scrolls report `Moved`, so Ctrl zoom doesn't stay latched | none; to be proposed | upstream fix | pdfcraft vendor patch (13), pdfcraft#1569 |
| W14 | `c488d2e9` | macOS: opt-in cancelable termination → close request (+ `tests/macos_termination.rs`); `dpi` from crates.io so git consumers get one `dpi` | none; to be proposed | upstream fix | DesignCraft pin `3d51077f` |
| W15 | `cf73a29c`, `b05ce129` | Wayland: file drops through a per-seat data device; clipboard offered on the same device (`platform::wayland::Clipboard`) | #4729 (0.30), open; master has #4571 (+#4658) | upstream 0.30 release or the egui → 0.31 move | photocraft#7828, filmcraft#1115, pdfcraft#3254; needs egui fork E4 |
| W15P | local integration | Wayland: PRIMARY selection through one `zwp_primary_selection_device_v1` per seat, bounded asynchronous reads, token-gated regular/PRIMARY claims, active-input-seat routing, and offer/source lifecycle; `platform::wayland::Clipboard` exposes `input_origin`, `begin_load_text`, `claim_text`, `begin_load_primary_text`, `claim_primary_text`, `load_primary_text` and `store_primary_text` | not upstream; PRIMARY is a separate protocol object | keep; paired with egui E4 PRIMARY | clipboard routing/offer/source/worker regressions and the integrated close-path guard; native Hyprland/KWin/Mutter protocol trace and text transfer | replace with an upstream core PRIMARY API plus native evidence |
| W15I | local integration | Wayland: bounded text/PNG read, revoked-result validation, global image budget, reserved original-serial PNG ownership and bounded source serving | not upstream | keep; paired with egui PNG bridge | 198 library tests, 44 IME guards, strict lint; 25-check private-Sway egui probe; broader native matrix pending | replace with equivalent upstream API and qualified consumers |
| W17 | `2be27382` | Regression tests for W2, W3, W5–W10, W12, W13: source guards (`tests/ime_patches.rs`, `tests/pointer_patches.rs`) and the W13 scroll-phase unit tests | n/a (tests) | each guard goes with its patch | storytold/winit PR #3; #4 (pointer tests) |
| W18 | `894bf2d7` | Windows: a focused window that allows IME keeps a hidden system caret over the IME cursor area (Sogou, some Japanese IMEs place their windows at the caret) | none; to be proposed | upstream fix | storytold/winit PR #4; craft-libs#198 (I6) |
| W19 | `fa2e1b3d` | Windows: a key-down the IME takes (`VK_PROCESSKEY`) isn't sent as a key press unless the IME passes text on with it | none; to be proposed | upstream fix | storytold/winit PR #4; craft-libs#198 (I6) |
| W20 | `019c95f1` | X11: one input context per window; disallowing IME resets and unfocuses it, so input methods keep their state (Vietnamese Telex). Replaces W5's refocus | none (master still recreates the context); to be proposed | upstream fix (drop with W5) | storytold/winit PR #5; craft-libs#198 (I3) |
| W21 | `23bb19a7` | X11: send the IME spot once the input method has been quiet for 100 ms, so keys it forwarded aren't filtered again (garbled Vietnamese) | issue #4777; to be proposed | upstream fix | storytold/winit PR #5; wordcraft#2932 |
| W22 | `0816871e` | macOS: the view reports a caret (`ImeDocument`) and honours replacement ranges next to it (press-and-hold accents, dictation); text typed without a composition is committed (`，`, ideographic space); input methods calling a view without a window don't abort | teardown guard is master's d6f7a28 (#4333); #3617, #4666 open | upstream release | storytold/winit PR #6; craft-libs#198 (I5) |
| W23 | `c8cd8aa1` | macOS: keys pressed with ⌘ are named by the layout's ⌘ table (`KeyEventExtMacOS::key_with_command`); ⌘V on "Dvorak – QWERTY ⌘" | none; to be proposed | upstream fix | storytold/winit PR #6; pdfcraft#3655, craft-libs#198 (K1) |
| W24 | `4b5bc5a0` | `Window::reset_ime()`: cancel the composition without committing it (X11 `XmbResetIC`, Wayland disable/enable that rejects the old generation, Windows `CPS_CANCEL`, macOS `discardMarkedText`); text already committed is still delivered | none; to be proposed | upstream release with an equivalent | storytold/winit PRs #5, #6; craft-libs#198 (I4) |
| W25 | `eb4007da` | X11/Wayland: `platform::latin_layout::KeyEventExtLatinLayout`, the key on the first Latin layout group for shortcuts on non-Latin layouts | none; to be proposed | upstream release with an equivalent | storytold/winit PR #5; craft-libs#198 (K2) |
| W26 | `2b9cd1cf` | macOS: replaced text is deleted by UTF-8 byte count with the commit (`WindowExtMacOS::take_ime_delete_surrounding`, same shape as W27) | master: `Ime::DeleteSurrounding` (a new `Ime` variant breaks 0.30 matchers) | the egui → 0.31 move | storytold/winit PR #6 (`d103edae`) |
| W27 | `f40c76a2` | Wayland: text-input-v3 surrounding text (`WindowExtWayland::set_ime_surrounding_text`, `set_ime_surrounding_text_with_change_cause`, `clear_ime_surrounding_text`), delete-surrounding attached to the commit (`take_ime_delete_surrounding` → `ImeDeleteSurrounding { before_bytes, after_bytes }`), per-seat preedit ownership and outgoing state waiting for a matching `done` (E21b) | master has both as core API (`Ime::DeleteSurrounding`) | the egui → 0.31 move | storytold/winit PR #7; craft-libs#198 (I7) |

Conflict resolved by hand: `wayland/seat/mod.rs` `remove_seat` keeps all three cleanups — cancel a
file drag (W15), remove the seat from winit's clipboard (W15), destroy the seat's text input (W2).

W17–W27 are the language-support patches (storytold/craft-libs#198); W16 stays with the pinch/rotation
patch below.

W22 gives every macOS window with IME allowed a real caret, so input methods send replacement
ranges: press-and-hold accents, dictation corrections, and input methods that switch from marked
text to `insertText:replacementRange:` once they see a caret. An app must call
`WindowExtMacOS::take_ime_delete_surrounding` (W26) when it handles each `Ime::Commit`, or the
replaced text stays and the new text is typed after it. egui-winit `craft/0.37` calls it. With the
caret, a held letter in such a window also stops repeating, because AppKit's press-and-hold takes the
repeats, as in native text fields.

W19 needs a test on Windows before it is proposed upstream. It drops every `VK_PROCESSKEY` key-down,
so a key that ends a composition acts only if the IME passes the real key on afterwards. With the
Microsoft Korean IME (type 한) and the Microsoft Japanese IME (inside a composition), check that the
arrow keys, Enter, Tab and Backspace still act. If they don't, limit the suppression to keys the
composition consumes.

Not in this cut: W16 (Wayland pinch/rotation, #4778), the earlier tablet backport (replaced by `craft-tablet`).

### W27 outgoing IME state (E21b)

`text-input-v3` double-buffers client state. A `done` whose serial differs from the number of
`commit` requests still has its incoming `preedit_string`/`commit_string`/`delete_surrounding_text`
applied, but the client must not change the protocol's current state; pending state requests are
sent and committed after a matching `done`. The Wayland backend now keeps that state explicitly:

- Per text input, `SentContext` records the acknowledged context (`Disabled` or the
  `OutgoingImeState` of the last acknowledged commit), the context carried by the newest
  unacknowledged commit, and a pending `text_change_cause` for a coalesced surrounding-text edit.
  The desired snapshot lives on `WindowState` (`ime_purpose`, `ime_cursor_area`,
  `ime_surrounding_text`). The desired cursor area is updated even while IME is disabled, so the
  next enable publishes the current field's caret instead of the area cached before focus moved.
- `set_ime_surrounding_text`, `set_ime_surrounding_text_with_change_cause`,
  `set_ime_cursor_area`, and purpose changes publish immediately only when no commit is
  unacknowledged. Otherwise they coalesce into the desired snapshot; the matching `done`
  acknowledges the outstanding commit and sends at most one fresh commit with all current setters.
  Unchanged snapshots send nothing, a mismatched serial sends nothing, and a disabled context is
  never re-enabled by a setter or a `done`.
- `set_ime_surrounding_text` is the public application-originated setter and sends
  `set_text_change_cause(other)` with its commit. The new
  `set_ime_surrounding_text_with_change_cause` lets a caller report `input_method` when the
  surrounding text changed because it applied an IME commit or deletion. Every state commit that
  carries surrounding text names its cause explicitly: initial enable, reset, purpose and
  cursor-only snapshots send `Other`, while a coalesced `InputMethod` receipt keeps its cause when
  a geometry-only update arrives while the commit is pending. The cause is sent before the
  surrounding-text request, and the protocol resets it after every commit; winit does not infer
  origin from timing. A cause-only setter change is deduplicated with unchanged text.
- Every commit goes through `TextInputDataInner::request_commit`, so the serial is the wrapping
  `u32` count of all commits, including the committed enable/disable of a reset. Enable and reset
  send `enable` -> content type/cursor rectangle/surrounding text -> `commit`; a reset sends
  `disable`/`commit` first. This resends the full snapshot after enter/enable/reset, as the
  protocol requires.
- `OutgoingImeState::new` drops surrounding text for `ImePurpose::Password` at the protocol
  boundary, and `set_ime_surrounding_text` ignores calls while the purpose is `Password`.
  Entering `Password` forgets the cache and performs a committed reset whenever an entered text
  input has an active or buffered incoming generation, even if no surrounding text was cached and
  no preedit is visible. Repeating the same purpose is idempotent rather than a per-frame
  disable/enable; privacy still comes from the first transition and from the protocol-boundary
  text filter. The same reset backs the new
  `WindowExtWayland::clear_ime_surrounding_text`.
- `clear_ime_surrounding_text` is the explicit "unavailable" operation. It forgets the cached
  text, preserves `ime_allowed`, never enables a disabled window, and resets when text was
  published, a composition is visible, or an incoming generation is active/buffered. That last
  condition makes the clear independent of cached text and visible preedit: a delayed `done`
  cannot apply a commit or deletion from the previous context to the new field or password.
- `preedit_string`, `commit_string`, and `delete_surrounding_text` remain independent buffered
  records. `commit_string` no longer erases an independently buffered preedit; `done` applies the
  protocol order (clear old preedit, delete, commit, new preedit).
- One window has one public `Ime::Preedit` slot. Public composition is window-level: every
  accepted commit from any seat emits an empty `Preedit` immediately before its commit and
  releases the old visible owner, so `Ime::Commit`'s documented clear-before-commit contract holds
  even for a non-owner seat. A new visible preedit from any seat then claims ownership. A
  non-owner's empty-preedit-only `done` neither clears another seat's newer composition nor
  claims ownership. This is not a claim of simultaneous per-seat editing support.
- Lifecycle seams use the stored surface and the window lock: `leave` resolves the stored owner
  (falling back to the event surface), and `enter` detaches a previous window before installing
  the new one. Seat removal detaches the proxy under every window lock before sampling whether it
  was active, so a concurrent `set_ime_allowed` cannot leave `Enabled` without `Disabled` or emit
  two `Disabled` events. `Enabled` is an untagged window lifecycle notice, so reset/clear cannot
  retract it before delivery. The event loop rechecks the closed-window tombstone immediately
  before each callback, and every IME path that issued a protocol request wakes the loop, including
  background-thread clear/reset/purpose without a visible preedit.
- The wire has no per-record generation. A compositor that coalesces an old record and sends only
  a new `done` cannot be distinguished from a genuine new commit; the client applies current-
  generation records rather than silently discarding a possibly valid new commit. This remains a
  native protocol qualification gate.
- A cursor/purpose commit still re-sends the full cached snapshot (now with an explicit `Other`
  cause). Whether a given compositor re-evaluates or drops the preedit on that unchanged state,
  and whether a deferred post-commit snapshot reaches the wire before the application can compute
  it, need a native text-input-v3 trace; no such trace was run here.
- `WindowExtWayland` gains required `set_ime_surrounding_text_with_change_cause` and
  `clear_ime_surrounding_text` methods with no no-op defaults. This is a documented source-breaking
  change for external implementors of the fork trait.
- An available empty document stays internally distinct as `Some(("", 0, 0))` and is sent as an
  empty surrounding-text request, while unavailable omits that request after the committed reset.
  This is only best-effort on the text-input-v3 wire: the protocol has no separate availability
  bit, and its XML says empty initial values mean "unsupported" and may make later changes have
  no effect. No durable compositor guarantee or native evidence is claimed.
- Request-recorder regressions cover the repaired reducer: stale and matching `done`, coalesced
  setters and their cause (including cursor geometry preserving a pending `InputMethod` receipt),
  wrapping serials, disable/password clearing of active input without cached text, unavailable
  versus available-empty, multi-seat acknowledgement, reset/enter ordering, cursor retention into
  the next enable, both preedit/commit arrival orders, non-owner empty `done` and window-level
  commit policy, and the incoming mismatched-serial regression. The `tests/ime_patches.rs` guards
  keep the wire order, clear API, cause setter, detached stored-surface lifecycle, untagged
  `Enabled`, pre-callback tombstone recheck, wake-on-send, and disabled-cursor cache in place.

Locking: dispatch takes the window-state lock first and the text-input data lock second. The `done`
handler resolves the owner, releases the data lock, takes the window lock, then the data lock; it
never takes a window lock under the data lock.

Limits and verification still required: no build, test, fmt, clippy, compositor, or native input
method was exercised for this repair. A runtime trace on wlroots/KWin is still needed to confirm
that state-only commits produce the expected `done` serials, that a mismatched `done` is followed
by a matching one, that empty `set_surrounding_text` is retained where a compositor supports it,
that `set_text_change_cause` reaches the input method, and that predictive or Vietnamese input
keeps its composition while surrounding text changes. The egui-winit
`crates/egui-winit/src/ime_surrounding.rs` adapter should replace its `set_ime_allowed(false)` /
`set_ime_allowed(true)` clear with `WindowExtWayland::clear_ime_surrounding_text`; its planned
ClearThenSet order then becomes clear, then `set_ime_surrounding_text` for the new context. Use
`set_ime_surrounding_text_with_change_cause(..., ImeTextChangeCause::InputMethod)` when republishing
text after applying an IME commit/deletion, and the default setter for keyboard/model/selection
changes (which reports `Other`). The adapter's own cache is still needed for owner/context/purpose
transitions and password exclusion, but it no longer has to fake a clear through the enable/disable
events.

### W15P/W15I: integrated PRIMARY and bounded PNG clipboard

The combined tree preserves W17–W27 and adds one PRIMARY device per seat beside winit's
regular data device. Reads and claims route through the active input seat, target window,
focus epoch and opaque single-use-per-family input token. Closing a window clears both its
IME batch and clipboard requests; per-turn reconciliation notices cleared regular offers.

Regular reads resolve one offer, prefer supported text, and otherwise accept exact `image/png`.
Transfers have byte caps and absolute deadlines. Completed results are revalidated before delivery.
`PngBytes` shares an admission permit from the process-global 128 MiB image budget. The 32 MiB
owned-source ledger covers installed sources, rollback references and serving writers; it is a
payload-length admission bound, not an RSS bound.

`reserve_image_claim` captures the original input serial for off-thread encoding, and
`claim_reserved_image_png` installs on the event-loop thread after seat/window/focus/ownership
and deadline checks. A later regular copy attempt invalidates an older image reservation even
when the later attempt fails. PRIMARY claims remain independent. Ownership calls follow the
event-loop thread contract documented by `platform::wayland::Clipboard`.

The 2026-10-24 integration copied the qualified staging snapshot at base
`5b708e919e3506b8f8927651159204b1df2f648b`, including the IME source-guard signature and
bare-done/reset test corrections. Before the documentation update, every tracked and untracked
file matched the snapshot recorded by native probe `clipboard-image-smoke-20261024-025844-315945`.
Managed `image-tests-19` passed 198 library tests, 44 IME guards and strict library/test clippy.
The native egui-winit probe passed 25 private-Sway checks: external/self text and PNG, exact
RGBA/alpha, malformed/absent inputs, last-command selection, worker wakes and an unchanged
external PRIMARY sentinel. The earlier native text probe separately exercised PRIMARY transfer.

These checks do not qualify multi-seat routing, slow/trickling native peers, focus-transfer
races, other compositors, Windows/macOS, full application GUI behavior or performance. The
full publication matrix and final immutable dependency locks remain separate landing gates.
The plan document retains earlier boundary descriptions as historical implementation notes;
this entry records the integrated state.

## Ordered IME reset completion

`Window::reset_ime_with_request(request_id)` adds an ordered cancellation boundary for custom
editors. `WindowEvent::ImeResetCompleted { request_id }` identifies the requesting native window
and follows input already accepted by its old composition. Fresh input follows that boundary.
The existing `reset_ime()` API keeps its behavior. Integrations must correlate the request with
their current editor owner, context and purpose; a completion alone does not identify an editor.

X11, Wayland, Windows and macOS implement the boundary. Unsupported backends and failed native
resets do not fabricate completion. A valid empty native context can complete without visible
preedit. Wayland completion is a local ordered boundary, not a compositor acknowledgement.
Window or context invalidation can discard queued completion. Windows admits a
native message's copied result before invoking callbacks; macOS defers completion until an outer
text callback finishes publishing its accepted payload. Wayland publishes while holding the
window's input lock and checks closure again before each application callback.

Linux validation passed 207 library tests, 44 IME source guards and 17 other target tests, strict
all-target lint, reduced-feature X11/Wayland checks and nightly formatting. Two regression tests
first reproduced suffix overtaking and completion after closure. All targets also passed
cross-compilation checks for Windows x86_64 GNU and macOS AArch64. Cross-compilation does not
execute native tests. The main-thread AppKit harness, Windows native message/IMM tests, live
XIM/Wayland input methods, matching egui adapter and downstream custom editors remain separate
qualification gates. Human review and merge are required.
