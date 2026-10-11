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
    /// Copied files (a file manager's Copy); the pictures among them are inserted.
    Files(Vec<PathBuf>),
}

/// Longest side of a pasted picture, in pixels.
pub const MAX_SIDE: usize = 16_000;
/// Most pixels in a pasted picture (200 MB of RGBA).
pub const MAX_PIXELS: usize = 50_000_000;
/// Largest picture file pasted (as `insert.picture` allows).
pub const MAX_FILE_BYTES: u64 = 200 << 20;
/// Most picture files one paste inserts.
const MAX_FILES: usize = 20;
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

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

fn is_picture_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGE_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
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
    Some(r)
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
}
