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
    /// Pick a file to open; `purpose` says what for (`document`, `picture`, `recipients`, `font`), as for
    /// `pick_open`.
    Open { purpose: String },
    /// Pick a path to save to, given a suggested file name.
    Save { name: String },
}

/// The host's answer to a [`FileDialogRequest`]: the picked path, `Ok(None)` when the user
/// cancelled, or an error when no system dialog could be shown (the app then shows its own).
pub type FileDialogAnswer = Result<Option<String>, String>;

const PICTURES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "wmf", "emf"];
const DOCUMENTS: &[&str] =
    &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "hwp", "hwpx", "rtf", "txt", "md", "html", "htm", "tex", "json", "xml"];

/// The file types Open offers, as (label, extensions; `*` is any file): pictures for Insert/Change
/// Picture, recipient lists for Select Recipients › Use an Existing List…, documents otherwise.
/// The host's system dialog and the built-in browser offer the same.
pub fn open_filters(purpose: &str) -> &'static [(&'static str, &'static [&'static str])] {
    match purpose {
        "picture" => &[("Pictures", PICTURES)],
        "font" => &[("Fonts", wordcraft_fonts::FONT_FILE_EXTS)],
        "recipients" => &[("Recipient lists", wordcraft_engine::cmd::mailings::RECIPIENT_EXTENSIONS), ("All files", &["*"])],
        _ => &[("Documents", DOCUMENTS), ("Word document", &["docx"]), ("All files", &["*"])],
    }
}

/// What to do with the path picked in a dialog.
pub(crate) enum AfterPick {
    /// Open the picked document.
    OpenDocument,
    /// Insert the picked picture, or replace the pending Change Picture target.
    Picture,
    /// Save the document there, then carry on.
    SaveAs(AfterSave),
    /// Save a copy there in this format (`pdf`, `docx`, `png`…).
    Export { ext: String },
    /// Write the PDF of these print settings there (File › Print › Save these pages as PDF).
    PrintPdf { settings: Value },
    /// Load the picked CSV/TSV/text file or spreadsheet as the mail-merge recipients (#240, #334).
    Recipients,
    /// Add the picked font file (#582).
    Font,
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
            FileDialogRequest::Open { purpose } => self.services.pick_open.as_ref().and_then(|f| f(purpose)),
            FileDialogRequest::Save { name } => self.services.pick_save.as_ref().and_then(|f| f(name)),
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
            Some(Err(TryRecvError::Disconnected)) => Ok(None),
        };
        let d = self.file_dialog.take()?;
        match answer {
            Ok(picked) => Some(self.finish_pick(d.after, picked)),
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

    /// Do what the dialog was for with the picked path (`None`: the user cancelled).
    pub(crate) fn finish_pick(&mut self, after: AfterPick, picked: Option<String>) -> Result<Value, String> {
        match after {
            AfterPick::OpenDocument => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                let r = self.run("file.open", json!({"path": path}));
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
                self.run("font.add", json!({"path": path}))
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
            AfterPick::SaveAs(after) => {
                // The save's error, kept for the Save Changes prompt to explain (#1445).
                let result = picked.map(|path| self.run("file.save", json!({"path": path})));
                let saved = matches!(&result, Some(Ok(v)) if v.get("saved").and_then(Value::as_bool) == Some(true) || v.get("downloaded").is_some());
                if saved {
                    self.ui.backstage = false;
                    self.canvas.want_focus = true;
                }
                match after {
                    AfterSave::Nothing => Ok(json!({"saved": saved})),
                    AfterSave::AutoSave => {
                        if saved {
                            self.autosave = true;
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

    /// Ask where to save, save there, then do `after`.
    pub(crate) fn save_as(&mut self, after: AfterSave) -> Asked {
        let name = self.default_save_name();
        self.ask_file(FileDialogRequest::Save { name }, AfterPick::SaveAs(after))
    }

    /// File › Export: ask where to save a copy in this format, then write it.
    pub(crate) fn export_dialog(&mut self, ext: &str) {
        let file_ext = if ext == crate::print_page::EXPORT_WORD_2003 { "xml" } else { ext };
        let name = format!("{}.{file_ext}", self.title_stem());
        let _ = self.ask_file(FileDialogRequest::Save { name }, AfterPick::Export { ext: ext.to_string() });
    }

    /// File › Print › Save these pages as PDF: ask where, then write the pages the print
    /// settings choose.
    pub(crate) fn print_pdf_dialog(&mut self) {
        let name = format!("{}.pdf", self.title_stem());
        let settings = self.ui.print.params();
        let _ = self.ask_file(FileDialogRequest::Save { name }, AfterPick::PrintPdf { settings });
    }

    pub(crate) fn open_dialog(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("document");
            return;
        }
        let _ = self.ask_file(FileDialogRequest::Open { purpose: "document".into() }, AfterPick::OpenDocument);
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
        let _ = self.ask_file(FileDialogRequest::Open { purpose: "font".into() }, AfterPick::Font);
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
        let (_, tx) = shown.borrow_mut().pop().unwrap();
        tx.send(Ok(picked.map(str::to_string))).unwrap();
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

    /// Save As returns at once (the UI keeps running); the frame loop saves once the answer arrives.
    #[test]
    fn save_as_answer_arriving_later_is_saved() {
        let dir = scratch("later");
        let path = dir.join("novel.docx");
        let (mut a, shown) = async_app();
        typed(&mut a);
        a.save_as_dialog();
        assert!(a.file_dialog_open());
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "Document1.docx".into() });
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
        assert_eq!(shown.borrow()[0].0, FileDialogRequest::Save { name: "two.docx".into() });
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
    /// Save As has actually saved; a cancelled Save As cancels the held-back New.
    #[test]
    fn save_changes_waits_for_the_save_as_answer() {
        let dir = scratch("prompt");
        let path = dir.join("novel.docx");
        let (mut a, shown) = async_app();
        typed(&mut a);
        a.run("file.new", json!({})).unwrap();
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        answer(&shown, None);
        a.poll_file_dialog();
        assert!(body_text(&a).contains(UNSAVED), "cancelled Save As keeps the document");
        assert!(a.session.dirty);

        a.run("file.close", json!({})).unwrap();
        let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(r["pending"], "saveAs");
        a.poll_file_dialog();
        assert!(!a.quit_requested, "nothing closes before the save");
        answer(&shown, Some(&path.to_string_lossy()));
        a.poll_file_dialog();
        assert!(a.quit_requested, "closed once saved");
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
}
