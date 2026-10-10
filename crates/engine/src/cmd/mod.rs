//! Command implementations, one module per area. Each module exposes `specs()`.

pub mod caret;
pub mod citations;
pub mod design;
pub mod edit;
pub mod equation;
pub mod file;
pub mod format;
pub mod insert;
pub mod mailings;
pub mod objects;
pub mod page;
pub mod para;
pub mod references;
pub mod review;
pub mod speech;
pub mod table;
pub mod table_style;
pub mod text;
pub mod tools;
pub mod view;

use serde_json::{Value, json};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Pos, Revision, RevisionKind, StoryRef};

use crate::{CmdError, CmdResult, Registry, Session};

/// The full registry.
pub fn registry() -> Registry {
    let mut v = Vec::new();
    v.extend(text::specs());
    v.extend(caret::specs());
    v.extend(edit::specs());
    v.extend(format::specs());
    v.extend(para::specs());
    v.extend(view::specs());
    v.extend(insert::specs());
    v.extend(equation::specs());
    v.extend(page::specs());
    v.extend(table::specs());
    v.extend(table_style::specs());
    v.extend(review::specs());
    v.extend(file::specs());
    v.extend(design::specs());
    v.extend(references::specs());
    v.extend(mailings::specs());
    v.extend(citations::specs());
    v.extend(objects::specs());
    v.extend(tools::specs());
    v.extend(speech::specs());
    Registry::new(v)
}

/// Parse a position from JSON (`{"story":"body","path":[0],"off":3}` or `{"block":0,"off":3}`).
pub fn parse_pos(v: &Value) -> Option<Pos> {
    if let Ok(p) = serde_json::from_value::<Pos>(v.clone()) {
        return Some(p);
    }
    let block = v.get("block")?.as_u64()? as usize;
    let off = v.get("off").and_then(Value::as_u64).unwrap_or(0) as usize;
    Some(Pos::body(block, off))
}

pub fn pos_json(p: &Pos) -> Value {
    serde_json::to_value(p).unwrap_or(Value::Null)
}

/// The standard result: the selection after the command.
pub fn sel_result(s: &Session) -> CmdResult {
    Ok(json!({"anchor": pos_json(&s.sel.anchor), "focus": pos_json(&s.sel.focus)}))
}

/// Delete the selection (respecting track changes); returns the collapsed position.
pub fn delete_selection(s: &mut Session) -> Result<Pos, CmdError> {
    let (a, b) = s.sel.ordered();
    if a == b {
        return Ok(a);
    }
    let at = if s.doc.settings.track_changes { track_delete(s, &a, &b)? } else { s.doc.delete_range(&a, &b)? };
    s.sel = crate::Selection::caret(at.clone());
    Ok(at)
}

/// A revision record for the current author.
pub fn new_revision(s: &mut Session, kind: RevisionKind) -> u32 {
    let date = now_iso();
    if let Some((i, _)) = s.doc.revisions.iter().enumerate().rev().find(|(_, r)| r.kind == kind && r.author == s.author && r.date == date) {
        return i as u32;
    }
    s.doc.revisions.push(Revision { kind, author: s.author.clone(), date });
    (s.doc.revisions.len() - 1) as u32
}

/// Whether the paragraph at `path` is followed by another paragraph in the same container (so
/// its paragraph mark can be removed by joining the two).
pub fn has_next_para(s: &Session, story: StoryRef, path: &wordcraft_doc::Path) -> bool {
    matches!(s.doc.block(story, &path.with_last(path.last().saturating_add(1))), Some(wordcraft_doc::Block::Para(_)))
}

/// Remove the paragraph mark of the paragraph at `path`: the next paragraph in the same container
/// joins it. The joined paragraph keeps this paragraph's properties and ends with the next one's
/// mark (and that mark's revisions). Returns `false`, changing nothing, when no paragraph follows
/// in the container (a story's or cell's last mark can't be removed).
pub fn join_next_para(s: &mut Session, story: StoryRef, path: &wordcraft_doc::Path) -> Result<bool, CmdError> {
    if !has_next_para(s, story, path) {
        return Ok(false);
    }
    let next = path.with_last(path.last().saturating_add(1));
    let len = s.doc.para(story, path).map(|p| p.len()).unwrap_or(0);
    let revs = s.doc.para(story, &next).map(|p| (p.mark.ins, p.mark.del)).unwrap_or_default();
    s.doc.delete_range(&Pos { story, path: path.clone(), off: len }, &Pos { story, path: next, off: 0 })?;
    let p = s.doc.para_mut(story, path)?;
    (p.mark.ins, p.mark.del) = revs;
    p.touch();
    Ok(true)
}

/// Tracked deletion: own insertions are removed, other text is marked deleted.
fn track_delete(s: &mut Session, a: &Pos, b: &Pos) -> Result<Pos, CmdError> {
    let rid = new_revision(s, RevisionKind::Delete);
    let author = s.author.clone();
    // Remove text this author inserted (it never existed for the reader); mark the rest.
    let paths = s.doc.paths_between(a, b);
    for path in paths.iter().rev() {
        let Some(p) = s.doc.para(a.story, path) else { continue };
        let from = if *path == a.path { a.off } else { 0 };
        let to = if *path == b.path { b.off } else { p.len() };
        // Text already deleted keeps its deletion (and its author), like Word.
        let ranges: Vec<(usize, usize, bool)> = p
            .run_ranges()
            .filter(|(r, c)| r.end > from && r.start < to && c.del.is_none())
            .map(|(r, c)| {
                let own = c.ins.and_then(|i| s.doc.revisions.get(i as usize)).is_some_and(|rv| rv.author == author);
                (r.start.max(from), r.end.min(to), own)
            })
            .collect();
        let para = s.doc.para_mut(a.story, path)?;
        for (x, y, own) in ranges.into_iter().rev() {
            if own {
                para.delete(x, y)?;
            } else {
                para.format(x, y, &|c| c.del = Some(rid))?;
            }
        }
        // The paragraph mark is inside the range unless this is its last paragraph. Like text,
        // a mark this author inserted is removed (the paragraphs join again), any other is
        // marked deleted; one already deleted keeps its deletion. Done after the text so a join
        // can't pull the next paragraph's text into this one's range.
        if *path != b.path && has_next_para(s, a.story, path) {
            let (ins, del) = s.doc.para(a.story, path).map(|p| (p.mark.ins, p.mark.del)).unwrap_or_default();
            if del.is_none() {
                let own = ins.and_then(|i| s.doc.revisions.get(i as usize)).is_some_and(|rv| rv.author == author);
                if own {
                    join_next_para(s, a.story, path)?;
                } else {
                    let p = s.doc.para_mut(a.story, path)?;
                    p.mark.del = Some(rid);
                    p.touch();
                }
            }
        }
    }
    Ok(a.clone())
}

/// Insert typed text at the caret (replacing the selection), with pending formatting and track changes.
pub fn type_text(s: &mut Session, text: &str) -> Result<(), CmdError> {
    let mut props = s.typing_props();
    delete_selection(s)?;
    if s.doc.settings.track_changes {
        props.ins = Some(new_revision(s, RevisionKind::Insert));
        props.del = None;
    } else {
        props.ins = None;
        props.del = None;
    }
    let mut at = s.sel.focus.clone();
    let mut first = true;
    for part in text.split(['\r', '\u{2029}']) {
        if !first {
            at = split_para(s, &at)?;
        }
        first = false;
        let part = part.replace('\u{000B}', "\n");
        if !part.is_empty() {
            at = s.doc.insert_text(&at, &part, &props)?;
        }
    }
    s.sel = crate::Selection::caret(at);
    s.goal_x = None;
    Ok(())
}

/// Take a paragraph out of its list like Word does when Enter or Backspace ends a list: a
/// "List Paragraph" goes back to Normal; any other style keeps itself with numbering switched off.
pub fn leave_list(para: &mut wordcraft_doc::Paragraph) {
    if para.props.style.as_deref() == Some("ListParagraph") {
        para.props.style = None;
        para.props.numbering = None;
    } else {
        // `num: 0` overrides numbering a style may carry.
        para.props.numbering = Some(wordcraft_doc::props::NumRef { num: 0, level: 0 });
    }
    para.props.indent_left = None;
    para.props.indent_first = None;
    para.touch();
}

/// Split the paragraph at `at` like Enter does: an empty list paragraph leaves the list, the
/// next paragraph gets the style's "next" style when Enter is at the end.
pub fn split_para(s: &mut Session, at: &Pos) -> Result<Pos, CmdError> {
    // `num: 0` means "explicitly not in a list", so it isn't a list item.
    let (at_end, style, empty_list) = match s.doc.para_at(at) {
        Some(p) => (at.off >= p.len(), p.props.style.clone(), p.props.numbering.filter(|n| n.num != 0 && p.is_empty())),
        None => return Err(CmdError::Failed("no paragraph at caret".into())),
    };
    if let Some(n) = empty_list {
        // Enter on an empty list item: a nested item moves up a level, a top-level one ends
        // the list (Word behaviour).
        let para = s.doc.para_mut(at.story, &at.path)?;
        if n.level > 0 {
            para.props.numbering = Some(wordcraft_doc::props::NumRef { num: n.num, level: n.level - 1 });
            para.touch();
        } else {
            leave_list(para);
        }
        return Ok(at.clone());
    }
    let mark_revs = s.doc.para_at(at).map(|p| (p.mark.ins, p.mark.del)).unwrap_or_default();
    let new = s.doc.split_paragraph(at)?;
    if at_end && let Some(st) = style.as_deref() {
        let next = s.doc.styles.get(st).and_then(|x| x.next.clone());
        if let Some(n) = next
            && n != st
        {
            let p = s.doc.para_mut(new.story, &new.path)?;
            p.props.style = Some(n);
            p.props.numbering = None;
            // A heading's character formatting doesn't carry into body text.
            p.mark = CharProps::default();
        }
    }
    if s.doc.settings.track_changes {
        let rid = new_revision(s, RevisionKind::Insert);
        let p = s.doc.para_mut(at.story, &at.path)?;
        p.mark.ins = Some(rid);
        // The paragraph after the split ends with the original mark: it keeps that mark's
        // revisions, never those of the text at the split point (which would credit the split
        // to the author who inserted that text).
        let t = s.doc.para_mut(new.story, &new.path)?;
        (t.mark.ins, t.mark.del) = mark_revs;
    }
    Ok(new)
}

/// ISO-8601 timestamp (UTC, seconds) for `secs` since the Unix epoch.
pub fn iso_from_unix_secs(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let t = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t / 60 % 60, t % 60)
}

/// Seconds since the Unix epoch (UTC): the system clock, or the browser's on the web.
pub fn now_unix() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        // Clamp a NaN or pre-epoch clock to 0 rather than fail.
        if ms.is_finite() && ms > 0.0 { (ms / 1000.0) as u64 } else { 0 }
    }
}

/// ISO-8601 timestamp (UTC, seconds).
pub fn now_iso() -> String {
    iso_from_unix_secs(now_unix())
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's algorithm).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Story given in params (`"story": "body"` / `{"part": 3}`), else the caret's.
pub fn story_param(s: &Session, v: &Value) -> StoryRef {
    v.get("story").and_then(|x| serde_json::from_value(x.clone()).ok()).unwrap_or(s.sel.focus.story)
}

#[cfg(test)]
mod iso_tests {
    use super::iso_from_unix_secs;

    #[test]
    fn formats_epoch_and_known_timestamps() {
        assert_eq!(iso_from_unix_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix_secs(1_760_082_785), "2025-10-10T07:53:05Z");
    }

    #[test]
    fn formats_leap_day_and_year_end() {
        assert_eq!(iso_from_unix_secs(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(iso_from_unix_secs(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_from_unix_secs(1_735_689_599), "2024-12-31T23:59:59Z");
    }
}
