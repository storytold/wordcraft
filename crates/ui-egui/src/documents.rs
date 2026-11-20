//! Several open documents. On the desktop each has a window of its own ([`crate::windows`]); the
//! web app shows them in one window, which shows the session's active document. What the app
//! keeps about a document (AutoSave's file, comment drafts) follows the active one. In one
//! window, the window's own state about the page (drags, menus, the composition…) starts afresh
//! when the document changes ([`WordApp::sync_document`]; per-window state is listed in
//! [`crate::window_ui`]). View › Window › Switch Windows lists the open documents, File › Close
//! closes the shown one, and quitting asks about every document with unsaved changes, one after
//! another, each in its window.

use serde_json::{Value, json};
use wordcraft_engine::DocInfo;

use craft::ui::contract::Show as _;
use craft::ui::menu::MenuItem;

use crate::{WordApp, panes};

/// What the window keeps for an open document while another one is shown.
#[derive(Default)]
pub(crate) struct Parked {
    /// [`WordApp`]'s `autosave_path`: the file AutoSave writes this document to.
    autosave_path: Option<std::path::PathBuf>,
    /// The comment panel's selection and the comment and reply text typed in this document and not
    /// yet written to it.
    comments: craft::review::ui::CommentsPanelState,
}

impl WordApp {
    /// Replace the document the window opened with before anything else happens (the first blank
    /// document, made again on the region File › Options chose): the session's document is
    /// swapped and the window follows it at once. Replacing it with `session.set_document`
    /// alone leaves the window showing the old document, so the first frame took the change for
    /// a switch and dropped the dialog a command line file had just asked for, a password box
    /// among them (#5299).
    pub fn replace_startup_document(&mut self, doc: wordcraft_doc::Document) {
        self.session.set_document(doc);
        self.sync_document();
    }

    /// Follow the session's active document. When it isn't the one the window last showed, the
    /// shown document's own state is put aside (or dropped, if it was closed or replaced), the
    /// new one's comes back, and the window's state about the page that was shown starts afresh
    /// (all of it per-window state, [`crate::window_ui::WindowUi`]): open
    /// dialogs and menus, drags, the input method's composition, the status message, caches
    /// keyed by revision. Its scroll position and zoom come back with it (the scroll area is per
    /// document; the zoom is in the session's view). Runs after every command and every frame.
    /// With native windows ([`crate::windows`]) the window that shows the document comes to the
    /// front instead, or a new one opens for it.
    pub(crate) fn sync_document(&mut self) {
        let id = self.session.document_id();
        if self.windows.native() {
            self.prune_windows();
        }
        if id == self.shown_document {
            return;
        }
        if self.windows.native() && self.follow_in_windows(id) {
            return;
        }
        let left = std::mem::replace(&mut self.shown_document, id);
        let seen = self.carry_document_state(left, id);
        self.window_shows(id);

        self.canvas.show_other_document(seen);
        // A Save Changes prompt asked about the document that left: it goes, and its answer with it. The
        // operation stays (a quit closing documents one by one moves on), and an answer that arrives anyway
        // is refused as being about a replaced document (`apply_close_answer`).
        if let Some(id) = self.closing.as_mut().and_then(|c| c.prompt.take()) {
            self.close_prompt(id);
        }
        if let Some(ctx) = self.ctx.clone() {
            crate::dialogs::host::close_all(self, &ctx);
        }
        self.spelling_review = None;
        self.restrict = crate::restrict_pane::RestrictPane { open: self.restrict.open, ..Default::default() };
        self.status = None;
        self.change_picture_target = None;
        self.recipient_list_pending = false;
        self.nav_search_focus = false;
        self.smart_art_item = None;
        self.read_aloud_at = None;
        self.info_editing = None;
        // Caches keyed by the revision alone: another document's revision can be the same number.
        self.status_counts = crate::window_ui::NOT_COUNTED;
        self.previews.document_changed();
        // The Start screen's placeholder (#1129) goes once a document is open beside it.
        if let Some(placeholder) = self.no_document
            && placeholder != id
            && self.session.is_open(placeholder)
        {
            let _ = self.session.close_document(placeholder);
            self.parked.remove(&placeholder);
        }
    }

    /// The active document is `to` and no longer `from`: `from`'s own state is put aside (or
    /// dropped, if it was closed or replaced) and `to`'s comes back. Makes `to` the session's
    /// active document. Returns whether `to` had state put aside.
    pub(crate) fn carry_document_state(&mut self, from: u64, to: u64) -> bool {
        if self.session.document_id() != to
            && let Err(e) = self.session.switch_document(to)
        {
            log::error!("couldn't show document {to}: {e}");
        }
        if from == to {
            return false;
        }
        let leaving = Parked { autosave_path: self.autosave_path.take(), comments: std::mem::take(&mut self.canvas.comments) };
        if self.session.is_open(from) {
            self.parked.insert(from, leaving);
        }
        let back = self.parked.remove(&to);
        let seen = back.is_some();
        let back = back.unwrap_or_default();
        self.autosave_path = back.autosave_path;
        self.canvas.comments = back.comments;
        let session = &self.session;
        self.parked.retain(|d, _| session.is_open(*d));
        seen
    }

    /// Open the file at `path` for the window showing document `into` (`None`: the active
    /// document's window). Every way of opening a file comes through here, so each follows the
    /// same rule: an untouched blank document is replaced in its own window, else the file opens
    /// in a window of its own (or its window comes forward if it is already open). A file a
    /// dialog picked arrives while any window draws, so the document it was asked from is made
    /// active again first.
    pub fn open_file(&mut self, path: &str, into: Option<u64>) -> Result<Value, String> {
        if let Some(id) = into
            && id != self.session.document_id()
            && self.session.is_open(id)
        {
            self.show_document(id)?;
        }
        self.run("file.open", json!({"path": path}))
    }

    /// Open a file the platform hands the app: one named on the command line (a double-click in
    /// the file manager, the shell's Open verb) or sent by the Finder. A Word template (`.dotx`,
    /// `.dotm`, `.dot`) starts a new, untitled document based on it, as Word's default verb for a
    /// template does (#3433): saving it asks where to write, and the template is never written
    /// over. File › Open still opens a template itself, to edit it, as in Word. Anything else
    /// opens as [`WordApp::open_file`] opens it. `data` holds the file's bytes (base64) when its
    /// name can't be passed as text (#1440); `path` then only names it.
    pub fn open_from_platform(&mut self, path: &str, data: Option<&str>) -> Result<Value, String> {
        let mut params = json!({"path": path});
        if let Some(data) = data {
            params["data"] = json!(data);
        }
        if is_word_template(path) {
            params["replaceBlank"] = json!(true);
            return self.run("file.newFromTemplate", params);
        }
        match data {
            Some(_) => self.run("file.open", params),
            None => self.open_file(path, None),
        }
    }

    /// Show open document `id` in the window (View › Window › Switch Windows, quitting).
    pub(crate) fn show_document(&mut self, id: u64) -> Result<(), String> {
        self.session.activate(id).map_err(|e| e.to_string())?;
        self.sync_document();
        Ok(())
    }

    /// Whether the open document `id` has changes that aren't written to a file yet, or a save
    /// the browser hasn't confirmed. The Save Changes prompt asks about the same work, so the
    /// prompt and a programmatic refusal cannot drift apart (#1268, #3274).
    pub(crate) fn document_unsaved_by_id(&self, id: u64) -> bool {
        let dirty = self.session.document_dirty(id);
        let unconfirmed = self.saves_in_flight.iter().any(|s| s.marked_saved && s.document == id);
        dirty || unconfirmed
    }

    /// Whether the open document `id` has comment or reply text typed but not yet written to
    /// it (#1348): closing the document would drop that text.
    pub(crate) fn document_has_comment_drafts(&self, id: u64) -> bool {
        if id == self.session.document_id() {
            return panes::has_comment_drafts(self);
        }
        self.parked.get(&id).is_some_and(|p| panes::state_has_drafts(&p.comments))
    }

    /// The open documents with work that would be lost on quitting: the shown one first, then
    /// the others in the order they were opened. Unsaved changes, a save the browser hasn't
    /// confirmed, or comment text not yet written to the document. The Start screen's
    /// placeholder is never one, though other windows' documents are.
    fn unsaved_documents(&self) -> Vec<u64> {
        let active = self.session.document_id();
        let mut out = Vec::new();
        if self.has_document() && (self.document_unsaved_by_id(active) || self.document_has_comment_drafts(active)) {
            out.push(active);
        }
        for d in self.session.documents() {
            if d.id != active && Some(d.id) != self.no_document && (self.document_unsaved_by_id(d.id) || self.document_has_comment_drafts(d.id)) {
                out.push(d.id);
            }
        }
        out
    }

    /// Whether an open document has work that quitting would lose.
    pub(crate) fn has_unsaved_documents(&self) -> bool {
        !self.unsaved_documents().is_empty()
    }

    /// Quitting: start the quit guard over the documents with unsaved work, which shows each in
    /// turn and asks Save / Don't Save / Cancel about it. A guard already running keeps its open
    /// prompt. With none left, the app quits.
    pub(crate) fn ask_before_quit(&mut self) -> Value {
        let documents = self.unsaved_documents();
        if documents.is_empty() {
            self.quit_requested = true;
            return json!({"quit": true});
        }
        if self.closing.is_some() {
            return json!({"quit": false});
        }
        self.ui.backstage = false;
        match self.begin_closing(craft::quit::CloseScope::Quit, documents, "file.close".into(), json!({"quit": true})) {
            Ok(_) => json!({"quit": self.quit_requested}),
            Err(e) => json!({"quit": false, "error": e}),
        }
    }
}

/// Is `path` a Word template by its extension (`.dotx`, `.dotm`, `.dot`, in any case)? These are
/// the template types WordCraft registers with the platform.
pub fn is_word_template(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|e| ["dotx", "dotm", "dot"].iter().any(|t| e.eq_ignore_ascii_case(t)))
}

/// The name an open document goes by in the window: the engine's title, with an untitled
/// document's `Document2` in the interface language.
pub fn display_title(d: &DocInfo) -> String {
    if d.path.is_none() {
        return untitled_title(&d.title).unwrap_or_else(|| d.title.clone());
    }
    d.title.clone()
}

/// `Document2` (the engine's English name for an untitled document) in the interface language,
/// or None for any other name. The catalogs translate `Document1`; the number follows it.
pub(crate) fn untitled_title(english: &str) -> Option<String> {
    let n = english.strip_prefix("Document")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let one = tl!("Document1");
    Some(match one.strip_suffix('1') {
        Some(stem) => format!("{stem}{n}"),
        None if n == "1" => one.to_string(),
        None => format!("{one} ({n})"),
    })
}

/// View › Window › Switch Windows: the open windows (in one window, the open documents) in the
/// order they were opened, this one marked, unsaved ones with the title bar's dot. Choosing one
/// brings that window to the front (in one window, shows the document).
pub(crate) fn switch_windows_menu(ui: &mut egui::Ui, app: &mut WordApp) {
    let native = app.windows.native();
    let entries: Vec<(u64, String, bool)> = if native {
        app.switch_windows_entries()
    } else {
        app.session.documents().into_iter().map(|d| (d.id, format!("{}{}", display_title(&d), if d.dirty { " •" } else { "" }), d.active)).collect()
    };
    for (key, label, current) in entries {
        if MenuItem::new(label).radio_mark(current).show(ui).ran {
            if native {
                app.show_window(key);
            } else {
                let _ = app.run("view.switchWindows", json!({"id": key}));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::StoryRef;
    use wordcraft_engine::Session;

    use crate::{Services, WordApp};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        crate::test_harness::builder().with_size(egui::vec2(2560.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
    }

    fn text(a: &WordApp) -> String {
        a.session.doc.plain_text(StoryRef::Body).trim().to_string()
    }

    /// The Save Changes prompt, when the close operation has one open. The framework's prompt has
    /// no command key, so it is found by the id `closing` holds, among the open dialogs.
    fn prompt(a: &WordApp) -> Option<craft::ui::dialog::DialogId> {
        let id = a.closing.as_ref().and_then(|c| c.prompt)?;
        a.dialog_host.open_dialogs().into_iter().find(|d| d.id == id).map(|d| d.id)
    }

    /// Mod+N with text typed opens a second document beside the first; Switch Windows lists both
    /// and brings the first back with its text.
    #[test]
    fn new_keeps_the_typed_document_and_switch_windows_brings_it_back() {
        use egui_kittest::kittest::Queryable;
        let mut a = app();
        a.run("text.insert", json!({"text": "First draft"})).unwrap();
        let first = a.session.document_id();
        let mut h = harness(a);
        // Each frame is given the app's command input, which the Mod+N chord resolves through.
        crate::test_harness::frames(&mut h, 2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
        crate::test_harness::frames(&mut h, 2);
        assert_ne!(h.state().session.document_id(), first, "Mod+N shows a new document");
        assert_eq!(text(h.state()), "");
        assert!(h.state().window_title().starts_with("Document2"), "{}", h.state().window_title());

        h.state_mut().run("ui.tab", json!({"tab": "View"})).unwrap();
        crate::test_harness::frames(&mut h, 2);
        // The ribbon's stacked label breaks the line after "Switch".
        // A split button's main part runs its command; its arrow part, the inner node of the same name (the label
        // and chevron), opens the menu.
        let at = h.query_all_by_label("Switch\nWindows").last().expect("the Switch Windows arrow").rect().center();
        // Pressed the way a person does, so the framework opens the menu: the pointer rests a frame over the arrow, and
        // the press and the release are frames of their own (kittest's `click` sends all three in one).
        h.event(egui::Event::PointerMoved(at));
        crate::test_harness::frames(&mut h, 2);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            crate::test_harness::frames(&mut h, 1);
        }
        crate::test_harness::frames(&mut h, 6);
        // The rows are radio menu items, the shown document marked.
        use egui::accesskit::Role::MenuItemRadio as Row;
        assert!(h.query_by_role_and_label(Row, "Document1 •").is_some(), "the first document is listed, unsaved");
        assert!(h.query_by_role_and_label(Row, "Document2").is_some(), "the new one is listed");
        h.get_by_role_and_label(Row, "Document1 •").click();
        crate::test_harness::frames(&mut h, 2);
        let a = h.state();
        assert_eq!(a.session.document_id(), first);
        assert_eq!(text(a), "First draft");
        assert_eq!(a.session.documents().len(), 2);
        assert!(a.window_title().starts_with("Document1"), "{}", a.window_title());
    }

    /// Close with two documents open asks about the shown one's changes; after Don't Save the
    /// other one shows.
    #[test]
    fn close_asks_then_shows_the_other_document() {
        let mut a = app();
        a.run("text.insert", json!({"text": "Kept"})).unwrap();
        let kept = a.session.document_id();
        a.run("file.new", json!({})).unwrap();
        assert!(prompt(&a).is_none(), "New asks nothing: nothing is replaced");
        a.run("text.insert", json!({"text": "Dropped"})).unwrap();
        a.run("file.close", json!({})).unwrap();
        assert!(prompt(&a).is_some() && a.closing.is_some(), "the close asks about the unsaved document");
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert_eq!(a.session.document_id(), kept);
        assert_eq!(text(&a), "Kept");
        assert_eq!(a.session.documents().len(), 1);
        assert!(a.has_document() && !a.quit_requested);
    }

    /// Closing the window with a document of unsaved changes behind the shown one asks about it
    /// (showing it); Cancel keeps the app open, and Don't Save quits.
    #[test]
    fn quitting_asks_about_a_document_that_is_not_shown() {
        let mut a = app();
        a.run("text.insert", json!({"text": "Behind"})).unwrap();
        let behind = a.session.document_id();
        a.run("file.new", json!({})).unwrap();
        assert!(!a.close_requested());
        assert_eq!(a.session.document_id(), behind, "the unsaved document is shown");
        assert!(prompt(&a).is_some() && a.closing.is_some(), "the quit guard asks about it");
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(!a.quit_requested);
        assert_eq!(a.session.documents().len(), 2, "nothing closed");

        assert!(!a.close_requested(), "asked again");
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(a.quit_requested);
        assert!(a.close_requested());
    }

    /// In one window (the web app), a request an agent's command makes for a document that isn't
    /// shown never acts on the shown one: Print doesn't open Backstage over it.
    #[test]
    fn requests_for_a_document_not_shown_leave_the_window_alone() {
        let mut a = app();
        a.windows.single();
        let first = a.session.document_id();
        a.run("file.new", json!({})).unwrap();
        a.session.with_document(first, |s| s.run("file.print", &json!({}))).unwrap().unwrap();
        for req in std::mem::take(&mut a.session.ui_requests) {
            assert_eq!(req["document"], first);
            a.handle_request(&req);
        }
        assert!(!a.ui.backstage, "Print for the other document left the window alone");
    }

    #[test]
    fn untitled_names_follow_the_interface_language() {
        assert_eq!(super::untitled_title("Document3").as_deref(), Some("Document3"));
        assert_eq!(super::untitled_title("Documents"), None);
        assert_eq!(super::untitled_title("Report"), None);
    }

    /// From the Start screen, Blank document is `Document1` and New after it `Document2`: the
    /// Start screen's stand-in document doesn't use up a name.
    #[test]
    fn start_screen_blank_document_is_document1() {
        let mut a = app();
        a.ui.start_screen = true;
        assert!(a.start_screen());
        a.run("file.new", json!({})).unwrap();
        let titles = |a: &WordApp| a.session.documents().into_iter().map(|d| d.title).collect::<Vec<_>>();
        assert_eq!(titles(&a), ["Document1"]);
        a.run("file.new", json!({})).unwrap();
        assert_eq!(titles(&a), ["Document1", "Document2"]);
    }
}
