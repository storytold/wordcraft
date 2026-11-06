//! Test helpers other crates' tests share (the `test-support` feature): what an exported PDF
//! draws, read back.

/// Each image drawn, over every page: its rectangle in PDF space (y up), its first pixel and
/// its size in pixels.
pub fn drawn_images(bytes: &[u8]) -> Vec<(kurbo::Rect, [u8; 3], (u32, u32))> {
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
                        if let Some(&[r, g, b]) = d.data.get(..3).and_then(|p| <&[u8; 3]>::try_from(p).ok()) {
                            found = Some((rect, [r, g, b], (d.width, d.height)));
                        }
                    }
                },
                None,
            );
            self.0.extend(found);
        }
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let Ok(pdf) = Pdf::new(bytes.to_vec()) else { return Vec::new() };
    let cache = InterpreterCache::new();
    let mut out = Images(Vec::new());
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        interpret_page(page, &mut ctx, &mut out);
    }
    out.0
}
