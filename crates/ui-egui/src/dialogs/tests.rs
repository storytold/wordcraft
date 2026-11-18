use super::*;
use craft::ui::dialog::{DialogId, HeldOk};
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value as Json, json};
use wordcraft_doc::Pos;
use wordcraft_doc::props::Align;
use wordcraft_engine::Session;

use super::bookmarks::bookmark_names;
use super::find::goto_page;
use super::host::{FormHost, close_all};
use craft::text::ui::character::ParagraphFormat;
use crate::dialogs::paragraph::ParagraphHost; // PLAN: assumed (dialogs-w15): `ParagraphHost = FormHost<ParagraphForm>`, `ParagraphForm { pub format: ParagraphFormat }`
use crate::dialogs::font::FontForm;

/// The Font dialog on the host, as `open_form` opens it.
type FontHost = FormHost<FontForm>;
use crate::dialogs::page_setup::{PageSetupForm, PageSetupHost}; // PLAN: assumed (dialogs-w18 note 5): `PageSetupHost = FormHost<PageSetupForm>`; the form holds `paper`, `landscape`, margins and `valign` in points

// ---------------------------------------------------------------- helpers

fn body_of(app: &WordApp) -> String {
    app.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

fn app_with(text: &str) -> WordApp {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let _ = app.run("document.setText", json!({"text": text}));
    app
}

fn props(app: &WordApp) -> wordcraft_doc::props::ParaProps {
    app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default()
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

/// The keys of the open dialogs, bottom first.
fn open_keys(app: &WordApp) -> Vec<String> {
    app.dialog_host.open_dialogs().into_iter().filter_map(|d| d.key).collect()
}

fn is_open(app: &WordApp, key: &str) -> bool {
    open_keys(app).iter().any(|k| k == key)
}

fn id_of(app: &WordApp, key: &str) -> DialogId {
    app.dialog_host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)).map(|d| d.id).unwrap_or_else(|| panic!("{key} is not open"))
}

/// The JSON fields of the open dialog `key`.
fn fields_of(app: &WordApp, key: &str) -> Json {
    app.dialog_host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)).and_then(|d| d.fields).unwrap_or(Json::Null)
}

/// Opens the named dialog through the name table and lets it show.
fn open_named(h: &mut egui_kittest::Harness<'_, WordApp>, name: &str, params: Json) -> DialogId {
    let opened = super::open(h.state_mut(), name, &params);
    h.run_steps(2);
    opened.unwrap_or_else(|| panic!("{name} opens nothing"))
}

fn set_field(h: &mut egui_kittest::Harness<'_, WordApp>, key: &str, field: &str, value: Json) {
    let id = id_of(h.state(), key);
    let ctx = h.ctx.clone();
    h.state_mut().dialog_host.set_json_field(&ctx, id, field, value).unwrap_or_else(|e| panic!("{key}.{field}: {e}"));
    h.run_steps(2);
}

fn press(h: &mut egui_kittest::Harness<'_, WordApp>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(3);
}

fn confirm(h: &mut egui_kittest::Harness<'_, WordApp>, id: DialogId) {
    let ctx = h.ctx.clone();
    h.state_mut().dialog_host.confirm_by_id(&ctx, id);
    h.run_steps(3);
}

/// Opens the Paragraph dialog, edits what it shows and answers OK.
fn apply_paragraph(h: &mut egui_kittest::Harness<'_, WordApp>, edit: impl FnOnce(&mut ParagraphFormat)) {
    let id = open_named(h, "paragraph", json!({}));
    if let Some(host) = h.state_mut().dialog_host.get_mut_by_id::<ParagraphHost>(id) {
        edit(&mut host.form_mut().format);
    }
    confirm(h, id);
}

/// What the Paragraph dialog shows for the selection.
fn shown_paragraph(h: &mut egui_kittest::Harness<'_, WordApp>) -> ParagraphFormat {
    let id = open_named(h, "paragraph", json!({}));
    let shown = h.state().dialog_host.get_by_id::<ParagraphHost>(id).map(|d| d.form().format.clone()).unwrap_or_default();
    let ctx = h.ctx.clone();
    h.state_mut().dialog_host.cancel_by_id(&ctx, id);
    h.run_steps(2);
    shown
}

/// Opens the Font dialog, edits what it shows and answers OK.
fn apply_font(h: &mut egui_kittest::Harness<'_, WordApp>, edit: impl FnOnce(&mut FontForm)) {
    let id = open_named(h, "font", json!({}));
    if let Some(host) = h.state_mut().dialog_host.get_mut_by_id::<FontHost>(id) {
        edit(host.form_mut());
    }
    confirm(h, id);
}

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

fn char_props_at_start(app: &WordApp) -> wordcraft_doc::props::CharProps {
    app.session.doc.para_at(&Pos::body(0, 0)).map(|p| p.props_of_char(0).clone()).unwrap_or_default()
}

// ---------------------------------------------------------------- Paste Special, Insert Table, Go To

/// Paste Special lists and pastes the same formats the engine chose: the clipboard ownership
/// result must survive into the dialog's payload, or another program's copy could be replaced
/// by WordCraft's own fragment.
#[test]
fn paste_special_dialog_keeps_the_clipboard_ownership_result() {
    let a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("")), crate::Services::default());
    // PLAN: assumed `PasteSpecialForm::read(&WordApp, &Value) -> PasteSpecialForm { formats: Vec<String>, payload: Value, .. }` (dialogs-w18 note 3)
    let form = super::paste_special::PasteSpecialForm::read(&a, &json!({"text": "Bold", "privateClipboard": false}));
    assert_eq!(form.formats, ["text"], "another program's copy isn't our fragment");
    assert_eq!(form.payload.get("privateClipboard"), Some(&json!(false)));
    assert_eq!(form.payload.get("text").and_then(Value::as_str), Some("Bold"));
}

/// #3931: hovering Insert › Table's grid asks the page for a preview of that table (rows by
/// columns) and changes nothing; leaving the grid drops it; a click inserts the table with no
/// preview left over for the page to show on top of it.
#[test]
fn the_table_grid_previews_the_hovered_table_until_the_click() {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
    a.run("ui.tab", json!({"tab": "Insert"})).unwrap();
    let mut h = app_harness(a);
    h.run_steps(4);
    h.get_by_label("Add a Table").click();
    h.run_steps(3);
    // The row 3, column 4 square: the smallest node that names a 4×3 table.
    // PLAN: assumed the picker's squares are named "<columns>×<rows> …" (ribbon-w16, `TableSizePicker`).
    let at = h
        .query_all(egui_kittest::kittest::By::new().label_contains("4×3"))
        .min_by(|a, b| (a.rect().width() * a.rect().height()).total_cmp(&(b.rect().width() * b.rect().height())))
        .expect("the grid's 4×3 square")
        .rect()
        .center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    assert_eq!(h.state().canvas.table_preview, Some((3, 4)), "three rows of four columns");
    assert!(h.query_all(egui_kittest::kittest::By::new().label_contains("4×3")).next().is_some(), "the label reads columns × rows");
    assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()) && !h.state().session.can_undo());
    h.event(egui::Event::PointerMoved(egui::pos2(1400.0, 880.0)));
    h.run_steps(3);
    assert_eq!(h.state().canvas.table_preview, None, "off the grid nothing is previewed");
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
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

// ---------------------------------------------------------------- Bookmark, Macros

/// Insert › Bookmark (#1236), as in Word: it opens on the bookmark around the caret, lists by
/// name, Go To selects a bookmark's text and Delete removes one — both leaving the dialog
/// open, Cancel then reading Close — and one undo brings the bookmark back.
#[test]
fn the_bookmark_dialog_goes_to_and_deletes_bookmarks() {
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
    let mut h = app_harness(app);
    h.run_steps(2);
    open_named(&mut h, "bookmark", json!({}));
    h.run_steps(2);
    // PLAN: assumed the Bookmark dialog's `json_fields` carry the name box as `name`.
    assert_eq!(fields_of(h.state(), "bookmark").get("name").and_then(Value::as_str), Some("Yankee"), "opens on the caret's bookmark");

    h.get_by_label("Zulu").click();
    h.run_steps(3);
    press(&mut h, "Go To");
    assert_eq!(h.state().session.selected_text(), "beta");
    assert!(is_open(h.state(), "bookmark"), "Go To leaves the dialog open");
    h.get_by_label("Zulu").click();
    h.run_steps(3);
    press(&mut h, "Delete");
    let names: Vec<String> = h.state().session.doc.bookmarks().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["_Hidden", "Yankee"]);
    assert!(is_open(h.state(), "bookmark"), "Delete leaves the dialog open");
    assert_eq!(fields_of(h.state(), "bookmark").get("name").and_then(Value::as_str), Some(""), "Delete clears the name box");
    assert_eq!(h.state().session.selected_text(), "beta", "the text stays selected");
    press(&mut h, "Close");
    assert!(!is_open(h.state(), "bookmark"));
    let _ = h.state_mut().run("edit.undo", json!({}));
    assert_eq!(h.state().session.doc.bookmarks().len(), 3, "one undo step");
    // Add inserts the typed name at the selection.
    open_named(&mut h, "bookmark", json!({}));
    set_field(&mut h, "bookmark", "name", json!("Fresh"));
    press(&mut h, "Add");
    assert!(h.state().session.doc.bookmarks().iter().any(|(n, _)| n == "Fresh"), "Add inserts the typed bookmark");
}

/// #4327: View › Macros opens a dialog that runs and records macros by the exact name listed
/// or typed; the command with a name to run or delete (what scripts send) still acts without one.
#[test]
fn the_macros_button_opens_a_dialog_that_runs_deletes_and_records() {
    let mut app = app_with("Start");
    for name in ["Greet", "Spare"] {
        app.run("tools.macros", json!({"define": {"name": name, "steps": [{"command": "text.insert", "params": {"text": name}}]}})).unwrap();
    }
    assert!(open_keys(&app).is_empty(), "defining a macro opens nothing");
    assert_eq!(app.run("tools.macros", json!({})).unwrap(), json!({"pending": "macros"}));
    let mut h = app_harness(app);
    h.run_steps(4);
    assert!(is_open(h.state(), "macros"));

    h.get_by_label("Greet").click();
    h.run_steps(3);
    press(&mut h, "Run");
    assert!(!is_open(h.state(), "macros"));
    assert!(body_of(h.state()).contains("Greet"), "the macro ran");

    // A macro whose name has spaces around it (only a script can make one) runs and is deleted by its exact listed name.
    let define_pad = |h: &mut egui_kittest::Harness<'_, WordApp>| {
        h.state_mut()
            .run("tools.macros", json!({"define": {"name": " Pad ", "steps": [{"command": "text.insert", "params": {"text": "Padded"}}]}}))
            .unwrap();
    };
    define_pad(&mut h);
    open_named(&mut h, "macros", json!({}));
    h.get_by_label(" Pad ").click();
    h.run_steps(3);
    press(&mut h, "Run");
    assert!(!is_open(h.state(), "macros"));
    assert!(body_of(h.state()).contains("Padded"), "the macro with spaces in its name ran");
    define_pad(&mut h);
    open_named(&mut h, "macros", json!({}));
    h.get_by_label(" Pad ").click();
    h.run_steps(3);
    press(&mut h, "Delete"); // PLAN: assumed Delete acts at once; a confirmation prompt would need its answer here.
    assert!(h.query_by_label(" Pad ").is_none(), "Delete removed the listed macro");

    // Record starts a recording under the typed name.
    open_named(&mut h, "macros", json!({}));
    set_field(&mut h, "macros", "name", json!("Fresh"));
    press(&mut h, "Record…");
    assert_eq!(h.state().session.recording.as_ref().map(|(n, _)| n.as_str()), Some("Fresh"));
    assert!(!is_open(h.state(), "macros"));
}

// ---------------------------------------------------------------- command search

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
    let query = |h: &egui_kittest::Harness<'_, WordApp>| fields_of(h.state(), "commands").get("query").and_then(Value::as_str).unwrap_or("?").to_string();
    let fresh = || {
        let mut h = app_harness(app_with("Text"));
        h.run_steps(4);
        h
    };
    // The shortcut and its text in one frame.
    let mut h = fresh();
    h.event(key(egui::Key::Q, true));
    h.event(egui::Event::Text("q".into()));
    h.run_steps(3);
    assert!(is_open(h.state(), "commands"), "Alt+Q opens the palette");
    assert_eq!(query(&h), "", "Alt+Q's q, same frame");
    // The text a frame late; then typing.
    let mut h = fresh();
    h.event(key(egui::Key::Q, true));
    h.step();
    h.event(egui::Event::Text("q".into()));
    h.step();
    assert_eq!(query(&h), "", "the q a frame late");
    h.event(key(egui::Key::B, false));
    h.event(egui::Event::Text("b".into()));
    h.run_steps(2);
    assert_eq!(query(&h), "b", "then typing lands");
    // A palette opened by a click takes typing.
    let mut h = fresh();
    open_named(&mut h, "commands", json!({}));
    h.run_steps(3);
    h.event(key(egui::Key::B, false));
    h.event(egui::Event::Text("b".into()));
    h.run_steps(2);
    assert_eq!(query(&h), "b", "a click-opened box takes typing");
}

/// #3398: the interactive command-search result opens the paper-colour selector. The engine
/// command itself remains parameterised for scripts, agents and other programmatic callers.
#[test]
fn screen_paper_color_search_result_opens_general_options() {
    use wordcraft_engine::cmd::view::PaperColor;

    let mut h = app_harness(app_with("Unchanged document"));
    h.run_steps(3);
    open_named(&mut h, "commands", json!({}));
    set_field(&mut h, "commands", "query", json!("screen paper color"));
    h.run_steps(2);
    h.query(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button).label_contains("Screen Paper Color"))
        .expect("the paper-colour command is a search result")
        .click();
    h.run_steps(3);

    assert!(h.state().ui.backstage);
    assert_eq!(h.state().ui.backstage_page, "options");
    assert_eq!(h.state().ui.options_pane, "general");
    assert_eq!(h.state().session.prefs.paper_color, PaperColor::Document, "opening the selector does not choose a preset");
}

/// #2986: a box that keeps the keyboard asks for focus only while it lacks it. `request_focus`
/// always interrupts IME; on X11 that recreates the XIM context, and Search asking every frame
/// left fcitx taking no keys at all. Once the box has focus, no frame interrupts, and typing lands;
/// after Tab takes the keyboard away, the box takes it back with one reset, not one per frame.
#[test]
fn boxes_that_keep_focus_do_not_interrupt_ime_every_frame() {
    // The dialog key and JSON field of the box each dialog types into; Go To is the Find dialog's `goToTarget`.
    let typed = |app: &WordApp, name: &str| -> String {
        let (key, field) = match name {
            "commands" | "icons" => (name, "query"),
            "goto" => ("find", "goToTarget"),
            _ => (name, "newLabel"),
        };
        fields_of(app, key).get(field).and_then(Value::as_str).unwrap_or_default().to_string()
    };
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    for name in ["commands", "icons", "goto", "newLabel"] {
        let mut h = app_harness(app_with("Unchanged document"));
        h.run_steps(2);
        let opened = super::open(h.state_mut(), name, &json!({}));
        assert!(opened.is_some(), "{name} opens");
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
        assert_eq!(typed(h.state(), name), "ab", "{name}: the keys reach the box");

        // Tab moves the keyboard to the dialog's next control and the box takes it back. As
        // egui-winit does, count a reset when IME comes back on or is interrupted while on.
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
        assert_eq!(typed(h.state(), name), "abc", "{name}: the box has the keyboard back after Tab");
    }
}

// ---------------------------------------------------------------- Paragraph

#[test]
fn paragraph_dialog_leaves_what_the_selected_paragraphs_differ_in() {
    // #1346: OK gave every selected paragraph the caret paragraph's formatting, as three
    // undo steps.
    let mut h = app_harness(mixed_selection());
    h.run_steps(2);
    let was: Vec<_> = (0..3).map(|i| para_props(h.state(), i)).collect();
    let shown = shown_paragraph(&mut h);
    assert!(shown.align.is_none() && shown.before_text.is_none() && shown.space_after.is_none() && shown.line_spacing.is_none());
    assert_eq!(shown.breaks.as_ref().and_then(|b| b.keep_with_next), None, "the fields the paragraphs differ in show blank");
    let depth = h.state().session.undo_depth();
    apply_paragraph(&mut h, |_| {});
    assert_eq!((0..3).map(|i| para_props(h.state(), i)).collect::<Vec<_>>(), was, "an unchanged OK changes nothing");
    assert_eq!(h.state().session.undo_depth(), depth, "nor adds an undo step");
    // One field changed: just that one, in every paragraph, as one undo step.
    apply_paragraph(&mut h, |f| f.space_before = Some(6.0));
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
    for (i, was) in was.iter().enumerate() {
        assert_eq!(para_props(h.state(), i), wordcraft_doc::props::ParaProps { space_before: Some(6.0), ..was.clone() }, "paragraph {i}");
    }
    let _ = h.state_mut().run("edit.undo", json!({}));
    assert_eq!((0..3).map(|i| para_props(h.state(), i)).collect::<Vec<_>>(), was, "undo takes all of it back");
}

/// #3028: the records dialog refuses a range with no records itself; a good range merges into
/// a new document beside the unsaved one, asking nothing.
#[test]
fn merge_records_dialog_checks_the_range_before_merging() {
    let mut app = app_with("Dear ");
    let _ = app.run("mailings.recipients", json!({"csv": "Name\nAda\nAlan\nGrace"}));
    let _ = app.run("text.insert", json!({"text": "unsaved"}));
    assert!(app.session.dirty);
    let source = app.session.document_id();
    let mut h = app_harness(app);
    h.run_steps(3);
    let id = open_named(&mut h, "mergeRecords", json!({}));
    set_field(&mut h, "mergeRecords", "choice", json!(2));
    set_field(&mut h, "mergeRecords", "from", json!("3"));
    set_field(&mut h, "mergeRecords", "to", json!("2"));
    press(&mut h, "OK");
    assert!(is_open(h.state(), "mergeRecords"), "the dialog stays open");
    let held = h.state().dialog_host.held_state(id);
    assert!(matches!(held, Some(HeldOk::Refused(_))), "the range with no records is refused");
    set_field(&mut h, "mergeRecords", "to", json!("3"));
    press(&mut h, "OK");
    assert!(!is_open(h.state(), "mergeRecords"), "nothing to ask about: the merge opens beside");
    assert_ne!(h.state().session.document_id(), source, "the merge is a new document");
    h.state_mut().run("view.switchWindows", json!({"id": source})).unwrap();
    assert!(body_of(h.state()).contains("unsaved"), "the unsaved document is still open");
}

#[test]
fn paragraph_dialog_sets_only_the_options_that_changed() {
    // #512: mirror indents, suppress line numbers, don't hyphenate.
    let mut h = app_harness(app_with("Hello"));
    h.run_steps(2);
    apply_paragraph(&mut h, |_| {});
    let p = props(h.state());
    assert_eq!((p.mirror_indents, p.suppress_line_numbers, p.suppress_hyphens), (None, None, None));
    apply_paragraph(&mut h, |f| {
        f.mirror_indents = Some(Some(true));
        if let Some(b) = f.breaks.as_mut() {
            b.suppress_line_numbers = Some(true);
            b.dont_hyphenate = Some(true);
        }
    });
    let p = props(h.state());
    assert_eq!((p.mirror_indents, p.suppress_line_numbers, p.suppress_hyphens), (Some(true), Some(true), Some(true)));
    let shown = shown_paragraph(&mut h);
    assert_eq!(shown.mirror_indents, Some(Some(true)), "the dialog shows what is set");
    apply_paragraph(&mut h, |f| {
        if let Some(b) = f.breaks.as_mut() {
            b.dont_hyphenate = Some(false);
        }
    });
    assert_eq!(props(h.state()).suppress_hyphens, Some(false));
    assert_eq!(props(h.state()).mirror_indents, Some(true));
}

#[test]
fn paragraph_dialog_shows_direction_and_visual_alignment() {
    use craft::text::ui::character::{ParagraphAlign, ParagraphDirection};
    let mut app = app_with("سلام دنیا");
    let _ = app.run("para.rtl", json!({}));
    let _ = app.run("para.set", json!({"props": {"indentLeft": 36.0}}));
    let mut h = app_harness(app);
    h.run_steps(2);
    let shown = shown_paragraph(&mut h);
    assert_eq!(shown.direction, Some(Some(ParagraphDirection::RightToLeft)));
    assert_eq!(shown.align, Some(ParagraphAlign::Right), "a start-aligned right-to-left paragraph shows as right aligned");
    assert_eq!((shown.before_text, shown.after_text), (Some(0.0), Some(Some(36.0))), "the start indent is the right one");
}

#[test]
fn paragraph_dialog_ok_keeps_what_it_shows() {
    for (rtl, align) in [(false, "para.alignCenter"), (false, "para.alignRight"), (true, "para.alignLeft"), (true, "para.alignRight")] {
        let mut app = app_with("متن");
        let _ = app.run(if rtl { "para.rtl" } else { "para.ltr" }, json!({}));
        let _ = app.run(align, json!({}));
        let _ = app.run("para.set", json!({"props": {"indentLeft": 18.0, "indentRight": 9.0}}));
        let before = props(&app);
        let mut h = app_harness(app);
        h.run_steps(2);
        apply_paragraph(&mut h, |_| {});
        let after = props(h.state());
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
        let mut h = app_harness(app);
        h.run_steps(2);
        apply_paragraph(&mut h, |_| {});
        assert_eq!(props(h.state()).line_spacing, Some(rule), "unchanged OK, {cmd}");
        apply_paragraph(&mut h, |f| {
            if let Some(b) = f.breaks.as_mut() {
                b.keep_with_next = Some(true);
            }
        });
        let p = props(h.state());
        assert_eq!((p.line_spacing, p.keep_next), (Some(rule), Some(true)), "Keep with next, {cmd}");
    }
    // A changed value still sets a multiple.
    let mut app = app_with("Example");
    let _ = app.run("para.lineSpacing", json!({"exactly": 24}));
    let mut h = app_harness(app);
    h.run_steps(2);
    apply_paragraph(&mut h, |f| f.line_spacing = Some(craft::text::ui::character::LineSpacing::Multiple(1.5)));
    assert_eq!(props(h.state()).line_spacing, Some(LineSpacing::Multiple(1.5)));
}

#[test]
fn paragraph_dialog_changes_direction() {
    use craft::text::ui::character::{ParagraphAlign, ParagraphDirection};
    let mut h = app_harness(app_with("Hello"));
    h.run_steps(2);
    // Right to left, aligned right on the page, 1" from the right edge.
    apply_paragraph(&mut h, |f| {
        f.direction = Some(Some(ParagraphDirection::RightToLeft));
        f.align = Some(ParagraphAlign::Right);
        f.after_text = Some(Some(72.0));
    });
    let p = props(h.state());
    assert_eq!(p.bidi, Some(true));
    assert_eq!(p.align, Some(Align::Left), "right on the page is the start edge");
    assert_eq!(p.indent_left, Some(72.0), "the right indent is the start indent");
    // A new direction alone keeps the alignment and indents where the dialog shows them.
    apply_paragraph(&mut h, |f| f.direction = Some(Some(ParagraphDirection::LeftToRight)));
    let p = props(h.state());
    assert_eq!((p.bidi, p.align, p.indent_left, p.indent_right), (Some(false), Some(Align::Right), Some(0.0), Some(72.0)));
}

// ---------------------------------------------------------------- Font

#[test]
fn font_dialog_sets_the_asian_and_latin_fonts_separately() {
    // #505: the Asian text font box and the Latin text font box each set their own font, and
    // OK leaves the one not changed as it was.
    let mut app = app_with("Hello 你好");
    let _ = app.run("select.all", json!({}));
    let fonts = |app: &WordApp| {
        let c = char_props_at_start(app);
        (c.font, c.font_ea)
    };
    let latin = fonts(&app).0;
    let mut h = app_harness(app);
    h.run_steps(2);
    apply_font(&mut h, |f| {
        assert!(f.asian, "Chinese text in the selection shows the Asian text font box");
        f.font_ea = "SimSun".into();
    });
    assert_eq!(fonts(h.state()), (latin, Some("SimSun".into())));
    apply_font(&mut h, |f| {
        assert_eq!(f.font_ea, "SimSun");
        f.font = "Georgia".into();
    });
    assert_eq!(fonts(h.state()), (Some("Georgia".into()), Some("SimSun".into())));
    // One undo step each.
    let _ = h.state_mut().run("edit.undo", json!({}));
    assert_eq!(fonts(h.state()).1.as_deref(), Some("SimSun"));
    assert_ne!(fonts(h.state()).0.as_deref(), Some("Georgia"));
}

#[test]
fn font_dialog_ok_keeps_what_the_user_left_alone() {
    // Issue #466: OK sent every field, clearing small caps, spacing and double underline
    // when only the size changed.
    let mut app = app_with("Example");
    let _ = app.run("select.all", json!({}));
    let _ = app.run("format.set", json!({"props": {"smallCaps": true, "caps": true, "hidden": true, "spacing": 2, "underline": "double"}}));
    let mut h = app_harness(app);
    h.run_steps(2);
    apply_font(&mut h, |f| {
        assert!(f.small_caps, "the dialog shows small caps");
        assert_eq!(f.spacing, Some(2.0));
        f.size = Some(14.0);
    });
    let c = char_props_at_start(h.state());
    assert_eq!(c.size, Some(14.0));
    assert_eq!((c.small_caps, c.caps, c.hidden, c.spacing), (Some(true), Some(true), Some(true), Some(2.0)));
    assert_eq!(c.underline, Some(wordcraft_doc::props::Underline::Double));
    // A field the user does change still applies.
    apply_font(&mut h, |f| f.small_caps = false);
    let c = char_props_at_start(h.state());
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
    let mut h = app_harness(app);
    h.run_steps(2);
    apply_font(&mut h, |f| {
        assert_eq!((f.kern, f.kern_at), (Some(false), Some(12.0)), "off, offering 12 pt");
        f.kern = Some(true);
        f.kern_at = Some(16.0);
    });
    assert_eq!((kern(h.state(), 0), kern(h.state(), 7)), (Some(16.0), Some(16.0)));
    let _ = h.state_mut().run("edit.undo", json!({}));
    assert_eq!(kern(h.state(), 0), Some(0.0), "one undo step");
    let _ = h.state_mut().run("edit.redo", json!({}));
    apply_font(&mut h, |f| {
        assert_eq!((f.kern, f.kern_at), (Some(true), Some(16.0)));
        f.kern = Some(false);
    });
    assert_eq!((kern(h.state(), 0), kern(h.state(), 7)), (Some(0.0), Some(0.0)));
    let _ = h.state_mut().run("select.text", json!({"text": "Kerned"}));
    let _ = h.state_mut().run("format.set", json!({"props": {"kern": 14}}));
    let _ = h.state_mut().run("select.all", json!({}));
    apply_font(&mut h, |f| {
        assert_eq!(f.kern, None, "mixed kerning shows neither on nor off");
        f.size = Some(13.0);
    });
    assert_eq!((kern(h.state(), 0), kern(h.state(), 7)), (Some(14.0), Some(0.0)), "each run keeps its kerning");
}

/// #2786: Define New Bullet's Font… is the Font dialog, opened on the bullet's formatting. OK
/// hands the font, size and bold back to Define New Bullet without formatting the text;
/// Cancel, and the title bar's close box, go back with the form as it was.
#[test]
fn font_dialog_formats_a_bullet_and_hands_it_back() {
    use crate::dialogs_bullet::DefineBulletForm;
    type Bullet = FormHost<DefineBulletForm>;
    let back = |button: &str, edit: bool| -> (DefineBulletForm, wordcraft_doc::props::CharProps) {
        let mut app = app_with("Example");
        let _ = app.run("para.bullets", json!({"kind": "▪"}));
        let mut h = app_harness(app);
        h.run_steps(3);
        let parent = open_named(&mut h, "defineBullet", json!({}));
        if let Some(b) = h.state_mut().dialog_host.get_mut_by_id::<Bullet>(parent) {
            b.form_mut().font = "Arial".into();
        }
        h.run_steps(2);
        press(&mut h, "Font…");
        assert!(is_open(h.state(), "font"), "Font… opens the Font dialog over Define New Bullet");
        if edit {
            let id = id_of(h.state(), "font");
            if let Some(f) = h.state_mut().dialog_host.get_mut_by_id::<FontHost>(id) {
                f.form_mut().size = Some(16.0);
                f.form_mut().bold = true;
            }
            h.run_steps(2);
        }
        press(&mut h, button);
        assert!(!is_open(h.state(), "font"), "{button} closes the Font dialog");
        let form = h.state().dialog_host.get::<Bullet>().map(|b| b.form().clone()).expect("back to Define New Bullet");
        (form, char_props_at_start(h.state()))
    };
    let (f, text) = back("OK", true);
    assert_eq!((f.ch.as_str(), f.font.as_str(), f.size.as_str(), f.bold), ("▪", "Arial", "16", true));
    assert_ne!(text.bold, Some(true), "the text isn't formatted");
    assert_ne!(text.size, Some(16.0), "the text isn't formatted");
    for button in ["Cancel", "Close"] {
        let (f, _) = back(button, true);
        assert_eq!((f.font.as_str(), f.size.as_str(), f.bold), ("Arial", "", false), "{button} keeps the form");
    }
}

// ---------------------------------------------------------------- Find and Replace

/// #4177: Replace All in the Find and Replace dialog, with Regular expressions on, expands
/// capture groups (`$2 $1`, `\2 \1`, `${name}`) instead of inserting them literally.
#[test]
fn replace_all_with_regex_groups_from_the_dialog() {
    for (with, want) in [("$2 $1", "John Smith and Jane Doe"), (r"\2 \1", "John Smith and Jane Doe"), ("${last}!", "Smith! and Doe!")] {
        let mut h = app_harness(app_with("Smith, John and Doe, Jane"));
        h.run_steps(2);
        h.state_mut().run("ui.dialog", json!({"name": "replace"})).unwrap();
        h.run_steps(2);
        set_field(&mut h, "find", "query", json!(r"(?<last>\w+), (\w+)")); // PLAN: assumed key `find` for the Find and Replace dialog
        set_field(&mut h, "find", "with", json!(with));
        set_field(&mut h, "find", "regex", json!(true));
        h.get_by_label("Replace All").click();
        h.run_steps(2);
        assert_eq!(body_of(h.state()), want, "{with}");
    }
}

// ---------------------------------------------------------------- Page Setup

/// Opens Page Setup in `h`, edits its form (points) and answers OK.
fn page_setup(h: &mut egui_kittest::Harness<'_, WordApp>, edit: impl FnOnce(&mut PageSetupForm)) {
    let id = open_named(h, "pageSetup", json!({}));
    if let Some(host) = h.state_mut().dialog_host.get_mut_by_id::<PageSetupHost>(id) {
        edit(host.form_mut());
    }
    h.run_steps(2);
    confirm(h, id);
}


/// The Page Setup paper size as opened, in points.
fn opened_paper(h: &mut egui_kittest::Harness<'_, WordApp>) -> (f32, f32) {
    let id = open_named(h, "pageSetup", json!({}));
    let paper = h.state().dialog_host.get_by_id::<PageSetupHost>(id).map(|d| d.form().paper).unwrap_or_default();
    let ctx = h.ctx.clone();
    h.state_mut().dialog_host.cancel_by_id(&ctx, id);
    h.run_steps(2);
    paper
}

/// #1433: Page Setup's Paper size shows the named size the section has, in either
/// orientation, and Custom size (None) for anything else.
#[test]
fn page_setup_reads_the_paper_size() {
    let mut h = app_harness(app_with("Page"));
    h.run_steps(2);
    let sect = |h: &egui_kittest::Harness<'_, WordApp>| wordcraft_engine::cmd::page::sect(&h.state().session);
    let (w, ht) = opened_paper(&mut h);
    assert!((w - sect(&h).page_w).abs() < 0.01 && (ht - sect(&h).page_h).abs() < 0.01, "{w}×{ht}");
    assert!(wordcraft_geom::paper_name(w, ht).is_some(), "a new document has a named size");
    let _ = h.state_mut().run("layout.size", json!({"name": "A4"}));
    let _ = h.state_mut().run("layout.orientation", json!({"value": "landscape"}));
    let (w, ht) = opened_paper(&mut h);
    assert!(w > ht, "landscape shows the wide side as the width: {w}×{ht}");
    assert_eq!(wordcraft_geom::paper_name(w, ht), Some("A4"));
    let _ = h.state_mut().run("layout.size", json!({"width": 300.0, "height": 500.0}));
    let (w, ht) = opened_paper(&mut h);
    assert_eq!(wordcraft_geom::paper_name(w, ht), None, "Custom size");
}

/// #1433: a custom width and height applied with OK set the section's size (before the
/// orientation, so a wide custom size ends up landscape); an unchanged size doesn't run
/// `layout.size`, and switching orientation alone only turns the page.
#[test]
fn page_setup_applies_a_custom_paper_size() {
    let size_steps = |app: &WordApp| app.session.undo_labels().iter().filter(|l| l.as_str() == "Size").count();
    let sect = |app: &WordApp| {
        let s = wordcraft_engine::cmd::page::sect(&app.session);
        (s.page_w.round(), s.page_h.round(), s.landscape)
    };
    // OK on an unchanged dialog leaves the size alone.
    let mut h = app_harness(app_with("Page"));
    h.run_steps(2);
    let before = sect(h.state());
    page_setup(&mut h, |_| {});
    assert!(!is_open(h.state(), "pageSetup"));
    assert_eq!((sect(h.state()), size_steps(h.state())), (before, 0), "no layout.size for an unchanged size");

    for ((w, ht), want) in [((400.0, 600.0), (400.0, 600.0, false)), ((800.0, 400.0), (800.0, 400.0, true))] {
        page_setup(&mut h, |f| {
            f.paper = (w, ht);
            f.landscape = w > ht;
        });
        assert_eq!(sect(h.state()), want, "{w}×{ht}");
    }
    assert_eq!(size_steps(h.state()), 2);

    // Portrait again with the same paper: the dialog swaps width and height, which is no new size.
    page_setup(&mut h, |f| {
        f.paper = (f.paper.1, f.paper.0);
        f.landscape = false;
    });
    assert_eq!(sect(h.state()), (400.0, 800.0, false));
    assert_eq!(size_steps(h.state()), 2, "turning the page isn't a new size");
}

/// #1433: Page Setup checks the new size and margins together: a page its margins don't fit
/// keeps the dialog open with a message and applies nothing; a smaller page with smaller
/// margins applies (margins first, so no step is refused on the way).
#[test]
fn page_setup_refuses_margins_that_dont_fit_and_applies_smaller_ones_first() {
    let mut h = app_harness(app_with("Page"));
    h.run_steps(2);
    let before = wordcraft_engine::cmd::page::sect(&h.state().session);
    let undo = h.state().session.undo_depth();
    let id = open_named(&mut h, "pageSetup", json!({}));
    if let Some(d) = h.state_mut().dialog_host.get_mut_by_id::<PageSetupHost>(id) {
        d.form_mut().paper = (100.0, 500.0);
    }
    h.run_steps(2);
    confirm(&mut h, id);
    assert!(is_open(h.state(), "pageSetup"), "stays open");
    let held = h.state().dialog_host.held_state(id);
    assert!(matches!(held, Some(HeldOk::Refused(_))), "with a message");
    assert_eq!((wordcraft_engine::cmd::page::sect(&h.state().session), h.state().session.undo_depth()), (before, undo), "nothing applied");

    if let Some(d) = h.state_mut().dialog_host.get_mut_by_id::<PageSetupHost>(id) {
        let f = d.form_mut();
        (f.top, f.bottom, f.left, f.right) = (18.0, 18.0, 18.0, 18.0);
    }
    h.run_steps(2);
    confirm(&mut h, id);
    assert!(!is_open(h.state(), "pageSetup"));
    let s = wordcraft_engine::cmd::page::sect(&h.state().session);
    assert_eq!((s.page_w.round(), s.page_h.round(), s.margin_left.round(), s.margin_top.round()), (100.0, 500.0, 18.0, 18.0));
}

/// #1433: one OK in Page Setup is one undo step, whichever order its commands run in (the
/// size first; the margins first; margins of 0 in between), and Undo restores the section.
#[test]
fn page_setup_ok_is_one_undo_step() {
    let sect = |h: &egui_kittest::Harness<'_, WordApp>| wordcraft_engine::cmd::page::sect(&h.state().session);
    let apply = |h: &mut egui_kittest::Harness<'_, WordApp>, (w, ht): (f32, f32), (t, b, l, r): (f32, f32, f32, f32)| {
        page_setup(h, |f| {
            f.paper = (w, ht);
            f.landscape = w > ht;
            (f.top, f.bottom, f.left, f.right) = (t, b, l, r);
        });
    };
    // Size first, then orientation and margins.
    let mut h = app_harness(app_with("Page"));
    h.run_steps(2);
    let (before, depth) = (sect(&h), h.state().session.undo_depth());
    apply(&mut h, (400.0, 600.0), (36.0, 36.0, 36.0, 36.0));
    assert!(!is_open(h.state(), "pageSetup"));
    assert_eq!((sect(&h).page_w, sect(&h).margin_left), (400.0, 36.0));
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(sect(&h), before, "one undo restores the section");

    // Neither size first nor margins first fits on the way: margins of 0 in between.
    h.state_mut().run("layout.margins", json!({"top": 72.0, "bottom": 72.0, "left": 250.0, "right": 250.0})).unwrap();
    let (before, depth) = (sect(&h), h.state().session.undo_depth());
    apply(&mut h, (300.0, 900.0), (400.0, 400.0, 36.0, 36.0));
    assert!(!is_open(h.state(), "pageSetup"));
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
    h.run_steps(2);
    let id = open_named(&mut h, "pageSetup", json!({}));
    if let Some(d) = h.state_mut().dialog_host.get_mut_by_id::<PageSetupHost>(id) {
        let f = d.form_mut();
        f.paper = (300.0, 500.0);
        (f.top, f.bottom, f.left, f.right) = (72.0, 72.0, 72.0, 72.0);
    }
    h.run_steps(2);
    confirm(&mut h, id);
    assert!(is_open(h.state(), "pageSetup"), "stays open with the error");
    let held = h.state().dialog_host.held_state(id);
    assert!(matches!(held, Some(HeldOk::Refused(_))), "the refused command is shown");
    let after = h.state().session.doc.sections().iter().map(|(_, x)| (*x).clone()).collect::<Vec<_>>();
    assert_eq!((after, h.state().session.undo_depth()), (doc, depth), "nothing applied");
}

/// #1230: Page Setup's Different first page check box gives the title page no page number
/// (its footer is the empty first-page one), in one undo step, with Odd & Even left as it was.
#[test]
fn page_setup_different_first_page_leaves_the_title_page_unnumbered() {
    let mut app = app_with("Title");
    app.run("insert.pageBreak", json!({})).unwrap();
    app.run("text.insert", json!({"text": "Body"})).unwrap();
    app.run("insert.pageNumber", json!({"position": "bottom", "align": "center"})).unwrap();
    let _ = app.run("insert.closeHeader", json!({}));
    assert!(app.session.layout().pages[0].footer_story.is_some(), "page 1 is numbered");
    let depth = app.session.undo_depth();
    let mut h = app_harness(app);
    h.run_steps(2);
    let id = open_named(&mut h, "pageSetup", json!({}));
    let shown = h.state().dialog_host.get_by_id::<PageSetupHost>(id).map(|d| (d.form().first_page, d.form().odd_even));
    assert_eq!(shown, Some((false, false)), "shows the section's values");
    if let Some(d) = h.state_mut().dialog_host.get_mut_by_id::<PageSetupHost>(id) {
        d.form_mut().first_page = true;
    }
    h.run_steps(2);
    confirm(&mut h, id);
    let s = &mut h.state_mut().session;
    let sect = wordcraft_engine::cmd::page::sect(s);
    assert!(sect.title_page);
    assert!(!s.doc.settings.even_odd_headers, "left as it was");
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    let pages = &s.layout().pages;
    assert!(pages[0].footer_story.is_none(), "the title page shows no page number");
    assert!(pages[1].footer_story.is_some(), "the next page keeps its number");
}

/// #1320: each changed Layout value (vertical alignment, Different first page, Odd & Even)
/// sends only its own command, in one undo step, and leaves the other two as they were.
#[test]
fn page_setup_each_layout_value_sends_only_its_own_command() {
    use wordcraft_doc::section::PageVAlign;
    for case in 0..3 {
        let mut h = app_harness(app_with("Body"));
        h.run_steps(2);
        let sect = |h: &egui_kittest::Harness<'_, WordApp>| wordcraft_engine::cmd::page::sect(&h.state().session);
        let before = sect(&h);
        let depth = h.state().session.undo_depth();
        page_setup(&mut h, |f| match case {
            0 => f.valign = PageVAlign::Justify,
            1 => f.first_page = true,
            _ => f.odd_even = true,
        });
        let after = sect(&h);
        let even_odd = h.state().session.doc.settings.even_odd_headers;
        assert_eq!(h.state().session.undo_depth(), depth + 1, "case {case}: one undo step");
        let valign_changed = after.valign != before.valign;
        assert_eq!(
            (valign_changed, after.title_page, even_odd),
            match case {
                0 => (true, false, false),
                1 => (false, true, false),
                _ => (false, false, true),
            },
            "case {case}: only its own value changed"
        );
        if case == 0 {
            assert_eq!(after.valign, PageVAlign::Justify);
        }
    }
}

// ---------------------------------------------------------------- SmartArt text pane

/// #2701: in the SmartArt Text Pane, Tab demotes and Shift+Tab promotes the focused item, as
/// Word's pane does; the row keeps its focus, so the next Tab acts on the same item. An item
/// that can't go down a level (the one before it isn't deeper) is left as it is.
#[test]
fn text_pane_tab_demotes_and_shift_tab_promotes_the_focused_item() {
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

// ---------------------------------------------------------------- Custom Properties

/// Type into a dialog text field, replacing what is there (as a user would: click, select
/// all, type over).
fn set_text_field(h: &mut egui_kittest::Harness<'_, WordApp>, label: &str, text: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).type_text(text);
    h.run_steps(2);
}

/// Whether the dialog shows a notice. PLAN: assumed a notice's node role is Alert or Status.
fn notice_shown(h: &egui_kittest::Harness<'_, WordApp>) -> bool {
    h.query_all(egui_kittest::kittest::By::new().predicate(|n| matches!(n.role(), egui::accesskit::Role::Alert | egui::accesskit::Role::Status)))
        .next()
        .is_some()
}

/// A harness with File › Info › Advanced Properties… opened on `app`.
fn custom_properties(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut app = app;
    let _ = app.run("ui.backstage", json!({"value": true, "page": "info"}));
    let mut h = app_harness(app);
    h.run_steps(4);
    h.get_by_label("Advanced Properties…").click();
    h.run_steps(4);
    assert!(is_open(h.state(), "customProperties"), "opens from File › Info");
    h
}

/// #2982: File › Info's Advanced Properties… opens the Custom Properties dialog; it lists
/// the document's custom properties, and Add / Modify / Delete edit a working copy that OK
/// applies as one undoable `file.properties` step, Cancel discarding it.
#[test]
fn the_custom_properties_dialog_adds_edits_and_deletes_in_one_step() {
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft", "Owner": "Mira"}}));
    let mut h = custom_properties(app);
    assert!(h.query_by_label("Status = draft").is_some(), "the list shows the properties");
    // A row is selected, so the buttons act on it.
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").accesskit_node().is_disabled());
    assert!(!h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").accesskit_node().is_disabled());
    // Add one through the form.
    set_text_field(&mut h, "Name:", "Reviewer");
    set_text_field(&mut h, "Value:", "Ada");
    press(&mut h, "Add");
    assert!(h.query_by_label("Reviewer = Ada").is_some(), "Add appends the typed property");
    // A duplicate name in another case replaces its value.
    set_text_field(&mut h, "Name:", "status");
    set_text_field(&mut h, "Value:", "final");
    press(&mut h, "Add");
    assert!(h.query_by_label("Status = final").is_some(), "the duplicate replaced the value");
    // Selecting a row fills the boxes; Modify changes its value.
    h.get_by_label("Reviewer = Ada").click();
    h.run_steps(2);
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Name:").value().as_deref(), Some("Reviewer"));
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Value:").value().as_deref(), Some("Ada"));
    set_text_field(&mut h, "Value:", "Ada L.");
    press(&mut h, "Modify");
    assert!(h.query_by_label("Reviewer = Ada L.").is_some(), "Modify keeps the edited value");
    // Delete another property, so the modified value is saved before that one goes.
    h.get_by_label("Owner = Mira").click();
    h.run_steps(2);
    press(&mut h, "Delete");
    assert!(h.query_by_label("Owner = Mira").is_none(), "Delete removes the selected property");
    // OK: one call, so one undo step, marking the document changed.
    let depth = h.state().session.undo_depth();
    h.state_mut().session.dirty = false;
    h.step();
    press(&mut h, "OK");
    h.run_steps(2);
    assert!(!is_open(h.state(), "customProperties"), "OK closes the dialog");
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

/// #2982: a rename that changes only the case of the name is still a rename. Modify and OK
/// must send the old spelling as a removal and the new one as a value, or the stored
/// spelling never changes.
#[test]
fn the_custom_properties_dialog_keeps_a_case_only_rename() {
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft"}}));
    let mut h = custom_properties(app);
    // Select the property and change only the case of its name, keeping the value.
    h.get_by_label("Status = draft").click();
    h.run_steps(2);
    set_text_field(&mut h, "Name:", "status");
    assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Value:").value().as_deref(), Some("draft"));
    press(&mut h, "Modify");
    assert!(h.query_by_label("status = draft").is_some(), "the list shows the new spelling");
    press(&mut h, "OK");
    h.run_steps(2);
    assert!(!is_open(h.state(), "customProperties"), "OK closes the dialog");
    let props: Vec<(String, String)> = h.state().session.doc.custom_props.iter().map(|p| (p.name.clone(), p.value.clone())).collect();
    assert_eq!(props, [("status".to_string(), "draft".to_string())], "the stored spelling follows the rename");
}

/// #4767: a dialog rename keeps the imported bookmark association through the command
/// boundary, Undo/Redo and a DOCX save, including a rename that changes only letter case.
#[test]
fn custom_property_rename_keeps_bookmark_link_through_docx() {
    for (name, linked, replacement) in [("Renamed", true, false), ("status", true, false), ("Taken", true, true), ("Taken", false, true)] {
        let mut app = app_with("Original linked content");
        app.session.run("select.all", &json!({})).unwrap();
        app.session.run("insert.bookmark", &json!({"name": "OriginalBookmark"})).unwrap();
        app.session.run("file.properties", &json!({"custom": {"Status": "draft"}})).unwrap();
        app.session.doc.custom_props[0].link_target = linked.then(|| "OriginalBookmark".into());
        if replacement {
            app.session.run("insert.bookmark", &json!({"name": "OtherBookmark"})).unwrap();
            app.session.doc.set_custom_prop("Taken", "draft");
            app.session.doc.custom_props[1].link_target = Some("OtherBookmark".into());
        }
        let original = app.session.doc.custom_props.clone();
        let mut h = custom_properties(app);
        set_text_field(&mut h, "Name:", name);
        press(&mut h, "Modify");
        press(&mut h, "OK");
        h.run_steps(2);
        let changed = h.state().session.doc.custom_props.clone();
        assert_eq!(changed[0].name, name);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].link_target.as_deref(), linked.then_some("OriginalBookmark"));
        let bytes = wordcraft_engine::io::save_bytes("linked.docx", &h.state().session.doc).unwrap();
        let reopened = wordcraft_engine::io::open_bytes("linked.docx", &bytes).unwrap();
        assert_eq!(reopened.custom_props, changed);
        h.state_mut().session.run("edit.undo", &json!({})).unwrap();
        assert_eq!(h.state().session.doc.custom_props, original);
        h.state_mut().session.run("edit.redo", &json!({})).unwrap();
        assert_eq!(h.state().session.doc.custom_props, changed);
    }
}

/// #2982: Cancel leaves the document as it was; empty names are refused with a message in
/// the dialog; the Name box stops at Word's limit; and the last deletion shows the empty
/// state with Modify and Delete disabled.
#[test]
fn the_custom_properties_dialog_cancel_changes_nothing_and_refuses_bad_names() {
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft"}}));
    let mut h = custom_properties(app);
    let depth = h.state().session.undo_depth();
    // Empty and blank names are refused, with the message shown, keeping the list as it was.
    assert!(!notice_shown(&h), "no message before a refusal");
    set_text_field(&mut h, "Name:", "");
    press(&mut h, "Add");
    assert!(notice_shown(&h), "an empty name shows the message");
    set_text_field(&mut h, "Name:", "   ");
    press(&mut h, "Add");
    assert!(notice_shown(&h), "a whitespace name shows the message");
    assert!(h.query_by_label("Status = draft").is_some(), "nothing was added");
    // The Name box caps typing at Word's limit.
    set_text_field(&mut h, "Name:", &"x".repeat(wordcraft_engine::cmd::file::MAX_CUSTOM_NAME + 20));
    let typed = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Name:").value().unwrap_or_default();
    assert_eq!(typed.chars().count(), wordcraft_engine::cmd::file::MAX_CUSTOM_NAME, "the Name box stops at the limit");
    // A valid entry, so Cancel has something to discard.
    set_text_field(&mut h, "Name:", "Reviewer");
    set_text_field(&mut h, "Value:", "Ada");
    press(&mut h, "Add");
    assert!(h.query_by_label("Reviewer = Ada").is_some());
    // Deleting every row shows the empty state and disables Modify and Delete.
    h.get_by_label("Reviewer = Ada").click();
    h.run_steps(2);
    press(&mut h, "Delete");
    h.get_by_label("Status = draft").click();
    h.run_steps(2);
    press(&mut h, "Delete");

    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Modify").accesskit_node().is_disabled());
    assert!(h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").accesskit_node().is_disabled());
    press(&mut h, "Cancel");
    h.run_steps(2);
    assert!(!is_open(h.state(), "customProperties"), "Cancel closes the dialog");
    let names: Vec<&str> = h.state().session.doc.custom_props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Status"], "the document is untouched");
    assert_eq!(h.state().session.undo_depth(), depth, "no step was recorded");
}

/// #2982: imported non-text properties (a number, Zotero's raw XML) stay as they are: Add
/// and Modify refuse to turn one into text, while Delete still removes it.
#[test]
fn the_custom_properties_dialog_leaves_imported_non_text_properties_alone() {
    let mut app = app_with("Report body");
    let _ = app.run("file.properties", json!({"custom": {"Status": "draft"}}));
    app.session.doc.custom_props.push(wordcraft_doc::CustomProp { name: "Count".into(), kind: "i4".into(), value: "7".into(), ..Default::default() });
    let mut h = custom_properties(app);
    assert!(h.query_by_label("Count = 7").is_some(), "the imported property is listed");
    // Add with its name is refused rather than replacing it with text.
    set_text_field(&mut h, "Name:", "count");
    set_text_field(&mut h, "Value:", "8");
    press(&mut h, "Add");
    assert!(notice_shown(&h), "Add refuses the non-text name");
    assert!(h.query_by_label("Count = 7").is_some(), "it keeps its value");
    // Modify of the non-text property itself is refused.
    h.get_by_label("Count = 7").click();
    h.run_steps(2);
    set_text_field(&mut h, "Value:", "8");
    press(&mut h, "Modify");
    assert!(notice_shown(&h), "Modify refuses the non-text property");
    assert!(h.query_by_label("Count = 7").is_some(), "its value is unchanged");
    // Renaming a text property onto a non-text one is refused too.
    h.get_by_label("Status = draft").click();
    h.run_steps(2);
    set_text_field(&mut h, "Name:", "Count");
    press(&mut h, "Modify");
    assert!(notice_shown(&h), "a rename onto it is refused");
    assert!(h.query_by_label("Status = draft").is_some(), "the text property keeps its name");
    // Delete still removes it outright.
    h.get_by_label("Count = 7").click();
    h.run_steps(2);
    press(&mut h, "Delete");
    assert!(h.query_by_label("Count = 7").is_none(), "Delete removes a non-text property");
    press(&mut h, "Cancel");
    h.run_steps(2);
    let kinds: Vec<(&str, &str)> = h.state().session.doc.custom_props.iter().map(|p| (p.name.as_str(), p.kind.as_str())).collect();
    assert_eq!(kinds, [("Status", "lpwstr"), ("Count", "i4")], "Cancel leaves the imported property as it was");
}

// ---------------------------------------------------------------- modality and names

/// #3859: a dialog is modal, as in Word. While Font is open, typing (from the frame it opens),
/// Mod+O and a click on File reach nothing behind it, and the dialog itself still works. Find
/// and Replace is modeless: with it open, File still opens.
#[test]
fn modal_dialogs_block_the_window_behind_them() {
    // Every pick the app asks for is recorded, none answered.
    let files = std::sync::Arc::new(craft::host::ScriptedFiles::held());
    let host = craft::host::Host::builder(craft::host::AppInfo { id: "wordcraft-test", name: "WordCraft", version: "0" }).files(files.clone()).build();
    let a = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default()).with_host(std::sync::Arc::new(host));
    let mut h = app_harness(a);
    h.run_steps(6);
    h.event(egui::Event::Text("Kilns".into()));
    h.run_steps(2);
    assert_eq!(body_of(h.state()), "Kilns");

    super::open(h.state_mut(), "font", &json!({})).expect("font opens");
    h.event(egui::Event::Text("x".into()));
    h.event(egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
    h.run_steps(2);
    assert_eq!(body_of(h.state()), "Kilns", "typing doesn't reach the document");
    assert!(files.requests().is_empty(), "Mod+O opens nothing");
    h.run_steps(2);
    h.get_by_label("File").click();
    h.run_steps(4);
    assert!(!h.state().ui.backstage, "File doesn't open");
    press(&mut h, "Cancel");
    assert!(!is_open(h.state(), "font"), "Cancel closes the dialog");
    h.event(egui::Event::Text("!".into()));
    h.run_steps(2);
    assert_eq!(body_of(h.state()), "Kilns!", "the document takes typing again");

    open_named(&mut h, "find", json!({}));
    h.get_by_label("File").click();
    h.run_steps(4);
    assert!(h.state().ui.backstage, "File opens beside Find");
}

/// Which dialogs leave the window working (#3859, Word for Mac 16.114.1): `shows_modal` and
/// `modal_open` agree with it for each.
#[test]
fn find_symbol_mark_entry_cross_reference_and_merge_fields_are_modeless() {
    let mut h = app_harness(app_with("Text"));
    h.run_steps(3);
    assert!(!h.state().dialog_host.shows_modal(), "nothing open");
    assert!(!h.state().modal_open());
    for (name, modal) in [
        ("find", false),
        ("replace", false),
        ("goto", false),
        ("symbol", false),
        ("crossReference", false),
        ("markEntry", false),
        ("insertMergeField", false),
        ("font", true),
        ("paragraph", true),
        ("pageSetup", true),
        ("wordCount", true),
        ("bookmark", true),
        ("insertTable", true),
        ("colors", true),
    ] {
        let opened = super::open(h.state_mut(), name, &json!({}));
        assert!(opened.is_some(), "{name} opens");
        h.run_steps(2);
        assert_eq!(h.state().dialog_host.shows_modal(), modal, "{name}");
        assert_eq!(h.state().modal_open(), modal, "{name}: modal_open");
        let ctx = h.ctx.clone();
        close_all(h.state_mut(), &ctx);
        h.run_steps(2);
        assert!(!h.state().dialog_host.shows_modal(), "{name}: closed");
    }
}

/// Every dialog name the name table knows opens its dialog under the key the host reports, or
/// nothing where its condition isn't met; scripts and every command use this table (dg.A2).
#[test]
fn every_dialog_name_opens_its_dialog_or_none() {
    // PLAN: keys are assumed equal to the names, except the three Find tabs (one dialog) and
    // `restrictEditing`, which opens a pane.
    let always = [
        "bookmark",
        "macros",
        "commands",
        "icons",
        "goto",
        "newLabel",
        "mergeRecords",
        "pageSetup",
        "font",
        "paragraph",
        "wordCount",
        "insertTable",
        "symbol",
        "crossReference",
        "markEntry",
        "insertMergeField",
        "defineBullet",
        "customProperties",
        "recipientList",
        "newRecipientList",
        "columns",
        "enclose",
        "phonetic",
        "asianTypography",
        "tabs",
        "borders",
        "pageBorders",
        "shading",
        "defineList",
        "trackChangesOptions",
        "link",
        "pasteSpecial",
        "watermark",
        "renameDocument",
        "zoom",
        "noteOptions",
        "customizeKeyboard",
        "newStyle",
        "manageStyles",
        "templates",
        "about",
        "fontDownload",
        "compare",
    ];
    // PLAN: the keys of the names added above are assumed equal to the names, except the border tabs (one dialog).
    for name in always {
        let mut a = app_with("Some text\nMore text");
        a.run("mailings.recipients", json!({"csv": "Name\nAda\nAlan"})).unwrap();
        let _ = a.run("select.all", json!({}));
        let opened = super::open(&mut a, name, &json!({}));
        assert!(opened.is_some(), "{name} opens");
        let keys = open_keys(&a);
        let expect = match name {
            "goto" => "find",
            "pageBorders" | "shading" => "borders",
            _ => name,
        };
        assert!(keys.iter().any(|k| k == expect), "{name}: open keys are {keys:?}");
    }
    for name in ["find", "replace"] {
        let mut a = app_with("x");
        assert!(super::open(&mut a, name, &json!({})).is_some(), "{name}");
        assert!(is_open(&a, "find"), "{name} opens the one Find and Replace dialog");
    }
    // Needs a selected chart: nothing opens without one.
    for name in ["changeChartType", "chartData", "nonsense"] {
        let mut a = app_with("x");
        assert!(super::open(&mut a, name, &json!({})).is_none(), "{name}: no dialog");
        assert!(open_keys(&a).is_empty(), "{name}: nothing is open");
    }
    // Restrict Editing is a pane, not a dialog.
    let mut a = app_with("x");
    let _ = super::open(&mut a, "restrictEditing", &json!({}));
    assert!(a.restrict.open, "restrictEditing opens its pane");
    assert!(open_keys(&a).is_empty());
}
