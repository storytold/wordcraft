use super::*;
use egui_kittest::kittest::{By, Queryable};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use craft::print::spool::{Pending, Spool, SpoolBackend};
use craft::print::ui::Feedback;

/// The title bar's search field.
const SEARCH_NAME: &str = "Search commands and help";
/// The title bar's community button.
const DISCORD_TITLE_NAME: &str = "Discord";
/// The community link of the File page's menu.
const DISCORD_LINK_NAME: &str = "Discord community";

/// What a job of the test spooler does when it runs.
#[derive(Clone)]
enum Reply {
    /// The job is accepted at once.
    Sent,
    /// The job fails at once with this text.
    Fail(&'static str),
    /// The job never answers while the test lives, as a printer driver that sleeps.
    Hold,
}

/// A spooler backend that records the PDFs it is given and answers as the test says, so a trial
/// drives the real `Spool` without a printer.
#[derive(Clone)]
struct Recorder {
    reply: Arc<Mutex<Reply>>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    cancels: Arc<AtomicUsize>,
    held: Arc<Mutex<Vec<mpsc::Sender<()>>>>,
    never_sent: bool,
    names: Vec<&'static str>,
}

impl Recorder {
    fn new(reply: Reply) -> Self {
        Recorder {
            reply: Arc::new(Mutex::new(reply)),
            sent: Arc::default(),
            cancels: Arc::default(),
            held: Arc::default(),
            never_sent: true,
            names: vec!["Test queue"],
        }
    }

    /// The printers the spooler lists; the first is the default.
    fn with_printers<const N: usize>(mut self, names: [&'static str; N]) -> Self {
        self.names = names.to_vec();
        self
    }

    /// A recorder whose cancel answers `never_sent`.
    fn cancelling(reply: Reply, never_sent: bool) -> Self {
        Recorder { never_sent, ..Recorder::new(reply) }
    }

    fn sent_count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    fn asked_to_cancel(&self) -> usize {
        self.cancels.load(Ordering::SeqCst)
    }

    /// A spool over this backend. A held job needs its own thread; the others run inside the call.
    fn spool(&self) -> Spool {
        let hold = matches!(*self.reply.lock().unwrap(), Reply::Hold);
        let jobs: Arc<dyn craft::host::Jobs> = if hold { Arc::new(craft::host::ThreadJobs) } else { Arc::new(craft::host::InlineJobs) };
        Spool::with_backend(jobs, Arc::new(self.clone()))
    }
}

impl SpoolBackend for Recorder {
    fn printers(&self) -> Option<Vec<craft::print::Printer>> {
        Some(
            self.names
                .iter()
                .enumerate()
                .map(|(at, name)| craft::print::Printer { name: (*name).into(), description: String::new(), default: at == 0, two_sided: None })
                .collect(),
        )
    }

    fn prepare(&self, pdf: Vec<u8>, _job: craft::print::Job) -> Pending {
        self.sent.lock().unwrap().push(pdf);
        let reply = self.reply.lock().unwrap().clone();
        let (keep, wait) = mpsc::channel::<()>();
        if matches!(reply, Reply::Hold) {
            self.held.lock().unwrap().push(keep);
        }
        let cancels = self.cancels.clone();
        let never_sent = self.never_sent;
        Pending {
            cancel: Box::new(move || {
                cancels.fetch_add(1, Ordering::SeqCst);
                never_sent
            }),
            run: Box::new(move || match reply {
                Reply::Sent => Ok(String::new()),
                Reply::Fail(text) => Err(craft::print::Error::Spool(text.into())),
                Reply::Hold => {
                    let _ = wait.recv();
                    Err(craft::print::Error::Spool("job abandoned".into()))
                }
            }),
            waits_after_cancel: false,
        }
    }
}

/// Lets the spooler's printers arrive, as the Print page's first drawing does: a spool that
/// chooses its printer refuses `ui.print` until one is discovered and selected.
fn discover_printers(a: &mut WordApp) {
    let ctx = crate::test_harness::context();
    a.run("ui.printSettings", json!({"refresh": true})).unwrap();
    for _ in 0..40 {
        a.poll_printing(&ctx);
        if a.printing.ready() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Discovery runs on native hosts only: where the test host isn't one, the app has found the
    // recorder's printer and chosen it, as `native_print.rs` sets its own up.
    let name = if a.printing.settings.printer.is_empty() { "Test queue".to_string() } else { a.printing.settings.printer.clone() };
    if !a.printing.printers.iter().any(|p| p.name == name) {
        a.printing.printers.push(crate::printing::Printer::new(name.clone()).is_default(true));
    }
    a.run("ui.printSettings", json!({"printer": name})).unwrap();
}

/// The status line's text.
fn said(a: &WordApp) -> String {
    a.status.as_ref().map(|s| s.text.clone()).unwrap_or_default()
}

/// Whether the command is enabled, by the state the command board resolves.
fn enabled(ctx: &egui::Context, id: &str) -> bool {
    craft::ui::runtime::commands::resolve(ctx, id).enabled
}

/// The event of a pressed chord, as the keyboard would send it.
fn chord_event(chord: &craft::keymap::Chord) -> Option<egui::Event> {
    let key = egui::Key::from_name(&chord.key.to_string())?;
    let mac = cfg!(target_os = "macos");
    let m = chord.mods;
    let modifiers = egui::Modifiers { alt: m.alt, shift: m.shift, ctrl: m.ctrl || (m.cmd && !mac), mac_cmd: m.cmd && mac, command: m.cmd };
    Some(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers })
}

fn shows_tabs(h: &egui_kittest::Harness<'_, WordApp>) -> bool {
    h.query_by_label("Insert").is_some()
}

/// The text the title bar's search field holds, or `None` while no field shows.
fn search_text(h: &egui_kittest::Harness<'_, WordApp>) -> Option<String> {
    h.query(By::new().role(egui::accesskit::Role::TextInput).label(SEARCH_NAME)).map(|n| n.value().map(|v| v.to_string()).unwrap_or_default())
}

/// #1678: Focus hides the title bar and ribbon; the screen bar's button drops the ribbon over the
/// page without leaving Focus, and Escape leaves Focus (and Full Screen View).
#[test]
fn focus_hides_the_chrome_drops_the_ribbon_and_esc_leaves() {
    let mut a = app();
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(shows_tabs(&h), "the ribbon's tabs show in the usual window");
    assert!(h.query_by_label(SEARCH_NAME).is_some(), "with the title bar's search");
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode, "view.focus turns Focus on");
    assert!(!shows_tabs(&h), "Focus hides the ribbon");
    assert!(h.query_by_label(SEARCH_NAME).is_none(), "and the title bar's search");
    // The screen bar's button.
    h.state_mut().run("ui.focusRibbon", json!({})).unwrap();
    h.run_steps(3);
    assert!(shows_tabs(&h), "the ribbon drops down");
    assert!(h.state().session.view.focus_mode, "still in Focus");
    // A click on the dropped ribbon works it and leaves it down; one on the page puts it away.
    h.get_by_label("Insert").click();
    h.run_steps(3);
    assert!(h.state().ribbon_dropped && h.state().ui.tab == "Insert", "a click on the ribbon keeps it down");
    // The dropped ribbon lies over the page but is chrome: its click hands the keyboard back
    // to the page, as a ribbon click does in the usual window (#1093).
    assert!(h.state().canvas.focused(&h.ctx), "a click on the dropped ribbon gives the page the keyboard");
    h.event(egui::Event::Text("x".into()));
    h.run_steps(2);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Hellox"), "typing after it reaches the page");
    assert!(h.state().ribbon_dropped, "typing leaves the dropped ribbon down");
    let page = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 600.0).unwrap();
    h.event(egui::Event::PointerMoved(page));
    h.event(egui::Event::PointerButton { pos: page, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: page, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert!(!h.state().ribbon_dropped && h.state().session.view.focus_mode, "a click on the page puts it away, still in Focus");
    h.state_mut().run("ui.focusRibbon", json!({})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.focus_mode && !h.state().ribbon_dropped, "Escape leaves Focus");
    h.state_mut().run("view.fullScreenView", json!({})).unwrap();
    h.run_steps(3);
    assert!(!shows_tabs(&h), "Full Screen View shows the pages alone");
    assert!(h.query_by_label(SEARCH_NAME).is_some(), "with Search at the top");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.full_screen_view, "Escape leaves Full Screen View");
    // Full Screen (#980, F11) is the whole window, ribbon and all.
    h.state_mut().run("view.fullScreen", json!({})).unwrap();
    h.run_steps(3);
    assert!(shows_tabs(&h), "Full Screen keeps the ribbon");
}

/// #1678: Focus's dropped ribbon is a command, `ui.focusRibbon`, which the screen bar's button and a
/// click on the page run; outside Focus it does nothing and never fails.
#[test]
fn focus_ribbon_is_a_command() {
    let mut a = app();
    let off = json!({"focusRibbon": false});
    assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), off, "outside Focus there is no ribbon to drop");
    assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap(), off);
    assert!(!a.ribbon_dropped);
    a.run("view.focus", json!({})).unwrap();
    assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), json!({"focusRibbon": true}), "no value toggles it");
    assert!(a.ribbon_dropped && a.session.view.focus_mode);
    assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), off);
    assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap()["focusRibbon"], true);
    assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap()["focusRibbon"], true, "true keeps it down");
    for junk in [json!({"value": "yes"}), json!({"value": 1}), json!({"value": null}), json!({"value": [true]})] {
        assert!(a.run("ui.focusRibbon", junk).is_err());
    }
    assert!(a.ribbon_dropped, "a refused value leaves it as it was");
    assert_eq!(a.run("ui.focusRibbon", json!({"value": false})).unwrap(), off);
    assert!(a.session.view.focus_mode, "putting the ribbon away stays in Focus");
}

/// #1678: one Escape path. In Focus with text selected on the page, the first Escape only
/// collapses the selection, as in a normal window, and the second leaves Focus. A handler that
/// takes Escape for Focus before the page sees it would leave Focus with the selection kept.
#[test]
fn focus_esc_collapses_the_selection_first_then_leaves() {
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().run("select.all", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.state().canvas.focused(&h.ctx) && h.state().session.view.focus_mode);
    assert!(!h.state().session.sel.is_collapsed(), "text is selected");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().session.sel.is_collapsed(), "the first Escape collapses the selection");
    assert!(h.state().session.view.focus_mode, "and keeps Focus");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.focus_mode, "the second Escape leaves Focus");
}

/// The Options toggle hides the community pill in the title bar; the backstage menu's
/// Discord link hides with it — one setting, everywhere the link shows.
///
/// The File page fills the window in place of the title bar (`AppShell::page`, ch.C5), so the title bar's button is
/// read in the editor window and the link on the File page.
#[test]
fn the_community_button_off_hides_the_backstage_discord_link_too() {
    for show in [true, false] {
        let mut a = app();
        a.ui.show_discord = show;
        let mut h = harness(a);
        h.run_steps(2);
        assert_eq!(h.query_by_label(DISCORD_TITLE_NAME).is_some(), show, "show {show}: the title bar's community button follows the setting");
        h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
        h.run_steps(2);
        assert_eq!(h.query_by_label(DISCORD_LINK_NAME).is_some(), show, "show {show}: the File page's link honors the same setting");
    }
}

/// #766: started with nothing to open, the app shows File › Home (the Start screen) as Word
/// does. #1129: no document is open behind it, so Esc (or Back) shows an empty window, with
/// no page, the ribbon greyed out but for Help, and nothing to save on close. With the setting
/// off it starts in the editor, and `WordApp::new` alone (tests, ui_shot, windows opened for a
/// document) never shows it.
#[test]
fn start_screen_shows_file_home_and_esc_leaves_no_document() {
    let mut a = app();
    assert!(!a.ui.backstage, "`new` starts in the editor");
    assert!(a.has_document());
    a.ui.backstage_page = "print".into();
    assert!(a.start_screen(), "on by default");
    assert!(a.ui.backstage && a.ui.backstage_page == "home");
    assert!(!a.has_document(), "no document behind the Start screen");
    let mut h = harness(a);
    h.run_steps(4);
    assert!(h.query_by_label("Blank document").is_some(), "Home lists Blank document and the templates");
    // The Recent heading and the Recent side-menu entry both carry the label.
    assert!(h.query_all_by_label("Recent").next().is_some(), "and the Recent list");
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    let ctx = h.ctx.clone();
    let a = h.state_mut();
    assert!(!a.ui.backstage, "Esc leaves the Start screen");
    assert!(!a.has_document(), "for an empty window, not a blank document");
    assert_eq!(a.window_title(), "WordCraft", "the window is titled with the app alone");
    // The ribbon's commands are off, but Help, the File tab and the settings work.
    for id in ["format.bold", "text.insert", "file.save", "file.saveAs", "file.print", "edit.paste", "insert.table", "ui.print"] {
        assert!(!enabled(&ctx, id), "{id} is off with no document");
    }
    for id in ["file.new", "file.open", "ui.backstage", "ui.tab", "ui.discord", "file.options", "ui.theme", "ui.statusCount"] {
        assert!(enabled(&ctx, id), "{id} works with no document");
    }
    assert!(!crate::chrome::qat_enabled(&ctx, a, "file.save"));
    // Only the listed interface commands work: the ones that act on the document don't.
    for id in ["ui.contextMenu", "ui.render", "ui.linkTextBox", "ui.zotero.refresh", "ui.unlistedCommand"] {
        assert!(!works_without_document(id), "{id} needs a document");
    }
    let refused = a.run("ui.zotero.status", json!({})).unwrap_err();
    assert!(refused.contains("No document is open"), "refused before it runs: {refused}");
    assert!(works_without_document("font.add"), "a font file dropped on the empty window is added");
    // #4585: File › Options › Proofing › AutoCorrect Options edits a user setting, not the document.
    a.run("tools.autocorrect", json!({})).unwrap();
    // PLAN: assumed the AutoCorrect dialog's host key is "autoCorrect" (dialogs sets)
    assert!(a.dialog_host.open_dialogs().iter().any(|d| d.key.as_deref() == Some("autoCorrect")));
    // PLAN: assumed `dialogs::close_all(app: &mut WordApp, ctx: &egui::Context)` (dialogs-w13)
    crate::dialogs::close_all(a, &ctx);
    // Neither keys nor commands reach the hidden placeholder.
    assert!(a.run("text.insert", json!({"text": "Hello"})).is_err());
    assert!(a.execute("format.bold", json!({})).is_err());
    h.event(egui::Event::Text("typed".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let a = h.state_mut();
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "");
    assert!(!a.session.dirty && !a.has_document());
    assert!(a.close_requested(), "closing asks nothing");
    assert!(a.dialog_host.open_dialogs().is_empty());

    let mut b = app();
    b.run("ui.startScreen", json!({"value": false})).unwrap();
    assert!(!b.start_screen(), "off: today's behaviour");
    assert!(!b.ui.backstage && b.has_document());
}

/// #1129: the empty window's File tab goes back to the Backstage with only Home, New, Open
/// and Options; picking Blank document there opens one, ready for typing.
#[test]
fn blank_document_opens_a_document_from_the_start_screen() {
    let mut a = app();
    a.start_screen();
    a.run("ui.backstage", json!({"value": false})).unwrap();
    a.run("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
    let first = a.session.document_id();
    let mut h = harness(a);
    h.run_steps(4);
    assert_eq!(h.state().ui.backstage_page, "home", "Info needs a document");
    h.get_by_label("Blank document").click();
    h.run_steps(4);
    let ctx = h.ctx.clone();
    let a = h.state_mut();
    assert!(a.has_document() && !a.ui.backstage, "Blank document opens one");
    assert_ne!(a.session.document_id(), first);
    assert!(enabled(&ctx, "format.bold"));
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Hello");
    let title = a.window_title();
    assert!(title.starts_with(&a.title_stem()), "{title}");
}

/// #1129: a template or a file opened from the Start screen (or handed over later) is the
/// document; a failed open leaves the window empty.
#[test]
fn templates_and_files_open_from_the_start_screen() {
    let mut a = app();
    a.start_screen();
    a.run("file.new", json!({"template": "letter"})).unwrap();
    assert!(a.has_document());

    let dir = scratch("start-screen-open");
    let path = docx_on_disk(&dir, "Signed copy");
    let mut b = app();
    b.start_screen();
    assert!(b.run("file.open", json!({"path": dir.join("missing.docx").to_string_lossy()})).is_err());
    assert!(!b.has_document(), "a failed open opens nothing");
    b.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(b.has_document());
    assert!(body_text(&b).contains("Signed copy"));
    let _ = std::fs::remove_dir_all(dir);
}

/// `ui.print` hands the document's PDF to the host's spool, and is an error (with nothing
/// printed) when the host has none; a failure of the job reaches the status.
#[test]
fn ui_print_needs_the_hosts_print_hook() {
    let mut a = typed();
    assert!(a.run("ui.print", json!({})).is_err(), "no spool: an error");

    let rec = Recorder::new(Reply::Sent);
    a.services.print = Some(rec.spool());
    discover_printers(&mut a);
    let r = a.run("ui.print", json!({})).unwrap();
    assert_eq!(r["printing"], true);
    assert_eq!(rec.sent_count(), 1, "printed once");
    assert!(rec.sent.lock().unwrap().first().is_some_and(|b| b.starts_with(b"%PDF")), "the backend got a PDF");
    assert!(a.session.dirty, "printing is not a save");

    let mut b = typed();
    b.services.print = Some(Recorder::new(Reply::Fail("blocked by the browser")).spool());
    discover_printers(&mut b);
    b.run("ui.print", json!({})).unwrap();
    let mut h = harness(b);
    h.run_steps(4);
    assert!(said(h.state()).contains("blocked by the browser"), "the status bar says why: {}", said(h.state()));
}

/// The print settings reach the spool: bad ones print nothing, a valid range submits one job.
#[test]
fn ui_print_honours_the_print_settings() {
    let mut a = typed();
    let rec = Recorder::new(Reply::Sent);
    a.services.print = Some(rec.spool());
    discover_printers(&mut a);
    assert!(a.run("ui.print", json!({"pages": "7-9"})).is_err(), "page 7 doesn't exist");
    assert!(a.run("ui.print", json!({"range": "nope"})).is_err());
    assert_eq!(rec.sent_count(), 0, "nothing was sent");
    a.run("ui.print", json!({"range": "current", "copies": 3})).unwrap();
    assert_eq!(rec.sent_count(), 1);
}

/// The Print page shows the Print button only when the host can print, and the button
/// prints through `ui.print`.
#[test]
fn the_print_button_shows_only_with_a_print_hook() {
    let mut buttons = [0usize; 2];
    for (at, spool) in [false, true].into_iter().enumerate() {
        let rec = Recorder::new(Reply::Sent);
        let mut a = app();
        if spool {
            a.services.print = Some(rec.spool());
        }
        a.run("file.print", json!({})).unwrap();
        let mut h = harness(a);
        h.run_steps(4);
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "print", "the Print page is open");
        assert!(h.query_by_label("PDF document (*.pdf)").is_some(), "spool {spool}: the page rendered its export choices");
        let print = || By::new().role(egui::accesskit::Role::Button).label("Print");
        buttons[at] = h.query_all(print()).count();
        if spool {
            // The page's own Print button (the backstage list has a Print entry too): the last.
            let all: Vec<_> = h.query_all(print()).collect();
            all.last().expect("a Print button").click();
            h.run_steps(2);
            assert_eq!(rec.sent_count(), 1, "the button printed");
        }
    }
    assert!(buttons[1] > buttons[0], "the Print button shows only with a spool: {buttons:?}");
}

/// A Print page with printers found (so it draws the job message), in English.
fn print_page_harness(message: Option<Feedback>, rec: &Recorder) -> egui_kittest::Harness<'static, WordApp> {
    let mut a = app();
    a.ui.language = "en".into();
    a.services.print = Some(rec.spool());
    let mut h = harness(a);
    // The page's first drawing starts discovery, which replaces the message: draw it once, leave,
    // and let the printers arrive.
    let _ = h.state_mut().run("file.print", json!({}));
    h.run_steps(4);
    let _ = h.state_mut().run("ui.backstage", json!({"value": false}));
    h.run_steps(3);
    set_print_message(&mut h, message);
    h.state_mut().print.said = None;
    h.state_mut().print.hidden = None;
    h
}

/// A job message that arrives without the app's own `print_feedback` acting on it (a submitted job
/// would close the Print page); what the window has said is set by the test.
fn set_print_message(h: &mut egui_kittest::Harness<'_, WordApp>, message: Option<Feedback>) {
    h.state_mut().printing.message = message.clone();
    h.state_mut().print_feedback_seen = message;
    h.state_mut().print_gen += 1;
}

fn leave_and_reopen_print(h: &mut egui_kittest::Harness<'_, WordApp>) {
    let _ = h.state_mut().run("ui.backstage", json!({"value": false}));
    h.run_steps(2);
    let _ = h.state_mut().run("file.print", json!({}));
    h.run_steps(3);
}

/// Opened again, the Print page doesn't repeat how the last job went when the window already said
/// it (in its status bar), but a failure that comes in while it is open stays, and what the
/// printers are doing (none found) shows each time. The shared message is never cleared, so
/// `ui.printSettings` still reports it.
#[test]
fn the_print_page_opens_without_the_last_jobs_message() {
    let mut h = print_page_harness(Some(Feedback::Submitted("job 7".into())), &Recorder::new(Reply::Sent));
    // A job went away earlier: its message was said in this window's status bar, and the page closed.
    h.state_mut().print.said = Some(h.state().print_gen);
    let _ = h.state_mut().run("file.print", json!({}));
    h.run_steps(4);
    assert!(h.query_by_label_contains("job 7").is_none(), "the last job's message isn't repeated");
    assert_eq!(h.state().printing.message, Some(Feedback::Submitted("job 7".into())), "but the shared message stays");
    // A failure arrives while the page is open, said (by the status bar) in the very frame, so
    // only the page being open keeps it showing: wrong reopen detection would hide it.
    let failure = Some(Feedback::Failed("no paper".into()));
    set_print_message(&mut h, failure.clone());
    h.state_mut().print.said = Some(h.state().print_gen);
    h.run_steps(3);
    assert!(h.query_by_label_contains("no paper").is_some(), "a failure on the open page stays");
    leave_and_reopen_print(&mut h);
    assert!(h.query_by_label_contains("no paper").is_none(), "reopened, the failure isn't repeated");
    assert_eq!(h.state().printing.message, failure, "but it is still the shared message");
    set_print_message(&mut h, Some(Feedback::NoPrinters));
    leave_and_reopen_print(&mut h);
    assert!(h.query_by_label_contains("No printers found").is_some(), "no printers still shows");
}

/// A retry that fails again with the very same text is a new result: it shows, though the window
/// said the same words for the job before.
#[test]
fn a_retry_that_fails_the_same_way_shows_again() {
    // Every job is refused at once, with the same words.
    let mut h = print_page_harness(None, &Recorder::new(Reply::Fail("no paper")));
    let _ = h.state_mut().run("file.print", json!({}));
    h.run_steps(3);
    h.state_mut().run("ui.print", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().printing.message, Some(Feedback::Failed("no paper".into())));
    assert!(h.query_by_label_contains("no paper").is_some(), "the first failure shows");
    leave_and_reopen_print(&mut h);
    assert!(h.query_by_label_contains("no paper").is_none(), "reopened, it isn't repeated");
    h.state_mut().run("ui.print", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().printing.message, Some(Feedback::Failed("no paper".into())));
    assert!(h.query_by_label_contains("no paper").is_some(), "the same failure of a new job shows");
}

/// A result nobody has said yet isn't dropped: a failure that arrived while File showed another
/// page (so no status bar was drawn) shows when Print opens.
#[test]
fn the_print_page_shows_a_result_that_was_never_said() {
    let mut h = print_page_harness(None, &Recorder::new(Reply::Sent));
    h.state_mut().ui.backstage = true;
    h.state_mut().ui.backstage_page = "home".into();
    h.run_steps(2);
    // Through the app's own `print_feedback` (run each frame), which says it in a status bar that isn't drawn.
    h.state_mut().printing.message = Some(Feedback::Failed("no paper".into()));
    h.run_steps(3);
    assert_eq!(h.state().print.said, None, "File's home page doesn't say it");
    let _ = h.state_mut().run("file.print", json!({}));
    h.run_steps(4);
    assert!(h.query_by_label_contains("no paper").is_some());
}

/// Per window: what one window has said of the last job isn't another window's, and the shared
/// message is the same in both. (The harness draws one viewport, which re-enters a window of its
/// own, so the per-window state is checked where windows swap; `the_print_page_opens_without_the_last_jobs_message`
/// and `the_print_page_shows_a_result_that_was_never_said` show what each state does on the page.)
#[test]
fn the_print_pages_of_two_windows_decide_separately() {
    let failure = Some(Feedback::Failed("no paper".into()));
    let mut a = app();
    a.printing.message = failure.clone();
    let first = a.windows.shown();
    a.print_gen = 1;
    a.print.said = Some(a.print_gen);
    a.new_window();
    assert_ne!(first, a.windows.shown());
    assert_eq!(a.print.said, None, "the second window never said it, so its page shows it");
    assert_eq!(a.printing.message, failure, "the message is shared, and still there");
    assert!(a.enter_window(first));
    assert_eq!(a.print.said, Some(1), "the first window said it, so its page doesn't repeat it");
    assert_eq!(a.printing.message, failure);
}

/// Return on the Print page prints, as in Word, where ⌘P opens on Print as the default
/// button; the document isn't typed into.
#[test]
fn return_on_the_print_page_prints() {
    // A failing job keeps the page open, so Return stays the page's key.
    let rec = Recorder::new(Reply::Fail("no paper"));
    let mut a = app().with_fallback_chooser(true);
    a.services.print = Some(rec.spool());
    discover_printers(&mut a);
    let before = body_text(&a);
    a.run("file.print", json!({})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert_eq!(rec.sent_count(), 0);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(rec.sent_count(), 1, "Return printed once");
    assert_eq!(body_text(h.state()), before, "the document is unchanged");
    // Held down, Return repeats (egui marks a press of a key already down as a repeat): one job.
    // The platform marks every press after the first of a held key as a repeat (`Event::Key::repeat`); the first
    // press here is the second job's, so it is not one.
    let enter = |pressed, repeat| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat, modifiers: egui::Modifiers::NONE };
    for n in 0..4 {
        h.event(enter(true, n > 0));
        h.run_steps(1);
    }
    h.event(enter(false, false));
    h.run_steps(1);
    assert_eq!(rec.sent_count(), 2, "a held Return prints once");
    // The fallback file chooser over the page takes Return for itself, even with nothing in it
    // focused: Return there saves or opens, and prints nothing.
    // The chooser is the desktop file service's (S-L2): the binary's `FilePanels::pump_with` opens it on the app's
    // dialog host for a `ui.fileBrowser` request, and this crate has no window to pump it with. It is opened on the
    // host here as that pump does, over a fake folder.
    let folders = craft::browse::places::FolderCache::new(
        Arc::new(craft::browse::places::FakeDirectories::new().folder("/d/drafts").file("/d/report.docx")),
        Arc::new(craft::host::ManualJobs::new()),
        craft::host::Wake::none(),
    );
    let request = craft::host::PickRequest::open("document").directory("/d").fallback(true);
    h.state_mut().dialog_host.open(craft::dialogs::ui::FileChooser::new(request, None, folders));
    h.run_steps(4);
    assert!(h.state().dialog_host.is_open::<craft::dialogs::ui::FileChooser>(), "the chooser opened");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(rec.sent_count(), 2, "Return in the chooser prints nothing");
}

/// #3210: a successful native print submission returns to the document, the same as Word —
/// previously the Print page stayed up until the user clicked Back, even though the job had
/// already gone to the spooler. A failed submission stays up instead, so the error (and the
/// settings to fix it) are still visible.
#[test]
fn a_successful_native_print_returns_to_the_document() {
    for ok in [true, false] {
        let mut a = app();
        a.printing.settings.printer = "Test Printer".into();
        a.services.print = Some(Recorder::new(if ok { Reply::Sent } else { Reply::Fail("no paper") }).with_printers(["Test Printer"]).spool());
        discover_printers(&mut a);
        a.run("file.print", json!({})).unwrap();
        let mut h = harness(a);
        h.run_steps(3);
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "print", "the Print page is open");
        assert!(!h.state().canvas.focused(&h.ctx), "the Print page has the keyboard");
        h.state_mut().run("ui.print", json!({})).unwrap();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage, !ok, "ok {ok}: whether the Print page is still open");
        assert_eq!(h.state().canvas.focused(&h.ctx), ok, "ok {ok}: the document takes keyboard focus back only once the Print page closes");
        if !ok {
            let a = h.state();
            assert_eq!(a.ui.backstage_page, "print", "the Print page is still the one showing, not some other backstage page");
            assert!(
                matches!(&a.printing.message, Some(Feedback::Failed(error)) if error == "no paper"),
                "the failure is still visible: {:?}",
                a.printing.message
            );
        }
    }
}

/// #5710: a job waiting on a sleeping network printer's driver locked the Print page until the
/// driver answered. `ui.printCancel` stops it before it is sent and frees the page at once,
/// keeping the chosen printer; a job already being sent can't be stopped, and the status says so.
#[test]
fn cancelling_a_waiting_print_job_frees_the_print_page() {
    let mut texts: [Option<String>; 2] = [None, None];
    for (never_sent, close_document) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut a = app();
        a.printing.settings.printer = "Office".into();
        a.printing.settings.copies = 3;
        // The worker never answers, as when the driver doesn't.
        let rec = Recorder::cancelling(Reply::Hold, never_sent).with_printers(["Office"]);
        a.services.print = Some(rec.spool());
        discover_printers(&mut a);
        for junk in [json!({}), json!(null), json!("all"), json!([1, 2]), json!({"cancel": -1, "job": "x"})] {
            assert!(a.run("ui.printCancel", junk).is_err(), "nothing to cancel before a job is submitted");
        }
        a.run("ui.print", json!({})).unwrap();
        assert!(a.printing.busy(), "the job waits");
        if close_document {
            a.execute("file.close", json!({})).unwrap();
            assert!(!a.has_document(), "the last document closed");
            assert!(a.printing.busy(), "closing the document keeps its pending print job");
        }
        let offered = |a: &WordApp| crate::printing::commands(a).iter().any(|c| c["id"] == "ui.printCancel" && c["enabled"] == true);
        assert!(offered(&a), "Cancel is offered while the job waits");
        let reply = a.execute("ui.printCancel", json!({})).unwrap();
        assert_eq!(reply["cancelled"], never_sent);
        assert_eq!(rec.asked_to_cancel(), 1, "the backend was asked to stop the job");
        assert_eq!(a.printing.busy(), !never_sent, "never_sent {never_sent}: whether the page is still locked");
        assert_eq!(a.printing.settings.printer, "Office", "the chosen printer is kept");
        assert_eq!(a.printing.settings.copies, 3, "the settings are kept");
        assert_eq!(a.printing.printers.len(), 1, "the printer list is kept");
        let text = said(&a);
        assert!(!text.is_empty(), "never_sent {never_sent}: the status bar says what happened");
        let slot = &mut texts[usize::from(never_sent)];
        match slot {
            Some(first) => assert_eq!(*first, text, "the same outcome is said the same way"),
            None => *slot = Some(text),
        }
        assert_eq!(offered(&a), !never_sent);
        if never_sent {
            assert!(a.execute("ui.printCancel", json!({})).is_err(), "no job left to cancel");
        }
        assert_eq!(rec.sent_count(), 1);
    }
    assert_ne!(texts[0], texts[1], "a stopped job and one that could not be stopped read differently");
}

/// #819: Alt+Q opens Search with its box focused, as in Word: the Q isn't typed into the
/// document, and what is typed next goes into the search box.
#[test]
fn alt_q_opens_search_and_typing_goes_there() {
    let mut h = harness(app());
    h.run_steps(6);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    h.event(egui::Event::Text("Kilns".into()));
    h.step();
    // Some platforms send the key's text with an Alt shortcut; it isn't typed.
    // The chord the keymap gives Search on this platform; macOS has none (Option+Q types a character).
    // Off macOS the chord is the interface's Search Commands (`app/command_board.rs`), which the engine's own
    // `tools.search` binding gives way to: it opens Search in the title bar's box where the window shows one.
    let search = craft::ui::runtime::commands::CommandRef::with_params("ui.dialog", json!({"name": "commands"}));
    let Some(chord) = craft::ui::runtime::commands::chord(&h.ctx, &search) else {
        #[cfg(not(target_os = "macos"))]
        panic!("Search has a chord off macOS");
        #[cfg(target_os = "macos")]
        return;
    };
    let Some(open) = chord_event(&chord) else {
        panic!("the chord {chord} names a key egui knows");
    };
    // Both in one frame's input, as the platform sends them: kittest's `event` runs a frame for each queued event.
    h.input_mut().events.extend([open.clone(), egui::Event::Text("q".into())]);
    h.step();
    // The chord fires the command in that pass; the app runs it when the next pass begins (`driver_logic`).
    h.step();
    // The title bar shows the Search box, so Search opens there, in place (#5398).
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "no Search Commands dialog over the page");
    assert!(crate::chrome::search::is_open(h.state()), "Search is open");
    h.step();
    assert_eq!(search_text(&h).as_deref(), Some(""), "the field is empty");
    assert!(h.query_by_label_contains("Word Count").is_some(), "an empty box suggests commands, as Word's does");
    h.event(egui::Event::Text("bold".into()));
    h.step();
    assert_eq!(search_text(&h).as_deref(), Some("bold"), "typing goes into the search box");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns");
    h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
    h.step();
    h.step();
    // The framework gives Escape to a field holding a draft first (`overlay/dismissal.rs`): the typed "bold" takes the
    // first one, and the second closes Search. The old app closed on one.
    h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
    h.step();
    h.step();
    assert!(!crate::chrome::search::is_open(h.state()), "Escape closes Search");
    assert!(!h.state().session.view.full_screen_view);
    // Words that name no command: Enter finds them in the document, in the Navigation pane.
    h.event(open);
    h.step();
    h.step();
    // Typing sends each key's press before its text.
    h.event(key(egui::Key::K, egui::Modifiers::SHIFT));
    h.event(egui::Event::Text("Kilns".into()));
    h.step();
    h.event(key(egui::Key::Enter, egui::Modifiers::NONE));
    h.step();
    h.step();
    assert!(!crate::chrome::search::is_open(h.state()), "Enter ran the search");
    assert!(h.state().session.view.nav_pane, "the Navigation pane shows the results");
    assert_eq!(h.state().session.find.results.len(), 1);
}

/// #5398 (review): Focus hides the Search box, so Search there is the dialog, even on the frame
/// Focus comes on; and Search left open when Focus comes on doesn't keep Escape from leaving it.
#[test]
fn focus_mode_search_never_opens_in_the_hidden_box() {
    let mut h = harness(app());
    h.run_steps(6);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    // Search, as Alt+Q opens it (Windows and Linux; macOS has no Alt+Q, Option+Q types a character).
    let search = |h: &mut egui_kittest::Harness<WordApp>| {
        let _ = h.state_mut().run("ui.dialog", json!({"name": "commands"}));
    };
    // The last frame drew the box; Focus comes on just before Search.
    h.state_mut().run("view.focus", json!({"value": true})).unwrap();
    search(&mut h);
    h.step();
    assert!(!crate::chrome::search::is_open(h.state()), "no Search in a box Focus hides");
    assert!(!h.state().dialog_host.open_dialogs().is_empty(), "Search opens as the dialog");
    let ctx = h.ctx.clone();
    // PLAN: assumed `dialogs::close_all(app: &mut WordApp, ctx: &egui::Context)` (dialogs-w13)
    crate::dialogs::close_all(h.state_mut(), &ctx);
    h.step();
    h.state_mut().run("view.focus", json!({"value": false})).unwrap();
    h.run_steps(3);
    // Search open in the title bar, then Focus comes on: Escape leaves Focus.
    search(&mut h);
    h.run_steps(2);
    assert!(crate::chrome::search::is_open(h.state()));
    h.state_mut().run("view.focus", json!({"value": true})).unwrap();
    h.run_steps(3);
    assert!(!crate::chrome::search::is_open(h.state()), "Search whose box is gone is dropped");
    h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
    h.run_steps(2);
    assert!(!h.state().session.view.focus_mode, "Escape leaves Focus");
}

/// #819: `tools.search` with a query lists matching commands for scripts and agents; every
/// word has to match, so "insert table" finds the command that inserts a table.
#[test]
fn search_command_matches_every_word() {
    let mut a = app();
    let r = a.run("tools.search", json!({"query": "insert table"})).unwrap();
    let ids: Vec<&str> = r["matches"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert!(ids.contains(&"insert.table"), "{ids:?}");
    assert!(r["matches"].as_array().unwrap().iter().all(|m| {
        let hay = format!("{} {} {}", m["label"], m["location"], m["id"]).to_lowercase();
        hay.contains("insert") && hay.contains("table")
    }));
    assert_eq!(a.run("tools.search", json!({"query": "   "})).unwrap()["matches"], json!([]));
    assert_eq!(a.run("tools.search", json!({"query": "zzqqxx"})).unwrap()["matches"], json!([]));
    assert!(a.dialog_host.open_dialogs().is_empty(), "a query never opens the box");
    a.run("tools.search", json!({})).unwrap();
    // The title bar's in-place field takes the request (#5398), where there is a box to show it; no dialog opens.
    assert!(crate::chrome::search::is_open(&a), "no query opens Search");
}

/// Issue #2716: taking documents off the list needs no open document — the Start screen shows
/// the recent list, and its buttons must work from there.
#[test]
fn recent_list_edits_work_without_a_document() {
    let mut a = app();
    a.ui.recent = vec!["/tmp/alpha.docx".into()];
    assert!(!a.session.dirty && a.session.path.is_none());
    assert!(works_without_document("ui.recentRemove") && works_without_document("ui.recentClear"));
    assert_eq!(a.run("ui.recentRemove", json!({"path": "/tmp/alpha.docx"})).unwrap()["removed"], 1);
    assert_eq!(a.run("ui.recentClear", json!({})).unwrap()["removed"], 0);
}
