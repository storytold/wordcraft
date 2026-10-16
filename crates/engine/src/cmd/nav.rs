//! The Navigation pane's outline, editable in place. The pane itself shows the headings of a
//! document and jumps to them; these commands are what turns that list into a tree that can be
//! worked on — a heading is promoted or demoted, one is added above, below or under it, one is
//! taken out, and one can be dragged somewhere else. A heading is always a whole section: the
//! paragraphs under it (down to the next heading of the same or a shallower level, or the end of
//! the container) move, go and change level with it, which is how the outline of an online editor
//! (Tencent Docs, WPS) and Word's own Navigation pane behave.
//!
//! The pane is their own surface, so its name is their location: the Outlining tab's commands
//! ([`crate::cmd::outline`]) reach the same levels, but on the selection, in Outline view.

use serde_json::{Value, json};
use wordcraft_doc::{Block, Path, StoryRef};

use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Where the heading is: an explicit `path`, or a top-level `block`, or (both absent) the
/// paragraph the caret is in — so the commands work from the pane, from a menu and from an agent
/// alike.
const TARGET: &str = r#"{"path"?: [block indices], "block"?: body block index (default: the caret's paragraph)}"#;

/// Where a new heading goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Where {
    /// Before the heading: a new one at the same level (its section stays below the old heading).
    Above,
    /// After the heading's whole section: a new one at the same level.
    Below,
    /// Right under the heading: a new one a level deeper, the first subheading of its section.
    Child,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("nav.promote", "Promote", "Navigation Pane", |s, v| step(s, v, true)).params(TARGET),
        CommandSpec::new("nav.demote", "Demote", "Navigation Pane", |s, v| step(s, v, false)).params(TARGET),
        CommandSpec::new("nav.remove", "Delete", "Navigation Pane", remove).params(TARGET),
        CommandSpec::new("nav.hideFromToc", "Do Not Show in TOC", "Navigation Pane", hide_from_toc).params(TARGET),
        CommandSpec::new("nav.addAbove", "Add Heading Above", "Navigation Pane", |s, v| insert(s, v, Where::Above))
            .params(TARGET),
        CommandSpec::new("nav.addBelow", "Add Heading Below", "Navigation Pane", |s, v| insert(s, v, Where::Below))
            .params(TARGET),
        CommandSpec::new("nav.addSubheading", "Add Subheading", "Navigation Pane", |s, v| insert(s, v, Where::Child))
            .params(TARGET),
        CommandSpec::new("nav.move", "Move Section", "Navigation Pane", move_to).params(
            r#"{"path": [block indices] (the heading to move), "before"?: [block indices] (the paragraph it goes in front of; absent = the end of its container)}"#,
        ),
    ]
}

/// The heading level (1–9) of a paragraph, as the pane and the TOC read it: the outline level
/// resolved through its style (or set directly on it). `None` for body text, and for the entries
/// of a table of contents themselves, which are styled `TOC*`.
pub fn level(s: &Session, story: StoryRef, path: &Path) -> Option<u8> {
    let para = s.doc.para(story, path)?;
    let rp = s.doc.styles.resolve_para(&para.props);
    if rp.style.starts_with("TOC") {
        return None;
    }
    rp.outline_level.filter(|l| *l < 9).map(|l| l + 1)
}

/// The path a command works on, from its parameters.
fn target(s: &Session, v: &Value) -> Result<Path, CmdError> {
    if let Some(arr) = v.get("path").and_then(Value::as_array) {
        let mut p = Path::default();
        for x in arr {
            let i = x.as_u64().ok_or_else(|| CmdError::Params("`path` holds block indices".into()))?;
            p.0.push(u32::try_from(i).map_err(|_| CmdError::Params("`path` index out of range".into()))?);
        }
        if p.0.is_empty() {
            return Err(CmdError::Params("`path` is empty".into()));
        }
        return Ok(p);
    }
    if let Some(b) = p::u64(v, "block") {
        let i = usize::try_from(b).map_err(|_| CmdError::Params("`block` out of range".into()))?;
        if s.doc.para(StoryRef::Body, &Path::top(i)).is_none() {
            return Err(CmdError::Params(format!("no paragraph at block {i}")));
        }
        return Ok(Path::top(i));
    }
    Ok(s.sel.focus.path.clone())
}

/// The heading `path` names, or an error when there is none there.
fn heading(s: &Session, path: &Path) -> Result<(Path, u8), CmdError> {
    if s.doc.para(StoryRef::Body, path).is_none() {
        return Err(CmdError::Params(format!("no paragraph at {path:?}")));
    }
    let l = level(s, StoryRef::Body, path).ok_or_else(|| CmdError::Params("that paragraph is not a heading".into()))?;
    Ok((path.clone(), l))
}

/// Promote (`up`) or demote one heading a level; Heading 1 and Heading 9 are the ends. Only the
/// heading itself changes: its section keeps the shape it has.
fn step(s: &mut Session, v: &Value, up: bool) -> CmdResult {
    let at = target(s, v)?;
    let (path, l) = heading(s, &at)?;
    let level = if up { l.saturating_sub(1).max(1) } else { (l + 1).min(9) };
    if level == l {
        return Ok(json!({"path": path.0, "level": l}));
    }
    super::para::set_heading(s, StoryRef::Body, &path, Some(level))?;
    s.sel = Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: path.clone(), off: 0 });
    Ok(json!({"path": path.0, "level": level}))
}

/// Take the heading out of the outline without touching how it looks: the paragraph keeps its
/// style (and so its formatting) but is marked as body text — Word's "Do Not Show in TOC", which
/// is what its Outline Level box does for a heading that should stay on the page as it is.
fn hide_from_toc(s: &mut Session, v: &Value) -> CmdResult {
    let at = target(s, v)?;
    let (path, l) = heading(s, &at)?;
    let para = s.doc.para_mut(StoryRef::Body, &path)?;
    para.props.outline_level = Some(9);
    para.touch();
    Ok(json!({"path": path.0, "was": l}))
}

/// Delete a heading and its whole section.
fn remove(s: &mut Session, v: &Value) -> CmdResult {
    let at = target(s, v)?;
    let (path, _) = heading(s, &at)?;
    let end = section_end(s, &path)?;
    let container = path.parent().to_vec();
    let first = path.last();
    // Removed from the end so the indices of what is still to go stay right. `remove_block` never
    // leaves its container empty.
    for i in (first..end).rev() {
        let mut p = Path(container.clone());
        p.0.push(i as u32);
        s.doc.remove_block(StoryRef::Body, &p)?;
    }
    s.doc.ensure_nonempty();
    // The caret goes where the section was: the block that moved up into its place, or the last.
    let left = s.doc.container(StoryRef::Body, &path).map(Vec::len).unwrap_or(0);
    let at = if first < left { first } else { left.saturating_sub(1) };
    let mut back = Path(container);
    back.0.push(at as u32);
    let back = if s.doc.para(StoryRef::Body, &back).is_some() { back } else { s.doc.start_of(StoryRef::Body).path };
    s.sel = Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: back, off: 0 });
    Ok(json!({"removed": end - first}))
}

/// Add an empty heading at `where` (see [`Where`]), with the same style as the one it is built
/// from, and put the caret in it so the title can be typed straight away.
fn insert(s: &mut Session, v: &Value, where_: Where) -> CmdResult {
    let at = target(s, v)?;
    let (path, l) = heading(s, &at)?;
    let level = if where_ == Where::Child { (l + 1).min(9) } else { l };
    let index = match where_ {
        Where::Above => path.last(),
        Where::Child => path.last() + 1,
        Where::Below => section_end(s, &path)?,
    };
    // Empty, but formatted like a heading of that level: the paragraph's own properties are what
    // carries the style (and any direct formatting the author put on it).
    let like = s.doc.para(StoryRef::Body, &path).ok_or_else(|| CmdError::Params("the heading went away".into()))?.props.clone();
    let mut blank = wordcraft_doc::Paragraph::new();
    blank.props = like;
    let mut at_new = Path(path.parent().to_vec());
    at_new.0.push(index as u32);
    s.doc.insert_block(StoryRef::Body, &at_new, Block::Para(blank))?;
    super::para::set_heading(s, StoryRef::Body, &at_new, Some(level))?;
    s.sel = Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: at_new.clone(), off: 0 });
    Ok(json!({"path": at_new.0, "level": level}))
}

/// Move a heading's whole section in front of `before` (the end of its container when absent).
/// The drop targets the pane offers are headings, so the section lands between two sections and
/// keeps its own subheadings in order.
fn move_to(s: &mut Session, v: &Value) -> CmdResult {
    let from = target(s, v)?;
    let (path, _) = heading(s, &from)?;
    let end = section_end(s, &path)?;
    let first = path.last();
    let container = path.parent().to_vec();
    // `before` is a paragraph path in the same container; nothing (the end of it) means "last".
    let before = match v.get("before") {
        Some(x) if !x.is_null() => {
            let mut p = Path::default();
            let arr = x.as_array().ok_or_else(|| CmdError::Params("`before` holds block indices".into()))?;
            for i in arr {
                let n = i.as_u64().ok_or_else(|| CmdError::Params("`before` holds block indices".into()))?;
                p.0.push(u32::try_from(n).map_err(|_| CmdError::Params("`before` index out of range".into()))?);
            }
            if p.parent() != container.as_slice() {
                return Err(CmdError::Params("`before` is not in the same container as the heading".into()));
            }
            Some(p.last())
        }
        _ => None,
    };
    let len = s.doc.container(StoryRef::Body, &path).map(Vec::len).unwrap_or(end);
    let at = before.map_or(len, |b| b.min(len));
    // Dropping a section inside itself (or right where it already is) changes nothing.
    if at >= first && at <= end {
        return Ok(json!({"moved": 0}));
    }
    // The blocks leave first, so the destination shifts back by what was in front of it.
    let taken: Vec<_> = {
        let blocks = s.doc.container_mut(StoryRef::Body, &path)?;
        blocks.drain(first..end).collect()
    };
    let to = if at > end { at - (end - first) } else { at };
    let mut landing = Path(container);
    landing.0.push(to as u32);
    let blocks = s.doc.container_mut(StoryRef::Body, &landing)?;
    for (n, b) in taken.into_iter().enumerate() {
        blocks.insert((to + n).min(blocks.len()), b);
    }
    let mut now = Path(path.parent().to_vec());
    now.0.push(to as u32);
    s.sel = Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: now, off: 0 });
    Ok(json!({"moved": end - first}))
}

/// The block index one past the section a heading opens: its container's length, or the index of
/// the next heading at the same or a shallower level (the first one that is no longer its own).
fn section_end(s: &Session, path: &Path) -> Result<usize, CmdError> {
    let l = level(s, StoryRef::Body, path).ok_or_else(|| CmdError::Params("that paragraph is not a heading".into()))?;
    let container = s.doc.container(StoryRef::Body, path).ok_or_else(|| CmdError::Params(format!("no block at {path:?}")))?;
    let first = path.last();
    for i in first + 1..container.len() {
        let mut p = Path(path.parent().to_vec());
        p.0.push(i as u32);
        if level(s, StoryRef::Body, &p).is_some_and(|n| n <= l) {
            return Ok(i);
        }
    }
    Ok(container.len())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::{Document, Path, StoryRef};

    use crate::Session;
    use crate::cmd::nav::level;

    /// A body whose headings are styled, one paragraph at a time.
    fn doc(lines: &[(&str, Option<u8>)]) -> Session {
        let mut s = Session::new(Document::new());
        let text = lines.iter().map(|(t, _)| *t).collect::<Vec<_>>().join("\n");
        s.run("document.setText", &json!({"text": text})).unwrap();
        for (i, (_, l)) in lines.iter().enumerate() {
            if let Some(n) = l {
                s.run("select.range", &json!({"anchor": {"block": i, "off": 0}, "focus": {"block": i, "off": 0}})).unwrap();
                s.run("outline.level", &json!({"level": n})).unwrap();
            }
        }
        s
    }

    fn texts(s: &Session) -> Vec<String> {
        s.doc.para_paths(StoryRef::Body).into_iter().filter_map(|p| s.doc.para(StoryRef::Body, &p).map(|p| p.text.clone())).collect()
    }

    fn levels(s: &Session) -> Vec<Option<u8>> {
        (0..s.doc.para_paths(StoryRef::Body).len()).map(|i| level(s, StoryRef::Body, &Path::top(i))).collect()
    }

    /// Two top-level sections, the first with a subheading and text of its own.
    fn two_sections() -> Session {
        doc(&[("One", Some(1)), ("one text", None), ("One A", Some(2)), ("one a text", None), ("Two", Some(1)), ("two text", None)])
    }

    #[test]
    fn promote_and_demote_a_heading() {
        let mut s = two_sections();
        s.run("nav.promote", &json!({"path": [2]})).unwrap();
        assert_eq!(levels(&s), [Some(1), None, Some(1), None, Some(1), None]);
        s.run("nav.demote", &json!({"path": [2]})).unwrap();
        assert_eq!(levels(&s), [Some(1), None, Some(2), None, Some(1), None]);
        // The ends hold: Heading 1 stays 1, Heading 9 stays 9.
        s.run("nav.promote", &json!({"path": [0]})).unwrap();
        assert_eq!(levels(&s)[0], Some(1));
    }

    #[test]
    fn a_heading_can_be_taken_out_of_the_outline_and_keeps_its_style() {
        let mut s = two_sections();
        let style = s.doc.para(StoryRef::Body, &Path::top(2)).unwrap().props.style.clone();
        s.run("nav.hideFromToc", &json!({"path": [2]})).unwrap();
        assert_eq!(levels(&s)[2], None, "it is no longer a heading");
        assert_eq!(s.doc.para(StoryRef::Body, &Path::top(2)).unwrap().props.style, style, "but keeps its look");
    }

    #[test]
    fn deleting_a_heading_takes_its_section_and_leaves_the_rest() {
        let mut s = two_sections();
        s.run("nav.remove", &json!({"path": [0]})).unwrap();
        assert_eq!(texts(&s), ["Two", "two text"]);
        // The last section can go too, and the document keeps a paragraph.
        s.run("nav.remove", &json!({"path": [0]})).unwrap();
        assert_eq!(texts(&s), [""]);
    }

    #[test]
    fn a_new_heading_goes_above_below_or_under() {
        let mut s = two_sections();
        let made = s.run("nav.addAbove", &json!({"path": [4]})).unwrap();
        assert_eq!(made["path"], json!([4]));
        assert_eq!(texts(&s), ["One", "one text", "One A", "one a text", "", "Two", "two text"]);
        assert_eq!(levels(&s)[4], Some(1), "same level as the heading it was made from");

        let mut s = two_sections();
        let made = s.run("nav.addSubheading", &json!({"path": [0]})).unwrap();
        assert_eq!(made["path"], json!([1]));
        assert_eq!(made["level"], json!(2));
        assert_eq!(texts(&s)[1], "", "a subheading right under its heading");

        let mut s = two_sections();
        let made = s.run("nav.addBelow", &json!({"path": [2]})).unwrap();
        assert_eq!(made["path"], json!([4]), "after the section, before the next heading");
        assert_eq!(levels(&s)[4], Some(2));
    }

    #[test]
    fn a_section_is_dragged_with_everything_under_it() {
        let mut s = two_sections();
        // "Two" moves to the top, in front of the first heading.
        s.run("nav.move", &json!({"path": [4], "before": [0]})).unwrap();
        assert_eq!(texts(&s), ["Two", "two text", "One", "one text", "One A", "one a text"]);
        assert_eq!(levels(&s), [Some(1), None, Some(1), None, Some(2), None]);
        // With no target at all it goes to the end of the body, and the whole section travels.
        s.run("nav.move", &json!({"path": [0]})).unwrap();
        assert_eq!(texts(&s), ["One", "one text", "One A", "one a text", "Two", "two text"]);
        assert_eq!(levels(&s), [Some(1), None, Some(2), None, Some(1), None]);
        // Dropping a section inside itself changes nothing.
        let same = s.run("nav.move", &json!({"path": [0], "before": [2]})).unwrap();
        assert_eq!(same["moved"], json!(0));
        assert_eq!(texts(&s)[0], "One");
    }

    #[test]
    fn the_commands_read_the_caret_when_given_no_path() {
        let mut s = two_sections();
        s.run("caret.set", &json!({"pos": {"block": 4, "off": 2}})).unwrap();
        s.run("nav.demote", &json!({})).unwrap();
        assert_eq!(levels(&s)[4], Some(2));
        // A paragraph that is not a heading is refused, caret or path alike.
        let err = s.run("nav.demote", &json!({"path": [1]})).unwrap_err();
        assert!(format!("{err}").contains("not a heading"), "{err}");
    }
}
