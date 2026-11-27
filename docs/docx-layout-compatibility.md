# DOCX layout compatibility

DOCX preservation and rendering are checked separately. A file can retain its
page count and text while still changing glyph positions, picture decoration,
or page breaks. Exporting a saved file through another application does not
validate WordCraft's own renderer.

The compatibility changes retain the following information:

- Source text-run boundaries and validated formatting/editing revision identifiers,
  independently of normalized editable character formatting. A text fingerprint
  invalidates stale hints after text edits; deleted text is not retained. Both
  `rsidRPr` and `rsidR` affect reference-application run placement, even without
  tracked changes. A leading bookmark does not erase the first run's identity.
- SVG alternatives and their raster fallback bytes. Per-use aliases keep two
  different vectors with identical fallback pixels distinct. Copy/paste and
  document adoption remap both relationships together.
- Picture glow, shadow and soft-edge parameters in OOXML integer units. Screen
  and PDF rendering share the glow/shadow masks; soft-edge painting is still a
  rendering gap even though its parameters survive saves.
- Widths of invisible DrawingML outlines. Suppressing ink does not make the
  outline's width irrelevant to text-box positioning in Word.
- Explicit typefaces on generated table-of-contents number prefixes, while the
  entry style supplies their size, weight and colour. Page references remain
  editable PAGEREF fields. A prefix needs a typeface override only when its font
  identity differs from the heading text. A theme slot and an explicit typeface
  remain different identities even when both currently resolve to the same name.
- Latin, East Asian and complex-script theme-font slots survive the editable
  model bridge. Explicit font choices clear inherited theme slots, including a
  choice whose resolved name happens to equal the theme's current font.
- Unchanged themed properties of paragraph, character and table styles, including
  source tint and shade attributes. They are retained as bounded compatibility hints rather than
  flattened to RGB on every save. Style, palette or theme-font edits invalidate
  the hints; malformed hints fall back to the editable model. Logical colour
  references are converted to their physical palette slots before reuse.
- Theme transforms on number-label character properties, including per-list
  level overrides. Labels can have a different tint from their heading text;
  retaining only the paragraph style does not preserve that colour on save.
  Label formatting and palette edits invalidate these source hints too.

Glyph painting uses a 600-dpi em size for body text and labels, while shaping,
advances, caret positions and line metrics retain the nominal size. Rounding
individual advances to that grid is incorrect, including for fixed-pitch fonts.
The physical PDF media box uses that grid too, without scaling the nominal
layout, link destinations or imposition slots.

Line metrics use the same Windows rules on every host: OS/2 typographic metrics
when `USE_TYPO_METRICS` is set, otherwise Windows ascent/descent with external
leading derived from hhea. The hhea table remains a fallback when OS/2 metrics
are unavailable. Equal total line heights do not imply equal baselines: Calibri
has different ascent/descent splits in the two tables. Output-coordinate rounding
is separate from this nominal metric selection and still needs further coverage.

Automatic hyphenation respects explicit unchecked text (`w:noProof`) separately
from hyperlink proofing. The first character of the word fragment remaining on
the current line controls eligibility. An unchecked interior character does not
suppress that fragment's hyphens, and a forced character wrap can expose a checked
continuation. Explicit optional hyphens remain usable with unchecked text or
paragraph-level automatic-hyphen suppression.

Spanish pattern breaks require a vocalic nucleus on each side. In foreign words,
`y` can supply the prefix's nucleus (`tyr-fesa`), but a suffix containing only `y`
is not sufficient (`rafe-sytr`, compared with `rafe-sitr`). This filter does not
replace language patterns or provide a complete syllabification algorithm;
explicit optional hyphens still apply independently of it.

SVG parsing and drawing use the shared bounded Craft adapters. PDF export draws
vectors; the raster renderer samples at the current display resolution with a
bounded cache. An optional SVG that cannot be parsed uses its raster fallback.
Brightness and contrast currently use the raster fallback. SVG font resolution
uses the loaded document font database and bundled generic fallbacks; additional
SVG-only typefaces and complex effects need further parity coverage.

These fixes do not establish universal pixel parity. Automatic hyphenation
opportunities still depend on the language data, font availability can change
metrics, and PDF and screen painting need their own comparisons. Regression
tests generate their own small inputs.

Picture glows and outer shadows use independently selected raster resolutions.
Both start at 200 dpi; positive glow radii use `200 / ceil(radius / 5.76)` and
shadow blur uses `200 / ceil(blur / 2.88)`, with radii in points. Thresholds are
compared in integer EMUs so an exact boundary retains the finer grid. A shadow
includes the translucent glow in its silhouette at the shadow's resolution.
The glow's dilation radius truncates to whole mask pixels before blurring;
rounding it up expands the half-opacity contour beyond the observed band.
Glow padding rounds outward to 0.24 pt and shadow padding to 0.12 pt.
Placement truncates the source origin
to twips, adds the mask's relative offset, then rounds the sum to 0.12 pt and
subtracts one device pixel. Rounding the shadow offset separately is incorrect.
Placement remains outside the cache, so repeated pictures share their mask bytes.
Large canvases and exceptionally large glow dilations retain resource bounds.
Exact canvas alignment and convolution kernels still differ from the reference
application.

Effect colours now preserve the intermediate premultiplication quantization:
rounded premultiplied channels, a truncated reciprocal when converting back to
straight colour, and truncated premultiplication for the final ink. The shared
RGBA8 representation rounds its straight channels to recover that ink when
painted. Generated controls cover 1,160 colour/opacity combinations, including
an independent full-channel ramp and mixed colours. These checks cover the
solid interiors; blur coverage and the reference PDF's premultiplied images
with a black matte remain separate sources of differences at effect edges.

Picture layout imports the outer box on the twip grid while retaining its source
dimensions for saving. A direct picture's internal DrawingML extent is fitted
uniformly inside that box at the top left; the bitmap's own aspect and the aspect
lock flag do not determine this fit. Distinct internal and outer extents survive
DOCX saves, and explicit picture resizing scales the internal extent too. Group
members retain their fractional coordinates and follow their group's scale;
applying the direct-picture fit to them would undo anisotropic group scaling.
