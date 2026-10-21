# Layout and pagination parity with Microsoft Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-18 · **Change:** trivial (the caret, clicks and selection edges fall between the letters of a ligature, #1974; previously trivial: a word after a slash wraps whole, like Word, #2963; previously trivial: VML header-cell floats always on the page, user-checked in Word for Mac, #1775; previously trivial: an at-least row with no room for its minimum starts the next page whole, #1742; previously trivial: AutoFit donors lose equal widths down to their content minima; previously trivial: automatic space between Asian text and Latin text or digits, #2358; previously trivial: hint-reliant CJK outlines; previously trivial: positional tabs go to the margins or indents, #2367; previously trivial: field results with right-to-left text read right to left, #1670; previously trivial: a list bullet no installed font can draw shows the level's default bullet, #1683; previously trivial: a missing Thai font is drawn in one Thai face for the whole run, #1549; previously trivial: the caret after spaces hanging past the margin stays at the margin, #2146; previously trivial: covered black-circle fallback before UI sans; previously trivial: resolved split-cell continuation top insets; previously trivial: Symbol PCL6 alias uses Symbol encoding; previously trivial: exact full font names select their installed face; previously trivial: Arial Unicode MS uses proportional fallbacks; previously trivial: DrawingML outline dashes, including zero-width paths; previously trivial: TOCs include table-cell headings and retain full targets; previously trivial: kept table rows follow the next row; previously trivial: consistent cell widths override a stale table grid; previously trivial: multi-paragraph field instructions no longer add blank lines; previously trivial: page-break-only list paragraphs hide their labels; previously trivial: floating-table overlap ignores empty line-break lines; previously trivial: paragraph spacing adds up when the document turns HTML auto spacing off; previously trivial: an inherited footer sizes its percentage tables by its own section; previously trivial: an empty section mark stays on a full page; previously trivial: a fitting row with less room than its minimum height moves on whole; oversized rows still split; previously trivial: text past an exact-height row is cut off; previously trivial: a list number stays with text after a leading page break; previously trivial: a bookmark after a page break takes no line; previously trivial: a page break followed by a page break before or a next-page section starts one page; previously trivial: All Caps and Small Caps draw ß as SS, #1581; previously trivial: justified vertical page alignment, #1320; previously trivial: page numbers restart at 0; previously trivial: a list label moves with its first line when the paragraph is centred or end-aligned (#1159); previously trivial: PAGE fields in a paragraph split across pages show the page they are on, #1352; previously trivial: a dragged floating object anchors in the column it lands in, #884; previously trivial: a paragraph ending on a page with footnotes keeps its space after above the separator, #1116; previously trivial: symbol-font text drawn as Unicode symbols, #776; previously trivial: large-table scrolling measured and fixed, #811; previously trivial: text boxes use their own margins; Resize shape to fit text never cuts text, #743; previously trivial: complex-script shaping and Myanmar line breaking, #604; previously trivial: body page totals, table-cell footnotes and bookmark link pages fixed, #450 #451 #455; previously major: first version; line breaking, pagination and object placement checked against Word's behaviour from the layout source on origin/main) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

A document only "opens correctly" if its pages break where Word breaks them: the same lines, the
same page count, footnotes and pictures on the same pages. This checklist covers line breaking,
pagination and object placement in `crates/layout`. Typing behaviour is in
[`typing-parity.md`](typing-parity.md); the reading and writing of the properties is in
[`file-format-parity.md`](file-format-parity.md).

**Dimension: ~55% (estimated), 90–145 h to full** (the sum of the rows below). The engine implements most of Word's layout
rules, but nothing has been compared page-for-page with Word on real documents, and the default
font can't match Word's metrics.

Additive paragraph spacing retains the full before-space after a section page break; ordinary overlapping spacing is unchanged. Synthetic control: `layout::tests::section_start_summed_spacing`.

Exact line spacing keeps the run font baseline around tall equations; automatic and minimum spacing still grow for math. Synthetic control: `layout::tests::exact_math_keeps_run_baseline`.

## Checklist

✅ matches Word as far as our tests show · 🟡 implemented, differs or untested against Word · ❌ missing

| Feature | State | Evidence / gap | Hours |
|---|---|---|---|
| 2026-10-16 | trivial | Split-cell content retains its resolved top margin and border band on continuation pages, matching original Word Mac16.113.2 controls. Synthetic regressions cover margins, borders, line spacing and image frames. |
| Hint-reliant CJK glyph outlines (DFKai-SB / 標楷體) | 🟡 | Required TrueType interpreter runs at units-per-em size; local Windows glyph-bounds regression and synthetic DOCX page/window comparison pass. Other platforms and hint-reliant families remain unmeasured; no native Word comparison. | — |
| Subpoint table grid columns | 🟡 | Preserve positive grid widths rather than imposing a text-box minimum; ECMA-376 §17.4.16. Native Word for Mac 16.113.2 build26092012 confirms narrow trailing grid units. `layout::tests::subpoint_grid_keeps_widths`, `tiny_grid_scales_to_width`. Tiny totals normalize without overflowing the scale factor. Zero and invalid widths retain their fallback. Border painting remains separate. | n/a |
| Line breaking (UAX #14 with Word's exceptions: no break after `/`) | ✅ | `layout::tests::no_line_break_right_after_a_slash` | — |
| A break before a slash that starts a word (" /word" wraps whole; not after a no-break space) | 🟡 | `layout::tests::a_word_after_a_slash_wraps_whole`, `layout::tests::a_no_break_space_before_a_slash_still_holds`; behaviour from the #2963 report, not compared with Word yet | — |
| Narrow table cells | ✅ | `layout::tests::narrow_cells_keep_their_width`: usable widths below 12 pt wrap without expansion; oversized glyphs still make progress | 0 |
| Justification, Word 2013+ space shrinking (compat mode 15) | ✅ | #109 | — |
| Spaces at the end of a line hang past the right margin instead of wrapping; the caret after them stays at the margin, in the body and in headers and footers | ✅ | `layout::tests::caret_after_hanging_spaces_stays_at_the_margin` (#2146): a letter typed after them wraps to the next line | — |
| Complex scripts (Myanmar, Khmer, Indic): shaped per script run with OpenType fallback fonts, one cluster per syllable, Myanmar wraps between syllables | 🟡 | `layout::tests_complex` (#604). Khmer, Thai and Lao still wrap only at spaces (need a dictionary) | 4–8 |
| Kashida justification (Arabic) | ❌ | no kashida in `crates/layout` | 3–5 |
| Missing Thai document fonts (TH Sarabun New, Angsana New, Cordia New, Browallia New, the UPC families, Leelawadee): one Thai substitute face for the whole run, Latin letters, digits and ฿ included, so the run keeps one font at one size as Word's Cordia New substitution does | ✅ | `fonts::word::tests::a_missing_thai_font_resolves_to_an_installed_thai_face`, `fonts::fontdb::tests::a_thai_run_with_latin_falls_back_to_one_face`, `layout::tests_complex::a_missing_thai_font_draws_the_whole_run_in_one_thai_face`, `layout::tests_complex::a_thai_substitute_without_latin_hands_the_run_to_one_face_covering_it` (#1549). Windows: Cordia New, then Leelawadee UI, Tahoma, Noto Sans Thai; macOS: Thonburi (Sathu for the serif families); Linux: Noto Sans Thai, then the TLWG fonts. The first listed face with Latin letters and digits as well as Thai wins. Other Thai names (`TH …`, `…UPC`, names with `Thai`) use the same lists. When no listed face is installed, or the one found lacks the run's Thai or Latin letters, the run moves to one installed face covering its Thai and Latin text if there is one. Only Thai fonts: a missing Latin font keeps Latin text in its own substitute, and a run with its own complex-script font keeps it for the Thai letters, as Word does | — |
| Distributed / Thai distributed alignment | 🟡 | read; layout untested | 1–2 |
| Default font metrics (Aptos, Calibri, Cambria…) | 🟡 | Missing Arial Unicode MS tries Arial then the Arial substitutes, case-insensitively (`fonts::word::tests::unicode_arial_is_proportional`); glyph coverage and exact Word line height can differ. Carlito/Caladea/Liberation are metric-compatible for Calibri/Cambria/Arial/Times/Courier; **Aptos (Word's default since 2024) has no metric-compatible open substitute**, so new Word documents wrap differently ([typing-parity.md](typing-parity.md) Known gaps). Needs an owner decision (commission or license a metric-matched font) | 6–10 + owner |
| East Asian automatic leading | 🟡 | Font code pages select 30% of raw hhea ascent plus descent, split above/below and replacing hhea gap. Native Chinese, Korean and Latin code-page controls; exact spacing and fixed graphics retain raw metrics. `fontdb::tests::line_metrics_follow_code_pages`. Missing requested fonts still use substitute metrics. | n/a |
| Colour emoji and colour fonts on screen and in page renders: COLR v0/v1 paint graphs (solid fills, linear, radial and sweep gradients, composite modes) and bitmap strikes (Apple Color Emoji `sbix`, Noto Color Emoji `CBDT`) drawn in the font's colours at the layout's position and size; a dark page doesn't invert them, the same as pictures | 🟡 | `fonts::color` (skrifa), `render::color_glyph`; `render::tests_color_glyphs::emoji_are_drawn_in_colour` and `emoji_keep_their_colours_on_a_dark_page` (system font, skip without one), `render::color_glyph::tests`, `fonts::color::tests` (#1269). CBDT is verified on a real page with Noto Color Emoji; COLR v0/v1 and sbix are implemented to the OpenType spec and tested with synthetic data, but not yet seen on a real font (Segoe UI Emoji, COLRv1 Noto, Apple Color Emoji). PDF export already drew them through krilla | 1–2 |
| Full font names in document font requests | ✅ | Exact English (or first available) OpenType name ID4 lookup in loaded faces and the native lazy catalog; family and legacy-family matches keep priority. No style-suffix guessing; named variable-instance aliases remain unsupported | 0 |
| Symbol fonts (Symbol including its PCL6 alias, Wingdings, Wingdings 2/3, Webdings) without the proprietary font: each character, stored as U+F020–U+F0FF or a plain 8-bit code, is drawn as its Unicode equivalent from a face that has it; PDF text and copies read as those symbols | ✅ | `doc::symbol_fonts` tables (Adobe Symbol table and Unicode L2/12-368), `layout::tests::wingdings_text_draws_and_reads_as_its_unicode_symbols` (#776). Shapes come from Noto Sans Symbols / Symbols 2, DejaVu Sans or Segoe UI Symbol, so they differ from Word's; Wingdings 0xFF (not in Unicode) still has no symbol | 0 |
| List bullets stored as a private-use character (U+E000–U+F8FF) in a font that isn't installed and isn't a mapped symbol-font code: drawn as the level's default bullet (• ○ ▪), not as whatever an installed icon font has at that code | ✅ | `layout::tests::private_use_bullet_no_font_draws_shows_the_levels_default_bullet` (#1683: U+F081 showed Font Awesome's Twitter logo) | — |
| Indents in characters (`w:firstLineChars`, `w:leftChars`…), measured in the paragraph mark's font size | ✅ | #534, `layout::tests::character_indents_use_the_paragraph_mark_size`; spacing in lines (`w:beforeLines`/`w:afterLines`) uses the stored points (needs the document grid) | — |
| Line spacing (single, 1.5, double, at least, exactly, multiple), spacing before/after, contextual spacing | 🟡 | Contextual spacing measured in body paragraphs and headers (below), with a four-paragraph table-cell regression; list-label ascent overflow is unscaled by multiple spacing and uses descent for automatic and at-least spacing (#601); other line-spacing combinations remain (#108, #135 open) | 3–5 |
| Kerning only where Kerning for fonts asks for it (`w:kern`, at or above its size) | ✅ | Measured in Word for Windows: no `w:kern` or `w:kern="0"` is unkerned; WordCraft kerned every run, so its lines held more text than Word's. `layout::tests::kerning_follows_kerning_for_fonts_like_word` | — |
| OpenType ligatures as the document asks (`w14:ligatures`: standard `liga`, contextual `clig`, historical `hlig`, discretional `dlig`, any combination, in runs, styles and defaults) | 🟡 | `layout::tests::ligatures_follow_w14_ligatures`, `ligature_flags_map_to_opentype_features`. Without the property WordCraft keeps drawing standard and contextual ligatures, while Word draws none; the Font dialog's Ligatures choice is still missing | 1 |
| Automatic (HTML-style) paragraph spacing (`w:beforeAutospacing`, `w:afterAutospacing`) | ✅ | Measured in Word for Windows: 14 pt at any size, none at the top of the document or a cell or after a cell's last paragraph, the larger of two neighbouring spaces. `layout::tests::automatic_paragraph_spacing_like_word`, `…_drops_at_cell_edges` | — |
| Paragraph boundaries inside field instructions | ✅ | Synthetic DOCX observed in Word: instruction-only paragraphs add no height; the empty result keeps a line. `docx::fixtures::field_code_spans_paragraphs`, `layout::tests::hidden_marks_take_no_space`. Other field-code display modes remain unverified | — |
| Widow/orphan control, keep with next, keep lines together, page break before | ✅ | `lib.rs` keep_next/keep_lines; a line holding only a page break stays at the bottom of a full page instead of leaving a blank page (#534); a paragraph ending with a page break keeps its space after on the old page, so the next page starts at the top (compatibility modes 12 to 15, `layout::tests::space_before_at_the_top_of_a_page_like_word`); a three-line paragraph with room for two moves whole (neither a widow nor an orphan is allowed), as in Word | — |
| Tabs (left, center, right, decimal, bar), leaders | 🟡 | Bar tabs untested. Positional tabs (`w:ptab`, ECMA-376 §17.3.3.23) go to the left edge, centre or right edge of the margins or indents whatever the tab stops, and to the next line when the text has passed that place (#2367, `layout::tests::right_positional_tab_ends_text_on_the_right_margin` and the three after it); not compared with Word on lines narrowed by floating objects or right-to-left paragraphs | 1–2 |
| Hyphenation (automatic, zone from the document, words in capitals, soft hyphens, consecutive limit, Manual Hyphenation, #407) | 🟡 | English patterns only; other languages hyphenate wrongly or not at all | in localization |
| Drop caps (dropped and in margin, lines, distance, font; `w:framePr w:dropCap`) | ✅ | 2026-10-17 | — |
| Caret, clicks and selection inside a ligature (fi, ffi, lam-alef) | ✅ | The glyph's advance is shared evenly among the letters' grapheme clusters, mirrored in right-to-left text; a grapheme cluster (a letter with a combining mark, an emoji ZWJ sequence, an Indic conjunct) is never split. `layout::tests_ligature`, `engine::tests_typing::right_arrow_moves_through_a_ligature_one_letter_at_a_time` (#1974); observed in Word for Mac 16.113.4 | — |
| All Caps and Small Caps: the full capital of each letter, so ß draws as SS (ﬀ as FF); the capitals drawn for one character are one cluster, so the caret, selection, hit testing and line breaking treat ß as one character | ✅ | `layout::tests::caps_draw_sharp_s_as_two_capitals_in_one_cluster` (#1581). The PDF text layer still reads the document's lowercase letters until #1473 | — |
| Columns, column breaks, separators, balancing | ✅ | Columns balance on a section's last page when the next section starts continuous (also at the document end, which Word only balances when the document ends with a continuous break); not after next-page/even/odd breaks or on a page with a column break; keep-together, keep-with-next, widow/orphan control and unsplittable rows can leave them slightly uneven (#350) | — |
| Section breaks (next page, continuous, even, odd), different first page, odd/even headers | ✅ | Section breaks don't show in Draft view (#42). A break inserted with text selected goes before the selection and keeps it, as in Word (#852). An otherwise empty section at page start retains its line (`empty_page_section_keeps_line`); a closing mark after text stays quiet. | 1–2 |
| Labels on page-break-only list paragraphs | ✅ | `layout::tests::break_only_list_has_no_label`: no bullet or number on the break, but the next number is still 3 after item 1 and a break. Plain empty items keep their labels. Synthetic DOCX observed through Adobe PDF conversion | — |
| List labels: number formats (decimal, letters, Russian letters, roman, ordinal text, Chinese/Japanese/Korean counting and legal, 一〇一 digits, 甲乙丙, ①…⑳, full-width), `%n` text such as `第%1章` / `（%1）`, legal numbering | ✅ | `doc::cjk_numbers` tests, `doc::section::tests::russian_letters_follow_word`, `layout::tests::chinese_counting_list_labels_and_their_font_runs`, `layout::tests::russian_letter_list_labels_draw_cyrillic`; a label mixing faces (digit + CJK comma) is drawn run by run (#534); Russian letters skip ё, й, ъ and ь and then double (аа, бб), as observed in Word. Hebrew, Arabic, Hindi, Thai and Vietnamese formats show decimal The label stays attached to the start of the text when the paragraph is centred or aligned to the end, as in Word (#1159). | — |
| Body top below a tall header, footer pushes body up | ✅ | #138; a footer taller than the bottom margin moves the body bottom up (#453), and a header or footer linked to the previous section takes its room too (#454) | — |
| Tables: row border exceptions (`w:tblPrEx/w:tblBorders`) | ✅ | `row_borders_stay_on_row`, `row_nil_overrides_style`: explicit cell borders win; row exceptions precede table styles and table borders | — |
| Footnotes at the page bottom, endnotes at the end, note numbering | 🟡 | A long note continues onto the next page at a line boundary (its first line stays with the reference), first in that page's note area under a full-width continuation separator (#352); the document's own separator, continuation separator and continuation notice are drawn (the notice under a note that continues); on a page with footnotes a paragraph ending there keeps its space after above the separator, or its last lines go to the next page with the note (#1116); a long endnote continues on the next column or page at a line boundary (#456), and the caret finds note text on its continuation page (#457); each section's note options: footnotes below the text, endnotes at the end of each section, start at, numbering restarted each section or each page (counted from where the notes land, at most three passes) (#385) | 1–2 |
| Tables: row heights (at least/exact), rows split across pages, header rows repeat, Can't Split | ✅ | 2026-10-06, #138; omitted cell-margin edges inherit current table defaults; every cell in a row gets the row's largest top and bottom margin; an exact height covers the top border, top margin and text, the bottom margin comes on top, and an exact row never splits; a row split across pages keeps its cells' bottom margin, border and last space after on the first page, widow/orphan control applies in its cells, each cell continues at the top of the next page and an at-least row keeps its minimum; an at-least row with less room than its minimum on a page that already holds something starts the next page whole (Allow row to break on or off, vAlign centre too), as observed in Word for Mac 2026-10-15 (a row of about 2.6 cm), #1742; that repeated header rows go with it and that a row taller than a page splits from the top of the next page come from the issue and synthetic tests, not a Word check (`…_an_at_least_row_with_no_room_starts_the_next_page_whole_either_way_breaking_is_set`, `…_an_at_least_row_taller_than_a_page_splits_from_the_top_of_the_next_page`, `layout::tests::a_row_shares_its_largest_top_and_bottom_margins`, `…_a_split_row_keeps_its_bottom_margin_and_widow_control`, `…_split_row_cells_each_continue_at_the_top`) | — |
| Tables: autofit to contents/window, fixed widths, Word 2013 edge | 🟡 | #137; #44: AutoFit Contents measures each column's narrowest and widest text and shares the width like Word, once (not live as you type); consistent a percentage width past 100% is kept, as Word does; cell widths override stale grid proportions when their totals agree (`cell_widths_override_grid`); conflicting widths and spans still differ | 3–5 |
| Floating tables (`w:tblpPr`) | 🟡 | #137; one placed on the page or margin that would cover visible text before it and does not fit below it moves to the next page (#534); empty line-break lines do not count as text; other overlap rules untested | 2–3 |
| Nested tables | 🟡 | render; polish missing | 2–4 |
| Floating pictures/shapes placement (relative to page/margin/column/paragraph/line/character) | ✅ | #136; in a header or footer table cell, a float not laid out in its cell (`layoutInCell="0"`, VML `o:allowincell="f"`, or any VML shape in a header or footer: user-checked in Word for Mac) and placed relative to the page or margins is placed on the page, drawn with the header behind or in front of its text, on every page: Word's picture watermark in a header that starts with a table is centred behind the body text (#1775, `layout::tests::header_cell_float_placed_on_the_page`). Other cell floats are still placed relative to their cell | — |
| Text wrap: square, top-and-bottom, behind, in front | ✅ | 2026-10-06; beside an object, a line's first word that doesn't fit the strip goes on whole to the row's next strip or the next row, instead of being split (#1376, `layout::tests::a_word_too_wide_for_the_strip_beside_a_float_moves_on_whole`; inferred from a reporter's comparison, not yet measured in Word) a paragraph running on into the next column or page is broken again there around that column's shapes, its own floating objects left where they are (#1374). Text in an earlier column doesn't yet wrap around a shape anchored in a later one (#1158) | #884: an object dragged on a multi-column page anchors to a paragraph in the column it lands in, so that column's text wraps around it. Text laid out before the anchor (an earlier column) doesn't yet wrap around an object anchored later on the page, nor do lines a paragraph continues into the next column (broken once, in the column it starts in) |
| Text wrap: tight, through (contour, wrap polygon) | 🟡 | #354: lines follow the wrap polygon, or the outline worked out from the object (shape geometry; a picture's opaque pixels as a convex hull; otherwise the rectangle), turned and flipped with it, kept the left/right distance from text; Through also uses gaps inside the outline. Wrap side (`wrapText`: both sides, left only, right only, largest side per line) applies to square, tight and through wraps, turned objects included; picture outlines are convex; not compared page-for-page with Word | 1–2 |
| Text boxes, overflow, linked text boxes | ✅ | #46; linked text boxes (#369): the text runs on through each box in turn at that box's line length, overflow past the last box hidden; Align Text top / middle / bottom; the box's own internal margins, and a box set to Resize shape to fit text shows all its text (#743) | — |
| Rotated objects and text | 🟡 | #332: pictures, shapes, charts and groups drawn turned and flipped about their centre (screen and PDF); square wrap and inline lines keep clear of the rotated bounds; hit testing on the turned shape. Text in a rotated text box stays upright | 2–3 |
| Page borders (from page edge or text), page colour, watermark | ✅ | | — |
| Line numbers (restart per page/section, count by) | ✅ | | — |
| Vertical page alignment | ✅ | Top, Center, Bottom and Justified (#1320): justified shares each page's spare room equally between its paragraphs and tables, first line on the top margin and last on the bottom one (above the footnotes), the last page and a page ending in a manual page break too; a page with one block stays at the top; the last paragraph's space after stays inside the justified height (Word for Mac, user-checked). A section's last page takes its own section's alignment (it took the next section's before). Multi-column justified pages stay at the top (not compared with Word) | — |
| Gutter, mirror margins, book fold, 2 pages per sheet | 🟡 | gutter and mirror margins (even pages put the inside margin and gutter on the right, #452; page-relative floats stay put, and inside/outside alignment and margins follow the page side, #1181); book fold missing | 2–3 |
| Vertical text (`tbRl`) in sections and text boxes; cell text direction | ✅ | cell text direction (#245); text-box text direction (#369); vertical sections (`tbRl`/`btLr`, #491): the page's text area turned, lines top to bottom stacking right to left, ideographs, kana and Hangul upright, Latin on its side, caret and clicks through the turn. Missing: OpenType `vert` alternates (punctuation is moved, not substituted), arrow keys along vertical lines, upright pictures take turned room | 2–4 |
| Document grid (`w:docGrid`, lines per page, characters per line) | 🟡 | #391, #401: line grid snaps line heights to whole pitches (text centred in them; `w:snapToGrid` off opts a paragraph out); character grid puts East Asian characters on the character pitch (`linesAndChars`) or all text (`snapToChars`), Latin text proportional; View › Gridlines draws it. Not yet: run-level `w:snapToGrid`, grid in headers/footers, paragraph spacing in grid lines, Word's exact glyph placement in a cell | 1–2 |
| Ruby / phonetic guide, enclose characters, combined characters | 🟡 | ruby (Phonetic Guide) landed (#288): ruby text over its base by alignment, line grows to fit, in PDF; enclose and combined characters missing; ruby not yet compared with Word page by page | 2–4 |
| Automatic space between Asian text and Latin text or digits (`w:autoSpaceDE`, `w:autoSpaceDN`) | 🟡 | #2358: a quarter of the Asian character's font size after the first of two touching characters, across runs (measured on Word's PDF of a Chinese document at 16 pt, as JIS X 4051 asks); not beside East Asian punctuation, Hangul, Latin punctuation or an existing space; a field is classed by the text it shows; none at a line's end, none on a character grid the paragraph snaps to (`layout::tests_autospace`). Not yet: justification widening the space as Word does, `autoSpaceLikeWord95` | 1–2 |
| Bidirectional paragraphs, mixed-direction lines; RTL tables and sections | 🟡 | #207; RTL tables (#362): grid columns from the right edge, start alignment and indent from the right margin, cell margins and left/right borders mirrored, merged and turned cells and nested tables included; RTL sections (#362): newspaper columns right to left with the line between mirrored. Right-hand page gutter (`w:rtlGutter`) missing | 1 |
| Equations: display layout, numbering, wrapping long display equations | ✅ | #191; long display equations break at top-level operators (`m:brkBin`, `m:brkBinSub`), manual breaks and `m:alnAt`, continuation lines indented by `m:wrapIndent` or set right (`m:wrapRight`) (#326); an n-ary operator, delimiter, accent or group character the math font lacks is drawn from a fallback font instead of a missing-glyph box (#1273) | — |
| Hidden text excluded from breaking and hyphenation | ✅ | #173 | — |
| Fields: PAGE, NUMPAGES, SECTIONPAGES, TOC page numbers | ✅ | #52; totals in the body need a second pass once the page count is known (#451); SECTIONPAGES includes shared continuous-section start pages | — |
| Compatibility modes (Word 2003/2007/2010 layout for old files: `w:compat` options) | 🟡 | `allowSpaceOfSameStyleInTable` supported in modes 11/12/14; mode 15 retains contextual suppression. Most other legacy compat options remain ignored | 10–20 |
| Fields: STYLEREF in headers and footers (running chapter titles) | ✅ | Evaluated per page at layout time: the page's first (or with `\l` last) paragraph of the style, else the nearest before it; style name, id or heading level (a level finds the style named `heading N` whatever its id, so localized documents match, #1286); character styles; `\n \r \w \s \t \p` (`\r` treated as `\w`); body STYLEREF updated with the other fields (#775). Field results with right-to-left text (a Hebrew heading, "פרק א") are laid out with bidi reordering like ordinary text: the result is an isolate taking its first strong letter's direction (else the text's around the field), and each level run is shaped and mirrored in its own direction, so `{ PAGE }` in field-code view and `(Chapter 1)` keep their brackets in right-to-left paragraphs; this covers every field and note mark shown as text (#1670). Header paragraphs are cached on their per-page results, so typing in body text no STYLEREF shows doesn't lay the headers out again (#1286) | — |
| Fields: PAGE, NUMPAGES, SECTIONPAGES, TOC page numbers, PAGEREF in headers and footers | ✅ | #52; totals in the body need a second pass once the page count is known (#451); SECTIONPAGES includes shared continuous-section start pages; a PAGE field in a paragraph that runs onto a later page (a page break inside it, or a natural split) shows that page's number, the rest of the paragraph broken again for the wider result (#1352); PAGEREF in headers and footers shows its bookmark's page, filled in after pagination like their totals (#1477); in the body it keeps its result until fields are updated | — |
| Page-by-page comparison harness against Word | ❌ | Run Word locally on our own synthetic documents, export PDF, compare line breaks; Word output stays under `plan/` (never committed) | 8–12 |

## Missing black-circle glyphs

When the requested face lacks U+25CF (●), try installed Times New Roman before the UI sans
fallback, checking glyph coverage. Existing primary glyphs, bold/italic selection and fallback
when that face is absent remain unchanged. This is an observed Word for Mac compatibility
preference, not an OOXML font requirement. Synthetic font-boundary regressions cover catalog
loading, coverage, exclusion, styles and disabled system lookup. A metric control retains tall
labels and pins the 0.682 pt ascent overflow of an 11 pt Source Sans 3 label over Arial text.
Other missing characters and paragraph wrapping are outside this change.
An original embedded-font control matches native Word inline fallback to Times while preserving
a covered primary glyph. A tall present-glyph label retains a 19.628 pt pitch (Word mean 19.632 pt).
The combined numbering-definition blocks are not used to infer their individual font choices.

## Floating tables across header and body

A body floating table with `tblOverlap=never` clears floating header tables.
Allowed overlap retains its requested position. Original native controls confirm
both cases. Earlier-body reflow around late tables remains separate.

## List-label line spacing

Controlled original DOCX probes against Word for Mac 16.113.3 (26092714), macOS 26.6.2,
compatibility mode 15, zero paragraph gaps, Roboto Regular 12 pt body text: a label's positive
ascent overflow adds to the text line pitch without being multiplied by automatic spacing.
Label descent does not enlarge automatic or AtLeast spacing. A label's ascent includes positive
hhea leading. The original base split text leading across ascent/descent; current main assigns
its full gap to ascent too, so labels now use that shared metric. The table records the original
measurements, not a new native comparison of this rebase. These are
observations of synthetic documents, not a claim about Word's internal implementation.

| Label / spacing | Word baseline gap | Fixed WordCraft | Before fix |
|---|---:|---:|---:|
| Roboto 24 pt / single | 25.20 pt | 25.1953 pt | 28.125 pt |
| Roboto 24 pt / 1.25 | 28.80 pt | 28.7109 pt | 35.1563 pt |
| Roboto 24 pt / double | 39.36 pt | 39.2578 pt | 56.25 pt |
| Apple Symbols 12 pt / AtLeast 12 pt | 13.92 pt | 14.0625 pt | 14.6338 pt |
| Apple Symbols 24 pt / AtLeast 12 pt | 20.88 pt | 20.9297 pt | 24.0 pt |

Actual font metrics and original-input provenance were checked locally; Word-derived exports
remain uncommitted. Tolerance is 0.25 pt; this evidence covers Latin text with the tested symbol
and decimal labels, automatic single/1.25/double spacing, AtLeast controls, Exactly 24 pt,
wrapped paragraphs and save/reopen. Regression tests inject the independently measured metrics
so CI does not require those installed fonts. Inline pictures retain their existing height floor.
Picture bullets are not implemented, and the private Linux Symbol substitution report has not
been reproduced. Absolute baseline-leading placement remains the separate #108 contribution.
This narrows gap #4 without raising the overall readiness estimate.

## Legacy table contextual spacing

Observed in Word for Mac 16.113.3 (build 26092714) on macOS 26.6.2, using
original synthetic DOCX files, Roboto Regular 12 pt, exactly 24 pt lines, equal paragraph
styles, both paragraphs contextual, first paragraph after-space 12 pt and before-space 0 pt.
The one-cell table had no borders or cell margins. Baseline separation was 36 pt with
`w:allowSpaceOfSameStyleInTable` enabled in compatibility modes 11, 12 and 14, versus
24 pt with it disabled. Mode 15 measured 24 pt in either state. Body and header controls
in modes 14 and 15 measured 24 pt with the flag enabled. Comparison tolerance: 0.25 pt.

The model and DOCX reader/writer preserve the flag. Cell layout bypasses contextual
suppression only for enabled modes 11/12/14, including cells without table-style formatting.
Tests: `legacy_table_contextual_spacing_flag_keeps_after_space` and
`legacy_table_contextual_spacing_flag_is_scoped_to_cells_and_legacy_mode`.
WordCraft original-open and save/reopen measurements match all 12 controls; reopening
its saved DOCX files in Word retains the same gaps. Word-derived exports stay local under
ignored `plan/word/`; no Word-generated fixture is committed.

The [published option](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.allowspaceofsamestyleintable?view=openxml-3.0.1)
specifies an override inside tables. Its mode 15 exception is not explained by that source;
the guard follows this observation. [MS-DOCX compatibilityMode](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-docx/90138c4d-eb18-4edc-aa6c-dfb799cb1d0d)
allows 11, 12, 14 and 15. Mode 13 is invalid and Word rejected its synthetic input.
Ordinary overlapping before/after space and one-sided contextual spacing remain separate
work in #108 and #568. Nested tables, rotated cells, other fonts and Word for Windows
were not reference-tested here. This closes one compatibility-option gap, not table parity.

## Contextual paragraph spacing

**Observed, 2026-10-11:** original synthetic DOCX probes in Word for Mac 16.113.3
(build 26092714), compatibility mode 15, Roboto Regular 12 pt, Exactly 24 pt line
spacing. With identical paragraph style IDs and contextual spacing enabled only
on the first paragraph, 12 pt after / 8 pt before adds 0 pt to baseline separation;
10 pt after / 12 pt before adds 1.92 pt, within the predeclared 0.25 pt tolerance
of 2 pt. Body paragraphs and headers give the same results. Enabling only the
second paragraph preserves the first paragraph's 12 pt after-space.

The first-only residual agrees with the worked example in
[ISO/IEC 29500-1 §17.3.1.9](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.contextualspacing?view=openxml-3.0.1).
`contextual_after_removes_a_smaller_before_gap`,
`contextual_after_keeps_only_the_residual_before_gap`, and
`contextual_spacing_preserves_each_side_and_style_identity` pin those relative
gaps and controls in body and header layout. Different style IDs retain their
spacing even when their definitions are identical.

This fixes contextual suppression in `place_para` and `layout_box`. Ordinary
before/after overlap still needs #108; the uniform first-baseline offset, page
boundaries, tables, text boxes and notes were not compared with Word in this study.
No overall parity estimate changes from these probes.

## Performance

| Measure | WordCraft | Word | Kind |
|---|---|---|---|
| Cold layout, 188-page document | 61 ms | not measured | measured 2026-10-06 (not re-run this pass) |
| Relayout after an edit | 1.4 ms | not measured | measured 2026-10-06 |
| Font picker with many installed fonts | fixed freezes (#233) | instant | issue #31, #59 |
| Large real-world documents (500+ pages, many pictures) | not measured | | gap |
| Scrolling a 1,096-page document that is one 5,300-row table (#811) | ~2.4 ms idle, ~4.5 ms median scroll frame (was ~600 ms); opening still lays out for ~4–8 s | not measured | measured 2026-10-12, release, headless (`tests_perf::big_table_scroll_bench`) |

TOC refresh includes heading and outline paragraphs inside tables, including nested cells. Links and page numbers retain their full paragraph paths when entries move body blocks. The existing link picker still accepts top-level heading block indices.

Explicit DrawingML preset and custom outline dashes retain their stroke-width units on import
and save, in raster output and in PDF. Synthetic controls cover red rectangles, ellipses and
connectors. Custom lists are capped at 64 pairs; painted spans are bounded to 0.01–20000 pt.
Theme-inherited dash styles, line caps, ink and open freeform strokes are not covered by this change.
When content exceeds a preferred column width, AutoFit donors lose equal widths until they reach their content minima. Original synthetic Word probes cover donor reductions; cached field refresh and wrapping remain separate gaps.

Behind-text pictures in table cells participate in centre and bottom vertical alignment without increasing row flow height.

TOC refresh reuses direct tab formatting from the first cached entry with a number-separator stop at each level for numbered entries, including imported range fields. Page-number tabs still use the TOC section width. No new default number-tab position is inferred.

Explicit cell page breaks (Word for Mac 16.113.2, compatibility modes 14 and 15): leading, interior and trailing breaks take no line height. Ordinary line breaks retain their lines; body page breaks still paginate. Synthetic regressions preserve UTF-8 cluster boundaries, bookmark objects and source offsets, including body/cell cache reuse without table-style formatting. Width, border strokes and residual row metrics remain outside this change.

Footer flow-table strokes reserve their drawn top edge, including the half-width above the cell box. Original Word for Mac 16.113.2 mode-15 controls confirm the three footer cases move to the next page. The 3 pt row-end border case without a footer still differs. Tight row-end and continuation border fitting remain separate.

## Footer page tables

Page-relative footer tables keep their physical wrap bounds without adding to footer text height. Body text resumes below the table. Original native controls cover empty, text-bearing and minimum-height tables. Body textbox boundaries and nested footer painting remain separate gaps.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-17 | trivial | Font code pages select automatic leading; exact spacing keeps the font baseline. |
| 2026-10-18 | trivial | A word after a slash (" /word") wraps whole instead of splitting between two letters (#2963) |
| 2026-10-15 | trivial | An at-least table row with less room than its minimum on a page that holds something starts the next page whole, as checked in Word (#1742); the taller-than-a-page case (it then splits from the top of that page) and repeated header rows (they go with it) follow the issue and synthetic tests, not a Word check |
| 2026-10-16 | trivial | Table row keep-next uses the first paragraph of the first cell, including inherited flags. Kept row chains start on a fresh page when the following row cannot start below them. Synthetic Word export covers mixed-cell flags, multiple paragraphs and kept prefixes before oversized followers. |
| 2026-10-16 | trivial | Include table-cell headings in regenerated TOCs with full target paths |
| 2026-10-17 | trivial | Explicit DrawingML preset/custom outline dashes survive import/save and paint in raster/PDF, scaled to the effective stroke width even for zero-width paths. Synthetic controls; theme-inherited dashes and line caps remain unverified. |
| 2026-10-17 | trivial | Arial Unicode MS uses Arial and its proportional substitutes; glyph coverage and exact Word line height remain font-dependent. |
| 2026-10-16 | trivial | Exact OpenType full names (name ID4) select installed static faces after family names; variable-instance aliases remain unsupported |
| 2026-10-16 | trivial | Symbol (PCL6) uses the existing Symbol mapping and Symbol face/fallback despite stale Roman font metadata; stored codes/fonts stay unchanged |
| 2026-10-16 | trivial | Prefer a covering Times New Roman face for an uncovered black-circle glyph; retain normal label-ascent and other fallback rules |
| 2026-10-15 | trivial | The caret after spaces hanging past the right margin stays at the margin instead of running off the page (#2146) |
| 2026-10-15 | trivial | A missing Thai document font is substituted by one Thai face for the whole run, Latin letters and digits included (#1549) |
| 2026-10-15 | trivial | A list bullet no installed font can draw (a private-use code) shows the level's default bullet instead of an icon from an unrelated font (#1683) |
| 2026-10-16 | trivial | Field results (STYLEREF, REF, PAGE text…) with right-to-left text are reordered like ordinary text, so a Hebrew heading in a header reads correctly (#1670) |
| 2026-10-16 | trivial | Positional tabs (`w:ptab`) are laid out at the margins or indents they name, not at the next tab stop (#2367) |
| 2026-10-16 | trivial | Hint-reliant CJK fonts execute TrueType contour assembly; Windows DFKai-SB no longer loses strokes. Synthetic local-font regression; no font binaries bundled. |
| 2026-10-16 | trivial | `w:autoSpaceDE` and `w:autoSpaceDN` are laid out: a quarter em between Asian text and Latin text or digits (#2358) |
| 2026-10-16 | trivial | AutoFit donors lose equal widths, bounded by content minima |
| 2026-10-16 | trivial | Behind-text pictures in table cells participate in centre and bottom vertical alignment without increasing row flow height. |
| 2026-10-15 | trivial | Absent or zero DrawingML effect extents remain zero on rotated pictures; contracted axes no longer reserve invented effects room. Synthetic quarter-turn import regression (`zero_effects_stay_zero_rotated`). |
| 2026-10-15 | trivial | Additive paragraph spacing retains the full before-space after a section page break; ordinary overlapping spacing is unchanged. |
| 2026-10-15 | trivial | Removed the 12 pt line-width floor in narrow cells and float spans; synthetic wrapping and oversized-glyph controls |
| 2026-10-15 | trivial | Partial cell margins retain omitted edges through DOCX save and inherit current table defaults during layout. Original import/write and row-height regressions. |
| 2026-10-15 | trivial | Exact line spacing keeps the run font baseline around tall equations; automatic and minimum spacing still grow for math. |
| 2026-10-16 | trivial | Zero-extent drawing anchors contribute no inline font height. Synthetic regressions. |
| 2026-10-16 | trivial | Preserve direct TOC number tabs during regeneration |
| 2026-10-16 | trivial | Explicit cell page breaks take no line height; original native Word controls in modes 14 and 15 and synthetic cluster/cache regressions. |
| 2026-10-16 | trivial | Flow-table top border strokes in footers reserve body clearance; original mode-15 Word controls and a synthetic layout regression. |
| 2026-10-16 | trivial | Subpoint grid units retain their declared widths without shrinking adjacent cells; tiny totals normalize without scale-factor overflow |
| 2026-10-18 | trivial | Any VML float in a header or footer cell is placed on the page, as Word for Mac does (user-checked, #1775) |
| 2026-10-16 | trivial | A header or footer cell's float placed relative to the page and not laid out in its cell (a picture watermark in a header that starts with a table) is placed on the page, behind the text (#1775) |
| 2026-10-17 | minor | Drop caps: In margin (letter in the left margin, text not indented), distance from text and the letter's font, round-tripped as `w:framePr` (#2601) |
| 2026-10-16 | trivial | An otherwise empty section at page start retains its line before continuous content; later closing marks stay quiet |
| 2026-10-16 | trivial | Math operators, delimiters and accents missing from the math font use a fallback font (#1273) |
| 2026-10-17 | trivial | SECTIONPAGES includes shared continuous-section pages in body and header/footer fields |
| 2026-10-17 | trivial | Footer page tables preserve text height and wrap intersecting body lines |
| 2026-10-17 | trivial | Non-overlapping body floating tables clear header tables; overlapping tables keep their requested position |
| 2026-10-16 | trivial | Synthetic regression confirms a fitting exact-height row containing overflowing pictures moves to the next page whole. Existing runtime behavior is unchanged. |
| 2026-10-15 | trivial | The caret, clicks and selection edges fall between the letters of a ligature (#1974) |
| 2026-10-14 | trivial | Row table border exceptions draw without leaking into other rows |
| 2026-10-14 | trivial | Consistent cell widths override a stale table grid |
| 2026-10-14 | trivial | Paragraphs inside complex-field instructions take no flow height; empty result paragraphs keep their line. |
| 2026-10-14 | trivial | A list paragraph containing only a page break has no visible label; it still advances the list counter. Ordinary empty list items keep their label. |
| 2026-10-14 | trivial | With `w:doNotUseHTMLParagraphAutoSpacing` the space between two paragraphs is the first's space after plus the second's space before, as measured in Word; without it, still the larger (#733) |
| 2026-10-14 | trivial | An inherited header or footer sizes its percentage-width tables by the section that defines it, as measured in Word (a portrait footer table keeps its width on inheriting landscape pages) |
| 2026-10-14 | trivial | An empty paragraph closing a section before a new page stays on a full page instead of starting a page of its own, as measured in Word |
| 2026-10-14 | trivial | A table row with an at-least height doesn't split where less than that height is left on the page; it starts the next page whole, as measured in Word |
| 2026-10-14 | trivial | Text that doesn't fit an exact-height table row is cut off at the cell's edge, as in Word, instead of drawn over the rows below |
| 2026-10-14 | trivial | A numbered paragraph whose text opens with a page break draws its number on the next page with the text, as Word does |
| 2026-10-14 | trivial | A bookmark (or other zero-width marker) after a paragraph's closing page break stays with the break instead of taking a line at the top of the next page |
| 2026-10-14 | trivial | A paragraph of only a page break followed by a page break before, or carrying a next-page section break, starts one page, not two: the break's mark and its space after stay on the page before, as in Word |
| 2026-10-13 | trivial | Justified vertical page alignment (#1320): the page's spare room goes equally between its blocks; a section's last page takes its own section's vertical alignment, not the next one's |
| 2026-10-13 | trivial | Floats and floating tables aligned inside/outside, or placed in the inside/outside margin, swap sides on even pages; with mirrored margins, page-relative floats keep their place and left/right margin areas are each page's own (#1181) |
| 2026-10-14 | trivial | Widow control: a three-line paragraph with room for two of its lines at the bottom of a page moves whole, as in Word (no two-and-one split leaving a widow); a paragraph continued from the previous page may still take one line along from two |
| 2026-10-13 | trivial | PAGEREF in headers and footers shows its bookmark's page (#1477) |
| 2026-10-14 | trivial | All Caps and Small Caps draw ß as SS, as Word does, kept one character for the caret and selection (#1581) |
| 2026-10-14 | trivial | A word too wide for the strip beside a square-wrapped object moves past it whole instead of being split across the strip (#1376) |
| 2026-10-14 | trivial | Russian letter list formats (`russianLower`, `russianUpper`) draw а) б) в) … аа) бб) as Word does, instead of digits |
| 2026-10-14 | trivial | A table's percentage width past 100% is kept, so a table pulled into the margins spans them as in Word |
| 2026-10-14 | trivial | A section can restart its page numbers at 0 (`w:pgNumType w:start="0"`), as a cover page numbered 0 does; the restart was read but laid out from 1 |
| 2026-10-13 | trivial | A shape or picture dragged into another column anchors there, so the text in that column wraps around it (#884) |
| 2026-10-13 | trivial | On a page with footnotes, a paragraph that ends there keeps its space after above the footnote separator; if it doesn't fit, its last line (and, with widow control, one more) goes to the next page with the note. Pages without notes are unchanged (#1116) |
| 2026-10-12 | trivial | STYLEREF fields in headers and footers show each page's chapter (or other styled text); Insert › Quick Parts › Field offers STYLEREF (#775) |
| 2026-10-12 | trivial | a list label moves with its first line when the paragraph is centred or end-aligned (#1159) |
| 2026-10-13 | trivial | A paragraph ending with a page break keeps its space after on the old page with its mark: the next page starts with the next paragraph at the top (it started that much too low, and a page break before after it opened a blank page) |
| 2026-10-13 | trivial | Table rows: every cell gets the row's largest top and bottom margin; exact rows add the bottom margin on top and never split; a split row keeps its cells' bottom margin, border and space after on the first page, applies widow control in its cells and continues each cell at the top of the next page |
| 2026-10-14 | trivial | A paragraph continued into the next column or page is broken again around the shapes there, not the ones where it started (#1374) |
| 2026-10-13 | trivial | PAGE fields after a page break inside their paragraph, or on a page the paragraph runs onto, show that page's number (#1352) |
| 2026-10-14 | trivial | Colour emoji row set to 🟡: CBDT verified on Noto Color Emoji; COLR v0/v1 and sbix not yet seen on a real font (#1269) |
| 2026-10-14 | trivial | Colour emoji and colour-font glyphs (COLR, sbix, CBDT) drawn on screen and in page renders (#1269) |
| 2026-10-12 | trivial | Document grid: the stale ❌ row removed; the 🟡 row (#391, #401) is the only one |
| 2026-10-12 | trivial | Spaces, tabs and ideographic spaces don't count toward a line's height, as in Word (a line of only them takes the paragraph mark's) |
| 2026-10-12 | trivial | A list label's font raises the first line's ascent but never its descent (Word keeps the text's descent below the baseline) |
| 2026-10-12 | trivial | Hidden end-of-cell mark (`w:hideMark`): a cell's empty last paragraph takes no room, spacing and all; a row of only such empty cells is just its borders |
| 2026-10-12 | trivial | Text frames in headers, footers, cells and text boxes are placed like floating objects with the text wrapping around them; body frames still lay out in line |
| 2026-10-16 | trivial | Framed paragraphs inside a text box (a frame from their style, usually) run with the box's text as in Word, instead of being placed apart on top of each other (#2357) |
| 2026-10-12 | trivial | List indents and style indents combine like Word's: a numbered style's own indents (and those of styles based on it) beat the list level's, its base styles' don't, the paragraph's own list beats every style; a paragraph that switches its list off (`numId` 0) drops the styles' indents |
| 2026-10-12 | trivial | Keep with next: a paragraph that doesn't fit on the page anyway breaks as usual (its last lines go over with the next paragraph) instead of moving whole; a chain of kept paragraphs moves together |
| 2026-10-12 | trivial | Continuous section breaks: an empty paragraph carrying only the break takes no room (text after it follows on; balanced columns ignore it) |
| 2026-10-12 | trivial | Floats relative to a paragraph (or its line) are measured from the paragraph's start above its space before |
| 2026-10-12 | trivial | Text after a column break in unequal columns is broken for its new column's width |
| 2026-10-12 | trivial | Space before at the top of a page after page breaks, measured in Word for Windows in compatibility modes 12 and 15 (32 cases) |
| 2026-10-12 | trivial | No automatic (HTML-style) spacing between two list items, of one list or two, at any level; the first item keeps its automatic space before and the last its space after |
| 2026-10-12 | trivial | Kerning only where the document asks for it, and automatic (HTML-style) paragraph spacing, both measured in Word for Windows against real .gov documents |
| 2026-10-12 | trivial | A section break inserted over a selection keeps the selected text: the first section no longer comes out empty (#852) |
| 2026-10-12 | trivial | Large-table scrolling (#811): per-frame page fingerprinting fixed; measurement added |
| 2026-10-12 | trivial | Symbol-font text (Wingdings, Wingdings 2/3, Webdings, Symbol) drawn as Unicode symbols (#776) |
| 2026-10-14 | trivial | Floating-table overlap checks use visible line content; empty line-break lines no longer move a table |
| 2026-10-11 | trivial | Chinese documents (#534): indents in characters, a page-anchored floating table after a full page moves to the next page, no blank page from a page break at the bottom of a full page |
| 2026-10-11 | trivial | East Asian list number formats (一、二、三, 第一章, ①); list labels whose characters come from two fonts draw each part with its own font (the digits of "1、" no longer show as other letters); a list level without indents leaves the label at the paragraph edge (#534) |
| 2026-10-11 | trivial | Vertical sections: `w:textDirection` in `w:sectPr`, upright East Asian glyphs (#491) |
| 2026-10-11 | trivial | Mirror indents (`w:mirrorIndents`): left/right indents swap on even pages; suppress line numbers inherited from styles (#512) |
| 2026-10-11 | trivial | NUMPAGES/SECTIONPAGES in the body show the real totals (the body is laid out again once the page count is known, at most three extra passes, #451); footnotes referenced in table cells go to the page bottom (#450); PDF links to a bookmark go to the page its start is on, not its paragraph's first page (#455) |
| 2026-10-11 | minor | Complex scripts: script itemisation, OpenType fallback fonts, syllable clusters, Myanmar syllable breaks (#604) |
| 2026-10-11 | trivial | Mirrored margins on even pages, tall and linked footers/headers push the body, long endnotes continue, caret in continued notes (#452, #453, #454, #456, #457) |
| 2026-10-11 | trivial | Hyphenation zone, words in capitals and consecutive-hyphen limit follow the document settings; Manual Hyphenation (#407) |
| 2026-10-11 | trivial | Document grid: line and character pitch (#391) |
| 2026-10-12 | trivial | Text boxes use their own internal margins; Resize shape to fit text never cuts text (#743) |
| 2026-10-11 | trivial | Linked text boxes, Align Text and text-box text direction (#369) |
| 2026-10-11 | trivial | Right-to-left tables and sections laid out mirrored (#362) |
| 2026-10-10 | trivial | The document's own note separators and continuation notice are drawn (#352) |
| 2026-10-11 | trivial | Newspaper columns balance before a continuous section break (#350) |
| 2026-10-12 | minor | Measured synthetic list-label line height; automatic and AtLeast spacing keep text descent and unscaled label ascent overflow |
| 2026-10-12 | trivial | Legacy table contextual spacing measured and supported; modes 11/12/14 versus 15 checked with original synthetic documents |
| 2026-10-11 | minor | Body/header contextual spacing corrected from original Word probes; ordinary overlap remains #108 |
| 2026-10-11 | trivial | Group and Ungroup keep turned members' orientation and placement (#441, #443); Size resizes a group (#442); the eraser hits turned ink where it is drawn (#445) |
| 2026-10-10 | trivial | Long footnotes continue onto the next page with a continuation separator (#352) |
| 2026-10-11 | trivial | `EQ \o` fields overstrike their arguments; enclosed characters draw their circle, square, triangle or diamond as vectors (#297) |
| 2026-10-11 | trivial | Ruby (Phonetic Guide) layout landed (#288) |
| 2026-10-10 | trivial | Wrap side: text on both sides, left only, right only or the largest side of an object (#354) |
| 2026-10-10 | trivial | Tight and Through wrapping follow the object's outline (#354) |
| 2026-10-10 | trivial | Rotated and flipped objects laid out and drawn (#332) |
| 2026-10-11 | trivial | Footnotes below text, endnotes at the end of each section, note numbering restarted per section or page (#385) |
| 2026-10-10 | trivial | Long display equations wrap across lines (#326) |
| 2026-10-10 | trivial | AutoFit Contents measures the text (#44) |
| 2026-10-10 | major | First version: layout and pagination checklist with evidence from `crates/layout`, PRs and issues |
| 2026-10-16 | trivial | OpenType ligatures follow `w14:ligatures` (none, standard, contextual, historical, discretional and their combinations); without it WordCraft's default stays |

Maintenance on 2026-10-12 retains current tracked-deletion, endnote, footer-total, text-box
and balloon layout while supplying explicit cell identity at every box call. The native Word
probes above are historical; this rebase has no new native or other-platform measurement.

Repeated table-cell control, 12 October 2026: four same-style contextual paragraphs with 6 pt before/after and exactly 24 pt lines retain zero extra gap at every boundary. The body control uses the same fixture. This is an original specification-based regression; no new native Word table comparison was run.
