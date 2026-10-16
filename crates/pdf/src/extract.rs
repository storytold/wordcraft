//! Extract Unicode text from PDF bytes (for export verification).

use hayro_cmap::BfString;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

/// What one page yielded: its text, the text outside page furniture, and how much of the page
/// its own pictures cover (so a scan can be told from a figure beside real text).
struct PageText {
    /// Every glyph drawn, page furniture (headers, footers, watermarks) included.
    all: String,
    /// Glyphs outside artifact-marked content: the document's own text, not its furniture.
    content: String,
    /// A picture outside artifact-marked content was drawn: a scan, photograph or figure.
    image: bool,
    /// Page-space area covered by pictures outside artifact-marked content, clamped finite.
    image_area: f64,
    /// The page's own area (its intersected crop box), used to judge picture coverage.
    page_area: f64,
}

struct Extractor {
    all: String,
    content: String,
    image: bool,
    image_area: f64,
    /// Marked-content nesting, innermost last; `true` where the sequence is an artifact.
    marks: Vec<bool>,
}

impl Extractor {
    fn in_artifact(&self) -> bool {
        self.marks.iter().any(|artifact| *artifact)
    }
}

impl Device<'_> for Extractor {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, g: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
        if matches!(mode, GlyphDrawMode::Stroke(_)) {
            return;
        }
        let artifact = self.in_artifact();
        match g.as_unicode() {
            Some(BfString::Char(c)) => {
                self.all.push(c);
                if !artifact {
                    self.content.push(c);
                }
            }
            Some(BfString::String(s)) => {
                self.all.push_str(&s);
                if !artifact {
                    self.content.push_str(&s);
                }
            }
            None => {
                self.all.push('\u{FFFD}');
                if !artifact {
                    self.content.push('\u{FFFD}');
                }
            }
        }
    }
    fn draw_image(&mut self, image: Image<'_, '_>, transform: kurbo::Affine) {
        // Page furniture can carry a logo or watermark image; that is not document content.
        if !self.in_artifact() {
            self.image = true;
            let area = placed_image_area(&image, transform);
            if area.is_finite() && area > 0.0 {
                self.image_area = (self.image_area + area).min(MAX_AREA);
            }
        }
    }
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    // Headers, footers and watermarks are written inside an `Artifact` marked-content sequence.
    fn begin_marked_content(&mut self, tag: &[u8], _: Option<i32>) {
        self.marks.push(tag == b"Artifact");
    }
    fn end_marked_content(&mut self) {
        self.marks.pop();
    }
}

/// Cap on a page's counted picture area, so a hostile transform can't overflow the sum.
const MAX_AREA: f64 = 1.0e12;

/// The page-space area a picture covers. The interpreter's transform maps the picture's own pixel
/// space to the page, so the placed rectangle's bounding box is the picture's extent. Anything
/// non-finite or empty counts as no coverage.
fn placed_image_area(image: &Image<'_, '_>, transform: kurbo::Affine) -> f64 {
    let w = image.width() as f64;
    let h = image.height() as f64;
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let corners = [
        transform * kurbo::Point::new(0.0, 0.0),
        transform * kurbo::Point::new(w, 0.0),
        transform * kurbo::Point::new(0.0, h),
        transform * kurbo::Point::new(w, h),
    ];
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in corners {
        if !p.x.is_finite() || !p.y.is_finite() {
            return 0.0;
        }
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    let area = (max_x - min_x) * (max_y - min_y);
    if area.is_finite() { area.max(0.0) } else { 0.0 }
}

/// Per page, the text inside and outside artifact-marked content, and whether the page drew a
/// picture of its own.
fn extract_pages(bytes: &[u8]) -> Result<Vec<PageText>, String> {
    if bytes.len() < 8 || !bytes.starts_with(b"%PDF") {
        return Err("not a PDF".into());
    }
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("{e:?}"))?;
    let cache = InterpreterCache::new();
    let mut out = Vec::new();
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        let mut ex = Extractor { all: String::new(), content: String::new(), image: false, image_area: 0.0, marks: Vec::new() };
        interpret_page(page, &mut ctx, &mut ex);
        let box_ = page.intersected_crop_box();
        let page_area = (box_.width().max(0.0) * box_.height().max(0.0)).min(MAX_AREA);
        out.push(PageText { all: ex.all, content: ex.content, image: ex.image, image_area: ex.image_area, page_area });
    }
    Ok(out)
}

/// Unicode extracted from every page (empty strings for pages with no text), page furniture
/// included. [`extract_verdict`] is what judges a PDF's text layer, leaving furniture out.
pub fn extract_page_text(bytes: &[u8]) -> Result<Vec<String>, String> {
    Ok(extract_pages(bytes)?.into_iter().map(|p| p.all).collect())
}

/// How much of a page a picture has to cover before a page with only a little text on it is read
/// as a scan rather than a text page with a small figure.
const SCAN_COVERAGE: f64 = 0.15;
/// What a PDF's text layer looks like once every page is considered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every page carries its own extractable text, or is blank or furniture-only.
    Searchable,
    /// The document carries no extractable text of its own and no picture that could hold some:
    /// nothing is extractable, so it is read as not searchable.
    NotSearchable,
    /// A page draws a picture that dominates it, so the page's own text (if any) cannot verify the
    /// picture's content.
    Uncertain,
}

/// Judge a PDF's text layer page by page.
///
/// A page counts as carrying a text layer when it has any extractable character of its own outside
/// artifact-marked furniture — a word, a number or a symbol all count; there is no minimum document
/// length, so a fully extractable one-line (or numeric) document is searchable.
///
/// Coverage is what matters, not a single page: a page that draws a picture of its own (a scan,
/// photograph or figure, outside page furniture) and covers most of the page cannot be certified
/// searchable from unrelated text on the same page (a caption, page number, or long paragraph
/// beside the scan) — so a searchable cover followed by scanned body pages is **not** certified
/// searchable, and a scan that is the whole document (with no text layer anywhere) is uncertain
/// too, not confidently without a text layer. Blank and furniture-only pages are allowed.
pub fn extract_verdict(bytes: &[u8]) -> Verdict {
    extract_verdict_detail(bytes).verdict
}

/// Per-page detail for [`extract_verdict`]: which pages could not be certified searchable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerdictDetail {
    pub verdict: Verdict,
    /// 1-based page numbers dominated by a picture whose text layer could not be verified.
    pub unverifiable_pages: Vec<usize>,
}

/// Judge a PDF's text layer and record pages whose scan coverage could not be verified.
pub fn extract_verdict_detail(bytes: &[u8]) -> VerdictDetail {
    let Ok(pages) = extract_pages(bytes) else {
        return VerdictDetail { verdict: Verdict::NotSearchable, unverifiable_pages: Vec::new() };
    };
    let mut any_text = false;
    let mut uncertain = false;
    let mut unverifiable_pages = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        match page_coverage(page) {
            Coverage::Covered => {}
            Coverage::Uncertain => {
                uncertain = true;
                unverifiable_pages.push(index + 1);
            }
        }
        if has_text_layer(&page.content) {
            any_text = true;
        }
    }
    if uncertain {
        // A page dominated by a picture cannot be certified from unrelated text on it — and that
        // holds whether or not the document carries a text layer anywhere. A scan with only a
        // caption or a page number, or no extractable text at all, is an unverified picture, not a
        // document confidently without a text layer.
        return VerdictDetail { verdict: Verdict::Uncertain, unverifiable_pages };
    }
    if !any_text {
        return VerdictDetail { verdict: Verdict::NotSearchable, unverifiable_pages: Vec::new() };
    }
    VerdictDetail { verdict: Verdict::Searchable, unverifiable_pages: Vec::new() }
}

/// One page's text coverage: its own words against any picture that dominates it.
enum Coverage {
    Covered,
    Uncertain,
}

fn page_coverage(page: &PageText) -> Coverage {
    if !page.image {
        // A page with its own text, or a blank or furniture-only page, never leaves a gap; the
        // document-wide "any text at all" rule is what rejects a PDF with no text layer.
        return Coverage::Covered;
    }
    let coverage = if page.page_area > 0.0 && page.image_area.is_finite() { (page.image_area / page.page_area).clamp(0.0, 1.0) } else { 1.0 };
    if coverage < SCAN_COVERAGE {
        // A small figure; the text beside it is the page's own content.
        return Coverage::Covered;
    }
    // A page-dominating picture (a scan) cannot be certified searchable from any extractable text on
    // the same page — captions, page numbers, or long paragraphs beside the scan do not verify the
    // image's content.
    Coverage::Uncertain
}

/// Does the page carry any extractable text of its own (outside artifact-marked furniture)? Any
/// non-whitespace character counts — a word, a number or a symbol — because a text layer is present
/// whatever the characters. Whether that text certifies a page-dominating picture is a different
/// question, answered by [`page_coverage`]: text on the same page never verifies the picture.
fn has_text_layer(content: &str) -> bool {
    content.chars().any(|c| !c.is_whitespace())
}
