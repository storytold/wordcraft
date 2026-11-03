# Optional PDF TOC refresh

`wordcraft-cli convert input.docx output.pdf --refresh-toc` rebuilds one supported body TOC in the disposable conversion session. The input file is never saved. Without the flag, conversion keeps the cached TOC structure and refreshes print cross-references as before.

The option uses `references.updateToc` with `forPdf: true`. It does not run `references.updateFields`, create a missing TOC, or change unrelated field caches. PDF is required. Combining it with `--update-fields` is an error. Refresh remains off by default.

Protected documents and a locked ordinary TOC skip rebuilding; normal print-field policy still applies. Field-code display is an explicit unsupported error. `TOC \c` figure lists are left unchanged and do not count as ordinary TOCs. Multiple ordinary TOCs, nested/non-body TOCs, malformed boundaries, unsupported objects, unknown switches, hidden/deleted headings and unsupported section boundaries return an error before an output file is written. Multiple ordinary TOCs remain unsupported even if locked.

The existing generator implements `\o`, `\u`, `\t`, `\f`, `\l`, `\h`, `\n`, `\p`. `\z` is retained and has no PDF-specific effect. An argument-less `\o` keeps the existing levels 1–9 default; trailing empty style-map separators keep the existing parser's behavior. Heading selection follows the field's selectors and existing styles/outline levels. A style named Heading2NoTOC without an outline level is not a built-in Heading2. No conditional template intent is inferred from missing bookmarks.

The transient adapter preserves a section mark on an end-only field paragraph. An offset-zero FieldEnd followed only by one page break or a balanced bookmark pair is detached before regeneration with its exact paragraph/run/mark/section properties. Arbitrary trailing text remains unsupported. A simple unlocked internal HYPERLINK cache is supported inside the replaced TOC; external links and nested range fields remain unsupported. The existing two-pass generator keeps a canonical TOCn style first. If absent, it can use the existing built-in paragraph style TOC n, preserving its inherited tabs and formatting. It does not add or rename styles. A final print-layout check rejects unresolved entry/page differences rather than emitting stale TOC numbers. TOC instructions are limited to 4096 bytes and entries to 10000. Unsupported forms can use the caller's explicit fallback policy; they are not reported as successful refreshes.

This feature depends on the transient print-reference settlement in PR #2612. It does not establish native Word rebuild parity.

The CLI reports `toc-refresh` status on stderr: `Refreshed`, `NoTOC`, or `LockedPreserved`. A refresh error exits without writing a PDF. Callers must distinguish that unsupported result from successful refresh.
