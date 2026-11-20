# Hangul documents (HWP, HWPX)

> 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
>
> (This product was developed with reference to Hancom's public document on the Hangul document
> file (.hwp) format. Hancom's terms for that document require this sentence, word for word, in the
> product's interface, manual, help and source.) Hangul and HWP are trademarks of Hancom Inc.
> WordCraft is not affiliated with, sponsored by or endorsed by Hancom Inc.

WordCraft opens, edits and saves documents of Hancom Hangul, the word processor most Korean
schools, governments and companies use: HWPX (OWPML, KS X 6101, a ZIP of XML parts) and HWP 5.0
(a compound file of binary records). There is no separate tool or conversion step: open a `.hwpx`
or `.hwp` file and save it with Ctrl+S.

- **Open**: both formats, also when the extension is missing (recognised by content).
- **Save into the original**: a document opened from a Hangul file is saved back into a copy of
  that file. Only edited paragraphs, tables and pictures are rewritten; everything else keeps its
  original bytes, so what WordCraft doesn't model survives, except where an edit takes something
  next to it along: editing the text of a table cell can drop a field or object only the cell's
  original records held (the save lists each such loss; see `compatibility.md`). Every save starts
  again from the file as first opened, so saving many times doesn't drift. What could not be
  written yet is listed in the `file.save` result and shown in the status bar.
- **New files**: any other document (new, .docx, .odt, the other Hangul format) is written as a new
  `.hwpx` or `.hwp`; charts and diagrams go in as pictures.

**Why**: HWP and HWPX are a regional format. They are little used outside Korea, but they are what
Korea's public sector writes in, from notices to forms that must be sent back in the same format.
Word doesn't read them. Hancom's free viewer only reads, and only for personal, non-commercial use.
So anyone who works in Word and needs those documents has to keep Hangul or its viewer as well.
Both formats are openly documented (Hancom's HWP 5.0 specification and OWPML model; HWPX is the
Korean standard KS X 6101), so WordCraft can support them clean-room, on every platform it runs on.

What works for each feature, by format and direction, is in [compatibility.md](compatibility.md).
On the 20 real documents in the test set (10 HWP, 10 HWPX) the page count matches Hancom's viewer
in 17; two differ by one page and one long document by 5. Rules checked only on documents that
later left the test set are listed in [cleanroom.md](cleanroom.md) as needing a new check.

## How it works

`crates/hwp` (`wordcraft-hwp`) is an L2 crate next to `docx`. The two can't depend on each other;
the engine (L4) joins them.

| Where | What |
| --- | --- |
| `crates/hwp/src/package.rs`, `xml.rs` | The HWPX package (ZIP) and its XML |
| `crates/hwp/src/hwp5.rs` | HWP 5.0 records rebuilt as the same element tree as HWPX, so one converter reads both; each paragraph keeps its record number |
| `crates/hwp/src/header.rs`, `convert.rs`, `drawing.rs` | Header (styles, fonts, shapes), body, and pictures and shapes converted to a DOCX package that `wordcraft-docx` reads. Each paragraph starts with a hidden bookmark naming where it came from (`_hp_<section>_<ordinal>`) |
| `crates/hwp/src/writer.rs` (`write_back`) | Save into an original HWPX: edited paragraphs are found by their bookmarks and written in |
| `crates/hwp/src/hwp5_write.rs` (`write_back_hwp`) | Save into an original HWP: only changed records are rewritten |
| `crates/hwp/src/hwp5_enc.rs`, `hwp5_docinfo.rs` | Paragraphs, tables, sections, headers and footers, notes, page numbers, pictures and shapes as HWP records; new character and paragraph shapes, borders and fonts as DocInfo records |
| `crates/hwp/src/hwpx_out.rs` (`new_hwpx`), `hwp5_new.rs` (`new_hwp`) | New files from any document (a new HWP is made as HWPX first, then turned into records) |
| `crates/hwp/src/fonts.rs` | Hancom fonts that aren't installed are drawn with an installed stand-in of the same kind (Nanum first; Gothic text takes Pretendard when NanumGothic is missing); the original names are kept and written back. When neither the font nor a stand-in is there, opening the document says in the status bar where to get Nanum and Pretendard (`font_notice`) |
| `crates/engine/src/io.rs` | Open (`read_hangul`) and save (`save_hwpx`, `save_hwp`). The original file is kept in `Document::passthrough` (`hwp/source.hwpx` or `hwp/source.hwp`); it is not written into other formats |
| `crates/engine/src/cmd/file.rs` | `file.save` reports `hwpx` / `hwp`: `modified`, `inserted`, `deleted`, `pictures`, `notWritten` |
| `crates/layout/src/kinsoku.rs` | Korean words wrap whole where the Hangul paragraph says so (`ParaProps::keep_hangul_words`). DOCX has no such setting, so the converter records it per paragraph (`Report::keep_words`) and the engine applies it after the read (`wordcraft_hwp::apply_keep_words`) |
| `crates/ui-egui/src/hangul_fonts.rs` | In the browser, a bar on Hangul documents offering the system's installed fonts (the web build bundles no Korean font, so Korean shows as boxes until then) |
| `packaging/windows/wordcraft.wxs`, `packaging/linux/ai.storyteller.wordcraft.*` | `.hwpx` / `.hwp` under "Open with" (MIME types `application/hwp+zip`, `application/x-hwp` on Linux). The default app stays the user's choice |

## Try it

```sh
cargo run --release -p wordcraft -- document.hwp                          # open in the app
cargo run -p wordcraft-hwp --example to_docx -- document.hwpx out.docx    # convert only
cargo run -p wordcraft-hwp --example hwp_records -- document.hwp          # list an HWP file's records
cargo test -p wordcraft-hwp
```

On Windows, `packaging/windows/register-hangul.ps1` adds a local build to Explorer's "Open with" for
the current user, without the installer or administrator rights (`-Remove` takes it out). It uses
`CARGO_TARGET_DIR`'s release build, or the repository's `target` folder; `-Exe` names another.

Tests that need real documents are ignored by default and read a folder you name:

```sh
HWP_SAMPLES=/path/to/documents cargo test -p wordcraft-hwp -- --ignored
HWP_SAMPLES=/path/to/documents cargo test -p wordcraft-engine hwp_tables_keep_objects -- --ignored
```

## Rules for this code

WordCraft's `AGENTS.md` applies (never panic, input is hostile, layers, `cargo xtask ci`). In
addition, see [cleanroom.md](cleanroom.md) for the full record:

- **Sources**: Hancom's published "한글 문서 파일 형식 5.0" (Hangul document file format 5.0) document, the element and
  attribute names of Hancom's [`hwpx-owpml-model`](https://github.com/hancom-io/hwpx-owpml-model)
  (Apache-2.0, names only, listed in [owpml-reference.md](owpml-reference.md)), [MS-CFB], and
  black-box comparison with Hancom's free viewer (open a file, look at the viewer's window).
- **Never read** other HWP/HWPX implementations (pyhwp, hwplib, hwpxlib, rhwp and others) or
  WordCraft PRs #2065 and #3764. Don't disassemble or analyse Hancom's programs.
- **Don't commit** Hancom's specification PDFs or real Hangul documents, and don't write a test
  document's file name or content into the repository; describe what a file showed without
  naming it. Test files written for a check are made by hand or by WordCraft.
- Keep the attribution sentence above in the source headers of `crates/hwp`, NOTICE, About and
  this page.
