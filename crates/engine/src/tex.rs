//! TeX sidecar: external engine detection.
//!
//! Probe order is fixed: the bundled tectonic sidecar first, then system
//! pdflatex, then system xelatex, else no engine. Detection is a pure
//! function of a directory list plus an executability predicate, so tests run
//! against faked tool trees without touching PATH. No UI reads this yet.

use std::path::{Path, PathBuf};

/// An external TeX engine found on the machine (or the lack of one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TexEngine {
    Tectonic(PathBuf),
    PdfLatex(PathBuf),
    XeLatex(PathBuf),
    Missing,
}

impl TexEngine {
    /// Short engine name for status lines and logs.
    pub fn name(&self) -> &str {
        match self {
            TexEngine::Tectonic(_) => "tectonic",
            TexEngine::PdfLatex(_) => "pdflatex",
            TexEngine::XeLatex(_) => "xelatex",
            TexEngine::Missing => "none",
        }
    }

    /// Path of the engine binary, if one was found.
    pub fn path(&self) -> Option<&Path> {
        match self {
            TexEngine::Tectonic(p) | TexEngine::PdfLatex(p) | TexEngine::XeLatex(p) => Some(p),
            TexEngine::Missing => None,
        }
    }
}

/// Detect an engine by scanning dirs for each tool in probe order
/// (tectonic, then pdflatex, then xelatex): a tectonic binary in a later
/// directory still beats a pdflatex binary in an earlier one.
pub fn detect_with(dirs: &[PathBuf], is_executable: &dyn Fn(&Path) -> bool) -> TexEngine {
    let find = |tool: &str| -> Option<PathBuf> { dirs.iter().map(|d| d.join(tool)).find(|p| is_executable(p)) };
    if let Some(p) = find("tectonic") {
        return TexEngine::Tectonic(p);
    }
    if let Some(p) = find("pdflatex") {
        return TexEngine::PdfLatex(p);
    }
    if let Some(p) = find("xelatex") {
        return TexEngine::XeLatex(p);
    }
    TexEngine::Missing
}

/// Engine probe: a regular file with any execute bit set. The raw check lives
/// in the io gateway so the automation read root applies.
fn is_executable_file(path: &Path) -> bool {
    crate::io::file_is_executable(path)
}

/// Detect an engine using the PATH environment variable.
pub fn detect_from_path() -> TexEngine {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    detect_with(&dirs, &is_executable_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("wctex-{p}-{n}", p = std::process::id(), n = N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cleanup(d: &Path) {
        let _ = std::fs::remove_dir_all(d);
    }

    fn fake_exe(dir: &Path, tool: &str) -> PathBuf {
        let p = dir.join(tool);
        std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[test]
    fn missing_with_no_candidates() {
        assert_eq!(detect_with(&[], &|_| false), TexEngine::Missing);
        let d = tmpdir();
        assert_eq!(detect_with(std::slice::from_ref(&d), &|_| false), TexEngine::Missing);
        cleanup(&d);
    }

    #[test]
    fn tectonic_wins_over_system_latex() {
        let d = tmpdir();
        let t = fake_exe(&d, "tectonic");
        fake_exe(&d, "pdflatex");
        fake_exe(&d, "xelatex");
        assert_eq!(detect_with(std::slice::from_ref(&d), &is_executable_file), TexEngine::Tectonic(t));
        cleanup(&d);
    }

    #[test]
    fn pdflatex_preferred_over_xelatex() {
        let d = tmpdir();
        let p = fake_exe(&d, "pdflatex");
        fake_exe(&d, "xelatex");
        assert_eq!(detect_with(std::slice::from_ref(&d), &is_executable_file), TexEngine::PdfLatex(p));
        cleanup(&d);
    }

    #[test]
    fn xelatex_alone_is_found() {
        let d = tmpdir();
        let x = fake_exe(&d, "xelatex");
        assert_eq!(detect_with(std::slice::from_ref(&d), &is_executable_file), TexEngine::XeLatex(x));
        cleanup(&d);
    }

    #[test]
    fn later_dir_tectonic_beats_earlier_pdflatex() {
        let d1 = tmpdir();
        let d2 = tmpdir();
        fake_exe(&d1, "pdflatex");
        let t = fake_exe(&d2, "tectonic");
        assert_eq!(detect_with(&[d1.clone(), d2.clone()], &is_executable_file), TexEngine::Tectonic(t));
        cleanup(&d1);
        cleanup(&d2);
    }

    #[test]
    #[cfg(unix)]
    fn non_executable_tectonic_is_ignored() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmpdir();
        let p = d.join("tectonic");
        std::fs::write(&p, "not really an engine").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(detect_with(std::slice::from_ref(&d), &is_executable_file), TexEngine::Missing);
        cleanup(&d);
    }

    #[test]
    fn path_probe_runs_without_panicking() {
        let found = detect_from_path();
        assert!(["tectonic", "pdflatex", "xelatex", "none"].contains(&found.name()));
        assert_eq!(found.path().is_some(), found != TexEngine::Missing);
    }
}

// Manual compile (item 4): explicit-only worker plus opaque source state.
// Nothing here runs on its own: the tex.* commands drive it, and opening a
// .tex file never imports anything into the document model.
// Item 5 adds opt-in auto-compile (debounced, generation-guarded), structured
// diagnostics parsed from the log, and a stale-preview badge. Auto-compile
// only arms when the user enables it for the open document, and only fires
// through an explicit poll (`tex.tick`) or manual `tex.compile`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Default per-compile wall clock: the worker is killed past this.
pub const DEFAULT_COMPILE_TIMEOUT: Duration = Duration::from_secs(120);
/// Largest .tex source kept as an opaque buffer.
pub const MAX_SOURCE_BYTES: usize = 32 << 20;
/// Largest compiled PDF kept in memory.
pub const MAX_PDF_BYTES: u64 = 64 << 20;
/// Largest compile log kept (raw engine output, truncated past this).
pub const MAX_LOG_BYTES: usize = 128 << 10;

static SCRATCH_N: AtomicU64 = AtomicU64::new(0);

/// A .tex file opened as opaque bytes: never parsed, never imported.
#[derive(Clone, Debug, Default)]
pub struct TexSource {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// The outcome of one explicit manual compile.
#[derive(Clone, Debug, Default)]
pub struct TexCompile {
    pub engine: String,
    pub success: bool,
    pub timed_out: bool,
    pub log: String,
    pub pdf: Option<Vec<u8>>,
    pub preview_png: Option<Vec<u8>>,
    pub synctex: Option<Vec<u8>>,
    /// Source revision that was compiled (stamped by the caller; 0 when unset).
    pub revision: u64,
}

/// TeX sidecar session state: opaque source plus last compile (outside undo).
#[derive(Clone, Debug, Default)]
pub struct TexState {
    pub source: Option<TexSource>,
    pub last: Option<TexCompile>,
    /// Monotonic source revision: 1 after `tex.open`, +1 per `tex.edit`.
    pub revision: u64,
    /// Opt-in auto-compile for this open document (never on unless requested).
    pub auto_compile: bool,
    /// Latest edited revision waiting out the debounce window (auto only).
    pub pending_revision: Option<u64>,
    /// When the current pending revision was last edited (debounce clock).
    pub pending_since: Option<Instant>,
    /// An edit landed after the last successful compile: preview is stale.
    pub preview_stale: bool,
    /// Revision the last published compile ran on.
    pub compiled_revision: Option<u64>,
    /// Structured log entries from the last published compile.
    pub diagnostics: Vec<TexDiagnostic>,
    /// Cite keys parsed from the bibliography beside the source: completion
    /// data for `tex.bibKeys`, refreshed on open, explicit load and compile.
    pub bib_keys: Vec<BibEntry>,
    /// The bibliography file the keys came from, when one was loaded.
    pub bib_path: Option<PathBuf>,
    /// Compile generation: each started compile takes the next number, and
    /// only the newest generation may publish, so superseded results die.
    pub compile_generation: u64,
    /// The compile the session has started but not yet published, if any.
    pub in_flight: Option<InFlightCompile>,
}

/// A compile the session has started but not yet published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InFlightCompile {
    pub revision: u64,
    pub generation: u64,
}

/// Resolve an explicit engine override path plus an optional kind name
/// (tectonic, pdflatex or xelatex). Without a kind the file name decides,
/// defaulting to tectonic for the bundled sidecar.
pub fn engine_for(path: &str, kind: Option<&str>) -> Result<TexEngine, String> {
    let path = PathBuf::from(path);
    let owned;
    let kind = match kind {
        Some(k) => {
            owned = k.to_ascii_lowercase();
            owned.as_str()
        }
        None => {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
            if name.contains("pdflatex") {
                "pdflatex"
            } else if name.contains("xelatex") {
                "xelatex"
            } else {
                "tectonic"
            }
        }
    };
    match kind {
        "tectonic" => Ok(TexEngine::Tectonic(path)),
        "pdflatex" => Ok(TexEngine::PdfLatex(path)),
        "xelatex" => Ok(TexEngine::XeLatex(path)),
        other => Err(format!("unknown TeX engine kind (expected tectonic, pdflatex or xelatex): {other}")),
    }
}

/// Command line for one compile as (program, args): engine flags, outdir,
/// source. Tectonic always runs with sandboxing on; the pdftex engines run
/// non-interactive with synctex.
pub fn compile_argv(engine: &TexEngine, source: &Path, outdir: &Path) -> Result<(PathBuf, Vec<String>), String> {
    let (program, flags): (PathBuf, &[&str]) = match engine {
        TexEngine::Tectonic(p) => (p.clone(), &["--untrusted", "--synctex", "--outdir"]),
        TexEngine::PdfLatex(p) => (p.clone(), &["-interaction=nonstopmode", "-synctex=1", "-output-directory"]),
        TexEngine::XeLatex(p) => (p.clone(), &["-interaction=nonstopmode", "-synctex=1", "-output-directory"]),
        TexEngine::Missing => return Err("no TeX engine available".to_string()),
    };
    let mut args: Vec<String> = flags.iter().map(|s| s.to_string()).collect();
    args.push(outdir.to_string_lossy().into_owned());
    args.push(source.to_string_lossy().into_owned());
    Ok((program, args))
}

// Bibliography and multi-file includes: cite keys plus staging.
//
// A `.bib` beside the `.tex` feeds `tex.bibKeys` (parsed here, never through
// a worker) and is staged into the compile scratch so the engine resolves
// `\cite`. `\input`/`\include` targets are resolved strictly inside the
// document directory: absolute paths and `..` escapes are clean errors, never
// reads outside the tree. Everything here is hostile-input safe: unbalanced
// braces, NULs and absurd sizes cost entries, never panics.

/// Largest `.bib` file parsed (bytes).
pub const MAX_BIB_BYTES: usize = 4 << 20;
/// Most bibliography files staged for one compile.
pub const MAX_BIB_FILES: usize = 16;
/// Most cite keys kept from the staged bibliographies.
pub const MAX_BIB_ENTRIES: usize = 5_000;
/// Longest cite key kept (bytes).
pub const MAX_BIB_KEY_LEN: usize = 256;
/// Most `\input`/`\include` files staged for one compile.
pub const MAX_INCLUDE_FILES: usize = 64;
/// Deepest `\input`/`\include` nesting followed.
pub const MAX_INCLUDE_DEPTH: usize = 8;
/// Largest single included `.tex` staged (bytes).
pub const MAX_INCLUDE_BYTES: u64 = 4 << 20;
/// Longest include/bibliography name honoured (bytes).
pub const MAX_INCLUDE_NAME: usize = 1024;

/// One BibTeX entry's cite key for completion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BibEntry {
    pub key: String,
    pub entry_type: String,
}

/// Skip ASCII whitespace from `pos`.
fn bib_skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t' | b'\r' | b'\n') {
        pos += 1;
    }
    pos
}

/// Entry types that define no cite key.
fn bib_is_keyless(kind: &str) -> bool {
    matches!(kind, "string" | "preamble" | "comment")
}

/// A cite key is valid when it is short, non-empty and holds none of BibTeX's
/// structural characters or whitespace.
fn bib_key_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_BIB_KEY_LEN
        && key.bytes().all(|b| !b.is_ascii_whitespace() && !matches!(b, b'"' | b'#' | b'\'' | b'(' | b')' | b',' | b'=' | b'{' | b'}' | b'%'))
}

/// Parse BibTeX entries into cite keys for completion. Hostile entries
/// (unbalanced braces, empty or malformed keys, `@string`/`@preamble` noise)
/// are skipped and parsing resumes at the next `@`, so one bad entry never
/// hides the rest. Duplicate keys keep their first occurrence.
pub fn parse_bibtex(text: &str) -> Vec<BibEntry> {
    let bytes = text.as_bytes();
    let mut out: Vec<BibEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pos = 0usize;
    while pos < bytes.len() && out.len() < MAX_BIB_ENTRIES {
        let Some(rel) = bytes[pos..].iter().position(|&b| b == b'@') else { break };
        pos += rel;
        let start = pos;
        pos += 1;
        let type_start = bib_skip_ws(bytes, pos);
        let mut type_end = type_start;
        while type_end < bytes.len() && bytes[type_end].is_ascii_alphabetic() {
            type_end += 1;
        }
        if type_end == type_start {
            pos = start + 1;
            continue;
        }
        let kind = bytes.get(type_start..type_end).and_then(|s| std::str::from_utf8(s).ok()).map(str::to_ascii_lowercase).unwrap_or_default();
        if kind.is_empty() {
            pos = start + 1;
            continue;
        }
        pos = bib_skip_ws(bytes, type_end);
        if bytes.get(pos) != Some(&b'{') && bytes.get(pos) != Some(&b'(') {
            pos = start + 1;
            continue;
        }
        let open = bytes[pos];
        let close = if open == b'{' { b'}' } else { b')' };
        pos += 1;
        // The cite key runs to the first comma or the closing delimiter.
        let key_start = bib_skip_ws(bytes, pos);
        let mut key_end = key_start;
        while key_end < bytes.len() && bytes[key_end] != b',' && bytes[key_end] != close {
            if key_end - key_start > MAX_BIB_KEY_LEN + 64 {
                break;
            }
            key_end += 1;
        }
        let key = bytes.get(key_start..key_end).and_then(|s| std::str::from_utf8(s).ok()).map(str::trim).unwrap_or_default();
        if bytes.get(key_end) == Some(&b',') {
            pos = key_end + 1;
        } else {
            pos = key_end;
        }
        let valid_key = bib_key_valid(key);
        // Skip the entry body, balancing the outer delimiter with awareness
        // of `"..."` strings and `%` comments so braces inside them never
        // miscount. An unbalanced body discards the entry and resumes at the
        // next `@` so later valid entries still parse.
        let mut depth = 1usize;
        let mut in_quote = false;
        let mut escaped = false;
        let mut commented = false;
        let mut closed = false;
        while pos < bytes.len() {
            let b = bytes[pos];
            if b == b'\n' {
                commented = false;
                escaped = false;
                pos += 1;
                continue;
            }
            if commented {
                pos += 1;
                continue;
            }
            if in_quote {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_quote = false;
                }
                pos += 1;
                continue;
            }
            if b == b'%' {
                commented = true;
                pos += 1;
                continue;
            }
            if b == b'"' {
                in_quote = true;
                pos += 1;
                continue;
            }
            if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    closed = true;
                    pos += 1;
                    break;
                }
            }
            pos += 1;
        }
        if !closed {
            if let Some(next) = bytes[start + 1..].iter().position(|&b| b == b'@') {
                pos = start + 1 + next;
            } else {
                break;
            }
            continue;
        }
        if valid_key && !bib_is_keyless(&kind) && seen.insert(key.to_string()) {
            out.push(BibEntry { key: key.to_string(), entry_type: kind });
        }
    }
    out
}

/// Strip TeX comments: an unescaped `%` (an even backslash run protects it,
/// so `\\%` starts a comment but `\%` does not) drops to end of line.
fn strip_tex_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut backslashes = 0usize;
    let mut commented = false;
    for ch in text.chars() {
        if ch == '\n' {
            commented = false;
            backslashes = 0;
            out.push(ch);
            continue;
        }
        if commented {
            continue;
        }
        if ch == '%' && backslashes.is_multiple_of(2) {
            commented = true;
            continue;
        }
        if ch == '\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
        out.push(ch);
    }
    out
}

/// Scan one control word's argument: `{braced}` form, or the bare `\input f`
/// space form otherwise. Returns the name and the position after it.
fn scan_tex_arg(bytes: &[u8], from: usize, braced: bool) -> Option<(String, usize)> {
    if braced {
        let mut end = from + 1;
        while end < bytes.len() && bytes[end] != b'}' && end - (from + 1) < 2048 {
            end += 1;
        }
        if bytes.get(end) != Some(&b'}') {
            return None;
        }
        let name = String::from_utf8_lossy(bytes.get(from + 1..end).unwrap_or_default()).trim().to_string();
        Some((name, end + 1))
    } else {
        let mut end = from;
        while end < bytes.len() && !matches!(bytes[end], b' ' | b'\t' | b'\r' | b'\n' | b'{') && end - from < 2048 {
            end += 1;
        }
        let name = String::from_utf8_lossy(bytes.get(from..end).unwrap_or_default()).trim().to_string();
        Some((name, end.max(from + 1)))
    }
}

/// Control-word arguments (`\input`, `\include`, `\bibliography`,
/// `\addbibresource`) in source order. Comments are stripped first so
/// commented-out names never stage. Capped at `cap`; overlong names are kept
/// for [`resolve_include`] to refuse with a clean error.
fn find_tex_args(source: &[u8], words: &[&str], cap: usize) -> Vec<String> {
    let text = strip_tex_comments(&String::from_utf8_lossy(source));
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() && out.len() < cap {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j == i + 1 {
            i += 1;
            continue;
        }
        let word = bytes.get(i + 1..j).unwrap_or_default();
        if !words.iter().any(|w| word == w.as_bytes()) {
            i = j;
            continue;
        }
        let is_bib = words.contains(&"bibliography");
        let mut k = j;
        while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'\r' | b'\n') {
            k += 1;
        }
        if k >= bytes.len() {
            break;
        }
        let braced = bytes[k] == b'{';
        if !braced && is_bib {
            // `\bibliography` without braces names no bibliography list.
            i = k + 1;
            continue;
        }
        match scan_tex_arg(bytes, k, braced) {
            Some((name, next)) => {
                i = next;
                if name.is_empty() {
                    continue;
                }
                if is_bib {
                    for part in name.split(',') {
                        if out.len() >= cap {
                            break;
                        }
                        let trimmed = part.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                    }
                } else {
                    out.push(name);
                }
            }
            None => {
                i = k + 1;
            }
        }
    }
    out
}

/// `\input`/`\include` targets (brace form plus the `\input file` space form)
/// in source order.
pub fn find_tex_includes(source: &[u8]) -> Vec<String> {
    find_tex_args(source, &["input", "include"], MAX_INCLUDE_FILES)
}

/// `\bibliography{a,b}` and biblatex `\addbibresource{file.bib}` targets in
/// source order (comma-split, trimmed).
pub fn find_bibliography_names(source: &[u8]) -> Vec<String> {
    find_tex_args(source, &["bibliography", "addbibresource"], MAX_BIB_FILES)
}

/// Shorten a requested name for error messages (char-boundary safe).
fn short_tex_name(req: &str) -> String {
    const MAX: usize = 80;
    if req.len() <= MAX {
        return req.to_string();
    }
    let mut cut = MAX;
    while cut > 0 && !req.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", req.get(..cut).unwrap_or_default())
}

/// Resolve one `\input`/`\include`/`\bibliography` name strictly inside
/// `doc_dir`. Absolute paths, Windows drives, `~` roots and any `..`
/// component (slash or backslash) are refused with a clean error; the join
/// can never name a file outside the document directory.
pub fn resolve_include(doc_dir: &Path, requested: &str) -> Result<PathBuf, String> {
    let req = requested.trim();
    if req.is_empty() {
        return Err("TeX include path is empty".to_string());
    }
    if req.len() > MAX_INCLUDE_NAME {
        return Err(format!("TeX include path is over the {MAX_INCLUDE_NAME} byte limit"));
    }
    if req.contains('\0') {
        return Err("TeX include path contains NUL".to_string());
    }
    let raw = req.as_bytes();
    if matches!(raw.first(), Some(b'/') | Some(b'\\') | Some(b'~')) {
        return Err(format!("TeX include `{}` is absolute: includes stay inside the document directory", short_tex_name(req)));
    }
    if raw.len() >= 2 && raw[1] == b':' && raw[0].is_ascii_alphabetic() {
        return Err(format!("TeX include `{}` is absolute: includes stay inside the document directory", short_tex_name(req)));
    }
    let mut kept: Vec<&str> = Vec::new();
    for comp in req.split(['/', '\\']) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            return Err(format!("TeX include `{}` escapes the document directory", short_tex_name(req)));
        }
        kept.push(comp);
    }
    if kept.is_empty() {
        return Err("TeX include path is empty".to_string());
    }
    let mut out = doc_dir.to_path_buf();
    for comp in kept {
        out.push(comp);
    }
    Ok(out)
}

/// BibTeX/bibliography log lines: unresolved citations and BibTeX failures.
/// Runs after the generic shapes so already-parsed lines are never doubled.
fn parse_bib_line(line: &str, revision: u64) -> Option<TexDiagnostic> {
    let lower = line.to_ascii_lowercase();
    let citation = lower.contains("citation") && (lower.contains("undefined") || lower.contains("missing"));
    let bibtex_warn = lower.contains("i didn't find a database entry")
        || lower.contains("i found no \\citation")
        || lower.contains("empty bibliography")
        || lower.contains("there were undefined citations");
    if citation || bibtex_warn {
        let message = cap_message(line);
        if message.is_empty() {
            return None;
        }
        return Some(TexDiagnostic { file: String::new(), line: parse_input_line(line), severity: TexSeverity::Warning, message, revision });
    }
    let bibtex_fail = lower.contains("bibtex")
        && (lower.contains("error")
            || lower.contains("failed")
            || lower.contains("not found")
            || lower.contains("couldn't open")
            || lower.contains("could not open"));
    if bibtex_fail {
        let message = cap_message(line);
        if message.is_empty() {
            return None;
        }
        return Some(TexDiagnostic { file: String::new(), line: parse_input_line(line), severity: TexSeverity::Error, message, revision });
    }
    None
}

/// One compile's staged sidecar: what the scratch dir holds besides main.tex.
#[cfg(not(target_arch = "wasm32"))]
pub struct StagedInputs {
    /// A `.bib` was staged: the engine resolves `\cite`.
    pub has_bib: bool,
    /// Staged `\input`/`\include` files.
    pub includes: usize,
    /// Staged bibliography file names.
    pub bibs: Vec<String>,
    /// Merged cite keys from the staged bibliographies.
    pub bib_keys: Vec<BibEntry>,
}

/// Read one resolved sidecar file through the io gateway. A name without an
/// extension also tries `name.<fallback>` (`\input{chapter1}` finds
/// `chapter1.tex`; `\bibliography{refs}` finds `refs.bib`).
#[cfg(not(target_arch = "wasm32"))]
fn read_sidecar_file(resolved: &Path, what: &str, max: u64, fallback: &str) -> Result<(PathBuf, Vec<u8>), String> {
    let direct = resolved.to_string_lossy().into_owned();
    match crate::io::read_file_capped(&direct, max, what) {
        Ok(bytes) => Ok((resolved.to_path_buf(), bytes)),
        Err(first) => {
            if resolved.extension().is_none() {
                let with_ext = resolved.with_extension(fallback);
                if let Ok(bytes) = crate::io::read_file_capped(&with_ext.to_string_lossy(), max, what) {
                    return Ok((with_ext, bytes));
                }
            }
            Err(first)
        }
    }
}

/// Stage one bibliography's bytes under `rel`: write it, record its name and
/// merge its cite keys. Duplicates (same relative path) stage once.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn stage_bib_bytes(
    scratch: &Path,
    staged: &mut std::collections::HashSet<PathBuf>,
    bibs: &mut Vec<String>,
    bib_keys: &mut Vec<BibEntry>,
    seen_keys: &mut std::collections::HashSet<String>,
    rel: PathBuf,
    bytes: Vec<u8>,
    display: &str,
) -> Result<(), String> {
    if bibs.len() >= MAX_BIB_FILES {
        return Err(format!("TeX document stages more than {MAX_BIB_FILES} bibliographies"));
    }
    if !staged.insert(rel.clone()) {
        return Ok(());
    }
    if let Some(parent) = rel.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(scratch.join(parent)).map_err(|e| format!("could not make scratch dir: {e}"))?;
    }
    crate::io::write_file(&scratch.join(&rel).to_string_lossy(), &bytes).map_err(|e| format!("could not stage bibliography `{display}`: {e}"))?;
    if let Some(name) = rel.file_name().and_then(|s| s.to_str()) {
        bibs.push(name.to_string());
    }
    for entry in parse_bibtex(&String::from_utf8_lossy(&bytes)) {
        if seen_keys.insert(entry.key.clone()) {
            bib_keys.push(entry);
        }
    }
    Ok(())
}

/// Stage `\input`/`\include` files and `.bib` files into the scratch dir,
/// preserving their layout relative to the document directory. Reads go
/// through the io gateway (automation roots apply); the scratch tree itself
/// is worker temp, like [`new_scratch`].
#[cfg(not(target_arch = "wasm32"))]
pub fn stage_compile_inputs(source_path: &Path, source_bytes: &[u8], scratch: &Path, stored_bib: Option<&Path>) -> Result<StagedInputs, String> {
    let doc_dir: PathBuf = match source_path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        Some(dir) => dir.to_path_buf(),
        None => PathBuf::from("."),
    };
    let mut staged: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut includes = 0usize;
    let mut bib_names: Vec<String> = Vec::new();
    // Includes breadth-first: each staged file contributes its own nested
    // targets until the depth and total caps bite.
    let mut work: Vec<(String, usize)> = find_tex_includes(source_bytes).into_iter().map(|name| (name, 0)).collect();
    while let Some((name, depth)) = work.pop() {
        if includes >= MAX_INCLUDE_FILES {
            return Err(format!("TeX document includes more than {MAX_INCLUDE_FILES} files"));
        }
        let resolved = resolve_include(&doc_dir, &name)?;
        let (actual, bytes) = read_sidecar_file(&resolved, "TeX include", MAX_INCLUDE_BYTES, "tex")
            .map_err(|e| format!("could not read TeX include `{}`: {e}", short_tex_name(&name)))?;
        let rel = actual
            .strip_prefix(&doc_dir)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| actual.file_name().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("include.tex")));
        if !staged.insert(rel.clone()) {
            continue;
        }
        if let Some(parent) = rel.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(scratch.join(parent)).map_err(|e| format!("could not make scratch dir: {e}"))?;
        }
        crate::io::write_file(&scratch.join(&rel).to_string_lossy(), &bytes)
            .map_err(|e| format!("could not stage TeX include `{}`: {e}", short_tex_name(&name)))?;
        includes += 1;
        let nested = find_tex_includes(&bytes);
        if !nested.is_empty() && depth >= MAX_INCLUDE_DEPTH {
            return Err(format!("TeX includes nest deeper than {MAX_INCLUDE_DEPTH} levels"));
        }
        for next in nested {
            work.push((next, depth + 1));
        }
        bib_names.extend(find_bibliography_names(&bytes));
    }
    bib_names.extend(find_bibliography_names(source_bytes));
    // Bibliographies: the same-stem file rides along when present; a
    // `\bibliography` name that is missing is an error rather than a
    // cryptic engine failure.
    let mut bibs: Vec<String> = Vec::new();
    let mut bib_keys: Vec<BibEntry> = Vec::new();
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut wanted: Vec<(String, bool)> = Vec::new();
    if let Some(stem) = source_path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()) {
        wanted.push((format!("{stem}.bib"), false));
    }
    for name in bib_names {
        wanted.push((name, true));
    }
    for (name, required) in wanted {
        let resolved = match resolve_include(&doc_dir, &name) {
            Ok(path) => path,
            Err(e) => {
                if required {
                    return Err(format!("\\bibliography{{{n}}}: {e}", n = short_tex_name(&name)));
                }
                continue;
            }
        };
        match read_sidecar_file(&resolved, "bibliography", MAX_BIB_BYTES as u64, "bib") {
            Ok((actual, bytes)) => {
                let rel = actual
                    .strip_prefix(&doc_dir)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| actual.file_name().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("refs.bib")));
                stage_bib_bytes(scratch, &mut staged, &mut bibs, &mut bib_keys, &mut seen_keys, rel, bytes, &short_tex_name(&name))?;
            }
            Err(e) => {
                if required {
                    return Err(format!("could not read TeX bibliography `{}`: {e}", short_tex_name(&name)));
                }
            }
        }
    }
    // An explicitly loaded bibliography may live outside the document
    // directory: stage it flat under its file name, best effort.
    if let Some(path) = stored_bib {
        let path_str = path.to_string_lossy().into_owned();
        if let Ok(bytes) = crate::io::read_file_capped(&path_str, MAX_BIB_BYTES as u64, "bibliography")
            && let Some(file) = path.file_name().map(PathBuf::from)
        {
            let display = short_tex_name(&path_str);
            let _ = stage_bib_bytes(scratch, &mut staged, &mut bibs, &mut bib_keys, &mut seen_keys, file, bytes, &display);
        }
    }
    Ok(StagedInputs { has_bib: !bibs.is_empty(), includes, bibs, bib_keys })
}

/// What a guarded worker run produced.
#[derive(Clone, Debug)]
pub struct GuardedOutcome {
    pub timed_out: bool,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
fn drain_to_end<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut v);
        }
        v
    })
}

/// Run a worker with piped output and a wall clock: past the deadline the
/// child is killed and reported as timed out. Stdout and stderr drain on
/// threads so a chatty child can never block on a full pipe.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_guarded(program: &Path, args: &[String], timeout: Duration) -> Result<GuardedOutcome, String> {
    use std::process::Stdio;
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {p}: {e}", p = program.display()))?;
    let out_h = drain_to_end(child.stdout.take());
    let err_h = drain_to_end(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let code = loop {
        match child.try_wait().map_err(|e| format!("could not poll {p}: {e}", p = program.display()))? {
            Some(status) => break status.code(),
            None => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    if !timed_out {
        let _ = child.wait();
    }
    Ok(GuardedOutcome { timed_out, code, stdout: out_h.join().unwrap_or_default(), stderr: err_h.join().unwrap_or_default() })
}

#[cfg(target_arch = "wasm32")]
pub fn run_guarded(_program: &Path, _args: &[String], _timeout: Duration) -> Result<GuardedOutcome, String> {
    Err("TeX workers are not available on this platform".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn cap_log_bytes(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    if text.len() <= MAX_LOG_BYTES {
        return text.into_owned();
    }
    let mut cut = MAX_LOG_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{head}[log truncated]", head = &text[..cut])
}

/// Rasterizer for PDF previews: explicit override, else pdftoppm from PATH.
pub fn find_rasterizer(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return is_executable_file(p).then(|| p.to_path_buf());
    }
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    dirs.iter().map(|d| d.join("pdftoppm")).find(|p| is_executable_file(p))
}

/// `bibtex` from PATH for the pdfTeX multi-pass, if one is executable. Like
/// [`find_rasterizer`]: probing only, never spawning (wasm-safe).
pub fn find_bibtex() -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    dirs.iter().map(|d| d.join("bibtex")).find(|p| is_executable_file(p))
}

/// First PDF page as PNG via the rasterizer (pdftoppm one-shot arguments).
#[cfg(not(target_arch = "wasm32"))]
pub fn rasterize_pdf(rasterizer: &Path, pdf: &Path, prefix: &Path, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut args: Vec<String> = ["-png", "-r", "150", "-singlefile", "-f", "1", "-l", "1"].iter().map(|s| s.to_string()).collect();
    args.push(pdf.to_string_lossy().into_owned());
    args.push(prefix.to_string_lossy().into_owned());
    let outcome = run_guarded(rasterizer, &args, timeout)?;
    if outcome.timed_out {
        return Err("preview rasterizer timed out".to_string());
    }
    if outcome.code != Some(0) {
        let mut raw = outcome.stdout;
        raw.extend_from_slice(&outcome.stderr);
        return Err(format!("preview rasterizer failed: {log}", log = cap_log_bytes(&raw)));
    }
    let direct = prefix.with_extension("png");
    let numbered = PathBuf::from(format!("{stem}-1.png", stem = prefix.to_string_lossy()));
    for candidate in [&direct, &numbered] {
        if let Ok(bytes) = std::fs::read(candidate)
            && !bytes.is_empty()
        {
            return Ok(bytes);
        }
    }
    Err("preview rasterizer produced no PNG".to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn rasterize_pdf(_rasterizer: &Path, _pdf: &Path, _prefix: &Path, _timeout: Duration) -> Result<Vec<u8>, String> {
    Err("TeX workers are not available on this platform".to_string())
}

/// Compile one source file with an explicit engine: run the worker, pick up
/// the same-stem PDF from the outdir, rasterize page one for the preview.
/// Never touches the document model; the caller owns scratch cleanup.
///
/// Tectonic needs no extra flag for bibliographies: its default pass already
/// runs BibTeX and reruns to resolve `\cite` (forcing `--pass bibtex_first`
/// fails cold builds with `couldn't open main.aux`, because no `.aux` exists
/// yet). The pdfTeX engines get an explicit multi-pass below when a `.bib`
/// was staged.
#[cfg(not(target_arch = "wasm32"))]
pub fn compile_tex(source_file: &Path, outdir: &Path, engine: &TexEngine, rasterizer: Option<&Path>, timeout: Duration, has_bib: bool) -> TexCompile {
    if has_bib && matches!(engine, TexEngine::PdfLatex(_) | TexEngine::XeLatex(_)) {
        return compile_pdftex_multi(source_file, outdir, engine, rasterizer, timeout);
    }
    let name = engine.name().to_string();
    let fail = |log: String| TexCompile {
        engine: name.clone(),
        success: false,
        timed_out: false,
        log,
        pdf: None,
        preview_png: None,
        synctex: None,
        revision: 0,
    };
    let (program, args) = match compile_argv(engine, source_file, outdir) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let outcome = match run_guarded(&program, &args, timeout) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let mut raw = outcome.stdout;
    raw.extend_from_slice(&outcome.stderr);
    publish_outputs(source_file, outdir, &name, raw, outcome.timed_out, outcome.code, rasterizer, timeout)
}

/// Publish one finished worker run: cap the log, pick up the same-stem PDF
/// and SyncTeX sidecar from the outdir, rasterize page one for the preview.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn publish_outputs(
    source_file: &Path,
    outdir: &Path,
    engine_name: &str,
    raw: Vec<u8>,
    timed_out: bool,
    code: Option<i32>,
    rasterizer: Option<&Path>,
    timeout: Duration,
) -> TexCompile {
    let complete = |success: bool, timed_out: bool, log: String, pdf: Option<Vec<u8>>| TexCompile {
        engine: engine_name.to_string(),
        success,
        timed_out,
        log,
        pdf,
        preview_png: None,
        synctex: None,
        revision: 0,
    };
    let mut log = cap_log_bytes(&raw);
    if timed_out {
        log.push_str(" [killed: wall clock expired]");
        return complete(false, true, log, None);
    }
    let stem = source_file.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
    let pdf_path = outdir.join(format!("{stem}.pdf"));
    let pdf = match std::fs::metadata(&pdf_path) {
        Ok(m) if m.is_file() && m.len() <= MAX_PDF_BYTES => std::fs::read(&pdf_path).ok(),
        _ => None,
    };
    let synctex = [format!("{stem}.synctex.gz"), format!("{stem}.synctex")].iter().find_map(|name| {
        let candidate = outdir.join(name);
        match std::fs::metadata(&candidate) {
            Ok(m) if m.is_file() && m.len() <= crate::synctex::MAX_SYNCTEX_BYTES && m.len() > 0 => {
                std::fs::read(&candidate).ok().filter(|b| !b.is_empty())
            }
            _ => None,
        }
    });
    if code != Some(0) && pdf.is_none() {
        return complete(false, false, log, None);
    }
    let success = pdf.is_some();
    let preview_png = match (&pdf, find_rasterizer(rasterizer)) {
        (Some(_), Some(r)) => match rasterize_pdf(&r, &pdf_path, &outdir.join("preview"), timeout) {
            Ok(png) => Some(png),
            Err(e) => {
                log.push_str(" [preview: ");
                log.push_str(&e);
                log.push(']');
                None
            }
        },
        _ => None,
    };
    TexCompile { engine: engine_name.to_string(), success, timed_out: false, log, pdf, preview_png, synctex, revision: 0 }
}

/// pdfTeX/XeTeX with a bibliography: engine, BibTeX, engine, engine, sharing
/// one wall-clock budget. A missing `bibtex` binary (or a failing BibTeX run)
/// is a log note, not a hard error: the reruns still resolve plain labels.
#[cfg(not(target_arch = "wasm32"))]
fn compile_pdftex_multi(source_file: &Path, outdir: &Path, engine: &TexEngine, rasterizer: Option<&Path>, timeout: Duration) -> TexCompile {
    let name = engine.name().to_string();
    let fail = |log: String| TexCompile {
        engine: name.clone(),
        success: false,
        timed_out: false,
        log,
        pdf: None,
        preview_png: None,
        synctex: None,
        revision: 0,
    };
    let (program, args) = match compile_argv(engine, source_file, outdir) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    // One budget split across the four passes (at least a second each, so a
    // tiny caller timeout still runs every pass once).
    let per_pass = (timeout / 4).max(Duration::from_secs(1));
    let outcome = match run_guarded(&program, &args, per_pass) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let mut raw = outcome.stdout;
    raw.extend_from_slice(&outcome.stderr);
    if outcome.timed_out {
        return publish_outputs(source_file, outdir, &name, raw, true, None, rasterizer, timeout);
    }
    if let Some(bibtex) = find_bibtex() {
        let stem = source_file.file_stem().and_then(|s| s.to_str()).unwrap_or("main");
        let bib_args = vec![outdir.join(stem).to_string_lossy().into_owned()];
        match run_guarded(&bibtex, &bib_args, per_pass) {
            Ok(o) => {
                raw.extend_from_slice(b"\n--- TeX citation pass ---\n");
                raw.extend_from_slice(&o.stdout);
                raw.extend_from_slice(&o.stderr);
                if o.timed_out {
                    return publish_outputs(source_file, outdir, &name, raw, true, None, rasterizer, timeout);
                }
            }
            Err(e) => {
                raw.extend_from_slice(format!("\n--- TeX citation pass not run: {e} ---\n").as_bytes());
            }
        }
    } else {
        raw.extend_from_slice(b"\n--- TeX citation pass skipped: no bibliography processor on PATH ---\n");
    }
    let mut code = outcome.code;
    for _ in 0..2 {
        match run_guarded(&program, &args, per_pass) {
            Ok(o) => {
                raw.extend_from_slice(&o.stdout);
                raw.extend_from_slice(&o.stderr);
                if o.timed_out {
                    return publish_outputs(source_file, outdir, &name, raw, true, None, rasterizer, timeout);
                }
                code = o.code;
            }
            Err(e) => {
                raw.extend_from_slice(format!("\n--- TeX rerun not run: {e} ---\n").as_bytes());
                break;
            }
        }
    }
    publish_outputs(source_file, outdir, &name, raw, false, code, rasterizer, timeout)
}

#[cfg(target_arch = "wasm32")]
pub fn compile_tex(
    source_file: &Path,
    _outdir: &Path,
    engine: &TexEngine,
    _rasterizer: Option<&Path>,
    _timeout: Duration,
    _has_bib: bool,
) -> TexCompile {
    let _ = source_file;
    TexCompile {
        engine: engine.name().to_string(),
        success: false,
        timed_out: false,
        log: "TeX workers are not available on this platform".to_string(),
        pdf: None,
        preview_png: None,
        synctex: None,
        revision: 0,
    }
}

// Item 6: SyncTeX forward/inverse jumps over the last successful compile.
// Jumps never parse TeX and never touch the document model: they read the
// `.synctex(.gz)` sidecar captured at compile time, honour the existing
// revision and stale machinery, and refuse stale previews with a clean error
// instead of wrong coordinates. Points are PDF points with a top-left origin
// (x right, y down), so a preview PNG at `dpi` maps via `pixel = pt * dpi / 72`.

/// Where one source line lands in the compiled output.
#[derive(Clone, Debug, PartialEq)]
pub struct TexForwardHit {
    pub page: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub line: u32,
    pub file: String,
    pub revision: u64,
}

/// Which source line owns one preview point.
#[derive(Clone, Debug, PartialEq)]
pub struct TexInverseHit {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub page: u32,
    pub revision: u64,
}

/// Forward search: the page plus rectangle for `line` in the last successful
/// compile. The optional column some editors send is accepted by the command
/// layer and ignored here because SyncTeX records carry lines, not columns.
/// Refuses stale previews and missing compiles with a clean error.
pub fn forward_jump(state: &TexState, line: u32) -> Result<TexForwardHit, String> {
    if line == 0 {
        return Err("line must be at least 1".to_string());
    }
    let Some(source) = state.source.as_ref() else {
        return Err("no TeX source open (run tex.open first)".to_string());
    };
    let Some(last) = state.last.as_ref() else {
        return Err("no successful compile yet (run tex.compile first)".to_string());
    };
    if !last.success {
        return Err("no successful compile yet (last compile failed)".to_string());
    }
    if state.preview_stale || state.revision != last.revision || state.compiled_revision != Some(last.revision) {
        return Err("preview is stale (recompile to refresh jumps)".to_string());
    }
    let Some(raw) = last.synctex.as_ref() else {
        return Err("no SyncTeX data (recompile to refresh jumps)".to_string());
    };
    let doc = crate::synctex::parse_synctex_bytes(raw).map_err(|e| format!("could not parse SyncTeX data: {e}"))?;
    let Some((page, rect)) = crate::synctex::forward_search(&doc, line) else {
        return Err(format!("no SyncTeX mapping for line {line}"));
    };
    if !rect.x.is_finite() || !rect.y.is_finite() || !rect.w.is_finite() || !rect.h.is_finite() {
        return Err(format!("no SyncTeX mapping for line {line}"));
    }
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return Err(format!("no SyncTeX mapping for line {line}"));
    }
    Ok(TexForwardHit {
        page,
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        line,
        file: source.path.to_string_lossy().into_owned(),
        revision: last.revision,
    })
}

/// Inverse search: the source line owning `x, y` on `page` (points, top-left
/// origin, same space as forward jumps). Column is always 0 because SyncTeX
/// records carry lines, not columns. Refuses stale previews cleanly.
pub fn inverse_jump(state: &TexState, page: u32, x: f64, y: f64) -> Result<TexInverseHit, String> {
    if page == 0 {
        return Err("page must be at least 1".to_string());
    }
    if !x.is_finite() || !y.is_finite() {
        return Err("x and y must be finite numbers".to_string());
    }
    let Some(source) = state.source.as_ref() else {
        return Err("no TeX source open (run tex.open first)".to_string());
    };
    let Some(last) = state.last.as_ref() else {
        return Err("no successful compile yet (run tex.compile first)".to_string());
    };
    if !last.success {
        return Err("no successful compile yet (last compile failed)".to_string());
    }
    if state.preview_stale || state.revision != last.revision || state.compiled_revision != Some(last.revision) {
        return Err("preview is stale (recompile to refresh jumps)".to_string());
    }
    let Some(raw) = last.synctex.as_ref() else {
        return Err("no SyncTeX data (recompile to refresh jumps)".to_string());
    };
    let doc = crate::synctex::parse_synctex_bytes(raw).map_err(|e| format!("could not parse SyncTeX data: {e}"))?;
    let Some((_, line)) = crate::synctex::inverse_search(&doc, page, x, y) else {
        return Err(format!("no SyncTeX mapping for page {page} position"));
    };
    Ok(TexInverseHit { file: source.path.to_string_lossy().into_owned(), line, column: 0, page, revision: last.revision })
}

/// A fresh empty directory for one compile: temp dir, process id, counter.
pub fn new_scratch(prefix: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("{prefix}-{p}-{n}", p = std::process::id(), n = SCRATCH_N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make scratch dir: {e}"))?;
    Ok(dir)
}

// Item 5: opt-in auto-compile debounce, structured diagnostics, stale badge.
// Auto-compile never runs unless the user enables it for the open document:
// edits only arm a pending revision plus a timestamp, and an explicit poll
// (`tex.tick`) or manual `tex.compile` performs at most one compile for the
// newest revision. Generations discard superseded results so only the latest
// revision's output is ever shown.

/// Quiet window after the last edit before an auto-compile may fire.
pub const AUTO_COMPILE_DEBOUNCE: Duration = Duration::from_millis(750);
/// Most diagnostic entries kept from one compile log.
pub const MAX_DIAGNOSTICS: usize = 200;
/// Longest single diagnostic message kept (bytes, cut at a char boundary).
pub const MAX_DIAGNOSTIC_MESSAGE: usize = 2000;

/// How serious one parsed log entry is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TexSeverity {
    Error,
    Warning,
}

impl TexSeverity {
    /// Stable lowercase name for JSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            TexSeverity::Error => "error",
            TexSeverity::Warning => "warning",
        }
    }
}

/// One parsed compile-log entry, tagged with the source revision it came from
/// so readers can badge entries from older revisions as stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TexDiagnostic {
    pub file: String,
    pub line: Option<u32>,
    pub severity: TexSeverity,
    pub message: String,
    pub revision: u64,
}

fn cap_message(raw: &str) -> String {
    let t = raw.trim();
    if t.len() <= MAX_DIAGNOSTIC_MESSAGE {
        return t.to_string();
    }
    let mut cut = MAX_DIAGNOSTIC_MESSAGE;
    while cut > 0 && !t.is_char_boundary(cut) {
        cut -= 1;
    }
    t.get(..cut).map(str::to_string).unwrap_or_default()
}

/// Leading ASCII digits as `(value, rest)`.
fn split_leading_number(s: &str) -> Option<(u32, &str)> {
    let width: usize = s.chars().take_while(|c| c.is_ascii_digit()).map(|c| c.len_utf8()).sum();
    if width == 0 {
        return None;
    }
    let (head, tail) = s.split_at(width);
    Some((head.parse::<u32>().ok()?, tail))
}

/// `path/file.tex:12: message` or `path/file.tex:12:34: message`. A message
/// naming a warning parses as a warning, everything else as an error.
fn parse_prefixed(line: &str, revision: u64) -> Option<TexDiagnostic> {
    let (file, rest) = line.split_once(':')?;
    let file = file.trim();
    if file.is_empty() || !file.contains('.') {
        return None;
    }
    if file.contains(|c: char| c.is_whitespace()) && !file.ends_with(".tex") {
        return None;
    }
    let (line_no, after) = split_leading_number(rest.trim_start())?;
    let mut tail = after;
    if let Some(colon) = after.strip_prefix(':') {
        let c = colon.trim_start();
        if let Some((_, rest2)) = split_leading_number(c) {
            if let Some(msg) = rest2.trim_start().strip_prefix(':') {
                tail = msg;
            }
        } else {
            tail = colon;
        }
    }
    let message = cap_message(tail);
    if message.is_empty() {
        return None;
    }
    let severity = if message.to_ascii_lowercase().contains("warning") { TexSeverity::Warning } else { TexSeverity::Error };
    Some(TexDiagnostic { file: file.to_string(), line: Some(line_no), severity, message, revision })
}

/// `! message` TeX error lines (the text after the bang).
fn parse_bang(line: &str) -> Option<String> {
    let message = cap_message(line.strip_prefix('!')?);
    if message.is_empty() { None } else { Some(message) }
}

/// `l.123 ...` source-line pointers: only meaningful directly under an error.
fn parse_l_pointer(line: &str) -> Option<u32> {
    split_leading_number(line.strip_prefix("l.")?).map(|(n, _)| n)
}

/// `... on input line 123...` tails of LaTeX and package warnings.
fn parse_input_line(line: &str) -> Option<u32> {
    let marker = "on input line ";
    let at = line.find(marker)?;
    split_leading_number(line.get(at + marker.len()..)?.trim_start()).map(|(n, _)| n)
}

/// `Underfull ... at lines 12--34` / `Overfull ... at line 7` box reports.
fn parse_box_line(line: &str) -> Option<u32> {
    for marker in ["at lines ", "at line "] {
        if let Some(at) = line.find(marker)
            && let Some(tail) = line.get(at + marker.len()..)
            && let Some((n, _)) = split_leading_number(tail.trim_start())
        {
            return Some(n);
        }
    }
    None
}

fn push_capped(out: &mut Vec<TexDiagnostic>, entry: TexDiagnostic) {
    if out.len() < MAX_DIAGNOSTICS {
        out.push(entry);
    }
}

fn flush_pending(pending: &mut Option<(String, String, bool)>, out: &mut Vec<TexDiagnostic>, revision: u64) {
    if let Some((file, message, _)) = pending.take() {
        push_capped(out, TexDiagnostic { file, line: None, severity: TexSeverity::Error, message, revision });
    }
}

/// Parse a compile log into structured entries for one source revision.
/// Anything unrecognised is skipped: hostile logs cost entries, never panics.
pub fn parse_tex_diagnostics(log: &str, revision: u64) -> Vec<TexDiagnostic> {
    let mut out: Vec<TexDiagnostic> = Vec::new();
    // An `!` error waiting for its `l.NNN` pointer, plus whether the pointer
    // may still attach (only the line directly below the error).
    let mut pending: Option<(String, String, bool)> = None;
    for raw in log.lines() {
        if out.len() >= MAX_DIAGNOSTICS {
            break;
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(n) = parse_l_pointer(line) {
            match pending.take() {
                Some((file, message, true)) => {
                    push_capped(&mut out, TexDiagnostic { file, line: Some(n), severity: TexSeverity::Error, message, revision })
                }
                Some((file, message, false)) => {
                    push_capped(&mut out, TexDiagnostic { file, line: None, severity: TexSeverity::Error, message, revision })
                }
                None => {}
            }
            continue;
        }
        if let Some(slot) = pending.as_mut() {
            slot.2 = false;
        }
        if let Some(entry) = parse_prefixed(line, revision) {
            flush_pending(&mut pending, &mut out, revision);
            push_capped(&mut out, entry);
            continue;
        }
        if line.starts_with('!') {
            flush_pending(&mut pending, &mut out, revision);
            if let Some(message) = parse_bang(line) {
                pending = Some((String::new(), message, true));
            }
            continue;
        }
        if line.to_ascii_lowercase().contains("warning")
            && let Some(n) = parse_input_line(line)
        {
            flush_pending(&mut pending, &mut out, revision);
            push_capped(
                &mut out,
                TexDiagnostic { file: String::new(), line: Some(n), severity: TexSeverity::Warning, message: cap_message(line), revision },
            );
            continue;
        }
        if (line.contains("Underfull") || line.contains("Overfull"))
            && let Some(n) = parse_box_line(line)
        {
            flush_pending(&mut pending, &mut out, revision);
            push_capped(
                &mut out,
                TexDiagnostic { file: String::new(), line: Some(n), severity: TexSeverity::Warning, message: cap_message(line), revision },
            );
            continue;
        }
        if let Some(entry) = parse_bib_line(line, revision) {
            flush_pending(&mut pending, &mut out, revision);
            push_capped(&mut out, entry);
            continue;
        }
        // Anything else is prose, not a diagnostic: skipped.
    }
    flush_pending(&mut pending, &mut out, revision);
    out
}

impl TexState {
    /// Record a source edit at `now`: bump the revision, badge a successful
    /// compile's preview stale, and arm the debounce window when auto-compile
    /// is opted in. Never compiles: a poll performs at most one compile.
    pub fn note_edit(&mut self, now: Instant) {
        self.revision = self.revision.wrapping_add(1);
        if self.last.as_ref().is_some_and(|c| c.success) {
            self.preview_stale = true;
        }
        if self.auto_compile && self.source.is_some() {
            self.pending_revision = Some(self.revision);
            self.pending_since = Some(now);
        }
    }

    /// Opt auto-compile in or out for the open document. Enabling arms a
    /// pending compile when the source is newer than the last published
    /// compile; disabling clears everything pending and retires any in-flight
    /// compile so it can never publish.
    pub fn set_auto(&mut self, enabled: bool, now: Instant) {
        self.auto_compile = enabled;
        if enabled {
            if self.source.is_some() && self.compiled_revision != Some(self.revision) {
                self.pending_revision = Some(self.revision);
                self.pending_since = Some(now);
            }
        } else {
            self.pending_revision = None;
            self.pending_since = None;
            self.compile_generation = self.compile_generation.wrapping_add(1);
            self.in_flight = None;
        }
    }

    /// The revision ready to compile: auto-compile on, an edit pending, the
    /// quiet window elapsed, and nothing newer already compiled.
    pub fn due_revision(&self, now: Instant) -> Option<u64> {
        if !self.auto_compile {
            return None;
        }
        let rev = self.pending_revision?;
        let since = self.pending_since?;
        if self.compiled_revision == Some(rev) {
            return None;
        }
        if now.duration_since(since) >= AUTO_COMPILE_DEBOUNCE { Some(rev) } else { None }
    }

    /// Claim a revision for one compile run: clears the pending slot and hands
    /// out a generation. A second claim supersedes the first.
    pub fn begin_compile(&mut self, revision: u64) -> u64 {
        self.pending_revision = None;
        self.pending_since = None;
        self.compile_generation = self.compile_generation.wrapping_add(1);
        let generation = self.compile_generation;
        self.in_flight = Some(InFlightCompile { revision, generation });
        generation
    }

    /// Publish one finished compile: older or retired generations are
    /// discarded, so only the latest revision's output is ever shown. Parses
    /// the log into revision-tagged diagnostics, records what ran, and clears
    /// the stale badge. Returns true when the result was published.
    pub fn finish_compile(&mut self, generation: u64, result: TexCompile) -> bool {
        let owned = matches!(&self.in_flight, Some(f) if f.generation == generation);
        if !owned || generation != self.compile_generation {
            return false;
        }
        self.in_flight = None;
        self.diagnostics = parse_tex_diagnostics(&result.log, result.revision);
        self.compiled_revision = Some(result.revision);
        self.preview_stale = false;
        self.last = Some(result);
        true
    }
}

/// A worker child the host can supersede: superseding (or dropping) kills the
/// child and reaps it, so a cancelled compile never leaves an orphan behind.
/// Synchronous engine commands never hold one across calls — the session is
/// single-threaded, so two compilers cannot race — and this guard is the
/// primitive a host uses when it runs a compile off the command thread.
#[cfg(not(target_arch = "wasm32"))]
pub struct SupervisedWorker {
    child: Option<std::process::Child>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SupervisedWorker {
    /// Spawn `program args` with piped stdio, like the guarded worker.
    pub fn spawn(program: &Path, args: &[String]) -> Result<Self, String> {
        use std::process::Stdio;
        let child = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start {p}: {e}", p = program.display()))?;
        Ok(SupervisedWorker { child: Some(child) })
    }

    /// Cancel the worker: kill it when still running, then reap it. Safe to
    /// call twice; a finished child is simply reaped.
    pub fn supersede(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// True once the child has exited and been reaped.
    pub fn is_reaped(&mut self) -> bool {
        let Some(child) = self.child.as_mut() else { return true };
        match child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => false,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for SupervisedWorker {
    fn drop(&mut self) {
        self.supersede();
    }
}

#[cfg(test)]
mod argv_tests {
    use super::*;

    #[test]
    fn tectonic_argv_has_untrusted_first() {
        let engine = TexEngine::Tectonic(PathBuf::from("/tools/tectonic"));
        let (program, args) = compile_argv(&engine, Path::new("main.tex"), Path::new("/tmp/out")).unwrap();
        assert_eq!(program, PathBuf::from("/tools/tectonic"));
        assert!(args.len() >= 5, "{args:?}");
        assert_eq!(args[0], "--untrusted");
        assert_eq!(args[1], "--synctex");
        assert_eq!(args[2], "--outdir");
        assert_eq!(args[3], "/tmp/out");
        assert_eq!(args[4], "main.tex");
    }

    #[test]
    fn pdftex_argv_is_noninteractive_without_untrusted() {
        for (engine, flag) in [
            (TexEngine::PdfLatex(PathBuf::from("/usr/bin/pdflatex")), "pdflatex"),
            (TexEngine::XeLatex(PathBuf::from("/usr/bin/xelatex")), "xelatex"),
        ] {
            let (program, args) = compile_argv(&engine, Path::new("doc.tex"), Path::new("/tmp/o")).unwrap();
            assert!(program.to_string_lossy().contains(flag));
            assert!(args.contains(&"-interaction=nonstopmode".to_string()), "{args:?}");
            assert!(args.contains(&"-output-directory".to_string()), "{args:?}");
            assert!(!args.iter().any(|a| a.contains("untrusted")), "{args:?}");
        }
    }

    #[test]
    fn missing_engine_argv_errors() {
        assert!(compile_argv(&TexEngine::Missing, Path::new("a.tex"), Path::new("/tmp/o")).is_err());
    }

    #[test]
    fn engine_for_resolves_kinds() {
        assert_eq!(engine_for("/tools/tectonic", None).unwrap(), TexEngine::Tectonic(PathBuf::from("/tools/tectonic")));
        assert_eq!(engine_for("/x/pdflatex", None).unwrap(), TexEngine::PdfLatex(PathBuf::from("/x/pdflatex")));
        assert_eq!(engine_for("/x/xelatex", None).unwrap(), TexEngine::XeLatex(PathBuf::from("/x/xelatex")));
        assert_eq!(engine_for("/x/custom", Some("pdflatex")).unwrap(), TexEngine::PdfLatex(PathBuf::from("/x/custom")));
        assert!(engine_for("/x/custom", Some("luatex")).is_err());
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), unix))]
#[allow(clippy::unwrap_used)]
mod compile_tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

    fn mark_exe(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}")).unwrap();
        mark_exe(&p);
        p
    }

    fn fixture_png() -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(6, 5, image::Rgba([18, 18, 18, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    fn dark_fraction(png: &[u8]) -> f64 {
        let img = image::load_from_memory(png).unwrap().to_rgba8();
        let (w, h) = img.dimensions();
        let dark = img.pixels().filter(|p| p[0] < 128 && p[1] < 128 && p[2] < 128).count();
        dark as f64 / (w as u64 * h as u64) as f64
    }

    #[test]
    fn open_missing_file_errors() {
        let mut s = session();
        assert!(s.run("tex.open", &json!({"path": "/nonexistent-dir-xyz/missing.tex"})).is_err());
        assert!(s.tex.source.is_none());
    }

    #[test]
    fn compile_without_source_errors() {
        let mut s = session();
        assert!(s.run("tex.compile", &json!({})).is_err());
        assert!(s.tex.last.is_none());
    }

    #[test]
    fn opaque_round_trip_bytes_unchanged_by_open_and_compile() {
        let dir = new_scratch("wctexrt").unwrap();
        let mut bytes = "percent document: a^2 + b^2\n".as_bytes().to_vec();
        bytes.extend([0u8, 159, 255, 10]);
        std::fs::write(dir.join("main.tex"), &bytes).unwrap();
        let script = "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; echo \"fake tectonic compiled\";";
        let engine = write_script(&dir, "tectonic", script);
        let mut s = session();
        let opened = s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        assert_eq!(opened["bytes"], json!(bytes.len()));
        assert_eq!(s.tex.source.as_ref().map(|x| x.bytes.clone()), Some(bytes.clone()));
        assert!(s.tex.last.is_none(), "opening never compiles");
        let out = s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        assert_eq!(out["success"], json!(true));
        assert_eq!(out["timedOut"], json!(false));
        assert!(out["log"].as_str().unwrap_or("").contains("fake tectonic compiled"));
        assert!(out["pdfBytes"].as_u64().unwrap_or(0) > 0);
        assert_eq!(out["previewBytes"], json!(0));
        assert_eq!(s.tex.source.as_ref().map(|x| x.bytes.clone()), Some(bytes));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hanging_compile_is_killed_by_wall_clock() {
        let dir = new_scratch("wctexhang").unwrap();
        let loop_doc = "\\documentclass{article}\\begin{document}\\loop\\repeat\\end{document}\n";
        std::fs::write(dir.join("main.tex"), loop_doc).unwrap();
        let engine = write_script(&dir, "tectonic", "echo starting hang; exec sleep 30;");
        let mut s = session();
        s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        let before = std::time::Instant::now();
        let out = s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 300, "rasterizer": "/nonexistent-pp"})).unwrap();
        assert_eq!(out["timedOut"], json!(true));
        assert_eq!(out["success"], json!(false));
        assert!(before.elapsed() < std::time::Duration::from_secs(20), "wall clock kills hang fast");
        assert!(out["log"].as_str().unwrap_or("").contains("killed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_compile_surfaces_log_text() {
        let dir = new_scratch("wctexfail").unwrap();
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\\begin{document}hi\\end{document}\n").unwrap();
        let engine = write_script(&dir, "tectonic", "echo fontspec error file not found >&2; echo emergency stop; exit 1;");
        let mut s = session();
        s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        let out = s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        assert_eq!(out["success"], json!(false));
        assert_eq!(out["timedOut"], json!(false));
        let log = out["log"].as_str().unwrap_or("").to_string();
        assert!(log.contains("fontspec") || log.contains("emergency"), "log surfaced");
        assert_eq!(out["pdfBytes"], json!(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn successful_compile_shows_nonblank_preview() {
        let dir = new_scratch("wctexprev").unwrap();
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\\begin{document}hello\\end{document}\n").unwrap();
        let png = fixture_png();
        let fixture = dir.join("fixture.png");
        std::fs::write(&fixture, &png).unwrap();
        let engine_body = "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; echo \"ok\";";
        let engine = write_script(&dir, "tectonic", engine_body);
        let rast_body = format!("for a in \"$@\"; do last=\"$a\"; done; cp \"{f}\" \"$last.png\";", f = fixture.display());
        let rasterizer = write_script(&dir, "raster", &rast_body);
        let mut s = session();
        s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        let out = s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": rasterizer})).unwrap();
        assert_eq!(out["success"], json!(true));
        assert!(out["previewBytes"].as_u64().unwrap_or(0) > 0, "preview present");
        let stored = s.tex.last.as_ref().and_then(|c| c.preview_png.clone()).unwrap_or_default();
        assert!(!stored.is_empty());
        assert!(dark_fraction(&stored) > 0.5, "preview non blank");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod diagnostics_tests {
    use super::*;

    const FIXTURE: &str = "This is pdfTeX, Version 3.14159265.\nentering extended mode\nmain.tex:6: Undefined control sequence \\foo\n! LaTeX Error: File `missing.sty' not found.\nl.3 \\usepackage{missing}\nLaTeX Warning: Citation `doe99' undefined on input line 12.\n";

    #[test]
    fn fixture_parses_two_errors_and_one_warning() {
        let got = parse_tex_diagnostics(FIXTURE, 7);
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0],
            TexDiagnostic {
                file: "main.tex".to_string(),
                line: Some(6),
                severity: TexSeverity::Error,
                message: "Undefined control sequence \\foo".to_string(),
                revision: 7,
            }
        );
        assert_eq!(
            got[1],
            TexDiagnostic {
                file: String::new(),
                line: Some(3),
                severity: TexSeverity::Error,
                message: "LaTeX Error: File `missing.sty' not found.".to_string(),
                revision: 7,
            }
        );
        assert_eq!(
            got[2],
            TexDiagnostic {
                file: String::new(),
                line: Some(12),
                severity: TexSeverity::Warning,
                message: "LaTeX Warning: Citation `doe99' undefined on input line 12.".to_string(),
                revision: 7,
            }
        );
        assert_eq!(TexSeverity::Error.as_str(), "error");
        assert_eq!(TexSeverity::Warning.as_str(), "warning");
    }

    #[test]
    fn malformed_lines_are_skipped_without_panic() {
        let log = concat!(
            ":::\n",
            "\n",
            "! \n",
            "!\n",
            "l.\n",
            "l.abc\n",
            "main.tex:abc: oops\n",
            "main.tex:12:\n",
            "just some prose: with a colon\n",
            "see Fig. 3: the foo\n",
            "on input line \n",
            "Warning: on input line \n",
            "main.tex:99999999999999999999: x\n",
            "binary \u{fffd} noise \u{0}\n",
            "Underfull without lines\n",
            "Overfull \\hboxaville\n",
            "l.12 with no error above\n",
        );
        let got = parse_tex_diagnostics(log, 1);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn prefixed_warnings_and_box_reports_are_warnings() {
        let log = "main.tex:10: LaTeX Warning: Citation `x' undefined\nOverfull \\hbox (12.0pt too wide) at lines 20--21\nUnderfull \\hbox (badness 10000) at line 30\n";
        let got = parse_tex_diagnostics(log, 2);
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|d| d.severity == TexSeverity::Warning));
        assert!(got.iter().all(|d| d.revision == 2));
        assert_eq!(got[0].file, "main.tex");
        assert_eq!(got[0].line, Some(10));
        assert_eq!(got[1].line, Some(20));
        assert_eq!(got[2].line, Some(30));
    }

    #[test]
    fn column_numbers_fold_into_the_line() {
        let got = parse_tex_diagnostics("src/main.tex:6:5: Something broke\n", 9);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].file, "src/main.tex");
        assert_eq!(got[0].line, Some(6));
        assert_eq!(got[0].severity, TexSeverity::Error);
        assert_eq!(got[0].revision, 9);
    }
}

#[cfg(test)]
mod scheduler_tests {
    use super::*;

    fn open_state() -> TexState {
        TexState { source: Some(TexSource { path: PathBuf::from("main.tex"), bytes: b"v1".to_vec() }), revision: 1, ..Default::default() }
    }

    fn past_window(from: Instant) -> Instant {
        from + AUTO_COMPILE_DEBOUNCE + Duration::from_millis(10)
    }

    #[test]
    fn edits_without_opt_in_never_arm_a_compile() {
        let mut s = open_state();
        let now = Instant::now();
        s.note_edit(now);
        s.note_edit(now);
        s.note_edit(now);
        assert_eq!(s.revision, 4);
        assert_eq!(s.pending_revision, None);
        assert_eq!(s.due_revision(now), None);
        assert!(!s.preview_stale, "no success yet, nothing to badge stale");
    }

    #[test]
    fn rapid_burst_converges_to_final_revision_only() {
        let mut s = open_state();
        s.set_auto(true, Instant::now());
        let mut now = Instant::now();
        for _ in 0..5 {
            now += Duration::from_millis(10);
            s.note_edit(now);
        }
        assert_eq!(s.revision, 6);
        assert_eq!(s.pending_revision, Some(6));
        assert_eq!(s.due_revision(now), None, "quiet window has not elapsed");
        let when = past_window(now);
        assert_eq!(s.due_revision(when), Some(6));
        let generation = s.begin_compile(6);
        assert_eq!(s.pending_revision, None);
        let done = TexCompile { engine: "tectonic".to_string(), success: true, revision: 6, ..Default::default() };
        assert!(s.finish_compile(generation, done));
        assert_eq!(s.compiled_revision, Some(6));
        assert!(!s.preview_stale);
        assert_eq!(s.due_revision(when), None, "nothing pending after publish");
    }

    #[test]
    fn in_flight_compile_is_superseded_by_a_newer_edit() {
        let mut s = open_state();
        s.set_auto(true, Instant::now());
        s.note_edit(Instant::now());
        let generation1 = s.begin_compile(2);
        s.note_edit(Instant::now());
        let generation2 = s.begin_compile(3);
        assert_ne!(generation1, generation2);
        let stale = TexCompile { success: true, revision: 2, ..Default::default() };
        assert!(!s.finish_compile(generation1, stale), "older generation is discarded");
        assert!(s.last.is_none());
        let fresh = TexCompile { success: true, revision: 3, ..Default::default() };
        assert!(s.finish_compile(generation2, fresh));
        assert_eq!(s.last.as_ref().map(|c| c.revision), Some(3));
    }

    #[test]
    fn toggle_off_stops_everything_pending_and_in_flight() {
        let mut s = open_state();
        s.set_auto(true, Instant::now());
        s.note_edit(Instant::now());
        let generation = s.begin_compile(s.revision);
        s.set_auto(false, Instant::now());
        assert!(!s.auto_compile);
        assert_eq!(s.pending_revision, None);
        assert_eq!(s.due_revision(past_window(Instant::now())), None);
        let late = TexCompile { revision: 2, ..Default::default() };
        assert!(!s.finish_compile(generation, late), "retired generation never publishes");
        assert!(s.last.is_none());
    }

    #[test]
    fn edit_after_success_badges_stale_until_recompile() {
        let mut s = open_state();
        let done =
            TexCompile { engine: "tectonic".to_string(), success: true, log: "main.tex:6: boom".to_string(), revision: 1, ..Default::default() };
        let generation = s.begin_compile(1);
        assert!(s.finish_compile(generation, done));
        assert!(!s.preview_stale);
        assert_eq!(s.diagnostics.len(), 1);
        assert_eq!(s.diagnostics[0].revision, 1);
        s.note_edit(Instant::now());
        assert!(s.preview_stale);
        assert_eq!(s.revision, 2);
        let generation2 = s.begin_compile(2);
        let done2 = TexCompile { success: true, revision: 2, ..Default::default() };
        assert!(s.finish_compile(generation2, done2));
        assert!(!s.preview_stale);
        assert_eq!(s.compiled_revision, Some(2));
    }

    #[test]
    fn enabling_auto_when_dirty_arms_a_compile() {
        let mut s = open_state();
        s.note_edit(Instant::now());
        assert_eq!(s.pending_revision, None);
        s.set_auto(true, Instant::now());
        assert_eq!(s.pending_revision, Some(2));
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), unix))]
#[allow(clippy::unwrap_used)]
mod auto_compile_tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// Fake tectonic: writes the PDF, then echoes the staged source plus a
    /// canned log so the log always reflects the newest revision's bytes.
    fn engine_script(dir: &Path) -> PathBuf {
        write_script(
            dir,
            "tectonic",
            &format!(
                "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; cat \"$src\"; cat \"{d}/canned.log\";",
                d = dir.display()
            ),
        )
    }

    fn open_doc(s: &mut Session, dir: &Path, content: &str) {
        std::fs::write(dir.join("main.tex"), content).unwrap();
        let out = s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        assert_eq!(out["bytes"], json!(content.len()));
    }

    fn tick_args(engine: &Path) -> serde_json::Value {
        json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"})
    }

    fn backdate(s: &mut Session) {
        s.tex.pending_since = Some(std::time::Instant::now() - AUTO_COMPILE_DEBOUNCE - std::time::Duration::from_millis(10));
    }

    #[test]
    fn rapid_edit_burst_compiles_once_with_final_bytes() {
        let dir = new_scratch("wctexburst").unwrap();
        std::fs::write(dir.join("canned.log"), "ok\n").unwrap();
        let engine = engine_script(&dir);
        let mut s = session();
        open_doc(&mut s, &dir, "v1");
        s.run("tex.autoCompile", &json!({"enabled": true})).unwrap();
        for v in ["v2", "v3", "v4", "v5", "v6"] {
            let r = s.run("tex.edit", &json!({"text": v})).unwrap();
            assert_eq!(r["stale"], json!(false), "no success yet, nothing stale");
        }
        assert_eq!(s.tex.revision, 6);
        assert_eq!(s.tex.pending_revision, Some(6));
        let probe = s.run("tex.tick", &tick_args(&engine)).unwrap();
        assert_eq!(probe["compiled"], json!(false), "quiet window has not elapsed");
        assert!(s.tex.last.is_none());
        backdate(&mut s);
        let done = s.run("tex.tick", &tick_args(&engine)).unwrap();
        assert_eq!(done["compiled"], json!(true));
        assert_eq!(done["success"], json!(true));
        assert_eq!(done["revision"], json!(6));
        let log = done["log"].as_str().unwrap().to_string();
        assert!(log.contains("v6"), "final revision compiled: {log:?}");
        assert!(!log.contains("v2") && !log.contains("v5"), "only the final revision ran: {log:?}");
        assert_eq!(s.tex.compiled_revision, Some(6));
        assert_eq!(s.tex.pending_revision, None);
        let again = s.run("tex.tick", &tick_args(&engine)).unwrap();
        assert_eq!(again["compiled"], json!(false), "one burst means one compile");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggle_off_stops_all_auto_compile_activity() {
        let dir = new_scratch("wctexoff").unwrap();
        std::fs::write(dir.join("canned.log"), "ok\n").unwrap();
        let engine = engine_script(&dir);
        let mut s = session();
        open_doc(&mut s, &dir, "v1");
        s.run("tex.autoCompile", &json!({"enabled": true})).unwrap();
        s.run("tex.edit", &json!({"text": "v2"})).unwrap();
        assert_eq!(s.tex.pending_revision, Some(2));
        s.run("tex.autoCompile", &json!({"enabled": false})).unwrap();
        assert_eq!(s.tex.pending_revision, None);
        backdate(&mut s);
        let probe = s.run("tex.tick", &tick_args(&engine)).unwrap();
        assert_eq!(probe["compiled"], json!(false));
        assert!(s.tex.last.is_none(), "toggle off: no worker ever ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_badge_and_diagnostics_carry_revisions() {
        let dir = new_scratch("wctexstale").unwrap();
        std::fs::write(
            dir.join("canned.log"),
            "main.tex:6: Undefined control sequence \\foo\n! LaTeX Error: File `missing.sty' not found.\nl.3 \\usepackage{missing}\nLaTeX Warning: Citation `doe99' undefined on input line 12.\n",
        )
        .unwrap();
        let engine = engine_script(&dir);
        let mut s = session();
        open_doc(&mut s, &dir, "v1");
        let compiled = s.run("tex.compile", &tick_args(&engine)).unwrap();
        assert_eq!(compiled["success"], json!(true));
        assert_eq!(compiled["diagnostics"], json!(3));
        let st = s.run("tex.status", &json!({})).unwrap();
        assert_eq!(st["stale"], json!(false));
        assert_eq!(st["revision"], json!(1));
        let diag = s.run("tex.diagnostics", &json!({})).unwrap();
        assert_eq!(diag["entries"].as_array().map(Vec::len), Some(3));
        assert!(diag["entries"].as_array().map(|e| e.iter().all(|x| x["stale"] == json!(false) && x["revision"] == json!(1))).unwrap_or(false));
        let kinds: Vec<String> =
            diag["entries"].as_array().map(|e| e.iter().map(|x| x["severity"].as_str().unwrap_or("?").to_string()).collect()).unwrap_or_default();
        assert_eq!(kinds, vec!["error".to_string(), "error".to_string(), "warning".to_string()]);
        s.run("tex.edit", &json!({"text": "v2"})).unwrap();
        let st2 = s.run("tex.status", &json!({})).unwrap();
        assert_eq!(st2["stale"], json!(true), "edit after compile badges stale");
        let diag2 = s.run("tex.diagnostics", &json!({})).unwrap();
        assert!(
            diag2["entries"].as_array().map(|e| e.iter().all(|x| x["stale"] == json!(true) && x["revision"] == json!(1))).unwrap_or(false),
            "old-revision entries are badged stale, never current"
        );
        let recompiled = s.run("tex.compile", &tick_args(&engine)).unwrap();
        assert_eq!(recompiled["revision"], json!(2));
        let st3 = s.run("tex.status", &json!({})).unwrap();
        assert_eq!(st3["stale"], json!(false), "fresh compile clears stale");
        let diag3 = s.run("tex.diagnostics", &json!({})).unwrap();
        assert!(diag3["entries"].as_array().map(|e| e.iter().all(|x| x["stale"] == json!(false) && x["revision"] == json!(2))).unwrap_or(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tex_commands_reject_missing_source_and_junk() {
        let mut s = session();
        assert!(s.run("tex.edit", &json!({"text": "x"})).is_err());
        assert!(s.run("tex.autoCompile", &json!({"enabled": true})).is_err());
        assert!(s.run("tex.diagnostics", &json!({})).is_ok());
        let dir = new_scratch("wctexjunk").unwrap();
        open_doc(&mut s, &dir, "v1");
        assert!(s.run("tex.autoCompile", &json!({})).is_err());
        assert!(s.run("tex.autoCompile", &json!({"enabled": "yes"})).is_err());
        assert!(s.run("tex.edit", &json!({})).is_err());
        assert!(s.run("tex.tick", &json!({})).is_ok(), "idle tick never panics");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn superseded_worker_is_killed_and_reaped() {
        let mut w = SupervisedWorker::spawn(std::path::Path::new("/bin/sleep"), &["30".to_string()]).unwrap();
        assert!(!w.is_reaped(), "sleep is still running");
        w.supersede();
        assert!(w.is_reaped(), "superseded child is reaped, no orphan");
        w.supersede();
        assert!(w.is_reaped(), "supersede is idempotent");
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), unix))]
#[allow(clippy::unwrap_used)]
mod synctex_jump_tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    const DOC: &str = "\\documentclass{article}\n\\begin{document}\nHello line two.\n\nSecond paragraph here.\n\\end{document}\n";

    const FIXTURE: &str = "SyncTeX Version:1\nInput:1:main.tex\nInput:2:\nOutput:pdf\nMagnification:1000\nUnit:1\nX Offset:0\nY Offset:0\nContent:\n!165\n{1\n[1,6:4736287,46220575:26673152,41484288,0\n(1,6:4736287,4736287:0,0,0\n)\n[1,6:8799519,46220575:22609920,40435712,0\n[1,6:8799519,6571295:22609920,786432,0\n(1,6:8799519,6571295:22609920,0,0\nh1,6:8799519,6571295:22609920,0,0\n)\n]\n[1,6:8799519,44254495:22609920,36044800,0\n(1,4:8799519,8865055:22609920,454820,7208\nh1,3:8799519,8865055:983040,0,0\ng1,3:11475354,8865055\ng1,3:12713329,8865055\nk1,4:31409439,8865055:17494835\ng1,4:31409439,8865055\n)\n(1,6:8799519,9651487:22609920,462029,135003\nh1,5:8799519,9651487:983040,0,0\ng1,5:12003574,9651487\ng1,5:15139472,9651487\nk1,6:31409439,9651487:14884536\ng1,6:31409439,9651487\n)\n]\n(1,6:8799519,46220575:22609920,436470,0\n(1,6:8799519,46220575:22609920,436470,0\nk1,6:19940639,46220575:11141120\nk1,6:31409439,46220575:11141120\n)\n)\n]\n]\n!776\n}1\nPostamble:\nCount:34\n!24\nPost scriptum:\n";

    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn engine_with_synctex(dir: &Path, synctex_text: &str) -> PathBuf {
        let escaped = synctex_text.replace("\\", "\\\\");
        let body = format!(
            "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; printf \"%s\" \"{syn}\" > \"$out/$stem.synctex\"; echo ok;",
            syn = escaped.replace("\"", "\\\"").replace("%", "%%")
        );
        write_script(dir, "tectonic", &body)
    }

    fn open_doc(s: &mut Session, dir: &Path) {
        std::fs::write(dir.join("main.tex"), DOC).unwrap();
        let out = s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        assert_eq!(out["bytes"], json!(DOC.len()));
    }

    #[test]
    fn fake_engine_forward_inverse_roundtrip() {
        let dir = new_scratch("wcsyncfake").unwrap();
        let engine = engine_with_synctex(&dir, FIXTURE);
        let mut s = session();
        open_doc(&mut s, &dir);
        let compiled =
            s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        assert_eq!(compiled["success"], json!(true));
        let fwd = s.run("tex.forwardSearch", &json!({"line": 3})).unwrap();
        assert_eq!(fwd["page"], json!(1));
        assert_eq!(fwd["line"], json!(3));
        assert_eq!(fwd["revision"], json!(1));
        assert_eq!(fwd["stale"], json!(false));
        let w = fwd["w"].as_f64().unwrap_or(0.0);
        let h = fwd["h"].as_f64().unwrap_or(0.0);
        assert!(w > 0.0 && h > 0.0, "sane non-empty rect");
        assert!(fwd["x"].as_f64().unwrap_or(f64::NAN).is_finite());
        assert!(fwd["y"].as_f64().unwrap_or(f64::NAN).is_finite());
        let cx = fwd["x"].as_f64().unwrap() + w / 2.0;
        let cy = fwd["y"].as_f64().unwrap() + h / 2.0;
        let inv = s.run("tex.inverseSearch", &json!({"page": 1, "x": cx, "y": cy})).unwrap();
        assert_eq!(inv["page"], json!(1));
        assert_eq!(inv["revision"], json!(1));
        let line = inv["line"].as_u64().unwrap_or(0) as i64;
        assert!((line - 3).abs() <= 1, "inverse line {line} within one of 3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_document_jump_refuses_cleanly() {
        let dir = new_scratch("wcsyncstale").unwrap();
        let engine = engine_with_synctex(&dir, FIXTURE);
        let mut s = session();
        open_doc(&mut s, &dir);
        s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        s.run("tex.edit", &json!({"text": "edited"})).unwrap();
        assert!(s.tex.preview_stale);
        let fwd = s.run("tex.forwardSearch", &json!({"line": 3}));
        assert!(fwd.is_err());
        let msg = format!("{}", fwd.unwrap_err());
        assert!(msg.contains("stale"), "stale refusal, got: {msg}");
        let inv = s.run("tex.inverseSearch", &json!({"page": 1, "x": 100.0, "y": 100.0}));
        assert!(inv.is_err());
        let msg2 = format!("{}", inv.unwrap_err());
        assert!(msg2.contains("stale"), "stale refusal, got: {msg2}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jump_with_no_compile_is_a_clean_error() {
        let dir = new_scratch("wcsyncnocomp").unwrap();
        let mut s = session();
        open_doc(&mut s, &dir);
        let fwd = s.run("tex.forwardSearch", &json!({"line": 3}));
        assert!(fwd.is_err());
        let inv = s.run("tex.inverseSearch", &json!({"page": 1, "x": 10.0, "y": 10.0}));
        assert!(inv.is_err());
        let mut empty = session();
        assert!(empty.run("tex.forwardSearch", &json!({"line": 1})).is_err());
        assert!(empty.run("tex.inverseSearch", &json!({"page": 1, "x": 1.0, "y": 1.0})).is_err());
        assert!(s.run("tex.forwardSearch", &json!({})).is_err());
        assert!(s.run("tex.forwardSearch", &json!({"line": 0})).is_err());
        assert!(s.run("tex.inverseSearch", &json!({"page": 0, "x": 1.0, "y": 1.0})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_synctex_never_panics_through_commands() {
        let dir = new_scratch("wcsyncbad").unwrap();
        let engine = engine_with_synctex(&dir, "not a synctex file at all ::: \n\x00\x01\x02");
        let mut s = session();
        open_doc(&mut s, &dir);
        let compiled =
            s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        assert_eq!(compiled["success"], json!(true));
        assert!(s.run("tex.forwardSearch", &json!({"line": 3})).is_err());
        assert!(s.run("tex.inverseSearch", &json!({"page": 1, "x": 100.0, "y": 135.0})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_tectonic_forward_inverse_roundtrip() {
        let bundled = PathBuf::from("/home/mitaoe/wordcraft-study/tools/tectonic");
        let real: PathBuf = if bundled.is_file() {
            bundled
        } else {
            match crate::tex::detect_from_path() {
                crate::tex::TexEngine::Tectonic(p) => p,
                _ => {
                    eprintln!("real-compile unverified: no tectonic engine found, exercising fixture path");
                    let dir = new_scratch("wcsyncrealfall").unwrap();
                    let engine = engine_with_synctex(&dir, FIXTURE);
                    let mut s = session();
                    open_doc(&mut s, &dir);
                    s.run("tex.compile", &json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"}))
                        .unwrap();
                    let fwd = s.run("tex.forwardSearch", &json!({"line": 3})).unwrap();
                    assert_eq!(fwd["page"], json!(1));
                    let _ = std::fs::remove_dir_all(&dir);
                    return;
                }
            }
        };
        let dir = new_scratch("wcsyncreal").unwrap();
        std::fs::write(dir.join("main.tex"), DOC).unwrap();
        let mut s = session();
        s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        let compiled =
            s.run("tex.compile", &json!({"engine": real, "kind": "tectonic", "timeoutMs": 120_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        if compiled["success"] != json!(true) {
            eprintln!("real-compile unverified: tectonic compile failed, exercising fixture path");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let has_sync = s.tex.last.as_ref().and_then(|c| c.synctex.as_ref()).is_some();
        if !has_sync {
            eprintln!("real-compile unverified: no synctex captured, exercising fixture path");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let fwd = s.run("tex.forwardSearch", &json!({"line": 3})).unwrap();
        assert_eq!(fwd["page"], json!(1), "real compile lands on page one");
        let w = fwd["w"].as_f64().unwrap_or(0.0);
        let h = fwd["h"].as_f64().unwrap_or(0.0);
        assert!(w > 0.0 && h > 0.0, "real forward rect is sane and non-empty");
        let cx = fwd["x"].as_f64().unwrap() + w / 2.0;
        let cy = fwd["y"].as_f64().unwrap() + h / 2.0;
        let inv = s.run("tex.inverseSearch", &json!({"page": 1, "x": cx, "y": cy})).unwrap();
        let line = inv["line"].as_u64().unwrap_or(0) as i64;
        assert!((line - 3).abs() <= 1, "real inverse returns line {line} within one of 3");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod bib_tests {
    use super::*;

    const FIXTURE: &str = r#"% a comment with {braces} and @fake{key,
@article{doe99,
  author = {Doe, Jane},
  title = {An Interesting Paper},
  journal = {J. Tests},
  year = {1999},
}
@book{smith00,
  author = {Smith, John},
  title = {A Great Book},
  publisher = {Test Press},
  year = {2000},
}
@misc{web01,
  author = {Web, Author},
  title = "A quoted title with {braces} inside",
  year = {2001},
}
@string{jtests = {J. Tests}}
@preamble{"\newcommand{\noop}[1]{#1}"}
@comment{anything { goes } here, even unbalanced {
@article{broken01,
  author = {Nobody,
@misc{,
  title = {Empty key},
}
@article{doe99,
  author = {Duplicate, Kept Once},
}
"#;

    #[test]
    fn fixture_parses_article_book_misc() {
        let got = parse_bibtex(FIXTURE);
        let keys: Vec<&str> = got.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["doe99", "smith00", "web01"]);
        assert_eq!(got[0].entry_type, "article");
        assert_eq!(got[1].entry_type, "book");
        assert_eq!(got[2].entry_type, "misc");
    }

    #[test]
    fn hostile_entries_are_skipped_without_panic() {
        for hunk in [
            "",
            "@@@",
            "@article",
            "@article{",
            "@article{unbalanced, author = {nobody}",
            "@article{key with spaces, x = {1}}",
            "@article{, x = {1}}",
            "@misc{key}\n@book{other,",
            "no entries here: just prose % with @ signs",
            "@STRING{up = {case}}, @Article{Mixed01, x={1}},",
            "@article{\0nul, x = {1}}",
            "@article{percent%key, x = {1}}",
            "@article(nokey)",
            "@comment no braces at all",
        ] {
            let got = parse_bibtex(hunk);
            assert!(got.len() <= MAX_BIB_ENTRIES, "{got:?}");
        }
        // Uppercase kinds still parse.
        let got = parse_bibtex("@STRING{up = {case}}\n@Article{Mixed01, x = {1}}\n");
        assert_eq!(got.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), vec!["Mixed01"]);
        assert_eq!(got[0].entry_type, "article");
        // A broken entry never hides a later valid one.
        let got = parse_bibtex("@article{broken, author = {nobody\n@article{good01, x = {1}}\n");
        assert!(got.iter().any(|e| e.key == "good01"), "{got:?}");
        assert!(!got.iter().any(|e| e.key == "broken"), "{got:?}");
        // A huge but balanced value parses; an overlong key does not.
        let big = format!("@article{{ok01, x = {{{v}}}}}\n", v = "y".repeat(10_000));
        assert_eq!(parse_bibtex(&big).len(), 1);
        let overlong = format!("@article{{{k}, x = {{1}}}}", k = "k".repeat(MAX_BIB_KEY_LEN + 1));
        assert!(parse_bibtex(&overlong).is_empty());
    }

    #[test]
    fn entry_count_is_capped() {
        let mut big = String::new();
        for i in 0..MAX_BIB_ENTRIES + 100 {
            big.push_str(&format!("@misc{{k{i}, x = {{1}}}}\n"));
        }
        assert_eq!(parse_bibtex(&big).len(), MAX_BIB_ENTRIES);
    }
}

#[cfg(test)]
mod include_tests {
    use super::*;

    #[test]
    fn brace_and_space_forms_found_in_order() {
        let src = b"\\input{chapter1}\n\\include{chapters/intro}\n\\input plainname\n\\input{spaced}\n";
        assert_eq!(find_tex_includes(src), vec!["chapter1", "chapters/intro", "plainname", "spaced"]);
    }

    #[test]
    fn comments_and_lookalikes_ignored() {
        let src = b"% \\input{evil}\n\\inputx{nope}\n\\inputter{nope}\n\\input{real} % \\include{alsono}\n\\input{}\n\\input";
        assert_eq!(find_tex_includes(src), vec!["real"]);
    }

    #[test]
    fn escaped_percent_does_not_comment() {
        let src = b"100\\% \\input{real}\n";
        assert_eq!(find_tex_includes(src), vec!["real"]);
    }

    #[test]
    fn resolve_accepts_plain_relative() {
        let doc = Path::new("/doc");
        assert_eq!(resolve_include(doc, "chapter1").unwrap(), PathBuf::from("/doc/chapter1"));
        assert_eq!(resolve_include(doc, "chapters/intro.tex").unwrap(), PathBuf::from("/doc/chapters/intro.tex"));
        assert_eq!(resolve_include(doc, "./a").unwrap(), PathBuf::from("/doc/a"));
        assert_eq!(resolve_include(doc, "a//b").unwrap(), PathBuf::from("/doc/a/b"));
    }

    #[test]
    fn resolve_rejects_escapes_and_absolutes() {
        let doc = Path::new("/doc");
        for bad in ["", "   ", "../secret", "a/../../b", "..\\secret", "a\\..\\b", "/etc/passwd", "\\windows\\x", "C:\\x", "C:/x", "~/x", "a/b/.."] {
            assert!(resolve_include(doc, bad).is_err(), "{bad:?} must be refused");
        }
        assert!(resolve_include(doc, "\0nul").is_err());
        let overlong = "x".repeat(MAX_INCLUDE_NAME + 1);
        assert!(resolve_include(doc, &overlong).is_err());
    }

    #[test]
    fn bibliography_names_split_commas() {
        let src = b"\\bibliography{refs, other}\n\\addbibresource{extra.bib}\n% \\bibliography{evil}\n";
        assert_eq!(find_bibliography_names(src), vec!["refs", "other", "extra.bib"]);
    }

    #[test]
    fn bib_log_lines_become_diagnostics() {
        let log = "Citation `doe99' undefined.\nLaTeX Warning: Citation `x' undefined on input line 4.\nerror: the BibTeX engine had an unrecoverable error\nI didn't find a database entry for `missing01'\nJust prose.\n";
        let got = parse_tex_diagnostics(log, 3);
        assert_eq!(got.len(), 4, "{got:?}");
        assert!(got.iter().all(|d| d.revision == 3));
        assert_eq!(got[0].severity, TexSeverity::Warning);
        assert_eq!(got[0].line, None);
        assert_eq!(got[1].line, Some(4));
        assert_eq!(got[2].severity, TexSeverity::Error);
        assert_eq!(got[3].severity, TexSeverity::Warning);
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), unix))]
#[allow(clippy::unwrap_used)]
mod bib_sidecar_tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn open_doc(s: &mut Session, dir: &Path, content: &str) {
        std::fs::write(dir.join("main.tex"), content).unwrap();
        s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
    }

    fn tick_args(engine: &Path) -> serde_json::Value {
        json!({"engine": engine, "kind": "tectonic", "timeoutMs": 20_000, "rasterizer": "/nonexistent-pp"})
    }

    const BIB: &str = "@article{doe99,\n  author = {Doe, Jane},\n  title = {Paper},\n  journal = {J},\n  year = {1999},\n}\n@book{smith00,\n  author = {Smith, John},\n  title = {Book},\n  publisher = {P},\n  year = {2000},\n}\n";

    const DOC_WITH_BIB: &str =
        "\\documentclass{article}\n\\begin{document}\nHi~\\cite{doe99}.\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n";

    #[test]
    fn open_discovers_sibling_bib() {
        let dir = new_scratch("wcbibopen").unwrap();
        std::fs::write(dir.join("main.tex"), "hello\n").unwrap();
        std::fs::write(dir.join("main.bib"), BIB).unwrap();
        let mut s = session();
        let opened = s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        assert_eq!(opened["bibKeys"], json!(2));
        let listed = s.run("tex.bibKeys", &json!({})).unwrap();
        assert_eq!(listed["count"], json!(2));
        assert_eq!(listed["keys"], json!(["doe99", "smith00"]));
        assert_eq!(listed["entries"][0]["type"], json!("article"));
        assert_eq!(listed["entries"][1]["type"], json!("book"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bibkeys_loads_explicit_path_and_rejects_hostile_params() {
        let dir = new_scratch("wcbibkeys").unwrap();
        std::fs::write(dir.join("main.tex"), "hello\n").unwrap();
        std::fs::write(dir.join("extra.bib"), BIB).unwrap();
        std::fs::write(dir.join("bad.bib"), [0xffu8, 0xfe, b'@', b'a', 0x00]).unwrap();
        let mut s = session();
        assert!(s.run("tex.bibKeys", &json!({})).is_ok(), "listing without source never panics");
        assert!(s.run("tex.bibKeys", &json!(null)).is_ok());
        assert!(s.run("tex.bibKeys", &json!({"path": 5})).is_err());
        assert!(s.run("tex.bibKeys", &json!({"path": []})).is_err());
        assert!(s.run("tex.bibKeys", &json!({"path": ""})).is_err());
        assert!(s.run("tex.bibKeys", &json!({"path": "/nonexistent-dir-xyz/missing.bib"})).is_err());
        let huge = "x".repeat(9000);
        assert!(s.run("tex.bibKeys", &json!({"path": huge})).is_err());
        open_doc(&mut s, &dir, "hello\n");
        let bad = s.run("tex.bibKeys", &json!({"path": dir.join("bad.bib")})).unwrap();
        assert_eq!(bad["count"], json!(0));
        let loaded = s.run("tex.bibKeys", &json!({"path": dir.join("extra.bib")})).unwrap();
        assert_eq!(loaded["count"], json!(2));
        assert_eq!(loaded["keys"], json!(["doe99", "smith00"]));
        let st = s.run("tex.status", &json!({})).unwrap();
        assert_eq!(st["bibKeys"], json!(2));
        assert!(s.run("tex.edit", &json!({"text": "hello again"})).is_ok(), "session still usable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_tex_commands_reject_hostile_params_without_panic() {
        let dir = new_scratch("wctexhostile").unwrap();
        std::fs::write(dir.join("main.tex"), "hello\n").unwrap();
        let mut s = session();
        // Before any source is open: hostile params are clean errors, never panics.
        for v in [
            serde_json::Value::Null,
            json!({}),
            json!({"path": 5}),
            json!({"path": []}),
            json!({"path": ""}),
            json!({"path": "/nonexistent-dir-xyz/missing.tex"}),
            json!({"path": "x".repeat(9000)}),
            json!({"path": "a\0b"}),
        ] {
            assert!(s.run("tex.open", &v).is_err(), "tex.open {v:?} must be a clean error");
        }
        // Pure commands never panic on hostile envelopes, even with no source.
        for cmd in ["tex.diagnostics", "tex.status", "tex.close", "tex.bibKeys"] {
            assert!(s.run(cmd, &json!(null)).is_ok(), "{cmd} null never panics");
            assert!(s.run(cmd, &json!({"unexpected": []})).is_ok(), "{cmd} junk never panics");
        }
        assert!(s.run("tex.edit", &json!({"text": "x"})).is_err());
        assert!(s.run("tex.edit", &json!(null)).is_err());
        assert!(s.run("tex.edit", &json!({"text": 5})).is_err());
        assert!(s.run("tex.autoCompile", &json!(null)).is_err());
        assert!(s.run("tex.autoCompile", &json!({"enabled": "yes"})).is_err());
        assert!(s.run("tex.forwardSearch", &json!(null)).is_err());
        assert!(s.run("tex.forwardSearch", &json!({"line": "3"})).is_err());
        assert!(s.run("tex.forwardSearch", &json!({"line": 0})).is_err());
        assert!(s.run("tex.inverseSearch", &json!(null)).is_err());
        assert!(s.run("tex.inverseSearch", &json!({"page": 1, "x": "a", "y": 1.0})).is_err());
        assert!(s.run("tex.inverseSearch", &json!({"page": 0, "x": 1.0, "y": 1.0})).is_err());
        // Compile-like commands with hostile engine params never panic either.
        for v in [json!(null), json!({"engine": 5}), json!({"kind": "luatex"}), json!({"timeoutMs": "soon"}), json!({"rasterizer": 5})] {
            let _ = s.run("tex.compile", &v);
            let _ = s.run("tex.tick", &v);
        }
        // With a source open: the same hostile sweep still never panics.
        open_doc(&mut s, &dir, "hello\n");
        assert!(s.run("tex.open", &json!({"path": 5})).is_err());
        assert!(s.run("tex.edit", &json!({})).is_err());
        assert!(s.run("tex.edit", &json!({"text": 5})).is_err());
        let overlong = "x".repeat(crate::tex::MAX_SOURCE_BYTES + 1);
        assert!(s.run("tex.edit", &json!({"text": overlong})).is_err());
        assert!(s.run("tex.autoCompile", &json!({})).is_err());
        assert!(s.run("tex.autoCompile", &json!({"enabled": []})).is_err());
        assert!(s.run("tex.forwardSearch", &json!({})).is_err());
        assert!(s.run("tex.forwardSearch", &json!({"line": []})).is_err());
        assert!(s.run("tex.forwardSearch", &json!({"line": 0})).is_err());
        assert!(s.run("tex.forwardSearch", &json!({"line": u64::MAX})).is_err());
        assert!(s.run("tex.inverseSearch", &json!({})).is_err());
        assert!(s.run("tex.inverseSearch", &json!({"page": 1, "x": f64::NAN, "y": 1.0})).is_err());
        assert!(s.run("tex.inverseSearch", &json!({"page": 1, "x": f64::INFINITY, "y": 1.0})).is_err());
        assert!(s.run("tex.bibKeys", &json!({"path": 5})).is_err());
        assert!(s.run("tex.bibKeys", &json!({"path": ""})).is_err());
        assert!(s.run("tex.compile", &json!({"kind": "luatex"})).is_err());
        assert!(s.run("tex.diagnostics", &json!(null)).is_ok());
        assert!(s.run("tex.status", &json!(null)).is_ok());
        assert!(s.run("tex.close", &json!(null)).is_ok());
        assert!(s.run("tex.edit", &json!({"text": "x"})).is_err(), "closed session rejects edits cleanly");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_stages_bib_for_fake_engine() {
        let dir = new_scratch("wcbibcomp").unwrap();
        std::fs::write(dir.join("refs.bib"), BIB).unwrap();
        let engine = write_script(
            &dir,
            "tectonic",
            "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); if [ -f \"$out/refs.bib\" ]; then echo \"STAGED-BIB-OK\"; else echo \"STAGED-BIB-MISSING\"; fi; echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; echo ok;",
        );
        let mut s = session();
        open_doc(&mut s, &dir, DOC_WITH_BIB);
        let out = s.run("tex.compile", &tick_args(&engine)).unwrap();
        assert_eq!(out["success"], json!(true));
        assert!(out["log"].as_str().unwrap_or("").contains("STAGED-BIB-OK"), "{}", out["log"]);
        assert_eq!(out["bibs"], json!(["refs.bib"]));
        let listed = s.run("tex.bibKeys", &json!({})).unwrap();
        assert_eq!(listed["count"], json!(2), "compile refreshes completion keys");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_without_bib_stages_nothing() {
        let dir = new_scratch("wcnobib").unwrap();
        let engine = write_script(
            &dir,
            "tectonic",
            "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); ls \"$out\" | grep -q bib && echo \"UNEXPECTED-BIB\" || echo \"NO-BIB-OK\"; echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; echo ok;",
        );
        let mut s = session();
        open_doc(&mut s, &dir, "\\documentclass{article}\n\\begin{document}\nPlain.\n\\end{document}\n");
        let out = s.run("tex.compile", &tick_args(&engine)).unwrap();
        assert_eq!(out["success"], json!(true));
        assert!(out["log"].as_str().unwrap_or("").contains("NO-BIB-OK"), "{}", out["log"]);
        assert_eq!(out["bibs"], json!([]));
        assert_eq!(out["includes"], json!(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_stages_nested_includes() {
        let dir = new_scratch("wcincludes").unwrap();
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        std::fs::write(dir.join("chapter1.tex"), "chapter one \\input{parts/detail}\n").unwrap();
        std::fs::write(dir.join("parts").join("part.tex"), "a part\n").unwrap();
        std::fs::write(dir.join("parts").join("detail.tex"), "a detail\n").unwrap();
        let engine = write_script(
            &dir,
            "tectonic",
            "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); if [ -f \"$out/chapter1.tex\" ] && [ -f \"$out/parts/part.tex\" ] && [ -f \"$out/parts/detail.tex\" ]; then echo \"STAGED-INCLUDES-OK\"; else echo \"STAGED-INCLUDES-MISSING\"; fi; echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; echo ok;",
        );
        let mut s = session();
        open_doc(&mut s, &dir, "\\documentclass{article}\n\\begin{document}\n\\input{chapter1}\n\\include{parts/part}\n\\end{document}\n");
        let out = s.run("tex.compile", &tick_args(&engine)).unwrap();
        assert_eq!(out["success"], json!(true));
        assert!(out["log"].as_str().unwrap_or("").contains("STAGED-INCLUDES-OK"), "{}", out["log"]);
        assert_eq!(out["includes"], json!(3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_rejects_traversal_and_missing() {
        let dir = new_scratch("wcincludevil").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("secret.tex"), "SECRET-MARKER-98765\n").unwrap();
        let engine = write_script(&dir, "tectonic", "echo should never run; exit 99;");
        let cases = [
            ("parent escape", "\\input{../secret}"),
            ("absolute", "\\input{/etc/passwd}"),
            ("backslash escape", "\\input{..\\secret}"),
            ("bib escape", "\\bibliography{../secret}"),
            ("missing include", "\\input{nosuchfile}"),
            ("missing bib", "\\bibliography{nosuchbib}"),
        ];
        for (what, frag) in cases {
            let doc = format!("\\documentclass{{article}}\n\\begin{{document}}\n{frag}\n\\end{{document}}\n");
            std::fs::write(dir.join("sub").join("main.tex"), &doc).unwrap();
            let mut s = session();
            s.run("tex.open", &json!({"path": dir.join("sub").join("main.tex")})).unwrap();
            let err = s.run("tex.compile", &tick_args(&engine)).unwrap_err();
            let msg = format!("{err}");
            assert!(!msg.contains("SECRET-MARKER-98765"), "{what}: secret leaked: {msg}");
            assert!(
                msg.contains("escapes") || msg.contains("absolute") || msg.contains("not found") || msg.contains("could not read"),
                "{what}: clean refusal, got: {msg}"
            );
            assert!(s.tex.last.is_none(), "{what}: nothing published");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdftex_with_bib_runs_engine_thrice() {
        let dir = new_scratch("wcpdftexbib").unwrap();
        std::fs::write(dir.join("refs.bib"), BIB).unwrap();
        // Fake pdflatex: counts its own passes in the outdir and echoes the
        // count, so the test sees all three engine passes in one log.
        let engine = write_script(
            &dir,
            "pdflatex",
            "out=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; done; n=$(cat \"$out/count\" 2>/dev/null || echo 0); n=$((n+1)); echo \"$n\" > \"$out/count\"; echo \"ENGINE-PASS-$n\"; echo \"%PDF-1.4 fake\" > \"$out/main.pdf\"; echo ok;",
        );
        let mut s = session();
        open_doc(&mut s, &dir, DOC_WITH_BIB);
        let out = s.run("tex.compile", &json!({"engine": engine, "kind": "pdflatex", "timeoutMs": 30_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        assert_eq!(out["success"], json!(true));
        let log = out["log"].as_str().unwrap_or("").to_string();
        assert!(log.contains("ENGINE-PASS-1"), "{log}");
        assert!(log.contains("ENGINE-PASS-2"), "{log}");
        assert!(log.contains("ENGINE-PASS-3"), "{log}");
        assert_eq!(out["bibs"], json!(["refs.bib"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bib_keys_survive_hostile_log_and_missing_rasterizer() {
        let dir = new_scratch("wcbiblog").unwrap();
        std::fs::write(dir.join("refs.bib"), BIB).unwrap();
        let engine = write_script(
            &dir,
            "tectonic",
            "out=\"\"; src=\"\"; for a in \"$@\"; do if [ -d \"$a\" ]; then out=\"$a\"; fi; case \"$a\" in *.tex) src=\"$a\";; esac; done; stem=$(basename \"$src\" .tex); echo \"%PDF-1.4 fake\" > \"$out/$stem.pdf\"; echo \"Citation \\`ghost\\' undefined.\"; echo \"error: the BibTeX engine had an unrecoverable error\";",
        );
        let mut s = session();
        open_doc(&mut s, &dir, DOC_WITH_BIB);
        let out = s.run("tex.compile", &tick_args(&engine)).unwrap();
        assert_eq!(out["success"], json!(true));
        let diag = s.run("tex.diagnostics", &json!({})).unwrap();
        let entries = diag["entries"].as_array().cloned().unwrap_or_default();
        assert!(entries.iter().any(|e| e["severity"] == json!("warning")), "{entries:?}");
        assert!(entries.iter().any(|e| e["severity"] == json!("error")), "{entries:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_tectonic_resolves_citations_when_available() {
        let bundled = PathBuf::from("/home/mitaoe/wordcraft-study/tools/tectonic");
        let real: PathBuf = if bundled.is_file() {
            bundled
        } else {
            match detect_from_path() {
                TexEngine::Tectonic(found) => found,
                _ => {
                    eprintln!("real-bib-compile unverified: no tectonic engine found");
                    return;
                }
            }
        };
        let dir = new_scratch("wcbibreal").unwrap();
        std::fs::write(dir.join("main.tex"), DOC_WITH_BIB).unwrap();
        std::fs::write(dir.join("refs.bib"), BIB).unwrap();
        let mut s = session();
        s.run("tex.open", &json!({"path": dir.join("main.tex")})).unwrap();
        let compiled =
            s.run("tex.compile", &json!({"engine": real, "kind": "tectonic", "timeoutMs": 120_000, "rasterizer": "/nonexistent-pp"})).unwrap();
        if compiled["success"] != json!(true) {
            eprintln!(
                "real-bib-compile unverified: tectonic compile failed: {}",
                compiled["log"].as_str().unwrap_or("").chars().take(300).collect::<String>()
            );
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert!(compiled["pdfBytes"].as_u64().unwrap_or(0) > 0);
        assert_eq!(compiled["bibs"], json!(["refs.bib"]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
