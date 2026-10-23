//! Physical sheet planning shared by PDF printing and the print preview.
use crate::PdfError;
use wordcraft_layout::DocLayout;

/// How logical pages are placed on each physical side of a sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Imposition {
    /// Keep one logical page per PDF page.
    #[default]
    Single,
    /// Fill a fixed grid in row order; supported counts are 2, 4, 6, 9 and 16.
    NUp(u8),
    /// Pad to a multiple of four and pair outer/inner pages for duplex folding.
    Booklet { reverse: bool },
}

/// One logical page's location on a physical sheet side, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// Zero-based logical layout page.
    pub page: usize,
    /// Cell left edge.
    pub x: f32,
    /// Cell top edge.
    pub y: f32,
    /// Available width in points.
    pub width: f32,
    /// Available height in points.
    pub height: f32,
}

/// One PDF page representing a physical sheet side; empty slots stay white.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    /// Available width in points.
    pub width: f32,
    /// Available height in points.
    pub height: f32,
    /// Occupied cells only, in drawing order.
    pub slots: Vec<Slot>,
}

impl Imposition {
    /// Plan one copy, keeping selected page order and padding only after its last page.
    /// Paper defaults to the first selected page. Two-up and booklets use landscape sheets.
    pub fn sheets(self, lay: &DocLayout, pages: &[usize], paper: Option<(f32, f32)>) -> Result<Vec<Sheet>, PdfError> {
        if pages.is_empty() {
            return Err(PdfError::NoPages);
        }
        if self != Self::Single && pages.len() > 5000 {
            return Err(PdfError::Write("a print job may contain at most 5000 logical pages".into()));
        }
        for &index in pages {
            if lay.pages.get(index).is_none() {
                return Err(PdfError::BadPage(index.saturating_add(1)));
            }
        }
        if self == Self::Single {
            return pages
                .iter()
                .map(|&page| {
                    let pg = lay.pages.get(page).ok_or(PdfError::BadPage(page.saturating_add(1)))?;
                    let fit = super::Fit::of(pg, paper);
                    Ok(Sheet {
                        width: fit.paper_w,
                        height: fit.paper_h,
                        slots: vec![Slot { page, x: 0.0, y: 0.0, width: fit.paper_w, height: fit.paper_h }],
                    })
                })
                .collect();
        }
        let first = lay.pages.get(*pages.first().ok_or(PdfError::NoPages)?).ok_or(PdfError::NoPages)?;
        let (mut width, mut height) = paper.unwrap_or((first.w - first.markup, first.h));
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 || width > 14400.0 || height > 14400.0 {
            return Err(PdfError::Write("invalid print sheet dimensions".into()));
        }
        let (cols, rows) = match self {
            Self::Single => (1, 1),
            Self::NUp(2) | Self::Booklet { .. } => {
                if width < height {
                    std::mem::swap(&mut width, &mut height);
                }
                (2, 1)
            }
            Self::NUp(4) => (2, 2),
            Self::NUp(6) => (2, 3),
            Self::NUp(9) => (3, 3),
            Self::NUp(16) => (4, 4),
            Self::NUp(_) => return Err(PdfError::Write("pages per sheet must be 1, 2, 4, 6, 9 or 16".into())),
        };
        let ordered: Vec<Option<usize>> = match self {
            Self::Booklet { reverse } => {
                let padded = pages.len().div_ceil(4) * 4;
                let mut out = Vec::with_capacity(padded);
                for side in 0..padded / 2 {
                    let pair = if side % 2 == 0 { [padded - 1 - side, side] } else { [side, padded - 1 - side] };
                    for index in if reverse { [pair[1], pair[0]] } else { pair } {
                        out.push(pages.get(index).copied());
                    }
                }
                out
            }
            _ => pages.iter().copied().map(Some).collect(),
        };
        Ok(ordered
            .chunks(cols * rows)
            .map(|chunk| {
                let slots = chunk
                    .iter()
                    .enumerate()
                    .filter_map(|(i, page)| {
                        page.map(|page| Slot {
                            page,
                            x: (i % cols) as f32 * width / cols as f32,
                            y: (i / cols) as f32 * height / rows as f32,
                            width: width / cols as f32,
                            height: height / rows as f32,
                        })
                    })
                    .collect();
                Sheet { width, height, slots }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Folding eight pages yields 8/1, 2/7, 6/3, 4/5, with mirrored pairs for right binding.
    #[test]
    fn booklet_pairs_padding_ranges_and_reverse() {
        let doc = wordcraft_doc::Document::from_text("1\u{c}2\u{c}3\u{c}4\u{c}5\u{c}6\u{c}7\u{c}8");
        let lay = wordcraft_layout::layout(&doc, &mut Default::default(), &Default::default());
        assert_eq!(lay.pages.len(), 8);
        for (pages, expected) in [
            (vec![0, 1, 2, 3, 4, 5, 6, 7], vec![vec![7, 0], vec![1, 6], vec![5, 2], vec![3, 4]]),
            (vec![1, 3, 5, 7, 0], vec![vec![1], vec![3], vec![5], vec![7, 0]]),
        ] {
            let sheets = Imposition::Booklet { reverse: false }.sheets(&lay, &pages, None).unwrap();
            assert_eq!(sheets.iter().map(|s| s.slots.iter().map(|p| p.page).collect::<Vec<_>>()).collect::<Vec<_>>(), expected);
            let reverse = Imposition::Booklet { reverse: true }.sheets(&lay, &pages, None).unwrap();
            for (a, b) in sheets.iter().zip(reverse) {
                for slot in &a.slots {
                    let r = b.slots.iter().find(|p| p.page == slot.page).unwrap();
                    assert_eq!(r.x, a.width - slot.x - slot.width);
                }
            }
        }
    }
}
