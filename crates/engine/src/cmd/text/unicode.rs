use super::autoformat::insert;
use super::*;

/// Toggle Unicode Character (Alt+X): the hex code before the caret (up to six digits, `U+`
/// optional) or the selected code becomes its character; otherwise the character before the
/// caret (or the one selected) becomes its code (`é` ↔ `00E9`). Formatting is kept; one undo step.
pub(super) fn toggle_unicode(s: &mut Session, _: &Value) -> CmdResult {
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
pub(super) fn insert_code(s: &mut Session, v: &Value) -> CmdResult {
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

pub(super) fn ins_char(s: &mut Session, c: char) -> CmdResult {
    let mut b = [0u8; 4];
    type_text(s, c.encode_utf8(&mut b))?;
    sel_result(s)
}
