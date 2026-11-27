use super::paint::append_path;
use super::{FaceRef, GlyphId, HashMap, Path, PathBuilder};

/// A shaped glyph id as krilla takes it. Shaping gives 32-bit ids (a format-12 cmap can map to
/// any), but krilla's font subsetting takes 16-bit ones and panics on a larger id (#3674): one
/// that doesn't fit is written as .notdef (glyph 0), keeping its place. Its own character doesn't
/// extract, since krilla maps no text to .notdef.
pub(crate) fn glyph_id(g: u32) -> GlyphId {
    GlyphId::new(u16::try_from(g).map_or(0, u32::from))
}

/// The outlines of `glyphs` (glyph id, pen position) at `size`, as one path in page coordinates.
pub(crate) fn glyph_outlines(face: &FaceRef, size: f32, glyphs: &[(u32, f32, f32)]) -> Option<Path> {
    let db = wordcraft_fonts::FontDb::global();
    let scale = f64::from(size) / face.upem.max(1.0);
    let mut pb = PathBuilder::new();
    for &(g, x, y) in glyphs {
        // Outlines are in font units, y-down: scale to the size, move to the pen position.
        let mut outline = (*db.outline(face, g)).clone();
        outline.apply_affine(kurbo::Affine::translate((f64::from(x), f64::from(y))) * kurbo::Affine::scale(scale));
        append_path(&mut pb, &outline);
    }
    pb.finish()
}

/// Whether a run's glyphs already run left to right, as PDF text extractors (PDFium in Chrome and
/// Edge, pdf.js, poppler) take them to: they read a run's glyphs as written left to right and turn
/// right-to-left words back into reading order themselves. Layout gives right-to-left text in
/// reading order.
pub(crate) fn left_to_right(glyphs: &[(u32, f32, f32)]) -> bool {
    glyphs.windows(2).all(|w| matches!(w, [a, b] if a.1 <= b.1))
}

/// Per-glyph text ranges for clusters of several glyphs (a letter and its vowel marks) when each
/// glyph is one of the cluster's characters — through the font's character map, or of their
/// decomposition when the cluster has more glyphs than characters (Thai ำ draws as its nikhahit
/// and aa parts) — with one glyph at most left to take the character left over, so a
/// viewer reversing a right-to-left word puts every mark back after its letter, and every glyph
/// carries a character of its own into the ToUnicode maps for viewers that ignore /ActualText
/// (#5883). The returned text spells every split cluster's parts out, with both range lists
/// based into it: each glyph's cluster (shared where a cluster stayed whole, such as a ligature
/// drawn as two glyphs) and each glyph's own part of its cluster.
pub(crate) fn split_clusters(
    text: &str,
    glyphs: &[(u32, f32, f32)],
    ranges: &[std::ops::Range<usize>],
    rev: &HashMap<u32, char>,
) -> (String, Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
    use unicode_normalization::UnicodeNormalization;

    let mut out = String::with_capacity(text.len());
    let mut clusters = Vec::with_capacity(ranges.len());
    let mut parts = Vec::with_capacity(ranges.len());
    let mut i = 0;
    while i < ranges.len() {
        let Some(r) = ranges.get(i) else { break };
        let n = ranges.get(i..).map_or(1, |rest| rest.iter().take_while(|x| *x == r).count()).max(1);
        let whole: Vec<char> = text.get(r.clone()).map_or(Vec::new(), |t| t.chars().collect());
        // More glyphs than characters: the shaper drew the parts a character decomposes into
        // (Thai ำ is its nikhahit and aa, a compatibility decomposition).
        let chars = if n > whole.len() { whole.iter().flat_map(|c| c.nfkd()).collect() } else { whole };
        let mut split = None;
        if n > 1 && chars.len() == n {
            // Glyphs the character map knows take their characters; one left over (a letter's
            // contextual form, which the map doesn't list) takes the one character left.
            let mut used = vec![false; n];
            let mut of: Vec<Option<usize>> = (i..i + n)
                .map(|g| {
                    let c = rev.get(&glyphs.get(g)?.0)?;
                    let k = chars.iter().enumerate().position(|(k, ch)| ch == c && !used.get(k).copied().unwrap_or(true))?;
                    *used.get_mut(k)? = true;
                    Some(k)
                })
                .collect();
            let free: Vec<usize> = (0..n).filter(|k| !used.get(*k).copied().unwrap_or(true)).collect();
            if let ([k], [slot]) = (free.as_slice(), of.iter_mut().filter(|o| o.is_none()).collect::<Vec<_>>().as_mut_slice()) {
                **slot = Some(*k);
            }
            split = of.into_iter().collect::<Option<Vec<_>>>();
        }
        let start = out.len();
        match split {
            Some(of) => {
                let bounds: Vec<usize> = chars
                    .iter()
                    .map(|&c| {
                        let a = out.len();
                        out.push(c);
                        a
                    })
                    .collect();
                clusters.extend(std::iter::repeat_n(start..out.len(), n));
                for k in of {
                    let a = bounds[k];
                    parts.push(a..a + chars[k].len_utf8());
                }
            }
            None => {
                out.push_str(text.get(r.clone()).unwrap_or(""));
                let whole = start..out.len();
                clusters.extend(std::iter::repeat_n(whole.clone(), n));
                parts.extend(std::iter::repeat_n(whole, n));
            }
        }
        i += n;
    }
    (out, clusters, parts)
}

/// Indices of a run's glyphs in visual order, left to right: its clusters (consecutive glyphs
/// sharing a range in `clusters`, such as a letter and its marks, or a lam-alef ligature) sorted
/// by their leftmost glyph and kept together. Inside a cluster, glyphs go in reverse order of
/// their own text (`parts`) when it is right to left, so a viewer reversing the word reads the
/// letter before its marks.
pub(crate) fn visual_order(
    text: &str,
    glyphs: &[(u32, f32, f32)],
    clusters: &[std::ops::Range<usize>],
    parts: &[std::ops::Range<usize>],
) -> Vec<usize> {
    let mut groups: Vec<(f32, Vec<usize>)> = Vec::new();
    for (i, (_, x, _)) in glyphs.iter().enumerate() {
        match groups.last_mut() {
            Some((left, idx)) if i > 0 && clusters.get(i).is_some() && clusters.get(i) == clusters.get(i - 1) => {
                *left = left.min(*x);
                idx.push(i);
            }
            _ => groups.push((*x, vec![i])),
        }
    }
    // Clusters at the same x go in text order, reversed when right to left: some fonts draw
    // lam-alef as a lam and a zero-width alef at one x, and a right-to-left run whose ranges
    // don't decrease loses its reading order (#2310). Stable for glyphs sharing their text.
    let tie = |idx: &[usize]| -> i64 {
        let Some(r) = idx.first().and_then(|&g| clusters.get(g)) else { return 0 };
        let start = i64::try_from(r.start).unwrap_or(i64::MAX);
        if text.get(r.clone()).is_some_and(|t| t.chars().any(wordcraft_fonts::is_rtl)) { -start } else { start }
    };
    groups.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| tie(&a.1).cmp(&tie(&b.1))));
    groups
        .into_iter()
        .flat_map(|(_, mut idx)| {
            let rtl = idx.first().and_then(|&g| text.get(clusters.get(g)?.clone())).is_some_and(|t| t.chars().any(wordcraft_fonts::is_rtl));
            if rtl {
                idx.sort_by_key(|&g| std::cmp::Reverse(parts.get(g).map_or(0, |r| r.start)));
            }
            idx
        })
        .collect()
}

/// The y most of a run's glyphs share: its baseline, which raised or lowered marks leave.
pub(crate) fn baseline(glyphs: &[(u32, f32, f32)]) -> Option<f32> {
    let &(_, _, first) = glyphs.first()?;
    if glyphs.iter().all(|g| g.2.to_bits() == first.to_bits()) {
        return Some(first);
    }
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for (_, _, y) in glyphs {
        *counts.entry(y.to_bits()).or_insert(0) += 1;
    }
    // Ties go to the lower y bits, so the result doesn't depend on hash order.
    counts.into_iter().max_by_key(|&(bits, n)| (n, std::cmp::Reverse(bits))).map(|(bits, _)| f32::from_bits(bits))
}

/// Where each glyph's text ends: the cluster (byte offset) of the first later glyph whose cluster
/// is past the glyph's own, else `len`. One backward pass keeping the later clusters that can
/// still be an answer, so a long run of glyphs sharing a cluster (combining marks) stays linear.
pub(crate) fn cluster_ends(clusters: &[usize], len: usize) -> Vec<usize> {
    let mut ends = vec![len; clusters.len()];
    // Later clusters, nearest last, each greater than every one nearer.
    let mut later: Vec<usize> = Vec::new();
    for (end, &c) in ends.iter_mut().zip(clusters).rev() {
        while later.last().is_some_and(|&n| n <= c) {
            later.pop();
        }
        *end = later.last().copied().unwrap_or(len);
        later.push(c);
    }
    ends
}
