//! Command implementations, one module per area. Each module exposes `specs()`.

pub mod caret;
pub mod citations;
pub mod design;
pub mod edit;
pub mod file;
pub mod format;
pub mod insert;
pub mod mailings;
pub mod objects;
pub mod page;
pub mod para;
pub mod references;
pub mod review;
pub mod table;
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
    v.extend(page::specs());
    v.extend(table::specs());
    v.extend(review::specs());
    v.extend(file::specs());
    v.extend(design::specs());
    v.extend(references::specs());
    v.extend(mailings::specs());
    v.extend(citations::specs());
    v.extend(objects::specs());
    v.extend(tools::specs());
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
        let ranges: Vec<(usize, usize, bool)> = p
            .run_ranges()
            .filter(|(r, _)| r.end > from && r.start < to)
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
    }
    Ok(new)
}

/// ISO-8601 timestamp (UTC, seconds).
pub fn now_iso() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let days = (secs / 86_400) as i64;
        let (y, m, d) = civil_from_days(days);
        let t = secs % 86_400;
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t / 60 % 60, t % 60)
    }
    #[cfg(target_arch = "wasm32")]
    {
        "2026-01-01T00:00:00Z".to_string()
    }
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
