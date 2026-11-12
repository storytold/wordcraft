//! An AppImage's environment inherited from another program (#5653).
//!
//! The AppImage runtime gives the program it starts `APPIMAGE` (the image's file), `APPDIR` (where
//! it is mounted), `ARGV0` and `OWD`, and whatever that program starts inherits them. So an AppImage
//! app that opens a document through `xdg-open` starts an installed WordCraft with *its* `APPIMAGE`,
//! and the self-update, which trusts that variable, replaces the other app's file with WordCraft.
//!
//! This copy is in the image only when its executable is under `APPDIR`. Otherwise the variables
//! are another program's, and the process starts over without them, before anything reads them.
//! (Removing them in place is `unsafe` under this edition, and `exec` keeps the process id, the
//! arguments and the standard streams.)

use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

/// What the AppImage runtime sets; the same list as the documented workaround on the issue.
const RUNTIME_VARS: [&str; 4] = ["APPIMAGE", "APPDIR", "ARGV0", "OWD"];

/// Whether the executable runs from the image mounted at `appdir`. Anything unclear counts as not:
/// the update then goes through the package or the plain file this copy really is, and at worst
/// says it can't, where the other answer would overwrite a file that isn't WordCraft's.
fn runs_from_image(exe: Option<&Path>, appdir: Option<&OsStr>) -> bool {
    let (Some(exe), Some(appdir)) = (exe, appdir) else { return false };
    let appdir = Path::new(appdir);
    if !appdir.is_absolute() {
        return false;
    }
    let Ok(appdir) = appdir.canonicalize() else { return false };
    // No image mounts at the root, where every program on the machine would count as inside it.
    appdir.parent().is_some() && exe.starts_with(&appdir)
}

/// Starts this process over without another program's AppImage variables; returns only when there
/// are none to drop (`None`) or starting over failed. Call it first, before any other thread starts.
pub(crate) fn drop_inherited() -> Option<std::io::Error> {
    let appimage = std::env::var_os("APPIMAGE")?;
    let exe = std::env::current_exe().ok();
    if runs_from_image(exe.as_deref(), std::env::var_os("APPDIR").as_deref()) {
        return None;
    }
    // The logger isn't up yet, and wouldn't survive `exec`. Unlike `eprintln!`, never panics.
    let _ = writeln!(std::io::stderr(), "wordcraft: ignoring the AppImage {:?} inherited from another program", Path::new(&appimage));
    let mut args = std::env::args_os();
    // This file, whatever has happened to its path since it started.
    let mut again = Command::new("/proc/self/exe");
    if let Some(arg0) = args.next() {
        again.arg0(arg0);
    }
    again.args(args);
    for var in RUNTIME_VARS {
        again.env_remove(var);
    }
    Some(again.exec())
}

#[cfg(test)]
mod tests {
    use super::runs_from_image;
    use std::path::{Path, PathBuf};

    /// A folder of its own for one test, removed when it ends.
    struct Tmp(PathBuf);

    impl Tmp {
        /// An empty folder named after `test` and this process, made afresh.
        fn new(test: &str) -> Tmp {
            let dir = std::env::temp_dir().join(format!("wordcraft-appimage-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
    }

    impl Drop for Tmp {
        /// Removes the folder and what the test left in it.
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A mount with the executable inside it, by real path (`/tmp` may be a link).
    fn mount(root: &Path, name: &str) -> (PathBuf, PathBuf) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("usr/bin")).unwrap();
        let dir = dir.canonicalize().unwrap();
        let exe = dir.join("usr/bin/wordcraft");
        (dir, exe)
    }

    #[test]
    fn an_executable_in_the_mount_is_the_image() {
        let tmp = Tmp::new("own");
        let (dir, exe) = mount(&tmp.0, ".mount_WordCr");
        assert!(runs_from_image(Some(&exe), Some(dir.as_os_str())));
    }

    #[test]
    fn a_link_to_the_mount_is_the_same_mount() {
        let tmp = Tmp::new("link");
        let (dir, exe) = mount(&tmp.0, ".mount_WordCr");
        let link = tmp.0.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(runs_from_image(Some(&exe), Some(link.as_os_str())));
    }

    #[test]
    fn an_installed_executable_is_not_in_another_programs_mount() {
        let tmp = Tmp::new("other");
        let (other, _) = mount(&tmp.0, ".mount_other");
        assert!(!runs_from_image(Some(Path::new("/usr/bin/wordcraft")), Some(other.as_os_str())));
    }

    /// Paths compare by component: one mount's name can start with another's.
    #[test]
    fn a_mount_whose_name_starts_the_same_is_another_mount() {
        let tmp = Tmp::new("prefix");
        let (other, _) = mount(&tmp.0, ".mount_ab");
        let (_, exe) = mount(&tmp.0, ".mount_abc");
        assert!(!runs_from_image(Some(&exe), Some(other.as_os_str())));
    }

    #[test]
    fn anything_unclear_is_not_the_image() {
        let tmp = Tmp::new("unclear");
        let (dir, exe) = mount(&tmp.0, ".mount_WordCr");
        // No `APPDIR`, one that's gone (the other program has exited) or empty.
        assert!(!runs_from_image(Some(&exe), None));
        assert!(!runs_from_image(Some(&exe), Some(tmp.0.join("gone").as_os_str())));
        assert!(!runs_from_image(Some(&exe), Some("".as_ref())));
        // The root, and a relative path that happens to lead here.
        assert!(!runs_from_image(Some(&exe), Some("/".as_ref())));
        assert!(!runs_from_image(Some(&exe), Some(".".as_ref())));
        // No executable path at all.
        assert!(!runs_from_image(None, Some(dir.as_os_str())));
    }
}
