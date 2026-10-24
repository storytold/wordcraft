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
//!
//! RTF goes beside them where we can add it: Windows ("Rich Text Format") and macOS
//! (`public.rtf`) after arboard's HTML, Wayland (`text/rtf`) in one offer with the text and HTML.
//! On X11 arboard owns the selection and serves only its own targets, so a copy there carries text
//! and HTML only; reading another program's RTF works on all four.

use wordcraft_engine::cmd::paste::MAX_PASTE_BYTES;
use wordcraft_ui_egui::paste_picture::ClipboardPicture;

fn open() -> Option<arboard::Clipboard> {
    arboard::Clipboard::new().map_err(|e| log::warn!("clipboard: {e}")).ok()
}

thread_local! {
    /// The clipboard copies are set through, kept open: on X11 the contents are served from it.
    static WRITER: std::cell::RefCell<Option<arboard::Clipboard>> = const { std::cell::RefCell::new(None) };
}

/// Put a copy on the clipboard: `text`, with `html` and `rtf` beside it where the platform takes
/// them (an empty one isn't offered), and `mathml` (a copy of one equation alone; empty for any
/// other copy) in the platform's MathML formats. `false` when the text and HTML couldn't be set
/// (the caller copies the text alone), including a Wayland session without the data-control
/// protocol, where arboard would fall back to X11 and native Wayland programs might not see it.
/// RTF or MathML that can't be added is logged: the HTML still carries the copy.
pub fn set_rich(text: &str, html: &str, rtf: &str, mathml: &str) -> bool {
    // Wayland: one offer with the text, HTML and RTF (no MathML formats there; see above).
    if platform::set_rich(text, html, rtf) {
        return true;
    }
    if !platform::can_set() {
        return false;
    }
    let set = set_html_text(html, text);
    if set && !rtf.is_empty() {
        platform::add_rtf(rtf);
    }
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
        let set = slot.as_mut().map(|c| if html.is_empty() { c.set_text(text) } else { c.set().html(html, Some(text)) });
        match set {
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

/// The clipboard's RTF: Windows ("Rich Text Format"), macOS (`public.rtf`), Wayland and X11
/// (`text/rtf`); not offered by arboard, so read here.
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
/// why (#833). The UI checks the sizes before inserting anything.
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

    /// Put the copy on a Wayland clipboard in one offer: the text (with the usual plain-text
    /// aliases), HTML and RTF, served from a background thread until something else is copied.
    /// `false` outside Wayland (X11 goes through arboard) or without the data-control protocol.
    pub fn set_rich(text: &str, html: &str, rtf: &str) -> bool {
        use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};

        if !wayland() {
            return false;
        }
        let offer = |mime: &str, data: &str| MimeSource { source: Source::Bytes(data.as_bytes().into()), mime_type: MimeType::Specific(mime.into()) };
        // The text first: the plain-text aliases serve the first text type offered.
        let mut sources = vec![offer("text/plain;charset=utf-8", text)];
        for (mime, data) in [("text/html", html), ("text/rtf", rtf)] {
            if !data.is_empty() {
                sources.push(offer(mime, data));
            }
        }
        match Options::new().copy_multi(sources) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("clipboard: {e}");
                false
            }
        }
    }

    /// Wayland copies carry their RTF already; X11 ones can't (see the module docs).
    pub fn add_rtf(_rtf: &str) {}

    /// RTF types a clipboard may offer, best first.
    const RTF_MIMES: [&str; 3] = ["text/rtf", "application/rtf", "text/richtext"];

    /// The RTF type to ask for among those `offered`.
    fn rtf_target(offered: &[String]) -> Option<&'static str> {
        RTF_MIMES.into_iter().find(|m| offered.iter().any(|o| o == m))
    }

    /// arboard serves its own MIME types only; Word, which reads the MathML formats, isn't here.
    pub fn add_mathml(_mathml: &str) {}

    pub fn rtf() -> Option<Vec<u8>> {
        if !wayland() {
            return std::env::var_os("DISPLAY").is_some().then(|| x11::rtf(super::MAX_PASTE_BYTES)).flatten();
        }
        let mime = rtf_target(&formats())?;
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

    /// Whether a property read of `len` bytes, with `bytes_after` left unread, is the whole value
    /// and at most `cap` bytes.
    fn complete(len: usize, bytes_after: u32, cap: usize) -> bool {
        bytes_after == 0 && len <= cap
    }

    /// Reading another program's RTF on X11, which arboard doesn't offer: ask the CLIPBOARD
    /// selection's owner for its targets, then for its RTF, into a property of a hidden window.
    mod x11 {
        use std::time::{Duration, Instant};

        use x11rb::connection::Connection;
        use x11rb::protocol::Event;
        use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, GetPropertyReply, Window, WindowClass};
        use x11rb::rust_connection::RustConnection;

        /// How long a paste waits for the selection's owner, in all.
        const TIMEOUT: Duration = Duration::from_secs(1);
        /// Most targets looked at.
        const MAX_TARGETS: usize = 128;

        struct Reader {
            conn: RustConnection,
            win: Window,
            clipboard: Atom,
            prop: Atom,
            deadline: Instant,
        }

        fn atom(conn: &RustConnection, name: &str) -> Option<Atom> {
            conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok().map(|r| r.atom)
        }

        impl Reader {
            fn new() -> Option<Self> {
                let (conn, screen) = x11rb::connect(None).ok()?;
                let root = conn.setup().roots.get(screen)?.root;
                let win = conn.generate_id().ok()?;
                conn.create_window(0, win, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, x11rb::COPY_FROM_PARENT, &CreateWindowAux::new()).ok()?;
                let clipboard = atom(&conn, "CLIPBOARD")?;
                let prop = atom(&conn, "WORDCRAFT_PASTE")?;
                Some(Self { conn, win, clipboard, prop, deadline: Instant::now() + TIMEOUT })
            }

            /// The selection converted to `target`, at most `cap` bytes; `None` when the owner
            /// refuses or doesn't answer in time. A longer value is `None` when `whole`, else
            /// its first `cap` bytes.
            fn convert(&self, target: Atom, cap: usize, whole: bool) -> Option<GetPropertyReply> {
                self.conn.convert_selection(self.win, self.clipboard, target, self.prop, x11rb::CURRENT_TIME).ok()?;
                self.conn.flush().ok()?;
                loop {
                    // Checked every time round, so a stream of other events can't keep us here.
                    if Instant::now() >= self.deadline {
                        return None;
                    }
                    match self.conn.poll_for_event().ok()? {
                        Some(Event::SelectionNotify(e)) if e.requestor == self.win => {
                            if e.property == x11rb::NONE {
                                return None;
                            }
                            break;
                        }
                        Some(_) => {}
                        None => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
                // In 32-bit units; for a whole value one more than the cap, so a larger one shows
                // as `bytes_after`.
                let units = u32::try_from(cap / 4 + usize::from(whole)).unwrap_or(u32::MAX);
                let r = self.conn.get_property(true, self.win, self.prop, AtomEnum::ANY, 0, units).ok()?.reply().ok()?;
                (!whole || super::complete(r.value.len(), r.bytes_after, cap)).then_some(r)
            }
        }

        impl Drop for Reader {
            fn drop(&mut self) {
                let _ = self.conn.destroy_window(self.win);
                let _ = self.conn.flush();
            }
        }

        /// The CLIPBOARD selection's RTF, at most `cap` bytes. Copies too large to send at once
        /// arrive in pieces (INCR), which aren't read: `None` then.
        pub fn rtf(cap: usize) -> Option<Vec<u8>> {
            let r = Reader::new()?;
            // The first targets only: a longer list is cut short, not refused.
            let targets = r.convert(atom(&r.conn, "TARGETS")?, MAX_TARGETS.saturating_mul(4), false)?;
            let cookies: Vec<_> = targets.value32()?.take(MAX_TARGETS).filter_map(|a| r.conn.get_atom_name(a).ok()).collect();
            let names: Vec<String> =
                cookies.into_iter().filter_map(|c| c.reply().ok()).map(|n| String::from_utf8_lossy(&n.name).into_owned()).collect();
            let target = atom(&r.conn, super::rtf_target(&names)?)?;
            let incr = atom(&r.conn, "INCR")?;
            let reply = r.convert(target, cap, true)?;
            (reply.type_ != incr).then_some(reply.value)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rtf_target_prefers_text_rtf() {
            let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            assert_eq!(rtf_target(&names(&["TARGETS", "text/richtext", "application/rtf", "text/rtf"])), Some("text/rtf"));
            assert_eq!(rtf_target(&names(&["text/richtext", "UTF8_STRING"])), Some("text/richtext"));
            assert_eq!(rtf_target(&names(&["text/html", "text/rtf;charset=x"])), None);
            assert_eq!(rtf_target(&[]), None);
        }

        #[test]
        fn property_reads_must_be_whole_and_capped() {
            assert!(complete(10, 0, 10));
            assert!(!complete(11, 0, 10), "over the cap");
            assert!(!complete(8, 4, 10), "more left to read");
            assert!(complete(0, 0, 0));
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use clipboard_win::{Clipboard, EnumFormats, raw};

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

    pub fn encoded_picture() -> Option<Vec<u8>> {
        None
    }

    pub fn can_set() -> bool {
        true
    }

    /// arboard sets the text and HTML.
    pub fn set_rich(_text: &str, _html: &str, _rtf: &str) -> bool {
        false
    }

    /// Add "Rich Text Format" to what arboard just set, NUL-terminated as is usual.
    pub fn add_rtf(rtf: &str) {
        let Some(format) = raw::register_format("Rich Text Format") else { return };
        let Ok(_open) = Clipboard::new_attempts(10) else { return };
        let mut bytes = Vec::with_capacity(rtf.len().saturating_add(1));
        bytes.extend_from_slice(rtf.as_bytes());
        bytes.push(0);
        if let Err(e) = raw::set_without_clear(format.get(), &bytes) {
            log::warn!("clipboard RTF: {e}");
        }
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
    use objc2_foundation::{NSData, NSString};

    /// RTF's pasteboard type (`NSPasteboardTypeRTF`, whose extern static needs unsafe to read).
    const RTF: &str = "public.rtf";

    pub fn formats() -> Vec<String> {
        Vec::new()
    }

    pub fn rtf() -> Option<Vec<u8>> {
        let data = NSPasteboard::generalPasteboard().dataForType(&NSString::from_str(RTF))?;
        (data.len() <= super::MAX_PASTE_BYTES).then(|| data.to_vec())
    }

    pub fn encoded_picture() -> Option<Vec<u8>> {
        None
    }

    pub fn can_set() -> bool {
        true
    }

    /// arboard sets the text and HTML.
    pub fn set_rich(_text: &str, _html: &str, _rtf: &str) -> bool {
        false
    }

    /// Add RTF to the pasteboard item arboard just wrote (it cleared the pasteboard first).
    pub fn add_rtf(rtf: &str) {
        if !NSPasteboard::generalPasteboard().setData_forType(Some(&NSData::with_bytes(rtf.as_bytes())), &NSString::from_str(RTF)) {
            log::warn!("clipboard: the RTF couldn't be added");
        }
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

    pub fn set_rich(_text: &str, _html: &str, _rtf: &str) -> bool {
        false
    }

    pub fn add_rtf(_rtf: &str) {}

    pub fn add_mathml(_mathml: &str) {}
}
