//! Content identity for a session's native file. No polling or typing-path filesystem work.
use super::FileConflict;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufReader, Read};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;

const MAX_BYTES: u64 = 2 << 30;

#[derive(Clone, Debug)]
pub(crate) struct FileVersion {
    path: PathBuf,
    resolved: PathBuf,
    identity: Option<OriginalIdentity>,
    digest: [u8; 32],
    case_aliases: [bool; 2],
}

// Windows replacement on filesystems without POSIX rename support cannot keep the
// destination open. Capture its identity while reading, then close the handle.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OriginalIdentity {
    volume: u64,
    file: u64,
}
// Hold the original inode alive on Unix so an unlinked identity cannot be recycled.
#[cfg(not(windows))]
type OriginalIdentity = Arc<same_file::Handle>;

fn original_identity(handle: same_file::Handle) -> std::io::Result<OriginalIdentity> {
    #[cfg(windows)]
    {
        let info = winapi_util::file::information(handle.as_file())?;
        Ok(OriginalIdentity { volume: info.volume_serial_number(), file: info.file_index() })
    }
    #[cfg(not(windows))]
    {
        Ok(Arc::new(handle))
    }
}

/// Keep the logical spelling: a retargeted parent symlink still names the session's file.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Resolve aliases even when an externally deleted final entry no longer canonicalizes.
fn resolved_spelling(path: &Path) -> Option<PathBuf> {
    match (path.parent().and_then(|p| fs::canonicalize(p).ok()), path.file_name()) {
        (Some(parent), Some(name)) => Some(parent.join(name)),
        _ => None,
    }
}

/// Unix opens must not wait for a writer if a path turns into a FIFO during lookup.
fn open_read(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags};
        rustix::fs::open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map(File::from).map_err(Into::into)
    }
    #[cfg(not(unix))]
    {
        File::open(path)
    }
}

#[derive(Clone, Copy)]
enum EntryKind {
    File,
    Directory,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntryIdentity {
    device: u64,
    inode: u64,
}
#[cfg(not(unix))]
type EntryIdentity = same_file::Handle;

/// Unix identity lookup needs metadata, not read permission on an unrelated Save As target.
/// Owned destinations still undergo the separate content check immediately before replacement.
fn entry_identity(path: &Path, kind: EntryKind) -> std::io::Result<EntryIdentity> {
    #[cfg(unix)]
    {
        let meta = fs::metadata(path)?;
        let accepts = match kind {
            EntryKind::File => meta.is_file(),
            EntryKind::Directory => meta.is_dir(),
        };
        if !accepts {
            return Err(std::io::Error::other("unexpected entry type during file identity check"));
        }
        Ok(EntryIdentity { device: meta.dev(), inode: meta.ino() })
    }
    #[cfg(not(unix))]
    {
        entry_handle(path, kind)
    }
}

/// Identity checks accept only the expected entry type, before and after opening it.
fn entry_handle(path: &Path, kind: EntryKind) -> std::io::Result<same_file::Handle> {
    let accepts = |meta: &fs::Metadata| match kind {
        EntryKind::File => meta.is_file(),
        EntryKind::Directory => meta.is_dir(),
    };
    if !accepts(&fs::metadata(path)?) {
        return Err(std::io::Error::other("unexpected entry type during file identity check"));
    }
    #[cfg(unix)]
    let handle = same_file::Handle::from_file(open_read(path)?)?;
    #[cfg(not(unix))]
    let handle = same_file::Handle::from_path(path)?;
    if !accepts(&handle.as_file().metadata()?) {
        return Err(std::io::Error::other("entry type changed during file identity check"));
    }
    Ok(handle)
}

fn regular(path: &Path) -> std::io::Result<File> {
    let meta = fs::metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a file (expected a regular file)"));
    }
    if meta.len() > MAX_BYTES {
        return Err(std::io::Error::other("file larger than 2 GB"));
    }
    let file = open_read(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a file (expected a regular file)"));
    }
    if meta.len() > MAX_BYTES {
        return Err(std::io::Error::other("file larger than 2 GB"));
    }
    Ok(file)
}

/// Observe case aliases while the entry exists; deletion otherwise loses that evidence.
/// No files are created. An uncertain lookup reserves case-equivalent names conservatively.
fn observes_case_aliases(path: &Path) -> bool {
    let Some(name) = path.file_name() else { return false };
    let mut other = name.to_ascii_uppercase();
    if other == name {
        other = name.to_ascii_lowercase();
    }
    if other == name
        && let Some(text) = name.to_str()
    {
        other = text.to_uppercase().into();
        if other == name {
            other = text.to_lowercase().into();
        }
    }
    if other == name {
        return false;
    }
    entry_identity(path, EntryKind::File)
        .and_then(|a| entry_identity(&path.with_file_name(other), EntryKind::File).map(|b| a == b))
        .unwrap_or_else(|e| e.kind() != std::io::ErrorKind::NotFound || !path.exists())
}

/// Canonically equivalent Unicode names stay reserved after deletion. Case aliases are
/// reserved where they were observed; distinct ASCII case remains available on Linux.
fn equivalent_missing_name(a: &OsStr, b: &OsStr, folds_case: bool) -> bool {
    if a == b || folds_case && a.eq_ignore_ascii_case(b) {
        return true;
    }
    let (Some(a), Some(b)) = (a.to_str(), b.to_str()) else { return false };
    a.nfd().eq(b.nfd())
        || folds_case
            && (a.chars().flat_map(char::to_uppercase).nfd().eq(b.chars().flat_map(char::to_uppercase).nfd())
                || a.chars().flat_map(char::to_lowercase).nfd().eq(b.chars().flat_map(char::to_lowercase).nfd()))
}

impl FileVersion {
    /// Native opens require a regular file so subsequent saves have a stable content baseline.
    /// Stream import needs separate semantics rather than bypassing identity verification.
    pub(super) fn read(path: &Path) -> Result<(Vec<u8>, Self), String> {
        let read = || -> std::io::Result<_> {
            let mut file = regular(path)?;
            let mut bytes = Vec::new();
            (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(std::io::Error::other("file grew beyond 2 GB"));
            }
            let identity = original_identity(same_file::Handle::from_file(file)?)?;
            let path = absolute(path);
            let resolved = fs::canonicalize(&path)?;
            let version = Self {
                case_aliases: [observes_case_aliases(&path), observes_case_aliases(&resolved)],
                path,
                resolved,
                identity: Some(identity),
                digest: Sha256::digest(&bytes).into(),
            };
            Ok((bytes, version))
        };
        read().map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Match a host's byte-backed open to its original read, without adopting a later disk version.
    pub(crate) fn matches_bytes(&self, bytes: &[u8]) -> bool {
        <[u8; 32]>::from(Sha256::digest(bytes)) == self.digest
    }

    /// Track the logical session path without adopting different file content.
    pub(crate) fn track_path(&mut self, path: &Path) {
        self.path = absolute(path);
        self.resolved = fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());
        self.case_aliases = [observes_case_aliases(&self.path), observes_case_aliases(&self.resolved)];
    }

    /// Path spellings, symlinks and hard links to the open file share its expected content.
    pub(crate) fn owns(&self, path: &Path) -> Result<bool, FileConflict> {
        let absolute = absolute(path);
        if absolute == self.path || absolute == self.resolved {
            return Ok(true);
        }
        // A replaced final symlink must not hide the directory entry being saved.
        if resolved_spelling(path).is_some_and(|p| p == self.resolved) || fs::canonicalize(path).is_ok_and(|p| p == self.resolved) {
            return Ok(true);
        }
        let conflict = || FileConflict { path: path.to_path_buf() };
        let current = match entry_identity(path, EntryKind::File) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return self.owns_missing(&absolute),
            Ok(identity) => identity,
            Err(_) => return Err(conflict()),
        };
        // The original handle can name an old inode after replacement. Compare the current
        // tracked entries too, including case aliases on filesystems that preserve spelling.
        for owned in [&self.path, &self.resolved] {
            match entry_identity(owned, EntryKind::File) {
                Ok(handle) if handle == current => return Ok(true),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(conflict()),
            }
        }
        let Some(original) = &self.identity else { return Err(FileConflict { path: path.to_path_buf() }) };
        #[cfg(windows)]
        {
            // Length is mutable: a changed-size hard link still belongs to the original file.
            original_identity(current).map(|identity| identity == *original).map_err(|_| conflict())
        }
        #[cfg(unix)]
        {
            original.as_file().metadata().map(|meta| EntryIdentity { device: meta.dev(), inode: meta.ino() } == current).map_err(|_| conflict())
        }
        #[cfg(not(any(unix, windows)))]
        {
            original.as_file().try_clone().and_then(same_file::Handle::from_file).map(|original| original == current).map_err(|_| conflict())
        }
    }

    fn owns_missing(&self, path: &Path) -> Result<bool, FileConflict> {
        let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else { return Ok(false) };
        for (owned, folds_case) in [&self.path, &self.resolved].into_iter().zip(self.case_aliases) {
            let (Some(original), Some(original_parent)) = (owned.file_name(), owned.parent()) else { continue };
            if !equivalent_missing_name(name, original, folds_case) {
                continue;
            }
            match entry_identity(parent, EntryKind::Directory).and_then(|a| entry_identity(original_parent, EntryKind::Directory).map(|b| a == b)) {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(FileConflict { path: path.to_path_buf() }),
            }
        }
        Ok(false)
    }

    pub(super) fn verify(&self, path: &Path) -> Result<(), FileConflict> {
        let check = || -> std::io::Result<bool> {
            let file = regular(path)?;
            let mut reader = BufReader::new(file).take(MAX_BYTES + 1);
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            let mut total = 0u64;
            loop {
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > MAX_BYTES {
                    return Ok(false);
                }
                // Read's successful byte count is bounded by the supplied buffer.
                digest.update(&buffer[..n]);
            }
            Ok(<[u8; 32]>::from(digest.finalize()) == self.digest)
        };
        if check().unwrap_or(false) { Ok(()) } else { Err(FileConflict { path: path.to_path_buf() }) }
    }

    /// Keep the identity of serialized bytes without reading a later writer's content.
    /// A destination we cannot open for reading is only a copy, since Save must verify it.
    pub(super) fn saved(path: &Path, bytes: &[u8]) -> Option<Self> {
        let path = absolute(path);
        let handle = entry_handle(&path, EntryKind::File);
        if handle.as_ref().is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied) {
            return None;
        }
        let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        Some(Self {
            case_aliases: [observes_case_aliases(&path), observes_case_aliases(&resolved)],
            path,
            resolved,
            identity: handle.and_then(original_identity).ok(),
            digest: Sha256::digest(bytes).into(),
        })
    }
}
