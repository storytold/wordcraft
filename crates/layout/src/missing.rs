//! The characters the output would draw as an empty box ([`DocLayout::missing_glyphs`]): glyph
//! id 0 in a [`Draw::Glyphs`] of the display list, traced back to its character through the
//! run's `text` and per-glyph `ranges`. Backs strict export (#6005).

use std::collections::{BTreeMap, HashMap};
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
    /// How many distinct code points were missing, those past the cap included.
    pub total: usize,
    /// Every distinct code point found, kept only once the cap was passed (136 KiB), so merging
    /// the watermark's can still count the distinct ones.
    pub(crate) seen: Option<Box<CodePointSet>>,
}

impl MissingGlyphs {
    /// Nothing is missing.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty() && !self.truncated
    }

    /// A list of `chars` that fits under the cap: its total is its length.
    pub fn from_listed(chars: Vec<(char, u64)>) -> Self {
        let total = chars.len();
        Self { chars, truncated: false, total, seen: None }
    }

    /// Add `other` (the watermark's characters): counts of a shared code point are summed, new
    /// ones are listed while the cap allows, and the total counts each distinct one once.
    pub fn merge(&mut self, other: MissingGlyphs) {
        let mut all: BTreeMap<char, u64> = std::mem::take(&mut self.chars).into_iter().collect();
        let mut seen = self.seen.take();
        for (c, n) in other.chars {
            if let Some(count) = all.get_mut(&c) {
                *count = count.saturating_add(n);
            } else if let Some(set) = seen.as_mut() {
                set.insert(c);
            } else if all.len() < MAX_MISSING_GLYPHS {
                all.insert(c, n);
            } else {
                let mut set = CodePointSet::of(all.keys().copied());
                set.insert(c);
                seen = Some(Box::new(set));
            }
        }
        if let Some(theirs) = other.seen {
            seen.get_or_insert_with(|| Box::new(CodePointSet::of(all.keys().copied()))).union(&theirs);
        }
        self.truncated |= other.truncated || seen.is_some();
        self.total = seen.as_ref().map_or(all.len(), |s| s.len());
        self.chars = all.into_iter().collect();
        self.seen = seen;
    }
}

/// A set of code points as a bitset over the whole code point space: bounded at 136 KiB however
/// many distinct characters a hostile file holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodePointSet(Vec<u64>);

impl CodePointSet {
    fn empty() -> Self {
        Self(vec![0; 0x11_0000 / 64])
    }

    fn of(chars: impl Iterator<Item = char>) -> Self {
        let mut set = Self::empty();
        for c in chars {
            set.insert(c);
        }
        set
    }

    fn insert(&mut self, c: char) {
        let n = c as usize;
        if let Some(word) = self.0.get_mut(n / 64) {
            *word |= 1 << (n % 64);
        }
    }

    fn union(&mut self, other: &Self) {
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a |= b;
        }
    }

    fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
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
                found.walk(doc, &d);
            }
            // The page picture is drawn behind every page, outside the display list.
            if let Some(pic) = &doc.settings.page_picture {
                found.metafile_text(doc, &pic.media);
            }
        }
        found.finish()
    }
}

/// How the watermark text is shaped for output, as the PDF export and the page raster both do:
/// its first `max_chars` characters in the font its name resolves to (no fallback face), so a
/// character that font lacks shapes to glyph id 0. Strict export checks these same glyphs
/// ([`missing_watermark_glyphs`]), so the check and the exporters can't drift. `None` for a
/// blank watermark.
pub fn watermark_run(wm: &wordcraft_doc::Watermark, max_chars: usize) -> Option<(wordcraft_fonts::FaceRef, String, Vec<ShapedGlyph>)> {
    if wm.text.trim().is_empty() {
        return None;
    }
    let face = wordcraft_fonts::word::resolve(&wm.font, false, false).face;
    let text: String = wm.text.chars().take(max_chars).collect();
    let shaped = wordcraft_fonts::shape(&face, &text, &[], |c| c);
    Some((face, text, shaped))
}

/// How a metafile text run is shaped for output, as the PDF export and the page raster both do:
/// in the face its font name resolves to (no fallback face), so a character that face lacks shapes
/// to glyph id 0. Strict export checks these same glyphs, so the check and the exporters can't
/// drift.
pub fn metafile_text_run(t: &wordcraft_metafile::Text) -> (wordcraft_fonts::FaceRef, Vec<ShapedGlyph>) {
    let face = wordcraft_fonts::word::resolve(&t.font, t.bold, t.italic).face;
    let shaped = wordcraft_fonts::shape(&face, &t.text, &[], |c| c);
    (face, shaped)
}

/// The characters of the watermark that draw as an empty box (glyph id 0), counted as
/// [`DocLayout::missing_glyphs`] counts them; sorted by code point. The watermark is on every
/// page, so it counts once.
pub fn missing_watermark_glyphs(wm: &wordcraft_doc::Watermark, max_chars: usize) -> MissingGlyphs {
    let mut found = Found::default();
    if let Some((face, text, shaped)) = watermark_run(wm, max_chars) {
        let ranges = cluster_ranges(&text, &shaped);
        found.run(&face, shaped.iter().map(|g| g.gid), &text, &ranges);
    }
    found.finish()
}

#[derive(Default)]
struct Found {
    /// The text runs of each metafile picture met, by media key, shaped once: (face, text, ranges, glyph ids).
    metafiles: HashMap<String, Vec<MetaRun>>,
    counts: BTreeMap<char, u64>,
    truncated: bool,
    /// Every distinct code point, started when the list passes its cap.
    over: Option<Box<CodePointSet>>,
}

/// A metafile text run as drawn: its face, text, per-glyph ranges and glyph ids.
struct MetaRun {
    face: wordcraft_fonts::FaceRef,
    text: String,
    ranges: Vec<Range<usize>>,
    gids: Vec<u32>,
}

impl Found {
    /// The text of the metafile picture `key` (nothing for a picture that is not a metafile, does
    /// not parse or has no size: the exporters draw none of it), counted once per call.
    fn metafile_text(&mut self, doc: &Document, key: &str) {
        if !self.metafiles.contains_key(key) {
            let runs = doc.media.get(key).map(|bytes| meta_runs(bytes)).unwrap_or_default();
            self.metafiles.insert(key.to_string(), runs);
        }
        let Some(runs) = self.metafiles.remove(key) else { return };
        for r in &runs {
            self.run(&r.face, r.gids.iter().copied(), &r.text, &r.ranges);
        }
        self.metafiles.insert(key.to_string(), runs);
    }

    fn finish(self) -> MissingGlyphs {
        let total = self.over.as_ref().map_or(self.counts.len(), |s| s.len());
        MissingGlyphs { chars: self.counts.into_iter().collect(), truncated: self.truncated, total, seen: self.over }
    }

    fn walk(&mut self, doc: &Document, d: &Draw) {
        match d {
            Draw::Glyphs { face, glyphs, text, ranges, .. } => self.run(face, glyphs.iter().map(|g| g.0), text, ranges),
            Draw::Image { media, .. } => self.metafile_text(doc, media),
            Draw::Figure { draws: items, .. } | Draw::Turned { items, .. } | Draw::Rotated { items, .. } | Draw::Clip { items, .. } => {
                for d in items {
                    self.walk(doc, d);
                }
            }
            _ => {}
        }
    }

    /// A run of glyphs of `face` (`gids`) showing `text`, glyph `k` showing `ranges[k]`.
    fn run(&mut self, face: &wordcraft_fonts::FaceRef, gids: impl Iterator<Item = u32>, text: &str, ranges: &[Range<usize>]) {
        let glyph_count = gids.size_hint().1.unwrap_or(0);
        let one_each = ranges.is_empty() && text.chars().count() == glyph_count;
        let mut chars = text.chars();
        for (k, gid) in gids.enumerate() {
            // A run without ranges (a list label) maps one character to one glyph, when the
            // counts agree; else it can't be traced and isn't counted.
            let own = chars.next();
            if gid != 0 {
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

    fn count(&mut self, c: char) {
        if let Some(n) = self.counts.get_mut(&c) {
            *n = n.saturating_add(1);
        } else if let Some(set) = self.over.as_mut() {
            set.insert(c);
        } else if self.counts.len() < MAX_MISSING_GLYPHS {
            self.counts.insert(c, 1);
        } else {
            self.truncated = true;
            let mut set = CodePointSet::of(self.counts.keys().copied());
            set.insert(c);
            self.over = Some(Box::new(set));
        }
    }
}

/// The text runs of a metafile picture that the exporters draw (a run whose layout fails is not).
fn meta_runs(bytes: &[u8]) -> Vec<MetaRun> {
    if !wordcraft_metafile::is_metafile(bytes) {
        return Vec::new();
    }
    let Ok(pic) = wordcraft_metafile::parse(bytes) else { return Vec::new() };
    if !pic.has_size() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for item in &pic.items {
        let wordcraft_metafile::Item::Text(t) = item else { continue };
        let (face, shaped) = metafile_text_run(t);
        let glyphs: Vec<_> = shaped.iter().map(|g| (g.cluster, f64::from(g.x_advance), f64::from(g.x_offset), f64::from(g.y_offset))).collect();
        if t.layout(face.upem, face.ascent, face.descent, &glyphs).is_none() {
            continue;
        }
        let ranges = cluster_ranges(&t.text, &shaped);
        out.push(MetaRun { gids: shaped.iter().map(|g| g.gid).collect(), text: t.text.clone(), ranges, face });
    }
    out
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
