//! Paragraph layout: resolve runs, shape them into clusters, break lines (first-fit, as word
//! processors do), place tabs, list labels and alignment.

use std::sync::Arc;

use wordcraft_doc::numbering::{Level, LevelSuffix};
use wordcraft_doc::para::{COLUMN_BREAK, InlineObject, LINE_BREAK, NoteKind, OBJ, PAGE_BREAK, SOFT_HYPHEN};
use wordcraft_doc::props::{Align, CharProps, LineSpacing, TabAlign, TabLeader, TabStop};
use wordcraft_doc::resolve::{ResolvedChar, ResolvedPara};
use wordcraft_doc::{Document, Paragraph};
use wordcraft_fonts::FaceRef;

use crate::fields::{FieldCtx, field_text};

/// A face at a size with the formatting needed to draw it.
#[derive(Clone, Debug)]
pub struct StyleRun {
    pub face: FaceRef,
    pub size: f32,
    pub synth_bold: bool,
    pub synth_italic: bool,
    pub rc: Arc<ResolvedChar>,
    /// Line ascent/descent at this size, points.
    pub ascent: f32,
    pub descent: f32,
    /// Baseline shift (superscript etc.), points, positive up.
    pub shift: f32,
}

/// A positioned glyph inside its cluster.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub gid: u32,
    /// Offset from the cluster's x, points.
    pub dx: f32,
    pub dy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClKind {
    Text,
    Space,
    Tab,
    LineBreak,
    PageBreak,
    ColumnBreak,
    /// Inline object with its index in `Paragraph::objects`.
    Object(usize),
    /// Zero-width marker (bookmark, comment anchor) or hidden text.
    Marker,
}

/// The smallest unit of layout: a grapheme cluster (or object, tab, break).
#[derive(Clone, Debug)]
pub struct Cluster {
    pub start: usize,
    pub end: usize,
    pub adv: f32,
    pub kind: ClKind,
    /// Index into `ParaLayout::styles`.
    pub style: u16,
    /// Glyph range in `ParaLayout::glyphs`.
    pub g0: u32,
    pub g1: u32,
    /// A line may break after this cluster.
    pub break_after: bool,
    /// Height above the baseline for objects (images), points.
    pub obj_h: f32,
    /// The cluster is a decimal separator (decimal tabs align on it).
    pub dot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnd {
    /// Wrapped (or last line).
    Wrap,
    LineBreak,
    PageBreak,
    ColumnBreak,
    /// End of paragraph.
    Para,
}

/// A laid-out line. Coordinates are relative to the paragraph's content box: x from the
/// column's left edge, y from the top of the first line.
#[derive(Clone, Debug)]
pub struct Line {
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    /// Cluster index range.
    pub c0: usize,
    pub c1: usize,
    /// x of each cluster in the line, plus the end x (len = c1 - c0 + 1).
    pub xs: Vec<f32>,
    /// For tab clusters: the leader to draw across them.
    pub leaders: Vec<(usize, TabLeader)>,
    pub end: LineEnd,
    /// Byte range of the paragraph covered by the line.
    pub start: usize,
    pub stop: usize,
    /// Left and right edges available to the line.
    pub left: f32,
    pub right: f32,
    /// The line ends at a hyphenation point: (style, glyph, advance) of the hyphen drawn there.
    pub hyphen: Option<(u16, u32, f32)>,
}

/// The list label drawn on the first line.
#[derive(Clone, Debug)]
pub struct Label {
    pub text: String,
    pub style: u16,
    pub glyphs: Vec<Glyph>,
    pub x: f32,
    pub width: f32,
}

/// A laid-out paragraph.
#[derive(Clone, Debug)]
pub struct ParaLayout {
    pub rp: ResolvedPara,
    pub styles: Vec<StyleRun>,
    pub glyphs: Vec<Glyph>,
    pub clusters: Vec<Cluster>,
    pub lines: Vec<Line>,
    pub label: Option<Label>,
    /// Sum of line heights (without space before/after).
    pub height: f32,
    pub text_len: usize,
    /// Paragraph contains fields whose text depends on the page.
    pub has_page_fields: bool,
    /// Note references (part id) in this paragraph, by cluster index.
    pub notes: Vec<(usize, u32)>,
    /// Proofing issues: (byte start, byte end, grammar?).
    pub issues: Vec<(usize, usize, bool)>,
    /// Drop cap: (clusters at the start that form it, lines it drops, width with its gap).
    pub drop_cap: Option<(usize, u8, f32)>,
    /// Clusters after which the line may break with a hyphen (soft hyphens, auto hyphenation), sorted.
    pub hyph_after: Vec<u32>,
}

/// Inputs that change a paragraph's layout beyond its own content.
pub struct ParaEnv<'a> {
    pub doc: &'a Document,
    /// Column width, points.
    pub width: f32,
    pub label: Option<(String, Level)>,
    pub fields: &'a FieldCtx,
    pub show_hidden: bool,
    /// Extra style applied to every run (table style conditional formatting), under direct formatting.
    pub table_chr: Option<&'a CharProps>,
    pub proofing: bool,
    /// Areas text must flow around (floating objects), relative to the paragraph: x from the
    /// column's left edge, y from the top of the first line.
    pub exclusions: &'a [Exclusion],
}

/// An area text wraps around.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exclusion {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    /// Text only above and below (no text beside it).
    pub top_bottom: bool,
}

fn style_key(rc: &ResolvedChar) -> String {
    format!("{}|{}|{}|{}|{}", rc.font, rc.size, rc.bold, rc.italic, rc.vert_align as u8)
}

struct Builder<'a> {
    env: &'a ParaEnv<'a>,
    styles: Vec<StyleRun>,
    style_index: std::collections::HashMap<(String, u32, bool), u16>,
    glyphs: Vec<Glyph>,
    clusters: Vec<Cluster>,
}

impl<'a> Builder<'a> {
    /// Style index for a resolved char in `face` (caps-scaled `size` override for small caps).
    fn style(&mut self, rc: &Arc<ResolvedChar>, face_override: Option<FaceRef>, small: bool) -> u16 {
        let r = wordcraft_fonts::word::resolve(&rc.font, rc.bold, rc.italic);
        let face = face_override.unwrap_or(r.face);
        let key = (
            format!("{}|{:?}|{}|{}", style_key(rc), rc.color, rc.underline as u8, small),
            face.id(),
            rc.strike || rc.double_strike || rc.link.is_some(),
        );
        let key = (format!("{}|{:?}|{:?}|{:?}|{:?}|{}", key.0, rc.highlight, rc.shading, rc.ins, rc.del, rc.hidden), key.1, key.2);
        if let Some(i) = self.style_index.get(&key) {
            return *i;
        }
        let size = rc.draw_size() * if small { 0.8 } else { 1.0 };
        let (a, d) = wordcraft_fonts::word::line_metrics(&face);
        let k = size as f64 / face.upem.max(1.0);
        let st = StyleRun {
            face,
            size,
            synth_bold: r.synth_bold && face_override.is_none(),
            synth_italic: r.synth_italic && face_override.is_none(),
            rc: rc.clone(),
            ascent: (a * k) as f32,
            descent: (d * k) as f32,
            shift: rc.baseline_shift(),
        };
        let i = self.styles.len().min(u16::MAX as usize) as u16;
        self.styles.push(st);
        self.style_index.insert(key, i);
        i
    }

    /// Shape `text` (a piece of the paragraph starting at byte `base`) in one style and append clusters.
    fn shape(&mut self, text: &str, base: usize, rc: &Arc<ResolvedChar>, kind_override: Option<ClKind>) {
        if text.is_empty() {
            return;
        }
        // Split by font coverage (fallback faces) and small caps case.
        let primary = wordcraft_fonts::word::resolve(&rc.font, rc.bold, rc.italic).face;
        let mut seg_start = 0;
        let mut cur: Option<(Option<FaceRef>, bool)> = None;
        let mut segs: Vec<(usize, usize, Option<FaceRef>, bool)> = Vec::new();
        for (i, c) in text.char_indices() {
            let face = if primary.covers(c) || c.is_whitespace() || c.is_control() || c == SOFT_HYPHEN {
                None
            } else {
                wordcraft_fonts::FontDb::global().fallback_for(c, primary.id()).map(|f| FaceRef::of(&f))
            };
            let small = rc.small_caps && !rc.caps && c.is_lowercase();
            let k = (face, small);
            match cur {
                Some(p) if p.0.map(|f| f.id()) == k.0.map(|f| f.id()) && p.1 == k.1 => {}
                Some(p) => {
                    segs.push((seg_start, i, p.0, p.1));
                    seg_start = i;
                    cur = Some(k);
                }
                None => cur = Some(k),
            }
        }
        if let Some(p) = cur {
            segs.push((seg_start, text.len(), p.0, p.1));
        }
        for (a, b, face, small) in segs {
            let Some(sub) = text.get(a..b) else { continue };
            let si = self.style(rc, face, small);
            let Some(st) = self.styles.get(si as usize).cloned() else { continue };
            let upper = rc.caps || small;
            let shaped = wordcraft_fonts::shape(&st.face, sub, &[], |c| if upper { c.to_uppercase().next().unwrap_or(c) } else { c });
            let k = st.size / st.face.upem.max(1.0) as f32;
            let hscale = rc.scale / 100.0;
            // Group glyphs by cluster byte offset; graphemes may span several shaper clusters.
            let bounds: Vec<usize> = unicode_segmentation::UnicodeSegmentation::grapheme_indices(sub, true).map(|(i, _)| i).collect();
            let mut gi = 0usize;
            for (bi, &gs) in bounds.iter().enumerate() {
                let ge = bounds.get(bi + 1).copied().unwrap_or(sub.len());
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
                let s = sub.get(gs..ge).unwrap_or("");
                let ch = s.chars().next().unwrap_or(' ');
                let kind = kind_override.unwrap_or(if ch == ' ' || ch == '\u{3000}' {
                    ClKind::Space
                } else if ch == SOFT_HYPHEN {
                    ClKind::Marker
                } else {
                    ClKind::Text
                });
                let adv = if kind == ClKind::Marker { 0.0 } else { adv + rc.spacing };
                self.clusters.push(Cluster {
                    start: base + a + gs,
                    end: base + a + ge,
                    adv,
                    kind,
                    style: si,
                    g0,
                    g1: if kind == ClKind::Marker { g0 } else { g1 },
                    break_after: false,
                    obj_h: 0.0,
                    dot: s == "." || s == ",",
                });
            }
        }
    }

    /// One cluster for a whole object with its display text (fields, note references).
    fn shape_atomic(&mut self, text: &str, start: usize, end: usize, rc: &Arc<ResolvedChar>) {
        let before = self.clusters.len();
        self.shape(text, start, rc, None);
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
                dot: false,
            });
            return;
        };
        // Merge into one cluster: rebase glyph dx onto the first cluster.
        let mut x = 0.0;
        for c in &added {
            for g in self.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                g.dx += x;
            }
            x += c.adv;
        }
        let g0 = first.g0;
        let style = first.style;
        let g1 = added.last().map(|c| c.g1).unwrap_or(g0);
        self.clusters.push(Cluster { start, end, adv: x, kind: ClKind::Text, style, g0, g1, break_after: false, obj_h: 0.0, dot: false });
    }
}

/// Lay out one paragraph.
pub fn layout_para(p: &Paragraph, env: &ParaEnv) -> ParaLayout {
    let doc = env.doc;
    let mut rp = doc.styles.resolve_para(&p.props);
    // List level indents apply unless the paragraph sets its own.
    if let Some((_, lvl)) = &env.label {
        if p.props.indent_left.is_none() {
            rp.indent_left = lvl.indent;
        }
        if p.props.indent_first.is_none() {
            rp.indent_first = -lvl.hanging;
        }
    }
    let para_style = p.props.style.as_deref();
    let resolve = |c: &CharProps| -> Arc<ResolvedChar> {
        match env.table_chr {
            Some(t) => {
                let merged = t.clone().overlaid(c);
                Arc::new(doc.styles.resolve_char(para_style, &merged))
            }
            None => Arc::new(doc.styles.resolve_char(para_style, c)),
        }
    };
    let mut b = Builder { env, styles: Vec::new(), style_index: Default::default(), glyphs: Vec::new(), clusters: Vec::new() };
    let mark_rc = resolve(&p.mark);
    let mark_style = b.style(&mark_rc, None, false);
    let mut has_page_fields = false;
    let mut notes = Vec::new();
    let mut obj_index = 0usize;
    // Drop cap: the first character, sized so its capital spans `lines` lines.
    let drop_first = (rp.drop_cap > 0)
        .then(|| p.text.chars().next())
        .flatten()
        .filter(|c| !matches!(*c, '\t' | LINE_BREAK | PAGE_BREAK | COLUMN_BREAK | OBJ | '\r' | ' '))
        .filter(|c| p.text.len() > c.len_utf8());
    let mut drop_cap = None;
    for (range, props) in p.run_ranges() {
        let rc = resolve(props);
        let Some(text) = p.text.get(range.clone()) else { continue };
        if rc.hidden && !b.env.show_hidden {
            let si = b.style(&rc, None, false);
            let g = b.glyphs.len() as u32;
            for (i, c) in text.char_indices() {
                if c == OBJ {
                    obj_index += 1;
                }
                b.clusters.push(Cluster {
                    start: range.start + i,
                    end: range.start + i + c.len_utf8(),
                    adv: 0.0,
                    kind: ClKind::Marker,
                    style: si,
                    g0: g,
                    g1: g,
                    break_after: false,
                    obj_h: 0.0,
                    dot: false,
                });
            }
            continue;
        }
        let mut seg = 0;
        if range.start == 0
            && let Some(c) = drop_first
            && let Some(first) = text.get(..c.len_utf8())
        {
            let line_h = (rc.size * 1.2).max(1.0);
            let lines = rp.drop_cap as f32;
            // Capital height is ~0.7 em: the letter's cap spans from line 1's caps to line N's baseline.
            let mut big = (*rc).clone();
            big.size = (((lines - 1.0) * line_h + rc.size * 0.7) / 0.7).clamp(rc.size, 1000.0);
            let before = b.clusters.len();
            b.shape(first, 0, &Arc::new(big), None);
            let w: f32 = b.clusters.get(before..).map(|c| c.iter().map(|c| c.adv).sum()).unwrap_or(0.0);
            drop_cap = Some((b.clusters.len() - before, rp.drop_cap, w + rc.size * 0.3));
            // The letter hangs beside the text; lines flow at the indent `break_lines` adds.
            for c in b.clusters.iter_mut().skip(before) {
                c.adv = 0.0;
            }
            seg = c.len_utf8();
        }
        for (i, c) in text.char_indices() {
            if i < seg {
                continue;
            }
            let special = matches!(c, '\t' | LINE_BREAK | PAGE_BREAK | COLUMN_BREAK | OBJ | '\r');
            if !special {
                continue;
            }
            if let Some(s) = text.get(seg..i) {
                b.shape(s, range.start + seg, &rc, None);
            }
            seg = i + c.len_utf8();
            let (start, end) = (range.start + i, range.start + i + c.len_utf8());
            let si = b.style(&rc, None, false);
            let g = b.glyphs.len() as u32;
            let push = |b: &mut Builder, kind: ClKind, adv: f32, h: f32| {
                b.clusters.push(Cluster { start, end, adv, kind, style: si, g0: g, g1: g, break_after: false, obj_h: h, dot: false })
            };
            match c {
                '\t' => push(&mut b, ClKind::Tab, 0.0, 0.0),
                LINE_BREAK | '\r' => push(&mut b, ClKind::LineBreak, 0.0, 0.0),
                PAGE_BREAK => push(&mut b, ClKind::PageBreak, 0.0, 0.0),
                COLUMN_BREAK => push(&mut b, ClKind::ColumnBreak, 0.0, 0.0),
                _ => {
                    let k = obj_index;
                    obj_index += 1;
                    match p.objects.get(k) {
                        Some(InlineObject::Image { w, h, float, .. }) | Some(InlineObject::Shape { w, h, float, .. }) => {
                            if float.wrap == wordcraft_doc::para::Wrap::Inline {
                                let maxw = (env.width - rp.indent_left.max(0.0) - rp.indent_right.max(0.0)).max(18.0);
                                let (w, h) = (w.clamp(1.0, 4000.0), h.clamp(1.0, 4000.0));
                                let s = if w > maxw { maxw / w } else { 1.0 };
                                push(&mut b, ClKind::Object(k), w * s, h * s);
                            } else {
                                push(&mut b, ClKind::Object(k), 0.0, 0.0);
                            }
                        }
                        Some(InlineObject::Field { instr, result, .. }) => {
                            let (t, page_dep) = field_text(instr, result, env.fields);
                            has_page_fields |= page_dep;
                            b.shape_atomic(&t, start, end, &rc);
                        }
                        Some(InlineObject::NoteRef { kind, id, custom }) => {
                            let num = if custom.is_empty() {
                                let n = env.fields.note_number(*id);
                                match kind {
                                    NoteKind::Footnote => doc.settings.footnote_format.format(n),
                                    NoteKind::Endnote => doc.settings.endnote_format.format(n),
                                }
                            } else {
                                custom.clone()
                            };
                            let mut sup = (*rc).clone();
                            sup.vert_align = wordcraft_doc::props::VertAlign::Superscript;
                            b.shape_atomic(&num, start, end, &Arc::new(sup));
                            notes.push((b.clusters.len().saturating_sub(1), *id));
                        }
                        Some(InlineObject::Equation { linear, .. }) => {
                            let mut eq = (*rc).clone();
                            eq.italic = true;
                            eq.font = "Cambria Math".into();
                            b.shape_atomic(linear, start, end, &Arc::new(eq));
                        }
                        Some(InlineObject::Opaque { text, .. }) => b.shape_atomic(text, start, end, &rc),
                        _ => push(&mut b, ClKind::Marker, 0.0, 0.0),
                    }
                }
            }
        }
        if let Some(s) = text.get(seg..) {
            b.shape(s, range.start + seg, &rc, None);
        }
    }
    // Break opportunities.
    let mut opps = std::collections::HashSet::new();
    for (i, o) in unicode_linebreak::linebreaks(&p.text) {
        if o == unicode_linebreak::BreakOpportunity::Allowed || i < p.text.len() {
            opps.insert(i);
        }
    }
    for c in &mut b.clusters {
        c.break_after = opps.contains(&c.end) || matches!(c.kind, ClKind::Object(_) | ClKind::Tab);
    }

    // List label.
    let label = env.label.as_ref().map(|(text, lvl)| {
        let lc = p.mark.clone().overlaid(&lvl.chr);
        let rc = resolve(&lc);
        // A Symbol/Wingdings bullet (U+F0B7…) without that font installed, or in a face that has
        // no glyph at the private-use code: draw the standard character instead, since a
        // substitute has another glyph, or none, there.
        let r = wordcraft_fonts::word::resolve(&rc.font, rc.bold, rc.italic);
        let text: String = text
            .chars()
            .map(|c| match wordcraft_fonts::word::symbol_font_char(&rc.font, c) {
                Some(u) if r.substituted || !r.face.covers(c) => u,
                _ => c,
            })
            .collect();
        let before_c = b.clusters.len();
        let before_g = b.glyphs.len();
        b.shape(&text, 0, &rc, None);
        let cl: Vec<Cluster> = b.clusters.drain(before_c..).collect();
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        let mut style = mark_style;
        for c in &cl {
            style = c.style;
            for g in b.glyphs.get(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                glyphs.push(Glyph { dx: g.dx + x, ..*g });
            }
            x += c.adv;
        }
        b.glyphs.truncate(before_g);
        Label { text, style, glyphs, x: 0.0, width: x }
    });

    let mut pl = ParaLayout {
        rp,
        styles: b.styles,
        glyphs: b.glyphs,
        clusters: b.clusters,
        lines: Vec::new(),
        label,
        height: 0.0,
        text_len: p.text.len(),
        has_page_fields,
        notes,
        issues: if env.proofing { proof_issues(p) } else { Vec::new() },
        drop_cap,
        hyph_after: Vec::new(),
    };
    pl.hyph_after = hyphenation_points(p, &pl, env.doc.settings.auto_hyphenation && !pl.rp.suppress_hyphens);
    for k in pl.hyph_after.clone() {
        if let Some(c) = pl.clusters.get_mut(k as usize) {
            c.break_after = true;
        }
    }
    break_lines(&mut pl, env, mark_style, env.label.as_ref().map(|(_, l)| l.suffix));
    pl
}

/// Next tab stop after `x` (column coordinates): explicit stops, the hanging-indent implicit stop,
/// then default stops.
fn next_tab(x: f32, tabs: &[TabStop], default_tab: f32, hanging_at: Option<f32>) -> TabStop {
    let mut best: Option<TabStop> = tabs.iter().copied().filter(|t| t.pos > x + 0.01).min_by(|a, b| a.pos.total_cmp(&b.pos));
    if let Some(h) = hanging_at
        && h > x + 0.01
        && best.is_none_or(|t| h < t.pos)
    {
        best = Some(TabStop { pos: h, align: TabAlign::Left, leader: TabLeader::None });
    }
    best.unwrap_or_else(|| {
        let d = if default_tab > 1.0 { default_tab } else { 36.0 };
        let n = ((x + 0.01) / d).floor() + 1.0;
        TabStop { pos: (n * d).min(1e6), align: TabAlign::Left, leader: TabLeader::None }
    })
}

fn break_lines(pl: &mut ParaLayout, env: &ParaEnv, mark_style: u16, suffix: Option<LevelSuffix>) {
    let rp = pl.rp.clone();
    let width = env.width.max(12.0);
    let base_right = (width - rp.indent_right).max(1.0);
    let default_tab = env.doc.settings.default_tab;
    let first_left = rp.indent_left + rp.indent_first;
    let hanging_at = if rp.indent_first < 0.0 { Some(rp.indent_left) } else { None };
    let n = pl.clusters.len();
    let mut hcache: Vec<(u16, u32, f32)> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    let mut i = 0usize;
    let mut top = 0.0f32;
    let mut first = true;
    let est_h = pl
        .styles
        .get(mark_style as usize)
        .map(|st| (st.ascent + st.descent) * if let LineSpacing::Multiple(m) = rp.line_spacing { m } else { 1.0 })
        .unwrap_or(14.0)
        .max(1.0);
    loop {
        let left;
        let right_edge;
        // Flow around floating objects: narrow the line or move it below them.
        let mut guard = 0;
        loop {
            let (mut lo, mut hi) = (if first { first_left } else { rp.indent_left }, base_right);
            if let Some((_, dl, dw)) = pl.drop_cap
                && lines.len() < dl as usize
            {
                lo = first_left.max(rp.indent_left) + dw;
            }
            let mut push: Option<f32> = None;
            for e in env.exclusions {
                if e.bottom <= top || e.top >= top + est_h {
                    continue;
                }
                if e.top_bottom || (e.left <= lo + 1.0 && e.right >= hi - 1.0) {
                    push = Some(push.map_or(e.bottom, |p: f32| p.max(e.bottom)));
                    continue;
                }
                if (e.left + e.right) / 2.0 < (lo + hi) / 2.0 {
                    lo = lo.max(e.right);
                } else {
                    hi = hi.min(e.left);
                }
            }
            if push.is_none() && hi - lo < 36.0 {
                push = env
                    .exclusions
                    .iter()
                    .filter(|e| e.bottom > top && e.top < top + est_h)
                    .map(|e| e.bottom)
                    .fold(None, |a: Option<f32>, b| Some(a.map_or(b, |a| a.min(b))));
            }
            match push {
                Some(y) if y > top && guard < 50 => {
                    top = y;
                    guard += 1;
                }
                _ => {
                    left = lo;
                    right_edge = hi.max(lo + 12.0);
                    break;
                }
            }
        }
        let mut x = left;
        // Label on the first line.
        if first && let Some(lab) = pl.label.as_mut() {
            lab.x = left;
            let end = left + lab.width;
            x = match suffix.unwrap_or(LevelSuffix::Tab) {
                LevelSuffix::Tab => {
                    let t = next_tab(end, &rp.tabs, default_tab, hanging_at);
                    t.pos
                }
                LevelSuffix::Space => end + pl.styles.get(lab.style as usize).map(|s| s.size * 0.25).unwrap_or(3.0),
                LevelSuffix::Nothing => end,
            };
        }
        let line_start_x = x;
        let c0 = i;
        let mut xs: Vec<f32> = Vec::new();
        let mut leaders = Vec::new();
        let mut last_break: Option<usize> = None; // cluster index after which we may break
        let mut last_plain: Option<(usize, f32)> = None; // last non-hyphen break and the x after it
        let mut end = LineEnd::Para;
        let mut j = i;
        let mut pending_tab: Option<(usize, TabStop, f32)> = None; // tab cluster, stop, x where tab started
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
                    resolve_tab(pl, &mut pending_tab, &mut xs, c0, x, &mut x);
                    let stop = next_tab(x, &rp.tabs, default_tab, hanging_at);
                    xs.push(x);
                    if stop.pos > right_edge + 0.01 && j > c0 && stop.align == TabAlign::Left {
                        // Tab past the right indent wraps (Word moves it to the next line).
                        if let Some(c) = pl.clusters.get_mut(j) {
                            c.adv = 0.0;
                        }
                        x = right_edge.max(x);
                        last_break = Some(j);
                        j += 1;
                        continue;
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
            // Text after a right/centre tab grows leftwards into the tab's space first.
            let absorbs = pending_tab.is_some_and(|(tj, stop, _)| {
                let room = pl.clusters.get(tj).map(|t| t.adv).unwrap_or(0.0);
                match stop.align {
                    TabAlign::Right | TabAlign::Decimal => room >= c.adv,
                    TabAlign::Center => room >= c.adv / 2.0 && x + c.adv / 2.0 <= right_edge + 0.01,
                    _ => false,
                }
            });
            let fits = absorbs || x + c.adv <= right_edge + 0.01 || c.kind == ClKind::Space || c.kind == ClKind::Marker;
            if !fits && j > c0 {
                // Wrap: back up to the last break opportunity on this line.
                end = LineEnd::Wrap;
                // Hyphenate only when the plain break leaves more than the hyphenation zone empty.
                if let (Some(bk), Some((pj, px))) = (last_break, last_plain)
                    && pj >= c0
                    && pj < bk
                    && pl.hyph_after.binary_search(&(bk as u32)).is_ok()
                    && right_edge - px <= HYPHENATION_ZONE
                {
                    last_break = Some(pj);
                }
                if let Some(bk) = last_break.filter(|bk| *bk >= c0) {
                    xs.truncate(bk + 1 - c0);
                    j = bk + 1;
                    // Recompute x for the kept clusters.
                    x = xs.last().copied().unwrap_or(line_start_x) + pl.clusters.get(bk).map(|c| c.adv).unwrap_or(0.0);
                } else {
                    // No opportunity: break here (a long word).
                }
                break;
            }
            xs.push(x);
            x += c.adv;
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
            if c.break_after {
                let hyph = pl.hyph_after.binary_search(&(j as u32)).is_ok();
                if !hyph {
                    last_break = Some(j);
                    last_plain = Some((j, x));
                } else if x + hyphen_glyph(pl, c.style, &mut hcache).2 <= right_edge + 0.01 {
                    last_break = Some(j);
                }
            }
            j += 1;
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
        let mut any = false;
        let dropped = pl.drop_cap.map_or(0, |d| d.0);
        for (k, c) in pl.clusters.get(c0..c1).into_iter().flatten().enumerate() {
            if c.kind == ClKind::Marker || c0 + k < dropped {
                continue;
            }
            if let Some(st) = pl.styles.get(c.style as usize) {
                let (a, d) = if matches!(c.kind, ClKind::Object(_)) && c.obj_h > 0.0 {
                    (c.obj_h, 0.0)
                } else {
                    (st.ascent + st.shift.max(0.0), st.descent + (-st.shift).max(0.0))
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
        if first
            && let Some(lab) = &pl.label
            && let Some(st) = pl.styles.get(lab.style as usize)
        {
            asc = asc.max(st.ascent);
            desc = desc.max(st.descent);
        }
        let natural = asc + desc;
        let height = match rp.line_spacing {
            LineSpacing::Multiple(m) => natural * m,
            LineSpacing::AtLeast(v) => natural.max(v),
            LineSpacing::Exactly(v) => v,
        };
        let baseline = top + height - desc;

        // Alignment / justification (trailing spaces hang).
        let mut content_end = xs.last().copied().unwrap_or(line_start_x);
        for k in (c0..c1).rev() {
            match pl.clusters.get(k).map(|c| c.kind) {
                Some(ClKind::Space) | Some(ClKind::Marker) | Some(ClKind::LineBreak) | Some(ClKind::PageBreak) | Some(ClKind::ColumnBreak) => {
                    content_end = xs.get(k - c0).copied().unwrap_or(content_end);
                }
                _ => break,
            }
        }
        let slack = right_edge - content_end;
        let last_tab = (c0..c1).rev().find(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == ClKind::Tab));
        let align_from = last_tab.map(|k| k + 1).unwrap_or(c0);
        let shift = match rp.align {
            Align::Center => slack / 2.0,
            Align::Right => slack,
            _ => 0.0,
        };
        if shift > 0.0 && slack > 0.0 {
            for v in xs.iter_mut().skip(align_from - c0) {
                *v += shift;
            }
        }
        let justify = (rp.align == Align::Justify && end == LineEnd::Wrap) || rp.align == Align::Distribute;
        if justify && slack > 0.0 {
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
        lines.push(Line { top, height, baseline, c0, c1, xs, leaders, end, start, stop, left, right: right_edge, hyphen });
        top += height;
        first = false;
        i = j;
        if i >= n {
            // A paragraph ending with a line break gets an empty last line.
            if matches!(end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) {
                let (asc, desc) = pl.styles.get(mark_style as usize).map(|s| (s.ascent, s.descent)).unwrap_or((10.0, 3.0));
                let natural = asc + desc;
                let h = match rp.line_spacing {
                    LineSpacing::Multiple(m) => natural * m,
                    LineSpacing::AtLeast(v) => natural.max(v),
                    LineSpacing::Exactly(v) => v,
                };
                let x0 = rp.indent_left;
                lines.push(Line {
                    top,
                    height: h,
                    baseline: top + h - desc,
                    c0: n,
                    c1: n,
                    xs: vec![x0],
                    leaders: Vec::new(),
                    end: LineEnd::Para,
                    start: pl.text_len,
                    stop: pl.text_len,
                    left: x0,
                    right: base_right,
                    hyphen: None,
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
        let dx = dw.min(l0.xs.first().copied().unwrap_or(0.0) - first_left.max(rp.indent_left));
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
}

/// Word's default hyphenation zone (0.25"), points.
const HYPHENATION_ZONE: f32 = 18.0;

/// The hyphen glyph and advance in a style (cached per paragraph).
fn hyphen_glyph(pl: &ParaLayout, style: u16, cache: &mut Vec<(u16, u32, f32)>) -> (u16, u32, f32) {
    if let Some(h) = cache.iter().find(|h| h.0 == style) {
        return *h;
    }
    let h = pl
        .styles
        .get(style as usize)
        .and_then(|st| {
            let g = wordcraft_fonts::shape(&st.face, "-", &[], |c| c).into_iter().next()?;
            let k = st.size / st.face.upem.max(1.0) as f32 * st.rc.scale / 100.0;
            Some((style, g.gid, g.x_advance as f32 * k))
        })
        .unwrap_or((style, 0, 0.0));
    cache.push(h);
    h
}

/// Clusters after which a line may end with a hyphen: soft hyphens always, and dictionary or
/// pattern hyphenation points of each word when automatic hyphenation is on.
fn hyphenation_points(p: &Paragraph, pl: &ParaLayout, auto: bool) -> Vec<u32> {
    let mut bytes: Vec<usize> = p.text.char_indices().filter(|(_, c)| *c == SOFT_HYPHEN).map(|(i, c)| i + c.len_utf8()).collect();
    if auto {
        let lim = wordcraft_proof::hyphen::Limits::default();
        let mut start: Option<usize> = None;
        let text = &p.text;
        for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
            let word_char = c.is_alphabetic() || c == '\'' || c == '\u{2019}';
            match (start, word_char) {
                (None, true) => start = Some(i),
                (Some(a), false) => {
                    if let Some(w) = text.get(a..i)
                        && w.chars().count() >= lim.min_word
                    {
                        let offs: Vec<usize> = w.char_indices().map(|(o, _)| o).collect();
                        for pt in wordcraft_proof::hyphen::hyphen_points(w, &lim) {
                            if let Some(o) = offs.get(pt) {
                                bytes.push(a + o);
                            }
                        }
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes.sort_unstable();
    let mut out: Vec<u32> = pl
        .clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.kind, ClKind::Text | ClKind::Marker) && bytes.binary_search(&c.end).is_ok())
        .map(|(i, _)| i as u32)
        .collect();
    out.dedup();
    out
}

fn pl_text_at(c: &Cluster) -> bool {
    c.dot
}

fn resolve_tab(pl: &mut ParaLayout, pending: &mut Option<(usize, TabStop, f32)>, xs: &mut [f32], c0: usize, x: f32, xo: &mut f32) {
    // Finish a centre/right/decimal tab: its text (possibly empty) ends at `x`.
    let Some((tj, stop, tx)) = pending.take() else { return };
    let seg = x - tx - pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
    let want = match stop.align {
        TabAlign::Right | TabAlign::Decimal => stop.pos - tx - seg,
        TabAlign::Center => stop.pos - tx - seg / 2.0,
        _ => return,
    }
    .max(0.0);
    let old = pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
    let delta = want - old;
    if delta.abs() > 0.001 {
        if let Some(c) = pl.clusters.get_mut(tj) {
            c.adv = want;
        }
        for v in xs.iter_mut().skip(tj + 1 - c0) {
            *v += delta;
        }
        *xo += delta;
    }
}

impl ParaLayout {
    /// x of byte offset `off` within line `li` (clamped to the line).
    pub fn x_of(&self, li: usize, off: usize) -> Option<f32> {
        let l = self.lines.get(li)?;
        for k in l.c0..l.c1 {
            let c = self.clusters.get(k)?;
            if off <= c.start {
                return l.xs.get(k - l.c0).copied();
            }
            if off < c.end {
                return l.xs.get(k - l.c0).copied();
            }
        }
        l.xs.last().copied()
    }
    /// The line holding offset `off` (a position at a wrap point belongs to the next line).
    pub fn line_of(&self, off: usize) -> usize {
        for (i, l) in self.lines.iter().enumerate() {
            let next_start = self.lines.get(i + 1).map(|n| n.start);
            match next_start {
                Some(ns) if off < ns => return i,
                Some(_) => {}
                None => return i,
            }
            if matches!(l.end, LineEnd::LineBreak | LineEnd::PageBreak | LineEnd::ColumnBreak) && off < l.stop {
                return i;
            }
        }
        self.lines.len().saturating_sub(1)
    }
    /// Byte offset closest to x on line `li`.
    pub fn off_at_x(&self, li: usize, x: f32) -> usize {
        let Some(l) = self.lines.get(li) else { return self.text_len };
        let mut best = l.start;
        let mut bestd = f32::MAX;
        for k in l.c0..=l.c1 {
            let Some(cx) = l.xs.get(k - l.c0) else { break };
            let off = if k < l.c1 { self.clusters.get(k).map(|c| c.start).unwrap_or(l.stop) } else { l.stop };
            // Don't place the caret after a line-ending break or wrap-trailing space on a wrapped line.
            if k == l.c1 && k > l.c0 {
                let lastc = self.clusters.get(k - 1);
                if lastc.is_some_and(|c| matches!(c.kind, ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak)) {
                    continue;
                }
                if l.end == LineEnd::Wrap && self.lines.get(li + 1).is_some() {
                    continue;
                }
            }
            let d = (cx - x).abs();
            if d < bestd {
                bestd = d;
                best = off;
            }
        }
        best
    }
    /// Line ascent/descent of the paragraph mark (for empty paragraphs and caret height).
    pub fn caret_metrics(&self, li: usize) -> (f32, f32) {
        let Some(l) = self.lines.get(li) else { return (10.0, 3.0) };
        (l.baseline - l.top, l.top + l.height - l.baseline)
    }
}

/// Spelling and grammar issues in a paragraph (skipping "do not check" and hidden runs).
fn proof_issues(p: &Paragraph) -> Vec<(usize, usize, bool)> {
    if p.text.trim().is_empty() || p.text.len() > 100_000 {
        return Vec::new();
    }
    let text = proof_text(p);
    let skip = |a: usize, b: usize| {
        p.run_ranges().any(|(r, c)| r.start < b && a < r.end && (c.no_proof == Some(true) || c.hidden == Some(true) || c.link.is_some()))
    };
    let mut v: Vec<(usize, usize, bool)> =
        wordcraft_proof::check_spelling(&text).into_iter().filter(|i| !skip(i.start, i.end)).map(|i| (i.start, i.end, false)).collect();
    v.extend(wordcraft_proof::check_grammar(&text).into_iter().filter(|i| !skip(i.start, i.end)).map(|i| (i.start, i.end, true)));
    v
}

/// Text for proofing with the same byte offsets: inline objects and tracked deletions become
/// U+0001 bytes (neither words nor spaces).
pub fn proof_text(p: &wordcraft_doc::Paragraph) -> String {
    let mut out = String::with_capacity(p.text.len());
    for (i, c) in p.text.char_indices() {
        let deleted = c == wordcraft_doc::para::OBJ || p.props_of_char(i).del.is_some();
        if deleted {
            for _ in 0..c.len_utf8() {
                out.push('\u{1}');
            }
        } else {
            out.push(c);
        }
    }
    out
}
