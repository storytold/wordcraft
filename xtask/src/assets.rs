//! `cargo xtask assets`: every non-code asset must be attributed in `ATTRIBUTION.md`.
//!
//! Scans all git-tracked (and untracked, not ignored) files with an asset extension, plus
//! everything under `assets/`, `docs/images/` and `examples/`, and fails if a path is not listed
//! as `` `path` `` in ATTRIBUTION.md. See the asset policy in AGENTS.md.

use std::path::Path;
use std::process::Command;

const ASSET_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "icns", "svg", "pdf", "ai", "eps", "psd", "icc", "icm", "ttf", "otf", "woff",
    "woff2", "ase", "indd", "idml", "aco", "abr", "docx", "odt", "rtf", "mp4", "wav", "mp3",
];
const ASSET_DIRS: &[&str] = &["assets/", "docs/images/", "examples/"];

pub fn is_asset(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase).unwrap_or_default();
    ASSET_DIRS.iter().any(|d| path.starts_with(d)) || ASSET_EXT.contains(&ext.as_str())
}

/// Paths that need an ATTRIBUTION.md entry but lack one.
pub fn missing(files: &[String], assets_md: &str) -> Vec<String> {
    files.iter().filter(|f| is_asset(f) && !assets_md.contains(&format!("`{f}`"))).cloned().collect()
}

/// The paths in `git ls-files -z` output: NUL-separated and never quoted, so a name with
/// non-ASCII letters (`"assets/\303\251.png"` without `-z`) or a newline is still checked.
fn git_paths(stdout: &[u8]) -> Vec<String> {
    stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect()
}

pub fn run(root: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    // A failed listing is no listing: read as empty, it would pass with nothing checked (#1364).
    if !out.status.success() {
        return Err(format!("git ls-files failed ({}): {}", out.status, String::from_utf8_lossy(&out.stderr).trim()));
    }
    let files: Vec<String> = git_paths(&out.stdout).into_iter().filter(|l| root.join(l).exists()).collect();
    let md = std::fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| format!("ATTRIBUTION.md: {e}"))?;
    let miss = missing(&files, &md);
    if miss.is_empty() {
        println!("assets: all {} asset files attributed in ATTRIBUTION.md", files.iter().filter(|f| is_asset(f)).count());
        Ok(())
    } else {
        Err(format!("{} asset file(s) lack an ATTRIBUTION.md entry (author, source, licence):\n  {}", miss.len(), miss.join("\n  ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_unattributed_assets() {
        let files = vec!["assets/icons/a.svg".to_string(), "crates/x/src/lib.rs".into(), "docs/images/b.png".into(), "tests/fixture.png".into()];
        let md = "| `assets/icons/a.svg` | me | here | MIT |";
        assert_eq!(missing(&files, md), vec!["docs/images/b.png".to_string(), "tests/fixture.png".into()]);
        assert!(!is_asset("crates/x/src/lib.rs"));
        assert!(is_asset("assets/fonts/OFL.txt"));
    }

    #[test]
    fn reads_raw_nul_separated_paths() {
        assert_eq!(git_paths(b"assets/\xc3\xa9 b.png\0docs/images/x.svg\0"), vec!["assets/é b.png".to_string(), "docs/images/x.svg".into()]);
        assert!(git_paths(b"").is_empty());
    }

    #[test]
    fn a_failed_file_listing_fails_the_check() {
        // #1364: outside a git checkout `git ls-files` fails, and its empty output read as "no
        // files" passed the check.
        let dir = std::env::temp_dir().join(format!("wordcraft-xtask-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ATTRIBUTION.md"), "").unwrap();
        let inside_git = Command::new("git").current_dir(&dir).args(["rev-parse", "--git-dir"]).output().is_ok_and(|o| o.status.success());
        let result = run(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        if inside_git {
            return; // the temporary folder is in a checkout here: nothing to test
        }
        assert!(result.as_ref().is_err_and(|e| e.starts_with("git ls-files failed")), "{result:?}");
    }
}
