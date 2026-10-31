# Bosnian interface

> **Last reviewed:** 2026-10-29 · **Last updated:** 2026-10-29 · **Change:** major (first version) · **Target:** WordCraft 0.7.0

Choose **File › Options › Interface language › Bosanski**, or follow a Bosnian system display
language (`bs-BA`, `bs_BA.UTF-8`, `bs-Latn-BA`). The catalog code is `bs`; `ui.language` with
`{"value":"bs"}` selects it explicitly, and `WORDCRAFT_LANGUAGE=bs-BA` does the same for a headless
screenshot or a launch while the preference is `auto`. Document text, command ids, CLI/MCP/control
responses and file paths do not change.

The catalog is craft-i18n's `locales/bs/wordcraft.tsv` (storytold/craft-i18n, 2,508 entries) over the
shared `locales/bs/common.tsv`. Both are embedded from the
craft-i18n commit pinned in `craft-i18n.rev` (0722485…), which includes them. The
catalog is written from the meaning of the English labels; no proprietary translation resources were
consulted. It is an AI-assisted draft that a native speaker has not yet reviewed.

## Conventions

- Latin script, ijekavian, standard Bosnian vocabulary where the neighbouring languages differ:
  *historija*, *sačuvaj*, *tačka*, *hiljada*, *sedmica*, *datoteka*, *postavke*. Cyrillic Bosnian
  (`bs-Cyrl-*`) has no catalog and stays in English.
- Commands and buttons use the imperative (`Sačuvaj`, `Umetni fusnotu`, `Prihvati`); ribbon groups
  and panes are nouns (`Pasus`, `Stilovi`, `Navigacijsko okno`).
- Messages address the user with the polite plural (`Unesite lozinku`).
- Only the first word of a label is capitalised.
- `@plural` rows use Bosnian's three CLDR forms, `one|few|other` (`{n} stavka|{n} stavke|{n}
  stavki`); the `craft-i18n` crate picks the form from CLDR's rules.
- Quotation marks „…”; placeholders, a trailing `…` and the spaces around joined fragments match the
  English.

## Terminology

| Source | Bosnian |
|---|---|
| File / Home / Insert / Draw / Design / Layout | Datoteka / Početna / Umetni / Crtaj / Dizajn / Raspored |
| References / Mailings / Review / View / Help | Reference / Pošiljke / Pregled izmjena / Prikaz / Pomoć |
| Font / Paragraph / Styles | Font / Pasus / Stilovi |
| Header / Footer / Heading | Zaglavlje / Podnožje / Naslov |
| Footnotes / Endnotes / Caption | Fusnote / Krajnje bilješke / Natpis |
| Table of Contents / Index | Sadržaj / Indeks |
| Citations / Citation Style / Bibliography | Citati / Stil citiranja / Bibliografija |
| Track Changes / Accept / Reject | Praćenje promjena / Prihvati / Odbij |
| Comment / Show Markup | Komentar / Prikaži oznake |
| Mail Merge | Spajanje pošte |
| Sections / Section Break | Odjeljci / Prelom odjeljka |
| Undo / Redo | Poništi / Ponovi |
| Ribbon / Navigation Pane / Quick Access Toolbar | Traka / Navigacijsko okno / Traka za brzi pristup |
| Bold / Italic / Underline / Strikethrough | Podebljano / Kurziv / Podvučeno / Precrtano |
| Hyphenation | Rastavljanje riječi |
| Watermark | Vodeni žig |
| Save / Save As / Open / Print | Sačuvaj / Sačuvaj kao / Otvori / Štampaj |
| Read Mode | Način čitanja |
| Text Box | Okvir za tekst |

## Verification and remaining scope

Tests in `crates/ui-egui/src/i18n/tests.rs` check locale selection (Cyrillic Bosnian excluded), the
saved preference, that choosing Bosnian leaves the document unchanged, and the bundled fonts'
coverage of č, ć, đ, š, ž and the catalog's punctuation. The shared checks (catalog validation and
the missing-translation report, `cargo xtask i18n-report`) include Bosnian automatically. Bosnian
spelling, grammar and hyphenation are not included. Native speaker review is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-29 | major | First version: Bosnian (`bs`) interface language, conventions and terminology |
