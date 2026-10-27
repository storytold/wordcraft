use super::*;

/// A level range such as `1-3` (or one level, `2`), clamped to 1–9.
pub fn level_range(s: &str) -> Option<(u8, u8)> {
    let mut it = s.split('-').map(|x| x.trim().parse::<u8>().ok());
    let a = it.next().flatten()?.clamp(1, 9);
    let b = match it.next() {
        Some(b) => b?.clamp(1, 9),
        None => a,
    };
    Some((a.min(b), a.max(b)))
}

/// The tab leader named in a command parameter.
pub fn leader(name: &str) -> TabLeader {
    match name {
        "none" => TabLeader::None,
        "hyphen" => TabLeader::Hyphen,
        "underscore" => TabLeader::Underscore,
        "middleDot" => TabLeader::MiddleDot,
        _ => TabLeader::Dot,
    }
}

/// The TOC field instruction for `references.toc`'s parameters (ECMA-376 §17.16.5.68): `\o`
/// heading styles, `\h` hyperlinks, `\z` hide tab and page numbers in Web view, `\u` outline
/// levels, `\f`/`\l` TC fields, `\n` without page numbers, `\p` separator, `\t` styles with levels.
pub fn toc_instr(v: &Value) -> String {
    let levels = p::u64(v, "levels").unwrap_or(3).clamp(1, 9);
    let on = |k: &str, d: bool| p::bool(v, k).unwrap_or(d);
    let mut s = String::from("TOC");
    if on("headingStyles", true) {
        s.push_str(&format!(" \\o \"1-{levels}\""));
    }
    if on("hyperlinks", true) {
        s.push_str(" \\h");
    }
    s.push_str(" \\z");
    if on("outlineLevels", true) {
        s.push_str(" \\u");
    }
    if on("tcFields", false) {
        s.push_str(&format!(" \\f \\l \"1-{levels}\""));
    }
    if !on("pageNumbers", true) {
        s.push_str(&format!(" \\n \"1-{levels}\""));
    } else if !on("rightAlign", true) {
        s.push_str(" \\p \" \"");
    }
    let clean = |n: &str| n.chars().filter(|c| !matches!(c, '"' | ',' | '\\')).take(64).collect::<String>().trim().to_string();
    let pairs: Vec<String> = match v.get("styleLevels") {
        Some(Value::Object(m)) => m
            .iter()
            .take(64)
            .filter_map(|(name, l)| Some((clean(name), l.as_u64()?.clamp(1, 9))))
            .filter(|(n, _)| !n.is_empty())
            .map(|(n, l)| format!("{n},{l}"))
            .collect(),
        _ => Vec::new(),
    };
    if !pairs.is_empty() {
        s.push_str(&format!(" \\t \"{}\"", pairs.join(",")));
    }
    s
}

/// A TOC entry: the paragraph it points at, its level (0-based), its text, and the leading page
/// breaks of that paragraph (its page is where the text after them lands).
struct TocEntry {
    path: Path,
    level: u8,
    text: String,
    lead: usize,
}

/// The headings of the body, in document order, as (body block, level 1–9, text, leading page
/// breaks): what Insert › Link › Place in This Document lists.
pub(crate) fn headings(s: &Session) -> Vec<(usize, u8, String, usize)> {
    let f = FieldCode::parse("TOC \\o \"1-9\" \\u");
    toc_entries(s, &f).into_iter().filter(|e| e.path.depth() == 0).map(|e| (e.path.last(), e.level.saturating_add(1), e.text, e.lead)).collect()
}

/// The heading level (1–9) of a built-in heading style.
pub(super) fn heading_style_level(p: &Paragraph) -> Option<u8> {
    p.props.style.as_deref()?.strip_prefix("Heading")?.trim().parse::<u8>().ok().filter(|l| (1..=9).contains(l))
}

/// The entries a TOC field collects from the body, in document order: built-in heading styles
/// in the `\o` range, paragraphs with an outline level (`\u`), styles listed in `\t`, and TC
/// fields (`\f`, levels `\l`).
fn toc_entries(s: &Session, f: &FieldCode) -> Vec<TocEntry> {
    let o = f.has('o').then(|| f.arg('o').and_then(level_range).unwrap_or((1, 9)));
    let max = o.map(|r| r.1).unwrap_or(9);
    let styles: Vec<(String, u8)> = f
        .arg('t')
        .map(|t| {
            let parts: Vec<&str> = t.split([',', ';']).map(str::trim).collect();
            parts.chunks(2).filter_map(|c| Some((c.first()?.to_string(), c.get(1)?.parse::<u8>().ok()?.clamp(1, 9)))).collect()
        })
        .unwrap_or_default();
    let tc = f.has('f').then(|| f.arg('f').unwrap_or("").to_ascii_uppercase());
    let tc_levels = f.arg('l').and_then(level_range).unwrap_or((1, 9));
    let mut v = Vec::new();
    // Every numbered paragraph advances the list counters, so an entry carries the number its
    // heading shows (`1.1`), followed by the level's separator, as Word writes it.
    let mut counters = Counters::default();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let number = p
            .props
            .numbering
            .or_else(|| s.doc.styles.resolve_para(&p.props).numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&s.doc.numbering, n.num, n.level))
            .filter(|(label, def)| !label.is_empty() && !matches!(def.format, NumFormat::Bullet | NumFormat::None))
            .map(|(label, def)| match def.suffix {
                LevelSuffix::Tab => format!("{label}\t"),
                LevelSuffix::Space => format!("{label} "),
                LevelSuffix::Nothing => label,
            });
        let style = p.props.style.as_deref().unwrap_or("Normal");
        if style.starts_with("TOC") {
            continue;
        }
        let lead = p.text.bytes().take_while(|b| *b == 0x0C).count();
        let by_style =
            styles.iter().find(|(n, _)| s.doc.styles.get(style).is_some_and(|st| st.name.eq_ignore_ascii_case(n)) || n == style).map(|(_, l)| *l);
        // `Some(9)` direct outline level is body text: "Do Not Show in Table of Contents".
        let by_outline = f.has('u').then_some(p.props.outline_level).flatten().filter(|l| *l < 9).map(|l| l + 1).filter(|l| *l <= max);
        let hidden = f.has('u') && p.props.outline_level == Some(9);
        let by_heading = o.and_then(|(a, b)| {
            let l = heading_style_level(p).or_else(|| {
                s.doc
                    .styles
                    .resolve_para(&wordcraft_doc::ParaProps { style: p.props.style.clone(), ..Default::default() })
                    .outline_level
                    .map(|l| l + 1)
            })?;
            (a..=b).contains(&l).then_some(l)
        });
        if let Some(l) = by_style.or(by_outline).or(if hidden { None } else { by_heading }) {
            let t = p.plain_text().replace(['\u{000C}', '\n', '\t'], " ").trim().to_string();
            if !t.is_empty() {
                let text = number.map_or_else(|| t.clone(), |n| format!("{n}{t}"));
                v.push(TocEntry { path: path.clone(), level: l - 1, text, lead });
            }
        }
        if let Some(id) = &tc {
            for off in p.object_offsets() {
                let Some(InlineObject::Field { instr, .. }) = p.object_at(off) else { continue };
                let c = FieldCode::parse(instr);
                if c.name != "TC" {
                    continue;
                }
                let kind = c.arg('f').unwrap_or("C").to_ascii_uppercase();
                let l = c.arg('l').and_then(|l| l.trim().parse::<u8>().ok()).unwrap_or(1).clamp(1, 9);
                let text = c.args.first().map(|t| t.trim().to_string()).unwrap_or_default();
                if (id.is_empty() || *id == kind) && (tc_levels.0..=tc_levels.1).contains(&l) && !text.is_empty() {
                    v.push(TocEntry { path: path.clone(), level: l - 1, text, lead: off });
                }
            }
        }
    }
    v
}

/// The TOC block range in the body: the paragraph holding the TOC field and the TOC entries after
/// it. A TOC with `\c` is a table of figures, kept by the generated lists.
pub(super) fn toc_range(s: &Session) -> Option<(usize, usize)> {
    let start = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| toc_field(p).is_some()))?;
    let mut end = start;
    while let Some(Block::Para(p)) = s.doc.body.get(end + 1).map(|b| &**b) {
        if p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading") {
            end += 1;
        } else {
            break;
        }
    }
    Some((start, end))
}

/// Removes the TOC blocks `start..=end` and returns where the TOC was. A field paragraph that ends
/// a section stays, emptied, so its section break isn't lost (#3889). A heading paragraph of its
/// own just above the field's paragraph (Word's tables start the field after their heading) goes
/// with the table.
pub(super) fn remove_toc(s: &mut Session, start: usize, end: usize) -> Result<usize, CmdError> {
    for i in (start + 1..=end).rev() {
        s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
    }
    let at = match s.doc.para_mut(StoryRef::Body, &Path::top(start)) {
        Ok(p) if p.section.is_some() => {
            *p = Paragraph { section: p.section.take(), ..Paragraph::new() };
            start + 1
        }
        _ => {
            s.doc.remove_block(StoryRef::Body, &Path::top(start))?;
            start
        }
    };
    let titled = start
        .checked_sub(1)
        .and_then(|i| s.doc.body.get(i))
        .and_then(|b| b.as_para())
        .is_some_and(|p| p.props.style.as_deref() == Some("TOCHeading") && p.section.is_none() && toc_field(p).is_none());
    if titled {
        s.doc.remove_block(StoryRef::Body, &Path::top(start - 1))?;
        return Ok(at - 1);
    }
    Ok(at)
}

/// The caret is in the table of contents (its heading or an entry), where Word's context menu
/// offers Update Field for the whole table.
pub fn caret_in_toc(s: &Session) -> bool {
    let at = &s.sel.focus;
    if at.story != StoryRef::Body || at.path.depth() != 0 {
        return false;
    }
    if toc_range(s).is_some_and(|(start, end)| (start..=end).contains(&at.path.last())) {
        return true;
    }
    // A saved or imported TOC is a range field spanning its entries until Update Table turns it
    // into the engine's TOC (#2182, a save and reopen; not the flow in the report). Only the
    // range Update Table would convert counts, so the menu never refreshes a different TOC.
    s.doc.field_ranges(StoryRef::Body).into_iter().find(|r| convertible_toc_range(s, r)).is_some_and(|r| {
        // The caret at the field's own marker (offset 0 of the first entry) and at the end of
        // the last entry belong to the range as well.
        (r.start.path.last()..=r.end.path.last()).contains(&at.path.last())
    })
}

/// The range field `range_toc_to_field` converts: an unlocked, top-level TOC (not a table of
/// figures) when the body has no engine TOC yet.
fn convertible_toc_range(s: &Session, r: &wordcraft_doc::FieldRange) -> bool {
    let field = FieldCode::parse(&r.instr);
    toc_range(s).is_none()
        && field.name == "TOC"
        && !field.is_table_of_figures()
        && r.depth == 0
        && !r.locked
        && r.start.path.depth() == 0
        && r.end.path.depth() == 0
}

/// A TOC imported as a range field (`FieldStart`…`FieldEnd`: one without a TOC heading, or with
/// content around it the engine's TOC can't reproduce) becomes the engine's TOC before an update,
/// so Update Table refreshes it rather than adding a second one: an empty `TOC` field where the
/// range started, its old result gone, and the regenerated entries after that paragraph (text
/// that followed the range moves below them). Only an unlocked, top-level TOC that isn't inside
/// another range field, and only when the body has no engine TOC already.
fn range_toc_to_field(s: &mut Session) -> Result<(), CmdError> {
    if toc_range(s).is_some() {
        return Ok(());
    }
    let Some(r) = s.doc.field_ranges(StoryRef::Body).into_iter().find(|r| convertible_toc_range(s, r)) else {
        return Ok(());
    };
    let Some(after_end) = r.end.off.checked_add(OBJ.len_utf8()) else { return Ok(()) };
    let end = Pos { off: after_end, ..r.end.clone() };
    let props = s.doc.para_at(&r.start).map(|p| p.props_of_char(r.start.off).clone()).unwrap_or_default();
    let mut at = s.doc.delete_range(&r.start, &end)?;
    // A range that filled its own paragraphs right under a TOC heading (as Word and other writers
    // lay it out) leaves an empty paragraph behind, which would show as a blank line above the
    // entries. The field goes at the end of the heading instead, as `insert_toc` places it.
    let i = at.path.last();
    // Or one holding only the end of Word's TOC control, still styled as an entry: Update Table
    // would delete it with the old entries anyway.
    let leftover = |p: &Paragraph| {
        p.is_empty()
            || (p.objects.iter().all(|o| matches!(o, InlineObject::ControlEnd))
                && p.text.chars().all(|c| c == OBJ)
                && p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
    };
    if at.path.depth() == 0
        && i > 0
        && s.doc.para_at(&at).is_some_and(leftover)
        // The paragraph before may also be the empty one the writer leaves where the engine's TOC
        // field was (the field is saved opening in the first entry), so the next save and update
        // doesn't add a blank line each time (#2182). A blank line the user typed right above a
        // range TOC is reused the same way, so deleting it later deletes the TOC field. So is one
        // ending the section before, as above entries in columns (#3889).
        && s.doc.body.get(i - 1).and_then(|b| b.as_para()).is_some_and(|p| {
            p.props.style.as_deref() == Some("TOCHeading") || (p.is_empty() && p.objects.is_empty())
        })
    {
        s.doc.remove_block(StoryRef::Body, &at.path)?;
        let heading = Path::top(i - 1);
        let len = s.doc.para(StoryRef::Body, &heading).map_or(0, |p| p.len());
        at = Pos { story: StoryRef::Body, path: heading, off: len };
    }
    let field = InlineObject::Field {
        instr: r.instr.trim().to_string(),
        result: String::new(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let after = s.doc.insert_object(&at, field, &props)?;
    if s.doc.para_at(&after).is_some_and(|p| after.off < p.len()) {
        s.doc.split_paragraph(&after)?;
    }
    s.clamp_selection();
    Ok(())
}

/// The TOC field instruction in `p` (not a table of figures).
fn toc_field(p: &Paragraph) -> Option<String> {
    p.objects.iter().find_map(|o| match o {
        InlineObject::Field { instr, .. } => {
            let f = FieldCode::parse(instr);
            (f.name == "TOC" && !f.is_table_of_figures()).then(|| instr.clone())
        }
        _ => None,
    })
}

/// The first cached number-separator tab formatting at each TOC level.
pub(super) fn cached_toc_tabs(s: &Session) -> Vec<(String, Vec<TabStop>)> {
    let span = toc_range(s).map(|(start, end)| (start.saturating_add(1), end)).or_else(|| {
        s.doc.field_ranges(StoryRef::Body).into_iter().find_map(|r| {
            let f = FieldCode::parse(&r.instr);
            (r.depth == 0 && !r.locked && r.start.path.depth() == 0 && r.end.path.depth() == 0 && f.name == "TOC" && !f.is_table_of_figures())
                .then_some((r.start.path.last(), r.end.path.last()))
        })
    });
    let Some(blocks) = span.and_then(|(start, end)| s.doc.body.get(start..=end)) else { return Vec::new() };
    let mut tabs: Vec<(String, Vec<TabStop>)> = Vec::new();
    for p in blocks.iter().filter_map(|b| b.as_para()) {
        let Some(style) = &p.props.style else { continue };
        let Some(direct) = &p.props.tabs else { continue };
        if !direct.iter().any(|t| matches!(t.align, TabAlign::Left | TabAlign::Center | TabAlign::Decimal)) {
            continue;
        }
        if !style.starts_with("TOC") || style == "TOCHeading" || tabs.iter().any(|(id, _)| id == style) {
            continue;
        }
        tabs.push((style.clone(), direct.clone()));
    }
    tabs
}

pub(super) fn insert_toc(s: &mut Session, v: &Value) -> CmdResult {
    let tabs = cached_toc_tabs(s);
    range_toc_to_field(s)?;
    let replace = v.as_object().is_some_and(|m| !m.is_empty());
    // Replacing a table puts the new one where the old one was, in place of an empty paragraph.
    let mut placeholder = false;
    let at = match toc_range(s) {
        // `{}` updates the table there; settings replace it where it is.
        Some(_) if !replace => {
            regenerate_toc(s, None, &tabs)?;
            return sel_result(s);
        }
        Some((start, end)) => {
            let start = remove_toc(s, start, end)?;
            s.doc.insert_block(StoryRef::Body, &Path::top(start), Block::Para(Paragraph::new()))?;
            placeholder = true;
            s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path::top(start), off: 0 });
            s.sel.focus.clone()
        }
        None => delete_selection(s)?,
    };
    let title = p::str(v, "title").unwrap_or("Contents").to_string();
    if at.story != StoryRef::Body || at.path.depth() > 0 {
        return Err(CmdError::Failed("a table of contents goes in the main text".into()));
    }
    // Title paragraph carrying the TOC field marker.
    let mut head = Paragraph::with_text(&title, CharProps::default()).styled("TOCHeading");
    head.insert_object(
        head.len(),
        InlineObject::Field { instr: toc_instr(v), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    )?;
    let i = if s.doc.para_at(&at).is_some_and(|p| p.is_empty()) {
        if placeholder {
            s.doc.remove_block(StoryRef::Body, &at.path)?;
        }
        at.path.last()
    } else {
        s.doc.split_paragraph(&at)?.path.last()
    };
    s.doc.insert_block(StoryRef::Body, &Path::top(i), Block::Para(head))?;
    let leader = leader(p::str(v, "tabLeader").unwrap_or("dot"));
    regenerate_toc(s, Some(leader), &tabs)?;
    s.clamp_selection();
    sel_result(s)
}

/// A hidden `_Toc` bookmark around the heading at body block `i` (after its leading page breaks),
/// reusing one already there; its name.
pub(crate) fn toc_bookmark(s: &mut Session, i: usize, lead: usize) -> Result<String, CmdError> {
    bookmark_heading(s, &Path::top(i), lead)
}

fn bookmark_heading(s: &mut Session, path: &Path, lead: usize) -> Result<String, CmdError> {
    const MARK: usize = wordcraft_doc::para::OBJ.len_utf8();
    let para = s.doc.para(StoryRef::Body, path).ok_or_else(|| CmdError::Failed("heading is gone".into()))?;
    if let Some(InlineObject::BookmarkStart { name }) = para.object_at(lead)
        && name.starts_with("_Toc")
    {
        return Ok(name.clone());
    }
    let next =
        s.doc.bookmarks().iter().filter_map(|(n, _)| n.strip_prefix("_Toc").and_then(|d| d.parse::<u64>().ok())).max().unwrap_or(0).saturating_add(1);
    let name = format!("_Toc{next:09}");
    let end = para.len();
    let props = CharProps::default();
    let pos = |off| Pos { story: StoryRef::Body, path: path.clone(), off };
    s.doc.insert_object(&pos(end), InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&pos(lead), InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    // The caret in that paragraph stays on the same text.
    let shift = |p: &mut Pos| {
        if p.story == StoryRef::Body && &p.path == path {
            if p.off > end {
                p.off = p.off.saturating_add(2 * MARK);
            } else if p.off >= lead && p.off > 0 {
                p.off = p.off.saturating_add(MARK);
            }
        }
    };
    shift(&mut s.sel.anchor);
    shift(&mut s.sel.focus);
    Ok(name)
}

/// Regenerate the TOC entries with page numbers (two layout passes: entries change pagination).
pub fn update_toc(s: &mut Session) -> Result<(), CmdError> {
    let tabs = cached_toc_tabs(s);
    regenerate_toc(s, None, &tabs)
}

/// Rebuild the entries the TOC field's switches ask for. The tab leader is `leader`, or the one
/// the entries had (Word keeps it in the entries' tab stops, not in the field).
fn regenerate_toc(s: &mut Session, leader: Option<TabLeader>, tabs: &[(String, Vec<TabStop>)]) -> Result<(), CmdError> {
    range_toc_to_field(s)?;
    let Some((start, end)) = toc_range(s) else { return Ok(()) };
    let Some(instr) = s.doc.body.get(start).and_then(|b| b.as_para()).and_then(toc_field) else { return Ok(()) };
    // A TOC field imported with an inline placeholder/cached result must have its result cleared so
    // that the engine's TOC shape matches `toc_span` and the DOCX writer spans the field across entries.
    if let Ok(p) = s.doc.para_mut(StoryRef::Body, &Path::top(start)) {
        for o in &mut p.objects {
            if let InlineObject::Field { instr, result, .. } = o {
                let f = FieldCode::parse(instr);
                if f.name == "TOC" && !f.is_table_of_figures() {
                    result.clear();
                }
            }
        }
        p.touch();
    }
    let f = FieldCode::parse(&instr);
    let leader = leader.unwrap_or_else(|| {
        s.doc
            .body
            .get(start + 1)
            .filter(|_| end > start)
            .and_then(|b| b.as_para())
            .and_then(|p| p.props.tabs.as_ref()?.last().map(|t| t.leader))
            .unwrap_or(TabLeader::Dot)
    });
    for i in (start + 1..=end).rev() {
        s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
    }
    // The column width of the TOC's own section, not the caret's: after Select All the caret sits
    // at the end of the document, possibly in a wider (landscape) section. In columns, the first
    // column's, as Word does (#3889); after a field paragraph ending a section, the next section's.
    let after_break = s.doc.body.get(start).and_then(|b| b.as_para()).is_some_and(|p| p.section.is_some());
    let section = s.doc.section_index_of(start + usize::from(after_break));
    let column = |x: &wordcraft_doc::SectionProps| x.column_boxes().first().map_or(x.text_width(), |(_, w)| *w);
    let width = s.doc.sections().get(section).map_or_else(|| column(&crate::cmd::page::sect(s)), |(_, x)| column(x));
    let entries = toc_entries(s, &f);
    let links: Vec<Option<String>> = if f.has('h') {
        let mut v = Vec::new();
        for e in &entries {
            // TC fields link to their paragraph's start.
            let lead = s.doc.para(StoryRef::Body, &e.path).map_or(0, |p| p.text.bytes().take_while(|b| *b == 0x0C).count());
            v.push(Some(format!("#{}", bookmark_heading(s, &e.path, lead)?)));
        }
        v
    } else {
        vec![None; entries.len()]
    };
    let no_pages = f.has('n').then(|| f.arg('n').and_then(level_range).unwrap_or((1, 9)));
    let sep = f.arg('p').map(str::to_string).unwrap_or_else(|| "\t".into());
    for pass in 0..2 {
        // Pass 1 lays out with the entries in place, so headings below the TOC sit `entries.len()`
        // blocks further down and their pages account for the TOC's own length.
        s.touch();
        let l = s.layout();
        if pass == 1 {
            // Replace the page numbers now that the entries exist.
            for _ in 0..entries.len() {
                s.doc.remove_block(StoryRef::Body, &Path::top(start + 1))?;
            }
        }
        let shift = if pass == 0 { 0 } else { entries.len() };
        for (k, e) in entries.iter().enumerate() {
            let mut target = e.path.clone();
            if let Some(top) = target.0.first_mut()
                && (*top as usize) > start
            {
                *top = top.saturating_add(u32::try_from(shift).unwrap_or(u32::MAX));
            }
            let page =
                l.caret(&Pos { story: StoryRef::Body, path: target, off: e.lead }).and_then(|c| l.pages.get(c.page)).map(|pg| pg.number).unwrap_or(1);
            let show_page = no_pages.is_none_or(|(a, b)| !(a..=b).contains(&(e.level + 1)));
            let text = if show_page { format!("{}{sep}{page}", e.text) } else { e.text.clone() };
            let props = CharProps { link: links.get(k).cloned().flatten(), ..Default::default() };
            let mut para = Paragraph::with_text(&text, props).styled(&format!("TOC{}", e.level + 1));
            para.mark = CharProps::default();
            if e.text.contains('\t') {
                para.props.tabs = tabs.iter().find(|(style, _)| Some(style) == para.props.style.as_ref()).map(|(_, tabs)| tabs.clone());
            }
            if show_page && sep.contains('\t') {
                let right = TabStop { pos: width - 0.5, align: TabAlign::Right, leader };
                para.props.tabs = Some(entry_tabs(s, &para.props, &e.text, right));
            }
            s.doc.insert_block(StoryRef::Body, &Path::top(start + 1 + k), Block::Para(para))?;
        }
    }
    s.touch();
    Ok(())
}

/// The tab stops of a TOC entry: the right-aligned stop for its page number, and, when the
/// entry text itself has no tab, Clear stops for the left, centre and decimal stops its style
/// sets before that. Those stops are for a number set off by a tab (`1.1<tab>Title`); with the
/// number set off by a space, or no number, a short entry's page tab would stop at one of them
/// (without the leader), and so would the tab on the last line of an entry that wraps.
fn entry_tabs(s: &Session, props: &wordcraft_doc::ParaProps, text: &str, right: TabStop) -> Vec<TabStop> {
    let mut tabs = props.tabs.clone().unwrap_or_default();
    tabs.retain(|t| t.pos < right.pos - 0.5 && !matches!(t.align, TabAlign::Right | TabAlign::Bar));
    if !text.contains('\t') {
        let style = s.doc.styles.resolve_para(&wordcraft_doc::ParaProps { style: props.style.clone(), ..Default::default() });
        for t in style.tabs {
            if t.pos < right.pos - 0.5 && !matches!(t.align, TabAlign::Right | TabAlign::Bar) {
                tabs.push(TabStop { pos: t.pos, align: TabAlign::Clear, leader: TabLeader::None });
            }
        }
    }
    tabs.push(right);
    tabs
}
