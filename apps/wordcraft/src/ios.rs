//! iPadOS and iOS: the system clipboard (UIPasteboard) and the app's folders.
//!
//! The desktop clipboard goes through arboard, which has no iOS backend, so the pasteboard is
//! reached here directly. It carries plain text, HTML, RTF and pictures (PNG, JPEG, TIFF, HEIC
//! read through the app's own picture decoder; PNG written).

use std::path::PathBuf;

use craft::platform::clipboard::{BITMAP, ClipboardBackend, ClipboardError, ClipboardImage, Flavor, HTML, PNG, RGBA8, RTF, TEXT};
use wordcraft_ios_pasteboard as pasteboard;

/// The MIME types the app uses and the uniform type identifiers the pasteboard keeps them under.
const TYPES: &[(&str, &str)] = &[(TEXT, "public.utf8-plain-text"), (HTML, "public.html"), (RTF, "public.rtf"), (PNG, "public.png")];

/// Bitmap types a paste reads, best first.
const PICTURE_TYPES: &[&str] = &["public.png", "public.tiff", "public.jpeg", "public.heic"];

/// The plain-text types other apps put on the pasteboard, besides UTF-8.
const OTHER_TEXT_TYPES: &[&str] = &["public.plain-text", "public.text", "public.utf16-plain-text"];

fn uti(mime: &str) -> Option<&'static str> {
    TYPES.iter().find(|(m, _)| *m == mime).map(|(_, u)| *u)
}

fn within(bytes: Vec<u8>, limit: u64) -> Result<Vec<u8>, ClipboardError> {
    if bytes.len() as u64 > limit { Err(ClipboardError::TooLarge { limit }) } else { Ok(bytes) }
}

/// A PNG of an `RGBA8` flavour, for a copy that offers its pixels only.
fn png_of_rgba8(flavor: &Flavor) -> Option<Vec<u8>> {
    let image = flavor.rgba8_image()?;
    let buffer = image::RgbaImage::from_raw(image.width, image.height, image.rgba)?;
    wordcraft_geom::picture::encode_png(&image::DynamicImage::ImageRgba8(buffer)).ok()
}

/// The general pasteboard, shared with every other app.
pub struct Pasteboard;

impl ClipboardBackend for Pasteboard {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        if flavors.is_empty() {
            pasteboard::set_item(&[]);
            return Ok(0);
        }
        let mut item: Vec<(&str, std::borrow::Cow<'_, [u8]>)> = Vec::new();
        let mut first = None;
        for (index, flavor) in flavors.iter().enumerate() {
            let (uti, bytes) = match uti(&flavor.mime) {
                Some(uti) => (uti, std::borrow::Cow::Borrowed(flavor.data.as_slice())),
                None if flavor.mime == RGBA8 => match png_of_rgba8(flavor) {
                    Some(png) => ("public.png", std::borrow::Cow::Owned(png)),
                    None => continue,
                },
                None => continue,
            };
            if item.iter().any(|(u, _)| *u == uti) {
                continue;
            }
            item.push((uti, bytes));
            first.get_or_insert(index);
        }
        let first = first.ok_or(ClipboardError::NoFormat)?;
        let item: Vec<(&str, &[u8])> = item.iter().map(|(u, b)| (*u, &**b)).collect();
        pasteboard::set_item(&item);
        Ok(first)
    }

    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        for &mime in mimes {
            if mime == BITMAP {
                if let Some(image) = self.read_image(limit)? {
                    return Ok(Some(Flavor::rgba8(&image)));
                }
                continue;
            }
            if mime == TEXT {
                // Any text another app copied, whichever text type it used.
                let text = pasteboard::string()
                    .or_else(|| OTHER_TEXT_TYPES.iter().find_map(|uti| pasteboard::data(uti).and_then(|b| String::from_utf8(b).ok())));
                if let Some(text) = text {
                    return Ok(Some(Flavor::new(TEXT, within(text.into_bytes(), limit)?)));
                }
                continue;
            }
            if let Some(uti) = uti(mime)
                && let Some(bytes) = pasteboard::data(uti)
            {
                return Ok(Some(Flavor::new(mime.to_string(), within(bytes, limit)?)));
            }
        }
        Ok(None)
    }

    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        for uti in PICTURE_TYPES {
            let Some(bytes) = pasteboard::data(uti) else { continue };
            let Ok(picture) = wordcraft_geom::picture::decode_memory(&bytes) else { continue };
            let rgba = picture.to_rgba8();
            let image = ClipboardImage { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() };
            if image.rgba.len() as u64 > limit {
                return Err(ClipboardError::TooLarge { limit });
            }
            return Ok(Some(image));
        }
        Ok(None)
    }

    fn read_files(&mut self, _limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        // Files arrive through the Files app, never as copied paths.
        Ok(None)
    }

    fn has(&mut self, mimes: &[&str]) -> bool {
        mimes.iter().any(|&mime| match mime {
            TEXT => pasteboard::has_strings(),
            BITMAP => pasteboard::has_images(),
            mime => uti(mime).is_some_and(|uti| pasteboard::contains(&[uti])),
        })
    }

    fn formats(&mut self) -> Vec<String> {
        let mut formats: Vec<String> = Vec::new();
        for uti in pasteboard::types() {
            let mime = TYPES.iter().find(|(_, u)| *u == uti).map(|(m, _)| (*m).to_string()).unwrap_or(uti);
            if !formats.contains(&mime) {
                formats.push(mime);
            }
        }
        formats
    }
}

/// The app's private folder for settings, logs, recovery, fonts and templates:
/// `Library/Application Support/WordCraft` in the app's container, which the person never sees.
pub fn settings_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support/WordCraft"))
}

/// The app's `Documents` folder: "On My iPad › WordCraft" in the Files app (the bundle's
/// `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`), where documents are opened
/// from and saved to.
pub fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Documents"))
}

/// Chooses the settings folder and makes the Documents folder, before anything reads either. A
/// folder that can't be made is logged: the app still starts, with settings in memory.
pub fn prepare_folders() {
    if let Some(dir) = settings_dir() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("settings folder {}: {e}", dir.display());
        }
        // A launch profile that chose its own folder keeps it.
        let _ = wordcraft_control_key::use_settings_dir(dir);
    }
    if let Some(dir) = documents_dir()
        && let Err(e) = std::fs::create_dir_all(&dir)
    {
        log::warn!("Documents folder {}: {e}", dir.display());
    }
}
