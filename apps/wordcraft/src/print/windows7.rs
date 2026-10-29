//! The experimental Windows 7 build's spooler (docs/windows7.md), as in VectorCraft's Windows 7
//! build. craft-print draws jobs with `Windows.Data.Pdf`, a Windows Runtime API Windows 7 doesn't
//! have, so this build lists printers through PowerShell's WMI (`Win32_Printer`) and hands the
//! print-ready PDF to the installed PDF viewer's Print / PrintTo verb. The viewer prints with its
//! own settings: copies, two-sided and grayscale chosen in WordCraft don't reach it, and the
//! result says so.

/// Where a job's PDF waits for the viewer: a fresh file in `dir`. The viewer reads it after the
/// verb returns, so it can't be removed then; files of jobs more than an hour old are removed
/// first instead. Never truncates an existing file, and removes one it couldn't finish.
pub(crate) fn job_file(dir: &std::path::Path, pdf: &[u8]) -> Result<std::path::PathBuf, String> {
    use std::io::Write as _;
    std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    sweep(dir);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let path = dir.join(format!("job-{}-{stamp}.pdf", std::process::id()));
    let mut file =
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("can't create {}: {e}", path.display()))?;
    if let Err(e) = file.write_all(pdf).and_then(|()| file.flush()) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(format!("can't write {}: {e}", path.display()));
    }
    Ok(path)
}

/// Removes the job files in `dir` more than an hour old: their viewer has long read them. Runs
/// before each job and whenever the printers are listed, so a job's file outlives it by about an
/// hour of using WordCraft's printing. A file still in use stays for the next sweep; the folder is
/// the user's own temp folder.
pub(crate) fn sweep(dir: &std::path::Path) {
    // A folder of a few files; the cap only bounds a runaway listing.
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten().take(4096) {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 3600);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The folder job files wait in for the viewer.
#[cfg(target_vendor = "win7")]
fn job_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("wordcraft-print")
}

/// PowerShell's `<Default>\t<Name>` lines → the printers, the default first.
pub(crate) fn parse_printers(text: &str) -> Vec<wordcraft_ui_egui::printing::Printer> {
    let mut printers: Vec<wordcraft_ui_egui::printing::Printer> = text
        .lines()
        .filter_map(|l| l.trim_end_matches('\r').split_once('\t'))
        .filter(|(_, name)| !name.trim().is_empty())
        .map(|(default, name)| wordcraft_ui_egui::printing::Printer {
            name: name.trim().to_string(),
            label: String::new(),
            default: default.trim().eq_ignore_ascii_case("true"),
        })
        .collect();
    printers.sort_by_key(|p| !p.default);
    printers
}

/// The environment variables that carry the job's file and printer into PowerShell. A value in
/// the environment is data to PowerShell, never parsed as script, whatever quotes it contains
/// (PowerShell also ends a single-quoted string at ‘ ’ ‚ ‛), so neither is ever written into the
/// script's source.
const FILE_VAR: &str = "WORDCRAFT_PRINT_FILE";
const PRINTER_VAR: &str = "WORDCRAFT_PRINT_PRINTER";

/// The fixed scripts: Print on the default printer, PrintTo on a named one. Each writes `ok`
/// once the viewer has been started; `$ErrorActionPreference = 'Stop'` turns a failure into no `ok`.
const PRINT_SCRIPT: &str = "Start-Process -FilePath $env:WORDCRAFT_PRINT_FILE -Verb Print -WindowStyle Hidden; 'ok'";
const PRINT_TO_SCRIPT: &str =
    "Start-Process -FilePath $env:WORDCRAFT_PRINT_FILE -Verb PrintTo -ArgumentList $env:WORDCRAFT_PRINT_PRINTER -WindowStyle Hidden; 'ok'";

/// The printers, as `<Default>\t<Name>` lines (a fixed script: it reads nothing from WordCraft).
#[cfg(target_vendor = "win7")]
const PRINTERS_SCRIPT: &str = "Get-WmiObject Win32_Printer | ForEach-Object { \"$($_.Default)`t$($_.Name)\" }";

/// The script and environment that print `file` on `printer` (the default printer when blank)
/// through the PDF viewer's verb. PrintTo takes the printer name in double quotes, which a
/// Windows printer name can't contain.
pub(crate) fn print_invocation(file: &str, printer: &str) -> (&'static str, Vec<(&'static str, String)>) {
    let mut env = vec![(FILE_VAR, file.to_string())];
    match printer.trim() {
        "" => (PRINT_SCRIPT, env),
        p => {
            env.push((PRINTER_VAR, format!("\"{}\"", p.replace('"', ""))));
            (PRINT_TO_SCRIPT, env)
        }
    }
}

/// How long PowerShell may take to list the printers or start the viewer: a hung WMI query or
/// print handler must not leave discovery or a job pending for ever.
#[cfg(target_vendor = "win7")]
const LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The most of PowerShell's output that is kept (a printer list is a few kilobytes).
#[cfg(any(test, target_vendor = "win7"))]
const OUTPUT_CAP: u64 = 1 << 20;

/// Runs `command` and collects its standard output, giving it `limit`: a command still running
/// then is killed and reaped, and the error is `TimedOut`. The output is read on a thread of its
/// own, so a full pipe never holds the command past the deadline. The exit status isn't checked.
#[cfg(any(test, target_vendor = "win7"))]
pub(crate) fn output_within(command: &mut std::process::Command, limit: std::time::Duration) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let stop = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    let Some(mut stdout) = child.stdout.take() else {
        stop(&mut child);
        return Err(std::io::Error::other("the command's output could not be read"));
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new().name("powershell output".into()).spawn(move || {
        let mut out = Vec::new();
        let read = (&mut stdout).take(OUTPUT_CAP).read_to_end(&mut out);
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
        let _ = tx.send(read.map(|_| out));
    });
    if let Err(e) = reader {
        stop(&mut child);
        return Err(e);
    }
    let timed_out = || std::io::Error::new(std::io::ErrorKind::TimedOut, format!("no answer within {limit:?}"));
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                // A moment's grace for output written just before the exit. A process the command
                // started may keep the pipe open; the wait for the output ends at the deadline too.
                let left = deadline.saturating_duration_since(std::time::Instant::now()).max(std::time::Duration::from_millis(100));
                return match rx.recv_timeout(left) {
                    Ok(read) => read,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(timed_out()),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(std::io::Error::other("the command's output was lost")),
                };
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                stop(&mut child);
                return Err(timed_out());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(e) => {
                stop(&mut child);
                return Err(e);
            }
        }
    }
}

/// What the status bar is told once the viewer has the job: the settings the viewer decides.
pub(crate) fn handed_over(copies: u32) -> String {
    let mut note = String::from("handed to the PDF viewer's print command, which prints with its own settings");
    if copies > 1 {
        note.push_str(&format!(" (one copy, not {copies})"));
    }
    note
}

/// Windows PowerShell (2.0 on Windows 7 SP1) running `script`, without a console window.
#[cfg(target_vendor = "win7")]
fn powershell(script: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let root = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()));
    let mut c = std::process::Command::new(root.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"));
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!("$ErrorActionPreference = 'Stop'; [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false); {script}"),
    ])
    .creation_flags(CREATE_NO_WINDOW);
    c
}

/// The spooler's printers; empty when they can't be listed.
#[cfg(target_vendor = "win7")]
pub fn printers() -> Vec<wordcraft_ui_egui::printing::Printer> {
    sweep(&job_dir());
    match output_within(&mut powershell(PRINTERS_SCRIPT), LIMIT) {
        Ok(out) => parse_printers(&String::from_utf8_lossy(&out)),
        Err(e) => {
            log::warn!("listing printers: PowerShell: {e}");
            Vec::new()
        }
    }
}

/// Print `pdf` on `printer` through the PDF viewer's verb.
#[cfg(target_vendor = "win7")]
pub fn submit(pdf: &[u8], printer: &str, copies: u32) -> Result<String, String> {
    let file = job_file(&job_dir(), pdf)?;
    let (script, env) = print_invocation(&file.to_string_lossy(), printer);
    let mut command = powershell(script);
    command.envs(env);
    let failed = match output_within(&mut command, LIMIT) {
        Ok(out) if String::from_utf8_lossy(&out).trim() == "ok" => None,
        Ok(_) => Some("PowerShell couldn't start it".to_string()),
        Err(e) => Some(format!("PowerShell: {e}")),
    };
    if let Some(e) = failed {
        // No viewer took the job, so nothing will read the file.
        let _ = std::fs::remove_file(&file);
        return Err(format!("no app prints PDF files here; install a PDF viewer that can print, or export a PDF ({e})"));
    }
    Ok(handed_over(copies))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_printer_lines_are_read_default_first() {
        let printers = parse_printers("False\tOffice Laser\r\nTrue\tHome Inkjet\r\nFalse\t \r\nno tab\r\n");
        let names: Vec<(&str, bool)> = printers.iter().map(|p| (p.name.as_str(), p.default)).collect();
        assert_eq!(names, [("Home Inkjet", true), ("Office Laser", false)]);
        assert!(parse_printers("").is_empty());
    }

    #[test]
    fn file_and_printer_reach_powershell_as_data_never_as_script() {
        // Quotes PowerShell would end a single-quoted string at, ASCII and typographic.
        let hostile = "Office \"Laser\" 'B' ‘x’ ‚y‛; Remove-Item C:\\ $(calc)";
        let (script, env) = print_invocation(r"C:\Bob's ‘dir’\job.pdf", hostile);
        assert_eq!(script, PRINT_TO_SCRIPT);
        assert!(!script.contains("Bob") && !script.contains("Laser"));
        assert_eq!(
            env,
            [(FILE_VAR, r"C:\Bob's ‘dir’\job.pdf".to_string()), (PRINTER_VAR, "\"Office Laser 'B' ‘x’ ‚y‛; Remove-Item C:\\ $(calc)\"".to_string())]
        );
        assert!(PRINT_TO_SCRIPT.contains(&format!("$env:{FILE_VAR}")) && PRINT_TO_SCRIPT.contains(&format!("$env:{PRINTER_VAR}")));
        assert!(PRINT_SCRIPT.contains(&format!("$env:{FILE_VAR}")));
        for blank in ["", "  "] {
            assert_eq!(print_invocation("job.pdf", blank), (PRINT_SCRIPT, vec![(FILE_VAR, "job.pdf".to_string())]));
        }
    }

    #[test]
    fn the_sweep_removes_job_files_older_than_an_hour_only() {
        let dir = std::env::temp_dir().join(format!("wordcraft-print-sweep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (old, new) = (dir.join("old.pdf"), dir.join("new.pdf"));
        std::fs::write(&old, b"a").unwrap();
        std::fs::write(&new, b"b").unwrap();
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(two_hours_ago).unwrap();
        sweep(&dir);
        assert!(!old.exists());
        assert!(new.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A command that outlives its limit is stopped, and one that finishes in time is read.
    #[cfg(unix)]
    #[test]
    fn powershell_gets_a_deadline() {
        let started = std::time::Instant::now();
        let slow = output_within(std::process::Command::new("sleep").arg("30"), std::time::Duration::from_millis(200));
        assert_eq!(slow.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let quick = output_within(std::process::Command::new("echo").arg("ok"), std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(String::from_utf8_lossy(&quick).trim(), "ok");
    }

    #[test]
    fn job_files_hold_the_pdf_for_the_viewer_and_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("wordcraft-print-test-{}", std::process::id()));
        let a = job_file(&dir, b"%PDF-1.7 a").unwrap();
        let b = job_file(&dir, b"%PDF-1.7 b").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"%PDF-1.7 a");
        assert_eq!(std::fs::read(&b).unwrap(), b"%PDF-1.7 b");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_result_says_the_viewer_decides_and_copies_are_not_applied() {
        assert!(!handed_over(1).contains("copy"));
        assert!(handed_over(3).ends_with("(one copy, not 3)"));
    }
}
