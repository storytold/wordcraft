//! File format dispatch by extension. Format crates plug in here.

use wordcraft_doc::Document;

#[cfg(not(target_arch = "wasm32"))]
mod safe_save;

/// Formats WordCraft opens.
pub const OPEN_EXTS: &[&str] = &[
    "docx",
    "docm",
    "dotx",
    "dotm",
    "doc",
    "dot",
    "txt",
    "md",
    "markdown",
    "html",
    "htm",
    "rtf",
    "odt",
    "tex",
    "latex",
    "ltx",
    "xml",
    "wcraft.json",
    "json",
];
/// Formats WordCraft saves (Save As).
pub const SAVE_EXTS: &[&str] = &["docx", "docm", "dotx", "dotm", "pdf", "txt", "md", "html", "rtf", "odt", "tex", "png", "json", "xml"];

/// Magic number for ZIP package signatures (used by .docx/.docm/.dotx/.dotm).
const ZIP_SIG: &[u8] = b"PK\x03\x04";
/// Magic number for OLE compound document signatures (used by .doc/.dot).
const OLE_SIG: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// The extension of a file name or path, lower-case; empty when the file name has none (a dot
/// in a folder name doesn't count).
fn ext_of(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default()
}

/// Is `ext` one of Word's package formats (.docx/.docm/.dotx/.dotm), which save without loss?
pub fn is_word_package(ext: &str) -> bool {
    wordcraft_docx::Flavor::from_ext(ext).is_some()
}

/// How a copy in another format is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    /// Print Markup: PDFs show tracked changes marked up and, with `comments`, each page scaled
    /// down beside a markup area with the comments in balloons (page images get the area too);
    /// HTML, Markdown and text copies end with the comments.
    pub markup: bool,
    /// With `markup`, show the comments (Track Changes Options › Comments).
    pub comments: bool,
    /// An `.xml` copy is a Word 2003 XML Document (WordprocessingML 2003) instead of a Word XML
    /// Document (Flat OPC): `format: "wordml2003"` on the save commands.
    pub word2003: bool,
}

/// A document password: wiped from memory when dropped, never printed (`Debug` shows `***`).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Password(zeroize::Zeroizing<String>);

impl Password {
    pub fn new(s: &str) -> Password {
        Password(zeroize::Zeroizing::new(s.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// The text, for a password box to edit in place.
    pub fn as_mut_string(&mut self) -> &mut String {
        &mut self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(***)")
    }
}

/// Does this open error mean the file is password-protected and no password was given?
pub fn needs_password(err: &str) -> bool {
    err.contains(wordcraft_docx::PASSWORD_REQUIRED)
}

/// Does this open error mean the password given was wrong?
pub fn wrong_password(err: &str) -> bool {
    err.contains(wordcraft_docx::WRONG_PASSWORD)
}

/// Is the file at `path` empty, zero bytes long? File Explorer's New › Microsoft Word Document
/// makes one, and `file.open` opens it as a new blank document (#1384). False for a file that
/// can't be read: opening it says why.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_empty_file(path: &std::path::Path) -> bool {
    crate::file_access::check_read(&path.to_string_lossy()).is_ok() && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == 0)
}

/// [`is_empty_file`]: on the web files come through the browser, never by path.
#[cfg(target_arch = "wasm32")]
pub fn is_empty_file(_path: &std::path::Path) -> bool {
    false
}

/// Can files with this extension be saved with a password (Word's packages)?
pub fn can_encrypt(name: &str) -> bool {
    is_word_package(&ext_of(name))
}

/// Finish a document just read from a file: never leave it empty, and bring its unlocked
/// `DATE`/`TIME` fields up to now the way Word does on open (see
/// [`crate::cmd::references::refresh_date_fields`]).
fn opened(doc: &mut Document) {
    doc.ensure_nonempty();
    crate::cmd::references::refresh_date_fields(doc);
}

/// Parse a document from bytes; `name` gives the format by extension.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Document, String> {
    open_bytes_with(name, bytes, None).map(|(doc, _)| doc)
}

/// Open a Word 97-2003 binary file, decrypting it with `password` when it is password-protected
/// (XOR, RC4, RC4 CryptoAPI). The errors for a missing or wrong password are the same texts the
/// `.docx` reader gives, so [`needs_password`] and [`wrong_password`] recognise them.
fn open_binary_doc(bytes: &[u8], password: Option<&str>) -> Result<(Document, bool), String> {
    use wordcraft_docbin::DocbinError;
    let encrypted = wordcraft_docbin::is_encrypted(bytes);
    match wordcraft_docbin::read_with_password(bytes, password) {
        Ok(doc) => Ok((doc, encrypted)),
        Err(DocbinError::PasswordRequired) => Err(wordcraft_docx::PASSWORD_REQUIRED.to_string()),
        Err(DocbinError::WrongPassword) => Err(wordcraft_docx::WRONG_PASSWORD.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// [`open_bytes`] for a file that may be password-protected: a Word document encrypted with a
/// password ([MS-OFFCRYPTO] agile encryption, or the Word 97-2003 schemes) is decrypted with
/// `password` first. Also says
/// whether the file was encrypted. Without the password needed the error satisfies
/// [`needs_password`]; with a wrong one, [`wrong_password`].
pub fn open_bytes_with(name: &str, bytes: &[u8], password: Option<&str>) -> Result<(Document, bool), String> {
    open_bytes_in(name, bytes, password, &wordcraft_geom::Region::default())
}

/// The page a file that gives no page size (plain text, Markdown, HTML) opens on: `region`'s
/// paper, as a new blank document (#838, #1381), with the default margins.
pub fn region_page(region: &wordcraft_geom::Region) -> wordcraft_doc::SectionProps {
    let (_, w, h) = region.paper();
    wordcraft_doc::SectionProps { page_w: w, page_h: h, ..Default::default() }
}

/// Settings when opening a document.
#[derive(Clone, Copy, Debug)]
pub struct DocOpenSettings {
    /// Automatically convert legacy Bijoy / ANSI documents to Unicode on open (default: `true`).
    pub convert_bijoy: bool,
}

impl Default for DocOpenSettings {
    fn default() -> Self {
        DocOpenSettings { convert_bijoy: true }
    }
}

/// [`open_bytes_in`] with custom settings (e.g. Bijoy conversion preferences).
pub fn open_bytes_opts(
    name: &str,
    bytes: &[u8],
    password: Option<&str>,
    region: &wordcraft_geom::Region,
    settings: &DocOpenSettings,
) -> Result<(Document, bool), String> {
    let page = region_page(region);
    let (doc, encrypted) = read_bytes(name, bytes, password, &page, settings.convert_bijoy)?;
    load_embedded_fonts(&doc);
    Ok((doc, encrypted))
}

/// [`open_bytes_with`] in `region`: a file that gives no page size opens on the region's paper
/// (see [`region_page`]); a document with its own page size keeps it.
pub fn open_bytes_in(name: &str, bytes: &[u8], password: Option<&str>, region: &wordcraft_geom::Region) -> Result<(Document, bool), String> {
    open_bytes_opts(name, bytes, password, region, &DocOpenSettings::default())
}

/// Load the fonts `doc` embeds (#1171) for this run, in memory only: text set in a font that isn't
/// installed then lays out and exports with it instead of a fallback. Faces already available
/// stay as they are.
fn load_embedded_fonts(doc: &Document) {
    let files: Vec<Vec<u8>> = wordcraft_docx::embedded_fonts(doc).iter().filter_map(|f| wordcraft_fonts::decode_font_file(f).ok()).collect();
    if !files.is_empty() {
        wordcraft_fonts::FontDb::global().add_document_fonts(files);
    }
}

fn read_bytes(
    name: &str,
    bytes: &[u8],
    password: Option<&str>,
    page: &wordcraft_doc::SectionProps,
    convert_bijoy: bool,
) -> Result<(Document, bool), String> {
    let ext = ext_of(name);
    if matches!(ext.as_str(), "docx" | "docm" | "dotx" | "dotm" | "doc" | "dot") && wordcraft_docx::is_encrypted(bytes) {
        let mut doc = wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string())?;
        if convert_bijoy {
            normalize_bijoy_fonts(&mut doc);
        }
        opened(&mut doc);
        return Ok((doc, true));
    }
    if ext.is_empty() || signature_contradicts(&ext, bytes) {
        return open_by_content(name, &ext, bytes, password, page, convert_bijoy);
    }
    let (mut doc, encrypted) = match ext.as_str() {
        "doc" | "dot" => open_binary_doc(bytes, password)?,
        // Plain text goes through the formats importer: BOMs, unmarked UTF-16, Windows-1252.
        "txt" | "text" => (wordcraft_formats::txt::import_on(bytes, page), false),
        "json" => {
            let mut d = serde_json::from_slice::<Document>(bytes).map_err(|e| format!("{name}: {e}"))?;
            d.normalize_runs();
            (d, false)
        }
        other => match crate::io_ext::open(other, bytes, page) {
            Some(r) => (r?, false),
            None => return open_by_content(name, other, bytes, password, page, convert_bijoy),
        },
    };
    if convert_bijoy {
        normalize_bijoy_fonts(&mut doc);
    }
    opened(&mut doc);
    Ok((doc, encrypted))
}

/// The narrowest text width of the document's sections, points: what a picture must fit.
pub(crate) fn text_width(doc: &Document) -> f32 {
    doc.sections().iter().map(|(_, sp)| sp.text_width()).fold(f32::INFINITY, f32::min).clamp(36.0, 4000.0)
}

/// Shrink inline pictures wider than `max_w` to it, keeping their proportions. Layout keeps a
/// picture's size (as Word does), so formats whose pictures come at their pixel size (HTML,
/// Markdown, ODT, RTF) are fitted when they are read, as Insert › Pictures does.
pub(crate) fn fit_block_pictures(b: &mut wordcraft_doc::Block, max_w: f32) {
    crate::cmd::paste::each_para(b, 0, &mut |p| {
        let mut changed = false;
        for o in &mut p.objects {
            if let wordcraft_doc::InlineObject::Image { w, h, float, .. } = o
                && float.wrap == wordcraft_doc::para::Wrap::Inline
                && w.is_finite()
                && *w > max_w
            {
                let k = max_w / *w;
                *w = max_w;
                *h = (*h * k).max(1.0);
                changed = true;
            }
        }
        if changed {
            p.touch();
        }
    });
}

/// [`fit_block_pictures`] for a whole document read from a file, against its own text width.
pub(crate) fn fit_pictures(doc: &mut Document) {
    let max_w = text_width(doc);
    for b in &mut doc.body {
        fit_block_pictures(std::sync::Arc::make_mut(b), max_w);
    }
}

/// Does the file hold a different document format than its Word or RTF extension says? An RTF
/// file saved as `.doc` (as accounting systems and report generators often save their
/// documents), a `.docx` package renamed `.doc` or a Word 97-2003 file renamed `.docx`: Word opens
/// each by what it holds, so they go through [`open_by_content`] instead of failing.
fn signature_contradicts(ext: &str, bytes: &[u8]) -> bool {
    let held = if bytes.starts_with(ZIP_SIG) {
        "zip"
    } else if bytes.starts_with(OLE_SIG) {
        "ole"
    } else if bytes.starts_with(b"{\\rtf") {
        "rtf"
    } else {
        return false;
    };
    let expected = match ext {
        "docx" | "docm" | "dotx" | "dotm" => "zip",
        "doc" | "dot" => "ole",
        "rtf" => "rtf",
        _ => return false,
    };
    held != expected
}

/// A file whose extension names no format WordCraft knows (`.m3u`, `.log`, `.ini`, none…),
/// opened by what it holds, as Word does: a Word document or RTF file by its signature, any
/// other text through the plain-text import with encoding detection (#569). Anything else is
/// refused with a clear error rather than shown as garbage.
fn open_by_content(
    name: &str,
    ext: &str,
    bytes: &[u8],
    password: Option<&str>,
    page: &wordcraft_doc::SectionProps,
    convert_bijoy: bool,
) -> Result<(Document, bool), String> {
    let (mut doc, encrypted) = if bytes.starts_with(OLE_SIG) && wordcraft_docx::is_encrypted(bytes) {
        (wordcraft_docx::read_with_password(bytes, password).map_err(|e| e.to_string())?, true)
    } else if bytes.starts_with(ZIP_SIG) {
        (wordcraft_docx::read(bytes).map_err(|e| format!("{name}: not a document WordCraft can open ({e})"))?, false)
    } else if bytes.starts_with(OLE_SIG) {
        open_binary_doc(bytes, password)
            .map_err(|e| if needs_password(&e) || wrong_password(&e) { e } else { format!("{name}: not a document WordCraft can open ({e})") })?
    } else if bytes.starts_with(b"{\\rtf") {
        (wordcraft_formats::rtf::import(bytes)?, false)
    } else if wordcraft_formats::txt::is_text(bytes) {
        (wordcraft_formats::txt::import_on(bytes, page), false)
    } else if ext.is_empty() {
        return Err(format!("{name}: not a document or text file WordCraft can open"));
    } else {
        return Err(format!("{name}: unsupported format `.{ext}` (not a document or text file)"));
    };
    if convert_bijoy {
        normalize_bijoy_fonts(&mut doc);
    }
    opened(&mut doc);
    Ok((doc, encrypted))
}

/// Serialise a document; `name` gives the format by extension.
pub fn save_bytes(name: &str, doc: &Document) -> Result<Vec<u8>, String> {
    save_bytes_opts(name, doc, &ExportOptions::default())
}

/// [`save_bytes`] with export options (Print Markup).
pub fn save_bytes_opts(name: &str, doc: &Document, opts: &ExportOptions) -> Result<Vec<u8>, String> {
    let ext = ext_of(name);
    match ext.as_str() {
        // List paragraphs get their label and a tab, as in Word's plain-text save.
        "txt" | "text" => {
            let bytes = wordcraft_formats::txt::export(doc);
            Ok(if opts.markup && opts.comments { wordcraft_formats::comments::append("txt", doc, bytes) } else { bytes })
        }
        "json" => serde_json::to_vec_pretty(doc).map_err(|e| e.to_string()),
        other => match crate::io_ext::save(other, doc, opts) {
            Some(r) => r,
            None => Err(format!("{name}: can't save as `.{other}`")),
        },
    }
}

/// [`save_bytes`], encrypted with `password` when there is one and the format is a Word package
/// (other formats are copies and are written as they are).
pub fn save_bytes_with(name: &str, doc: &Document, password: Option<&str>, opts: &ExportOptions) -> Result<Vec<u8>, String> {
    let bytes = save_bytes_opts(name, doc, opts)?;
    match password {
        Some(pw) if can_encrypt(name) => wordcraft_docx::encrypt(&bytes, pw).map_err(|e| e.to_string()),
        _ => Ok(bytes),
    }
}

pub fn open_path(path: &std::path::Path) -> Result<Document, String> {
    open_path_with(path, None).map(|(doc, _)| doc)
}

/// [`open_path`] for a file that may be password-protected (see [`open_bytes_with`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_path_with(path: &std::path::Path, password: Option<&str>) -> Result<(Document, bool), String> {
    open_path_in(path, password, &wordcraft_geom::Region::default())
}

/// [`open_path_with`] in `region` (see [`open_bytes_in`]).
/// [`open_path_in`] with custom settings (e.g. Bijoy conversion preferences).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_path_opts(
    path: &std::path::Path,
    password: Option<&str>,
    region: &wordcraft_geom::Region,
    settings: &DocOpenSettings,
) -> Result<(Document, bool), String> {
    crate::file_access::check_read(&path.to_string_lossy())?;
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > 2 << 30 {
        return Err(format!("{}: file is larger than 2 GB", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.to_string_lossy();
    if matches!(ext_of(&name).as_str(), "html" | "htm" | "xhtml") {
        // Pictures referenced by a relative path load from beside the HTML file.
        let images = LocalImages::new(path.parent().unwrap_or(std::path::Path::new("")));
        let html = wordcraft_formats::html::decode(&bytes);
        let mut doc = wordcraft_formats::html::import_on(&html, &|src| images.load(src), &region_page(region));
        fit_pictures(&mut doc);
        opened(&mut doc);
        return Ok((doc, false));
    }
    open_bytes_opts(&name, &bytes, password, region, settings)
}

/// [`open_path_with`] in `region` (see [`open_bytes_in`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_path_in(path: &std::path::Path, password: Option<&str>, region: &wordcraft_geom::Region) -> Result<(Document, bool), String> {
    open_path_opts(path, password, region, &DocOpenSettings::default())
}

/// The pictures an HTML file names by a path relative to its folder.
///
/// Only regular files inside the folder (after resolving `..` and symlinks) are read. The folder is
/// the HTML file's own, which `open_path_with` has already checked against the automation read root. URLs
/// (`http:`, `file:`…) and absolute paths are not followed. Each file is read at most once and
/// at most `max_file` bytes of it; every reference, cached or not, counts against a budget for
/// the whole document, so one large picture can't be multiplied by referencing it many times.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct LocalImages {
    /// The folder, canonical; `None` when it can't be resolved (nothing loads then).
    dir: Option<std::path::PathBuf>,
    max_file: u64,
    /// Bytes the document may still load.
    left: std::cell::Cell<u64>,
    cache: std::cell::RefCell<Vec<(std::path::PathBuf, std::sync::Arc<Vec<u8>>)>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl LocalImages {
    /// Largest picture file read.
    const MAX_FILE: u64 = 256 << 20;
    /// Picture bytes one document may load, counting every reference.
    const BUDGET: u64 = 1 << 30;
    /// Files whose bytes are kept for further references.
    const CACHE: usize = 64;

    pub(crate) fn new(dir: &std::path::Path) -> LocalImages {
        LocalImages::with_limits(dir, LocalImages::MAX_FILE, LocalImages::BUDGET)
    }

    pub(crate) fn with_limits(dir: &std::path::Path, max_file: u64, budget: u64) -> LocalImages {
        let dir = if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir };
        LocalImages { dir: std::fs::canonicalize(dir).ok(), max_file, left: std::cell::Cell::new(budget), cache: Default::default() }
    }

    /// Bytes of the file `src` names, or `None` when it isn't a regular file inside the folder,
    /// is too large, or the budget is spent.
    pub(crate) fn load(&self, src: &str) -> Option<std::sync::Arc<Vec<u8>>> {
        let path = self.resolve(src)?;
        let cached = self.cache.borrow().iter().find(|(p, _)| *p == path).map(|(_, d)| d.clone());
        if let Some(data) = cached {
            self.charge(data.len())?;
            return Some(data);
        }
        let limit = self.max_file.min(self.left.get());
        let bytes = read_regular_file(&path, limit)?;
        // What was read is spent even when it's over the limit.
        self.charge(bytes.len())?;
        if bytes.len() as u64 > limit {
            return None;
        }
        let data = std::sync::Arc::new(bytes);
        let mut cache = self.cache.borrow_mut();
        if cache.len() < LocalImages::CACHE {
            cache.push((path, data.clone()));
        }
        Some(data)
    }

    /// Take `n` bytes from the budget; `None` (and the budget emptied) when there aren't enough.
    fn charge(&self, n: usize) -> Option<()> {
        let left = self.left.get().checked_sub(n as u64);
        self.left.set(left.unwrap_or(0));
        left.map(|_| ())
    }

    /// The canonical path `src` names, if it's inside the folder.
    fn resolve(&self, src: &str) -> Option<std::path::PathBuf> {
        let src = src.split(['#', '?']).next().unwrap_or("");
        if src.is_empty() || src.starts_with("//") || src.split_once(':').is_some_and(|(scheme, _)| !scheme.contains('/')) {
            return None;
        }
        let rel = String::from_utf8(wordcraft_formats::model::percent_decode(src)).ok()?;
        let rel = std::path::Path::new(&rel);
        if rel.components().any(|c| matches!(c, std::path::Component::Prefix(_) | std::path::Component::RootDir)) {
            return None;
        }
        let dir = self.dir.as_ref()?;
        // Canonical paths have `..` and symlinks resolved, so this keeps both inside the folder.
        let path = std::fs::canonicalize(dir.join(rel)).ok()?;
        path.starts_with(dir).then_some(path)
    }
}

/// Up to `limit + 1` bytes of the regular file at `path` (more than `limit` means too large), or
/// `None` for anything else. A named pipe blocks when opened and a device reports a length of 0
/// and may never end, so the path is checked before opening, the opened handle is checked again,
/// and the read is bounded whatever length the file reports.
#[cfg(not(target_arch = "wasm32"))]
fn read_regular_file(path: &std::path::Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let mut out = Vec::with_capacity(usize::try_from(meta.len().min(limit)).unwrap_or(0));
    file.take(limit.saturating_add(1)).read_to_end(&mut out).ok()?;
    Some(out)
}

/// The bytes of the file at `path`, for commands that take a path (pictures, mail-merge data).
/// Every such read goes through here so the automation read root applies ([`crate::file_access`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_file(path: &str) -> Result<Vec<u8>, String> {
    crate::file_access::check_read(path)?;
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub fn read_file(path: &str) -> Result<Vec<u8>, String> {
    Err(format!("{path}: files are opened through the browser on the web"))
}

/// [`read_file`] for a file that must be a regular file of at most `max` bytes, checked before
/// anything is read (a font named by the user, which could be any size).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_file_capped(path: &str, max: u64, what: &str) -> Result<Vec<u8>, String> {
    crate::file_access::check_read(path)?;
    let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path}: not a file"));
    }
    if meta.len() > max {
        return Err(format!("{path}: the {what} is larger than {} MB", max >> 20));
    }
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

/// The first `max` bytes of the file at `path` (fewer when it is shorter), inside the automation
/// read root when there is one: enough to tell a file's format by its start.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_file_head(path: &str, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    crate::file_access::check_read(path)?;
    let f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut head = Vec::new();
    f.take(max).read_to_end(&mut head).map_err(|e| format!("{path}: {e}"))?;
    Ok(head)
}

/// Rename without replacing another file, respecting both automation access roots.
#[cfg(not(target_arch = "wasm32"))]
pub fn rename_file(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    crate::file_access::check_read(&source.to_string_lossy())?;
    crate::file_access::check_write(&source.to_string_lossy())?;
    crate::file_access::check_write(&destination.to_string_lossy())?;
    // Creating a sibling hard link refuses collisions atomically and keeps the original bytes.
    if let Err(e) = std::fs::hard_link(source, destination) {
        if e.kind() == std::io::ErrorKind::AlreadyExists && only_case_differs(source, destination) {
            return std::fs::rename(source, destination).map_err(|e| format!("Could not rename document: {e}"));
        }
        return Err(format!("Could not rename document: {e}"));
    }
    if let Err(e) = std::fs::remove_file(source) {
        let _ = std::fs::remove_file(destination);
        return Err(format!("Could not rename document: {e}"));
    }
    Ok(())
}

/// Whether `destination` names `source` in the same directory spelled with different case, and
/// the directory holds no entry spelled exactly as `destination`.
#[cfg(not(target_arch = "wasm32"))]
fn only_case_differs(source: &std::path::Path, destination: &std::path::Path) -> bool {
    let (Some(dir), Some(from), Some(to)) = (source.parent(), source.file_name(), destination.file_name()) else {
        return false;
    };
    if destination.parent() != Some(dir) || from == to || from.to_string_lossy().to_lowercase() != to.to_string_lossy().to_lowercase() {
        return false;
    }
    let dir = if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir };
    std::fs::read_dir(dir).is_ok_and(|entries| entries.filter_map(Result::ok).all(|entry| entry.file_name() != to))
}

/// Write `bytes` to the file at `path`, inside the automation write root when there is one.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    crate::file_access::check_write(path)?;
    // Like a document save: the old file is replaced only once the new bytes are fully written.
    safe_save::write(std::path::Path::new(path), bytes).map_err(|e| format!("{path}: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub fn write_file(path: &str, _bytes: &[u8]) -> Result<(), String> {
    Err(format!("{path}: files are saved through the browser on the web"))
}

#[cfg(target_arch = "wasm32")]
pub fn open_path_with(path: &std::path::Path, _password: Option<&str>) -> Result<(Document, bool), String> {
    Err(format!("{}: files are opened through the browser on the web", path.display()))
}

#[cfg(target_arch = "wasm32")]
pub fn open_path_opts(
    path: &std::path::Path,
    password: Option<&str>,
    _region: &wordcraft_geom::Region,
    _settings: &DocOpenSettings,
) -> Result<(Document, bool), String> {
    open_path_with(path, password)
}

#[cfg(target_arch = "wasm32")]
pub fn open_path_in(path: &std::path::Path, password: Option<&str>, _region: &wordcraft_geom::Region) -> Result<(Document, bool), String> {
    open_path_with(path, password)
}

pub fn save_path(path: &std::path::Path, doc: &Document) -> Result<(), String> {
    save_path_with(path, doc, None, &ExportOptions::default())
}

/// [`save_path`], encrypted with `password` as [`save_bytes_with`] does.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_path_with(path: &std::path::Path, doc: &Document, password: Option<&str>, opts: &ExportOptions) -> Result<(), String> {
    crate::file_access::check_write(&path.to_string_lossy())?;
    let bytes = save_bytes_with(&path.to_string_lossy(), doc, password, opts)?;
    safe_save::write(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(target_arch = "wasm32")]
pub fn save_path_with(path: &std::path::Path, _doc: &Document, _password: Option<&str>, _opts: &ExportOptions) -> Result<(), String> {
    Err(format!("{}: files are saved through the browser on the web", path.display()))
}

/// Decode text bytes: UTF-8 (with or without BOM), UTF-16 LE/BE (BOM or sniffed), else the
/// legacy encoding they look like — GBK, Big5, Shift_JIS, Windows-1252… (see
/// [`wordcraft_formats::txt::decode`]).
pub fn decode_text(b: &[u8]) -> String {
    wordcraft_formats::txt::decode(b)
}

/// Known legacy Bijoy / ANSI Bengali font names that store glyph-encoded ASCII.
fn is_bijoy_font(font_name: &str) -> bool {
    let lower = font_name.trim().to_ascii_lowercase();
    lower.contains("sutonnymj")
        || lower.contains("sutonny")
        || lower.contains("siyam rupali ansi")
        || lower.contains("bijoy")
        || lower == "classic bangla"
        || lower.ends_with("mj")
}

/// Whether a font pair makes its text Bijoy. The Latin font decides when one is set, because
/// Bijoy keystrokes sit in the Latin range; the complex-script font decides only when no Latin
/// font is set.
fn font_pair_is_bijoy(latin: Option<&str>, complex: Option<&str>) -> bool {
    match latin {
        Some(font) => is_bijoy_font(font),
        None => complex.is_some_and(is_bijoy_font),
    }
}

fn char_props_has_bijoy(c: &wordcraft_doc::CharProps) -> bool {
    font_pair_is_bijoy(c.font.as_deref(), c.font_cs.as_deref())
}

fn normalize_bijoy_char_props(props: &mut wordcraft_doc::CharProps) {
    props.font = Some("Kalpurush".into());
    props.font_cs = Some("Kalpurush".into());
    // Kalpurush renders slightly larger than SutonnyMJ at the same nominal point size.
    // Reduce by 2 pt (clamped to a minimum of 6 pt) so converted documents look the
    // same as they did in Word / Bijoy.
    if props.size.is_none() {
        props.size = props.size_cs;
    }
    if let Some(sz) = props.size.as_mut() {
        *sz = (*sz - 2.0).max(6.0);
    }
    // Keep size_cs in sync with the adjusted size.
    props.size_cs = props.size;
    if props.bold.is_some() {
        props.bold_cs = props.bold;
    }
    if props.italic.is_some() {
        props.italic_cs = props.italic;
    }
}

fn is_sutonny_glyph(c: char) -> bool {
    matches!(c, '†' | '‡' | 'ˆ' | '‰' | 'Š' | '‹' | 'Œ' | '‚' | '„' | '•' | '›' | 'œ' | 'š' | 'Ÿ' | 'ƒ') || ('\u{00A1}'..='\u{00FF}').contains(&c)
}

/// A Bengali letter or sign; the Bengali digits (U+09E6–U+09EF) don't count, so a number with a
/// symbol (`২৫°C`, `১০০£`) is never taken for a half-converted word.
fn is_bn_letter(c: char) -> bool {
    ('\u{0980}'..='\u{09E5}').contains(&c) || ('\u{09F0}'..='\u{09FF}').contains(&c)
}

/// One word of Unicode Bangla that still holds SutonnyMJ from an earlier, partial conversion: a
/// Bengali letter together with a SutonnyMJ glyph in the same word, an unconverted pre-base `w`/`z`
/// against a Bengali letter, or a known broken sequence. Words are split at whitespace, so a
/// symbol standing apart (`© ২০২৪`, `½ কেজি`) or a Latin word (`café`) is ordinary text.
fn is_leftover_word(word: &str) -> bool {
    if !word.chars().any(is_bn_letter) {
        return false;
    }
    if word.chars().any(is_sutonny_glyph) || word.contains("šত্ন") || word.contains("¯ত্ন") || word.contains("চঁাদ") {
        return true;
    }
    let chars: Vec<char> = word.chars().collect();
    chars.iter().enumerate().any(|(i, ch)| {
        let near = |j: Option<usize>| j.and_then(|j| chars.get(j)).is_some_and(|c| is_bn_letter(*c));
        let malformed_cluster = matches!(ch, 'া' | 'ি' | 'ী' | 'ু' | 'ূ' | 'ৃ' | 'ে' | 'ৈ' | 'ো' | 'ৌ')
            && chars.get(i + 1) == Some(&'্')
            && chars.get(i + 2).copied().is_some_and(crate::cmd::bangla::is_bn_consonant);
        malformed_cluster || (matches!(ch, 'w' | 'z') && (near(i.checked_sub(1)) || near(i.checked_add(1))))
    })
}

/// Detects text that was partially converted in an earlier session: any of its words is a
/// leftover (see [`is_leftover_word`]).
fn has_leftover_bijoy(text: &str) -> bool {
    text.split_whitespace().any(is_leftover_word)
}

/// Repairs a partially converted Bengali string word by word: only the leftover words are decoded;
/// every other word (Unicode Bangla, English, numbers, symbols) and the whitespace stay as they are.
fn repair_partial_bijoy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let mut in_space = false;
    let flush = |token: &str, in_space: bool, out: &mut String| {
        if !in_space && is_leftover_word(token) {
            out.push_str(&crate::cmd::bangla::ansi_to_unicode(token));
        } else {
            out.push_str(token);
        }
    };
    for ch in text.chars() {
        let is_ws = ch.is_whitespace();
        if is_ws != in_space && !token.is_empty() {
            flush(&token, in_space, &mut out);
            token.clear();
        }
        in_space = is_ws;
        token.push(ch);
    }
    if !token.is_empty() {
        flush(&token, in_space, &mut out);
    }
    out
}

fn blocks_have_bijoy(blocks: &[std::sync::Arc<wordcraft_doc::Block>]) -> bool {
    for b in blocks {
        match &**b {
            wordcraft_doc::Block::Para(p) => {
                if char_props_has_bijoy(&p.mark) || p.runs.iter().any(|r| char_props_has_bijoy(&r.props)) || has_leftover_bijoy(&p.text) {
                    return true;
                }
            }
            wordcraft_doc::Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        if blocks_have_bijoy(&cell.blocks) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// The first character boundary at or after `at` (never past the text): a run length from a file can end
/// inside a character, and the run then takes the rest of that character.
fn char_boundary_at_or_after(text: &str, at: usize) -> usize {
    (at..=text.len()).find(|&i| text.is_char_boundary(i)).unwrap_or(text.len())
}

fn slice_safe(s: &str, start: usize, end: usize) -> &str {
    let end = end.min(s.len());
    let start = start.min(end);
    s.get(start..end).unwrap_or("")
}

/// Legacy Bijoy documents often align right-hand header fields (such as `তারিখ`) using spaces
/// plus multiple tabs plus long runs of spaces (`   \t\t\t                `). Because Unicode
/// `Kalpurush` glyphs and spaces are slightly wider than `SutonnyMJ`, trailing spaces before `\t`
/// can push a tab past a 36pt stop and long space runs after `\t` can wrap the date onto line 2.
fn compact_bijoy_tab_spaces(s: &str) -> String {
    if !s.contains('\t') {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    let mut after_tab = false;
    while i < chars.len() {
        let Some(&ch) = chars.get(i) else { break };
        if ch == ' ' {
            let mut j = i;
            while chars.get(j) == Some(&' ') {
                j += 1;
            }
            let space_count = j - i;
            if chars.get(j) == Some(&'\t') {
                // Strip spaces immediately preceding a tab stop.
                i = j;
                continue;
            }
            if after_tab && space_count >= 4 {
                let keep = (space_count / 4).max(1);
                for _ in 0..keep {
                    out.push(' ');
                }
                after_tab = false;
                i = j;
                continue;
            }
            for _ in 0..space_count {
                out.push(' ');
            }
            after_tab = false;
            i = j;
        } else if ch == '\t' {
            out.push('\t');
            after_tab = true;
            i += 1;
        } else {
            out.push(ch);
            after_tab = false;
            i += 1;
        }
    }
    out
}

/// A (Latin, complex-script) font pair; `None` where nothing sets that font.
type FontPair<'a> = (Option<&'a str>, Option<&'a str>);

/// What the open path needs to know about each style, worked out once per document with the
/// document model's rules (`StyleSheet::resolve_char`), so a paragraph or run is one lookup rather than
/// a walk up its style chain.
struct StyleFonts<'a> {
    /// The fonts a paragraph's text gets from its paragraph style: the paragraph styles in its chain over
    /// the document defaults.
    paragraph: std::collections::HashMap<&'a str, FontPair<'a>>,
    /// The fonts a run's character style sets: the character and paragraph styles in its chain.
    character: std::collections::HashMap<&'a str, FontPair<'a>>,
    /// The Latin and complex-script sizes a paragraph style's chain sets.
    size: std::collections::HashMap<&'a str, (Option<f32>, Option<f32>)>,
    /// The size a run's character style sets, if any.
    char_size: std::collections::HashMap<&'a str, Option<f32>>,
    defaults: FontPair<'a>,
    default_size: (Option<f32>, Option<f32>),
    /// The styles the open path rewrites to Kalpurush: a paragraph style that shows Bijoy (its chain over
    /// the defaults), and a character or table style that sets a font itself and shows Bijoy. A style that
    /// sets no font of its own is never given one, since a font on a character style would override the
    /// paragraph's font.
    rewrite: std::collections::HashSet<String>,
}

impl<'a> StyleFonts<'a> {
    fn new(styles: &'a wordcraft_doc::StyleSheet) -> Self {
        use wordcraft_doc::StyleKind;
        // The first style with an id is the one `StyleSheet::get` finds.
        let mut index: std::collections::HashMap<&'a str, &'a wordcraft_doc::Style> = std::collections::HashMap::new();
        for st in &styles.styles {
            index.entry(st.id.as_str()).or_insert(st);
        }
        let defaults = (styles.default_chr.font.as_deref(), styles.default_chr.font_cs.as_deref());
        let mut fonts = Self {
            paragraph: std::collections::HashMap::new(),
            character: std::collections::HashMap::new(),
            size: std::collections::HashMap::new(),
            char_size: std::collections::HashMap::new(),
            defaults,
            default_size: (styles.default_chr.size, styles.default_chr.size_cs),
            rewrite: std::collections::HashSet::new(),
        };
        for (&id, &st) in &index {
            let chain = style_chain(&index, st);
            let nearest = |kinds: &dyn Fn(&StyleKind) -> bool| -> FontPair<'a> {
                let latin = chain.iter().filter(|s| kinds(&s.kind)).find_map(|s| s.chr.font.as_deref());
                let complex = chain.iter().filter(|s| kinds(&s.kind)).find_map(|s| s.chr.font_cs.as_deref());
                (latin, complex)
            };
            let para = nearest(&|k| *k == StyleKind::Paragraph);
            let para = (para.0.or(defaults.0), para.1.or(defaults.1));
            fonts.paragraph.insert(id, para);
            fonts.character.insert(id, nearest(&|k| *k == StyleKind::Character || *k == StyleKind::Paragraph));

            let own = nearest(&|_| true);
            let sets_font = own.0.is_some() || own.1.is_some();
            let rewritten = match st.kind {
                StyleKind::Paragraph => font_pair_is_bijoy(para.0, para.1),
                _ => sets_font && font_pair_is_bijoy(own.0.or(defaults.0), own.1.or(defaults.1)),
            };
            if rewritten {
                fonts.rewrite.insert(id.to_string());
            }

            fonts.size.insert(id, (chain.iter().find_map(|s| s.chr.size), chain.iter().find_map(|s| s.chr.size_cs)));
            let chr = |s: &&&wordcraft_doc::Style| matches!(s.kind, StyleKind::Character | StyleKind::Paragraph);
            let char_size = chain.iter().filter(chr).find_map(|s| s.chr.size).or_else(|| chain.iter().filter(chr).find_map(|s| s.chr.size_cs));
            fonts.char_size.insert(id, char_size);
        }
        fonts
    }

    /// The fonts a paragraph's text inherits: from its paragraph style, Normal when it names none, or the
    /// document defaults when the style is not in the sheet.
    fn paragraph(&self, style: Option<&str>) -> FontPair<'a> {
        self.paragraph.get(style.unwrap_or("Normal")).copied().unwrap_or(self.defaults)
    }

    /// The point size a paragraph's text inherits. Latin keystrokes are drawn at the Latin size, so it is
    /// the Latin size of the style chain, then the defaults', before any complex-script size.
    fn size(&self, style: Option<&str>) -> Option<f32> {
        let (latin, complex) = self.size.get(style.unwrap_or("Normal")).copied().unwrap_or((None, None));
        latin.or(self.default_size.0).or(complex).or(self.default_size.1)
    }

    /// The point size a run's character style sets, if any: the model draws the run at it, before the
    /// paragraph style's size.
    fn char_size(&self, style: Option<&str>) -> Option<f32> {
        style.and_then(|id| self.char_size.get(id)).copied().flatten()
    }

    /// The fonts a run (or paragraph mark) shows: its own, then its character style's, then the paragraph's.
    fn run<'b>(&'b self, props: &'b wordcraft_doc::CharProps, para: FontPair<'b>) -> FontPair<'b> {
        let style = props.style.as_deref().and_then(|id| self.character.get(id)).copied().unwrap_or((None, None));
        (props.font.as_deref().or(style.0).or(para.0), props.font_cs.as_deref().or(style.1).or(para.1))
    }

    /// The ids of the styles the open path rewrites to Kalpurush (see the field).
    fn bijoy_ids(&self) -> std::collections::HashSet<String> {
        self.rewrite.clone()
    }
}

/// A style's `based_on` chain from the style itself up, as `StyleSheet::chain` walks it: at most 32
/// styles, stopping where a style repeats.
fn style_chain<'a>(
    index: &std::collections::HashMap<&'a str, &'a wordcraft_doc::Style>,
    st: &'a wordcraft_doc::Style,
) -> Vec<&'a wordcraft_doc::Style> {
    let mut chain: Vec<&'a wordcraft_doc::Style> = Vec::new();
    let mut current = Some(st);
    while let Some(style) = current {
        if chain.len() >= 32 || chain.iter().any(|seen| seen.id == style.id) {
            break;
        }
        chain.push(style);
        current = style.based_on.as_deref().and_then(|base| index.get(base).copied());
    }
    chain
}

/// Whether a run (or paragraph mark) is Bijoy: by the fonts it shows (see [`StyleFonts::run`]), so a run
/// that sets only a complex-script font under an English style is English.
fn run_is_bijoy(props: &wordcraft_doc::CharProps, fonts: &StyleFonts<'_>, para_fonts: FontPair<'_>) -> bool {
    let (latin, complex) = fonts.run(props, para_fonts);
    font_pair_is_bijoy(latin, complex)
}

fn normalize_blocks_bijoy(blocks: &mut [std::sync::Arc<wordcraft_doc::Block>], fonts: &StyleFonts<'_>) {
    for b in blocks.iter_mut() {
        let block = std::sync::Arc::make_mut(b);
        match block {
            wordcraft_doc::Block::Para(p) => {
                // A paragraph is Bijoy if the fonts its style (Normal when it names none) or the document
                // defaults give its text are Bijoy, or its mark or any run shows a Bijoy font.
                let para_fonts = fonts.paragraph(p.props.style.as_deref());
                let style_size = fonts.size(p.props.style.as_deref());
                let style_is_bijoy = font_pair_is_bijoy(para_fonts.0, para_fonts.1);
                let para_has_bijoy =
                    style_is_bijoy || run_is_bijoy(&p.mark, fonts, para_fonts) || p.runs.iter().any(|r| run_is_bijoy(&r.props, fonts, para_fonts));
                let para_is_partial = !para_has_bijoy && has_leftover_bijoy(&p.text);

                if para_has_bijoy {
                    if p.mark.size.is_none() && p.mark.size_cs.is_none() {
                        p.mark.size = style_size;
                    }
                    normalize_bijoy_char_props(&mut p.mark);
                    // Also normalize the paragraph mark's tracked-change old props
                    if let Some(fc) = &mut p.mark.fmt_change
                        && char_props_has_bijoy(&fc.old)
                    {
                        normalize_bijoy_char_props(&mut fc.old);
                    }
                    // Force single line spacing (1.0×) on converted Bijoy paragraphs (including empty ones).
                    p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
                } else if para_is_partial {
                    p.mark.font = Some("Kalpurush".into());
                    p.mark.font_cs = Some("Kalpurush".into());
                }

                if (para_has_bijoy || para_is_partial) && !p.text.is_empty() {
                    // 1. Extract slices and classify each run. Consecutive Bijoy runs form one group that
                    // is converted as one text, so pre-base vowels (`w`, `†`, `‰`), conjuncts (`¯Í`) and reph
                    // (`©`) reorder across run boundaries; each converted character keeps the formatting of
                    // the run it comes from. A group ends at an inline object placeholder.
                    enum Pending {
                        Bijoy { text: crate::cmd::bangla::Tagged, props: Vec<wordcraft_doc::CharProps> },
                        Partial { slice: String, props: wordcraft_doc::CharProps },
                        Plain { slice: String, props: wordcraft_doc::CharProps },
                    }
                    const OBJ: char = wordcraft_doc::para::OBJ;

                    let mut offset = 0usize;
                    let mut staged: Vec<Pending> = Vec::with_capacity(p.runs.len().max(1));

                    let last_run = p.runs.len().saturating_sub(1);
                    for (k, r) in p.runs.iter().enumerate() {
                        // Run lengths come from the file: the last run takes whatever text is left, so
                        // no text is lost when the runs fall short of it.
                        let end = if k == last_run {
                            p.text.len()
                        } else {
                            char_boundary_at_or_after(&p.text, offset.saturating_add(r.len).min(p.text.len()))
                        };
                        let run_slice = slice_safe(&p.text, offset, end);
                        offset = end;
                        if run_slice.is_empty() {
                            continue;
                        }

                        // A run is Bijoy by the fonts it shows: its own, its character style's, then the paragraph's.
                        let is_bijoy = run_is_bijoy(&r.props, fonts, para_fonts);
                        let is_partial = !is_bijoy && has_leftover_bijoy(run_slice);

                        let mut props = r.props.clone();
                        if is_bijoy {
                            if props.size.is_none() && props.size_cs.is_none() {
                                props.size = fonts.char_size(props.style.as_deref()).or(style_size);
                            }
                            normalize_bijoy_char_props(&mut props);
                            // Normalize the run's tracked-change old formatting too
                            if let Some(fc) = &mut props.fmt_change
                                && char_props_has_bijoy(&fc.old)
                            {
                                normalize_bijoy_char_props(&mut fc.old);
                            }
                        } else if is_partial {
                            props.font = Some("Kalpurush".into());
                            props.font_cs = Some("Kalpurush".into());
                        } else if para_has_bijoy {
                            // Inline Latin/English runs inside a Bijoy paragraph also reduce
                            // their font size by 2pt so they stay proportional to the surrounding
                            // Bengali sentence.
                            if props.size.is_none() && props.size_cs.is_none() {
                                props.size = fonts.char_size(props.style.as_deref()).or(style_size);
                            }
                            if props.size.is_none() {
                                props.size = props.size_cs;
                            }
                            if let Some(sz) = props.size.as_mut() {
                                *sz = (*sz - 2.0).max(6.0);
                            }
                            props.size_cs = props.size;
                        }

                        if !is_bijoy {
                            staged.push(if is_partial {
                                Pending::Partial { slice: run_slice.to_string(), props }
                            } else {
                                Pending::Plain { slice: run_slice.to_string(), props }
                            });
                            continue;
                        }
                        if let Some(Pending::Bijoy { text, props: group }) = staged.last_mut()
                            && !text.chars().last().is_some_and(|&(c, _)| c == OBJ)
                            && !run_slice.starts_with(OBJ)
                        {
                            let tag = u32::try_from(group.len()).unwrap_or(u32::MAX);
                            group.push(props);
                            text.push_str(run_slice, tag);
                            continue;
                        }
                        staged.push(Pending::Bijoy { text: crate::cmd::bangla::Tagged::uniform(run_slice, 0), props: vec![props] });
                    }

                    let mut new_text = String::new();
                    let mut new_runs = Vec::with_capacity(staged.len().max(1));

                    for item in staged {
                        match item {
                            Pending::Bijoy { text, props } => {
                                let text = text.with_run_marks(&props);
                                // Tab spaces are compacted within each run's text, as before runs were
                                // converted together: spaces in one run are never shortened by a tab in another.
                                let converted = crate::cmd::bangla::ansi_to_unicode_tagged(text);
                                for (segment, tag) in converted.segments() {
                                    let segment = compact_bijoy_tab_spaces(&segment);
                                    if segment.is_empty() {
                                        continue;
                                    }
                                    let run_props = converted
                                        .prop(tag)
                                        .cloned()
                                        .unwrap_or_else(|| props.get(crate::cmd::bangla::tag_index(tag)).cloned().unwrap_or_default());
                                    crate::cmd::bangla::push_run(&mut new_runs, segment.len(), run_props);
                                    new_text.push_str(&segment);
                                }
                            }
                            Pending::Partial { slice, props } => {
                                let converted = compact_bijoy_tab_spaces(&repair_partial_bijoy(&slice));
                                crate::cmd::bangla::push_run(&mut new_runs, converted.len(), props);
                                new_text.push_str(&converted);
                            }
                            Pending::Plain { slice, props } => {
                                crate::cmd::bangla::push_run(&mut new_runs, slice.len(), props);
                                new_text.push_str(&slice);
                            }
                        }
                    }

                    if new_runs.is_empty() {
                        // A paragraph with no runs: a Bijoy one is decoded and resized; a half-converted
                        // one only has its leftover words repaired and its font set, as a partial run would.
                        let mut props = wordcraft_doc::CharProps { size: style_size, ..Default::default() };
                        let raw = if para_is_partial {
                            props.font = Some("Kalpurush".into());
                            props.font_cs = Some("Kalpurush".into());
                            repair_partial_bijoy(&p.text)
                        } else {
                            normalize_bijoy_char_props(&mut props);
                            crate::cmd::bangla::ansi_to_unicode(&p.text)
                        };
                        let converted = compact_bijoy_tab_spaces(&raw);
                        new_runs.push(wordcraft_doc::Run { len: converted.len(), props });
                        new_text = converted;
                    }

                    p.text = new_text;
                    p.runs = new_runs;
                    p.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
                }
            }
            wordcraft_doc::Block::Table(t) => {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        normalize_blocks_bijoy(&mut cell.blocks, fonts);
                    }
                }
            }
        }
    }

    // Keep centered bold underlined Bijoy section headings (and any single empty spacing
    // paragraph right below them) together with the following body paragraph so reducing
    // font sizes by 2pt never orphans a section title at the bottom of a page.
    let mut keep_empty = false;
    for b in blocks.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            if p.text.trim().is_empty() {
                if keep_empty {
                    p.props.keep_next = Some(true);
                    keep_empty = false;
                }
            } else {
                keep_empty = false;
                let is_centered = p.props.align == Some(wordcraft_doc::props::Align::Center);
                let is_bold_ul = p.mark.bold == Some(true) && p.mark.underline.is_some();
                if is_centered && is_bold_ul && p.mark.font.as_deref() == Some("Kalpurush") {
                    p.props.keep_next = Some(true);
                    keep_empty = true;
                }
            }
        } else {
            keep_empty = false;
        }
    }
}

/// The ids of the styles the open path rewrites to Kalpurush (see [`StyleFonts::bijoy_ids`]).
#[cfg(test)]
fn bijoy_style_set(styles: &wordcraft_doc::StyleSheet) -> std::collections::HashSet<String> {
    StyleFonts::new(styles).bijoy_ids()
}

/// Automatically detects legacy Bijoy/ANSI fonts in the document, transparently
/// converts raw ASCII glyph keystrokes to standard Bengali Unicode, and assigns
/// a clean OpenType Unicode font ("Kalpurush") while strictly preserving character
/// sizing, bold styling, and document colors.
pub fn normalize_bijoy_fonts(doc: &mut Document) {
    let default_is_bijoy = char_props_has_bijoy(&doc.styles.default_chr);
    let fonts = StyleFonts::new(&doc.styles);
    let bijoy_style_ids = fonts.bijoy_ids();

    let has_bijoy = default_is_bijoy
        || !bijoy_style_ids.is_empty()
        || blocks_have_bijoy(&doc.body)
        || doc.parts.values().any(|part| blocks_have_bijoy(&part.blocks));

    if !has_bijoy {
        return;
    }

    // Decode all body blocks recursively (including tables)
    normalize_blocks_bijoy(&mut doc.body, &fonts);

    // Decode all secondary parts (headers, footers, footnotes, endnotes, comments, text boxes)
    for part in doc.parts.values_mut() {
        normalize_blocks_bijoy(&mut part.blocks, &fonts);
    }

    // Update style sheet default font and any named styles that referenced Bijoy
    if char_props_has_bijoy(&doc.styles.default_chr) {
        normalize_bijoy_char_props(&mut doc.styles.default_chr);
    }
    for st in &mut doc.styles.styles {
        // Styles whose own chain sets a Bijoy font, by nearest precedence: a style whose own font is
        // Bijoy but whose nearest Latin font is not is English and keeps its font, and a style that sets
        // no font (it shows the defaults, rewritten above) is left without one.
        if bijoy_style_ids.contains(&st.id) {
            normalize_bijoy_char_props(&mut st.chr);
            st.para.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
        }
    }

    // Convert watermark text if it uses a Bijoy font
    if let Some(w) = &mut doc.settings.watermark
        && is_bijoy_font(&w.font)
    {
        w.text = crate::cmd::bangla::ansi_to_unicode(&w.text);
        w.font = "Kalpurush".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{Block, Cell, CharProps, Document, Paragraph, Rgb, Row, Run, Table, TextColor};

    #[test]
    fn normalize_bijoy_fonts_preserves_formatting_properties() {
        let text = "evsjv‡`k K…wl e¨vsK".to_string();
        let green = TextColor::Rgb(Rgb(0, 102, 0));
        let props = CharProps {
            font: Some("SutonnyMJ".into()),
            font_cs: Some("Calibri".into()),
            size: Some(26.0),
            bold: Some(true),
            color: Some(green),
            ..Default::default()
        };
        let p = Paragraph {
            text: text.clone(),
            runs: vec![Run { len: text.len(), props }],
            mark: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        let mut doc = Document { body: vec![std::sync::Arc::new(Block::Para(p))], ..Default::default() };

        normalize_bijoy_fonts(&mut doc);

        let p_out = doc.body[0].as_para().unwrap();
        assert_eq!(p_out.text, "বাংলাদেশ কৃষি ব্যাংক");
        assert_eq!(p_out.runs.len(), 1);
        assert_eq!(p_out.runs[0].props.font.as_deref(), Some("Kalpurush"));
        assert_eq!(p_out.runs[0].props.font_cs.as_deref(), Some("Kalpurush"));
        // Font size reduced by 2 pt (26 pt Bijoy → 24 pt Kalpurush).
        assert_eq!(p_out.runs[0].props.size, Some(24.0));
        assert_eq!(p_out.runs[0].props.size_cs, Some(24.0));
        assert_eq!(p_out.runs[0].props.bold, Some(true));
        assert_eq!(p_out.runs[0].props.bold_cs, Some(true));
        assert_eq!(p_out.runs[0].props.color, Some(green));
        assert_eq!(p_out.mark.font.as_deref(), Some("Kalpurush"));
        assert_eq!(p_out.mark.font_cs.as_deref(), Some("Kalpurush"));
        // Line spacing is forced to single after conversion.
        assert_eq!(p_out.props.line_spacing, Some(wordcraft_doc::props::LineSpacing::Multiple(1.0)));
    }

    #[test]
    fn normalize_bijoy_coalesces_split_runs_and_tables() {
        // Word splits "BZ¨vw`" across two runs ("BZ¨vw" and "`") inside a table cell
        let r1 = Run { len: "BZ¨vw".len(), props: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() } };
        let r2 = Run { len: "`".len(), props: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() } };
        let cell_para = Paragraph { text: "BZ¨vw`".into(), runs: vec![r1, r2], ..Default::default() };
        let table = Table {
            rows: vec![Row {
                cells: vec![Cell { blocks: vec![std::sync::Arc::new(Block::Para(cell_para))], ..Default::default() }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut doc = Document { body: vec![std::sync::Arc::new(Block::Table(table))], ..Default::default() };

        normalize_bijoy_fonts(&mut doc);

        let Block::Table(t) = &*doc.body[0] else { panic!("expected table") };
        let p_out = t.rows[0].cells[0].blocks[0].as_para().unwrap();
        assert_eq!(p_out.text, "ইত্যাদি");
    }

    #[test]
    fn normalize_bijoy_style_chain_and_watermark() {
        use wordcraft_doc::{Style, StyleKind, Watermark};

        // Base style with SutonnyMJ 14pt
        let mut st1 = Style { id: "BaseBangla".into(), name: "Base Bangla".into(), kind: StyleKind::Paragraph, ..Default::default() };
        st1.chr.font = Some("SutonnyMJ".into());
        st1.chr.size = Some(14.0);

        // Child style inheriting from BaseBangla
        let st2 = Style {
            id: "ChildBangla".into(),
            name: "Child Bangla".into(),
            kind: StyleKind::Paragraph,
            based_on: Some("BaseBangla".into()),
            ..Default::default()
        };

        let mut doc = Document::new();
        doc.styles.styles.push(st1);
        doc.styles.styles.push(st2);

        // Paragraph using ChildBangla with no explicit font or size on runs
        let text = "evsjv‡`k".to_string();
        let mut p = Paragraph::new();
        p.props.style = Some("ChildBangla".into());
        p.text = text.clone();
        p.runs = vec![Run { len: text.len(), props: CharProps::default() }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];

        // Watermark in SutonnyMJ
        doc.settings.watermark = Some(Watermark { text: "evsjv‡`k".into(), font: "SutonnyMJ".into(), ..Default::default() });

        normalize_bijoy_fonts(&mut doc);

        let p_out = doc.body[0].as_para().unwrap();
        assert_eq!(p_out.text, "বাংলাদেশ");
        assert_eq!(p_out.runs[0].props.font.as_deref(), Some("Kalpurush"));
        assert_eq!(p_out.runs[0].props.size, Some(12.0));
        assert_eq!(p_out.props.line_spacing, Some(wordcraft_doc::props::LineSpacing::Multiple(1.0)));

        let wm = doc.settings.watermark.unwrap();
        assert_eq!(wm.text, "বাংলাদেশ");
        assert_eq!(wm.font, "Kalpurush");
    }

    /// A one-paragraph document whose text is in an ordinary (Unicode) Bengali font.
    fn unicode_doc(text: &str) -> Document {
        let props = CharProps { font: Some("Kalpurush".into()), ..Default::default() };
        let p = Paragraph { text: text.into(), runs: vec![Run { len: text.len(), props }], ..Default::default() };
        Document { body: vec![std::sync::Arc::new(Block::Para(p))], ..Default::default() }
    }

    fn body_text(doc: &Document) -> String {
        doc.body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default()
    }

    /// Opening a document never rewrites ordinary Unicode Bangla, even beside characters SutonnyMJ
    /// also uses as glyphs (©, °, £, ½, é): set apart by spaces or next to digits they are text.
    #[test]
    fn normalize_bijoy_leaves_unicode_bangla_beside_symbols_alone() {
        for text in ["© ২০২৪ বাংলাদেশ", "তাপমাত্রা ২৫°C আজ", "দাম ১০০£ মাত্র", "½ কেজি চাল", "ক্যাফে café বন্ধ"]
        {
            let mut doc = unicode_doc(text);
            normalize_bijoy_fonts(&mut doc);
            assert_eq!(body_text(&doc), text, "changed on open");
        }
    }

    /// In Unicode text left half-converted, only the word that still holds a SutonnyMJ glyph is
    /// repaired; the Bangla words around it stay as they are.
    #[test]
    fn normalize_bijoy_repairs_only_the_leftover_word() {
        let mut doc = unicode_doc("আমি šত্ন করি");
        normalize_bijoy_fonts(&mut doc);
        let out = body_text(&doc);
        let words: Vec<&str> = out.split(' ').collect();
        assert_eq!(words.len(), 3, "{out:?}");
        assert_eq!((words[0], words[2]), ("আমি", "করি"), "{out:?}");
        assert_ne!(words[1], "šত্ন", "the leftover word is repaired: {out:?}");
        assert!(!words[1].contains('š'), "{out:?}");
    }

    #[test]
    fn unused_bijoy_style_must_not_convert_plain_text() {
        let mut doc = Document::from_text("Hello world");
        doc.styles.default_chr = CharProps { font: Some("Calibri".into()), ..Default::default() };
        doc.styles.styles.push(wordcraft_doc::Style {
            id: "UnusedBanglaStyle".into(),
            name: "Unused Bangla Style".into(),
            chr: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        });
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "Hello world", "plain text should not be decoded as Bijoy");
    }

    #[test]
    fn mixed_english_and_bijoy_import_preserves_english_and_converts_bijoy() {
        let mut doc = Document::default();
        doc.styles.default_chr = CharProps { font: Some("Calibri".into()), ..Default::default() };
        doc.styles.styles.push(wordcraft_doc::Style {
            id: "BanglaStyle".into(),
            name: "Bangla Style".into(),
            chr: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        });
        let p_en = Paragraph {
            text: "Hello world".into(),
            props: wordcraft_doc::props::ParaProps { style: Some("Normal".into()), ..Default::default() },
            ..Default::default()
        };
        let p_bn = Paragraph {
            text: "Avgvi †mvbvi evsjv".into(),
            props: wordcraft_doc::props::ParaProps { style: Some("BanglaStyle".into()), ..Default::default() },
            ..Default::default()
        };
        doc.body = vec![std::sync::Arc::new(Block::Para(p_en)), std::sync::Arc::new(Block::Para(p_bn))];

        normalize_bijoy_fonts(&mut doc);

        let out_en = doc.body[0].as_para().map(|p| p.text.as_str()).unwrap_or("");
        let out_bn = doc.body[1].as_para().map(|p| p.text.as_str()).unwrap_or("");
        assert_eq!(out_en, "Hello world");
        assert_eq!(out_bn, "আমার সোনার বাংলা");
        assert_eq!(doc.body[1].as_para().and_then(|p| p.mark.font.as_deref()), Some("Kalpurush"));
    }

    /// A document whose styles are `styles` and whose one paragraph uses `style_id` with `text` as its
    /// only run, carrying no direct formatting.
    fn document_with_styled_paragraph(styles: Vec<wordcraft_doc::Style>, style_id: &str, text: &str) -> Document {
        let mut doc = Document::new();
        doc.styles.styles = styles;
        let mut p = Paragraph::new();
        p.props.style = Some(style_id.into());
        p.text = text.into();
        p.runs = vec![Run { len: text.len(), props: CharProps::default() }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        doc
    }

    #[test]
    fn explicit_latin_child_style_ignores_a_bijoy_ancestor() {
        use wordcraft_doc::{Style, StyleKind};
        // The parent sets both Latin and complex-script fonts to SutonnyMJ; the child overrides only
        // the Latin font, so its complex-script font is inherited Bijoy. The Latin font still decides.
        let parent = Style {
            id: "Bijoy".into(),
            name: "Bijoy".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("SutonnyMJ".into()), font_cs: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        let child = Style {
            id: "Latin".into(),
            name: "Latin".into(),
            kind: StyleKind::Paragraph,
            based_on: Some("Bijoy".into()),
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        };
        let mut doc = document_with_styled_paragraph(vec![parent, child], "Latin", "Hello world");
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "Hello world", "English under an explicit Latin font is not decoded");
    }

    #[test]
    fn intervening_latin_style_overrides_a_bijoy_ancestor() {
        use wordcraft_doc::{Style, StyleKind};
        let bijoy = Style {
            id: "Bijoy".into(),
            name: "Bijoy".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        let latin = Style {
            id: "Latin".into(),
            name: "Latin".into(),
            kind: StyleKind::Paragraph,
            based_on: Some("Bijoy".into()),
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        };
        let grandchild = Style {
            id: "Grandchild".into(),
            name: "Grandchild".into(),
            kind: StyleKind::Paragraph,
            based_on: Some("Latin".into()),
            ..Default::default()
        };
        let styles = vec![bijoy, latin, grandchild];
        let mut doc = document_with_styled_paragraph(styles.clone(), "Grandchild", "Hello world");
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "Hello world", "the nearer Latin font wins over the Bijoy ancestor");
        assert!(!bijoy_style_set(&wordcraft_doc::StyleSheet { styles, ..Default::default() }).contains("Grandchild"));
    }

    #[test]
    fn style_without_its_own_font_still_inherits_bijoy_from_an_ancestor() {
        use wordcraft_doc::{Style, StyleKind};
        let bijoy = Style {
            id: "Bijoy".into(),
            name: "Bijoy".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        let child =
            Style { id: "Child".into(), name: "Child".into(), kind: StyleKind::Paragraph, based_on: Some("Bijoy".into()), ..Default::default() };
        let styles = vec![bijoy, child];
        let mut doc = document_with_styled_paragraph(styles.clone(), "Child", "evsjv‡`k");
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(
            body_text(&doc),
            crate::cmd::bangla::ansi_to_unicode("evsjv‡`k"),
            "a child with no font of its own keeps its Bijoy ancestor's font"
        );
        assert!(bijoy_style_set(&wordcraft_doc::StyleSheet { styles, ..Default::default() }).contains("Child"));
    }

    #[test]
    fn run_setting_only_a_complex_font_under_an_english_style_is_not_decoded() {
        use wordcraft_doc::{Style, StyleKind};
        let english = Style {
            id: "English".into(),
            name: "English".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        };
        let mut doc = Document::new();
        doc.styles.styles = vec![english];
        let mut p = Paragraph::new();
        p.props.style = Some("English".into());
        p.text = "Hello world".into();
        p.runs = vec![Run { len: "Hello world".len(), props: CharProps { font_cs: Some("SutonnyMJ".into()), ..Default::default() } }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "Hello world", "the paragraph's Latin font makes the run English");
    }

    #[test]
    fn style_whose_own_font_is_bijoy_but_whose_nearest_latin_font_is_english_keeps_its_font() {
        use wordcraft_doc::{Style, StyleKind};
        let parent = Style {
            id: "English".into(),
            name: "English".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        };
        let child = Style {
            id: "Mixed".into(),
            name: "Mixed".into(),
            kind: StyleKind::Paragraph,
            based_on: Some("English".into()),
            chr: CharProps { font_cs: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        let mut doc = document_with_styled_paragraph(vec![parent, child], "English", "evsjv");
        // A Bijoy run elsewhere makes the document Bijoy, so the style sheet is rewritten.
        doc.body.push(std::sync::Arc::new(Block::Para(Paragraph {
            text: "evsjv".into(),
            runs: vec![Run { len: 5, props: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() } }],
            ..Default::default()
        })));
        normalize_bijoy_fonts(&mut doc);
        let mixed = doc.styles.styles.iter().find(|st| st.id == "Mixed");
        assert_eq!(mixed.and_then(|st| st.chr.font.clone()), None, "the style's nearest Latin font is Calibri, so it is not rewritten");
    }

    #[test]
    fn formatting_change_at_a_neutral_boundary_inside_a_word_keeps_its_runs() {
        let bold = CharProps { bold: Some(true), font: Some("SutonnyMJ".into()), ..Default::default() };
        let plain = CharProps { font: Some("SutonnyMJ".into()), ..Default::default() };
        let text = "evsjv:";
        let mut doc = Document::new();
        let mut p = Paragraph::new();
        p.text = text.into();
        p.runs = vec![Run { len: "evsjv".len(), props: bold }, Run { len: ":".len(), props: plain }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        normalize_bijoy_fonts(&mut doc);
        let out = doc.body[0].as_para().cloned().unwrap_or_default();
        assert_eq!(out.text, crate::cmd::bangla::ansi_to_unicode(text));
        assert_eq!(out.runs.len(), 2, "a colon cannot change the converted text, so the formatting stays split");
        assert_eq!(out.runs.first().and_then(|r| r.props.bold), Some(true));
        assert_eq!(out.runs.get(1).and_then(|r| r.props.bold), None);
    }

    #[test]
    fn formatting_inside_a_vowel_cluster_follows_each_character() {
        let bold = CharProps { bold: Some(true), font: Some("SutonnyMJ".into()), ..Default::default() };
        let plain = CharProps { font: Some("SutonnyMJ".into()), ..Default::default() };
        let text = "wK";
        let mut doc = Document::new();
        let mut p = Paragraph::new();
        p.text = text.into();
        p.runs = vec![Run { len: "w".len(), props: bold }, Run { len: "K".len(), props: plain }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        normalize_bijoy_fonts(&mut doc);
        let out = doc.body[0].as_para().cloned().unwrap_or_default();
        assert_eq!(out.text, "কি", "the pre-base vowel reorders across the formatting change");
        let bold_of: Vec<Option<bool>> = out.runs.iter().map(|r| r.props.bold).collect();
        assert_eq!(bold_of, vec![None, Some(true)], "the plain consonant comes first, then the bold vowel sign");
        assert_eq!(out.runs.first().map(|r| r.len), Some("ক".len()));
    }

    fn sutonny(extra: CharProps) -> CharProps {
        CharProps { font: Some("SutonnyMJ".into()), ..extra }
    }

    /// A document whose one paragraph is the pieces joined, one run per piece, with no style.
    fn document_in_runs(pieces: &[(&str, CharProps)]) -> Document {
        let mut doc = Document::new();
        doc.styles.default_chr = CharProps::default();
        let text: String = pieces.iter().map(|(t, _)| *t).collect();
        let runs = pieces.iter().map(|(t, props)| Run { len: t.len(), props: props.clone() }).collect();
        doc.body = vec![std::sync::Arc::new(Block::Para(Paragraph { text, runs, ..Default::default() }))];
        doc
    }

    /// The first paragraph's runs as (text, run).
    fn runs_of(doc: &Document) -> Vec<(String, Run)> {
        let p = doc.body.first().and_then(|b| b.as_para()).cloned().unwrap_or_default();
        let mut at = 0usize;
        p.runs
            .iter()
            .map(|r| {
                let text = p.text.get(at..at + r.len).unwrap_or("").to_string();
                at += r.len;
                (text, r.clone())
            })
            .collect()
    }

    #[test]
    fn split_bijoy_word_with_identical_formatting_opens_as_one_run() {
        let mut doc = document_in_runs(&[("w", sutonny(CharProps::default())), ("K", sutonny(CharProps::default()))]);
        normalize_bijoy_fonts(&mut doc);
        let runs = runs_of(&doc);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs.first().map(|r| r.0.as_str()), Some("কি"));
    }

    #[test]
    fn english_run_inside_a_bijoy_paragraph_ends_the_bijoy_group() {
        let calibri = CharProps { font: Some("Calibri".into()), ..Default::default() };
        let mut doc = document_in_runs(&[("Avwg ", sutonny(CharProps::default())), ("w and ", calibri), ("evsjv", sutonny(CharProps::default()))]);
        normalize_bijoy_fonts(&mut doc);
        let runs = runs_of(&doc);
        let texts: Vec<&str> = runs.iter().map(|r| r.0.as_str()).collect();
        // The English `w` would become a pre-base ি if the groups on either side were joined through it.
        assert_eq!(texts, vec!["আমি ", "w and ", "বাংলা"]);
        assert_eq!(runs.get(1).and_then(|r| r.1.props.font.as_deref()), Some("Calibri"));
    }

    #[test]
    fn tab_spaces_are_compacted_within_a_run_only() {
        // In one run: spaces before a tab go, and a long space run after it shrinks to a quarter.
        let mut doc = document_in_runs(&[("K   \t      L", sutonny(CharProps::default()))]);
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "ক\t খ");

        // Spaces in one run are never shortened by a tab in another, as when runs were converted alone.
        let bold = CharProps { bold: Some(true), ..Default::default() };
        let mut doc = document_in_runs(&[("K   ", sutonny(bold)), ("\t      L", sutonny(CharProps::default()))]);
        normalize_bijoy_fonts(&mut doc);
        let texts: Vec<String> = runs_of(&doc).into_iter().map(|r| r.0).collect();
        assert_eq!(texts, vec!["ক   ".to_string(), "\t খ".to_string()]);
    }

    #[test]
    fn run_with_only_a_bijoy_complex_font_and_no_latin_font_is_decoded() {
        let mut doc = document_in_runs(&[("evsjv", CharProps { font_cs: Some("SutonnyMJ".into()), ..Default::default() })]);
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "বাংলা", "with no Latin font set anywhere, the complex-script font decides");
    }

    #[test]
    fn style_based_on_cycle_ends() {
        use wordcraft_doc::{Style, StyleKind};
        let a = Style { id: "A".into(), name: "A".into(), kind: StyleKind::Paragraph, based_on: Some("B".into()), ..Default::default() };
        let b = Style { id: "B".into(), name: "B".into(), kind: StyleKind::Paragraph, based_on: Some("A".into()), ..Default::default() };
        let mut doc = document_with_styled_paragraph(vec![a, b], "A", "Hello world");
        doc.styles.default_chr = CharProps { font: Some("Calibri".into()), ..Default::default() };
        doc.body.push(std::sync::Arc::new(Block::Para(Paragraph {
            text: "evsjv".into(),
            runs: vec![Run { len: 5, props: sutonny(CharProps::default()) }],
            ..Default::default()
        })));
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "Hello world");
        assert!(bijoy_style_set(&doc.styles).is_empty());
    }

    #[test]
    fn tracked_deletion_of_a_pre_base_glyph_deletes_only_its_vowel_sign() {
        // wLª with the w deleted: the deletion moves with the vowel sign; খ্র stays live.
        let mut doc = document_in_runs(&[("w", sutonny(CharProps { del: Some(0), ..Default::default() })), ("Lª", sutonny(CharProps::default()))]);
        normalize_bijoy_fonts(&mut doc);
        let marks: Vec<(String, Option<u32>)> = runs_of(&doc).into_iter().map(|r| (r.0, r.1.props.del)).collect();
        assert_eq!(marks, vec![("খ্র".to_string(), None), ("ি".to_string(), Some(0))]);
    }

    #[test]
    fn latin_child_style_over_bijoy_ancestor_must_not_convert_plain_text() {
        let mut doc = Document::default();
        doc.styles.default_chr = CharProps { font: Some("Calibri".into()), ..Default::default() };
        // Grandparent/parent style has SutonnyMJ
        doc.styles.styles.push(wordcraft_doc::Style {
            id: "BanglaParent".into(),
            name: "Bangla Parent".into(),
            chr: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        });
        // Child style explicitly overrides font with Latin Calibri
        doc.styles.styles.push(wordcraft_doc::Style {
            id: "LatinChild".into(),
            name: "Latin Child".into(),
            based_on: Some("BanglaParent".into()),
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        });
        let p = Paragraph {
            text: "Hello world".into(),
            props: wordcraft_doc::props::ParaProps { style: Some("LatinChild".into()), ..Default::default() },
            ..Default::default()
        };
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];

        normalize_bijoy_fonts(&mut doc);

        let out = doc.body[0].as_para().map(|p| p.text.as_str()).unwrap_or("");
        assert_eq!(out, "Hello world", "explicit Latin child style must override Bijoy ancestor");
        // Style font must remain Calibri, NOT converted to Kalpurush
        let child_style = doc.styles.styles.iter().find(|s| s.id == "LatinChild").unwrap();
        assert_eq!(child_style.chr.font.as_deref(), Some("Calibri"));
    }

    #[test]
    fn intervening_latin_parent_breaks_bijoy_inheritance() {
        let mut doc = Document::default();
        doc.styles.default_chr = CharProps { font: Some("Calibri".into()), ..Default::default() };
        // Grandparent style has SutonnyMJ
        doc.styles.styles.push(wordcraft_doc::Style {
            id: "BanglaGrandparent".into(),
            name: "Bangla Grandparent".into(),
            chr: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        });
        // Intervening parent style overrides with Latin Calibri
        doc.styles.styles.push(wordcraft_doc::Style {
            id: "LatinParent".into(),
            name: "Latin Parent".into(),
            based_on: Some("BanglaGrandparent".into()),
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        });
        // Child style specifies no explicit font, inherits from LatinParent
        doc.styles.styles.push(wordcraft_doc::Style {
            id: "ChildStyle".into(),
            name: "Child Style".into(),
            based_on: Some("LatinParent".into()),
            ..Default::default()
        });
        let p = Paragraph {
            text: "Important announcement".into(),
            props: wordcraft_doc::props::ParaProps { style: Some("ChildStyle".into()), ..Default::default() },
            ..Default::default()
        };
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];

        normalize_bijoy_fonts(&mut doc);

        let out = doc.body[0].as_para().map(|p| p.text.as_str()).unwrap_or("");
        assert_eq!(out, "Important announcement", "intervening Latin parent must break Bijoy inheritance");
    }

    #[test]
    fn import_preserves_distinct_formatting_across_intra_word_runs() {
        let mut doc = Document::default();
        // Run 0 has 'w' (italic) in SutonnyMJ
        let r0 = Run { len: "w".len(), props: CharProps { font: Some("SutonnyMJ".into()), italic: Some(true), ..Default::default() } };
        // Run 1 has 'K' (bold) in SutonnyMJ
        let r1 = Run { len: "K".len(), props: CharProps { font: Some("SutonnyMJ".into()), bold: Some(true), ..Default::default() } };
        let p = Paragraph {
            text: "wK".into(),
            runs: vec![r0, r1],
            mark: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];

        normalize_bijoy_fonts(&mut doc);

        let p_out = doc.body[0].as_para().expect("paragraph");
        assert_eq!(p_out.text, "কি", "wK in SutonnyMJ must convert to Unicode কি on import");
        assert_eq!(p_out.runs.len(), 2, "must keep 2 runs preserving distinct bold and italic formatting");

        // In Unicode 'কি', 'ক' comes first (inheriting props of 'K', bold)
        assert_eq!(p_out.runs[0].len, "ক".len());
        assert_eq!(p_out.runs[0].props.bold, Some(true));
        assert_eq!(p_out.runs[0].props.italic, None);
        assert_eq!(p_out.runs[0].props.font.as_deref(), Some("Kalpurush"));

        // Followed by 'ি' (inheriting props of 'w', italic)
        assert_eq!(p_out.runs[1].len, "ি".len());
        assert_eq!(p_out.runs[1].props.italic, Some(true));
        assert_eq!(p_out.runs[1].props.bold, None);
        assert_eq!(p_out.runs[1].props.font.as_deref(), Some("Kalpurush"));
    }

    const TEXT_FC: u32 = 0x600;

    fn fc_of(cp: u32) -> u32 {
        TEXT_FC + 2 * cp
    }

    fn grpprl(prls: &[(u16, &[u8])]) -> Vec<u8> {
        let mut v = Vec::new();
        for (op, operand) in prls {
            v.extend_from_slice(&op.to_le_bytes());
            v.extend_from_slice(operand);
        }
        v
    }

    fn std_bytes(name: &str) -> Vec<u8> {
        let mut base = vec![0u8; 10];
        base[0] = 0xFF;
        base[1] = 0x0F;
        let w1 = 1u16 | (0x0FFFu16 << 4);
        base[2..4].copy_from_slice(&w1.to_le_bytes());
        let w2 = 2u16 | (0x0FFFu16 << 4);
        base[4..6].copy_from_slice(&w2.to_le_bytes());
        let mut v = base;
        let units: Vec<u16> = name.chars().map(|c| c as u16).collect();
        v.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for u in &units {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        let cb = v.len() as u16;
        let mut out = cb.to_le_bytes().to_vec();
        out.extend_from_slice(&v);
        out[6 + 2] = cb as u8;
        out[7 + 2] = (cb >> 8) as u8;
        out
    }

    fn ffn_bytes(name: &str) -> Vec<u8> {
        let utf16z = |s: &str| -> Vec<u8> { s.chars().chain(['\0']).flat_map(|c| (c as u16).to_le_bytes()).collect() };
        let mut v = 1u16.to_le_bytes().to_vec();
        v.extend_from_slice(&0u16.to_le_bytes());
        let mut ffn = vec![0u8; 39];
        let primary = utf16z(name);
        ffn.extend_from_slice(&primary);
        v.push(ffn.len() as u8);
        v.extend_from_slice(&ffn);
        v
    }

    fn chpx_fkp(runs: &[(u32, Vec<u8>)], end_cp: u32) -> Vec<u8> {
        let mut page = vec![0u8; 512];
        let n = runs.len();
        for (i, r) in runs.iter().enumerate() {
            page[i * 4..i * 4 + 4].copy_from_slice(&fc_of(r.0).to_le_bytes());
        }
        page[n * 4..n * 4 + 4].copy_from_slice(&fc_of(end_cp).to_le_bytes());
        let rgb_at = (n + 1) * 4;
        let mut data_at = rgb_at + n + 1;
        for (i, r) in runs.iter().enumerate() {
            if r.1.is_empty() {
                page[rgb_at + i] = 0;
                continue;
            }
            data_at += data_at & 1;
            let half = (data_at / 2) as u8;
            page[rgb_at + i] = half;
            page[data_at] = r.1.len() as u8;
            page[data_at + 1..data_at + 1 + r.1.len()].copy_from_slice(&r.1);
            data_at += 1 + r.1.len();
        }
        page[511] = n as u8;
        page
    }

    fn papx_fkp(entries: &[(u32, u16, Vec<u8>)]) -> Vec<u8> {
        let mut page = vec![0u8; 512];
        let n = entries.len();
        for (i, e) in entries.iter().enumerate() {
            page[i * 4..i * 4 + 4].copy_from_slice(&fc_of(e.0).to_le_bytes());
        }
        page[n * 4..n * 4 + 4].copy_from_slice(&fc_of(0x3FF0_0000).to_le_bytes());
        let rgbx_at = (n + 1) * 4;
        let mut data_at = rgbx_at + n * 13;
        for (i, e) in entries.iter().enumerate() {
            data_at += data_at & 1;
            let half = (data_at / 2) as u8;
            page[rgbx_at + i * 13] = half;
            let grp = [e.1.to_le_bytes().as_slice(), e.2.as_slice()].concat();
            let cb = ((grp.len() + 2) / 2) as u8;
            page[data_at] = cb;
            page[data_at + 1..data_at + 1 + grp.len()].copy_from_slice(&grp);
            data_at += 1 + grp.len() + ((1 + grp.len()) & 1);
        }
        page[511] = n as u8;
        page
    }

    fn plcpcd(text: &str, first_fc: u32) -> Vec<u8> {
        let mut v = Vec::new();
        let cp0 = 0u32;
        let cp1 = text.chars().count() as u32;
        v.extend_from_slice(&cp0.to_le_bytes());
        v.extend_from_slice(&cp1.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&first_fc.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v
    }

    fn cfb_file(streams: &[(&str, &[u8])]) -> Vec<u8> {
        let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).expect("cfb create");
        for (name, data) in streams {
            let mut s = comp.create_stream(format!("/{name}")).expect("cfb stream");
            std::io::Write::write_all(&mut s, data).expect("cfb write");
        }
        comp.into_inner().into_inner()
    }

    fn make_synthetic_doc_streams(text: &str, font: &str) -> (Vec<u8>, Vec<u8>) {
        let ccp = text.chars().count() as u32;
        let plc = plcpcd(text, TEXT_FC);
        let mut clx = vec![0x02u8];
        clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc);

        let chpx_page_at = (TEXT_FC as usize + text.len() * 2).div_ceil(512) * 512;
        let papx_page_at = chpx_page_at + 512;
        let mut word = vec![0u8; TEXT_FC as usize];
        let units: Vec<u16> = text.chars().map(|c| c as u16).collect();
        for u in units {
            word.extend_from_slice(&u.to_le_bytes());
        }
        word.resize(chpx_page_at, 0);
        let chpx = grpprl(&[(0x4A4F, &0u16.to_le_bytes())]); // sprmCRgftc0 -> font index 0
        word.extend_from_slice(&chpx_fkp(&[(0, chpx)], ccp));
        word.extend_from_slice(&papx_fkp(&[(0, 0, Vec::new())]));

        let stsh_body = std_bytes("Normal");
        let stshi = [1u16.to_le_bytes(), 10u16.to_le_bytes()].concat();
        let mut stsh = (stshi.len() as u16).to_le_bytes().to_vec();
        stsh.extend_from_slice(&stshi);
        stsh.extend_from_slice(&stsh_body);

        let ffn = ffn_bytes(font);
        let clx_at = 0x40;
        let plcf_chpx =
            [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((chpx_page_at / 512) as u32).to_le_bytes().as_slice()]
                .concat();
        let plcf_papx =
            [0u32.to_le_bytes().as_slice(), fc_of(0x3FF0_0000).to_le_bytes().as_slice(), ((papx_page_at / 512) as u32).to_le_bytes().as_slice()]
                .concat();

        let mut table = vec![0u8; clx_at];
        table.extend_from_slice(&clx);
        let chpx_bins_at = table.len();
        table.extend_from_slice(&plcf_chpx);
        let papx_bins_at = table.len();
        table.extend_from_slice(&plcf_papx);
        let stsh_at = table.len();
        table.extend_from_slice(&stsh);
        let ffn_at = table.len();
        table.extend_from_slice(&ffn);

        let mut fib = vec![0u8; 0x9A + 0x5D * 8 + 2];
        fib[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
        fib[2..4].copy_from_slice(&0x00C1u16.to_le_bytes());
        fib[0x0A..0x0C].copy_from_slice(&0x1000u16.to_le_bytes());
        fib[0x0C..0x0E].copy_from_slice(&0x00BFu16.to_le_bytes());
        fib[0x20..0x22].copy_from_slice(&0x000Eu16.to_le_bytes());
        fib[0x3E..0x40].copy_from_slice(&0x0016u16.to_le_bytes());
        fib[0x40..0x44].copy_from_slice(&0x1000u32.to_le_bytes());
        fib[0x4C..0x50].copy_from_slice(&ccp.to_le_bytes());
        fib[0x98..0x9A].copy_from_slice(&0x005Du16.to_le_bytes());

        let put_pair = |fib: &mut [u8], idx: usize, fc: usize, lcb: usize| {
            fib[0x9A + idx * 8..0x9A + idx * 8 + 4].copy_from_slice(&(fc as u32).to_le_bytes());
            fib[0x9A + idx * 8 + 4..0x9A + idx * 8 + 8].copy_from_slice(&(lcb as u32).to_le_bytes());
        };
        put_pair(&mut fib, 33, clx_at, clx.len());
        put_pair(&mut fib, 1, stsh_at, stsh.len());
        put_pair(&mut fib, 15, ffn_at, ffn.len());
        put_pair(&mut fib, 12, chpx_bins_at, plcf_chpx.len());
        put_pair(&mut fib, 13, papx_bins_at, plcf_papx.len());

        word[..fib.len()].copy_from_slice(&fib);
        (word, table)
    }

    const PAD_ARRAY: [u8; 15] = [0xBB, 0xFF, 0xFF, 0xBA, 0xFF, 0xFF, 0xB9, 0x80, 0x00, 0xBE, 0x0F, 0x00, 0xBF, 0x0F, 0x00];
    const INITIAL_CODE: [u16; 15] =
        [0xE1F0, 0x1D0F, 0xCC9C, 0x84C0, 0x110C, 0x0E10, 0xF1CE, 0x313E, 0x1872, 0xE139, 0xD40F, 0x84F9, 0x280C, 0xA96A, 0x4EC3];
    const XOR_MATRIX: [u16; 105] = [
        0xAEFC, 0x4DD9, 0x9BB2, 0x2745, 0x4E8A, 0x9D14, 0x2A09, 0x7B61, 0xF6C2, 0xFDA5, 0xEB6B, 0xC6F7, 0x9DCF, 0x2BBF, 0x4563, 0x8AC6, 0x05AD,
        0x0B5A, 0x16B4, 0x2D68, 0x5AD0, 0x0375, 0x06EA, 0x0DD4, 0x1BA8, 0x3750, 0x6EA0, 0xDD40, 0xD849, 0xA0B3, 0x5147, 0xA28E, 0x553D, 0xAA7A,
        0x44D5, 0x6F45, 0xDE8A, 0xAD35, 0x4A4B, 0x9496, 0x390D, 0x721A, 0xEB23, 0xC667, 0x9CEF, 0x29FF, 0x53FE, 0xA7FC, 0x5FD9, 0x47D3, 0x8FA6,
        0x0F6D, 0x1EDA, 0x3DB4, 0x7B68, 0xF6D0, 0xB861, 0x60E3, 0xC1C6, 0x93AD, 0x377B, 0x6EF6, 0xDDEC, 0x45A0, 0x8B40, 0x06A1, 0x0D42, 0x1A84,
        0x3508, 0x6A10, 0xAA51, 0x4483, 0x8906, 0x022D, 0x045A, 0x08B4, 0x1168, 0x76B4, 0xED68, 0xCAF1, 0x85C3, 0x1BA7, 0x374E, 0x6E9C, 0x3730,
        0x6E60, 0xDCC0, 0xA9A1, 0x4363, 0x86C6, 0x1DAD, 0x3331, 0x6662, 0xCCC4, 0x89A9, 0x0373, 0x06E6, 0x0DCC, 0x1021, 0x2042, 0x4084, 0x8108,
        0x1231, 0x2462, 0x48C4,
    ];

    fn xor_key(pw: &[u8]) -> u16 {
        let mut key = INITIAL_CODE.get(pw.len().wrapping_sub(1)).copied().unwrap_or(0);
        let mut current = 0x68i32;
        for &ch in pw.iter().rev() {
            let mut ch = ch;
            for _ in 0..7 {
                if ch & 0x40 != 0
                    && let Some(m) = usize::try_from(current).ok().and_then(|c| XOR_MATRIX.get(c))
                {
                    key ^= m;
                }
                ch <<= 1;
                current -= 1;
            }
        }
        key
    }

    fn verifier_method1(pw: &[u8]) -> u16 {
        let mut v = 0u16;
        for &b in pw.iter().rev().chain(std::iter::once(&(pw.len() as u8))) {
            let i1 = u16::from(v & 0x4000 != 0);
            let i2 = v.wrapping_mul(2) & 0x7FFF;
            v = (i1 | i2) ^ u16::from(b);
        }
        v ^ 0xCE4B
    }

    fn xor_verifier(pw: &[u8]) -> u32 {
        (u32::from(xor_key(pw)) << 16) | u32::from(verifier_method1(pw))
    }

    fn xor_doc(mut word: Vec<u8>, mut table: Vec<u8>, pw: &[u8]) -> Vec<u8> {
        let key = (xor_verifier(pw) >> 16) as u16;
        let (high, low) = ((key >> 8) as u8, key as u8);
        let mut arr = [0u8; 16];
        for (i, slot) in arr.iter_mut().enumerate() {
            *slot = pw.get(i).copied().unwrap_or_else(|| PAD_ARRAY.get(i.saturating_sub(pw.len())).copied().unwrap_or(0));
        }
        for chunk in arr.as_chunks_mut::<2>().0 {
            chunk[0] = (chunk[0] ^ low).rotate_right(1);
            chunk[1] = (chunk[1] ^ high).rotate_right(1);
        }
        let fl = u16::from_le_bytes([word[0x0A], word[0x0B]]) | 0x0100 | 0x8000;
        word[0x0A..0x0C].copy_from_slice(&fl.to_le_bytes());
        word[0x0E..0x12].copy_from_slice(&xor_verifier(pw).to_le_bytes());

        for (data, skip) in [(&mut word, 68), (&mut table, 0)] {
            for (i, b) in data.iter_mut().enumerate().skip(skip) {
                if *b != 0 && *b ^ arr[i % 16] != 0 {
                    *b ^= arr[i % 16];
                }
            }
        }
        cfb_file(&[("WordDocument", &word), ("0Table", &table)])
    }

    #[test]
    fn binary_doc_legacy_bijoy_normalization_and_encryption() {
        let text = "evsjv‡`k\r";
        let (word, table) = make_synthetic_doc_streams(text, "SutonnyMJ");
        let plain_doc_bytes = cfb_file(&[("WordDocument", &word), ("0Table", &table)]);

        // 1. Open with .doc extension: must normalize SutonnyMJ to Kalpurush and decode text to "বাংলাদেশ"
        let (doc_ext, enc_ext) = open_bytes_with("test.doc", &plain_doc_bytes, None).expect("opens test.doc");
        assert!(!enc_ext, "not encrypted");
        let p_ext = doc_ext.body[0].as_para().expect("paragraph");
        assert_eq!(p_ext.text, "বাংলাদেশ", "SutonnyMJ text must be normalized to Unicode in .doc");
        assert_eq!(p_ext.runs[0].props.font.as_deref(), Some("Kalpurush"));

        // 2. Open with .dot extension: must also normalize
        let (doc_dot, enc_dot) = open_bytes_with("template.dot", &plain_doc_bytes, None).expect("opens template.dot");
        assert!(!enc_dot, "not encrypted");
        let p_dot = doc_dot.body[0].as_para().expect("paragraph");
        assert_eq!(p_dot.text, "বাংলাদেশ");
        assert_eq!(p_dot.runs[0].props.font.as_deref(), Some("Kalpurush"));

        // 3. Compare with opening the same bytes without an extension (open_by_content route)
        let (doc_no_ext, enc_no_ext) = open_bytes_with("test", &plain_doc_bytes, None).expect("opens test without extension");
        assert!(!enc_no_ext, "not encrypted");
        let p_no_ext = doc_no_ext.body[0].as_para().expect("paragraph");
        assert_eq!(p_ext.text, p_no_ext.text, "extension vs no extension text must match");
        assert_eq!(p_ext.runs[0].props.font, p_no_ext.runs[0].props.font, "extension vs no extension font must match");

        // 4. Password-protected binary .doc input
        let password = "SecretPassword";
        let enc_doc_bytes = xor_doc(word, table, password.as_bytes());

        // Opening without password fails with password required
        let err_missing = open_bytes_with("test.doc", &enc_doc_bytes, None).unwrap_err();
        assert!(needs_password(&err_missing), "err: {err_missing}");

        // Opening with wrong password fails with wrong password
        let err_wrong = open_bytes_with("test.doc", &enc_doc_bytes, Some("wrong")).unwrap_err();
        assert!(wrong_password(&err_wrong), "err: {err_wrong}");

        // Opening with correct password decrypts, reports encrypted: true, and normalizes Bijoy fonts
        let (doc_enc, was_enc) = open_bytes_with("test.doc", &enc_doc_bytes, Some(password)).expect("decrypts test.doc");
        assert!(was_enc, "must report encrypted = true");
        let p_enc = doc_enc.body[0].as_para().expect("paragraph");
        assert_eq!(p_enc.text, "বাংলাদেশ", "encrypted .doc text must be normalized to Unicode on import");
        assert_eq!(p_enc.runs[0].props.font.as_deref(), Some("Kalpurush"));

        // Compare encrypted .doc with opening the encrypted bytes without extension
        let (doc_enc_no_ext, was_enc_no_ext) = open_bytes_with("test", &enc_doc_bytes, Some(password)).expect("decrypts without extension");
        assert!(was_enc_no_ext, "must report encrypted = true without extension");
        let p_enc_no_ext = doc_enc_no_ext.body[0].as_para().expect("paragraph");
        assert_eq!(p_enc.text, p_enc_no_ext.text, "encrypted extension vs no extension text must match");
        assert_eq!(p_enc.runs[0].props.font, p_enc_no_ext.runs[0].props.font, "encrypted extension vs no extension font must match");
    }

    #[test]
    fn open_with_convert_bijoy_disabled_preserves_raw_ansi() {
        let text = "evsjv‡`k\r";
        let (word, table) = make_synthetic_doc_streams(text, "SutonnyMJ");
        let doc_bytes = cfb_file(&[("WordDocument", &word), ("0Table", &table)]);

        // 1. With convert_bijoy = false: raw ANSI and SutonnyMJ font are strictly preserved
        let preserve_settings = DocOpenSettings { convert_bijoy: false };
        let (doc_raw, _) = open_bytes_opts("test.doc", &doc_bytes, None, &wordcraft_geom::Region::default(), &preserve_settings)
            .expect("opens test.doc with convert_bijoy=false");
        let p_raw = doc_raw.body[0].as_para().expect("paragraph");
        assert_eq!(p_raw.text.trim_end(), "evsjv‡`k", "raw Bijoy ANSI bytes must be preserved");
        assert_eq!(p_raw.runs[0].props.font.as_deref(), Some("SutonnyMJ"), "original SutonnyMJ font name must be preserved");

        // 2. With convert_bijoy = true (default): converted to Unicode Kalpurush
        let default_settings = DocOpenSettings::default();
        let (doc_norm, _) = open_bytes_opts("test.doc", &doc_bytes, None, &wordcraft_geom::Region::default(), &default_settings)
            .expect("opens test.doc with default settings");
        let p_norm = doc_norm.body[0].as_para().expect("paragraph");
        assert_eq!(p_norm.text, "বাংলাদেশ", "converted to Unicode Bengali");
        assert_eq!(p_norm.runs[0].props.font.as_deref(), Some("Kalpurush"), "font normalized to Kalpurush");
    }

    #[test]
    fn unstyled_paragraph_under_a_bijoy_normal_style_is_decoded() {
        // Word writes no pStyle for Normal paragraphs; the model resolves such a paragraph through Normal.
        let mut doc = Document::new();
        doc.styles.default_chr = CharProps { font: Some("Calibri".into()), ..Default::default() };
        if let Some(normal) = doc.styles.get_mut("Normal") {
            normal.chr.font = Some("SutonnyMJ".into());
        }
        let text = "Avgvi †mvbvi evsjv";
        let mut p = Paragraph::new();
        p.text = text.into();
        p.runs = vec![Run { len: text.len(), props: CharProps::default() }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(doc.styles.get("Normal").and_then(|st| st.chr.font.as_deref()), Some("Kalpurush"), "Normal's font is switched");
        assert_eq!(body_text(&doc), "আমার সোনার বাংলা", "so the text Normal shows is decoded too");
    }

    #[test]
    fn run_with_a_bijoy_character_style_is_decoded() {
        use wordcraft_doc::{Style, StyleKind};
        let english = Style {
            id: "English".into(),
            name: "English".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        };
        let bangla = Style {
            id: "BanglaChar".into(),
            name: "Bangla Char".into(),
            kind: StyleKind::Character,
            chr: CharProps { font: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        let mut doc = document_with_styled_paragraph(vec![english, bangla], "English", "evsjv");
        if let Some(Block::Para(p)) = doc.body.first_mut().map(std::sync::Arc::make_mut) {
            p.runs = vec![Run { len: "evsjv".len(), props: CharProps { style: Some("BanglaChar".into()), ..Default::default() } }];
        }
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "বাংলা", "the run shows its character style's SutonnyMJ over the paragraph's Calibri");
    }

    #[test]
    fn mark_setting_only_a_complex_font_under_an_english_style_keeps_the_paragraph_english() {
        use wordcraft_doc::{Style, StyleKind};
        let english = Style {
            id: "English".into(),
            name: "English".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("Calibri".into()), size: Some(12.0), ..Default::default() },
            ..Default::default()
        };
        let cs_only = CharProps { font_cs: Some("SutonnyMJ".into()), ..Default::default() };
        let mut doc = Document::new();
        doc.styles.styles = vec![english];
        let mut p = Paragraph::new();
        p.props.style = Some("English".into());
        p.text = "Hello world".into();
        p.runs = vec![Run { len: "Hello world".len(), props: cs_only.clone() }];
        // Word writes the mark with the same rPr as the paragraph's text.
        p.mark = cs_only.clone();
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        normalize_bijoy_fonts(&mut doc);
        let out = doc.body[0].as_para().cloned().unwrap_or_default();
        assert_eq!(out.text, "Hello world");
        assert_eq!(out.runs.first().map(|r| r.props.clone()), Some(cs_only), "the English run keeps its size");
        assert_eq!(out.props.line_spacing, None, "the English paragraph keeps its line spacing");
        assert_eq!(out.mark.font.as_deref(), None, "the English paragraph mark is not rewritten to Kalpurush");
    }

    #[test]
    fn hostile_run_lengths_on_open_neither_overflow_nor_lose_text() {
        // A .json document reaches normalize_bijoy_fonts without Paragraph::normalize, so its run
        // lengths are whatever the file says: here past the end of the text, then short of it.
        for runs in [vec![1, usize::MAX], vec![usize::MAX, usize::MAX], vec![1, 0]] {
            let mut doc = Document::new();
            doc.styles.default_chr = CharProps::default();
            let runs: Vec<Run> = runs.into_iter().map(|len| Run { len, props: sutonny(CharProps::default()) }).collect();
            doc.body = vec![std::sync::Arc::new(Block::Para(Paragraph { text: "evsjv".into(), runs, ..Default::default() }))];
            normalize_bijoy_fonts(&mut doc);
            let p = doc.body.first().and_then(|b| b.as_para()).cloned().unwrap_or_default();
            assert_eq!(p.text, "বাংলা");
            assert_eq!(p.runs.iter().map(|r| r.len).sum::<usize>(), p.text.len(), "the rebuilt runs cover the text exactly");
            assert!(p.runs.iter().all(|r| r.len > 0), "no empty runs: {:?}", p.runs);
        }
    }

    #[test]
    fn bijoy_and_unicode_runs_that_end_up_with_the_same_formatting_open_as_one_run() {
        let unicode = CharProps { font: Some("Kalpurush".into()), font_cs: Some("Kalpurush".into()), size: Some(14.0), ..Default::default() };
        let mut doc = document_in_runs(&[("evsjv ", sutonny(CharProps { size: Some(14.0), ..Default::default() })), ("বাংলা", unicode)]);
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "বাংলা বাংলা");
        let runs = runs_of(&doc);
        assert_eq!(runs.len(), 1, "equal formatting is one run: {runs:?}");
    }

    #[test]
    fn font_less_character_style_keeps_an_english_paragraph_styles_font() {
        use wordcraft_doc::{Style, StyleKind};
        let english = Style {
            id: "English".into(),
            name: "English".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("Calibri".into()), ..Default::default() },
            ..Default::default()
        };
        let link = Style { id: "Hyperlink".into(), name: "Hyperlink".into(), kind: StyleKind::Character, ..Default::default() };
        let mut doc = Document::new();
        doc.styles.default_chr = CharProps { font: Some("SutonnyMJ".into()), ..Default::default() };
        doc.styles.styles = vec![english, link];
        let text = "www.example.com";
        let run = CharProps { style: Some("Hyperlink".into()), ..Default::default() };
        let mut p = Paragraph::new();
        p.props.style = Some("English".into());
        p.text = text.into();
        p.runs = vec![Run { len: text.len(), props: run.clone() }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), text);
        assert_eq!(
            doc.styles.resolve_char(Some("English"), &run).font,
            "Calibri",
            "a character style with no font of its own is not given one, so it never overrides the paragraph style's font"
        );
    }

    #[test]
    fn complex_only_style_under_an_english_default_is_not_rewritten() {
        // The Latin font of the defaults is English, so this style's text is English: its complex-script font
        // must not make the style Bijoy. The Bijoy run elsewhere makes the document Bijoy, so the style sheet
        // is rewritten and this style is the one that must be left alone.
        use wordcraft_doc::{Style, StyleKind};
        let mixed = Style {
            id: "Mixed".into(),
            name: "Mixed".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font_cs: Some("SutonnyMJ".into()), ..Default::default() },
            ..Default::default()
        };
        let mut doc = document_with_styled_paragraph(vec![mixed], "Mixed", "Hello world");
        doc.styles.default_chr = CharProps { font: Some("Calibri".into()), ..Default::default() };
        doc.body.push(std::sync::Arc::new(Block::Para(Paragraph {
            text: "evsjv".into(),
            runs: vec![Run { len: 5, props: sutonny(CharProps::default()) }],
            ..Default::default()
        })));
        normalize_bijoy_fonts(&mut doc);
        assert_eq!(body_text(&doc), "Hello world");
        let mixed = doc.styles.get("Mixed").cloned().unwrap_or_default();
        assert_eq!((mixed.chr.font, mixed.chr.font_cs.as_deref()), (None, Some("SutonnyMJ")), "the English style is not rewritten");
    }

    #[test]
    fn bijoy_run_takes_its_character_styles_size_before_the_paragraphs() {
        use wordcraft_doc::{Style, StyleKind};
        let english = Style {
            id: "English".into(),
            name: "English".into(),
            kind: StyleKind::Paragraph,
            chr: CharProps { font: Some("Calibri".into()), size: Some(12.0), ..Default::default() },
            ..Default::default()
        };
        let big = Style {
            id: "BigBangla".into(),
            name: "Big Bangla".into(),
            kind: StyleKind::Character,
            chr: CharProps { font: Some("SutonnyMJ".into()), size: Some(16.0), ..Default::default() },
            ..Default::default()
        };
        let mut doc = document_with_styled_paragraph(vec![english, big], "English", "evsjv");
        if let Some(Block::Para(p)) = doc.body.first_mut().map(std::sync::Arc::make_mut) {
            p.runs = vec![Run { len: 5, props: CharProps { style: Some("BigBangla".into()), ..Default::default() } }];
        }
        normalize_bijoy_fonts(&mut doc);
        let runs = runs_of(&doc);
        assert_eq!(runs.first().and_then(|r| r.1.props.size), Some(14.0), "16pt from the character style, less the 2pt: {runs:?}");
    }

    #[test]
    fn unstyled_paragraph_takes_normals_latin_size_before_a_complex_script_size() {
        // Latin keystrokes are drawn at the Latin size: Normal sets only the complex-script size (14pt), so the
        // paragraph's text is the defaults' 11pt, less the 2pt.
        let mut doc = Document::new();
        doc.styles.default_chr = CharProps { font: Some("SutonnyMJ".into()), size: Some(11.0), ..Default::default() };
        if let Some(normal) = doc.styles.get_mut("Normal") {
            normal.chr.size_cs = Some(14.0);
        }
        let text = "evsjv";
        let mut p = Paragraph::new();
        p.text = text.into();
        p.runs = vec![Run { len: text.len(), props: CharProps::default() }];
        doc.body = vec![std::sync::Arc::new(Block::Para(p))];
        normalize_bijoy_fonts(&mut doc);
        let runs = runs_of(&doc);
        assert_eq!(runs.first().and_then(|r| r.1.props.size), Some(9.0), "{runs:?}");
    }

    #[test]
    fn run_length_ending_inside_a_character_takes_the_whole_character() {
        // The first run would end in the middle of the 3-byte € sign: it takes the whole sign, and no text is
        // lost to the run after it.
        let text = "a€b";
        let mut doc = Document::new();
        doc.styles.default_chr = CharProps::default();
        let runs = vec![Run { len: 2, props: sutonny(CharProps::default()) }, Run { len: 2, props: sutonny(CharProps::default()) }];
        doc.body = vec![std::sync::Arc::new(Block::Para(Paragraph { text: text.into(), runs, ..Default::default() }))];
        normalize_bijoy_fonts(&mut doc);
        let p = doc.body.first().and_then(|b| b.as_para()).cloned().unwrap_or_default();
        assert!(p.text.contains('€'), "{:?}", p.text);
        assert_eq!(p.runs.iter().map(|r| r.len).sum::<usize>(), p.text.len(), "the runs cover the text exactly: {:?}", p.runs);
        assert!(p.runs.iter().all(|r| r.len > 0), "{:?}", p.runs);
    }
}
