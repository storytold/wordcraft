# Winit 0.30.13 fork plan for the egui 0.37 / language migration

Status: planning only. This document changes no production source, manifest, lockfile, branch,
tag or PR. No build, test, VM, compositor run or publication was performed while preparing it.
The local checkout is `codex/egui-037-plan-20261022` based on `origin/craft/0.30.13`.

The companion egui plan is `egui/docs/egui-037-migration-plan.md`; the suite-wide acceptance
protocol is `craft-libs/docs/egui-037-suite-rollout.md`. This plan covers only the
`storytold/winit` side and the API hand-offs those plans need.

Shared contracts apply here and are not restated as winit-only rules:

- `craft-libs/docs/egui-037-suite-rollout.md` owns the performance sample counts, noise policy and
  the shared lifecycle/capture/IME text contracts. Winit measurements use its minimums; app-level
  measurements also record the fixture, font and renderer fields the suite requires.
- `egui/docs/egui-037-migration-plan.md` section 10.6 and
  `evidence/additional-font-fixtures.json` pin the font sources, licences and SHA256 values used by
  the IME and text regressions. Winit does not bundle fonts, but every app-level IME/clipboard
  workflow below names the same fixture set.
- `evidence/lifecycle-source-audit.md` governs the logic/UI phase assumptions used by the E4/E19/
  E21 adapters. Winit does not decide when egui runs logic; it only delivers events and bounded
  transfer completions into the existing event loop.
- The shared capture slot contract (craft-libs plan) does not change winit event delivery. App
  screenshot/capture evidence is recorded by the app plan; winit only guarantees the event loop
  keeps running while a capture callback is pending.
- The E21b producer and the E4 transfer adapter belong to the egui plan; this plan fixes only the
  winit-side state, protocol and test contract they consume.

## 1. Decision

Keep the suite on winit **0.30.13** and its existing `craft/0.30.13` patch base. Integrate the
reviewed language-support series **W17-W27** into `storytold/winit`, add a bounded asynchronous
transfer API to W15, and add one new PRIMARY selection extension (provisionally called **W15P**)
required by the egui 0.37 E4 work. E21b is a paired egui prerequisite: W27's outgoing
surrounding-text API is incomplete without its egui-winit caller.

Do **not** upgrade to winit 0.31 in this scope. There is no verified suite need for a 0.31 move:
there is no stable 0.31 release in the evidence used here (the history shows a
`0.31.0-beta.3` version bump, not a qualified release), and the two egui patches that would
consume its core `Ime::DeleteSurrounding` (E21 and W26/W27) can stay on their 0.30-compatible
fork API until a separately reviewed migration has native evidence. A 0.31 move would also force new
review of every windowing consumer and is not a prerequisite for egui 0.37.

The target production shape is:

```text
winit 0.30.13
  storytold/winit craft/0.30.13 (ffae5198, W1-W15 published base; W5 explicitly reverted/replaced by W20)
    + reviewed language series W17-W27, including the W27 focus/owner/serial lifecycle revision
    + W15 bounded async transfer API
    + W15P PRIMARY path
    + tests (pinned xkb fixtures), bounded-transfer/IME lifecycle tests and PATCHES.md corrections
```

The aiongg branch `craft/0.30.13-lang` at `8b35388b` is an input and evidence source, not the
production pin. Its exact tree must be reviewed commit by commit into `storytold/winit`, with
authorship and co-authorship preserved, and consumers must use immutable `rev` pins, never a
branch name.

## 2. Exact baseline, candidate and dependent revisions

No final production SHA exists yet. A final `storytold/winit` revision must be a reachable,
reviewed commit on the `craft/0.30.13` line that contains the published base, the approved
W17-W27 integration, the W5/W20 disposition and the new W15P PRIMARY plus bounded-transfer work.
The final `storytold/egui` revision must contain E21b and the E4 hand-off. Until those commits
exist, plans and manifests use `<FINAL_STORYTOLD_WINIT_REV>` and
`<FINAL_STORYTOLD_EGUI_REV>`; do not invent, guess or copy a SHA from a candidate branch.

| Role | Revision | What it is |
|---|---|---|
| Upstream winit release | `e9809ef54b18499bb4f2cac945719ecc2a61061b` (`v0.30.13`) | Upstream base of the fork |
| Published fork base | `ffae5198770d7730e718043f035a772b7dbc3b1b` (`origin/craft/0.30.13`, local `craft/0.30.13`) | W1-W15, the current production base; it contains W5, which the final series must replace explicitly |
| Language candidate | `8b35388bd719e8cc86a95971ba0bc026c9a8f058` (`aiongg/craft/0.30.13-lang`) | W17-W27 research input only; this checkout is shallow and grafted |
| Historical full source | `/home/codex/work/CraftApps/craft-fork-20261020/winit-ai-lang` | Same `8b35388b` tree with individual W17-W27 commits; read-only evidence |
| Earlier published pre-view | `df33fb76` (`origin/craft/0.30.13-pre-publish`) | Historical W11-W13 staging; not a target |
| W16 only | `c08e9ab161fa82eb31f8f1d7acbc13069e6fbcf2` (`origin/codex/wayland-gestures-20261013`) | Pinch/rotation plus a native Wayland harness; explicitly outside this cut |
| Final winit prerequisite (no SHA yet) | `<FINAL_STORYTOLD_WINIT_REV>` | Approved, reachable `storytold/winit` commit containing W1-W15 minus W5, W20, W17-W27, W15P and the bounded-transfer API |
| egui fork research input | `a4f9a17f64a6bae61350fb85804c9df0d9865916` (`aiongg/craft/0.37`) | E1-E21 reference only; it has no E21b and no E4 PRIMARY routing |
| Final egui prerequisite (no SHA yet) | `<FINAL_STORYTOLD_EGUI_REV>` | Approved, reachable `storytold/egui` commit containing E1-E21, E21b and the E4/P2b hand-off |
| Published egui base | `767b1cad294120689a514bb4bd68c077e46679b9` (`storytold/craft/0.36.2`) | E1-E4 rollback base |
| egui E4 | `66bacf23` (0.37 branch), `c01acc2a` (0.36.2 backport) | Wayland clipboard routing; paired with W15 |
| craft-libs current tree | `49190884278f16c9a9f5823d7698a8ffb5deb062` | Current shared-library base for the CL stage |
| craft-libs PR 199 | head `b68de8c5f4f1c060904d211a7b1320aa0f7bce78`; audit tree `ec8840af8c5fda695cf195cdd05e5ae078415c07` | **Stale trees, not bases.** They omit `crates/platform` and revert current security/settings/docking/printer fixes. Reuse only selected API-port hunks from `8614e4a04fae516fba16c22b31ff1008cb6be10b` and relevant README changes from `1764bca` on `4919088`; never merge either tree |
| PdfCraft vendored winit | `e9809ef5` in `pdfcraft/vendor/winit` | W1-W13 only, source guards in the app; W5 requires rewrite, not retention |
| PdfCraft vendored egui-wgpu | `49682f8b` in `pdfcraft/vendor/egui-wgpu` | E2+E3; separate from this plan |

Approved reachable storytold forks are prerequisites, not assumptions: W15P and E21b are new work,
so no consumer may describe `8b35388b` or `a4f9a17f` as the final pin, and no plan may publish a
placeholder as a real SHA. The aiongg revisions remain research inputs whose individual commits
are reviewed into `storytold/winit` with authorship preserved.

Candidate caveat: the checkout here has only the grafted candidate commit, so `git show --stat`
against it reports the whole tree as added. The meaningful comparison is the tree diff:

```text
git diff --stat origin/craft/0.30.13 refs/remotes/aiongg/craft/0.30.13-lang
43 files changed, 2144 insertions(+), 175 deletions(-)
```

That diff contains W17-W27 only (plus `PATCHES.md`). It does not replace the published history
with a giant commit; the full historical clone is where the individual commits and messages are
readable.

## 3. Repository and target inventory

### 3.1 Packages, manifests and workspaces

- One root package, `winit` 0.30.13, plus the `dpi` subcrate (`dpi/Cargo.toml`).
- Root `Cargo.toml` is the workspace. There is no tracked `Cargo.lock` at the base. Do not add
  one to publication unless the repository's release process requires it.
- No `[patch.crates-io]` in the winit root. `dpi` is a registry dependency (`dpi = "0.1.1"`) after
  W14; keep it that way so a git consumer resolves one `dpi` family. Do not reintroduce a path
  dependency for publication.
- No `vendor/`, `vendored/` or source-replacement directory in this repository.
- Examples: `examples/window.rs`, `control_flow.rs`, `child_window.rs`, `pump_events.rs`,
  `run_on_demand.rs`, `x11_embed.rs`, `util/fill.rs`.
- Tests at base: `tests/macos_termination.rs`, `send_objects.rs`, `sync_object.rs`,
  `serde_objects.rs`. Candidate adds `tests/ime_patches.rs`, `tests/pointer_patches.rs`,
  `tests/macos_keyboard.rs`.
- No benchmark directory at base. Performance work below therefore needs a documented test or
  bench target, not an assumed existing Cargo bench.
- The egui E4 cfg includes Linux and the BSD family (`dragonfly`, `freebsd`, `netbsd`, `openbsd`);
  winit's unix platform cfg is the corresponding family. Compile at least one BSD target where
  the project's CI supports it, and do not add a Linux-only assumption to W15/W15P.

### 3.2 Features and target gates

Default features are `rwh_06`, `x11`, `wayland`, `wayland-dlopen`, `wayland-csd-adwaita`.
Relevant optional features are `wayland`, `x11`, `android-native-activity`,
`android-game-activity`, `serde`, `mint`, `rwh_04`, `rwh_05`, `rwh_06` and the
`wayland-csd-adwaita*` variants.

Required compatibility rows:

| Target / feature | Code that must compile | Notes |
|---|---|---|
| Linux X11, `--features x11` | `src/platform_impl/linux/x11/**`, W20/W21/W24/W25 | XIM, XCB/RandR, xkbcommon |
| Linux Wayland, `--features wayland` | `src/platform_impl/linux/wayland/**`, W2/W13/W15/W15P/W24/W27 | sctk 0.19.2, text-input-v3, data device, PRIMARY |
| Linux both `x11,wayland` | `src/platform_impl/linux/mod.rs`, `KeyEventExtra` | one `Window` enum with both backends |
| Windows MSVC | `src/platform_impl/windows/**`, W1/W3/W4/W18/W19 | IME, DPI, hidden maximized, caret |
| macOS | `src/platform_impl/macos/**`, W6-W10/W12/W14/W22/W23/W26 | AppKit, NSTextInputClient, termination |
| Android `android-native-activity` / `android-game-activity` | `src/platform_impl/android/**`, `Window::reset_ime` no-op | review FilmCraft Android work before changing |
| iOS, Web, Orbital | `Window::reset_ime` no-op and cfg-safe APIs | no native IME/PRIMARY claims |
| `docsrs` | `src/platform/latin_layout.rs`, platform extension traits | docs must expose all target APIs |
| `--no-default-features` + one backend | cfg combinations | no accidental Wayland/X11 dependencies |

`src/platform/latin_layout.rs` is gated with `any(x11_platform, wayland_platform, docsrs)`.
`KeyEventExtLatinLayout` is implemented only for Linux key events. `KeyEventExtMacOS`,
`WindowExtMacOS::take_ime_delete_surrounding`, `WindowExtWayland::set_ime_surrounding_text` and
`WindowExtWayland::take_ime_delete_surrounding` must remain target-gated; consumers must use the
same cfg gates as the egui-winit code that calls them.

`Window::reset_ime()` is public on every target: X11, Wayland, Windows and macOS implement it;
iOS, Android, Web and Orbital keep no-op stubs. That keeps `eframe`/app code portable without
pretending unsupported platforms have IME composition.

### 3.3 Current source anchors

The review uses these symbols rather than line numbers, which move:

- `src/window.rs:Window::reset_ime` (W24).
- `src/platform/mod.rs` latin-layout module gate (W25).
- `src/platform/latin_layout.rs:KeyEventExtLatinLayout::{latin_key,is_latin_layout}` (W25).
- `src/platform/macos.rs:WindowExtMacOS::take_ime_delete_surrounding`,
  `ImeDeleteSurrounding`, `KeyEventExtMacOS::key_with_command` (W23/W26).
- `src/platform/wayland.rs:Clipboard::{for_display,load_text,store_text}` (W15),
  `ImeDeleteSurrounding`, `WindowExtWayland::{set_ime_surrounding_text,take_ime_delete_surrounding}`
  (W27).
- `src/platform_impl/linux/common/xkb/keymap.rs:XkbKeymap::{latin_char,is_latin_layout}`,
  `LETTER_KEYCODES`, `is_latin_layout` helper (W25).
- `src/platform_impl/linux/common/xkb/mod.rs:KeyContext::process_key_event` (W25).
- `src/platform_impl/linux/mod.rs:KeyEventExtra` and `Window::reset_ime` (W24/W25).
- `src/platform_impl/linux/x11/ime/context.rs:ImeContext::{reset,set_allowed}` (W20/W24).
- `src/platform_impl/linux/x11/ime/mod.rs:Ime::{set_ime_allowed,reset}` (W20/W24).
- `src/platform_impl/linux/x11/event_processor.rs:{handle_ime_requests,send_ime_spots,ime_spots_deadline}`,
  `IME_SETTLE_TIME` (W20/W21).
- `src/platform_impl/linux/x11/mod.rs:poll_events_with_timeout` (W21).
- `src/platform_impl/linux/x11/window.rs:reset_ime` (W24).
- `src/platform_impl/linux/wayland/clipboard.rs:ClipboardState` and its seat/selection/serial state
  (W15/W15P).
- `src/platform_impl/linux/wayland/seat/text_input/mod.rs:TextInputDataInner`,
  `surrounding_text_excerpt`, `MAX_SURROUNDING_TEXT_BYTES` (W27).
- `src/platform_impl/linux/wayland/window/state.rs:{set_ime_surrounding_text,reset_ime,apply_ime_state}`,
  `ime_delete_surrounding` (W24/W27).
- `src/platform_impl/linux/wayland/event_loop/mod.rs` commit/deletion dispatch;
  `src/platform_impl/linux/wayland/state.rs:ime_deletions` (W27).
- `src/platform_impl/macos/ime_document.rs:ImeDocument` (W22/W26).
- `src/platform_impl/macos/view.rs:{set_marked_text,insert_text,key_down,queue_commit,reset_ime,
  selected_range,take_ime_delete_surrounding}` (W6-W10/W22/W23/W24/W26).
- `src/platform_impl/macos/command_key.rs` and `src/platform_impl/macos/event.rs` (W23).
- `src/platform_impl/windows/ime_caret.rs:{SystemCaret,CaretArea,CaretChange}` (W18).
- `src/platform_impl/windows/ime.rs:{get_composing_text_and_cursor,cancel_composition,
  update_system_caret}` (W3/W18/W24).
- `src/platform_impl/windows/keyboard.rs` `VK_PROCESSKEY` branch (W19).
- `src/platform_impl/windows/window.rs:{set_ime_cursor_area,set_ime_allowed,reset_ime}` (W18/W24).
- `src/platform_impl/windows/window_state.rs` visibility/maximized logic and `system_caret`
  (W4/W18).
- `tests/ime_patches.rs`, `tests/pointer_patches.rs`, `tests/macos_keyboard.rs`,
  `tests/macos_termination.rs`, and the native source guards in PdfCraft.

## 4. Patch provenance matrix, W1-W27 plus gaps

Status is a dated snapshot from the repository and evidence snapshots; it is not proof that an
upstream PR is merged today. "Remove when" is the retirement condition. Original commit authors
and co-authors must be preserved; do not squash away the human contribution.

| ID | Commit(s) | Change and source anchors | Upstream at inspection | Decision | Tests required | Remove when |
|---|---|---|---|---|---|---|
| W1 | `b357a332` | Windows 11 uses the `WM_DPICHANGED` suggested rect; `windows/event_loop.rs`, `dark_mode.rs` | #4341 merged on master, no 0.30/0.31 release used here | Keep | Windows 11 mixed-DPI drag; source guard; geometry unit test where factorable | Release in use contains #4341 and native mixed-DPI check passes |
| W2 | `13ca693b` | Wayland seat text input destroyed only with the seat; `wayland/seat/mod.rs` | #4747 merged on master | Keep | `tests/ime_patches.rs::wayland_text_input_lives_until_the_seat_goes`; native capability-loss test | Release contains #4747; guard travels with patch |
| W3 | `fd104a4c` | Windows `GCS_CURSORPOS` only when reported; `windows/ime.rs`, `event_loop.rs` | #4746 open | Keep | Korean IME caret native; source guard | Release contains equivalent |
| W4 | `3fffb7ce` | Hidden maximized windows stay hidden until shown; `windows/window_state.rs` | #4587 open (focus-safe form of #4575) | Keep | PdfCraft source guard; native restore-maximized at startup | Release contains equivalent |
| W5 | `283370ce` | X11 focus replacement IME context; old `event_processor.rs`/`ime/mod.rs` | #4727 open but superseded by W20 in this fork | **Drop via the S2 explicit revert/rebase**; retain only W20's replacement | W20 tests plus the rewritten PdfCraft W5 guard cover the behavior | W20 lands; never carry both |
| W6 | `a82b90fb` | macOS `NSTextInputClient` commit flow; `macos/view.rs` | #4650 open for 0.30; #4651 merged on master | Keep | source guard; macOS Apple Korean commit native | Release contains #4650/#4651 equivalent |
| W7 | `9b90d5bd` | macOS discard marked text when IME is disabled; `view.rs` | #4745 open | Keep | source guard; native focus-leaving-composition | Release contains equivalent |
| W8 | `6a87addd` | macOS emoji-picker text as `Ime::Commit`; `view.rs` (dictation part of #4748 excluded) | #4749 open for 0.30; #4748 on master | Keep | source guard; native emoji picker | Release contains emoji part |
| W9 | `9500f25d` | macOS no duplicate `KeyboardInput` for Apple Korean space; `view.rs` | Related #4478; to propose | Keep | source guard; native Korean space | Upstream fix / equivalent |
| W10 | `4a7dd791` | macOS retry first Apple Korean key; `ime_startup.rs`, `view.rs` | #4744 open | Keep | `ime_startup` policy tests; native first key | Release contains equivalent |
| W11 | `4df8554b` | X11 server DPI before panel physical DPI; `x11/util/randr.rs:default_dpi_factor` | No upstream PR | Keep | randr unit tests; native X11 96/144/192 DPI; PdfCraft source guard | Upstream equivalent and native check |
| W12 | `7facf001` | macOS `mouseDownCanMoveWindow` from `movable_by_window_background`; `view.rs`, `window_delegate.rs` | No upstream PR | Keep | source guard; native title-bar drag | Upstream equivalent and native drag |
| W13 | `5506b1a9` | Wayland pixel/wheel-tilt/continuous scrolls report `Moved`; `seat/pointer/scroll.rs`, `pointer/mod.rs` | No upstream PR | Keep | `tests/pointer_patches.rs` and `scroll::phase` unit tests; native Ctrl+wheel release | Upstream equivalent and native zoom check |
| W14 | `c488d2e9` | macOS opt-in termination to `CloseRequested`; `platform/macos.rs`, `app_state.rs`, `event_loop.rs`, `tests/macos_termination.rs`; registry `dpi` | No upstream PR | Keep | `tests/macos_termination` native scenarios | Upstream equivalent and native Quit/logout |
| W15 | `cf73a29c`, `b05ce129` plus a new transfer patch | Wayland file drops and clipboard through one per-seat data device; `seat/dnd.rs`, `wayland/clipboard.rs`, `platform/wayland.rs`. The new patch replaces `read_with_timeout`/per-send threads with bounded async transfers | #4729 open for 0.30; master #4571 (+#4658); no upstream bounded-transfer equivalent | Keep; paired with egui E4 | protocol trace/no second device; total-deadline, byte-cap, bounded-worker, cancellation and trickle/oversize tests; native Hyprland/KWin/Mutter copy/paste and self-paste; source guard | Release in use has #4729/#4571 and a bounded transfer API; E4 removal follows |
| W15P | New; no commit yet | PRIMARY selection on the existing connection/per-seat lifecycle; `platform/wayland.rs`, `wayland/clipboard.rs`, `wayland/state.rs`; reuses the bounded transfer API from W15 | Not upstream; separate protocol object | **Add**; paired with egui E4 PRIMARY | pure per-seat focus/offer/source/generation tests; keyboard and pointer triggering serials with origin and freshness; bounded async reads/writes; protocol trace; native Hyprland/KWin/Mutter matrix | Remove with E4 PRIMARY, or replace with an upstream core PRIMARY API plus native evidence |
| W16 | `c08e9ab1` (not in base) | Wayland pinch/rotation plus close-redraw change and native harness | Not part of this cut | **Exclude** | Separate plan and native gestures | Only a separately reviewed pinch plan |
| W17 | `2be27382` | Source guards for W2/W3/W5-W10/W12/W13 plus W13 policy tests | n/a (tests) | Keep, but strengthen | `tests/ime_patches.rs`, `tests/pointer_patches.rs`; mutation check each guard | Each guard is removed with its corresponding patch; replace textual guards with behavior tests where feasible |
| W18 | `894bf2d7` | Windows hidden system caret at IME cursor area; `windows/ime_caret.rs`, `ime.rs`, `window.rs`, `window_state.rs`, `event_loop.rs` | No upstream PR; to propose | Keep | `SystemCaret` unit tests; native Sogou/Japanese IME; focus loss destroys caret | Upstream equivalent |
| W19 | `fa2e1b3d` | Windows `VK_PROCESSKEY` key-down not sent as a key press unless WM_CHAR carries text; `windows/keyboard.rs` | No upstream PR; to propose | Keep | source guard; native IME Backspace/Enter/arrow | Upstream equivalent |
| W20 | `019c95f1` | X11 one input context per window; disallow resets and unfocuses the same context, allow refocuses it without remove_context; `x11/ime/*`, `event_processor.rs` | Master still recreates context; to propose | Keep; replaces W5 | source guard; native Vietnamese Telex/fcitx5; focus transitions | Upstream equivalent; remove with W5 row |
| W21 | `23bb19a7` | X11 IME spot queued until 100 ms quiet and no queued event; `event_processor.rs`, `x11/mod.rs` | Issue #4777; to propose | Keep | source guard + pure timer/deadline tests; native Vietnamese forwarded Backspaces | Upstream equivalent |
| W22 | `0816871e` | macOS `ImeDocument` caret/replacement/no-composition commits/teardown; `ime_document.rs`, `view.rs` | #3617 and #4666 open; teardown guard master `d6f7a28` (#4333) | Keep; co-author Jeremiah preserved | `ImeDocument` tests; native press-and-hold/dictation/punctuation/teardown | Release contains #3617/#4666 and equivalent |
| W23 | `c8cd8aa1` | macOS ⌘-table key naming; `command_key.rs`, `platform/macos.rs`, `event.rs`, `view.rs` | No upstream PR; to propose | Keep; event-time API, release pairing belongs to egui-winit P7 | `tests/macos_keyboard.rs`; native Dvorak–QWERTY ⌘/V; egui-winit P7 release-Command-before-letter test | Upstream equivalent |
| W24 | `4b5bc5a0` | `Window::reset_ime` for X11/Wayland/Windows/macOS; stubs on iOS/Android/Web/Orbital | No upstream PR; to propose | Keep | source guards per backend; native field-change/reset | Upstream release with equivalent |
| W25 | `eb4007da` | X11/Wayland Latin-layout key; `latin_layout.rs`, xkb keymap, `KeyEventExtra` | No upstream PR; to propose | Keep; required by E19. `latin_key`/`is_latin_layout` remain event-time and modifier-sensitive; egui-winit P7 pairs releases | deterministic pinned xkb fixtures (no host data); Dvorak/Russian/`ru,us`; native Russian/Greek/Arabic/Hebrew/Thai + Dvorak; egui-winit P7 layout-switch-while-held and non-clipboard shortcut tests | Upstream equivalent |
| W26 | `2b9cd1cf` | macOS replacement deletion as UTF-8 bytes with commit; `macos/ime_document.rs`, `view.rs`, `platform/macos.rs`, `window_delegate.rs` | Master has `Ime::DeleteSurrounding`; 0.30 enum is exhaustive | Keep; paired with E21 | `ImeDocument` byte tests; native accent/flag/ZWJ/dictation/one undo | E21/E21b and W26/W27 move to a winit 0.31/core API together |
| W27 | `f40c76a2` plus a lifecycle revision | Wayland text-input-v3 surrounding text and delete-surrounding; `platform/wayland.rs`, `seat/text_input/mod.rs`, `state.rs`, `event_loop/mod.rs`, `window/state.rs` | Master has both as core API; 0.30 enum is exhaustive | Keep; paired with E21 **and E21b**. W27 is incomplete without an egui-winit caller that sends fresh outgoing surrounding text | `surrounding_text_excerpt` tests; focus/owner epoch clearing on leave/disable/reset/seat removal; late `Done`; current-epoch serial mismatch vs stale epoch; context-transition disable/enable; incoming delete → commit → updated outgoing text on the wire; commit/deletion pairing and two-window dispatch; native Vietnamese/predictive IME | E21/E21b and W26/W27 move to core API together |

### 4.1 Provenance and gaps that must not be glossed over

1. **W5 and W20 overlap; the final base must remove W5 explicitly.** The published base
   `ffae5198` contains W5 (`283370ce`), and W20 (`019c95f1`) replaces its behavior. Do not say
   "W5 dropped" without an exact base. The implementation chooses one reviewed construction and
   records it in `PATCHES.md`:
   - final series from `ffae5198` with a reviewed `git revert 283370ce` (or an equivalent
     W5-free rebase) followed by W20 in the same stage; or
   - a rebuilt series from upstream `e9809ef5` plus W1-W4, W6-W15, then W17-W27 and W15P.

   Either construction must end with no W5 `remove_context` behavior and no W5 test asserting the
   old focus-replacement shape. Rewrite the PdfCraft vendor guard and patch-5 tests to W20's
   one-context-per-window / `x11_input_context_outlives_disallowing_ime` assertions, preserving
   W5's published intent (a focused replacement context that survives field focus changes) through
   W20's implementation. PATCHES.md marks W5 "superseded by W20", not "keep". Re-run the rewritten
   vendor guards against the final revision before removing `vendor/winit`.
2. **W16 is an explicit exclusion, not a missing language patch.** Its number stays reserved for
   the pinch/rotation work; W15P is a deliberately different provisional name. W16 exists only on
   `origin/codex/wayland-gestures-20261013` (`c08e9ab1`) with a separate native harness and
   `Event` API changes. Do not merge it as part of this work. Its native harness can be studied
   for clipboard/IME protocol testing, but only as a separate reviewed change.
3. **W17 is mostly textual.** Source guards prove that a patch's code is present; they do not
   prove the behavior. Keep them, but add behavior-level policy tests and mutation checks. W17
   does not cover W1, W4, W11, W14 or W15; those need their own tests (some already exist in
   PdfCraft).
4. **W15 has no test in the fork.** The published base added the clipboard/data-device code but no
   `tests/` coverage. A regular-clipboard pass is not PRIMARY acceptance and not proof that a
   second `wl_data_device` was avoided.
5. **W26/W27 are 0.30-specific API shapes.** They expose `ImeDeleteSurrounding` as a platform
   extension because `winit::event::Ime` is exhaustive in 0.30. When core `Ime::DeleteSurrounding`
   is adopted, delete the extension, the egui E21/E21b events and the guards together; do not leave
   two incompatible IME APIs.
6. **The candidate `PATCHES.md` is internally inconsistent** about W16 ("stays with the
   pinch/rotation patch below" while the bottom says "Not in this cut"). Correct the document in
   the integration PR, keeping W16 out.
7. **The candidate is shallow here.** Do not use its single commit as evidence that the published
   history was replaced, and do not write a plan around `git show`'s all-files-added output.
8. **W27's pending batch is per-`TextInputData` but persists across focus changes.** Static
   candidate inspection finds `TextInputEvent::Leave` clears only `surface`; `pending_commit`, `pending_preedit`,
   `pending_delete` and `preedit_shown` survive until re-entry. `set_ime_allowed(false)` and
   `reset_ime()` also do not clear them. The global `ime_deletions` queue is positional and carries
   no window or generation, but FIFO order alone is not proof of cross-window corruption: commits
   and deletions are pushed in the same order. The required fix is an explicit owner/focus epoch,
   clearing pending batch state on leave/disable/reset/seat removal, and dispatch-time validation
   of window plus epoch (tagged records or an equivalent pairing test). Preserve the commit/deletion
   pairing when replacing the queue.
9. **W15's transfer path is not bounded by the stated timeout.** `read_with_timeout` applies the
   2-second poll interval after each `WouldBlock`, so a trickling source can keep it alive
   indefinitely; there is no byte cap or cancellation. `load_text` reads synchronously on the
   caller's thread, and `send` spawns one unbounded thread per request while retaining an `Arc<str>`.
   The final W15 revision needs the bounded async transfer API described in 5.3; a synchronous
   compatibility wrapper may remain but is never used on the egui event path.

## 5. API contracts and compatibility

### 5.1 E19 hand-off: W25, W23 and egui-winit P7 release pairing

`KeyEventExtLatinLayout` exists only for X11/Wayland key events:

```rust
fn latin_key(&self) -> Option<Key>;
fn is_latin_layout(&self) -> bool;
```

- `latin_key` is the character the key types on the first Latin layout when the active layout is
  not Latin; it returns a non-letter character when the key has no Latin letter (Dvorak's `'` on
  the Russian `й` key). `None` means no active non-Latin layout or no Latin layout configured.
- `is_latin_layout` is decided for the whole layout by the 26 letter keycodes, so Dvorak, AZERTY
  and other Latin layouts keep their own letters.
- The Win32 side of E19 does not call W25; it uses `MapVirtualKeyExW` in egui-winit. The macOS side
  uses `KeyEventExtMacOS::key_with_command` (W23) while ⌘ is held and typed letters otherwise.
- E19 must compile with both `x11` and `wayland` features and with each separately. Its
  `RawInput::latin_layout_chars`/`latin_layout_char` path must preserve ordering through
  `append`/`take`; that is an egui-side test, but the winit extension must not add per-frame
  allocations.

`latin_key`, `is_latin_layout`, `logical_key` and `key_with_command` are **event-time** values.
They describe the current event under the current layout and modifiers; winit will not cache a
keydown translation or redefine all `logical_key` semantics. That narrower contract remains
documented because held-key release is a real lifecycle risk: a layout or modifier transition
between keydown and keyup can otherwise translate the physical key differently.

The repair belongs to egui-winit E19/P7, not to a global winit API change:

- P7 remembers the egui key actually emitted on keydown, keyed by the winit physical key, and uses
  that identity for repeats and the matching keyup. It keeps live text/modifier reporting separate,
  so `KeyEvent::text`, `logical_key` and the W25/W23 values remain as reported.
- Repeat events reuse the remembered identity and do not re-decide the logical key; a fresh press
  after release starts a new entry. An unidentified physical key uses the event-time logical key
  and is not forced into a Latin alias.
- Two physical keys that map to one logical key remain separate entries; releasing either removes
  only its own contribution, and egui's `keys_down` must not lose the other held key.
- Tracking is cleared on focus loss, IME interception, viewport destruction and reset. The cache is
  per window and bounded by the number of physically held keys.
- Copy/paste shortcuts return before emitting an egui Key-down in egui-winit, so Ctrl+C/Cmd+V
  cannot by themselves prove a stuck `keys_down` entry. Tests use a non-clipboard shortcut such as
  Ctrl+Z/Cmd+K plus ordinary held keys, and include release-Command-before-letter, layout switch
  while held, repeats, focus loss and a subsequent fresh press.
- The winit-side W25/W23 tests verify the documented event-time values on deterministic fixtures;
  the release-pairing tests live in egui-winit P7 and require no new winit public API beyond the
  existing physical key and pressed state.

Do not expose `KeyEventExtLatinLayout` on Windows, macOS or non-Linux targets. Do not add a generic
"latin layout" field to `KeyEvent` public API; `KeyEventExtra` remains crate-private and
target-specific. If a winit-side metadata change proves necessary, document the narrower reason and
add the equivalent egui-winit test in the same stage; do not silently change all key semantics.

### 5.2 E21/E21b hand-off: W26/W27 byte and lifecycle contract

The invariant is **the public IME deletion and surrounding-text offsets are UTF-8 bytes**, not
characters, UTF-16 units or grapheme clusters.

| Layer | Native unit | Conversion in the fork | Contract |
|---|---|---|---|
| Win32 `GCS_CURSORPOS` | UTF-16 code units | `ImeContext::get_composition_cursor` sums `char::len_utf8` | `Ime::Preedit` range is bytes |
| X11 XIM preedit | character positions | `calc_byte_position` sums `len_utf8` | `Ime::Preedit` range is bytes |
| Wayland text-input-v3 | bytes | protocol values used directly; `PreeditString` boundaries validated | `Ime::Preedit` range is bytes |
| macOS `NSRange` | UTF-16 code units | `ImeDocument::byte_offset` and `utf16_len` convert around the caret | `ImeDeleteSurrounding` is UTF-8 bytes |
| egui E21/E21b | bytes | `surrounding_bytes_to_chars` converts to character counts for `TextEdit` deletion; E21b produces byte indices | deletion is exact; a byte count ending inside a char leaves that char; outgoing indices are bytes |

#### 5.2.1 Incoming bytes and protocol order

1. `ImeDeleteSurrounding { before_bytes, after_bytes }` excludes the selection and the preedit:
   `before_bytes` ends where the preedit/selection starts, `after_bytes` begins where it ends.
2. Wayland `delete_surrounding_text` is applied in text-input-v3 order: remove preedit, delete
   surrounding bytes, insert commit, then receive updated outgoing surrounding text. A deletion
   without a commit gets an empty `Ime::Commit` so the app has an event to handle; the app must not
   see a deletion attached to a later unrelated commit.
3. macOS replacement ranges are only honoured when they reach the caret and start inside the
   known typed text. A range that needs the app's own text is ignored rather than guessed.
   `ImeDocument` keeps at most 256 characters (`KEPT_CHARS`), cuts at char boundaries, grows
   offsets across resets, and reports `before_bytes` for the replaced suffix. It never uses
   Backspace presses or grapheme counts. W26 remains a separate macOS path; this plan makes no new
   macOS surrounding-text API claim.
4. `set_ime_surrounding_text(text, cursor, anchor)` clamps offsets to the text, moves them back to
   char boundaries, and cuts to at most 4000 bytes around the cursor/selection without splitting a
   character. The cursor/anchor sent to the compositor are byte offsets in the excerpt.

#### 5.2.2 W27 focus, owner and serial lifecycle

Static candidate inspection (not a runtime reproduction) finds that
`pending_commit`, `pending_preedit`, `pending_delete` and `preedit_shown` survive
`TextInputEvent::Leave`, `set_ime_allowed(false)` and `reset_ime()`. A deletion or commit received
before focus loss can therefore be emitted after re-entry. W27's lifecycle revision must:

- Clear all pending per-surface batch state on leave, disable, reset and seat removal, and send the
  required empty preedit/disable boundary before the surface is reused.
- Carry an explicit owner/focus generation (epoch) plus the protocol commit serial. Tag pending
  deletion records with `{window_id, epoch, delete}` (or test an equivalent pairing) and validate
  both at dispatch. FIFO order alone is not proof of cross-window correctness; the test must exercise
  the actual dispatch/filtering path rather than assert a bug from the queue shape.
- Distinguish a **stale owner/focus generation** from a **current-generation serial mismatch**.
  Per text-input-v3, a `done` with a mismatched serial still applies incoming edits normally; only
  outgoing pending state waits for a matching serial. Do not drop a valid current-focus batch merely
  because its serial differs, and do not publish outgoing state under the wrong serial.
- Keep surrounding state valid until a committed enable/disable. Changing normal/password/
  unavailable support needs an explicit owner/context transition that clears the cache and commits
  the disable/enable boundary; an empty `set_surrounding_text` is not a general clearing API.
  A valid empty document context is distinct from unavailable support.
- Define preedit cancellation/reset: cancel the old preedit without inserting it into a new owner;
  same-owner ordinary text/selection updates preserve composition.

#### 5.2.3 E21b: outgoing surrounding text is a required hand-off

`WindowExtWayland::set_ime_surrounding_text` currently has no caller in egui-winit, and `IMEOutput`
has no logical text (`evidence/ime-surrounding-gap.md`). Incoming delete/commit is not full
surrounding-text support. W27's API is complete only when paired with an egui-winit caller that
sends the post-commit excerpt.

The egui plan owns E21b/P7b: `IMEOutput` gains an owner/context-generation identity and an explicit
available/unavailable state; an available payload contains logical committed text and UTF-8 byte
cursor/anchor; a valid empty document is distinct from unavailable support. `TextEdit` produces the
payload after applying deletion/commit/cursor changes, excluding preedit, and custom editors supply
model-aware adapters. egui-winit sends it on text/selection/IME-owner change through W27 on Wayland
only, and clears it on password/no-IME/focus loss/reset/unavailable transitions. The payload,
owner and descriptor are one atomic value across passes; a previous payload must not survive an
authoritative `None`.

Required protocol-sequence tests: incoming delete → commit → **updated outgoing surrounding text on
the wire**, deletion-only batches, field/window switching, password clearing, focus/reset/late
serials and two windows. W27 without E21b is unused API, not "complete IME support".

E21, E21b and W26/W27 must be landed as one dependency stage. E21 against base winit is a compile
error; W26/W27 without E21/E21b is unused API but still requires tests.

### 5.3 E4 hand-off: W15, bounded transfer and new W15P PRIMARY

The current W15 API is regular clipboard only and its synchronous read is unsuitable for a UI
event path:

```rust
winit::platform::wayland::Clipboard::
    for_display(NonNull<c_void>) -> Option<Clipboard>
    load_text(&self) -> io::Result<String>      // blocking; bounded compatibility only
    store_text(&self, impl Into<String>) -> io::Result<()>  // one thread per send today
```

It shares the per-seat `wl_data_device` used for file drops. A client must keep one regular data
device per seat; Hyprland is the observed one-device-per-client case in the upstream #8657
discussion. E4 uses this path and leaves smithay stopped, with arboard for images and a fallback
only for absent/non-text regular clipboard errors (`NotFound`/`InvalidData`). Present
empty text returns `Some("")` without fallback; test all three cases separately.

Available-empty surrounding text is an explicit `set_surrounding_text("", 0, 0)` plus
commit; track presence separately from string length. Unavailable omits that request
after the committed clear boundary. Returning support enables and supplies the payload
before commit; ordinary empty/non-empty edits do not reset composition. Follow egui P7b's
wire mapping and Chromium v3 reference; protocol/native tests cover initially empty,
non-empty→empty→non-empty, unavailable/password→available and ongoing composition.

#### 5.3.1 Bounded asynchronous transfer

`read_with_timeout` currently applies `READ_TIMEOUT` after each `WouldBlock`, so a source that
trickles bytes can keep the read alive indefinitely; there is no maximum byte count, cancellation
or total deadline. `load_text` reads on the caller's thread, and `send` spawns an unmanaged thread
per request while retaining the text. The final W15/W15P API replaces this with:

```rust
// Proposed names; ownership and routing are fixed by the contract below.
pub struct TransferOptions {
    pub target_window: WindowId,
    pub caller_generation: u64,
    pub total_deadline: Duration, // default 2 s from admission, never reset per poll
    pub max_bytes: usize,         // default 16 MiB; accept the 10 MiB fixture
    pub completion_waker: Arc<dyn Fn() + Send + Sync>,
}
pub struct SelectionClaim {
    pub target_window: WindowId,
    pub caller_generation: u64,
    pub trigger: InputOriginToken, // opaque winit-issued seat/event/epoch token
}
pub struct TransferHandle { /* owns result/cancel/wake slot; drop cancels */ }
impl TransferHandle {
    pub fn try_take(&self) -> Option<io::Result<Arc<str>>>;
    pub fn cancel(&self);
}
impl Clipboard {
    pub fn begin_load_text(&self, options: TransferOptions) -> io::Result<TransferHandle>;
    // begin_load_primary_text has the same handle contract.
    pub fn claim_text(&self, text: Arc<str>, claim: SelectionClaim) -> io::Result<()>;
    // claim_primary_text has the same synchronous ownership contract.
    // Only later pipe serving uses bounded workers.
}
```

Semantics:

- `claim_text`/`claim_primary_text` synchronously validate the opaque triggering token
  against its target window, focused seat and current ownership epoch, reserve bounded
  source bytes, and call `set_selection` in the same input turn. Return `Ok(())` only
  after admission/claim; invalid/stale origin returns `InvalidInput`, unsupported protocol
  `Unsupported`, and capacity rejection `WouldBlock`, before replacing ownership.
  No claim-completion handle or callback is needed. Compositor cancellation, replacement,
  seat/window destruction cancels that source and its serving jobs. Each serving write
  has the deadline/byte/worker bounds below; overload closes that pipe and records an
  error, never spawns another writer. Completion wake/caller cancellation apply to reads.
- The total deadline starts when the request is queued, not on each `WouldBlock`; expiry closes the
  pipe, cancels the read and returns `TimedOut`.
- The byte cap is enforced cumulatively; exceeding it returns `InvalidData` and cancels the
  transfer. A short read followed by a trickle cannot extend the deadline or the budget.
- Potentially blocking pipe reads and serving writes run on bounded workers: at most
  4 in-flight reads, 2 writer threads and queue depth 8. Synchronous ownership claims
  reserve source bytes and validate the triggering serial/seat before creating the source
  and sending set_selection in that same input turn. Never queue that claim behind I/O.
  Reject admission before replacing an owned source; serving overload closes/cancels
  that transfer with a recorded error, never blocks the event loop.
- TransferHandle owns the result/cancel/waker slot; workers hold Weak references.
  Bind admission to target_window and its validated per-seat input origin/epoch.
  Egui-winit stores the handle with ViewportId/focus/input generation in that window's
  State. Its waker requests repaint of that viewport via Context's existing callback
  to eframe's event-loop proxy. Take/revalidate once at that State's next input
  preparation. No generic broadcast or event-sink guess and no spin polling. Release
  the lock before wake; drop/cancel invalidates late results and releases the wake
  closure. Test two targets with delayed regular/PRIMARY requests, focus movement
  before dispatch, close and request-ID reuse.

- Cancellation occurs on explicit cancel, clipboard drop, seat removal, owner/selection change and
  stale request generation. The pipe fd and worker permit are released exactly once.
- The self-paste marker path returns the owned `Arc<str>` immediately, without a compositor round
  trip. A synchronous `load_text`/`store_text` compatibility wrapper may remain for non-egui
  callers, but it must use the same total deadline, byte cap and bounded worker admission; it is
  never called from egui-winit event handling.
- Tests cover a trickling source, an oversized source, repeated slow requests, cancellation on
  drop, self-paste, absent/text/image offers, owner changes, peak RSS/thread counts and actual
  event-loop responsiveness. Performance measures reads as well as writes.

#### 5.3.1a First image transport boundary (staging status)

The staging winit tree now contains only the first reviewed boundary of the Wayland image plan.
`platform::wayland::Clipboard::begin_load_regular(TransferOptions)` resolves exactly one
active-input-seat offer and returns `ClipboardContent::Text` (existing text MIME order) or
`ClipboardContent::ImagePng` (exact `image/png`, only when no text type is offered). The public
surface adds `IMAGE_PNG_MIME`, `ClipboardError`, `ClipboardContent`, `RegularTransferHandle` and
`PngBytes`. `ClipboardError::io_kind()` preserves the legacy text mapping, including
`WouldBlock` for source/worker capacity and backend kinds for other failures. Existing text and
PRIMARY APIs and signatures are unchanged.

The handle freezes the window, seat token, focus epoch and exact offer or owned-source token.
`try_take` revalidates that route, so a result completed before focus loss, owner change, seat
removal or window close becomes `Interrupted` instead of escaping. Keyboard focus loss interrupts
both families. A stored but unconsumed result is replaced without a second wake, cancellation does
not wake, and user callbacks never run under a clipboard or slot lock. The absolute deadline covers
queue wait, pipe I/O and text conversion.

`PngBytes` is immutable, clone-shares one budget permit, and is admitted against a process-global
128 MiB capacity budget at `2 * max_bytes` per image read (at most four full-cap reads, including
queued and completed-but-unconsumed requests). The 16 MiB per-transfer/per-source caps remain.

The owned-source ledger has a hard 32 MiB admission cap. Installed sources, replaced sources held
for rollback until their protocol flush completes, and serving-writer clones all remain charged;
a replacement never discounts the old source while the install path still holds it for rollback.
A new source must fit beside those retained charges or the claim is refused before ownership
changes. This ledger is not an RSS bound: it charges payload length rather than allocation
capacity, and it excludes read results already handed to a caller, including self-paste clones.
Retained PNG capacity is covered separately by the 128 MiB image permit above.

`PngBytes::admit` exists as fallible off-thread encoder admission groundwork.

Not implemented here: `reserve_image_claim`, `claim_reserved_image_png`, image source serving,
self-image sources, the egui-winit decode/encode workers and composed bridge, and all native image
evidence. No Cargo, test, fmt, clippy, metadata, VM or compositor command has been run on this
staging tree for this boundary; the parent must run managed checks and the native gates separately.
This is a local implementation gate, not publication or app acceptance.

#### 5.3.2 Per-seat PRIMARY state

PRIMARY is a **separate protocol object**. W15P is a winit-owned path on the same connection/event
queue and per-seat lifecycle, using sctk 0.19.2's existing `primary_selection` module
(`PrimarySelectionManagerState`, device/offer/source types and their handler traits). Do not bind a
second regular `wl_data_device` for PRIMARY. A protocol trace must show one regular
`wl_data_device` and one `zwp_primary_selection_device_v1` per seat.

The public PRIMARY transfer shape mirrors W15 (provisional names, same `TransferOptions` and
`TransferHandle`):

```rust
pub fn begin_load_primary_text(&self, options: TransferOptions) -> io::Result<TransferHandle>;
pub fn claim_primary_text(&self, text: Arc<str>, claim: SelectionClaim) -> io::Result<()>;
// Same synchronous admission/claim and bounded asynchronous serving as claim_text.
```

The existing `note_input` path is not sufficient: it records any input serial and sets
`active_seat` unconditionally, has no keyboard-focus flag, and `active_device` falls back to
`seats.first()`. W15P needs separate per-seat PRIMARY state with at least:

- `keyboard_focus: bool`, set only by keyboard `Enter` and cleared on keyboard `Leave`;
- the last keyboard `Enter`/key-press serial and the last pointer press/release serial, each with
  origin, timestamp and ownership epoch, kept distinct;
- the active PRIMARY offer, the owned source and its epoch/generation.

The packaged `primary-selection-unstable-v1.xml` permits pointer-driven selections:
`zwp_primary_selection_device_v1.set_selection(source, serial)` takes the serial of the event
that triggered the request (keyboard or pointer). Therefore `claim_primary_text` requires a focused seat
and a fresh triggering serial from **either** keyboard or pointer input on that seat, with its
origin recorded; do not impose a keyboard-only serial rule and never fall back to an arbitrary
first seat. Focus transfer clears the old focus flag and invalidates stale serials. Seat removal
drops the offer, owned source and pending requests for that seat only.

Internal design:

- Bind `PrimarySelectionManagerState` when the global is present; absent global means no PRIMARY
  support, not a fallback that binds another data device.
- Create one `PrimarySelectionDevice` per seat, alongside the regular `SeatClipboard`; implement
  `PrimarySelectionDeviceHandler` and `PrimarySelectionSourceHandler` for `WinitState`, and add
  `sctk::delegate_primary_selection!(WinitState)`.
- Drop an owned PRIMARY source on `PrimarySelectionSourceHandler::cancelled`, seat removal or
  clipboard drop. Forget the active primary offer when the compositor clears it.
- The PRIMARY load operation returns `Unsupported` when the global is absent and
  `NotFound` for no offer and `InvalidData` for an offer without text MIME. A present
  zero-byte text stream is `Ok(Arc::from(""))`, never absence/fallback. It does not reclaim or re-set an
  unchanged selection. The store operation only sets when the caller asks and does not run on every
  selection change.
- W15P reads use the same bounded transfer handle, cancellation and completion wake as W15.
  Claims are synchronous; serving writes use its bounded pool and source lifecycle. Test
  present empty text separately from absent offer and non-text MIME.

#### 5.3.3 E4 integration and atomicity

The egui-winit 0.37 E4 shape is `unsafe Clipboard::new(Option<RawDisplayHandle>)` because the
display handle must outlive the clipboard. E4 must initialize arboard, leave `smithay: None`, set
`winit_wayland: Some(...)`, and only then return. `get` falls back to arboard only for absent/non-text errors
(`NotFound`/`InvalidData`); `Ok("")` is a present empty value and never falls back; `set_text` logs and returns after the winit store even on error; `get_image` and
`set_image` remain arboard-only. Do not make the winit clipboard path optional in a way that lets
smithay bind a second regular device beside it.

E4 regular may compile and be reviewed before W15P **only** if PRIMARY is explicitly reported as
`Unsupported` and no consumer treats that revision as PRIMARY acceptance. Regular E4 and W15P/E4
PRIMARY are one consumer hand-off: the final consumer pin must contain both, and the CL stage is
blocked until the E4 PRIMARY stage exists.

The E4 adapter records the requesting viewport, focus owner and input generation, wakes that
viewport on transfer completion, and emits paste exactly once only if the target is still current.
It invalidates a pending request on another edit, focus/selection-changing action, owner change or
close; a repaint or release of the initiating shortcut must not cancel an in-flight paste. It does
not call a blocking read. Tests cover delayed regular/PRIMARY paste, self-paste, text/image
fallback, trickle/oversized offers, cancellation, stale target/input generation and actual
event-loop responsiveness.

If W15P cannot meet a proven compositor requirement, the fallback decision is explicitly a
**measured compatibility fallback**: keep regular W15/E4, report PRIMARY as unsupported on that
compositor, and do not silently bind smithay/arboard in a way that breaks regular paste. The
fallback must be recorded with native evidence; it is not the default design. A documented
unsupported limitation requires explicit acceptance, not compile-only sign-off.

### 5.4 W24 reset semantics

`Window::reset_ime()` discards the composition without committing it:

- X11: `XmbResetIC`, clear client preedit, send empty preedit if one was shown; do not destroy the
  window's input context.
- Wayland: disable and enable text input in two commits, re-apply purpose/cursor area/surrounding
  text, and send an empty preedit if one was shown.
- Windows: `ImmNotifyIME(..., NI_COMPOSITIONSTR, CPS_CANCEL, 0)` only while a composition is in
  progress; the IME's end messages clear the preedit.
- macOS: `discardMarkedText`, then empty preedit if the input method did not send one.
- iOS/Android/Web/Orbital: no-op.

Focus change to another field must reset the old composition; re-requesting the already-focused
field must not interrupt an active composition (E6 is the egui-side complement). Tests must assert
both, including an empty preedit event exactly once.

### 5.5 Suite language-work mapping

The winit hand-offs to the suite language work (craft-libs#198 / egui E19 and E21) are:

| Suite item | Winit requirement | Consumer evidence |
|---|---|---|
| I3 X11 Vietnamese IME | W20 one context per window; W21 quiet-time spot | fcitx5/IBus native Vietnamese Telex and forwarded Backspaces |
| I4 IME reset | W24 all four desktop backends; no-ops elsewhere | Native field-change/reset, one empty preedit |
| I5 macOS replacement/dictation | W22 `ImeDocument`; W26 byte-count deletion | Native press-and-hold, dictation, accent/flag/ZWJ |
| I6 Windows IME | W3/W18 caret; W19 IME-owned key; W4 hidden maximized | Native Korean, Sogou/Japanese, Backspace/Enter/arrow |
| I7 Wayland surrounding text | W27 `set_ime_surrounding_text`, lifecycle revision and delete-surrounding commit; E21b caller | Native text-input-v3 correction/prediction plus outgoing-text protocol trace |
| K1 macOS Command layout | W23 `KeyEventExtMacOS::key_with_command` (event-time) | Native Dvorak–QWERTY ⌘/V; egui-winit P7 release-Command test |
| K2 Latin-layout shortcuts | W25 `KeyEventExtLatinLayout` (event-time) | Native Russian/Greek/Arabic/Hebrew/Thai plus Dvorak; egui-winit P7 layout-switch test |
| E19 | W25 + W23 (Windows side uses `MapVirtualKeyExW` in egui-winit) plus egui-winit P7 release pairing | E19 shortcut tests with non-clipboard sequences; no winit 0.31 requirement |
| E21 | W26 + W27 | E21 exact-byte/one-undo tests; retire together with core 0.31 API |
| E21b | W27 caller/serial/lifecycle revision | Outgoing surrounding text after delete+commit, focus/password clearing, two windows |
| E4 regular clipboard | W15 + bounded transfer API | Native regular copy/paste; no second data device; no blocking UI read |
| E4 PRIMARY | W15P + bounded transfer API | Native Hyprland/KWin/Mutter PRIMARY matrix; per-seat focus/serial/owner tests |

This mapping is the boundary: winit provides native events, byte offsets, protocol state and
bounded transfer completions; egui owns shortcut classification, release pairing, undo, password
exclusion, paste target/input generation and selection-reclaim policy.

## 6. Vendor and open-PR disposition

The PdfCraft vendor is the original application-local source of several W patches. It is not the
same thing as the fork.

| Vendor / open work | Current state | Disposition |
|---|---|---|
| `pdfcraft/vendor/winit` | `e9809ef5`, W1-W13, source guards in `apps/pdfcraft/tests/it/`; vendor patch 5 is W5 | Transfer/retain equivalent behavior in the fork; rewrite the W5 guard and tests to W20's one-context replacement, and require W1-W4, W6-W13, W20 replacement, W14/W15/W25/W26/W27 equivalence plus native checks before removing vendor/winit. Do not merge PR199's stale trees |
| `pdfcraft/vendor/egui-wgpu` | `49682f8b`, E2+E3 | Separate egui plan; not this repository |
| PdfCraft #3254 (Wayland drops) | Open; W15 into vendor | Rebase onto W15; remove duplicate vendor patch after integration |
| FilmCraft #1115 (Wayland drops/clipboard) | Open; W15 | Same; do not carry a second implementation |
| GridCraft #2222, PhotoCraft #8019 (Wayland drops) | Open | Same W15 consolidation |
| FilmCraft #2735 (Android vendored winit) | Open, not present in this checkout; not inspected line by line | Preserve. Review before consolidation; decide whether it is app-local Android build/config work or a winit behavior patch. If it is a winit patch, give it its own provenance row and Android test; do not fold it silently into W18/W24. |
| PDFCraft #3880 (Wayland tablet pens) | Open; vendored winit | `craft-tablet`/tablet work; not this cut and not W16 |
| FilmCraft #634, GridCraft #165, PdfCraft #784 (XWayland default) | Open | App launch policy; not a winit patch |
| [CADCraft #668](https://github.com/storytold/cadcraft/pull/668) | Open; head `0cc7e74321e430e0f7d7579b36ac73501c20cd2e`, base `fc4e98315a8d771b5ab832f0ade0b5e7b106df7f` | Proposed `vendor/winit`, `vendor/patches/winit/01..10` and `apps/cadcraft/tests/winit_*.rs` carry W1–W10. Current app main has no vendor tree. Reuse behavioral guards/provenance rather than merge the full vendor tree; W5 is superseded by W20 and its guard must assert reset/unfocus/refocus without context destruction. Require W1–W4/W6–W10 equivalents and native control-path evidence before declaring the PR covered. No PR closure is authorized. |
| [WordCraft #787](https://github.com/storytold/wordcraft/pull/787) | Open; head `98f3dcbe9d080a479e5879045fd226ad7f9765aa`, base `7c5f72a31b0a001de15e537aa804c66aebfe1998` | Proposed `vendor/winit`, `vendor/patches/winit/source-delta.json`, numbered 01–10 notes and `apps/wordcraft/tests/winit_*.rs` carry W1–W10. No vendor on current main. Transfer guards/provenance, rewrite W5 for W20, retain seven first-Korean-key policy tests and unchanged-platform controls. W22’s broader macOS replacement handling needs new native tests because this proposal excluded selectedRange/dictation changes. Compare exact source delta before implementation; do not import the vendor tree or claim existing native acceptance. |
| DesignCraft #1197, PhotoCraft #606 (macOS IME workarounds) | Open | W9/W10 make the workarounds unnecessary; remove only after native macOS evidence |
| DeckCraft #170, GridCraft #389, PhotoCraft #3724 (W11) | Open | Use W11 fork; remove per-app vendor/pin copies |
| PdfCraft #3655 (⌘V on Dvorak–QWERTY ⌘) | Open | W23; verify command table natively |
| PdfCraft #2347 (macOS title-bar drag) | Open | W12; verify native drag |
| PdfCraft #1569 (Wayland zoom latch) | Open | W13; verify native Ctrl+wheel release |
| GridCraft #761, PhotoCraft #4007 (W1/E2) | Open | W1 belongs here; E2 belongs to egui-wgpu |
| PhotoCraft #7828 (Wayland file drops) | Open | W15; consolidate |
| PdfCraft #1150 (X11 DPI) | Open | W11; native X11 check |

Rules:

- Do not remove unrelated PDF vendor fixes or major versions merely to make a `cargo tree`
  duplicate list empty.
- A vendored copy is removed only after the consumer root pins the reviewed immutable fork
  revision and the transferred source/behavior guards pass at that revision.
- Machine-local path patches and branch names must not appear in publication. The winit root has
  none; consumers must replace `vendor/winit = { path = ... }` with the approved git `rev` and
  remove `vendor/winit` in the same reviewed change.
- FilmCraft Android work is a separate gate; an Android compile/no-op `reset_ime` is not evidence
  that Android IME or that open PR is correct.

### 6.1 Consumer hand-offs

Consumer plans must pin `<FINAL_STORYTOLD_WINIT_REV>` (W1-W15 minus W5, W20, W17-W27, W15P,
bounded transfer) by immutable full SHA. `8b35388b` is a research input, not a production pin and
not evidence of W15P/E21b.

**DesignCraft native quit stage (DC-W14).** The final winit revision must contain W14 and the
`EventLoopBuilderExtMacOS::with_close_requested_on_terminate` builder method. DesignCraft currently
enables it at `apps/designcraft/src/main.rs:446-453` and preserves documents at
`main.rs:118-133` (`preserve_documents_on_exit` -> `finish_confirmed_quit`); it must retain both
after repinning. The stage is reviewed with the DesignCraft consumer change, not assumed from the
winit PR:

- Native evidence: AppKit Dock Quit, ⌘Q, and system logout/shutdown each reach the normal
  Save / Don't Save / Cancel flow; Cancel keeps the app and documents alive; Save and Don't Save
  complete or preserve documents as designed; force-quit behavior is recorded separately.
- The existing `finish_confirmed_quit` unit tests around
  `crates/ui-egui/src/lib.rs:2261` and `:2288` (helper at `:1247`, broader quit-flow
  tests from `:1927`) are logic tests only, not native termination evidence.
- A dirty document edited after the last autosave tick is preserved or blocks exit with the
  documented recovery error; the native row records the actual recovery directory and result.
- This stage is independent of E19/E21 but depends on the final winit W14 source; it may land with
  DesignCraft D1’s atomic complete dependency landing, after the shared fork prerequisites.
  It does not authorize an earlier intermediate production pin.

## 7. Test plan

The winit tests are mostly policy/source tests. They are necessary but not sufficient: native
window, focus, compositor, IME and clipboard behavior require the rows in 7.2. This plan does not
claim any of those were run.

### 7.1 Synthetic and policy regressions

| Area | Test | Observable assertion / mutation |
|---|---|---|
| W1 Windows DPI | Extract the suggested-rect policy if practical; otherwise source guard | A hidden/geometry test fails if the old nudge is restored |
| W2 seat text input | `wayland_text_input_lives_until_the_seat_goes` | Remove `text_input.destroy()` from `remove_seat`, or add it to `remove_capability`: test fails |
| W3 Korean caret | source guard in `tests/ime_patches.rs` | Remove flag check: test fails |
| W4 hidden maximized | pure `WindowFlags` policy test if factorable; source guard | Old `SW_SHOW` branch causes test failure |
| W5/W20 X11 context | `x11_input_context_outlives_disallowing_ime` plus native test | Reinstate `remove_context`: guard fails; field focus native behavior regresses |
| W6-W10 macOS commit | each source guard plus `ImeDocument`/`ImeStartup`/`plain_insert` tests | Remove pending commit, discard, emoji commit, duplicate suppression or retry: corresponding test fails |
| W11 X11 DPI | `default_dpi_factor` unit tests | Reorder Xft/server/panel: tests fail |
| W12 title-bar drag | source guard; native drag | Restore old `mouseDownCanMoveWindow`: guard/native check fails |
| W13 scroll phase | `scroll::phase` unit tests | Revert to `Started`: unit test fails |
| W14 termination | `tests/macos_termination` six scenarios plus DesignCraft DC-W14 native Dock Quit/Cancel/Save/Don't Save/logout | Return `NSTerminateNow`, queue synchronously, or fail to coalesce: scenario fails; DesignCraft unit-only coverage is not native evidence |
| W15 regular clipboard | pure state test for seat/serial/owned marker; bounded-transfer tests for total deadline, byte cap, trickle, cancellation and worker admission; protocol trace (`WAYLAND_DEBUG=1` or a test compositor) | Bind a second `wl_data_device`, lose active seat, remove self-marker, reset the deadline per poll, exceed the byte/worker cap or spawn one thread per send: test/trace fails |
| W15P PRIMARY | pure per-seat focus/offer/source/epoch tests; keyboard and pointer triggering serials with origin; bounded async transfer; protocol trace (`WAYLAND_DEBUG=1` or a test compositor); native matrix | Fall back to a second regular data device, a first-seat serial, a stale/absent serial or reclaim unchanged selection: test/trace fails |
| W17 guards | run `tests/ime_patches.rs` and `tests/pointer_patches.rs`; mutation check | Removing any guarded patch makes the guard fail |
| W18 system caret | `SystemCaret` unit tests | Do not destroy on focus loss, create before area, or recreate when only position moves: tests fail |
| W19 VK_PROCESSKEY | source guard | Re-send `Process` key: test fails |
| W21 IME spot | pure deadline/settle test plus source guard | Send immediately while busy, busy-loop, or miss wake deadline: test fails |
| W22 ImeDocument | unit tests for caret, replacement, UTF-16 offsets, bounded history, stale ranges | Use chars instead of UTF-16, or return a range before the caret: tests fail |
| W23 ⌘ table | `tests/macos_keyboard.rs`; egui-winit P7 release-Command-before-letter and Dvorak–QWERTY ⌘/K | Use `charactersIgnoringModifiers` or drop a held key after modifier release: test fails |
| W24 reset | per-backend source guards + pure preedit-shown tests | Commit instead of discard, enable without commit, Windows complete instead of cancel: tests fail |
| W25 Latin layout | xkb keymap tests with pinned deterministic fixtures (no host data); Dvorak/Russian/`ru,us`; egui-winit P7 layout-switch-while-held, repeat, unidentified-key and alias tests | Per-key instead of whole-layout classification, a missing fixture that passes early, or a stuck non-clipboard shortcut after layout switch: test fails |
| W26 macOS bytes | `ImeDocument` byte tests with combining accent, flag, ZWJ, UTF-16 selection | Backspace/grapheme deletion: test fails |
| W27 Wayland bytes/lifecycle | `surrounding_text_excerpt` tests; commit/delete order; focus/owner epoch clearing; late `Done`; serial mismatch vs stale epoch; context disable/enable; incoming delete → commit → outgoing text trace; two-window dispatch; E21b caller | Char-boundary cut, >4000 bytes, deletion attached to wrong commit/window, surviving pending batch after leave, dropped valid current-epoch edit, stale outgoing text or missing E21b caller: test fails |

W17's single test file may be split by patch in the staged PRs: no guard lands before the
behavior it checks. W17 and the later source guards should be supplemented or replaced with
behavior tests where the logic is plain Rust (`SystemCaret`, `ImeDocument`, `ImeStartup`, `plain_insert`, `command_key`,
`scroll::phase`, `default_dpi_factor`, `surrounding_text_excerpt`, `surrounding_bytes_to_chars`).
Textual guards remain only for platform code that cannot be exercised off-platform; every guard
must fail when its patch is removed.

### 7.2 Native matrix

| Platform / hardware | Cases |
|---|---|
| Windows 11, real monitors with different scale factors | W1 drag across monitors; W4 start hidden+maximized; W3 Microsoft Korean caret; W18 Sogou/Japanese candidate/caret placement; W19 Backspace/Enter/arrow in composition; reset_ime; copy/paste |
| Windows 10 | W1 fallback path unchanged; W4 restore; IME composition |
| Linux X11, fcitx5 and IBus | W20 Vietnamese Telex across fields; W21 forwarded Backspaces/word correction; W24 reset; W25 Russian/Greek/Arabic/Hebrew/Thai + Dvorak + no-Latin-layout; W11 96/144/192 DPI; regular clipboard; no PRIMARY claim without native |
| Wayland Hyprland/wlroots | W15 regular clipboard and file drops; W15P PRIMARY (per-seat focus, keyboard and pointer triggering serials with origin/freshness, owner change, empty/absent offer, self-paste, slow/canceled transfer); one data device per seat; W27 Vietnamese/predictive surrounding deletion and outgoing text; W13 Ctrl+wheel release; W24 reset |
| Wayland KWin | Same W15/W15P/W27/W13/W24 set; compositor-specific PRIMARY ownership/serial |
| Wayland GNOME/Mutter | Same set; explicitly record PRIMARY support/failure rather than assuming parity |
| macOS real hardware | Apple Korean first key/space; Japanese composition; Chinese `，`/ideographic space; press-and-hold accents; dictation; emoji picker; W22 teardown; W23 Dvorak–QWERTY ⌘/V; W12 title-bar drag; W24 reset; W14 Dock Quit/logout/force-quit distinction |
| Linux headless or no compositor | compile/check only; not acceptance |
| Android (FilmCraft gate) | Build/check; review #2735; no IME/PRIMARY claim from `reset_ime` no-op |
| Web/iOS/Orbital | `reset_ime` no-op and cfg compile; no native IME/PRIMARY claim |

Headless rendering, source guards and pure policy tests are not proof of native focus, IME,
compositor or picker behavior. macOS and physical Windows rows remain unresolved until real
evidence exists.

### 7.3 Byte-vs-character and event-order tests

Add to the winit policy tests and the egui integration tests:

- `e + U+0301`, `🇯🇵`, `👨‍👩‍👧`, mixed Latin/CJK and a selection spanning a direction change.
- Replacement of the last character after a combining accent and a ZWJ sequence; assert the
  character before the replaced range survives. A grapheme/Backspace implementation must fail.
- Deletion with `before_bytes` and `after_bytes` both nonzero; selection and preedit excluded;
  empty commit when only a deletion is requested.
- A UTF-8 offset that would end inside a character: the implementation leaves the character
  rather than splitting it; offsets from native layers are also rejected/clamped at their source.
- 4000-byte Wayland surrounding text cut at char boundaries; a selection longer than the limit
  keeps the cursor and clamps the anchor; enabling/reset re-sends the excerpt.
- Stale focus/serial/batch: send a commit after focus moved, a deletion from an older batch, and
  two windows' commits; assert no deletion is applied to the wrong window or later commit. Also
  cover a late `Done` after leave/re-entry, reset while a commit/deletion is pending, and seat
  removal while a batch is pending; pending batch state must not survive re-entry.
- Current-epoch serial mismatch: `Done` with a mismatched serial still applies incoming edits once;
  outgoing surrounding text waits for a matching serial and is not dropped with the batch.
- Owner/context transition: normal -> password -> normal and available -> unavailable -> available
  clear cached surrounding text and commit a disable/enable boundary; an empty document is not
  treated as unavailable support and a preedit is cancelled without insertion into the new owner.
- E21/E21b outgoing text: after incoming delete + commit, assert the wire carries the updated
  excerpt with cursor/anchor in UTF-8 bytes; deletion-only batches send an empty commit and updated
  text; two windows and field switches never send stale text.
- E21 one undo step: deletion + commit change the text in one TextEdit action; a second unrelated
  commit must not reuse the first deletion.
- Bounded transfer: trickling and oversized sources, repeated slow requests, owner change and
  cancellation; assert total deadline, byte cap, bounded thread/queue counts and event-loop
  responsiveness; self-paste returns without a compositor round trip.
- PRIMARY serials: keyboard-only focus, pointer-only press/release, focus transfer, seat removal and
  stale serials; a pointer-triggered `set_selection` is accepted on the focused seat, and no
  arbitrary first-seat fallback occurs.
- macOS NSRange inside a surrogate pair and a range that starts before the known document; assert
  the documented ignore/clamp behavior.
- Windows Korean caret with no `GCS_CURSORPOS`; assert caret after the syllable, then reset.

### 7.4 Performance tests

Winit does no rendering, so do not use compile time or egui frame time as winit performance.
Measure the paths this branch changes, paired baseline/candidate on the same host, exact SHA and
feature set. These counts are minimums from `craft-libs/docs/egui-037-suite-rollout.md`; an app may
tighten them, never loosen them.

| Path | Workload | Samples | Metrics | Provisional threshold |
|---|---|---|---|---|
| W25 key translation | 1,000,000 key-down/up events on US, `us,ru`, `ru,us`, Dvorak, `ru,gr`; `KeyContext::process_key_event` | 10 warmups, 30 timed iterations, >=1,000 samples for any p99 | median/p95/p99 latency, allocations, RSS | no per-event allocation regression; investigate median > +5% or p95 > +10%; p99 <= +15% and at least 0.05 ms above noise floor |
| W21 X11 spot policy | 10,000 queued spots with/without input-method replies, synthetic clock | 10 warmups, 30 timed iterations | event-loop wakeups, idle CPU, time to send | no busy wakeups when no spot; one wake at deadline; idle CPU effectively 0 |
| W15/W15P clipboard | 1 KiB, 1 MiB, 10 MiB copy/paste and self-paste; one trickling and one oversized source; canceled request | 10 warmups, 30 timed iterations per size and case | operation latency, total transfer time, event-loop stall, peak RSS, live thread/queue counts | pipe serving off the event loop, ownership claim in the input turn; no >20 ms event-loop stall for 10 MiB; total 2 s deadline and 16 MiB cap honored; thread/queue counts stay within the caps; stale/canceled requests release resources |
| W27 surrounding text / E21b | 10 B, 4 KiB, 1 MiB, 8 MiB text; cursor at start/middle/end; selection > limit; repeated unchanged passes | 10 warmups, 30 timed iterations, >=1,000 samples for p99 | latency, allocations, excerpt bytes | O(n) not O(n²); excerpt <=4000 bytes; p99 <= max(2x baseline, baseline + 0.05 ms) and no document-sized per-frame allocation |
| W27 lifecycle | 1,000 leave/re-enter, reset and interleaved two-window batches | 10 warmups, 30 timed iterations | dispatch latency, allocations, leaked state count | no unbounded queue growth; every stale batch invalidated once |
| W22/W26 ImeDocument | 10,000 inserts/replacements with 256-char bound and emoji | 10 warmups, 30 timed iterations | time, max retained bytes/RSS | constant bounded history; no growth over 10,000 operations |
| W18 Windows caret | 1,000 cursor-area updates, unchanged and changed, focus toggles | 10 warmups, 30 timed iterations | Win32 call counts, latency | no create/destroy churn when area unchanged; move only when size is unchanged |
| W1/W4/W12/W13/W14 | geometry/visibility/scroll/termination paths | 10 warmups, 30 timed iterations where measurable | no frame hot path by themselves | no measurable app regression; native behavior gates above |

Protocol for every measurement: record exact SHA, OS, CPU, GPU/software renderer (where relevant),
compositor and input-method versions, XKB data version, features, DPI, viewport, profile and
toolchain; use at least 10 warmups and 30 timed iterations for steady operations; use at least 20
fresh processes for any startup measurement; use at least 1,000 frames/samples for any p99 or frame
distribution claim. For sub-millisecond measurements require the relative increase to also exceed
0.05 ms or corroborate it with stable instruction/allocation counts. Keep raw samples and paired
confidence intervals where suitable. p95/p99 from 20/30/50 samples is not acceptance evidence.
Fonts, fixtures, renderer/driver/profile and texture-upload counters are not applicable to
winit-only policy measurements; when a measurement goes through an app, record those app-level
fields (including the section 10.6 font fixture identities) as its plan requires. Re-run and
profile a regression; never change a golden or delete samples to make a threshold pass. A changed
visual golden requires an explained intended change and is not a performance fix.

#### 7.4.1 Deterministic XKB fixtures

W25 tests must not read the host's `/usr/share/X11/xkb` data and must not early-return when a
keymap cannot be built. Check in a small deterministic fixture set (precompiled keymap strings or a
minimal data tree loaded with `xkb_keymap_new_from_string`) for at least:

- `us`, `ru`, `gr`, `ara`, `il`, `th`, `vn`, `de`, `fr`, `us(dvorak)`, and the ordered pairs
  `us,ru`, `ru,us`, `ru,gr`.

Provenance for the fixture generation: Debian `xkb-data` 2.42-1 (xkeyboard-config 2.42), licence
text in `/usr/share/doc/xkb-data/copyright` (MIT/X11-style package licence), with these source-file
SHA256 values:

```text
us    ef0b555f6fddb71699673c89f2390122db44751397fc07024c840d582983ee0c
ru    2c45a55022a8255d7c71e0e47040fa1b6586c63add9ae5082fb93afc4716f8ae
gr    6494270909d1af5dbf060116acc2fab94f33d86cda5daf905b648d4464db1d2c
ara   254c076bd38e97a5ad75ea1d8158fa8093be218da66e30c720b8c76d9070c037
il    f42072a57f056b3bfe05c781f24d913d9cf18a604f5a7e53008e3416fe270aff
th    d79780232cab492ca0ae2c55e6fcd15631051852a1f68aaffb7cdcfa51e47a4d
vn    4ef686847cbde54e030af22e50d37e7d91f0524c1178b2943769061220933907
de    4e5710a31d27b40dc056e71d274531a57d3eea672e28be83f163fdfc9e2200bf
fr    01e0d17752c3641a89b4f690e23c2e36311f52e44bbba5a7b3be1b6b7fb6e6c6
```

Fixtures are generated once from the full `xkb-data` 2.42-1 data tree; the layout-specific source
files above are pinned individually, and the manifest records the package version and generator.
The implementation PR records a manifest with the exact checked-in fixture path, SHA256 and the
licence file; acceptance fails if the manifest, a fixture or its hash is missing, if a fixture is
regenerated from a different package version, or if the test cannot build the keymap. A missing
fixture is a hard failure, never a green pass: replace the candidate's
`let Some(...) else { return; }` with an `expect`/panic, or mark the test `#[ignore]` and make the
CI gate fail on any ignored xkb test. The `XKB_CONFIG_ROOT`/fixture path is explicit in the test
harness, so CI does not silently fall back to system data.

## 8. Staged PRs, reachability and rollback

### 8.1 Dependency DAG

```text
S0  docs/PATCHES corrections + W5 disposition (this plan)
 |
 W17 tests (can land alone; guards for patches present at that stage)
 |
 +-- W18 -> W19 (Windows IME)
 +-- W20 -> W21 (X11 IME; W5 explicitly reverted/replaced in this stage)
 +-- W22 -> W26 (macOS caret -> byte deletion)
 +-- W23 (macOS Command table, event-time)
 +-- W25 (Latin layout, event-time)
 +-- W27 lifecycle revision (Wayland surrounding/delete)
 |
 W24 (reset_ime; after W27 so its Wayland state-restore test sees surrounding text)
 |
 W15 bounded async transfer (after published W15; no new protocol object)
 |
 W15P PRIMARY (after W15 bounded transfer; independent of E19/E21)
 |
 egui E19/P7 = needs W23 + W25 (+ existing E7 folded; P7 owns release pairing)
 egui E21 = needs W26 + W27
 egui E21b/P7b = needs E21 + W27 lifecycle revision
 egui E4 regular = needs W15 bounded transfer; PRIMARY explicitly Unsupported until E4 PRIMARY
 egui E4 PRIMARY = needs W15P + W15 bounded transfer
 |
 DesignCraft DC-W14 native quit = needs final winit W14 source
 craft-libs/consumer pins = need <FINAL_STORYTOLD_WINIT_REV> + <FINAL_STORYTOLD_EGUI_REV>
```

Intermediate revisions are allowed only in isolated fork development/control branches.
Production consumers wait for the completed reviewed/published winit series and egui P8,
including P2b, P7, P7b and P7c; no consumer pins `8b35388b` for W15P/E21b.

Reviewable PRs, one coherent outcome each:

| Stage | Repository | Contents | Depends on | Validation | Rollback |
|---|---|---|---|---|---|
| S0 | storytold/winit | `docs/egui-037-migration-plan.md`; correct `PATCHES.md` (W5 superseded by W20 with the exact base/revert, W16 explicitly out) | none | docs review; no code claims | revert docs commit |
| S1 | storytold/winit | W17 guards for W2/W3/W6-W10/W12/W13 at this stage, W18, W19 | S0 | task.py Linux compile; task.py Windows compile; native Windows IME | revert S1 |
| S2 | storytold/winit | W20, W21 and the W5/W20/W21 guards. Land an explicit reviewed `git revert 283370ce` (or a W5-free rebase from `ffae5198`) with W20; no W5 row/commit remains | S1 or S0 | task.py Linux X11 compile; rewritten vendor W5 guards pass; native fcitx5/IBus Vietnamese | revert S2; do not re-add W5 separately |
| S3 | storytold/winit | W22, W23, W26 | S0 | task.py macOS compile; `tests/macos_keyboard`; native macOS accents/dictation/Korean/Command | revert S3 |
| S4 | storytold/winit | W25 with pinned deterministic xkb fixtures | S0 | task.py Linux xkb fixture tests; native layouts; E19 later | revert S4 |
| S5 | storytold/winit | W27 lifecycle revision (pending-batch clearing, owner/focus epoch, serial rules, context transition) | S0 | task.py Linux Wayland compile; `surrounding_text_excerpt`/lifecycle/order tests; native text-input-v3 | revert S5 |
| S6 | storytold/winit | W24 | S5 for the Wayland test expectations | task.py Linux/Windows/macOS compile; native reset per platform | revert S6 |
| S7 | storytold/winit | W15 bounded async transfer API, total deadline/byte/concurrency caps, cancellation and completion wake | published W15 base | task.py Linux Wayland tests; bounded-transfer/event-loop responsiveness; native regular clipboard | revert S7; keep published synchronous W15/E4 |
| S8 | storytold/winit | W15P PRIMARY on the existing connection/per-seat lifecycle, reusing S7 transfers | S7 | per-seat focus/serial/owner tests; protocol trace; native Hyprland/KWin/Mutter matrix | revert S8; keep regular W15/E4 |
| E-A | storytold/egui | egui P1 base on final winit, then E19/P7 after W23/W25; no intermediate consumer handoff | completed reviewed/published S1–S8 | task.py egui/winit compile matrix; app shortcut/release-pairing tests | revert E-A; old 0.36 lock |
| E-B | storytold/egui | E21 byte deletion | E-A, S3, S5 | `ime_delete_surrounding` tests; native app IME | revert E-B with W26/W27 if API is wrong |
| E-Bb | storytold/egui | E21b outgoing surrounding text, owner/context payload, caller to W27, custom-editor adapters | E-B, S5 | outgoing protocol trace; TextEdit/custom-editor tests; password/focus clearing; two windows | revert E-Bb with its adapters; keep incoming E21 |
| E-C | storytold/egui | E4/P2b regular + PRIMARY routing and bounded-transfer consumption | E-A, S8, S7 | regular + PRIMARY native matrix; stale target/input generation; event-loop responsiveness | restore coherent prior dependency set; PRIMARY failure blocks consumer handoff |
| DC-W14 | designcraft | Repin final winit; retain `with_close_requested_on_terminate(true)` and `preserve_documents_on_exit`; native quit regression | final winit W14 source | native Dock Quit/⌘Q/logout Save/Don't Save/Cancel; unit tests are not native evidence | revert DesignCraft pin and source change |
| CL | craft-libs/consumers | immutable pins, remove `vendor/winit`, one family/winit source, PR199 API-port hunks only on `4919088` | completed egui P8 (P2b/P7/P7b/P7c), final winit; DC-W14 linked acceptance | consumer gates; native app matrix; `cargo tree` exactly one source | restore old coherent 0.36.2 + 0.30.13 lock and vendors |

Publication rules:

- Each branch/tag push and each new PR needs the user's explicit approval for that exact
  publication and its validation. This plan grants none.
- Publish to `storytold/winit` on the `craft/0.30.13` line, not upstream-tracking main. The aiongg
  branch is an input; `8b35388b` or a mutable aiongg branch name must not be a production pin.
- No consumer may pin `8b35388b` for E4 PRIMARY or E21b acceptance. The final pin must be a
  reachable `storytold/winit` SHA containing W15P and the bounded transfer API, and a reachable
  `storytold/egui` SHA containing E21b and E4 PRIMARY. Placeholders stay placeholders until those
  commits exist and are reviewed.
- Keep original human authorship and co-authorship (`Jeremiah`, `DorotaC`, and others) in the
  published commits. Do not add agent/model/tool attribution tags or signatures.
- Consumers pin the exact reviewed `storytold/winit` commit with `rev`, and repeat the winit
  `[patch.crates-io]` in each consumer root; patch sections are not transitive.
- Keep `craft/0.30.13` and the old egui/winit dependency set intact for rollback. Use revert
  commits, not force-push, and do not delete the old branches.
- W26/W27 and E21/E21b retire as a group. W15 and E4 regular retire as a group. W15P and E4 PRIMARY
  retire as a group. A release that contains part of a group does not justify removing the other
  part.
- Every build/test in these stages uses `maintenance/task.py run/status/finish` with its own task,
  source and evidence. No direct `cargo` test/build is a substitute for that workflow.

## 9. Acceptance criteria and unresolved questions

Blocking criteria:

1. No production consumer pins a branch, a path patch or `8b35388b`. Exactly one winit source and
   one egui family source resolve in each consumer. Final manifests use reachable
   `<FINAL_STORYTOLD_WINIT_REV>` / `<FINAL_STORYTOLD_EGUI_REV>` values; placeholders are not
   accepted as a published SHA.
2. All W1-W15 behavior still present in the fork, with W5 explicitly removed by the reviewed
   revert/rebase and represented by W20's replacement; W15P and W17-W27 committed with authorship
   preserved. The final tree must not contain both W5 and W20.
3. W25 + W23 make E19 compile and pass non-Latin/Dvorak shortcut tests. egui-winit P7 pairs the
   emitted key by physical key for repeats/release after a layout or modifier change; winit's
   `logical_key` semantics are unchanged. Tests use non-clipboard sequences (Ctrl+Z/Cmd+K and
   ordinary held keys), plus repeat, unidentified-key and two-physical-alias cases.
4. W26 + W27 + E21 + E21b make the full IME path compile and pass exact-byte deletion, one-undo,
   selection/preedit, stale batch/focus, UTF-16-to-UTF-8 and outgoing surrounding-text tests. The
   protocol trace proves incoming delete → commit → updated outgoing text on the wire; W27 without
   the E21b caller is not accepted as complete IME support.
5. W27 clears pending preedit/commit/delete on leave/disable/reset/seat removal; owner/focus epochs
   invalidate stale batches while a current-epoch serial mismatch still applies incoming edits
   once; a context transition clears surrounding text and commits the disable/enable boundary.
6. W15/W15P provide bounded asynchronous reads and writes with total deadline, byte cap,
   cancellation and bounded concurrency; no blocking read on the egui event path, no one-thread-
   per-send growth. E4 records viewport/focus/input generation, wakes on completion and invalidates
   stale paste targets.
7. W15P + E4 pass regular clipboard on Hyprland/KWin/Mutter and PRIMARY on each compositor, or
   PRIMARY has a documented measured fallback that does not break regular paste. Native
   one-data-device evidence is required. PRIMARY state is per seat; keyboard or pointer triggering
   serials are accepted with origin/freshness, and no first-seat fallback is allowed.
8. W24 native reset tests pass on X11, Wayland, Windows and macOS; re-focusing the same field
   does not interrupt composition.
9. W1/W4/W11/W12/W13/W14 native cases pass with real hardware evidence; headless/source guards
   alone do not close them. DesignCraft DC-W14 passes native Dock Quit/⌘Q/logout Save/Don't
   Save/Cancel; its unit tests alone are not native termination evidence.
10. W15/W15P use no second regular `wl_data_device`; the E4/egui policy never publishes a password
    selection through W15P and never re-claims an unchanged selection. Winit does not infer
    password policy from text.
11. Performance protocol followed with the section 7.4 counts (10 warmups, 30 timed iterations,
    >=1,000 samples for p99, 0.05 ms sub-millisecond floor), paired baseline/candidate, raw samples;
    no auto-accepted golden.
12. W25 tests use the pinned deterministic XKB fixtures and fail on missing data; no system-data
    read and no early `return` that reports green.
13. Open vendor/filmcraft PRs reviewed before deleting any vendor copy; FilmCraft Android #2735
    explicitly dispositioned; PdfCraft W5 guards rewritten for W20.

Unresolved at planning time:

- Exact W15P public method names and whether a common `Clipboard` handle or a separate PRIMARY
  handle is clearer. The bounded transfer names/constants in 5.3.1 are provisional; semantics and
  caps are fixed and must be propagated to the egui plan before exact-hash approval.
- Whether Mutter/KWin expose PRIMARY with the same serial/owner behavior as wlroots; native
  evidence is absent. The protocol permits pointer-triggered selection; the final rule must accept
  keyboard or pointer serials from the focused seat, not a keyboard-only rule.
- W27's positional `ime_deletions` queue is not proven corrupt merely because it is global. The
  lifecycle revision must test the actual dispatch/filtering path for two interleaved windows and
  preserve commit/deletion pairing; replace it with tagged `{window_id, epoch}` records if the
  test shows a real mismatch.
- Whether the 100 ms `IME_SETTLE_TIME` is sufficient for slow IMEs and whether waiting for a
  queued event + quiet time is the right policy. Measure with fcitx5/IBus; keep the timer explicit.
- W25's Latin-layout classification and `to_ascii_lowercase` behavior for non-ASCII Latin letters;
  test with the pinned fixtures and decide whether a precomputed per-layout map is warranted.
- W22's 256-character known-text window and the exact replacement-range contract for dictation and
  selections that extend before the window.
- W18's one-caret-per-thread behavior with multiple top-level windows and focus transitions.
- W1/W4 Windows 10 vs Windows 11 behavior and the exact monitor identity during a drag.
- W14 termination during system logout/shutdown and interaction with native panels; DesignCraft's
  recovery path and force-quit distinction need native evidence.
- FilmCraft Android #2735: open and not inspected here; it may be an app-local vendor fix rather
  than a winit language patch.
- Upstream status is a dated snapshot. A later release may already contain W1/W2/#4651/#4729 or
  core IME deletion. Recheck immutable upstream refs before each retirement.

## 10. Evidence used and limitations

Read-only evidence used:

- `PLANNING-CONTRACT.md` in this checkout's parent and in the evidence directory.
- `AGENTS.md` / planning instructions at the server level and the repository-level `PATCHES.md`.
- Actual source at `origin/craft/0.30.13`, candidate tree `8b35388b`, and the historical full
  clone `/home/codex/work/CraftApps/craft-fork-20261020/winit-ai-lang`.
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/{PLANNING-CONTRACT.md,
  parent-reconciliation-log.md, text-input-research.md, upstream-discussions.json,
  ime-surrounding-gap.md, lifecycle-source-audit.md, additional-font-fixtures.json,
  local-guidance/engineering-decisions.md}`.
- `plan-reviews/winit-v1.md` (nine findings) and `winit-review-triage.md` (reconciliation; rejects
  the keyboard-only PRIMARY serial rule, the FIFO-only cross-window claim and the clipboard-shortcut
  stuck-key example).
- `craft-libs/docs/egui-037-suite-rollout.md` performance/sample protocol and shared IME/lifecycle
  contracts; `egui/docs/egui-037-migration-plan.md` E19/P7, E21b/P7b, E4 and section 10.6 fixtures.
- `/home/codex/work/ssd2/task-evidence/craft-fork-ai-lines-20261022/SURVEY.md`,
  `/home/codex/work/ssd2/task-evidence/craft-fork-plan-20261020/PLAN.md`,
  `/home/codex/work/ssd2/task-evidence/craft-fork-egui037-20261021/RESULTS.md` (historical, not
  fresh validation).
- PdfCraft `vendor/README.md` and `vendor/winit/.cargo_vcs_info.json` (W1-W13, `e9809ef5`).
- sctk 0.19.2 `primary_selection` source and the wayland-protocols text-input-v3 XML.

Limitations:

- No builds, tests, VMs, compositor sessions, native Windows/macOS runs or performance measurements
  were performed for this document. The historical survey's "85 passed" and PdfCraft source
  guards are not acceptance for this plan. No plan statement here is a reproduced lifecycle,
  protocol, clipboard or performance result; the candidate findings are static source inspection.
- No final winit or egui SHA is asserted. W15P, E21b, the W27 lifecycle revision and the bounded
  transfer API do not exist yet in an approved storytold fork; they are prerequisites.
- PR 199 was not re-reviewed line by line here; the plan relies on the current shared-library
  review that its trees are stale relative to `4919088` and only selected API-port hunks may be
  reused.
- The candidate checkout is shallow; individual W17-W27 commit history was read from the
  historical read-only clone, not reconstructed here.
- Upstream PR/release state is a dated snapshot; no GitHub API/network query was made.
- FilmCraft #2735 and several app PRs are open and were not line-by-line inspected in this
  checkout.
- This host has an i915 render node visible in other evidence, but that is availability only; no
  native Vulkan/OpenGL/compositor qualification is claimed. Windows remains software-rendered
  until a managed VM run proves otherwise.
- Native macOS and physical Windows IME/DPI/PRIMARY evidence remains required.


## Local implementation update — 2026-10-23

The reviewed plans remain local and implementation is authorized before publication. Earlier plan-publication prerequisites are superseded; final pushes and PR publication still require approval. Research inputs advanced to egui `0f72e2ea3a059c6c5c45270099672b5472f75e7d` and winit `5b708e919e3506b8f8927651159204b1df2f648b`. Production storytold fork branches remain unchanged.

The winit delta fixes destructive macOS replacement on dropped insertText and adds useful unchanged-surrounding deduplication. It does not fix pending incoming batches crossing lifecycle boundaries or provide the outgoing egui context bridge. Windows cancel/reset still needs behavioral coverage for reentrant result strings and duplicate preedit clears. The first local repair targets incoming Wayland batch isolation; serial/outgoing-context support remains separately required.

E22 monitor metadata is a new candidate feature, not part of the reviewed E1–E21 compatibility set. Review found idle hotplug freshness, mixed-DPI coordinate-space and clone-allocation concerns. Retain existing host-owned placement while evaluating any E22 adoption separately; do not silently pull it into the migration by copying the branch tip. Physical positions can be reconstructed from its scale and logical positions (subject to f32 precision); the review claim that this information is completely discarded is not accepted.

The static review's claim that input-method-v2 has no serial is rejected: its commit request explicitly takes a serial. This is a different protocol from text-input-v3. Preserve unchanged-state deduplication and the existing rule that a mismatched text-input-v3 done still applies current-focus incoming edits. Available-empty is distinct from an absent surrounding-text request; native empty→typed transition remains required, without adopting the review's unsupported unconditional reset recommendation.

Craft-libs PR199 is now `1dcde7ed1fe136df6d2b7a52d52b55e538193e1f`, which incorporates the old main security fixes. Its current fourteen-file API delta must be reviewed onto new main `f180fc0d35d521c5d18e118c881d3efde1ec1eee`; old warnings about b68de8c5 do not describe this new tree. Source review evidence is `aiongg-diffs-refresh-20261023-1535.json`, `overnight-fork-review-20261023.md` and `craft-libs-pr199-delta-20261023.json` in the task evidence directory.

### W27 review resolution: window-level composition and explicit cause (2026-10-23)

The v3 independent review identifies real missing lifecycle cases. Preserve winit's public
clear-before-every-Commit contract. Its public IME events identify a window, not a seat, so a
window has one visible composition: an accepted commit from any seat ends that composition,
emits its empty Preedit immediately before deletion/Commit, and releases the old visible owner.
A new nonempty preedit then becomes the owner. A nonowner empty-preedit-only batch must neither
clear another seat's visible text nor claim ownership. Preserve all accepted commits and event
ordering; teardown of a nonowner still cannot erase another seat's newer preedit. This is an
explicit compatibility policy, not a claim of simultaneous per-seat editing support. Adding
seat identities to every public consumer would be a separate larger API migration.

Research: the official text-input-v3 XML defines per-seat focus and clear/delete/commit/new-preedit
ordering; it does not define how a window-only toolkit API merges seats:
https://cgit.freedesktop.org/wayland/wayland-protocols/tree/unstable/text-input/text-input-unstable-v3.xml
SDL's current Wayland source was inspected (events blob 523e8c19d806bbed2ce4867152e4fdc85acbb3cf):
https://github.com/libsdl-org/SDL/blob/main/src/video/wayland/SDL_waylandevents.c
Its text-input handlers have per-seat `has_preedit` but send window-global editing/text events,
leave enter/leave no-op and do not implement delete-surrounding; this is not a sufficient oracle
for our richer surrounding-text contract. No SDL or GTK source is copied.

Every state commit that sends surrounding text must send an explicit change cause. Initial
enable/re-enable/reset/purpose and geometry-only updates use Other. An explicit IME application
receipt may use InputMethod; pending geometry-only updates must preserve an already-coalesced
text mutation's cause. Cause-only setter changes remain deduplicated. Add recorder coverage of
the actual enable/ack/reset/geometry paths, not merely pure enum mapping.

Detach a text-input proxy under window locking before sampling/end_owner on removal. Resolve
leave using stored ownership; detach stale entries on enter to a different surface, including
unmapped leave surfaces. Deliver Enabled as a window lifecycle notice; recheck closed-window
tombstones immediately before callbacks. Wake the event loop for all emitted state requests,
including background-thread clear/reset/purpose paths without visible preedit.

The coalesced-old-record/done(new-serial) ambiguity remains a native protocol qualification gate:
the wire carries no record generation. Do not silently discard genuine newly committed text to
claim a guarantee the protocol cannot prove. Test the observed supported compositor sequences,
record that limitation, and revisit the barrier design if native traces demonstrate it.
The new required WindowExtWayland methods are a documented fork API break; no fake no-op default.


## Integrated fork status — 2026-10-24

The PRIMARY and PNG boundary sections above record earlier implementation stages. The final
local integration includes asynchronous regular/PRIMARY reads, bounded PNG decode/encode,
reserved image ownership, failure cancellation and independent PRIMARY handling. The winit
and egui implementation sources were checked byte-for-byte against the staging snapshot
qualified by `clipboard-image-smoke-20261024-025844-315945` before this status note was added.
That private-Sway run passed 25 checks through a real egui-winit State, including external/self
text and PNG, exact RGBA/alpha, malformed/absent cases, mixed-output last-wins behavior, worker
wakes and an unchanged external PRIMARY sentinel. It does not qualify multi-seat, native
slow-peer/focus-transfer races, other compositors/platforms or performance. Final immutable
pins and canonical locks are a separate landing step.
