use super::cross_refs::{caption_label, is_caption};
use super::render::entry;
use super::sources::bib_order;
use super::styles::style;
use super::{
    Block, CharProps, CmdError, CmdResult, Document, FieldCode, InlineObject, Paragraph, Path, Pos, Session, SortKey, StoryRef, TabAlign, TabLeader,
    TabStop, Value, delete_selection, heading_letter, p, sel_result, sort_key,
};
use std::sync::Arc;

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

/// Most paragraphs a list held as a field around its entries spans.
const MAX_WRAPPED: usize = 10_000;

/// A generated list (index, bibliography, table of figures or authorities) held as a complex
/// field that starts at body block `i` and runs over its entries: its field code and the block
/// it ends in. `None` for anything else (a TOC has its own update).
fn wrapped_list(s: &Session, i: usize) -> Option<(String, usize, usize)> {
    let p = s.doc.body.get(i)?.as_para()?;
    let (k, off, instr) = p.objects_with_offsets().enumerate().find_map(|(k, (off, o))| match o {
        InlineObject::FieldStart { instr, .. } => {
            let t = instr.trim_start();
            (t.starts_with("BIBLIOGRAPHY") || t.starts_with("INDEX") || t.starts_with("TOA") || figures_label(t).is_some())
                .then(|| (k, off, t.to_string()))
        }
        _ => None,
    })?;
    // Only when it starts the paragraph: text before it is the document's, not the list's.
    if !p.text.get(..off).is_some_and(|t| t.chars().all(|c| c.is_whitespace() || c == wordcraft_doc::para::OBJ)) {
        return None;
    }
    // Where it ends: the field end that closes it, nested fields counted from its own start (a
    // field before it in its paragraph, an empty `PAGE`…, closes before it and isn't it).
    let mut depth = 0usize;
    for (j, b) in s.doc.body.iter().enumerate().skip(i).take(MAX_WRAPPED) {
        let Some(q) = b.as_para() else { continue };
        for (at, o) in q.objects_with_offsets().skip(if j == i { k } else { 0 }) {
            match o {
                InlineObject::FieldStart { .. } => depth += 1,
                InlineObject::FieldEnd => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some((instr, j, at + wordcraft_doc::para::OBJ.len_utf8()));
                    }
                }
                _ => {}
            }
        }
    }
    None
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
fn captioned_as(doc: &Document, p: &Paragraph, ident: &str) -> bool {
    if !is_caption(doc, p) {
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
        // A list held as a field around its entries (as Word and ODT files hold them): made the
        // field's own paragraph, as WordCraft inserts one, its old entries rebuilt below.
        // What follows its closing field end, and its first paragraph's properties and section
        // break, are the document's and stay.
        if let Some((instr, last, end)) = wrapped_list(s, i) {
            // The entries between its first and last paragraphs go.
            for _ in i.saturating_add(1)..last {
                s.doc.remove_block(StoryRef::Body, &Path::top(i + 1))?;
            }
            // Its first paragraph keeps only the field; its last (or, ending in its first, the
            // same) loses what is up to and through the field end, and goes when nothing is left
            // of it and it ends no section.
            let first_end = if last > i {
                let q = s.doc.para_mut(StoryRef::Body, &Path::top(i + 1))?;
                q.delete(0, end)?;
                if q.is_empty() && q.section.is_none() {
                    s.doc.remove_block(StoryRef::Body, &Path::top(i + 1))?;
                }
                s.doc.para_mut(StoryRef::Body, &Path::top(i))?.len()
            } else {
                end
            };
            let p = s.doc.para_mut(StoryRef::Body, &Path::top(i))?;
            p.delete(0, first_end)?;
            p.insert_object(
                0,
                InlineObject::Field { instr, result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
                &Default::default(),
            )?;
            // What followed the field end in its own paragraph comes after the list, in a
            // paragraph of its own that takes the section break.
            let field = wordcraft_doc::para::OBJ.len_utf8();
            if p.len() > field {
                let rest = p.split_off(field)?;
                s.doc.insert_block(StoryRef::Body, &Path::top(i + 1), Block::Para(rest))?;
            }
        }
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
        Arc::make_mut(&mut s.doc.styles).upsert(wordcraft_doc::Style {
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
            if captioned_as(&s.doc, p, ident) {
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
                if captioned_as(&s.doc, p, ident) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A field before a list's in its paragraph (an empty `PAGE`) closes first: the list still
    /// ends at its own field end, paragraphs below, not in its first paragraph.
    #[test]
    fn a_wrapped_list_ends_at_its_own_field_end() {
        let mut d = Document::from_text("Old one\nOld two\nAfter");
        let props = CharProps::default();
        if let Ok(p) = d.para_mut(StoryRef::Body, &Path::top(0)) {
            p.insert_object(0, InlineObject::FieldStart { instr: "BIBLIOGRAPHY".into(), locked: false, dirty: false }, &props).unwrap();
            p.insert_object(0, InlineObject::FieldEnd, &props).unwrap();
            p.insert_object(0, InlineObject::FieldStart { instr: "PAGE".into(), locked: false, dirty: false }, &props).unwrap();
        }
        if let Ok(p) = d.para_mut(StoryRef::Body, &Path::top(1)) {
            let n = p.len();
            p.insert_object(n, InlineObject::FieldEnd, &props).unwrap();
        }
        let s = Session::new(d);
        assert_eq!(wrapped_list(&s, 0).map(|(instr, last, _)| (instr, last)), Some(("BIBLIOGRAPHY".to_string(), 1)));
    }

    /// A wrapped list as a session holds it: the field starting paragraph `first`'s text, ending
    /// in paragraph `last` at byte `at`.
    fn wrapped(text: &str, first: usize, last: usize, at: usize) -> Session {
        let mut d = Document::from_text(text);
        let props = CharProps::default();
        if let Ok(p) = d.para_mut(StoryRef::Body, &Path::top(last)) {
            p.insert_object(at, InlineObject::FieldEnd, &props).unwrap();
        }
        if let Ok(p) = d.para_mut(StoryRef::Body, &Path::top(first)) {
            p.insert_object(0, InlineObject::FieldStart { instr: "BIBLIOGRAPHY".into(), locked: false, dirty: false }, &props).unwrap();
        }
        Session::new(d)
    }

    fn texts(s: &Session) -> Vec<String> {
        s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect()
    }

    /// What follows a wrapped list's closing field end, in its last paragraph or in its only one,
    /// stays, after the list; the last paragraph's section break stays too.
    #[test]
    fn what_follows_a_wrapped_lists_end_stays() {
        // Ending in a later paragraph, with text after the end and a section break.
        let mut s = wrapped("Old one\nOld two\nOld three. After\nBelow", 0, 2, "Old three.".len());
        if let Ok(p) = s.doc.para_mut(StoryRef::Body, &Path::top(2)) {
            p.section = Some(Box::default());
        }
        if let Ok(p) = s.doc.para_mut(StoryRef::Body, &Path::top(0)) {
            p.props.indent_left = Some(9.0);
        }
        update_generated(&mut s).unwrap();
        let t = texts(&s);
        assert!(t.iter().all(|t| !t.contains("Old")), "{t:?}");
        assert!(t.iter().any(|t| t == " After"), "{t:?}");
        assert_eq!(t.last().map(String::as_str), Some("Below"));
        let after = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.plain_text() == " After");
        assert!(after.is_some_and(|p| p.section.is_some()), "the section break went");
        let first = s.doc.body.first().and_then(|b| b.as_para());
        assert!(first.is_some_and(|p| p.props.indent_left == Some(9.0) && list_kind(p).is_some()), "{first:?}");
        // An empty last paragraph without a section break goes.
        let mut s = wrapped("Old one\nOld two\nBelow", 0, 1, "Old two".len());
        update_generated(&mut s).unwrap();
        // (The field's paragraph shows nothing: there are no sources to list.)
        assert_eq!(texts(&s), ["", "Below"]);
        // Ending in its own paragraph: what follows comes after the list.
        let mut s = wrapped("Old entry. Then this\nBelow", 0, 0, "Old entry.".len());
        update_generated(&mut s).unwrap();
        let t = texts(&s);
        assert!(t.first().is_some_and(|t| !t.contains("Old")), "{t:?}");
        assert_eq!(t.get(1).map(String::as_str), Some(" Then this"), "{t:?}");
        assert_eq!(t.last().map(String::as_str), Some("Below"));
    }
}
