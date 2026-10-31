# Spelling and grammar task pane

> **Last updated:** 2026-10-31 · **Status:** synchronized with main `eecbb8f`; validation of the new synchronization SHA is pending. The parent `6e96b547` passed its exact-head hosted gate, three-platform Printing and proofing preview; those results do not validate this synchronization. Historical status: historical three-platform checks passed; at f1264f3 the revised hosted gate, previews, Windows and Linux passed, while macOS remained unvalidated after pre-run cancellations. Later review corrections required their own hosted checks.

## Word reference

Observed the actual Word for Windows interface on the preparation PC on 2026-10-26, using a newly created document with the synthetic sentence "Le chat mange une souri. Ce documant contient une faute." F7 selected "documant" and opened a right-side pane titled Vérification. Existing documents were left unchanged. The exact Word build was not recorded.

At a 1920 × 1152 window, the pane occupied approximately 400 logical pixels on the right. Its body had a category heading and dictionary explanation, a framed sentence context approximately 48 px high, a Suggestions heading, and three bordered suggestion rows approximately 68 px high. Ignore Once, Ignore All and Add to Dictionary were text actions below the list. A suggestion's chevron opened Read Aloud, Spell Out, Change All and AutoCorrect actions. These are observations, not a copied screenshot or asset.

WordCraft uses this arrangement and its shared pane host, icons, typography and theme palette. Clicking a suggestion applies the correction directly and advances; the central modal proposal is superseded. The pane supports sentence context, Read Aloud through the existing command, manual replacement and AutoCorrect actions. Word's dictionary definitions/synonyms, Spell Out and Change All are not implemented here. Visual similarity must be assessed from the generated light/dark previews; implementation alone is not evidence of pixel identity.

## Scope and behavior

Review › Spelling & Grammar, Home › Editor and F7 open the task pane. The pass begins at the next issue from the current selection and continues to the end of the current story (body, header, note or other active text), not all document stories or only a selection. Each correction uses the existing review command and remains undoable. Ignore Once advances without changing the document or dictionaries. Ignore All lasts for the session; Add to Dictionary uses the personal dictionary. Those two actions are offered only for spelling issues. Completion remains visible; Check Again starts a wrapping search. The pane host supplies Close and docking.

If another command changes the document body or selection, a cached suggestion is refused; Check Again obtains a fresh target. Read-only/protected document commands retain their checks. The pane is local to each window, clears on a document switch and does not alter the compact right-click menu. Programmatic review commands return structured results without opening UI.

This work is independent of French contribution #3185. It consumes language-aware engine results and adds neither dictionaries nor grammar rules. Labels use the external shared translation lookup with English fallback for missing keys. No release date or readiness increase is implied.

## Technical synchronization (2026-10-31)

The existing pane is merged with main `eecbb8fe57a56340e7fa109e8da7f1376a03e154`, retaining its updater/dependency changes and current instructions. Proofing source and regressions are unchanged from `6e96b547`: a valid explicit position remains a point lookup at the focus, while an absent/unparseable position preserves exact-selection priority. Navigation, correction/undo, compact menus and per-window state are retained. Fresh exact-SHA hosted checks and rendered previews must validate this synchronization; no new feature or readiness increase is claimed.

## Technical synchronization and API follow-up (2026-10-30)

The existing pane is merged with main `3b7f7ed10b2eb0ae3c39b71551d36ed2df1d1d8b`, retaining the upstream menus, pane host and per-window state. For `review.suggestions`, a successfully parsed explicit `pos` is a pure point lookup even when it equals the selection focus; an absent or unparseable `pos` keeps exact-selection priority. The follow-up preserves correction and undo; fresh hosted checks must validate its new SHA. Results for its parent `c077c155` do not validate the follow-up.

## Technical synchronization (2026-10-27)

The language-independent pane is rebased by a three-way merge with main `436d444ef0ee35c2e2b9fb44f87aa7d08d72641f`. Engine navigation and exact selected-range lookup live in `crates/engine/src/cmd/review/proofing.rs`; the existing regressions follow into `review/tests.rs`. The UI command hook follows into `app/commands.rs`, construction into `app.rs`, inspection into `control/handler.rs`, and pane state/closure into `panes/state.rs` and `panes/host.rs`. The upstream context-menu and dropdown implementations stay intact. Workflow path filters cover the extracted modules while retaining historical paths. No dictionary, grammar rule, new feature or readiness increase is introduced by this synchronization.

## Verification

- Engine: one-/two-paragraph navigation, forward-only completion, explicit wrapping, exact spelling selection within overlapping grammar in either selection direction, explicit point lookup at the focus, absent/unparseable-position selection fallback, unchanged selection, correction and undo.
- UI: direct suggestion correction and undo, Ignore Once progress/completion, Ignore All, personal dictionary addition, stale selection/body refusal, completion and Close, per-window isolation and document switching.
- Offscreen: the ignored spelling_review_dialog_preview test writes light/dark PNGs for comparison with the recorded Word arrangement.
- Required gate: `cargo xtask ci`, plus the existing proofing preview workflow. Local attempt outcomes are recorded separately; an environment-limited local attempt does not replace hosted checks. No local compilation or test success is claimed for this synchronization. The coordinator must validate its new SHA; earlier hosted results validate their own SHAs only.

The first light/dark artifacts at `20205a5` were inspected: the sentence frame shrank to its text and the unbounded suggestion list pushed actions below the viewport. The revised layout fills the pane width and scrolls the suggestion list after approximately three cards, keeping the actions below it visible. At `f1264f3`, the revised light/dark previews and full hosted gate passed; Windows and Linux UI checks passed, while macOS remained unvalidated after pre-run cancellations. Both PNGs were inspected, including the right-aligned Read Aloud control. The following review revision gives the exact selected spelling range priority over an overlapping grammar issue and requires new hosted checks. Exact pixel parity is not claimed.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-31 | trivial | Synchronize the existing Proofing pane with main `eecbb8f`; preserve explicit-position lookup, implicit selection, navigation, undo and per-window state; fresh exact-SHA hosted validation pending, readiness unchanged |
| 2026-10-30 | trivial | Merge the existing pane with main `3b7f7ed`; preserve explicit-position point lookup at the selection focus, implicit/parse-fallback selection priority and correction/undo; new follow-up SHA requires fresh hosted validation |
| 2026-10-27 | trivial | Synchronize the existing pane with extracted engine/UI modules on main `436d444`; preserve selection/navigation/undo tests and compact menus, expand workflow path filters; new-SHA validation pending |
| 2026-10-26 | minor | Resolve selected spelling before overlapping grammar, add command/pane regressions, include pane-integration workflow paths and distinguish validated history from pending review corrections |
| 2026-10-26 | minor | Inspect first successful light/dark artifacts; fill the sentence frame and bound suggestion-list height, retaining all suggestions through scrolling |
| 2026-10-26 | minor | Actual Word for Windows observation replaces the modal proposal with a right-side Proofing pane, sentence context and directly clickable suggestion cards |
| 2026-10-26 | minor | Separate language-independent correction-form proposal and verification scope |
