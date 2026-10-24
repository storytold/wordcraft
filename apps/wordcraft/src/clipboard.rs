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

/// Open the system clipboard. A session where that fails (Wayland without data-control and no
/// X11) fails on every read, so only the first failure is a warning (#1368).
fn open() -> Option<arboard::Clipboard> {
    arboard::Clipboard::new().map_err(|e| log_open_failure(&e)).ok()
}

fn log_open_failure(e: &arboard::Error) {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log::debug!("clipboard: {e}");
    } else {
        log::warn!("clipboard: {e}");
    }
}

/// What a picture paste yields when the clipboard couldn't be opened: whatever `fallback` reads,
/// else the explanation.
fn unopened_picture(detail: &str, fallback: impl FnOnce() -> Option<Vec<u8>>) -> ClipboardPicture {
    match fallback() {
        Some(bytes) => ClipboardPicture::Encoded(bytes),
        None => ClipboardPicture::Unreadable(open_failed_message(detail)),
    }
}

/// What the paste says when no way of reading the clipboard worked: what's wrong, and what to do.
pub fn open_failed_message(detail: &str) -> String {
    format!("WordCraft couldn't open the system clipboard ({detail}). Use Insert > Picture, or drag the picture file into the window")
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
/// why (#833). The UI checks the sizes before inserting anything.
pub fn picture() -> Option<ClipboardPicture> {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            log_open_failure(&e);
            // arboard has no usable backend here (#1368): ask Wayland and wl-paste / xclip directly.
            return Some(unopened_picture(&e.to_string(), platform::fallback_picture));
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
    /// The read runs on its own thread so a producer that offers the type but never closes its
    /// pipe can't hold the caller: after five seconds it gives up (the thread ends with the pipe).
    fn read(mime: &str, cap: u64) -> Option<Vec<u8>> {
        let mime = mime.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("clipboard-wayland-read".into())
            .spawn(move || {
                let got = (|| {
                    let (pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Specific(&mime)).ok()?;
                    let mut out = Vec::new();
                    pipe.take(cap.saturating_add(1)).read_to_end(&mut out).ok()?;
                    (out.len() as u64 <= cap).then_some(out)
                })();
                let _ = tx.send(got);
            })
            .ok()?;
        rx.recv_timeout(std::time::Duration::from_secs(5)).ok().flatten()
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

    /// The programs that can read the clipboard when arboard and Wayland's data-control can't.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ExternalCommand {
        WlPaste,
        Xclip,
    }

    impl ExternalCommand {
        fn invocation(self) -> (&'static str, &'static [&'static str]) {
            match self {
                Self::WlPaste => ("wl-paste", &["--no-newline", "--type", "image/png"]),
                Self::Xclip => ("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"]),
            }
        }
    }

    /// Run `program` (no shell), returning its stdout when it exits successfully with at most
    /// `cap` bytes inside `timeout`. A missing program, failure, oversize or hang is `None`.
    pub fn run_capture(program: &str, args: &[&str], cap: u64, timeout: std::time::Duration) -> Option<Vec<u8>> {
        use std::process::{Command, Stdio};
        let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::Builder::new().name("clipboard-read".into()).spawn(move || {
            let mut out = Vec::new();
            let r = (&mut stdout).take(cap.saturating_add(1)).read_to_end(&mut out).map(|_| out);
            let _ = tx.send(r);
        });
        let out = if reader.is_ok() { rx.recv_timeout(timeout).ok().and_then(Result::ok) } else { None };
        // Always stop the program before reaping it: one that closed stdout but keeps running
        // must not hang the caller. The output is complete once stdout reached its end.
        let _ = child.kill();
        let _ = child.wait();
        out.filter(|o| o.len() as u64 <= cap)
    }

    /// The first command whose output decodes as a PNG (the type asked for; a program that prints
    /// text or an error instead, or a file cut short, is ignored and the next command is tried).
    pub fn picture_via(commands: &[ExternalCommand], run: &mut dyn FnMut(ExternalCommand) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
        commands.iter().find_map(|&c| run(c).filter(|b| b.starts_with(b"\x89PNG") && wordcraft_render::decode_pixmap(b).is_some()))
    }

    /// A picture read by `wl-paste` / `xclip`, for sessions where arboard can't open the clipboard.
    pub fn external_picture() -> Option<Vec<u8>> {
        let mut commands = Vec::new();
        if wayland() {
            commands.push(ExternalCommand::WlPaste);
        }
        if std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty()) {
            commands.push(ExternalCommand::Xclip);
        }
        // One deadline for all helpers, so two that hang cost five seconds, not ten.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        picture_via(&commands, &mut |c| {
            let (program, args) = c.invocation();
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            run_capture(program, args, wordcraft_ui_egui::paste_picture::MAX_FILE_BYTES, left)
        })
    }

    /// Every way left to read a picture once arboard can't open the clipboard.
    pub fn fallback_picture() -> Option<Vec<u8>> {
        encoded_picture().or_else(external_picture)
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

    pub fn fallback_picture() -> Option<Vec<u8>> {
        None
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

    pub fn fallback_picture() -> Option<Vec<u8>> {
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

    pub fn fallback_picture() -> Option<Vec<u8>> {
        None
    }

    pub fn can_set() -> bool {
        true
    }

    pub fn add_mathml(_mathml: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_message_says_what_to_do() {
        let m = open_failed_message("X11 server connection timed out");
        assert!(m.contains("couldn't open the system clipboard"), "{m}");
        assert!(m.contains("X11 server connection timed out"), "{m}");
        assert!(m.contains("Insert > Picture") && m.contains("drag"), "{m}");
    }

    #[test]
    fn unopened_clipboard_runs_the_fallback_then_explains() {
        assert!(matches!(unopened_picture("x", || Some(vec![1, 2])), ClipboardPicture::Encoded(b) if b == [1, 2]));
        assert!(matches!(unopened_picture("boom", || None), ClipboardPicture::Unreadable(m) if m.contains("boom") && m.contains("Insert > Picture")));
    }

    #[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))))]
    mod external {
        use super::super::platform::{ExternalCommand, picture_via};

        fn png() -> Vec<u8> {
            // A real 1x1 PNG: the output must decode, not just start with the signature.
            b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\x0d\x49\x44\x41\x54\x78\xda\x63\x64\x60\xf8\x5f\x0f\x00\x02\x87\x01\x80\xeb\x47\xba\x92\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82".to_vec()
        }

        #[test]
        fn first_command_with_output_wins() {
            let mut asked = Vec::new();
            let got = picture_via(&[ExternalCommand::WlPaste, ExternalCommand::Xclip], &mut |c| {
                asked.push(c);
                (c == ExternalCommand::Xclip).then(png)
            });
            assert_eq!(got, Some(png()));
            assert_eq!(asked, [ExternalCommand::WlPaste, ExternalCommand::Xclip]);
        }

        #[test]
        fn empty_or_missing_output_is_none() {
            assert_eq!(picture_via(&[ExternalCommand::WlPaste], &mut |_| Some(Vec::new())), None);
            assert_eq!(picture_via(&[ExternalCommand::WlPaste, ExternalCommand::Xclip], &mut |_| None), None);
            assert_eq!(picture_via(&[], &mut |_| Some(png())), None);
        }

        #[test]
        fn truncated_png_is_rejected_and_the_next_command_is_tried() {
            let mut cut = png();
            cut.truncate(30);
            let got = picture_via(&[ExternalCommand::WlPaste, ExternalCommand::Xclip], &mut |c| {
                Some(if c == ExternalCommand::WlPaste { cut.clone() } else { png() })
            });
            assert_eq!(got, Some(png()));
            assert_eq!(picture_via(&[ExternalCommand::WlPaste], &mut |_| Some(b"\x89PNG\r\n\x1a\nrest".to_vec())), None);
        }

        #[test]
        fn text_instead_of_a_picture_is_rejected() {
            // wl-paste prints its errors to stderr, but a program may hand back text on stdout.
            assert_eq!(picture_via(&[ExternalCommand::Xclip], &mut |_| Some(b"hello".to_vec())), None);
        }

        #[test]
        fn missing_program_is_none_not_a_panic() {
            let out = super::super::platform::run_capture("wordcraft-no-such-program", &[], 16, std::time::Duration::from_secs(1));
            assert_eq!(out, None);
        }

        #[test]
        fn output_is_capped_and_timeout_kills() {
            use std::time::Duration;
            let big = super::super::platform::run_capture("sh", &["-c", "head -c 1000 /dev/zero"], 100, Duration::from_secs(5));
            assert_eq!(big, None, "larger than the cap");
            let ok = super::super::platform::run_capture("sh", &["-c", "printf abc"], 100, Duration::from_secs(5));
            assert_eq!(ok, Some(b"abc".to_vec()));
            let t = std::time::Instant::now();
            let slow = super::super::platform::run_capture("sh", &["-c", "sleep 5"], 100, Duration::from_millis(200));
            assert_eq!(slow, None);
            assert!(t.elapsed() < Duration::from_secs(3));
        }

        #[test]
        fn program_that_closes_stdout_but_keeps_running_does_not_hang() {
            use std::time::{Duration, Instant};
            let t = Instant::now();
            let out = super::super::platform::run_capture("sh", &["-c", "printf abc; exec >&-; exec sleep 5"], 100, Duration::from_secs(10));
            assert_eq!(out, Some(b"abc".to_vec()));
            assert!(t.elapsed() < Duration::from_secs(3), "waited for the program to exit");
        }
    }
}
