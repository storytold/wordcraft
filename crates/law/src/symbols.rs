//! Legal symbols: insert commands, spacing after §/¶, and typing AutoFormat.

use std::sync::Once;

use serde_json::{Value, json};
use wordcraft_doc::para::NBSP;
use wordcraft_doc::{Paragraph, Path, Pos, StoryRef};
use wordcraft_engine::cmd::{delete_selection, sel_result, type_text};
use wordcraft_engine::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

const LOC: &str = "Law › Symbols";

/// One entry in the legal symbols palette (ribbon when LawCraft is on).
pub struct PaletteEntry {
    pub command: &'static str,
    pub label: &'static str,
    pub glyph: &'static str,
}

/// Legal symbols shown in the palette (in order). ⸹ is omitted when the UI font lacks it.
pub const PALETTE: &[PaletteEntry] = &[
    PaletteEntry { command: "law.symbols.section", label: "Section Sign", glyph: "§" },
    PaletteEntry { command: "law.symbols.sections", label: "Section Signs", glyph: "§§" },
    PaletteEntry { command: "law.symbols.paragraph", label: "Paragraph Sign", glyph: "¶" },
    PaletteEntry { command: "law.symbols.paragraphs", label: "Paragraph Signs", glyph: "¶¶" },
    PaletteEntry { command: "law.symbols.copyright", label: "Copyright Sign", glyph: "©" },
    PaletteEntry { command: "law.symbols.registered", label: "Registered Sign", glyph: "®" },
    PaletteEntry { command: "law.symbols.trademark", label: "Trademark Sign", glyph: "™" },
    PaletteEntry { command: "law.symbols.serviceMark", label: "Service Mark", glyph: "℠" },
    PaletteEntry { command: "law.symbols.dagger", label: "Dagger", glyph: "†" },
    PaletteEntry { command: "law.symbols.doubleDagger", label: "Double Dagger", glyph: "‡" },
    PaletteEntry { command: "law.symbols.referenceMark", label: "Reference Mark", glyph: "⸹" },
    PaletteEntry { command: "law.symbols.enDash", label: "En Dash", glyph: "–" },
    PaletteEntry { command: "law.symbols.emDash", label: "Em Dash", glyph: "—" },
    PaletteEntry { command: "law.symbols.nbsp", label: "Nonbreaking Space", glyph: "\u{00a0}" },
    PaletteEntry { command: "law.symbols.nbHyphen", label: "Nonbreaking Hyphen", glyph: "\u{2011}" },
];

/// Optional reference mark (U+2E39); hosts may hide it when the interface font has no glyph.
pub const REFERENCE_MARK: char = '\u{2e39}';

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("law.symbols.section", "Section Sign", LOC, section).key("Mod+Alt+4"),
        CommandSpec::new("law.symbols.sections", "Section Signs", LOC, sections).key("Mod+Alt+Shift+4"),
        CommandSpec::new("law.symbols.paragraph", "Paragraph Sign", LOC, paragraph).key("Mod+Alt+6"),
        CommandSpec::new("law.symbols.paragraphs", "Paragraph Signs", LOC, paragraphs).key("Mod+Alt+Shift+6"),
        CommandSpec::new("law.symbols.copyright", "Copyright Sign", LOC, copyright),
        CommandSpec::new("law.symbols.registered", "Registered Sign", LOC, registered),
        CommandSpec::new("law.symbols.trademark", "Trademark Sign", LOC, trademark),
        CommandSpec::new("law.symbols.serviceMark", "Service Mark", LOC, service_mark),
        CommandSpec::new("law.symbols.dagger", "Dagger", LOC, dagger),
        CommandSpec::new("law.symbols.doubleDagger", "Double Dagger", LOC, double_dagger),
        CommandSpec::new("law.symbols.referenceMark", "Reference Mark", LOC, reference_mark),
        CommandSpec::new("law.symbols.enDash", "En Dash", LOC, en_dash),
        CommandSpec::new("law.symbols.emDash", "Em Dash", LOC, em_dash),
        CommandSpec::new("law.symbols.nbsp", "Nonbreaking Space", LOC, nbsp),
        CommandSpec::new("law.symbols.nbHyphen", "Nonbreaking Hyphen", LOC, nb_hyphen),
        CommandSpec::new("law.symbols.fixSpacing", "Fix Legal Sign Spacing", LOC, fix_spacing)
            .params(r#"{"scope"?: "selection" | "document"} — replace an ordinary space after § or ¶ before a digit with a non-breaking space; default is the selection, or the whole document when the selection is empty"#),
    ]
}

fn section(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "§")
}
fn sections(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "§§")
}
fn paragraph(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "¶")
}
fn paragraphs(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "¶¶")
}
fn copyright(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "©")
}
fn registered(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "®")
}
fn trademark(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "™")
}
fn service_mark(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "℠")
}
fn dagger(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "†")
}
fn double_dagger(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "‡")
}
fn reference_mark(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "⸹")
}
fn en_dash(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "–")
}
fn em_dash(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "—")
}
fn nbsp(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "\u{00a0}")
}
fn nb_hyphen(s: &mut Session, _: &Value) -> CmdResult {
    insert_text(s, "\u{2011}")
}

/// The Law tab's groups: the legal symbols, in palette order.
pub fn ribbon_groups() -> Vec<wordcraft_engine::ribbon::CustomGroup> {
    vec![wordcraft_engine::ribbon::CustomGroup { name: "Legal Symbols".into(), commands: PALETTE.iter().map(|e| e.command.to_string()).collect() }]
}

/// Register the typing AutoFormat hook once per process.
pub fn register_text_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| wordcraft_engine::hooks::register_on_text_autoformat(on_text_autoformat));
}

fn insert_text(s: &mut Session, text: &str) -> CmdResult {
    let _ = delete_selection(s)?;
    type_text(s, text)?;
    if needs_nbsp_after_signs(text) {
        maybe_append_sign_nbsp(s)?;
    }
    sel_result(s)
}

fn needs_nbsp_after_signs(text: &str) -> bool {
    text.chars().last().is_some_and(|c| c == '§' || c == '¶')
}

fn maybe_append_sign_nbsp(s: &mut Session) -> Result<(), CmdError> {
    let f = s.sel.focus.clone();
    // What sits at the caret right after the sign: a digit (no space yet), or an ordinary space
    // with a digit after it (the text already had " 1.010" and the sign was inserted before it).
    let (digit_after, space_before_digit) = {
        let Some(para) = s.doc.para_at(&f) else { return Ok(()) };
        let rest = para.text.get(f.off..).unwrap_or("");
        let mut it = rest.chars();
        match it.next() {
            Some(c) if c.is_ascii_digit() => (true, false),
            Some(' ') if it.next().is_some_and(|c| c.is_ascii_digit()) => (false, true),
            _ => (false, false),
        }
    };
    if digit_after {
        let mut b = [0u8; 4];
        type_text(s, NBSP.encode_utf8(&mut b))?;
    } else if space_before_digit {
        // Replace the existing ordinary space with an NBSP, keeping its character formatting so
        // the sign and number still read as one unbreakable unit. The command's checkpoint makes
        // the whole insertion one undo step.
        let space_start = f.off;
        let space_end = space_start + 1;
        if s.doc.settings.track_changes {
            // As a tracked change, like the sign itself: Reject puts the ordinary space back.
            let end = wordcraft_engine::cmd::replace_text(s, &Pos { off: space_start, ..f.clone() }, &Pos { off: space_end, ..f.clone() }, NBSP_STR)?;
            s.sel = Selection::caret(end);
            return Ok(());
        }
        let para = s.doc.para_mut(f.story, &f.path)?;
        let props = para.props_of_char(space_start).clone();
        para.delete(space_start, space_end)?;
        let mut b = [0u8; 4];
        para.insert_text(space_start, NBSP.encode_utf8(&mut b), &props)?;
        para.touch();
        // Leave the caret after the wider NBSP, not inside it.
        s.sel = Selection::caret(Pos { off: space_start + NBSP.len_utf8(), ..f });
    }
    Ok(())
}

/// `§ ` before a digit → `§` + NBSP + digit (same for ¶ and doubled signs).
pub fn fix_sign_spacing_in_str(text: &str) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '§' || c == '¶' {
            let sign = c;
            let mut j = i;
            while j < chars.len() && chars[j] == sign {
                j += 1;
            }
            if j < chars.len() && chars[j] == ' ' && chars.get(j + 1).is_some_and(|d| d.is_ascii_digit()) {
                out.extend(chars[i..j].iter());
                out.push(NBSP);
                changed = true;
                i = j + 1;
                continue;
            }
            // No replacement: emit the whole run and skip it in one step. Advancing one character
            // would rescan the run's tail for every sign, which is quadratic (a long run could
            // freeze the document command).
            out.extend(chars[i..j].iter());
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    (out, changed)
}

/// Byte ranges `[start, end)` of ASCII spaces after §/¶ runs that should become NBSP, wholly inside `text[from..to]`.
pub fn sign_spacing_space_ranges(text: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let from = from.min(text.len());
    let to = to.min(text.len());
    if from >= to {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = from;
    while i < to {
        let Some(first) = text.get(i..).and_then(|s| s.chars().next()) else { break };
        if first == '§' || first == '¶' {
            let sign = first;
            let sign_start = i;
            let mut j = i + first.len_utf8();
            while let Some(c) = text.get(j..).and_then(|s| s.chars().next())
                && c == sign
            {
                j += c.len_utf8();
            }
            if j < text.len()
                && text.get(j..j + 1) == Some(" ")
                && text.get(j + 1..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_digit())
            {
                let space_start = j;
                let space_end = j + 1;
                let digit = text[space_end..].chars().next().unwrap_or('\0');
                let digit_end = space_end + digit.len_utf8();
                if sign_start >= from && digit_end <= to {
                    out.push((space_start, space_end));
                }
                i = digit_end;
                continue;
            }
            // No replacement: skip the whole scanned run. Advancing one character would rescan
            // the run's tail for every sign, which is quadratic (a long run could freeze the
            // document command).
            i = j;
            continue;
        }
        i += first.len_utf8();
    }
    out
}

fn fix_sign_spacing_in_para(para: &mut Paragraph, from: usize, to: usize) -> Result<Vec<(usize, usize)>, wordcraft_doc::DocError> {
    let spaces = sign_spacing_space_ranges(&para.text, from, to);
    if spaces.is_empty() {
        return Ok(Vec::new());
    }
    for (space_start, space_end) in spaces.iter().rev() {
        let props = para.props_of_char(*space_start).clone();
        para.delete(*space_start, *space_end)?;
        let mut buf = [0u8; 4];
        para.insert_text(*space_start, NBSP.encode_utf8(&mut buf), &props)?;
    }
    para.touch();
    Ok(spaces)
}

/// Move a selection endpoint over each replaced space. Each `(start, end)` is a one-byte ASCII
/// space that became a longer NBSP, so every offset after `start` grows by the difference. The
/// anchor and focus are mapped independently, which preserves the selection's direction. A
/// `Path` alone is not enough: the body and a header, footer or note can share it, so the edited
/// story must match too or a caret in another story drifts.
fn map_selection_through(sel: &mut Selection, story: StoryRef, path: &Path, replaced: &[(usize, usize)]) {
    let shift = |off: usize| off + replaced.iter().filter(|(start, _)| *start < off).count();
    if sel.anchor.story == story && sel.anchor.path == *path {
        sel.anchor.off = shift(sel.anchor.off);
    }
    if sel.focus.story == story && sel.focus.path == *path {
        sel.focus.off = shift(sel.focus.off);
    }
}

fn fix_spacing(s: &mut Session, v: &Value) -> CmdResult {
    let scope = p::str(v, "scope");
    let whole = scope == Some("document") || (scope != Some("selection") && s.sel.is_collapsed());
    let (a, b) = s.sel.ordered();
    if s.doc.settings.track_changes {
        return fix_spacing_tracked(s, whole, &a, &b);
    }
    let mut fixes = 0usize;
    if whole {
        // Every text story, not just the body: footnotes, endnotes, headers, footers and text
        // boxes carry legal signs too.
        for story in s.doc.text_stories() {
            for path in s.doc.para_paths(story) {
                let para = s.doc.para_mut(story, &path)?;
                let len = para.len();
                let replaced = fix_sign_spacing_in_para(para, 0, len)?;
                if !replaced.is_empty() {
                    fixes += 1;
                    map_selection_through(&mut s.sel, story, &path, &replaced);
                }
            }
        }
    } else {
        let story = a.story;
        for path in s.doc.paths_between(&a, &b) {
            let para = s.doc.para_mut(story, &path)?;
            let from = if path == a.path { a.off } else { 0 };
            let to = if path == b.path { b.off } else { para.len() };
            if from >= to {
                continue;
            }
            let replaced = fix_sign_spacing_in_para(para, from, to)?;
            if !replaced.is_empty() {
                fixes += 1;
                map_selection_through(&mut s.sel, story, &path, &replaced);
            }
        }
    }
    Ok(json!({"fixes": fixes}))
}

/// [`fix_spacing`] with Track Changes on: each space is replaced as a tracked change (the space
/// marked deleted, the non-breaking space marked inserted), so Reject All puts the spaces back.
fn fix_spacing_tracked(s: &mut Session, whole: bool, a: &Pos, b: &Pos) -> CmdResult {
    let mut work: Vec<(StoryRef, Path, Vec<(usize, usize)>)> = Vec::new();
    let stories = if whole { s.doc.text_stories() } else { vec![a.story] };
    for story in stories {
        let paths = if whole { s.doc.para_paths(story) } else { s.doc.paths_between(a, b) };
        for path in paths {
            let Some(para) = s.doc.para(story, &path) else { continue };
            let (from, to) =
                if whole { (0, para.len()) } else { (if path == a.path { a.off } else { 0 }, if path == b.path { b.off } else { para.len() }) };
            if from >= to {
                continue;
            }
            let ranges = sign_spacing_space_ranges(&para.text, from, to);
            if !ranges.is_empty() {
                work.push((story, path, ranges));
            }
        }
    }
    let fixes = work.len();
    for (story, path, ranges) in work {
        for (start, end) in ranges.into_iter().rev() {
            let before = s.doc.para(story, &path).map_or(0, |p| p.len());
            wordcraft_engine::cmd::replace_text(
                s,
                &Pos { story, path: path.clone(), off: start },
                &Pos { story, path: path.clone(), off: end },
                NBSP_STR,
            )?;
            let delta = s.doc.para(story, &path).map_or(0, |p| p.len()) as isize - before as isize;
            // Offsets after the replaced space move by what the edit added.
            for p in [&mut s.sel.anchor, &mut s.sel.focus] {
                if p.story == story && p.path == path && p.off > start {
                    p.off = (p.off as isize + delta).max(start as isize) as usize;
                }
            }
        }
    }
    s.touch();
    Ok(json!({"fixes": fixes}))
}

/// A non-breaking space as text.
const NBSP_STR: &str = "\u{a0}";

/// After a digit is typed, turn `§ 1` into `§` NBSP `1`.
fn on_text_autoformat(s: &mut Session, text: &str) -> Result<bool, CmdError> {
    if !s.module_enabled(crate::MODULE) || !s.prefs.autocorrect.enabled {
        return Ok(false);
    }
    let ch = text.chars().next().unwrap_or('\0');
    if !ch.is_ascii_digit() {
        return Ok(false);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f).cloned() else { return Ok(false) };
    let off = f.off;
    if off == 0 {
        return Ok(false);
    }
    let bytes = para.text.as_bytes();
    let digit_start = off.saturating_sub(ch.len_utf8());
    if digit_start >= bytes.len() {
        return Ok(false);
    }
    let has_ascii_space = digit_start > 0 && para.text.get(digit_start - 1..digit_start) == Some(" ");
    let sign_run = if has_ascii_space {
        let before_space = para.text.get(..digit_start - 1).unwrap_or("");
        sign_run_end(before_space)
    } else {
        sign_run_end(para.text.get(..digit_start).unwrap_or(""))
    };
    if sign_run == 0 {
        return Ok(false);
    }
    if s.doc.settings.track_changes {
        // As a tracked change, like the rest of what was typed: Reject puts the space back.
        let (from, to) = if has_ascii_space { (digit_start - 1, digit_start) } else { (digit_start, digit_start) };
        let end = wordcraft_engine::cmd::replace_text(s, &Pos { off: from, ..f.clone() }, &Pos { off: to, ..f.clone() }, NBSP_STR)?;
        s.sel = Selection::caret(Pos { off: end.off + ch.len_utf8(), ..f });
        return Ok(true);
    }
    let para = s.doc.para_mut(f.story, &f.path)?;
    let mut buf = [0u8; 4];
    let nbsp = NBSP.encode_utf8(&mut buf);
    let delta = if has_ascii_space {
        let space_start = digit_start - 1;
        let props = para.props_of_char(space_start).clone();
        para.delete(space_start, digit_start)?;
        para.insert_text(space_start, nbsp, &props)?;
        NBSP.len_utf8() - 1
    } else {
        let props = para.props_of_char(digit_start).clone();
        para.insert_text(digit_start, nbsp, &props)?;
        NBSP.len_utf8()
    };
    s.sel = Selection::caret(Pos { off: f.off + delta, ..f });
    Ok(true)
}

fn sign_run_end(before_space: &str) -> usize {
    let mut n = 0usize;
    for c in before_space.chars().rev() {
        if c == '§' || c == '¶' {
            n += c.len_utf8();
        } else {
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_engine::Session;

    use crate::{MODULE, install};

    fn law() -> Session {
        let mut s = Session::new(Document::new());
        s.prefs.modules.insert(MODULE.into(), true);
        install(&mut s);
        s
    }

    #[test]
    fn fix_sign_spacing_in_string() {
        let (t, ch) = fix_sign_spacing_in_str("§ 1.010");
        assert!(ch);
        assert_eq!(t, format!("§{NBSP}1.010"));
        let (t, ch) = fix_sign_spacing_in_str("§§ 12");
        assert!(ch);
        assert_eq!(t, format!("§§{NBSP}12"));
        let (t, ch) = fix_sign_spacing_in_str("sec. 1");
        assert!(!ch);
        assert_eq!(t, "sec. 1");
    }

    #[test]
    fn typed_section_spacing_undo_restores_normal_space() {
        register_text_hook();
        let mut s = law();
        s.run("text.insert", &json!({"text": "§"})).unwrap();
        s.run("text.insert", &json!({"text": " "})).unwrap();
        s.run("text.insert", &json!({"text": "1"})).unwrap();
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert_eq!(text, format!("§{NBSP}1"));
        run(&mut s, "text.insert", json!({"text": "0"}));
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert_eq!(text, format!("§{NBSP}10"));
        assert_eq!(s.undo_label(), Some("Typing"));
        assert!(s.undo());
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert_eq!(text, format!("§{NBSP}1"));
        assert_eq!(s.undo_label(), Some("AutoFormat"));
        assert!(s.undo());
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert_eq!(text, "§ 1", "undo restores the ordinary space: {text:?}");
        assert!(s.redo());
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert_eq!(text, format!("§{NBSP}1"), "redo re-applies the AutoFormat: {text:?}");
        assert!(s.redo());
        let text = s.doc.para_at(&s.sel.focus).unwrap().text.clone();
        assert_eq!(text, format!("§{NBSP}10"), "redo replays the rest of the typing: {text:?}");
    }

    fn run(s: &mut Session, id: &str, v: serde_json::Value) {
        s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}
