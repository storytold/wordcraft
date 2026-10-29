//! The system clipboard beyond egui's text (#45, #584, #687, #833): text for the ribbon's Paste,
//! another program's formatted copy (HTML, and RTF where the platform offers it), pictures and
//! copied files, and the names of the formats there for a paste that finds nothing it can use.
//!
//! On Linux, arboard talks to Wayland compositors through the data-control protocol (KDE Plasma,
//! wlroots compositors); without it, a native Wayland window only sees what the compositor copies
//! over to Xwayland, which KWin does only while an X11 window has focus — so pictures copied in a
//! browser or Spectacle never arrived (#833). Compositors without the protocol fall back to X11.
//!
//! Everything read here comes from other programs: sizes are capped as it is read.
//!
//! Written here: a copy puts HTML beside its text, so other programs keep its formatting (#1523),
//! with equations as MathML (#816). arboard sets HTML with a text alternative on every desktop
//! platform. A copy of one equation alone also goes in the platform's MathML formats (#1297),
//! added beside arboard's: on Windows "MathML" and "MathML Presentation" (UTF-16LE), on macOS
//! `public.mathml.presentation` and `public.mathml` — the names W3C MathML 3 §6.2.4 registers
//! for transferring MathML, and the ones Microsoft documents its Office apps reading first when
//! they paste math (learn.microsoft.com/office/math/mathml, "Clipboard support"). Pasting into
//! Word and LibreOffice is not yet verified. Linux and the BSDs have no Word, and arboard can't
//! add MIME types of other names, so there the HTML carries the MathML alone.

use wordcraft_engine::cmd::paste::MAX_PASTE_BYTES;
use wordcraft_ui_egui::paste_picture::ClipboardPicture;

fn open() -> Option<arboard::Clipboard> {
    arboard::Clipboard::new().map_err(|e| log::warn!("clipboard: {e}")).ok()
}

thread_local! {
    /// The clipboard copies are set through, kept open: on X11 the contents are served from it.
    static WRITER: std::cell::RefCell<Option<arboard::Clipboard>> = const { std::cell::RefCell::new(None) };
}

/// Put `html` on the clipboard with `text` as its plain-text alternative, and `mathml` (a copy of
/// one equation alone; empty for any other copy) in the platform's MathML formats. `false` when
/// the HTML and text couldn't be set (the caller copies the text alone), including a Wayland
/// session without the data-control protocol, where arboard would fall back to X11 and native
/// Wayland programs might not see it. MathML that can't be added is logged: the HTML still
/// carries it.
pub fn set_html(html: &str, text: &str, mathml: &str) -> bool {
    if !platform::can_set() {
        return false;
    }
    let set = set_html_text(html, text);
    if set && !mathml.is_empty() && mathml.len() <= MAX_PASTE_BYTES {
        platform::add_mathml(mathml);
    }
    set
}

fn set_html_text(html: &str, text: &str) -> bool {
    WRITER.with(|w| {
        let Ok(mut slot) = w.try_borrow_mut() else { return false };
        if slot.is_none() {
            *slot = open();
        }
        match slot.as_mut().map(|c| c.set().html(html, Some(text))) {
            Some(Ok(())) => true,
            Some(Err(e)) => {
                log::warn!("clipboard: {e}");
                *slot = None;
                false
            }
            None => false,
        }
    })
}

/// The clipboard's text.
pub fn text() -> Option<String> {
    open()?.get_text().ok()
}

/// The clipboard's HTML (Windows: the fragment of its "HTML Format"), if not too large.
pub fn html() -> Option<String> {
    open()?.get().html().ok().filter(|h| h.len() <= MAX_PASTE_BYTES)
}

/// The clipboard's RTF: Windows ("Rich Text Format") and Wayland (`text/rtf`); not offered by
/// arboard, so read here.
pub fn rtf() -> Option<String> {
    let bytes = platform::rtf()?;
    if bytes.len() > MAX_PASTE_BYTES {
        return None;
    }
    // RTF is 7-bit by the spec; stray 8-bit bytes become replacement characters.
    let s = String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string();
    s.trim_start().starts_with("{\\rtf").then_some(s)
}

/// The names of the formats on the clipboard, where the platform lists them (Wayland, Windows).
pub fn formats() -> Vec<String> {
    platform::formats()
}

/// The picture on the clipboard: copied files, else image pixels, else (Wayland) an encoded
/// picture in a format arboard doesn't decode. A picture that is there but can't be read says
/// why (#833). The UI checks the sizes before inserting anything; on Windows they are checked
/// from the header before anything is decoded (#3623).
pub fn picture() -> Option<ClipboardPicture> {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("clipboard: {e}");
            return Some(ClipboardPicture::Unreadable(format!("the system clipboard couldn't be opened ({e})")));
        }
    };
    if let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        return Some(ClipboardPicture::Files(files));
    }
    decoded_picture(&mut clipboard)
}

/// The clipboard's picture on Windows, read past arboard, which decodes the PNG and DIB formats
/// with no size limits (#3623).
#[cfg(target_os = "windows")]
fn decoded_picture(_: &mut arboard::Clipboard) -> Option<ClipboardPicture> {
    platform::picture()
}

/// The picture arboard decodes from the clipboard, else an encoded one read past it.
#[cfg(not(target_os = "windows"))]
fn decoded_picture(clipboard: &mut arboard::Clipboard) -> Option<ClipboardPicture> {
    match clipboard.get_image() {
        Ok(image) => Some(ClipboardPicture::Pixels { width: image.width, height: image.height, rgba: image.bytes.into_owned() }),
        Err(arboard::Error::ContentNotAvailable) => platform::encoded_picture().map(ClipboardPicture::Encoded),
        Err(arboard::Error::ConversionFailure) => Some(
            platform::encoded_picture()
                .map(ClipboardPicture::Encoded)
                .unwrap_or_else(|| ClipboardPicture::Unreadable("WordCraft can't decode its format".into())),
        ),
        Err(e) => {
            log::warn!("clipboard picture: {e}");
            platform::encoded_picture().map(ClipboardPicture::Encoded).or(Some(ClipboardPicture::Unreadable(e.to_string())))
        }
    }
}

/// Picture types read straight from Wayland when arboard (PNG only) finds none, best first.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))))]
const PICTURE_MIMES: [&str; 6] = ["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/x-bmp"];

#[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))))]
mod platform {
    use std::io::Read;

    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents, get_mime_types_ordered};

    fn wayland() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
    }

    /// One MIME type's bytes, at most `cap`; `None` when absent or larger.
    fn read(mime: &str, cap: u64) -> Option<Vec<u8>> {
        let (pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Specific(mime)).ok()?;
        let mut out = Vec::new();
        pipe.take(cap.saturating_add(1)).read_to_end(&mut out).ok()?;
        (out.len() as u64 <= cap).then_some(out)
    }

    pub fn formats() -> Vec<String> {
        if !wayland() {
            return Vec::new();
        }
        get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified).unwrap_or_default()
    }

    /// Whether arboard sets the clipboard natively: X11, or Wayland with data-control.
    pub fn can_set() -> bool {
        !wayland()
            || !matches!(
                get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified),
                Err(wl_clipboard_rs::paste::Error::MissingProtocol { .. } | wl_clipboard_rs::paste::Error::WaylandConnection(_))
            )
    }

    /// arboard serves its own MIME types only; Word, which reads the MathML formats, isn't here.
    pub fn add_mathml(_mathml: &str) {}

    pub fn rtf() -> Option<Vec<u8>> {
        if !wayland() {
            return None;
        }
        let offered = formats();
        let mime = ["text/rtf", "application/rtf", "text/richtext"].into_iter().find(|m| offered.iter().any(|o| o == m))?;
        read(mime, super::MAX_PASTE_BYTES as u64)
    }

    pub fn encoded_picture() -> Option<Vec<u8>> {
        if !wayland() {
            return None;
        }
        let offered = formats();
        let mime = super::PICTURE_MIMES.into_iter().find(|m| offered.iter().any(|o| o == m))?;
        read(mime, wordcraft_ui_egui::paste_picture::MAX_FILE_BYTES)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use clipboard_win::{Clipboard, EnumFormats, raw};
    use wordcraft_ui_egui::paste_picture::{self, ClipboardPicture};

    pub fn formats() -> Vec<String> {
        let Ok(_open) = Clipboard::new_attempts(10) else { return Vec::new() };
        // Standard formats are named too (CF_BITMAP, CF_UNICODETEXT…).
        EnumFormats::new().take(64).filter_map(raw::format_name_big).collect()
    }

    pub fn rtf() -> Option<Vec<u8>> {
        let format = raw::register_format("Rich Text Format")?;
        let _open = Clipboard::new_attempts(10).ok()?;
        if raw::size(format.get()).is_none_or(|n| n.get() > super::MAX_PASTE_BYTES) {
            return None;
        }
        let mut out = Vec::new();
        raw::get_vec(format.get(), &mut out).ok()?;
        Some(out)
    }

    /// The clipboard's picture, read without arboard: its "PNG" format as it is, else a DIB as
    /// pixels, each refused from its header when a paste can't take its size (#3623). None when
    /// it holds neither.
    pub fn picture() -> Option<ClipboardPicture> {
        use clipboard_win::formats::{CF_DIB, CF_DIBV5};
        let Ok(_open) = Clipboard::new_attempts(10) else {
            return Some(ClipboardPicture::Unreadable("the system clipboard couldn't be opened".into()));
        };
        if let Some(png) = raw::register_format("PNG").map(|f| f.get()).filter(|&f| raw::is_format_avail(f)) {
            return Some(read(png).map_or_else(ClipboardPicture::Unreadable, paste_picture::encoded_picture));
        }
        let dib = [CF_DIBV5, CF_DIB].into_iter().find(|&f| raw::is_format_avail(f))?;
        Some(read(dib).map_or_else(ClipboardPicture::Unreadable, paste_picture::dib_picture))
    }

    /// A clipboard format's bytes, refused past the largest picture a paste takes.
    fn read(format: u32) -> Result<Vec<u8>, String> {
        match raw::size(format) {
            Some(n) if n.get() as u64 <= paste_picture::MAX_FILE_BYTES => {}
            Some(_) => return Err("the clipboard picture is larger than 200 MB".into()),
            None => return Err("the clipboard picture is empty".into()),
        }
        let mut out = Vec::new();
        raw::get_vec(format, &mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }

    pub fn can_set() -> bool {
        true
    }

    /// Add the MathML to what arboard just set, as "MathML" and "MathML Presentation": UTF-16LE
    /// text (with its terminating NUL, as Windows text formats have).
    pub fn add_mathml(mathml: &str) {
        let Ok(_open) = Clipboard::new_attempts(10) else {
            log::warn!("clipboard: couldn't open it to add MathML");
            return;
        };
        let bytes: Vec<u8> = mathml.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
        for name in super::MATHML_FORMATS {
            let Some(format) = raw::register_format(name) else { continue };
            if let Err(e) = raw::set_without_clear(format.get(), &bytes) {
                log::warn!("clipboard: {name}: {e}");
            }
        }
    }
}

/// The clipboard format names for MathML that Windows programs read (W3C MathML 3 §6.2.4; Office
/// reads and writes both).
#[cfg(target_os = "windows")]
const MATHML_FORMATS: [&str; 2] = ["MathML", "MathML Presentation"];

#[cfg(target_os = "macos")]
mod platform {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;

    pub fn formats() -> Vec<String> {
        Vec::new()
    }

    pub fn rtf() -> Option<Vec<u8>> {
        None
    }

    pub fn encoded_picture() -> Option<Vec<u8>> {
        None
    }

    pub fn can_set() -> bool {
        true
    }

    /// Add the MathML to the pasteboard item arboard just wrote, as the MathML types (W3C MathML 3
    /// §6.2.4): presentation markup, and the generic type for readers that only know that one.
    pub fn add_mathml(mathml: &str) {
        let board = NSPasteboard::generalPasteboard();
        let text = NSString::from_str(mathml);
        for name in ["public.mathml.presentation", "public.mathml"] {
            if !board.setString_forType(&text, &NSString::from_str(name)) {
                log::warn!("clipboard: couldn't add {name}");
            }
        }
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(unix, not(any(target_os = "macos", target_os = "android", target_os = "emscripten")))
)))]
mod platform {
    pub fn formats() -> Vec<String> {
        Vec::new()
    }

    pub fn rtf() -> Option<Vec<u8>> {
        None
    }

    pub fn encoded_picture() -> Option<Vec<u8>> {
        None
    }

    pub fn can_set() -> bool {
        true
    }

    pub fn add_mathml(_mathml: &str) {}
}
