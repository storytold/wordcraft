use std::cell::RefCell;
use std::collections::HashMap;

use egui::{RawInput, ViewportCommand, ViewportEvent, ViewportId, ViewportIdMap, ViewportInfo};
use serde_json::json;
use wordcraft_doc::StoryRef;
use wordcraft_engine::Session;

use crate::{Services, WordApp};

thread_local! {
    /// What the test backend tells each viewport about itself (focus, close requests).
    static INFOS: RefCell<ViewportIdMap<ViewportInfo>> = RefCell::new(ViewportIdMap::default());
    /// Input events for each viewport's next pass.
    static EVENTS: RefCell<HashMap<ViewportId, Vec<egui::Event>>> = RefCell::new(HashMap::new());
    /// The viewports that drew in the last frame, in order.
    static DRAWN: RefCell<Vec<ViewportId>> = const { RefCell::new(Vec::new()) };
    /// Viewport commands the app sent, by viewport.
    static COMMANDS: RefCell<HashMap<ViewportId, Vec<ViewportCommand>>> = RefCell::new(HashMap::new());
    /// Textures uploaded (page rasters, the font atlas) since the counter was reset.
    static UPLOADS: RefCell<usize> = const { RefCell::new(0) };
    /// Each viewport's last input-method output.
    static IME: RefCell<HashMap<ViewportId, Option<egui::output::IMEOutput>>> = RefCell::new(HashMap::new());
}

fn input(id: ViewportId) -> RawInput {
    let mut raw =
        RawInput { viewport_id: id, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))), ..Default::default() };
    raw.viewports = INFOS.with(|i| i.borrow().clone());
    raw.focused = raw.viewports.entry(id).or_default().focused.unwrap_or(false);
    raw.events = EVENTS.with(|e| e.borrow_mut().remove(&id).unwrap_or_default());
    raw
}

fn keep(out: egui::FullOutput, id: ViewportId) {
    DRAWN.with(|d| d.borrow_mut().push(id));
    UPLOADS.with(|u| *u.borrow_mut() += out.textures_delta.set.len());
    IME.with(|i| i.borrow_mut().insert(id, out.platform_output.ime.clone()));
    for (v, o) in &out.viewport_output {
        COMMANDS.with(|c| c.borrow_mut().entry(*v).or_default().extend(o.commands.iter().cloned()));
    }
    out.drop_without_applying_deltas();
}

/// A backend with native viewports, as eframe's: each immediate viewport runs a pass of its
/// own, with its own input.
fn backend() -> egui::Context {
    INFOS.with(|i| i.borrow_mut().clear());
    EVENTS.with(|e| e.borrow_mut().clear());
    COMMANDS.with(|c| c.borrow_mut().clear());
    UPLOADS.with(|u| *u.borrow_mut() = 0);
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    egui::Context::set_immediate_viewport_renderer(|ctx, viewport| {
        let id = viewport.ids.this;
        let mut raw = input(id);
        raw.viewports.entry(id).or_default().parent = Some(viewport.ids.parent);
        let mut cb = viewport.viewport_ui_cb;
        keep(ctx.run_ui(raw, |ui| cb(ui)), id);
    });
    ctx
}

/// One frame of every window.
fn frame(ctx: &egui::Context, a: &mut WordApp) {
    DRAWN.with(|d| d.borrow_mut().clear());
    let mut raw = input(ViewportId::ROOT);
    a.raw_input_hook(ctx, &mut raw);
    let out = ctx.run_ui(raw, |ui| {
        a.logic(ui.ctx());
        a.ui(ui);
    });
    keep(out, ViewportId::ROOT);
    INFOS.with(|i| i.borrow_mut().values_mut().for_each(|v| v.events.clear()));
}

fn frames(ctx: &egui::Context, a: &mut WordApp, n: usize) {
    for _ in 0..n {
        frame(ctx, a);
    }
}

/// Focus viewport `id` alone, as the desktop does when a window is clicked.
fn focus(id: ViewportId) {
    INFOS.with(|i| {
        let mut i = i.borrow_mut();
        for v in i.values_mut() {
            v.focused = Some(false);
        }
        i.entry(id).or_default().focused = Some(true);
    });
}

fn sent(id: ViewportId, cmd: &ViewportCommand) -> bool {
    COMMANDS.with(|c| c.borrow().get(&id).is_some_and(|v| v.contains(cmd)))
}

fn app(text: &str) -> WordApp {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("text.insert", json!({"text": text})).unwrap();
    a
}

fn text(a: &WordApp) -> String {
    a.session.doc.plain_text(StoryRef::Body).trim().to_string()
}

fn prompt(a: &WordApp) -> Option<&'static str> {
    a.dialog.as_ref().map(|d| d.name())
}

/// New opens the document in a second native window, focused, with a fresh interface; the
/// first keeps its document, its ribbon tab and its open dialog, and gets them back when it
/// is focused again.
#[test]
fn new_opens_a_second_window_and_each_keeps_its_state() {
    let ctx = backend();
    let mut a = app("First");
    let first = a.session.document_id();
    frames(&ctx, &mut a, 3);
    a.ui.tab = "Insert".into();
    a.run("ui.dialog", json!({"name": "paragraph"})).unwrap();
    assert_eq!(prompt(&a), Some("paragraph"));

    a.run("file.new", json!({})).unwrap();
    let second = a.session.document_id();
    assert_ne!(second, first);
    assert_eq!(a.window_count(), 2);
    assert!(a.dialog.is_none() && a.ui.tab == crate::DEFAULT_TAB, "the new window starts afresh");
    frames(&ctx, &mut a, 3);
    let child = a.active_viewport();
    assert_ne!(child, ViewportId::ROOT, "the new window has a viewport of its own");
    assert_eq!(DRAWN.with(|d| d.borrow().clone()), [child, ViewportId::ROOT], "both windows draw each frame");
    frames(&ctx, &mut a, 3);
    assert!(UPLOADS.with(|u| *u.borrow()) > 0, "the pages were rendered");
    UPLOADS.with(|u| *u.borrow_mut() = 0);
    frames(&ctx, &mut a, 2);
    assert_eq!(UPLOADS.with(|u| *u.borrow()), 0, "a window whose document didn't change renders no page again");
    assert!(a.sent_title.starts_with("Document2"), "{}", a.sent_title);
    assert_eq!(a.session.document_id(), second);
    a.run("text.insert", json!({"text": "Second"})).unwrap();

    focus(ViewportId::ROOT);
    frames(&ctx, &mut a, 2);
    assert_eq!(a.active_viewport(), ViewportId::ROOT);
    assert_eq!(a.session.document_id(), first);
    assert_eq!((text(&a), a.ui.tab.as_str(), prompt(&a)), ("First".into(), "Insert", Some("paragraph")));
    focus(child);
    frames(&ctx, &mut a, 2);
    assert_eq!((text(&a), a.ui.tab.as_str(), prompt(&a)), ("Second".into(), crate::DEFAULT_TAB, None));
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
    let ctx = backend();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    frames(&ctx, &mut a, 2);
    let window = a.windows.shown();
    a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!((a.window_count(), a.windows.shown(), text(&a).as_str()), (1, window, "Report"), "the blank document's window shows it");
    let report = a.session.document_id();

    a.run("file.new", json!({})).unwrap();
    frames(&ctx, &mut a, 2);
    assert_eq!(a.window_count(), 2);
    assert_ne!(a.session.document_id(), report);
    COMMANDS.with(|c| c.borrow_mut().clear());
    let r = a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["alreadyOpen"], true);
    assert_eq!((a.window_count(), a.windows.shown(), a.session.document_id()), (2, window, report));
    frames(&ctx, &mut a, 1);
    assert!(sent(ViewportId::ROOT, &ViewportCommand::Focus), "the file's window comes to the front");
    assert_eq!(a.session.document_id(), report);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// A window's close button closes it, asking first about unsaved changes; after Don't Save the
/// other window comes forward. When the root viewport's window closes, the other window moves
/// into the root viewport.
#[test]
fn closing_a_window_asks_first_and_the_root_adopts_another() {
    let ctx = backend();
    let mut a = app("Kept");
    a.run("file.save", json!({"path": docx_on_disk("kept", "Kept").to_string_lossy()})).unwrap();
    let kept = a.session.document_id();
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    a.run("text.insert", json!({"text": "Dropped"})).unwrap();
    frames(&ctx, &mut a, 2);
    let child = a.active_viewport();

    INFOS.with(|i| i.borrow_mut().entry(child).or_default().events.push(ViewportEvent::Close));
    frame(&ctx, &mut a);
    assert_eq!(prompt(&a), Some("saveChanges"), "the dirty window asks");
    assert_eq!((a.window_count(), a.active_viewport()), (2, child));
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert_eq!((a.window_count(), a.session.document_id(), text(&a).as_str()), (1, kept, "Kept"));
    assert_eq!(a.active_viewport(), ViewportId::ROOT);
    frames(&ctx, &mut a, 2);
    assert_eq!(DRAWN.with(|d| d.borrow().clone()), [ViewportId::ROOT], "the closed window's viewport is gone");

    a.run("file.new", json!({})).unwrap();
    let other = a.windows.shown();
    frames(&ctx, &mut a, 2);
    COMMANDS.with(|c| c.borrow_mut().clear());
    INFOS.with(|i| i.borrow_mut().entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close));
    assert!(a.closes_a_window(), "the host leaves the root's close to the app");
    frame(&ctx, &mut a);
    assert!(sent(ViewportId::ROOT, &ViewportCommand::CancelClose), "the app keeps running");
    assert!(!a.quit_requested);
    assert_eq!((a.window_count(), a.windows.shown()), (1, other));
    assert_eq!(a.active_viewport(), ViewportId::ROOT, "the remaining window moved into the root viewport");
    assert_eq!(a.canvas.scroll_request, Some(a.canvas.scroll_offset), "its page stays where it was");
    frames(&ctx, &mut a, 2);
    assert_eq!(DRAWN.with(|d| d.borrow().clone()), [ViewportId::ROOT]);
    assert!(a.has_document() && a.session.documents().len() == 1);
}

/// A request an agent's command makes for the document of another window is that window's:
/// Print opens the Print page there when it draws, and the active window and document stay
/// as they are, so the agent's next command works where it did.
#[test]
fn a_request_for_another_windows_document_is_handled_there() {
    let ctx = backend();
    let mut a = app("First");
    let first = a.session.document_id();
    let first_window = a.windows.shown();
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    let second = a.session.document_id();
    frames(&ctx, &mut a, 2);
    let child = a.active_viewport();
    a.session.with_document(first, |s| s.run("file.print", &json!({}))).unwrap().unwrap();
    for req in std::mem::take(&mut a.session.ui_requests) {
        assert_eq!(req["document"], first);
        a.handle_request(&req);
    }
    assert_eq!((a.session.document_id(), a.active_viewport()), (second, child), "the active document stays");
    assert!(!a.ui.backstage);
    frames(&ctx, &mut a, 2);
    assert_eq!((a.session.document_id(), a.active_viewport()), (second, child));
    assert!(!a.ui.backstage, "the active window keeps its page");
    let other = a.windows.get(first_window).and_then(|w| w.ui.as_ref()).map(|w| (w.backstage, w.backstage_page.as_str()));
    assert_eq!(other, Some((true, "print")), "the Print page opened in the document's window");
}

/// Quitting from the Start screen's empty window asks about the documents other windows
/// show (one an agent made and typed in).
#[test]
fn quitting_from_the_empty_window_asks_about_the_other_windows() {
    let ctx = backend();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    frames(&ctx, &mut a, 2);
    a.run("file.close", json!({})).unwrap();
    assert!(!a.has_document());
    let empty = a.windows.shown();
    let placeholder = a.session.document_id();
    let other = a.session.with_document(placeholder, |s| s.add_document(wordcraft_doc::Document::new())).unwrap();
    a.session.with_document(other, |s| s.run("text.insert", &json!({"text": "Agent"}))).unwrap().unwrap();
    frames(&ctx, &mut a, 2);
    assert_eq!((a.window_count(), a.windows.shown()), (2, empty), "the agent's document has a window behind");
    assert!(!a.has_document());

    assert!(!a.quit());
    assert_eq!((a.session.document_id(), prompt(&a), text(&a).as_str()), (other, Some("saveChanges"), "Agent"));
    assert_ne!(a.windows.shown(), empty, "the document's own window asks");
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(a.quit_requested);
}

/// Mod+W in a window with no changes closes it; the keys typed after it in the same frame
/// don't reach the window that comes forward.
#[test]
fn keys_after_closing_a_window_go_nowhere() {
    let ctx = backend();
    let mut a = app("First");
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    let child = a.active_viewport();
    focus(child);
    frames(&ctx, &mut a, 8);
    assert!(a.canvas.focused);
    let mac = cfg!(target_os = "macos");
    let modifiers = egui::Modifiers { ctrl: !mac, mac_cmd: mac, command: true, ..Default::default() };
    EVENTS.with(|e| {
        e.borrow_mut().entry(child).or_default().extend([
            egui::Event::Key { key: egui::Key::W, physical_key: None, pressed: true, repeat: false, modifiers },
            egui::Event::Text("x".into()),
        ])
    });
    frame(&ctx, &mut a);
    assert_eq!(a.window_count(), 1, "Mod+W closed the window");
    frames(&ctx, &mut a, 2);
    assert_eq!(text(&a), "First");
}

/// A window whose document closed without the windows following (an agent closed it) isn't
/// drawn or made active; the next frame closes it.
#[test]
fn a_window_whose_document_closed_is_not_entered() {
    let ctx = backend();
    let mut a = app("First");
    let first = a.session.document_id();
    let first_window = a.windows.shown();
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    let (second, second_window) = (a.session.document_id(), a.windows.shown());
    frames(&ctx, &mut a, 2);
    a.session.close_document(first).unwrap();
    assert!(!a.enter_window(first_window));
    assert_eq!((a.windows.shown(), a.session.document_id()), (second_window, second));
    frames(&ctx, &mut a, 2);
    assert_eq!((a.window_count(), a.session.document_id()), (1, second));
}

/// A composition in one window shows and commits only there: the other window draws no
/// composition and its document gets none of the text.
#[test]
fn a_composition_stays_in_its_window() {
    let ctx = backend();
    let mut a = app("A");
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    a.run("text.insert", json!({"text": "B"})).unwrap();
    let b = a.active_viewport();
    focus(ViewportId::ROOT);
    frames(&ctx, &mut a, 8);
    assert_eq!(a.active_viewport(), ViewportId::ROOT);
    assert!(a.canvas.focused, "the page has the keyboard");
    EVENTS.with(|e| {
        e.borrow_mut()
            .entry(ViewportId::ROOT)
            .or_default()
            .push(egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほん".into(), active_range_chars: None }))
    });
    frames(&ctx, &mut a, 2);
    assert_eq!(a.canvas.ime_preedit, "にほん");
    let other = a.windows.open.iter().find_map(|w| w.ui.as_ref());
    assert!(other.is_some_and(|w| w.canvas.ime_preedit.is_empty()), "the other window composes nothing");
    assert!(IME.with(|i| i.borrow().get(&ViewportId::ROOT).cloned().flatten()).is_some(), "the composing window places the candidate window");

    focus(b);
    frames(&ctx, &mut a, 2);
    assert!(a.canvas.ime_preedit.is_empty(), "the focused window has no composition");
    EVENTS.with(|e| e.borrow_mut().entry(ViewportId::ROOT).or_default().push(egui::Event::Ime(egui::ImeEvent::Commit("にほん".into()))));
    frames(&ctx, &mut a, 2);
    assert_eq!(text(&a), "B", "a commit in the other window doesn't land here");
}

/// Control-channel typing goes to the active window when it has a viewport of its own (the
/// root's raw-input hook doesn't see that window's input), once.
#[test]
fn control_typing_reaches_the_active_window() {
    let ctx = backend();
    let mut a = app("First");
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    let child = a.active_viewport();
    focus(child);
    frames(&ctx, &mut a, 8);
    assert!(a.canvas.focused, "the new window's page has the keyboard");
    let (req, _reply) = crate::ControlRequest::new("ui.text", json!({"text": "Typed"}));
    assert!(matches!(crate::control::handle(&mut a, &ctx, &req), crate::control::Outcome::Done(_)));
    frames(&ctx, &mut a, 4);
    assert_eq!(text(&a), "Typed");
    assert!(a.synthetic.is_empty());
    focus(ViewportId::ROOT);
    frames(&ctx, &mut a, 2);
    assert_eq!(text(&a), "First", "the other window got none of it");
}

/// `ui.screenshot` captures the active window: the capture is sized from its viewport and the
/// command goes to that viewport. A capture still pending when another window comes forward
/// fails with an error instead of capturing that window.
#[test]
fn screenshot_captures_the_active_window() {
    let ctx = backend();
    let mut a = app("First");
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    let child = a.active_viewport();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    INFOS.with(|i| i.borrow_mut().entry(child).or_default().inner_rect = Some(rect));
    focus(child);
    frames(&ctx, &mut a, 4);

    let (reply, replies) = std::sync::mpsc::channel();
    crate::control::admit_screenshot(&mut a, &ctx, None, reply);
    assert!(replies.try_recv().is_err(), "the capture is admitted");
    assert_eq!(a.shots.requests.iter().map(|s| (s.viewport, s.expected)).collect::<Vec<_>>(), [(child, [800, 600])]);
    a.shots.requests.iter_mut().for_each(|s| s.ready_at = 0.0);
    COMMANDS.with(|c| c.borrow_mut().clear());
    frames(&ctx, &mut a, 4);
    let shot_to =
        |id: ViewportId| COMMANDS.with(|c| c.borrow().get(&id).is_some_and(|v| v.iter().any(|c| matches!(c, ViewportCommand::Screenshot(_)))));
    assert!(shot_to(child), "the active window is captured");
    assert!(!shot_to(ViewportId::ROOT), "the first window is not");

    let (reply, later) = std::sync::mpsc::channel();
    crate::control::admit_screenshot(&mut a, &ctx, None, reply);
    focus(ViewportId::ROOT);
    frames(&ctx, &mut a, 2);
    for r in [replies, later] {
        let v = r.try_recv().expect("the capture was answered");
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().is_some_and(|e| e.contains("another window")), "{v}");
    }
    assert!(a.shots.requests.is_empty());
}

/// Quitting with two documents of unsaved changes asks about each in its window, the window
/// coming to the front; Cancel keeps both.
#[test]
fn quitting_asks_about_each_document_in_its_window() {
    let ctx = backend();
    let mut a = app("One");
    let one = a.session.document_id();
    frames(&ctx, &mut a, 2);
    a.run("file.new", json!({})).unwrap();
    a.run("text.insert", json!({"text": "Two"})).unwrap();
    let two = a.session.document_id();
    let child = a.active_viewport();
    frames(&ctx, &mut a, 2);

    assert!(!a.quit());
    assert_eq!((a.session.document_id(), prompt(&a)), (two, Some("saveChanges")), "the active window asks first");
    a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    assert!(!a.quit_requested);
    assert_eq!((a.window_count(), a.session.documents().len()), (2, 2), "Cancel keeps both");

    assert!(!a.quit());
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert_eq!((a.session.document_id(), prompt(&a)), (one, Some("saveChanges")), "then the other window asks");
    assert_eq!(a.window_count(), 1);
    COMMANDS.with(|c| c.borrow_mut().clear());
    frame(&ctx, &mut a);
    assert!(sent(ViewportId::ROOT, &ViewportCommand::Focus), "its window comes to the front");
    assert!(!DRAWN.with(|d| d.borrow().contains(&child)));
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
    let ctx = backend();
    let mut a = app("Hello world");
    let doc = a.session.document_id();
    frames(&ctx, &mut a, 2);
    a.run("view.newWindow", json!({})).unwrap();
    assert_eq!((a.window_count(), a.session.document_id()), (2, doc));
    frames(&ctx, &mut a, 3);
    let b = a.active_viewport();
    assert_ne!(b, ViewportId::ROOT);
    assert_eq!(window_labels(&a), ["Document1:1 •", "Document1:2 •"]);
    assert_eq!(a.window_title(), super::title("Document1:2", true));

    a.run("view.zoom", json!({"value": 150})).unwrap();
    a.run("text.insert", json!({"text": "!"})).unwrap();
    focus(ViewportId::ROOT);
    frames(&ctx, &mut a, 2);
    assert_eq!(a.active_viewport(), ViewportId::ROOT);
    assert_eq!(text(&a), "Hello world!", "the edit shows in the other window");
    assert_eq!((a.session.view.zoom, a.session.sel.focus.off), (1.0, 11), "window A keeps its zoom and caret");

    a.run("select.text", json!({"text": " world!"})).unwrap();
    a.run("text.delete", json!({})).unwrap();
    focus(b);
    frames(&ctx, &mut a, 2);
    assert_eq!(a.active_viewport(), b);
    assert!((a.session.view.zoom - 1.5).abs() < 1e-6, "window B keeps its zoom");
    a.run("text.insert", json!({"text": "?"})).unwrap();
    assert_eq!(text(&a), "Hello?", "B's caret was clamped into what is left");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(text(&a), "Hello");
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(text(&a), "Hello world!", "Undo in B undoes A's delete");
    frames(&ctx, &mut a, 2);
    assert_eq!(a.session.documents().len(), 1);
}

/// Closing one of two windows on a document asks nothing, even with unsaved changes, and the
/// other window goes back to the plain name; closing the last one asks. Quitting with a
/// document in two windows asks once.
#[test]
fn closing_one_of_two_windows_on_a_document_asks_nothing() {
    let ctx = backend();
    let mut a = app("Kept");
    let doc = a.session.document_id();
    frames(&ctx, &mut a, 2);
    a.run("view.newWindow", json!({})).unwrap();
    frames(&ctx, &mut a, 2);
    let child = a.active_viewport();
    INFOS.with(|i| i.borrow_mut().entry(child).or_default().events.push(ViewportEvent::Close));
    frame(&ctx, &mut a);
    assert_eq!(prompt(&a), None, "no prompt while another window shows the document");
    assert_eq!((a.window_count(), a.session.document_id(), text(&a).as_str()), (1, doc, "Kept"));
    assert!(a.session.dirty);
    frames(&ctx, &mut a, 2);
    assert_eq!(a.window_title(), super::title("Document1", true));
    a.run("file.close", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("saveChanges"), "the last window asks");
    a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();

    a.run("view.newWindow", json!({})).unwrap();
    frames(&ctx, &mut a, 2);
    assert_eq!(a.window_count(), 2);
    assert!(!a.quit());
    assert_eq!(prompt(&a), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(a.quit_requested, "one prompt for the document");
}

/// Two windows, the second (focused) on an untouched blank document: an app whose host
/// answers file dialogs only when the test does, plus the dialog's answer channel.
fn blank_beside_a_document() -> (egui::Context, WordApp, Shown, u64) {
    let shown: Shown = std::rc::Rc::default();
    let log = shown.clone();
    let services = Services {
        file_dialog: Some(Box::new(move |_| {
            let (tx, rx) = std::sync::mpsc::channel();
            log.borrow_mut().push(tx);
            rx
        })),
        ..Default::default()
    };
    let ctx = backend();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
    frames(&ctx, &mut a, 2);
    a.run("file.open", json!({"path": docx_on_disk("first", "First").to_string_lossy()})).unwrap();
    a.run("file.new", json!({})).unwrap();
    frames(&ctx, &mut a, 2);
    assert!(a.window_count() == 2 && a.session.is_untouched_blank());
    let blank = a.windows.shown();
    (ctx, a, shown, blank)
}

type Shown = std::rc::Rc<RefCell<Vec<std::sync::mpsc::Sender<crate::file_dialogs::FileDialogAnswer>>>>;

/// Each way of opening a file replaces an untouched blank document in its own window, even
/// when the file arrives while another window draws: File › Open, ⌘O and the File page's
/// Browse (an asynchronous dialog), the File page's and Start screen's recent list, and
/// Finder (between frames, in the focused window).
#[test]
fn every_open_replaces_an_untouched_blank_document_in_its_window() {
    let second = docx_on_disk("second", "Second");
    let path = second.to_string_lossy().to_string();
    let opens: [(&str, fn(&mut WordApp, &egui::Context, &Shown, &str)); 3] = [
        ("dialog", |a, ctx, shown, path| {
            a.run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
            a.run("ui.openFileDialog", json!({})).unwrap();
            frames(ctx, a, 2);
            let tx = shown.borrow_mut().pop().unwrap();
            tx.send(Ok(vec![path.to_string()])).unwrap();
            frames(ctx, a, 3);
        }),
        ("recent", |a, ctx, _, path| {
            crate::backstage::open_recent(a, path).unwrap();
            frames(ctx, a, 2);
        }),
        ("finder", |a, ctx, _, path| {
            a.open_file(path, None).unwrap();
            frames(ctx, a, 2);
        }),
    ];
    for (how, open) in opens {
        let (ctx, mut a, shown, blank) = blank_beside_a_document();
        open(&mut a, &ctx, &shown, &path);
        assert_eq!(a.window_count(), 2, "{how}: no third window");
        assert!(a.enter_window(blank), "{how}: the blank document's window is still open");
        assert_eq!(text(&a), "Second", "{how}: it shows the opened file");
    }
    std::fs::remove_dir_all(second.parent().unwrap()).ok();
}
