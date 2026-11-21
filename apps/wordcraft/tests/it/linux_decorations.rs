//! Integration test for Linux Wayland client-side window decoration controls (#5814, #5375).
//!
//! Ensures that on Linux, client-side window controls use font-free Adwaita vector paths
//! so the close button is rendered as a clean vector cross rather than a square fallback,
//! and that sctk-adwaita has proper license attribution in NOTICE without bundling font files.

use std::path::Path;

/// Strips an unquoted TOML `#` comment from a single line.
fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '#' if !in_double && !in_single => return &line[..idx],
            _ => {}
        }
    }
    line
}

/// Extracts the comment-stripped body of a TOML table section by exact `[header]`.
fn toml_section(content: &str, header: &str) -> String {
    let mut in_section = false;
    let mut out = String::new();
    for raw_line in content.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.starts_with('[') && line.ends_with(']') {
            if in_section {
                break;
            }
            in_section = line == header;
            continue;
        }
        if in_section && !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    assert!(in_section, "expected target section {header} in apps/wordcraft/Cargo.toml");
    out
}

/// Parses the `features = [...]` string array from the `winit = { ... }` dependency entry in a TOML section.
fn winit_features(section: &str) -> Vec<&str> {
    let winit_start = section.find("winit").expect("target section must declare a winit dependency");
    let after_winit = &section[winit_start..];
    let table_end = after_winit.find('}').expect("winit inline table must close with '}'");
    let winit_table = &after_winit[..=table_end];
    let features_start = winit_table.find("features").expect("winit dependency must declare features");
    let after_features = &winit_table[features_start..];
    let open_bracket = after_features.find('[').expect("winit features must open with '['");
    let after_open = &after_features[open_bracket + 1..];
    let close_bracket = after_open.find(']').expect("winit features must close with ']'");
    let array_body = &after_open[..close_bracket];
    array_body.split(',').map(|item| item.trim().trim_matches('"')).filter(|item| !item.is_empty()).collect()
}

/// Verifies that `apps/wordcraft/Cargo.toml` enables `wayland-csd-adwaita-notitle` for Linux.
#[test]
fn linux_manifest_enables_wayland_csd_adwaita_notitle() {
    let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_toml).expect("read apps/wordcraft/Cargo.toml");

    let linux_section = toml_section(&content, "[target.'cfg(target_os = \"linux\")'.dependencies]");
    let linux_features = winit_features(&linux_section);
    assert!(
        linux_features.contains(&"x11") && linux_features.contains(&"wayland-csd-adwaita-notitle"),
        "Linux winit dependency must enable x11 and wayland-csd-adwaita-notitle, got: {linux_features:?}"
    );

    let other_unix_section = toml_section(&content, "[target.'cfg(all(unix, not(any(target_os = \"macos\", target_os = \"linux\"))))'.dependencies]");
    let other_unix_features = winit_features(&other_unix_section);
    assert!(
        other_unix_features.contains(&"x11") && !other_unix_features.contains(&"wayland-csd-adwaita-notitle"),
        "Non-Linux Unix winit dependency must enable x11 without wayland-csd-adwaita-notitle, got: {other_unix_features:?}"
    );
}

/// Verifies that `NOTICE` documents sctk-adwaita and its MIT licence.
#[test]
fn notice_documents_sctk_adwaita_attribution() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspace root");
    let notice = std::fs::read_to_string(workspace_root.join("NOTICE")).expect("read NOTICE");
    assert!(notice.contains("Linux Wayland window decorations"), "NOTICE must include 'Linux Wayland window decorations'");
    assert!(notice.contains("sctk-adwaita"), "NOTICE must document sctk-adwaita");
    assert!(notice.contains("Bartłomiej Maryńczak"), "NOTICE must retain sctk-adwaita's author copyright");
}

/// Verifies that `Cargo.lock` resolves `sctk-adwaita` without font dependencies.
#[test]
fn cargo_lock_contains_font_free_sctk_adwaita() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("workspace root");
    let lockfile = std::fs::read_to_string(workspace_root.join("Cargo.lock")).expect("read Cargo.lock");
    assert!(lockfile.contains("name = \"sctk-adwaita\""), "Cargo.lock must include sctk-adwaita");

    // Parse the sctk-adwaita package entry from Cargo.lock and verify its dependencies.
    let adwaita_entry =
        lockfile.split("[[package]]").find(|chunk| chunk.contains("name = \"sctk-adwaita\"")).expect("sctk-adwaita package entry in Cargo.lock");

    assert!(
        !adwaita_entry.contains("fontdue")
            && !adwaita_entry.contains("ab_glyph")
            && !adwaita_entry.contains("cosmic-text")
            && !adwaita_entry.contains("crossfont"),
        "sctk-adwaita must not depend on any font renderer in the notitle configuration"
    );
}
