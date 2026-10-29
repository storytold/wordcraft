use super::*;
use wordcraft_doc::Pos;
use wordcraft_doc::props::Align;
use wordcraft_engine::Session;

/// #3931: hovering Insert › Table's grid asks the page for a preview of that table (rows by
/// columns) and changes nothing; leaving the grid drops it; a click inserts the table with no
/// preview left over for the page to show on top of it.
#[test]
fn the_table_grid_previews_the_hovered_table_until_the_click() {
    use egui_kittest::kittest::Queryable;
    let a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 400.0)).build_ui_state(|ui, app: &mut WordApp| table_grid_picker(ui, app), a);
    h.run_steps(2);
    let spacing = h.ctx.global_style().spacing.clone();
    let top = h.get_by_label("Insert Table").rect();
    // Squares are 16 points with 2 between, in grid columns at least `interact_size` wide:
    // the middle of row 3, column 4.
    let col = spacing.interact_size.x.max(16.0) + 2.0;
    let at = egui::pos2(top.left() + 3.0 * col + 8.0, top.bottom() + spacing.item_spacing.y + 2.0 * 18.0 + 8.0);
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().canvas.table_preview, Some((3, 4)), "three rows of four columns");
    assert!(h.query_by_label("4×3 Table").is_some(), "the label reads columns × rows");
    assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()) && !h.state().session.can_undo());
    h.event(egui::Event::PointerMoved(egui::pos2(390.0, 390.0)));
    h.state_mut().canvas.table_preview = None;
    h.run_steps(2);
    assert_eq!(h.state().canvas.table_preview, None, "off the grid nothing is previewed");
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    // The page takes the request every frame; here nothing does.
    h.state_mut().canvas.table_preview = None;
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
    let t = h.state().session.doc.body.iter().find_map(|b| b.as_table()).expect("the click inserts the table");
    assert_eq!((t.rows.len(), t.cols()), (3, 4));
    assert_eq!(h.state().canvas.table_preview, None, "the inserted table isn't previewed again");
}

/// Go To's page box (#712): a number, +n / -n from the current page (a typographic minus
/// too), empty for the next page; anything else is refused.
#[test]
fn go_to_reads_its_page_box() {
    assert_eq!(goto_page("7"), Some(json!({"page": 7})));
    assert_eq!(goto_page(" +2 "), Some(json!({"pageBy": 2})));
    assert_eq!(goto_page("-1"), Some(json!({"pageBy": -1})));
    assert_eq!(goto_page("\u{2212}3"), Some(json!({"pageBy": -3})));
    assert_eq!(goto_page(""), Some(json!({"pageBy": 1})));
    for junk in ["0", "abc", "+x", "1.5", "99999999999999999999999"] {
        assert_eq!(goto_page(junk), None, "{junk}");
    }
}

/// Insert › Bookmark (#1236), as in Word: it opens on the bookmark around the caret, lists by
/// name, Go To selects a bookmark's text and Delete removes one — both leaving the dialog
/// open, Cancel then reading Close — and one undo brings the bookmark back.
#[test]
fn the_bookmark_dialog_goes_to_and_deletes_bookmarks() {
    /// Clicks the labelled control inside the open Bookmark dialog and advances three frames.
    fn press(h: &mut egui_kittest::Harness<'_, WordApp>, label: &str) {
        use egui_kittest::kittest::Queryable;
        let dialog = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", "Bookmark")))).expect("the dialog is open");
        h.query_all_by_label(label).find(|n| dialog.contains(n.rect().center())).expect(label).click();
        for _ in 0..3 {
            h.step();
        }
    }
    let mut app = app_with("Alpha beta gamma");
    for (word, name) in [("Alpha", "_Hidden"), ("beta", "Zulu"), ("gamma", "Yankee")] {
        let _ = app.run("select.text", json!({"text": word}));
        let _ = app.run("insert.bookmark", json!({"name": name}));
    }
    let doc = &app.session.doc;
    assert_eq!(bookmark_names(doc, false, false), ["Yankee", "Zulu"], "by name, hidden ones left out");
    assert_eq!(bookmark_names(doc, true, false), ["Zulu", "Yankee"], "by location");
    assert_eq!(bookmark_names(doc, false, true), ["_Hidden", "Yankee", "Zulu"]);
    let _ = app.run("select.text", json!({"text": "gamma"}));
    app.dialog = Dialog::open("bookmark", &mut app);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    assert!(matches!(&h.state().dialog, Some(Dialog::Bookmark { name, .. }) if name == "Yankee"), "opens on the caret's bookmark");
    press(&mut h, "Zulu");
    press(&mut h, "Go To");
    assert_eq!(h.state().session.selected_text(), "beta");
    press(&mut h, "Delete");
    let names: Vec<String> = h.state().session.doc.bookmarks().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["_Hidden", "Yankee"]);
    assert!(matches!(&h.state().dialog, Some(Dialog::Bookmark { name, deleted: true, .. }) if name.is_empty()), "still open");
    assert_eq!(h.state().session.selected_text(), "beta", "the text stays selected");
    press(&mut h, "Close");
    assert!(h.state().dialog.is_none());
    let _ = h.state_mut().run("edit.undo", json!({}));
    assert_eq!(h.state().session.doc.bookmarks().len(), 3, "one undo step");
}

/// #1904: with no bookmarks the list says so instead of a blank box.
#[test]
fn the_bookmark_dialog_says_when_there_are_no_bookmarks() {
    let mut app = app_with("Alpha beta gamma");
    app.dialog = Dialog::open("bookmark", &mut app);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    let texts: Vec<String> = h
        .output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t.contains("The document has no bookmarks yet.")), "placeholder shown");
}

/// Find, Replace and Go To are one dialog with three tabs (#712).
#[test]
fn find_replace_and_go_to_are_one_dialog() {
    let mut app = app_with("One");
    for (name, tab) in [("find", FindTab::Find), ("replace", FindTab::Replace), ("goto", FindTab::GoTo)] {
        let _ = app.run("ui.dialog", json!({"name": name}));
        assert!(matches!(&app.dialog, Some(Dialog::Find { tab: t, .. }) if *t == tab), "{name}");
        assert_eq!(app.dialog.as_ref().map(|d| d.name()), Some(name));
    }
}

fn app_with(text: &str) -> WordApp {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let _ = app.run("document.setText", json!({"text": text}));
    app
}

fn props(app: &WordApp) -> wordcraft_doc::props::ParaProps {
    app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default()
}

/// #819: the text a shortcut sends with its key isn't typed into the box it opened, whether it
/// comes in the same frame or the next; real typing (a key press first) is.
#[test]
fn shortcut_text_is_swallowed_once() {
    let key = |key, alt| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers { alt, ..Default::default() },
    };
    let text = |t: &str| egui::Event::Text(t.into());
    let run = |events: &[egui::Event]| {
        let mut swallow = true;
        events.iter().filter(|e| !swallow_shortcut_text(&mut swallow, e)).cloned().collect::<Vec<_>>()
    };
    assert_eq!(run(&[key(egui::Key::Q, true), text("q")]), vec![key(egui::Key::Q, true)], "Alt+Q's q, same frame");
    assert_eq!(run(&[text("q"), text("b")]), vec![text("b")], "the q a frame late; then typing");
    assert_eq!(run(&[key(egui::Key::B, false), text("b")]), vec![key(egui::Key::B, false), text("b")], "a click-opened box takes typing");
}

/// #819: Search matches every word anywhere in the label, location or id, in any order.
#[test]
fn command_search_matches_every_word() {
    let app = app_with("");
    let reg = app.session.registry.clone();
    let table = reg.get("insert.table").unwrap();
    assert!(command_matches(table, "insert table"), "the box's own example");
    assert!(command_matches(table, "  Table   INSERT "), "any order, any case, extra spaces");
    assert!(!command_matches(table, "insert chart"), "every word has to match");
    assert!(!command_matches(table, ""), "an empty box lists nothing");
    assert!(!command_matches(table, "   "));
}

/// Ordinary Turkish queries must find dotted-I labels, while canonical English labels and
/// ASCII command IDs retain their old case-insensitive search in every interface language.
#[test]
fn command_search_respects_turkish_i_pairs_and_keeps_canonical_matches() {
    use egui_kittest::kittest::Queryable;
    // Turkish casing follows the language code; the labels are fixtures with both I pairs.
    const LABELS: &str = "\tItalic\tİtalik\n\tHome\tGiriş\n\tFont\tYazı Tipi\n\tTable of Contents\tİçindekiler Tablosu\n\tReferences\tBaşvurular\n";
    let reg = app_with("").session.registry.clone();
    let italic = reg.get("format.italic").unwrap();
    let table = reg.get("insert.table").unwrap();
    let toc = reg.all().iter().find(|c| c.label == "Table of Contents" && c.location == "References › Table of Contents").unwrap();
    crate::i18n::set_current(crate::i18n::fixture_lang("tr", LABELS));
    for query in ["italik", "İTALİK", "YAZI TİPİ", "FORMAT.ITALIC", "ITALIC"] {
        assert!(command_matches(italic, query), "tr: {query}");
    }
    for query in ["içindekiler", "TABLE OF CONTENTS"] {
        assert!(command_matches(toc, query), "tr: {query}");
    }
    for query in ["INSERT", "INSERT.TABLE"] {
        assert!(command_matches(table, query), "tr: {query}");
    }
    assert_eq!(crate::i18n::lowercase("Iİıi"), "ıiıi");
    // The same labels under another language's code use Unicode's default casing.
    crate::i18n::set_current(crate::i18n::fixture_lang("x-fixture", LABELS));
    assert!(!command_matches(italic, "italik"), "İ lowercases to i plus a combining dot");
    assert_eq!(crate::i18n::lowercase("Iİıi"), "ii\u{307}ıi");
    crate::i18n::set_current(crate::i18n::Lang::EN);

    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        move |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            if fonts {
                show(app, &ctx);
            } else {
                crate::theme::install_fonts_for(&ctx, false);
                fonts = true;
            }
        },
        app_with("Unchanged document"),
    );
    let before = serde_json::to_value(&h.state().session.doc).unwrap();
    for (query, expected) in
        [("ITALIC", "Italic   —   Home › Font"), ("FORMAT.ITALIC", "Italic   —   Home › Font"), ("INSERT.TABLE", "Table   —   Insert › Tables")]
    {
        h.state_mut().dialog = Some(Dialog::Commands { query: query.into(), swallow: false });
        h.run_steps(4);
        assert!(
            h.query(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button).label_contains(expected)).is_some(),
            "{query}\n{:?}",
            h.query_all_by_role(egui::accesskit::Role::Button).collect::<Vec<_>>()
        );
    }
    assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), before, "search does not change the document");
}

/// #3914: the Phonetic Guide dialog opens with the keyboard in its Ruby text box, so typing
/// fills the box and does not replace the selected text in the document.
#[test]
fn phonetic_guide_ruby_box_takes_the_keyboard_on_open() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut app = app_with("Unchanged document");
    app.dialog = Some(Dialog::Phonetic {
        base: "Unchanged".into(),
        ruby: String::new(),
        align: "center".into(),
        size: 12.0,
        offset: 0.0,
        existing: false,
        focus: true,
    });
    let mut h = app_harness(app);
    h.run_steps(4);
    // Every editable field has a name, and the Ruby box takes the keyboard on open.
    assert!(h.get_by_role_and_label(Role::TextInput, "Ruby text:").is_focused(), "the Ruby text box takes the keyboard on open");
    assert_eq!(
        h.get_by_role_and_label(Role::ComboBox, "Alignment:").value().as_deref(),
        Some("Center"),
        "Alignment is named and shows the current alignment"
    );
    h.get_by_role_and_label(Role::SpinButton, "Offset (pt):");
    h.get_by_role_and_label(Role::SpinButton, "Size (pt):");
    h.event(egui::Event::Text("ab".into()));
    h.run_steps(2);
    match h.state().dialog.as_ref() {
        Some(Dialog::Phonetic { ruby, .. }) => assert_eq!(ruby, "ab", "the typed text reaches the Ruby text box"),
        _ => panic!("the Phonetic Guide dialog is still open"),
    }
}

/// #2986: a box that keeps the keyboard asks for focus only while it lacks it. `request_focus`
/// always interrupts IME; on X11 that recreates the XIM context, and Search asking every frame
/// left fcitx taking no keys at all. Once the box has focus, no frame interrupts, and typing lands;
/// after Tab takes the keyboard away, the box takes it back with one reset, not one per frame.
#[test]
fn boxes_that_keep_focus_do_not_interrupt_ime_every_frame() {
    fn typed(d: Option<&Dialog>) -> Option<&str> {
        match d? {
            Dialog::Commands { query, .. } | Dialog::Icons { query, .. } => Some(query.as_str()),
            Dialog::Find { goto_target, .. } => Some(goto_target.as_str()),
            Dialog::Refs(r) => match r.as_ref() {
                crate::dialogs_refs::RefsDialog::NewLabel(f) => Some(f.new_label.as_str()),
                _ => None,
            },
            _ => None,
        }
    }
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    for name in ["commands", "icons", "goto", "newLabel"] {
        let mut app = app_with("Unchanged document");
        let dialog = match name {
            "newLabel" => Dialog::Refs(Box::new(crate::dialogs_refs::RefsDialog::NewLabel(crate::dialogs_refs::CaptionForm::read(&app)))),
            _ => Dialog::open(name, &mut app).unwrap(),
        };
        app.dialog = Some(dialog);
        let mut h = app_harness(app);
        // The box takes the keyboard from the page in the first frames.
        h.run_steps(4);
        let interrupted = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.should_interrupt_composition);
        for frame in 0..6 {
            h.step();
            assert_eq!(interrupted(&h), Some(false), "{name}: frame {frame} resets the input method");
        }
        for (k, t) in [(egui::Key::A, "a"), (egui::Key::B, "b")] {
            h.event(key(k));
            h.event(egui::Event::Text(t.into()));
            h.step();
            assert_eq!(interrupted(&h), Some(false), "{name}: typing {t} resets the input method");
        }
        assert_eq!(typed(h.state().dialog.as_ref()), Some("ab"), "{name}: the keys reach the box");

        // Tab moves the keyboard to the dialog's next control and the box takes it back. As
        // egui-winit does, count a reset when IME comes back on or is interrupted while on (each
        // recreates the XIM context): one or two for the round trip, then none, and the next
        // key reaches the box.
        let reset = |h: &egui_kittest::Harness<'_, WordApp>, on: &mut bool| {
            let ime = h.output().platform_output.ime.as_ref();
            let reset = ime.is_some_and(|i| !*on || i.should_interrupt_composition);
            *on = ime.is_some();
            reset
        };
        let mut on = true;
        h.event(key(egui::Key::Tab));
        let mut back = 0;
        for _ in 0..4 {
            h.step();
            back += usize::from(reset(&h, &mut on));
        }
        assert!((1..=2).contains(&back), "{name}: Tab and back reset the input method {back} times");
        for frame in 0..4 {
            h.step();
            assert!(!reset(&h, &mut on), "{name}: frame {frame} after Tab resets the input method");
        }
        h.event(key(egui::Key::C));
        h.event(egui::Event::Text("c".into()));
        h.step();
        assert_eq!(typed(h.state().dialog.as_ref()), Some("abc"), "{name}: the box has the keyboard back after Tab");
    }
}

/// Open the Paragraph dialog and press OK without changing anything.
fn ok_unchanged(app: &mut WordApp) {
    ok_edit(app, |_| {});
}

/// Open the Paragraph dialog, change its options (`opts`) with `edit` and press OK.
fn ok_with(app: &mut WordApp, edit: impl FnOnce(&mut [bool; 4])) {
    ok_edit(app, |d| {
        if let Dialog::Paragraph { opts, .. } = d {
            edit(opts);
        }
    });
}

/// Open the Paragraph dialog, change it with `edit` and press OK.
fn ok_edit(app: &mut WordApp, edit: impl FnOnce(&mut Dialog)) {
    let Some(mut d) = Dialog::open("paragraph", app) else { panic!("no paragraph dialog") };
    edit(&mut d);
    apply_paragraph(app, &d);
}

/// The `i`th paragraph's own formatting.
fn para_props(app: &WordApp, i: usize) -> wordcraft_doc::props::ParaProps {
    app.session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(i)).map(|p| p.props.clone()).unwrap_or_default()
}

/// Three paragraphs, the first centred, indented, 30 pt after, kept with the next and with a
/// line spacing of 6 (more than the dialog's box goes up to), all of them selected.
fn mixed_selection() -> WordApp {
    let mut app = app_with("One\nTwo\nThree");
    let _ = app.run("caret.docStart", json!({}));
    let props = json!({"align": "center", "indentLeft": 36, "spaceAfter": 30, "keepNext": true, "lineSpacing": {"rule": "multiple", "value": 6}});
    let _ = app.run("para.set", json!({ "props": props }));
    let _ = app.run("select.all", json!({}));
    app
}

#[test]
fn paragraph_dialog_leaves_what_the_selected_paragraphs_differ_in() {
    // #1346: OK gave every selected paragraph the caret paragraph's formatting, as three
    // undo steps.
    let mut app = mixed_selection();
    let was: Vec<_> = (0..3).map(|i| para_props(&app, i)).collect();
    let Some(Dialog::Paragraph { mixed, .. }) = Dialog::open("paragraph", &mut app) else { panic!() };
    let differ = ["align", "keepNext", "left", "lineSpacing", "spaceAfter"].map(String::from);
    assert_eq!(mixed, BTreeSet::from(differ), "the fields the paragraphs differ in show blank");
    let depth = app.session.undo_depth();
    ok_unchanged(&mut app);
    assert_eq!((0..3).map(|i| para_props(&app, i)).collect::<Vec<_>>(), was, "an unchanged OK changes nothing");
    assert_eq!(app.session.undo_depth(), depth, "nor adds an undo step");
    // One field changed: just that one, in every paragraph, as one undo step.
    ok_edit(&mut app, |d| {
        if let Dialog::Paragraph { before, .. } = d {
            *before = 6.0;
        }
    });
    assert_eq!(app.session.undo_depth(), depth + 1, "one undo step");
    for (i, was) in was.iter().enumerate() {
        assert_eq!(para_props(&app, i), wordcraft_doc::props::ParaProps { space_before: Some(6.0), ..was.clone() }, "paragraph {i}");
    }
    let _ = app.run("edit.undo", json!({}));
    assert_eq!((0..3).map(|i| para_props(&app, i)).collect::<Vec<_>>(), was, "undo takes all of it back");
}

/// #1346: a check box the selected paragraphs differ in shows as neither on nor off; a click
/// and OK turn it on in all of them, and the other fields they differ in stay as they were.
/// #3028: the records dialog refuses a range with no records itself; a good range merges into
/// a new document beside the unsaved one, asking nothing.
#[test]
fn merge_records_dialog_checks_the_range_before_merging() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with("Dear ");
    let _ = app.run("mailings.recipients", json!({"csv": "Name\nAda\nAlan\nGrace"}));
    let _ = app.run("text.insert", json!({"text": "unsaved"}));
    assert!(app.session.dirty);
    let source = app.session.document_id();
    app.dialog = Dialog::open("mergeRecords", &mut app);
    if let Some(Dialog::MergeRecords { choice, from, to, .. }) = &mut app.dialog {
        *choice = 2;
        *from = "3".into();
        *to = "2".into();
    }
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "From:");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "To:");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    for _ in 0..2 {
        h.step();
    }
    let Some(Dialog::MergeRecords { message, .. }) = &h.state().dialog else { panic!("the dialog stays open") };
    assert!(message.contains("no records from 3 to 2"), "{message}");
    if let Some(Dialog::MergeRecords { to, .. }) = &mut h.state_mut().dialog {
        *to = "3".into();
    }
    for _ in 0..2 {
        h.step();
    }
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    for _ in 0..2 {
        h.step();
    }
    assert!(h.state().dialog.is_none(), "nothing to ask about: the merge opens beside");
    assert_ne!(h.state().session.document_id(), source, "the merge is a new document");
    h.state_mut().run("view.switchWindows", json!({"id": source})).unwrap();
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("unsaved"), "the unsaved document is still open");
}

#[test]
fn paragraph_dialog_sets_a_mixed_check_box_for_all() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut app = mixed_selection();
    app.dialog = Dialog::open("paragraph", &mut app);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    // The line spacing box doesn't pull 6 into its range: still blank, still left alone.
    let Some(Dialog::Paragraph { mixed, .. }) = &h.state().dialog else { panic!("no paragraph dialog") };
    assert!(mixed.contains("lineSpacing"), "{mixed:?}");
    if let Some(Dialog::Paragraph { tab, .. }) = &mut h.state_mut().dialog {
        *tab = 1;
    }
    for _ in 0..4 {
        h.step();
    }
    // Screen readers hear it as mixed.
    let toggled = |label: &str| format!("{:?}", h.get_by_label(label).accesskit_node().toggled());
    assert_eq!(toggled("Keep with next"), "Some(Mixed)", "neither on nor off");
    assert_eq!(toggled("Keep lines together"), "Some(False)", "shared");
    h.get_by_label("Keep with next").click();
    for _ in 0..2 {
        h.step();
    }
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    for _ in 0..2 {
        h.step();
    }
    let app = h.state();
    assert!(app.dialog.is_none(), "OK closes the dialog");
    let p: Vec<_> = (0..3).map(|i| para_props(app, i)).collect();
    assert_eq!(p.iter().map(|p| p.keep_next).collect::<Vec<_>>(), [Some(true); 3]);
    assert_eq!(p.iter().map(|p| p.align).collect::<Vec<_>>(), [Some(Align::Center), None, None], "alignment untouched");
    assert_eq!(p.iter().map(|p| p.space_after).collect::<Vec<_>>(), [Some(30.0), None, None], "spacing untouched");
    assert_eq!(p[0].line_spacing, Some(wordcraft_doc::props::LineSpacing::Multiple(6.0)), "line spacing untouched");
}

#[test]
fn paragraph_dialog_sets_only_the_options_that_changed() {
    // #512: mirror indents, same-style spacing, suppress line numbers, don't hyphenate.
    let mut app = app_with("Hello");
    ok_unchanged(&mut app);
    let p = props(&app);
    assert_eq!((p.mirror_indents, p.contextual_spacing, p.suppress_line_numbers, p.suppress_hyphens), (None, None, None, None));
    ok_with(&mut app, |o| *o = [true; 4]);
    let p = props(&app);
    assert_eq!(
        (p.mirror_indents, p.contextual_spacing, p.suppress_line_numbers, p.suppress_hyphens),
        (Some(true), Some(true), Some(true), Some(true))
    );
    let Some(Dialog::Paragraph { opts, .. }) = Dialog::open("paragraph", &mut app) else { panic!() };
    assert_eq!(opts, [true; 4], "the dialog shows what is set");
    ok_with(&mut app, |o| o[3] = false);
    assert_eq!(props(&app).suppress_hyphens, Some(false));
    assert_eq!(props(&app).mirror_indents, Some(true));
}

#[test]
fn paragraph_dialog_shows_direction_and_visual_alignment() {
    let mut app = app_with("سلام دنیا");
    let _ = app.run("para.rtl", json!({}));
    let _ = app.run("para.set", json!({"props": {"indentLeft": 36.0}}));
    let Some(Dialog::Paragraph { rtl, align, left, right, .. }) = Dialog::open("paragraph", &mut app) else { panic!() };
    assert!(rtl);
    assert_eq!(align, "right", "a start-aligned right-to-left paragraph shows as right aligned");
    assert_eq!((left, right), (0.0, 0.5), "the start indent is the right one");
}

#[test]
fn paragraph_dialog_ok_keeps_what_it_shows() {
    for (rtl, align) in [(false, "para.alignCenter"), (false, "para.alignRight"), (true, "para.alignLeft"), (true, "para.alignRight")] {
        let mut app = app_with("متن");
        let _ = app.run(if rtl { "para.rtl" } else { "para.ltr" }, json!({}));
        let _ = app.run(align, json!({}));
        let _ = app.run("para.set", json!({"props": {"indentLeft": 18.0, "indentRight": 9.0}}));
        let before = props(&app);
        ok_unchanged(&mut app);
        let after = props(&app);
        assert_eq!((after.bidi, after.align), (before.bidi, before.align), "{align} in rtl={rtl}");
        assert_eq!((after.indent_left, after.indent_right), (Some(18.0), Some(9.0)), "{align} in rtl={rtl}");
    }
}

#[test]
fn paragraph_dialog_keeps_fixed_line_spacing() {
    use wordcraft_doc::props::LineSpacing;
    for (cmd, rule) in [(json!({"exactly": 24}), LineSpacing::Exactly(24.0)), (json!({"atLeast": 24}), LineSpacing::AtLeast(24.0))] {
        // OK without changes, and OK after changing only Keep with next (#467).
        let mut app = app_with("Example");
        let _ = app.run("para.lineSpacing", cmd.clone());
        ok_unchanged(&mut app);
        assert_eq!(props(&app).line_spacing, Some(rule), "unchanged OK, {cmd}");
        ok_edit(&mut app, |d| {
            if let Dialog::Paragraph { keep_next, .. } = d {
                *keep_next = true;
            }
        });
        let p = props(&app);
        assert_eq!((p.line_spacing, p.keep_next), (Some(rule), Some(true)), "Keep with next, {cmd}");
    }
    // A changed value still sets a multiple.
    let mut app = app_with("Example");
    let _ = app.run("para.lineSpacing", json!({"exactly": 24}));
    ok_edit(&mut app, |d| {
        if let Dialog::Paragraph { line, .. } = d {
            *line = 1.5;
        }
    });
    assert_eq!(props(&app).line_spacing, Some(LineSpacing::Multiple(1.5)));
}

#[test]
fn font_dialog_sets_the_asian_and_latin_fonts_separately() {
    // #505: the Asian text font box and the Latin text font box each set their own font, and
    // OK leaves the one not changed as it was.
    let mut app = app_with("Hello 你好");
    let _ = app.run("select.all", json!({}));
    let fonts = |app: &WordApp| {
        let c = app.session.doc.para_at(&Pos::body(0, 0)).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
        (c.font, c.font_ea)
    };
    let latin = fonts(&app).0;
    let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
    let Dialog::Font { asian, font_ea, .. } = &mut d else { panic!() };
    assert!(*asian, "Chinese text in the selection shows the Asian text font box");
    *font_ea = "SimSun".into();
    apply_font(&mut app, &d);
    assert_eq!(fonts(&app), (latin, Some("SimSun".into())));
    let Some(mut d) = Dialog::open("font", &mut app) else { panic!() };
    let Dialog::Font { font, font_ea, .. } = &mut d else { panic!() };
    assert_eq!(font_ea, "SimSun");
    *font = "Georgia".into();
    apply_font(&mut app, &d);
    assert_eq!(fonts(&app), (Some("Georgia".into()), Some("SimSun".into())));
    // One undo step each.
    let _ = app.run("edit.undo", json!({}));
    assert_eq!(fonts(&app).1.as_deref(), Some("SimSun"));
    assert_ne!(fonts(&app).0.as_deref(), Some("Georgia"));
}

#[test]
fn paragraph_dialog_changes_direction() {
    let mut app = app_with("Hello");
    // Right to left, aligned right on the page, 1" from the right edge.
    ok_edit(&mut app, |d| {
        if let Dialog::Paragraph { rtl, align, right, .. } = d {
            *rtl = true;
            *align = "right".into();
            *right += 1.0;
        }
    });
    let p = props(&app);
    assert_eq!(p.bidi, Some(true));
    assert_eq!(p.align, Some(Align::Left), "right on the page is the start edge");
    assert_eq!(p.indent_left, Some(72.0), "the right indent is the start indent");
    // A new direction alone keeps the alignment and indents where the dialog shows them.
    ok_edit(&mut app, |d| {
        if let Dialog::Paragraph { rtl, .. } = d {
            *rtl = false;
        }
    });
    let p = props(&app);
    assert_eq!((p.bidi, p.align, p.indent_left, p.indent_right), (Some(false), Some(Align::Right), Some(0.0), Some(72.0)));
}

/// Switching the Paragraph dialog's tabs re-centred it, so the tab strip jumped up and down
/// by half the height difference (#507): the top edge now stays put and only the bottom moves.
#[test]
fn switching_paragraph_tabs_keeps_the_top_edge() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with("Some text");
    app.dialog = Dialog::open("paragraph", &mut app);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    let mut tops = vec![];
    let mut oks = vec![];
    for page in [0u8, 1, 2, 0] {
        if let Some(Dialog::Paragraph { tab, .. }) = &mut h.state_mut().dialog {
            *tab = page;
        }
        for _ in 0..4 {
            h.step();
        }
        tops.push(h.get_by_label("Indents and Spacing").rect().top());
        oks.push(h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").rect().top());
    }
    assert!((oks[0] - oks[1]).abs() > 10.0, "the tabs differ in height: {oks:?}");
    for t in &tops {
        assert!((t - tops[0]).abs() <= 0.5, "the tab strip stays put: {tops:?}");
    }
}

/// #3663: "NaN" in the Font dialog's Size box parses as a float; the preview draws at 12 pt
/// instead of a NaN size, which tripped epaint's glyph assertion and closed the app.
#[test]
fn font_dialog_preview_ignores_a_size_that_is_not_a_number() {
    for (typed, pt) in [("NaN", 12.0), ("nan", 12.0), ("inf", 12.0), ("-infinity", 12.0), ("10.5", 10.5), ("200", 72.0), ("", 12.0)] {
        assert_eq!(preview_size(typed), pt, "{typed:?}");
    }
    let mut app = app_with("Example");
    let _ = app.run("select.all", json!({}));
    app.dialog = Dialog::open("font", &mut app);
    let Some(Dialog::Font { size, .. }) = &mut app.dialog else { panic!("no font dialog") };
    *size = "NaN".into();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    assert!(matches!(&h.state().dialog, Some(Dialog::Font { size, .. }) if size == "NaN"), "the dialog stays open");
}

#[test]
fn font_dialog_ok_keeps_what_the_user_left_alone() {
    // Issue #466: OK sent every field, clearing small caps, spacing and double underline
    // when only the size changed.
    let mut app = app_with("Example");
    let _ = app.run("select.all", json!({}));
    let _ = app.run("format.set", json!({"props": {"smallCaps": true, "caps": true, "hidden": true, "spacing": 2, "underline": "double"}}));
    let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
    if let Dialog::Font { size, small_caps, spacing, .. } = &mut d {
        assert!(*small_caps, "the dialog shows small caps");
        assert_eq!(*spacing, 2.0);
        *size = "14".into();
    }
    apply_font(&mut app, &d);
    let c = app.session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
    assert_eq!(c.size, Some(14.0));
    assert_eq!((c.small_caps, c.caps, c.hidden, c.spacing), (Some(true), Some(true), Some(true), Some(2.0)));
    assert_eq!(c.underline, Some(wordcraft_doc::props::Underline::Double));
    // A field the user does change still applies.
    let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
    if let Dialog::Font { small_caps, .. } = &mut d {
        *small_caps = false;
    }
    apply_font(&mut app, &d);
    let c = app.session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
    assert_eq!((c.small_caps, c.caps, c.size), (Some(false), Some(true), Some(14.0)));
}

/// Kerning for fonts and its Points and above size set `w:kern` on the selection in one undo
/// step; off is 0. Text that differs shows the box neither on nor off, and OK leaves each
/// run's own kerning.
#[test]
fn font_dialog_sets_kerning_for_fonts() {
    let mut app = app_with("Kerned plain");
    let _ = app.run("select.all", json!({}));
    let _ = app.run("format.set", json!({"props": {"kern": 0}}));
    let kern = |app: &WordApp, off| app.session.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props_of_char(off).kern);
    let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
    let Dialog::Font { kern: on, kern_at, .. } = &mut d else { panic!() };
    assert_eq!((*on, *kern_at), (Some(false), 12.0), "off, offering 12 pt");
    *on = Some(true);
    *kern_at = 16.0;
    apply_font(&mut app, &d);
    assert_eq!((kern(&app, 0), kern(&app, 7)), (Some(16.0), Some(16.0)));
    let _ = app.run("edit.undo", json!({}));
    assert_eq!(kern(&app, 0), Some(0.0), "one undo step");
    let _ = app.run("edit.redo", json!({}));
    let Some(mut d) = Dialog::open("font", &mut app) else { panic!() };
    let Dialog::Font { kern: on, kern_at, .. } = &mut d else { panic!() };
    assert_eq!((*on, *kern_at), (Some(true), 16.0));
    *on = Some(false);
    apply_font(&mut app, &d);
    assert_eq!((kern(&app, 0), kern(&app, 7)), (Some(0.0), Some(0.0)));
    let _ = app.run("select.text", json!({"text": "Kerned"}));
    let _ = app.run("format.set", json!({"props": {"kern": 14}}));
    let _ = app.run("select.all", json!({}));
    let Some(mut d) = Dialog::open("font", &mut app) else { panic!() };
    let Dialog::Font { kern: on, size, .. } = &mut d else { panic!() };
    assert_eq!(*on, None, "mixed kerning shows neither on nor off");
    *size = "13".into();
    apply_font(&mut app, &d);
    assert_eq!((kern(&app, 0), kern(&app, 7)), (Some(14.0), Some(0.0)), "each run keeps its kerning");
}

/// #1178: Modify Style's font box is the ribbon's font box, so typing part of a name picks the
/// installed family instead of storing the typed text.
#[test]
fn modify_style_font_box_finds_the_family() {
    use egui_kittest::kittest::{By, Queryable};
    let mut app = app_with("Example");
    app.dialog = Dialog::modify_style(&app, "Normal");
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        move |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            if fonts {
                show(app, &ctx);
            } else {
                crate::theme::install_fonts_for(&ctx, false);
                fonts = true;
            }
        },
        app,
    );
    h.run_steps(3);
    h.get(By::new().role(egui::accesskit::Role::TextInput).value(wordcraft_doc::styles::BODY_FONT)).focus();
    h.step();
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    h.event(key(egui::Key::A, egui::Modifiers::COMMAND));
    h.step();
    h.event(egui::Event::Text("aptos disp".into()));
    h.step();
    // Enter both picks the highlighted family and presses OK, as in the Font dialog.
    h.event(key(egui::Key::Enter, egui::Modifiers::NONE));
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "Enter pressed OK");
    let rc = h.state().session.doc.styles.resolve_char(Some("Normal"), &wordcraft_doc::CharProps::default());
    assert_eq!(rc.font, wordcraft_doc::styles::HEADING_FONT);
}

/// A dialog was pinned by its anchor, so dragging its title bar did nothing (#732).
#[test]
fn dragging_a_dialog_title_bar_moves_it() {
    let mut app = app_with("Some text");
    app.dialog = Dialog::open("paragraph", &mut app);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    let id = egui::Id::unique(("dialog", "Paragraph"));
    let before = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is open");
    let screen = h.ctx.content_rect();
    assert!((before.center().x - screen.center().x).abs() <= 1.0, "opens centred: {before:?} in {screen:?}");
    let grab = egui::pos2(before.center().x, before.top() + 10.0);
    let drop = grab + vec2(-200.0, 120.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.event(egui::Event::PointerMoved(grab));
    h.step();
    h.event(button(grab, true));
    h.step();
    h.event(egui::Event::PointerMoved(drop));
    h.step();
    h.event(button(drop, false));
    for _ in 0..4 {
        h.step();
    }
    let after = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is still open");
    assert!((after.min.x - (before.min.x - 200.0)).abs() <= 1.0, "moved left: {before:?} -> {after:?}");
    assert!((after.min.y - (before.min.y + 120.0)).abs() <= 1.0, "moved down: {before:?} -> {after:?}");
}

/// #1885: the close control is the whole box at the title bar's right end, as on a Windows
/// dialog, not only the ✕. A click 6 px from the window's right edge, mid title bar, closes
/// the dialog; one just under the title bar leaves it open; a drag that starts on the box
/// doesn't move the dialog. The box spans the title bar egui lays out.
#[test]
fn the_title_bar_close_box_is_the_whole_corner() {
    fn press(h: &mut egui_kittest::Harness<'_, WordApp>, from: egui::Pos2, to: egui::Pos2) {
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        h.event(egui::Event::PointerMoved(from));
        h.step();
        h.event(button(from, true));
        h.step();
        if to != from {
            h.event(egui::Event::PointerMoved(to));
            h.step();
        }
        h.event(button(to, false));
        for _ in 0..4 {
            h.step();
        }
    }
    let mut app = app_with("Some text");
    app.dialog = Dialog::open("wordCount", &mut app);
    let mut h = app_harness(app);
    for _ in 0..4 {
        h.step();
    }
    let id = egui::Id::unique(("dialog", "Word Count"));
    let before = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is open");
    let bx = crate::window_close::close_rect(before, 1.0, crate::window_close::geometry(&h.ctx));
    let title = h.ctx.read_response(id.with("__title_click")).expect("egui's title bar").rect;
    assert!((bx.top() - before.top()).abs() <= 1.5 && (bx.bottom() - title.bottom()).abs() <= 1.5, "box {bx:?} spans the title bar {title:?}");
    assert!(bx.width() >= 1.5 * bx.height() && bx.right() + 1.5 >= before.right(), "a wide box at the right end: {bx:?} in {before:?}");
    let under = egui::pos2(before.right() - 6.0, bx.bottom() + 4.0);
    press(&mut h, under, under);
    assert!(h.state().dialog.is_some(), "a click under the title bar keeps the dialog");
    let at = egui::pos2(before.right() - 6.0, bx.center().y);
    press(&mut h, at, at + vec2(-150.0, 90.0));
    assert!(h.state().dialog.is_some(), "a drag from the box isn't a click");
    let after = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is still open");
    assert!((after.min - before.min).length() <= 1.0, "the box isn't a drag handle: {before:?} -> {after:?}");
    press(&mut h, at, at);
    assert!(h.state().dialog.is_none(), "a click in the box, away from the X, closes the dialog");
}

/// #4177: Replace All in the Find and Replace dialog, with Regular expressions on, expands
/// capture groups (`$2 $1`, `\2 \1`, `${name}`) instead of inserting them literally.
#[test]
fn replace_all_with_regex_groups_from_the_dialog() {
    use egui_kittest::kittest::Queryable;
    for (with, want) in [("$2 $1", "John Smith and Jane Doe"), (r"\2 \1", "John Smith and Jane Doe"), ("${last}!", "Smith! and Doe!")] {
        let mut h = app_harness(app_with("Smith, John and Doe, Jane"));
        h.state_mut().run("ui.dialog", json!({"name": "replace"})).unwrap();
        h.run_steps(2);
        if let Some(Dialog::Find { query, replace, regex, .. }) = h.state_mut().dialog.as_mut() {
            *query = r"(?<last>\w+), (\w+)".into();
            *replace = with.into();
            *regex = true;
        } else {
            panic!("Replace opens the Find and Replace dialog");
        }
        h.run_steps(2);
        h.get_by_label("Replace All").click();
        h.run_steps(2);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), want, "{with}");
    }
}

/// #4404: a right-click in Find what or Replace with opens a menu with Cut, Copy, Paste and
/// Select All, as the system's text boxes have. The right-click keeps the selection (egui would
/// move the caret to it), Paste reads the system clipboard into the field, not the document, and
/// the dialog stays open.
#[test]
fn find_and_replace_fields_have_an_edit_menu() {
    use egui_kittest::kittest::Queryable;
    let services = crate::Services { clipboard_text: Some(Box::new(|| Some("glow".into()))), ..Default::default() };
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
    let _ = app.run("document.setText", json!({"text": "Lanterns"}));
    let mut h = app_harness(app);
    h.state_mut().run("ui.dialog", json!({"name": "replace"})).unwrap();
    h.run_steps(2);
    if let Some(Dialog::Find { query, .. }) = h.state_mut().dialog.as_mut() {
        *query = "old".into();
    }
    h.run_steps(2);
    let fields = |h: &egui_kittest::Harness<'_, WordApp>| match h.state().dialog.as_ref() {
        Some(Dialog::Find { query, replace, .. }) => (query.clone(), replace.clone()),
        _ => panic!("the dialog stays open"),
    };
    // Right-click a field, then pick a menu item (the ribbon has its own Cut and Paste, above).
    let menu = |h: &mut egui_kittest::Harness<'_, WordApp>, field: &str, item: &str| {
        let at = h.get_by_label(field).rect().center();
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
        h.event(egui::Event::PointerMoved(at));
        h.step();
        h.event(button(true));
        h.step();
        h.event(button(false));
        h.run_steps(2);
        let node = h.query_all_by_label(item).find(|n| n.rect().top() >= at.y).unwrap_or_else(|| panic!("{item} is in the menu"));
        node.click();
        h.run_steps(3);
    };
    menu(&mut h, "Find what:", "Select All");
    menu(&mut h, "Find what:", "Cut");
    assert_eq!(fields(&h).0, "", "Cut takes the text the right-click kept selected");
    menu(&mut h, "Find what:", "Paste");
    menu(&mut h, "Replace with:", "Paste");
    assert_eq!(fields(&h), ("glow".to_string(), "glow".to_string()), "Paste puts the clipboard's text in the field");
    // With no selection, a right-click moves the caret to the click, as on the page: Paste
    // goes there, not where the caret was (the end of the text).
    let field = h.get_by_label("Replace with:").rect();
    let start = egui::pos2(field.left() + 1.0, field.center().y);
    let button =
        |pressed| egui::Event::PointerButton { pos: start, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
    h.event(egui::Event::PointerMoved(start));
    h.step();
    h.event(button(true));
    h.step();
    h.event(button(false));
    h.run_steps(2);
    h.query_all_by_label("Paste").find(|n| n.rect().top() >= start.y).expect("Paste is in the menu").click();
    h.run_steps(3);
    assert_eq!(fields(&h).1, "glowglow");
    // Typing shows where the caret is: after the text pasted at the start, not at the end.
    h.event(egui::Event::Text("!".into()));
    h.run_steps(2);
    assert_eq!(fields(&h).1, "glow!glow", "pasted where the right-click put the caret");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Lanterns", "the document is untouched");
}

fn app_harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

/// The Page Setup paper size as opened, in points, and the Paper size entry it shows.
fn opened_paper(app: &mut WordApp) -> ((f32, f32), Option<&'static str>) {
    let Some(Dialog::PageSetup { paper, .. }) = Dialog::open("pageSetup", app) else { panic!("no page setup dialog") };
    let ppu = crate::options::unit().pt_per_unit();
    ((paper.0 * ppu, paper.1 * ppu), paper_entry(paper))
}

/// #1433: Page Setup's Paper size shows the named size the section has, in either
/// orientation, and Custom size (None) for anything else.
#[test]
fn page_setup_reads_the_paper_size() {
    let mut app = app_with("Page");
    let sect = |app: &WordApp| wordcraft_engine::cmd::page::sect(&app.session);
    let ((w, h), entry) = opened_paper(&mut app);
    assert!((w - sect(&app).page_w).abs() < 0.01 && (h - sect(&app).page_h).abs() < 0.01, "{w}×{h}");
    assert_eq!(entry, wordcraft_geom::paper_name(w, h));
    assert!(entry.is_some(), "a new document has a named size");
    let _ = app.run("layout.size", json!({"name": "A4"}));
    let _ = app.run("layout.orientation", json!({"value": "landscape"}));
    let ((w, h), entry) = opened_paper(&mut app);
    assert!(w > h, "landscape shows the wide side as the width: {w}×{h}");
    assert_eq!(entry, Some("A4"));
    let _ = app.run("layout.size", json!({"width": 300.0, "height": 500.0}));
    assert_eq!(opened_paper(&mut app).1, None, "Custom size");
}

/// #1433: a custom width and height applied with OK set the section's size (before the
/// orientation, so a wide custom size ends up landscape); an unchanged size doesn't run
/// `layout.size`, and switching orientation alone only turns the page.
#[test]
fn page_setup_applies_a_custom_paper_size() {
    use egui_kittest::kittest::Queryable;
    let ppu = crate::options::unit().pt_per_unit();
    let size_steps = |app: &WordApp| app.session.undo_labels().iter().filter(|l| l.as_str() == "Size").count();
    let sect = |app: &WordApp| {
        let s = wordcraft_engine::cmd::page::sect(&app.session);
        (s.page_w.round(), s.page_h.round(), s.landscape)
    };
    // OK on an unchanged dialog leaves the size alone.
    let mut h = app_harness(app_with("Page"));
    let before = sect(h.state());
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!((sect(h.state()), size_steps(h.state())), (before, 0), "no layout.size for an unchanged size");

    for ((w, ht), want) in [((400.0, 600.0), (400.0, 600.0, false)), ((800.0, 400.0), (800.0, 400.0, true))] {
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(2);
        if let Some(Dialog::PageSetup { paper, landscape, .. }) = h.state_mut().dialog.as_mut() {
            *paper = (w / ppu, ht / ppu);
            *landscape = w > ht;
        }
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(sect(h.state()), want, "{w}×{ht}");
    }
    assert_eq!(size_steps(h.state()), 2);

    // Portrait again with the same paper: the dialog swaps width and height, which is no new size.
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(2);
    if let Some(Dialog::PageSetup { paper, landscape, .. }) = h.state_mut().dialog.as_mut() {
        *paper = (paper.1, paper.0);
        *landscape = false;
    }
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(sect(h.state()), (400.0, 800.0, false));
    assert_eq!(size_steps(h.state()), 2, "turning the page isn't a new size");
}

/// #1433: the Paper group's own widgets: choosing A4 in Paper size fills Width and Height,
/// Landscape swaps them, OK gives an A4 landscape section; typing a width wider than the
/// height turns the orientation to Landscape and shows Custom size; choosing Custom size
/// keeps the numbers and stays chosen.
#[test]
fn page_setup_paper_widgets_work_together() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let ppu = crate::options::unit().pt_per_unit();
    let mut h = app_harness(app_with("Page"));
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(3);
    let state = |h: &egui_kittest::Harness<'_, WordApp>| match &h.state().dialog {
        Some(Dialog::PageSetup { paper, landscape, .. }) => {
            ((paper.0 * ppu * 100.0).round() / 100.0, (paper.1 * ppu * 100.0).round() / 100.0, *landscape)
        }
        _ => panic!("no page setup dialog"),
    };
    let combo = |h: &egui_kittest::Harness<'_, WordApp>| h.get_by_role_and_label(Role::ComboBox, "Paper size:").value();
    assert_eq!(combo(&h).as_deref(), Some("Letter"));
    h.get_by_role_and_label(Role::ComboBox, "Paper size:").click();
    h.run_steps(3);
    h.get_by_label("A4").click();
    h.run_steps(3);
    assert_eq!(state(&h), (595.28, 841.89, false), "A4 fills the fields");
    assert_eq!(combo(&h).as_deref(), Some("A4"));
    h.get_by_label("Landscape").click();
    h.run_steps(3);
    assert_eq!(state(&h), (841.89, 595.28, true), "Landscape swaps them");
    assert_eq!(combo(&h).as_deref(), Some("A4"));
    h.get_by_label("OK").click();
    h.run_steps(3);
    let sect = wordcraft_engine::cmd::page::sect(&h.state().session);
    assert_eq!(((sect.page_w * 100.0).round() / 100.0, (sect.page_h * 100.0).round() / 100.0, sect.landscape), (841.89, 595.28, true));

    // Portrait A4 again; a width wider than the height turns it to Landscape, Custom size.
    h.state_mut().run("layout.orientation", json!({"value": "portrait"})).unwrap();
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(3);
    h.get_by_value("8.27\"").focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("15".into()));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(state(&h), (1080.0, 841.89, true), "wider than tall: Landscape");
    assert_eq!(combo(&h).as_deref(), Some("Custom size"));

    // Back to A4, then Custom size: the numbers stay and so does the choice.
    h.get_by_role_and_label(Role::ComboBox, "Paper size:").click();
    h.run_steps(3);
    h.get_by_label("A4").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ComboBox, "Paper size:").click();
    h.run_steps(3);
    h.get_by_label("Custom size").click();
    h.run_steps(3);
    assert_eq!(state(&h), (841.89, 595.28, true));
    assert_eq!(combo(&h).as_deref(), Some("Custom size"));
}

/// #1433: Page Setup checks the new size and margins together: a page its margins don't fit
/// keeps the dialog open with a message and applies nothing; a smaller page with smaller
/// margins applies (margins first, so no step is refused on the way).
#[test]
fn page_setup_refuses_margins_that_dont_fit_and_applies_smaller_ones_first() {
    use egui_kittest::kittest::Queryable;
    let ppu = crate::options::unit().pt_per_unit();
    let mut h = app_harness(app_with("Page"));
    let before = wordcraft_engine::cmd::page::sect(&h.state().session);
    let undo = h.state().session.undo_depth();
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(2);
    if let Some(Dialog::PageSetup { paper, .. }) = h.state_mut().dialog.as_mut() {
        *paper = (100.0 / ppu, 500.0 / ppu);
    }
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(
        matches!(&h.state().dialog, Some(Dialog::PageSetup { message, .. }) if message.contains("margins don't fit")),
        "stays open with a message"
    );
    assert_eq!((wordcraft_engine::cmd::page::sect(&h.state().session), h.state().session.undo_depth()), (before, undo), "nothing applied");

    if let Some(Dialog::PageSetup { top, bottom, left, right, .. }) = h.state_mut().dialog.as_mut() {
        (*top, *bottom, *left, *right) = (18.0 / ppu, 18.0 / ppu, 18.0 / ppu, 18.0 / ppu);
    }
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    let s = wordcraft_engine::cmd::page::sect(&h.state().session);
    assert_eq!((s.page_w.round(), s.page_h.round(), s.margin_left.round(), s.margin_top.round()), (100.0, 500.0, 18.0, 18.0));
}

/// Open Page Setup in `h`, set its paper and margins (points) and press OK.
fn page_setup_ok(h: &mut egui_kittest::Harness<'_, WordApp>, (w, ht): (f32, f32), (t, b, l, r): (f32, f32, f32, f32)) {
    use egui_kittest::kittest::Queryable;
    let ppu = crate::options::unit().pt_per_unit();
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(2);
    if let Some(Dialog::PageSetup { paper, landscape, top, bottom, left, right, .. }) = h.state_mut().dialog.as_mut() {
        *paper = (w / ppu, ht / ppu);
        *landscape = w > ht;
        (*top, *bottom, *left, *right) = (t / ppu, b / ppu, l / ppu, r / ppu);
    }
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
}

/// #1433: one OK in Page Setup is one undo step, whichever order its commands run in (the
/// size first; the margins first; margins of 0 in between), and Undo restores the section.
#[test]
fn page_setup_ok_is_one_undo_step() {
    let sect = |h: &egui_kittest::Harness<'_, WordApp>| wordcraft_engine::cmd::page::sect(&h.state().session);
    // Size first, then orientation and margins.
    let mut h = app_harness(app_with("Page"));
    let (before, depth) = (sect(&h), h.state().session.undo_depth());
    page_setup_ok(&mut h, (400.0, 600.0), (36.0, 36.0, 36.0, 36.0));
    assert!(h.state().dialog.is_none());
    assert_eq!((sect(&h).page_w, sect(&h).margin_left), (400.0, 36.0));
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(sect(&h), before, "one undo restores the section");

    // Neither size first nor margins first fits on the way: margins of 0 in between.
    h.state_mut().run("layout.margins", json!({"top": 72.0, "bottom": 72.0, "left": 250.0, "right": 250.0})).unwrap();
    let (before, depth) = (sect(&h), h.state().session.undo_depth());
    page_setup_ok(&mut h, (300.0, 900.0), (400.0, 400.0, 36.0, 36.0));
    assert!(h.state().dialog.is_none());
    let s = sect(&h);
    assert_eq!((s.page_w, s.page_h, s.margin_top, s.margin_left), (300.0, 900.0, 400.0, 36.0));
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(sect(&h), before, "one undo restores the section");
}

/// #1433: a command the dialog runs that fails (here `layout.size`, refused by the second
/// selected section's margins) shows its error, keeps the dialog open and leaves the
/// document and the undo stack as they were.
#[test]
fn page_setup_shows_a_refused_command_and_applies_nothing() {
    let mut app = app_with("one");
    app.run("layout.break", json!({"kind": "nextPage"})).unwrap();
    app.run("text.insert", json!({"text": "two"})).unwrap();
    app.run("layout.margins", json!({"left": 150.0, "right": 150.0})).unwrap();
    // Both sections selected, the first one's settings in the dialog.
    app.run("select.all", json!({})).unwrap();
    let sel = &mut app.session.sel;
    std::mem::swap(&mut sel.anchor, &mut sel.focus);
    let doc = app.session.doc.sections().iter().map(|(_, x)| (*x).clone()).collect::<Vec<_>>();
    let depth = app.session.undo_depth();
    let mut h = app_harness(app);
    page_setup_ok(&mut h, (300.0, 500.0), (72.0, 72.0, 72.0, 72.0));
    assert!(
        matches!(&h.state().dialog, Some(Dialog::PageSetup { message, .. }) if message.contains("margins don't fit")),
        "stays open with the error"
    );
    let after = h.state().session.doc.sections().iter().map(|(_, x)| (*x).clone()).collect::<Vec<_>>();
    assert_eq!((after, h.state().session.undo_depth()), (doc, depth), "nothing applied");
}

/// #1433: Layout › Size › More Paper Sizes… opens Page Setup.
#[test]
fn more_paper_sizes_opens_page_setup() {
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(400.0, 600.0))
        .build_ui_state(|ui, app: &mut WordApp| crate::ribbon::size_menu(ui, app), app_with("Page"));
    h.run_steps(2);
    h.get(egui_kittest::kittest::By::new().label_contains("More Paper Sizes…")).click();
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("pageSetup"));
}

/// #1320: Page Setup shows the section's vertical alignment, and OK applies a change.
#[test]
fn page_setup_dialog_sets_vertical_alignment() {
    use egui_kittest::kittest::Queryable;
    use wordcraft_doc::section::PageVAlign;
    let mut app = app_with("Some text");
    let _ = app.run("layout.verticalAlign", json!({"value": "bottom"}));
    app.dialog = Dialog::open("pageSetup", &mut app);
    assert!(matches!(app.dialog, Some(Dialog::PageSetup { valign: PageVAlign::Bottom, .. })), "shows the section's value");
    if let Some(Dialog::PageSetup { valign, .. }) = &mut app.dialog {
        *valign = PageVAlign::Justify;
    }
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Vertical alignment:");
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Characters per line:");
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Lines per page:");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    for _ in 0..4 {
        h.step();
    }
    assert_eq!(wordcraft_engine::cmd::page::sect(&h.state().session).valign, PageVAlign::Justify);
}

/// #1230: Page Setup's Different first page check box gives the title page no page number
/// (its footer is the empty first-page one), in one undo step.
#[test]
fn page_setup_different_first_page_leaves_the_title_page_unnumbered() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with("Title");
    app.run("insert.pageBreak", json!({})).unwrap();
    app.run("text.insert", json!({"text": "Body"})).unwrap();
    app.run("insert.pageNumber", json!({"position": "bottom", "align": "center"})).unwrap();
    let _ = app.run("insert.closeHeader", json!({}));
    assert!(app.session.layout().pages[0].footer_story.is_some(), "page 1 is numbered");
    app.dialog = Dialog::open("pageSetup", &mut app);
    assert!(matches!(app.dialog, Some(Dialog::PageSetup { first_page: false, odd_even: false, .. })));
    let depth = app.session.undo_depth();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..4 {
        h.step();
    }
    assert!(h.query_by_label("Different Odd & Even Pages").is_some(), "the dialog has both check boxes");
    h.get_by_label("Different First Page").click();
    for _ in 0..2 {
        h.step();
    }
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    for _ in 0..4 {
        h.step();
    }
    let s = &mut h.state_mut().session;
    assert!(wordcraft_engine::cmd::page::sect(s).title_page);
    assert!(!s.doc.settings.even_odd_headers, "left as it was");
    assert_eq!(s.undo_depth(), depth + 1);
    let pages = &s.layout().pages;
    assert!(pages[0].footer_story.is_none(), "the title page shows no page number");
    assert!(pages[1].footer_story.is_some(), "the next page keeps its number");
}

/// #2701: in the SmartArt Text Pane, Tab demotes and Shift+Tab promotes the focused item, as
/// Word's pane does; the row keeps its focus, so the next Tab acts on the same item. An item
/// that can't go down a level (the one before it isn't deeper) is left as it is.
#[test]
fn text_pane_tab_demotes_and_shift_tab_promotes_the_focused_item() {
    use egui_kittest::kittest::Queryable;
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.run("insert.smartArt", json!({"layout": "hierarchy"})).unwrap();
    app.run("smartArt.textPane", json!({"value": true})).unwrap();
    let levels = |app: &WordApp| {
        wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).map(|g| g.items.iter().map(|i| i.level).collect::<Vec<u8>>())
    };
    assert_eq!(levels(&app), Some([0, 1, 2, 1, 2, 2].to_vec()), "Lead, Design, Sketches, Build, Code, Tests");
    let mut h = app_harness(app);
    h.run_steps(2);
    // Focus a pane row: the text box with this value (the graphic drawn on the page shows the
    // same words as shape names, so match by role).
    let focus_row = |h: &mut egui_kittest::Harness<'static, WordApp>, text: &str| {
        let node = h
            .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::TextInput).value(text))
            .next()
            .unwrap_or_else(|| panic!("no text box for {text}"));
        node.focus();
        h.run_steps(2);
    };
    let tab = |h: &mut egui_kittest::Harness<'static, WordApp>, shift: bool| {
        if shift {
            h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        } else {
            h.key_press(egui::Key::Tab);
        }
        h.run_steps(2);
    };
    // Focus "Design" (the second top-level item, at level 1): Tab can't demote it, because
    // the item before it ("Lead") is shallower — Word leaves the level alone.
    focus_row(&mut h, "Design");
    assert_eq!(h.state().smart_art_item, Some(1), "the pane remembers the focused row");
    tab(&mut h, false);
    assert_eq!(levels(h.state()), Some([0, 1, 2, 1, 2, 2].to_vec()), "Tab on an item that can't go down changes nothing");
    assert_eq!(h.state().smart_art_item, Some(1), "the row keeps focus after a refused demote");
    // Focus "Build" (level 1 after the deeper "Sketches"): Tab demotes it and its sub-items,
    // and the next Tab (the row stayed focused) demotes them again; Shift+Tab promotes back.
    focus_row(&mut h, "Build");
    tab(&mut h, false);
    assert_eq!(levels(h.state()), Some([0, 1, 2, 2, 3, 3].to_vec()), "Tab demotes the focused item");
    tab(&mut h, false);
    assert_eq!(levels(h.state()), Some([0, 1, 2, 3, 4, 4].to_vec()), "the same row takes the next Tab");
    tab(&mut h, true);
    assert_eq!(levels(h.state()), Some([0, 1, 2, 2, 3, 3].to_vec()), "Shift+Tab promotes them back");
}

/// Type into a dialog text field, replacing what is there (as a user would: click, select
/// all, type over).
fn set_text_field(h: &mut egui_kittest::Harness<'_, WordApp>, label: &str, text: &str) {
    use egui_kittest::kittest::Queryable;
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).type_text(text);
    h.run_steps(2);
}

/// #2982: the list keeps its full values but previews only the first line, at most 80
/// characters, with an ellipsis when either is cut.
#[test]
fn custom_property_previews_stay_on_one_bounded_line() {
    assert_eq!(custom_value_preview("short"), "short");
    assert_eq!(custom_value_preview(&"x".repeat(80)), "x".repeat(80), "exactly the limit is not cut");
    assert_eq!(custom_value_preview(&"x".repeat(81)), format!("{}…", "x".repeat(80)));
    assert_eq!(custom_value_preview("first\nsecond"), "first…", "a later line is cut");
    let wide = "é".repeat(80);
    assert_eq!(custom_value_preview(&wide), wide, "80 characters, not 80 bytes");
    let wide = "é".repeat(81);
    assert_eq!(custom_value_preview(&wide).chars().count(), 81, "80 characters plus the ellipsis");
    assert!(custom_value_preview(&wide).ends_with('…'));
}

/// #2982: File › Info's Advanced Properties… opens the Custom Properties dialog; it lists
/// the document's custom properties, and Add / Modify / Delete edit a working copy that OK
/// applies as one undoable `file.properties` step, Cancel discarding it.
#[test]
fn the_custom_properties_dialog_adds_edits_and_deletes_in_one_step() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft", "Owner": "Mira"}}));
    let _ = app.run("ui.backstage", json!({"value": true, "page": "info"}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("Advanced Properties…").click();
    h.run_steps(4);
    assert!(matches!(&h.state().dialog, Some(Dialog::CustomProperties(_))), "opens from File › Info");
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("customProperties"));
    assert!(h.query_by_label("Status = draft").is_some(), "the list shows the properties");
    // A row is selected, so the buttons act on it.
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").accesskit_node().is_disabled());
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").accesskit_node().is_disabled());
    // Add one through the form.
    set_text_field(&mut h, "Name:", "Reviewer");
    set_text_field(&mut h, "Value:", "Ada");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Add").click();
    h.run_steps(2);
    assert!(h.query_by_label("Reviewer = Ada").is_some(), "Add appends the typed property");
    // A duplicate name in another case replaces its value.
    set_text_field(&mut h, "Name:", "status");
    set_text_field(&mut h, "Value:", "final");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Add").click();
    h.run_steps(2);
    assert!(h.query_by_label("Status = final").is_some(), "the duplicate replaced the value");
    // Selecting a row fills the boxes; Modify changes its value.
    h.get_by_label("Reviewer = Ada").click();
    h.run_steps(2);
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Name:").value().as_deref(), Some("Reviewer"));
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Value:").value().as_deref(), Some("Ada"));
    set_text_field(&mut h, "Value:", "Ada L.");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").click();
    h.run_steps(2);
    assert!(h.query_by_label("Reviewer = Ada L.").is_some(), "Modify keeps the edited value");
    // Delete another property, so the modified value is saved before that one goes.
    h.get_by_label("Owner = Mira").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").click();
    h.run_steps(2);
    assert!(h.query_by_label("Owner = Mira").is_none(), "Delete removes the selected property");
    // OK: one call, so one undo step, marking the document changed.
    let depth = h.state().session.undo_depth();
    h.state_mut().session.dirty = false;
    h.step();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    h.run_steps(4);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
    let props: Vec<(String, String)> = h.state().session.doc.custom_props.iter().map(|p| (p.name.clone(), p.value.clone())).collect();
    assert_eq!(props, [("Status".to_string(), "final".to_string()), ("Reviewer".to_string(), "Ada L.".to_string())]);
    assert!(h.state().session.dirty, "the document is marked changed");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
    // One undo brings the old values back.
    let _ = h.state_mut().run("edit.undo", json!({}));
    assert_eq!(h.state().session.doc.custom_prop("Status"), Some("draft"), "one undo step");
    assert_eq!(h.state().session.doc.custom_prop("Owner"), Some("Mira"));
    assert_eq!(h.state().session.doc.custom_prop("Reviewer"), None);
}

/// #2982: the highlighted first row is ready to modify: its name and value fill the boxes,
/// so clicking Modify straight away is not refused for an empty name.
#[test]
fn the_custom_properties_dialog_starts_ready_to_modify_the_first_property() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft"}}));
    let _ = app.run("ui.backstage", json!({"value": true, "page": "info"}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("Advanced Properties…").click();
    h.run_steps(4);
    let role = egui::accesskit::Role::TextInput;
    assert_eq!(h.get_by_role_and_label(role, "Name:").value().as_deref(), Some("Status"));
    assert_eq!(h.get_by_role_and_label(role, "Value:").value().as_deref(), Some("draft"));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").click();
    h.run_steps(2);
    assert!(h.query_by_label("Property names are 1–255 characters.").is_none(), "the prefilled name is accepted");
    assert!(h.query_by_label("Status = draft").is_some(), "Modify with no edits keeps the property");
    // A real edit through the same prefilled form reaches the document as one undo step.
    let depth = h.state().session.undo_depth();
    set_text_field(&mut h, "Value:", "final");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    h.run_steps(4);
    assert_eq!(h.state().session.doc.custom_prop("Status"), Some("final"));
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undoable change");
}

/// #2982: a rename that changes only the case of the name is still a rename. Modify and OK
/// must send the old spelling as a removal and the new one as a value, or the stored
/// spelling never changes.
#[test]
fn the_custom_properties_dialog_keeps_a_case_only_rename() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft"}}));
    let _ = app.run("ui.backstage", json!({"value": true, "page": "info"}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("Advanced Properties…").click();
    h.run_steps(4);
    // Select the property and change only the case of its name, keeping the value.
    h.get_by_label("Status = draft").click();
    h.run_steps(2);
    set_text_field(&mut h, "Name:", "status");
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Value:").value().as_deref(), Some("draft"));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").click();
    h.run_steps(2);
    assert!(h.query_by_label("status = draft").is_some(), "the list shows the new spelling");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    h.run_steps(4);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
    let props: Vec<(String, String)> = h.state().session.doc.custom_props.iter().map(|p| (p.name.clone(), p.value.clone())).collect();
    assert_eq!(props, [("status".to_string(), "draft".to_string())], "the stored spelling follows the rename");
}

/// #2982: Cancel leaves the document as it was; empty names are refused with a message in
/// the dialog; the Name box stops at Word's limit; and the last deletion shows the empty
/// state with Modify and Delete disabled.
#[test]
fn the_custom_properties_dialog_cancel_changes_nothing_and_refuses_bad_names() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft"}}));
    let _ = app.run("ui.backstage", json!({"value": true, "page": "info"}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("Advanced Properties…").click();
    h.run_steps(4);
    let depth = h.state().session.undo_depth();
    // Empty and blank names are refused, with the message shown, keeping the list as it was.
    set_text_field(&mut h, "Name:", "");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Add").click();
    h.run_steps(2);
    assert!(h.query_by_label("Property names are 1–255 characters.").is_some(), "an empty name shows the message");
    set_text_field(&mut h, "Name:", "   ");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Add").click();
    h.run_steps(2);
    assert!(h.query_by_label("Property names are 1–255 characters.").is_some(), "a whitespace name shows the message");
    assert!(h.query_by_label("Status = draft").is_some(), "nothing was added");
    // The Name box caps typing at Word's limit.
    set_text_field(&mut h, "Name:", &"x".repeat(wordcraft_engine::cmd::file::MAX_CUSTOM_NAME + 20));
    let typed = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Name:").value().unwrap_or_default();
    assert_eq!(typed.chars().count(), wordcraft_engine::cmd::file::MAX_CUSTOM_NAME, "the Name box stops at the limit");
    // A valid entry, so Cancel has something to discard.
    set_text_field(&mut h, "Name:", "Reviewer");
    set_text_field(&mut h, "Value:", "Ada");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Add").click();
    h.run_steps(2);
    assert!(h.query_by_label("Reviewer = Ada").is_some());
    // Deleting every row shows the empty state and disables Modify and Delete.
    h.get_by_label("Reviewer = Ada").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").click();
    h.run_steps(2);
    h.get_by_label("Status = draft").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").click();
    h.run_steps(2);
    assert!(h.query_by_label("This document has no custom properties.").is_some(), "the empty list says so");
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").accesskit_node().is_disabled());
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").accesskit_node().is_disabled());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    h.run_steps(4);
    assert!(h.state().dialog.is_none(), "Cancel closes the dialog");
    let names: Vec<&str> = h.state().session.doc.custom_props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Status"], "the document is untouched");
    assert_eq!(h.state().session.undo_depth(), depth, "no step was recorded");
}

/// #2982: imported non-text properties (a number, Zotero's raw XML) stay as they are: Add
/// and Modify refuse to turn one into text, while Delete still removes it.
#[test]
fn the_custom_properties_dialog_leaves_imported_non_text_properties_alone() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft"}}));
    app.session.doc.custom_props.push(wordcraft_doc::CustomProp { name: "Count".into(), kind: "i4".into(), value: "7".into() });
    let _ = app.run("ui.backstage", json!({"value": true, "page": "info"}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("Advanced Properties…").click();
    h.run_steps(4);
    assert!(h.query_by_label("Count = 7").is_some(), "the imported property is listed");
    // Add with its name is refused rather than replacing it with text.
    set_text_field(&mut h, "Name:", "count");
    set_text_field(&mut h, "Value:", "8");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Add").click();
    h.run_steps(2);
    assert!(h.query_by_label("Only text properties can be changed.").is_some(), "Add refuses the non-text name");
    assert!(h.query_by_label("Count = 7").is_some(), "it keeps its value");
    // Modify of the non-text property itself is refused.
    h.get_by_label("Count = 7").click();
    h.run_steps(2);
    set_text_field(&mut h, "Value:", "8");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").click();
    h.run_steps(2);
    assert!(h.query_by_label("Only text properties can be changed.").is_some(), "Modify refuses the non-text property");
    assert!(h.query_by_label("Count = 7").is_some(), "its value is unchanged");
    // Renaming a text property onto a non-text one is refused too.
    h.get_by_label("Status = draft").click();
    h.run_steps(2);
    set_text_field(&mut h, "Name:", "Count");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").click();
    h.run_steps(2);
    assert!(h.query_by_label("Only text properties can be changed.").is_some(), "a rename onto it is refused");
    assert!(h.query_by_label("Status = draft").is_some(), "the text property keeps its name");
    // Delete still removes it outright.
    h.get_by_label("Count = 7").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").click();
    h.run_steps(2);
    assert!(h.query_by_label("Count = 7").is_none(), "Delete removes a non-text property");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    h.run_steps(4);
    let kinds: Vec<(&str, &str)> = h.state().session.doc.custom_props.iter().map(|p| (p.name.as_str(), p.kind.as_str())).collect();
    assert_eq!(kinds, [("Status", "lpwstr"), ("Count", "i4")], "Cancel leaves the imported property as it was");
}
