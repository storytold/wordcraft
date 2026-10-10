# Where WordCraft falls short of Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** minor (beta distance restated: ~15 points, ~150–230 h; previously: first version; every known shortfall from the 2026-10-10 re-measure, ranked) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

The work list. One entry per shortfall, ranked by how much it stops a Word user from switching
(**B** = blocks beta). Each says what's missing, the evidence, who it hurts, an Opus 5.5 agent
wall-clock estimate, and which parity document tracks it. Agents: pick from the top, check open PRs
first (`gh pr list`), and update this file and the parity doc when a gap closes.

## Ranked

| # | Gap | B | Evidence | Impact | Hours | Doc |
|---|---|---|---|---|---|---|
| 1 | **DOCX never tested on real-world files.** No corpus of files from Word, Google Docs, LibreOffice and Pages has been opened, rendered and round-tripped and compared with Word page by page | B | No corpus in the repo; `crates/docx/tests` are synthetic fixtures; "DOCX fidelity corpus: not started" since 2026-10-06 | Everyone: the files people receive are the files that break | 30–50 | [file-format-parity.md](file-format-parity.md) |
| 2 | **Charts, SmartArt, OLE objects, ink and group shapes are dropped on open and lost on save, silently** | B | No `c:chart`, `dgm`, `wpg`, `w:object` handling in `crates/docx` | Business and academic documents lose content without warning | 10–15 to preserve and show fallback pictures; 45–70 to render and edit charts and SmartArt | [file-format-parity.md](file-format-parity.md) |
| 3 | **Crashes and freezes in the field**: startup crash on Intel UHD Graphics (#170), freezes (#77, #59 Thai fonts, #31 font selector), no writes without admin on Windows 11 (#218), broken shape tool (#142), images can't be moved (#82) | B | 62 open issues, ~15 of them crashes, hangs or data-path failures | Anyone hit loses work or trust; the never-crash standard covers panics, not hangs or GPU driver faults | 20–30 | [target-app-parity.md](target-app-parity.md) (Stability) |
| 4 | **Pagination differs from Word**: Aptos has no metric-matched substitute; columns don't balance; long footnotes don't continue; legacy compatibility options ignored; no page-by-page comparison harness | B | [layout-parity.md](layout-parity.md); [typing-parity.md](typing-parity.md) Known gaps | Page counts and line breaks differ, which matters for forms, legal and academic work | 30–50 + owner (font) | [layout-parity.md](layout-parity.md) |
| 5 | **Formatting revisions, move tracking and content controls** are lost (`w:rPrChange`/`w:pPrChange` absent; `w:sdt` unwrapped) | B | Issue #41; source grep | Legal and editorial review workflows; templates with form controls | 12–18 | [file-format-parity.md](file-format-parity.md) |
| 6 | **No native printing** on desktop (PDF export only); web prints through the browser (#209) | B | `file.print` exports PDF; issue #15 | Everyone who prints | 6–10 | [hardware-parity.md](hardware-parity.md) |
| 7 | **Dialog depth**: 16 modal dialogs vs Word's ~100 (Tabs, Borders and Shading, Columns, Symbol, Field, Define Multilevel List, Table Properties, Track Changes Options, Options panes…) | B | `Dialog` enum in `crates/ui-egui/src/dialogs.rs` | Power users reach for dialogs the ribbon alone doesn't cover | 20–30 | [ui-parity.md](ui-parity.md) |
| 8 | **Proofing is English only**; no language per run, no dictionaries for other languages | | `crates/proof` has one dictionary; Word ships 120 proofing tools; issues #25, #40, #100 | Every non-English writer | 25–45 | [localization-parity.md](localization-parity.md) |
| 9 | **Password-protected files** can't be opened or saved | | #55; `docbin` detects and refuses RC4/XOR | Corporate and legal users | 6–10 | [file-format-parity.md](file-format-parity.md) |
| 10 | **Objects**: no group (#267 open), rotation handle, contour (tight/through) wrap, shape effects, WordArt, connectors, Drawing Canvas, Icons | | [parity-checklist.md](parity-checklist.md) (Shape Format 2/6, Layout › Group) | Newsletters, flyers, reports with diagrams | 20–30 | [target-app-parity.md](target-app-parity.md) |
| 11 | **Interface languages**: 8 of the 12 key languages missing (hi, ar, fr, id, de, ko, vi, pt-PT); no mirrored RTL interface; ~5% of strings still English in the six translated catalogs | | [localization-parity.md](localization-parity.md) | Most of the world's writers | 50–80 | [localization-parity.md](localization-parity.md) |
| 12 | **Draw tab / ink**: all 11 commands missing | | [parity-checklist.md](parity-checklist.md) (Draw 0/11) | Pen and tablet users | 15–25 | [hardware-parity.md](hardware-parity.md) |
| 13 | **Charts and SmartArt editing** (own renderer, chart data editing, SmartArt layouts) | | `insert.chart`, `insert.smartArt` missing | Reports and business documents | 45–70 | [target-app-parity.md](target-app-parity.md) |
| 14 | **`.doc` can't be written; metafile pictures and Word 6/95 files refused** | | `crates/docbin` is read-only | Users exchanging files with old Word or legacy systems | 25–40 | [file-format-parity.md](file-format-parity.md) |
| 15 | **References depth**: 4 of Word's 12 bibliography styles; sources not saved in DOCX (#169 open); no EndNote/Mendeley desktop integration | | Word's `Resources/Style` lists 12 styles | Students and researchers | 10–15 | [target-app-parity.md](target-app-parity.md) |
| 16 | **Right-to-left completeness**: RTL sections and tables, kashida, RTL in HTML/ODT/RTF; eight older RTL bug reports to re-verify after #207 (#215, #211, #199, #66, #63, #48, #19) | | [layout-parity.md](layout-parity.md) | Arabic, Persian, Hebrew writers | 10–15 | [layout-parity.md](layout-parity.md) |
| 17 | **East Asian typography**: vertical text, ruby/Phonetic Guide, Enclose Characters, document grid, Asian Typography options | | Catalog misses `format.phonetic`, `format.enclose`, `para.asianTypography` | Chinese, Japanese and Korean documents | 20–30 | [layout-parity.md](layout-parity.md) |
| 18 | **Screen-reader access** to the document canvas untested | | AccessKit enabled; no tests | Blind and low-vision users; public-sector procurement | 10–15 | [ui-parity.md](ui-parity.md) |
| 19 | **RTF and ODT depth**: notes, comments, revisions, sections, RTL | | ~1,500 lines each in `crates/formats` | Users exchanging with LibreOffice or older tools | 18–27 | [file-format-parity.md](file-format-parity.md) |
| 20 | **Performance on large real-world documents** unmeasured (500+ pages, many pictures) | | Only the 188-page sample benchmark | Thesis and book authors | 10–20 | [layout-parity.md](layout-parity.md) |
| 21 | **Mail merge data sources**: no Excel, Outlook/Contacts, email merge; CSV and typed lists only (#247) | | | Office administrators | 6–10 | [target-app-parity.md](target-app-parity.md) |
| 22 | **Ecosystem**: no VBA execution (macros preserved only), no Office add-ins, no cloud storage, no real-time co-authoring | | `.docm` keeps macros (#172) | Enterprise users with macro templates; teams | 40–80 + owner decisions | [target-app-parity.md](target-app-parity.md) |
| 23 | **PDF Reflow** (open a PDF as an editable document, Word for Windows) | | | Users who edit PDFs they receive | 30–50 | [file-format-parity.md](file-format-parity.md) |
| 24 | **Customize Ribbon / Keyboard**, Quick Access Toolbar customization; ~100 of Word's ~250 shortcuts | | [ui-parity.md](ui-parity.md) | Power users | 10–16 | [ui-parity.md](ui-parity.md) |
| 25 | **In-app AI assistant** (Copilot-style draft, rewrite, summarize) | | Chat PR #178 open | Users expecting Copilot | 15–30 + owner (provider) | [target-app-parity.md](target-app-parity.md) |
| 26 | **Minor formats**: Flat OPC XML, Word 2003 XML, MHT, Works/WordPerfect | | | Rare | 20–35 | [file-format-parity.md](file-format-parity.md) |
| 27 | **View windows**: Side by Side, Synchronous Scrolling, Arrange All, Switch Windows | | [parity-checklist.md](parity-checklist.md) (View 25/29) | Comparing documents | 3–5 | [ui-parity.md](ui-parity.md) |
| 28 | **Dictate** | | `tools.dictate` missing | Dictation users | 8–15 + owner (speech model) | [hardware-parity.md](hardware-parity.md) |
| 29 | **Equations**: long display equations don't break across lines; ink equations | | #191 notes | Maths-heavy documents | 3–4 | [layout-parity.md](layout-parity.md) |

**Beta needs #1–#7** (~150–230 h with the stability and layout work they imply): they are the
blocking gaps, and closing them lifts file formats, layout, stability and UI enough to take the
full number from ~60% to ~75%. Everything else is depth on the way to full parity.

## By kind

The same gaps grouped the way the parity documents are, for agents working in one area.

- **Feature gaps:** #2 (preserve), #5, #10, #12, #13, #15, #17, #21, #25, #27, #28, #29; Clipboard
  pane, Manage Styles, Group, Draw Table (open PRs exist for the first three — review and merge
  before re-implementing). Paste Special, Advanced Find, Column Selection, Style Inspector and
  custom table styles landed on 2026-10-10.
- **UI/UX gaps:** #7, #18, #24; rotation and crop handles on the canvas; table column drag on the
  ruler (#49, #217); context menus for objects and tables; Linux title bar theming (#78).
- **File-format gaps:** #1, #2, #5, #9, #14, #19, #23, #26; embedded fonts; glossary/building blocks.
- **Hardware gaps:** #6, #12, #28; scanner/Continuity Camera insert; KDE soft text (#140).
- **Localization gaps:** #8, #11, #16, #17; native-speaker review of every catalog (human).
- **Stability and performance gaps:** #3, #20.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | trivial | Merged main: landed features removed from the feature-gap list |
| 2026-10-10 | minor | Beta note restated: full number back to the additive ~60% (method aligned with the standard, no new evidence) |
| 2026-10-10 | major | First version, from the full re-measure: 29 ranked gaps with evidence, impact and hours |
