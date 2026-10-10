# WordCraft roadmap

WordCraft aims for complete feature parity with Microsoft Word, then goes further on speed, openness
and agent control. This file tracks where we are honestly. Generated numbers come from
`cargo xtask parity` (`docs/parity.md`).

## Where we are (2026-10-09)

| Measure | Value |
|---|---|
| Commands (every action, scriptable by CLI/MCP/control channel) | **431** |
| Feature catalog coverage (Word ribbon/menu features with a command) | **385 / 430 (90%)** |
| **Estimated real feature parity** (depth and fidelity, not just a command) | **~62%** |
| **Distance to alpha** | **~85% of the way; ≈20–25 h of Claude wall-clock work** |
| **Distance to 100% parity** | **≈120–150 h of Claude Opus 5.5 wall-clock work** (with parallel agents) |
| Tests | ~325 tests (unit, round-trip, fuzz/proptest, MCP acceptance) |
| Layout speed (188-page document) | 61 ms cold, **1.4 ms** relayout after an edit |
| Code | ~44k lines of Rust in 12 crates and 3 apps |

Catalog coverage overstates parity: many commands are first versions. The real-parity estimate
weighs each area by how much of Word's behaviour it reproduces, and by how often people use it.

## Alpha

**What we mean by alpha:** someone can write, format and review real documents every day, open
the .docx files they receive and send back files Word opens cleanly, on every desktop platform
from a signed download, without losing work.

| Alpha requirement | State |
|---|---|
| Editing, formatting, styles, lists, tables, headers/footers, references, review | done (first versions, tested) |
| Pagination fidelity: wrap, row splitting, hyphenation, drop caps, line numbers, borders | done this arc |
| Comments in margin balloons, track changes | done |
| Never-crash standard (no panics, hostile-param fuzzing, panic guard) | done |
| Autosave and crash recovery | done; needs a soak test |
| **DOCX fidelity against a corpus of real-world files** (open, render, round-trip, fix) | **not started — the main alpha blocker (≈10 h)** |
| Footnotes that continue onto the next page; column balancing | missing (≈4 h) |
| Native printing (today printing goes through PDF) | missing (≈4 h) |
| Signed builds for macOS, Windows, Linux, FreeBSD and web | pipeline written; **needs the GitHub remote and release secrets from the owner** |
| Real app icon / mascot art | placeholder; **needs owner art** |

So: alpha is roughly **85% of the way**, with about **20–25 hours** of agent work left, plus two
owner actions (push to `github.com/storytold/wordcraft` and enable the release secrets, and supply
the icon art).

## Parity by area

| Area | Status | Parity |
|---|---|---|
| Typing, selection, clipboard, undo, find/replace | Solid; IME, autocorrect, smart quotes, list autoformat, column (block) selection | 85% |
| Character & paragraph formatting | Nearly all properties; Font/Paragraph dialogs | 85% |
| Styles (gallery, pane, create/modify, style sets, themes) | Good | 75% |
| Lists (bullets, numbering, multilevel, restart, set value) | Good | 75% |
| Tables (insert, merge/split, styles, borders, header rows, sort, formula) | Good; rows split across pages; no drawn tables, no nested-table polish | 72% |
| Page layout (margins, size, orientation, columns, breaks, sections) | Good; page borders, line numbers, vertical alignment, drop caps, hyphenation; columns don't balance | 72% |
| Headers/footers, page numbers, fields | Good; first/even/odd, link to previous | 70% |
| Footnotes/endnotes | Placed and editable; long notes don't continue onto the next page | 60% |
| References (TOC, citations APA/MLA/Chicago/IEEE, bibliography, index, figures, cross-refs, TOA) | Working first versions; Zotero/Mendeley `ADDIN` fields and custom properties round-trip; Zotero in the app (`ui.zotero.*`, `wordcraft-cli zotero`): citations, bibliography, refresh, unlink, document preferences, in-text ↔ footnote switching, Zotero ribbon tab, Move Past Citation; Read Aloud player (speed, previous/next sentence, skips citations and bibliography) | 60% |
| Review (spelling, grammar, thesaurus, comments, track changes, compare, protect) | Good; comment balloons in the margin; no track-changes balloons | 75% |
| Mailings (mail merge, rules, preview, envelopes, labels) | Working | 70% |
| Pictures & shapes (insert, size, crop, recolour, effects, styles, float position) | Text wraps around floats (rectangular); text boxes render and edit their text; select, drag to move, resize handles, arrow nudge on the canvas; no tight/contour wrap, no rotation handles | 60% |
| Draw tab (ink), SmartArt, charts, 3D models, equations editor | Equations: full OMML read/write, 2D layout (OpenType MATH) and in-place editing with an Equation tab; long display equations don't break across lines yet. Ink, SmartArt, charts, 3D not started | 20% |
| File formats: DOCX read/write | Good (Word opens our files); charts/SmartArt/OLE dropped; untested on a real-world corpus | 75% |
| File formats: PDF, ODT, RTF, HTML, Markdown, LaTeX, TXT | Working | 70% |
| File formats: Word 97-2003 `.doc`/`.dot` import | Text, formatting, styles, sections, headers/footers, tables, lists, notes, fields, bookmarks, pictures (spec-based reader; metafile images and Word 6/95 rejected) | 60% |
| View modes (print, web, draft, read, focus, zoom, navigation pane) | Working | 70% |
| Backstage (new from templates, open, info, export, options) | Working; printing goes through PDF | 55% |
| Right-to-left and complex scripts (Persian, Arabic, Hebrew) | Unicode Bidirectional Algorithm, Arabic shaping (joining, ZWNJ, lam-alef, mirrored brackets), mixed Persian/Latin/number lines, mirrored paragraphs (indents, tabs, list labels, ruler), visual caret movement and hit testing, complex-script fonts/sizes in DOCX; no kashida justification, RTL sections/tables or right-to-left UI yet | 55% |
| Agent control (CLI, MCP, control channel, macros) | Beyond Word | 100%+ |

## Estimate to 100%

**About 120–150 hours of Claude Opus 5.5 wall-clock work** (with parallel agents), in this order:

1. **Alpha blockers** (≈20–25 h): DOCX fidelity corpus, footnote continuation, column balancing,
   native printing, autosave soak test, first signed release.
2. **Objects** (≈12 h): tight/through (contour) wrap, rotation handles on the canvas,
   grouping, z-order polish, track-changes balloons and formatting revisions.
3. **Draw tab / ink** (≈15 h).
4. **Charts (own renderer) and SmartArt-style diagrams** (≈20 h).
5. **Equation editor** — done: OMML read/write, 2D layout, in-place editing with Word's Equation tab (structures, symbols, built-ins, Unicode/LaTeX input, build-up, Math AutoCorrect, `#` numbers, structure commands). Remaining: line breaking of long display equations, ink equations (≈4 h).
6. **Dialog depth**: every Word dialog with all its options (Font, Paragraph, Tabs, Borders and
   Shading, Page Setup, Styles, Columns, Index/TOC options, Mail Merge wizard, Options panes) (≈20 h).
7. **Accessibility (screen readers), localisation, and the rest of right-to-left** (≈12 h): right-to-left
   sections and tables (`w:bidi` in `w:sectPr`, `w:bidiVisual`), kashida justification, a Persian UI
   translation and a mirrored interface, RTL in the HTML/ODT/RTF converters, Indic shaping checks.
8. **Long tail**: kerning/ligature options, drawn tables, nested-table polish, more templates,
   Word's less-used commands still missing from the catalog (≈10 h).

## Milestones

| # | Milestone | State |
|---|---|---|
| M0 | Skeleton + vertical slice (model, layout, render, engine, Word-style UI, CLI, MCP, web) | **done** |
| M1 | DOCX I/O | **done** (first version; corpus testing next) |
| M2 | Home tab complete | **done** |
| M3 | Insert tab | mostly done (charts/SmartArt/icons/3D missing) |
| M4 | Layout + Design tabs | **done** except column balancing |
| M5 | Tables | **done** (row splitting landed) |
| M6 | References | first version done; footnote continuation missing |
| M7 | Review | **done** (margin balloons landed) |
| M8 | View | mostly done |
| M9 | Mailings | done (first version) |
| M10 | File/Backstage | mostly done; native printing missing |
| M11 | Draw + objects | text wrap and text boxes done; ink, rotation, grouping missing |
| M12 | Formats breadth (PDF, ODT, RTF, HTML, MD, TXT) | **done** (first versions) |
| M13 | Performance budgets | on track (1.4 ms relayout) |
| M14 | 1.0 polish, packaging, signing | pipeline written; waiting on remote and secrets |

## Recently landed
- Right-to-left and Persian text: bidi reordering (UAX #9), Arabic-script shaping, right-to-left
  paragraphs (Left-to-Right / Right-to-Left Text Direction), visual arrow keys, `w:rtl`/`w:cs`/`w:szCs`
  /`w:bCs`/`w:iCs`/`w:rFonts w:cs`/`w:lang w:bidi` round-trip, Persian font substitutes.
- Ukrainian interface catalog: all existing 771 labels, Ukrainian locale detection and a persisted language choice (#85).
- Brazilian Portuguese interface catalog: all existing 771 labels, `pt-BR` locale detection and a persisted language choice (#127).
- Text wraps around floating pictures and shapes; text boxes lay out their own text.
- Table rows split across pages between lines (Can't Split honoured, header rows repeat).
- Drop caps; automatic hyphenation with Word's 0.25" hyphenation zone; soft hyphens.
- Page borders measured from the page edge, line numbers, vertical alignment, column separators.
- Comment balloons in a markup area beside each page, with leader lines to their anchors.

## Next
The DOCX fidelity corpus, then footnote continuation, column balancing and native printing.
