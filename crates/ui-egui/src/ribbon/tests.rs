use super::arrange::group_menu_command;
use super::*;
use wordcraft_engine::Session;

/// Two clicks arm and disarm the painter; the double-click handling then leaves it sticky (#2785).
#[test]
fn double_click_on_format_painter_makes_it_sticky() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    app.run("edit.formatPainter", json!({})).unwrap();
    assert!(matches!(app.session.painter, Some((_, _, false))), "one click arms it for one use");
    app.run("edit.formatPainter", json!({})).unwrap();
    assert!(app.session.painter.is_none(), "the second click of a double-click turns it off");
    painter_double_click(&mut app);
    assert!(matches!(app.session.painter, Some((_, _, true))), "the double-click leaves it sticky");
}

fn png_data() -> String {
    let img = image::RgbaImage::from_fn(20, 10, |x, y| {
        if x > 2 && x < 17 && y > 2 && y < 7 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
    });
    let mut b = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
    wordcraft_engine::cmd::insert::base64_encode(&b)
}

/// The style set gallery fills exactly its own box and the next control starts after a gap,
/// so Colors and Fonts can't overlap the thumbnails (#638).
#[test]
fn design_style_set_gallery_stays_in_its_box() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let mut seen = None;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 200.0)).build_ui(|ui| {
        ui.horizontal_top(|ui| {
            let (gallery, used) = style_set_box(&mut app, ui);
            seen = Some((gallery, used, ui.cursor().min.x));
        });
    });
    h.run_steps(2);
    drop(h);
    let (gallery, used, next_x) = seen.unwrap();
    assert!((gallery.width() - crate::previews::STYLE_SET_GALLERY_W).abs() < 0.01);
    assert!(used.right() <= gallery.right() + 0.01, "{used:?} {gallery:?}");
    assert!(next_x >= gallery.right() + 8.0 - 0.01, "{next_x} {gallery:?}");
}

/// #2738: Home › Underline's drop-down: every line sample is a named entry (assistive
/// technology, tooltips) that applies its own style; Words only, None and Underline Color ›
/// Automatic do what they say.
#[test]
fn underline_menu_entries_apply_their_style() {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::props::Underline;
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
    a.run("select.all", json!({})).unwrap();
    // The app's real interface fonts carry the macOS shortcut symbols (⌘⇧⌥⌃); egui's
    // bare test defaults do not.
    let mut h = egui_kittest::Harness::builder()
        .with_size(vec2(400.0, 700.0))
        .with_context_setup(|ctx| ctx.set_fonts(crate::theme::font_definitions(false, false)))
        .build_ui_state(|ui, app: &mut WordApp| underline_menu(ui, app), a);
    h.run_steps(2);
    let run = |h: &egui_kittest::Harness<'_, WordApp>| h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap().runs[0].props.clone();
    for (_, name) in UNDERLINE_STYLES {
        assert!(h.query_by_label(name).is_some(), "{name} is a named entry");
    }
    // Entries made with `mi` carry their shortcut text in their label (Words only: Ctrl+Shift+W).
    for (name, want) in
        [("Double", Underline::Double), ("Dot-dot-dash", Underline::DotDotDash), ("Wave", Underline::Wave), ("Words only", Underline::Words)]
    {
        h.get_by_label_contains(name).click();
        h.run_steps(2);
        assert_eq!(run(&h).underline, Some(want), "{name}");
    }
    h.get_by_label("None").click();
    h.run_steps(2);
    assert_eq!(run(&h).underline, Some(Underline::None));
    h.get_by_label("Double").click();
    h.run_steps(2);
    // The submenu opens on a click once the pointer is on it.
    let open_colors = |h: &mut egui_kittest::Harness<'_, WordApp>| {
        h.get_by_label("Underline Color").hover();
        h.run();
        h.get_by_label("Underline Color").click();
        h.run();
    };
    open_colors(&mut h);
    // Standard Colors' first swatch (C00000) sits just under its heading; swatches have no label.
    let heading = h.get_by_label("Standard Colors").rect();
    let at = egui::pos2(heading.left() + 7.0, heading.bottom() + 10.0);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert_eq!(run(&h).underline_color, wordcraft_doc::Rgb::parse("C00000"));
    assert_eq!(run(&h).underline, Some(Underline::Double), "a colour keeps the style");
    open_colors(&mut h);
    h.get_by_label_contains("Automatic").click();
    h.run_steps(2);
    assert_eq!(run(&h).underline_color, None);
    assert_eq!(run(&h).underline, Some(Underline::Double));
}

/// #658: menu buttons grey out (and don't open) while the command their menu is about is
/// disabled: the Table Layout menus outside a table; menus with no command stay on.
#[test]
fn menu_buttons_follow_their_command() {
    use crate::widgets::menu_enabled;
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    for id in ["table.selectTable", "table.deleteTable", "table.autofit"] {
        assert!(!menu_enabled(&a, id), "{id} is off outside a table");
    }
    assert!(menu_enabled(&a, ""), "a menu without a command is always on");
    assert!(menu_enabled(&a, "insert.header"));
    a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
    for id in ["table.selectTable", "table.deleteTable", "table.autofit"] {
        assert!(menu_enabled(&a, id), "{id} is on inside a table");
    }
}

/// #4229: with a group selected the Group button stays on, so Ungroup in its menu can be
/// reached; with nothing selected it is off, and with two objects it offers Group.
#[test]
fn the_group_menu_stays_on_for_a_selected_group() {
    use crate::widgets::{enabled, menu_enabled};
    let mut a = crate::WordApp::new(
        Session::new(wordcraft_doc::Document::from_text(
            "one
two
three",
        )),
        crate::Services::default(),
    );
    assert!(!menu_enabled(&a, group_menu_command(&a)), "nothing selected");
    a.run("insert.shape", json!({"kind": "rectangle", "width": 80, "height": 40})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 150})).unwrap();
    a.run("select.collapse", json!({"end": true})).unwrap();
    a.run("insert.shape", json!({"kind": "ellipse", "width": 80, "height": 40})).unwrap();
    a.run("arrange.bounds", json!({"page": 0, "x": 300, "y": 150})).unwrap();
    a.run("select.addObject", json!({"index": 0})).unwrap();
    assert_eq!(group_menu_command(&a), "arrange.group");
    assert!(menu_enabled(&a, group_menu_command(&a)), "two objects to group");
    a.run("arrange.group", json!({})).unwrap();
    assert!(!enabled(&a, "arrange.group"), "one group: nothing to group");
    assert_eq!(group_menu_command(&a), "arrange.ungroup");
    assert!(menu_enabled(&a, group_menu_command(&a)), "the menu reaches Ungroup");
}

/// #1206: Margins, Orientation, Size and Columns mark the caret section's setting, as Word
/// does. Every value differs from a new document's, so a default can't satisfy it.
#[test]
fn page_setup_menus_mark_the_current_setting() {
    use egui_kittest::kittest::{By, NodeT, Queryable};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    app.run("layout.margins", json!({"preset": "narrow"})).unwrap();
    app.run("layout.orientation", json!({"value": "landscape"})).unwrap();
    app.run("layout.size", json!({"name": "A4"})).unwrap();
    app.run("layout.columns", json!({"preset": "right"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 1200.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            margins_items(ui, app);
            orientation_items(ui, app);
            size_menu(ui, app);
            columns_items(ui, app);
        },
        app,
    );
    h.run_steps(2);
    let checked: Vec<String> = h
        .query_all(By::new().predicate(|n| n.toggled() == Some(egui::accesskit::Toggled::True)))
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    let first_words: Vec<&str> = checked.iter().filter_map(|l| l.trim_start_matches('✓').split_whitespace().next()).collect();
    assert_eq!(first_words, ["Narrow", "Landscape", "A4", "Right"], "{checked:?}");
}

/// #860 review: a tag the language list doesn't have shows by language and region, not as
/// the tag; Portuguese (Angola) isn't Brazil's, and no name shows twice in the menu.
#[test]
fn other_regions_show_by_name_once() {
    assert_eq!(language_name(Some("fr-CA")), "French (Canada)");
    assert_eq!(language_name(Some("en-AU")), "English (Australia)");
    assert_eq!(language_name(Some("de_AT")), "German (Austria)");
    assert_eq!(language_name(Some("pt-AO")), "Portuguese (Angola)");
    assert_eq!(language_name(Some("sr-Latn-BA")), "Serbian (Bosnia and Herzegovina)");
    assert_eq!(language_name(Some("en-ZZ")), "English (ZZ)");
    assert_eq!(language_name(Some("xx-YY")), "xx-YY");
    let exact = |cur: &'static str| move |t: &str| t.eq_ignore_ascii_case(cur);
    for tag in ["fr-CA", "pt-AO", "en-SG"] {
        let c = language_checks(tag, exact(tag));
        let name = c.extra.expect("shown above the list");
        assert!(!PROOFING_LANGUAGES.iter().any(|(_, n)| *n == name), "{name} is listed once");
        assert!(!c.listed.contains(&true), "{tag}: nothing in the list checked");
    }
    // A tag named like a list entry checks that entry instead of showing it again.
    let c = language_checks("PT-br-x-test", |_| false);
    assert_eq!(c.extra, None);
    assert_eq!(c.listed, PROOFING_LANGUAGES.map(|(t, _)| t == "pt-BR"));
}

/// The Englishes WordCraft proofs differently are in the menu, checked by their tags, and
/// every English region has a name (#1281, #1045).
#[test]
fn english_dialects_are_in_the_language_menu() {
    for (tag, name) in
        [("en-US", "English (United States)"), ("en-GB", "English (United Kingdom)"), ("en-AU", "English (Australia)"), ("en-CA", "English (Canada)")]
    {
        assert!(PROOFING_LANGUAGES.contains(&(tag, name)), "{tag}");
        assert_eq!(language_name(Some(tag)), name);
        let c = language_checks(&tag.to_ascii_lowercase(), |t| t.eq_ignore_ascii_case(tag));
        assert_eq!((c.extra, c.listed), (None, PROOFING_LANGUAGES.map(|(t, _)| t == tag)), "{tag}");
    }
    for (tag, name) in
        [("en-NZ", "English (New Zealand)"), ("en-IE", "English (Ireland)"), ("en-ZA", "English (South Africa)"), ("en-IN", "English (India)")]
    {
        assert!(crate::dialogs_layout::LANGUAGES.contains(&(tag, name)), "{tag}");
        assert_eq!(language_name(Some(tag)), name);
    }
    assert_eq!(language_name(Some("en-SG")), "English (Singapore)");
}

#[test]
fn indonesian_has_a_name_in_the_language_menu() {
    assert!(PROOFING_LANGUAGES.contains(&("id-ID", "Indonesian")));
    for tag in ["id", "id-ID", "ID-id", "ind", "in"] {
        assert_eq!(language_name(Some(tag)), "Indonesian", "{tag}");
    }
}

/// #3360, #3553: the Spanishes WordCraft proofs are in the menu, checked by their tags; other
/// Spanish regions show by name, and a bare `es` is Spain's.
#[test]
fn spanish_varieties_are_in_the_language_menu() {
    for (tag, name) in
        [("es-ES", "Spanish (Spain)"), ("es-MX", "Spanish (Mexico)"), ("es-419", "Spanish (Latin America)"), ("es-UY", "Spanish (Uruguay)")]
    {
        assert!(PROOFING_LANGUAGES.contains(&(tag, name)), "{tag}");
        assert!(crate::dialogs_layout::LANGUAGES.contains(&(tag, name)), "{tag}");
        assert_eq!(language_name(Some(tag)), name);
        let c = language_checks(&tag.to_ascii_lowercase(), |t| t.eq_ignore_ascii_case(tag));
        assert_eq!((c.extra, c.listed), (None, PROOFING_LANGUAGES.map(|(t, _)| t == tag)), "{tag}");
    }
    assert_eq!(language_name(Some("es")), "Spanish (Spain)");
    assert_eq!(language_name(Some("es-AR")), "Spanish (Argentina)");
    let c = language_checks("es-AR", |t| t.eq_ignore_ascii_case("es-AR"));
    assert_eq!(c.extra.as_deref(), Some("Spanish (Argentina)"));
    assert!(!c.listed.contains(&true));
}

#[test]
fn czech_has_a_name_in_the_language_menu() {
    assert!(PROOFING_LANGUAGES.contains(&("cs-CZ", "Czech")));
    for tag in ["cs", "cs-CZ", "CS-cz", "ces", "CZE"] {
        assert_eq!(language_name(Some(tag)), "Czech", "{tag}");
    }
}

#[test]
fn portuguese_has_a_name_in_the_language_menu() {
    assert!(PROOFING_LANGUAGES.iter().any(|(t, _)| *t == "pt-BR"));
    assert_eq!(language_name(Some("pt-BR")), "Portuguese (Brazil)");
    assert_eq!(language_name(Some("pt")), "Portuguese (Brazil)");
    assert_eq!(language_name(Some("PT-BR".to_string().as_str())), "Portuguese (Brazil)");
}

#[test]
fn turkish_has_a_name_in_the_language_menu() {
    assert!(PROOFING_LANGUAGES.iter().any(|(t, _)| *t == "tr-TR"));
    assert_eq!(language_name(Some("tr-TR")), "Turkish");
    assert_eq!(language_name(Some("tr")), "Turkish");
    assert_eq!(language_name(Some("TR-TR")), "Turkish");
}

#[test]
fn picture_format_tab_appears_when_picture_selected() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    assert!(!has_picture_selected(&s));
    assert!(!contextual_tabs(&s).contains(&"Picture Format"));
    s.run("insert.picture", &json!({"data": png_data()})).unwrap();
    assert!(has_picture_selected(&s));
    assert!(contextual_tabs(&s).contains(&"Picture Format"));
    s.run("select.collapse", &json!({"end": true})).unwrap();
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    assert!(!has_picture_selected(&s));
}

#[test]
fn shape_format_tab_appears_when_shape_selected() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
    assert!(contextual_tabs(&s).contains(&"Shape Format"));
    assert!(!contextual_tabs(&s).contains(&"Picture Format"));
    s.run("select.collapse", &json!({"end": true})).unwrap();
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    assert!(!contextual_tabs(&s).contains(&"Shape Format"));
}

#[test]
fn page_setup_launcher_opens_the_dialog() {
    // A group launcher runs its command with no parameters (#471).
    let mut app = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.run("layout.pageSetup", json!({})).unwrap();
    assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::PageSetup { .. })));
    // `ui.dialog` needs a dialog name, so it can't be a launcher.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let ribbon: String = crate::rust_sources(&src)
        .iter()
        .filter(|p| **p == src.join("ribbon.rs") || p.starts_with(src.join("ribbon")))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    let launchers: Vec<&str> =
        ribbon.split("group(ui, \"").filter_map(|g| g.split_once('"')?.1.strip_prefix(", Some(\"")?.split('"').next()).collect();
    assert!(launchers.contains(&"layout.pageSetup"), "{launchers:?}");
    assert!(!launchers.contains(&"ui.dialog"), "{launchers:?}");
}

#[test]
fn smart_art_picker_inserts_and_design_tab_follows_the_selection() {
    let mut app = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
    // Insert › SmartArt without a layout opens the picker.
    let _ = app.run("insert.smartArt", json!({}));
    assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("insertSmartArt"));
    app.dialog = None;
    app.run("insert.smartArt", json!({"layout": "cycle"})).unwrap();
    assert!(contextual_tabs(&app.session).contains(&"SmartArt Design"));
    app.run("smartArt.textPane", json!({})).unwrap();
    assert!(app.session.view.smart_art_pane);
    app.session.run("select.collapse", &json!({"end": true})).unwrap();
    app.session.run("text.insert", &json!({"text": "x"})).unwrap();
    assert!(!contextual_tabs(&app.session).contains(&"SmartArt Design"));
}

#[test]
fn chart_design_tab_appears_when_chart_selected_and_buttons_open_dialogs() {
    let mut app = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
    // Insert › Chart without a type opens the gallery; choosing inserts.
    let _ = app.run("insert.chart", json!({}));
    assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("insertChart"));
    app.dialog = None;
    app.run("insert.chart", json!({"type": "doughnut"})).unwrap();
    assert!(contextual_tabs(&app.session).contains(&"Chart Design"));
    let _ = app.run("chart.editData", json!({}));
    assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("chartData"));
    let _ = app.run("chart.type", json!({}));
    assert_eq!(app.dialog.as_ref().map(crate::dialogs::Dialog::name), Some("changeChartType"));
    app.session.run("select.collapse", &json!({"end": true})).unwrap();
    app.session.run("text.insert", &json!({"text": "x"})).unwrap();
    assert!(!contextual_tabs(&app.session).contains(&"Chart Design"));
}

/// No group's content runs into its label: View › Show stacked four check boxes in a column
/// made for three, so "Hide White Space" was drawn under "Show". Every tab, every language.
#[test]
fn ribbon_groups_fit_above_their_labels() {
    let app = WordApp::new(Session::new(wordcraft_engine::sample::sample_document()), crate::Services::default());
    // Wide enough that no tab hides groups behind its scroll arrows.
    let mut h = egui_kittest::Harness::builder().with_size(vec2(2560.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    let mut tall = Vec::new();
    for lang in crate::i18n::Lang::all() {
        h.state_mut().run("ui.language", json!({"value": lang.code()})).unwrap();
        for tab in TABS.iter().filter(|t| **t != "File") {
            h.state_mut().run("ui.tab", json!({"tab": tab})).unwrap();
            h.run_steps(2);
            crate::widgets::TALL_GROUPS.with(|g| g.borrow_mut().clear());
            h.run_steps(1);
            let found = crate::widgets::TALL_GROUPS.with(|g| g.take());
            tall.extend(found.into_iter().map(|g| format!("{} {tab} › {g}", lang.code())));
        }
    }
    tall.sort();
    tall.dedup();
    assert!(tall.is_empty(), "groups taller than the ribbon: {tall:#?}");
}

/// #897: the tabs that didn't fit were drawn under Comments / Editing / Share, so at 1280 pt
/// in Polish the table tabs were out of reach. They stay on the strip and a click opens them.
#[test]
fn table_tabs_stay_clickable_when_the_tab_strip_overflows() {
    if !crate::i18n::catalogs_embedded() {
        return;
    }
    use egui_kittest::kittest::Queryable;
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    app.run("insert.table", json!({"rows": 3, "cols": 3})).unwrap();
    app.run("ui.language", json!({"value": "pl"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    let buttons = [tl!("Comments"), tl!("Editing")].map(|b| h.get_by_label(b).rect());
    for tab in ["Table Design", "Table Layout"] {
        let r = h.get_by_label(tl!(tab)).rect();
        assert!(r.right() <= 1280.0 && buttons.iter().all(|b| !b.intersects(r)), "{tab} {r:?} under {buttons:?}");
        h.get_by_label(tl!(tab)).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.tab, tab);
    }
}

/// #2026: at every width and in the longest languages no tab is drawn over Comments /
/// Editing / Share, the tabs stay inside the window, and the buttons keep their room.
#[test]
fn tab_strip_never_overlaps_the_right_buttons() {
    if !crate::i18n::catalogs_embedded() {
        return;
    }
    use egui_kittest::kittest::Queryable;
    for lang in ["en", "pl", "de"] {
        for width in [300.0, 420.0, 520.0, 640.0, 765.0, 1280.0] {
            let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
            app.run("insert.table", json!({"rows": 3, "cols": 3})).unwrap();
            app.run("ui.language", json!({"value": lang})).unwrap();
            app.run("ui.tab", json!({"tab": "Table Layout"})).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 600.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
            h.run_steps(3);
            let buttons: Vec<Rect> =
                [tl!("Comments"), tl!("Editing"), tl!("Share")].iter().filter_map(|b| h.query_by_label(b).map(|n| n.rect())).collect();
            assert_eq!(buttons.len(), 3, "{lang} {width}: the three buttons are on the strip");
            assert!(buttons.iter().all(|b| b.right() <= width + 0.5 && b.left() >= 0.0), "{lang} {width}: buttons inside {buttons:?}");
            for tab in crate::ribbon::TABS.iter().copied().chain(["Table Design", "Table Layout"]) {
                // A group folded into a button can share the tab's name (Draw): only the node on
                // the strip is the tab.
                let Some(r) = h.query_all_by_label(tl!(tab)).map(|n| n.rect()).find(|r| r.top() <= 40.0) else { continue };
                assert!(r.right() <= width + 0.5, "{lang} {width}: {tab} {r:?} past the window");
                assert!(buttons.iter().all(|b| !b.intersects(r)), "{lang} {width}: {tab} {r:?} under {buttons:?}");
            }
            assert!(h.query_by_label(tl!("File")).is_some(), "{lang} {width}: File stays on the strip");
        }
    }
}

#[test]
fn plan_strip_gives_way_in_order() {
    let spec = |text_w, file, active, contextual| TabSpec { text_w, file, active, contextual };
    let tabs = [spec(20.0, true, false, false), spec(30.0, false, true, false), spec(30.0, false, false, false), spec(40.0, false, false, true)];
    let buttons = [200.0, 150.0, 60.0];
    // Plenty of room: nothing gives way.
    let p = plan_strip(&tabs, buttons, 600.0, 26.0);
    assert_eq!((p.pad, p.buttons, p.fits.iter().all(|f| *f)), (18.0, 0, true));
    // Tighter: the padding goes first, then the Comments / Editing labels, then Share's.
    let p = plan_strip(&tabs, buttons, 380.0, 26.0);
    assert_eq!((p.pad, p.buttons), (10.0, 0));
    let p = plan_strip(&tabs, buttons, 330.0, 26.0);
    assert_eq!((p.pad, p.buttons), (10.0, 1));
    let p = plan_strip(&tabs, buttons, 240.0, 26.0);
    assert_eq!((p.pad, p.buttons), (10.0, 2));
    // Then the plain tab goes into »; File, the active and the contextual tab stay.
    let p = plan_strip(&tabs, buttons, 215.0, 26.0);
    assert_eq!(p.fits, [true, true, false, true]);
    // Then the labels of the active and contextual tabs are cut down.
    let p = plan_strip(&tabs, buttons, 208.0, 26.0);
    assert_eq!(p.fits, [true, true, false, true]);
    assert!(p.widths[3] < 52.0 && p.widths[3] >= MIN_TAB_W - 0.01, "{:?}", p.widths);
    // No room for that either: the contextual one goes into », the active one last.
    let p = plan_strip(&tabs, buttons, 190.0, 26.0);
    assert_eq!(p.fits, [true, true, false, false]);
    let p = plan_strip(&tabs, buttons, 140.0, 26.0);
    assert_eq!(p.fits, [true, false, false, false]);
    // The buttons' room is never taken, and hostile numbers don't panic.
    for avail in [0.0, -5.0, f32::NAN, f32::INFINITY] {
        let _ = plan_strip(&tabs, buttons, avail, 26.0);
    }
    let _ = plan_strip(&[], buttons, 100.0, 26.0);
    let _ = plan_strip(&[spec(f32::NAN, false, true, false)], [f32::NAN; 3], 100.0, f32::NAN);
}

/// #1110: the strip's drawn buttons have names for screen readers: Share, and the "»" that
/// holds the tabs that don't fit ("More tabs", not a glyph).
#[test]
fn the_tab_strip_buttons_have_names() {
    use egui_kittest::kittest::Queryable;
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Text")), crate::Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(vec2(760.0, 700.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, tl!("Share"));
    let more = h.get_by_role_and_label(egui::accesskit::Role::Button, tl!("More tabs"));
    more.click();
    h.run_steps(2);
    // The menu lists a tab that didn't fit (View is among the last on a narrow strip).
    assert!(h.query_by_label(tl!("View")).is_some(), "the » menu lists the tabs left out");
}

#[test]
fn fit_tabs_keeps_the_kept_tabs_and_drops_from_the_end() {
    let tabs = [(40.0, true), (50.0, false), (50.0, false), (50.0, false), (60.0, true)];
    assert_eq!(fit_tabs(&tabs, 250.0, 20.0), [true; 5]);
    // 250 doesn't fit 240 with room for »: the last plain tabs give way, the kept ones stay.
    assert_eq!(fit_tabs(&tabs, 240.0, 20.0), [true, true, true, false, true]);
    // 200 would fit 210, but not with the » button too.
    assert_eq!(fit_tabs(&tabs, 210.0, 20.0), [true, true, false, false, true]);
    assert_eq!(fit_tabs(&tabs, 150.0, 20.0), [true, false, false, false, true]);
    // Nothing but the kept tabs fits: they stay anyway.
    assert_eq!(fit_tabs(&tabs, 10.0, 20.0), [true, false, false, false, true]);
    // Hostile numbers don't panic.
    let _ = fit_tabs(&[(f32::NAN, false), (f32::INFINITY, false)], f32::NAN, 20.0);
}

/// #860: Review › Language (and the status bar) reach every proofing language, not just the
/// short list: Set Proofing Language… opens the Language dialog (French, Set As Default),
/// Language Preferences… opens Options › Language, and French text shows by name, checked.
#[test]
fn language_menu_reaches_every_language() {
    use egui_kittest::kittest::{By, NodeT, Queryable};
    let item = |label: &'static str| By::new().label_contains(label);
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Bonjour")), crate::Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 600.0)).build_ui_state(|ui, app: &mut WordApp| language_menu(ui, app), app);
    h.run();
    assert!(h.query(item("French (France)")).is_none(), "the short list only, on an English document");
    assert!(!h.get(item("Set Proofing Language…")).accesskit_node().is_disabled(), "never greyed out on a fresh document");
    assert!(!h.get(item("English (United States)")).accesskit_node().is_disabled());
    h.get(item("Set Proofing Language…")).click();
    h.run();
    assert_eq!(h.state().dialog.as_ref().map(crate::dialogs::Dialog::name), Some("language"));
    h.state_mut().dialog = None;
    // French for the whole document: the menu and the status bar name it.
    h.state_mut().run("review.language", json!({"lang": "fr-FR", "default": true})).unwrap();
    h.run();
    assert_eq!(caret_language(h.state()), "French (France)");
    assert!(h.query(item("French (France)")).is_some(), "the current language is listed (only while it is the current one)");
    h.get(item("Language Preferences…")).click();
    h.run();
    assert!(h.state().ui.backstage, "File › Options opened");
    assert_eq!(h.state().ui.options_pane, "language");
    assert_eq!(language_name(Some("de-DE")), "German (Germany)");
}

/// #1281: every short-list entry (the four Englishes, Indonesian…) is picked in the menu:
/// clicking it sets the selection's proofing language, and the entry is then the checked one.
#[test]
fn every_proofing_language_is_picked_from_the_language_menu() {
    use egui_kittest::kittest::{By, NodeT, Queryable};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Colour")), crate::Services::default());
    app.run("select.all", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 600.0)).build_ui_state(|ui, app: &mut WordApp| language_menu(ui, app), app);
    h.run();
    for (tag, name) in PROOFING_LANGUAGES {
        h.get(By::new().label_contains(name)).click();
        h.run();
        let st = h.state_mut().session.run("format.state", &json!({})).unwrap();
        assert_eq!(st.get("lang").and_then(Value::as_str), Some(tag), "{name}");
        for (_, other) in PROOFING_LANGUAGES {
            let ticked = h.get(By::new().label_contains(other)).accesskit_node().toggled();
            assert_eq!(ticked == Some(egui::accesskit::Toggled::True), other == name, "{name}: {other} ticked {ticked:?}");
        }
    }
}

#[test]
fn resolve_tab_falls_back_when_contextual_tab_expires() {
    let all = ["Home", "Table Design", "Picture Format"];
    assert_eq!(resolve_tab("Picture Format", &all), "Picture Format");
    assert_eq!(resolve_tab("Picture Format", &["Home"]), "Home");
    assert_eq!(resolve_tab("Table Design", &["Home"]), "Home");
    assert_eq!(resolve_tab("Home", &["Home"]), "Home");
}

/// #2573, #1901: Picture Format › Size's width shows the interface unit and takes a size typed
/// in pixels. It applies once typed (Enter), not while typing, keeping the picture's proportions, as one
/// undo step.
#[test]
fn picture_width_typed_in_pixels_keeps_the_proportions() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("file.options", json!({"units": "centimeters"})).unwrap();
    a.run("insert.picture", json!({"data": png_data()})).unwrap();
    a.run("ui.tab", json!({"tab": "Picture Format"})).unwrap();
    let size = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h)));
    assert_eq!(size(&a), Some((15.0, 7.5)), "20×10 px at 96 ppi");
    // Wide enough for the whole tab: the Size group is its last.
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    let shown = h.get_by_role_and_label(Role::SpinButton, "W:").value().unwrap_or_default();
    assert!(shown.ends_with(" cm"), "the interface unit: {shown:?}");
    h.get_by_role_and_label(Role::SpinButton, "W:").click();
    h.run_steps(2);
    // Editing, the field is still a spin button, now showing the bare number to type over.
    let editing = h.get_by_role_and_label(Role::SpinButton, "W:");
    assert!(editing.is_focused() && editing.value().is_some_and(|v| !v.ends_with(" cm")), "a click starts typing");
    editing.type_text("300 px");
    h.run_steps(2);
    assert_eq!(size(h.state()), Some((15.0, 7.5)), "nothing applies while typing");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let (w, ht) = size(h.state()).unwrap();
    assert!((w - 225.0).abs() < 0.01 && (ht - 112.5).abs() < 0.01, "300 px = 225 pt, the height follows: {w}×{ht}");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(size(h.state()), Some((15.0, 7.5)), "one undo step");
    // Esc throws a typed size away.
    h.get_by_role_and_label(Role::SpinButton, "W:").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::SpinButton, "W:").type_text("500 px");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(size(h.state()), Some((15.0, 7.5)), "Esc cancels the typing");
}

/// #2573 review: showing Picture Format never resizes a picture whose size is outside the
/// fields' range (a 1×1 px spacer is 0.75 pt): egui clamps such a value by default and reports
/// it as a change.
#[test]
fn picture_format_leaves_a_tiny_picture_alone() {
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png)})).unwrap();
    a.run("ui.tab", json!({"tab": "Picture Format"})).unwrap();
    let size = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h)));
    assert_eq!(size(&a), Some((0.75, 0.75)));
    let undo = a.session.undo_depth();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(6);
    assert_eq!(size(h.state()), Some((0.75, 0.75)), "shown, not resized");
    assert_eq!(h.state().session.undo_depth(), undo, "no undo step");
}

/// #3242: Align used to run `arrange.align` directly with no `value`, which always centers
/// (`crates/engine/src/cmd/objects.rs`) — there was no way to reach Left or Right from the
/// ribbon short of dragging. Align now opens a menu, like Word's own Align dropdown.
#[test]
fn align_menu_reaches_left_and_right_not_just_center() {
    use egui_kittest::kittest::{By, Queryable};
    let item = |label: &'static str| By::new().label_contains(label);
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.picture", json!({"data": png_data()})).unwrap();
    let frame = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, f.x)));
    assert!(matches!(frame(&a), Some((wordcraft_doc::para::Wrap::Inline, _))), "a freshly inserted picture is inline");
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(300.0, 300.0))
        .build_ui_state(|ui, app: &mut crate::WordApp| align_menu(ui, app, None), a);
    h.run();
    h.get(item("Align")).click();
    h.run();
    assert!(h.query(item("Align Left")).is_some(), "Align opens a menu, instead of running arrange.align with no value");
    h.get(item("Align Right")).click();
    h.run();
    let (wrap, right_x) = frame(h.state()).unwrap();
    assert_eq!(wrap, wordcraft_doc::para::Wrap::Square, "aligning floats an inline picture, as arrange.align already does");
    // A positive x alone doesn't pin Right down: Center's x is positive too whenever the
    // picture is narrower than the text column, so Right could silently dispatch Center.
    h.get(item("Align")).click();
    h.run();
    h.get(item("Align Center")).click();
    h.run();
    let center_x = frame(h.state()).unwrap().1;
    h.get(item("Align")).click();
    h.run();
    h.get(item("Align Left")).click();
    h.run();
    let left_x = frame(h.state()).unwrap().1;
    assert!(left_x < center_x && center_x < right_x, "Left, Center and Right each land at a distinct x: {left_x} < {center_x} < {right_x}");
}

/// #2573 review: Shape Format and Chart Design leave sizes outside their fields' ranges alone
/// too (a 0.5 pt high line; a chart narrower than 36 pt), as Picture Format does.
#[test]
fn shape_and_chart_formats_leave_out_of_range_sizes_alone() {
    let size = |a: &crate::WordApp| wordcraft_engine::cmd::objects::selected(&a.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h)));
    let shown = |a: crate::WordApp| {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 900.0)).build_ui_state(
            |ui, app: &mut crate::WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(6);
        let a = h.state();
        (size(a), a.session.undo_depth())
    };
    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    a.run("picture.size", json!({"width": 3000.0, "height": 1.0, "lockAspect": false})).unwrap();
    a.run("ui.tab", json!({"tab": "Shape Format"})).unwrap();
    let before = (size(&a), a.session.undo_depth());
    assert_eq!(before.0, Some((3000.0, 1.0)));
    assert_eq!(shown(a), before, "Shape Format shown, the shape untouched");

    let mut a = crate::WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.run("insert.chart", json!({"type": "doughnut"})).unwrap();
    a.run("picture.size", json!({"width": 20.0, "height": 2500.0, "lockAspect": false})).unwrap();
    a.run("ui.tab", json!({"tab": "Chart Design"})).unwrap();
    let before = (size(&a), a.session.undo_depth());
    assert_eq!(before.0, Some((20.0, 2500.0)));
    assert_eq!(shown(a), before, "Chart Design shown, the chart untouched");
}
