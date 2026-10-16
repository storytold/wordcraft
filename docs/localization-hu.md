# Hungarian interface

> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-15 · **Change:** minor (Hungarian proofing: spelling, suggestions, grammar, hyphenation; previously minor: Hungarian interface catalog and regression checks) · **Target:** WordCraft 0.5.0

Choose **File › Options › Interface language › Magyar**, or follow a Hungarian system display
language (`hu-HU`, `hu_HU.UTF-8`). The catalog code is `hu`; `ui.language` with `{"value":"hu"}`
selects it explicitly, and `WORDCRAFT_LANGUAGE=hu-HU` does the same for a headless screenshot or a
launch while the preference is `auto`. The choice is saved with the other interface settings.
Document text, command ids, CLI/MCP/control responses and file paths do not change.

`crates/ui-egui/src/i18n/hu.tsv` has 2,419 entries: every key of the Russian catalog (built-in style
names, equation gallery labels, paper sizes, watermarks and greetings included) plus the citation
style names. It is written from the meaning of the English labels; no proprietary translation resources were consulted. The catalog
format has no plural forms, and Hungarian doesn't need them: a noun after a number stays singular
(`0 szó`, `1 szó`, `21 szó`). Placeholders, `\n` breaks in ribbon labels and dialog ellipses are
kept, and strings added later fall back to English until they are translated.

## Conventions

- Commands are nouns (`Mentés`, `Másolás`, `Beillesztés`); messages address the user with the
  informal second person (`Mentsd a dokumentumot`).
- Only the first word of a label is capitalised, as Hungarian spelling requires
  (`Összes módosítás elfogadása`).
- Decimal comma (`1,5-es sorköz`) and Hungarian quotation marks („…”).
- A placeholder never takes a case ending, because the value is not known when the text is
  written. The wording puts it after a colon or in parentheses instead
  (`Undo {action}` → `Visszavonás: {action}`, `Couldn't open {path}: {error}` →
  `Nem sikerült megnyitni: {path} ({error})`).
- Text columns and table columns are different words: `Hasábok` (Layout › Columns, column break)
  and `Oszlopok` (table rows and columns).

## Terminology

| Source | Hungarian |
|---|---|
| Home / Insert / Draw / Design / Layout | Kezdőlap / Beszúrás / Rajzolás / Tervezés / Elrendezés |
| References / Mailings / Review / View / Help | Hivatkozások / Levelezés / Véleményezés / Nézet / Súgó |
| Font / Paragraph / Styles | Betűtípus / Bekezdés / Stílusok |
| Header / Footer / Heading | Élőfej / Élőláb / Címsor |
| Footnote / Endnote / Caption | Lábjegyzet / Végjegyzet / Felirat |
| Table of Contents / Index | Tartalomjegyzék / Tárgymutató |
| Citation / Bibliography | Forráshivatkozás / Irodalomjegyzék |
| Link / Hyperlink | Link |
| Track Changes / Accept / Reject | Változáskövetés / Elfogadás / Elutasítás |
| Comment / Markup | Megjegyzés / Jelölések |
| Mail Merge / Recipients / Merge Field | Körlevél / Címzettek / Egyesítési mező |
| Columns (text) / Columns (table) | Hasábok / Oszlopok |
| Section / Section Break | Szakasz / Szakasztörés |
| Undo / Redo / Repeat | Visszavonás / Újra / Ismétlés |
| Undo {action} | Visszavonás: {action} |
| Page {page} of {pages} | Oldal: {page}/{pages} |
| Ribbon / Pane / Quick Access Toolbar | Menüszalag / Panel / Gyorselérési eszköztár |
| Proofing / Word Count | Nyelvi ellenőrzés / Szavak száma |

Product and format names stay as they are (WordCraft, PDF, DOCX, Markdown, MCP, Zotero). The Font
dialog's preview sample is `AaBbYyZz ŐőŰű`, so the All Caps preview shows the Hungarian double
acute capitals.

## Verification and remaining scope

`cargo test -p wordcraft-ui-egui --lib i18n::tests` covers Hungarian locale tags and preference
order, saving and restoring `hu`, unchanged document content, every key of the Russian catalog,
built-in style names, placeholders, count wording, the font sample's capitals, English fallback,
and the glyphs `ÁáÉéÍíÓóÖöŐőÚúÜüŰű` in the bundled Inter (Regular, Medium, SemiBold) and JetBrains Mono. The shared catalog checks
(validation, every tab and command translated) include Hungarian automatically, and the on-demand
interface audit (`untranslated_interface_text`) checks Hungarian alongside Russian.

A native-speaker review of the catalog is welcome.

## Proofing

Text in Hungarian (`hu`, `hu-HU`) is proofed by `crates/proof/src/hu`. A Hungarian interface
writes new documents in Hungarian.

- **Spelling** against the Hungarian Hunspell dictionary (Magyar Ispell, `hu_HU.aff`/`.dic`)
  installed on the system: an OS package (`hunspell-hu`), Homebrew, LibreOffice, or the folder set
  in File › Options › Proofing › Dictionaries folder. Without it Hungarian text has no spelling
  marks. The affix engine reads the dictionary's flag aliases, compounding rules and word breaks,
  so compounds (`almafa`, `házkulcsban`), the 6–3 rule for long ones, and hyphenated forms
  (`SMS-ben`, `Petőfi-vers`) are checked as Hunspell checks them: on 31,248 test words labelled by
  the Hunspell command line, the results agree on 99.7%.
- **Suggestions**: missing accents first (`hazban` → `házban`), the dictionary's confusion table
  (`hejesírás` → `helyesírás`), a missing space (`nemis` → `nem is`), one typo, then a stem scan.
- **Grammar**, with Hungarian messages: repeated word, space before punctuation, double space,
  sentence capital (knowing `pl.`, `stb.`, ordinals and dates), a missing comma before `hogy`,
  `mert`, `hanem`, `de`, `ha` and the relative pronouns, `a` before a vowel (`az alma`), a capital
  month after a year, doubled punctuation.
- **Hyphenation** with the Hungarian patterns installed on the system (`hyph_hu_HU.dic`, the
  `hyphen-hu` package or LibreOffice), which know compound boundaries; without them, the
  Hungarian syllable rules. A doubled digraph (`ssz` in `asszony`) changes its spelling when
  broken (`asz-szony`); WordCraft doesn't break there.

### Licence of the dictionary data

The Hungarian dictionary (Magyar Ispell, by László Németh and Ferenc Godó) and the Hungarian
hyphenation patterns (by László Németh and contributors) are tri-licensed MPL 1.1 / GPL 2.0+ /
LGPL 2.1+. WordCraft uses them under the MPL 1.1 option, which permits use in open-source
projects under other licences such as MIT and Apache-2.0. Neither is bundled: WordCraft reads the
copies installed on the system. The test dictionaries in the repository are written for the tests
and contain no data from either.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | minor | Hungarian proofing: spelling with the installed `hu_HU` dictionary (compounds, word breaks), suggestions, grammar rules, hyphenation with the installed patterns or the syllable rules; the dictionary licence |
| 2026-10-11 | minor | Hungarian catalog (2,419 entries: built-in style names, equation gallery, citation styles), conventions, terminology, scope and regression checks |
