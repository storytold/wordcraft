//! Typing and deleting.

use serde_json::Value;
use wordcraft_doc::para::{COLUMN_BREAK, NB_HYPHEN, NBSP, OBJ, PAGE_BREAK, SOFT_HYPHEN};
use wordcraft_doc::props::{Border, BorderStyle, NumRef};
use wordcraft_doc::{Block, ListKind, Path, Pos, StoryRef, Table};

use super::{delete_selection, non_mac, sel_result, split_para, type_text};
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
        // Alt+X on Windows and Linux; macOS has no default (Option+X types a character there).
        CommandSpec::new("text.toggleUnicode", "Toggle Unicode Character", "Editing", toggle_unicode).key(if cfg!(target_os = "macos") {
            ""
        } else {
            "Alt+X"
        }),
        // Alt+numpad codes: the shell collects the keypad digits while Alt is held (#961).
        CommandSpec::new("text.typeCode", "Type Character Code", "Editing", insert_code)
            .params(r#"{"code": string (decimal digits as typed on the keypad, e.g. "0233"), "ansi"?: 874 | 1250–1258 (the leading-0 code page, default 1252), "oem"?: 437 | 737 | 850 | 852 | 857 | 866 (default from the locale)}"#),
        CommandSpec::new("text.emDash", "Em Dash", "Insert › Symbols", |s, _| ins_char(s, '—')).key("Mod+Alt+-"),
        // On macOS Option+G / R / 2 / ; type these directly, so Word for Mac has no Cmd shortcut.
        CommandSpec::new("text.copyright", "Copyright Sign", "Insert › Symbols", |s, _| ins_char(s, '©')).key(non_mac("Mod+Alt+C")),
        CommandSpec::new("text.registered", "Registered Sign", "Insert › Symbols", |s, _| ins_char(s, '®')).key(non_mac("Mod+Alt+R")),
        CommandSpec::new("text.trademark", "Trademark Sign", "Insert › Symbols", |s, _| ins_char(s, '™')).key(non_mac("Mod+Alt+T")),
        CommandSpec::new("text.ellipsis", "Ellipsis", "Insert › Symbols", |s, _| ins_char(s, '…')).key(non_mac("Mod+Alt+.")),
    ]
}

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
    let opening = prev.is_none_or(|p| p.is_whitespace() || "([{<\u{2014}\u{2013}".contains(p));
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
fn typing_options(s: &mut Session, text: &str) {
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

fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?;
    let raw = p::bool(v, "raw").unwrap_or(false);
    typing_options(s, text);
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
    let listed = text == " " && s.prefs.autocorrect.enabled && list_autoformat(s)?;
    if !listed {
        super::tools::autocorrect(s)?;
    }
    if text == " " && !listed && s.prefs.autocorrect.enabled && s.prefs.autocorrect.dashes {
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

/// "* " / "- " / "> " → bullets, "1. " / "a) " → numbering, typed at the start of a paragraph.
fn list_autoformat(s: &mut Session) -> Result<bool, CmdError> {
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return Ok(false) };
    if para.props.numbering.is_some_and(|n| n.num != 0) || f.off != para.len() {
        return Ok(false);
    }
    let kind = match para.text.as_str() {
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
        && before.trim_end()[..i].chars().next_back().is_some_and(|c| c != ' ')
    {
        let p = s.doc.para_mut(f.story, &f.path)?;
        let props = p.props_at(i).clone();
        p.delete(i, i + 2)?;
        p.insert_text(i, "\u{2014}", &props)?;
        s.sel = Selection::caret(Pos { off: f.off + 1, ..f });
    }
    Ok(())
}

/// An AutoFormat that Enter applies to the paragraph just finished.
enum EnterAutoFormat {
    /// "---" and friends: a bottom border.
    Border(Border),
    /// "+---+-----+": a one-row table; the dash count of each column.
    Table(Vec<usize>),
}

/// AutoFormat border lines: Enter after "---", "___", "===", "***", "~~~" or "###".
fn border_line(s: &Session, at: &Pos) -> Option<Border> {
    let para = s.doc.para_at(at)?;
    let text = para.text.as_str();
    let c = text.chars().next()?;
    if !s.prefs.autocorrect.enabled
        || !s.prefs.autocorrect.border_lines
        || at.off != text.len()
        || text.len() < 3
        || !text.chars().all(|x| x == c)
        || para.props.numbering.is_some_and(|n| n.num != 0)
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
/// of hyphens). Returns the hyphen count of each column; more columns than a table can hold
/// leave the text alone.
fn table_line(s: &Session, at: &Pos) -> Option<Vec<usize>> {
    let para = s.doc.para_at(at)?;
    let text = para.text.as_str();
    if !s.prefs.autocorrect.enabled
        || at.story != StoryRef::Body
        || at.path.0.len() != 1
        || at.off != text.len()
        || para.props.numbering.is_some_and(|n| n.num != 0)
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

/// Turn the border-line characters into a bottom border on the paragraph above (or this one
/// at the top of a story).
fn apply_border_line(s: &mut Session, at: &Pos, line: Border) -> Result<(), CmdError> {
    let set_bottom = |p: &mut wordcraft_doc::Paragraph| {
        let mut b = p.props.borders.unwrap_or_default();
        b.bottom = Some(line);
        p.props.borders = Some(b);
        p.touch();
    };
    let i = at.path.last();
    let prev = if i > 0 { Some(at.path.with_last(i - 1)) } else { None };
    let cur = s.doc.para_mut(at.story, &at.path)?;
    let len = cur.len();
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
    Ok(())
}

/// Replace the "+---+" characters with a one-row table across the text width, each column as
/// wide as its share of the hyphens; the emptied paragraph stays after the table and the caret
/// goes into the first cell.
fn apply_table_line(s: &mut Session, at: &Pos, cols: &[usize]) -> Result<(), CmdError> {
    let total = cols.iter().sum::<usize>().max(1) as f32;
    let block = at.path.0.first().copied().unwrap_or(0) as usize;
    let width = s.doc.sections().get(s.doc.section_index_of(block)).map(|(_, sp)| sp.text_width()).unwrap_or(468.0).max(36.0);
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
    let p = s.doc.para_mut(at.story, &at.path)?;
    let len = p.len();
    p.delete(0, len)?;
    s.doc.insert_block(at.story, &at.path, Block::Table(t))?;
    let mut first = at.path.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: at.story, path: Path(first), off: 0 });
    Ok(())
}

/// Toggle Unicode Character (Alt+X): the hex code before the caret (up to six digits, `U+`
/// optional) or the selected code becomes its character; otherwise the character before the
/// caret (or the one selected) becomes its code (`é` ↔ `00E9`). Formatting is kept; one undo step.
fn toggle_unicode(s: &mut Session, _: &Value) -> CmdResult {
    let (a, b) = s.sel.ordered();
    if a.story != b.story || a.path != b.path {
        return Err(CmdError::Failed("select a character code in one paragraph".into()));
    }
    let Some(para) = s.doc.para_at(&b) else { return Err(CmdError::Failed("no paragraph at the caret".into())) };
    let bad = || CmdError::Failed("the caret isn't on a character boundary".into());
    let (start, replacement) = if a != b {
        let sel = para.text.get(a.off..b.off).ok_or_else(bad)?;
        let code = sel.strip_prefix("U+").or_else(|| sel.strip_prefix("u+")).unwrap_or(sel);
        if let Some(c) = hex_char(code) {
            (a.off, c.to_string())
        } else {
            let mut it = sel.chars();
            match (it.next(), it.next()) {
                (Some(c), None) if codeable(c) => (a.off, code_of(c)),
                _ => return Err(CmdError::Failed("select a character code (like 00E9) or one character".into())),
            }
        }
    } else {
        let before = para.text.get(..b.off).ok_or_else(bad)?;
        // The hex digits right before the caret (at most six, nearest first); the longest that
        // names a character wins.
        let digits: Vec<usize> = before.char_indices().rev().take_while(|(_, c)| c.is_ascii_hexdigit()).take(6).map(|(i, _)| i).collect();
        let found = digits.iter().rev().find_map(|at| {
            let c = hex_char(before.get(*at..)?)?;
            let prefixed = before.get(..*at).is_some_and(|x| x.ends_with("U+") || x.ends_with("u+"));
            Some((if prefixed { at - 2 } else { *at }, c.to_string()))
        });
        match (found, before.chars().next_back()) {
            (Some(f), _) => f,
            (None, Some(c)) if codeable(c) => (b.off - c.len_utf8(), code_of(c)),
            _ => return Err(CmdError::Failed("no character or code before the caret".into())),
        }
    };
    s.sel = Selection { anchor: Pos { off: start, ..b.clone() }, focus: b };
    // The replacement keeps the replaced text's formatting.
    s.pending = None;
    type_text(s, &replacement)?;
    sel_result(s)
}

/// Alt+numpad entry: type the character a decimal code names at the caret, as typed text (one
/// undo step, AutoFormat as for a typed character). `ansi` and `oem` are the system's code pages
/// (Windows-1252 and the locale's OEM page by default). The result's `char` is the character.
fn insert_code(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::encoding::{OEM_CODE_PAGES, SINGLE_BYTE_CODE_PAGES};
    let code = p::req_str(v, "code")?;
    let page = |key: &str, known: &[u16], default: u16| match p::u64(v, key) {
        Some(cp) => u16::try_from(cp).ok().filter(|cp| known.contains(cp)).ok_or_else(|| CmdError::Params(format!("{key} must be one of {known:?}"))),
        None => Ok(default),
    };
    let ansi = page("ansi", &SINGLE_BYTE_CODE_PAGES, 1252)?;
    let oem = page("oem", &OEM_CODE_PAGES, system_oem_code_page())?;
    let c = alt_code_char(code, ansi, oem).ok_or_else(|| CmdError::Failed(format!("{code:?} isn't a character code")))?;
    let mut r = insert(s, &serde_json::json!({"text": c.to_string()}))?;
    if let Value::Object(m) = &mut r {
        m.insert("char".into(), Value::String(c.to_string()));
    }
    Ok(r)
}

/// The character Word on Windows types for an Alt+numpad code (decimal digits, as typed):
/// - with a leading 0 and a value up to 255, the ANSI code page's character (`ansi`, one of
///   [`wordcraft_doc::encoding::SINGLE_BYTE_CODE_PAGES`]): `0233` → é and `0128` → € in
///   Windows-1252, `0185` → ą in 1250;
/// - without one, 1–255 is the OEM code page's character (`oem`, one of
///   [`wordcraft_doc::encoding::OEM_CODE_PAGES`]): `233` → Θ in 437, Ú in 850 (é is `130` in
///   both), `165` → ą in 852; 1–31 and 127 are the code page's glyphs (`1` → ☺, `7` → •);
/// - above 255, with or without the 0, the Unicode character with that decimal value
///   (`8364` → €), as Word inserts it.
///
/// `None` for an empty or non-numeric code, 0, a control character, a byte the code page leaves
/// unassigned or a value past Unicode.
pub fn alt_code_char(code: &str, ansi: u16, oem: u16) -> Option<char> {
    if code.is_empty() || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = code.trim_start_matches('0');
    // More than seven significant digits is past U+10FFFF (1114111).
    if digits.len() > 7 {
        return None;
    }
    let value: u32 = if digits.is_empty() { 0 } else { digits.parse().ok()? };
    let c = match u8::try_from(value) {
        Ok(0) => return None,
        Ok(b) if code.starts_with('0') => wordcraft_doc::encoding::windows_byte(ansi, b),
        Ok(b) => wordcraft_doc::encoding::oem_byte(oem, b),
        Err(_) => char::from_u32(value)?,
    };
    Some(c).filter(|c| codeable(*c) && *c != '\u{FFFD}')
}

/// The OEM code page for the system locale the shell reported (437 headless).
pub fn system_oem_code_page() -> u16 {
    oem_code_page(crate::session::system_locale())
}

/// The (ANSI, OEM) code pages of the system locale the shell reported, as Windows sets them
/// ([`windows_code_pages`]).
pub fn system_code_pages() -> (u16, u16) {
    windows_code_pages(crate::session::system_locale())
}

/// The OEM code page `text.typeCode` assumes for a locale (BCP 47 tag) when the shell names
/// none: 850 for Western European languages and English outside the US, 437 otherwise. The
/// other pages come only from the shell (Windows passes [`windows_code_pages`]).
pub fn oem_code_page(locale: Option<&str>) -> u16 {
    match windows_code_pages(locale) {
        (_, 850) => 850,
        _ => 437,
    }
}

/// The (ANSI, OEM) code pages Windows uses for a locale (BCP 47 tag or POSIX name), limited to
/// the pages `text.typeCode` decodes: Windows-1252 and OEM 437 for US English, unknown locales
/// and those with other pages (Asian double-byte pages, OEM 775/720/862…).
pub fn windows_code_pages(locale: Option<&str>) -> (u16, u16) {
    let Some(tag) = locale else { return (1252, 437) };
    let parts: Vec<&str> = tag.split(['-', '_', '.', '@']).collect();
    let lang = parts.first().copied().unwrap_or_default().to_ascii_lowercase();
    let rest = parts.get(1..).unwrap_or_default();
    let region = rest.iter().find(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic())).map(|r| r.to_ascii_uppercase());
    let script = rest.iter().find(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic())).map(|r| r.to_ascii_lowercase());
    const WESTERN: [&str; 18] = ["ca", "da", "de", "es", "eu", "fi", "fo", "fr", "ga", "gl", "is", "it", "nb", "nl", "nn", "no", "pt", "sv"];
    match lang.as_str() {
        "en" => match region.as_deref() {
            None | Some("US") => (1252, 437),
            Some(_) => (1252, 850),
        },
        l if WESTERN.contains(&l) => (1252, 850),
        "pl" | "cs" | "sk" | "hu" | "sl" | "hr" | "ro" | "sq" | "bs" => (1250, 852),
        "sr" if script.as_deref() == Some("latn") => (1250, 852),
        "ru" | "uk" | "be" | "bg" => (1251, 866),
        // Macedonian and Cyrillic Serbian use OEM 855, which isn't decoded.
        "sr" | "mk" => (1251, 437),
        "el" => (1253, 737),
        "az" if script.as_deref() == Some("cyrl") => (1251, 866),
        "tr" | "az" => (1254, 857),
        "he" => (1255, 437),
        "ar" | "fa" | "ur" => (1256, 437),
        "et" | "lv" | "lt" => (1257, 437),
        "vi" => (1258, 437),
        "th" => (874, 437),
        _ => (1252, 437),
    }
}

/// The character a hex code (1–6 digits) names, if it is one Alt+X makes.
fn hex_char(code: &str) -> Option<char> {
    if code.is_empty() || code.len() > 6 || !code.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(code, 16).ok().and_then(char::from_u32).filter(|c| codeable(*c))
}

/// Characters Alt+X converts (not control characters, breaks or object placeholders).
fn codeable(c: char) -> bool {
    !c.is_control() && c != wordcraft_doc::para::OBJ
}

/// A character's code as Word writes it: at least four upper-case hex digits.
fn code_of(c: char) -> String {
    format!("{:04X}", u32::from(c))
}

fn ins_char(s: &mut Session, c: char) -> CmdResult {
    let mut b = [0u8; 4];
    type_text(s, c.encode_utf8(&mut b))?;
    sel_result(s)
}

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
fn fold_at_end(s: &Session) -> bool {
    let f = &s.sel.focus;
    f.story == StoryRef::Body
        && f.path.0.len() == 1
        && s.doc.para_at(f).is_some_and(|p| f.off >= p.len())
        && super::outline::folds(s, f.path.last())
        && super::outline::folded(s).get(f.path.last().saturating_add(1)).copied().unwrap_or(false)
}

fn new_paragraph(s: &mut Session, _: &Value) -> CmdResult {
    // Enter in a one-line plain text control (or a check box or list) does nothing, as in Word.
    if super::controls::refuses_paragraph(s) {
        return sel_result(s);
    }
    super::controls::prepare_edit(s)?;
    if let Some(text) = sample_text(s) {
        let end = s.sel.focus.clone();
        s.sel = Selection { anchor: Pos { off: 0, ..end.clone() }, focus: end };
        type_text(s, &text)?;
        return sel_result(s);
    }
    // Enter at the end of a collapsed heading only expands it (Word for Mac, #1745).
    if s.sel.is_collapsed() && fold_at_end(s) {
        super::outline::expand_heading(s, s.sel.focus.path.last());
        return sel_result(s);
    }
    let mut at = delete_selection(s)?;
    let auto = border_line(s, &at).map(EnterAutoFormat::Border).or_else(|| table_line(s, &at).map(EnterAutoFormat::Table));
    if let Some(auto) = auto {
        // Word: Undo right after reverts only the AutoFormat, so the typed characters come back
        // and the Enter stays. Record that plain Enter as its own undo step.
        let (doc, sel) = (s.doc.clone(), s.sel.clone());
        let new = split_para(s, &at)?;
        s.sel = Selection::caret(new);
        let plain_doc = std::mem::replace(&mut s.doc, doc);
        let plain_sel = std::mem::replace(&mut s.sel, sel);
        match auto {
            EnterAutoFormat::Border(line) => apply_border_line(s, &at, line)?,
            EnterAutoFormat::Table(cols) => apply_table_line(s, &at, &cols)?,
        }
        s.goal_x = None;
        s.push_undo("AutoFormat", plain_doc, plain_sel);
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

fn backspace(s: &mut Session, v: &Value) -> CmdResult {
    let r = backspace_inner(s, v)?;
    super::controls::restore_placeholder(s)?;
    Ok(r)
}

fn backspace_inner(s: &mut Session, _: &Value) -> CmdResult {
    s.goal_x = None;
    s.pending = None;
    if !s.sel.is_collapsed() {
        delete_selection(s)?;
        return sel_result(s);
    }
    let f = s.sel.focus.clone();
    let Some(para) = s.doc.para_at(&f) else { return sel_result(s) };
    if f.off > 0 {
        let a = Pos { off: para.backspace_from(f.off), ..f.clone() };
        s.sel = Selection { anchor: a, focus: f };
        delete_selection(s)?;
        return sel_result(s);
    }
    // At the start of a paragraph.
    // Just after a fold, the first press only expands it (Word for Mac, #1745).
    if f.story == StoryRef::Body
        && f.path.0.len() == 1
        && let Some(prev) = f.path.last().checked_sub(1)
        && super::outline::folded(s).get(prev).copied().unwrap_or(false)
    {
        super::outline::show_block(s, prev);
        return sel_result(s);
    }
    if para.props.numbering.is_some_and(|n| n.num != 0) {
        // First Backspace removes the number but keeps the item's indent (Word): the text stays
        // where it was, so the indent becomes the paragraph's own (`num: 0` alone drops it).
        let text_at = para.props.numbering.and_then(|n| s.doc.numbering.level(n.num, n.level)).map(|l| l.indent);
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
        return sel_result(s);
    }
    if para.props.style.as_deref() == Some("ListParagraph") {
        // The second one takes it out of the list's indent instead of joining paragraphs.
        super::leave_list(s.doc.para_mut(f.story, &f.path)?);
        return sel_result(s);
    }
    if para.props.indent_first.is_some_and(|x| x > 0.0) || para.props.indent_first_chars.is_some_and(|x| x > 0.0) {
        let p = s.doc.para_mut(f.story, &f.path)?;
        p.props.indent_first = Some(0.0);
        p.props.cancel_char_indents(false, false, true);
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

fn delete(s: &mut Session, v: &Value) -> CmdResult {
    let r = delete_inner(s, v)?;
    super::controls::restore_placeholder(s)?;
    Ok(r)
}

fn delete_inner(s: &mut Session, _: &Value) -> CmdResult {
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
    // At the end of a collapsed heading, the first press only expands it (Word for Mac, #1745).
    if fold_at_end(s) {
        super::outline::expand_heading(s, f.path.last());
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
    super::controls::restore_placeholder(s)?;
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
    super::controls::restore_placeholder(s)?;
    sel_result(s)
}
