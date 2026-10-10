//! Typing and deleting.

use serde_json::Value;
use wordcraft_doc::para::{COLUMN_BREAK, NB_HYPHEN, NBSP, PAGE_BREAK, SOFT_HYPHEN};
use wordcraft_doc::props::{Border, BorderStyle, NumRef};
use wordcraft_doc::{Block, ListKind, Pos};

use super::{delete_selection, sel_result, split_para, type_text};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("text.insert", "Type Text", "Editing", insert).params(r#"{"text": string}"#),
        CommandSpec::new("text.newParagraph", "New Paragraph", "Editing", new_paragraph).key("Enter"),
        CommandSpec::new("text.lineBreak", "Line Break", "Editing", |s, _| ins_char(s, '\n')).key("Shift+Enter"),
        CommandSpec::new("text.pageBreak", "Page Break", "Insert › Pages", |s, _| ins_char(s, PAGE_BREAK)).key("Mod+Enter"),
        CommandSpec::new("text.columnBreak", "Column Break", "Layout › Breaks", |s, _| ins_char(s, COLUMN_BREAK)).key("Mod+Shift+Enter"),
        CommandSpec::new("text.tab", "Tab", "Editing", tab).key("Tab"),
        CommandSpec::new("text.backTab", "Shift+Tab", "Editing", back_tab).key("Shift+Tab"),
        CommandSpec::new("text.backspace", "Backspace", "Editing", backspace).key("Backspace"),
        CommandSpec::new("text.delete", "Delete", "Editing", delete).key("Delete"),
        CommandSpec::new("text.deleteWordBack", "Delete Previous Word", "Editing", delete_word_back).key("Mod+Backspace / Alt+Backspace"),
        CommandSpec::new("text.deleteWordForward", "Delete Next Word", "Editing", delete_word_fwd).key("Mod+Delete / Alt+Delete"),
        CommandSpec::new("text.nbsp", "Nonbreaking Space", "Insert › Symbols", |s, _| ins_char(s, NBSP)).key("Mod+Shift+Space"),
        CommandSpec::new("text.nbHyphen", "Nonbreaking Hyphen", "Insert › Symbols", |s, _| ins_char(s, NB_HYPHEN)).key("Mod+Shift+-"),
        CommandSpec::new("text.optionalHyphen", "Optional Hyphen", "Insert › Symbols", |s, _| ins_char(s, SOFT_HYPHEN)).key("Mod+-"),
    ]
}

/// Smart quotes and AutoFormat-as-you-type (lists from "* " / "1. ", dashes).
fn autoformat(s: &mut Session, text: &str) -> Option<String> {
    if text.chars().count() != 1 {
        return None;
    }
    let c = text.chars().next()?;
    let f = &s.sel.focus;
    let para = s.doc.para_at(f)?;
    let before = para.text.get(..f.off)?;
    let prev = before.chars().next_back();
    let opening = prev.is_none_or(|p| p.is_whitespace() || "([{<\u{2014}\u{2013}".contains(p));
    match c {
        '"' => Some(if opening { "\u{201C}" } else { "\u{201D}" }.into()),
        '\'' => Some(if opening { "\u{2018}" } else { "\u{2019}" }.into()),
        _ => None,
    }
}

fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?;
    let raw = p::bool(v, "raw").unwrap_or(false);
    let t = if raw { None } else { autoformat(s, text) };
    type_text(s, t.as_deref().unwrap_or(text))?;
    let trigger = text.chars().count() == 1 && text.chars().all(|c| c == ' ' || ",.;:!?".contains(c));
    if raw || !trigger {
        return sel_result(s);
    }
    // The paragraph as typed, so an automatic change can be undone on its own.
    let f = s.sel.focus.clone();
    let typed = s.doc.para_at(&f).cloned().map(|p| (p, s.sel.clone()));
    // A list label ("a. ") becomes a list before AutoCorrect could capitalise it.
    let listed = text == " " && list_autoformat(s)?;
    if !listed {
        super::tools::autocorrect(s)?;
    }
    if text == " " && !listed {
        dash_autoformat(s)?;
    }
    if let Some((para, sel)) = typed
        && s.doc.para_at(&f) != Some(&para)
    {
        let mut doc = s.doc.clone();
        if let Ok(p) = doc.para_mut(f.story, &f.path) {
            *p = para;
            p.touch();
            s.push_undo("AutoFormat", doc, sel);
        }
    }
    sel_result(s)
}

/// "* " / "- " → bullets, "1. " / "a) " → numbering, typed at the start of a paragraph.
fn list_autoformat(s: &mut Session) -> Result<bool, CmdError> {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return Ok(false) };
    if para.props.numbering.is_some_and(|n| n.num != 0) || f.off != para.len() {
        return Ok(false);
    }
    let kind = match para.text.as_str() {
        "* " | "> " => ListKind::Bullet,
        "- " => ListKind::BulletChar('-'),
        "1. " => ListKind::Numbered,
        "1) " => ListKind::NumberedParen,
        "a. " => ListKind::LowerLetterDot,
        "a) " => ListKind::LowerLetter,
        "A. " => ListKind::UpperLetter,
        "i. " => ListKind::LowerRoman,
        "I. " => ListKind::Outline,
        _ => return Ok(false),
    };
    // Bullets join an existing bullet list; typing "1. " always starts numbering again at 1,
    // as in Word.
    let found = if kind.is_bullet() { s.doc.numbering.find_kind(kind) } else { None };
    let num = found.unwrap_or_else(|| s.doc.numbering.add_list(kind));
    let para = s.doc.para_mut(f.story, &f.path)?;
    let len = para.len();
    para.delete(0, len)?;
    para.props.numbering = Some(NumRef { num, level: 0 });
    para.props.style = Some("ListParagraph".into());
    s.sel = Selection::caret(Pos { off: 0, ..f });
    Ok(true)
}

/// "word -- word" → en dash, "word--word" → em dash (on the space after).
fn dash_autoformat(s: &mut Session) -> Result<(), CmdError> {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return Ok(()) };
    let Some(before) = para.text.get(..f.off) else { return Ok(()) };
    if before.ends_with(' ')
        && let Some(i) = before.trim_end().rfind(" -- ")
        && before.trim_end().get(i + 4..).is_some_and(|w| !w.is_empty() && !w.contains(' '))
    {
        // "word -- word" → "word – word" once the next word is typed.
        let p = s.doc.para_mut(f.story, &f.path)?;
        let props = p.props_at(i + 1).clone();
        p.delete(i + 1, i + 3)?;
        p.insert_text(i + 1, "\u{2013}", &props)?;
        s.sel = Selection::caret(Pos { off: f.off + 1, ..f });
    } else if before.ends_with(' ')
        && let Some(i) = before.trim_end().rfind(" - ")
        && !before.trim_end()[i + 3..].contains(' ')
    {
        let p = s.doc.para_mut(f.story, &f.path)?;
        let props = p.props_at(i + 1).clone();
        p.delete(i + 1, i + 2)?;
        p.insert_text(i + 1, "\u{2013}", &props)?;
        s.sel = Selection::caret(Pos { off: f.off + 2, ..f });
    } else if before.ends_with(' ')
        && let Some(i) = before.trim_end().rfind("--")
        && i > 0
        && before.trim_end().get(i - 1..i).is_some_and(|c| c != " ")
    {
        let p = s.doc.para_mut(f.story, &f.path)?;
        let props = p.props_at(i).clone();
        p.delete(i, i + 2)?;
        p.insert_text(i, "\u{2014}", &props)?;
        s.sel = Selection::caret(Pos { off: f.off + 1, ..f });
    }
    Ok(())
}

/// AutoFormat border lines: Enter after "---", "___", "===", "***", "~~~" or "###" turns the
/// characters into a bottom border on the paragraph above (or this one at the top of a story).
fn border_line_autoformat(s: &mut Session, at: &Pos) -> Result<bool, CmdError> {
    let Some(para) = s.doc.para_at(at) else { return Ok(false) };
    let text = para.text.as_str();
    let Some(c) = text.chars().next() else { return Ok(false) };
    if !s.autocorrect_on
        || at.off != text.len()
        || text.len() < 3
        || !text.chars().all(|x| x == c)
        || para.props.numbering.is_some_and(|n| n.num != 0)
    {
        return Ok(false);
    }
    let (style, width) = match c {
        '-' => (BorderStyle::Single, 0.75),
        '_' => (BorderStyle::Thick, 1.5),
        '=' => (BorderStyle::Double, 0.75),
        '*' => (BorderStyle::Dotted, 2.25),
        '~' => (BorderStyle::Wave, 0.75),
        '#' => (BorderStyle::Triple, 0.75),
        _ => return Ok(false),
    };
    let line = Border { style, width, color: None, space: 1.0 };
    let set_bottom = |p: &mut wordcraft_doc::Paragraph| {
        let mut b = p.props.borders.unwrap_or_default();
        b.bottom = Some(line);
        p.props.borders = Some(b);
        p.touch();
    };
    let i = at.path.last();
    let prev = if i > 0 { Some(at.path.with_last(i - 1)) } else { None };
    let len = text.len();
    let cur = s.doc.para_mut(at.story, &at.path)?;
    cur.delete(0, len)?;
    let start = Pos { off: 0, ..at.clone() };
    match prev.filter(|q| matches!(s.doc.block(at.story, q), Some(Block::Para(_)))) {
        Some(q) => {
            // The characters' paragraph becomes the one typing continues in.
            set_bottom(s.doc.para_mut(at.story, &q)?);
            s.sel = Selection::caret(start);
        }
        None => {
            set_bottom(s.doc.para_mut(at.story, &at.path)?);
            let new = split_para(s, &start)?;
            let p = s.doc.para_mut(new.story, &new.path)?;
            p.props.borders = None;
            p.touch();
            s.sel = Selection::caret(new);
        }
    }
    s.goal_x = None;
    Ok(true)
}

fn ins_char(s: &mut Session, c: char) -> CmdResult {
    let mut b = [0u8; 4];
    type_text(s, c.encode_utf8(&mut b))?;
    sel_result(s)
}

fn new_paragraph(s: &mut Session, _: &Value) -> CmdResult {
    let mut at = delete_selection(s)?;
    if border_line_autoformat(s, &at)? {
        return sel_result(s);
    }
    // Enter finishes a word like a space does ("teh" → "the", sentence capitals, links).
    if s.sel.is_collapsed() && at.off > 0 {
        super::tools::autocorrect_word(s, true)?;
        at = s.sel.focus.clone();
    }
    let new = split_para(s, &at)?;
    s.sel = Selection::caret(new);
    s.goal_x = None;
    sel_result(s)
}

/// In a table: next cell (adds a row at the end). At the start of a list item: demote. Else a tab.
fn tab(s: &mut Session, _: &Value) -> CmdResult {
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
        para.props.numbering = Some(NumRef { num: n.num, level: (n.level + 1).min(8) });
        para.touch();
        return sel_result(s);
    }
    if !s.sel.is_collapsed() && s.sel.anchor.path != s.sel.focus.path {
        // Multiple paragraphs selected: increase indent.
        return super::para::indent(s, &Value::Null);
    }
    ins_char(s, '\t')
}

fn back_tab(s: &mut Session, _: &Value) -> CmdResult {
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
    super::para::outdent(s, &Value::Null)
}

fn backspace(s: &mut Session, _: &Value) -> CmdResult {
    s.goal_x = None;
    s.pending = None;
    if !s.sel.is_collapsed() {
        delete_selection(s)?;
        return sel_result(s);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return sel_result(s) };
    if f.off > 0 {
        let a = Pos { off: para.prev_boundary(f.off), ..f.clone() };
        s.sel = Selection { anchor: a, focus: f };
        delete_selection(s)?;
        return sel_result(s);
    }
    // At the start of a paragraph.
    if para.props.numbering.is_some_and(|n| n.num != 0) {
        // First Backspace removes the number but keeps the item's indent (Word).
        let p = s.doc.para_mut(f.story, &f.path)?;
        p.props.numbering = Some(NumRef { num: 0, level: 0 });
        p.touch();
        return sel_result(s);
    }
    if para.props.style.as_deref() == Some("ListParagraph") {
        // The second one takes it out of the list's indent instead of joining paragraphs.
        super::leave_list(s.doc.para_mut(f.story, &f.path)?);
        return sel_result(s);
    }
    if para.props.indent_first.is_some_and(|x| x > 0.0) {
        let p = s.doc.para_mut(f.story, &f.path)?;
        p.props.indent_first = Some(0.0);
        p.touch();
        return sel_result(s);
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
            // Move into the table's last cell.
            let last = s.doc.para_paths(f.story).into_iter().rfind(|q| q.0.first() == prev_path.0.first() && q.0.len() > 1);
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

fn delete(s: &mut Session, _: &Value) -> CmdResult {
    s.goal_x = None;
    s.pending = None;
    if !s.sel.is_collapsed() {
        delete_selection(s)?;
        return sel_result(s);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return sel_result(s) };
    if f.off < para.len() {
        let b = Pos { off: para.next_boundary(f.off), ..f.clone() };
        s.sel = Selection { anchor: f, focus: b };
        delete_selection(s)?;
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
    }
    sel_result(s)
}

fn delete_word_back(s: &mut Session, v: &Value) -> CmdResult {
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
    sel_result(s)
}

fn delete_word_fwd(s: &mut Session, v: &Value) -> CmdResult {
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
    sel_result(s)
}
