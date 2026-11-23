use std::path::PathBuf;
use std::sync::Arc;

use craft::automation::control::{ControlRequest, Reply};
use craft::host::{Picked, ScriptedFiles};
use craft_ui_test_kit::windows::Windows as Kit;
use egui::{ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
use serde_json::json;
use wordcraft_doc::StoryRef;
use wordcraft_engine::Session;

use crate::{Services, WordApp};

/// The window kit with the root and one child viewport: what each reports about itself and
/// what the frames sent and drew.
struct Rig {
    kit: Kit,
    /// The viewport with OS focus.
    focused: Option<ViewportId>,
    /// Viewports whose next frame carries a close request.
    closing: Vec<ViewportId>,
    /// The window commands of the last frame.
    commands: Vec<(ViewportId, ViewportCommand)>,
    /// The viewports that drew in the last frame, in order.
    drawn: Vec<ViewportId>,
    /// How the viewports asked to open their windows in every frame so far.
    builders: Vec<egui::ViewportBuilder>,
}

impl Rig {
    fn new() -> Rig {
        Rig { kit: Kit::new(), focused: None, closing: Vec::new(), commands: Vec::new(), drawn: Vec::new(), builders: Vec::new() }
    }

    fn ctx(&self) -> egui::Context {
        self.kit.ctx.clone()
    }

    /// The facts of the root (`child` false) or of the kit's one child, which the kit reports for whichever child
    /// viewport the app opens: a request made for any non-root viewport (`close`, `focus`) is the child's.
    fn info(&self, child: bool) -> ViewportInfo {
        let is_it = |v: &ViewportId| (*v != ViewportId::ROOT) == child;
        let events = if self.closing.iter().any(is_it) { vec![ViewportEvent::Close] } else { Vec::new() };
        // The window host reads a window's facts (focus included) only once it has reported its inner rect.
        let size = if child { egui::vec2(400.0, 300.0) } else { egui::vec2(800.0, 600.0) };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        ViewportInfo { focused: Some(self.focused.iter().any(is_it)), events, inner_rect: Some(rect), outer_rect: Some(rect), ..Default::default() }
    }

    /// One frame of every window.
    fn frame(&mut self, a: &mut WordApp) {
        self.kit.root_info(self.info(false));
        // The kit has one child, whichever viewport the app opens it in.
        self.kit.child_info(self.info(true));
        // PLAN: assumed `embed::before_input(ctx, |out| app.frame_input(out))` (sh.A2) from app-hub.
        craft::ui::shell::embed::before_input(&self.kit.ctx, |out| a.frame_input(out));
        let out = self.kit.frame(Vec::new(), Vec::new(), |ui| {
            a.logic(ui.ctx());
            // The root window's close request goes through the app's close decision, as the host's frame runs it.
            craft::ui::shell::embed::logic(ui.ctx(), || a.close_decision());
            a.ui(ui);
        });
        self.builders.extend(out.builders.into_iter().map(|(_, b)| b));
        self.commands = out.commands;
        self.drawn = out.trees.iter().map(|(v, _)| *v).collect();
        self.closing.clear();
    }

    fn frames(&mut self, a: &mut WordApp, n: usize) {
        for _ in 0..n {
            self.frame(a);
        }
    }

    /// Focus viewport `id` alone, as the desktop does when a window is clicked.
    fn focus(&mut self, id: ViewportId) {
        self.focused = Some(id);
    }

    /// The window's close button is pressed in the next frame.
    fn close(&mut self, id: ViewportId) {
        self.closing.push(id);
    }

    fn sent(&self, id: ViewportId, cmd: &ViewportCommand) -> bool {
        self.commands.iter().any(|(v, c)| *v == id && c == cmd)
    }
}

fn app(text: &str) -> WordApp {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("text.insert", json!({"text": text})).unwrap();
    a
}

fn text(a: &WordApp) -> String {
    a.session.doc.plain_text(StoryRef::Body).trim().to_string()
}

/// What the window asks: `saveChanges` while a close waits on the Save prompt, else the key of the
/// first open dialog.
fn prompt(a: &WordApp) -> Option<String> {
    if a.closing.as_ref().is_some_and(|c| c.prompt.is_some()) {
        return Some("saveChanges".to_owned());
    }
    a.dialog_host.open_dialogs().into_iter().find_map(|d| d.key)
}

/// New opens the document in a second native window, focused, with a fresh interface; the
/// first keeps its document, its ribbon tab and its open dialog, and gets them back when it
/// is focused again.
#[test]
fn new_opens_a_second_window_and_each_keeps_its_state() {
    let mut rig = Rig::new();
    let mut a = app("First");
    let first = a.session.document_id();
    rig.frames(&mut a, 3);
    a.ui.tab = "Insert".into();
    a.run("ui.dialog", json!({"name": "paragraph"})).unwrap();
    assert_eq!(prompt(&a).as_deref(), Some("paragraph"));

    a.run("file.new", json!({})).unwrap();
    let second = a.session.document_id();
    assert_ne!(second, first);
    assert_eq!(a.window_count(), 2);
    assert!(a.dialog_host.is_empty() && a.ui.tab == crate::DEFAULT_TAB, "the new window starts afresh");
    rig.frames(&mut a, 3);
    let child = a.active_viewport();
    assert_ne!(child, ViewportId::ROOT, "the new window has a viewport of its own, the one its container names");
    assert!(rig.drawn.contains(&child) && rig.drawn.contains(&ViewportId::ROOT), "both windows draw each frame: {:?}", rig.drawn);
    assert!(a.window_title().starts_with("Document2"), "{}", a.window_title());
    assert_eq!(a.session.document_id(), second);
    a.run("text.insert", json!({"text": "Second"})).unwrap();

    rig.focus(ViewportId::ROOT);
    rig.frames(&mut a, 2);
    assert_eq!(a.active_viewport(), ViewportId::ROOT);
    assert_eq!(a.session.document_id(), first);
    assert_eq!((text(&a), a.ui.tab.as_str(), prompt(&a).as_deref()), ("First".into(), "Insert", Some("paragraph")));
    rig.focus(child);
    rig.frames(&mut a, 2);
    assert_eq!((text(&a), a.ui.tab.as_str(), prompt(&a).as_deref()), ("Second".into(), crate::DEFAULT_TAB, None));
}

/// Each window has its own key, and the root window's stays `main` when it adopts another entry.
#[test]
fn window_keys_differ_and_the_root_keeps_main() {
    let mut rig = Rig::new();
    let mut a = app("First");
    rig.frames(&mut a, 2);
    assert_eq!(a.windows.window_key().as_str(), crate::windows::WindowKey::main().as_str());
    a.run("file.new", json!({})).unwrap();
    rig.frames(&mut a, 2);
    let second = a.windows.window_key();
    assert_eq!(second.as_str(), format!("window-{}", a.windows.shown()));
    assert_ne!(second.as_str(), crate::windows::WindowKey::main().as_str());

    // The root window's close button closes that window alone; its document has unsaved text, so it asks first.
    rig.close(ViewportId::ROOT);
    rig.frame(&mut a);
    assert_eq!(prompt(&a).as_deref(), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    rig.frame(&mut a);
    assert_eq!(a.window_count(), 1);
    assert_eq!(a.windows.window_key().as_str(), crate::windows::WindowKey::main().as_str(), "the adopted entry is the root's");
}

fn docx_on_disk(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-windows-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.docx"));
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": body})).unwrap();
    s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
    path
}

/// Open of a file a window already shows brings that window to the front; Open with only an
/// untouched blank document open replaces it in its own window.
#[test]
fn open_focuses_the_files_window_or_reuses_a_blank_one() {
    let path = docx_on_disk("report", "Report");
    let mut rig = Rig::new();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    rig.frames(&mut a, 2);
    let window = a.windows.shown();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!((a.window_count(), a.windows.shown(), text(&a).as_str()), (1, window, "Report"), "the blank document's window shows it");
    let report = a.session.document_id();

    a.run("file.new", json!({})).unwrap();
    rig.frames(&mut a, 2);
    assert_eq!(a.window_count(), 2);
    assert_ne!(a.session.document_id(), report);
    let r = a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["alreadyOpen"], true);
    assert_eq!((a.window_count(), a.windows.shown(), a.session.document_id()), (2, window, report));
    rig.frames(&mut a, 1);
    assert!(rig.sent(ViewportId::ROOT, &ViewportCommand::Focus), "the file's window comes to the front");
    assert_eq!(a.session.document_id(), report);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// A window's close button closes it, asking first about unsaved changes; after Don't Save the
/// other window comes forward. When the root's window closes, the other window moves into the
/// root viewport and keeps its page's place.
#[test]
fn closing_a_window_asks_first_and_the_root_adopts_another() {
    let mut rig = Rig::new();
    let mut a = app("Kept");
    a.run("file.save", json!({"path": docx_on_disk("kept", "Kept").to_string_lossy()})).unwrap();
    let kept = a.session.document_id();
    rig.frames(&mut a, 2);
    a.run("file.new", json!({})).unwrap();
    a.run("text.insert", json!({"text": "Dropped"})).unwrap();
    rig.frames(&mut a, 2);
    let child = a.active_viewport();

    rig.close(child);
    rig.frame(&mut a);
    assert_eq!(prompt(&a).as_deref(), Some("saveChanges"), "the dirty window asks");
    assert_eq!((a.window_count(), a.active_viewport()), (2, child));
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert_eq!((a.window_count(), a.session.document_id(), text(&a).as_str()), (1, kept, "Kept"));
    assert_eq!(a.active_viewport(), ViewportId::ROOT);
    rig.frames(&mut a, 2);
    assert_eq!(rig.drawn, [ViewportId::ROOT], "the closed window's viewport is gone");

    a.run("file.new", json!({})).unwrap();
    let other = a.windows.shown();
    rig.frames(&mut a, 2);
    let offset = a.canvas.view.offset;
    rig.close(ViewportId::ROOT);
    assert!(a.closes_a_window(), "the host leaves the root's close to the app");
    rig.frame(&mut a);
    assert!(!rig.sent(ViewportId::ROOT, &ViewportCommand::Close), "the app keeps running");
    assert!(!a.quit_requested);
    assert_eq!((a.window_count(), a.windows.shown()), (1, other));
    assert_eq!(a.windows.root.get(), other, "the remaining window is the root's entry");
    assert_eq!(a.active_viewport(), ViewportId::ROOT, "the remaining window moved into the root viewport");
    assert_eq!(a.canvas.view.offset, offset, "its page stays where it was");
    rig.frames(&mut a, 2);
    assert_eq!(rig.drawn, [ViewportId::ROOT]);
    assert!(a.has_document() && a.session.documents().len() == 1);
}

/// A request an agent's command makes for the document of another window is that window's:
/// Print opens the Print page there when it draws, and the active window and document stay
/// as they are, so the agent's next command works where it did.
#[test]
fn a_request_for_another_windows_document_is_handled_there() {
    let mut rig = Rig::new();
    let mut a = app("First");
    let first = a.session.document_id();
    let first_window = a.windows.shown();
    rig.frames(&mut a, 2);
    a.run("file.new", json!({})).unwrap();
    let second = a.session.document_id();
    rig.frames(&mut a, 2);
    let child = a.active_viewport();
    a.session.with_document(first, |s| s.run("file.print", &json!({}))).unwrap().unwrap();
    for req in std::mem::take(&mut a.session.ui_requests) {
        assert_eq!(req["document"], first);
        a.handle_request(&req);
    }
    assert_eq!((a.session.document_id(), a.active_viewport()), (second, child), "the active document stays");
    assert!(!a.ui.backstage);
    rig.frames(&mut a, 2);
    assert_eq!((a.session.document_id(), a.active_viewport()), (second, child));
    assert!(!a.ui.backstage, "the active window keeps its page");
    let other = a.windows.get(first_window).and_then(|w| w.ui.as_ref()).map(|w| (w.backstage, w.backstage_page.as_str()));
    assert_eq!(other, Some((true, "print")), "the Print page opened in the document's window");
}

/// Quitting from the Start screen's empty window asks about the documents other windows
/// show (one an agent made and typed in).
#[test]
fn quitting_from_the_empty_window_asks_about_the_other_windows() {
    let mut rig = Rig::new();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    rig.frames(&mut a, 2);
    a.run("file.close", json!({})).unwrap();
    assert!(!a.has_document());
    let empty = a.windows.shown();
    let placeholder = a.session.document_id();
    let other = a.session.with_document(placeholder, |s| s.add_document(wordcraft_doc::Document::new())).unwrap();
    a.session.with_document(other, |s| s.run("text.insert", &json!({"text": "Agent"}))).unwrap().unwrap();
    rig.frames(&mut a, 2);
    assert_eq!((a.window_count(), a.windows.shown()), (2, empty), "the agent's document has a window behind");
    assert!(!a.has_document());

    assert!(!a.quit());
    assert_eq!((a.session.document_id(), prompt(&a).as_deref(), text(&a).as_str()), (other, Some("saveChanges"), "Agent"));
    assert_ne!(a.windows.shown(), empty, "the document's own window asks");
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(a.quit_requested);
}

/// A window whose document closed without the windows following (an agent closed it) isn't
/// drawn or made active; the next frame closes it.
#[test]
fn a_window_whose_document_closed_is_not_entered() {
    let mut rig = Rig::new();
    let mut a = app("First");
    let first = a.session.document_id();
    let first_window = a.windows.shown();
    rig.frames(&mut a, 2);
    a.run("file.new", json!({})).unwrap();
    let (second, second_window) = (a.session.document_id(), a.windows.shown());
    rig.frames(&mut a, 2);
    a.session.close_document(first).unwrap();
    assert!(!a.enter_window(first_window));
    assert_eq!((a.windows.shown(), a.session.document_id()), (second_window, second));
    rig.frames(&mut a, 2);
    assert_eq!((a.window_count(), a.session.document_id()), (1, second));
}

/// Quitting with two documents of unsaved changes asks about each in its window, the window
/// coming to the front; Cancel keeps both.
#[test]
fn quitting_asks_about_each_document_in_its_window() {
    let mut rig = Rig::new();
    let mut a = app("One");
    let one = a.session.document_id();
    rig.frames(&mut a, 2);
    a.run("file.new", json!({})).unwrap();
    a.run("text.insert", json!({"text": "Two"})).unwrap();
    let two = a.session.document_id();
    rig.frames(&mut a, 2);
    // Read once the window has drawn: before that it has no viewport and the root stands in.
    let child = a.active_viewport();
    assert_ne!(child, ViewportId::ROOT);

    assert!(!a.quit());
    assert_eq!((a.session.document_id(), prompt(&a).as_deref()), (two, Some("saveChanges")), "the active window asks first");
    a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    assert!(!a.quit_requested);
    assert_eq!((a.window_count(), a.session.documents().len()), (2, 2), "Cancel keeps both");

    assert!(!a.quit());
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert_eq!((a.session.document_id(), prompt(&a).as_deref()), (one, Some("saveChanges")), "then the other window asks");
    assert_eq!(a.window_count(), 1);
    rig.frame(&mut a);
    assert!(rig.sent(ViewportId::ROOT, &ViewportCommand::Focus), "its window comes to the front");
    assert!(!rig.drawn.contains(&child));
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(a.quit_requested);
}

fn window_labels(a: &WordApp) -> Vec<String> {
    let mut l: Vec<String> = a.switch_windows_entries().into_iter().map(|(_, label, _)| label).collect();
    l.sort();
    l
}

/// View › New Window opens a second window on the document, titled `Name:1` / `Name:2`. Text
/// typed in one shows in the other; each keeps its own caret and zoom. Typing in a window
/// whose caret was in text the other window deleted lands at the end of what is left, and
/// Undo in that window undoes its own typing, then the other window's delete.
#[test]
fn new_window_shows_the_document_twice_each_with_its_own_caret_and_zoom() {
    let mut rig = Rig::new();
    let mut a = app("Hello world");
    let doc = a.session.document_id();
    rig.frames(&mut a, 2);
    a.run("view.newWindow", json!({})).unwrap();
    assert_eq!((a.window_count(), a.session.document_id()), (2, doc));
    rig.frames(&mut a, 3);
    let b = a.active_viewport();
    assert_ne!(b, ViewportId::ROOT);
    assert_eq!(window_labels(&a), ["Document1:1 •", "Document1:2 •"]);
    assert_eq!(a.window_title(), super::title("Document1:2", true));

    a.run("view.zoom", json!({"value": 150})).unwrap();
    a.run("text.insert", json!({"text": "!"})).unwrap();
    rig.focus(ViewportId::ROOT);
    rig.frames(&mut a, 2);
    assert_eq!(a.active_viewport(), ViewportId::ROOT);
    assert_eq!(text(&a), "Hello world!", "the edit shows in the other window");
    assert_eq!((a.session.view.zoom, a.session.sel.focus.off), (1.0, 11), "window A keeps its zoom and caret");

    a.run("select.text", json!({"text": " world!"})).unwrap();
    a.run("text.delete", json!({})).unwrap();
    rig.focus(b);
    rig.frames(&mut a, 2);
    assert_eq!(a.active_viewport(), b);
    assert!((a.session.view.zoom - 1.5).abs() < 1e-6, "window B keeps its zoom");
    a.run("text.insert", json!({"text": "?"})).unwrap();
    assert_eq!(text(&a), "Hello?", "B's caret was clamped into what is left");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(text(&a), "Hello");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(text(&a), "Hello world!", "Undo in B undoes A's delete");
    rig.frames(&mut a, 2);
    assert_eq!(a.session.documents().len(), 1);
}

/// Closing one of two windows on a document asks nothing, even with unsaved changes, and the
/// other window goes back to the plain name; closing the last one asks. Quitting with a
/// document in two windows asks once.
#[test]
fn closing_one_of_two_windows_on_a_document_asks_nothing() {
    let mut rig = Rig::new();
    let mut a = app("Kept");
    let doc = a.session.document_id();
    rig.frames(&mut a, 2);
    a.run("view.newWindow", json!({})).unwrap();
    rig.frames(&mut a, 2);
    let child = a.active_viewport();
    rig.close(child);
    rig.frame(&mut a);
    assert_eq!(prompt(&a).as_deref(), None, "no prompt while another window shows the document");
    assert_eq!((a.window_count(), a.session.document_id(), text(&a).as_str()), (1, doc, "Kept"));
    assert!(a.session.dirty);
    rig.frames(&mut a, 2);
    assert_eq!(a.window_title(), super::title("Document1", true));
    a.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&a).as_deref(), Some("saveChanges"), "the last window asks");
    a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();

    a.run("view.newWindow", json!({})).unwrap();
    rig.frames(&mut a, 2);
    assert_eq!(a.window_count(), 2);
    assert!(!a.quit());
    assert_eq!(prompt(&a).as_deref(), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(a.quit_requested, "one prompt for the document");
}

/// #3274: a programmatic `file.close` on a document shown in two windows closes the window
/// alone, as the window's own close does: the document and its work stay open, so nothing is
/// lost and the control channel must not refuse it as unsaved work it would never lose.
#[test]
fn a_programmatic_close_of_one_of_two_windows_on_a_document_is_accepted() {
    let mut rig = Rig::new();
    let mut a = app("Kept");
    let doc = a.session.document_id();
    rig.frames(&mut a, 2);
    a.run("view.newWindow", json!({})).unwrap();
    rig.frames(&mut a, 2);
    assert_eq!(a.window_count(), 2);
    assert!(a.session.dirty, "the document has unsaved work");

    // The control channel's refusal: it must not stand in the way of a close that keeps the
    // document open.
    let refused = crate::control::close_refused(&a, "file.close", &json!({}));
    assert!(refused.is_none(), "closing a window alone loses nothing: {refused:?}");

    // And the command itself runs, closing the window and leaving the document and its work.
    let (req, _reply) = ControlRequest::new("engine.execute", json!({"command": "file.close"}));
    let r = match crate::control::handle(&mut a, &rig.ctx(), &req) {
        Reply::Done(v) => v,
        _ => json!({"ok": false, "error": "not a reply"}),
    };
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(a.window_count(), 1, "the window closed");
    assert_eq!(a.session.document_id(), doc, "the document stays open");
    assert!(a.session.dirty, "its work stays");
    assert_eq!(text(&a), "Kept");

    // The last window closes the document, so the same command is refused for its unsaved work.
    let refused = crate::control::close_refused(&a, "file.close", &json!({}));
    assert!(refused.is_some(), "closing the document would lose the work: {refused:?}");
}

/// The file picks the app asked for, answered when the test says so.
type Shown = Arc<ScriptedFiles>;

/// Two windows, the second (focused) on an untouched blank document: an app whose host
/// answers file dialogs only when the test does.
fn blank_beside_a_document() -> (Rig, WordApp, Shown, u64) {
    let files: Shown = Arc::new(ScriptedFiles::held());
    let mut rig = Rig::new();
    let host = Arc::new(craft::host::Host::builder(crate::APP).files(files.clone()).build());
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default()).with_host(host);
    rig.frames(&mut a, 2);
    a.run("file.open", json!({"path": docx_on_disk("first", "First").to_string_lossy()})).unwrap();
    a.run("file.new", json!({})).unwrap();
    rig.frames(&mut a, 2);
    assert!(a.window_count() == 2 && a.session.is_untouched_blank());
    let blank = a.windows.shown();
    (rig, a, files, blank)
}

/// Each way of opening a file replaces an untouched blank document in its own window, even
/// when the file arrives while another window draws: File › Open, ⌘O and the File page's
/// Browse (an asynchronous dialog), the File page's and Start screen's recent list, and
/// Finder (between frames, in the focused window).
#[test]
fn every_open_replaces_an_untouched_blank_document_in_its_window() {
    let second = docx_on_disk("second", "Second");
    let path = second.to_string_lossy().to_string();
    let opens: [(&str, fn(&mut WordApp, &mut Rig, &Shown, &str)); 3] = [
        ("dialog", |a, rig, files, path| {
            a.run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
            a.run("ui.openFileDialog", json!({})).unwrap();
            rig.frames(a, 2);
            assert!(files.answer(Picked::Paths(vec![PathBuf::from(path)])).is_some(), "the dialog asked for a file");
            rig.frames(a, 3);
        }),
        ("recent", |a, rig, _, path| {
            crate::backstage::open_recent(a, path).unwrap();
            rig.frames(a, 2);
        }),
        ("finder", |a, rig, _, path| {
            a.open_file(path, None).unwrap();
            rig.frames(a, 2);
        }),
    ];
    for (how, open) in opens {
        let (mut rig, mut a, files, blank) = blank_beside_a_document();
        open(&mut a, &mut rig, &files, &path);
        assert_eq!(a.window_count(), 2, "{how}: no third window");
        assert!(a.enter_window(blank), "{how}: the blank document's window is still open");
        assert_eq!(text(&a), "Second", "{how}: it shows the opened file");
    }
    std::fs::remove_dir_all(second.parent().unwrap()).ok();
}

/// #3634: a NUL in the document title never reaches an OS window title (X11 and Wayland panic on
/// it): not a native window's builder, not a `Title` command, not the window menu. Checks what the
/// frames send, so a new title source that skips `display_title` fails here.
#[test]
fn nul_in_the_document_title_never_reaches_an_os_window_title() {
    for raw in ["\u{0}", "a\u{0}b"] {
        let mut rig = Rig::new();
        let mut a = app("x");
        a.run("file.properties", json!({"title": raw})).unwrap();
        let mut sent = Vec::new();
        for i in 0..6 {
            if i == 3 {
                a.run("view.newWindow", json!({})).unwrap();
                assert_eq!(a.window_count(), 2);
            }
            rig.frame(&mut a);
            sent.extend(rig.commands.iter().filter_map(|(_, c)| if let ViewportCommand::Title(t) = c { Some(t.clone()) } else { None }));
        }
        sent.extend(rig.builders.iter().filter_map(|b| b.title.clone()));
        sent.extend(a.switch_windows_entries().into_iter().map(|e| e.1));
        assert!(rig.builders.iter().any(|b| b.title.is_some()), "the new window's builder carries a title");
        for t in &sent {
            assert!(std::ffi::CString::new(t.as_str()).is_ok(), "an OS window title without NUL ({raw:?}): {t:?}");
        }
    }
}
