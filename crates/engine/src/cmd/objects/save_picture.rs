//! Save as Picture (#7743): the selected picture written to a file, as Word's picture context menu
//! saves it, so a table pasted as a picture can be sent on as an image.
//!
//! The file's type comes from its extension: PNG, JPEG, GIF, BMP, TIFF or WebP. With no `dpi`, a
//! picture shown as its file is (no crop, no brightness, contrast or transparency) saved in its
//! own format is written byte for byte; any other is drawn as it is shown (cropped, adjusted) at
//! its own resolution: its pixels across the width it is shown at, uncropped. With a `dpi` it is
//! drawn at that resolution. A drawn picture records its resolution in the file, so other programs
//! show and print it at the size it has in the document.

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_geom::Rect;
use wordcraft_layout::display::Draw;

use super::select::selected;
use crate::{CmdError, CmdResult, Session, p};

/// The resolutions Save as Picture offers besides the picture's own, DPI.
pub const DPI_CHOICES: [u32; 6] = [96, 150, 300, 600, 1200, 1500];
/// Lowest resolution a saved picture may be drawn at, DPI.
pub const MIN_DPI: u32 = 36;
/// Highest resolution a saved picture may be drawn at, DPI. A picture past
/// [`wordcraft_geom::MAX_RASTER_PIXELS`] or [`wordcraft_render::MAX_SIDE`] at it is refused.
pub const MAX_DPI: u32 = 2400;
/// The resolution a metafile (WMF, EMF), which has no pixels of its own, is drawn at when no
/// `dpi` is given and it is saved as another type, DPI.
const METAFILE_DPI: f32 = 300.0;
/// Points per inch.
const POINTS_PER_INCH: f32 = 72.0;

/// The file types Save as Picture writes: (extensions, format).
const TYPES: [(&[&str], image::ImageFormat); 6] = [
    (&["png"], image::ImageFormat::Png),
    (&["jpg", "jpeg"], image::ImageFormat::Jpeg),
    (&["gif"], image::ImageFormat::Gif),
    (&["bmp"], image::ImageFormat::Bmp),
    (&["tif", "tiff"], image::ImageFormat::Tiff),
    (&["webp"], image::ImageFormat::WebP),
];

/// The format a path's extension names, if Save as Picture writes it.
pub fn format_of(path: &str) -> Option<image::ImageFormat> {
    let ext = std::path::Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    TYPES.iter().find(|(exts, _)| exts.contains(&ext.as_str())).map(|(_, f)| *f)
}

/// `picture.saveAs`: `{"path"?: string, "dpi"?: n | "original"}`. Without a path the app asks
/// where (and, from the menu, at which resolution first).
pub(super) fn save_as(s: &mut Session, v: &Value) -> CmdResult {
    if selected_picture(s).is_none() {
        return Err(CmdError::Disabled("select a picture first".into()));
    }
    dpi(v)?;
    let Some(path) = p::str(v, "path") else {
        let mut req = json!({"open": "savePicture"});
        if let Some(d) = v.get("dpi").filter(|d| !d.is_null()) {
            req["dpi"] = d.clone();
        }
        s.push_ui_request(req);
        return Ok(json!({"saved": false}));
    };
    let format = format_of(path).ok_or_else(|| CmdError::Params("save a picture as .png, .jpg, .gif, .bmp, .tif or .webp".into()))?;
    let (bytes, mut info) = picture_file(s, v, format)?;
    crate::io::write_file(path, &bytes).map_err(CmdError::Failed)?;
    info["path"] = json!(path);
    Ok(info)
}

/// The selected picture: its media key, shown size (points), crop and adjustments.
fn selected_picture(s: &Session) -> Option<(String, f32, f32, [f32; 4], wordcraft_doc::picfx::PicFx)> {
    match selected(s)? {
        (_, InlineObject::Image { media, w, h, crop, fx, .. }) => Some((media, w, h, crop, fx)),
        _ => None,
    }
}

/// The selected picture as a file of `format` at the resolution `v` asks for (`dpi`, see the
/// module documentation), and what was written: `{"width", "height"` (pixels)`, "bytes", "drawn",
/// "dpi"` (drawn only)`}`. For a host that saves the bytes itself (the browser) as for a path.
pub fn picture_file(s: &Session, v: &Value, format: image::ImageFormat) -> Result<(Vec<u8>, Value), CmdError> {
    let (media, w, h, crop, fx) = selected_picture(s).ok_or_else(|| CmdError::Disabled("select a picture first".into()))?;
    let dpi = dpi(v)?;
    let bytes = s.doc.media.get(&media).cloned().ok_or_else(|| CmdError::Failed("the picture's file is missing from the document".into()))?;
    let raster = wordcraft_geom::picture::probe(&bytes).ok();
    let shown_as_its_file = crop == [0.0; 4] && fx.is_identity();
    // Its own file, unchanged: the same type, nothing cropped or adjusted, no resolution asked.
    if dpi.is_none()
        && shown_as_its_file
        && let Some(r) = raster.as_ref().filter(|r| r.format == format)
    {
        let info = json!({"width": r.width, "height": r.height, "bytes": bytes.len(), "drawn": false});
        return Ok((bytes.to_vec(), info));
    }
    // Its own resolution: its pixels across the width it is shown at, before cropping.
    let uncropped_w = w / (1.0 - crop[0] - crop[2]).max(0.01);
    let dpi = match (dpi, &raster) {
        (Some(d), _) => d,
        (None, Some(r)) => (r.width as f32 / (uncropped_w / POINTS_PER_INCH)).clamp(MIN_DPI as f32, MAX_DPI as f32),
        (None, None) => METAFILE_DPI,
    };
    let scale = dpi / POINTS_PER_INCH;
    let (w_px, h_px) = ((w * scale).round().max(1.0), (h * scale).round().max(1.0));
    let too_large = || CmdError::Failed(format!("the picture is too large to save at {dpi:.0} DPI; choose a lower resolution"));
    if w_px > wordcraft_render::MAX_SIDE as f32 || h_px > wordcraft_render::MAX_SIDE as f32 {
        return Err(too_large());
    }
    if (w_px as u64).saturating_mul(h_px as u64) > wordcraft_geom::MAX_RASTER_PIXELS {
        return Err(too_large());
    }
    // Drawn as the page draws it, at its shown size: points scaled to these pixels exactly.
    let rect = Rect::new(0.0, 0.0, w, h);
    let item = Draw::Image { rect, media, crop, alpha: fx.opacity(), alt: String::new(), fx, decorative: false };
    let drawn = wordcraft_render::render_items(&s.doc, &[item], rect, w_px / w.max(f32::MIN_POSITIVE), &Default::default());
    let rgba = image::RgbaImage::from_raw(drawn.width, drawn.height, drawn.to_straight())
        .ok_or_else(|| CmdError::Failed("the drawn picture has the wrong size".into()))?;
    let img = match format {
        // No transparency in these: what is see-through is shown on white, as on the page.
        image::ImageFormat::Jpeg | image::ImageFormat::Bmp => image::DynamicImage::ImageRgb8(on_white(&rgba)),
        _ => image::DynamicImage::ImageRgba8(rgba),
    };
    let out = wordcraft_geom::picture::encode_format_at(&img, format, dpi).map_err(|e| CmdError::Failed(format!("picture encode: {e}")))?;
    let dpi = (f64::from(dpi) * 100.0).round() / 100.0;
    let info = json!({"width": drawn.width, "height": drawn.height, "bytes": out.len(), "drawn": true, "dpi": dpi});
    Ok((out, info))
}

/// The resolution `v` asks for: `dpi` from [`MIN_DPI`] to [`MAX_DPI`], or `None` for the picture's
/// own (no `dpi`, or `"original"`).
fn dpi(v: &Value) -> Result<Option<f32>, CmdError> {
    match v.get("dpi") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s == "original" => Ok(None),
        Some(d) => {
            let dpi = d.as_f64().filter(|d| d.is_finite()).ok_or_else(|| CmdError::Params("`dpi` must be a number or \"original\"".into()))?;
            if !(f64::from(MIN_DPI)..=f64::from(MAX_DPI)).contains(&dpi) {
                return Err(CmdError::Params(format!("`dpi` must be from {MIN_DPI} to {MAX_DPI}, or \"original\"")));
            }
            Ok(Some(dpi as f32))
        }
    }
}

/// `rgba` over white.
fn on_white(rgba: &image::RgbaImage) -> image::RgbImage {
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over = |c: u8| ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a)) + 127) / 255) as u8;
        image::Rgb([over(r), over(g), over(b)])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_names_the_type() {
        assert_eq!(format_of("a/b.PNG"), Some(image::ImageFormat::Png));
        assert_eq!(format_of("photo.jpeg"), Some(image::ImageFormat::Jpeg));
        assert_eq!(format_of("scan.tif"), Some(image::ImageFormat::Tiff));
        assert_eq!(format_of("notes.docx"), None);
        assert_eq!(format_of("png"), None, "a name with no extension");
    }

    #[test]
    fn dpi_is_a_number_or_the_original() {
        assert_eq!(dpi(&json!({})).unwrap(), None);
        assert_eq!(dpi(&json!({"dpi": "original"})).unwrap(), None);
        assert_eq!(dpi(&json!({"dpi": 600})).unwrap(), Some(600.0));
        for bad in [json!({"dpi": 1}), json!({"dpi": 5000}), json!({"dpi": "high"}), json!({"dpi": true})] {
            assert!(matches!(dpi(&bad), Err(CmdError::Params(_))), "{bad}");
        }
    }

    #[test]
    fn transparency_is_shown_on_white() {
        let img = image::RgbaImage::from_raw(2, 1, vec![0, 0, 0, 0, 0, 0, 0, 255]).unwrap();
        let rgb = on_white(&img);
        assert_eq!(rgb.get_pixel(0, 0).0, [255, 255, 255]);
        assert_eq!(rgb.get_pixel(1, 0).0, [0, 0, 0]);
    }
}
