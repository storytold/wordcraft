//! Android entry point for WordCraft.
//!
//! Android's `NativeActivity` loads this library and calls [`android_main`] on a dedicated
//! thread. It opens the shared app ([`wordcraft::run_android`], the same code the desktop binary
//! runs) and returns when the window closes, ending the activity's main-loop thread.
//!
//! This crate exists because that export must be unmangled, and on edition 2024 the only spelling
//! (`#[unsafe(no_mangle)]`) trips the workspace-forbidden `unsafe_code` lint — see `Cargo.toml`
//! for why the crate carries its own lints instead. It also holds `android_access` (Android
//! builds), this crate's one piece of real `unsafe` — the JNI to the activity's JVM that checks
//! and opens Android's "All files access" grant, named as the exception in AGENTS.md (Never
//! crash) — whose safe closures are handed to the shared app as [`wordcraft::SharedStorageAccess`].
//!
//! Android limitations (v1): no system file picker and no system message dialogs (rfd has no
//! Android backend), so Save, Open, exports and picture import are served by WordCraft's own
//! built-in file browser: app-private storage always, shared storage (`/storage/emulated/0`)
//! too once the user grants "All files access" (`android_access`); and there is no per-user
//! settings folder (`HOME` and `XDG_CONFIG_HOME` are unset): preferences, logs, recovery, user
//! fonts and templates live in the app-private storage directory the activity resolves.
//!
//! On every other target this crate is empty: only Android's loader ever looks for `android_main`.

#[cfg(target_os = "android")]
mod android_access;

/// NativeActivity's main-loop thread entry point (required; see the `android-activity` docs).
/// winit pumps the activity's lifecycle and input events inside [`wordcraft::run_android`], which
/// needs the activity's handle to create its event loop; when the activity is destroyed that
/// returns and this returns with it.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: android_activity::AndroidApp) {
    // The "All files access" grant, checked and requested over JNI — this crate's unsafe stays
    // in `android_access`; `run_android` only ever sees the plain closures built here.
    let shared = android_access::capture(&app).map(|a| wordcraft::SharedStorageAccess {
        granted: std::sync::Arc::new(move || a.shared_storage_granted()),
        ask: std::sync::Arc::new(move || a.ask_shared_storage_grant()),
    });
    if let Err(e) = wordcraft::run_android(app, shared) {
        log::error!("wordcraft exited with an error: {e}");
    }
}
