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
<<<<<<< HEAD
=======

/// The AutoSave knob is at the right when the switch is on and at the left when it is off; a
/// right-to-left interface mirrors that.
#[test]
fn the_autosave_knob_sits_right_when_on_and_mirrors_right_to_left() {
    let track = Rect::from_min_max(egui::pos2(100.0, 0.0), egui::pos2(130.0, 16.0));
    assert_eq!(autosave_knob_x(false, true, track), 122.0, "left to right, on: right");
    assert_eq!(autosave_knob_x(false, false, track), 108.0, "left to right, off: left");
    assert_eq!(autosave_knob_x(true, true, track), 108.0, "right to left, on: left");
    assert_eq!(autosave_knob_x(true, false, track), 122.0, "right to left, off: right");
}

/// #5886: in a narrow window the title bar's search box gives up width first, then Discord its
/// label, then the search box its text; a wide window keeps them as they were.
#[test]
fn the_title_bar_controls_fit_a_narrow_window() {
    let fit = |room| TitleBarFit::new(room, true, 26.0);
    assert_eq!(fit(1000.0), TitleBarFit { search: 260.0, discord_label: true });
    assert_eq!(fit(300.0), TitleBarFit { search: 208.0, discord_label: true });
    assert_eq!(fit(200.0), TitleBarFit { search: 170.0, discord_label: false });
    assert_eq!(fit(120.0), TitleBarFit { search: 26.0, discord_label: false });
    assert_eq!(TitleBarFit::new(200.0, false, 26.0).search, 200.0, "without Discord the search box has its room");
}

/// The labelled controls drawn by `draw` in a window `width` wide at `scale`: (label, left, right).
fn labelled_scaled(a: &mut WordApp, width: f32, scale: f32, draw: fn(&mut WordApp, &mut Ui)) -> Vec<(String, f64, f64)> {
    let ctx = ctx();
    if (scale - 1.0).abs() > 0.001 {
        ctx.set_zoom_factor(scale);
    }
    ctx.enable_accesskit();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 700.0))), ..Default::default() };
    let mut out1 = ctx.run_ui(input.clone(), |ui| draw(a, ui));
    out1.textures_delta.clear();
    let mut out = ctx.run_ui(input, |ui| draw(a, ui));
    out.textures_delta.clear();
    let update = out.platform_output.accesskit_update.take().unwrap();
    update
        .nodes
        .iter()
        .filter_map(|(_, node)| Some((node.label()?.to_string(), node.bounds()?)))
        .filter(|(_, b)| b.width() > 0.0)
        .map(|(l, b)| (l, b.x0, b.x1))
        .collect()
}

/// The text strings painted by `draw` in a window `width` wide after settling controls width.
fn painted_texts(a: &mut WordApp, width: f32, draw: fn(&mut WordApp, &mut Ui)) -> Vec<String> {
    /// Recursively collects painted text strings from egui shapes.
    fn extract(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => {
                for s in v {
                    extract(s, out);
                }
            }
            _ => {}
        }
    }
    let ctx = ctx();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 700.0))), ..Default::default() };
    let mut out1 = ctx.run_ui(input.clone(), |ui| draw(a, ui));
    out1.textures_delta.clear();
    let mut out = ctx.run_ui(input, |ui| draw(a, ui));
    out.textures_delta.clear();
    let mut texts = Vec::new();
    for clipped in &out.shapes {
        extract(&clipped.shape, &mut texts);
    }
    texts
}

/// #5886: down to the new minimum width the title bar's controls don't overlap: the search box
/// narrows instead of covering the Quick Access Toolbar. Parametrized at the exact 600 pt minimum,
/// half-screen split widths (640 pt at 150% scale, 683 pt), and across 1.0x and 1.5x scale factors.
#[test]
fn title_bar_controls_never_overlap_in_a_narrow_window() {
    for scale in [1.0, 1.5] {
        for width in [crate::window_geometry::MIN_SIZE.x, 640.0, 680.0, 760.0, 1440.0] {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            a.ui.show_discord = true;
            let nodes = labelled_scaled(&mut a, width, scale, title_bar);
            assert!(nodes.iter().any(|(l, ..)| l == "Search commands and help"), "{scale}x {width}: {nodes:?}");
            for (i, (la, a0, a1)) in nodes.iter().enumerate() {
                assert!(*a1 <= f64::from(width) + 0.5, "{scale}x {width}: {la} runs off the window");
                for (lb, b0, b1) in nodes.iter().skip(i + 1) {
                    assert!(a1 <= b0 || b1 <= a0, "{scale}x {width}: {la} ({a0}–{a1}) overlaps {lb} ({b0}–{b1})");
                }
            }
        }
    }
}

/// #5886 (review): when search is active in the title bar at narrow widths down to the minimum
/// width (and at icon-only search widths), the active text editor stays within the fitted width
/// without expanding or overlapping surrounding controls (Quick Access Toolbar or Discord).
#[test]
fn title_bar_controls_never_overlap_when_search_is_active_in_a_narrow_window() {
    for scale in [1.0f32, 1.5f32] {
        // Test narrow icon-only widths (e.g. 400 pt where TitleBarFit returns control_h) as well
        // as the 600 pt split minimum and wider split widths.
        for width in [400.0, 450.0, crate::window_geometry::MIN_SIZE.x, 640.0, 680.0, 760.0, 1440.0] {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
            a.ui.show_discord = true;
            let ctx = ctx();
            if (scale - 1.0).abs() > 0.001 {
                ctx.set_zoom_factor(scale);
            }
            ctx.enable_accesskit();
            a.ctx = Some(ctx.clone());
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 700.0))), ..Default::default() };
            // Frame 1: draw title bar to record search box presence.
            let mut out1 = ctx.run_ui(input.clone(), |ui| title_bar(&mut a, ui));
            out1.textures_delta.clear();
            assert!(open_title_search(&mut a), "search opens in the title bar box");
            // Frame 2: draw title bar with search active.
            let mut out = ctx.run_ui(input, |ui| title_bar(&mut a, ui));
            out.textures_delta.clear();
            assert!(title_search_open(&a, &ctx), "search is active in frame 2");
            let update = out.platform_output.accesskit_update.take().unwrap();
            let nodes: Vec<(String, f64, f64)> = update
                .nodes
                .iter()
                .filter_map(|(_, node)| Some((node.label()?.to_string(), node.bounds()?)))
                .filter(|(_, b)| b.width() > 0.0)
                .map(|(l, b)| (l, b.x0, b.x1))
                .collect();
            assert!(nodes.iter().any(|(l, ..)| l == "Search commands and help"), "{scale}x {width}: {nodes:?}");
            for (i, (la, a0, a1)) in nodes.iter().enumerate() {
                assert!(*a1 <= f64::from(width) + 0.5, "{scale}x {width}: {la} runs off the window");
                for (lb, b0, b1) in nodes.iter().skip(i + 1) {
                    assert!(a1 <= b0 || b1 <= a0, "{scale}x {width}: active search {la} ({a0}–{a1}) overlaps {lb} ({b0}–{b1})");
                }
            }
        }
    }
}

/// #5886: the status bar's texts stop before its controls on the right; in a narrow window the
/// ones that don't fit are left out or shortened instead of being drawn over Focus, the views and the zoom.
/// Parametrized across exact 600 pt minimum, split widths, and 1.0x and 1.5x scale factors.
#[test]
fn status_bar_texts_stop_before_the_controls_in_a_narrow_window() {
    let controls = ["Focus", "Read Mode", "Print Layout", "Web Layout", "Zoom Out", "Zoom", "Zoom In"];
    for scale in [1.0, 1.5] {
        for width in [crate::window_geometry::MIN_SIZE.x, 640.0, 680.0, 760.0, 1440.0] {
            let mut a =
                WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Several words here")), crate::Services::default());
            a.run("review.trackChanges", json!({"value": true})).unwrap();
            a.run("select.all", json!({})).unwrap();
            let nodes = labelled_scaled(&mut a, width, scale, status_bar);
            let focus = nodes.iter().find(|(l, ..)| l == "Focus").unwrap_or_else(|| panic!("{scale}x {width}: Focus is drawn")).1;
            assert!(nodes.iter().any(|(l, ..)| l == "Page 1 of 1"), "{scale}x {width}: the page is always shown: {nodes:?}");
            for (label, _, right) in nodes.iter().filter(|(l, ..)| !controls.contains(&l.as_str()) && !l.ends_with('%')) {
                assert!(*right <= focus, "{scale}x {width}: {label} ends at {right}, past the controls at {focus}");
            }
            if width >= 1000.0 && scale <= 1.0 {
                assert!(nodes.iter().any(|(l, ..)| l == "Track Changes: On"), "a wide window shows everything: {nodes:?}");
            }
        }
    }
}

/// #5886: when a language label is too long to fit before the status bar controls in a narrow
/// window, it falls back to "…" while preserving clickability to open the language menu,
/// and its tooltip and accessibility label retain the full language name.
#[test]
fn status_bar_language_falls_back_to_ellipsis_in_a_narrow_window() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Several words here")), crate::Services::default());
    a.run("review.trackChanges", json!({"value": true})).unwrap();
    a.run("select.all", json!({})).unwrap();
    std::sync::Arc::make_mut(&mut a.session.doc.styles).default_chr.lang = Some("en-US".into());

    let narrow = painted_texts(&mut a, crate::window_geometry::MIN_SIZE.x, status_bar);
    assert!(narrow.iter().any(|t| t == "…"), "narrow window paints ellipsis for language: {narrow:?}");
    assert!(!narrow.iter().any(|t| t == "English (United States)"), "narrow window does not paint full language name: {narrow:?}");

    let wide = painted_texts(&mut a, 1440.0, status_bar);
    assert!(wide.iter().any(|t| t == "English (United States)"), "wide window paints full language name: {wide:?}");
    assert!(!wide.iter().any(|t| t == "…"), "wide window does not paint ellipsis: {wide:?}");

    let ctx = ctx();
    ctx.enable_accesskit();
    let mut n = 0;
    let mut drawn = |a: &mut WordApp, events: Vec<egui::Event>| -> Vec<(String, egui::Rect)> {
        n += 1;
        let input = egui::RawInput {
            time: Some(f64::from(n) / 60.0),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(crate::window_geometry::MIN_SIZE.x, 700.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| status_bar(a, ui));
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.take().unwrap();
        update
            .nodes
            .into_iter()
            .filter_map(|(_, node)| {
                let b = node.bounds()?;
                Some((node.label()?.to_string(), egui::Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))))
            })
            .collect()
    };

    // Frame 1: Accessibility label retains the full language name.
    let seen = drawn(&mut a, vec![]);
    let lang_node = seen.iter().find(|(l, _)| l == "English (United States)");
    assert!(lang_node.is_some(), "narrow window maintains full language name in accessibility label: {seen:?}");
    let lang_at = lang_node.unwrap().1.center();

    // Hover over the button: tooltip contains the full language name.
    let hovered = drawn(&mut a, vec![egui::Event::PointerMoved(lang_at)]);
    assert!(
        hovered.iter().any(|(l, _)| l.contains("English (United States)")),
        "hovering the ellipsized language button shows tooltip with the full language name: {hovered:?}"
    );

    // Click the button: the menu opens with proofing options.
    let click_down = egui::Event::PointerButton { pos: lang_at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() };
    let click_up = egui::Event::PointerButton { pos: lang_at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    drawn(&mut a, vec![click_down]);
    drawn(&mut a, vec![click_up]);
    let clicked = drawn(&mut a, vec![]);
    assert!(
        clicked.iter().any(|(l, _)| l == "Set Proofing Language…"),
        "clicking the ellipsized language button opens the language menu: {clicked:?}"
    );
}

/// #5886: each native window has its own cached status bar controls width in egui's temporary data,
/// so drawing multiple windows with different widths does not overwrite each other or trigger
/// `request_discard` redraw loops.
#[test]
fn status_bar_controls_width_cache_is_per_window_and_avoids_discard_loops() {
    let ctx = ctx();
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello")), crate::Services::default());

    let win1 = a.windows.shown();
    a.new_window();
    let win2 = a.windows.shown();
    assert_ne!(win1, win2, "two distinct windows open");

    let id2 = a.window_id("status_controls_width");
    assert!(a.enter_window(win1));
    let id1 = a.window_id("status_controls_width");
    assert_ne!(id1, id2, "per-window cache keys are distinct");

    // Frame 1: Window 1 at 600 pt (narrow), Window 2 at 1200 pt (wide).
    // The first pass populates each window's cache.
    assert!(a.enter_window(win1));
    ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 700.0))), ..Default::default() }, |ui| {
        status_bar(&mut a, ui);
    })
    .drop_without_applying_deltas();

    assert!(a.enter_window(win2));
    ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 700.0))), ..Default::default() }, |ui| {
        status_bar(&mut a, ui);
    })
    .drop_without_applying_deltas();

    let cached1: Option<f32> = ctx.data(|d| d.get_temp(id1));
    let cached2: Option<f32> = ctx.data(|d| d.get_temp(id2));
    assert!(cached1.is_some(), "window 1 controls width is cached");
    assert!(cached2.is_some(), "window 2 controls width is cached");

    // Frame 2 and subsequent frames: rendering both windows alternating does NOT trigger discard loops.
    // Each run_ui must complete in exactly 1 pass (0 discards requested).
    for frame in 2..=4 {
        assert!(a.enter_window(win1));
        let mut passes1 = 0;
        ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 700.0))), ..Default::default() },
            |ui| {
                status_bar(&mut a, ui);
                passes1 += 1;
            },
        )
        .drop_without_applying_deltas();
        assert_eq!(passes1, 1, "frame {frame} window 1 settled without discard loop");

        assert!(a.enter_window(win2));
        let mut passes2 = 0;
        ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 700.0))), ..Default::default() },
            |ui| {
                status_bar(&mut a, ui);
                passes2 += 1;
            },
        )
        .drop_without_applying_deltas();
        assert_eq!(passes2, 1, "frame {frame} window 2 settled without discard loop");
    }

    // Both window caches remain intact and separate.
    assert_eq!(ctx.data(|d| d.get_temp(id1)), cached1);
    assert_eq!(ctx.data(|d| d.get_temp(id2)), cached2);
}

/// #5886: when rendering the first pass with an empty cache and max_passes = 1 (where egui declines
/// request_discard), status bar texts never overlap the right-hand controls across narrow window widths.
#[test]
fn status_bar_texts_never_overlap_controls_on_cache_miss_with_single_pass() {
    let controls = ["Focus", "Read Mode", "Print Layout", "Web Layout", "Zoom Out", "Zoom", "Zoom In"];
    for scale in [1.0f32, 1.5f32] {
        for width in [crate::window_geometry::MIN_SIZE.x, 640.0, 680.0, 760.0, 1440.0] {
            let mut a =
                WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Several words here")), crate::Services::default());
            a.run("review.trackChanges", json!({"value": true})).unwrap();
            a.run("select.all", json!({})).unwrap();

            let ctx = ctx();
            if (scale - 1.0f32).abs() > 0.001 {
                ctx.set_zoom_factor(scale);
            }
            ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(1).unwrap());
            ctx.enable_accesskit();

            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 700.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| status_bar(&mut a, ui));
            out.textures_delta.clear();
            let update = out.platform_output.accesskit_update.take().unwrap();
            let nodes: Vec<(String, f64, f64)> = update
                .nodes
                .iter()
                .filter_map(|(_, node)| Some((node.label()?.to_string(), node.bounds()?)))
                .filter(|(_, b)| b.width() > 0.0)
                .map(|(l, b)| (l, b.x0, b.x1))
                .collect();

            let focus = nodes.iter().find(|(l, ..)| l == "Focus").unwrap_or_else(|| panic!("{scale}x {width}: Focus is drawn")).1;
            assert!(nodes.iter().any(|(l, ..)| l == "Page 1 of 1"), "{scale}x {width}: the page is always shown: {nodes:?}");
            for (label, _, right) in nodes.iter().filter(|(l, ..)| !controls.contains(&l.as_str()) && !l.ends_with('%')) {
                assert!(*right <= focus, "{scale}x {width} (pass 1): {label} ends at {right}, past the controls at {focus}");
            }
        }
    }
}
>>>>>>> up/main
