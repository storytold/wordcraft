use super::select::{obj_size, selected, with_obj};
use super::{CmdError, CmdResult, InlineObject, Rgb, Session, Value, Wrap, p};
use wordcraft_doc::picfx::PicFx;

/// `picture.crop`: the crop, and optionally the new display size, as one edit. With a size, a
/// floating picture positioned by offsets moves by what the crop on its left/top side on the page
/// trims (or restores) — for a flipped picture that's the source's right/bottom — so what stays
/// visible keeps its place on the page: a crop handle drop is one command.
pub(super) fn crop(s: &mut Session, v: &Value) -> CmdResult {
    let mut c = [p::f32(v, "left"), p::f32(v, "top"), p::f32(v, "right"), p::f32(v, "bottom")].map(|x| x.unwrap_or(0.0).clamp(0.0, 0.95));
    // The two sides of an axis keep at least 5% of the source between them, as the renderer
    // draws it: the second side stops where the first one already left less than that.
    c[2] = c[2].min(0.95 - c[0]);
    c[3] = c[3].min(0.95 - c[1]);
    let size = |k: &str| p::f32(v, k).map(|x| x.clamp(1.0, 4000.0));
    let (nw, nh) = (size("width"), size("height"));
    with_obj(s, |o| {
        if let InlineObject::Image { crop, w, h, float, .. } = o {
            // The source's full size at the new display size (the visible part is 1-lo-hi of it).
            let full = |len: f32, lo: f32, hi: f32| len / (1.0 - lo - hi).max(0.05);
            // Flipped, the source's right (bottom) edge is the one on the page's left (top).
            let (left, top) = (if float.flip_h { 2 } else { 0 }, if float.flip_v { 3 } else { 1 });
            if let Some(nw) = nw {
                if float.wrap != Wrap::Inline && float.h_align.is_none() {
                    float.x += (c[left] - crop[left]) * full(nw, c[0], c[2]);
                }
                *w = nw;
            }
            if let Some(nh) = nh {
                if float.wrap != Wrap::Inline && float.v_align.is_none() {
                    float.y += (c[top] - crop[top]) * full(nh, c[1], c[3]);
                }
                *h = nh;
            }
            *crop = c;
        }
    })
}

/// A picture's original size: its image's pixels at 96 ppi (the size Insert › Pictures gives it)
/// or a metafile's own frame, before any crop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OriginalSize {
    /// Points.
    pub w: f32,
    pub h: f32,
    /// A bitmap's pixels (None for a metafile).
    pub pixels: Option<(u32, u32)>,
}

/// The original size of the picture showing `media`, read from the image its Picture Format
/// adjustments started from (Compress Pictures may have resampled the current one); None when the
/// image can't be read.
pub fn original_size(s: &Session, media: &str) -> Option<OriginalSize> {
    let key = s.originals.get(media).map_or(media, String::as_str);
    let bytes = s.doc.media.get(key).or_else(|| s.doc.media.get(media))?;
    if let Some((pw, ph)) = wordcraft_render::image_size(bytes) {
        return Some(OriginalSize { w: pw as f32 * 0.75, h: ph as f32 * 0.75, pixels: Some((pw, ph)) });
    }
    let (w, h, _) = wordcraft_render::metafile_size(bytes)?;
    Some(OriginalSize { w, h, pixels: None })
}

/// What a picture's crop leaves of its original size: the size Reset gives back and what a scale
/// of the original counts from. None for anything but a picture whose image can be read.
pub fn cropped_original_size(s: &Session, o: &InlineObject) -> Option<(f32, f32)> {
    let InlineObject::Image { media, crop, .. } = o else { return None };
    let orig = original_size(s, media)?;
    let part = |lo: f32, hi: f32| (1.0 - lo - hi).max(0.05);
    Some((orig.w * part(crop[0], crop[2]), orig.h * part(crop[1], crop[3])))
}

/// Size: `scale` is a percentage of the current size, or with `original` of what the crop leaves
/// of the original size (`original` alone gives that back: the Size dialog's Reset); then `width`
/// and `height`, where one alone keeps the proportions unless `lockAspect` is false.
pub(super) fn size(s: &mut Session, v: &Value) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no object".into()))?;
    let lock = p::bool(v, "lockAspect").unwrap_or(true);
    let (mut w, mut h) = obj_size(&o);
    let scale = p::f32(v, "scale");
    if p::bool(v, "original").unwrap_or(false) {
        let (ow, oh) =
            cropped_original_size(s, &o).ok_or_else(|| CmdError::Disabled("only a picture whose image can be read has an original size".into()))?;
        let k = scale.unwrap_or(100.0) / 100.0;
        (w, h) = (ow * k, oh * k);
    } else if let Some(sc) = scale {
        w *= sc / 100.0;
        h *= sc / 100.0;
    }
    let (nw, nh) = (p::f32(v, "width"), p::f32(v, "height"));
    match (nw, nh) {
        (Some(nw), Some(nh)) => {
            w = nw;
            h = nh;
        }
        (Some(nw), None) => {
            if lock && w > 0.0 {
                h *= nw / w;
            }
            w = nw;
        }
        (None, Some(nh)) => {
            if lock && h > 0.0 {
                w *= nh / h;
            }
            h = nh;
        }
        _ => {}
    }
    // A size that keeps the proportions shrinks on both sides to fit, rather than flattening.
    let keep = match (nw, nh) {
        (Some(_), Some(_)) => false,
        (None, None) => true,
        _ => lock,
    };
    let (w, h) = fit_size(w, h, keep);
    // Any drawing: a group's members scale with its frame (its child space is kept); a chart or
    // SmartArt graphic lays itself out again at the new size.
    let theme = s.doc.settings.theme_colors.clone();
    with_obj(s, |o| {
        o.set_size(w, h);
        crate::cmd::chart::redraw(o, &theme);
        crate::cmd::smart_art::redraw(o, &theme);
    })
}

/// An object size within 1–4000 pt. With `keep`, both sides scale by one factor, the one nearest
/// 1 that brings them within the limits, so the proportions hold; a shape too long and thin for any
/// such factor is clamped side by side.
pub fn fit_size(w: f32, h: f32, keep: bool) -> (f32, f32) {
    let side = |x: f32| if x.is_nan() { 1.0 } else { x.clamp(1.0, 4000.0) };
    if keep && w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 {
        let (lo, hi) = ((1.0 / w).max(1.0 / h), (4000.0 / w).min(4000.0 / h));
        if lo <= hi {
            let k = if lo > 1.0 {
                lo
            } else if hi < 1.0 {
                hi
            } else {
                1.0
            };
            return (side(w * k), side(h * k));
        }
    }
    (side(w), side(h))
}

/// Apply an image operation to the selected picture's bitmap (stored as a new PNG).
pub(super) fn adjust(s: &mut Session, f: impl Fn(image::DynamicImage) -> Result<image::DynamicImage, CmdError>) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    adjust_media(s, &media, |_| {}, f)
}

/// The selected picture's pre-edit media key (itself when never edited).
pub(super) fn original_media(s: &Session) -> Result<String, CmdError> {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    Ok(s.originals.get(&media).cloned().unwrap_or(media))
}

/// Drops brightness and contrast read from the file: an edit baked them from the original.
pub(super) fn clear_lum(fx: &mut PicFx) {
    fx.bright = 0;
    fx.contrast = 0;
}

/// Like [`adjust`], but decodes `base` (a media key, usually the pre-edit original) instead
/// of the picture's current bitmap. The result still chains onto the originals map, so Reset
/// Picture keeps working.
/// `fx` changes the picture's brightness, contrast and transparency from the file (see
/// [`PicFx`]) for an edit that bakes its own.
pub(super) fn adjust_media(
    s: &mut Session,
    base: &str,
    fx: impl Fn(&mut PicFx),
    f: impl Fn(image::DynamicImage) -> Result<image::DynamicImage, CmdError>,
) -> CmdResult {
    let bytes = s.doc.media.get(base).cloned().ok_or_else(|| CmdError::Failed("picture data missing".into()))?;
    // Within the budget pictures are drawn within, checked from the header first (#3644, #3647).
    let img = wordcraft_geom::picture::decode(&bytes, wordcraft_geom::MAX_DECODED_BYTES).map_err(|e| match e {
        wordcraft_geom::picture::Refusal::TooLarge { .. } => too_large(),
        e => CmdError::Failed(format!("can't decode the picture: {e}")),
    })?;
    let out = f(img)?;
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| CmdError::Failed(e.to_string()))?;
    let key = s.doc.add_media(png, "png");
    // Chain onto the ultimate original (the base's, or the current picture's when the base
    // is itself derived), so Reset Picture always finds it.
    let cur = selected(s).and_then(|(_, o)| match o {
        InlineObject::Image { media, .. } => Some(media),
        _ => None,
    });
    let orig = s
        .originals
        .get(base)
        .cloned()
        .or_else(|| cur.as_ref().and_then(|m| s.originals.get(m).cloned()))
        .unwrap_or_else(|| cur.clone().unwrap_or_else(|| key.clone()));
    s.originals.entry(key.clone()).or_insert(orig);
    with_obj(s, |o| {
        if let InlineObject::Image { media, fx: pic_fx, .. } = o {
            *media = key.clone();
            fx(pic_fx);
        }
    })
}

fn too_large() -> CmdError {
    CmdError::Failed("picture is too large to edit".into())
}

/// A `w` × `h` bitmap of `fill` for an edit to draw on, refused when the picture it makes would be
/// too large to draw (two RGBA buffers within the decode budget), before it is allocated (#3611).
fn canvas(w: u32, h: u32, fill: [u8; 4]) -> Result<image::RgbaImage, CmdError> {
    if !wordcraft_geom::decode_fits(w, h, wordcraft_geom::MAX_DECODED_BYTES) {
        return Err(too_large());
    }
    Ok(image::RgbaImage::from_pixel(w, h, image::Rgba(fill)))
}

/// `w` and `h` grown by `pad` on every side, None past `u32`.
pub(super) fn grown(w: u32, h: u32, pad: u32) -> Option<(u32, u32)> {
    let twice = pad.checked_mul(2)?;
    Some((w.checked_add(twice)?, h.checked_add(twice)?))
}

/// What Artistic Effects, Picture Styles and Picture Border drew on a picture's bitmap: the
/// bitmap they started from and the effect, style and border on it, so the next one draws again
/// from that bitmap instead of over the last result. Picking a second style replaces the first,
/// as in Word, instead of framing the frame (#2052). Kept in [`Session::picture_looks`] under the
/// media key of the drawn bitmap.
#[derive(Clone, Debug, Default)]
pub struct PictureLook {
    /// The media key of the bitmap before any of them.
    pub base: String,
    pub effect: Option<String>,
    pub style: Option<String>,
    pub border: Option<(Rgb, u32)>,
    /// The drawn bitmap: a media key that names other bytes later (undo then a new picture
    /// reusing the key) is not this look's.
    pub drawn: std::sync::Arc<Vec<u8>>,
}

/// Change the selected picture's effect, style or border and draw it again from the bitmap
/// under them (effect, then style, then border).
pub(super) fn relook(s: &mut Session, change: impl FnOnce(&mut PictureLook)) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    let cur = s.doc.media.get(&media).cloned();
    let mut look = s
        .picture_looks
        .get(&media)
        .filter(|l| cur.as_ref().is_some_and(|c| std::sync::Arc::ptr_eq(c, &l.drawn)) && s.doc.media.contains_key(&l.base))
        .cloned()
        .unwrap_or_else(|| PictureLook { base: media.clone(), ..Default::default() });
    change(&mut look);
    if look.effect.is_none() && look.style.is_none() && look.border.is_none() {
        // Nothing left on it: the bitmap as it was.
        let base = look.base.clone();
        return with_obj(s, |o| {
            if let InlineObject::Image { media, .. } = o {
                *media = base.clone();
            }
        });
    }
    let l = look.clone();
    // Styles, borders and effects leave the file's brightness, contrast and transparency alone.
    let out = adjust_media(
        s,
        &look.base,
        |_| {},
        move |img| {
            let mut img = match &l.effect {
                Some(e) => artistic_effect(img, e),
                None => img,
            };
            if let Some(st) = &l.style {
                img = picture_style(img, st)?;
            }
            if let Some((c, w)) = l.border {
                img = frame(img, w, [c.0, c.1, c.2, 255])?;
            }
            Ok(img)
        },
    )?;
    if let Some((_, InlineObject::Image { media, .. })) = selected(s)
        && let Some(drawn) = s.doc.media.get(&media).cloned()
    {
        s.picture_looks.insert(media, PictureLook { drawn, ..look });
    }
    Ok(out)
}

fn artistic_effect(img: image::DynamicImage, effect: &str) -> image::DynamicImage {
    match effect {
        "blur" => img.blur(3.0),
        "sharpen" => img.unsharpen(2.0, 2),
        "invert" => {
            let mut i = img;
            i.invert();
            i
        }
        "posterize" => {
            let mut rgba = img.to_rgba8();
            for px in rgba.pixels_mut() {
                for c in px.0.iter_mut().take(3) {
                    *c = (*c / 64) * 85;
                }
            }
            image::DynamicImage::ImageRgba8(rgba)
        }
        "pixelate" => {
            let (w, h) = (img.width().max(1), img.height().max(1));
            img.resize_exact((w / 12).max(1), (h / 12).max(1), image::imageops::FilterType::Nearest).resize_exact(
                w,
                h,
                image::imageops::FilterType::Nearest,
            )
        }
        _ => img,
    }
}

pub(super) fn reset(s: &mut Session, _: &Value) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    let orig = s.originals.get(&media).cloned().unwrap_or(media);
    // Like Word, Reset Picture discards every correction, those read from the file (`fx`:
    // brightness, contrast, transparency, #1711, #1766) included.
    with_obj(s, |o| {
        if let InlineObject::Image { media, crop, fx, .. } = o {
            *media = orig.clone();
            *crop = [0.0; 4];
            *fx = PicFx::default();
        }
    })
}

pub(super) fn recolor(img: image::DynamicImage, mode: &str, sat: f32) -> image::DynamicImage {
    let mut rgba = img.to_rgba8();
    for px in rgba.pixels_mut() {
        let [r, g, b, a] = px.0;
        let (rf, gf, bf) = (r as f32, g as f32, b as f32);
        let l = 0.299 * rf + 0.587 * gf + 0.114 * bf;
        let out = match mode {
            "grayscale" => [l, l, l],
            "sepia" => [
                (0.393 * rf + 0.769 * gf + 0.189 * bf).min(255.0),
                (0.349 * rf + 0.686 * gf + 0.168 * bf).min(255.0),
                (0.272 * rf + 0.534 * gf + 0.131 * bf).min(255.0),
            ],
            "washout" => [rf * 0.35 + 255.0 * 0.65, gf * 0.35 + 255.0 * 0.65, bf * 0.35 + 255.0 * 0.65],
            "blackAndWhite" => {
                let v = if l > 127.0 { 255.0 } else { 0.0 };
                [v, v, v]
            }
            "tint" => [l * 0.6 + 0.4 * 0x3B as f32, l * 0.6 + 0.4 * 0x5B as f32, l * 0.6 + 0.4 * 0xDB as f32],
            _ => {
                let k = sat / 100.0;
                [l + (rf - l) * k, l + (gf - l) * k, l + (bf - l) * k]
            }
        };
        px.0 = [out[0].clamp(0.0, 255.0) as u8, out[1].clamp(0.0, 255.0) as u8, out[2].clamp(0.0, 255.0) as u8, a];
    }
    image::DynamicImage::ImageRgba8(rgba)
}

/// Flood the border-connected region close to the corner colour with transparency.
pub(super) fn remove_background(img: image::DynamicImage, tol: f32) -> image::DynamicImage {
    let mut rgba = img.to_rgba8();
    clear_background(&mut rgba, tol);
    image::DynamicImage::ImageRgba8(rgba)
}

/// [`remove_background`] on `rgba`, a row span at a time: besides the border's seeds, the stack
/// holds one for each run of matching pixels beside a filled span, instead of four for every
/// pixel filled (#3647). Returns the most seeds it held.
pub(super) fn clear_background(rgba: &mut image::RgbaImage, tol: f32) -> usize {
    let (w, h) = rgba.dimensions();
    let Some(bg) = rgba.get_pixel_checked(0, 0).map(|p| p.0) else { return 0 };
    let close = |p: [u8; 4]| (0..3).map(|i| (p[i] as f32 - bg[i] as f32).abs()).fold(0.0f32, f32::max) <= tol;
    let at = |x: u32, y: u32| y as usize * w as usize + x as usize;
    // Not cleared yet, and close to the corner colour.
    let open = |rgba: &image::RgbaImage, seen: &[bool], x: u32, y: u32| {
        !seen.get(at(x, y)).copied().unwrap_or(true) && rgba.get_pixel_checked(x, y).is_some_and(|p| close(p.0))
    };
    let mut seen = vec![false; (w as usize) * (h as usize)];
    let mut stack = border_pixels(w, h);
    let mut most = stack.len();
    while let Some((x, y)) = stack.pop() {
        if !open(rgba, &seen, x, y) {
            continue;
        }
        let (x0, x1) = run_span(x, y, w, |xi, yi| open(rgba, &seen, xi, yi));
        for xi in x0..=x1 {
            if let Some(s) = seen.get_mut(at(xi, y)) {
                *s = true;
            }
            if let Some(p) = rgba.get_pixel_mut_checked(xi, y) {
                p.0[3] = 0;
            }
        }
        for ny in [y.checked_sub(1), y.checked_add(1).filter(|&n| n < h)].into_iter().flatten() {
            push_open_runs(&mut stack, x0, x1, ny, |xi, yi| open(rgba, &seen, xi, yi));
        }
        most = most.max(stack.len());
    }
    most
}

/// Every pixel on the image's edge, the flood-fill seeds.
fn border_pixels(w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut stack = Vec::new();
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h - 1));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w - 1, y));
    }
    stack
}

/// The horizontal run of open pixels around `(x, y)`, both ends included.
fn run_span(x: u32, y: u32, w: u32, open: impl Fn(u32, u32) -> bool) -> (u32, u32) {
    let (mut x0, mut x1) = (x, x);
    while x0 > 0 && open(x0 - 1, y) {
        x0 -= 1;
    }
    while x1 + 1 < w && open(x1 + 1, y) {
        x1 += 1;
    }
    (x0, x1)
}

/// Queues the first pixel of each open run in row `ny` between `x0` and `x1`.
fn push_open_runs(stack: &mut Vec<(u32, u32)>, x0: u32, x1: u32, ny: u32, open: impl Fn(u32, u32) -> bool) {
    let mut xi = x0;
    while xi <= x1 {
        if open(xi, ny) {
            stack.push((xi, ny));
            while xi <= x1 && open(xi, ny) {
                xi += 1;
            }
        } else {
            xi += 1;
        }
    }
}

fn frame(img: image::DynamicImage, border: u32, color: [u8; 4]) -> Result<image::DynamicImage, CmdError> {
    let (w, h) = grown(img.width(), img.height(), border).ok_or_else(too_large)?;
    let mut out = canvas(w, h, color)?;
    image::imageops::overlay(&mut out, &img.to_rgba8(), border as i64, border as i64);
    Ok(image::DynamicImage::ImageRgba8(out))
}

pub(super) fn picture_style(img: image::DynamicImage, style: &str) -> Result<image::DynamicImage, CmdError> {
    let m = (img.width().max(img.height()) / 40).max(4);
    Ok(match style {
        "thickFrame" => frame(frame(img, m * 2, [255, 255, 255, 255])?, m / 2, [40, 40, 40, 255])?,
        "rounded" | "softEdge" => {
            let mut rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let r = (w.min(h) as f32) * if style == "rounded" { 0.12 } else { 0.0 };
            let feather = if style == "softEdge" { (w.min(h) as f32 * 0.08).max(2.0) } else { 1.5 };
            for y in 0..h {
                for x in 0..w {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let dx = (r - fx).max(fx - (w as f32 - r)).max(0.0);
                    let dy = (r - fy).max(fy - (h as f32 - r)).max(0.0);
                    let edge = (fx.min(w as f32 - fx)).min(fy.min(h as f32 - fy));
                    let inside = if r > 0.0 && dx > 0.0 && dy > 0.0 { r - (dx * dx + dy * dy).sqrt() } else { edge };
                    let k = (inside / feather).clamp(0.0, 1.0);
                    let p = rgba.get_pixel_mut(x, y);
                    p.0[3] = (p.0[3] as f32 * k) as u8;
                }
            }
            image::DynamicImage::ImageRgba8(rgba)
        }
        "shadow" => {
            let (w, h) = (img.width(), img.height());
            let off = m;
            let (pw, ph) = grown(w, h, off).ok_or_else(too_large)?;
            let mut out = canvas(pw, ph, [0, 0, 0, 0])?;
            // A rectangle of alpha 90 the picture's size, one and a half margins in, blurred. Its
            // blur is the product of its sides' blurs along each axis, so it is drawn from two
            // profiles, not by blurring every pixel with a kernel that grows with the picture (#3647).
            let (sigma, start) = (off as f32 / 2.0, off + off / 2);
            let (xs, ys) = (blurred_span(pw, start, w, sigma), blurred_span(ph, start, h, sigma));
            for (x, y, p) in out.enumerate_pixels_mut() {
                let k = xs.get(x as usize).copied().unwrap_or(0.0) * ys.get(y as usize).copied().unwrap_or(0.0);
                p.0[3] = (90.0 * k).round() as u8;
            }
            image::imageops::overlay(&mut out, &img.to_rgba8(), 0, 0);
            image::DynamicImage::ImageRgba8(out)
        }
        _ => frame(img, m, [255, 255, 255, 255])?,
    })
}

/// How much of a span `len` long from `start` covers each of `n` positions once blurred with the
/// Gaussian `image::imageops::blur` uses for `sigma` (outside the `n` positions counts as clear):
/// 0 to 1. Each position is one difference of the kernel's running sums.
fn blurred_span(n: u32, start: u32, len: u32, sigma: f32) -> Vec<f32> {
    let kernel = gaussian_kernel(sigma);
    let mut sums = vec![0.0f32];
    for k in &kernel {
        sums.push(sums.last().copied().unwrap_or(0.0) + k);
    }
    let (size, r) = (kernel.len() as i64, (kernel.len() / 2) as i64);
    let sum_to = |k: i64| sums.get(k.clamp(0, size) as usize).copied().unwrap_or(0.0);
    let (start, end) = (i64::from(start), i64::from(start) + i64::from(len));
    // Position i takes weight k from i + k - r, inside the span for k in [start - i + r, end - i + r).
    (0..i64::from(n)).map(|i| sum_to(end - i + r) - sum_to(start - i + r)).collect()
}

/// The normalized kernel `image::imageops::blur` (0.25) uses for `sigma`: odd, about 6.7 σ wide.
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let size = ((((sigma - 0.8) / 0.3 + 1.0) * 2.0 + 1.0).max(3.0) as usize) | 1;
    let mean = (size / 2) as f32;
    let weights: Vec<f32> = (0..size).map(|x| (-0.5 * ((x as f32 - mean) / sigma).powi(2)).exp()).collect();
    let total: f32 = weights.iter().sum();
    weights.into_iter().map(|v| v / total).collect()
}
