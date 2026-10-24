//! More than one document: one window is one document, so File › New / Open from a window that
//! already holds a document starts another WordCraft process instead of replacing it (as Word
//! does). A separate process keeps each window's Save Changes prompt, Ctrl+W / Cmd+W and
//! quitting independent. The child gets the document on its command line, never as an Apple
//! event, so macOS can't route it back into this process.

use std::path::{Path, PathBuf};
use wordcraft_ui_egui::NewWindow;

/// The command line the new process gets (after the program).
pub fn child_args(target: &NewWindow) -> Vec<String> {
    match target {
        NewWindow::Open(path) => {
            // A leading `-` would read as a flag; the file stays a file.
            let p = absolute(Path::new(path));
            let p = p.to_string_lossy().to_string();
            if p.starts_with('-') { vec![format!("./{p}")] } else { vec![p] }
        }
        NewWindow::New(template) => vec!["--new".into(), template.clone()],
    }
}

/// `p` made absolute against the current directory, so the new process finds the same file.
fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf()) }
}

/// The `.app` bundle holding `exe` (`X.app/Contents/MacOS/X`), if it is in one.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

/// The program and arguments that start the new window. In a macOS bundle `open -n` starts a new
/// instance the normal way (Dock icon, focus); otherwise the executable runs directly.
pub fn command_for(exe: &Path, target: &NewWindow) -> (PathBuf, Vec<String>) {
    let args = child_args(target);
    match bundle_of(exe) {
        Some(bundle) if cfg!(target_os = "macos") => {
            let mut a = vec!["-n".to_string(), bundle.to_string_lossy().to_string(), "--args".to_string()];
            a.extend(args);
            (PathBuf::from("/usr/bin/open"), a)
        }
        _ => (exe.to_path_buf(), args),
    }
}

/// Start the new window.
pub fn spawn(target: NewWindow) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| failure(&e.to_string()))?;
    let (program, args) = command_for(&exe, &target);
    let mut cmd = std::process::Command::new(&program);
    cmd.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // The control channel and its key belong to this window.
    cmd.env_remove("WORDCRAFT_CONTROL_PORT");
    let fail = |e: std::io::Error| failure(&e.to_string());
    if program == Path::new("/usr/bin/open") {
        // Without `-W`, `open` returns once LaunchServices has accepted the request.
        return open_result(cmd.status().map_err(fail)?);
    }
    let mut child = cmd.spawn().map_err(fail)?;
    // The new window outlives this call; reap it when it exits so it doesn't stay a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The status-bar text when the new window couldn't start.
fn failure(error: &str) -> String {
    wordcraft_ui_egui::i18n::fmt(wordcraft_ui_egui::tl!("Couldn't open a new window: {error}"), &[("error", error)])
}

/// `open`'s exit status as the hook's result.
fn open_result(status: std::process::ExitStatus) -> Result<(), String> {
    if status.success() { Ok(()) } else { Err(failure(&format!("open exited with {status}"))) }
}

/// The hook for `Services::new_window`.
pub fn hook() -> Box<dyn Fn(NewWindow) -> Result<(), String>> {
    Box::new(spawn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open passes an absolute path; New passes `--new` and the template.
    #[test]
    fn open_passes_an_absolute_path_and_new_passes_the_template() {
        assert_eq!(child_args(&NewWindow::Open("/tmp/a.docx".into())), vec!["/tmp/a.docx"]);
        assert_eq!(child_args(&NewWindow::New("letter".into())), vec!["--new", "letter"]);
        let rel = child_args(&NewWindow::Open("a.docx".into()));
        assert!(Path::new(&rel[0]).is_absolute());
    }

    /// Only an `X.app/Contents/MacOS/X` executable counts as a bundle.
    #[test]
    fn bundle_is_found_only_for_app_layout() {
        assert_eq!(bundle_of(Path::new("/Applications/WordCraft.app/Contents/MacOS/WordCraft")), Some(PathBuf::from("/Applications/WordCraft.app")));
        assert_eq!(bundle_of(Path::new("/x/target/release/wordcraft")), None);
    }

    /// Outside a bundle the executable itself runs, with the file.
    #[test]
    fn unbundled_runs_the_executable_with_the_path() {
        let (prog, args) = command_for(Path::new("/x/wordcraft"), &NewWindow::Open("/tmp/a.docx".into()));
        assert_eq!((prog, args), (PathBuf::from("/x/wordcraft"), vec!["/tmp/a.docx".to_string()]));
    }

    #[cfg(unix)]
    /// `open`'s failing exit status becomes the error text.
    #[test]
    fn open_result_maps_the_exit_status() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(open_result(std::process::ExitStatus::from_raw(0)), Ok(()));
        let e = open_result(std::process::ExitStatus::from_raw(1 << 8)).unwrap_err();
        assert!(e.starts_with("Couldn't open a new window: open exited with"), "{e}");
    }

    #[cfg(target_os = "macos")]
    /// In a macOS bundle the new window starts through `open -n … --args`.
    #[test]
    fn bundled_macos_uses_open_n_with_args() {
        let (prog, args) = command_for(Path::new("/Applications/WordCraft.app/Contents/MacOS/WordCraft"), &NewWindow::Open("/tmp/a.docx".into()));
        assert_eq!(prog, PathBuf::from("/usr/bin/open"));
        assert_eq!(args, vec!["-n", "/Applications/WordCraft.app", "--args", "/tmp/a.docx"]);
    }
}
