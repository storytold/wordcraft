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
    substitutes_by_name(family).0
}

/// Does `family` have substitutes of its own, picked by its name (a metric-compatible clone such
/// as Liberation Serif for Times New Roman, or the same design under another name), rather than
/// the generic lists any unknown font gets?
pub fn has_named_substitutes(family: &str) -> bool {
    if family.contains(';') {
        return list_class(family).named;
    }
    substitutes_by_name(charset_base_family(family)).1
}

const SANS: &[&str] = &["Carlito", "Helvetica Neue", "Arial", "Liberation Sans", "Noto Sans", "DejaVu Sans", "Source Sans 3"];

/// [`substitutes`], and whether they were chosen for this family by name.
fn substitutes_by_name(family: &str) -> (&'static [&'static str], bool) {
    if family.contains(';') {
        for cand in family.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (subs, named) = substitutes_by_name(cand);
            if named || is_thai_family(cand) || subs != SANS {
                return (subs, named);
            }
        }
        if let Some(first) = family.split(';').map(str::trim).find(|s| !s.is_empty()) {
            return substitutes_by_name(first);
        }
    }
    const SERIF: &[&str] = &["Caladea", "Times New Roman", "Liberation Serif", "Georgia", "Noto Serif", "DejaVu Serif", "Source Serif 4"];
    const MONO: &[&str] = &["Courier New", "Liberation Mono", "Menlo", "Consolas", "DejaVu Sans Mono", "JetBrains Mono"];
    const ARIAL: &[&str] = &["Liberation Sans", "Helvetica", "Helvetica Neue", "Arimo", "Source Sans 3"];
    const ARIAL_UNICODE: &[&str] = &["Arial", "Liberation Sans", "Helvetica", "Helvetica Neue", "Arimo", "Source Sans 3"];
    const TIMES: &[&str] = &["Liberation Serif", "Times", "Tinos", "Source Serif 4"];
    // Word's symbol fonts (Symbol, Wingdings…) are proprietary: their characters are drawn as their
    // Unicode equivalents (`wordcraft_doc::symbol_fonts`) from these faces or any face that has them.
    const SYMBOL: &[&str] = &["Apple Symbols", "Segoe UI Symbol", "DejaVu Sans", "Noto Sans Symbols", "Noto Sans Symbols 2", "Source Sans 3"];
    // Persian / Arabic document fonts. Naskh (book) faces such as B Nazanin, B Lotus, B Zar and
    // Traditional Arabic, then the sans faces (B Yekan, B Titr, IRANSans) people use for
    // headings and screens. Open fonts with full Persian coverage first.
    const NASKH: &[&str] = &["Noto Naskh Arabic", "XB Zar", "XB Niloofar", "Amiri", "Vazirmatn", "Noto Sans Arabic", "DejaVu Sans", "Source Serif 4"];
    const ARABIC_SANS: &[&str] = &["Vazirmatn", "Sahel", "Shabnam", "Samim", "Noto Sans Arabic", "Noto Naskh Arabic", "DejaVu Sans", "Source Sans 3"];
    // Hebrew document fonts: platform-specific fonts first, then common open source fallbacks.
    // Windows: Microsoft Office fonts (David, Miriam) are common; Segoe UI is standard.
    // macOS: Helvetica has good Hebrew support; Geeza Pro is included by default.
    // Linux: Noto Sans Hebrew is the best option.
    #[cfg(windows)]
    const HEBREW: &[&str] = &["Noto Sans Hebrew", "David", "Miriam", "Segoe UI", "Arial", "Tahoma", "DejaVu Sans", "Source Sans 3"];
    #[cfg(target_os = "macos")]
    const HEBREW: &[&str] = &["Helvetica", "Noto Sans Hebrew", "Arial", "Tahoma", "Geeza Pro", "DejaVu Sans", "Source Sans 3"];
    #[cfg(not(any(windows, target_os = "macos")))]
    const HEBREW: &[&str] = &["Noto Sans Hebrew", "Segoe UI", "Arial", "Tahoma", "DejaVu Sans", "Source Sans 3"];
    // Thai document fonts (TH Sarabun New, Cordia New, the UPC families…): Thai faces, every one
    // with Thai letters; [`substitute`] prefers one with Latin letters and digits too, so a run
    // mixing Thai and Latin keeps one font at one size, as Word substitutes Cordia New for the
    // whole run (#1549). Windows' own Thai fonts first, then the Mac's, then the open Noto and Thai
    // Linux Working Group families. The Mac's Thonburi and Sathu carry Apple's `morx` tables
    // instead of OpenType ones; the shaper applies those, so their marks stack as they should.
    #[cfg(windows)]
    const THAI: &[&str] = &["Cordia New", "Leelawadee UI", "Leelawadee", "Tahoma", "Noto Sans Thai", "Browallia New", "Angsana New"];
    #[cfg(target_os = "macos")]
    const THAI: &[&str] = &["Thonburi", "Ayuthaya", "Sathu", "Krungthep", "Noto Sans Thai", "Tahoma"];
    #[cfg(not(any(windows, target_os = "macos")))]
    const THAI: &[&str] = &["Noto Sans Thai", "Laksaman", "Garuda", "Waree", "Tlwg Typo", "Loma", "Umpush", "Noto Serif Thai"];
    // Serif Thai document fonts (Angsana New, EucrosiaUPC, Norasi…) prefer a serif Thai face.
    #[cfg(windows)]
    const THAI_SERIF: &[&str] = &["Angsana New", "AngsanaUPC", "Noto Serif Thai", "Cordia New", "Leelawadee UI", "Tahoma", "Noto Sans Thai"];
    #[cfg(target_os = "macos")]
    const THAI_SERIF: &[&str] = &["Sathu", "Noto Serif Thai", "Thonburi", "Ayuthaya", "Krungthep", "Noto Sans Thai", "Tahoma"];
    #[cfg(not(any(windows, target_os = "macos")))]
    const THAI_SERIF: &[&str] = &["Noto Serif Thai", "Norasi", "Kinnari", "Noto Sans Thai", "Laksaman", "Garuda", "Waree"];
    // Khmer document fonts (Khmer OS …, Khmer Mool1…): a missing one is drawn in the system's
    // Khmer font, as Word does, not in whatever Khmer font the document happens to load: the Khmer
    // OS fonts are wider, so lines wrap and boxes overflow where Word's don't.
    #[cfg(windows)]
    const KHMER: &[&str] = &["Leelawadee UI", "Khmer UI", "DaunPenh", "MoolBoran", "Noto Sans Khmer"];
    #[cfg(target_os = "macos")]
    const KHMER: &[&str] = &["Khmer Sangam MN", "Khmer MN", "Noto Sans Khmer"];
    #[cfg(not(any(windows, target_os = "macos")))]
    const KHMER: &[&str] = &["Noto Sans Khmer", "Noto Serif Khmer"];
    // Palatino Linotype / Book Antiqua: macOS ships the same design as "Palatino"; open
    // look-alikes with matching metrics (TeX Gyre Pagella, URW P052 / Palladio) on Linux.
    const PALATINO: &[&str] = &[
        "Palatino Linotype",
        "Palatino",
        "Book Antiqua",
        "TeX Gyre Pagella",
        "P052",
        "URW Palladio L",
        "Palladio",
        "Caladea",
        "Times New Roman",
        "Liberation Serif",
        "Georgia",
        "Noto Serif",
        "DejaVu Serif",
        "Source Serif 4",
    ];
    let f = family.to_ascii_lowercase();
    if is_thai_serif(&f) {
        return (THAI_SERIF, true);
    }
    if is_thai(&f) {
        return (THAI, true);
    }
    if is_hebrew(&f) {
        return (HEBREW, true);
    }
    if is_khmer(&f) {
        return (KHMER, true);
    }
    if is_persian_naskh(&f) {
        return (NASKH, true);
    }
    if is_persian_sans(&f) {
        return (ARABIC_SANS, true);
    }
    match f.as_str() {
        "palatino linotype" | "book antiqua" | "palatino" => (PALATINO, true),
        "arial" | "helvetica" | "arial nova" => (ARIAL, true),
        "arial unicode ms" => (ARIAL_UNICODE, true),
        "times new roman" | "times" => (TIMES, true),
        "courier new" | "courier" | "consolas" | "lucida console" | "cascadia code" | "cascadia mono" => (MONO, true),
        "symbol" | "wingdings" | "wingdings 2" | "wingdings 3" | "segoe ui symbol" | "webdings" => (SYMBOL, true),
        // Serif faces with no open clone of their own: the generic serif list, as for any font.
        "cambria"
        | "cambria math"
        | "georgia"
        | "garamond"
        | "constantia"
        | "baskerville old face"
        | "century"
        | "century schoolbook"
        | "bookman old style"
        | "sitka text"
        | "sitka"
        | "aptos serif" => (SERIF, false),
        _ if f.contains("mono") || f.contains("code") => (MONO, false),
        _ if f.contains("serif") && !f.contains("sans") => (SERIF, false),
        _ => (SANS, false),
    }
}

/// Ascent and descent (em) of Word's symbol fonts, which aren't open fonts. Their characters are
/// drawn from other faces (`wordcraft_doc::symbol_fonts`), but Word sizes a line by the font
/// itself: a Symbol bullet makes its line taller than the text alone. The values are those Word's
/// PDFs give the fonts (`/Ascent`, `/Descent`).
pub fn symbol_font_metrics(family: &str) -> Option<(f64, f64)> {
    family.split(';').map(str::trim).filter(|f| !f.is_empty()).find_map(|f| match f.to_ascii_lowercase().as_str() {
        "symbol" | "symbol (pcl6)" => Some((1.005, 0.216)),
        "wingdings" => Some((0.899, 0.205)),
        _ => None,
    })
}

/// The (ascent, descent) in em Word gives a line with a run in a symbol font, when that isn't the
/// resolved face's own: the font isn't installed (`resolved` is a substitute), or the installed
/// face is one em tall, as Apple's Symbol is (hhea 0.701 / -0.299, no gap). Word for Mac spaces
/// such lines like Windows' Symbol: 1.222 em at 11 and 20 pt on synthetic documents (#2681).
pub fn symbol_line(family: &str, resolved: &Resolved) -> Option<(f64, f64)> {
    let m = symbol_font_metrics(family)?;
    (resolved.substituted || one_em(line_metrics(&resolved.face), resolved.face.upem)).then_some(m)
}

/// Is a line's (ascent, descent) one `upem` em tall?
fn one_em((ascent, descent): (f64, f64), upem: f64) -> bool {
    upem > 0.0 && ((ascent + descent) / upem - 1.0).abs() < 0.005
}

/// The font a Windows 3.1-era charset name stands for: `Times New Roman Cyr`, `Arial CE`,
/// `Courier New Baltic`, `Arial (Hebrew)`… are aliases Windows keeps (its FontSubstitutes) for
/// the font itself, and old Russian and Central European documents name them. Other names are
/// returned unchanged.
pub fn charset_base_family(family: &str) -> &str {
    const SUFFIXES: &[&str] = &[" cyr", " ce", " greek", " tur", " baltic", " (arabic)", " (hebrew)", " (vietnamese)"];
    let lower = family.trim_end().to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find(|s| lower.ends_with(*s) && lower.len() > s.len())
        .and_then(|s| family.trim_end().get(..lower.len() - s.len()))
        .map_or(family, str::trim_end)
}

/// Fonts made to measure alike (the same advance widths and line height), so text drawn in one
/// wraps and paginates as in the others: Carlito for Calibri, Caladea for Cambria, Liberation and
/// Croscore fonts for Arial, Times New Roman and Courier New, the Palatino look-alikes for Palatino
/// Linotype and Book Antiqua, and Arial and Helvetica (drawn to Helvetica's widths) for one
/// another. The one list of them: a missing font is drawn in its first installed twin
/// ([`substitute`]), and text drawn in a twin doesn't change the layout ([`metric_compatible`]).
/// Not macOS Times or Courier: their advance widths differ from Times New Roman's and Courier
/// New's.
const METRIC_TWINS: &[&[&str]] = &[
    &["Arial", "Liberation Sans", "Helvetica", "Arimo"],
    &["Times New Roman", "Liberation Serif", "Tinos"],
    &["Courier New", "Liberation Mono", "Cousine"],
    &["Calibri", "Carlito"],
    &["Cambria", "Caladea"],
    &["Palatino Linotype", "Palatino", "Book Antiqua", "TeX Gyre Pagella", "P052", "URW Palladio L", "Palladio"],
];

/// The fonts that measure like `family` ([`METRIC_TWINS`]), in the order they are tried, without
/// `family` itself.
fn metric_twins(family: &str) -> impl Iterator<Item = &'static str> + '_ {
    let f = family.trim();
    METRIC_TWINS
        .iter()
        .filter(move |g| g.iter().any(|t| t.eq_ignore_ascii_case(f)))
        .flat_map(|g| g.iter().copied())
        .filter(move |t| !t.eq_ignore_ascii_case(f))
}

/// Whether `substitute` was made to measure like `family` ([`METRIC_TWINS`]), so text drawn in it
/// wraps and paginates as in `family`.
pub fn metric_compatible(family: &str, substitute: &str) -> bool {
    let s = substitute.trim();
    family.split(';').map(str::trim).filter(|f| !f.is_empty()).any(|f| {
        let base = charset_base_family(f);
        base.eq_ignore_ascii_case(s) || metric_twins(base).any(|t| t.eq_ignore_ascii_case(s))
    })
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

/// Thai document fonts: Word's Thai families (Cordia New, Angsana New, Browallia New, the UPC
/// families, Leelawadee), the Thai national fonts (TH Sarabun New and the other "TH …" fonts),
/// the Mac's, the Thai Linux Working Group's and Noto's. Matched by name only.
fn is_thai(f: &str) -> bool {
    const NAMES: &[&str] = &[
        "cordia new",
        "cordiaupc",
        "angsana new",
        "angsanaupc",
        "browallia new",
        "browalliaupc",
        "leelawadee",
        "leelawadee ui",
        "leelawadee ui semilight",
        "th sarabun new",
        "th sarabunpsk",
        "th sarabun psk",
        "th sarabun",
        "thonburi",
        "ayuthaya",
        "sathu",
        "krungthep",
        "silom",
        "sukhumvit set",
        "norasi",
        "kinnari",
        "garuda",
        "laksaman",
        "loma",
        "purisa",
        "sawasdee",
        "umpush",
        "waree",
        "tlwg typo",
        "tlwg typist",
        "tlwg mono",
    ];
    NAMES.contains(&f) || f.ends_with("upc") || f.starts_with("th ") || f.contains("thai")
}

/// Serif Thai document fonts.
fn is_thai_serif(f: &str) -> bool {
    const NAMES: &[&str] = &["angsana new", "angsanaupc", "eucrosiaupc", "jasmineupc", "kodchiangupc", "dilleniaupc", "norasi", "kinnari"];
    NAMES.contains(&f) || (f.contains("thai") && f.contains("serif"))
}

/// Is `family` a Thai document font ([`is_thai`])? A missing one is drawn in a Thai face for the
/// whole run, never in a Latin substitute (#1549).
pub fn is_thai_family(family: &str) -> bool {
    if family.contains(';') {
        return list_class(family).thai;
    }
    let f = family.trim().to_ascii_lowercase();
    is_thai(&f) || is_thai_serif(&f)
}

/// What the name-based predicates answer about one semicolon-separated fallback list. Shaping asks
/// them for every piece of every run set in the list, so the answer is worked out once per list
/// rather than once per piece.
#[derive(Clone, Copy, Default)]
struct ListClass {
    /// [`is_thai_family`].
    thai: bool,
    /// [`has_named_substitutes`].
    named: bool,
}

/// [`ListClass`] of the semicolon-separated `family`, memoised by its text.
fn list_class(family: &str) -> ListClass {
    let key = family_list_key(family);
    let generation = FontDb::global().generation();
    {
        let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.generation != generation {
            *c = FallbackListMemo { generation, ..Default::default() };
        } else if let Some((cached, class)) = c.classes.get(&key)
            && cached == family
        {
            return *class;
        }
    }
    let class = ListClass { thai: classify_thai(family), named: classify_named(family) };
    let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.generation == generation {
        if c.classes.len() >= FAMILY_CACHE_MAX {
            c.classes.clear();
        }
        c.classes.insert(key, (family.to_string(), class));
    }
    class
}

/// Whether any family of the list is a Thai document font: what [`is_thai_family`] answers.
fn classify_thai(family: &str) -> bool {
    family.split(';').map(str::trim).filter(|f| !f.is_empty()).any(|f| {
        let f = f.to_ascii_lowercase();
        is_thai(&f) || is_thai_serif(&f)
    })
}

/// Whether any family of the list has substitutes of its own: what [`has_named_substitutes`]
/// answers.
fn classify_named(family: &str) -> bool {
    family.split(';').map(str::trim).filter(|f| !f.is_empty()).any(|f| substitutes_by_name(charset_base_family(f)).1)
}

/// Khmer document fonts: the Khmer OS family, Khmer Mool1 and the like, Windows' and the Mac's
/// own and Noto's. Matched by name only.
fn is_khmer(f: &str) -> bool {
    f.starts_with("khmer")
        || f.contains("khmer")
        || matches!(f, "daunpenh" | "moolboran" | "limon" | "battambang" | "bokor" | "content" | "dangrek" | "angkor")
}

/// Hebrew document fonts.
fn is_hebrew(f: &str) -> bool {
    const NAMES: &[&str] = &["david", "miriam", "noto sans hebrew", "arial hebrew", "calibri hebrew"];
    NAMES.contains(&f) || f.contains("hebrew")
}

/// The first installed substitute for a missing `family`. For a Thai font, the first with Latin
/// letters and digits as well as Thai ones (a Thai face without Latin would leave a mixed run in
/// two fonts, #1549), else the first with Thai letters, else the first installed.
fn substitute(db: &FontDb, family: &str) -> Option<&'static str> {
    // A twin draws the text as the font would: Carlito for Calibri, Helvetica for Arial on a Mac.
    if let Some(twin) = metric_twins(family).find(|t| db.has_family(t)) {
        return Some(twin);
    }
    let mut installed = substitutes(family).iter().copied().filter(|s| db.has_family(s));
    if !is_thai_family(family) {
        return installed.next();
    }
    let installed: Vec<&'static str> = installed.collect();
    let covers = |name: &str, chars: &[char]| {
        let face = db.face(name, "Regular");
        face.named(name) && chars.iter().all(|c| face.covers(*c))
    };
    installed
        .iter()
        .copied()
        .find(|s| covers(s, &['\u{0E19}', 'A', 'z', '0', '9']))
        .or_else(|| installed.iter().copied().find(|s| covers(s, &['\u{0E19}'])))
        .or_else(|| installed.first().copied())
}

fn style_name(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    }
}

type Memo = (u64, HashMap<(String, bool, bool), Resolved>);

fn resolve_cache() -> &'static Mutex<Memo> {
    static CACHE: OnceLock<Mutex<Memo>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new((0, HashMap::new())))
}

const FAMILY_CACHE_MAX: usize = 4096;
/// Family names longer than this (bytes) are resolved but never remembered as string keys.
const FAMILY_KEY_MAX: usize = 256;
/// Maximum number of direct `FontDb::has_family` catalog scans per uncached fallback list
/// for candidate names not present in [`FontDb::families`].
const MAX_DIRECT_CANDIDATE_PROBES: usize = 32;

/// Memoised fallback-list resolutions and installed-family lookups for the current [`FontDb::generation`].
#[derive(Default)]
struct FallbackListMemo {
    generation: u64,
    families: std::collections::HashSet<String>,
    probed: HashMap<String, bool>,
    lists: HashMap<(u64, usize), (String, (String, bool))>,
    long_resolved: HashMap<(u64, usize, bool, bool), (String, Resolved)>,
    classes: HashMap<(u64, usize), (String, ListClass)>,
}

/// Shared cache for fallback-list resolutions and candidate availability checks.
fn fallback_list_cache() -> &'static Mutex<FallbackListMemo> {
    static CACHE: OnceLock<Mutex<FallbackListMemo>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(FallbackListMemo::default()))
}

/// Hash and byte length of `family`, used as a compact cache key for fallback lists.
fn family_list_key(family: &str) -> (u64, usize) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    family.hash(&mut h);
    (h.finish(), family.len())
}

/// Whether `cand` is an installed family in `db`, using the memoised family set and bounded
/// direct catalog probes so long fallback lists do not rescan the font catalog for every candidate.
fn candidate_has_family(db: &FontDb, cand: &str, direct_probes: &mut usize) -> bool {
    let cand = cand.trim();
    if cand.is_empty() || cand.len() > FAMILY_KEY_MAX {
        return false;
    }
    let generation = db.generation();
    let lower = cand.to_ascii_lowercase();
    {
        let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.generation != generation {
            *c = FallbackListMemo { generation, ..Default::default() };
        }
        if c.families.is_empty() {
            c.families = db.families().into_iter().map(|f| f.to_ascii_lowercase()).collect();
        }
        if c.families.contains(&lower) {
            return true;
        }
        if let Some(&hit) = c.probed.get(&lower) {
            return hit;
        }
    }
    if *direct_probes == 0 {
        return false;
    }
    *direct_probes -= 1;
    let hit = db.has_family(cand);
    let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.generation == generation {
        if c.probed.len() >= FAMILY_CACHE_MAX {
            c.probed.clear();
        }
        c.probed.insert(lower, hit);
    }
    hit
}

/// Record a resolved `(family, substituted)` pair for a semicolon-separated fallback list.
fn remember_list_resolution(generation: u64, key: (u64, usize), family: &str, result: (String, bool)) -> (String, bool) {
    let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.generation == generation {
        if c.lists.len() >= FAMILY_CACHE_MAX {
            c.lists.clear();
        }
        c.lists.insert(key, (family.to_string(), result.clone()));
    }
    result
}

/// Resolve a single family or semicolon-separated fallback list to `(family_name, substituted)`.
fn resolve_family(db: &FontDb, family: &str) -> (String, bool) {
    if !family.contains(';') {
        let base = if family.trim().eq_ignore_ascii_case("Symbol (PCL6)") { "Symbol" } else { charset_base_family(family) };
        return if db.has_family(family) {
            (family.to_string(), false)
        } else if base != family && db.has_family(base) {
            (base.to_string(), false)
        } else {
            (substitute(db, base).unwrap_or(crate::FALLBACK_FAMILY).to_string(), true)
        };
    }

    let generation = db.generation();
    let list_key = family_list_key(family);
    {
        let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.generation != generation {
            *c = FallbackListMemo { generation, ..Default::default() };
        } else if let Some((cached_family, r)) = c.lists.get(&list_key)
            && cached_family == family
        {
            return r.clone();
        }
    }

    let candidates: Vec<&str> = family.split(';').map(str::trim).filter(|s| !s.is_empty() && s.len() <= FAMILY_KEY_MAX).collect();
    if candidates.is_empty() {
        return remember_list_resolution(generation, list_key, family, (crate::FALLBACK_FAMILY.to_string(), true));
    }

    let mut direct_probes = MAX_DIRECT_CANDIDATE_PROBES;

    // 1. First installed family in the list:
    for &cand in &candidates {
        let base = if cand.eq_ignore_ascii_case("Symbol (PCL6)") { "Symbol" } else { charset_base_family(cand) };
        if candidate_has_family(db, cand, &mut direct_probes) {
            return remember_list_resolution(generation, list_key, family, (cand.to_string(), false));
        }
        if base != cand && candidate_has_family(db, base, &mut direct_probes) {
            return remember_list_resolution(generation, list_key, family, (base.to_string(), false));
        }
    }

    // 2. None installed: fall back to an installed substitute for an entry in the list rather than generic Carlito.
    // Try Thai substitutes before metric twins of later candidates:
    for &cand in &candidates {
        let base = if cand.eq_ignore_ascii_case("Symbol (PCL6)") { "Symbol" } else { charset_base_family(cand) };
        if is_thai_family(base)
            && let Some(sub) = substitute(db, base)
        {
            return remember_list_resolution(generation, list_key, family, (sub.to_string(), true));
        }
        let mut twin_probes = MAX_DIRECT_CANDIDATE_PROBES;
        if let Some(twin) = metric_twins(base).find(|t| candidate_has_family(db, t, &mut twin_probes)) {
            return remember_list_resolution(generation, list_key, family, (twin.to_string(), true));
        }
    }

    // Try candidates with named or category substitutes (Thai, Hebrew, Khmer, Persian, serif, mono):
    for &cand in &candidates {
        let base = if cand.eq_ignore_ascii_case("Symbol (PCL6)") { "Symbol" } else { charset_base_family(cand) };
        let (subs, named) = substitutes_by_name(base);
        if (named || is_thai_family(base) || subs != SANS)
            && let Some(sub) = substitute(db, base)
        {
            return remember_list_resolution(generation, list_key, family, (sub.to_string(), true));
        }
    }

    // Fall back to the first candidate's substitute:
    if let Some(&first) = candidates.first() {
        let base = if first.eq_ignore_ascii_case("Symbol (PCL6)") { "Symbol" } else { charset_base_family(first) };
        if let Some(sub) = substitute(db, base) {
            return remember_list_resolution(generation, list_key, family, (sub.to_string(), true));
        }
    }

    remember_list_resolution(generation, list_key, family, (crate::FALLBACK_FAMILY.to_string(), true))
}

/// Resolve `family` with bold/italic to a face (memoised until user fonts are added or removed,
/// [`FontDb::generation`]).
pub fn resolve(family: &str, bold: bool, italic: bool) -> Resolved {
    let cache = resolve_cache();
    let db = FontDb::global();
    let generation = db.generation();
    let cacheable = family.len() <= FAMILY_KEY_MAX;
    let key = (if cacheable { family.to_string() } else { String::new() }, bold, italic);
    let long_key = (!cacheable && family.contains(';')).then(|| {
        let (h, len) = family_list_key(family);
        (h, len, bold, italic)
    });
    {
        let mut c = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.0 != generation {
            *c = (generation, HashMap::new());
        }
        if cacheable && let Some(r) = c.1.get(&key) {
            return *r;
        }
    }
    if let Some(lk) = long_key {
        let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.generation != generation {
            *c = FallbackListMemo { generation, ..Default::default() };
        } else if let Some((cached_family, r)) = c.long_resolved.get(&lk)
            && cached_family == family
        {
            return *r;
        }
    }
    let (fam, substituted) = resolve_family(db, family);
    let face = db.face(&fam, style_name(bold, italic));
    let r = Resolved { synth_bold: bold && face.weight < 600.0, synth_italic: italic && !face.italic, substituted, face: FaceRef::of(&face) };
    if cacheable {
        let mut c = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.0 == generation {
            if c.1.len() >= FAMILY_CACHE_MAX {
                c.1.clear();
            }
            c.1.insert(key, r);
        }
    } else if let Some(lk) = long_key {
        let mut c = fallback_list_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.generation == generation {
            if c.long_resolved.len() >= FAMILY_CACHE_MAX {
                c.long_resolved.clear();
            }
            c.long_resolved.insert(lk, (family.to_string(), r));
        }
    }
    r
}

/// A line's (ascent, descent) from hhea values, both positive: the line gap goes above the ascent.
fn hhea_line(ascender: i16, descender: i16, line_gap: i16) -> (f64, f64) {
    (f64::from(ascender) + f64::from(line_gap.max(0)), f64::from(descender).abs())
}

/// The line (ascent, descent) in units of an `upem` em that Word gives the classic PostScript
/// families: their Windows counterparts' spacing, though Apple's own Times and Courier are one em
/// tall (hhea 0.75 / -0.25, no gap). Measured in Word for Mac on synthetic documents (single
/// spacing, 11 and 20 pt): Times lines lie exactly as far apart as Times New Roman's (hhea 1825 /
/// -443, gap 87 per 2048), Courier's as Courier New's (1705 / -615, no gap). On Windows, Word
/// draws a "Times" document in Times New Roman anyway.
fn borrowed_line(family: &str, upem: f64) -> Option<(f64, f64)> {
    let (a, d) = match family.trim().to_ascii_lowercase().as_str() {
        "times" => hhea_line(1825, -443, 87),
        "courier" => hhea_line(1705, -615, 0),
        _ => return None,
    };
    Some((a * upem / 2048.0, d * upem / 2048.0))
}

/// Line metrics for a face in font units: (ascent, descent), both positive, the way word
/// processors measure single line spacing: the hhea values with the line gap above the ascent
/// (Word puts the first baseline that far below the line top), falling back to OS/2
/// usWinAscent/usWinDescent; Times and Courier borrow their Windows counterparts' (see
/// [`borrowed_line`]).
pub fn line_metrics(face: &crate::FontFace) -> (f64, f64) {
    static CACHE: OnceLock<Mutex<HashMap<u32, (f64, f64)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&face.id()) {
        return *v;
    }
    let mut v = (face.ascent.max(1.0), face.descent.abs());
    if let Some(m) = borrowed_line(&face.family, face.upem) {
        v = m;
    } else if let Some(f) = face.skrifa() {
        // hhea ascender + descender + line gap (Word on macOS), the gap above the ascent; OS/2
        // win metrics only when the hhea values are missing, and never more than twice the em.
        let hhea = f.hhea().ok().map(|h| hhea_line(h.ascender().to_i16(), h.descender().to_i16(), h.line_gap().to_i16()));
        let win = f.os2().ok().map(|o| (o.us_win_ascent() as f64, o.us_win_descent() as f64));
        let ok = |m: &(f64, f64)| m.0 > 0.0 && m.0 + m.1 <= face.upem * 2.0;
        if let Some(m) = hhea.filter(ok) {
            v = m;
        } else if let Some(m) = win.filter(ok) {
            v = m;
        }
    }
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(face.id(), v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_gap_goes_above_the_ascent() {
        // Times New Roman (hhea 1825 / -443, gap 87), and a font with a large gap: Word puts the
        // first baseline the ascent plus the whole gap below the line top.
        assert_eq!(hhea_line(1825, -443, 87), (1912.0, 443.0));
        assert_eq!(hhea_line(1536, -512, 452), (1988.0, 512.0));
        // A negative gap adds nothing; the most negative descender doesn't overflow.
        assert_eq!(hhea_line(1000, -300, -50), (1000.0, 300.0));
        assert_eq!(hhea_line(1000, i16::MIN, 0), (1000.0, 32768.0));
    }

    #[test]
    fn times_and_courier_take_their_windows_counterparts_line() {
        // Apple's Times has 2048 units to the em, Times New Roman's line is 1.15 em.
        assert_eq!(borrowed_line("Times", 2048.0), Some((1912.0, 443.0)));
        assert_eq!(borrowed_line(" times ", 1000.0), Some((1912.0 / 2.048, 443.0 / 2.048)));
        assert_eq!(borrowed_line("Courier", 2048.0), Some((1705.0, 615.0)));
        // Only those two: the Windows fonts themselves, and other Mac fonts, keep their own.
        for family in ["Times New Roman", "Courier New", "Helvetica", "Hoefler Text", "Palatino", ""] {
            assert_eq!(borrowed_line(family, 2048.0), None, "{family}");
        }
    }

    #[test]
    fn family_cache_stays_bounded() {
        for i in 0..(FAMILY_CACHE_MAX + 50) {
            let _ = resolve(&format!("Zz Missing Family {i}"), false, false);
        }
        assert!(resolve_cache().lock().unwrap_or_else(|e| e.into_inner()).1.len() <= FAMILY_CACHE_MAX);
        let long = "L".repeat(10_000);
        let _ = resolve(&long, false, false);
        assert!(!resolve_cache().lock().unwrap_or_else(|e| e.into_inner()).1.keys().any(|k| k.0.len() > 1000));
    }

    #[test]
    fn metric_compatible_substitutes_are_known() {
        assert!(metric_compatible("Calibri", "Carlito"));
        assert!(metric_compatible(" arial ", "Liberation Sans"));
        assert!(metric_compatible("Arial CE", "Helvetica"));
        assert!(metric_compatible("Book Antiqua", "TeX Gyre Pagella"));
        assert!(!metric_compatible("Calibri", "Helvetica Neue"));
        assert!(!metric_compatible("Open Sans", "Carlito"));
        assert!(!metric_compatible("Times New Roman", "Times"));
        assert_eq!(metric_twins("arial").collect::<Vec<_>>(), ["Liberation Sans", "Helvetica", "Arimo"]);
    }

    #[test]
    fn a_missing_font_is_drawn_in_an_installed_twin() {
        // The twins that decide the font box's warning are the faces layout draws in.
        let db = FontDb::global();
        for family in METRIC_TWINS.iter().flat_map(|g| g.iter()) {
            let r = resolve(family, false, false);
            if r.substituted && metric_twins(family).any(|t| db.has_family(t)) {
                assert!(metric_compatible(family, &r.face.family), "{family} → {}", r.face.family);
            }
        }
    }

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
    fn palatino_family_prefers_palatino_look_alikes_over_generic_serif() {
        for name in ["Palatino Linotype", "Book Antiqua", "palatino"] {
            let list = substitutes(name);
            let pos = |n: &str| list.iter().position(|x| *x == n);
            assert!(pos("Palatino").is_some() && pos("TeX Gyre Pagella").is_some() && pos("P052").is_some(), "{name}");
            // All of them come before the generic serif fallbacks (Caladea, Times New Roman).
            assert!(pos("Palatino") < pos("Caladea") && pos("TeX Gyre Pagella") < pos("Times New Roman"), "{name}");
        }
        // Other serif families are unchanged.
        assert_eq!(substitutes("Georgia").first(), Some(&"Caladea"));
        // Charset aliases are the font itself, so they substitute like it.
        assert_eq!(charset_base_family("Times New Roman Cyr"), "Times New Roman");
        assert_eq!(charset_base_family("Arial CYR"), "Arial");
        assert_eq!(charset_base_family("Courier New CE"), "Courier New");
        assert_eq!(charset_base_family("Arial (Hebrew)"), "Arial");
        assert_eq!(charset_base_family("Cyr"), "Cyr");
        assert_eq!(charset_base_family("Calibri"), "Calibri");
        assert_eq!(substitutes(charset_base_family("Times New Roman Cyr")), substitutes("Times New Roman"));
    }

    #[test]
    fn pcl6_symbol_has_symbols_line_metrics() {
        assert_eq!(symbol_font_metrics("Symbol (PCL6)"), symbol_font_metrics("Symbol"));
        assert_eq!(symbol_font_metrics("Arial"), None);
    }

    #[test]
    fn a_one_em_symbol_face_takes_windows_symbols_line() {
        // Apple's Symbol (2048 units): hhea 1436 / -612, no gap, one em; Windows' Symbol line is 1.221 em.
        assert!(one_em(hhea_line(1436, -612, 0), 2048.0));
        assert!(!one_em((1005.0, 216.0), 1000.0));
        assert!(!one_em((899.0, 211.0), 1000.0));
        assert!(!one_em((0.0, 0.0), 0.0));
        let arial = resolve("Arial", false, false);
        assert_eq!(symbol_line("Arial", &arial), None);
        let symbol = resolve("Symbol", false, false);
        if symbol.substituted {
            assert_eq!(symbol_line("Symbol", &symbol), symbol_font_metrics("Symbol"));
        }
    }

    #[test]
    fn pcl6_symbol_resolves_as_symbol() {
        let actual = resolve("Symbol (PCL6)", false, false);
        let db = FontDb::global();
        if db.has_family("Symbol (PCL6)") {
            assert_eq!(actual.face.id(), db.face("Symbol (PCL6)", "Regular").id());
            assert!(!actual.substituted);
            return;
        }
        let expected = resolve("Symbol", false, false);
        assert_eq!(actual.face.id(), expected.face.id());
        assert_eq!(actual.substituted, expected.substituted);
    }

    #[test]
    fn unicode_arial_is_proportional() {
        for name in ["Arial Unicode MS", "ARIAL UNICODE MS"] {
            let candidates = substitutes(name);
            assert_eq!(candidates.first(), Some(&"Arial"));
            assert_eq!(&candidates[1..], substitutes("Arial"));
        }
        assert_eq!(substitutes("Cascadia Code").first(), Some(&"Courier New"));
    }

    #[test]
    fn symbol_fonts_substitute_symbol_faces() {
        for f in ["Symbol", "Wingdings", "Wingdings 2", "Wingdings 3", "Webdings"] {
            assert!(substitutes(f).contains(&"Noto Sans Symbols 2"), "{f}");
        }
    }

    #[test]
    fn resolves_weights_named_as_their_own_family() {
        // Word stores a Medium face as the family "Inter Medium" (#219).
        let r = resolve("Inter Medium", false, false);
        assert!(!r.substituted && !r.synth_bold && !r.synth_italic);
        assert_eq!((r.face.family.as_str(), r.face.style.as_str()), ("Inter", "Medium"));
        // Bold and italic within it are drawn from the Medium face.
        let b = resolve("Inter Medium", true, true);
        assert_eq!(b.face.style, "Medium");
        assert!(!b.substituted && b.synth_bold && b.synth_italic);
    }

    #[test]
    fn synthesizes_missing_styles() {
        // JetBrains Mono ships only Regular here.
        let r = resolve("JetBrains Mono", true, true);
        assert!(r.synth_bold && r.synth_italic);
    }

    #[test]
    fn hebrew_fonts_substitute_hebrew_faces() {
        for f in ["David", "Miriam", "Arial Hebrew", "Calibri Hebrew", "Noto Sans Hebrew"] {
            assert!(substitutes(f).contains(&"Noto Sans Hebrew"), "{f}");
            // David is a Windows font; only the Windows list offers it.
            if cfg!(target_os = "windows") {
                assert!(substitutes(f).contains(&"David") || f == "David", "{f}");
            }
        }
        // Generic names with "Hebrew" in them also get Hebrew substitutions
        assert!(substitutes("SomeFont Hebrew").contains(&"Noto Sans Hebrew"));
    }

    #[test]
    fn khmer_fonts_substitute_khmer_faces() {
        for f in ["Khmer OS Bokor", "Khmer OS Battambang", "Khmer Mool1", "Khmer OS Muol Light"] {
            assert!(substitutes(f).iter().any(|s| s.contains("Khmer")), "{f}: {:?}", substitutes(f));
        }
    }

    #[test]
    fn thai_fonts_substitute_thai_faces() {
        // Every Thai document family maps to a list of Thai faces, never to the Latin lists.
        for f in [
            "TH Sarabun New",
            "TH SarabunPSK",
            "Angsana New",
            "AngsanaUPC",
            "Cordia New",
            "CordiaUPC",
            "Browallia New",
            "BrowalliaUPC",
            "JasmineUPC",
            "KodchiangUPC",
            "LilyUPC",
            "DilleniaUPC",
            "EucrosiaUPC",
            "FreesiaUPC",
            "IrisUPC",
            "Leelawadee",
            "Leelawadee UI",
            "Noto Sans Thai",
            "TH Niramit AS",
        ] {
            assert!(is_thai_family(f), "{f}");
            let list = substitutes(f);
            assert!(list.iter().any(|s| s.contains("Thai")), "{f}: {list:?}");
            assert!(!list.contains(&"Carlito") && !list.contains(&"Caladea"), "{f}: {list:?}");
        }
        // Serif Thai families prefer a serif Thai face; the sans families a sans one.
        let pos = |family: &str, name: &str| substitutes(family).iter().position(|s| *s == name).unwrap_or(usize::MAX);
        assert!(pos("Angsana New", "Noto Serif Thai") < pos("Angsana New", "Noto Sans Thai"));
        assert!(pos("TH Sarabun New", "Noto Sans Thai") < pos("TH Sarabun New", "Noto Serif Thai"));
        // Latin and other script families are unchanged.
        assert!(!is_thai_family("Arial") && !is_thai_family("Tahoma") && !is_thai_family("Calibri") && !is_thai_family("David"));
        assert_eq!(substitutes("Calibri").first(), Some(&"Carlito"));
    }

    #[test]
    fn a_missing_thai_font_resolves_to_an_installed_thai_face() {
        // The issue's run (#1549): Thai letters and digits, Latin letters, digits and the baht
        // sign in one run of a Thai font that isn't installed.
        const RUN: &str = "[D] น้ำ ค่ำ ปั้น ฝั่ง ปู่ ๑๒๓๔๕๖๗๘๙๐ ฿1,250.50";
        let db = FontDb::global();
        for family in ["TH Sarabun New", "Angsana New", "Cordia New", "BrowalliaUPC"] {
            if db.has_family(family) {
                continue;
            }
            let r = resolve(family, false, false);
            let face = r.face.get();
            if !face.covers('\u{0E19}') {
                eprintln!("skipped {family}: no Thai face from its substitute list is installed");
                continue;
            }
            assert!(r.substituted, "{family}");
            let thai: Vec<char> = RUN.chars().filter(|c| ('\u{0E00}'..='\u{0E7F}').contains(c) && !face.covers(*c)).collect();
            assert!(thai.is_empty(), "{family} → {} lacks {thai:?}", face.family);
            // A listed Thai face with Latin letters and digits wins over one without them.
            let latin = |name: &str| {
                let f = db.face(name, "Regular");
                f.named(name) && f.covers('\u{0E19}') && f.covers('D') && f.covers('5')
            };
            if substitutes(family).iter().any(|s| db.has_family(s) && latin(s)) {
                let missing: Vec<char> = RUN.chars().filter(|c| !c.is_whitespace() && !face.covers(*c)).collect();
                assert!(missing.is_empty(), "{family} → {} lacks {missing:?}", face.family);
            }
        }
    }

    #[test]
    fn thonburi_stacks_thai_marks_through_its_apple_tables() {
        // Thonburi has `morx` but no GSUB: the shaper must still move a tone mark up over an upper
        // vowel (ปั่) instead of drawing it where it sits on a bare consonant (ป่).
        let db = FontDb::global();
        if !db.has_family("Thonburi") {
            eprintln!("skipped: Thonburi not installed");
            return;
        }
        let face = db.face("Thonburi", "Regular");
        let tone = |text: &str| crate::shape(&face, text, &[], |c| c).into_iter().last().map(|g| (g.gid, g.y_offset));
        let (bare, stacked) = (tone("\u{0E1B}\u{0E48}"), tone("\u{0E1B}\u{0E31}\u{0E48}"));
        assert!(bare.is_some() && stacked.is_some());
        assert_ne!(bare, stacked, "the tone mark over ั is raised or swapped");
    }

    #[test]
    fn fonts_with_a_clone_have_named_substitutes() {
        for f in ["Times New Roman", "Times New Roman Cyr", "Arial", "Courier New", "Palatino Linotype", "Symbol"] {
            assert!(has_named_substitutes(f), "{f}");
        }
        for f in ["No Such Brand Font", "Cambria", "Georgia", "Brand Serif"] {
            assert!(!has_named_substitutes(f), "{f}");
        }
    }

    /// Semicolon-separated fallback lists resolve to the first installed family, or to a
    /// candidate's substitute when no listed family is installed.
    #[test]
    fn libreoffice_fallback_list_resolves_first_installed_family() {
        let db = FontDb::global();
        if db.has_family("Liberation Sans") || db.has_family("Arial") {
            let r = resolve("Liberation Sans;Arial", false, false);
            assert!(!r.substituted, "should not be substituted when a candidate is installed: got {:?}", r.face.family);
            if db.has_family("Liberation Sans") {
                assert_eq!(r.face.family, "Liberation Sans");
            } else {
                assert_eq!(r.face.family, "Arial");
            }
        }
        let bundled = resolve("NonExistentSansFace123;Source Serif 4", false, false);
        assert!(!bundled.substituted);
        assert_eq!(bundled.face.family, "Source Serif 4");

        // When none of the families is installed, fall back to a substitute for an entry in the list rather than generic Carlito:
        if !db.has_family("NonExistentSansFace123") && !db.has_family("Times New Roman") {
            let r = resolve("NonExistentSansFace123;Times New Roman", false, false);
            assert!(r.substituted);
            assert_ne!(r.face.family, "Carlito", "should not fall back to generic Carlito when an entry is Times New Roman");
            assert!(r.face.family.contains("Serif") || r.face.family == "Times" || r.face.family == "Tinos", "got {:?}", r.face.family);
        }
        let r = resolve("NonExistentSansFace123;NonExistentSerifFace123", false, false);
        assert!(r.substituted);
        assert_ne!(r.face.family, "Carlito", "should not fall back to generic Carlito when an entry is a serif family");
        assert!(
            r.face.family.contains("Serif")
                || r.face.family == "Times"
                || r.face.family == "Times New Roman"
                || r.face.family == "Tinos"
                || r.face.family == "Caladea"
                || r.face.family == "Georgia",
            "got {:?}",
            r.face.family
        );

        // Metric compatibility of fallback list:
        assert!(metric_compatible("Open Sans;Arial", "Liberation Sans"));
        assert!(metric_compatible("Liberation Sans;Arial", "Helvetica"));
        assert!(!metric_compatible("Open Sans;Arial", "Carlito"));
    }

    /// In a mixed Thai-and-Latin fallback list where neither listed family is installed, an
    /// earlier Thai substitute takes precedence over a later candidate's metric twin.
    #[test]
    fn thai_fallback_list_prefers_thai_substitute_before_later_metric_twin() {
        let db = FontDb::global();
        let Some(thai_sub) = substitute(db, "NonExistentThaiUPC") else {
            eprintln!("skipped: no Thai substitute installed");
            return;
        };
        if !db.has_family("Cordia New") && !db.has_family("Arial") && metric_twins("Arial").any(|t| db.has_family(t)) {
            let r = resolve("Cordia New;Arial", false, false);
            assert!(r.substituted);
            assert!(substitutes("Cordia New").contains(&r.face.family.as_str()), "got {:?}", r.face.family);
        }
        if let Some(latin) =
            METRIC_TWINS.iter().flat_map(|g| g.iter().copied()).find(|&f| !db.has_family(f) && metric_twins(f).any(|t| db.has_family(t)))
        {
            let list = format!("NonExistentThaiUPC;{latin}");
            let r = resolve(&list, false, false);
            assert!(r.substituted);
            assert_eq!(r.face.family, thai_sub);
        }
    }

    /// Fallback lists longer than `FAMILY_KEY_MAX` still select a later installed family and
    /// cache their resolution without growing `resolve_cache` string keys.
    #[test]
    fn long_fallback_list_selects_later_installed_family_and_caches() {
        let mut candidates: Vec<String> = (0..120).map(|i| format!("MissingCandidateFont{i}")).collect();
        candidates.push("Source Serif 4".to_string());
        let list = candidates.join(";");
        assert!(list.len() > FAMILY_KEY_MAX);

        let r1 = resolve(&list, false, false);
        let r2 = resolve(&list, false, false);
        assert!(!r1.substituted);
        assert_eq!(r1.face.family, "Source Serif 4");
        assert_eq!(r1.face.id(), r2.face.id());
        assert!(!resolve_cache().lock().unwrap_or_else(|e| e.into_inner()).1.keys().any(|k| k.0.len() > FAMILY_KEY_MAX));
    }

    /// The name-based predicates look at every candidate, and their answer is cached per list: a
    /// later Thai or named fallback still counts, and a list is classified once rather than for
    /// every piece of text shaped in it.
    #[test]
    fn list_classification_reads_every_candidate_and_is_cached() {
        // The Thai family is last, past any shorter bound, and still classifies the list.
        let mut candidates: Vec<String> = (0..200).map(|i| format!("MissingCandidateFont{i}")).collect();
        candidates.push("TH Sarabun New".to_string());
        let thai_list = candidates.join(";");
        assert!(is_thai_family(&thai_list), "a later Thai family still counts");

        // A named substitute at the end classifies too (Times New Roman has Liberation Serif).
        candidates.pop();
        candidates.push("Times New Roman".to_string());
        let named_list = candidates.join(";");
        assert!(has_named_substitutes(&named_list), "a later named fallback still counts");

        // Both are memoised by list text: a second ask answers without rescanning, and a list
        // whose text differs only after the key's hash is not mistaken for the cached one.
        assert!(is_thai_family(&thai_list) && !is_thai_family(&named_list));
        let cached = fallback_list_cache().lock().unwrap_or_else(|e| e.into_inner()).classes.len();
        assert!(cached >= 2, "{cached} lists classified");
    }
}
