# Joining consecutive tables

Last updated: 2026-11-11. Initial engine API for Leaf issue #277. Synthetic
regressions only; native Word comparison and Leaf integration are separate work.

`table.joinNext {}` appends the next table's rows to the caret's table. This
creates one structured table; it does not merge cells. Tables must share their
story and block container. One exact default empty paragraph, as produced by
`table.splitTable`, may separate them. Other content is never skipped.

`table.joinNextStatus {}` is a non-mutating command returning `available` and
`reason` (null when available). Rust frontends can use
`wordcraft_engine::cmd::table::join_next_unavailable(&session)` for the same
reason. Dispatch resolves the live caret and target again. No table/block index
returned by a previous menu or query is trusted. `Session::why_disabled` also
uses the same compatibility gate.

The first implementation requires identical, finite positive column grids and
identical table properties. Row grid coverage includes skipped columns and
horizontal spans. Direct row, cell, paragraph and character properties are
moved unchanged, together with their blocks and inline relationships. Nested
tables and joins inside a shared nested cell are supported within the existing
depth limit. Images keep their media keys and original bytes; notes, bookmarks,
comments and fields keep their markers and part identities. The join does not
paste or adopt content into new identities.

The following are deliberately unavailable rather than silently reconciled:

- Different grids, table styles, sizing modes or other table settings.
- Floating tables, captions/descriptions and preserved source table markup or
  wrappers whose relationships/unsupported properties cannot be reconstructed.
- A non-default or source-preserved empty separator; any intervening content,
  section boundary, page break or unrelated container/story. A separator also
  remains content when it inherits visible shading or outer borders from Normal
  or the document defaults.
- Protection, Track Changes, existing document revisions, tracked formatting,
  and content controls/permission markers anywhere in the current story. This
  conservative first API does not try to decide which control locks overlap.
- Row-dependent conditional styles (including bands, first/last row and corner
  regions), including definitions whose look switches are currently off. This
  avoids changing their interpretation or manufacturing direct formatting.
- Repeated header flags in the second table, or a vertical-merge continuation
  in its first row. First-table repeated headers and internal valid merges are
  retained. No new merge is created across the boundary.
- Invalid/empty row grids, malformed vertical continuations, non-prefix headers,
  row-level table exceptions, or a combined row count exceeding the model limit.
  Vertical continuations must start in the same grid column as their predecessor;
  equal spans alone do not make shifted cells a valid merge.
- Nonzero inter-table spacing, different outside/inside horizontal table borders,
  or conflicting cell borders facing the boundary: a join must not erase lines.
- Automatic sizing: joining changes the set of rows contributing to AutoFit. The
  first API requires explicit fixed layout instead of freezing or approximating it.
  Fixed tables also decline single-column cell widths that contradict their grid,
  percentage cell widths and incompatible start origins from the first cell.
  Stored preferred widths can replace a declared grid even in fixed layout.
- Neighboring Caption-style/SEQ paragraphs, range fields, table formulas and
  opaque inline content. These need relationship/formula policies first.
- Equation editing, extended/cell/object selection and formatting protection.

A successful join uses the normal Session transaction: one undo step restores
both original tables, any separator and the previous selection. Redo restores
the joined result. The caret moves to the first paragraph of the first appended
row, descending through a nested cell when necessary. Rejected operations do
not dirty the document or add undo entries. Ordinary deletion and Split Table
remain independent commands.

Tests in `crates/engine/tests/join_tables.rs` cover actual table structure,
compatibility refusals, nested cells, attached content, history and selection,
warm/cold multipage layout, first-table repeated headers, DOCX and ODT reopening,
and PDF generation. DOCX retains the tested per-cell borders. The existing ODT
writer canonicalizes shared lines onto one neighbor, so different border
properties on opposing sides of that line do not round-trip independently. A
separately assembled table reproduces the same normalization without this
command. This exporter limitation is not claimed fixed here. Each format's assertions are separate. These synthetic
controls do not certify all imported documents, native Word parity, or Leaf UI
behavior. In particular, the initial join refuses unchanged source-preserved
ODT tables rather than discarding their raw markup; joins created in the model
are saved and reopened independently in ODT and DOCX.

| Revision | Change |
| --- | --- |
| 2026-11-11 | Added the explicit engine join and live availability API with a conservative compatibility policy. |
