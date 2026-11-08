//! The characters the output would draw as an empty box ([`DocLayout::missing_glyphs`]): glyph
//! id 0 in a [`Draw::Glyphs`] of the display list, traced back to its character through the
//! run's `text` and per-glyph `ranges`. Backs strict export (#6005).

use std::collections::BTreeMap;
use std::ops::{Range, RangeInclusive};

use wordcraft_doc::Document;
use wordcraft_fonts::ShapedGlyph;

use crate::DocLayout;
use crate::display::{DisplayOptions, Draw, page_display};
use crate::para::draws_something;

/// Most distinct code points [`DocLayout::missing_glyphs`] lists: a hostile file full of unmapped
/// characters can't make the report unbounded.
pub const MAX_MISSING_GLYPHS: usize = 256;

/// The missing glyphs of a render: characters it draws that no face in the font fallback chain can
/// draw (glyph id 0, an empty box).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MissingGlyphs {
    /// (code point, times drawn), sorted by code point, at most [`MAX_MISSING_GLYPHS`] long.
    pub chars: Vec<(char, u64)>,
    /// More than [`MAX_MISSING_GLYPHS`] distinct code points were missing: the rest aren't listed.
    pub truncated: bool,
}

impl MissingGlyphs {
    /// Nothing is missing.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty() && !self.truncated
    }
}

impl DocLayout {
    /// The characters drawn as glyph id 0 on `pages` (0-based, inclusive; `None` is every page),
    /// from the display list the PDF and PNG exports draw with `opts`: body, headers, footers,
    /// text boxes, notes, table cells, charts, diagrams and equations. A character counts once per
    /// glyph drawn for it. Default-ignorable and control characters never count (they draw
    /// nothing), and a Private Use Area character counts only if it really drew as glyph 0.
    /// Pages past the end are ignored.
    pub fn missing_glyphs(&self, doc: &Document, opts: &DisplayOptions, pages: Option<RangeInclusive<usize>>) -> MissingGlyphs {
        let mut found = Found::default();
        for (i, page) in self.pages.iter().enumerate() {
            if pages.as_ref().is_some_and(|r| !r.contains(&i)) {
                continue;
            }
            for d in page_display(doc, page, opts) {
                found.walk(&d);
            }
        }
        MissingGlyphs { chars: found.counts.into_iter().collect(), truncated: found.truncated }
    }
}

#[derive(Default)]
struct Found {
    counts: BTreeMap<char, u64>,
    truncated: bool,
}

impl Found {
    fn walk(&mut self, d: &Draw) {
        match d {
            Draw::Glyphs { face, glyphs, text, ranges, .. } => {
                let one_each = ranges.is_empty() && text.chars().count() == glyphs.len();
                let mut chars = text.chars();
                for (k, (gid, ..)) in glyphs.iter().enumerate() {
                    // A run without ranges (a list label) maps one character to one glyph, when
                    // the counts agree; else it can't be traced and isn't counted.
                    let own = chars.next();
                    if *gid != 0 {
                        continue;
                    }
                    let shown: Vec<char> = if one_each {
                        own.into_iter().collect()
                    } else {
                        ranges.get(k).and_then(|r| text.get(r.clone())).map(|t| t.chars().collect()).unwrap_or_default()
                    };
                    let drawn: Vec<char> = shown.into_iter().filter(|c| draws_something(*c)).collect();
                    // A cluster of several characters drew one empty box: it is the one the face
                    // lacks, not the letter beside it.
                    let lacking: Vec<char> = drawn.iter().copied().filter(|c| !face.covers(*c)).collect();
                    for c in if lacking.is_empty() { drawn } else { lacking } {
                        self.count(c);
                    }
                }
            }
            Draw::Figure { draws: items, .. } | Draw::Turned { items, .. } | Draw::Rotated { items, .. } | Draw::Clip { items, .. } => {
                for d in items {
                    self.walk(d);
                }
            }
            _ => {}
        }
    }

    fn count(&mut self, c: char) {
        if let Some(n) = self.counts.get_mut(&c) {
            *n = n.saturating_add(1);
        } else if self.counts.len() < MAX_MISSING_GLYPHS {
            self.counts.insert(c, 1);
        } else {
            self.truncated = true;
        }
    }
}

/// The byte range of `text` each of `shaped` shows: from its cluster to the next cluster start in
/// the text (or its end), whichever order the glyphs are in. A ligature's glyph covers all its
/// characters; the glyphs of one cluster share its range. Ranges always lie on character
/// boundaries of `text`.
pub(crate) fn cluster_ranges(text: &str, shaped: &[ShapedGlyph]) -> Vec<Range<usize>> {
    let mut starts: Vec<usize> = shaped.iter().map(|g| g.cluster.min(text.len())).filter(|b| text.is_char_boundary(*b)).collect();
    starts.sort_unstable();
    starts.dedup();
    shaped
        .iter()
        .map(|g| {
            let from = g.cluster.min(text.len());
            let from = if text.is_char_boundary(from) { from } else { (0..from).rev().find(|b| text.is_char_boundary(*b)).unwrap_or(0) };
            let to = match starts.binary_search(&from) {
                Ok(i) => starts.get(i + 1).copied().unwrap_or(text.len()),
                Err(_) => text.len(),
            };
            from..to.max(from)
        })
        .collect()
}
