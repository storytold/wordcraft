# File-format parity with Microsoft Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first version; every format Word reads or writes, measured from the readers and writers on origin/main) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4, plus Word for Windows' extra formats

Every format Word opens or saves, what WordCraft does with it, and how it's tested. Fidelity is the
share of a typical real-world file's content and formatting that survives (read: shows correctly;
write: Word opens it showing the same). Summary in [`target-app-parity.md`](target-app-parity.md);
the work items are in [`gaps.md`](gaps.md).

**Dimension: ~60% (estimated), 185–300 h to full** (the sum of the rows below). The weighted table
gives ~64%; we discount it to ~60% because DOCX fidelity has never been checked on real-world
files. DOCX dominates the weight: a Word user sends and receives .docx; everything else is
occasional.

## Formats

Weights are the share of a Word user's file traffic (estimate). Read/write: ✅ yes, 🟡 partial,
❌ no, — Word doesn't either.

| Format | Weight | Word | WordCraft read | WordCraft write | Fidelity | Hours | How it's tested | Notes |
|---|---|---|---|---|---|---|---|---|
| `.docx` Word Document (ECMA-376 Transitional) | 70% | R/W | ✅ | ✅ | ~65% | 70–110 | `crates/docx/tests` (roundtrip 28, fixtures 21, malformed 8, bidi 4, citations 5); synthetic fixtures only | Word opens our files. Untested on a real-world corpus. Details below |
| `.docx` Strict Open XML | 1% | R/W | ✅ (namespace mapping in `xml.rs`) | ❌ | ~60% | 2–4 | one fixture | Writing Strict is rare |
| `.docm` / `.dotx` / `.dotm` | 3% | R/W | ✅ | ✅ | ~65% | incl. above | `macro_packages.rs` | Macros and signatures kept on save (#172); macros never run |
| `.doc` Word 97-2003 (and `.dot`) | 8% | R/W | 🟡 | ❌ | ~55% | 25–40 | `crates/docbin/src/tests.rs` (50) | Spec-based reader ([MS-DOC]): text, formatting, styles, sections, headers/footers, tables, lists, notes, fields, bookmarks, pictures. Metafile (WMF/EMF) pictures, Word 6/95 files and encrypted files are refused or dropped. No writer |
| PDF export | 8% | W | — | ✅ | ~80% | 6–10 | `crates/pdf/src/tests.rs` | krilla: embedded subsets, selectable text incl. field results (#133), links, outline, tagging. Fonts it can't subset become outlines (#132). No PDF/A option, no "best for printing/online" choice |
| PDF open (PDF Reflow, Word for Windows) | 1% | R | ❌ | — | 0% | 30–50 | — | Converting a PDF into an editable document. A sibling app (PdfCraft) may supply the parser |
| `.rtf` Rich Text Format | 3% | R/W | 🟡 | 🟡 | ~50% | 10–15 | `crates/formats/src/tests.rs` | ~1,500 lines: text, character/paragraph formatting, styles, lists, tables, pictures (`\shppict`, #154), fields, bookmarks. Thin: comments, notes, revisions, sections, RTL |
| `.odt` OpenDocument Text | 2% | R/W | 🟡 | 🟡 | ~50% | 8–12 | `crates/formats/src/tests.rs` | Styles, lists (#87), tables, pictures, headers/footers, page breaks (#86). No footnotes, comments, tracked changes, fields or RTL |
| `.txt` Plain text | 2% | R/W | ✅ | ✅ | ~95% | 1–2 | unit tests | Encoding detection; Word's encoding choice dialog and line-ending options missing |
| `.htm/.html` Web Page and Web Page, Filtered | 1% | R/W | 🟡 | 🟡 | ~60% | 6–10 | `crates/formats/src/tests.rs` | Tables (#102), relative pictures embedded (#113), missing `</head>` tolerated (#154). No Word "Web Page" (with `_files` folder) round-trip |
| `.mht/.mhtml` Single File Web Page | <1% | R/W | ❌ | ❌ | 0% | 3–5 | — | |
| `.xml` Word XML Document (Flat OPC) | <1% | R/W | ❌ | ❌ | 0% | 3–5 | — | The DOCX parts in one XML file; cheap once the DOCX code is reused |
| `.xml` Word 2003 XML (WordprocessingML 2003) | <1% | R/W | ❌ | ❌ | 0% | 5–8 | — | Legacy |
| `.wps` Works 6–9, WordPerfect `.wpd` (Windows) | <1% | R | ❌ | — | 0% | 10–20 | — | Windows-only converters; low value |
| Encrypted/password-protected `.docx`/`.doc` | 1% | R/W | ❌ | ❌ | 0% | 6–10 | — | ECMA-376 Agile Encryption; `.doc` RC4/XOR detected and refused (`docbin` fib). Issue #55 |
| `.md` Markdown | — | (no) | ✅ | ✅ | — | — | `crates/formats/src/tests.rs` | Beyond Word |
| `.tex` LaTeX | — | — | ✅ | ✅ | — | — | `crates/formats/src/tests.rs` | Beyond Word (#11), incl. math |
| `.png` page images | — | (Save as picture, Mac) | — | ✅ | — | — | `io_ext.rs` | |
| `.json` document model | — | — | ✅ | ✅ | — | — | engine tests | For agents |

## DOCX in depth

Measured by grepping `crates/docx/src/read` and `src/write` for the OOXML elements (2026-10-10).

| Feature | Read | Write | Notes |
|---|---|---|---|
| Text, runs, paragraph and character properties, styles, numbering, sections | ✅ | ✅ | Including complex-script props (`w:rtl`, `w:cs`, `w:szCs`, `w:bCs`…), East Asian fonts kept apart from Latin (#111) |
| Tables incl. floating (`w:tblpPr`), table styles and conditional formatting | ✅ | ✅ | Custom table styles round-trip (#256) |
| Headers/footers (first, even/odd), page borders, line numbers, gutter, mirror margins | ✅ | ✅ | |
| Footnotes, endnotes, comments | ✅ | ✅ | `commentsExtended` written; `commentsIds`, modern threaded comments (`w16cex`) partly |
| Tracked insertions and deletions (`w:ins`/`w:del`), paragraph-mark revisions | ✅ | ✅ | #125, #244 |
| Formatting revisions (`w:rPrChange`, `w:pPrChange`, `w:sectPrChange`, `w:tblPrChange`) | ❌ | ❌ | Lost on open. Issue #41 |
| Move tracking (`w:moveFrom`/`w:moveTo`) | 🟡 | ❌ | Read as plain insert/delete |
| Fields (`w:fldChar`, `w:fldSimple`), TOC, cross-references, `ADDIN` citations | ✅ | ✅ | Field codes Word supports but we don't evaluate keep their cached result |
| Content controls (`w:sdt`) | 🟡 | ❌ | Content kept, the control (type, binding, placeholder, lock) dropped |
| Legacy form fields (`w:ffData`), check boxes, drop-downs | ❌ | ❌ | |
| DrawingML pictures inline and anchored (`wp:anchor`), wrap square/tight/through/top-bottom | ✅ | ✅ | Tight/through read and written, laid out as square |
| DrawingML shapes and text boxes (`wps:`) | ✅ | ✅ | Preset geometries subset |
| Freeform shapes (`a:custGeom`: `a:moveTo`, `a:lnTo`, Bézier curves flattened) and WordCraft ink | ✅ | ✅ | Ink is written as a freeform `wps:wsp` in a `wp:anchor` (round caps, alpha for highlighter) and recognised again by its drawing name (#307). It is saved as a custom-geometry shape, not as Word's own ink (InkML in `w14:contentPart`), so Word shows WordCraft ink as a freeform shape it can move and recolour but not erase with its ink eraser; arcs drawn straight |
| Group shapes (`wpg:`), drawing canvas | ❌ | ❌ | Dropped |
| VML (`w:pict`, `v:shape`, `v:textbox`) | 🟡 | ❌ | Pictures and text boxes, best effort (#242) |
| Charts (`c:chart`), SmartArt (`dgm`), Word ink (`w14:contentPart`, InkML), 3D models, OLE objects (`w:object`) | ❌ | ❌ | Dropped on read, not preserved on save. A document with a chart loses it silently |
| Equations (OMML `m:oMath`) | ✅ | ✅ | #191 |
| Themes, font table, settings, compatibility mode | ✅ | ✅ | Embedded fonts (`w:embedRegular`) not read |
| Custom XML parts, document properties, bibliography sources | 🟡 | 🟡 | Custom properties round-trip; bibliography sources pending (#169) |
| Ruby, `w:eastAsianLayout`, `w:fitText`, document grid (`w:docGrid`) | ❌ | ❌ | East Asian layout |
| Glossary document (building blocks), `w:altChunk`, sub-documents | ❌ | ❌ | |
| Hostile input | ✅ | — | `malformed.rs`, capped allocations, never-crash standard |

## What decides beta for formats

1. A **real-world DOCX corpus** opened, rendered and round-tripped, compared page by page with
   Word (local only: Word output can't be committed; keep a manifest and our own renders).
2. **Preserve what we can't render**: charts, SmartArt, OLE, ink and content controls survive a
   round trip (keep the parts and the run), and charts/SmartArt show their fallback picture.
3. Formatting revisions and content controls read and written.
4. Encrypted `.docx` opens with a password.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | trivial | Freeform shapes and ink as DrawingML custom geometry, read and written (#307) |
| 2026-10-10 | trivial | Custom table styles round-trip (#256 merged) |
| 2026-10-10 | major | First version: Word's full format list, read/write status, DOCX element coverage from the source, fidelity and hours |
