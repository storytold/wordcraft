//! The added fonts' catalogue: what each file in the user font folder holds, kept beside the files
//! (`.catalog.json`), so a start reads that one small file instead of every font file, and a
//! font's file is read the first time one of its faces is used. A start still lists the folder and
//! reads each file's metadata, and the faces' descriptions take memory; the fonts' bytes don't.
//!
//! An entry describes the file of one exact name while its size and modification time are as they
//! were, and on Unix its inode and status-change time, which a rewrite or a replacement changes
//! even when it keeps the other two. Windows offers only the creation time, which a rewrite in
//! place keeps (see `identity`). A file added, replaced or changed since is read and checked again,
//! as an added font is. A file that can't be used isn't kept: it is read and checked at every
//! start, so a repaired one shows.
//!
//! The catalogue's first line is a digest of the rest: a damaged or edited catalogue is made again
//! from the fonts, as it is when WordCraft's version or the catalogue's format changes. A face
//! listed from it is checked against its file before it is first handed out
//! ([`FontDb::face`](super::super::FontDb::face), also before a style at other axis values is made
//! from it, and the fallback searches), so a description that doesn't match the file never reaches
//! layout: the file's faces are made again from its bytes, and the entry with them. A font list
//! preview of a font no document uses is drawn from a copy read from the file.
//!
//! A file's bytes are read only while it has the stamp it was listed with: a font whose file is
//! replaced, removed or made unreadable during a session, before a document uses it, falls back
//! until the next start, which reads the file as it is then.
//!
//! Each write goes to a temporary file of its own, renamed over the catalogue once complete, so a
//! reader finds a whole catalogue, old or new, whatever other writers (other instances of the app)
//! are doing. One left for an hour (its writer stopped) is cleared at a later start.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::super::face::{FontBytes, FontFace, HINT_STRIKE, NEXT_ID};
use super::super::fallback_data::{Coverage, face_coverage};
use super::super::names::LocalName;
use super::{MAX_FACES_PER_FILE, MAX_FONT_FILE_BYTES, UserFontError, decode_owned};

/// The catalogue's file in the user font folder.
pub(super) const CATALOG_FILE: &str = ".catalog.json";

/// Bump when what an entry holds, or how it is worked out, changes.
const FORMAT: u32 = 2;

/// The largest catalogue read (a face takes a few hundred bytes; 1,000 fonts of 256 faces each
/// would be about 80 MB).
const MAX_CATALOG_BYTES: u64 = 128 << 20;

/// A temporary catalogue this old belongs to a write that never finished (its process stopped).
const ABANDONED: Duration = Duration::from_secs(60 * 60);

/// What tells a file's contents apart without reading them: its size, its modification time, and
/// where the platform offers it (Unix), what changes when the file is replaced or rewritten even
/// when both are kept (as a restore can keep them). An entry describes the file only while all of
/// them match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stamp {
    len: u64,
    /// Seconds from 1970 (negative before) and the nanoseconds past them.
    secs: i64,
    nanos: u32,
    /// The inode (Unix).
    node: u64,
    /// The status-change time (Unix), which no program can set back; the creation time (Windows).
    changed: i64,
    changed_nanos: u32,
}

impl Stamp {
    /// The stamp of the file `meta` describes (`None` where the platform keeps no modification
    /// time: such a file is read and checked at every start).
    pub(super) fn of(meta: &std::fs::Metadata) -> Option<Stamp> {
        let (secs, nanos) = match meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => (i64::try_from(d.as_secs()).ok()?, d.subsec_nanos()),
            Err(e) => (i64::try_from(e.duration().as_secs()).ok()?.checked_neg()?, e.duration().subsec_nanos()),
        };
        let (node, changed, changed_nanos) = identity(meta);
        Some(Stamp { len: meta.len(), secs, nanos, node, changed, changed_nanos })
    }

    fn of_path(path: &Path) -> Option<Stamp> {
        std::fs::metadata(path).ok().as_ref().and_then(Stamp::of)
    }
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> (u64, i64, u32) {
    use std::os::unix::fs::MetadataExt;
    (meta.ino(), meta.ctime(), u32::try_from(meta.ctime_nsec()).unwrap_or(0))
}

/// Windows offers no inode or status-change time without a handle, and the creation time doesn't
/// change when a file is rewritten in place, nor always when it is replaced (file-system tunnelling
/// can give a file saved under the old one's name within seconds its creation time). So a file
/// changed keeping its size and modification time can keep its whole stamp: its faces, whether
/// listed from the catalogue or read at this start, are checked against it when first handed out.
#[cfg(windows)]
fn identity(meta: &std::fs::Metadata) -> (u64, i64, u32) {
    use std::os::windows::fs::MetadataExt;
    (0, i64::try_from(meta.creation_time()).unwrap_or(i64::MAX), 0)
}

#[cfg(not(any(unix, windows)))]
fn identity(_: &std::fs::Metadata) -> (u64, i64, u32) {
    (0, 0, 0)
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
    /// The record of `face` (a face made from the whole file, or listed from a record).
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

/// What one usable file in the folder holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// The file's name as text (exactly that name when it is valid Unicode).
    pub(super) file: String,
    /// The file's exact name when it isn't valid Unicode (its bytes, Unix), so that an entry stands
    /// for one file only, never for another whose name reads the same once made text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw: Option<Vec<u8>>,
    pub(super) stamp: Stamp,
    pub(super) faces: Vec<FaceRecord>,
}

impl Entry {
    /// The entry for the file named `name`: `None` when the name can't be kept exactly (a name
    /// that isn't valid Unicode, outside Unix), and the file is read at every start.
    pub(super) fn new(name: &OsStr, stamp: Stamp, faces: Vec<FaceRecord>) -> Option<Entry> {
        let raw = match name.to_str() {
            Some(_) => None,
            None => Some(name_bytes(name)?),
        };
        Some(Entry { file: name.to_string_lossy().into_owned(), raw, stamp, faces })
    }

    /// The exact name of the file it describes.
    pub(super) fn name(&self) -> Option<OsString> {
        match &self.raw {
            None => Some(OsString::from(&self.file)),
            Some(bytes) => name_from_bytes(bytes),
        }
    }
}

/// Can a file named `name` be catalogued (see [`Entry::new`])?
pub(super) fn keeps(name: &OsStr) -> bool {
    name.to_str().is_some() || cfg!(unix)
}

#[cfg(unix)]
fn name_bytes(name: &OsStr) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    Some(name.as_bytes().to_vec())
}

#[cfg(not(unix))]
fn name_bytes(_: &OsStr) -> Option<Vec<u8>> {
    None
}

#[cfg(unix)]
fn name_from_bytes(bytes: &[u8]) -> Option<OsString> {
    use std::os::unix::ffi::OsStringExt;
    Some(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn name_from_bytes(_: &[u8]) -> Option<OsString> {
    None
}

#[derive(Serialize)]
struct CatalogOut<'a> {
    format: u32,
    version: &'a str,
    files: &'a [Entry],
}

#[derive(Deserialize)]
struct CatalogIn {
    format: u32,
    version: String,
    files: Vec<Entry>,
}

/// The catalogue kept in `dir`, by exact file name; empty when there is none, it can't be read,
/// its digest doesn't match (damaged or edited), or an earlier format or WordCraft version wrote it.
pub(super) fn read(dir: &Path) -> HashMap<OsString, Entry> {
    let mut text = Vec::new();
    let read = std::fs::File::open(dir.join(CATALOG_FILE)).and_then(|f| f.take(MAX_CATALOG_BYTES).read_to_end(&mut text));
    let Ok(_) = read else { return HashMap::new() };
    let parsed = unseal(&text)
        .ok_or_else(|| "its digest doesn't match".to_string())
        .and_then(|body| serde_json::from_slice::<CatalogIn>(body).map_err(|e| e.to_string()));
    match parsed {
        Ok(c) if c.format == FORMAT && c.version == env!("CARGO_PKG_VERSION") => {
            c.files.into_iter().filter(|e| e.faces.len() <= MAX_FACES_PER_FILE).filter_map(|e| Some((e.name()?, e))).collect()
        }
        Ok(_) => HashMap::new(),
        Err(e) => {
            log::warn!("the added fonts' catalogue in {} can't be used ({e}): reading the fonts again", dir.display());
            HashMap::new()
        }
    }
}

/// Keep `entries` as the catalogue in `dir`, replacing the old one whole: written to a temporary
/// file of this write's own, then renamed over the catalogue. A write never opens, reuses or
/// removes another writer's temporary file, so no writer can publish another's unfinished file.
pub(super) fn write(dir: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let body =
        serde_json::to_vec(&CatalogOut { format: FORMAT, version: env!("CARGO_PKG_VERSION"), files: entries }).map_err(std::io::Error::other)?;
    let (tmp, mut file) = own_temporary(dir)?;
    let written = file.write_all(format!("{:016x}\n", seal(&body)).as_bytes()).and_then(|()| file.write_all(&body));
    drop(file);
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, dir.join(CATALOG_FILE))) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// A new temporary file for one catalogue write (`.catalog.json.<process>-<n>.part`), created
/// afresh: never opened through a name that is there already.
fn own_temporary(dir: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut taken = None;
    for _ in 0..16 {
        let tmp = dir.join(format!("{CATALOG_FILE}.{}-{}.part", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            // Left by an earlier process that had the same id: another name.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => taken = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(taken.unwrap_or_else(|| std::io::Error::other("no free temporary name")))
}

/// Is `path` a temporary catalogue whose write never finished (left for [`ABANDONED`])?
pub(super) fn abandoned(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else { return false };
    name.starts_with(CATALOG_FILE)
        && name.len() > CATALOG_FILE.len()
        && name.ends_with(".part")
        && std::fs::symlink_metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > ABANDONED)
}

/// A digest of `body`, enough to notice a damaged or edited catalogue (64-bit FNV-1a).
fn seal(body: &[u8]) -> u64 {
    body.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The catalogue `text` without its first line, when that line is the digest of the rest.
fn unseal(text: &[u8]) -> Option<&[u8]> {
    let (head, body) = text.split_first_chunk::<17>()?;
    let digest = std::str::from_utf8(&head[..16]).ok().and_then(|h| u64::from_str_radix(h, 16).ok())?;
    (head[16] == b'\n' && digest == seal(body)).then_some(body)
}

/// Give the catalogue in `dir` the digest of what it holds now, as though it had been written so
/// (tests: a catalogue that is whole but describes its files wrongly).
#[cfg(test)]
pub(super) fn reseal(dir: &Path) {
    let path = dir.join(CATALOG_FILE);
    let text = std::fs::read(&path).unwrap();
    let body = &text[17..];
    let mut sealed = format!("{:016x}\n", seal(body)).into_bytes();
    sealed.extend_from_slice(body);
    std::fs::write(&path, sealed).unwrap();
}

/// Read the font file at `path` whole and check it as an added font is checked
/// ([`check_faces`](super::check_faces)): its faces, or why it can't be used, or an error reading
/// it now.
pub(super) fn scan(path: &Path, shared: &FileShared) -> std::io::Result<Result<Vec<FaceRecord>, String>> {
    shared.count_read();
    let bytes = std::fs::read(path)?;
    Ok(decode_owned(bytes)
        .and_then(|sfnt| super::check_faces(FontBytes::Owned(Arc::new(sfnt))))
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
    /// The stamp it had when it was listed: the bytes are read only while it still has it.
    stamp: Option<Stamp>,
    /// Its bytes once read; empty when it couldn't be read.
    bytes: OnceLock<Vec<u8>>,
    /// Were the faces listed for it checked against its bytes (or made from them, by a start that
    /// read it)?
    checked: AtomicBool,
    /// The check found them unlike the file: they were replaced by faces made from its bytes.
    unfit: AtomicBool,
    /// Held while the file is checked, so it is checked once.
    checking: Mutex<()>,
    shared: Arc<FileShared>,
}

impl FontFile {
    /// `checked`: the faces listed for it were made from what a read of it found at this start.
    pub(super) fn new(path: PathBuf, stamp: Option<Stamp>, shared: Arc<FileShared>, checked: bool) -> FontFile {
        FontFile {
            path,
            stamp,
            bytes: OnceLock::new(),
            checked: AtomicBool::new(checked),
            unfit: AtomicBool::new(false),
            checking: Mutex::new(()),
            shared,
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn stamp(&self) -> Option<Stamp> {
        self.stamp
    }

    /// Do the faces listed for the file fit it? The first call runs `check` (which reads the file
    /// and answers that), with other callers waiting for it; later calls give its answer.
    pub(super) fn listed_faces_fit(&self, check: impl FnOnce() -> bool) -> bool {
        if !self.checked.load(Ordering::Acquire) {
            let _one = self.checking.lock().unwrap_or_else(PoisonError::into_inner);
            if !self.checked.load(Ordering::Acquire) {
                self.unfit.store(!check(), Ordering::Release);
                self.checked.store(true, Ordering::Release);
            }
        }
        !self.unfit.load(Ordering::Acquire)
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

    /// The file, if it is still the one it was when listed, as plain OpenType bytes.
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
