//! `cargo xtask word-compare`: the page-by-page comparison harness against Word.
//!
//! Writes the synthetic probe documents ([`probes`]), has Word export each to PDF (macOS, Word
//! for Mac through AppleScript, only with `--word`), exports the same documents with WordCraft,
//! and compares the two PDFs line by line: page count, line breaks, page breaks and baseline
//! offsets ([`lines`]). Everything goes to `plan/word/compare/` (gitignored: files produced by
//! Word are never committed). Word's PDFs are kept, named by a hash of the document, so later
//! runs need Word only for probes that changed.
//!
//! Word for Mac asks once for access to the output folder (Grant File Access); grant it in
//! Word, or pass `--out` with a folder Word may already use.

mod lines;
mod package;
mod probes;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const USAGE: &str = "usage: cargo xtask word-compare [--word] [--only TEXT] [--out DIR] [--wordcraft BIN] [--list]\n       cargo xtask word-compare --pair WORD.pdf WORDCRAFT.pdf";

/// Longest wait for Word to export one probe.
const WORD_TIMEOUT: Duration = Duration::from_secs(90);

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    if let Some(i) = args.iter().position(|a| *a == "--pair") {
        // Any two PDFs of one document (a real file exported by Word and by WordCraft).
        let (Some(a), Some(b)) = (args.get(i + 1), args.get(i + 2)) else { return Err(USAGE.into()) };
        let (r, rp) = lines::lines(read(Path::new(a))?)?;
        let (t, tp) = lines::lines(read(Path::new(b))?)?;
        let c = lines::compare(&r, rp, &t, tp);
        println!("{}", serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?);
        return Ok(());
    }
    let value = |flag: &str| {
        args.iter().position(|a| *a == flag).map(|i| args.get(i + 1).copied().ok_or(format!("{flag} needs a value\n{USAGE}"))).transpose()
    };
    let only = value("--only")?;
    let out = value("--out")?.map_or_else(|| root.join("plan/word/compare"), PathBuf::from);
    let wordcraft = value("--wordcraft")?.map(PathBuf::from);
    let use_word = args.contains(&"--word");
    let known = ["--word", "--only", "--out", "--wordcraft", "--list"];
    if let Some(a) =
        args.iter().enumerate().find(|(i, a)| !known.contains(a) && !(*i > 0 && ["--only", "--out", "--wordcraft"].contains(&args[i - 1])))
    {
        return Err(format!("unknown argument `{}`\n{USAGE}", a.1));
    }
    let probes: Vec<probes::Probe> = probes::all().into_iter().filter(|p| only.is_none_or(|o| p.name.contains(o))).collect();
    if args.contains(&"--list") {
        for p in &probes {
            println!("{:<22} {}", p.name, p.about);
        }
        return Ok(());
    }
    if probes.is_empty() {
        return Err("no probe matches --only".into());
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let cli = match wordcraft {
        Some(b) => b,
        None => build_cli(root)?,
    };
    let mut rows = Vec::new();
    for p in &probes {
        let bytes = package::docx(&p.spec.parts());
        let hash = format!("{:016x}", fnv(&bytes));
        let docx = out.join(format!("{}.docx", p.name));
        std::fs::write(&docx, &bytes).map_err(|e| format!("{}: {e}", docx.display()))?;
        let word_pdf = out.join(format!("{}.{}.word.pdf", p.name, &hash[..10]));
        if !word_pdf.exists() && use_word {
            eprint!("Word: {} … ", p.name);
            match word_export(&docx, &word_pdf) {
                Ok(()) => eprintln!("ok"),
                Err(e) => eprintln!("{e}"),
            }
        }
        let wc_pdf = out.join(format!("{}.wordcraft.pdf", p.name));
        let _ = std::fs::remove_file(&wc_pdf);
        let o = Command::new(&cli).arg("convert").arg(&docx).arg(&wc_pdf).output().map_err(|e| format!("{}: {e}", cli.display()))?;
        let mut row = json!({"probe": p.name, "about": p.about});
        if !o.status.success() {
            row["status"] = json!("wordcraft failed");
            row["error"] = json!(String::from_utf8_lossy(&o.stderr).trim());
        } else if !word_pdf.exists() {
            row["status"] = json!("no Word PDF");
        } else {
            let (r, rp) = lines::lines(read(&word_pdf)?)?;
            let (t, tp) = lines::lines(read(&wc_pdf)?)?;
            let c = lines::compare(&r, rp, &t, tp);
            if let (Value::Object(row), Value::Object(c)) = (&mut row, c) {
                row.extend(c);
            }
        }
        rows.push(row);
    }
    let report = json!({"tolerance_pt": lines::TOL, "probes": rows});
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let md = markdown(&rows);
    std::fs::write(out.join("report.md"), &md).map_err(|e| e.to_string())?;
    print!("{md}");
    println!("\n{}", out.join("report.md").display());
    Ok(())
}

fn read(p: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Builds the CLI in release mode and returns its path.
fn build_cli(root: &Path) -> Result<PathBuf, String> {
    let ok = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "-q", "-p", "wordcraft-cli"])
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if !ok {
        return Err("cargo build -p wordcraft-cli failed".into());
    }
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let bin = target.join("release").join(if cfg!(windows) { "wordcraft-cli.exe" } else { "wordcraft-cli" });
    if bin.exists() { Ok(bin) } else { Err(format!("{} not found after the build", bin.display())) }
}

/// Exports `docx` to `pdf` with Word for Mac (read-only open, PDF save, close without saving).
fn word_export(docx: &Path, pdf: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("Word export needs macOS (Word for Mac through AppleScript)".into());
    }
    const SCRIPT: &str = r#"on run argv
	set inPath to item 1 of argv
	set outPath to item 2 of argv
	with timeout of 80 seconds
		tell application "Microsoft Word"
			set display alerts to none
			open (POSIX file inPath) with read only
			set d to active document
			save as d file name outPath file format format PDF
			close d saving no
		end tell
	end timeout
end run"#;
    let tmp = pdf.with_extension("partial.pdf");
    let _ = std::fs::remove_file(&tmp);
    let mut child = Command::new("osascript")
        .arg("-e")
        .arg(SCRIPT)
        .arg(docx)
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("osascript: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                let mut err = String::new();
                if let Some(mut s) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut s, &mut err);
                }
                return Err(format!("Word failed: {} (a dialog waiting in Word, such as Grant File Access, blocks it)", err.trim()));
            }
            break;
        }
        if start.elapsed() > WORD_TIMEOUT {
            let _ = child.kill();
            return Err("Word did not answer (a dialog may be waiting in Word)".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    std::fs::rename(&tmp, pdf).map_err(|e| format!("Word wrote no PDF: {e}"))
}

fn markdown(rows: &[Value]) -> String {
    let mut s = String::from(
        "| Probe | Status | Pages W/WC | Lines W/WC | Same breaks % | dy median / max | dx max | First difference |\n|---|---|---|---|---|---|---|---|\n",
    );
    let pair = |v: &Value| v.as_array().map(|a| a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("/")).unwrap_or_default();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for r in rows {
        let status = r["status"].as_str().unwrap_or("");
        *counts.entry(status.to_string()).or_default() += 1;
        let first = match &r["first_difference"] {
            Value::Object(f) => {
                let kind = f.get("kind").and_then(Value::as_str).unwrap_or("");
                let text = |k: &str| f.get(k).and_then(Value::as_str).map(|t| t.chars().take(40).collect::<String>()).unwrap_or_default();
                match kind {
                    "line break" => format!(
                        "p{} line break: Word “{}” / WordCraft “{}”",
                        f.get("page").map_or(String::new(), Value::to_string),
                        text("word"),
                        text("wordcraft")
                    ),
                    "page break" => format!(
                        "“{}” on Word p{} / WordCraft p{}",
                        text("word"),
                        f.get("word_page").map_or(String::new(), Value::to_string),
                        f.get("wordcraft_page").map_or(String::new(), Value::to_string)
                    ),
                    _ => format!(
                        "p{} {kind} dy {} dx {}: “{}”",
                        f.get("page").map_or(String::new(), Value::to_string),
                        f.get("dy").map_or(String::new(), Value::to_string),
                        f.get("dx").map_or(String::new(), Value::to_string),
                        text("word")
                    ),
                }
            }
            _ => r["error"].as_str().unwrap_or("").chars().take(60).collect(),
        };
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} / {} | {} | {} |\n",
            r["probe"].as_str().unwrap_or(""),
            status,
            pair(&r["pages"]),
            pair(&r["lines"]),
            r["same_break_pct"],
            r["dy_median"],
            r["dy_max"],
            r["dx_max"],
            first.replace('|', "\\|")
        ));
    }
    s.push_str(&format!("\n{} probes: {}\n", rows.len(), counts.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", ")));
    s
}

/// FNV-1a, 64 bit: names Word's PDF after the document it was made from.
fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_probe_is_a_zip_with_a_document_part() {
        for p in probes::all() {
            let z = package::docx(&p.spec.parts());
            assert!(z.starts_with(b"PK\x03\x04"), "{}", p.name);
            assert!(z.windows(17).any(|w| w == b"word/document.xml"), "{}", p.name);
        }
    }
}
