use super::bidi::{even_row, reorder_line};
use super::grid::{Exclusion, Grid};
use super::justify::{HYPHENATION_ZONE, hyphen_glyph, shrink_line_spaces, space_room};
use super::model::{ClKind, Cluster, Line, LineEnd, ParaEnv};
use super::paragraph::word_widths;
use super::tabs::{decimal_position, next_tab, ptab_stop, resolve_tab};
use super::{Align, Arc, Level, LevelSuffix, LineSpacing, MathJc, ParaLayout, TabAlign, TabLeader, TabStop, VecDeque, WrapText};

/// Narrow cells and float spans must retain their usable width.
const MIN_LINE_WIDTH: f32 = 1.0;

/// What a paragraph's lines are broken with: the paragraph's measures and the state the lines
/// are broken in, from one to the next.
struct Breaker<'e> {
    level: Option<&'e Level>,
    rp: wordcraft_doc::resolve::ResolvedPara,
    mark_style: u32,
    exclusions: &'e [Exclusion],
    n: usize,
    width: f32,
    base_right: f32,
    first_left: f32,
    default_tab: f32,
    hanging_at: Option<f32>,
    /// Word 2013+ (compatibility mode 15) fits more text on a justified line by shrinking its
    /// spaces (`space_room`); never on lines with tabs.
    shrink_spaces: bool,
    /// Word stretches a justified line ending in a manual line break unless the document says not to.
    expand_break: bool,
    /// Hyphenation Options: the zone, and how many lines in a row may end with a hyphen.
    zone: f32,
    hyph_limit: usize,
    /// The width of every word, measured once: the word beside a float is measured again for
    /// every row it is tried on, and measuring it again costs a lookup rather than another pass
    /// over the word (#18).
    widths: Vec<f32>,
    /// A single punctuation character may extend past the text edge (`w:overflowPunct`).
    hanging_punctuation: Vec<bool>,
    /// Measured removable room and ink shift of eligible punctuation, by cluster.
    punctuation_room: Vec<PunctuationCompression>,
    /// A right-to-left paragraph is laid out from its start edge, as if mirrored: indents, tabs,
    /// the list label and alignment are measured from the right. Lines are mirrored back (and
    /// bidirectional text reordered) once broken; floating objects are mirrored in.
    rtl_para: bool,
    /// The document grid's line pitch, for paragraphs that snap to it.
    grid: Option<Grid>,
    bidi: bool,
    est_h: f32,
    table_h: f32,
    first_x0: Option<f32>,
    hcache: Vec<(u32, u32, f32)>,
    lines: Vec<Line>,
    /// The label is drawn on line 0 or, after an opening page break, line 1. Apply that
    /// line's alignment once both lines are known, before mirroring right-to-left labels.
    label_shifts: [f32; 2],
    /// The first cluster of the next line.
    i: usize,
    top: f32,
    first: bool,
    /// The rest of the current row: spans on the far side of floating objects, left to right.
    row_rest: VecDeque<(f32, f32)>,
    row_first: usize,
    /// Spans skipped in a row because the next word didn't fit (see `Scan::skip`).
    skipped: usize,
    /// A left tab past the right indent that wrapped to start the next line.
    wrapped_tab: Option<usize>,
}

/// One line being broken: where it is, and what has been put on it.
struct Scan {
    /// The span's left and right edges, and the row's left edge.
    left: f32,
    right_edge: f32,
    row_lo: f32,
    /// The span is narrowed by a floating object.
    narrowed: bool,
    /// A line already sits on this row (a span before this one may have been skipped).
    beside: bool,
    c0: usize,
    line_start_x: f32,
    x: f32,
    xs: Vec<f32>,
    leaders: Vec<(usize, TabLeader)>,
    /// Cluster index after which we may break.
    last_break: Option<usize>,
    /// The last non-hyphen break and the x after it.
    last_plain: Option<(usize, f32)>,
    /// Where the space before the current word starts.
    word_from: f32,
    /// After `hyph_limit` hyphenated lines in a row this one may not end with a hyphen.
    hyph_ok: bool,
    end: LineEnd,
    /// The cluster being looked at.
    j: usize,
    /// Tab cluster, stop, and x where the tab started.
    pending_tab: Option<(usize, TabStop, f32)>,
    /// The width of the spaces so far on this line.
    space_w: f32,
    has_tab: bool,
    /// Provisional compressed advances; only clusters kept on this line are committed.
    compressed: Vec<(usize, f32, f32)>,
    /// The first word doesn't fit a span narrowed by a floating object: it moves on whole.
    skip: bool,
}

#[derive(Clone, Copy, Default)]
struct PunctuationCompression {
    room: f32,
    ink_shift: f32,
    opening: bool,
}

/// Whether anything but markers (a bookmark, say) is among the clusters `c0..j`: an empty line
/// has no room to wrap from.
fn has_content(pl: &ParaLayout, c0: usize, j: usize) -> bool {
    (c0..j).any(|k| pl.clusters.get(k).is_some_and(|c| c.kind != ClKind::Marker))
}

/// A line's first item stays on it even when it doesn't fit; markers before it (a content
/// control's start, a bookmark) and floating objects' anchors take no room and don't count (Word:
/// a picture a little wider than the line, after an anchor, stays on the first line rather than
/// leaving the anchor a line of its own).
fn takes_room(c: &Cluster) -> bool {
    c.kind != ClKind::Marker && !(matches!(c.kind, ClKind::Object(_)) && c.adv == 0.0 && c.obj_h == 0.0)
}

/// The objects' exclusions mirrored across a line of `width`, for a right-to-left paragraph.
fn mirrored_exclusions(exclusions: &[Exclusion], width: f32) -> Vec<Exclusion> {
    exclusions
        .iter()
        .map(|e| Exclusion {
            left: width - e.right,
            right: width - e.left,
            contour: e.contour.as_ref().map(|c| Arc::new(c.mirrored(e.right - e.left))),
            // Mirrored, the object's left side is the line's right.
            side: match e.side {
                WrapText::Left => WrapText::Right,
                WrapText::Right => WrapText::Left,
                s => s,
            },
            ..e.clone()
        })
        .collect()
}

/// Whether shaping found one full-width glyph (rather than the font's missing glyph).
fn fullwidth_kana_glyph(shaped: &[wordcraft_fonts::ShapedGlyph], upem: f32) -> bool {
    matches!(shaped, [g] if g.gid != 0 && (g.x_advance as f32 / upem.max(1.0) - 1.0).abs() < 0.001)
}

/// Provisional width removed between this tab and its first decimal separator.
fn decimal_compression(compressed: &[(usize, f32, f32)], tab: usize, dot: Option<usize>, extra: Option<(usize, f32)>) -> f32 {
    let in_segment = |k| dot.is_some_and(|d| k > tab && k < d);
    let saved: f32 = compressed.iter().filter(|(k, _, _)| in_segment(*k)).map(|(_, amount, _)| amount).sum();
    saved + extra.filter(|(k, _)| in_segment(*k)).map_or(0.0, |(_, amount)| amount)
}

/// Break a paragraph into lines, measure each line, apply the paragraph's alignment and place
/// the list label. Fills `pl.lines`, `pl.height` and the label's position.
pub(super) fn break_lines(pl: &mut ParaLayout, text: &str, env: &ParaEnv, mark_style: u32, level: Option<&Level>) {
    let width = env.width.max(MIN_LINE_WIDTH);
    let mirrored = pl.rp.bidi.then(|| mirrored_exclusions(env.exclusions, width));
    let exclusions = mirrored.as_deref().unwrap_or(env.exclusions);
    let mut bk = Breaker::new(pl, (env, exclusions), mark_style, level);
    if bk.rp.overflow_punct {
        bk.hanging_punctuation = pl
            .clusters
            .iter()
            .map(|c| {
                if c.kind != ClKind::Text {
                    return false;
                }
                let Some(s) = text.get(c.start..c.end) else { return false };
                let mut chars = s.chars();
                let Some(ch) = chars.next() else { return false };
                let lang = pl.styles.get(c.style as usize).and_then(|s| s.rc.lang_ea.as_deref().or(s.rc.lang.as_deref()));
                chars.next().is_none() && crate::kinsoku::hanging_punctuation(ch, lang)
            })
            .collect();
    }
    if bk.rp.align == Align::Justify
        && matches!(
            env.doc.settings.character_spacing_control,
            wordcraft_doc::CharacterSpacingControl::CompressPunctuation | wordcraft_doc::CharacterSpacingControl::CompressPunctuationAndJapaneseKana
        )
    {
        let mut feature_cache = std::collections::HashMap::new();
        let mut fullwidth_kana_cache = std::collections::HashMap::new();
        bk.punctuation_room = pl
            .clusters
            .iter()
            .map(|c| {
                // Restrict the eligible characters to forms observed independently in Word.
                let Some(s) = text.get(c.start..c.end) else { return PunctuationCompression::default() };
                let mut chars = s.chars();
                let Some(ch) = chars.next() else { return PunctuationCompression::default() };
                let opening = "「『（".contains(ch);
                let closing = "、。，．」』）".contains(ch);
                let kana = matches!(ch as u32, 0x3041..=0x3096 | 0x30A1..=0x30FA)
                    && matches!(
                        env.doc.settings.character_spacing_control,
                        wordcraft_doc::CharacterSpacingControl::CompressPunctuationAndJapaneseKana
                    );
                if c.kind != ClKind::Text || chars.next().is_some() || !(opening || closing || kana) {
                    return PunctuationCompression::default();
                }
                let Some(st) = pl.styles.get(c.style as usize) else { return PunctuationCompression::default() };
                if st.rc.fit_text.is_some_and(|f| f.width > 0.0) {
                    return PunctuationCompression::default();
                }
                let em = st.size * st.rc.scale / 100.0;
                let spacing = st.rc.spacing * if env.doc.settings.balance_single_byte_double_byte_width { 2.0 } else { 1.0 };
                if kana {
                    // Apply this rule only to one full-width glyph from the resolved face.
                    let fullwidth = *fullwidth_kana_cache.entry((st.face.id(), ch)).or_insert_with(|| {
                        let shaped = wordcraft_fonts::shape(&st.face, &ch.to_string(), &[wordcraft_fonts::feature_value(b"kern", 0)], |c| c);
                        fullwidth_kana_glyph(&shaped, st.face.upem as f32)
                    });
                    if !fullwidth {
                        return PunctuationCompression::default();
                    }
                    // At 72pt Word admits 1/16 em of overrun but not 0.0632 em.
                    // Do not remove tracking or compression already applied by a grid.
                    let full_room = em / 16.0;
                    let room = (c.adv - (em - full_room + spacing).max(0.0)).min(full_room);
                    return if room.is_finite() && room > 0.0 {
                        PunctuationCompression { room, ink_shift: 0.0, opening: false }
                    } else {
                        PunctuationCompression::default()
                    };
                }
                let (feature_room, feature_shift) = *feature_cache.entry((st.face.id(), ch)).or_insert_with(|| {
                    let text = ch.to_string();
                    let normal = wordcraft_fonts::shape(&st.face, &text, &[wordcraft_fonts::feature_value(b"kern", 0)], |c| c);
                    let alt = wordcraft_fonts::shape(
                        &st.face,
                        &text,
                        &[wordcraft_fonts::feature_value(b"kern", 0), wordcraft_fonts::feature_value(b"palt", 1)],
                        |c| c,
                    );
                    match (normal.as_slice(), alt.as_slice()) {
                        ([a], [b]) if a.gid == b.gid && a.x_advance > b.x_advance => {
                            let upem = st.face.upem.max(1.0) as f32;
                            ((a.x_advance - b.x_advance) as f32 / upem, (b.x_offset - a.x_offset) as f32 / upem)
                        }
                        // The observed full-width comma bound is half an em when no palt
                        // is available. Opening marks also require a measured ink position.
                        _ if ch == '、' => (0.5, 0.0),
                        _ => (0.0, 0.0),
                    }
                });
                // Keep at least half an em plus the run's explicit tracking: a compressed grid or
                // earlier punctuation kerning must not remove the same bearing a second time.
                let full_room = (em * feature_room).clamp(0.0, em.max(0.0));
                let room = (c.adv - (em - full_room + spacing).max(0.0)).min(full_room);
                if room.is_finite() && room > 0.0 && full_room > 0.0 {
                    PunctuationCompression { room, ink_shift: em * feature_shift * room / full_room, opening }
                } else {
                    PunctuationCompression::default()
                }
            })
            .collect();
    }
    loop {
        let mut sc = bk.begin_line(pl);
        sc.scan(&mut bk, pl);
        if sc.skip {
            bk.skip_row(&sc);
            continue;
        }
        let end = bk.close_line(pl, sc, text);
        if bk.next_line(pl, end) {
            break;
        }
    }
    bk.finish(pl);
}

impl<'e> Breaker<'e> {
    fn new(pl: &ParaLayout, (env, exclusions): (&'e ParaEnv<'e>, &'e [Exclusion]), mark_style: u32, level: Option<&'e Level>) -> Self {
        let rp = pl.rp.clone();
        let width = env.width.max(MIN_LINE_WIDTH);
        let settings = &env.doc.settings;
        let est_h = pl
            .styles
            .get(mark_style as usize)
            .map(|st| (st.ascent + st.descent) * if let LineSpacing::Multiple(m) = rp.line_spacing { m } else { 1.0 })
            .unwrap_or(14.0)
            .max(1.0);
        Breaker {
            level,
            mark_style,
            exclusions,
            n: pl.clusters.len(),
            width,
            base_right: (width - rp.indent_right).max(1.0),
            first_left: rp.indent_left + rp.indent_first,
            default_tab: settings.default_tab,
            hanging_at: if rp.indent_first < 0.0 { Some(rp.indent_left) } else { None },
            shrink_spaces: rp.align == Align::Justify && settings.compat_mode >= wordcraft_doc::COMPAT_MODE_CURRENT,
            expand_break: !settings.do_not_expand_shift_return,
            zone: Some(settings.hyphenation_zone).filter(|z| z.is_finite() && *z >= 0.0).unwrap_or(HYPHENATION_ZONE).min(1584.0),
            hyph_limit: settings.consecutive_hyphen_limit as usize,
            widths: word_widths(&pl.clusters, &pl.hyph_after),
            hanging_punctuation: Vec::new(),
            punctuation_room: Vec::new(),
            rtl_para: rp.bidi,
            grid: Some(env.grid).filter(|g| g.line.is_some() && rp.snap_to_grid),
            bidi: rp.bidi || !pl.bidi_levels.is_empty(),
            est_h,
            // Exact spacing fixes the table-contact boundary even when the mark has other metrics.
            // Keep the existing estimate for shapes and for non-exact spacing.
            table_h: if let LineSpacing::Exactly(h) = rp.line_spacing { h } else { est_h },
            rp,
            first_x0: None,
            hcache: Vec::new(),
            lines: Vec::new(),
            label_shifts: [0.0; 2],
            i: 0,
            top: 0.0,
            first: true,
            row_rest: VecDeque::new(),
            row_first: 0,
            skipped: 0,
            wrapped_tab: None,
        }
    }

    /// The left edge of the row's first span: the indent, behind a drop cap on its lines.
    fn row_left(&self, pl: &ParaLayout) -> f32 {
        let lo = if self.first { self.first_left } else { self.rp.indent_left };
        match pl.drop_cap {
            Some((_, dl, dw)) if dw > 0.0 && self.lines.len() < dl as usize => self.first_left.max(self.rp.indent_left) + dw,
            _ => lo,
        }
    }

    /// The next span of text: the rest of the row, else the first of the next row (flowing around
    /// floating objects: the spans beside them, or below them). Its left and right edges, and
    /// whether a floating object narrows it.
    fn next_span(&mut self, pl: &ParaLayout) -> (f32, f32, bool) {
        let narrowed = !self.row_rest.is_empty();
        if let Some((left, right)) = self.row_rest.pop_front() {
            return (left, right, narrowed);
        }
        let mut guard = 0;
        loop {
            let lo = self.row_left(pl);
            match row_spans(self.exclusions, self.top, self.est_h, self.table_h, lo, self.base_right).map(|mut spans| (spans.pop_front(), spans)) {
                Ok((Some((a, b)), rest)) => {
                    let narrowed = !rest.is_empty() || a > lo || b < self.base_right;
                    self.row_rest = rest;
                    self.row_first = self.lines.len();
                    return (a, b.max(a + MIN_LINE_WIDTH), narrowed);
                }
                Err(below) if below > self.top && guard < 50 => {
                    self.top = below;
                    guard += 1;
                }
                Ok((None, _)) | Err(_) => {
                    self.row_first = self.lines.len();
                    return (lo, self.base_right.max(lo + MIN_LINE_WIDTH), narrowed);
                }
            }
        }
    }

    /// The start of a line: its span, and the list label if it is the first.
    fn begin_line(&mut self, pl: &mut ParaLayout) -> Scan {
        let beside = !self.row_rest.is_empty() && self.lines.len() > self.row_first;
        let row_lo = if self.first { self.first_left } else { self.rp.indent_left };
        let (left, right_edge, narrowed) = self.next_span(pl);
        let x = if self.first { self.place_label(pl, left) } else { left };
        Scan {
            left,
            right_edge,
            row_lo,
            narrowed,
            beside,
            c0: self.i,
            line_start_x: x,
            x,
            xs: Vec::new(),
            leaders: Vec::new(),
            last_break: None,
            last_plain: None,
            word_from: left,
            hyph_ok: self.hyph_limit == 0 || self.lines.iter().rev().take_while(|l| l.hyphen.is_some()).count() < self.hyph_limit,
            end: LineEnd::Para,
            j: self.i,
            pending_tab: None,
            space_w: 0.0,
            has_tab: false,
            compressed: Vec::new(),
            skip: false,
        }
    }

    /// Place the label on the first line, `left` being the first-line indent. Returns where the
    /// text starts.
    fn place_label(&self, pl: &mut ParaLayout, left: f32) -> f32 {
        let Some(lab) = pl.label.as_mut() else { return left };
        // The number is aligned at the first-line indent: its left edge, centre or right edge.
        lab.x = match self.level.map(|l| l.align) {
            Some(Align::Center) => left - lab.width / 2.0,
            Some(Align::Right) => left - lab.width,
            _ => left,
        };
        let end = lab.x + lab.width;
        match self.level.map(|l| l.suffix).unwrap_or(LevelSuffix::Tab) {
            LevelSuffix::Tab => {
                let t = next_tab(end, &self.rp.tabs, self.default_tab, self.hanging_at).pos;
                // The level's own tab stop after the number, when it comes first.
                match self.level.and_then(|l| l.tab).filter(|p| p.is_finite() && *p > end + 0.01) {
                    Some(p) if p < t => p,
                    _ => t,
                }
            }
            LevelSuffix::Space => end + pl.styles.get(lab.style as usize).map(|s| s.size * 0.25).unwrap_or(3.0),
            LevelSuffix::Nothing => end,
        }
    }

    /// The first word doesn't fit the span beside a floating object: try the row's next span,
    /// or the next row.
    fn skip_row(&mut self, sc: &Scan) {
        self.skipped += 1;
        if !self.row_rest.is_empty() {
            return;
        }
        self.top = if self.lines.len() > self.row_first {
            self.top + even_row(self.lines.get_mut(self.row_first..).unwrap_or_default())
        } else {
            // Nothing on this row: go below the first object that ends, as when no span is wide enough.
            self.exclusions
                .iter()
                .filter(|e| e.intersects_row(self.top, self.est_h, self.table_h, sc.row_lo, self.base_right))
                .map(|e| e.bottom)
                .reduce(f32::min)
                .unwrap_or(self.top + self.est_h)
        };
    }

    /// Finish the line scanned: lay out its clusters, measure and align it, and add it.
    /// Returns how it ends.
    fn close_line(&mut self, pl: &mut ParaLayout, mut sc: Scan, text: &str) -> LineEnd {
        // A scan can backtrack to an earlier break or abandon a float span. Keep only the
        // compression of clusters that actually made it onto this line.
        for &(index, amount, shift) in &sc.compressed {
            if index < sc.j
                && let Some(cluster) = pl.clusters.get_mut(index)
            {
                cluster.adv = (cluster.adv - amount).max(0.0);
                for glyph in pl.glyphs.get_mut(cluster.g0 as usize..cluster.g1 as usize).into_iter().flatten() {
                    glyph.dx += shift;
                }
            }
        }
        if sc.end == LineEnd::Para || sc.end == LineEnd::Wrap {
            let xc = sc.x;
            resolve_tab(pl, &mut sc.pending_tab, &mut sc.xs, sc.c0, xc, &mut sc.x);
        }
        if sc.j >= self.n && sc.end != LineEnd::Wrap && !matches!(sc.end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) {
            sc.end = LineEnd::Para;
        }
        let (c1, j, hyphen) = self.settle_clusters(pl, &mut sc);
        let v = self.measure(pl, (sc.c0, c1));
        self.align(pl, &mut sc, c1, text);
        let start = pl.clusters.get(sc.c0).map(|c| c.start).unwrap_or(pl.text_len);
        let stop = if c1 > sc.c0 { pl.clusters.get(c1 - 1).map(|c| c.end).unwrap_or(pl.text_len) } else { start };
        if self.first_x0.is_none() {
            self.first_x0 = sc.xs.first().copied();
        }
        let end = sc.end;
        let mut line = Line {
            top: self.top,
            height: v.height,
            baseline: v.baseline,
            text_asc: v.text_asc,
            text_desc: v.text_desc,
            c0: sc.c0,
            c1,
            xs: sc.xs,
            leaders: sc.leaders,
            end,
            start,
            stop,
            left: sc.left,
            right: sc.right_edge,
            hyphen,
            beside: sc.beside,
            vis: Vec::new(),
            rtl: self.rtl_para,
        };
        if self.bidi {
            reorder_line(pl, &mut line, self.width);
        }
        self.lines.push(line);
        self.first = false;
        self.skipped = 0;
        self.i = j;
        end
    }

    /// The line's clusters up to `c1` (at least one, so the paragraph progresses) with an x each
    /// and the line's end, a hyphen after the last if it wrapped at a hyphenation point. Returns
    /// the end of the clusters, where the next line starts and the hyphen.
    fn settle_clusters(&mut self, pl: &ParaLayout, sc: &mut Scan) -> (usize, usize, Option<(u32, u32, f32)>) {
        let (c0, n) = (sc.c0, self.n);
        let c1 = sc.j.max(c0);
        let (c1, j) = if c1 == c0 && c0 < n { (c0 + 1, c0 + 1) } else { (c1, sc.j) };
        while sc.xs.len() < c1 - c0 {
            let last = sc.xs.last().copied().unwrap_or(sc.line_start_x);
            let adv = pl.clusters.get(c0 + sc.xs.len().saturating_sub(1)).map(|c| c.adv).unwrap_or(0.0);
            sc.xs.push(if sc.xs.is_empty() { sc.line_start_x } else { last + adv });
        }
        sc.xs.truncate(c1 - c0);
        let hyphen = (sc.end == LineEnd::Wrap && c1 > c0 && pl.hyph_after.binary_search(&((c1 - 1) as u32)).is_ok())
            .then(|| pl.clusters.get(c1 - 1).map(|c| hyphen_glyph(pl, c.style, &mut self.hcache)))
            .flatten();
        let end_x = match (sc.xs.last(), c1.checked_sub(1).and_then(|k| pl.clusters.get(k))) {
            (Some(lx), Some(c)) => lx + c.adv + hyphen.map_or(0.0, |h| h.2),
            _ => sc.line_start_x,
        };
        sc.xs.push(end_x);
        (c1, j, hyphen)
    }

    /// The line's ascent and descent over its baseline from the clusters `c0..c1`, and the
    /// tallest picture on it (which sits on the baseline).
    fn extents(&self, pl: &ParaLayout, (c0, c1): (usize, usize)) -> Extents {
        let rp = &self.rp;
        let mut ex = Extents { asc: 0.0, desc: 0.0, obj_asc: 0.0 };
        let mut any = false;
        let dropped = pl.drop_cap.map_or(0, |d| d.0);
        for (k, c) in pl.clusters.get(c0..c1).into_iter().flatten().enumerate() {
            // Spaces and tabs never make a line taller, whatever their size: Word sizes a line by
            // its other characters (a line of only them takes the paragraph mark's), measured in
            // Word for Windows with 9 pt text and 11–20 pt tabs, spaces and ideographic spaces.
            // Floating and hidden drawings have no inline extent; their anchor font takes no height.
            let anchor = matches!(c.kind, ClKind::Object(_)) && c.adv == 0.0 && c.obj_h == 0.0;
            if matches!(c.kind, ClKind::Marker | ClKind::Space | ClKind::Tab) || anchor || c0 + k < dropped {
                continue;
            }
            let Some(st) = pl.styles.get(c.style as usize) else { continue };
            // A picture sits on the baseline of its run: the line keeps that font's ascent
            // and descent, and grows above the baseline to fit a taller picture.
            let (a, d) = if matches!(c.kind, ClKind::Object(_)) && c.obj_h > 0.0 {
                if pl.maths.iter().any(|(k, _)| c.kind == ClKind::Object(*k)) {
                    // Exact spacing keeps the run's baseline; tall math may overflow the
                    // fixed line box without moving adjacent text up by its denominator.
                    if matches!(rp.line_spacing, LineSpacing::Exactly(_)) { (st.ascent, st.descent) } else { (c.obj_h, c.obj_d) }
                } else {
                    ex.obj_asc = ex.obj_asc.max(c.obj_h);
                    (st.ascent, st.descent)
                }
            } else {
                // `obj_h` is set on text clusters only under ruby text, which the line grows to fit.
                let (a, d) = st.line_extent();
                (a.max(c.obj_h + st.shift), d)
            };
            ex.asc = ex.asc.max(a);
            ex.desc = ex.desc.max(d);
            any = true;
        }
        if (!any || (ex.asc == 0.0 && ex.desc == 0.0))
            && let Some(st) = pl.styles.get(self.mark_style as usize)
        {
            ex.asc = st.ascent;
            ex.desc = st.descent;
        }
        ex
    }

    /// The line's height, baseline and text extent.
    fn measure(&self, pl: &ParaLayout, range: (usize, usize)) -> Measure {
        let rp = &self.rp;
        let Extents { mut asc, mut desc, obj_asc } = self.extents(pl, range);
        let text_natural = asc + desc;
        let (text_asc, text_desc) = (asc.max(obj_asc), desc);
        let text_pictures = if obj_asc > asc { obj_asc + desc } else { 0.0 };
        let mut label_extra = 0.0;
        if self.first
            && let Some(lab) = &pl.label
        {
            if !matches!(rp.line_spacing, LineSpacing::Exactly(_)) {
                // Automatic/minimum spacing keeps text descent. Only the label's ascent
                // overflow adds room, unscaled by the automatic spacing multiplier.
                label_extra = (lab.ascent - asc).max(0.0);
                asc = asc.max(lab.ascent);
            } else {
                for st in lab.runs.iter().filter_map(|r| pl.styles.get(r.style as usize)) {
                    asc = asc.max(st.ascent);
                    desc = desc.max(st.descent);
                }
            }
        }
        // Keep the picture floor independent of list-label metrics and line-spacing multipliers.
        // shortcut: Word leaves less than a full descent below a line of only pictures (0.5pt to
        // 1.9pt in docxide-pdf's header fixtures case121/case123, where the descent is 3.3pt), and
        // no rule tried so far (none, only the text's descent) matched both; it needs Word probes.
        let natural = asc + desc;
        let pictures = if obj_asc > asc { obj_asc + desc } else { 0.0 };
        // Document grid: the line takes whole line pitches, its text centred in them.
        let grid = self.grid.as_ref();
        let (height, centred) = match (rp.line_spacing, grid) {
            (LineSpacing::Multiple(m), Some(g)) => ((g.snap_height(natural) * m).max(g.snap_height(pictures)), true),
            (LineSpacing::AtLeast(v), Some(g)) => (g.snap_height(natural.max(pictures)).max(v), true),
            (LineSpacing::Multiple(m), None) => ((text_natural * m + label_extra).max(text_pictures), false),
            (LineSpacing::AtLeast(v), None) => (natural.max(text_pictures).max(v), false),
            (LineSpacing::Exactly(v), _) => (v, false),
        };
        let baseline = if centred {
            let up = asc.max(obj_asc);
            self.top + ((height - up - desc) / 2.0).max(0.0) + up
        } else {
            self.top + baseline_offset(rp.line_spacing, asc, height, desc).max(obj_asc.min(height - desc))
        };
        Measure { height, baseline, text_asc, text_desc }
    }

    /// Alignment / justification of the line (trailing spaces hang).
    fn align(&mut self, pl: &mut ParaLayout, sc: &mut Scan, c1: usize, text: &str) {
        let rp = &self.rp;
        let c0 = sc.c0;
        let mut content_end = content_end(pl, sc, c1);
        let last_tab = (c0..c1).rev().find(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == ClKind::Tab));
        // Word distributes the reserved right/decimal tab gap over the following text on
        // a justified wrapped line. Other tab kinds keep their stop unchanged.
        if rp.align == Align::Justify
            && sc.end == LineEnd::Wrap
            && let Some(tj) = last_tab
            && !pl.ptabs.iter().any(|p| p.0 == tj)
            && let Some(&origin) = sc.xs.get(tj - c0)
            && matches!(next_tab(origin, &rp.tabs, self.default_tab, self.hanging_at).align, TabAlign::Right | TabAlign::Decimal)
            && let Some(tab) = pl.clusters.get_mut(tj)
        {
            let released = tab.adv.max(0.0);
            tab.adv -= released;
            for x in sc.xs.iter_mut().skip(tj + 1 - c0) {
                *x -= released;
            }
            content_end -= released;
        }
        let slack = sc.right_edge - content_end;
        let align_from = last_tab.map(|k| k + 1).unwrap_or(c0);
        let display = pl.displays.iter().find(|(ci, _)| *ci >= c0 && *ci < c1).map(|d| d.1);
        let shift = match (display, rp.align) {
            (Some(MathJc::Left), _) => 0.0,
            (Some(MathJc::Right), _) => slack,
            (Some(_), _) => slack / 2.0,
            (None, Align::Center) => slack / 2.0,
            (None, Align::Right) => slack,
            _ => 0.0,
        };
        if shift > 0.0 && slack > 0.0 {
            for v in sc.xs.iter_mut().skip(align_from - c0) {
                *v += shift;
            }
            // After a tab the label stays, as the text before the tab does.
            if last_tab.is_none()
                && let Some(label_shift) = self.label_shifts.get_mut(self.lines.len())
            {
                *label_shift = shift;
            }
        }
        // A line ending in a manual line break is stretched too, unless `doNotExpandShiftReturn` says not to.
        let stretch_end = sc.end == LineEnd::Wrap || (sc.end == LineEnd::LineBreak && self.expand_break);
        // Without hanging punctuation Word leaves a line that used punctuation compression at
        // its compact width. With hanging enabled it still distributes the remaining space.
        let compressed_without_hanging = !rp.overflow_punct && sc.compressed.iter().any(|(index, _, _)| *index >= c0 && *index < c1);
        let justify =
            display.is_none() && !compressed_without_hanging && ((rp.align == Align::Justify && stretch_end) || rp.align == Align::Distribute);
        // A line that only fits with shrunk spaces is shrunk, the last line of the paragraph too.
        let shrink = display.is_none() && self.shrink_spaces && slack < 0.0 && last_tab.is_none();
        if shrink {
            shrink_line_spaces(pl, &mut sc.xs, c0, c1, content_end, slack);
        } else if justify && slack > 0.0 {
            justify_line(pl, &mut sc.xs, (c0, c1, align_from), (content_end, slack), rp.align == Align::Distribute, text);
        }
    }

    /// After a line: whether the paragraph is done. Else the next line starts, below the row
    /// when the line ends it.
    fn next_line(&mut self, pl: &ParaLayout, end: LineEnd) -> bool {
        // Text that wrapped continues on the row's far side; anything else ends the row.
        if end != LineEnd::Wrap || self.i >= self.n {
            self.row_rest.clear();
        }
        if self.row_rest.is_empty() {
            self.top += even_row(self.lines.get_mut(self.row_first..).unwrap_or_default());
        }
        if self.i >= self.n {
            // A paragraph ending with a line break gets an empty last line.
            if matches!(end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) {
                self.empty_last_line(pl);
            }
            return true;
        }
        self.lines.len() > 200_000
    }

    fn empty_last_line(&mut self, pl: &ParaLayout) {
        let rp = &self.rp;
        let grid = self.grid.as_ref();
        let (asc, desc) = pl.styles.get(self.mark_style as usize).map(|s| (s.ascent, s.descent)).unwrap_or((10.0, 3.0));
        let natural = asc + desc;
        let h = match (rp.line_spacing, grid) {
            (LineSpacing::Multiple(m), Some(g)) => g.snap_height(natural) * m,
            (LineSpacing::AtLeast(v), Some(g)) => g.snap_height(natural).max(v),
            (LineSpacing::Multiple(m), None) => natural * m,
            (LineSpacing::AtLeast(v), None) => natural.max(v),
            (LineSpacing::Exactly(v), _) => v,
        };
        let (width, x0) = (self.width, rp.indent_left);
        let (x0, left, right) = if self.rtl_para { (width - x0, width - self.base_right, width - x0) } else { (x0, x0, self.base_right) };
        self.lines.push(Line {
            top: self.top,
            height: h,
            baseline: self.top + if grid.is_some() { h - desc } else { baseline_offset(rp.line_spacing, asc, h, desc) },
            text_asc: asc,
            text_desc: desc,
            c0: self.n,
            c1: self.n,
            xs: vec![x0],
            leaders: Vec::new(),
            end: LineEnd::Para,
            start: pl.text_len,
            stop: pl.text_len,
            left,
            right,
            hyphen: None,
            beside: false,
            vis: Vec::new(),
            rtl: self.rtl_para,
        });
        self.top += h;
    }

    /// Hang the drop cap from the first line's caps down to line N's baseline, left of the text.
    fn hang_drop_cap(&mut self, pl: &mut ParaLayout) {
        let rp = &self.rp;
        let lines = &self.lines;
        let Some((nc, dl, dw)) = pl.drop_cap else { return };
        let (Some(l0), Some(ln)) = (lines.first(), lines.get((dl as usize).min(lines.len()).saturating_sub(1))) else { return };
        let short = lines.len() < dl as usize;
        let dy = if short { (dl as f32 - 1.0) * l0.height } else { ln.baseline - l0.baseline };
        let lead = self.first_x0.unwrap_or(0.0) - self.first_left.max(rp.indent_left);
        // In margin, the letter's right edge stays the distance away from the text's edge.
        let dx = if rp.drop_cap_margin { lead.max(0.0) + pl.drop_cap_w + rp.drop_cap_distance } else { dw.min(lead) };
        // Toward the start edge: left, or right (and from the letter's right edge) when mirrored.
        let dx = if self.rtl_para { pl.drop_cap_w - dx } else { dx };
        if short {
            self.top = self.top.max(l0.top + l0.height * dl as f32);
        }
        for c in pl.clusters.get(..nc).unwrap_or(&[]).to_vec() {
            for g in pl.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dy -= dy;
                g.dx -= dx;
            }
        }
    }

    /// Store the lines and the paragraph's height, and move the label into place.
    fn finish(mut self, pl: &mut ParaLayout) {
        self.hang_drop_cap(pl);
        pl.height = self.top;
        pl.lines = std::mem::take(&mut self.lines);
        let label_shift = self.label_shifts.get(pl.label_line()).copied().unwrap_or(0.0);
        if let Some(lab) = pl.label.as_mut() {
            lab.x += label_shift;
            if self.rtl_para {
                lab.x = self.width - lab.x - lab.width;
            }
        }
    }
}

/// A line's ascent and descent over the baseline, and the tallest picture on it.
struct Extents {
    asc: f32,
    desc: f32,
    obj_asc: f32,
}

/// A line's height and baseline, and the extent of its text.
struct Measure {
    height: f32,
    baseline: f32,
    text_asc: f32,
    text_desc: f32,
}

/// Where the line's content ends: trailing spaces and markers hang.
fn content_end(pl: &ParaLayout, sc: &Scan, c1: usize) -> f32 {
    let mut content_end = sc.xs.last().copied().unwrap_or(sc.line_start_x);
    for k in (sc.c0..c1).rev() {
        match pl.clusters.get(k).map(|c| c.kind) {
            Some(ClKind::Space) | Some(ClKind::Marker) | Some(ClKind::LineBreak) | Some(ClKind::PageBreak) | Some(ClKind::ColumnBreak) => {
                content_end = sc.xs.get(k - sc.c0).copied().unwrap_or(content_end);
            }
            _ => {
                // The automatic space after the line's last character goes with the wrap.
                content_end -= pl.auto_space_after(k);
                break;
            }
        }
    }
    content_end
}

/// Stretch spaces and legal East Asian character boundaries to the right edge. Latin words
/// remain whole; distributed alignment instead stretches every cluster boundary.
fn justify_line(
    pl: &ParaLayout,
    xs: &mut [f32],
    (c0, c1, align_from): (usize, usize, usize),
    (content_end, slack): (f32, f32),
    distribute: bool,
    text: &str,
) {
    if distribute {
        let cnt = (c1 - align_from).saturating_sub(1).max(1) as f32;
        let per = slack / cnt;
        for (q, v) in xs.iter_mut().enumerate().skip(align_from - c0) {
            let k = (q + c0 - align_from) as f32;
            *v += per * k.min(cnt);
        }
        return;
    }
    let mut gaps = Vec::new();
    let mut previous: Option<(usize, char)> = None;
    for k in align_from..c1 {
        let Some(c) = pl.clusters.get(k) else { break };
        match c.kind {
            ClKind::Space => {
                // Trailing spaces hang outside justification, as they do outside wrapping.
                if xs.get(k - c0).copied().unwrap_or(f32::MAX) < content_end - 0.01 {
                    gaps.push(k);
                }
                previous = None;
            }
            ClKind::Text => {
                let shown = pl.shown.binary_search_by_key(&k, |s| s.0).ok().and_then(|i| pl.shown.get(i)).map(|s| s.1.as_str());
                let shown = shown.or_else(|| text.get(c.start..c.end)).unwrap_or("");
                let (Some(first), Some(last)) = (shown.chars().next(), shown.chars().next_back()) else {
                    previous = None;
                    continue;
                };
                if let Some((pk, last)) = previous
                    && (crate::kinsoku::is_east_asian(last) || crate::kinsoku::is_east_asian(first))
                    && pl.clusters.get(pk).is_some_and(|p| p.break_after)
                {
                    // Reuse the paragraph's break rules: no gap after an opening bracket,
                    // before closing punctuation, or inside a Latin word / atomic cluster.
                    gaps.push(pk);
                }
                previous = Some((k, last));
            }
            ClKind::Marker if c.adv == 0.0 => {}
            _ => previous = None,
        }
    }
    if gaps.is_empty() {
        return;
    }
    let per = slack / gaps.len() as f32;
    let mut add = 0.0;
    let mut si = 0;
    for k in align_from..c1 {
        if let Some(v) = xs.get_mut(k - c0) {
            *v += add;
        }
        if gaps.get(si) == Some(&k) {
            add += per;
            si += 1;
        }
    }
    if let Some(v) = xs.last_mut() {
        *v += add;
    }
}

impl Scan {
    /// Put clusters on the line until it is full or ends.
    fn scan(&mut self, bk: &mut Breaker, pl: &mut ParaLayout) {
        while self.j < bk.n && !self.step(bk, pl) {}
    }

    /// Put the cluster at `j` on the line. True when the line ends here.
    fn step(&mut self, bk: &mut Breaker, pl: &mut ParaLayout) -> bool {
        let Some(c) = pl.clusters.get(self.j).cloned() else { return true };
        match c.kind {
            ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak => {
                self.finish_pending_tab(pl);
                self.xs.push(self.x);
                self.end = match c.kind {
                    ClKind::LineBreak => LineEnd::LineBreak,
                    ClKind::PageBreak => LineEnd::PageBreak,
                    _ => LineEnd::ColumnBreak,
                };
                self.j += 1;
                true
            }
            ClKind::Tab => self.tab(bk, pl),
            _ => self.cluster(bk, pl, &c),
        }
    }

    /// A tab: its stop, and the room to it. True when the line ends before it.
    fn tab(&mut self, bk: &mut Breaker, pl: &mut ParaLayout) -> bool {
        let j = self.j;
        self.has_tab = true;
        self.finish_pending_tab(pl);
        let rp = &bk.rp;
        let ptab = pl.ptabs.iter().find(|t| t.0 == j).map(|&(_, a, b, l)| ptab_stop(a, b, l, bk.width, (rp.indent_left, rp.indent_right)));
        let stop = ptab.unwrap_or_else(|| next_tab(self.x, &rp.tabs, bk.default_tab, bk.hanging_at));
        // A positional tab whose place the text has passed goes there on the next line
        // (ECMA-376 §17.3.3.23); a line with nothing but markers on it is empty.
        if ptab.is_some() && stop.pos < self.x - 0.01 && has_content(pl, self.c0, j) {
            self.end = LineEnd::Wrap;
            return true;
        }
        self.xs.push(self.x);
        if ptab.is_none() && stop.pos > self.right_edge + 0.01 && stop.align == TabAlign::Left {
            // Not when only markers (a bookmark, say) are before it: the line is empty.
            if has_content(pl, self.c0, j) {
                // Tab past the right indent wraps: like Word, it starts the next line
                // and goes to the first stop there (a TOC entry's leader and page number).
                self.xs.pop();
                self.end = LineEnd::Wrap;
                bk.wrapped_tab = Some(j);
                return true;
            }
            if bk.wrapped_tab == Some(j) {
                // Still past the right indent on the line it wrapped to: it takes no
                // room, and its text starts at the indent rather than on a line of its own.
                if let Some(c) = pl.clusters.get_mut(j) {
                    c.adv = 0.0;
                }
                self.last_break = Some(j);
                self.j += 1;
                return false;
            }
        }
        self.place_tab(pl, stop);
        self.last_break = Some(j);
        self.j += 1;
        false
    }

    /// Give the tab at `j` its room to `stop`, or wait for the text after it (a right, centre or
    /// decimal tab).
    fn place_tab(&mut self, pl: &mut ParaLayout, stop: TabStop) {
        let j = self.j;
        match stop.align {
            TabAlign::Left | TabAlign::Bar | TabAlign::Clear => {
                let w = (stop.pos - self.x).max(0.0);
                if let Some(c) = pl.clusters.get_mut(j) {
                    c.adv = w;
                }
                self.x += w;
            }
            _ => self.pending_tab = Some((j, stop, self.x)),
        }
        if stop.leader != TabLeader::None {
            self.leaders.push((j, stop.leader));
        }
    }

    /// Width reserved for a pending tab after adding `advance`, including provisional
    /// compression. Fitting and placement must use the same decimal anchor and tab width.
    fn pending_tab_width(&self, pl: &ParaLayout, advance: f32, extra: Option<(usize, f32)>) -> Option<(usize, f32, f32)> {
        let (tj, stop, tx) = self.pending_tab?;
        let old = pl.clusters.get(tj).map_or(0.0, |c| c.adv);
        let seg = self.x + advance - tx - old;
        let width = match stop.align {
            TabAlign::Right => stop.pos - tx - seg,
            TabAlign::Center => stop.pos - tx - seg / 2.0,
            TabAlign::Decimal => {
                let width = decimal_position(pl, tj, self.j)
                    .map(|(dot, width)| width - decimal_compression(&self.compressed, tj, Some(dot), extra))
                    .unwrap_or(seg);
                stop.pos - tx - width
            }
            _ => return None,
        };
        Some((tj, old, width.max(0.0)))
    }

    /// Resolve a pending tab virtually, including provisional line-end compression.
    fn predicted_end(&self, pl: &ParaLayout, advance: f32, extra: Option<(usize, f32)>) -> f32 {
        match self.pending_tab_width(pl, advance, extra) {
            Some((_, old, width)) => self.x + advance + width - old,
            None => self.x + advance,
        }
    }

    /// A text or object cluster. True when the line ends before or at it.
    fn cluster(&mut self, bk: &mut Breaker, pl: &mut ParaLayout, c: &Cluster) -> bool {
        let (j, c0) = (self.j, self.c0);
        // A display equation sits on a line of its own.
        let is_display = pl.displays.iter().any(|(ci, _)| *ci == j);
        if is_display && has_content(pl, c0, j) {
            self.end = LineEnd::Wrap;
            return true;
        }
        // A grapheme can have several drawing segments; fit it as one unit while keeping
        // each segment's font, glyphs and position for painting.
        let advance = pl.atomic_advance(j);
        let shrink = if bk.shrink_spaces && !self.has_tab {
            space_room(self.space_w, self.x + advance - pl.auto_space_after(j) - self.word_from)
        } else {
            0.0
        };
        // A positional tab can target the page margin beyond the paragraph indent.
        let edge = self
            .pending_tab
            .filter(|(tj, _, _)| pl.ptabs.iter().any(|p| p.0 == *tj))
            .map_or(self.right_edge, |(_, stop, _)| self.right_edge.max(stop.pos));
        let limit = edge + shrink + pl.auto_space_after(j);
        let end = self.predicted_end(pl, advance, None);
        // A fit at the compression limit needs only floating-point rounding tolerance;
        // a hundredth of a point would admit a measurably wider glyph at small sizes.
        let fit_epsilon = f32::EPSILON * limit.abs().max(1.0) * 8.0;
        let candidate = bk.punctuation_room.get(j).copied().unwrap_or_default();
        let compressed = if !candidate.opening
            && candidate.room > 0.0
            && end > limit + 0.01
            && self.predicted_end(pl, advance - candidate.room, Some((j, candidate.room))) <= limit + fit_epsilon
        {
            candidate.room
        } else {
            0.0
        };
        // A preceding mark can release its bearing when the following character fits.
        if compressed == 0.0 && end > limit + 0.01 && j > c0 && c.kind == ClKind::Text {
            let previous = bk.punctuation_room.get(j - 1).copied().unwrap_or_default();
            if previous.room > 0.0
                && !self.compressed.iter().any(|(k, _, _)| *k == j - 1)
                && self.predicted_end(pl, advance - previous.room, Some((j - 1, previous.room))) <= limit + fit_epsilon
            {
                self.x -= previous.room;
                self.compressed.push((j - 1, previous.room, previous.ink_shift));
            }
        }
        let fits = self.predicted_end(pl, advance, None) <= limit + 0.01
            || compressed > 0.0
            // Only the punctuation itself may overflow: its starting pen must still
            // be inside the text extent. A second character cannot hang after it.
            || (!self.has_tab && self.x <= self.right_edge + 0.01 && bk.hanging_punctuation.get(j) == Some(&true))
            || c.kind == ClKind::Space
            || c.kind == ClKind::Marker;
        if !fits && pl.is_edit_boundary(c.start) && (c0..j).any(|k| pl.clusters.get(k).is_some_and(takes_room)) {
            self.wrap(bk, pl);
            return true;
        }
        self.xs.push(self.x);
        self.x += c.adv - compressed;
        if compressed > 0.0 {
            self.compressed.push((j, compressed, candidate.ink_shift));
        }
        if c.kind == ClKind::Space {
            self.space_w += c.adv;
        }
        self.shift_pending_tab(pl);
        if is_display {
            self.end_display(pl, bk.n);
            return true;
        }
        if c.break_after {
            self.note_break(bk, pl, c, shrink);
        }
        self.j += 1;
        false
    }

    /// End a tab segment using the scan's still-provisional compressed advances.
    /// Line finalization resolves committed advances separately, without applying them twice.
    fn finish_pending_tab(&mut self, pl: &mut ParaLayout) {
        self.shift_pending_tab(pl);
        self.pending_tab = None;
    }

    /// Text after a decimal/centre/right tab shifts as it grows.
    fn shift_pending_tab(&mut self, pl: &mut ParaLayout) {
        let Some((tj, old, shift)) = self.pending_tab_width(pl, 0.0, None) else { return };
        if (shift - old).abs() > 0.001 {
            let delta = shift - old;
            if let Some(c) = pl.clusters.get_mut(tj) {
                c.adv = shift;
            }
            let from = tj + 1 - self.c0;
            for v in self.xs.iter_mut().skip(from) {
                *v += delta;
            }
            self.x += delta;
        }
    }

    /// A display equation ends its line: markers right after it stay on its line; anything else
    /// starts a new one.
    fn end_display(&mut self, pl: &ParaLayout, n: usize) {
        self.j += 1;
        while self.j < n && pl.clusters.get(self.j).is_some_and(|c| c.kind == ClKind::Marker) {
            self.xs.push(self.x);
            self.j += 1;
        }
        self.end = if self.j >= n { LineEnd::Para } else { LineEnd::Wrap };
    }

    /// The cluster (already on the line) is followed by a break opportunity, or a hyphenation point.
    fn note_break(&mut self, bk: &mut Breaker, pl: &ParaLayout, c: &Cluster, shrink: f32) {
        let j = self.j;
        if pl.hyph_after.binary_search(&(j as u32)).is_err() {
            self.last_break = Some(j);
            self.last_plain = Some((j, self.x));
            self.word_from = if c.kind == ClKind::Space { self.x - c.adv } else { self.x };
        } else if self.hyph_ok
            && !pl.clusters.get(j + 1).is_some_and(|n| n.kind == ClKind::Text && n.g1 <= n.g0)
            && self.x + hyphen_glyph(pl, c.style, &mut bk.hcache).2 <= self.right_edge + shrink + 0.01
        {
            self.last_break = Some(j);
        }
    }

    /// The cluster at `j` doesn't fit: wrap, backing up to the last break opportunity on the line.
    fn wrap(&mut self, bk: &mut Breaker, pl: &ParaLayout) {
        let c0 = self.c0;
        self.end = LineEnd::Wrap;
        // Hyphenate only when the plain break leaves more than the hyphenation zone empty.
        if let (Some(bk_i), Some((pj, px))) = (self.last_break, self.last_plain)
            && pj >= c0
            && pj < bk_i
            && pl.hyph_after.binary_search(&(bk_i as u32)).is_ok()
            && self.right_edge - px <= bk.zone
        {
            self.last_break = Some(pj);
        }
        self.tab_with_its_text(pl);
        if let Some(b) = self.last_break.filter(|b| *b >= c0) {
            self.xs.truncate(b + 1 - c0);
            self.j = b + 1;
            // Recompute x for the kept clusters.
            let compressed = self.compressed.iter().find(|(k, _, _)| *k == b).map_or(0.0, |(_, amount, _)| *amount);
            self.x = self.xs.last().copied().unwrap_or(self.line_start_x) + pl.clusters.get(b).map(|c| c.adv).unwrap_or(0.0) - compressed;
        } else {
            self.break_long_word(bk, pl);
        }
    }

    /// The text after a right, centre or decimal tab doesn't fit: like Word, the tab goes to the
    /// next line with its text (a TOC entry's page number stays at the right margin, after a
    /// leader) instead of ending this line.
    fn tab_with_its_text(&mut self, pl: &ParaLayout) {
        if let Some((tj, ..)) = self.pending_tab
            && self.last_break == Some(tj)
            && has_content(pl, self.c0, tj)
        {
            self.last_break = Some(tj - 1);
            self.pending_tab = None;
            self.leaders.retain(|(k, _)| *k < tj);
        }
    }

    /// No opportunity to break: break here (a long word), unless the span is beside a floating
    /// object and the word fits the full line; then it tries the row's next span, or the next row.
    /// The word runs to the next break that isn't a hyphenation point.
    fn break_long_word(&mut self, bk: &Breaker, pl: &ParaLayout) {
        let (c0, j) = (self.c0, self.j);
        let full = bk.base_right - self.row_lo;
        self.skip = self.narrowed && bk.skipped < 50 && bk.widths.get(c0).is_some_and(|w| *w <= full + 0.01);
        // Never split a ligature's letters or a grapheme's drawing segments across
        // lines: back up to their first segment (unless it starts the line).
        let glyphless = |k: usize| pl.clusters.get(k).is_some_and(|c| c.kind == ClKind::Text && c.g1 <= c.g0);
        let mut b = j;
        while b > c0 + 1 && (glyphless(b) || pl.clusters.get(b).is_some_and(|c| !pl.is_edit_boundary(c.start))) {
            b -= 1;
        }
        while b > c0 && pl.clusters.get(b).is_some_and(|c| !pl.is_edit_boundary(c.start)) {
            b -= 1;
        }
        if b < j
            && b > c0
            && !self.skip
            && let Some(bx) = self.xs.get(b - c0).copied()
        {
            self.xs.truncate(b - c0);
            self.x = bx;
            self.j = b;
        }
    }
}

/// Narrowest span text flows into beside a floating object, points. Word puts an empty
/// paragraph beside a floating table in a 21.3pt gap but not in an 18.6pt one.
const MIN_SPAN: f32 = 20.0;

/// How far below a line's top its baseline sits. Multiple spacing at or above single keeps the
/// text at the top of the line and puts the extra leading below the baseline; below single it
/// shrinks the whole box, ascent included (Word's output for docxide-pdf's fixtures: 1.15-spaced
/// Times New Roman 12pt sits one ascent below the line top, not 1.8pt lower). Exact and at-least
/// spacing keep the descent below the baseline and the rest above it.
fn baseline_offset(spacing: LineSpacing, asc: f32, height: f32, desc: f32) -> f32 {
    match spacing {
        LineSpacing::Multiple(m) => asc * m.min(1.0),
        LineSpacing::AtLeast(_) | LineSpacing::Exactly(_) => height - desc,
    }
}

/// The spans of a row at `top` (about `h` tall) between `lo` and `hi` that text can use around
/// the exclusions, left to right (Square wrapping uses both sides of an object). `Err(y)`: none
/// here, try again at `y` (below what's in the way).
fn row_spans(exclusions: &[Exclusion], top: f32, h: f32, table_h: f32, lo: f32, hi: f32) -> Result<VecDeque<(f32, f32)>, f32> {
    let here: Vec<&Exclusion> = exclusions.iter().filter(|e| e.intersects_row(top, h, table_h, lo, hi)).collect();
    if let Some(below) = here.iter().filter(|e| e.top_bottom).map(|e| e.bottom).reduce(f32::max) {
        return Err(below);
    }
    // What each object takes of the row: its whole area, or (Tight, Through) the stretches its
    // outline takes, as (left, right, bottom of the object's area).
    let mut taken: Vec<(f32, f32, f32)> = Vec::new();
    for e in &here {
        let first = taken.len();
        match &e.contour {
            None => taken.push((e.left, e.right, e.bottom)),
            Some(c) => taken.extend(c.occupied(top - e.top, top + h - e.top).into_iter().map(|(a, b)| (a + e.left, b + e.left, e.bottom))),
        }
        // Text on one side only: the object's other side of the row is taken too.
        let Some(mine) = taken.get(first..).filter(|t| !t.is_empty() && !e.side.is_both()) else { continue };
        let left = mine.iter().map(|t| t.0).fold(f32::INFINITY, f32::min);
        let right = mine.iter().map(|t| t.1).fold(f32::NEG_INFINITY, f32::max);
        let text_left = match e.side {
            WrapText::Left => true,
            WrapText::Right => false,
            // The side with more room on this line (left on a tie).
            _ => left - lo >= hi - right,
        };
        taken.truncate(first);
        taken.push(if text_left { (left, hi.max(right), e.bottom) } else { (lo.min(left), right, e.bottom) });
    }
    taken.retain(|(l, r, _)| *r > lo && *l < hi);
    if taken.is_empty() {
        return Ok(VecDeque::from([(lo, hi)]));
    }
    let mut spans = vec![(lo, hi)];
    for &(left, right, _) in &taken {
        spans = spans
            .into_iter()
            .flat_map(|(a, b)| if right <= a || left >= b { vec![(a, b)] } else { vec![(a, left.min(b)), (right.max(a), b)] })
            .collect();
    }
    spans.retain(|(a, b)| b - a >= MIN_SPAN);
    if spans.is_empty() {
        // Nothing wide enough: the line goes below the first object that ends.
        return Err(taken.iter().map(|t| t.2).reduce(f32::min).unwrap_or(top));
    }
    Ok(spans.into())
}

#[cfg(test)]
mod kana_glyph_tests {
    use super::{decimal_compression, fullwidth_kana_glyph};

    #[test]
    fn decimal_tab_ignores_compression_from_an_earlier_segment() {
        // The comma at index 1 precedes the tab at 3; only index 4 precedes the
        // decimal point in this tab's own segment. A candidate at 6 is after it.
        let compressed = [(1, 4.0, 0.0), (4, 1.0, 0.0), (6, 2.0, 0.0)];
        assert_eq!(decimal_compression(&compressed, 3, Some(5), None), 1.0);
        assert_eq!(decimal_compression(&compressed, 3, Some(5), Some((1, 3.0))), 1.0);
        assert_eq!(decimal_compression(&compressed, 3, Some(5), Some((4, 0.5))), 1.5);
        assert_eq!(decimal_compression(&compressed, 3, None, None), 0.0);
    }

    #[test]
    fn pending_tabs_keep_provisional_compression_at_segment_endings() {
        use super::super::{Grid, ParaEnv, layout_para};
        use super::{Breaker, TabAlign, TabStop};
        use wordcraft_doc::para::{COLUMN_BREAK, LINE_BREAK, PAGE_BREAK};
        use wordcraft_doc::{Document, Paragraph};

        let doc = Document::new();
        let fields = Default::default();
        let env = ParaEnv {
            doc: &doc,
            fields: &fields,
            width: 300.0,
            label: None,
            show_hidden: false,
            hide_deleted: Default::default(),
            table: None,
            proofing: None,
            typing_word: None,
            field_codes: false,
            grid: Grid::default(),
            exclusions: &[],
            eq_number: 0,
            numeral: Default::default(),
            read_line_spacing: None,
            fit_objects: false,
            fitted: &[],
        };
        for align in [TabAlign::Decimal, TabAlign::Right, TabAlign::Center] {
            for ending in [Some('\t'), Some(LINE_BREAK), Some(PAGE_BREAK), Some(COLUMN_BREAK), None] {
                let text = ending.map_or_else(|| "\tA.B".to_string(), |ch| format!("\tA.B{ch}X"));
                let mut p = Paragraph::with_text(&text, Default::default());
                p.props.tabs = Some(vec![TabStop { pos: 160.0, align: TabAlign::Left, leader: Default::default() }]);
                let mut pl = layout_para(&p, &env);
                let mark_style = pl.clusters[0].style;
                let mut bk = Breaker::new(&pl, (&env, &[]), mark_style, None);
                let mut sc = bk.begin_line(&mut pl);
                // Synthetic advances avoid depending on any installed CJK font. The scan
                // has removed 4pt from A, while its stored advance remains uncommitted.
                let tab_width = match align {
                    TabAlign::Decimal => 94.0,
                    TabAlign::Right => 87.0,
                    _ => 93.5,
                };
                pl.clusters[0].adv = tab_width;
                pl.clusters[1].adv = 10.0;
                pl.clusters[2].adv = 2.0;
                pl.clusters[3].adv = 5.0;
                sc.pending_tab = Some((0, TabStop { pos: 100.0, align, leader: Default::default() }, 0.0));
                sc.compressed = vec![(1, 4.0, 0.0)];
                sc.has_tab = true;
                sc.xs = vec![0.0, tab_width, tab_width + 6.0, tab_width + 8.0];
                sc.x = tab_width + 13.0;
                sc.j = 4;
                if ending.is_some() {
                    sc.step(&mut bk, &mut pl);
                    assert!((sc.xs[2] - tab_width - 6.0).abs() < 0.001, "{align:?} {ending:?}: separator moved at segment end");
                    assert!(sc.pending_tab.is_none(), "the completed tab must not be resolved again");
                }
                bk.close_line(&mut pl, sc, &text);
                assert!((pl.clusters[0].adv - tab_width).abs() < 0.001, "{align:?} {ending:?}: tab advance changed");
                assert!((pl.clusters[1].adv - 6.0).abs() < 0.001, "compression must be committed exactly once");
                assert!((bk.lines[0].xs[2] - tab_width - 6.0).abs() < 0.001, "{align:?} {ending:?}: final separator moved");
            }
        }
    }

    #[test]
    fn missing_glyph_with_fullwidth_advance_is_not_compressed() {
        let missing = wordcraft_fonts::ShapedGlyph { gid: 0, cluster: 0, x_advance: 1000, x_offset: 0, y_offset: 0 };
        assert!(!fullwidth_kana_glyph(&[missing], 1000.0));
        assert!(fullwidth_kana_glyph(&[wordcraft_fonts::ShapedGlyph { gid: 1, ..missing }], 1000.0));
    }
}
