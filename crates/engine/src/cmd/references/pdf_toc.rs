//! Admission checks for rebuilding a TOC in a disposable PDF session.

use super::*;

const MAX_INSTRUCTION_BYTES: usize = 4096;
const MAX_ENTRIES: usize = 10_000;

fn failed(reason: &str) -> CmdError {
    CmdError::Failed(format!("PDF TOC refresh: {reason}"))
}

fn is_toc(instr: &str) -> bool {
    instr.split_whitespace().next().is_some_and(|name| name.eq_ignore_ascii_case("TOC"))
}

fn valid_levels(text: &str) -> bool {
    let mut parts = text.split('-');
    let level = |s: &str| s.trim().parse::<u8>().ok().filter(|n| (1..=9).contains(n));
    let Some(a) = parts.next().and_then(level) else { return false };
    let b = match parts.next() {
        Some(s) => match level(s) {
            Some(b) => b,
            None => return false,
        },
        None => a,
    };
    parts.next().is_none() && a <= b
}

fn check_switches(code: &FieldCode) -> Result<(), CmdError> {
    let mut seen = std::collections::HashSet::new();
    for (key, arg) in &code.switches {
        if !seen.insert(*key) {
            return Err(failed("repeated TOC switches are unsupported"));
        }
        let valid = match key {
            'h' | 'u' | 'z' => arg.is_none(),
            'o' => arg.as_deref().is_none_or(valid_levels),
            'l' => arg.as_deref().is_some_and(valid_levels),
            'n' => arg.as_deref().is_none_or(valid_levels),
            'p' => arg.is_some(),
            'f' => true,
            't' => arg.as_deref().is_some_and(|text| {
                let text = text.trim_end_matches(|c: char| matches!(c, ',' | ';') || c.is_whitespace());
                let parts: Vec<_> = text.split([',', ';']).collect();
                !parts.is_empty()
                    && parts.len() % 2 == 0
                    && parts.chunks(2).all(|pair| {
                        pair.first().is_some_and(|s| !s.trim().is_empty()) && pair.get(1).is_some_and(|s| !s.contains('-') && valid_levels(s))
                    })
            }),
            _ => false,
        };
        if !valid {
            return Err(failed("unsupported or malformed TOC switch"));
        }
    }
    if !code.args.is_empty() || (code.has('l') && !code.has('f')) || !['o', 'u', 't', 'f'].iter().any(|key| code.has(*key)) {
        return Err(failed("TOC needs supported heading, style or TC selectors"));
    }
    Ok(())
}

struct Table {
    start: usize,
    code: FieldCode,
    split_end: Option<Pos>,
}

fn supported_tail(para: &Paragraph, end: &Pos) -> bool {
    if end.off != 0 || para.props.style.as_deref().is_some_and(|style| style.starts_with("TOC")) {
        return false;
    }
    if para.text == format!("{OBJ}\u{000c}") && matches!(para.objects.as_slice(), [InlineObject::FieldEnd]) {
        return true;
    }
    para.text == OBJ.to_string().repeat(3)
        && matches!(para.objects.as_slice(), [InlineObject::FieldEnd, InlineObject::BookmarkStart { name }, InlineObject::BookmarkEnd { name: other }] if !name.is_empty() && name == other)
}

fn internal_link_cache(object: &InlineObject) -> bool {
    let InlineObject::Field { instr, locked: false, code: false, form, .. } = object else { return false };
    if instr.len() > MAX_INSTRUCTION_BYTES || !form.is_empty() {
        return false;
    }
    let code = FieldCode::parse(instr);
    code.name == "HYPERLINK" && code.args.is_empty() && matches!(code.switches.as_slice(), [('l', Some(target))] if !target.trim().is_empty())
}

#[derive(serde::Serialize)]
enum RefreshState {
    Refreshed,
    #[serde(rename = "NoTOC")]
    NoToc,
    LockedPreserved,
}

enum Admission {
    Refresh(Table),
    Keep(RefreshState),
}

fn report(state: RefreshState) -> Value {
    json!({"tocRefresh": state})
}

/// Inspect every TOC before touching the first one. The generator only updates one body TOC.
fn admit(s: &Session) -> Result<Admission, CmdError> {
    if s.doc.settings.protection.is_some() {
        return Ok(Admission::Keep(RefreshState::LockedPreserved));
    }
    let mut found = None;
    for story in std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().copied().map(StoryRef::Part)) {
        for path in s.doc.para_paths(story) {
            let Some(para) = s.doc.para(story, &path) else { continue };
            for (off, object) in para.object_offsets().into_iter().zip(&para.objects) {
                let (instr, locked) = match object {
                    InlineObject::Field { instr, locked, .. } | InlineObject::FieldStart { instr, locked, .. } if is_toc(instr) => (instr, *locked),
                    _ => continue,
                };
                if instr.len() > MAX_INSTRUCTION_BYTES {
                    return Err(failed("TOC instruction exceeds supported limits"));
                }
                // Figure lists have their own updater and stay unchanged in this mode.
                if FieldCode::parse(instr).has('c') {
                    continue;
                }
                if found.is_some() {
                    return Err(failed("multiple TOCs are unsupported"));
                }
                found = Some((Pos::new(story, path.clone(), off), instr.clone(), locked, object));
            }
        }
    }
    let Some((at, instr, locked, object)) = found else { return Ok(Admission::Keep(RefreshState::NoToc)) };
    let ranges = s.doc.field_ranges(at.story);
    if locked || ranges.iter().any(|range| range.locked && range.start < at && at < range.end) {
        return Ok(Admission::Keep(RefreshState::LockedPreserved));
    }
    let controls = s.doc.control_ranges(at.story);
    if controls.iter().any(|range| !range.control.lock.can_edit() && range.start < at && at < range.end) {
        return Ok(Admission::Keep(RefreshState::LockedPreserved));
    }
    if at.story != StoryRef::Body || at.path.depth() != 0 || ranges.iter().any(|range| range.start < at && at < range.end) {
        return Err(failed("only a top-level body TOC is supported"));
    }
    if instr.len() > MAX_INSTRUCTION_BYTES || s.doc.has_unbalanced_field_ranges() {
        return Err(failed("TOC instruction or field boundaries exceed supported limits"));
    }
    if matches!(object, InlineObject::Field { code: true, .. }) {
        return Err(failed("TOC field-code display is unsupported"));
    }
    let code = FieldCode::parse(&instr);
    check_switches(&code)?;
    let end = if matches!(object, InlineObject::FieldStart { .. }) {
        let range = ranges.iter().find(|range| range.start == at).ok_or_else(|| failed("TOC field end is missing"))?;
        if range.depth != 0 || range.end.path.depth() != 0 {
            return Err(failed("nested TOC boundaries are unsupported"));
        }
        Some(range.end.clone())
    } else {
        None
    };
    let last = end.as_ref().map_or_else(|| toc_range(s).map_or(at.path.last(), |(_, end)| end), |end| end.path.last());
    let mut split_end = None;
    if let Some(end) = &end {
        let para = s.doc.para_at(end).ok_or_else(|| failed("TOC end paragraph is missing"))?;
        let after = end.off.checked_add(OBJ.len_utf8()).ok_or_else(|| failed("TOC field offset overflow"))?;
        let tail = para.text.get(after..).ok_or_else(|| failed("TOC field end is invalid"))?;
        if !tail.is_empty() && !supported_tail(para, end) {
            return Err(failed("text after the TOC field end is unsupported"));
        }
        if !tail.is_empty() || para.section.is_some() {
            split_end = Some(end.clone());
        }
        if s.doc
            .body
            .get(last + 1)
            .and_then(|block| block.as_para())
            .is_some_and(|para| para.props.style.as_deref().is_some_and(|style| style.starts_with("TOC") && style != "TOCHeading"))
        {
            return Err(failed("TOC-styled content outside the field is ambiguous"));
        }
    } else if s.doc.para_at(&at).is_none_or(|para| para.props.style.as_deref() != Some("TOCHeading"))
        || !matches!(object, InlineObject::Field { result, form, .. } if result.is_empty() && form.is_empty())
    {
        return Err(failed("inline TOC cache ownership is ambiguous"));
    }
    for i in at.path.last()..=last {
        let para = s.doc.body.get(i).and_then(|block| block.as_para()).ok_or_else(|| failed("tables inside a TOC are unsupported"))?;
        if para.section.is_some() && end.as_ref().is_none_or(|end| i != end.path.last() || end.off != 0) {
            return Err(failed("section breaks inside TOC content are unsupported"));
        }
        if para.section.is_some() && para.props.style.as_deref().is_some_and(|style| style.starts_with("TOC")) {
            return Err(failed("TOC-styled section marks are unsupported"));
        }
        for (off, object) in para.object_offsets().into_iter().zip(&para.objects) {
            if i == at.path.last() && off == at.off {
                continue;
            }
            if end.as_ref().is_some_and(|end| i == end.path.last() && off == end.off && matches!(object, InlineObject::FieldEnd)) {
                continue;
            }
            if end.as_ref().is_some_and(|end| i == end.path.last() && off > end.off) {
                continue;
            }
            match object {
                InlineObject::Field { locked: true, .. } | InlineObject::FieldStart { locked: true, .. } => {
                    return Ok(Admission::Keep(RefreshState::LockedPreserved));
                }
                InlineObject::Field { instr, .. } if FieldCode::parse(instr).name == "PAGEREF" => {}
                object if internal_link_cache(object) => {}
                _ => return Err(failed("TOC contains unsupported objects or field nesting")),
            }
        }
    }
    let entries = toc_entries(s, &code);
    if entries.len() > MAX_ENTRIES {
        return Err(failed("too many TOC entries"));
    }
    for entry in entries {
        let Some(para) = s.doc.para(StoryRef::Body, &entry.path) else { continue };
        let target = Pos::new(StoryRef::Body, entry.path.clone(), entry.lead);
        if controls.iter().any(|range| !range.control.lock.can_edit() && range.start < target && target < range.end) {
            return Ok(Admission::Keep(RefreshState::LockedPreserved));
        }
        if para.runs.iter().any(|run| {
            let props = s.doc.styles.resolve_char(para.props.style.as_deref(), &run.props);
            props.hidden || props.del.is_some()
        }) {
            return Err(failed("hidden or deleted TOC headings are unsupported"));
        }
    }
    let moves_to_heading = end.is_some()
        && at.off == 0
        && at
            .path
            .last()
            .checked_sub(1)
            .and_then(|i| s.doc.body.get(i))
            .and_then(|block| block.as_para())
            .is_some_and(|para| para.props.style.as_deref() == Some("TOCHeading"));
    let start = at.path.last().saturating_sub(usize::from(moves_to_heading));
    if moves_to_heading && s.doc.body.get(start).and_then(|block| block.as_para()).is_some_and(|para| para.section.is_some()) {
        return Err(failed("TOC heading ends a different section"));
    }
    Ok(Admission::Refresh(Table { start, code, split_end }))
}

pub(super) fn refresh(s: &mut Session, v: &Value) -> CmdResult {
    let table = match admit(s)? {
        Admission::Keep(state) => return Ok(report(state)),
        Admission::Refresh(table) => table,
    };
    let sections: Vec<_> = s.doc.sections().into_iter().map(|(_, section)| section.clone()).collect();
    // Detach the admitted tail before deletion joins it onto the TOC's start paragraph.
    // Keep its formatting and section after the regenerated entries.
    if let Some(end) = table.split_end {
        let mark = s.doc.para_at(&end).ok_or_else(|| failed("TOC end paragraph is missing"))?.mark.clone();
        let off = end.off.checked_add(OBJ.len_utf8()).ok_or_else(|| failed("TOC field offset overflow"))?;
        let tail = s.doc.split_paragraph(&Pos { off, ..end })?;
        s.doc.para_mut(tail.story, &tail.path)?.mark = mark;
    }
    update_toc(s)?;
    if s.doc.sections().into_iter().map(|(_, section)| section.clone()).collect::<Vec<_>>() != sections {
        return Err(failed("TOC refresh changed section properties"));
    }
    // Page references can reflow after regeneration. Check the printed result rather than
    // assuming the generator's two passes have settled; an error rolls back this command.
    let markup = crate::cmd::file::export_options(s, v);
    let opts =
        wordcraft_layout::LayoutOptions { hide_deleted: !markup.markup, comment_balloons: markup.markup && markup.comments, ..Default::default() };
    let print = crate::print_fields::prepare(&s.doc, &opts).map_err(CmdError::Failed)?;
    let printed = Session::new(print.doc);
    let entries = toc_entries(&printed, &table.code);
    let Some((start, end)) = toc_range(&printed) else { return Err(failed("refreshed TOC is missing")) };
    if start != table.start || end.saturating_sub(start) != entries.len() {
        return Err(failed("TOC entry boundaries did not settle"));
    }
    let no_pages = table.code.has('n').then(|| table.code.arg('n').and_then(level_range).unwrap_or((1, 9)));
    let sep = table.code.arg('p').unwrap_or("\t");
    for (i, entry) in entries.iter().enumerate() {
        let page = print
            .layout
            .caret(&Pos::new(StoryRef::Body, entry.path.clone(), entry.lead))
            .and_then(|caret| print.layout.pages.get(caret.page))
            .map(|page| page.number)
            .ok_or_else(|| failed("TOC heading has no printed page"))?;
        let show_page = no_pages.is_none_or(|(a, b)| !(a..=b).contains(&(entry.level + 1)));
        let expected = if show_page { format!("{}{sep}{page}", entry.text) } else { entry.text.clone() };
        if printed.doc.body.get(start + 1 + i).and_then(|block| block.as_para()).is_none_or(|para| para.plain_text() != expected) {
            return Err(failed("TOC page numbers did not settle in the final print layout"));
        }
    }
    Ok(report(RefreshState::Refreshed))
}

#[cfg(test)]
mod tests;
