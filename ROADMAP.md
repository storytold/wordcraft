# WordCraft roadmap

**Stage: alpha** · next: beta, ~15 points (60% → 75% ready for real work) and ~150–230 h away

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-16 · **Change:** trivial (progress log: the Styles pane docks left or right, or floats, #1756; previously trivial: table style editor formats every part, #714; previously trivial: VML freeforms read and kept on save; previously trivial: progress log: consistent preferred cell widths; previously trivial: multi-paragraph field instructions no longer add blank lines; previously trivial: page-break-only list paragraphs hide their labels; previously trivial: progress log: grouped `flowChartOr` shapes draw their circle and crossing spokes; previously trivial: progress log: copies keep their formatting in other programs, #1523; previously trivial: progress log: privacy options on save, #1062; previously trivial: progress log: equation pictures in theme colours on Windows, #1002; previously trivial: symbol fonts, #776; previously trivial: the save prompt answers from the keyboard, #861; previously trivial: table column and row markers on the rulers, #700; previously trivial: table borders and the resize handle drag on the page, #217; previously trivial: formatting revisions landed, #41; previously trivial: Draw tab ink, #307, and Draw Table and Eraser, #303, landed: catalog 402/434; previously minor: readiness table with hours per audience; full number back to the additive weighted sum, 60%; mainstream and essentials numbers added; alpha gate checked; previously major: full re-measure against Word for Mac 16.113.4; restructured to the craftrules progress-docs standard) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-14 · **Change:** trivial (progress log: print-area strings translated in every catalog; previously trivial: progress log: ODT sections, #1379; previously trivial: progress log: table deletes, Change Case and Unlink Fields respect Track Changes and locked content controls, #1329; previously trivial: progress log: rounded rectangle corners, #1476; previously trivial: progress log: Save As type list, #1399; previously trivial: progress log: privacy options on save, #1062; previously trivial: progress log: equation pictures in theme colours on Windows, #1002; previously trivial: symbol fonts, #776; previously trivial: the save prompt answers from the keyboard, #861; previously trivial: table column and row markers on the rulers, #700; previously trivial: table borders and the resize handle drag on the page, #217; previously trivial: formatting revisions landed, #41; previously trivial: Draw tab ink, #307, and Draw Table and Eraser, #303, landed: catalog 402/434; previously minor: readiness table with hours per audience; full number back to the additive weighted sum, 60%; mainstream and essentials numbers added; alpha gate checked; previously major: full re-measure against Word for Mac 16.113.4; restructured to the craftrules progress-docs standard) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4
> **Printing update:** 2026-10-12 · direct desktop printing through craft-print; details below.

WordCraft aims for complete parity with Microsoft Word, then goes further on speed, openness and
agent control. This page is the summary; the assessment is
[`docs/target-app-parity.md`](docs/target-app-parity.md), the work list is
[`docs/gaps.md`](docs/gaps.md), and milestones and the current focus are in
[`docs/roadmap.md`](docs/roadmap.md).

## Headline numbers (2026-10-10)

| Number | Value | Kind |
| 2026-10-12 | Table formulas: cell references and ranges (`=B2+B3`, `=SUM(B2:C3)`), arithmetic and comparisons, IF, AND, OR, NOT, TRUE, FALSE, DEFINED and the `\#` number format; ABOVE and LEFT sum the run of numbers next to the cell as Word does; an unknown function or a syntax error is an error instead of a sum; Update Field (F9) recalculates formula fields |
| Ribbon/menu catalog coverage | **487 / 502 (97.0%)** | measured (`cargo xtask parity` → [`docs/parity-checklist.md`](docs/parity-checklist.md)) |
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
balancing or footnote continuation); content controls are lost; there is open field evidence of
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
| Localization | 27% | 75–120 + native review | [localization-parity.md](docs/localization-parity.md) |
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
| Proofing (spelling, grammar, thesaurus, languages) | 40% | 20–40 |
| Accessibility | 35% | 15–25 |
| East Asian typography | 25% | 15–25 |
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
| French | fr | partial (interface) | ~95% |
| Portuguese | pt-br (pt-PT none) | partial | ~95% |
| Indonesian | id | none | 0% |
| Japanese | ja | partial | ~95% |
| German | de | partial (#574) | ~95% |
| Korean | ko | partial (interface catalog; proofing unavailable) | 100% (2,463 entries) |
| Vietnamese | vi | partial (#1434) | ~95% |

Also shipped: Traditional Chinese (`zh-hant`), Ukrainian (`uk`), Serbian (`sr`, `sr-latn`, #250), Russian (`ru`) and Polish (`pl`), all partial (~95%). Bulgarian (`bg`) covers everything the interface audit finds (2,419 entries); Swedish (`sv`) covers that too, plus 44 newer strings no other catalog has yet (2,504 entries). Estonian (`et`), Italian (`it`, #594) and Czech (`cs`) now have complete interface catalogs, and Turkish (`tr`) a complete 2,325-entry one (every key of the Russian catalog), all with
system-locale selection and a saved language choice (Italian also has 51 strings none of the others had yet); Estonian, Turkish and Italian proofing are not included. Hungarian (`hu`) has a
complete 2,419-entry catalog, also without proofing. Vietnamese (`vi`, #1434) has a complete
2,448-entry catalog, also without proofing.
Proofing: English, Polish, Portuguese and Russian (spelling, grammar, hyphenation), plus Czech (system Hunspell spelling, generic checks), chosen per run from its language. Word ships
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
| 2026-10-16 | The Styles pane docks on the right or left or floats as a movable, resizable window (header menu, drag the title out, drop at a window edge to dock); `view.stylesPaneDock`; kept in the preferences, a floating pane saved off screen comes back (#1756) |
| 2026-10-14 | DOCX row table border exceptions render and survive saving. Explicit cell borders win; a row exception does not change other rows. |
| 2026-10-14 | Tab stops edited on the ruler: the corner selector picks Left/Center/Right/Decimal/Bar, a click sets a stop (Alt skips the snap), dragging moves it with a guide line and position tooltip, dragging off the ruler clears it; markers draw per kind and mirror for RTL; one `para.tabs` call and one undo step per gesture |
| 2026-10-12 | Fields keep `w:dirty` through DOCX saves (#877): a begin marker flagged by a generator that couldn't compute a result (pandoc-style TOC placeholders) keeps the flag on open/save — as a buffered field or a range field — so Word still refreshes it on open; computing a result (Update Field, Update Table) clears it, like Word's F9 |
|---|---|
| 2026-10-14 | Consistent preferred cell widths override a stale table grid when both totals agree, for fixed and AutoFit tables. Conflicting widths, spans and percentages remain unchanged. |
| 2026-10-14 | Legacy VML freeforms (`v:shape` with its own `path`), such as the rule lines in many older headers, are read as freeform shapes: they show on screen and in PDFs and are saved back as DrawingML custom geometry instead of vanishing |
| 2026-10-14 | Field instructions spanning paragraphs no longer add blank lines during pagination; empty result paragraphs still take a line. |
| 2026-10-14 | Page-break-only list paragraphs no longer print stray bullets or numbers. The next item keeps its number, matching the reference export. |
| 2026-10-12 | Backspace at the start of the paragraph after a table nested in a cell moves into the nested table's last cell, no longer into the last cell of the table around it |
| 2026-10-14 | Grouped `flowChartOr` shapes draw their circle and crossing spokes, as in Word |
| 2026-10-14 | Deleting a comment removes its anchors from headers, footers, notes and text boxes too, not just the body, and deletes the whole thread below it, replies to replies included; no anchor or reply is left pointing at a deleted comment (#1345) |
| 2026-10-14 | Legacy DOC DIB conversion checks the BMP header and copy before allocation, uses the caller's decode allowance, and caps PNG output while encoding; a BMP fallback is kept when it fits. A generated 1-pixel regression covers 57/58/512-byte limits. Resource-exhaustion consequences remain unmeasured. Related: #1334 and #1554 |
| 2026-10-13 | Justified vertical page alignment (#1320): a section set to Justified spreads each page's paragraphs and tables from the top margin to the bottom one, as Word does, including the last page and pages ending in a manual page break. DOCX `w:vAlign="both"` opens as Justified (it showed centred) and saves back; `layout.verticalAlign` takes `justify`, and Page Setup has a Vertical alignment box |
| 2026-10-13 | macOS shortcuts as in Word for Mac: ⌘⇧S Save As, ⌘⌥E Insert Endnote, ⌘⌥A New Comment, ⌃H Replace, ⌘⌥R collapse the ribbon |
| 2026-10-13 | The status bar flags a proofing language whose Hunspell dictionary isn't installed (Polish, Portuguese): "Polish (no dictionary)", with a tooltip naming the dictionary and where it is looked up (#1167) |
| 2026-10-14 | A file name that isn't valid Unicode, such as a Latin-1 `café.docx`, no longer crashes `wordcraft-cli` or the app (#1440). The app and the CLI's `convert`, `info`, `text`, `inspect`, `render`, `zotero` and `run --file` open such a file, and saving it without a new name (`run --file … --cmd file.save`, or Save in the app) writes back to it. Any other such argument is an error naming it, an output name included; the app exits with status 2 on a `--control` port that isn't valid Unicode |
| 2026-10-13 | A picture (or shape) set to Behind Text can be dragged over the text it overlaps, whether or not it was selected first, in the body and in a header/footer. The press becomes an object drag only after the pointer moves, so an ordinary click still places the text caret in the overlapping text. The UI regression tests cover In Front of Text too |
| 2026-10-13 | Saving to DOCX keeps the watermark: it is written as Word's own text-watermark shape in every header, and a document without headers gains one to hold it (#1290) |
| 2026-10-13 | DOCX files with Word's text watermark (Design › Watermark, a VML text shape in the headers) show it on every page on screen and in the PDF (#1290) |
| 2026-10-13 | Insert › Link has Word's Place in This Document: a link to a bookmark, a heading (through a hidden _Toc bookmark) or the top of the document, from the dialog's Link to: choice or `insert.link` with `bookmark`, `heading` or `top`; `link.targets` lists the places and Go To follows `_top` (#1234) |
| 2026-10-13 | Cut copies to the system clipboard however it is run (#1265): the ribbon, the context menu and the native menu cut deleted the words without copying them (only the Cmd+X event wrote the clipboard, so Paste re-inserted the previous copy); Copy/Cut publication is centralised in one sink and every entry point is tested |
| 2026-10-13 | Insert › Bookmark deletes bookmarks (#1236): the dialog is laid out as Word's, with Add, Delete and Go To beside the list, Sort by Name or Location and Hidden bookmarks; it opens on the bookmark around the caret, Delete and Go To keep it open, and Go To selects the bookmark's text. `insert.deleteBookmark {name}` removes both ends of a bookmark, also one across paragraphs (adding a bookmark under a name in use left such a one's end behind) |
| 2026-10-12 | Password-protected Word 97-2003 `.doc` files open after a password prompt (XOR obfuscation, RC4, RC4 CryptoAPI; wrong passwords are rejected by the stored verifier, #1079); written from the public specs and tested on synthetic files only, AES-encrypted `.doc` files are still refused |
| 2026-10-12 | Comments can show only for the selected text (#1055): Track Changes Options › Comments › "Only for the selected text" drops the margin column, and the comment under the caret, or the clicked one, floats beside the page (over its edge in a narrow window). The default stays "In the margin", as in Word; `review.trackingOptions {"commentDisplay"}` for agents |
| 2026-10-12 | Clicking inside a selected comment's balloon moves the caret in its text (#1013); the click used to leave the balloon, so the caret could only be moved with the arrow keys. A press on any balloon now belongs to the balloon, not the page behind it |
| 2026-10-12 | Page background pictures and textures (Design › Page Color › Fill Effects) are drawn (tiled or stretched) and kept on save instead of being dropped from the package (#999); PDF export paints them too |
| 2026-10-12 | Holding a click on the ribbon or toolbar no longer scrolls the document up (#1019): a double-click in the header or footer area left a text drag running, so the next press anywhere drove the edge auto-scroll; a drag now ends with the button, wherever it's released |
| 2026-10-12 | Tracked insertions and deletions inside a field result (a `=SUM(ABOVE)` subtotal) are kept as revisions: Accept All and Reject All act on them and a save writes the `w:del` back (#1037) |
| 2026-10-12 | Document title opens Rename Document; the account badge opens Options › General for user name and initials. `file.rename {name}` preserves file extensions and refuses an existing destination; unsaved names become the suggested save name without changing the Title property. |
| 2026-10-13 | Rounded rectangles (#1476): a `roundRect` keeps the corner radius its file gives it (`adj`), on screen, in PDF and when saved; all were drawn with a 16% corner |
| 2026-10-13 | PAGEREF in headers and footers (#1477): "Page X of Y" with Y a page reference to a bookmark at each letter's end shows each letter's own total instead of the result stored in the file |
| 2026-10-13 | Replace All respects locked content controls with Track Changes on or off; it refuses the entire batch before editing when any match would change locked content or remove a control locked against deletion |
| 2026-10-14 | File › Print translated in all 18 interface languages — printer list, duplex, grayscale, discovery/submission feedback and print commands (18 strings that shipped English-only with the craft-print integration; written from the English meaning, pending native-speaker review like the rest of the catalogs) |
| 2026-10-14 | All Caps and Small Caps draw ß as SS, as Word does (#1581): "Straße" in All Caps reads STRASSE, not STRASE, and other letters whose capital is two characters (ﬀ, ŉ) draw in full. The ß is still one character: the caret never stops between the two S, and a click on them lands before or after it |
| 2026-10-13 | Windows Magnifier follows the caret onto a new line: after Enter or Shift+Enter the view moves to the empty line at once, instead of staying on the line above until two more letters are typed (#1119) |
| 2026-10-13 | A window too big for its screen shrinks to fit when it opens (#1215): on a laptop screen the first window reached under the taskbar or past the screen edge, with the end of the scroll bar, so dragging the scroll bar stopped where the pointer met the edge. A window saved that size is fitted on its next launch too |
| 2026-10-14 | `layout.pageSetup` with a `section` checks its values like Margins, Size and Columns (#1582): non-finite numbers and margins that leave no room for text are refused with nothing changed; page sides, column count and gap, header/footer distances and line-numbering values are clamped to the same ranges |
| 2026-10-13 | `wordcraft --control` checks its port (#1426): a missing port, one outside 1–65535 or one that isn't a number prints an error naming it and exits with status 2, instead of starting without the control channel (and dropping `WORDCRAFT_CONTROL_PORT`); a flag right after it, such as `--sample` or `--version`, is read as a flag rather than as the port |
| 2026-10-14 | Text wrap (#1376): in the narrow strip beside a square-wrapped picture, a word that doesn't fit moves on whole to the next row; it used to be split across the strip ("docu" / "mentazione") |
| 2026-10-14 | DOCX files with legacy VML preset shapes show them (#1377): Word's right arrow (`_x0000_t13`) and the other presets appear with their fill, outline and size, drawn as WordCraft draws the same DrawingML preset (the nearest shape when it has no exact one), and saving keeps them as DrawingML shapes; before, they were dropped |
| 2026-10-14 | Lists numbered with Russian letters (`russianLower`, `russianUpper`) show а), б), в) … я), then аа), бб), as in Word, instead of 1), 2), 3); the 29-letter sequence (no ё, й, ъ or ь) was observed in Word |
| 2026-10-14 | "Do you want to save changes?" answers from the keyboard after typing too (#1597): Save takes focus from the page when the prompt opens, so the arrow keys and Tab move between Save, Don't Save and Cancel instead of the caret behind it, and Enter answers |
| 2026-10-12 | Text boxes from older Word files (VML) keep their internal margins, Resize shape to fit text and vertical alignment, so letterhead address blocks and form headers sit where Word puts them instead of 7 pt in and 4 pt down. Hidden drawings stay hidden (they were drawn as empty boxes) and are kept on save |
| 2026-10-13 | Word's privacy options are kept and honoured when saving .docx (#1062): with Remove personal information, comments and tracked changes are saved by "Author" and the file has no creator or last-modified-by; with Remove date and time, they are saved without dates. The open document keeps the real names and dates; `file.privacyOptions` sets both |
| 2026-10-13 | A selected comment balloon's Delete, Reply and Resolve buttons work again (#1085): pressing them no longer counts as a click on the page that closes the balloon first, so a new empty comment, or any other, can be deleted from its balloon |
| 2026-10-12 | Sample-text formulas on Enter (#1067): `=rand()` and `=lorem()`, optional paragraph/sentence counts, 5/3 defaults, original English and one-step undo |
| 2026-10-12 | File › Export › Word 2003 XML Document (#1081): documents save as WordprocessingML 2003 (`format: "wordml2003"` on the save commands, `wordcraft-cli convert --format wordml2003`); shapes, text boxes, charts and SmartArt don't survive, and it hasn't been opened in Word yet |
| 2026-10-12 | Direct run-level native InkML (`w:contentPart`) now survives DOCX edits and repeated saves with its relationship, content type and bytes intact. Native InkML rendering/editing and ink nested in groups or canvases remain missing |
| 2026-10-13 | The caret is black or white, whichever shows against what lies under it — the paper, a page colour, a dark page, shading or a highlight — whatever the UI theme, as Word's inverted caret does; before, it stayed black on dark page colours and highlights and could vanish (#299) |
| 2026-10-13 | Clicking the ribbon's tab names, empty space, gallery tiles or View check boxes, or the status bar's view buttons and language menu, no longer makes the caret vanish: typing goes on in the document as in Word, without another click on the page (#1093) |
| 2026-10-13 | Dragging a Square-wrapped shape or picture on a multi-column page wraps the text of the column it lands in around it, and its old place closes up (#884) |
| 2026-10-13 | Pagination with footnotes follows Word (#1116): a paragraph ending on a page with footnotes keeps its space after above the separator, otherwise its last lines move to the next page with the note; pages without notes are unchanged |
| 2026-10-13 | Interface size means what it says (#822): a chosen size is absolute, as in PhotoCraft and PdfCraft, so "100%" is one screen pixel per point even where the system reports 200 % (a bare window manager on X11, where winit guesses the scale from the monitor's DPI). The new default, Auto, keeps the system's display scale and its menu entry shows the size it draws at; sizes run 75–300 %. A saved size from before stays the same if it was the default (now Auto) and becomes absolute otherwise |
| 2026-10-14 | Copied text keeps its formatting in other programs (#1523): Copy and Cut put HTML beside the plain text on the system clipboard for every selection, not only one holding an equation, so a browser, mail client or another word processor pastes bold, italic, fonts and sizes. RTF on the clipboard is not written yet |
| 2026-10-13 | Copying an equation also puts MathML in HTML on the clipboard beside the linear text like `y=2x_s` (#816; Word/LibreOffice paste not yet verified; OMML is #1297). HTML export writes equations as MathML too, and opening or pasting HTML with MathML gives equations back |
| 2026-10-13 | The Equation tab's galleries, structure buttons and symbols keep the theme's colours on Windows: their pictures share a few textures written in place (no wipe of pictures already on screen, room for the built-ins at high display scales), instead of hundreds of small textures freed in bulk, which on DirectX 12 showed inverted, red or teal tiles (#1002) |
| 2026-10-13 | `file.close` over the control channel or MCP bridge no longer quits and discards unsaved changes (#1268): it returns an error unless the document is saved or `{"force": true}` is passed; the user's Save / Don't Save / Cancel prompt is unchanged |
| 2026-10-14 | Text copied out of an exported PDF reads as shown (#1473): All Caps and Small Caps text comes out in capitals, as from Word's PDFs, and plain text after it keeps its own case (before, a capital first drawn for a lowercase letter made later capitals extract lowercase) |
| 2026-10-14 | Web: Save in the "save changes?" prompt before New or Open now waits for the browser's save picker: the new or opened document replaces yours only once the file is written and closed. Cancelling the picker, or a write that fails, keeps your document open with its edits and undo, still unsaved, and the prompt comes back with the reason; a save that ends after another document has replaced it no longer runs the New/Open on that one or marks it unsaved, and New/Open while a picker save is still in flight ask first (#1315) |
| 2026-10-13 | Closing with unsaved changes → Save that can't write the file (no permission, locked by another app, folder gone) now says so plainly instead of looking like Cancel: the prompt comes back with the reason in it ("Couldn't save … so it is still open and nothing was lost") for Save again, Don't Save or Cancel, the status bar says it too, and `ui.saveChanges` returns that error; the same for a Save As from the prompt; the document stays open with its edits (#1445) |
| 2026-10-14 | File › Info › Inspect Document finds hidden text the way Word does (#1344): hidden by direct formatting, a character style or a paragraph style, in the body, headers, footers, footnotes and endnotes, comments and text boxes, and Remove All deletes all of it (and nothing visible) in one undo step, whole hidden paragraphs and the notes of hidden references included. The result lists anything it couldn't reach (`residual`), so a sanitised file is never reported clean by mistake |
| 2026-10-14 | ODT files keep their tracked changes (gap 19): insertions and deletions made in LibreOffice open as revisions with their author and date, deleted paragraph breaks and text inserted then deleted included, so Accept All and Reject All give the right text; saving to ODT writes them back, and the Track Changes setting goes both ways |
| 2026-10-14 | RTF keeps tracked changes (gap 19): insertions and deletions open as tracked changes with their authors and dates (`\revtbl`, `\revised`, `\deleted`, `\revdttm`) instead of plain text, so deleted text no longer reappears as normal text; saving to RTF writes them back, and Accept All / Reject All work after reopening |
| 2026-10-14 | Alignment guides: moving a picture, shape or text box snaps its edges or centre to the page's edges and centre, the margins and the other floating objects on the page, with a thin line across the page for each guide while dragging; Alt moves freely; File › Options › Advanced › Editing turns them off (`ui.alignmentGuides`). Our own design: Word for Mac shows no guides |
| 2026-10-14 | ODT files open with their sections (#1379): a paragraph that switches to another page style starts a new section on a new page, with that page's size, orientation, margins and columns and, when the file asks, page numbering restarting; a two-column `text:section` becomes a continuous two-column section. Before, the whole document was one portrait section, so landscape pages, columns and restarts were lost on open and on save to DOCX |
| 2026-10-14 | Hebrew and Arabic in exported PDFs copy and search as whole words (#1383): right-to-left glyphs are written left to right as they appear, the order Chrome and Edge (PDFium) and poppler read text back from (checked by hand), instead of in reading order, which they reversed (`שלום` came back as `םולש`, Arabic in pieces). Lam-alef stays whole and vowel marks follow their letters, though Chrome and poppler can still break a fully vowelled word with spaces. The page looks the same. Readers that take glyphs strictly in file order get only plain Hebrew runs right, through `/ReversedChars`; screen readers are unverified |
| 2026-10-14 | HTML import (#1381): header cells (`<th>`) are bold unless their own CSS says otherwise, and `text-align` on a cell, its row, row group or table aligns the cell's paragraphs (a paragraph's own alignment wins), so `wordcraft-cli convert page.html out.docx` writes the bold header and `w:jc` right-aligned cells. HTML, Markdown and plain text, which give no page size, open on the region's paper (A4 in Italy, Letter in the US) like a new blank document, in the app and in `wordcraft-cli`; documents with their own page size keep it |
| 2026-10-14 | Custom page sizes (#1433): Layout › Size ends with More Paper Sizes…, which opens Page Setup's new Paper group — a Paper size list (the named sizes, or Custom size) and Width and Height in the measurement unit; OK sets the size only when it changed, and a wider-than-tall size comes out landscape. `layout.size` now clamps each side to Word's 0.1"–22" and refuses a page the margins don't fit (the margins rule: 36 pt of text room), and Page Setup checks the size and margins together before applying either |
| 2026-10-13 | Highlighted text reads on a dark page (View › Switch Modes, #1252): highlights and character shading are dimmed, keeping their hue (yellow turns a dim olive, as in Word), and the text on them is drawn light (coloured text below 4.5:1 is drawn white, and underlines and strikethroughs follow the same rule), instead of white text on bright yellow; light page, print and exports unchanged |
| 2026-10-13 | Full screen (#980): F11, or View › Window › Full Screen, fills the screen with the window (⌃⌘F on macOS); F11 again or Escape gives back the window as it was. Next Field, which Word puts on F11, keeps Word's Alt+F1 and Alt+Shift+F1 (and Shift+F11). The web app leaves full screen to the browser's own F11 |
| 2026-10-13 | Double-clicking the top or bottom margin of a page creates its header or footer when it has none and puts the caret in it, as in Word (#960): the one that page shows (different first page, odd and even pages), only in Print Layout; one left empty is taken away again when you leave it, so nothing empty is saved. `insert.editHeader`/`insert.editFooter` take `{"page"}` for agents |
| 2026-10-13 | Alt+numpad character codes, as in Word on Windows (#961): hold Alt, type a decimal code, release Alt — `0233` → é (Windows-1252), `233` → Θ (OEM code page 437; 850 in Western European locales and English outside the US), codes above 255 are Unicode (`8364` → €). Not on macOS; `text.typeCode` for agents |
| 2026-10-13 | Review › Language and the status bar's language menu offer Set Proofing Language… again (#860): the Language dialog marks text as French, German or any of 29 languages and Set As Default makes it the document's language; Language Preferences… opens Options › Language. Text in a language outside the short list shows by name in the status bar and checked in the menu. Spelling and grammar remain English, Polish and Portuguese; text in other languages gets no marks |
| 2026-10-12 | A list label moves with its text when the paragraph is centred or aligned to the end (it stayed at the margin), in left-to-right and right-to-left paragraphs (#1159) |
| 2026-10-13 | A paragraph that ends with a page break keeps its space after on the page before, as in Word: the next page no longer starts up to a line too low, and a page break before right after it no longer leaves a blank page |
| 2026-10-13 | Tables break across pages like Word's: a cell without its own margins lines up with its neighbours, exact row heights include the bottom margin, and a row whose margin doesn't fit at a page bottom is split with widow control instead of moving to the next page whole |
| 2026-10-13 | Equation parsers are hardened (#1456): deeply chained input (`x^1^1…`, `a/a/…`, `\sum\sum…`) no longer builds a tree thousands of levels deep that overflows small stacks, `\text{…}` is escaped in LaTeX output so it reads back unchanged, and runs merge inside every structure. |
| 2026-10-13 | A WMF with a million object records opens in milliseconds instead of a second, and a large polygon or a small crop of a large bitmap no longer builds a second full-size copy while it is read (#1463) |
| 2026-10-13 | PDF export (#1474): a horizontal or vertical line inside a group (a divider in a grouped box) is drawn, as on screen; it was left out because it has no height |
| 2026-10-14 | A paragraph that runs on into the next column or onto the next page wraps around the shapes there (#1374): its continued lines are broken again for that column's wrap areas instead of running through a shape there (or around one left on the page before); the lines before, and the paragraph's own floating objects, stay where they were. Paragraphs with nothing new to wrap around are not laid out again |
| 2026-10-13 | A PAGE field in the body shows the number of the page it is actually on (#1352): after a page break inside its paragraph, or when the paragraph runs over onto the next page, it no longer repeats the first page's number; the rest of the paragraph is broken again for the wider number (page 9 to 10). Only paragraphs with PAGE fields are laid out again |
| 2026-10-14 | RTF import keeps sections (#1378): a file with portrait and landscape parts, a two-column stretch or a page-numbering restart opens with one section each, as in Word, instead of one landscape section for the whole document. Each keeps its page size and orientation, margins, header/footer distance, columns, break type (continuous, next page, odd/even, column), page-number restart, start and format, title page and its own headers and footers; a section without its own links to the previous one's |
| 2026-10-14 | The system Save As dialog's file-type list names every format (Word Document, PDF, OpenDocument Text, Rich Text Format…) instead of one "Documents" entry, with the document's own format first (#1399) |
| 2026-10-14 | Colour emoji show on the page (#1269): 🧠, ✅ and other emoji were blank on screen and in `render_page` (only PDF export had them). Glyphs from colour fonts are now drawn in the font's own colours: bitmap emoji (Apple Color Emoji, Noto Color Emoji) and COLR fonts (Segoe UI Emoji, newer Noto builds, with gradients). A dark page doesn't invert them, the same as pictures |
| 2026-10-14 | `.doc` tables keep their cell shading (#1380), from Word 97 through Word 365, so a dark header row with white text is readable again, and lose the extra empty column at the right of every row; a table no longer takes the previous table's columns, borders or alignment. Rows merged across the table (a legend) come with #1123 |
| 2026-10-12 | On the web, pasting a picture while a comment, the Find box, a dialog field or the Backstage has the keyboard no longer inserts it into the document; it inserts once only when the page is the edit target (#1349) |
| 2026-10-14 | Table deletes, Change Case and Unlink Fields respect Track Changes and locked content controls (#1329): with tracking on, Delete Table, Rows, Columns and Cells mark the cells' text deleted and keep the table (like Word), and Change Case is a tracked deletion and insertion, so Reject All restores the original; all of them refuse to delete a control locked against deletion or change content locked against editing |
| 2026-10-14 | Draw: Ink to Shape recognises arrows. A line and a V-shaped head drawn in one stroke (shaft first or head first, any direction) becomes a straight arrow connector with the head at the tip, in the stroke's colour and width; a lone V, a line with one barb and scribbles stay as they are. Our own recogniser: Word for Mac has no Ink to Shape to compare. Two-stroke arrows and arrows with a closed triangle head not yet. |
| 2026-10-12 | A DOCX hyperlink whose relationship has an empty target (as LibreOffice writes) is no link (#1005): no "Ctrl+Click to follow link" tooltip over the text, which keeps its Hyperlink style |
| 2026-10-12 | Table eraser beside vertically merged cells (#1039): the border between two cells merged down the same rows merges them into one cell that stays merged down those rows; a border whose two sides can't make one rectangle is hidden on both sides along the whole edge instead of refused |
| 2026-10-13 | Styles gallery for DOCX files that mark no style `w:qFormat` (LibreOffice) shows Word's usual quick styles: Normal, No Spacing, Headings, Title, Subtitle… (#1238) |
| 2026-10-13 | Pictures can be cropped on the page: Picture Format › Crop shows black crop handles; dragging one trims that side (a corner, two) while the picture keeps its scale and the visible part stays put, dragging back out restores it, and the cropped-away part shows under a dark dim. Esc, Enter, a click outside or Crop again keep the crop; each drag is one undo step |
| 2026-10-13 | Table Layout › Cell Size: the Height and Width boxes apply to every row and column the selection covers (a selected table changed only its last row and column), and to the caret's row and column without a selection (#1242) |
| 2026-10-12 | Forms protection: filling content controls under `w:edit="forms"` (#1077) — entries and typing inside fillable controls run, every other edit rolls back ("only the editable regions of this protected document can change"); `review.restrict` reports `formFields`; Find Next Region covers form fields (pane button shows for forms documents); a save/reopen keeps the protection, the filled values and the password guard |
| 2026-10-12 | RTF import now uses the RTF spec's defaults where a file names none (#902): no space before or after paragraphs, single line spacing, and the `\deff` default font, instead of WordCraft's Normal style; RTF files that relied on the old defaults open tighter, like in Word |
| 2026-10-12 | `--help` and `-h` print the usage and exit (#968): `wordcraft --help` lists the app's options without opening a window (it used to try to open a file named `--help`), and `wordcraft-cli <command> --help` works for every command instead of failing with `unknown option --help` |
| 2026-10-11 | Right-clicking a table cell offers the table's commands, and as in Word they follow the selection: with the caret in a cell Insert, Delete Cells, Split Cells, Text Direction and Table Properties…; with several cells Merge Cells instead of Split; with whole columns or rows Delete Columns or Rows, Merge Cells and Distribute Rows and Columns; with the whole table Delete Table, Distribute, AutoFit and Insert Caption…. A right-click beside selected cells moves the caret there instead of keeping them |
| 2026-10-11 | Drags, Shift+arrows and Shift+click over table cells select whole cells, as in Word: from one cell into another the rectangle between them (bold, alignment and Delete leave the other cells alone), into or out of a table whole rows; extending again goes on from where the selection started |
| 2026-10-11 | More of the table mouse: arrows above a column, left of a row and along a cell's left edge select it (a drag selects more; a selected column is edited cell by cell, numbering it makes one list), "+" buttons beside the borders insert a row or column, the move handle past the top-left corner selects the table or drags it to float where it's dropped, and a double-click on a column border fits the column; Select Column in the ribbon; `table.selectAt`, `table.insertAt`, `table.moveDrag`, `table.fitColumn` and `table.selectColumn` for agents |
| 2026-10-12 | Table fixes (#971): a double-click on a nested table's column border fits the column within the cell around the table (it could run out to the page's right margin); a selection from text into or across a table shades the whole cells of the rows it takes, not just their letters; Shift+Right at the end of a cell's text selects that cell first, as in Word, and the next press takes the next cell too |
| 2026-10-12 | AutoCorrect keys its built-in English replacement list by language (#835): Polish, Croatian, Bosnian and Serbian `i` ("and") is no longer turned into `I`; English and untagged text unchanged |
| 2026-10-12 | Ribbon groups fit above their labels: View › Show splits its four check boxes into two columns (Hide White Space, #645, had pushed the fourth onto the "Show" label), Developer › Controls loses the gaps that ran into its label, and Layout › Paragraph (#654) no longer trips a debug assertion and sits 2.5 pt higher; a test draws every tab in every language and fails on any group taller than the ribbon |
| 2026-10-12 | Down and Up from the start of a line beside a table go into it (#1007): the caret, left of the first cell's text there, used to skip the whole table to the paragraph beyond it; it now lands in the cell under it or the nearest one, as in Word |
| 2026-10-12 | Window caption: a non-Latin file name is drawn with a document fallback face (or the CJK UI font), instead of `?` |
| 2026-10-12 | Comments: Arabic, Hebrew and other non-Latin comment text and author names are drawn with a document fallback face (or the CJK UI font) in the balloons and Comments pane, instead of boxes; read-only balloons show right-to-left text in reading order, set against the right edge |
| 2026-10-13 | Word 365 cover pages (#743): shapes placed relative to the page's height now sit where Word puts them (they were at the top of the column), and a freeform drawn back to its start without being closed is filled, so the cover's coloured panel and its white title show |
| 2026-10-12 | 32-bit Windows starts on OpenGL. DirectX 12 graphics pipeline creation returns "The parameter is incorrect" and the app exited before the window appeared |
| 2026-10-12 | New documents follow the system region (#838): A4 and centimetres outside the US (Letter in the CLDR Letter territories, inches in the US), lengths written with the region's decimal separator (`2,54 cm`) and typed with either; an explicit Measurement units choice wins; `file.options {"region"}` for agents |
| 2026-10-12 | Paste from other programs keeps their formatting (#687): Ctrl+V and Home › Paste read the system clipboard's HTML (and RTF on Windows and Wayland); Merge Formatting takes the destination's font and paragraph and keeps bold, italic, underline and strikethrough; Paste Special lists what the system clipboard holds (HTML, RTF, picture, text). Linux: pictures copied in a browser or Spectacle paste on KDE and other Wayland desktops (data-control clipboard), and a paste that finds nothing usable names what the clipboard holds instead of "empty" (#833) |
| 2026-10-12 | Table style editor (#714): New/Modify Table Style edits each of the 13 table style parts (header row, total row, first/last column, odd/even bands, corner cells) with font, colour, fill, borders and cell alignment; every table using the style follows; all parts round-trip in DOCX; `table.modifyStyle` takes `part`/`parts` and per-edge borders |
| 2026-10-12 | STYLEREF fields (#775): running headers and footers show each page's chapter title (page's first, or `\l` last, paragraph of the style, else the nearest before), style names, ids, heading levels and character styles, number switches; body STYLEREF updates with the other fields; STYLEREF in the Field dialog; PDF export follows the layout |
| 2026-10-12 | Undo history list (#701): a ▾ beside the Quick Access Toolbar's Undo lists recent steps newest first; hovering highlights what a click undoes ("Undo 3 Actions"), and a click undoes them in one go. `edit.undoTo {count}` and `edit.undoHistory` (newest first, up to 100) for CLI and MCP |
| 2026-10-12 | Start screen (#766): launched with nothing to open, WordCraft starts on File › Home (Blank document, templates, Recent) like Word, desktop and web; Esc leaves a blank document; File › Options › General › Start-up turns it off |
| 2026-10-13 | No document behind the Start screen (#1129): Esc or Back from it shows an empty window (no page, ribbon greyed out but for Help) until Blank document, a template or a file is opened, as in Word; closing it asks nothing |
| 2026-10-12 | Word 365 cover pages (#743): a freeform inside a group keeps its shape (it was drawn as a rectangle); text boxes keep their own internal margins and Resize shape to fit text, so a tight title box no longer loses its second line; both survive saving. The missing fills in the report were theme-coloured shapes, fixed by #611 |
| 2026-10-12 | Alt+Q moves to "Tell me" command search, as in Word for Windows (#819); not on macOS, where Option+Q types a character |
| 2026-10-12 | PDFW: PDF export uses the shared craft-pdf writer for metadata, font preparation and serialization while retaining WordCraft layout, drawing, tagging, links and font-outline retry. Writer-only builds do not enable PDF import codecs. |
| 2026-10-12 | PDF export of long documents with many internal links (a linked table of contents) is linear: NIH's 67-page protocol template 10.6 s → 0.9 s |
| 2026-10-12 | Keytips: Word for Windows' tab letters (File F, Design G, References S, Review R, View W, Help Y) |
| 2026-10-13 | Keytips reach the table tabs as in Word: Alt, J, T for Table Design and Alt, J, L for Table Layout, then their controls' badges (#1183) |
| 2026-10-12 | Table-of-contents links look like Word's: plain text, not blue and underlined (Word ignores the Hyperlink style on TOC entry links; measured in 9 cases) |
| 2026-10-12 | Line heights like Word's: spaces and tabs never make a line taller (13 cases measured in Word for Windows); tables of contents with big tab runs were a third taller |
| 2026-10-12 | `.doc` import: tab stops (centred titles, right-aligned dates, leaders) and sections ending at their section marks (continuous breaks, footers); EPA .doc templates now paginate like Word |
| 2026-10-12 | DOCX headings numbered like Word when a heading style gives only a list level (6.4.5, not 6.15) |
| 2026-10-12 | A list label makes its line taller above the baseline only, as in Word (6 cases measured in Word for Windows): Symbol bullets no longer add their deeper descent, so bulleted lists keep Word's height and page breaks |
| 2026-10-12 | Hidden end-of-cell marks (`w:hideMark`) read, written and laid out like Word (9 cases measured in Word for Windows): a cell's empty last paragraph takes no room, so rows of such empty cells shrink to their borders |
| 2026-10-12 | Text frames (`w:framePr`) in headers, footers, cells and text boxes: placed apart with text wrapping around them, and kept on save (a framed footer page number took a line of its own and was lost on save) |
| 2026-10-13 | A paragraph with more than 65,536 distinct formats keeps each one (#1340): the layout numbered a paragraph's formats with 16 bits, so every run past that drew with the 65,536th format's font, size, link and tracked-change marking |
| 2026-10-12 | List indents like Word's when a numbered style has indents of its own: the style giving the list (and styles based on it) beat the level's indents, and a paragraph with its list switched off drops the styles' indents (27 cases measured in Word for Windows); NIH's Heading 3s now sit at their 0.9" |
| 2026-10-12 | Keep with next like Word (20 cases measured in Word for Windows): a kept paragraph moves to the next page only when it would fit on this one by itself, otherwise it breaks as usual; chains of kept paragraphs move together |
| 2026-10-12 | An empty paragraph that only carries a continuous section break takes no room, as in Word (10 cases measured in Word for Windows): text after it follows right on and balanced columns don't count it |
| 2026-10-12 | Floating objects positioned relative to their paragraph start above its space before, as in Word (10 cases measured in Word for Windows) |
| 2026-10-12 | Unequal columns: text after a column break is broken for the column it lands in (NIH's SARP form had a paragraph a word or two wide) |
| 2026-10-13 | New comments, notes, text boxes and lists in a document whose file used the largest ids (#1338): they take a free id instead of overflowing (a crash) or reusing one already taken (a new list could pick up another's definition); `Document::try_add_part` and `Numbering::try_add_list` report ids running out instead |
| 2026-10-12 | Space before at the top of a page like Word after page breaks (32 cases measured): a paragraph ending with a page break keeps its mark with it, the next paragraph gets no space before; page break before drops it in Word 2013 mode |
| 2026-10-12 | Automatic paragraph spacing drops between list items, as in Word (16 cases measured in Word for Windows): bulleted items that ask for automatic space before and after now touch, as in NIH's monitoring plan |
| 2026-10-12 | Rendering like Word for Windows, found by comparing real .gov documents page by page: text is kerned only where Kerning for fonts asks for it (line breaks now match Word's), and automatic paragraph spacing (`w:beforeAutospacing`/`w:afterAutospacing`, common in pasted web text) is 14 pt, dropped at the document's and cells' edges |
| 2026-10-12 | Keyboard context menu (#821): Shift+F10 opens the page's right-click menu below the caret; `ui.contextMenu` |
| 2026-10-12 | The Font list no longer keeps every family it previews in memory (#848): the Font box drew each family's name in its own face by loading the whole family for good, so every trip through the list kept tens of megabytes (1.47 GB after 40 changes on macOS); entries are now drawn from a face read for the preview alone and freed (120 families previewed: +17 MB RSS instead of +411 MB); only fonts actually used load |
| 2026-10-13 | An empty file opens (#1384): the zero-byte .docx that File Explorer's New › Microsoft Word Document makes opens as a new blank document (the region's paper, the editing language), and Save writes it back to that file instead of asking for a name |
| 2026-10-12 | Scrolling a document with one very large table stays smooth (#811): pages are fingerprinted for the texture cache once per layout and only when shown (not every page every frame), the markup-area check is cached per revision, and textures far from the view are dropped. The issue's 1,096-page table: ~600 ms → ~2.4 ms idle frames, ~590 ms → ~4.5 ms scroll frames |
| 2026-10-13 | Paragraph dialog over differently formatted paragraphs (#1346): a field they differ in shows blank (a check box neither on nor off, no direction picked) and OK leaves it as each paragraph has it; OK sends only the fields that changed, as one undo step, instead of giving every paragraph the caret paragraph's formatting in three undo steps |
| 2026-10-12 | Search (#819): Alt+Q, or Alt then Q, opens the Search box from anywhere with nothing typed into it; queries match every word; `tools.search` for agents |
| 2026-10-12 | Section breaks (#852): Layout › Breaks with text selected puts the break before the selection and keeps the text, as Word does; it used to delete it, leaving the first section empty |
| 2026-10-12 | Find, Replace and Go To like Word (#712): Ctrl+F types into the Navigation pane's search, one tabbed Find and Replace dialog, Go To by page (absolute or relative) and bookmark; `edit.findReplace`, `edit.goto` `pageBy` |
| 2026-10-12 | Symbol fonts (#776): text in Wingdings, Wingdings 2, Wingdings 3, Webdings and Symbol (proprietary, never bundled) shows its intended symbols — full code tables from Unicode's published correspondence, both storage forms (`w:sym` private-use codes and plain codes); PDF text, copy and TXT export use the Unicode characters; the saved text is unchanged |
| 2026-10-11 | Hide White Space (#645): `view.hideWhiteSpace` (View › Show checkbox, or double-click the gap between pages) collapses margins, headers and footers between pages to a thin line in Print Layout; screen only, page count unchanged |
| 2026-10-11 | Fonts: Palatino Linotype and Book Antiqua fall back to Palatino (macOS) or TeX Gyre Pagella / P052 / URW Palladio (Linux) before generic serif (#670) |
|---|---|
| 2026-10-12 | Czech proofing: cs-CZ spelling reads the installed Hunspell dictionary (`hunspell-cs` on Debian/Ubuntu), suggestions use runtime MAP/REP/TRY/KEY tables, generic repeated-word and spacing checks explain in Czech; Review › Language offers Czech/Čeština; the Czech interface writes new documents and opened TXT/Markdown in cs-CZ. Affix headers accept trailing comments. No system dictionary data or Czech hyphenation patterns bundled; parity estimates unchanged |
| 2026-10-12 | Tables: dragging the border of one selected cell (Select Cell, or a click along its left edge) moves it in that cell's row only, as in Word, like the border of cells selected across (#962) |
| 2026-10-13 | Korean interface catalog completed: all 2,463 entries translated; the ko-KR system locale and saved language choice select the interface. The UI prefers Korean embedded/system font faces so Hangul renders instead of boxes, and print settings and printer feedback are translated. Korean proofing remains unavailable; macOS Korean IME fixes are still pending (#163, #164) |
| 2026-10-12 | Word default shortcuts expanded (gap #24): 140 commands bound on macOS (169 combinations), 158 on Linux/Windows (188 combinations). Bound Style Inspector (Shift+F1), Macros (Alt+F8), Strikethrough (Mod+Shift+X), Find Next (Shift+F4), Copy/Paste (Mod+Insert, Shift+Insert), Go Back (Mod+Alt+Z), File Open/Save/Save As/Print (Mod+Alt+F2, Alt+Shift+F2, Alt+F2, Mod+Alt+I), Field navigation (Alt+F1, Alt+Shift+F1), Outline show first line (Alt+Shift+L), Next Footnote (Alt+Shift+>), Link to Previous (Alt+Shift+R), AutoText (Alt+F3), Spelling on Mac (Mod+Alt+L) |
| 2026-10-13 | Web and Draft Layout now render visible full-resolution raster tiles for exceptionally tall pages rather than downsampling the entire page into one texture; verified with a 440-paragraph headless UI reproduction |
| 2026-10-12 | Track Changes: Accept All after a tracked deletion from the end of one paragraph into a later one no longer joins the paragraph that follows the range, and Reject All after typing into an empty paragraph keeps that paragraph (#864) |
| 2026-10-12 | Ctrl+drag copies an object (#1014): holding Ctrl (⌘ on macOS) while dragging a picture, shape or text box leaves the original and drops a copy where you let go (text boxes get their own text); the copy is selected, the clipboard is untouched, one undo step; `arrange.copyTo` |
| 2026-10-11 | Table border drags like Word's (#711): Ctrl shares a column border's change among the columns right of it (the table keeps its width), the table's left edge moves a left-aligned table's indent (a centred or right-aligned table keeps its alignment, as in Word), a border of cells selected across moves only in their rows, and Alt shows the column widths on the ruler |
| 2026-10-11 | Tables on the rulers (#700): with the caret in a table the horizontal ruler marks its row's column borders and the vertical ruler its rows' bottoms; dragging a marker resizes the column (the columns right of it move along; with Shift the neighbour makes up the difference) or the row, one undo step, Escape cancels; in a cell the indents and tabs are measured from the cell |
| 2026-10-12 | Save Changes prompt answers from the keyboard: Save has focus when it opens; arrow keys and Tab move between Save, Don't Save and Cancel; Enter answers (#861) |
| 2026-10-12 | Builds with craft-fonts draw simplified-Chinese characters in documents from the bundled Noto Sans CJK SC (it was only used by the interface), so a CJK run gets one face covering it; fixes the FreeBSD job's `east_asian_run_uses_one_face_covering_the_whole_run` failure after #567 |
| 2026-10-12 | Insert › Picture, drag-and-drop and paste take Windows Metafiles (`.wmf`) and Enhanced Metafiles (`.emf`) at their own frame size (#802) |
| 2026-10-14 | A picture or icon inserted by a command (`wordcraft-cli run`, MCP, control channel) leaves the caret after it, so the next `text.newParagraph` or `text.insert` no longer deletes it, and two inserted in a row are both kept; Picture Format commands still act on it, and a picture the user inserts or pastes in the app is still selected (#965) |
| 2026-10-12 | AutoCorrect capitals (#544): Correct TWo INitial CApitals (with exceptions), accidental cAPS LOCK, first letter of table cells; each undoable and switchable |
| 2026-10-11 | Turkish (`tr`) interface completed (#774): 2,325 entries, every key of the Russian catalog (equation gallery, built-in style names, watermarks, paper sizes, preview tiles); the interface audit checks Turkish too |
| 2026-10-12 | Navigation headings truncate within the resizable pane, with an ellipsis for long titles and full text on hover (#759) |
| 2026-10-11 | Reading Mode: existing View ribbon button now opens a read-only, viewport-column layout with animated horizontal paging, transient display controls, heading navigation (Navigation pane) and resize anchoring; document layout and export unchanged |
| 2026-10-11 | DOCX open/save keeps inherited complex-script size, bold and italic independent of Latin formatting; explicit values are retained, and font commands still format both scripts. Covered by synthetic round trips and a local GUI save/reopen check. |
| 2026-10-11 | Czech (`cs`) interface catalog: 2,021 entries (the full zh-hans key set), Czech locale detection and persisted language choice |
| 2026-10-12 | Portuguese proofing: pt-BR spelling against the installed Hunspell `pt_BR` dictionary (VERO; read at runtime, not bundled) — the affix engine grew `FLAG UTF-8`, 128 flag bits, continuation classes and `FORBIDDENWORD` to read it — suggestions tuned for Portuguese (missing accents first), conservative grammar rules with Portuguese messages, TeX Portuguese hyphenation patterns (BSD 3-clause, attributed); Review › Language offers Português (Brasil); a pt-br interface writes new documents in pt-BR |
| 2026-10-11 | Polish: interface (`pl`, the thirteenth language, every catalog key) and proofing — spelling against the installed Hunspell `pl_PL` dictionary (read at runtime, not bundled) through a new Hunspell-style affix engine, suggestions tuned for Polish, Polish grammar rules with Polish messages, TeX Polish hyphenation patterns (MIT option); proofing language per run (`w:lang`) for squiggles, F7, suggestions and hyphenation, languages without tools left unmarked; Review › Language menu and the status bar's language; a Polish interface writes new documents in Polish |
| 2026-10-11 | German (`de`) interface (#574): 2,021 translated entries covering the full zh-hans key set, locale and saved-preference tests, bundled-font checks; no German proofing resources |
| 2026-10-12 | Sorting and table numbers follow the text's language (#836, #837): Table › Sort, Home › Sort, the index (with its letter headings), the bibliography and the Table of Authorities use the language's alphabet (Polish `Łódź` between `Kraków` and `Zakopane`, Swedish `å ä ö` after `z`, German `ä` with `a`); table sort and formulas read `1,5` and `1 250,00` as Polish decimals and `=SUM` writes `5,75` |
| 2026-10-14 | `.doc` notes, headers and text boxes read their text once (#1319): a story table that steps back no longer has many footnotes, endnotes, headers or text boxes copy the same text into a part each; a story starting inside an earlier one is dropped (a header or footer left empty is linked to the previous section's), so the stories together never hold more than the file's text |
| 2026-10-11 | `.doc` import keeps pictures and drawings (#551): inline and floating pictures (PNG, JPEG, DIB, EMF, WMF), shapes, lines, groups and text boxes with position and wrap, from [MS-ODRAW] |
| 2026-10-14 | ODT and `.doc` pictures stay within their byte budget (#1334): an ODT picture entry is inflated once however many frames point at it, and only when the size it declares fits in what is left (one inflating past that uses the budget up); a `.doc` inline picture that can't be read or doesn't fit is no longer decoded and copied again for every anchor on it, and is decoded only within what is left |
| 2026-10-11 | Tables resize with the mouse (#217): drag a column border (the neighbour makes up the difference; Shift moves one column), a row's bottom border, or the handle past the bottom-right corner to scale the table; a dashed guide while dragging, one undo step per drag; `table.resizeDrag` for agents |
| 2026-10-11 | Animation effects setting: File › Options › Accessibility › Animation effects (Use system setting, On, Off). With the system's animations off (Windows' Animation effects, macOS' Reduce motion, the Linux/BSD desktop's setting, a browser's reduced motion) the page no longer smooth-scrolls: wheel notches and Ctrl+wheel zoom land at once, in the document and in every list and pane, and menus and buttons don't fade; `ui.animations` for agents |
| 2026-10-11 | Linux/BSD: Open, Save As, Export and Insert Picture work without a file chooser portal or zenity (labwc, sway…): WordCraft shows its own file browser and says how to get the system dialog; File › Open › Browse in WordCraft… is always there (#576) |
| 2026-10-11 | Every measurement follows File › Options › Advanced › Measurement units (#755): the Line Numbers "From text" distance and the Hyphenation zone, still fixed in inches after #485, now convert like every other dialog |
| 2026-10-11 | Review › Track Changes Options › Show revisions in balloons: tracked deletions in the body leave the text for "Deleted: …" balloons in the margin, joined to where they were (the option was saved but drew nothing new before) |
| 2026-10-11 | Security review fixes: `ui.render`/`ui.screenshot` without a `path` (and the MCP bridge's `render_page`/`screenshot`) return the PNG in the reply instead of writing a fixed name into the shared temporary folder; exported HTML/Markdown write `href` only for web, mail, bookmark and relative links, never `javascript:` and other schemes (one allow-list in `wordcraft_doc::link`) |
| 2026-10-11 | Design tab: the style set gallery gets a box and gaps of its own, so it no longer runs into Colors and Fonts (#638) |
| 2026-10-11 | File › Print: page preview, page range (all, current, selected, custom 1-3,5), odd/even, copies and collate go into the printed PDF (`file.printPdf`, `ui.print` params); printer, duplex and pages per sheet are still the system dialog's (#641, stage 1) |
| 2026-10-11 | A vertically merged table cell (eraser on a horizontal border) lays out, shows and clicks as one cell spanning its rows (#640) |
| 2026-10-11 | Word XML files (#548): Word XML Document (Flat OPC `.xml`) opens and saves through the DOCX reader and writer; Word 2003 XML opens (styles, lists, tables, sections, notes, comments, pictures, hyperlinks) |
| 2026-10-14 | Open DrawingML freeform paths draw `a:tailEnd` at the final path segment |
| 2026-10-12 | File › Print selects a system printer, copies, duplex and grayscale, submits PDF jobs through craft-print on a worker and shows spooler errors; browser printing and PDF export remain available |
| 2026-10-11 | Font fallback indexes which characters each installed font maps, reading each font file once (#605): text in a script no installed font covers no longer re-reads every system font per distinct character (200 such characters: 13 s → 0.2 s) |
| 2026-10-13 | Markdown import of text full of backtick runs that never close is linear (#1336): every code span and link bracket over them searched the rest of the text for a closing run (2 MB in brackets: 17.8 s → 0.06 s) |
| 2026-10-13 | Markdown import keeps every link in a paragraph (#1452): a `[` nothing closes, bracketed text that is not a link and each link used up the bracket scans' look-ahead budget as if they had read the rest of the paragraph, so later links were plain text; a scan now costs what it reads, and brackets it has passed are looked up |
| 2026-10-11 | Find and Replace special characters (`^p`, `^t`, `^#`, `^&` …) and Use wildcards (ranges, counts, word anchors, groups with `\1` in Replace); `^p` finds and splits paragraphs; Special ▾ menu and search options in the dialog (#543) |
| 2026-10-11 | Typing parity (#546): `+---+` Enter makes a table, `> ` `-> ` `=> ` arrow bullets, smileys inside words (`ok:)`), Undo right after a border line brings the characters back |
| 2026-10-11 | Windows: DirectX 12 shaders compile with FXC, built into Windows, so the app no longer quits at launch with "Parent device is lost" when no matching `dxcompiler.dll` + `dxil.dll` pair is around (#603) |
| 2026-10-11 | Insert › SmartArt (#497): list, process, cycle, hierarchy, pyramid, radial, matrix and Venn graphics laid out by our own code, Text Pane, SmartArt Design tab (Add Shape, Promote, Demote, layouts, colours, Reset); saved as DOCX diagram parts with the drawing. Catalog 410/441 (93.0%) |
| 2026-10-11 | Outline view (#493): Outlining tab, outline symbols and indents, collapse/expand, Show Level, first line only, plain text; catalog 428/450 → 441/463 (95.2%) |
| 2026-10-11 | Vertical sections (#491): Layout › Text Direction turns the section (or the cell or text box the caret is in); lines run top to bottom, right to left, ideographs upright; `w:textDirection` in `w:sectPr` read and written |
| 2026-10-11 | File › Options in Word's panes (#485): General, Display, Proofing, Save, Language, Accessibility, Advanced, Agents; initials, individual formatting marks, spelling ignore options, custom dictionary, overtype, typing replaces selection, measurement units, recent-documents count, print options; `file.options` reads and sets them headlessly |
| 2026-10-14 | Word XML Document (Flat OPC) reading is bounded everywhere in the file (#1353): nesting and elements are capped as the XML tree reader caps a part's, every child of the package and every part's data count against the part count and size limits (a repeated part name too), and data is checked before it's copied; base64 written as CDATA or with character references reads as the same bytes, and an entity in it is an error instead of bytes gone missing |
| 2026-10-11 | Right-click Wrap Text submenu on pictures, shapes and text boxes: wrapping styles, Move with Text / Fix Position on Page (#513) |
| 2026-10-14 | Text boxes in tables nested deeper than 16 survive edits (#1339): the paragraph walk behind text box pruning, copy and paste, reading order, drawing ids and note conversion goes through tables however deep they nest instead of stopping 16 tables down, so an unrelated edit no longer drops the text of a box shown only from deeper, and a copied deep table takes its boxes along |
| 2026-10-11 | Caret animation: keys and typing glide the caret to its new place in 100 ms along the curve measured from Word for Windows, and the blink fades; clicks and scripted moves don't glide. File › Options › Advanced › Animation turns it off, kept between runs; `ui.caretAnimation` for agents |
| 2026-10-11 | Paragraph dialog options (#512): mirror indents (`w:mirrorIndents`, laid out on even pages), same-style spacing, suppress line numbers, don't hyphenate |
| 2026-10-11 | Screen readers can read the document canvas (#488): paragraphs, headings, lists, tables, links, pictures and the caret exposed through AccessKit |
| 2026-10-11 | Add fonts without rebuilding (#582): Home › Font › Add Font… (end of the font list), dropping a TTF/OTF/TTC/WOFF/WOFF2 file, or `font.add`/`font.list`/`font.remove` for agents. Files are checked (50 MB cap, every face must parse and map characters), never replace a bundled or loaded face, and are kept between runs (desktop: `fonts` in the settings folder; web: IndexedDB). Linking a font by URL is still to come |
| 2026-10-11 | Print Markup (#589): File › Print/Export checkbox, `file.printMarkup`, `markup` on save/export commands and `wordcraft-cli convert --markup` print comments (with replies) in balloons beside each page, scaled to the paper like Word's "Document showing markup"; HTML, Markdown and text copies end with the comments |
| 2026-10-11 | Custom colours (#606): every colour drop-down gains Recent Colors and More Colors…, which opens a Colors dialog with a Standard honeycomb and a Custom hue/saturation field, lightness slider, RGB/HSL numbers and a hex box |
| 2026-10-11 | Complex scripts (#604): Myanmar, Khmer and Indic text shaped one script at a time in OpenType fallback fonts (regular weight, not AAT-only fonts), the caret and line breaks never split a syllable, Myanmar wraps between syllables, Backspace removes one sign at a time |
| 2026-10-11 | Any text file opens (#569): `.m3u`/`.m3u8` playlists and other unknown extensions open as plain text with encoding detection (UTF-8, BOMs, UTF-16, Windows-1252), misnamed Word/RTF files by their signature; binary files give a clear error |
| 2026-10-11 | Select by dragging whole units (#549): after a double-click the drag grows by words, after a triple-click by paragraphs, after Ctrl/⌘+click by sentences, as in Word |
| 2026-10-12 | Selection bar (#856): in the left margin beside body text a click selects the line, a double-click the paragraph, a triple-click or Ctrl/⌘+click the document; dragging there selects whole lines and Shift+click extends by lines; the pointer turns to an arrow. Right margin for right-to-left paragraphs not yet |
| 2026-10-11 | Compare / Combine dialog with comparison settings, Restrict Editing pane (exceptions, SHA-512 password, locked styles; DOCX round trip), AutoCorrect Options dialog saved per user (#412) |
| 2026-10-11 | DOCX diagrams (#611): lines, connectors and boxes styled by the theme now show (they were invisible); callouts and flowchart boxes drawn as the nearest shape; VML lines, rectangles and ovals |
| 2026-10-11 | WordArt (#416): text fill/outline/shadow/glow/reflection, Transform warps, our own styles; straight/elbow/curved connectors glued to shapes that re-route; DOCX round trip |
| 2026-10-11 | Content controls (`w:sdt`) kept, shown and edited; Developer tab, Design Mode, Properties dialog (#381) |
| 2026-10-11 | Insert › Chart (#409): column, bar, line, area, pie, doughnut and scatter charts with Chart Design (type, Edit Data, title, legend, data labels), saved as real chart parts; catalog 408/439 (92.9%) |
| 2026-10-11 | Line Numbers, Hyphenation (zone, CAPS, consecutive limit, Manual), Language, Envelopes and Labels dialogs (#407); hyphenation options kept in DOCX; 30 modal dialogs |
| 2026-10-11 | Accessibility (#386): Alt Text pane for every object and table, Mark as decorative, alt text round-trips in DOCX, deeper checker with fix actions |
| 2026-10-11 | References dialogs (#402): Caption (labels, chapter numbering), Index and Mark Entry, Custom Table of Contents, Source Manager and Create/Edit Source, with TOC/INDEX/XE/SEQ switches honoured on update |
| 2026-10-11 | Font dialog: separate Asian text font and Latin text font (#505); `format.font {"eastAsia"}`; East Asian fonts found by the font scan (OS/2 code pages) |
| 2026-10-11 | Document grid (#391): Page Setup › Document Grid sets lines per page and characters per line (`w:docGrid` round trip); lines snap to the line pitch, East Asian characters to the character pitch; View › Gridlines shows it |
| 2026-10-11 | Move tracking (#400): `w:moveFrom`/`w:moveTo` and their named ranges kept in DOCX, a tracked cut and paste of a paragraph or sentence records a move, accept/reject at either end resolves both, green double strikethrough/underline and "Moved from/to" balloons |
| 2026-10-11 | Footnote and Endnote dialog from the References › Footnotes launcher: options per section or document (below text, end of section, number format, start at, restart each section or page), custom marks, convert footnotes ↔ endnotes; kept in DOCX (#385) |
| 2026-10-11 | Shape Format › Text: text direction, Align Text and linked text boxes (Create Link / Break Link), read and written in DOCX (#369) |
| 2026-10-11 | Quick Access Toolbar KeyTips in the customized button order (Alt/F10 then a number), disabled-command checks and a saved File › Options switch (`ui.keytips`), #620 |
| 2026-10-11 | Mail-merge recipients from Excel (.xlsx/.xlsm) and OpenDocument (.ods) spreadsheets, with Select Table to pick the sheet (#334) |
| 2026-10-11 | Insert › Icons (#309): a library of 61 original icons drawn in code, a searchable picker and Icon Color |
| 2026-10-11 | Home › Font › Enclose Characters (#297): circle, square, triangle or diamond around a character, saved as Word's `EQ \o\ac` field |
| 2026-10-11 | Phonetic Guide (#288): ruby text over base text, `w:ruby` round trip |
| 2026-10-12 | Custom XML backing stores preserved as opaque data and related properties through edited DOCX saves; content-control markup is handled by the existing reader/writer and #532 bibliography I/O remains separate; binding evaluation and readiness unchanged |
| 2026-10-12 | Imported TOCs keep actual result boundaries, nested page fields, formatting and locks through DOCX saves without relying on TOC styles; existing engine-managed styled TOCs remain refreshable |
| 2026-10-11 | Customize Ribbon and the Quick Access Toolbar: custom tabs and groups, hide and reorder tabs, add or remove toolbar commands (right-click any button, or the toolbar's … menu); kept between runs (#379, #376) |
| 2026-10-11 | Customize Keyboard: assign, remove and reset keyboard shortcuts; custom keys override built-ins and are kept between runs (#368) |
| 2026-10-12 | Native saves use exclusively created sibling temporary files, synchronize before replacement and clean up on failure; existing temporary-name files/links and original files on failed saves are protected. Synthetic CLI and focused tests on macOS; no new Word parity or readiness claim. |
| 2026-10-11 | Field codes like Word's field keys (#339): show codes (Alt+F9, Shift+F9), edit, unlink, lock and step through fields |
| 2026-10-11 | Word's common keyboard shortcuts (#330): 101 → 125 of ~250 |
| 2026-10-13 | Swedish (`sv`) interface: 2,504 catalog entries (every key of the reference catalog, the equation gallery, everything the interface audit finds, and 44 newer strings no catalog had yet, among them the native print settings), `sv-SE`/`sv-FI` system locales and a saved language choice; locale, catalog and bundled-font tests; AI-assisted, terminology and the running app reviewed by a native speaker; no Swedish proofing resources |
| 2026-10-12 | Bulgarian (`bg`) interface: 2,419 catalog entries (every key of the reference catalog, the equation gallery and everything the interface audit finds), `bg-BG` system locales and a saved language choice; locale, catalog and bundled-font tests; no Bulgarian proofing resources |
| 2026-10-13 | Croatian (`hr`) interface: 2,507 catalog entries (every key of the reference catalog, built-in style names and the equation gallery), `hr-*` system locales and a saved language choice; locale, catalog and bundled-font tests; no Croatian proofing yet |
| 2026-10-11 | Hungarian (`hu`) interface: all 2,419 catalog entries, built-in style names translated, `hu-HU` system locales and a saved language choice; locale, catalog and bundled-font tests; no Hungarian proofing resources |
| 2026-10-11 | Turkish (`tr`) interface: all 1,307 catalog entries, `tr-TR`/`tr-CY` system locales and a saved language choice; command search keeps Turkish dotted and dotless I apart (`italik` finds İtalik); locale, catalog, search and bundled-font tests; no Turkish proofing resources |
| 2026-10-11 | Italian (`it`) interface (#594): 2,399 entries, feature names as Italian Word users know them, locale and saved-preference tests, bundled-font checks; no Italian proofing resources |
| 2026-10-12 | Native drawing-level Word ink preserved through DOCX edits and repeated saves, including its InkML part and optional VML picture fallback; native ink rendering and editing remain missing |
| 2026-10-12 | List-label line height: automatic spacing scales body text before adding label ascent overflow; AtLeast spacing keeps text descent. Original Word for Mac probes and deterministic layout tests cover symbol/decimal labels, wrapped lines, fixed/minimum spacing and inline pictures. Overall readiness unchanged; [evidence](docs/layout-parity.md#list-label-line-spacing) |
| 2026-10-12 | Local desktop recovery: unsaved drafts and edits are snapshotted in the profile; File › Recover restores an unsaved copy or confirms discard. Full model storage includes media and opaque parts. Password sessions skip plaintext snapshots. See [recovery.md](docs/recovery.md); stability estimates remain unchanged. |
| 2026-10-12 | Legacy table paragraph spacing: `allowSpaceOfSameStyleInTable` survives DOCX save; enabled modes 11/12/14 keep contextual same-style spacing in cells. Mode 15 and non-cell controls unchanged; measured against Word for Mac 16.113.3 |
| 2026-10-11 | Contextual paragraph spacing: same-style first-only 12/8 pt and 10/12 pt gaps corrected to 0 pt and 2 pt in body and header layout, with per-side controls; measured against Word 16.113.3. Ordinary overlap remains #108 |
| 2026-10-11 | Citations and bibliography in all 12 of Word's styles: Harvard, ISO 690 (author-date and numerical), Turabian, GB/T 7714, GOST (name and title sort) and SIST02 join APA, MLA, Chicago and IEEE; numbered styles count in order of first citation (#383) |
| 2026-10-11 | Interface size setting (#475): File › Options › General scales the interface 80–200 % on top of the system display scale, kept between runs; `ui.interfaceSize` for agents |
| 2026-10-11 | DOCX paragraph defaults: preserve omission separately from explicit empty defaults, with repeated-save and default-edit regressions |
| 2026-10-11 | Formatting revisions (#41): `w:rPrChange`/`w:pPrChange`, table/row/cell, section and numbering changes kept in DOCX; formatting recorded as revisions while tracking; accept/reject, Reviewing Pane descriptions, "Formatted: …" balloons and change bars |
| 2026-10-11 | Draw: Lasso Select (move, recolour, delete the strokes inside a loop), Add Pen gallery kept between runs, Ink to Shape (own recogniser: line, rectangle/square, ellipse/circle, triangle, polygon), Ink Replay in drawing order with play/pause/rewind/forward; drawing order kept in DOCX (#395); catalog 406/434 (93.5%) |
| 2026-10-10 | Estonian (`et`) interface (#294): 1,161 translated entries, locale and saved-preference tests, bundled-font checks; no Estonian proofing resources |
| 2026-10-10 | Draw tab: pen, pencil and highlighter ink, stroke eraser, Select, Review › Hide Ink; ink saved to DOCX as DrawingML freeforms (#307); catalog 399/431 (92.6%) |
| 2026-10-10 | Draw Table and Eraser (#303): a pen draws one-cell tables and splits cells along drawn lines; the eraser merges the cells beside a border. Catalog 393/431 (91.2%) |
| 2026-10-10 | DOCX charts, SmartArt and OLE objects survive open and save with their parts (#319) |
| 2026-10-10 | Full re-measure against Word for Mac 16.113.4 and progress docs to the craftrules standard (this page, `docs/target-app-parity.md`, `gaps.md`, `roadmap.md`, `architecture.md`, format/layout/UI/hardware/localization parity; `docs/parity.md` → `docs/parity-checklist.md`). Landed the same day (~80 PRs): equations with OMML and an Equation tab (#191); Word 97-2003 `.doc` import (#145); right-to-left and Persian text (#207); Zotero integration (#189); Read Aloud player (#190); text boxes and floating objects editable (#46) and placed like Word (#136); floating tables (#137); track-changes fixes (#125, #244); typing parity with Word (#204); hidden text (#173); save prompts and AutoSave rules (#151); LaTeX import/export (#11); keytips and mini-toolbar (#43); Spanish, Ukrainian and Brazilian Portuguese interfaces; system theme (#249); Linux file dialogs no longer freeze (#246); Chinese UI font on Windows (#248); every installed font weight (#239); cell text direction (#245); Paste Special (#235), Advanced Find (#234), Column Selection (#237), Style Inspector (#236), custom table styles (#256), View gridlines (#243), zoom buttons (#232), inertial touchpad scrolling (#252), File Info fields kept (#268), ¶ in mixed-direction paragraphs (#278), Serbian interface (#250), table Height/Width boxes, AutoFit Contents and Table Properties (#44), AutoSave switch greyed out with the reason where it can't save (.md/.txt/.html, unsaved, browser) (#196, #176), OpenGL fallback on Windows PCs without a DirectX 12 driver (#316), Columns, Symbol and Field dialogs (#321), Tabs and Borders and Shading dialogs (#320); free rotation and flips for pictures, shapes and text boxes with a rotation handle (#332); v0.4.0 released; file associations on Windows and Linux with a Make-default button in Options (#295) |
| 2026-10-09 | Interface languages follow the system or Options (#12; zh-hans, zh-hant, ja); TOC page numbers as an updatable field (#52); rotating log file (#17); window geometry remembered (#38) |
| 2026-10-08 | v0.2.0 and v0.3.0 released; Flatpak and AppImage auto-update (#22) |
| 2026-10-07 | v0.1.0 released; contributor credits in About (#6) |
| 2026-10-06 | Layout arc: wrap around floats, text boxes, page borders, line numbers, row splitting, drop caps, hyphenation, comment balloons; release pipeline for every platform; first estimate (~62% real feature parity, 120–150 h to 100%) |
| 2026-10-05 | M0–M12 first versions: model, layout, render, engine with 200+ commands, Word-style UI, DOCX I/O, PDF and other formats, references, review, mailings, CLI, MCP, web |
| 2026-10-12 | Mail-merge recipients (#778): legacy Excel .xls workbooks; CSV/TXT lists in GBK, Big5, Shift_JIS, UTF-16 and other encodings read correctly (plain-text import too); Edit Recipient List filters (equal, not equal, less, greater, blank, contains… joined by And/Or) and sorts, and preview, Finish & Merge and labels use only the matching records in that order (`mailings.filterRecipients`, `mailings.sortRecipients`) |
| 2026-10-13 | The interface-translation audit runs in CI again (#1128): the size labels of Envelopes, Labels and Layout › Size are translated (`4,13 × 9,5 дюйма`) and use the region's decimal separator, and status-bar messages (table split and merge, ink replay, citations, SmartArt, Researcher, Design defaults), the Reading Mode toolbar, Go To and Search's Suggested heading are translated in every interface language, and the Read Aloud player no longer shows a speech error a frame before the status bar does, the race behind the audit's rare failure under load |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | trivial | Progress log: the Styles pane docks left or right, or floats (#1756) |
| 2026-10-14 | trivial | Progress log: row table border exceptions |
| 2026-10-12 | minor | Czech proofing review: short-word splits rank after deletion, NOSUGGEST excludes suggestions while accepting typed forms, affix comments apply only to headers, ratios keep spaces and repeated-word messages use Czech quotes; synthetic engine end-to-end and latency checks |
| 2026-10-12 | minor | Czech spelling from the installed Hunspell dictionary, runtime-table suggestions and generic checks; Czech interface writes cs-CZ documents; no Czech hyphenation patterns bundled; estimates unchanged |
| 2026-10-12 | trivial | Progress log: one selected cell's border drags in its row only (#962) |
| 2026-10-12 | minor | Progress log: Word default shortcuts expanded to 140 commands on macOS (169 combinations) and 158 on Linux/Windows (188 combinations), addressing gap #24 |
| 2026-10-13 | trivial | Web/Draft long-page blur fixed with visible full-resolution raster tiles |
| 2026-10-14 | trivial | Progress log: VML freeforms read and kept on save |
| 2026-10-14 | trivial | Progress log: consistent preferred cell widths |
| 2026-10-14 | trivial | Field instructions spanning paragraphs no longer add blank lines during pagination; empty result paragraphs still take a line. |
| 2026-10-14 | trivial | A list paragraph containing only a page break has no visible label; it still advances the list counter. Ordinary empty list items keep their label. |
| 2026-10-14 | trivial | Floating-table overlap checks use visible line content; empty line-break lines no longer move a table |
| 2026-10-14 | trivial | Progress log: deleting a comment leaves no dangling anchors or replies (#1345) |
| 2026-10-14 | minor | Legacy DOC DIB conversion byte limits only; related #1334/#1554, stage and readiness estimates unchanged |
| 2026-10-13 | trivial | Progress log: justified vertical page alignment (#1320) |
| 2026-10-14 | trivial | Progress log: file names that aren't valid Unicode (#1440) |
| 2026-10-13 | trivial | Behind Text pictures and shapes drag over overlapping text in the body and headers, selected or not, while ordinary clicks keep editing text; In Front of Text drag covered too |
| 2026-10-13 | trivial | Progress log: the watermark written to DOCX (#1290) |
| 2026-10-13 | trivial | Progress log: Word's text watermark read from DOCX (#1290) |
| 2026-10-13 | trivial | Progress log: Insert Hyperlink › Place in This Document (#1234) |
| 2026-10-13 | trivial | Progress log: Insert › Bookmark deletes and goes to bookmarks (#1236) |
| 2026-10-12 | trivial | Progress log: an option to show only the selected text's comment (#1055) |
| 2026-10-12 | trivial | Progress log: clicking in a comment's text moves its caret (#1013) |
| 2026-10-12 | trivial | Progress log: a press on the ribbon no longer scrolls the document (#1019) |
| 2026-10-12 | trivial | Progress log: Ctrl+drag copies an object (#1014) |
| 2026-10-12 | trivial | Document title opens Rename Document; the account badge opens Options › General for user name and initials. |
| 2026-10-13 | trivial | Progress log: rounded rectangle corners (#1476) |
| 2026-10-13 | trivial | Progress log: PAGEREF in headers and footers (#1477) |
| 2026-10-13 | trivial | Replace All checks content-control locks before editing; a refused batch leaves the document and undo history unchanged; existing deletion-lock rules reused, with tracked/untracked regression coverage |
| 2026-10-14 | trivial | Progress log: print-area strings translated in every catalog |
| 2026-10-14 | trivial | Progress log: All Caps and Small Caps draw ß as SS (#1581) |
| 2026-10-13 | trivial | Progress log: Windows Magnifier follows the caret onto a new line (#1119) |
| 2026-10-13 | trivial | Progress log: a window too big for its screen shrinks to fit (#1215) |
| 2026-10-14 | trivial | Progress log: Page Setup sections are validated (#1582) |
| 2026-10-13 | trivial | Progress log: `--control` checks its port (#1426) |
| 2026-10-14 | trivial | Progress log: words beside a floating picture are no longer split (#1376) |
| 2026-10-14 | trivial | Progress log: VML preset shapes (#1377) |
| 2026-10-14 | trivial | Progress log: Russian letter list labels |
| 2026-10-14 | trivial | Progress log: the save prompt takes the keyboard from the page (#1597) |
| 2026-10-12 | trivial | Progress log: VML text box margins, fit and alignment; hidden drawings |
| 2026-10-13 | trivial | Progress log: comment balloon buttons work, empty comments can be deleted (#1085) |
| 2026-10-12 | trivial | Progress log: sample-text formulas on Enter (#1067) |
| 2026-10-12 | trivial | Progress log: direct run-level native InkML content parts survive DOCX edits and repeated saves |
| 2026-10-13 | trivial | Progress log: the caret contrasts with its background (#299) |
| 2026-10-13 | trivial | Progress log: clicks on ribbon chrome keep typing in the document (#1093) |
| 2026-10-13 | trivial | Progress log: dragged shapes wrap in their new column (#884) |
| 2026-10-14 | trivial | Progress log: All Caps and Small Caps text extracts from PDFs as shown (#1473) |
| 2026-10-14 | trivial | Progress log: the web's Save before New/Open waits for the save picker (#1315) |
| 2026-10-13 | trivial | Progress log: a failed save from the close prompt says so (#1445) |
| 2026-10-14 | trivial | Progress log: Inspect Document removes hidden text in every story (#1344) |
| 2026-10-14 | trivial | Progress log: ODT tracked changes (gap 19) |
| 2026-10-14 | trivial | Progress log: RTF tracked insertions and deletions (gap 19) |
| 2026-10-14 | trivial | Progress log: alignment guides while dragging pictures and shapes |
| 2026-10-14 | trivial | Progress log: ODT sections on import (#1379) |
| 2026-10-14 | trivial | Progress log: right-to-left text in exported PDFs extracts as whole words (#1383) |
| 2026-10-14 | trivial | Progress log: HTML header cells, cell alignment and region paper for opened HTML/Markdown/text (#1381) |
| 2026-10-14 | trivial | Progress log: custom page size from More Paper Sizes… and Page Setup (#1433) |
| 2026-10-13 | trivial | Progress log: highlighted text on a dark page (#1252) |
| 2026-10-13 | trivial | Progress log: full screen on F11 (#980) |
| 2026-10-13 | trivial | Progress log: interface audit back in CI (#1128) |
| 2026-10-13 | trivial | Progress log: double-click the margin to create and edit a header or footer (#960) |
| 2026-10-13 | trivial | Progress log: Set Proofing Language… in the Language menu (#860) |
| 2026-10-12 | trivial | Progress log: list label follows paragraph alignment (#1159) |
| 2026-10-13 | trivial | Progress log: space after a page break |
| 2026-10-13 | trivial | Progress log: table row margins and splitting |
| 2026-10-13 | trivial | Progress log: lines inside groups in PDF export (#1474) |
| 2026-10-14 | trivial | Progress log: continued paragraphs wrap around the next column's shapes (#1374) |
| 2026-10-13 | trivial | Progress log: PAGE fields on the pages a paragraph runs onto (#1352) |
| 2026-10-14 | trivial | Progress log: RTF import keeps sections (#1378) |
| 2026-10-14 | trivial | Progress log: colour emoji drawn on screen and in page renders (#1269) |
| 2026-10-14 | trivial | Progress log: `.doc` table shading and the extra column (#1380) |
| 2026-10-12 | trivial | Progress log: web picture paste respects the edit target (#1349) |
| 2026-10-14 | trivial | Progress log: table deletes, Change Case and Unlink Fields respect Track Changes and locked content controls (#1329) |
| 2026-10-14 | trivial | Progress log: Ink to Shape recognises single-stroke arrows |
| 2026-10-13 | trivial | Progress log: Styles gallery fallback for DOCX files without qFormat (#1238) |
| 2026-10-13 | trivial | Progress log: crop handles on the canvas |
| 2026-10-13 | trivial | Progress log: cell size boxes act on the whole selection (#1242) |
| 2026-10-12 | trivial | Progress log: Accept All and Reject All keep the right paragraph marks (#864) |
| 2026-10-12 | trivial | Progress log: `--help` for the app and every CLI subcommand (#968) |
| 2026-10-12 | trivial | Progress log: Down and Up no longer skip a table (#1007) |
| 2026-10-13 | trivial | Progress log: equation pictures keep the theme colours on Windows (#1002) |
| 2026-10-12 | trivial | Progress log: table style editor (#714) |
| 2026-10-11 | trivial | Progress log: Ctrl, Alt, left-edge and selected-cells table border drags (#711) |
| 2026-10-11 | trivial | Progress log: table markers on the rulers (#700) |
| 2026-10-11 | trivial | Progress log: the table context menu |
| 2026-10-12 | trivial | Progress log: AutoFit in a nested table, whole cells shaded for a selection through a table, Shift+Right at a cell's end (#971) |
| 2026-10-11 | trivial | Progress log: drags and Shift+arrows over table cells select whole cells |
| 2026-10-11 | trivial | Progress log: table selection areas, insert buttons, move handle and double-click AutoFit; catalog 484/499 |
| 2026-10-12 | trivial | Progress log: Undo history list (#701) |
| 2026-10-14 | trivial | Progress log: Save As type list (#1399) |
| 2026-10-12 | trivial | Progress log: Start screen at launch (#766) |
| 2026-10-13 | trivial | Progress log: no document behind the Start screen (#1129) |
| 2026-10-12 | trivial | Progress log: Alt+Q command search (#819); log table header fixed |
| 2026-10-12 | trivial | Progress log: ribbon groups fit above their labels |
| 2026-10-12 | trivial | Font list previews no longer grow memory (#848) |
| 2026-10-12 | trivial | Progress log: section break keeps the selected text (#852) |
| 2026-10-12 | trivial | Progress log: save prompt from the keyboard (#861) |
| 2026-10-13 | trivial | Progress log: an empty .docx opens and saves back to itself (#1384) |
| 2026-10-12 | trivial | Progress log: craft-fonts Simplified Chinese in documents |
| 2026-10-12 | trivial | Progress log: symbol-font text drawn as Unicode symbols (#776) |
| 2026-10-13 | trivial | Progress log: the Paragraph dialog leaves mixed formatting alone (#1346) |
| 2026-10-12 | trivial | Progress log: WMF and EMF pictures can be inserted (#802) |
| 2026-10-14 | trivial | Progress log: a picture inserted by a command survives the next typing (#965) |
| 2026-10-11 | trivial | Progress log and shipped languages: Turkish interface completed, 2,325 entries (#774) |
| 2026-10-12 | trivial | Progress log: Navigation heading truncation (#759) |
| 2026-10-11 | trivial | Progress log: Reading Mode implementation |
| 2026-10-12 | trivial | Progress log: new documents follow the system region (#838) |
| 2026-10-11 | trivial | DOCX complex-script formatting inheritance fix and round-trip coverage |
| 2026-10-11 | minor | Czech (`cs`) interface catalog added |
| 2026-10-12 | trivial | Progress log: AutoCorrect replacement list follows the text's language (#835) |
| 2026-10-11 | trivial | Progress log: German interface (#574) |
| 2026-10-12 | trivial | Progress log: language-aware sorting and table numbers (#836, #837) |
| 2026-10-13 | trivial | Progress log: Word 365 cover pages, positions and closed freeforms (#743) |
| 2026-10-12 | trivial | Progress log: Word 365 cover pages (#743) |
| 2026-10-11 | trivial | Progress log: tables resize with the mouse (#217) |
| 2026-10-11 | trivial | Progress log: Animation effects setting |
| 2026-10-11 | trivial | Progress log: built-in file browser when the system file dialog is unavailable (#576) |
| 2026-10-12 | trivial | Progress log: large-table scrolling (#811) |
| 2026-10-11 | trivial | Progress log: security review fixes (temp files, exported link schemes) |
| 2026-10-13 | trivial | Progress log: every distinct format in a paragraph keeps its own layout style (#1340) |
| 2026-10-11 | trivial | Progress log: Find and Replace special characters and wildcards (#543) |
| 2026-10-12 | minor | Desktop File > Print now discovers printers and submits PDF jobs through pinned craft-print with printer choice, copies, collation, duplex and grayscale; the upstream preview, ranges, parity and Print Markup remain available, along with browser printing and PDF export. Remaining print parity includes driver-specific tray/options and pages-per-sheet UI; physical printer fidelity needs hardware validation. |
| 2026-10-11 | trivial | Progress log: Windows DX12 shaders compiled with FXC (#603) |
| 2026-10-11 | trivial | Progress log: Insert › SmartArt (#497); catalog 429/451 → 437/458 (95.4%) |
| 2026-10-11 | trivial | Progress log: Outline view (#493) |
| 2026-10-11 | trivial | Progress log: vertical sections (#491) |
| 2026-10-11 | trivial | Progress log: Paragraph dialog options, mirror indents (#512) |
| 2026-10-11 | trivial | Progress log: Print Markup (#589) |
| 2026-10-11 | trivial | Progress log: custom colours via More Colors… (#606) |
| 2026-10-11 | trivial | Progress log: complex-script shaping (#604) |
| 2026-10-11 | trivial | Progress log: unknown text extensions open as text (#569) |
| 2026-10-11 | trivial | Progress log: drag-select by words/paragraphs/sentences (#549) |
| 2026-10-12 | trivial | Progress log: selection bar in the left margin (#856) |
| 2026-10-11 | trivial | Progress log: content controls (#381) |
| 2026-10-11 | trivial | Progress log: Insert › Chart (#409); catalog 408/439 (92.9%) |
| 2026-10-13 | trivial | Progress log: free part, list and comment ids after a file's largest ones (#1338) |
| 2026-10-11 | trivial | Progress log: Line Numbers, Hyphenation, Language, Envelopes and Labels dialogs (#407) |
| 2026-10-11 | trivial | Progress log: Alt Text pane, decorative objects, deeper accessibility checker (#386) |
| 2026-10-11 | trivial | Progress log: Draw lasso, Add Pen, Ink to Shape, Ink Replay (#395); catalog 406/434 |
| 2026-10-11 | trivial | Progress log: document grid (#391) |
| 2026-10-11 | trivial | Progress log: text direction, Align Text and linked text boxes (#369) |
| 2026-10-10 | trivial | Progress: Drawing Canvas (#344): insert, shapes/pictures/text boxes inside, Fit/Expand/Scale, DOCX `wpc:wpc`, turns and flips as a whole; catalog 404/434 |
| 2026-10-11 | trivial | Quick Access Toolbar KeyTips and persisted enable/disable setting (#620) |
| 2026-10-11 | trivial | Progress log: mail-merge recipients from .xlsx/.xlsm/.ods (#334) |
| 2026-10-11 | trivial | Insert › Icons landed (#309): catalog 402/434 → 403/434 (92.9%) |
| 2026-10-13 | trivial | Progress log: Markdown backtick runs read in linear time (#1336) |
| 2026-10-13 | trivial | Progress log: Markdown brackets no longer turn later links into text (#1452) |
| 2026-10-11 | trivial | Enclose Characters (#297): catalog 403/434 (92.9%) |
| 2026-10-12 | minor | Progress log: opaque custom XML store preservation, with synthetic package regressions and explicit limits |
| 2026-10-12 | minor | Preserve imported TOC ranges and retain #538 refresh/undo/save coverage on current main; readiness unchanged |
| 2026-10-11 | trivial | Progress log: Customize Ribbon and the Quick Access Toolbar (#379) |
| 2026-10-11 | trivial | Omitted `w:pPrDefault` stays omitted across saves; explicit empty/default properties and later spacing edits remain writable. Synthetic preservation checks only; readiness estimates unchanged |
| 2026-10-11 | trivial | Progress log: Customize Keyboard (#368) |
| 2026-10-12 | trivial | Native saves protect existing temporary-name entries and originals on pre-replacement failure; synthetic macOS evidence, explicit durability/platform limits; readiness unchanged |
| 2026-10-11 | trivial | Word's common keyboard shortcuts (#330): 125 of ~250 |
| 2026-10-11 | trivial | Collapsed duplicate rows left by parallel merges |
| 2026-10-13 | trivial | Swedish interface (`sv`): progress log and shipped languages |
| 2026-10-12 | trivial | Bulgarian interface (`bg`): progress log and shipped languages |
| 2026-10-13 | trivial | Croatian interface (`hr`): progress log and shipped languages |
| 2026-10-11 | trivial | Hungarian interface (`hu`): progress log and shipped languages |
| 2026-10-11 | trivial | Turkish interface (`tr`): progress log and shipped languages |
| 2026-10-14 | trivial | Progress log: `.doc` notes, headers and text boxes read their text once (#1319) |
| 2026-10-12 | minor | Native drawing-level ink and its picture fallback survive DOCX edits and repeated saves; adapted to the current graphic writer; rendering and editing InkML remain missing |
| 2026-10-12 | minor | List-label automatic/AtLeast line-height inflation corrected; gap #4 remains open and readiness unchanged |
| 2026-10-12 | minor | Local desktop recovery: unsaved drafts and edits are snapshotted in the profile; File › Recover restores an unsaved copy or confirms discard. Full model storage includes media and opaque parts. Password sessions skip plaintext snapshots. See [recovery.md](docs/recovery.md); stability estimates remain unchanged. |
| 2026-10-12 | trivial | Legacy table paragraph spacing preserved; modes 11/12/14 versus 15 checked with original synthetic documents |
| 2026-10-11 | minor | Progress log: contextual paragraph spacing in body paragraphs and headers |
| 2026-10-11 | trivial | Progress log: eight more bibliography styles (#383) |
| 2026-10-14 | trivial | Progress log: ODT and `.doc` pictures read once, within their byte budget (#1334) |
| 2026-10-10 | trivial | Progress log: Draw tab ink (#307) |
| 2026-10-10 | trivial | Draw Table and Eraser (#303): catalog 393/431 (91.2%) |
| 2026-10-10 | trivial | Progress log: charts, SmartArt and OLE objects kept on DOCX save (#319) |
| 2026-10-10 | trivial | Progress log: contour wrap (#354) |
| 2026-10-14 | trivial | Progress log: text boxes in deeply nested tables kept through edits (#1339) |
| 2026-10-10 | trivial | Progress log: free rotation and flips (#332) |
| 2026-10-10 | trivial | Progress log: Windows OpenGL fallback without a DirectX 12 driver (#316) |
| 2026-10-14 | trivial | Progress log: Flat OPC reading bounded everywhere in the file, CDATA base64 read (#1353) |
| 2026-10-10 | minor | Password-protected .docx (#281): files encrypted with a password (agile encryption) open and save; File › Info › Encrypt with Password |
| 2026-10-10 | minor | Relanded clipboard pane, table styles editor, Manage Styles, Asian typography (kinsoku), Group/Ungroup and Shape Effects (#264 #263 #270 #271 #267 #280): catalog 390/431 (90.5%) |
| 2026-10-10 | minor | Polish interface and proofing landed: Localization 25% → 27%, Proofing 35% → 40% |
| 2026-10-10 | minor | Merged main: catalog 384/430 (89.3%), 431 commands; Paste Special, Advanced Find, Column Selection, Style Inspector, custom table styles, gridlines, zoom buttons, touchpad scrolling, File Info, Serbian landed; mainstream 54→55% |
| 2026-10-10 | minor | Readiness table with hours per audience; full number restored to the additive weighted sum (~47% → ~60%): method aligned with the standard, no new evidence; beta distance back to ~15 points and ~150–230 h |
| 2026-10-10 | minor | Mainstream practitioner (~54%) and essentials user (~63%) numbers added; full number recomputed 60% → ~47% with the standard's discounts; beta distance now ~28 points and ~450–700 h; stage stays alpha |
| 2026-10-10 | minor | Alpha gate checked (six core workflows pass); stage stays alpha |
| 2026-10-10 | major | Full re-measure; restructured to the progress-docs standard (stage banner, two numbers, dimensions, languages, upcoming, progress log); parity tables moved to `docs/target-app-parity.md`, milestones to `docs/roadmap.md` |
| 2026-10-09 | minor | Recently landed: RTL, Ukrainian and Brazilian Portuguese |
| 2026-10-06 | major | First honest estimate: 88% catalog, ~62% real parity, alpha ~85% of the way |
| 2026-10-12 | trivial | Progress log: recipient filter and sort, .xls and legacy-encoding recipient lists (#778) |
