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
