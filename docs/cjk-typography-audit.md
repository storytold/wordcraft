# CJK typography audit

> **Status:** investigative, 2026-10-30 · **Scope:** what WordCraft does today for Chinese and Japanese text, which tests pin it, and where the gaps are. No behaviour changes.

Findings are tagged **Verified** (read in code, or pinned by a test named here) or **Hypothesis** (reasoned, not demonstrated).

The product target is Word parity (observed behaviour plus ECMA-376 / ISO 29500), not full [JLREQ](https://www.w3.org/TR/jlreq/). JLREQ, [UAX #14](https://www.unicode.org/reports/tr14/) (line breaks) and [UAX #29](https://www.unicode.org/reports/tr29/) (grapheme clusters) are references for what a test should expect; where Word and JLREQ differ, Word wins unless a change argues otherwise.

## 1. Text handling

| Finding | Status | Evidence |
|---|---|---|
| Caret and Backspace step one ideograph or kana at a time; Han and kana are not in `joins_cluster`. Left/Right also treat か+U+3099, halfwidth ﾊ+ﾞ, emoji ZWJ sequences and decomposed Hangul L/V/T as one unit (Backspace/Delete for these are not yet asserted) | Verified | `doc::cluster` tests `japanese_and_chinese_text_steps_one_character_at_a_time`, `mixed_latin_and_cjk_keep_grapheme_boundaries`; engine `tests/cjk.rs` |
| A supplementary-plane ideograph (U+20000…) is one cluster and one Backspace | Verified | `supplementary_plane_ideograph_is_one_cluster`; engine `supplementary_plane_ideograph_types_and_deletes_as_one_character` |
| An ideographic variation sequence (base + U+E0100…) is one caret/Backspace unit | Verified | `ideographic_variation_sequence_is_one_grapheme_cluster`; engine `ideographic_variation_sequence_edits_as_one_grapheme` |
| IVS and supplementary characters survive shaping as addressable layout clusters | Verified | `layout/tests/cjk.rs::supplementary_and_ivs_codepoints_survive_layout` |
| No IVS-specific logic in non-test code; the text is passed to the shaper, but no test checks glyph IDs from a font with a variation mapping, so variation-glyph selection is unverified | Verified (absence of code); behaviour Hypothesis | — |
| `Document::clamp` forces char boundaries only, so a programmatic selection may sit inside a multi-scalar grapheme until the next caret move | Hypothesis | — |
| Ctrl+arrow word motion uses alphanumeric runs (`bidi::is_word_char`), so unspaced Japanese is one "word"; there is no CJK word segmentation | Hypothesis (read, not exercised) | — |
| IME delete-surrounding counts scalars, not clusters | Hypothesis (read, not exercised) | `doc/src/ime_text.rs` |

## 2. Line breaking

**Verified.** Break opportunities come from `unicode-linebreak` (UAX #14), then `kinsoku::apply` adjusts them using the paragraph's `kinsoku` and `word_wrap` flags. With kinsoku on, fixed `NO_START` / `NO_END` lists stop a line starting with closing punctuation or ending with opening punctuation; with it off, any two East Asian characters may be split. This is Word's Asian-typography model, not a JLREQ line-breaking engine.

Pinned by:

- `kinsoku::tests`: the break opportunities around `、。「」` and in mixed Latin/CJK text; supplementary ideographs count as East Asian.
- `layout/tests/cjk.rs::wrapped_japanese_line_does_not_start_with_full_stop` and `…_does_not_end_with_opening_bracket`: a narrow column is sized so the punctuation falls exactly at the wrap point; each test first checks that with kinsoku off the line *would* break there, so it fails if the lists stop being applied.

**Verified gaps.** `overflowPunct` / `topLinePunct` are stored and shown in the Paragraph dialog but nothing in `crates/layout` reads them. Custom `w:noLineBreaksBefore` / `After` lists are written to DOCX settings but not passed to `kinsoku::apply`.

## 3. Fonts and shaping

**Verified.** East Asian runs use `font_ea` and run-level CJK fallback; shaping is HarfRust; Word-style CJK line metrics live in `fonts/src/word.rs`.

**Verified gaps.**

- `vert` / `vrt2` are exercised only in a `fonts` test; document layout never requests them. Upright text in vertical sections is rotated geometry, not glyph substitution.
- `lang_ea` is read and written, but layout does not pass it to the shaper as a language tag.

## 4. Advanced East Asian features

Columns: model · DOCX I/O · layout · rendering.

| Feature | Model | DOCX | Layout | Render | Notes |
|---|---|---|---|---|---|
| Ruby / Phonetic Guide | ✅ | ✅ | ✅ | ✅ | `format.phonetic` |
| Vertical text (`tbRl`, `btLr`) | ✅ | ✅ | ✅ | ✅ | no `vert` alternates |
| Enclose characters | ✅ | ✅ | ✅ | ✅ | |
| Document grid | ✅ | ✅ | partial | ✅ | |
| Tate-chū-yoko (`w:eastAsianLayout/@w:vert`) | ✅ | ✅ | ❌ | ❌ | stored for round trip; layout never reads it |
| Warichū (`w:eastAsianLayout/@w:combine`) | ✅ | ✅ | ❌ | ❌ | same |
| Emphasis marks, jidori, furiwake, kanbun marks | ❌ | ❌ | ❌ | ❌ | no model |

**Verified.** No code under `crates/layout` reads `CharProps::east_asian_layout`, so warichū and tate-chū-yoko text is laid out as ordinary runs. The DOCX round trip is covered by `crates/docx/tests/it/east_asian_layout.rs`. The layout gap is recorded as an `#[ignore]`d test (`east_asian_layout_combine_should_alter_line_geometry`); it currently asserts only that warichū changes cluster advances, and should be tightened when layout lands. There is no equivalent test yet for tate-chū-yoko.

## 5. Not covered here

- PDF text extraction quality for ruby, vertical and complex CJK text.
- Justification: Word widens Asian–Latin auto-space gaps when justifying; WordCraft does not (`docs/layout-parity.md`). Hypothesis.
- Visual (screenshot or PDF) comparison against Word. Any such fixtures must be synthetic and original, with the expected result recorded as a rule (UAX #14 rule, JLREQ clause, or observed Word behaviour), not as a pixel match.

## 6. Candidate follow-ups

Each is its own change with its own tests; none is part of the change that added this document.

1. Apply `overflowPunct` / `topLinePunct` in layout.
2. Lay out tate-chū-yoko, then warichū.
3. Request `vert` / `vrt2` for upright text in vertical sections.
4. Wire custom kinsoku lists into `kinsoku::apply`.
5. Pass `lang_ea` to the shaper.
