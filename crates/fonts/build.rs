//! Optional craft-fonts build input (https://github.com/storytold/craft-fonts, recipe from its
//! `docs/integration.md`). With `CRAFT_FONTS_DIR=<checkout>` the fonts in its
//! `fonts/manifest.txt` are embedded as `CRAFT_FONTS`; unset, `CRAFT_FONTS` is empty. Web
//! (wasm32) builds embed only the UI face, BIZ UDPGothic Regular, to stay within the web size
//! budget. With `CRAFT_FONTS_REQUIRED` (releases), a native build also fails when the checkout
//! has no font for a CJK interface language (Japanese, Simplified Chinese), as an outdated pin
//! once did (#241). It only reads the local checkout: no network.
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
    let manifest = dir.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let mut out = String::new();
    let mut covered = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        if wasm && !(*family == "BIZ UDPGothic" && *style == "Regular") {
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
    Ok((out, covered))
}
