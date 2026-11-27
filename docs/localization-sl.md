# Slovenian interface

> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-23 · **Change:** trivial (the catalog moved to storytold/craft-i18n, `locales/sl/wordcraft.tsv`; previously trivial: interface-language tests: a missing translation never fails a test, `cargo xtask i18n-report` in `cargo xtask ci` prints the strings each language lacks, and lookup and template tests use inline fixture catalogs or the pseudo-language; previously trivial: catalog completed to every key of every other catalog, 2,614 entries) · **Target:** WordCraft 0.7.0

Choose **File › Options › Interface language › Slovenščina**, or follow a Slovenian system display
language (`sl-SI`, `sl_SI.UTF-8`). The catalog code is `sl`; `ui.language` with `{"value":"sl"}`
selects it explicitly, and `WORDCRAFT_LANGUAGE=sl-SI` does the same for a headless screenshot or a
launch while the preference is `auto`. Document text, command ids, CLI/MCP/control responses and
file paths do not change.

craft-i18n's `locales/sl/wordcraft.tsv` (storytold/craft-i18n; it was `crates/ui-egui/src/i18n/sl.tsv`) has 2,614 entries: every key of every other catalog (built-in
style names, equation gallery labels, paper sizes, watermarks, greetings, citation style names and
native print settings included). It is written from the meaning of the English labels; no proprietary translation
resources were consulted.

## Conventions

- Ribbon tabs, groups, panes and dialog titles are nouns (`Vstavljanje`, `Pisava`, `Podokno slogov`).
- Commands and buttons use the imperative (`Shrani`, `Vstavi tabelo`, `Sprejmi vse spremembe`);
  options and toggles use the imperative as well (`Preverjaj črkovanje med tipkanjem`).
- Messages address the user with the polite plural (`Shranite dokument`).
- Only the first word of a label is capitalised.
- Slovenian has singular, dual and plural; the catalog has no plural forms, so counts use
  count-neutral wording with a colon (`Besede: 21`, `Razveljavi dejanja: {n}`).
- Placeholders never take a case ending; the wording puts them after a colon instead.
- Quotation marks »…«; decimal comma in prose (`Razmik vrstic 1,5`); paper sizes keep inches as in
  the source.
- Text columns and table columns are both `stolpci`, as in common Slovenian usage.

## Terminology

| Source | Slovenian |
|---|---|
| File / Home / Insert / Draw / Design / Layout | Datoteka / Osnovno / Vstavljanje / Risanje / Načrt / Postavitev |
| References / Mailings / Review / View / Help | Sklici / Pošiljanje / Pregled / Pogled / Pomoč |
| Font / Paragraph / Styles | Pisava / Odstavek / Slogi |
| Header / Footer / Heading | Glava / Noga / Naslov |
| Footnote / Endnote / Caption | Sprotna opomba / Končna opomba / Napis |
| Table of Contents / Index | Kazalo vsebine / Stvarno kazalo |
| Citation / Citation Style / Bibliography | Citat / Slog citiranja / Bibliografija |
| Track Changes / Accept / Reject | Sledenje spremembam / Sprejmi / Zavrni |
| Comment / Markup | Komentar / Oznake |
| Mail Merge / Merge Field | Spajanje dokumentov / Polje spajanja |
| Section / Section Break | Odsek / Prelom odseka |
| Undo / Redo / Repeat | Razveljavi / Uveljavi znova / Ponovi |
| Ribbon / Pane / Quick Access Toolbar | Trak / Podokno / Orodna vrstica za hitri dostop |
| Bold / Italic / Underline / Strikethrough | Krepko / Ležeče / Podčrtano / Prečrtano |
| Hyphenation / Proofing | Deljenje besed / Preverjanje |
| Character Spacing / Tracking | Razmik med znaki |
| Watermark | Vodni žig |
| Grayscale | Odtenki sive |
| Printer / Print | Tiskalnik / Natisni |
| Reading Mode | Način branja |
| Full Screen | Celozaslonski način |
| Recover | Obnovi |
| Text box / Content control | Polje z besedilom / Kontrolnik vsebine |

## Verification and remaining scope

Tests in `crates/ui-egui/src/i18n/tests.rs` check locale selection, the saved preference and the
bundled fonts' coverage of č, š, ž. The shared checks (catalog validation and the missing-translation report, `cargo xtask i18n-report`)
include Slovenian automatically. Slovenian spelling, grammar and hyphenation are not included. Native speaker review
is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-23 | trivial | The catalog moved to storytold/craft-i18n (`locales/sl/wordcraft.tsv`) |
| 2026-10-23 | trivial | Lookup, context, template, style-name and command-search tests use inline fixture catalogs or the pseudo-language; tests that asserted one language's catalog wording removed |
| 2026-10-23 | trivial | Interface-language tests: a missing translation falls back to English and never fails a test; `cargo xtask i18n-report` (part of `cargo xtask ci`) prints the strings each language lacks; single-language coverage tests removed |
| 2026-10-15 | trivial | Catalog completed to every key of every other catalog (2,614 entries) |
