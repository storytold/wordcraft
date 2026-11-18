use super::*;

/// #1678: Focus hides the title bar, ribbon and status bar; "⋯" drops the ribbon over the
/// page without leaving Focus, and Escape leaves Focus (and Full Screen View).
#[test]
fn focus_hides_the_chrome_drops_the_ribbon_and_esc_leaves() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(h.query_by_label("Insert").is_some(), "the ribbon's tabs show in the usual window");
    // Focus on the status bar, as a user turns it on.
    h.get_by_label("Focus").click();
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode, "the status bar's Focus turns Focus on");
    assert!(h.query_by_label("Insert").is_none(), "Focus hides the ribbon");
    assert!(h.query_by_label("Search commands and help").is_none(), "and the title bar");
    // "⋯" in the Focus bar, at the top centre.
    h.get_by_label("Show the ribbon").click();
    h.run_steps(3);
    assert!(h.query_by_label("Insert").is_some(), "the ribbon drops down");
    assert!(h.state().session.view.focus_mode, "still in Focus");
    // A click on the dropped ribbon works it and leaves it down; one on the page puts it away.
    h.get_by_label("Insert").click();
    h.run_steps(3);
    assert!(h.state().focus_ribbon && h.state().ui.tab == "Insert", "a click on the ribbon keeps it down");
    // The dropped ribbon lies over the page but is chrome: its click hands the keyboard back
    // to the page, as a ribbon click does in the usual window (#1093).
    assert!(h.state().canvas.focused, "a click on the dropped ribbon gives the page the keyboard");
    h.event(egui::Event::Text("x".into()));
    h.run_steps(2);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Hellox"), "typing after it reaches the page");
    assert!(h.state().focus_ribbon, "typing leaves the dropped ribbon down");
    let page = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 600.0).unwrap();
    h.event(egui::Event::PointerMoved(page));
    h.event(egui::Event::PointerButton { pos: page, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: page, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert!(!h.state().focus_ribbon && h.state().session.view.focus_mode, "a click on the page puts it away, still in Focus");
    h.get_by_label("Show the ribbon").click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    let a = h.state_mut();
    assert!(!a.session.view.focus_mode && !a.focus_ribbon, "Escape leaves Focus");
    a.run("view.fullScreenView", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Insert").is_none(), "Full Screen View shows the pages alone");
    assert!(h.query_by_label("Search commands and help").is_some(), "with Search at the top");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.full_screen_view, "Escape leaves Full Screen View");
    // Full Screen (#980, F11) is the whole window, ribbon and all.
    h.state_mut().run("view.fullScreen", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Insert").is_some(), "Full Screen keeps the ribbon");
}

/// #1678: Focus's dropped ribbon is a command, `ui.focusRibbon`, which "⋯" and a click on the
/// page run; outside Focus it does nothing and never fails.
#[test]
fn focus_ribbon_is_a_command() {
    let mut a = app();
    let off = json!({"focusRibbon": false});
    assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), off, "outside Focus there is no ribbon to drop");
    assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap(), off);
    assert!(!a.focus_ribbon);
    a.run("view.focus", json!({})).unwrap();
    assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), json!({"focusRibbon": true}), "no value toggles it");
    assert!(a.focus_ribbon && a.session.view.focus_mode);
    assert_eq!(a.run("ui.focusRibbon", json!({})).unwrap(), off);
    assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap()["focusRibbon"], true);
    assert_eq!(a.run("ui.focusRibbon", json!({"value": true})).unwrap()["focusRibbon"], true, "true keeps it down");
    for junk in [json!({"value": "yes"}), json!({"value": 1}), json!({"value": null}), json!({"value": [true]})] {
        assert!(a.run("ui.focusRibbon", junk).is_err());
    }
    assert!(a.focus_ribbon, "a refused value leaves it as it was");
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
    assert!(h.state().canvas.focused && h.state().session.view.focus_mode);
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
#[test]
fn the_community_button_off_hides_the_backstage_discord_link_too() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.run("ui.backstage", json!({"value": true})).unwrap();
    let mut h = harness(a);
    h.run_steps(2);
    assert!(h.query_by_label("Discord community").is_some(), "the community link shows by default");

    let mut a = app();
    a.ui.show_discord = false;
    a.run("ui.backstage", json!({"value": true})).unwrap();
    let mut h = harness(a);
    h.run_steps(2);
    assert!(h.query_by_label("Discord community").is_none(), "the backstage link honors the same setting");
}

/// #766: started with nothing to open, the app shows File › Home (the Start screen) as Word
/// does. #1129: no document is open behind it, so Esc (or Back) shows an empty window, with
/// no page, the ribbon greyed out but for Help, and nothing to save on close. With the setting
/// off it starts in the editor, and `WordApp::new` alone (tests, ui_shot, windows opened for a
/// document) never shows it.
#[test]
fn start_screen_shows_file_home_and_esc_leaves_no_document() {
    use egui_kittest::kittest::Queryable;
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
    assert!(h.query_by_label("Recent").is_some(), "and the Recent list");
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    let a = h.state_mut();
    assert!(!a.ui.backstage, "Esc leaves the Start screen");
    assert!(!a.has_document(), "for an empty window, not a blank document");
    assert_eq!(a.sent_title, "WordCraft", "the window is titled with the app alone");
    // The ribbon's commands are off, but Help, the File tab and the settings work.
    for id in ["format.bold", "text.insert", "file.save", "file.saveAs", "file.print", "edit.paste", "insert.table", "ui.print"] {
        assert!(!widgets::enabled(a, id), "{id} is off with no document");
    }
    for id in ["file.new", "file.open", "ui.backstage", "ui.tab", "ui.discord", "file.options", "ui.theme", "ui.statusCount"] {
        assert!(widgets::enabled(a, id), "{id} works with no document");
    }
    assert!(!chrome::qat_enabled(a, "file.save"));
    // Only the listed interface commands work: the ones that act on the document don't.
    for id in ["ui.contextMenu", "ui.render", "ui.linkTextBox", "ui.zotero.refresh", "ui.unlistedCommand"] {
        assert!(!works_without_document(id), "{id} needs a document");
    }
    let refused = a.run("ui.zotero.status", json!({})).unwrap_err();
    assert!(refused.contains("No document is open"), "refused before it runs: {refused}");
    assert!(works_without_document("font.add"), "a font file dropped on the empty window is added");
    // #4585: File › Options › Proofing › AutoCorrect Options edits a user setting, not the document.
    a.run("tools.autocorrect", json!({})).unwrap();
    assert_eq!(a.dialog.as_ref().map(|d| d.name()), Some("autoCorrect"));
    a.dialog = None;
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
    assert!(a.dialog.is_none());

    let mut b = app();
    b.run("ui.startScreen", json!({"value": false})).unwrap();
    assert!(!b.start_screen(), "off: today's behaviour");
    assert!(!b.ui.backstage && b.has_document());
}

/// #1129: the empty window's File tab goes back to the Backstage with only Home, New, Open
/// and Options; picking Blank document there opens one, ready for typing.
#[test]
fn blank_document_opens_a_document_from_the_start_screen() {
    use egui_kittest::kittest::Queryable;
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
    let a = h.state_mut();
    assert!(a.has_document() && !a.ui.backstage, "Blank document opens one");
    assert_ne!(a.session.document_id(), first);
    assert!(widgets::enabled(a, "format.bold"));
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Hello");
    assert!(a.sent_title.starts_with(&a.title_stem()), "{}", a.sent_title);
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

/// `ui.print` hands the document's PDF to the host's print hook, and is an error (with
/// nothing printed) when the host has none.
#[test]
fn ui_print_needs_the_hosts_print_hook() {
    let mut a = typed();
    assert!(a.run("ui.print", json!({})).is_err(), "no print hook: an error");

    let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = got.clone();
    a.services.print = Some(Box::new(move |bytes| {
        seen.borrow_mut().push(bytes.to_vec());
        Ok(())
    }));
    let r = a.run("ui.print", json!({})).unwrap();
    assert_eq!(r["printing"], true);
    assert_eq!(got.borrow().len(), 1, "printed once");
    assert!(got.borrow().first().is_some_and(|b| b.starts_with(b"%PDF")), "the hook got a PDF");
    assert!(a.session.dirty, "printing is not a save");

    a.services.print = Some(Box::new(|_| Err("blocked by the browser".into())));
    a.status_msg = None;
    let e = a.run("ui.print", json!({})).unwrap_err();
    assert!(e.contains("blocked by the browser"), "{e}");
    assert!(a.status_msg.as_ref().is_some_and(|(m, _)| m.contains("blocked by the browser")), "the status bar says why");
}

/// The print settings reach the hook: bad ones print nothing, `copies` repeats the pages.
#[test]
fn ui_print_honours_the_print_settings() {
    let mut a = typed();
    let sizes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = sizes.clone();
    a.services.print = Some(Box::new(move |bytes| {
        seen.borrow_mut().push(bytes.len());
        Ok(())
    }));
    assert!(a.run("ui.print", json!({"pages": "7-9"})).is_err(), "page 7 doesn't exist");
    assert!(a.run("ui.print", json!({"range": "nope"})).is_err());
    assert!(sizes.borrow().is_empty(), "nothing was sent");
    a.run("ui.print", json!({"range": "current", "copies": 3})).unwrap();
    assert_eq!(sizes.borrow().len(), 1);
}

/// The Print page shows the Print button only when the host can print, and the button
/// prints through `ui.print`.
#[test]
fn the_print_button_shows_only_with_a_print_hook() {
    use egui_kittest::kittest::Queryable;
    const BLURB: &str = "Send the document straight to the system print dialog, or save a copy in another format below.";
    for hook in [false, true] {
        let printed = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut a = app();
        if hook {
            let seen = printed.clone();
            a.services.print = Some(Box::new(move |_| {
                seen.set(seen.get() + 1);
                Ok(())
            }));
        }
        a.run("file.print", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..4 {
            h.step();
        }
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "print", "the Print page is open");
        assert_eq!(h.query_by_label(BLURB).is_some(), hook, "hook {hook}: the Print button's blurb");
        assert!(h.query_by_label("PDF document (*.pdf)").is_some(), "hook {hook}: the page rendered its export choices");
        if hook {
            // The page's own Print button (the backstage list has a Print entry too): the last.
            let buttons: Vec<_> = h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button).label("Print")).collect();
            buttons.last().expect("a Print button").click();
            h.step();
            h.step();
            assert_eq!(printed.get(), 1, "the button printed");
        }
    }
}

/// A Print page with printers found (so it draws the job message), in English.
fn print_page_harness(message: Option<crate::printing::Feedback>) -> egui_kittest::Harness<'static, WordApp> {
    let mut a = app();
    a.ui.language = "en".into();
    a.services.printers = Some(Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = sender.send(vec![printing::Printer { name: "Test queue".into(), label: String::new(), default: true }]);
        receiver
    }));
    a.services.print_native = Some(Box::new(|_, _, _, _| std::sync::mpsc::channel().1));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    // The page's first drawing starts discovery, which replaces the message: draw it once, leave,
    // and let the printers arrive.
    let _ = h.state_mut().run("file.print", json!({}));
    h.run_steps(4);
    let _ = h.state_mut().run("ui.backstage", json!({"value": false}));
    h.run_steps(3);
    set_print_message(&mut h, message);
    h.state_mut().print_memo = Default::default();
    h
}

/// A job message that arrives without the app's own `print_feedback` acting on it (a submitted job
/// would close the Print page); what the window has said is set by the test.
fn set_print_message(h: &mut egui_kittest::Harness<'_, WordApp>, message: Option<crate::printing::Feedback>) {
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
    use crate::printing::Feedback;
    use egui_kittest::kittest::Queryable;
    let mut h = print_page_harness(Some(Feedback::Submitted("job 7".into())));
    // A job went away earlier: its message was said in this window's status bar, and the page closed.
    h.state_mut().print_memo.said = Some(h.state().print_gen);
    let _ = h.state_mut().run("file.print", json!({}));
    h.run_steps(4);
    assert!(h.query_by_label_contains("job 7").is_none(), "the last job's message isn't repeated");
    assert_eq!(h.state().printing.message, Some(Feedback::Submitted("job 7".into())), "but the shared message stays");
    // A failure arrives while the page is open, said (by the status bar) in the very frame, so
    // only the page being open keeps it showing: wrong reopen detection would hide it.
    let failure = Some(Feedback::Failed("no paper".into()));
    set_print_message(&mut h, failure.clone());
    h.state_mut().print_memo.said = Some(h.state().print_gen);
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
    use crate::printing::Feedback;
    use egui_kittest::kittest::Queryable;
    let mut h = print_page_harness(None);
    // Every job is refused at once, with the same words.
    h.state_mut().services.print_native = Some(Box::new(|_, _, _, _| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = sender.send(Err("no paper".to_owned()));
        receiver
    }));
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
    use crate::printing::Feedback;
    use egui_kittest::kittest::Queryable;
    let mut h = print_page_harness(None);
    h.state_mut().ui.backstage = true;
    h.state_mut().ui.backstage_page = "home".into();
    h.run_steps(2);
    // Through the app's own `print_feedback` (run each frame), which says it in a status bar that isn't drawn.
    h.state_mut().printing.message = Some(Feedback::Failed("no paper".into()));
    h.run_steps(3);
    assert_eq!(h.state().print_memo.said, None, "File's home page doesn't say it");
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
    use crate::printing::Feedback;
    let failure = Some(Feedback::Failed("no paper".into()));
    let mut a = app();
    a.printing.message = failure.clone();
    let first = a.windows.shown();
    a.print_gen = 1;
    a.print_memo.said = Some(a.print_gen);
    a.new_window();
    assert_ne!(first, a.windows.shown());
    assert_eq!(a.print_memo.said, None, "the second window never said it, so its page shows it");
    assert_eq!(a.printing.message, failure, "the message is shared, and still there");
    assert!(a.enter_window(first));
    assert_eq!(a.print_memo.said, Some(1), "the first window said it, so its page doesn't repeat it");
    assert_eq!(a.printing.message, failure);
}

/// Return on the Print page prints, as in Word, where ⌘P opens on Print as the default
/// button; the document isn't typed into.
#[test]
fn return_on_the_print_page_prints() {
    let printed = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut a = app();
    let seen = printed.clone();
    a.services.print = Some(Box::new(move |_| {
        seen.set(seen.get() + 1);
        Ok(())
    }));
    let before = body_text(&a);
    a.run("file.print", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    assert_eq!(printed.get(), 0);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(printed.get(), 1, "Return printed once");
    assert_eq!(body_text(h.state()), before, "the document is unchanged");
    // Held down, Return repeats (egui marks a press of a key already down as a repeat): one job.
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    for _ in 0..4 {
        h.event(enter(true));
        h.run_steps(1);
    }
    h.event(enter(false));
    h.run_steps(1);
    assert_eq!(printed.get(), 2, "a held Return prints once");
    // The built-in file browser over the page takes Return for itself, even with nothing in it
    // focused: Return there saves or opens, and prints nothing.
    let _ = h.state_mut().run("ui.fileBrowser", json!({"purpose": "document"}));
    h.run_steps(4);
    assert!(h.state().file_browser.open.is_some(), "the browser opened");
    h.ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(printed.get(), 2, "Return in the file browser prints nothing");
}

/// #3210: a successful native print submission returns to the document, the same as Word —
/// previously the Print page stayed up until the user clicked Back, even though the job had
/// already gone to the spooler. A failed submission stays up instead, so the error (and the
/// settings to fix it) are still visible.
#[test]
fn a_successful_native_print_returns_to_the_document() {
    use printing::Printer;
    for ok in [true, false] {
        let mut a = app();
        a.printing.printers = vec![Printer { name: "Test Printer".into(), label: String::new(), default: true }];
        a.printing.settings.printer = "Test Printer".into();
        a.services.print_native = Some(Box::new(move |_, _, _, _| {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(if ok { Ok(String::new()) } else { Err("no paper".into()) });
            rx
        }));
        a.run("file.print", json!({})).unwrap();
        assert!(a.ui.backstage && a.ui.backstage_page == "print", "the Print page is open");
        a.canvas.want_focus = false;
        a.run("ui.print", json!({})).unwrap();
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
        assert_eq!(a.ui.backstage, !ok, "ok {ok}: whether the Print page is still open");
        assert_eq!(a.canvas.want_focus, ok, "ok {ok}: the document should take keyboard focus back only once the Print page actually closes");
        if !ok {
            assert_eq!(a.ui.backstage_page, "print", "the Print page is still the one showing, not some other backstage page");
            assert!(
                matches!(&a.printing.message, Some(printing::Feedback::Failed(error)) if error == "no paper"),
                "the failure is still visible: {:?}",
                a.printing.message
            );
        }
    }
}

/// #5710: a job waiting on a sleeping network printer's driver locked the Print page until the
/// driver answered. `ui.printCancel` stops it before it is sent and frees the page at once,
/// keeping the chosen printer; a job already being sent can't be stopped, and the page says so.
#[test]
fn cancelling_a_waiting_print_job_frees_the_print_page() {
    use printing::Printer;
    for (never_sent, close_document) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut a = app();
        a.printing.printers = vec![Printer { name: "Office".into(), label: String::new(), default: true }];
        a.printing.settings.printer = "Office".into();
        a.printing.settings.copies = 3;
        // The worker never answers, as when the driver doesn't: keep its sender alive.
        let held = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let senders = held.clone();
        a.services.print_native = Some(Box::new(move |_, _, _, _| {
            let (tx, rx) = std::sync::mpsc::channel();
            senders.borrow_mut().push(tx);
            rx
        }));
        let asked = std::rc::Rc::new(std::cell::Cell::new(0));
        let cancels = asked.clone();
        a.services.cancel_print = Some(Box::new(move || {
            cancels.set(cancels.get() + 1);
            never_sent
        }));
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
        let enabled = |a: &WordApp| printing::commands(a).iter().any(|c| c["id"] == "ui.printCancel" && c["enabled"] == true);
        assert!(enabled(&a), "Cancel is offered while the job waits");
        let reply = a.execute("ui.printCancel", json!({})).unwrap();
        assert_eq!(reply["cancelled"], never_sent);
        assert_eq!(asked.get(), 1, "the host was asked to stop the job");
        assert_eq!(a.printing.busy(), !never_sent, "never_sent {never_sent}: whether the page is still locked");
        assert_eq!(a.printing.settings.printer, "Office", "the chosen printer is kept");
        assert_eq!(a.printing.settings.copies, 3, "the settings are kept");
        assert_eq!(a.printing.printers.len(), 1, "the printer list is kept");
        let said = a.status_msg.as_ref().map(|(text, _)| text.as_str()).unwrap_or_default();
        let expected = if never_sent {
            "Printing cancelled. Nothing was sent to the printer."
        } else {
            "Cancellation could not be confirmed here. The printer driver may still be processing the job. If it appears in the system's print queue, cancel it there."
        };
        assert_eq!(said, expected, "never_sent {never_sent}: the status bar says what happened");
        assert_eq!(enabled(&a), !never_sent);
        if never_sent {
            assert_eq!(a.execute("ui.printCancel", json!({})).unwrap_err(), "no print job is being submitted");
        }
        assert_eq!(held.borrow().len(), 1);
    }
}

/// #819: Alt+Q opens Search with its box focused, as in Word: the Q isn't typed into the
/// document, and what is typed next goes into the search box.
#[cfg(not(target_os = "macos"))]
#[test]
fn alt_q_opens_search_and_typing_goes_there() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app(),
    );
    for _ in 0..6 {
        h.step();
    }
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    h.event(egui::Event::Text("Kilns".into()));
    h.step();
    // Some platforms send the key's text with an Alt shortcut; it isn't typed.
    h.event(key(egui::Key::Q, egui::Modifiers::ALT));
    h.event(egui::Event::Text("q".into()));
    h.step();
    // The title bar shows the Search box, so Search opens there, in place (#5398).
    let query = |h: &egui_kittest::Harness<WordApp>| crate::chrome::title_search_query(h.state(), &h.ctx);
    assert!(h.state().dialog.is_none(), "no Search Commands dialog over the page");
    assert_eq!(query(&h).as_deref(), Some(""), "Search is open and empty");
    h.step();
    {
        use egui_kittest::kittest::Queryable;
        assert!(h.query_by_label_contains("Word Count").is_some(), "an empty box suggests commands, as Word's does");
    }
    h.event(egui::Event::Text("bold".into()));
    h.step();
    assert_eq!(query(&h).as_deref(), Some("bold"), "typing goes into the search box");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns");
    h.event(key(egui::Key::Escape, egui::Modifiers::NONE));
    h.step();
    h.step();
    assert!(query(&h).is_none(), "Escape closes Search");
    assert!(!h.state().session.view.full_screen_view);
    // Words that name no command: Enter finds them in the document, in the Navigation pane.
    h.event(key(egui::Key::Q, egui::Modifiers::ALT));
    h.step();
    h.step();
    // Typing sends each key's press before its text.
    h.event(key(egui::Key::K, egui::Modifiers::SHIFT));
    h.event(egui::Event::Text("Kilns".into()));
    h.step();
    h.event(key(egui::Key::Enter, egui::Modifiers::NONE));
    h.step();
    h.step();
    assert!(query(&h).is_none(), "Enter ran the search");
    assert!(h.state().session.view.nav_pane, "the Navigation pane shows the results");
    assert_eq!(h.state().session.find.results.len(), 1);
}

/// #5398 (review): Focus hides the Search box, so Search there is the dialog, even on the frame
/// Focus comes on; and Search left open when Focus comes on doesn't keep Escape from leaving it.
#[test]
fn focus_mode_search_never_opens_in_the_hidden_box() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app(),
    );
    h.run_steps(6);
    let query = |h: &egui_kittest::Harness<WordApp>| crate::chrome::title_search_query(h.state(), &h.ctx);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    // Search, as Alt+Q opens it (Windows and Linux; macOS has no Alt+Q, Option+Q types a character).
    let search = |h: &mut egui_kittest::Harness<WordApp>| {
        let _ = h.state_mut().run("ui.dialog", json!({"name": "commands"}));
    };
    // The last frame drew the box; Focus comes on just before Search.
    h.state_mut().run("view.focus", json!({"value": true})).unwrap();
    search(&mut h);
    h.step();
    assert!(query(&h).is_none(), "no Search in a box Focus hides");
    assert!(matches!(h.state().dialog, Some(dialogs::Dialog::Commands { .. })), "Search opens as the dialog");
    h.state_mut().dialog = None;
    h.state_mut().run("view.focus", json!({"value": false})).unwrap();
    h.run_steps(3);
    // Search open in the title bar, then Focus comes on: Escape leaves Focus.
    search(&mut h);
    h.run_steps(2);
    assert!(query(&h).is_some());
    h.state_mut().run("view.focus", json!({"value": true})).unwrap();
    h.run_steps(3);
    assert!(query(&h).is_none(), "Search whose box is gone is dropped");
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
    assert!(a.dialog.is_none(), "a query never opens the box");
    a.run("tools.search", json!({})).unwrap();
    assert!(matches!(a.dialog, Some(dialogs::Dialog::Commands { .. })), "no query opens Search");
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
