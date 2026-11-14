# Corpus manifest — wordcraft-validate

Every entry below was computed from the shipped generator output on
2026-11-06; regenerated 2026-11-07 after the generator learned to list
its separator notes in footnotePr/endnotePr (NOTE-PR-LISTING). Regenerating with the same seeds reproduces these SHA-256
hashes byte-for-byte (verified: two `--all` runs, identical hashes).

## Stress corpus (generated, binaries not committed)

Regenerate: `python3 gen_stress.py --all` (writes `corpus/` beside the
script; copy or point the corpus hook at that directory). Validator
verdict for every file: **PASS** (exit 0) — the corpus is valid by
construction, so any Corrupt/Error finding on it is a false positive.

| File | Seed | Size (bytes) | Parts | Axis | SHA-256 |
|---|---|---|---|---|---|
| `monster.docx` | 7 | 2,866,580 | 76 | everything high; altChunk + background on | `70a87e245c0d1c55917e624c8f8701c07f92cc8e6c92c0ff37b69995785ab70f` |
| `axis-revisions.docx` | 8 | 912,367 | 43 | rev-density 0.95, 10 authors, 900 paragraphs | `9db82bcb1c0cadf22a5d0e3c3fa0571d004e1826f91988020cdd1c6966ac080b` |
| `axis-comments.docx` | 9 | 1,019,829 | 43 | 220 threads x depth 8 -> 1,089 comments | `58ddf0130287a168c69620938972497ca470bce9228773b357f2bd6b26bb43c2` |
| `axis-numbering.docx` | 10 | 899,160 | 43 | 300 abstractNum, 60 orphans, 1,200 paragraphs | `3955ee3a1ff074bc12528b06031bd266e3756565f9fafdf3fcae718ccd1ca89b` |
| `axis-styles.docx` | 11 | 851,185 | 43 | 600 styles, basedOn depth 30 | `d8afe3f7c0f5c91d542ae7bda39046e939a51255a655dfc12e5aeceea680fae9` |
| `axis-tables.docx` | 12 | 1,331,043 | 43 | 30 tables, 40x8, nesting depth 3 | `84823e4eab65df85c980108506ed0afc7a38172c9d48a2004f7882b74cddd663` |
| `axis-media.docx` | 13 | 6,473,181 | 89 | 40 media parts, 200 refs, 10 orphans | `b91255ea8b2b62330cb98c247a6373c7fadc7d681029c8b5530377920fc1aa7c` |
| `axis-sections.docx` | 14 | 876,005 | 103 | 12 sections (72 header/footer parts) | `ad4abb8f93e4ba363f47607ab4e92978aebbe5a2e3a62522f264054caba75dd9` |
| `axis-fields-math.docx` | 15 | 862,474 | 43 | 150 fields, 120 math expressions, 400 bookmarks | `a7136d054b8d39f83f7297c3affa7f038998109056945065550bff7d32983364` |
| `small-valid.docx` | 16 | 647,721 | 42 | control: everything on, all knobs low | `83669349c790d3af7f1bdc3c608f05182d12a9415a30838b74b9971ccbbe0a33` |

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
| `00-control.docx` | — | valid control (corrected pic construction) | 3342 | `d2b4ded4c8033d37c22c72b5940378d4b9bc03e72ded2df649339ca353f2d8b3` |
| `01-not-zip.docx` | PKG-NOT-ZIP | not a zip archive | 45 | `7040d0be0810f86cd9eac153915469478ebebe4344f2d3fdd5b6ab2c19fe4347` |
| `02-dup-entry.docx` | PKG-DUP-ENTRY | document.xml twice in the archive | 4220 | `f98cc84fa2ae83128d4656bd8b5b13ed750aeca3602ffc1149c59b8962075df9` |
| `03-case-collision.docx` | PKG-CASE-COLLISION | word/Document.xml + word/document.xml | 4220 | `c9da07108ba06206e27918d50efefef895fda566784bbd5c176655ea8e30e212` |
| `04-no-content-types.docx` | PKG-NO-CONTENT-TYPES | [Content_Types].xml missing | 2934 | `06cf9ee0e3acbc445597a46b074fa5215520a004736a63d9b9f796973d2037e6` |
| `05-no-main.docx` | PKG-NO-MAIN | no officeDocument relationship | 3349 | `ac179ca398a26600764f650abae97529f48f088d0d0755c44ef1d23f9521d7de` |
| `06-ct-undeclared.docx` | CT-UNDECLARED | part with undeclared extension | 3451 | `3fb2b19da4fca38605d2a182d3ef22ee1129c37ea0cece15f0a016cb83d447e6` |
| `07-ct-main-type.docx` | CT-MAIN-TYPE | main part typed application/xml | 3341 | `710d438566b31b57c9c2bb319f8c13ccce594bd38472ca9225ecbcb96166358b` |
| `08-xml-malformed-main.docx` | XML-MALFORMED | unclosed tag in document.xml | 3339 | `40f5c51f28da4e8c8873ed92c5712a80d0c3b75f27fd81bd2f6b71401390ba00` |
| `09-xml-malformed-unused.docx` | XML-MALFORMED | malformed word/unused.xml, unreferenced | 3472 | `58e0bc86752ad42691695a25ca9bcd425d19c5cda68cc6a543d0a8b3dcd1bd09` |
| `10-xml-undeclared-prefix.docx` | XML-UNDECLARED-PREFIX | w15 used, never declared | 3354 | `bb0975e28fb1da902db5c379d55e3cc1b06b68890a7570547d79a2a6d9133896` |
| `11-rel-dup-id.docx` | REL-DUP-ID | rId3 twice in document rels | 3339 | `af190f4a444bdb527f1db485ddc803f99c236fe10eb5c4dbf486497e9ddd07a6` |
| `12-rel-dangling.docx` | REL-DANGLING | image relationship -> missing part | 3344 | `6bba0f0fb9739bc2cc166a4683f57ff7ebb49d8ea515dba85896642730c8f8ac` |
| `13-rel-typed.docx` | REL-TYPED | header relationship -> image part | 3337 | `b74899588a509a5786f9f16d73eeaf2de4c754ba424f326d5db334f7a51ac373` |
| `14-comment-dup.docx` | ID-COMMENT-DUP | two w:comment with id 0 | 3366 | `3e3793816b046e182523e5beaa6f930391b4c60f0787d638e1f48f0486a74bbf` |
| `15-comment-ref.docx` | REF-COMMENT | commentReference -> missing comment id 9 | 3343 | `95f6b8ba142d87ee61f071725f20f59aa18af934d45c07df5a67160fc47f2faf` |
| `16-docpr-dup.docx` | ID-DOCPR-DUP | two pictures, same wp:docPr id | 3364 | `efd02f1f4f2766f44c3adb3efbe2e4745eb3f49a5b5fcc8784dcc2c28dd5a797` |
| `17-bookmark-dup.docx` | ID-BOOKMARK-DUP | two bookmarkStart, same id, different names | 3362 | `3d1847ad30c670d55019429f3c7d441de6e6335c28af5e03ec3179472cd7b52b` |
| `18-revision-dup.docx` | ID-REVISION-DUP | two w:ins, same revision id | 3368 | `9effe62ec6252d7a0b15bec76560ad00fb51eb67b257adf2a082927afcba6f04` |
| `19-note-ref.docx` | REF-NOTE | footnoteReference -> missing footnote id 9 | 3342 | `0e0c6bafec3d836780bf16e024c4f3f1b35ecc98d981f71c53f79a814ca16fc9` |
| `20-header-ref.docx` | REF-HEADER | headerReference -> missing rId99 | 3343 | `7f7d82cca596c926e7e7f78acb6aa424ee371f9e018953a5ec9c84dd63f3e3c2` |
| `21-style-ref.docx` | REF-STYLE | pStyle -> undefined style | 3346 | `3b76eeddbae6e4ba7a49e8e758c1443a75e5c1b86404fb24ed22b1e7b9efe5cd` |
| `22-fld-unbalanced.docx` | FLD-UNBALANCED | fldChar begin, no end | 3380 | `bfef44ade4bc236750cddfa405e5083d57416ba422d2d1f45373bb5eb8864b9c` |
| `23-tc-no-p.docx` | STRUCT-TC-NO-P | table cell with no block-level content | 3404 | `d0c08352cd81d9f3a3fa62bae946d4e0e9f2dc94bfbf0dccbfd117cf3df8b560` |
| `24-deltext.docx` | STRUCT-DELTEXT | w:del containing w:t instead of w:delText | 3367 | `464d59b9c05c31b0d341830d847e94ea36dd370eb694615ba32afc08c03242db` |
| `25-move-unpaired.docx` | STRUCT-MOVE-UNPAIRED | moveFromRangeStart, no end | 3370 | `aeef0df53e79067fa8701c52c34ec873d504cef806682257c904398bdabd1450` |
| `26-media-magic.docx` | MEDIA-MAGIC | JPEG bytes in a .png part typed image/png | 3288 | `9c82a9de2e3bc35504e9764e156fd61a574dd706760ddf1ba03228c79a30f165` |
