//! File dialogs (Open, Save As, Export, Insert/Change Picture) that don't freeze the window (#94).
//!
//! Every dialog is a ticket on the host's file service (`host.files().pick`). [`WordApp::logic`]
//! takes the answer on a later frame and finishes what the dialog was for (open the file, save
//! and carry on with a pending New/Open/Close, export, insert the picture). A service that
//! already holds the answer finishes the job inside the request. One dialog at a time: asking
//! for another while one is open does nothing.
//!
//! When no system dialog can open (a Linux desktop with neither a file chooser portal nor
//! zenity, #576) the service opens the framework's fallback chooser itself; the app only asks
//! for it by name with [`PickRequest::fallback`].

use craft::host::{PickId, PickRequest, Picked};
use serde_json::{Value, json};

use crate::WordApp;

/// Most font files one Add Font pick tries; the rest of a bigger selection is skipped.
const MAX_FONTS_PER_PICK: usize = 2000;

/// Picture suffixes shared by native/web choosers, drop dispatch and copied-file paste.
pub const PICTURES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "wmf", "emf"];
const DOCUMENTS: &[&str] =
    &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "ott", "fodt", "rtf", "txt", "md", "html", "htm", "mht", "mhtml", "tex", "json", "xml"];

/// The file types Open offers, as (label, extensions; `*` is any file): pictures for Insert/Change
/// Picture, recipient lists for Select Recipients › Use an Existing List…, documents otherwise.
pub fn open_filters(purpose: &str) -> &'static [(&'static str, &'static [&'static str])] {
    match purpose {
        "picture" => &[("Pictures", PICTURES)],
        "font" => &[("Fonts", wordcraft_fonts::FONT_FILE_EXTS)],
        "sources" => &[("Word sources", &["xml"]), ("All files", &["*"])],
        "recipients" => &[("Recipient lists", wordcraft_engine::cmd::mailings::RECIPIENT_EXTENSIONS), ("All files", &["*"])],
        _ => &[("Documents", DOCUMENTS), ("Word document", &["docx"]), ("All files", &["*"])],
    }
}

/// Whether a file name has a picture suffix offered by the Insert/Change Picture chooser.
pub(crate) fn is_picture_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| PICTURES.iter().any(|known| known.eq_ignore_ascii_case(ext)))
}

/// `request` with the file types `open_filters` offers for `purpose`.
fn with_open_filters(mut request: PickRequest, purpose: &str) -> PickRequest {
    for (label, exts) in open_filters(purpose) {
        request = request.filter(tl!(label), exts);
    }
    request
}

/// A path as the status line shows it: isolated in a right-to-left interface, unchanged otherwise.
fn interface_path(path: &str) -> String {
    let dir = if crate::i18n::current().rtl() { craft::i18n::Direction::Rtl } else { craft::i18n::Direction::Ltr };
    craft::i18n::path(path, dir)
}

/// What to do with the path picked in a dialog.
pub(crate) enum AfterPick {
    /// Open the picked document from the window showing `document` ([`WordApp::open_file`]).
    OpenDocument { document: u64 },
    /// Insert the picked picture, or replace the pending Change Picture target.
    Picture,
    /// Save the document there, then carry on.
    SaveAs { after: AfterSave, document: u64 },
    /// Save a copy there in this format (`pdf`, `docx`, `png`…).
    Export { ext: String },
    /// Write the PDF of these print settings there (File › Print › Save these pages as PDF).
    PrintPdf { settings: Value },
    /// Load the picked CSV/TSV/text file or spreadsheet as the mail-merge recipients (#240, #334).
    Recipients,
    /// Add every picked font file (#582, #1789).
    Font,
    /// Import the picked Word bibliography file (`b:Sources`: Word's `Sources.xml`) into the
    /// document through Manage Sources (#2640).
    #[cfg(not(target_arch = "wasm32"))]
    Sources,
}

/// What to do once Save As has written the document.
pub(crate) enum AfterSave {
    Nothing,
    /// The AutoSave switch asked for the save: turn AutoSave on.
    AutoSave,
    /// "Save changes?" answered Save: the command it held back (New, Open, Close…) lives in
    /// `closing` and runs once the document is safely written and is still the one asked about.
    Closing,
}

/// A dialog the file service is showing; its answer is taken by `id`.
pub(crate) struct PendingDialog {
    id: PickId,
    after: AfterPick,
}

/// How asking for a file went.
pub(crate) enum Asked {
    /// Another file dialog is still open; nothing happened.
    Busy,
    /// The service shows the dialog; the answer is handled on a later frame.
    Pending,
    /// The service answered on its first poll and the follow-up ran.
    Done(Result<Value, String>),
}

impl WordApp {
    /// Whether a file dialog is open (its answer hasn't arrived yet).
    pub fn file_dialog_open(&self) -> bool {
        self.file_dialog.is_some()
    }

    /// Forces the fallback chooser for every file dialog (`WORDCRAFT_FILE_DIALOG=builtin`).
    pub fn with_fallback_chooser(mut self, on: bool) -> Self {
        self.fallback_chooser = on;
        self
    }

    /// Ask the file service for a file and do `after` with the answer. Called in the pass of the
    /// user's gesture, since the browser's picker needs its activation. Ignored while another
    /// dialog is open.
    pub(crate) fn ask_file(&mut self, req: PickRequest, after: AfterPick) -> Asked {
        if self.file_dialog_open() {
            log::debug!("file dialog already open; ignoring {req:?}");
            self.status(tl!("A file dialog is already open."));
            return Asked::Busy;
        }
        let fallback = req.fallback || self.fallback_chooser;
        let id = self.host.files().pick(req.fallback(fallback));
        self.file_dialog = Some(PendingDialog { id, after });
        // A service that answered at once is handled right away.
        self.poll_file_dialog().map_or(Asked::Pending, Asked::Done)
    }

    /// Pick up the answer of an open file dialog, if it has arrived (every frame, from `logic`),
    /// and finish what it was for. `None` while there is nothing to finish.
    pub(crate) fn poll_file_dialog(&mut self) -> Option<Result<Value, String>> {
        let id = self.file_dialog.as_ref()?.id;
        let picked = self.host.files().take_answer(id)?;
        let d = self.file_dialog.take()?;
        if matches!(d.after, AfterPick::Recipients) {
            self.recipient_list_pending = false;
        }
        match picked {
            Picked::Cancelled => Some(self.finish_pick(d.after, None)),
            Picked::Unavailable(cause) => {
                #[cfg(not(target_arch = "wasm32"))]
                self.status(craft::dialogs::ui::UnavailableWords::default().sentence(&cause));
                #[cfg(target_arch = "wasm32")]
                self.status(format!("No file dialog can open: {cause}"));
                Some(self.finish_pick(d.after, None))
            }
            Picked::Failed(reason) => {
                // A plain Save As has no prompt to carry the reason to and Backstage hides the
                // status bar, so the Save As page says it too.
                if matches!(d.after, AfterPick::SaveAs { after: AfterSave::Nothing, .. }) {
                    self.fail_save_as(&reason);
                } else {
                    self.status(reason);
                }
                Some(self.finish_pick(d.after, None))
            }
            Picked::Paths(paths) => {
                let paths = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                Some(self.finish_picks(d.after, paths))
            }
            Picked::Saved { path, .. } => Some(self.finish_picks(d.after, vec![path.to_string_lossy().into_owned()])),
            Picked::Contents(files) => Some(self.finish_contents(d.after, files)),
            // The page handed the save over as a download: the save finishes under that name, which
            // the engine answers with `downloaded`, and the document keeps the file it had.
            Picked::Downloaded(name) => Some(self.finish_pick(d.after, Some(name))),
        }
    }

    /// Treat the dialog the service is showing as cancelled, e.g. one that never showed up;
    /// returns the format of the export it was for, if any. `ui.fileBrowser` reuses it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn give_up_file_dialog(&mut self) -> Option<String> {
        let d = self.file_dialog.take()?;
        self.host.files().forget(d.id);
        let export = if let AfterPick::Export { ext } = &d.after { Some(ext.clone()) } else { None };
        let _ = self.finish_pick(d.after, None);
        export
    }

    /// Do what the dialog was for with the picked paths (none: the user cancelled). Only Add Font
    /// takes more than one; the others use the first.
    fn finish_picks(&mut self, after: AfterPick, picked: Vec<String>) -> Result<Value, String> {
        match after {
            AfterPick::Font if !picked.is_empty() => self.add_fonts(picked.into_iter().map(|p| (p.clone(), json!({"path": p}))).collect()),
            after => self.finish_pick(after, picked.into_iter().next()),
        }
    }

    /// Do what the dialog was for with files the host handed over as bytes (the browser's picker
    /// reads them: a name and the data, no path). Only Add Font takes more than one.
    fn finish_contents(&mut self, after: AfterPick, mut files: Vec<(String, Vec<u8>)>) -> Result<Value, String> {
        use wordcraft_engine::cmd::insert::base64_encode;
        if files.is_empty() {
            return self.finish_pick(after, None);
        }
        match after {
            AfterPick::Font => {
                let fonts = files.into_iter().map(|(name, bytes)| (name.clone(), json!({"name": name, "data": base64_encode(&bytes)}))).collect();
                self.add_fonts(fonts)
            }
            after => {
                let (name, bytes) = files.swap_remove(0);
                let data = base64_encode(&bytes);
                match after {
                    AfterPick::OpenDocument { .. } => {
                        let r = self.run("file.open", json!({"path": name, "data": data}));
                        self.ui.backstage = false;
                        r
                    }
                    AfterPick::Picture => self.insert_or_change_picture(json!({"data": data})),
                    AfterPick::Recipients => self.load_recipients(json!({"path": name, "data": data})),
                    _ => Err("this kind of pick answers with a path".into()),
                }
            }
        }
    }

    /// Add each picked font file, going on past the ones that fail, and say how it went in one
    /// status line (#1789). Each font is its display name and the `font.add` params that read it;
    /// one font keeps `font.add`'s own result and message.
    fn add_fonts(&mut self, mut fonts: Vec<(String, Value)>) -> Result<Value, String> {
        if fonts.len() == 1 {
            return self.run("font.add", fonts.remove(0).1);
        }
        let skipped = fonts.len().saturating_sub(MAX_FONTS_PER_PICK);
        fonts.truncate(MAX_FONTS_PER_PICK);
        let (mut added, mut failed) = (0usize, Vec::new());
        for (path, params) in fonts {
            match self.run("font.add", params) {
                Ok(_) => added += 1,
                Err(e) => {
                    log::warn!("couldn't add font {path}: {e}");
                    let name = std::path::Path::new(&path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned());
                    failed.push(name);
                }
            }
        }
        let n = added.to_string();
        let mut msg = if added == 1 { tl!("Added 1 font").to_string() } else { crate::i18n::fmt(tl!("Added {n} fonts"), &[("n", &n)]) };
        if !failed.is_empty() {
            let count = failed.len().to_string();
            let mut names = failed.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            if failed.len() > 3 {
                let more = (failed.len() - 3).to_string();
                names = crate::i18n::fmt(tl!("{names} and {n} more"), &[("names", &names), ("n", &more)]);
            }
            msg = crate::i18n::fmt(tl!("{added}; {count} couldn't be read: {names}"), &[("added", &msg), ("count", &count), ("names", &names)]);
        }
        if skipped > 0 {
            let (n, max) = (skipped.to_string(), MAX_FONTS_PER_PICK.to_string());
            msg = crate::i18n::fmt(tl!("{added}; {n} skipped (limit {max})"), &[("added", &msg), ("n", &n), ("max", &max)]);
        }
        self.status(msg);
        Ok(json!({"added": added, "failed": failed, "skipped": skipped}))
    }

    /// Do what the dialog was for with the picked path (`None`: the user cancelled).
    pub(crate) fn finish_pick(&mut self, after: AfterPick, picked: Option<String>) -> Result<Value, String> {
        match after {
            AfterPick::OpenDocument { document } => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                let r = self.open_file(&path, Some(document));
                self.ui.backstage = false;
                r
            }
            AfterPick::Picture => {
                let Some(path) = picked else {
                    self.change_picture_target = None;
                    return Ok(json!({"cancelled": true}));
                };
                self.insert_or_change_picture(json!({"path": path}))
            }
            AfterPick::Font => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                self.add_fonts(vec![(path.clone(), json!({"path": path}))])
            }
            #[cfg(not(target_arch = "wasm32"))]
            AfterPick::Sources => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                let before = self.session.doc.sources.len();
                let r = self.run("references.sources", json!({"import": path}));
                if r.is_ok() {
                    let added = self.session.doc.sources.len().saturating_sub(before);
                    let msg = match added {
                        0 => tl!("No new sources in that file").to_string(),
                        1 => tl!("Imported 1 source").to_string(),
                        n => crate::i18n::fmt(tl!("Imported {n} sources"), &[("n", &n.to_string())]),
                    };
                    self.status(msg);
                }
                r
            }
            AfterPick::Recipients => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                self.load_recipients(json!({"path": path}))
            }
            AfterPick::Export { ext } => {
                let Some(mut path) = picked else {
                    // Cancelled: an earlier refusal is not news any more.
                    self.clear_export_error();
                    return Ok(json!({"cancelled": true}));
                };
                let r = if ext == "png" {
                    self.run("file.exportPng", json!({"path": path}))
                } else if ext == "xml" || ext == crate::print_page::EXPORT_WORD_2003 {
                    // The engine takes an explicit format only for an .xml path, and a system Save
                    // dialog lets the user type any name: give it the extension
                    // (`notes.docx` becomes `notes.xml`, `notes.v2` `notes.v2.xml`).
                    let named = crate::print_page::export_name(&path, "xml");
                    if named != path {
                        path = named;
                        // The dialog only confirmed the name it was given, not this one.
                        if std::path::Path::new(&path).exists() {
                            // Callers drop the result, so say it where the person sees it.
                            return Err(self.fail_export(format!("{path} already exists; choose another name")));
                        }
                    }
                    // Word XML even over a Word 2003 XML file, which a save by path would keep.
                    let format = if ext == "xml" { wordcraft_engine::cmd::file::FORMAT_FLAT_OPC } else { crate::print_page::EXPORT_WORD_2003 };
                    self.run("file.saveAs", json!({"path": path, "format": format}))
                } else {
                    self.run("file.saveAs", json!({"path": path}))
                };
                match &r {
                    Ok(_) => {
                        self.clear_export_error();
                        self.status(crate::i18n::fmt(tl!("Exported {path}"), &[("path", &interface_path(&path))]));
                    }
                    Err(e) => {
                        self.fail_export(e.clone());
                    }
                }
                r
            }
            AfterPick::PrintPdf { settings } => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                let mut params = settings;
                params["path"] = json!(path);
                let r = self.run("file.printPdf", params);
                if r.is_ok() {
                    self.status(crate::i18n::fmt(tl!("Exported {path}"), &[("path", &interface_path(&path))]));
                }
                r
            }
            AfterPick::SaveAs { after, document } => {
                // The document it was for may wait behind another one: it is shown and saved.
                if document != self.session.document_id() {
                    if !self.session.is_open(document) {
                        return Err("the document for this save has been closed".into());
                    }
                    self.show_document(document)?;
                }
                // The save's error, kept for the Save Changes prompt to explain (#1445). A plain
                // Save As (Backstage's Browse…, AutoSave) has no prompt to carry it to, so a
                // failed write would otherwise leave the Save As page with nothing to say why,
                // since its own render loop skips the status bar while Backstage is open.
                // `fail_save_as` reaches both.
                let result = picked.map(|path| self.run("file.save", json!({"path": path})));
                let saved = matches!(&result, Some(Ok(v)) if v.get("saved").and_then(Value::as_bool) == Some(true) || v.get("downloaded").is_some());
                if saved {
                    self.ui.backstage = false;
                    if let Some(ctx) = self.ctx.clone() {
                        self.canvas.focus_page(&ctx);
                    }
                    self.clear_save_as_error();
                }
                match after {
                    AfterSave::Nothing => {
                        if let Some(Err(error)) = &result {
                            self.fail_save_as(error);
                        }
                        Ok(json!({"saved": saved}))
                    }
                    AfterSave::AutoSave => {
                        if saved {
                            self.autosave = true;
                        } else if let Some(Err(error)) = &result {
                            self.fail_save_as(error);
                        }
                        Ok(json!({"saved": saved}))
                    }
                    // PLAN(shell-w8): sh.A7 continue_after_save
                    AfterSave::Closing => self.continue_after_save(result, saved),
                }
            }
        }
    }

    /// Ask where to save, then save there (File › Save As).
    pub fn save_as_dialog(&mut self) {
        let _ = self.save_as(AfterSave::Nothing);
    }

    /// A plain Save As (Backstage's Browse…, AutoSave) failed to write: say so on the status bar,
    /// and on the Save As page, whose render loop skips the status bar while Backstage is open.
    fn fail_save_as(&mut self, error: &str) {
        self.status(error.to_string());
        self.backstage_state.save_as_error = Some(error.to_string());
    }

    /// An export failed or was refused: say so on the status bar and, for the Export page,
    /// there too, since Backstage's render loop skips the status bar (callers drop the result).
    fn fail_export(&mut self, error: String) -> String {
        self.status(error.clone());
        self.export_error = error.clone();
        error
    }

    fn clear_export_error(&mut self) {
        self.export_error.clear();
    }

    /// Drop a Save As page error once a later attempt has actually saved.
    fn clear_save_as_error(&mut self) {
        self.backstage_state.clear_errors();
    }

    /// Ask where to save, save there, then do `after`.
    pub(crate) fn save_as(&mut self, after: AfterSave) -> Asked {
        let name = self.default_save_name();
        let mut req = PickRequest::save("document").file_name(name.clone()).save_as(true);
        req.filters = crate::save_types::save_filters(&name);
        // Start in the open file's folder (#4143); a document never saved starts in the
        // remembered one. A path opened as `one.docx` has an empty parent, so it's made absolute
        // first.
        let opened = self.session.path.as_deref().and_then(|p| std::path::absolute(p).ok());
        if let Some(dir) = opened.as_deref().and_then(std::path::Path::parent) {
            req = req.directory(dir);
        }
        self.ask_file(req, AfterPick::SaveAs { after, document: self.session.document_id() })
    }

    /// The user templates folder (#2852): the one File › Options names, else the app's own.
    /// `None` on the web, which has no folders.
    pub fn user_templates_dir(&self) -> Option<std::path::PathBuf> {
        // The saved folder is checked again (the settings file may have been edited, the folder
        // removed): a network folder is never looked at, and one that's gone gives way to the
        // app's own.
        let saved = self.session.prefs.user_templates_folder.as_deref().map(str::trim).filter(|f| !f.is_empty());
        match saved {
            Some(f) if wordcraft_engine::cmd::user_templates::check_dir(f).is_ok() && std::path::Path::new(f).is_dir() => {
                Some(std::path::PathBuf::from(f))
            }
            Some(f) => {
                log::debug!("user templates folder {f:?} isn't usable; using the app's own");
                self.services.templates_dir.clone()
            }
            None => self.services.templates_dir.clone(),
        }
    }

    /// File › Save As › Save as Word Template (#2852): ask where to save a `.dotx`, starting in
    /// the user templates folder (the app's own is made on first use), then save there as Save As
    /// does. Returns the folder the dialog starts in.
    pub(crate) fn save_as_template_dialog(&mut self) -> Option<String> {
        let dir = self.user_templates_dir();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(d) = &dir
            && !d.is_dir()
            && let Err(e) = std::fs::create_dir_all(d)
        {
            log::warn!("couldn't make the templates folder {}: {e}", d.display());
        }
        let dir = dir.filter(|d| d.is_dir());
        let name = format!("{}.dotx", self.title_stem());
        let mut req = PickRequest::save("template").file_name(name).save_as(true).filter(tl!("Word template"), &["dotx"]);
        if let Some(d) = &dir {
            req = req.directory(d.clone());
        }
        let after = AfterPick::SaveAs { after: AfterSave::Nothing, document: self.session.document_id() };
        let _ = self.ask_file(req, after);
        dir.map(|d| d.to_string_lossy().into_owned())
    }

    /// File › Export: ask where to save a copy in this format, then write it.
    pub(crate) fn export_dialog(&mut self, ext: &str) {
        let req = self.export_request(ext);
        // A refusal or failure is reported by `finish_pick` itself (Export page and status bar).
        let _ = self.ask_file(req, AfterPick::Export { ext: ext.to_string() });
    }

    /// The request that asks where to save a copy in export format `ext`: the format's name and type,
    /// so the chooser's type menu shows it. Also what Browse in WordCraft… asks again for a pending export.
    pub(crate) fn export_request(&self, ext: &str) -> PickRequest {
        let file_ext = crate::print_page::export_file_ext(ext).to_string();
        let name = format!("{}.{}", self.title_stem(), file_ext);
        let mut req = PickRequest::save("export").file_name(name).suggested_extension(file_ext.clone());
        if let Some(&(label, _)) = crate::print_page::EXPORT_FORMATS.iter().find(|&&(_, e)| e == ext) {
            req = req.filter(tl!(label), &[file_ext.as_str()]);
        }
        req
    }

    /// File › Print › Save these pages as PDF: ask where, then write the pages the print
    /// settings choose.
    pub(crate) fn print_pdf_dialog(&mut self) {
        let name = format!("{}.pdf", self.title_stem());
        let settings = self.ui.print.params();
        let req = PickRequest::save("export").file_name(name).suggested_extension("pdf").filter(tl!("PDF"), &["pdf"]);
        let _ = self.ask_file(req, AfterPick::PrintPdf { settings });
    }

    pub(crate) fn open_dialog(&mut self) {
        let req = with_open_filters(PickRequest::open("document"), "document");
        let _ = self.ask_file(req, AfterPick::OpenDocument { document: self.session.document_id() });
    }

    pub(crate) fn pick_picture(&mut self) {
        let req = with_open_filters(PickRequest::open("picture"), "picture");
        let _ = self.ask_file(req, AfterPick::Picture);
    }

    /// Pick one or more font files to add (Add Font…).
    pub(crate) fn pick_font(&mut self) {
        let req = with_open_filters(PickRequest::open_many("font"), "font");
        let _ = self.ask_file(req, AfterPick::Font);
    }

    /// Request loading local system installed fonts (web: queryLocalFonts).
    pub(crate) fn load_system_fonts(&mut self) {
        if let Some(f) = &self.services.load_system_fonts {
            f();
        } else {
            self.status(tl!("All system fonts are already available."));
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod picture_dispatch_tests {
    use std::sync::Arc;

    use craft::host::{AppInfo, Host, Picked, ScriptedFiles};
    use craft::platform::open::{DroppedFile, FileIntake};

    use super::*;

    /// Pickers, drops and copied paths must agree on TIFF suffixes after the framework cutover.
    #[test]
    fn tiff_suffixes_dispatch_picked_and_dropped_images_and_copied_paths_as_pictures() {
        for ext in ["tif", "tiff", "TIF", "TIFF"] {
            let name = format!("scan.{ext}");
            // Routing depends on the supplied name; the insertion decoder probes these PNG bytes.
            let bytes = crate::paste_picture::png_from_rgba(4, 4, vec![200; 4 * 4 * 4]).unwrap();
            let files = Arc::new(ScriptedFiles::held());
            let host = Host::builder(AppInfo { id: "wordcraft-test", name: "WordCraft", version: "0" }).files(files.clone()).build();
            let session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
            let mut app = WordApp::new(session, crate::Services::default()).with_host(Arc::new(host));
            let document = app.session.document_id();
            app.pick_picture();
            let request = files.requests().pop().unwrap();
            let extensions: Vec<_> = request.filters.iter().flat_map(|f| f.extensions.iter().map(String::as_str)).collect();
            assert!(extensions.contains(&"tif") && extensions.contains(&"tiff") && extensions.contains(&"png"));
            assert!(!extensions.contains(&"txt"));
            files.answer(Picked::Contents(vec![(name.clone(), bytes.clone())]));
            app.poll_file_dialog().unwrap().unwrap();
            assert_eq!(app.session.document_id(), document, "picker inserts into the current document: {ext}");
            let pictures = app.session.run("arrange.selectionPane", &json!({})).unwrap();
            assert_eq!(pictures.as_array().unwrap().len(), 1, "picker inserts a picture: {ext}");

            let (intake, sender) = FileIntake::new(craft::host::Wake::none());
            let services = crate::Services { intake: Some(intake), ..Default::default() };
            let mut dropped = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
            let document = dropped.session.document_id();
            sender.dropped(vec![DroppedFile { path: None, name, bytes: Some(bytes.into()) }]);
            dropped.drain_inbox();
            assert_eq!(dropped.session.document_id(), document, "drop inserts into the current document: {ext}");
            let pictures = dropped.session.run("arrange.selectionPane", &json!({})).unwrap();
            assert_eq!(pictures.as_array().unwrap().len(), 1, "drop inserts a picture: {ext}");
            assert!(crate::paste_picture::names_picture_files(&format!("file:///tmp/scan.{ext}")));
        }
        assert!(!is_picture_name("notes.txt"));
    }
}
