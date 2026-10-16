# File-format parity with Microsoft Word

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-16 · **Change:** trivial (Markdown source view: a `.md` file edited as its text and saved back unchanged; previously trivial: every table style part round-trips, #714); previously trivial: VML freeforms read and kept on save; previously trivial: multi-paragraph field instructions no longer add blank lines; previously trivial: `flowChartOr` draws its circle and cross spokes, as in Word; previously trivial: RTF: Mac OS Roman and DOS text, #1653; previously trivial: VML text boxes keep their internal margins, fit to their text and vertical alignment; hidden drawings not shown; previously trivial: PDF export draws horizontal and vertical lines inside groups, #1474; previously trivial: symbol-font text exported as Unicode, #776; previously trivial: text-box margins, Resize shape to fit text and freeforms in groups, #743; previously trivial: formatted paste from the system clipboard's HTML and RTF, #687; previously trivial: independent complex-script formatting inheritance and round-trip regression tests; previously trivial: import/export fixes for TXT, RTF, ODT, HTML and LaTeX, #433–#440; previously major: first version; every format Word reads or writes, measured from the readers and writers on origin/main) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4, plus Word for Windows' extra formats

Every format Word opens or saves, what WordCraft does with it, and how it's tested. Fidelity is the
share of a typical real-world file's content and formatting that survives (read: shows correctly;
write: Word opens it showing the same). Summary in [`target-app-parity.md`](target-app-parity.md);
the work items are in [`gaps.md`](gaps.md).

**Dimension: ~60% (estimated), 179–290 h to full** (the sum of the rows below). The weighted table
gives ~64%; we discount it to ~60% because DOCX fidelity has never been checked on real-world
files. DOCX dominates the weight: a Word user sends and receives .docx; everything else is
occasional.

## Legacy DOC/DOT document properties

The binary Word reader imports SummaryInformation text properties: title, subject, author,
keywords, description, last editor and revision. Code-page strings use their declared
Windows single-byte encoding, UTF-8 or UTF-16LE; UnicodeString lengths are counted in UTF-16
units. Metadata streams are capped at 2 MiB and individual values at 64 KiB. Invalid optional
metadata is skipped without losing the document body. Creation/modification timestamps and
DocumentSummaryInformation/custom properties are not imported by this change; DOC/DOT
remain read-only formats and encrypted legacy documents remain unsupported.

Tests create OLE property sets in Rust and exercise the public document reader for normal
DOCs, templates, accented/Cyrillic/Unicode properties and malformed lengths/offsets. A local
LibreOffice-generated DOC and DOT containing an original synthetic document reproduced the
missing-title bug before this change and retain their title after it.

## Formats

Weights are the share of a Word user's file traffic (estimate). Read/write: ✅ yes, 🟡 partial,
❌ no, — Word doesn't either.

| Format | Weight | Word | WordCraft read | WordCraft write | Fidelity | Hours | How it's tested | Notes |
|---|---|---|---|---|---|---|---|---|
| `.docx` page background picture/texture (`v:background/v:fill`) | — | R/W | ✅ | ✅ | ~80% | — | `roundtrip.rs::page_background_picture_round_trips`, `crates/render` | Image part and `type`/`o:title` kept; tiled at 96 dpi or stretched; drawn on screen and in PDF; no Fill Effects dialog yet, gradients and patterns are not read |
| `.docx` Word Document (ECMA-376 Transitional) | 70% | R/W | ✅ | ✅ | ~65% | 70–110 | `crates/docx/tests` (roundtrip 34, fixtures 29, malformed 8, bidi 5, citations 5); synthetic fixtures only | Word opens our files. Untested on a real-world corpus. Tracked changes inside a field result stay revisions (the field is kept as a range, #1037). Details below |
| `.docx` Strict Open XML | 1% | R/W | ✅ (namespace mapping in `xml.rs`) | ❌ | ~60% | 2–4 | one fixture | Writing Strict is rare |
| `.docm` / `.dotx` / `.dotm` | 3% | R/W | ✅ | ✅ | ~65% | incl. above | `macro_packages.rs` | Macros and signatures kept on save (#172); macros never run |
| `.doc` Word 97-2003 (and `.dot`) | 8% | R/W | 🟡 | ❌ | ~60% | 25–40 | `crates/docbin/src/tests.rs`, `tests/drawing.rs` | Spec-based reader ([MS-DOC]): text, formatting, styles, sections, headers/footers, tables, lists, notes, fields, bookmarks; inline and floating pictures (PNG, JPEG, DIB, TIFF, EMF, WMF; Macintosh PICT as a sized placeholder) with crop, alt text, position and wrap; basic shapes, lines, groups and text boxes ([MS-ODRAW]). Table cell shading from every SHD record (`sprmTDefTableShd`, `sprmTDefTableShdRaw` and their 2nd/3rd ranges, with the Word 97 `Shd80` as fallback; a clear pattern shows its background colour); the row-end mark is not a cell, and each table has its own grid and properties (#1380). Horizontally merged rows stored as fewer, wider cells come with #1123. Freeform shapes, z-order and Word 6/95 files are dropped or refused. Password-protected files (XOR obfuscation, RC4, RC4 CryptoAPI) open after a password prompt, from the public specs; not yet tried on a file Word wrote (AES-encrypted ones are refused). No writer. DIB conversion caps BMP bytes (header included) and encoded PNG at the supplied BLIP byte limit, and uses it as the decoder's allocation limit; a BMP fallback is kept if it fits (related #1334 and #1554). |
| PDF export | 8% | W | — | ✅ | ~80% | 6–10 | `crates/pdf/src/tests.rs` | krilla: embedded subsets, selectable text incl. field results (#133); Hebrew and Arabic copy and search as whole words (right-to-left glyphs written in visual order, #1383); All Caps and Small Caps text extracts as the capitals shown, as from Word's PDFs, and never changes the case of other text (#1473), links, outline, tagging. Fonts it can't subset become outlines (#132). Print Markup (#589): comments in balloons beside each page (scaled to its paper) and tracked changes; HTML/Markdown/text copies end with a comment list. Horizontal and vertical lines inside groups are drawn (#1474). No PDF/A option, no "best for printing/online" choice |
| PDF open (PDF Reflow, Word for Windows) | 1% | R | ❌ | — | 0% | 30–50 | — | Converting a PDF into an editable document. A sibling app (PdfCraft) may supply the parser |
| `.rtf` Rich Text Format | 3% | R/W | 🟡 | 🟡 | ~50% | 10–15 | `crates/formats/src/tests.rs` | ~1,500 lines: text, character/paragraph formatting, styles, lists, tables, pictures (`\shppict`, #154), fields, bookmarks. 8-bit text in the document's code page (`\ansicpg`) or the current font's (`\fcharset`, `\cpg`): Cyrillic, Central European, Greek, Turkish, Hebrew, Arabic, Baltic, Vietnamese, Thai (was Windows-1252 only), Mac OS Roman (`\mac`, `\fcharset77`) and DOS code pages (`\pc` 437, `\pca` 850, `\cpg` 437/737/850/852/857/866, `\fcharset254`) (#1653). Double-byte East Asian code pages (Japanese `\fcharset128`, Chinese, Korean) not decoded yet. Pasted from the system clipboard where the platform offers it (Windows, Wayland) when there is no HTML (#687). Tracked insertions and deletions read and written (`\revtbl`, `\revised`/`\deleted`, authors, `\revdttm` dates), so Accept/Reject All work after opening or a round trip; paragraph-mark and formatting revisions not yet. Sections on import (#1378): each `\sect` ends a section with its own page size and orientation, margins, header/footer distance, columns, break type, page-number restart, start and format, title page and headers/footers (those it leaves out link to the previous); export still writes one section. Thin: comments, notes, section export, RTL |
| `.odt` OpenDocument Text | 2% | R/W | 🟡 | 🟡 | ~50% | 8–12 | `crates/formats/src/tests.rs`, `crates/formats/src/odt.rs` unit tests, `crates/engine/tests/odt_sections.rs` | Styles, lists (#87), tables, pictures, headers/footers, page breaks (#86), page size, orientation and margins (#810), sections: one per master-page change on a paragraph or table (next page, with its own page size, orientation, margins, columns and page-numbering restart) and per columned `text:section` (continuous, with its columns and separator line) (#1379); tracked insertions and deletions with author and date, inserted and deleted paragraph marks and text inserted then deleted included (ODF change tracking; format changes read as none; pictures and tables inside a tracked deletion are dropped). No footnotes, comments, fields or RTL |
| `.txt` Plain text | 2% | R/W | ✅ | ✅ | ~95% | 1–2 | unit tests | Encoding detection. Files with other extensions (`.m3u`/`.m3u8` playlists, logs, config files, no extension) open as text when they are text, Word and RTF files by their signature; binary files are refused (#569). A Word or RTF extension that contradicts the content also opens by signature, as in Word: RTF saved as `.doc` (hh.ru résumés, accounting exports), a `.docx` renamed `.doc`, a Word 97-2003 file renamed `.docx`. Word's encoding choice dialog and line-ending options missing; such files save only through Save As |
| `.htm/.html` Web Page and Web Page, Filtered | 1% | R/W | 🟡 | 🟡 | ~60% | 6–10 | `crates/formats/src/tests.rs` | Tables (#102), relative pictures embedded (#113), missing `</head>` tolerated (#154). Header cells (`<th>`) bold unless their CSS says otherwise; `text-align` on a cell, or inherited from its row, row group or table, aligns the cell's paragraphs (#1381). HTML gives no page size, so it opens on the region's paper (A4 or Letter), as Markdown and plain text do; `@page` isn't read (#1381). No Word "Web Page" (with `_files` folder) round-trip. The same reader takes HTML from the system clipboard: Paste keeps another program's formatting (a Word document, a web page; Windows' "HTML Format" header understood), Merge Formatting keeps its emphasis (#687). Equations are written as MathML (with the linear format as a UnicodeMath annotation) and read back from MathML: our annotation, else the presentation elements, else a TeX annotation (#816) |
| `.mht/.mhtml` Single File Web Page | <1% | R/W | ❌ | ❌ | 0% | 3–5 | — | |
| `.xml` Word XML Document (Flat OPC) | <1% | R/W | ✅ | ✅ | ~65% | — | `crates/docx/tests/word_xml.rs`, `roundtrip.rs` (`flat_opc_round_trip_equals_docx`) | The DOCX package in one XML file, read and written by the DOCX code (#548); File › Export › Word XML Document. `.xml` files are told apart by their root element |
| `.xml` Word 2003 XML (WordprocessingML 2003) | <1% | R/W | 🟡 | 🟡 | ~55% | 1–2 | `crates/docx/tests/word_xml.rs` | Opened by rewriting it as a package for the DOCX reader (#548): styles, default fonts, lists, tables, sections, headers/footers, footnotes/endnotes, comments, bookmarks, tracked insertions/deletions, hyperlinks, `w:binData` pictures, document properties. Not read: formatting changes, `w:docPr` settings, smart tags. Written (#1081) by rewriting the DOCX writer's package into the 2003 vocabulary (`crates/docx/src/wordml2003_write.rs`, `crates/docx/tests/wordml2003_write.rs`): core properties, styles (document defaults folded into Normal), lists, text and paragraph/character formatting, tables, hyperlinks, footnotes/endnotes inline, headers/footers in the section properties, bookmarks, comments and tracked insertions/deletions as `aml:annotation`, section page setup, and PNG/JPEG/GIF/BMP/EMF/WMF pictures as `w:pict` with `w:binData`. **Dropped on write:** shapes, text boxes, groups, charts, SmartArt, ink, equations, OLE objects, picture crop, rotation and floating position (pictures become inline), content-control wrappers (their content stays), formatting-change tracking, theme colours/fonts, custom XML, macros, even/odd header setting. File › Export › Word 2003 XML Document, `file.save`/`file.saveAs` with `format: "wordml2003"`, `wordcraft-cli convert … --format wordml2003`; a plain save over a 2003 file keeps that format. Never opened in Word yet (written from the published schemas; needs a tester) |
| `.wps` Works 6–9, WordPerfect `.wpd` (Windows) | <1% | R | ❌ | — | 0% | 10–20 | — | Windows-only converters; low value |
| Encrypted/password-protected `.docx`/`.doc` | 1% | R/W | ❌ | ❌ | 0% | 6–10 | — | ECMA-376 Agile Encryption; `.doc` XOR, RC4 and RC4 CryptoAPI open with a password (`docbin` `crypt.rs`, #1079; synthetic tests only, untested against Word-written files; AES `.doc` refused). Issue #55 |
| `.xlsx`/`.xlsm`/`.xls`/`.ods` as a mail-merge recipient list | — | R (data source) | ✅ | — | ~90% | 1–3 | `crates/formats/src/sheet.rs`, `cmd/mailings.rs` | Read-only, one sheet's cell values (#334): shared/inline/rich strings, numbers, booleans, cached formula results, dates and times as ISO 8601 (`yyyy-mm-dd`) from the cell's number format, sparse cells; ODS value types, repeated rows/columns, covered cells. Select Table picks the sheet. Capped (64 MiB per part, 100,000 rows, 1,000 columns, 2 M cells, 32,767 characters a cell). Legacy `.xls` (Excel 97–2003 BIFF8, #778): SST/LABELSST, LABEL, NUMBER, RK/MULRK, BOOLERR and cached FORMULA results with the same date handling; Excel 5.0/95 and password-protected `.xls` are refused with a message; a CSV/TSV named `.xls` reads as text. Text lists (.csv/.tsv/.txt) decode UTF-8/UTF-16 by BOM or shape, else the detected legacy encoding (GBK/GB18030, Big5, Shift_JIS, EUC-KR, Windows code pages; `encoding` param overrides) (#778). Not shown: number formats other than dates (currency, percent, thousands separators); no Access or Outlook sources |
| Encrypted/password-protected `.docx` | 1% | R/W | ✅ | ✅ | ~60% | 2–4 | `crates/docx/src/crypt.rs` (6 unit tests), `crates/engine/src/tests.rs::password_protected_documents_round_trip` | ECMA-376 / [MS-OFFCRYPTO] agile encryption (AES-256-CBC, SHA-512): a protected `.docx` opens with its password and saves encrypted (#55, #281). Synthetic files only; never checked against Word-made protected files |
| Encrypted/password-protected `.doc` (RC4/XOR) | <1% | R | ❌ | — | 0% | 4–6 | `crates/docbin/src/tests.rs::encrypted_rejected` | Detected and refused (`DocbinError::Encrypted`, `docbin` fib). Not decrypted |
| `.md` Markdown | — | (no) | ✅ | ✅ | — | — | `crates/formats/src/tests.rs`, `crates/engine/src/cmd/markdown.rs` | Beyond Word. View › Markdown Source (`view.markdownSource`) edits the file's own text, one line per paragraph, and saves it back byte for byte (line feeds); switching back reads the text again. Saved as another format, a document in source view saves the document its text describes |
| `.tex` LaTeX | — | — | ✅ | ✅ | — | — | `crates/formats/src/tests.rs` | Beyond Word (#11), incl. math |
| Pictures for Insert › Picture, drag-and-drop and paste | — | R | ✅ | — | — | — | `insert_picture_takes_a_windows_metafile` (engine), `paste_picture.rs` | PNG, JPEG, GIF, WebP, BMP, and Windows Metafile `.wmf` / Enhanced Metafile `.emf` at their own frame size (#802), drawn by `crates/metafile`. Not yet: SVG, TIFF, ICO, HEIC |
| `.png` page images | — | (Save as picture, Mac) | — | ✅ | — | — | `io_ext.rs` | |
| `.json` document model | — | — | ✅ | ✅ | — | — | engine tests | For agents |

## DOCX in depth

Measured by grepping `crates/docx/src/read` and `src/write` for the OOXML elements (2026-10-10).

| Feature | Read | Write | Notes |
|---|---|---|---|
| Text, runs, paragraph and character properties, styles, numbering, sections | ✅ | ✅ | Including complex-script props (`w:rtl`, `w:cs`, `w:szCs`, `w:bCs`…), East Asian fonts kept apart from Latin (#111); independent complex-script formatting inherited through defaults/styles and preserved over repeated saves (`complex_script_inheritance.rs`), indents in characters (`w:firstLineChars`, `w:hangingChars`, `w:leftChars`, `w:rightChars`, #534); every ST_NumberFormat value of `w:numFmt` is kept on save, Chinese/Japanese/Korean, enclosed and full-width formats are drawn, the rest (Hebrew, Arabic, Hindi, Thai…) show decimal (#534). `w:beforeLines`/`w:afterLines` not kept (the points are) |
| Tables incl. floating (`w:tblpPr`), table styles and conditional formatting | ✅ | ✅ | Custom table styles round-trip (#256), every `w:tblStylePr` part (wholeTable, firstRow, lastRow, firstCol, lastCol, band1/2Vert, band1/2Horz, ne/nw/se/swCell) with run formatting, `w:jc`, cell borders, shading and `w:vAlign` (#714); a cell's own `w:vAlign w:val="top"` is kept and beats the style's (#1288); right-to-left tables (`w:bidiVisual`) and sections (`w:bidi`) round-trip (#362) |
| Text, runs, paragraph and character properties, styles, numbering, sections | ✅ | ✅ | Including a Styles gallery for files that mark no style `w:qFormat` (LibreOffice: Word's usual quick styles are added, #1238), complex-script props (`w:rtl`, `w:cs`, `w:szCs`, `w:bCs`…), East Asian fonts kept apart from Latin (#111); independent complex-script formatting inherited through defaults/styles and preserved over repeated saves (`complex_script_inheritance.rs`), indents in characters (`w:firstLineChars`, `w:hangingChars`, `w:leftChars`, `w:rightChars`, #534); every ST_NumberFormat value of `w:numFmt` is kept on save, Chinese/Japanese/Korean, enclosed and full-width formats are drawn, the rest (Hebrew, Arabic, Hindi, Thai…) show decimal (#534). `w:beforeLines`/`w:afterLines` not kept (the points are) |
| Tables incl. floating (`w:tblpPr`), table styles and conditional formatting | ✅ | ✅ | Row table border exceptions (`w:tblPrEx/w:tblBorders`) round-trip (`row_table_borders_round_trip`); other row table exceptions are not preserved. Custom table styles round-trip (#256); right-to-left tables (`w:bidiVisual`) and sections (`w:bidi`) round-trip (#362) |
| Headers/footers (first, even/odd), page borders, line numbers, gutter, mirror margins | ✅ | ✅ | |
| Hyphenation settings (`w:autoHyphenation`, `w:hyphenationZone`, `w:doNotHyphenateCaps`, `w:consecutiveHyphenLimit`) | ✅ | ✅ | Zone, capitals and limit kept and honoured by layout (#407) |
| Footnotes, endnotes, comments | ✅ | ✅ | The document's own note separators and continuation notice (`w:type` `separator`, `continuationSeparator`, `continuationNotice`) round-trip as ids -1, 0 and 1 (#352); note options (`w:pos`, `w:numFmt`, `w:numStart`, `w:numRestart`) round-trip in the settings and in each `w:sectPr` (#385); `commentsExtended` written; `commentsIds` (durable ids) and `commentsExtensible` (UTC dates) read and regenerated consistently with the written paragraph ids; `people.xml` carried verbatim |
| Tracked insertions and deletions (`w:ins`/`w:del`), paragraph-mark revisions | ✅ | ✅ | #125, #244 |
| Formatting revisions (`w:rPrChange`, `w:pPrChange`, `w:sectPrChange`, `w:tblPrChange`, `w:trPrChange`, `w:tcPrChange`, `w:numberingChange`) | ✅ | ✅ | #41: kept on open and save, schema order (change element last); changes inside `styles.xml` are dropped. RTF/ODT ignore them |
| Fields (`w:fldChar`, `w:fldSimple`), TOC, cross-references, `ADDIN` citations | ✅ | ✅ | Field codes Word supports but we don't evaluate keep their cached result. Instruction-only paragraphs carry hidden marks instead of blank lines; an empty result paragraph keeps its line. Imported TOCs preserve actual result boundaries, formatted entries and nested fields without relying on TOC styles. `EQ \o` (overstrike, enclosed characters) is drawn; enclosed characters are written as Word stores them, their style in the code's run sizes (#297); a cached picture (`INCLUDEPICTURE`) stays as the content of a range field (#465) |
| Move tracking (`w:moveFrom`/`w:moveTo`, `w:moveFromRangeStart/End`, `w:moveToRangeStart/End`), moved paragraph marks | ✅ | ✅ | #400: both ends linked by the range name; ranges rewritten around each moved stretch (ids renumbered); a move missing an end or a range end opens as a plain deletion/insertion. ODT/PDF export the result (moved-away text hidden like deletions); RTF writes a move as a plain tracked deletion and insertion |
| Content controls (`w:sdt`) | ✅ | ✅ | Block, inline, row and cell level, nested; title, tag, id, lock, placeholder, temporary, rich/plain text, `w14:checkbox`, combo box, drop-down list, date, picture, building block gallery, `w15` repeating sections; other `w:sdtPr` children (data binding, appearance) kept verbatim (#381). A TOC's control is rebuilt by the TOC. Replace All respects content-edit and control-deletion locks and refuses a batch before making any edit |
| Restrict Editing (`w:documentProtection` edit/formatting/enforcement and password hash, `w:locked` styles, `w:permStart`/`w:permEnd` exceptions) | ✅ | ✅ | #412: SHA-512 hash with salt and spin count written; Word's legacy `w:cryptAlgorithmSid`/`w:hash` form (SHA-1/-256/-384/-512, ECMA-376 Part 4 §14.8.1) is kept and verified; MD2/MD4/MD5, `w:algIdExt` and other unsupported forms are refused, never unlocked; `w:colFirst`/`w:colLast` table-column ranges read as plain ranges; forms mode (`w:edit="forms"`) lets edits inside fillable content controls (`w:lock` content-editable, innermost wins) run, everything else rolls back; row-wrapped controls and control deletion remain refused; `review.restrict` reports `formFields`; Find Next Region covers form fields (the pane button shows for forms documents); under active protection, `developer.properties` refuses `lockEdit`/`lockDelete` changes — as Word blocks property edits until Stop Protection — while other properties (title, tag, placeholder…) stay editable |
| Legacy form fields (`w:ffData`), check boxes, drop-downs | ❌ | ❌ | |
| DrawingML pictures inline and anchored (`wp:anchor`), wrap square/tight/through/top-bottom | ✅ | ✅ | Tight/through read and written with their `wp:wrapPolygon` (and `edited`); without one, a shape's outline is written (a picture's rectangle). `wrapText` (both sides, left, right, largest) read and written for square, tight and through |
| WordArt text effects (`w14:textFill`, `w14:textOutline`, `w14:shadow`, `w14:glow`, `w14:reflection` in `w:rPr`) | ✅ | ✅ | Solid, gradient (linear) and no fill, outline colour/width, shadow, glow, reflection (#416); `w14:scene3d`/`w14:props3d` dropped |
| DrawingML shapes and text boxes (`wps:`) | ✅ | ✅ | Preset geometries subset (callouts and flowchart boxes drawn as the nearest shape, #611); `flowChartOr` draws a circle with horizontal and vertical spokes; rounded rectangles keep their own corner (`roundRect` `adj`: read, written back, drawn on screen and in PDF, #1476); fill and outline from the shape or from the theme through its style references (`a:fillRef`/`a:lnRef`, #611), scheme, preset, system, scRGB and HSL colours; rotation and flips (`a:xfrm` `rot`/`flipH`/`flipV`, pictures and groups too, #332), the effect extent covering the rotated bounds; `wps:bodyPr` `vert` and `anchor`, internal margins (`lIns`/`tIns`/`rIns`/`bIns`) and Resize shape to fit text (`a:spAutoFit`: the text is never cut, #743), linked text boxes (`wps:txbx/@id`, `wps:linkedTxbx` `id`/`seq`, #369); text warps (`a:prstTxWarp` with adjust values, all 41 presets kept; unknown ones dropped), connectors (`straightConnector1`, `bentConnector2`–`5`, `curvedConnector2`–`5` read; 3-segment forms written) with glued ends (`wps:cNvCnPr` `a:stCxn`/`a:endCxn`; the glued shapes keep their drawing ids) and arrowheads (`a:headEnd`/`a:tailEnd`, drawn as triangles), including custom-geometry paths (#416). Hidden drawings (`wp:docPr/@hidden`, a group member's `cNvPr/@hidden`) are kept and saved but neither drawn nor laid out, nor exported to other formats, as in Word for Windows; `.doc` and RTF shapes don't read their hidden flag yet |
| Freeform shapes (`a:custGeom`: `a:moveTo`, `a:lnTo`, Bézier curves flattened) and WordCraft ink | ✅ | ✅ | Ink is written as a freeform `wps:wsp` in a `wp:anchor` (round caps, alpha for highlighter) and recognised again by its drawing name (#307). It is saved as a custom-geometry shape, not as Word's own ink (InkML in `w14:contentPart`), so Word shows WordCraft ink as a freeform shape it can move and recolour but not erase with its ink eraser; arcs drawn straight |
| Group shapes (`wpg:`), drawing canvas (`wpc:`) | ✅ | ✅ | Groups (#267) and canvases (#344) with pictures, shapes, freeforms (custom geometry, #743; a path drawn back to its start without `a:close` is filled, as in Word) and text boxes; nested groups flattened. The canvas schema has no transform, so a turned or flipped canvas keeps its turn in our own `wpc:extLst` extension (`urn:wordcraft:canvas-xfrm`; Word shows it unturned). Legacy VML canvases (`v:group editas="canvas"`) aren't read as canvases |
| VML (`w:pict`, `v:shape`, `v:textbox`) | 🟡 | 🟡 | Pictures, text boxes, lines (with arrowheads), rectangles and ovals, best effort (#242, #611). Preset shapes (`v:shape` whose `o:spt`, own or from its `v:shapetype`, or Word's shapetype id `_x0000_tNN`, names a preset; [MS-ODRAW] §2.4.24 numbers) are drawn and saved like the DrawingML preset of the same shape: exactly for rectangle, rounded rectangle, ellipse, diamond, isosceles triangle, five-point star, right arrow, line and straight connector, heart, text box, and the flowchart process, decision and connector; the others as their nearest shape, as for DrawingML (left/up/down arrows as the arrow, right triangle as the triangle, callouts as their box). WordArt text shapes and picture frames without a picture aren't drawn. Fill, outline and size come from the shape, else its shapetype; text inside keeps the shape around it (#1377). Adjust values (`adj`) aren't read: presets use WordCraft's proportions. Text boxes keep their internal margins (`inset`, unless `o:insetmode="auto"`), Resize shape to fit text (`mso-fit-shape-to-text`) and vertical alignment (`v-text-anchor`). Hidden shapes (`visibility:hidden`, like DrawingML's `wp:docPr/@hidden`) are kept and saved but neither drawn nor laid out, as in Word for Windows (Word for Mac draws them). Not read: vertical text (`layout-flow`) Word's text watermark (a `v:textpath` shape in a header or footer, recognised by Word's `PowerPlusWaterMarkObject` id or by being centred on the page both ways) becomes the document's watermark: text, font, fill colour, semitransparent (`v:fill` `opacity`) and diagonal (any turn); drawn with WordCraft's own size and angle, on every page even when only some sections' headers (or only the first-page header) have it (#1290). Saving writes the watermark back the same way: a `v:textpath` shape centred on the margins, behind the text, at the start of every header's first paragraph; a section whose pages would show no header gets a header holding only the watermark (default, and first-page or even-page ones when those are on), while sections linked to an earlier header stay linked. Freeforms (`v:shape` with its own `path`: moves, lines, Béziers and closes in its `coordsize`/`coordorigin` space, `nf`/`ns`, `v:stroke` opacity) are read as freeforms and saved as DrawingML custom geometry; fill opacity: none at 0, otherwise opaque (as for DrawingML fills); arcs are drawn straight; shapes whose path uses formulas (`@n`) and paths inherited from a `v:shapetype` aren't read. Other VML is written only for OLE object pictures. Other VML WordArt (`v:textpath` in the body, or a header logo) is still dropped |
| Alt text of drawings (`wp:docPr`/`cNvPr` `descr` and `title`, decorative mark `adec:decorative` from [MS-ODRAWXML]) and tables (`w:tblCaption`, `w:tblDescription`) | ✅ | ✅ | Pictures, shapes, text boxes, groups and their members, charts and SmartArt; OLE objects get theirs written into the kept XML (#386) |
| Charts (`c:chart`), SmartArt (`dgm`) | ✅ | ✅ | Drawn from the chart's cached data and SmartArt's stored drawing (#292), with data labels (`c:showVal`); a chart of only types not drawn yet (radar, stock, bubble, surface, 3-D, pie of pie) shows a placeholder in its frame with its title and type (#998). Charts made in WordCraft (Insert › Chart, #409) are written as a `c:chartSpace` part with their data as literals (`c:strLit`/`c:numLit`, no embedded workbook, so Word shows them and offers to create a workbook on Edit Data) and read back as editable; a chart part holding anything more (any chart from Word, or one re-saved by Word) stays read-only. Saved back as read, with every part they reach (chart, embedded workbook, colours and style; diagram data, layout, quick style, colours and drawing) and their content types; a moved or resized one keeps its new frame, and a pasted copy gets its own parts (#319). SmartArt made in WordCraft (#497) is written from its model: data (`dgm:dataModel` points and connections), our own layout, quick-style and colour definitions under the built-in layout identifiers, and a `dsp:drawing` with every shape placed; it reads back editable only when its data part is exactly what WordCraft writes |
| OLE objects (`w:object`: embedded or linked files, ActiveX) | 🟡 | ✅ | Shown as their picture (VML or DrawingML); saved back whole with the embedding and picture parts, at the size, rotation and flips set in WordCraft (#319, #332). Can't be opened or edited |
| Native Word ink (`w14:contentPart`, InkML) | 🟡 | 🟡 | Drawing-level ink kept opaque with its InkML part, relationship and content type; direct run-level `w:contentPart` and run-level `mc:AlternateContent` keep the native branch and VML picture fallback. Direct content parts remain direct through repeated save/reopen cycles, without a native rendering. Drawing-level ink shows the preview or a missing-picture box. No native stroke rendering, eraser, conversion or replay; ink nested in groups/canvases remains unsupported |
| 3D models | ❌ | ❌ | Dropped on read, not preserved on save |
| Equations (OMML `m:oMath`) | ✅ | ✅ | #191; manual breaks (`m:brk`) and the settings' `m:mathPr` wrapping options (#326) |
| Themes, font table, settings, compatibility mode | ✅ | ✅ | `w:allowSpaceOfSameStyleInTable` read and written in schema order; absent/false default off, enabled flag retained in modes 11/12/14/15. Embedded fonts (`w:embedRegular`) not used for rendering, but the font table, its relationships and the embedded font parts are written back unchanged (#770); attached template (`w:attachedTemplate` and its external relationship) and `w:linkStyles` round-trip (#377). `w:removePersonalInformation` and `w:removeDateAndTime` round-trip and are honoured on save (#1062): comment and revision authors written as "Author" (initials "A"), no `dc:creator`/`cp:lastModifiedBy`, no annotation dates; the open document keeps its values (`file.privacyOptions` sets them, `file.privacy` reads them; File › Options › Save › Privacy for this document). The template path comes from the file, so opening never reads it: styles update from it only on an explicit Update Styles. Settings the model doesn't hold (other `w:compatSetting`s, rsids, document variables, proofing state, colour scheme mapping…) survive a save: the writer merges into the file's own `settings.xml` ([below](#settings-preservation)); `w:latentStyles`, `w:default` on localized default styles and lists' `w:nsid`/`w:tmpl`/`w:multiLevelType` round-trip |
| Custom XML parts, document properties, bibliography sources | 🟡 | 🟡 | Custom XML stores reached from the package/main document preserve opaque data, properties and relationships; binding evaluation remains absent. Custom properties round-trip; bibliography model I/O remains separate (#532 open) |
| Section text direction (`w:textDirection` in `w:sectPr`, §17.6.20) | ✅ | ✅ | `tbRl`, `btLr` (and `tbRlV`/`tbLrV`, read as `tbRl`); `lrTbV` and unknown values read as horizontal (#491) |
| Section vertical alignment (`w:vAlign` in `w:sectPr`, §17.6.23) | ✅ | ✅ | `top`, `center`, `bottom` and `both` (Justified, read as Center before #1320); unknown values read as top, top writes nothing (`crates/docx/tests/section_valign.rs`). ODT/RTF don't carry it |
| Ruby (`w:ruby`: `w:rubyPr`, `w:rt`, `w:rubyBase`) | ✅ | ✅ | #288; base and ruby text with their run formatting, alignment, sizes, raise, language |
| Document grid (`w:docGrid`: type, line pitch, `w:charSpace`), paragraph `w:snapToGrid` | ✅ | ✅ | #391; run-level `w:snapToGrid` not read |
| `w:eastAsianLayout`, `w:fitText` | ❌ | ❌ | East Asian layout |
| Glossary document (building blocks), `w:altChunk`, sub-documents | ❌ | ❌ | |
| Hostile input | ✅ | — | `malformed.rs`, capped allocations, never-crash standard |
| Document paragraph defaults (`w:pPrDefault`) | ✅ | ✅ | Omission and explicit empty defaults remain distinct across repeated saves; nonempty defaults and later default-spacing edits are written |

## Opening files from the system

Double-clicking a document, Open With, and making WordCraft the default app (#295). The installers
register WordCraft for these types but never take over a default; File › Options › General has
**Make WordCraft the default for Word documents…** where the system allows it.

| Platform | Registered types | Default app | Notes |
|---|---|---|---|
| Windows (MSI) | `.docx` `.docm` `.dotx` `.dotm` `.doc` `.dot` `.rtf` `.odt`; `.md` `.txt` under Open With only | Options button opens Settings › Default apps on WordCraft's page | One ProgId per type, `OpenWithProgids`, Capabilities + `RegisteredApplications` (`packaging/windows/wordcraft.wxs`). Not yet tried on a real install |
| Linux, BSD (deb, rpm, Flatpak, AppImage) | Word, ODT, RTF, Markdown, LaTeX, HTML, text (`MimeType=` in the desktop entry, AppStream `mediatype`) | Options button runs `xdg-mime default` for the Word, RTF and ODT types | Not from inside Flatpak (the sandbox can't change host defaults); a bare AppImage needs desktop integration first |
| macOS | `CFBundleDocumentTypes` as Alternate handler (Word, ODT, RTF, Markdown, HTML, text) | Finder › Get Info › Open with › Change All (the Options page says so) | Apple events open the files (#27) |

## Paragraph-default preservation
The styles reader records whether `w:pPrDefault` exists. The writer keeps an omitted
container omitted when its modeled properties are still empty. An explicitly empty
container stays present, and editing document-default spacing writes the new values.
New documents and older serialized style sheets keep their explicit defaults.
`crates/docx/tests/fixtures.rs` covers omitted `docDefaults`, omitted paragraph defaults,
empty containers and populated defaults across two save/reopen cycles. This preserves
application-defined defaults without assigning inferred Word spacing values to the model.
Observed in Word for Mac 16.113.3 (26092714): three synthetic paragraph-default
controls (omitted, explicitly empty, populated) and two reduced invoice controls retain
identical text, glyph/image geometry, page sizes and embedded font-program hashes through
two WordCraft save/reopen cycles. All fifteen variants open without repair warnings.
Word PDFs and measurements remain local under ignored `plan/word/`; they are not fixtures.
These checks cover preservation on Word reopen, not WordCraft pagination parity.
The [public OOXML reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.paragraphpropertiesdefault?view=openxml-3.0.1)
describes omission as leaving paragraph defaults application-defined. Real-world corpus
coverage and WordCraft layout matching remain separate gaps.
## Custom XML store preservation
A reduced original bound-control DOCX loses its backing data, item properties and relationship
on main, both without editing and after unrelated edits. Main's content-control support (#381)
preserves control metadata, including binding attributes, while this custom XML path keeps the
backing store through the bounded opaque part graph: payload bytes, content types and
related-part IDs survive two edited save/reopen cycles.
The public [dataBinding contract](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.databinding?view=openxml-3.0.1)
maps a control to a custom XML data part; its [item properties](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.customxmldataproperties.datastoreitem?view=openxml-3.0.1)
identify the store. Keeping displayed text alone does not preserve that mapping. This change
preserves stores without evaluating XPath or synchronizing control edits into XML.
Focused original tests cover package/main roots, multiple/shared stores, cycles, external schema
references, missing parts, UTF-16 payloads, strict relationship types and reserved filenames.
The bibliography names used by #532 are reserved; bibliography already loaded into the source
model belongs to its writer. Word-derived files are not fixtures. No readiness estimate rises
from these synthetic package checks, and native Word rendering/binding behavior remains untested.
## Native save-path guarantees
Native `io::save_path_with` serializes (and, for password-protected DOCX, encrypts) before creating an exclusively owned sibling temporary file. It writes and synchronizes that file, closes it, then replaces the destination. Failed preparation or replacement keeps the original bytes; cleanup removes only the save's temporary file, on a best-effort basis. Existing `<name>.tmp` files, directories and links are untouched. A new document gets the default mode (`0666` less the umask). When replacing an existing regular file, the Unix temporary file starts with mode `0600` and then takes the existing file's basic permissions. Serialization and format support are unchanged. Native `io::write_file` (Print to PDF, `file.printPdf`; Export Page as PNG, `file.exportPng`) writes through the same path (#1355).
Synthetic CLI cases and `engine/tests/safe_save.rs` cover occupied names, links, blocked replacement, missing directories, Save As session state, password-protected saves and original-file protection on macOS. Unit tests inject preparation errors and interleave two prepared saves. Independent saves use separate files; the last successful replacement wins. This is not document locking or conflict detection.
Windows/Linux runtime behavior, ACLs and extended metadata, real disk-full conditions, power loss and network filesystems are not verified by these tests. The containing directory is not explicitly synchronized. Cleanup can fail when storage or permissions change, and process termination can leave a temporary file. No Word observation was needed for this filesystem change; it makes no new format-fidelity claim.
## What decides beta for formats

1. A **real-world DOCX corpus** opened, rendered and round-tripped, compared page by page with
   Word (local only: Word output can't be committed; keep a manifest and our own renders).
2. **Preserve what we can't render**: remaining ink forms and unsupported graphics survive a round trip
   (keep the parts and the run). Charts, SmartArt and OLE objects do (#292, #319); drawing-level and direct run-level native ink now keep their InkML and optional picture fallback.
3. Move tracking (`w:moveFrom`/`w:moveTo`) read and written (formatting revisions: #41; content controls: #381).
4. Encrypted `.docx` opens with a password.

## Imported TOC result preservation

Original synthetic TOCs without TOC paragraph styles lose their outer result range on main
and #538. Main also flattens a nested page field in a single-paragraph result. Imported TOCs
now use the existing range-field markers to retain the actual result boundaries, nested
page-reference fields, formatting, links and locks through edited saves.

The public [complex-field contract](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.fieldchar?view=openxml-3.0.1)
defines begin/separate/end delimiters and permits nested fields. Paragraph styles do not define
those boundaries. A complete range is converted to the engine's styled TOC representation only
when that writer can reproduce its exact extent; partial or custom-styled ranges stay explicit.

Focused original regressions cover single/multiple paragraphs, partial ranges, adjacent styled
text, multiple/simple fields, table cells, section breaks, missing ends and bounded nesting.
The existing styled TOC write/read path remains supported. Update Table and Update Fields refresh
unlocked top-level body TOC ranges in place through the engine's generator; one undo restores
the imported range. Locked, nested and table-cell ranges remain preserved without regeneration.
Preservation is separate from field evaluation. Native Word reopening and
refresh, full schema validation and company documents remain untested. Synthetic checks do not
raise readiness estimates. No Word-derived files are fixtures.

## Settings preservation

A save used to regenerate `settings.xml` from the model alone, so every setting the model doesn't
hold was dropped without a warning (on 18 real-world Word documents: compatibility settings in 18
of 18; the privacy options are modelled since #1062). Now the writer merges into the
file's original (`crates/docx/src/write/settings_merge.rs`): the original root and every child
the model doesn't own stay byte for byte; owned children follow the model, in CT_Settings order;
in `w:compat` only `compatibilityMode` is replaced. Kept children that point at a relationship
(a mail-merge data source) are dropped. Files WordCraft wrote itself keep nothing extra. Tests:
`crates/docx/tests/settings_merge.rs`.

## Unmodelled-part preservation

Parts the package or the main document relate to that WordCraft neither models nor writes (web
settings, styles-with-effects, add-in task panes, people, the thumbnail, a glossary, future parts)
are carried verbatim with their relationships and content types. Relationship types the writer
produces, and any the document's markup refers to by id, are not (`HANDLED_RELS` in
`crates/docx/src/read/embed.rs`), so nothing is written twice. `app.xml` keeps Template, Manager,
Company, HyperlinkBase and the link/sharing flags; counts (pages, lines, characters, editing
time) and the producing application's version are not carried, and Manager and Company are
dropped when the file asks to remove personal information on save. Not yet: unknown elements and
attributes inside stories and style or list definitions; a regenerated theme. Tests:
`crates/docx/tests/passthrough.rs`.



## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | trivial | Markdown source view (`view.markdownSource`, beyond Word): a `.md` document switches between formatted and its file's text; in source view `.md`/`.txt` saves write the text exactly, other formats the document it describes, native JSON keeps the view |
| 2026-10-13 | trivial | A cell's explicit top vertical alignment (`w:vAlign w:val="top"`) is kept on read and write and beats a table style that centres or bottom-aligns (#1288) |
| 2026-10-14 | trivial | Row table border exceptions survive DOCX saves |
| 2026-10-12 | trivial | Fields keep `w:dirty` through DOCX round trips — on buffered fields and range-field begin markers alike — and computing a result (Update Field, Update Table) clears it like Word's F9, so a generator's placeholder TOC still refreshes when Word opens the file until WordCraft replaces the placeholder (#877) |
| 2026-10-14 | trivial | VML freeforms (`v:shape` with a `path`, such as rule lines in headers) read as freeforms and kept on save instead of being dropped |
| 2026-10-14 | trivial | DOCX paragraphs inside complex-field instructions carry a hidden paragraph mark; cached result paragraphs remain visible. |
| 2026-10-14 | trivial | DrawingML `flowChartOr` preserves its circle and crossing spokes, including when grouped or rotated |
| 2026-10-14 | trivial | `minorBidi`/`majorBidi` theme fonts name the theme's complex-script font (`a:cs`); an empty one names nothing, so the inherited font applies, as measured in Word |
| 2026-10-14 | trivial | A missing font the font table classes as roman is drawn in Cambria, as measured in Word (Calibri for the other classes, as before) |
| 2026-10-14 | minor | Legacy DOC DIB helper honours the supplied byte limit for its BMP header/copy, decoder and PNG output, with a bounded BMP fallback. Generated 1-pixel regression only; resource-exhaustion consequences unmeasured, fidelity/readiness estimates unchanged. Related: #1334 and #1554 |
| 2026-10-14 | trivial | Right triangles (`rtTriangle`) keep their right angle at the bottom left instead of becoming isosceles triangles, and save back as `rtTriangle` |
| 2026-10-13 | trivial | Section `w:vAlign="both"` reads as Justified and writes back as `both` (#1320) |
| 2026-10-13 | trivial | DOCX: the document's watermark is written as a VML text shape in every header, adding a header where pages had none (#1290) |
| 2026-10-13 | trivial | DOCX: Word's VML text watermark in headers is read as the document's watermark (#1290) |
| 2026-10-13 | trivial | Rounded rectangles keep their own corner radius: `roundRect` `adj` read, written back and drawn (#1476) |
| 2026-10-13 | trivial | Replace All checks content-control locks before editing; a refused batch leaves the document and undo history unchanged; existing deletion-lock rules reused, with tracked/untracked regression coverage |
| 2026-10-14 | trivial | VML preset shapes (`o:spt`, `_x0000_tNN` shapetypes) drawn like the DrawingML preset of the same shape (the nearest shape when WordCraft has none) and kept on save, e.g. Word's right arrow `_x0000_t13` (#1377) |
| 2026-10-14 | trivial | RTF import (#1653): Mac OS Roman (code page 10000) for `\mac` documents, `\fcharset77` fonts and `\cpg`/`\ansicpg` 10000, and DOS text: `\pc` (437), `\pca` (850), the OEM pages through `\cpg` and `\fcharset254` (was read as Windows-1252); a code page number out of range is ignored instead of truncated to another; East Asian double-byte code pages still to do |
| 2026-10-12 | trivial | VML text boxes keep their internal margins (`inset`), fit to their text (`mso-fit-shape-to-text`) and vertical alignment (`v-text-anchor`); hidden drawings (`visibility:hidden`, `wp:docPr/@hidden`) are kept but not shown |
| 2026-10-13 | trivial | Remove personal information / Remove date and time settings kept and honoured on save: anonymous annotation authors, no creator or last-modified-by, no annotation dates (#1062) |
| 2026-10-12 | trivial | Direct run-level `w:contentPart` InkML keeps its relationship, content type and bytes through DOCX edits and repeated saves; nested group/canvas ink and native rendering remain missing |
| 2026-10-13 | trivial | HTML export writes equations as MathML instead of linear text, and HTML import (open and paste) reads `<math>` back into equations; a copy holding an equation puts that HTML on the system clipboard: MathML in HTML on the clipboard (Word/LibreOffice paste not yet verified; OMML is #1297) (#816) |
| 2026-10-14 | minor | DOCX save merges `settings.xml` into the file's original: unmodelled settings (compatibility options, rsids, document variables, proofing state…) survive (#1034) |
| 2026-10-14 | minor | DOCX save carries unmodelled package parts verbatim; comment durable ids and UTC dates, localized `w:default`, latent styles, list `nsid`/`tmpl`/`multiLevelType` and `app.xml` template/company round-trip; comment reference marks use the document's own style id (#1034) |
| 2026-10-14 | trivial | PDF export: All Caps and Small Caps text extracts in capitals, as displayed, and no longer turns later capitals in uncapitalised text lowercase (#1473) |
| 2026-10-14 | trivial | ODT tracked changes: `text:tracked-changes` insertions and deletions open as revisions (author, date, inserted and deleted paragraph marks, insertions nested in deletions) and are written back, a missing date as the save time; `text:track-changes` maps to Track Changes on/off. Pictures, other objects and tables inside a tracked deletion are dropped |
| 2026-10-14 | trivial | RTF reads and writes tracked insertions and deletions with their authors and dates (gap 19); deleted text no longer opens as normal text |
| 2026-10-14 | trivial | ODT import reads sections: a paragraph naming another master page starts a new-page section with that page's size, orientation, margins and columns, and its `style:page-number` restart; a columned `text:section` is a continuous section with its columns (#1379) |
| 2026-10-14 | trivial | PDF export writes right-to-left runs' glyphs in visual order, so Chrome, Edge (PDFium) and poppler extract Hebrew and Arabic as whole words (vowelled words keep their letters in order but may extract with spaces inside), and Ctrl+F finds them (#1383). `/ReversedChars` (for strict file-order readers) covers plain runs and vowelled runs whose marks split into their own glyphs, not runs with ligatures or mixed directions |
| 2026-10-14 | trivial | HTML: `<th>` bold, cell `text-align` (own or inherited) reaches the cell's paragraphs; HTML, Markdown and plain text open on the region's paper, in the app and `wordcraft-cli` (#1381) |
| 2026-10-13 | trivial | PDF export draws horizontal and vertical lines inside groups (#1474) |
| 2026-10-14 | trivial | RTF import keeps sections: page size and orientation, margins, columns, break type, page numbering and each section's headers and footers (#1378) |
| 2026-10-14 | trivial | `.doc` tables: cell shading read from every SHD record (`sprmTDefTableShd`/`ShdRaw` and their 2nd/3rd ranges for cells 22-62; Word 97 `Shd80` clear fills now show their background colour; exact colours win over the palette fallback), the row-end mark no longer becomes an extra empty column, and a table no longer inherits the previous table's grid, borders, alignment or indent (#1380). Horizontally merged rows (a row of fewer, wider cells) come with #1123 |
| 2026-10-12 | trivial | Password-protected DOCX row corrected: agile-encrypted `.docx` opens and saves since #281 (was listed as unsupported); legacy encrypted `.doc` (RC4/XOR) split out, still refused |
| 2026-10-12 | trivial | Restrict Editing: forms mode (`w:edit="forms"`) fills content controls — edits inside fillable controls run and everything else rolls back; `review.restrict` reports `formFields`; Find Next Region covers form fields; a save/reopen keeps `w:edit="forms"`, the filled value and the password guard; under active protection, content-control lock-flag property changes are refused, as Word blocks property edits until Stop Protection (#1077) |
| 2026-10-12 | trivial | Paste keeps the formatting of HTML and RTF on the system clipboard; Paste Special lists them and pictures (#687, #833) |
| 2026-10-12 | trivial | ODT page size, orientation and margins read back from `styles.xml` on open, so they survive a save and reopen (#810) |
| 2026-10-12 | trivial | All 13 table style conditional formatting parts round-trip with alignment (#714) |
| 2026-10-13 | trivial | Print to PDF and PNG export replace an existing file only after the new one is fully written, as document saves do (#1355) |
| 2026-10-12 | minor | RTF reads 8-bit text in the document's or the font's code page (`\ansicpg`, `\fcharset`, `\cpg`) instead of always Windows-1252; a `.doc`/`.docx`/`.rtf` whose content is another of these formats opens by signature. Found on a private corpus of real Russian documents: 38 of 44 `.doc` files were RTF, all now open with their Cyrillic text |
| 2026-10-12 | trivial | `w:sym` and symbol-font runs keep their stored characters on save; TXT export and copy write their Unicode equivalents (#776) |
| 2026-10-12 | trivial | Insert › Picture, drag-and-drop and paste take Windows Metafiles and Enhanced Metafiles (#802); new row for picture formats |
| 2026-10-11 | trivial | Independent complex-script size, bold and italic inherit correctly through defaults and styles; explicit equal values survive; six synthetic multi-generation round-trip tests, including tracked formatting, added |
| 2026-10-11 | minor | Opening files from the system: file associations and the default-app button (#295) |
| 2026-10-12 | trivial | DOCX: a table of contents' entry links (Hyperlink-styled runs in hyperlinks inside a TOC field) import without the Hyperlink style, as Word draws them; measured in Word for Windows |
| 2026-10-12 | trivial | `.doc`: custom tab stops read from styles and paragraphs (`sprmPChgTabsPapx`, `sprmPChgTabs`, with the alignment and leader of each stop; `sprmPChgTabs`' operand size fixed), and a section mark ends its section's last paragraph instead of becoming a page break, so the sections (continuous breaks, footers) are kept. Found by comparing EPA templates with Word for Windows; two of them now paginate like Word |
| 2026-10-12 | trivial | DOCX: a style or paragraph `w:numPr` with a level and no list takes the list of its style chain (Heading 3 based on a numbered Heading 2 numbers 6.4.5, not 6.15), found in NIH's monitoring-plan template against Word for Windows |
| 2026-10-12 | trivial | DOCX: `w:hideMark` on table cells read and written |
| 2026-10-12 | trivial | DOCX text frames (`w:framePr` other than drop caps) are read and written back; they were dropped on save |
| 2026-10-11 | trivial | `.doc`: inline and floating pictures (BLIP store, PNG/JPEG/DIB/TIFF/EMF/WMF), basic shapes, groups and text boxes read from OfficeArt drawings (#551); fidelity ~55% → ~60% |
| 2026-10-12 | trivial | Word 2003 XML Document (WordprocessingML 2003) written (#1081): Export, `format: "wordml2003"`, CLI `--format`; not yet checked in Word |
| 2026-10-11 | trivial | Word XML Document (Flat OPC) read and written, Word 2003 XML read (#548) |
| 2026-10-11 | trivial | #534: DOCX indents in characters read and written; `.doc` first-line indents (`sprmPDxaLeft1`, its two-byte operand, and `sprmPDxaLeft1_80`), indents in characters (`sprmPDxc*`) and exact/at-least line spacing (LSPD) read correctly |
| 2026-10-11 | trivial | List and page number formats: all ST_NumberFormat values round-trip; Chinese, Japanese and Korean counting, legal and digit formats, stems/branches, kana, hangul, enclosed and full-width numbers drawn; unknown values are decimal; .doc number format codes mapped (#534) |
| 2026-10-11 | trivial | SmartArt made in WordCraft written as diagram parts (data, layout, quick style, colours, drawing) and read back editable (#497) |
| 2026-10-11 | trivial | Attached template (`w:attachedTemplate`, external relationship of the settings part) and `w:linkStyles` read and written in schema order; opening never reads the template (#377) |
| 2026-10-11 | trivial | A bookmark (or comment range) ending after the document's last table keeps its end, in the table's last cell (#464); a field whose cached result is a picture (`INCLUDEPICTURE`) is kept as a range field around the picture, so the picture survives save; the linked file is never read (#465) |
| 2026-10-11 | trivial | Print Markup: comments exported in PDFs, prints, page images and HTML/Markdown/text copies (#589) |
| 2026-10-11 | trivial | Files with an extension WordCraft doesn't know open by content: text (`.m3u`/`.m3u8` playlists, logs…) through the encoding-detecting text import, misnamed Word/RTF files by signature, binary files refused with an error (#569) |
| 2026-10-11 | trivial | Restrict Editing: Word's legacy password hash (`w:cryptAlgorithmSid`, `w:hash`, `w:salt`, `w:cryptSpinCount`) verified per ECMA-376 Part 4 §14.8.1; unsupported hash forms refused (#412) |
| 2026-10-11 | trivial | Restrict Editing: `w:documentProtection` (edit, formatting, password hash), `w:locked` styles and `w:permStart`/`w:permEnd` exceptions read and written (#412) |
| 2026-10-13 | trivial | Word 365 cover page (#743): a position wrapped in `mc:AlternateContent` (`wp14:pctPosVOffset`) takes the Fallback's offset; a freeform path drawn back to its start is closed
| 2026-10-12 | trivial | Word 365 cover page (#743): text-box internal margins and `a:spAutoFit` read and written; freeforms inside groups keep their custom geometry |
| 2026-10-11 | trivial | DOCX diagrams (#611): shapes and connectors styled by the theme (`wps:style`) show their outline and fill; preset/system colours; VML lines and rectangles; lines run top-left to bottom-right as in DrawingML |
| 2026-10-14 | trivial | Open DrawingML freeform paths draw `a:tailEnd` arrowheads from their final segment |
| 2026-10-11 | trivial | WordArt text effects (`w14:` run properties), text warps (`a:prstTxWarp`) and connectors with glued ends read and written (#416) |
| 2026-10-11 | trivial | Content controls read and written with their properties and nesting (#381) |
| 2026-10-11 | trivial | Charts made in WordCraft written as chart parts and read back as editable (#409) |
| 2026-10-11 | trivial | Hyphenation zone, words in capitals and consecutive-hyphen limit read and written (#407) |
| 2026-10-11 | trivial | Alt text, titles and the decorative mark for every drawing, table captions and descriptions round-trip (#386) |
| 2026-10-11 | trivial | Document grid (`w:docGrid`) and paragraph `w:snapToGrid` read and written (#391) |
| 2026-10-11 | trivial | Move tracking (`w:moveFrom`/`w:moveTo` and their ranges) read and written (#400) |
| 2026-10-11 | trivial | DOCX text-box `bodyPr` `vert`/`anchor` and linked text boxes (`wps:linkedTxbx`) read and written (#369) |
| 2026-10-10 | trivial | Right-to-left tables (`w:bidiVisual`) read and written (#362) |
| 2026-10-10 | trivial | Drawing canvas (`wpc:wpc`) read and written (#344); group shapes row brought up to date (#267) |
| 2026-10-11 | trivial | Spreadsheets (.xlsx/.xlsm/.ods) read as mail-merge recipient lists (#334) |
| 2026-10-12 | trivial | Font table (`word/fontTable.xml`), its relationships and embedded fonts (`word/fonts/*`) written back unchanged (#770) |
| 2026-10-12 | minor | Preserve opaque custom XML stores and related properties; bindings and control support remain separate |
| 2026-10-11 | trivial | Native saves protect existing temporary-name entries and originals on pre-replacement failure; synthetic macOS evidence, explicit durability/platform limits; readiness unchanged |
| 2026-10-11 | minor | Preserve imported TOC ranges independently of paragraph styles, with explicit evaluation limits |
| 2026-10-11 | trivial | Import/export fixes (#433–#440): TXT opens through the encoding-sniffing reader (Windows-1252, unmarked UTF-16) and saves list labels; RTF `\uN` in document info, bookmark names and field instructions; ODT common paragraph styles' text formatting and table-cell background colours; HTML borderless tables export without cell borders and `white-space:pre` keeps edge spaces; LaTeX brace groups scope `\centering`/`\raggedleft`/`\raggedright` |
| 2026-10-11 | trivial | `.doc` import fixes per [MS-DOC]: piece Prm0/Prm1 decoding (#458), section-break kinds (#459), explicit orientation (#460), negative `sprmPIlfo` list references (#461), zero-based `istdBase` style inheritance (#462), font-table entries with alternate names (#463) |
| 2026-10-11 | minor | Native drawing-level ink and its picture fallback survive DOCX edits and repeated saves; rendering and editing InkML remain missing |
| 2026-10-12 | minor | Native JSON now preserves media, opaque parts and embedded source markup/relationships for lossless recovery of the document model. Older JSON still opens; [recovery limits](recovery.md). |
| 2026-10-12 | trivial | AllowSpaceOfSameStyleInTable read and written; modes 11/12/14 versus 15 checked with original synthetic documents |
| 2026-10-11 | trivial | Formatting revisions (`w:rPrChange`, `w:pPrChange`, table, row, cell, section and numbering changes) read and written (#41) |
| 2026-10-11 | trivial | Omitted `w:pPrDefault` stays omitted across saves; explicit empty/default properties and later spacing edits remain writable. Synthetic preservation checks only; readiness estimates unchanged |
| 2026-10-10 | trivial | OLE objects turned or flipped in WordCraft keep the turn on save (VML `rotation`/`flip`, DrawingML `a:xfrm` with the rotated effect extent); test counts refreshed |
| 2026-10-11 | trivial | Section text direction read and written (#491) |
| 2026-10-10 | trivial | Charts, SmartArt and OLE objects written back on save with their parts (#319); chart/SmartArt reading (#292) recorded |
| 2026-10-10 | trivial | Footnote and endnote separators and continuation notices read and written (#352) |
| 2026-10-11 | trivial | Enclosed characters (`EQ \o\ac`) read, drawn and written (#297) |
| 2026-10-11 | trivial | Ruby (`w:ruby`) read and written (#288) |
| 2026-10-10 | trivial | `wrapText` on square, tight and through wraps read and written (#354) |
| 2026-10-10 | trivial | Wrap polygons for Tight and Through wrapping read and written (#354) |
| 2026-10-10 | trivial | DrawingML rotation and flips read and written (#332) |
| 2026-10-11 | trivial | Footnote and endnote options round-trip for the document and per section (#385) |
| 2026-10-10 | trivial | Equation breaks and `m:mathPr` wrapping options round-trip (#326) |
| 2026-10-10 | trivial | Custom table styles round-trip (#256 merged) |
| 2026-10-10 | major | First version: Word's full format list, read/write status, DOCX element coverage from the source, fidelity and hours |
| 2026-10-10 | trivial | Freeform shapes and ink as DrawingML custom geometry, read and written (#307) |
| 2026-10-12 | trivial | Charts of types WordCraft doesn't draw (radar, stock…) show a placeholder with their title and type (#998) |
| 2026-10-13 | trivial | DOCX styles without any `w:qFormat` (LibreOffice) get Word's usual Styles gallery: matching built-in styles are marked quick and missing quick built-ins added (#1238) |
| 2026-10-12 | trivial | Recipient lists: legacy .xls workbooks and GBK/Big5/Shift_JIS/UTF-16 CSV/TXT (#778); plain-text import detects legacy encodings too |
