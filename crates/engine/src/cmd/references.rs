//! References tab: table of contents, footnotes/endnotes, captions; field updates.

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, NoteKind};
use wordcraft_doc::props::{CharProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::{Block, Paragraph, PartKind, Path, Pos, StoryRef, para_block};

use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("references.toc", "Table of Contents", "References › Table of Contents", insert_toc)
            .params(r#"{"levels"?: 1-9, "title"?: string}"#),
        CommandSpec::new("references.updateToc", "Update Table", "References › Table of Contents", |s, _| {
            update_toc(s)?;
            sel_result(s)
        }),
        CommandSpec::new("references.removeToc", "Remove Table of Contents", "References › Table of Contents", |s, _| {
            if let Some((start, end)) = toc_range(s) {
                for i in (start..=end).rev() {
                    s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
                }
                s.clamp_selection();
            }
            sel_result(s)
        }),
        CommandSpec::new("references.addText", "Add Text", "References › Table of Contents", |s, v| {
            let level = p::u64(v, "level").unwrap_or(0).min(9) as u8;
            super::para::fmt(s, &|pp| pp.outline_level = if level == 0 { Some(9) } else { Some(level - 1) })
        })
        .params(r#"{"level": 0 (do not show) | 1-9}"#),
        CommandSpec::new("references.footnote", "Insert Footnote", "References › Footnotes", |s, v| note(s, v, NoteKind::Footnote))
            .key("Mod+Alt+F")
            .params(r#"{"text"?: string, "mark"?: string (a custom mark instead of the number)}"#),
        CommandSpec::new("references.endnote", "Insert Endnote", "References › Footnotes", |s, v| note(s, v, NoteKind::Endnote))
            .key("Mod+Alt+D")
            .params(r#"{"text"?: string, "mark"?: string (a custom mark instead of the number)}"#),
        CommandSpec::new("references.convertNotes", "Convert Notes", "References › Footnotes", |s, v| {
            let (foot, end) = match p::req_str(v, "to")? {
                "endnote" | "endnotes" => (true, false),
                "footnote" | "footnotes" => (false, true),
                "swap" => (true, true),
                t => return Err(CmdError::Params(format!("unknown `to` {t:?}; use endnote, footnote or swap"))),
            };
            let n = s.doc.convert_notes(foot, end);
            Ok(json!({"converted": n}))
        })
        .params(r#"{"to": "endnote" (footnotes become endnotes) | "footnote" (endnotes become footnotes) | "swap"}"#),
        CommandSpec::new("references.nextFootnote", "Next Footnote", "References › Footnotes", |s, _| {
            let caret = s.sel.focus.clone();
            let refs = note_refs(s);
            if let Some(p) = refs.iter().find(|p| **p > caret).or(refs.first()) {
                s.sel = Selection::caret(p.clone());
            }
            sel_result(s)
        })
        .pure(),
        CommandSpec::new("references.notes", "Show Notes", "References › Footnotes", |s, _| {
            let mut out = Vec::new();
            for (id, part) in &s.doc.parts {
                if matches!(part.kind, PartKind::Footnote | PartKind::Endnote) {
                    out.push(json!({"id": id, "kind": part.kind, "text": s.doc.plain_text(StoryRef::Part(*id))}));
                }
            }
            Ok(Value::Array(out))
        })
        .pure(),
        CommandSpec::new("references.caption", "Insert Caption", "References › Captions", caption)
            .params(r#"{"label"?: "Figure|Table|Equation", "text"?: string}"#),
        CommandSpec::new("references.updateFields", "Update Field", "References", |s, _| {
            update_fields(s)?;
            sel_result(s)
        })
        .key("F9"),
    ]
}

/// Headings (outline level < levels) in the body: (path, level, text).
fn headings(s: &Session, levels: u8) -> Vec<(Path, u8, String)> {
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        if path.depth() > 0 {
            continue;
        }
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let style = p.props.style.as_deref().unwrap_or("Normal");
        if style.starts_with("TOC") {
            continue;
        }
        let rp = s.doc.styles.resolve_para(&p.props);
        if let Some(l) = rp.outline_level
            && l < levels
        {
            let t = p.plain_text().replace(['\u{000C}', '\n', '\t'], " ").trim().to_string();
            if !t.is_empty() {
                v.push((path, l, t));
            }
        }
    }
    v
}

/// The TOC block range in the body: the paragraph holding the TOC field and the TOC entries after it.
fn toc_range(s: &Session) -> Option<(usize, usize)> {
    let start = s.doc.body.iter().position(|b| {
        b.as_para().is_some_and(|p| {
            p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if instr.trim_start().to_ascii_uppercase().starts_with("TOC")))
        })
    })?;
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

fn toc_levels(s: &Session, start: usize) -> u8 {
    s.doc
        .body
        .get(start)
        .and_then(|b| b.as_para())
        .and_then(|p| {
            p.objects.iter().find_map(|o| match o {
                InlineObject::Field { instr, .. } => instr
                    .split("\\o")
                    .nth(1)
                    .and_then(|r| r.trim().trim_matches('"').split('-').nth(1))
                    .and_then(|n| n.trim_matches('"').parse::<u8>().ok()),
                _ => None,
            })
        })
        .unwrap_or(3)
        .clamp(1, 9)
}

fn insert_toc(s: &mut Session, v: &Value) -> CmdResult {
    if toc_range(s).is_some() {
        update_toc(s)?;
        return sel_result(s);
    }
    let levels = p::u64(v, "levels").unwrap_or(3).clamp(1, 9) as u8;
    let title = p::str(v, "title").unwrap_or("Contents").to_string();
    let at = delete_selection(s)?;
    if at.story != StoryRef::Body || at.path.depth() > 0 {
        return Err(CmdError::Failed("a table of contents goes in the main text".into()));
    }
    // Title paragraph carrying the TOC field marker.
    let mut head = Paragraph::with_text(&title, CharProps::default()).styled("TOCHeading");
    head.insert_object(
        head.len(),
        InlineObject::Field { instr: format!("TOC \\o \"1-{levels}\" \\h \\z \\u"), result: String::new(), locked: false },
        &CharProps::default(),
    )?;
    let i = if s.doc.para_at(&at).is_some_and(|p| p.is_empty()) { at.path.last() } else { s.doc.split_paragraph(&at)?.path.last() };
    s.doc.insert_block(StoryRef::Body, &Path::top(i), Block::Para(head))?;
    update_toc(s)?;
    sel_result(s)
}

/// Regenerate the TOC entries with page numbers (two layout passes: entries change pagination).
pub fn update_toc(s: &mut Session) -> Result<(), CmdError> {
    let Some((start, end)) = toc_range(s) else { return Ok(()) };
    let levels = toc_levels(s, start);
    for i in (start + 1..=end).rev() {
        s.doc.remove_block(StoryRef::Body, &Path::top(i))?;
    }
    let width = super::page::sect(s).text_width();
    let entries = headings(s, levels);
    // A heading typed after a manual page break starts with that break; its page is where the
    // text after the break lands, so look the page up past any leading breaks.
    let leads: Vec<usize> =
        entries.iter().map(|(path, ..)| s.doc.para(StoryRef::Body, path).map_or(0, |p| p.text.bytes().take_while(|b| *b == 0x0C).count())).collect();
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
        for (k, (path, level, text)) in entries.iter().enumerate() {
            let target = Path::top(path.last() + if path.last() > start { shift } else { 0 });
            let off = leads.get(k).copied().unwrap_or(0);
            let page = l.caret(&Pos { story: StoryRef::Body, path: target, off }).and_then(|c| l.pages.get(c.page)).map(|pg| pg.number).unwrap_or(1);
            let mut para = Paragraph::with_text(&format!("{text}\t{page}"), CharProps::default()).styled(&format!("TOC{}", level + 1));
            para.props.tabs = Some(vec![TabStop { pos: width - 0.5, align: TabAlign::Right, leader: TabLeader::Dot }]);
            s.doc.insert_block(StoryRef::Body, &Path::top(start + 1 + k), Block::Para(para))?;
        }
    }
    s.touch();
    Ok(())
}

fn note(s: &mut Session, v: &Value, kind: NoteKind) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    // A custom mark: a few characters, as Word's dialog takes.
    let custom: String = p::str(v, "mark").unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(10).collect();
    let (pk, style, refstyle) = match kind {
        NoteKind::Footnote => (PartKind::Footnote, "FootnoteText", "FootnoteReference"),
        NoteKind::Endnote => (PartKind::Endnote, "EndnoteText", "EndnoteReference"),
    };
    let mut para = Paragraph::new().styled(style);
    let id = s.doc.add_part(pk, Vec::new());
    para.insert_object(
        0,
        InlineObject::NoteRef { kind, id, custom: custom.clone() },
        &CharProps { style: Some(refstyle.into()), ..Default::default() },
    )?;
    let len = para.len();
    para.insert_text(len, &format!(" {text}"), &CharProps::default())?;
    s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    let at = delete_selection(s)?;
    s.doc.insert_object(&at, InlineObject::NoteRef { kind, id, custom }, &CharProps { style: Some(refstyle.into()), ..Default::default() })?;
    // Like Word, the caret moves into the new note.
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"id": id}))
}

/// `references.noteOptions`: footnote or endnote placement, number format, start and restart,
/// for the whole document (each section then follows it) or the selection's sections.
pub(super) fn note_options(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::section::{MAX_NOTE_START, NotePos, NoteProps, NoteRestart, NumFormat};
    let fmt = |k: &str| -> Result<Option<NumFormat>, CmdError> {
        match p::str(v, k) {
            None => Ok(None),
            Some(f) if NumFormat::from_ooxml(f).ooxml() == f => Ok(Some(NumFormat::from_ooxml(f))),
            Some(f) => Err(CmdError::Params(format!("unknown number format {f:?}"))),
        }
    };
    // The earlier form: the document's number formats.
    let (old_foot, old_end) = (fmt("footnoteFormat")?, fmt("endnoteFormat")?);
    let endnote = match p::str(v, "kind") {
        None | Some("footnote") => false,
        Some("endnote") => true,
        Some(k) => return Err(CmdError::Params(format!("unknown `kind` {k:?}; use footnote or endnote"))),
    };
    let pos = match p::str(v, "pos") {
        None => None,
        Some(x) => match NotePos::from_ooxml(x) {
            Some(p @ (NotePos::PageBottom | NotePos::BeneathText)) if !endnote => Some(p),
            Some(p @ (NotePos::SectEnd | NotePos::DocEnd)) if endnote => Some(p),
            _ => {
                let ok = if endnote { "sectEnd or docEnd" } else { "pageBottom or beneathText" };
                return Err(CmdError::Params(format!("`pos` {x:?} doesn't apply here; use {ok}")));
            }
        },
    };
    let num_restart = match p::str(v, "numRestart") {
        None => None,
        Some(r) => match NoteRestart::from_ooxml(r) {
            Some(NoteRestart::EachPage) if endnote => return Err(CmdError::Params("endnotes can't start again on each page".into())),
            Some(r) => Some(r),
            None => return Err(CmdError::Params(format!("unknown `numRestart` {r:?}; use continuous, eachSect or eachPage"))),
        },
    };
    let change = NoteProps {
        pos,
        num_fmt: fmt("numFmt")?,
        num_start: p::f32(v, "numStart").map(|n| n.round().clamp(1.0, MAX_NOTE_START as f32) as u32),
        num_restart,
    };
    let whole = match p::str(v, "scope") {
        None | Some("document") => true,
        Some("section") => false,
        Some(x) => return Err(CmdError::Params(format!("unknown `scope` {x:?}; use document or section"))),
    };
    // Set `from`'s fields on `to`; `clear` instead unsets the fields `from` sets.
    let merge = |to: &mut NoteProps, from: &NoteProps, clear: bool| {
        if from.pos.is_some() {
            to.pos = if clear { None } else { from.pos };
        }
        if from.num_fmt.is_some() {
            to.num_fmt = if clear { None } else { from.num_fmt };
        }
        if from.num_start.is_some() {
            to.num_start = if clear { None } else { from.num_start };
        }
        if from.num_restart.is_some() {
            to.num_restart = if clear { None } else { from.num_restart };
        }
    };
    fn pick(sect: &mut wordcraft_doc::SectionProps, endnote: bool) -> &mut NoteProps {
        if endnote { &mut sect.endnote_pr } else { &mut sect.footnote_pr }
    }
    if change.is_empty() {
        // Nothing to change: the options in effect.
    } else if whole {
        let st = &mut s.doc.settings;
        if let Some(f) = change.num_fmt {
            *(if endnote { &mut st.endnote_format } else { &mut st.footnote_format }) = f;
        }
        merge(if endnote { &mut st.endnote_pr } else { &mut st.footnote_pr }, &NoteProps { num_fmt: None, ..change }, false);
        // Every section follows the document now.
        let ends: Vec<usize> = s.doc.sections().iter().map(|(e, _)| *e).collect();
        for e in ends {
            merge(pick(s.doc.section_mut(e), endnote), &change, true);
        }
    } else {
        super::page::with_sect(s, |sect| merge(pick(sect, endnote), &change, false))?;
    }
    if let Some(f) = old_foot {
        s.doc.settings.footnote_format = f;
    }
    if let Some(f) = old_end {
        s.doc.settings.endnote_format = f;
    }
    let sect = super::page::sect(s);
    Ok(json!({
        "footnote": s.doc.note_options(&sect, false),
        "endnote": s.doc.note_options(&sect, true),
        "footnoteFormat": s.doc.settings.footnote_format,
        "endnoteFormat": s.doc.settings.endnote_format,
    }))
}

fn note_refs(s: &Session) -> Vec<Pos> {
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for o in p.object_offsets() {
            if matches!(p.object_at(o), Some(InlineObject::NoteRef { .. })) {
                v.push(Pos { story: StoryRef::Body, path: path.clone(), off: o });
            }
        }
    }
    v
}

fn caption(s: &mut Session, v: &Value) -> CmdResult {
    let label = p::str(v, "label").unwrap_or("Figure").to_string();
    let text = p::str(v, "text").unwrap_or("").to_string();
    // Number = existing captions with this label before the caret + 1.
    let caret = s.sel.focus.clone();
    let n = s
        .doc
        .para_paths(StoryRef::Body)
        .into_iter()
        .filter(|q| *q < caret.path)
        .filter(|q| s.doc.para(StoryRef::Body, q).is_some_and(|p| p.props.style.as_deref() == Some("Caption") && p.text.starts_with(&label)))
        .count()
        + 1;
    let f = s.sel.focus.clone();
    let len = s.doc.para_at(&f).map(|p| p.len()).unwrap_or(0);
    let at = Pos { off: len, ..f };
    let new = s.doc.split_paragraph(&at)?;
    let para = s.doc.para_mut(new.story, &new.path)?;
    para.props = wordcraft_doc::ParaProps { style: Some("Caption".into()), ..Default::default() };
    para.mark = CharProps::default();
    let base = format!("{label} ");
    para.insert_text(0, &base, &CharProps::default())?;
    para.insert_object(
        base.len(),
        InlineObject::Field { instr: format!("SEQ {label} \\* ARABIC"), result: n.to_string(), locked: false },
        &CharProps::default(),
    )?;
    let end = para.len();
    if !text.is_empty() {
        para.insert_text(end, &format!(": {text}"), &CharProps::default())?;
    }
    let off = para.len();
    s.sel = Selection::caret(Pos { off, ..new });
    sel_result(s)
}

/// Update fields in the body: dates, SEQ numbering, cross-references, TOC.
pub fn update_fields(s: &mut Session) -> Result<(), CmdError> {
    let mut seq: std::collections::HashMap<String, u32> = Default::default();
    let mut pages: std::collections::HashMap<String, u32> = Default::default();
    for (name, pos) in s.doc.bookmarks() {
        let l = s.layout();
        let page = l.caret(&pos).and_then(|c| l.pages.get(c.page)).map(|p| p.number).unwrap_or(1);
        pages.entry(name).or_insert(page);
    }
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        if !p.objects.iter().any(|o| matches!(o, InlineObject::Field { .. })) {
            continue;
        }
        let mut updates: Vec<(usize, String)> = Vec::new();
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Field { instr, locked: false, .. } = o else { continue };
            let name = wordcraft_layout::fields::field_name(instr);
            match name.as_str() {
                "DATE" | "TIME" | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => {
                    let pic = date_picture(instr).unwrap_or_else(|| if name == "TIME" { "h:mm am/pm".into() } else { "M/d/yyyy".into() });
                    updates.push((k, super::insert::format_date(&pic)));
                }
                "SEQ" => {
                    let id = instr.split_whitespace().nth(1).unwrap_or("").to_string();
                    let n = seq.entry(id).or_insert(0);
                    *n += 1;
                    updates.push((k, n.to_string()));
                }
                "REF" | "PAGEREF" => {
                    let target = instr.split_whitespace().nth(1).unwrap_or("");
                    let r = if name == "REF" { bookmark_text(&s.doc, target) } else { pages.get(target).map(u32::to_string) };
                    if let Some(r) = r {
                        updates.push((k, r));
                    }
                }
                "TITLE" => updates.push((k, s.doc.core.title.clone())),
                "AUTHOR" => updates.push((k, s.doc.core.creator.clone())),
                "NUMWORDS" => updates.push((k, s.doc.word_count().to_string())),
                _ => {}
            }
        }
        if updates.is_empty() {
            continue;
        }
        let para = s.doc.para_mut(StoryRef::Body, &path)?;
        for (k, r) in updates {
            if let Some(InlineObject::Field { result, .. }) = para.objects.get_mut(k) {
                *result = r;
            }
        }
        para.touch();
    }
    super::citations::update_citations(s)?;
    update_toc(s)
}

/// A field's `\@` date picture: the quoted text after it (switches may follow), or one word.
pub fn date_picture(instr: &str) -> Option<String> {
    let rest = instr.split_once("\\@")?.1.trim_start();
    let pic = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or(""),
        None => rest.split_whitespace().next().unwrap_or(""),
    };
    (!pic.is_empty()).then(|| pic.to_string())
}

/// The text a bookmark marks (field results included), up to its end in the same paragraph or
/// the paragraph's end. `None` when there is no such bookmark.
pub fn bookmark_text(doc: &wordcraft_doc::Document, name: &str) -> Option<String> {
    let (_, start) = doc.bookmarks().into_iter().find(|(n, _)| n == name)?;
    let para = doc.para(start.story, &start.path)?;
    let end = para
        .object_offsets()
        .into_iter()
        .find(|o| *o > start.off && matches!(para.object_at(*o), Some(InlineObject::BookmarkEnd { name: n }) if n == name))
        .unwrap_or(para.len());
    Some(doc.copy_range(&start, &Pos { off: end, ..start.clone() }).plain_text().trim().to_string())
}
