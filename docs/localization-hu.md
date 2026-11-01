# Hungarian interface

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-31 · **Change:** trivial (a verbal prefix with two suffixes, an ending after a sign, long compounds with -hoz and -i, word pairs listed with a space, fused adjectives before a hyphen; previously trivial: abbreviations with their period, no NOSUGGEST suggestions, fewer false comma and space marks; previously minor: Hungarian proofing: spelling, suggestions, grammar, hyphenation; previously trivial: the catalog moved to storytold/craft-i18n, `locales/hu/wordcraft.tsv`; previously trivial: interface-language tests: a missing translation never fails a test, `cargo xtask i18n-report` in `cargo xtask ci` prints the strings each language lacks, and lookup and template tests use inline fixture catalogs or the pseudo-language; previously trivial: the interface audit lists missing Hungarian entries instead of failing; previously minor: Hungarian interface catalog and regression checks) · **Target:** WordCraft 0.5.0

Choose **File › Options › Interface language › Magyar**, or follow a Hungarian system display
language (`hu-HU`, `hu_HU.UTF-8`). The catalog code is `hu`; `ui.language` with `{"value":"hu"}`
selects it explicitly, and `WORDCRAFT_LANGUAGE=hu-HU` does the same for a headless screenshot or a
launch while the preference is `auto`. The choice is saved with the other interface settings.
Document text, command ids, CLI/MCP/control responses and file paths do not change.

craft-i18n's `locales/hu/wordcraft.tsv` (storytold/craft-i18n; it was `crates/ui-egui/src/i18n/hu.tsv`) has 2,419 entries: every key of the Russian catalog (built-in style
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
order, saving and restoring `hu`, unchanged document content and the glyphs `ÁáÉéÍíÓóÖöŐőÚúÜüŰű` in
the bundled Inter (Regular, Medium, SemiBold) and JetBrains Mono. The shared checks (catalog validation and the missing-translation report, `cargo xtask i18n-report`)
include Hungarian automatically.

A native-speaker review of the catalog is welcome.

## Proofing

Text in Hungarian (`hu`, `hu-HU`) is proofed by `crates/proof/src/hu`. A Hungarian interface
writes new documents in Hungarian.

- **Spelling** against the Hungarian Hunspell dictionary (Magyar Ispell, `hu_HU.aff`/`.dic`)
  installed on the system: an OS package (`hunspell-hu`), Homebrew, LibreOffice, or the folder set
  in File › Options › Proofing › Dictionaries folder. Without it Hungarian text has no spelling
  marks. The affix engine reads the dictionary's flag aliases, compounding rules and word breaks,
  so compounds (`almafa`, `házkulcsban`), the 6–3 rule for long ones, and hyphenated forms
  (`SMS-ben`, `Petőfi-vers`) are checked as Hunspell checks them, a verbal prefix with two suffixes
  too (`megrendelések`, `díjbekérőt`), and a forbidden word stays wrong with endings
  (`megfedésemben`). Long compounds count their syllables for the 6–3 rule as Hunspell does
  (`adóvégrehajtáshoz`, `nyugdíjbiztosítási`). A word that splits into two words of its own is
  a compound only as those two (`műanyagcső` is wrong, `műanyagcsőgyár` right; before a hyphen
  it may still be, `közlésó-`), and a stem the dictionary lists with a space keeps its parts
  from compounding (`vasbeton fal`: `vasbetonfal` is wrong, `vasbetonfalelem` right). Before a
  hyphen a compound may start with a fused adjective (`forgalmiadó-hiány`) or a first part
  with a suffix (`jövedékiadó-fizetési`), and the 6–3 rule doesn't apply there; the last two
  parts may not be one stem twice, its forms included (`házházban`, `házbanház-`). Against
  labels from the Hunspell command line the results agree on 99.9% of 31,248 test words and of
  the 49,742 most frequent Hungarian words. Abbreviations the dictionary lists with their period
  (`pl.`, `stb.`, `kb.`) are correct with it and wrong without it, and take their endings after a
  hyphen (`Kft.-ben`, `szept.-ben`). An ending after a sign is checked with the sign, as the
  dictionary lists it (`27%-os`, `§-ában`, `5 °C-on`, `€-ért`).
- **Suggestions**: missing accents first (`hazban` → `házban`), the dictionary's confusion table
  (`hejesírás` → `helyesírás`), a missing space (`nemis` → `nem is`), one typo, then a stem scan.
  Words the dictionary marks `NOSUGGEST` or `SUBSTANDARD`, and compound-only parts, are never
  suggested; an abbreviation typed without its period gets it (`pl` → `pl.`).
- **Grammar**, with Hungarian messages: repeated word, space before punctuation, double space,
  sentence capital (knowing `pl.`, `stb.`, the abbreviations the dictionary lists with a period,
  ordinals and dates), a missing comma before `hogy`,
  `mert`, `hanem`, `de`, `ha` and the relative pronouns, `a` before a vowel (`az alma`), a capital
  month after a year, doubled punctuation. `de facto`, `de jure` and `de` after a name
  (`Charles de Gaulle`, `Jean de la Fontaine`) take no comma, words that only start like a
  relative pronoun (`aminosav`) open no clause, and a space before a file extension (`a .docx
  fájl`, `.NET`) is no stray space.
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
| 2026-10-31 | trivial | Before a hyphen a compound may start with a fused adjective (`forgalmiadó-hiány`) or a suffixed first part (`jövedékiadó-fizetési`, `jövedékiadóház-ügy`) and the 6–3 rule doesn't apply; these were marked. The last two parts may not be one stem twice, its forms included (`házházban`) |
| 2026-10-31 | trivial | Compounds as Hunspell checks them: a word that splits into two words of its own is a compound only as those two (`műanyagcső`, `darabothaj`), a stem with a space keeps its parts from compounding (`vasbetonfal`, `féláron`, `hajnyírógép`), and a forbidden word ends a compound (`háztanárúr`); these were accepted |
| 2026-10-30 | trivial | Long compounds count syllables for the 6–3 rule as Hunspell does: three-word compounds ending in `-hoz` or `-i` (`adóvégrehajtáshoz`, `nyugdíjbiztosítási`, `leltárértékelésiben`) were marked |
| 2026-10-30 | trivial | An ending after a sign is checked with it (`27%-os`, `§-ával`, `°C-on`): it was marked alone |
| 2026-10-30 | trivial | A verbal prefix with two suffixes is a word (`megrendelések`, `visszaigazolások`), its cross products as the Hunspell command line licenses them; a forbidden stem found with fewer affixes still makes the word wrong (`megfedésemben`) |
| 2026-10-23 | trivial | The catalog moved to storytold/craft-i18n (`locales/hu/wordcraft.tsv`) |
| 2026-10-23 | trivial | Lookup, context, template, style-name and command-search tests use inline fixture catalogs or the pseudo-language; tests that asserted one language's catalog wording removed |
| 2026-10-23 | trivial | Interface-language tests: a missing translation falls back to English and never fails a test; `cargo xtask i18n-report` (part of `cargo xtask ci`) prints the strings each language lacks; single-language coverage tests removed |
| 2026-10-23 | trivial | The interface audit lists Hungarian strings the catalog lacks; a missing entry doesn't fail the audit |
| 2026-10-21 | trivial | Review fixes: abbreviations checked with their period and hyphenated endings, NOSUGGEST and compound-only words never suggested, `pl` → `pl.`, no comma asked before `de facto` or after a name, relative pronouns told by their endings, a space before `.docx` left alone; 99.8% agreement |
| 2026-10-15 | minor | Hungarian proofing: spelling with the installed `hu_HU` dictionary (compounds, word breaks), suggestions, grammar rules, hyphenation with the installed patterns or the syllable rules; the dictionary licence |
| 2026-10-11 | minor | Hungarian catalog (2,419 entries: built-in style names, equation gallery, citation styles), conventions, terminology, scope and regression checks |
