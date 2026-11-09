//! Integration test for Linux Wayland client-side window decoration controls (#5814, #5375).
//!
//! Ensures that on Linux, client-side window controls use font-free Adwaita vector paths
//! so the close button is rendered as a clean vector cross rather than a square fallback,
//! and that sctk-adwaita has proper license attribution in NOTICE without bundling font files.

use std::path::Path;

/// Verifies that `apps/wordcraft/Cargo.toml` enables `wayland-csd-adwaita-notitle` for Linux.
#[test]
fn linux_manifest_enables_wayland_csd_adwaita_notitle() {
    let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_toml).expect("read apps/wordcraft/Cargo.toml");
    assert!(
        content.contains("wayland-csd-adwaita-notitle"),
        "apps/wordcraft/Cargo.toml must enable wayland-csd-adwaita-notitle for Linux decorations"
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
        !adwaita_entry.contains("fontdue") && !adwaita_entry.contains("ab_glyph") && !adwaita_entry.contains("cosmic-text"),
        "sctk-adwaita must not depend on any font renderer in the notitle configuration"
    );
}
