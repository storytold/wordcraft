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
                        self.execute_user(&then, params).map(|r| json!({"done": true, "result": r}))
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
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc::{Sender, channel};

    use super::*;
    use crate::Services;
    use wordcraft_engine::Session;

    const UNSAVED: &str = "My unsaved novel chapter";

    /// Original Word 97 compound file: one compressed text piece, built from MS-DOC's FIB/Clx.
    fn legacy_word_bytes() -> Vec<u8> {
        use std::io::{Cursor, Write};
        let text = b"Legacy contract\r";
        let mut word = vec![0u8; 0x600];
        for (at, value) in [(0, 0xA5ECu16), (2, 0x00C1), (0x0A, 0x1000), (0x0C, 0x00BF), (0x20, 14), (0x3E, 22), (0x98, 0x005D)] {
            word[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        word[0x40..0x44].copy_from_slice(&0x1000u32.to_le_bytes());
        word[0x4C..0x50].copy_from_slice(&(text.len() as u32).to_le_bytes());
        let mut clx = vec![2];
        clx.extend_from_slice(&16u32.to_le_bytes());
        clx.extend_from_slice(&0u32.to_le_bytes());
        clx.extend_from_slice(&(text.len() as u32).to_le_bytes());
        clx.extend_from_slice(&[0, 0]);
        clx.extend_from_slice(&(0x4000_0000u32 | (2 * 0x600)).to_le_bytes());
        clx.extend_from_slice(&[0, 0]);
        let pair = 0x9A + 33 * 8;
        word[pair..pair + 4].copy_from_slice(&0x40u32.to_le_bytes());
        word[pair + 4..pair + 8].copy_from_slice(&(clx.len() as u32).to_le_bytes());
        word.extend_from_slice(text);
        let mut table = vec![0; 0x40];
        table.extend_from_slice(&clx);
        let mut file = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        for (name, data) in [("/WordDocument", word), ("/0Table", table)] {
            file.create_stream(name).unwrap().write_all(&data).unwrap();
        }
        file.into_inner().into_inner()
    }

    /// #2445: Save on an imported binary Word file asks for DOCX, preserving the import on Cancel.
    #[test]
    fn imported_word_save_asks_for_docx_and_preserves_the_original() {
        for ext in ["doc", "DOT", "DOC", "dot"] {
            let dir = scratch(&format!("imported-{ext}"));
            let original = dir.join(format!("contract.{ext}"));
            let bytes = legacy_word_bytes();
            std::fs::write(&original, &bytes).unwrap();
            let (mut a, shown) = async_app();
            a.run("file.open", json!({"path": original.to_string_lossy()})).unwrap();
            assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Legacy contract"));
            a.run("text.insert", json!({"text": "Revised "})).unwrap();
            let draft = a.session.doc.clone();
            let revision = a.session.rev();
            let result = a.run("file.save", json!({})).unwrap();
            assert_eq!(result["pending"], "saveAs");
            assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "contract.docx".into(), dir: None });
            answer(&shown, None);
            assert_eq!(a.poll_file_dialog().unwrap().unwrap()["saved"], false);
            assert_eq!(a.session.doc, draft);
            assert_eq!(a.session.rev(), revision);
            assert!(a.session.dirty);
            assert_eq!(a.session.path.as_deref(), Some(original.as_path()));
            assert_eq!(std::fs::read(&original).unwrap(), bytes);

            a.run("file.save", json!({})).unwrap();
            let saved = dir.join("revised.docx");
            answer(&shown, Some(&saved.to_string_lossy()));
            assert_eq!(a.poll_file_dialog().unwrap().unwrap()["saved"], true);
            assert!(!a.session.dirty);
            assert_eq!(a.session.path.as_deref(), Some(saved.as_path()));
            assert!(saved_text(&saved).contains("Revised Legacy contract"));
            assert_eq!(std::fs::read(&original).unwrap(), bytes);
            // The new supported path saves directly, without another chooser.
            a.run("text.insert", json!({"text": "Again "})).unwrap();
            assert_eq!(a.run("file.save", json!({})).unwrap()["saved"], true);
            assert!(shown.borrow().is_empty());
            assert!(saved_text(&saved).contains("Again "));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn imported_word_close_waits_for_save_as_and_keeps_failed_or_cancelled_drafts() {
        let dir = scratch("imported-close");
        let original = dir.join("contract.doc");
        let bytes = legacy_word_bytes();
        std::fs::write(&original, &bytes).unwrap();
        let (mut a, shown) = async_app();
        a.run("file.open", json!({"path": original.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        let draft = a.session.doc.clone();

        a.run("file.close", json!({})).unwrap();
        assert_eq!(a.run("ui.saveChanges", json!({"answer": "save"})).unwrap()["pending"], "saveAs");
        assert!(a.has_document() && !a.quit_requested);
        answer(&shown, Some(&dir.join("missing").join("contract.docx").to_string_lossy()));
        let error = a.poll_file_dialog().unwrap().unwrap_err();
        assert!(error.contains("still open"), "{error}");
        assert!(matches!(a.dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })));
        assert!(a.has_document() && a.session.dirty && !a.quit_requested);
        assert_eq!(a.session.path.as_deref(), Some(original.as_path()));
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), draft.plain_text(wordcraft_doc::StoryRef::Body));

        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        answer(&shown, None);
        assert_eq!(a.poll_file_dialog().unwrap().unwrap()["done"], false);
        assert!(a.has_document() && a.session.dirty && !a.quit_requested);
        assert_eq!(a.session.path.as_deref(), Some(original.as_path()));

        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        let saved = dir.join("contract.docx");
        answer(&shown, Some(&saved.to_string_lossy()));
        assert_eq!(a.poll_file_dialog().unwrap().unwrap()["done"], true);
        // File › Close keeps the window (#1712): the document closes once the DOCX save succeeds.
        assert!(!a.has_document() && !a.quit_requested, "Close runs only after the DOCX save succeeds");
        assert!(saved_text(&saved).contains("Revised Legacy contract"));
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A late Save As answer belongs to the imported document that opened the picker: once that
    /// document is closed, it writes nothing.
    #[test]
    fn imported_word_stale_save_as_never_writes_the_replacement_document() {
        for close in [false, true] {
            let dir = scratch(if close { "imported-stale-close" } else { "imported-stale-save" });
            let original = dir.join("contract.doc");
            let bytes = legacy_word_bytes();
            std::fs::write(&original, &bytes).unwrap();
            let destination = dir.join("existing.docx");
            let sentinel = b"Existing destination must survive";
            std::fs::write(&destination, sentinel).unwrap();
            let (mut a, shown) = async_app();
            a.run("file.open", json!({"path": original})).unwrap();
            typed(&mut a);
            if close {
                a.run("file.close", json!({})).unwrap();
                a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
            } else {
                a.run("file.save", json!({})).unwrap();
            }
            assert!(a.file_dialog_open());
            let previous = a.session.document_id();
            // Automation bypasses the user prompt and can close a document during a picker.
            a.execute("file.closeDocument", json!({"force": true})).unwrap();
            typed(&mut a);
            assert_ne!(previous, a.session.document_id());
            let draft = a.session.doc.clone();
            let revision = a.session.rev();
            answer(&shown, Some(&destination.to_string_lossy()));
            let _ = a.poll_file_dialog().unwrap();
            assert!(std::fs::read(&destination).unwrap() == sentinel, "stale picker overwrote the destination");
            assert_eq!(std::fs::read(&original).unwrap(), bytes);
            assert_eq!(a.session.doc, draft);
            assert_eq!(a.session.rev(), revision);
            assert!(a.session.dirty && a.session.path.is_none() && !a.quit_requested);
        }
    }

    #[test]
    fn imported_word_programmatic_and_explicit_saves_never_open_dialogs() {
        let dir = scratch("imported-explicit");
        let original = dir.join("contract.doc");
        let bytes = legacy_word_bytes();
        std::fs::write(&original, &bytes).unwrap();
        let (mut a, shown) = async_app();
        a.run("file.open", json!({"path": original.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        let error = a.execute("file.save", json!({})).unwrap_err();
        assert!(error.contains("can't save as `.doc`"), "{error}");
        let error = a.run("file.save", json!({"path": original.to_string_lossy()})).unwrap_err();
        assert!(error.contains("can't save as `.doc`"), "{error}");
        assert!(shown.borrow().is_empty());
        assert!(a.session.dirty);
        assert_eq!(a.session.path.as_deref(), Some(original.as_path()));
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imported_word_save_controls_ask_for_docx() {
        use egui_kittest::kittest::Queryable;
        let dir = scratch("imported-entry-points");
        let original = dir.join("contract.doc");
        let bytes = legacy_word_bytes();
        std::fs::write(&original, &bytes).unwrap();
        let (mut a, shown) = async_app();
        a.run("ui.language", json!({"value": "en"})).unwrap();
        a.run("file.open", json!({"path": original})).unwrap();
        typed(&mut a);
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(3);
        assert!(h.state().file_dialog_open(), "the Save button asks for a supported target");
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "contract.docx".into(), dir: None });
        answer(&shown, None);
        h.run_steps(3);

        h.state_mut().run("ui.backstage", json!({"value": false})).unwrap();
        h.state_mut().canvas.want_focus = true;
        h.run_steps(6);
        assert!(h.state().canvas.focused, "Mod+S starts in the editor");
        h.event(egui::Event::Key { key: egui::Key::S, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
        h.run_steps(3);
        assert!(h.state().file_dialog_open(), "the focused editor's Mod+S asks too");
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "contract.docx".into(), dir: None });
        answer(&shown, None);
        h.run_steps(3);
        assert!(h.state().session.dirty);
        assert_eq!(h.state().session.path.as_deref(), Some(original.as_path()));
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
    }

    /// Dialogs the fake host was asked to show, each with the channel its answer goes through.
    type Shown = Rc<RefCell<Vec<(FileDialogRequest, Sender<FileDialogAnswer>)>>>;

    /// An app whose host shows dialogs asynchronously: nothing is answered until the test does.
    fn async_app() -> (WordApp, Shown) {
        let shown: Shown = Rc::default();
        let log = shown.clone();
        let services = Services {
            file_dialog: Some(Box::new(move |req| {
                let (tx, rx) = channel();
                log.borrow_mut().push((req, tx));
                rx
            })),
            ..Default::default()
        };
        (WordApp::new(Session::new(wordcraft_doc::Document::new()), services), shown)
    }

    /// Answer the dialog the host is showing (the last one asked for).
    fn answer(shown: &Shown, picked: Option<&str>) {
        answer_all(shown, picked.as_slice());
    }

    /// Answer the dialog with several picked paths (none: cancelled).
    fn answer_all(shown: &Shown, picked: &[&str]) {
        let (_, tx) = shown.borrow_mut().pop().unwrap();
        tx.send(Ok(picked.iter().map(|p| p.to_string()).collect())).unwrap();
    }

    fn typed(a: &mut WordApp) {
        a.run("text.insert", json!({"text": UNSAVED})).unwrap();
        assert!(a.session.dirty);
    }

    fn body_text(a: &WordApp) -> String {
        a.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-dialogs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn saved_text(path: &std::path::Path) -> String {
        wordcraft_engine::io::open_path(path).unwrap().plain_text(wordcraft_doc::StoryRef::Body)
    }

    /// Save as Word Template (#2852) suggests a .dotx in the user templates folder, which is made
    /// on first use; the picked path is saved as a template. A folder named in File › Options
    /// replaces the app's own.
    #[test]
    fn save_as_template_starts_in_the_templates_folder() {
        let dir = scratch("save-template");
        let own = dir.join("Templates");
        let (mut a, shown) = async_app();
        a.services.templates_dir = Some(own.clone());
        typed(&mut a);
        let r = a.run("ui.saveAsTemplate", json!({})).unwrap();
        assert!(own.is_dir(), "the app's templates folder is made on first use");
        assert_eq!(r["dir"], json!(own.to_string_lossy()));
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "Document1.dotx".into(), dir: Some(own.to_string_lossy().into_owned()) });
        let path = own.join("Memo.dotx");
        answer(&shown, Some(&path.to_string_lossy()));
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
        assert!(path.is_file(), "saved as a template");
        assert_eq!(saved_text(&path), UNSAVED);
        let listed = wordcraft_engine::cmd::user_templates::list_templates(&own.to_string_lossy()).unwrap();
        assert_eq!(listed.templates.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["Memo"]);

        // A folder chosen in Options wins.
        let chosen = dir.join("Chosen");
        std::fs::create_dir_all(&chosen).unwrap();
        a.run("file.options", json!({"userTemplatesFolder": chosen.to_string_lossy()})).unwrap();
        assert_eq!(a.user_templates_dir(), Some(chosen.clone()));
        a.run("ui.saveAsTemplate", json!({})).unwrap();
        assert!(
            matches!(&shown.borrow()[0].0, FileDialogRequest::Save { name, dir: Some(d) } if name == "Memo.dotx" && *d == chosen.to_string_lossy())
        );
        answer(&shown, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save As returns at once (the UI keeps running); the frame loop saves once the answer arrives.
    #[test]
    fn save_as_answer_arriving_later_is_saved() {
        let dir = scratch("later");
        let path = dir.join("novel.docx");
        let (mut a, shown) = async_app();
        typed(&mut a);
        a.save_as_dialog();
        assert!(a.file_dialog_open());
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "Document1.docx".into(), dir: None });
        let ctx = egui::Context::default();
        let frame = |a: &mut WordApp| ctx.run_ui(egui::RawInput::default(), |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
        frame(&mut a);
        assert!(a.session.dirty, "nothing saved before the user answers");
        answer(&shown, Some(&path.to_string_lossy()));
        frame(&mut a);
        assert!(!a.file_dialog_open());
        assert!(!a.session.dirty);
        assert!(saved_text(&path).contains(UNSAVED));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #689: Save As honours the filename typed in the dialog, not just the folder: a document
    /// opened as `a/one.docx` and saved as `b/two.docx` is written there and takes that path.
    #[test]
    fn save_as_with_a_new_filename_saves_under_it() {
        let dir = scratch("rename");
        let opened = dir.join("one.docx");
        let picked = dir.join("sub").join("two.docx");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("original"))
            .run("file.save", &json!({"path": opened.to_string_lossy()}))
            .unwrap();
        let (mut a, shown) = async_app();
        a.run("file.open", json!({"path": opened.to_string_lossy()})).unwrap();
        assert_eq!(a.session.path.as_deref(), Some(opened.as_path()));
        a.save_as_dialog();
        answer(&shown, Some(&picked.to_string_lossy()));
        let r = a.poll_file_dialog().unwrap().unwrap();
        assert_eq!(r, json!({"saved": true}), "the save-as succeeded");
        assert_eq!(a.session.path.as_deref(), Some(picked.as_path()), "the picked filename is the document's path");
        assert!(picked.is_file(), "the file is written under the picked name");
        assert!(saved_text(&picked).contains("original"));
        // F12 (file.saveAs without a path) lands in the same dialog, suggesting the new name.
        let r = a.run("file.saveAs", json!({})).unwrap();
        assert_eq!(r, json!({"saved": false}));
        assert!(a.file_dialog_open(), "F12's saveAs request opens the dialog");
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "two.docx".into(), dir: None });
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Backstage File > Save > Save As page > Browse… picks a folder that can't be written to
    /// (missing, no permission...): the write fails, but the page previously said nothing about
    /// it, leaving the user with a Browse… button and no document, no clue why. The failure now
    /// reaches the status bar, and the document (and its unsaved text) stays open regardless.
    #[test]
    fn a_failed_save_as_says_so_and_keeps_the_document() {
        let dir = scratch("write-fails");
        let bogus = dir.join("does-not-exist").join("out.docx");
        let (mut a, shown) = async_app();
        typed(&mut a);
        a.run("ui.backstage", json!({"value": true, "page": "saveAs"})).unwrap();
        a.save_as_dialog();
        answer(&shown, Some(&bogus.to_string_lossy()));
        let r = a.poll_file_dialog().unwrap().unwrap();
        assert_eq!(r["saved"], false, "the write failed");
        assert!(!bogus.exists());
        assert!(a.ui.backstage, "backstage stays open: nothing was actually saved");
        assert!(a.has_document() && body_text(&a).contains(UNSAVED), "the document and its text are kept");
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(!msg.is_empty(), "the failure is no longer silent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The AutoSave switch's own Save As (clicked while blocked on `Unsaved`, chrome.rs) goes
    /// through the same `AfterPick::SaveAs` arm as Backstage's, under `AfterSave::AutoSave`
    /// instead of `AfterSave::Nothing`: a write failure there must reach the status bar too
    /// (visible here, since the switch only shows while Backstage is closed), and AutoSave must
    /// stay off rather than claim coverage it doesn't have (the raw `autosave` flag defaults to
    /// `true`; what actually governs coverage is `autosaves()`, the flag plus the block).
    #[test]
    fn a_failed_autosave_switch_save_as_says_so_and_leaves_autosave_off() {
        let dir = scratch("autosave-switch-write-fails");
        let bogus = dir.join("does-not-exist").join("out.docx");
        let (mut a, shown) = async_app();
        typed(&mut a);
        assert!(matches!(a.autosave_block(), Some(crate::AutoSaveBlock::Unsaved)), "a new document blocks AutoSave until saved");
        assert!(!a.autosaves(), "blocked: AutoSave isn't covering the document yet");
        a.save_as(AfterSave::AutoSave);
        answer(&shown, Some(&bogus.to_string_lossy()));
        let r = a.poll_file_dialog().unwrap().unwrap();
        assert_eq!(r["saved"], false, "the write failed");
        assert!(!a.autosaves(), "still blocked: the save that would have lifted it never happened");
        assert!(a.session.path.is_none(), "unsaved: the write never actually landed");
        assert!(a.has_document() && body_text(&a).contains(UNSAVED), "the document and its text are kept");
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(!msg.is_empty(), "the failure is no longer silent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// While a dialog is open, more Save As / Open / Export / Picture clicks don't open a second one.
    #[test]
    fn a_second_dialog_while_one_is_open_is_ignored() {
        let (mut a, shown) = async_app();
        a.save_as_dialog();
        a.save_as_dialog();
        a.open_dialog();
        a.export_dialog("pdf");
        a.pick_picture();
        assert_eq!(shown.borrow().len(), 1, "only the first dialog was shown");
        answer(&shown, None);
        a.poll_file_dialog();
        assert!(!a.file_dialog_open());
        a.open_dialog();
        assert_eq!(shown.borrow().len(), 1, "a new dialog once the first was answered");
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Open { purpose: "document".into() });
    }

    /// #2313: Export PDF from the command search (no path given) asks where to save it, as the
    /// Backstage's Export button does, instead of failing with "bad parameters: `path` (string)
    /// is required" on the status bar.
    #[test]
    fn export_pdf_without_a_path_asks_where_to_save() {
        let (mut a, shown) = async_app();
        let r = a.run("file.exportPdf", json!({})).unwrap();
        assert_eq!(r, json!({"exported": false}));
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: format!("{}.pdf", a.title_stem()), dir: None });
        let dir = scratch("export-pdf");
        let picked = dir.join("out.pdf");
        answer(&shown, Some(&picked.to_string_lossy()));
        let r = a.poll_file_dialog().unwrap().unwrap();
        assert_eq!(r["path"], json!(picked.to_string_lossy().to_string()));
        let bytes = std::fs::read(&picked).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "a PDF is written to the picked path");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Open, Save As and Export PNG from the command search (no path) ask for a file the same way.
    #[test]
    fn file_commands_without_a_path_ask_for_one() {
        let (mut a, shown) = async_app();
        a.run("file.open", json!({})).unwrap();
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Open { purpose: "document".into() });
        // A second dialog can't open while one is up, so cancel before the next command.
        answer(&shown, None);
        a.poll_file_dialog();
        a.run("file.saveAs", json!({})).unwrap();
        assert!(matches!(shown.borrow()[0].0, FileDialogRequest::Save { .. }));
        answer(&shown, None);
        a.poll_file_dialog();
        a.run("file.exportPng", json!({})).unwrap();
        assert!(matches!(shown.borrow()[0].0, FileDialogRequest::Save { ref name, .. } if name.ends_with(".png")));
    }

    /// #1445: "Save changes?" → Save on a new document while another file dialog is still open:
    /// nothing can be asked, so nothing happens, and the status bar says why.
    #[test]
    fn save_changes_while_a_file_dialog_is_open_says_so() {
        let (mut a, shown) = async_app();
        typed(&mut a);
        a.open_dialog();
        assert_eq!(shown.borrow().len(), 1);
        a.run("file.close", json!({})).unwrap();
        a.status_msg = None;
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["done"], false);
        assert!(!a.quit_requested);
        assert!(a.session.dirty && body_text(&a).contains(UNSAVED));
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(msg.contains("file dialog is already open"), "the status bar says why: {msg:?}");
    }

    /// "Save changes?" → Save on a new document: the held-back Close runs only once the later
    /// Save As has actually saved; a cancelled Save As cancels it.
    #[test]
    fn save_changes_waits_for_the_save_as_answer() {
        let dir = scratch("prompt");
        let path = dir.join("novel.docx");
        let (mut a, shown) = async_app();
        typed(&mut a);
        a.run("file.close", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        answer(&shown, None);
        a.poll_file_dialog();
        assert!(body_text(&a).contains(UNSAVED), "cancelled Save As keeps the document");
        assert!(a.session.dirty);

        a.run("file.close", json!({})).unwrap();
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["pending"], "saveAs");
        a.poll_file_dialog();
        assert!(a.has_document(), "nothing closes before the save");
        answer(&shown, Some(&path.to_string_lossy()));
        a.poll_file_dialog();
        assert!(!a.has_document(), "closed once saved");
        assert!(saved_text(&path).contains(UNSAVED));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Desktop (#240 on #94): Use an Existing List… asks the host's file dialog for a recipient
    /// list and, once the answer arrives, loads it as the mail-merge recipients, not as a document.
    #[test]
    fn a_recipient_list_picked_later_loads_recipients() {
        let dir = scratch("recipients");
        let csv = dir.join("people.csv");
        std::fs::write(&csv, "First Name,City\nAda,London\nAlan,Wilmslow\n").unwrap();
        let (mut a, shown) = async_app();
        typed(&mut a);
        a.run("ui.openRecipientList", json!({})).unwrap();
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Open { purpose: "recipients".into() });
        assert!(a.file_dialog_open());
        answer(&shown, Some(&csv.to_string_lossy()));
        a.poll_file_dialog().unwrap().unwrap();
        assert_eq!(a.session.merge.headers, vec!["First Name", "City"]);
        assert_eq!(a.session.merge.rows.len(), 2);
        assert!(body_text(&a).contains(UNSAVED), "the document stays open");
        // Cancelling loads nothing.
        a.run("ui.openRecipientList", json!({})).unwrap();
        answer(&shown, None);
        assert_eq!(a.poll_file_dialog().unwrap().unwrap(), json!({"cancelled": true}));
        assert_eq!(a.session.merge.rows.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #2640 (desktop): Manage Sources › Import… hands the picked Word bibliography file
    /// (`Sources.xml`, `b:Sources`) to `references.sources`, and the status says how many
    /// sources joined the document. Cancelling imports nothing, and importing the same file
    /// twice adds nothing the second time.
    #[test]
    fn an_imported_sources_file_adds_its_sources() {
        let dir = scratch("sources");
        let xml = dir.join("Sources.xml");
        std::fs::write(
            &xml,
            r#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
  <b:Source><b:Tag>Riv21</b:Tag><b:SourceType>Book</b:SourceType><b:Title>Shared Spaces</b:Title><b:Year>2021</b:Year></b:Source>
</b:Sources>"#,
        )
        .unwrap();
        let (mut a, _) = async_app();
        a.finish_pick(AfterPick::Sources, Some(xml.to_string_lossy().to_string())).unwrap();
        assert_eq!(a.session.doc.sources.len(), 1);
        assert_eq!(a.session.doc.sources[0].tag, "Riv21");
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| m.contains("Imported 1 source")), "{:?}", a.status_msg);
        assert_eq!(a.finish_pick(AfterPick::Sources, None).unwrap(), json!({"cancelled": true}));
        a.finish_pick(AfterPick::Sources, Some(xml.to_string_lossy().to_string())).unwrap();
        assert_eq!(a.session.doc.sources.len(), 1, "the tag is already here");
        assert!(a.status_msg.as_ref().is_some_and(|(m, _)| m == "No new sources in that file"), "{:?}", a.status_msg);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A workbook with several sheets (#334) shows Select Table; its answer loads that sheet.
    #[test]
    fn a_workbook_with_several_sheets_asks_which_one() {
        use std::io::Write;
        let dir = scratch("workbook");
        let book = dir.join("people.ods");
        let table = |name: &str, cells: &[&str]| {
            let cells: String = cells
                .iter()
                .map(|c| {
                    format!(
                        r#"<table:table-row><table:table-cell office:value-type="string"><text:p>{c}</text:p></table:table-cell></table:table-row>"#
                    )
                })
                .collect();
            format!(r#"<table:table table:name="{name}">{cells}</table:table>"#)
        };
        let content = format!(
            r#"<office:document-content xmlns:office="o" xmlns:table="t" xmlns:text="x"><office:body><office:spreadsheet>{}{}</office:spreadsheet></office:body></office:document-content>"#,
            table("Notes", &["n"]),
            table("Guests", &["Name", "Ada", "Alan"])
        );
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, body) in [("mimetype", "application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", content.as_str())] {
            z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        std::fs::write(&book, z.finish().unwrap().into_inner()).unwrap();
        let (mut a, shown) = async_app();
        a.run("ui.openRecipientList", json!({})).unwrap();
        answer(&shown, Some(&book.to_string_lossy()));
        a.poll_file_dialog().unwrap().unwrap();
        let Some(crate::dialogs::Dialog::SelectTable { sheets, params, headers: true, .. }) = &a.dialog else { panic!("Select Table expected") };
        assert_eq!(sheets, &["Notes", "Guests"]);
        assert!(a.session.merge.headers.is_empty(), "nothing loads before a sheet is chosen");
        let mut p = params.clone();
        p["sheet"] = json!(2);
        a.load_recipients(p).unwrap();
        assert_eq!(a.session.merge.headers, vec!["Name"]);
        assert_eq!(a.session.merge.rows.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_system_fonts_runs_hook_when_present() {
        use std::cell::Cell;
        let called = Rc::new(Cell::new(false));
        let called2 = called.clone();
        let services = Services { load_system_fonts: Some(Box::new(move || called2.set(true))), ..Default::default() };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("font.loadSystem", json!({})).unwrap();
        assert!(called.get());
    }

    #[test]
    fn load_system_fonts_without_hook_reports_status() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("font.loadSystem", json!({})).unwrap();
        assert_eq!(a.status_msg.as_ref().map(|(s, _)| s.as_str()), Some("All system fonts are already available."));
    }

    /// The bundled face `file` of family `original` renamed `family` (as long as `original`), so it
    /// is a font the database doesn't have.
    fn renamed_font(file: &str, original: &str, family: &str) -> Vec<u8> {
        assert_eq!(family.len(), original.len());
        let mut data = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        for (from, to) in [(utf16(original), utf16(family)), (original.as_bytes().to_vec(), family.as_bytes().to_vec())] {
            let mut i = 0;
            while let Some(at) = data.get(i..).and_then(|d| d.windows(from.len()).position(|w| w == from)) {
                data[i + at..i + at + to.len()].copy_from_slice(&to);
                i += at + to.len();
            }
        }
        data
    }

    fn font_families(a: &mut WordApp) -> Vec<String> {
        let list = a.run("font.list", json!({"all": true})).unwrap();
        list["families"].as_array().unwrap().iter().filter_map(|f| f.as_str().map(str::to_string)).collect()
    }

    /// #1789: Add Font asks for several files, and every one picked is added; an unreadable file
    /// is reported and the rest still go in.
    #[test]
    fn add_font_imports_every_selected_file() {
        const ALPHA: &str = "Mfpick Alpha 1";
        const BRAVO: &str = "Mfpick Bravo 2";
        let dir = scratch("fonts");
        let alpha = dir.join("MfpickAlpha1-Regular.ttf");
        let junk = dir.join("Junk.ttf");
        let bravo = dir.join("MfpickBravo2-Regular.ttf");
        std::fs::write(&alpha, renamed_font("JetBrainsMono-Regular.ttf", "JetBrains Mono", ALPHA)).unwrap();
        std::fs::write(&junk, b"this is not a font").unwrap();
        std::fs::write(&bravo, renamed_font("JetBrainsMono-Regular.ttf", "JetBrains Mono", BRAVO)).unwrap();

        let (mut a, shown) = async_app();
        a.pick_font();
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::OpenMany { purpose: "font".into() }, "Add Font is multi-select");
        let paths: Vec<String> = [&alpha, &junk, &bravo].iter().map(|p| p.to_string_lossy().into_owned()).collect();
        answer_all(&shown, &paths.iter().map(String::as_str).collect::<Vec<_>>());
        let r = a.poll_file_dialog().unwrap().unwrap();
        assert_eq!(r["added"], 2);
        assert_eq!(r["failed"].as_array().map(Vec::len), Some(1));
        let families = font_families(&mut a);
        assert!(families.iter().any(|f| f == ALPHA) && families.iter().any(|f| f == BRAVO), "both valid fonts were added");
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert_eq!(msg, "Added 2 fonts; 1 couldn't be read: Junk.ttf");
        for f in [ALPHA, BRAVO] {
            a.run("font.remove", json!({"family": f})).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only Add Font is multi-select; a cancelled pick adds nothing.
    #[test]
    fn only_add_font_asks_for_several_files() {
        let (mut a, shown) = async_app();
        a.open_dialog();
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Open { purpose: "document".into() });
        answer(&shown, None);
        a.poll_file_dialog();
        a.pick_picture();
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Open { purpose: "picture".into() });
        answer(&shown, None);
        a.poll_file_dialog();
        a.pick_font();
        assert!(matches!(shown.borrow()[0].0, FileDialogRequest::OpenMany { .. }));
        answer_all(&shown, &[]);
        assert_eq!(a.poll_file_dialog().unwrap().unwrap(), json!({"cancelled": true}));
        assert!(!a.file_dialog_open());
    }

    /// A hostile selection is bounded: only the first MAX_FONTS_PER_PICK are tried.
    #[test]
    fn a_huge_font_selection_is_capped() {
        let (mut a, _) = async_app();
        let paths: Vec<String> = (0..MAX_FONTS_PER_PICK + 50).map(|i| format!("/nonexistent/f{i}.ttf")).collect();
        let r = a.add_fonts(paths).unwrap();
        assert_eq!(r["added"], 0);
        assert_eq!(r["failed"].as_array().map(Vec::len), Some(MAX_FONTS_PER_PICK));
        assert_eq!(r["skipped"], 50);
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert_eq!(msg, "Added 0 fonts; 2000 couldn't be read: f0.ttf, f1.ttf, f2.ttf and 1997 more; 50 skipped (limit 2000)");
    }

    /// Past three failures the status line names three and counts the rest.
    #[test]
    fn many_failed_fonts_end_with_a_count() {
        let (mut a, _) = async_app();
        let paths: Vec<String> = (0..5).map(|i| format!("/nonexistent/f{i}.ttf")).collect();
        a.add_fonts(paths).unwrap();
        let msg = a.status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert_eq!(msg, "Added 0 fonts; 5 couldn't be read: f0.ttf, f1.ttf, f2.ttf and 2 more");
    }
}
