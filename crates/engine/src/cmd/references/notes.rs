use super::{
    CharProps, CmdError, CmdResult, InlineObject, NoteKind, Paragraph, PartKind, Pos, Selection, Session, StoryRef, Value, delete_selection, json, p,
    para_block,
};

pub(super) fn note(s: &mut Session, v: &Value, kind: NoteKind) -> CmdResult {
    let text = p::str(v, "text").unwrap_or("").to_string();
    // A custom mark: a few characters, as Word's dialog takes.
    let custom: String = p::str(v, "mark").unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(10).collect();
    let (pk, style, refstyle) = match kind {
        NoteKind::Footnote => (PartKind::Footnote, "FootnoteText", "FootnoteReference"),
        NoteKind::Endnote => (PartKind::Endnote, "EndnoteText", "EndnoteReference"),
    };
    let mut para = Paragraph::new().styled(style);
    let id = s.doc.try_add_part(pk, Vec::new())?;
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
pub(in crate::cmd) fn note_options(s: &mut Session, v: &Value) -> CmdResult {
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
        crate::cmd::page::with_sect(s, |sect| merge(pick(sect, endnote), &change, false))?;
    }
    if let Some(f) = old_foot {
        s.doc.settings.footnote_format = f;
    }
    if let Some(f) = old_end {
        s.doc.settings.endnote_format = f;
    }
    let sect = crate::cmd::page::sect(s);
    Ok(json!({
        "footnote": s.doc.note_options(&sect, false),
        "endnote": s.doc.note_options(&sect, true),
        "footnoteFormat": s.doc.settings.footnote_format,
        "endnoteFormat": s.doc.settings.endnote_format,
    }))
}

pub(super) fn note_refs(s: &Session, kind: Option<NoteKind>) -> Vec<Pos> {
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for o in p.object_offsets() {
            if let Some(InlineObject::NoteRef { kind: k, .. }) = p.object_at(o)
                && (kind.is_none() || kind == Some(*k))
            {
                v.push(Pos { story: StoryRef::Body, path: path.clone(), off: o });
            }
        }
    }
    v
}
