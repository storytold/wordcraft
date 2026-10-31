# WordCraft parity with Microsoft Word

> **Last reviewed:** 2026-10-31 · **Last updated:** 2026-10-31 · **Change:** minor (re-rated against main at v0.13.2: measured counts, stale evidence, area and dimension percentages; no real-world comparison); previously trivial (Tables row: Table Properties applies to the selected rows, columns and cells, #4857; no percentage change); previously trivial (Recipient labels continue across product-sized sheets instead of stopping after the first sheet (#4880)); previously trivial (Insert Table of Figures lists any caption label, #3013; previously trivial: Performance row: typing no longer starts render threads on every repaint, #3027; previously trivial: shapes inserted in a row are all kept, #2019; previously trivial: East Asian typography: `w:eastAsianLayout` kept on save, not laid out; previously trivial: Ink to Shape recognises arrows; previously trivial: Windows Magnifier follows the caret onto a new line, #1119; previously trivial: table border drags with Ctrl and Alt, from the left edge and of selected cells, #711; previously trivial: table column and row markers on the rulers, #700; previously trivial: table borders and the resize handle drag on the page, #217; previously trivial: all 12 bibliography styles, #383; previously trivial: formatting revisions landed, #41; previously trivial: Draw Table and Eraser landed, #303; previously minor: readiness table with hours per audience; full number back to the additive weighted sum, 60%; mainstream and essentials numbers added; alpha gate checked; previously major: full re-measure against Word for Mac 16.113.4; replaces the parity tables that lived in ROADMAP.md) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

The authoritative assessment of how close WordCraft is to Microsoft Word. [`ROADMAP.md`](../ROADMAP.md)
summarizes it; [`gaps.md`](gaps.md) itemizes every shortfall; the deep checklists are
[`file-format-parity.md`](file-format-parity.md), [`layout-parity.md`](layout-parity.md),
[`typing-parity.md`](typing-parity.md), [`ui-parity.md`](ui-parity.md),
[`hardware-parity.md`](hardware-parity.md) and [`localization-parity.md`](localization-parity.md).
The generated ribbon checklist is [`parity-checklist.md`](parity-checklist.md).

Labels → New Document with one label per recipient continues across sheets for all selected recipients (#4880). The final sheet leaves unused labels blank; address mapping, fixed-text and single-label modes stay unchanged. This is label generation, not a new Update Labels or Finish & Merge template workflow. Readiness estimates are unchanged.

## Headline (2026-10-31, origin/main `eecbb8fe`, v0.13.2)

| Number | Value | Kind |
|---|---|---|
| Ribbon/menu catalog coverage | **505 / 520 (97.1%)** | **measured**: `wordcraft-cli parity --markdown` built from this commit; identical to the checked-in `parity-checklist.md` |
| Feature breadth (weighted, beyond the ribbon: dialog options, citation styles, proofing languages, chart types, UI languages) | **~84%** | estimated |
| Feature depth (weighted by use, table below) | **~74%** | estimated |
| **Ready for real work** (full target) | **~65%** (range 60–68%) | estimated: weighted sum over the dimension table ([By dimension](#by-dimension)) |
| **Mainstream practitioner** | **~61%** | estimated, same method |
| **Essentials user** | **~68%** | estimated, same method |
| Remaining effort to beta | **~135–230 h** of Opus 5.5 agent wall-clock: the blocking beta list ([gaps.md](gaps.md) #1–#7, whose rows sum to 116–195 h, plus ~20–35 h of the stability and layout work they imply), which lifts file formats, layout, stability and UI enough to reach ~75% | estimated |
| Remaining effort to full parity with Word desktop | **~600–1,050 h** (the sum of the dimension table) | estimated |
| Commands | 629 engine commands (+ `ui.*` commands in the front end) | measured (`wordcraft-cli commands`) |
| Tests | 4,713 `#[test]` functions + 22 `proptest!` blocks | measured (source count) |
| Code | ~337,600 lines of Rust in 16 crates, 3 apps and xtask | measured (`wc -l`) |

**Stage: alpha.** All six core workflows passed the [alpha gate](roadmap.md#alpha-gate-core-workflows) when last run, on 2026-10-10; see [`ROADMAP.md`](../ROADMAP.md) for why it isn't beta, and the distance.

## What we measured against

- **Microsoft Word for Mac 16.113.4** (Microsoft 365 subscription build, installed on the owner's Mac
  at `/Applications/Microsoft Word.app`; version read with `mdls`, bundle modified 2026-10-06).
- Inspected black-box, by **file listing only**, as `AGENTS.md` § Clean room requires (no bundle file
  was opened or read): 31 `.lproj` folders (30 interface languages plus Base), 120 proofing tools
  under `SharedSupport/Proofing Tools` (spellers, grammars, hyphenators and thesauri for ~50
  languages), 12 bibliography styles under `Resources/Style` (APA 6th, Chicago, GB 7714, GOST name
  and title, Harvard Anglia, IEEE, ISO 690 author-date and numerical, MLA 7th, SIST02, Turabian),
  11 Quick Style sets plus Word 2010, Office themes, a PDF print-dialog extension (`WordPDE.plugin`),
  ink rendering (`InkRender.bundle`), a Copilot backstage bundle, and text converters (Excel 2.x–11,
  Recover Text).
- Word's file formats, ribbon, dialogs and keyboard shortcuts from Microsoft's public documentation
  and our knowledge of Word for Windows and Mac (Microsoft 365). Windows-only features (PDF Reflow
  open, Works/WordPerfect import) are counted, because a Word user on Windows expects them.
- WordCraft as it is on origin/main today: source (catalog, command registry, readers/writers,
  layout), tests, the i18n catalogs, open issues (1,214 on 2026-10-31; 62 on 2026-10-10) and open PRs (1,079 on 2026-10-31; ~45 on 2026-10-10), git history.
- **Not done this pass (2026-10-31 either):** opening real-world .docx files in both apps and comparing pages.
  There is no real-world corpus in the repo (Word-produced files can't be committed). That
  comparison is the first gap in [`gaps.md`](gaps.md). Word itself was not re-measured on
  2026-10-31, and the core-workflow gate was not re-run.

## Feature areas

Percent is depth and fidelity against Word, not presence. Weight is our estimate of the share of a
typical Word user's time spent in the area (sums to 100). Hours are Opus 5.5 agent wall-clock to
full parity for the area.

| Area | Weight | Parity | Was (2026-10-10) | Hours | Evidence and what's missing |
|---|---|---|---|---|---|
| Typing, selection, clipboard, undo, find/replace | 12 | 87% | 85% | 6–10 | Typing parity pinned by `tests/typing.rs` ([typing-parity.md](typing-parity.md)). Paste Special (#235), Advanced Find (#234) and Column Selection (#237) landed; Clipboard pane landed (#264, #323). Regular-expression Replace expands capture groups. |
| Character and paragraph formatting | 11 | 84% | 82% | 6–10 | Nearly every property, Font/Paragraph dialogs, character border, Phonetic Guide (#288), Enclose Characters (#297). Kerning for fonts in the Font dialog. Asian Typography tab landed (#323). Missing: ligature/stylistic-set options in the Font dialog's Advanced tab. |
| Styles and themes | 8 | 77% | 74% | 6–10 | Gallery, pane, create/modify, style sets, themes. Style Inspector (#236) and Manage Styles (#270, #323) landed. Missing: style separators, linked-style edge cases; Word ships 11 Quick Style sets plus dozens of themes, we ship our own smaller set. |
| Lists | 6 | 80% | 78% | 4–7 | Bullets, numbering, multilevel, restart, `w:lvlOverride` levels (#134), Word's list AutoFormat and Enter/Backspace behaviour. Missing: Define New Multilevel List dialog depth, list styles, legal numbering edge cases. |
| Tables | 8 | 80% | 76% | 8–12 | Merge/split, styles (cell text takes style formatting, #104), custom table styles saved in the .docx (#256), Word 2013 edge, floating tables, rows split across pages, sort, formula, cell gridlines (#243), Draw Table and Eraser (#303), tables nested 24 deep at most, as the .docx reader and writer keep them (#1596; Word has no limit). Cell Margins opens an editable dialog initialized from table defaults and preserves unchanged values (#1960). Missing: splitting or erasing around vertically merged cells, nested-table polish. Dragging column and row borders and the resize handle on the page (#217) and the column and row markers on the rulers (#700) landed, with Word's Ctrl, Alt, left-edge and selected-cells drags (#711), and so did selecting columns, rows and cells, the "+" insert buttons, the move handle and double-click AutoFit on the page, Select Column, and drags and Shift+arrows over cells selecting whole cells; right-clicking a cell offers the table's commands, following the selection as in Word (Delete Cells, Rows, Columns or Table, Merge and Split Cells, Distribute, AutoFit, Text Direction, Insert Caption…, Table Properties…); cell text direction landed (#245). |
| Page layout and sections | 7 | 75% | 70% | 8–14 | Margins, size, columns, breaks, page borders, line numbers, vertical alignment, drop caps, hyphenation. Columns balance before a continuous break (#350), not after next-page breaks. RTL sections (#362) and vertical sections (tbRl/btLr, #491) landed. Detail: [layout-parity.md](layout-parity.md). |
| Headers, footers, page numbers, fields | 5 | 73% | 70% | 6–10 | First/even/odd, link to previous, TOC as an updatable field (#52), cross-references to real bookmarks (#227). Field dialog landed (#321); field results take `\* DollarText`, `\* Hex` and `\* ArabicDash`. Field coverage is a subset of Word's ~90 field codes. |
| Footnotes and endnotes | 3 | 70% | 60% | 3–5 | Placed and editable, numbers from Word files (#103); long notes continue onto the next page under the continuation separator (#352). Footnote and Endnote dialog with options per section: below text, end of section, number format, start at, restart each section or page, custom mark, convert (#385). |
| References (TOC, citations, bibliography, captions, index, TOA) | 5 | 72% | 65% | 8–14 | Working first versions, Zotero integration (#189), Zotero/Mendeley `ADDIN` fields round-trip. Caption, Index, Mark Entry, Custom TOC, Table of Figures and Create/Edit Source dialogs; updates honour the TOC (`\o \u \t \f \n \p \h`), table of figures (`\c \a \n \p`, any caption label, so a list of tables sits beside a list of figures; #3013), INDEX (`\c \e \h \r`), XE (`\b \i \t \r`) and SEQ/STYLEREF chapter switches (#402); Insert Caption… on the right-click menu of pictures, shapes, text boxes and charts (#1230). All 12 of Word's bibliography styles (APA, MLA, Chicago, IEEE, Harvard, ISO 690 author-date and numerical, Turabian, GB/T 7714, GOST name and title sort, SIST02; #383); bibliography sources saved in DOCX (#790). Manage Sources imports Word's bibliography file (`b:Sources`, `Sources.xml`, #2640). Cross-reference dialog (#916), with captions in a style based on Caption (#4411); corporate authors kept (#4848). |
| Review (proofing UI, comments, track changes, compare, protect) | 8 | 77% | 72% | 10–18 | Comment balloons, tracked insert/delete incl. paragraph marks (#244, #125), Compare / Combine with comparison settings (case, white space, formatting, tables, word or character level, #412), Restrict Editing pane with formatting and editing restrictions, exceptions for everyone and a password (#412). Formatting revisions (#41): kept in DOCX, recorded while tracking, accepted/rejected, "Formatted: …" balloons and change bars. Moves (#400): kept in DOCX, recorded on a tracked cut and paste, accepted/rejected at either end, green markup and "Moved from/to" balloons. Show revisions in balloons takes tracked deletions in the body out of the text into "Deleted: …" balloons (insertions stay inline, as in Word; deletions in headers, footers, notes and text boxes, and in print and PDF, stay inline). Missing: drag-and-drop moves, insert/delete balloons, Translate, Block Authors, modern comment threads/mentions. |
| Proofing (spelling, grammar, thesaurus, languages) | 5 | 50% | 44% | 20–40 | Ten languages, chosen per run from their language (`w:lang`): English, Polish, Portuguese, Italian, Croatian, Turkish and Russian (spelling, grammar; hyphenation except Croatian), and Czech (#1048), Indonesian (#498) and Spanish (four varieties, #3360, #3553) from the installed Hunspell dictionaries with generic checks. The first four in detail: English word list (Moby plus our supplement of modern words such as "desktop", #520) and rule-based grammar; Polish against the installed Hunspell dictionary through a Hunspell-style affix engine, Polish grammar rules and TeX hyphenation patterns (2026-10-10); Portuguese (pt-BR) against the installed VERO Hunspell dictionary — the affix engine reads its `FLAG UTF-8` flags, continuation classes and forbidden spellings — with accent-first suggestions, conservative grammar rules and TeX patterns (2026-10-12); Italian (it-IT) against the installed Hunspell dictionary through the same affix engine, with accent-first suggestions and conservative grammar rules. Other languages are left unmarked. The custom dictionary persists across sessions (#40). Word ships 120 proofing tools for ~50 languages plus Editor (style refinements, similarity). Issues #25, #40, #100. |
| Mailings | 2 | 75% | 70% | 4–8 | Mail merge from CSV, fields, rules (IF, SKIPIF, NEXT, NEXTIF, evaluated alike in preview and merge), fields in headers/footers (one section per letter), preview, envelopes, labels. Select Recipients takes a typed list, CSV (#247) or a sheet of an Excel (.xlsx/.xlsm/.xls) or OpenDocument (.ods) workbook, with Select Table to pick the sheet (#334, #778); Edit Recipient List filters, sorts and finds (#778, #1576); Preview Results navigates records and Finish & Merge takes a record range (#3028); recipient labels continue across sheets (#4880). No Outlook/Contacts/Access data sources, no email merge. |
| Pictures, shapes, text boxes, WordArt | 6 | 68% | 60% | 20–30 | Insert, crop, recolour, styles, floating placement like Word (#136), wrap (square/top-bottom/behind/front), text boxes edit and overflow (#46), VML text boxes (#242), free rotation and flips with a rotation handle (#332), text-box text direction (90°, 270°, stacked), Align Text and linked text boxes (#369), Icons: an original icon library drawn in code with a searchable picker, inserted as pictures and recoloured with Icon Color (#309), Drawing Canvas with Fit/Expand/Scale and DOCX `wpc:wpc` (#344; members edited by command, not yet dragged inside the canvas; the canvas rotates and flips as a whole). Group and ungroup (#267), contour wrap (#354), shape gradients and transparency (#1500), WordArt fill, outline, shadow, glow, reflection and warps, and connectors glued to connection sites (#416) landed. Missing: text in a freely rotated text box, shape effects, WordArt 3-D/bevel, connector routing around shapes, online pictures, screenshot. |
| Equations | 1 | 80% | 75% | 2–3 | OMML read/write, OpenType MATH layout, in-place editor with an Equation tab (#191). Long display equations break at operators (#326). Missing: ink equations. |
| Charts, SmartArt, Draw/ink, 3D models, icons | 4 | 35% | 3% | 45–70 | Icons landed (#309; an original library drawn in code). Draw: pens, pencil, highlighter, stroke eraser and Hide Ink (#307), ink saved as DrawingML freeforms; Lasso Select (move, recolour, delete), Add Pen gallery, Ink to Shape (line, arrow, rectangle, ellipse, triangle, polygon) and Ink Replay (#395); Ink to Math, pressure and Word's own InkML missing; charts: Insert › Chart makes column, bar (clustered and stacked), line, area, pie, doughnut and scatter charts with Chart Design (change type, Edit Data grid, title, legend, data labels) and saves them as real chart parts (#409); charts from Word are drawn (#292) and kept on save (#319) but stay read-only; SmartArt is drawn and kept (#292, #319); Insert › SmartArt (#497) creates and edits list, process, cycle, hierarchy, pyramid, radial, matrix and Venn graphics with our own layouts, a Text Pane and the SmartArt Design tab, saved as diagram parts with the drawing; SmartArt from other programs stays read-only. |
| View (modes, zoom, panes, windows) | 3 | 76% | 72% | 3–5 | Print/web/draft/read/focus, dark page (#194), pages side by side when zoomed out (#251), navigation pane; Outline view with the Outlining tab: levels, promote/demote, moving with collapsed content, expand/collapse, Show Level, first line only, text formatting (#493); one window per open document, New Window and Switch Windows. Missing: View Side by Side, Synchronous Scrolling, Arrange All (#331 open), Master Document, Immersive Reader depth. |
| Backstage, printing, options | 3 | 66% | 58% | 8–14 | New/Home pages (#254), save prompts (#151), web printing via the browser (#209). Desktop printing goes straight to the printer through craft-print: printer, copies, collation, duplex, grayscale, page ranges and scaling to another paper size (#641, #2108); crash reports on File › Print stay open (#4333). Options in Word's panes (#485), still short of Word's depth. |
| Right-to-left and complex scripts | 1 | 58% | 50% | 7–11 | UAX #9, Arabic shaping, RTL paragraphs (#207), RTL tables and sections (#362), Hebrew and Arabic PDF text copies whole (#1383). No kashida justification or mirrored UI; open bug reports (#199, #66, #63, #48) need re-testing against #207. |
| East Asian typography | 1 | 40% | 25% | 11–19 | IME input works; native acceptance is open (the winit PRs #155–#164 closed with the egui 0.37 move). Vertical text in sections, text boxes and cells with upright ideographs (#491; no `vert` glyph alternates). Ruby/Phonetic Guide with DOCX round trip (#288); document grid (lines per page, characters per line, #391). Two lines in one and horizontal in vertical (`w:eastAsianLayout`) are kept on save but not laid out. Enclose Characters (#297), the Asian Typography tab (#323) and automatic Asian/Latin spacing (#2358) landed. |
| Accessibility | 1 | 50% | 50% | 8–14 | AccessKit enabled in eframe. Alt Text pane for pictures, shapes, text boxes, groups, charts and tables with Mark as decorative, all round-tripping in DOCX (#386). The Accessibility pane groups errors, warnings and tips with jump-to and one-click fixes; rules for alt text, heading order, header rows, blank table rows/columns, merged cells, low contrast (WCAG ratio), floating objects and blank paragraphs. Screen-reader access to the document canvas is still untested (#18); no reading-order tools. The document canvas describes its text, headings, lists, tables, links, pictures and caret to screen readers through AccessKit (#488); Windows Magnifier follows the caret, onto a new line too (#1119). |
| Agent control (CLI, MCP, control channel, macro record/playback) | 0 | beyond Word | 100%+ | — | Every action is a command id usable from CLI, MCP and a keyed control port. Word has VBA and Office.js instead (see Ecosystem). |

Weighted feature depth: **~74%** (Σ weight × parity / 100 over the table; the Mailings and Pictures rows were listed twice until 2026-10-31).

## Readiness numbers: full, mainstream practitioner, essentials user

Three numbers, each with its own hours. The
**full** number is the additive weighted sum over the dimension table below. **Mainstream** and
**essentials** average depth over the areas in scope, then multiply by written-down discounts for
what still stops real work. The stage follows the full number and the core-workflow gate.

| Audience | Ready | Opus 5.5 agent wall-clock hours to ~95% | Work that dominates |
|---|---|---|---|
| Full target (ready for real work) | **~65%** | **~530–900 h** (70–80% parallelizes) | Breadth: charts/SmartArt/Draw, localization and proofing languages, East Asian and RTL typography, `.doc` writing, PDF Reflow, ecosystem; plus everything below |
| Mainstream practitioner | **~61%** | **~280–450 h** (~65% parallelizes) | DOCX real-world fidelity, pagination fidelity, dialog depth, objects, stability |
| Essentials user | **~68%** | **~80–130 h** (~50% parallelizes) | Install/launch stability, opening files people send (corpus subset), printing stability, picture handling, English spelling depth |

Hours are calibrated as in [Calibration](#calibration-of-hours-from-this-repos-history) and are
subsets: essentials ⊂ mainstream ⊂ full. Mainstream to ~95%: its areas' depth (~145–225 h) plus
the file-exchange, stability and interaction work behind the discounts (~135–225 h). Essentials to
~95%: launch/install stability (15–25 h), opening received files (30–50 h), printing (3–5 h),
pictures and tables (12–18 h), spelling depth (8–12 h), polish (10–20 h).

### The discounts (evidence)

| Discount | Mainstream | Essentials | Evidence |
|---|---|---|---|
| Interaction fidelity | ×0.95 | — | Typing pinned to Word (`tests/typing.rs`), ribbon, keytips, mini-toolbar, customizable ribbon and keys; but 63 modal dialogs against Word's ~100, 164 of ~250 shortcuts, lines wrap differently without an Aptos-metric font |
| Discoverability and UI clarity | — | ×0.96 | Word-style ribbon a Word user already knows, command search, keytips, disabled controls that say why; some newer pane strings and tooltips terse or untranslated |
| Stability on real machines | ×0.90 | ×0.90 (launch and install) | Never-crash standard, hostile-param fuzzing, panic guard, local recovery and owned-temp saves; the early freezes and installer reports are closed (#77, #59, #31, #218, #64, #73, #106, #51); but 38 critical issues are open on 2026-10-31, with startup crashes (#170, #5058), File › Print crashes (#4333) and save failures (#4931, #3991). Unchanged: gains and new reports offset |
| Exchanging files with Word users | ×0.92 | ×0.94 (opening files people send) | Word opens our .docx; `.doc` import; charts, SmartArt, OLE, ink, content controls and revisions kept on save; but no real-world corpus test, and pagination differs (Aptos, no page-by-page comparison). Simple documents, which are most of what a casual user receives, come through well; #279 (opening Word documents) is still open |
| Product | **×0.787** | **×0.812** | |

### Full target (ready for real work): ~65%

The additive weighted sum over the [dimension table](#by-dimension): **0.30·74 + 0.20·63 + 0.15·60 +
0.10·70 + 0.10·50 + 0.05·80 + 0.04·38 + 0.03·95 + 0.01·45 + 0.02·15 ≈ 65%** (range 60–68%). No
multiplicative discounts: stability, file formats, layout and UI are weighted dimensions here.

### Mainstream practitioner: ~61%

The typical professional Word user (office, legal, academic, writing) in their own language, weekly
work only. Left out: equations, mail merge, Draw/ink, East Asian and right-to-left typography,
accessibility tools, proofing in other languages, add-ins/VBA, Copilot, cloud and co-authoring,
dictation and pens.

| Area | Weight | Depth |
|---|---|---|
| Typing, selection, clipboard, undo, find/replace | 17 | 87% |
| Character and paragraph formatting | 16 | 84% |
| Styles | 9 | 77% |
| Tables | 9 | 80% |
| Review (comments, track changes) | 9 | 77% |
| Lists | 8 | 80% |
| Page layout and sections | 6 | 75% |
| Headers, footers, page numbers, fields | 5 | 73% |
| Proofing in the user's language (English) | 5 | 60% |
| Pictures, shapes, text boxes | 5 | 68% |
| References (TOC, captions, citations) | 4 | 72% |
| Footnotes | 2 | 70% |
| Charts and SmartArt (pasted from Excel or received) | 2 | 35% |
| Print and PDF | 2 | 75% |
| View | 1 | 76% |

Base ≈ **77.5%**; **77.5% × 0.787 ≈ 61%**.

### Essentials user: ~68%

Someone who uses only the core of Word: create or open a document, type and format it, a list, a
picture, a simple table, spelling, undo, save, PDF or print, default settings. Advanced options,
professional workflows and file-exchange edge cases are left out, as is everything left out of
mainstream.

| Feature | Weight | Depth |
|---|---|---|
| Typing, selection, copy/paste, undo | 20 | 87% |
| Font, size, bold/italic/underline, colour, highlight | 15 | 90% |
| New, open, save, AutoSave, templates | 12 | 85% |
| Alignment, line spacing, indents | 8 | 87% |
| Bullets and numbering | 8 | 87% |
| Spelling (English) | 8 | 70% |
| Insert, move and resize a picture | 7 | 70% |
| Simple table | 6 | 82% |
| Heading styles from the gallery | 5 | 80% |
| Print and export PDF | 5 | 80% |
| Find and replace | 3 | 88% |
| Page numbers and a simple header | 3 | 78% |

Base ≈ **83.4%**; **83.4% × 0.812 ≈ 68%**.

### User evidence (GitHub)

Counts on 2026-10-31: 1,214 open issues and 1,052 closed; 178 open issues labelled bug, 38 critical,
152 high priority, 46 security, 35 parity. 1,079 PRs open, 1,416 merged. These include audit findings
and agent-filed work, so they aren't a measure of outside users; the classification below has not
been redone since 2026-10-10.

2026-10-10 (no issue was filed by a storytold org member):


- **120 issues** filed by 80+ outside users in 5 days; about 60 closed, most fixed within a day.
- **Praise:** 4 issues or comments ("Awesome job guys!" #28, "amazing how much it advanced" #146,
  "appreciate the repo" #57, thanks in #73/#51). **"Switched from Word" reports: 0.**
- **Open issues by kind** (hand-classified, ~56 open at classification): **~21 core-path bugs**
  (install/launch: #64, #73, #106, #51, #47, #170, #218; hangs: #77, #59, #31; editing: #82, #142,
  #44, #45, #217, #95; files and saving: #187, #195, #279, #18; printing #15; several may already
  be fixed on main), **~11 right-to-left and language** reports (#19, #48, #63, #66, #199, #211,
  #215, #25, #40, #56, #100), **~24 feature requests or minor bugs** (catalog parity, macros,
  passwords, cloud, theming).
- Reading: people are trying WordCraft as a Word replacement and report core-path problems more
  than niche ones; the install and stability discount is earned.

## By dimension

Weights are our estimate of how much each dimension decides whether a professional can switch from
Word. Their weighted sum is the **full ready for real work** number.

| Dimension | Weight | Parity | Hours to full | Doc | Evidence |
|---|---|---|---|---|---|
| Features (depth, table above) | 30% | 74% | 145–235 | this file | Weighted table above. Hours: the area table sums to 193–324 h; the Page layout, Footnotes, RTL and East Asian rows (29–49 h) are counted under Layout and the Proofing row (20–40 h) under Localization, so they aren't counted twice |
| File formats (DOCX fidelity first) | 20% | 63% | 175–287 | [file-format-parity.md](file-format-parity.md) | DOCX opens in Word and round-trips our own tests, but no real-world corpus test yet; charts, SmartArt, OLE, ink, content controls and revisions are kept on save and encrypted packages open; `.doc` can't be written |
| Layout and pagination fidelity | 15% | 60% | 61–108 | [layout-parity.md](layout-parity.md) | Checklist: 38 ✅, 27 🟡, 2 ❌. Column balancing, footnote continuation and the document grid landed; Aptos has no metric-matched substitute so lines break differently from Word; no page-by-page comparison; compatibility modes beyond 15 partial |
| UI/UX fidelity | 10% | 70% | 44–71 | [ui-parity.md](ui-parity.md) | Ribbon, keytips, mini-toolbar, Backstage, ribbon and keyboard customization, 164 shortcuts; 63 modal dialogs against Word's ~100 |
| Stability | 10% | 50% | 25–45 | [gaps.md](gaps.md) | Never-crash standard, hostile-param fuzzing and a security-audit hardening pass in place, with recovery for every open document and owned-temp saves; but 38 critical issues open on 2026-10-31, among them startup crashes (#170, #5058) and File › Print crashes (#4333), and no longer soak test. Unchanged from 2026-10-10: gains and new reports offset |
| Performance | 5% | 80% | 10–20 | [layout-parity.md](layout-parity.md) | 61 ms cold layout and 1.4 ms relayout on a 188-page document (measured 2026-10-06, not re-run); font picker freezes fixed in #233; typing no longer starts and stops render threads on every repaint (#3027); scaling fixes for many relationships (#3411), long unbroken words (#3412), many content controls (#13), scrolling a 1,096-page table (#811); no large real-world documents measured |
| Localization | 4% | 38% | 70–115 + native review | [localization-parity.md](localization-parity.md) | 27 UI languages (Word 30); 9 of the 12 key languages shipped, none but English `full`; 10 proofing languages (Word ~50); no mirrored UI |
| Platforms | 3% | 95% | 5–10 | [ROADMAP.md](../ROADMAP.md) | macOS (universal, signed), Windows x64/x86/arm64, Linux (AppImage/deb/rpm/Flatpak), FreeBSD, web. Beyond Word on Linux, BSD and the web; Word's iPad/iPhone/Android apps are out of scope |
| Hardware | 1% | 45% | 20–40 | [hardware-parity.md](hardware-parity.md) | Direct desktop printing without tray options or pages per sheet; ink without pressure; no dictation; GPU drawing and HiDPI fine. Ink hours are in the Features row (Draw tab) |
| Ecosystem and plugins | 2% | 15% | 40–80 | [gaps.md](gaps.md) | Zotero built in, macros record/play commands, `.docm` macros preserved (#172). No VBA execution, no Office add-ins, no EndNote/Mendeley plugins, no cloud storage or co-authoring |
| AI features | 0% | 40% | 15–30 | [gaps.md](gaps.md) | Beyond Word for external agents (MCP, CLI). No in-app writing assistant like Copilot yet (chat PR #178 open). Weight 0: Copilot is a paid add-on and not what decides a switch |

**Ready for real work ≈ 0.30·74 + 0.20·63 + 0.15·60 + 0.10·70 + 0.10·50 + 0.05·80 + 0.04·38 +
0.03·95 + 0.01·45 + 0.02·15 ≈ 65%.**

## Methodology

- **The 2026-10-31 re-rating** kept the method, weights and discounts' structure of the 2026-10-10 pass
  and changed the percentages only where landed work is on record in this file, the gap list or
  the parity checklists. It is a desk review of main at v0.13.2: counts were measured, the
  percentages are judgement, Word was not re-measured and no real-world file was compared.
  Audience hours are the 2026-10-10 figures reduced by what landed, not derived anew.
- **Measured** numbers come from the source on origin/main: the catalog/registry diff (the same
  computation as `cargo xtask parity`), `#[test]` counts, i18n catalog rows, reader/writer element
  coverage (`grep` over `crates/docx`, `crates/docbin`, `crates/formats`) and the Word bundle
  listing. Nothing was built or run this pass (disk budget).
- **Estimated** numbers are judgements from that evidence plus Word's documented feature set. The
  previous estimate (~62% "real feature parity", 2026-10-06) measured feature depth only. This pass
  splits it into feature depth (67%, up: equations, .doc import, RTL, text boxes, Zotero landed) and
  ready-for-real-work (60%, additive weighted sum), which adds file fidelity, stability,
  localization and the rest. A multiplicative version (~47%) was briefly used the same day and
  reverted; no new evidence.
- **Why the hours went up** from "120–150 h to 100%" (2026-10-06): that figure left out
  localization, multilingual proofing, ecosystem, hardware, PDF Reflow and `.doc` writing, and
  priced DOCX corpus fidelity at 10 h. The new evidence is the Word bundle inventory (30 UI
  languages, 120 proofing tools, 12 bibliography styles), the 62 open issues as of 2026-10-10 (crashes, freezes,
  RTL reports) and the size of comparable work actually done (below).

### Calibration of hours (from this repo's history)

The repo is five days old (first commit 2026-10-05 20:23, `40c5d51` 2026-10-10): 124
commits, 95 merged PRs, ~85,600 lines of Rust. PRs are written by agents, so their size and
open-to-merge time bound the agent time:

| PR | Work | Lines added | PR open → merged |
|---|---|---|---|
| #191 | Equations: OMML import/export, math layout, editor and Equation tab | 9,666 | 5.2 h |
| #145 | Word 97-2003 `.doc` reader (spec-based) | 4,488 | 10.2 h |
| #189 | Zotero integration with a ribbon tab | 3,043 | 4.7 h |
| #207 | Right-to-left and Persian text (bidi, shaping, DOCX) | 2,048 | 4.2 h |
| #46 | Text boxes and floating objects (edit, drag, wrap, overflow) | 2,628 | 36.1 h (incl. review wait) |
| #136 | Place floating objects the way Word does (fidelity work) | 574 | 10.8 h |

Rule of thumb used: **a new, self-contained subsystem runs ~600–1,000 tested lines per agent hour**
(equations, Zotero, RTL: 4–10 h each); **fidelity work against Word's observed behaviour runs
~5–10× slower per line** (#136, #138, #104: 0.5–1 h per 100 lines, mostly comparing and testing).
Most of the remaining gap is the second kind, which is why the hours are high relative to the
code already written. About 70–80% of the hours parallelize across agents by area (formats, layout,
objects, localization, proofing are independent crates). Human time is needed for: native-speaker
review of translations, owner decisions (cloud/co-authoring scope, AI provider, VBA scope), a
licensed or commissioned metric-compatible Aptos substitute, and a real-world DOCX corpus that
can be used without committing Word output.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-31 | minor | Re-rated against main at v0.13.2: measured counts; feature depth 68% → 74%, full 60% → 65%, mainstream 55% → 61%, essentials 63% → 68%; beta ~150–230 h → ~135–230 h, full parity ~750–1,250 h → ~600–1,050 h; duplicate Mailings and Pictures rows removed; stale evidence corrected. Desk review only: no real-world comparison and no new Word measurement |
| 2026-10-29 | trivial | Recipient labels continue across product-sized sheets instead of stopping after the first sheet (#4880). Filtered/sorted order, product geometry and single-label mode retained; readiness estimates unchanged. |
| 2026-10-18 | trivial | References: Insert Table of Figures dialog; a table of figures lists any caption label (`TOC \c`), without label and number (`\a`), without page numbers (`\n`) or with them after the text (`\p`); Word's switch order is recognised and the leader kept on update (#3013); no percentage change |
| 2026-10-15 | trivial | Pictures, shapes, text boxes: Insert › Shapes adds the new shape beside the selection, as Word draws it, so a second shape no longer replaces the first (still selected after its insert) and selected text stays; a picture still replaces a selected picture, as in Word (#2019) |
| 2026-10-19 | trivial | Performance row: render threads reused while typing (#3027) |
| 2026-10-15 | trivial | Pictures, shapes, text boxes: Picture Styles, Picture Border and Artistic Effects draw again from the picture under them, so a second style replaces the first instead of framing the frame (Thick Frame then Simple Frame is a simple frame), a border replaces the border and keeps the style, a style replaces the border, and an effect replaces the effect (None removes it), as in Word; transparency was already absolute (#2052) |
| 2026-10-18 | trivial | References row: Manage Sources imports Word's `Sources.xml` bibliography file (#2640); no percentage change |
| 2026-10-16 | trivial | East Asian typography row: `w:eastAsianLayout` kept on save, not laid out; no percentage change |
| 2026-10-15 | trivial | Equations: a script or limit after `_` or `^` is the whole run of letters and digits (`V_dd`, `x^ab`), as in Word for Mac 16.113.4, and Linear format writes it bare (#1609) |
| 2026-10-15 | trivial | Tables: Cell Margins (#1960) opens initialized from table defaults and applies edits only after confirmation; per-cell margins remain intact |
| 2026-10-15 | trivial | Pictures, shapes, text boxes: Insert › Shapes adds the new shape beside the selection, as Word draws it, so a second shape no longer replaces the first (still selected after its insert) and selected text stays; a picture still replaces a selected picture, as in Word (#2019) |
| 2026-10-15 | trivial | Pictures, shapes, text boxes: Picture Styles, Picture Border and Artistic Effects draw again from the picture under them, so a second style replaces the first instead of framing the frame (Thick Frame then Simple Frame is a simple frame), a border replaces the border and keeps the style, a style replaces the border, and an effect replaces the effect (None removes it), as in Word; transparency was already absolute (#2052) |
| 2026-10-16 | trivial | References: Insert Caption… on the right-click menu of pictures, shapes, text boxes and charts, captions in the object's own cell (#1230); no percentage change |
| 2026-10-12 | minor | Czech proofing review: short-word splits rank after deletion, NOSUGGEST excludes suggestions while accepting typed forms, affix comments apply only to headers, ratios keep spaces and repeated-word messages use Czech quotes; synthetic engine end-to-end and latency checks |
| 2026-10-12 | minor | Czech spelling from the installed Hunspell dictionary, runtime-table suggestions and generic checks; Czech interface writes cs-CZ documents; no Czech hyphenation patterns bundled; estimates unchanged |
| 2026-10-13 | trivial | Windows Magnifier follows the caret onto a new line (#1119): Accessibility row updated; no percentage change |
| 2026-10-14 | trivial | Tables: nesting stops at 24 levels, the depth .docx keeps (#1596); Tables row updated, no percentage change |
| 2026-10-12 | minor | Italian proofing: it-IT spelling, accent-first suggestions and conservative grammar; Proofing row updated, 42% → 44% |
| 2026-10-12 | trivial | The 62 open issues and ~45 open PRs cited as evidence are dated 2026-10-10 (the measurement pass); not re-counted (197 issues were open on 2026-10-12) |
| 2026-10-11 | trivial | Tables: Ctrl, Alt, left-edge and selected-cells border drags (#711); Tables row updated, no percentage change |
| 2026-10-11 | trivial | Tables: column and row markers on the rulers (#700); Tables row updated, no percentage change |
| 2026-10-11 | trivial | Tables: the table context menu, its items following the selection as in Word (Delete Cells, Rows, Columns or Table, Merge and Split Cells, Distribute, AutoFit, Text Direction, Insert Caption…, Table Properties…); Tables row updated, no percentage change |
| 2026-10-11 | trivial | Tables: ordinary drags, Shift+arrows and Shift+click over cells select whole cells, as in Word; Tables row updated, no percentage change |
| 2026-10-11 | trivial | Tables: selection areas for columns, rows and cells, "+" insert buttons, the move handle and double-click AutoFit on the page; Select Column (catalog 484/499); Tables row updated, no percentage change |
| 2026-10-12 | minor | Portuguese proofing (pt-BR spelling from the installed VERO dictionary, accent-first suggestions, grammar, hyphenation; Review › Language gains Português (Brasil)): Proofing 40% → 42%; other dimensions unchanged at this precision |
| 2026-10-12 | trivial | Proofing: custom dictionary persists across sessions (#40); no percentage change |
| 2026-10-11 | trivial | Proofing: runs in a language without a dictionary are skipped instead of flagged against the English dictionary (#100); no percentage change |
| 2026-10-11 | minor | Polish interface (13 UI languages) and Polish proofing with per-run languages: Proofing 35% → 40% (25–45 h → 20–40 h), Localization 25% → 27%; feature depth and readiness unchanged at this precision |
| 2026-10-11 | trivial | Tables: column and row borders and the resize handle drag on the page (#217); Tables row and interaction-fidelity evidence updated, no percentage change |
| 2026-10-11 | trivial | Review: Show revisions in balloons takes tracked deletions in the body out of the text into "Deleted: …" balloons (it drew like the comments-and-formatting mode before); Review row updated, no percentage change |
| 2026-10-11 | trivial | Insert › SmartArt (#497): charts/SmartArt row updated; catalog 429/451 → 437/458 (95.4%); no percentage change |
| 2026-10-11 | trivial | Outline view depth (#493): View row updated; catalog 428/450 → 441/463 (95.2%) with the Outlining tab rows |
| 2026-10-11 | trivial | Screen readers can read the document canvas (#488): Accessibility row updated; no percentage change |
| 2026-10-11 | trivial | Compare settings, Restrict Editing pane and AutoCorrect Options (#412): Review row updated; no percentage change |
| 2026-10-11 | trivial | Pictures, shapes, text boxes, WordArt: WordArt text effects and Transform warps, connectors glued to shapes (#416); catalog 423/448 → 428/453 (94.5%); no percentage change |
| 2026-10-11 | trivial | Content controls kept, shown and edited, Developer › Controls (#381); catalog +11 rows, all live (402/434 → 413/445, 92.8%); no percentage change |
| 2026-10-11 | trivial | Insert › Chart (#409): charts row updated, catalog 408/439; no percentage change |
| 2026-10-11 | trivial | Accessibility 35% → 50%: Alt Text pane for every object and tables, decorative flag, deeper checker with an Accessibility pane (#386); weight 1, headline unchanged |
| 2026-10-11 | trivial | References: Caption, Index, Mark Entry, Custom Table of Contents and Create/Edit Source dialogs, with the field switches they write honoured on update (#402); no percentage change |
| 2026-10-11 | trivial | Move tracking (#400): Review row updated; no percentage change |
| 2026-10-11 | trivial | Footnotes and endnotes: Footnote and Endnote dialog, options per section (position, format, start, restart each section/page), convert notes (#385); no percentage change |
| 2026-10-11 | trivial | Pictures, shapes, text boxes: text direction, Align Text and linked text boxes (Create Link / Break Link), DOCX `bodyPr` `vert`/`anchor` and `wps:linkedTxbx` (#369); no percentage change |
| 2026-10-10 | trivial | Drawing Canvas (#344): Insert › Shapes › New Drawing Canvas and Draw › Drawing Canvas live; catalog 402/434 → 404/434 (93.1%) |
| 2026-10-11 | trivial | Mailings: Excel and OpenDocument spreadsheets as recipient lists (#334); score unchanged |
| 2026-10-11 | trivial | Insert › Icons (#309): catalog 402/434 → 403/434 (92.9%); moved from missing to done under pictures and shapes |
| 2026-10-11 | trivial | Enclose Characters (#297): catalog 402/434 → 403/434 (92.9%) |
| 2026-10-11 | trivial | Phonetic Guide (ruby) landed (#288): catalog +1 feature |
| 2026-10-11 | trivial | Shortcut count 101 → 125 (#330) |
| 2026-10-11 | trivial | Equation fixes: removing numbers keeps every row (#444), caret moves keep imported OMML (#446), build-up on a caret move is an undoable edit (#447), LaTeX `\text{}` keeps its spaces (#448); no percentage change |
| 2026-10-11 | trivial | English spelling (#520): common modern words ("desktop", "download", "email"…) recognised via our supplement list; Proofing row updated, no percentage change |
| 2026-10-12 | minor | Local desktop recovery implemented; original files are protected, password sessions skip plaintext recovery. Crash prevention and broader field/soak validation remain. No change to stability or readiness estimates. |
| 2026-10-11 | trivial | References: all 12 of Word's bibliography styles (eight more: Harvard, ISO 690 ×2, Turabian, GB/T 7714, GOST ×2, SIST02) (#383); no percentage change |
| 2026-10-11 | trivial | Mail merge fixes (#427–#430): SKIPIF drops matching records, NEXT advances later fields, IF previews, header/footer fields merge; Mailings row updated, no percentage change |
| 2026-10-11 | trivial | Formatting revisions (#41): Review row updated; no percentage change |
| 2026-10-16 | trivial | Font dialog: Kerning for fonts and its Points and above size; Character and paragraph formatting row updated, no percentage change |
| 2026-10-14 | trivial | Ink to Shape recognises single-stroke arrows |
| 2026-10-11 | trivial | #395: Lasso Select, Add Pen, Ink to Shape and Ink Replay; catalog 423/448 → 427/448 (95.3%) |
| 2026-10-10 | trivial | #307: Draw tab pens, eraser and Hide Ink; catalog 384/430 → 390/430 (90.7%) |
| 2026-10-11 | trivial | Vertical sections (#491): Page layout and East Asian rows updated; no percentage change |
| 2026-10-10 | trivial | Draw Table and Eraser (#303): catalog 384/430 → 387/430 (90.0%) |
| 2026-10-10 | trivial | Pictures, shapes, text boxes: free rotation and flips, rotation handle, rotated wrap bounds, DOCX `a:xfrm` rot/flipH/flipV (#332); no percentage change |
| 2026-10-10 | minor | Merged main (12 PRs): Paste Special, Advanced Find, Column Selection, Style Inspector, custom table styles, gridlines, zoom buttons, touchpad scrolling, File Info fields, Serbian. Catalog 378/428 → 384/430 (89.3%); typing 82→85%, styles 72→74%, tables 74→76%; feature depth 67→68%; mainstream 54→55%; full stays ~60%, essentials ~63% |
| 2026-10-10 | minor | Readiness table: hours to ~95% per audience (full ~650–1,100 h, mainstream ~300–500 h, essentials ~90–140 h). Full number restored to the additive weighted sum, ~47% → ~60%: no new evidence; beta distance back to ~15 points and ~150–230 h |
| 2026-10-11 | trivial | East Asian typography 15% → 25%: document grid (#391); kinsoku and Asian Typography options already landed |
| 2026-10-10 | minor | Added mainstream practitioner (~54%) and essentials user (~63%) numbers with written weights and discounts; full ready-for-real-work recomputed with the same multiplicative discounts: 60% → ~47% (method change, not new product evidence; stage stays alpha: above 40% and the gate passes); beta now ~450–700 h to the 75% bar; user-evidence counts from GitHub |
| 2026-10-10 | minor | Stage checked against the core-workflow alpha gate: passes, stays alpha |
| 2026-10-10 | major | Created from ROADMAP.md's "Parity by area" and "Estimate to 100%" sections; full re-measure against Word for Mac 16.113.4 (bundle listing, catalog diff from source, issue tracker); split into feature depth (67%) and ready for real work (60%); added proofing, East Asian, accessibility areas and the dimension table; hours re-calibrated from PR history |
| 2026-10-06 | major | First estimate in ROADMAP.md: ~62% real feature parity, 120–150 h to 100% |
| 2026-10-29 | trivial | Tables row: Table Properties' row, column and cell settings apply to the selection, as in Word (#4857); no percentage change |
