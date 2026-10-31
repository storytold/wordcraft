//! Named page-label segments reuse document sections and the normal undo transaction.
use crate::{CmdError, CmdResult, Selection, Session};
use serde_json::{Value, json};
use wordcraft_doc::page_label::PageLabel;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::section::{HeaderSet, NumFormat, SectionStart};
use wordcraft_doc::{Block, Paragraph, PartKind, Path, Pos, StoryRef};

pub(super) fn apply(s: &mut Session, v: &Value) -> CmdResult {
    let label: Option<PageLabel> = serde_json::from_value(v.get("label").cloned().ok_or_else(|| CmdError::Params("label is required".into()))?)
        .map_err(|e| CmdError::Params(e.to_string()))?;
    if let Some(label) = &label {
        label.validate().map_err(|e| CmdError::Params(e.into()))?;
    }
    // Imported sections can use formats beyond the dialog's five common choices.
    let format = v
        .get("format")
        .map(|value| {
            let name = value.as_str().ok_or_else(|| CmdError::Params("page number format must be a string".into()))?;
            let format = NumFormat::from_ooxml(name);
            if format.ooxml() != name {
                return Err(CmdError::Params("unknown page number format".into()));
            }
            Ok(format)
        })
        .transpose()?;
    let start = match v.get("start") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(
            value.as_u64().filter(|n| *n <= 100_000).ok_or_else(|| CmdError::Params("start must be null or an integer from 0 to 100000".into()))?
                as u32,
        )),
    };
    let new = v.get("new").and_then(Value::as_bool).unwrap_or(false);
    let mut index = match v.get("section") {
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < s.doc.sections().len())
            .ok_or_else(|| CmdError::Params("unknown section".into()))?,
        None => focus_section(s),
    };
    if new {
        let from = v.get("from").and_then(Value::as_str).unwrap_or("page");
        if !matches!(from, "page" | "cursor") {
            return Err(CmdError::Params("choose a body cursor or a document page".into()));
        }
        let at = start_position(s, from == "page")?;
        index = split(s, at)?;
    }
    let end = s.doc.sections().get(index).map(|(end, _)| *end).ok_or_else(|| CmdError::Params("unknown section".into()))?;
    // Each section gets private copies of its effective header/footer stories. Preserve later
    // linked sections before changing this one, so their page numbers and text cannot change.
    if label.is_some() {
        isolate(s, index)?;
    }
    let section = s.doc.section_mut(end);
    if let Some(format) = format {
        section.page_num_format = format;
    }
    if let Some(start) = start {
        section.page_num_start = start;
    }
    section.page_num_set = true;
    section.page_label = label;
    if new {
        section.start = SectionStart::NextPage;
    }
    ensure_page_field(s, end)?;
    Ok(json!({"section": index}))
}

pub(crate) fn focus_section(s: &mut Session) -> usize {
    if s.sel.focus.story == StoryRef::Body {
        return super::section_index_of_focus(s);
    }
    let hint = s.page_hint;
    s.layout().pages.get(hint).map(|page| page.section).unwrap_or(0)
}

pub(super) fn start_position(s: &mut Session, from_page: bool) -> Result<Pos, CmdError> {
    if from_page {
        return page_start(s);
    }
    if s.sel.focus.story != StoryRef::Body || s.sel.focus.path.depth() != 0 {
        return Err(CmdError::Params("place a body cursor before or after the table to start a segment".into()));
    }
    Ok(s.sel.focus.clone())
}

fn page_start(s: &mut Session) -> Result<Pos, CmdError> {
    let focus = s.sel.focus.clone();
    let hint = s.page_hint;
    let layout = s.layout();
    let page = if focus.story == StoryRef::Body { layout.page_of(&focus).unwrap_or(hint) } else { hint };
    let page = layout.pages.get(page).ok_or_else(|| CmdError::Params("no current document page".into()))?;
    for item in &page.items {
        if let wordcraft_layout::Placed::Lines { story: StoryRef::Body, path, para, l0, .. } = item {
            if path.depth() > 0 {
                return Err(CmdError::Params("place a body cursor before or after the table to start a segment".into()));
            }
            let off = para.lines.get(*l0).map(|line| line.start).unwrap_or(0);
            return Ok(Pos { story: StoryRef::Body, path: path.clone(), off });
        }
    }
    Ok(Pos { story: StoryRef::Body, path: Path::top(page.first_block), off: 0 })
}

fn split(s: &mut Session, at: Pos) -> Result<usize, CmdError> {
    let block = at.path.0.first().copied().unwrap_or(0) as usize;
    if block >= s.doc.body.len() || at.story != StoryRef::Body || at.path.depth() != 0 {
        return Err(CmdError::Params("segment boundary must be in the document body".into()));
    }
    let section = s.doc.section_index_of(block);
    let props = s.doc.sections().get(section).map(|(_, sec)| (*sec).clone()).unwrap_or_default();
    if at.off == 0 {
        if block == 0 || s.doc.sections().iter().any(|(end, _)| end.saturating_add(1) == block) {
            s.sel = Selection::caret(at);
            return Ok(section);
        }
        if matches!(s.doc.body.get(block - 1).map(|b| &**b), Some(Block::Para(_))) {
            s.doc.para_mut(StoryRef::Body, &Path::top(block - 1))?.section = Some(Box::new(props));
            s.sel = Selection::caret(at);
        } else {
            let mut marker = Paragraph::new();
            marker.section = Some(Box::new(props));
            s.doc.insert_block(StoryRef::Body, &Path::top(block), Block::Para(marker))?;
            s.sel = Selection::caret(Pos { path: Path::top(block + 1), ..at });
        }
    } else {
        s.sel = Selection::caret(at);
        super::breaks(s, &json!({"kind":"nextPage"}))?;
    }
    Ok(s.doc.section_index_of(s.sel.focus.path.0.first().copied().unwrap_or(0) as usize))
}

fn effective(s: &Session, index: usize, footer: bool) -> HeaderSet {
    let mut set = HeaderSet::default();
    for (_, sec) in s.doc.sections().into_iter().take(index.saturating_add(1)) {
        let own = if footer { sec.footers } else { sec.headers };
        set.default = own.default.or(set.default);
        set.first = own.first.or(set.first);
        set.even = own.even.or(set.even);
    }
    set
}

fn isolate(s: &mut Session, index: usize) -> Result<(), CmdError> {
    let ends: Vec<_> = s.doc.sections().into_iter().map(|(end, _)| end).collect();
    for footer in [false, true] {
        let inherited = effective(s, index, footer);
        if let Some(end) = ends.get(index + 1) {
            let sec = s.doc.section_mut(*end);
            let next = if footer { &mut sec.footers } else { &mut sec.headers };
            next.default = next.default.or(inherited.default);
            next.first = next.first.or(inherited.first);
            next.even = next.even.or(inherited.even);
        }
        let mut own = inherited;
        let named = s.doc.sections().get(index).map(|(_, sec)| if footer { sec.footers } else { sec.headers }).unwrap_or_default();
        let mut copies = std::collections::HashMap::new();
        for slot in [&mut own.default, &mut own.first, &mut own.even] {
            let Some(id) = *slot else { continue };
            let named_here = [named.default, named.first, named.even].contains(&Some(id));
            let shared = s.doc.sections().iter().enumerate().any(|(i, (_, sec))| {
                i != index
                    && [sec.headers.default, sec.headers.first, sec.headers.even, sec.footers.default, sec.footers.first, sec.footers.even]
                        .contains(&Some(id))
            });
            if named_here && !shared {
                continue;
            }
            if let Some(copy) = copies.get(&id) {
                *slot = Some(*copy);
            } else if s.doc.parts.contains_key(&id) {
                let copy = s.doc.copy_part(id)?;
                copies.insert(id, copy);
                *slot = Some(copy);
            }
        }
        if let Some(end) = ends.get(index) {
            let sec = s.doc.section_mut(*end);
            if footer {
                sec.footers = own;
            } else {
                sec.headers = own;
            }
        }
    }
    Ok(())
}

fn ensure_page_field(s: &mut Session, end: usize) -> Result<(), CmdError> {
    let section = s.doc.section_mut(end).clone();
    if section.page_label.is_none() {
        return Ok(());
    }
    for slot in 0..3 {
        let section = s.doc.section_mut(end).clone();
        if (slot == 1 && !section.title_page) || (slot == 2 && !s.doc.settings.even_odd_headers) {
            continue;
        }
        let get = |set: HeaderSet| match slot {
            0 => set.default,
            1 => set.first,
            _ => set.even.or(set.default),
        };
        let has_page = [get(section.headers), get(section.footers)]
            .into_iter()
            .flatten()
            .any(|id| crate::cmd::insert::story_has_page_field(&s.doc, StoryRef::Part(id)));
        if has_page {
            continue;
        }
        let mut para = Paragraph::new().styled("Footer");
        para.props.align = Some(wordcraft_doc::props::Align::Center);
        para.insert_object(
            0,
            InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )?;
        let id = if let Some(id) = get(section.footers) {
            id
        } else {
            // None means "inherit", not "empty". Pin an empty footer in the following
            // section before adding a previously absent story here.
            let next = s.doc.sections().into_iter().skip_while(|(e, _)| *e != end).nth(1).map(|(e, sec)| (e, get(sec.footers)));
            if let Some((next_end, None)) = next {
                let empty = s.doc.add_part(PartKind::Footer, Vec::new());
                let feet = &mut s.doc.section_mut(next_end).footers;
                match slot {
                    0 => feet.default = Some(empty),
                    1 => feet.first = Some(empty),
                    _ => feet.even = Some(empty),
                }
            }
            let id = s.doc.add_part(PartKind::Footer, Vec::new());
            let feet = &mut s.doc.section_mut(end).footers;
            match slot {
                0 => feet.default = Some(id),
                1 => feet.first = Some(id),
                _ => feet.even = Some(id),
            }
            id
        };
        let mut blocks = s.doc.parts.get(&id).map(|p| p.blocks.clone()).unwrap_or_default();
        blocks.push(std::sync::Arc::new(Block::Para(para)));
        s.doc.set_story(StoryRef::Part(id), blocks)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
