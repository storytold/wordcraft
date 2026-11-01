# Page-label segments

> **Last updated:** 2026-10-31 · External-edit protection and table-boundary guidance.

Insert › Header & Footer › Page Number › **Format Page Numbers** opens the page-number settings.
The segment selector at the top selects the section to edit. **+ New segment…**, below the
numbering settings, creates a segment beginning at the current document page or at the body cursor
on a new page. It ends at the next existing section boundary or the end of the document. An
existing boundary is edited rather than duplicated.
For tables, place a body cursor before or after the table; a segment cannot split a table.
The dialog explains an invalid boundary and disables Create segment until a valid start is selected.

Enable **Custom page labels** to give a segment a name and select its label:

| Mode | Example |
|---|---|
| No label | An unnumbered cover |
| Text only | `Appendix` on every page |
| Counter only | `0, 1, 2`, `I, II, III` or `A, B, C` |
| Text and counter | `TOC I`, `Annex 0` or `Appendix A` |

The name in the selector is independent of the printed text. Text is literal, and the separator
is editable. Roman and alphabetic formats support both cases. Choose **Start at** 0, 1 or another
nonnegative value up to 100,000, or continue the previous numbered segment. Text-only and hidden
segments do not advance that sequence. Changing the format retains the numeric value when
continuing. A hidden page label inside a numbered section is distinct from a segment with no counter.

Roman and alphabetic formats have no zero symbol: at start 0 the **Zero-page label** is the entire
first label. Leave it empty for no label, or enter text such as `Opening`; the following pages use
I, II or A, B. The preview shows three labels. Editing the form does not change the document until
**OK** or **Create segment**. **Cancel** discards the draft; the applied change is undoable.

Existing header/footer text is retained. Segment changes copy the effective header/footer stories
so their editing does not overwrite earlier or later linked sections. A PAGE field is added to an
active footer slot if neither its header nor footer contains one. Existing first/even-page settings
are respected. Turning off **Custom page labels** restores the section's ordinary number format.

For a book, configure the cover as No label, contents as Text and counter (`TOC`, upper Roman,
start 1), preface as Counter only (letters, start 1), and main text as Counter only (decimal, start 1).
Adding front matter does not alter the main text's explicit restart.

Update the table of contents and other generated lists after changing the document. Their page
references use complete labels; references to an empty label use its segment name. Repeated labels
do not change bookmark destinations. Physical page positions and the document's total page count
are retained for navigation and printing.

JSON and DOCX retain segment settings. DOCX contains ordinary text, PAGE and IF field fallbacks.
It also stores segment rules in standard custom document properties, referenced by short tags on
the generated content controls. WordCraft restores the editable PAGE fields from these records,
even without its ignorable extension attributes. The records follow their header/footer through
section reordering and inheritance; long Unicode names and text are split into bounded properties.
Synthetic tests cover this recovery, repeated saves and preservation of unrelated document properties.
In a user-assisted Windows Word test (2026-10-30), added page breaks and text survived saving in
Word and reopening in WordCraft. Word removed the ignorable attributes but retained all 13 backup
properties unchanged. All six segment rules, including the long Unicode name, were restored:
an unlabelled cover, three `Vorwort` pages, `TOC I–III`, `A–C`, `1–3` and `Annex 0–2`.
The first contents page rendered `TOC I` rather than Word's cached `TOC III`; another DOCX
save/reopen retained every section setting. The file has 16 pages in both applications after fixing
an extra blank page caused by a manual page break followed by an empty paragraph carrying a
next-page section break. This verifies this synthetic case, not general layout parity.
Keep the WordCraft source: removing the custom properties or generated content controls loses the
recovery information. New exports fingerprint generated literal labels and field codes. If these are edited in another
editor, WordCraft opens the entire document using its ordinary Word text/fields and skips automatic
segment-rule recovery, preserving sibling labels as well as the edit. Reconfigure custom segments
in WordCraft if needed. Cached field-result updates, run splitting and formatting are excluded from
the comparison; cached-result edits cannot be distinguished from normal field updates. Older exports
without fingerprints retain the previous recovery behavior. This is conflict preservation, not
synchronization of Word edits into segment rules. Other editors remain unverified.
Continuation that skips non-counting segments is WordCraft-specific.
PDF output and its viewer page labels use the selected labels, including labels starting at zero.
Other export formats do not currently preserve the custom segment metadata.

The new interface strings use the translation catalog with English fallback. See the
[German instructions](page-label-segments-de.md).

Programmatic equivalent: `layout.pageSegment`, for example

```json
{"new":true,"from":"cursor","label":{"name":"Appendix","mode":"textNumber","text":"Annex","separator":" ","zeroLabel":""},"format":"upperLetter","start":1}
```

Without `new`, apply to the current section, or specify its zero-based `section` index. Omitted
`format` and `start` preserve the section's existing settings; an explicit `start:null`
continues numbering. `label:null` restores ordinary page labels. Names, text and the zero-page label
are limited to 256 characters, separators to 16; control characters are rejected.

To edit a header or footer independently, enter it on the desired page and toggle **Insert ›
Header & Footer › Link to Previous** off. Its content is copied, including anchored text boxes;
other sections retain their content. Toggle it on to use the preceding section again. The control
applies to the current header/footer variant (default, first or even page), is unavailable in the
first section, and supports Undo. An intentionally empty unlinked header/footer remains empty
when you close it or save. A dedicated Header & Footer contextual tab is not added here.

## Revision history

| Date | Change |
|---|---|
| 2026-10-31 | Documented guarded exports, conservative conflict import and table-boundary guidance. |
