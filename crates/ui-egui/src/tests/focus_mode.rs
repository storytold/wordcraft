//! Focus (View › Immersive › Focus, `view.focus`, #1678) integration tests through the whole
//! frame loop: what the window shows in Focus, how it is entered and left from every path, and
//! how it meets panes, dialogs, the File page, keytips, Read Mode and a window whose last
//! document was closed. The engine side (the command's contract, per-document view state) is
//! tested in `wordcraft-engine`.

use super::*;

/// Where the first page's top-left corner lands on screen: how far down the page starts.
fn page_top(h: &mut egui_kittest::Harness<'static, WordApp>) -> f32 {
    crate::canvas::page_to_screen(h.state_mut(), 0, 0.0, 0.0).map_or(f32::NAN, |p| p.y)
}

/// The part of the window the page is drawn in: the panes take their width out of it.
fn viewport(h: &mut egui_kittest::Harness<'static, WordApp>) -> egui::Rect {
    h.state_mut().canvas.viewport.unwrap_or(egui::Rect::NOTHING)
}

/// Focus hides every pane, the ruler and the status bar, moves the page up to the slim bar, and
/// gives all of it back when it ends.
#[test]
fn focus_hides_panes_the_ruler_and_the_status_bar_and_gives_them_back() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("view.navigationPane", json!({"value": true})).unwrap();
    a.run("edit.clipboardPane", json!({"value": true})).unwrap();
    a.run("view.ruler", json!({"value": true})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    let usual = viewport(&mut h);
    assert!(usual.left() > 400.0, "the two docked panes take their width out of the page: {usual:?}");
    let usual_top = page_top(&mut h);
    assert!(usual_top > 0.0, "the page has a place on screen");

    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(4);
    assert!(h.state().session.view.focus_mode);
    assert!(viewport(&mut h).left() <= 10.0, "Focus hides the panes and gives the page the whole width ({:?})", viewport(&mut h));
    assert!(h.query_by_label("Focus").is_none(), "and the status bar, so the Focus button goes with it");
    assert!(h.query_by_label("Show the ribbon").is_some(), "the slim bar's button is the only chrome left");
    let in_focus = page_top(&mut h);
    assert!(in_focus < usual_top, "the page starts higher: no ruler, no status bar ({in_focus} vs {usual_top})");
    assert!(h.state().session.view.ruler, "the ruler is hidden, not turned off");
    assert!(h.state().session.view.nav_pane, "and the panes are hidden, not closed");

    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(!h.state().session.view.focus_mode, "Escape leaves Focus");
    assert!(h.query_by_label("Focus").is_some(), "the status bar comes back, with its Focus button");
    assert_eq!(viewport(&mut h), usual, "and the panes take their width back");
    assert!((page_top(&mut h) - usual_top).abs() < 0.5, "the page is back where it was");
}

/// Both of the app's Focus controls work: the status bar's button and the ribbon's View ›
/// Immersive › Focus, which a keytip reaches. Focus's slim bar has the drop-down button only — no
/// Search box, which belongs to Full Screen View's bar.
#[test]
fn the_status_bar_button_and_the_ribbon_button_both_turn_focus_on() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(app());
    h.run_steps(4);
    // The status bar's button: the only "Focus" the window shows while the Home tab is up.
    assert_eq!(h.query_all_by_label("Focus").count(), 1);
    h.get_by_label("Focus").click();
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode, "the status bar's Focus turns Focus on");
    assert!(h.query_by_label("Search commands and help").is_none(), "Focus's bar has no Search box");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.focus_mode);

    // The ribbon's button, through keytips: F10, W (View), then the button's own letter.
    h.key_press(egui::Key::F10);
    h.run_steps(2);
    h.key_press(egui::Key::W);
    h.run_steps(3);
    assert_eq!(h.state().ui.tab, "View");
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Commands);
    let index = crate::keytips::controls_public("View").iter().position(|c| c.id == "view.focus").expect("the View tab has a Focus control");
    let letter = crate::keytips::letter_for(h.state(), index);
    assert!(!letter.is_empty(), "the Focus control has a keytip letter");
    let key = egui::Key::from_name(&letter).unwrap_or_else(|| panic!("no key named {letter}"));
    h.key_press(key);
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode, "the ribbon's Focus turns Focus on");
    assert!(!h.state().focus_ribbon, "the ribbon does not drop down by itself");
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Off, "and keytips end");
}

/// The ribbon's large buttons are drawn, not egui widgets, but they are named for screen readers
/// now: the ribbon's Focus is announced like the status bar's.
#[test]
fn both_focus_buttons_have_accessible_names() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(app());
    h.run_steps(4);
    h.state_mut().run("ui.tab", json!({"tab": "View"})).unwrap();
    h.run_steps(3);
    let named = h.query_all_by_label("Focus").map(|n| n.rect().top()).collect::<Vec<_>>();
    assert_eq!(named.len(), 2, "the ribbon's Focus and the status bar's: {named:?}");
    assert!(named.iter().any(|y| *y < 200.0), "the ribbon's, at the top: {named:?}");
    assert!(named.iter().any(|y| *y > 400.0), "and the status bar's, at the bottom: {named:?}");
}

/// Escape closes a dialog first and leaves Focus only after it: one Escape path, the nearest
/// thing first.
#[test]
fn escape_closes_a_dialog_before_it_leaves_focus() {
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.state_mut().run("ui.dialog", json!({"name": "find"})).unwrap();
    h.run_steps(3);
    assert!(h.state().dialog.is_some(), "the Find dialog is open");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "the first Escape closes the dialog");
    assert!(h.state().session.view.focus_mode, "and keeps Focus");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.focus_mode, "the next Escape leaves Focus");
}

/// The File page opens in Focus and closing it comes back to Focus, not to the usual window.
#[test]
fn the_file_page_opens_in_focus_and_escape_returns_to_it() {
    use egui_kittest::kittest::Queryable;
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
    h.run_steps(4);
    assert!(h.state().ui.backstage, "the File page is open");
    assert!(h.query_by_label("Show the ribbon").is_none(), "the File page takes the whole window");
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(!h.state().ui.backstage, "Escape closes the File page");
    assert!(h.state().session.view.focus_mode, "and comes back to Focus");
    assert!(h.query_by_label("Show the ribbon").is_some(), "the slim bar is back");
}

/// Keytips in Focus: F10 (Alt on Windows) starts them, the tab badges are laid along the slim bar
/// (there is no ribbon to hang them on), and picking a tab drops the ribbon down so its command
/// badges have somewhere to land. The keys work throughout.
#[test]
fn keytips_in_focus_show_their_badges_on_the_slim_bar() {
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::F10);
    h.run_steps(3);
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Tabs, "F10 starts keytips");
    // The badges are painted, so they are checked where they are laid out: one per tab, inside the
    // slim bar, left to right, and clear of the "⋯" in the middle.
    let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, crate::chrome::SCREEN_BAR_HEIGHT));
    let badges = crate::keytips::slim_bar_badges(h.state(), bar, &[]);
    let letters: Vec<&str> = badges.iter().map(|(_, l)| l.as_str()).collect();
    assert!(letters.starts_with(&["F", "H", "N"]), "the tab badges in tab order: {letters:?}");
    assert!(badges.iter().all(|(r, _)| bar.contains_rect(*r)), "every badge is in the slim bar: {badges:?}");
    assert!(badges.windows(2).all(|w| w[0].0.max.x <= w[1].0.min.x), "and none overlaps the next");
    // None reaches the "⋯", which sits at the bar's centre: the badges stop before it.
    let centre = egui::Rect::from_center_size(bar.center(), egui::vec2(40.0, 20.0));
    assert!(badges.iter().all(|(r, _)| !r.intersects(centre)), "clear of the ... button: {badges:?}");
    // Every badge the frame paints: the bar's tabs, plus whatever the chrome recorded (in Full
    // Screen View the Search box records its own Q).
    let all: Vec<String> = crate::keytips::badge_rects(h.state(), bar, Vec::new(), Vec::new()).into_iter().map(|(_, l)| l).collect();
    assert_eq!(all, letters.iter().map(|l| l.to_string()).collect::<Vec<_>>(), "the bar's badges are what Focus paints");
    // W is the View tab's keytip letter: the ribbon drops down, since command badges need it.
    h.key_press(egui::Key::W);
    h.run_steps(3);
    assert_eq!(h.state().ui.tab, "View", "the key picked the View tab");
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Commands, "and waits for a command's letter");
    assert!(h.state().focus_ribbon, "the ribbon dropped down for the command badges");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Off, "Escape cancels keytips");
    assert!(h.state().session.view.focus_mode, "and stays in Focus");
    h.event(egui::Event::Text("x".into()));
    h.run_steps(2);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains('x'), "typing reaches the page again");
}

/// A tab hidden in Customize Ribbon has no slim-bar badge in Focus: the badges follow the tabs the
/// strip shows, so a keytip cannot select a tab that is not on display (#1678).
#[test]
fn a_tab_hidden_in_customize_ribbon_has_no_slim_bar_badge() {
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().run("tools.customizeRibbon", json!({"hide": "Review"})).unwrap();
    h.run_steps(3);
    assert!(!crate::keytips::badge_tabs(h.state()).contains(&"Review"), "the hidden tab has no badge: {:?}", crate::keytips::badge_tabs(h.state()));
    let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, crate::chrome::SCREEN_BAR_HEIGHT));
    let badges = crate::keytips::slim_bar_badges(h.state(), bar, &[]);
    let letters: Vec<&str> = badges.iter().map(|(_, l)| l.as_str()).collect();
    assert!(!letters.contains(&"R"), "no badge for the hidden Review tab: {letters:?}");
    assert!(letters.starts_with(&["F", "H", "N"]), "the visible tabs still badge in order: {letters:?}");
}

/// The operating system's own full-screen exit (the green button, the window manager) ends
/// Focus.
#[test]
fn the_system_leaving_full_screen_ends_focus() {
    let ctx = egui::Context::default();
    let mut a = app();
    a.run("view.focus", json!({})).unwrap();
    let frame = |a: &mut WordApp, fullscreen: Option<bool>| -> Vec<bool> {
        let mut input = egui::RawInput::default();
        input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { fullscreen, focused: Some(true), ..Default::default() });
        let out = ctx.run_ui(input, |ui| {
            a.logic(ui.ctx());
            a.ui(ui);
        });
        let sent: Vec<bool> = out
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .filter_map(|c| if let egui::ViewportCommand::Fullscreen(b) = c { Some(*b) } else { None })
            .collect();
        out.drop_without_applying_deltas();
        sent
    };
    let asked: Vec<bool> = (0..4).flat_map(|_| frame(&mut a, None)).collect();
    assert_eq!(asked, [true], "Focus asks for full screen");
    assert!(a.session.view.focus_mode);
    // The system reports the window back out of full screen, as its green button does.
    let sent = frame(&mut a, Some(false));
    assert!(!a.session.view.focus_mode, "leaving full screen ends Focus");
    assert!(sent.is_empty(), "and the window is already out: nothing more to ask ({sent:?})");
}

/// Closing the last document while in Focus keeps the Focus bar: the empty window is still in the
/// screen state, so "⋯" and Escape are there to leave it, and the view commands work with no
/// document open. Nothing can strand the window in full screen with ordinary chrome and no way
/// out.
#[test]
fn closing_the_last_document_in_focus_keeps_the_way_out() {
    use egui_kittest::kittest::Queryable;
    // A blank document, so closing it asks nothing and really closes it.
    let mut a = app();
    a.run("view.focus", json!({})).unwrap();
    let mut h = harness(a);
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode);
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(4);
    assert!(!h.state().has_document(), "the last document is closed");
    assert!(h.state().session.view.focus_mode, "Focus is still on");
    assert!(h.query_by_label("Show the ribbon").is_some(), "the empty window keeps the Focus bar");
    assert!(h.query_by_label("Insert").is_none(), "and not the usual chrome");
    // The view commands are allowed with no document open, so the bar's button and Escape both
    // work, and so does the ribbon's.
    for id in ["view.focus", "view.fullScreenView", "view.fullScreen"] {
        assert!(h.state_mut().run(id, json!({"value": false})).is_ok(), "{id} works with no document open");
    }
    h.state_mut().run("view.focus", json!({"value": true})).unwrap();
    h.run_steps(3);
    // "⋯" works too: the ribbon comes back over the empty window, its tabs live (File opens a
    // document) even though its buttons are greyed.
    h.get_by_label("Show the ribbon").click();
    h.run_steps(3);
    assert!(h.state().focus_ribbon, "the empty window's bar drops the ribbon");
    assert!(h.query_by_label("File").is_some(), "and the tabs are there to open a document");
    h.state_mut().run("ui.focusRibbon", json!({"value": false})).unwrap();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.focus_mode, "Escape leaves Focus with no document open");
    assert!(h.query_by_label("Insert").is_some(), "and the empty window shows its ribbon again");
    // A document opened afterwards is not dragged into Focus either.
    h.state_mut().run("file.new", json!({})).unwrap();
    h.run_steps(4);
    assert!(h.state().has_document());
    assert!(!h.state().session.view.focus_mode, "the new document opens in the usual window");
    assert!(h.query_by_label("Insert").is_some());
}

/// Focus and Read Mode at once: the reading view on the black Focus canvas, and Escape leaves
/// Focus first, then Read Mode, so one key press never leaves two views.
#[test]
fn focus_and_read_mode_together() {
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.state_mut().run("view.readMode", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode && h.state().session.view.read_mode);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.focus_mode, "the first Escape leaves Focus");
    assert!(h.state().session.view.read_mode, "and stays in Read Mode");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.read_mode, "the next one leaves Read Mode");
}

/// Typing, formatting and undo work in Focus, and leaving it changes nothing.
#[test]
fn typing_formatting_and_undo_work_in_focus() {
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text(" world".into()));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello world");
    h.state_mut().run("select.all", json!({})).unwrap();
    h.state_mut().run("format.bold", json!({})).unwrap();
    h.run_steps(2);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello world", "undo works in Focus");
    // The selection is still there: the first Escape collapses it, as in the usual window.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().session.sel.is_collapsed() && h.state().session.view.focus_mode, "the first Escape collapses the selection");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.view.focus_mode);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello world", "leaving Focus changes nothing");
}

/// A text field in the chrome that has the keyboard when Focus starts, turned on without a click
/// (a keybinding, the control channel, MCP): the field goes with the ribbon, and the keyboard goes
/// back to the page, as a click on the chrome gives it (#1093). Collapsing the ribbon with the
/// field focused does the same, since the same chrome disappears.
#[test]
fn focus_started_without_a_click_hands_the_keyboard_to_the_page() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(app());
    h.run_steps(4);
    h.get_by_label("Font Size").click();
    h.run_steps(3);
    assert!(h.state_mut().ctx.as_ref().is_some_and(|ctx| ctx.memory(|m| m.focused().is_some())), "the Font Size box has the keyboard");
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode);
    assert!(h.state().canvas.focused, "the page has the keyboard");
    h.event(egui::Event::Text("x".into()));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "x", "typing reaches the page");

    // The ribbon's own collapse does the same, without any screen state.
    let mut h = harness(app());
    h.run_steps(4);
    h.get_by_label("Font Size").click();
    h.run_steps(3);
    h.state_mut().run("ui.collapseRibbon", json!({"value": true})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.ribbon_collapsed && h.state().canvas.focused, "collapsing the ribbon gives the page the keyboard");
    h.event(egui::Event::Text("y".into()));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "y");
}

/// The click path is fine: clicking the status bar's Focus button gives the page the keyboard, as
/// any click on the chrome does (#1093).
#[test]
fn focus_clicked_from_the_status_bar_gives_the_page_the_keyboard() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(app());
    h.run_steps(4);
    h.get_by_label("Font Size").click();
    h.run_steps(3);
    h.get_by_label("Focus").click();
    h.run_steps(3);
    assert!(h.state().session.view.focus_mode && h.state().canvas.focused);
    h.event(egui::Event::Text("z".into()));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "z");
}

/// The Search palette is a dialog, not chrome, so it still opens in Focus (F10, Q — its keytip)
/// and Escape closes it without leaving Focus.
#[test]
fn the_search_palette_opens_in_focus() {
    let mut h = canvas_harness();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::F10);
    h.run_steps(2);
    h.key_press(egui::Key::Q);
    h.run_steps(3);
    assert!(matches!(h.state().dialog, Some(dialogs::Dialog::Commands { .. })), "Search opened in Focus");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "Escape closes Search");
    assert!(h.state().session.view.focus_mode, "and Focus stays on");
}

/// The dropped ribbon respects a collapsed ribbon (Ctrl+F1): it drops as the tab strip only, as
/// the ribbon does in the usual window, and the tabs still switch.
#[test]
fn the_dropped_ribbon_respects_a_collapsed_ribbon() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(app());
    h.run_steps(4);
    h.state_mut().run("ui.collapseRibbon", json!({"value": true})).unwrap();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Show the ribbon").click();
    h.run_steps(3);
    assert!(h.state().focus_ribbon);
    assert!(h.query_by_label("Insert").is_some(), "the tab strip drops down");
    assert!(h.query_by_label("Paste").is_none(), "the buttons stay collapsed");
    h.get_by_label("Insert").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tab, "Insert", "the tabs still switch");
    assert!(h.state().session.view.focus_mode, "and Focus stays on");
}

/// F6 with the panes hidden: the keyboard stays on the page instead of moving to a pane that is
/// not drawn, so typing keeps working.
#[test]
fn f6_in_focus_keeps_the_keyboard_on_the_page() {
    let mut h = canvas_harness();
    h.state_mut().run("view.navigationPane", json!({"value": true})).unwrap();
    h.state_mut().run("view.focus", json!({})).unwrap();
    h.run_steps(3);
    for _ in 0..3 {
        h.key_press(egui::Key::F6);
        h.run_steps(2);
        assert!(h.state().canvas.focused, "F6 leaves the keyboard on the page: the panes are not drawn");
    }
    h.event(egui::Event::Text("k".into()));
    h.run_steps(2);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hellok");
}

/// Full Screen View's keytips: its bar has a Search box, which records a Q badge during layout.
/// The tab badges must still be laid along the bar — testing "no rects were recorded" would leave
/// that state showing Q alone (a review finding) — and the ribbon still drops for the commands.
#[test]
fn full_screen_view_keeps_its_tab_badges_beside_search() {
    let mut h = canvas_harness();
    h.state_mut().run("view.fullScreenView", json!({"value": true})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::F10);
    h.run_steps(3);
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Tabs);
    // What the Search box records during layout (a Q badge over itself), and what the frame paints
    // on top of it.
    let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, crate::chrome::SCREEN_BAR_HEIGHT));
    let search = (egui::Rect::from_min_size(egui::pos2(700.0, 6.0), egui::vec2(15.0, 15.0)), "Q".to_string());
    let painted: Vec<String> = crate::keytips::badge_rects(h.state(), bar, vec![search], Vec::new()).into_iter().map(|(_, l)| l).collect();
    assert!(painted.contains(&"Q".to_string()), "Search keeps its badge: {painted:?}");
    assert!(painted.contains(&"F".to_string()) && painted.contains(&"W".to_string()), "and the tabs are there too: {painted:?}");
    // A W picks View and drops the ribbon, whose command badges then have a home.
    h.key_press(egui::Key::W);
    h.run_steps(3);
    assert!(h.state().focus_ribbon, "the ribbon drops down for the command badges");
}

/// A narrow window: Full Screen View's Search box takes the bar's middle, and the tab badges must
/// stop before it instead of sliding under its `Q` badge.
#[test]
fn narrow_full_screen_view_keeps_search_clear_of_the_tab_badges() {
    let mut h = canvas_harness();
    h.state_mut().run("view.fullScreenView", json!({"value": true})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::F10);
    h.run_steps(3);
    assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Tabs);
    // The bar is 716 points wide, so the Search box spans 228–488 with its Q badge over it.
    let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(716.0, crate::chrome::SCREEN_BAR_HEIGHT));
    let search = egui::Rect::from_center_size(bar.center(), egui::vec2(260.0, 22.0));
    let search_badge = (search, "Q".to_string());
    let painted = crate::keytips::badge_rects(h.state(), bar, vec![search_badge], Vec::new());
    let (tabs, q): (Vec<(egui::Rect, String)>, Vec<(egui::Rect, String)>) = painted.into_iter().partition(|(_, l)| l != "Q");
    assert_eq!(q.len(), 1, "Search keeps its badge");
    assert!(tabs.iter().all(|(r, _)| !r.intersects(search.expand2(egui::vec2(4.0, 2.0)))), "no tab badge reaches the Search box: {tabs:?}");
    assert!(!tabs.is_empty(), "the tabs that fit are still there: {tabs:?}");
    assert!(tabs.iter().all(|(r, _)| bar.contains_rect(*r)), "every tab badge is in the bar: {tabs:?}");
    // With the ribbon dropped over the page the tab strip is laid out, so the slim-bar set must
    // not double it up.
    h.key_press(egui::Key::W);
    h.run_steps(3);
    assert!(h.state().focus_ribbon);
    let painted: Vec<String> = crate::keytips::badge_rects(
        h.state(),
        bar,
        vec![(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(30.0, 15.0)), "F".to_string())],
        Vec::new(),
    )
    .into_iter()
    .map(|(_, l)| l)
    .collect();
    assert_eq!(painted.iter().filter(|l| l.as_str() == "F").count(), 1, "one F badge, not two: {painted:?}");
}

/// Full Screen View can drop the ribbon too, which is how its keytips show their command badges:
/// it has no "⋯" of its own, but a click on the page puts the ribbon away and leaves the state.
#[test]
fn full_screen_view_drops_the_ribbon_for_keytips() {
    let mut h = canvas_harness();
    h.state_mut().run("view.fullScreenView", json!({"value": true})).unwrap();
    h.run_steps(3);
    assert!(h.state().session.view.full_screen_view && !h.state().focus_ribbon);
    h.key_press(egui::Key::F10);
    h.run_steps(2);
    h.key_press(egui::Key::W);
    h.run_steps(3);
    assert!(h.state().focus_ribbon, "the ribbon drops down for the command badges");
    assert!(h.state().session.view.full_screen_view, "and Full Screen View stays on");
    // A click on the page puts it away without leaving the state.
    let page = crate::canvas::page_to_screen(h.state_mut(), 0, 300.0, 600.0).unwrap_or(egui::pos2(400.0, 400.0));
    h.event(egui::Event::PointerMoved(page));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: page, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    h.run_steps(3);
    assert!(!h.state().focus_ribbon, "a click on the page puts the ribbon away");
    assert!(h.state().session.view.full_screen_view);
}
