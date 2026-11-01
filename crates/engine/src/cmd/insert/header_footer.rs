use super::{
    Align, Block, Border, Borders, CharProps, CmdError, CmdResult, InlineObject, Paragraph, PartKind, Selection, Session, StoryRef, Table, Value,
    json, p, para_block,
};

/// The header (or footer) of the caret's section in the slot the caret's page uses (first page,
/// even page or the default), made when there is none: with Different First Page on, a header
/// inserted on page 1 is the one page 1 shows (#2826).
pub(super) fn hf_part(s: &mut Session, header: bool) -> u32 {
    let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
    let layout = s.layout();
    let page = if s.sel.focus.story == StoryRef::Body { layout.page_of(&s.sel.focus) } else { Some(s.page_hint) };
    let slot = page.and_then(|pg| layout.pages.get(pg)).map(|pg| pg.hf_slot).unwrap_or_default();
    fn slot_of(slot: wordcraft_layout::HfSlot, set: &mut wordcraft_doc::section::HeaderSet) -> &mut Option<u32> {
        match slot {
            wordcraft_layout::HfSlot::First => &mut set.first,
            wordcraft_layout::HfSlot::Even => &mut set.even,
            wordcraft_layout::HfSlot::Default => &mut set.default,
        }
    }
    let mut sect = s.doc.section_mut(block).clone();
    let existing = *slot_of(slot, if header { &mut sect.headers } else { &mut sect.footers });
    if let Some(id) = existing.filter(|id| s.doc.parts.contains_key(id)) {
        return id;
    }
    let style = if header { "Header" } else { "Footer" };
    let id = s.doc.add_part(if header { PartKind::Header } else { PartKind::Footer }, vec![para_block(Paragraph::new().styled(style))]);
    let sect = s.doc.section_mut(block);
    *slot_of(slot, if header { &mut sect.headers } else { &mut sect.footers }) = Some(id);
    id
}

/// Three independently editable cells spanning the section's text width.
fn three_column_hf(s: &Session, style: &str) -> wordcraft_doc::Blocks {
    let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
    // The section `section_mut` would pick, read without making anything unique.
    let sections = s.doc.sections();
    let own = sections.iter().position(|(end, _)| block <= *end).filter(|i| i + 1 < sections.len()).and_then(|i| sections.get(i));
    let width = own.map_or_else(|| s.doc.last_section.text_width(), |(_, sec)| sec.text_width());
    let mut table = Table::new(1, 3, width);
    table.props.style = Some("TableNormal".into());
    table.props.width = Some(width);
    table.props.fixed = true;
    // Explicit nil borders also override borders inherited from the document's table style.
    table.props.borders = Some(Borders::all(Border::default()));
    for row in &mut table.rows {
        for (cell, align) in row.cells.iter_mut().zip([Align::Left, Align::Center, Align::Right]) {
            let mut para = Paragraph::new().styled(style);
            para.props.align = Some(align);
            cell.blocks = vec![para_block(para)];
        }
    }
    // Keep a trailing paragraph so the story can also be edited after the table.
    vec![std::sync::Arc::new(Block::Table(table)), para_block(Paragraph::new().styled(style))]
}

pub(super) fn header_footer(s: &mut Session, v: &Value, header: bool) -> CmdResult {
    let id = hf_part(s, header);
    let style = if header { "Header" } else { "Footer" };
    if let Some(text) = p::str(v, "text") {
        let blocks = text.split('\n').map(|l| para_block(Paragraph::with_text(l, CharProps::default()).styled(style))).collect();
        s.doc.set_story(StoryRef::Part(id), blocks)?;
    } else if p::str(v, "preset") == Some("blankThree") {
        let blocks = three_column_hf(s, style);
        s.doc.set_story(StoryRef::Part(id), blocks)?;
    } else if let Some(preset) = p::str(v, "preset") {
        let mut para = Paragraph::new().styled(style);
        match preset {
            "title" => {
                para.insert_text(0, if s.doc.core.title.is_empty() { "[Document title]" } else { &s.doc.core.title }, &CharProps::default())?;
                para.props.align = Some(Align::Center);
            }
            "pageNumber" => {
                para.props.align = Some(Align::Center);
                para.insert_object(
                    0,
                    InlineObject::Field {
                        instr: "PAGE".into(),
                        result: "1".into(),
                        locked: false,
                        dirty: false,
                        code: false,
                        form: String::new(),
                        code_xml: String::new(),
                    },
                    &CharProps::default(),
                )?;
            }
            _ => {
                para.insert_text(0, "[Type here]", &CharProps::default())?;
            }
        }
        s.doc.set_story(StoryRef::Part(id), vec![para_block(para)])?;
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

pub(super) fn edit_hf(s: &mut Session, v: &Value, header: bool) -> CmdResult {
    let mut page = p::u64(v, "page").map(|page| usize::try_from(page).unwrap_or(usize::MAX));
    if page.is_none() && s.sel.focus.story == StoryRef::Body {
        // The caret's page: with Different First Page on, page 1 edits its own header (#2826).
        let focus = s.sel.focus.clone();
        page = s.layout().page_of(&focus);
    }
    let shown = match page {
        Some(page) => shown_hf(s, page, header)?,
        None => {
            let block = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
            let set = s
                .doc
                .sections()
                .into_iter()
                .find(|(end, _)| block <= *end)
                .or_else(|| s.doc.sections().pop())
                .map(|(_, sp)| if header { sp.headers.default } else { sp.footers.default });
            set.flatten().filter(|id| s.doc.parts.contains_key(id))
        }
    };
    let Some(id) = shown else {
        // None yet: made by the mutating command, so it is an undo step and protection applies.
        let cmd = if header { "insert.createHeader" } else { "insert.createFooter" };
        let params = page.map_or_else(|| json!({}), |page| json!({ "page": page }));
        return s.run(cmd, &params);
    };
    if let Some(page) = page {
        s.page_hint = page;
    }
    s.touch();
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

/// The header (or footer) page `page` shows, if any; an error for a page that isn't there.
fn shown_hf(s: &mut Session, page: usize, header: bool) -> Result<Option<u32>, CmdError> {
    let layout = s.layout();
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    let shown = if header { pg.header_story } else { pg.footer_story };
    Ok(shown.filter(|id| s.doc.parts.contains_key(id)))
}

pub(super) fn create_hf(s: &mut Session, v: &Value, header: bool) -> CmdResult {
    let id = match p::u64(v, "page") {
        Some(page) => {
            let page = usize::try_from(page).unwrap_or(usize::MAX);
            s.page_hint = page;
            page_hf_part(s, page, header)?
        }
        None => {
            let before = s.doc.parts.len();
            let id = hf_part(s, header);
            if s.doc.parts.len() > before {
                s.new_header_parts.push(id);
            }
            id
        }
    };
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Part(id)));
    Ok(json!({"story": id}))
}

/// The header (or footer) page `page` shows; when it shows none, a new empty one in the slot the
/// page uses (first page, even page or the default), as Word creates on a double-click in the
/// empty header area (#960). It goes in the first section: sections link to the previous one by
/// default, so as in Word it shows on every page that uses that slot and has none of its own.
fn page_hf_part(s: &mut Session, page: usize, header: bool) -> Result<u32, CmdError> {
    if let Some(id) = shown_hf(s, page, header)? {
        return Ok(id);
    }
    let layout = s.layout();
    let slot = layout.pages.get(page).map(|pg| pg.hf_slot).unwrap_or_default();
    // The page shows none, so no section up to its own has this slot: the first section's is
    // inherited all the way to it.
    let block = s.doc.sections().first().map(|(end, _)| *end).unwrap_or(0);
    let style = if header { "Header" } else { "Footer" };
    let id = s.doc.add_part(if header { PartKind::Header } else { PartKind::Footer }, vec![para_block(Paragraph::new().styled(style))]);
    let sect = s.doc.section_mut(block);
    let set = if header { &mut sect.headers } else { &mut sect.footers };
    match slot {
        wordcraft_layout::HfSlot::First => set.first = Some(id),
        wordcraft_layout::HfSlot::Even => set.even = Some(id),
        wordcraft_layout::HfSlot::Default => set.default = Some(id),
    }
    s.new_header_parts.push(id);
    Ok(id)
}

/// The headers/footers made by `insert.createHeader`/`insert.createFooter` that are still empty
/// (one empty paragraph). The list keeps them after they are taken away, so one an undo brings
/// back is taken away again on the next close.
pub(super) fn new_empty_headers(s: &Session) -> Vec<u32> {
    s.new_header_parts
        .iter()
        .copied()
        .filter(|id| {
            s.doc.parts.get(id).is_some_and(|part| {
                matches!(part.kind, PartKind::Header | PartKind::Footer)
                    && part.blocks.len() == 1
                    && part.blocks.first().is_some_and(|b| matches!(&**b, Block::Para(p) if p.text.is_empty() && p.objects.is_empty()))
            })
        })
        .collect()
}

/// The document as saved: without the headers/footers just made and still empty, so an unwritten
/// one is never saved (`None` when there are none, to save the document as it is).
pub(crate) fn saved_copy(s: &Session) -> Option<wordcraft_doc::Document> {
    let ids = new_empty_headers(s);
    if ids.is_empty() {
        return None;
    }
    let mut doc = s.doc.clone();
    strip_headers(&mut doc, &ids).then_some(doc)
}

/// Takes headers/footers `ids` out of the document and every section that refers to them;
/// whether anything was taken away.
pub(super) fn strip_headers(doc: &mut wordcraft_doc::Document, ids: &[u32]) -> bool {
    if ids.is_empty() {
        return false;
    }
    let clear = |set: &mut wordcraft_doc::section::HeaderSet| {
        for slot in [&mut set.default, &mut set.first, &mut set.even] {
            if slot.is_some_and(|id| ids.contains(&id)) {
                *slot = None;
            }
        }
    };
    clear(&mut doc.last_section.headers);
    clear(&mut doc.last_section.footers);
    for path in doc.para_paths(StoryRef::Body) {
        if doc.para(StoryRef::Body, &path).is_some_and(|p| p.section.is_some())
            && let Ok(p) = doc.para_mut(StoryRef::Body, &path)
            && let Some(sec) = p.section.as_mut()
        {
            clear(&mut sec.headers);
            clear(&mut sec.footers);
        }
    }
    let mut any = false;
    for id in ids {
        any |= doc.parts.remove(id).is_some();
    }
    any
}
