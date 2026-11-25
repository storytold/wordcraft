use super::*;
use egui_kittest::kittest::{NodeT as _, Queryable};
use serde_json::json;

/// The whole app in the shared harness, past the frames the fonts need.
fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(app_frame, a);
    crate::test_harness::frames(&mut h, 6);
    h
}

/// Like [`harness`], in the integrated frame the desktop app draws on Windows and Linux (S-L417): the title bar draws
/// the caption buttons. The shared test spec has no decided frame, so the bar draws none.
fn integrated_harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    use craft::ui::shell::{Caption, CaptionFamily, Captions, Frame};
    use craft::window::policy::{Decision, Desktop, DesktopKind};
    let spec = || {
        let layout = craft::window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
        let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
        crate::test_harness::install_spec().frame(Decision { frame, overridden: None, desktop })
    };
    let mut h = crate::test_harness::builder_with(spec).with_size(egui::vec2(1440.0, 900.0)).build_ui_state(app_frame, a);
    crate::test_harness::frames(&mut h, 6);
    h
}

fn app_frame(ui: &mut egui::Ui, app: &mut WordApp) {
    let ctx = ui.ctx().clone();
    app.logic(&ctx);
    app.ui(ui);
}

/// Presses and releases `button` over `at` the way a person does: the pointer arrives and rests a frame before the
/// press, and the release is a frame of its own. The framework opens a menu from its opener on that sequence;
/// kittest's `click` sends the move, press and release in one frame.
fn click_at(h: &mut egui_kittest::Harness<'static, WordApp>, at: egui::Pos2, button: egui::PointerButton) {
    h.event(egui::Event::PointerMoved(at));
    crate::test_harness::frames(h, 2);
    h.event(egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: egui::Modifiers::NONE });
    crate::test_harness::frames(h, 1);
    h.event(egui::Event::PointerButton { pos: at, button, pressed: false, modifiers: egui::Modifiers::NONE });
    // The menu opens in the pass after the release and settles over the passes after that.
    crate::test_harness::frames(h, 6);
}

/// The title bar's title: its button (the bar also names a group of the same text).
fn title_rect(h: &egui_kittest::Harness<'static, WordApp>) -> egui::Rect {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Document1 \u{2014} Not saved").rect()
}

/// The viewport commands the next `steps` passes send.
fn commands_over(h: &mut egui_kittest::Harness<'static, WordApp>, steps: usize) -> Vec<egui::ViewportCommand> {
    let mut sent = Vec::new();
    for _ in 0..steps {
        crate::test_harness::step(h);
        sent.extend(h.output().viewport_output.values().flat_map(|v| v.commands.iter().cloned()));
    }
    sent
}

/// Whether a primary drag from `at` asks the OS to move the window.
fn drag_moves_the_window(h: &mut egui_kittest::Harness<'static, WordApp>, at: egui::Pos2) -> bool {
    let to = at + egui::vec2(40.0, 0.0);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    let mut sent = commands_over(h, 2);
    h.event(egui::Event::PointerMoved(to));
    sent.extend(commands_over(h, 2));
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    sent.extend(commands_over(h, 2));
    sent.iter().any(|c| matches!(c, egui::ViewportCommand::StartDrag))
}

fn new_app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default())
}

/// #701: the arrow beside Undo lists the session's undo steps, newest first; clicking the third
/// row undoes three steps at once.
#[test]
fn undo_list_matches_the_session_and_undoes_up_to_the_clicked_row() {
    use egui::accesskit::Role;
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("select.all", json!({})).unwrap();
    a.run("format.bold", json!({})).unwrap();
    a.run("format.italic", json!({})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("text.insert", json!({"text": " world"})).unwrap();
    let session: Vec<String> = a.session.undo_history().into_iter().map(str::to_string).collect();
    assert_eq!(session, ["Typing", "Italic", "Bold", "Typing"]);
    assert_eq!(undo_list_rows(&a), session);

    let mut h = harness(a);
    h.get_by_label("Undo History").click();
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(h.query_all_by_role(Role::ListBoxOption).count(), 4, "one row per step");
    h.get_by_role_and_label(Role::ListBoxOption, "Bold").click();
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(h.state().session.undo_history(), ["Typing"], "three steps undone in one go");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello");
    assert_eq!(h.query_all_by_role(Role::ListBoxOption).count(), 0, "the list closes");
}

/// #3040: clicking a row in the Undo History popover undoes the step and returns keyboard focus
/// to the document so typing continues immediately.
#[test]
fn undo_list_row_returns_focus_to_the_document() {
    use egui::accesskit::Role;
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    assert_eq!(a.session.undo_history().len(), 1);

    let mut h = harness(a);
    assert!(h.state().canvas.focused(&h.ctx), "canvas starts focused");

    let arrow = h.get_by_label("Undo History").rect().center();
    click_at(&mut h, arrow, egui::PointerButton::Primary);
    assert_eq!(h.query_all_by_role(Role::ListBoxOption).count(), 1, "the arrow opens the Undo list");
    assert!(!h.state().canvas.focused(&h.ctx), "the open Undo list holds the keyboard");

    let row = h.get_by_role_and_label(Role::ListBoxOption, "Typing").rect().center();
    click_at(&mut h, row, egui::PointerButton::Primary);
    assert_eq!(h.query_all_by_role(Role::ListBoxOption).count(), 0, "picking a row closes the Undo list");
    assert!(h.state().session.undo_history().is_empty(), "the step was undone");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "", "the undone text is gone");
    assert!(h.state().canvas.focused(&h.ctx), "the row click returns focus to the document");

    h.event(egui::Event::Text("!".into()));
    crate::test_harness::frames(&mut h, 2);
    assert_eq!(
        h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body),
        "!",
        "typing reaches the document immediately after picking an Undo list row"
    );
}

/// #1975: the zoom percentage gains and loses digits (9% → 100% → 500%); the controls to its
/// left must not shift with it.
#[test]
fn status_bar_controls_hold_still_across_zoom_levels() {
    let mut h = harness(new_app());
    let names = ["Focus", "Print Layout", "Web Layout"];
    let mut seen: Vec<(f64, Vec<f32>)> = Vec::new();
    // 9% is under the engine's floor (10%) and the canvas clamps to it, so the digits go 2, 3, 3.
    for zoom in [10.0_f64, 25.0, 100.0, 500.0] {
        h.state_mut().run("view.zoom", json!({"value": zoom})).unwrap();
        crate::test_harness::frames(&mut h, 3);
        let xs = names.iter().map(|name| h.get_by_label(name).rect().min.x).collect();
        seen.push((zoom, xs));
    }
    let first = seen[0].1.clone();
    for (zoom, xs) in &seen {
        assert_eq!(xs, &first, "status bar controls moved at {zoom}% zoom");
    }
}

/// The status bar's view buttons are icons, as the old app's (Read Mode, Print Layout, Web Layout in that order),
/// each named by its view and the current view pressed.
#[test]
fn status_bar_view_buttons_are_icons_with_the_view_pressed() {
    use egui::accesskit::{Role, Toggled};
    let h = harness(new_app());
    let names = ["Read Mode", "Print Layout", "Web Layout"];
    let rects: Vec<egui::Rect> = names.iter().map(|name| h.get_by_role_and_label(Role::Button, name).rect()).collect();
    for (name, r) in names.iter().zip(&rects) {
        assert!(r.width() <= r.height() + 4.0, "{name} shows its icon only: {r:?}");
    }
    assert!(rects.windows(2).all(|w| w[0].min.x < w[1].min.x), "Read Mode, Print Layout, Web Layout from the start: {rects:?}");
    let pressed: Vec<bool> =
        names.iter().map(|name| h.get_by_role_and_label(Role::Button, name).accesskit_node().toggled() == Some(Toggled::True)).collect();
    assert_eq!(pressed, [false, true, false], "Print Layout, the view, alone is pressed");
}

/// #2797: the status bar's totals are the Word Count dialog's numbers (words, characters with
/// spaces, characters without spaces), and its selection line counts the selection the same way.
#[test]
fn the_status_bar_totals_are_the_word_count_dialogs_numbers() {
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Hello world.\nSecond line"})).unwrap();
    // Straight from the engine: `a.run` would only open the Word Count dialog (#240).
    let stats = a.session.run("review.wordCount", &json!({})).unwrap();
    let counts = a.cached_status_counts();
    assert_eq!(counts.words, stats["words"].as_u64().unwrap_or_default() as usize, "words, as the dialog counts them");
    assert_eq!(counts.chars, stats["charactersWithSpaces"].as_u64().unwrap_or_default() as usize, "characters with spaces");
    assert_eq!(counts.chars_no_spaces, stats["characters"].as_u64().unwrap_or_default() as usize, "characters without spaces");
    // "world." selected: its own count of the chosen unit over the document's total.
    a.run("select.text", json!({"text": "world."})).unwrap();
    let line = |unit: StatusCount| {
        let selected = crate::count_in_unit(&a.session.selected_text(), unit);
        status_count_text(unit, counts.in_unit(unit), Some(selected))
    };
    assert_eq!(line(StatusCount::Words), "1 of 4 words");
    assert_eq!(line(StatusCount::Characters), "6 of 23 characters");
    assert_eq!(line(StatusCount::CharactersNoSpaces), "6 of 21 characters (no spaces)");
}

/// The status bar's actions, through its menus by name: the count row sets the unit and is kept between
/// runs, Overtype switches back to inserting.
#[test]
fn the_status_bar_actions_run_their_commands() {
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Hello world.\nSecond line"})).unwrap();
    a.session.prefs.overtype = true;
    let mut h = harness(a);
    assert_eq!(h.state().ui.status_count, StatusCount::Words);
    let at = h.get_by_label("4 words").rect().center();
    click_at(&mut h, at, egui::PointerButton::Secondary);
    h.get_by_label("Characters (with spaces)").click();
    crate::test_harness::frames(&mut h, 3);
    assert_eq!(h.state().ui.status_count, StatusCount::Characters, "the menu's choice is taken");
    assert!(h.query_by_label("23 characters").is_some(), "the count changes at once");
    h.get_by_label("Overtype").click();
    crate::test_harness::frames(&mut h, 3);
    assert!(!h.state().session.prefs.overtype, "back to inserting");
    // The choice comes back after a restart.
    let saved = serde_json::to_vec(&h.state().prefs()).unwrap();
    let mut b = new_app();
    b.apply_prefs(serde_json::from_slice::<crate::prefs::Prefs>(&saved).unwrap());
    assert_eq!(b.ui.status_count, StatusCount::Characters, "kept between runs");
}

/// The status bar's "editing a non-body story" line named every one of them "Editing
/// header/footer", footnotes and endnotes included — it only ever checked "not the body,
/// not a text box", with no regard for which kind of part it actually was.
#[test]
fn status_bar_names_the_actual_part_being_edited() {
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("references.footnote", json!({})).unwrap();
    let mut h = harness(a);
    assert!(h.query_by_label("Editing footnote").is_some());
    h.state_mut().run("caret.docEnd", json!({})).unwrap();
    h.state_mut().run("references.endnote", json!({})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    assert!(h.query_by_label("Editing endnote").is_some());
    h.state_mut().run("insert.createHeader", json!({})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    assert!(h.query_by_label("Editing header/footer").is_some());
}

/// #5124: in the integrated frame the title bar moves the window whatever is open over it (the Font dialog,
/// the File page), its title stays clear of the caption buttons, and Close asks the OS to close the window,
/// which goes through the unsaved-documents guard, instead of closing anything itself.
#[test]
fn the_integrated_title_bar_moves_and_closes_the_window() {
    use egui::accesskit::Role;
    let mut h = integrated_harness(new_app());
    let title = title_rect(&h);
    assert!(h.query_by_role_and_label(Role::Button, "Close").is_some(), "the caption buttons are drawn");
    for caption in ["Minimize", "Maximize", "Restore", "Close"] {
        if let Some(button) = h.query_by_role_and_label(Role::Button, caption) {
            assert!(!button.rect().intersects(title), "the title is clear of {caption}");
        }
    }
    // A free point of the bar beside the title.
    let at = egui::pos2(title.max.x + 8.0, title.center().y);

    for (command, params) in [("ui.dialog", json!({"name": "font"})), ("ui.backstage", json!({"value": true}))] {
        let mut h = integrated_harness(new_app());
        h.state_mut().run(command, params).unwrap();
        crate::test_harness::frames(&mut h, 4);
        assert!(drag_moves_the_window(&mut h, at), "{command}: the window still moves by its title bar");
    }

    h.get_by_role_and_label(Role::Button, "Close").click();
    let sent = commands_over(&mut h, 3);
    assert!(sent.iter().any(|c| matches!(c, egui::ViewportCommand::Close)), "Close asks the OS to close the window");
    assert!(h.state().has_document(), "and closes nothing by itself");
}

/// Under the OS's own title bar (a native frame) the app's bar still names the document in its middle, as the old
/// bar did: the title is drawn as its Rename Document button, not only given as the bar's accessible name.
#[test]
fn the_title_shows_in_a_native_frame() {
    use craft::ui::shell::Frame;
    use craft::window::policy::{Decision, Desktop, DesktopKind};
    use egui::accesskit::Role;
    let spec = || {
        let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
        crate::test_harness::install_spec().frame(Decision { frame: Frame::Native, overridden: None, desktop })
    };
    let mut h = crate::test_harness::builder_with(spec).with_size(egui::vec2(1440.0, 900.0)).build_ui_state(app_frame, new_app());
    crate::test_harness::frames(&mut h, 6);
    let shown = h.query_all_by_label_contains("Not saved").any(|n| n.accesskit_node().role() == Role::Button);
    assert!(shown, "the document title is drawn in the bar");
}

/// #1945: Hide Quick Access Toolbar, the last item of the toolbar's menu, takes the toolbar's
/// buttons and its … button out of the title bar, which keeps its title; the command brings
/// them back.
#[test]
fn hide_quick_access_toolbar_takes_the_toolbar_out_of_the_title_bar() {
    let mut h = harness(new_app());
    assert!(h.query_by_label("Customize Quick Access Toolbar").is_some(), "the … button is drawn");
    let at = h.get_by_label("Customize Quick Access Toolbar").rect().center();
    click_at(&mut h, at, egui::PointerButton::Primary);
    h.get_by_label("Hide Quick Access Toolbar").click();
    crate::test_harness::frames(&mut h, 3);
    assert!(!h.state().ui.quick_access_toolbar, "the menu's last item hides the toolbar");
    assert!(h.query_by_label("Hide Quick Access Toolbar").is_none(), "and closes the menu");
    assert!(h.query_by_label("Customize Quick Access Toolbar").is_none(), "so is the … button");
    assert!(h.query_all_by_label_contains("Not saved").count() > 0, "the title is still drawn");
    h.state_mut().run("ui.quickAccessToolbar", json!({"value": true})).unwrap();
    crate::test_harness::frames(&mut h, 3);
    assert!(h.query_by_label("Customize Quick Access Toolbar").is_some(), "the … button is back");
}

#[test]
fn the_font_box_warns_where_the_status_bar_does_and_before_typing() {
    let mut a = new_app();
    let missing = "Wordcraft Nowhere Face";
    // An empty paragraph in a missing font: no letters show it yet, still worth a warning.
    a.session.run("select.all", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": missing})).unwrap();
    assert!(a.font_box_warning(missing).is_some(), "an empty paragraph warns");
    // Text in it: the status bar's list decides.
    a.session.run("text.insert", &json!({"text": "Hello"})).unwrap();
    let l = a.session.layout();
    let (list, _) = a.cached_missing_fonts(&l);
    assert!(list.iter().any(|f| f.font == missing), "{list:?}");
    assert!(a.font_box_warning(missing).is_some());
    // A missing font just picked at the end of text in an installed one: the next letter will be
    // drawn in a substitute, so the box warns before it is typed.
    a.session.run("select.all", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": "JetBrains Mono"})).unwrap();
    a.session.run("caret.docEnd", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": "Wordcraft Just Picked"})).unwrap();
    assert!(a.font_box_warning("Wordcraft Just Picked").is_some(), "picked at the caret");
    // Over a selection the box shows the selected text's font, which the list decides.
    a.session.run("select.all", &json!({})).unwrap();
    assert!(a.font_box_warning("Wordcraft Half Typed").is_none());
}

#[test]
fn the_east_asian_font_box_names_the_face_its_text_is_drawn_in() {
    // MS Mincho is roman in many font tables; the run's font is drawn in Cambria then, but CJK
    // text never is: it goes to the run's font or a fallback face.
    let mincho = "Wordcraft Mincho Gone";
    let mut doc = wordcraft_doc::Document::new();
    doc.settings.roman_fonts = vec![mincho.into()];
    let mut a = WordApp::new(wordcraft_engine::Session::new(doc), crate::Services::default());
    let want = wordcraft_engine::cmd::font::east_asian_shown_as(&a.session.doc, mincho, "JetBrains Mono");
    assert!(want.is_some(), "not installed");
    assert_eq!(a.east_asian_font_box_warning(mincho, "JetBrains Mono"), want, "an empty paragraph warns");
}

#[test]
fn the_font_dialog_warns_for_a_missing_font_over_a_selection() {
    // Selected Latin text: no East Asian text shows the Asian font, and none shows a font just
    // typed into the dialog, so the list has neither; OK would apply them all the same.
    let mut a = new_app();
    a.session.run("text.insert", &json!({"text": "Hello"})).unwrap();
    a.session.run("select.all", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": "JetBrains Mono"})).unwrap();
    assert!(a.east_asian_font_box_warning("Wordcraft Mincho Gone", "JetBrains Mono").is_some(), "the Asian box");
    assert!(a.font_dialog_warning("Wordcraft Dialog Gone").is_some(), "the Latin box");
    assert!(a.font_dialog_warning("JetBrains Mono").is_none(), "an installed font");
    // The ribbon's box over the selection shows the selected text's font: the list decides.
    assert!(a.font_box_warning("Wordcraft Dialog Gone").is_none());
}

#[test]
fn the_asian_box_names_the_face_cjk_text_gets_even_for_a_font_latin_text_uses() {
    // The status bar lists the font with the face its Latin text is drawn in; Chinese, Japanese
    // and Korean text in it would go to the Latin text font or a fallback face instead.
    let gone = "Wordcraft Both Ways Gone";
    let mut a = new_app();
    a.session.run("text.insert", &json!({"text": "Hello"})).unwrap();
    a.session.run("select.all", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": gone})).unwrap();
    assert!(a.font_box_warning(gone).is_some(), "listed for its Latin text");
    let want = wordcraft_engine::cmd::font::east_asian_shown_as(&a.session.doc, gone, "JetBrains Mono");
    assert!(want.is_some());
    assert_eq!(a.east_asian_font_box_warning(gone, "JetBrains Mono"), want);
}

/// The whole app at `width` × 700 with the interface at `percent`, Discord shown, a selection over a document with
/// Track Changes on, past the frames the fonts need.
fn narrow_harness(width: f32, percent: u16) -> egui_kittest::Harness<'static, WordApp> {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Several words here")), crate::Services::default());
    a.ui.show_discord = true;
    a.run("ui.interfaceSize", json!({"percent": percent})).unwrap();
    a.run("review.trackChanges", json!({"value": true})).unwrap();
    a.run("select.all", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(width, 700.0)).build_ui_state(app_frame, a);
    crate::test_harness::frames(&mut h, 6);
    h
}

/// The controls on the row of `anchor` (its vertical middle inside theirs): (label, rect).
fn controls_on_row_of(h: &egui_kittest::Harness<'static, WordApp>, anchor: egui::Rect) -> Vec<(String, egui::Rect)> {
    use egui::accesskit::Role;
    let roles = [Role::Button, Role::Switch, Role::CheckBox, Role::ComboBox, Role::SearchInput, Role::TextInput, Role::Slider];
    let y = anchor.center().y;
    roles
        .into_iter()
        .flat_map(|role| {
            h.query_all_by_role(role).map(|n| (n.accesskit_node().label().unwrap_or_default().to_string(), n.rect())).collect::<Vec<_>>()
        })
        // A field's editor is an unnamed text input inside the named field (Search, the combo boxes): a part of the
        // control, not another control.
        .filter(|(l, r)| !l.is_empty() && r.width() > 0.0 && r.min.y <= y && y <= r.max.y)
        .collect()
}

/// #5886: down to the 600 pt minimum width (and half-screen split widths, at 100 % and 150 %) the title bar's
/// controls stay inside the window and none covers another: the search gives way instead of covering the Quick
/// Access Toolbar.
#[test]
fn title_bar_controls_never_overlap_in_a_narrow_window() {
    for percent in [100, 150] {
        for width in [crate::window_geometry::MIN_SIZE.x, 640.0, 683.0, 760.0, 1440.0] {
            let h = narrow_harness(width, percent);
            let right = h.ctx.content_rect().max.x;
            // The bar is named by the title it shows (turning Track Changes on edited the document). The title itself
            // gives way first in a narrow window, so the row is the bar's.
            let bar = h.get_by_role_and_label(egui::accesskit::Role::Group, "Document1 \u{2014} Edited").rect();
            let nodes = controls_on_row_of(&h, bar);
            assert!(nodes.iter().any(|(l, _)| l == "Search commands and help"), "{percent}% {width}: {nodes:?}");
            for (i, (la, a)) in nodes.iter().enumerate() {
                assert!(a.max.x <= right + 0.5, "{percent}% {width}: {la} runs off the window");
                for (lb, b) in nodes.iter().skip(i + 1) {
                    assert!(a.max.x <= b.min.x + 0.5 || b.max.x <= a.min.x + 0.5, "{percent}% {width}: {la} {a:?} overlaps {lb} {b:?}");
                }
            }
        }
    }
}

/// The texts the last frame painted on the row of `anchor` (its vertical middle inside theirs), each with the part of
/// it its clip lets show.
fn painted_on_row_of(h: &egui_kittest::Harness<'static, WordApp>, anchor: egui::Rect) -> Vec<(String, egui::Rect)> {
    let y = anchor.center().y;
    h.output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), c.shape.visual_bounding_rect().intersect(c.clip_rect))),
            _ => None,
        })
        .filter(|(_, r)| r.is_positive() && r.min.y <= y && y <= r.max.y)
        .collect()
}

/// #5886: the status bar's texts stop before its controls on the right; in a narrow window the ones that don't fit
/// are left out or shortened instead of drawn over Focus, the views and the zoom. The page is always shown.
///
/// The bar's readouts are members of a framework `Bar`, whose body scrolls under its end region: a member its view
/// shows only in part is not painted (and takes no click) until a scroll brings it in whole, while it keeps its
/// accessibility node and stop so Tab reaches and reveals it. So what is drawn is read from the painted texts, not
/// from the nodes' rects.
#[test]
fn status_bar_texts_stop_before_the_controls_in_a_narrow_window() {
    for percent in [100, 150] {
        for width in [crate::window_geometry::MIN_SIZE.x, 640.0, 683.0, 760.0, 1440.0] {
            let h = narrow_harness(width, percent);
            let focus = h.get_by_role_and_label(egui::accesskit::Role::Button, "Focus").rect();
            let painted = painted_on_row_of(&h, focus);
            assert!(painted.iter().any(|(t, _)| t == "Page 1 of 1"), "{percent}% {width}: the page is always shown: {painted:?}");
            // The controls' own texts: Focus and the zoom percentage (the views and the zoom buttons are icons).
            for (text, r) in painted.iter().filter(|(t, _)| t != "Focus" && !t.ends_with('%')) {
                assert!(r.max.x <= focus.min.x + 0.5, "{percent}% {width}: {text:?} is drawn to {}, past the controls at {}", r.max.x, focus.min.x);
            }
        }
    }
    let wide = narrow_harness(1440.0, 100);
    let focus = wide.get_by_role_and_label(egui::accesskit::Role::Button, "Focus").rect();
    assert!(painted_on_row_of(&wide, focus).iter().any(|(t, _)| t == "Track Changes: On"), "a wide window shows everything");
}

/// Finds the painted track and knob rects inside a rendered `craft::ui::choice::Switch`.
/// `Switch` sets `PARTS = false` and exposes only its `Role::Switch` node and toggled state, so
/// the thumb position is read from the unblurred filled shapes painted inside the switch rect.
fn switch_track_and_knob(shapes: &[egui::epaint::ClippedShape], switch_rect: egui::Rect) -> (egui::Rect, egui::Rect) {
    fn filled_rects(shape: &egui::Shape, inside: egui::Rect, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Vec(children) => children.iter().for_each(|c| filled_rects(c, inside, out)),
            egui::Shape::Rect(r) if r.blur_width == 0.0 && r.fill != egui::Color32::TRANSPARENT && inside.contains_rect(r.rect) => {
                out.push(r.rect);
            }
            _ => {}
        }
    }
    let mut rects = Vec::new();
    for clipped in shapes {
        filled_rects(&clipped.shape, switch_rect.expand(1.0), &mut rects);
    }
    let track = rects
        .iter()
        .copied()
        .filter(|r| r.width() > r.height())
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or_else(|| panic!("expected painted switch track inside {switch_rect:?}: {rects:?}"));
    let knob = rects
        .iter()
        .copied()
        .filter(|r| track.contains_rect(*r) && r.width() < track.width())
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or_else(|| panic!("expected painted switch knob inside track {track:?}: {rects:?}"));
    (track, knob)
}

/// The AutoSave switch renders its knob on the right when on and on the left when off in a
/// left-to-right interface, and mirrors through `WordApp::root_env()` for an Arabic interface.
#[test]
fn the_autosave_knob_sits_right_when_on_and_mirrors_right_to_left() {
    use craft::i18n::Direction;
    use egui::accesskit::{Role, Toggled};

    struct ResetLang;
    impl Drop for ResetLang {
        fn drop(&mut self) {
            crate::i18n::apply_language(crate::i18n::Lang::EN);
        }
    }
    let _reset = ResetLang;

    let mut a = new_app();
    a.ui.animations = crate::theme::Animations::Off;
    a.ui.language = "en".into();
    // Mark the document as saved here without touching disk (`session.dirty` stays false).
    let path = std::path::PathBuf::from("draft.docx");
    a.session.path = Some(path.clone());
    a.autosave_path = Some(path);

    let mut h = harness(a);
    // Arabic UI labels draw even when built without `CRAFT_FONTS_DIR` (embedded Arabic faces).
    h.ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    let mut measure = |lang: crate::i18n::Lang, on: bool| -> (egui::Rect, egui::Rect) {
        h.state_mut().run("ui.language", json!({"value": lang.code()})).unwrap();
        h.state_mut().run("file.autosave", json!({"value": on})).unwrap();
        let want_dir = if lang.rtl() { Direction::Rtl } else { Direction::Ltr };
        assert_eq!(h.state().root_env().direction, want_dir, "root_env supplies direction for {}", lang.code());
        assert_eq!(h.state().autosaves(), on);
        crate::test_harness::frames(&mut h, 4);

        let label = crate::i18n::tr(lang, "AutoSave");
        let node = h.get_by_role_and_label(Role::Switch, label);
        assert_eq!(node.accesskit_node().toggled(), Some(if on { Toggled::True } else { Toggled::False }));
        switch_track_and_knob(&h.output().shapes, node.rect())
    };

    let (ltr_on_track, ltr_on_knob) = measure(crate::i18n::Lang::EN, true);
    let (ltr_off_track, ltr_off_knob) = measure(crate::i18n::Lang::EN, false);
    assert!(ltr_on_knob.center().x > ltr_on_track.center().x, "left to right, on: knob sits right ({ltr_on_knob:?} in {ltr_on_track:?})");
    assert!(ltr_off_knob.center().x < ltr_off_track.center().x, "left to right, off: knob sits left ({ltr_off_knob:?} in {ltr_off_track:?})");

    let Some(ar) = crate::i18n::Lang::from_code("ar") else {
        eprintln!("skipped RTL check: built without the craft-i18n Arabic catalog (set CRAFT_I18N_DIR to a checkout)");
        return;
    };
    let (rtl_on_track, rtl_on_knob) = measure(ar, true);
    let (rtl_off_track, rtl_off_knob) = measure(ar, false);
    assert!(rtl_on_knob.center().x < rtl_on_track.center().x, "right to left, on: knob sits left ({rtl_on_knob:?} in {rtl_on_track:?})");
    assert!(rtl_off_knob.center().x > rtl_off_track.center().x, "right to left, off: knob sits right ({rtl_off_knob:?} in {rtl_off_track:?})");
    assert!(rtl_on_track.center().x > ltr_on_track.center().x, "right to left mirrors the AutoSave switch toward the start (right) edge");
    assert!(((ltr_on_knob.center().x - ltr_on_track.min.x) - (rtl_on_track.max.x - rtl_on_knob.center().x)).abs() <= 0.5);
    assert!(((ltr_off_knob.center().x - ltr_off_track.min.x) - (rtl_off_track.max.x - rtl_off_knob.center().x)).abs() <= 0.5);
}

/// #7311: the title bar's Discord community button keeps its solid accent fill and contrasting
/// `on_strong` icon/label in both wide (pill with label) and narrow (compact icon-only) windows,
/// across Light and Dark themes, and still dispatches `ui.discord` when clicked.
#[test]
fn title_bar_discord_button_keeps_accent_fill_and_icon_contrast_in_wide_and_narrow_windows() {
    use egui::accesskit::Role;

    fn to_color32(rgba: craft::theme::Rgba) -> egui::Color32 {
        egui::Color32::from(craft::theme::Srgba::from(rgba))
    }

    fn collect_shapes<'a>(shapes: &'a [egui::epaint::ClippedShape], out: &mut Vec<&'a egui::Shape>) {
        for clipped in shapes {
            fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
                match shape {
                    egui::Shape::Vec(children) => {
                        for child in children {
                            walk(child, out);
                        }
                    }
                    other => out.push(other),
                }
            }
            walk(&clipped.shape, out);
        }
    }

    for theme in ["light", "dark"] {
        for (width, expect_compact) in [(1440.0_f32, false), (520.0_f32, true)] {
            let mut a = new_app();
            a.ui.show_discord = true;
            a.run("ui.theme", json!({"value": theme})).unwrap();
            let mut h = crate::test_harness::builder().with_size(egui::vec2(width, 700.0)).build_ui_state(app_frame, a);
            crate::test_harness::frames(&mut h, 10);

            let node = h.get_by_role_and_label(Role::Button, "Discord");
            let rect = node.rect();
            assert!(rect.is_positive(), "{theme} {width}: Discord button must be laid out");
            if expect_compact {
                assert!(rect.width() <= rect.height() + 4.0, "{theme} {width}: expected compact icon-only Discord button, got {rect:?}");
            } else {
                assert!(rect.width() > rect.height() + 10.0, "{theme} {width}: expected full labelled Discord pill, got {rect:?}");
            }

            let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
            let accent_strong = to_color32(env.tokens.color.accent.strong);
            let on_strong = to_color32(env.tokens.color.text.on_strong);

            let mut shapes = Vec::new();
            collect_shapes(&h.output().shapes, &mut shapes);

            let bg = shapes.iter().find_map(|s| match s {
                egui::Shape::Rect(r)
                    if (r.rect.min.x - rect.min.x).abs() <= 1.5
                        && (r.rect.max.x - rect.max.x).abs() <= 1.5
                        && (r.rect.min.y - rect.min.y).abs() <= 1.5
                        && (r.rect.max.y - rect.max.y).abs() <= 1.5
                        && r.fill != egui::Color32::TRANSPARENT =>
                {
                    Some(r)
                }
                _ => None,
            });
            let bg = bg.unwrap_or_else(|| panic!("{theme} {width}: expected a filled background rect for Discord button {rect:?}"));
            assert_eq!(bg.fill, accent_strong, "{theme} {width}: Discord button fill should be accent.strong");
            if !expect_compact {
                let min_pill_radius = (rect.height() / 2.0 - 1.5).floor().max(0.0) as u8;
                assert!(
                    bg.corner_radius.nw >= min_pill_radius,
                    "{theme} {width}: wide Discord button should have pill corners (>= {min_pill_radius}), got {:?}",
                    bg.corner_radius
                );
            }

            let icon_tinted_on_strong = shapes.iter().any(|s| match s {
                egui::Shape::Mesh(mesh) => {
                    let bounds = mesh.calc_bounds();
                    rect.expand(1.5).contains_rect(bounds) && !mesh.vertices.is_empty() && mesh.vertices.iter().all(|v| v.color == on_strong)
                }
                _ => false,
            });
            assert!(
                icon_tinted_on_strong,
                "{theme} {width}: Discord icon inside {rect:?} must paint in text.on_strong ({on_strong:?}), not accent ink"
            );

            if !expect_compact {
                let has_label = shapes.iter().any(|s| match s {
                    egui::Shape::Text(t) => rect.expand(1.5).contains_rect(s.visual_bounding_rect()) && t.galley.text() == "Discord",
                    _ => false,
                });
                assert!(has_label, "{theme} {width}: wide Discord pill must paint its \"Discord\" label");
            }

            // Clicking the button dispatches `ui.discord` and opens the community URL.
            h.get_by_role_and_label(Role::Button, "Discord").click();
            let mut opened = None;
            for _ in 0..4 {
                crate::test_harness::step(&mut h);
                for cmd in &h.output().platform_output.commands {
                    if let egui::OutputCommand::OpenUrl(url) = cmd {
                        opened = Some(url.url.clone());
                    }
                }
            }
            assert_eq!(
                opened.as_deref(),
                Some("https://discord.gg/artcraft"),
                "{theme} {width}: clicking the Discord button should open the community URL"
            );
        }
    }
}

/// #7445: the title bar's Discord community button uses the dedicated single-ink `discordPill` icon
/// with `on_strong` contrast over the solid accent pill, while the ribbon's Review › Help › Community
/// Discord button retains the branded `discord` icon with `class="accent"` ink.
#[test]
fn discord_icons_distinguish_title_bar_pill_and_ribbon_accent() {
    use egui::accesskit::Role;

    fn to_color32(rgba: craft::theme::Rgba) -> egui::Color32 {
        egui::Color32::from(craft::theme::Srgba::from(rgba))
    }

    fn collect_shapes<'a>(shapes: &'a [egui::epaint::ClippedShape], out: &mut Vec<&'a egui::Shape>) {
        for clipped in shapes {
            fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
                match shape {
                    egui::Shape::Vec(children) => {
                        for child in children {
                            walk(child, out);
                        }
                    }
                    other => out.push(other),
                }
            }
            walk(&clipped.shape, out);
        }
    }

    for theme in ["light", "dark"] {
        let mut a = new_app();
        a.ui.show_discord = true;
        a.ui.language = "en".into();
        a.run("ui.theme", json!({"value": theme})).unwrap();
        a.run("ui.tab", json!({"tab": "Help"})).unwrap();

        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 700.0)).build_ui_state(app_frame, a);
        crate::test_harness::frames(&mut h, 6);

        let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
        let on_strong = to_color32(env.tokens.color.text.on_strong);
        let accent_solid = to_color32(env.tokens.color.accent.solid);

        let mut shapes = Vec::new();
        collect_shapes(&h.output().shapes, &mut shapes);

        // 1. Title-bar button uses discordPill icon (single-ink text.on_strong).
        let title_bar_node = h.get_by_role_and_label(Role::Button, "Discord");
        let title_bar_rect = title_bar_node.rect();
        assert!(title_bar_rect.is_positive(), "{theme}: Title-bar Discord button must be laid out");

        let title_bar_icon_on_strong = shapes.iter().any(|s| match s {
            egui::Shape::Mesh(mesh) => {
                let bounds = mesh.calc_bounds();
                title_bar_rect.expand(1.5).contains_rect(bounds) && !mesh.vertices.is_empty() && mesh.vertices.iter().all(|v| v.color == on_strong)
            }
            _ => false,
        });
        assert!(title_bar_icon_on_strong, "{theme}: Title-bar Discord icon inside {title_bar_rect:?} must paint in text.on_strong ({on_strong:?})");

        // 2. Ribbon Help › Community Discord button uses discord icon with class="accent".
        let ribbon_node = h
            .query_all_by_role(Role::Button)
            .find(|n| n.accesskit_node().label().is_some_and(|l| l.contains("Discord")) && n.rect() != title_bar_rect)
            .expect("Ribbon Community Discord button");
        let ribbon_rect = ribbon_node.rect();
        assert!(ribbon_rect.is_positive(), "{theme}: Ribbon Community Discord button must be laid out");

        let ribbon_icon_accent = shapes.iter().any(|s| match s {
            egui::Shape::Mesh(mesh) => {
                let bounds = mesh.calc_bounds();
                ribbon_rect.expand(1.5).contains_rect(bounds) && !mesh.vertices.is_empty() && mesh.vertices.iter().all(|v| v.color == accent_solid)
            }
            _ => false,
        });
        assert!(ribbon_icon_accent, "{theme}: Ribbon Community Discord icon inside {ribbon_rect:?} must paint in accent.solid ({accent_solid:?})");
    }
}

/// #7319: view mode toggle buttons (Read Mode, Print Layout, Web Layout) in the status bar are grouped
/// with `Gap::Item` intra-group spacing, separated from Focus and Zoom controls via `Gap::Section`.
/// The Zoom slider has size `ControlSize::Md`, giving its thumb a 14 pt grab affordance matching sibling `IconButton`s.
#[test]
fn status_bar_view_mode_toggles_and_zoom_slider_proportions() {
    use egui::accesskit::Role;
    let h = harness(new_app());

    let focus = h.get_by_role_and_label(Role::Button, "Focus").rect();
    let read_mode = h.get_by_role_and_label(Role::Button, "Read Mode").rect();
    let print_layout = h.get_by_role_and_label(Role::Button, "Print Layout").rect();
    let web_layout = h.get_by_role_and_label(Role::Button, "Web Layout").rect();
    let zoom_out = h.get_by_role_and_label(Role::Button, "Zoom Out").rect();
    let slider = h.get_by_role_and_label(Role::Slider, "Zoom").rect();
    let zoom_in = h.get_by_role_and_label(Role::Button, "Zoom In").rect();

    let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
    let gap_section = env.tokens.space.gap.section;
    let gap_item = env.tokens.space.gap.item;
    let gap_row = env.tokens.space.gap.row;
    let glyph_md = env.tokens.size.icon.glyph_md;

    // 1. Spacing and grouping:
    // Focus -> View modes group gap is Gap::Section.
    let focus_to_views_gap = read_mode.min.x - focus.max.x;
    assert!(
        (focus_to_views_gap - gap_section).abs() <= 0.5,
        "Focus and Read Mode should be separated by Gap::Section ({gap_section} pt), got {focus_to_views_gap}"
    );

    // Intra-group spacing between view mode toggle buttons is Gap::Item.
    let read_to_print_gap = print_layout.min.x - read_mode.max.x;
    let print_to_web_gap = web_layout.min.x - print_layout.max.x;
    assert!(
        (read_to_print_gap - gap_item).abs() <= 0.5,
        "Read Mode and Print Layout intra-group spacing should be Gap::Item ({gap_item} pt), got {read_to_print_gap}"
    );
    assert!(
        (print_to_web_gap - gap_item).abs() <= 0.5,
        "Print Layout and Web Layout intra-group spacing should be Gap::Item ({gap_item} pt), got {print_to_web_gap}"
    );

    // View modes group -> Zoom controls group gap is Gap::Section.
    let views_to_zoom_gap = zoom_out.min.x - web_layout.max.x;
    assert!(
        (views_to_zoom_gap - gap_section).abs() <= 0.5,
        "Web Layout and Zoom Out should be separated by Gap::Section ({gap_section} pt), got {views_to_zoom_gap}"
    );

    // Zoom controls intra-group spacing is default row gap (Gap::Row).
    let zoom_out_to_slider_gap = slider.min.x - zoom_out.max.x;
    let slider_to_zoom_in_gap = zoom_in.min.x - slider.max.x;
    assert!(
        (zoom_out_to_slider_gap - gap_row).abs() <= 0.5,
        "Zoom Out and Zoom Slider intra-group spacing should be Gap::Row ({gap_row} pt), got {zoom_out_to_slider_gap}"
    );
    assert!(
        (slider_to_zoom_in_gap - gap_row).abs() <= 0.5,
        "Zoom Slider and Zoom In intra-group spacing should be Gap::Row ({gap_row} pt), got {slider_to_zoom_in_gap}"
    );

    // 2. Zoom slider thumb proportions:
    // Slider thumb is rendered as a round knob sized at glyph_md (14 pt) for ControlSize::Md.
    fn collect_shapes<'a>(shapes: &'a [egui::epaint::ClippedShape], out: &mut Vec<&'a egui::Shape>) {
        for clipped in shapes {
            fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
                match shape {
                    egui::Shape::Vec(children) => {
                        for child in children {
                            walk(child, out);
                        }
                    }
                    other => out.push(other),
                }
            }
            walk(&clipped.shape, out);
        }
    }

    let mut shapes = Vec::new();
    collect_shapes(&h.output().shapes, &mut shapes);

    let has_14pt_knob = shapes.iter().any(|s| match s {
        egui::Shape::Rect(r) => {
            let rect = r.rect;
            slider.expand(2.0).contains_rect(rect) && (rect.width() - glyph_md).abs() <= 0.5 && (rect.height() - glyph_md).abs() <= 0.5
        }
        _ => false,
    });
    assert!(has_14pt_knob, "Zoom slider should paint a {glyph_md} pt thumb (ControlSize::Md) matching 14 pt IconButton icons inside {slider:?}");
}
