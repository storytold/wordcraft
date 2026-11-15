//! Insert ▸ Library (#2868): the pictures in the user's own folders, on the Library core the Craft
//! apps share ([`craft_library`], first asked for in storytold/deckcraft#958). This module is what
//! is WordCraft's own: the files its Pictures tab lists ([`Pictures`]: the raster pictures
//! WordCraft decodes and the Windows metafiles it draws, one picture per file, known from its
//! name, so listing and search read no file), how it opens one and draws its thumbnail, and its
//! file access ([`Confined`]), which goes through [`crate::io`] so the automation read roots
//! confine every folder listing and file read. The `library.*` commands are in
//! [`crate::cmd::library`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use craft_library::{
    EntryKind, Item, MAX_DEPTH, MAX_ENTRIES, MAX_FILE_BYTES, MAX_FOLDERS, MAX_ITEMS, MAX_THUMB_HEIGHT, Nouns, Stamp, THUMB_WIDTH, THUMB_WIDTHS,
    Thumbnail, sanitized_folders,
};
use image::DynamicImage;
use wordcraft_geom::picture;

use crate::io;

/// The files the Pictures tab lists: the pictures `insert.picture` inserts, the raster pictures it
/// decodes and the Windows Metafiles and Enhanced Metafiles it draws as vectors (#802). SVG
/// doesn't insert at all.
pub const EXTENSIONS: [&str; 10] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "wmf", "emf"];

/// What a file that is none of [`EXTENSIONS`]' pictures is listed with.
const NOT_A_PICTURE: &str = "not a supported picture (PNG, JPEG, GIF, WebP, BMP, TIFF, WMF or EMF)";

/// The items' names, singular and plural: the keys of the `library.*` answers.
pub const NOUNS: Nouns = Nouns { one: "picture", many: "pictures" };

/// What every `library.*` command says in the browser build, which has no folders to read.
pub const NOT_IN_BROWSER: &str =
    "the Library is not available in the browser build: it reads folders of pictures on this computer, which a web page cannot";

/// WordCraft's Library and what it has read (in [`crate::Session`]).
pub type Cache = craft_library::Library<Pictures>;

/// Is `name` a file the Pictures tab lists? Hidden files and Office lock files are not.
pub fn is_picture(name: &str) -> bool {
    craft_library::is_listed(name, &EXTENSIONS)
}

/// File access through [`crate::io`], inside the automation read roots when they are in force.
pub struct Confined;

impl craft_library::Fs for Confined {
    fn read_dir(&self, path: &Path, max: usize) -> Result<(Vec<craft_library::DirEntry>, bool), String> {
        io::library_entries(path, max)
    }
    fn stat(&self, path: &Path) -> Result<(EntryKind, Stamp), String> {
        io::library_stat(path)
    }
    fn canonical(&self, path: &Path) -> Result<PathBuf, String> {
        io::library_canonical(path)
    }
    fn read(&self, path: &Path, max: u64) -> Result<Vec<u8>, String> {
        io::library_read(path, max)
    }
}

/// A picture file as the Library holds it once it is read for a thumbnail or an insert: its
/// bytes, which an insert puts into the document as they are, and the size its header gives.
/// Nothing is decoded until a thumbnail is drawn.
#[derive(Clone, Debug)]
pub struct Picture {
    /// The file name, which is the item's title.
    pub name: String,
    pub bytes: Vec<u8>,
    /// Its size in pixels: a raster picture's own, a metafile's frame at 96 ppi (the natural size
    /// Insert › Pictures gives it).
    pub width: u32,
    pub height: u32,
}

/// Picture files, each holding one picture.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pictures;

impl craft_library::Kind for Pictures {
    type Doc = Picture;

    fn extensions(&self) -> &[&str] {
        &EXTENSIONS
    }

    fn nouns(&self) -> Nouns {
        NOUNS
    }

    /// Reads only the header (a metafile without a frame there is read through): a file that
    /// isn't a picture WordCraft inserts, or one it never decodes (a TIFF with JPEG strips,
    /// #3645), has no thumbnail and isn't inserted, and says why.
    fn open(&self, name: &str, bytes: &[u8]) -> Result<Picture, String> {
        let (width, height) = match picture::probe(bytes) {
            Ok(probe) => (probe.width, probe.height),
            Err(e) => match metafile(bytes) {
                Some(Metafile { width, height, .. }) => (width, height),
                None => return Err(refusal(&e, bytes)),
            },
        };
        Ok(Picture { name: name.to_string(), bytes: bytes.to_vec(), width, height })
    }

    fn title(&self, _doc: &Picture) -> Option<String> {
        None
    }

    fn count(&self, _doc: &Picture) -> usize {
        1
    }

    fn item(&self, doc: &Picture, _index: usize) -> Item {
        picture_item(&doc.name)
    }

    fn thumbnail(&self, doc: &Picture, _index: usize, width: u32) -> Result<Thumbnail, String> {
        draw_thumbnail(&doc.bytes, width)
    }

    /// Every picture file is one picture, titled by its name: the Library lists and searches
    /// pictures without reading them, and reads one only for its thumbnail or an insert.
    fn item_from_name(&self, name: &str) -> Option<Item> {
        is_picture(name).then(|| picture_item(name))
    }
}

/// The picture `name` holds: titled by the file name, its format (by the extension) searchable.
fn picture_item(name: &str) -> Item {
    Item { title: name.to_string(), hidden: false, text: format_by_extension(name).to_string() }
}

/// Why a file the raster probe refused (`e`) and that isn't a readable metafile has no thumbnail
/// and isn't inserted: a file that starts like a metafile is a damaged one.
fn refusal(e: &picture::Refusal, bytes: &[u8]) -> String {
    match e {
        picture::Refusal::NotAPicture if craft_metafile::is_metafile(bytes) => match craft_metafile::size(bytes) {
            Err(why) => format!("the metafile is damaged ({why})"),
            Ok(_) => "the metafile is damaged (its picture has no size)".into(),
        },
        picture::Refusal::NotAPicture => NOT_A_PICTURE.into(),
        _ => e.to_string(),
    }
}

/// What a Windows Metafile or Enhanced Metafile's header says (#802).
struct Metafile {
    /// Its frame in pixels at 96 ppi, at least one each way.
    width: u32,
    height: u32,
    /// Its frame in points, as Insert › Pictures sizes it: at most 22 inches on the longer side.
    points: (f32, f32),
}

/// The metafile in `bytes`, or None for anything else (or a metafile with no picture to draw).
fn metafile(bytes: &[u8]) -> Option<Metafile> {
    let (w, h, _ext) = wordcraft_render::metafile_size(bytes)?;
    // `as` saturates; the frame is at most 1,584 points (2,112 pixels) a side.
    let px = |pt: f32| ((f64::from(pt) / 0.75).round() as u32).max(1);
    Some(Metafile { width: px(w), height: px(h), points: (w, h) })
}

/// The format a picture's extension names, for search (`PNG`, `JPEG`… `WMF`, `EMF`).
fn format_by_extension(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "PNG",
        "jpg" | "jpeg" => "JPEG",
        "gif" => "GIF",
        "webp" => "WebP",
        "bmp" => "BMP",
        "tif" | "tiff" => "TIFF",
        "wmf" => "WMF",
        "emf" => "EMF",
        _ => "picture",
    }
}

/// The size a `w` × `h` picture is drawn at for a thumbnail `width` pixels wide: never wider than
/// the picture itself (the pane scales a small one up) and never taller than
/// [`MAX_THUMB_HEIGHT`], keeping its shape; at least a pixel each way.
pub fn thumbnail_size(w: u32, h: u32, width: u32) -> (u32, u32) {
    fit_thumbnail(f64::from(w.max(1)), f64::from(h.max(1)), width, false)
}

/// The size a vector picture `w` × `h` (in any unit) is drawn at for a thumbnail `width` pixels
/// wide: that wide, since vectors are sharp at any size, unless it would be taller than
/// [`MAX_THUMB_HEIGHT`], keeping its shape; at least a pixel each way. A side that isn't a
/// positive number counts as 1.
pub fn vector_thumbnail_size(w: f32, h: f32, width: u32) -> (u32, u32) {
    let side = |v: f32| if v.is_finite() && v > 0.0 { f64::from(v) } else { 1.0 };
    fit_thumbnail(side(w), side(h), width, true)
}

/// [`thumbnail_size`] for positive `fw` × `fh`, scaled up to `width` too when `enlarge`.
fn fit_thumbnail(fw: f64, fh: f64, width: u32, enlarge: bool) -> (u32, u32) {
    let mut scale = f64::from(width.max(1)) / fw;
    if !enlarge {
        scale = scale.min(1.0);
    }
    if fh * scale > f64::from(MAX_THUMB_HEIGHT) {
        scale = f64::from(MAX_THUMB_HEIGHT) / fh;
    }
    // `as` saturates (and makes NaN 0), and the clamps keep both sides within the thumbnail's
    // bounds.
    let tw = ((fw * scale).round() as u32).clamp(1, width.max(1));
    let th = ((fh * scale).round() as u32).clamp(1, MAX_THUMB_HEIGHT);
    (tw, th)
}

/// Draw the picture in `bytes` at most `width` pixels wide as PNG. A raster picture is decoded
/// within the budget drawing it on a page has ([`wordcraft_geom::MAX_DECODED_BYTES`]), so a
/// picture that can't be drawn has no thumbnail either; a metafile (WMF, EMF) is drawn as pages
/// draw it ([`wordcraft_render::render_metafile`]), the full width.
pub fn draw_thumbnail(bytes: &[u8], width: u32) -> Result<Thumbnail, String> {
    let width = width.clamp(*THUMB_WIDTHS.start(), *THUMB_WIDTHS.end());
    let probe = match picture::probe(bytes) {
        Ok(probe) => probe,
        Err(e) => {
            let Some(meta) = metafile(bytes) else { return Err(refusal(&e, bytes)) };
            return draw_metafile(bytes, &meta, width);
        }
    };
    if probe.width == 0 || probe.height == 0 {
        return Err("the picture has no pixels".into());
    }
    let img = probe.decode(wordcraft_geom::MAX_DECODED_BYTES).map_err(|e| e.to_string())?;
    let (w, h) = thumbnail_size(img.width(), img.height(), width);
    let small = DynamicImage::ImageRgba8(img.thumbnail_exact(w, h).to_rgba8());
    let png = picture::encode_png(&small)?;
    Ok(Thumbnail { png: Arc::new(png), width: small.width(), height: small.height() })
}

/// A metafile's thumbnail `width` pixels wide (fewer when it is very tall), transparent where it
/// draws nothing, as the picture is on a page.
fn draw_metafile(bytes: &[u8], meta: &Metafile, width: u32) -> Result<Thumbnail, String> {
    let (w, h) = vector_thumbnail_size(meta.points.0, meta.points.1, width);
    let drawn = wordcraft_render::render_metafile(bytes, w, h).ok_or("the metafile is damaged (it has no picture to draw)")?;
    let png = drawn.to_png();
    if png.is_empty() {
        return Err("the thumbnail could not be encoded".into());
    }
    Ok(Thumbnail { png: Arc::new(png), width: drawn.width, height: drawn.height })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_named_by_extension() {
        for yes in ["a.png", "A.JPG", "b.jpeg", "c.gif", "d.webp", "e.bmp", "f.tif", "g.TIFF", "h.wmf", "i.EMF"] {
            assert!(is_picture(yes), "{yes}");
        }
        for no in ["~$a.png", ".a.png", "~$a.emf", "a.svg", "a.emz", "a.wmz", "a.docx", "png", "a.png.txt", "a.wmf.txt"] {
            assert!(!is_picture(no), "{no}");
        }
    }

    #[test]
    fn a_picture_is_known_from_its_name() {
        use craft_library::Kind;
        let item = Pictures.item_from_name("Harbour.JPG").unwrap();
        assert_eq!((item.title.as_str(), item.text.as_str(), item.hidden), ("Harbour.JPG", "JPEG", false));
        assert_eq!(Pictures.item_from_name("scan.tif").map(|i| i.text), Some("TIFF".into()));
        assert_eq!(Pictures.item_from_name("clip.EMF").map(|i| i.text), Some("EMF".into()));
        assert_eq!(Pictures.item_from_name("logo.wmf").map(|i| i.text), Some("WMF".into()));
        for no in ["notes.txt", ".hidden.png", "~$lock.png", "drawing.svg"] {
            assert!(Pictures.item_from_name(no).is_none(), "{no}");
        }
    }

    #[test]
    fn thumbnails_keep_the_shape_within_bounds() {
        assert_eq!(thumbnail_size(1600, 900, 160), (160, 90));
        // A small picture is not enlarged.
        assert_eq!(thumbnail_size(40, 20, 160), (40, 20));
        // A very tall one is drawn narrower, no taller than the limit.
        let (w, h) = thumbnail_size(10, 1_000_000, 512);
        assert_eq!(h, MAX_THUMB_HEIGHT);
        assert_eq!(w, 1);
        // Hostile sizes stay finite and at least a pixel.
        assert_eq!(thumbnail_size(0, 0, 0), (1, 1));
        assert_eq!(thumbnail_size(u32::MAX, 1, 512), (512, 1));
        assert_eq!(thumbnail_size(1, u32::MAX, 512), (1, MAX_THUMB_HEIGHT));
    }

    #[test]
    fn vector_thumbnails_are_drawn_the_full_width_within_bounds() {
        assert_eq!(vector_thumbnail_size(144.0, 72.0, 160), (160, 80));
        // A small frame is drawn the full width: vectors stay sharp.
        assert_eq!(vector_thumbnail_size(10.0, 5.0, 512), (512, 256));
        // A very tall one is drawn narrower, no taller than the limit.
        assert_eq!(vector_thumbnail_size(10.0, 1584.0, 512), (13, MAX_THUMB_HEIGHT));
        // Hostile sizes stay finite and at least a pixel.
        for (w, h) in [(0.0, 0.0), (f32::NAN, 1.0), (-5.0, f32::INFINITY), (f32::MAX, f32::MIN_POSITIVE), (f32::MIN_POSITIVE, f32::MAX)] {
            let (tw, th) = vector_thumbnail_size(w, h, 512);
            assert!((1..=512).contains(&tw) && (1..=MAX_THUMB_HEIGHT).contains(&th), "{w}×{h}: {tw}×{th}");
        }
        assert_eq!(vector_thumbnail_size(1.0, 1.0, 0), (1, 1));
    }
}
