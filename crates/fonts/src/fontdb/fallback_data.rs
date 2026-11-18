#[cfg(not(target_arch = "wasm32"))]
use super::MetadataProvider;
use super::face::{FALLBACK_FAMILY, FontFace};
#[cfg(not(target_arch = "wasm32"))]
use super::names::{TableReader, mini_font};
use super::{Arc, HashMap};

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

/// Installed font files larger than this are not loaded for a fallback character the coverage
/// index finds in them (a preferred fallback family loads whatever its size).
#[cfg(not(target_arch = "wasm32"))]
pub(super) const SYS_FALLBACK_MAX_FILE: u64 = 40 << 20;

/// Which characters each cataloged font file maps (#605). Before it, every distinct character no
/// loaded font covered read and parsed every installed font file again: 500 distinct Tangut
/// characters froze the app for most of a minute. Now each file's character maps are read once,
/// when a search first gets to it, and later searches answer from its ranges, loading only a file
/// that matches. The system fallback asks it too before loading a preferred family.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(super) struct CoverageIndex {
    /// The catalog scan the file list was taken from (`None` until the first search).
    pub(super) generation: Option<u32>,
    /// The cataloged font files, sorted.
    pub(super) files: Vec<IndexedFile>,
    /// Font files whose character maps fallback searches read (each once, and again only when
    /// a rescan finds it replaced).
    pub(super) reads: usize,
    /// Files loaded by family name ([`FontDb::load_cataloged`]), whether or not they are indexed
    /// yet: a search never loads them a second time.
    pub(super) loaded: std::collections::HashSet<std::path::PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
impl CoverageIndex {
    /// Does the cataloged file at `path` map `cp`? Its character maps are read the first time.
    pub(super) fn file_holds(&mut self, path: &std::path::Path, cp: u32) -> bool {
        let Ok(i) = self.files.binary_search_by(|f| f.path.as_path().cmp(path)) else { return false };
        let Some(file) = self.files.get_mut(i) else { return false };
        ranges_hold(file.ranges(&mut self.reads), cp)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) struct IndexedFile {
    pub(super) path: std::path::PathBuf,
    /// The code points the file's faces map, as sorted, disjoint inclusive ranges (`None` until
    /// its character maps are read; empty when they can't be).
    pub(super) ranges: Option<Box<[(u32, u32)]>>,
    /// The file's size and modification time when its character maps were read: a rescan
    /// drops the ranges of a file replaced since.
    pub(super) stamp: Option<(u64, std::time::SystemTime)>,
    /// Loaded for a character it covers: its faces are searched as loaded fonts from then on.
    pub(super) used: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl IndexedFile {
    /// The code points the file maps. Its character maps are read the first time (counted in
    /// `reads`), and its stamp kept with them, so a rescan keeps them unless the file is replaced.
    pub(super) fn ranges(&mut self, reads: &mut usize) -> &[(u32, u32)] {
        let (path, stamp) = (&self.path, &mut self.stamp);
        self.ranges.get_or_insert_with(|| {
            *reads += 1;
            *stamp = file_stamp(path);
            file_coverage(path)
        })
    }
}

/// The size and modification time of the file at `path` (`None` when it can't be read).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn file_stamp(path: &std::path::Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Does `ranges` (sorted, disjoint, inclusive) hold `cp`?
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn ranges_hold(ranges: &[(u32, u32)], cp: u32) -> bool {
    let i = ranges.partition_point(|r| r.1 < cp);
    ranges.get(i).is_some_and(|r| r.0 <= cp)
}

/// The code points a font file's faces map, gathered face by face into sorted, disjoint,
/// inclusive ranges. Bounded however damaged the file: a budget of mappings read across its
/// faces, and a cap on the ranges kept.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(super) struct Coverage {
    ranges: Vec<(u32, u32)>,
    mappings: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl Coverage {
    pub(super) const MAX_MAPPINGS: usize = 4 * 0x11_0000;
    pub(super) const MAX_RANGES: usize = 1 << 17;

    /// Add the code points `f` maps. False once a bound is reached: later faces are left out.
    pub(super) fn add(&mut self, f: &skrifa::FontRef<'_>) -> bool {
        f.charmap().mappings().all(|(cp, _)| self.push(cp))
    }

    /// Add one mapped code point. False once a bound is reached.
    pub(super) fn push(&mut self, cp: u32) -> bool {
        if self.mappings >= Self::MAX_MAPPINGS {
            return false;
        }
        self.mappings += 1;
        match self.ranges.last_mut() {
            Some(last) if last.1.checked_add(1) == Some(cp) => last.1 = cp,
            Some(last) if last.0 <= cp && cp <= last.1 => {}
            _ => {
                if self.ranges.len() >= Self::MAX_RANGES {
                    self.compact();
                    // A compaction must free a quarter of the cap, or a crafted file could make
                    // every later mapping sort all the ranges again.
                    if self.ranges.len() > Self::MAX_RANGES / 4 * 3 {
                        return false;
                    }
                }
                self.ranges.push((cp, cp));
            }
        }
        true
    }

    fn compact(&mut self) {
        self.ranges.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(self.ranges.len());
        for &(s, e) in self.ranges.iter() {
            match out.last_mut() {
                Some(last) if s <= last.1.saturating_add(1) => last.1 = last.1.max(e),
                _ => out.push((s, e)),
            }
        }
        self.ranges = out;
    }

    pub(super) fn finish(mut self) -> Box<[(u32, u32)]> {
        self.compact();
        self.ranges.into_boxed_slice()
    }
}

/// The code points the faces of the font file at `path` map, as sorted, disjoint, inclusive
/// ranges, read from their character maps (`cmap`) alone: Apple Color Emoji's 180 MB cost a few
/// kilobytes. Empty when the file can't be read.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn file_coverage(path: &std::path::Path) -> Box<[(u32, u32)]> {
    /// The longest character map read (a format 12 map of every code point is 13 MB).
    const MAX_CMAP: u32 = 16 << 20;
    let mut coverage = Coverage::default();
    let Some(mut file) = TableReader::open(path) else { return coverage.finish() };
    // The faces of a collection often share one map: each is read once.
    let mut read: Vec<(u32, u32)> = Vec::new();
    for face in 0..file.faces.len() {
        let Some(at) = file.locate(face, b"cmap") else { continue };
        if read.contains(&at) {
            continue;
        }
        read.push(at);
        let Some(cmap) = file.read(at, MAX_CMAP) else { continue };
        // A font holding just the map, to read it as the font itself would.
        let font = mini_font(&[(*b"cmap", cmap)]);
        let Ok(f) = skrifa::FontRef::new(&font) else { continue };
        if !coverage.add(&f) {
            break;
        }
    }
    coverage.finish()
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
