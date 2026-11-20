# Hangul document compatibility

What is kept and what is lost when WordCraft opens, edits and saves `.hwp` / `.hwpx` (as of
2026-10-31). The evidence is the code (`crates/hwp`) and tests that opened the documents of the test
set at the time, 30 documents (13 HWPX, 17 HWP), in Hancom's viewer. The test set has changed since
(see "Test set" in [cleanroom.md](cleanroom.md), section 3); items checked only on documents no longer
in it are listed there as needing a new check.

## Key

| Mark | Meaning |
| --- | --- |
| ✅ | Works (checked on documents and in Hancom's viewer) |
| ⚠️ | Works in part (see notes) |
| ❌ | Doesn't work. An edit of it is left out or left as it was, and the save lists it (`notWritten`). Something reading drops is gone from the document: a new file doesn't have it, and nothing tells the user (the rows say so) |
| ➖ | Not applicable |
| ❔ | Handled in the code but not checked separately. Needs a check |

Columns:

- **Open**: whether it shows when a Hangul document is opened in WordCraft
- **Save to original**: whether an edit is written when a Hangul document is edited and saved in the
  same format (only the edited parts are written into the original)
- **New**: whether it is written when a new document, a `.docx` (or a document opened from another
  format) is saved in a Hangul format

When saving into the original, **everything not edited stays as in the original**. So elements
WordCraft doesn't handle are not lost as long as they aren't touched. Some edits are
exceptions, and the save lists what they lose (`notWritten`): editing the text of an HWP table
cell that holds a field drops the field (section 3), and an edit across an object or one of HWP's
invisible inline controls is not written (section 1).

## 1. Text and paragraphs

| Feature | Open HWPX | Open HWP | Save to original HWPX | Save to original HWP | New HWPX | New HWP | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Typing, editing, deleting text | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | |
| Adding, deleting, joining paragraphs | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Paragraphs with section settings or memos are not deleted (the user is told) |
| Tabs, line breaks, non-breaking and fixed-width spaces | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | |
| Editing text between pictures, tables and fields | ➖ | ➖ | ⚠️ | ⚠️ | ➖ | ➖ | Works when the edit is within one piece. Edits across an object, and text between HWP's invisible inline controls, are not written. In HWPX, page controls in the text's run (page hiding, page numbers, new number, columns) are kept in a run of their own after the text |
| Font, size, bold, italic, underline, strikethrough, colour, shading | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | |
| Superscript, subscript | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | |
| Width ratio, letter spacing | ❔ | ❔ | ❌ | ❌ | ✅ | ✅ | Edited values are not written into the original |
| Outline, shadow, emboss, engrave | ❔ | ❔ | ❌ | ❌ | ✅ | ✅ | |
| Highlighter (markpen) | ❌ | ❌ | ➖ | ➖ | ❌ | ❌ | Lost when reading, without a notice. Saving into the original keeps it; a new .hwpx or .hwp made from the document has none |
| Hancom fonts that aren't installed | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Drawn with an installed font of the same kind (Myeongjo: Nanum Myeongjo first; Gothic: NanumGothic, else Pretendard), saved with the original name. Without them, the user is told where to get them. A new HWPX or HWP writes Hangul that names no East Asian font in the system's Hangul font (Malgun Gothic on Windows, for example) |
| Alignment, margins, indents, paragraph spacing, line spacing | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | |
| Page and column breaks | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ | Breaks newly added to an original are not written |
| Paragraph borders and shading | ❔ | ❔ | ❌ | ❌ | ❌ | ❌ | |
| Custom tab stops | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | New files get default tabs only |
| Numbered and bulleted lists | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ | Starting or ending a list in an original is not written |
| Outline numbering | ✅ | ✅ | ❌ | ❌ | ⚠️ | ⚠️ | New files write it as a numbered list |
| Paragraph styles (names) | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | Changing a style in an original changes only the formatting; the style name stays. Character styles are lost in new files |
| Vertical writing | ✅ | ✅ | ➖ | ➖ | ❌ | ❌ | |

## 2. Pages

| Feature | Open HWPX | Open HWP | Save to original HWPX | Save to original HWP | New HWPX | New HWP | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Paper size, orientation, margins | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ | Page settings changed in an original are not written |
| Columns | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ | Including a change of column count within a page |
| Header and footer text | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | In an original, text edits only |
| Adding or removing headers and footers | ➖ | ➖ | ❌ | ❌ | ✅ | ✅ | |
| Different odd and even page headers | ❔ | ❔ | ➖ | ➖ | ✅ | ✅ | |
| Different first page header | ❔ | ❔ | ➖ | ➖ | ⚠️ | ⚠️ | Only hiding on the first page works. First-page-only content is lost: Hangul has no such feature |
| Page numbers | ✅ | ✅ | ➖ | ➖ | ✅ | ✅ | |
| Footnotes and endnotes | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | In an original, editing note text only; adding new notes doesn't work |
| Master pages | ⚠️ | ⚠️ | ➖ | ➖ | ❌ | ❌ | Imitated with the header when reading |
| Page borders | ⚠️ | ⚠️ | ➖ | ➖ | ❌ | ❌ | A body-relative border is imitated with a rectangle in the header |

## 3. Tables

| Feature | Open HWPX | Open HWP | Save to original HWPX | Save to original HWP | New HWPX | New HWP | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Showing tables (merged cells, borders, fill, cell margins, vertical alignment) | ✅ | ✅ | ➖ | ➖ | ✅ | ✅ | |
| Editing text in cells | ➖ | ➖ | ✅ | ✅ | ➖ | ➖ | |
| Adding and deleting rows and columns, merging cells, changing size, borders and fill | ➖ | ➖ | ✅ | ✅ | ➖ | ➖ | Only changed tables are rewritten. HWP: cells holding fields, pictures, form objects or nested tables whose content is unchanged are written from their original records, keeping them, and the table caption is kept; other cells are written again. Numbers and bullets in cells written again are left out (the user is told). Editing the text of a cell with a field drops the field (the user is told); editing the text of a cell with a picture leaves the table as it was (the user is told) |
| Inserting new tables | ➖ | ➖ | ✅ | ✅ | ✅ | ✅ | |
| Deleting tables | ➖ | ➖ | ✅ | ✅ | ➖ | ➖ | |
| Nested tables | ✅ | ✅ | ❔ | ❔ | ✅ | ✅ | |
| Tables floating over or behind text | ⚠️ | ⚠️ | ➖ | ➖ | ⚠️ | ⚠️ | Text is pushed away differently from Hangul, so the position may differ a little |
| Cell diagonals | ⚠️ | ⚠️ | ➖ | ➖ | ➖ | ➖ | WordCraft has no diagonals (kept in the original when saving into it) |
| Cell zone (cellzone) borders and fill | ⚠️ | ⚠️ | ➖ | ➖ | ➖ | ➖ | Only the borders and fills the zone sets are laid over the cells |

## 4. Pictures and shapes

| Feature | Open HWPX | Open HWP | Save to original HWPX | Save to original HWP | New HWPX | New HWP | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Showing pictures | ✅ | ✅ | ➖ | ➖ | ✅ | ✅ | |
| Inserting, replacing, resizing, deleting pictures | ➖ | ➖ | ✅ | ✅ | ✅ | ✅ | HWPX: written in original paragraphs and new body paragraphs (picture files added to `BinData/` and the list). A picture inserted in the middle of text is put at the paragraph's end, and the user is told. HWP: written in new paragraphs and original ones (including table cells and paragraphs with section settings or fields; a BIN_DATA is added to DocInfo). In original paragraphs the remaining objects keep their records and only the added or removed objects change. Deleting a picture so that text pieces join, and inserting in the middle of text, also work. A changed picture in a paragraph with a table is reported |
| Cropping pictures | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ | |
| Rotating and flipping pictures | ❔ | ❔ | ❌ | ❌ | ❔ | ❔ | |
| Text boxes | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | Original: editing text inside, inserting new text boxes, deleting and resizing existing shapes. HWP also works in paragraphs with other controls (section settings, fields, pictures), and a resized shape has only the size and scale of its original records changed. A changed shape in a paragraph with a table is reported |
| Rectangles, rounded rectangles, ellipses, lines | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | Inserting into an original on the same terms as text boxes |
| Triangles, diamonds, arrows, stars, hearts | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ✅ | Written as polygons. Inserting into an original on the same terms as text boxes |
| Connectors | ❔ | ❔ | ❌ | ❌ | ⚠️ | ⚠️ | Written as straight lines |
| Polygons, curves, arcs | ✅ | ✅ | ❌ | ❌ | ➖ | ➖ | |
| Pen, pencil and highlighter ink (freehand) | ➖ | ➖ | ❌ | ❌ | ✅ | ✅ | New HWPX: open paths as `hp:curve` (straight segments), closed paths as `hp:polygon` |
| Groups | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ | New HWPX: `hp:container` (groups in groups, pictures included). A resized group is written at scale 1, with the scale multiplied into its children's size and position |
| Charts, SmartArt | ❔ | ❔ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | Written as a picture of what WordCraft draws (PNG, 2 pixels per pt). Can't be edited as a chart in Hangul. Written into an original HWP on the same terms as pictures |
| Text art, OLE objects | ❔ | ❔ | ❌ | ❌ | ❌ | ❌ | |
| Equations | ⚠️ | ⚠️ | ❌ | ❌ | ❔ | ❔ | Shown as the equation's text when reading |

## 5. Fields, memos and other

| Feature | Open HWPX | Open HWP | Save to original HWPX | Save to original HWP | New HWPX | New HWP | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Hyperlinks | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | Links newly added in an original are not written. New files keep the linked text without the destination (the user is told) |
| Input fields (누름틀) | ⚠️ | ⚠️ | ❌ | ❌ | ➖ | ➖ | Shown as their guide text. Editing paragraphs with them is limited |
| Bookmarks | ❔ | ❔ | ❌ | ❌ | ❔ | ❔ | |
| Memos | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ | Read with author, time and text. Saving into the original: memos added or deleted are not written (the save lists them; a deleted memo stays as it was). New HWPX and HWP: written as Hangul memos (HWP as a `%unk` field and a MEMO_LIST at the section's end). Ranges across paragraphs kept; replies go inside the original memo with "↳ author". Memos in notes and headers are written too (WordCraft reads them back; Hancom's viewer shows only body memos as balloons) |
| Track changes | ⚠️ | ⚠️ | ❌ | ❌ | ❌ | ❌ | Deletion marks are treated as deleted text |
| Form objects (buttons, input boxes and so on) | ⚠️ | ⚠️ | ❌ | ❌ | ➖ | ➖ | |

## 6. Files

| Item | Status | Notes |
| --- | --- | --- |
| HWP 5.0 (`.hwp`), HWPX (`.hwpx`) | ✅ | |
| Password-protected and distribution documents | ❌ | The user is told on open |
| HWP 3.0 and earlier, HWPML (`.hml`) | ❌ | |
| Page counts (against Hancom's viewer) | ⚠️ | On the 30-document set: HWP 14 of 17 (4 pages of difference in total), HWPX 5 of 13 (11 pages). Space width, minimum space, `CHAR` margins and Korean word-unit line breaking (`KEEP_WORD`) are handled as in Hangul. Most remaining differences come from Hancom fonts that aren't installed: the two programs use different stand-ins, and Hancom's are 6 to 9% wider. With the same font (Hamchorom Batang) installed, even line breaks match. On the current 20-document set (2026-11-17): HWP 8 of 10, HWPX 9 of 10; two differ by one page, one long HWP document by 5 |
| Opening in the web build (browser) | ⚠️ | A browser can't use installed fonts directly, and the web build bundles no Korean font, so Korean shows as boxes until installed fonts are loaded. Opening a Hangul document shows a bar below the ribbon offering to load them (the browser asks for permission; once allowed, installed fonts are used; checked 2026-11-03 in Chromium-based browsers) |
| Saving unedited | ✅ | On the 30-document set, and again on the current 20 (2026-11-16): every HWP is byte-identical to the original; for every HWPX each file in the package has the same content (only the ZIP entries' time stamps are new) |

## When checking

- ❔ items are features no test document had, so they weren't seen. Put such a document in
  `samples/` and check.
- Edits that couldn't be written on save are counted in the status bar and described in `notWritten`
  of the `file.save` result.
