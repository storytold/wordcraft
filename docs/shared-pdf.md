# Shared PDF writer

WordCraft uses `craft-pdf` with default features disabled and `write` enabled. Both that dependency
and the required root Krilla patch use the immutable `storytold/craft-libs` revision
`17744430be327d0257496a6a31c8d38fe20a6756`. Rust 1.95 or newer is required. The revision must be
available from the remote before a fresh checkout can resolve it. The shared writer checks a
backend marker at compile time so an unpatched Krilla cannot replace the required backend.

The shared writer owns document construction, metadata serialization, fallible font preparation
and final serialization. WordCraft owns layout, pagination, selected page order, markup and
hidden-text policy, comments, drawing, field text, link annotations, heading outlines and tagged
structure. It retains its bounded retry with glyph outlines when a selected font cannot be
subsetted. Each retry constructs a fresh writer. Readable fonts otherwise remain embedded,
selectable text.

WordCraft supplies its existing metadata policy: option overrides, subject/description fallback,
split keywords, and nonempty ASCII language tags. Creation dates come from the document when valid,
otherwise from the native system clock or the browser clock on WebAssembly. The shared writer
never chooses a clock or changes this policy.

## Backend sources and features

Krilla remains 0.8.2 with raster-image support only: the shared workspace turns Krilla's default
features off, so its naive `simple-text` shaping and the unmaintained `rustybuzz` and `ttf-parser`
crates behind it are not built. WordCraft shapes text itself and hands Krilla positioned glyphs, so
it does not enable the shared `write-simple-text` feature. WordCraft does not embed existing PDF
pages, so it does not enable Krilla's optional `pdf` feature or the shared `write-pdf` feature.
The shared writer has no parser dependency. Hayro syntax and interpretation remain explicit test
dependencies at their existing registry versions; no parser or codec fork is introduced. The
existing `craft-print` and `craft-ui` dependency pins are retained separately.

Krilla's existing bundled profiles retain their upstream licences, recorded in the pinned shared
repository's `licenses/pdf-writer/` and `ATTRIBUTION.md`. This migration
adds no WordCraft font or profile binaries.

## Failure handling

Both public PDF export functions guard the full native operation, including layout where that
function performs it. The vendored Krilla skips surface/page finalization while already unwinding,
preventing a second destructor panic from aborting before the guard can return an error. Failed
page or surface cleanup permanently marks that document as failed, so final serialization rejects
a retained writer even if later pages finish successfully. Successful serialization is unchanged.

Unreadable fonts retain WordCraft's existing skip policy. Font allocation, variation-validation
and backend failures fail the export instead of returning incomplete text. A serialization error
for an unsubsettable font still enters WordCraft's existing bounded outline retry. The guard does
not cover preparation done by a caller before invoking a PDF export function, WebAssembly aborts,
or process-wide allocation failure.
