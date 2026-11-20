# HWP/HWPX clean-room record

`crates/hwp` (reading and writing HWP/HWPX) is clean-room: built from published format documents and
from observation of Hancom's viewer, without the code of any other HWP/HWPX implementation. This page
records what it was built from, what was excluded, and every rule settled by observation. New work
on the crate follows the same rules, together with WordCraft's own clean-room rules (`AGENTS.md`,
"Clean room").

**History**: the work started as a Python tool outside WordCraft, in a private repository
(2026-10-23 to 10-28). The rules found there were moved to Rust on 2026-10-28, from the same
sources. The Rust version produced byte-identical DOCX output to the Python version (`--keep-fonts`)
on all 30 documents of the test set at the time (13 HWPX, 17 HWP). The measurements in sections 4
to 6 below were made with the Python version; the places named in the tables are the Rust files.
Features only the Python version had (font replacement, `fonts.py`) are marked as such. HWPX save
(`writer.py`) was ported as described in section 6.

## 1. Sources used (allowed)

| Source | What it is | Licence or terms | What it was used for |
| --- | --- | --- | --- |
| Hancom `hwpx-owpml-model` (github.com/hancom-io/hwpx-owpml-model, commit 1453388) | The OWPML model Hancom published | Apache-2.0 | Element and attribute names, enumeration values, namespace URIs, package paths. Only the names were extracted, with `docs/hwp/tools/extract_owpml_names.py`, into `docs/hwp/owpml-reference.md`. No code was taken |
| Office Open XML (ECMA-376) | The DOCX standard | Public standard | The output side |
| Hand-made test files (`crates/hwp/src/tests.rs`; in the Python version `tests/make_fixture.py`) | XML and records written by hand from the name list and the published documents | Owned by this project | Checking behaviour |
| Hancom "한글 문서 파일 형식 5.0" (Hangul document file format 5.0), revision 1.3 (published in Hancom's download centre; kept outside the repository) | Hancom's published document | Attribution sentence required | HWP record structure, tag numbers, field layout (`crates/hwp/src/hwp5.rs`) |
| E. W. Myers, "An O(ND) Difference Algorithm and Its Variations", Algorithmica 1 (1986) | A published paper (an algorithm, no code) | None needed | Finding the difference between two texts: `opcodes`, `shared_runs`, `step` and `trace_back` in `crates/hwp/src/writer.rs` are written from the paper's greedy search. Until 2026-11-17 this was a Rust port of CPython's `difflib.SequenceMatcher` (PSF License Version 2, outside the licences WordCraft allows); it was replaced by new code, written without reading the port or `difflib` |
| Microsoft [MS-CFB] | Published compound file specification | Open Specifications | Reading the HWP container (Rust: the `cfb` crate (MIT) WordCraft already uses; Python: a hand-written `cfb.py`). `cfb` is a general compound-file library, not an HWP implementation |

A product built with the HWP 5.0 document must carry the sentence "본 제품은 한글과컴퓨터의 한글 문서
파일(.hwp) 공개 문서를 참고하여 개발하였습니다." ("This product was developed with reference to Hancom's
public document on the Hangul document file (.hwp) format."). It is in the header comments of
`crates/hwp/src/lib.rs` and `hwp5.rs`, in `NOTICE`, and in the app's About window (Help › About, which
also serves as help), next to a sentence saying WordCraft is not affiliated with Hancom. It is drawn
with the font the system draws Hangul with (Malgun Gothic on Windows, for example) whatever the
interface language; only on a computer with no Korean font at all is an English sentence of the same
meaning shown instead (no font is bundled for it).

### Terms checked (2026-11-02)

A legal-risk review before publishing pointed out that the original terms of Hancom's viewer and of
the published document had not been read, so both were read.

**Hancom Office Viewer** (Hancom Office 2024 Viewer; used to open documents and capture the viewer's
window for comparison)

- Where: the "사용권 및 저작권 안내" (licence and copyright) section of the download page
  <https://www.hancom.com/product/office/officeViewer>. The page links no separate licence agreement
  (the "이용약관" at the bottom is the website's terms), and the installed viewer's folder has no
  licence file (only notices for open-source libraries).
- Clauses (in our words):
  - Anyone may download and use it freely, but only for personal, non-commercial purposes.
  - Companies and organisations use it after Hancom's written approval.
  - It may not be redistributed or passed on without Hancom's permission.
  - Its accompanying data files and documents may not be modified, altered or copied, in whole or in
    part, without Hancom's prior written consent.
  - It may be used until the next version is released.
- Conclusion: no clause forbids reverse engineering, decompiling, comparison testing or screenshots.
  This project uses it as an individual, for non-commercial open-source development, and does not
  modify, copy or distribute the viewer or its files. Check again for use on behalf of a company or
  organisation, or when a new version is released.
- Installer agreement (checked 2026-11-14): the installer signed by Hancom (a newer build, 13.0.0.434,
  of the same 2024 viewer; the comparisons used 13.0.0.418) was run and the full text of its end-user
  licence agreement screen was read, stopping without agreeing. It contains all the clauses above in
  the same words. Clauses only on that screen: no warranty or
  liability, automatic updates, collection of user information for updates and advertising,
  advertising, installation of third-party services, and Microsoft Edge WebView2. None of them forbids
  reverse engineering, comparing or testing output, screenshots or developing other products. The
  conclusion is the same.

**"한글 문서 파일 형식 5.0", revision 1.3** (the "저작권" (copyright) section at the start of the
document)

- Clauses:
  - Anyone may freely view, copy, distribute, publish and use it. Only unmodified originals or copies
    may be distributed.
  - The copyright of results developed with reference to the document belongs to the person or group
    that developed them.
  - Results must carry the attribution sentence in all of the product's user interface, manual, help
    and source; where some of these do not exist, in those that do.
  - Hancom may assert its rights against anyone who obtains exclusive rights from results made with
    the document and asserts them against Hancom.
  - Hancom does not warrant the accuracy of the results.
- Conclusion: nothing forbids implementing behaviour the document does not describe, found from real
  files, nor writing (saving) as well as reading. The attribution sentence is in the About window
  (interface and help), `docs/hwp/README.md` (manual), the source header comments and `NOTICE`. This
  project does not seek exclusive rights such as patents.

**NanumGothic notice in the web build** (checked 2026-11-14; withdrawn 2026-11-18)

- Withdrawn: the web build no longer embeds NanumGothic. Korean in a browser comes from the
  computer's installed fonts, through the Load Installed Fonts bar; no Korean font is bundled for
  it. What follows records the earlier check.
- This change also embedded craft-fonts' NanumGothic (NanumGothic Regular, OFL-1.1) in the web build
  (`crates/fonts/build.rs`). A browser cannot use system fonts, so without it Korean shows as boxes.
  The desktop build already embeds it upstream.
- The OFL requires the copyright notice and the licence text to go with the font when it is
  distributed. The font file's own information carries only the copyright line ("Copyright © 2011 NHN
  Corporation…"), not the OFL text.
- Checked: the web zip was built with `CRAFT_FONTS_DIR` set and upstream's `packaging/web/package.sh`
  (the script official releases use for the web build). The zip contains `OFL-nanum-gothic.txt` (the
  NHN copyright line and the OFL text), `NOTICE` and `ATTRIBUTION.md`, and the wasm contains
  NanumGothic with its copyright information. The OFL file comes from upstream's `copy_font_licences`,
  which copies every OFL file of craft-fonts.
- `NOTICE` and `ATTRIBUTION.md` name NanumGothic, its copyright holder and licence, that the web build
  carries it too, and the licence file's name.

**WordCraft brand terms** (upstream `docs/brand/LICENSE-brand.txt`, as of upstream commit `4fffa810`
(2026-10-12), checked 2026-11-14)

- Covers: "The ArtCraft name, wordmark and mark". The name "WordCraft" is not in these terms, and
  nothing in the upstream repository calls it a trademark.
- Clauses:
  - "If you distribute or publish a modified version of WordCraft, or any work derived from it, you
    must remove the ArtCraft Marks from it (or replace them with your own) and must not call it
    ArtCraft or present it as an ArtCraft product."
  - "A fork kept only to propose changes back to this repository may keep the marks unchanged."
- Conclusion: this work is not released separately; it is made to propose changes to this
  repository (decided 2026-11-14). So the name, the logo files (`docs/brand/`) and the app id stay as upstream
  has them. A separate release would have to remove the ArtCraft marks under the first clause, so
  check again then.

**Check before publishing** (2026-11-14, whole repository with `git grep`)

- Where Hancom's attribution sentence (exactly as above) is: the About window (`HANCOM_NOTICE` in
  `crates/ui-egui/src/dialogs/dispatch.rs`, with the English sentence only on computers without a
  Korean font), help ("Help" on the ribbon's Help tab opens that window: `ribbon/review.rs`,
  `keytips.rs`), the manual (`docs/hwp/README.md`), the source (the header comments of all 22 source
  and example files of `crates/hwp`), `NOTICE` and `README.md`.
- Every name and identifier of a dropped plan for a separate release (app name, app id, file-type
  ProgIds, the installer's UpgradeCode and component GUIDs, executable and package names, settings
  folder, the contributor's repository address) was searched for one by one; none is in the repository. The
  remaining `wordcraft-hwp` is this crate's name.

## 2. Excluded material (not allowed)

Never read it, and never use it as a dependency:

- pyhwp / hwp5 (AGPL), hwplib · hwpxlib, python-hwpx, hwp.js, rhwp, syhwp, HOP, hwpforge, the
  LibreOffice HWP filter, and every other HWP/HWPX implementation
- Results of reverse engineering Hancom Office programs
- XML fragments copied from real documents made by Hancom (test files are written by hand)
- WordCraft's HWP pull requests (#2065, #3764) and their branches (other people's work on the same
  feature)
- Documentation of other HWP tools (such as k-skill's rhwp guides): dropped on 2026-10-27

**Review (2026-10-24)**: hwpxlib (neolord0, Apache-2.0, Java) is allowed by its licence but is not
used. It adds little to reading, and using it would lose the clean room, the single licence and the
absence of outside dependencies. To be reviewed again when HWPX save starts (stage 5); even then,
no code is taken, and the preferred use is as a **checking tool** only: reading the HWPX we write with
it to see that there are no errors.

## 3. How the work is done

1. Element names and attributes are looked up in `docs/hwp/owpml-reference.md`, and the parser is
   written from those names only.
2. Where the published material doesn't give a value's meaning (unit, reference point and so on),
   implement a guess and list it under section 7, "Still to check".
3. A guess is checked by comparing results on real HWPX files, made or received, and then taken off
   section 7. Real files are only for comparison and never go into the repository. Documents for
   comparison are kept in this repository's `samples/` (ignored by git) and read only there
   (`_out/hancom_pages.tsv` keeps the page counts read from Hancom's viewer).
   Since 2026-11-02 the evidence of comparisons is kept, not deleted, outside the repository under
   `hwp-evidence/<date>-<topic>/`, as the clean-room record: test files we made (.hwp/.hwpx saved by
   WordCraft and the command scripts), captures of Hancom's viewer window showing them, WordCraft's
   view of the same document, and a `note.md` saying what was checked and decided. No copies of
   `samples/` documents are kept there (only what a capture shows). This folder is neither published
   nor committed.
4. The Rust version uses only crates already in the WordCraft workspace (craft-zip, craft-xml,
   cfb, miniz_oxide, and WordCraft's own wordcraft-doc and wordcraft-fonts). No HWP-specific library.
   (The Python version used only the standard library.)

### Test set

- **Until 2026-11-14**: 30 documents (13 HWPX, 17 HWP).
- **2026-11-14 to 11-16**: 15 documents (13 HWPX, 2 HWP). Rechecked on them: an unedited `.hwp` saves
  back byte for byte (record reading and writing, the top-bit rule for a paragraph's character
  count); tables and the objects in table cells survive a save (`records_round_trip`,
  `hwp_tables_keep_objects`). Page counts matched Hancom's viewer in 6 of 15 (HWP 1/2, HWPX 5/13).
- **From 2026-11-16**: 20 documents (10 HWP, 10 HWPX), replacing the earlier set. Figures elsewhere
  on this page are from the set of their date. Checked on this set (2026-11-16):
  - Saved unedited: the 10 HWP are byte-identical to the original; for the 10 HWPX every file in the
    package has the same content.
  - Page counts match Hancom's viewer in 16 of 20 (HWP 7/10, HWPX 9/10). Three differ by one page;
    one long HWP document is 140 pages in WordCraft against 145 (176 before the cell-margin rule of
    section 5). (2026-11-17) 17 of 20 (HWP 8/10) with the group and inline-table rules of section 5.
  - A row added to every table of each HWP document and saved: nothing left unwritten, and every
    table has its new row when read back (`hwp_tables_keep_objects` adds one table at a time). At
    first nine files opened in Hancom's viewer; the long document with 165 tables rewritten at once
    made the viewer quit on most tries, more often the more tables were rewritten. Three causes were
    found in the written tables, and with them fixed all ten open (3 tries each, the long one 9 of 9):
    - A row inserted inside a vertical merge left the merge's continued cells under a cell that
      doesn't start one. The writer skipped them, leaving holes in the table grid (173 in 28
      tables of that document; a few in two others, which still opened). Now a continued cell joins
      the cell above it (section 6b), as WordCraft draws it.
    - Cells with −1 margins were written back with 655 pt margins, so the estimated row heights grew
      to over a metre (section 5).
    - Each rewritten table added its own copies of the same generated shapes (character shapes
      1,220 → 3,664). They are now shared (2,304; the remaining ones all differ). This alone didn't
      stop the viewer quitting.
    - Before the first two fixes, dropping every line layout cache in the file (a trial, not kept)
      made it open more often but not always.
  - The horizontal base value 0 of an object is the paper edge (section 6b, `horzRelTo` in
    `hwp5.rs`): in the one document with such objects, page-number boxes placed 96.7 mm from the left
    with value 0 sit in the middle of the page in Hancom's viewer, as they do in WordCraft.
- No test document has ever been in the repository's history.
- **Needs a new check**: these rules were checked only on documents no longer in the test set. They
  differ from the published document, so the code stays as it is, but they count as unconfirmed until
  checked again on other documents:
  - Pictures: a picture record points to BIN_DATA by record **order**, not by storage id (section
    6c). The current set can't tell: in its two documents with pictures the storage ids run 1, 2, …
    in record order.
  - The structure and place of memo fields and MEMO_LIST (sections 6b, 6c, 6d): no document of the
    current set has memos.
- **Checked again on the current set (2026-11-17)**:
  - Bullet records (section 6d), against the published document (table 42): paragraph head
    information (table 39 lists attributes 4, width correction 2, distance 2 and a character shape
    id 4 bytes: 12 bytes, though it gives the length as 8), the bullet character 2, whether it is an
    image bullet 4, the image bullet's contrast, brightness, effect and id 4, a check bullet
    character 2. The current set's records are 25 bytes: those 24 and one more zero byte the
    document doesn't list. The earlier set's were 23, as the new-HWP writer writes (version
    5.0.4.0): the check bullet character cut short. Reading takes the character at byte 12 either
    way.
  - Inserting, deleting and resizing pictures in original paragraphs (section 6c): on a cover page
    with four pictures, one picture halved, one deleted, and new pictures added to a text paragraph
    and to the first paragraph (section settings and two pictures): Hancom's viewer shows each edit,
    and the page count stays.
  - The width of a level line with no height (`convert.rs`): a level arrow in a group of lines is
    drawn as long as in Hancom's viewer.
  - A bullet in the private-use range (a symbol font's glyph): U+F0FC is drawn as a check mark in
    Hancom's viewer (four list items in one document), so it reads as "✓"; other private-use
    bullets stay "•" until seen (`bullet_char` in `convert.rs`).
  - The explanation of HWP page differences in section 5 holds for the one HWP document of this
    set that is a page longer in WordCraft only in part: its last box went to the next page, but
    because of three reading errors around a group of lines, now fixed (section 5, 2026-11-17).
    It now has one page, as in Hancom's viewer (page counts 17 of 20).
- Tests and page comparisons that use `samples/` read only the files directly in `samples/`.

## 4. Checked on real documents

### How

- 15 real HWPX documents were compared side by side in **Hancom's viewer** and in WordCraft.
  First round (2026-10-23): a two-page survey of mostly paragraphs, a one-page table form. Second: 3
  public documents (37 pages with 4 sections, 62 tables and 5 pictures; 4 pages; one landscape page).
  Third (2026-10-24): 10 administrative documents (9 to 66 pages, multi-column contents, form tables,
  supplementary provisions).
- Hangul page counts were read from the viewer's status bar ("1/37쪽") through Windows UI
  Automation. Page counts from the layout information inside the file were not used, because they
  miss tables and columns that span pages.
- Third round: of 13 documents, 7 match in page count, 3 differ by one page, 3 by two (9 pages in
  total over about 300). The viewer was only run and its window captured; the program itself was
  not analysed.
- The **line layout results Hancom stores in HWPX files (`hp:lineseg`)** were read to find the real
  values of line height and spacing. This is public data that is part of the format.
- Table row heights were compared by measuring the position of the table borders in both views, in
  mm.
- The documents compared were not committed.

### Results

| Item | Status | Evidence | Handling |
| --- | --- | --- | --- |
| `hp:switch` choice | Confirmed | In all 43 HwpUnitChar blocks of two documents, the `hp:default` paragraph margins are exactly twice the others. In the 2016 paragraph block, `hp:default` loses the outline information | A `hp:case` of a known namespace first, else `hp:default` (`KNOWN_CASES` in `crates/hwp/src/xml.rs`) |
| Paragraph margin unit | Evidence found | The `hp:case` values are outline 1 to 7 = 10 to 70 pt, body = 15 pt, Hangul's defaults | HWPUNIT as is. Measuring a paragraph with margins on screen is still to do |
| Page margin model | Confirmed | Body start: top 10 + header 10 = 20 mm, top 20 + header 15 = 35 mm, in both documents | Body start = top + header |
| Line spacing (%) | Confirmed | `lineseg` line spacing = font size × ratio (10 pt at 160% → 1600) | Font size × ratio |
| Last line of a cell | Confirmed | The last line in a cell gets no extra line spacing (5 pt at 130%: height = margins + 500) | A one-line last paragraph in a cell has a fixed height equal to the font size |
| `landscape` | Confirmed | Portrait documents say `WIDELY`, landscape ones `NARROWLY` (stored with width < height, shown landscape) | `NARROWLY` = landscape, width and height swapped |
| Cell zones (`hp:cellzone`) | Confirmed | The diagonal in a table's header cell belongs to the cell zone's border, not the cell's. The zone's border is `NONE`, yet the cell borders still show | Only what the zone sets (borders other than NONE, diagonals, fill) is laid over the cells |
| Page position | Confirmed | `lineseg vertpos` starts again from 0 on each page | Used to count which page a paragraph is on |
| Hide on first page, restart page numbers | Confirmed | Page number hidden on the cover (`pageHiding`), the next section starts at 1 (`newNum`) → the same page numbers as Hangul | On the first page of a section, as DOCX first-page header/footer and start number |
| Column change inside a section (`colPr`) | Confirmed | Only the contents are in 2 or 3 columns, the body is back to one. Making the whole section multi-column changed the page count by up to −22 | A DOCX continuous section break wherever the columns change |
| Overlaid characters (`hp:compose`) | Confirmed | Circled numbers 19 to 26 are stored as two characters of Hancom's private use area (tens U+F0288+n, units U+F0292+n). Guessed from the clause numbers before and after, then checked on the viewer's screen ("①～⑲", "⑳", "㉑～㉖") | Replaced by Unicode circled numbers (1 to 50) |
| Page break + paragraph starting with a table | Confirmed | Each page of an annexed form is "page break + table"; a table can't carry a page break, so it was lost | A paragraph of almost no height carries the page break |
| Table that fills the page | Confirmed | Form tables are up to 0.5 mm taller than the body (including outer margins). Hangul keeps them on one page | A table slightly (up to 5%) taller than the body has its tallest row shrunk to fit (see the WordCraft table below) |

### WordCraft's own behaviour (measured, WordCraft 0.10)

| Behaviour | Measured | Handling |
| --- | --- | --- |
| Cell top and bottom margins are **added** to the row height (`w:trHeight`) | 144 pt row + 25 pt × 2 margins → 196 pt | Cell margins are subtracted from the row height |
| Border width is added to the row height | 3 pt border → +1.06 mm per row | The top border (and on the last row the bottom one) is subtracted from the row height |
| A document ending in a table gets an empty paragraph of normal height | A one-page table form became two pages | End with a paragraph of almost no height |
| No Korean word-unit line breaking | A word broken between syllables at the line end (like "해바라/기" for "해바라기") | Can't be fixed in conversion; needs a WordCraft feature |
| "At least" line spacing grows to the font's own height (about 1.2 to 1.3×) | A 100% paragraph is taller than in Hangul | Paragraphs with only text get an "exact" line height = font size × ratio |
| In an "exact" line the text sits at the bottom, the extra space above it (as in Word); Hangul puts the text at the top and the line spacing below it (2026-11-17) | A 12 pt heading at 180% followed by a table: Hancom's layout cache has the table's line 12 + 9.6 pt below the heading's, the gap below the text; in WordCraft the heading's text touches the table | Line heights are the same, so page counts are not affected. Not changed: "multiple" spacing keeps the text at the top but scales the font's own height, not its size (see the row above), and placing text in an exact line is WordCraft's layout |
| Cell diagonals (`tl2br`/`tr2bl`) are not drawn | Thick red and blue diagonals don't show | Kept in the DOCX (Word shows them). Needs a WordCraft feature |
| Fonts are found by their Korean names | Both "함초롬바탕" and "HCR Batang" are recognised | Names used as they are |
| A table that almost exactly fits the body has its last row moved to the next page | Moved even with 0.5 mm left; not moved with 2.5 mm (700 HWPUNIT) | Tables that nearly fill the page are fitted to body height − 2.5 mm |
| A missing font is drawn in Gothic even for Myeongjo (serif) Korean, and the `w:altName` alternative name is ignored | "휴먼명조" shows in Gothic | Only fonts that aren't installed are replaced by an installed font of the same kind (brush, Myeongjo, Gothic) (`crates/hwp/src/fonts.rs`; Myeongjo and brush in the order Nanum → Hamchorom → Batang; Gothic NanumGothic, else Pretendard, else WordCraft's own fallback (2026-11-18)). Whether a font is installed is checked with WordCraft's font list, so it works on every OS (2026-10-31; before that, Windows only). The replacements are kept in the DOCX custom property "HWPX font substitutions" and the original names are restored when saving to HWPX |

### Remaining differences and methods tried

- Hangul draws Hancom fonts such as 한양신명조 with fonts that ship inside the viewer. Judging from where
  lines break, their Korean glyphs are 6 to 9% wider than the stand-in (Hamchorom Batang), so
  WordCraft breaks fewer lines and gets one or two pages fewer.
- Wider letter spacing to make up for it was tried (+0.03 to 0.09 em). It gained one page at most,
  changes how text looks and would have to be undone when saving to HWPX, so it is not used.
- Candidate stand-ins (Hamchorom Batang/Dotum, Batang/Gulim, Batang/Malgun Gothic, Nanum
  Myeongjo/Gothic) were compared on all 13 documents: Hamchorom and Nanum were equal (15 pages of
  difference in total) and Batang was worse (19). Hamchorom stays.

### Paragraph margins and space width (2026-11-01, measured in the viewer)

Test documents were made with one value changed at a time, and the position of the text was
measured on Hancom's viewer screen (capture pixels → pt).

| Item | Measured | Handling |
| --- | --- | --- |
| Paragraph margin unit `CHAR` (the `unit` of `hc:left` and others) | 2000 is 120 pt with 12 pt text and 240 pt with 24 pt text. The first line and space before follow the same ratio. So a value of 100 is one half-width character (font size × value ÷ 200) | `ParaPr::in_hwpunit` in `header.rs`: converted to HWPUNIT with the paragraph's font size |
| Space width (`useFontSpace="0"`, HWP character shape attribute bit 25 = 0) | At 20 pt one space is 10 pt with width ratio 100 and spacing 0; 9 pt at 90% width; 9 pt at −10% spacing; 8 pt with both. So half the font size times the width ratio, with letter spacing as a ratio of that half width | All 1,321 character shapes in the sample HWPX files are 0. WordCraft uses the font's own space (0.25 to 0.3 em for the stand-ins), so spaces are split off and given recalculated letter spacing (`space_rpr` in `convert.rs`) |
| Minimum space (`hh:paraPr condense`, HWP paragraph shape attribute bits 9 to 15) | A paragraph whose 11 spaces must shrink by 8.2% to fit one line stays on one line from value 9, and wraps up to 8. So spaces shrink by up to that ratio when a line is filled | WordCraft's spaces don't shrink, so they start at the shrunk width (lines break the same way, and justified lines stretch again). Spaces at the start of a paragraph (indentation typed with spaces) are not shrunk |

On a real document (two pages of one HWPX test document): the second-line positions set by
paragraph margins and hanging indents were already within 1 pt of Hancom's, and first lines starting
with spaces went from 6 to 8 pt off to 0.2 to 1.2 pt.

Page counts (30 documents): of the 17 HWP, 14 still match, and the total difference fell from 8 pages
to 4 (the longest test document 88 → 92 pages; Hancom 94). Of the 13 HWPX, matches fell from 7 to 5
and the total difference rose from 8 pages to 11. What the remaining differences showed:

- Hangul's paragraph shape `breakNonLatinWord` (60% of the sample paragraph shapes are `KEEP_WORD`)
  is followed (2026-11-01). WordCraft's document model has a paragraph property for Hangul documents,
  `keep_hangul_words`; when it is on, break opportunities next to Hangul letters are removed, so lines
  break only between words (`layout/src/kinsoku.rs`). WordprocessingML has no such value, so the
  converter records it per paragraph (`Report::keep_words`) and the engine sets it after the DOCX read
  (`wordcraft_hwp::apply_keep_words`). It is not written to .docx, and line breaking of .docx documents
  is unchanged (Word's Korean behaviour was not checked). A test document in the same font broke its
  lines at the same places as Hancom's. Sample page counts did not change (19/30).
- In one document, table cells are taller in WordCraft.

## 5. Reading HWP (binary) (2026-10-27)

HWP records are rebuilt into the same element tree as HWPX, so one converter handles both. Where
the published document and real files differ, the 17 HWP files in `samples/` at the time were
compared by the values inside the file and by Hancom's viewer screen (the viewer was only run and
captured, not analysed).

| Item | Differs from the published document / not in it | Evidence | Handling |
| --- | --- | --- | --- |
| Line type | 0 = no line; the document's table is shifted by one | A border with no line is 0, a solid line 1 | `LINE_TYPES` |
| Section definition byte 36 | Not in the document's table | Non-zero only in documents that look vertical | `textDirection="VERTICAL"`, DOCX `tbRl` |
| Object placement (attribute bits 21 to 23) | 0 wrap, 1 take space, 2 behind text, 3 in front of text (as documented). Order differs from HWPX's six values | Only 0 to 3 occur; a table treated as a character is 1 | `common` in `hwp5.rs` |
| Position and size of objects in a group | The stored size is from before the group was resized. The rendering information (translation matrix, plus scale and rotation matrices per level) must be multiplied in to get the real box | The multiplied box matches the group size and the screen | `placed` in `hwp5.rs` |
| The outermost group's drawing space (2026-11-17) | Its members are placed in final units like a nested group's, so its space is its current size, not its stored original size | A group of lines 159 pt tall with an original size of 220 pt: drawn in the original size it was 0.72 times too short; in its current size it matches Hancom's viewer | `component` in `hwp5.rs` |
| A table on a text line (2026-11-17) | Sits where its paragraph aligns it (a right-aligned paragraph puts it at the right of the column or cell), not by its own horizontal alignment. Floating objects in the same paragraph are placed from the top of the table's line | In a right-aligned paragraph a table 31 pt narrower than its cell stood 31 pt in, with a group of lines left of it, in Hancom's viewer; at the left the table covered the lines | DOCX tables don't follow their paragraph: the alignment becomes the table's (`table` in `convert.rs`), and the piece of the paragraph before the table holding only floating objects and spaces takes (almost) no height (`para_table`) |
| Form objects (`form`) | Properties are stored as one string "name:type:length:value" | The string describes its own format | Radio buttons, check boxes, buttons and input fields shown as text boxes of the same size |
| Field guide text | Not shown in the viewer when the content equals the guide text | Viewer screen | Kept as hidden text (`w:vanish`) |
| Cell margins of −1 (2026-11-16) | The four cell margins are HWPUNIT16 (signed). All four −1 (0xFFFF) means the cell has no margins of its own; nothing in the document says so | One document of the current set has 249 such cells (in tables whose own inner margins are 1.4 or 5.1 pt). Read as 655.35 pt margins, the document was 176 pages in WordCraft; taking the table's margins it is 140, against 145 in Hancom's viewer, with no other document's count changed | `hasMargin="0"` (the table's margins, as in HWPX) in `hwp5.rs` |

Further Hangul layout rules found (`lineseg` values and the screen):

- Line height is set **per line** by the largest character on that line. In a paragraph with a few
  larger characters, such as a citation "[1]", the line height follows the size of most characters
  (when the largest is at most 1.2×).
- The extra line spacing after the last line of a page (or column) takes no space. A one-line
  paragraph at the end of a page takes only the text height.
- A table treated as a character on the same line as text or other tables (such as member number
  boxes) is put in an invisible text box to stay on the line.
- The line height of a one-line paragraph includes the size of its bullet or paragraph number (a
  heading with 13 pt text has a 14 pt line). One-line text paragraphs use the `lineseg` line height
  (when it is between the font size and 1.5×).
- Page border and fill (`PAGE_BORDER_FILL`): attributes, spacing on four sides and border/fill id as
  documented. A picture fill has, after fill type bit 1, the mode (1 byte), brightness, contrast and
  effect, and the picture id (2 bytes). A picture without resolution information is tiled at its
  96 dpi size (a 383×449 px GIF → 287×337 pt, as on screen). DOCX has one background for the whole
  document, so it is laid in the header as a "behind text" box, and the border goes in `w:pgBorders`.
  Every document has this record, so the header is made only when there is a real background (an
  empty header pushes the body down).

WordCraft's side (0.10): of the space after one paragraph and the space before the next, only the
larger is kept (20 pt + 20 pt → 20 pt), so the previous paragraph's space after is added to the next
paragraph's space before. Vertical sections (`tbRl`) and text boxes holding tables are drawn.

Shape and table placement (2026-10-28, compared with Hancom's viewer):

- For a rotated object, `hp:sz` is the box around the rotated shape and `hp:curSz` the size before
  rotation (a red arrow: 9749×29837 / 29837×9749, 90°). WordCraft (0.11) doesn't draw rotated shapes
  that stick out of their frame, so polygons and curves are rotated point by point inside the frame.
- Picture record: border (colour, width, line type = the low 6 bits of the line attribute) and crop
  (left, top, right, bottom, in the picture's own coordinates). Without resolution information,
  picture coordinates = pixels × 75 (96 dpi). A mountain photo cut in two and placed side by side
  matched the screen.
- "Behind text / in front of text" tables: a floating DOCX table pushes text away, so the table is
  put in a text box that doesn't (a table used as a title bar).
- The outer margins (top and bottom) and the bottom vertical offset of a table in the text flow take
  space above and below the table (for example next line = 13784 + 158 + 2835 + 24878 + 141 = 41796).
  Only 10 pt or more becomes an empty paragraph. HWPX writes negative positions as unsigned 32-bit
  (4294967046 = −250).
- Cell gradients: DOCX cells have no gradients, so the average of the colours is used.

HWP check (stage 4, 2026-10-28): all 17 open, no broken characters, under a second per file. One or
two pages of each file were compared side by side with Hancom's viewer, which led to these fixes:

| Item | Seen on screen or in the file | Handling |
| --- | --- | --- |
| Picture captions | The caption of a picture placed as a character was lost | As a paragraph above or below the picture line. Picture, table and equation numbers are counted in document order |
| Memos | `MEMO_LIST` (memo number) + paragraph list below the section's last paragraph, and a body field "MEMO/…/number/…/author" | DOCX comments |
| Hyperlinks to other documents | `.\other.hwp`, `E:\…\other.hwp` | Relative path / file: link (document extensions only) |
| Automatic tab at the right edge | autoTabRight in the tab definition; the right-most text lines up with the column's right edge | A right tab at the end of that line's width (cell, column) |
| Column breaks in multi-column text | The cache restarts from 0 for each column. A column break in a paragraph defining new columns is ignored | In an n-column area only the n-th restart is a page; the others are column breaks |
| Documents without a line layout cache | The last-line-of-cell rule can't be used (table rows grow 1.5×) | Whether a cell is one line is estimated from the cell width and character count |
| Master pages | A paragraph list under the section definition. A "page" base position matches the paper | Behind text in the header, taking no space |
| Page border base | Attribute bit 0 = 1 means relative to the body (drawn 14 pt outside the body). The opposite of the document's "0 body / 1 paper" | A body-relative border becomes a rectangle in the header (WordCraft doesn't draw body-relative `w:pgBorders`) |
| Page number decoration | The last 2 bytes of the page number record '-' → viewer shows "- 1 -" | Decoration characters on both sides |
| Page background gradient | Type 2 = radial (bright in the middle) | `a:path circle` |
| "As character" tables that don't fit on one line | Stacked vertically. In a text box the last row was cut | A text box only when they fit in one line's width |

Result (2026-10-27): 15 of the 17 HWP match in page count. The 2 remaining differ by −6 and +1 (of 94
pages). The same fixes made 3 HWPX match as well.

- The −6 document: converted and compared section by section. The loss is in items whose explanation
  line wraps to two lines only in Hangul. The original font "한컴돋움" isn't installed and is drawn with
  Hamchorom Dotum, while the viewer uses its own wider font (the cause in section 4, "Remaining
  differences"). Not matched in conversion.
- The +1 document: a "wrap" table that fits the space left at the bottom of a page with 1.5 pt to
  spare is moved to the next page by WordCraft. Even shrinking rows by the table's position (−5.5 pt)
  doesn't stop it, and making it a flow table broke a similar document, so it stays.

## 6. Saving to HWPX (stage 5, 2026-10-28 onward)

Moved to Rust on 2026-10-28 (`crates/hwp/src/writer.rs`; text differences then worked like Python's
`difflib.SequenceMatcher`, since 2026-11-17 by the shortest edit script, section 1). Compared with
the Python `writer.py` (before the 2026-11-17 change): on 13 samples × 3 kinds of edit
(save unchanged / add text and a new paragraph / delete and join paragraphs and edit a cell) and many
edits in one long document, every entry of the resulting package and the report (including the list
of edits not written) were the same. Three `.hwpx` files opened and saved from the app opened
normally in Hancom's viewer (same page count as the original, edits visible). What changed in Rust:
entries are copied still compressed (Python recompressed them), and `hp:nbSpace` is compared as
U+00A0 like the converter (Python's `writer.py` compared it as a normal space and couldn't edit
paragraphs containing it). The reading converter's `hp:nbSpace` was also corrected to U+00A0.

No new published material was used. The only sources are the names used for reading
(`docs/hwp/owpml-reference.md`) and the original file's own XML.

Method: the original XML is not rebuilt; **only the text of changed paragraphs** is spliced into the
original text (`writer.rs`).

- When converting, the paragraph start tags of the original section XML are counted in order, and
  each DOCX paragraph gets a hidden bookmark `_hp_<section>_<ordinal>`. Saving finds the original
  paragraph by the same rule (counted on the raw text before `hp:switch` handling, so it can't drift).
  Further pieces of a paragraph (text between tables or objects) all carry `_hq_` with the same
  number, and each is compared with the piece at the same place (2026-11-16; before, all were
  compared with the first, so a paragraph holding two tables always read as edited).
- A table is found by the first source mark in its cells. A table whose first cell starts with a
  table shares that mark with the inner one, so it is found by walking out past the tables in
  between (2026-11-16; before, an edit of the outer table was written over the inner one).
- When saving, a "before" DOCX converted again from the original is compared with the edited DOCX,
  paragraph by paragraph. Text differences are applied to the original list of characters (inserted
  text takes the character shape of its neighbour), and the edited paragraph's `hp:linesegarray` is
  removed.
- Only "simple paragraphs", whose original text equals the converted text, are edited. Otherwise
  (fields, footnotes, overlaid characters and so on) they are not written and the user is told.
- The package is copied with its entry order and compression, and only changed sections are replaced.

WordCraft (0.11) save behaviour (measured):

| What was put in | After saving |
| --- | --- |
| Bookmarks inside paragraphs | Kept |
| `w14:paraId` | Lost (hence bookmarks) |
| Bookmarks between paragraphs (directly under the body) | Moved into the next paragraph → a paragraph holding only a table carries a `_ht_` mark, which is looked for in the table's first cell or a separator paragraph and moved back to the table's place |
| Custom properties (original path and hash, original font names) | Kept |
| Web link field codes | Become ordinary hyperlinks (same content) |

Check: all 13 HWPX samples opened in WordCraft and saved unedited → every file in the package has
the same content as the original (only the ZIP entries' time stamps are new). A file with edited
text, replace all, split, added and emptied paragraphs, edited table cells and edits next to
full-width spaces opened normally in Hancom's viewer (page count and content checked).

Pictures (2026-10-31): pictures in original paragraphs are matched in order with the document model
(by position when the count is the same, else by picture content). Size goes to `hp:sz`, `hp:curSz`
and `scaMatrix` (current/original size); a replaced picture is a new `hp:pic`, and a deleted one is
removed with its element. New pictures go in as `BinData/image<n>.<ext>` and are listed in
`content.hpf` (numbers after those in the package). Paragraphs whose pictures changed lose their line
layout (`hp:linesegarray`) so that Hangul lays them out again. A picture inserted in the middle of
text is put at the end of the paragraph, and the user is told. Pictures in an edited table are
written with their files. Check: the 13 HWPX samples saved unedited still give the same package
content. In a test document made in WordCraft, a resized picture and a picture added at the end of
another paragraph opened in Hancom's viewer at the new size (200 pt) and place.

## 6b. Writing new HWPX (2026-10-28 onward)

A document not opened from HWPX (new, .docx, .hwp) has no original, so section 6 can't be used: the
whole package is written from the document model (`wordcraft-doc`) (`crates/hwp/src/hwpx_out.rs`).
Element and attribute names come only from `owpml-reference.md`; no XML of Hancom-made files was
copied. Checked by opening the files in Hancom's viewer and comparing with WordCraft side by side
(black box).

| Item | Handling | Checked |
| --- | --- | --- |
| Package | `mimetype` (first entry, stored), `version.xml`, `META-INF/container.xml`, `Contents/content.hpf` (list and order), `header.xml`, `section<n>.xml` (one per section) | The viewer opens it |
| Paragraph margins | `hh:margin` in HWPUNIT, without `hp:switch` | Left 72 pt, first line 36 pt, hanging 36 pt, right 144 pt at the same places as WordCraft |
| Hanging indent | Hangul's left margin = start of the first line, negative `intent` (the reverse of reading) | As above |
| Line spacing | Multiple m → `PERCENT` m×120 (a Word line ≈ 1.2 × font size; Hangul's % is of the font size), exact → `FIXED`, at least → `AT_LEAST` | Line positions for double and exact 30 pt nearly the same as WordCraft |
| Page margins | DOCX top/bottom margin = Hangul top + header (the reading rule reversed) | Left 2.54 cm, landscape paper (`NARROWLY`) |
| Page breaks | Paragraphs are split at page (column) break characters, and the next paragraph gets `pageBreak`/`columnBreak` | Splits into two pages |
| Character shape | Size, bold, italic, underline, strikethrough, colour, highlight, superscript and subscript, width ratio, letter spacing, font (per-language list) | Viewer screen |

Stage 2, tables (2026-10-28):

| Item | Handling | Checked |
| --- | --- | --- |
| Table structure | Each cell has `hp:cellAddr` (column and row), `hp:cellSpan` (horizontal merge = DOCX gridSpan, vertical merge = the number of vMerge continuation rows); hidden cells are not written. A continued cell under a cell that doesn't start a merge (a row inserted inside a merge) joins that cell, as WordCraft draws it; skipped, it left a hole that made Hancom's viewer quit (2026-11-16, section 3) | Three table-heavy samples saved .hwp/.hwpx → WordCraft → new HWPX: same cell layout as the original in Hancom's viewer |
| Column widths, cell size | Sum of the table's grid widths; `hp:cellSz` height = row height + cell top and bottom margins (DOCX row height excludes margins: the reverse of the reading rule). Without a row height, estimated from text length | Before adding margins, rows were a little short; after, the table height matches the original |
| Borders and fill | Four sides (line type, nearest Hangul line width, colour) and background colour, decided as cell > table > table style, as `hh:borderFill` (identical ones shared) | Thick separators, background colours, header cell colours |
| Position | Tables in the text flow (`TOP_AND_BOTTOM`, paragraph-relative); centre/right alignment or indent as `horzAlign`/`horzOffset` | |

Cell diagonals are not in WordCraft's document model, so they are already lost when reading.

Stage 3, objects (2026-10-28):

| Item | Handling | Checked |
| --- | --- | --- |
| Headers and footers | A `hp:header`/`hp:footer` control in the first run of the section's first paragraph (both pages, or odd and even when they differ). An empty first page is `hp:pageHiding` | Footer "- 1 -", a test document's header |
| Page numbers and other fields | PAGE → `hp:autoNum numType="PAGE"`, NUMPAGES → `TOTAL_PAGE`, other fields as their displayed result text | Page numbers in the footer |
| Footnotes and endnotes | `hp:footNote`/`hp:endNote` at the reference (content paragraphs, number `autoNum` first). Per section `hp:footNotePr`/`hp:endNotePr` (number `DIGIT` + `)`, continuous numbering, footnotes per column and endnotes at document end, a 0.12 mm solid separator 144 pt long as in WordCraft). Name and value pairs checked in 18 sections of the HWPX samples | Note numbers and text shown at the page bottom. The separator didn't show without `footNotePr` and shows in both HWPX and HWP with it (2026-11-01) |
| Pictures | Picture files as `BinData/imageN.<ext>`, listed in `content.hpf`, `hp:pic` (size; crop as `imgClip`/`imgDim` in picture pixels × 75) | Background pictures and photos in samples, a test PNG |
| Text boxes and shapes | Boxes with text, rectangles and rounded rectangles as `hp:rect` (+ `hp:drawText`), ellipses `hp:ellipse`, lines `hp:line`; line colour and width, solid and gradient fills | A test text box, a gradient box in a sample |
| Object position | Relative to paper (PAPER), page (PAGE), column or paragraph, by alignment or offset. As-character (inline) is `treatAsChar` | The two rules below checked in the viewer |

Placement rules checked in the viewer (a test document's box opened with only its base changed):

- The horizontal and vertical offsets of a body object are the position of the box **including the
  object's outer margins**. In DOCX they are the object's own position, so the outer margin (9 pt by
  default) is subtracted.
- Objects in headers and footers are measured from the **top left of the header (footer) area**, even
  when they say paper or page. So paper offsets are converted to the area (a page border box comes
  back to its original place).
- Page-relative objects get `flowWithText="0"` (not held inside the body area).
- (2026-11-01) The value 0 of the horizontal base (bits 8 to 9) in an HWP object's common attributes
  means the paper. The published document gives both 0 and 1 as "page", but in one test document a
  picture and lines with value 0 (offsets 0 and 186 mm) start from the left edge of the paper in
  Hancom's viewer. Writing uses paper = 0, as before.
- (2026-11-01) Horizontal and vertical lines have a current height or width of 0. The original size
  (100×100) is only the range of the end point coordinates, so lines use the current size even when
  one side is 0 (before, they were drawn as 1 pt lines and didn't show). On resize, a side that is 0
  stays 0. WordCraft's size command sets every object to at least 1 pt, so a horizontal line with
  end points (0,0) → (100,100) was saved 1 pt slanted. After the fix, a file with that document's
  first line shortened to 200 pt shows it level (on the same screen row) in Hancom's viewer.
- A continuous section (only the number of columns changes, on the same page) written as a new
  section starts a new page in Hangul, so it is written on in the previous section with only
  `hp:colPr` in that paragraph (53 pages → 8, as the original).

- Numbered and bulleted lists (2026-10-30): written as the paragraph shape's `hh:heading` (type
  `NUMBER`/`BULLET`, level from 0) and `hh:numbering`'s `hh:paraHead` (level from 1, number format like
  `^1.`), and `hh:bullet` (`char`). With `autoIndent="0"` and the paragraph's left margin and hanging
  indent set to WordCraft's list level indents, the number position and second-line start in the
  viewer match WordCraft (checked with two number levels, two bullet levels and parenthesised
  numbers).
- Only the paragraph styles in use are written to `hh:styles`, with their names, linked by
  `styleIDRef`. 0 is always 바탕글 (Normal). Reopening gives back the same names.
- A new table in an original HWPX needs the table style "Table Grid". Converted documents didn't have
  it, so WordCraft drew them without borders; on open, WordCraft's default table styles (Table Normal,
  Table Grid) are filled in (`read_hangul` in `engine`).

- Triangles, right triangles, diamonds, arrows (up, down, left, right), stars and hearts are written
  as the points (`hc:pt`) of `hp:polygon`; connectors as `hp:line`; the flowchart OR as `hp:ellipse`
  (2026-10-31, shape and colour checked in the viewer).
- Groups (2026-10-31) are `hp:container`. Decided from **only the structure** of one group in a sample
  (tag names and order, relations between values): the group's own size elements (`offset` to
  `renderingInfo`) → children → `sz`, `pos`, `outMargin`. Children have no `sz` or `pos` and have
  `groupLevel` 1, and a child's `hp:offset` equals the translation values (e3, e6) of `transMatrix`.
  Measured in the viewer: a group's scale (`orgSz` ≠ `curSz`, `scaMatrix`) applies to its children's
  **size** but not their **position** (offset) (in a group enlarged 1.5×, an ellipse stayed at
  160 pt). So groups are written at scale 1, with the scale already multiplied into the children's
  position and size. The viewer and WordCraft then draw them at the same place (rectangle 180 pt,
  ellipse 240 pt, picture 120 pt checked).
- Freehand curves (including ink) are written per path: open paths as `hp:curve` with
  `hp:seg type="LINE"` (the curve segment type of the published HWP 5.0 document, the same value as
  reading), closed paths as `hp:polygon` (first point repeated at the end). Several paths are wrapped
  in one group. Line shape and colour checked in the viewer.

- Charts, SmartArt and ink (2026-10-31): Hancom's chart format isn't in any published material, so it
  isn't written. Instead, just before saving, the engine draws what WordCraft shows on screen as a PNG
  (2 pixels per pt) and replaces the object with a picture of the same size, place and alternative
  text (`engine::io_ext::graphics_as_pictures`). From there it is a picture. A chart newly inserted in
  an original HWPX takes the same path (reading Hangul never creates charts, so it can't be confused
  with an original chart). Bar and pie chart shapes and text checked in the viewer.

- Memos (2026-11-01). The only sources are the names in the reference (`hp:subList` and
  `hp:parameters` of `hp:fieldBegin`, `hh:memoProperties`/`hh:memoPr`, `secPr`'s `memoShapeIDRef`) and
  the memo field of one HWP test document. The HWPX samples have no memos. The sample's field was type
  `%unk` with the command string
  `MEMO/65535/<memo number>/<low 32 bits of time>/<high 32 bits of time>/<author>/\;;`. Tried in the
  viewer:
  - Field `type="MEMO"` with the memo paragraphs in the field's `hp:subList` shows the author and text
    in the memo pane. The same with `type="UNKNOWN"` made the viewer fail to open the file and close
    (twice).
  - With `hh:memoProperties` (one shape) and `memoShapeIDRef="1"` it shows in that shape; without
    them, in Hancom's default shape.
  - The time is a Windows FILETIME (100 ns since 1601, UTC). 2025-01-02 03:04 UTC showed in the viewer
    as 2025/01/02 12:04 (Korean time).
  Written: a field start at the memo's start (with the memo paragraphs) and a field end at its end.
  The memo range is kept within one paragraph (spanning was not tried, so it closes at the end of the
  start paragraph). `\`, `/` and `;` in the author are escaped with `\`, and split the same way when
  reading. (Limits of this first version; replies, memos in notes and headers, and memos in new .hwp
  were solved in the 2026-11-01 items below.)

- (2026-11-01) A memo range spanning paragraphs closes at its end (a memo starting and ending in
  different paragraphs checked in the viewer). It doesn't cross out of its text flow (table cell,
  note, text box) or section. Hangul memos have no replies, so a reply is added inside the original
  memo as a line "↳ author" followed by its text. The paragraph shape's `breakNonLatinWord` is written
  as read from a Hangul document (`KEEP_WORD` for other documents).
- (2026-11-01) Memos in a new .hwp: in the body a field start `[3,'me','%%',0,0,0,0,3]` and end
  `[4,'em','%',0xFF01,0x00FF,number,0,4]`, control `%unk` (attribute 0x8001, 1 byte, command string,
  instance id, memo number), and below the section's last paragraph a MEMO_LIST (number), a 16-byte
  list header and the paragraphs. Structure checked in one HWP test document. The memo shape (DocInfo
  MEMO_SHAPE) is not written (the viewer uses its default). On reading, the time in the command string
  (the two FILETIME parts) becomes the memo time.

Lost (the user is told on save): content of a first-page-only header (Hangul has no first-page-only
header).

## 6c. Saving to HWP (2026-10-30 onward)

A document opened from `.hwp` is saved to `.hwp`. Reading rebuilds the records as an HWPX-shaped
element tree, so that tree is written as XML, the HWPX save (section 6) finds the changed paragraphs,
and the result is turned back into records. Each paragraph carries its original PARA_HEADER record
number.

| Rule | Evidence |
| --- | --- |
| Unchanged records keep their original bytes; only changed section streams are replaced in the compound file (recompressed if the file is compressed) | Reading records and writing them back gives byte-identical files for all 17 samples (test `records_round_trip`) |
| The top bit (0x80000000) of a paragraph's character count means "last paragraph of a list (section, cell, text box …)" | No meaning in the published document. Holds for every paragraph of the 17 samples |
| A paragraph with only the paragraph end mark has no PARA_TEXT | Empty paragraphs in samples |
| An edited paragraph gets new PARA_TEXT and PARA_CHAR_SHAPE and loses its line layout (PARA_LINE_SEG) and range tags. Control characters (section and column settings, tables, pictures) stay where they were relative to the text | Real files (5.1.1.0) without line layout exist. The viewer lays them out again |
| A section's memos (MEMO_LIST and its paragraphs) must be below the section's last paragraph. When a new paragraph is added after the last one, the memos move under the new last paragraph | Otherwise Hancom's viewer can't open the file (a sample with memos) |
| Paragraphs with a section definition (secd) or memos are not deleted (the user is told). (2026-11-17) Such a paragraph is kept whole, with the paragraphs of the tables and text boxes in it | Deleting them breaks the document. Keeping the paragraph but deleting its table's cell paragraphs left a cell with none: a document with its first 773 blocks deleted then opened blank in Hancom's viewer (3 of 3); kept whole, it opens (3 of 3) |
| Text split between objects and fields is handled piece by piece: an edit within one piece rewrites only that piece; an edit across an object is not written and the user is told. Text typed at the boundary goes to the previous piece (2026-10-31; HWPX save the same) | Reading gives each control its own run, and invisible controls keep their place as an empty `hp:ctrl`. When the count doesn't match the record's text pieces (invisible inline controls), nothing is written |
| (Stage 2) New character and paragraph shapes copy the original shape record, change only the values that differ in the XML, go after the last record of the same kind, and increase the count in ID_MAPPINGS. Values reading doesn't move to XML (shadow colour, emphasis marks, border spacing and so on) stay as they were | The same positions as reading (paragraph margins stored doubled, the underline shape table shifted by one) |
| (Stage 2) A new font is a FACE_NAME with only a name, at the end of its language group | Opens in the viewer after changing the Latin font |
| (Stage 2) New border/fill: no fill 40 bytes, solid 53 bytes (fill type, background colour, pattern colour, no pattern −1, extra length 0, 1 byte) | BORDER_FILL records in samples |
| (Stage 2) Tables: control character `[11, 'tbl ', 0×4, 11]`, CTRL_HEADER 46 bytes, TABLE (cell count per row), per cell a 47-byte LIST_HEADER (text width and reserved bytes after the cell attributes) followed by the cell's paragraphs | Table records in samples. A rewritten table keeps the original table's position settings. A new table copies the document's first table, or is built from scratch when there is none |
| (Stage 2) A table that can't be rewritten (cells holding fields or objects) keeps all its original records | If the original cell paragraphs are lost, the viewer can't open the file |
| (2026-11-01) New pictures: a BIN_DATA (embedded 1, storage id, extension) after the last BIN_DATA in DocInfo, and the picture count in ID_MAPPINGS increased. The stream is `BinData/BIN%04X.<ext>` (storage id after the largest the file uses; compressed if the file is). A `BinData` storage is created if missing. A picture record points to BIN_DATA by its record **order** (from 1) | In one test document the BIN_DATA records have storage ids 3, 2 in that order and there are 2, so pointing by id shows the wrong picture. Reading also looks up by order. Pictures and charts added to another test document without pictures checked in the viewer |
| (2026-11-01) An original paragraph whose pictures changed (insert, delete, resize, replace) with unchanged text has only its pictures changed between the original records: a deleted picture loses its control character and records, a new picture goes right after the previous picture (else before the text or at the end of the paragraph), and other controls (section settings, columns, fields, tables) keep their original records. When the text was edited too, only paragraphs with text and pictures alone are rebuilt from XML | Inserting a picture into, and resizing and deleting the background picture of, one test document's first paragraph (section settings + background picture) checked in the viewer |
| (2026-11-01) Original paragraphs with both pictures and text edited are written the same way: the edited text goes into each text piece (the text between controls) when the piece count is the same. New shapes and text boxes are written in new paragraphs and in text paragraphs without controls (the paragraph rebuilt from XML). HWPX adds and removes shapes like pictures, as the paragraph's list of drawing objects | An ellipse and a text box added to one HWP and one HWPX test document checked in the viewer |
| (2026-11-01) A resized picture doesn't get new records: only the control's width and height, and the object element's current size, rotation centre and first scale matrix, are changed in the original records. The control's width and height base bits (15 to 17, 18 to 19) are set to absolute | With newly built picture records that page looked entirely blank. The original background picture's height 4267 was 42.67% of the page height (height base = page); writing only the absolute value read it as 90% and stretched it |
| (2026-11-01) In a rewritten table, original cells with controls (fields, pictures, form objects, bookmarks and so on) that stay with the same content in the edited table keep their original paragraph records. "Same content" means the text of each paragraph (except an empty paragraph at the cell's end, and including the text of nested tables), the number of pictures, shapes and form objects (`gso `, `form`) and the number of cells in nested tables. Cells are paired by the longest common subsequence of the original and new cell order (adding a row doesn't shift other cells). Controls of unpaired original cells are lost, and the user is told. (2026-11-16) Only cells with controls are paired, so empty cells can't outnumber them. Three things read differently on the two sides and are made the same: a paragraph with text and a table reads as the text and then the table alone (a paragraph with no text that holds a table gives no text entry); a fixed-width space (control 31) reads as U+3000; a table on a text line reads as an invisible text box (no outline, fill or margins) holding it | A row added to every table of the 17 samples and saved: picture, shape and group counts and table places all as in the original, and Hancom's viewer opens them (test `hwp_tables_keep_objects`). The 2026-11-16 rules: the same test over the 10 HWP documents of the current set, with no edit left unwritten; not yet checked in the viewer |
| (2026-11-01) A table caption is the list header and paragraphs before the TABLE record (below the control, before TABLE), and is kept when the table is rewritten | Before, captions were dropped and their paragraphs disappeared (a sample table with a caption) |

Check (2026-10-30): the 17 samples × 2 kinds of edit (text added at the start and a new paragraph at
the end / paragraph deleted, joined and a cell edited) opened and edited with the app's engine and
saved as `.hwp`. All open in our reader and in Hancom's viewer with the edits visible. Documents that
grew by a page were cases where the new paragraph at the end or a longer cell text moved to the next
page; editing the same cell with text of the same length kept the original page count.

Check (2026-10-31, stage 2): the same samples with formatting edits (bold, size, colour, centre,
indent, italic, underline), a Latin font change, and table edits (row added and a new table at the
end / table deleted) saved as `.hwp`. All open in Hancom's viewer with the formatting, new rows and
new tables visible. The stage 1 edits run again gave the same result.

## 6d. Writing new HWP (2026-10-31 onward)

A document not opened from HWP is first made as a new HWPX (6b), and its header and section XML are
turned into HWP records (sharing the encoder of 6c). All were checked to open in Hancom's viewer
looking the same as the `.hwpx` result.

| Rule | Evidence |
| --- | --- |
| File header 256 bytes: signature, version 5.0.4.0, compression flag. DocInfo, section and picture streams compressed with raw deflate | Section 3.2 of the published document |
| DocInfo: document properties (section count, start numbers 1), ID_MAPPINGS (18 counts), pictures, fonts (at least one per language), border/fill, character shapes (74 bytes), tabs (8 bytes), numbering, bullets, paragraph shapes (54 bytes, the same value in the old line spacing field too), styles (name, English name, kind, next, language 1042, paragraph and character shape, 2 bytes 0) | Record sizes and order in samples |
| First paragraph of a section: control characters `[2,'secd',…]` and `[2,'cold',…]`, column break kinds 0x01 and 0x02. Below the 47-byte section definition, the 40-byte page settings, two 28-byte footnote/endnote shapes (the file's defaults), three 14-byte page border/fills | First paragraphs in samples |
| Common object attributes of tables and pictures: width base = absolute (4), height base = absolute (2), numbering kind (picture 1, table 2). TABLE attribute bit 26 | Without them Hancom reads the width relative to the paper and draws columns twice as wide |
| Numbering: 7 levels (12-byte head + a string like `^1.`), start number, per-level start numbers. Bullets: 12-byte head + character + 9 zero bytes (23 bytes) | NUMBERING and BULLET in samples; the bullet fields as in the published document's table 42 (section 3, test set) |
| Headers and footers: control `head`/`foot` (code 16; applies to 0 both, 1 even, 2 odd pages), 34-byte list header (text area width and height). Page numbers: `atno` (code 18, 16 bytes) | Samples, tables 140 to 143 of the published document |
| Footnotes and endnotes: `fn  `/`en  ` (code 17) + 8 bytes, 8-byte list header | Published document 4.3.10.4 (not in samples; checked in the viewer) |
| Pictures: BIN_DATA (embedded 1, number, extension) + `BinData/BINxxxx.<ext>`, control `gso ` (code 11), object element (`$pic` twice, size, rotation centre, three matrices) 196 bytes, picture record 94 bytes (four corners of the picture, crop, inner margins, brightness and contrast, BinData number) | Picture records in samples |
| (2026-11-01) Drawing objects (text box, rectangle, ellipse, line, polygon, curve, group): control `gso ` (code 11) 46 bytes (no numbering kind); the object element has its id (`$rec`, `$ell`, `$lin`, `$pol`, `$cur`, `$con`; once for children of a group) followed by the attributes of table 83 of the published document and the rendering matrices (one more scale and rotation pair per group level; identity matrices in this writer) | Published document 4.3.9.2, record lengths in samples (252 bytes for a top-level rectangle, and so on) |
| The object element's line information is 13 bytes (colour 4, width 4, attributes 4, outline 1); fill is a 4-byte type, plus 12 bytes for solid (background colour, pattern colour, no pattern −1) and 1 byte, then 4 bytes of extra length, a 16-byte shadow (kind, colour, x and y offset), a 4-byte instance id, and 2 bytes | The published document gives width as 2 bytes and has no tail. Matched to the tail lengths of object elements in samples (26 with no fill, 27 solid) |
| Geometry: rectangle 33 bytes (curvature 1 + x,y pairs of four corners), ellipse 60 bytes (attributes + centre, axis 1, axis 2, start, end, start 2, end 2), line 20 bytes (start, end + 4 bytes of attributes), polygons and curves a 4-byte point count + x,y pairs (curves with segment type bytes) + 4 bytes | The published document has all x then all y and a 2-byte point count. Samples have pairs and 4 bytes |
| Text boxes: below the object element a 33-byte list header (paragraph count, attributes (vertical alignment), four inner margins, maximum text width, 13 zero bytes) and the paragraphs, then the geometry record. Bit 24 of the object element's flip field is set | Without bit 24 the viewer can't open the file. All 32 rectangles with text in samples have bit 24 set, and most objects without text don't. Opens without line layout records (checked in the viewer) |
| Groups: at the end of the object element, a 2-byte child count, the child ids and the instance id. The children's object elements below it | Group records in samples |

Check (2026-11-01): a new document with a text box, ellipse, triangle, line, group (rectangle +
circle) and ink saved as `.hwp` opens in Hancom's viewer looking the same as the `.hwpx` result. The
cause was narrowed down with one file per shape (only the text box didn't open; fixed by bit 24).

Charts and SmartArt are written as pictures, as in new HWPX. Memos are written as at the end of
section 6b.

Editing pictures and shapes in original .hwp paragraphs (2026-11-01):

- The paragraph's original objects (`gso ` controls: kind, BinData number, current size) are paired
  in order with the objects of the edited paragraph. Pictures from the file point to BinData by number
  (new pictures are `imageN`); shapes read from the file have no instance id (the writer gives new
  shapes one). Paired objects keep their original records, unpaired original objects lose theirs, and
  new objects go after the previous remaining object (else before the text or at the paragraph's end).
- When removing a picture leaves only deleted objects between two text pieces, they count as one
  piece and the edited text is fitted to it.
- A resized object has its records edited in place: the control's width and height (bytes 16 and 20)
  with absolute bases (bits 15 to 17 = 4, 18 to 19 = 2), the object element's current size (28, 32),
  rotation centre (42, 46), and e1 and e5 of the scale matrix (100, 132). Object element ids are
  stored reversed (`cip$` and so on). Shapes are compared by their content without size, so a shape
  whose size alone changed isn't made new.
- Paragraphs with tables are still rebuilt or left as they were, and the user is told.
- Check: a file with a new ellipse in one test document's first paragraph (the paragraph with the
  section definition) opens in Hancom's viewer. The 17 samples saved unedited are byte-identical.

Memos in notes and headers (2026-11-01): written as the same field as in the body, with the memo text
gathered in the MEMO_LIST at the section's end. Hancom's viewer opens the file but shows only body
memos as balloons. WordCraft reads all three memos back.

## 7. Still to check (guesses)

| Item | Current handling | Where |
| --- | --- | --- |
| Hyperlink address | Before the first `;` of `stringParam name="Command"`, with `\:` unescaped | `hyperlink_url` in `convert.rs` |
| Numbering level | `hh:heading level` from 0, `hh:paraHead level` from 1 | `num_pr` in `convert.rs` |
| Tab position unit | HWPUNIT | `ppr_inner` in `convert.rs` |
