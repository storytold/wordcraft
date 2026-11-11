# Document and account edge-case tests

Last updated: 2026-11-10. These behavioral regressions supplement the application’s existing tests.
They execute engine commands and the real egui application through its headless harness.

Run the focused suite with the same pinned fonts and language data used by CI:

```sh
cargo test --locked -p wordcraft-engine -p wordcraft-ui-egui --lib edge_case_
```

| Area | Boundaries and observable guarantees |
| --- | --- |
| Rename input | Missing and non-string parameters, control characters and invalid separators are refused without modifying the draft or consuming its undo step. |
| Unicode | ASCII, accented letters, CJK and emoji names accept 200 characters and refuse 201 without changing the accepted name. |
| Malformed documents | Corrupt/truncated DOCX, invalid legacy DOC and invalid ODT inputs leave the current document, metadata, suggested name and undo history intact. |
| Empty files | A zero-byte document opens as a new blank document while the original draft, suggested name and undo history remain available. |
| Account settings | Blank names are refused; long Unicode names and control characters normalize within the existing limits, without editing the document. |
| Save and rename failures | A directory collision preserves the original bytes and unsaved edits; a later successful save writes those edits. |
| External deletion | A missing rename source reports failure while retaining the in-memory document, which can still be saved elsewhere. |
| Symlink collisions | On Unix, live and dangling destination links are refused without replacing links or modifying either file. |
| Document switching | New/Open close a pending Rename dialog, preventing its name from reaching another document. |
| Cancellation | An invalid name shows an error; Escape closes the dialog and retains the original name. |
| Same-document editing | A command that edits the existing document keeps Rename available and preserves those edits when renaming. |

These tests use synthetic temporary documents. The UI tests do not drive native file pickers or a browser,
and the Unix symlink test is conditional. They do not replace cross-platform testing, real-document
format fidelity work, long-running recovery/AutoSave soak testing, or native IME acceptance.

## Placeholder font fallback

macOS's `.LastResort` font maps almost every code point to a placeholder box. Automatic
character and whole-run fallback, and every coverage check they make, ignore it, so strict
export reports such characters as missing glyphs instead of drawing boxes. Explicit selection
of the family and readable fallback fonts still work. Two regressions use synthetic family
names on bundled open font bytes, so neither needs an Apple font:

```sh
cargo test --locked -p wordcraft-fonts --lib last_resort
```

## Revision history

| Date | Change |
| --- | --- |
| 2026-11-10 | Added document-lifecycle, filename, filesystem-failure and account boundary coverage. |
