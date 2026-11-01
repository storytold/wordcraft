use super::{Arc, Block, Blocks, ControlEvent, ControlKind, Document, HashSet, InlineObject, OBJ, StyleKind, control_events, pair_events};

/// Word draws a table of contents' entry links without the Hyperlink character style (no blue,
/// no underline) though the entries carry it (measured in Word for Windows: a Hyperlink-styled run
/// in a hyperlink inside a TOC field's result is plain; one outside a hyperlink, another style,
/// direct formatting, or a hyperlink in another field keep their look). The style is dropped from
/// those runs so they look the same here; the links stay. The style is found by its built-in
/// name: localized Word and older files give it another id (`Hipervnculo`, `Hyperlnk`).
pub(super) fn unstyle_toc_links(doc: &mut Document) {
    let ids: HashSet<String> = doc
        .styles
        .styles
        .iter()
        .filter(|s| s.kind == StyleKind::Character && s.name.eq_ignore_ascii_case("Hyperlink"))
        .map(|s| s.id.clone())
        .chain(["Hyperlink".to_string()])
        .collect();
    for f in doc.field_ranges(wordcraft_doc::StoryRef::Body) {
        if is_toc(&f.instr) {
            let _ = doc.format_range(&f.start, &f.end, &|c| {
                if c.link.is_some() && c.style.as_deref().is_some_and(|s| ids.contains(s)) {
                    c.style = None;
                }
            });
        }
    }
}

/// Word's Table of Contents content control belongs to the TOC: the writer puts it back around
/// the TOC heading and its entries (`toc_span`), which References › Update Table rebuilds. It is
/// handed to the TOC only when the writer would put it back around the same paragraphs; any other
/// is kept as read, or saving would drop it (#2427).
pub(super) fn toc_control_to_toc(body: &mut Blocks) {
    let span = {
        let events = control_events(body);
        let (pairs, _) = pair_events(&events);
        pairs.iter().find_map(|&(o, c, depth)| {
            let (ControlEvent::Open { block: s, off: Some(0), control }, ControlEvent::Close { block: e, off: Some(off) }) =
                (events.get(o)?, events.get(c)?)
            else {
                return None;
            };
            let toc = matches!(&control.kind, ControlKind::Gallery { list: false, gallery, .. } if gallery == "Table of Contents");
            let at_end = body.get(*e)?.as_para()?.len() == off.checked_add(OBJ.len_utf8())?;
            (depth == 0 && toc && at_end).then_some((*s, *e, *off))
        })
    };
    let Some((s, e, off)) = span else { return };
    let mut toc = body.clone();
    // The end first: both markers may be in the same paragraph.
    if let Some(Block::Para(p)) = toc.get_mut(e).map(Arc::make_mut) {
        let _ = p.delete(off, p.len());
    }
    if let Some(Block::Para(p)) = toc.get_mut(s).map(Arc::make_mut) {
        let _ = p.delete(0, OBJ.len_utf8());
    }
    toc_field_to_heading(&mut toc);
    if crate::write::written_toc_span(&toc) == Some((s, e)) {
        *body = toc;
        return;
    }
    // TOCs the writer rebuilds in the same paragraphs once Update Table has turned them into the
    // engine's TOC (`range_toc_to_field`) are handed over as the range read, as without the control:
    // - Word's layout, the field's end alone in a paragraph after the entries (it goes with them);
    // - ours with a heading that isn't a `TOCHeading`, written empty and the field opening in the
    //   first entry. Kept in the control, that paragraph wouldn't be empty and each save and update
    //   would add a blank line above the entries (#2182).
    let mut engine = toc.clone();
    let mut last = e;
    // The entry before Word's paragraph holding nothing but the field's end, and that end's format.
    let end_only = e.checked_sub(1).filter(|&l| l > s).and_then(|l| {
        let p = engine.get(e)?.as_para()?;
        let only = p.section.is_none() && p.len() == OBJ.len_utf8() && matches!(p.object_at(0), Some(InlineObject::FieldEnd));
        only.then(|| (l, p.props_of_char(0).clone()))
    });
    if let Some((l, props)) = end_only {
        engine.remove(e);
        last = l;
        if let Some(Block::Para(p)) = engine.get_mut(l).map(Arc::make_mut) {
            let len = p.len();
            let _ = p.insert_object(len, InlineObject::FieldEnd, &props);
        }
    }
    // An empty paragraph opening the control takes the field. It may end a section, as above
    // entries in columns (#3889).
    if engine.get(s).and_then(|b| b.as_para()).is_some_and(|p| p.is_empty()) {
        field_to_heading(&mut engine, s + 1);
    } else {
        toc_field_to_heading(&mut engine);
    }
    if crate::write::written_toc_span(&engine) == Some((s, last)) {
        *body = toc;
    }
}

/// Normalize only a complete TOC range that the engine's heading/entry representation can
/// reproduce exactly. Other imported TOCs keep their explicit boundaries, independent of styles.
pub(super) fn toc_field_to_heading(body: &mut Blocks) {
    for i in 1..body.len() {
        if body[i - 1].as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOCHeading")) {
            field_to_heading(body, i);
        }
    }
}

/// Moves the TOC field that opens paragraph `i` (a complete range over the entries, or an empty
/// field) to the end of paragraph `i - 1`, the engine's TOC heading, unless that one has a TOC
/// field already.
fn field_to_heading(body: &mut Blocks, i: usize) {
    let has_toc = |o: &InlineObject| matches!(o, InlineObject::Field { instr, .. } if is_toc(instr));
    if i == 0 || body.get(i - 1).and_then(|b| b.as_para()).is_none_or(|h| h.objects.iter().any(has_toc)) {
        return;
    }
    let Some((field, props, end)) = body.get(i).and_then(|b| b.as_para()).and_then(|p| {
        let (field, end) = match p.object_at(0) {
            Some(InlineObject::FieldStart { instr, locked, dirty }) if is_toc(instr) => (
                InlineObject::Field {
                    instr: instr.clone(),
                    result: String::new(),
                    locked: *locked,
                    dirty: *dirty,
                    code: false,
                    form: String::new(),
                    code_xml: String::new(),
                },
                Some(toc_range_end(body, i)?),
            ),
            Some(f @ InlineObject::Field { result, .. }) if result.is_empty() && has_toc(f) => (f.clone(), None),
            _ => return None,
        };
        Some((field, p.props_of_char(0).clone(), end))
    }) else {
        return;
    };
    // Delete the end first: the two markers may occupy the same paragraph.
    if let Some(Block::Para(p)) = end.and_then(|end| body.get_mut(end)).map(Arc::make_mut) {
        let off = p.len().saturating_sub(OBJ.len_utf8());
        let _ = p.delete(off, p.len());
    }
    if let Some(Block::Para(p)) = body.get_mut(i).map(Arc::make_mut) {
        let _ = p.delete(0, OBJ.len_utf8());
    }
    if let Some(Block::Para(h)) = body.get_mut(i - 1).map(Arc::make_mut) {
        let end = h.len();
        let _ = h.insert_object(end, field, &props);
    }
}

pub(super) fn is_toc(instr: &str) -> bool {
    instr.split_whitespace().next().is_some_and(|k| k.eq_ignore_ascii_case("TOC"))
}

/// The actual matching end must also be the end the styled writer would choose: at the end
/// of the last contiguous TOC entry, with no section break or trailing content outside it.
fn toc_range_end(body: &Blocks, start: usize) -> Option<usize> {
    let entry =
        |p: &wordcraft_doc::Paragraph| p.section.is_none() && p.props.style.as_deref().is_some_and(|s| s.starts_with("TOC") && s != "TOCHeading");
    let mut depth = 0_usize;
    for (i, b) in body.iter().enumerate().skip(start) {
        let p = b.as_para().filter(|p| entry(p))?;
        for (off, o) in p.object_offsets().into_iter().zip(&p.objects) {
            match o {
                InlineObject::FieldStart { .. } => depth = depth.checked_add(1)?,
                InlineObject::FieldEnd => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        let following_entry = body.get(i + 1).and_then(|b| b.as_para()).is_some_and(entry);
                        return (off.checked_add(OBJ.len_utf8()) == Some(p.len()) && !following_entry).then_some(i);
                    }
                }
                _ => {}
            }
        }
    }
    None
}
