//! Command implementations, one module per area. Each module exposes `specs()`.

pub mod accessibility;
pub mod canvas;
pub mod caret;
pub mod chart;
pub mod citations;
pub mod column;
pub mod connectors;
pub mod controls;
pub mod design;
pub mod draw;
pub mod edit;
pub mod equation;
pub mod fields;
pub mod file;
pub mod find_pattern;
pub mod fmt_revisions;
pub mod font;
pub mod format;
pub mod index;
pub mod ink_shape;
pub mod insert;
pub mod inspector;
pub mod lists;
pub mod mailings;
pub mod moves;
pub mod objects;
pub mod options;
pub mod outline;
pub mod page;
pub mod para;
pub mod paste;
pub mod phonetic;
pub mod protect;
pub mod references;
pub mod review;
pub mod shape_fill;
pub mod smart_art;
pub mod speech;
pub mod table;
pub mod table_draw;
pub mod table_formula;
pub mod table_mouse;
pub mod table_resize;
pub mod table_style;
pub mod templates;
pub mod text;
pub mod themes;
pub mod tools;
pub mod user_templates;
pub mod view;
pub mod wordart;

use serde_json::{Value, json};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Pos, Revision, RevisionKind, StoryRef};

use crate::{CmdError, CmdResult, Registry, Session};

/// The full registry.
pub fn registry() -> Registry {
    let mut v = Vec::new();
    v.extend(text::specs());
    v.extend(caret::specs());
    v.extend(column::specs());
    v.extend(edit::specs());
    v.extend(paste::specs());
    v.extend(format::specs());
    v.extend(font::specs());
    v.extend(phonetic::specs());
    v.extend(para::specs());
    v.extend(lists::specs());
    v.extend(inspector::specs());
    v.extend(view::specs());
    v.extend(outline::specs());
    v.extend(insert::specs());
    v.extend(equation::specs());
    v.extend(page::specs());
    v.extend(table::specs());
    v.extend(table_style::specs());
    v.extend(table_draw::specs());
    v.extend(table_resize::specs());
    v.extend(table_mouse::specs());
    v.extend(review::specs());
    v.extend(file::specs());
    v.extend(options::specs());
    v.extend(design::specs());
    v.extend(themes::specs());
    v.extend(references::specs());
    v.extend(fields::specs());
    v.extend(mailings::specs());
    v.extend(citations::specs());
    v.extend(index::specs());
    v.extend(objects::specs());
    v.extend(shape_fill::specs());
    v.extend(smart_art::specs());
    v.extend(chart::specs());
    v.extend(draw::specs());
    v.extend(canvas::specs());
    v.extend(wordart::specs());
    v.extend(connectors::specs());
    v.extend(tools::specs());
    v.extend(templates::specs());
    v.extend(user_templates::specs());
    v.extend(protect::specs());
    v.extend(speech::specs());
    v.extend(controls::specs());
    Registry::new(v)
}

/// A shortcut that differs on macOS, where Word for Mac moves it (⌘Q quits, ⌘Space is
/// Spotlight, ⌘⌥H hides other apps…): `mac` there, `other` everywhere else.
pub const fn mac_or(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") { mac } else { other }
}

/// A shortcut with no macOS equivalent (Option+letter types the character there).
pub const fn non_mac(k: &'static str) -> &'static str {
    mac_or("", k)
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

/// The paragraphs the selection covers: those between its ends (not the next one when it ends at
/// that one's start, #418) or, for whole table cells that aren't one run of the text (part of a
/// column), just the cells' own.
pub fn selected_paths(s: &Session) -> Vec<wordcraft_doc::Path> {
    if s.cell_range().is_some()
        && let Some(segs) = s.column_segments()
    {
        return segs.iter().map(|(a, _)| a.path.clone()).collect();
    }
    let (a, b) = s.sel.ordered();
    s.doc.para_paths_in(&a, &b)
}

/// Delete the selection (respecting track changes); returns the collapsed position.
pub fn delete_selection(s: &mut Session) -> Result<Pos, CmdError> {
    let (a, b) = s.sel.ordered();
    if a == b {
        return Ok(a);
    }
    controls::check_delete(s, &a, &b)?;
    let at = if s.doc.settings.track_changes { track_delete(s, &a, &b)? } else { s.doc.delete_range(&a, &b)? };
    s.sel = crate::Selection::caret(at.clone());
    Ok(at)
}

/// A revision record for the current author.
pub fn new_revision(s: &mut Session, kind: RevisionKind) -> u32 {
    let date = now_iso();
    // Never one end of a move, nor the deletion a cut may still turn into one.
    let held = s.pending_move.as_ref().map(|m| m.rev);
    if let Some((i, _)) = s
        .doc
        .revisions
        .iter()
        .enumerate()
        .rev()
        .find(|(i, r)| r.kind == kind && r.author == s.author && r.date == date && r.move_name.is_none() && held != Some(*i as u32))
    {
        return i as u32;
    }
    s.doc.revisions.push(Revision { kind, author: s.author.clone(), date, move_name: None });
    (s.doc.revisions.len() - 1) as u32
}

/// Whether the paragraph at `path` is followed by another paragraph in the same container (so
/// its paragraph mark can be removed by joining the two).
pub fn has_next_para(s: &Session, story: StoryRef, path: &wordcraft_doc::Path) -> bool {
    matches!(s.doc.block(story, &path.with_last(path.last().saturating_add(1))), Some(wordcraft_doc::Block::Para(_)))
}

/// Remove the paragraph mark of the paragraph at `path`: the next paragraph in the same container
/// joins it. The joined paragraph keeps this paragraph's properties and ends with the next one's
/// mark (and that mark's revisions). Returns `false`, changing nothing, when no paragraph follows
/// in the container (a story's or cell's last mark can't be removed).
pub fn join_next_para(s: &mut Session, story: StoryRef, path: &wordcraft_doc::Path) -> Result<bool, CmdError> {
    if !has_next_para(s, story, path) {
        return Ok(false);
    }
    let next = path.with_last(path.last().saturating_add(1));
    let len = s.doc.para(story, path).map(|p| p.len()).unwrap_or(0);
    let revs = s.doc.para(story, &next).map(|p| (p.mark.ins, p.mark.del)).unwrap_or_default();
    s.doc.delete_range(&Pos { story, path: path.clone(), off: len }, &Pos { story, path: next, off: 0 })?;
    let p = s.doc.para_mut(story, path)?;
    (p.mark.ins, p.mark.del) = revs;
    p.touch();
    Ok(true)
}

/// Remove the paragraph at `path` when it is empty, carries no section break and another
/// paragraph follows in the same container: the next paragraph keeps its own properties. Returns
/// `false`, changing nothing, otherwise.
pub fn remove_empty_para(s: &mut Session, story: StoryRef, path: &wordcraft_doc::Path) -> Result<bool, CmdError> {
    let empty = s.doc.para(story, path).is_some_and(|p| p.is_empty() && p.section.is_none());
    if !empty || !has_next_para(s, story, path) {
        return Ok(false);
    }
    s.doc.remove_block(story, path)?;
    Ok(true)
}

/// Tracked deletion: own insertions are removed, other text is marked deleted.
pub(crate) fn track_delete(s: &mut Session, a: &Pos, b: &Pos) -> Result<Pos, CmdError> {
    let rid = new_revision(s, RevisionKind::Delete);
    track_delete_as(s, a, b, rid)
}

/// [`track_delete`] marking the deleted text with revision `rid`.
pub(crate) fn track_delete_as(s: &mut Session, a: &Pos, b: &Pos, rid: u32) -> Result<Pos, CmdError> {
    let author = s.author.clone();
    // Remove text this author inserted (it never existed for the reader); mark the rest.
    let paths = s.doc.paths_between(a, b);
    for path in paths.iter().rev() {
        let Some(p) = s.doc.para(a.story, path) else { continue };
        let from = if *path == a.path { a.off } else { 0 };
        let to = if *path == b.path { b.off } else { p.len() };
        // Text already deleted keeps its deletion (and its author), like Word.
        let ranges: Vec<(usize, usize, bool)> = p
            .run_ranges()
            .filter(|(r, c)| r.end > from && r.start < to && c.del.is_none())
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
        // The paragraph mark is inside the range unless this is its last paragraph. Like text,
        // a mark this author inserted is removed (the paragraphs join again), any other is
        // marked deleted; one already deleted keeps its deletion. Done after the text so a join
        // can't pull the next paragraph's text into this one's range.
        if *path != b.path && has_next_para(s, a.story, path) {
            let (ins, del) = s.doc.para(a.story, path).map(|p| (p.mark.ins, p.mark.del)).unwrap_or_default();
            if del.is_none() {
                let own = ins.and_then(|i| s.doc.revisions.get(i as usize)).is_some_and(|rv| rv.author == author);
                if own {
                    join_next_para(s, a.story, path)?;
                } else {
                    let p = s.doc.para_mut(a.story, path)?;
                    p.mark.del = Some(rid);
                    p.touch();
                }
            }
        }
    }
    Ok(a.clone())
}

/// Insert typed text at the caret (replacing the selection), with pending formatting and track changes.
pub fn type_text(s: &mut Session, text: &str) -> Result<(), CmdError> {
    insert_text(s, text, true)
}

/// Insert pasted plain text like [`type_text`], except that its paragraph separators are text
/// being transferred, not Enter key presses: an empty list item between them stays (Enter on
/// one would end the list).
pub fn paste_text(s: &mut Session, text: &str) -> Result<(), CmdError> {
    insert_text(s, text, false)
}

fn insert_text(s: &mut Session, text: &str, enter: bool) -> Result<(), CmdError> {
    let mut props = s.typing_props();
    if s.sel.is_collapsed() {
        s.sel = crate::Selection::caret(s.doc.past_field_ends(&s.sel.focus));
    }
    // Content controls: a placeholder is replaced, locked ones refuse.
    if let Some(p) = controls::prepare_edit(s)? {
        props = p;
    }
    // Pasted text is text: pending properties left by AutoCorrect don't give it a symbol's font (#2036).
    if !enter {
        props = props.without_symbol();
    }
    delete_selection(s)?;
    // Typed text is new: never deleted, and not a formatting change of the text around it.
    props.fmt_change = None;
    // Its direction comes from its own letters, as on screen, not from the run it's typed into:
    // Latin typed into reopened Hebrew must not be saved as right-to-left (#1543). The DOCX
    // writer marks the right-to-left pieces itself.
    props.rtl = None;
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
            at = split_para_as(s, &at, enter)?;
        }
        first = false;
        let part = part.replace('\u{000B}', "\n");
        if !part.is_empty() {
            let starts_para = direction_unset(s, &at);
            at = s.doc.insert_text(&at, &part, &props)?;
            // Typing Arabic, Hebrew… as the first letters of a paragraph whose direction nothing
            // sets makes it a right-to-left paragraph, so the caret and text start at the right
            // margin. Paragraphs that already have letters, or a direction, keep theirs.
            if starts_para
                && let Ok(p) = s.doc.para_mut(at.story, &at.path)
                && wordcraft_doc::bidi::first_strong_is_rtl(&p.text)
            {
                p.props.bidi = Some(true);
                p.touch();
            }
        }
    }
    s.sel = crate::Selection::caret(at);
    s.goal_x = None;
    Ok(())
}

/// Is the paragraph at `at` still without letters (empty, or only spaces, digits and
/// punctuation), with no direction set on it, its style chain or the document defaults?
fn direction_unset(s: &Session, at: &Pos) -> bool {
    let Some(p) = s.doc.para_at(at) else { return false };
    if p.props.bidi.is_some() || p.text.chars().any(char::is_alphabetic) || s.doc.styles.default_para.bidi.is_some() {
        return false;
    }
    let style = p.props.style.as_deref().unwrap_or("Normal");
    !s.doc.styles.chain(style).iter().any(|st| st.para.bidi.is_some())
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
    para.props.indent_left_chars = None;
    para.props.indent_first_chars = None;
    para.touch();
}

/// Split the paragraph at `at` like Enter does: an empty list paragraph leaves the list, the
/// next paragraph gets the style's "next" style when Enter is at the end.
pub fn split_para(s: &mut Session, at: &Pos) -> Result<Pos, CmdError> {
    split_para_as(s, at, true)
}

/// [`split_para`]; without `enter` an empty list item is split like any other paragraph.
fn split_para_as(s: &mut Session, at: &Pos, enter: bool) -> Result<Pos, CmdError> {
    // `num: 0` means "explicitly not in a list", so it isn't a list item. The list may come from
    // the paragraph's style (List Number 2 in a Word file) rather than the paragraph itself.
    let (at_end, style, empty_list) = match s.doc.para_at(at) {
        Some(p) => (
            at.off >= p.len(),
            p.props.style.clone(),
            p.props.numbering.or_else(|| s.doc.styles.resolve_para(&p.props).numbering).filter(|n| enter && n.num != 0 && p.is_empty()),
        ),
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
    let mark_revs = s.doc.para_at(at).map(|p| (p.mark.ins, p.mark.del)).unwrap_or_default();
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
        // The paragraph after the split ends with the original mark: it keeps that mark's
        // revisions, never those of the text at the split point (which would credit the split
        // to the author who inserted that text).
        let t = s.doc.para_mut(new.story, &new.path)?;
        (t.mark.ins, t.mark.del) = mark_revs;
    }
    Ok(new)
}

/// ISO-8601 timestamp (UTC, seconds) for `secs` since the Unix epoch.
pub fn iso_from_unix_secs(secs: u64) -> String {
    use chrono::Datelike;
    let t = secs % 86_400;
    let hms = format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60);
    // Within chrono's range, read the date from it; past it, keep the proleptic Gregorian answer
    // `civil_from_days` gives rather than falling back to the epoch.
    let (y, m, d) = match i64::try_from(secs).ok().and_then(|s| chrono::DateTime::from_timestamp(s, 0)) {
        Some(dt) => (i64::from(dt.year()), dt.month(), dt.day()),
        None => civil_from_days((secs / 86_400) as i64),
    };
    // `{y:04}` rather than `%Y`, which writes a `+` before years past 9999.
    format!("{y:04}-{m:02}-{d:02}T{hms}Z")
}

/// Seconds since the Unix epoch (UTC): the system clock, or the browser's on the web.
pub fn now_unix() -> u64 {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let ms = js_sys::Date::now();
        // Clamp a NaN or pre-epoch clock to 0 rather than fail.
        if ms.is_finite() && ms > 0.0 { (ms / 1000.0) as u64 } else { 0 }
    }
}

/// ISO-8601 timestamp (UTC, seconds).
pub fn now_iso() -> String {
    iso_from_unix_secs(now_unix())
}

/// Days since 1970-01-01 → (year, month, day).
///
/// `NaiveDate` only covers about ±262143 years, but this is reached by the system clock (and by
/// [`iso_from_unix_secs`] for a `u64` seconds count past it), so it must stay exact for every
/// `z`. The Gregorian calendar repeats every 400 years (146_097 days): chrono converts the day
/// within its cycle and whole cycles go back into the year (an i128 so i64::MIN/MAX can't
/// overflow). 1970-01-01 is `num_days_from_ce` 719_163, and 0..146_097 days from CE day 0 always
/// fit chrono. The result matches the proleptic Gregorian count the old hand-rolled code gave.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    use chrono::Datelike;
    let (cycles, rest) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    match i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt) {
        Some(d) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), d.month(), d.day())
        }
        // Unreachable: `rest + 719_163` is 719_163..865_259, always a valid CE day.
        None => (1970, 1, 1),
    }
}

/// Story given in params (`"story": "body"` / `{"part": 3}`), else the caret's.
pub fn story_param(s: &Session, v: &Value) -> StoryRef {
    v.get("story").and_then(|x| serde_json::from_value(x.clone()).ok()).unwrap_or(s.sel.focus.story)
}

/// The language (`w:lang`) of a paragraph's text: that of its first run with text (else of the
/// paragraph mark), through styles and the document defaults. Sorting and number parsing follow it.
pub fn para_lang(doc: &wordcraft_doc::Document, p: &wordcraft_doc::Paragraph) -> Option<String> {
    let run = p.runs.iter().find(|r| r.len > 0).map(|r| &r.props).unwrap_or(&p.mark);
    doc.styles.resolve_char(p.props.style.as_deref(), run).lang
}

/// The document's default text language (`w:docDefaults`, the Normal style).
pub fn doc_lang(doc: &wordcraft_doc::Document) -> Option<String> {
    doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default()).lang
}

/// The language of text that names none: the document's default, else the editing language (a
/// Polish interface writes Polish), else none (English). As AutoCorrect decides it.
pub fn default_lang(s: &Session) -> Option<String> {
    doc_lang(&s.doc).or_else(|| s.editing_lang.clone())
}

#[cfg(test)]
mod iso_tests {
    use super::{civil_from_days, iso_from_unix_secs};

    #[test]
    fn formats_epoch_and_known_timestamps() {
        assert_eq!(iso_from_unix_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix_secs(1_760_082_785), "2025-10-10T07:53:05Z");
    }

    #[test]
    fn formats_leap_day_and_year_end() {
        assert_eq!(iso_from_unix_secs(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(iso_from_unix_secs(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_from_unix_secs(1_735_689_599), "2024-12-31T23:59:59Z");
    }

    #[test]
    fn formats_dates_outside_chronos_range() {
        // Year 10000 is still inside chrono's range: a plain zero-padded year, no leading `+`.
        assert_eq!(iso_from_unix_secs(253_402_300_800), "10000-01-01T00:00:00Z");
        // Past chrono's range, keep the proleptic Gregorian date instead of the Unix epoch.
        assert_eq!(iso_from_unix_secs(10_000_000_000_000), "318857-05-20T17:46:40Z");
    }

    #[test]
    fn civil_from_days_is_exact_beyond_chronos_range() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_734), (2026, 10, 8));
        // Past `NaiveDate`'s ±262143 the 400-year cycles carry the year; values from the
        // pre-chrono formula, both directions, both sides of the epoch.
        assert_eq!(civil_from_days(95_026_602), (262_144, 1, 1));
        assert_eq!(civil_from_days(99_000_000), (273_022, 10, 18));
        assert_eq!(civil_from_days(-96_465_964), (-262_145, 3, 1));
        assert_eq!(civil_from_days(-99_000_000), (-269_083, 3, 17));
        // The extremes must not overflow or panic (i128 intermediates).
        let _ = (civil_from_days(i64::MAX), civil_from_days(i64::MIN));
    }
}
