//! Page images (`file.exportPng`, #7743): which pages, at what resolution, under which names.
//!
//! A page image is written at a resolution in dots per inch (`dpi`) or, as before, a `scale` in
//! pixels per point. Several pages go to several files named after the one chosen: `notes.png`
//! with pages 1–3 is `notes-1.png`, `notes-2.png`, `notes-3.png` (zero-padded so they sort, past
//! page 9). The pages are chosen the way File › Print chooses them ([`crate::print::plan`]).

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_layout::DocLayout;

use crate::{CmdError, Session, p};

/// The parameters that choose what a page image is: the pages, the resolution, Print Markup and
/// strictness. A request for the save dialog carries them over to the export after it, and the
/// app asks for them in its own dialog only when none is given.
pub const CHOICES: [&str; 8] = ["dpi", "scale", "page", "range", "pages", "parity", "markup", "strict"];

/// Whether `v` makes any of the [`CHOICES`] (a `null` is no choice).
pub fn has_choices(v: &Value) -> bool {
    CHOICES.iter().any(|k| v.get(*k).is_some_and(|x| !x.is_null()))
}
/// The resolutions File › Export › Page images offers, DPI.
pub const DPI_CHOICES: [u32; 6] = [96, 150, 300, 600, 1200, 1500];
/// The resolution a page image has when none is given, DPI (2 pixels a point).
pub const DEFAULT_DPI: u32 = 144;
/// Lowest resolution a page image may be written at, DPI.
pub const MIN_DPI: u32 = 36;
/// Highest resolution a page image may be written at, DPI. A page past
/// [`wordcraft_render::MAX_PAGE_IMAGE_PIXELS`] at it is refused, so large paper gets less.
pub const MAX_DPI: u32 = 2400;
/// Lowest and highest `scale` (pixels per point) the older parameter takes; it is clamped, as it
/// always was.
const SCALE_RANGE: (f32, f32) = (0.1, 8.0);
/// Points per inch.
const POINTS_PER_INCH: f32 = 72.0;

/// The pixels per point `v` asks for: `dpi` (refused outside [`MIN_DPI`]..=[`MAX_DPI`]), else
/// `scale` (clamped), else [`DEFAULT_DPI`]. Both at once is a parameter error.
pub fn scale(v: &Value) -> Result<f32, CmdError> {
    let dpi = v.get("dpi").filter(|d| !d.is_null());
    match (dpi, v.get("scale").filter(|s| !s.is_null())) {
        (Some(_), Some(_)) => Err(CmdError::Params("give `dpi` or `scale`, not both".into())),
        (Some(d), None) => {
            let dpi = d.as_f64().filter(|d| d.is_finite()).ok_or_else(|| CmdError::Params("`dpi` must be a number".into()))?;
            if !(f64::from(MIN_DPI)..=f64::from(MAX_DPI)).contains(&dpi) {
                return Err(CmdError::Params(format!("`dpi` must be from {MIN_DPI} to {MAX_DPI}")));
            }
            Ok(dpi as f32 / POINTS_PER_INCH)
        }
        (None, Some(_)) => Ok(p::f32(v, "scale").unwrap_or(DEFAULT_DPI as f32 / POINTS_PER_INCH).clamp(SCALE_RANGE.0, SCALE_RANGE.1)),
        (None, None) => Ok(DEFAULT_DPI as f32 / POINTS_PER_INCH),
    }
}

/// The 0-based pages `v` chooses from layout `l`, whose caret and selection sit on pages
/// `current`: one `page` (1-based), or a print-style `range` (`all`, `current`, `selection`,
/// `custom`) with `pages` (`1-3, 5`) and `parity`; with neither, the first page, as before.
pub fn chosen(v: &Value, l: &DocLayout, current: (usize, usize)) -> Result<Vec<usize>, CmdError> {
    let ranged = ["range", "pages", "parity"].iter().any(|k| v.get(*k).is_some_and(|x| !x.is_null()));
    if let Some(n) = p::u64(v, "page") {
        if ranged {
            return Err(CmdError::Params("give `page` or `range`/`pages`/`parity`, not both".into()));
        }
        let page = crate::p::page_index(n);
        return if page < l.pages.len() { Ok(vec![page]) } else { Err(CmdError::Params(format!("no page {}", page.saturating_add(1)))) };
    }
    if !ranged {
        return if l.pages.is_empty() { Err(CmdError::Params("no page 1".into())) } else { Ok(vec![0]) };
    }
    if p::str(v, "range") == Some("custom") && p::str(v, "pages").is_none_or(|t| t.trim().is_empty()) {
        return Err(CmdError::Params("type the pages to export, like 1-3, 5".into()));
    }
    // One image a page: a page range has no copies.
    let mut single = v.clone();
    single["copies"] = json!(1);
    single["collate"] = json!(true);
    Ok(crate::print::plan(&single, l.pages.len(), current)?.pages)
}

/// Page images checked and ready to draw: every page `v` chooses, each within the size a page
/// image may have and, with `strict`, with nothing missing. Made before anything is written, so a
/// refusal writes nothing.
pub struct PageImages {
    layout: Arc<DocLayout>,
    /// The pages, 0-based, in document order.
    pub pages: Vec<usize>,
    scale: f32,
    opts: wordcraft_render::RenderOptions,
}

impl PageImages {
    /// The page images `v` asks of `s` (see [`scale`] and [`chosen`]; `markup` and `strict` as
    /// `file.exportPng` takes them).
    pub fn plan(s: &Session, v: &Value) -> Result<Self, CmdError> {
        let scale = scale(v)?;
        let export = crate::cmd::file::export_options(s, v);
        let layout = s.export_layout_with(export.markup);
        let pages = chosen(v, &layout, s.current_pages(&layout))?;
        // A page image is the printed page: headers and footers at full strength. Dimming them is
        // the on-screen cue while editing the body, not part of the page. Tracked changes are
        // marked as Track Changes Options mark them on screen.
        let opts = crate::io_ext::page_image_options(&export);
        for &page in &pages {
            let number = page.saturating_add(1);
            let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {number}")))?;
            wordcraft_render::page_image_size(pg, scale).map_err(|e| CmdError::Params(format!("page {number}: {e}")))?;
            if export.strict {
                let report = crate::strict::strict_report(&s.doc, &layout, &opts.display, Some(page..=page), wordcraft_layout::StrictOutput::Png);
                if !report.passed() {
                    return Err(CmdError::Failed(report.refusal()));
                }
            }
        }
        Ok(PageImages { layout, pages, scale, opts })
    }

    /// The resolution, DPI (to two decimals: a `scale` need not be a whole number of DPI).
    pub fn dpi(&self) -> f64 {
        (f64::from(self.scale) * f64::from(POINTS_PER_INCH) * 100.0).round() / 100.0
    }

    /// Page `page` (0-based, one of [`PageImages::pages`]) of `doc` as PNG bytes, with its size in
    /// pixels. One page at a time, so only one is held at once.
    pub fn png(&self, doc: &wordcraft_doc::Document, page: usize) -> Result<(Vec<u8>, u32, u32), CmdError> {
        let pg = self.layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {}", page.saturating_add(1))))?;
        let (w, h) = wordcraft_render::page_image_size(pg, self.scale).map_err(CmdError::Failed)?;
        let png = wordcraft_render::page_png(doc, pg, self.scale, &self.opts).map_err(CmdError::Failed)?;
        Ok((png, w, h))
    }
}

/// The one page image `v` asks of `s`, as PNG bytes, for a host that saves one download at a time
/// (the browser) rather than files beside each other: more than one page is a parameter error.
pub fn single_png(s: &Session, v: &Value) -> Result<Vec<u8>, CmdError> {
    let images = PageImages::plan(s, v)?;
    let [page] = images.pages[..] else {
        return Err(CmdError::Params(format!(
            "the browser saves one page image at a time and {} pages are chosen; choose one page",
            images.pages.len()
        )));
    };
    Ok(images.png(&s.doc, page)?.0)
}

/// The file each of `pages` (0-based) is written to: `path` itself for one page, else `path` with
/// `-N` before `.png`, N the page number padded to the width of the largest one.
pub fn paths(path: &str, pages: &[usize]) -> Vec<String> {
    if let [_] = pages {
        return vec![path.to_string()];
    }
    let p = Path::new(path);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let width = pages.iter().max().map_or(1, |last| last.saturating_add(1).to_string().len());
    pages
        .iter()
        .map(|page| {
            let name = format!("{stem}-{:0width$}.png", page.saturating_add(1));
            p.with_file_name(name).to_string_lossy().into_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_and_scale_give_pixels_per_point() {
        assert_eq!(scale(&json!({})).unwrap(), 2.0, "144 DPI by default, as before");
        assert_eq!(scale(&json!({"dpi": 300})).unwrap(), 300.0 / 72.0);
        assert_eq!(scale(&json!({"dpi": 1500})).unwrap(), 1500.0 / 72.0);
        assert_eq!(scale(&json!({"scale": 1.0})).unwrap(), 1.0);
        assert_eq!(scale(&json!({"scale": 100.0})).unwrap(), 8.0, "`scale` is clamped, as it always was");
        assert_eq!(scale(&json!({"dpi": null, "scale": 3.0})).unwrap(), 3.0, "null is absent");
        for bad in [json!({"dpi": 35}), json!({"dpi": 2401}), json!({"dpi": "300"}), json!({"dpi": -1}), json!({"dpi": 300, "scale": 2})] {
            assert!(matches!(scale(&bad), Err(CmdError::Params(_))), "{bad}");
        }
    }

    #[test]
    fn several_pages_get_numbered_names_that_sort() {
        assert_eq!(paths("out/table.png", &[2]), ["out/table.png"]);
        let named = paths("out/table.png", &[0, 1, 2]);
        assert_eq!(named.len(), 3);
        assert!(named[0].ends_with("table-1.png") && named[2].ends_with("table-3.png"), "{named:?}");
        let padded = paths("t.png", &[0, 9, 11]);
        assert!(padded[0].ends_with("t-01.png") && padded[1].ends_with("t-10.png") && padded[2].ends_with("t-12.png"), "{padded:?}");
        assert_eq!(paths("Ünïcode.png", &[0, 1])[1], "Ünïcode-2.png");
    }
}
