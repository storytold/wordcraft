# WordCraft roadmap

**Stage: alpha** · next: beta, ~15 points (60% → 75% ready for real work) and ~150–230 h away

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-11 · **Change:** trivial (formatting revisions landed, #41; previously trivial: Draw tab ink, #307, and Draw Table and Eraser, #303, landed: catalog 402/434; previously minor: readiness table with hours per audience; full number back to the additive weighted sum, 60%; mainstream and essentials numbers added; alpha gate checked; previously major: full re-measure against Word for Mac 16.113.4; restructured to the craftrules progress-docs standard) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

WordCraft aims for complete parity with Microsoft Word, then goes further on speed, openness and
agent control. This page is the summary; the assessment is
[`docs/target-app-parity.md`](docs/target-app-parity.md), the work list is
[`docs/gaps.md`](docs/gaps.md), and milestones and the current focus are in
[`docs/roadmap.md`](docs/roadmap.md).

## Headline numbers (2026-10-10)

| Number | Value | Kind |
|---|---|---|
| Ribbon/menu catalog coverage | **402 / 434 (92.6%)** | measured (`cargo xtask parity` → [`docs/parity-checklist.md`](docs/parity-checklist.md)) |
| **Feature breadth** (weighted, incl. dialog options, styles, languages beyond the ribbon) | **~80%** | estimated |
| **Ready for real work** (full target) | **~60%** (55–63%) | estimated, additive weighted sum over the dimensions |
| **Mainstream practitioner** | **~55%** | estimated: weekly-work depth × discounts for interaction (×0.93), stability (×0.90), file exchange (×0.90) |
| **Essentials user** | **~63%** | estimated: core-feature depth × discounts for launch (×0.90), discoverability (×0.95), opening received files (×0.92) |
| Remaining to **beta** | **~150–230 h** of Opus 5.5 agent wall-clock (gaps #1–#7) | estimated |
| Remaining to **full parity** with Word desktop | **~750–1,250 h** (70–80% parallelizable) | estimated |
| Commands / tests / code | 433 commands · 855 tests + 10 property blocks · ~88,700 lines of Rust | measured (source) |
| Releases | v0.4.0 (2026-10-10): signed macOS universal DMG, Windows MSI x64/x86/arm64, Linux AppImage/deb/rpm/Flatpak x86_64 + aarch64, FreeBSD, web | measured |

Hours are calibrated from this repo's PRs (equations #191: ~9.7k lines in ~5 h; `.doc` reader
#145: ~4.5k lines in ~10 h; layout fidelity PRs run 5–10× slower per line). The earlier "≈120–150 h
to 100%" (2026-10-06) left out localization, multilingual proofing, ecosystem, hardware, PDF Reflow
and `.doc` writing, and priced the DOCX corpus at 10 h; see
[methodology](docs/target-app-parity.md#methodology). The three readiness numbers, their weights,
discounts and the user evidence (120 issues from outside users, 4 praise, 0 "switched from Word",
~21 open core-path bugs) are in
[readiness numbers](docs/target-app-parity.md#readiness-numbers-full-mainstream-practitioner-essentials-user).

| Audience | Ready | Opus 5.5 agent wall-clock hours to ~95% | Work that dominates |
|---|---|---|---|
| Full target (ready for real work) | **~60%** | **~650–1,100 h** (70–80% parallelizes) | Breadth: charts/SmartArt/Draw, localization and proofing languages, East Asian and RTL typography, `.doc` writing, PDF Reflow, ecosystem; plus everything below |
| Mainstream practitioner | **~55%** | **~300–500 h** (~65% parallelizes) | DOCX real-world fidelity and preserving charts/SmartArt, pagination fidelity, formatting revisions, dialog depth, objects, stability |
| Essentials user | **~63%** | **~90–140 h** (~50% parallelizes) | Install/launch stability, opening files people send (corpus subset, chart fallbacks), native printing, picture handling, English spelling depth |

**Why alpha:** the core workflows (write, format, styles, lists, tables, references, review, mail
merge, open and save .docx) work end to end and ship as signed builds on every desktop platform
and the web. **Why not beta:** DOCX has never been tested on real-world files; charts, SmartArt and
OLE objects are dropped silently; pagination differs from Word (no Aptos-metric font, no column
balancing or footnote continuation); move tracking and content controls are lost; there is open field evidence of
a startup crash and several hangs; no native printing. Gaps #1–#7 in [`docs/gaps.md`](docs/gaps.md)
are the beta list. All six core workflows pass the
[alpha gate](docs/roadmap.md#alpha-gate-core-workflows) end to end on macOS, with save and reopen;
the two partial ones (real-world .docx, printing) complete, roughly.

## By dimension

| Dimension | Parity | Hours to full | Doc |
|---|---|---|---|
| Features (depth) | 68% | 210–335 | [target-app-parity.md](docs/target-app-parity.md#feature-areas) |
| UI/UX fidelity | 65% | 65–100 | [ui-parity.md](docs/ui-parity.md) · [typing-parity.md](docs/typing-parity.md) |
| File formats | 60% | 185–300 | [file-format-parity.md](docs/file-format-parity.md) |
| Layout and pagination fidelity | 55% | 90–145 | [layout-parity.md](docs/layout-parity.md) |
| Hardware | 40% | 20–40 | [hardware-parity.md](docs/hardware-parity.md) |
| Localization | 25% | 80–130 + native review | [localization-parity.md](docs/localization-parity.md) |
| Performance | 75% | 10–20 | [layout-parity.md](docs/layout-parity.md#performance) |
| Stability | 50% | 25–45 | [gaps.md](docs/gaps.md) (#3) |
| Platforms | 95% (beyond Word on Linux, BSD, web) | 5–10 | this page |
| Ecosystem and plugins | 15% | 40–80 + owner decisions | [gaps.md](docs/gaps.md) (#22) |
| AI features | 40% (beyond Word for external agents; no in-app assistant) | 15–30 | [gaps.md](docs/gaps.md) (#25) |

## Features

| Area | Parity | Hours |
|---|---|---|
| Typing, selection, clipboard, undo, find/replace | 85% | 8–12 |
| Character and paragraph formatting | 82% | 8–12 |
| Lists | 78% | 5–8 |
| Equations | 75% | 4–8 |
| Tables | 76% | 10–16 |
| Styles and themes | 74% | 8–12 |
| Review (comments, track changes, compare, protect) | 72% | 15–25 |
| View | 72% | 6–10 |
| Page layout and sections | 70% | 12–20 |
| Mailings | 70% | 6–10 |
| Headers, footers, page numbers, fields | 70% | 8–12 |
| References (TOC, citations, bibliography, captions, index) | 65% | 12–20 |
| Footnotes and endnotes | 60% | 5–8 |
| Pictures, shapes, text boxes, WordArt | 60% | 20–30 |
| Backstage, printing, options | 58% | 12–20 |
| Right-to-left and complex scripts | 50% | 15–25 |
| Proofing (spelling, grammar, thesaurus, languages) | 35% | 25–45 |
| Accessibility | 35% | 15–25 |
| East Asian typography | 15% | 20–30 |
| Charts, SmartArt, Draw/ink, 3D models, icons | 3% | 70–110 |
| Agent control (CLI, MCP, control channel, macros) | beyond Word | — |

Weights and evidence per area: [docs/target-app-parity.md](docs/target-app-parity.md#feature-areas).

## Languages

| Language | Code | Status | UI strings |
|---|---|---|---|
| English | en | full | 100% |
| Simplified Chinese | zh-hans | partial | ~95% |
| Spanish | es | partial | ~95% |
| Hindi | hi | none | 0% |
| Arabic | ar | none (document shaping and bidi work; no mirrored UI) | 0% |
| French | fr | none | 0% |
| Portuguese | pt-br (pt-PT none) | partial | ~95% |
| Indonesian | id | none | 0% |
| Japanese | ja | partial | ~95% |
| German | de | none (PR #203 open) | 0% |
| Korean | ko | none | 0% |
| Vietnamese | vi | none | 0% |

Also shipped: Traditional Chinese (`zh-hant`), Ukrainian (`uk`) and Serbian (`sr`, `sr-latn`, #250), all partial (~95%). Estonian (`et`) now has a 1,161-entry interface catalog with system-locale
selection and a saved language choice; Estonian proofing is not included. Word ships
30 interface languages and proofing for ~50. Detail: [docs/localization-parity.md](docs/localization-parity.md).

## Upcoming

Ranked, to beta (detail and remaining milestones in [docs/roadmap.md](docs/roadmap.md)):

1. Review and merge the ~45 open PRs that already close gaps (owner).
2. DOCX real-world corpus: open, render, round-trip, compare with Word (30–50 h).
3. Preserve charts, SmartArt, OLE and ink through a round trip with fallback pictures (10–15 h).
4. Stability sweep of the open crash, hang and write-failure issues; autosave soak test (20–30 h).
5. Pagination fidelity: column balancing, footnote continuation, compat options, comparison
   harness; an Aptos-metric font decision (30–50 h + owner).
6. Formatting revisions and content controls (12–18 h); native printing (6–10 h); dialog depth
   (20–30 h).

## Progress log

| Date | What landed |
|---|---|
| 2026-10-11 | Citations and bibliography in all 12 of Word's styles: Harvard, ISO 690 (author-date and numerical), Turabian, GB/T 7714, GOST (name and title sort) and SIST02 join APA, MLA, Chicago and IEEE; numbered styles count in order of first citation (#383) |
| 2026-10-11 | Formatting revisions (#41): `w:rPrChange`/`w:pPrChange`, table/row/cell, section and numbering changes kept in DOCX; formatting recorded as revisions while tracking; accept/reject, Reviewing Pane descriptions, "Formatted: …" balloons and change bars |
| 2026-10-10 | Estonian (`et`) interface (#294): 1,161 translated entries, locale and saved-preference tests, bundled-font checks; no Estonian proofing resources |
| 2026-10-10 | Draw tab: pen, pencil and highlighter ink, stroke eraser, Select, Review › Hide Ink; ink saved to DOCX as DrawingML freeforms (#307); catalog 399/431 (92.6%) |
| 2026-10-10 | Draw Table and Eraser (#303): a pen draws one-cell tables and splits cells along drawn lines; the eraser merges the cells beside a border. Catalog 393/431 (91.2%) |
| 2026-10-10 | DOCX charts, SmartArt and OLE objects survive open and save with their parts (#319) |
| 2026-10-10 | Full re-measure against Word for Mac 16.113.4 and progress docs to the craftrules standard (this page, `docs/target-app-parity.md`, `gaps.md`, `roadmap.md`, `architecture.md`, format/layout/UI/hardware/localization parity; `docs/parity.md` → `docs/parity-checklist.md`). Landed the same day (~80 PRs): equations with OMML and an Equation tab (#191); Word 97-2003 `.doc` import (#145); right-to-left and Persian text (#207); Zotero integration (#189); Read Aloud player (#190); text boxes and floating objects editable (#46) and placed like Word (#136); floating tables (#137); track-changes fixes (#125, #244); typing parity with Word (#204); hidden text (#173); save prompts and AutoSave rules (#151); LaTeX import/export (#11); keytips and mini-toolbar (#43); Spanish, Ukrainian and Brazilian Portuguese interfaces; system theme (#249); Linux file dialogs no longer freeze (#246); Chinese UI font on Windows (#248); every installed font weight (#239); cell text direction (#245); Paste Special (#235), Advanced Find (#234), Column Selection (#237), Style Inspector (#236), custom table styles (#256), View gridlines (#243), zoom buttons (#232), inertial touchpad scrolling (#252), File Info fields kept (#268), ¶ in mixed-direction paragraphs (#278), Serbian interface (#250), table Height/Width boxes, AutoFit Contents and Table Properties (#44), AutoSave switch greyed out with the reason where it can't save (.md/.txt/.html, unsaved, browser) (#196, #176), OpenGL fallback on Windows PCs without a DirectX 12 driver (#316), Columns, Symbol and Field dialogs (#321), Tabs and Borders and Shading dialogs (#320); v0.4.0 released |
| 2026-10-10 | Full re-measure against Word for Mac 16.113.4 and progress docs to the craftrules standard (this page, `docs/target-app-parity.md`, `gaps.md`, `roadmap.md`, `architecture.md`, format/layout/UI/hardware/localization parity; `docs/parity.md` → `docs/parity-checklist.md`). Landed the same day (~80 PRs): equations with OMML and an Equation tab (#191); Word 97-2003 `.doc` import (#145); right-to-left and Persian text (#207); Zotero integration (#189); Read Aloud player (#190); text boxes and floating objects editable (#46) and placed like Word (#136); floating tables (#137); track-changes fixes (#125, #244); typing parity with Word (#204); hidden text (#173); save prompts and AutoSave rules (#151); LaTeX import/export (#11); keytips and mini-toolbar (#43); Spanish, Ukrainian and Brazilian Portuguese interfaces; system theme (#249); Linux file dialogs no longer freeze (#246); Chinese UI font on Windows (#248); every installed font weight (#239); cell text direction (#245); Paste Special (#235), Advanced Find (#234), Column Selection (#237), Style Inspector (#236), custom table styles (#256), View gridlines (#243), zoom buttons (#232), inertial touchpad scrolling (#252), File Info fields kept (#268), ¶ in mixed-direction paragraphs (#278), Serbian interface (#250), table Height/Width boxes, AutoFit Contents and Table Properties (#44), AutoSave switch greyed out with the reason where it can't save (.md/.txt/.html, unsaved, browser) (#196, #176), OpenGL fallback on Windows PCs without a DirectX 12 driver (#316), Columns, Symbol and Field dialogs (#321); v0.4.0 released |
| 2026-10-10 | Full re-measure against Word for Mac 16.113.4 and progress docs to the craftrules standard (this page, `docs/target-app-parity.md`, `gaps.md`, `roadmap.md`, `architecture.md`, format/layout/UI/hardware/localization parity; `docs/parity.md` → `docs/parity-checklist.md`). Landed the same day (~80 PRs): equations with OMML and an Equation tab (#191); Word 97-2003 `.doc` import (#145); right-to-left and Persian text (#207); Zotero integration (#189); Read Aloud player (#190); text boxes and floating objects editable (#46) and placed like Word (#136); floating tables (#137); track-changes fixes (#125, #244); typing parity with Word (#204); hidden text (#173); save prompts and AutoSave rules (#151); LaTeX import/export (#11); keytips and mini-toolbar (#43); Spanish, Ukrainian and Brazilian Portuguese interfaces; system theme (#249); Linux file dialogs no longer freeze (#246); Chinese UI font on Windows (#248); every installed font weight (#239); cell text direction (#245); Paste Special (#235), Advanced Find (#234), Column Selection (#237), Style Inspector (#236), custom table styles (#256), View gridlines (#243), zoom buttons (#232), inertial touchpad scrolling (#252), File Info fields kept (#268), ¶ in mixed-direction paragraphs (#278), Serbian interface (#250), table Height/Width boxes, AutoFit Contents and Table Properties (#44), AutoSave switch greyed out with the reason where it can't save (.md/.txt/.html, unsaved, browser) (#196, #176), OpenGL fallback on Windows PCs without a DirectX 12 driver (#316); v0.4.0 released |
| 2026-10-10 | Full re-measure against Word for Mac 16.113.4 and progress docs to the craftrules standard (this page, `docs/target-app-parity.md`, `gaps.md`, `roadmap.md`, `architecture.md`, format/layout/UI/hardware/localization parity; `docs/parity.md` → `docs/parity-checklist.md`). Landed the same day (~80 PRs): equations with OMML and an Equation tab (#191); Word 97-2003 `.doc` import (#145); right-to-left and Persian text (#207); Zotero integration (#189); Read Aloud player (#190); text boxes and floating objects editable (#46) and placed like Word (#136); floating tables (#137); track-changes fixes (#125, #244); typing parity with Word (#204); hidden text (#173); save prompts and AutoSave rules (#151); LaTeX import/export (#11); keytips and mini-toolbar (#43); Spanish, Ukrainian and Brazilian Portuguese interfaces; system theme (#249); Linux file dialogs no longer freeze (#246); Chinese UI font on Windows (#248); every installed font weight (#239); cell text direction (#245); Paste Special (#235), Advanced Find (#234), Column Selection (#237), Style Inspector (#236), custom table styles (#256), View gridlines (#243), zoom buttons (#232), inertial touchpad scrolling (#252), File Info fields kept (#268), ¶ in mixed-direction paragraphs (#278), Serbian interface (#250), table Height/Width boxes, AutoFit Contents and Table Properties (#44), AutoSave switch greyed out with the reason where it can't save (.md/.txt/.html, unsaved, browser) (#196, #176), OpenGL fallback on Windows PCs without a DirectX 12 driver (#316); free rotation and flips for pictures, shapes and text boxes with a rotation handle (#332); v0.4.0 released |
| 2026-10-09 | Interface languages follow the system or Options (#12; zh-hans, zh-hant, ja); TOC page numbers as an updatable field (#52); rotating log file (#17); window geometry remembered (#38) |
| 2026-10-08 | v0.2.0 and v0.3.0 released; Flatpak and AppImage auto-update (#22) |
| 2026-10-07 | v0.1.0 released; contributor credits in About (#6) |
| 2026-10-06 | Layout arc: wrap around floats, text boxes, page borders, line numbers, row splitting, drop caps, hyphenation, comment balloons; release pipeline for every platform; first estimate (~62% real feature parity, 120–150 h to 100%) |
| 2026-10-05 | M0–M12 first versions: model, layout, render, engine with 200+ commands, Word-style UI, DOCX I/O, PDF and other formats, references, review, mailings, CLI, MCP, web |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-11 | trivial | Progress log: eight more bibliography styles (#383) |
| 2026-10-10 | trivial | Progress log: Draw tab ink (#307) |
| 2026-10-10 | trivial | Draw Table and Eraser (#303): catalog 393/431 (91.2%) |
| 2026-10-10 | trivial | Progress log: charts, SmartArt and OLE objects kept on DOCX save (#319) |
| 2026-10-10 | trivial | Progress log: free rotation and flips (#332) |
| 2026-10-10 | trivial | Progress log: Windows OpenGL fallback without a DirectX 12 driver (#316) |
| 2026-10-10 | minor | Password-protected .docx (#281): files encrypted with a password (agile encryption) open and save; File › Info › Encrypt with Password |
| 2026-10-10 | minor | Relanded clipboard pane, table styles editor, Manage Styles, Asian typography (kinsoku), Group/Ungroup and Shape Effects (#264 #263 #270 #271 #267 #280): catalog 390/431 (90.5%) |
| 2026-10-10 | minor | Merged main: catalog 384/430 (89.3%), 431 commands; Paste Special, Advanced Find, Column Selection, Style Inspector, custom table styles, gridlines, zoom buttons, touchpad scrolling, File Info, Serbian landed; mainstream 54→55% |
| 2026-10-10 | minor | Readiness table with hours per audience; full number restored to the additive weighted sum (~47% → ~60%): method aligned with the standard, no new evidence; beta distance back to ~15 points and ~150–230 h |
| 2026-10-10 | minor | Mainstream practitioner (~54%) and essentials user (~63%) numbers added; full number recomputed 60% → ~47% with the standard's discounts; beta distance now ~28 points and ~450–700 h; stage stays alpha |
| 2026-10-10 | minor | Alpha gate checked (six core workflows pass); stage stays alpha |
| 2026-10-10 | major | Full re-measure; restructured to the progress-docs standard (stage banner, two numbers, dimensions, languages, upcoming, progress log); parity tables moved to `docs/target-app-parity.md`, milestones to `docs/roadmap.md` |
| 2026-10-09 | minor | Recently landed: RTL, Ukrainian and Brazilian Portuguese |
| 2026-10-06 | major | First honest estimate: 88% catalog, ~62% real parity, alpha ~85% of the way |
