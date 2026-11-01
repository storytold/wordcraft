# Focus (distraction-free / "Zen") mode: integration test report

> **Last reviewed:** 2026-10-31 · **Last updated:** 2026-10-31 · **Change:** issues 1–5 fixed and their tests flipped to the fixed behaviour (25 tests); a review follow-up widened fix 2 to four more commands that read `value` without the shared pattern, and fixed the tab badges beside Full Screen View's Search badge (25 tests). Issue 6 stands as notes. First version the same day: integration testing of View › Immersive › Focus (`view.focus`, #1678) across the engine, the egui front end, the control channel, the CLI and MCP; six issues found (one severe), all pinned by new tests.

Focus is WordCraft's distraction-free mode: the document alone on a black canvas, with a slim bar
whose `…` drops the ribbon over the page. It shares the "screen state" code with Full Screen View
(`view.fullScreenView`) and Full Screen (`view.fullScreen`), and it interacts with panes, the ruler,
the status bar, Read Mode, keytips, dialogs, the File page and the window's full-screen state.

This report records what was tested, what works, and what does not. Every issue below has a
regression test; the five that were fixed in this change have had their assertions flipped to the
fixed behaviour, and the "Fixed" note under each says what changed.

## Test inventory

New test files (the fixes themselves are in `crates/engine/src`, `crates/ui-egui/src`):

| File | Tests | What it covers |
|---|---|---|
| `crates/engine/src/tests/focus_mode.rs` | 8 | The command's contract: `view.state` reporting, toggle vs. `value`, a non-boolean `value` refused (every view switch), no undo/dirty/document change, the three full-screen states, Read Mode, per-document view state, not saved in the file |
| `crates/ui-egui/src/tests/focus_mode.rs` | 17 | The whole frame loop: chrome/panes/ruler/status bar hidden and restored, both entry buttons, the ribbon button by keytip, Escape ordering (dialog, selection, File page), keytips on the slim bar and the ribbon they drop, OS full-screen exit, closing the last document, Read Mode, typing/formatting/undo, F6, the keyboard handover, the Search palette, collapsed ribbon, accessible names, preferences, Full Screen View's dropped ribbon |

Run them with:

```
cargo test -p wordcraft-engine --lib -- tests::focus_mode
cargo test -p wordcraft-ui-egui --lib -- tests::focus_mode
```

End-to-end checks were also run by hand and are reproduced in "Evidence" below:

- `cargo run -p wordcraft-ui-egui --example ui_shot -- script.jsonl` — headless window screenshots
  in Focus, with the ribbon dropped, on the File page, in Full Screen View, in Read Mode, in Draft,
  in Outline, Side to Side, Page Width fit, dark pages, the Find dialog, the Search palette, and an
  empty window.
- `wordcraft-cli commands` and `wordcraft-cli run --template sample --cmd 'view.focus' --save …`.
- `wordcraft-cli mcp` over stdio: `execute {"command": "view.focus"}` → `{"value": true}`,
  `execute {"command": "view.state"}` → `"focusMode": true`.

## What works

- **The command contract.** `view.focus` is a pure view toggle: no undo checkpoint, no dirtying,
  no document change; `view.state` reports `focusMode`; without `value` it toggles, with a bool it
  sets, and `true`/`false` are idempotent. It works in Print, Web, Draft and Outline layouts, and
  `view.zoom`/fit modes (One Page, Multiple Pages, Page Width) refit as the canvas widens.
- **Entering and leaving.** The status bar's Focus button and the ribbon's View › Immersive › Focus
  both work; the ribbon's is reachable by keytip (`F10`, `W`, then its letter). Escape leaves Focus
  in the documented order: a dialog first, then a text selection, then the File page, then Focus.
- **What Focus hides.** Title bar, ribbon, status bar, every task pane (Navigation, Clipboard,
  Styles…), the ruler and the panes' width all go; the page moves up under the slim bar. Everything
  comes back on exit, with the panes' and ruler's own settings untouched (they are hidden, not
  turned off). F6 does not move the keyboard to a pane that is not drawn.
- **Overlays still work.** Dialogs (Find and Replace, Search Commands) open and close over the
  black canvas; the File page opens and returns to Focus; the Search palette is reachable by keytip.
- **The dropped ribbon.** `…` drops the ribbon over the page without moving the page; a click on
  the page puts it away and keeps Focus; typing after a ribbon click reaches the page; a collapsed
  ribbon (Ctrl+F1) drops as the tab strip only, and its tabs still switch.
- **Window state.** Focus asks the window for full screen once per change (switching to Full Screen
  View does not ask again); the OS leaving full screen ends Focus; Full Screen (F11) can be on with
  Focus and is left alone by it.
- **Not persisted.** Focus is not written to the document (a reopened `.docx` starts in the usual
  window) and not written to the preferences (a new run starts in the usual window).
- **Per document.** The view is per-document: a document opened beside a Focused one inherits
  Focus, and switching away and back keeps each document's own state.
- **Automation.** Control (`engine.execute`), MCP (`execute`) and the CLI all run `view.focus` and
  read `focusMode` back from `view.state`.

## Issues found (all but the last fixed in this change)

### 1. Severe: closing the last document in Focus leaves a full-screen window with no way out — **fixed**

`paint_window` draws the empty window's own chrome (title bar, ribbon, grey canvas, empty status
bar) whenever no document is open, ignoring the screen state — but the view flags are untouched, so
the window stays full screen. Worse, `view.focus`, `view.fullScreenView` and `view.fullScreen` are
all missing from `works_without_document` (`crates/ui-egui/src/app/commands.rs`), so:

- the ribbon's View › Focus is greyed out (its `enabled` check refuses it),
- the status bar is empty, so its Focus button is gone,
- Escape does nothing: `leave_full_screen` runs `view.focus` and ignores the refusal, so it reports
  the key as handled and Focus stays on.

The user is left in a full-screen window that looks like an ordinary empty window, with no Focus
bar and no keyboard way out. Only opening a document (which inherits Focus and brings the bar back)
or the operating system's own full-screen control gets them out.

Reproduce: open a document, View › Focus, File › Close. On the desktop the window is full screen.

Tests: `tests::focus_mode::closing_the_last_document_in_focus_keeps_the_way_out` (ui-egui).

**Fixed:** the empty window honours the screen state now — it draws the slim bar (and the dropped
ribbon) instead of the usual chrome — and `view.focus`, `view.fullScreenView`, `view.fullScreen` and
`ui.focusRibbon` are in `works_without_document`, so Escape, the bar's `…` and the commands
themselves all work with no document open. `leave_full_screen` also no longer reports Escape as
handled when the command it runs was refused.

### 2. Moderate: `view.focus` with a non-boolean `value` silently toggles — **fixed**

`toggle` reads `value` with `p::bool`, which returns `None` for anything that is not a JSON
boolean, and falls back to toggling. So `{"value": "true"}` — what a loosely typed control, MCP or
script client may send — turns Focus **off** when it was on, and `{"value": null}` toggles too.
Nothing is refused and nothing is reported. Compare `ui.focusRibbon`, which refuses junk with a
parameter error.

This affects every `toggle`-based view command (`view.marks`, `view.ruler`, `view.darkMode`…), but
Focus is the one where a wrong guess is most visible.

Test: `tests::focus_mode::a_non_boolean_value_is_refused_and_leaves_the_view_alone` (engine),
which checks the whole view switch family, plus `line_and_page_break_toggles_take_an_explicit_value`
and `style_inspector_pane_and_hostile_params`, which used to assert the silent toggle.

**Fixed:** `p::toggle`/`p::switch` (`crates/engine/src/lib.rs`) are the one place a switch reads
`value`: a boolean sets, an absent value toggles (or takes the default), and anything else — a
string, a number, `null`, an array — is a parameter error that changes nothing. Every command that
read `value` with `p::bool(...).unwrap_or(...)` now goes through them, so paragraph toggles, review
switches, panes, options and equations share the contract. MCP reports it as
`bad parameters: `value` must be true or false`.

**Review follow-up:** six command families read `value` without that pattern, so a junk value fell
through to the "no value" branch — the worst being `draw.inkToShape`, which would convert the
lasso-selected strokes to shapes. All six now validate first and change nothing:
`draw.inkToShape`, `layout.hyphenation`, the character-toggle helper behind `format.bold` and
friends, `format.underline`, `table.repeatHeader` and `table.rtl`. The widened test checks thirty
switches — with ink lasso-selected, so the refusal is proven to come before the conversion — and
compares `view.state`, `document.inspect` and the undo labels before and after every refusal.

### 3. Moderate: Focus started without a click strands the keyboard in the vanished field — **fixed**

If a chrome text field has the keyboard (the Font Size box, say) and Focus is turned on *without a
click* — a keybinding, `engine.execute` over the control channel, or MCP — egui keeps the keyboard
on a widget that is no longer drawn. Typing goes nowhere, and it stays lost after leaving Focus
until something takes the keyboard back. The click path is fine: clicking the status bar's Focus
button hands the keyboard to the page (#1093).

The same happens when the ribbon is collapsed with that field focused, so this is the general
"focused widget disappears" problem, not Focus's own — but Focus is the case that hides the whole
chrome at once, and it is exactly the path automation takes.

Test: `tests::focus_mode::focus_started_without_a_click_hands_the_keyboard_to_the_page` (ui-egui),
which also covers collapsing the ribbon with the field focused, with
`focus_clicked_from_the_status_bar_gives_the_page_the_keyboard` as the control.

**Fixed:** a command that changes the screen state or the collapsed ribbon
(`WordApp::after_chrome_change`) hands the keyboard to the page when a chrome text field had it, and
`hand_keyboard_to_the_page` now only claims it for a page the pass actually drew — which also stops
a stale "the page has focus" flag from swallowing every key in the empty window.

### 4. Minor: keytips in Focus draw no badges — **fixed**

`F10` (or Alt on Windows) starts keytips in Focus, and the keys still work — `W` picks the View
tab, `Q` opens Search — but nothing is drawn: badges hang off the ribbon's own rects, and the
ribbon is not laid out in Focus. The user sees no feedback at all while every key is being consumed
by an invisible mode (Escape cancels it). Word keeps its keytips visible in Focus.

Tests: `tests::focus_mode::keytips_in_focus_show_their_badges_on_the_slim_bar` and
`full_screen_view_drops_the_ribbon_for_keytips` (ui-egui).

**Fixed:** with no ribbon laid out the tab badges are laid along the slim bar
(`keytips::slim_bar_badges`), and picking a tab drops the ribbon down — in Focus and in Full Screen
View, which has no `…` of its own but puts it away on a click on the page — so the command badges
have somewhere to land. Badges are painted on the dropped ribbon's own layer, which is above the
window's.

### 5. Minor: the ribbon's Focus button has no accessible name — **fixed**

Ribbon large buttons are painted, not egui widgets, and never call `widget_info`, so the ribbon's
Focus button — like every large ribbon button — is invisible to a screen reader; only the status
bar's Focus is named. A screen-reader user can still reach Focus from the status bar, but not from
the ribbon's View › Immersive group, and not in Focus at all (where the status bar is hidden).

Test: `tests::focus_mode::both_focus_buttons_have_accessible_names` (ui-egui).

**Fixed:** `big_button` (and so `big`/`big_toggle`) publishes `WidgetInfo::labeled`, so every large
ribbon button has a name. Two older tests that clicked a ribbon button by a label the status bar
shares (`Web Layout`, `Focus`) now pick the right one by position.

### 6. Note (unchanged): Focus and Read Mode can be on at once, and the web has no full screen

`view.readMode` and `view.immersive` leave Focus on, so the window can be in Focus *and* Read Mode;
Escape then leaves Focus first and Read Mode second (one key press never leaves two views). That
ordering is right, but Read Mode's reading toolbar ("Edit Document") and page-turn pill are light
chrome drawn on the black canvas, and neither the status bar nor the ribbon is available to change
reading options — only a keybinding or automation can get there.

`view.focus` has no wasm guard (unlike `view.fullScreen`, which `full_screen_available` disables on
the web), and `sync_full_screen` asks for full screen on every platform; on the web the request is a
no-op, so Focus there hides all the chrome without the browser being full screen. Not verified on a
real browser: the wasm gate could not be run here (see "Not verified").

While testing Focus + Read Mode I also saw reading-view tables break words across lines
("Equip ment", "Loca tion"). That is not Focus-specific — the normal window's Read Mode shows the
same at this window width — so it belongs to the Read Mode layout, not to this report's scope.

## Not verified

- **wasm:** `cargo xtask wasm` cannot run on this machine — `rustup` is not installed (Homebrew
  Rust) and the `wasm32-unknown-unknown` target is missing, so every crate fails with
  `can't find crate for core`. CI covers it.
- **Native multi-window:** Focus is per document; with native windows (craft-window) each window
  shows one document and the harness cannot create them. The per-document behaviour is covered at
  the engine level.
- **A real desktop full screen:** the headless harness never reports the window as full screen, so
  `sync_full_screen`'s follow-the-OS path is tested by injecting the viewport state, not by a real
  window manager. The File page's back chevron positions itself by `view.full_screen` rather than by
  the screen state, so in Focus (before the OS confirms full screen) it sits where the traffic
  lights would be; once the OS reports full screen it moves. Not a defect, but the two flags
  disagree for a few frames.

## Incidental finding (not Focus)

`cargo test -p wordcraft-engine --lib` failed on a clean checkout:
`tests::ime_body_epoch_marks_body_replacements_not_edits` asserted `s.document_id() == 1`, but
document ids come from the process-global `NEXT_DOCUMENT_ID` counter
(`crates/engine/src/session/documents.rs`), so the assertion only held when that test happened to
create the first session in the process. It failed in every full run and passed when run alone,
which meant `cargo xtask ci` was red.

**Fixed:** the test remembers the id and checks it does not change.

## Evidence

Headless screenshots (all `ui_shot`, 1440×900 pt at 2×, sample document):

| Shot | What it shows |
|---|---|
| Focus | Black canvas, page centred, slim bar with `…` at top centre, no title bar, ribbon, ruler or status bar |
| Focus + `…` | Ribbon dropped over the page from `y = 30 pt`, page unmoved |
| Focus + File | File page over the whole window; Escape returns to Focus |
| Full Screen View | Grey canvas, slim bar with the Search box (no `…`) |
| Focus + Read Mode | Reading columns on the black canvas, light "Edit Document" pill bottom right |
| Focus + Draft / Outline | Draft column and outline indents on black |
| Focus + Side to Side | Two pages, black gutter between them (measured: `(0,0,0)` at the gutter) |
| Focus + Page Width | Page refits to the wider canvas |
| Focus + dark pages | Dark page on black, page edge still discernible |
| Focus + Find / Search palette | Dialogs draw and work over the black canvas |
| Empty window after File › Close in Focus | The slim bar with `…`, and the ribbon it drops with live File/Help tabs — issue 1's fix |
| Keytips in Focus / Full Screen View | Tab badges along the slim bar, then the ribbon dropped with its command badges — issue 4's fix |

Automation transcripts:

```
$ wordcraft-cli commands | grep -i focus
view.focus                       Focus                                                         View › Immersive
view.fullScreenView              Full Screen View                                              View › Immersive

$ wordcraft-cli run --template sample --cmd 'view.focus' --cmd 'view.state' --cmd 'view.focus={"value":false}' --save /tmp/cli.docx
wrote /tmp/cli.docx

MCP stdio: initialize → tools/call execute {"command":"view.focus"} → {"value": true}
           tools/call execute {"command":"view.state"} → … "focusMode": true …
```

Gates, latest runs (after the fixes below): `cargo fmt --all -- --check` (clean), `cargo clippy -p
wordcraft-engine -p wordcraft-ui-egui --all-targets` (clean), `cargo xtask layers` (ok), `cargo
xtask assets` (57 files attributed), `cargo test -p wordcraft-engine --lib` (1077 passed; the
`ime_body_epoch…` failure recorded below was fixed in this change) and
`cargo test -p wordcraft-ui-egui --lib` (1003 passed).

## Revision history

> 2026-10-31 · Review follow-up 2: the slim-bar badge list now comes from the tabs the ribbon
> strip shows (a new `ribbon::visible_tabs`, shared with the strip itself), so a tab hidden in
> Customize Ribbon gets no badge in Focus or Full Screen View and cannot be picked from the slim
> bar; the report scopes its `cargo xtask ci` claim to the engine and ui-egui gates, which run
> green while the pre-existing Thai font-fallback failure stays red on this machine. 26 tests.

> 2026-10-31 · Review follow-up: fix 2 widened to six command families that read `value` without
> the shared pattern — `draw.inkToShape` (which converted the lasso's strokes), `layout.hyphenation`,
> the character-toggle helper behind `format.bold` and friends, `format.underline`,
> `table.repeatHeader` and `table.rtl` — and fix 4 widened to Full Screen View, whose Search box's
> own `Q` badge would have hidden the tab badges, and to the badges not doubling up when Focus's
> ribbon is dropped. 25 tests.

> 2026-10-31 · Fixes: issues 1–5 resolved — the empty window keeps the screen state and its bar, so
> nothing can strand a full-screen window; `p::toggle`/`p::switch` refuse a non-boolean `value`
> across every switch; a chrome change hands the keyboard back to the page; keytips draw on the slim
> bar and drop the ribbon; large ribbon buttons are named for screen readers. Two older tests that
> clicked by a now-shared label pick by position. The flaky `ime_body_epoch_marks_body_replacements_not_edits`
> (a hard-coded document id) is fixed too. The engine and ui-egui gates run green; `cargo xtask ci`
> stays red on this machine at the pre-existing
> `wordcraft-fonts::a_thai_run_with_latin_falls_back_to_one_face` (clean `main` fails the same way:
> this machine's Thonburi lacks layout tables, so the test cannot pick a shaping face). 24 tests in
> all (8 engine, 16 UI).

> 2026-10-31 · First version: 23 new integration tests (8 engine, 15 ui-egui), all green, pinning six
> findings — the empty-window trap after closing the last document in Focus (severe), the silent
> toggle on a non-boolean `value`, the stranded keyboard when Focus is started without a click,
> invisible keytips in Focus, the ribbon Focus button's missing accessible name, and the
> Focus + Read Mode / web full-screen notes. Also records one pre-existing engine test failure
> (`ime_body_epoch_marks_body_replacements_not_edits`) that makes `cargo xtask ci` red.
