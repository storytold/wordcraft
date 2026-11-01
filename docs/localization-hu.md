# Hungarian interface

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-23 · **Change:** trivial (the catalog moved to storytold/craft-i18n, `locales/hu/wordcraft.tsv`; previously trivial: interface-language tests: a missing translation never fails a test, `cargo xtask i18n-report` in `cargo xtask ci` prints the strings each language lacks, and lookup and template tests use inline fixture catalogs or the pseudo-language; previously trivial: the interface audit lists missing Hungarian entries instead of failing; previously minor: Hungarian interface catalog and regression checks) · **Target:** WordCraft 0.5.0

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

Spelling, grammar and hyphenation stay English-only: this is an interface translation, not
Hungarian proofing. A native-speaker review of the catalog is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-23 | trivial | The catalog moved to storytold/craft-i18n (`locales/hu/wordcraft.tsv`) |
| 2026-10-23 | trivial | Lookup, context, template, style-name and command-search tests use inline fixture catalogs or the pseudo-language; tests that asserted one language's catalog wording removed |
| 2026-10-23 | trivial | Interface-language tests: a missing translation falls back to English and never fails a test; `cargo xtask i18n-report` (part of `cargo xtask ci`) prints the strings each language lacks; single-language coverage tests removed |
| 2026-10-23 | trivial | The interface audit lists Hungarian strings the catalog lacks; a missing entry doesn't fail the audit |
| 2026-10-11 | minor | Hungarian catalog (2,419 entries: built-in style names, equation gallery, citation styles), conventions, terminology, scope and regression checks |
