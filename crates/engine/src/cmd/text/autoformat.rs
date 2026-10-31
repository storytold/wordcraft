use super::enter_tab::insert_paragraphs;
use super::{
    Block, Border, BorderStyle, CmdError, CmdResult, ListKind, NumRef, OBJ, Path, Pos, Selection, Session, StoryRef, Table, Value, delete_selection,
    p, sel_result, split_para, type_text_effect,
};

/// Smart quotes and AutoFormat-as-you-type (lists from "* " / "1. ", dashes).
fn autoformat(s: &mut Session, text: &str) -> Option<String> {
    if text.chars().count() != 1 {
        return None;
    }
    let c = text.chars().next()?;
    if !s.prefs.autocorrect.enabled || !s.prefs.autocorrect.smart_quotes {
        return None;
    }
    let f = &s.sel.focus;
    let para = s.doc.para_at(f)?;
    let before = para.text.get(..f.off)?;
    let prev = before.chars().next_back();
    let opening = prev.is_none_or(|p| {
        p.is_whitespace()
            || "([{<\u{2014}\u{2013}\u{00AB}\u{2039}\u{201E}\u{201A}".contains(p)
            || (c == '"' && p == '\u{2018}')
            || (c == '\'' && p == '\u{201C}')
    });
    match c {
        '"' => Some(if opening { "\u{201C}" } else { "\u{201D}" }.into()),
        '\'' => Some(if opening { "\u{2018}" } else { "\u{2019}" }.into()),
        _ => None,
    }
}

/// File › Options › Advanced before typed text goes in: with "Typing replaces selected text"
/// off the text goes in front of the selection and leaves it; in overtype the text replaces as
/// many characters after the caret as it has, up to the paragraph's end or an object (picture,
/// field), which stay.
pub(super) fn typing_options(s: &mut Session, text: &str) {
    if !s.sel.is_collapsed() {
        if !s.prefs.typing_replaces_selection {
            let (start, _) = s.sel.ordered();
            s.sel = Selection::caret(start);
        }
        return;
    }
    if !s.prefs.overtype || text.contains(['\r', '\n', '\u{2029}', '\u{000B}']) {
        return;
    }
    let f = s.sel.focus.clone();
    let Some(rest) = s.doc.para_at(&f).and_then(|p| p.text.get(f.off..)) else { return };
    let n = text.chars().count();
    let len: usize = rest.chars().take(n).take_while(|c| *c != OBJ).map(char::len_utf8).sum();
    if len > 0 {
        // The caret (focus) stays where typing happens; the characters after it are selected.
        s.sel = Selection { anchor: Pos { off: f.off + len, ..f.clone() }, focus: f };
    }
}

/// `text.insert`: type `text` at the selection as the keyboard does (AutoCorrect and
/// AutoFormat unless `raw`), or, with `paragraphs`, line by line with Enter between. A single
/// trigger character records the IME outcome, and list, AutoCorrect, dash and double-space
/// AutoFormat run as a separate undo step when they rewrite the paragraph.
pub(super) fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?;
    let raw = p::bool(v, "raw").unwrap_or(false);
    if p::bool(v, "paragraphs").unwrap_or(false) {
        return insert_paragraphs(s, text);
    }
    typing_options(s, text);
    let smart_quote = if raw { None } else { autoformat(s, text) };
    let was_collapsed = s.sel.is_collapsed();
    let effect = type_text_effect(s, smart_quote.as_deref().unwrap_or(text))?;
    let mut outcome = crate::ImeEditOutcome {
        classified: true,
        content_changed: effect.content_changed,
        state_changed: effect.state_changed,
        transformed: smart_quote.is_some(),
        selection_changed: false,
    };
    let trigger = text.chars().count() == 1 && text.chars().all(|c| c == ' ' || ",.;:!?".contains(c));
    if raw || !trigger {
        s.set_ime_edit_outcome(outcome);
        return sel_result(s);
    }
    // The paragraph as typed, so an automatic change can be undone on its own.
    let f = s.sel.focus.clone();
    let typed = s.doc.para_at(&f).cloned().map(|p| (p, s.sel.clone()));
    // A list label ("a. ") becomes a list before AutoCorrect could capitalise it.
    let listed = text == " " && s.prefs.autocorrect.enabled && list_autoformat(s)?;
    if !listed {
        crate::cmd::tools::autocorrect(s).inspect_err(|_| s.autocap = None)?;
    }
    let dashed = text == " "
        && !listed
        && s.prefs.autocorrect.enabled
        && s.prefs.autocorrect.dashes
        && dash_autoformat(s).inspect_err(|_| s.autocap = None)?;
    if text == " " && !listed && was_collapsed && s.prefs.autocorrect.enabled && s.prefs.autocorrect.double_space_period {
        double_space_period(s)?;
    }
    // Compare the paragraph with the one exactly as typed. This is the actual effect of
    // AutoCorrect (including formatting-only superscripts and links), not an undo label that
    // may have been left by an earlier step. It also covers the double-space period.
    let rewritten = typed.as_ref().is_some_and(|(para, _)| s.doc.para_at(&f) != Some(para));
    if listed || dashed || rewritten {
        outcome.content_changed = true;
        outcome.state_changed = true;
        outcome.transformed = true;
    }
    if let Some((para, sel)) = typed
        && s.doc.para_at(&f) != Some(&para)
    {
        let mut doc = s.doc.clone();
        if let Ok(p) = doc.para_mut(f.story, &f.path) {
            *p = para;
            p.touch();
            s.push_undo("AutoFormat", doc, sel);
            // Undo right after it makes the capitalised word an exception.
            let depth = s.undo_steps_so_far();
            if let Some(c) = s.autocap.as_mut().filter(|c| c.depth == u64::MAX) {
                c.depth = depth;
            }
        }
    }
    s.autocap.take_if(|c| c.depth == u64::MAX);
    s.set_ime_edit_outcome(outcome);
    sel_result(s)
}

/// "* " / "- " / "> " → bullets, "1. " / "a) " → numbering, typed at the start of a paragraph
/// (Word triggers on the paragraph's first characters, whatever text already follows the caret,
/// not only when the trigger is the paragraph's entire content — #3352).
fn list_autoformat(s: &mut Session) -> Result<bool, CmdError> {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return Ok(false) };
    // A style-linked list (a heading-linked Multilevel List preset, #1793) numbers the paragraph
    // without `props.numbering` itself being set: resolve it, so autoformat doesn't paper over an
    // already-numbered heading with a fresh bullet.
    if s.doc.styles.resolve_para(&para.props).numbering.is_some_and(|n| n.num != 0) {
        return Ok(false);
    }
    let Some(prefix) = para.text.get(..f.off) else { return Ok(false) };
    let kind = match prefix {
        "* " => ListKind::Bullet,
        "- " => ListKind::BulletChar('-'),
        // Arrow bullets (Word draws these from a symbol font; we use the Unicode arrows).
        "> " => ListKind::BulletChar('\u{27A2}'),
        "-> " => ListKind::BulletChar('\u{2794}'),
        "=> " => ListKind::BulletChar('\u{21E8}'),
        "1. " => ListKind::Numbered,
        "1) " => ListKind::NumberedParen,
        "a. " => ListKind::LowerLetterDot,
        "a) " => ListKind::LowerLetter,
        "A. " => ListKind::UpperLetter,
        "i. " => ListKind::LowerRoman,
        "I. " => ListKind::Outline,
        _ => return Ok(false),
    };
    let ac = &s.prefs.autocorrect;
    if !(if kind.is_bullet() { ac.bullets } else { ac.numbering }) {
        return Ok(false);
    }
    // Bullets join an existing bullet list; typing "1. " always starts numbering again at 1,
    // as in Word.
    let found = if kind.is_bullet() { s.doc.numbering.find_kind(kind) } else { None };
    let num = match found {
        Some(n) => n,
        None => s.doc.numbering.try_add_list(kind)?,
    };
    let para = s.doc.para_mut(f.story, &f.path)?;
    para.delete(0, f.off)?;
    para.props.numbering = Some(NumRef { num, level: 0 });
    // A heading stays a heading (Word: "1. " in a Heading 1 gives a numbered Heading 1).
    if para.props.style.as_deref().is_none_or(|st| st == "Normal") {
        para.props.style = Some("ListParagraph".into());
    }
    s.sel = Selection::caret(Pos { off: 0, ..f });
    Ok(true)
}

/// "word" + two spaces → "word. ": the second Space turns the first into a period, after a
/// letter, a digit or a closing bracket or quote; not after other punctuation, at the start of a
/// paragraph, after a tab, or for a third space (the text before them ends in a period by then).
fn double_space_period(s: &mut Session) -> Result<(), CmdError> {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return Ok(()) };
    let Some(before) = para.text.get(..f.off) else { return Ok(()) };
    let Some(word) = before.strip_suffix("  ") else { return Ok(()) };
    if !word.chars().next_back().is_some_and(|c| c.is_alphanumeric() || ")]}\"'\u{201D}\u{2019}\u{00BB}\u{203A}".contains(c)) {
        return Ok(());
    }
    // The period takes the formatting of the space it replaces.
    let at = word.len();
    let p = s.doc.para_mut(f.story, &f.path)?;
    let props = p.props_at(at + 1).clone().without_symbol();
    p.delete(at, at + 1)?;
    p.insert_text(at, ".", &props)?;
    s.sel = Selection::caret(f);
    Ok(())
}

/// "word -- word" → en dash, "word--word" → em dash (on the space after).
fn dash_autoformat(s: &mut Session) -> Result<bool, CmdError> {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return Ok(false) };
    let Some(before) = para.text.get(..f.off) else { return Ok(false) };
    if before.ends_with(' ')
        && let Some(i) = before.trim_end().rfind(" -- ")
        && before.trim_end().get(i + 4..).is_some_and(|w| !w.is_empty() && !w.contains(' '))
    {
        // "word -- word" → "word – word" once the next word is typed.
        let p = s.doc.para_mut(f.story, &f.path)?;
        let props = p.props_at(i + 1).clone().without_symbol();
        p.delete(i + 1, i + 3)?;
        p.insert_text(i + 1, "\u{2013}", &props)?;
        s.sel = Selection::caret(Pos { off: f.off + 1, ..f });
        return Ok(true);
    } else if before.ends_with(' ')
        && let Some(i) = before.trim_end().rfind(" - ")
        && !before.trim_end()[i + 3..].contains(' ')
    {
        let p = s.doc.para_mut(f.story, &f.path)?;
        let props = p.props_at(i + 1).clone().without_symbol();
        p.delete(i + 1, i + 2)?;
        p.insert_text(i + 1, "\u{2013}", &props)?;
        s.sel = Selection::caret(Pos { off: f.off + 2, ..f });
        return Ok(true);
    } else if before.ends_with(' ')
        && let Some(i) = before.trim_end().rfind("--")
        && i > 0
        && before.trim_end()[..i].chars().next_back().is_some_and(|c| c != ' ')
    {
        let p = s.doc.para_mut(f.story, &f.path)?;
        let props = p.props_at(i).clone().without_symbol();
        p.delete(i, i + 2)?;
        p.insert_text(i, "\u{2014}", &props)?;
        s.sel = Selection::caret(Pos { off: f.off + 1, ..f });
        return Ok(true);
    }
    Ok(false)
}

/// An AutoFormat that Enter applies to the paragraph just finished.
pub(super) enum EnterAutoFormat {
    /// "---" and friends: a bottom border.
    Border(Border),
    /// "+---+-----+": a one-row table; the dash count of each column.
    Table(Vec<usize>),
    /// A short line before an empty paragraph: Heading 1, 2 or 3.
    Heading(u8),
}

/// AutoFormat border lines: Enter after "---", "___", "===", "***", "~~~" or "###". A line
/// holding a tracked deletion stays as it is.
pub(super) fn border_line(s: &Session, at: &Pos) -> Option<Border> {
    let para = s.doc.para_at(at)?;
    let text = para.text.as_str();
    let c = text.chars().next()?;
    if !s.prefs.autocorrect.enabled
        || !s.prefs.autocorrect.border_lines
        || at.off != text.len()
        || text.len() < 3
        || !text.chars().all(|x| x == c)
        || s.doc.styles.resolve_para(&para.props).numbering.is_some_and(|n| n.num != 0)
        || para.run_ranges().any(|(_, r)| r.del.is_some())
    {
        return None;
    }
    let (style, width) = match c {
        '-' => (BorderStyle::Single, 0.75),
        '_' => (BorderStyle::Thick, 1.5),
        '=' => (BorderStyle::Double, 0.75),
        '*' => (BorderStyle::Dotted, 2.25),
        '~' => (BorderStyle::Wave, 0.75),
        '#' => (BorderStyle::Triple, 0.75),
        _ => return None,
    };
    Some(Border { style, width, color: None, space: 1.0 })
}

/// AutoFormat tables: a body paragraph that is exactly "+---+-----+" (plus signs around runs
/// of hyphens). Returns the hyphen count of each column; more columns than a table can hold,
/// or a line holding a tracked deletion, leave the text alone.
pub(super) fn table_line(s: &Session, at: &Pos) -> Option<Vec<usize>> {
    let para = s.doc.para_at(at)?;
    let text = para.text.as_str();
    if !s.prefs.autocorrect.enabled
        || at.story != StoryRef::Body
        || at.path.0.len() != 1
        || at.off != text.len()
        || s.doc.styles.resolve_para(&para.props).numbering.is_some_and(|n| n.num != 0)
        || para.run_ranges().any(|(_, c)| c.del.is_some())
    {
        return None;
    }
    let inner = text.strip_prefix('+')?.strip_suffix('+')?;
    let mut cols = Vec::new();
    for run in inner.split('+') {
        if run.is_empty() || !run.bytes().all(|b| b == b'-') || cols.len() >= wordcraft_doc::table::MAX_COLS {
            return None;
        }
        cols.push(run.len());
    }
    Some(cols)
}

/// AutoFormat Built-in Heading styles: Enter in an empty body paragraph whose previous paragraph
/// is a line of Normal text with one to five words that ends in a letter or digit and holds no
/// tracked deletion. The line becomes Heading 1, or Heading 2 or 3 when it starts with one or
/// two tabs.
pub(super) fn heading_line(s: &Session, at: &Pos) -> Option<u8> {
    let ac = &s.prefs.autocorrect;
    let cur = s.doc.para_at(at)?;
    let i = at.path.last();
    if !ac.enabled || !ac.heading_styles || at.story != StoryRef::Body || at.path.0.len() != 1 || i == 0 || !cur.is_empty() {
        return None;
    }
    let Some(Block::Para(prev)) = s.doc.block(at.story, &at.path.with_last(i - 1)) else { return None };
    // Normal and outside a list, also one the style sheet gives (a numbered Normal style).
    let plain = |p: &wordcraft_doc::Paragraph| {
        let r = s.doc.styles.resolve_para(&p.props);
        p.props.style.as_deref().is_none_or(|st| st == "Normal") && r.numbering.is_none_or(|n| n.num == 0) && r.outline_level.is_none_or(|l| l >= 9)
    };
    let text = prev.text.trim_start_matches('\t');
    let tabs = prev.text.len() - text.len();
    if !plain(cur)
        || !plain(prev)
        || !prev.objects.is_empty()
        || prev.section.is_some()
        || prev.run_ranges().any(|(_, c)| c.del.is_some())
        || tabs > 2
        || !(1..=5).contains(&text.split_whitespace().count())
        || text.starts_with(char::is_whitespace)
        || text.contains(char::is_control)
        || !text.ends_with(char::is_alphanumeric)
    {
        return None;
    }
    let level = tabs as u8 + 1;
    // Formatting limited to the document's styles: a locked heading style isn't applied.
    let id = format!("Heading{level}");
    if crate::cmd::protect::enforced(&s.doc) && s.doc.settings.protect_formatting && s.doc.styles.get(&id).is_some_and(|st| st.locked) {
        return None;
    }
    Some(level)
}

/// Give the paragraph before the empty one at `at` Heading `level` without its leading tabs;
/// the caret stays in the empty paragraph. With Track Changes on, the tabs are marked deleted
/// (or removed, when this author typed them) and the style change is a formatting revision.
pub(super) fn apply_heading_line(s: &mut Session, at: &Pos, level: u8) -> Result<(), CmdError> {
    let prev = at.path.with_last(at.path.last().saturating_sub(1));
    let Some(before) = s.doc.para(at.story, &prev).cloned() else { return Ok(()) };
    let tabs = before.text.len() - before.text.trim_start_matches('\t').len();
    let start = Pos { story: at.story, path: prev.clone(), off: 0 };
    s.sel = Selection { anchor: start.clone(), focus: Pos { off: tabs, ..start } };
    delete_selection(s)?;
    crate::cmd::para::set_heading(s, at.story, &prev, Some(level))?;
    // Removed tabs change the text, and the formatting comparison after the command only looks
    // at paragraphs that keep theirs, so the style change is recorded here.
    if s.doc.settings.track_changes
        && s.prefs.markup.track_formatting
        && s.doc.para(at.story, &prev).is_some_and(|p| p.text != before.text && p.props.fmt_change.is_none())
    {
        let rev = crate::cmd::new_revision(s, wordcraft_doc::RevisionKind::Format);
        let p = s.doc.para_mut(at.story, &prev)?;
        p.props.fmt_change = wordcraft_doc::props::PropChange::boxed(rev, before.props.formatting());
        p.touch();
    }
    s.sel = Selection::caret(at.clone());
    Ok(())
}

/// Turn the border-line characters into a bottom border on the paragraph above (or this one
/// at the top of a story). With Track Changes on, the characters are marked deleted (or
/// removed, when this author typed them) and the border is a formatting revision.
pub(super) fn apply_border_line(s: &mut Session, at: &Pos, line: Border) -> Result<(), CmdError> {
    let set_bottom = |p: &mut wordcraft_doc::Paragraph| {
        let mut b = p.props.borders.unwrap_or_default();
        b.bottom = Some(line);
        p.props.borders = Some(b);
        p.touch();
    };
    let i = at.path.last();
    let prev = if i > 0 { Some(at.path.with_last(i - 1)) } else { None };
    let end = delete_typed_line(s, at)?;
    match prev.filter(|q| matches!(s.doc.block(at.story, q), Some(Block::Para(_)))) {
        Some(q) => {
            // The characters' paragraph becomes the one typing continues in.
            set_bottom(s.doc.para_mut(at.story, &q)?);
            s.sel = Selection::caret(end);
        }
        None => {
            let before = s.doc.para(at.story, &at.path).map(|p| p.props.clone());
            set_bottom(s.doc.para_mut(at.story, &at.path)?);
            let new = split_para(s, &end)?;
            let p = s.doc.para_mut(new.story, &new.path)?;
            p.props.borders = None;
            p.touch();
            // The split changes the block count, and the formatting comparison after the command
            // skips a container whose block count changed, so the border is recorded here.
            if s.doc.settings.track_changes
                && s.prefs.markup.track_formatting
                && let Some(before) = before.filter(|b| b.fmt_change.is_none())
            {
                let rev = crate::cmd::new_revision(s, wordcraft_doc::RevisionKind::Format);
                let p = s.doc.para_mut(at.story, &at.path)?;
                p.props.fmt_change = wordcraft_doc::props::PropChange::boxed(rev, before.formatting());
                p.touch();
            }
            s.sel = Selection::caret(new);
        }
    }
    Ok(())
}

/// Delete the AutoFormat characters of the paragraph at `at`, tracked when Track Changes is on.
/// Returns the end of what is left: the paragraph's end, after any characters marked deleted.
fn delete_typed_line(s: &mut Session, at: &Pos) -> Result<Pos, CmdError> {
    let len = s.doc.para(at.story, &at.path).map_or(0, |p| p.len());
    s.sel = Selection { anchor: Pos { off: 0, ..at.clone() }, focus: Pos { off: len, ..at.clone() } };
    delete_selection(s)?;
    let left = s.doc.para(at.story, &at.path).map_or(0, |p| p.len());
    Ok(Pos { off: left, ..at.clone() })
}

/// Replace the "+---+" characters with a one-row table across the text width, each column as
/// wide as its share of the hyphens; the emptied paragraph stays after the table and the caret
/// goes into the first cell. With Track Changes on, the characters are marked deleted (or
/// removed, when this author typed them); the table is inserted untracked, as Insert › Table
/// inserts one.
pub(super) fn apply_table_line(s: &mut Session, at: &Pos, cols: &[usize]) -> Result<(), CmdError> {
    let total = cols.iter().sum::<usize>().max(1) as f32;
    let block = at.path.0.first().copied().unwrap_or(0) as usize;
    let width = s.doc.laid_out_sections().get(s.doc.section_index_of(block)).map(|(_, sp)| sp.text_width()).unwrap_or(468.0).max(36.0);
    let mut t = Table::new(1, cols.len(), width);
    for (i, d) in cols.iter().enumerate() {
        let w = (width * *d as f32 / total).max(6.0);
        if let Some(g) = t.grid.get_mut(i) {
            *g = w;
        }
        if let Some(c) = t.rows.get_mut(0).and_then(|r| r.cells.get_mut(i)) {
            c.props.width = Some(w);
        }
    }
    delete_typed_line(s, at)?;
    s.doc.insert_block(at.story, &at.path, Block::Table(t))?;
    let mut first = at.path.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: at.story, path: Path(first), off: 0 });
    Ok(())
}
