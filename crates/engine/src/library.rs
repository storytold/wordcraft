//! Insert ▸ Library (#2868): the pictures in the user's own folders, on the Library core the Craft
//! apps share ([`craft_library`], first asked for in storytold/deckcraft#958). This module is what
//! is WordCraft's own: the files its Pictures tab lists ([`Pictures`]: the raster pictures
//! WordCraft decodes, one picture per file, known from its name, so listing and search read no
//! file), how it opens one and draws its thumbnail, and its file access ([`Confined`]), which goes
//! through [`crate::io`] so the automation read roots confine every folder listing and file read.
//! The `library.*` commands are in [`crate::cmd::library`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use craft_library::{
    EntryKind, Item, MAX_DEPTH, MAX_ENTRIES, MAX_FILE_BYTES, MAX_FOLDERS, MAX_ITEMS, MAX_THUMB_HEIGHT, Nouns, Stamp, THUMB_WIDTH, THUMB_WIDTHS,
    Thumbnail, sanitized_folders,
};
use image::DynamicImage;
use wordcraft_geom::picture;

use crate::io;

pub mod drawer;

/// The files the Pictures tab lists: the raster pictures `insert.picture` decodes. Metafiles
/// (WMF, EMF) insert too, but have no thumbnail here yet, and SVG doesn't insert at all.
pub const EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];

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

    /// Reads only the header: a file that isn't a picture WordCraft decodes, or one it never
    /// decodes (a TIFF with JPEG strips, #3645), has no thumbnail and isn't inserted, and says why.
    fn open(&self, name: &str, bytes: &[u8]) -> Result<Picture, String> {
        let probe = picture::probe(bytes).map_err(|e| e.to_string())?;
        Ok(Picture { name: name.to_string(), bytes: bytes.to_vec(), width: probe.width, height: probe.height })
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

/// The format a picture's extension names, for search (`PNG`, `JPEG`…).
fn format_by_extension(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "PNG",
        "jpg" | "jpeg" => "JPEG",
        "gif" => "GIF",
        "webp" => "WebP",
        "bmp" => "BMP",
        "tif" | "tiff" => "TIFF",
        _ => "picture",
    }
}

/// The size a `w` × `h` picture is drawn at for a thumbnail `width` pixels wide: never wider than
/// the picture itself (the pane scales a small one up) and never taller than
/// [`MAX_THUMB_HEIGHT`], keeping its shape; at least a pixel each way.
pub fn thumbnail_size(w: u32, h: u32, width: u32) -> (u32, u32) {
    let (fw, fh) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let mut scale = (f64::from(width.max(1)) / fw).min(1.0);
    if fh * scale > f64::from(MAX_THUMB_HEIGHT) {
        scale = f64::from(MAX_THUMB_HEIGHT) / fh;
    }
    // `as` saturates, and the clamps keep both sides within the thumbnail's bounds.
    let tw = ((fw * scale).round() as u32).clamp(1, width.max(1));
    let th = ((fh * scale).round() as u32).clamp(1, MAX_THUMB_HEIGHT);
    (tw, th)
}

/// Draw the picture in `bytes` at most `width` pixels wide as PNG. It is decoded within the budget
/// drawing it on a page has ([`wordcraft_geom::MAX_DECODED_BYTES`]), so a picture that can't be
/// drawn has no thumbnail either.
pub fn draw_thumbnail(bytes: &[u8], width: u32) -> Result<Thumbnail, String> {
    let probe = picture::probe(bytes).map_err(|e| e.to_string())?;
    if probe.width == 0 || probe.height == 0 {
        return Err("the picture has no pixels".into());
    }
    let img = probe.decode(wordcraft_geom::MAX_DECODED_BYTES).map_err(|e| e.to_string())?;
    let (w, h) = thumbnail_size(img.width(), img.height(), width.clamp(*THUMB_WIDTHS.start(), *THUMB_WIDTHS.end()));
    let small = DynamicImage::ImageRgba8(img.thumbnail_exact(w, h).to_rgba8());
    let png = picture::encode_png(&small)?;
    Ok(Thumbnail { png: Arc::new(png), width: small.width(), height: small.height() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_named_by_extension() {
        for yes in ["a.png", "A.JPG", "b.jpeg", "c.gif", "d.webp", "e.bmp", "f.tif", "g.TIFF"] {
            assert!(is_picture(yes), "{yes}");
        }
        for no in ["~$a.png", ".a.png", "a.svg", "a.emf", "a.wmf", "a.docx", "png", "a.png.txt"] {
            assert!(!is_picture(no), "{no}");
        }
    }

    #[test]
    fn a_picture_is_known_from_its_name() {
        use craft_library::Kind;
        let item = Pictures.item_from_name("Harbour.JPG").unwrap();
        assert_eq!((item.title.as_str(), item.text.as_str(), item.hidden), ("Harbour.JPG", "JPEG", false));
        assert_eq!(Pictures.item_from_name("scan.tif").map(|i| i.text), Some("TIFF".into()));
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
}
