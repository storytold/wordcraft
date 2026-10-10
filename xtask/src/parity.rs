//! `cargo xtask parity`: write docs/parity-checklist.md from the live command registry compared with the
//! word-processor feature catalog (via `wordcraft-cli parity --markdown`).

use std::path::Path;

pub fn run(root: &Path) -> Result<(), String> {
    let out = crate::cargo()
        .args(["run", "-q", "-p", "wordcraft-cli", "--", "parity", "--markdown"])
        .output()
        .map_err(|e| format!("cargo run wordcraft-cli: {e}"))?;
    if !out.status.success() {
        return Err(format!("wordcraft-cli parity failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let md = String::from_utf8_lossy(&out.stdout).to_string();
    let path = root.join("docs/parity-checklist.md");
    std::fs::write(&path, &md).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(line) = md.lines().find(|l| l.starts_with("**")) {
        println!("{line}");
    }
    println!("wrote {}", path.display());
    Ok(())
}
