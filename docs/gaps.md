# Where WordCraft falls short of Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-17 · **Change:** trivial (Language dialog keeps inherited proofing language and no-proof choices (#2717)); previously trivial (VML freeforms read and kept on save; previously trivial: consistent preferred cell widths; previously trivial: multi-paragraph field instructions no longer add blank lines; previously trivial: page-break-only list paragraphs hide their labels; previously trivial: floating-table overlap ignores empty line-break lines; previously trivial: copies keep their formatting on the clipboard as HTML, #1523; previously trivial: sample-text formulas on Enter, #1067; previously trivial: #29: copied equations as MathML, OMML open as #1297, #816; previously trivial: the Font list no longer keeps every previewed font in memory, #848; previously trivial: table column and row markers on the rulers, #700; previously trivial: table borders and the resize handle drag on the page, #217; previously trivial: formatting revisions landed, #41; previously trivial: Draw Table and Eraser landed, #303; previously minor: beta distance restated: ~15 points, ~150–230 h; previously: first version; every known shortfall from the 2026-10-10 re-measure, ranked) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

> **Printing update:** 2026-10-12 · direct desktop printing through craft-print; details below.

The work list. One entry per shortfall, ranked by how much it stops a Word user from switching
(**B** = blocks beta). Each says what's missing, the evidence, who it hurts, an Opus 5.5 agent
wall-clock estimate, and which parity document tracks it. Agents: pick from the top, check open PRs
first (`gh pr list`), and update this file and the parity doc when a gap closes.

Native save reliability: synthetic cases on macOS reproduced loss through the predictable temporary-file name. Saves now use owned, exclusively created siblings, synchronize before replacement and clean up on failure; originals survive pre-replacement errors. This narrows save-path reliability under gap #3; the listed field failures and broader stability work remain open. See [save-path guarantees](file-format-parity.md#native-save-path-guarantees).
Row table border exceptions (`w:tblPrEx/w:tblBorders`) are read, drawn and saved. Other row table exceptions, including widths and cell margins, remain unsupported.

Consistent cell widths override stale table grid proportions when their totals agree. Conflicting widths, spans and percentages still need the shared-grid algorithm.

Local desktop recovery now covers unsaved drafts and edits to existing files; see
[recovery.md](recovery.md). This narrows gap #3 but does not resolve the crash/hang reports or
establish cross-platform reliability.

Document renaming: desktop renaming requires a filesystem that supports hard links (exFAT/FAT drives and some network shares refuse it). Behaviour: [UI parity](ui-parity.md).

## Ranked

| # | Gap | B | Evidence | Impact | Hours | Doc |
|---|---|---|---|---|---|---|
| 1 | **DOCX never tested on real-world files.** No corpus of files from Word, Google Docs, LibreOffice and Pages has been opened, rendered and round-tripped and compared with Word page by page | B | No corpus in the repo; `crates/docx/tests` are synthetic fixtures (including omitted/empty paragraph defaults and imported TOC range preservation); "DOCX fidelity corpus: not started" since 2026-10-06. Field reports fixed one by one: a cover page's theme-coloured shapes (#611), freeforms inside groups and text-box margins and Resize to fit (#743), cover-page positions relative to the page and freeforms drawn back to their start (#743), Word's text watermark in headers, read and written (#1290), VML freeform rule lines in headers, and `flowChartOr` drawn with its spokes (#1644) | Everyone: the files people receive are the files that break | 30–50 | [file-format-parity.md](file-format-parity.md) |
| 3 | **Crashes and freezes in the field**: startup crash on Intel UHD Graphics (#170, open), a crash on macOS 12.7.6 Intel (#979, open) (lag on GPUs without a DirectX 12 driver fixed, #316; 32-bit Windows exited at launch because DirectX 12 graphics pipeline creation failed, now OpenGL), freezes (#59 Thai fonts and #31 font selector, both open; #77 closed), broken shape tool (#142, open); the Windows 11 no-write-without-admin (#218) and unmovable images (#82) reports are closed (pictures and shapes in Behind Text drag through the text over them, #1313); memory growth on every font change fixed (#848: the font list no longer loads each family it previews) | B | 197 open issues (2026-10-12), among them the crash and freeze reports #170, #979, #59 and #31 | Anyone hit loses work or trust; the never-crash standard covers panics, not hangs or GPU driver faults | 20–30 | [target-app-parity.md](target-app-parity.md) (Stability) |
| 4 | **Pagination differs from Word**: Aptos has no metric-matched substitute; legacy compatibility options only partially covered; no page-by-page comparison harness | B | Legacy same-style cell spacing preserved for measured modes 11/12/14, with mode 15 and non-cell controls; Contextual spacing corrected in body/header probes and repeated table-cell runs; [layout-parity.md](layout-parity.md); [typing-parity.md](typing-parity.md) Known gaps; stray labels on page-break-only list paragraphs fixed multi-paragraph field-code blank lines fixed | Page counts and line breaks differ, which matters for forms, legal and academic work | 30–50 + owner (font) | [layout-parity.md](layout-parity.md) |
| 2 | **Remaining native ink forms are dropped on open and lost on save, silently**; OLE objects show only their picture | B | Drawing-level native ink, its optional picture fallback, and direct run-level content parts are preserved; ink nested in groups/canvases remains unsupported. Drawing canvases are read and written (#344); group shapes (`wpg`) read and written (#267). Charts and SmartArt are drawn (#292) and, with OLE objects, written back with all their parts on save (#319) | Documents with diagrams drawn in Word or pen notes lose content without warning | 4–8 to preserve remaining native ink forms; 45–70 to render and edit charts and SmartArt (#13) | [file-format-parity.md](file-format-parity.md) |
| 5 | **Content controls** landed (#381): `w:sdt` kept, shown and edited (Developer tab, Design Mode, Properties). Move tracking landed (#400): `w:moveFrom`/`w:moveTo` with their named ranges round-trip, a tracked cut and paste of a paragraph or sentence records a move, either end accepts/rejects both, green double strike/underline markup and "Moved from/to" balloons; still missing: drag-and-drop moves (no text drag yet) and move options in Track Changes Options. Formatting revisions landed (#41): `w:rPrChange`/`w:pPrChange`/`w:tblPrChange`/`w:trPrChange`/`w:tcPrChange`/`w:sectPrChange`/`w:numberingChange` round-trip, are recorded while tracking, accepted/rejected and shown in balloons; still missing: tracked style-definition changes and table/section changes made in WordCraft. Opaque custom XML backing stores now survive saves; binding evaluation remains absent. Replace All now checks content-control locks before editing any match; a refused batch preserves the document and undo history. | B | List-label pitch corrected in synthetic metric controls; Issues #41, #400; source grep | Legal and editorial review workflows; templates with form controls | 5–8 | [file-format-parity.md](file-format-parity.md) |
| 6 | **Print dialog depth**: desktop printer choice, copies, collation, duplex and grayscale submit through craft-print; File › Print retains preview, page ranges and odd/even (#641); driver tray/options and pages per sheet remain; web prints through the browser (#209) | B | `ui.print` → `apps/wordcraft/src/print.rs`; issue #15 | Everyone who prints | 5–8 | [hardware-parity.md](hardware-parity.md) |
| 7 | **Dialog depth**: 54 modal dialogs vs Word's ~100 | B | `Dialog` enum in `crates/ui-egui/src/dialogs.rs` | Power users reach for dialogs the ribbon alone doesn't cover | 11–19 | [ui-parity.md](ui-parity.md) |
| 9 | **Password-protected `.doc` files** (AES) can't be opened; password-protected `.docx` opens and saves (agile encryption, #55 closed by #281) | | `crates/docx/src/crypt.rs`; `crates/docbin/src/lib.rs` opens XOR/RC4/RC4 CryptoAPI `.doc` with a password (#1079, untested on Word-written files; AES `.doc` refused) | Corporate and legal users with old files | 3–5 | [file-format-parity.md](file-format-parity.md) |
| 8 | **Proofing covers English, Polish, Portuguese, Italian, Croatian, Turkish, Russian and Czech only**; no dictionaries, grammar or hyphenation for the other ~42 languages Word proofs (text in them is left unmarked) | | `crates/proof` `ProofLang::PROOFED`: English, Polish, Portuguese (pt-BR via the installed VERO dictionary, #797), Italian (it-IT via the installed Hunspell dictionary), Croatian (hr-HR, 2026-10-13), Turkish (tr_TR spelling, grammar, TDK hyphenation, 2026-10-13) and Russian (the installed `ru_RU` dictionary, 2026-10-12), a language per run; Bulgarian has hyphenation only (#826); open PRs add Danish (#723), Russian (#894) and Czech (#1048); Word ships 120 proofing tools; issues #25, #40, #100 open | Every writer outside English, Polish, Portuguese, Italian, Croatian, Turkish and Russian | 20–40 | [localization-parity.md](localization-parity.md) |
| 10 | **Objects**: text in a freely rotated text box (free rotation and flips landed, #332; text direction, alignment and linked text boxes landed, #369; contour wrap and wrap side landed, #354), shape effects, WordArt 3-D effects and connector routing around shapes (WordArt effects and warps and glued connectors landed, #416), dragging members inside a Drawing Canvas (the canvas landed, #344) | | [parity-checklist.md](parity-checklist.md) (Shape Format 11/11, Layout 22/22); group and ungroup landed (#267) | Newsletters, flyers, reports with diagrams | 20–30 | [target-app-parity.md](target-app-parity.md) |
| 11 | **Interface languages**: 3 of the 12 key languages missing (hi, ar, id) and pt-PT (French landed, #729; Vietnamese, #1434; Korean, #1216; 21 interface languages in `crates/ui-egui/src/i18n`); open PRs for Arabic (#737, #1042, #1032), Indonesian (#572) and pt-PT (#1028); no mirrored RTL interface yet; ~5% of strings still English in the translated catalogs | | [localization-parity.md](localization-parity.md) | Most of the world's writers | 50–80 | [localization-parity.md](localization-parity.md) |
| 12 | **Draw tab / ink**: pens, pencil, highlighter, eraser, Select and Hide Ink landed (#307); Lasso Select, Add Pen, Ink to Shape and Ink Replay landed (#395); Ink to Shape also recognises single-stroke arrows (a line plus a V head drawn in one stroke becomes a straight connector with an arrowhead); missing: Ink to Math, two-stroke arrows and arrows with a closed triangle head, pressure, rendering/editing Word's own ink (drawing-level and direct run-level InkML content parts now survive DOCX save) | | [parity-checklist.md](parity-checklist.md) (Draw 10/11) | Pen and tablet users | 4–8 | [hardware-parity.md](hardware-parity.md) |
| 13 | **Charts and SmartArt editing** (editing charts and SmartArt from Word, more chart types and formatting) | | `insert.chart` and Chart Design basics landed (#409): WordCraft's own charts are editable; charts from Word stay read-only (no embedded-workbook editing), no combo/3D/radar/stock charts (charts of those types show a placeholder with their title and type, #998), axis or series formatting; Insert › SmartArt landed (#497) for graphics WordCraft makes | Reports and business documents | 35–55 | [target-app-parity.md](target-app-parity.md) |
| 14 | **`.doc` can't be written; Word 6/95 files refused** | | `crates/docbin` is read-only | Users exchanging files with old Word or legacy systems | 25–40 | [file-format-parity.md](file-format-parity.md) |
| 15 | **References depth**: no EndNote/Mendeley desktop integration (bibliography sources are saved in DOCX, #790; Zotero landed, #189) | | #790 stores sources in `customXml` and reads them back; `crates/zotero` | Students and researchers | 3–6 | [target-app-parity.md](target-app-parity.md) |
| 16 | **Right-to-left completeness**: kashida, RTL in HTML/ODT/RTF; six still-open older RTL bug reports to re-verify after #207 (#211, #199, #66, #63, #48, #19; #215 is closed). RTL tables and sections landed (#362); PDF export's Hebrew and Arabic text copies and searches as whole words (#1383) | | [layout-parity.md](layout-parity.md) | Arabic, Persian, Hebrew writers | 7–11 | [layout-parity.md](layout-parity.md) |
| 17 | **East Asian typography**: Asian Typography options (Phonetic Guide landed, #288; Enclose Characters landed, #297; document grid landed, #391; vertical text landed, #491) | | `para.asianTypography` is live (Paragraph dialog Asian Typography tab, #323) and no East Asian item is left in the catalog's missing list; the remaining depth is not measured | Chinese, Japanese and Korean documents | 11–19 | [layout-parity.md](layout-parity.md) |
| 19 | **RTF and ODT depth**: notes, comments, revisions (RTF and ODT insertions and deletions done), sections (RTF import done, #1378; RTF export and ODT left), RTL. ODT sections landed on import (#1379): page-style changes and columned `text:section`s | | ~2,100 (RTF) and ~1,600 (ODT) lines in `crates/formats`; RTF code pages (#896) and page setup (#902), ODT page setup (#829) landed; open PRs: RTF notes and comments (#1056), ODT notes and comments (#1057), ODT headers and footers (#873) | Users exchanging with LibreOffice or older tools | 18–27 | [file-format-parity.md](file-format-parity.md) |
| 18 | **Screen-reader access** to the document canvas: exposed, not yet tried with real screen readers | | The canvas describes paragraphs, headings, lists, tables, links, pictures and the caret to AccessKit (#488), tested headlessly; Windows Magnifier follows the caret, onto a new line too (#1119); no NVDA, VoiceOver or Orca run yet | Blind and low-vision users; public-sector procurement | 3–6 | [ui-parity.md](ui-parity.md) |
| 20 | **Performance on large real-world documents** unmeasured (500+ pages, many pictures) | | Only the 188-page sample benchmark; scrolling a 1,096-page one-table document went from ~600 ms to ~2–5 ms a frame (#811), but opening it still lays out for seconds | Thesis and book authors | 10–20 | [layout-parity.md](layout-parity.md) |
| 21 | **Mail merge data sources**: no Outlook/Contacts, Access or database sources, no email merge; CSV, typed lists (#247) and Excel/OpenDocument spreadsheets (#334, legacy .xls #778) only | | Select Recipients reads .csv/.tsv/.txt in any common encoding (UTF-8/16, GBK, Big5, Shift_JIS…), .xlsx/.xlsm, .xls and .ods; Edit Recipient List filters and sorts (#778) | Office administrators | 4–8 | [target-app-parity.md](target-app-parity.md) |
| 22 | **Ecosystem**: no VBA execution (macros preserved only), no Office add-ins, no cloud storage, no real-time co-authoring | | `.docm` keeps macros (#172) | Enterprise users with macro templates; teams | 40–80 + owner decisions | [target-app-parity.md](target-app-parity.md) |
| 23 | **PDF Reflow** (open a PDF as an editable document, Word for Windows) | | | Users who edit PDFs they receive | 30–50 | [file-format-parity.md](file-format-parity.md) |
| 24 | **Default shortcuts**: 140 (macOS) / 158 (Linux/Windows) of Word's ~250 (#330, #339, #343; Customize Keyboard #368, Customize Ribbon and Quick Access Toolbar #379; inspector, macros, strikethrough, file/edit keys) | | [ui-parity.md](ui-parity.md) | Power users | 1–3 | [ui-parity.md](ui-parity.md) |
| 25 | **In-app AI assistant** (Copilot-style draft, rewrite, summarize) | | Chat PR #178 and AI assistant PR #815 open | Users expecting Copilot | 15–30 + owner (provider) | [target-app-parity.md](target-app-parity.md) |
| 27 | **View windows**: Side by Side, Synchronous Scrolling, Arrange All, Switch Windows | | [parity-checklist.md](parity-checklist.md) (View 25/29); PR #331 open | Comparing documents | 3–5 | [ui-parity.md](ui-parity.md) |
| 26 | **Minor formats**: MHT, Works/WordPerfect | | Word XML Document (Flat OPC) opens and saves, Word 2003 XML opens (#548) and saves (#1081, not yet checked in Word) | Rare | 14–26 | [file-format-parity.md](file-format-parity.md) |
| 28 | **Dictate** | | `tools.dictate` missing | Dictation users | 8–15 + owner (speech model) | [hardware-parity.md](hardware-parity.md) |
| 29 | **Equations**: ink equations; copied equations as OMML for Word (#816, #1297) | | #191 notes; long display equations wrap at operators (#326); a copy holding an equation puts MathML in HTML on the clipboard (#816; Word/LibreOffice paste not yet verified) | Maths-heavy documents | 2–3 | [layout-parity.md](layout-parity.md) |

**Beta needs #1–#7** (~150–230 h with the stability and layout work they imply): they are the
blocking gaps, and closing them lifts file formats, layout, stability and UI enough to take the
full number from ~60% to ~75%. Everything else is depth on the way to full parity.

## By kind

Sample-text formulas on Enter (#1067) are implemented: `=rand()` and `=lorem()`,
optional paragraph/sentence counts and one-step undo. See [typing-parity.md](typing-parity.md)
for the accepted parameter range and validation limits; readiness numbers are unchanged.

The same gaps grouped the way the parity documents are, for agents working in one area.

- **Feature gaps:** #2 (preserve), #5, #10, #12, #13, #15, #17, #21, #25, #27, #28, #29; the Clipboard pane, Manage Styles and Group landed (#281, #323, #267). Paste Special, Advanced Find, Column Selection, Style Inspector, custom table
  styles, and Draw Table and Eraser (#303) landed on 2026-10-10. Paste from other programs keeps
  their formatting (system clipboard HTML/RTF), Merge Formatting keeps emphasis, Paste Special lists
  what the system clipboard holds, and Linux/Wayland clipboard pictures paste (#687, #833);
- **UI/UX gaps:** #7, #18, #24; context menus for objects (the table's landed; it still lacks Border Styles); Linux title bar theming (#78); File ›
  still missing: the Paste Options button after a paste, and RTF on macOS and X11. Copy puts
  HTML beside the text on the clipboard, so other programs keep the formatting (#1523), with
  equations as MathML (#816; pasting into Word/LibreOffice not yet verified; OMML on the
  clipboard is #1297); RTF on the clipboard is not written yet.
  Options › Animation effects › Use system setting reads macOS (Reduce motion) and the Linux/BSD
  desktops (`apps/wordcraft/src/animations.rs`) but hasn't been tried on a Mac or in a real
  GNOME or KDE session yet.
- **File-format gaps:** #1, #2, #5, #9 (`.doc` only), #14, #19, #23, #26; embedded fonts; glossary/building blocks.
- **Hardware gaps:** #6, #12, #28; scanner/Continuity Camera insert; KDE soft text (#140).
- **Localization gaps:** #8, #11, #16, #17; native-speaker review of every catalog (human).
- **Stability and performance gaps:** #3, #20.

The Language dialog reads effective character formatting, including document defaults, paragraph styles and character styles. Accepting it unchanged keeps inherited language and no-proof choices (#2717); explicit direct formatting still takes precedence. Other proofing and Windows-specific reports remain separate.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-17 | trivial | Language dialog keeps inherited proofing language and no-proof choices (#2717); readiness estimates unchanged |
| 2026-10-14 | trivial | Row table border exceptions fixed; other row table exceptions remain unsupported |
| 2026-10-12 | minor | Czech proofing review: short-word splits rank after deletion, NOSUGGEST excludes suggestions while accepting typed forms, affix comments apply only to headers, ratios keep spaces and repeated-word messages use Czech quotes; synthetic engine end-to-end and latency checks |
| 2026-10-12 | minor | Czech spelling from the installed Hunspell dictionary, runtime-table suggestions and generic checks; Czech interface writes cs-CZ documents; no Czech hyphenation patterns bundled; estimates unchanged |
| 2026-10-12 | minor | Gap #24 default shortcuts expanded (140 commands on macOS / 158 on Linux/Windows); stale gaps #9 (DOCX agile encryption already works, only encrypted .doc missing) and #15 (DOCX bibliography sources already saved, only desktop integrations missing) updated |
| 2026-10-13 | trivial | Previously untracked long Web/Draft page blur resolved with visible high-resolution raster tiling; no gap remains for this issue |
| 2026-10-14 | trivial | #1: VML freeforms (rule lines in headers) read and kept on save |
| 2026-10-14 | trivial | Consistent preferred cell widths |
| 2026-10-14 | trivial | Pagination gap #4 narrowed: paragraph boundaries inside complex-field instructions add no flow height. Floating-table placement and other paragraph metrics remain open. |
| 2026-10-14 | trivial | A list paragraph containing only a page break has no visible label; it still advances the list counter. Ordinary empty list items keep their label. |
| 2026-10-14 | trivial | Field report: grouped `flowChartOr` omitted its crossing spokes; the preset now draws them; wider DOCX corpus work remains open |
| 2026-10-14 | trivial | Floating-table overlap checks use visible line content; empty line-break lines no longer move a table |
| 2026-10-13 | trivial | Gap #3: Behind Text pictures and shapes drag over text in the body and headers, selected or not; a click without movement still edits the text |
| 2026-10-13 | trivial | Row 1: the watermark is saved to DOCX headers (#1290) |
| 2026-10-13 | trivial | Row 1: Word's text watermark in DOCX headers is drawn (#1290) |
| 2026-10-12 | trivial | Re-audited every row against main: #9 narrowed to encrypted `.doc` (#281), #15 sources saved in DOCX (#790), #10 group landed (#267), #11 French landed (#729), #12 Draw 10/11, #16 #215 closed, #17 `para.asianTypography` live, #3 closed reports and 197 open issues, #8/#19/#21/#25/#27 open PRs named, evidence counts refreshed |
| 2026-10-12 | trivial | Document title opens Rename Document; the account badge opens Options › General for user name and initials. |
| 2026-10-13 | trivial | Replace All checks content-control locks before editing; a refused batch leaves the document and undo history unchanged; existing deletion-lock rules reused, with tracked/untracked regression coverage |
| 2026-10-13 | trivial | #18: Windows Magnifier follows the caret onto a new line after Enter or Shift+Enter (#1119); real screen-reader testing remains |
| 2026-10-12 | trivial | Sample-text formulas on Enter (#1067) implemented; numerical limits not compared in Word |
| 2026-10-12 | minor | Italian proofing landed (it-IT spelling from the installed Hunspell dictionary, accent-first suggestions, conservative grammar); gap #8 narrowed to the remaining languages |
| 2026-10-12 | trivial | #2 narrowed: direct run-level `w:contentPart` InkML survives DOCX edits and repeated saves; nested group/canvas ink and native rendering remain |
| 2026-10-14 | trivial | Gap 19: ODT tracked insertions and deletions (with paragraph marks, and text inserted then deleted) read and written |
| 2026-10-14 | trivial | Gap 19: RTF tracked insertions and deletions read and written; RTF paragraph-mark and formatting revisions, and ODT revisions, remain |
| 2026-10-14 | trivial | #19 narrowed: ODT import keeps sections (master-page changes, columned `text:section`s, page-numbering restarts) (#1379) |
| 2026-10-14 | trivial | #16: PDF export writes right-to-left glyphs in visual order, so Hebrew and Arabic extract, copy and search as whole words (#1383) |
| 2026-10-14 | trivial | RTF import keeps sections (#1378): gap 19 narrowed to RTF section export and ODT sections |
| 2026-10-12 | trivial | Forms protection filling landed (#1077): under Restrict Editing with `w:edit="forms"` the fillable content controls take entries and typing, everything else rolls back; Find Next Region covers form fields (pane button shows for forms documents); residual gap: legacy `w:ffData` form fields still ❌ (row in [file-format-parity.md](file-format-parity.md)) |
| 2026-10-13 | trivial | Crop handles on the canvas landed: off the UI/UX gaps |
| 2026-10-11 | trivial | Table column and row markers on the rulers landed (#700): the ruler's column markers are off the UI/UX gaps |
| 2026-10-11 | trivial | UI/UX gaps: the table context menu landed, its items following the selection as in Word (Delete Cells, Rows, Columns or Table, Merge and Split Cells, Distribute, AutoFit, Text Direction, Insert Caption…, Table Properties…); objects' menus and Border Styles remain |
| 2026-10-14 | trivial | #12 narrowed: Ink to Shape recognises single-stroke arrows |
| 2026-10-11 | trivial | Tables: ordinary drags, Shift+arrows and Shift+click over cells select a rectangle of whole cells (into or out of a table, whole rows); removed from the UI/UX gaps |
| 2026-10-11 | trivial | Tables: selection areas, insert buttons, move handle and double-click AutoFit on the page landed; ordinary drags over cells of several rows (a run of text, not a rectangle) added to the UI/UX gaps |
| 2026-10-13 | trivial | #1: Word 365 cover page (#743): shapes placed relative to the page (`wp14:pctPosVOffset`) take their offset; freeforms drawn back to their start are filled |
| 2026-10-12 | trivial | #1: Word 365 cover page (#743): freeforms in groups keep their outline; text boxes keep their own margins and Resize shape to fit text, so no line is cut |
| 2026-10-12 | trivial | #3: 32-bit Windows no longer exits at launch; DirectX 12 graphics pipelines fail to create, so that build starts on OpenGL |
| 2026-10-12 | trivial | Gap #3: browsing the Font list no longer keeps every previewed family in memory (#848); each applied family still loads once |
| 2026-10-12 | trivial | Custom dictionary persists across sessions (`custom-dictionary.txt` in the settings folder; #40) |
| 2026-10-11 | trivial | #8 narrowed: runs in a proofing language without a dictionary are skipped instead of flagged against the English dictionary (#100); dictionaries still English-only |
| 2026-10-11 | trivial | #16: DOCX open/save preserves independent complex-script formatting inheritance; RTL sections, tables and typography remain open |
| 2026-10-11 | trivial | `.doc` pictures, shapes and text boxes read (#551); gap #14 narrowed to the writer and Word 6/95 files |
| 2026-10-11 | trivial | Table column and row borders and the resize handle drag on the page (#217); UI/UX gap narrowed to the ruler's column markers |
| 2026-10-11 | trivial | UI/UX gaps list: the system's animation setting is now read on macOS and the Linux/BSD desktops; trying it on real machines remains |
| 2026-10-11 | trivial | Animation effects setting landed (File › Options › Accessibility); UI/UX gaps list: the system's animation setting isn't read on macOS or Linux/BSD yet |
| 2026-10-12 | trivial | Saving Word 2003 XML Document landed (#1081); gap #26 narrowed to 14–26 h |
| 2026-10-11 | trivial | Word XML Document (Flat OPC) and Word 2003 XML opening landed (#548); gap #26 narrowed to 15–28 h |
| 2026-10-12 | minor | Desktop File > Print now discovers printers and submits PDF jobs through pinned craft-print with printer choice, copies, collation, duplex and grayscale; the upstream preview, ranges, parity and Print Markup remain available, along with browser printing and PDF export. Remaining print parity includes driver-specific tray/options and pages-per-sheet UI; physical printer fidelity needs hardware validation. |
| 2026-10-11 | trivial | #13 narrowed: Insert › SmartArt with eight layouts, Text Pane and SmartArt Design landed (#497); 25–40 h left for charts |
| 2026-10-11 | trivial | File › Options panes landed (#485); removed from the dialog-depth list |
| 2026-10-11 | trivial | #18: the document canvas describes its text, structure and caret to screen readers through AccessKit (#488); real screen-reader testing remains |
| 2026-10-11 | trivial | Compare / Combine Documents and AutoCorrect Options dialogs and the Restrict Editing pane landed (#412); removed from the dialog-depth list; duplicate #7 rows merged |
| 2026-10-14 | trivial | Open DrawingML freeform paths draw their `a:tailEnd` arrowhead |
| 2026-10-11 | trivial | #10: WordArt text effects and Transform warps, connectors glued to shapes (#416) |
| 2026-10-11 | trivial | #5 narrowed: content controls kept, shown and edited (#381); move tracking remains |
| 2026-10-11 | trivial | Insert › Chart landed (#409); gap #13 narrowed to editing Word's charts, more types and SmartArt, 35–55 h |
| 2026-10-11 | trivial | Line Numbers, Hyphenation, Language, Envelopes and Labels dialogs landed (#407); gap #7 at 30 dialogs, 16–24 h; duplicated #7 rows merged |
| 2026-10-11 | trivial | Caption, Index, Mark Index Entry, Custom Table of Contents, Source Manager and Create/Edit Source dialogs landed (#402); removed from the dialog-depth list |
| 2026-10-12 | minor | #8: Portuguese proofing (spelling, suggestions, grammar, hyphenation) landed; the gap is now the other languages (20–40 h) |
| 2026-10-11 | trivial | Document grid landed (#391); removed from the East Asian gap |
| 2026-10-11 | trivial | Move tracking landed (#400); gap #5 narrowed to content controls, 5–8 h |
| 2026-10-11 | trivial | #4: note options per section (below text, end of section, restart each section/page) and the Footnote and Endnote dialog (#385) |
| 2026-10-11 | trivial | #10: Shape Format › Text: text direction, align text and linked text boxes (#369) |
| 2026-10-11 | trivial | #16: right-to-left tables (`w:bidiVisual`) and sections (`w:bidi`) laid out mirrored, with `table.rtl` and `layout.sectionRtl` (#362) |
| 2026-10-11 | trivial | #4: columns balance before a continuous section break (#350) |
| 2026-10-11 | trivial | #21: Excel (.xlsx/.xlsm) and OpenDocument (.ods) spreadsheets as recipient lists (#334); hours 6–10 → 4–8 |
| 2026-10-11 | trivial | Insert › Icons landed (#309); removed from the objects gap |
| 2026-10-12 | minor | Preserve opaque custom XML data/properties through saves; gap #5 and readiness remain unchanged |
| 2026-10-11 | minor | Imported TOC boundaries preserved in synthetic checks; real-world corpus gap and readiness unchanged |
| 2026-10-11 | trivial | #24 narrowed: Customize Ribbon and Quick Access Toolbar customization landed (#379); the default shortcuts remain |
| 2026-10-11 | trivial | #24 narrowed: Customize Keyboard landed (#368); Customize Ribbon and the Quick Access Toolbar remain |
| 2026-10-11 | trivial | Native saves protect existing temporary-name entries and originals on pre-replacement failure; synthetic macOS evidence, explicit durability/platform limits; readiness unchanged |
| 2026-10-11 | trivial | #24: Word's field keys (Alt+F9, Shift+F9, Ctrl+Shift+F9, Ctrl+F11, Ctrl+Shift+F11, F11, Shift+F11) (#339) |
| 2026-10-10 | trivial | #24: 125 of ~250 keyboard shortcuts after #330 |
| 2026-10-11 | trivial | Collapsed duplicate rows left by parallel merges |
| 2026-10-11 | minor | Native drawing-level ink and its picture fallback survive DOCX edits and repeated saves; rendering and editing InkML remain missing |
| 2026-10-12 | minor | List-label automatic/AtLeast line-height inflation corrected; gap #4 remains open and readiness unchanged |
| 2026-10-12 | minor | Gap #3 narrowed: local desktop recovery snapshots, discovery, unsaved restore and confirmed discard are implemented. Password sessions skip plaintext copies. Crash prevention, Windows field testing and soak testing remain; readiness estimates unchanged. |
| 2026-10-12 | trivial | One legacy table compatibility option now supported; modes 11/12/14 versus 15 checked with original synthetic documents |
| 2026-10-11 | minor | #4: body/header contextual spacing corrected; ordinary overlap remains #108 and pagination remains a beta gap |
| 2026-10-11 | trivial | All 12 bibliography styles landed (#383); removed from References depth |
| 2026-10-11 | trivial | Formatting revisions landed (#41); gap #5 narrowed to move tracking and content controls, 8–12 h |
| 2026-10-11 | trivial | Omitted `w:pPrDefault` stays omitted across saves; explicit empty/default properties and later spacing edits remain writable. Synthetic preservation checks only; readiness estimates unchanged |
| 2026-10-11 | trivial | #12 narrowed: Lasso Select, Add Pen, Ink to Shape and Ink Replay landed (#395) |
| 2026-10-10 | trivial | Draw Table and Eraser landed (#303); removed from the feature-gap list |
| 2026-10-10 | trivial | Tabs and Borders and Shading dialogs landed (#320); removed from the dialog-depth list |
| 2026-10-10 | trivial | Columns, Symbol and Field dialogs landed (#321); removed from the dialog-depth list |
| 2026-10-10 | trivial | #2 narrowed: charts and SmartArt drawn (#292), and with OLE objects kept on save (#319); group shapes landed (#267); ink and drawing canvases remain |
| 2026-10-10 | trivial | #4: a document's own footnote separators and continuation notice are drawn and kept on save (#352) |
| 2026-10-10 | trivial | #4: long footnotes continue onto the next page (#352) |
| 2026-10-10 | trivial | Define New Multilevel List and Track Changes Options dialogs landed (#328); removed from the dialog-depth list |
| 2026-10-11 | trivial | Enclose Characters landed (#297); removed from the East Asian typography gap |
| 2026-10-11 | trivial | Phonetic Guide (ruby) landed (#288): gap 17 narrowed |
| 2026-10-10 | trivial | #10: wrap side (both sides, left, right, largest) for Square, Tight and Through objects (#354) |
| 2026-10-10 | trivial | #10: Tight and Through wrapping follow the object's outline (#354) |
| 2026-10-10 | trivial | #10: free rotation and flips for pictures, shapes and text boxes, with a rotation handle (#332); text boxes rotate their frame; their text stays upright |
| 2026-10-10 | trivial | #10: Drawing Canvas landed (#344); in-canvas dragging of members remains |
| 2026-10-14 | trivial | Feature gaps: every copy puts HTML beside its text on the clipboard, not only copies with an equation (#1523); writing RTF stays open |
| 2026-10-13 | trivial | #29: copied equations reach the clipboard as MathML in HTML (#816); OMML for Word stays open (#1297) |
| 2026-10-10 | trivial | #29: long display equations wrap across lines (#326) |
| 2026-10-10 | trivial | #3: Windows lag on GPUs without a DirectX 12 driver fixed with an OpenGL fallback (#316) |
| 2026-10-10 | minor | #8: Polish proofing (spelling, grammar, hyphenation) and a proofing language per run landed; the gap is now the other languages (20–40 h) |
| 2026-10-10 | trivial | #6: desktop File › Print opens the system PDF viewer (#286) |
| 2026-10-10 | trivial | Table Properties dialog landed (#44); removed from the dialog-depth list |
| 2026-10-10 | trivial | Merged main: landed features removed from the feature-gap list |
| 2026-10-10 | minor | Beta note restated: full number back to the additive ~60% (method aligned with the standard, no new evidence) |
| 2026-10-11 | trivial | #491: vertical sections landed; vertical text removed from the East Asian gap |
| 2026-10-10 | major | First version, from the full re-measure: 29 ranked gaps with evidence, impact and hours |
| 2026-10-10 | trivial | #307: Draw tab pens, eraser and Hide Ink landed; ink gap narrowed |
| 2026-10-12 | trivial | #21: .xls workbooks and GBK/Big5/Shift_JIS/UTF-16 text lists as recipients; Filter and Sort in Edit Recipient List (#778) |
| 2026-10-12 | trivial | #811: large-table scrolling fixed; performance gap (#20) narrowed |
| 2026-10-12 | trivial | #13: radar, stock and other undrawn chart types show a placeholder instead of blank space (#998) |
