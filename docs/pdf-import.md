# Open a PDF as an editable document

Choose **File → Open** and select a PDF, or drop it into WordCraft. Text becomes ordinary
editable paragraphs; supported raster pictures become inline pictures. The import report
lists conversion limits. A PDF with a user password uses the existing password prompt.

The converted document is an **unsaved copy**. Save asks for a new filename; choose DOCX to
keep editing. Opening or editing never rewrites the source PDF. An explicit export to a PDF
path still writes the edited document's newly generated PDF, not an incremental edit of the
original PDF.

The same importer is available to automation:

```sh
wordcraft-cli convert input.pdf editable.docx
```

`file.open` accepts a filesystem `path`, or a filename plus base64 `data` (web/CLI/MCP), and an
optional `password`. The password opens the PDF; it is not automatically applied to a new DOCX. Use `file.encrypt` or a Save As password to protect that copy. Its result includes `import.format`, `pages`, `textCharacters`, `images`,
`warnings` and `saveAsRequired`. A renamed PDF is detected from its header before text/HTML
decoding. Existing `io::open_bytes` / `open_path` callers use this same conversion.

## What survives

- Unicode text where the PDF supplies a usable mapping, including invisible OCR text layers.
- Font size and solid text color; font family, bold and italic where font metadata is available
  (including page-level standard Type1 fonts). Fonts are resolved by WordCraft when editing;
  PDF font subsets are not installed or embedded in the resulting DOCX.
- Approximate horizontal reading order and word spacing. Neighboring lines with similar
  indentation, leading and size join into a reflowing paragraph. Source page boundaries become
  paragraph page breaks, but edited text may occupy more pages.
- Raster pictures, including image-based scans, decoded to PNG and scaled to fit the document.

## Limits

PDF is a page-description format, so this is not a lossless PDF-to-Word round trip. Tables,
columns, headers/footers, lists, footnotes, links, forms, annotations, tagging, vector drawings,
rules, backgrounds, arbitrary clipping/rotation and complex bidirectional/vertical layouts are
not reconstructed. Review the result before using it. The original PDF remains available.

There is **no OCR**: a scan without a text layer stays a picture. Outlined text without Unicode
cannot become editable text. Missing Unicode mappings produce replacement characters and a
warning. Files containing only unsupported vector content are rejected instead of silently
opening as an empty document. A failed import leaves the current document unchanged.

Import accepts up to 64 MiB of encoded PDF input and 500 pages, with up to 100,000 glyphs per
page and one million overall (at most 8 MiB of decoded text and 4096 fonts per page). Pictures are limited to 16 megapixels each, 64 megapixels total
and 1000 occurrences. These bound the conversion's own collections; decoding is provided by
hayro. Per-page interpretation caches are released after conversion. Reading order is sorted
once per page and paragraph runs are assembled in linear passes.

Tests use generated PDFs only: positioned words, fill/stroke text, font metadata, Unicode,
page boundaries, scanned pictures, malformed/oversized input, renamed PDFs, and an
open/edit/save-to-DOCX round trip that verifies the source PDF remains unchanged.
