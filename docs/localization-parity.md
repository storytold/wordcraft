# Localization parity with Microsoft Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** minor (Polish interface, the tenth language; Polish proofing — spelling, grammar, hyphenation — and a proofing language per run; previously major: first version; catalogs measured from `crates/ui-egui/src/i18n`, Word's languages from its bundle listing) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

Interface languages, script support and proofing per language. How translation works (catalog
format, adding a language, clean-room rule): `crates/ui-egui/src/i18n/mod.rs`.

**Dimension: ~27% (estimated; 25% before Polish and per-run proofing languages), 75–120 h to full
for the twelve key languages** plus ~45 h for the other ~17 languages Word ships, plus
native-speaker review (human).

## What Word ships (measured by listing the bundle)

- **30 interface languages** (31 `.lproj` folders incl. Base): ar, cs, da, de, el, en, en-GB, es,
  es-MX, fi, fr, fr-CA, he, hu, id, it, ja, ko, nl, no, pl, pt (BR), pt-PT, ru, sk, sv, th, tr,
  zh-CN, zh-TW. Arabic and Hebrew get a mirrored interface. No Hindi or Vietnamese interface on Mac.
- **120 proofing tools** (`SharedSupport/Proofing Tools`): spellers for ~50 languages, grammar
  checkers for ~20, hyphenators for ~16, thesauri for ~20.

## What WordCraft ships (measured)

- **10 interface languages**: English plus `zh-hans`, `zh-hant`, `ja`, `uk`, `es`, `pt-br`,
  Serbian `sr`/`sr-latn` (#250) and Polish `pl` (catalogs of ~1,025–1,045 entries each; Polish has
  every key of every other catalog, a test checks it). A test (`every_tab_and_command_is_translated`) enforces that every ribbon
  tab, command label and ribbon location is translated in every language. `pl`, `pl-PL` and
  `pl_PL.UTF-8` system locales pick Polish automatically.
- Of the 675 English strings a scan extracts from `tl!(…)` calls and command labels/locations, each
  catalog covers **661 (98%)**. Some newer dialog and pane strings (Equation tab tooltips, Zotero,
  Read Aloud, Backstage greetings) aren't wrapped in `tl!` yet, so they show in English in every
  language: the true share of visible strings is **~93–95%** (estimated).
- Pending PRs: German (#203), Norwegian Bokmål (#208) and Nynorsk (#210), Hebrew with RTL (#20).
- **Proofing: English and Polish**, chosen per run from its language (`w:lang`, resolved through
  the styles; Review › Language and the status bar set it). Languages without proofing tools get
  no spelling or grammar marks (as in Word without the language's tools) instead of being flagged
  as English. Detail in [Proofing per language](#proofing-per-language).
- Document scripts: shaping via HarfRust (HarfBuzz port) for every script; bidi (UAX #9) and Arabic
  shaping tested (#207); IME on all desktop platforms (winit fixes pending, #155–#164).

## The twelve key languages

Status: `full` = every user-visible string translated and the script renders and edits correctly;
`partial`; `menus only`; `none`. Hours are to `full`, Opus 5.5 agent wall-clock, excluding the
shared items below.

| Language | Code | UI strings translated | Dialogs / tooltips / help | Script support | Native review | Status | Hours to full |
|---|---|---|---|---|---|---|---|
| English | en | source (100%) | ✅ | Latin ✅ | yes | **full** | 0 |
| Simplified Chinese (Mandarin) | zh-hans | 929 entries; 98% of extracted, ~95% of visible | most; newer panes English | CJK ✅, IME ✅; Windows UI font fixed (#248); no vertical text | no | partial | 3–5 |
| Spanish | es | ~930; 98% / ~95% | most | Latin ✅ | no | partial | 1–2 |
| Hindi | hi | 0 | ❌ | Devanagari shaping via HarfRust, untested; no hyphenation | no | none | 4–6 |
| Arabic | ar | 0 | ❌ | shaping, joining, lam-alef, bidi ✅ (#207); no kashida; **no mirrored UI** | no | none | 6–8 |
| French | fr | 0 | ❌ | Latin ✅ | no | none | 3–4 |
| Portuguese | pt-br (pt-PT none) | ~930; 98% / ~95% (pt-BR) | most | Latin ✅ | no | partial | 2–4 (pt-BR finish + pt-PT) |
| Indonesian | id | 0 | ❌ | Latin ✅ | no | none | 3–4 |
| Japanese | ja | ~930; 98% / ~95% | most | CJK ✅, IME ✅; no vertical text, ruby or document grid | no | partial | 3–5 |
| German | de | 0 (PR #203 open) | ❌ | Latin ✅ | no | none | 1–2 |
| Korean | ko | 0 | ❌ | Hangul ✅; macOS Korean IME fixes pending (#163, #164) | no | none | 3–5 |
| Vietnamese | vi | 0 | ❌ | Latin with stacked diacritics, untested | no | none | 3–4 |

Other shipped languages: **5** — Traditional Chinese (`zh-hant`), Ukrainian (`uk`), Serbian
Cyrillic and Latin (`sr`, `sr-latn`, #250) and Polish (`pl`, 2026-10-10), all partial (~95%: the
strings not wrapped in `tl!` yet show in English in every language). Polish is the only one with
proofing tools, and a Polish interface writes new documents in Polish (`pl-PL`).

## Proofing per language

| Language | Spelling | Suggestions | Grammar | Hyphenation |
|---|---|---|---|---|
| English (`en`, any region; text with no language) | Moby word list (public domain, ~160k words) with inflection rules | edit distance | repeated word, `a`/`an`, sentence capital, space before punctuation, double space | Moby dictionary, our Liang patterns, heuristic |
| Polish (`pl`) | SJP.PL dictionary 2026-09-01 (Apache-2.0 option): 349,513 stems, 7,410 affix rules, ~4.5 million forms, read by a Hunspell-style affix engine (`crates/proof/src/affix.rs`); names capitalized, compounds part by part, `Kennedy'ego` | missing diacritics first (`zolw` → `żółw`), the dictionary's confusion table (`żeka` → `rzeka`, `chuśtawka` → `huśtawka`), missing space (`napewno` → `na pewno`), one typo, then a stem scan within two typos | Polish messages: repeated word, space before punctuation, double space, sentence capital (knows `np.`, `r.`, initials, ordinals, dialogue dashes), missing comma before `że`/`który`/`ponieważ`/`gdyż`/`aby`/`żeby`/`jeśli`/`jeżeli`/`gdyby`/`lecz` (not after `mimo`, `chyba`, `tak`, prepositions…), `we`/`ze` (`we wtorek`, `ze szkoły`), doubled punctuation | TeX Polish patterns (Kołodziejska, Jackowski, Ryćko; MIT option) with their 20 exception words; a Polish syllable heuristic for letters outside the alphabet |
| Any other | — (no marks) | — | — | the English patterns, as before |

Measured: every one of the ~4.5 million forms an independent expansion of the SJP.PL affix file
produces is accepted, and 200,000 one-edit mutations that aren't words are all rejected; a check
takes ~1 µs, a suggestion list 1–35 ms (release build). Not yet: other languages' dictionaries,
a thesaurus for Polish, Word's style and clarity refinements, Polish-specific AutoCorrect.

## Shared work (not in the per-language hours)

| Item | Hours | Notes |
|---|---|---|
| Wrap the remaining English literals in `tl!` and translate them in every catalog | 4–6 | Makes the six partial languages full except script items |
| Mirrored (right-to-left) interface for Arabic and Hebrew | 15–25 | Ribbon, panes, dialogs, rulers |
| Vertical text, ruby, document grid (Chinese, Japanese, Korean) | counted in [layout-parity.md](layout-parity.md) | |
| Proofing dictionaries, grammar and hyphenation for the 11 non-English key languages | 20–40 | Open dictionaries only (licence check per dictionary); issues #25, #40, #100. Per-run languages, the Hunspell-style affix engine and the TeX pattern reader landed with Polish, so a language with an open Hunspell dictionary and hyphenation patterns is mostly data plus grammar rules |
| Native-speaker review of every catalog | human | None reviewed yet |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | minor | Polish interface (10 languages) and Polish proofing: SJP.PL spelling with suggestions, Polish grammar rules, TeX Polish hyphenation; proofing language per run; dimension ~25% → ~27% |
| 2026-10-10 | minor | Serbian (Cyrillic and Latin) interface merged (#250) |
| 2026-10-10 | major | First version: twelve-language table, catalog coverage measured, Word's 30 UI languages and 120 proofing tools listed |
