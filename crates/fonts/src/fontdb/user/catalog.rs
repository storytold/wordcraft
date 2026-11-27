//! The added fonts' catalogue: what each file in the user font folder holds, kept beside the files
//! (`.catalog.json`), so a start reads that one small file instead of every font file, and a
//! font's file is read the first time a document uses one of its faces. Startup no longer grows
//! with the fonts the user added and doesn't use, and neither does memory.
//!
//! An entry is keyed by the file's name, size and modification time. A file added, replaced or
//! changed since is read and checked again, as an added font is, and an unusable one is skipped
//! with its reason, which the entry keeps, so the next start skips it without reading it. The
//! whole catalogue is made again when WordCraft's version or the catalogue's format changes, so
//! what it says always comes from the code that loads the fonts: each face carries the
//! descriptions [`make_face`](super::super::names::make_face) works out (names, weight, metrics)
//! and the answers fallback searches and font lists ask of it, which a loaded face would give.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use super::super::face::{FontBytes, FontFace, HINT_STRIKE, NEXT_ID};
use super::super::fallback_data::{Coverage, face_coverage};
use super::super::names::LocalName;
use super::{MAX_FACES_PER_FILE, MAX_FONT_FILE_BYTES, UserFontError, decode_owned};

/// The catalogue's file in the user font folder.
pub(super) const CATALOG_FILE: &str = ".catalog.json";

/// Bump when what an entry holds, or how it is worked out, changes.
const FORMAT: u32 = 1;

/// The largest catalogue read (a face takes a few hundred bytes; 1,000 fonts of 256 faces each
/// would be about 80 MB).
const MAX_CATALOG_BYTES: u64 = 128 << 20;

/// A file's size and modification time: an entry describes the file only while both match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stamp {
    len: u64,
    /// Seconds from 1970 (negative before) and the nanoseconds past them.
    secs: i64,
    nanos: u32,
}

impl Stamp {
    /// The stamp of the file `meta` describes (`None` where the platform keeps no modification
    /// time: such a file is read and checked at every start).
    pub(super) fn of(meta: &std::fs::Metadata) -> Option<Stamp> {
        let (secs, nanos) = match meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => (i64::try_from(d.as_secs()).ok()?, d.subsec_nanos()),
            Err(e) => (i64::try_from(e.duration().as_secs()).ok()?.checked_neg()?, e.duration().subsec_nanos()),
        };
        Some(Stamp { len: meta.len(), secs, nanos })
    }

    fn of_path(path: &Path) -> Option<Stamp> {
        std::fs::metadata(path).ok().as_ref().and_then(Stamp::of)
    }
}

/// One face of a catalogued file, as [`make_face`](super::super::names::make_face) made it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct FaceRecord {
    index: u32,
    pub(super) family: String,
    pub(super) style: String,
    legacy: String,
    /// (language, name, English name) of the family's names in other languages.
    local: Vec<(String, String, String)>,
    full_name: Option<String>,
    postscript: Option<String>,
    coords: Vec<([u8; 4], f32)>,
    weight: f32,
    italic: bool,
    upem: f64,
    ascent: f64,
    descent: f64,
    cap_height: f64,
    x_height: f64,
    avg_char_width: f64,
    described: Described,
}

impl FaceRecord {
    /// The record of `face` (a face made from the whole file).
    pub(super) fn of(face: &FontFace) -> FaceRecord {
        FaceRecord {
            index: face.index,
            family: face.family.clone(),
            style: face.style.clone(),
            legacy: face.legacy_family.clone(),
            local: face.local_names.iter().map(|n| (n.lang.clone(), n.name.clone(), n.english.clone())).collect(),
            full_name: face.full_name.clone(),
            postscript: face.postscript.clone(),
            coords: face.coords.clone(),
            weight: face.weight,
            italic: face.italic,
            upem: face.upem,
            ascent: face.ascent,
            descent: face.descent,
            cap_height: face.cap_height,
            x_height: face.x_height,
            avg_char_width: face.avg_char_width,
            described: Described { east_asian: face.is_east_asian(), serif: face.is_serif(), layout_tables: face.has_layout_tables() },
        }
    }

    /// Could [`make_face`](super::super::names::make_face) have made it? A catalogue is read from
    /// disk: one with impossible values (damaged, or written by hand) describes no face.
    fn plausible(&self) -> bool {
        let finite = [f64::from(self.weight), self.ascent, self.descent, self.cap_height, self.x_height, self.avg_char_width];
        !self.family.is_empty()
            && self.upem >= 1.0
            && self.upem.is_finite()
            && finite.iter().all(|v| v.is_finite())
            && self.coords.iter().all(|(_, v)| v.is_finite())
    }

    /// The face it describes, reading `file` only when the face is first used.
    pub(super) fn face(&self, file: &Arc<FontFile>) -> FontFace {
        FontFace {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            family: self.family.clone(),
            style: self.style.clone(),
            legacy_family: self.legacy.clone(),
            local_names: self
                .local
                .iter()
                .map(|(lang, name, english)| LocalName { lang: lang.clone(), name: name.clone(), english: english.clone() })
                .collect(),
            full_name: self.full_name.clone(),
            postscript: self.postscript.clone(),
            weight: self.weight,
            italic: self.italic,
            upem: self.upem,
            ascent: self.ascent,
            descent: self.descent,
            cap_height: self.cap_height,
            x_height: self.x_height,
            avg_char_width: self.avg_char_width,
            shaper: OnceLock::new(),
            coords: self.coords.clone(),
            location: OnceLock::new(),
            instance: OnceLock::new(),
            described: Some(self.described),
            bytes: FontBytes::File(file.clone()),
            index: self.index,
            bmp: OnceLock::new(),
            ranges: OnceLock::new(),
            outline_hinter: OnceLock::new(),
            glyph_styles: OnceLock::new(),
            hint_draws: Default::default(),
            hint_slow: Default::default(),
            hinting_off: Default::default(),
            hint_strike: HINT_STRIKE,
        }
    }
}

/// What fallback searches and font lists ask of a face, answered without reading its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Described {
    /// [`FontFace::is_east_asian`].
    pub(crate) east_asian: bool,
    /// [`FontFace::is_serif`].
    pub(crate) serif: bool,
    /// [`FontFace::has_layout_tables`].
    pub(crate) layout_tables: bool,
}

/// What one file in the folder holds: its faces, or why it can't be used.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub(super) file: String,
    pub(super) stamp: Stamp,
    pub(super) faces: Result<Vec<FaceRecord>, String>,
}

#[derive(Serialize, Deserialize)]
struct CatalogFile {
    format: u32,
    version: String,
    files: Vec<Entry>,
}

/// The catalogue kept in `dir`, by file name; empty when there is none, it can't be read, or an
/// earlier format or WordCraft version wrote it.
pub(super) fn read(dir: &Path) -> std::collections::HashMap<String, Entry> {
    use std::io::Read;
    let mut text = String::new();
    let read = std::fs::File::open(dir.join(CATALOG_FILE)).and_then(|f| f.take(MAX_CATALOG_BYTES).read_to_string(&mut text));
    let Ok(_) = read else { return Default::default() };
    match serde_json::from_str::<CatalogFile>(&text) {
        Ok(c) if c.format == FORMAT && c.version == env!("CARGO_PKG_VERSION") => {
            // An entry describing an impossible face is left out: its file is read again.
            let plausible = |e: &Entry| e.faces.as_ref().map_or(true, |f| f.len() <= MAX_FACES_PER_FILE && f.iter().all(FaceRecord::plausible));
            c.files.into_iter().filter(plausible).map(|e| (e.file.clone(), e)).collect()
        }
        Ok(_) => Default::default(),
        Err(e) => {
            log::warn!("the added fonts' catalogue in {} can't be read ({e}): reading the fonts again", dir.display());
            Default::default()
        }
    }
}

/// Keep `entries` as the catalogue in `dir`, replacing the old one whole (through a temporary
/// file, so a crash never leaves half a catalogue).
pub(super) fn write(dir: &Path, entries: &[Entry]) -> std::io::Result<()> {
    // One write at a time; another process's write may still land last, which is fine: either
    // describes files that are there, and an entry that no longer matches is read again.
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = WRITE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = serde_json::to_string(&CatalogFile { format: FORMAT, version: env!("CARGO_PKG_VERSION").into(), files: entries.to_vec() })
        .map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{CATALOG_FILE}.part"));
    // Created afresh, never through what is there (see `write_unique`).
    let create = || std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp);
    let mut file = match create() {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(&tmp)?;
            create()?
        }
        r => r?,
    };
    let written = std::io::Write::write_all(&mut file, text.as_bytes());
    drop(file);
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, dir.join(CATALOG_FILE))) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Read the font file at `path` whole and check it as an added font is checked
/// ([`check_faces`](super::check_faces)): its faces, or why it can't be used (kept with the entry),
/// or an error reading it now (which may pass, so it isn't kept).
pub(super) fn scan(path: &Path, shared: &FileShared) -> std::io::Result<Result<Vec<FaceRecord>, String>> {
    shared.count_read();
    let bytes = std::fs::read(path)?;
    Ok(decode_owned(bytes)
        .and_then(|sfnt| super::check_faces(Arc::new(sfnt)))
        .map(|faces| faces.iter().map(FaceRecord::of).collect())
        .map_err(|e| e.to_string()))
}

/// What an added font's file shares with its database.
#[derive(Default)]
pub(crate) struct FileShared {
    /// Files that couldn't be read when first used: part of [`FontDb::generation`](super::super::FontDb::generation),
    /// so lookups made while they were thought usable are made again.
    pub(super) failures: AtomicU64,
    /// Font files read, whole or in part (tests).
    #[cfg(test)]
    pub(super) reads: std::sync::atomic::AtomicUsize,
}

impl FileShared {
    fn count_read(&self) {
        #[cfg(test)]
        self.reads.fetch_add(1, Ordering::Relaxed);
    }
}

/// An added font's file, read the first time one of its faces is used and kept from then on.
pub(crate) struct FontFile {
    path: PathBuf,
    /// The size and modification time it had when it was read and checked.
    stamp: Option<Stamp>,
    /// Its bytes once read; empty when it couldn't be read.
    bytes: OnceLock<Vec<u8>>,
    shared: Arc<FileShared>,
}

impl FontFile {
    pub(super) fn new(path: PathBuf, stamp: Option<Stamp>, shared: Arc<FileShared>) -> FontFile {
        FontFile { path, stamp, bytes: OnceLock::new(), shared }
    }

    /// The file's bytes, read on the first call.
    pub(crate) fn data(&self) -> &[u8] {
        self.bytes.get_or_init(|| match self.read() {
            Ok(b) => b,
            Err(e) => {
                log::warn!("couldn't read the added font {}: {e}; its text is drawn in another font", self.path.display());
                self.shared.failures.fetch_add(1, Ordering::AcqRel);
                Vec::new()
            }
        })
    }

    /// The file's bytes, read for a caller that doesn't keep them (a font list preview).
    pub(crate) fn read_unkept(&self) -> Option<Vec<u8>> {
        match self.bytes.get() {
            Some(b) if !b.is_empty() => Some(b.clone()),
            Some(_) => None,
            None => self.read().ok(),
        }
    }

    /// Has the file been read (and could it be)?
    pub(crate) fn loaded(&self) -> bool {
        self.bytes.get().is_some_and(|b| !b.is_empty())
    }

    /// Couldn't the file be read when it was first used?
    pub(crate) fn failed(&self) -> bool {
        self.bytes.get().is_some_and(Vec::is_empty)
    }

    /// The file, if it is still the one the catalogue describes, as plain OpenType bytes.
    fn read(&self) -> Result<Vec<u8>, UserFontError> {
        let changed = || UserFontError::Unusable("the file changed since it was added".into());
        let meta = std::fs::metadata(&self.path).map_err(|e| UserFontError::Unusable(e.to_string()))?;
        if self.stamp.is_some() && Stamp::of(&meta) != self.stamp {
            return Err(changed());
        }
        if meta.len() > MAX_FONT_FILE_BYTES as u64 {
            return Err(UserFontError::TooLarge { bytes: usize::try_from(meta.len()).unwrap_or(usize::MAX) });
        }
        self.shared.count_read();
        let bytes = std::fs::read(&self.path).map_err(|e| UserFontError::Unusable(e.to_string()))?;
        decode_owned(bytes)
    }

    /// The code points face `index` maps: from the bytes when they are read, otherwise from the
    /// file's character map alone.
    pub(crate) fn coverage(&self, index: u32) -> Box<[(u32, u32)]> {
        if let Some(b) = self.bytes.get() {
            let mut coverage = Coverage::default();
            if let Ok(f) = skrifa::FontRef::from_index(b, index) {
                coverage.add(&f);
            }
            return coverage.finish();
        }
        if self.stamp.is_some() && Stamp::of_path(&self.path) != self.stamp {
            return Box::default();
        }
        self.shared.count_read();
        face_coverage(&self.path, index)
    }
}
