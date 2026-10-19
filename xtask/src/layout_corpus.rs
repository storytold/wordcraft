//! `cargo xtask layout-corpus --dir <corpus> [--out <dir>] [--private] [--dpi N]`:
//! compare WordCraft's PDF with a Word reference for every case in a local layout corpus.
//!
//! A case is a directory with `case.docx` and, optionally, `case.word.pdf` and
//! `case.expected.json`. The Word reference is exported by hand from Word and is never committed.
//! The command:
//!
//! 1. exports WordCraft's PDF (`wordcraft-cli convert case.docx case.wordcraft.pdf`);
//! 2. skips the case with `skipped: no Word reference` when `case.word.pdf` is absent;
//! 3. runs the existing `cargo xtask visual-diff` and reads its `result.json`;
//! 4. aggregates page-count deltas, same-break percentages, median |Δy|, SSIM and worst pages;
//! 5. writes `<out>/summary.json` and `<out>/summary.md`, worst cases first.
//!
//! `--private` passes `visual-diff --private` (numbers and line hashes, no diff maps) and keeps
//! case names out of the report: they become `case-<hash>` labels.
//!
//! Needs `wordcraft-cli` (an existing `target/{release,debug}` build, `--wordcraft-cli PATH`, or
//! a `cargo run` fallback) and poppler (`pdftoppm`, `pdftotext`) for `visual-diff`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::visual_diff;

const SCHEMA: &str = "wordcraft.layout-corpus/1";
const USAGE: &str = "\
usage: cargo xtask layout-corpus --dir <corpus> [--out <dir>] [--private] [--dpi N] [--wordcraft-cli PATH]

  --dir DIR            corpus folder with <case>/case.docx (or a single folder with case.docx)
  --out DIR            report and generated files (default: target/layout-corpus)
  --private            numbers and hashes only: no case names, line text or diff maps
  --dpi N              raster resolution for visual-diff (36..=300, default 100)
  --wordcraft-cli PATH use this wordcraft-cli binary instead of an existing build or cargo run
";

const DEFAULT_DPI: u32 = 100;
const MAX_CASES: usize = 1_000;
const MAX_RESULT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXPECTED_BYTES: u64 = 1024 * 1024;
const MAX_PDF_HEADER: u64 = 1024;
const MAX_REASON_CHARS: usize = 2_000;
const WORST_PAGES_PER_CASE: usize = 3;
const WORST_PAGES_TOTAL: usize = 10;

#[derive(Clone, Debug)]
struct Options {
    dir: PathBuf,
    out: PathBuf,
    private: bool,
    dpi: u32,
    cli: Option<PathBuf>,
    root: PathBuf,
}

/// The `wordcraft-cli` the harness will run, resolved once so the report names the exact binary.
#[derive(Clone, Debug)]
enum Cli {
    Binary(PathBuf),
    CargoRun,
}

impl Cli {
    fn resolve(opts: &Options) -> Self {
        if let Some(path) = &opts.cli {
            return Cli::Binary(path.clone());
        }
        if let Some(path) = std::env::var_os("WORDCRAFT_CLI") {
            return Cli::Binary(PathBuf::from(path));
        }
        let exe = format!("wordcraft-cli{}", std::env::consts::EXE_SUFFIX);
        let candidates = [opts.root.join("target/release").join(&exe), opts.root.join("target/debug").join(&exe)];
        match candidates.iter().find(|path| path.is_file()) {
            Some(path) => Cli::Binary(path.clone()),
            None => Cli::CargoRun,
        }
    }

    fn describe(&self, opts: &Options) -> String {
        match self {
            Cli::Binary(path) => path.display().to_string(),
            Cli::CargoRun => format!("cargo run -p wordcraft-cli (in {})", opts.root.display()),
        }
    }

    fn command(&self, opts: &Options) -> Command {
        match self {
            Cli::Binary(path) => Command::new(path),
            Cli::CargoRun => {
                let mut cargo = crate::cargo();
                cargo.current_dir(&opts.root);
                cargo.args(["run", "--quiet", "-p", "wordcraft-cli", "--"]);
                cargo
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Case {
    name: String,
    docx: PathBuf,
    reference: PathBuf,
    expected: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Compared,
    Skipped,
    Failed,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Compared => "compared",
            Status::Skipped => "skipped",
            Status::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
struct Expected {
    page_count: Option<usize>,
    known_differences: Vec<String>,
}

#[derive(Clone, Debug)]
struct PageMetric {
    page: usize,
    ssim: Option<f64>,
    same_break_pct: Option<f64>,
    dy: Option<f64>,
    lines: Option<(usize, usize)>,
    mismatch: f64,
}

#[derive(Clone, Debug)]
struct Metrics {
    reference_pages: usize,
    wordcraft_pages: usize,
    same_break_pct: Option<f64>,
    median_dy: Option<f64>,
    ssim: Option<f64>,
    page_starts_same: Option<usize>,
    mismatch: f64,
    worst_pages: Vec<PageMetric>,
}

#[derive(Clone, Debug)]
struct CaseRun {
    name: String,
    label: String,
    status: Status,
    reason: Option<String>,
    private_reason: Option<String>,
    expected: Option<Expected>,
    metrics: Option<Metrics>,
    pdf_rel: Option<String>,
    result_rel: Option<String>,
}

/// `cargo xtask layout-corpus …`.
pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    if args.iter().any(|a| *a == "-h" || *a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let opts = parse_args(root, args)?;
    let cases = discover(&opts.dir)?;
    std::fs::create_dir_all(&opts.out).map_err(|e| format!("create {}: {e}", opts.out.display()))?;
    let cli = Cli::resolve(&opts);
    eprintln!("layout-corpus: wordcraft-cli: {}", cli.describe(&opts));

    let mut records = Vec::with_capacity(cases.len());
    let mut failures = Vec::new();
    for case in &cases {
        let export = |docx: &Path, pdf: &Path| export_pdf(&cli, &opts, docx, pdf);
        let run = evaluate_case(case, &opts, &export, &diff_pdf);
        let label = if opts.private { run.label.clone() } else { run.name.clone() };
        match run.status {
            Status::Compared => {}
            Status::Skipped => eprintln!("layout-corpus: {label}: skipped ({})", run.reason.as_deref().unwrap_or("no Word reference")),
            Status::Failed => {
                let reason = if opts.private {
                    run.private_reason.clone().unwrap_or_else(|| "failed".into())
                } else {
                    run.reason.clone().unwrap_or_else(|| "failed".into())
                };
                eprintln!("layout-corpus: {label}: failed: {reason}");
                failures.push(label);
            }
        }
        records.push(case_json(&run, &opts, case));
    }

    let report = build_report(records, &opts, &cli);
    write_report(&opts.out, &report)?;
    let aggregate = &report["aggregate"];
    println!(
        "layout-corpus: {} case(s): {} compared, {} skipped, {} failed; report {}",
        aggregate["cases"].as_u64().unwrap_or(0),
        aggregate["compared"].as_u64().unwrap_or(0),
        aggregate["skipped"].as_u64().unwrap_or(0),
        aggregate["failed"].as_u64().unwrap_or(0),
        opts.out.display()
    );
    if failures.is_empty() { Ok(()) } else { Err(format!("{} case(s) failed: {}", failures.len(), failures.join(", "))) }
}

fn parse_args(root: &Path, args: &[&str]) -> Result<Options, String> {
    let mut dir = None;
    let mut out = None;
    let mut private = false;
    let mut dpi = DEFAULT_DPI;
    let mut cli = None;
    let mut i = 0;
    while let Some(arg) = args.get(i).copied() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (arg, None),
        };
        match name {
            "--dir" => dir = Some(PathBuf::from(option_value(args, &mut i, inline, name)?)),
            "--out" => out = Some(PathBuf::from(option_value(args, &mut i, inline, name)?)),
            "--wordcraft-cli" => cli = Some(PathBuf::from(option_value(args, &mut i, inline, name)?)),
            "--private" => {
                if inline.is_some() {
                    return Err(format!("{name} takes no value\n{USAGE}"));
                }
                private = true;
            }
            "--dpi" => {
                let value = option_value(args, &mut i, inline, name)?;
                dpi = value.parse::<u32>().ok().filter(|d| (36..=300).contains(d)).ok_or("--dpi takes 36..=300")?;
            }
            other => return Err(format!("unknown option `{other}`\n{USAGE}")),
        }
        i += 1;
    }
    let dir = dir.ok_or_else(|| format!("--dir is required\n{USAGE}"))?;
    let dir = absolute(&dir)?;
    if !dir.is_dir() {
        return Err(format!("--dir {}: not a directory", dir.display()));
    }
    let out = out.unwrap_or_else(|| root.join("target/layout-corpus"));
    let out = absolute(&out)?;
    if out == dir {
        return Err("--out must differ from --dir (reports and generated PDFs would mix with the corpus)".into());
    }
    Ok(Options { dir, out, private, dpi, cli, root: root.to_path_buf() })
}

fn option_value(args: &[&str], i: &mut usize, inline: Option<&str>, name: &str) -> Result<String, String> {
    if let Some(value) = inline {
        return if value.is_empty() { Err(format!("{name} needs a value")) } else { Ok(value.to_string()) };
    }
    match args.get(*i + 1) {
        Some(value) if !value.starts_with("--") => {
            *i += 1;
            Ok((*value).to_string())
        }
        _ => Err(format!("{name} needs a value")),
    }
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir().map(|cwd| cwd.join(path)).map_err(|e| format!("current directory: {e}"))
    }
}

fn discover(dir: &Path) -> Result<Vec<Case>, String> {
    let dir = std::fs::canonicalize(dir).map_err(|e| format!("--dir {}: {e}", dir.display()))?;
    let single = dir.join("case.docx");
    if single.is_file() {
        return Ok(vec![case_at(&dir, case_name(&dir))]);
    }
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    let mut cases = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("read {}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.starts_with('_') || !path.is_dir() {
            continue;
        }
        if path.join("case.docx").is_file() {
            if cases.len() >= MAX_CASES {
                return Err(format!("more than {MAX_CASES} cases in {}; split the corpus", dir.display()));
            }
            cases.push(case_at(&path, name));
        }
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    if cases.is_empty() {
        return Err(format!("no case.docx found in {} (expected <case>/case.docx or case.docx)", dir.display()));
    }
    Ok(cases)
}

fn case_at(dir: &Path, name: String) -> Case {
    Case { name, docx: dir.join("case.docx"), reference: dir.join("case.word.pdf"), expected: dir.join("case.expected.json") }
}

fn case_name(dir: &Path) -> String {
    dir.file_name().and_then(|n| n.to_str()).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| "case".into())
}

fn evaluate_case<E, D>(case: &Case, opts: &Options, export: &E, diff: &D) -> CaseRun
where
    E: Fn(&Path, &Path) -> Result<(), String>,
    D: Fn(&Path, &Path, &Path, bool, u32) -> Result<Value, String>,
{
    let label = if opts.private { format!("case-{}", visual_diff::hash(&case.name)) } else { case.name.clone() };
    if !case.reference.is_file() {
        // A missing reference always skips, even if the optional expectation is malformed.
        let expected = load_expected(&case.expected).ok().flatten();
        return CaseRun {
            name: case.name.clone(),
            label,
            status: Status::Skipped,
            reason: Some("no Word reference".into()),
            private_reason: Some("no Word reference".into()),
            expected,
            metrics: None,
            pdf_rel: None,
            result_rel: None,
        };
    }
    let expected = match load_expected(&case.expected) {
        Ok(expected) => expected,
        Err(e) => {
            return failed_run(case, label, format!("case.expected.json: {e}"), "invalid case.expected.json", None);
        }
    };
    if let Err(e) = validate_pdf(&case.reference) {
        return failed_run(case, label, format!("Word reference is not a PDF: {e}"), "invalid Word reference", expected);
    }
    let case_out = opts.out.join(&label);
    if let Err(e) = std::fs::create_dir_all(&case_out) {
        return failed_run(case, label, format!("create {}: {e}", case_out.display()), "output folder failed", expected);
    }
    let pdf = case_out.join("case.wordcraft.pdf");
    if let Err(e) = export(&case.docx, &pdf) {
        if opts.private {
            let _ = std::fs::remove_file(&pdf);
        }
        return failed_run(case, label, format!("WordCraft export failed: {e}"), "WordCraft export failed", expected);
    }
    if let Err(e) = validate_pdf(&pdf) {
        if opts.private {
            let _ = std::fs::remove_file(&pdf);
        }
        return failed_run(case, label, format!("WordCraft output is not a PDF: {e}"), "WordCraft output was not a PDF", expected);
    }
    let diff_out = case_out.join("visual-diff");
    let result = diff(&case.reference, &pdf, &diff_out, opts.private, opts.dpi);
    if opts.private {
        // The report keeps numbers and hashes only; the intermediate WordCraft PDF has the same
        // text as the confidential source, so it does not outlive the comparison.
        let _ = std::fs::remove_file(&pdf);
    }
    let result = match result {
        Ok(result) => result,
        Err(e) => return failed_run(case, label, format!("visual-diff failed: {e}"), "visual-diff failed", expected),
    };
    let metrics = match metrics_from_result(&result) {
        Ok(metrics) => metrics,
        Err(e) => return failed_run(case, label, format!("visual-diff result.json: {e}"), "bad visual-diff result", expected),
    };
    CaseRun {
        name: case.name.clone(),
        label,
        status: Status::Compared,
        reason: None,
        private_reason: None,
        expected,
        metrics: Some(metrics),
        pdf_rel: Some(relative(&pdf, &opts.out)),
        result_rel: Some(relative(&diff_out.join("result.json"), &opts.out)),
    }
}

fn failed_run(case: &Case, label: String, reason: String, private_reason: &str, expected: Option<Expected>) -> CaseRun {
    CaseRun {
        name: case.name.clone(),
        label,
        status: Status::Failed,
        reason: Some(clip(&reason, MAX_REASON_CHARS)),
        private_reason: Some(private_reason.to_string()),
        expected,
        metrics: None,
        pdf_rel: None,
        result_rel: None,
    }
}

fn load_expected(path: &Path) -> Result<Option<Expected>, String> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err("not a file".into());
    }
    let value = read_json(path, MAX_EXPECTED_BYTES)?;
    let object = value.as_object().ok_or("must be a JSON object")?;
    let page_count = match object.get("page_count").or_else(|| object.get("pages")) {
        None => None,
        Some(value) => {
            let n = value.as_u64().ok_or("page_count must be a positive integer")?;
            let n = usize::try_from(n).map_err(|_| "page_count is too large")?;
            if n == 0 {
                return Err("page_count must be greater than zero".into());
            }
            Some(n)
        }
    };
    let mut known_differences = Vec::new();
    if let Some(value) = object.get("known_differences") {
        let items = value.as_array().ok_or("known_differences must be an array")?;
        for item in items.iter().take(100) {
            if let Some(label) = expected_label(item) {
                known_differences.push(clip(&label, 500));
            }
        }
    }
    Ok(Some(Expected { page_count, known_differences }))
}

fn expected_label(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Object(object) => {
            let page = object.get("page").and_then(Value::as_u64);
            let reason = object.get("reason").or_else(|| object.get("difference")).or_else(|| object.get("description")).and_then(Value::as_str);
            match (page, reason) {
                (Some(page), Some(reason)) => Some(format!("page {page}: {reason}")),
                (None, Some(reason)) => Some(reason.to_string()),
                (Some(page), None) => Some(format!("page {page}")),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

fn validate_pdf(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = file.take(MAX_PDF_HEADER);
    let mut head = [0u8; MAX_PDF_HEADER as usize];
    let n = reader.read(&mut head).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = head.get(..n).ok_or_else(|| format!("{}: short read", path.display()))?;
    if bytes.windows(5).any(|w| w == b"%PDF-") { Ok(()) } else { Err("missing %PDF- header".into()) }
}

fn export_pdf(cli: &Cli, opts: &Options, docx: &Path, pdf: &Path) -> Result<(), String> {
    if let Some(parent) = pdf.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let mut command = cli.command(opts);
    command.arg("convert").arg(docx).arg(pdf);
    let output = command.output().map_err(|e| format!("could not start wordcraft-cli: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        Err(clip(&format!("{}: {}", output.status, detail.trim()), MAX_REASON_CHARS))
    }
}

fn diff_pdf(reference: &Path, test: &Path, out: &Path, private: bool, dpi: u32) -> Result<Value, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let exe = std::env::current_exe().map_err(|e| format!("current xtask binary: {e}"))?;
    let mut command = Command::new(exe);
    command.arg("visual-diff").arg(reference).arg(test).arg(out);
    if private {
        command.arg("--private");
    }
    command.arg("--dpi").arg(dpi.to_string());
    let output = command.output().map_err(|e| format!("could not start visual-diff: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        return Err(clip(&format!("{}: {}", output.status, detail.trim()), MAX_REASON_CHARS));
    }
    read_result(&out.join("result.json"))
}

fn read_result(path: &Path) -> Result<Value, String> {
    let value = read_json(path, MAX_RESULT_BYTES)?;
    if value.get("summary").and_then(Value::as_object).is_none() {
        return Err("result.json has no summary object".into());
    }
    if value.get("pages").and_then(Value::as_array).is_none() {
        return Err("result.json has no pages array".into());
    }
    Ok(value)
}

fn read_json(path: &Path, max: u64) -> Result<Value, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > max {
        return Err(format!("{}: larger than {max} bytes", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: bad JSON: {e}", path.display()))
}

fn metrics_from_result(result: &Value) -> Result<Metrics, String> {
    let summary = result.get("summary").and_then(Value::as_object).ok_or("result.json has no summary object")?;
    let pages = result.get("pages").and_then(Value::as_array).ok_or("result.json has no pages array")?;
    let counts = summary.get("pages").and_then(Value::as_array).ok_or("summary.pages is missing")?;
    let reference_pages = count_at(counts, 0).ok_or("summary.pages[0] is missing")?;
    let wordcraft_pages = count_at(counts, 1).ok_or("summary.pages[1] is missing")?;
    let ssim = number(summary.get("ssim_mean"));
    let same_break_pct = number(summary.get("same_break_pct"));
    let page_starts_same = summary.get("page_starts_same").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    let mut page_metrics = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let Some(object) = page.as_object() else { continue };
        let page_number = object.get("page").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(index + 1);
        let page_ssim = number(object.get("ssim"));
        let page_same = number(object.get("same_break"));
        let page_dy = number(object.get("dy"));
        let lines = object.get("lines").and_then(Value::as_array).and_then(|values| {
            let reference = values.first()?.as_u64().and_then(|n| usize::try_from(n).ok())?;
            let test = values.get(1)?.as_u64().and_then(|n| usize::try_from(n).ok())?;
            Some((reference, test))
        });
        page_metrics.push(PageMetric {
            page: page_number,
            ssim: page_ssim,
            same_break_pct: page_same,
            dy: page_dy,
            lines,
            mismatch: page_mismatch(page_ssim, page_same, page_dy, lines),
        });
    }
    page_metrics.sort_by(|a, b| b.mismatch.total_cmp(&a.mismatch).then_with(|| a.page.cmp(&b.page)));
    let median_dy = median(page_metrics.iter().filter_map(|page| page.dy).collect());
    let page_delta = page_delta(reference_pages, wordcraft_pages);
    let mismatch = mismatch_score(ssim.unwrap_or(0.0), same_break_pct.unwrap_or(0.0), median_dy.unwrap_or(0.0), page_delta);
    Ok(Metrics {
        reference_pages,
        wordcraft_pages,
        same_break_pct,
        median_dy,
        ssim,
        page_starts_same,
        mismatch,
        worst_pages: page_metrics.into_iter().take(WORST_PAGES_PER_CASE).collect(),
    })
}

fn count_at(values: &[Value], index: usize) -> Option<usize> {
    values.get(index)?.as_u64().and_then(|n| usize::try_from(n).ok())
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn page_mismatch(ssim: Option<f64>, same_break: Option<f64>, dy: Option<f64>, lines: Option<(usize, usize)>) -> f64 {
    let ssim_term = match ssim {
        Some(ssim) => (1.0 - ssim.clamp(0.0, 1.0)) * 100.0,
        // visual-diff has no raster pair for a page on only one side.
        None => match lines {
            Some((reference, test)) if reference > 0 || test > 0 => 100.0,
            _ => 0.0,
        },
    };
    let break_term = same_break.map(|pct| (100.0 - pct.clamp(0.0, 100.0)).max(0.0)).unwrap_or(0.0);
    let dy_term = dy.map(|dy| dy.abs().min(72.0) * 2.0).unwrap_or(0.0);
    visual_diff::round(ssim_term + break_term + dy_term, 2)
}

fn mismatch_score(ssim: f64, same_break_pct: f64, median_dy: f64, page_delta: i64) -> f64 {
    let ssim_term = (1.0 - ssim.clamp(0.0, 1.0)) * 100.0;
    let break_term = (100.0 - same_break_pct.clamp(0.0, 100.0)).max(0.0);
    let dy_term = median_dy.abs().min(72.0) * 2.0;
    let page_term = (page_delta.unsigned_abs().min(100) as f64) * 20.0;
    visual_diff::round(ssim_term + break_term + dy_term + page_term, 2)
}

fn page_delta(reference_pages: usize, wordcraft_pages: usize) -> i64 {
    let reference = i64::try_from(reference_pages).unwrap_or(i64::MAX);
    let wordcraft = i64::try_from(wordcraft_pages).unwrap_or(i64::MAX);
    wordcraft.saturating_sub(reference)
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.retain(|value| value.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values.get(middle).copied()
    } else {
        let a = values.get(middle.saturating_sub(1))?;
        let b = values.get(middle)?;
        Some((a + b) / 2.0)
    }
}

fn mean(values: Vec<f64>) -> Option<f64> {
    let values: Vec<f64> = values.into_iter().filter(|value| value.is_finite()).collect();
    if values.is_empty() { None } else { Some(values.iter().sum::<f64>() / values.len() as f64) }
}

fn case_json(run: &CaseRun, opts: &Options, case: &Case) -> Value {
    let name = if opts.private { &run.label } else { &run.name };
    let mut record = json!({
        "name": name,
        "status": run.status.as_str(),
        "reason": if opts.private { run.private_reason.clone() } else { run.reason.clone() },
        "expected": expected_json(run.expected.as_ref(), run.metrics.as_ref(), opts.private),
        "pages": Value::Null,
        "same_break_pct": Value::Null,
        "median_dy": Value::Null,
        "ssim": Value::Null,
        "page_starts_same": Value::Null,
        "mismatch": Value::Null,
        "worst_pages": Vec::<Value>::new(),
    });
    if !opts.private {
        record["source"] = json!(relative(&case.docx, &opts.dir));
        record["reference"] = json!(relative(&case.reference, &opts.dir));
        if let Some(path) = &run.pdf_rel {
            record["wordcraft_pdf"] = json!(path);
        }
        if let Some(path) = &run.result_rel {
            record["result_json"] = json!(path);
        }
    }
    if let Some(metrics) = &run.metrics {
        record["pages"] = json!({
            "reference": metrics.reference_pages,
            "wordcraft": metrics.wordcraft_pages,
            "delta": page_delta(metrics.reference_pages, metrics.wordcraft_pages),
        });
        record["same_break_pct"] = json!(metrics.same_break_pct);
        record["median_dy"] = json!(metrics.median_dy);
        record["ssim"] = json!(metrics.ssim);
        record["page_starts_same"] = json!(metrics.page_starts_same);
        record["mismatch"] = json!(metrics.mismatch);
        record["worst_pages"] = Value::Array(metrics.worst_pages.iter().map(page_json).collect());
    }
    record
}

fn expected_json(expected: Option<&Expected>, metrics: Option<&Metrics>, private: bool) -> Value {
    let Some(expected) = expected else { return Value::Null };
    let page_count_matches = match (expected.page_count, metrics) {
        (Some(pages), Some(metrics)) => Some(metrics.reference_pages == pages && metrics.wordcraft_pages == pages),
        _ => None,
    };
    let known_differences: Vec<String> = if private {
        expected.known_differences.iter().map(|difference| visual_diff::hash(difference)).collect()
    } else {
        expected.known_differences.clone()
    };
    json!({
        "page_count": expected.page_count,
        "known_differences": known_differences,
        "page_count_matches": page_count_matches,
    })
}

fn page_json(page: &PageMetric) -> Value {
    json!({
        "page": page.page,
        "mismatch": page.mismatch,
        "ssim": page.ssim,
        "same_break_pct": page.same_break_pct,
        "dy": page.dy,
        "lines": page.lines.map(|(reference, test)| json!([reference, test])),
    })
}

fn build_report(mut records: Vec<Value>, opts: &Options, cli: &Cli) -> Value {
    // Worst first in the JSON too: failures, then compared cases by mismatch, then skipped ones.
    records.sort_by(|a, b| {
        let (sa, sb) = (status_order(a), status_order(b));
        sa.cmp(&sb).then_with(|| {
            if sa == 1 {
                let am = a.get("mismatch").and_then(Value::as_f64).unwrap_or(0.0);
                let bm = b.get("mismatch").and_then(Value::as_f64).unwrap_or(0.0);
                bm.total_cmp(&am)
            } else {
                let an = a.get("name").and_then(Value::as_str).unwrap_or("");
                let bn = b.get("name").and_then(Value::as_str).unwrap_or("");
                an.cmp(bn)
            }
        })
    });
    let aggregate = aggregate(&records);
    let mut ranked: Vec<(String, f64)> = records
        .iter()
        .filter(|record| record.get("status").and_then(Value::as_str) == Some(Status::Compared.as_str()))
        .filter_map(|record| {
            let name = record.get("name").and_then(Value::as_str)?.to_string();
            let mismatch = record.get("mismatch").and_then(Value::as_f64)?;
            Some((name, mismatch))
        })
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let ranked: Vec<Value> = ranked.into_iter().map(|(name, mismatch)| json!({"name": name, "mismatch": mismatch})).collect();
    json!({
        "schema": SCHEMA,
        "tool": "cargo xtask layout-corpus",
        "private": opts.private,
        "dpi": opts.dpi,
        "directory": if opts.private { Value::Null } else { json!(opts.dir.to_string_lossy()) },
        "wordcraft_cli": if opts.private { Value::Null } else { json!(cli.describe(opts)) },
        "cases": records,
        "aggregate": aggregate,
        "ranked": ranked,
    })
}

fn status_order(record: &Value) -> u8 {
    match record.get("status").and_then(Value::as_str) {
        Some("failed") => 0,
        Some("compared") => 1,
        Some("skipped") => 2,
        _ => 3,
    }
}

fn aggregate(records: &[Value]) -> Value {
    let mut compared = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut page_delta_abs = 0i64;
    let mut page_delta_signed = 0i64;
    let mut page_delta_max = 0i64;
    let mut same_breaks = Vec::new();
    let mut dys = Vec::new();
    let mut ssims = Vec::new();
    let mut mismatches = Vec::new();
    let mut worst_pages: Vec<Value> = Vec::new();
    for record in records {
        match record.get("status").and_then(Value::as_str) {
            Some("compared") => {
                compared += 1;
                if let Some(delta) = record.get("pages").and_then(|pages| pages.get("delta")).and_then(Value::as_i64) {
                    page_delta_abs = page_delta_abs.saturating_add(delta.saturating_abs());
                    page_delta_signed = page_delta_signed.saturating_add(delta);
                    page_delta_max = page_delta_max.max(delta.saturating_abs());
                }
                if let Some(value) = record.get("same_break_pct").and_then(Value::as_f64).filter(|value| value.is_finite()) {
                    same_breaks.push(value);
                }
                if let Some(value) = record.get("median_dy").and_then(Value::as_f64).filter(|value| value.is_finite()) {
                    dys.push(value);
                }
                if let Some(value) = record.get("ssim").and_then(Value::as_f64).filter(|value| value.is_finite()) {
                    ssims.push(value);
                }
                if let Some(value) = record.get("mismatch").and_then(Value::as_f64).filter(|value| value.is_finite()) {
                    mismatches.push(value);
                }
                if let (Some(name), Some(pages)) = (record.get("name").and_then(Value::as_str), record.get("worst_pages").and_then(Value::as_array)) {
                    for page in pages {
                        let mut page = page.clone();
                        if let Some(object) = page.as_object_mut() {
                            object.insert("case".into(), json!(name));
                        }
                        worst_pages.push(page);
                    }
                }
            }
            Some("skipped") => skipped += 1,
            Some("failed") => failed += 1,
            _ => {}
        }
    }
    worst_pages.sort_by(|a, b| {
        let am = a.get("mismatch").and_then(Value::as_f64).unwrap_or(0.0);
        let bm = b.get("mismatch").and_then(Value::as_f64).unwrap_or(0.0);
        bm.total_cmp(&am)
    });
    worst_pages.truncate(WORST_PAGES_TOTAL);
    let worst_page = worst_pages.first().cloned().unwrap_or(Value::Null);
    json!({
        "cases": records.len(),
        "compared": compared,
        "skipped": skipped,
        "failed": failed,
        "page_count_delta_abs": page_delta_abs,
        "page_count_delta_signed": page_delta_signed,
        "page_count_delta_max": page_delta_max,
        "same_break_pct": mean(same_breaks),
        "median_dy": median(dys),
        "ssim": mean(ssims),
        "worst_mismatch": mismatches.into_iter().reduce(f64::max),
        "worst_page": worst_page,
        "worst_pages": worst_pages,
    })
}

fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).to_string_lossy().into_owned()
}

fn write_report(out: &Path, report: &Value) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report).map_err(|e| format!("summary.json: {e}"))?;
    let json_path = out.join("summary.json");
    std::fs::write(&json_path, format!("{json}\n")).map_err(|e| format!("{}: {e}", json_path.display()))?;
    let markdown = markdown(report);
    let markdown_path = out.join("summary.md");
    std::fs::write(&markdown_path, markdown).map_err(|e| format!("{}: {e}", markdown_path.display()))?;
    println!("wrote {} and {}", json_path.display(), markdown_path.display());
    Ok(())
}

fn markdown(report: &Value) -> String {
    let private = report.get("private").and_then(Value::as_bool).unwrap_or(false);
    let aggregate = report.get("aggregate").unwrap_or(&Value::Null);
    let cases = report.get("cases").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let ranked = report.get("ranked").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let mut out = String::new();
    out.push_str("# Layout corpus comparison\n\n");
    out.push_str("Layout only, not visual identity: SSIM is a proxy for placement and ink, and font");
    out.push_str(" substitution makes small pixel differences expected.\n\n");
    if private {
        out.push_str("- Private report: numbers and hashes only.\n");
    } else if let Some(directory) = report.get("directory").and_then(Value::as_str) {
        out.push_str(&format!("- Corpus: `{}`\n", md_code(directory)));
    }
    out.push_str(&format!(
        "- Cases: {} ({} compared, {} skipped, {} failed)\n",
        int(aggregate, "cases"),
        int(aggregate, "compared"),
        int(aggregate, "skipped"),
        int(aggregate, "failed")
    ));
    out.push_str(&format!(
        "- Page-count delta: {} page(s) absolute (signed {:+}), worst {}\n",
        int(aggregate, "page_count_delta_abs"),
        signed(aggregate, "page_count_delta_signed"),
        int(aggregate, "page_count_delta_max")
    ));
    out.push_str(&format!("- Mean same-break percentage: {}\n", pct(aggregate.get("same_break_pct"))));
    out.push_str(&format!("- Median |Δy|: {}\n", points(aggregate.get("median_dy"))));
    out.push_str(&format!("- Mean SSIM: {}\n", decimal(aggregate.get("ssim"), 4)));
    out.push_str(&format!("- Raster resolution: {} dpi\n", report.get("dpi").and_then(Value::as_u64).unwrap_or(0)));
    out.push_str("\n## Ranked by layout mismatch (worst first)\n\n");
    if ranked.is_empty() {
        out.push_str("No cases were compared.\n");
    } else {
        out.push_str("| # | Case | Mismatch | Pages (Word → WordCraft) | Δ pages | Same breaks | Median \\|Δy\\| | SSIM | Worst page |\n");
        out.push_str("|---|---|---:|---:|---:|---:|---:|---:|---|\n");
        for (index, entry) in ranked.iter().enumerate() {
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("case");
            let Some(record) = cases.iter().find(|record| record.get("name").and_then(Value::as_str) == Some(name)) else { continue };
            let reference = record.get("pages").and_then(|pages| pages.get("reference")).and_then(Value::as_u64).unwrap_or(0);
            let wordcraft = record.get("pages").and_then(|pages| pages.get("wordcraft")).and_then(Value::as_u64).unwrap_or(0);
            let delta = record.get("pages").and_then(|pages| pages.get("delta")).and_then(Value::as_i64).unwrap_or(0);
            let worst = record
                .get("worst_pages")
                .and_then(Value::as_array)
                .and_then(|pages| pages.first())
                .and_then(|page| page.get("page"))
                .and_then(Value::as_u64);
            out.push_str(&format!(
                "| {} | `{}` | {} | {} → {} | {:+} | {} | {} | {} | {} |\n",
                index + 1,
                md_code(name),
                decimal(entry.get("mismatch"), 2),
                reference,
                wordcraft,
                delta,
                pct(record.get("same_break_pct")),
                points(record.get("median_dy")),
                decimal(record.get("ssim"), 4),
                worst.map_or_else(|| "—".to_string(), |page| format!("p. {page}"))
            ));
        }
    }

    let failed: Vec<&Value> = cases.iter().filter(|record| record.get("status").and_then(Value::as_str) == Some("failed")).collect();
    if !failed.is_empty() {
        out.push_str("\n## Failed\n\n");
        for record in failed {
            out.push_str(&format!(
                "- `{}`: {}\n",
                md_code(record.get("name").and_then(Value::as_str).unwrap_or("case")),
                md_text(record.get("reason").and_then(Value::as_str).unwrap_or("failed"))
            ));
        }
    }
    let skipped: Vec<&Value> = cases.iter().filter(|record| record.get("status").and_then(Value::as_str) == Some("skipped")).collect();
    if !skipped.is_empty() {
        out.push_str("\n## Skipped\n\n");
        for record in skipped {
            out.push_str(&format!(
                "- `{}`: {}\n",
                md_code(record.get("name").and_then(Value::as_str).unwrap_or("case")),
                md_text(record.get("reason").and_then(Value::as_str).unwrap_or("no Word reference"))
            ));
        }
    }

    let mut compared: Vec<&Value> = cases.iter().filter(|record| record.get("status").and_then(Value::as_str) == Some("compared")).collect();
    compared.sort_by(|a, b| {
        let am = a.get("mismatch").and_then(Value::as_f64).unwrap_or(0.0);
        let bm = b.get("mismatch").and_then(Value::as_f64).unwrap_or(0.0);
        bm.total_cmp(&am)
    });
    if !compared.is_empty() {
        out.push_str("\n## Compared cases\n");
        for record in compared {
            out.push_str(&format!(
                "\n### `{}`\n\n- Mismatch: {}\n- Pages: {} (Word) → {} (WordCraft), Δ {:+}\n- Same line breaks: {}\n- Median |Δy|: {}\n- Mean SSIM: {}\n",
                md_code(record.get("name").and_then(Value::as_str).unwrap_or("case")),
                decimal(record.get("mismatch"), 2),
                record.get("pages").and_then(|pages| pages.get("reference")).and_then(Value::as_u64).unwrap_or(0),
                record.get("pages").and_then(|pages| pages.get("wordcraft")).and_then(Value::as_u64).unwrap_or(0),
                record.get("pages").and_then(|pages| pages.get("delta")).and_then(Value::as_i64).unwrap_or(0),
                pct(record.get("same_break_pct")),
                points(record.get("median_dy")),
                decimal(record.get("ssim"), 4)
            ));
            if let Some(pages) = record.get("worst_pages").and_then(Value::as_array).filter(|pages| !pages.is_empty()) {
                out.push_str("- Worst pages: ");
                for (index, page) in pages.iter().enumerate() {
                    if index > 0 {
                        out.push_str("; ");
                    }
                    out.push_str(&format!(
                        "p. {} (mismatch {}, SSIM {}, same breaks {})",
                        page.get("page").and_then(Value::as_u64).unwrap_or(0),
                        decimal(page.get("mismatch"), 2),
                        decimal(page.get("ssim"), 4),
                        pct(page.get("same_break_pct"))
                    ));
                }
                out.push('\n');
            }
            if let Some(expected) = record.get("expected").filter(|expected| !expected.is_null()) {
                if let Some(pages) = expected.get("page_count").and_then(Value::as_u64) {
                    let verdict = match expected.get("page_count_matches").and_then(Value::as_bool) {
                        Some(true) => "matches",
                        Some(false) => "does not match",
                        None => "not checked",
                    };
                    out.push_str(&format!("- Expected page count: {pages} ({verdict})\n"));
                }
                if let Some(differences) = expected.get("known_differences").and_then(Value::as_array).filter(|items| !items.is_empty()) {
                    let labels: Vec<String> = differences
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|difference| if private { format!("hash {}", md_code(difference)) } else { md_text(difference) })
                        .collect();
                    out.push_str(&format!("- Known differences: {}\n", labels.join("; ")));
                }
            }
        }
    }
    out
}

fn int(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn signed(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn decimal(value: Option<&Value>, places: usize) -> String {
    match value.and_then(Value::as_f64).filter(|value| value.is_finite()) {
        Some(value) => format!("{value:.places$}"),
        None => "—".into(),
    }
}

fn pct(value: Option<&Value>) -> String {
    match value.and_then(Value::as_f64).filter(|value| value.is_finite()) {
        Some(value) => format!("{value:.1}%"),
        None => "—".into(),
    }
}

fn points(value: Option<&Value>) -> String {
    match value.and_then(Value::as_f64).filter(|value| value.is_finite()) {
        Some(value) => format!("{value:.1} pt"),
        None => "—".into(),
    }
}

fn md_text(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '|' => "\\|".to_string(),
            '\n' | '\r' => " ".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

fn md_code(value: &str) -> String {
    md_text(value).replace('`', "'")
}

fn clip(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut clipped: String = value.chars().take(max).collect();
        clipped.push('…');
        clipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("wordcraft-layout-corpus-{}-{tag}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn options(dir: &Path, private: bool) -> Options {
        Options { dir: dir.to_path_buf(), out: dir.join("_report"), private, dpi: DEFAULT_DPI, cli: None, root: dir.to_path_buf() }
    }

    fn result(reference_pages: u64, wordcraft_pages: u64, ssim: f64, same: f64, dy: f64) -> Value {
        let pages: Vec<Value> = (1..=reference_pages.max(wordcraft_pages))
            .map(|page| {
                if page <= reference_pages && page <= wordcraft_pages {
                    json!({"page": page, "ssim": ssim, "same_break": same, "dy": dy, "lines": [5, 5]})
                } else {
                    json!({"page": page, "lines": [0, 5]})
                }
            })
            .collect();
        json!({
            "summary": {
                "pages": [reference_pages, wordcraft_pages],
                "ssim_mean": ssim,
                "same_break_pct": same,
                "page_starts_same": reference_pages,
                "lines": [10, 10],
            },
            "pages": pages,
            "dx_diffs": [],
            "break_diffs": [],
        })
    }

    fn record(name: &str, status: Status, metrics: Option<Metrics>) -> Value {
        let dir = PathBuf::from("corpus/layout").join(name);
        let case = case_at(&dir, name.to_string());
        let run = CaseRun {
            name: name.to_string(),
            label: name.to_string(),
            status,
            reason: if status == Status::Skipped { Some("no Word reference".into()) } else { None },
            private_reason: if status == Status::Skipped { Some("no Word reference".into()) } else { None },
            expected: None,
            metrics,
            pdf_rel: None,
            result_rel: None,
        };
        case_json(&run, &options(Path::new("corpus/layout"), false), &case)
    }

    #[test]
    fn aggregation_ranks_the_bad_case_first_and_keeps_the_schema() {
        let good = metrics_from_result(&result(2, 2, 0.98, 98.0, 1.0)).unwrap();
        let bad = metrics_from_result(&result(2, 4, 0.60, 40.0, 20.0)).unwrap();
        let records =
            vec![record("good", Status::Compared, Some(good)), record("bad", Status::Compared, Some(bad)), record("empty", Status::Skipped, None)];
        let report = build_report(records, &options(Path::new("corpus/layout"), false), &Cli::Binary(PathBuf::from("wordcraft-cli")));
        assert_eq!(report["schema"], SCHEMA);
        assert_eq!(report["wordcraft_cli"], "wordcraft-cli");
        assert_eq!(report["aggregate"]["cases"], 3);
        assert_eq!(report["aggregate"]["compared"], 2);
        assert_eq!(report["aggregate"]["skipped"], 1);
        assert_eq!(report["aggregate"]["failed"], 0);
        assert_eq!(report["aggregate"]["page_count_delta_abs"], 2);
        assert_eq!(report["ranked"][0]["name"], "bad");
        assert_eq!(report["ranked"][1]["name"], "good");
        assert!(report["ranked"][0]["mismatch"].as_f64().unwrap() > report["ranked"][1]["mismatch"].as_f64().unwrap());
        assert_eq!(report["cases"][0]["status"], "compared");
        assert_eq!(report["cases"][0]["name"], "bad");
        assert_eq!(report["cases"][2]["status"], "skipped");
        assert!(report["cases"][0].get("pages").is_some());
        assert!(report["cases"][0].get("worst_pages").is_some());
        let markdown = markdown(&report);
        assert!(markdown.find("`bad`").unwrap() < markdown.find("`good`").unwrap());
        assert!(markdown.contains("Ranked by layout mismatch"));
        assert!(markdown.contains("signed +2"));
    }

    fn table_columns(row: &str) -> usize {
        let mut pipes: usize = 0;
        let mut escaped = false;
        for ch in row.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '|' => pipes += 1,
                _ => {}
            }
        }
        pipes.saturating_sub(1)
    }

    #[test]
    fn markdown_ranked_table_has_one_cell_per_column() {
        let good = metrics_from_result(&result(2, 2, 0.98, 98.0, 1.0)).unwrap();
        let bad = metrics_from_result(&result(2, 4, 0.60, 40.0, 20.0)).unwrap();
        let records = vec![record("good", Status::Compared, Some(good)), record("bad", Status::Compared, Some(bad))];
        let report = build_report(records, &options(Path::new("corpus/layout"), false), &Cli::CargoRun);
        let markdown = markdown(&report);
        let mut rows = markdown.lines().skip_while(|line| !line.starts_with("| # | Case |"));
        let header = rows.next().unwrap();
        let delimiter = rows.next().unwrap();
        assert!(header.contains("Median \\|Δy\\|"));
        let columns = table_columns(header);
        assert_eq!(columns, 9);
        assert_eq!(table_columns(delimiter), columns);
        for row in rows.take_while(|line| line.starts_with('|')) {
            assert_eq!(table_columns(row), columns, "row: {row}");
        }
    }

    #[test]
    fn extra_pages_score_worse_than_identical_pages() {
        let metrics = metrics_from_result(&result(1, 2, 0.99, 100.0, 0.0)).unwrap();
        assert_eq!(metrics.reference_pages, 1);
        assert_eq!(metrics.wordcraft_pages, 2);
        assert_eq!(page_delta(metrics.reference_pages, metrics.wordcraft_pages), 1);
        assert_eq!(metrics.worst_pages.first().map(|page| page.page), Some(2));
        assert!(metrics.mismatch >= 20.0);
    }

    #[test]
    fn a_case_without_a_word_reference_is_skipped_before_export() {
        let dir = temp_dir("skip");
        let case_dir = dir.join("case-a");
        std::fs::create_dir_all(&case_dir).unwrap();
        std::fs::write(case_dir.join("case.docx"), b"not needed").unwrap();
        let cases = discover(&dir).unwrap();
        assert_eq!(cases.len(), 1);
        let opts = options(&dir, false);
        let exported = std::cell::Cell::new(false);
        let run = evaluate_case(
            &cases[0],
            &opts,
            &|_, _| {
                exported.set(true);
                Ok(())
            },
            &|_, _, _, _, _| Err("must not run".into()),
        );
        assert!(!exported.get());
        assert_eq!(run.status, Status::Skipped);
        assert_eq!(run.reason.as_deref(), Some("no Word reference"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_writes_a_report_for_missing_references() {
        let dir = temp_dir("run-skip");
        let case_dir = dir.join("case-a");
        std::fs::create_dir_all(&case_dir).unwrap();
        std::fs::write(case_dir.join("case.docx"), b"not needed").unwrap();
        let out = dir.join("out");
        let result = run(&dir, &["--dir", dir.to_str().unwrap(), "--out", out.to_str().unwrap()]);
        assert!(result.is_ok(), "{result:?}");
        let summary: Value = serde_json::from_slice(&std::fs::read(out.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["cases"][0]["status"], "skipped");
        assert_eq!(summary["cases"][0]["reason"], "no Word reference");
        assert!(summary["wordcraft_cli"].as_str().unwrap().contains("wordcraft-cli"));
        assert_eq!(summary["aggregate"]["skipped"], 1);
        let markdown = std::fs::read_to_string(out.join("summary.md")).unwrap();
        assert!(markdown.contains("no Word reference"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_comparison_removes_the_intermediate_pdf() {
        let dir = temp_dir("private-ok");
        let case_dir = dir.join("case-a");
        std::fs::create_dir_all(&case_dir).unwrap();
        std::fs::write(case_dir.join("case.docx"), b"not needed").unwrap();
        std::fs::write(case_dir.join("case.word.pdf"), b"%PDF-1.7\n").unwrap();
        let opts = options(&dir, true);
        let cases = discover(&dir).unwrap();
        let run = evaluate_case(
            &cases[0],
            &opts,
            &|_, pdf| {
                std::fs::write(pdf, b"%PDF-1.7\n").unwrap();
                Ok(())
            },
            &|_, _, _, _, _| Ok(result(1, 1, 0.99, 100.0, 0.0)),
        );
        assert_eq!(run.status, Status::Compared);
        assert!(run.metrics.is_some());
        assert!(run.label.starts_with("case-"));
        assert!(!opts.out.join(&run.label).join("case.wordcraft.pdf").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_reference_pdfs_fail_cleanly_and_private_reports_hide_names() {
        let dir = temp_dir("invalid-pdf");
        let case_dir = dir.join("secret-case");
        std::fs::create_dir_all(&case_dir).unwrap();
        std::fs::write(case_dir.join("case.docx"), b"not needed").unwrap();
        std::fs::write(case_dir.join("case.word.pdf"), b"this is not a PDF").unwrap();
        let out = dir.join("out");
        let result = run(&dir, &["--dir", dir.to_str().unwrap(), "--out", out.to_str().unwrap(), "--private"]);
        assert!(result.is_err());
        let summary: Value = serde_json::from_slice(&std::fs::read(out.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["private"], true);
        assert!(summary["wordcraft_cli"].is_null());
        assert_eq!(summary["cases"][0]["status"], "failed");
        assert_eq!(summary["cases"][0]["reason"], "invalid Word reference");
        assert!(summary["cases"][0].get("source").is_none());
        assert!(summary["cases"][0].get("reference").is_none());
        let name = summary["cases"][0]["name"].as_str().unwrap();
        assert!(name.starts_with("case-"));
        assert!(!name.contains("secret"));
        let markdown = std::fs::read_to_string(out.join("summary.md")).unwrap();
        assert!(!markdown.contains("secret-case"));
        assert!(markdown.contains("invalid Word reference"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_header_validation_accepts_a_pdf_and_rejects_text() {
        let dir = temp_dir("pdf-header");
        let pdf = dir.join("ok.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\n").unwrap();
        assert!(validate_pdf(&pdf).is_ok());
        let text = dir.join("bad.pdf");
        std::fs::write(&text, b"plain text").unwrap();
        assert!(validate_pdf(&text).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expected_file_keeps_page_count_and_known_differences() {
        let dir = temp_dir("expected");
        let path = dir.join("case.expected.json");
        std::fs::write(&path, br#"{"page_count": 2, "known_differences": ["footer differs", {"page": 3, "reason": "table splits"}]}"#).unwrap();
        let expected = load_expected(&path).unwrap().unwrap();
        assert_eq!(expected.page_count, Some(2));
        assert_eq!(expected.known_differences, vec!["footer differs".to_string(), "page 3: table splits".to_string()]);
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load_expected(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_visual_diff_results_are_rejected() {
        assert!(metrics_from_result(&json!({"pages": []})).is_err());
        assert!(metrics_from_result(&json!({"summary": {}})).is_err());
        let dir = temp_dir("bad-result");
        std::fs::write(dir.join("result.json"), b"{}").unwrap();
        assert!(read_result(&dir.join("result.json")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inconsistent_expected_page_counts_are_flagged() {
        let metrics = metrics_from_result(&result(3, 3, 0.99, 100.0, 0.0)).unwrap();
        let expected = Expected { page_count: Some(2), known_differences: vec![] };
        let value = expected_json(Some(&expected), Some(&metrics), false);
        assert_eq!(value["page_count"], 2);
        assert_eq!(value["page_count_matches"], false);
    }
}
