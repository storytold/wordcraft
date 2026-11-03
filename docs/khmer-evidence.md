# Khmer boundary and proofing evidence

The goal is good line filling with meaningful break opportunities, while leaving words intact enough for spelling correction. A compound that stays whole is not automatically wrong. A finer useful division can help a narrow column without replacing the document text or requiring manually inserted separators.

## Examples that explain the choice

Vertical bars below show optional opportunities, not inserted characters or mandatory newlines. The exact compared heads are touchde/#3110 `a71c1184e94ffb0a37f15bec8c77e6fdf36b0c32` and NguyenDuc2309/#1579 `5c9cd9774efd948df467cbb3072ad4f57b7eebb2`.

| Input | Combined candidate | Pinned alternatives | Meaning |
| --- | --- | --- | --- |
| សហរដ្ឋអាមេរិក | សហរដ្ឋ · អាមេរិក | Both keep the compound whole | Extra useful opportunity explicitly discussed in [W3C sealreq #35](https://github.com/w3c/sealreq/issues/35) |
| ខ្ញុំស្រឡាញ់ភាសាខ្មែរ | ខ្ញុំ · ស្រឡាញ់ · ភាសា · ខ្មែរ | Both keep ភាសាខ្មែរ together | Finer line filling, not a claim that the whole compound is invalid |
| កសីករ | One misspelled token | Both offer ក · សី · ករ | Human-reviewed example: preserving the token enables the correction កសិករ |
| អ្វីៗ | Repetition sign stays attached | #3110 also keeps it attached; #1579 offers a boundary before ៗ | Credit #3110 equally; a boundary guard could also address this in an ICU adapter |

The W3C typography overview describes variation in compound boundaries and author-supplied ZWSP corrections: [Approaches to line breaking](https://www.w3.org/International/articles/typography/linebreak.en.html). This supports useful finer units, not indiscriminate syllable splitting.

Additional short cases come from the [ICU Khmer word regression block](https://github.com/unicode-org/icu/blob/b34cf4d07e71acf06f01b3a1902759609c111e2d/icu4c/source/test/testdata/rbbitst.txt#L1341) and [ICU4X spacing issue #7218](https://github.com/unicode-org/icu4x/issues/7218). The word cases exercise the lexical decisions used for wrapping; alternative compound granularity can still be acceptable. The spacing report concerns an older LineSegmenter API, so it is not evidence that WordCraft #1579 has that historical bug. [Recorded inputs, sources and candidate offsets](khmer-evidence/sourced-cases.json) include derived ZWSP/WJ variants clearly labeled as such. These captures use the tested preview core, not newly captured current-main app layouts.

## Sourced cases and native-reader follow-up

The original captured probes in `sourced-cases.json` preserve the earlier outputs:

- `សណ្ដូកកបណ្ដែតខ្លួន`: #3110 recovered `សណ្ដូក · ក` while the original candidate joined `សណ្ដូកក`. The current general short-word refinement now recovers that division, with no literal word exception.
- `ពណ៌សម្ដេចថាខ្មៅ`: ICU4X follows `ពណ៌ស · ម្ដេច`, while the candidate and #3110 choose `ពណ៌ · សម្ដេច`. @sungkhum explicitly accepted the latter; it is retained. Matching this source reference alone is not a correctness verdict.

## Real-document coverage, counted by words

Evaluation includes **302,428 Khmer word occurrences in real book text extracted from InDesign files**, across 3,331 paragraphs. These are model-counted word spans, not independently annotated gold words. For the general short-word refinement, 302,427 original spans stay identical; one joined span becomes two, with no removed boundaries. Real-document coverage complements focused synthetic word-pair, typo and safety cases; our evidence does not depend only on contrived challenge sentences. The private book text is not redistributed.

The same real documents were also checked with all 256,278 authored U+200B characters removed from a derived copy, leaving other text and controls unchanged. That automatic-breaking comparison covers **302,020 original Khmer word occurrences**: 302,018 spans remain identical and two become finer divisions, with no removed boundaries. One added boundary recovers an opportunity already present in the authored document; the other is the contextual split accepted by @sungkhum. These are two modes of the same corpus, not independent corpora or accuracy scores.

[Current refinement evidence](khmer-evidence/short-word-refinement.json) records the corpus digest, word-span counts, changed public examples and unchanged proof/suggestion comparisons. The existing 37,424 fixture records plus 11 sourced cases yield 37,435 checks: 14 records change, including duplicated fixtures shared by the two public suites. They represent four additional public synthetic word-pair strings, the requested phrase, and one private document token. No existing line opportunity is removed. The additional public strings were generated as word pairs, not injected misspellings. All 1,305 existing proof results and 276 suggestion results are identical; proofing the 276 mutation/edge inputs also gives identical results. The requested phrase's former spelling flag is removed.

The refinement only considers an unknown original model token with a known prefix of at least three Khmer clusters and a known one-cluster suffix, a unique two-word dictionary division, and no competing close spelling correction. The original token and its adjacent-token contexts are protected when known, preferred/fused, or plausibly misspelled. No recursive splitting or boundary removal occurs. This remains a conservative heuristic: an accidental extra character can also form a legitimate short word. The native reviewer accepted the changed real-document division after seeing the surrounding paragraph. Other synthetic pair outputs remain disclosed rather than assumed to be linguistic wins.

## Human review and regression protection

@sungkhum supplied 14 records covering 13 distinct specimens. Deduplicating the repeated authored/automatic `អ្វីៗ` judgment leaves 19 rejected engine/position pairs at 13 distinct positions in eight strings. One whole phrase row was explicitly accepted for the combined candidate. There are no recorded rejections of that candidate; this is selected review evidence, not an accuracy rate. “Done” without a judgment does not mean approved. The reviewer clarified that compounds kept whole can be fine; smaller meaningful units are preferred for layout flexibility.

The companion shared-library PR includes the sanitized review records, negative-boundary tests, the accepted finer phrase and the sourced compound/control regression. A proofing regression embeds `កសីករ` inside unspaced Khmer, checks the exact issue range, obtains `កសិករ` as the first suggestion, applies it and verifies clean spelling afterwards. It does not promise recovery of every arbitrary misspelling. App tests cover corrections across formatting, repetition-sign preservation and exact undo.

## Costs and scope

[Isolated release probe measurements](khmer-evidence/four-way-benchmark.json) used 15 rotating fresh-process trials and 2,400 warm segmentation calls. These are historical same-machine harness measurements; they are not app typing latency, installer size or current-main performance.

| Metric | Combined | #3110 | #1579 |
| --- | ---: | ---: | ---: |
| Median first call | 15.286 ms | 7.549 ms | 0.019 ms |
| Median total for 2,400 warm calls | 10.742 ms | 2.781 ms | 1.542 ms |
| Median peak RSS | 10,846,208 B | 7,389,184 B | 2,473,984 B |
| Executable delta over empty wrapper | 613,712 B | 314,368 B | 1,850,416 B |

The combined model is slower and more memory intensive than both alternatives. The additional scope is spelling, suggestions and preferred-form/phrase checks. #1579 also addresses Thai and Lao, while this proposal is Khmer-specific. #3110 has its own justification work; this proposal does not claim to incorporate every feature of either PR.

All 39,127 model weights and complete dictionaries remain: 714,343 compressed resource bytes from 4,230,002 canonical bytes. No linguistic pruning is proposed. Before the current-main reconciliation, 37,424 comparisons retained the previous optimized engine’s core/proof/suggestion output exactly; 3,331 contained private book paragraphs whose text is not published. This is preservation evidence, not independent accuracy. Historical native safety checks covered 480 book layouts and 10,707 continuation starts with no tested structural bad starts, not a linguistic verdict for every line.

No general syntax/semantic grammar claim is made. Dictionary-based Khmer word counts, full Aksara web-plugin parity, runtime download support for this custom data format, Windows runtime acceptance and official signing remain outside this draft. See [integration details](khmer-integration.md).

## Validation scope

The shared engine passes 30 core tests and 484 proofing tests, with one ignored.
The three formerly excluded proofing tests now use stable unknown/non-linguistic
absence tags and a synthetic positive French dictionary fixture; none is skipped
because of installed host dictionaries. Strict Clippy for both changed shared
crates/all targets passes. The complete shared workspace is not claimed green.
The app passes all nine `cargo xtask ci` gates with the explicit Khmer pack
and pinned companion commits: formatting, strict Clippy, tests, interface audit,
language validation, doctests, assets, layers and WebAssembly (15 crates).
Validation ran on macOS with Rust 1.99.1; Windows runtime acceptance remains open.

## Review follow-up (2026-11-03)

Refinement now shares a 160,000-entry dictionary-comparison budget across a
paragraph's candidate spans and adjacent contexts. A depleted budget conservatively
retains original model spans. The corpus counts and timings above describe the
historical unlimited-scan candidate; they are retained as historical evidence,
not a new latency or accuracy claim. Raw repeated benchmark runs and conversational
metadata have been removed from the published JSON; the summaries, pinned inputs
and artifact hashes remain.

Caret affinity is restricted to safe Khmer dictionary wraps. Other scripts,
hyphens, slashes and emergency wraps retain existing navigation and hit testing.
The multi-page regression sizes its text from measured pagination rather than
macOS font metrics. Runtime acceptance on Linux and Windows remains separate
from the local synthetic tests.

[Budget preservation results](khmer-evidence/refinement-budget.json): all 34,104
public records are identical to the prior candidate, as are all 3,331 private
paragraphs with authored boundaries. With U+200B removed, two long paragraphs
retain original model spans where the prior refinement added an extra break.
These are conservative abstentions at the work limit, not removals of original
model opportunities. The requested phrase and the independent short-suffix,
known-word and correctable-typo regressions pass. No private text is included.

The release audit adds regressions for lazy non-Khmer logical navigation, styled
Arabic/Khmer/Indic/Thai shaping fallback with a Latin control, repeated header
and footer page hints, empty/rejected edits, nested rollback, and mixed
English/Vietnamese/Khmer proofing. These synthetic checks do not replace native
platform and Khmer-reader acceptance.
