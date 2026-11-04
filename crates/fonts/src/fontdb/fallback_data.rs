use super::face::{FALLBACK_FAMILY, FontFace};
use super::{Arc, HashMap};
#[cfg(not(target_arch = "wasm32"))]
use super::{FileRef, MetadataProvider};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(super) struct SysFallback {
    pub(super) enabled: bool,
    pub(super) misses: std::collections::HashSet<char>,
    /// Preferred fallback families already loaded (or tried) by a fallback search.
    pub(super) tried: std::collections::HashSet<&'static str>,
}

/// Characters remembered as covered by no installed font (a cap on memory: once full the set
/// starts over, and the coverage index answers again quickly).
#[cfg(not(target_arch = "wasm32"))]
pub(super) const SYS_MISSES_MAX: usize = 65_536;

/// Installed font files larger than this are not searched for fallback characters.
#[cfg(not(target_arch = "wasm32"))]
const SYS_FALLBACK_MAX_FILE: u64 = 40 << 20;

/// Which characters each cataloged font file maps (#605). Before it, every distinct character no
/// loaded font covered read and parsed every installed font file again: 500 distinct Tangut
/// characters froze the app for most of a minute. Now each file is read once, when a search
/// first gets to it, and later searches answer from its ranges, reading only a file that matches.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(super) struct CoverageIndex {
    /// The catalog scan the file list was taken from (`None` until the first search).
    pub(super) generation: Option<u32>,
    /// The cataloged font files, sorted.
    pub(super) files: Vec<IndexedFile>,
    /// Font files read by fallback searches (each at most once to index it, once more to load it).
    pub(super) reads: usize,
    /// Files loaded by family name ([`FontDb::load_cataloged`]), whether or not they are indexed
    /// yet: a search never loads them a second time.
    pub(super) loaded: std::collections::HashSet<std::path::PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) struct IndexedFile {
    pub(super) path: std::path::PathBuf,
    /// The code points the file's faces map, as sorted, disjoint inclusive ranges (`None` until
    /// the file is read; empty when it can't be read or is too large).
    pub(super) ranges: Option<Box<[(u32, u32)]>>,
    /// Loaded for a character it covers: its faces are searched as loaded fonts from then on.
    pub(super) used: bool,
}

/// Does `ranges` (sorted, disjoint, inclusive) hold `cp`?
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn ranges_hold(ranges: &[(u32, u32)], cp: u32) -> bool {
    let i = ranges.partition_point(|r| r.1 < cp);
    ranges.get(i).is_some_and(|r| r.0 <= cp)
}

/// The code points mapped by the faces of `data` (a font file or collection), as sorted,
/// disjoint, inclusive ranges. Bounded however damaged the file: at most 256 faces, a budget of
/// mappings read across them, and a cap on the ranges kept.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn coverage_ranges(data: &[u8]) -> Box<[(u32, u32)]> {
    const MAX_FACES: u32 = 256;
    const MAX_MAPPINGS: usize = 4 * 0x11_0000;
    const MAX_RANGES: usize = 1 << 17;
    fn compact(r: &mut Vec<(u32, u32)>) {
        r.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(r.len());
        for &(s, e) in r.iter() {
            match out.last_mut() {
                Some(last) if s <= last.1.saturating_add(1) => last.1 = last.1.max(e),
                _ => out.push((s, e)),
            }
        }
        *r = out;
    }
    let count = match FileRef::new(data) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len().min(MAX_FACES),
        Err(_) => 0,
    };
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut budget = MAX_MAPPINGS;
    'faces: for i in 0..count {
        let Ok(f) = skrifa::FontRef::from_index(data, i) else { continue };
        for (cp, _) in f.charmap().mappings() {
            budget = match budget.checked_sub(1) {
                Some(b) => b,
                None => break 'faces,
            };
            match ranges.last_mut() {
                Some(last) if last.1.checked_add(1) == Some(cp) => last.1 = cp,
                Some(last) if last.0 <= cp && cp <= last.1 => {}
                _ => {
                    if ranges.len() >= MAX_RANGES {
                        compact(&mut ranges);
                        if ranges.len() >= MAX_RANGES {
                            break 'faces;
                        }
                    }
                    ranges.push((cp, cp));
                }
            }
        }
    }
    compact(&mut ranges);
    ranges.into_boxed_slice()
}

/// An installed font file's bytes, `None` when it can't be read or is larger than
/// [`SYS_FALLBACK_MAX_FILE`] (also when it grew past that since it was cataloged).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_fallback_file(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > SYS_FALLBACK_MAX_FILE {
        return None;
    }
    let mut data = Vec::new();
    file.take(SYS_FALLBACK_MAX_FILE + 1).read_to_end(&mut data).ok()?;
    (data.len() as u64 <= SYS_FALLBACK_MAX_FILE).then_some(data)
}

/// Families preferred for right-to-left scripts (Persian, Arabic, Urdu, Hebrew) when the
/// document's font lacks them: open fonts with full Persian coverage first (the Persian-only
/// letters پ چ ژ گ, the Persian digits), then the platforms' own.
/// Right-to-left script fonts (Hebrew, Arabic, Persian, Syriac…) in fallback order.
/// Windows prioritizes David/Miriam (Office) and Segoe UI (standard).
/// macOS prioritizes Helvetica, Geeza Pro (system), and SF fonts.
/// Linux uses Noto fonts.
#[cfg(windows)]
pub const RTL_FALLBACKS: &[&str] = &[
    "Vazirmatn",
    "Noto Naskh Arabic",
    "Noto Sans Arabic",
    "Noto Sans Arabic UI",
    "Sahel",
    "Shabnam",
    "Samim",
    "Noto Sans Hebrew",
    "David",
    "Miriam",
    "Segoe UI",
    "Tahoma",
    "Arial",
    "DejaVu Sans",
];

#[cfg(target_os = "macos")]
pub const RTL_FALLBACKS: &[&str] = &[
    "Helvetica",
    "Vazirmatn",
    "Noto Naskh Arabic",
    "Noto Sans Arabic",
    "Noto Sans Hebrew",
    "Geeza Pro",
    "SF Arabic",
    "Arial",
    "Tahoma",
    "DejaVu Sans",
];

#[cfg(not(any(windows, target_os = "macos")))]
pub const RTL_FALLBACKS: &[&str] =
    &["Noto Naskh Arabic", "Noto Sans Arabic", "Noto Sans Hebrew", "Vazirmatn", "Segoe UI", "Arial", "Tahoma", "DejaVu Sans"];

/// Families tried (when installed) for characters the loaded fonts lack: CJK, symbols, emoji.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const SYSTEM_FALLBACKS: &[&str] = &[
    "Helvetica Neue",
    "Arial",
    "Segoe UI",
    "Noto Sans",
    "DejaVu Sans",
    "PingFang SC",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Apple SD Gothic Neo",
    "Heiti SC",
    "STHeiti",
    "Microsoft YaHei",
    "Yu Gothic",
    "Malgun Gothic",
    "Noto Sans CJK SC",
    "Noto Sans CJK JP",
    "Arial Unicode MS",
    "Apple Symbols",
    "Segoe UI Symbol",
    "Noto Sans Symbols",
    "Noto Sans Symbols2",
    "Noto Emoji",
    "Segoe UI Emoji",
    "Apple Color Emoji",
    "Noto Color Emoji",
];

/// Arabic-script marks and digits (bidi classes NSM / AN): they belong with the Arabic letters
/// around them when choosing a fallback font.
pub(super) fn arabic_block(c: char) -> bool {
    matches!(u32::from(c), 0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

#[derive(Default)]
pub(super) struct FallbackCache {
    pub(super) faces: usize,
    pub(super) map: HashMap<(char, u32), Option<Arc<FontFace>>>,
}

/// What a run-level fallback lookup is keyed by: sorted, deduplicated chars, excluded face, bold, italic.
pub(super) type RunFallbackKey = (Vec<char>, u32, bool, bool);

/// [`FontDb::fallback_for_run`] results: (sorted, deduplicated chars, excluded face, bold,
/// italic) → face, valid while the loaded face count is `faces`.
#[derive(Default)]
pub(super) struct RunFallbackCache {
    pub(super) faces: usize,
    /// Chars held in the keys (bounds memory: hostile runs can be long).
    pub(super) chars: usize,
    pub(super) map: HashMap<RunFallbackKey, Option<Arc<FontFace>>>,
}

/// Most cached run-level fallbacks, and most chars in their keys, before the cache is reset.
pub(super) const RUN_FALLBACK_CACHE_MAX: usize = 4096;

pub(super) const RUN_FALLBACK_CACHE_CHARS: usize = 1 << 20;

/// Loaded faces other than `exclude`, in fallback order: for right-to-left text the fonts made
/// for the script first (so a word isn't split across fonts, which breaks Arabic joining); for a
/// complex-script character `complex` (Thai, Myanmar…) faces with OpenType layout tables, the
/// script's own families first, as the per-character fallback ranks them; then the fallback
/// family, upright, nearest to regular weight.
pub(super) fn fallback_order(faces: &[Arc<FontFace>], exclude: u32, rtl: bool, complex: Option<char>) -> Vec<&Arc<FontFace>> {
    let mut order: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.id != exclude).collect();
    let rank =
        |f: &FontFace| if rtl { RTL_FALLBACKS.iter().position(|n| f.family.eq_ignore_ascii_case(n)).unwrap_or(RTL_FALLBACKS.len()) } else { 0 };
    let shaping = |f: &FontFace| match complex {
        Some(c) => (!f.has_layout_tables(), crate::fallback_rank(c, &f.family)),
        None => (false, 0),
    };
    order.sort_by_key(|f| (rank(f), shaping(f), !f.family.eq_ignore_ascii_case(FALLBACK_FAMILY), f.italic, (f.weight - 400.0).abs() as i32));
    order
}

pub(super) const FALLBACK_CACHE_MAX: usize = 20_000;
