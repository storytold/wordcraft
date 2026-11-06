//! Design › Watermark: the picture watermark's size is one answer on screen, in a PDF and in a
//! saved DOCX.

use wordcraft_doc::{CharProps, Document, Paragraph, PartKind, SectionProps, Watermark, WatermarkPicture, para_block};
use wordcraft_layout::{LayoutCache, LayoutOptions, layout};
use wordcraft_render::{RenderOptions, render_page};

/// Each image drawn, over every page: its rectangle in PDF space (y up), its first pixel and
/// its size in pixels.
fn drawn_images(bytes: &[u8]) -> Vec<(kurbo::Rect, [u8; 3], (u32, u32))> {
    use hayro_interpret::font::Glyph;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, ImageData, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        interpret_page,
    };
    use hayro_syntax::Pdf;
    use kurbo::Shape;
    struct Images(Vec<(kurbo::Rect, [u8; 3], (u32, u32))>);
    impl Device<'_> for Images {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, img: Image<'_, '_>, t: kurbo::Affine) {
            let Image::Raster(r) = img else { return };
            let mut found = None;
            r.with_rgba(
                |data, _| {
                    if let ImageData::Rgb(d) = data {
                        let rect = (t * kurbo::Rect::new(0.0, 0.0, d.width as f64, d.height as f64).to_path(0.1)).bounding_box();
                        found = Some((rect, [d.data[0], d.data[1], d.data[2]], (d.width, d.height)));
                    }
                },
                None,
            );
            self.0.extend(found);
        }
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let cache = InterpreterCache::new();
    let mut out = Images(Vec::new());
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut out);
    }
    out.0
}

/// A solid `w`×`h` px PNG.
fn solid_png(w: u32, h: u32) -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([20, 60, 140, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

/// A portrait first section whose header the landscape last section links to ("link to
/// previous"), with a wide picture watermark at Auto (or, when `header` is false, no header at all,
/// so the file gives the watermark a header of its own).
fn portrait_then_landscape(header: bool, scale: Option<f32>) -> Document {
    let mut d = Document::new();
    let mut s1 = SectionProps::default();
    if header {
        s1.headers.default = Some(d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Head", CharProps::default()))]));
    }
    let mut one = Paragraph::with_text("Portrait", CharProps::default());
    one.section = Some(Box::new(s1));
    d.body = vec![para_block(one), para_block(Paragraph::with_text("Landscape", CharProps::default()))];
    d.last_section.set_landscape(true);
    let key = d.add_media(solid_png(400, 200), "png");
    d.settings.watermark = Some(Watermark { picture: Some(WatermarkPicture { media: key, scale, washout: false }), ..Default::default() });
    d
}

/// The width of the blue picture on the middle row of a page as drawn on screen (points).
fn screen_width(d: &Document, page: usize) -> f32 {
    let l = layout(d, &mut LayoutCache::new(), &LayoutOptions::default());
    let p = &l.pages[page];
    let img = render_page(d, p, 1.0, &RenderOptions::default());
    let row = (p.body.y + p.body.h / 2.0) as usize;
    let blue = |x: usize| {
        let i = (row * img.width as usize + x) * 4;
        img.pixels.get(i..i + 3).is_some_and(|c| c.iter().zip([20u8, 60, 140]).all(|(a, b)| a.abs_diff(b) <= 3))
    };
    (0..img.width as usize).filter(|x| blue(*x)).count() as f32
}

/// The widths of the picture watermark on each page of a PDF of `d` (points).
fn pdf_widths(d: &Document) -> Vec<f64> {
    drawn_images(&wordcraft_pdf::export(d, &wordcraft_pdf::PdfOptions::default()).unwrap()).iter().map(|(r, ..)| r.width()).collect()
}

/// The width the first header of a saved DOCX gives the picture watermark, from its shape.
fn docx_width(d: &Document) -> f32 {
    let bytes = wordcraft_docx::write(d).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/header1.xml").unwrap(), &mut xml).unwrap();
    let at = xml.find("width:").expect("a sized shape") + "width:".len();
    xml[at..].split("pt").next().unwrap().parse().unwrap()
}

/// #2954 review: Auto is sized from the first section that names the page's header, the same on
/// every page and in every output: a landscape section linked to a portrait section's header
/// shows the portrait-sized picture on screen, in a PDF and in a saved DOCX (not one re-fitted
/// to its wider page on screen and in a PDF while the file holds the first size).
#[test]
fn auto_picture_watermark_size_is_the_same_on_screen_in_a_pdf_and_in_a_docx() {
    for header in [true, false] {
        let d = portrait_then_landscape(header, None);
        // 400 × 200 px is 300 × 150 pt; the portrait page's margins are 468 pt wide.
        let docx = docx_width(&d);
        assert!((docx - 468.0).abs() < 0.01, "header {header}: the DOCX holds {docx}");
        for page in 0..2 {
            let screen = screen_width(&d, page);
            assert!((screen - docx).abs() <= 2.0, "header {header}: page {page} on screen {screen}, DOCX {docx}");
        }
        let pdf = pdf_widths(&d);
        assert_eq!(pdf.len(), 2, "one per page");
        assert!(pdf.iter().all(|w| (w - f64::from(docx)).abs() < 0.5), "header {header}: PDF {pdf:?}, DOCX {docx}");
        // And the file reads back as Auto, not as a scale.
        let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
        assert_eq!(back.settings.watermark.and_then(|w| w.picture).map(|p| p.scale), Some(None), "header {header}");
    }
}
