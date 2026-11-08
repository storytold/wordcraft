use super::format::fmt;
use super::{CmdError, CmdResult, LineSpacing, ListKind, NumRef, ParaProps, Pos, Session, Style, StyleKind, Value, json, p, sel_result};
use std::collections::HashMap;
use std::sync::Arc;
use wordcraft_doc::Path;
use wordcraft_doc::numbering::Counters;
use wordcraft_doc::section::NumFormat;

/// Toggle lists using effective paragraph numbering; an explicit off request removes numbering.
pub(super) fn list(s: &mut Session, v: &Value, default: ListKind) -> CmdResult {
    let kind = p::str(v, "kind").and_then(ListKind::parse).unwrap_or(default);
    let off = p::bool(v, "off").unwrap_or(false);
    let a = s.sel.ordered().0;
    // Toggle: if every selected paragraph already has this kind of list, remove it.
    let current: Vec<Option<NumRef>> = crate::cmd::selected_paths(s)
        .iter()
        .map(|p| s.doc.para(a.story, p).and_then(|x| s.doc.styles.resolve_para(&x.props).numbering).filter(|n| n.num != 0))
        .collect();
    let same_kind = |n: &NumRef| {
        s.doc.numbering.level(n.num, n.level).is_some_and(|l| {
            let want = wordcraft_doc::numbering::levels_for(kind);
            want.get(usize::from(n.level)).is_some_and(|w| {
                (w.format == wordcraft_doc::section::NumFormat::Bullet) == (l.format == wordcraft_doc::section::NumFormat::Bullet)
                    && (p::str(v, "kind").is_none() || w.text == l.text)
            })
        })
    };
    let remove = off || (!current.is_empty() && current.iter().all(|n| n.as_ref().is_some_and(same_kind)));
    if remove {
        let keep_list_style = s.doc.styles.resolve_para(&ParaProps { style: Some("ListParagraph".into()), ..Default::default() }).numbering.is_some();
        return fmt(s, &|p| {
            p.numbering = Some(NumRef { num: 0, level: 0 });
            p.indent_left = None;
            p.indent_first = None;
            p.indent_left_chars = None;
            p.indent_first_chars = None;
            if p.style.as_deref() == Some("ListParagraph") && !keep_list_style {
                p.style = None;
            }
        });
    }
    // Continue the list of the previous paragraph if it has the same kind; else a new list.
    let prev_num = s
        .doc
        .prev_para(a.story, &a.path)
        .and_then(|q| s.doc.para(a.story, &q).and_then(|x| x.props.numbering))
        .filter(|n| n.num != 0 && same_kind(&NumRef { level: 0, ..*n }));
    let num = match prev_num {
        Some(n) => n.num,
        None => match kind {
            ListKind::Bullet | ListKind::BulletChar(_) => match s.doc.numbering.find_kind(kind) {
                Some(n) => n,
                None => Arc::make_mut(&mut s.doc.numbering).try_add_list(kind)?,
            },
            _ => Arc::make_mut(&mut s.doc.numbering).try_add_list(kind)?,
        },
    };
    fmt(s, &|p| {
        let level = p.numbering.filter(|n| n.num != 0).map(|n| n.level).unwrap_or(0);
        p.numbering = Some(NumRef { num, level });
        p.indent_left = None;
        p.indent_first = None;
        p.indent_left_chars = None;
        p.indent_first_chars = None;
        if p.style.is_none() || p.style.as_deref() == Some("Normal") {
            p.style = Some("ListParagraph".into());
        }
    })?;
    strip_typed_labels(s);
    sel_result(s)
}

/// A list label typed at a paragraph's start, as [`strip_typed_labels`] reads it: how many bytes
/// of the paragraph it takes (the label and the space or tab after it, and any leading spaces or
/// tabs before it) and what it spells (`1.`, `a)`, `1.1.`).
fn typed_label(text: &str) -> Option<(usize, &str)> {
    let start = text.len().checked_sub(text.trim_start_matches([' ', '\t']).len())?;
    let rest = text.get(start..)?;
    let mut end = 0usize;
    let mut separator = false;
    for c in rest.chars() {
        match c {
            c if c.is_ascii_alphanumeric() => end = end.saturating_add(1),
            ')' => {
                end = end.saturating_add(1);
                separator = true;
                break;
            }
            '.' => {
                end = end.saturating_add(1);
                separator = true;
            }
            _ => break,
        }
    }
    // A label is a letter or digit run closed by "." or ")", then a space or tab and some text
    // (a paragraph holding only its typed label stays as it is, as in Word).
    if end == 0 || !separator {
        return None;
    }
    let after = rest.get(end..)?;
    if !after.starts_with([' ', '\t']) || after.trim_start_matches([' ', '\t']).is_empty() {
        return None;
    }
    // The whitespace after the label goes with it: Word leaves the text against the list's tab,
    // not a stray space.
    let spaces = after.len().saturating_sub(after.trim_start_matches([' ', '\t']).len());
    let taken = start.saturating_add(end).saturating_add(spaces);
    let label = rest.get(..end)?;
    (taken <= text.len()).then_some((taken, label))
}

/// Remove a typed label in front of paragraphs the applied list numbers the same way, as Word
/// does: `1. alpha` numbered becomes `1. alpha`, not `1. 1. alpha` (#6017).
///
/// A label the list does not give that paragraph — a jump of the author's, `2.` where the list
/// says `1.` — stays as text, which is also what Word does with it. Nothing is removed from a
/// bulleted list: Word keeps a typed `-` or `+` when Bullets is applied.
fn strip_typed_labels(s: &mut Session) {
    let story = s.sel.focus.story;
    let typed: Vec<(Path, usize, String)> = crate::cmd::selected_paths(s)
        .into_iter()
        .filter_map(|path| {
            let p = s.doc.para(story, &path)?;
            let (start, label) = typed_label(&p.text)?;
            Some((path, start, label.to_lowercase()))
        })
        .collect();
    if typed.is_empty() {
        return;
    }
    // What each paragraph's list label reads, counted in document order as the page does.
    let mut counters = Counters::default();
    let mut labels: HashMap<Path, String> = HashMap::new();
    for path in s.doc.para_paths(story) {
        let Some(p) = s.doc.para(story, &path) else { continue };
        let n = p.props.numbering.or_else(|| s.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0);
        if let Some(n) = n
            && let Some((label, def)) = counters.next_label(&s.doc.numbering, n.num, n.level)
            && def.format != NumFormat::Bullet
        {
            labels.insert(path, label.to_lowercase());
        }
    }
    let remove: Vec<(Path, usize)> =
        typed.into_iter().filter(|(path, _, label)| labels.get(path) == Some(label)).map(|(path, start, _)| (path, start)).collect();
    if remove.is_empty() {
        return;
    }
    // The label goes through the ordinary deletion, so Track Changes marks it like any edit.
    let saved = s.sel.clone();
    let mut shift: Vec<(Path, usize)> = Vec::new();
    for (path, len) in remove {
        let before = s.doc.para(story, &path).map(|p| p.len()).unwrap_or(0);
        s.sel = crate::Selection { anchor: Pos { story, path: path.clone(), off: 0 }, focus: Pos { story, path: path.clone(), off: len } };
        // Refused (a locked control, say): the paragraph keeps its text and only the list is
        // applied, rather than the whole command failing after the numbering was written.
        let _ = crate::cmd::delete_selection_effect(s);
        // With Track Changes the label is marked, not removed, and nothing moves. Otherwise the
        // paragraph really did lose its first bytes, so a caret after them moves with the text.
        let removed = before.saturating_sub(s.doc.para(story, &path).map(|p| p.len()).unwrap_or(0));
        if removed > 0 {
            shift.push((path, removed));
        }
    }
    // The removed text shortened the paragraphs, so the caller's caret may no longer fit.
    s.sel = saved;
    for pos in [&mut s.sel.anchor, &mut s.sel.focus] {
        if let Some((_, removed)) = shift.iter().find(|(path, _)| *path == pos.path) {
            pos.off = pos.off.saturating_sub(*removed);
        }
    }
    s.clamp_selection();
}

pub(super) fn restart_numbering(s: &mut Session, _: &Value) -> CmdResult {
    let f = s.sel.focus.clone();
    let Some(n) = s.doc.para_at(&f).and_then(|p| p.props.numbering).filter(|n| n.num != 0) else {
        return Err(CmdError::Failed("not in a numbered list".into()));
    };
    let new = Arc::make_mut(&mut s.doc.numbering).restart(n.num).ok_or_else(|| CmdError::Failed("bad list".into()))?;
    // This paragraph and the following ones of the same list use the new instance.
    let paths: Vec<_> = s.doc.para_paths(f.story).into_iter().filter(|p| *p >= f.path).collect();
    for path in paths {
        let para = s.doc.para_mut(f.story, &path)?;
        match para.props.numbering {
            Some(x) if x.num == n.num => {
                para.props.numbering = Some(NumRef { num: new, level: x.level });
                para.touch();
            }
            _ => {
                if para.props.numbering.is_none() && !para.is_empty() {
                    break;
                }
            }
        }
    }
    sel_result(s)
}

/// Manage Styles' list: paragraph, character, linked and table styles (a linked pair once, as
/// its paragraph half), sorted as recommended (priority, then name) or alphabetically.
pub fn manage_list(s: &Session, alphabetical: bool) -> Value {
    let sheet = &s.doc.styles;
    let links_back = |a: &Style, b: &str| sheet.get(b).is_some_and(|x| x.linked.as_deref() == Some(a.id.as_str()));
    let mut list: Vec<&Style> = sheet
        .styles
        .iter()
        .filter(|st| match st.kind {
            StyleKind::Numbering => false,
            // The character half of a linked pair is listed as the linked style.
            StyleKind::Character => {
                !st.linked.as_deref().is_some_and(|l| sheet.get(l).is_some_and(|x| x.kind == StyleKind::Paragraph) && links_back(st, l))
            }
            _ => true,
        })
        .collect();
    if alphabetical {
        list.sort_by_cached_key(|st| st.name.to_lowercase());
    } else {
        list.sort_by_cached_key(|st| (st.priority.unwrap_or(99), st.name.to_lowercase()));
    }
    Value::Array(
        list.into_iter()
            .map(|st| {
                let ty = match st.kind {
                    // Linked: its character half links back (or isn't in the document yet).
                    StyleKind::Paragraph
                        if st.linked.as_deref().is_some_and(|l| sheet.get(l).is_none_or(|x| x.kind == StyleKind::Character && links_back(st, l))) =>
                    {
                        "linked"
                    }
                    StyleKind::Paragraph => "paragraph",
                    StyleKind::Character => "character",
                    _ => "table",
                };
                json!({
                    "id": st.id,
                    "name": st.name,
                    "type": ty,
                    "builtIn": st.builtin || (st.kind == StyleKind::Table && crate::cmd::table_style::is_builtin_table_style(st)),
                    "inGallery": st.quick && !st.hidden && st.kind != StyleKind::Table,
                    "hidden": st.hidden,
                    "basedOn": st.based_on.as_deref().and_then(|b| sheet.get(b)).map(|b| b.name.clone()),
                    "priority": st.priority,
                    "format": own_format(st),
                })
            })
            .collect(),
    )
}

/// The formatting a style sets itself (not what it inherits), for its description.
fn own_format(st: &Style) -> Value {
    let mut f = serde_json::Map::new();
    let c = &st.chr;
    if let Some(x) = &c.font {
        f.insert("font".into(), json!(x));
    }
    if let Some(x) = c.size {
        f.insert("size".into(), json!(x));
    }
    for (k, v) in [("bold", c.bold), ("italic", c.italic), ("caps", c.caps), ("smallCaps", c.small_caps)] {
        if let Some(b) = v {
            f.insert(k.into(), json!(b));
        }
    }
    if let Some(wordcraft_doc::TextColor::Rgb(rgb)) = c.color {
        f.insert("color".into(), json!(rgb.hex()));
    }
    let pp = &st.para;
    if let Some(a) = pp.align {
        f.insert("align".into(), json!(a));
    }
    if let Some(x) = pp.space_before {
        f.insert("spaceBefore".into(), json!(x));
    }
    if let Some(x) = pp.space_after {
        f.insert("spaceAfter".into(), json!(x));
    }
    if let Some(x) = pp.indent_left {
        f.insert("indentLeft".into(), json!(x));
    }
    if let Some(LineSpacing::Multiple(m)) = pp.line_spacing {
        f.insert("lineSpacing".into(), json!(m));
    }
    Value::Object(f)
}

/// Style edits re-lay out every paragraph: the layout cache is keyed on the style sheet, so
/// nothing needs touching here (kept as the one place to hook style-change side effects).
pub fn touch_all(s: &mut Session) {
    s.dirty = true;
}

/// Which Home › Paragraph list buttons show as on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListButtons {
    pub bullets: bool,
    pub numbering: bool,
    pub multilevel: bool,
}

/// The list buttons for a paragraph with resolved `numbering` (the ribbon passes the paragraph
/// props it already resolved). The paragraph's own level decides Bullets (a bulleted level) or
/// Numbering (a numbered one); Multilevel is on too when the list numbers its levels together:
/// some level's label shows another level's number (`1.1.`) or uses legal numbering.
pub fn list_buttons(doc: &wordcraft_doc::Document, numbering: Option<NumRef>) -> ListButtons {
    let Some(n) = numbering.filter(|n| n.num != 0) else { return ListButtons::default() };
    let Some(level) = doc.numbering.level(n.num, n.level) else { return ListButtons::default() };
    let bullets = level.format == wordcraft_doc::section::NumFormat::Bullet;
    let joined = |i: u8| {
        doc.numbering.level(n.num, i).is_some_and(|l| {
            let own = u32::from(i) + 1;
            l.legal || l.text.split('%').skip(1).filter_map(|t| t.chars().next()?.to_digit(10)).any(|k| k != own)
        })
    };
    let multilevel = (0..9u8).any(joined);
    ListButtons { bullets, numbering: !bullets, multilevel }
}
