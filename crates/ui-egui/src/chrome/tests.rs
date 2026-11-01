use super::*;

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, crate::theme::Appearance::Light);
    ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
    ctx
}

fn frame(ctx: &egui::Context, n: &mut u32, events: Vec<egui::Event>, a: &mut WordApp) {
    *n += 1;
    let input = egui::RawInput { time: Some(f64::from(*n) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
    ctx.run_ui(input, |ui| title_bar(a, ui)).drop_without_applying_deltas();
}

fn click(ctx: &egui::Context, n: &mut u32, pos: egui::Pos2, a: &mut WordApp) {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(ctx, n, vec![egui::Event::PointerMoved(pos), button(true)], a);
    frame(ctx, n, vec![button(false)], a);
}

/// #701: the arrow beside Undo lists the session's undo steps, newest first; clicking the third
/// row undoes three steps at once.
#[test]
fn undo_list_matches_the_session_and_undoes_up_to_the_clicked_row() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("select.all", json!({})).unwrap();
    a.run("format.bold", json!({})).unwrap();
    a.run("format.italic", json!({})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("text.insert", json!({"text": " world"})).unwrap();
    let session: Vec<String> = a.session.undo_history().into_iter().map(str::to_string).collect();
    assert_eq!(session, ["Typing", "Italic", "Bold", "Typing"]);
    assert_eq!(undo_list_rows(&a), session);
    assert_eq!(undo_list_footer(0), "Cancel");
    assert_eq!(undo_list_footer(1), "Undo 1 Action");
    assert_eq!(undo_list_footer(3), "Undo 3 Actions");

    let (ctx, mut n) = (ctx(), 0);
    frame(&ctx, &mut n, vec![], &mut a);
    let arrow = ctx.read_response(undo_list_button_id()).unwrap();
    let popup = egui::Popup::default_response_id(&arrow);
    click(&ctx, &mut n, arrow.rect.center(), &mut a);
    assert!(egui::Popup::is_id_open(&ctx, popup), "the arrow opens the Undo list");
    frame(&ctx, &mut n, vec![], &mut a);
    for i in 0..4 {
        assert!(ctx.read_response(undo_list_row_id(i)).is_some(), "row {i} is drawn");
    }
    assert!(ctx.read_response(undo_list_row_id(4)).is_none(), "one row per step");
    let third = ctx.read_response(undo_list_row_id(2)).unwrap();
    click(&ctx, &mut n, third.rect.center(), &mut a);
    assert_eq!(a.session.undo_history(), ["Typing"], "three steps undone in one go");
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello");
    frame(&ctx, &mut n, vec![], &mut a);
    assert!(!egui::Popup::is_id_open(&ctx, popup), "the list closes");
}

/// #1975: the zoom percentage gains and loses digits (9% → 100% → 500%); the controls to its
/// left must not shift with it.
#[test]
fn status_bar_controls_hold_still_across_zoom_levels() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let ctx = ctx();
    ctx.enable_accesskit();
    let names = ["Focus", "Print Layout", "Web Layout"];
    let mut seen: Vec<(f32, Vec<f64>)> = Vec::new();
    for (n, zoom) in [10.0_f32, 9.0, 100.0, 500.0].into_iter().enumerate() {
        a.canvas.scale = zoom / 100.0 * crate::canvas::PX_PER_PT;
        let input = egui::RawInput {
            time: Some(n as f64 / 60.0),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| status_bar(&mut a, ui));
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.take().unwrap();
        let xs: Vec<f64> = names
            .iter()
            .map(|name| {
                let node = update.nodes.iter().find(|(_, node)| node.label() == Some(*name)).unwrap_or_else(|| panic!("{name} is drawn")).1.clone();
                node.bounds().unwrap().x0
            })
            .collect();
        seen.push((zoom, xs));
    }
    let first = &seen[0].1;
    for (zoom, xs) in &seen {
        assert_eq!(xs, first, "status bar controls moved at {zoom}% zoom");
    }
}

/// #2797: the count line's wording per unit — words (the default), characters with spaces,
/// characters without spaces — whole document or "selected of total".
#[test]
fn the_count_line_words_each_unit() {
    for (unit, total, selected, expected) in [
        (StatusCount::Words, 12, None, "12 words"),
        (StatusCount::Words, 12, Some(3), "3 of 12 words"),
        (StatusCount::Characters, 23, None, "23 characters"),
        (StatusCount::Characters, 23, Some(6), "6 of 23 characters"),
        (StatusCount::CharactersNoSpaces, 21, None, "21 characters (no spaces)"),
        (StatusCount::CharactersNoSpaces, 21, Some(6), "6 of 21 characters (no spaces)"),
    ] {
        assert_eq!(status_count_text(unit, total, selected), expected, "{unit:?} with {selected:?} selected");
    }
}

/// #2797: the status bar's totals are the Word Count dialog's numbers (words, characters with
/// spaces, characters without spaces), and its selection line counts the selection the same way.
#[test]
fn the_status_bar_totals_are_the_word_count_dialogs_numbers() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
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

/// #2797: the status bar draws the unit the person chose, and right-clicking the count offers
/// the three units, the choice taking effect at once and kept between runs.
#[test]
fn the_status_bar_count_is_chosen_from_its_context_menu() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("text.insert", json!({"text": "Hello world.\nSecond line"})).unwrap();
    let ctx = ctx();
    ctx.enable_accesskit();
    let mut n = 0;
    // What the status bar drew last frame: each widget's label and where it sits.
    let drawn = |a: &mut WordApp, n: &mut u32, events: Vec<egui::Event>| -> Vec<(String, egui::Rect)> {
        *n += 1;
        let input = egui::RawInput {
            time: Some(f64::from(*n) / 60.0),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0))),
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
    let at =
        |seen: &[(String, egui::Rect)], label: &str| seen.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("{label} is drawn")).1.center();
    let button = |at: egui::Pos2, button: egui::PointerButton, pressed: bool| egui::Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers: Default::default(),
    };
    // Words are counted: the default unit.
    let mut seen = drawn(&mut a, &mut n, vec![]);
    assert!(seen.iter().any(|(l, _)| l == "4 words"), "{seen:?}");
    // Right-click the count: the menu lists the three units.
    let count = at(&seen, "4 words");
    drawn(&mut a, &mut n, vec![egui::Event::PointerMoved(count), button(count, egui::PointerButton::Secondary, true)]);
    drawn(&mut a, &mut n, vec![button(count, egui::PointerButton::Secondary, false)]);
    seen = drawn(&mut a, &mut n, vec![]);
    for label in ["Words", "Characters (with spaces)", "Characters (no spaces)"] {
        assert!(seen.iter().any(|(l, _)| l == label), "the menu lists {label}: {seen:?}");
    }
    // Pick characters with spaces: the count changes at once, with no document change.
    let pick = at(&seen, "Characters (with spaces)");
    drawn(&mut a, &mut n, vec![egui::Event::PointerMoved(pick), button(pick, egui::PointerButton::Primary, true)]);
    drawn(&mut a, &mut n, vec![button(pick, egui::PointerButton::Primary, false)]);
    assert_eq!(a.ui.status_count, StatusCount::Characters, "the menu's choice is taken");
    assert_eq!(a.run("ui.statusCount", json!({})).unwrap()["statusCount"], "characters", "and answered to scripts");
    seen = drawn(&mut a, &mut n, vec![]);
    assert!(seen.iter().any(|(l, _)| l == "23 characters"), "{seen:?}");
    // With a selection: "selected of total".
    a.run("select.text", json!({"text": "world."})).unwrap();
    seen = drawn(&mut a, &mut n, vec![]);
    assert!(seen.iter().any(|(l, _)| l == "6 of 23 characters"), "{seen:?}");
    // The choice comes back after a restart.
    let mut b = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    b.apply_prefs(crate::UiState::from_json(&saved).unwrap());
    assert_eq!(b.ui.status_count, StatusCount::Characters, "kept between runs");
}

/// #2402: a long message (the path of a file that isn't there) is shortened so that it stops
/// before the controls on the right, instead of being drawn over the Focus button.
#[test]
fn a_long_status_message_stops_before_the_controls() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let long = "C:\\Test\\Documents\\Reports\\Final individual test folder\\Second level folder\\Project - Gadgets and sleep of teenagers.docx: The system cannot find the path specified. (os error 3)";
    a.status_msg = Some((long.to_string(), crate::now_ms()));
    let ctx = ctx();
    ctx.enable_accesskit();
    let input =
        egui::RawInput { time: Some(0.0), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0))), ..Default::default() };
    let mut whole = 0.0;
    let mut out = ctx.run_ui(input, |ui| {
        status_bar(&mut a, ui);
        // The width the whole message would take, unshortened.
        let t = Tokens::get(ui.ctx());
        let font = TypeRung::Caption.regular();
        let track = TypeRung::Caption.tracking();
        whole = text_width(ui.painter(), long, &font, track, t.accent_text);
    });
    out.textures_delta.clear();
    let update = out.platform_output.accesskit_update.take().unwrap();
    let focus = update.nodes.iter().find(|(_, node)| node.label() == Some("Focus")).unwrap_or_else(|| panic!("Focus is drawn")).1.bounds().unwrap();
    let msg = ctx.read_response(status_message_id()).expect("the message is drawn");
    assert!(msg.rect.width() < whole, "the message is shortened: {:?} of {whole}", msg.rect);
    assert!(f64::from(msg.rect.right()) <= focus.x0, "the message stops before the Focus button: {:?} vs {}", msg.rect, focus.x0);
}

/// The status bar's "editing a non-body story" line named every one of them "Editing
/// header/footer", footnotes and endnotes included — it only ever checked "not the body,
/// not a text box", with no regard for which kind of part it actually was.
#[test]
fn status_bar_names_the_actual_part_being_edited() {
    let label = |a: &mut WordApp| {
        let ctx = ctx();
        ctx.enable_accesskit();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| status_bar(a, ui));
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.take().unwrap();
        for prefix in ["Editing footnote", "Editing endnote", "Editing header/footer"] {
            if update.nodes.iter().any(|(_, node)| node.label().is_some_and(|l| l == prefix)) {
                return prefix.to_string();
            }
        }
        "(none shown)".to_string()
    };
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("references.footnote", json!({})).unwrap();
    assert_eq!(label(&mut a), "Editing footnote");
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("references.endnote", json!({})).unwrap();
    assert_eq!(label(&mut a), "Editing endnote");
    a.run("insert.createHeader", json!({})).unwrap();
    assert_eq!(label(&mut a), "Editing header/footer");
}

/// #1903: the footer's Cancel row closes the Undo list without undoing anything.
#[test]
fn undo_list_cancel_closes_without_undoing() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    let before: Vec<String> = a.session.undo_history().into_iter().map(str::to_string).collect();
    assert!(!before.is_empty());

    let (ctx, mut n) = (ctx(), 0);
    frame(&ctx, &mut n, vec![], &mut a);
    let arrow = ctx.read_response(undo_list_button_id()).unwrap();
    let popup = egui::Popup::default_response_id(&arrow);
    click(&ctx, &mut n, arrow.rect.center(), &mut a);
    assert!(egui::Popup::is_id_open(&ctx, popup), "the arrow opens the Undo list");
    frame(&ctx, &mut n, vec![], &mut a);
    let cancel = ctx.read_response(undo_list_cancel_id()).expect("Cancel is drawn");
    click(&ctx, &mut n, cancel.rect.center(), &mut a);
    let after: Vec<String> = a.session.undo_history().into_iter().map(str::to_string).collect();
    assert_eq!(after, before, "nothing undone");
    frame(&ctx, &mut n, vec![], &mut a);
    assert!(!egui::Popup::is_id_open(&ctx, popup), "Cancel closes the list");
}

#[test]
fn mac_title_bar_centres_and_clears_the_traffic_lights() {
    // The lights are centred in the bar, and content keeps the same gap after them.
    let margin = (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;
    let lights_end = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT;
    assert_eq!(MAC_CONTENT_LEFT - lights_end, margin);
    // `egui::Margin` is in `i8`.
    assert!(MAC_CONTENT_LEFT <= f32::from(i8::MAX));
}

#[test]
fn mac_title_bar_follows_the_measured_lights() {
    // The standard lights give the standard bar.
    let lights_end = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT;
    assert_eq!(MacTitleBar::around_lights(lights_end, MAC_TITLE_BAR / 2.0, MAC_LIGHT), MacTitleBar::default());
    // Larger, lower lights (macOS 26): a taller bar, and content clear of the green button.
    let bar = MacTitleBar::around_lights(78.0, 18.0, 14.0);
    assert_eq!(bar.height, 36.0);
    assert_eq!(bar.content_left, 89.0);
    // Never tighter than the standard layout.
    assert_eq!(MacTitleBar::around_lights(40.0, 10.0, 12.0), MacTitleBar::default());
}

/// #1945: Hide Quick Access Toolbar, the last item of the toolbar's menu, takes the toolbar's
/// buttons and its … button out of the title bar, which keeps its title; the command brings
/// them back.
#[test]
fn hide_quick_access_toolbar_takes_the_toolbar_out_of_the_title_bar() {
    use egui_kittest::kittest::Queryable;
    let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            // The fonts `logic` installs take effect on the next frame, as `WordApp::ui` waits.
            if app.fonts_frames < 2 {
                app.fonts_frames += 1;
                return;
            }
            title_bar(app, ui);
        },
        a,
    );
    h.run_steps(4);
    let drawn = |h: &egui_kittest::Harness<'static, WordApp>, id: &str| h.ctx.read_response(qat_button_id(id)).is_some();
    for id in ["file.save", "edit.undo", "edit.redo"] {
        assert!(drawn(&h, id), "{id} is on the toolbar");
    }
    let at = h.ctx.read_response(qat_menu_button_id()).expect("the … button").rect.center();
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    h.run_steps(2);
    h.get_by_label("Hide Quick Access Toolbar").click();
    // `read_response` also reads the frame before last, so run two frames past the change.
    h.run_steps(3);
    assert!(!h.state().ui.quick_access_toolbar, "the menu's last item hides the toolbar");
    assert!(h.query_by_label("Hide Quick Access Toolbar").is_none(), "and closes the menu");
    for id in ["file.save", "edit.undo", "edit.redo"] {
        assert!(!drawn(&h, id), "{id} is gone");
    }
    assert!(h.ctx.read_response(qat_menu_button_id()).is_none(), "so is the … button");
    assert!(h.query_by_label_contains("Not saved").is_some(), "the title is still drawn");
    h.state_mut().run("ui.quickAccessToolbar", json!({"value": true})).unwrap();
    h.run_steps(2);
    for id in ["file.save", "edit.undo", "edit.redo"] {
        assert!(drawn(&h, id), "{id} is back");
    }
    assert!(h.ctx.read_response(qat_menu_button_id()).is_some());
}

/// #3040: clicking a row in the Undo list hands the keyboard back to the document.
#[test]
fn undo_list_row_returns_focus_to_the_document() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    assert_eq!(a.session.undo_history().len(), 1);

    let (ctx, mut n) = (ctx(), 0);
    frame(&ctx, &mut n, vec![], &mut a);
    let arrow = ctx.read_response(undo_list_button_id()).unwrap();
    let popup = egui::Popup::default_response_id(&arrow);
    click(&ctx, &mut n, arrow.rect.center(), &mut a);
    assert!(egui::Popup::is_id_open(&ctx, popup), "the arrow opens the Undo list");
    frame(&ctx, &mut n, vec![], &mut a);
    let row = ctx.read_response(undo_list_row_id(0)).unwrap();
    a.canvas.want_focus = false;
    click(&ctx, &mut n, row.rect.center(), &mut a);
    assert!(a.canvas.want_focus, "the row click returns focus to the document");
    assert!(a.session.undo_history().is_empty(), "the step was undone");
}

/// A saved .docx (so AutoSave can cover it) with the title bar drawn over it.
fn autosave_harness() -> (egui_kittest::Harness<'static, WordApp>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("wordcraft-ui-autosave-label-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("draft.docx");
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("text.insert", json!({"text": "Hello"})).unwrap();
    a.run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
    assert!(a.autosaves(), "a saved .docx is covered by AutoSave");
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            // The fonts `logic` installs take effect on the next frame, as `WordApp::ui` waits.
            if app.fonts_frames < 2 {
                app.fonts_frames += 1;
                return;
            }
            title_bar(app, ui);
        },
        a,
    );
    h.run_steps(4);
    (h, dir)
}

/// #3370: the AutoSave text is part of the switch, not a caption beside it: clicking the text
/// toggles AutoSave, as clicking the switch does.
#[test]
fn clicking_the_autosave_label_toggles_autosave() {
    use egui_kittest::kittest::Queryable;
    let (mut h, dir) = autosave_harness();
    assert!(h.state().autosaves());
    h.get_by_label("AutoSave").click();
    h.run_steps(2);
    assert!(!h.state().autosaves(), "clicking the text turns AutoSave off");
    h.get_by_label("AutoSave").click();
    h.run_steps(2);
    assert!(h.state().autosaves(), "and clicking it again turns it back on");
    let _ = std::fs::remove_dir_all(dir);
}
