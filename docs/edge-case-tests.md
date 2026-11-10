# Document and account edge-case tests

Last updated: 2026-11-10. These behavioral regressions supplement the application’s existing tests.
They execute engine commands and the real egui application through its headless harness.

The focused run on 2026-11-10 passed all 11 document/account tests (8 engine, 3 UI) on macOS ARM64.
The initial zero-byte-file refusal expectation was corrected
to the documented blank-document behavior; the original draft remains available.

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

## Finding from the broader suite

Three existing strict-export CLI tests failed on macOS because the system’s `.LastResort`
font claims coverage for unsupported characters by drawing placeholder boxes. A direct strict
PDF export of U+40000 incorrectly succeeded before the fix. Automatic character and whole-run
fallback now exclude that placeholder family, while explicit selection and genuine fallback
fonts remain available. A twelfth regression uses synthetic family names on bundled open font
bytes, so this check does not require an Apple font. It failed before the fix and passed after it.

Final validation on 2026-11-10: `cargo xtask ci` passed all nine steps (format, Clippy,
workspace tests, interface audit, language check, doctests, assets, layers and WebAssembly).
The three existing strict-export regressions also passed. A live CLI recheck of U+40000
returned exit code 3, reported the missing glyph, and created no PDF.

## Revision history

| Date | Change |
| --- | --- |
| 2026-11-10 | Added document-lifecycle, filename, filesystem-failure and account boundary coverage. |
