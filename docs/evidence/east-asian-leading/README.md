# Scoped automatic-leading evidence (#2713)

Measured source: `cbcee3deaf2edca7772587320c47a55bd79d97d5`.
Reference: Word for Mac 16.113.2 (26092012), not the checklist's 16.113.4 target.
[Raw numbers and build binding](numbers.json) retain unrounded errors in points,
defined as CLI minus Word. The Linux CLI used read-only, hash-verified system font
files. Eligible PDF subsets matched the expected PostScript face names. Subset
names do not prove byte identity with Word's original font files.

Automatic pitch errors are at most 0.181755 pt across the measured faces and sizes.
This does not establish global font, first-baseline or pagination parity.
Exact pitches agree within PDF decimal serialization precision, but exact CJK
label-relative first baselines remain early by 2.024680 to 5.024700 pt. Brush Script
at 72 pt retains a -0.273450 pt first-baseline error. Preserving exact-spacing and
fixed-graphic behavior means the implementation is unchanged, not that its first
baselines match Word.

Word content coordinates have an observed 0.24 pt quantum. No coordinates were
snapped or averaged; no new pass threshold or wider tolerance is inferred from it.
All measured metadata-positive faces have zero hhea gap. Brush Script is positive
because of its CJK codepage bits despite lacking CJK glyph coverage. Arial is the
negative control. Nonzero positive CJK-gap replacement has synthetic evidence only.
E4 is excluded: requested Hiragino selected Cambria in Word and Arial in CLI.
Thai and missing-font substitution remain unqualified. Historical native reference
measurements without a recorded Word build remain historical unknown-build evidence.

## Original control generator

Run with Python 3, using a new output directory outside the checkout:

```sh
python3 docs/evidence/east-asian-leading/generate.py /tmp/leading-controls
```

The standard-library generator creates three original OOXML documents and a
relative-name/hash manifest. It reads no font files or private inputs. Font names
are requests only. Runs and paragraph marks specify all font slots and sizes;
paragraph spacing is zero, grid snapping is off, and each case has a preceding
label and two lines. Brush Script requests italic. CJK automatic/exact pairs use
18/36 pt; Arial supplies a negative control. E4 remains a substitution detector,
not qualified evidence.

These are new synthetic controls, not byte replicas of the measured private
inputs. Their changed labels and packaging require fresh measurements. Generated
documents are not native exports or baseline oracles and must not be committed.
No fonts, glyph outlines, Word-produced files, images or private paths are included.

Existing `line_metrics_follow_code_pages` and `east_asian_raw_ascent_guard` controls
retain metadata discrimination, nonzero gaps, invalid-ascent guards, expansion
limits and caching. Future baseline work should isolate the preceding label's
contribution using synthetic OFL metadata; this evidence changes no runtime policy.
