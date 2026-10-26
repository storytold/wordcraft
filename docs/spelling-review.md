# Spelling and grammar task pane

> **Last updated:** 2026-10-26 · **Status:** historical three-platform checks passed; at f1264f3 the revised hosted gate, previews, Windows and Linux passed, while macOS remained unvalidated after pre-run cancellations. Overlap-review corrections now require new hosted checks.

## Word reference

Observed the actual Word for Windows interface on the preparation PC on 2026-10-26, using a newly created document with the synthetic sentence "Le chat mange une souri. Ce documant contient une faute." F7 selected "documant" and opened a right-side pane titled Vérification. Existing documents were left unchanged. The exact Word build was not recorded.

At a 1920 × 1152 window, the pane occupied approximately 400 logical pixels on the right. Its body had a category heading and dictionary explanation, a framed sentence context approximately 48 px high, a Suggestions heading, and three bordered suggestion rows approximately 68 px high. Ignore Once, Ignore All and Add to Dictionary were text actions below the list. A suggestion's chevron opened Read Aloud, Spell Out, Change All and AutoCorrect actions. These are observations, not a copied screenshot or asset.

WordCraft uses this arrangement and its shared pane host, icons, typography and theme palette. Clicking a suggestion applies the correction directly and advances; the central modal proposal is superseded. The pane supports sentence context, Read Aloud through the existing command, manual replacement and AutoCorrect actions. Word's dictionary definitions/synonyms, Spell Out and Change All are not implemented here. Visual similarity must be assessed from the generated light/dark previews; implementation alone is not evidence of pixel identity.

## Scope and behavior

Review › Spelling & Grammar, Home › Editor and F7 open the task pane. The pass begins at the next issue from the current selection and continues to the end of the current story (body, header, note or other active text), not all document stories or only a selection. Each correction uses the existing review command and remains undoable. Ignore Once advances without changing the document or dictionaries. Ignore All lasts for the session; Add to Dictionary uses the personal dictionary. Those two actions are offered only for spelling issues. Completion remains visible; Check Again starts a wrapping search. The pane host supplies Close and docking.

If another command changes the document body or selection, a cached suggestion is refused; Check Again obtains a fresh target. Read-only/protected document commands retain their checks. The pane is local to each window, clears on a document switch and does not alter the compact right-click menu. Programmatic review commands return structured results without opening UI.

This work is independent of French contribution #3185. It consumes language-aware engine results and adds neither dictionaries nor grammar rules. Labels use the external shared translation lookup with English fallback for missing keys. No release date or readiness increase is implied.

## Verification

- Engine: one-/two-paragraph navigation, forward-only completion, explicit wrapping, exact spelling selection within overlapping grammar in either selection direction, explicit-position fallback, correction and undo.
- UI: direct suggestion correction and undo, Ignore Once progress/completion, Ignore All, personal dictionary addition, stale selection/body refusal, completion and Close, per-window isolation and document switching.
- Offscreen: the ignored spelling_review_dialog_preview test writes light/dark PNGs for comparison with the recorded Word arrangement.
- Required gate: cargo xtask ci. Cargo is unavailable on the preparation PC; hosted validation is required, and test definitions alone do not establish success.

The first light/dark artifacts at `20205a5` were inspected: the sentence frame shrank to its text and the unbounded suggestion list pushed actions below the viewport. The revised layout fills the pane width and scrolls the suggestion list after approximately three cards, keeping the actions below it visible. At `f1264f3`, the revised light/dark previews and full hosted gate passed; Windows and Linux UI checks passed, while macOS remained unvalidated after pre-run cancellations. Both PNGs were inspected, including the right-aligned Read Aloud control. The following review revision gives the exact selected spelling range priority over an overlapping grammar issue and requires new hosted checks. Exact pixel parity is not claimed.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | minor | Resolve selected spelling before overlapping grammar, add command/pane regressions, include pane-integration workflow paths and distinguish validated history from pending review corrections |
| 2026-10-26 | minor | Inspect first successful light/dark artifacts; fill the sentence frame and bound suggestion-list height, retaining all suggestions through scrolling |
| 2026-10-26 | minor | Actual Word for Windows observation replaces the modal proposal with a right-side Proofing pane, sentence context and directly clickable suggestion cards |
| 2026-10-26 | minor | Separate language-independent correction-form proposal and verification scope |
