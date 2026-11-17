use super::exporter::Exporter;
use super::paint::{append_path, norm};
use super::{
    Arc, Fill, FillRule, Image, LineCap, LineJoin, NormalizedF32, Path, PathBuilder, Picture, PlacedItem, Rect, Size, Stroke, StrokeDash, Surface,
    Transform, krilla, rgb,
};
use craft_metafile::Item;
use kurbo::Shape as _;
use wordcraft_doc::picfx::PicFx;

/// Largest picture side we embed (pixels).
const MAX_IMAGE_SIDE: u32 = 30_000;

/// A metafile picture, parsed once. The geometry comes placed by the metafile crate; the bitmaps and
/// pattern tiles are embedded images, one per picture item (None for paths, text and undecodable
/// bitmaps). Fills that share a tile share its image. `averages` holds each pattern fill's tile
/// average color (transparent for other items), computed once a tile.
pub(crate) struct Vector {
    pic: Picture,
    images: Vec<Option<Image>>,
    averages: Vec<[u8; 4]>,
}

/// Parses a metafile and embeds its bitmaps and pattern tiles. None when the picture has no usable
/// size.
pub(crate) fn build_vector(bytes: &[u8]) -> Option<Vector> {
    let mut pic = craft_metafile::parse(bytes).ok()?;
    if !pic.has_size() {
        return None;
    }
    let mut tiles: std::collections::HashMap<usize, (Option<Image>, [u8; 4])> = std::collections::HashMap::new();
    let (images, averages) = pic
        .items
        .iter_mut()
        .map(|it| match it {
            Item::Pattern { tile, .. } => tiles
                .entry(Arc::as_ptr(tile) as usize)
                .or_insert_with(|| {
                    let n = (tile.width as usize).saturating_mul(tile.height as usize).saturating_mul(4);
                    let img = (tile.width > 0 && tile.height > 0 && tile.rgba.len() == n)
                        .then(|| Image::from_rgba8(tile.rgba.clone(), tile.width, tile.height));
                    (img, wordcraft_layout::metafile_average_color(&tile.rgba))
                })
                .clone(),
            _ => (it.take_pixels().map(|(w, h, px)| Image::from_rgba8(px, w, h)), [0; 4]),
        })
        .unzip();
    Some(Vector { pic, images, averages })
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
/// space (strokes keep their width under non-uniform scaling), bitmaps stretched (or turned by their
/// transform), pattern fills tiled, each item inside its clip region and clip paths, all faded by
/// `alpha`. Every path sets its fill and stroke, so none carries over; strokes take the pen's caps,
/// joins and dashes. Text runs are drawn as text.
pub(crate) fn vector_picture(ex: &mut Exporter, s: &mut Surface, v: &Vector, r: &Rect, crop: &[f32; 4], clip: &Path, alpha: f32) {
    let target = kurbo::Rect::new(f64::from(r.x), f64::from(r.y), f64::from(r.x + r.w), f64::from(r.y + r.h));
    let fade = alpha < 0.999;
    if fade {
        s.push_opacity(norm(alpha));
    }
    s.push_clip_path(clip, &FillRule::NonZero);
    for it in v.pic.place(target, wordcraft_layout::crop_sides(*crop)) {
        match it {
            PlacedItem::Path { path, fill, stroke, line, even_odd, region, clips } => {
                let Some(p) = path_from(&path) else { continue };
                let fill = fill.map(|c| {
                    let (paint, opacity) = paint_of(c);
                    let rule = if even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                    Fill { paint: paint.into(), opacity, rule }
                });
                let stroke = stroke.map(|(c, w)| {
                    let (paint, opacity) = paint_of(c);
                    pen(paint.into(), opacity, w, &line, &path)
                });
                let Some(n) = push_clips(s, region.as_deref(), &clips) else { continue };
                s.set_fill(fill);
                s.set_stroke(stroke);
                s.draw_path(&p);
                pop_clips(s, n);
            }
            PlacedItem::Bitmap { index, rect, transform, region, clips } => {
                let Some(Some(img)) = v.images.get(index) else { continue };
                let c = transform.as_coeffs();
                // Upright and not mirrored: the bitmap fills `rect`.
                let upright = c[1] == 0.0 && c[2] == 0.0 && c[0] > 0.0 && c[3] > 0.0;
                let (tf, size) = if upright {
                    (Transform::from_translate(rect.x0 as f32, rect.y0 as f32), Size::from_wh(rect.width() as f32, rect.height() as f32))
                } else {
                    // A turned, skewed or mirrored bitmap: `transform` maps the unit square onto its place.
                    let c = c.map(|v| v as f32);
                    (Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5]), Size::from_wh(1.0, 1.0))
                };
                let Some(size) = size else { continue };
                let Some(n) = push_clips(s, region.as_deref(), &clips) else { continue };
                s.push_transform(&tf);
                s.draw_image(img.clone(), size);
                s.pop();
                pop_clips(s, n);
            }
            PlacedItem::Pattern { index, path, even_odd, tile, region, clips } => {
                let Some(p) = path_from(&path) else { continue };
                let Some(craft_metafile::Item::Pattern { tile: t, .. }) = v.pic.items.get(index) else { continue };
                let rule = if even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                let paint = match v.images.get(index) {
                    Some(Some(img)) => tile_pattern(s, img, t.width, t.height, tile).map(|p| (p.into(), norm(1.0))),
                    _ => None,
                };
                let (paint, opacity) = paint.unwrap_or_else(|| {
                    let (c, o) = paint_of(v.averages.get(index).copied().unwrap_or([0; 4]));
                    (c.into(), o)
                });
                let Some(n) = push_clips(s, region.as_deref(), &clips) else { continue };
                s.set_fill(Some(Fill { paint, opacity, rule }));
                s.set_stroke(None);
                s.draw_path(&p);
                pop_clips(s, n);
            }
            PlacedItem::Text { index, transform, clip, region, clips } => {
                let Some(craft_metafile::Item::Text(t)) = v.pic.items.get(index) else { continue };
                let Some(n) = push_clips(s, region.as_deref(), &clips) else { continue };
                let text_clip = clip.and_then(|c| path_from(&c.to_path(0.1)));
                if let Some(c) = &text_clip {
                    s.push_clip_path(c, &FillRule::NonZero);
                }
                ex.vector_text(s, t, transform);
                if text_clip.is_some() {
                    s.pop();
                }
                pop_clips(s, n);
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

/// Pushes a metafile item's clip region and clip paths; pop them with [`pop_clips`] and the count
/// returned. None (nothing pushed) when a clip has no area to draw in.
fn push_clips(s: &mut Surface, region: Option<&kurbo::BezPath>, clips: &[Arc<craft_metafile::PlacedClip>]) -> Option<usize> {
    let mut paths = Vec::with_capacity(clips.len() + 1);
    if let Some(region) = region {
        paths.push((path_from(region)?, FillRule::NonZero));
    }
    for c in clips {
        paths.push((path_from(&c.path)?, if c.even_odd { FillRule::EvenOdd } else { FillRule::NonZero }));
    }
    for (p, rule) in &paths {
        s.push_clip_path(p, rule);
    }
    Some(paths.len())
}

fn pop_clips(s: &mut Surface, n: usize) {
    for _ in 0..n {
        s.pop();
    }
}

/// A metafile pen for `path`: `width` (points, at least [`HAIRLINE`]), the pen's caps, joins,
/// miter limit and dashes (`wordcraft_layout::metafile_dashes`, with the hairline as the pixel).
fn pen(paint: krilla::paint::Paint, opacity: NormalizedF32, width: f64, line: &craft_metafile::LineStyle, path: &kurbo::BezPath) -> Stroke {
    let line_cap = match line.cap {
        craft_metafile::Cap::Round => LineCap::Round,
        craft_metafile::Cap::Square => LineCap::Square,
        craft_metafile::Cap::Flat => LineCap::Butt,
    };
    let line_join = match line.join {
        craft_metafile::Join::Round => LineJoin::Round,
        craft_metafile::Join::Bevel => LineJoin::Bevel,
        craft_metafile::Join::Miter => LineJoin::Miter,
    };
    let dash = if line.dash.is_empty() { None } else { wordcraft_layout::metafile_dashes(&line.dash, width == 0.0, HAIRLINE, path.perimeter(1.0)) };
    Stroke {
        paint,
        width: width.max(HAIRLINE) as f32,
        miter_limit: wordcraft_layout::metafile_miter_limit(line.miter_limit),
        line_cap,
        line_join,
        opacity,
        dash: dash.map(|d| StrokeDash { array: d.iter().map(|v| *v as f32).collect(), offset: 0.0 }),
    }
}

/// A tiling pattern of `img` (`w` × `h` pixels); `tile` maps tile pixels to page space. None when
/// the mapping is not finite.
fn tile_pattern(s: &mut Surface, img: &Image, w: u32, h: u32, tile: kurbo::Affine) -> Option<krilla::paint::Pattern> {
    let c = tile.as_coeffs().map(|v| v as f32);
    if !c.iter().all(|v| v.is_finite()) {
        return None;
    }
    let size = Size::from_wh(w as f32, h as f32)?;
    let mut sb = s.stream_builder();
    let mut ps = sb.surface();
    ps.draw_image(img.clone(), size);
    ps.finish();
    let stream = sb.finish();
    Some(krilla::paint::Pattern { stream, transform: Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5]), width: w as f32, height: h as f32 })
}

/// A kurbo path as a krilla path (None when empty).
fn path_from(b: &kurbo::BezPath) -> Option<Path> {
    let mut pb = PathBuilder::new();
    append_path(&mut pb, b);
    pb.finish()
}

/// Decode a picture for embedding (validated with the `image` crate first, so a broken file
/// can't fail the whole export), with its brightness and contrast `fx` applied to the pixels.
///
/// A JPEG goes in as it is unless `fx` changes its pixels; corrected pixels go in raw and krilla
/// compresses them (Flate), and `Exporter::image` embeds one picture with the same corrections
/// once. Corrected or not, a picture goes through the same size checks and decode, so it never
/// fails here only because it has corrections.
pub(crate) fn load_image(data: &Arc<Vec<u8>>, fx: PicFx) -> Option<Image> {
    // The shared probe also charges what the decoder holds beside its output (#3644).
    if !wordcraft_geom::picture::probe(data).is_ok_and(|p| p.fits(wordcraft_geom::MAX_DECODED_BYTES)) {
        return None;
    }
    load_image_within(data, fx, wordcraft_geom::MAX_DECODED_BYTES)
}

/// [`load_image`] with an explicit RGBA byte budget (a small one in tests).
pub(crate) fn load_image_within(data: &Arc<Vec<u8>>, fx: PicFx, budget: u64) -> Option<Image> {
    let fmt = wordcraft_geom::picture::guess_format(data)?;
    // The decision to draw or not is the shared one (`wordcraft_geom::picture`), the one strict
    // export checks with.
    let dec = wordcraft_geom::picture::decode_for_drawing(data, budget, MAX_IMAGE_SIDE).ok()?;
    let (w, h) = (dec.width(), dec.height());
    if !fx.changes_pixels() && fmt == image::ImageFormat::Jpeg && matches!(dec.color(), image::ColorType::L8 | image::ColorType::Rgb8) {
        return Image::from_jpeg(data.clone().into(), true).ok();
    }
    let mut px = dec.into_rgba8().into_raw();
    fx.apply_rgba(&mut px);
    Some(Image::from_rgba8(px, w, h))
}
