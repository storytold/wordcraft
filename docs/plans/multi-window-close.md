# Implementation plan: closing document windows

For [the specification](../specs/multi-window-close.md) (#7822, part of #7671). Steps run in order; each is
its own PR.

## 1. craft-libs prerequisites (base `next`)

- **Hidden root window** (storytold/craft-libs#584, [ADR 0001](../adr/0001-hidden-root-window.md)): the root
  OS window can close or hide while document windows stay open, and adopts nothing.
- **Test kit with any number of windows** (C5-8): `craft_ui_test_kit::windows::Windows` keeps focus and close
  events per child, so three-window tests can target one child.
- Not in scope: the cursor on non-root windows (C5-5, `cursor::image_target`) gets its own issue.

## 2. Repin craft-libs in WordCraft

Brings storytold/craft-libs#3290 (one modal stack across windows) and the two changes above. Stage 3 of
#7671 does this repin.

## 3. WordCraft PR (Fixes #7822)

Code is in `crates/ui-egui/src/` unless noted.

| Spec rule | Change |
|---|---|
| 1 | Drop the root taking over another document (`settle_windows` in `windows.rs`); closing the root window closes it through step 1's hidden root. Rewrite `closing_a_window_asks_first_and_the_root_adopts_another`. |
| 2 | `other_window_on_document` filters windows by document (C5-6). |
| 3, 9 | `app::Closing` records the window that owns the prompt. Pumping, cancelling and closing the prompt use that window's dialog host, not the active one. |
| 4, 9 | `run` and `close_decision` check `closing` first: a close while one is running raises the prompt and returns, never calls `begin_closing` again. |
| 9 | Clearing `closing.prompt` in `sync_document` and `apply_close_answer` either continues the close or ends it, so no `Hold` is left with no prompt. `apply_close_answer` matches the guard's document, not the active one. |
| 5 | Cancelling Save As during a close cancels the close or quit (`continue_after_save`). |
| 7 | On one-document-at-a-time hosts, the root close acts on the active entry, not `root_key()`; the fallback layer keeps its close button (C5-1..3). |
| 8 | `file.close` takes `save`; `close_refused` names both options. `ui.closeWindow` takes `window` and defaults to `active_viewport()` (`control/handler.rs`). Update `docs/control-protocol.md` and `docs/mcp.md`. |
| — | `queue_for_window` repaints the target window (C5-7). |

Tests: the table in the specification, in `windows/tests.rs` and `documents.rs`.

Checks: `cargo fmt --all --check`, `cargo clippy -p wordcraft-ui-egui --all-targets -- -D warnings`,
`cargo test -p wordcraft-ui-egui`, `cargo xtask layers`, `cargo xtask wasm`.
