# Layout comparison corpus

Original synthetic documents for `cargo xtask layout-corpus`. They exercise page geometry, line
breaking, tables, heading page breaks and footnotes without using any Microsoft asset or wording.

Each case is a folder:

```
case-name/
  case.docx            committed source document (original, synthetic)
  case.word.pdf        Word's PDF, exported by hand (local only, never committed)
  case.expected.json   optional expected page count and known differences (committed)
```

`case.word.pdf` is deliberately ignored by git: Word output stays local. WordCraft's export,
visual-diff maps and reports also stay local (the default report folder is
`target/layout-corpus`).

## Run

```sh
cargo xtask layout-corpus --dir corpus/layout
# or keep the report next to the corpus (also ignored):
cargo xtask layout-corpus --dir corpus/layout --out corpus/layout/_report
```

For each case the harness:

1. exports `case.wordcraft.pdf` with `wordcraft-cli convert`;
2. skips the case with `skipped: no Word reference` when `case.word.pdf` is missing;
3. runs `cargo xtask visual-diff` (poppler's `pdftoppm` and `pdftotext` must be installed) and
   reads its `result.json`;
4. writes `summary.json` and `summary.md`, ranked worst first.

`--private` keeps case names out of the report (they become `case-<hash>` labels), passes
`--private` to visual-diff and writes numbers and hashes only.

## Adding a case

1. Write an original DOCX (do not copy Microsoft templates, sample text or assets).
2. Open it in Word on the local machine, export to PDF by hand, and save it as
   `case.word.pdf` in the case folder. Do not automate Word with keystrokes on a shared machine.
3. Optionally add `case.expected.json`:

   ```json
   {
     "page_count": 2,
     "known_differences": [
       "footer sits 1 pt lower",
       {"page": 3, "reason": "the table splits one row earlier"}
     ]
   }
   ```

4. Add the DOCX to `ATTRIBUTION.md` (author, source, licence) and run
   `cargo xtask layout-corpus --dir corpus/layout`.

The harness reports layout differences; it does not certify visual identity. Different fonts and
font metrics change pixels, so use the page count, same-break percentage and line offsets to judge
paginated layout, not SSIM alone.
