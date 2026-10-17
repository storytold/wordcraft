//! Sibling temporary files owned by one save. The destination changes only after preparation
//! succeeds; concurrent saves have separate files and the last successful replacement wins.
//!
//! When the folder refuses the temporary file (macOS privacy protection, #2138, or a read-only
//! folder), an existing destination is never written in place: it is left exactly as it was and
//! the error says to use Save As to another folder or to give WordCraft access to this one. Only
//! a destination that doesn't exist yet, so has no previous version to lose, is created directly;
//! a failed direct write removes it again.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let permissions = match fs::metadata(path) {
        Ok(meta) if meta.is_file() => Some(meta.permissions()),
        Ok(_) => None,
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    // A new document gets the usual mode (0666 less the umask); a replaced one keeps its own,
    // and its temporary file stays private until that mode is applied.
    let private = permissions.is_some();
    let written = replace_with(path, private, |file| {
        file.write_all(bytes)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        // Synchronize the prepared file before replacing the destination. Directory durability
        // and network-filesystem/power-loss guarantees are outside this operation's contract.
        file.sync_all()
    });
    written.map_err(explain)
}

fn replace_with(path: &Path, private: bool, prepare: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let mut temp = match Temporary::create(path, private) {
        Ok(temp) => temp,
        // macOS only lets an app write the file picked in the Save panel inside a protected
        // folder (Desktop, Documents, Downloads, iCloud Drive), not a sibling temporary file
        // (#2138); a read-only folder refuses it too.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return create_directly(path, e, prepare),
        Err(e) => return Err(e),
    };
    prepare(temp.file()?)?;
    temp.commit(path)
}

/// No temporary file may be created beside `path` (`refused` says why). A file that doesn't exist
/// yet is created directly (there is no previous version to lose) and removed again if the save
/// fails. Anything already at `path` is left untouched: overwriting it in place could destroy the
/// only copy, so the error says how to save safely instead.
fn create_directly(path: &Path, refused: io::Error, prepare: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    // `create_new` never opens an existing file or follows a link, so it can't touch one.
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(guided(
                refused,
                "This folder doesn't let WordCraft create the temporary file a safe save needs, so the existing file was left unchanged.",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(guided(e, "WordCraft can't create files in this folder.")),
        Err(e) => return Err(e),
    };
    let Err(failed) = prepare(&mut file) else {
        return Ok(());
    };
    drop(file);
    match fs::remove_file(path) {
        Ok(()) => Err(failed),
        Err(e) => Err(io::Error::new(failed.kind(), format!("{failed}; removing the incomplete new file also failed ({e})"))),
    }
}

/// A save refused for want of folder access, already worded with what to do about it.
#[derive(Debug)]
struct Guided(String);

impl std::fmt::Display for Guided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Guided {}

/// `e`, then `why`, then how to save anyway: Save As to another folder, or access to this one.
fn guided(e: io::Error, why: &str) -> io::Error {
    let access = if cfg!(target_os = "macos") {
        "allow WordCraft to use this folder in System Settings › Privacy & Security › Files and Folders"
    } else {
        "get write access to this folder"
    };
    io::Error::new(e.kind(), Guided(format!("{e}. {why} Use Save As to save the document in another folder, or {access}, then save again.")))
}

/// A permission error on macOS usually means its privacy protection keeps WordCraft out of the
/// folder; say where to allow it (errors from [`guided`] already do).
fn explain(e: io::Error) -> io::Error {
    let worded = e.get_ref().is_some_and(|inner| inner.is::<Guided>());
    if cfg!(target_os = "macos") && e.kind() == io::ErrorKind::PermissionDenied && !worded {
        return io::Error::new(
            e.kind(),
            format!(
                "{e}. macOS may be blocking WordCraft from this folder: allow it in System Settings › Privacy & Security › Files and Folders, or save somewhere else"
            ),
        );
    }
    e
}

struct Temporary {
    path: PathBuf,
    file: Option<File>,
    committed: bool,
}

impl Temporary {
    /// `private`: create the file readable by its owner only (0600) rather than with the default
    /// mode (0666 less the umask).
    fn create(destination: &Path, private: bool) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if destination.file_name().is_none() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "save destination needs a file name"));
        }
        // A short name also works beside a destination already at the filename-length limit.
        for _ in 0..128 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = destination.with_file_name(format!(".wordcraft-save-{}-{id}.tmp", std::process::id()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            if private {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            #[cfg(not(unix))]
            let _ = private;
            match options.open(&path) {
                Ok(file) => return Ok(Self { path, file: Some(file), committed: false }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not create a save temporary file"))
    }

    fn file(&mut self) -> io::Result<&mut File> {
        self.file.as_mut().ok_or_else(|| io::Error::other("save temporary file is closed"))
    }

    fn commit(mut self, destination: &Path) -> io::Result<()> {
        // Close before replacement, including on Windows.
        drop(self.file.take());
        fs::rename(&self.path, destination)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.committed {
            // If filesystem permissions change, cleanup can fail too. Never remove a neighbour
            // or the destination to compensate for a failed save.
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path =
                std::env::temp_dir().join(format!("wordcraft-save-owner-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn target(&self) -> PathBuf {
            self.0.join("original.docx")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn partial_and_complete_preparation_failures_leave_original_and_no_owned_temp() {
        for partial in [true, false] {
            let d = Scratch::new();
            let path = d.target();
            fs::write(&path, b"Original bytes.").unwrap();
            let err = replace_with(&path, true, |file| {
                file.write_all(if partial { b"Partial" } else { b"Complete replacement bytes." })?;
                Err(io::Error::other("injected preparation failure"))
            })
            .unwrap_err();
            assert!(err.to_string().contains("injected"));
            assert_eq!(fs::read(&path).unwrap(), b"Original bytes.");
            assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
        }
    }

    #[test]
    fn overlapping_prepared_saves_keep_separate_bytes_and_commit_complete_files() {
        let d = Scratch::new();
        let path = d.target();
        fs::write(&path, b"Original bytes.").unwrap();
        let mut first = Temporary::create(&path, true).unwrap();
        let mut second = Temporary::create(&path, true).unwrap();
        assert_ne!(first.path, second.path);
        first.file().unwrap().write_all(b"First complete save.").unwrap();
        second.file().unwrap().write_all(b"Second complete save.").unwrap();
        first.file().unwrap().sync_all().unwrap();
        second.file().unwrap().sync_all().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"Original bytes.");
        first.commit(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"First complete save.");
        second.commit(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"Second complete save.");
        assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
    }

    /// Exports (`write_file`) replace the destination instead of truncating it in place: another
    /// hard link to the old file keeps the old bytes, and no temporary file is left behind.
    #[test]
    fn write_file_replaces_the_destination_instead_of_truncating_it() {
        let d = Scratch::new();
        let path = d.target();
        let alias = d.0.join("alias.docx");
        fs::write(&path, b"Original bytes.").unwrap();
        fs::hard_link(&path, &alias).unwrap();
        crate::io::write_file(&path.to_string_lossy(), b"Exported bytes.").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"Exported bytes.");
        assert_eq!(fs::read(&alias).unwrap(), b"Original bytes.");
        assert_eq!(fs::read_dir(&d.0).unwrap().count(), 2);
    }

    /// Makes `dir` read-only while `f` runs. `None` when running as root, where the folder isn't
    /// read-only to us.
    #[cfg(unix)]
    fn in_read_only<T>(dir: &Path, f: impl FnOnce() -> T) -> Option<T> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        let probe = fs::write(dir.join("probe"), b"");
        let out = f();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
        probe.is_err().then_some(out)
    }

    /// #2138 review: where no sibling temporary file may be created (macOS privacy protection,
    /// or a read-only folder), an existing document is left byte for byte as it was, and the
    /// error says to use Save As or get access to the folder.
    #[cfg(unix)]
    #[test]
    fn an_existing_file_in_a_folder_refusing_the_temporary_file_is_left_untouched() {
        let d = Scratch::new();
        let path = d.target();
        fs::write(&path, b"Original bytes.").unwrap();
        let Some(saved) = in_read_only(&d.0, || write(&path, b"Saved bytes.")) else { return };
        let err = saved.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        let message = err.to_string();
        assert!(message.contains("left unchanged") && message.contains("Save As") && message.contains("this folder"), "{message}");
        assert_eq!(message.matches("Privacy & Security").count(), usize::from(cfg!(target_os = "macos")), "{message}");
        assert_eq!(fs::read(&path).unwrap(), b"Original bytes.");
        assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
    }

    /// FredApps' #2138 regression: a save that would fail partway through still leaves the
    /// previous document byte-identical when the folder refuses the temporary file.
    #[cfg(unix)]
    #[test]
    fn a_failed_save_in_a_folder_refusing_the_temporary_file_keeps_the_original() {
        let d = Scratch::new();
        let path = d.target();
        fs::write(&path, b"Original bytes.").unwrap();
        let Some(result) = in_read_only(&d.0, || {
            replace_with(&path, true, |file| {
                file.write_all(b"Partial")?;
                Err(io::Error::other("injected partial write failure"))
            })
        }) else {
            return;
        };
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"Original bytes.", "failed save must preserve the previous document");
        assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
    }

    /// A folder that refuses new files altogether refuses a new document too, with the same
    /// guidance, and leaves nothing behind.
    #[cfg(unix)]
    #[test]
    fn a_new_file_in_a_read_only_folder_is_refused_with_guidance() {
        let d = Scratch::new();
        let path = d.target();
        let Some(saved) = in_read_only(&d.0, || write(&path, b"New document.")) else { return };
        let message = saved.unwrap_err().to_string();
        assert!(message.contains("Save As"), "{message}");
        assert!(!path.exists());
    }

    /// When only the temporary file is refused (macOS lets the app write the file picked in the
    /// Save panel), a file that doesn't exist yet is created directly; nothing is replaced.
    #[test]
    fn a_new_file_is_created_directly_when_only_the_temporary_file_is_refused() {
        let d = Scratch::new();
        let path = d.target();
        let refused = io::Error::from(io::ErrorKind::PermissionDenied);
        create_directly(&path, refused, |file| file.write_all(b"New document.")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"New document.");
        assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
    }

    #[test]
    fn a_failed_direct_save_of_a_new_file_leaves_nothing() {
        let d = Scratch::new();
        let path = d.target();
        let refused = io::Error::from(io::ErrorKind::PermissionDenied);
        let err = create_directly(&path, refused, |file| {
            file.write_all(b"Partial")?;
            Err(io::Error::other("injected"))
        })
        .unwrap_err();
        assert!(err.to_string().contains("injected"));
        assert!(!path.exists());
    }

    /// The direct path never opens an existing file and never runs the save: the original is
    /// untouched and the error carries the Save As guidance.
    #[test]
    fn an_existing_file_is_never_written_directly() {
        let d = Scratch::new();
        let path = d.target();
        fs::write(&path, b"Original bytes.").unwrap();
        let refused = io::Error::from(io::ErrorKind::PermissionDenied);
        let mut ran = false;
        let err = create_directly(&path, refused, |file| {
            ran = true;
            file.write_all(b"New")
        })
        .unwrap_err();
        assert!(!ran);
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("Save As") && err.to_string().contains("left unchanged"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), b"Original bytes.");
    }

    #[cfg(unix)]
    #[test]
    fn new_temporary_files_are_private_until_prepared() {
        use std::os::unix::fs::PermissionsExt;
        let d = Scratch::new();
        let temp = Temporary::create(&d.target(), true).unwrap();
        assert_eq!(fs::metadata(&temp.path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    /// A new document gets the default mode, as `fs::write` would give it — not 0600; a replaced
    /// document keeps its own mode.
    #[cfg(unix)]
    #[test]
    fn new_documents_get_the_default_mode_and_replaced_ones_keep_theirs() {
        use std::os::unix::fs::PermissionsExt;
        let d = Scratch::new();
        let reference = d.0.join("reference.docx");
        fs::write(&reference, b"x").unwrap();
        let default_mode = fs::metadata(&reference).unwrap().permissions().mode() & 0o777;
        let path = d.target();
        write(&path, b"New document.").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, default_mode);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write(&path, b"Saved again.").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
        assert_eq!(fs::read(&path).unwrap(), b"Saved again.");
    }
}
