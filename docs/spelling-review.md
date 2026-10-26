# Spelling and grammar correction form

> **Last updated:** 2026-10-26 · **Status:** implementation proposed; hosted checks and visual acceptance must be verified before integration.

Review › Spelling & Grammar, Home › Editor and F7 open a persistent correction form. The issue's text, explanation and proofing language appear above the suggestions. Choose a suggestion or type replacement text in Change to, then choose Change. Each replacement uses the existing review command and is one undoable document edit. Ignore Once advances without changing the document or dictionaries. Ignore All lasts for the session; Add to Dictionary uses the existing personal dictionary. Those two actions are offered only for spelling issues.

The pass begins at the next issue from the current selection and continues to the end of the current story (body, header, note or other active text). It does not traverse all document stories or restrict itself to a selected range. Check Again starts another search with wrapping; ignored-once occurrences can be revisited. Completion stays visible until Close or Escape. The compact right-click/Shift+F10 menu is retained.

The form consumes language-aware engine results. It works with the existing proofing languages and is independent of French contribution #3185; adding an interface does not install a dictionary or add grammar rules. Interface labels use the shared translation lookup and fall back to English when a catalog lacks a key. No release date or completeness increase is implied.

If another command changes the document body or selection while the form is open, an old suggestion is refused. Check Again obtains a fresh target. Read-only/protected document commands retain the engine's existing checks and display refusals in the form.

## Verification

- Engine regression: one- and two-paragraph navigation, forward-only completion and explicit wrapping.
- UI regressions: suggestion application and undo, Ignore Once progress/completion, Ignore All, personal dictionary addition, stale selection/body refusal, no-issue completion and Escape.
- Offscreen preview: the ignored spelling_review_dialog_preview test writes light/dark PNGs; inspect them before integration.
- Required repository gate: cargo xtask ci. Cargo is unavailable on the preparation PC, so hosted checks are required; test definitions alone are not evidence of success.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | minor | Separate language-independent correction-form proposal and verification scope |
