# ODT element audit

> **Last reviewed:** 2026-11-04 · **Last updated:** 2026-11-04 · **Change:** trivial (paragraphs tied together kept as read as groups); previously trivial (paragraphs with charts kept as read); previously trivial (sections in table cells and at empty paragraphs kept); previously trivial (password, number, time and grid form controls kept); previously minor (first version: every element of the ODF 1.3 schema against two corpora and a generated sample of each element never seen)

Every element the ODF 1.3 schema (OASIS, Part 3, RELAX NG) defines, checked against what WordCraft's ODT reader and writer do with it. The question for each: does a file holding it open, and does a save without edits keep it? (See [file-format-parity.md](file-format-parity.md) for what WordCraft models; markup it doesn't model is written back as read while unedited: [odf-interpretation.md](odf-interpretation.md).)

## Method

1. **The schema**: its 596 element names, by namespace.
2. **Two corpora** (kept locally, not in the repository): 133 documents (LibreOffice's templates, bug-tracker and converter files, documents made for the checks) and 232 from the ODF Toolkit's and Calligra's test data, made by Microsoft Office, OpenOffice.org, StarOffice, LibreOffice, Calligra/KOffice, Lotus Symphony and others. Each is opened and saved by WordCraft, both copies are rendered by LibreOffice and compared page by page and by text, and a loss check lists every element, attribute and value the saved file has fewer of.
3. **Elements no file held**: a small document of our own for each one a text document can hold, opened and saved by WordCraft without edits and with an edit, each saved copy checked well-formed and opened in LibreOffice.

## Namespaces

| Namespace | Elements | Seen in a file | Note |
|---|---|---|---|
| `text:` | 174 | 124 |  |
| `table:` | 98 | 11 | most are a spreadsheet's (ranges, pivot tables, database ranges); a text document's tables use few |
| `draw:` | 44 | 27 |  |
| `style:` | 43 | 42 |  |
| `office:` | 28 | 23 |  |
| `form:` | 30 | 13 |  |
| `number:` | 27 | 18 |  |
| `svg:` | 10 | 2 |  |
| `dc:` | 6 | 6 |  |
| `meta:` | 15 | 14 |  |
| `config:` | 5 | 5 | settings.xml, kept as read |
| `script:` | 1 | 1 |  |
| `math:` | 1 | 1 | MathML, in formula objects |
| `xforms:` | 1 | 0 |  |
| `chart:` | 25 | 18 | charts, read from their own objects |
| `dr3d:` | 6 | 0 | 3D scenes, kept as read with their paragraph |
| `presentation:` | 20 | 6 | presentations (a text document holds none) |
| `anim:` | 12 | 0 | presentations |
| `db:` | 50 | 0 | databases only |

## Elements no file held, a text document can

Each in a document of our own: it opens, saves without edits keeping it, and saves with an edit; every saved copy is well-formed and LibreOffice opens it.

| Element | Unedited save |
|---|---|
| `draw:contour-path` | kept |
| `draw:glue-point` | kept |
| `draw:image-map` | kept |
| `draw:plugin` | kept |
| `form:connection-resource` | kept |
| `form:grid` | kept (as read, with its columns; LibreOffice opens the saved file, though its PDF export of a grid control fails whenever a document has automatic styles, ours or its own) |
| `form:list-value` | kept (the sample held none; a list box's values are kept with the form) |
| `form:password` | kept (as read) |
| `number:boolean-style` | kept |
| `number:embedded-text` | kept |
| `number:era` | kept |
| `number:fill-character` | kept |
| `number:fraction` | kept |
| `number:percentage-style` | kept |
| `number:quarter` | kept |
| `number:week-of-year` | kept |
| `style:region-center` | kept |
| `text:alphabetical-index-auto-mark-file` | kept with its index (as read) |
| `text:author-initials` | kept |
| `text:creation-time` | kept |
| `text:drop-down` | kept |
| `text:editing-cycles` | kept |
| `text:editing-duration` | kept |
| `text:expression` | kept |
| `text:hidden-paragraph` | kept |
| `text:linenumbering-separator` | kept |
| `text:meta-field` | kept |
| `text:modification-time` | kept |
| `text:note-continuation-notice-forward` | kept |
| `text:object-index` | kept |
| `text:page-variable-get` | kept |
| `text:page-variable-set` | kept |
| `text:print-time` | kept |
| `text:printed-by` | kept |
| `text:sender-country` | kept |
| `text:sender-initials` | kept |
| `text:sender-phone-private` | kept |
| `text:sender-title` | kept |
| `text:sort-key` | kept |
| `text:table-formula` | kept |
| `text:table-index` | kept |
| `text:toc-mark-start` | kept |
| `text:toc-mark` | kept |
| `text:user-field-input` | kept |
| `text:user-index-mark-start` | kept |
| `text:user-index-mark` | kept |
| `text:user-index` | kept |

## Found, fixed

The check found these, each fixed with a test (see [file-format-parity.md](file-format-parity.md), 2026-11-03 and 2026-11-04):

- A file the package held without a media type (another application's, LibreOffice's empty accelerator configuration) was written without its manifest entry, and LibreOffice refused the saved file.
- A section hidden with `text:display="none"` showed its text once written without the section.
- A reference mark was written as a bookmark only, so references to it found nothing.
- Lists lost what a paragraph's markup can't say (a label followed by a space, picture bullets, an unnumbered header): unedited lists are now kept as read.
- Styles of different families, and list styles, sharing a name overwrote each other among the definitions kept.
- A form's data source (`form:connection-resource`) was taken for a control; index marks over text, plugins and floating frames weren't kept.
- Password, number, time and grid form controls (ODF 1.3 §13.5), of kinds WordCraft doesn't model, were dropped; they are kept as read, in their place in the text, as buttons and image controls were.

## Left

- **A paragraph showing a form control**: written from the model, which keeps what WordCraft models of it.
- **Calligra's page sequence** (`text:page-sequence`, a layout of fixed pages) and **numbered paragraphs** (`text:numbered-paragraph`): read as text, written from the model.
- **Soft page breaks between paragraphs** (a layout hint LibreOffice recomputes) and **unused automatic styles**: not written.

## Revision history

| Date | Change | What |
|---|---|---|
| 2026-11-04 | minor | First version |
| 2026-11-04 | trivial | Password, number, time and grid form controls kept |
| 2026-11-04 | trivial | Sections in table cells and at empty paragraphs kept |
| 2026-11-04 | trivial | Paragraphs with charts kept as read |
| 2026-11-04 | trivial | Paragraphs tied together (a comment, a change or a mark reaching into another, linked text boxes) kept as read as groups |
