# Evidence record — wordcraft-validate

One row per (fixture × consumer). An `observed` evidence grade in the checker
traces to a row here. New consumers add rows; a result for one consumer is
never silently promoted to a claim about another.

**Consumer for all current rows:** Word for the web (Microsoft 365 web
build; exact build/version not exposed by the product UI).
**Dates:** 2026-11-06. **Protocol:** file uploaded, opened, editing attempted
(Edit clicked when the file opened in Reading View); verdicts scored only
after the product's own error latency elapsed; no document was edited or
saved.

**Response tiers observed in this consumer:** (1) blocking "can't be opened
for editing" dialog; (2) generic full error page; (3) yellow "Document
corrupted" banner, view-only, content rendered; (4) Reading View opens, Edit
refused as corrupt; (5) opens in editing mode with content loss; (6) opens
clean. A refusal whose wording cites an unsupported *feature* (e.g. custom
XML elements) is a by-design editing limit of the web consumer, **not**
corruption — the verbatim wording is what distinguishes the two.

## Calibration batch 1 — package / XML / relationship rules

| Fixture | Outcome | Notes |
|---|---|---|
| 00-control | opens clean, editing mode | rich valid control (corrected picture construction) |
| 01-not-zip | refusal, blocking dialog | "Sorry, this document can't be opened for editing." |
| 02-dup-entry | refusal, blocking dialog | same wording |
| 03-case-collision | refusal, blocking dialog | same wording |
| 04-no-content-types | refusal, full error page | generic service error page |
| 05-no-main | refusal, full error page | generic service error page |
| 06-ct-undeclared | tier 3: corruption banner, view-only | content fully rendered |
| 07-ct-main-type | refusal, full error page | generic service error page |
| 08-xml-malformed-main | refusal, blocking dialog | as 01 |
| 09-xml-malformed-unused | opens clean, editing mode | malformed unreferenced part ignored entirely |
| 10-xml-undeclared-prefix | refusal, blocking dialog | as 01 |
| 11-rel-dup-id | tier 4: Reading View, Edit refused | "Sorry, Word can't edit this document because it appears to be corrupt." |
| 12-rel-dangling | tier 5: opens in editing mode, content loss | referenced picture shows "Unable to load: picture"; rest intact |
| 13-rel-typed | tier 4: Reading View, Edit refused | header missing in view; corrupt wording on Edit |

## Calibration batch 2 — id / reference / structure / media rules

| Fixture | Outcome | Notes |
|---|---|---|
| 14-comment-dup | tier 4: Edit refused ("can't be opened for editing") | **contradicts ECMA-376 §17.13.4.2 tolerance text**; severity restored to Error on this observation |
| 15-comment-ref | tier 3: corruption banner, view-only | comment silently absent |
| 16-docpr-dup | opens clean | both pictures render; **conflicts with desktop field reports** — shape-dependent; graded Warning with both observations on record |
| 17-bookmark-dup | opens clean | both bookmark icons shown |
| 18-revision-dup | opens clean | both insertions render as tracked changes |
| 19-note-ref | tier 5: opens, silent content loss | phantom footnote: mark renders, footnote pane shows "1 / 1" with an empty body |
| 20-header-ref | refusal, blocking dialog | "can't be opened for editing" |
| 21-style-ref | opens clean | fallback styling, no complaint |
| 22-fld-unbalanced | tier 4: Edit refused | "Sorry, there was a problem and we can't open this document. If this happens again, try opening the document in Microsoft Word." **contradicts §17.16.18 graceful-degradation text**; Error restored |
| 23-tc-no-p | opens clean | cell present in structure, nothing visible; spec basis stands, web tolerance recorded |
| 24-deltext | opens clean | text renders as a tracked deletion anyway (lenient recovery) |
| 25-move-unpaired | opens clean, semantic loss | text demoted to plain text; move semantics silently dropped |
| 26-media-magic | tier 5: opens, content loss | picture does not render; no content-sniffing rescue |

## Oracle probes — extension-part content-type spellings (A/B)

**Consumer:** Word for the web (build/version not exposed). **Date:** 2026-11-06.
Two probe files, byte-identical apart from the three MS-DOCX extension-part
content-type strings:

| Probe | SHA-256 | Outcome |
|---|---|---|
| `spelling-ooxml.docx` | `c5d4450818f7f41d4c8602261cb4d85837d7409eeab4d0c73dacb78b9a9976b3` | opens for editing; `commentsExtended` resolved state honoured |
| `spelling-msword.docx` | `76ef1c909a6ac8a16a5b3211769ea70ca7d2abd8c3b980f8967364017078a48a` | edit-refused as corrupt ("Sorry, Word can't edit this document because it appears to be corrupt.") |

**Caveat:** all three content-type strings changed together, so the refusal is
observed for the set, not isolated per part. The three CT-MISMATCH extension
rows (`commentsExtended`, `commentsIds`, `people`) therefore rest on this one
combined observation; `commentsExtended` additionally has the positive
observation that its resolved state survives under the accepted spelling.

## Pre-calibration finding — picture construction

The first hand-built control was flagged "Document corrupted" (view-only)
because its `pic:pic` element lacked the required `pic:nvPicPr` and
`pic:spPr` children. With them, the control opens clean. A
missing-required-children content-model violation in a drawing earns the
view-only corruption banner in this consumer — the first data point for the
sequence/cardinality rule family.

## Stress corpus — Word outcomes

`small-valid.docx` (stress control): **edit-refused, tier 4 wording** —
"Sorry, Word can't edit this document because it appears to be corrupt." —
while this checker returns PASS. Recorded as a known gap in MANIFEST.md.

Dissection rows (same consumer, same dates):

- Scaffolding-only derivative (minimal content, all support parts present
  but empty): opens clean in editing mode.
- Fifteen derivatives each removing one feature family from small-valid:
  all still edit-refused with the corruption wording, **except** the
  glossary-removal derivative, which instead drew the unsupported-feature
  refusal: "Sorry, this document can't be opened because it contains custom
  XML elements. To edit this document, please open it in Microsoft Word."
- Seven derivatives each adding one residue feature to the scaffolding base
  (bookmarks, permission ranges, hyperlinks, block SDT, block `w:customXml`,
  rsid accumulation, pure scale to ~626 KB): all open clean **except** the
  customXml addition, which drew the unsupported-feature refusal above —
  not the corruption wording.
- Conclusion on record: no single family or residue feature explains the
  corruption refusal; dissection of the real file's bytes (body truncation
  and one-at-a-time part replacement) is in progress, and this section will
  be updated with the resolution.

## Stress corpus — dissection round 4 and resolution (same consumer, 2026-11-06)

Seventeen variants of the real `small-valid.docx`, each changing exactly one
thing: body truncations (empty body; first 1/5/13/27 of 54 children) and
one-at-a-time part replacements from the clean scaffolding file (document,
styles, settings, numbering, comments, commentsExtended, commentsIds,
footnotes, endnotes, header1, footer1) **all still edit-refused with the
corruption wording**; only the glossary-removal variant changed the wording,
to the custom-XML refusal. The trigger therefore lived outside the document
part entirely.

**Root cause (direct inspection, then oracle confirmation).** The package
manifest declared the glossary part as
`...wordprocessingml.document.main+xml`; ECMA-376 Part 1 prescribes
`...wordprocessingml.document.glossary+xml` for the part reached by the
`glossaryDocument` relationship (the relationship itself was correct). The
package in effect claimed two main documents. A variant with **only that
one content-type string corrected** lost the corruption refusal entirely —
it then drew only the custom-XML unsupported-feature refusal recorded
above, from the file's (legal) body `w:customXml` element.

The defect was in the corpus generator (all ten files) and was also a
checker blind spot: the checker's CT-MISMATCH rule did not cover the
glossary pairing, and its role discovery adopted the mistyped glossary as
a second main owner. Both are fixed: the generator emits the correct type
and audits every relationship/content-type pairing at build time; the
checker verifies the pairing (CT-MISMATCH, Error, observed — this row).
The corpus was regenerated; MANIFEST.md carries the fixed hashes.

The remaining nine stress files have not yet been shown to this consumer.
