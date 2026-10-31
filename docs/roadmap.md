# WordCraft roadmap: milestones and what's next

> **Last reviewed:** 2026-10-31 · **Last updated:** 2026-10-31 · **Change:** minor (Current focus, alpha-gate evidence and milestones brought up to v0.13.2; beta ~135–230 h); previously trivial (HTML import keeps preformatted lines with CR, CRLF or LF input newlines (#4810); numeric character references retain their separate meaning; readiness estimates unchanged; previously trivial: egui 0.37 migration: 27 interface languages remain pinned by `craft-i18n.rev` (9205108…), and the input-method handling has synthetic regressions while managed CI passed and native acceptance (#155–#164) remains open; previously trivial: M6: Manage Sources imports Word's `Sources.xml`, #2640; previously minor: Czech proofing review fixes; previously minor: Portuguese proofing; previously minor: Polish interface and Polish proofing; previously trivial: M7: formatting revisions landed, #41; previously trivial: M5: Draw Table and Eraser landed, #303; previously minor: beta distance restated: ~15 points, ~150–230 h; previously: alpha gate checked: six core workflows pass; previously major: milestones moved here from ROADMAP.md; Current focus set from the re-measured gaps) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

Forward-looking plan. The summary is [`ROADMAP.md`](../ROADMAP.md); the ranked work list is
[`gaps.md`](gaps.md); the numbers are in [`target-app-parity.md`](target-app-parity.md). Hours are
Opus 5.5 agent wall-clock.

## Current focus (toward beta, ~135–230 h)

1. **Triage the 1,079 open PRs** (2026-10-31; ~45 on 2026-10-10) before starting overlapping work:
   several close gaps (Hebrew RTL #20, Norwegian #210, Arabic #737, #1042, #1032, RTF and ODT notes
   #1056, #1057, View windows #331). Owner review needed.
2. **Stability sweep** (gap #3, 25–45 h): the 38 critical issues, led by startup crashes (#170,
   #5058), File › Print crashes (#4333) and save failures (#4931, #3991), plus an autosave/recovery
   soak test. Local desktop recovery is implemented; cross-platform field validation and longer
   soak testing remain. See [recovery.md](recovery.md).
3. **DOCX real-world corpus** (gap #1, 30–50 h): collect openly licensed real-world .docx files,
   open/render/round-trip them, compare with Word locally, fix what breaks. Omitted/empty
   paragraph defaults now have synthetic save/reopen coverage; the corpus gate remains open.
4. **Pagination fidelity** (gap #4, 30–50 h): compat options, a local page-by-page comparison
   harness; an owner decision on an Aptos-metric font. Column balancing (#350) and footnote
   continuation (#352) landed.
5. **Word's ink and OLE** (gap #2, 10–15 h): draw native ink from its strokes and open OLE objects.
   Charts, SmartArt, OLE and every native ink form already survive a save.
6. **Remaining depth**: content controls and revisions (gap #5, 5–8 h), print dialog (gap #6,
   5–8 h), dialogs (gap #7, 11–19 h).

## Alpha gate (core workflows)

The core-workflow gate: what a typical Word user does
every day, checked end to end on the main platform (macOS desktop), including save and reopen.
"Partial" here means rough but not blocking: the workflow completes. Last run 2026-10-10; the
evidence column was brought up to date from the source on 2026-10-31 without re-running the gate.

| Core workflow | Works end to end? | Evidence | Hours to pass |
|---|---|---|---|
| Write and format a document: type, fonts, paragraph formatting, styles, lists, undo, find/replace; save as .docx and reopen | yes | 629 commands, typing parity pinned by `tests/typing.rs`, DOCX round-trip tests (`crates/docx/tests/it/roundtrip.rs`) | 0 |
| Open a .docx someone sent, edit it, send back a .docx Word opens | partial (not blocking) | Word opens our files; typical text/table/picture documents work; never tested on a real-world corpus; charts, SmartArt, OLE, ink and moves kept on save (gaps #1, #2) | 0 (beta work: 40–65) |
| Build a structured document: tables, pictures, headers/footers, page numbers, sections, columns | yes | Tables with row splitting and floating tables (#137), floating pictures (#136), first/odd/even headers; columns balance before a continuous break (#350) | 0 |
| Review with others: comments, track changes, accept/reject, compare | yes | Margin balloons, tracked insert/delete incl. paragraph marks (#125, #244), formatting revisions (#41), compare | 0 |
| References for reports and papers: TOC, footnotes, citations and bibliography, captions | yes | Updatable TOC field (#52), notes, APA/MLA/Chicago/IEEE, Zotero (#189); long footnotes continue (#352) | 0 |
| Share or print: export PDF, print | partial (not blocking) | PDF export solid (krilla); direct desktop printing through craft-print (gap #6), with crash reports open (#4333) | 0 (beta work: 5–8) |

**Result: passes.** Every core workflow completes on macOS and the work saves and reopens; the
partial rows are fidelity gaps (beta work), not broken workflows. WordCraft stays **alpha**.

## Milestones

| # | Milestone | State | Remaining |
|---|---|---|---|
| M0 | Skeleton and vertical slice (model, layout, render, engine, Word-style UI, CLI, MCP, web) | done | — |
| M1 | DOCX I/O | done (first version); charts, SmartArt, OLE and ink kept on save; real-world corpus not started | 40–65 h |
| M2 | Home tab | done (Paste Special, Clipboard pane, Style Inspector, Manage Styles and Asian Typography landed); Dictate missing | 2–4 h |
| M3 | Insert tab | mostly done; equations (#191), icons (#309), charts (#409) and SmartArt (#497) landed; editing charts from Word, 3D models, screenshot and online media missing | 40–60 h |
| M4 | Layout and Design tabs | done; columns balance before a continuous break (#350), group (#267) | 2–4 h |
| M5 | Tables | done (row splitting, floating tables, style formatting); custom table styles (#256); Draw Table and Eraser (#303) | 4–8 h |
| M6 | References | first version done (TOC fields, cross-references, Zotero); footnotes continue (#352); all 12 bibliography styles (#383); bibliography sources import from Word's `Sources.xml` (#2640); EndNote and Mendeley desktop integration missing | 8–14 h |
| M7 | Review | done (balloons, paragraph-mark revisions, formatting revisions #41, moves #400); Translate, Block Authors, modern comment threads and multilingual proofing missing | 30–50 h |
| M8 | View | mostly done; one window per document, New Window and Switch Windows landed; Arrange All, Side by Side and Synchronous Scrolling missing (#331 open) | 3–5 h |
| M9 | Mailings | done (first version); Excel, OpenDocument and `.xls` sources landed (#334, #778); Outlook and Access sources and email merge missing | 4–8 h |
| M10 | File/Backstage | mostly done; direct desktop printing through craft-print (#641, #2108); print crash reports (#4333), tray options, pages per sheet and Options depth remain | 8–14 h |
| M11 | Draw and objects | text wrap, text boxes, canvas handles, rotation (#332), ink pens and eraser (#307), contour wrap (#354), lasso, Add Pen, Ink to Shape and Ink Replay (#395), grouping (#267) and Drawing Canvas (#344) done; Ink to Math, shape effects and connector routing missing | 25–40 h |
| M12 | Formats breadth (PDF, ODT, RTF, HTML, MD, TXT, LaTeX, `.doc` import) | done (first versions); encrypted packages open and save (#55, #281); `.doc` write and minor formats missing | 50–85 h |
| M13 | Performance budgets | on track (1.4 ms relayout, 2026-10-06); scaling fixes landed (#811, #3411, #3412, #13); large real-world documents unmeasured | 10–20 h |
| M14 | 1.0 polish, packaging, signing | releases v0.1.0–v0.13.2 published (macOS, Windows MSI and portable x64/x86/arm64, Linux x86_64/aarch64/riscv64, FreeBSD, web) | — |
| M15 | Localization (12 key languages, RTL interface, proofing languages) | 27 UI languages from the shared catalogs pinned by `craft-i18n.rev` (`9205108c…`); proofing: English, Polish, Portuguese, Italian, Croatian, Turkish, Russian, Czech, Indonesian and Spanish, plus Bulgarian and German hyphenation only; native-speaker review and native IME acceptance remain open (the winit PRs #155–#164 closed with the egui 0.37 move) | 70–115 h |
| M16 | Right-to-left and East Asian typography | RTL paragraphs (#207), tables and sections (#362); vertical text (#491), document grid (#391), ruby (#288), Enclose Characters (#297), Asian Typography (#323); kashida and `w:eastAsianLayout` layout missing | 18–30 h |

## Next after beta

Charts and SmartArt editing, the Draw tab, `.doc` writing, PDF Reflow, the remaining interface
languages, screen-reader access, ecosystem decisions (VBA, add-ins, cloud), an in-app assistant.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-31 | minor | Current focus reordered with stability second and the 1,079 open PRs first; alpha-gate evidence and milestones M1–M14 and M16 updated from the source; the gate itself not re-run |
| 2026-10-29 | trivial | HTML import keeps preformatted lines with CR, CRLF or LF input newlines (#4810); numeric character references retain their separate meaning; readiness estimates unchanged. |
| 2026-10-25 | trivial | egui 0.37 migration: 27 interface languages remain pinned by `craft-i18n.rev` (9205108…); input-method surrounding-delete and commit handling has synthetic regressions only, with managed CI passing; native acceptance (#155–#164) and native-speaker review remain open |
| 2026-10-18 | trivial | M6: Manage Sources imports Word's `Sources.xml` (#2640) |
| 2026-10-12 | minor | Czech proofing review: short-word splits rank after deletion, NOSUGGEST excludes suggestions while accepting typed forms, affix comments apply only to headers, ratios keep spaces and repeated-word messages use Czech quotes; synthetic engine end-to-end and latency checks |
| 2026-10-12 | minor | Czech spelling from the installed Hunspell dictionary, runtime-table suggestions and generic checks; Czech interface writes cs-CZ documents; no Czech hyphenation patterns bundled; estimates unchanged |
| 2026-10-12 | minor | M15: Portuguese proofing (pt-BR spelling from the installed VERO dictionary, suggestions, grammar, hyphenation; Review › Language gains Português (Brasil)) |
| 2026-10-11 | trivial | M15: Polish interface and Polish proofing landed |
| 2026-10-11 | trivial | M3: icons landed (#309) |
| 2026-10-11 | trivial | M11: contour (Tight/Through) wrap done (#354) |
| 2026-10-11 | minor | Native drawing-level ink and its picture fallback survive DOCX edits and repeated saves; rendering and editing InkML remain missing |
| 2026-10-12 | minor | Stability sweep: local desktop recovery implemented with original-file protection and focused lifecycle tests. Cross-platform field validation and longer soak testing remain. |
| 2026-10-11 | trivial | M6: the eight missing bibliography styles landed (#383) |
| 2026-10-11 | trivial | M7: formatting revisions landed (#41) |
| 2026-10-11 | trivial | Omitted `w:pPrDefault` stays omitted across saves; explicit empty/default properties and later spacing edits remain writable. Synthetic preservation checks only; readiness estimates unchanged |
| 2026-10-10 | trivial | M5: Draw Table and Eraser landed (#303) |
| 2026-10-10 | trivial | Merged main: PRs that landed removed from the Current focus list |
| 2026-10-10 | minor | Current focus heading back to ~150–230 h to beta: full number restored to the additive weighted sum (~60%), no new evidence |
| 2026-10-10 | minor | Alpha gate table added (core-workflow gate): all six workflows pass, stage stays alpha |
| 2026-10-10 | major | Created: milestones moved from ROADMAP.md with remaining hours; Current focus from gaps.md; M15–M16 added |
| 2026-10-10 | trivial | M11: ink pens and eraser landed (#307) |
| 2026-10-11 | trivial | M11: lasso, Add Pen, Ink to Shape and Ink Replay landed (#395) |
