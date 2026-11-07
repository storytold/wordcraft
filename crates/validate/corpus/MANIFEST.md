# Corpus manifest — wordcraft-validate

Every entry below was computed from the shipped generator output on
2026-11-06. Regenerating with the same seeds reproduces these SHA-256
hashes byte-for-byte (verified: two `--all` runs, identical hashes).

## Stress corpus (generated, binaries not committed)

Regenerate: `python3 gen_stress.py --all` (writes `corpus/` beside the
script; copy or point the corpus hook at that directory). Validator
verdict for every file: **PASS** (exit 0) — the corpus is valid by
construction, so any Corrupt/Error finding on it is a false positive.

| File | Seed | Size (bytes) | Parts | Axis | SHA-256 |
|---|---|---|---|---|---|
| `monster.docx` | 7 | 2866914 | 76 | everything high; altChunk + background on | `ceeb8027f1709bc134048926e16a2e56c03bf6b73e1a7a58e5914b094b4c8b85` |
| `axis-revisions.docx` | 8 | 912317 | 43 | rev-density 0.95, 10 authors, 900 paragraphs | `07e74be596a45cb990d45a96bc1c60c68c12173954005f3a6d090ca8b5772578` |
| `axis-comments.docx` | 9 | 1019817 | 43 | 220 threads x depth 8 -> 1,089 comments | `c6ed51ec082d47a7e9c0b2fc713ffd2826edbbd26a03e01b5ef1c0d1b1713f9e` |
| `axis-numbering.docx` | 10 | 899111 | 43 | 300 abstractNum, 60 orphans, 1,200 paragraphs | `b1ac3927f5447e8c081d4f34c5cc52425763d578cb8a28fbc495eff9243b3cac` |
| `axis-styles.docx` | 11 | 851139 | 43 | 600 styles, basedOn depth 30 | `844647766822e8c284fb151ba4954f2107faf24aae866450f4e7e379dcf722bf` |
| `axis-tables.docx` | 12 | 1329626 | 43 | 30 tables, 40x8, nesting depth 3 | `134b3abac3aa48dd08ea6fb4f0f3f3e65aa004ea0260a395fe857733ac10dfb6` |
| `axis-media.docx` | 13 | 6473128 | 89 | 40 media parts, 200 refs, 10 orphans | `017c0ebe4fe6306fd3d2d2e65d044ce705efc131a7cd66de3ce7a314aaca9461` |
| `axis-sections.docx` | 14 | 875946 | 103 | 12 sections (72 header/footer parts) | `271dfdc1490b1b49617e304bae784c08712c0af7aeef12f1c76084b76caca9ef` |
| `axis-fields-math.docx` | 15 | 862438 | 43 | 150 fields, 120 math expressions, 400 bookmarks | `7db498285ff7b8edfa1c4c35826736e7b62689df7e60810112d21b1aff0d78dc` |
| `small-valid.docx` | 16 | 647755 | 42 | control: everything on, all knobs low | `79361a4f653d1bf4920ed029211d8389587f35d03b547cbcf59cbe94cdc3a8c2` |

**Resolved gap (2026-11-06):** the first build of this corpus declared
the glossary part with the main document content type in
[Content_Types].xml; Word for the web refused every file carrying it
as corrupt, and the checker missed it. Dissection isolated the cause,
a one-string correction removed the refusal in Word, the generator
was fixed and gained a relationship/content-type pairing audit, the
checker gained the corresponding CT-MISMATCH pairing, and the corpus
was regenerated — the hashes above are the FIXED corpus. Full story
in EVIDENCE.md. The nine non-control files have not yet been
spot-checked in Word; their absence from EVIDENCE.md means exactly
that.

## Calibration fixtures (binaries committed under `calibration/`)

One defect per file, built by `make_calibration.py` on a rich valid
control (tracked changes, comment, footnote, header, bookmark pair,
custom style, embedded picture). Word for the web outcomes for every
fixture are recorded in EVIDENCE.md.

| File | Rule | Defect | Size (bytes) | SHA-256 |
|---|---|---|---|---|
| `00-control.docx` | — | valid control (corrected pic construction) | 3342 | `2039a3b52283ce87d0783482e6d0e176cc811fb2c31224c96e13ec4d0c0facd2` |
| `01-not-zip.docx` | PKG-NOT-ZIP | not a zip archive | 45 | `7040d0be0810f86cd9eac153915469478ebebe4344f2d3fdd5b6ab2c19fe4347` |
| `02-dup-entry.docx` | PKG-DUP-ENTRY | document.xml twice in the archive | 4220 | `728618b8caa9a053389d8288e03189b2a8805764220ec7fdd2ce25da891e0e48` |
| `03-case-collision.docx` | PKG-CASE-COLLISION | word/Document.xml + word/document.xml | 4220 | `1e811a86492bb48e8ab538ab9edcb5f8909f68e8fdcd1f45bc7413445cbb4d74` |
| `04-no-content-types.docx` | PKG-NO-CONTENT-TYPES | [Content_Types].xml missing | 2934 | `d0ead75e548e900e4262db673a02f0500eefac41a46bcb19c8e326f37c27c101` |
| `05-no-main.docx` | PKG-NO-MAIN | no officeDocument relationship | 3349 | `ed1b623686aa7820e6630da112e088242b0425874d786c093aead5ff6a5900aa` |
| `06-ct-undeclared.docx` | CT-UNDECLARED | part with undeclared extension | 3451 | `c77534a6fede7f8d924836b8d024beeff8a6185ab487a61489ce22ed8e28406e` |
| `07-ct-main-type.docx` | CT-MAIN-TYPE | main part typed application/xml | 3341 | `64d855aa5f65b8629791f34fef87d460a7f8f5eeb74afa06017e4450c3542cb4` |
| `08-xml-malformed-main.docx` | XML-MALFORMED | unclosed tag in document.xml | 3339 | `31bd9e2efa6772742fcf1fddffc9363c1c78742431013b0fd0c2fa2fa532f5a6` |
| `09-xml-malformed-unused.docx` | XML-MALFORMED | malformed word/unused.xml, unreferenced | 3472 | `b685a1605a6da197c708b877dfbf831a8a2a138236230494fb0dbef8c747c6e4` |
| `10-xml-undeclared-prefix.docx` | XML-UNDECLARED-PREFIX | w15 used, never declared | 3354 | `385cf5e8682b0bcd1a9d920d67e27971d87472ef36dbb8dd51e99e751ead0315` |
| `11-rel-dup-id.docx` | REL-DUP-ID | rId3 twice in document rels | 3339 | `61722fe9829570031bd1be85ee8cfbe1caa7bf0caa08eabcc946cc3bf69d13b2` |
| `12-rel-dangling.docx` | REL-DANGLING | image relationship -> missing part | 3344 | `b9e3b5d65400e5baa02ef4c218d6db379d54e9348b792dfd7e5377caa462f3f8` |
| `13-rel-typed.docx` | REL-TYPED | header relationship -> image part | 3337 | `f1ffd4f991cb5e2b8ff0e5baf51c97dac755d5935e011d9666180bb184d7b061` |
| `14-comment-dup.docx` | ID-COMMENT-DUP | two w:comment with id 0 | 3366 | `e8cd38ae5adf1185be8b41dd409398f1da510ce69c4628d4aa5e8ac5be6f2255` |
| `15-comment-ref.docx` | REF-COMMENT | commentReference -> missing comment id 9 | 3343 | `548660ff9e83dd803a595dcfdcf7dc914f6b8593fcc2dd79b917002dd765ad36` |
| `16-docpr-dup.docx` | ID-DOCPR-DUP | two pictures, same wp:docPr id | 3364 | `79dd00098770512f61a98abf5e8fa4e4a6b7425041cc2f1ff3db17bc5d669c5d` |
| `17-bookmark-dup.docx` | ID-BOOKMARK-DUP | two bookmarkStart, same id, different names | 3362 | `b1287fc0504f6f2e564f476780e025fd113fca4ec8000c5639e3c325591f684e` |
| `18-revision-dup.docx` | ID-REVISION-DUP | two w:ins, same revision id | 3368 | `3df4eff3080ef7abf410fc3c826dc0cfe75ba0a3771b3ef366287ae7890fea0f` |
| `19-note-ref.docx` | REF-NOTE | footnoteReference -> missing footnote id 9 | 3342 | `a0169e68150e8bb04be1005f90c5bcc6066f9c21bacfdb73b1a7a2f421561f29` |
| `20-header-ref.docx` | REF-HEADER | headerReference -> missing rId99 | 3343 | `1e089ca9e8c3464cad3607632dc97a71b8f6da981e23c83ea4e69a0c4fde13c4` |
| `21-style-ref.docx` | REF-STYLE | pStyle -> undefined style | 3346 | `60c36794f955c91d36b2bac4d8896dc6dd9713719a746d08f47a1115cdc97c8a` |
| `22-fld-unbalanced.docx` | FLD-UNBALANCED | fldChar begin, no end | 3380 | `ddd0c8b065c442cc52db3dad46650c7bd032e1dbbea1d0abc6b4f7bc69c4fed1` |
| `23-tc-no-p.docx` | STRUCT-TC-NO-P | table cell with no block-level content | 3388 | `3fb232c76139fc0ebb6d9b3c75f3e62ec07b4fb5e23948c4a91e6b483fba0509` |
| `24-deltext.docx` | STRUCT-DELTEXT | w:del containing w:t instead of w:delText | 3367 | `906cb1131d8b95d14e2eb39bba5ddfabb05186f7caf2389c410a06ce25944d41` |
| `25-move-unpaired.docx` | STRUCT-MOVE-UNPAIRED | moveFromRangeStart, no end | 3370 | `e1a6fc7ff39d85e793330c55437edc871b65e632e0fe62bc7ef0bb382cb45edd` |
| `26-media-magic.docx` | MEDIA-MAGIC | JPEG bytes in a .png part typed image/png | 3288 | `5626fb30b2e3e226dee2cd8ad3964576ef5a5789d8ff9e6a4fc1f47fd5c959c0` |
