//! Google Drive in the app (File › Google Drive): `ui.drive.status`, `ui.drive.signIn`,
//! `ui.drive.cancel`, `ui.drive.signOut`, `ui.drive.list {"search"?, "more"?}`,
//! `ui.drive.open {"id"}`, `ui.drive.save {"name"?, "copy"?}`, and `ui.drive.start` /
//! `ui.drive.reload` (read the OAuth client and the keychain; again after setting it up).
//!
//! Network calls run on one background thread that owns the [`wordcraft_gdrive::Drive`] client;
//! the UI thread sends it jobs and [`poll`] applies what comes back each frame, as the Zotero
//! link does. Bytes are produced and documents opened on the UI thread, through the same engine
//! calls as File › Open and the web build's Save.
//!
//! A document opened from Drive remembers its Drive file and the `version` it was opened at, so
//! File › Save (`file.save` without a `path`) goes back to Drive, and is refused rather than
//! overwriting when someone changed the file in Drive meanwhile. It has no local path, so
//! nothing is written to disk and it stays out of the recent-files list.
//!
//! Only the desktop app turns this on ([`crate::services::Services::drive_profile`]); without
//! it (tests, headless, web) every command answers that Drive isn't available and nothing
//! touches the network or the keychain.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::WordApp;

/// What a document opened from (or saved to) Drive is in Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub id: String,
    pub name: String,
    pub version: String,
    /// A Google Doc exported as `.docx`: saves go to a new `.docx` file, never over it.
    pub google_doc: bool,
}

/// A file in the last listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub modified: String,
    pub google_doc: bool,
}

/// The app's Drive state.
#[derive(Default)]
pub struct DriveLink {
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<native::Worker>,
    /// `None` until the worker has looked in the keychain.
    pub signed_in: Option<bool>,
    /// What the worker is doing, for the page and `ui.drive.status`.
    pub busy: Option<&'static str>,
    /// Why Drive can't be used (no OAuth client set up), shown on the page.
    pub unavailable: Option<String>,
    /// The last error, shown on the page until the next job.
    pub error: Option<String>,
    /// The consent page, shown as a link in case the browser didn't open.
    pub consent_url: Option<String>,
    pub files: Vec<Listed>,
    pub next_page: Option<String>,
    /// The page's search box.
    pub search: String,
    /// The last conflict, so the page can offer Save a Copy.
    pub conflict: Option<u64>,
    /// The page listed files since sign-in (so an empty Drive isn't listed every frame).
    pub listed: bool,
    /// File › Export's "Save the copy to Google Drive" checkbox.
    pub export_to_drive: bool,
    origins: HashMap<u64, Origin>,
    /// Per document: how many local saves set where it lives (see [`note_local_save`]). A Drive
    /// upload that ends after a newer one leaves the destination alone.
    local_saves: HashMap<u64, u64>,
}

impl DriveLink {
    /// The Drive file the document `doc` was opened from or last saved to.
    pub fn origin(&self, doc: u64) -> Option<&Origin> {
        self.origins.get(&doc)
    }
}

/// Handle `ui.drive.*`. `None` when `id` isn't one.
pub fn command(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let name = id.strip_prefix("ui.drive.")?;
    match name {
        "status" => return Some(Ok(status(app))),
        "enable" => return Some(enable(app, params)),
        _ if !app.ui.google_drive => return Some(Err(OFF.into())),
        _ => {}
    }
    Some(imp::run(app, name, params))
}

/// The File › Export formats a copy can go to Google Drive in (the ones written from bytes).
pub(crate) const EXPORT_FORMATS: [&str; 10] = ["pdf", "docx", "odt", "ott", "fodt", "rtf", "html", "md", "tex", "txt"];

/// File › Export sends the copy to Google Drive: Google Drive is on and signed in, and the
/// checkbox on the Export page is ticked.
pub(crate) fn exports_to_drive(app: &WordApp) -> bool {
    app.ui.google_drive && app.drive.signed_in == Some(true) && app.drive.export_to_drive
}

/// File › Export's "Save the copy to Google Drive" checkbox, shown once signed in.
pub(crate) fn export_checkbox(app: &mut WordApp, ui: &mut egui::Ui) {
    if !app.ui.google_drive || app.drive.signed_in != Some(true) {
        return;
    }
    ui.checkbox(&mut app.drive.export_to_drive, tl!("Save the copy to Google Drive (My Drive) instead of a folder on this computer"));
    ui.add_space(8.0);
}

const OFF: &str = "Google Drive is off; turn it on in File › Options › Accounts";

/// `ui.drive.enable {value}`: turns Google Drive on or off (File › Options › Accounts). No value
/// reads it. Turning it off leaves the sign-in in the keychain, so turning it on again needs none.
fn enable(app: &mut WordApp, params: &Value) -> Result<Value, String> {
    match params.get("value") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(on)) => {
            app.ui.google_drive = *on;
            if !on && app.ui.backstage_page == "drive" {
                app.ui.backstage_page = "home".into();
            }
        }
        Some(_) => return Err("`value` must be true or false".into()),
    }
    Ok(json!({"enabled": app.ui.google_drive}))
}

/// `file.save` for a document that came from Drive (no `path` given): saves back to Drive.
/// `None` when the document isn't a Drive document, so the ordinary save runs.
pub fn intercept_save(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    // Turned off: an ordinary save (it asks where, since the document has no local path).
    if id != "file.save" || !app.ui.google_drive || params.get("path").is_some_and(|p| !p.is_null()) {
        return None;
    }
    destination(app)?;
    Some(imp::run(app, "save", params))
}

/// After `file.save` / `file.saveAs` ran in the engine: counts a successful save that made a local
/// file the document's home (the engine set its path to the file written). A copy, such as a
/// plain-text save to another file, doesn't count.
pub(crate) fn note_local_save(app: &mut WordApp, id: &str, document: u64, result: &Result<Value, String>) {
    if !matches!(id, "file.save" | "file.saveAs") || app.session.document_id() != document {
        return;
    }
    let Ok(v) = result else { return };
    let written = v.get("path").and_then(Value::as_str).map(std::path::Path::new);
    if written.is_some() && written == app.session.path.as_deref() {
        let n = app.drive.local_saves.entry(document).or_default();
        *n = n.wrapping_add(1);
    }
}

/// The active document's Drive file, while Drive is where it was last opened from or saved to.
/// A local Save As gives the document a path, and from then on Save writes that file; saving to
/// Drive again clears the path (see `Event::Saved`).
fn destination(app: &WordApp) -> Option<&Origin> {
    if app.session.path.is_some() {
        return None;
    }
    app.drive.origin(app.session.document_id())
}

fn status(app: &WordApp) -> Value {
    let d = &app.drive;
    let origin = destination(app);
    json!({
        "enabled": app.ui.google_drive,
        "available": d.unavailable.is_none() && app.services.drive_profile.is_some(),
        "unavailable": d.unavailable,
        "signedIn": d.signed_in,
        "busy": d.busy,
        "error": d.error,
        "files": d.files.iter().map(|f| json!({"id": f.id, "name": f.name, "modified": f.modified, "googleDoc": f.google_doc})).collect::<Vec<_>>(),
        "morePages": d.next_page.is_some(),
        "document": origin.map(|o| json!({"id": o.id, "name": o.name, "version": o.version, "googleDoc": o.google_doc})),
    })
}

/// Whether the active document's saved state is a file in Drive (not a Google Doc's Word copy,
/// which has no file of its own until it is saved): the title bar then says so instead of
/// "Not saved".
pub(crate) fn saved_in_drive(app: &WordApp) -> bool {
    destination(app).is_some_and(|o| !o.google_doc)
}

/// Apply the worker's results (each frame).
pub fn poll(app: &mut WordApp) {
    imp::poll(app);
}

/// File › Options at its Accounts pane.
fn open_accounts(app: &mut WordApp) {
    app.ui.options_pane = "accounts".into();
    let _ = app.run("ui.backstage", json!({"value": true, "page": "options"}));
}

/// File › Options › Accounts: turn Google Drive on, add the Google client, sign in or out.
pub fn accounts(app: &mut WordApp, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Google Drive")).font(crate::theme::semibold(15.0)));
    ui.add_space(2.0);
    let mut on = app.ui.google_drive;
    if ui.checkbox(&mut on, tl!("Use Google Drive (File › Google Drive: open from Drive and save back)")).changed() {
        let _ = app.run("ui.drive.enable", json!({"value": on}));
    }
    ui.label(tl!("WordCraft talks to Google Drive directly: the Google Drive desktop app isn't needed."));
    if !app.ui.google_drive {
        return;
    }
    if app.services.drive_profile.is_none() {
        ui.label(tl!("Google Drive works with the WordCraft desktop app."));
        return;
    }
    if app.drive.signed_in.is_none() && app.drive.busy.is_none() && app.drive.unavailable.is_none() {
        let _ = app.run("ui.drive.start", json!({}));
    }
    ui.add_space(10.0);
    // The client: an OAuth client of type Desktop app from the person's own Google Cloud project.
    let configured = app.drive.unavailable.is_none();
    ui.label(if configured {
        tl!("Google client: set up. Enter a new one below to replace it.")
    } else {
        tl!("Google client: not set up. Create an OAuth client of type Desktop app in your own Google Cloud project (see Google Drive in WordCraft's documentation), then enter it here.")
    });
    let id_key = egui::Id::unique("accounts_google_client_id");
    let secret_key = egui::Id::unique("accounts_google_client_secret");
    let mut id = ui.data(|d| d.get_temp::<String>(id_key)).unwrap_or_default();
    let mut secret = ui.data(|d| d.get_temp::<String>(secret_key)).unwrap_or_default();
    egui::Grid::new("accounts_google_client").num_columns(2).show(ui, |ui| {
        let l = ui.label(tl!("Client ID:")).id;
        ui.add(egui::TextEdit::singleline(&mut id).desired_width(380.0).hint_text("1234-abc.apps.googleusercontent.com")).labelled_by(l);
        ui.end_row();
        let l = ui.label(tl!("Client secret:")).id;
        ui.add(egui::TextEdit::singleline(&mut secret).password(true).desired_width(380.0)).labelled_by(l);
        ui.end_row();
    });
    if ui.add_enabled(!id.trim().is_empty(), egui::Button::new(tl!("Save client"))).clicked() {
        let secret_param = if secret.trim().is_empty() { Value::Null } else { Value::from(secret.trim()) };
        match app.run("ui.drive.client", json!({"clientId": id.trim(), "clientSecret": secret_param})) {
            Ok(_) => {
                id.clear();
                secret.clear();
                app.status(tl!("Saved the Google client").to_string());
            }
            Err(e) => app.status(e),
        }
    }
    ui.data_mut(|d| {
        d.insert_temp(id_key, id);
        d.insert_temp(secret_key, secret);
    });
    if !configured {
        return;
    }
    ui.add_space(10.0);
    match (app.drive.signed_in, app.drive.busy) {
        (Some(true), _) => {
            ui.label(tl!("Signed in to Google."));
            ui.horizontal(|ui| {
                if ui.button(tl!("Open File › Google Drive")).clicked() {
                    let _ = app.run("ui.backstage", json!({"value": true, "page": "drive"}));
                }
                if ui.add_enabled(app.drive.busy.is_none(), egui::Button::new(tl!("Sign out"))).clicked() {
                    let _ = app.run("ui.drive.signOut", json!({}));
                }
            });
        }
        (_, Some("signIn")) => {
            ui.label(tl!("Finish signing in in your browser."));
            if ui.button(tl!("Cancel")).clicked() {
                let _ = app.run("ui.drive.cancel", json!({}));
            }
        }
        (Some(false), busy) => {
            if ui.add_enabled(busy.is_none(), egui::Button::new(tl!("Sign in with Google"))).clicked() {
                let _ = app.run("ui.drive.signIn", json!({}));
            }
        }
        (None, _) => {
            ui.label(tl!("Checking your Google account…"));
        }
    }
}

/// File › Google Drive.
pub fn page(app: &mut WordApp, ui: &mut egui::Ui) {
    use egui::vec2;
    ui.label(egui::RichText::new(tl!("Google Drive")).font(crate::theme::semibold(26.0)));
    ui.add_space(16.0);
    if app.services.drive_profile.is_none() {
        ui.label(tl!("Google Drive works with the WordCraft desktop app."));
        return;
    }
    if !app.ui.google_drive {
        ui.label(tl!("Google Drive is off. Turn it on in File › Options › Accounts."));
        ui.add_space(12.0);
        if ui.button(tl!("Open Accounts")).clicked() {
            open_accounts(app);
        }
        return;
    }
    if app.drive.signed_in.is_none() && app.drive.busy.is_none() && app.drive.unavailable.is_none() {
        let _ = app.run("ui.drive.start", json!({}));
    }
    if let Some(why) = app.drive.unavailable.clone() {
        ui.label(tl!(
            "To use Google Drive, WordCraft needs an OAuth client from your own Google Cloud project. Add it in File › Options › Accounts."
        ));
        ui.add_space(8.0);
        ui.label(why);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Open Accounts")).clicked() {
                open_accounts(app);
            }
            if ui.button(tl!("Try again")).clicked() {
                let _ = app.run("ui.drive.reload", json!({}));
            }
        });
        return;
    }
    let busy = app.drive.busy;
    if let Some(e) = app.drive.error.clone() {
        ui.colored_label(ui.visuals().error_fg_color, e);
        ui.add_space(12.0);
    }
    match (app.drive.signed_in, busy) {
        (None, _) | (_, Some("starting")) => {
            ui.label(tl!("Checking your Google account…"));
            return;
        }
        (_, Some("signIn")) => {
            ui.label(tl!("Finish signing in in your browser."));
            if let Some(url) = app.drive.consent_url.clone() {
                ui.hyperlink_to(tl!("Open the Google sign-in page again"), url);
            }
            ui.add_space(12.0);
            if ui.button(tl!("Cancel")).clicked() {
                let _ = app.run("ui.drive.cancel", json!({}));
            }
            return;
        }
        (Some(false), _) => {
            ui.label(tl!("Sign in to open documents from Google Drive and save them back."));
            ui.add_space(12.0);
            if ui.add_enabled(busy.is_none(), egui::Button::new(tl!("Sign in with Google"))).clicked() {
                let _ = app.run("ui.drive.signIn", json!({}));
            }
            return;
        }
        (Some(true), _) => {}
    }

    // This document.
    let doc = app.session.document_id();
    if app.has_document() {
        let origin = app.drive.origin(doc).cloned();
        match &origin {
            Some(o) if !o.google_doc => {
                ui.label(crate::i18n::fmt(tl!("This document is {name} in Google Drive. Save sends your changes there."), &[("name", &o.name)]));
            }
            Some(o) => {
                ui.label(crate::i18n::fmt(
                    tl!("This document is a Word copy of the Google Doc {name}. Save to Google Drive adds it as a new .docx file."),
                    &[("name", &o.name)],
                ));
            }
            None => {
                ui.label(tl!("Save this document to My Drive."));
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(busy.is_none(), |ui| {
                if ui.button(tl!("Save to Google Drive")).clicked() {
                    let _ = app.run("ui.drive.save", json!({}));
                }
                if (origin.is_some() || app.drive.conflict == Some(doc)) && ui.button(tl!("Save a Copy")).clicked() {
                    let _ = app.run("ui.drive.save", json!({"copy": true}));
                }
            });
        });
        ui.add_space(24.0);
    }

    // Open.
    ui.label(egui::RichText::new(tl!("Open from Google Drive")).font(crate::theme::semibold(18.0)));
    ui.add_space(8.0);
    let mut go = false;
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut app.drive.search).hint_text(tl!("Search Google Drive")).desired_width(320.0));
        go |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        go |= ui.add_enabled(busy.is_none(), egui::Button::new(tl!("Search"))).clicked();
    });
    if go || (app.drive.files.is_empty() && busy.is_none() && !app.drive.listed) {
        app.drive.listed = true;
        let search = app.drive.search.clone();
        let _ = app.run("ui.drive.list", json!({"search": search}));
    }
    ui.add_space(12.0);
    if matches!(busy, Some("list")) {
        ui.label(tl!("Loading…"));
    } else if app.drive.files.is_empty() {
        ui.label(tl!("No Word, OpenDocument, RTF or Google Docs files found."));
    }
    let files = app.drive.files.clone();
    crate::rtl::grid(ui, egui::Grid::new("drive_files").min_col_width(160.0).spacing(vec2(24.0, 10.0)), |ui| {
        if !files.is_empty() {
            ui.strong(tl!("Name"));
            ui.strong(tl!("Modified"));
            ui.end_row();
        }
        for f in &files {
            ui.label(&f.name);
            ui.label(crate::local_time::utc_time(&f.modified));
            if ui.add_enabled(busy.is_none(), egui::Button::new(tl!("Open"))).clicked() {
                let _ = app.run("ui.drive.open", json!({"id": f.id}));
            }
            ui.end_row();
        }
    });
    if app.drive.next_page.is_some() {
        ui.add_space(8.0);
        if ui.add_enabled(busy.is_none(), egui::Button::new(tl!("Show more"))).clicked() {
            let _ = app.run("ui.drive.list", json!({"search": app.drive.search.clone(), "more": true}));
        }
    }
    ui.add_space(24.0);
    if ui.add_enabled(busy.is_none(), egui::Button::new(tl!("Sign out"))).clicked() {
        let _ = app.run("ui.drive.signOut", json!({}));
    }
}

/// Names a document saves to Drive under: the Drive name, else the document's name, as `.docx`
/// unless it already has an extension WordCraft can save without losing anything.
// The web build has no Drive saves; the tests still check the names.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn save_name(app: &WordApp, given: Option<&str>) -> String {
    let base = given
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| app.drive.origin(app.session.document_id()).filter(|o| !o.google_doc).map(|o| o.name.clone()))
        .or_else(|| app.session.file_name.clone())
        .or_else(|| app.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| app.title_stem());
    if crate::state::keeps_everything(&base) { base } else { format!("{}.docx", base.trim_end_matches(".docx")) }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;
    pub fn run(_app: &mut WordApp, _name: &str, _params: &Value) -> Result<Value, String> {
        Err("Google Drive works with the WordCraft desktop app".into())
    }
    pub fn poll(_app: &mut WordApp) {}
}

#[cfg(not(target_arch = "wasm32"))]
use native as imp;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::Duration;

    use base64::Engine as _;
    use wordcraft_gdrive::{Drive, DriveFile, Error as DriveError, SaveTarget};

    use super::{Listed, Origin, Value, WordApp, json, save_name};
    use crate::i18n;

    /// How long sign-in waits for the browser.
    const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

    pub(super) struct Worker {
        jobs: Sender<Job>,
        events: Receiver<Event>,
        cancel: Arc<AtomicBool>,
    }

    enum Job {
        SignIn,
        SignOut,
        List {
            search: Option<String>,
            page: Option<String>,
        },
        Open {
            id: String,
        },
        /// `local`: the document's local-save count when the upload started; a different count
        /// when it ends means a newer local save set the document's home.
        Save {
            doc: u64,
            rev: u64,
            local: u64,
            target: SaveTarget,
            bytes: Vec<u8>,
            marked_saved: bool,
        },
        /// File › Export with "Save the copy to Google Drive": a new file in My Drive. The
        /// document's own home doesn't change.
        Export {
            name: String,
            bytes: Vec<u8>,
        },
    }

    pub(super) enum Event {
        Unavailable(String),
        /// What the keychain held at start.
        Restored(bool),
        SignedIn(bool),
        ConsentUrl(String),
        Listed {
            files: Vec<DriveFile>,
            next: Option<String>,
            append: bool,
        },
        Opened {
            file: DriveFile,
            bytes: Vec<u8>,
        },
        Saved {
            doc: u64,
            rev: u64,
            local: u64,
            file: DriveFile,
            marked_saved: bool,
        },
        Exported(DriveFile),
        Failed {
            what: &'static str,
            error: DriveError,
            doc: Option<u64>,
        },
    }

    /// Starts the worker on first use. `Err` when this app has no Drive (tests, headless).
    fn worker(app: &mut WordApp) -> Result<&Worker, String> {
        if app.drive.worker.is_none() {
            let Some(profile) = app.services.drive_profile.clone() else {
                return Err("Google Drive works with the WordCraft desktop app".into());
            };
            let (jobs, job_rx) = channel();
            let (event_tx, events) = channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let ctx = app.ctx.clone();
            let c = cancel.clone();
            std::thread::Builder::new()
                .name("google-drive".into())
                .spawn(move || run_worker(&profile, &job_rx, &event_tx, &c, ctx.as_ref()))
                .map_err(|e| format!("could not start Google Drive: {e}"))?;
            app.drive.worker = Some(Worker { jobs, events, cancel });
            app.drive.busy = Some("starting");
        }
        app.drive.worker.as_ref().ok_or_else(|| "Google Drive didn't start".to_string())
    }

    fn send(app: &mut WordApp, job: Job, busy: &'static str) -> Result<Value, String> {
        if let Some(b) = app.drive.busy.filter(|b| *b != "starting") {
            return Err(format!("Google Drive is still busy ({b})"));
        }
        let w = worker(app)?;
        w.cancel.store(false, Ordering::Relaxed);
        w.jobs.send(job).map_err(|_| "Google Drive stopped; restart WordCraft".to_string())?;
        app.drive.busy = Some(busy);
        app.drive.error = None;
        app.drive.conflict = None;
        Ok(json!({"started": busy}))
    }

    pub fn run(app: &mut WordApp, name: &str, params: &Value) -> Result<Value, String> {
        match name {
            "signIn" => send(app, Job::SignIn, "signIn"),
            "cancel" => {
                if let Some(w) = &app.drive.worker {
                    w.cancel.store(true, Ordering::Relaxed);
                }
                Ok(json!({"cancelled": app.drive.busy == Some("signIn")}))
            }
            "signOut" => send(app, Job::SignOut, "signOut"),
            // File › Options › Accounts: writes `google-drive.json`, then reads it again.
            "client" => {
                let text = |k: &str| match params.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(s)) => Ok(Some(s.as_str())),
                    Some(_) => Err(format!("`{k}` must be a string")),
                };
                let id = text("clientId")?.ok_or("`clientId` is required")?;
                let secret = text("clientSecret")?;
                let Some(dir) = app.services.drive_profile.clone() else {
                    return Err("Google Drive works with the WordCraft desktop app".into());
                };
                wordcraft_gdrive::write_client(&dir, id, secret).map_err(|e| e.to_string())?;
                run(app, "reload", &json!({}))
            }
            // Starts the worker (the page does when it opens): it reads the OAuth client and the
            // keychain, nothing else.
            "start" => worker(app).map(|_| json!({"started": true})),
            // Reads `google-drive.json` again (after setting it up) by starting a new worker.
            "reload" => {
                if app.drive.busy.is_some_and(|b| b != "starting") {
                    return Err("Google Drive is busy; wait or cancel first".into());
                }
                app.drive.worker = None;
                app.drive.unavailable = None;
                app.drive.signed_in = None;
                app.drive.busy = None;
                worker(app).map(|_| json!({"started": true}))
            }
            "list" => {
                let more = params.get("more").and_then(Value::as_bool) == Some(true);
                let search = match params.get("search") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) => Some(s.chars().take(200).collect::<String>()),
                    Some(_) => return Err("`search` must be a string".into()),
                };
                let page = if more {
                    match app.drive.next_page.clone() {
                        Some(p) => Some(p),
                        None => return Err("there are no more files to list".into()),
                    }
                } else {
                    None
                };
                send(app, Job::List { search, page }, if more { "listMore" } else { "list" })
            }
            "open" => {
                let Some(id) = params.get("id").and_then(Value::as_str) else {
                    return Err("give `id`: a file id from ui.drive.list".into());
                };
                wordcraft_gdrive::drive::check_id(id).map_err(|e| e.to_string())?;
                send(app, Job::Open { id: id.to_owned() }, "open")
            }
            "save" => save(app, params),
            "export" => export(app, params),
            other => Err(format!("unknown Google Drive command `{other}`; use status, start, reload, signIn, cancel, signOut, list, open or save")),
        }
    }

    /// `ui.drive.export {format}`: a copy in `format` (an extension File › Export offers) as a new
    /// file in My Drive, named after the document. Nothing about the document changes.
    fn export(app: &mut WordApp, params: &Value) -> Result<Value, String> {
        let format = params.get("format").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
        if !super::EXPORT_FORMATS.contains(&format.as_str()) {
            return Err(format!("`format` is one of {}", super::EXPORT_FORMATS.join(", ")));
        }
        if app.services.drive_profile.is_none() {
            return Err("Google Drive works with the WordCraft desktop app".into());
        }
        let name = format!("{}.{format}", app.title_stem());
        let password = app.session.password.as_ref().map(wordcraft_engine::Password::as_str);
        let opts = wordcraft_engine::cmd::file::export_options(&app.session, &json!({}));
        let bytes = wordcraft_engine::io::save_bytes_with(&name, &app.session.doc, password, &opts)?;
        let r = send(app, Job::Export { name: name.clone(), bytes }, "export")?;
        app.status(i18n::fmt(tl!("Exporting {name} to Google Drive…"), &[("name", &name)]));
        Ok(r)
    }

    /// Serializes the document on the UI thread and hands the bytes to the worker.
    fn save(app: &mut WordApp, params: &Value) -> Result<Value, String> {
        let copy = params.get("copy").and_then(Value::as_bool) == Some(true);
        let given = match params.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            Some(_) => return Err("`name` must be a string".into()),
        };
        if app.services.drive_profile.is_none() {
            return Err("Google Drive works with the WordCraft desktop app".into());
        }
        let doc = app.session.document_id();
        let origin = app.drive.origin(doc).cloned();
        let name = save_name(app, given);
        // A Google Doc, a copy, or a new name: a new file. Otherwise the file it came from.
        let target = match origin {
            Some(o) if !copy && !o.google_doc && given.is_none() => SaveTarget::Existing { id: o.id, name: o.name, version: o.version },
            _ => SaveTarget::New { name: name.clone(), parent: None },
        };
        // A save is a save wherever it goes: stamp first, as the desktop and web saves do.
        app.session.stamp_save();
        let password = app.session.password.as_ref().map(wordcraft_engine::Password::as_str);
        let opts = wordcraft_engine::cmd::file::export_options(&app.session, &json!({}));
        let bytes = wordcraft_engine::io::save_bytes_with(&name, &app.session.doc, password, &opts)?;
        let marked_saved = crate::state::keeps_everything(&name);
        let rev = app.session.rev();
        let local = app.drive.local_saves.get(&doc).copied().unwrap_or(0);
        let r = send(app, Job::Save { doc, rev, local, target, bytes, marked_saved }, "save")?;
        app.status(i18n::fmt(tl!("Saving {name} to Google Drive…"), &[("name", &name)]));
        Ok(r)
    }

    fn run_worker(profile: &std::path::Path, jobs: &Receiver<Job>, events: &Sender<Event>, cancel: &AtomicBool, ctx: Option<&egui::Context>) {
        let emit = |e: Event| {
            let _ = events.send(e);
            if let Some(c) = ctx {
                c.request_repaint();
            }
        };
        let cfg = match wordcraft_gdrive::ClientConfig::load(Some(profile)) {
            Ok(c) => c,
            Err(e) => {
                emit(Event::Unavailable(e.to_string()));
                // Keep answering so commands fail with the reason instead of hanging.
                while let Ok(job) = jobs.recv() {
                    emit(Event::Failed { what: job_name(&job), error: e.clone(), doc: job_doc(&job) });
                }
                return;
            }
        };
        let http: Arc<dyn wordcraft_gdrive::http::Transport> = Arc::new(wordcraft_gdrive::http::Ureq::default());
        let mut drive = Drive::new(wordcraft_gdrive::Auth::new(cfg, http, wordcraft_gdrive::store::system()));
        let restored = drive.auth.restore().unwrap_or_else(|e| {
            log::warn!("Google Drive: {e}");
            false
        });
        emit(Event::Restored(restored));
        while let Ok(job) = jobs.recv() {
            let what = job_name(&job);
            let doc = job_doc(&job);
            let r = match job {
                Job::SignIn => {
                    let open = |url: &str| {
                        emit(Event::ConsentUrl(url.to_owned()));
                        if let Some(c) = ctx {
                            c.open_url(egui::OpenUrl::new_tab(url));
                        }
                    };
                    drive.auth.sign_in(&open, cancel, SIGN_IN_TIMEOUT).map(|()| Event::SignedIn(true))
                }
                Job::SignOut => drive.auth.sign_out().map(|()| Event::SignedIn(false)),
                Job::List { search, page } => {
                    let append = page.is_some();
                    drive.list(search.as_deref(), page.as_deref()).map(|(files, next)| Event::Listed { files, next, append })
                }
                Job::Open { id } => drive.metadata(&id).and_then(|file| drive.download(&file).map(|bytes| Event::Opened { file, bytes })),
                Job::Save { doc, rev, local, target, bytes, marked_saved } => {
                    drive.save(&target, &bytes).map(|file| Event::Saved { doc, rev, local, file, marked_saved })
                }
                Job::Export { name, bytes } => drive.save(&SaveTarget::New { name, parent: None }, &bytes).map(Event::Exported),
            };
            emit(r.unwrap_or_else(|error| Event::Failed { what, error, doc }));
            if !drive.auth.signed_in() {
                emit(Event::SignedIn(false));
            }
        }
    }

    fn job_name(job: &Job) -> &'static str {
        match job {
            Job::SignIn => "signIn",
            Job::SignOut => "signOut",
            Job::List { .. } => "list",
            Job::Open { .. } => "open",
            Job::Save { .. } => "save",
            Job::Export { .. } => "export",
        }
    }

    fn job_doc(job: &Job) -> Option<u64> {
        match job {
            Job::Save { doc, .. } => Some(*doc),
            _ => None,
        }
    }

    pub fn poll(app: &mut WordApp) {
        while let Some(event) = app.drive.worker.as_ref().and_then(|w| w.events.try_recv().ok()) {
            apply(app, event);
        }
    }

    pub(super) fn apply(app: &mut WordApp, event: Event) {
        let d = &mut app.drive;
        match event {
            Event::Unavailable(why) => {
                d.unavailable = Some(why);
                d.signed_in = Some(false);
                d.busy = None;
            }
            Event::Restored(yes) => {
                if d.busy == Some("starting") {
                    d.busy = None;
                }
                d.signed_in = Some(yes);
            }
            Event::SignedIn(yes) => {
                // Sign-in and sign-out finish here; a job that signed the person out on the way
                // (a revoked token) reports its own failure separately.
                if matches!(d.busy, Some("signIn" | "signOut")) {
                    d.busy = None;
                    d.consent_url = None;
                }
                if d.signed_in == Some(true) && !yes {
                    d.files.clear();
                    d.next_page = None;
                }
                d.listed = false;
                d.signed_in = Some(yes);
            }
            Event::ConsentUrl(url) => d.consent_url = Some(url),
            Event::Listed { files, next, append } => {
                d.busy = None;
                if !append {
                    d.files.clear();
                }
                d.files.extend(files.into_iter().map(|f| Listed {
                    google_doc: f.is_google_doc(),
                    id: f.id,
                    name: f.name,
                    modified: f.modified_time,
                }));
                d.next_page = next;
            }
            Event::Opened { file, bytes } => {
                d.busy = None;
                open(app, file, &bytes);
            }
            Event::Saved { doc, rev, local, file, marked_saved } => {
                d.busy = None;
                let name = file.name.clone();
                d.origins.insert(doc, Origin { google_doc: file.is_google_doc(), id: file.id, name: file.name, version: file.version });
                // Only the document that was saved, and only if nothing was typed while it uploaded.
                // A local save that set the document's home while this one uploaded (a Save As,
                // or Save to the same local file) is newer: it keeps that destination.
                let newer_local = d.local_saves.get(&doc).copied().unwrap_or(0) != local;
                if app.session.document_id() == doc && !newer_local {
                    // Drive is now where the document lives: Save goes back there, not to a
                    // local file it was saved to earlier.
                    app.session.path = None;
                    app.session.file_name = Some(name.clone());
                    if marked_saved && app.session.rev() == rev {
                        app.session.dirty = false;
                    }
                }
                app.status(i18n::fmt(tl!("Saved {name} to Google Drive"), &[("name", &name)]));
            }
            Event::Exported(file) => {
                d.busy = None;
                app.status(i18n::fmt(tl!("Exported {name} to Google Drive"), &[("name", &file.name)]));
            }
            Event::Failed { what, error, doc } => {
                d.busy = None;
                d.consent_url = None;
                if matches!(error, DriveError::Conflict { .. }) {
                    d.conflict = doc;
                }
                let msg = match &error {
                    DriveError::Conflict { name, .. } => i18n::fmt(
                        tl!("{name} was changed in Google Drive since you opened it. Nothing was overwritten: save a copy instead."),
                        &[("name", name)],
                    ),
                    DriveError::NotSignedIn => tl!("Sign in to Google Drive first.").to_string(),
                    e => i18n::fmt(tl!("Google Drive: {error}"), &[("error", &e.to_string())]),
                };
                log::warn!("Google Drive {what} failed: {error}");
                d.error = Some(msg.clone());
                app.status(msg);
            }
        }
    }

    /// Opens downloaded bytes the way File › Open opens a file's bytes, then makes the document
    /// a Drive document: no local path (so Save never writes beside the app, and nothing joins
    /// the recent files) and its Drive origin.
    fn open(app: &mut WordApp, file: DriveFile, bytes: &[u8]) {
        let name = file.open_name();
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        match app.session.run("file.open", &json!({"path": name, "data": data})) {
            Ok(_) => {
                app.session.path = None;
                app.session.file_name = Some(name.clone());
                app.session.dirty = false;
                let doc = app.session.document_id();
                app.drive.origins.insert(doc, Origin { google_doc: file.is_google_doc(), id: file.id, name: file.name, version: file.version });
                let _ = app.run("ui.backstage", json!({"value": false}));
                app.canvas.want_focus = true;
                app.status(i18n::fmt(tl!("Opened {name} from Google Drive"), &[("name", &name)]));
            }
            Err(e) => {
                let msg = i18n::fmt(tl!("Couldn't open {name}: {error}"), &[("name", &name), ("error", &e.to_string())]);
                app.drive.error = Some(msg.clone());
                app.status(msg);
            }
        }
    }
}

#[cfg(test)]
mod tests;
