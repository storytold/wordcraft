//! The folders automation may read and write (#769).
//!
//! `wordcraft-cli mcp --automation-read-root <dir> --automation-write-root <dir>` confines the
//! files an agent's commands reach. [`AutomationRoots`] holds the two folders. While they are in force, every path a command
//! reads goes through [`check_read`] and every path it writes through [`check_write`]:
//!
//! - The path is made absolute against the working directory, as the file system reads it. Its
//!   links are followed: for a file that doesn't exist yet, the links of its nearest existing
//!   folder, the rest of the path being plain names (`..` below a missing folder is refused, and so
//!   is a link that leads nowhere).
//! - The result must lie inside the root, compared by whole components (`/work2` isn't inside
//!   `/work`), without regard to case on Windows.
//! - On Windows a network path (`\\server\share`, `\\?\UNC\...`, WebDAV's `\\host@SSL\share`), a
//!   device path (`\\.\...`) or a verbatim path that isn't a disk (a volume by its `\\?\Volume{…}` name is local) is refused by its name before
//!   the file system is asked about it, unless the root is on that same server and share (or that
//!   same verbatim prefix). Looking such a path up would contact a host the agent chose. A link
//!   whose text leads to one is refused before it is followed.
//! - Writing replaces or removes a directory entry, so for write access the entry itself (its
//!   folder's links followed, its own name as given) must lie inside the root as well: a link
//!   outside the root that leads into it is not a way to write outside (#3735).
//! - Read and write access are separate: a root given alone grants only its own kind of access.
//!
//! Roots are in force on a thread inside [`confine`], and in the whole process after
//! [`confine_process`] (`wordcraft-cli mcp`), except inside [`unconfined`]. Without roots
//! nothing is checked.
//!
//! A path is checked, then opened: a process that can change the folders in between (swapping in
//! a link) can still win that race. The roots keep an agent to the files it was given; they don't
//! guard against other programs on the computer.

use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Read access or write access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// The folders automation may read and write; a root left out grants no access of its kind.
#[derive(Debug)]
pub struct AutomationRoots {
    read: Option<Root>,
    write: Option<Root>,
}

#[derive(Debug)]
struct Root {
    /// The folder made absolute, links followed: what paths are compared with.
    resolved: PathBuf,
    /// The same without Windows' `\\?\` prefix, for people.
    shown: PathBuf,
}

impl Root {
    /// The folder at `path`, which must exist.
    fn open(path: &Path, access: Access) -> Result<Self, String> {
        let kind = match access {
            Access::Read => "read",
            Access::Write => "write",
        };
        let resolved = std::fs::canonicalize(path)
            .and_then(|p| if p.is_dir() { Ok(p) } else { Err(std::io::ErrorKind::NotADirectory.into()) })
            .map_err(|e| format!("cannot open the automation {kind} root `{}`: {e}", path.display()))?;
        Ok(Self { shown: plain(resolved.clone()), resolved })
    }
}

impl AutomationRoots {
    /// The roots `read` and `write`, which must be existing folders; `None` when neither is given
    /// (nothing is confined then).
    pub fn new(read: Option<&Path>, write: Option<&Path>) -> Result<Option<Arc<Self>>, String> {
        if read.is_none() && write.is_none() {
            return Ok(None);
        }
        let read = read.map(|p| Root::open(p, Access::Read)).transpose()?;
        let write = write.map(|p| Root::open(p, Access::Write)).transpose()?;
        Ok(Some(Arc::new(Self { read, write })))
    }

    /// The read root, absolute with its links followed.
    pub fn read_root(&self) -> Option<&Path> {
        self.read.as_ref().map(|r| r.shown.as_path())
    }

    /// The write root, absolute with its links followed.
    pub fn write_root(&self) -> Option<&Path> {
        self.write.as_ref().map(|r| r.shown.as_path())
    }

    /// May the file or folder at `path` be read (or written)? `Err` says why not.
    pub fn check(&self, path: &str, access: Access) -> Result<(), String> {
        let root = match access {
            Access::Read => self.read.as_ref(),
            Access::Write => self.write.as_ref(),
        };
        let Some(Root { resolved: folder, shown }) = root else {
            return match access {
                Access::Read => Err(format!("automation filesystem access is not granted: read authority is absent: {path}")),
                Access::Write => Err(format!("automation filesystem access is not granted: write authority is absent: {path}")),
            };
        };
        // Writing replaces or removes the directory entry `path` names (a safe save renames its
        // temporary file over it, a rename removes it): that entry must be inside the root too,
        // not only what a link there leads to (#3735).
        let roots = [folder.as_path()];
        let entry_inside = || match access {
            Access::Read => Ok(true),
            Access::Write => resolve_entry(path, &roots).map(|entry| inside(&entry, folder)),
        };
        if inside(&resolve(path, &roots)?, folder) && entry_inside()? {
            return Ok(());
        }
        let root = shown.display();
        match access {
            Access::Read => Err(format!("automation path rejected: outside the read root {root}: {path}")),
            Access::Write => Err(format!("automation path rejected: outside the write root {root}: {path}")),
        }
    }
}

/// Where the roots are in force on a thread ([`confine`], [`unconfined`]).
#[derive(Clone, Debug)]
enum Scope {
    Confined(Arc<AutomationRoots>),
    Unconfined,
}

thread_local! {
    /// This thread's scope; `None`: the process's roots ([`confine_process`]).
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// The roots in force in every thread outside [`confine`] and [`unconfined`].
static PROCESS: OnceLock<Arc<AutomationRoots>> = OnceLock::new();

/// Put `roots` in force in the whole process, for good. `false` when roots were in force already
/// (they stay as they were).
pub fn confine_process(roots: Arc<AutomationRoots>) -> bool {
    PROCESS.set(roots).is_ok()
}

/// Puts the scope it replaced back when dropped (also when `f` panics).
struct Restore(Option<Scope>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        SCOPE.with(|s| *s.borrow_mut() = previous);
    }
}

fn with_scope<T>(scope: Option<Scope>, f: impl FnOnce() -> T) -> T {
    let _restore = Restore(SCOPE.with(|s| s.replace(scope)));
    f()
}

/// Run `f` with `roots` in force on this thread (`None`: as things stand).
pub fn confine<T>(roots: Option<&Arc<AutomationRoots>>, f: impl FnOnce() -> T) -> T {
    match roots {
        Some(r) => with_scope(Some(Scope::Confined(r.clone())), f),
        None => f(),
    }
}

/// Run `f` with no roots in force on this thread: what the person at the keyboard does, and the
/// app's own folders.
pub fn unconfined<T>(f: impl FnOnce() -> T) -> T {
    with_scope(Some(Scope::Unconfined), f)
}

/// The roots in force on this thread, if any.
pub fn current() -> Option<Arc<AutomationRoots>> {
    match SCOPE.with(|s| s.borrow().clone()) {
        Some(Scope::Confined(r)) => Some(r),
        Some(Scope::Unconfined) => None,
        None => PROCESS.get().cloned(),
    }
}

/// `path` without Windows' `\\?\` prefix (which `canonicalize` adds), for people to read.
fn plain(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

/// May `path` be read here? Always, without roots in force.
pub fn check_read(path: &str) -> Result<(), String> {
    current().map_or(Ok(()), |r| r.check(path, Access::Read))
}

/// May `path` be written here? Always, without roots in force.
pub fn check_write(path: &str) -> Result<(), String> {
    current().map_or(Ok(()), |r| r.check(path, Access::Write))
}

/// `path` made absolute with its links followed; for a path that doesn't exist (yet), its nearest
/// existing folder's links followed and the rest appended.
///
/// A Windows network, device or non-disk verbatim prefix is refused before any of that, unless one
/// of `roots` (resolved folders) already stands on the same server and share or the same verbatim prefix.
fn resolve(path: &str, roots: &[&Path]) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("automation path rejected: path is empty".into());
    }
    // Checked as given: making `…\NUL` absolute gives the device path `\\.\NUL` on Windows.
    if cfg!(windows) && Path::new(path).components().any(|c| matches!(c, Component::Normal(n) if is_device_name(&n.to_string_lossy()))) {
        return Err(format!("automation path rejected: reserved device names are not allowed: {path}"));
    }
    // Win32 trims a trailing period or space from a name where it looks one up, but not always from a name in the middle: refused as given.
    if cfg!(windows) && has_trimmed_name(Path::new(path)) {
        return Err(format!("automation path rejected: a name Windows trims: {path}"));
    }
    refuse_prefix(path, Path::new(path), roots)?;
    let absolute = std::path::absolute(path).map_err(|e| format!("automation path rejected: cannot resolve {path}: {e}"))?;
    refuse_prefix(path, &absolute, roots)?;
    resolve_absolute(path, &absolute, roots)
}

/// `Err` naming the prefix when `at` (a path made from `path`, the way it was given) starts with a
/// Windows network, device or non-disk verbatim prefix that none of `roots` stands on.
fn refuse_prefix(path: &str, at: &Path, roots: &[&Path]) -> Result<(), String> {
    match refused_prefix(at, roots) {
        Some(why) => Err(format!("automation path rejected: {why}: {path}")),
        None => Ok(()),
    }
}

/// The directory entry `path` names: its folder resolved as [`resolve`] resolves it, then its own
/// name as given, so a link there is the entry itself, not what it leads to. That entry is what
/// replacing or removing the file changes.
fn resolve_entry(path: &str, roots: &[&Path]) -> Result<PathBuf, String> {
    // Win32 trims a trailing period or space from a name where it looks one up, but not always from a name in the middle: refused as given.
    if cfg!(windows) && has_trimmed_name(Path::new(path)) {
        return Err(format!("automation path rejected: a name Windows trims: {path}"));
    }
    refuse_prefix(path, Path::new(path), roots)?;
    let absolute = std::path::absolute(path).map_err(|e| format!("automation path rejected: cannot resolve {path}: {e}"))?;
    refuse_prefix(path, &absolute, roots)?;
    let (Some(folder), Some(name)) = (absolute.parent(), absolute.file_name()) else {
        return Err(format!("automation path rejected: not a file name: {path}"));
    };
    let mut entry = resolve_absolute(path, folder, roots)?;
    entry.push(name);
    Ok(entry)
}

/// [`resolve`] for `absolute`, `path` made absolute (`path` as given, for messages).
fn resolve_absolute(path: &str, absolute: &Path, roots: &[&Path]) -> Result<PathBuf, String> {
    // A link is judged by its own text: canonicalizing one that leads to a network path would
    // contact that host. Only Windows has such prefixes. One walk of the whole path covers every
    // shorter one the loop below tries.
    if cfg!(windows)
        && let Some(why) = link_leads_to_refused_prefix(absolute, roots)
    {
        return Err(format!("automation path rejected: {why}: {path}"));
    }
    // The names below the nearest existing folder, last first.
    let mut missing: Vec<OsString> = vec![];
    let mut at = absolute;
    loop {
        match std::fs::canonicalize(at) {
            Ok(mut found) => {
                found.extend(missing.iter().rev());
                return Ok(found);
            }
            Err(e) => match std::fs::symlink_metadata(at) {
                Ok(m) if m.file_type().is_symlink() => return Err(format!("automation path rejected: a link that leads nowhere: {path}")),
                // There, yet its real path can't be had (no permission): nothing to compare.
                Ok(_) => return Err(format!("automation path rejected: cannot resolve {path}: {e}")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("automation path rejected: cannot resolve {path}: {e}")),
            },
        }
        let parent = at.parent();
        match (at.components().next_back(), parent) {
            (Some(Component::Normal(name)), Some(parent)) => {
                missing.push(name.to_os_string());
                at = parent;
            }
            (Some(Component::ParentDir), _) => {
                return Err(format!("automation path rejected: parent traversal below a folder that doesn't exist: {path}"));
            }
            // Down to the root (or drive) and it doesn't exist either.
            _ => {
                let e = std::io::Error::from(std::io::ErrorKind::NotFound);
                return Err(format!("automation path rejected: cannot resolve {path}: {e}"));
            }
        }
    }
}

/// Is `path` (resolved) `root` or inside it, component by component (without regard to case on
/// Windows, whose file systems ignore it)?
fn inside(path: &Path, root: &Path) -> bool {
    let mut path = path.components();
    root.components().all(|r| path.next().is_some_and(|p| same_component(p, r)))
}

fn same_component(a: Component, b: Component) -> bool {
    if a == b {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    match (a.as_os_str().to_str(), b.as_os_str().to_str()) {
        (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
        _ => false,
    }
}

/// A Windows device name (`CON`, `NUL`, `COM1`… also with an extension), which names no file.
fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

/// A Windows path prefix that isn't a local disk: opening or even resolving a path that starts
/// with one can reach another computer.
#[derive(Debug)]
enum OffDisk {
    /// `\\server\share` or `\\?\UNC\server\share`; WebDAV's `\\host@SSL\share` is a server name too.
    Unc(OsString, OsString),
    /// `\\.\device`, which no root stands on: a configured one is canonicalized to a disk path.
    Device,
    /// `\\?\` followed by anything but a drive or `UNC`.
    Verbatim(OsString),
}

impl OffDisk {
    /// The prefix of `path` when it isn't a local disk. A drive letter, a path without a prefix
    /// and every path off Windows give `None`.
    fn of(path: &Path) -> Option<Self> {
        use std::path::Prefix;
        let Some(Component::Prefix(prefix)) = path.components().next() else { return Self::unparsed(path) };
        match prefix.kind() {
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => None,
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(Self::Unc(server.to_os_string(), share.to_os_string())),
            Prefix::DeviceNS(_) => Some(Self::Device),
            // A volume named by its GUID (a folder a volume is mounted in) is on this computer.
            Prefix::Verbatim(rest) if is_volume_name(rest) => None,
            Prefix::Verbatim(rest) => Some(Self::Verbatim(rest.to_os_string())),
        }
    }

    /// A Windows path `std` finds no prefix in, though Windows reads one: two separators that aren't a whole
    /// `\\server\share` (`\\server`, `\\\x`) name a computer, so that is a UNC path with no share; `\??\` is
    /// the NT form of `\\?\` (`\??\UNC\server\share`), so that is a device path.
    fn unparsed(path: &Path) -> Option<Self> {
        let bytes = path.as_os_str().as_encoded_bytes();
        let separator = |b: Option<&u8>| matches!(b, Some(b'\\' | b'/'));
        if !cfg!(windows) {
            None
        } else if separator(bytes.first()) && separator(bytes.get(1)) {
            Some(Self::Unc(OsString::new(), OsString::new()))
        } else if bytes.get(..4).is_some_and(|b| b.eq_ignore_ascii_case(br"\??\")) {
            Some(Self::Device)
        } else {
            None
        }
    }

    /// The refusal for a path with this prefix, which names the prefix.
    fn refusal(&self) -> &'static str {
        match self {
            Self::Unc(..) => "UNC paths are refused unless a root is on the same server and share",
            Self::Device => "device paths are refused",
            Self::Verbatim(_) => "verbatim paths are refused unless a root uses the same prefix",
        }
    }

    /// Whether `other` is the same server and share, or the same verbatim prefix, the ASCII case aside. This
    /// decides only whether that host may be contacted; [`inside`] still decides the folder.
    fn same_as(&self, other: &Self) -> bool {
        let same = |a: &OsString, b: &OsString| a.eq_ignore_ascii_case(b);
        match (self, other) {
            (Self::Unc(server, share), Self::Unc(other_server, other_share)) => same(server, other_server) && same(share, other_share),
            (Self::Verbatim(rest), Self::Verbatim(other)) => same(rest, other),
            _ => false,
        }
    }
}

/// Why `path` must not be looked up: it starts with a Windows network, device or non-disk
/// verbatim prefix and none of `roots` stands on that same server and share or verbatim prefix. A local path gives `None`.
fn refused_prefix(path: &Path, roots: &[&Path]) -> Option<&'static str> {
    let prefix = OffDisk::of(path)?;
    if roots.iter().filter_map(|root| OffDisk::of(root)).any(|root| root.same_as(&prefix)) { None } else { Some(prefix.refusal()) }
}

/// How many links in a row [`link_leads_to_refused_prefix`] follows before it gives up and refuses.
const MAX_LINK_HOPS: usize = 40;

/// How many spellings of a path one walk scans in all, so a hostile set of links can't make it
/// scan without end.
const MAX_SPELLINGS: usize = 1024;

/// The refusal for a link in `path`, taken from the link's own text.
///
/// Folders are inspected from the top down with [`std::fs::symlink_metadata`], which doesn't follow
/// a link, and a link's target is read with [`std::fs::read_link`]. A link to a network path is
/// therefore refused before [`std::fs::canonicalize`] could open that host, also when the link is
/// only an ancestor of the name. A link whose target can't be read, the one after
/// [`MAX_LINK_HOPS`] in a row, or a walk past [`MAX_SPELLINGS`] spellings, is refused rather than
/// opened.
fn link_leads_to_refused_prefix(path: &Path, roots: &[&Path]) -> Option<&'static str> {
    follow_links(path, roots, &mut std::collections::HashSet::new(), MAX_LINK_HOPS)
}

/// [`link_leads_to_refused_prefix`] for one spelling of the path. `judged` holds every spelling the
/// walk has scanned (so none is scanned twice, nor in a cycle forever) and `hops_left` the links
/// still allowed in this chain, which the other spelling of a target does not use up.
fn follow_links(path: &Path, roots: &[&Path], judged: &mut std::collections::HashSet<PathBuf>, hops_left: usize) -> Option<&'static str> {
    // Without a trailing separator, the rest as written: Windows reads `remote\` as a folder to go into, so the probe itself would follow a link
    // named `remote` to wherever it leads instead of seeing a link.
    let normalized = without_trailing_separator(path);
    let path = normalized.as_path();
    if !judged.insert(path.to_path_buf()) {
        return None;
    }
    if judged.len() > MAX_SPELLINGS {
        return Some("too many links");
    }
    let mut ancestors: Vec<&Path> = path.ancestors().collect();
    ancestors.reverse();
    for prefix in ancestors {
        if prefix.as_os_str().is_empty() {
            continue;
        }
        if let Some(why) = refused_prefix(prefix, roots) {
            return Some(why);
        }
        let meta = match std::fs::symlink_metadata(prefix) {
            Ok(meta) => meta,
            // Nothing is there, so no link is below it.
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => return None,
            // Anything else (no permission to look): a link below it can't be seen, so don't go on.
            Err(_) => return Some("a folder that can't be looked at"),
        };
        if !meta.file_type().is_symlink() {
            continue;
        }
        if hops_left == 0 {
            return Some("too many links");
        }
        let Ok(target) = std::fs::read_link(prefix) else { return Some("a link that leads nowhere") };
        if cfg!(windows) && is_native_name(&target) {
            return Some("device paths are refused");
        }
        // A reserved device name in a target (`NUL`, `COM1`) names the device in a Win32 lookup, so the joined path would not be the link's.
        if cfg!(windows) && target.components().any(|c| matches!(c, Component::Normal(n) if is_device_name(&n.to_string_lossy()))) {
            return Some("a link to a reserved device name");
        }
        // Win32 lookups trim a trailing period or space, the kernel follows a link target as written: the two could name different things.
        if cfg!(windows) && has_trimmed_name(&target) {
            return Some("a link to a name Windows trims");
        }
        // Judged as written: joined onto the link's folder, a bare `\\server` would keep that folder's drive.
        if let Some(why) = refused_prefix(&target, roots) {
            return Some(why);
        }
        let joined = match prefix.parent() {
            Some(parent) if !target.is_absolute() && !parent.as_os_str().is_empty() => parent.join(target),
            _ => target,
        };
        let next = match path.strip_prefix(prefix) {
            Ok(rest) if !rest.as_os_str().is_empty() => joined.join(rest),
            _ => joined,
        };
        if let Some(why) = refused_prefix(&next, roots) {
            return Some(why);
        }
        // Windows takes the `..` of a relative target by name, so `missing\..\remote` is `remote`
        // though `missing` isn't there: judge that spelling as well as the one written.
        let by_name = lexical(&next);
        if by_name != next
            && let Some(why) = follow_links(&by_name, roots, judged, hops_left - 1)
        {
            return Some(why);
        }
        return follow_links(&next, roots, judged, hops_left - 1);
    }
    None
}

/// Whether `rest` (the part after `\\?\`) names a volume by its GUID, `Volume{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`: a local volume, such as the one
/// a folder is a mount point of.
fn is_volume_name(rest: &OsStr) -> bool {
    let bytes = rest.as_encoded_bytes();
    bytes.len() == 44
        && bytes.get(..7).is_some_and(|b| b.eq_ignore_ascii_case(b"Volume{"))
        && bytes.ends_with(b"}")
        && bytes[7..43].iter().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { *b == b'-' } else { b.is_ascii_hexdigit() })
}

/// Whether `path` is a name that starts with one separator and no drive (`\Device\Mup\server\share`, `\RPC Control\x`,
/// `\??\UNC\x`, `\work`), as a link may hold one: `std` sees no prefix in it, yet the system reads it in its own object
/// namespace or on the link's drive, and may follow it to wherever it names. Joined onto the link's folder it would look local.
fn is_native_name(path: &Path) -> bool {
    matches!(path.as_os_str().as_encoded_bytes(), [b'\\', rest @ ..] if rest.first() != Some(&b'\\'))
}

/// `path` without the separator it ends in, when it has a name to end in: `C:\work\remote\` is `C:\work\remote`, but the
/// root `C:\` and a path ending in `..` are kept. Every other component is kept as it was written, `..` included.
fn without_trailing_separator(path: &Path) -> PathBuf {
    let ends_in_separator = matches!(path.as_os_str().as_encoded_bytes().last(), Some(b'\\' | b'/'));
    match (ends_in_separator, path.parent(), path.file_name()) {
        (true, Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

/// Whether any plain name in `path` ends in a period or a space, which Win32 path handling trims before it looks the name up.
fn has_trimmed_name(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::Normal(n) if n.as_encoded_bytes().last().is_some_and(|b| matches!(b, b'.' | b' '))))
}

/// `path` with its `.` and `..` taken by name, never above its root.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder holding `in` (read root), `out` (write root) and `outside`.
    fn folders(name: &str) -> (PathBuf, Arc<AutomationRoots>) {
        let base = std::env::temp_dir().join(format!("wc-file-access-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["in/sub", "out", "outside", "in2"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        std::fs::write(base.join("in/a.docx"), "x").unwrap();
        std::fs::write(base.join("outside/secret.docx"), "x").unwrap();
        let roots = AutomationRoots::new(Some(&base.join("in")), Some(&base.join("out"))).unwrap().unwrap();
        (base, roots)
    }

    fn s(p: PathBuf) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn paths_inside_the_roots_pass_and_others_dont() {
        let (base, roots) = folders("inside");
        assert_eq!(roots.check(&s(base.join("in/a.docx")), Access::Read), Ok(()));
        assert_eq!(roots.check(&s(base.join("in")), Access::Read), Ok(()), "the root itself");
        assert_eq!(roots.check(&s(base.join("in/sub/new.docx")), Access::Read), Ok(()), "missing files are checked by their folder");
        assert_eq!(roots.check(&s(base.join("out/new/deeper/x.png")), Access::Write), Ok(()), "and by their nearest folder");
        let e = roots.check(&s(base.join("outside/secret.docx")), Access::Read).unwrap_err();
        assert!(e.starts_with("automation path rejected: outside the read root"), "{e}");
        // Read and write access are separate.
        let e = roots.check(&s(base.join("in/a.docx")), Access::Write).unwrap_err();
        assert!(e.contains("outside the write root"), "{e}");
        assert!(roots.check(&s(base.join("out/x.png")), Access::Read).unwrap_err().contains("outside the read root"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_sibling_with_the_same_prefix_is_outside() {
        let (base, roots) = folders("prefix");
        std::fs::write(base.join("in2/b.docx"), "x").unwrap();
        assert!(roots.check(&s(base.join("in2/b.docx")), Access::Read).is_err(), "in2 is not inside in");
        assert!(roots.check(&format!("{}2/c.docx", s(base.join("in"))), Access::Read).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn parent_traversal_is_resolved_or_refused() {
        let (base, roots) = folders("dotdot");
        let sep = std::path::MAIN_SEPARATOR;
        let up = format!("{}{sep}..{sep}outside{sep}secret.docx", s(base.join("in")));
        assert!(roots.check(&up, Access::Read).unwrap_err().contains("outside the read root"), "`..` out of the root");
        let back = format!("{}{sep}..{sep}a.docx", s(base.join("in/sub")));
        assert_eq!(roots.check(&back, Access::Read), Ok(()), "`..` that stays inside");
        let missing = format!("{}{sep}nope{sep}..{sep}..{sep}outside{sep}x.docx", s(base.join("out")));
        let e = roots.check(&missing, Access::Write).unwrap_err();
        // Windows resolves `..` by name first (so does opening the file there): then it's outside.
        assert!(e.contains("parent traversal below a folder that doesn't exist") || e.contains("outside the write root"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn relative_paths_resolve_against_the_working_directory() {
        let (base, roots) = folders("relative");
        // The tests' working directory is the crate's folder: not inside the roots.
        assert!(roots.check("a.docx", Access::Read).unwrap_err().contains("outside the read root"));
        assert!(roots.check("", Access::Read).unwrap_err().contains("path is empty"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_missing_root_is_an_error_and_none_is_no_confinement() {
        let (base, _) = folders("missing");
        let e = AutomationRoots::new(Some(&base.join("nope")), None).unwrap_err();
        assert!(e.starts_with("cannot open the automation read root"), "{e}");
        let e = AutomationRoots::new(None, Some(&base.join("in/a.docx"))).unwrap_err();
        assert!(
            e.starts_with("cannot open the automation write root")
                && e.ends_with(&std::io::Error::from(std::io::ErrorKind::NotADirectory).to_string()),
            "{e}"
        );
        assert!(AutomationRoots::new(Some(Path::new("")), None).unwrap_err().starts_with("cannot open the automation read root ``"));
        assert!(AutomationRoots::new(None, None).unwrap().is_none());
        // One root alone grants only its own access.
        let read_only = AutomationRoots::new(Some(&base.join("in")), None).unwrap().unwrap();
        let e = read_only.check(&s(base.join("in/x.docx")), Access::Write).unwrap_err();
        assert!(e.contains("write authority is absent"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn links_are_followed_before_comparing() {
        let (base, roots) = folders("links");
        #[cfg(unix)]
        let made = |target: &Path, link: &Path| std::os::unix::fs::symlink(target, link).is_ok();
        #[cfg(windows)]
        let made = |target: &Path, link: &Path| {
            let made =
                if target.is_dir() { std::os::windows::fs::symlink_dir(target, link) } else { std::os::windows::fs::symlink_file(target, link) };
            made.is_ok()
        };
        #[cfg(not(any(unix, windows)))]
        let made = |_: &Path, _: &Path| false;
        // Windows needs Developer Mode or elevation to make links: nothing to test without them.
        if !made(&base.join("outside/secret.docx"), &base.join("in/leak.docx")) {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        assert!(roots.check(&s(base.join("in/leak.docx")), Access::Read).unwrap_err().contains("outside the read root"), "a file link out");
        assert!(made(&base.join("outside"), &base.join("out/away")));
        let e = roots.check(&s(base.join("out/away/new.png")), Access::Write).unwrap_err();
        assert!(e.contains("outside the write root"), "a folder link out: {e}");
        assert!(made(&base.join("outside/gone.docx"), &base.join("out/dangling.png")));
        let e = roots.check(&s(base.join("out/dangling.png")), Access::Write).unwrap_err();
        assert!(e.contains("a link that leads nowhere"), "writing would create the file outside: {e}");
        // A link to a folder inside the root stays inside.
        assert!(made(&base.join("in/sub"), &base.join("in/alias")));
        assert_eq!(roots.check(&s(base.join("in/alias/x.docx")), Access::Read), Ok(()));
        // A link outside the write root that leads into it: writing would replace the link,
        // outside (#3735). One inside that leads to a file inside is fine.
        std::fs::write(base.join("out/doc.docx"), "x").unwrap();
        assert!(made(&base.join("out/doc.docx"), &base.join("outside/alias.docx")));
        let e = roots.check(&s(base.join("outside/alias.docx")), Access::Write).unwrap_err();
        assert!(e.contains("outside the write root"), "the link itself is outside: {e}");
        assert!(made(&base.join("out/doc.docx"), &base.join("out/alias.docx")));
        assert_eq!(roots.check(&s(base.join("out/alias.docx")), Access::Write), Ok(()));
        // A folder link outside that leads into the root: the entry is inside, as the system opens it.
        assert!(made(&base.join("out"), &base.join("outside/to-out")));
        assert_eq!(roots.check(&s(base.join("outside/to-out/new.docx")), Access::Write), Ok(()));
        // A root given through a link is the folder it leads to.
        assert!(made(&base.join("in"), &base.join("in-link")));
        let linked = AutomationRoots::new(Some(&base.join("in-link")), None).unwrap().unwrap();
        assert_eq!(linked.check(&s(base.join("in/a.docx")), Access::Read), Ok(()));
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn windows_ignores_case_and_refuses_devices() {
        let (base, roots) = folders("case");
        let upper = s(base.join("IN/A.SVG"));
        assert_eq!(roots.check(&upper, Access::Read), Ok(()), "{upper}");
        assert_eq!(roots.check(&s(base.join("In/Sub/New.docx")), Access::Read), Ok(()));
        let e = roots.check(&s(base.join("out/NUL")), Access::Write).unwrap_err();
        assert!(e.contains("reserved device names"), "{e}");
        assert!(roots.check(&s(base.join("out/com1.txt")), Access::Write).is_err());
        assert_eq!(roots.check(&s(base.join("out/console.txt")), Access::Write), Ok(()));
        let _ = std::fs::remove_dir_all(base);
    }

    /// A path with no Windows prefix, a drive letter, and every path off Windows have no off-disk prefix to refuse.
    #[test]
    fn a_local_path_has_no_off_disk_prefix() {
        for p in ["/home/me/a.docx", "a.docx", r"C:\Projects\a.docx", r"\\?\C:\Projects\a.docx"] {
            assert!(OffDisk::of(Path::new(p)).is_none(), "{p}");
            assert_eq!(refused_prefix(Path::new(p), &[]), None, "{p}");
        }
    }

    /// A network, device or non-disk verbatim path is refused by its name before the file system is
    /// asked, so no host the agent names is contacted. The error names the prefix; it is not the
    /// error that opening the path would give. Nothing here touches the network.
    #[cfg(windows)]
    #[test]
    fn a_network_path_is_refused_before_it_is_opened() {
        let (base, roots) = folders("unc");
        let paths = [
            (r"\\203.0.113.5\s\x.docx", "UNC"),
            (r"\\?\UNC\203.0.113.5\s\x.docx", "UNC"),
            ("//203.0.113.5/s/x.docx", "UNC"),
            (r"\\host@SSL\share\x.docx", "UNC"),
            (r"\\host@SSL@443\share\x.docx", "UNC"),
            (r"\\203.0.113.5", "UNC"),
            (r"\\\203.0.113.5\s\x.docx", "UNC"),
            ("//203.0.113.5", "UNC"),
            (r"\\.\PhysicalDrive0", "device path"),
            (r"\\?\GLOBALROOT\Device\Mup\203.0.113.5\s\x.docx", "verbatim path"),
            (r"\??\UNC\203.0.113.5\s\x.docx", "device path"),
        ];
        for (p, kind) in paths {
            for access in [Access::Read, Access::Write] {
                let e = roots.check(p, access).unwrap_err();
                assert!(e.starts_with("automation path rejected:"), "{p}: {e}");
                assert!(e.contains(kind), "{p}: {e}");
                assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{p}: {e}");
            }
        }
        let e = roots.check(&s(base.join("outside/secret.docx")), Access::Read).unwrap_err();
        assert!(e.contains("outside the read root"), "a local path is still judged by its folder: {e}");
        let verbatim = format!(r"\\?\{}", s(base.join("in").join("a.docx")));
        assert_eq!(roots.check(&verbatim, Access::Read), Ok(()), "a verbatim disk path is a local path: {verbatim}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// A root on a share lets that server and share through (either spelling, any ASCII case) and nothing else.
    #[cfg(windows)]
    #[test]
    fn a_root_on_a_share_lets_only_that_share_through() {
        assert!(OffDisk::of(Path::new(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\x")).is_none(), "a volume by GUID is local");
        assert!(OffDisk::of(Path::new(r"\\?\Volume{remote}\x")).is_some(), "a name that is not a GUID is not a volume");
        let share = [Path::new(r"\\?\UNC\FileServer\Projects\work")];
        assert_eq!(refused_prefix(Path::new(r"\\fileserver\projects\a.docx"), &share), None);
        assert_eq!(refused_prefix(Path::new(r"\\?\UNC\FILESERVER\PROJECTS\a.docx"), &share), None);
        assert!(refused_prefix(Path::new(r"\\fileserver\secret\a.docx"), &share).is_some(), "another share");
        assert!(refused_prefix(Path::new(r"\\203.0.113.5\Projects\a.docx"), &share).is_some(), "another server");
        let work = [Path::new(r"\\?\UNC\work\Projects\w")];
        assert!(refused_prefix(Path::new("\\\\wor\u{212a}\\Projects\\a.docx"), &work).is_some(), "U+212A is not k");
        assert!(refused_prefix(Path::new(r"\\.\PhysicalDrive0"), &share).is_some(), "a share is not a device");
        let device = [Path::new(r"\\.\PhysicalDrive0")];
        assert!(refused_prefix(Path::new(r"\\.\PhysicalDrive0"), &device).is_some(), "a device is refused even when a root names it");
    }

    /// A link whose text is a bare `\\server` (no share) is refused as written: joined onto the link's folder
    /// it would read as a local path and pass.
    #[cfg(windows)]
    #[test]
    fn a_link_to_a_bare_server_name_is_refused() {
        let (base, roots) = folders("unc-link-bare");
        let link = base.join("in").join("bare");
        if std::os::windows::fs::symlink_dir(r"\\203.0.113.5", &link).is_err() {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let e = roots.check(&s(link.join("x.docx")), Access::Read).unwrap_err();
        assert!(e.contains("UNC"), "{e}");
        assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// Nine links that each go through `sub\..\` to the next are nine links, not a reason to give up: both
    /// spellings of each target are judged once.
    #[cfg(windows)]
    #[test]
    fn a_chain_of_links_with_parent_names_is_followed_in_few_steps() {
        let (base, roots) = folders("link-chain");
        let folder = base.join("in");
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        for n in 1..=9 {
            let target = if n == 9 { "sub".to_string() } else { format!(r"sub\..\l{}", n + 1) };
            if std::os::windows::fs::symlink_dir(&target, folder.join(format!("l{n}"))).is_err() {
                eprintln!("skipped: symbolic links can't be made here");
                let _ = std::fs::remove_dir_all(base);
                return;
            }
        }
        assert_eq!(roots.check(&s(folder.join("l1").join("x.docx")), Access::Read), Ok(()));
        let _ = std::fs::remove_dir_all(base);
    }

    /// A name that starts with one separator and no drive is a native or rooted name, whichever object directory it
    /// names (`\Device`, `\RPC Control`, `\BaseNamedObjects`, `\??`): a link to it is refused, and it is the same
    /// with the rest of the path after it.
    #[test]
    fn rooted_names_without_a_drive_are_native_names() {
        for p in
            [r"\Device", r"\Device\Mup\203.0.113.5\s", r"\RPC Control\remote", r"\BaseNamedObjects\x", r"\??\UNC\x\y", r"\GLOBAL??", r"\work", r"\"]
        {
            assert!(is_native_name(Path::new(p)), "{p}");
        }
        for p in [r"Device", r"work\x", r"C:\x", r"C:\Device", r"\\server\share", r"\\?\C:\x", ""] {
            assert!(!is_native_name(Path::new(p)), "{p}");
        }
    }

    /// A link whose text is a native NT name (`\Device\Mup\server\share`) isn't a local path just because
    /// `std` sees no prefix in it: it is refused as written, before it is joined onto the link's folder.
    #[cfg(windows)]
    #[test]
    fn a_link_to_a_native_device_name_is_refused() {
        let (base, roots) = folders("native-link");
        let link = base.join("in").join("native");
        if std::os::windows::fs::symlink_dir(r"\Device\Mup\203.0.113.5\s", &link).is_err()
            || std::fs::read_link(&link).ok().as_deref() != Some(std::path::Path::new(r"\Device\Mup\203.0.113.5\s"))
        {
            eprintln!("skipped: this system can't make that link, or stores its text differently");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let e = roots.check(&s(link.join("x.docx")), Access::Read).unwrap_err();
        assert!(e.contains("device path"), "{e}");
        assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// A link whose target ends in a period or a space (`remote.`) is refused as written: Win32 lookups trim
    /// that ending but the kernel follows the text as it is, so the two could name different things.
    #[cfg(windows)]
    #[test]
    fn a_link_to_a_name_windows_trims_is_refused() {
        let (base, roots) = folders("trimmed-link");
        let link = base.join("in").join("trimmed");
        if std::os::windows::fs::symlink_dir(r"remote.", &link).is_err() {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let e = roots.check(&s(link.join("x.docx")), Access::Read).unwrap_err();
        assert!(e.contains("name Windows trims"), "{e}");
        assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// A link whose target is a reserved device name (`nul`) is refused as written: in a Win32 lookup that names the device,
    /// so the target joined onto the link's folder would not be the file the kernel follows the link to.
    #[cfg(windows)]
    #[test]
    fn a_link_to_a_reserved_device_name_is_refused() {
        let (base, roots) = folders("device-link");
        let link = base.join("in").join("device");
        if std::os::windows::fs::symlink_dir(r"nul", &link).is_err() {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let e = roots.check(&s(link.join("x.docx")), Access::Read).unwrap_err();
        assert!(e.contains("reserved device name"), "{e}");
        assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// A trailing separator is dropped from the last name only: the root, a `..` and the other names stay as written.
    #[test]
    fn a_trailing_separator_is_dropped_from_the_last_name_only() {
        assert_eq!(without_trailing_separator(Path::new("a/b/")), Path::new("a/b"));
        assert_eq!(without_trailing_separator(Path::new("a/b//")), Path::new("a/b"));
        assert_eq!(without_trailing_separator(Path::new("a/b")), Path::new("a/b"));
        assert_eq!(without_trailing_separator(Path::new("a/b/../c/")), Path::new("a/b/../c"));
        assert_eq!(without_trailing_separator(Path::new("a/b/../")), Path::new("a/b/../"));
        #[cfg(windows)]
        {
            assert_eq!(without_trailing_separator(Path::new(r"C:\")), Path::new(r"C:\"));
            assert_eq!(without_trailing_separator(Path::new(r"\\?\C:\work\remote\..\a")), Path::new(r"\\?\C:\work\remote\..\a"));
            assert_eq!(without_trailing_separator(Path::new(r"\\?\C:\work\remote\")), Path::new(r"\\?\C:\work\remote"));
        }
    }

    /// A path that names a link with a trailing separator (`remote\`) is judged as the link: Windows reads that as a
    /// folder to go into, and would follow the link to a network path to find out what it is.
    #[cfg(windows)]
    #[test]
    fn a_link_with_a_trailing_separator_is_judged_as_the_link() {
        let (base, roots) = folders("slashed-link");
        let link = base.join("in").join("slashed");
        if std::os::windows::fs::symlink_dir(r"\\203.0.113.5\s", &link).is_err() {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let e = roots.check(&format!(r"{}\", link.display()), Access::Read).unwrap_err();
        assert!(e.contains("UNC"), "{e}");
        assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// A path with a name that ends in a space (`remote \x`) is refused as given: Windows keeps that space in the
    /// middle of a path but trims it when it looks the folder up by itself, so the two could name different things.
    #[cfg(windows)]
    #[test]
    fn a_path_with_a_name_windows_trims_is_refused() {
        let (base, roots) = folders("trimmed-path");
        let e = roots.check(&s(base.join("in").join("remote ").join("x.docx")), Access::Read).unwrap_err();
        assert!(e.contains("name Windows trims"), "{e}");
        assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// A link whose relative target goes through a missing folder and back (`missing\..\remote`) is
    /// judged with the `..` taken by name too, as Windows takes it: `remote` is a link to a network path.
    #[cfg(windows)]
    #[test]
    fn a_relative_link_through_a_missing_folder_still_reaches_the_network_check() {
        let (base, roots) = folders("unc-link-dotdot");
        let (remote, alias) = (base.join("in").join("remote"), base.join("in").join("alias"));
        if std::os::windows::fs::symlink_dir(r"\\203.0.113.5\s", &remote).is_err() {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        assert!(std::os::windows::fs::symlink_dir(r"missing\..\remote", &alias).is_ok());
        let e = roots.check(&s(alias.join("x.docx")), Access::Read).unwrap_err();
        assert!(e.contains("UNC"), "{e}");
        assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }

    /// A link whose text names a network path, or that has one as a folder on the way, is refused
    /// from that text before the link is followed: following it would contact the host.
    #[cfg(windows)]
    #[test]
    fn a_link_to_a_network_path_is_refused_before_it_is_followed() {
        let (base, roots) = folders("unc-link");
        let file_link = base.join("in/leak.docx");
        if std::os::windows::fs::symlink_file(r"\\203.0.113.5\s\x.docx", &file_link).is_err() {
            eprintln!("skipped: symbolic links can't be made here");
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let folder_link = base.join("in/folder-leak");
        assert!(std::os::windows::fs::symlink_dir(r"\\203.0.113.5\s", &folder_link).is_ok());
        for path in [file_link, folder_link.join("x.docx")] {
            let e = roots.check(&s(path.clone()), Access::Read).unwrap_err();
            assert!(e.contains("UNC"), "{}: {e}", path.display());
            assert!(!e.contains("cannot resolve") && !e.to_ascii_lowercase().contains("os error"), "{}: {e}", path.display());
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn scopes_nest_and_stay_on_their_thread() {
        let (base, roots) = folders("scopes");
        let outside = s(base.join("outside/secret.docx"));
        assert_eq!(check_read(&outside), Ok(()), "nothing is confined by default");
        confine(Some(&roots), || {
            assert!(check_read(&outside).is_err());
            assert_eq!(unconfined(|| check_read(&outside)), Ok(()));
            assert!(check_read(&outside).is_err(), "restored after unconfined");
            let o = outside.clone();
            let there = std::thread::spawn(move || check_read(&o).is_ok()).join().unwrap();
            assert!(there, "another thread is not confined by this thread's scope");
        });
        assert_eq!(check_read(&outside), Ok(()), "restored after confine");
        let r = std::panic::catch_unwind(|| confine(Some(&roots), || panic!("boom")));
        assert!(r.is_err());
        assert_eq!(check_read(&outside), Ok(()), "restored after a panic");
        assert_eq!(confine(None, || check_read(&outside)), Ok(()));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn device_names() {
        for d in ["CON", "nul", "Com1", "LPT9.txt", "aux.docx", "NUL "] {
            assert!(is_device_name(d), "{d}");
        }
        for f in ["console", "com10", "COM0", "lpt", "a.docx", "nul-x"] {
            assert!(!is_device_name(f), "{f}");
        }
    }

    /// Every file a command reads or writes by path goes through `io` (`open_path_with`,
    /// `save_path_with`, `read_file`, `write_file`), which check the roots. Engine code that
    /// reaches for `std::fs` itself would bypass them, so none may: every file under `src/`
    /// (commands, `print.rs`, `session.rs`…) except `io.rs` with its `io/` helpers (`safe_save`,
    /// reached only after `io` checked the roots), this module, and the test files.
    /// The host-enabled recovery store owns its profile files; commands address known snapshot
    /// IDs, never paths. Only that storage module is exempt, not recovery commands in general.
    #[test]
    fn commands_do_not_touch_the_file_system_themselves() {
        /// Every `.rs` file under `dir`, added to `files`.
        fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, files);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![];
        walk(&src, &mut files);
        assert!(files.iter().any(|f| f.ends_with("print.rs")) && files.iter().any(|f| f.ends_with("cmd/file.rs")), "{files:?}");
        let mut offenders = vec![];
        for path in files {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // `io` and this module are the checked gateway; `tests*.rs` files and `tests/`
            // directories are test-only modules.
            if path.parent() == Some(src.as_path()) && (name == "io.rs" || name == "file_access.rs")
                || name.starts_with("tests")
                || path.strip_prefix(&src).is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == "tests"))
                || path.starts_with(src.join("io"))
                || path == src.join("recovery/desktop.rs")
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Inline test modules (which build their own fixtures) come last in each file. A
            // `#[cfg(test)]` on a `use` line, a `mod tests;` declaration or another item doesn't end
            // the code.
            let end = text
                .match_indices("#[cfg(test)]")
                .find(|&(i, a)| {
                    let first = text[i + a.len()..].trim_start().lines().next().unwrap_or("");
                    first.starts_with("mod ") && first.trim_end().ends_with('{')
                })
                .map_or(text.len(), |(i, _)| i);
            let code = &text[..end];
            for (n, line) in code.lines().enumerate() {
                let line = line.trim_start();
                let touches = ["std::fs", "File::open", "File::create", "OpenOptions"].iter().any(|w| line.contains(w));
                if !line.starts_with("//") && touches {
                    offenders.push(format!("{}:{}: {line}", path.display(), n + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "use crate::io::{{read_file, write_file, open_path_with, save_path_with}}: {offenders:#?}");
    }
}
