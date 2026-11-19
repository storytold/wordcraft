//! Request-signatures UI: craft-ui's real esign feature (storytold/craft-libs#661), not a local
//! placeholder.
//!
//! Nobody connects a backend here. `EsignDialog::set_backend` is never called anywhere in this
//! repository, so `EsignDialog::available` stays false. `request_button` (re-exported below)
//! already returns `None` while that holds, so the title bar's request-signatures button draws
//! nothing: see `chrome.rs`'s call to it.
//!
//! Independent of any cloud storage state on purpose: an e-sign service and a cloud storage
//! account are different things to have configured. WordCraft has no cloud feature today; esign
//! must never be made to depend on one arriving later, the way the review of PdfCraft's own copy
//! of this UI warned against before storytold/craft-libs#661 merged.

pub use craft_ui::esign::*;

/// Drive the dialog every frame. [`EsignDialog::show`] draws it, self-gating on whether it is
/// open the same way `WordApp::window_work`'s other per-window polls (Zotero, Read Aloud) do.
/// Anything the dialog's own UI assembled into a [`SendRequest`] gets encoded and submitted the
/// same way a Save would encode the document: see `encode_active_document`.
pub fn pump(app: &mut crate::WordApp, ctx: &egui::Context) {
    app.esign.show(ctx);
    if let Some(request) = app.esign.take_send_request() {
        match encode_active_document(app) {
            Ok(document) => {
                let create = Request::Create { draft: Box::new(request.draft), document, idempotency_key: request.idempotency_key, send: true };
                app.esign.submit_request(create);
            }
            Err(error) => app.esign.note_create_result(Err(error)),
        }
    }
    // A finished ceremony's signed bytes open as their own document, never assumed to be the one
    // the envelope was created from: `CompletedDocument`'s own docs warn that a provider's
    // sealing (a certificate page, flattening, a visible audit stamp) can change the bytes
    // relative to the original. Signing services overwhelmingly return that sealed artifact as a
    // PDF regardless of the input format, so the opened copy is named and sniffed as one; `.pdf`
    // is already one of `wordcraft_engine::io`'s supported open extensions.
    if let Some(bytes) = app.esign.take_completed_bytes() {
        let name = format!("{} (signed).pdf", app.title_stem());
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        let _ = app.run("file.open", serde_json::json!({"path": name, "data": data}));
    }
}

/// The active document's bytes, the same clean full copy a Save produces. An envelope's signed
/// artifact must not carry revisions the ceremony never saw.
///
/// No password: a recipient asked to sign must be able to open what the service sends them, even
/// when the owner's own copy is password protected (the same choice `commands.rs` makes for an
/// export, as opposed to a Save or Save As, which do carry the session's password).
fn encode_active_document(app: &mut crate::WordApp) -> Result<Vec<u8>, String> {
    if !app.has_document() {
        return Err("no document is open".to_owned());
    }
    let name = app.default_save_name();
    let opts = wordcraft_engine::cmd::file::export_options(&app.session, &serde_json::json!({}));
    wordcraft_engine::io::save_bytes_with(&name, &app.session.doc, None, &opts)
}

#[cfg(test)]
#[path = "esign_tests.rs"]
mod tests;
