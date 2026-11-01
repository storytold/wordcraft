# Localization parity with Microsoft Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-30 · **Change:** trivial (word count counts each Han ideograph and kana as a word, #4547; previously trivial: the Traditional Chinese interface draws hanzi to the Taiwan standard with an installed Traditional Chinese font first: PingFang TC, Microsoft JhengHei, Noto Sans CJK TC; previously minor: the egui 0.37 migration preserves the equation “Delete Around Caret in Equation” translation in all 27 catalogs through the shared catalog pin; previously minor: Spanish proofing: es-ES, es-MX, es-419 and es-UY from the installed Hunspell dictionaries, Uruguayan `vos` forms, opening `¿`/`¡` checks, #3360, #3553; previously minor: the interface catalogs moved to storytold/craft-i18n and are looked up through the `craft-i18n` crate, embedded at build time from the commit in `craft-i18n.rev`; previously trivial: interface-language tests: a missing translation never fails a test, `cargo xtask i18n-report` in `cargo xtask ci` prints the strings each language lacks, and lookup and template tests use inline fixture catalogs or the pseudo-language; previously trivial: interface audit: missing catalog entries are reported per language and don't fail the audit; previously minor: disabled-control reasons in ribbon hints translated in every catalog, #2678; previously trivial: Slovenian interface catalog: every key of every other catalog, 2,614 entries; previously minor: Romanian interface catalog; previously trivial: the File › Home greeting is translated in every catalog; previously trivial: Japanese row: ruby (Phonetic Guide) is supported and HTML export/import keeps it, #1681; previously trivial: Russian sorting keeps ё with е and recognises decomposed ё/й; previously minor: English dialects: en-GB, en-CA, en-AU and the other British-spelling Englishes proof their own spellings and flag the other dialect's, #1281, #1045; previously minor: how to install the Hunspell dictionaries on Windows, macOS and Linux: `docs/spelling-dictionaries.md`; previously minor: Read Aloud speaks each sentence in its own language, #1843; previously minor: Indonesian proofing: id-ID spelling from the installed Hunspell dictionary, dictionary-driven suggestions, generic checks with Indonesian messages, #498; previously trivial: print-area strings translated in every catalog; previously trivial: Italian: the interface audit checks the Italian catalog, `Privacy Settings` wording; previously trivial: Italian grammar: `po'`, `qual è`, `È`, `un`/`un'` rules; more elisions and truncations in spelling; previously trivial: Czech interface catalog completed, 2,447 entries; previously trivial: interface audit runs in CI again: size labels translated, 20 strings in every catalog, Read Aloud error race fixed, #1128; previously trivial: Review › Language reaches every language again — Set Proofing Language… and Set As Default, #860; previously minor: Italian proofing: it-IT spelling from the installed Hunspell dictionary, accent-first suggestions, conservative grammar with Italian messages; previously minor: Portuguese proofing; previously trivial: Turkish interface catalog completed, 2,325 entries, #774; previously minor: Hungarian interface added; previously trivial: sorting and table numbers follow the text's language, #836, #837; previously minor: Polish interface; Polish proofing — spelling, grammar, hyphenation — and a proofing language per run; previously minor: Turkish interface added; previously major: first version; catalogs measured from `crates/ui-egui/src/i18n`, Word's languages from its bundle listing) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

Interface languages, script support and proofing per language. How translation works (adding a
language, the embedded catalogs): `crates/ui-egui/src/i18n.rs`. The catalogs, their format and
the clean-room rule live in [storytold/craft-i18n](https://github.com/storytold/craft-i18n)
(`locales/<locale>/wordcraft.tsv` and the shared `common.tsv`).

**Dimension: ~29% (estimated; 28% before Italian proofing, 27% before Portuguese proofing, 25% before Polish and per-run proofing languages), 70–115 h to full
for the twelve key languages** plus ~45 h for the other ~17 languages Word ships, plus
native-speaker review (human).

## What Word ships (measured by listing the bundle)

- **30 interface languages** (31 `.lproj` folders incl. Base): ar, cs, da, de, el, en, en-GB, es,
  es-MX, fi, fr, fr-CA, he, hu, id, it, ja, ko, nl, no, pl, pt (BR), pt-PT, ru, sk, sv, th, tr,
  zh-CN, zh-TW. Arabic and Hebrew get a mirrored interface. No Hindi or Vietnamese interface on Mac.
- **120 proofing tools** (`SharedSupport/Proofing Tools`): spellers for ~50 languages, grammar
  checkers for ~20, hyphenators for ~16, thesauri for ~20.

## What WordCraft ships (measured)

- **27 interface languages**: English plus `zh-hans`, `zh-hant`, `ja`, Korean `ko` (#1216), `uk`, `es`, `pt-br`,
  Serbian `sr`/`sr-latn` (#250), Estonian `et`, Turkish `tr`, Russian `ru`, Italian `it` (#594), German `de` (#574), Polish `pl`, Czech `cs`, Hungarian `hu`, French `fr`, Bulgarian `bg`, Vietnamese `vi` (#1434), Swedish `sv` (#1219), Croatian `hr` (#1271), Romanian `ro`, Slovenian `sl`, Dutch `nl` and Danish `da` (#2722). Catalogs have 2,061–2,674 entries
  each; Polish has every key of every other catalog, including the built-in style names and the equation gallery, except the citation style names only Hungarian, Bulgarian, Swedish, Romanian and Slovenian translate so far. A missing translation falls back to English and never fails a test or
  build. `cargo xtask i18n-report`, part of `cargo xtask ci`, prints per language the interface
  strings its catalog lacks: everything the interface audit saw looked up, the ribbon tabs, command
  labels and ribbon locations, the File › Home greetings, disabled-control reasons and the equation
  gallery. The catalogs are embedded at build time from craft-i18n (`CRAFT_I18N_DIR`, the commit
  in `craft-i18n.rev`; a build without it is English only), and craft-i18n's checker applies the
  format checks to every language alike (rows parse, no duplicate keys, placeholders match the
  English, plural forms, ellipses); the report also lists rows the crate skips and strings kept in English. Font tests check that the bundled faces have a glyph for each catalog's letters.
  Tests of the lookup code (English fallback, contexts, placeholder templates, built-in style names,
  the per-thread language, command search) use small inline catalogs or the pseudo-language, so
  rewording a translation never fails a test.
  `pl`, `pl-PL` and `pl_PL.UTF-8` system locales pick Polish automatically.
- Of the 675 English strings a scan extracts from `tl!(…)` calls and command labels/locations, each
  catalog covers **661 (98%)**. Some newer dialog and pane strings (Equation tab tooltips, Zotero,
  Read Aloud) aren't wrapped in `tl!` yet, so they show in English in every
  language: the true share of visible strings is **~93–95%** (estimated).
- Pending PRs: Norwegian Bokmål (#208) and Nynorsk (#210), Hebrew with RTL (#20).
- **Proofing: English, Polish, Portuguese, Italian, Russian, Czech, Indonesian and Spanish**, chosen per run from its language (`w:lang`, resolved through
  the styles; Review › Language and the status bar set it: a short list, plus Set Proofing
  Language… for any of 38 languages and Set As Default for the document and future blank documents). Languages without proofing tools get
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
| Simplified Chinese (Mandarin) | zh-hans | 929 entries; 98% of extracted, ~95% of visible | most; newer panes English | CJK ✅, IME ✅; Windows UI font fixed (#248); interface font chain covers every catalog character, installed CJK font after the embedded ones when it doesn't, PingFang found on macOS 12 (#486); font list by Chinese names (宋体, 微软雅黑, #505); word count counts each Han ideograph and kana as a word, not punctuation (#4547); no vertical text | no | partial | 3–5 |
| Spanish | es | ~930; 98% / ~95% | most | Latin ✅ | no | partial | 1–2 |
| Hindi | hi | 0 | ❌ | Devanagari shaping via HarfRust, untested; no hyphenation | no | none | 4–6 |
| Arabic | ar | 0 | ❌ | shaping, joining, lam-alef, bidi ✅ (#207); no kashida; **no mirrored UI** | no | none | 6–8 |
| French | fr | 2,615 entries (complete catalog) | all | Latin ✅ | no | partial (no proofing) | 1–2 (proofing) |
| Portuguese | pt-br (pt-PT none) | ~930; 98% / ~95% (pt-BR) | most | Latin ✅ | no | partial | 2–4 (pt-BR finish + pt-PT) |
| Indonesian | id | 0 | ❌ | Latin ✅ | no | none | 3–4 |
| Japanese | ja | ~930; 98% / ~95% | most | CJK ✅, IME ✅; ruby (Phonetic Guide) over its base text, kept by HTML export/import (#1681); word count counts each Han ideograph and kana as a word, not punctuation (#4547); no vertical text or document grid | no | partial | 3–5 |
| German | de | 2,021 entries; same key set as zh-hans (#574); 98% of extracted, ~95% of visible | most | Latin ✅ | no | partial | 1–2 |
| Korean | ko | 2,463 entries (all catalog keys and preview context) | most; newer panes English | Hangul ✅ (Korean-first embedded/system UI font fallback); macOS Korean IME fixes pending (#163, #164) | no | partial | 3–5 |
| Vietnamese | vi | 2,448 entries; every key of the reference catalogs (#1434); 98% of extracted, ~95% of visible | most | Latin ✅ | no | partial | 0–1 (review) |
| Romanian | ro | 2,674 entries; every key of the other catalogs; AI-assisted draft | most | Latin ✅ (ă â î ș ț in the bundled faces) | no | partial | 0–1 (review) |

Other shipped languages: **20** — Traditional Chinese (`zh-hant`), Ukrainian (`uk`), Serbian
Cyrillic and Latin (`sr`, `sr-latn`, #250), Estonian (`et`), Turkish (`tr`), Russian (`ru`), Italian (`it`, #594), Polish (`pl`), Czech (`cs`), Hungarian (`hu`), French (`fr`), Bulgarian (`bg`), Vietnamese (`vi`, #1434), Swedish (`sv`, #1219), Croatian (`hr`, #1271), Romanian (`ro`), Slovenian (`sl`), Dutch (`nl`) and Danish (`da`, #2722), all partial.
Swedish (`sv-SE` and `sv-FI` system locales, `{"value":"sv"}`) has 2,504 entries. Counts use
count-neutral wording (`{words} ord`: the noun is the same in the singular and the plural). The
catalog is an AI-assisted draft whose terminology and running app were reviewed by a native
speaker. Swedish proofing is not included; terminology: [`localization-sv.md`](localization-sv.md).
Romanian (`ro-RO` and `ro-MD` system locales, `{"value":"ro"}`) has 2,674 entries covering every key of the other catalogs. Commands use the noun form ("Salvare", "Deschidere…") and messages address the user formally. Counts keep the number and noun as the English does; Romanian proofing is a separate change. The catalog is an AI-assisted draft that a native speaker has not yet reviewed line by line.
Traditional Chinese draws its hanzi to the Taiwan standard on the desktop: the embedded Chinese face (Noto Sans CJK SC) has
mainland glyph shapes (說, 這, 為, 戶), so an installed Traditional Chinese font comes before every CJK face — PingFang TC on
macOS, Microsoft JhengHei on Windows, Noto Sans CJK TC (or Source Han Sans TW) on Linux; without one, and on the web, the
Simplified Chinese order stays.
Croatian (`hr-HR` system locales, `{"value":"hr"}`) has 2,507 entries: every key of the reference catalog, built-in style names and the equation gallery. Counts use count-neutral wording (`Riječi: 21`) because Croatian numerals take three noun forms. The catalog was written and checked by a native speaker; Croatian proofing is a separate change.
Estonian uses the existing fonts, follows `et-EE` system locales, and is available in Options or
through `ui.language` with `{"value":"et"}`. Its 1,307 entries cover the existing interface catalog.
Estonian spelling, grammar, dates and document templates are not added by the interface catalog.
Turkish works the same way (`tr-TR` and `tr-CY` system locales, `{"value":"tr"}`) with all 2,325
entries, the same keys as the Russian catalog. Command search folds the Turkish dotted and dotless I (`italik` finds İtalik, `YAZI TİPİ`
finds Yazı Tipi) while English labels and ASCII command ids keep their usual matching. Turkish
proofing is not included; terminology and checks: [`localization-tr.md`](localization-tr.md).
Bulgarian (`bg-BG` system locales, `{"value":"bg"}`) has 2,419 entries: every key of the reference
catalog, the equation gallery's labels and everything the interface audit finds. Counts use count-neutral wording (`Думи: 21`). Bulgarian text is
hyphenated by the Bulgarian syllable rules (#826); Bulgarian spelling and grammar are not included.
Slovenian (`sl-SI` system locales, `{"value":"sl"}`) has 2,614 entries: every key of every other
catalog. Counts use count-neutral wording (`Besede: 21`); terminology: [`localization-sl.md`](localization-sl.md).
Slovenian proofing is not included.
Hungarian follows `hu-HU` system locales and `{"value":"hu"}`, with a complete catalog of 2,419 entries: every key of
the Russian catalog, built-in style names shown translated, plus the citation style names. Hungarian proofing is not included; terminology and conventions:
[`localization-hu.md`](localization-hu.md).

Czech follows `cs-CZ` system locales and `{"value":"cs"}`, with 2,447 entries: every key of the
Russian and Spanish catalogs, including built-in style names and numbered families, the equation
gallery, watermarks, paper sizes and preview tiles. Counts use count-neutral wording
(`Počet slov: 21`). Czech proofing is not included.

When no saved proofing-language default exists, Polish, Portuguese and Czech interfaces write
new documents and opened TXT/Markdown text in their proofing languages (`pl-PL`, `pt-BR`, `cs-CZ`).
A Spanish interface writes them in the region's Spanish: `es-` plus the system region when that
region's language is Spanish (`es-UY` on a Uruguayan system, `es-MX` on a Mexican one), else
Spain's `es-ES` (#3360).

**Interface audit (runs in CI).** `cargo xtask i18n-report` runs `untranslated_interface_text`,
which lays the app out in a pseudo-language (every tab, File page, dialog, pane, the right-click
menu, and a hover and a click on every ribbon control) and fails on visible text that skipped the
catalog. It then prints the missing-translation report described above, which never fails. Proper names (citation styles such as APA or
GOST, LaTeX, Unicode) and `bad parameters:` errors, which name a command's JSON parameters for
scripts and MCP, stay English. Size labels (Envelopes, Labels, Layout › Size) are translated
patterns (`{name}   {height} × {width} in`) with the region's decimal separator; paper names
other than envelopes (Letter, Legal, A4, B5 (JIS)) stay as they are (#1128).
The report's disabled-control reasons (the reason a ribbon hint gives for a greyed-out control,
#1086, #2678) are read from every command's `enabled` check in the engine's command sources.

Set Proofing Language › Set As Default keeps the chosen language in user preferences across restarts (#2882), taking precedence over the interface language for blank documents and plain-text/Markdown files. Existing DOCX and built-in English templates keep their own language. Ordinary selection-language changes do not change this preference; older settings keep following the interface language. A rejected command, including a final protected-region check, retains the previous saved default. This does not install dictionaries or add proofing support.

## Proofing per language

| Language | Spelling | Suggestions | Grammar | Hyphenation |
|---|---|---|---|---|
| English: American (`en-US`, other regions, text with no language), British spelling (`en-GB`, `en-AU`, `en-NZ`, `en-IE`, `en-ZA`, `en-IN`…), Canadian (`en-CA`) | Moby word list (public domain, ~160k words) with inflection rules, plus our list of ~280 American/British pairs (`assets/spelling/en-variants.txt`) with inflections by rule: each English flags the other dialect's spellings (`colour` in en-US; `color`, `center`, `analyze` in en-GB; `organise` in en-CA), British English accepts `-ize` and `-ise` (Oxford spelling), and a dialect's form of a known word is right even if the list lacks it (`realisation`). No system dictionary needed | the dialect's own spelling first (`colour` → `color`), then edit distance with the shared suggestions in the dialect's spelling | repeated word, `a`/`an`, sentence capital, space before punctuation, double space | Moby dictionary, our Liang patterns, heuristic |
| Russian (`ru`) | A Russian Hunspell dictionary installed on the system (`ru_RU` from the OS, a package manager, LibreOffice's `dict-ru` or the `WORDCRAFT_DICTIONARIES` folder; UTF-8, KOI8-R or Windows-1251; not bundled, so no Russian spelling marks without it), read by the same affix engine; `ё` optional (`елка`/`ёлка`, `еще`/`ещё`); hyphenated words whole, then by parts (`кто-то`, `по-русски`, `кое-где`, `северо-западный`, `ИИ-консультант`); solid compounds (`высокоэластичный`), `не` with adjectives, participles and verbal nouns (not verbs), productive prefixes (`ультра-`, `эко-`) and prefixed verbs; abbreviations with endings (`ГИПа`) and camel case left alone; our supplement of ~200 modern words with their declensions (`assets/spelling/ru-extra.txt`: `маркетплейс`, `промокод`, `интеграция`, `колеровать`) | `ё`/`е` first, then slips within letter groups (unstressed vowels, voiced and voiceless pairs, `ь`/`ъ`), the common confusions (`жы`, `чя`, `тся`/`ться`, doubled consonants, unpronounced consonants: `здраствуй` → `здравствуй`), a missing space (`незнаю` → `не знаю`; not a preposition before a verb), one typo, then a stem scan | Russian messages: repeated word, space before punctuation, double space, sentence capital (knows `т. е.`, `г.`, `руб.`, `ул.`, initials, numbers, dialogue dashes), possibly missing comma before `что`/`чтобы`/`если`/`хотя`/`поскольку`/`который` and before `потому что` (not after conjunctions and particles such as `и`, `только`, `даже`, `так`, `для того`; not after a preposition; not in `хотя бы`, `что-то`), doubled punctuation | The rules of Russian word division as code (no pattern file): a vowel on each side, two letters at least, `ь`/`ъ`/`й` kept with the letter before, a break after `ъ`, a consonant cluster divided after its first consonant |
| Polish (`pl`) | The Hunspell `pl_PL` dictionary installed on the system (OS package, Homebrew, LibreOffice, the folder set in File › Options › Proofing › Dictionaries folder, or `WORDCRAFT_DICTIONARIES`; not bundled, so no Polish spelling marks without it; the same pane shows which file each language found), read at runtime by a Hunspell-style affix engine (`crates/proof/src/affix.rs`); names capitalized, compounds part by part, `Kennedy'ego` | missing diacritics first (`zolw` → `żółw`), the dictionary's confusion table (`żeka` → `rzeka`, `chuśtawka` → `huśtawka`), missing space (`napewno` → `na pewno`), one typo, then a stem scan within two typos | Polish messages: repeated word, space before punctuation, double space, sentence capital (knows `np.`, `r.`, initials, ordinals, dialogue dashes), missing comma before `że`/`który`/`ponieważ`/`gdyż`/`aby`/`żeby`/`jeśli`/`jeżeli`/`gdyby`/`lecz` (not after `mimo`, `chyba`, `tak`, prepositions…), `we`/`ze` (`we wtorek`, `ze szkoły`), doubled punctuation | TeX Polish patterns (Kołodziejska, Jackowski, Ryćko; MIT option) with their 20 exception words; a Polish syllable heuristic for letters outside the alphabet |
| Portuguese (`pt`, any region) | The Hunspell `pt_BR` dictionary installed on the system — VERO and its derivatives (same places as Polish; not bundled, LGPL/MPL), read at runtime by the same affix engine, which reads its `FLAG UTF-8`, 106 flags, continuation classes and `FORBIDDENWORD` spellings; names capitalized, compounds part by part (`guarda-roupa`, `ex-marido`) | missing accents first (`coraçao` → `coração`, `saude` → `saúde`), the dictionary's confusion table (`seje` → `seja`, `excessão` → `exceção`), missing space (`nãoé` → `não é`), one typo, then an accent-folding stem scan | Portuguese messages: repeated word (the emphatic doubles are allowed), space before punctuation, double space, sentence capital (knows `etc.`, `pág.`, `Dr.`, initials, ordinals, dialogue dashes), doubled punctuation; no comma rule — `que` opens a clause only sometimes | TeX Portuguese patterns (Rezende, Almeida, Araujo, Benevides; BSD 3-clause) with their 2 exception words |
| Czech (`cs`, `cs-CZ`, `ces`, `cze`) | The system Hunspell `cs_CZ` dictionary, read at runtime (Debian/Ubuntu: `hunspell-cs`; the same system, LibreOffice and `WORDCRAFT_DICTIONARIES` search paths as Polish). No dictionary data is bundled; without an installed dictionary spelling stays off | suggestions from the loaded dictionary's MAP/REP/TRY/KEY tables, missing spaces and a stem scan; short-word splits rank after deleting an extra letter; NOSUGGEST stems and their derived forms are accepted when typed but excluded from suggestions | Czech messages (including Czech quotation marks) for repeated words, space before punctuation and double spaces only; digit-space-colon-space-digit ratios keep their spaces; no Czech-specific grammar rules | existing English fallback, as for `Other`; no Czech patterns bundled (the TeX/LibreOffice Czech patterns are GPL-2) |
| Italian (`it`, any region) | The Hunspell `it_IT` dictionary installed on the system (same places as Polish and Portuguese; not bundled — the upstream file is GPL-licensed), read at runtime by the same affix engine; elided forms check the word after the apostrophe (`l'amico`, `un'idea`, `c'è`, `dov'è`, `senz'altro`), truncations keep their apostrophe (`po'`, `mo'`), hyphenated compounds part by part | missing accents first (`caffe` → `caffè`, `perche` → `perché`), the dictionary's confusion table, missing space (`perme` → `per me`), one typo, then an accent-folding stem scan | Italian messages: repeated word (emphatic doubles like `piano piano` are allowed), space before punctuation, double space, sentence capital (knows `ecc.`, `dott.`, `prof.`, initials, ordinals), doubled punctuation, `un pò` → `un po'`, `qual'è` → `qual è`, `E'` → `È`, `un'altro` → `un altro` and `un altra` → `un'altra` (gender from the ending, masculine `-ista`/`-ma`/`-cida`… left alone); these work without a dictionary; no comma rule | TeX Italian patterns (Beccari; MIT option), no exception words |
| Turkish (`tr`, any region) | The Hunspell `tr_TR` dictionary installed on the system (same places as Polish; not bundled), read at runtime; proper nouns capitalized, apostrophe suffixes (`Ankara'da`, `WordCraft'ta`) and hyphenated compounds | missing diacritics / ASCII restoration first (`turkce` → `Türkçe`, `isik` → `ışık`), common confusion table (`yalnış` → `yanlış`, `herşey` → `her şey`), detached suffixes (`bende` → `ben de`, `geldinmi` → `geldi mi`), one typo | Turkish messages: repeated word (idiomatic reduplications allowed: `yavaş yavaş`), space before punctuation, double space, sentence capital (knows `vb.`, `vs.`, `prof.`, `dr.`), no comma before/after conjunctions (`ve`, `veya`, `de`), doubled punctuation | Turkish syllable rules (TDK rules); no external pattern file or word list needed |
| German (`de`, any region) | — (no marks yet, #1564) | — | — | TeX German patterns for the reformed spelling (Trennmustermannschaft; MIT), no exception words; Swiss text uses them too. The patterns leave ambiguous compound joints unbroken on purpose (`Staubecken`, `Urinstinkt`), so German text can hyphenate a little less than in Word |
| Indonesian (`id`, `id-ID`, `ind`, legacy `in`) | The system Hunspell `id_ID` dictionary, read at runtime (Debian/Ubuntu: `hunspell-id`; LibreOffice's `dict-id`; the same system, LibreOffice, Dictionaries folder and `WORDCRAFT_DICTIONARIES` search paths as Polish). No dictionary data is bundled; without an installed dictionary spelling stays off. Hyphenated reduplication is accepted whole (`kupu-kupu`) or by parts (`anak-anak`); hyphenated clitics after capitals and abbreviations (`KTP-nya`, `se-Indonesia`) are accepted | the same dictionary-driven ranking as Czech: MAP/REP/TRY/KEY tables of the loaded dictionary, missing spaces and a stem scan; nothing Indonesian hard-coded | Indonesian messages for repeated words (offering the hyphenated reduplication `anak-anak` first, then the single word), space before punctuation and double spaces only; no Indonesian-specific grammar rules | existing English fallback, as for `Other`; no Indonesian patterns bundled |
| Spanish (`es`, `spa`; by region: Spain and no region, `es-MX`, `es-UY`, and `es-419` with the other countries of the Americas) | The system Hunspell dictionary of each variety, read at runtime, the first file name found in this order: Spain `es_ES`, `es_ANY`, `es`; Mexico `es_MX`, `es_419`, `es_ANY`, `es`; Latin America `es_419`, `es_ANY`, `es`, then the ones LibreOffice ships (it has no `es_419`): `es_US`, `es_MX`, `es_AR`; Uruguay `es_UY`, the Rioplatense `es_AR`, `es_419`, `es_ANY`, `es` (each regional name also with a hyphen, `es-MX`); LibreOffice's `dict-es` and Debian/Ubuntu's `hunspell-es` have them (RLA-ES). No dictionary data is bundled; without one the status bar says the variety has no dictionary. The RLA-ES files load since their `☎️` flag is read (#2164). Hyphenated compounds are checked part by part (`teórico-práctico`). Uruguayan Spanish also accepts the `vos` forms of every verb in the dictionary: imperatives (`vení`, `tené`, `andá`), with pronouns (`decime`, `fijate`, `decímelo`) and the present (`tenés`, `querés`) | the same dictionary-driven ranking as Czech, from the variety's dictionary (its MAP restores accents and `ñ` first: `tambien` → `también`; the regional REP tables carry `vosotros` → `ustedes`); in Uruguay a `vos` form typed without its accent gets it back first (`veni` → `vení`) | Spanish messages for repeated words, space before punctuation and double spaces; a closing `?` or `!` without its opening `¿` or `¡` (the sentence with the mark is offered; `¡…?` pairs, `(?)` and `(!)` are allowed) and a space after an opening mark | existing English fallback, as for `Other`; no Spanish patterns bundled |
| Any other | — (no marks) | — | — | the English patterns, as before |

The dictionary file names each language looks for, the folders searched on each system and how to
install one (`pt_BR` as the example): [spelling-dictionaries.md](spelling-dictionaries.md).

Measured with the SJP.PL dictionary: every one of the ~4.5 million forms an independent expansion of its affix file
produces is accepted, and 200,000 one-edit mutations that aren't words are all rejected; a check
takes ~1 µs, a suggestion list 1–35 ms (release build). Not yet: Czech hyphenation patterns or Czech-specific grammar, other languages' dictionaries,
a thesaurus for Polish, Word's style and clarity refinements, Polish-specific AutoCorrect.

Measured with the LibreOffice `ru_RU` dictionary on a private corpus of 205 real-world Russian
business documents (354,025 Cyrillic words): the dictionary loads in ~0.1 s, the whole corpus is
checked in ~0.6 s, and 1.63% of the words are marked, almost all of them names of people, places
and companies (2.96% before the supplement and the compound rules). Not yet: a Russian thesaurus,
Russian AutoCorrect, agreement checks.

Italian (`it`, #594) follows `it`, `it-IT` and `it-CH` system locales. Its catalog has every key of
the others plus 51 strings that are wrapped in `tl!` but were in no catalog yet (File › Info
properties, the File › Home greeting, list levels, Equation tab gallery and tooltips). Ribbon tabs,
groups and command names use the names Italian Word users know (feature names, as `AGENTS.md`
allows); all other text is written from the meaning of the English. Italian proofing: see the table above.

A native speaker reviewed the catalog on 2026-10-14 (wording fixes, consistent terms).
## Sorting and numbers in the text's language

Sorts follow the alphabet of the text's language (`w:lang`, else the document default):
Table › Sort, Home › Sort, the index and its letter headings, the bibliography and the Table of
Authorities. Letters with diacritics sort with their base letter (`é` with `e`) unless the
language makes them letters of their own: Polish `ą ć ę ł ń ó ś ź ż`, Czech/Slovak `č ch ř š ž`
(and Slovak `ä ô`), Hungarian `cs dz dzs gy ly ny sz ty zs ö ü`, Swedish/Finnish `å ä ö` and
Danish/Norwegian `æ ø å` after `z`, Spanish `ñ`, Turkish, Croatian/Bosnian/Serbian Latin,
Romanian, Estonian, Latvian, Lithuanian, Slovenian and Icelandic (`crates/doc/src/collate.rs`,
a hand-written per-language alphabet table; full CLDR collation is not used). Table sort and
table formulas read numbers with the language's decimal and group separators (`1 250,50` in
Polish, `1.250,50` in German, `1,250.50` in English) and write formula results with its decimal
separator (`crates/doc/src/locale_number.rs`). Cuban and Peruvian Spanish (`es-CU`, `es-PE`) use
decimal points (`1,234.50`), matching the existing region settings and CLDR's Latin American
Spanish parent locale; `cuban_and_peruvian_numbers_use_decimal_points` covers parsing and result
formatting. Interface lists (styles, fonts) still sort by code point.

Russian (`ru`) sorting keeps ё with е (a secondary diaeresis difference), before ж. Composed and decomposed ё and й have the same keys and index headings; the stored text is unchanged. Tests cover Russian locale aliases, case, representative words and these two canonical pairs. This is a narrow Cyrillic correction, not full Unicode normalization or CLDR collation. Ordering reference: [Unicode CLDR Russian collation chart](https://unicode.org/cldr/charts/42/collation/ru.html).

## Region conventions (#838)

As in Word, a new blank document takes its paper from the system **region**, not the interface
language: US Letter in the territories CLDR lists for it (US, Canada, Mexico, the Philippines,
Chile, Colombia, Venezuela and a few Central American ones), A4 everywhere else. Lengths are shown
in inches in the US (and Liberia, Myanmar) and centimetres elsewhere, written with the region's
decimal separator (`2,54 cm` in Poland, Germany, France…); fields take either separator and any
unit (`1 in` in a centimetre field). An explicit File › Options › Advanced › Measurement units
choice still wins; its Automatic entry follows the region again.

Where the region comes from:
- **File › Options › Advanced › Region** (saved; agents: `file.options {"region": "pl-PL"}`, `auto`
  returns to the system's);
- else `WORDCRAFT_REGION` or `LC_ALL`;
- else, on Unix, each convention from its own locale: numbers from `LC_NUMERIC` (else `LANG`),
  paper from `LC_PAPER`, measurement from `LC_MEASUREMENT`;
- else the platform's preferred locale: the macOS preferred locale (`en-PL`), the browser's
  language on the web, and on Windows the home location (Settings › Time & language › Region)
  with the display language. Word follows Windows' regional format instead, so an en-US format
  set by someone living in Germany gives A4 and a decimal comma here; reading the regional format
  needs Win32 calls that are `unsafe` in Rust, which WordCraft forbids.

Runs without preferences (`WORDCRAFT_NO_PREFS`) stay `en-US` unless `WORDCRAFT_REGION` is set, so
agents and screenshots give the same result on every machine. The decimal separator belongs to
the language's own number format (CLDR): a decimal-comma language uses a comma everywhere except
where CLDR says otherwise (`de-CH`), and English, whose `en-XX` locales follow the territory, uses a
comma in decimal-comma territories (`en-DE`, `en-PL`). Other decimal-point languages keep the point
in any territory (`ja-PL` is `2.54 cm`), as CLDR's fallback to the language gives.

Templates (Letter, Resume, Report) keep their own size. Not yet: Word's "Default paper size"
preference of its own, and Word's locale-specific Normal margins (2,5 cm in some regions;
WordCraft keeps 2,54 cm).

## Shared work (not in the per-language hours)

| Item | Hours | Notes |
|---|---|---|
| Wrap the remaining English literals in `tl!` and translate them in every catalog | 4–6 | Makes the six partial languages full except script items |
| Mirrored (right-to-left) interface for Arabic and Hebrew | 15–25 | Ribbon, panes, dialogs, rulers |
| Vertical text, ruby, document grid (Chinese, Japanese, Korean) | counted in [layout-parity.md](layout-parity.md) | |
| Proofing dictionaries, grammar and hyphenation for the 11 non-English key languages | 20–40 | Open dictionaries only (licence check per dictionary); issues #25, #40, #100. Per-run languages, the Hunspell-style affix engine and the TeX pattern reader landed with Polish, so a language with an open Hunspell dictionary and hyphenation patterns is mostly data plus grammar rules |
| Native-speaker review of every catalog | human | Swedish: terminology and the running app reviewed (2026-10-13); Italian reviewed (2026-10-14); the others not yet |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-28 | trivial | Traditional Chinese interface: an installed Traditional Chinese font (PingFang TC, Microsoft JhengHei, Noto Sans CJK TC; the TC face of a pan-CJK collection) goes before the embedded Noto Sans CJK SC, whose mainland glyph shapes looked wrong to Taiwan readers; native only, wasm unchanged; estimates unchanged |
| 2026-10-24 | minor | Preserved “Delete Around Caret in Equation” in all 27 shared WordCraft catalogs; the native/release pin and Flatpak source advance together. |
| 2026-10-23 | minor | Interface catalogs moved to storytold/craft-i18n (`locales/<locale>/wordcraft.tsv`), looked up through craft-libs' `craft-i18n` crate and embedded from the pinned commit (`craft-i18n.rev`); strings other Crafting Apps already translate in the shared catalog now show translated; the "Open" paragraph-spacing preset no longer shows the file verb |
| 2026-10-23 | trivial | Lookup, context, template, style-name and command-search tests use inline fixture catalogs or the pseudo-language; tests that asserted one language's catalog wording removed |
| 2026-10-23 | trivial | Interface-language tests: a missing translation falls back to English and never fails a test; `cargo xtask i18n-report` (part of `cargo xtask ci`) prints the strings each language lacks; single-language coverage tests removed |
| 2026-10-23 | trivial | Interface audit: strings a catalog lacks are printed per language, for every language, instead of failing for Russian, Hungarian, Turkish and Bulgarian |
| 2026-10-22 | minor | Spanish proofing (#3360, #3553): es-ES, es-MX, es-419 and es-UY each read their installed Hunspell dictionary (RLA-ES, not bundled), Czech's dictionary-driven suggestions, Uruguayan `vos` forms (imperatives included), repeated-word, spacing and opening-mark checks with Spanish messages; Review › Language, the status bar, Set Proofing Language and File › Options › Proofing list the four varieties, and a Spanish interface writes new documents in the region's Spanish; estimates unchanged |
| 2026-10-21 | trivial | Japanese row: ruby (Phonetic Guide) is supported over its base text and HTML export/import keeps it (#1681); the row had listed ruby as missing |
| 2026-10-20 | trivial | Russian sorting keeps ё with е and recognises decomposed ё/й; synthetic ordering, case and index-heading regressions; broader parity estimates unchanged |
| 2026-10-20 | trivial | File › Home's greeting (Good morning / afternoon / evening) added to the nine catalogs that lacked it (de, et, ja, pt-br, sr, sr-latn, uk, zh-hans, zh-hant); a test keeps it in every language |
| 2026-10-20 | trivial | Cuban and Peruvian Spanish table numbers use decimal points for parsing and formula results, matching CLDR and the existing region settings; regression covers grouped values and a Spain control |
| 2026-10-20 | trivial | Set As Default persists the proofing language across restarts; command and dialog regressions cover new documents, interface changes, existing document languages and rejection rollback (#2882); readiness estimates unchanged |
| 2026-10-18 | trivial | German hyphenation: runs in `de` (any region) use the TeX German patterns for the reformed spelling (MIT) instead of the English ones; still no German spelling or grammar |
| 2026-10-14 | trivial | The sentence-capital rule (English, Polish, Croatian, Italian, Turkish, Portuguese) skips the first word of a list item (#1741) |
| 2026-10-16 | minor | Indonesian proofing (#498): id-ID spelling from the installed Hunspell `id_ID` dictionary (not bundled), Czech's dictionary-driven suggestions, repeated-word (hyphenated reduplication offered) and spacing checks with Indonesian messages; Review › Language, the status bar, Set Proofing Language and File › Options › Proofing list Indonesian; estimates unchanged |
| 2026-10-16 | trivial | Read Aloud picks a voice for the sentence's language (run tag, else script) when the default voice doesn't speak it, reads Windows text as UTF-8 |
| 2026-10-16 | trivial | Linked the new guide to installing the Hunspell dictionaries on Windows, macOS and Linux (`docs/spelling-dictionaries.md`, `pt_BR` as the example); estimates unchanged |
| 2026-10-16 | minor | French interface catalog completed (2,615 entries, native-speaker review); proofing still pending |
| 2026-10-16 | minor | English dialects (#1281, #1045): en-GB (and en-AU, en-NZ, en-IE, en-ZA, en-IN…, which spell the British way) and en-CA are proofing languages of their own: each accepts its spellings and flags the other dialect's with its own form suggested first, from our American/British pair list; Review › Language lists English (Australia) and (Canada), the Language dialog six more Englishes; an English interface writes new documents in the region's English |
| 2026-10-12 | minor | Czech proofing review: short-word splits rank after deletion, NOSUGGEST excludes suggestions while accepting typed forms, affix comments apply only to headers, ratios keep spaces and repeated-word messages use Czech quotes; synthetic engine end-to-end and latency checks |
| 2026-10-12 | minor | Czech spelling from the installed Hunspell dictionary, runtime-table suggestions and generic checks; Czech interface writes cs-CZ documents; no Czech hyphenation patterns bundled; estimates unchanged |
| 2026-10-13 | minor | Swedish interface catalog (2,504 entries), locale selection, saved preference, audit coverage and font checks (#1219) |
| 2026-10-14 | trivial | Print area translated in all 18 interface catalogs (printer list, duplex, grayscale, discovery/submission feedback, print commands) — 18 strings that shipped English-only with the craft-print integration; written from the English meaning, pending native-speaker review like the rest of the catalogs |
| 2026-10-14 | trivial | Italian: the interface audit's catalog check covers Italian too, as it does Russian, Hungarian, Turkish and Bulgarian; `Privacy Settings` no longer says "current" ("attuali"); a test pins the envelope size labels #1128 made translatable (Italian inches, the region's decimal comma) |
| 2026-10-14 | minor | Italian grammar rules that work without a dictionary: `un pò`/`un po` → `un po'`, `qual'è` → `qual è`, `E'` → `È`, `un'` only before feminine words (`un'altro` → `un altro`, `un altra` → `un'altra`, also `nessun`/`ciascun`/`qualcun`/`alcun`); spelling accepts more elisions (`dov'è`, `anch'io`, `senz'altro`, `nessun'altra`) and truncations (`po'`, `mo'`) and leaves `pò`/`qual'è` to grammar so they aren't marked twice; more emphatic doubles allowed |
| 2026-10-12 | minor | File › Options › Proofing › Dictionaries folder (#1063): a saved folder of Hunspell dictionaries searched first, so an app started from Finder or the Dock finds them without `WORDCRAFT_DICTIONARIES`; the pane lists the dictionary file each language found — Polish, Portuguese, Italian, Croatian, Turkish (`file.options` `dictionariesFolder`) |
| 2026-10-12 | minor | Czech interface catalog completed: 386 entries added (2,447 total), including built-in style names, numbered families, equation gallery, watermarks, paper sizes and preview tiles; coverage test requires every Russian and Spanish key and retains Chinese coverage |
| 2026-10-14 | minor | Italian interface reviewed by a native speaker: Word names for tabs and commands, consistent terms (input penna, Disponi testo, Ombreggiatura), imperatives for commands |
| 2026-10-12 | minor | Italian proofing: it-IT spelling from the installed Hunspell dictionary (system `it_IT`, never bundled — upstream is GPL), accent-first suggestions, conservative grammar with Italian messages, TeX Italian hyphenation patterns (MIT option); Review › Language already offered Italian; an `it` interface writes it-IT documents |
| 2026-10-13 | minor | Interface audit back in CI (#1128): Envelopes, Labels and Layout › Size labels go through translated patterns with the region's decimal separator (the old fixed-format paper rows, dead since #838, removed); 20 strings added to all 18 catalogs (status-bar messages, Reading Mode toolbar, Go To, Search's Suggested); citation style names and `bad parameters:` errors exempt; the Read Aloud player no longer draws a speech error before the frame has polled it, the audit's rare failure under load |
| 2026-10-12 | minor | Russian proofing: spelling against an installed `ru_RU` Hunspell dictionary (also KOI8-R and Windows-1251), suggestions, grammar rules with Russian messages and Russian word division |
| 2026-10-11 | minor | Turkish interface catalog completed (2,325 entries, the same keys as Russian): equation gallery, built-in style names, watermarks, paper sizes and preview tiles; the interface audit checks Turkish too (#774) |
| 2026-10-12 | trivial | Region conventions: File › Options › Advanced › Region (saved) and an Automatic units entry; Unix `LC_PAPER`/`LC_MEASUREMENT`/`LC_NUMERIC` each decide their own convention; the web reads the browser's locale; `WORDCRAFT_NO_PREFS` runs stay en-US; where the decimal separator comes from (#838 review) |
| 2026-10-12 | minor | New blank documents follow the system region: A4 outside the Letter territories, centimetres outside the US, the region's decimal separator in length fields; explicit Measurement units choice wins (#838) |
| 2026-10-11 | minor | Czech (`cs`) interface catalog added |
| 2026-10-13 | trivial | Review › Language and the status bar menu open Set Proofing Language… (29 languages, Set As Default) and Language Preferences…; French or German text shows by name (#860) |
| 2026-10-12 | minor | Portuguese proofing: pt-BR spelling from the installed VERO dictionary (the affix engine grew `FLAG UTF-8`, 128 flag bits, continuation classes and `FORBIDDENWORD`), accent-first suggestions, conservative grammar with Portuguese messages, TeX Portuguese hyphenation patterns (BSD 3-clause); Review › Language offers Português (Brasil); a pt-br interface writes pt-BR documents; dimension ~27% → ~28% |
| 2026-10-11 | minor | Polish interface (13 languages) and Polish proofing: SJP.PL spelling with suggestions, Polish grammar rules, TeX Polish hyphenation; proofing language per run; dimension ~25% → ~27% |
| 2026-10-11 | minor | German interface catalog (#574): 2,021 entries covering the full zh-hans key set; locale, saved-preference and bundled-font tests; no German proofing |
| 2026-10-11 | minor | Italian interface catalog (#594), locale selection, saved preference and font coverage checks; count labels put the noun first so they read right at 1 |
| 2026-10-12 | minor | Language-aware sorting (tables, paragraphs, index, bibliography, Table of Authorities) and table numbers/formulas in the text's language (#836, #837) |
| 2026-10-11 | minor | Font dialog sets the Asian text font (`w:eastAsia`) and the Latin text font separately; the Asian list holds the installed Chinese, Japanese and Korean fonts by their interface-language names (#505) |
| 2026-10-11 | minor | French interface catalog (2,033 entries, @pechnikov2026-star), `fr-*` system locales, saved preference and font coverage checks; no French proofing |
| 2026-10-11 | minor | Font list shows installed fonts by their names in the interface language (宋体 for SimSun in Chinese, Japanese and Korean names likewise) and finds them by either name; a Chinese, Japanese or Korean font is stored as `w:eastAsia` too (#505) |
| 2026-10-11 | minor | Chinese/Japanese interface fonts: a test checks every catalog character has a glyph; an installed CJK font follows the embedded faces whenever they miss one; macOS font asset folders searched for PingFang (#486) |
| 2026-10-10 | minor | Russian interface (#300; 1,524 entries, built-in style names shown translated; covers everything the interface audit finds) |
| 2026-10-15 | trivial | Slovenian interface catalog (2,614 entries: every key of every other catalog), locale selection, saved preference and font checks |
| 2026-10-12 | minor | Bulgarian interface catalog (2,419 entries), locale selection, saved preference, audit coverage and font checks |
| 2026-10-19 | minor | Romanian interface catalog (2,674 entries), `ro-*` system locales, saved preference and font coverage checks; no Romanian proofing |
| 2026-10-13 | minor | Croatian interface catalog (2,507 entries, built-in style names shown translated), `hr-*` system locales, saved preference and font coverage checks; the template name "Letter" has its own `template` context so the paper size keeps its name; no Croatian proofing |
| 2026-10-11 | minor | Hungarian interface catalog (2,419 entries, built-in style names shown translated; covers everything the interface audit finds), locale selection, saved preference and font coverage checks |
| 2026-10-11 | minor | Turkish interface catalog (1,307 entries), locale selection, saved preference, Turkish casing in command search and font coverage checks; catalog sizes re-counted |
| 2026-10-17 | trivial | English spelling (#1694): `-ically` adverbs come from their `-ic` adjective ("specifically", "scientifically"), so they're no longer flagged; other words don't take `-ally` ("desktopally" is still flagged) |
| 2026-10-11 | trivial | English spelling (#520): our supplement `assets/spelling/en-extra.txt` adds ~220 common modern words the Moby list lacks ("desktop", "download", "email"…) |
| 2026-10-10 | minor | Estonian interface catalog, locale selection, saved preference and font coverage checks; count labels put the noun first so they read right at 1 |
| 2026-10-10 | minor | Serbian (Cyrillic and Latin) interface merged (#250) |
| 2026-10-10 | major | First version: twelve-language table, catalog coverage measured, Word's 30 UI languages and 120 proofing tools listed |
| 2026-10-17 | trivial | Ribbon hints' disabled-control reasons (28 strings such as `select a chart first`, `only in Outline view`) translated in all 22 catalogs, machine-written from the English meaning and pending native-speaker review; a test reads the reasons from the engine's command sources and requires every catalog to have them (#2678) |
| 2026-10-28 | trivial | Word count counts each Han ideograph and kana as a word, as Word does, so unspaced Chinese and Japanese text is no longer one word per run; Hangul still counts by spaced words (#4547) |
