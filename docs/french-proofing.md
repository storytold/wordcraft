# French proofing through Language packs

> **Last reviewed:** 2026-11-04 · **Last updated:** 2026-11-04 · **Change:** minor (adapt the French contribution to the extracted shared proofing library and regional Language packs) · **Status:** proposed; depends on [craft-libs#557](https://github.com/storytold/craft-libs/pull/557)

## Implementation and integration

The generic French Language pack already supplies spelling dictionaries, suggestions and
hyphenation. This contribution adds the French-specific behavior in the shared
[craft-proof library](https://github.com/storytold/craft-libs/pull/557), plus the application's
France/Canada language choices and new-document defaults. The already merged Proofing pane
is reused unchanged.

French remains a `ProofLang::Pack(PackLocale)`. Dictionary loading, the memory budget, reloads,
regional overrides and hyphenation stay on the common pack paths. The removed WordCraft
`crates/proof` directory is not restored; there is no separate French dictionary slot or
embedded French word list/pattern file. All language data remains in craft-i18n.

The proposed app dependency temporarily pins the immutable library PR head
`55a16b7a724cd7ab3f9873a415fbd6fbcd8b3c96` through the official storytold/craft-libs URL.
The library includes upstream elision-first suggestions and the Catalan Valencia locale fix.
Suggestions already admitted by the collector remain available after later collection consumes its
budget; new candidates and changed letter case still retain the dictionary eligibility checks.
That commit is not merged. Before integrating the application, merge/review the shared-library
change, update the app pin and Cargo.lock to its official merge commit, and run the repository
checks on that new application SHA. A passed check of a parent does not validate a new pin.

## Using it

Install the French Language pack in **File › Options › Language**, or choose the offered pack
for a French document. In the Review language menu or language dialog choose **French (France)**
(`fr-FR`) or **French (Canada)** (`fr-CA`). A French interface starts new documents in `fr-FR`;
an explicit editing preference wins, and inheriting the system language preserves a real
French region such as Canada or Belgium. An extension containing `ca` does not mean Canada.

The dictionary search order is unchanged. Configured dictionary folders are searched first;
then an exact regional Language pack and an exact regional system dictionary, before pack
fallbacks and the remaining system names. A system `fr_CA` dictionary can therefore precede
a generic `fr` pack. The locale determines the names: France prefers `fr_FR` / `fr-FR`,
Canada `fr_CA` / `fr-CA`, both allowing generic `fr` fallback.
An isolated folder with `fr.aff` and `fr.dic` therefore serves both regions. An explicit
regional pair can override its region without replacing the other region's dictionary.
French hyphenation comes from the selected pack, with generic `fr` fallback.

## What the French rules do

- Check actual dictionary forms, accents and elisions, including straight/curly apostrophes;
  retain explicitly stored regional forms and the shared KEEPCASE/NOSUGGEST policy.
- Offer bounded accent-oriented suggestions, dictionary REP confusions, missing spaces and
  ordinary typos through the shared suggestion engine. Corrections are offered in Review;
  the text is replaced only when the user applies a suggestion.
- Use non-breaking spaces in French punctuation and guillemets, with Quebec spacing before
  `; ! ?`; keep conservative repeated-word, sentence-capital and elision checks.
- Honor the uppercase/number options, user dictionary and Ignore All. Lookups normalize a
  copy to NFC while issue ranges and replacements keep the original document bytes.

No new agreement, conjugation, homophone or locution follow-up is included. These checks do
not claim a complete grammatical analysis or a guaranteed correction of every misspelling.

## Verification

Shared-library commands and the opt-in pinned Dicollecte gate are documented in
[craft-proof's README](https://github.com/totoz64/craft-libs/blob/55a16b7a724cd7ab3f9873a415fbd6fbcd8b3c96/crates/proof/README.md).
The dictionary is a runtime input, never a committed test asset. The library tests cover
regional pack overrides and reloads, apostrophes, options, NFC/NFD ranges and hyphenation.

Application checks use the normal repository gates, without a feature-specific workflow:

```sh
cargo xtask ci
cargo test -p wordcraft-engine --test french_proofing
```

The engine integration uses original tiny fixtures in a separate test process. It exercises
France/Canada tags through Review, correction/undo, saved editing preference and hot pack
reload, together with another pack language. UI language-label/default tests require the
pinned craft-i18n checkout and `CRAFT_I18N_REQUIRED=1` so catalog assertions actually execute.
No interactive GUI validation or cross-platform success is implied by a local format check.

## Revision history

| Date | Change | Evidence boundary |
|---|---|---|
| 2026-11-04 | Synchronized with current shared styles, dynamic language lists and shared-library fixes; kept France/Canada selection, regional defaults and translated language/region names | New exact-head application and library checks are required; previous runs remain historical |
| 2026-11-03 | Updated the temporary shared-library pin after the compound-apostrophe review fix | New app checks are required for this dependency revision |
| 2026-11-03 | Adapted the original French contribution to shared craft-proof and Language packs; retained explicit France/Canada selection and regional new-document defaults | Shared library and app are reviewed/tested separately; integration depends on the official library merge and tests of the final app pin |
