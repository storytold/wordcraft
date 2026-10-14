//! Best-effort PDF reflow into ordinary editable paragraphs and inline pictures.
//! PDF drawing coordinates are evidence for reading order, not a word-processing model.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use hayro_cmap::BfString;
use hayro_interpret::CacheKey;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, ImageData, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
    interpret_page,
};
use hayro_syntax::object::{Dict, Name};
use hayro_syntax::{LoadPdfError, Pdf};
use kurbo::{Affine, BezPath, Point, Rect};
use wordcraft_doc::para::{InlineObject, Run};
use wordcraft_doc::props::TextColor;
use wordcraft_doc::{CharProps, Document, Paragraph, Rgb, para_block};

/// Maximum encoded input size accepted by the reflow importer.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAGES: usize = 500;
const MAX_GLYPHS: usize = 1_000_000;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_FONTS: usize = 4096;
const MAX_PAGE_GLYPHS: usize = 100_000;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_TOTAL_PIXELS: u64 = 64_000_000;
const MAX_IMAGES: usize = 1000;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("PDF import needs a password")]
    NeedsPassword,
    #[error("the PDF password is incorrect")]
    WrongPassword,
    #[error("invalid or unsupported PDF")]
    Invalid,
    #[error("PDF import limit exceeded: {0}")]
    Limit(&'static str),
    #[error("PDF contains no extractable text or supported pictures; outlined text requires OCR")]
    NoContent,
    #[error("PDF picture could not be imported: {0}")]
    Image(String),
}

/// Conversion details for UI, CLI and MCP callers. Warnings describe information not retained.
#[derive(Debug, Default)]
pub struct ImportSummary {
    pub pages: usize,
    pub text_characters: usize,
    pub images: usize,
    pub warnings: Vec<String>,
}

/// Identify PDF content even when the filename has no extension or a misleading one.
pub fn is_pdf(bytes: &[u8]) -> bool {
    // PDF readers commonly accept a header after a short leading wrapper.
    bytes.get(..bytes.len().min(1024)).is_some_and(|b| b.trim_ascii_start().starts_with(b"%PDF-"))
}

/// Import without changing or writing the original PDF. No scripts or external resources run.
pub fn read(bytes: &[u8], password: Option<&str>) -> Result<(Document, ImportSummary), ImportError> {
    if bytes.len() > MAX_BYTES {
        return Err(ImportError::Limit("64 MiB input"));
    }
    let pdf = Pdf::new_with_password(bytes.to_vec(), password.unwrap_or("")).map_err(|e| match e {
        LoadPdfError::Decryption(hayro_syntax::DecryptionError::PasswordProtected) => {
            if password.is_some() {
                ImportError::WrongPassword
            } else {
                ImportError::NeedsPassword
            }
        }
        _ => ImportError::Invalid,
    })?;
    if pdf.pages().is_empty() || pdf.pages().len() > MAX_PAGES {
        return Err(ImportError::Limit("1–500 pages"));
    }
    let mut doc = Document::new();
    doc.body.clear();
    let mut summary = ImportSummary { pages: pdf.pages().len(), ..Default::default() };
    if password.is_some_and(|p| !p.is_empty()) {
        summary.warnings.push("To protect the converted document, set a password when saving it. The PDF password only unlocks the source.".into());
    }
    let mut pixels = 0;
    let mut glyphs = 0;
    let mut text_bytes = 0;
    let mut has_graphics = false;
    let mut missing_unicode = false;
    let mut complex_layout = false;
    let mut picture_only = false;
    let interpretation_warning = Arc::new(AtomicBool::new(false));
    for (index, page) in pdf.pages().iter().enumerate() {
        // Release per-page interpretation caches rather than retaining decoded resources for all pages.
        let cache = InterpreterCache::new();
        let warning = interpretation_warning.clone();
        let settings = InterpreterSettings {
            render_annotations: false,
            warning_sink: Arc::new(move |_| {
                warning.store(true, Ordering::Relaxed);
            }),
            ..Default::default()
        };
        let (w, h) = page.render_dimensions();
        if !w.is_finite() || !h.is_finite() || w > 14_400.0 || h > 14_400.0 {
            return Err(ImportError::Limit("page dimensions"));
        }
        let mut context = Context::new(
            Affine::new(page.initial_transform(true).as_coeffs()),
            Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
            &cache,
            pdf.xref(),
            settings,
        );
        let mut collector = Collector::new(pixels, summary.images, MAX_TEXT_BYTES - text_bytes);
        collector.register_type1_fonts(&page.resources().fonts);
        if let Some(error) = collector.error.take() {
            return Err(error);
        }
        interpret_page(page, &mut context, &mut collector);
        if let Some(error) = collector.error.take() {
            return Err(error);
        }
        pixels = collector.pixels;
        text_bytes += collector.text_bytes;
        glyphs += collector.glyphs.len();
        if glyphs > MAX_GLYPHS {
            return Err(ImportError::Limit("one million text glyphs"));
        }
        summary.text_characters += collector.glyphs.iter().map(|g| g.text.chars().count()).sum::<usize>();
        summary.images += collector.pictures.len();
        picture_only |= collector.glyphs.is_empty() && !collector.pictures.is_empty();
        has_graphics |= collector.has_graphics;
        missing_unicode |= collector.missing_unicode;
        complex_layout |= collector.complex_layout;
        let paragraphs = reflow(collector, &mut doc)?;
        for (i, mut paragraph) in paragraphs.into_iter().enumerate() {
            if index > 0 && i == 0 {
                paragraph.props.page_break_before = Some(true);
            }
            doc.body.push(para_block(paragraph));
        }
    }
    if summary.text_characters == 0 && summary.images == 0 {
        return Err(ImportError::NoContent);
    }
    summary.warnings.push(
        "PDF layout is reconstructed approximately. Tables, columns, forms, links, annotations and document structure are not reconstructed.".into(),
    );
    if picture_only {
        summary.warnings.push("Some pages have no extractable text. Their pictures remain images; OCR is not performed.".into());
    }
    if has_graphics {
        summary.warnings.push("Vector drawings, rules and decorative backgrounds are not imported.".into());
    }
    if missing_unicode {
        summary.warnings.push("Some characters have no usable Unicode mapping and were replaced with �.".into());
    }
    if complex_layout {
        summary.warnings.push(
            "Rotated, right-to-left or unusually positioned content needs review after conversion to horizontal text and inline pictures.".into(),
        );
    }
    if interpretation_warning.load(Ordering::Relaxed) {
        summary.warnings.push("The PDF interpreter reported unsupported or unreadable content; inspect the converted document.".into());
    }
    doc.ensure_nonempty();
    Ok((doc, summary))
}

struct TextGlyph {
    text: String,
    x: f64,
    y: f64,
    end: f64,
    size: f32,
    font: Arc<CharProps>,
    color: Option<TextColor>,
}

struct Picture {
    y: f64,
    x: f64,
    width: f32,
    height: f32,
    bytes: Vec<u8>,
}

struct Collector {
    glyphs: Vec<TextGlyph>,
    pictures: Vec<Picture>,
    fonts: HashMap<u128, Arc<CharProps>>,
    pixels: u64,
    previous_images: usize,
    text_bytes: usize,
    max_text_bytes: usize,
    error: Option<ImportError>,
    has_graphics: bool,
    missing_unicode: bool,
    complex_layout: bool,
}

impl Collector {
    // The interpreter exposes embedded TrueType metadata through Glyph::font_data, but not
    // Type1/standard-font metadata. Its documented cache key identifies the resource dictionary.
    fn register_type1_fonts(&mut self, fonts: &Dict<'_>) {
        for (name, _) in fonts.entries() {
            if self.fonts.len() >= MAX_FONTS {
                self.error = Some(ImportError::Limit("4096 fonts per page"));
                return;
            }
            let Some(font) = fonts.get::<Dict<'_>>(name.as_str()) else { continue };
            if !font.get::<Name<'_>>(b"Subtype").is_some_and(|n| matches!(n.as_str(), "Type1" | "MMType1")) {
                continue;
            }
            let Some(base) = font.get::<Name<'_>>(b"BaseFont") else { continue };
            let base = base.as_str().split_once('+').map_or(base.as_str(), |(_, name)| name);
            let descriptor = font.get::<Dict<'_>>(b"FontDescriptor");
            let weight = descriptor.as_ref().and_then(|d| d.get::<u32>(b"FontWeight")).unwrap_or(400);
            let flags = descriptor.as_ref().and_then(|d| d.get::<u32>(b"Flags")).unwrap_or(0);
            let family = ["-BoldOblique", "-BoldItalic", "-Oblique", "-Italic", "-Bold", "-Roman"]
                .iter()
                .find_map(|suffix| base.strip_suffix(suffix))
                .unwrap_or(base);
            self.fonts.insert(
                font.cache_key(),
                Arc::new(CharProps {
                    font: Some(family.to_string()),
                    bold: Some(weight >= 600 || base.contains("Bold")),
                    italic: Some(flags & 64 != 0 || base.contains("Italic") || base.contains("Oblique")),
                    ..Default::default()
                }),
            );
        }
    }

    fn new(pixels: u64, previous_images: usize, max_text_bytes: usize) -> Self {
        Self {
            glyphs: Vec::new(),
            pictures: Vec::new(),
            fonts: HashMap::new(),
            pixels,
            previous_images,
            text_bytes: 0,
            max_text_bytes,
            error: None,
            has_graphics: false,
            missing_unicode: false,
            complex_layout: false,
        }
    }
}

impl Device<'_> for Collector {
    fn draw_glyph(&mut self, glyph: &Glyph<'_>, transform: Affine, glyph_transform: Affine, paint: &Paint<'_>, mode: &GlyphDrawMode) {
        if self.error.is_some() {
            return;
        }
        if self.glyphs.len() >= MAX_PAGE_GLYPHS {
            self.error = Some(ImportError::Limit("100,000 glyphs per page"));
            return;
        }
        let t = transform * glyph_transform;
        let [a, b, c, d, x, y] = t.as_coeffs();
        let size = (c.hypot(d) * 1000.0) as f32;
        if !t.as_coeffs().iter().all(|v| v.is_finite()) || !(0.1..=1440.0).contains(&size) {
            self.complex_layout = true;
            return;
        }
        self.complex_layout |= b.abs() > a.abs() * 0.1 || a < 0.0;
        let text = match glyph.as_unicode() {
            Some(BfString::Char(c)) => c.to_string(),
            Some(BfString::String(s)) => s,
            None => {
                self.missing_unicode = true;
                "�".into()
            }
        };
        if text.len() > 1024 {
            self.error = Some(ImportError::Limit("Unicode mapping length"));
            return;
        }
        self.complex_layout |= text.chars().any(wordcraft_fonts::is_rtl);
        // Object markers and control characters cannot be placed in paragraph text literally.
        let text: String = text.chars().filter(|c| !c.is_control() && *c != '\u{fffc}').collect();
        if text.is_empty() {
            return;
        }
        // Fill+stroke draws one glyph twice; stroke-only text must still be editable.
        if matches!(mode, GlyphDrawMode::Stroke(_))
            && let Some(last) = self.glyphs.last_mut()
            && last.text == text
            && (last.x - x).abs() < 0.01
            && (last.y - y).abs() < 0.01
        {
            Arc::make_mut(&mut last.font).bold = Some(true);
            return;
        }
        if self.text_bytes + text.len() > self.max_text_bytes {
            self.error = Some(ImportError::Limit("8 MiB decoded text"));
            return;
        }
        self.text_bytes += text.len();
        let (font, advance) = if let Glyph::Outline(g) = glyph {
            if self.fonts.len() >= MAX_FONTS && !self.fonts.contains_key(&g.font_cache_key()) {
                self.error = Some(ImportError::Limit("4096 fonts per page"));
                return;
            }
            let font = self
                .fonts
                .entry(g.font_cache_key())
                .or_insert_with(|| {
                    let mut props = CharProps::default();
                    if let Some(data) = g.font_data() {
                        props.bold = Some(data.weight.is_some_and(|w| w >= 600));
                        props.italic = Some(data.is_italic);
                        props.font = data.postscript_name.map(|name| name.split_once('+').map_or(name.clone(), |(_, family)| family.to_string()));
                    }
                    Arc::new(props)
                })
                .clone();
            (font, g.advance_width().map(f64::from).unwrap_or(500.0))
        } else {
            (Arc::new(CharProps::default()), 500.0)
        };
        let end = (t * Point::new(advance, 0.0)).x;
        self.glyphs.push(TextGlyph { text, x, y, end, size, font, color: text_color(paint) });
    }

    fn draw_image(&mut self, image: Image<'_, '_>, transform: Affine) {
        if self.error.is_some() {
            return;
        }
        let count = u64::from(image.width()) * u64::from(image.height());
        if count == 0
            || count > MAX_IMAGE_PIXELS
            || self.pixels + count > MAX_TOTAL_PIXELS
            || self.previous_images + self.pictures.len() >= MAX_IMAGES
        {
            self.error = Some(ImportError::Limit("picture size/count (16 MP each, 64 MP total, 1000 pictures)"));
            return;
        }
        self.pixels += count;
        let bounds = transform.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(image.width()), f64::from(image.height())));
        if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            self.error = Some(ImportError::Invalid);
            return;
        }
        let mut decoded = None;
        match image {
            Image::Raster(image) => image.with_rgba(
                |data, alpha| {
                    let (w, h) = (data.width(), data.height());
                    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
                        return;
                    }
                    let mut rgba = match data {
                        ImageData::Rgb(data) => image::RgbImage::from_raw(w, h, data.data).map(|i| image::DynamicImage::ImageRgb8(i).into_rgba8()),
                        ImageData::Luma(data) => image::GrayImage::from_raw(w, h, data.data).map(|i| image::DynamicImage::ImageLuma8(i).into_rgba8()),
                    };
                    if let (Some(rgba), Some(alpha)) = (&mut rgba, alpha) {
                        if u64::from(alpha.width) * u64::from(alpha.height) > MAX_IMAGE_PIXELS {
                            return;
                        }
                        let Some(alpha) = image::GrayImage::from_raw(alpha.width, alpha.height, alpha.data) else { return };
                        let alpha = if alpha.dimensions() == (w, h) {
                            alpha
                        } else {
                            image::imageops::resize(&alpha, w, h, image::imageops::FilterType::Triangle)
                        };
                        for (pixel, opacity) in rgba.pixels_mut().zip(alpha.into_raw()) {
                            pixel.0[3] = opacity;
                        }
                    }
                    decoded = rgba;
                },
                None,
            ),
            Image::Stencil(image) => image.with_stencil(
                |mask, paint| {
                    if u64::from(mask.width) * u64::from(mask.height) > MAX_IMAGE_PIXELS {
                        return;
                    }
                    let color = match text_color(paint) {
                        Some(TextColor::Rgb(c)) => c,
                        _ => Rgb::BLACK,
                    };
                    let mut rgba = image::RgbaImage::new(mask.width, mask.height);
                    for (pixel, alpha) in rgba.pixels_mut().zip(mask.data) {
                        *pixel = image::Rgba([color.0, color.1, color.2, alpha]);
                    }
                    decoded = Some(rgba);
                },
                None,
            ),
        }
        let Some(mut decoded) = decoded else {
            self.error = Some(ImportError::Image("unsupported or invalid image data".into()));
            return;
        };
        let [a, b, c, d, _, _] = transform.as_coeffs();
        // Preserve axis-aligned mirrors and quarter turns, including a page's /Rotate.
        if b.abs() < 1e-6 && c.abs() < 1e-6 {
            if a < 0.0 {
                image::imageops::flip_horizontal_in_place(&mut decoded);
            }
            if d < 0.0 {
                image::imageops::flip_vertical_in_place(&mut decoded);
            }
        } else if a.abs() < 1e-6 && d.abs() < 1e-6 {
            decoded = image::imageops::rotate90(&decoded);
            if c > 0.0 {
                image::imageops::flip_horizontal_in_place(&mut decoded);
            }
            if b < 0.0 {
                image::imageops::flip_vertical_in_place(&mut decoded);
            }
        } else {
            self.complex_layout = true;
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        if let Err(e) = decoded.write_to(&mut bytes, image::ImageFormat::Png) {
            self.error = Some(ImportError::Image(e.to_string()));
            return;
        }
        self.pictures.push(Picture {
            x: bounds.x0,
            y: bounds.y0,
            width: bounds.width() as f32,
            height: bounds.height() as f32,
            bytes: bytes.into_inner(),
        });
    }

    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {
        self.has_graphics = true;
    }
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn pop_clip_path(&mut self) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn pop_transparency_group(&mut self) {}
}

fn text_color(paint: &Paint<'_>) -> Option<TextColor> {
    match paint {
        Paint::Color(c) => {
            let [r, g, b, _] = c.to_rgba().to_rgba8();
            Some(TextColor::Rgb(Rgb(r, g, b)))
        }
        Paint::Pattern(_) => None,
    }
}

struct Line {
    glyphs: Vec<TextGlyph>,
    x: f64,
    y: f64,
    end: f64,
    size: f32,
}

/// Sort once, then build lines and runs in linear passes (no repeated paragraph insertion).
fn reflow(mut collector: Collector, doc: &mut Document) -> Result<Vec<Paragraph>, ImportError> {
    collector.glyphs.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    let mut rows: Vec<Line> = Vec::new();
    for glyph in collector.glyphs {
        if let Some(row) = rows.last_mut()
            && (glyph.y - row.y).abs() <= f64::from(row.size.min(glyph.size)) * 0.25
        {
            row.x = row.x.min(glyph.x);
            row.end = row.end.max(glyph.end);
            row.glyphs.push(glyph);
        } else {
            rows.push(Line { x: glyph.x, y: glyph.y, end: glyph.end, size: glyph.size, glyphs: vec![glyph] });
        }
    }
    for row in &mut rows {
        row.glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    let max_width = rows.iter().map(|r| r.end - r.x).fold(0.0_f64, f64::max);
    collector.pictures.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    let mut pictures = collector.pictures.into_iter().peekable();
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut previous: Option<(f64, f64, f64, f32)> = None;
    for row in rows {
        while pictures.peek().is_some_and(|p| p.y <= row.y) {
            if let Some(picture) = pictures.next() {
                paragraphs.push(picture_paragraph(picture, doc)?);
            }
            previous = None;
        }
        let continues = previous.is_some_and(|(x, y, end, size)| {
            row.y > y
                && row.y - y < f64::from(size) * 1.6
                && (row.x - x).abs() < f64::from(size) * 0.6
                && (row.size - size).abs() < 0.5
                && end - x > max_width * 0.75
        });
        if !continues {
            paragraphs.push(Paragraph::new());
        }
        if let Some(p) = paragraphs.last_mut() {
            let mut end = None;
            for glyph in &row.glyphs {
                let mut props = (*glyph.font).clone();
                props.size = Some(glyph.size);
                props.color = glyph.color;
                let gap = end.is_some_and(|e| glyph.x - e > f64::from(glyph.size) * 0.2);
                if (continues && end.is_none() || gap) && !p.text.ends_with(char::is_whitespace) && !glyph.text.starts_with(char::is_whitespace) {
                    append(p, " ", &props);
                }
                append(p, &glyph.text, &props);
                end = Some(glyph.end);
            }
        }
        previous = Some((row.x, row.y, row.end, row.size));
    }
    for picture in pictures {
        paragraphs.push(picture_paragraph(picture, doc)?);
    }
    if paragraphs.is_empty() {
        paragraphs.push(Paragraph::new());
    }
    Ok(paragraphs)
}

fn append(p: &mut Paragraph, text: &str, props: &CharProps) {
    p.text.push_str(text);
    if let Some(run) = p.runs.last_mut()
        && &run.props == props
    {
        run.len += text.len();
    } else {
        p.runs.push(Run { len: text.len(), props: props.clone() });
    }
    if p.runs.len() == 1 {
        p.mark = props.clone();
    }
}

fn picture_paragraph(picture: Picture, doc: &mut Document) -> Result<Paragraph, ImportError> {
    let scale = (doc.last_section.text_width() / picture.width).min((doc.last_section.text_height() - 24.0) / picture.height).min(1.0);
    let media = doc.add_media(picture.bytes, "png");
    let mut p = Paragraph::new();
    p.insert_object(
        0,
        InlineObject::Image {
            media,
            w: picture.width * scale,
            h: picture.height * scale,
            alt: "Imported PDF picture".into(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        },
        &CharProps::default(),
    )
    .map_err(|e| ImportError::Image(e.to_string()))?;
    Ok(p)
}

#[cfg(test)]
mod tests;
