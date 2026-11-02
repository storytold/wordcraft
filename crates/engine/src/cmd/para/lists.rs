use super::format::fmt;
use super::{CmdError, CmdResult, LineSpacing, ListKind, NumRef, ParaProps, Session, Style, StyleKind, Value, json, p, sel_result};

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
    // Continue the nearest list above if it has the same kind, past unnumbered paragraphs in
    // between (Word: 1., a paragraph, then Numbering gives 2.); else a new list. A heading
    // numbered by its style counts as the nearest list. The paragraph right above continues
    // whatever numbered list it is in; across a gap the list must look like `kind` (1. 2. 3.
    // for the plain button), so a) b) further up isn't picked up by it.
    let prev = s.doc.prev_para(a.story, &a.path);
    let nearest = s.doc.para_paths(a.story).into_iter().rev().filter(|q| *q < a.path).find_map(|q| {
        let x = s.doc.para(a.story, &q)?;
        let n = x.props.numbering.or_else(|| s.doc.styles.resolve_para(&x.props).numbering).filter(|n| n.num != 0)?;
        Some((q, n, x.props.numbering.is_some()))
    });
    let looks_like_kind = |n: &NumRef| {
        let want = wordcraft_doc::numbering::levels_for(kind);
        s.doc.numbering.level(n.num, 0).zip(want.first()).is_some_and(|(l, w)| l.format == w.format && l.text == w.text)
    };
    let prev_num = nearest
        .filter(|(_, _, direct)| *direct)
        .map(|(q, n, _)| (q, NumRef { level: 0, ..n }))
        .filter(|(q, n)| same_kind(n) && (Some(q) == prev.as_ref() || looks_like_kind(n)))
        .map(|(_, n)| n);
    let num = match prev_num {
        Some(n) => n.num,
        None => match kind {
            ListKind::Bullet | ListKind::BulletChar(_) => match s.doc.numbering.find_kind(kind) {
                Some(n) => n,
                None => s.doc.numbering.try_add_list(kind)?,
            },
            _ => s.doc.numbering.try_add_list(kind)?,
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
    })
}

pub(super) fn restart_numbering(s: &mut Session, _: &Value) -> CmdResult {
    restart_at(s, None)
}

/// Restart the caret's list here, its level at `value` (`None`: the level's own start). This
/// paragraph and every later one of the same list move to the new instance, past paragraphs in
/// between, so the rest of the list counts on from here.
pub(crate) fn restart_at(s: &mut Session, value: Option<u32>) -> CmdResult {
    let f = s.sel.focus.clone();
    let Some(n) = s.doc.para_at(&f).and_then(|p| p.props.numbering).filter(|n| n.num != 0) else {
        return Err(CmdError::Failed("not in a numbered list".into()));
    };
    let new = s.doc.numbering.restart(n.num).ok_or_else(|| CmdError::Failed("bad list".into()))?;
    if let Some(num) = s.doc.numbering.nums.iter_mut().find(|x| x.id == new) {
        // Only this level and the ones below start again: a later item of a level above, moved
        // to the new instance too, counts on (1., 2., a., then 3.).
        num.start_overrides.retain(|(l, _)| *l >= n.level);
        if let Some(value) = value {
            num.start_overrides.retain(|(l, _)| *l != n.level);
            num.start_overrides.push((n.level, value));
        }
    }
    let paths: Vec<_> = s.doc.para_paths(f.story).into_iter().filter(|p| *p >= f.path).collect();
    for path in paths {
        let para = s.doc.para_mut(f.story, &path)?;
        if let Some(x) = para.props.numbering.filter(|x| x.num == n.num) {
            para.props.numbering = Some(NumRef { num: new, level: x.level });
            para.touch();
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
