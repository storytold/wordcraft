# Native InkML rendering profile

> **Status:** 2026-11-08 · Composed circular-width fitting and zero-area export guards; bounded, partial previews. Synthetic evidence only;
> Word-written file validation and native Word visual comparison remain deferred.

The decoder follows the public [W3C InkML Recommendation](https://www.w3.org/TR/2011/REC-InkML-20110920/).
This is **not full InkML conformance or native stroke editing**. Original XML, metadata,
relationships and picture fallbacks remain authoritative for DOCX saves; disposable preview
geometry never replaces the original part with WordCraft freeform shapes.

## Supported subset

| Area | Implemented rendering profile |
|---|---|
| XML | Bounded strict UTF-8 or BOM-marked UTF-16; XML 1.0/1.1 literal/attribute newline and character rules; scoped namespaces, qualified/normalized XML IDs, duplicate expanded attributes rejected, opaque annotations |
| Trace data | Decimal/exponent/hex tokens, explicit/first-/second-difference samples, `*`, numeric/boolean channels, sparse intermittent values/defaults and `?`; custom nonvisual channels consumed without interpretation |
| Contact and continuity | `S` overrides trace contact state; known pen-up samples break runs; unknown contact refuses the whole preview; begin/middle/end continuations join independently decoded fragments; forks and invalid prior references refused |
| Contexts | Local brushes, sources and contexts, inheritance and child-over-reference precedence, declaration-time stream contexts and isolated definition defaults |
| Units and sources | Reserved-channel defaults, dimension-checked compound physical units, nominal `em`/`ex`, explicit percentage length/angle ranges, and `dev` coordinates/rotation calibrated from source resolutions |
| Canvases | Local formats and bounds; forward identity, affine, product and scalar Content MathML mappings; mapped visual channels override source channels |
| MathML | Scalar arithmetic and logical/relational operations; integer/real/rational literals, including `sep`; n-ary comparisons, empty sum/product/logical identities, reference rebinding and output-only variables |
| Tables | Complete numerical tensor grids; absolute/relative outputs; floor/middle/ceiling/linear interpolation; univariate natural cubic splines with at least four knots; boolean grids use exact lookup without interpolation |
| Inverses | Full-column-rank affine systems, including rectangular/coupled systems and affine MathML; independent reversible scalar chains; monotone linear/cubic tables; finite boolean/bounded-integer domains; mixed continuous/discrete products partitioned by dependencies |
| Views | Local definition-backed trace/group/view references and nested, one-based inclusive selections; cropped continuations join only retained adjacent boundaries; already-present trace views do not paint their opacity twice |
| Brushes | Ellipse, rectangle and drop nibs; OR rotation; BW/BH/W; packed RGB and normalized RGB/CMYK; transparency/A; normalized pressure; `ignorePressure`, curve fitting, `copyPen`, `noOperation`; binary-component `maskPen` |
| Placement | Inline, floating, direct run-level, group and drawing-canvas ink; inner offsets/extents and rotation about the inner frame, resizing, Hide Ink, screen and vector PDF output |

Only qualified `xml:id` establishes rendering references; IDs remain unique inside opaque
annotations too. Annotation payloads cannot inject brushes, contexts or traces. The parser
checks document envelopes, names, reserved namespace bindings, character references and expanded
attribute uniqueness. Literal XML 1.1 newlines are normalized, not character-reference replacements.
DTD-bearing parts and fragment references under unresolved nonempty `xml:base` refuse decoding.
External resources are never fetched. Additional encodings, DTD/entity processing, base-URI
resolution and complete element/attribute qualification are outside this profile.

## Application policies

Declared length channels without units use millimeters, `F` defaults to `%`, and `OR` to degrees
(InkML §3.1.2). Explicit units take precedence; mappings operate on raw channel values before
canvas-unit conversion. The implicit application format uses point-sized nominal `em`; `ex` is
half a point. Unitless brush dimensions use application points. Percentage lengths map 0–1 through
explicit min/max in application points; percentage rotation ranges use degrees. Missing ranges
are not guessed.

Integer brush transparency uses 0–255; `%` uses fractions 0–1. Pressure has a linear response only
with an explicit range or percentage units, including the default for `F`. Missing/uncalibrated
pressure refuses decoding unless explicitly ignored. Time does not change the pen. Curve fitting
uses bounded Catmull–Rom samples; changing its policy within a connected run refuses the preview.
Curved nibs use 24 vertices. Color and opacity changes are sample-stepped; compound outlines paint
once per continuous paint run within a stroke. Separate strokes retain separate opacity.
Centerlines and circular taps use the mean of the composed X/Y fitting scales, applied once.
Preview metadata retains the axes through JSON recovery so a further nonuniform resize does not
multiply arithmetic means or change thickness after DOCX save/reopen. Outlined nibs scale in both
axes. Direct run-level ink has no persisted outer frame; its intrinsic-size rendering remains
supported, but arbitrary frame resizing cannot be retained in that direct markup.

The separate renderer primitives retain native widths without changing authored/freeform ink's
legacy clamp. Binary mask colors have RGB components of 0 or 255: Multiply equals bitwise AND
for those components, with coverage/opacity applied once. Dark pages use complementary colors
and Screen blending, a display adaptation rather than a native Word fidelity claim. Mask clips
are non-isolated; ordinary clips retain isolated compositing. Dimmed header/body opacity groups
remain isolated, so their masks may not interact with the page as undimmed masks do.

Inverse scalar chains include affine arithmetic, reciprocals, exponentials, fixed-base logarithms,
constant powers/degrees, sign-constrained absolute values, principal arc functions and explicitly
bounded monotone sine/cosine/tangent branches. Min/max require ranges proving identity. Numeric
safety limits never substitute for declared domain bounds. Cubic inversion checks derivative
extrema and solves the original spline, not a swapped-knot approximation. A known forward map
wins. Recovered values must satisfy channel domains and reproduce all known recorded observations
after integer rounding. Discrete comparisons are exact; real comparisons use bounded tolerance.
Continuous inverses of integer-output maps require an integral affine/identity proof.

Finite inverse domains use declared inclusive integer bounds or boolean states. Connected input
dependencies are solved jointly, including quantized observations. Every candidate must evaluate
within numeric and channel limits: failed/exhausted candidates are not silently discarded.
Unknown observations impose no constraint; exactly one state must match. Collisions, close
real-valued alternatives, absent preimages and ambiguous branches refuse the whole preview.

## Preservation and fallbacks

Unsupported required semantics refuse the **whole** preview, not just a stroke. Drawing-level
ink retains its picture or missing-picture box. Direct run-level ink has no outer picture/frame,
so unreadable ink displays a 72 × 36 pt placeholder. Group/canvas members keep their own frames
and optional pictures. Retained namespaces are handled by the separate retained-XML fix.

Readable picture fallbacks are retained independently of decoded geometry through JSON and
document adoption. ODT/OTT uses the whole picture where native vectors cannot be represented,
including group/canvas members; hidden/nonpainting ink does not resurrect a picture. Fully
zero-area swept nibs, including all-zero-pressure strokes, use the nonpainting sentinel while
retaining their bounds for placement; a zero-pressure endpoint of a painted run is not discarded.
Without a
picture, native path export to ODT/OTT remains unsupported. Screen and PDF still use vectors.

Remaining gaps include nonbinary masks, other application-defined raster/visual semantics,
general continuous nonlinear/multivariate inverses, alternative re-renderings of already-visible
trace views, full conformance and native editing/erasing/conversion/replay. `piecewise` is outside
InkML §6.1.1's permitted Content MathML subset and is intentionally unsupported.

## Resource limits and evidence

- Source/transcoded XML: 2 MiB; 8,192 nodes; 64 attributes per element, namespace declarations included.
- XML/reference/mapping/view depth: 16; recorded channels: 16; decoded samples: 100,000 per part,
  including pen-up data and expanded references.
- Separate file-wide ink allowance: 2,000,000 work units, shared through nested DOCX chunks;
  at most 1,000,000 units to decode one part. Parsing, compilation/evaluation, view expansion,
  nib generation and geometry copies are charged. Accepted reservations remain charged if a
  later step fails; refused reservations and unused part allowance are not charged. Ink cannot
  exhaust the independent chart/SmartArt allowance. Successful and failed part decodes are cached.
- Brush color literals: 64 bytes before trimming. Unit expressions: 256 bytes and 64 primitives.
- Lookup tables: 1,024 rows; finite inverse joint domains: 1,024 candidates. Compilation, lookup
  and reverse checks consume work. Cubic inverse evaluation uses at most 64 bisection steps;
  generated inverse expressions retain the 16-level depth cap.

Synthetic tests cover hostile inputs/property generation, geometry/opacity, all placements,
transforms/resizing, JSON, repeated DOCX saves, original-byte/fallback preservation, editing and
undo/redo, Hide Ink, painted pixels and vector-only PDF output. Heavy curved/translucent ink
before later ink, charts and SmartArt retains later items/models, including through nested DOCX
chunks and save/reopen. No Word-written files or native Word comparison are claimed.

## Revision history

- 2026-11-08: Fix nonuniform circular-width resize/save consistency and zero-pressure ODT/OTT
  picture resurrection. Synthetic canvas-member, framed-placement, JSON, raster and export
  regressions; native Word qualification remains deferred.

- 2026-11-08: Extract the bounded decoder/placement profile for the third #5731 split; retain
  per-part/file-wide ink accounting, export fallbacks and hostile-input regressions.
