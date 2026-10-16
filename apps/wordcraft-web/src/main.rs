//! WordCraft in the browser.
//!
//! Runs the same [`wordcraft_ui_egui::WordApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory (output in `dist/web`).
//!
//! Differences from the desktop app: no TCP control channel; Open and Insert › Pictures use the
//! browser file picker (bytes arrive through `Services::inbox`); Save and Export download.
//! URL flags: `?webgl` forces WebGL2; `?sample` opens the sample document.
//!
//! Self-hosted deployments can build with `--features self-hosted-update` (off by default): a
//! notice tells visitors when a newer release exists and the administrator can start the update
//! (`updates`, docs/self-hosted-updates.md).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// The web shell uses it; natively it only exists for its tests.
#[cfg(all(feature = "self-hosted-update", any(target_arch = "wasm32", test)))]
mod updates;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("wordcraft-web only runs in the browser: build it with `trunk build --release` in apps/wordcraft-web");
}
