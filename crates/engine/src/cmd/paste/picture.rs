//! Paste Special › Picture of WordCraft's own copy (#7743): copied text and tables drawn as a
//! picture, as Word pastes a copy of its own text as a picture.
//!
//! The copy is laid out on a scratch page of this document: its styles, numbering and theme, as
//! wide as the text of the section the caret is in, with no margins, header or footer, page colour
//! or watermark. It is drawn on white paper at the resolution asked for and cropped to what was
//! drawn, so the picture is the copy and nothing around it. The picture is inserted at its real
//! size (its pixels at that resolution), which the PNG records too.

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Block, Document, Paragraph};
use wordcraft_geom::Rect;
use wordcraft_render::Rendered;

use crate::{CmdError, Session};

/// The resolutions Paste Special offers for a picture of the copy, DPI.
pub const DPI_CHOICES: [u32; 6] = [96, 150, 300, 600, 1200, 1500];
/// The resolution of a picture of the copy when none is given, DPI: print quality.
pub const DEFAULT_DPI: u32 = 300;
/// Lowest resolution a picture of the copy may have, DPI.
pub const MIN_DPI: u32 = 36;
/// Highest resolution a picture of the copy may have, DPI. A picture past
/// [`wordcraft_geom::MAX_RASTER_PIXELS`] or [`wordcraft_render::MAX_SIDE`] at it is refused.
pub const MAX_DPI: u32 = 2400;
/// Height of the scratch page the copy is laid out on, points (200 inches): a copy that needs
/// more is refused rather than cut.
const SCRATCH_PAGE_H: f32 = 14_400.0;
/// Pixels per point of the pass that finds where the copy is drawn.
const PROBE_SCALE: f32 = 0.5;
/// Points added around what that coarse pass found, so the full-resolution pass misses no edge of
/// a glyph or border it saw only faintly; the full pass is cropped exactly.
const PROBE_SLACK: f32 = 4.0;
/// Points per inch.
const POINTS_PER_INCH: f32 = 72.0;

/// The resolution `v` asks for (`dpi`, refused outside [`MIN_DPI`]..=[`MAX_DPI`]), else
/// [`DEFAULT_DPI`].
pub(super) fn dpi(v: &Value) -> Result<f32, CmdError> {
    let Some(d) = v.get("dpi").filter(|d| !d.is_null()) else { return Ok(DEFAULT_DPI as f32) };
    let dpi = d.as_f64().filter(|d| d.is_finite()).ok_or_else(|| CmdError::Params("`dpi` must be a number".into()))?;
    if !(f64::from(MIN_DPI)..=f64::from(MAX_DPI)).contains(&dpi) {
        return Err(CmdError::Params(format!("`dpi` must be from {MIN_DPI} to {MAX_DPI}")));
    }
    Ok(dpi as f32)
}

/// A picture of the copy: PNG bytes, and its width in points (its height follows from its pixels).
pub(super) struct CopyPicture {
    pub png: Vec<u8>,
    pub width: f32,
}

/// `frag`, copied from `s`, drawn as a picture at `dpi` (see the module documentation).
pub(super) fn draw(s: &Session, frag: &Fragment, dpi: f32) -> Result<CopyPicture, CmdError> {
    let doc = scratch_document(s, frag);
    let lo = wordcraft_layout::LayoutOptions { view: wordcraft_layout::ViewMode::Print, ..Default::default() };
    let layout = wordcraft_layout::layout(&doc, &mut wordcraft_layout::LayoutCache::new(), &lo);
    let page = match layout.pages.as_slice() {
        [page] => page,
        [] => return Err(nothing_to_draw()),
        _ => return Err(CmdError::Failed("the copy is too long to paste as a picture; copy less of it".into())),
    };
    // Drawn as a page image draws (no proofing marks, tracked changes marked as Track Changes
    // Options mark them on screen), on white paper.
    let opts = crate::io_ext::page_image_options(&crate::cmd::file::export_options(s, &json!({})));
    let paper = opts.paper_of(&doc);
    // Where the copy is drawn, found coarsely over the whole page…
    let whole = Rect::new(0.0, 0.0, page.w, page.h);
    let probe = wordcraft_render::render_area(&doc, page, whole, PROBE_SCALE, &opts);
    let (x0, y0, x1, y1) = ink_bounds(&probe, paper).ok_or_else(nothing_to_draw)?;
    let px_to_pt = |px: u32| px as f32 / PROBE_SCALE;
    let left = (px_to_pt(x0) - PROBE_SLACK).max(0.0);
    let top = (px_to_pt(y0) - PROBE_SLACK).max(0.0);
    let area = Rect::new(left, top, (px_to_pt(x1 + 1) + PROBE_SLACK).min(page.w) - left, (px_to_pt(y1 + 1) + PROBE_SLACK).min(page.h) - top);
    // …then drawn at the resolution asked for, within the limits a raster has.
    let scale = dpi / POINTS_PER_INCH;
    let (w_px, h_px) = ((area.w * scale).ceil(), (area.h * scale).ceil());
    let too_large = || CmdError::Failed(format!("the copy is too large for a picture at {dpi:.0} DPI; choose a lower resolution"));
    if w_px > wordcraft_render::MAX_SIDE as f32 || h_px > wordcraft_render::MAX_SIDE as f32 {
        return Err(too_large());
    }
    if (w_px as u64).saturating_mul(h_px as u64) > wordcraft_geom::MAX_RASTER_PIXELS {
        return Err(too_large());
    }
    let full = wordcraft_render::render_area(&doc, page, area, scale, &opts);
    if (full.width as f32, full.height as f32) != (w_px, h_px) {
        return Err(too_large());
    }
    let (cx0, cy0, cx1, cy1) = ink_bounds(&full, paper).ok_or_else(nothing_to_draw)?;
    let cropped = crop(&full, cx0, cy0, cx1, cy1)?;
    let w = cropped.width();
    let png = wordcraft_geom::picture::encode_format_at(&image::DynamicImage::ImageRgba8(cropped), image::ImageFormat::Png, dpi)
        .map_err(|e| CmdError::Failed(format!("PNG encode: {e}")))?;
    Ok(CopyPicture { png, width: w as f32 / scale })
}

fn nothing_to_draw() -> CmdError {
    CmdError::Failed("the copy has nothing to draw as a picture".into())
}

/// A copy of `s`'s document whose body is `frag`, on a page as wide as the text of the caret's
/// section and [`SCRATCH_PAGE_H`] tall, with no margins, header, footer, columns, borders, line
/// numbers, page colour, page picture or watermark.
fn scratch_document(s: &Session, frag: &Fragment) -> Document {
    let mut doc = s.doc.clone();
    let mut sect = crate::cmd::page::sect(s);
    let width = sect.text_width();
    let mut body: Vec<Arc<Block>> = frag
        .blocks
        .iter()
        .map(|b| {
            let mut b = b.clone();
            // A copied section break would start another page; the copy is one picture.
            if let Block::Para(p) = &mut b {
                p.section = None;
            }
            Arc::new(b)
        })
        .collect();
    if body.is_empty() {
        body.push(Arc::new(Block::Para(Paragraph::new())));
    }
    doc.body = body;
    // The copy's text box stories and pictures, over any of this document's with the same keys:
    // they may come from another document.
    doc.parts.extend(frag.parts.clone());
    doc.media.extend(frag.media.clone());
    doc.settings.page_color = None;
    doc.settings.page_picture = None;
    doc.settings.watermark = None;
    sect.page_w = width;
    sect.page_h = SCRATCH_PAGE_H;
    (sect.margin_top, sect.margin_bottom, sect.margin_left, sect.margin_right) = (0.0, 0.0, 0.0, 0.0);
    (sect.header, sect.footer, sect.header_gap, sect.footer_gap, sect.gutter) = (0.0, 0.0, 0.0, 0.0, 0.0);
    sect.columns = Default::default();
    sect.headers = Default::default();
    sect.footers = Default::default();
    sect.title_page = false;
    sect.line_numbers = None;
    sect.page_borders = None;
    sect.valign = Default::default();
    sect.doc_grid = None;
    doc.last_section = sect;
    doc
}

/// The first and last column and row (inclusive) of `img` drawn in anything but `paper`, if any.
fn ink_bounds(img: &Rendered, paper: wordcraft_doc::props::Rgb) -> Option<(u32, u32, u32, u32)> {
    let w = img.width as usize;
    if w == 0 {
        return None;
    }
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (y, row) in img.pixels.chunks_exact(w * 4).enumerate() {
        for (x, px) in row.as_chunks::<4>().0.iter().enumerate() {
            // The paper is opaque, so every pixel is: its colour is its premultiplied colour.
            if px[..3] != [paper.0, paper.1, paper.2] || px[3] != 255 {
                let (x, y) = (x as u32, y as u32);
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    bounds
}

/// Columns `x0..=x1` and rows `y0..=y1` of `img` as straight RGBA.
fn crop(img: &Rendered, x0: u32, y0: u32, x1: u32, y1: u32) -> Result<image::RgbaImage, CmdError> {
    let straight = img.to_straight();
    let (w, cw, ch) = (img.width as usize, (x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let mut out = Vec::with_capacity(cw * ch * 4);
    for y in y0 as usize..=y1 as usize {
        let at = (y * w + x0 as usize) * 4;
        out.extend_from_slice(straight.get(at..at + cw * 4).ok_or_else(|| CmdError::Failed("picture crop out of bounds".into()))?);
    }
    image::RgbaImage::from_raw(cw as u32, ch as u32, out).ok_or_else(|| CmdError::Failed("picture crop has the wrong size".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_is_checked() {
        assert_eq!(dpi(&json!({})).unwrap(), 300.0);
        assert_eq!(dpi(&json!({"dpi": 1500})).unwrap(), 1500.0);
        assert_eq!(dpi(&json!({"dpi": null})).unwrap(), 300.0);
        for bad in [json!({"dpi": 0}), json!({"dpi": 3000}), json!({"dpi": "300"})] {
            assert!(matches!(dpi(&bad), Err(CmdError::Params(_))), "{bad}");
        }
    }

    #[test]
    fn ink_bounds_finds_what_differs_from_the_paper() {
        let white = wordcraft_doc::props::Rgb(255, 255, 255);
        let mut img = Rendered { width: 4, height: 3, pixels: [255u8; 48].to_vec() };
        assert_eq!(ink_bounds(&img, white), None);
        img.pixels[24] = 0; // red of (2, 1)
        img.pixels[37] = 10; // green of (1, 2)
        assert_eq!(ink_bounds(&img, white), Some((1, 1, 2, 2)));
        let c = crop(&img, 1, 1, 2, 2).unwrap();
        assert_eq!((c.width(), c.height()), (2, 2));
        assert_eq!(c.get_pixel(1, 0).0, [0, 255, 255, 255]);
    }
}
