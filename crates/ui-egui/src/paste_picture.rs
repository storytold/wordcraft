//! Pasting a picture from the system clipboard (#45).
//!
//! egui's paste event only carries text, so a clipboard holding just a picture (a screenshot, a
//! browser's Copy Image) or a copied picture file used to paste nothing. The host reads the
//! clipboard through [`crate::Services::clipboard_picture`]; this module checks what it got — the
//! clipboard comes from other programs, so sizes are hostile — and inserts it with
//! `insert.picture`, one undoable step like any inserted picture.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::WordApp;

/// What the host found on the system clipboard.
pub enum ClipboardPicture {
    /// Straight (not premultiplied) RGBA8 pixels, row by row.
    Pixels { width: usize, height: usize, rgba: Vec<u8> },
    /// An encoded picture (PNG, JPEG, GIF, WebP, BMP) as another program offered it.
    Encoded(Vec<u8>),
    /// Copied files (a file manager's Copy); the pictures among them are inserted.
    Files(Vec<PathBuf>),
    /// A picture the host couldn't read, and why (its format isn't supported, the clipboard
    /// couldn't be reached) — said instead of "the clipboard is empty" (#833).
    Unreadable(String),
}

/// Longest side of a pasted picture, in pixels.
pub const MAX_SIDE: usize = 16_000;
/// Most pixels in a pasted picture (200 MB of RGBA).
pub const MAX_PIXELS: usize = 50_000_000;
/// Largest picture file pasted (as `insert.picture` allows).
pub const MAX_FILE_BYTES: u64 = 200 << 20;
/// Most picture files one paste inserts.
const MAX_FILES: usize = 20;
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "wmf", "emf"];

/// PNG bytes for clipboard pixels, or why they can't be pasted.
pub fn png_from_rgba(width: usize, height: usize, rgba: Vec<u8>) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 {
        return Err("the clipboard picture is empty".into());
    }
    let pixels = width.checked_mul(height).filter(|&n| width <= MAX_SIDE && height <= MAX_SIDE && n <= MAX_PIXELS);
    let Some(pixels) = pixels else {
        return Err(format!("the clipboard picture is too large to paste ({width} × {height} pixels)"));
    };
    if Some(rgba.len()) != pixels.checked_mul(4) {
        return Err("the clipboard picture is damaged (its size doesn't match its pixels)".into());
    }
    let (Ok(w), Ok(h)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err("the clipboard picture is too large to paste".into());
    };
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("the clipboard picture is damaged")?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| format!("couldn't read the clipboard picture: {e}"))?;
    Ok(png)
}

/// Whether a paste takes a `width` × `height` picture: the sides and pixels [`png_from_rgba`]
/// allows.
fn pastes(width: u32, height: u32) -> bool {
    let (w, h) = (width as usize, height as usize);
    w <= MAX_SIDE && h <= MAX_SIDE && w.checked_mul(h).is_some_and(|n| n <= MAX_PIXELS)
}

fn too_large_to_paste(width: u32, height: u32) -> ClipboardPicture {
    ClipboardPicture::Unreadable(format!("the clipboard picture is too large to paste ({width} × {height} pixels)"))
}

/// An encoded picture the host read from the clipboard (the Windows "PNG" format), refused from
/// its header when a paste can't take its size, before anything decodes it (#3623).
pub fn encoded_picture(bytes: Vec<u8>) -> ClipboardPicture {
    match wordcraft_geom::picture::probe(&bytes).map(|p| (p.width, p.height)) {
        Ok((w, h)) if pastes(w, h) => ClipboardPicture::Encoded(bytes),
        Ok((w, h)) => too_large_to_paste(w, h),
        Err(e) => ClipboardPicture::Unreadable(e.to_string()),
    }
}

/// A DIB from the Windows clipboard (`CF_DIBV5`, `CF_DIB`) as pixels: its size is checked from
/// its header, then it is decoded within the decode budget. arboard decoded it with no limits
/// before anything was checked (#3623).
pub fn dib_picture(dib: Vec<u8>) -> ClipboardPicture {
    let p = match wordcraft_geom::picture::probe_dib(dib) {
        Ok(p) => p,
        Err(e) => return ClipboardPicture::Unreadable(e.to_string()),
    };
    if !pastes(p.width, p.height) {
        return too_large_to_paste(p.width, p.height);
    }
    match p.decode(wordcraft_geom::MAX_DECODED_BYTES) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            ClipboardPicture::Pixels { width: rgba.width() as usize, height: rgba.height() as usize, rgba: rgba.into_raw() }
        }
        Err(e) => ClipboardPicture::Unreadable(e.to_string()),
    }
}

fn is_picture_file(path: &Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|e| IMAGE_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Pasted text that is only paths or `file://` addresses of picture files — what file managers
/// put on the clipboard as text when files are copied.
pub fn names_picture_files(text: &str) -> bool {
    if text.len() > 64 << 10 {
        return false;
    }
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).peekable();
    lines.peek().is_some()
        && lines.take(MAX_FILES + 1).enumerate().all(|(i, l)| {
            let path = l.strip_prefix("file://").unwrap_or(l);
            i < MAX_FILES && (l.starts_with("file://") || Path::new(path).is_absolute()) && is_picture_file(Path::new(path))
        })
}

/// Paste the picture on the system clipboard. `None` when there is none (or no clipboard
/// service): the caller pastes as usual.
pub(crate) fn paste(app: &mut WordApp) -> Option<Result<Value, String>> {
    let picture = app.services.clipboard_picture.as_ref()?()?;
    let r = match picture {
        ClipboardPicture::Pixels { width, height, rgba } => match png_from_rgba(width, height, rgba) {
            Ok(png) => app.execute("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png)})),
            Err(e) => {
                log::warn!("clipboard picture: {e}");
                app.status(e.clone());
                Err(e)
            }
        },
        ClipboardPicture::Encoded(bytes) if bytes.len() as u64 > MAX_FILE_BYTES => {
            let e = "the clipboard picture is larger than 200 MB".to_string();
            app.status(e.clone());
            Err(e)
        }
        ClipboardPicture::Encoded(bytes) => app.execute("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&bytes)})),
        ClipboardPicture::Unreadable(why) => {
            let e = format!("the picture on the clipboard couldn't be read: {why}");
            log::warn!("clipboard picture: {e}");
            app.status(e.clone());
            Err(e)
        }
        ClipboardPicture::Files(files) => {
            let files: Vec<PathBuf> = files.into_iter().filter(|p| is_picture_file(p)).take(MAX_FILES).collect();
            if files.is_empty() {
                return None;
            }
            let mut r = Err(String::new());
            for path in files {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                r = if size > MAX_FILE_BYTES {
                    let e = format!("{} is larger than 200 MB", path.display());
                    app.status(e.clone());
                    Err(e)
                } else {
                    app.execute("insert.picture", json!({"path": path.to_string_lossy()}))
                };
                if r.is_err() {
                    break;
                }
            }
            r
        }
    };
    // Selected once, after the last picture: a selected picture is replaced by the next insert.
    app.select_inserted_picture(&r);
    Some(r)
}

/// The picture on the system clipboard as PNG (or the first copied picture file's bytes), for
/// Paste Special to offer. `None` when there is none, or it is too large or unreadable.
pub(crate) fn clipboard_png(app: &WordApp) -> Option<Vec<u8>> {
    match app.services.clipboard_picture.as_ref()?()? {
        ClipboardPicture::Pixels { width, height, rgba } => png_from_rgba(width, height, rgba).ok(),
        ClipboardPicture::Encoded(bytes) => Some(bytes),
        ClipboardPicture::Files(files) => {
            let path = files.into_iter().take(MAX_FILES).find(|p| is_picture_file(p))?;
            let size = std::fs::metadata(&path).ok()?.len();
            if size > MAX_FILE_BYTES.min(wordcraft_engine::cmd::paste::MAX_PICTURE_BYTES as u64 / 4 * 3) {
                return None;
            }
            std::fs::read(path).ok()
        }
        ClipboardPicture::Unreadable(_) => None,
    }
}

/// Paste text naming copied picture files: insert the pictures when the clipboard holds those
/// files. False leaves the text to the usual paste.
pub(crate) fn paste_named_files(app: &mut WordApp, text: &str) -> bool {
    names_picture_files(text) && paste(app).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use wordcraft_engine::Session;

    fn app_with(picture: impl Fn() -> Option<ClipboardPicture> + 'static) -> WordApp {
        let services = Services { clipboard_picture: Some(Box::new(picture)), ..Default::default() };
        WordApp::new(Session::new(wordcraft_doc::Document::new()), services)
    }

    fn pictures(a: &mut WordApp) -> usize {
        a.session.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(|x| x.len()).unwrap_or(0)
    }

    /// Paste with only a picture on the clipboard inserts it at its natural size, one undo step.
    #[test]
    fn paste_inserts_clipboard_pixels() {
        let mut a = app_with(|| Some(ClipboardPicture::Pixels { width: 40, height: 20, rgba: vec![200; 40 * 20 * 4] }));
        let r = a.run("edit.paste", json!({})).unwrap();
        // 96 ppi: 40 × 20 px is 30 × 15 pt.
        assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(30.0), Some(15.0)));
        assert_eq!(pictures(&mut a), 1);
        let media = a.session.doc.media.values().next().unwrap();
        assert_eq!(wordcraft_render::image_size(media), Some((40, 20)));
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(pictures(&mut a), 0);
        // Pasted text that names picture files only becomes pictures when files were copied.
        // `/tmp/…` has no drive, so it isn't absolute on Windows.
        let abs = if cfg!(windows) { r"C:\tmp\b.JPG" } else { "/tmp/b.JPG" };
        assert!(names_picture_files(&format!("file:///home/me/shot%201.png\n{abs}\n")));
        assert!(!names_picture_files("see /tmp/b.png") && !names_picture_files("notes.txt") && !names_picture_files(""));
    }

    /// A BITMAPV5HEADER DIB of 32-bit `BI_RGB` pixels (bottom-up) with an alpha mask, as Windows
    /// programs put one on the clipboard.
    fn dib(w: i32, h: i32, pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut d = vec![0u8; 124];
        d[0..4].copy_from_slice(&124u32.to_le_bytes());
        d[4..8].copy_from_slice(&w.to_le_bytes());
        d[8..12].copy_from_slice(&h.to_le_bytes());
        d[12..14].copy_from_slice(&1u16.to_le_bytes());
        d[14..16].copy_from_slice(&32u16.to_le_bytes());
        d[52..56].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
        d.extend(pixels.iter().flatten());
        d
    }

    /// #3623: what the Windows clipboard holds is checked from its header before it is decoded.
    #[test]
    fn windows_clipboard_pictures_are_checked_from_their_headers() {
        let mut png = Vec::new();
        image::RgbaImage::new(3, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert!(matches!(encoded_picture(png.clone()), ClipboardPicture::Encoded(b) if b == png));
        let mut wide = Vec::new();
        image::GrayImage::new(20_000, 1).write_to(&mut std::io::Cursor::new(&mut wide), image::ImageFormat::Png).unwrap();
        assert!(matches!(encoded_picture(wide), ClipboardPicture::Unreadable(e) if e.contains("too large")));
        assert!(matches!(encoded_picture(b"not a picture".to_vec()), ClipboardPicture::Unreadable(_)));
        // BGRA in memory: a half-transparent red pixel and an opaque blue one.
        let ClipboardPicture::Pixels { width, height, rgba } = dib_picture(dib(2, 1, &[[0, 0, 255, 128], [255, 0, 0, 255]])) else {
            panic!("pixels")
        };
        assert_eq!((width, height, rgba), (2, 1, vec![255, 0, 0, 128, 0, 0, 255, 255]));
        // A header claiming 60000 × 60000 over no pixels is refused unread.
        assert!(matches!(dib_picture(dib(60_000, 60_000, &[])), ClipboardPicture::Unreadable(e) if e.contains("too large")));
        assert!(matches!(dib_picture(vec![0; 10]), ClipboardPicture::Unreadable(_)));
        // And a DIB pastes as a picture.
        let mut a = app_with(|| Some(dib_picture(dib(2, 2, &[[9, 9, 9, 255]; 4]))));
        a.run("edit.paste", json!({})).unwrap();
        assert_eq!(pictures(&mut a), 1);
    }

    /// Hostile clipboard sizes are refused without inserting anything (or allocating for them).
    #[test]
    fn hostile_clipboard_pictures_are_refused() {
        assert!(png_from_rgba(100_000, 1, vec![0; 400_000]).is_err());
        assert!(png_from_rgba(usize::MAX, 2, Vec::new()).is_err());
        assert!(png_from_rgba(10_000, 10_000, Vec::new()).is_err());
        assert!(png_from_rgba(4, 4, vec![0; 63]).is_err());
        assert!(png_from_rgba(0, 4, Vec::new()).is_err());
        let mut a = app_with(|| Some(ClipboardPicture::Pixels { width: 1 << 40, height: 1 << 40, rgba: Vec::new() }));
        assert!(a.run("edit.paste", json!({})).is_err());
        assert_eq!(pictures(&mut a), 0);
        // Copied files that aren't pictures leave Paste to the usual clipboard.
        let mut a = app_with(|| Some(ClipboardPicture::Files(vec![PathBuf::from("/tmp/notes.txt")])));
        assert_eq!(a.run("edit.paste", json!({})).unwrap_err(), "the clipboard is empty");
    }

    /// The ribbon and context-menu Paste commands send no text; the system clipboard's text is
    /// pasted, as Ctrl+V pastes it, even with a picture copied alongside (#584).
    #[test]
    fn paste_without_text_reads_the_system_clipboard() {
        let services = Services {
            clipboard_text: Some(Box::new(|| Some("outside".to_string()))),
            clipboard_picture: Some(Box::new(|| Some(ClipboardPicture::Pixels { width: 4, height: 4, rgba: vec![200; 4 * 4 * 4] }))),
            ..Default::default()
        };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("edit.paste", json!({})).unwrap();
        a.run("edit.pasteText", json!({})).unwrap();
        a.run("edit.pasteMerge", json!({})).unwrap();
        assert_eq!(pictures(&mut a), 0);
        let focus = a.session.sel.focus.clone();
        assert_eq!(a.session.doc.para_at(&focus).map(|p| p.text.clone()), Some("outside".repeat(3)));
        // Paste Special offers the clipboard's text too.
        let r = a.run("edit.pasteSpecial", json!({})).unwrap();
        assert!(r["formats"].as_array().is_some_and(|f| f.iter().any(|x| x["id"] == "text")), "{r}");
    }

    /// Ribbon Paste after copying a picture file in a file manager inserts the picture, not its
    /// path, as Ctrl+V does.
    #[test]
    fn ribbon_paste_of_copied_picture_files_inserts_them() {
        let path = std::env::temp_dir().join(format!("wc-ribbon-paste-{}.png", std::process::id()));
        std::fs::write(&path, png_from_rgba(4, 4, vec![200; 4 * 4 * 4]).unwrap()).unwrap();
        let (text, file) = (path.to_string_lossy().into_owned(), path.clone());
        let services = Services {
            clipboard_text: Some(Box::new(move || Some(text.clone()))),
            clipboard_picture: Some(Box::new(move || Some(ClipboardPicture::Files(vec![file.clone()])))),
            ..Default::default()
        };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        let r = a.run("edit.paste", json!({}));
        let _ = std::fs::remove_file(&path);
        r.unwrap();
        assert_eq!(pictures(&mut a), 1);
        let focus = a.session.sel.focus.clone();
        assert_eq!(a.session.doc.para_at(&focus).map(|p| p.text.contains("wc-ribbon-paste")), Some(false));
    }

    /// A pasted picture is selected, with handles, as one inserted from Insert › Pictures is
    /// (#965); pasting several copied picture files keeps them all and selects the last.
    #[test]
    fn pasted_pictures_are_selected() {
        let mut a = app_with(|| Some(ClipboardPicture::Pixels { width: 4, height: 4, rgba: vec![200; 4 * 4 * 4] }));
        a.run("edit.paste", json!({})).unwrap();
        assert!(!a.session.sel.is_collapsed() && crate::objects::selected(&a).is_some(), "pixels: the pasted picture is selected");
        let dir = std::env::temp_dir().join(format!("wc-paste-965-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = (0..2).map(|i| dir.join(format!("{i}.png"))).collect();
        for f in &files {
            std::fs::write(f, png_from_rgba(4, 4, vec![200; 4 * 4 * 4]).unwrap()).unwrap();
        }
        let mut a = app_with(move || Some(ClipboardPicture::Files(files.clone())));
        let r = a.run("edit.paste", json!({}));
        let _ = std::fs::remove_dir_all(&dir);
        r.unwrap();
        assert_eq!(pictures(&mut a), 2, "files: both pictures are kept");
        assert!(!a.session.sel.is_collapsed() && crate::objects::selected(&a).is_some(), "files: the last picture is selected");
    }
}
