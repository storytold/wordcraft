use super::enter_tab::tab_indent_on;
use super::*;

pub(super) fn backspace(s: &mut Session, v: &Value) -> CmdResult {
    let r = backspace_inner(s, v)?;
    crate::cmd::controls::restore_placeholder(s)?;
    Ok(r)
}

/// Deletes the selection or the character before the caret. At a paragraph start it removes list
/// numbering while keeping the text position, takes a List Paragraph out of the list, outdents,
/// joins the previous paragraph or moves into a preceding table; the command wrapper restores a
/// content-control placeholder.
/// The first Backspace at a numbered paragraph's start removes the number but keeps the item's
/// indent (Word): the text stays where it was, so the indent becomes the paragraph's own.
fn remove_number(s: &mut Session, f: &Pos, n: NumRef) -> CmdResult {
    let text_at = s.doc.numbering.level(n.num, n.level).map(|l| l.indent);
    let p = s.doc.para_mut(f.story, &f.path)?;
    if p.props.style.as_deref() == Some("ListParagraph") {
        if p.props.indent_left.is_none() && p.props.indent_left_chars.is_none_or(|c| c == 0.0) {
            p.props.indent_left = text_at;
        }
        if p.props.indent_first.is_none() && p.props.indent_first_chars.is_none_or(|c| c == 0.0) {
            p.props.indent_first = Some(0.0);
        }
    }
    p.props.numbering = Some(NumRef { num: 0, level: 0 });
    p.touch();
    sel_result(s)
}

fn backspace_inner(s: &mut Session, _: &Value) -> CmdResult {
    s.goal_x = None;
    s.pending = None;
    if !s.sel.is_collapsed() {
        let effect = delete_selection_effect(s)?;
        s.set_ime_edit_outcome(crate::ImeEditOutcome {
            classified: true,
            content_changed: effect.content_changed,
            state_changed: effect.state_changed,
            transformed: false,
            selection_changed: false,
        });
        return sel_result(s);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return sel_result(s) };
    if f.off > 0 {
        let a = Pos { off: para.backspace_from(f.off), ..f.clone() };
        if let Some(c) = s.autocap.as_mut() {
            c.note_backspace(&a, &f);
        }
        s.sel = Selection { anchor: a, focus: f };
        let effect = delete_selection_effect(s)?;
        s.set_ime_edit_outcome(crate::ImeEditOutcome {
            classified: true,
            content_changed: effect.content_changed,
            state_changed: effect.state_changed,
            transformed: false,
            selection_changed: false,
        });
        return sel_result(s);
    }
    // At the start of a paragraph.
    if let Some(n) = para.props.numbering.filter(|n| n.num != 0) {
        return remove_number(s, &f, n);
    }
    if para.props.style.as_deref() == Some("ListParagraph") {
        // The second one takes it out of the list's indent instead of joining paragraphs.
        crate::cmd::leave_list(s.doc.para_mut(f.story, &f.path)?);
        return sel_result(s);
    }
    if para.props.indent_first.is_some_and(|x| x > 0.0) || para.props.indent_first_chars.is_some_and(|x| x > 0.0) {
        let p = s.doc.para_mut(f.story, &f.path)?;
        p.props.indent_first = Some(0.0);
        p.props.cancel_char_indents(false, false, true);
        p.touch();
        return sel_result(s);
    }
    if tab_indent_on(s) && (para.props.indent_left.is_some_and(|x| x > 0.0) || para.props.indent_left_chars.is_some_and(|x| x > 0.0)) {
        // Set left- and first-indent with tabs and backspaces: the paragraph's own left indent
        // (in points or characters) goes back one tab stop at a time before the paragraphs join.
        return crate::cmd::para::outdent(s, &Value::Null);
    }
    let i = f.path.last();
    if i == 0 {
        return sel_result(s); // start of a story or cell
    }
    let prev_path = f.path.with_last(i - 1);
    match s.doc.block(f.story, &prev_path) {
        Some(Block::Para(prev)) => {
            let a = Pos { story: f.story, path: prev_path, off: prev.len() };
            // An empty paragraph before a non-empty one takes the latter's formatting (Word).
            if s.doc.settings.track_changes {
                // Tracked: the paragraph mark above is marked deleted (or removed, when this
                // author inserted it) and the caret moves before it, as in Word.
                s.sel = Selection { anchor: a, focus: f };
                delete_selection(s)?;
                return sel_result(s);
            }
            let at = s.doc.delete_range(&a, &f)?;
            s.sel = Selection::caret(at);
        }
        Some(Block::Table(_)) => {
            // Move into the table's last cell. The table is matched by its whole path: a table
            // nested in a cell shares only its first index with the table around it.
            let last = s.doc.para_paths(f.story).into_iter().rfind(|q| q.0.len() > prev_path.0.len() && q.0.starts_with(&prev_path.0));
            if let Some(q) = last {
                let off = s.doc.para(f.story, &q).map(|x| x.len()).unwrap_or(0);
                if s.doc.para_at(&f).is_some_and(|x| x.is_empty()) && s.doc.container(f.story, &f.path).is_some_and(|c| c.len() > i + 1) {
                    s.doc.remove_block(f.story, &f.path)?;
                }
                s.sel = Selection::caret(Pos { story: f.story, path: q, off });
            }
        }
        None => {}
    }
    sel_result(s)
}

pub(super) fn delete(s: &mut Session, v: &Value) -> CmdResult {
    let r = delete_inner(s, v)?;
    crate::cmd::controls::restore_placeholder(s)?;
    Ok(r)
}

/// Deletes the selection or the character after the caret, joins the next paragraph, handles a
/// following table and removes a trailing section break; tracked changes keep their marks.
fn delete_inner(s: &mut Session, _: &Value) -> CmdResult {
    s.goal_x = None;
    s.pending = None;
    if !s.sel.is_collapsed() {
        let effect = delete_selection_effect(s)?;
        s.set_ime_edit_outcome(crate::ImeEditOutcome {
            classified: true,
            content_changed: effect.content_changed,
            state_changed: effect.state_changed,
            transformed: false,
            selection_changed: false,
        });
        return sel_result(s);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return sel_result(s) };
    if f.off < para.len() {
        let b = Pos { off: para.next_boundary(f.off), ..f.clone() };
        s.sel = Selection { anchor: f, focus: b };
        let effect = delete_selection_effect(s)?;
        s.set_ime_edit_outcome(crate::ImeEditOutcome {
            classified: true,
            content_changed: effect.content_changed,
            state_changed: effect.state_changed,
            transformed: false,
            selection_changed: false,
        });
        return sel_result(s);
    }
    // At the end: join with the next paragraph in the same container.
    let next = f.path.with_last(f.path.last() + 1);
    if let Some(Block::Para(_)) = s.doc.block(f.story, &next) {
        let b = Pos { story: f.story, path: next, off: 0 };
        if s.doc.settings.track_changes {
            // Tracked: the paragraph mark is marked deleted (or removed, when this author
            // inserted it). A mark left in place as a deletion is stepped over, as in Word.
            let blocks = s.doc.container(f.story, &f.path).map(|c| c.len());
            s.sel = Selection { anchor: f.clone(), focus: b.clone() };
            delete_selection(s)?;
            if s.doc.container(f.story, &f.path).map(|c| c.len()) == blocks && s.doc.para_at(&f).is_some_and(|p| p.mark.del.is_some()) {
                s.sel = Selection::caret(b);
            }
            return sel_result(s);
        }
        let at = s.doc.delete_range(&f, &b)?;
        s.sel = Selection::caret(at);
    } else if let Some(Block::Table(_)) = s.doc.block(f.story, &next) {
        // An empty paragraph before a table is removed; else nothing.
        if para.is_empty() && f.path.last() > 0 {
            s.doc.remove_block(f.story, &f.path)?;
            let mut path = next.0.clone();
            if let Some(l) = path.last_mut() {
                *l -= 1;
            }
            path.extend([0, 0, 0]);
            s.sel = Selection::caret(Pos { story: f.story, path: wordcraft_doc::Path(path), off: 0 });
        }
    } else if f.story == StoryRef::Body && f.path.0.len() == 1 && !s.doc.settings.track_changes {
        // The last paragraph: no paragraph to join, but a section break on its mark (the last
        // section then holds no paragraph) still goes, as in Word (#2127).
        s.doc.remove_section_break(f.path.last());
    }
    sel_result(s)
}

pub(super) fn delete_word_back(s: &mut Session, v: &Value) -> CmdResult {
    if !s.sel.is_collapsed() {
        return backspace(s, v);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return sel_result(s) };
    if f.off == 0 {
        return backspace(s, v);
    }
    let a = Pos { off: para.word_start(f.off), ..f.clone() };
    s.sel = Selection { anchor: a, focus: f };
    delete_selection(s)?;
    crate::cmd::controls::restore_placeholder(s)?;
    sel_result(s)
}

pub(super) fn delete_word_fwd(s: &mut Session, v: &Value) -> CmdResult {
    if !s.sel.is_collapsed() {
        return delete(s, v);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return sel_result(s) };
    if f.off >= para.len() {
        return delete(s, v);
    }
    let b = Pos { off: para.word_end(f.off), ..f.clone() };
    s.sel = Selection { anchor: f, focus: b };
    delete_selection(s)?;
    crate::cmd::controls::restore_placeholder(s)?;
    sel_result(s)
}
