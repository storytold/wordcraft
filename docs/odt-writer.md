# ODT writer

> **Status:** body text, named styles, direct formatting and the page from the full model; the
> rest of a document still through the exchange-flow writer (#2669)

`odt::export` writes OpenDocument Text (ODF 1.3) from the full `wordcraft_doc::Document`, the
way the DOCX writer does, instead of lowering it to the exchange `Flow` first (which Markdown,
HTML and RTF still use). The writer lives in `crates/formats/src/odt/write/`. The reader
(`odt::import`) stays on `Flow`.

## Two writers, one switch

The writer lands in parts. Until it writes everything, `odt::export` asks `write::can_write(doc)`
whether the new writer writes everything the document holds; when it doesn't, the document goes
through the exchange-flow writer (`odt::export_flow`), as every document did before. So the
same document is written one way or the other as a whole, never half and half, and no part makes
an export lose what the flow writer kept. Each part widens `can_write`:

| Part | The new writer takes documents with |
|---|---|
| 1 | paragraphs in the default paragraph style, direct paragraph and character formatting, links, bookmarks, tabs, line, page and column breaks, pictures in line; one section without headers and footers |
| 2 (this) | named paragraph and character styles with their `based on` chain |
| 3 | lists and tables |
| 4 | several sections, headers and footers, page numbering restarts |

Tracked changes stay outside `can_write` through all four parts: a document with revisions, or
with Track Changes on, is written by the flow writer, which writes the revisions but not what the
new writer adds (named styles beyond the built-in ones, headers and footers). Toggling Track
Changes therefore changes which writer saves the document. This is the known limit of the series
until the writer writes tracked changes itself; `tests::documents_it_cannot_write_whole_go_to_the_flow_writer`
pins it.

## Architecture

`write::write(doc)` walks the document once and builds the two XML parts:

1. `text::body` writes the body into `content.xml`.
2. `pages::master_styles` writes the page layout and the `Standard` master page into
   `styles.xml`.
3. `styles::common` writes `office:styles`: the default style and the document's paragraph and
   character styles (the model's default paragraph style is `Standard`).
4. `package::zip` packages `content.xml`, `styles.xml`, `meta.xml`, `settings.xml`, the
   manifest and the pictures.

`Ctx` (`mod.rs`) is the state the parts share: `auto_style` and `auto` make automatic styles,
deduplicated, in the file being written (`target`: automatic styles of headers and footers
belong in `styles.xml`, their names start with `M`); `font` declares a font face; `picture`
packages a `Document::media` entry once; `name` makes unique numbered names; `break_next` is a
page or column break waiting for the next block.

### Modules

| File | Writes |
|---|---|
| `package.rs` | zip, manifest, `meta.xml`, `settings.xml`, font declarations |
| `styles.rs` | the default style, named paragraph and character styles with `based on`, automatic paragraph and text styles, tab stops, borders |
| `pages.rs` | the page layout of the section: size, orientation, margins, gutter, columns, page number format |
| `text.rs` | paragraphs and headings, spans, links, bookmarks, tabs, line, page and column breaks, ruby; pictures in line, `PAGE` and `NUMPAGES` fields, other fields as their result, equations as their linear text |

## LibreOffice behaviours the writer works around

The reference is LibreOffice opening the DOCX of the same document: the ODT must lay out the
same.

- `config:config-item-set config:name="ooo:configuration-settings"` is a qualified name: without
  `xmlns:ooo` on the root of `settings.xml` every setting in it is ignored. The settings are the
  layout options LibreOffice gives a document it opens from Word formats.
- Tab stops are relative to the paragraph indent unless `TabsRelativeToIndent` is false; the
  model measures them from the text column, so `settings.xml` turns it off.
- The space above, the space below and contextual spacing are one property: one set alone is
  completed from the default paragraph style, not from the parent. Styles and paragraphs that
  set any of them get all three, resolved along their style chain.
- A child style's tab stops replace its parent's in ODF and add to them in the model: each style
  writes its whole merged list.
- Widow and orphan control is on when the model leaves it unset; ODF's default is off, so the
  default style says so.
- A paragraph ending with a page break: Word draws its mark beside the break and starts the next
  block on the new page, so the break goes on the next block, not on an empty paragraph.

## Checking

- Unit tests (`crates/formats/src/odt/write/tests.rs`) build synthetic documents through the
  model API and read the package back with a small normalizer: styles of both files resolved
  along their parents to the properties in effect, paragraphs with their text. Each test first
  checks that `can_write` takes its document, so it tests this writer and not the flow one.
  Every package is checked as an ODF package (`mimetype` first and stored, the manifest listing
  exactly the files there are); with `ODF_JING` (a jing jar) and `ODF_SCHEMA_DIR` (the OASIS ODF
  1.3 RelaxNG schemas, not vendored) set, every part is also validated against the schema.
- The round trips in `crates/formats/src/tests.rs` and the reader's tests pass through whichever
  writer `can_write` picks.
