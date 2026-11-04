# Khmer language integration

Reconciled 2026-11-04 with WordCraft main `1340705f`, craft-libs main `01315d9`,
and craft-i18n main `575cfde` (including WordCraft’s catalog updates `cde4900` and `9d6d54d`).
This draft pins the companion heads of [craft-libs #551](https://github.com/storytold/craft-libs/pull/551)
and [craft-i18n #122](https://github.com/storytold/craft-i18n/pull/122), fetched through the upstream repositories.
Merge the dependencies first and retain or update those pins. See [examples and evidence](khmer-evidence.md).

## User behavior

An explicit Khmer language build provides offline dictionary/model word wrapping,
spelling suggestions and preferred-spelling/phrase rules. There is no JavaScript
runtime or online checking service. Preferred-form rules are not general sentence
grammar or semantic checking.

Khmer text in a default/English paragraph is detected automatically. Explicit
other-language and no-proof spans remain respected. Corrections use the existing
commands, formatting and undo. Personal dictionaries and ignored words continue
through the native proofing path. `km`, `km-KH` and `khm` resolve to Khmer.

Without the optional pack, no Khmer dictionary is advertised and Khmer words are
not marked against the English dictionary. Structural wrapping safeguards still
attach repetition signs and honor word joiners. The language options explain
when Khmer proofing data was not included. The draft pack is hidden from normal
pack releases until the suite's installed-data mechanism can read its source kinds.
This change does not implement or promise an in-app download button.

## What the fourth implementation combines

- **SBBIC Aksara:** feature-set v3 cluster-boundary model, full frequency and
  spelling lexicons, preferred-form rules, affix/title data and native proofing.
  A fixed-size cache reuses exact overlapping dictionary queries; probes borrow
  slices of the original text. Scorer tests retain exact floating-point bits.
- **touchde, WordCraft #3110:** the contiguous dictionary-storage approach informs
  a compact immutable spelling index, retaining frequency counts and suggestion
  ranking. Explicit cluster/repetition safety and post-tab geometry were reviewed
  alongside that contribution. Wrapped-line caret affinity is a separately
  reviewable editor change; it does not determine lexical breaks.
- **NguyenDuc2309, WordCraft #1579:** a small dictionary adapter isolates script
  detection and break opportunities from paragraph layout. Non-Khmer paragraphs
  avoid Khmer data initialization. The fourth implementation does not import
  ICU's Khmer data or union competing segmenters' break positions.

These mechanisms can be combined; incompatible lexical policies cannot simply
be added together. A faster greedy dictionary hit is not substituted for the
model: recognized compounds can intentionally contain useful break positions.
No claim of universal linguistic superiority is made.

## Boundaries and original text

`wordcraft_proof::khmer` is the shared core API. Its ranges address the original
UTF-8 bytes; segmentation never normalizes or inserts zero-width spaces into the
document. Paragraph layout segments visible text across formatting runs, then
filters opportunities through shaped/grapheme boundaries and wrap-only glue.
Caret stops remain independent of word-joiner and repetition wrapping protection.

A manually inserted U+200B supplies an optional opportunity, including within a
model word and across bold formatting. It is not a mandatory newline. U+2060
forbids adjacent wrapping. Dependent marks and COENG sequences must remain attached.
The tests exercise narrow and wide columns, mixed text, tabs, justification,
formatting boundaries, correction/undo and exact document text preservation. Continuous-view Page Up/Down
uses the visible side of a soft-wrap caret; one regression covers Web, Draft and
Outline together, including the first/last-line limits. Relative Go To, layout
summary and status-page reporting also use the caret’s visible page; the same
regression checks a wrap crossing pages.

## Shared data and build integration

1. `craft-i18n` pins unchanged Aksara raw files, SHA256, exact upstream revision
   and MIT notices in its existing language-pack manifest. New custom kinds are
   staged by its existing verified cache/fetch tooling. No raw Khmer dictionaries
   or publisher books are committed to the app or shared-library repository.
2. `craft-libs` adds `craft-khmer`, below the app layer, and integrates it into
   `craft-proof`. `CRAFT_KHMER_DIR` selects a staged pack. The offline build
   validates source and canonical hashes and produces separate lossless Brotli
   assets. `CRAFT_KHMER_REQUIRED=1` rejects absent data; explicit corrupt or
   incomplete data always fails. With no directory, the crate builds without data.
3. WordCraft consumes shared proofing and break positions. It requires no new
   local language crate. Cargo.toml and Cargo.lock pin craft-libs #551 through
   the official git URL; craft-i18n.rev pins #122. No local Cargo patch is
   required to reproduce the submitted app. No release workflow changes are included.
   Existing default CI checks the no-Khmer-data path; use the explicit build below
   to exercise the Khmer language data. Runtime-pack loading remains a separate decision.

The full language payload is 714,343 compressed bytes from 4,230,002 canonical
bytes, with every dictionary record and all 39,127 f64 model weights retained.
This is resource size, not an installer download delta. Lazy initialization
can keep ordinary known-word layout from loading spelling data; rare short-word
refinement consults the shared spelling index and one-edit checks. A shared
160,000-comparison budget bounds those scans per paragraph segmentation,
including adjacent contexts. On exhaustion, refinement retains the original
model span; cached, completed answers remain usable. Three independent layout
queries therefore perform at most 480,000 such comparisons, regardless of the
number of distinct candidate spans. In the no-ZWSP private-book stress test it
conservatively suppresses two extra refinements from the prior candidate; all
authored-boundary paragraphs and 34,104 public comparison records are unchanged.
This budget does not change source data.
The local warm probe for the requested short phrase increased from 0.010 ms
to 0.89 ms; normal prose stayed around 0.040 ms. These optimized-development
probe timings are not packaged-app typing latency. See the evidence for scope.

## Acceptance and remaining decisions

Document statistics use the shared model/dictionary Khmer word spans. The status
bar, selected-text count, Word Count dialog, NUMWORDS fields and saved DOCX word
statistics share the same counter within their existing story scopes. The count is independent of page width, fonts and justification;
no separators are inserted into document text. Real whitespace remains the input
for justification spacing. A two-generation paragraph-text cache retains at most
16 MiB of keys and 65,536 entries in total (8 MiB and 32,768 entries per
generation); these limits exclude HashMap buckets, allocator and model overhead.
Paragraphs over 64 KiB are not retained. A hit in the previous generation moves
to the current one; when the current generation fills, it becomes the previous
one and the older generation is cleared. Old documents, intermediate edits and
partial selections therefore age out, and newly opened/edited text can be cached
without restarting. Sequential scans retain active working sets that fit one
generation; larger working sets can require repeated segmentation.

The status bar also caches its selection count by document id, revision and
selection, so unchanged repaints do not extract/segment selected text, including
paragraphs above 64 KiB. Each window owns this count and document switches reset
it. The document model calls `craft-khmer` directly rather than depending on
`craft-proof` and its other proofing data. Regressions preserve repeated
20,000-paragraph counts and verify recovery on a 2,000-paragraph/122,000-word
synthetic document after stale entries fill a cache, then a one-paragraph edit.
Without Khmer data, authored zero-width boundaries still count,
while otherwise unspaced Khmer uses the existing token fallback.

The acceptance target is good Khmer wrapping: reject malformed cluster splits,
detached marks and ignored valid manual opportunities; permit multiple reasonable
compound divisions. Exact agreement is useful for regression detection but is not
an independent linguistic score. Recovered IDML publisher markers are incomplete
reference data with unknown training overlap, not a replacement held-out corpus.

Before release: independent Khmer-reader review, runtime acceptance on Windows
and macOS with multiple fonts, IME and long-document edits; official platform
packaging/signing; and agreement on optional pack installation versus an explicit
Khmer language build. Current comparative timings do not measure Windows typing
latency. Retain the reproducible test evidence and disclose unresolved failures.

The earlier renamed preview installers and updater changes are deliberately not
part of the upstream contribution. Official release integration removes a private
product fork; SBBIC's language-data stewardship remains useful and necessary.

## Proofing menu fonts

The right-click menu borrows installed document-script fonts for its explanation, suggestions and synonyms, independently of the proofing pane. When a new face is needed, the menu waits until the next UI pass before painting text. This prevents missing-glyph boxes and strict-renderer failures on the first opening. Fonts are reused on later openings; Latin-only menus do not preload Khmer fonts. No font files are bundled. The regression tests require an installed Khmer face for the Khmer rendering assertion; platform and font-specific human acceptance remains necessary.

## Release edge cases (2026-11-03)

Logical Left/Right keeps layout lazy in English, French, German, Japanese and
Thai paragraphs. Khmer dictionary-wrap geometry is requested only where needed;
right-to-left navigation keeps its existing visual path. Bold/italic fallback
retains a shaping-capable regular face when the requested style lacks layout
tables, while Latin styles keep their existing selection. Explicit page hints
follow repeated headers and footers. Empty and failed edits preserve the visible
wrap caret; real edits clear it. Command and font regressions cover these cases.

The shared dependency also retains English grammar adjacent to Khmer and checks
Khmer beside automatically detected Vietnamese. Explicit language and no-proof
spans keep their boundaries. Installed French dictionaries are now a positive
proofing fixture rather than an assumption that French is unavailable.

## Repetition-sign proofing

Dictionary lookup accepts either an exact repeated expression or its valid stem before trailing LEK TOO (ៗ). A misspelled stem is still checked. Spelling and preferred-form issues use the original byte range of the stem, so correcting it preserves the sign and its formatting; direct full-word suggestions retain the supplied suffix. This is proofing cleanup and does not introduce a break before the sign. Other Aksara cleanup differences, including embedded join controls and quoted title handling, remain separately documented; this is not a claim of complete web-plugin parity.

## Credit

Contributor: **@sungkhum**, with Sol-agent assistance. Aksara is MIT licensed,
Copyright (c) 2025 SBBIC Khmer. Both previous WordCraft contributors are credited
above and in source. See their original proposals:
[touchde #3110](https://github.com/storytold/wordcraft/pull/3110),
[NguyenDuc2309 #1579](https://github.com/storytold/wordcraft/pull/1579).
The contributor's founding of SBBIC.org and work on Khmer word breaking with the
Unicode Consortium provide relevant experience, not institutional endorsement.

## Reproduce an explicit Khmer build

Check out the companion data revision named in `craft-i18n.rev`, then use its
existing manifest tool to stage the source files. From the WordCraft checkout,
with `../craft-i18n` at that revision and a fresh `../khmer-pack` destination:

```sh
cargo run --manifest-path ../craft-i18n/Cargo.toml --bin language-packs -- stage --locale km --out ../khmer-pack
CRAFT_I18N_DIR="$(cd ../craft-i18n && pwd)" CRAFT_KHMER_DIR="$(cd ../khmer-pack && pwd)" CRAFT_KHMER_REQUIRED=1 cargo xtask ci
CRAFT_I18N_DIR="$(cd ../craft-i18n && pwd)" CRAFT_KHMER_DIR="$(cd ../khmer-pack && pwd)" CRAFT_KHMER_REQUIRED=1 cargo run --locked -p wordcraft
```

Use absolute paths for environment variables when Cargo is invoked from another
directory. Staging downloads only pinned files and verifies their hashes; the
subsequent language engine runs offline. On Windows, set the same environment
variables using PowerShell before the Cargo commands. For a no-Khmer-data build,
leave both `CRAFT_KHMER_DIR` and `CRAFT_KHMER_REQUIRED` unset.

## Data provenance and notices

On 2026-11-03, SBBIC contributor @sungkhum identified SBBIC internal work,
[SEAlang's frequency lists](http://sealang.net/project/list/), and
[SIL NRSI's khmerlbdict](https://github.com/silnrsi/khmerlbdict) as sources for
Aksara's language resources; SBBIC also contributed to khmerlbdict. SIL's
[README](https://github.com/silnrsi/khmerlbdict/blob/master/README.md) independently
records its use of the SEAlang Khmer frequency list and other named lists.
SIL publishes its [MIT license](https://github.com/silnrsi/khmerlbdict/blob/master/LICENSE),
Copyright (c) 2015 SIL NRSI. The contributor supplied SEAlang's MIT notice,
Copyright (c) 2010, Doug Cooper, with a screenshot of the project's License page.
Aksara's pinned repository carries the MIT notice, Copyright (c) 2025 SBBIC Khmer.
All three copyright lines and the full permission notice ship in WordCraft's `NOTICE`.

This establishes the reported resource lineage and published/supplied license
notices. SBBIC confirmed on 2026-11-03 that the tagger was trained on internal
documents it has permission to train with. This is not a source-by-source
training-data manifest. No private source corpus is shipped in this PR. The pinned
raw-file hashes identify the distributed Aksara artifacts.

The `craft-i18n.rev` update retains the companion Khmer manifest together with
its upstream catalog history; unrelated catalog commits are not Khmer translations.
The `fr-FR` to `zxx` test-fixture adjustment is intentional: French is supported
by the newer shared proofing dependency, so a no-proof fixture must explicitly
request non-linguistic content rather than depend on French tools being absent.
