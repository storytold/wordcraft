use super::exporter::Exporter;
use super::paint::{append_path, norm};
use super::{
    Arc, Fill, FillRule, Image, LineCap, LineJoin, NormalizedF32, Path, PathBuilder, Picture, PlacedItem, Rect, Size, Stroke, Surface, Transform, rgb,
};

/// Largest picture side we embed (pixels).
const MAX_IMAGE_SIDE: u32 = 30_000;

/// A metafile picture, parsed once. The geometry comes placed by the metafile crate; the bitmaps are
/// embedded images, one per picture item (None for paths and undecodable bitmaps).
pub(crate) struct Vector {
    pic: Picture,
    images: Vec<Option<Image>>,
}

/// Parses a metafile and embeds its bitmaps. None when the picture has no usable size.
pub(crate) fn build_vector(bytes: &[u8]) -> Option<Vector> {
    let mut pic = wordcraft_metafile::parse(bytes).ok()?;
    if !pic.has_size() {
        return None;
    }
    let images = pic.items.iter_mut().map(|it| it.take_pixels().map(|(w, h, px)| Image::from_rgba8(px, w, h))).collect();
    Some(Vector { pic, images })
}

/// A straight RGBA colour as a krilla colour and opacity.
pub(crate) fn paint_of(c: [u8; 4]) -> (rgb::Color, NormalizedF32) {
    (rgb::Color::new(c[0], c[1], c[2]), norm(f32::from(c[3]) / 255.0))
}

/// Narrowest metafile stroke in points. Cosmetic (width 0) pens get it rather than PDF's 0-width
/// "thinnest line": krilla bounds a 0-width stroke by the bare path, which is empty for a horizontal
/// line, so a faded (grouped) picture would clip it away.
const HAIRLINE: f64 = 0.25;

/// Draws a metafile picture into the clip (the picture frame): paths are filled and stroked in page
/// space (strokes keep their width under non-uniform scaling), bitmaps stretched, all faded by `alpha`.
/// Every path sets its fill and stroke, so none carries over; strokes have round caps and joins. Text
/// runs are drawn as text.
pub(crate) fn vector_picture(ex: &mut Exporter, s: &mut Surface, v: &Vector, r: &Rect, crop: &[f32; 4], clip: &Path, alpha: f32) {
    let target = kurbo::Rect::new(f64::from(r.x), f64::from(r.y), f64::from(r.x + r.w), f64::from(r.y + r.h));
    let fade = alpha < 0.999;
    if fade {
        s.push_opacity(norm(alpha));
    }
    s.push_clip_path(clip, &FillRule::NonZero);
    for it in v.pic.place(target, *crop) {
        match it {
            PlacedItem::Path { path, fill, stroke, even_odd } => {
                let Some(p) = path_from(&path) else { continue };
                let fill = fill.map(|c| {
                    let (paint, opacity) = paint_of(c);
                    let rule = if even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                    Fill { paint: paint.into(), opacity, rule }
                });
                let stroke = stroke.map(|(c, w)| {
                    let (paint, opacity) = paint_of(c);
                    Stroke {
                        paint: paint.into(),
                        width: w.max(HAIRLINE) as f32,
                        opacity,
                        line_cap: LineCap::Round,
                        line_join: LineJoin::Round,
                        ..Default::default()
                    }
                });
                s.set_fill(fill);
                s.set_stroke(stroke);
                s.draw_path(&p);
            }
            PlacedItem::Bitmap { index, rect } => {
                let Some(Some(img)) = v.images.get(index) else { continue };
                let Some(size) = Size::from_wh(rect.width() as f32, rect.height() as f32) else { continue };
                s.push_transform(&Transform::from_translate(rect.x0 as f32, rect.y0 as f32));
                s.draw_image(img.clone(), size);
                s.pop();
            }
            PlacedItem::Text { index, tf } => {
                if let Some(wordcraft_metafile::Item::Text(t)) = v.pic.items.get(index) {
                    ex.vector_text(s, t, tf);
                }
            }
        }
    }
    s.set_fill(None);
    s.set_stroke(None);
    s.pop();
    if fade {
        s.pop();
    }
}

/// Draws an SVG picture into `rect` as vectors (krilla-svg), cropped by `crop` and faded by
/// `alpha`. Filters become images at twice the page resolution; text is drawn as outlines in the
/// document fonts the picture was parsed with.
/// `images_ok` says whether krilla can decode the raster images it holds (asked only when the
/// picture fits the vector budget). None when nothing could be drawn (the caller then draws the
/// grey placeholder).
pub(crate) fn svg_picture(
    s: &mut Surface,
    svg: &wordcraft_svg::Svg,
    rect: &Rect,
    crop: &[f32; 4],
    clip: &Path,
    alpha: f32,
    images_ok: impl FnOnce() -> bool,
) -> Option<()> {
    use krilla_svg::{SurfaceExt, SvgSettings};
    let [cl, ct, vis_w, vis_h] = wordcraft_metafile::crop_window(*crop).map(|v| v as f32);
    let (fw, fh) = (rect.w / vis_w, rect.h / vis_h);
    let size = Size::from_wh(fw.max(0.01), fh.max(0.01))?;
    let fade = alpha < 0.999;
    if fade {
        s.push_opacity(norm(alpha));
    }
    s.push_clip_path(clip, &FillRule::NonZero);
    s.push_transform(&Transform::from_translate(rect.x - cl * fw, rect.y - ct * fh));
    // krilla-svg rasterises filters with resvg outside our drawing budget: a picture whose
    // filters would take too much memory goes in as our own budgeted raster instead (#1910
    // review 10).
    let (sw, sh) = svg.size_px();
    let t = s.cur_transform();
    let page = t.sx().hypot(t.ky()).max(t.kx().hypot(t.sy()));
    let scale = page * (size.width() / sw).max(size.height() / sh);
    // krilla decodes the raster images an SVG holds when it writes the file: one it can't decode
    // would fail the whole export (review 11 I-8), so they are tried first.
    let drawn = if svg.pdf_vectors_fit(scale) && images_ok() {
        s.draw_svg(svg.tree(), size, SvgSettings { embed_text: false, filter_scale: 2.0 })
    } else {
        svg_raster(svg, size, page).map(|img| s.draw_image(img, size))
    };
    s.pop();
    s.pop();
    if fade {
        s.pop();
    }
    drawn
}

/// An SVG picture as an image `size` big at twice the page resolution (at most
/// `wordcraft_svg::MAX_RASTER_PIXELS`), drawn within the SVG drawing budget.
fn svg_raster(svg: &wordcraft_svg::Svg, size: Size, page: f32) -> Option<Image> {
    let (sw, sh) = svg.size_px();
    let k = 2.0 * if page.is_finite() && page > 0.0 { page } else { 1.0 };
    let (mut w, mut h) = ((size.width() * k).max(1.0), (size.height() * k).max(1.0));
    let cap = wordcraft_svg::MAX_RASTER_PIXELS as f32;
    if w * h > cap {
        let f = (cap / (w * h)).sqrt();
        (w, h) = ((w * f).max(1.0), (h * f).max(1.0));
    }
    let (w, h) = (w as u32, h as u32);
    let r = svg.render(w, h, [w as f32 / sw, 0.0, 0.0, h as f32 / sh, 0.0, 0.0])?;
    // Premultiplied, as krilla wants it straight.
    let mut px = r.rgba;
    for [r, g, b, a] in px.as_chunks_mut::<4>().0 {
        if *a > 0 && *a < 255 {
            let un = |c: u8| ((u16::from(c) * 255 + u16::from(*a) / 2) / u16::from(*a)).min(255) as u8;
            (*r, *g, *b) = (un(*r), un(*g), un(*b));
        }
    }
    Some(Image::from_rgba8(px, r.width, r.height))
}

/// A kurbo path as a krilla path (None when empty).
fn path_from(b: &kurbo::BezPath) -> Option<Path> {
    let mut pb = PathBuilder::new();
    append_path(&mut pb, b);
    pb.finish()
}

/// Decode a picture for embedding (validated with the `image` crate first, so a broken file
/// can't fail the whole export).
pub(crate) fn load_image(data: &Arc<Vec<u8>>) -> Option<Image> {
    // The shared probe also charges what the decoder holds beside its output (#3644).
    if !wordcraft_geom::picture::probe(data).is_ok_and(|p| p.fits(wordcraft_geom::MAX_DECODED_BYTES)) {
        return None;
    }
    load_image_within(data, wordcraft_geom::MAX_DECODED_BYTES)
}

/// [`load_image`] with an explicit RGBA byte budget (a small one in tests).
pub(crate) fn load_image_within(data: &Arc<Vec<u8>>, budget: u64) -> Option<Image> {
    use image::ImageDecoder;
    let fmt = image::guess_format(data).ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    let reader = || {
        let mut r = image::ImageReader::new(std::io::Cursor::new(data.as_slice())).with_guessed_format().ok()?;
        r.limits(limits.clone());
        Some(r)
    };
    // Header-only size check before anything is allocated (the decoder's buffer at its own pixel
    // size, 8 bytes for 16-bit RGBA, plus the RGBA conversion are charged); the side limits and
    // the default 512 MB allocation cap apply to the header read and the decode alike.
    let (hw, hh, native) = {
        let d = reader()?.into_decoder().ok()?;
        let (w, h) = d.dimensions();
        (w, h, u64::from(d.color_type().bytes_per_pixel()))
    };
    if !wordcraft_geom::decode_peak_fits(hw, hh, native, budget) {
        return None;
    }
    let dec = reader()?.decode().ok()?;
    let (w, h) = (dec.width(), dec.height());
    if w == 0 || h == 0 || w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return None;
    }
    if fmt == image::ImageFormat::Jpeg && matches!(dec.color(), image::ColorType::L8 | image::ColorType::Rgb8) {
        return Image::from_jpeg(data.clone().into(), true).ok();
    }
    Some(Image::from_rgba8(dec.into_rgba8().into_raw(), w, h))
}

#[cfg(test)]
thread_local! {
    /// How often [`image_decodes`] ran on this thread.
    pub(crate) static IMAGE_DECODES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether krilla can embed a raster image of an SVG picture, `kind` as usvg read it: a JPEG as
/// it is (krilla doesn't decode it), a GIF as krilla decodes it (the gif crate's own 50 MB limit;
/// review 12 I-9), a PNG or WebP within the same limits as [`load_image`].
pub(crate) fn image_decodes(kind: &str, data: &Arc<Vec<u8>>) -> bool {
    #[cfg(test)]
    IMAGE_DECODES.with(|n| n.set(n.get() + 1));
    let format = match kind {
        "jpeg" => return true,
        "gif" => {
            let mut opts = gif::DecodeOptions::new();
            opts.set_color_output(gif::ColorOutput::RGBA);
            return opts.read_info(data.as_slice()).ok().and_then(|mut d| d.read_next_frame().ok().flatten().map(|_| ())).is_some();
        }
        "png" => image::ImageFormat::Png,
        "webp" => image::ImageFormat::WebP,
        _ => return false,
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data.as_slice()), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    reader.limits(limits);
    reader.decode().is_ok()
}
