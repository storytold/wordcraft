//! Android entry point for WordCraft.
//!
//! Android's `NativeActivity` loads this library and calls [`android_main`] on a dedicated
//! thread. It opens the shared app ([`wordcraft::run_android`], the same code the desktop binary
//! runs) and returns when the window closes, ending the activity's main-loop thread.
//!
//! This crate exists because that export must be unmangled, and on edition 2024 the only spelling
//! (`#[unsafe(no_mangle)]`) trips the workspace-forbidden `unsafe_code` lint — see `Cargo.toml`
//! for why the crate carries its own lints instead.
//!
//! Android limitations (v1): no system file picker and no system message dialogs (rfd has no
//! Android backend), so Save, Open, exports and picture import are served by WordCraft's own
//! built-in file browser (a later step), and there is no per-user settings folder (`HOME` and
//! `XDG_CONFIG_HOME` are unset): preferences, logs, recovery, user fonts and templates live in
//! the app-private storage directory the activity resolves.
//!
//! On every other target this crate is empty: only Android's loader ever looks for `android_main`.

/// NativeActivity's main-loop thread entry point (required; see the `android-activity` docs).
/// winit pumps the activity's lifecycle and input events inside [`wordcraft::run_android`], which
/// needs the activity's handle to create its event loop; when the activity is destroyed that
/// returns and this returns with it.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: android_activity::AndroidApp) {
    if let Err(e) = wordcraft::run_android(app) {
        log::error!("wordcraft exited with an error: {e}");
    }
}
