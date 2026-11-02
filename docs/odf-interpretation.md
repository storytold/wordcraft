# How WordCraft interprets OpenDocument files

> **Last reviewed:** 2026-11-02 · **Last updated:** 2026-11-02 · **Change:** major (new: the ODF interpretation profile and the list of every case)

OpenDocument (ODF 1.3, OASIS) leaves some of a document's look to the application. Part 3 §16.2
says that when no style sets a property and the standard gives no default, "an
implementation-dependent value is used". So a file can look different in WordCraft, LibreOffice,
Google Docs and ONLYOFFICE without anyone being wrong. This page lists every such case WordCraft
knows of, so none gets lost.

## The rule

1. **What the standard defines is law.** WordCraft follows it whatever the settings say. That
   includes `fo:` properties, which the standard defines by referring to XSL 1.0. Where it does,
   XSL's initial value is the default.
2. **What the standard leaves open is a setting.** Each such case is one value of the ODF
   interpretation profile (`OdfProfile`, `crates/doc/src/odf_profile.rs`):
   - **WordCraft**, the default: what WordCraft does for its own and Word documents.
   - **LibreOffice**: what LibreOffice (and Collabora, which shares its engine) does. Measured
     black-box in LibreOffice 24 where it isn't documented.
   - **Custom**: the user's own value for each case. It starts from the preset that was in use.
3. **A value the file states is the file's.** A preset never replaces it: the profile only fills
   in what the file (its styles, defaults and `settings.xml`) leaves unsaid. Three cases are not
   missing values but stated ones whose meaning the standard leaves open: how a header's stated
   height and spacing combine, what a stated width of 0 draws at, and a frame position stated
   against something the standard doesn't define for its anchor. They are marked *(meaning)*
   below. `formats::odt::tests_more::profile` checks that stated values survive every preset.
4. The profile applies to ODF files (`.odt`, `.ott`, `.fodt`) *as they are opened*. A document
   keeps what it was opened with. Changing the setting affects the next file opened.

Where to set it:

| Interface | How |
|---|---|
| App | File › Options › Advanced › OpenDocument files |
| Command | `file.options` `{"odf": {"preset": "libreoffice"}}`, or one case's value: `{"odf": {"columnGap": 18}}` (this makes the profile custom) |
| MCP | the same command |
| CLI | `WORDCRAFT_ODF=wordcraft\|libreoffice` |

## Cases the standard leaves open (the profile)

| Case | Standard | Setting (`file.options` `odf.*`) | WordCraft | LibreOffice |
|---|---|---|---|---|
| Height of a header or footer whose style states none | §20.212.2 `fo:min-height`: no default | `headerMinHeight` | 0 (as tall as its text) | 0.5 cm |
| *(meaning)* Whether a header's stated minimum height includes its stated spacing from the body | §20.212.2, §20.207: not said | `headerHeightIncludesSpacing` | no: spacing comes after the height | yes |
| A header taller than its height with `style:dynamic-spacing` unstated | §20.264: no default | `dynamicSpacing` | off: the spacing is kept (on, with no height stated, would always drop the spacing the file states) | off |
| Where paragraph tab stops are measured from when `settings.xml` doesn't say | §19.512.3 `style:position` doesn't say; §19.861 covers only indexes | `tabsFromIndent` | the margin (as Word; ONLYOFFICE's files mean this too) | the paragraph's indent (its `TabsRelativeToIndent` default) |
| Gap between columns when `style:columns` states none | §19.233 `fo:column-gap`: no default (and explicitly not XSL's) | `columnGap` | 0.5 in (Word's) | 0 |
| A drawing shape's fill colour when no style gives one | §20.119 `draw:fill-color`: no default | `shapeFill` | `#156082` (WordCraft's shape blue) | `#729FCF` |
| A drawing shape's line colour when no style gives one | §20.408 `svg:stroke-color`: no default | `shapeLine` | `#0E405A` | `#3465A4` |
| A shape's line width when unstated, and *(meaning)* what a stated width of 0 (a hairline) draws at | §20.410 `svg:stroke-width`: no default | `hairline` | 0.25 pt | 0.5 pt |
| *(meaning)* A page-anchored frame positioned against a paragraph or character | §20.299, §20.398 Table 23: combination not defined | `pageFrameParagraphIsPage` | the page's text area | the page's edge |
| *(meaning)* An in-line frame positioned against a paragraph or page | §20.398 Table 23: combination not defined | `inlineInvalidRelationIsBaseline` | stands on the baseline | positioned against the baseline (raised by its offset) |
| Height of a line holding only in-line pictures | no line-height algorithm in the standard | `pictureLinesFitPictures` | as tall as its text would be too | as tall as the pictures |

## Law: what the standard defines and WordCraft follows whatever the profile

| Case | Rule | Note |
|---|---|---|
| Page margins a page layout doesn't state | `fo:margin-*` (§20.207 et seq.) refer to XSL §7.10.1: initial value 0 | LibreOffice draws 2 cm, but its own files always state their margins. A file without them (some templates) opens with 0 margins. |
| Paragraph padding on a side without a border | `fo:padding` (§20.217) is XSL's box model: padding takes room whether or not there is a border | Word ignores padding without a border. ODF documents don't. |
| A frame's `style:run-through` when unstated | §20.351: `foreground` | |
| `style:dynamic-spacing="true"/"false"` when stated | §20.264 | WordCraft writes `false` when a header or footer has spacing, so every reader keeps it. |

## Open in the standard, and the same in every preset

These are documented so they aren't mistaken for law:

| Case | Standard | What WordCraft does |
|---|---|---|
| Paper size when no page layout states one | not defined | the region's paper (Letter in the US, A4 elsewhere), as for new documents |
| `style:wrap` unstated on a frame | §20.400: no default | text wraps on both sides (LibreOffice too) |
| A drawing shape's `draw:fill` / `draw:stroke` unstated | §20.118, §20.167: no default | solid (LibreOffice too); a text frame's are none; the colours are in the profile |
| A document's default font and size when no default style gives them | not defined | WordCraft's defaults |

## How WordCraft writes ODF

WordCraft writes files that every reader takes the same way. It states what the standard leaves
open instead of relying on any application's default:

- header and footer heights, with `style:dynamic-spacing="false"` when there is spacing;
- a frame's placement as an explicit position against the page or paragraph;
- in-line pictures standing on the baseline;
- tab stops measured as `settings.xml` says.

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
  Nothing in these files depends on the profile beyond what the LibreOffice preset covers.
- **ONLYOFFICE** states page size and margins only. It has no header styles and no default
  graphic style. It writes tab stops with no `TabsRelativeToIndent` and means "from the margin"
  (its documents come from Word's model), which is WordCraft's default.

So neither needs a preset of its own yet. If later files show cases where they differ, a preset is
one more `OdfPreset` with its values, and the cases above stay the same.

## Revision history

| Date | Change |
|---|---|
| 2026-11-02 | First version: the profile, the eleven open cases, law cases, writer choices, Google Docs and ONLYOFFICE findings. Stated values are never replaced by a preset; the cases about the meaning of stated values are marked. |
