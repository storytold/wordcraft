//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `curl`, `tar`) are
//! invoked through `std::process::Command`.

mod assets;
mod ico;
mod layers;
mod parity;
mod stats;
mod version;
mod visual_diff;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  assets          check that every icon/image/font/asset is attributed in ATTRIBUTION.md
  layers          enforce the crate dependency layering (plan/architecture.md §3)
  wasm            cargo check --target wasm32-unknown-unknown for the wasm-safe crates (one call)
  test            workspace tests (cargo nextest when installed, else cargo test), the interface
                  audit with its missing-translation report, then doctests
  i18n-report     the interface audit: fails on text that skips the catalog, and prints the
                  strings each language has no translation for (never a failure)
  ci              fmt --check, clippy -D warnings, test, i18n-report, doctest, assets, layers, wasm
                  (stops at first failure)
  corpus [--download]
                  show where test corpora live; --download fetches PngSuite into corpus/pngsuite
  bench [--pages N] [--keys K]
                  large-document benchmark (#554): a synthetic N-page book (default 500) timed for
                  open, layout, keystrokes, rendering, save and PDF export; release build, nice 19
  stats [--exact] count tests and lines per crate (--exact: ask the test harness via `-- --list`)
  parity          recompute the Word feature-parity summary in docs/parity-checklist.md
  visual-diff <ref.pdf> <test.pdf> <outdir> [--private] [--dpi N]
                  compare two PDFs of a document page by page (SSIM, line breaks, offsets);
                  needs poppler (pdftoppm, pdftotext)
  ico <out.ico> <in.png>...
                  pack square PNGs (<= 256 px) into a Windows .ico (see packaging/icons.sh)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("assets") => assets::run(&root()),
        Some("layers") => cmd_layers(),
        Some("wasm") => cmd_wasm(),
        Some("test") => cmd_test().and_then(|()| cmd_i18n_report()).and_then(|()| cmd_doctest()),
        Some("i18n-report") => cmd_i18n_report(),
        Some("ci") => cmd_ci(),
        Some("corpus") => cmd_corpus(rest.contains(&"--download")),
        Some("bench") => cmd_bench(&rest),
        Some("stats") => stats::run(&root(), rest.contains(&"--exact")),
        Some("parity") => parity::run(&root()),
        Some("ico") => ico::run(&rest),
        Some("visual-diff") => visual_diff::run(&rest),
        Some("version") => version::run(&root(), &rest),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has a parent dir").to_path_buf()
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (plan/architecture.md §3)\n");
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<28} {:<14} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Workspace packages that must build for wasm32: all L0–L5 crates plus
/// the egui shell.
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5 || layers::short_name(&c.name) == "ui-egui",
            Some(layers::Class::Standalone) => true,
            _ => false,
        })
        .map(|c| c.name)
        .collect())
}

/// One `cargo check` for the whole set: each crate is checked once, and
/// `--keep-going` still reports every failing crate. Features unify across the
/// set, so each crate is checked with the union of the features the set enables.
fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    if set.is_empty() {
        return Err("no wasm-safe crates found in the workspace metadata".into());
    }
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown", "--keep-going"]);
    for pkg in &set {
        c.args(["-p", pkg]);
    }
    run(c, &format!("cargo check --target wasm32-unknown-unknown --keep-going -p {}", set.join(" -p ")))?;
    println!("\nwasm32-unknown-unknown check: ok ({} crates: {})", set.len(), set.join(", "));
    Ok(())
}

/// Test targets for the test build. Examples are left out so they aren't
/// linked; clippy `--all-targets` still compiles them.
const TEST_TARGETS: [&str; 4] = ["--workspace", "--lib", "--bins", "--tests"];

fn nextest_available() -> bool {
    cargo().args(["nextest", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// The interface audit, which `cmd_i18n_report` runs on its own so its report is printed.
const I18N_AUDIT: &str = "i18n::audit::untranslated_interface_text";
const I18N_AUDIT_FILTER: &str = "package(wordcraft-ui-egui) & test(=i18n::audit::untranslated_interface_text)";

fn cmd_test() -> Result<(), String> {
    test_run(&["-E", &format!("not ({I18N_AUDIT_FILTER})")], &["--skip", I18N_AUDIT])
}

/// Missing translations never fail the build: the report lists them in the log.
fn cmd_i18n_report() -> Result<(), String> {
    test_run(&["-E", I18N_AUDIT_FILTER, "--no-capture"], &["--exact", I18N_AUDIT, "--nocapture"])
}

/// A test run over `TEST_TARGETS`, with nextest arguments or, without nextest, libtest arguments.
/// Every run builds the same targets, so a second run reuses the first one's build.
fn test_run(nextest: &[&str], libtest: &[&str]) -> Result<(), String> {
    let mut c = cargo();
    if nextest_available() {
        c.args(["nextest", "run"]).args(TEST_TARGETS).args(nextest);
        run(c, &format!("cargo nextest run {} {}", TEST_TARGETS.join(" "), nextest.join(" ")))
    } else {
        eprintln!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)");
        c.arg("test").args(TEST_TARGETS).arg("--").args(libtest);
        run(c, &format!("cargo test {} -- {}", TEST_TARGETS.join(" "), libtest.join(" ")))
    }
}

/// nextest does not run doctests.
fn cmd_doctest() -> Result<(), String> {
    let mut c = cargo();
    c.args(["test", "--workspace", "--doc"]);
    run(c, "cargo test --workspace --doc")
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --all-targets -- -D warnings")
            }),
        ),
        ("test", Box::new(cmd_test)),
        ("i18n-report", Box::new(cmd_i18n_report)),
        ("doctest", Box::new(cmd_doctest)),
        ("assets", Box::new(|| assets::run(&root()))),
        ("layers", Box::new(cmd_layers)),
        ("wasm", Box::new(cmd_wasm)),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

const PNGSUITE_URL: &str = "http://www.schaik.com/pngsuite/PngSuite-2017jul19.tgz";

/// `cargo xtask bench`: run the large-document benchmark example in release mode at low priority
/// (the machine may be shared), passing the size options through.
fn cmd_bench(args: &[&str]) -> Result<(), String> {
    let mut c = if cfg!(unix) {
        let mut c = Command::new("nice");
        c.args(["-n", "19"]).arg(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        c.current_dir(root());
        c
    } else {
        cargo()
    };
    c.args(["run", "--release", "-p", "wordcraft-engine", "--example", "large_doc_bench", "--"]).args(args);
    run(c, "cargo run --release --example large_doc_bench")
}

fn cmd_corpus(download: bool) -> Result<(), String> {
    let corpus = root().join("corpus");
    println!(
        "Test corpora live under {} (git-ignored, never committed).
Tests that use a corpus skip cleanly when it is absent.

  corpus/pngsuite/   PngSuite (public domain) — wordcraft-codecs compares every file
                     against the `image` crate. Fetch: cargo xtask corpus --download
  corpus/psd/        PSD samples from MIT/BSD projects (ag-psd, psd-tools test data).
                     Copy files in manually; licences must be MIT/BSD/CC0.
  corpus/tiff/       libtiff pics (optional)
  corpus/exr/        OpenEXR sample images (optional)
  corpus/raw/        raw.pixls.us samples, CC0 (optional)
",
        corpus.display()
    );
    if !download {
        return Ok(());
    }
    let dest = corpus.join("pngsuite");
    std::fs::create_dir_all(&dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    let tgz = corpus.join("PngSuite-2017jul19.tgz");
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "-o"]).arg(&tgz).arg(PNGSUITE_URL);
    run(curl, &format!("curl {PNGSUITE_URL}"))?;
    let mut tar = Command::new("tar");
    tar.arg("-xzf").arg(&tgz).arg("-C").arg(&dest);
    run(tar, "tar -xzf PngSuite-2017jul19.tgz")?;
    let _ = std::fs::remove_file(&tgz);
    let n = std::fs::read_dir(&dest).map(|d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "png")).count()).unwrap_or(0);
    println!("PngSuite: {n} PNG files in {}", dest.display());
    if n == 0 { Err("no PNG files extracted".into()) } else { Ok(()) }
}
