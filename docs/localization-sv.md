# Swedish interface

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-23 · **Change:** trivial (the catalog moved to storytold/craft-i18n, `locales/sv/wordcraft.tsv`; previously trivial: interface-language tests: a missing translation never fails a test, `cargo xtask i18n-report` in `cargo xtask ci` prints the strings each language lacks, and lookup and template tests use inline fixture catalogs or the pseudo-language; previously trivial: the interface audit lists missing Swedish entries instead of failing; previously minor: Swedish interface catalog and regression checks) · **Target:** WordCraft 0.6.1

Choose **File › Options › Interface language › Svenska**, or follow a Swedish system display
language (`sv-SE`, `sv_SE.UTF-8`, and `sv-FI` for Swedish in Finland). The catalog code is `sv`;
`ui.language` with `{"value":"sv"}` selects it explicitly, and `WORDCRAFT_LANGUAGE=sv-SE` does the
same for a headless screenshot or a launch while the preference is `auto`. The choice is saved with
the other interface settings. Document text, command ids, CLI/MCP/control responses and file paths
do not change.

craft-i18n's `locales/sv/wordcraft.tsv` (storytold/craft-i18n; it was `crates/ui-egui/src/i18n/sv.tsv`) has 2,504 entries: every key of the Spanish reference catalog and
of the Bulgarian one (built-in style names, equation gallery labels, paper sizes, watermarks and
citation styles included), plus 44 newer strings no other catalog has yet: 23 that the interface
audit finds and 21 that the code sends through translation outside the audit's reach (native print
settings, Go To, command search). It is written from the meaning of the English labels; no other
product's Swedish interface, string table or terminology database was consulted, and no other
catalog was used as a source. The catalog format has no plural forms, and Swedish counts don't need
them: `ord` (word) is the same in the singular and the plural (`1 ord`, `21 ord`). Placeholders,
`\n` breaks in ribbon labels and dialog ellipses are kept, and strings added later fall back to
English until they are translated.

The draft was made with Claude Code. A native speaker approved the terminology and every open
wording choice, and reviewed the running app.

## Conventions

- Commands are imperatives (`Spara`, `Infoga tabell`); messages address the user as `du`. When two
  words are equally right, the softer everyday one wins (`Använd`, not `Verkställ`).
- Ribbon tabs keep the English word class: `Infoga`, `Rita`, `Granska` and `Visa` are verbs because
  the same keys are also buttons and groups. Other tabs and groups are nouns.
- Only the first word of a label is capitalised (`Acceptera alla ändringar`).
- `Markera` means select and `Välj` means choose from a list. Mark, markup and highlight use other
  words (`Märk post`, `märkning`, `Framhäv`, `Överstrykning`), so the two never mix.
- Ribbon buttons with a `\n` break are separate keys from the command names and may be shorter
  where the group gives the context: the button `Framhäv\nfält`, the command `Framhäv kopplingsfält`.
- Decimal comma in fixed labels (`Radavstånd 1,5`, `8,5" × 11"`), Swedish quotation marks (”…”),
  and a spaced en dash where English has an em dash (`Redigering – redigera dokumentet direkt`).
- Names stay as they are: WordCraft, Zotero, SmartArt, WordArt, the Craft theme, paper sizes
  (Letter, Legal …) and citation styles (APA, MLA …). Loanwords Swedish speakers use stay too:
  `community`, `commit`.

## Terminology

| Source | Swedish |
|---|---|
| File / Home / Insert / Draw / Design / Layout | Arkiv / Start / Infoga / Rita / Design / Layout |
| References / Mailings / Review / View / Help | Referenser / Utskick / Granska / Visa / Hjälp |
| Ribbon / Pane / Quick Access Toolbar | Verktygspanel / Panel / Snabbverktygsfält |
| Font / Font Size / Font Color | Typsnitt / Teckenstorlek / Teckenfärg |
| Paragraph / Style / Styles | Stycke / Stil / Stilar |
| Select / Selection / Choose | Markera / Markering / Välj |
| Highlight / Markup / Mark Entry | Överstrykning / Märkning / Märk post |
| Picture / Shape / Caption | Bild / Figur / Bildtext |
| Header / Footer / Heading | Sidhuvud / Sidfot / Rubrik |
| Footnote / Endnote / Notes | Fotnot / Slutnot / Fot- och slutnoter |
| Table of Contents / Index / Bibliography | Innehållsförteckning / Register / Källförteckning |
| Track Changes / Accept / Reject | Spåra ändringar / Acceptera / Avvisa |
| Mail Merge / Recipients / Merge Field | Dokumentkoppling / Mottagare / Kopplingsfält |
| Columns (text and table) | Kolumner |
| Options / Apply | Alternativ / Använd |
| Page {page} of {pages} / {words} words | Sida {page} av {pages} / {words} ord |

## Known compromises

The catalog has one translation per English key, so a key used in two senses gets the wording that
fits its most visible place:

- `Outline` is both the View tab's Outline view and the Font menu's text outline effect:
  `Disposition`, which is wrong for the text effect.
- `Display` is both an Options page and the Equation tab's display-equation button: `Visning`; the
  button's tooltip explains it.
- `Open` is File › Open and a paragraph-spacing preset: `Öppna`.

Giving these call sites a translation context (`tc`) would fix them. Some AutoCorrect options
describe English typing (`Capitalize names of days`, `Ordinals (1st) as superscript`, smart
quotes); they are translated as written and still act on English text.

## Verification and remaining scope

`cargo test -p wordcraft-ui-egui --lib i18n::tests` covers Swedish locale tags (`sv`, `sv-SE`,
`sv_SE.UTF-8`, `SV-se`, `sv-FI`) and preference order, saving and restoring `sv`, unchanged document
content and the glyphs `ÅÄÖåäöÉé` plus every other accented letter the catalog uses in the bundled
Inter (Regular, Medium, SemiBold) and JetBrains Mono. The shared checks (catalog validation and the missing-translation report, `cargo xtask i18n-report`)
include Swedish automatically.

Spelling, grammar and hyphenation stay English-only: this is an interface translation, not
Swedish proofing.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-23 | trivial | The catalog moved to storytold/craft-i18n (`locales/sv/wordcraft.tsv`) |
| 2026-10-23 | trivial | Lookup, context, template, style-name and command-search tests use inline fixture catalogs or the pseudo-language; tests that asserted one language's catalog wording removed |
| 2026-10-23 | trivial | Interface-language tests: a missing translation falls back to English and never fails a test; `cargo xtask i18n-report` (part of `cargo xtask ci`) prints the strings each language lacks; single-language coverage tests removed |
| 2026-10-23 | trivial | The interface audit lists Swedish strings the catalog lacks; a missing entry doesn't fail the audit |
| 2026-10-13 | minor | Swedish catalog (2,504 entries), conventions, terminology, known compromises and regression checks |
