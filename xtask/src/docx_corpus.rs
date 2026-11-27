//! `cargo xtask docx-corpus --dir <path> [--out <report.json>]`: an opt-in, reproducible
//! DOCX corpus runner.
//!
//! Every `.docx` under `--dir` is opened and inspected in a child `wordcraft-cli` process with a
//! bounded timeout, rendered to PNG (page 1, or every page with `--all-pages`), saved to a
//! temporary `.docx`, reopened and compared with its original plain text and page count. A
//! failure in one file is recorded and the run continues. The report is stable JSON plus a short
//! Markdown summary; renders live beside the report.
//!
//! The corpus itself is local only (see `corpus/docx/README.md`): `/corpus` is git-ignored and no
//! Word-produced or proprietary document may be committed.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

const SCHEMA: &str = "wordcraft.docx-corpus/v1";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_TIMEOUT_SECS: u64 = 3600;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_ALL_PAGES: u64 = 2_000;
const RENDER_SCALE: &str = "1.5";
/// How long to wait for a finished child's pipes to close.
const OUTPUT_GRACE: Duration = Duration::from_secs(10);
/// How long to wait for pipes after killing a timed-out child.
const KILL_GRACE: Duration = Duration::from_secs(2);

const USAGE: &str = "\
usage: cargo xtask docx-corpus --dir <path> [--out <report.json>] [options]

Run every .docx under DIR through open/inspect, render, save, reopen and text/page compare.
The corpus is local only (never committed); see corpus/docx/README.md.

options:
  --dir <path>       corpus directory (required)
  --out <path>       JSON report path (default: target/docx-corpus/report.json);
                     the Markdown summary is written beside it with an .md extension
  --all-pages        render every page, not only page 1 (capped at 2000 pages)
  --timeout <secs>   timeout for each child process (default: 60, max: 3600)
  --cli <path>       wordcraft-cli binary (default: build it and use the one beside this xtask binary)
  -h, --help         this help
";

pub fn run(args: &[&str]) -> Result<(), String> {
    if args.iter().any(|a| *a == "-h" || *a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let opts = Options::parse(args)?;
    let out = opts.out.clone();
    let cli = ProcessCli { program: resolve_cli(opts.cli.as_deref())? };
    let Some(report) = execute(&opts, &cli)? else {
        return Ok(());
    };
    write_outputs(&out, &report)
}

struct Options {
    dir: PathBuf,
    out: PathBuf,
    all_pages: bool,
    timeout: Duration,
    cli: Option<PathBuf>,
}

impl Options {
    fn parse(args: &[&str]) -> Result<Self, String> {
        let mut dir = None;
        let mut out = None;
        let mut all_pages = false;
        let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
        let mut cli = None;
        let mut it = args.iter().copied();
        while let Some(arg) = it.next() {
            match arg {
                "--dir" => dir = Some(PathBuf::from(it.next().ok_or("--dir requires a path")?)),
                "--out" => out = Some(PathBuf::from(it.next().ok_or("--out requires a path")?)),
                "--all-pages" => all_pages = true,
                "--timeout" => {
                    let value = it.next().ok_or("--timeout requires seconds")?;
                    timeout_secs = value.parse::<u64>().map_err(|_| format!("--timeout `{value}` is not a whole number of seconds"))?;
                    if timeout_secs == 0 || timeout_secs > MAX_TIMEOUT_SECS {
                        return Err(format!("--timeout must be 1..={MAX_TIMEOUT_SECS} seconds"));
                    }
                }
                "--cli" => cli = Some(PathBuf::from(it.next().ok_or("--cli requires a path")?)),
                other => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
            }
        }
        let dir = dir.ok_or_else(|| format!("--dir is required\n\n{USAGE}"))?;
        let out = out.unwrap_or_else(|| crate::root().join("target/docx-corpus/report.json"));
        Ok(Self { dir, out, all_pages, timeout: Duration::from_secs(timeout_secs), cli })
    }
}

/// One child process result.
struct CliOutput {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// The seam between the runner and `wordcraft-cli`; tests supply an in-process fake.
trait Cli {
    fn call(&self, args: &[OsString], timeout: Duration) -> Result<CliOutput, String>;
}

struct ProcessCli {
    program: PathBuf,
}

impl Cli for ProcessCli {
    fn call(&self, args: &[OsString], timeout: Duration) -> Result<CliOutput, String> {
        let mut cmd = Command::new(&self.program);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("spawn {}: {e}", self.program.display()))?;
        let stdout = child.stdout.take().map(Reader::spawn);
        let stderr = child.stderr.take().map(Reader::spawn);
        let start = Instant::now();
        let deadline = start.checked_add(timeout).unwrap_or(start);
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        break None;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("wait for {}: {e}", self.program.display()));
                }
            }
        };
        if timed_out || status.is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if timed_out {
            // A killed child normally closes its pipes at once. If a grandchild still holds them,
            // abandon the readers rather than letting one hostile file stall the run.
            if let Some(reader) = stdout {
                let _ = reader.take(KILL_GRACE, "stdout");
            }
            if let Some(reader) = stderr {
                let _ = reader.take(KILL_GRACE, "stderr");
            }
            return Err(format!("timed out after {}s", timeout.as_secs()));
        }
        let (out, out_too_big) = match stdout {
            Some(reader) => reader.take(OUTPUT_GRACE, "stdout")?,
            None => (Vec::new(), false),
        };
        let (err, err_too_big) = match stderr {
            Some(reader) => reader.take(OUTPUT_GRACE, "stderr")?,
            None => (Vec::new(), false),
        };
        if out_too_big || err_too_big {
            return Err(format!("output exceeds the {} MiB cap", MAX_OUTPUT_BYTES / (1024 * 1024)));
        }
        let (success, code) = match status {
            Some(status) => (status.success(), status.code()),
            None => (false, None),
        };
        Ok(CliOutput { success, code, stdout: out, stderr: err })
    }
}

type ReaderResult = io::Result<(Vec<u8>, bool)>;

/// A pipe reader thread whose result is collected with a deadline. Dropping the receiver lets a
/// still-blocked thread finish (or exit) without the runner waiting for it.
struct Reader {
    rx: mpsc::Receiver<ReaderResult>,
}

impl Reader {
    fn spawn<R: Read + Send + 'static>(reader: R) -> Self {
        let (tx, rx) = mpsc::channel();
        let _ = thread::spawn(move || {
            let _ = tx.send(read_capped(reader, MAX_OUTPUT_BYTES));
        });
        Self { rx }
    }

    fn take(self, grace: Duration, what: &str) -> Result<(Vec<u8>, bool), String> {
        match self.rx.recv_timeout(grace) {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(e)) => Err(format!("read child {what}: {e}")),
            Err(RecvTimeoutError::Timeout) => Err(format!("child {what} did not close after the process exited")),
            Err(RecvTimeoutError::Disconnected) => Err(format!("read child {what}: the reader stopped")),
        }
    }
}

/// Read at most `cap` bytes. Past the cap the pipe is closed, so a child producing unbounded
/// output fails fast instead of filling memory; the caller reports the cap as the stage error.
fn read_capped<R: Read>(mut reader: R, cap: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut out = Vec::with_capacity(cap.min(64 * 1024));
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(out.len());
        let take = n.min(room);
        if let Some(chunk) = buf.get(..take) {
            out.extend_from_slice(chunk);
        }
        if take < n {
            return Ok((out, true));
        }
    }
    Ok((out, false))
}

/// Cargo arguments that build `wordcraft-cli`. `--release` is a `build` option, not a cargo
/// global one, so it must follow the subcommand: `cargo build --release`, never
/// `cargo --release build`.
fn cli_build_args(release: bool) -> Vec<&'static str> {
    let mut args = vec!["build", "-p", "wordcraft-cli"];
    if release {
        args.push("--release");
    }
    args
}

fn resolve_cli(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        return Err(format!("--cli {} is not a file", path.display()));
    }
    let exe = std::env::current_exe().map_err(|e| format!("current exe: {e}"))?;
    let dir = exe.parent().ok_or_else(|| format!("{} has no parent directory", exe.display()))?;
    resolve_cli_beside(dir, build_cli)
}

/// Build `wordcraft-cli` beside the running xtask binary and return its path.
///
/// The build runs on every invocation: cargo is a no-op when the binary is current, and
/// returning an existing candidate would let a stale binary produce a stale report.
fn resolve_cli_beside(dir: &Path, build: impl FnOnce(&Path) -> Result<(), String>) -> Result<PathBuf, String> {
    eprintln!("docx-corpus: building wordcraft-cli ...");
    build(dir)?;
    let candidate = dir.join(cli_name());
    if candidate.is_file() { Ok(candidate) } else { Err(format!("{} was not built; pass --cli", candidate.display())) }
}

fn cli_name() -> &'static str {
    if cfg!(windows) { "wordcraft-cli.exe" } else { "wordcraft-cli" }
}

/// Build `wordcraft-cli` in the xtask binary's directory (cargo is a no-op when it is current).
fn build_cli(dir: &Path) -> Result<(), String> {
    let release = dir.file_name().is_some_and(|n| n == "release");
    let status = crate::cargo().args(cli_build_args(release)).status().map_err(|e| format!("build wordcraft-cli: {e}"))?;
    if !status.success() {
        return Err(format!("build wordcraft-cli failed ({status})"));
    }
    Ok(())
}

/// Run one corpus. `None` means the directory is missing, which is not an error.
fn execute(opts: &Options, cli: &dyn Cli) -> Result<Option<Report>, String> {
    let meta = match fs::metadata(&opts.dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            println!(
                "docx-corpus: no corpus directory at {} — populate one as described in corpus/docx/README.md; nothing to do.",
                opts.dir.display()
            );
            return Ok(None);
        }
        Err(e) => return Err(format!("{}: {e}", opts.dir.display())),
    };
    if !meta.is_dir() {
        return Err(format!("{} is not a directory", opts.dir.display()));
    }
    let files = collect_docx(&opts.dir)?;
    if files.is_empty() {
        println!("docx-corpus: no .docx files under {}; writing an empty report.", opts.dir.display());
    }
    let temp = TempDir::new()?;
    let out_parent = output_parent(&opts.out);
    let artifacts_root = artifacts_dir(&opts.out);
    let mut reports = Vec::with_capacity(files.len());
    for (index, path) in files.iter().enumerate() {
        let relative = relative_name(&opts.dir, path);
        eprintln!("docx-corpus: [{}/{}] {relative}", index + 1, files.len());
        let paths = ArtifactPaths { root: &artifacts_root, out_parent };
        reports.push(run_file(cli, opts, &temp, paths, index, path, &relative));
    }
    Ok(Some(Report::new(opts.dir.display().to_string(), reports)))
}

fn collect_docx(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let file_type = entry.file_type().map_err(|e| format!("{}: {e}", entry.path().display()))?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                stack.push((path, depth + 1));
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("docx")) {
                found.push(path);
                if found.len() > MAX_FILES {
                    return Err(format!("more than {MAX_FILES} .docx files under {}", root.display()));
                }
            }
        }
    }
    found.sort();
    Ok(found)
}

/// A per-run temporary directory, removed on drop (best effort).
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Result<Self, String> {
        let base = std::env::temp_dir();
        for n in 0..100u32 {
            let path = base.join(format!("wordcraft-docx-corpus-{}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Err(format!("{}: could not create a temporary directory", base.display()))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Open,
    Render,
    Save,
    Reopen,
    TextMismatch,
    PageCountDelta,
}

impl FailureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Render => "render",
            Self::Save => "save",
            Self::Reopen => "reopen",
            Self::TextMismatch => "text_mismatch",
            Self::PageCountDelta => "page_count_delta",
        }
    }
}

#[derive(Serialize, Clone)]
struct Failure {
    kind: &'static str,
    message: String,
}

#[derive(Serialize)]
struct FileReport {
    file: String,
    bytes: u64,
    status: String,
    open_ms: u64,
    pages: u64,
    words: u64,
    render_ms: u64,
    round_trip_ms: u64,
    round_trip_pages: Option<u64>,
    page_delta: Option<i64>,
    text_match: Option<bool>,
    rendered: Vec<String>,
    failures: Vec<Failure>,
    error: Option<String>,
}

impl FileReport {
    fn new(file: String, bytes: u64) -> Self {
        Self {
            file,
            bytes,
            status: "ok".into(),
            open_ms: 0,
            pages: 0,
            words: 0,
            render_ms: 0,
            round_trip_ms: 0,
            round_trip_pages: None,
            page_delta: None,
            text_match: None,
            rendered: Vec::new(),
            failures: Vec::new(),
            error: None,
        }
    }

    fn fail(&mut self, kind: FailureKind, message: impl Into<String>) {
        self.failures.push(Failure { kind: kind.as_str(), message: message.into() });
    }

    fn finish(mut self) -> Self {
        self.status = if self.failures.is_empty() { "ok" } else { "failed" }.into();
        self.error = if self.failures.is_empty() {
            None
        } else {
            Some(self.failures.iter().map(|f| format!("{}: {}", f.kind, f.message)).collect::<Vec<_>>().join("; "))
        };
        self
    }
}

#[derive(Serialize)]
struct Totals {
    files: usize,
    ok: usize,
    failed: usize,
    bytes: u64,
    pages: u64,
    words: u64,
}

#[derive(Default, Serialize)]
struct FailureCounts {
    open: u64,
    render: u64,
    save: u64,
    reopen: u64,
    text_mismatch: u64,
    page_count_delta: u64,
}

impl FailureCounts {
    fn add(&mut self, kind: &str) {
        match kind {
            "open" => self.open = self.open.saturating_add(1),
            "render" => self.render = self.render.saturating_add(1),
            "save" => self.save = self.save.saturating_add(1),
            "reopen" => self.reopen = self.reopen.saturating_add(1),
            "text_mismatch" => self.text_mismatch = self.text_mismatch.saturating_add(1),
            "page_count_delta" => self.page_count_delta = self.page_count_delta.saturating_add(1),
            _ => {}
        }
    }
}

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    corpus: String,
    files: Vec<FileReport>,
    totals: Totals,
    failures: FailureCounts,
}

impl Report {
    fn new(corpus: String, mut files: Vec<FileReport>) -> Self {
        files.sort_by(|a, b| a.file.cmp(&b.file));
        let mut failures = FailureCounts::default();
        let mut totals = Totals { files: files.len(), ok: 0, failed: 0, bytes: 0, pages: 0, words: 0 };
        for file in &files {
            if file.failures.is_empty() {
                totals.ok = totals.ok.saturating_add(1);
            } else {
                totals.failed = totals.failed.saturating_add(1);
            }
            totals.bytes = totals.bytes.saturating_add(file.bytes);
            totals.pages = totals.pages.saturating_add(file.pages);
            totals.words = totals.words.saturating_add(file.words);
            for failure in &file.failures {
                failures.add(failure.kind);
            }
        }
        Self { schema: SCHEMA, corpus, files, totals, failures }
    }
}

struct ArtifactPaths<'a> {
    root: &'a Path,
    out_parent: &'a Path,
}

fn run_file(cli: &dyn Cli, opts: &Options, temp: &TempDir, paths: ArtifactPaths<'_>, index: usize, path: &Path, relative: &str) -> FileReport {
    let bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut report = FileReport::new(relative.to_string(), bytes);
    if bytes > MAX_FILE_BYTES {
        report.fail(FailureKind::Open, format!("file is {bytes} bytes, above the {} MiB cap", MAX_FILE_BYTES / (1024 * 1024)));
        return report.finish();
    }

    let open_start = Instant::now();
    let info = match read_info(cli, path, opts.timeout) {
        Ok(info) => info,
        Err(e) => {
            report.open_ms = millis(open_start);
            report.fail(FailureKind::Open, e);
            return report.finish();
        }
    };
    report.pages = info.pages;
    report.words = info.words;
    let original_text = match read_text(cli, path, opts.timeout) {
        Ok(text) => text,
        Err(e) => {
            report.open_ms = millis(open_start);
            report.fail(FailureKind::Open, format!("plain text: {e}"));
            return report.finish();
        }
    };
    report.open_ms = millis(open_start);

    let render_start = Instant::now();
    let page_count = if opts.all_pages { info.pages.max(1) } else { 1 };
    if opts.all_pages && page_count > MAX_ALL_PAGES {
        report.fail(FailureKind::Render, format!("{page_count} pages exceeds the {MAX_ALL_PAGES}-page render cap"));
    } else {
        let slug = format!("{index:04}-{}", slug(relative));
        let dir = paths.root.join(&slug);
        if let Err(e) = fs::create_dir_all(&dir) {
            report.fail(FailureKind::Render, format!("{}: {e}", dir.display()));
        } else {
            for page in 1..=page_count {
                let target = dir.join(format!("page-{page:04}.png"));
                let args = vec![
                    OsString::from("render"),
                    path.as_os_str().to_os_string(),
                    target.as_os_str().to_os_string(),
                    OsString::from("--page"),
                    OsString::from(page.to_string()),
                    OsString::from("--scale"),
                    OsString::from(RENDER_SCALE),
                ];
                if let Err(e) = run_ok(cli, &args, opts.timeout) {
                    report.fail(FailureKind::Render, format!("page {page}: {e}"));
                    break;
                }
                report.rendered.push(relative_name(paths.out_parent, &target));
            }
        }
    }
    report.render_ms = millis(render_start);

    let round_trip_start = Instant::now();
    let copy = temp.path.join(format!("{index:04}-{}.docx", slug(relative)));
    let save_args = vec![OsString::from("convert"), path.as_os_str().to_os_string(), copy.as_os_str().to_os_string()];
    let saved = match run_ok(cli, &save_args, opts.timeout) {
        Ok(_) => true,
        Err(e) => {
            report.fail(FailureKind::Save, scrub_temp(&e, &temp.path));
            false
        }
    };
    if saved {
        match read_info(cli, &copy, opts.timeout) {
            Ok(info) => {
                report.round_trip_pages = Some(info.pages);
                let delta = i64::try_from(info.pages).unwrap_or(i64::MAX).saturating_sub(i64::try_from(report.pages).unwrap_or(i64::MAX));
                report.page_delta = Some(delta);
                if info.pages != report.pages {
                    report.fail(FailureKind::PageCountDelta, format!("page count {} → {} ({delta:+})", report.pages, info.pages));
                }
                match read_text(cli, &copy, opts.timeout) {
                    Ok(text) => {
                        let matches = text == original_text;
                        report.text_match = Some(matches);
                        if !matches {
                            report.fail(FailureKind::TextMismatch, text_mismatch_message(&original_text, &text));
                        }
                    }
                    Err(e) => report.fail(FailureKind::Reopen, scrub_temp(&format!("plain text: {e}"), &temp.path)),
                }
            }
            Err(e) => report.fail(FailureKind::Reopen, scrub_temp(&e, &temp.path)),
        }
    }
    report.round_trip_ms = millis(round_trip_start);
    let _ = fs::remove_file(&copy);
    report.finish()
}

struct Info {
    pages: u64,
    words: u64,
}

fn read_info(cli: &dyn Cli, path: &Path, timeout: Duration) -> Result<Info, String> {
    let args = vec![OsString::from("info"), path.as_os_str().to_os_string()];
    let output = run_ok(cli, &args, timeout)?;
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("file.info returned invalid JSON: {e}"))?;
    let pages = value.get("pages").and_then(Value::as_u64).ok_or("file.info has no integer pages")?;
    let words = value.get("words").and_then(Value::as_u64).ok_or("file.info has no integer words")?;
    Ok(Info { pages, words })
}

fn read_text(cli: &dyn Cli, path: &Path, timeout: Duration) -> Result<Vec<u8>, String> {
    let args = vec![OsString::from("text"), path.as_os_str().to_os_string()];
    Ok(run_ok(cli, &args, timeout)?.stdout)
}

fn run_ok(cli: &dyn Cli, args: &[OsString], timeout: Duration) -> Result<CliOutput, String> {
    let output = cli.call(args, timeout)?;
    if output.success {
        Ok(output)
    } else {
        let code = match output.code {
            Some(code) => format!("code {code}"),
            None => "no exit code".to_string(),
        };
        let stderr = stderr_excerpt(&output.stderr);
        if stderr.is_empty() { Err(format!("wordcraft-cli exited with {code}")) } else { Err(format!("wordcraft-cli exited with {code}: {stderr}")) }
    }
}

fn stderr_excerpt(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim().replace('\n', " ");
    if text.chars().count() > 400 {
        let mut short: String = text.chars().take(400).collect();
        short.push('…');
        short
    } else {
        text
    }
}

/// Keep the report reproducible: the temporary copy's random name is not part of the result.
fn scrub_temp(message: &str, temp: &Path) -> String {
    let path = temp.to_string_lossy();
    if path.is_empty() { message.to_string() } else { message.replace(path.as_ref(), "<tmp>") }
}

fn text_mismatch_message(original: &[u8], reopened: &[u8]) -> String {
    let at = original.iter().zip(reopened.iter()).position(|(a, b)| a != b);
    match at {
        Some(at) => format!("text differs at byte {at} (original {} bytes, reopened {} bytes)", original.len(), reopened.len()),
        None => format!("text differs in length (original {} bytes, reopened {} bytes)", original.len(), reopened.len()),
    }
}

fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn relative_name(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

fn slug(relative: &str) -> String {
    relative.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect()
}

fn output_parent(out: &Path) -> &Path {
    out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new(""))
}

fn artifacts_dir(out: &Path) -> PathBuf {
    let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "report".into());
    output_parent(out).join(format!("{stem}-artifacts"))
}

fn write_outputs(out: &Path, report: &Report) -> Result<(), String> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report).map_err(|e| format!("report JSON: {e}"))?;
    fs::write(out, format!("{json}\n")).map_err(|e| format!("{}: {e}", out.display()))?;
    let summary = out.with_extension("md");
    fs::write(&summary, summary_markdown(report)).map_err(|e| format!("{}: {e}", summary.display()))?;
    println!("docx-corpus: wrote {} and {}", out.display(), summary.display());
    Ok(())
}

fn summary_markdown(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("# DOCX corpus report\n\n");
    out.push_str(&format!("- Corpus: `{}`\n", report.corpus.replace('`', "'")));
    out.push_str(&format!("- Files: {} ({} ok, {} failed)\n", report.totals.files, report.totals.ok, report.totals.failed));
    out.push_str(&format!("- Pages: {} · Words: {} · Bytes: {}\n", report.totals.pages, report.totals.words, report.totals.bytes));
    out.push_str(&format!(
        "- Failures: open {} · render {} · save {} · reopen {} · text mismatch {} · page-count delta {}\n\n",
        report.failures.open,
        report.failures.render,
        report.failures.save,
        report.failures.reopen,
        report.failures.text_mismatch,
        report.failures.page_count_delta
    ));
    if report.files.is_empty() {
        out.push_str("No `.docx` files were found.\n");
        return out;
    }
    out.push_str("| File | Status | Pages | Words | Open ms | Render ms | Round-trip ms | Error |\n");
    out.push_str("|---|---|---:|---:|---:|---:|---:|---|\n");
    for file in &report.files {
        let error = file.error.as_deref().map(escape_table).unwrap_or_default();
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
            escape_table(&file.file),
            file.status,
            file.pages,
            file.words,
            file.open_ms,
            file.render_ms,
            file.round_trip_ms,
            error
        ));
    }
    out
}

fn escape_table(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| {
            if c == '|' {
                '/'
            } else if c == '\n' || c == '\r' {
                ' '
            } else {
                c
            }
        })
        .take(240)
        .collect();
    if text.chars().count() > 240 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    struct FakeCli {
        calls: RefCell<Vec<Vec<String>>>,
        copy_pages: u64,
        copy_text: &'static str,
    }

    impl FakeCli {
        fn new() -> Self {
            Self { calls: RefCell::new(Vec::new()), copy_pages: 1, copy_text: "Hello\n" }
        }
    }

    impl Cli for FakeCli {
        fn call(&self, args: &[OsString], _timeout: Duration) -> Result<CliOutput, String> {
            let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
            self.calls.borrow_mut().push(args.clone());
            let command = args.first().map(String::as_str).unwrap_or("");
            let path = args.get(1).cloned().unwrap_or_default();
            if path.ends_with("bad.docx") {
                return Err("invalid zip: not a DOCX package".into());
            }
            // The runner's temporary copy is named `NNNN-<slug>.docx`.
            let copy = Path::new(&path).file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                name.as_bytes().get(4) == Some(&b'-') && name.as_bytes().get(..4).is_some_and(|n| n.iter().all(u8::is_ascii_digit))
            });
            match command {
                "info" => {
                    let pages = if copy { self.copy_pages } else { 1 };
                    Ok(ok(json!({"pages": pages, "words": 2}).to_string()))
                }
                "text" => Ok(ok(if copy { self.copy_text.to_string() } else { "Hello\n".to_string() })),
                "render" => {
                    if let Some(out) = args.get(2) {
                        if let Some(parent) = Path::new(out).parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(out, b"png");
                    }
                    Ok(ok(String::new()))
                }
                "convert" => {
                    if let Some(out) = args.get(2) {
                        let _ = fs::write(out, b"docx");
                    }
                    Ok(ok(String::new()))
                }
                other => Err(format!("unexpected command {other}")),
            }
        }
    }

    fn ok(stdout: String) -> CliOutput {
        CliOutput { success: true, code: Some(0), stdout: stdout.into_bytes(), stderr: Vec::new() }
    }

    fn temp_case(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("wordcraft-docx-corpus-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn opts(dir: PathBuf, out: PathBuf) -> Options {
        Options { dir, out, all_pages: false, timeout: Duration::from_secs(5), cli: None }
    }

    #[test]
    fn report_schema_and_order_are_stable() {
        let a = FileReport::new("b/second.docx".into(), 20).finish();
        let b = FileReport::new("a/first.docx".into(), 10).finish();
        let report = Report::new("corpus/docx".into(), vec![a, b]);
        assert_eq!(report.schema, "wordcraft.docx-corpus/v1");
        assert_eq!(report.files.first().map(|f| f.file.as_str()), Some("a/first.docx"));
        assert_eq!(report.files.get(1).map(|f| f.file.as_str()), Some("b/second.docx"));
        assert_eq!(report.totals.files, 2);
        assert_eq!(report.totals.bytes, 30);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["schema"], "wordcraft.docx-corpus/v1");
        for key in [
            "file",
            "bytes",
            "status",
            "open_ms",
            "pages",
            "words",
            "render_ms",
            "round_trip_ms",
            "round_trip_pages",
            "page_delta",
            "text_match",
            "rendered",
            "failures",
            "error",
        ] {
            assert!(value["files"][0].get(key).is_some(), "missing report field {key}");
        }
        let once = serde_json::to_string(&report).unwrap();
        let twice = serde_json::to_string(&report).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn aggregation_counts_statuses_and_failure_kinds() {
        let mut bad = FileReport::new("bad.docx".into(), 7);
        bad.pages = 2;
        bad.words = 3;
        bad.fail(FailureKind::Open, "broken");
        bad.fail(FailureKind::TextMismatch, "different");
        let bad = bad.finish();
        let mut good = FileReport::new("good.docx".into(), 5);
        good.pages = 1;
        good.words = 4;
        let good = good.finish();
        let report = Report::new("corpus".into(), vec![good, bad]);
        assert_eq!(report.totals.files, 2);
        assert_eq!(report.totals.ok, 1);
        assert_eq!(report.totals.failed, 1);
        assert_eq!(report.totals.bytes, 12);
        assert_eq!(report.totals.pages, 3);
        assert_eq!(report.totals.words, 7);
        assert_eq!(report.failures.open, 1);
        assert_eq!(report.failures.text_mismatch, 1);
        assert_eq!(report.failures.render, 0);
        assert_eq!(report.failures.reopen, 0);
        assert_eq!(report.failures.save, 0);
        assert_eq!(report.failures.page_count_delta, 0);
    }

    #[test]
    fn cli_build_args_put_the_subcommand_before_flags() {
        // Regression: `--release` used to be appended first, producing the invalid
        // `cargo --release build -p wordcraft-cli` and failing a release xtask run.
        assert_eq!(cli_build_args(false), ["build", "-p", "wordcraft-cli"]);
        assert_eq!(cli_build_args(true), ["build", "-p", "wordcraft-cli", "--release"]);
        for release in [false, true] {
            assert_eq!(cli_build_args(release).first().copied(), Some("build"));
        }
    }

    #[test]
    fn resolve_cli_builds_every_run_even_when_the_binary_exists() {
        // Regression: an existing binary used to be returned without rebuilding, so a report
        // could describe stale code.
        let dir = temp_case("resolve-cli-rebuild");
        let candidate = dir.join(cli_name());
        fs::write(&candidate, b"stale").unwrap();
        let builds = RefCell::new(Vec::new());
        let resolved = resolve_cli_beside(&dir, |dir| {
            builds.borrow_mut().push(dir.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(resolved, candidate);
        assert_eq!(builds.borrow().as_slice(), std::slice::from_ref(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_cli_uses_an_explicit_binary_without_building() {
        let dir = temp_case("resolve-cli-explicit");
        let binary = dir.join("my-cli");
        fs::write(&binary, b"cli").unwrap();
        assert_eq!(resolve_cli(Some(&binary)).unwrap(), binary);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_cli_rejects_a_missing_explicit_binary() {
        let dir = temp_case("resolve-cli-missing");
        let err = resolve_cli(Some(&dir.join("absent"))).unwrap_err();
        assert!(err.contains("is not a file"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_cli_reports_build_problems() {
        let dir = temp_case("resolve-cli-build-errors");
        let err = resolve_cli_beside(&dir, |_| Err("build wordcraft-cli failed (exit status: 1)".into())).unwrap_err();
        assert!(err.contains("build wordcraft-cli failed"), "{err}");
        let err = resolve_cli_beside(&dir, |_| Ok(())).unwrap_err();
        assert!(err.contains("pass --cli"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("wordcraft-docx-corpus-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cli = FakeCli::new();
        let out = dir.join("report.json");
        let report = execute(&opts(dir, out), &cli).unwrap();
        assert!(report.is_none());
        assert!(cli.calls.borrow().is_empty());
    }

    #[test]
    fn corrupt_file_is_classified_and_the_run_continues() {
        let dir = temp_case("corrupt");
        fs::write(dir.join("bad.docx"), b"not a zip").unwrap();
        fs::write(dir.join("good.docx"), b"PK\x03\x04synthetic").unwrap();
        let out = dir.join("report.json");
        let cli = FakeCli::new();
        let report = execute(&opts(dir.clone(), out), &cli).unwrap().unwrap();
        assert_eq!(report.files.len(), 2);
        let bad = &report.files[0];
        assert_eq!(bad.file, "bad.docx");
        assert_eq!(bad.status, "failed");
        assert_eq!(bad.failures.first().map(|f| f.kind), Some("open"));
        assert!(bad.error.as_deref().is_some_and(|e| e.contains("invalid zip")));
        let good = &report.files[1];
        assert_eq!(good.file, "good.docx");
        assert_eq!(good.status, "ok");
        assert_eq!(good.pages, 1);
        assert_eq!(good.words, 2);
        assert_eq!(good.text_match, Some(true));
        assert_eq!(report.totals.ok, 1);
        assert_eq!(report.totals.failed, 1);
        assert_eq!(report.failures.open, 1);
        let calls = cli.calls.borrow();
        assert!(calls.iter().any(|c| c.get(1).is_some_and(|p| p.ends_with("good.docx"))), "the run continued after the corrupt file");
        drop(calls);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_mismatch_and_page_delta_are_classified() {
        let dir = temp_case("mismatch");
        fs::write(dir.join("good.docx"), b"PK\x03\x04synthetic").unwrap();
        let out = dir.join("report.json");
        let cli = FakeCli { calls: RefCell::new(Vec::new()), copy_pages: 2, copy_text: "Bye\n" };
        let report = execute(&opts(dir.clone(), out), &cli).unwrap().unwrap();
        let file = &report.files[0];
        assert_eq!(file.status, "failed");
        assert_eq!(file.text_match, Some(false));
        assert_eq!(file.page_delta, Some(1));
        assert_eq!(file.round_trip_pages, Some(2));
        assert_eq!(report.failures.text_mismatch, 1);
        assert_eq!(report.failures.page_count_delta, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
