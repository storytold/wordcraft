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

/// Per-glyph text ranges for clusters of several glyphs (a letter and its vowel marks, or
/// Thai/Lao composite vowels) when each glyph is one of the cluster's characters (through
/// the font's character map, one glyph at most left to take the character left over), so a viewer
/// reversing a right-to-left word puts every mark back after its letter. In Thai and Lao clusters
/// with composite vowels (such as SARA AM `\u{0E33}` or `\u{0EB3}`), decomposes the composite
/// vowel into NIKHAHIT and SARA AA when shaped as separate glyphs, giving each glyph its own
/// range in the decomposed text so ToUnicode mappings are unique and do not collide with
/// standalone SARA AA (`\u{0E32}` / `\u{0EB2}`). Clusters of any other script, such as a Devanagari
/// pre-base vowel or a ligature drawn as two glyphs, keep their shared logical range.
#[cfg(test)]
pub(crate) fn split_clusters<'a>(
    text: &'a str,
    glyphs: &[(u32, f32, f32)],
    ranges: &[std::ops::Range<usize>],
    rev: &HashMap<u32, char>,
) -> (std::borrow::Cow<'a, str>, Vec<std::ops::Range<usize>>) {
    let (txt, parts, _) = split_clusters_with_actual(text, glyphs, ranges, rev, true);
    (txt, parts)
}

/// Same as [`split_clusters`], plus `(byte_range_in_output, original_text)` spans for any
/// compatibility-decomposed Thai/Lao composite vowels so callers can preserve the original source
/// spelling in `/ActualText` while keeping per-glyph `ToUnicode` entries decomposed.
///
/// When `decompose_sara_am` is `false` (untagged paths that cannot wrap runs in `/ActualText`),
/// SARA AM clusters keep their shared logical range so `krilla` preserves the original spelling.
pub(crate) fn split_clusters_with_actual<'a>(
    text: &'a str,
    glyphs: &[(u32, f32, f32)],
    ranges: &[std::ops::Range<usize>],
    rev: &HashMap<u32, char>,
    decompose_sara_am: bool,
) -> (std::borrow::Cow<'a, str>, Vec<std::ops::Range<usize>>, Vec<(std::ops::Range<usize>, &'a str)>) {
    let has_sara_am =
        decompose_sara_am && ranges.iter().any(|r| text.get(r.clone()).is_some_and(|t| t.contains(['\u{0E33}', '\u{0EB3}']) && r.start < r.end));
    if !has_sara_am {
        let mut out = ranges.to_vec();
        let mut i = 0;
        while i < ranges.len() {
            let Some(r) = ranges.get(i) else { break };
            let n = ranges.get(i..).map_or(1, |rest| rest.iter().take_while(|x| *x == r).count()).max(1);
            let chars: Vec<(usize, char)> =
                text.get(r.clone()).map(|t| t.char_indices().map(|(k, c)| (r.start + k, c)).collect()).unwrap_or_default();
            let viewer_splits =
                text.get(r.clone()).is_some_and(|t| t.chars().any(|c| wordcraft_fonts::is_rtl(c) || matches!(c, '\u{0E00}'..='\u{0EFF}')));
            if n > 1 && chars.len() == n && viewer_splits {
                let cluster_glyphs = glyphs.get(i..i + n).unwrap_or(&[]);
                if let Some(cluster_out) = out.get_mut(i..i + n) {
                    split_one_to_one_cluster(&chars, cluster_glyphs, rev, cluster_out);
                }
            }
            i += n;
        }
        return (std::borrow::Cow::Borrowed(text), out, Vec::new());
    }

    let mut new_text = String::with_capacity(text.len() + 16);
    let mut out = ranges.to_vec();
    let mut actual_spans = Vec::new();
    let mut last_copied = 0;
    let mut i = 0;
    while i < ranges.len() {
        let Some(r) = ranges.get(i) else { break };
        let n = ranges.get(i..).map_or(1, |rest| rest.iter().take_while(|x| *x == r).count()).max(1);

        if r.start > last_copied
            && let Some(chunk) = text.get(last_copied..r.start)
        {
            new_text.push_str(chunk);
        }

        let sub = text.get(r.clone()).unwrap_or("");
        let chars: Vec<(usize, char)> = sub.char_indices().collect();
        let fc = chars.first().map(|c| c.1);

        if n > 1 && sub.contains(['\u{0E33}', '\u{0EB3}']) && fc.is_some_and(|c| c != '\u{0E33}' && c != '\u{0EB3}') {
            let cluster_glyphs = glyphs.get(i..i + n).unwrap_or(&[]);
            if let Some(cluster_out) = out.get_mut(i..i + n) {
                split_composite_vowel_cluster(sub, &chars, cluster_glyphs, rev, &mut new_text, cluster_out, &mut actual_spans);
            }
            last_copied = last_copied.max(r.end);
            i += n;
            continue;
        }

        let cluster_start = new_text.len();
        new_text.push_str(sub);
        let cluster_range = cluster_start..new_text.len();
        if let Some(dst) = out.get_mut(i..i + n) {
            dst.fill(cluster_range);
        }

        let chars_mapped: Vec<(usize, char)> = chars.iter().map(|(k, c)| (cluster_start + k, *c)).collect();
        let viewer_splits = sub.chars().any(|c| wordcraft_fonts::is_rtl(c) || matches!(c, '\u{0E00}'..='\u{0EFF}'));
        if n > 1 && chars_mapped.len() == n && viewer_splits {
            let cluster_glyphs = glyphs.get(i..i + n).unwrap_or(&[]);
            if let Some(cluster_out) = out.get_mut(i..i + n) {
                split_one_to_one_cluster(&chars_mapped, cluster_glyphs, rev, cluster_out);
            }
        }

        last_copied = last_copied.max(r.end);
        i += n;
    }

    if last_copied < text.len()
        && let Some(tail) = text.get(last_copied..)
    {
        new_text.push_str(tail);
    }

    (std::borrow::Cow::Owned(new_text), out, actual_spans)
}

fn split_one_to_one_cluster(
    chars: &[(usize, char)],
    cluster_glyphs: &[(u32, f32, f32)],
    rev: &HashMap<u32, char>,
    cluster_out: &mut [std::ops::Range<usize>],
) {
    let n = cluster_glyphs.len();
    let mut used = vec![false; n];
    let mut of: Vec<Option<usize>> = cluster_glyphs
        .iter()
        .map(|g| {
            let c = rev.get(&g.0)?;
            let k = chars.iter().enumerate().position(|(k, (_, ch))| ch == c && !used.get(k).copied().unwrap_or(true))?;
            *used.get_mut(k)? = true;
            Some(k)
        })
        .collect();
    let free: Vec<usize> = (0..n).filter(|k| !used.get(*k).copied().unwrap_or(true)).collect();
    if let ([k], [slot]) = (free.as_slice(), of.iter_mut().filter(|o| o.is_none()).collect::<Vec<_>>().as_mut_slice()) {
        **slot = Some(*k);
    }
    let parts: Option<Vec<std::ops::Range<usize>>> = of
        .iter()
        .map(|k| {
            let (a, ch) = chars.get((*k)?)?;
            Some(*a..a + ch.len_utf8())
        })
        .collect();
    if let Some(parts) = parts
        && parts.len() == cluster_out.len()
    {
        cluster_out.clone_from_slice(&parts);
    }
}

fn split_composite_vowel_cluster<'a>(
    sub: &'a str,
    chars: &[(usize, char)],
    cluster_glyphs: &[(u32, f32, f32)],
    rev: &HashMap<u32, char>,
    new_text: &mut String,
    cluster_out: &mut [std::ops::Range<usize>],
    actual_spans: &mut Vec<(std::ops::Range<usize>, &'a str)>,
) {
    let n = cluster_glyphs.len();
    let c_len = chars.first().map_or(0, |c| c.1.len_utf8());
    let marks = chars.len().saturating_sub(2);
    let nikhahit = |g: u32| matches!(rev.get(&g), Some('\u{0E4D}' | '\u{0ECD}'));
    let sara_aa = |g: u32| matches!(rev.get(&g), Some('\u{0E32}' | '\u{0EB2}'));
    let is_vowel = matches!(chars.last().map(|c| c.1), Some('\u{0E33}' | '\u{0EB3}'));
    let has_sara_aa_glyph = n > 2 && cluster_glyphs.last().is_some_and(|g| sara_aa(g.0));
    let vowel_char = chars.last().map(|c| c.1).unwrap_or('\0');
    let per_mark_structure = n >= 3 && marks == n - 3 && cluster_glyphs.get(1).is_some_and(|g| nikhahit(g.0));
    let lao_structure = vowel_char == '\u{0EB3}';
    let structure = is_vowel && has_sara_aa_glyph && (per_mark_structure || lao_structure);

    if structure && let Some(&(vowel_off, _)) = chars.last() {
        let (nikhahit_char, sara_aa_char) = if vowel_char == '\u{0E33}' { ('\u{0E4D}', '\u{0E32}') } else { ('\u{0ECD}', '\u{0EB2}') };
        let cluster_start = new_text.len();
        if let Some(pre_vowel) = sub.get(..vowel_off) {
            new_text.push_str(pre_vowel);
        }
        let nikhahit_start = new_text.len();
        new_text.push(nikhahit_char);
        let sara_aa_start = new_text.len();
        new_text.push(sara_aa_char);
        let sara_aa_end = new_text.len();

        let mut parts = Vec::with_capacity(n);
        parts.push(cluster_start..cluster_start + c_len);

        if per_mark_structure {
            parts.push(nikhahit_start..sara_aa_start);
            for k in 0..marks {
                if let Some(&(char_off, ch)) = chars.get(k + 1) {
                    parts.push((cluster_start + char_off)..(cluster_start + char_off + ch.len_utf8()));
                }
            }
        } else {
            let mark_range = (cluster_start + c_len)..sara_aa_start;
            for _ in 1..n - 1 {
                parts.push(mark_range.clone());
            }
        }
        parts.push(sara_aa_start..sara_aa_end);

        if parts.len() == cluster_out.len() {
            cluster_out.clone_from_slice(&parts);
            let actual_range = if per_mark_structure {
                (nikhahit_start..sara_aa_end, sub.get(vowel_off..))
            } else {
                ((cluster_start + c_len)..sara_aa_end, sub.get(c_len..))
            };
            if let (range, Some(orig)) = actual_range
                && !orig.is_empty()
            {
                actual_spans.push((range, orig));
            }
            return;
        }
    }

    let cluster_start = new_text.len();
    new_text.push_str(sub);
    let rest_range = (cluster_start + c_len)..new_text.len();
    let is_precomposed = n == 2 && cluster_glyphs.get(1).is_some_and(|g| chars.last().map(|c| c.1).is_some_and(|v| rev.get(&g.0) == Some(&v)));
    if is_precomposed {
        if let Some(first) = cluster_out.first_mut() {
            *first = cluster_start..cluster_start + c_len;
        }
        if let Some(second) = cluster_out.get_mut(1) {
            *second = rest_range;
        }
    } else {
        if let Some(first) = cluster_out.first_mut() {
            *first = cluster_start..cluster_start + c_len;
        }
        for slot in cluster_out.iter_mut().skip(1) {
            *slot = rest_range.clone();
        }
    }
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
            let cluster_text = idx.first().and_then(|&g| text.get(clusters.get(g)?.clone()));
            if cluster_text.is_some_and(|t| t.chars().any(wordcraft_fonts::is_rtl)) {
                idx.sort_by_key(|&g| std::cmp::Reverse(parts.get(g).map_or(0, |r| r.start)));
            } else if cluster_text.is_some_and(|t| t.chars().any(|c| matches!(c, '\u{0E00}'..='\u{0EFF}'))) {
                idx.sort_by_key(|&g| parts.get(g).map_or(usize::MAX, |r| r.start));
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
