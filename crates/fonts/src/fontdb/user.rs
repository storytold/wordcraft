//! Fonts an end user adds at run time, without rebuilding (#582).
//!
//! The bundled faces (and the system fonts) stay the baseline: a user font only adds faces, never
//! replaces one (a face whose family and style are already loaded is skipped), so documents that
//! don't name it keep their metrics. A file is checked before anything is added: its size is
//! capped, WOFF and WOFF2 are unwrapped, and every face must parse and map characters.
//!
//! Desktop: [`FontDb::load_user_fonts`] names a folder (the app's `fonts` folder); the fonts there
//! load at start-up and each font added later is copied there (unwrapped from WOFF/WOFF2). Without
//! a folder (the web, tests, `WORDCRAFT_NO_PREFS`) added fonts last for the session; the web
//! shell keeps them in the browser itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use skrifa::MetadataProvider;
use skrifa::raw::FileRef;

use super::{FontBytes, FontDb, enumerate_faces, make_face};

/// Largest font file accepted (as given: TTF, OTF, TTC, WOFF or WOFF2).
pub const MAX_FONT_FILE_BYTES: usize = 50 << 20;
/// Largest font a WOFF/WOFF2 file may unwrap to.
const MAX_DECODED_BYTES: usize = 128 << 20;
/// Most faces one file may add (collections, variable fonts' named instances).
const MAX_FACES_PER_FILE: usize = 256;
/// Most user fonts at once.
const MAX_USER_FONTS: usize = 1000;
/// File extensions [`FontDb::load_user_fonts`] reads (fonts are stored unwrapped).
const STORED_EXTS: &[&str] = &["ttf", "otf", "ttc", "otc"];
/// Extensions a font file the user picks or drops may have.
pub const FONT_FILE_EXTS: &[&str] = &["ttf", "otf", "ttc", "otc", "woff", "woff2"];

/// Is `name` (a file name or path) named like a font file?
pub fn is_font_file_name(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    ext.is_some_and(|e| FONT_FILE_EXTS.contains(&e.as_str()))
}

/// Why a font couldn't be added or removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserFontError {
    Empty,
    TooLarge {
        bytes: usize,
    },
    /// Not a font file this reads (TrueType, OpenType, collections, WOFF, WOFF2).
    NotAFont,
    /// A WOFF/WOFF2 file that couldn't be unwrapped.
    BadWebFont(String),
    /// A font file that doesn't parse, or has no usable face.
    Unusable(String),
    /// Every face in the file is already available.
    AlreadyAvailable(Vec<String>),
    TooMany,
    /// The family isn't one the user added (bundled and system fonts can't be removed).
    NotUserFont(String),
}

impl std::fmt::Display for UserFontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserFontError::Empty => write!(f, "the font file is empty"),
            UserFontError::TooLarge { bytes } => {
                write!(f, "the font file is too large ({} MB; at most {} MB)", bytes.div_ceil(1 << 20), MAX_FONT_FILE_BYTES >> 20)
            }
            UserFontError::NotAFont => write!(f, "not a font file (TrueType, OpenType, TTC, WOFF or WOFF2)"),
            UserFontError::BadWebFont(e) => write!(f, "the web font couldn't be read: {e}"),
            UserFontError::Unusable(e) => write!(f, "the font couldn't be read: {e}"),
            UserFontError::AlreadyAvailable(families) => write!(f, "already available: {}", families.join(", ")),
            UserFontError::TooMany => write!(f, "too many added fonts (at most {MAX_USER_FONTS}); remove some first"),
            UserFontError::NotUserFont(family) => write!(f, "{family:?} isn't a font you added"),
        }
    }
}

impl std::error::Error for UserFontError {}

/// A font file the user added.
#[derive(Clone, Debug, PartialEq)]
pub struct UserFont {
    /// Its file name in the user font folder (unique among the user fonts), also the key the web
    /// shell stores it under.
    pub file: String,
    /// The faces it added: (typographic family, style).
    pub faces: Vec<(String, String)>,
    /// Where it is stored (desktop); `None` when it lasts for the session only.
    pub path: Option<PathBuf>,
    face_ids: Vec<u32>,
}

impl UserFont {
    /// The families it added (typographic names, in file order, each once).
    pub fn families(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for (family, _) in &self.faces {
            if !v.iter().any(|f| f.eq_ignore_ascii_case(family)) {
                v.push(family.clone());
            }
        }
        v
    }
}

/// What [`FontDb::install_font`] did.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub font: UserFont,
    /// The font was added for this session but couldn't be stored for the next one.
    pub warning: Option<String>,
}

#[derive(Default)]
pub(super) struct UserFonts {
    fonts: Vec<UserFont>,
    dir: Option<PathBuf>,
}

/// The kind of font file `bytes` holds, from its signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFormat {
    TrueType,
    OpenType,
    Collection,
    Woff,
    Woff2,
}

pub fn sniff(bytes: &[u8]) -> Option<FontFormat> {
    match bytes.get(..4)? {
        [0, 1, 0, 0] | b"true" | b"typ1" => Some(FontFormat::TrueType),
        b"OTTO" => Some(FontFormat::OpenType),
        b"ttcf" => Some(FontFormat::Collection),
        b"wOFF" => Some(FontFormat::Woff),
        b"wOF2" => Some(FontFormat::Woff2),
        _ => None,
    }
}

/// The font file in `bytes` as plain OpenType (TTF/OTF/TTC) bytes: WOFF and WOFF2 are unwrapped.
/// Checks the size and the signature only; [`FontDb::install_font`] checks the faces.
pub fn decode_font_file(bytes: &[u8]) -> Result<Vec<u8>, UserFontError> {
    if bytes.is_empty() {
        return Err(UserFontError::Empty);
    }
    if bytes.len() > MAX_FONT_FILE_BYTES {
        return Err(UserFontError::TooLarge { bytes: bytes.len() });
    }
    let decoded = match sniff(bytes).ok_or(UserFontError::NotAFont)? {
        FontFormat::TrueType | FontFormat::OpenType | FontFormat::Collection => return Ok(bytes.to_vec()),
        FontFormat::Woff => {
            check_woff1_size(bytes)?;
            unwrap_web_font(|| wuff::decompress_woff1(bytes))?
        }
        FontFormat::Woff2 => {
            let mut too_many_points = false;
            let unwrapped = unwrap_web_font(|| {
                wuff::decompress_woff2_with_custom_brotli(bytes, &mut |data, size| {
                    let stream = unbrotli(data, size)?;
                    too_many_points = !woff2_points_fit(bytes, &stream);
                    if too_many_points { Err(TOO_MANY_POINTS.into()) } else { Ok(stream) }
                })
            });
            if too_many_points {
                return Err(UserFontError::BadWebFont(TOO_MANY_POINTS.into()));
            }
            unwrapped?
        }
    };
    if decoded.len() > MAX_DECODED_BYTES {
        return Err(UserFontError::TooLarge { bytes: decoded.len() });
    }
    match sniff(&decoded) {
        Some(FontFormat::TrueType | FontFormat::OpenType | FontFormat::Collection) => Ok(decoded),
        _ => Err(UserFontError::BadWebFont("it doesn't hold a TrueType or OpenType font".into())),
    }
}

/// WOFF 1.0: the font its table directory describes must fit [`MAX_DECODED_BYTES`]. The decoder
/// sizes each table's buffer from the directory's `origLength` before inflating it (#3703).
/// A short header or directory is left to the decoder to report.
fn check_woff1_size(bytes: &[u8]) -> Result<(), UserFontError> {
    let u32_at = |at: usize| bytes.get(at..at.saturating_add(4)).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
    let tables = bytes.get(12..14).and_then(|n| n.try_into().ok()).map_or(0, |n| usize::from(u16::from_be_bytes(n)));
    let mut total = 12 + 16 * tables;
    for i in 0..tables {
        let entry = 44 + 20 * i;
        let (Some(compressed), Some(orig)) = (u32_at(entry + 8), u32_at(entry + 12)) else { break };
        total = total.saturating_add((compressed.max(orig) as usize).saturating_add(3) & !3);
    }
    if total > MAX_DECODED_BYTES {
        return Err(UserFontError::TooLarge { bytes: total });
    }
    Ok(())
}

/// Why a WOFF2 file whose glyph has more points than a glyph may is refused.
const TOO_MANY_POINTS: &str = "a glyph has more than 65536 points";

/// WOFF2's brotli stream, which must decompress to exactly `size` bytes (the decoder has checked
/// `size` against its own cap and the compression ratio).
fn unbrotli(data: &[u8], size: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(size);
    brotli_decompressor::Decompressor::new(data, 4096).take(size as u64 + 1).read_to_end(&mut out)?;
    if out.len() != size {
        return Err("the compressed data doesn't have the size the directory gives".into());
    }
    Ok(out)
}

/// Does every glyph in the WOFF2 file's transformed `glyf` tables have at most 65536 points, as
/// the decoder requires? It allocates a glyph's points from its counts before checking them, so a
/// small file could claim gigabytes (#3703). `stream` is the decompressed table data; anything
/// that doesn't parse is refused too (the decoder would refuse it).
fn woff2_points_fit(file: &[u8], stream: &[u8]) -> bool {
    woff2_transformed_glyf(file, stream).is_some_and(|tables| tables.iter().all(|t| glyf_points_fit(t).unwrap_or(false)))
}

/// The transformed `glyf` tables in `stream`, found from the WOFF2 table directory in `file`
/// (tables follow each other in directory order).
fn woff2_transformed_glyf<'a>(file: &[u8], stream: &'a [u8]) -> Option<Vec<&'a [u8]>> {
    let tables = u16::from_be_bytes(file.get(12..14)?.try_into().ok()?);
    let (mut at, mut offset, mut glyf) = (48usize, 0usize, Vec::new());
    for _ in 0..tables {
        let flags = *file.get(at)?;
        at += 1;
        let tag: Option<&[u8]> = if flags & 63 == 63 {
            at += 4;
            Some(file.get(at - 4..at)?)
        } else {
            None
        };
        let is = |index: u8, name: &[u8]| flags & 63 == index || tag == Some(name);
        let (is_glyf, is_loca) = (is(10, b"glyf"), is(11, b"loca"));
        let orig = base128(file, &mut at)?;
        // glyf and loca are transformed at version 0, other tables at any other version.
        let transformed = if is_glyf || is_loca { flags >> 6 == 0 } else { flags >> 6 != 0 };
        let len = if transformed { base128(file, &mut at)? } else { orig };
        let end = offset.checked_add(len)?;
        if is_glyf && transformed {
            glyf.push(stream.get(offset..end)?);
        }
        offset = end;
    }
    Some(glyf)
}

/// WOFF2's UIntBase128.
fn base128(b: &[u8], at: &mut usize) -> Option<usize> {
    let mut v: u32 = 0;
    for i in 0..5 {
        let byte = *b.get(*at)?;
        *at += 1;
        if (i == 0 && byte == 0x80) || v >> 25 != 0 {
            return None;
        }
        v = v << 7 | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return usize::try_from(v).ok();
        }
    }
    None
}

/// Does every glyph of the transformed `glyf` table `t` have at most 65536 points?
fn glyf_points_fit(t: &[u8]) -> Option<bool> {
    let u32_at = |at: usize| t.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes).map(|v| v as usize);
    let glyphs = usize::from(u16::from_be_bytes(t.get(4..6)?.try_into().ok()?));
    let contours_end = 36usize.checked_add(u32_at(8)?)?;
    let contours = t.get(36..contours_end)?;
    let points = t.get(contours_end..contours_end.checked_add(u32_at(12)?)?)?;
    let mut at = 0;
    for g in 0..glyphs {
        // The decoder reads the count as unsigned and only 0xFFFF is a composite glyph (no points
        // here): 0x8000..=0xFFFE are that many contours, so they are counted, not read as negative.
        let n = u16::from_be_bytes(contours.get(2 * g..2 * g + 2)?.try_into().ok()?);
        let mut total = 0usize;
        for _ in 0..if n == 0xFFFF { 0 } else { n } {
            // 255UInt16
            let code = *points.get(at)?;
            at += 1;
            let mut byte = || {
                at += 1;
                points.get(at - 1).copied().map(usize::from)
            };
            total += match code {
                253 => byte()? << 8 | byte()?,
                254 => byte()? + 253 * 2,
                255 => byte()? + 253,
                v => usize::from(v),
            };
            if total > 1 << 16 {
                return Some(false);
            }
        }
    }
    Some(true)
}

/// Run a WOFF/WOFF2 decoder. The decoder checks its input, but it is third-party code reading
/// hostile bytes: a panic in it is caught (native) and reported as a damaged file.
fn unwrap_web_font(decode: impl FnOnce() -> Result<Vec<u8>, wuff::WuffErr>) -> Result<Vec<u8>, UserFontError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(UserFontError::BadWebFont(format!("{e:?}"))),
        Err(_) => Err(UserFontError::BadWebFont("damaged file".into())),
    }
}

/// A file name for a user font from the name it came with: letters, digits, spaces, `-` and `_`
/// of its stem (others become `_`, at most 64), and the extension of its unwrapped format.
fn stored_name(name: &str, sfnt: &[u8]) -> String {
    // The last component, whichever separator the name came with.
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    let mut clean: String = stem.chars().take(64).map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    clean = clean.trim().to_string();
    if clean.is_empty() || clean.starts_with('-') {
        clean = format!("font{clean}");
    }
    let ext = match sniff(sfnt) {
        Some(FontFormat::OpenType) => "otf",
        Some(FontFormat::Collection) => "ttc",
        _ => "ttf",
    };
    format!("{clean}.{ext}")
}

impl FontDb {
    /// A counter that changes whenever user fonts are added or removed: a UI caching the font
    /// list or a layout redraws when it moves.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn read_user(&self) -> std::sync::RwLockReadGuard<'_, UserFonts> {
        self.user.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_user(&self) -> std::sync::RwLockWriteGuard<'_, UserFonts> {
        self.user.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The fonts the user added, in the order added.
    pub fn user_fonts(&self) -> Vec<UserFont> {
        self.read_user().fonts.clone()
    }

    /// The folder user fonts are stored in, if any ([`FontDb::load_user_fonts`]).
    pub fn user_font_dir(&self) -> Option<PathBuf> {
        self.read_user().dir.clone()
    }

    /// Add the font file `bytes` (TTF, OTF, TTC, WOFF or WOFF2) named `name` (its file name, for
    /// the stored copy). Every face is checked first; faces whose family and style are already
    /// available are skipped (the bundled baseline is never replaced). With a user font folder the
    /// font is stored there too, for the next start.
    pub fn install_font(&self, name: &str, bytes: &[u8]) -> Result<Installed, UserFontError> {
        let sfnt = decode_font_file(bytes)?;
        let mut font = self.add_user_faces(name, sfnt.clone(), None)?;
        let warning = match self.user_font_dir() {
            Some(dir) => match write_unique(&dir, &font.file, &sfnt) {
                Ok(path) => {
                    font.file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(font.file);
                    font.path = Some(path);
                    None
                }
                Err(e) => {
                    // Debug-quoted: a line break in the supplied name must not start a forged log record.
                    log::warn!("couldn't store the font {name:?} in {}: {e}", dir.display());
                    Some(format!("added for this session only: couldn't store it in {}: {e}", dir.display()))
                }
            },
            None => None,
        };
        if let Some(f) = self.write_user().fonts.iter_mut().find(|f| f.face_ids == font.face_ids) {
            f.file.clone_from(&font.file);
            f.path.clone_from(&font.path);
        }
        Ok(Installed { font, warning })
    }

    /// Add the fonts a document embeds (#1171) for this run: not recorded as user fonts, never
    /// stored. Faces already available are kept. Lookups made while the family was missing are
    /// forgotten ([`FontDb::generation`]), so text set in it now resolves to it. Each file comes
    /// with the name the document's font table gives it, which its faces answer to as well: Word
    /// embeds a subset ("embed only the characters used") under a made-up family
    /// (`___WRD_EMBED_SUB_46`), and the text asks for it by the table's name (#2240). Returns how
    /// many faces were added.
    pub fn add_document_fonts(&self, files: Vec<(Vec<u8>, String)>) -> usize {
        let added = self.add_fonts(files.into_iter().map(|(data, name)| (data, Some(name))).collect());
        if added > 0 {
            self.fonts_changed();
        }
        added
    }

    /// Check the faces in `sfnt` and add the new ones, recording them as a user font.
    fn add_user_faces(&self, name: &str, sfnt: Vec<u8>, path: Option<PathBuf>) -> Result<UserFont, UserFontError> {
        if self.read_user().fonts.len() >= MAX_USER_FONTS {
            return Err(UserFontError::TooMany);
        }
        if FileRef::new(&sfnt).is_err() {
            return Err(UserFontError::Unusable("not a valid OpenType file".into()));
        }
        let found = enumerate_faces(&sfnt, MAX_FACES_PER_FILE + 1);
        if found.is_empty() {
            return Err(UserFontError::Unusable("no face with a family name".into()));
        }
        if found.len() > MAX_FACES_PER_FILE {
            return Err(UserFontError::Unusable(format!("more than {MAX_FACES_PER_FILE} faces")));
        }
        let file = stored_name(name, &sfnt);
        let data = Arc::new(sfnt);
        let mut new = Vec::new();
        for f in found {
            let label = format!("{} {}", f.family, f.style);
            let face = make_face(FontBytes::Owned(data.clone()), f.index, f.family, f.style, f.legacy, f.coords)
                .ok_or_else(|| UserFontError::Unusable(format!("{label} doesn't parse")))?;
            if face.skrifa().is_none_or(|s| s.charmap().mappings().next().is_none()) {
                return Err(UserFontError::Unusable(format!("{label} maps no characters")));
            }
            new.push(Arc::new(face));
        }
        let families = |faces: &[Arc<super::FontFace>]| {
            let mut v: Vec<String> = faces.iter().map(|f| f.family.clone()).collect();
            v.dedup();
            v
        };
        let mut user = self.write_user();
        // Unique among the user fonts (the folder is checked when writing).
        let file = unique_name(&file, |n| user.fonts.iter().any(|f| f.file.eq_ignore_ascii_case(n)));
        let added: Vec<Arc<super::FontFace>> = {
            let mut faces = self.faces.write().unwrap_or_else(|e| e.into_inner());
            let fresh: Vec<_> = new
                .iter()
                .filter(|f| !faces.iter().any(|g| g.family.eq_ignore_ascii_case(&f.family) && g.style.eq_ignore_ascii_case(&f.style)))
                .cloned()
                .collect();
            faces.extend(fresh.iter().cloned());
            fresh
        };
        if added.is_empty() {
            return Err(UserFontError::AlreadyAvailable(families(&new)));
        }
        let font = UserFont {
            file,
            faces: added.iter().map(|f| (f.family.clone(), f.style.clone())).collect(),
            path,
            face_ids: added.iter().map(|f| f.id).collect(),
        };
        user.fonts.push(font.clone());
        drop(user);
        self.fonts_changed();
        Ok(font)
    }

    /// Forget cached fallback choices and tell watchers ([`FontDb::generation`]).
    fn fonts_changed(&self) {
        self.fallbacks.write().unwrap_or_else(|e| e.into_inner()).map.clear();
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Remove the user fonts that added `family` (a typographic or legacy family name): their
    /// faces leave the font list and their stored files are deleted. Text set in the family falls
    /// back as for any font that isn't installed. Bundled and system fonts can't be removed.
    pub fn remove_user_font(&self, family: &str) -> Result<Vec<UserFont>, UserFontError> {
        let mut user = self.write_user();
        let ids: Vec<u32> = {
            let faces = self.read_faces();
            faces.iter().filter(|f| f.named(family)).map(|f| f.id).collect()
        };
        let (gone, kept): (Vec<UserFont>, Vec<UserFont>) = std::mem::take(&mut user.fonts)
            .into_iter()
            .partition(|u| u.faces.iter().any(|(f, _)| f.eq_ignore_ascii_case(family)) || u.face_ids.iter().any(|i| ids.contains(i)));
        user.fonts = kept;
        drop(user);
        if gone.is_empty() {
            return Err(UserFontError::NotUserFont(family.to_string()));
        }
        let drop_ids: Vec<u32> = gone.iter().flat_map(|u| u.face_ids.iter().copied()).collect();
        self.faces.write().unwrap_or_else(|e| e.into_inner()).retain(|f| !drop_ids.contains(&f.id));
        for u in &gone {
            if let Some(p) = &u.path
                && let Err(e) = std::fs::remove_file(p)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("couldn't delete the font file {}: {e}", p.display());
            }
        }
        self.fonts_changed();
        Ok(gone)
    }

    /// Use `dir` as the user font folder: load the fonts stored there (each checked like an added
    /// font; one that fails is logged and skipped) and store fonts added later there. The folder is
    /// made when the first font is stored. Returns the fonts loaded and the files skipped (with
    /// why).
    pub fn load_user_fonts(&self, dir: &Path) -> (Vec<UserFont>, Vec<(String, String)>) {
        self.write_user().dir = Some(dir.to_path_buf());
        let mut loaded = Vec::new();
        let mut skipped = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else { return (loaded, skipped) };
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| p.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|e| STORED_EXTS.contains(&e.to_ascii_lowercase().as_str())))
            .collect();
        paths.sort();
        for p in paths {
            let file = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let read = match std::fs::metadata(&p) {
                Ok(m) if m.len() > MAX_FONT_FILE_BYTES as u64 => {
                    Err(UserFontError::TooLarge { bytes: usize::try_from(m.len()).unwrap_or(usize::MAX) })
                }
                Ok(_) => std::fs::read(&p).map_err(|e| UserFontError::Unusable(e.to_string())),
                Err(e) => Err(UserFontError::Unusable(e.to_string())),
            };
            let r = read.and_then(|b| decode_font_file(&b)).and_then(|sfnt| self.add_user_faces(&file, sfnt, Some(p.clone())));
            match r {
                Ok(mut f) => {
                    // Its own file name, so removing it deletes this file.
                    if let Some(u) = self.write_user().fonts.iter_mut().find(|u| u.face_ids == f.face_ids) {
                        u.file.clone_from(&file);
                    }
                    f.file = file;
                    loaded.push(f);
                }
                Err(e) => {
                    log::warn!("skipped the user font {}: {e}", p.display());
                    skipped.push((file, e.to_string()));
                }
            }
        }
        (loaded, skipped)
    }
}

/// `name`, or `name` with `-2`, `-3`… before its extension, the first that `taken` refuses.
fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..10_000).map(|n| format!("{stem}-{n}.{ext}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{stem}-x.{ext}"))
}

/// Write `bytes` to a new file in `dir` named `name` (or a numbered variant when that exists),
/// through a temporary file so a crash never leaves half a font.
fn write_unique(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    // One store at a time: two adds racing for the same name would otherwise pick the same file
    // and unlink each other's temporary. ponytail: in-process only; two app instances adding the
    // same name at the same instant can still collide (the later rename wins).
    static STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = STORE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    std::fs::create_dir_all(dir)?;
    let name = unique_name(name, |n| dir.join(n).exists());
    let path = dir.join(&name);
    let tmp = dir.join(format!(".{name}.part"));
    // Created afresh, never opened through what is there: a link planted at the temporary name
    // would otherwise carry the write to a file outside the folder. A leftover from a crash (or
    // such a link) is unlinked, not followed, and the create tried once more.
    let create = || std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp);
    let mut file = match create() {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(&tmp)?;
            create()?
        }
        r => r?,
    };
    let written = std::io::Write::write_all(&mut file, bytes);
    drop(file);
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(path)
}

#[cfg(test)]
pub(crate) mod tests;
