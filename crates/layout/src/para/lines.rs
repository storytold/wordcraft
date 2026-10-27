use super::bidi::{even_row, reorder_line};
use super::grid::Exclusion;
use super::justify::{HYPHENATION_ZONE, hyphen_glyph, shrink_line_spaces, space_room};
use super::model::{ClKind, Cluster, Line, LineEnd, ParaEnv};
use super::paragraph::word_widths;
use super::tabs::{next_tab, pl_text_at, ptab_stop, resolve_tab};
use super::*;

/// Break a paragraph into lines, measure each line, apply the paragraph's alignment and place
/// the list label. Fills `pl.lines`, `pl.height` and the label's position.
pub(super) fn break_lines(pl: &mut ParaLayout, env: &ParaEnv, mark_style: u32, level: Option<&Level>) {
    // Narrow cells and float spans must retain their usable width.
    const MIN_LINE_WIDTH: f32 = 1.0;
    let suffix = level.map(|l| l.suffix);
    let rp = pl.rp.clone();
    let width = env.width.max(MIN_LINE_WIDTH);
    let base_right = (width - rp.indent_right).max(1.0);
    let default_tab = env.doc.settings.default_tab;
    let first_left = rp.indent_left + rp.indent_first;
    // Word 2013+ (compatibility mode 15) fits more text on a justified line by shrinking its
    // spaces (`space_room`); never on lines with tabs.
    let shrink_spaces = rp.align == Align::Justify && env.doc.settings.compat_mode >= wordcraft_doc::COMPAT_MODE_CURRENT;
    // Word stretches a justified line ending in a manual line break unless the document says not to.
    let expand_break = !env.doc.settings.do_not_expand_shift_return;
    let hanging_at = if rp.indent_first < 0.0 { Some(rp.indent_left) } else { None };
    // Hyphenation Options: the zone, and how many lines in a row may end with a hyphen.
    let zone = Some(env.doc.settings.hyphenation_zone).filter(|z| z.is_finite() && *z >= 0.0).unwrap_or(HYPHENATION_ZONE).min(1584.0);
    let hyph_limit = env.doc.settings.consecutive_hyphen_limit as usize;
    let n = pl.clusters.len();
    // The width of every word, measured once: the word beside a float is measured again for
    // every row it is tried on, and measuring it again costs a lookup rather than another pass
    // over the word (#18).
    let widths = word_widths(&pl.clusters, &pl.hyph_after);
    // A right-to-left paragraph is laid out from its start edge, as if mirrored: indents, tabs,
    // the list label and alignment are measured from the right. Lines are mirrored back (and
    // bidirectional text reordered) once broken; floating objects are mirrored in.
    let rtl_para = rp.bidi;
    // The document grid's line pitch, for paragraphs that snap to it.
    let grid = Some(env.grid).filter(|g| g.line.is_some() && rp.snap_to_grid);
    let grid = grid.as_ref();
    let bidi = rtl_para || !pl.bidi_levels.is_empty();
    let mirrored: Vec<Exclusion>;
    let exclusions: &[Exclusion] = if rtl_para {
        mirrored = env
            .exclusions
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
            .collect();
        &mirrored
    } else {
        env.exclusions
    };
    let mut first_x0: Option<f32> = None;
    let mut hcache: Vec<(u32, u32, f32)> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    // The label is drawn on line 0 or, after an opening page break, line 1. Apply that
    // line's alignment once both lines are known, before mirroring right-to-left labels.
    let mut label_shifts = [0.0; 2];
    let mut i = 0usize;
    let mut top = 0.0f32;
    let mut first = true;
    let est_h = pl
        .styles
        .get(mark_style as usize)
        .map(|st| (st.ascent + st.descent) * if let LineSpacing::Multiple(m) = rp.line_spacing { m } else { 1.0 })
        .unwrap_or(14.0)
        .max(1.0);
    // Exact spacing fixes the table-contact boundary even when the mark has other metrics.
    // Keep the existing estimate for shapes and for non-exact spacing.
    let table_h = if let LineSpacing::Exactly(h) = rp.line_spacing { h } else { est_h };
    // The rest of the current row: spans on the far side of floating objects, left to right.
    let mut row_rest: VecDeque<(f32, f32)> = VecDeque::new();
    let mut row_first = 0usize;
    // Spans skipped in a row because the next word didn't fit (see `skip`).
    let mut skipped = 0;
    // A left tab past the right indent that wrapped to start the next line.
    let mut wrapped_tab: Option<usize> = None;
    loop {
        // A line already sits on this row (a span before this one may have been skipped).
        let beside = !row_rest.is_empty() && lines.len() > row_first;
        // The span is narrowed by a floating object.
        let mut narrowed = !row_rest.is_empty();
        let row_lo = if first { first_left } else { rp.indent_left };
        let (left, right_edge) = if let Some(span) = row_rest.pop_front() {
            span
        } else {
            // Flow around floating objects: the spans beside them, or below them.
            let mut guard = 0;
            loop {
                let mut lo = if first { first_left } else { rp.indent_left };
                if let Some((_, dl, dw)) = pl.drop_cap
                    && dw > 0.0
                    && lines.len() < dl as usize
                {
                    lo = first_left.max(rp.indent_left) + dw;
                }
                match row_spans(exclusions, top, est_h, table_h, lo, base_right).map(|mut spans| (spans.pop_front(), spans)) {
                    Ok((Some((a, b)), rest)) => {
                        narrowed = !rest.is_empty() || a > lo || b < base_right;
                        row_rest = rest;
                        row_first = lines.len();
                        break (a, b.max(a + MIN_LINE_WIDTH));
                    }
                    Err(below) if below > top && guard < 50 => {
                        top = below;
                        guard += 1;
                    }
                    Ok((None, _)) | Err(_) => {
                        row_first = lines.len();
                        break (lo, base_right.max(lo + MIN_LINE_WIDTH));
                    }
                }
            }
        };
        let mut x = left;
        // Label on the first line.
        if first && let Some(lab) = pl.label.as_mut() {
            // The number is aligned at the first-line indent: its left edge, centre or right edge.
            lab.x = match level.map(|l| l.align) {
                Some(Align::Center) => left - lab.width / 2.0,
                Some(Align::Right) => left - lab.width,
                _ => left,
            };
            let end = lab.x + lab.width;
            x = match suffix.unwrap_or(LevelSuffix::Tab) {
                LevelSuffix::Tab => {
                    let t = next_tab(end, &rp.tabs, default_tab, hanging_at).pos;
                    // The level's own tab stop after the number, when it comes first.
                    match level.and_then(|l| l.tab).filter(|p| p.is_finite() && *p > end + 0.01) {
                        Some(p) if p < t => p,
                        _ => t,
                    }
                }
                LevelSuffix::Space => end + pl.styles.get(lab.style as usize).map(|s| s.size * 0.25).unwrap_or(3.0),
                LevelSuffix::Nothing => end,
            };
        }
        let line_start_x = x;
        let c0 = i;
        // The first word doesn't fit a span narrowed by a floating object: it moves on whole.
        let mut skip = false;
        let mut xs: Vec<f32> = Vec::new();
        let mut leaders = Vec::new();
        let mut last_break: Option<usize> = None; // cluster index after which we may break
        let mut last_plain: Option<(usize, f32)> = None; // last non-hyphen break and the x after it
        let mut word_from = left; // where the space before the current word starts
        // After `hyph_limit` hyphenated lines in a row this one may not end with a hyphen.
        let hyph_ok = hyph_limit == 0 || lines.iter().rev().take_while(|l| l.hyphen.is_some()).count() < hyph_limit;
        let mut end = LineEnd::Para;
        let mut j = i;
        let mut pending_tab: Option<(usize, TabStop, f32)> = None; // tab cluster, stop, x where tab started
        let mut space_w = 0.0f32; // width of the spaces so far on this line
        let mut has_tab = false;
        while j < n {
            let Some(c) = pl.clusters.get(j).cloned() else { break };
            match c.kind {
                ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak => {
                    resolve_tab(pl, &mut pending_tab, &mut xs, c0, x, &mut x);
                    xs.push(x);
                    end = match c.kind {
                        ClKind::LineBreak => LineEnd::LineBreak,
                        ClKind::PageBreak => LineEnd::PageBreak,
                        _ => LineEnd::ColumnBreak,
                    };
                    j += 1;
                    break;
                }
                ClKind::Tab => {
                    has_tab = true;
                    resolve_tab(pl, &mut pending_tab, &mut xs, c0, x, &mut x);
                    let ptab = pl.ptabs.iter().find(|t| t.0 == j).map(|&(_, a, b, l)| ptab_stop(a, b, l, width, (rp.indent_left, rp.indent_right)));
                    let stop = ptab.unwrap_or_else(|| next_tab(x, &rp.tabs, default_tab, hanging_at));
                    // A positional tab whose place the text has passed goes there on the next line
                    // (ECMA-376 §17.3.3.23); a line with nothing but markers on it is empty.
                    if ptab.is_some() && stop.pos < x - 0.01 && (c0..j).any(|k| pl.clusters.get(k).is_some_and(|c| c.kind != ClKind::Marker)) {
                        end = LineEnd::Wrap;
                        break;
                    }
                    xs.push(x);
                    if ptab.is_none() && stop.pos > right_edge + 0.01 && stop.align == TabAlign::Left {
                        // Not when only markers (a bookmark, say) are before it: the line is empty.
                        if (c0..j).any(|k| pl.clusters.get(k).is_some_and(|c| c.kind != ClKind::Marker)) {
                            // Tab past the right indent wraps: like Word, it starts the next line
                            // and goes to the first stop there (a TOC entry's leader and page number).
                            xs.pop();
                            end = LineEnd::Wrap;
                            wrapped_tab = Some(j);
                            break;
                        }
                        if wrapped_tab == Some(j) {
                            // Still past the right indent on the line it wrapped to: it takes no
                            // room, and its text starts at the indent rather than on a line of its own.
                            if let Some(c) = pl.clusters.get_mut(j) {
                                c.adv = 0.0;
                            }
                            last_break = Some(j);
                            j += 1;
                            continue;
                        }
                    }
                    match stop.align {
                        TabAlign::Left | TabAlign::Bar | TabAlign::Clear => {
                            let w = (stop.pos - x).max(0.0);
                            if let Some(c) = pl.clusters.get_mut(j) {
                                c.adv = w;
                            }
                            if stop.leader != TabLeader::None {
                                leaders.push((j, stop.leader));
                            }
                            x += w;
                        }
                        _ => {
                            pending_tab = Some((j, stop, x));
                            if stop.leader != TabLeader::None {
                                leaders.push((j, stop.leader));
                            }
                        }
                    }
                    last_break = Some(j);
                    j += 1;
                    continue;
                }
                _ => {}
            }
            // A display equation sits on a line of its own.
            let is_display = pl.displays.iter().any(|(ci, _)| *ci == j);
            if is_display && (c0..j).any(|k| pl.clusters.get(k).is_some_and(|c| c.kind != ClKind::Marker)) {
                end = LineEnd::Wrap;
                break;
            }
            // A grapheme can have several drawing segments; fit it as one unit while keeping
            // each segment's font, glyphs and position for painting.
            let advance = pl.atomic_advance(j);
            // Text after a right/centre tab grows leftwards into the tab's space first.
            let absorbs = pending_tab.is_some_and(|(tj, stop, _)| {
                let room = pl.clusters.get(tj).map(|t| t.adv).unwrap_or(0.0);
                match stop.align {
                    TabAlign::Right | TabAlign::Decimal => room >= advance,
                    TabAlign::Center => room >= advance / 2.0 && x + advance / 2.0 <= right_edge + 0.01,
                    _ => false,
                }
            });
            let shrink = if shrink_spaces && !has_tab { space_room(space_w, x + advance - pl.auto_space_after(j) - word_from) } else { 0.0 };
            let fits =
                absorbs || x + advance - pl.auto_space_after(j) <= right_edge + shrink + 0.01 || c.kind == ClKind::Space || c.kind == ClKind::Marker;
            // A line's first item stays on it even when it doesn't fit; markers before it
            // (a content control's start, a bookmark) and floating objects' anchors take no room
            // and don't count (Word: a picture a little wider than the line, after an anchor,
            // stays on the first line rather than leaving the anchor a line of its own).
            let takes_room = |c: &Cluster| c.kind != ClKind::Marker && !(matches!(c.kind, ClKind::Object(_)) && c.adv == 0.0 && c.obj_h == 0.0);
            if !fits && pl.is_edit_boundary(c.start) && (c0..j).any(|k| pl.clusters.get(k).is_some_and(takes_room)) {
                // Wrap: back up to the last break opportunity on this line.
                end = LineEnd::Wrap;
                // Hyphenate only when the plain break leaves more than the hyphenation zone empty.
                if let (Some(bk), Some((pj, px))) = (last_break, last_plain)
                    && pj >= c0
                    && pj < bk
                    && pl.hyph_after.binary_search(&(bk as u32)).is_ok()
                    && right_edge - px <= zone
                {
                    last_break = Some(pj);
                }
                // The text after a right, centre or decimal tab doesn't fit: like Word, the tab
                // goes to the next line with its text (a TOC entry's page number stays at the
                // right margin, after a leader) instead of ending this line.
                if let Some((tj, ..)) = pending_tab
                    && last_break == Some(tj)
                    && (c0..tj).any(|k| pl.clusters.get(k).is_some_and(|c| c.kind != ClKind::Marker))
                {
                    last_break = Some(tj - 1);
                    pending_tab = None;
                    leaders.retain(|(k, _)| *k < tj);
                }
                if let Some(bk) = last_break.filter(|bk| *bk >= c0) {
                    xs.truncate(bk + 1 - c0);
                    j = bk + 1;
                    // Recompute x for the kept clusters.
                    x = xs.last().copied().unwrap_or(line_start_x) + pl.clusters.get(bk).map(|c| c.adv).unwrap_or(0.0);
                } else {
                    // No opportunity: break here (a long word), unless the span is beside a
                    // floating object and the word fits the full line; then it tries the row's
                    // next span, or the next row. The word runs to the next break that isn't a
                    // hyphenation point.
                    let full = base_right - row_lo;
                    skip = narrowed && skipped < 50 && widths.get(c0).is_some_and(|w| *w <= full + 0.01);
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
                        && !skip
                        && let Some(bx) = xs.get(b - c0).copied()
                    {
                        xs.truncate(b - c0);
                        x = bx;
                        j = b;
                    }
                }
                break;
            }
            xs.push(x);
            x += c.adv;
            if c.kind == ClKind::Space {
                space_w += c.adv;
            }
            // Decimal/center/right tab: shift pending text as it grows.
            if let Some((tj, stop, tx)) = pending_tab {
                let seg_w = x - tx;
                let shift = match stop.align {
                    TabAlign::Right => stop.pos - tx - seg_w,
                    TabAlign::Center => stop.pos - tx - seg_w / 2.0,
                    TabAlign::Decimal => {
                        // Width up to the first '.' in the segment.
                        let mut w = 0.0;
                        let mut found = false;
                        for k in tj + 1..=j {
                            let Some(cc) = pl.clusters.get(k) else { break };
                            let s = pl_text_at(cc);
                            if found {
                                break;
                            }
                            if s {
                                found = true;
                                break;
                            }
                            w += cc.adv;
                        }
                        stop.pos - tx - if found { w } else { seg_w }
                    }
                    _ => 0.0,
                };
                let shift = shift.max(0.0);
                let old = pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
                if (shift - old).abs() > 0.001 {
                    let delta = shift - old;
                    if let Some(c) = pl.clusters.get_mut(tj) {
                        c.adv = shift;
                    }
                    let from = tj + 1 - c0;
                    for v in xs.iter_mut().skip(from) {
                        *v += delta;
                    }
                    x += delta;
                }
            }
            if is_display {
                // Markers right after it stay on its line; anything else starts a new one.
                j += 1;
                while j < n && pl.clusters.get(j).is_some_and(|c| c.kind == ClKind::Marker) {
                    xs.push(x);
                    j += 1;
                }
                end = if j >= n { LineEnd::Para } else { LineEnd::Wrap };
                break;
            }
            if c.break_after {
                let hyph = pl.hyph_after.binary_search(&(j as u32)).is_ok();
                if !hyph {
                    last_break = Some(j);
                    last_plain = Some((j, x));
                    word_from = if c.kind == ClKind::Space { x - c.adv } else { x };
                } else if hyph_ok
                    && !pl.clusters.get(j + 1).is_some_and(|n| n.kind == ClKind::Text && n.g1 <= n.g0)
                    && x + hyphen_glyph(pl, c.style, &mut hcache).2 <= right_edge + shrink + 0.01
                {
                    last_break = Some(j);
                }
            }
            j += 1;
        }
        if skip {
            skipped += 1;
            if row_rest.is_empty() {
                top = if lines.len() > row_first {
                    top + even_row(lines.get_mut(row_first..).unwrap_or_default())
                } else {
                    // Nothing on this row: go below the first object that ends, as when no span is wide enough.
                    exclusions
                        .iter()
                        .filter(|e| e.intersects_row(top, est_h, table_h, row_lo, base_right))
                        .map(|e| e.bottom)
                        .reduce(f32::min)
                        .unwrap_or(top + est_h)
                };
            }
            continue;
        }
        if end == LineEnd::Para || end == LineEnd::Wrap {
            let xc = x;
            resolve_tab(pl, &mut pending_tab, &mut xs, c0, xc, &mut x);
        }
        if j >= n && end != LineEnd::Wrap && !matches!(end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) {
            end = LineEnd::Para;
        }
        let c1 = j.max(c0);
        // Ensure progress.
        let (c1, j) = if c1 == c0 && c0 < n { (c0 + 1, c0 + 1) } else { (c1, j) };
        while xs.len() < c1 - c0 {
            let last = xs.last().copied().unwrap_or(line_start_x);
            let adv = pl.clusters.get(c0 + xs.len().saturating_sub(1)).map(|c| c.adv).unwrap_or(0.0);
            xs.push(if xs.is_empty() { line_start_x } else { last + adv });
        }
        xs.truncate(c1 - c0);
        // A line wrapped at a hyphenation point shows a hyphen after its last cluster.
        let hyphen = (end == LineEnd::Wrap && c1 > c0 && pl.hyph_after.binary_search(&((c1 - 1) as u32)).is_ok())
            .then(|| pl.clusters.get(c1 - 1).map(|c| hyphen_glyph(pl, c.style, &mut hcache)))
            .flatten();
        let end_x = match (xs.last(), c1.checked_sub(1).and_then(|k| pl.clusters.get(k))) {
            (Some(lx), Some(c)) => lx + c.adv + hyphen.map_or(0.0, |h| h.2),
            _ => line_start_x,
        };
        xs.push(end_x);
        pending_tab = None;
        let _ = pending_tab;

        // Vertical metrics.
        let (mut asc, mut desc) = (0.0f32, 0.0f32);
        // The tallest picture on the line (it sits on the baseline).
        let mut obj_asc = 0.0f32;
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
            if let Some(st) = pl.styles.get(c.style as usize) {
                // A picture sits on the baseline of its run: the line keeps that font's ascent
                // and descent, and grows above the baseline to fit a taller picture.
                let (a, d) = if matches!(c.kind, ClKind::Object(_)) && c.obj_h > 0.0 {
                    if pl.maths.iter().any(|(k, _)| c.kind == ClKind::Object(*k)) {
                        // Exact spacing keeps the run's baseline; tall math may overflow the
                        // fixed line box without moving adjacent text up by its denominator.
                        if matches!(rp.line_spacing, LineSpacing::Exactly(_)) { (st.ascent, st.descent) } else { (c.obj_h, c.obj_d) }
                    } else {
                        obj_asc = obj_asc.max(c.obj_h);
                        (st.ascent, st.descent)
                    }
                } else {
                    // `obj_h` is set on text clusters only under ruby text, which the line grows to fit.
                    let (a, d) = st.line_extent();
                    (a.max(c.obj_h + st.shift), d)
                };
                asc = asc.max(a);
                desc = desc.max(d);
                any = true;
            }
        }
        if (!any || (asc == 0.0 && desc == 0.0))
            && let Some(st) = pl.styles.get(mark_style as usize)
        {
            asc = st.ascent;
            desc = st.descent;
        }
        let text_natural = asc + desc;
        let (text_asc, text_desc) = (asc.max(obj_asc), desc);
        let text_pictures = if obj_asc > asc { obj_asc + desc } else { 0.0 };
        let mut label_extra = 0.0;
        if first && let Some(lab) = &pl.label {
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
        let (height, centred) = match (rp.line_spacing, grid) {
            (LineSpacing::Multiple(m), Some(g)) => ((g.snap_height(natural) * m).max(g.snap_height(pictures)), true),
            (LineSpacing::AtLeast(v), Some(g)) => (g.snap_height(natural.max(pictures)).max(v), true),
            (LineSpacing::Multiple(m), None) => ((text_natural * m + label_extra).max(text_pictures), false),
            (LineSpacing::AtLeast(v), None) => (natural.max(text_pictures).max(v), false),
            (LineSpacing::Exactly(v), _) => (v, false),
        };
        let baseline = if centred {
            let up = asc.max(obj_asc);
            top + ((height - up - desc) / 2.0).max(0.0) + up
        } else {
            top + baseline_offset(rp.line_spacing, asc, height, desc).max(obj_asc.min(height - desc))
        };

        // Alignment / justification (trailing spaces hang).
        let mut content_end = xs.last().copied().unwrap_or(line_start_x);
        for k in (c0..c1).rev() {
            match pl.clusters.get(k).map(|c| c.kind) {
                Some(ClKind::Space) | Some(ClKind::Marker) | Some(ClKind::LineBreak) | Some(ClKind::PageBreak) | Some(ClKind::ColumnBreak) => {
                    content_end = xs.get(k - c0).copied().unwrap_or(content_end);
                }
                _ => {
                    // The automatic space after the line's last character goes with the wrap.
                    content_end -= pl.auto_space_after(k);
                    break;
                }
            }
        }
        let slack = right_edge - content_end;
        let last_tab = (c0..c1).rev().find(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == ClKind::Tab));
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
            for v in xs.iter_mut().skip(align_from - c0) {
                *v += shift;
            }
            // After a tab the label stays, as the text before the tab does.
            if last_tab.is_none()
                && let Some(label_shift) = label_shifts.get_mut(lines.len())
            {
                *label_shift = shift;
            }
        }
        // A line ending in a manual line break is stretched too, unless `doNotExpandShiftReturn` says not to.
        let stretch_end = end == LineEnd::Wrap || (end == LineEnd::LineBreak && expand_break);
        let khmer_mode = matches!(rp.align, Align::ThaiDistribute | Align::KhmerJustify);
        let justify = display.is_none() && (((rp.align == Align::Justify || khmer_mode) && stretch_end) || rp.align == Align::Distribute);
        // A line that only fits with shrunk spaces is shrunk, the last line of the paragraph too.
        let shrink = display.is_none() && shrink_spaces && slack < 0.0 && last_tab.is_none();
        if shrink {
            shrink_line_spaces(pl, &mut xs, c0, c1, content_end, slack);
        } else if justify && slack > 0.0 && khmer_mode {
            // Space goes between the clusters of Thai and Khmer text, which has no spaces to
            // stretch: all alike (Thai Distributed) or by rank (Khmer Justify). Not after the
            // line's last cluster, nor after a space that hangs past the edge, nor in the spaces that
            // start the line.
            let mode = if rp.align == Align::KhmerJustify { crate::khmer_just::Mode::Tiered } else { crate::khmer_just::Mode::Even };
            // The spaces before the first character (a Thai letter indents a paragraph with a run
            // of them) keep their width, as in Word; so do anchors and empty objects around them.
            let blank = |k: usize| {
                pl.clusters.get(k).is_some_and(|c| {
                    matches!(c.kind, ClKind::Space | ClKind::Marker) || (matches!(c.kind, ClKind::Object(_)) && c.adv == 0.0 && c.obj_h == 0.0)
                })
            };
            let first = (align_from..c1).find(|k| !blank(*k)).unwrap_or(c1);
            let last = (align_from..c1).rev().find(|k| !blank(*k));
            let slots: Vec<usize> = match last {
                Some(l) if l > first => (first..l).collect(),
                _ => Vec::new(),
            };
            let gaps: Vec<(wordcraft_doc::khmer::Gap, f32)> = slots
                .iter()
                .map(|&k| {
                    let em = pl.clusters.get(k).and_then(|c| pl.styles.get(c.style as usize)).map_or(12.0, |s| s.size);
                    (pl.gaps.get(k).copied().unwrap_or(wordcraft_doc::khmer::Gap::None), em)
                })
                .collect();
            let adds = crate::khmer_just::distribute(mode, slack, &gaps);
            let mut add = 0.0;
            let mut it = slots.iter().zip(adds.iter()).peekable();
            for k in align_from..c1 {
                if let Some(v) = xs.get_mut(k - c0) {
                    *v += add;
                }
                if let Some((_, a)) = it.next_if(|(s, _)| **s == k) {
                    add += *a;
                }
            }
            if let Some(v) = xs.last_mut() {
                *v += add;
            }
        } else if justify && slack > 0.0 {
            // Spaces inside the content (after the last tab).
            let spaces: Vec<usize> = (align_from..c1)
                .filter(|k| {
                    pl.clusters.get(*k).is_some_and(|c| c.kind == ClKind::Space) && xs.get(k - c0).copied().unwrap_or(f32::MAX) < content_end - 0.01
                })
                .collect();
            if rp.align == Align::Distribute || spaces.is_empty() {
                let cnt = (c1 - align_from).saturating_sub(1).max(1) as f32;
                if rp.align == Align::Distribute {
                    let per = slack / cnt;
                    for (q, v) in xs.iter_mut().enumerate().skip(align_from - c0) {
                        let k = (q + c0 - align_from) as f32;
                        *v += per * k.min(cnt);
                    }
                }
            } else {
                let per = slack / spaces.len() as f32;
                let mut add = 0.0;
                let mut si = 0;
                for k in align_from..c1 {
                    if let Some(v) = xs.get_mut(k - c0) {
                        *v += add;
                    }
                    if spaces.get(si) == Some(&k) {
                        add += per;
                        si += 1;
                    }
                }
                if let Some(v) = xs.last_mut() {
                    *v += add;
                }
            }
        }
        let start = pl.clusters.get(c0).map(|c| c.start).unwrap_or(pl.text_len);
        let stop = if c1 > c0 { pl.clusters.get(c1 - 1).map(|c| c.end).unwrap_or(pl.text_len) } else { start };
        if first_x0.is_none() {
            first_x0 = xs.first().copied();
        }
        let mut line = Line {
            top,
            height,
            baseline,
            text_asc,
            text_desc,
            c0,
            c1,
            xs,
            leaders,
            end,
            start,
            stop,
            left,
            right: right_edge,
            hyphen,
            beside,
            vis: Vec::new(),
            rtl: rtl_para,
        };
        if bidi {
            reorder_line(pl, &mut line, width);
        }
        lines.push(line);
        first = false;
        skipped = 0;
        i = j;
        // Text that wrapped continues on the row's far side; anything else ends the row.
        if end != LineEnd::Wrap || i >= n {
            row_rest.clear();
        }
        if row_rest.is_empty() {
            top += even_row(lines.get_mut(row_first..).unwrap_or_default());
        }
        if i >= n {
            // A paragraph ending with a line break gets an empty last line.
            if matches!(end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) {
                let (asc, desc) = pl.styles.get(mark_style as usize).map(|s| (s.ascent, s.descent)).unwrap_or((10.0, 3.0));
                let natural = asc + desc;
                let h = match (rp.line_spacing, grid) {
                    (LineSpacing::Multiple(m), Some(g)) => g.snap_height(natural) * m,
                    (LineSpacing::AtLeast(v), Some(g)) => g.snap_height(natural).max(v),
                    (LineSpacing::Multiple(m), None) => natural * m,
                    (LineSpacing::AtLeast(v), None) => natural.max(v),
                    (LineSpacing::Exactly(v), _) => v,
                };
                let x0 = rp.indent_left;
                let (x0, left, right) = if rtl_para { (width - x0, width - base_right, width - x0) } else { (x0, x0, base_right) };
                lines.push(Line {
                    top,
                    height: h,
                    baseline: top + if grid.is_some() { h - desc } else { baseline_offset(rp.line_spacing, asc, h, desc) },
                    text_asc: asc,
                    text_desc: desc,
                    c0: n,
                    c1: n,
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
                    rtl: rtl_para,
                });
                top += h;
            }
            break;
        }
        if lines.len() > 200_000 {
            break;
        }
    }
    // Hang the drop cap from the first line's caps down to line N's baseline, left of the text.
    if let Some((nc, dl, dw)) = pl.drop_cap
        && let (Some(l0), Some(ln)) = (lines.first(), lines.get((dl as usize).min(lines.len()).saturating_sub(1)))
    {
        let short = lines.len() < dl as usize;
        let dy = if short { (dl as f32 - 1.0) * l0.height } else { ln.baseline - l0.baseline };
        let lead = first_x0.unwrap_or(0.0) - first_left.max(rp.indent_left);
        // In margin, the letter's right edge stays the distance away from the text's edge.
        let dx = if rp.drop_cap_margin { lead.max(0.0) + pl.drop_cap_w + rp.drop_cap_distance } else { dw.min(lead) };
        // Toward the start edge: left, or right (and from the letter's right edge) when mirrored.
        let dx = if rtl_para { pl.drop_cap_w - dx } else { dx };
        if short {
            top = top.max(l0.top + l0.height * dl as f32);
        }
        for c in pl.clusters.get(..nc).unwrap_or(&[]).to_vec() {
            for g in pl.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dy -= dy;
                g.dx -= dx;
            }
        }
    }
    pl.height = top;
    pl.lines = lines;
    let label_shift = label_shifts.get(pl.label_line()).copied().unwrap_or(0.0);
    if let Some(lab) = pl.label.as_mut() {
        lab.x += label_shift;
        if rtl_para {
            lab.x = width - lab.x - lab.width;
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
