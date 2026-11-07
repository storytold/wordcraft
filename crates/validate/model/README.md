# Content-model tables for `wordcraft-validate`

This directory holds the data behind the grammar (content-model) checks and the tools that
produce it. The chain runs from the ECMA-376 schemas to JSON tables to Rust. Every step is a
deterministic, stdlib-only Python script, and every intermediate file is checked in:

```
ECMA-376 XSDs ──extract_content_model.py──▶ content_model_*.json ──generate_model.py──▶ ../src/model.rs
```

The crate never runs Python. `../src/model.rs` is checked-in generated Rust, and the crate has no
`build.rs`. The tables are release data, not a build input, so offline, wasm and sandboxed builds
need nothing beyond the Rust sources. A change to the tables shows up in review as data lines in
`model.rs`.

## Files

| File | What |
|---|---|
| `extract_content_model.py` | XSD → JSON extractor (v2: namespace-qualified type keys). Profiles: `wml` (default) and `dml`. |
| `content_model_v2.json` | WML tables (`w:`, `m:`, `b:`, `wp:`, `pic:`), Strict |
| `content_model_v2_transitional.json` | WML tables, Transitional (input to the Strict/Transitional differ) |
| `content_model_dml_v2.json` | DrawingML tables (`a:`, `c:`), Strict |
| `content_model_dml_v2_transitional.json` | DrawingML tables, Transitional |
| `generate_model.py` | JSON → `../src/model.rs` generator |
| `verify_model.py` | Drift check: verifies the SHA-256 table below and each JSON's internal consistency |

SHA-256 of the checked-in files:

| File | SHA-256 |
|---|---|
| `extract_content_model.py` | `259d43e4cfe6785c2a62e5b4a68ca61f971332d7205b643c25a5ee1b66b81466` |
| `generate_model.py` | `0732c987ce12378365e9a36c88690fb0bde757830d027af7706bbbe23a55f1fe` |
| `content_model_v2.json` | `f36a42e324bae4cf4d6ce03ebbfb53e2e5729d76ae5f4929332a272d02545978` |
| `content_model_v2_transitional.json` | `92c433577c914785908f42553718cc41b4f12161125596383ab6836ac3c54fbf` |
| `content_model_dml_v2.json` | `9a3e58c9bc70d3b2fa9718a02666b51d2f4822e66ff61e212cdea63e3e569549` |
| `content_model_dml_v2_transitional.json` | `81290399fb8f89aa287d275172b7214378449a54fd3783e42a8f899958dae127` |
| `../src/model.rs` | `2b9afc35bb5d65fa829e17c1a80a661df326f06f00e9b7ccce0723c3ae513d0b` |

## Sources

The schemas are not copied into the repository. They come from the official ECMA-376 5th-edition
archives, which anyone can download:

| Archive | URL | SHA-256 |
|---|---|---|
| ECMA-376 Part 1 (Strict) | https://ecma-international.org/wp-content/uploads/ECMA-376-1_5th_edition_december_2016.zip | `9d0bcad9cf06054785b03762fcfadbf6bab7e54a5f9d69434e34b7fd464d4129` |
| ECMA-376 Part 4 (Transitional) | https://ecma-international.org/wp-content/uploads/ECMA-376-4_5th_edition_december_2016.zip | `bd25da1109f73762356596918bf5ff8b74a1331642dba5f1c1d1dfc6bed34ecd` |

The Part 1 archive contains `OfficeOpenXML-XMLSchema-Strict.zip`, and the Part 4 archive contains
`OfficeOpenXML-XMLSchema-Transitional.zip`. Extract each inner archive to its own directory. Below,
`<strict>` and `<transitional>` are those directories. Key Strict file hashes: `wml.xsd`
`44fc2233c5df2cfbe0b3b50e7024bd8412ec99dfe7176313ef58845fdec5498d`, `dml-main.xsd`
`078871240cff323ddd22a395281421b737fa61508fd39ffaa2947ba2bc01175e`, `dml-chart.xsd`
`189b9631129a7d6c9f971b42b94c8942d260f11f81615f427db059bbf442cdc3`. Each JSON file records the
SHA-256 of every schema file it read in `meta.file_sha256`.

## Regenerating

Run from this directory:

```
python3 extract_content_model.py <strict>                    content_model_v2.json
python3 extract_content_model.py <transitional>              content_model_v2_transitional.json
python3 extract_content_model.py --profile dml <strict>       content_model_dml_v2.json
python3 extract_content_model.py --profile dml <transitional> content_model_dml_v2_transitional.json
python3 generate_model.py --model-dir . --xsd-dir <strict> --out ../src/model.rs
```

Both scripts are deterministic. Re-running either one over the same inputs gives byte-identical
output — and the output does not depend on the schema directories' paths or names, only on the
schema bytes (each JSON records them in `meta.file_sha256`). Then run `cargo test -p wordcraft-validate`. The tests in `src/model_tests.rs` pin the
table sizes, the spot values the grammar checks depend on, and the `ST_NumberFormat` count.

To check the checked-in files have not drifted (no schemas needed):

```
python3 verify_model.py
```

It verifies every SHA-256 in the table above and each JSON's internal consistency, and fails
loudly if any file was edited without regenerating.

`generate_model.py` exits with an error instead of writing a drifted table. It fails when:

- a JSON file is not a v2 (qualified-key) run;
- the Strict counts differ from 794 elements / 380 types (WML) or 541 elements / 367 types (DML);
- a qualified key appears in both profiles with different content;
- one of the bare-name collision families (`CT_R`, `CT_Style`, `CT_OnOff`, `CT_String`, `CT_Text`,
  `CT_TwipsMeasure` as `w:`/`m:`; `CT_Boolean` as `a:`/`c:`) is not two separate rows;
- the number of types whose children differ between Strict and Transitional is not 19 (WML) and
  exactly `c:CT_PrintSettings` (DML);
- a part root (`w:document`, `w:hdr`, `c:chartSpace`, `a:theme`, …) has more than one type
  variant;
- a group context expands to no parent type, unless no type flattens that group at all and every
  in-scope complex type that references it is an empty row (see below);
- an in-scope schema file the extractor reads is missing from `--xsd-dir`.

Four group contexts are flattened into no type, and contribute no parent types. `a:EG_Media` and
`a:EG_TextGeometry` are referenced by no in-scope complex type. `w:EG_RubyContent` and `a:EG_Effect`
are the whole content of `w:CT_RubyContent` and `a:CT_EffectContainer`, as a bare top-level
`xsd:group ref`. The extractor does not flatten that shape and records both types as `empty`. The
generator confirms each case against the `xsd:group ref` sites in the seven in-scope schema files
(`wml.xsd`, `shared-math.xsd`, `shared-bibliography.xsd`, `dml-wordprocessingDrawing.xsd`,
`dml-picture.xsd`, `dml-main.xsd`, `dml-chart.xsd`) and lists them in the `model.rs` header.

## What `model.rs` contains

- `TYPES`: the 747 Strict complex types of both profiles (380 WML + 367 DML), merged into one
  table keyed by qualified name (`w:CT_R`, `m:CT_R`, `a:CT_FontCollection`). The merge resolves
  the WML profile's 18 references into `dml-main.xsd` (for example, `wp:spPr` →
  `a:CT_ShapeProperties`). No pruning is applied.
- `TRANSITIONAL_TYPES`: only the types whose child models differ in Transitional. The grammar
  checks use these as a veto, so a finding must hold under both profiles.
- `ELEMENTS`: every Strict element row, with all its type variants. Each variant's `contexts`
  lists the **parent type keys** it is selected under. The generator expands group contexts
  (`w:EG_RunLevelElts`) to every type whose flattened children include that element through that
  group. It expands global declarations to every type that holds the element and that no other
  variant claims. A (parent type, element) pair claimed by two variants is removed from both, so
  the checker falls back to union membership there.
- `NUM_FMT_VALUES`: the whole `ST_NumberFormat` enumeration from Strict `wml.xsd`, in XSD
  order. The list is never hand-trimmed. A test asserts it has 63 values and includes the Hebrew,
  Arabic, Hindi and Thai formats.

## Scope limits

- `w:CT_RubyContent` (`w:rt`, `w:rubyBase`) and `a:CT_EffectContainer` (`a:cont`, `a:effectDag`)
  have an empty child list in `TYPES`, but the schema allows content there (see above). A grammar
  check must treat these two types as unmodelled, not as "no children allowed".
- Attribute models and simple-type facets are not extracted, apart from `ST_NumberFormat`.
- An element with no row is not checked internally: an unresolved type, `<any>` content such as
  `a:graphicData`, a Transitional-only name, or `w14:`/`w15:`/`mc:` (no ECMA XSD exists for
  these). It is still checked as a child of its parent.
- Choice-group flattening loses a `choice` with `maxOccurs="1"` nested in repeated content, so
  choice-group aggregate maxima cannot be read from the tables.
- `extract_content_model.py`'s docstring mentions the v1 extractor and its `content_model.json`
  output. Those are history from the extraction work and are not shipped here.
