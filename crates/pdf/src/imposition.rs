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
                    let (width, height) = super::export::Fit::of(pg, paper).paper_size();
                    Ok(Sheet { width, height, slots: vec![Slot { page, x: 0.0, y: 0.0, width, height }] })
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
    /// #641: independent folding sequences pin blank positions for both bindings at 5–9 pages.
    #[test]
    fn booklet_five_through_nine_pages_keep_blank_slot_positions() {
        let doc = wordcraft_doc::Document::from_text("1\u{c}2\u{c}3\u{c}4\u{c}5\u{c}6\u{c}7\u{c}8\u{c}9");
        let lay = wordcraft_layout::layout(&doc, &mut Default::default(), &Default::default());
        assert_eq!(lay.pages.len(), 9);
        let cases = [
            (5, vec![[None, Some(0)], [Some(1), None], [None, Some(2)], [Some(3), Some(4)]]),
            (6, vec![[None, Some(0)], [Some(1), None], [Some(5), Some(2)], [Some(3), Some(4)]]),
            (7, vec![[None, Some(0)], [Some(1), Some(6)], [Some(5), Some(2)], [Some(3), Some(4)]]),
            (8, vec![[Some(7), Some(0)], [Some(1), Some(6)], [Some(5), Some(2)], [Some(3), Some(4)]]),
            (9, vec![[None, Some(0)], [Some(1), None], [None, Some(2)], [Some(3), Some(8)], [Some(7), Some(4)], [Some(5), Some(6)]]),
        ];
        for (count, expected) in cases {
            for reverse in [false, true] {
                let pages: Vec<usize> = (0..count).collect();
                let sheets = Imposition::Booklet { reverse }.sheets(&lay, &pages, None).unwrap();
                assert_eq!(sheets.len(), expected.len(), "page count {count}, reverse {reverse}");
                let actual: Vec<_> = sheets
                    .iter()
                    .map(|side| {
                        assert!(side.width >= side.height);
                        assert!(side.slots.len() <= 2);
                        let mut cells = [None, None];
                        for slot in &side.slots {
                            assert_eq!(slot.width, side.width / 2.0);
                            assert_eq!(slot.height, side.height);
                            assert_eq!(slot.y, 0.0);
                            let col = usize::from(slot.x != 0.0);
                            assert_eq!(slot.x, col as f32 * side.width / 2.0);
                            assert!(cells[col].replace(slot.page).is_none());
                        }
                        cells
                    })
                    .collect();
                let wanted: Vec<_> = expected.iter().map(|&[left, right]| if reverse { [right, left] } else { [left, right] }).collect();
                assert_eq!(actual, wanted, "page count {count}, reverse {reverse}");
                let mut occupied: Vec<_> = actual.into_iter().flatten().flatten().collect();
                occupied.sort_unstable();
                assert_eq!(occupied, pages, "every selected page occurs exactly once");
            }
        }
    }
}
