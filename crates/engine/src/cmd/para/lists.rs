use super::format::fmt;
use super::{CmdError, CmdResult, LineSpacing, ListKind, NumRef, ParaProps, Session, Style, StyleKind, Value, json, p, sel_result};
use std::sync::Arc;

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
    })
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

/// Continue Numbering (#5361): the numbered list at the caret goes on from the nearest numbered
/// list before it in the same story, as Word's Continue Numbering does. Every paragraph of the
/// caret's list after that earlier list takes the earlier list's numbering, so its numbers carry
/// on (3, 4… after 1, 2) instead of starting again.
pub(super) fn continue_numbering(s: &mut Session, _: &Value) -> CmdResult {
    let (current, earlier, after) = continuation(s).ok_or_else(|| CmdError::Failed("no earlier numbered list to continue".into()))?;
    let story = s.sel.focus.story;
    let paths: Vec<_> = s.doc.para_paths(story).into_iter().filter(|p| *p > after).collect();
    for path in paths {
        let para = s.doc.para_mut(story, &path)?;
        if let Some(x) = para.props.numbering.filter(|x| x.num == current) {
            para.props.numbering = Some(NumRef { num: earlier, level: x.level });
            para.touch();
        }
    }
    sel_result(s)
}

/// Why Continue Numbering is unavailable at the caret, or `None` when it can run.
pub(super) fn continue_numbering_enabled(s: &Session) -> Option<&'static str> {
    continuation(s).is_none().then_some("no earlier numbered list to continue")
}

/// What Continue Numbering joins: the list at the caret (its `num`), the nearest earlier numbered
/// list in the same story that is a different list, and the path of that list's last paragraph
/// before the caret. `None` when the caret isn't in a numbered list (bullets don't count) or no
/// such earlier list exists.
fn continuation(s: &Session) -> Option<(u32, u32, wordcraft_doc::Path)> {
    let f = &s.sel.focus;
    let numbered =
        |n: &NumRef| n.num != 0 && s.doc.numbering.level(n.num, n.level).is_some_and(|l| l.format != wordcraft_doc::section::NumFormat::Bullet);
    let here = s.doc.para_at(f).and_then(|p| p.props.numbering).filter(numbered)?;
    s.doc.para_paths(f.story).into_iter().filter(|p| *p < f.path).rev().find_map(|path| {
        let n = s.doc.para(f.story, &path)?.props.numbering.filter(numbered)?;
        (n.num != here.num).then_some((here.num, n.num, path))
    })
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::{Document, StoryRef};
    use wordcraft_layout::Placed;

    use crate::Session;

    /// Each body paragraph's number as laid out (`None`: no number).
    fn labels(s: &mut Session) -> Vec<Option<String>> {
        let l = s.layout();
        l.pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|i| if let Placed::Lines { para, l0: 0, .. } = i { Some(para.label.as_ref().map(|lb| lb.text.clone())) } else { None })
            .collect()
    }

    fn label(t: &str) -> Option<String> {
        Some(t.to_string())
    }

    /// Paragraphs `texts`, with the ones `numbered` picks made a numbered list each run of them
    /// apart (the way a person does: Numbering on the first item of each run).
    fn document(texts: &[&str], numbered: &[bool]) -> Session {
        let mut s = Session::new(Document::new());
        for (i, t) in texts.iter().enumerate() {
            if i > 0 {
                s.run("text.newParagraph", &json!({})).unwrap();
                // A new paragraph takes the list of the one it was split from; set each one's own.
                if !numbered[i] {
                    s.run("para.numbering", &json!({"off": true})).unwrap();
                }
            }
            if numbered[i] && (i == 0 || !numbered[i - 1]) {
                s.run("para.numbering", &json!({"kind": "numbered"})).unwrap();
            }
            s.run("text.insert", &json!({"text": t})).unwrap();
        }
        s
    }

    fn caret_in(s: &mut Session, k: usize) {
        let path = s.doc.para_paths(StoryRef::Body)[k].clone();
        s.sel = crate::Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path, off: 0 });
    }

    /// #5361: a list that starts again at 1 after a paragraph of text carries on from the list
    /// before it, from its first item or any later one, in one undo step, and keeps its numbers
    /// through a .docx save.
    #[test]
    fn continue_numbering_carries_on_from_the_list_before() {
        let texts = ["One", "Two", "Between the lists.", "Three", "Four"];
        let numbered = [true, true, false, true, true];
        let mut s = document(&texts, &numbered);
        let before = vec![label("1."), label("2."), None, label("1."), label("2.")];
        assert_eq!(labels(&mut s), before, "the second list starts again at 1");

        for item in [3, 4] {
            let mut s = document(&texts, &numbered);
            caret_in(&mut s, item);
            assert_eq!(s.why_disabled("para.continueNumbering"), None);
            s.run("para.continueNumbering", &json!({})).unwrap();
            let after = vec![label("1."), label("2."), None, label("3."), label("4.")];
            assert_eq!(labels(&mut s), after, "from item {item}");

            let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
            assert_eq!(labels(&mut Session::new(back)), after, "the numbers survive a .docx save");

            s.run("edit.undo", &json!({})).unwrap();
            assert_eq!(labels(&mut s), before, "one undo step puts the restart back");
        }
    }

    /// Continue Numbering needs a numbered list at the caret and an earlier numbered list in the
    /// same story; otherwise it is disabled and a programmatic call is an error that changes
    /// nothing.
    #[test]
    fn continue_numbering_needs_an_earlier_numbered_list() {
        // The first list has nothing to continue, and a plain paragraph is in no list.
        let mut s = document(&["One", "Two", "Plain text."], &[true, true, false]);
        for k in [0, 1, 2] {
            caret_in(&mut s, k);
            assert!(s.why_disabled("para.continueNumbering").is_some(), "paragraph {k}");
            assert!(s.run("para.continueNumbering", &json!({})).is_err(), "paragraph {k}");
        }
        assert_eq!(labels(&mut s), [label("1."), label("2."), None]);

        // Bullets have no numbers to continue, and don't count as the earlier list either.
        let mut s = document(&["Dot", "Plain text.", "One"], &[true, false, true]);
        caret_in(&mut s, 0);
        s.run("para.bullets", &json!({})).unwrap();
        caret_in(&mut s, 2);
        assert!(s.why_disabled("para.continueNumbering").is_some(), "a bullet before isn't a numbered list");
        caret_in(&mut s, 0);
        assert!(s.why_disabled("para.continueNumbering").is_some(), "a bullet item isn't numbered");
    }
}
