use super::exporter::Exporter;
use super::metadata::metadata;
use super::options::{PdfError, PdfOptions};
use super::paint::{clamp_side, ok, rect_path};
use super::{
    DocLayout, Document, FillRule, HashMap, HashSet, KrillaError, LayoutCache, LayoutOptions, Page, PageSettings, Rect, Size, Transform, WriteError,
    Writer, imposition, krilla, layout, write,
};

/// Lay out `doc` and write it as PDF. Without markup the layout is the final text: tracked
/// deletions are left out, as in Word's "No Markup" view. With `comments` each page carries the
/// markup area with the comment balloons.
pub fn export(doc: &Document, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    write::guard(|| {
        let lo = LayoutOptions { hide_deleted: !opts.include_markup, comment_balloons: opts.comments, numeral: opts.numeral, ..Default::default() };
        let lay = layout(doc, &mut LayoutCache::new(), &lo);
        export_layout_inner(doc, &lay, opts)
    })
}

/// Write an existing layout of `doc` as PDF.
pub fn export_layout(doc: &Document, lay: &DocLayout, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    write::guard(|| export_layout_inner(doc, lay, opts))
}

/// Validate selected pages and retry embedding failures with outlined fonts.
fn export_layout_inner(doc: &Document, lay: &DocLayout, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    let mut adjusted = opts.clone();
    adjusted.tagged &= adjusted.sheet_copies <= 1;
    let opts = &adjusted;
    let pages: Vec<usize> = match &opts.pages {
        Some(v) => {
            if let Some(bad) = v.iter().find(|p| **p >= lay.pages.len()) {
                return Err(PdfError::BadPage(bad.saturating_add(1)));
            }
            v.clone()
        }
        None => (0..lay.pages.len()).collect(),
    };
    if pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    // krilla subsets fonts only when the document is finished, so one font it can't subset fails
    // the whole export. Draw that font's glyphs as outlines instead and write the document again.
    let mut outlined = HashSet::new();
    loop {
        match write_attempt(doc, lay, opts, &pages, &outlined)? {
            Attempt::Done(bytes) => return Ok(bytes),
            Attempt::BadFont(id, msg) if outlined.len() < MAX_OUTLINED_FONTS && outlined.insert(id) => {
                log::warn!("PDF: {msg}; its text is drawn as outlines");
            }
            Attempt::BadFont(_, msg) => return Err(PdfError::Write(msg)),
        }
    }
}

/// How many unsubsettable fonts we fall back to outlines for before giving up.
const MAX_OUTLINED_FONTS: usize = 32;

/// One try at writing the PDF.
enum Attempt {
    Done(Vec<u8>),
    /// The face with this id could not be embedded (and why).
    BadFont(u32, String),
}

/// Serialize physical sheet sides while mapping link and outline destinations onto their cells.
fn write_attempt(doc: &Document, lay: &DocLayout, opts: &PdfOptions, pages: &[usize], outlined: &HashSet<u32>) -> Result<Attempt, PdfError> {
    let one_copy = opts.imposition.sheets(lay, pages, opts.paper)?;
    let copies = opts.sheet_copies.clamp(1, 999) as usize;
    if (opts.imposition != imposition::Imposition::Single || copies > 1) && one_copy.len().checked_mul(copies).is_none_or(|n| n > 5000) {
        return Err(PdfError::Write("the print job exceeds 5000 sheet sides".into()));
    }
    let sheets: Vec<_> = if opts.sheet_collate {
        (0..copies).flat_map(|_| one_copy.iter()).collect()
    } else if matches!(opts.imposition, imposition::Imposition::Booklet { .. }) {
        // A booklet's front/back pair must remain one physical sheet even without collation.
        one_copy.chunks(2).flat_map(|pair| (0..copies).flat_map(move |_| pair.iter())).collect()
    } else {
        one_copy.iter().flat_map(|s| std::iter::repeat_n(s, copies)).collect()
    };
    let mut destinations = HashMap::new();
    for (output, sheet) in sheets.iter().enumerate() {
        for slot in &sheet.slots {
            let page = lay.pages.get(slot.page).ok_or(PdfError::BadPage(slot.page.saturating_add(1)))?;
            destinations.entry(slot.page).or_insert((output, Fit::in_slot(page, *slot)));
        }
    }
    let settings = krilla::SerializeSettings { compress_content_streams: opts.compress, enable_tagging: opts.tagged, ..Default::default() };
    let language = doc.styles.default_chr.lang.clone().filter(|l| !l.is_empty() && l.is_ascii());
    let mut writer = Writer::new_with_language(settings, metadata(doc, opts), language);
    let pdf = writer.document_mut();

    let mut ex = Exporter {
        doc,
        lay,
        opts,
        out_index: destinations,
        fonts: HashMap::new(),
        font_error: None,
        outlined,
        cmaps: HashMap::new(),
        images: HashMap::new(),
        vectors: HashMap::new(),
        tags: Vec::new(),
        tag_index: HashMap::new(),
        links: Vec::new(),
        bookmarks: std::cell::OnceCell::new(),
    };
    if let Some(o) = ex.outline(pages) {
        pdf.set_outline(o);
    }
    for sheet in sheets {
        let size = Size::from_wh(clamp_side(sheet.width), clamp_side(sheet.height)).ok_or(PdfError::NoPages)?;
        let mut kp = pdf.start_page_with(PageSettings::new(size));
        let mut annotations = Vec::new();
        let mut surface = kp.surface();
        for slot in &sheet.slots {
            let page = lay.pages.get(slot.page).ok_or(PdfError::BadPage(slot.page.saturating_add(1)))?;
            let fit = Fit::in_slot(page, *slot);
            ex.links.clear();
            let clip = rect_path(&Rect::new(slot.x, slot.y, slot.width, slot.height));
            if let Some(clip) = &clip {
                surface.push_clip_path(clip, &FillRule::NonZero);
            }
            surface.push_transform(&Transform::from_row(fit.k, 0.0, 0.0, fit.k, fit.dx, fit.dy));
            ex.page(&mut surface, page, clamp_side(fit.page_w), clamp_side(page.h));
            surface.pop();
            if clip.is_some() {
                surface.pop();
            }
            if opts.links {
                annotations.extend(ex.annotations(fit));
            }
        }
        surface.finish();
        for annotation in annotations {
            kp.add_annotation(annotation);
        }
        kp.finish();
    }
    if opts.tagged {
        pdf.set_tag_tree(ex.tag_tree());
    }
    if let Some(error) = ex.font_error {
        return Err(error);
    }
    match writer.finish() {
        Ok(bytes) => Ok(Attempt::Done(bytes)),
        Err(WriteError::Serialization(error)) => match *error {
            KrillaError::Font(font, msg) => {
                let bad = ex.fonts.iter().find(|(_, (_, f))| f.as_ref() == Some(&font));
                match bad {
                    Some((id, (face, _))) => Ok(Attempt::BadFont(*id, format!("font {} {} could not be embedded ({msg})", face.family, face.style))),
                    None => Err(PdfError::Write(format!("a font could not be embedded ({msg})"))),
                }
            }
            error => Err(PdfError::Write(format!("{error:?}"))),
        },
        Err(error) => Err(error.into()),
    }
}

/// How a layout page maps onto its PDF page: a page with a markup area (Print Markup) is scaled
/// by `k` to its paper width and centred vertically (`dy`). With [`PdfOptions::paper`] that
/// sheet is then scaled to the chosen paper and centred on it (`dx`, `dy`). Any other page is
/// drawn as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fit {
    /// The PDF page.
    paper_w: f32,
    paper_h: f32,
    /// The document page's width in layout points (without the markup area).
    page_w: f32,
    k: f32,
    dx: f32,
    dy: f32,
}

impl Fit {
    /// The physical output page dimensions, shared with the imposition planner.
    pub(crate) fn paper_size(self) -> (f32, f32) {
        (self.paper_w, self.paper_h)
    }

    /// Fit the complete logical page (including markup) within one fixed sheet cell.
    fn in_slot(page: &Page, slot: imposition::Slot) -> Self {
        let own = Self::of(page, None);
        let k = (slot.width / page.w).min(slot.height / page.h);
        Self {
            paper_w: slot.width,
            paper_h: slot.height,
            page_w: own.page_w,
            k,
            dx: slot.x + (slot.width - page.w * k) / 2.0,
            dy: slot.y + (slot.height - page.h * k) / 2.0,
        }
    }

    /// Map one document page and its markup area onto its requested paper.
    pub(crate) fn of(page: &Page, paper: Option<(f32, f32)>) -> Fit {
        let paper_w = page.w - page.markup;
        let own = if page.markup > 0.0 && ok(page.markup) && ok(page.w) && paper_w > 1.0 {
            let k = paper_w / page.w;
            Fit { paper_w, paper_h: page.h, page_w: paper_w, k, dx: 0.0, dy: page.h * (1.0 - k) / 2.0 }
        } else {
            Fit { paper_w: page.w, paper_h: page.h, page_w: page.w, k: 1.0, dx: 0.0, dy: 0.0 }
        };
        let Some((w, h)) = paper else { return own };
        let f = wordcraft_geom::fit_to_paper(own.paper_w, own.paper_h, w, h);
        Fit { paper_w: f.paper_w, paper_h: f.paper_h, page_w: own.page_w, k: own.k * f.k, dx: f.dx, dy: own.dy * f.k + f.dy }
    }

    /// A layout point on the PDF page.
    pub(crate) fn point(self, x: f32, y: f32) -> (f32, f32) {
        (x * self.k + self.dx, y * self.k + self.dy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N-up fits the whole markup page once, with the same centering as the physical-sheet preview.
    #[test]
    fn imposed_markup_pages_fit_the_slot_without_double_scaling() {
        let page = wordcraft_layout::Page { w: 732.0, h: 792.0, markup: 120.0, ..Default::default() };
        for (width, height, scale) in [(1000.0, 396.0, 0.5), (183.0, 1000.0, 0.25)] {
            let slot = imposition::Slot { page: 0, x: 17.0, y: 23.0, width, height };
            let fit = Fit::in_slot(&page, slot);
            assert!((fit.k - scale).abs() < 0.00001);
            assert!((fit.dx - (17.0 + (width - 732.0 * scale) / 2.0)).abs() < 0.00001);
            assert!((fit.dy - (23.0 + (height - 792.0 * scale) / 2.0)).abs() < 0.00001);
            assert_eq!(fit.page_w, 612.0, "the unscaled document width excludes the balloon area");
            assert_eq!((fit.paper_w, fit.paper_h), (width, height));
        }
    }
}
