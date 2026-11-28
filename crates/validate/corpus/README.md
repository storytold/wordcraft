# Validation corpus for `wordcraft-validate`

The evidence base for the checker in this crate: a seeded stress corpus of
valid-by-construction documents, a set of one-defect calibration fixtures, and
the recorded outcomes of showing those files to a real Word consumer.

## Layout

- `gen_stress.py` — stress-corpus generator. Pure Python 3 stdlib,
  deterministic (pinned zip timestamps, one seeded RNG): the same seed
  reproduces byte-identical output. `python3 gen_stress.py --all` builds the
  whole corpus; `--preset NAME --seed N` builds one file. Every build runs a
  construction sanity check (CRC, XML parse of every part, every `r:id`
  resolved against its own part's rels, content-type coverage, per-part
  namespace audit).
- `make_calibration.py` — builds the calibration fixtures: a rich valid
  control plus one file per rule defect.
- `calibration/` — the 27 calibration fixture binaries (small; committed).
  The stress-corpus binaries are **not** committed — regenerate them and
  verify against `MANIFEST.md`.
- `MANIFEST.md` — seeds, sizes, part counts and SHA-256 for every corpus
  file, plus the calibration fixture catalogue.
- `EVIDENCE.md` — one row per (fixture × consumer): product/platform, build
  or "not exposed", date, outcome class and the exact message. Every
  `observed` evidence grade in the checker traces to a row here. New
  consumers (desktop Word, other platforms) add rows; a web result is never
  silently promoted to a desktop claim.

## Running the corpus against the checker

The integration tests honour `WORDCRAFT_VALIDATE_CORPUS`: when set to a
directory of `.docx` files, every file must validate with no Corrupt/Error
findings (a set-but-empty directory, or any INCOMPLETE verdict, fails the
test). Point it at a regenerated stress corpus:

```sh
python3 crates/validate/corpus/gen_stress.py --all   # writes corpus/ beside the script
WORDCRAFT_VALIDATE_CORPUS=crates/validate/corpus/corpus cargo test -p wordcraft-validate
```

## The two loops this corpus serves

1. **False-positive hunt.** The stress corpus is valid by construction, so
   the checker must PASS every file. Any finding is a bug in a rule or in
   the generator; the generator's own sanity checker exists to tell the two
   apart.
2. **Oracle dissection.** Extreme files go to a real Word consumer. If Word
   flags a by-construction-legal file, the difference is dissected — that is
   how the missing-required-children picture rule was found during
   calibration, and how the glossary content-type defect recorded in
   `EVIDENCE.md` was isolated: the first corpus build declared the glossary
   part with the main document content type, Word refused the files as
   corrupt, and the checker — which classifies parts by their declared
   type — adopted the glossary as a second main document and missed it.
   The generator now audits every relationship/content-type pairing at
   build time, and the checker verifies the same pairings.

## Coverage and honest gaps

The stress generator covers the element families of the ECMA-376 coverage
matrix in combination and at scale: revisions (ins/del/moves, property-change
snapshots), four-part comment threading, numbering and style graphs with
realistic orphan accumulation, nested tables with exact grid math, sections
with all header/footer variants, media (including orphan parts), fields,
text boxes as AlternateContent pairs, notes, bookmarks/permissions, math,
rsid build-up, a chart with a real embedded workbook, a glossary, and (in
`monster.docx`) an altChunk.

Not constructed, deliberately: OLE objects (no valid stdlib source for the
binary payload), embedded fonts (obfuscation unverifiable headlessly),
`w:subDoc` and mail merge (external by design), bibliography parts (no
structural join), move *range* forms, `w:cellMerge` records, SDT types
beyond richText/text, w15 repeating sections, and captions.
