use super::ParaLayout;
use super::model::{ClKind, Line, VisCl};

/// The bidi levels a piece of text is shaped by.
#[derive(Clone, Copy)]
pub(super) enum Dir<'l> {
    /// Paragraph text: the paragraph's levels at the piece's offset.
    Para,
    /// Text laid out on its own (a field result): these levels, by byte of the text.
    Own(&'l [u8]),
    /// Each strong run in its own direction (list labels, ruby, left-to-right results).
    Guess,
}

/// The direction of `c` if it is a strong letter (UAX #9 P2): `Some(true)` right to left.
pub(super) fn strong_rtl(c: char) -> Option<bool> {
    use unicode_bidi::BidiClass::{AL, L, R};
    // In ASCII only the letters are strong (L).
    if c.is_ascii() {
        return c.is_ascii_alphabetic().then_some(false);
    }
    match unicode_bidi::bidi_class(c) {
        R | AL => Some(true),
        L => Some(false),
        _ => None,
    }
}

/// Bidi embedding levels per byte of `text` (UAX #9 rules up to I2), or empty when the
/// paragraph is left to right and has no right-to-left characters.
pub fn bidi_levels(text: &str, rtl_para: bool) -> Vec<u8> {
    if !rtl_para && !text.chars().any(wordcraft_fonts::is_rtl) {
        return Vec::new();
    }
    let level = if rtl_para { unicode_bidi::Level::rtl() } else { unicode_bidi::Level::ltr() };
    let info = unicode_bidi::ParagraphBidiInfo::new(text, Some(level));
    let mut levels: Vec<u8> = info.levels.iter().map(|l| l.number()).collect();
    keep_latin_text_together(text, level.number(), &mut levels);
    levels
}

/// Keeps Latin text in order inside right-to-left text: "2-naphthol" and
/// "Windows 11" read as typed. The span runs from the word with the first
/// Latin letter to the last letter or digit, never across a right-to-left
/// letter, tab or embedding mark, and takes its letters' level. Sentence
/// punctuation at either end and the neutrals around it keep the base level.
fn keep_latin_text_together(text: &str, base: u8, levels: &mut [u8]) {
    use unicode_bidi::BidiClass::{AL, AN, B, BN, CS, EN, ES, ET, FSI, L, LRE, LRI, LRO, NSM, ON, PDF, PDI, R, RLE, RLI, RLO, S, WS};
    let chars: Vec<(usize, char, unicode_bidi::BidiClass)> = text.char_indices().map(|(i, c)| (i, c, unicode_bidi::bidi_class(c))).collect();
    let stop = |c: &unicode_bidi::BidiClass| matches!(c, R | AL | B | S | LRE | RLE | LRO | RLO | PDF | LRI | RLI | FSI | PDI);
    let sentence_end = |c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '…' | '،' | '؛' | '؟');
    // Bytes inside an explicit embedding, override or isolate: neutrals
    // there do not take the paragraph's level.
    let mut embedded = vec![false; text.len()];
    let mut depth = 0u32;
    for &(i, c, class) in &chars {
        match class {
            LRE | RLE | LRO | RLO | LRI | RLI | FSI => depth = depth.saturating_add(1),
            PDF | PDI => depth = depth.saturating_sub(1),
            B => depth = 0,
            _ => {}
        }
        if let Some(e) = embedded.get_mut(i..i + c.len_utf8()) {
            e.fill(depth > 0);
        }
    }
    for seg in chars.split(|(_, _, c)| stop(c)) {
        let Some(first) = seg.iter().position(|(_, _, c)| *c == L) else { continue };
        let Some(last) = seg.iter().rposition(|(_, _, c)| matches!(c, L | EN | AN)) else { continue };
        let word_start = seg.get(..first).and_then(|s| s.iter().rposition(|(_, _, c)| *c == WS)).map_or(0, |k| k + 1);
        let start = seg.get(word_start..first).and_then(|s| s.iter().position(|&(_, c, _)| !sentence_end(c))).map_or(first, |k| word_start + k);
        let word_end = seg.get(last..).and_then(|s| s.iter().position(|(_, _, c)| *c == WS)).map_or(seg.len(), |k| last + k);
        let end = seg.get(..word_end).and_then(|s| s.iter().rposition(|&(_, c, _)| !sentence_end(c))).map_or(0, |k| k + 1).max(last + 1);
        let Some(span) = seg.get(start..end) else { continue };
        let mut letter_levels = span.iter().filter(|(_, _, c)| *c == L).filter_map(|&(i, _, _)| levels.get(i).copied());
        let Some(lv) = letter_levels.next() else { continue };
        if !letter_levels.all(|l| l == lv) {
            continue;
        }
        let mut set = |&(i, c, _): &(usize, char, unicode_bidi::BidiClass), to: u8| {
            if let Some(l) = levels.get_mut(i..i + c.len_utf8()) {
                l.fill(to);
            }
        };
        // Combining marks at the span start belong to the right-to-left
        // letter before it (rule W1): they keep its level.
        span.iter().skip_while(|(_, _, c)| *c == NSM).for_each(|ch| set(ch, lv));
        // Inside an explicit embedding the neutrals keep their level.
        if lv > base.saturating_add(1) || seg.first().is_some_and(|&(i, _, _)| embedded.get(i).copied().unwrap_or(false)) {
            continue;
        }
        // A combining mark at the edge keeps the level of the letter before
        // it, so the word is shaped as one run.
        let neutral = |ch: &&(usize, char, unicode_bidi::BidiClass)| matches!(ch.2, WS | ON | CS | ES | ET | BN);
        let before = seg.get(..start).unwrap_or_default().iter().rev().take_while(neutral);
        let after = seg.get(end..).unwrap_or_default().iter().take_while(neutral);
        before.chain(after).for_each(|ch| set(ch, base));
    }
}

/// Rule L2 of UAX #9: the visual order (left to right) of items with these levels: from the
/// highest level down to the lowest odd one, reverse every run at that level or above.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(min_odd) = levels.iter().copied().filter(|l| l % 2 == 1).min() else { return order };
    let max = levels.iter().copied().max().unwrap_or(0);
    let level_at = |order: &[usize], i: usize| order.get(i).and_then(|&o| levels.get(o)).copied().unwrap_or(0);
    let mut lvl = max;
    while lvl >= min_odd {
        let mut i = 0;
        while i < order.len() {
            if level_at(&order, i) >= lvl {
                let mut j = i;
                while j < order.len() && level_at(&order, j) >= lvl {
                    j += 1;
                }
                if let Some(run) = order.get_mut(i..j) {
                    run.reverse();
                }
                i = j;
            } else {
                i += 1;
            }
        }
        if lvl == 0 {
            break;
        }
        lvl -= 1;
    }
    order
}

/// Make a row's lines as tall as its tallest, and return that height.
pub(super) fn even_row(row: &mut [Line]) -> f32 {
    let h = row.iter().map(|l| l.height).fold(0.0f32, f32::max);
    for l in row {
        l.baseline += h - l.height;
        l.height = h;
    }
    h
}

/// Finish a broken line of a bidirectional paragraph: resolve each cluster's level for the line
/// (UAX #9 rule L1), order the clusters visually (rule L2) and give each its box. `line.xs`
/// arrive in logical order measured from the paragraph's start edge (mirrored for a
/// right-to-left paragraph); afterwards `line.vis` holds the boxes and `xs` their left edges.
pub(super) fn reorder_line(pl: &ParaLayout, line: &mut Line, width: f32) {
    let rtl_para = line.rtl;
    if rtl_para {
        let (l, r) = (line.left, line.right);
        line.left = width - r;
        line.right = width - l;
    }
    let (c0, c1) = (line.c0, line.c1);
    let n = c1.saturating_sub(c0);
    if n == 0 {
        if rtl_para {
            line.xs = line.xs.iter().map(|x| width - x).collect();
        }
        return;
    }
    let para_level = u8::from(rtl_para);
    let mut levels: Vec<u8> = (c0..c1).map(|k| pl.clusters.get(k).and_then(|c| pl.bidi_levels.get(c.start)).copied().unwrap_or(para_level)).collect();
    // L1: tabs and breaks, and the whitespace before them or at the end of the line, take the
    // paragraph level (so trailing spaces stay at the line's end edge).
    let mut trailing = true;
    for (i, k) in (c0..c1).enumerate().rev() {
        let kind = pl.clusters.get(k).map(|c| c.kind);
        let reset = match kind {
            Some(ClKind::Tab | ClKind::LineBreak) => {
                trailing = true;
                true
            }
            Some(ClKind::Space | ClKind::PageBreak | ClKind::ColumnBreak | ClKind::Marker) => trailing,
            _ => {
                trailing = false;
                false
            }
        };
        if reset && let Some(v) = levels.get_mut(i) {
            *v = para_level;
        }
    }
    if !rtl_para && levels.iter().all(|l| *l == 0) {
        return; // A left-to-right line of a paragraph with right-to-left text elsewhere.
    }
    let order = visual_order(&levels);
    let hyph = line.hyphen.map_or(0.0, |h| h.2);
    let mut w: Vec<f32> = (0..n).map(|i| line.xs.get(i + 1).zip(line.xs.get(i)).map_or(0.0, |(b, a)| b - a)).collect();
    if let Some(last) = w.last_mut() {
        *last -= hyph;
    }
    let total: f32 = w.iter().sum();
    let x0 = line.xs.first().copied().unwrap_or(0.0);
    let mut x = if rtl_para { width - x0 - total } else { x0 };
    let mut vis = vec![VisCl { x: 0.0, w: 0.0, level: para_level }; n];
    for &i in &order {
        if let (Some(v), Some(wi), Some(l)) = (vis.get_mut(i), w.get(i), levels.get(i)) {
            *v = VisCl { x, w: *wi, level: *l };
            x += *wi;
        }
    }
    line.vis = vis;
    let end = line.end_x();
    line.xs = line.vis.iter().map(|v| v.x).chain(std::iter::once(end)).collect();
}
