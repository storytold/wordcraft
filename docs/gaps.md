> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-01 · **Change:** trivial (DOCX table rows and cells inside selected alternate-content branches survive open and repeated saves (#5219); broader corpus qualification and readiness estimates unchanged); previously trivial (arrowheads keep their kind, width and length and are drawn at Word's sizes; no ranked gap changed); previously trivial (.doc default font from the stylesheet's standard font, #5066; no ranked gap changed); previously trivial (a middle or bottom cell of a row split across pages lines up on the first page; no ranked gap changed); previously trivial (tab leader dots spaced and placed as in Word; no ranked gap changed); previously trivial (a list level above counts its first number at the first deeper item; no ranked gap changed); previously trivial (only the PDF sent to the printer leaves out link annotations; Save these pages as PDF keeps them, #4734; no ranked gap changed); previously trivial (documents with links print: the print job leaves out link annotations, #4734; no ranked gap changed; previously trivial: aligned-copy review fixes retain the document axis through scrolling and include secondary selections (#4764)); previously trivial (connector rerouting indexes laid-out object positions once per batch (#3631); previously trivial (custom-property bookmark-link preservation, #4767); previously trivial (Relative font-size commands preserve mixed sizes in the selection (#4903); existing button steps and one-point shortcuts are unchanged; readiness estimates unchanged); previously trivial (Arabic Backspace removes one combining mark and keeps its base letter (#4757); previously trivial (note conversion reference-run membership uses one ordered walk, #4750); previously trivial (DOCX comment-ID save lookups); previously trivial (gap 19: RTF reads and writes WMF and EMF pictures, #4259); previously trivial (#4658 bounds nested Markdown reference-label work); previously trivial (HTML import keeps preformatted lines with CR or CRLF input newlines, #4810); previously trivial (Recipient labels continue across product-sized sheets instead of stopping after the first sheet (#4880)); previously trivial (View windows: several open documents and Switch Windows in the engine; previously trivial: explicit documents keep the editor when recovery discovery finishes; readable recovery controls; previously trivial: text mini toolbar available with the right-click menu, #2959; previously trivial: personal templates in File › New and Save as Word Template, #2852; previously trivial: tracked table/row/cell/section property changes record in WordCraft, narrowing gap 5; previously trivial: a document with many content controls saves in the time the controls take, #13; previously trivial: Format Page Numbers dialog carries Word's number formats and Continue from previous section / Start at, #1113; previously trivial: centred list labels after leading page breaks, #2964; previously trivial: fixed font inputs in four layout controls; previously trivial: a right, centre or decimal tab whose text doesn't fit, or a left tab past the right indent, goes to the next line with its text, #1840; previously trivial: Bullets and Numbering toggle off style-inherited lists at their effective level, including basedOn chains and numbered ListParagraph styles; other paragraphs and list definitions stay unchanged (related to #3459); previously trivial: Russian sorting keeps ё with е and recognises decomposed ё/й; previously trivial: #8: English dialects — en-GB, en-CA, en-AU… flag each other's spellings, #1281; previously trivial: #1: shape gradients and transparency, #1500; previously trivial: text typed into a content control takes the control's formatting; localized placeholder styles recognised; previously trivial: forms protection keeps its content controls: no inserting, Properties or Remove Content Control; previously trivial: clearing a no-match font query keeps the recent heading visible, #2068; previously trivial: line breaking: a slash opening a word travels whole to the next line, #2963; previously trivial: recipient list search and merged-only view, #1576; previously trivial: #2 impact restated: only ink rendering and OLE editing remain; previously trivial: #2 narrowed: native ink nested in groups and drawing canvases kept on save; previously trivial: opening a DOCX no longer copies its pictures; previously trivial: RTF export writes sections (gap 19); previously trivial: ODT export writes sections (gap 19); previously trivial: automatic Asian/Latin and Asian/number spacing laid out, #2358; previously trivial: hint-reliant CJK outlines; previously trivial: numbered headings, #1793; previously trivial: legacy outline, shadow, emboss and imprint text effects drawn, #2534; previously trivial: #8 narrowed: Indonesian proofing from the installed Hunspell dictionary, #498; previously trivial: glossary/building blocks closed; previously trivial: Split Cells and Convert Text to Table follow the Word check, #2592; previously trivial: Language dialog keeps inherited proofing language and no-proof choices (#2717); previously trivial: #29: a copied equation also goes in the MathML clipboard formats, #1297; previously trivial: #24: the Spike, Ctrl+F3 and Ctrl+Shift+F3; previously trivial: VML freeforms read and kept on save; previously trivial: consistent preferred cell widths; previously trivial: multi-paragraph field instructions no longer add blank lines; previously trivial: page-break-only list paragraphs hide their labels; previously trivial: floating-table overlap ignores empty line-break lines; previously trivial: copies keep their formatting on the clipboard as HTML, #1523; previously trivial: sample-text formulas on Enter, #1067; previously trivial: #29: copied equations as MathML, OMML open as #1297, #816; previously trivial: the Font list no longer keeps every previewed font in memory, #848; previously trivial: table column and row markers on the rulers, #700; previously trivial: table borders and the resize handle drag on the page, #217; previously trivial: formatting revisions landed, #41; previously trivial: Draw Table and Eraser landed, #303; previously minor: beta distance restated: ~15 points, ~150–230 h; previously: first version; every known shortfall from the 2026-10-10 re-measure, ranked) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4
> 2026-10-27 · Technical synchronization: existing Proofing pane hooks and regression tests follow the extracted engine/UI modules on main `436d444`; compact context menus remain upstream. New-SHA validation is pending; readiness unchanged.
>
> 2026-10-23 · cu20we review follow-up: Only paragraph-relative box floats use the pre-spacing origin. Line-relative floats and page/margin fallbacks in cells and text boxes keep the text-top origin. Synthetic controls compare zero and 24 pt space-before across all four box contexts. New cap conversion helpers and regression tests document their contracts; readiness estimates unchanged.

> 2026-10-23 · cu20we: DrawingML flat, square and round line caps are imported, saved, displayed and exported to PDF; absent or unknown caps default to flat. VML end caps (including shapetype inheritance) are preserved through DOCX save. Ink and metafile stroke handling remain separate. Synthetic DOCX, raster and PDF regressions cover the change; no new native Word comparison.

> 2026-10-23 · cu20we: Paragraph-relative floats in headers, footers, cells and text boxes now start above effective space before; synthetic layout regressions cover all four contexts. Framed paragraphs already used the pre-spacing position. No new native Word comparison; broader object-position parity remains open.

| Date | Change | Summary |
|---|---|---|
| 2026-11-02 | trivial | #26 narrowed: MHT opens and `w:altChunk` content is brought in on open; saving MHT remains |
| 2026-11-01 | trivial | Preserve table rows and cells inside selected alternate-content branches (#5219); readiness estimates unchanged. |
| 2026-11-01 | trivial | Arrowheads keep their kind, width and length (DrawingML `a:headEnd`/`a:tailEnd` `type`/`w`/`len`, VML `startarrow`/`endarrow` with `…arrowwidth`/`…arrowlength`), read and written back, and are drawn 2, 3 or 5 times the line's width long and wide (small, medium, large), a line under 2 points as a 2-point one, as measured in Word for Windows; every kind is still drawn as a triangle; no ranked gap changed; readiness estimates unchanged |
| 2026-10-31 | trivial | The .doc reader's default font is the stylesheet's standard font, not the font table's first entry (#5066); no ranked gap changed; readiness estimates unchanged |
| 2026-11-01 | trivial | A middle or bottom cell of a row split across pages lines up on the first page; no ranked gap changed; readiness estimates unchanged |
| 2026-11-01 | trivial | Tab leader dots spaced and placed as in Word; no ranked gap changed; readiness estimates unchanged |
| 2026-10-31 | trivial | A list level above that hasn't counted yet takes its first number at the first deeper item, as measured in Word (10.1–10.4, then 11); not a ranked gap; readiness estimates unchanged |
| 2026-10-30 | trivial | Only the PDF sent to the printer omits link annotations; File › Print › Save these pages as PDF (`file.printPdf`) keeps them, like Export (#4734); no ranked gap changed; readiness estimates unchanged |
| 2026-10-29 | trivial | The print job's PDF omits link annotations, which the spooler refused to fit to the paper, so documents with links reach the printer (#4734); physical printer output remains to be validated; no ranked gap changed; readiness estimates unchanged |
| 2026-10-31 | trivial | Synchronize the existing Proofing pane with main `eecbb8f`; preserve explicit-position lookup, implicit selection, navigation, undo and per-window state; fresh exact-SHA hosted validation pending, readiness unchanged |
| 2026-10-30 | trivial | Synchronize the existing Proofing pane and navigation/selection/undo regressions with main `3b7f7ed`; new-SHA validation pending, readiness unchanged |
| 2026-10-28 | trivial | Aligned copies retain their locked document coordinate during middle-button scrolling and can start on secondary selected objects; existing Ctrl and Shift selection behavior stays (#4764) |
| 2026-10-28 | trivial | Ctrl/Command+Shift drags a selected floating object into an axis-aligned copy (#4764); synthetic Linux interaction and persistence controls, native desktop and Word untested; gap/readiness estimates unchanged |
| 2026-10-28 | trivial | Connector object lookups retain the typed column-selection inputs on current main (#3631); readiness estimates unchanged |
| 2026-10-27 | trivial | Connector rerouting indexes laid-out object positions once per batch, preserving page hints, first-match order and same-page glue; synthetic equivalence, geometry and scaling regressions (#3631); routing policy and readiness estimates unchanged |
| 2026-10-28 | trivial | Preserve custom-property bookmark link targets through saves, value edits and dialog renames (#4767); no linked-value evaluation or readiness change |
| 2026-10-29 | trivial | Relative font-size commands preserve mixed sizes in the selection (#4903); existing button steps and one-point shortcuts are unchanged; readiness estimates unchanged |
| 2026-10-28 | trivial | Arabic Backspace removes one combining mark and keeps its base letter (#4757); stored mark order, caret/forward-Delete and other-script behavior retained; original synthetic command/history/protection/save-reopen checks, no native Word comparison; readiness estimates unchanged |
| 2026-10-28 | trivial | Note conversion reference-run membership uses one ordered walk, #4750; readiness estimates unchanged. |
| 2026-10-29 | trivial | DOCX comment IDs use ordered paragraph lookup and one durable-ID reservation set; threading and ID rules unchanged |
| 2026-10-28 | trivial | #4658: bound Markdown reference-label rejection work, preserving literal source and ordinary link matching |
| 2026-10-29 | trivial | Recipient labels continue across product-sized sheets instead of stopping after the first sheet (#4880). Filtered/sorted order, product geometry and single-label mode retained; readiness estimates unchanged. |
| 2026-10-29 | trivial | HTML import keeps preformatted lines with CR, CRLF or LF input newlines (#4810); numeric character references retain their separate meaning; readiness estimates unchanged. |
| 2026-10-26 | minor | Word-observed Proofing task pane uses shared review commands; clicking a suggestion corrects and advances; context, suggestion cards and ignore/dictionary links follow the Windows reference; inspected previews prompted a full-width context and scrollable three-card list; exact selected spelling takes priority over overlapping grammar, with command/pane regressions and integration workflow paths; readiness estimates unchanged |
| 2026-10-22 | trivial | View windows: several open documents and Switch Windows in the engine |
| 2026-10-25 | trivial | egui 0.37 migration: interface catalogs remain pinned by `craft-i18n.rev` (`9205108…`; 27 languages, English fallback); IME surrounding deletions and composition resets are implemented with regressions, but managed CI passed; native IME acceptance and native-speaker catalog review remain open; no gap or readiness change |
| 2026-10-17 | trivial | Gap 9 narrowed: Standard (Word 2007, AES-ECB) encrypted packages open with a password under any Word extension; AES headers in Word 97-2003 files are outside the public spec and refused with a clear message; estimate 3–5 → 0 h, since the remaining AES-in-`.doc` case is refused by design |
| 2026-10-25 | trivial | Small follow-ups from the field, table and AutoFormat batch (#4232) |
| 2026-10-25 | trivial | Highlight decisions include fully selected paragraph marks in whole cells and ordinary ranges, including empty paragraphs; partial text columns exclude unselected marks (#4085). |
| 2026-10-25 | trivial | Highlight toggle checks the full target beyond 10,000 runs without collecting resolved formatting; mixed and inherited tails and Undo/Redo covered (#4085). |
| 2026-10-24 | trivial | Highlight shortcut toggles the last chosen colour off when the whole selection has it; explicit colour commands remain setters (#4085). |
| 2026-10-22 | minor | #8 narrowed: Spanish proofing (es-ES, es-MX, es-419, es-UY spelling from the installed Hunspell dictionaries, Uruguayan `vos` forms, generic and opening-mark checks, #3360, #3553) |
| 2026-10-25 | trivial | Text Highlight Color buttons show the remembered colour and no bar after No Color (#4087); no ranked gap or readiness estimate changes |
| 2026-10-23 | trivial | A command that changes nothing adds no undo step, marks no unsaved changes and evicts nothing from the undo history; the redo stack survives it. No gap row (defect fix); estimates unchanged. |
| 2026-10-23 | trivial | cu20we: preserve and render outline caps across DOCX/VML, screen and PDF; synthetic regressions; readiness estimates unchanged |
| 2026-10-23 | trivial | cu20we: paragraph-relative box floats use the pre-spacing origin; header/footer/cell/text-box regressions; readiness estimates unchanged |
| 2026-10-24 | trivial | Exact-spacing table contact uses the specified line height in row selection and word skipping; large/small mark regressions in modes 11/12/14/15 (#3977); shape estimates unchanged |
| 2026-10-24 | trivial | Floating tables count exact line-bottom contact; shapes keep strict overlap (#3977); original boundary/skip regressions in modes 11/12/14/15, expected Word rule from cyril265; readiness estimates unchanged |
| 2026-10-22 | trivial | Gap 3: proofing robustness — linear affix rule walks, bounded dictionary reads, linear Russian hyphenation, Turkish word cap (#3613 part, #3661, #3657, #3625); readiness estimates unchanged |
| 2026-10-22 | trivial | Gap #3 note: document model quadratic scans and text box paste copies bounded (#3692, part of #3614) |
| 2026-10-22 | trivial | Gap 10: group connector rerouting preserves rotated/flipped free and unresolved endpoints under nonuniform scaling, with safe degenerate-dimension and display-size-cap handling; routing around shapes and readiness estimates unchanged |
| 2026-10-21 | trivial | The ranked gap list held two rows each for #19, #20 and #21; the stale copies are gone, so each gap appears once, and the #20 row carries the measured figure (#3411) |
| 2026-10-21 | trivial | #20 narrowed: a document with many relationships opens in the time its part takes (#3411) |
| 2026-10-22 | minor | Compare's new and revised untitled results clear the previous document's version snapshots; a rejected protected comparison or mailing result preserves the snapshots and file identity. No gap row; estimates unchanged. |
| 2026-10-21 | trivial | #20 narrowed: a word with no break opportunity in it no longer makes the layout quadratic (#3412) |
| 2026-10-20 | minor | The Document Inspector's removals are refused while the document tracks changes (they aren't tracked), and a version snapshot belongs to the document it was taken from. No gap row (enforcement fix); estimates unchanged |
| 2026-10-22 | trivial | Markdown code spans normalize line endings before export, keeping block markers literal (#3738); format readiness unchanged |
| 2026-10-18 | trivial | Explicit New/Open is not interrupted by startup recovery discovery; older copies stay available and Recover follows the interface theme. |
| 2026-10-21 | trivial | Bullets and Numbering toggle off style-inherited lists at their effective level, including basedOn chains and numbered ListParagraph styles; other paragraphs and list definitions stay unchanged (related to #3459) |
| 2026-10-21 | trivial | #3172: the ribbon's font name and size boxes, and the dropdown arrows beside them, say what they are to a screen reader |
| 2026-10-21 | trivial | Gap 19: ODT table column widths are read on import, so they survive a save |
| 2026-10-21 | trivial | Pin the font input of four geometric layout controls; assertions and renderer font policy unchanged. |
| 2026-10-21 | trivial | #20 narrowed: a document with many content controls saves in the time the controls take (#13) |
| 2026-10-20 | trivial | Russian sorting keeps ё with е and recognises decomposed ё/й; synthetic ordering, case and index-heading regressions; broader parity estimates unchanged |
| 2026-10-20 | trivial | Nested-picture textboxes retain their frames; WPS picture fills keep their existing image fallback. |
| 2026-10-20 | trivial | Corrected Cuban and Peruvian Spanish table-number parsing and formula results to use decimal points; broader localization estimates unchanged |
| 2026-10-20 | trivial | Set As Default persists the proofing language across restarts; command and dialog regressions cover new documents, interface changes, existing document languages and rejection rollback (#2882); readiness estimates unchanged |
| 2026-10-20 | trivial | Restrict Editing, three more gaps closed: the protection of a protected document only changes through the explicit stop, the session author can't change through `file.setAuthor` under protection, and the post-check compares the settings and the metadata. No gap row (enforcement fix); estimates unchanged |
| 2026-10-22 | trivial | The CLI author flag and the Options user name field now respect that protected-document refusal (#3279 follow-up). No gap row; estimates unchanged. |
| 2026-10-20 | trivial | Restrict Editing: a file command that changes the open document is checked like any other (the privacy options, the properties, the inspector's removals), and restoring a stored version keeps the protection and its password. No gap row (enforcement fix); estimates unchanged |
| 2026-10-19 | trivial | #4: a right, centre or decimal tab whose text doesn't fit, or a left tab past the right indent, goes to the next line with its text, so a TOC entry's page number keeps its leader (#1840) |
| 2026-10-19 | trivial | #7: Format Page Numbers dialog (#1113), 60 modal dialogs |
| 2026-10-19 | trivial | The text context menu includes the existing two-row formatting mini toolbar, at a caret or over selected text (#2959); formatting and its dropdowns preserve the selection, regular menu commands and dismissal still close the popup. Paste options remain in separate PR #2599. |
| 2026-10-18 | trivial | Gap 8 narrowed: German text hyphenates by the TeX German patterns (reformed spelling); German spelling and grammar remain |
| 2026-10-19 | trivial | Restrict Editing: with Filling in forms enforced, inserting content controls, their Properties and Remove Content Control are refused, as in Word (it greys them out), so a form's fields stay as they are and only their content is filled in |
| 2026-10-19 | trivial | Content controls: text typed over a placeholder takes the control's own run properties (`w:sdtPr/w:rPr`, ECMA-376 §17.5.2.27), as in Word, and the placeholder style is recognised by its built-in name "Placeholder Text" whatever its localized id (German Word: `Platzhaltertext`), so entries in forms from localized Word no longer stay grey |
| 2026-10-18 | trivial | Font list starts at its first row after clearing a no-match query; recent heading remains visible (#2068) |
| 2026-10-19 | trivial | #2964: a centred or end-aligned list label after a leading page break uses its text line's alignment. Synthetic LTR/RTL numbered/bulleted controls and the issue's DOCX export; broader pagination gaps and readiness estimates unchanged |
| 2026-10-17 | trivial | #1327: Restrict Editing exceptions no longer let table, row or cell formatting change protected tables; formatting-only protection refuses Paste Formatting, table shading/borders/alignment/margins/text direction, table-style edits and locked table styles; recipient-list commands no longer rewrite merge-field results in a protected document. No gap row (enforcement fix); estimates unchanged |
| 2026-10-17 | trivial | #21: Find in list and "Show only records that will be merged" in Edit Recipient List (#1576) |
| 2026-10-18 | trivial | #15 narrowed: Word's `Sources.xml` bibliography file imports into Manage Sources (#2640) |
| 2026-10-18 | trivial | #2445: pending Save As answers keep their originating document identity, so a replacement document is never written to an old picker destination; real Save-button and focused-editor shortcut controls verified |
| 2026-10-16 | trivial | #2445: interactive Save on import-only .doc/.dot paths uses the existing DOCX Save As flow; cancel and failed writes keep the draft, and Close waits for a successful save. Explicit/script saves still report unsupported legacy output |
| 2026-10-17 | trivial | #1327: Restrict Editing exceptions no longer let table, row or cell formatting change protected tables; formatting-only protection refuses Paste Formatting, table shading/borders/alignment/margins/text direction, table-style edits and locked table styles; recipient-list commands no longer rewrite merge-field results in a protected document. No gap row (enforcement fix); estimates unchanged |
| 2026-10-17 | trivial | #21: Find in list and "Show only records that will be merged" in Edit Recipient List (#1576) |
| 2026-10-16 | trivial | #24: the Spike (Ctrl+F3, Ctrl+Shift+F3) |
| 2026-10-18 | trivial | #1: shape gradients and transparency (#1500): Word 365 cover pages' theme gradients and translucent shapes now open, draw and save |
| 2026-10-17 | trivial | Glossary/building blocks closed (DOCX glossary read, kept and written); `w:altChunk` and sub-documents remain |
| 2026-10-15 | trivial | #5 narrowed: Track Changes Options › Moves (Moved from / Moved to marks and colours) and Track moves (`w:doNotTrackMoves`) landed; drag-and-drop moves remain |
| 2026-10-17 | trivial | Language dialog keeps inherited proofing language and no-proof choices (#2717); readiness estimates unchanged |
| 2026-10-15 | trivial | Gap 19: RTF tracked paragraph marks and character/paragraph formatting revisions read and written; RTF section/table and ODT formatting revisions remain |
| 2026-10-14 | trivial | #19 narrowed: ODT export done (master page and page layout per section, columned `text:section`s) |
| 2026-10-17 | trivial | Explicit DrawingML preset/custom outline dashes survive import/save and paint in raster/PDF, scaled to the effective stroke width even for zero-width paths. Synthetic controls; theme-inherited dashes and line caps remain unverified. |
| 2026-10-15 | trivial | `wordcraft-cli convert --update-fields` explicitly refreshes unlocked fields before conversion; ordinary conversion keeps cached results. Field refresh can change pagination |
| 2026-10-17 | trivial | Arial Unicode MS uses Arial and its proportional substitutes; glyph coverage and exact Word line height remain font-dependent. |
| 2026-10-18 | trivial | #7: Split Cells and Convert Text to Table follow the Word check (#2592) |
| 2026-10-16 | trivial | #7: Split Cells and Convert Text to Table dialogs (#2592), 58 modal dialogs (matches ui-parity.md) |
| 2026-10-16 | trivial | #8 narrowed: Indonesian proofing (id-ID spelling from the installed Hunspell dictionary, generic checks, #498) |
| 2026-10-16 | trivial | Legacy outline, shadow, emboss and imprint text effects drawn (#2534) |
| 2026-10-15 | trivial | Numbered headings (#1793): heading-linked Multilevel List presets; numbering keeps heading styles. Not a ranked gap; style gallery previews without numbers remain |
| 2026-10-16 | trivial | Hint-reliant CJK fonts execute TrueType contour assembly; Windows DFKai-SB no longer loses strokes. Synthetic local-font regression; no font binaries bundled. |
| 2026-10-16 | trivial | #17: automatic space between Asian text and Latin text or digits laid out (#2358) |
| 2026-10-16 | trivial | Drawing dispatch uses the direct graphic-data child so nested pictures keep their text boxes. Synthetic regressions. |
| 2026-10-14 | trivial | Gap 19: RTF export writes sections (ODT export landed too) |
| 2026-10-14 | trivial | #19 narrowed: ODT export done (master page and page layout per section, columned `text:section`s) |
| 2026-10-15 | trivial | Additive paragraph spacing retains the full before-space after a section page break; ordinary overlapping spacing is unchanged. |
| 2026-10-15 | trivial | Exact line spacing keeps the run font baseline around tall equations; automatic and minimum spacing still grow for math. |
| 2026-10-16 | trivial | Zero-extent drawing anchors contribute no inline font height. Synthetic regressions. |
| 2026-10-15 | trivial | #2 impact restated: Word ink is no longer lost; it shows as its picture and can't be edited |
| 2026-10-15 | trivial | #2 narrowed: native ink in groups and drawing canvases (`w14:contentPart` members, with their picture fallback) survives DOCX edits and repeated saves; native ink rendering remains |
| 2026-10-16 | trivial | Body and active note text keep normal opacity while editing footnotes or endnotes (#2545); header/footer dimming is unchanged |
| 2026-10-18 | minor | #7: Drop Cap Options dialog (#2601), 59 modal dialogs |
| 2026-10-16 | trivial | An otherwise empty section at page start retains its line before continuous content; later closing marks stay quiet |
| 2026-10-17 | trivial | SECTIONPAGES includes shared continuous-section pages in body and header/footer fields |
| 2026-10-17 | trivial | Footer page tables preserve text height and wrap intersecting body lines |
| 2026-10-17 | minor | File › Print's Scale to paper size (#2108): pick Letter, Legal, A4 or any Layout › Size paper and every page is scaled to it and centred, the paper turned to each page's orientation, in the preview, the print job (with the spooler's media) and Save these pages as PDF; the document keeps its page size. `paper` param on `ui.print`/`file.printPdf` |
| 2026-10-17 | trivial | Non-overlapping body floating tables clear header tables; overlapping tables keep their requested position |
| 2026-10-16 | trivial | #8: English is three proofing languages — American, British spelling (en-GB, en-AU, en-NZ, en-IE, en-ZA, en-IN…) and Canadian — each flagging the other's spellings (#1281, #1045) |
| 2026-10-16 | trivial | #10 narrowed: pictures and shapes without text stories preview rotation while dragging (#2277); remaining object gaps and estimates unchanged |
| 2026-10-14 | trivial | Row table border exceptions fixed; other row table exceptions remain unsupported |
| 2026-10-12 | minor | Czech proofing review: short-word splits rank after deletion, NOSUGGEST excludes suggestions while accepting typed forms, affix comments apply only to headers, ratios keep spaces and repeated-word messages use Czech quotes; synthetic engine end-to-end and latency checks |
| 2026-10-15 | trivial | Three-column header/footer preset uses a borderless table with editing guides (#2197); broader gaps and estimates unchanged |
| 2026-10-12 | minor | Czech spelling from the installed Hunspell dictionary, runtime-table suggestions and generic checks; Czech interface writes cs-CZ documents; no Czech hyphenation patterns bundled; estimates unchanged |
| 2026-10-16 | trivial | Gap #20: opening a DOCX no longer copies its pictures |
| 2026-10-12 | minor | Gap #24 default shortcuts expanded (140 commands on macOS / 158 on Linux/Windows); stale gaps #9 (DOCX agile encryption already works, only encrypted .doc missing) and #15 (DOCX bibliography sources already saved, only desktop integrations missing) updated |
| 2026-10-13 | trivial | Previously untracked long Web/Draft page blur resolved with visible high-resolution raster tiling; no gap remains for this issue |
| 2026-10-14 | trivial | #1: VML freeforms (rule lines in headers) read and kept on save |
| 2026-10-14 | trivial | Consistent preferred cell widths |
| 2026-10-14 | trivial | Pagination gap #4 narrowed: paragraph boundaries inside complex-field instructions add no flow height. Floating-table placement and other paragraph metrics remain open. |
| 2026-10-14 | trivial | A list paragraph containing only a page break has no visible label; it still advances the list counter. Ordinary empty list items keep their label. |
| 2026-10-14 | trivial | Field report: grouped `flowChartOr` omitted its crossing spokes; the preset now draws them; wider DOCX corpus work remains open |
| 2026-10-14 | trivial | Floating-table overlap checks use visible line content; empty line-break lines no longer move a table |
| 2026-10-15 | trivial | #12: Ink to Shape pairs a line with a separately drawn V or closed-triangle head, and recognises one-stroke arrows with a closed triangle head |
| 2026-10-13 | trivial | Gap #3: Behind Text pictures and shapes drag over text in the body and headers, selected or not; a click without movement still edits the text |
| 2026-10-13 | trivial | Row 1: the watermark is saved to DOCX headers (#1290) |
| 2026-10-13 | trivial | Row 1: Word's text watermark in DOCX headers is drawn (#1290) |
| 2026-10-12 | trivial | Re-audited every row against main: #9 narrowed to encrypted `.doc` (#281), #15 sources saved in DOCX (#790), #10 group landed (#267), #11 French landed (#729), #12 Draw 10/11, #16 #215 closed, #17 `para.asianTypography` live, #3 closed reports and 197 open issues, #8/#19/#21/#25/#27 open PRs named, evidence counts refreshed |
| 2026-10-12 | trivial | Document title opens Rename Document; the account badge opens Options › General for user name and initials. |
| 2026-10-14 | trivial | Comment commands reject malformed explicit IDs instead of truncating floats or targeting the caret (#1690, finding 3); synthetic command, undo/redo and DOCX save/reopen coverage; readiness estimates unchanged |
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
| 2026-10-16 | trivial | #29: a copy of one equation alone also goes in the MathML clipboard formats Word reads first (#1297); the gap keeps confirming the paste in Word |
| 2026-10-10 | trivial | #29: long display equations wrap across lines (#326) |
| 2026-10-10 | trivial | #3: Windows lag on GPUs without a DirectX 12 driver fixed with an OpenGL fallback (#316) |
| 2026-10-10 | minor | #8: Polish proofing (spelling, grammar, hyphenation) and a proofing language per run landed; the gap is now the other languages (20–40 h) |
| 2026-10-10 | trivial | #6: desktop File › Print opens the system PDF viewer (#286) |
| 2026-10-10 | trivial | Table Properties dialog landed (#44); removed from the dialog-depth list |
| 2026-10-10 | trivial | Merged main: landed features removed from the feature-gap list |
| 2026-10-10 | minor | Beta note restated: full number back to the additive ~60% (no new evidence) |
| 2026-10-11 | trivial | #491: vertical sections landed; vertical text removed from the East Asian gap |
| 2026-10-10 | major | First version, from the full re-measure: 29 ranked gaps with evidence, impact and hours |
| 2026-10-10 | trivial | #307: Draw tab pens, eraser and Hide Ink landed; ink gap narrowed |
| 2026-10-12 | trivial | #21: .xls workbooks and GBK/Big5/Shift_JIS/UTF-16 text lists as recipients; Filter and Sort in Edit Recipient List (#778) |
| 2026-10-12 | trivial | #811: large-table scrolling fixed; performance gap (#20) narrowed |
| 2026-10-12 | trivial | #13: radar, stock and other undrawn chart types show a placeholder instead of blank space (#998) |
| 2026-10-19 | trivial | Personal templates in File › New and Save as Word Template (#2852); no gap entry tracked it |
| 2026-10-29 | trivial | #15 narrowed: a Word bibliography source with a corporate author keeps it (read, saved, cited and listed whole), and Create Source has the Corporate Author box (#4848) |
| 2026-10-29 | trivial | Table Properties: row, column and cell settings apply to the selected rows, columns and cells, not only the caret's (#4857); no gap entry tracked it |
| 2026-10-28 | trivial | Gap 19 narrowed: RTF reads and writes WMF and EMF pictures, chosen by their bytes (#4259); ranking unchanged |

## Layout validation fonts

Four geometry-dependent regression controls request Source Sans 3 explicitly. Platform-installed substitutes otherwise change their expected line counts. Assertions remain unchanged; renderer font metrics and substitution gaps remain open.
