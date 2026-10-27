use super::lists::touch_all;
use super::*;

/// Move Paragraph Up / Down (Alt+Shift+Up/Down, ⌃⇧↑/↓ on macOS): the paragraphs the selection
/// touches swap places with the paragraph above or below, in their own container (the story or
/// a table cell), keeping their formatting and list membership; the selection moves with them.
/// A selection that ends at the very start of a paragraph doesn't touch that paragraph (a
/// selected paragraph includes its mark). Refused at the top or bottom of the container, next to
/// a table, across a section break and for a selection across table cells.
pub(super) fn move_paras(s: &mut Session, up: bool) -> CmdResult {
    // Outline view: what's collapsed moves with its heading, past whole shown paragraphs.
    if let Some(r) = crate::cmd::outline::move_shown(s, up) {
        return r;
    }
    let (a, b) = s.sel.ordered();
    if a.story != b.story || a.path.parent() != b.path.parent() {
        return Err(CmdError::Failed("select paragraphs in one place (not across table cells) to move them".into()));
    }
    let i0 = a.path.last();
    let mut i1 = b.path.last();
    let past_end = i1 > i0 && b.off == 0;
    if past_end {
        i1 -= 1;
    }
    let bl = s.doc.container_mut(a.story, &a.path)?;
    let (lo, hi) = if up {
        let Some(n) = i0.checked_sub(1) else { return Err(CmdError::Failed("already at the top".into())) };
        (n, i1)
    } else {
        let n = i1.saturating_add(1);
        if n >= bl.len() {
            return Err(CmdError::Failed("already at the bottom".into()));
        }
        (i0, n)
    };
    let neighbour = if up { lo } else { hi };
    if !matches!(bl.get(neighbour).map(|b| &**b), Some(Block::Para(_))) {
        return Err(CmdError::Failed("a table is in the way".into()));
    }
    let Some(slice) = bl.get_mut(lo..=hi) else { return Err(CmdError::Failed("bad range".into())) };
    if slice.iter().any(|b| b.as_para().is_some_and(|p| p.section.is_some())) {
        return Err(CmdError::Failed("paragraphs don't move across a section break".into()));
    }
    if up {
        slice.rotate_left(1);
    } else {
        slice.rotate_right(1);
    }
    // The selection follows the moved paragraphs.
    let len = bl.len();
    let shift = |p: &Pos| -> Pos {
        let i = p.path.last();
        let j = if (i0..=i1).contains(&i) {
            if up { i - 1 } else { i + 1 }
        } else if !up && i == i1 + 1 {
            // The end just past the range: the start of whatever follows it now.
            return if i1 + 2 < len {
                Pos { path: p.path.with_last(i1 + 2), off: 0, ..p.clone() }
            } else {
                Pos { path: p.path.with_last(i1 + 1), off: usize::MAX, ..p.clone() }
            };
        } else {
            i
        };
        Pos { path: p.path.with_last(j), ..p.clone() }
    };
    let (anchor, focus) = (shift(&s.sel.anchor), shift(&s.sel.focus));
    s.sel = crate::Selection { anchor: s.doc.clamp(&anchor), focus: s.doc.clamp(&focus) };
    s.goal_x = None;
    sel_result(s)
}

/// The heading level (1–9) a paragraph's style gives it, if it is a heading.
fn heading_level(s: &Session, props: &ParaProps) -> Option<u8> {
    s.doc.styles.resolve_para(props).outline_level.filter(|l| *l < 9).map(|l| l + 1)
}

/// Give paragraph `path` Heading `level` (1–9), or make it body text (`None`: the default
/// paragraph style); its own outline level goes, so the style's counts.
pub(crate) fn set_heading(s: &mut Session, story: wordcraft_doc::StoryRef, path: &wordcraft_doc::Path, level: Option<u8>) -> Result<(), CmdError> {
    let style = level.map(|n| ensure_heading_style(s, n.clamp(1, 9)));
    let para = s.doc.para_mut(story, path)?;
    para.props.style = style;
    para.props.outline_level = None;
    para.touch();
    Ok(())
}

/// Make sure the built-in `Heading{n}` style (and its linked character style) is in the
/// document, for files that didn't bring it.
pub(crate) fn ensure_heading_style(s: &mut Session, n: u8) -> String {
    let id = format!("Heading{n}");
    if s.doc.styles.get(&id).is_none() {
        let builtin = wordcraft_doc::styles::StyleSheet::builtin();
        for sid in [id.clone(), format!("{id}Char")] {
            if s.doc.styles.get(&sid).is_none()
                && let Some(st) = builtin.get(&sid).cloned()
            {
                s.doc.styles.upsert(st);
            }
        }
        touch_all(s);
    }
    id
}

/// Promote / Demote (Alt+Shift+Left/Right, ⌃⇧←/→ on macOS), Word's outline keys outside Outline
/// view: a heading goes up or down one level (Heading 1 and Heading 9 are the ends); body text
/// promoted becomes a heading at the level of the heading before it (Heading 1 when there is
/// none) and can't be demoted further; a list item changes list level like Shift+Tab / Tab.
pub(super) fn outline_step(s: &mut Session, promote: bool) -> CmdResult {
    // Outline view: the headings hidden under a collapsed heading move a level with it.
    let carried = crate::cmd::outline::hidden_subheadings(s);
    let (a, b) = s.sel.ordered();
    let paths = s.doc.paths_between(&a, &b);
    // The heading before the selection, for promoted body text.
    let before = s
        .doc
        .para_paths(a.story)
        .into_iter()
        .take_while(|p| *p < a.path)
        .filter_map(|p| s.doc.para(a.story, &p).and_then(|q| heading_level(s, &q.props)))
        .last()
        .unwrap_or(1);
    let mut changed = false;
    for path in paths {
        let Some(p) = s.doc.para(a.story, &path) else { continue };
        let list = p.props.numbering.filter(|n| n.num != 0);
        let target = match (list, heading_level(s, &p.props)) {
            (Some(n), _) => {
                let level = if promote { n.level.saturating_sub(1) } else { n.level.saturating_add(1).min(8) };
                if level != n.level {
                    let para = s.doc.para_mut(a.story, &path)?;
                    para.props.numbering = Some(NumRef { num: n.num, level });
                    para.touch();
                    changed = true;
                }
                continue;
            }
            (None, Some(h)) if promote => h.checked_sub(1).filter(|l| *l >= 1),
            (None, Some(h)) => Some(h + 1).filter(|l| *l <= 9),
            (None, None) if promote => Some(before),
            (None, None) => None,
        };
        let Some(n) = target else { continue };
        let id = ensure_heading_style(s, n);
        let para = s.doc.para_mut(a.story, &path)?;
        para.props.style = Some(id);
        para.props.outline_level = None;
        para.touch();
        changed = true;
    }
    if !changed {
        return Err(CmdError::Failed(if promote { "nothing to promote".into() } else { "nothing to demote".into() }));
    }
    for (j, h) in carried {
        let n = if promote { h.saturating_sub(1).max(1) } else { (h + 1).min(9) };
        if n != h {
            set_heading(s, wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(j), Some(n))?;
        }
    }
    sel_result(s)
}

/// Sort the selected paragraphs (same container) alphabetically.
pub(super) fn sort(s: &mut Session, v: &Value) -> CmdResult {
    let desc = p::bool(v, "descending").unwrap_or(false);
    let (a, b) = s.sel.ordered();
    if a.path.parent() != b.path.parent() {
        return Err(CmdError::Failed("select paragraphs in one place to sort".into()));
    }
    let (i0, i1) = (a.path.last(), b.path.last());
    let lang = s.doc.para(a.story, &a.path).and_then(|p| crate::cmd::para_lang(&s.doc, p)).or_else(|| crate::cmd::default_lang(s));
    let bl = s.doc.container_mut(a.story, &a.path)?;
    let Some(slice) = bl.get_mut(i0..=i1) else { return Err(CmdError::Failed("bad range".into())) };
    if slice.iter().any(|b| !matches!(**b, Block::Para(_))) {
        return Err(CmdError::Failed("sorting tables isn't supported here; use Table › Sort".into()));
    }
    let mut keyed: Vec<_> =
        slice.iter().map(|b| (natural_key(&b.as_para().map(|p| p.plain_text()).unwrap_or_default(), lang.as_deref()), b.clone())).collect();
    keyed.sort_by(|(x, _), (y, _)| {
        let o = natural_cmp(x, y);
        if desc { o.reverse() } else { o }
    });
    for (dst, (_, blk)) in slice.iter_mut().zip(keyed) {
        *dst = blk;
    }
    // Keep a section break on the last paragraph of the range where it was.
    s.sel = crate::Selection { anchor: Pos { off: 0, ..a }, focus: b };
    sel_result(s)
}

/// A paragraph's sort key: the number it starts with, then its text in its language's order.
fn natural_key(text: &str, lang: Option<&str>) -> (Option<f64>, SortKey) {
    let num = text.trim().split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<f64>().ok());
    (num, sort_key(text, lang))
}

/// Paragraphs starting with a number come first, in numeric order ("item 2" < "item 10" within
/// the same number, by text); the rest follow in collation order. A total order.
fn natural_cmp(a: &(Option<f64>, SortKey), b: &(Option<f64>, SortKey)) -> std::cmp::Ordering {
    let num = match (a.0, b.0) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    };
    num.then_with(|| a.1.cmp(&b.1))
}
