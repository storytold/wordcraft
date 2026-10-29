//! Tests for the macOS key translation that run on every platform.

use std::path::Path;

// `command_key` is plain Rust.
#[allow(dead_code)]
#[path = "../src/platform_impl/macos/command_key.rs"]
mod command_key;

#[test]
fn macos_command_takes_the_layouts_command_table() {
    // winit 0.30.13 names a key pressed with ⌘ by `charactersIgnoringModifiers`, which ignores the
    // layout's ⌘ table: on "Dvorak – QWERTY ⌘", ⌘V was reported as ⌘K (storytold/pdfcraft#3655).
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform_impl/macos/event.rs");
    let event = std::fs::read_to_string(path).unwrap();
    assert!(
        event.contains("match has_cmd.then(|| command_table(has_shift)).flatten() {"),
        "a key pressed with ⌘ ignores the ⌘ table"
    );
    assert!(event.contains("uc_modifier_state(shift, true)"), "the ⌘ table isn't read with ⌘");
}
