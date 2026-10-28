//! Font database: bundled OFL fonts, user fonts, the installed system fonts (cataloged the first
//! time a lookup needs them, native only), outline cache.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use kurbo::BezPath;
use skrifa::instance::{Location, LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, Hinting, HintingInstance, HintingOptions, OutlinePen, Target};
use skrifa::raw::FileRef;
use skrifa::string::StringId;
use skrifa::{GlyphId, MetadataProvider};

pub(crate) mod user;
pub use user::{FONT_FILE_EXTS, FontFormat, Installed, MAX_FONT_FILE_BYTES, UserFont, UserFontError, decode_font_file, is_font_file_name, sniff};

mod catalog;
mod face;
mod fallback;
mod fallback_data;
mod lookup;
mod names;

use face::{BUNDLED, FontBytes};
pub use face::{FALLBACK_FAMILY, FaceRef, FontFace, bundled};
#[cfg(not(target_arch = "wasm32"))]
use fallback_data::{CoverageIndex, SysFallback};
use fallback_data::{FallbackCache, RunFallbackCache};
#[cfg(test)]
use fallback_data::{RUN_FALLBACK_CACHE_CHARS, coverage_ranges, ranges_hold};
#[cfg(not(target_arch = "wasm32"))]
use names::CatalogEntry;
pub(crate) use names::face_names;
pub use names::{LocalName, base_style, name_language_matches, system_font_dirs};
use names::{MAX_FILE_FACES, enumerate_faces, make_face};
#[cfg(test)]
use names::{file_face_names, font_faces, mini_font, name};
#[cfg(test)]
pub(crate) use names::{last_resort_face, legacy_family};

/// Process-wide font database.
pub struct FontDb {
    faces: RwLock<Vec<Arc<FontFace>>>,
    outlines: Mutex<HashMap<(u32, u32), Arc<BezPath>>>,
    /// Fallback faces found per (character, excluded face), valid while the face count is the
    /// one stored (faces are only ever added). Layout asks once per uncovered character, and the
    /// search sorts every loaded face, so without this text in a script the requested font lacks
    /// (Thai, #59) got slower with every font loaded.
    fallbacks: RwLock<FallbackCache>,
    run_fallbacks: RwLock<RunFallbackCache>,
    #[cfg(not(target_arch = "wasm32"))]
    catalog: RwLock<Vec<CatalogEntry>>,
    /// Fonts the user added at run time, and the folder they are stored in (#582).
    user: RwLock<user::UserFonts>,
    /// Bumped whenever user fonts are added or removed ([`FontDb::generation`]).
    generation: AtomicU64,
    /// The folders the system font scan reads (the platform's font folders for [`FontDb::global`]).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    font_dirs: Vec<std::path::PathBuf>,
    /// Set once the font folders have been scanned. Lookups by family name wait for the first
    /// scan, so what they find doesn't depend on what ran before them.
    #[cfg(not(target_arch = "wasm32"))]
    cataloged: std::sync::OnceLock<()>,
    /// Held while an installed family's files load, so two lookups don't load it twice.
    #[cfg(not(target_arch = "wasm32"))]
    loading: Mutex<()>,
    /// System fallback state: enabled, and characters no system font covers.
    #[cfg(not(target_arch = "wasm32"))]
    sys: Mutex<SysFallback>,
    /// Held through a system fallback search, so searches run one at a time. Without it a search
    /// racing another that was still loading a preferred family ("Padauk" for Myanmar) skipped
    /// that family as already tried and loaded whatever the coverage index found first (only
    /// Padauk Bold), so which face a character fell back to depended on thread timing.
    #[cfg(not(target_arch = "wasm32"))]
    searching: Mutex<()>,
    /// The characters each installed font file maps, read the first time a fallback search
    /// needs that file. Held through a search, so two searches never read a file twice.
    #[cfg(not(target_arch = "wasm32"))]
    coverage: Mutex<CoverageIndex>,
    /// Counts the font folder scans, so the coverage index notices a rescan.
    #[cfg(not(target_arch = "wasm32"))]
    catalog_gen: AtomicU32,
}

impl FontDb {
    /// A database holding the bundled fonts, then the craft-fonts Japanese faces (Mincho first,
    /// so they are the fallback for Japanese text after the requested and bundled fonts), then the
    /// other craft-fonts faces: Simplified Chinese, for the ideographs the Japanese fonts lack, and
    /// Arabic, so Arabic and Persian text draws without system fonts (#1652) (all empty without
    /// `CRAFT_FONTS_DIR`), whose system font scan reads `font_dirs`.
    pub fn with_font_dirs(font_dirs: Vec<std::path::PathBuf>) -> Self {
        let mut faces = Vec::new();
        let others = crate::CRAFT_FONTS.iter().filter(|f| !f.scripts.contains(&"Jpan"));
        let craft = crate::japanese_document_fonts().into_iter().chain(others).map(|f| f.bytes);
        for data in BUNDLED.iter().copied().chain(craft) {
            for f in enumerate_faces(data, MAX_FILE_FACES) {
                if let Some(mut face) = make_face(FontBytes::Static(data), f.index, f.family, f.style, f.legacy, f.coords) {
                    face.local_names = f.local;
                    faces.push(Arc::new(face));
                }
            }
        }
        Self {
            faces: RwLock::new(faces),
            outlines: Mutex::new(HashMap::new()),
            fallbacks: RwLock::new(FallbackCache::default()),
            run_fallbacks: RwLock::new(RunFallbackCache::default()),
            #[cfg(not(target_arch = "wasm32"))]
            catalog: RwLock::new(Vec::new()),
            user: RwLock::new(user::UserFonts::default()),
            generation: AtomicU64::new(0),
            font_dirs,
            #[cfg(not(target_arch = "wasm32"))]
            cataloged: std::sync::OnceLock::new(),
            #[cfg(not(target_arch = "wasm32"))]
            loading: Mutex::new(()),
            #[cfg(not(target_arch = "wasm32"))]
            sys: Mutex::new(SysFallback { enabled: true, ..Default::default() }),
            #[cfg(not(target_arch = "wasm32"))]
            searching: Mutex::new(()),
            #[cfg(not(target_arch = "wasm32"))]
            coverage: Mutex::new(CoverageIndex::default()),
            #[cfg(not(target_arch = "wasm32"))]
            catalog_gen: AtomicU32::new(0),
        }
    }

    /// Enable or disable the lazy system-font fallback for characters the loaded fonts lack
    /// (native only; on by default).
    pub fn set_system_fallback(&self, on: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.sys.lock().unwrap_or_else(std::sync::PoisonError::into_inner).enabled = on;
        }
        // Misses remembered while it was off may be found now.
        self.fallbacks.write().unwrap_or_else(std::sync::PoisonError::into_inner).map.clear();
        *self.run_fallbacks.write().unwrap_or_else(std::sync::PoisonError::into_inner) = RunFallbackCache::default();
        #[cfg(target_arch = "wasm32")]
        let _ = on;
    }

    /// Process-wide database: the bundled fonts, then the installed system fonts, cataloged the
    /// first time a lookup by family name needs them (or ahead of time by
    /// [`FontDb::scan_in_background`]).
    pub fn global() -> &'static FontDb {
        static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
        DB.get_or_init(|| FontDb::with_font_dirs(system_font_dirs()))
    }

    fn read_faces(&self) -> std::sync::RwLockReadGuard<'_, Vec<Arc<FontFace>>> {
        self.faces.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The system font catalog, scanned first if it hasn't been yet.
    #[cfg(not(target_arch = "wasm32"))]
    fn read_catalog(&self) -> std::sync::RwLockReadGuard<'_, Vec<CatalogEntry>> {
        self.ensure_catalog();
        self.catalog.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
