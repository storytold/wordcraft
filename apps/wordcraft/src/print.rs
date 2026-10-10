//! Desktop printing (#15): File › Print hands the document to the system's print flow.
//!
//! The app renders the document to PDF (`ui.print`, the Print button on File › Print) and calls
//! [`Services::print`] with the bytes. Here they go to a fresh temporary file
//! (`wordcraft-print-….pdf` in the OS temp folder), which is then opened without waiting for it:
//! in Preview on macOS (`open -a Preview`), and in the default PDF app elsewhere (`xdg-open` and
//! friends on Linux and BSD, the file association on Windows, via the `open` crate). The user
//! prints from there through the system print dialog.
//!
//! The viewer reads the file after we return, so it can't be removed right away: files older than
//! [`KEEP`] are cleaned up off the UI thread when the app starts and after each print.
//!
//! [`Services::print`]: wordcraft_ui_egui::Services::print

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Temporary print files start with this and end with `.pdf`.
const PREFIX: &str = "wordcraft-print-";
/// How long a print file is kept for its viewer before the clean-up removes it.
const KEEP: Duration = Duration::from_secs(60 * 60);

/// The `Services::print` hook.
pub type Hook = Box<dyn Fn(&[u8]) -> Result<(), String>>;

/// The desktop print hook; also clears print files left over from earlier runs.
pub fn hook() -> Hook {
    let dir = std::env::temp_dir();
    spawn_cleanup(dir.clone());
    Box::new(move |bytes| {
        let r = print_with(&dir, bytes, open_viewer);
        spawn_cleanup(dir.clone());
        r
    })
}

/// Writes `bytes` to a new print file in `dir` and has `open` show it. When `open` fails the file
/// is removed again.
fn print_with(dir: &Path, bytes: &[u8], open: impl Fn(&Path) -> Result<(), String>) -> Result<(), String> {
    let path = write_temp(dir, bytes).map_err(|e| format!("couldn't write the PDF to print: {e}"))?;
    open(&path).inspect_err(|_| {
        let _ = std::fs::remove_file(&path);
    })
}

/// A new file in `dir` with a unique print-file name, holding `bytes`. Never overwrites (or
/// follows a link at) an existing path; readable by the user only where the OS supports it.
fn write_temp(dir: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    for _ in 0..16 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("{PREFIX}{}-{stamp}-{n}.pdf", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                return match file.write_all(bytes) {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free temporary file name"))
}

/// Removes print files in `dir` last written `keep` or longer before `now`; returns how many.
/// Other files, links and files it may not remove are left alone.
fn remove_stale(dir: &Path, now: SystemTime, keep: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        if !is_print_file(&entry.file_name().to_string_lossy()) {
            continue;
        }
        // `DirEntry::metadata` doesn't follow links.
        let Ok(meta) = entry.metadata() else { continue };
        let old = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= keep);
        if meta.is_file() && old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn is_print_file(name: &str) -> bool {
    name.starts_with(PREFIX) && name.ends_with(".pdf")
}

/// Runs [`remove_stale`] on a background thread (the temp folder can be large).
fn spawn_cleanup(dir: PathBuf) {
    let spawned = std::thread::Builder::new().name("print-cleanup".into()).spawn(move || {
        let n = remove_stale(&dir, SystemTime::now(), KEEP);
        if n > 0 {
            log::info!("removed {n} old print file(s)");
        }
    });
    if let Err(e) = spawned {
        log::warn!("print clean-up: {e}");
    }
}

/// Opens the PDF in Preview without waiting for it; if Preview can't open it, the default app.
#[cfg(target_os = "macos")]
fn open_viewer(path: &Path) -> Result<(), String> {
    let mut child =
        std::process::Command::new("open").arg("-a").arg("Preview").arg(path).spawn().map_err(|e| format!("couldn't open Preview: {e}"))?;
    let path = path.to_path_buf();
    let waiter = std::thread::Builder::new().name("print-open".into()).spawn(move || {
        if !child.wait().is_ok_and(|s| s.success())
            && let Err(e) = open::that(&path)
        {
            log::error!("print: couldn't open {}: {e}", path.display());
        }
    });
    if let Err(e) = waiter {
        log::warn!("print: {e}");
    }
    Ok(())
}

/// Opens the PDF in the default app without waiting for it (`xdg-open`, `gio`… on Linux and BSD;
/// the file association on Windows).
#[cfg(not(target_os = "macos"))]
fn open_viewer(path: &Path) -> Result<(), String> {
    open::that_detached(path).map_err(|e| format!("couldn't open a PDF viewer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wordcraft_engine::Session;
    use wordcraft_ui_egui::{Services, WordApp};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-print-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// File › Print's Print button (`ui.print`) writes the document's PDF to a print file and
    /// hands that file to the viewer (faked here: no process is started); a viewer that can't
    /// start is reported in the status bar and leaves no file behind.
    #[test]
    fn file_print_opens_a_pdf_file() {
        let dir = temp_dir("open");
        let opened: Rc<RefCell<Vec<(PathBuf, Vec<u8>)>>> = Rc::default();
        let seen = opened.clone();
        let hook_dir = dir.clone();
        let services = Services {
            print: Some(Box::new(move |bytes| {
                print_with(&hook_dir, bytes, |path| {
                    seen.borrow_mut().push((path.to_path_buf(), std::fs::read(path).map_err(|e| e.to_string())?));
                    Ok(())
                })
            })),
            ..Default::default()
        };
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        app.run("text.insert", json!({"text": "Print me"})).unwrap();
        app.run("file.print", json!({})).unwrap();
        assert!(app.ui.backstage && app.ui.backstage_page == "print", "File › Print opens the Print page");
        app.run("ui.print", json!({})).unwrap();
        app.run("ui.print", json!({})).unwrap();

        let opened = opened.borrow();
        assert_eq!(opened.len(), 2, "each print opens the viewer once");
        for (path, bytes) in opened.iter() {
            assert!(bytes.starts_with(b"%PDF"), "the viewer gets a PDF");
            assert_eq!(path.parent(), Some(dir.as_path()));
            assert!(is_print_file(&path.file_name().unwrap().to_string_lossy()), "{}", path.display());
        }
        assert_ne!(opened[0].0, opened[1].0, "every print gets its own file");

        let mut app = WordApp::new(
            Session::new(wordcraft_doc::Document::new()),
            Services {
                print: Some(Box::new({
                    let dir = dir.clone();
                    move |bytes| print_with(&dir, bytes, |_| Err("no PDF viewer".into()))
                })),
                ..Default::default()
            },
        );
        let e = app.run("ui.print", json!({})).unwrap_err();
        assert!(e.contains("no PDF viewer"), "{e}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "the failed print's file is removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The clean-up removes old print files only.
    #[test]
    fn stale_print_files_are_removed() {
        let dir = temp_dir("stale");
        let fresh = write_temp(&dir, b"%PDF-1.7").unwrap();
        let old = write_temp(&dir, b"%PDF-1.7").unwrap();
        let hour_ago = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        OpenOptions::new().write(true).open(&old).unwrap().set_modified(hour_ago).unwrap();
        let other = dir.join("notes.pdf");
        std::fs::write(&other, b"keep").unwrap();
        OpenOptions::new().write(true).open(&other).unwrap().set_modified(hour_ago).unwrap();

        assert_eq!(remove_stale(&dir, SystemTime::now(), KEEP), 1);
        assert!(fresh.exists() && other.exists() && !old.exists());
        assert_eq!(remove_stale(&dir.join("missing"), SystemTime::now(), KEEP), 0, "a missing folder is fine");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
