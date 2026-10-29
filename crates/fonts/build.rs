//! Optional craft-fonts build input (https://github.com/storytold/craft-fonts, recipe from its
//! `docs/integration.md`). With `CRAFT_FONTS_DIR=<checkout>` the fonts in its
//! `fonts/manifest.txt` are embedded as `CRAFT_FONTS`; unset, `CRAFT_FONTS` is empty. Web
//! (wasm32) builds embed only the Georgian interface face (about 230 KB), to stay within the web
//! size budget; the Japanese one is 4.7 MB. With `CRAFT_FONTS_REQUIRED` (releases), a native
//! build also fails when the checkout has no font for a CJK interface language (Japanese,
//! Simplified Chinese), as an outdated pin once did (#241). It only reads the local checkout: no
//! network.
use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    let mut src = String::from("pub static CRAFT_FONTS: &[CraftFont] = &[\n");
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").map(PathBuf::from) {
        match craft_fonts(&dir) {
            Ok((entries, covered)) => {
                src.push_str(&entries);
                // Each CJK interface language needs its own face: without the Chinese one, Chinese
                // menus fall back to the Japanese face and show boxes for simplified hanzi (#241).
                let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
                if std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() && !wasm {
                    for s in REQUIRED_SCRIPTS.iter().filter(|s| !covered.iter().any(|c| c == *s)) {
                        println!("cargo::error=CRAFT_FONTS_DIR={}: no {s} font in fonts/manifest.txt (craft-fonts too old?)", dir.display());
                    }
                }
            }
            Err(e) if std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() => {
                println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display());
            }
            Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        }
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// Scripts a release build (`CRAFT_FONTS_REQUIRED`, native) must embed a font for: the CJK
/// interface languages' (Japanese, Simplified Chinese).
const REQUIRED_SCRIPTS: &[&str] = &["Jpan", "Hans"];

/// One `CraftFont { .. }` initialiser per manifest line, and the scripts the embedded fonts cover.
fn craft_fonts(dir: &std::path::Path) -> Result<(String, Vec<String>), String> {
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let mut out = String::new();
    let mut covered = Vec::new();
    let mut seen_files = std::collections::HashSet::new();
    read_manifest(dir, &dir.join("fonts/manifest.txt"), wasm, &mut seen_files, &mut out, &mut covered)?;
    if !wasm {
        let hebrew = dir.join("fonts/hebrew-manifest.txt");
        println!("cargo::rerun-if-changed={}", hebrew.display());
        match hebrew.try_exists() {
            Ok(false) => {}
            Ok(true) => {
                let mut he_seen = seen_files.clone();
                let mut he_out = String::new();
                let mut he_covered = Vec::new();
                match read_manifest(dir, &hebrew, false, &mut he_seen, &mut he_out, &mut he_covered) {
                    Ok(()) => {
                        seen_files = he_seen;
                        out.push_str(&he_out);
                        covered.extend(he_covered);
                    }
                    Err(e) => println!("cargo::warning=skipping optional Hebrew fonts: {e}"),
                }
            }
            Err(e) => println!("cargo::warning=skipping optional Hebrew fonts: {}: {e}", hebrew.display()),
        }
    }
    let _ = seen_files;
    Ok((out, covered))
}

/// Parses a craft-fonts manifest file and appends Rust source entries for each eligible face.
fn read_manifest(
    dir: &std::path::Path,
    manifest: &std::path::Path,
    wasm: bool,
    seen_files: &mut std::collections::HashSet<String>,
    out: &mut String,
    covered: &mut Vec<String>,
) -> Result<(), String> {
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        if wasm && !scripts.split(',').any(|s| s.trim() == "Geor") {
            continue;
        }
        if !seen_files.insert((*file).to_string()) {
            continue;
        }
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        covered.extend(scripts.split(',').map(|s| s.trim().to_string()));
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }
    Ok(())
}
