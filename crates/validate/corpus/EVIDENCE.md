# Evidence record — wordcraft-validate

One row per (fixture × consumer). An `observed` evidence grade in the checker
traces to a row here. New consumers add rows; a result for one consumer is
never silently promoted to a claim about another.

**Consumer for the original calibration:** Word for the web (Microsoft 365 web
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
| 26-media-magic | tier 5: opens, content loss | The drawing references a JPEG signature stub in a part declared as PNG. The picture does not render. |

## XML-DOCTYPE — isolated part probes, 2026-11-22–23

Experiment A and the per-part series for PR #5875 used original synthetic DOCX
fixtures. The per-part series added one declaration to one member of a shared
control. The fixture names below refer to those probes. The local per-part
manifest records the original series' package hashes and changed members.
Desktop outcomes describe the open attempt. Web outcomes distinguish an open
refusal, an editing refusal, and view mode with a corruption banner.

| Fixture (unused entity) | Target role | Consumer | Mode and outcome |
| --- | --- | --- | --- |
| `doctype-main-document.docx` | Main document | Word for the web | Browser open refused |
| `doctype-header.docx` | Header | Word for the web | Editing refused |
| `doctype-footer.docx` | Footer | Word for the web | Editing refused |
| `doctype-footnotes.docx` | Footnotes | Word for the web | Editing refused |
| `doctype-endnotes.docx` | Endnotes | Word for the web | Editing refused |
| `doctype-comments.docx` | Comments | Word for the web | Editing refused |
| `doctype-glossary.docx` | Glossary document | Word for the web | Editing refused |
| `doctype-settings.docx` | Settings | Word for the web | Editing refused |
| `doctype-styles.docx` | Styles | Word for the web | Editing refused |
| `doctype-numbering.docx` | Numbering | Word for Windows | Desktop open failed as corrupted |
| `doctype-numbering.docx` | Numbering | Word for the web | Opened cleanly |
| `doctype-font-table.docx` | Font table | Word for the web | Editing refused |
| `doctype-theme.docx` | Theme | Word for the web | Editing refused |
| `doctype-chart.docx` | Chart | Word for the web | View-only with corruption banner |
| `doctype-custom-xml.docx` | Custom XML item | Word for the web | View-only with corruption banner |
| `doctype-core-properties.docx` | Package core properties | Word for the web | Editing refused |

Word for Mac also prompted for recovery on the footnotes, endnotes, comments,
glossary, settings, styles, numbering, font table, theme, custom XML item and
core-properties probes. Its timeouts on the main document, header, footer and
chart are corroborating context for the web outcomes. In Experiment A, web
open was refused for `dtd-inert.docx` (unused entity), `dtd-used.docx`
(referenced entity), and `dtd-external.docx` (external-subset declaration),
each with the declaration in the main document. The web controls opened.

| Fixture | Relationships role | Consumer | Mode and outcome |
| --- | --- | --- | --- |
| `doctype-document-rels.docx` | Main document, unused entity | Word for Mac | Desktop recovery prompt |
| `doctype-document-rels.docx` | Main document, unused entity | Word for Windows | Desktop open failed as corrupted |
| `doctype-document-rels.docx` | Main document, unused entity | Word for the web | Browser open refused |
| `dtd-attlist.docx` | Main document, ATTLIST | Word for Mac | Desktop recovery prompt |
| `dtd-attlist.docx` | Main document, ATTLIST | Word for the web | Opened cleanly |
| `doctype-pkg-rels.docx` | Package, unused entity | Word for Windows | Desktop open failed |
| `doctype-pkg-rels.docx` | Package, unused entity | Word for the web | Browser open refused |
| `doctype-pkg-rels.docx` | Package, unused entity | Word for Mac | Desktop recovery prompt |
| `doctype-pkg-rels-attlist.docx` | Package, ATTLIST | Word for Windows | Desktop open failed |
| `doctype-pkg-rels-attlist.docx` | Package, ATTLIST | Word for the web | Browser open refused |
| `doctype-pkg-rels-attlist.docx` | Package, ATTLIST | Word for Mac | Desktop recovery prompt |
| `doctype-chart-rels.docx` | Chart, unused entity | Word for Windows | Desktop open failed |
| `doctype-chart-rels.docx` | Chart, unused entity | Word for the web | Browser open refused |
| `doctype-chart-rels-attlist.docx` | Chart, ATTLIST | Word for Windows | Desktop open failed |
| `doctype-chart-rels-attlist.docx` | Chart, ATTLIST | Word for the web | Browser open refused |
| `doctype-chart-rels-attlist.docx` | Chart, ATTLIST | Word for Mac | Desktop recovery prompt |
| `doctype-item-rels.docx` | Custom XML item, unused entity | Word for Windows | Desktop open failed |
| `doctype-item-rels.docx` | Custom XML item, unused entity | Word for the web | Opened in view mode |
| `doctype-item-rels.docx` | Custom XML item, unused entity | Word for Mac | Desktop recovery prompt |
| `doctype-item-rels-attlist.docx` | Custom XML item, ATTLIST | Word for Windows | Desktop open failed |
| `doctype-item-rels-attlist.docx` | Custom XML item, ATTLIST | Word for the web | Opened in view mode |
| `doctype-item-rels-attlist.docx` | Custom XML item, ATTLIST | Word for Mac | Desktop recovery prompt |
| `doctype-footnotes-rels.docx` | Footnotes, unused entity | Word for Mac | Opened cleanly, both controls clean |
| `doctype-footnotes-rels.docx` | Footnotes, unused entity | Word for Windows | Opened |
| `doctype-footnotes-rels.docx` | Footnotes, unused entity | Word for the web | Opened cleanly |
| `doctype-footnotes-rels-attlist.docx` | Footnotes, ATTLIST | Word for Mac | Opened cleanly, both controls clean |
| `doctype-footnotes-rels-attlist.docx` | Footnotes, ATTLIST | Word for Windows | Opened |
| `doctype-footnotes-rels-attlist.docx` | Footnotes, ATTLIST | Word for the web | Opened in view mode with content rendered |

The ATTLIST main-relationships probe supplies an external hyperlink's omitted
target mode. The ATTLIST footnotes probe supplies a default on an image
relationship. The footnotes observations apply to the reported open modes.
The same validator Warning can accompany a Pass for the unused entity or
Incomplete for the ATTLIST when relationship declaration coverage is lost.

## XML-DOCTYPE — closure part probes, 2026-11-24

The closure series used 15 probe/control pairs from 12 distinct files in the
49,996-file local corpus. The four diagram pairs shared a control source.
Each control was a byte-for-byte copy of its source. Each probe inserted one
DOCTYPE into one part's original prolog. All other member payloads were
unchanged. The table records the SHA-256 of each source filename from the
local manifest. The corpus documents and their contents are held locally.

Word for Mac and Word for Windows opened all matched controls. Word for the web
opened the other 14 controls cleanly. It refused to edit both the altChunk
probe and its untouched control, so that pair provides no evidence of the
declaration's effect on the web. Both desktop consumers opened that probe.

| Target part (role) | Declaration form | Word for Mac | Word for Windows | Word for the web | Corpus filename SHA-256 |
| --- | --- | --- | --- | --- | --- |
| `word/stylesWithEffects.xml` | Unused internal entity | Desktop recovery prompt | Desktop open failed | View-only | `d10b3a2380bc9a056d31725280507888b69d7e14ed029424631447aedfb73c3e` |
| `word/diagrams/data1.xml` | Unused internal entity | Desktop hang (timed out) | Desktop open failed | View-only | `0db00346e9d6efe7e1b11f25ef5ea6001e0716f396cf72fbe3df58b974dc69cb` |
| `word/diagrams/layout1.xml` | Unused internal entity | Opened cleanly | Opened | Opened cleanly | `0db00346e9d6efe7e1b11f25ef5ea6001e0716f396cf72fbe3df58b974dc69cb` |
| `word/diagrams/colors1.xml` | Unused internal entity | Opened cleanly | Opened | Opened cleanly | `0db00346e9d6efe7e1b11f25ef5ea6001e0716f396cf72fbe3df58b974dc69cb` |
| `word/diagrams/quickStyle1.xml` | Unused internal entity | Opened cleanly | Opened | Opened cleanly | `0db00346e9d6efe7e1b11f25ef5ea6001e0716f396cf72fbe3df58b974dc69cb` |
| `word/commentsExtended.xml` | Unused internal entity | Desktop recovery prompt | Desktop open failed | Browser refused | `1c13d58054583c8aa24e05db93abbf54b01d2d9a624797c9eb4317e2a5f9eb56` |
| `word/commentsIds.xml` | Unused internal entity | Opened cleanly | Opened | Browser refused | `9e8fd4c9f6bcb8cffd8ae593098268a7bb8aeedafb7ba8f5a5e9efad6b2ae574` |
| `word/people.xml` | Unused internal entity | Desktop recovery prompt | Desktop open failed | Browser refused | `3eda8a9d24ed09b53b31e8f8a1390ecd6ffff87628093f114f25d5688b64d7c3` |
| `docMetadata/LabelInfo.xml` | Unused internal entity | Desktop hang (timed out) | Desktop open failed | Opened cleanly | `3665d206e148c5b45352ae62870eb07151dfba988578a072b6c154f1cdea17ea` |
| `word/webextensions/webextension1.xml` | Unused internal entity | Desktop hang (timed out) | Desktop open failed | Browser refused | `5a250435b731686a6850116d55b84995b222947ccc5adaa14cd3ee46b569be19` |
| `word/_rels/header1.xml.rels` | Unused internal entity | Desktop recovery prompt | Desktop open failed | Browser refused | `742b205a0d7aa658f611d85a395c49244cf926033c53b1ab4cd525b27e07e340` |
| `word/_rels/footer1.xml.rels` | Unused internal entity | Desktop recovery prompt | Desktop open failed | Browser refused | `444c82ee5b46d16c7e4835324c679458019721443c7fd854670657615fee9a23` |
| `word/_rels/endnotes.xml.rels` | Unused internal entity | Desktop recovery prompt | Desktop open failed | Browser refused | `2b0d2998a24b1a4be18f0dbcdf2691804fba690482ab2805842f499e4a808dca` |
| `word/_rels/comments.xml.rels` | Unused internal entity | Desktop recovery prompt | Desktop open failed | Browser refused | `c4e7717e8763b602bd4f11c94c90bc3b767190721088b5180cc4c6c3933a7234` |
| `chunk.xhtml` | External PUBLIC, no internal subset | Opened cleanly | Opened | Not judgeable (editing refusal identical to control) | `c72659d7fc86d286be958445cc6c950c5b8dff24541670eb122d7400691631d5` |

Eleven of the closure probes had an adverse outcome on at least one consumer.
The other four were tolerated by the consumers able to judge them. Together
the original and closure series cover 40 part-role and declaration-form
combinations, 34 adverse and six tolerated.

The closure regression fixtures in `tests/it/validate.rs` are original,
repository-licensed packages generated by the shared `Doc` test builder.
Their diagram entity, diagram ATTLIST control, and XHTML altChunk external
identifier use synthetic XML derived from the observed declaration forms.

## XML-DOCTYPE — declaration-form probes, 2026-11-24

The forms series added one declaration to each matched control. All 16
controls opened cleanly on Word for Mac, Word for Windows, and Word for the
web. Windows results are from run `verdicts2-run-20261124-171011` and web
results are from the 2026-11-24 run.

| Target role | Declaration form | Word for Mac | Word for Windows | Word for the web |
| --- | --- | --- | --- | --- |
| Main document | ATTLIST | Desktop hang (timed out) | Desktop open failed | Browser refused |
| Styles | ATTLIST | Desktop recovery prompt | Desktop open failed | Browser refused |
| Styles | External SYSTEM | Desktop recovery prompt | Desktop open failed | Browser refused |
| Settings | ATTLIST | Desktop recovery prompt | Desktop open failed | Browser refused |
| Settings | External SYSTEM | Desktop recovery prompt | Desktop open failed | Browser refused |
| Styles with effects | ATTLIST | Desktop recovery prompt | Desktop open failed | View-only with corruption banner |
| Styles with effects | External SYSTEM | Desktop recovery prompt | Desktop open failed | Opened cleanly |
| Font table | ATTLIST | Desktop recovery prompt | Desktop open failed | Browser refused |
| Font table | External SYSTEM | Desktop recovery prompt | Desktop open failed | Browser refused |
| Theme | ATTLIST | Desktop recovery prompt | Desktop open failed | Browser refused |
| Theme | External SYSTEM | Desktop recovery prompt | Desktop open failed | Browser refused |
| Numbering | ATTLIST | Desktop recovery prompt | Desktop open failed | Opened cleanly |
| Numbering | External SYSTEM | Desktop recovery prompt | Desktop open failed | Opened cleanly |
| Diagram layout | ATTLIST | Opened cleanly | Opened | Opened cleanly |
| Diagram colors | ATTLIST | Opened cleanly | Opened | Opened cleanly |
| Diagram quickStyle | ATTLIST | Opened cleanly | Opened | Opened cleanly |

Thirteen probes had an adverse outcome on at least one consumer. The three
diagram ATTLIST probes were tolerated by all three. Across the three series,
56 combinations were probed, 47 adverse and 9 tolerated. The document
relationships cell in this matrix covers the internal entity form. The
historical `dtd-attlist.docx` entries above do not establish a separate
document relationships ATTLIST cell in this matrix.

All unmeasured part-role and declaration-form combinations are reported as
Warning with ByConstruction evidence as an expedient interim pending the
measurements tracked in issue #7735.

The external diagram regression fixture in `tests/it/validate.rs` is an
original, repository-licensed package generated by the shared `Doc` test
builder with synthetic XML for an unmeasured external identifier. The evidence
boundary tests in the same file use original, repository-licensed packages
generated by that builder with synthetic entity, ATTLIST and external
declarations for document and relationships roles.

## Corpus probes — media mismatches

Word for the web was used to open the drawn probes and inspect the picture on 2026-11-18. The complete JPEG displayed when declared as PNG. The signature stub and the text payload both left the drawing without a picture. Desktop Word for Mac 16.114.4 opened the drawn stub and text documents cleanly at the document level.

| Probe | SHA-256 | Web picture outcome |
| --- | --- | --- |
| `probe-media-drawn-jpeg` | `bb8014c5882d68a1bd6b1e1d6fb7b858d2d9a7bb2fc2e757ab092d8c3ec01703` | Displayed |
| `probe-media-drawn-stub` | `06a372229fa0c9a902144715412936cb2cbad053c41efe236072c525f33d574a` | Missing |
| `probe-media-drawn-text` | `944c4fc7782cbc90692c72b02e56204d7d96488c772445f1332e63e926c3c03b` | Missing |

The open-only probes `probe-mediamagic-bin` and `probe-mediamagic-text` have image relationships and no consuming drawings. The measured `probe-mediamagic-bin` archive has SHA-256 `02ad532831e4850bd75b7d9e5d3594572f5cc8b4d96022974cd176ea8b4c4942`.

## Corpus probes — duplicate font names

On 2026-11-18, Word for Mac 16.114.4 opened the `Calibri`/`CALIBRI` probe cleanly. Word for the web opened it in Editing mode without a banner, accepted an edit and saved the result. The probe `probe-fontcase` has SHA-256 `21c40c8b9be1d2968a7a467e8f5fd996da49fdea4a472f6005d0ded8132dd934`. Exact-spelling duplicates were also accepted in the measured corpus sample.

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
