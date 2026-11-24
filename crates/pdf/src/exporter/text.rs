use std::borrow::Cow;

use super::{
    ContentTag, Exporter, FaceRef, Fill, FillRule, KrillaGlyph, Node, Point, Rect, Rgb, Role, SpanTag, Stroke, Surface, Transform, baseline,
    cluster_ends, fill, glyph_id, glyph_outlines, left_to_right, norm, ok, paint_of, rect_path, rgb, split_clusters_with_actual, visual_order,
};

/// How a run of glyphs is painted: its colour, opacity and synthetic bold and italic.
#[derive(Clone, Copy)]
pub(super) struct GlyphPaint {
    pub color: Rgb,
    pub alpha: f32,
    pub bold: bool,
    pub italic: bool,
}

impl Exporter<'_> {
    /// Unicode text for each glyph: (text, per-glyph byte range). Glyphs and the run's text are
    /// aligned through the font's reverse character map (ligatures take several characters,
    /// extra glyphs share their cluster's text).
    fn map_text(&mut self, face: &FaceRef, glyphs: &[(u32, f32, f32)], text: &str) -> (String, Vec<std::ops::Range<usize>>) {
        let cmap = self.cmap(face);
        let rev = |g: u32| cmap.get(&g).copied();
        if text.is_empty() {
            let mut t = String::new();
            let ranges = glyphs
                .iter()
                .map(|(g, _, _)| {
                    let a = t.len();
                    t.push(rev(*g).unwrap_or(' '));
                    a..t.len()
                })
                .collect();
            return (t, ranges);
        }
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let n = chars.len();
        let byte = |i: usize| chars.get(i).map(|c| c.0).unwrap_or(text.len());
        let mut ranges = Vec::with_capacity(glyphs.len());
        let mut i = 0usize;
        for (k, (g, _, _)) in glyphs.iter().enumerate() {
            let left_g = glyphs.len() - k;
            let left_c = n.saturating_sub(i);
            if left_c == 0 {
                let r = ranges.last().cloned().unwrap_or(0..text.len());
                ranges.push(r);
                continue;
            }
            if k + 1 == glyphs.len() {
                ranges.push(byte(i)..text.len());
                i = n;
                continue;
            }
            let this = chars.get(i).map(|c| c.1);
            let take = if left_c <= left_g || rev(*g) == this {
                if left_g > left_c && rev(*g) != this && k > 0 {
                    // More glyphs than characters: this one shares the previous cluster.
                    let r = ranges.last().cloned().unwrap_or(byte(i)..byte(i + 1));
                    ranges.push(r);
                    continue;
                }
                1
            } else {
                // A ligature (or substituted glyph): consume until the next glyph's character.
                let next = glyphs.get(k + 1).and_then(|(ng, _, _)| rev(*ng));
                let max = left_c - (left_g - 1);
                let mut t = 1;
                while t < max && chars.get(i + t).map(|c| c.1) != next {
                    t += 1;
                }
                if t == max && next.is_some() && chars.get(i + t).map(|c| c.1) != next {
                    // The next glyph doesn't match anything: assume one character each.
                    t = 1;
                }
                t
            };
            ranges.push(byte(i)..byte(i + take));
            i += take;
        }
        (text.to_string(), ranges)
    }

    /// Draws a positioned run of glyphs to the PDF surface.
    ///
    /// Resolves character ranges across multi-glyph clusters so that ToUnicode mappings
    /// and cluster-level `/ActualText` spans preserve original text for extractors, orders RTL
    /// glyphs visually when needed, and renders either font glyphs or vector outlines.
    pub(super) fn glyphs(
        &mut self,
        s: &mut Surface,
        (face, size): (&FaceRef, f32),
        glyphs: &[(u32, f32, f32)],
        style: GlyphPaint,
        (text, known): (&str, &[std::ops::Range<usize>]),
        para_tag: Option<usize>,
    ) {
        let GlyphPaint { color, alpha, bold, italic } = style;
        if !size.is_finite() || size <= 0.0 || size > 5000.0 {
            return;
        }
        if glyphs.is_empty() {
            return;
        }
        if !glyphs.iter().all(|(_, x, y)| ok(*x) && ok(*y)) {
            return;
        }
        let font = self.font(face);
        if font.is_none() && !self.outlined.contains(&face.id()) {
            log::warn!("PDF: font {} {} could not be embedded", face.family, face.style);
            return;
        }
        // Layout's glyph → text mapping when it has one (right-to-left text, contextual forms and
        // ligatures can't be guessed back from the font's cmap), else a guess.
        let valid = known.len() == glyphs.len() && known.iter().all(|r| r.start < r.end && text.get(r.clone()).is_some());
        let (txt, ranges) = if valid {
            (Cow::Borrowed(text), Cow::Borrowed(known))
        } else {
            let (mapped_txt, mapped_ranges) = self.map_text(face, glyphs, text);
            (Cow::Owned(mapped_txt), Cow::Owned(mapped_ranges))
        };
        let mut is_thai_or_lao = false;
        let mut is_rtl = false;
        for c in txt.chars() {
            if c >= '\u{0590}' {
                if matches!(c, '\u{0E00}'..='\u{0E7F}' | '\u{0E80}'..='\u{0EFF}') {
                    is_thai_or_lao = true;
                }
                if wordcraft_fonts::is_rtl(c) {
                    is_rtl = true;
                }
            }
        }
        let upem = face.upem.max(1.0) as f32;
        let needs_reorder = is_rtl && !left_to_right(glyphs);
        let has_thai_or_lao_clusters = is_thai_or_lao && ranges.windows(2).any(|w| w[0] == w[1]);
        let (order, ranges, txt, actual_spans) = if !needs_reorder && !has_thai_or_lao_clusters {
            ((0..glyphs.len()).collect(), ranges, txt, Vec::new())
        } else {
            let (new_txt, parts, actual_spans) = split_clusters_with_actual(&txt, glyphs, &ranges, &self.cmap(face));
            let order = if needs_reorder {
                visual_order(&txt, glyphs, &ranges, &parts)
            } else if has_thai_or_lao_clusters && !is_rtl {
                // Marks come out of shaping in visual order; write them in text order so a
                // viewer without `/ActualText` reads the cluster as it is spelled. Right-to-left
                // runs keep their visual order for the extractor to reverse.
                let mut order: Vec<usize> = (0..glyphs.len()).collect();
                order.sort_by_key(|&g| parts.get(g).map_or(usize::MAX, |r| r.start));
                order
            } else {
                (0..glyphs.len()).collect()
            };
            (order, Cow::Owned(parts), Cow::Owned(new_txt.into_owned()), actual_spans)
        };
        let placed: Vec<(u32, f32, f32, std::ops::Range<usize>)> =
            order.iter().filter_map(|&i| Some((glyphs.get(i).copied()?, ranges.get(i).cloned()?))).map(|((g, x, y), r)| (g, x, y, r)).collect();
        let Some(&(_, _, y0, _)) = placed.first() else { return };
        let pushed = italic;
        if italic {
            // The run's baseline: the y most of its glyphs share (marks sit higher or lower).
            let base = baseline(glyphs).unwrap_or(y0);
            s.push_transform(&Transform::from_row(1.0, 0.0, -0.21, 1.0, 0.21 * base, 0.0));
        }
        s.set_fill(Some(fill(color, alpha)));
        s.set_stroke(if bold {
            Some(Stroke { paint: rgb::Color::new(color.0, color.1, color.2).into(), width: size * 0.03, opacity: norm(alpha), ..Default::default() })
        } else {
            None
        });
        let draw_segment = |s: &mut Surface, font: &super::Font, seg: &[(u32, f32, f32, std::ops::Range<usize>)]| {
            let Some(&(_, sx0, sy0, _)) = seg.first() else { return };
            let kg: Vec<KrillaGlyph> = seg
                .iter()
                .enumerate()
                .map(|(k, (g, x, y, r))| {
                    let adv = match seg.get(k + 1) {
                        Some((_, nx, _, _)) => (nx - x) / size,
                        None => face.advance(*g) as f32 / upem,
                    };
                    KrillaGlyph::new(glyph_id(*g), adv, 0.0, (sy0 - y) / size, 0.0, r.clone(), None)
                })
                .collect();
            s.draw_glyphs(Point::from_xy(sx0, sy0), &kg, font.clone(), &txt, size, false);
        };
        match font {
            Some(font) => match para_tag.filter(|_| self.opts.tagged) {
                Some(idx) => {
                    for (seg_range, actual) in glyph_segments(&placed, &actual_spans) {
                        if let Some(seg) = placed.get(seg_range) {
                            let id = s.start_tagged(ContentTag::Span(SpanTag::empty().with_actual_text(actual)));
                            draw_segment(s, &font, seg);
                            s.end_tagged();
                            if let Some(e) = self.tags.get_mut(idx) {
                                e.children.push(Node::Leaf(id));
                            }
                        }
                    }
                }
                None => draw_segment(s, &font, &placed),
            },
            None => {
                let draw_outlines = |s: &mut Surface| {
                    if let Some(path) = glyph_outlines(face, size, glyphs) {
                        s.draw_path(&path);
                    }
                };
                match para_tag.filter(|_| self.opts.tagged) {
                    Some(idx) => self.tagged(s, Role::Para(idx), None, |_, s| draw_outlines(s)),
                    None => draw_outlines(s),
                }
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
        if pushed {
            s.pop();
        }
    }

    pub(super) fn link_rect(&mut self, face: &FaceRef, size: f32, glyphs: &[(u32, f32, f32)], link: &str) {
        let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else { return };
        let adv = face.advance(last.0) as f32 * size / face.upem.max(1.0) as f32;
        let (x0, x1) = (first.1.min(last.1), first.1.max(last.1) + adv);
        let (y0, y1) = (first.2 - size * 0.85, first.2 + size * 0.25);
        if ![x0, x1, y0, y1].iter().all(|v| ok(*v)) {
            return;
        }
        if let Some(prev) = self.links.last_mut()
            && prev.4 == link
            && (prev.1 - y0).abs() < 0.5
            && x0 - prev.2 < size
        {
            prev.0 = prev.0.min(x0);
            prev.2 = prev.2.max(x1);
            return;
        }
        if self.links.len() < 10_000 {
            self.links.push((x0, y0, x1, y1, link.to_string()));
        }
    }

    /// Draws a metafile text run as PDF text (outlines when the font cannot be embedded). `tf` maps
    /// its text space (`craft_metafile::Text::transform`) to the page.
    pub(crate) fn vector_text(&mut self, s: &mut Surface, t: &craft_metafile::Text, tf: kurbo::Affine) {
        let (face, shaped) = wordcraft_layout::metafile_text_run(t);
        let glyphs: Vec<_> = shaped.iter().map(|g| (g.cluster, f64::from(g.x_advance), f64::from(g.x_offset), f64::from(g.y_offset))).collect();
        let Some(lay) = wordcraft_layout::layout_metafile_text(t, face.upem, face.ascent, face.descent, &glyphs) else { return };
        let font = self.font(&face);
        if font.is_none() && !self.outlined.contains(&face.id()) {
            return;
        }
        let em = lay.em as f32;
        let c = tf.as_coeffs().map(|v| v as f32);
        if !c.iter().all(|v| v.is_finite()) || !em.is_finite() {
            return;
        }
        let (paint, opacity) = paint_of(t.color);
        s.push_transform(&Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5]));
        s.set_stroke(None);
        // An opaque background mode fills the character cells first.
        if let Some(bg) = t.background {
            let [x0, y0, x1, y1] = lay.cell.map(|v| v as f32);
            if let Some(cell) = rect_path(&Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }) {
                let (paint, opacity) = paint_of(bg);
                s.set_fill(Some(Fill { paint: paint.into(), opacity, rule: FillRule::NonZero }));
                s.draw_path(&cell);
            }
        }
        s.set_fill(Some(Fill { paint: paint.into(), opacity, rule: FillRule::NonZero }));
        match font {
            Some(font) => {
                let upem = face.upem.max(1.0) as f32;
                let ends = cluster_ends(&shaped.iter().map(|g| g.cluster).collect::<Vec<_>>(), t.text.len());
                let kg: Vec<KrillaGlyph> = shaped
                    .iter()
                    .enumerate()
                    .map(|(i, g)| {
                        let end = ends.get(i).copied().unwrap_or(t.text.len());
                        let start = g.cluster.min(end);
                        let x = lay.glyphs.get(i).map_or(0.0, |p| p.0);
                        let adv = match lay.glyphs.get(i + 1) {
                            Some(n) => ((n.0 - x) / lay.em) as f32,
                            None => g.x_advance as f32 / upem,
                        };
                        let dy = lay.glyphs.get(i).map_or(0.0, |p| ((lay.baseline - p.1) / lay.em) as f32);
                        KrillaGlyph::new(glyph_id(g.gid), adv, 0.0, dy, 0.0, start..end, None)
                    })
                    .collect();
                let x0 = lay.glyphs.first().map_or(0.0, |p| p.0) as f32;
                s.draw_glyphs(Point::from_xy(x0, lay.baseline as f32), &kg, font, &t.text, em, false);
            }
            None => {
                let pen: Vec<(u32, f32, f32)> = shaped.iter().zip(&lay.glyphs).map(|(g, (x, y))| (g.gid, *x as f32, *y as f32)).collect();
                if let Some(path) = glyph_outlines(&face, em, &pen) {
                    s.draw_path(&path);
                }
            }
        }
        s.set_fill(None);
        s.pop();
    }
}

/// Partitions `placed` into contiguous glyph slices paired with an optional `/ActualText` override
/// for any span in `actual_spans`. Each `actual_spans` entry applies only if all of its glyphs
/// appear contiguously so a split span never duplicates its `/ActualText`.
fn glyph_segments<'a>(
    placed: &[(u32, f32, f32, std::ops::Range<usize>)],
    actual_spans: &[(std::ops::Range<usize>, &'a str)],
) -> Vec<(std::ops::Range<usize>, Option<&'a str>)> {
    if actual_spans.is_empty() {
        return vec![(0..placed.len(), None)];
    }
    let mut emitted = vec![false; actual_spans.len()];
    let mut segs = Vec::new();
    let mut start = 0usize;
    let mut cur: Option<usize> = None;
    for (pos, (_, _, _, r)) in placed.iter().enumerate() {
        let next =
            actual_spans.iter().position(|(sr, _)| sr.start <= r.start && r.end <= sr.end).filter(|&i| !emitted.get(i).copied().unwrap_or(true));
        if pos == 0 {
            cur = next;
            continue;
        }
        if next != cur {
            if let Some(prev) = cur
                && let Some(flag) = emitted.get_mut(prev)
            {
                *flag = true;
            }
            segs.push((start..pos, cur.and_then(|i| actual_spans.get(i).map(|s| s.1))));
            start = pos;
            cur = next;
        }
    }
    segs.push((start..placed.len(), cur.and_then(|i| actual_spans.get(i).map(|s| s.1))));
    segs
}
