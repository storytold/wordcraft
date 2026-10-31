//! `cargo xtask field-oracle`: field codes in, a Word-versus-WordCraft table out.
//!
//! Each code is inserted as a field in a temporary document of real Word (COM through
//! `powershell.exe`; from WSL it needs `WSL_INTEROP`) and of `wordcraft-cli`, both are updated, and
//! the two result texts are compared. Word's answers are never stored: Word is asked each time.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PS_SCRIPT: &str = include_str!("field_oracle.ps1");
const WORD_TIMEOUT: Duration = Duration::from_secs(120);

pub const USAGE: &str = "\
usage: cargo xtask field-oracle <codes.txt | --code CODE>... [--out table.md] [--cli wordcraft-cli]
  codes.txt   one field code per line (the text between the braces); blank and # lines are skipped
  --code      add one code from the command line (repeatable)
  --out       also write the table to this file
  --cli       a built wordcraft-cli (default: cargo run -p wordcraft-cli)
  --no-word   skip Word and print WordCraft's results only
";

/// The codes in `text`: one per line, blank and `#` lines skipped.
pub fn parse_codes(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect()
}

fn to_hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(h: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

/// Word's per-code results by index, from `S|E<TAB>index<TAB>hex` records.
pub fn parse_word_output(out: &str, n: usize) -> Result<Vec<Result<String, String>>, String> {
    let mut res = vec![None; n];
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.splitn(3, '\t');
        let kind = parts.next().unwrap_or("");
        let i = parts.next().ok_or_else(|| format!("unexpected output from Word: {line}"))?;
        let hex = parts.next().ok_or_else(|| format!("unexpected output from Word: {line}"))?;
        let i: usize = i.parse().map_err(|_| format!("unexpected output from Word: {line}"))?;
        let text = from_hex(hex).ok_or_else(|| format!("bad result from Word: {line}"))?;
        let result = match kind {
            "S" => Ok(text),
            "E" => Err(text),
            _ => return Err(format!("unexpected output from Word: {line}")),
        };
        *res.get_mut(i).ok_or_else(|| format!("Word returned index {i} for {n} codes"))? = Some(result);
    }
    res.into_iter().enumerate().map(|(i, r)| r.ok_or_else(|| format!("Word returned no result for code {i}"))).collect()
}

fn powershell() -> Command {
    let mut c = if cfg!(windows) {
        Command::new("powershell.exe")
    } else {
        let wsl = Path::new("/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut c = Command::new(if wsl.exists() { wsl.to_str().unwrap_or("powershell.exe") } else { "powershell.exe" });
        if std::env::var_os("WSL_INTEROP").is_none() {
            c.env("WSL_INTEROP", "/run/WSL/2_interop");
        }
        c
    };
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]);
    c
}

/// A path PowerShell understands: `wslpath -w` under WSL, the path itself on Windows.
fn windows_path(p: &Path) -> Result<String, String> {
    if cfg!(windows) {
        return Ok(p.to_string_lossy().into_owned());
    }
    let o = Command::new("wslpath").arg("-w").arg(p).output().map_err(|e| format!("wslpath: {e}"))?;
    if !o.status.success() {
        return Err(format!("wslpath -w {}: {}", p.display(), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn word_results(codes: &[String]) -> Result<Vec<Result<String, String>>, String> {
    // A script file, not `-Command -`: PowerShell reads a piped multi-line script line by line and runs nothing.
    let dir = std::env::temp_dir().join(format!("field-oracle-word-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let ps1 = dir.join("field_oracle.ps1");
    let run = std::fs::write(&ps1, format!("$codeHex = '{}'\n{PS_SCRIPT}", to_hex(&codes.join("\n"))))
        .map_err(|e| format!("{}: {e}", ps1.display()))
        .and_then(|()| run_script(&ps1, codes.len()));
    let _ = std::fs::remove_dir_all(&dir);
    run
}

fn run_script(ps1: &Path, n: usize) -> Result<Vec<Result<String, String>>, String> {
    let mut child = powershell()
        .arg(windows_path(ps1)?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("powershell.exe: {e}"))?;
    // Read both pipes off-thread so a full pipe never stalls Word's script while we poll the clock.
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    let out = std::thread::spawn(move || read_all(out.as_mut()));
    let err = std::thread::spawn(move || read_all(err.as_mut()));
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| format!("powershell.exe: {e}"))? {
            break s;
        }
        if start.elapsed() > WORD_TIMEOUT {
            // The script's `finally` can't run now; Word is left to the caller, never killed by name.
            let _ = child.kill();
            return Err(format!("Word did not finish within {}s (a Word it started may still be running)", WORD_TIMEOUT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if !status.success() {
        return Err(format!("Word run failed ({status}): {}", err.trim()));
    }
    parse_word_output(&out, n)
}

fn read_all(r: Option<&mut impl std::io::Read>) -> String {
    let mut s = String::new();
    if let Some(r) = r {
        let mut b = Vec::new();
        let _ = r.read_to_end(&mut b);
        s = String::from_utf8_lossy(&b).into_owned();
    }
    s
}

fn cli_command(cli: Option<&Path>) -> Command {
    match cli {
        Some(p) => Command::new(p),
        None => {
            let mut c = crate::cargo();
            c.args(["run", "-q", "-p", "wordcraft-cli", "--"]);
            c
        }
    }
}

/// WordCraft's results: one paragraph per code, every field updated, saved as text.
fn wordcraft_results(codes: &[String], cli: Option<&Path>) -> Result<Vec<String>, String> {
    let dir = std::env::temp_dir().join(format!("field-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let out = dir.join("out.txt");
    let mut commands = Vec::new();
    for c in codes {
        commands.push(serde_json::json!({"command": "insert.field", "params": {"instr": c}}));
        commands.push(serde_json::json!("text.newParagraph"));
    }
    commands.push(serde_json::json!({"command": "references.updateFields", "params": {"all": true}}));
    let script = serde_json::json!({"template": "blank", "commands": commands, "save": [out.to_string_lossy()]});
    let run = (|| {
        let mut child = cli_command(cli)
            .args(["run", "--script", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("wordcraft-cli: {e}"))?;
        child.stdin.take().ok_or("wordcraft-cli: no stdin")?.write_all(script.to_string().as_bytes()).map_err(|e| format!("wordcraft-cli: {e}"))?;
        let o = child.wait_with_output().map_err(|e| format!("wordcraft-cli: {e}"))?;
        if !o.status.success() {
            return Err(format!("wordcraft-cli failed ({}): {}", o.status, String::from_utf8_lossy(&o.stderr).trim()));
        }
        std::fs::read_to_string(&out).map_err(|e| format!("{}: {e}", out.display()))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    split_paragraphs(&run?, codes.len())
}

/// The `n` lines of the saved text, one per code: the empty last paragraph only adds the final line break.
pub fn split_paragraphs(text: &str, n: usize) -> Result<Vec<String>, String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != n {
        return Err(format!("WordCraft saved {} lines for {n} codes (a result with a line break?)", lines.len()));
    }
    Ok(lines.iter().map(|l| l.trim_end().to_string()).collect())
}

fn cell(s: &str) -> String {
    format!("`{}`", s.replace('|', "\\|").replace('`', "'"))
}

/// Fields WordCraft fills in at layout time: the text export has no result for them (render to see them).
fn is_layout_field(code: &str) -> bool {
    let name = code.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    ["PAGE", "NUMPAGES", "SECTIONPAGES", "SECTION"].contains(&name.as_str())
}

/// The comparison table. `word` is `None` when Word was skipped.
pub fn table(codes: &[String], word: Option<&[Result<String, String>]>, wc: &[String]) -> String {
    let mut s = String::from("| Code | Word | WordCraft | |\n|---|---|---|---|\n");
    let (mut same, mut diff, mut errors) = (0, 0, 0);
    for (i, code) in codes.iter().enumerate() {
        let w = word.and_then(|w| w.get(i));
        let verdict = match w {
            None => "",
            Some(Err(_)) => {
                errors += 1;
                "n/a (Word error)"
            }
            Some(Ok(_)) if is_layout_field(code) && wc.get(i).is_none_or(String::is_empty) => "n/a (layout field)",
            Some(Ok(w)) if Some(w) == wc.get(i) => {
                same += 1;
                "match"
            }
            Some(Ok(_)) => {
                diff += 1;
                "**DIFF**"
            }
        };
        let word_cell = w.map_or(String::new(), |result| match result {
            Ok(value) => cell(value),
            Err(error) => format!("Word error: {}", cell(error)),
        });
        s += &format!("| {} | {} | {} | {verdict} |\n", cell(code), word_cell, cell(wc.get(i).map_or("", String::as_str)));
    }
    if word.is_some() {
        if errors == 0 {
            s += &format!("\n{} codes: {same} match, {diff} differ.\n", codes.len());
        } else {
            s += &format!("\n{} codes: {same} match, {diff} differ, {errors} Word errors.\n", codes.len());
        }
    }
    s
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let (mut codes, mut out, mut cli, mut no_word) = (Vec::new(), None::<PathBuf>, None::<PathBuf>, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--code" => codes.push(it.next().ok_or("--code needs a value")?.to_string()),
            "--out" => out = Some(it.next().ok_or("--out needs a value")?.into()),
            "--cli" => cli = Some(it.next().ok_or("--cli needs a value")?.into()),
            "--no-word" => no_word = true,
            f if f.starts_with("--") => return Err(format!("unknown option `{f}`\n\n{USAGE}")),
            f => {
                let p = Path::new(f);
                let p = if p.exists() { p.to_path_buf() } else { root.join(f) };
                codes.extend(parse_codes(&std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?));
            }
        }
    }
    if codes.is_empty() {
        return Err(format!("no field codes given\n\n{USAGE}"));
    }
    if let Some(c) = codes.iter().find(|c| c.contains(['\n', '\r'])) {
        return Err(format!("a field code cannot contain a line break: {c:?}"));
    }
    let wc = wordcraft_results(&codes, cli.as_deref())?;
    let word = if no_word { None } else { Some(word_results(&codes)?) };
    let md = table(&codes, word.as_deref(), &wc);
    print!("{md}");
    if let Some(o) = out {
        std::fs::write(&o, &md).map_err(|e| format!("{}: {e}", o.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_skip_comments_and_blanks() {
        assert_eq!(parse_codes("# c\n\n= 1+1 \n  PAGE \\* roman\n"), ["= 1+1", "PAGE \\* roman"]);
    }

    #[test]
    fn hex_round_trips_unicode() {
        assert_eq!(from_hex(&to_hex("a→b \\* Upper")).as_deref(), Some("a→b \\* Upper"));
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex("abc"), None);
    }

    #[test]
    fn word_output_maps_by_index_and_rejects_gaps() {
        let out = format!("S\t1\t{}\nS\t0\t\n", to_hex("six"));
        assert_eq!(parse_word_output(&out, 2).unwrap(), [Ok(String::new()), Ok("six".into())]);
        assert!(parse_word_output("S\t0\t\n", 2).is_err());
        assert!(parse_word_output("S\t5\t\n", 2).is_err());
    }

    #[test]
    fn word_output_keeps_com_errors_separate_from_values() {
        let out = format!("E\t0\t{}\nS\t1\t{}\n", to_hex("field failed"), to_hex("six"));
        assert_eq!(parse_word_output(&out, 2).unwrap(), [Err("field failed".into()), Ok("six".into())]);
        let codes = vec!["bad field".into(), "NUMWORDS".into()];
        let table = table(&codes, Some(&[Err("field failed".into()), Ok("6".into())]), &["".into(), "6".into()]);
        assert!(table.contains("| `bad field` | Word error: `field failed` | `` | n/a (Word error) |"));
        assert!(table.contains("2 codes: 1 match, 0 differ, 1 Word errors."), "{table}");
    }

    #[test]
    fn table_marks_differences_and_escapes_pipes() {
        let codes = vec!["a".to_string(), "b|c".to_string()];
        let t = table(&codes, Some(&[Ok("1".into()), Ok("2".into())]), &["1".into(), "3".into()]);
        let layout = table(&["PAGE \\* roman".to_string()], Some(&[Ok("i".into())]), &["".into()]);
        assert!(layout.contains("n/a (layout field)") && layout.contains("0 match, 0 differ"));
        assert!(t.contains("| `a` | `1` | `1` | match |"));
        assert!(t.contains("| `b\\|c` | `2` | `3` | **DIFF** |"));
        assert!(t.contains("2 codes: 1 match, 1 differ."));
    }

    #[test]
    fn paragraphs_drop_the_trailing_empty_one() {
        assert_eq!(split_paragraphs("x\ny  \n", 2).unwrap(), ["x", "y"]);
        assert!(split_paragraphs("x\nembedded\ny\n", 2).is_err());
        assert!(split_paragraphs("x\n", 2).is_err());
    }
}
