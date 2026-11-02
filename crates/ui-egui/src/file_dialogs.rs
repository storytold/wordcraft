//! File dialogs (Open, Save As, Export, Insert/Change Picture) that don't freeze the window (#94).
//!
//! A host with [`Services::file_dialog`](crate::Services::file_dialog) shows the native dialog
//! without blocking the UI thread and sends the answer back later; [`WordApp::logic`] picks it up
//! on a later frame and finishes what the dialog was for (open the file, save and carry on with a
//! pending New/Open/Close, export, insert the picture). A host with only the blocking
//! `pick_open` / `pick_save` gets the same follow-up straight away. One dialog at a time: asking
//! for another while one is open does nothing.
//!
//! When the host can't show a system dialog at all (a Linux desktop with neither a file chooser
//! portal nor zenity, #576) it answers with an error, and the app finishes the job in its own
//! [`file_browser`](crate::file_browser) instead.

use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};

use crate::WordApp;

/// A file dialog the app asks the host to show ([`Services::file_dialog`](crate::Services::file_dialog)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogRequest {
    /// Pick a file to open; `purpose` says what for (`document`, `picture`, `recipients`, `font`,
    /// `sources`), as for `pick_open`.
    Open { purpose: String },
    /// Pick one or more files (Add Font…); the answer lists every one picked.
    OpenMany { purpose: String },
    /// Pick a path to save to, given a suggested file name and, if it matters, the folder to
    /// start in (Save as Word Template starts in the user templates folder, #2852).
    Save { name: String, dir: Option<String> },
}

/// The host's answer to a [`FileDialogRequest`]: the picked paths (one for everything but
/// [`FileDialogRequest::OpenMany`]), empty when the user cancelled, or an error when no system
/// dialog could be shown (the app then shows its own).
pub type FileDialogAnswer = Result<Vec<String>, String>;

/// Most font files one Add Font pick tries; the rest of a bigger selection is skipped.
const MAX_FONTS_PER_PICK: usize = 2000;

const PICTURES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "wmf", "emf"];
const DOCUMENTS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "rtf", "txt", "md", "html", "htm", "tex", "json", "xml"];

/// The file types Open offers, as (label, extensions; `*` is any file): pictures for Insert/Change
/// Picture, recipient lists for Select Recipients › Use an Existing List…, documents otherwise.
/// The host's system dialog and the built-in browser offer the same.
pub fn open_filters(purpose: &str) -> &'static [(&'static str, &'static [&'static str])] {
    match purpose {
        "picture" => &[("Pictures", PICTURES)],
        "font" => &[("Fonts", wordcraft_fonts::FONT_FILE_EXTS)],
        "sources" => &[("Word sources", &["xml"]), ("All files", &["*"])],
        "recipients" => &[("Recipient lists", wordcraft_engine::cmd::mailings::RECIPIENT_EXTENSIONS), ("All files", &["*"])],
        _ => &[("Documents", DOCUMENTS), ("Word document", &["docx"]), ("All files", &["*"])],
    }
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
    /// Hand the picked path to the open dialog (`WordApp::dialog_pick`): Templates and Add-ins,
    /// Organizer (#377).
    ForDialog,
    /// Put the picked document into the Compare dialog (original or revised, #412).
    ComparePath { revised: bool },
}

/// What to do once Save As has written the document.
pub(crate) enum AfterSave {
    Nothing,
    /// The AutoSave switch asked for the save: turn AutoSave on.
    AutoSave,
    /// "Save changes?" answered Save: run the command it held back (New, Open, Close…), as long as
    /// the document is safely written and is still the one the prompt asked about.
    Continue {
        then: String,
        params: Value,
        document: u64,
    },
}

/// A dialog the host is showing; the answer arrives on `rx` (`None`: cancelled).
pub(crate) struct PendingDialog {
    rx: Receiver<FileDialogAnswer>,
    req: FileDialogRequest,
    after: AfterPick,
}

/// How asking for a file went.
pub(crate) enum Asked {
    /// Another file dialog is still open; nothing happened.
    Busy,
    /// The host shows the dialog; the answer is handled on a later frame.
    Pending,
    /// The host answered straight away and the follow-up ran.
    Done(Result<Value, String>),
}

impl WordApp {
    /// Whether a file dialog is open (its answer hasn't arrived yet), the host's or the built-in one.
    pub fn file_dialog_open(&self) -> bool {
        self.file_dialog.is_some() || self.file_browser_open()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_browser_open(&self) -> bool {
        self.file_browser.open.is_some()
    }

    #[cfg(target_arch = "wasm32")]
    fn file_browser_open(&self) -> bool {
        false
    }

    /// Ask the host for a file and do `after` with the answer: now for a blocking dialog, on a
    /// later frame for an asynchronous one. Ignored while another dialog is open.
    pub(crate) fn ask_file(&mut self, req: FileDialogRequest, after: AfterPick) -> Asked {
        if self.file_dialog_open() {
            log::debug!("file dialog already open; ignoring {req:?}");
            self.status(tl!("A file dialog is already open."));
            return Asked::Busy;
        }
        if let Some(show) = &self.services.file_dialog {
            let rx = show(req.clone());
            self.file_dialog = Some(PendingDialog { rx, req, after });
            // A host that answered at once (or failed to show anything) is handled right away.
            return self.poll_file_dialog().map_or(Asked::Pending, Asked::Done);
        }
        let picked = match &req {
            FileDialogRequest::Open { purpose } | FileDialogRequest::OpenMany { purpose } => {
                self.services.pick_open.as_ref().and_then(|f| f(purpose))
            }
            FileDialogRequest::Save { name, .. } => self.services.pick_save.as_ref().and_then(|f| f(name)),
        };
        Asked::Done(self.finish_pick(after, picked))
    }

    /// Pick up the answer of an open file dialog, if it has arrived (every frame, from `logic`),
    /// and finish what it was for. `None` while there is nothing to finish.
    pub(crate) fn poll_file_dialog(&mut self) -> Option<Result<Value, String>> {
        let answer = match self.file_dialog.as_ref().map(|d| d.rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => return None,
            Some(Ok(answer)) => answer,
            // The host dropped the dialog without an answer: as good as cancelled.
            Some(Err(TryRecvError::Disconnected)) => Ok(Vec::new()),
        };
        let d = self.file_dialog.take()?;
        match answer {
            Ok(picked) => Some(self.finish_picks(d.after, picked)),
            Err(reason) => {
                log::warn!("no system file dialog ({reason}); showing the built-in file browser");
                self.dialog_unavailable(d.req, d.after)
            }
        }
    }

    /// Treat the system dialog the host is showing as cancelled, e.g. one that never showed up.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn give_up_file_dialog(&mut self) {
        if let Some(d) = self.file_dialog.take() {
            let _ = self.finish_pick(d.after, None);
        }
    }

    /// The host couldn't show a system dialog (#576): ask in the built-in browser instead, which
    /// finishes the job later. `None` while it is open.
    #[cfg(not(target_arch = "wasm32"))]
    fn dialog_unavailable(&mut self, req: FileDialogRequest, after: AfterPick) -> Option<Result<Value, String>> {
        self.show_file_browser(req, after, true);
        None
    }

    /// The browser has no built-in file browser (it can't list the user's folders): cancelled.
    #[cfg(target_arch = "wasm32")]
    fn dialog_unavailable(&mut self, _req: FileDialogRequest, after: AfterPick) -> Option<Result<Value, String>> {
        Some(self.finish_pick(after, None))
    }

    /// Do what the dialog was for with the picked paths (none: the user cancelled). Only Add Font
    /// takes more than one; the others use the first.
    fn finish_picks(&mut self, after: AfterPick, picked: Vec<String>) -> Result<Value, String> {
        match after {
            AfterPick::Font if !picked.is_empty() => self.add_fonts(picked),
            after => self.finish_pick(after, picked.into_iter().next()),
        }
    }

    /// Add each picked font file, going on past the ones that fail, and say how it went in one
    /// status line (#1789). One file keeps `font.add`'s own result and message.
    fn add_fonts(&mut self, mut paths: Vec<String>) -> Result<Value, String> {
        if paths.len() == 1 {
            return self.run("font.add", json!({"path": paths.remove(0)}));
        }
        let skipped = paths.len().saturating_sub(MAX_FONTS_PER_PICK);
        paths.truncate(MAX_FONTS_PER_PICK);
        let (mut added, mut failed) = (0usize, Vec::new());
        for path in paths {
            match self.run("font.add", json!({"path": path})) {
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
                self.add_fonts(vec![path])
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
            AfterPick::ForDialog => {
                let picked = picked.filter(|p| !p.is_empty());
                let r = json!({"picked": picked});
                self.dialog_pick = picked;
                Ok(r)
            }
            AfterPick::Recipients => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                self.load_recipients(json!({"path": path}))
            }
            AfterPick::ComparePath { revised } => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                crate::dialogs_review::set_compare_path(self, revised, path);
                Ok(json!({"picked": true}))
            }
            AfterPick::Export { ext } => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                let r = if ext == "png" {
                    self.run("file.exportPng", json!({"path": path}))
                } else if ext == crate::print_page::EXPORT_WORD_2003 {
                    self.run("file.saveAs", json!({"path": path, "format": ext}))
                } else {
                    self.run("file.saveAs", json!({"path": path}))
                };
                if r.is_ok() {
                    self.status(crate::i18n::fmt(tl!("Exported {path}"), &[("path", &path)]));
                }
                r
            }
            AfterPick::PrintPdf { settings } => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                let mut params = settings;
                params["path"] = json!(path);
                let r = self.run("file.printPdf", params);
                if r.is_ok() {
                    self.status(crate::i18n::fmt(tl!("Exported {path}"), &[("path", &path)]));
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
                // Save As (Backstage's Browse…, AutoSave) has no prompt to carry it to, so it was
                // otherwise dropped: a failed write (no permission, disk full, a bad path) left the
                // Save As page sitting there with nothing to say why, since its own render loop
                // skips the status bar while Backstage is open. `fail_save_as` reaches both.
                let result = picked.map(|path| self.run("file.save", json!({"path": path})));
                let saved = matches!(&result, Some(Ok(v)) if v.get("saved").and_then(Value::as_bool) == Some(true) || v.get("downloaded").is_some());
                if saved {
                    self.ui.backstage = false;
                    self.canvas.want_focus = true;
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
                    AfterSave::Continue { then, params, document } => {
                        if let Some(Err(error)) = result {
                            return Err(self.save_failed(&error, then, params, document));
                        }
                        if !self.safely_saved(saved) || document != self.session.document_id() {
                            return Ok(json!({"done": false}));
                        }
                        self.execute_held_back(&then, params).map(|r| json!({"done": true, "result": r}))
                    }
                }
            }
        }
    }

    /// Ask where to save, then save there (File › Save As).
    pub fn save_as_dialog(&mut self) {
        let _ = self.save_as(AfterSave::Nothing);
    }

    /// A plain Save As (Backstage's Browse…, AutoSave) failed to write: say so on the status bar,
    /// and, if the Save As page is what's showing, there too — its own render loop skips the
    /// status bar while Backstage is open, so that page was otherwise left with nothing to explain
    /// a failed write (no permission, disk full, a bad path).
    fn fail_save_as(&mut self, error: &str) {
        self.status(error.to_string());
        if let Some(ctx) = self.ctx.clone() {
            ctx.data_mut(|d| d.insert_temp(crate::backstage::save_as_error_id(), error.to_string()));
        }
    }

    /// Drop a Save As page error once a later attempt has actually saved.
    fn clear_save_as_error(&mut self) {
        if let Some(ctx) = self.ctx.clone() {
            crate::backstage::forget_save_as_error(&ctx);
        }
    }

    /// Ask where to save, save there, then do `after`.
    pub(crate) fn save_as(&mut self, after: AfterSave) -> Asked {
        let name = self.default_save_name();
        self.ask_file(FileDialogRequest::Save { name, dir: None }, AfterPick::SaveAs { after, document: self.session.document_id() })
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
        let dir = dir.filter(|d| d.is_dir()).map(|d| d.to_string_lossy().into_owned());
        let name = format!("{}.dotx", self.title_stem());
        let after = AfterPick::SaveAs { after: AfterSave::Nothing, document: self.session.document_id() };
        let _ = self.ask_file(FileDialogRequest::Save { name, dir: dir.clone() }, after);
        dir
    }

    /// File › Export: ask where to save a copy in this format, then write it.
    pub(crate) fn export_dialog(&mut self, ext: &str) {
        let file_ext = if ext == crate::print_page::EXPORT_WORD_2003 { "xml" } else { ext };
        let name = format!("{}.{file_ext}", self.title_stem());
        let _ = self.ask_file(FileDialogRequest::Save { name, dir: None }, AfterPick::Export { ext: ext.to_string() });
    }

    /// File › Print › Save these pages as PDF: ask where, then write the pages the print
    /// settings choose.
    pub(crate) fn print_pdf_dialog(&mut self) {
        let name = format!("{}.pdf", self.title_stem());
        let settings = self.ui.print.params();
        let _ = self.ask_file(FileDialogRequest::Save { name, dir: None }, AfterPick::PrintPdf { settings });
    }

    pub(crate) fn open_dialog(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("document");
            return;
        }
        let _ =
            self.ask_file(FileDialogRequest::Open { purpose: "document".into() }, AfterPick::OpenDocument { document: self.session.document_id() });
    }

    pub(crate) fn pick_picture(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("picture");
            return;
        }
        let _ = self.ask_file(FileDialogRequest::Open { purpose: "picture".into() }, AfterPick::Picture);
    }

    /// Pick a font file to add (Add Font…; web: it arrives through the inbox).
    pub(crate) fn pick_font(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("font");
            return;
        }
        let _ = self.ask_file(FileDialogRequest::OpenMany { purpose: "font".into() }, AfterPick::Font);
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
