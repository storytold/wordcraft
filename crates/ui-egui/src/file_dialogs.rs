//! File dialogs (Open, Save As, Export, Insert/Change Picture) that don't freeze the window (#94).
//!
//! A host with [`Services::file_dialog`](crate::Services::file_dialog) shows the native dialog
//! without blocking the UI thread and sends the answer back later; [`WordApp::logic`] picks it up
//! on a later frame and finishes what the dialog was for (open the file, save and carry on with a
//! pending New/Open/Close, export, insert the picture). A host with only the blocking
//! `pick_open` / `pick_save` gets the same follow-up straight away. One dialog at a time: asking
//! for another while one is open does nothing.

use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};

use crate::WordApp;

/// A file dialog the app asks the host to show ([`Services::file_dialog`](crate::Services::file_dialog)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogRequest {
    /// Pick a file to open; `purpose` says what for (`document`, `picture`, `recipients`), as for
    /// `pick_open`.
    Open { purpose: String },
    /// Pick a path to save to, given a suggested file name.
    Save { name: String },
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
    /// Load the picked CSV/TSV/text file or spreadsheet as the mail-merge recipients (#240, #334).
    Recipients,
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
    rx: Receiver<Option<String>>,
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
    /// Whether a file dialog is open (its answer hasn't arrived yet).
    pub fn file_dialog_open(&self) -> bool {
        self.file_dialog.is_some()
    }

    /// Ask the host for a file and do `after` with the answer: now for a blocking dialog, on a
    /// later frame for an asynchronous one. Ignored while another dialog is open.
    pub(crate) fn ask_file(&mut self, req: FileDialogRequest, after: AfterPick) -> Asked {
        if self.file_dialog.is_some() {
            log::debug!("file dialog already open; ignoring {req:?}");
            self.status(tl!("A file dialog is already open."));
            return Asked::Busy;
        }
        if let Some(show) = &self.services.file_dialog {
            let rx = show(req);
            self.file_dialog = Some(PendingDialog { rx, after });
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
        let picked = match self.file_dialog.as_ref().map(|d| d.rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => return None,
            Some(Ok(picked)) => picked,
            // The host dropped the dialog without an answer: as good as cancelled.
            Some(Err(TryRecvError::Disconnected)) => None,
        };
        let d = self.file_dialog.take()?;
        Some(self.finish_pick(d.after, picked))
    }

    /// Do what the dialog was for with the picked path (`None`: the user cancelled).
    fn finish_pick(&mut self, after: AfterPick, picked: Option<String>) -> Result<Value, String> {
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
            AfterPick::Recipients => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                self.load_recipients(json!({"path": path}))
            }
            AfterPick::Export { ext } => {
                let Some(path) = picked else { return Ok(json!({"cancelled": true})) };
                let r = if ext == "png" { self.run("file.exportPng", json!({"path": path})) } else { self.run("file.saveAs", json!({"path": path})) };
                if r.is_ok() {
                    self.status(crate::i18n::fmt(tl!("Exported {path}"), &[("path", &path)]));
                }
                r
            }
            AfterPick::SaveAs(after) => {
                let saved = picked.is_some_and(|path| {
                    self.run("file.save", json!({"path": path})).is_ok_and(|v| v.get("saved").and_then(Value::as_bool) == Some(true))
                });
                match after {
                    AfterSave::Nothing => Ok(json!({"saved": saved})),
                    AfterSave::AutoSave => {
                        if saved {
                            self.autosave = true;
                        }
                        Ok(json!({"saved": saved}))
                    }
                    AfterSave::Continue { then, params, document } => {
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
        let name = format!("{}.{ext}", self.title_stem());
        let _ = self.ask_file(FileDialogRequest::Save { name }, AfterPick::Export { ext: ext.to_string() });
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
    type Shown = Rc<RefCell<Vec<(FileDialogRequest, Sender<Option<String>>)>>>;

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
        tx.send(picked.map(str::to_string)).unwrap();
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
}
