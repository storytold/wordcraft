use super::bidi::{Dir, bidi_levels, visual_order};
use super::builder::{Builder, left_out, overstrike};
use super::chars::caps_char;
use super::grid::{auto_space, snap_chars};
use super::justify::hyphenation_points;
use super::lines::break_lines;
use super::model::{ClKind, Cluster, Glyph, Label, LabelRun, ParaEnv, undrawable_bullet};
use super::proof::{ProofRun, is_list_item, proof_issues};
use super::*;

/// Lay out one paragraph.
pub fn layout_para(p: &Paragraph, env: &ParaEnv) -> ParaLayout {
    let doc = env.doc;
    let mut rp = doc.styles.resolve_para_in(&p.props, env.table.map(|t| &t.para));
    if let Some(spacing) = env.read_line_spacing.filter(|s| s.is_finite() && (1.0..=2.0).contains(s)) {
        rp.line_spacing = wordcraft_doc::props::LineSpacing::Multiple(spacing);
    }
    let para_style = p.props.style.as_deref();
    let table_chr = env.table.map(|t| &t.chr);
    // Indents in characters are measured in the paragraph mark's font size.
    if rp.indent_chars.iter().any(|c| *c != 0.0) {
        rp.set_char_unit(doc.styles.resolve_char_in(para_style, table_chr, &p.mark).size);
    }
    // List level indents apply unless the paragraph sets its own, or, for a list from the style,
    // the style giving the list (or one based on it) does (`indents_over_list`). A paragraph
    // that switches its style's list off (`num: 0`) drops the styles' indents, as Word does.
    let own_left = p.props.indent_left.is_some() || p.props.indent_left_chars.is_some_and(|c| c != 0.0);
    let own_first = p.props.indent_first.is_some() || p.props.indent_first_chars.is_some_and(|c| c != 0.0);
    if let Some((_, lvl)) = &env.label {
        let (style_left, style_first) =
            if p.props.numbering.is_some() { (false, false) } else { doc.styles.indents_over_list(para_style.unwrap_or("Normal")) };
        if !own_left && !style_left {
            rp.indent_left = lvl.indent;
        }
        if !own_first && !style_first {
            rp.indent_first = -lvl.hanging;
        }
    } else if p.props.numbering.is_some_and(|n| n.num == 0) {
        if !own_left {
            rp.indent_left = 0.0;
        }
        if !own_first {
            rp.indent_first = 0.0;
        }
    }
    let resolve = |c: &CharProps| -> Arc<ResolvedChar> { Arc::new(doc.styles.resolve_char_in(para_style, table_chr, c)) };
    let levels = bidi_levels(&p.text, rp.bidi);
    let mut b =
        Builder { env, styles: Vec::new(), style_index: Default::default(), glyphs: Vec::new(), clusters: Vec::new(), levels, shown: Vec::new() };
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
    let mut drop_cap_w = 0.0;
    let mut maths = Vec::new();
    let mut enclosures = Vec::new();
    let mut rubies = Vec::new();
    let mut displays = Vec::new();
    let mut ptabs = Vec::new();
    let mut eq_counter = env.eq_number;
    let math_props = doc.settings.math.clone().unwrap_or_default();
    // The byte ranges of runs left out of the layout, in order, adjacent runs merged.
    let mut left: Vec<std::ops::Range<usize>> = Vec::new();
    // The byte ranges of objects with nothing to show (bookmarks, comment anchors), in order.
    let mut bare: Vec<std::ops::Range<usize>> = Vec::new();
    // Each run's proofing language and whether it is checked (spelling, grammar, hyphenation).
    let mut runs: Vec<ProofRun> = Vec::new();
    for (range, props) in p.run_ranges() {
        let rc = resolve(props);
        runs.push(ProofRun::of(range.clone(), &rc));
        let Some(text) = p.text.get(range.clone()) else { continue };
        if left_out(doc, &rc, b.env.show_hidden, b.env.hide_deleted) {
            match left.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => left.push(range.clone()),
            }
            let si = b.style(&rc, None, false);
            let g = b.glyphs.len() as u32;
            // Word still prints the note of a hidden reference mark (and numbers it); a deleted
            // reference is gone from the final text, note and all.
            let keeps_notes = !b.env.hide_deleted.hides(doc, rc.del);
            for (i, c) in text.char_indices() {
                let mut note = None;
                if c == OBJ {
                    if let Some(InlineObject::NoteRef { id, .. }) = p.objects.get(obj_index)
                        && keeps_notes
                    {
                        note = Some(*id);
                    }
                    obj_index += 1;
                }
                if let Some(id) = note {
                    notes.push((b.clusters.len(), id));
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
                    obj_d: 0.0,
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
            // `clamp` would panic here when the text itself is larger than 1000 pt.
            big.size = (((lines - 1.0) * line_h + rc.size * 0.7) / 0.7).min(1000.0).max(rc.size);
            if let Some(f) = &rp.drop_cap_font {
                big.font = f.clone();
            }
            let before = b.clusters.len();
            b.shape(first, 0, &Arc::new(big), None);
            let w: f32 = b.clusters.get(before..).map(|c| c.iter().map(|c| c.adv).sum()).unwrap_or(0.0);
            // Dropped: the first lines step aside by the letter and its distance from the text.
            // In margin: they keep their indent and the letter hangs left of them.
            let indent = if rp.drop_cap_margin { 0.0 } else { w + rp.drop_cap_distance };
            drop_cap = Some((b.clusters.len() - before, rp.drop_cap, indent));
            drop_cap_w = w;
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
                b.clusters.push(Cluster { start, end, adv, kind, style: si, g0: g, g1: g, break_after: false, obj_h: h, obj_d: 0.0, dot: false })
            };
            match c {
                '\t' => push(&mut b, ClKind::Tab, 0.0, 0.0),
                LINE_BREAK | '\r' => push(&mut b, ClKind::LineBreak, 0.0, 0.0),
                // Word ignores explicit page breaks in cells, preserving their source offsets.
                PAGE_BREAK if env.table.is_some() => push(&mut b, ClKind::Marker, 0.0, 0.0),
                PAGE_BREAK => push(&mut b, ClKind::PageBreak, 0.0, 0.0),
                COLUMN_BREAK => push(&mut b, ClKind::ColumnBreak, 0.0, 0.0),
                _ => {
                    let k = obj_index;
                    obj_index += 1;
                    match p.objects.get(k) {
                        Some(o) if o.is_drawing() => {
                            // A hidden one takes no room, and with no height its line doesn't draw it.
                            if let Some((w, saved_h, float)) = o.frame()
                                && float.wrap == wordcraft_doc::para::Wrap::Inline
                                && !float.hidden
                            {
                                // A picture wider than the line keeps its size, as in Word: it runs past
                                // the margin and the page edge clips it (see `ParaEnv::fit_objects`).
                                let maxw = (env.width - rp.indent_left.max(0.0) - rp.indent_right.max(0.0)).max(18.0);
                                let h = env.fitted.iter().find(|(i, _)| *i == k).map_or(saved_h, |(_, h)| *h);
                                let (w, h) = (w.clamp(1.0, 4000.0), h.clamp(1.0, 4000.0));
                                let s = if env.fit_objects && w > maxw { maxw / w } else { 1.0 };
                                // The line makes room for effects (shadows) around the picture too, and
                                // for a rotated one's bounds, as Word does.
                                let [el, et, er, eb] = float.effect_extent();
                                let (px, py) = float.spin_pad(w * s, h * s);
                                push(&mut b, ClKind::Object(k), w * s + el + er + 2.0 * px, h * s + et + eb + 2.0 * py);
                            } else {
                                push(&mut b, ClKind::Object(k), 0.0, 0.0);
                            }
                        }
                        Some(o @ InlineObject::Field { instr, result, code, .. }) => {
                            let form = if env.field_codes != *code { None } else { o.legacy_form() };
                            if let Some(wordcraft_doc::legacy_form::LegacyForm::CheckBox { size, .. }) = form {
                                // Word draws a legacy check box as a square as tall as the text's
                                // line (its ascent and descent), scaled to an explicit size, and
                                // takes that much room on the line (see `display::check_box`).
                                let (a, d) = b.styles.get(si as usize).map_or((rc.size * 0.8, rc.size * 0.2), |s| (s.ascent, s.descent));
                                let scale = size.map_or(1.0, |sz| sz / rc.size.max(0.5));
                                let (a, d) = (a * scale, d * scale);
                                b.clusters.push(Cluster {
                                    start,
                                    end,
                                    adv: a + d,
                                    kind: ClKind::Object(k),
                                    style: si,
                                    g0: g,
                                    g1: g,
                                    break_after: false,
                                    obj_h: a.max(0.01),
                                    obj_d: d,
                                    dot: false,
                                });
                            } else if let Some(text) = form.as_ref().and_then(|f| f.shown_text()) {
                                // A drop-down shows its selected entry, whatever result was saved.
                                b.shape_atomic(text, start, end, &rc);
                            } else if env.field_codes != *code {
                                // The code in braces on grey, like Word's field shading.
                                let mut shaded = (*rc).clone();
                                if shaded.highlight.is_none() && shaded.shading.is_none() {
                                    shaded.shading = Some(FIELD_SHADING);
                                }
                                b.shape_atomic(&field_code_text(instr), start, end, &Arc::new(shaded));
                            } else {
                                match wordcraft_doc::eq::parse_overstrike(instr) {
                                    Some(o) => overstrike(&mut b, &o, instr, start, end, &rc, &mut enclosures),
                                    None => {
                                        let (t, page_dep) = field_text_on(instr, result, env.fields, env.fields.page_at(start));
                                        has_page_fields |= page_dep;
                                        let rc = if rc.link.is_none()
                                            && let Some(link) = reference_link(instr)
                                        {
                                            let mut linked = (*rc).clone();
                                            linked.link = Some(link);
                                            Arc::new(linked)
                                        } else {
                                            rc.clone()
                                        };
                                        b.shape_atomic(&t, start, end, &rc);
                                    }
                                }
                            }
                        }
                        Some(InlineObject::NoteRef { kind, id, custom }) => {
                            let num = if custom.is_empty() {
                                let fmt = match kind {
                                    NoteKind::Footnote => doc.settings.footnote_format,
                                    NoteKind::Endnote => doc.settings.endnote_format,
                                };
                                env.fields.note_mark(*id, fmt)
                            } else {
                                custom.clone()
                            };
                            let mut sup = (*rc).clone();
                            sup.vert_align = wordcraft_doc::props::VertAlign::Superscript;
                            b.shape_atomic(&num, start, end, &Arc::new(sup));
                            notes.push((b.clusters.len().saturating_sub(1), *id));
                        }
                        Some(InlineObject::Equation { linear, display, math }) => {
                            let avail = (env.width - rp.indent_left.max(0.0) - rp.indent_right.max(0.0)).max(12.0);
                            let ml = crate::math::layout_equation(math, linear, &rc, *display, avail, &math_props, &mut eq_counter);
                            let st = b.styles.get(si as usize);
                            // The line is at least as tall as the text around it.
                            let (a, d) = st.map(|s| (s.ascent, s.descent)).unwrap_or((0.0, 0.0));
                            b.clusters.push(Cluster {
                                start,
                                end,
                                adv: ml.width,
                                kind: ClKind::Object(k),
                                style: si,
                                g0: g,
                                g1: g,
                                break_after: false,
                                obj_h: ml.ascent.max(a).max(0.01),
                                obj_d: ml.descent.max(d),
                                dot: false,
                            });
                            if *display {
                                displays.push((b.clusters.len() - 1, math.jc));
                            }
                            maths.push((k, Arc::new(ml)));
                        }
                        // A note separator's line: a short one, or one across the column.
                        Some(o) if o.as_note_separator().is_some() => {
                            let cont = o.as_note_separator() == Some(true);
                            let maxw = (env.width - rp.indent_left.max(0.0) - rp.indent_right.max(0.0)).max(1.0);
                            push(&mut b, ClKind::Object(k), if cont { maxw } else { crate::NOTE_SEP_W.min(maxw) }, 0.0);
                        }
                        Some(InlineObject::Ruby { base, ruby, align, size, raise, props: ruby_props, .. }) => {
                            let mut rr = (*resolve(&props.clone().overlaid(ruby_props))).clone();
                            let size = if size.is_finite() { size.clamp(1.0, 1584.0) } else { rc.size * 0.5 };
                            (rr.size, rr.size_cs, rr.position) = (size, size, 0.0);
                            rr.vert_align = wordcraft_doc::props::VertAlign::Baseline;
                            let raise = if raise.is_finite() { raise.clamp(0.0, 1584.0) } else { rc.size };
                            if let Some(r) = b.ruby(base, ruby, *align, raise, start, end, &rc, &Arc::new(rr)) {
                                rubies.push(r);
                            }
                        }
                        Some(InlineObject::Opaque { text, .. }) => b.shape_atomic(text, start, end, &rc),
                        Some(InlineObject::PositionalTab { align, relative_to, leader }) => {
                            push(&mut b, ClKind::Tab, 0.0, 0.0);
                            ptabs.push((b.clusters.len() - 1, *align, *relative_to, *leader));
                        }
                        _ => {
                            bare.push(start..end);
                            push(&mut b, ClKind::Marker, 0.0, 0.0)
                        }
                    }
                }
            }
        }
        if let Some(s) = text.get(seg..) {
            b.shape(s, range.start + seg, &rc, None);
        }
    }
    if rp.snap_to_grid {
        snap_chars(&mut b, &p.text, &env.grid);
    }
    // Text on a character grid keeps to its cells.
    let auto_space =
        if rp.snap_to_grid && env.grid.chars.is_some() { Vec::new() } else { auto_space(&mut b, &p.text, rp.auto_space_de, rp.auto_space_dn) };
    // Break opportunities and hyphenation points come from the text as laid out: hidden text that is
    // not shown and, without markup, tracked deletions are left out, so a space or soft hyphen there
    // is no place to break, and words are hyphenated as they read. An object with nothing to show
    // is left out too: its U+FFFC has the line-break class CB, but a bookmark never moves a break.
    let mut skip = left.clone();
    skip.extend(bare);
    skip.sort_unstable_by_key(|r| r.start);
    // The text and, for each char of it, its end there and in `p.text`.
    let (text, ends): (std::borrow::Cow<str>, Vec<(usize, usize)>) = if skip.is_empty() {
        (p.text.as_str().into(), Vec::new())
    } else {
        let (mut text, mut ends) = (String::new(), Vec::new());
        let mut next = skip.iter().peekable();
        for (i, c) in p.text.char_indices() {
            // `skip` is in order: skip the ranges that end here, then is `i` in the next one?
            while next.next_if(|r| r.end <= i).is_some() {}
            if next.peek().is_some_and(|r| r.start <= i) {
                continue;
            }
            text.push(c);
            ends.push((text.len(), i + c.len_utf8()));
        }
        (text.into(), ends)
    };
    // The end of a char in `text` → the same place in `p.text`.
    let to_para = |i: usize| -> Option<usize> {
        if skip.is_empty() { Some(i) } else { ends.binary_search_by_key(&i, |e| e.0).ok().and_then(|k| ends.get(k)).map(|e| e.1) }
    };
    let mut found = Vec::new();
    for (i, o) in unicode_linebreak::linebreaks(&text) {
        // Word keeps "and/or" and web addresses whole: no break right after a slash (a word
        // too long for the line still breaks anywhere).
        let after_slash =
            text.get(..i).is_some_and(|t| t.ends_with('/')) && text.get(i..).and_then(|t| t.chars().next()).is_some_and(char::is_alphanumeric);
        if (o == unicode_linebreak::BreakOpportunity::Allowed || i < text.len()) && !after_slash {
            found.push(i);
        }
    }
    // UAX #14 (LB13) also forbids a break between a space and a slash, so with the break after the
    // slash gone as well, "caption /word" had no break at all and the word split mid-letter. Word
    // wraps "/word" whole to the next line: break before a slash that starts a word.
    let before_slash: Vec<usize> = text
        .match_indices('/')
        .filter(|&(i, _)| {
            text.get(..i).and_then(|t| t.chars().next_back()).is_some_and(|c| c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}'))
                && text.get(i + 1..).and_then(|t| t.chars().next()).is_some_and(char::is_alphanumeric)
        })
        .map(|(i, _)| i)
        .collect();
    if !before_slash.is_empty() {
        found.extend(before_slash);
    }

    // Word breaks after a hyphen even when a digit follows ("2016-" / "2022"), where UAX #14
    // keeps a hyphen and a number together (LB25): add those positions back. Only U+002D before
    // ASCII digits is added here; a non-breaking hyphen (U+2011) and a minus sign (U+2212) stay
    // unbreakable, and the other dashes keep their ordinary UAX #14 breaks.
    let hyphen_breaks: Vec<usize> = text
        .match_indices('-')
        .filter_map(|(pos, _)| {
            let i = pos + 1;
            text.get(i..).and_then(|t| t.chars().next()).is_some_and(|n| n.is_ascii_digit()).then_some(i)
        })
        .collect();
    if !hyphen_breaks.is_empty() {
        found.extend(hyphen_breaks);
        found.sort_unstable();
        found.dedup();
    }
    // Myanmar has no spaces between words: it wraps between syllables.
    if text.chars().any(|c| matches!(u32::from(c), 0x1000..=0x109F | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F)) {
        found.extend(wordcraft_doc::cluster::myanmar_breaks(&text));
        found.sort_unstable();
        found.dedup();
    }
    // Thai and Lao have no spaces between words: replace UAX #14's per-character SA breaks with
    // ICU4X dictionary word boundaries.
    if text.chars().any(wordcraft_doc::cluster::is_thai_lao) {
        found.retain(|&i| {
            let prev = text.get(..i).and_then(|s| s.chars().next_back());
            let next = text.get(i..).and_then(|s| s.chars().next());
            !matches!((prev, next), (Some(p), Some(n))
                if wordcraft_doc::cluster::is_thai_lao(p) && wordcraft_doc::cluster::is_thai_lao(n))
        });
        found.extend(wordcraft_doc::cluster::thai_lao_breaks(&text));
        found.sort_unstable();
        found.dedup();
    }
    // Asian typography: kinsoku and breaking Latin words anywhere.
    crate::kinsoku::apply(&text, &mut found, rp.kinsoku, rp.word_wrap);
    let opps: std::collections::HashSet<usize> = found.into_iter().filter_map(to_para).collect();
    for c in &mut b.clusters {
        c.break_after = opps.contains(&c.end) || matches!(c.kind, ClKind::Object(_) | ClKind::Tab);
    }

    // Word hides a list label on a paragraph containing only a page break. The list counter
    // has already advanced, so the following item keeps its number.
    let break_only =
        b.clusters.iter().any(|c| c.kind == ClKind::PageBreak) && b.clusters.iter().all(|c| matches!(c.kind, ClKind::PageBreak | ClKind::Marker));
    let label = env.label.as_ref().filter(|_| !break_only).map(|(text, lvl)| {
        let lc = p.mark.clone().overlaid(&lvl.chr);
        let rc = resolve(&lc);
        // A bullet no installed font can draw would come from whatever icon font has its
        // private-use code (Font Awesome's Twitter logo at U+F081, #1683): it shows the level's
        // default bullet instead.
        let fixed = undrawable_bullet(text, lvl, &rc)
            .then(|| wordcraft_doc::numbering::default_bullet(usize::from(rp.numbering.map_or(0, |n| n.level))).to_string());
        let text: &str = fixed.as_deref().unwrap_or(text);
        // A Symbol/Wingdings bullet (U+F0B7…) is drawn as its Unicode equivalent when the font
        // isn't installed (shaping maps it); the label's text reads as that character too.
        let shown = |t: &str| -> String { wordcraft_doc::symbol_fonts::map_text(&rc.font, t) };
        // A right-to-left paragraph's label reads right to left too ("1." shows as ".1"), and its
        // brackets are mirrored (UAX #9 L4), which the shaper does for the runs the levels report
        // as right to left ("(" drawn as ")"), so "(1)" reads "(1)" and not ")1(".
        let lv = if rp.bidi { bidi_levels(text, true) } else { Vec::new() };
        let before_c = b.clusters.len();
        let before_g = b.glyphs.len();
        b.shape_with(text, 0, &rc, None, if lv.is_empty() { Dir::Guess } else { Dir::Own(&lv) });
        let mut cl: Vec<Cluster> = b.clusters.drain(before_c..).collect();
        if !lv.is_empty() {
            let cl_lv: Vec<u8> = cl.iter().map(|c| lv.get(c.start).copied().unwrap_or(1)).collect();
            let order = visual_order(&cl_lv);
            cl = order.iter().filter_map(|&i| cl.get(i).cloned()).collect();
        }
        let mut glyphs = Vec::new();
        let mut runs: Vec<LabelRun> = Vec::new();
        let mut ascent = 0.0f32;
        let mut x = 0.0;
        let mut style = mark_style;
        for c in &cl {
            style = c.style;
            if let Some(st) = b.styles.get(style as usize) {
                ascent = ascent.max(st.ascent + st.shift.max(0.0));
            }
            let g0 = glyphs.len();
            for g in b.glyphs.get(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                glyphs.push(Glyph { dx: g.dx + x, ..*g });
            }
            // A label in an All Caps paragraph reads as the capitals drawn (#1473).
            let chars: String = shown(text.get(c.start..c.end).unwrap_or("")).chars().map(|ch| caps_char(&rc, ch)).collect();
            match runs.last_mut() {
                Some(r) if r.style == c.style => {
                    r.glyphs.end = glyphs.len();
                    r.text.push_str(&chars);
                }
                _ => runs.push(LabelRun { style: c.style, glyphs: g0..glyphs.len(), text: chars }),
            }
            x += c.adv;
        }
        b.glyphs.truncate(before_g);
        Label { text: shown(text), ascent, style, glyphs, runs, x: 0.0, width: x }
    });

    // Shaping happens separately per font/style segment. Segment-local grapheme starts can
    // therefore land inside a paragraph grapheme (e.g. an emoji whose components use different
    // fallback faces). Retain the drawing segments and record only those illegal edit stops.
    let joined_cluster_starts = if p.text.is_ascii() {
        Vec::new()
    } else {
        let boundaries = wordcraft_doc::cluster::cluster_starts(&p.text);
        let mut joined: Vec<usize> =
            b.clusters.iter().filter_map(|c| (c.start < p.text.len() && boundaries.binary_search(&c.start).is_err()).then_some(c.start)).collect();
        joined.sort_unstable();
        joined.dedup();
        joined
    };
    let mut pl = ParaLayout {
        rp,
        styles: b.styles,
        glyphs: b.glyphs,
        clusters: b.clusters,
        shown: b.shown,
        lines: Vec::new(),
        label,
        height: 0.0,
        text_len: p.text.len(),
        joined_cluster_starts,
        has_page_fields,
        notes,
        issues: env.proofing.map(|o| proof_issues(p, &runs, &o, env.typing_word, is_list_item(doc, p))).unwrap_or_default(),
        drop_cap,
        hyph_after: Vec::new(),
        bidi_levels: b.levels,
        drop_cap_w,
        maths,
        rubies,
        displays,
        ptabs,
        left_out: Vec::new(),
        enclosures,
        auto_space,
    };
    let auto = env.doc.settings.auto_hyphenation && !pl.rp.suppress_hyphens;
    pl.hyph_after = hyphenation_points(&text, to_para, &pl, auto, env.doc.settings.hyphenate_caps, &runs);
    // A paragraph whose text and mark are all a deletion the layout leaves out (a deleted
    // paragraph in the final text, or moved-away text marked Hidden) is gone: it takes no room.
    // So is one whose mark is hidden and whose text is empty or hidden too: Word shows nothing of
    // it, not even its list number, though the number is counted (#3982).
    let mark_left_out = env.hide_deleted.hides(doc, mark_rc.del) || (mark_rc.hidden && !env.show_hidden);
    let gone = mark_left_out && (p.text.is_empty() || left.first() == Some(&(0..p.text.len())));
    pl.left_out = left;
    // Neither ordinary nor hyphenated wrapping may split a drawing group inside a grapheme.
    for c in &mut pl.clusters {
        c.break_after &= pl.joined_cluster_starts.binary_search(&c.end).is_err();
    }
    pl.hyph_after.retain(|&k| pl.clusters.get(k as usize).is_some_and(|c| pl.joined_cluster_starts.binary_search(&c.end).is_err()));
    for k in pl.hyph_after.clone() {
        if let Some(c) = pl.clusters.get_mut(k as usize) {
            c.break_after = true;
        }
    }
    break_lines(&mut pl, env, mark_style, env.label.as_ref().map(|(_, l)| l));
    if gone {
        collapse(&mut pl);
    }
    pl
}

/// The width of the word that starts at each cluster: the clusters up to the first break
/// opportunity that isn't a hyphenation point, their advance, spaces aside.
///
/// The word is the same set the scan this replaces gathered, which stopped at that first
/// opportunity rather than running on to the end of the paragraph.
///
/// Every word is measured once, before the line is broken, so a word that has to be measured
/// again — it doesn't fit the span beside a floating object, so it moves on to the next row —
/// costs a lookup rather than another pass over it (#18, #3412).
///
/// The width is the sum of the advances, the negative ones included: a font that pulls a glyph
/// back takes the width off again, so a word whose advances alternate is measured by that sum
/// and not by how wide it gets on the way. The sum is taken over the whole word for that
/// reason, and the words after a break are left out of it: they are words of their own.
///
/// The sum is added up from the word's end rather than its start, which is the same number in
/// all but the last bits of a float: the sum a float holds depends on the order it is added in,
/// and the two differ by far less than the tenth of a point the comparison that uses the width
/// allows.
pub(crate) fn word_widths(clusters: &[Cluster], hyph_after: &[u32]) -> Vec<f32> {
    let n = clusters.len();
    let mut width = vec![0.0f32; n + 1];
    for k in (0..n).rev() {
        let c = &clusters[k];
        let adv = if c.kind == ClKind::Space { 0.0 } else { c.adv };
        // The word ends at a break opportunity. A hyphenation point isn't one: the break there
        // is optional, so the word runs on to the next.
        width[k] = if c.break_after && hyph_after.binary_search(&(k as u32)).is_err() { adv } else { adv + width[k + 1] };
    }
    width
}

/// Lays a paragraph out as nothing: no height, no spacing, no label, its lines (kept for the
/// caret and hit testing) all at its top with no height.
fn collapse(pl: &mut ParaLayout) {
    for l in &mut pl.lines {
        l.top = 0.0;
        l.height = 0.0;
        l.baseline = 0.0;
    }
    pl.height = 0.0;
    pl.rp.space_before = 0.0;
    pl.rp.space_after = 0.0;
    pl.label = None;
    // Its shading and borders would be drawn across the page at no height.
    pl.rp.shading = None;
    pl.rp.borders = None;
}
