//! Table of authorities as in Word: Mark Citation (`TA` fields with long and short citations and
//! a category, and Mark All), Insert Table of Authorities (one `TOA` table per category) and the
//! entries Update Table builds for each.

use super::generated::{page_of, update_generated};
use super::{
    Block, CharProps, CmdError, CmdResult, FieldCode, InlineObject, Paragraph, Path, Pos, Session, SortKey, StoryRef, Value, delete_selection, json,
    p, sel_result, sort_key,
};

/// Word's style for table of authorities entries, which a table read from Word already uses.
pub(super) const TOA_ENTRY_STYLE: &str = "TableofAuthorities";

/// Add Word's Table of Authorities style if the document doesn't have it: Word's own definition,
/// a 12 pt hanging indent for a citation that takes more than one line.
pub(super) fn add_toa_entry_style(s: &mut Session) {
    if s.doc.styles.get(TOA_ENTRY_STYLE).is_none() {
        s.doc.styles.upsert(wordcraft_doc::Style {
            id: TOA_ENTRY_STYLE.into(),
            name: "Table of Authorities".into(),
            kind: wordcraft_doc::StyleKind::Paragraph,
            based_on: Some("Normal".into()),
            next: Some("Normal".into()),
            para: wordcraft_doc::ParaProps { indent_left: Some(12.0), indent_first: Some(-12.0), ..Default::default() },
            ..Default::default()
        });
    }
}

/// With `\h` a table of authorities starts with its category's name, in the paragraph holding
/// its field (Word's TOA Heading paragraph, the field first): the text there becomes the name.
pub(super) fn toa_heading(s: &mut Session, i: usize, kind: &str) -> Result<(), CmdError> {
    let f = FieldCode::parse(kind);
    let category = toa_category(&f);
    if !f.has('h') || category == 0 {
        return Ok(());
    }
    let name = toa_category_name(category);
    let p = s.doc.para_mut(StoryRef::Body, &Path::top(i))?;
    let text: String = p.text.chars().filter(|c| *c != wordcraft_doc::para::OBJ).collect();
    if text == name {
        return Ok(());
    }
    // Delete the text after the last object (the field), back to front, and put the name there.
    let after = p.text.rfind(wordcraft_doc::para::OBJ).map_or(0, |o| o + wordcraft_doc::para::OBJ.len_utf8());
    let props = p.props_of_char(after.min(p.len().saturating_sub(1))).clone();
    let plain: Vec<(usize, usize)> =
        p.text.char_indices().filter(|(_, c)| *c != wordcraft_doc::para::OBJ).map(|(at, c)| (at, at + c.len_utf8())).collect();
    for (a, b) in plain.into_iter().rev() {
        p.delete(a, b)?;
    }
    let at = p.text.rfind(wordcraft_doc::para::OBJ).map_or(0, |o| o + wordcraft_doc::para::OBJ.len_utf8());
    p.insert_text(at, &name, &CharProps { hidden: None, ..props })?;
    p.touch();
    Ok(())
}

/// Word's names for table of authorities categories 1–7 (Mark Citation's list); 8–16 have none.
pub(super) fn toa_category_name(c: u32) -> String {
    match c {
        1 => "Cases",
        2 => "Statutes",
        3 => "Other Authorities",
        4 => "Rules",
        5 => "Treatises",
        6 => "Regulations",
        7 => "Constitutional Provisions",
        n => return n.to_string(),
    }
    .to_string()
}

/// A citation-mark field: a `TA` field with no result, so nothing shows. Not hidden text: Word for
/// Mac's Update Table leaves a mark formatted hidden out of the table (observed in Word for Mac
/// 16.114: "No table of authorities entries found." until the hidden formatting was removed).
pub(super) fn ta_field(instr: String) -> (InlineObject, CharProps) {
    (InlineObject::Field { instr, result: String::new(), locked: false, dirty: false, code: false, form: String::new() }, CharProps::default())
}

/// The most short-citation marks one Mark All adds: a one-character citation would otherwise
/// mark every place it appears in the document. A brief cites one authority far fewer times.
pub(super) const MARK_ALL_CAP: usize = 1000;

/// A citation as it goes in a `TA` field's quoted argument: whitespace runs as one space, and no
/// quotes or backslashes, which would end the argument or start a switch (`\c 3`, `\s`) when the
/// field is read again; control characters count as spaces.
fn clean_citation(t: &str) -> String {
    let kept: String = t.chars().filter(|c| !matches!(c, '"' | '\\')).map(|c| if c.is_control() { ' ' } else { c }).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Mark Citation: a `TA` field after the selection with the long citation, the short one and the
/// category, as Word's Mark writes it. With `all` (Mark All) also a short-citation mark (`TA \s`)
/// after every other place in the main text where the long or the short citation appears exactly
/// as typed (whole words) and isn't marked yet, as Microsoft documents Mark All, up to
/// [`MARK_ALL_CAP`] marks (`capped` says the cap was reached).
pub(super) fn mark_citation(s: &mut Session, v: &Value) -> CmdResult {
    let clean = clean_citation;
    let long = clean(&p::str(v, "long").or_else(|| p::str(v, "entry")).map(str::to_string).unwrap_or_else(|| s.selected_text()));
    if long.is_empty() {
        return Err(CmdError::Params("select a citation or give `long`".into()));
    }
    let short = p::str(v, "short").map(clean).filter(|x| !x.is_empty()).unwrap_or_else(|| long.clone());
    let category = p::u64(v, "category").unwrap_or(1).clamp(1, 16);
    let (a, b) = s.sel.ordered();
    let (field, props) = ta_field(format!("TA \\l \"{long}\" \\s \"{short}\" \\c {category}"));
    s.doc.insert_object(&b, field, &props)?;
    let mut marked = 1;
    let mut capped = false;
    if p::bool(v, "all").unwrap_or(false) {
        let (added, full) = mark_all(s, &long, &short, (&a, &b))?;
        marked += added;
        capped = full;
    }
    s.clamp_selection();
    s.touch();
    Ok(json!({"long": long, "short": short, "category": category, "marked": marked, "capped": capped}))
}

/// Mark All's short-citation marks, in document order: how many it added, and whether it stopped
/// at [`MARK_ALL_CAP`] with more left. The selection (`selected`) was just marked.
pub(super) fn mark_all(s: &mut Session, long: &str, short: &str, selected: (&Pos, &Pos)) -> Result<(usize, bool), CmdError> {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let mut added = 0;
    let mut capped = false;
    for path in s.doc.para_paths(StoryRef::Body) {
        if capped {
            break;
        }
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let mut hits: Vec<(usize, usize)> = Vec::new();
        for needle in [long, short] {
            for (at, m) in p.text.match_indices(needle) {
                let end = at + m.len();
                if !word(p.text[..at].chars().next_back()) && !word(p.text[end..].chars().next()) {
                    hits.push((at, end));
                }
            }
        }
        // The longest match at each place; none inside another.
        hits.sort_by_key(|(a, b)| (*a, std::cmp::Reverse(*b)));
        let mut kept: Vec<(usize, usize)> = Vec::new();
        for (a, b) in hits {
            if kept.last().is_none_or(|(_, end)| a >= *end) {
                kept.push((a, b));
            }
        }
        let in_selection = |a: usize, b: usize| {
            selected.0.story == StoryRef::Body
                && selected.0.path == path
                && selected.1.path == path
                && a < selected.1.off.saturating_add(wordcraft_doc::para::OBJ.len_utf8())
                && b > selected.0.off
        };
        let marked = |end: usize| matches!(p.object_at(end), Some(InlineObject::Field { instr, .. }) if FieldCode::parse(instr).name == "TA");
        let mut ends: Vec<usize> = kept.into_iter().filter(|(a, b)| !in_selection(*a, *b) && !marked(*b)).map(|(_, b)| b).collect();
        let room = MARK_ALL_CAP.saturating_sub(added);
        if ends.len() > room {
            ends.truncate(room);
            capped = true;
        }
        if ends.is_empty() {
            continue;
        }
        let para = s.doc.para_mut(StoryRef::Body, &path)?;
        for end in ends.into_iter().rev() {
            let (field, props) = ta_field(format!("TA \\s \"{short}\""));
            para.insert_object(end, field, &props)?;
            added += 1;
        }
        para.touch();
    }
    Ok((added, capped))
}

/// Insert Table of Authorities: for each category (all those with citations by default, as Word
/// inserts them; Cases when none are marked yet) a TOA Heading paragraph holding
/// `TOA \h \c "n"` (and `\p`), which Update Table fills with the category's name and entries.
pub(super) fn insert_authorities(s: &mut Session, v: &Value) -> CmdResult {
    let passim = p::bool(v, "passim").unwrap_or(true);
    let wanted = p::u64(v, "category").unwrap_or(0).min(16) as u32;
    let mut categories: Vec<u32> = if wanted > 0 {
        vec![wanted]
    } else {
        let mut used = std::collections::BTreeSet::new();
        for path in s.doc.para_paths(StoryRef::Body) {
            for o in s.doc.para(StoryRef::Body, &path).map(|p| p.objects.as_slice()).unwrap_or_default() {
                if let InlineObject::Field { instr, .. } = o {
                    let f = FieldCode::parse(instr);
                    if f.name == "TA" && f.arg('l').is_some() {
                        // A mark without a category is in no category's table (Word for Mac
                        // leaves it out of Cases, observed in 16.114).
                        if toa_category(&f) > 0 {
                            used.insert(toa_category(&f));
                        }
                    }
                }
            }
        }
        used.into_iter().collect()
    };
    if categories.is_empty() {
        categories.push(1);
    }
    let at = delete_selection(s)?;
    if at.story != StoryRef::Body || at.path.depth() > 0 {
        return Err(CmdError::Failed("lists go in the main text".into()));
    }
    add_toa_heading_style(s);
    // At the start of a paragraph the tables go in before it; elsewhere it is split there.
    let i = if at.off == 0 || s.doc.para_at(&at).is_some_and(|p| p.is_empty()) { at.path.last() } else { s.doc.split_paragraph(&at)?.path.last() };
    for (k, c) in categories.into_iter().enumerate() {
        let mut head = Paragraph::new().styled(TOA_HEADING_STYLE);
        let instr = format!("TOA \\h \\c \"{c}\"{}", if passim { " \\p" } else { "" });
        head.insert_object(
            0,
            InlineObject::Field { instr, result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )?;
        s.doc.insert_block(StoryRef::Body, &Path::top(i + k), Block::Para(head))?;
    }
    update_generated(s)?;
    sel_result(s)
}

/// Word's style for a table of authorities' category heading.
const TOA_HEADING_STYLE: &str = "TOAHeading";

/// Add Word's TOA Heading style if the document doesn't have it: bold, 12 pt, 6 pt before.
fn add_toa_heading_style(s: &mut Session) {
    if s.doc.styles.get(TOA_HEADING_STYLE).is_none() {
        s.doc.styles.upsert(wordcraft_doc::Style {
            id: TOA_HEADING_STYLE.into(),
            name: "TOA Heading".into(),
            kind: wordcraft_doc::StyleKind::Paragraph,
            based_on: Some("Normal".into()),
            next: Some("Normal".into()),
            para: wordcraft_doc::ParaProps { space_before: Some(6.0), ..Default::default() },
            chr: CharProps { bold: Some(true), size: Some(12.0), ..Default::default() },
            ..Default::default()
        });
    }
}

pub(super) fn is_toa(instr: &str) -> bool {
    FieldCode::parse(instr).name == "TOA"
}

/// The category a `TA` or `TOA` field names (`\c`); 0 when it names none.
pub(super) fn toa_category(f: &FieldCode) -> u32 {
    f.arg('c').and_then(|c| c.trim().parse().ok()).unwrap_or(0)
}

/// A table of authorities' entries as Word builds them (observed in Word for Mac 16.114): each
/// long citation (`TA \l`) of the table's category (`\c`; every category without it), in
/// alphabetical order, with the pages of its own mark and of every mark of its short citation
/// (`TA \s` alone, as Mark All inserts), and "passim" for five pages or more with `\p`. `\e`
/// separates a citation from its pages (a tab by default) and `\l` the pages (", ").
pub(super) fn toa_entries(s: &mut Session, kind: &str) -> Vec<(String, bool)> {
    let f = FieldCode::parse(kind);
    let category = toa_category(&f);
    let mut marks: Vec<(FieldCode, Pos)> = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for off in p.object_offsets() {
            if let Some(InlineObject::Field { instr, .. }) = p.object_at(off) {
                let ta = FieldCode::parse(instr);
                if ta.name == "TA" {
                    marks.push((ta, Pos { story: StoryRef::Body, path: path.clone(), off }));
                }
            }
        }
    }
    // The table's long citations, and the long citation each short one stands for.
    let mut pages: std::collections::HashMap<String, Vec<u32>> = Default::default();
    let mut long_of_short: std::collections::HashMap<String, String> = Default::default();
    for (ta, _) in &marks {
        if let Some(long) = ta.arg('l').filter(|l| !l.trim().is_empty())
            && (category == 0 || toa_category(ta) == category)
        {
            pages.entry(long.to_string()).or_default();
            if let Some(short) = ta.arg('s') {
                long_of_short.entry(short.to_string()).or_insert_with(|| long.to_string());
            }
        }
    }
    for (ta, pos) in &marks {
        let long = match ta.arg('l') {
            Some(l) => Some(l.to_string()).filter(|l| pages.contains_key(l)),
            None => ta.arg('s').and_then(|short| long_of_short.get(short).cloned()),
        };
        let Some(long) = long else { continue };
        let page = page_of(s, pos);
        if let Some(list) = pages.get_mut(&long)
            && !list.contains(&page)
        {
            list.push(page);
        }
    }
    let lang = crate::cmd::default_lang(s);
    let mut sorted: Vec<(SortKey, String, Vec<u32>)> = pages.into_iter().map(|(long, list)| (sort_key(&long, lang.as_deref()), long, list)).collect();
    sorted.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let entry_sep = f.arg('e').unwrap_or("\t");
    let page_sep = f.arg('l').unwrap_or(", ");
    sorted
        .into_iter()
        .map(|(_, long, mut list)| {
            list.sort_unstable();
            let numbers =
                if f.has('p') && list.len() >= 5 { "passim".to_string() } else { list.iter().map(u32::to_string).collect::<Vec<_>>().join(page_sep) };
            (format!("{long}{entry_sep}{numbers}"), false)
        })
        .collect()
}
