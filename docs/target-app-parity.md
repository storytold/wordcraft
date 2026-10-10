# WordCraft parity with Microsoft Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** minor (readiness table with hours per audience; full number back to the additive weighted sum, 60%; mainstream and essentials numbers added; alpha gate checked; previously major: full re-measure against Word for Mac 16.113.4; replaces the parity tables that lived in ROADMAP.md) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

The authoritative assessment of how close WordCraft is to Microsoft Word. [`ROADMAP.md`](../ROADMAP.md)
summarizes it; [`gaps.md`](gaps.md) itemizes every shortfall; the deep checklists are
[`file-format-parity.md`](file-format-parity.md), [`layout-parity.md`](layout-parity.md),
[`typing-parity.md`](typing-parity.md), [`ui-parity.md`](ui-parity.md),
[`hardware-parity.md`](hardware-parity.md) and [`localization-parity.md`](localization-parity.md).
The generated ribbon checklist is [`parity-checklist.md`](parity-checklist.md).

## Headline (2026-10-10, origin/main `a99dddc`, v0.4.0 + ~90 merged PRs)

| Number | Value | Kind |
|---|---|---|
| Ribbon/menu catalog coverage | **384 / 430 (89.3%)** | **measured**: `cargo xtask parity` (re-derived this pass from `crates/engine/src/catalog.rs` against every `CommandSpec::new` id in the source; identical to the checked-in `parity-checklist.md`) |
| Feature breadth (weighted, beyond the ribbon: dialog options, citation styles, proofing languages, chart types, UI languages) | **~80%** | estimated |
| Feature depth (weighted by use, table below) | **~68%** | estimated |
| **Ready for real work** (full target) | **~60%** (range 55–63%) | estimated: weighted sum over the dimension table ([By dimension](#by-dimension)) |
| **Mainstream practitioner** | **~55%** | estimated, same method |
| **Essentials user** | **~63%** | estimated, same method |
| Remaining effort to beta | **~150–230 h** of Opus 5.5 agent wall-clock: the blocking beta list ([gaps.md](gaps.md) #1–#7), which lifts file formats, layout, stability and UI enough to reach ~75% | estimated |
| Remaining effort to full parity with Word desktop | **~750–1,250 h** | estimated |
| Commands | 431 engine commands (+ `ui.*` commands in the front end) | measured (source count) |
| Tests | 852 `#[test]` functions + 10 `proptest!` blocks | measured (source count) |
| Code | ~88,700 lines of Rust in 15 crates, 3 apps and xtask | measured (`wc -l`) |

**Stage: alpha.** All six core workflows pass the [alpha gate](roadmap.md#alpha-gate-core-workflows); see [`ROADMAP.md`](../ROADMAP.md) for why it isn't beta, and the distance.

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
  layout), tests, the i18n catalogs, open issues (62) and open PRs (~45), git history.
- **Not done this pass:** opening real-world .docx files in both apps and comparing pages. There is
  no real-world corpus in the repo (Word-produced files can't be committed), and the disk budget
  ruled out building. That comparison is the first gap in [`gaps.md`](gaps.md).

## Feature areas

Percent is depth and fidelity against Word, not presence. Weight is our estimate of the share of a
typical Word user's time spent in the area (sums to 100). Hours are Opus 5.5 agent wall-clock to
full parity for the area.

| Area | Weight | Parity | Was (2026-10-06) | Hours | Evidence and what's missing |
|---|---|---|---|---|---|
| Typing, selection, clipboard, undo, find/replace | 12 | 85% | 85% | 8–12 | Typing parity pinned by `tests_typing.rs` ([typing-parity.md](typing-parity.md)). Paste Special (#235), Advanced Find (#234) and Column Selection (#237) landed; Clipboard pane still missing (PR #264 open). |
| Character and paragraph formatting | 11 | 82% | 85% | 8–12 | Nearly every property, Font/Paragraph dialogs, character border. Missing: Phonetic Guide, Enclose Characters, Asian Typography, kerning/ligature/stylistic-set options in the Font dialog's Advanced tab. |
| Styles and themes | 8 | 74% | 75% | 8–12 | Gallery, pane, create/modify, style sets, themes. Style Inspector landed (#236). Missing: Manage Styles (#270 open), style separators, linked-style edge cases; Word ships 11 Quick Style sets plus dozens of themes, we ship our own smaller set. |
| Lists | 6 | 78% | 75% | 5–8 | Bullets, numbering, multilevel, restart, `w:lvlOverride` levels (#134), Word's list AutoFormat and Enter/Backspace behaviour. Missing: Define New Multilevel List dialog depth, list styles, legal numbering edge cases. |
| Tables | 8 | 76% | 72% | 10–16 | Merge/split, styles (cell text takes style formatting, #104), custom table styles saved in the .docx (#256), Word 2013 edge, floating tables, rows split across pages, sort, formula, cell gridlines (#243). Missing: Draw Table/Eraser, nested-table polish, interactive column resize (#49 open, #217). Cell text direction landed (#245). |
| Page layout and sections | 7 | 70% | 72% | 12–20 | Margins, size, columns, breaks, page borders, line numbers, vertical alignment, drop caps, hyphenation. Columns don't balance; no document grid; no vertical text; RTL sections missing. Detail: [layout-parity.md](layout-parity.md). |
| Headers, footers, page numbers, fields | 5 | 70% | 70% | 8–12 | First/even/odd, link to previous, TOC as an updatable field (#52), cross-references to real bookmarks (#227). Field coverage is a subset of Word's ~90 field codes; Field dialog missing. |
| Footnotes and endnotes | 3 | 60% | 60% | 5–8 | Placed and editable, numbers from Word files (#103). Long notes don't continue onto the next page; no continuation separator; no note options per section. |
| References (TOC, citations, bibliography, captions, index, TOA) | 5 | 65% | 60% | 12–20 | Working first versions, Zotero integration (#189), Zotero/Mendeley `ADDIN` fields round-trip. 4 of Word's 12 bibliography styles (APA, MLA, Chicago, IEEE); source manager depth; bibliography sources not yet in DOCX (#169 open). |
| Review (proofing UI, comments, track changes, compare, protect) | 8 | 72% | 75% | 15–25 | Comment balloons, tracked insert/delete incl. paragraph marks (#244, #125), compare, restrict editing. Missing: formatting revisions (`w:rPrChange`/`w:pPrChange`, issue #41), move tracking, track-changes balloons, Translate, Block Authors, modern comment threads/mentions. Lowered: these gaps are confirmed by source (no `rPrChange` anywhere). |
| Proofing (spelling, grammar, thesaurus, languages) | 5 | 35% | (in Review) | 25–45 | English only: one dictionary, a rule-based grammar checker. Word ships 120 proofing tools for ~50 languages plus Editor (style refinements, similarity). Issues #25, #40, #100. |
| Mailings | 2 | 70% | 70% | 6–10 | Mail merge from CSV, fields, rules, preview, envelopes, labels. Select Recipients takes a typed list or CSV (#247); no Excel/Outlook/Contacts data sources, no email merge. |
| Pictures, shapes, text boxes, WordArt | 6 | 60% | 60% | 20–30 | Insert, crop, recolour, styles, floating placement like Word (#136), wrap (square/top-bottom/behind/front), text boxes edit and overflow (#46), VML text boxes (#242). Missing: tight/through contour wrap, rotation handle, group (#267 open), shape effects, WordArt, connectors, Drawing Canvas, Icons, online pictures, screenshot. |
| Equations | 1 | 75% | 20% (with Draw) | 4–8 | OMML read/write, OpenType MATH layout, in-place editor with an Equation tab (#191). Missing: line breaking of long display equations, ink equations. |
| Charts, SmartArt, Draw/ink, 3D models, icons | 4 | 3% | (20% with equations) | 70–110 | Nothing yet: all 11 Draw tab commands missing; charts and SmartArt are dropped on DOCX read (no `c:chart`/`dgm` handling in `crates/docx`). |
| View (modes, zoom, panes, windows) | 3 | 72% | 70% | 6–10 | Print/web/draft/read/focus, dark page (#194), pages side by side when zoomed out (#251), navigation pane. Missing: View Side by Side, Synchronous Scrolling, Arrange All, Switch Windows, Outline view depth, Immersive Reader depth. |
| Backstage, printing, options | 3 | 58% | 55% | 12–20 | New/Home pages (#254), save prompts (#151), web printing via the browser (#209). Desktop printing still goes through PDF; Options has a fraction of Word's panes. |
| Right-to-left and complex scripts | 1 | 50% | 55% | 15–25 | UAX #9, Arabic shaping, RTL paragraphs (#207). No RTL sections/tables, kashida justification or mirrored UI; open bug reports (#215, #211, #199, #66, #63, #48, #19) need re-testing against #207. |
| East Asian typography | 1 | 15% | not rated | 20–30 | IME input works (winit fixes pending, #155–#164). No vertical text, ruby/phonetic guide, document grid, `w:eastAsianLayout`, enclose characters, Asian line-break options. |
| Accessibility | 1 | 35% | not rated | 15–25 | AccessKit enabled in eframe; an accessibility checker exists. Screen-reader access to the document canvas is untested; no alt-text workflow polish, no read-order tools. |
| Agent control (CLI, MCP, control channel, macro record/playback) | 0 | beyond Word | 100%+ | — | Every action is a command id usable from CLI, MCP and a keyed control port. Word has VBA and Office.js instead (see Ecosystem). |

Weighted feature depth: **~68%** (Σ weight × parity / 100 over the table).

## Readiness numbers: full, mainstream practitioner, essentials user

Three numbers, each with its own hours (craftrules `standards/progress-docs.md`, "Numbers"). The
**full** number is the additive weighted sum over the dimension table below. **Mainstream** and
**essentials** average depth over the areas in scope, then multiply by written-down discounts for
what still stops real work. The stage follows the full number and the core-workflow gate.

| Audience | Ready | Opus 5.5 agent wall-clock hours to ~95% | Work that dominates |
|---|---|---|---|
| Full target (ready for real work) | **~60%** | **~650–1,100 h** (70–80% parallelizes) | Breadth: charts/SmartArt/Draw, localization and proofing languages, East Asian and RTL typography, `.doc` writing, PDF Reflow, ecosystem; plus everything below |
| Mainstream practitioner | **~55%** | **~300–500 h** (~65% parallelizes) | DOCX real-world fidelity and preserving charts/SmartArt, pagination fidelity, formatting revisions, dialog depth, objects, stability |
| Essentials user | **~63%** | **~90–140 h** (~50% parallelizes) | Install/launch stability, opening files people send (corpus subset, chart fallbacks), native printing, picture handling, English spelling depth |

Hours are calibrated as in [Calibration](#calibration-of-hours-from-this-repos-history) and are
subsets: essentials ⊂ mainstream ⊂ full. Mainstream to ~95%: its areas' depth (~180–280 h) plus
the file-exchange, stability and interaction work behind the discounts (~135–225 h). Essentials to
~95%: launch/install stability (15–25 h), opening received files (30–50 h), native printing (6–10
h), pictures and tables (12–18 h), spelling depth (8–12 h), polish (10–20 h).

### The discounts (evidence)

| Discount | Mainstream | Essentials | Evidence |
|---|---|---|---|
| Interaction fidelity | ×0.93 | — | Typing pinned to Word (`tests_typing.rs`), ribbon, keytips, mini-toolbar; but 16 modal dialogs against Word's ~100, 101 of ~250 shortcuts, no table column drag on the canvas (#217, #49), picture/shape handling complaints (#82, #142), lines wrap differently without an Aptos-metric font |
| Discoverability and UI clarity | — | ×0.95 | Word-style ribbon a Word user already knows, command search, keytips; some newer pane strings and tooltips terse or untranslated |
| Stability on real machines | ×0.90 | ×0.90 (launch and install) | Never-crash standard, hostile-param fuzzing, panic guard; but open startup crash on Intel UHD (#170), freezes (#77, #59, #31), Windows writes fail without admin (#218), installer conflicts with other Crafting Apps (#64, #73, #106, #51); 5 days of field history |
| Exchanging files with Word users | ×0.90 | ×0.92 (opening files people send) | Word opens our .docx; `.doc` import; but no real-world corpus test, charts/SmartArt/OLE dropped silently, formatting revisions lost (#41), pagination differs (Aptos, no column balancing or footnote continuation). Simple documents, which are most of what a casual user receives, come through well; double-click open on macOS is fixed on main but not yet released (#223, #279) |
| Product | **×0.753** | **×0.787** | |

### Full target (ready for real work): ~60%

The additive weighted sum over the [dimension table](#by-dimension): **0.30·68 + 0.20·60 + 0.15·55 +
0.10·65 + 0.10·50 + 0.05·75 + 0.04·25 + 0.03·95 + 0.01·40 + 0.02·15 ≈ 60%** (range 55–63%). No
multiplicative discounts: stability, file formats, layout and UI are weighted dimensions here.

### Mainstream practitioner: ~55%

The typical professional Word user (office, legal, academic, writing) in their own language, weekly
work only. Left out: equations, mail merge, Draw/ink, East Asian and right-to-left typography,
accessibility tools, proofing in other languages, add-ins/VBA, Copilot, cloud and co-authoring,
dictation and pens.

| Area | Weight | Depth |
|---|---|---|
| Typing, selection, clipboard, undo, find/replace | 17 | 85% |
| Character and paragraph formatting | 16 | 82% |
| Styles | 9 | 74% |
| Tables | 9 | 76% |
| Review (comments, track changes) | 9 | 72% |
| Lists | 8 | 78% |
| Page layout and sections | 6 | 70% |
| Headers, footers, page numbers, fields | 5 | 70% |
| Proofing in the user's language (English) | 5 | 55% |
| Pictures, shapes, text boxes | 5 | 60% |
| References (TOC, captions, citations) | 4 | 65% |
| Footnotes | 2 | 60% |
| Charts and SmartArt (pasted from Excel or received) | 2 | 3% |
| Print and PDF | 2 | 65% |
| View | 1 | 72% |

Base ≈ **73%**; **73% × 0.753 ≈ 55%**.

### Essentials user: ~63%

Someone who uses only the core of Word: create or open a document, type and format it, a list, a
picture, a simple table, spelling, undo, save, PDF or print, default settings. Advanced options,
professional workflows and file-exchange edge cases are left out, as is everything left out of
mainstream.

| Feature | Weight | Depth |
|---|---|---|
| Typing, selection, copy/paste, undo | 20 | 85% |
| Font, size, bold/italic/underline, colour, highlight | 15 | 88% |
| New, open, save, AutoSave, templates | 12 | 80% |
| Alignment, line spacing, indents | 8 | 85% |
| Bullets and numbering | 8 | 85% |
| Spelling (English) | 8 | 65% |
| Insert, move and resize a picture | 7 | 65% |
| Simple table | 6 | 78% |
| Heading styles from the gallery | 5 | 78% |
| Print and export PDF | 5 | 70% |
| Find and replace | 3 | 85% |
| Page numbers and a simple header | 3 | 75% |

Base ≈ **80%**; **80% × 0.787 ≈ 63%**.

### User evidence (GitHub, 2026-10-10; no issue was filed by a storytold org member)

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
| Features (depth, table above) | 30% | 68% | 210–335 | this file | Weighted table above. Hours: the area table sums to 290–460 h; the Page layout, Footnotes, RTL and East Asian rows (~50–85 h) are counted under Layout and the Proofing row (25–45 h) under Localization, so they aren't counted twice |
| File formats (DOCX fidelity first) | 20% | 60% | 185–300 | [file-format-parity.md](file-format-parity.md) | DOCX opens in Word and round-trips our own tests, but no real-world corpus test yet; charts, SmartArt, OLE, content controls (unwrapped), formatting revisions and encrypted files are lost or refused |
| Layout and pagination fidelity | 15% | 55% | 90–145 | [layout-parity.md](layout-parity.md) | Aptos has no metric-matched substitute so lines break differently from Word; no column balancing, footnote continuation or document grid; compatibility modes beyond 15 partial |
| UI/UX fidelity | 10% | 65% | 65–100 | [ui-parity.md](ui-parity.md) | Ribbon, keytips, mini-toolbar, Backstage, 101 shortcuts; 16 modal dialogs against Word's ~100; no ribbon/keyboard customization |
| Stability | 10% | 50% | 25–45 | [gaps.md](gaps.md) | Never-crash standard and hostile-param fuzzing in place, but open reports of a startup crash on Intel UHD (#170), freezes (#77, #59, #31) and Windows write failure without admin (#218); no soak test; 5 days of field history |
| Performance | 5% | 75% | 10–20 | [layout-parity.md](layout-parity.md) | 61 ms cold layout and 1.4 ms relayout on a 188-page document (measured 2026-10-06, not re-run); font picker freezes fixed in #233; no large real-world documents measured |
| Localization | 4% | 25% | 80–130 + native review | [localization-parity.md](localization-parity.md) | 9 UI languages (Word 30); 4 of the 12 key languages partly done; English-only proofing |
| Platforms | 3% | 95% | 5–10 | [ROADMAP.md](../ROADMAP.md) | macOS (universal, signed), Windows x64/x86/arm64, Linux (AppImage/deb/rpm/Flatpak), FreeBSD, web. Beyond Word on Linux, BSD and the web; Word's iPad/iPhone/Android apps are out of scope |
| Hardware | 1% | 40% | 20–40 | [hardware-parity.md](hardware-parity.md) | No native printing on desktop, no pen/ink, no dictation; GPU drawing and HiDPI fine. Ink hours are in the Features row (Draw tab) |
| Ecosystem and plugins | 2% | 15% | 40–80 | [gaps.md](gaps.md) | Zotero built in, macros record/play commands, `.docm` macros preserved (#172). No VBA execution, no Office add-ins, no EndNote/Mendeley plugins, no cloud storage or co-authoring |
| AI features | 0% | 40% | 15–30 | [gaps.md](gaps.md) | Beyond Word for external agents (MCP, CLI). No in-app writing assistant like Copilot yet (chat PR #178 open). Weight 0: Copilot is a paid add-on and not what decides a switch |

**Ready for real work ≈ 0.30·68 + 0.20·60 + 0.15·55 + 0.10·65 + 0.10·50 + 0.05·75 + 0.04·25 +
0.03·95 + 0.01·40 + 0.02·15 ≈ 60%.**

## Methodology

- **Measured** numbers come from the source on origin/main: the catalog/registry diff (the same
  computation as `cargo xtask parity`), `#[test]` counts, i18n catalog rows, reader/writer element
  coverage (`grep` over `crates/docx`, `crates/docbin`, `crates/formats`) and the Word bundle
  listing. Nothing was built or run this pass (disk budget).
- **Estimated** numbers are judgements from that evidence plus Word's documented feature set. The
  previous estimate (~62% "real feature parity", 2026-10-06) measured feature depth only. This pass
  splits it into feature depth (67%, up: equations, .doc import, RTL, text boxes, Zotero landed) and
  ready-for-real-work (60%, additive weighted sum), which adds file fidelity, stability,
  localization and the rest. A multiplicative version (~47%) was briefly used the same day and
  reverted to align with the standard; no new evidence.
- **Why the hours went up** from "120–150 h to 100%" (2026-10-06): that figure left out
  localization, multilingual proofing, ecosystem, hardware, PDF Reflow and `.doc` writing, and
  priced DOCX corpus fidelity at 10 h. The new evidence is the Word bundle inventory (30 UI
  languages, 120 proofing tools, 12 bibliography styles), the 62 open issues (crashes, freezes,
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
| 2026-10-10 | minor | Merged main (12 PRs): Paste Special, Advanced Find, Column Selection, Style Inspector, custom table styles, gridlines, zoom buttons, touchpad scrolling, File Info fields, Serbian. Catalog 378/428 → 384/430 (89.3%); typing 82→85%, styles 72→74%, tables 74→76%; feature depth 67→68%; mainstream 54→55%; full stays ~60%, essentials ~63% |
| 2026-10-10 | minor | Readiness table: hours to ~95% per audience (full ~650–1,100 h, mainstream ~300–500 h, essentials ~90–140 h). Full number restored to the additive weighted sum, ~47% → ~60%: method aligned with the standard, no new evidence; beta distance back to ~15 points and ~150–230 h |
| 2026-10-10 | minor | Added mainstream practitioner (~54%) and essentials user (~63%) numbers with written weights and discounts; full ready-for-real-work recomputed with the same multiplicative discounts: 60% → ~47% (method change, not new product evidence; stage stays alpha: above 40% and the gate passes); beta now ~450–700 h to the 75% bar; user-evidence counts from GitHub |
| 2026-10-10 | minor | Stage checked against the core-workflow alpha gate: passes, stays alpha |
| 2026-10-10 | major | Created from ROADMAP.md's "Parity by area" and "Estimate to 100%" sections; full re-measure against Word for Mac 16.113.4 (bundle listing, catalog diff from source, issue tracker); split into feature depth (67%) and ready for real work (60%); added proofing, East Asian, accessibility areas and the dimension table; hours re-calibrated from PR history |
| 2026-10-06 | major | First estimate in ROADMAP.md: ~62% real feature parity, 120–150 h to 100% |
