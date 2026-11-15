# Finnish interface

> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** minor (Finnish interface catalog and regression checks) · **Target:** WordCraft 0.7.0

Choose **File › Options › Interface language › Suomi**, or follow a Finnish system display
language (`fi-FI`, `fi_FI.UTF-8`). The catalog code is `fi`; `ui.language` with `{"value":"fi"}`
selects it explicitly, and `WORDCRAFT_LANGUAGE=fi-FI` does the same for a headless screenshot or a
launch while the preference is `auto`. The choice is saved with the other interface settings.
Document text, command ids, CLI/MCP/control responses and file paths do not change.

`crates/ui-egui/src/i18n/fi.tsv` has 2,620 entries: every key of every other catalog, plus four
regional German proofing strings (`German (Austria)`, `German (Switzerland)`, the proofing-language
hint and the missing-dictionary message) that the other catalogs do not carry yet. It is written
from the meaning of the English labels; no other product's Finnish interface, string table or
terminology database was consulted, and no other catalog was used as a source. The contribution is
an original translation by @jernbole-dot (#2740), pending native-speaker review. Placeholders, `\n` breaks in ribbon labels
and dialog ellipses are kept, and strings added later fall back to English until they are
translated.

## Conventions

- Commands are imperatives (`Tallenna`, `Lisää taulukko`); messages address the reader as `sinä`
  (`Voit vaihtaa sen…`). Labels use sentence case; only the first word is capitalised.
- Counts use count-neutral wording: `Sanoja: {words}` reads right for `0`, `1`, `21` and every
  other number, so the catalog needs no plural forms.
- Decimal comma in fixed labels (`Riviväli 1,5`) and Finnish quotation marks (`”…”`).
- Product, format and technology names stay as they are: WordCraft, PDF, DOCX, Markdown, MCP,
  Zotero, SmartArt, WordArt, the Craft theme, paper sizes (Letter, Legal …) and citation styles
  (APA, MLA …).

## Terminology

| Source | Finnish |
|---|---|
| File / Home / Insert / Draw / Design / Layout | Tiedosto / Aloitus / Lisää / Piirrä / Rakenne / Asettelu |
| References / Mailings / Review / View / Help | Viittaukset / Postitukset / Tarkista / Näytä / Ohje |
| Font / Paragraph / Styles | Fontti / Kappale / Tyylit |
| Header / Footer / Heading | Ylätunniste / Alatunniste / Otsikko |
| Table of Contents / Index / Bibliography | Sisällysluettelo / Hakemisto / Lähdeluettelo |
| Comment / Track Changes / Accept / Reject | Kommentti / Muutosten seuranta / Hyväksy / Hylkää |
| Mail Merge / Recipients / Merge Field | Joukkokirje / Vastaanottajat / Yhdistämiskenttä |
| Columns / Section / Section Break | Sarakkeet / Osa / Osanvaihto |
| Undo / Redo / Repeat | Kumoa / Tee uudelleen / Toista |
| Ribbon / Pane / Quick Access Toolbar | Valintanauha / Ruutu / Pikatyökalurivi |
| Proofing / Word Count | Oikoluku / Sanamäärä |
| Page {page} of {pages} | Sivu {page}/{pages} |
| {words} words | Sanoja: {words} |

## Verification and remaining scope

`cargo test -p wordcraft-ui-egui --lib i18n::tests` covers Finnish locale tags (`fi`, `fi-FI`,
`fi_FI.UTF-8`, `FI-fi`) and preference order, saving and restoring `fi`, unchanged document content,
every key of every other catalog, no catalog errors, the command-search example, page and count
wording, English fallback, and the letters `ÄÖÅäöåŠšŽž` plus every other accented letter the catalog
uses in the bundled Inter (Regular, Medium, SemiBold) and JetBrains Mono. The shared catalog checks
(validation, every tab and command translated) include Finnish automatically, and the on-demand
interface audit (`untranslated_interface_text`) checks Finnish alongside the other complete
catalogs.

Spelling, grammar and hyphenation stay English-only: this is an interface translation, not Finnish
proofing. The separate Voikko contribution (#1155) would add Finnish proofing; when it lands,
`editing_lang` can become `fi-FI` so new documents are written in Finnish. Until then new documents
keep WordCraft's default, US English. A native-speaker review of the catalog is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-18 | minor | Finnish catalog (2,620 entries: every key of every other catalog plus the regional German proofing strings), conventions, terminology, scope and regression checks (#2740) |
