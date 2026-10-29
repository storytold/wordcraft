use super::autoformat::{
    EnterAutoFormat, apply_border_line, apply_heading_line, apply_table_line, border_line, heading_line, table_line, typing_options,
};
use super::unicode::ins_char;
use super::{CmdError, CmdResult, NumRef, Pos, Selection, Session, StoryRef, Value, delete_selection, sel_result, split_para, type_text};

/// Exact sample-text formulas; bounded counts keep typed input from allocating without limit.
fn sample_text(s: &Session) -> Option<String> {
    let at = &s.sel.focus;
    let para = s.doc.para_at(at)?;
    if !s.sel.is_collapsed() || at.off != para.len() {
        return None;
    }
    let (args, sentences) = if let Some(args) = para.text.strip_prefix("=rand(") {
        (
            args,
            [
                "A blank page leaves room for a new idea.",
                "The morning light falls across a notebook on the desk.",
                "Small changes in spacing can give a story a different rhythm.",
            ],
        )
    } else {
        (
            para.text.strip_prefix("=lorem(")?,
            [
                "Lorem ipsum dolor sit amet, consectetur adipiscing elit.",
                "Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.",
                "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.",
            ],
        )
    };
    let args = args.strip_suffix(')')?;
    if args.contains(['(', ')']) {
        return None;
    }
    let count = |arg: &str| arg.trim().parse::<usize>().ok().filter(|n| (1..=200).contains(n));
    let counts = if args.is_empty() {
        Some((5, 3))
    } else if let Some((p, s)) = args.split_once(',') {
        count(p).zip(count(s))
    } else {
        count(args).map(|p| (p, 3))
    };
    let (paragraphs, lines) = counts.unwrap_or((5, 3));
    let mut text = String::new();
    for _ in 0..paragraphs {
        for sentence in sentences.iter().cycle().take(lines) {
            if !text.is_empty() && !text.ends_with('\r') {
                text.push(' ');
            }
            text.push_str(sentence);
        }
        text.push('\r');
    }
    Some(text)
}

/// Whether the caret is at the end of a body heading that hides what follows it (#1745).
pub(super) fn fold_at_end(s: &Session) -> bool {
    let f = &s.sel.focus;
    f.story == StoryRef::Body
        && f.path.0.len() == 1
        && s.doc.para_at(f).is_some_and(|p| f.off >= p.len())
        && crate::cmd::outline::folds(s, f.path.last())
        && crate::cmd::outline::folded(s).get(f.path.last().saturating_add(1)).copied().unwrap_or(false)
}

/// `text.newParagraph`: Enter, unless the selection is in a control that takes no paragraph break.
pub(super) fn new_paragraph(s: &mut Session, _: &Value) -> CmdResult {
    // Enter in a one-line plain text control (or a check box or list) does nothing, as in Word.
    if crate::cmd::controls::refuses_paragraph(s) {
        return sel_result(s);
    }
    enter(s, true)?;
    sel_result(s)
}

/// Most lines `text.insert` with `paragraphs` types in one call, for every caller (control
/// channel, CLI, MCP): each line is an Enter, with its AutoFormat and AutoCorrect, so the work
/// grows with the line count. More is refused before anything is typed (#3692).
pub const MAX_TYPED_PARAGRAPHS: usize = 10_000;

/// `text.insert` with `paragraphs`: each line typed as it is (`raw`), and between lines the
/// paragraph break Enter makes ([`enter`]: its AutoFormat and AutoCorrect), all in one command, so
/// one undo step instead of a command and a whole-document snapshot per line (#3692). Where
/// Enter would do nothing (a one-line control, a check box, a list) the text is refused and,
/// like any failed command, nothing of it stays. More than [`MAX_TYPED_PARAGRAPHS`] lines are
/// refused before anything is typed.
pub(super) fn insert_paragraphs(s: &mut Session, text: &str) -> CmdResult {
    let text = text.replace("\r\n", "\n");
    let lines = text.split(['\n', '\r', '\u{2029}']).count();
    if lines > MAX_TYPED_PARAGRAPHS {
        return Err(CmdError::Params(format!("text.insert: `text` has {lines} lines; type at most {MAX_TYPED_PARAGRAPHS} paragraphs per call")));
    }
    typing_options(s, &text);
    for (i, line) in text.split(['\n', '\r', '\u{2029}']).enumerate() {
        if i > 0 {
            if crate::cmd::controls::refuses_paragraph(s) {
                return Err(CmdError::Disabled("text.insert: this content control takes no paragraph breaks".into()));
            }
            enter(s, false)?;
        }
        if !line.is_empty() {
            type_text(s, line)?;
        }
    }
    sel_result(s)
}

/// Enter's paragraph break once a control has allowed it: `=rand()` sample text, AutoFormat of a
/// border or table line, AutoCorrect of the word it ends, else a plain split. With `undo_step`
/// an AutoFormat is recorded as its own undo step after the plain Enter, as in Word; without it
/// (typed paragraphs, one undo step for all) no extra snapshot is taken.
fn enter(s: &mut Session, undo_step: bool) -> Result<(), CmdError> {
    crate::cmd::controls::prepare_edit(s)?;
    if let Some(text) = sample_text(s) {
        let end = s.sel.focus.clone();
        s.sel = Selection { anchor: Pos { off: 0, ..end.clone() }, focus: end };
        return type_text(s, &text);
    }
    // Enter at the end of a collapsed heading only expands it (Word for Mac, #1745).
    if s.sel.is_collapsed() && fold_at_end(s) {
        crate::cmd::outline::expand_heading(s, s.sel.focus.path.last());
        return Ok(());
    }
    let mut at = delete_selection(s)?;
    let auto = border_line(s, &at)
        .map(EnterAutoFormat::Border)
        .or_else(|| table_line(s, &at).map(EnterAutoFormat::Table))
        .or_else(|| heading_line(s, &at).map(EnterAutoFormat::Heading));
    if let Some(auto) = auto {
        // Word: Undo right after reverts only the AutoFormat, so the typed characters come back
        // and the Enter stays. Record that plain Enter as its own undo step.
        let plain = if undo_step {
            let (doc, sel) = (s.doc.clone(), s.sel.clone());
            let new = split_para(s, &at)?;
            s.sel = Selection::caret(new);
            Some((std::mem::replace(&mut s.doc, doc), std::mem::replace(&mut s.sel, sel)))
        } else {
            None
        };
        match auto {
            EnterAutoFormat::Border(line) => apply_border_line(s, &at, line)?,
            EnterAutoFormat::Table(cols) => apply_table_line(s, &at, &cols)?,
            EnterAutoFormat::Heading(level) => apply_heading_line(s, &at, level)?,
        }
        s.goal_x = None;
        if let Some((plain_doc, plain_sel)) = plain {
            s.push_undo("AutoFormat", plain_doc, plain_sel);
        }
        return Ok(());
    }
    // Enter finishes a word like a space does ("teh" → "the", sentence capitals, links).
    if s.sel.is_collapsed() && at.off > 0 {
        // A `*bold*` or `_italic_` word: like the border lines above, Undo right after brings
        // the marks back and the Enter stays.
        let plain = crate::cmd::tools::has_emphasis(s, true).then(|| (s.doc.clone(), s.sel.clone()));
        if let Some((doc, sel)) = plain {
            // Removes the marks, then AutoCorrects the word as without them.
            crate::cmd::tools::autocorrect_word(s, true)?;
            s.autocap.take_if(|c| c.depth == u64::MAX);
            let (done_doc, done_sel) = (std::mem::replace(&mut s.doc, doc), std::mem::replace(&mut s.sel, sel));
            let new = split_para(s, &at)?;
            let c = s.doc.para_at(&new).map(|p| p.mark.clone()).unwrap_or_default();
            s.sel = Selection::caret(new);
            let plain_doc = std::mem::replace(&mut s.doc, done_doc);
            let plain_sel = std::mem::replace(&mut s.sel, done_sel);
            let new = split_para(s, &s.sel.focus.clone())?;
            let p = s.doc.para_mut(new.story, &new.path)?;
            // The new paragraph is formatted as after a plain Enter (the closing mark's
            // formatting, or none when the next style starts afresh); its mark keeps its revisions.
            let (ins, del, fmt_change) = (p.mark.ins, p.mark.del, p.mark.fmt_change.take());
            p.mark = wordcraft_doc::props::CharProps { ins, del, fmt_change, ..c };
            p.touch();
            s.sel = Selection::caret(new);
            s.goal_x = None;
            s.push_undo("AutoFormat", plain_doc, plain_sel);
            return Ok(());
        }
        crate::cmd::tools::autocorrect_word(s, true)?;
        s.autocap.take_if(|c| c.depth == u64::MAX);
        at = s.sel.focus.clone();
    }
    let new = split_para(s, &at)?;
    s.sel = Selection::caret(new);
    s.goal_x = None;
    Ok(())
}

/// In a table: next cell (adds a row at the end). At the start of a list item: demote. With "Set
/// left- and first-indent with tabs and backspaces" on, at the start of a line of a paragraph with
/// text: indent. Else a tab.
pub(super) fn tab(s: &mut Session, _: &Value) -> CmdResult {
    if let Some((tpath, r, c)) = s.sel.focus.path.cell() {
        let story = s.sel.focus.story;
        let Some(t) = s.doc.table(story, &tpath) else { return ins_char(s, '\t') };
        let row_len = t.rows.get(r).map(|x| x.cells.len()).unwrap_or(0);
        let (nr, nc) = if c + 1 < row_len { (r, c + 1) } else { (r + 1, 0) };
        if nr >= t.rows.len() {
            let t = s.doc.table_mut(story, &tpath)?;
            t.insert_row(nr, r);
        }
        let mut path = tpath.0.clone();
        path.extend([nr as u32, nc as u32, 0]);
        let start = Pos { story, path: wordcraft_doc::Path(path), off: 0 };
        // Select the cell's content like Word.
        let end_path = s.doc.para_paths(story).into_iter().rfind(|q| q.0.starts_with(&start.path.0[..start.path.0.len() - 1]));
        let end = end_path.map(|q| {
            let off = s.doc.para(story, &q).map(|x| x.len()).unwrap_or(0);
            Pos { story, path: q, off }
        });
        s.sel = Selection { anchor: start.clone(), focus: end.unwrap_or(start) };
        return sel_result(s);
    }
    let f = s.sel.focus.clone();
    if s.sel.is_collapsed()
        && f.off == 0
        && let Some(n) = s.doc.para_at(&f).and_then(|x| x.props.numbering)
        && n.num != 0
    {
        let para = s.doc.para_mut(f.story, &f.path)?;
        para.props.numbering = Some(NumRef { num: n.num, level: n.level.saturating_add(1).min(8) });
        para.touch();
        return sel_result(s);
    }
    if s.sel.is_collapsed() && tab_indent_on(s) && tab_indents(s, &f) {
        return indent_by_tab(s, &f);
    }
    let whole_para = !s.sel.is_collapsed() && s.sel.anchor.path == s.sel.focus.path && {
        let (a, b) = s.sel.ordered();
        a.off == 0 && s.doc.para_at(&a).is_some_and(|p| b.off == p.len())
    };
    if !s.sel.is_collapsed() && (s.sel.anchor.path != s.sel.focus.path || whole_para) {
        // Several paragraphs or a whole paragraph selected: increase indent.
        return crate::cmd::para::indent(s, &Value::Null);
    }
    ins_char(s, '\t')
}

/// Is AutoFormat As You Type's "Set left- and first-indent with tabs and backspaces" on? Not
/// while the document limits formatting to its styles: the indents are direct formatting.
pub(super) fn tab_indent_on(s: &Session) -> bool {
    let ac = &s.prefs.autocorrect;
    ac.enabled && ac.tab_indent && !(crate::cmd::protect::enforced(&s.doc) && s.doc.settings.protect_formatting)
}

/// Does Tab at `f` indent instead of typing a tab? Only at the start of a line of a paragraph
/// with text. Not before the first line of a list item numbered by its style, where Tab types a
/// tab character as with the option off.
fn tab_indents(s: &mut Session, f: &Pos) -> bool {
    let Some(para) = s.doc.para_at(f).filter(|p| !p.is_empty()) else { return false };
    if f.off == 0 {
        return s.doc.styles.resolve_para(&para.props).numbering.is_none_or(|n| n.num == 0);
    }
    let l = s.layout();
    l.line_bounds(f, s.page_hint).is_some_and(|(a, _)| a == *f)
}

/// Tab before the first line indents the first line by one default tab stop; before any other
/// line, or when the first line is indented or hanging already, it indents the whole paragraph.
/// Before a later line of a list item, numbered directly or by its style, it takes the item down
/// a level.
fn indent_by_tab(s: &mut Session, f: &Pos) -> CmdResult {
    let Some(para) = s.doc.para_at(f) else { return sel_result(s) };
    let r = s.doc.styles.resolve_para(&para.props);
    if f.off > 0
        && para.props.numbering.is_none()
        && let Some(n) = r.numbering.filter(|n| n.num != 0)
    {
        let p = s.doc.para_mut(f.story, &f.path)?;
        p.props.numbering = Some(NumRef { num: n.num, level: n.level.saturating_add(1).min(8) });
        p.touch();
        return sel_result(s);
    }
    if f.off == 0 && r.indent_first == 0.0 && r.indent_chars[2] == 0.0 {
        let tab = s.doc.settings.default_tab.max(1.0);
        let p = s.doc.para_mut(f.story, &f.path)?;
        p.props.indent_first = Some(tab);
        p.props.cancel_char_indents(false, false, true);
        p.touch();
        return sel_result(s);
    }
    crate::cmd::para::indent(s, &Value::Null)
}

pub(super) fn back_tab(s: &mut Session, _: &Value) -> CmdResult {
    if let Some((tpath, r, c)) = s.sel.focus.path.cell() {
        let story = s.sel.focus.story;
        let (nr, nc) = if c > 0 {
            (r, c - 1)
        } else if r > 0 {
            (r - 1, s.doc.table(story, &tpath).and_then(|t| t.rows.get(r - 1)).map(|x| x.cells.len().saturating_sub(1)).unwrap_or(0))
        } else {
            return sel_result(s);
        };
        let mut path = tpath.0.clone();
        path.extend([nr as u32, nc as u32, 0]);
        s.sel = Selection::caret(Pos { story, path: wordcraft_doc::Path(path), off: 0 });
        return sel_result(s);
    }
    let f = s.sel.focus.clone();
    if let Some(n) = s.doc.para_at(&f).and_then(|x| x.props.numbering).filter(|n| n.num != 0) {
        let para = s.doc.para_mut(f.story, &f.path)?;
        para.props.numbering = Some(NumRef { num: n.num, level: n.level.saturating_sub(1) });
        para.touch();
        return sel_result(s);
    }
    crate::cmd::para::outdent(s, &Value::Null)
}
