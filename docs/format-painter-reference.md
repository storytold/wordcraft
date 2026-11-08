# Format Painter paragraph selection rules

Measured on 2026-11-08 in **Word for Mac 16.114.4 (26110224)**, French UI,
using a locally authored synthetic RTF. Source: Arial 12, bold, centered,
0.5-inch left indent, 12-point space before. Targets: left aligned, no indent
or space before. No issue attachments or external reproduction documents used.
These observations establish Mac behavior only.

Native Copy/Paste Formatting used Cmd+Option+C / Cmd+Option+V. The toolbar
Format Painter was tested separately with a mouse drag. Word's tooltip identifies
Cmd+Shift+V as paste text only; shortcut/buffer changes belong to #5616, separately
from the paragraph-selection rules in #5964.

| Source | Destination | Native Paste Formatting alignment | Toolbar brush alignment |
| --- | --- | --- | --- |
| Partial word | First word, middle word, caret, whole paragraph, cross-paragraph range | Preserved | Preserved (first-word drag tested) |
| Whole paragraph text | Whole paragraph | Copied | — |
| Whole paragraph text without mark | First word | — | Copied |
| Whole paragraph including mark | First or middle word | Preserved | Copied |
| Whole paragraph including mark | Caret at start or inside word | Copied | — |
| Whole paragraph including mark | Whole paragraph text, with or without mark | Copied | — |
| Whole paragraph including mark | Cross-paragraph partial-to-partial range | Copied to both touched paragraphs | — |
| Caret inside source word | Whole paragraph including mark | Copied | — |

A whole paragraph's text suffices; explicitly selecting its paragraph mark is
not necessary. Starting a shorter selection at offset zero does not suffice.
The engine regression matrix covers partial/whole source selections against
first-word, middle-word, and whole-paragraph targets for both commands. Its
`mark=true` cases represent a selection ending at offset zero of the next
paragraph; they do not exercise a real paragraph-mark selection. The matrix rows
above report native observations, including the rows with and without the mark;
the synthetic cases do not establish UI paragraph-mark selection coverage.
Additional caret and cross-paragraph tests cover the native destination rules.

Scope: the tests dispatch engine commands directly. Existing UI coupling between
Copy Formatting and the armed brush, copied-buffer lifetime, and platform shortcut
bindings remain separate work in #5616; this change does not establish complete
native keyboard UI parity.
