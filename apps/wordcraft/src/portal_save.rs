//! File › Save As through the XDG desktop portal on Linux and BSDs (#6180).
//!
//! The GTK file chooser behind the portal keeps the typed name when the user picks another file
//! type, and `rfd` 0.17 reads only the file name from the portal's answer, so `Document1.docx`
//! with PDF picked in the dropdown came back as `Document1.docx` and was saved as a Word document.
//! The portal's answer does say which type was picked (`current_filter`), so Save As asks the portal
//! itself over D-Bus via `zbus` and gives a name the dialog left alone that type's extension
//! ([`with_chosen_type`]). `rfd` stays the fallback (zenity, or no portal), and this can go once an
//! `rfd` release reads the picked type (upstream PolyMeilex/rfd#341).
//!
//! The dialog is attached to the window on X11 (`x11:<xid>`). On Wayland it has no parent: naming the
//! window to the portal needs an xdg-foreign handle, which takes `unsafe` code the workspace forbids.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use zbus::blocking::{MessageIterator, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// One file type in the dialog: its label and extensions (`("PDF (*.pdf)", ["pdf"])`).
pub type Filter = (String, Vec<String>);

/// Quicker than anyone can see a dialog and dismiss it (#2999).
const TOO_QUICK: Duration = Duration::from_millis(300);

/// A thread-safe cancellation token to signal cancellation of an ongoing save request.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancelToken {
    /// Creates a new cancellation token in the active (not cancelled) state.
    pub fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    /// Signals that the active save request should be cancelled.
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns `true` if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl PartialEq for CancelToken {
    /// Compares two cancellation tokens by their current cancellation status.
    fn eq(&self, other: &Self) -> bool {
        self.is_cancelled() == other.is_cancelled()
    }
}

impl Eq for CancelToken {}

/// A Save dialog request to pass to the desktop portal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveRequest {
    /// The portal's parent window identifier (`x11:<xid>`), or empty.
    pub parent: String,
    /// The name the dialog shows: the suggested file name (`Document1.docx`).
    pub name: Option<String>,
    /// The suggested file's name (`Document1.docx`): a name that comes back with its extension still
    /// takes the extension of the type picked ([`with_chosen_type`]).
    pub suggested: Option<String>,
    /// The initial folder to open in, if any.
    pub folder: Option<PathBuf>,
    /// The list of filters offered in the dropdown.
    pub filters: Vec<Filter>,
    /// The index of the filter the dialog starts on.
    pub current: Option<usize>,
    /// Optional cancellation token allowing the caller to cancel a pending save request.
    pub cancel: Option<CancelToken>,
    /// Optional timeout after which a request waiting for a response is considered stalled.
    pub timeout: Option<Duration>,
}

/// What the portal dialog returned: the path, as typed, and the filter chosen in the dropdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The filesystem path chosen in the dialog.
    pub path: PathBuf,
    /// The filter chosen in the dropdown, if reported by the portal: `(label, extensions)`.
    pub filter: Option<Filter>,
}

/// The portal's parent window identifier for `window`: `x11:<xid>` on X11, empty elsewhere.
///
/// On Wayland, exporting an xdg-foreign handle requires `unsafe` code forbidden by workspace rules
/// (`#![forbid(unsafe_code)]`, Rust Rule 1), so Wayland dialogs remain unparented, matching
/// PhotoCraft and VectorCraft.
pub fn parent_id(window: Option<eframe::wgpu::rwh::RawWindowHandle>) -> String {
    use eframe::wgpu::rwh::RawWindowHandle;
    match window {
        Some(RawWindowHandle::Xlib(h)) => format!("x11:{:x}", h.window),
        Some(RawWindowHandle::Xcb(h)) => format!("x11:{:x}", h.window.get()),
        _ => String::new(),
    }
}

/// Converts a filter tuple into the portal's wire format: label and a list of `(0, "*.ext")` glob pairs.
fn wire(filter: &Filter) -> (String, Vec<(u32, String)>) {
    let globs = filter.1.iter().map(|e| (0u32, if e.is_empty() || e == "*" { "*".to_string() } else { format!("*.{e}") })).collect();
    (filter.0.clone(), globs)
}

/// The `SaveFile` options dictionary for `req`, answering on `token`.
fn options<'a>(req: &'a SaveRequest, token: &'a str) -> HashMap<&'static str, Value<'a>> {
    let mut o: HashMap<&'static str, Value<'a>> = HashMap::from([("handle_token", Value::from(token)), ("modal", Value::from(true))]);
    if let Some(name) = &req.name {
        o.insert("current_name", Value::from(name.as_str()));
    }
    if let Some(folder) = &req.folder {
        // A byte string with its terminating NUL, as the portal specifies.
        use std::os::unix::ffi::OsStrExt;
        let mut bytes = folder.as_os_str().as_bytes().to_vec();
        bytes.push(0);
        o.insert("current_folder", Value::from(bytes));
    }
    if !req.filters.is_empty() {
        let wired = req.filters.iter().map(wire).collect::<Vec<_>>();
        o.insert("filters", Value::from(wired));
    }
    if let Some(f) = req.current.and_then(|i| req.filters.get(i)).or_else(|| req.filters.first()) {
        o.insert("current_filter", Value::from(wire(f)));
    }
    o
}

/// Reads the portal's `Response`: code 0 with the first `uris` entry. 1 is Cancel, and 2 a dialog
/// closed some other way (e.g. Esc in GNOME): no file either. When code 2 arrives quicker than
/// [`TOO_QUICK`], the portal failed to show its dialog (#2999) and an error is returned so the
/// caller can fall back.
fn answer(code: u32, results: &HashMap<String, OwnedValue>, elapsed: Duration) -> Result<Option<Saved>, String> {
    match code {
        0 => {}
        1 => return Ok(None),
        2 if elapsed < TOO_QUICK => return Err("the save dialog closed at once".to_string()),
        2 => return Ok(None),
        _ => return Err(format!("the save dialog failed (portal response {code})")),
    }
    let uris: Vec<String> = results
        .get("uris")
        .ok_or_else(|| "no file in the portal's answer".to_string())?
        .try_clone()
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|e: zbus::zvariant::Error| e.to_string())?;
    let uri = uris.first().ok_or_else(|| "no file in the portal's answer".to_string())?;
    let path = file_uri_path(uri).ok_or_else(|| format!("not a local file: {uri}"))?;
    let filter = results.get("current_filter").and_then(|v| {
        let val = v.try_clone().ok()?;
        if let Ok((label, globs)) = <(String, Vec<(u32, String)>)>::try_from(val.clone()) {
            let mut exts = Vec::new();
            for (_, g) in globs {
                let s = g.trim_start_matches('*').trim_start_matches('.');
                if !s.is_empty() && s != "*" {
                    exts.push(s.to_ascii_lowercase());
                }
            }
            if exts.is_empty() {
                exts.extend(extract_exts_from_label(&label));
            }
            Some((label, exts))
        } else if let Ok((label, exts)) = <(String, Vec<String>)>::try_from(val.clone()) {
            Some((label, exts.into_iter().map(|s| s.to_ascii_lowercase()).collect()))
        } else if let Ok(label) = String::try_from(val) {
            let exts = extract_exts_from_label(&label);
            Some((label, exts))
        } else {
            None
        }
    });
    Ok(Some(Saved { path, filter }))
}

/// Extracts file extensions from a filter label such as `"PDF (*.pdf)"` or `"*.pdf"`.
fn extract_exts_from_label(label: &str) -> Vec<String> {
    let text = if let Some(inside) = label.split('(').nth(1).and_then(|s| s.split(')').next()) { inside } else { label };
    text.split(|c: char| c == ';' || c == ',' || c.is_whitespace())
        .filter_map(|part| {
            let s = part.trim().trim_start_matches('*').trim_start_matches('.');
            if s.is_empty() || s == "*" || s.contains('/') || s.contains('\\') { None } else { Some(s.to_ascii_lowercase()) }
        })
        .collect()
}

/// Converts a `file://` URI from the portal into a local `PathBuf`.
/// Returns `None` for non-local URIs or invalid percent-encoding.
pub fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Only local files: `file:///…` or `file://localhost/…`.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'%') {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())?;
            out.push(hex);
            i = i.saturating_add(3);
        } else if let Some(&b) = bytes.get(i) {
            out.push(b);
            i = i.saturating_add(1);
        } else {
            break;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Returns the index of the chosen filter in `filters` by matching the picked index, or the first filter
/// if only one was offered.
fn chosen_type(filters: &[Filter], picked: Option<usize>) -> Option<usize> {
    match picked {
        Some(i) if i < filters.len() => Some(i),
        None if filters.len() == 1 => Some(0),
        _ => None,
    }
}

/// Returns `path` with the extension of the file type picked in the save dialog (`chosen`),
/// where the dialog left the suggested name's extension alone (#6180).
///
/// The GTK file chooser keeps the typed name when another type is picked. Only a name that
/// still has the extension the dialog opened with (`opened_with`, e.g. `Document1.docx` with
/// PDF picked) or none gets the type's extension, as PolyMeilex/rfd#341 does. An extension the
/// user explicitly typed that WordCraft saves (e.g. `art.txt` with PDF untouched) is kept,
/// and so does one of the type's own extensions. Any extension that WordCraft does not save
/// (e.g. `scan.2024`) is treated as part of the stem, and the type's extension is added.
pub fn with_chosen_type(path: &Path, opened_with: Option<&str>, chosen: &[String]) -> PathBuf {
    let Some(first) = chosen.first() else { return path.to_path_buf() };
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let opened_ext = opened_with
        .and_then(|n| Path::new(n).extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| wordcraft_engine::io::SAVE_EXTS.contains(&e.as_str()));
    match ext {
        Some(ext) if chosen.contains(&ext) => path.to_path_buf(),
        Some(ext) if opened_ext.as_ref() == Some(&ext) => path.with_extension(first),
        Some(ext) if wordcraft_engine::io::SAVE_EXTS.contains(&ext.as_str()) => path.to_path_buf(),
        _ => {
            let mut name = path.as_os_str().to_owned();
            name.push(".");
            name.push(first);
            PathBuf::from(name)
        }
    }
}

/// Returns true if `path` exists as a file, directory, or symlink on disk.
fn entry_exists(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Closes the active portal request handle by calling `org.freedesktop.portal.Request.Close`.
pub fn close_request(conn: &zbus::blocking::Connection, portal: Option<&str>, handle: &OwnedObjectPath) {
    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const REQUEST: &str = "org.freedesktop.portal.Request";
    let dest = portal.unwrap_or(DESKTOP);
    if let Ok(proxy) = Proxy::new(conn, dest, handle.as_str(), REQUEST) {
        let _ = proxy.call_method("Close", &());
    }
}

/// RAII guard that closes an open portal request handle if dropped before being disarmed.
struct RequestGuard<'a> {
    conn: &'a zbus::blocking::Connection,
    portal: Option<String>,
    handle: Option<OwnedObjectPath>,
}

impl<'a> RequestGuard<'a> {
    /// Creates a new guard for the given request `handle`.
    fn new(conn: &'a zbus::blocking::Connection, portal: Option<String>, handle: OwnedObjectPath) -> Self {
        Self { conn, portal, handle: Some(handle) }
    }

    /// Disarms the guard when a response has been successfully received, preventing `Close` from being called on drop.
    fn disarm(&mut self) {
        self.handle = None;
    }

    /// Returns a reference to the request handle, if still active.
    fn handle(&self) -> Option<&OwnedObjectPath> {
        self.handle.as_ref()
    }
}

impl<'a> Drop for RequestGuard<'a> {
    /// Closes the active portal request handle when dropped unless disarmed.
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            close_request(self.conn, self.portal.as_deref(), &handle);
        }
    }
}

/// Asks the XDG desktop portal on `conn` for the `SaveFile` dialog of `req` and waits for its answer.
pub fn save_file(conn: &zbus::blocking::Connection, req: &SaveRequest) -> Result<Option<Saved>, String> {
    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const REQUEST: &str = "org.freedesktop.portal.Request";
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let token = format!("wordcraft_save_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));

    // Listen before asking, on every request path: a portal older than version 0.9 ignores
    // `handle_token` and answers on the request path it returns, and its answer can come before
    // the call's reply. The stream keeps every `Response` from the time it is registered.
    let dbus = zbus::blocking::fdo::DBusProxy::new(conn).map_err(|e| e.to_string())?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DESKTOP)
        .and_then(|b| b.interface(REQUEST))
        .and_then(|b| b.member("Response"))
        .map_err(|e| e.to_string())?
        .build();
    dbus.add_match_rule(rule).map_err(|e| e.to_string())?;

    let owner_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .and_then(|b| b.interface("org.freedesktop.DBus"))
        .and_then(|b| b.member("NameOwnerChanged"))
        .map_err(|e| e.to_string())?
        .build();
    dbus.add_match_rule(owner_rule).map_err(|e| e.to_string())?;

    let responses = MessageIterator::from(conn);
    let chooser = Proxy::new(conn, DESKTOP, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser").map_err(|e| e.to_string())?;

    let start = std::time::Instant::now();
    let call = chooser.call_method("SaveFile", &(req.parent.as_str(), "Save As", options(req, &token))).map_err(|e| e.to_string())?;
    let handle: OwnedObjectPath = call.body().deserialize().map_err(|e| e.to_string())?;
    let portal = call.header().sender().map(|s| s.as_str().to_string());

    let mut guard = RequestGuard::new(conn, portal.clone(), handle);
    let (tx, rx) = std::sync::mpsc::channel();
    let _listener = std::thread::Builder::new().name("portal save listener".into()).spawn(move || {
        for msg in responses.filter_map(Result::ok) {
            if tx.send(msg).is_err() {
                break;
            }
        }
    });

    let reply = loop {
        if req.cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
            return Err(ERR_CANCELLED.to_string());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => {
                let header = m.header();
                if header.path().map(|p| p.as_str()) == guard.handle().map(|h| h.as_str()) && header.sender().map(|s| s.as_str()) == portal.as_deref()
                {
                    break Ok(m);
                }
                #[allow(clippy::collapsible_if)]
                if header.interface().map(|i| i.as_str()) == Some("org.freedesktop.DBus")
                    && header.member().map(|m| m.as_str()) == Some("NameOwnerChanged")
                {
                    if let Ok((name, old_owner, "")) = m.body().deserialize::<(&str, &str, &str)>() {
                        if (name == DESKTOP || portal.as_deref() == Some(name)) && (portal.is_none() || portal.as_deref() == Some(old_owner)) {
                            break Err("the desktop portal terminated while the save dialog was open".to_string());
                        }
                        if name.starts_with("org.freedesktop.impl.portal.desktop") {
                            break Err("the desktop portal backend terminated while the save dialog was open".to_string());
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if req.timeout.is_some_and(|t| start.elapsed() >= t) {
                    break Err("the save dialog request timed out".to_string());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break Err("the save dialog closed without an answer".to_string());
            }
        }
    }?;

    guard.disarm();
    let elapsed = start.elapsed();
    let (code, results): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(|e| e.to_string())?;
    answer(code, &results, elapsed)
}

/// Returns true if `path` is located in the XDG document portal (`/run/user/<uid>/doc/…`, or
/// `/run/flatpak/doc/…` inside a Flatpak sandbox).
fn is_document_portal(path: &Path) -> bool {
    let s = path.to_string_lossy();
    ((s.starts_with("/run/user/") || s.starts_with("/var/run/user/")) && s.contains("/doc/")) || s.starts_with("/run/flatpak/doc/")
}

/// Error message returned when a pending save request was cancelled by the caller.
pub const ERR_CANCELLED: &str = "the save dialog request was cancelled";

/// Returns true if `err` represents a save request cancelled by the caller (not a cancel in the
/// dialog itself, which is `Ok(None)`).
pub fn is_cancelled_error(err: &str) -> bool {
    err == ERR_CANCELLED
}

/// Error message returned when the portal dialog does not report the selected filter and
/// the saved filename does not explicitly specify a distinct format.
pub const ERR_AMBIGUOUS_FILTER: &str = "the save dialog did not report which file type was picked";

/// Returns true if `err` represents an ambiguous filter error from the desktop portal.
pub fn is_ambiguous_filter_error(err: &str) -> bool {
    err == ERR_AMBIGUOUS_FILTER
}

/// Finds the index of the filter in `filters` matching `saved_filter`, first by exact label,
/// then by unambiguous match against the filter's extensions.
fn match_filter(filters: &[Filter], saved_filter: Option<&Filter>) -> Option<usize> {
    let (label, exts) = saved_filter?;
    if let Some(pos) = filters.iter().position(|(l, _)| l == label) {
        return Some(pos);
    }
    if !exts.is_empty() {
        let matching: Vec<usize> = filters
            .iter()
            .enumerate()
            .filter(|(_, (_, f_exts))| exts.iter().any(|e| f_exts.iter().any(|fe| fe.eq_ignore_ascii_case(e))))
            .map(|(i, _)| i)
            .collect();
        if matching.len() == 1 {
            return matching.first().copied();
        }
    }
    None
}

/// Returns true if the portal did not provide a unique matching filter when multiple filters were offered
/// and the user did not explicitly type a filename with a distinct supported format extension.
pub fn is_ambiguous_filter(saved: &Saved, req: &SaveRequest) -> bool {
    if req.filters.len() <= 1 {
        return false;
    }
    if match_filter(&req.filters, saved.filter.as_ref()).is_some() {
        return false;
    }
    let ext = saved.path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let suggested_ext = req
        .suggested
        .as_deref()
        .and_then(|n| Path::new(n).extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| wordcraft_engine::io::SAVE_EXTS.contains(&e.as_str()));
    match ext {
        Some(ext) if wordcraft_engine::io::SAVE_EXTS.contains(&ext.as_str()) => suggested_ext.as_ref() == Some(&ext),
        _ => true,
    }
}

/// Save As on Linux and BSDs: the portal's dialog, with the file name given the extension of the
/// file type picked in the dropdown. When that changes the name to one that already exists on disk,
/// the dialog opens again under the new name, so the portal asks before replacing it (it asked only
/// about the typed name).
pub fn save_as(conn: &zbus::blocking::Connection, mut req: SaveRequest) -> Result<Option<PathBuf>, String> {
    loop {
        let Some(saved) = save_file(conn, &req)? else { return Ok(None) };
        if is_ambiguous_filter(&saved, &req) {
            return Err(ERR_AMBIGUOUS_FILTER.to_string());
        }
        let picked = match_filter(&req.filters, saved.filter.as_ref());
        let exts: &[String] = match chosen_type(&req.filters, picked) {
            Some(i) => req.filters.get(i).map(|(_, e)| e.as_slice()).unwrap_or(&[]),
            None => &[],
        };
        let path = with_chosen_type(&saved.path, req.suggested.as_deref(), exts);
        let needs_reopen = entry_exists(&path) || (path != saved.path && is_document_portal(&saved.path));
        if path == saved.path || !needs_reopen {
            return Ok(Some(path));
        }
        req.name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        req.suggested = req.name.clone();
        req.folder = path.parent().map(Path::to_path_buf);
        req.current = picked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #6180: the file type picked in the save dialog gives the name its extension where the
    /// dialog left the one it opened with, and only there.
    #[test]
    fn the_picked_file_type_gives_the_name_its_extension() {
        let exts = |e: &[&str]| e.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (pdf, docx) = (exts(&["pdf"]), exts(&["docx", "docm"]));
        let opened = Some("Document1.docx");
        let cases = [
            // PDF picked, the name as the dialog opened (or renamed, its extension kept).
            ("/docs/Document1.docx", opened, &pdf, "/docs/Document1.pdf"),
            ("/docs/letter.docx", opened, &pdf, "/docs/letter.pdf"),
            ("/docs/LETTER.DOCX", opened, &pdf, "/docs/LETTER.pdf"),
            // No extension: takes the picked type's extension.
            ("/docs/report", opened, &pdf, "/docs/report.pdf"),
            // An extension WordCraft doesn't save is part of the name.
            ("/docs/scan.2024", opened, &pdf, "/docs/scan.2024.pdf"),
            // An opened extension WordCraft doesn't save (e.g. .5 in Report 1.5) is kept as part of the stem.
            ("/docs/Report 1.5", Some("Report 1.5"), &docx, "/docs/Report 1.5.docx"),
            // Typed, an extension WordCraft saves is the user's: kept, whatever the type says.
            ("/docs/notes.txt", opened, &pdf, "/docs/notes.txt"),
            ("/docs/page.html", opened, &pdf, "/docs/page.html"),
            ("/docs/file.rtf", opened, &pdf, "/docs/file.rtf"),
            // Already the type's: kept.
            ("/docs/letter.pdf", opened, &pdf, "/docs/letter.pdf"),
            ("/docs/letter.PDF", opened, &pdf, "/docs/letter.PDF"),
            // The dialog opened on a .dotx template: that is the extension the type replaces.
            ("/docs/memo.dotx", Some("memo.dotx"), &pdf, "/docs/memo.pdf"),
            ("/docs/doc.docx", Some("memo.dotx"), &pdf, "/docs/doc.docx"),
            // Word document picked for a file named without extension.
            ("/docs/contract", opened, &docx, "/docs/contract.docx"),
        ];
        for (typed, opened_with, chosen, want) in cases {
            assert_eq!(
                with_chosen_type(Path::new(typed), opened_with, chosen),
                PathBuf::from(want),
                "failed for typed={typed}, opened_with={opened_with:?}"
            );
        }
        // No type picked: the name as typed is kept.
        assert_eq!(with_chosen_type(Path::new("/docs/report.docx"), opened, &[]), PathBuf::from("/docs/report.docx"));
    }

    #[test]
    fn file_uris_become_local_paths() {
        assert_eq!(file_uri_path("file:///home/me/Document1.docx"), Some(PathBuf::from("/home/me/Document1.docx")));
        assert_eq!(file_uri_path("file:///home/me/A%20b%C3%A9.pdf"), Some(PathBuf::from("/home/me/A bé.pdf")));
        assert_eq!(file_uri_path("file://localhost/tmp/x.pdf"), Some(PathBuf::from("/tmp/x.pdf")));
        assert_eq!(file_uri_path("file:///tmp/a%ZZ.pdf"), None);
        assert_eq!(file_uri_path("file:///tmp/a%"), None);
        assert_eq!(file_uri_path("file:///tmp/a%2"), None);
        assert_eq!(file_uri_path("sftp://host/x.pdf"), None);
        assert_eq!(file_uri_path("file://host/x.pdf"), None);
        assert_eq!(file_uri_path("http://example.com/doc.pdf"), None);
    }

    #[test]
    fn x11_windows_are_named_to_the_portal() {
        use eframe::wgpu::rwh::{RawWindowHandle, XlibWindowHandle};
        assert_eq!(parent_id(Some(RawWindowHandle::Xlib(XlibWindowHandle::new(0x3a0000f)))), "x11:3a0000f");
        assert_eq!(parent_id(None), "");
    }

    #[test]
    fn answer_parses_portal_response_or_errors_on_quick_failure() {
        let mut results = HashMap::new();
        let uris: Vec<String> = vec!["file:///tmp/doc.docx".to_string()];
        results.insert("uris".to_string(), OwnedValue::try_from(Value::from(uris)).unwrap());
        let globs: Vec<(u32, String)> = vec![(0u32, "*.docx".to_string())];
        let filter: (String, Vec<(u32, String)>) = ("Word Document (*.docx)".to_string(), globs);
        results.insert("current_filter".to_string(), OwnedValue::try_from(Value::from(filter)).unwrap());

        // Code 0 with filter
        let ans = answer(0, &results, Duration::from_millis(1000)).unwrap();
        assert_eq!(
            ans,
            Some(Saved { path: PathBuf::from("/tmp/doc.docx"), filter: Some(("Word Document (*.docx)".to_string(), vec!["docx".to_string()])) })
        );

        // Code 0 without filter
        let mut results_no_filter = HashMap::new();
        results_no_filter.insert("uris".to_string(), results.get("uris").unwrap().clone());
        let ans_no_filter = answer(0, &results_no_filter, Duration::from_millis(1000)).unwrap();
        assert_eq!(ans_no_filter, Some(Saved { path: PathBuf::from("/tmp/doc.docx"), filter: None }));

        // Code 1 (Cancel)
        assert_eq!(answer(1, &results, Duration::from_millis(50)).unwrap(), None);

        // Code 2 quick (< TOO_QUICK)
        assert!(answer(2, &results, Duration::from_millis(100)).is_err());

        // Code 2 slow (>= TOO_QUICK)
        assert_eq!(answer(2, &results, Duration::from_millis(500)).unwrap(), None);

        // Code 3 (failure)
        assert!(answer(3, &results, Duration::from_millis(1000)).is_err());
    }

    struct KillOnDrop(std::process::Child);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Configuration for spawning a mock portal daemon.
    struct MockPortalConfig {
        code: u32,
        delay_reply: Option<Duration>,
        exit_after_reply: bool,
        send_response: bool,
        close_called: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        emit_backend_owner_loss: bool,
    }

    impl Default for MockPortalConfig {
        fn default() -> Self {
            Self { code: 0, delay_reply: None, exit_after_reply: false, send_response: true, close_called: None, emit_backend_owner_loss: false }
        }
    }

    /// Spawns a mock portal daemon that answers `SaveFile` and handles `Close` according to `config`.
    fn mock_portal_with_config(config: MockPortalConfig) -> Option<(zbus::blocking::Connection, KillOnDrop)> {
        use std::io::BufRead as _;

        let daemon = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .ok()?;
        let mut guard = KillOnDrop(daemon);
        let mut address = String::new();
        std::io::BufReader::new(guard.0.stdout.take()?).read_line(&mut address).ok()?;
        let address = address.trim().to_string();
        let server =
            zbus::blocking::connection::Builder::address(address.as_str()).ok()?.name("org.freedesktop.portal.Desktop").ok()?.build().ok()?;
        let incoming = zbus::blocking::MessageIterator::from(&server);
        let thread_address = address.clone();
        std::thread::spawn(move || {
            for msg in incoming {
                let Ok(msg) = msg else { break };
                let header = msg.header();
                if header.message_type() != zbus::message::Type::MethodCall {
                    continue;
                }
                if header.member().map(|m| m.as_str()) == Some("Close") {
                    if let Some(flag) = &config.close_called {
                        flag.store(true, Ordering::Relaxed);
                    }
                    let _ = server.reply(&header, &());
                    continue;
                }
                if header.member().map(|m| m.as_str()) != Some("SaveFile") {
                    continue;
                }
                let Ok((_parent, _title, options)) = msg.body().deserialize::<(String, String, HashMap<String, OwnedValue>)>() else {
                    break;
                };
                let text = |key: &str| options.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
                let Some(sender) = header.sender().map(|s| s.as_str().trim_start_matches(':').replace('.', "_")) else { break };
                let token = text("handle_token").unwrap_or_default();
                let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
                let Ok(handle) = OwnedObjectPath::try_from(path.as_str()) else { break };

                if config.exit_after_reply {
                    let _ = server.reply(&header, &handle);
                    drop(server);
                    break;
                }

                if config.emit_backend_owner_loss {
                    let _ = server.reply(&header, &handle);
                    let addr = thread_address.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(50));
                        if let Ok(backend) = zbus::blocking::connection::Builder::address(addr.as_str())
                            .and_then(|b| b.name("org.freedesktop.impl.portal.desktop.gtk"))
                            .and_then(|b| b.build())
                        {
                            std::thread::sleep(Duration::from_millis(50));
                            drop(backend);
                        }
                    });
                    continue;
                }

                if config.send_response {
                    let to = header.sender().cloned().map(zbus::names::BusName::from);
                    let results: HashMap<&str, Value<'_>> = HashMap::new();
                    let _ = server.emit_signal(to, path.as_str(), "org.freedesktop.portal.Request", "Response", &(config.code, &results));
                }

                if let Some(delay) = config.delay_reply {
                    std::thread::sleep(delay);
                }
                if server.reply(&header, &handle).is_err() {
                    break;
                }
            }
        });
        let client = zbus::blocking::connection::Builder::address(address.as_str()).ok()?.build().ok()?;
        Some((client, guard))
    }

    /// Spawns a mock portal daemon that answers `SaveFile`.
    /// If `delay_reply` is set, it emits the Response signal immediately and delays the method
    /// reply by that duration. If `exit_after_reply` is set, it replies to `SaveFile` and exits
    /// without sending a `Response`.
    fn mock_portal(code: u32, delay_reply: Option<Duration>, exit_after_reply: bool) -> Option<(zbus::blocking::Connection, KillOnDrop)> {
        mock_portal_with_config(MockPortalConfig {
            code,
            delay_reply,
            exit_after_reply,
            send_response: true,
            close_called: None,
            emit_backend_owner_loss: false,
        })
    }

    /// A delayed SaveFile method reply with an already-queued response code 2 measures elapsed time
    /// from before calling SaveFile, so it is treated as a user cancellation rather than a quick
    /// portal failure that re-opens another dialog.
    #[test]
    fn delayed_method_reply_with_queued_cancel_is_not_treated_as_quick_failure() {
        let Some((client, _daemon)) = mock_portal(2, Some(Duration::from_millis(350)), false) else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let req = SaveRequest {
            name: Some("test.docx".to_string()),
            suggested: Some("test.docx".to_string()),
            filters: vec![("Word Document (*.docx)".to_string(), vec!["docx".to_string()])],
            ..Default::default()
        };
        let res = save_file(&client, &req);
        assert_eq!(res, Ok(None));
    }

    /// If the desktop portal terminates while a save dialog is open (e.g. crashing after replying
    /// to SaveFile), save_file detects NameOwnerChanged and returns an error immediately.
    #[test]
    fn portal_terminating_while_open_returns_error() {
        let Some((client, _daemon)) = mock_portal(0, None, true) else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let req = SaveRequest {
            name: Some("test.docx".to_string()),
            suggested: Some("test.docx".to_string()),
            filters: vec![("Word Document (*.docx)".to_string(), vec!["docx".to_string()])],
            ..Default::default()
        };
        let res = save_file(&client, &req);
        assert!(res.is_err(), "portal termination returns an error so rfd fallback runs: got {res:?}");
    }

    /// If the desktop portal backend implementation (e.g. GTK, GNOME, KDE) terminates while the
    /// dialog is open, save_file detects NameOwnerChanged on the implementation service and returns
    /// an error immediately so the rfd fallback can run.
    #[test]
    fn portal_backend_terminating_while_open_returns_error() {
        let Some((client, _daemon)) =
            mock_portal_with_config(MockPortalConfig { emit_backend_owner_loss: true, send_response: false, ..Default::default() })
        else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let req = SaveRequest {
            name: Some("test.docx".to_string()),
            suggested: Some("test.docx".to_string()),
            filters: vec![("Word Document (*.docx)".to_string(), vec!["docx".to_string()])],
            ..Default::default()
        };
        let res = save_file(&client, &req);
        assert!(res.is_err(), "backend termination returns an error so rfd fallback runs: got {res:?}");
        assert!(res.as_ref().unwrap_err().contains("backend terminated"), "expected backend termination error: got {res:?}");
    }

    /// When a save request is cancelled via `CancelToken`, save_file aborts waiting for the response,
    /// calls Close on the portal request object path, and returns a cancellation error immediately.
    #[test]
    fn save_file_cancellation_closes_request_and_returns_error() {
        let close_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let Some((client, _daemon)) =
            mock_portal_with_config(MockPortalConfig { send_response: false, close_called: Some(close_called.clone()), ..Default::default() })
        else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let cancel = CancelToken::new();
        let cancel_thread = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            cancel_thread.cancel();
        });
        let req = SaveRequest {
            name: Some("test.docx".to_string()),
            suggested: Some("test.docx".to_string()),
            filters: vec![("Word Document (*.docx)".to_string(), vec!["docx".to_string()])],
            cancel: Some(cancel),
            ..Default::default()
        };
        let res = save_file(&client, &req);
        assert!(res.is_err(), "cancelled request returns error: got {res:?}");
        assert!(res.as_ref().unwrap_err().contains("cancelled"), "expected cancellation error: got {res:?}");
        std::thread::sleep(Duration::from_millis(50));
        assert!(close_called.load(Ordering::Relaxed), "Close must be called on portal request handle");
    }

    /// When a save request times out waiting for a stalled portal backend response, save_file aborts,
    /// calls Close on the portal request object path, and returns a timeout error.
    #[test]
    fn save_file_timeout_closes_request_and_returns_error() {
        let close_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let Some((client, _daemon)) =
            mock_portal_with_config(MockPortalConfig { send_response: false, close_called: Some(close_called.clone()), ..Default::default() })
        else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let req = SaveRequest {
            name: Some("test.docx".to_string()),
            suggested: Some("test.docx".to_string()),
            filters: vec![("Word Document (*.docx)".to_string(), vec!["docx".to_string()])],
            timeout: Some(Duration::from_millis(50)),
            ..Default::default()
        };
        let res = save_file(&client, &req);
        assert!(res.is_err(), "timed out request returns error: got {res:?}");
        assert!(res.as_ref().unwrap_err().contains("timed out"), "expected timeout error: got {res:?}");
        std::thread::sleep(Duration::from_millis(50));
        assert!(close_called.load(Ordering::Relaxed), "Close must be called on portal request handle");
    }

    /// `entry_exists` detects existing files and dangling symlinks, distinguishing them from
    /// absent entries.
    #[test]
    fn entry_exists_detects_dangling_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nonexistent_target.txt");
        let link = dir.path().join("dangling_link.txt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(!link.exists(), "target does not exist so exists() is false");
            assert!(entry_exists(&link), "symlink entry exists so entry_exists() is true");
        }
        assert!(!entry_exists(&target), "nonexistent file does not exist");
    }

    /// `is_document_portal` identifies paths within the XDG document portal mount, including the
    /// `/run/flatpak/doc` mount a Flatpak sandbox sees.
    #[test]
    fn is_document_portal_detects_portal_fuse_paths() {
        assert!(is_document_portal(Path::new("/run/user/1000/doc/1234abcd/test.docx")));
        assert!(is_document_portal(Path::new("/var/run/user/1000/doc/by-app/app/test.docx")));
        assert!(is_document_portal(Path::new("/run/flatpak/doc/1234abcd/test.docx")));
        assert!(!is_document_portal(Path::new("/home/user/Documents/test.docx")));
        assert!(!is_document_portal(Path::new("/tmp/doc/test.docx")));
        assert!(!is_document_portal(Path::new("/run/flatpak/other/test.docx")));
    }

    /// When the portal omits `current_filter`, ambiguous saves (unchanged suggested extension or bare name)
    /// are rejected, while an explicitly typed different format is accepted.
    #[test]
    fn ambiguous_filters_require_explicit_format() {
        let exts = |e: &[&str]| e.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let filters = vec![("Word Document (*.docx)".into(), exts(&["docx"])), ("PDF (*.pdf)".into(), exts(&["pdf"]))];
        let req = SaveRequest { filters: filters.clone(), suggested: Some("Document1.docx".into()), ..Default::default() };

        // Reported filter with matching label is not ambiguous.
        let with_filter = Saved { path: PathBuf::from("/docs/Document1.docx"), filter: Some(("Word Document (*.docx)".into(), exts(&["docx"]))) };
        assert_eq!(match_filter(&filters, with_filter.filter.as_ref()), Some(0));
        assert!(!is_ambiguous_filter(&with_filter, &req));

        // Reported filter with unmatched label but unambiguous extension (e.g. KDE *.pdf) is not ambiguous.
        let kde_pdf_filter = Saved { path: PathBuf::from("/docs/Document1.docx"), filter: Some(("*.pdf".into(), exts(&["pdf"]))) };
        assert_eq!(match_filter(&filters, kde_pdf_filter.filter.as_ref()), Some(1));
        assert!(!is_ambiguous_filter(&kde_pdf_filter, &req));

        // Reported filter with unrecognized extensions and unchanged suggested extension is ambiguous.
        let unknown_filter = Saved { path: PathBuf::from("/docs/Document1.docx"), filter: Some(("Custom (*.xyz)".into(), exts(&["xyz"]))) };
        assert_eq!(match_filter(&filters, unknown_filter.filter.as_ref()), None);
        assert!(is_ambiguous_filter(&unknown_filter, &req));

        // Multiple filters matching same extension is ambiguous when label doesn't match.
        let duplicate_filters = vec![("Plain Text (*.txt)".into(), exts(&["txt"])), ("Text Document (*.txt)".into(), exts(&["txt"]))];
        let dup_filter = ("*.txt".to_string(), exts(&["txt"]));
        assert_eq!(match_filter(&duplicate_filters, Some(&dup_filter)), None);

        // Missing filter with unchanged suggested extension is ambiguous.
        let unchanged = Saved { path: PathBuf::from("/docs/Document1.docx"), filter: None };
        assert!(is_ambiguous_filter(&unchanged, &req));

        // Missing filter with bare filename is ambiguous.
        let bare = Saved { path: PathBuf::from("/docs/Document1"), filter: None };
        assert!(is_ambiguous_filter(&bare, &req));

        // Missing filter but user explicitly typed a different format is NOT ambiguous.
        let explicit = Saved { path: PathBuf::from("/docs/Document1.pdf"), filter: None };
        assert!(!is_ambiguous_filter(&explicit, &req));

        // Single filter offered is never ambiguous even if filter is omitted.
        let single_req =
            SaveRequest { filters: vec![("PDF (*.pdf)".into(), exts(&["pdf"]))], suggested: Some("Document1.docx".into()), ..Default::default() };
        assert!(!is_ambiguous_filter(&unchanged, &single_req));
    }
}
