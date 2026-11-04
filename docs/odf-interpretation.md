# How WordCraft interprets OpenDocument files

> **Last reviewed:** 2026-11-02 · **Last updated:** 2026-11-04 · **Change:** major (no more settings: one choice for each case the standard leaves open); previously trivial (what is written as read: unchanged paragraphs with drawings, comments, changes or marks too); previously minor (the two presets are Microsoft Word style and LibreOffice style; CLI values listed one by one); previously minor (deviations from the standard listed; ODF 1.2 → 1.3 checked); previously major (new: the ODF interpretation profile and the list of every case)

OpenDocument (ODF 1.3, OASIS) leaves some of a document's look to the application. Part 3 §16.2
says that when no style sets a property and the standard gives no default, "an
implementation-dependent value is used". So a file can look different in WordCraft, LibreOffice,
Google Docs and ONLYOFFICE without anyone being wrong. This page lists every such case WordCraft
knows of, so none gets lost.

## The rule

1. **What the standard defines is law.** WordCraft follows it. That includes `fo:` properties,
   which the standard defines by referring to XSL 1.0, for their meaning. Whether such a reference
   also brings XSL's initial value as the default the standard doesn't say; where an application's
   choice differs from XSL's (page margins), the case is listed below, with XSL's value as
   WordCraft's.
2. **What the standard leaves open, WordCraft decides once.** Each case listed below has one
   choice, the one WordCraft makes for its own and Word documents, and no setting. LibreOffice's
   choice (and Collabora's, which shares its engine) is listed beside it, measured black-box in
   LibreOffice 24 where it isn't documented, because a file made there can look different here.
3. **A value the file states is the file's.** WordCraft's choices fill in only what the file (its
   styles, defaults and `settings.xml`) leaves unsaid. Three cases are not missing values but
   stated ones whose meaning the standard leaves open: how a header's stated height and spacing
   combine, what a stated width of 0 draws at, and a frame position stated against something the
   standard doesn't define for its anchor. They are marked *(meaning)* below.
   `formats::odt::tests_more::unstated` checks that stated values are kept.
4. **A deviation from the standard is made only out of necessity**: where the files in
   circulation contradict it and following it would damage every such file. Each one is listed
   under "Deviations from the standard, out of necessity" below, with the standard's rule, what
   WordCraft does instead and why; WordCraft never writes the deviating form.

## Cases the standard leaves open

| Case | Standard | What WordCraft does | What LibreOffice does |
|---|---|---|---|
| Page margins a page layout doesn't state | §20.206–20.209 refer to XSL §7.10 (initial value 0); whether that default comes with the reference is not said | 0 (XSL's initial value) | 2 cm (its own files always state theirs; some of its templates don't) |
| Height of a header or footer whose style states none | §20.212.2 `fo:min-height`: no default | 0 (as tall as its text) | 0.5 cm |
| *(meaning)* Whether a header's stated minimum height includes its stated spacing from the body | §20.212.2, §20.207: not said | no: spacing comes after the height | yes |
| A header taller than its height with `style:dynamic-spacing` unstated | §20.264: no default | off: the spacing is kept (on, with no height stated, would always drop the spacing the file states) | off |
| Where paragraph tab stops, default ones too, are measured from when `settings.xml` doesn't say | §19.512.3 `style:position` doesn't say; §19.861 covers only indexes | the margin (as Word; ONLYOFFICE's files mean this too) | the paragraph's left indent (its `TabsRelativeToIndent` default): explicit stops, the default stops every interval and the hanging indent's stop all move with the indent (measured) |
| Gap between columns when `style:columns` states none | §19.233 `fo:column-gap`: no default (and explicitly not XSL's) | 0.5 in (Word's) | 0 |
| A drawing shape's fill colour when no style gives one | §20.119 `draw:fill-color`: no default | `#156082` (WordCraft's shape blue) | `#729FCF` |
| A drawing shape's line colour when no style gives one | §20.408 `svg:stroke-color`: no default | `#0E405A` | `#3465A4` |
| A shape's line width when unstated, and *(meaning)* what a stated width of 0 (a hairline) draws at | §20.410 `svg:stroke-width`: no default | 0.25 pt | 0.5 pt |
| *(meaning)* A page-anchored frame positioned against a paragraph or character | §20.299, §20.398 Table 23: combination not defined | the page's text area | the page's edge |
| *(meaning)* An in-line frame positioned against a paragraph or page | §20.398 Table 23: combination not defined | stands on the baseline | positioned against the baseline (raised by its offset) |
| Height of a line holding only in-line pictures | no line-height algorithm in the standard | as tall as its text would be too | as tall as the pictures |
| How the space between two paragraphs adds up when the document doesn't say | not defined (`fo:margin-top`/`-bottom` refer to XSL's margins; whether XSL's space resolution, which keeps the larger, comes with them is not said); LibreOffice's `AddParaTableSpacing` setting says when present (false: the larger) | the larger of the two (as Word) | their sum |
| Widow and orphan control when neither the default paragraph style, Standard nor the paragraph's style states `fo:widows`/`fo:orphans` | §20.312, §20.244 refer to XSL's widows and orphans (initial value 2); whether that default comes with the reference is not said | on: 2 lines (XSL's initial value, and Word's) | off (a first or last line can stand alone at a page's edge) |
| *(meaning)* Whether a columned section's columns end balanced when nothing follows it on the page, and when it doesn't say | §20.428 `text:dont-balance-text-columns`: no default, and its value descriptions contradict its name (WordCraft reads the name; so does LibreOffice) | as Word: only when text follows on the same page | balanced |

## Law: what the standard defines and WordCraft follows

| Case | Rule | Note |
|---|---|---|
| Paragraph padding on a side without a border | `fo:padding` (§20.217) is XSL's box model: padding takes room whether or not there is a border | Word ignores padding without a border. ODF documents don't. |
| A frame's `style:run-through` when unstated | §20.351: `foreground` | |
| `style:dynamic-spacing="true"/"false"` when stated | §20.264 | WordCraft writes `false` when a header or footer has spacing, so every reader keeps it. |

## Other cases open in the standard

These are documented so they aren't mistaken for law:

| Case | Standard | What WordCraft does |
|---|---|---|
| Paper size when no page layout states one | not defined | the region's paper (Letter in the US, A4 elsewhere), as for new documents |
| `style:wrap` unstated on a frame | §20.400: no default | text wraps on both sides (LibreOffice too) |
| A drawing shape's `draw:fill` / `draw:stroke` unstated | §20.118, §20.167: no default | solid (LibreOffice too); a text frame's are none; the colours are listed above |
| A document's default font and size when no default style gives them | not defined | WordCraft's defaults |

## How WordCraft writes ODF

What nobody edited is written back as it was read, so something WordCraft doesn't model survives
a save that doesn't touch it:

- a paragraph or table WordCraft hasn't changed keeps its own markup, with what it names (its
  automatic styles, common styles, font faces, and the package files and folders they refer to);
- while no style or list definition changed, the file's own style definitions are written;
- while the page setup didn't change, its page layouts and master pages, and each section starts
  with the master page it started with;
- named sections around unedited paragraphs, the document properties WordCraft doesn't model,
  the declarations opening the text and the document's scripts.

Paragraphs and tables holding drawings, comments, tracked changes, notes or marks are written as
read too while those are unchanged; paragraphs tied together (a comment, change or mark reaching
into another, linked text boxes) are kept as a group. What is edited is written from the model, as
are the cases listed under "Left" in [odt-element-audit.md](odt-element-audit.md).
`odt::tests_more::lossless` saves samples without edits and fails on anything lost.

Files are saved as ODF 1.3, whatever version they were read as (as LibreOffice does). The changes
from ODF 1.2 (ODF 1.3 Part 3, Appendix G, with their OASIS issues) were checked: none gives a
value written for 1.2 another meaning, apart from a narrow rule on line caps
(OFFICE-3742: a line style's `svg:stroke-linecap` now wins over its dash's `draw:style`), which
matches how LibreOffice drew them. A few give defaults ODF 1.2 left unsaid (`text:anchor-type`
paragraph, `style:run-through` foreground, `number:forced-exponent-sign` true), again as
LibreOffice did. Markup kept from an older file is brought to 1.3 where its producers' meaning
differs: gradient and hatch angles, a deviation from the standard described below.

What WordCraft writes from its model, it writes so that every reader takes it the same way. It
states what the standard leaves open instead of relying on any application's default:

- header and footer heights, with `style:dynamic-spacing="false"` when there is spacing;
- a frame's placement as an explicit position against the page or paragraph;
- in-line pictures standing on the baseline;
- drawings not held in the text's area (`style:flow-with-text="false"`, §20.267 gives no default);
- tab stops measured as the file it was opened from measured them (`TabsRelativeToIndent` in
  `settings.xml`), from the margin for WordCraft's own documents;
- widow and orphan control, on or off, in the default paragraph style.

## Deviations from the standard, out of necessity

WordCraft follows the standard wherever it says something. These are the exceptions: places
where the files in circulation contradict the standard, and following it would damage every
such file. Each is kept as narrow as possible, and WordCraft never writes the deviating form.

| Case | Standard | What WordCraft does, and why |
|---|---|---|
| A gradient's, transparency gradient's or hatch's angle without a unit (`draw:angle`, §19.112; `draw:rotation`, §19.209) in a file older than ODF 1.3 | §18.3.1 (since ODF 1.2): an angle without a unit is in degrees, so `300` is 300° | Read as tenths of a degree (`300` is 30°). LibreOffice and OpenOffice wrote these angles in tenths in every file before ODF 1.3 and still read them so (observed: the same `draw:angle="300"` draws at 30° in an ODF 1.2 file and at 300° in an ODF 1.3 one). Read by the standard, the gradients of every older LibreOffice document would turn the wrong way. Files declaring ODF 1.3 or later are read by the standard. WordCraft writes these angles with a unit (`30deg`), which every reader takes the same way, and markup it keeps from an older file is given that unit. (`strokes::legacy_angles`) |

## Known differences between WordCraft's and LibreOffice's layout of the same file

Found by saving WordCraft's own documents as ODT and opening them in LibreOffice. These aren't
choices the file can state:

- **Footers.** Word, and WordCraft for its own documents, places a footer's text so it ends at the
  footer's distance from the bottom of the page. ODF places it at the top of the footer area. A
  one-line footer in a 1 inch bottom margin sits about 20 pt higher in LibreOffice. Matching it
  would need the footer's text height when writing, and would depend on how a footer's height and
  spacing combine, which is one of the cases above.
- **Proportional line spacing.** Both draw the same distance from line to line. Word puts the
  extra space of a multiple (say 1.15) above the text, LibreOffice below it. Where the font size
  changes (a heading after body text) the two differ by a few points.
- **Leader dots** in a table of contents start right after the text in LibreOffice and after a
  small gap in WordCraft.
- **Line breaks after a slash.** LibreOffice breaks a long path after a `/`; WordCraft, as Word,
  keeps the text after a slash with it and breaks such a word where it must. The standard
  doesn't define line breaking.
- **Objects that may not overlap text** (`style:wrap="none"` with several on one paragraph):
  LibreOffice moves them down by its own rules (`draw:wrap-influence-on-position`).

## Not implemented

LibreOffice refreshes fixed dates and fills in the user's sender fields when it opens a template
(`.ott`) to make a new document. That is application behaviour, not something the file defines.
WordCraft opens a template as it is.

## Other applications

Google Docs and ONLYOFFICE files were checked for cases they rely on (November 2026, nine files
from public bug reports and ONLYOFFICE's converter test snapshots):

- **Google Docs** exports through a LibreOffice build (its generator reads `LibreOfficeDev/6.0`).
  It states its page size and margins, writes `TabsRelativeToIndent=false`, and its default
  graphic style states LibreOffice's colours. Its headers are empty styles with no content.
  Nothing in these files depends on the cases above more than LibreOffice's own files do.
- **ONLYOFFICE** states page size and margins only. It has no header styles and no default
  graphic style. It writes tab stops with no `TabsRelativeToIndent` and means "from the margin"
  (its documents come from Word's model), which is WordCraft's choice too.

## Revision history

| Date | Change |
|---|---|
| 2026-11-04 | The settings removed: File › Options › Advanced › OpenDocument files, `file.options` `odf`, `WORDCRAFT_ODF`, the LibreOffice style and custom values. WordCraft makes one choice for each case the standard leaves open, the one the default made, so nothing changes for a document opened with the default. What a file states is still followed, its `TabsRelativeToIndent` and `AddParaTableSpacing` among it. LibreOffice's choices stay listed beside WordCraft's. |
| 2026-11-04 | How WordCraft writes ODF: unchanged paragraphs and tables with drawings, comments, tracked changes, notes or marks are written as read too (the text said from the model); what is edited, and the cases left in the element audit, from the model. |
| 2026-11-04 | The two presets are the two styles a document can be shown in: Microsoft Word style (`word`, the default; `wordcraft`, its earlier id, still taken) and LibreOffice style (`libreoffice`). The CLI's values listed one by one. |
| 2026-11-03 | Deviations from the standard listed: unitless gradient and hatch angles in files older than ODF 1.3 are read as tenths of a degree, as their producers wrote them, out of necessity (in ODF 1.3 files, degrees, as the standard says). The ODF 1.2 → 1.3 changes checked. |
| 2026-11-03 | More written as read (tables, pages, sections, properties, declarations); files saved as ODF 1.3 with older markup brought to 1.3's meaning; drawings state `style:flow-with-text`. |
| 2026-11-03 | Unedited content written as read (see How WordCraft writes ODF). |
| 2026-11-03 | Tabs measured from the indent: default stops count from the indent too (measured in LibreOffice), in WordCraft's layout, and a file measuring tabs so is written back so (it was written from the margin, moving LibreOffice's default stops). |
| 2026-11-03 | Widow and orphan control a file doesn't state (`widowControl`): WordCraft as XSL's initial value, LibreOffice off. WordCraft writes it on its default style either way. |
| 2026-11-03 | Known layout differences listed. |
| 2026-11-03 | Paragraph spacing (`sumParagraphSpacing`); a file's `AddParaTableSpacing` is followed, and WordCraft writes it. WordCraft's `settings.xml` now declares the `ooo` namespace, without which LibreOffice ignored all of its settings. |
| 2026-11-03 | Section column balancing (`balanceSectionColumns`); §20.428's contradiction recorded. WordCraft writes `text:dont-balance-text-columns` on every columned section. |
| 2026-11-02 | Unstated page margins are a setting (`pageMargin`: WordCraft 0, XSL's initial value; LibreOffice 2 cm), not law: the standard refers to XSL's margins without saying whether XSL's default comes too. |
| 2026-11-02 | First version: the profile, the open cases, law cases, writer choices, Google Docs and ONLYOFFICE findings. Stated values are never replaced by a preset; the cases about the meaning of stated values are marked. |
