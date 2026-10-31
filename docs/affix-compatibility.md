# Shared affix compatibility baseline

> **Last reviewed:** 2026-10-20 · **Last updated:** 2026-10-20 · **Change:** minor (frozen pre-French behavior and old compact dictionaries)

`cargo test --locked -p wordcraft-proof --test affix_compatibility` checks the
shared affix refactor against the implementation before French proofing, upstream
`e6c5a701ddff5be16e266028168c70658aae39d9`.

The seven fixtures in `crates/proof/tests/fixtures/affix_compatibility_cases.rs`
are original synthetic examples of Polish, Portuguese, Italian, Czech, Russian
and Indonesian dictionary mechanics, plus 70 distinct flags. They exercise
conditional stripping, prefix/suffix cross products, non-crossing rules, two
suffixes, Unicode flags and words, proper-name casing, forbidden and NOSUGGEST
forms, and suggestion metadata. They are not complete language dictionaries.

The expected file records the old engine's actual results, rather than predictions
made by the refactored engine. It includes generated forms, exact/case-folded
lookups, suggestion eligibility and TRY/KEY/MAP/REP metadata. The companion compact
file contains the old engine's WCAFF1 bytes, hex encoded, one dictionary per line.
The second test loads those old bytes with the new reader and requires the same
observable results. The first test also checks a current-writer round trip.

To reproduce the historical recording, compile that revision's unmodified
`crates/proof/src/affix.rs` and `patterns.rs` in a small Rust binary with
`miniz_oxide = "=0.8.9"`. Use the fixtures and the test's `snapshot` function,
substituting the historical `AffixDict` import. For each fixture, construct with
the comment `original compatibility fixture`, concatenate the snapshots, and
hex encode `to_bytes()` followed by a newline. Do not refresh these expectations
from the new engine when a test fails.

| Recorded input/output | SHA-256 |
|---|---|
| Historical `affix.rs` | `a130c7d65b0efc92bf6df941dac831cd59faf3aaa727d0c4bb12e8413524cc64` |
| Historical `patterns.rs` | `62495a3375731020395a251c156ad6a792669b7dc41e1f440abb7d4181561bc5` |
| `affix_compatibility.expected` (33,298 bytes) | `2d359815888dd8697ff3df22eb628f07f33bc7dbf7507f53e20383261f9b26d3` |
| `affix_compatibility.compact` (2,533 bytes) | `37b27277cf01062b83e4bfeaa91a5350e56dd27c7029482ef85fb12cf3e4ef4f` |

This is a bounded compatibility regression, not a guarantee for all possible
Hunspell dictionaries. The existing language-specific spelling, suggestions,
grammar and hyphenation tests remain in the full `wordcraft-proof` suite. French
CI runs that suite and the checksum-pinned real French dictionary gate on all
three desktop systems, with Rust 1.98.1, and prints the compiler version.

## Additional local comparison with real dictionaries

A separate Windows check compiled the unmodified historical engine and the
French engine at `7ddce854c89b7a2a330727438293bc0a8733256f`. The two versions
produced byte-identical observations from three complete runtime dictionaries
at LibreOffice/dictionaries revision `67032ee957631908098a72edf48dc81cac27e369`:

| Dictionary | Loaded stems | Rules | Sampled stems | Word/case/deletion observations |
|---|---:|---:|---:|---:|
| Portuguese (pt_BR) | 312,295 | 25,829 | 1,000 | 59,683 |
| Italian (it_IT) | 95,346 | 3,173 | 1,000 | 155,034 |
| Polish (pl_PL) | 348,901 | 7,410 | 1,000 | 52,712 |

The [metadata report](affix-real-dictionary-comparison.json) records exact input
URLs, byte counts, hashes and equal output hashes. The files were decoded as
UTF-8 (BOM removed) or ISO-8859-2 for Polish, without editing rules or words.
The dictionary contents and generated observation files are not redistributed.

Sampling sorts all stems and selects index `i * (len - 1) / 999` for each
`i` from 0 through 999. For each stem, the report records all sorted distinct
generated forms. Lookup probes use up to the first 256 forms plus the stem,
adding lowercase, uppercase, initial-capital and first-character-deleted variants;
duplicates are removed. Each observation records `contains`, `check`,
`can_suggest` and `contains_suggestible`. Header metadata also matches.
This was a sampled local comparison, separate from the permanent synthetic
matrix regression. It does not measure language-specific suggestion ranking or
the correctness of every word in those dictionaries.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-20 | minor | Historical output and compact-format compatibility regressions for the shared affix refactor |
