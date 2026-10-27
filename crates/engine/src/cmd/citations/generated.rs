use super::cross_refs::caption_label;
use super::render::entry;
use super::sources::bib_order;
use super::styles::style;
use super::*;

/// The TOC field for `references.tableOfFigures` (ECMA-376 §17.16.5.68): `\c "<label>"` lists the
/// captions that label's SEQ field numbers, `\a "<label>"` lists them without their label and
/// number; `\n` leaves page numbers out and `\p " "` puts them right after the text.
pub fn figures_instr(v: &Value) -> String {
    let label: String = p::str(v, "label").unwrap_or("Figure").chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(40).collect();
    let ident = seq_ident(&label);
    let ident = if ident.is_empty() { "Figure".to_string() } else { ident };
    let on = |k: &str| p::bool(v, k).unwrap_or(true);
    let mut s = format!("TOC \\{} \"{ident}\"", if on("includeLabel") { 'c' } else { 'a' });
    if !on("pageNumbers") {
        s.push_str(" \\n");
    } else if !on("rightAlign") {
        s.push_str(" \\p \" \"");
    }
    s
}

/// The SEQ identifier of a caption label: Word joins a multi-word label with underscores.
fn seq_ident(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join("_")
}

pub(super) fn table_of_figures(s: &mut Session, v: &Value) -> CmdResult {
    generated_list_with(s, &figures_instr(v), "", p::str(v, "tabLeader").map(crate::cmd::references::leader))?;
    sel_result(s)
}

/// Generated lists (BIBLIOGRAPHY, INDEX, table of figures, TOA): a heading paragraph holding
/// the field, then entry paragraphs marked with the "GeneratedEntry" style family.
pub(crate) fn generated_list(s: &mut Session, instr: &str, title: &str) -> Result<(), CmdError> {
    generated_list_with(s, instr, title, None)
}

/// [`generated_list`] whose entries take `leader`. The leader lives in the entries' tab stops (as
/// in a table of contents) and an update keeps the first entry's, so a placeholder entry carries
/// it through the rebuild, however the lists above it change.
fn generated_list_with(s: &mut Session, instr: &str, title: &str, leader: Option<TabLeader>) -> Result<(), CmdError> {
    let at = delete_selection(s)?;
    if at.story != StoryRef::Body || at.path.depth() > 0 {
        return Err(CmdError::Failed("lists go in the main text".into()));
    }
    let mut head = if title.is_empty() { Paragraph::new() } else { Paragraph::with_text(title, CharProps::default()).styled("Heading1") };
    let end = head.len();
    head.insert_object(
        end,
        InlineObject::Field { instr: instr.to_string(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &CharProps::default(),
    )?;
    let i = if s.doc.para_at(&at).is_some_and(|p| p.is_empty()) { at.path.last() } else { s.doc.split_paragraph(&at)?.path.last() };
    s.doc.insert_block(StoryRef::Body, &Path::top(i), Block::Para(head))?;
    if let Some(leader) = leader {
        let mut seed = Paragraph::new().styled(ENTRY_STYLE);
        seed.props.tabs = Some(vec![TabStop { pos: 0.0, align: TabAlign::Right, leader }]);
        s.doc.insert_block(StoryRef::Body, &Path::top(i.saturating_add(1)), Block::Para(seed))?;
    }
    update_generated(s)
}

fn list_kind(p: &Paragraph) -> Option<String> {
    p.objects.iter().find_map(|o| match o {
        InlineObject::Field { instr, .. } => {
            let t = instr.trim_start();
            (t.starts_with("BIBLIOGRAPHY") || t.starts_with("INDEX") || t.starts_with("TOA") || figures_label(t).is_some()).then(|| t.to_string())
        }
        _ => None,
    })
}

/// The caption label (SEQ identifier) a table-of-figures field lists, `\c` or `\a`.
fn figures_label(instr: &str) -> Option<String> {
    let f = FieldCode::parse(instr);
    if !f.is_table_of_figures() {
        return None;
    }
    Some(f.arg('c').or_else(|| f.arg('a')).map_or_else(|| "Figure".to_string(), seq_ident))
}

/// Whether `p` is a caption numbered by SEQ identifier `ident` (any case, as Word matches it). A
/// caption typed by hand, without a SEQ field, goes by its label.
fn captioned_as(p: &Paragraph, ident: &str) -> bool {
    if p.props.style.as_deref() != Some("Caption") {
        return false;
    }
    let seq = p.objects.iter().find_map(|o| match o {
        InlineObject::Field { instr, .. } => Some(FieldCode::parse(instr)).filter(|f| f.name == "SEQ"),
        _ => None,
    });
    match seq {
        Some(f) => f.args.first().is_some_and(|id| id.eq_ignore_ascii_case(ident)),
        None => caption_label(p, &p.plain_text()).is_some_and(|l| seq_ident(&l).eq_ignore_ascii_case(ident)),
    }
}

/// A caption's text without its label and number (`\a`): what follows its SEQ field, or for a
/// caption typed by hand what follows the label and the number, less the separator.
fn caption_body(p: &Paragraph, label: &str) -> String {
    let seq_end = p.object_offsets().into_iter().find_map(|o| match p.object_at(o) {
        Some(InlineObject::Field { instr, .. }) if FieldCode::parse(instr).name == "SEQ" => Some(o + wordcraft_doc::para::OBJ.len_utf8()),
        _ => None,
    });
    let rest = match seq_end {
        Some(end) => {
            let mut k = 0;
            let mut rest = String::new();
            for (at, c) in p.text.char_indices() {
                if c == wordcraft_doc::para::OBJ {
                    if at >= end
                        && let Some(o) = p.objects.get(k)
                    {
                        rest.push_str(o.plain_text());
                    }
                    k += 1;
                } else if at >= end {
                    rest.push(c);
                }
            }
            rest
        }
        None => {
            let text = p.plain_text();
            // The label in any case, as captions are matched (ASCII case keeps the byte length).
            let start = text.trim_start();
            let after = start
                .get(..label.len())
                .filter(|head| head.eq_ignore_ascii_case(label))
                .and_then(|_| start.get(label.len()..))
                .unwrap_or(&text)
                .trim_start();
            // The number: the word after the label, when it starts with a digit.
            match after.find(char::is_whitespace) {
                Some(n) if after.starts_with(|c: char| c.is_ascii_digit()) => after[n..].to_string(),
                _ if after.starts_with(|c: char| c.is_ascii_digit()) => String::new(),
                _ => after.to_string(),
            }
        }
    };
    rest.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '.' | '-' | '\u{2013}' | '\u{2014}')).trim_end().to_string()
}

pub(super) const ENTRY_STYLE: &str = "ListEntry";

/// Rebuild every generated list.
pub fn update_generated(s: &mut Session) -> Result<(), CmdError> {
    let mut i = 0;
    while i < s.doc.body.len() {
        let kind = s.doc.body.get(i).and_then(|b| b.as_para()).and_then(list_kind);
        let Some(kind) = kind else {
            i += 1;
            continue;
        };
        add_entry_style(s);
        if kind.starts_with("INDEX") {
            crate::cmd::index::rebuild(s, i, None)?;
            i += 1;
            continue;
        }
        // A table of figures from Word can hold the list it saved as the field's result, in the
        // field's own paragraph: the rebuilt entries replace it, keeping its leader.
        let mut saved_leader = None;
        if figures_label(&kind).is_some()
            && let Ok(p) = s.doc.para_mut(StoryRef::Body, &Path::top(i))
        {
            let leader = p.props.tabs.as_ref().and_then(|t| t.last()).map(|t| t.leader);
            let mut cleared = false;
            for o in &mut p.objects {
                if let InlineObject::Field { instr, result, .. } = o
                    && !result.is_empty()
                    && figures_label(instr).is_some()
                {
                    result.clear();
                    cleared = true;
                }
            }
            if cleared {
                saved_leader = leader;
                p.touch();
            }
        }
        // Remove old entries, keeping their tab leader.
        let leader = s
            .doc
            .body
            .get(i + 1)
            .and_then(|b| b.as_para())
            .filter(|p| p.props.style.as_deref() == Some(ENTRY_STYLE))
            .and_then(|p| p.props.tabs.as_ref()?.last().map(|t| t.leader))
            .or(saved_leader)
            .unwrap_or(TabLeader::Dot);
        while s.doc.body.get(i + 1).and_then(|b| b.as_para()).is_some_and(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)) {
            s.doc.remove_block(StoryRef::Body, &Path::top(i + 1))?;
        }
        let lines = entries_for(s, &kind);
        let width = crate::cmd::page::sect(s).text_width();
        for (k, (text, hanging)) in lines.into_iter().enumerate() {
            let mut para = Paragraph::with_text(&text, CharProps::default()).styled(ENTRY_STYLE);
            if hanging {
                para.props.indent_left = Some(36.0);
                para.props.indent_first = Some(-36.0);
            }
            if text.contains('\t') {
                para.props.tabs = Some(vec![TabStop { pos: width - 0.5, align: TabAlign::Right, leader }]);
            }
            s.doc.insert_block(StoryRef::Body, &Path::top(i + 1 + k), Block::Para(para))?;
        }
        i += 1;
    }
    s.touch();
    Ok(())
}

/// Add the style generated list entries take, if the document doesn't have it yet. Only when a
/// list is there: a new style changes the formatting every paragraph is laid out with, so the
/// layout cache starts over (and the saved file would gain a style it doesn't use).
fn add_entry_style(s: &mut Session) {
    if s.doc.styles.get(ENTRY_STYLE).is_none() {
        s.doc.styles.upsert(wordcraft_doc::Style {
            id: ENTRY_STYLE.into(),
            name: "List Entry".into(),
            kind: wordcraft_doc::StyleKind::Paragraph,
            based_on: Some("Normal".into()),
            para: wordcraft_doc::ParaProps { space_after: Some(4.0), ..Default::default() },
            hidden: true,
            ..Default::default()
        });
    }
}

pub(super) fn page_of(s: &mut Session, pos: &Pos) -> u32 {
    let l = s.layout();
    l.caret(pos).and_then(|c| l.pages.get(c.page)).map(|p| p.number).unwrap_or(1)
}

/// Entry lines (text, hanging indent) for a generated list.
fn entries_for(s: &mut Session, kind: &str) -> Vec<(String, bool)> {
    if kind.starts_with("BIBLIOGRAPHY") {
        let st = style(&s.bib_style);
        return bib_order(&s.doc, st, crate::cmd::default_lang(s))
            .into_iter()
            .enumerate()
            .filter_map(|(k, i)| s.doc.sources.get(i).map(|x| (entry(x, st.name, k.saturating_add(1)), !st.numbered())))
            .collect();
    }
    // Collect marks with positions.
    let mut marks: Vec<(String, Pos)> = Vec::new();
    let figures = figures_label(kind);
    let bare = FieldCode::parse(kind).has('a');
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        if let Some(ident) = &figures {
            if captioned_as(p, ident) {
                let text = if bare { caption_body(p, &ident.replace('_', " ")) } else { p.plain_text() };
                marks.push((text, Pos { story: StoryRef::Body, path: path.clone(), off: 0 }));
            }
            continue;
        }
        for off in p.object_offsets() {
            if let Some(InlineObject::Field { instr, .. }) = p.object_at(off) {
                let t = instr.trim_start();
                let want = if kind.starts_with("INDEX") { "XE" } else { "TA" };
                if t.starts_with(want) {
                    let e = t.split('"').nth(1).unwrap_or("").to_string();
                    marks.push((e, Pos { story: StoryRef::Body, path: path.clone(), off }));
                }
            }
        }
    }
    let mut with_pages: Vec<(String, u32)> = marks.into_iter().map(|(e, _)| (e, 0)).collect();
    // Pages (layout once).
    let positions: Vec<Pos> = {
        let mut v = Vec::new();
        for path in s.doc.para_paths(StoryRef::Body) {
            let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
            if let Some(ident) = &figures {
                if captioned_as(p, ident) {
                    v.push(Pos { story: StoryRef::Body, path: path.clone(), off: 0 });
                }
                continue;
            }
            for off in p.object_offsets() {
                if let Some(InlineObject::Field { instr, .. }) = p.object_at(off) {
                    let want = if kind.starts_with("INDEX") { "XE" } else { "TA" };
                    if instr.trim_start().starts_with(want) {
                        v.push(Pos { story: StoryRef::Body, path: path.clone(), off });
                    }
                }
            }
        }
        v
    };
    for (i, pos) in positions.iter().enumerate() {
        let pg = page_of(s, pos);
        if let Some(x) = with_pages.get_mut(i) {
            x.1 = pg;
        }
    }
    if figures.is_some() {
        // `\n` without page numbers; `\p` the separator before them (a tab by default).
        let f = FieldCode::parse(kind);
        let sep = f.arg('p').unwrap_or("\t");
        return with_pages.into_iter().map(|(t, p)| (if f.has('n') { t } else { format!("{t}{sep}{p}") }, false)).collect();
    }
    // Index / TOA: group by entry, sort, list pages.
    let lang = crate::cmd::default_lang(s);
    let mut grouped: std::collections::BTreeMap<(SortKey, String), Vec<u32>> = Default::default();
    for (e, p) in with_pages {
        let pages = grouped.entry((sort_key(&e, lang.as_deref()), e)).or_default();
        if !pages.contains(&p) {
            pages.push(p);
        }
    }
    let mut out = Vec::new();
    let mut letter = None;
    for ((_, e), pages) in grouped {
        let first = heading_letter(&e, lang.as_deref());
        if kind.starts_with("INDEX") && first != letter {
            letter = first.clone();
            if let Some(l) = first {
                out.push((l, false));
            }
        }
        let list = pages.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
        out.push(if kind.starts_with("INDEX") { (format!("{e}, {list}"), true) } else { (format!("{e}\t{list}"), false) });
    }
    out
}
