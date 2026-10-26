//! The folders automation may read and write (#769).
//!
//! `wordcraft-cli mcp --automation-read-root <dir> --automation-write-root <dir>` confines the
//! files an agent's commands reach; each flag may be given more than once (#3582).
//! [`AutomationRoots`] holds the folders. While they are in force, every path a command
//! reads goes through [`check_read`] and every path it writes through [`check_write`]:
//!
//! - The path is made absolute against the working directory, as the file system reads it. Its
//!   links are followed: for a file that doesn't exist yet, the links of its nearest existing
//!   folder, the rest of the path being plain names (`..` below a missing folder is refused, and so
//!   is a link that leads nowhere).
//! - The result must lie inside one of the roots, compared by whole components (`/work2` isn't
//!   inside `/work`), without regard to case on Windows.
//! - Writing replaces or removes a directory entry, so for write access the entry itself (its
//!   folder's links followed, its own name as given) must lie inside one of the write roots as
//!   well: a link outside them that leads into one is not a way to write outside (#3735).
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
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Read access or write access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// The folders automation may read and write; with no root of a kind, no access of that kind.
#[derive(Debug)]
pub struct AutomationRoots {
    read: Vec<Root>,
    write: Vec<Root>,
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
        Self::with_roots(read.as_slice(), write.as_slice())
    }

    /// Several read and write roots, which must all be existing folders: a path may be read (or
    /// written) when it lies inside any of them. `None` when no root is given (nothing is confined
    /// then).
    pub fn with_roots(read: &[&Path], write: &[&Path]) -> Result<Option<Arc<Self>>, String> {
        if read.is_empty() && write.is_empty() {
            return Ok(None);
        }
        let open = |paths: &[&Path], access| paths.iter().map(|p| Root::open(p, access)).collect::<Result<Vec<_>, _>>();
        Ok(Some(Arc::new(Self { read: open(read, Access::Read)?, write: open(write, Access::Write)? })))
    }

    /// The read roots, absolute with their links followed.
    pub fn read_roots(&self) -> Vec<&Path> {
        self.read.iter().map(|r| r.shown.as_path()).collect()
    }

    /// The write roots, absolute with their links followed.
    pub fn write_roots(&self) -> Vec<&Path> {
        self.write.iter().map(|r| r.shown.as_path()).collect()
    }

    /// May the file or folder at `path` be read (or written)? `Err` says why not.
    pub fn check(&self, path: &str, access: Access) -> Result<(), String> {
        let (roots, kind) = match access {
            Access::Read => (&self.read, "read"),
            Access::Write => (&self.write, "write"),
        };
        if roots.is_empty() {
            return Err(format!("automation filesystem access is not granted: {kind} authority is absent: {path}"));
        }
        let in_a_root = |p: &Path| roots.iter().any(|r| inside(p, &r.resolved));
        // Writing replaces or removes the directory entry `path` names (a safe save renames its
        // temporary file over it, a rename removes it): that entry must be inside a write root
        // too, not only what a link there leads to (#3735).
        let entry_inside = || match access {
            Access::Read => Ok(true),
            Access::Write => resolve_entry(path).map(|entry| in_a_root(&entry)),
        };
        if in_a_root(&resolve(path)?) && entry_inside()? {
            return Ok(());
        }
        let shown = roots.iter().map(|r| r.shown.display().to_string()).collect::<Vec<_>>().join(", ");
        let plural = if roots.len() > 1 { "s" } else { "" };
        Err(format!("automation path rejected: outside the {kind} root{plural} {shown}: {path}"))
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
fn resolve(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("automation path rejected: path is empty".into());
    }
    // Checked as given: making `…\NUL` absolute gives the device path `\\.\NUL` on Windows.
    if cfg!(windows) && Path::new(path).components().any(|c| matches!(c, Component::Normal(n) if is_device_name(&n.to_string_lossy()))) {
        return Err(format!("automation path rejected: reserved device names are not allowed: {path}"));
    }
    let absolute = std::path::absolute(path).map_err(|e| format!("automation path rejected: cannot resolve {path}: {e}"))?;
    resolve_absolute(path, &absolute)
}

/// The directory entry `path` names: its folder resolved as [`resolve`] resolves it, then its own
/// name as given, so a link there is the entry itself, not what it leads to. That entry is what
/// replacing or removing the file changes.
fn resolve_entry(path: &str) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|e| format!("automation path rejected: cannot resolve {path}: {e}"))?;
    let (Some(folder), Some(name)) = (absolute.parent(), absolute.file_name()) else {
        return Err(format!("automation path rejected: not a file name: {path}"));
    };
    let mut entry = resolve_absolute(path, folder)?;
    entry.push(name);
    Ok(entry)
}

/// [`resolve`] for `absolute`, `path` made absolute (`path` as given, for messages).
fn resolve_absolute(path: &str, absolute: &Path) -> Result<PathBuf, String> {
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

    /// Two read and two write roots (#3582): each grants its own access; paths outside all of
    /// them, through `..` or into a same-prefix sibling are refused, and the error names them all.
    #[test]
    fn several_roots_each_grant_their_access() {
        let (base, _) = folders("several");
        // Folder and file names with spaces, accents and typographic punctuation.
        let shared = base.join("Proj · Dé – shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("Smart Society 5.0 – WP leader meeting.docx"), "x").unwrap();
        let (r_in, r_shared, r_out) = (base.join("in"), shared.clone(), base.join("out"));
        let roots = AutomationRoots::with_roots(&[&r_in, &r_shared], &[&r_out, &r_shared]).unwrap().unwrap();
        assert_eq!(roots.read_roots().len(), 2);
        assert_eq!(roots.write_roots().len(), 2);
        // Inside each root: allowed, for the access each was given.
        assert_eq!(roots.check(&s(base.join("in/a.docx")), Access::Read), Ok(()));
        assert_eq!(roots.check(&s(shared.join("Smart Society 5.0 – WP leader meeting.docx")), Access::Read), Ok(()));
        assert_eq!(roots.check(&s(shared.join("Desktop home · target.png")), Access::Write), Ok(()), "a new file");
        assert_eq!(roots.check(&s(base.join("out/x.pdf")), Access::Write), Ok(()));
        assert!(roots.check(&s(base.join("in/a.docx")), Access::Write).unwrap_err().contains("outside the write roots"));
        // Outside every root: refused, naming them all.
        let e = roots.check(&s(base.join("outside/secret.docx")), Access::Read).unwrap_err();
        assert!(e.starts_with("automation path rejected: outside the read roots "), "{e}");
        let (in_shown, shared_shown) = (s(std::fs::canonicalize(&r_in).unwrap()), s(std::fs::canonicalize(&shared).unwrap()));
        assert!(e.contains(&format!("{}, {}", plain(in_shown.into()).display(), plain(shared_shown.into()).display())), "{e}");
        // A sibling with the same prefix as either root is outside.
        assert!(roots.check(&s(base.join("in2/c.docx")), Access::Read).is_err());
        assert!(roots.check(&format!("{} x/y.docx", s(shared.clone())), Access::Write).is_err(), "`… shared x` is not `… shared`");
        // `..` out of a root, even into the folder holding another root's parent.
        let sep = std::path::MAIN_SEPARATOR;
        let up = format!("{}{sep}..{sep}outside{sep}secret.docx", s(shared.clone()));
        assert!(roots.check(&up, Access::Read).unwrap_err().contains("outside the read roots"), "{up}");
        let across = format!("{}{sep}..{sep}in{sep}a.docx", s(shared.clone()));
        assert_eq!(roots.check(&across, Access::Read), Ok(()), "`..` that lands in another root");
        // Given more than once, a missing folder still stops it.
        let e = AutomationRoots::with_roots(&[&r_in, &base.join("nope")], &[]).unwrap_err();
        assert!(e.starts_with("cannot open the automation read root"), "{e}");
        assert!(AutomationRoots::with_roots(&[], &[]).unwrap().is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    /// With several roots, a link in any of them that leads out of all of them is outside; a link
    /// from one root into another is inside.
    #[cfg(unix)]
    #[test]
    fn a_link_in_any_root_that_leads_out_is_outside() {
        let (base, _) = folders("several-links");
        let shared = base.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let roots = AutomationRoots::with_roots(&[&base.join("in"), &shared], &[&shared]).unwrap().unwrap();
        std::os::unix::fs::symlink(base.join("outside/secret.docx"), shared.join("leak.docx")).unwrap();
        std::os::unix::fs::symlink(base.join("outside"), shared.join("away")).unwrap();
        assert!(roots.check(&s(shared.join("leak.docx")), Access::Read).unwrap_err().contains("outside the read roots"));
        assert!(roots.check(&s(shared.join("away/new.docx")), Access::Write).unwrap_err().contains("outside the write root"));
        // A link from one root into another stays inside.
        std::os::unix::fs::symlink(base.join("in/a.docx"), shared.join("a.docx")).unwrap();
        assert_eq!(roots.check(&s(shared.join("a.docx")), Access::Read), Ok(()));
        // Writing replaces the link itself (#3735): one outside every write root that leads into
        // one is outside; one inside a write root is inside, whichever write root it leads to.
        let write = AutomationRoots::with_roots(&[], &[&base.join("out"), &shared]).unwrap().unwrap();
        std::fs::write(base.join("out/doc.docx"), "x").unwrap();
        std::os::unix::fs::symlink(base.join("out/doc.docx"), base.join("outside/alias.docx")).unwrap();
        assert!(write.check(&s(base.join("outside/alias.docx")), Access::Write).unwrap_err().contains("outside the write roots"));
        std::os::unix::fs::symlink(base.join("out/doc.docx"), shared.join("alias.docx")).unwrap();
        assert_eq!(write.check(&s(shared.join("alias.docx")), Access::Write), Ok(()));
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
            // `io` and this module are the checked gateway; `tests*.rs` are test-only modules.
            if path.parent() == Some(src.as_path()) && (name == "io.rs" || name == "file_access.rs" || name.starts_with("tests"))
                || path.starts_with(src.join("io"))
                || path == src.join("recovery/desktop.rs")
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Test modules (which build their own fixtures) come last in each file.
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
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
