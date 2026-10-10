//! Word-processor font resolution: map a requested family + bold/italic to an available face,
//! with metric-compatible substitutes for common proprietary document fonts, and Word-style line
//! metrics (Windows ascent/descent, which is what Word uses for single line spacing).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use skrifa::raw::TableProvider;

use crate::{FaceRef, FontDb};

/// A requested font resolved to a face.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub face: FaceRef,
    /// The face isn't bold but bold was asked for: embolden when drawing.
    pub synth_bold: bool,
    /// The face isn't italic but italic was asked for: slant when drawing.
    pub synth_italic: bool,
    /// The requested family wasn't available.
    pub substituted: bool,
}

/// Substitutes to try, in order, for a family that isn't installed. Open fonts that are
/// metric-compatible come first (Carlito ↔ Calibri, Caladea ↔ Cambria, Liberation ↔ Arial /
/// Times New Roman / Courier New), then common system fonts, then our bundled families.
fn substitutes(family: &str) -> &'static [&'static str] {
    const SANS: &[&str] = &["Carlito", "Helvetica Neue", "Arial", "Liberation Sans", "Noto Sans", "DejaVu Sans", "Source Sans 3"];
    const SERIF: &[&str] = &["Caladea", "Times New Roman", "Liberation Serif", "Georgia", "Noto Serif", "DejaVu Serif", "Source Serif 4"];
    const MONO: &[&str] = &["Courier New", "Liberation Mono", "Menlo", "Consolas", "DejaVu Sans Mono", "JetBrains Mono"];
    const ARIAL: &[&str] = &["Liberation Sans", "Helvetica", "Helvetica Neue", "Arimo", "Source Sans 3"];
    const TIMES: &[&str] = &["Liberation Serif", "Times", "Tinos", "Source Serif 4"];
    const SYMBOL: &[&str] = &["Apple Symbols", "Segoe UI Symbol", "DejaVu Sans", "Noto Sans Symbols", "Source Sans 3"];
    // Persian / Arabic document fonts. Naskh (book) faces such as B Nazanin, B Lotus, B Zar and
    // Traditional Arabic, then the sans faces (B Yekan, B Titr, IRANSans) people use for
    // headings and screens. Open fonts with full Persian coverage first.
    const NASKH: &[&str] = &["Noto Naskh Arabic", "XB Zar", "XB Niloofar", "Amiri", "Vazirmatn", "Noto Sans Arabic", "DejaVu Sans", "Source Serif 4"];
    const ARABIC_SANS: &[&str] = &["Vazirmatn", "Sahel", "Shabnam", "Samim", "Noto Sans Arabic", "Noto Naskh Arabic", "DejaVu Sans", "Source Sans 3"];
    let f = family.to_ascii_lowercase();
    if is_persian_naskh(&f) {
        return NASKH;
    }
    if is_persian_sans(&f) {
        return ARABIC_SANS;
    }
    match f.as_str() {
        "arial" | "helvetica" | "arial nova" => ARIAL,
        "times new roman" | "times" => TIMES,
        "courier new" | "courier" | "consolas" | "lucida console" | "cascadia code" | "cascadia mono" => MONO,
        "symbol" | "wingdings" | "segoe ui symbol" | "webdings" => SYMBOL,
        "cambria"
        | "cambria math"
        | "georgia"
        | "garamond"
        | "book antiqua"
        | "palatino linotype"
        | "constantia"
        | "baskerville old face"
        | "century"
        | "century schoolbook"
        | "bookman old style"
        | "sitka text"
        | "sitka"
        | "aptos serif" => SERIF,
        _ if f.contains("mono") || f.contains("code") => MONO,
        _ if f.contains("serif") && !f.contains("sans") => SERIF,
        _ => SANS,
    }
}

/// Naskh-style Persian/Arabic document fonts (the "B …" family names come from the common
/// Persian font packs; matched by name only).
fn is_persian_naskh(f: &str) -> bool {
    const NAMES: &[&str] = &[
        "b nazanin",
        "b lotus",
        "b zar",
        "b mitra",
        "b badr",
        "b yagut",
        "b roya",
        "b compset",
        "b nazanin bold",
        "b lotus bold",
        "nazanin",
        "lotus",
        "zar",
        "mitra",
        "badr",
        "yagut",
        "roya",
        "ir nazanin",
        "ir lotus",
        "ir zar",
        "ir mitra",
        "ir badr",
        "irnazanin",
        "irlotus",
        "irzar",
        "irmitra",
        "irbadr",
        "traditional arabic",
        "simplified arabic",
        "arabic typesetting",
        "sakkal majalla",
        "adobe arabic",
        "andalus",
        "times new roman (arabic)",
    ];
    NAMES.contains(&f)
}

/// Sans Persian/Arabic fonts for headings and screens.
fn is_persian_sans(f: &str) -> bool {
    const NAMES: &[&str] = &[
        "b yekan",
        "b titr",
        "b homa",
        "b koodak",
        "b traffic",
        "b jadid",
        "b elham",
        "b kamran",
        "b davat",
        "b ferdosi",
        "yekan",
        "titr",
        "homa",
        "koodak",
        "traffic",
        "ir titr",
        "ir homa",
        "irtitr",
        "irhoma",
        "iransans",
        "iransansweb",
        "iran sans",
        "iranyekan",
        "iran yekan",
        "iranyekanweb",
        "dana",
        "estedad",
        "dubai",
        "aldhabi",
        "urdu typesetting",
        "arabic transparent",
    ];
    NAMES.contains(&f) || f.starts_with("iransans") || f.starts_with("iranyekan")
}

fn style_name(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    }
}

/// Resolve `family` with bold/italic to a face (memoised).
pub fn resolve(family: &str, bold: bool, italic: bool) -> Resolved {
    static CACHE: OnceLock<Mutex<HashMap<(String, bool, bool), Resolved>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (family.to_string(), bold, italic);
    if let Some(r) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *r;
    }
    let db = FontDb::global();
    let (fam, substituted) = if db.has_family(family) {
        (family.to_string(), false)
    } else {
        let s = substitutes(family).iter().find(|s| db.has_family(s)).copied().unwrap_or(crate::FALLBACK_FAMILY);
        (s.to_string(), true)
    };
    let face = db.face(&fam, style_name(bold, italic));
    let r = Resolved { synth_bold: bold && face.weight < 600.0, synth_italic: italic && !face.italic, substituted, face: FaceRef::of(&face) };
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, r);
    r
}

/// The standard Unicode character for a Symbol or Wingdings character stored the way Word stores
/// it, as the private-use code U+F000 + the font's own code (U+F0B7 is the Symbol bullet). Used
/// when that font isn't installed: a substitute draws something else, or nothing, at the
/// private-use code. `None` for other fonts and for codes not in the table.
pub fn symbol_font_char(family: &str, c: char) -> Option<char> {
    let code = u32::from(c).checked_sub(0xF000)?;
    let u = match (family.to_ascii_lowercase().as_str(), code) {
        // Adobe Symbol encoding.
        ("symbol", 0xA7) => '\u{2663}',
        ("symbol", 0xA8) => '\u{2666}',
        ("symbol", 0xA9) => '\u{2665}',
        ("symbol", 0xAA) => '\u{2660}',
        ("symbol", 0xAE) => '\u{2192}',
        ("symbol", 0xB7) => '\u{2022}',
        ("symbol", 0xD7) => '\u{22C5}',
        ("symbol", 0xE0) => '\u{25CA}',
        // Wingdings characters common as list bullets.
        ("wingdings", 0x6C) => '\u{25CF}',
        ("wingdings", 0x6E) => '\u{25A0}',
        ("wingdings", 0x76) => '\u{2756}',
        ("wingdings", 0xA7) => '\u{25AA}',
        ("wingdings", 0xD8) => '\u{27A2}',
        ("wingdings", 0xFC) => '\u{2714}',
        _ => return None,
    };
    Some(u)
}

/// Line metrics for a face in font units: (ascent, descent), both positive, the way word
/// processors measure single line spacing (OS/2 usWinAscent/usWinDescent, falling back to the
/// hhea values plus line gap).
pub fn line_metrics(face: &crate::FontFace) -> (f64, f64) {
    static CACHE: OnceLock<Mutex<HashMap<u32, (f64, f64)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&face.id()) {
        return *v;
    }
    let mut v = (face.ascent.max(1.0), face.descent.abs());
    if let Some(f) = face.skrifa() {
        // hhea ascender + descender + line gap (Word on macOS); OS/2 win metrics only when the
        // hhea values are missing, and never more than twice the em.
        let hhea = f.hhea().ok().map(|h| {
            let gap = h.line_gap().to_i16().max(0) as f64;
            (h.ascender().to_i16() as f64 + gap / 2.0, (h.descender().to_i16() as f64).abs() + gap / 2.0)
        });
        let win = f.os2().ok().map(|o| (o.us_win_ascent() as f64, o.us_win_descent() as f64));
        let ok = |m: &(f64, f64)| m.0 > 0.0 && m.0 + m.1 <= face.upem * 2.0;
        if let Some(m) = hhea.filter(ok) {
            v = m;
        } else if let Some(m) = win.filter(ok) {
            v = m;
        }
    }
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(face.id(), v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_family_substitutes() {
        let r = resolve("Definitely Not A Font 123", false, false);
        assert!(r.substituted);
        let b = resolve("Source Serif 4", true, false);
        assert!(!b.substituted);
        assert!(!b.synth_bold);
        let (a, d) = line_metrics(&b.face);
        assert!(a > 0.0 && d >= 0.0);
    }

    #[test]
    fn symbol_font_chars_map_to_unicode() {
        assert_eq!(symbol_font_char("Symbol", '\u{F0B7}'), Some('\u{2022}'));
        assert_eq!(symbol_font_char("symbol", '\u{F0B7}'), Some('\u{2022}'));
        assert_eq!(symbol_font_char("Wingdings", '\u{F0A7}'), Some('\u{25AA}'));
        assert_eq!(symbol_font_char("Wingdings", '\u{F0D8}'), Some('\u{27A2}'));
        // Same code, other font; unknown codes; ordinary characters.
        assert_eq!(symbol_font_char("Arial", '\u{F0B7}'), None);
        assert_eq!(symbol_font_char("Symbol", '\u{F041}'), None);
        assert_eq!(symbol_font_char("Symbol", 'a'), None);
        assert_eq!(symbol_font_char("Symbol", '\u{10FFFF}'), None);
    }

    #[test]
    fn synthesizes_missing_styles() {
        // JetBrains Mono ships only Regular here.
        let r = resolve("JetBrains Mono", true, true);
        assert!(r.synth_bold && r.synth_italic);
    }
}
