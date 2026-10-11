# Localization parity with Microsoft Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-11 · **Change:** minor (CJK interface font coverage, #486) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

Interface languages, script support and proofing per language. How translation works (catalog
format, adding a language, clean-room rule): `crates/ui-egui/src/i18n/mod.rs`.

**Dimension: ~25% (estimated), 80–130 h to full for the twelve key languages** plus ~50 h for the
other ~18 languages Word ships, plus native-speaker review (human).

## What Word ships (measured by listing the bundle)

- **30 interface languages** (31 `.lproj` folders incl. Base): ar, cs, da, de, el, en, en-GB, es,
  es-MX, fi, fr, fr-CA, he, hu, id, it, ja, ko, nl, no, pl, pt (BR), pt-PT, ru, sk, sv, th, tr,
  zh-CN, zh-TW. Arabic and Hebrew get a mirrored interface. No Hindi or Vietnamese interface on Mac.
- **120 proofing tools** (`SharedSupport/Proofing Tools`): spellers for ~50 languages, grammar
  checkers for ~20, hyphenators for ~16, thesauri for ~20.

## What WordCraft ships (measured)

- **10 interface languages**: English plus `zh-hans`, `zh-hant`, `ja`, `uk`, `es`, `pt-br`,
  Serbian `sr`/`sr-latn` (#250), and Estonian `et` (1,161 entries). Catalogs have ~1,100–1,160 entries
  each. A test (`every_tab_and_command_is_translated`) enforces that every ribbon tab, command
  label and ribbon location is translated in every language.
- Of the 675 English strings a scan extracts from `tl!(…)` calls and command labels/locations, each
  catalog covers **661 (98%)**. Some newer dialog and pane strings (Equation tab tooltips, Zotero,
  Read Aloud, Backstage greetings) aren't wrapped in `tl!` yet, so they show in English in every
  language: the true share of visible strings is **~93–95%** (estimated).
- Pending PRs: German (#203), Norwegian Bokmål (#208) and Nynorsk (#210), Hebrew with RTL (#20).
- **Proofing: English only** (one dictionary, rule-based grammar, English hyphenation patterns).
- Document scripts: shaping via HarfRust (HarfBuzz port) for every script; bidi (UAX #9) and Arabic
  shaping tested (#207); IME on all desktop platforms (winit fixes pending, #155–#164).

## The twelve key languages

Status: `full` = every user-visible string translated and the script renders and edits correctly;
`partial`; `menus only`; `none`. Hours are to `full`, Opus 5.5 agent wall-clock, excluding the
shared items below.

| Language | Code | UI strings translated | Dialogs / tooltips / help | Script support | Native review | Status | Hours to full |
|---|---|---|---|---|---|---|---|
| English | en | source (100%) | ✅ | Latin ✅ | yes | **full** | 0 |
| Simplified Chinese (Mandarin) | zh-hans | 929 entries; 98% of extracted, ~95% of visible | most; newer panes English | CJK ✅, IME ✅; Windows UI font fixed (#248); interface font chain covers every catalog character, installed CJK font after the embedded ones when it doesn't, PingFang found on macOS 12 (#486); no vertical text | no | partial | 3–5 |
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
Cyrillic and Latin (`sr`, `sr-latn`, #250), and Estonian (`et`), all partial.
Estonian uses the existing fonts, follows `et-EE` system locales, and is available in Options or
through `ui.language` with `{"value":"et"}`. Its 1,161 entries cover the existing interface catalog.
Estonian spelling, grammar, dates and document templates are not added by the interface catalog.

## Shared work (not in the per-language hours)

| Item | Hours | Notes |
|---|---|---|
| Wrap the remaining English literals in `tl!` and translate them in every catalog | 4–6 | Makes the six partial languages full except script items |
| Mirrored (right-to-left) interface for Arabic and Hebrew | 15–25 | Ribbon, panes, dialogs, rulers |
| Vertical text, ruby, document grid (Chinese, Japanese, Korean) | counted in [layout-parity.md](layout-parity.md) | |
| Proofing dictionaries, grammar and hyphenation for the 11 non-English key languages | 25–45 | Open dictionaries only (licence check per dictionary); issues #25, #40, #100 |
| Native-speaker review of every catalog | human | None reviewed yet |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-11 | minor | Chinese/Japanese interface fonts: a test checks every catalog character has a glyph; an installed CJK font follows the embedded faces whenever they miss one; macOS font asset folders searched for PingFang (#486) |
| 2026-10-10 | minor | Estonian interface catalog, locale selection, saved preference and font coverage checks |
| 2026-10-10 | minor | Serbian (Cyrillic and Latin) interface merged (#250) |
| 2026-10-10 | major | First version: twelve-language table, catalog coverage measured, Word's 30 UI languages and 120 proofing tools listed |
