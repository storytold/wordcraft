# Khmer Justify and Thai Distributed

Status: first version. Revision history at the end.

Two paragraph alignments for scripts written without spaces between words:

| Alignment | Command | `.docx` | Behaviour |
|---|---|---|---|
| Thai Distributed | `para.thaiDistribute` | `<w:jc w:val="thaiDistribute"/>` | Word-compatible. A wrapped line's spare room is shared **evenly** between the character clusters of Thai and Khmer text (and spaces). Last line stays at the start edge. |
| Khmer Justify | `para.khmerJustify` (`Mod+Alt+J`) | `<w:jc w:val="thaiDistribute" wc:khmer="1"/>` | WordCraft's own. Spare room is shared **by rank**: phrase spaces, then word boundaries, then orthographic syllables, each up to a cap; never inside a syllable. |

Both are `Align` variants in `crates/doc/src/props.rs` (`ThaiDistribute`, `KhmerJustify`).

## File format

Word has one value for this, `thaiDistribute` (ST_Jc, ECMA-376 §17.18.44), VBA
`wdAlignParagraphThaiJustify` = 9. It is not a macro; the "Thai Distributed" entry in Word is a
paragraph alignment (shown when a complex-script/Thai editing language is enabled).

- Reading: `thaiDistribute` → `Align::ThaiDistribute`. With the mark `wc:khmer="1"` →
  `Align::KhmerJustify`. A file written by Word has no mark, so its paragraphs stay Thai Distributed.
- Writing: both write `thaiDistribute`, so Word shows Thai Distributed for a Khmer Justify
  paragraph. The mark is in namespace `urn:wordcraft:ext` (`wc`), declared on every body part and
  listed in `mc:Ignorable`, so Word skips it.
- RTF writes `\qt` (Thai distributed) for both and reads it back as Thai Distributed. ODT and HTML write plain justification.

What was **not** verified: how Word itself spaces Khmer under Thai Distributed (no Khmer-enabled
Word was driven). The even distribution is built from the documented effect; compare against Word
output on a Khmer document before treating it as exact.

## The script (what the layout relies on)

Sources: Unicode chapter 16 (Khmer), W3C *Khmer Layout Requirements*, Microsoft Learn *Line and
word breaking*.

- No spaces between words. Spaces separate **phrases**. U+200B ZERO WIDTH SPACE optionally marks
  word boundaries (the NIDA keyboard types it with the space bar).
- Line breaking is by **word** (dictionary based). The shaping unit is the **orthographic
  syllable**: base consonant or independent vowel + stacked subscripts (each introduced by the
  invisible COENG U+17D2) + dependent vowels + signs. Stacks must never be split.
- Justification: *"adjusts the space between phrases, but also makes certain adjustments to
  inter-character spacing"* (W3C). Inter-character spacing is not standard in running text, so
  here it is the last resort and kept small.
- Classes (`crates/doc/src/khmer.rs`): consonants U+1780–17A2; independent vowels U+17A3–17B3;
  dependent vowels U+17B4–17C5 (the prefix vowels េ ែ ៃ are written after the consonant but
  drawn before it); signs U+17C6–17D1, 17D3, 17DD (register shifters ៉ ៊, robat ៌, bantoc ់ …);
  coeng U+17D2; punctuation ។ ៕ ៖ ៘ ៙ ៚; lek too ៗ (belongs to the word before); digits U+17E0–17E9;
  symbols U+19E0–19FF.

## Font study (fonts installed on the dev machine, read with fontTools)

| Font | Khmer glyphs | Script tag | ZWSP (U+200B) in cmap | Space advance (em) | Notes |
|---|---|---|---|---|---|
| Khmer Sangam MN (macOS) | 114 | `khmr` | yes | 0.30 | features `pref blwf pstf abvf clig dist`; COENG has an advance of 0.48 em |
| Khmer MN (macOS) | 114 | `khmr` | yes | 0.25 | COENG advance 0.60 em |
| Khmer OS Battambang / New | 100 | `khmr`, `latn` | **no** | 0.28 | ~60 `zzNN` stylistic features; no `kern`/`mark` |
| Khmer OS System | 114 | `khmr` | yes | 0.29 | `abvs blws psts mkmk` |
| Battambang (Google) | 100 | `khmr` | yes | 0.26 | minimal feature set |
| Khmer Busra | 114 | `khmr`, `latn` | yes | **0.71** | `mark mkmk ccmp` |
| Content / Angkor / Chantrea | 100–114 | `khmr`, `latn` | mixed | 0.28 | |

What this means for the layout:

1. **ZWSP can be missing from the font.** Khmer OS Battambang/New have no glyph for it, so a
   text typed with the Khmer keyboard would draw a `.notdef` box or leave a gap. Layout now treats
   U+200B (and the word joiner U+2060 and U+FEFF) as an invisible zero-width marker cluster,
   whatever the font has, and U+200B also as a word boundary (`ClKind::Marker`; break allowed after
   it). Other zero-width markers (bookmark and comment anchors, soft hyphens) between two syllables
   don't hide the gap between them.
2. **Space width varies 3×** (0.25–0.71 em), so stretch limits are in **ems of the cluster's
   font size**, never multiples of the space width.
3. **COENG may have an advance** in some fonts. The shaper (`harfrust`) consumes it in the
   subscript substitution; the stack is one cluster (`doc::cluster`), and extra room goes only
   after the cluster, so the stack's internal positioning (GPOS `mark`/`abvm`, `blws`, `pres`) is
   untouched.
4. **Feature sets differ**; some fonts have no `kern`. Nothing in justification relies on kerning
   or on one font's mark positions.
5. Fonts without a Khmer glyph fall back per character (existing fallback), so a line may mix
   faces; each gap uses its own cluster's size.

## How the room is shared (`crates/layout/src/khmer_just.rs`)

Each cluster carries the kind of gap after it (`ParaLayout::gaps`, from
`wordcraft_doc::khmer::gap_between` / `thai::gap_between`):

`Space` > `Word` (after ZWSP, after ។ ៕, between Khmer and a Latin letter or digit; a full stop, bracket or quote stays attached to its word) > `Syllable` (between
orthographic syllables) > `None` (inside a syllable, before a vowel/sign/coeng/ៗ, inside a Latin
word).

*Khmer Justify* fills tiers in order, each gap up to a cap, water-filling when font sizes differ:

| Tier | Cap per gap |
|---|---|
| Space | 0.50 em |
| Word | 0.20 em |
| Syllable | 0.06 em |

If the caps hold back room (a very loose line) the remainder is shared evenly over all gaps, so
the line always reaches the margin. A line with no gaps stays ragged (as Justify does with no
spaces). *Thai Distributed* shares the room evenly over all gaps. Neither adds room after the
last cluster of the line, after a hanging space, or in the spaces that start the line (Thai letters indent a paragraph with a run of them; Word keeps their width), nor on the paragraph's last line or a line
ended by a manual break.

## Known limits / next

- Word boundaries come only from spaces, ZWSP, punctuation and script changes. Text typed
  without either wraps at syllable (cluster) boundaries when a word is too long. A dictionary
  segmenter (ICU's Khmer dictionary / LSTM model, or a rule-based one) would give true word
  wrapping and make the `Word` tier effective for unspaced text; it needs a data licence decision
  and a size budget for WASM.
- Thai gaps use cluster boundaries without word segmentation. Checked against Word for Windows
  on one Thai paragraph (Tahoma 14 pt): same lines, wrapped lines reach the margin, last line not
  stretched, marks stay on their consonant. Word also gaps after the leading vowels เ แ โ ใ ไ
  (done here) and between a consonant and SARA AM ำ (not done: it stays in its grapheme cluster).
- Tune the caps against real Khmer typesetting samples.

## Tests

`doc::khmer`, `doc::thai`, `layout::khmer_just`, `layout/tests/khmer.rs` (filling, no stretch
inside a syllable, rank order, ZWSP, Latin text), `docx/tests/roundtrip.rs`
(`thai_distributed_and_khmer_justify_stay_apart`), `docx::read::props::khmer_tests`.

## Revision history

- First version: alignments, docx mapping, tiered distribution, ZWSP handling, font study.
- Spaces before a line's first character keep their width (found by @pakiat against Word).
- Review fixes: punctuation next to Khmer is not a gap; anchors between syllables keep their gap; leading Thai vowels get a gap; RTF `\qt`; word joiner and U+FEFF are invisible.
