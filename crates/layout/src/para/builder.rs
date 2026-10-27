use super::bidi::{Dir, bidi_levels, strong_rtl, visual_order};
use super::chars::{
    SegKey, caps_expands, context_digit_dirs, is_khmer, is_mark, is_numeral_digit, ligature_features, lookup_family, remap_range, thai_run_face,
    upper_char,
};
use super::grid::east_asian;
use super::model::{ClKind, Cluster, Enclosed, Glyph, ParaEnv, RubyLayout, StyleRun, SymbolRun};
use super::*;

pub(super) struct Builder<'a> {
    pub(super) env: &'a ParaEnv<'a>,
    pub(super) styles: Vec<StyleRun>,
    /// Hash of a style's format → (index in `styles`, small caps), see `style`.
    pub(super) style_index: std::collections::HashMap<u64, Vec<(u32, bool)>>,
    pub(super) glyphs: Vec<Glyph>,
    pub(super) clusters: Vec<Cluster>,
    /// Bidi level per byte of the paragraph text (empty: all left to right).
    pub(super) levels: Vec<u8>,
    /// Text drawn by clusters that stand for an object, by cluster index (see `ParaLayout::shown`).
    pub(super) shown: Vec<(usize, String, Vec<std::ops::Range<usize>>)>,
}

impl<'a> Builder<'a> {
    /// Displayed form of `c` at run-relative byte `at` (Word › Numeral): one char in, one
    /// char out. `rtl` is the shaping direction; in Context mode `context` (from
    /// [`context_digit_dirs`], empty otherwise) gives each digit's surroundings, since European
    /// numbers take an even bidi level.
    fn numeral_char(&self, context: &[bool], at: usize, c: char, rtl: bool) -> char {
        let dir = if !context.is_empty() && is_numeral_digit(c) { context.get(at).copied().unwrap_or(false) } else { rtl };
        self.env.numeral.map(c, dir)
    }

    /// Style index for a resolved char in `face` (caps-scaled `size` override for small caps).
    pub(super) fn style(&mut self, rc: &Arc<ResolvedChar>, face_override: Option<FaceRef>, small: bool) -> u32 {
        let r = wordcraft_fonts::word::resolve(lookup_family(self.env.doc, &rc.font), rc.bold, rc.italic);
        let face = face_override.unwrap_or(r.face);
        // The display list reads every run's format (link target, strike kind, underline colour,
        // baseline shift…) from its StyleRun, so runs share one only when their formats are equal.
        // Keyed by a hash of the commonly varying fields (no allocation: this runs for every shaped
        // piece), then confirmed by comparing whole formats within the bucket.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (face.id(), small, &rc.font, rc.size.to_bits(), rc.bold, rc.italic, rc.color, rc.highlight, rc.shading).hash(&mut h);
            (rc.underline_color, rc.strike, rc.double_strike, &rc.link, rc.ins, rc.del, rc.hidden).hash(&mut h);
            h.finish()
        };
        let bucket = self.style_index.entry(key).or_default();
        // Formats that never compare equal (a NaN from a hostile file) get their own styles; a capped
        // scan keeps such a bucket from turning quadratic.
        for &(i, s) in bucket.iter().rev().take(64) {
            if s == small
                && let Some(st) = self.styles.get(i as usize)
                && st.face.id() == face.id()
                && (Arc::ptr_eq(&st.rc, rc) || *st.rc == **rc)
            {
                return i;
            }
        }
        let size = rc.draw_size() * if small { 0.8 } else { 1.0 };
        // A symbol font that isn't installed, or is installed one em tall (Apple's Symbol), takes
        // the height Word gives it.
        let (a, d) = match wordcraft_fonts::word::symbol_line(&rc.font, &r) {
            Some((a, d)) => (a * face.upem, d * face.upem),
            None => wordcraft_fonts::word::line_metrics(&face),
        };
        let k = size as f64 / face.upem.max(1.0);
        // A fallback face (letters the requested font lacks) is emboldened or slanted when it has
        // no bold or italic of its own.
        let (synth_bold, synth_italic) = match face_override {
            None => (r.synth_bold, r.synth_italic),
            Some(f) => (rc.bold && f.get().weight < 600.0, rc.italic && !f.get().italic),
        };
        let st = StyleRun {
            face,
            size,
            synth_bold,
            synth_italic,
            rc: rc.clone(),
            ascent: (a * k) as f32,
            descent: (d * k) as f32,
            shift: rc.baseline_shift(),
        };
        // A `u32` index (like the glyph ranges) counts more distinct formats than a paragraph can
        // hold in memory, so every format keeps its own style.
        let i = u32::try_from(self.styles.len()).unwrap_or(u32::MAX);
        self.styles.push(st);
        self.style_index.entry(key).or_default().push((i, small));
        i
    }

    /// Shape `text` (a piece of the paragraph starting at byte `base`) in one style and append clusters.
    pub(super) fn shape(&mut self, text: &str, base: usize, rc: &Arc<ResolvedChar>, kind_override: Option<ClKind>) {
        self.shape_with(text, base, rc, kind_override, Dir::Para);
    }

    /// `dir`: the bidi levels `text` is shaped by (see [`Dir`]).
    pub(super) fn shape_with(&mut self, text: &str, base: usize, rc: &Arc<ResolvedChar>, kind_override: Option<ClKind>, dir: Dir<'_>) {
        if text.is_empty() {
            return;
        }
        // Complex-script characters (Persian, Arabic…) use the run's complex-script font, size,
        // bold and italic.
        let rc_cs: Option<Arc<ResolvedChar>> = text.chars().any(|c| rc.uses_complex(c)).then(|| Arc::new(rc.complex()));
        let resolved = wordcraft_fonts::word::resolve(lookup_family(self.env.doc, &rc.font), rc.bold, rc.italic);
        let primary = resolved.face;
        // Symbol-font characters are laid out as their Unicode equivalents.
        let sym = SymbolRun::of(&rc.font, &resolved);
        let show = |c: char| sym.map_or(c, |m| m.show(c));
        let primary_cs = rc_cs.as_ref().map(|r| wordcraft_fonts::word::resolve(lookup_family(self.env.doc, &r.font), r.bold, r.italic).face);
        // Chinese, Japanese and Korean characters use the run's East Asian font (`w:eastAsia`) when
        // it is installed and has them.
        let primary_ea = rc
            .font_ea
            .as_ref()
            .filter(|f| !f.eq_ignore_ascii_case(&rc.font) && text.chars().any(east_asian))
            .map(|f| wordcraft_fonts::word::resolve(f, rc.bold, rc.italic))
            .filter(|r| !r.substituted)
            .map(|r| r.face);
        // The levels by byte of `text`, from `lbase` on.
        let (levels, lbase): (&[u8], usize) = match dir {
            Dir::Para => (&self.levels, base),
            Dir::Own(l) => (l, 0),
            Dir::Guess => (&[], 0),
        };
        let use_levels = !levels.is_empty();
        let context = if self.env.numeral == NumeralMode::Context && use_levels { context_digit_dirs(text, levels, lbase) } else { Vec::new() };
        // A missing Thai font: one face for the run's Thai and Latin text (#1549). A run with its
        // own complex-script font keeps that font for the Thai letters.
        let run_face: Option<FaceRef> = if primary_cs.is_none_or(|f| f.id() == primary.id()) {
            thai_run_face(&rc.font, resolved.substituted, primary, text, rc.bold, rc.italic)
        } else {
            None
        };
        // One fallback face for the run's East Asian characters: the first face in fallback order
        // covering every character the primary font lacks, so the run draws in a single CJK face
        // instead of mixing faces per character (e.g. a bundled Japanese Mincho for JIS-covered
        // ideographs beside a Simplified-Chinese sans for the rest). Whitespace and characters the
        // primary covers don't constrain the choice. `None` keeps the per-character fallback.
        let ea_face: Option<FaceRef> = {
            let ea: Vec<char> = text
                .chars()
                .filter(|c| crate::kinsoku::is_east_asian(*c) && !primary.covers(*c) && !c.is_whitespace() && !c.is_control() && *c != SOFT_HYPHEN)
                .collect();
            if ea.is_empty() {
                None
            } else {
                wordcraft_fonts::FontDb::global().fallback_for_run(&ea, primary.id(), rc.bold, rc.italic).map(|f| FaceRef::of(&f))
            }
        };
        // Split by font coverage (fallback faces), small caps case, script formatting and direction.
        let mut seg_start = 0;
        let mut cur: Option<SegKey> = None;
        let mut segs: Vec<(usize, usize, SegKey)> = Vec::new();
        for (i, c) in text.char_indices() {
            let rtl = use_levels && levels.get(lbase + i).is_some_and(|l| l % 2 == 1);
            // A symbol-font character becomes its Unicode symbol (#776).
            let c = show(c);
            // Displayed form (Word › Numeral): one char in, one char out. Coverage and the
            // fallback face are picked for the displayed char, so Hindi digits land in an
            // Arabic-capable face and Western digits in a Latin one.
            let dc = self.numeral_char(&context, i, c, rtl);
            let mark = is_mark(c);
            let complex = match cur {
                Some(k) if mark => k.complex,
                _ => rc_cs.is_some() && rc.uses_complex(dc),
            };
            let prim = if complex { primary_cs.unwrap_or(primary) } else { primary };
            let face = match cur {
                Some(k) if mark => k.face,
                _ if !complex && east_asian(c) && primary_ea.is_some_and(|f| f.covers(c)) => primary_ea,
                // The run's one substitute face for a missing Thai font (#1549).
                _ if !c.is_control() && c != SOFT_HYPHEN && run_face.is_some_and(|f| f.covers(dc)) => run_face,
                _ if prim.covers(dc) || c.is_whitespace() || c.is_control() || c == SOFT_HYPHEN => None,
                // Stay in the fallback face the word started in while it covers the letters.
                Some(SegKey { face: Some(f), .. }) if f.covers(dc) => Some(f),
                // The run's unified East Asian face, when one face covers the whole run.
                _ if !complex && ea_face.is_some() && crate::kinsoku::is_east_asian(c) => ea_face,
                _ => {
                    let (bold, italic) = if complex { (rc.bold_cs, rc.italic_cs) } else { (rc.bold, rc.italic) };
                    wordcraft_fonts::FontDb::global().fallback_styled(dc, prim.id(), bold, italic).map(|f| FaceRef::of(&f))
                }
            };
            let small = rc.small_caps && !rc.caps && c.is_lowercase();
            let k = SegKey { face, small, complex, rtl };
            match cur {
                Some(p) if p.same(&k) => {}
                Some(p) => {
                    segs.push((seg_start, i, p));
                    seg_start = i;
                    cur = Some(k);
                }
                None => cur = Some(k),
            }
        }
        if let Some(p) = cur {
            segs.push((seg_start, text.len(), p));
        }
        let no_kern = [wordcraft_fonts::feature_value(b"kern", 0)];
        for (a, b, key) in segs {
            let Some(sub) = text.get(a..b) else { continue };
            let rc = match (&rc_cs, key.complex) {
                (Some(cs), true) => cs.clone(),
                _ => rc.clone(),
            };
            let rc = &rc;
            let small = key.small;
            let si = self.style(rc, key.face, small);
            let Some(st) = self.styles.get(si as usize).cloned() else { continue };
            let upper = rc.caps || small;
            // All Caps and Small Caps draw a letter's full capital, which can be several
            // characters (ß → SS, ﬀ → FF, ŉ → ʼN), as Word does (#1581). A symbol-font run
            // draws its codes as symbols, never as expanded capitals.
            let expands = sym.is_none() && upper && sub.chars().any(caps_expands);
            // Displayed text: numeral substitution is 1 char → 1 char and a capital may be
            // several; `map_starts` holds where each document char's displayed form starts and
            // `orig_of` maps it back to the char's original byte offset (+ a `sub.len()` sentinel).
            let substituted = expands || sub.char_indices().any(|(i, c)| self.numeral_char(&context, a + i, c, key.rtl) != c);
            let mut mapped = String::new();
            let mut map_starts: Vec<usize> = Vec::new();
            let mut orig_of: Vec<usize> = Vec::new();
            if substituted {
                mapped = String::with_capacity(sub.len().saturating_add(8));
                for (i, c) in sub.char_indices() {
                    map_starts.push(mapped.len());
                    orig_of.push(i);
                    let d = self.numeral_char(&context, a + i, c, key.rtl);
                    if expands {
                        mapped.extend(d.to_uppercase());
                    } else {
                        mapped.push(d);
                    }
                }
                orig_of.push(sub.len());
            }
            let shape_text = if substituted { mapped.as_str() } else { sub };
            let map = |c: char| {
                let d = show(c);
                if d != c {
                    d
                } else if c == '\u{a0}' {
                    // Word sets a no-break space as wide as a space; some fonts draw their own
                    // U+00A0 glyph wider. Only the shaping changes: line breaking reads the text.
                    ' '
                } else if upper {
                    upper_char(c)
                } else {
                    c
                }
            };
            // The shaper kerns by default; Word kerns only when Kerning for fonts covers the size.
            // Without `w14:ligatures` the shaper's default (standard and contextual) stays.
            let [kern_off] = no_kern;
            // Khmer shaping builds its syllables (pre-base vowels, stacked signs) in `liga` and
            // `clig`, so a run's ligature setting never reaches Khmer text: Word ignores it there.
            let ligatures = rc.ligatures.filter(|_| !shape_text.chars().any(is_khmer));
            let all = match ligatures {
                Some(l) => {
                    let [a, b, c, d] = ligature_features(l);
                    [kern_off, a, b, c, d]
                }
                None => [kern_off; 5],
            };
            let from = usize::from(rc.kerned());
            let to = if ligatures.is_some() { all.len() } else { 1 };
            let features: &[wordcraft_fonts::Feature] = all.get(from..to).unwrap_or_default();
            let shaped = if use_levels {
                wordcraft_fonts::shape_run(&st.face, shape_text, features, map, key.rtl)
            } else {
                wordcraft_fonts::shape(&st.face, shape_text, features, map)
            };
            let k = st.size / st.face.upem.max(1.0) as f32;
            let hscale = rc.scale / 100.0;
            // Group glyphs by cluster byte offset; graphemes may span several shaper clusters.
            // Complex-script syllables the graphemes split (Myanmar vowel signs, stacked
            // consonants) stay one cluster: the caret never stops inside them.
            let mut bounds: Vec<usize> = wordcraft_doc::cluster::cluster_starts(shape_text);
            if expands {
                // The capitals drawn for one character (the two S of ß) are one cluster: the
                // caret, selection and line breaking never stop inside the character.
                bounds.retain(|b| map_starts.binary_search(b).is_ok());
            }
            let seg_c0 = self.clusters.len();
            let mut gi = 0usize;
            for (bi, &gs) in bounds.iter().enumerate() {
                let ge = bounds.get(bi + 1).copied().unwrap_or(shape_text.len());
                let g0 = self.glyphs.len() as u32;
                let mut adv = 0.0f32;
                while let Some(g) = shaped.get(gi) {
                    if g.cluster >= ge {
                        break;
                    }
                    self.glyphs.push(Glyph { gid: g.gid, dx: adv + g.x_offset as f32 * k * hscale, dy: g.y_offset as f32 * k });
                    adv += g.x_advance as f32 * k * hscale;
                    gi += 1;
                }
                let g1 = self.glyphs.len() as u32;
                let s = shape_text.get(gs..ge).unwrap_or("");
                let ch = s.chars().next().unwrap_or(' ');
                let kind = kind_override.unwrap_or(if ch == ' ' || ch == '\u{3000}' {
                    ClKind::Space
                } else if ch == SOFT_HYPHEN {
                    ClKind::Marker
                } else {
                    ClKind::Text
                });
                let adv = if kind == ClKind::Marker { 0.0 } else { adv + rc.spacing };
                // Clusters index the document text, never the displayed text.
                let (cs, ce) = if substituted { remap_range(&map_starts, &orig_of, sub.len(), gs, ge) } else { (gs, ge) };
                self.clusters.push(Cluster {
                    start: base + a + cs,
                    end: base + a + ce,
                    adv,
                    kind,
                    style: si,
                    g0,
                    g1: if kind == ClKind::Marker { g0 } else { g1 },
                    break_after: false,
                    obj_h: 0.0,
                    obj_d: 0.0,
                    dot: s == "." || s == ",",
                });
            }
            if matches!(dir, Dir::Para) {
                self.split_ligature_advances(seg_c0, rc.spacing, key.rtl);
            }
        }
    }

    /// A ligature glyph (fi, ffi, lam-alef) sits on the first cluster of the letters it draws and
    /// the clusters after it have no glyph and no width, so the caret, clicks and selection edges
    /// inside it all land after the whole glyph. Give each letter an even share of the glyph's
    /// advance, as Word and browsers do (#1974). The glyph stays on the first cluster; in
    /// right-to-left text that cluster is the rightmost, so the glyph moves back by the other
    /// letters' shares to start at the ligature's left edge. Clusters are grapheme clusters, so a
    /// letter with a combining mark is never split.
    fn split_ligature_advances(&mut self, from: usize, spacing: f32, rtl: bool) {
        let mut i = from;
        while let Some(head) = self.clusters.get(i) {
            if head.kind != ClKind::Text || head.g1 <= head.g0 {
                i += 1;
                continue;
            }
            let n =
                self.clusters.get(i + 1..).unwrap_or(&[]).iter().take_while(|c| c.kind == ClKind::Text && c.g1 <= c.g0 && c.adv == spacing).count();
            if n == 0 {
                i += 1;
                continue;
            }
            // The letters after the head are glyphless and carry only their character spacing, so
            // the ligature's width is the glyph's advance (spacing included) plus those spacings.
            let unit = (head.adv + spacing * n as f32) / (n + 1) as f32;
            let (g0, g1) = (head.g0 as usize, head.g1 as usize);
            if unit.is_finite() {
                for c in self.clusters.get_mut(i..=i + n).unwrap_or(&mut []) {
                    c.adv = unit;
                }
                if rtl {
                    for g in self.glyphs.get_mut(g0..g1).unwrap_or(&mut []) {
                        g.dx -= unit * n as f32;
                    }
                }
            }
            i += n + 1;
        }
    }

    /// One cluster for a whole object with its display text (fields, note references).
    pub(super) fn shape_atomic(&mut self, text: &str, start: usize, end: usize, rc: &Arc<ResolvedChar>) {
        // Right-to-left text in the result reads like ordinary text (UAX #9, #1670): the result is
        // an isolate whose direction is its first strong letter's (else the text's around the
        // field), and each level run is shaped (mirrored, joined) and placed in that direction.
        let around = self.levels.get(start).is_some_and(|l| l % 2 == 1);
        let rtl_base = text.chars().find_map(strong_rtl).unwrap_or(around);
        let lv = bidi_levels(text, rtl_base);
        let before = self.clusters.len();
        self.shape_with(text, start, rc, None, if lv.is_empty() { Dir::Guess } else { Dir::Own(&lv) });
        let added: Vec<Cluster> = self.clusters.drain(before..).collect();
        let Some(first) = added.first() else {
            let si = self.style(rc, None, false);
            let g = self.glyphs.len() as u32;
            self.clusters.push(Cluster {
                start,
                end,
                adv: 0.0,
                kind: ClKind::Marker,
                style: si,
                g0: g,
                g1: g,
                break_after: false,
                obj_h: 0.0,
                obj_d: 0.0,
                dot: false,
            });
            return;
        };
        let order = if lv.is_empty() {
            (0..added.len()).collect()
        } else {
            let cl_lv: Vec<u8> = added.iter().map(|c| lv.get(c.start.saturating_sub(start)).copied().unwrap_or(u8::from(rtl_base))).collect();
            visual_order(&cl_lv)
        };
        // Merge into one cluster: rebase glyph dx onto the first cluster drawn, and keep each
        // glyph's text (a ligature's glyph takes the letters of the clusters after it that have none).
        let mut x = 0.0;
        for c in order.iter().filter_map(|&i| added.get(i)) {
            for g in self.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dx += x;
            }
            x += c.adv;
        }
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        for c in &added {
            let (a, b) = (c.start.saturating_sub(start), c.end.saturating_sub(start));
            if c.g1 <= c.g0 {
                let from = ranges.last().map(|r| r.start);
                for r in ranges.iter_mut().rev().take_while(|r| Some(r.start) == from) {
                    r.end = b;
                }
            }
            ranges.extend((c.g0..c.g1).map(|_| a..b));
        }
        let g0 = first.g0;
        let style = first.style;
        let g1 = added.last().map(|c| c.g1).unwrap_or(g0);
        self.shown.push((self.clusters.len(), text.to_string(), ranges));
        self.clusters.push(Cluster { start, end, adv: x, kind: ClKind::Text, style, g0, g1, break_after: false, obj_h: 0.0, obj_d: 0.0, dot: false });
    }

    /// Shape `texts` on top of one another as one cluster for `start..end`: each aligned within
    /// the cluster (at least `width` wide) and raised `dy`. Returns the cluster's index.
    #[allow(clippy::too_many_arguments)]
    fn shape_overstruck(
        &mut self,
        texts: &[&str],
        start: usize,
        end: usize,
        rc: &Arc<ResolvedChar>,
        align: wordcraft_doc::eq::OAlign,
        width: f32,
        dy: f32,
    ) -> Option<usize> {
        use wordcraft_doc::eq::OAlign;
        let (c0, s0) = (self.clusters.len(), self.shown.len());
        for t in texts {
            self.shape_atomic(t, start, end, rc);
        }
        let added: Vec<Cluster> = self.clusters.drain(c0..).collect();
        let shown: Vec<(usize, String, Vec<std::ops::Range<usize>>)> = self.shown.drain(s0..).collect();
        let adv = added.iter().filter(|c| c.kind == ClKind::Text).map(|c| c.adv).fold(width.max(0.0), f32::max);
        let (mut text, mut ranges) = (String::new(), Vec::new());
        let (mut g0, mut g1) = (u32::MAX, 0u32);
        for (ci, t, r) in &shown {
            let Some(c) = added.get(ci.saturating_sub(c0)) else { continue };
            let dx = match align {
                OAlign::Left => 0.0,
                OAlign::Center => (adv - c.adv) / 2.0,
                OAlign::Right => adv - c.adv,
            };
            for g in self.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dx += dx;
                g.dy += dy;
            }
            let off = text.len();
            ranges.extend(r.iter().map(|x| x.start + off..x.end + off));
            text.push_str(t);
            g0 = g0.min(c.g0);
            g1 = g1.max(c.g1);
        }
        let g = self.glyphs.len() as u32;
        let (g0, g1) = if g0 <= g1 { (g0, g1) } else { (g, g) };
        let style = match added.iter().find(|c| c.kind == ClKind::Text) {
            Some(c) => c.style,
            None => self.style(rc, None, false),
        };
        let k = self.clusters.len();
        if !text.is_empty() {
            self.shown.push((k, text, ranges));
        }
        self.clusters.push(Cluster { start, end, adv, kind: ClKind::Text, style, g0, g1, break_after: false, obj_h: 0.0, obj_d: 0.0, dot: false });
        Some(k)
    }
}

/// Lay out an `EQ \o` field (ECMA-376 Part 1 §17.16.5.20): its arguments drawn on top of one
/// another, as one cluster. An enclosure symbol among them (○ □ △ ◇, see
/// [`wordcraft_doc::eq::EncloseShape`]) is drawn as a shape around the text rather than as a
/// glyph, so it never depends on the font: "shrink text" scales the text to fit inside a shape
/// one em high, "enlarge symbol" grows the shape around text at its own size.
pub(super) fn overstrike(
    b: &mut Builder,
    o: &wordcraft_doc::eq::Overstrike,
    instr: &str,
    start: usize,
    end: usize,
    rc: &Arc<ResolvedChar>,
    enclosures: &mut Vec<(usize, Enclosed)>,
) {
    use wordcraft_doc::eq::{EncloseShape, EncloseStyle, Enclosure};
    use wordcraft_doc::para::ShapeKind;
    let shapes: Vec<EncloseShape> = o.args.iter().filter_map(|a| EncloseShape::from_symbol(a)).collect();
    let texts: Vec<&str> = o.args.iter().filter(|a| !a.is_empty() && EncloseShape::from_symbol(a).is_none()).map(String::as_str).collect();
    let Some(shape) = shapes.first().copied() else {
        b.shape_overstruck(&texts, start, end, rc, o.align, 0.0, 0.0);
        return;
    };
    let style = Enclosure::parse(instr).map(|e| e.style).unwrap_or_default();
    let s = rc.size.clamp(1.0, 1638.0);
    // The widest text at the run's own size.
    let (c0, g0, s0) = (b.clusters.len(), b.glyphs.len(), b.shown.len());
    for t in &texts {
        b.shape_atomic(t, start, end, rc);
    }
    let w0 = b.clusters.get(c0..).map(|cs| cs.iter().map(|c| c.adv).fold(0.0f32, f32::max)).unwrap_or(0.0).max(0.0);
    b.clusters.truncate(c0);
    b.glyphs.truncate(g0);
    b.shown.truncate(s0);
    // The shape's box (`bw` × `bh`, centred `cy` above the baseline) and the text's size.
    let cy = s * 0.34;
    let (bw, bh, size) = match style {
        EncloseStyle::Shrink => {
            // Room for the text inside each shape: (height, width) as fractions of the box.
            let (fh, fw) = match shape {
                EncloseShape::Circle => (0.62, 0.7),
                EncloseShape::Square => (0.68, 0.78),
                EncloseShape::Triangle => (0.45, 0.5),
                EncloseShape::Diamond => (0.5, 0.55),
            };
            let bh = s * 1.05;
            let mut t = s * fh;
            if w0 > 0.0 && w0 * t / s > bh * fw {
                t = bh * fw * s / w0;
            }
            (bh, bh, t.max(s * 0.2))
        }
        EncloseStyle::Enlarge => {
            let (kh, kw) = match shape {
                EncloseShape::Circle => (1.4, 1.4),
                EncloseShape::Square => (1.3, 1.3),
                EncloseShape::Triangle => (1.75, 2.0),
                EncloseShape::Diamond => (1.7, 1.8),
            };
            let bh = s * kh;
            (bh.max(w0 * kw), bh, s)
        }
    };
    // A triangle's middle is low; the text sits there.
    let ty = if shape == EncloseShape::Triangle { cy - bh * 0.13 } else { cy };
    let trc = if (size - rc.size).abs() > 0.01 {
        let mut r = (**rc).clone();
        r.size = size;
        Arc::new(r)
    } else {
        rc.clone()
    };
    let pad = s * 0.05;
    let Some(k) = b.shape_overstruck(&texts, start, end, &trc, o.align, bw + 2.0 * pad, ty - size * 0.34) else { return };
    let adv = b.clusters.get(k).map(|c| c.adv).unwrap_or(bw);
    let kind = match shape {
        EncloseShape::Circle => ShapeKind::Ellipse,
        EncloseShape::Square => ShapeKind::Rectangle,
        EncloseShape::Triangle => ShapeKind::Triangle,
        EncloseShape::Diamond => ShapeKind::Diamond,
    };
    let stroke = (s * 0.06).clamp(0.4, 4.0);
    enclosures.push((k, Enclosed { kind, x: (adv - bw) / 2.0, top: cy + bh / 2.0, w: bw, h: bh, stroke }));
}

impl Builder<'_> {
    /// One cluster for ruby `base` text (an [`InlineObject::Ruby`] at `start..end`), as wide as the
    /// wider of the base and the ruby, with the ruby text laid out over it by `align`. The line
    /// grows to fit the ruby through the cluster's `obj_h`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ruby(
        &mut self,
        base: &str,
        ruby: &str,
        align: RubyAlign,
        raise: f32,
        start: usize,
        end: usize,
        rc: &Arc<ResolvedChar>,
        ruby_rc: &Arc<ResolvedChar>,
    ) -> Option<RubyLayout> {
        self.shape_atomic(base, start, end, rc);
        let ci = self.clusters.len().checked_sub(1)?;
        let (bc, bg) = (self.clusters.len(), self.glyphs.len());
        self.shape_with(ruby, 0, ruby_rc, None, Dir::Guess);
        let cl: Vec<Cluster> = self.clusters.drain(bc..).collect();
        let ruby_w: f32 = cl.iter().map(|c| c.adv).sum();
        let base_w = self.clusters.get(ci).map_or(0.0, |c| c.adv);
        let w = base_w.max(ruby_w);
        let n = cl.len().max(1) as f32;
        // Where the narrower text starts, and the extra gap between its letters (ruby only).
        let lead = |room: f32| match align {
            RubyAlign::Left => 0.0,
            RubyAlign::Right | RubyAlign::RightVertical => room,
            _ => room / 2.0,
        };
        let (x0, gap) = match align {
            RubyAlign::DistributeLetter if cl.len() > 1 => (0.0, (w - ruby_w) / (n - 1.0)),
            RubyAlign::DistributeSpace => ((w - ruby_w) / n / 2.0, (w - ruby_w) / n),
            _ => (lead(w - ruby_w), 0.0),
        };
        let mut glyphs = Vec::new();
        let mut ranges = Vec::new();
        let mut x = x0;
        for c in &cl {
            for g in self.glyphs.get(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                glyphs.push((c.style, Glyph { dx: g.dx + x, ..*g }));
                ranges.push(c.start..c.end);
            }
            x += c.adv + gap;
        }
        self.glyphs.truncate(bg);
        // The base text centres (or aligns) under a wider ruby.
        let shift = lead(w - base_w);
        let ascent = cl.iter().filter_map(|c| self.styles.get(c.style as usize)).map(|s| s.ascent).fold(0.0f32, f32::max);
        let c = self.clusters.get_mut(ci)?;
        for g in self.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
            g.dx += shift;
        }
        c.adv = w;
        c.obj_h = raise + ascent;
        Some(RubyLayout { cluster: ci, glyphs, text: ruby.to_string(), ranges, raise })
    }
}

/// Whether text with this formatting is left out of the layout: hidden text unless it is shown, and
/// tracked deletions in the final text. It takes no room, draws nothing and is no place to break.
pub(crate) fn left_out(doc: &Document, rc: &ResolvedChar, show_hidden: bool, hide_deleted: crate::HideDeleted) -> bool {
    (rc.hidden && !show_hidden) || hide_deleted.hides(doc, rc.del)
}
