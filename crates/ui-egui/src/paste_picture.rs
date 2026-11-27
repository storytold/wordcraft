//! Pasting a picture from the system clipboard (#45).
//!
//! A clipboard holding just a picture (a screenshot, a browser's Copy Image) or a copied picture
//! file pastes nothing through egui's text-only paste. The picture is read through the installed
//! clipboard, whose reads are bounded by its limits (the clipboard comes from other programs);
//! this module keeps WordCraft's own rules for copied files and inserts the result with
//! `insert.picture`, one undoable step like any inserted picture.

use std::path::{Path, PathBuf};

// PLAN(app-hub): the `craft` facade dependency of ui-egui (`craft::platform` = craft-platform), as in system_clipboard.rs
use craft::platform::clipboard::ClipboardError;
use serde_json::{Value, json};

use crate::{WordApp, system_clipboard};

/// Largest picture file pasted (as `insert.picture` allows).
pub(crate) const MAX_FILE_BYTES: u64 = 200 << 20;
/// Most picture files one paste inserts.
const MAX_FILES: usize = 20;

/// PNG bytes for clipboard pixels, or why they can't be pasted. The platform read has checked the
/// size already.
pub fn png_from_rgba(width: usize, height: usize, rgba: Vec<u8>) -> Result<Vec<u8>, String> {
    let (Ok(w), Ok(h)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err("the clipboard picture is too large to paste".into());
    };
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("the clipboard picture is damaged (its size doesn't match its pixels)")?;
    wordcraft_geom::picture::encode_png(&image::DynamicImage::ImageRgba8(img)).map_err(|e| format!("couldn't read the clipboard picture: {e}"))
}

fn is_picture_file(path: &Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|e| crate::file_dialogs::PICTURES.iter().any(|x| x.eq_ignore_ascii_case(e)))
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

/// Insert the picture files among `files`. `None` when none of them is a picture, so the usual
/// paste applies.
fn paste_files(app: &mut WordApp, files: Vec<PathBuf>) -> Option<Result<Value, String>> {
    let files: Vec<PathBuf> = files.into_iter().filter(|p| is_picture_file(p)).take(MAX_FILES).collect();
    if files.is_empty() {
        return None;
    }
    let mut r = Err(String::new());
    for path in files {
        // A clipboard path can name a pipe or device that reports no length and never ends.
        let md = std::fs::metadata(&path).ok();
        let size = md.as_ref().map_or(0, |m| m.len());
        r = if md.as_ref().is_some_and(|m| !m.is_file()) {
            let e = format!("{} is not a regular file", path.display());
            app.status(e.clone());
            Err(e)
        } else if size > MAX_FILE_BYTES {
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
    Some(r)
}

/// Paste the picture on the system clipboard: copied picture files, else its bitmap. `None` when
/// there is none (or no clipboard): the caller pastes as usual.
pub(crate) fn paste(app: &mut WordApp) -> Option<Result<Value, String>> {
    let clip = system_clipboard::handle(app)?;
    let r = match clip.read_files().ok().flatten().filter(|f| !f.is_empty()) {
        Some(files) => paste_files(app, files)?,
        None => match clip.read_image() {
            Ok(None) => return None,
            Ok(Some(image)) => match png_from_rgba(image.width as usize, image.height as usize, image.rgba) {
                Ok(png) => app.execute("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png)})),
                Err(e) => {
                    log::warn!("clipboard picture: {e}");
                    app.status(e.clone());
                    Err(e)
                }
            },
            Err(e) => {
                let e = match e {
                    ClipboardError::TooLarge { .. } => "the clipboard picture is too large to paste".to_string(),
                    e => crate::i18n::fmt(tl!("The picture on the clipboard couldn't be read: {error}"), &[("error", &e.to_string())]),
                };
                log::warn!("clipboard picture: {e}");
                app.status(e.clone());
                Err(e)
            }
        },
    };
    // Selected once, after the last picture: a selected picture is replaced by the next insert.
    app.select_inserted_picture(&r);
    Some(r)
}

/// The picture on the system clipboard as PNG (or the first copied picture file's bytes), for
/// Paste Special to offer. `None` when there is none, or it is too large or unreadable.
pub(crate) fn clipboard_png(app: &WordApp) -> Option<Vec<u8>> {
    let clip = system_clipboard::handle(app)?;
    if let Some(image) = clip.read_image().ok().flatten() {
        return png_from_rgba(image.width as usize, image.height as usize, image.rgba).ok();
    }
    let path = clip.read_files().ok().flatten()?.into_iter().take(MAX_FILES).find(|p| is_picture_file(p))?;
    read_picture_file(&path, MAX_FILE_BYTES.min(wordcraft_engine::cmd::paste::MAX_PICTURE_BYTES as u64 / 4 * 3))
}

/// The bytes of the regular file at `path` when it holds at most `limit` of them; `None` for
/// anything else, such as a device like `/dev/zero` that never ends (#3692). A file that says it
/// is larger is refused before any of it is read.
fn read_picture_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    wordcraft_engine::io::read_regular_file_within(path, limit)
}

/// Paste text naming copied picture files: insert the pictures when the clipboard holds those
/// files. False leaves the text to the usual paste.
pub(crate) fn paste_named_files(app: &mut WordApp, text: &str) -> bool {
    names_picture_files(text) && paste(app).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_clipboard::testing::{app_holding, rgba8};
    use craft::platform::clipboard::{Flavor, RGBA8, URI_LIST};

    fn pictures(a: &mut WordApp) -> usize {
        a.session.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(|x| x.len()).unwrap_or(0)
    }

    fn uri_list(paths: &[PathBuf]) -> Flavor {
        let lines: Vec<String> = paths.iter().map(|p| format!("file://{}\r\n", p.display())).collect();
        Flavor::new(URI_LIST, lines.concat().into_bytes())
    }

    /// Hostile or damaged clipboard pictures and non-picture files are refused without panicking.
    #[test]
    fn hostile_clipboard_pictures_are_refused() {
        assert!(png_from_rgba(usize::MAX, 2, Vec::new()).is_err());
        assert!(png_from_rgba(10_000, 10_000, Vec::new()).is_err());
        assert!(png_from_rgba(4, 4, vec![0; 63]).is_err());
        assert!(png_from_rgba(0, 4, Vec::new()).is_err());

        // Damaged RGBA8 pixels buffer on the clipboard.
        let mut damaged = Vec::new();
        damaged.extend_from_slice(&(4u32).to_le_bytes());
        damaged.extend_from_slice(&(4u32).to_le_bytes());
        damaged.extend_from_slice(&[0u8; 10]);
        let mut a = app_holding(&[Flavor::new(RGBA8, damaged)]);
        assert!(a.run("edit.paste", json!({})).is_err());
        assert_eq!(pictures(&mut a), 0);

        // Copied files that aren't pictures leave Paste to the usual clipboard.
        let mut a = app_holding(&[uri_list(&[PathBuf::from("/tmp/notes.txt")])]);
        assert_eq!(a.run("edit.paste", json!({})).unwrap_err(), system_clipboard::nothing_to_paste(&a));
        assert_eq!(pictures(&mut a), 0);
    }

    /// Paste with only a picture on the clipboard inserts it at its natural size, one undo step.
    #[test]
    fn paste_inserts_clipboard_pixels() {
        let mut a = app_holding(&[rgba8(40, 20, 200)]);
        let r = a.run("edit.paste", json!({})).unwrap();
        // 96 ppi: 40 × 20 px is 30 × 15 pt.
        assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(30.0), Some(15.0)));
        assert_eq!(pictures(&mut a), 1);
        let media = a.session.doc.media.values().next().unwrap();
        assert_eq!(wordcraft_render::image_size(media), Some((40, 20)));
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(pictures(&mut a), 0);
        // `/tmp/…` has no drive, so it isn't absolute on Windows.
        let abs = if cfg!(windows) { r"C:\tmp\b.JPG" } else { "/tmp/b.JPG" };
        assert!(names_picture_files(&format!("file:///home/me/shot%201.png\n{abs}\n")));
        assert!(!names_picture_files("see /tmp/b.png") && !names_picture_files("notes.txt") && !names_picture_files(""));
    }

    /// Text on the clipboard wins over a picture beside it (#584).
    #[test]
    fn paste_without_text_reads_the_system_clipboard() {
        let mut a = app_holding(&[Flavor::text("outside"), rgba8(4, 4, 200)]);
        a.run("edit.paste", json!({})).unwrap();
        a.run("edit.pasteText", json!({})).unwrap();
        a.run("edit.pasteMerge", json!({})).unwrap();
        assert_eq!(pictures(&mut a), 0);
        let focus = a.session.sel.focus.clone();
        assert_eq!(a.session.doc.para_at(&focus).map(|p| p.text.clone()), Some("outside".repeat(3)));
        let r = a.run("edit.pasteSpecial", json!({})).unwrap();
        assert!(r["formats"].as_array().is_some_and(|f| f.iter().any(|x| x["id"] == "text")), "{r}");
    }

    /// Copied picture files insert as pictures, not as their paths; a folder entry and a
    /// non-picture file are skipped, and files that aren't pictures leave Paste to the usual path.
    #[test]
    fn ribbon_paste_of_copied_picture_files_inserts_them() {
        let dir = std::env::temp_dir().join(format!("wc-ribbon-paste-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let png = dir.join("wc-ribbon-paste.png");
        std::fs::write(&png, png_from_rgba(4, 4, vec![200; 64]).unwrap()).unwrap();
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, "notes").unwrap();
        let mut a = app_holding(&[uri_list(&[dir.join("folder"), notes.clone(), png.clone()])]);
        let r = a.run("edit.paste", json!({}));
        let mut b = app_holding(&[uri_list(&[notes])]);
        let none = b.run("edit.paste", json!({}));
        let _ = std::fs::remove_dir_all(&dir);
        r.unwrap();
        assert_eq!(pictures(&mut a), 1);
        let focus = a.session.sel.focus.clone();
        assert_eq!(a.session.doc.para_at(&focus).map(|p| p.text.contains("wc-ribbon-paste")), Some(false));
        assert_eq!(none.unwrap_err(), system_clipboard::nothing_to_paste(&b));
    }

    /// A pasted picture is selected, with handles (#965); several copied files are all kept and
    /// the last is selected.
    #[test]
    fn pasted_pictures_are_selected() {
        let mut a = app_holding(&[rgba8(4, 4, 200)]);
        a.run("edit.paste", json!({})).unwrap();
        assert!(!a.session.sel.is_collapsed() && crate::objects::selected(&a).is_some(), "pixels: the pasted picture is selected");
        let dir = std::env::temp_dir().join(format!("wc-paste-965-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = (0..2).map(|i| dir.join(format!("{i}.png"))).collect();
        for f in &files {
            std::fs::write(f, png_from_rgba(4, 4, vec![200; 64]).unwrap()).unwrap();
        }
        let mut a = app_holding(&[uri_list(&files)]);
        let r = a.run("edit.paste", json!({}));
        let _ = std::fs::remove_dir_all(&dir);
        r.unwrap();
        assert_eq!(pictures(&mut a), 2, "files: both pictures are kept");
        assert!(!a.session.sel.is_collapsed() && crate::objects::selected(&a).is_some(), "files: the last picture is selected");
    }
}
