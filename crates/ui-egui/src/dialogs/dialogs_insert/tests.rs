use super::*;

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Default::default())
}

fn drop_cap_form(a: &mut WordApp) -> Option<DropCapForm> {
    a.run("ui.dialog", json!({"name": "dropCap"})).unwrap();
    match a.dialog.take() {
        Some(crate::dialogs::Dialog::Insert(d)) => match *d {
            InsertDialog::DropCap(f) => Some(f),
            d => panic!("not drop cap: {d:?}"),
        },
        _ => None,
    }
}

/// #2601: Drop Cap Options… opens on the paragraph's drop cap (the defaults without one:
/// 3 lines, distance 0, the text's font) in the interface unit, and OK runs `insert.dropCap`
/// with the chosen values; it reopens showing them. Choosing the text's own font sends none.
#[test]
fn drop_cap_dialog_prefills_and_applies() {
    let mut a = app();
    a.run("file.options", json!({"units": "centimeters"})).unwrap();
    let mut f = drop_cap_form(&mut a).expect("the dialog");
    assert_eq!((f.position.as_str(), f.lines, f.distance), ("none", 3, 0.0));
    assert_eq!(f.font, f.text_font, "the text's font");
    assert!(!f.font.is_empty());
    assert_eq!(f.params(), json!({"position": "none"}));
    f.position = "margin".into();
    f.lines = 4;
    f.distance = 1.0;
    a.run("insert.dropCap", f.params()).unwrap();
    let props = |a: &WordApp| a.session.doc.body.first().and_then(|b| b.as_para()).map(|p| p.props.clone()).unwrap_or_default();
    let p = props(&a);
    assert_eq!((p.drop_cap, p.drop_cap_margin, p.drop_cap_font.as_deref()), (Some(4), Some(true), None));
    assert!((p.drop_cap_distance.unwrap_or_default() - wordcraft_geom::PT_PER_CM).abs() < 0.01, "1 cm: {:?}", p.drop_cap_distance);
    let mut f = drop_cap_form(&mut a).expect("the dialog");
    assert_eq!((f.position.as_str(), f.lines), ("margin", 4));
    assert!((f.distance - 1.0).abs() < 0.01, "{} cm", f.distance);
    f.position = "dropped".into();
    f.font = "Liberation Mono".into();
    a.run("insert.dropCap", f.params()).unwrap();
    let p = props(&a);
    assert_eq!((p.drop_cap, p.drop_cap_margin, p.drop_cap_font.as_deref()), (Some(4), None, Some("Liberation Mono")));
    assert_eq!(drop_cap_form(&mut a).map(|f| f.font), Some("Liberation Mono".to_string()));
    assert_eq!(wordcraft_engine::cmd::insert::drop_cap_state(&a.session).position, "dropped", "the menu checks Dropped");

    // Where Word greys Drop Cap out, there is no dialog.
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    assert!(drop_cap_form(&mut a).is_none(), "an empty paragraph");
}

/// More Columns: two unequal columns with a line between apply as one undo step.
#[test]
fn columns_dialog_applies_unequal_columns_in_one_step() {
    let mut a = app();
    a.run("ui.dialog", json!({"name": "columns"})).unwrap();
    let Some(crate::dialogs::Dialog::Insert(d)) = a.dialog.take() else { panic!("no dialog") };
    let InsertDialog::Columns(mut f) = *d else { panic!("not columns") };
    assert_eq!((f.count, f.equal), (1, true));
    f.preset("left");
    assert_eq!(f.current_preset(), Some("left"));
    // Widen the first column: the second gives up the room, the total stays the text width.
    if let Some(c) = f.cols.first_mut() {
        c.0 += 0.5;
    }
    f.rebalance(0);
    let total: f32 = f.cols.iter().map(|(w, s)| w + s).sum();
    assert!((total - f.text_width).abs() < 0.01, "{total} vs {}", f.text_width);
    f.separator = true;
    let undo_before = a.session.undo_depth();
    a.run("layout.columns", f.params()).unwrap();
    let c = wordcraft_engine::cmd::page::sect(&a.session).columns;
    assert_eq!((c.count, c.separator, c.widths.len()), (2, true, 2));
    assert_eq!(a.session.undo_depth(), undo_before + 1, "one undo step");
    // The dialog reopens with what was applied.
    let again = ColumnsForm::read(&a);
    assert!(!again.equal && again.separator && again.cols.len() == 2);
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(wordcraft_engine::cmd::page::sect(&a.session).columns.count, 1);
}

/// With text selected, More Columns applies to the selected text by default, as in Word
/// (#2024): the other paragraphs keep one column.
#[test]
fn columns_dialog_applies_to_the_selected_text() {
    let mut a = app();
    assert!(!ColumnsForm::read(&a).selection, "no selection, no Selected text");
    a.run("select.paragraph", json!({})).unwrap();
    let mut f = ColumnsForm::read(&a);
    assert_eq!((f.selection, f.apply.as_str()), (true, "selection"));
    f.preset("two");
    a.run("layout.columns", f.params()).unwrap();
    let counts: Vec<u32> = a.session.doc.sections().iter().map(|(_, x)| x.columns.count).collect();
    assert_eq!(counts, vec![2, 1]);
}

/// The Symbol dialog inserts in the chosen font and remembers the symbol.
#[test]
fn symbol_dialog_inserts_with_its_font() {
    let mut a = app();
    let mut f = SymbolForm::new(&a, 0);
    f.font = "DejaVu Sans".into();
    f.selected = Some('\u{2665}');
    let p = f.params().unwrap();
    assert_eq!(p, json!({"char": "\u{2665}", "font": "DejaVu Sans"}));
    a.run("insert.symbol", p).unwrap();
    remember_symbol(&mut a, "\u{2665}", "DejaVu Sans");
    assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).starts_with('\u{2665}'));
    assert_eq!(a.ui.recent_symbols.first().map(|r| r.font.as_str()), Some("DejaVu Sans"));
    assert_eq!(subset_of('\u{2665}'), Some("Miscellaneous Symbols"));
    // (normal text) inserts without a font.
    f.font.clear();
    assert_eq!(f.params().unwrap(), json!({"char": "\u{2665}"}));
}

/// The Field dialog builds the field code from its options.
#[test]
fn field_dialog_builds_the_instruction() {
    let a = app();
    let mut f = FieldForm::new(&a);
    assert_eq!(f.instruction(), r#"DATE \@ "M/d/yyyy" \* MERGEFORMAT"#);
    f.date_format = "yyyy-MM-dd".into();
    f.preserve = false;
    assert_eq!(f.instruction(), r#"DATE \@ "yyyy-MM-dd""#);
    f.select("PAGE");
    f.number_format = "roman".into();
    assert_eq!(f.instruction(), r"PAGE \* roman");
    f.select("SEQ");
    f.target = "Table".into();
    assert_eq!(f.instruction(), r"SEQ Table");
    f.select("REF");
    f.target = "intro".into();
    assert_eq!(f.instruction(), r"REF intro \h");
    f.select("STYLEREF");
    assert_eq!(f.target, "Heading 1");
    f.style_switches = "pl".into();
    assert_eq!(f.instruction(), r#"STYLEREF "Heading 1" \l \p"#);
    f.select("MERGEFIELD");
    f.target = "First Name".into();
    assert_eq!(f.params().unwrap(), json!({"instr": r#"MERGEFIELD "First Name""#, "result": "«First Name»"}));
    // Field Codes: the typed code wins.
    f.codes = true;
    f.code = "NUMPAGES \\* Arabic".into();
    assert_eq!(f.final_code(), "NUMPAGES \\* Arabic");
    f.code = "  ".into();
    assert!(f.params().is_none());
}

/// The Field dialog lists Word's document information fields, and the counts take a number
/// format.
#[test]
fn field_dialog_lists_the_document_information_fields() {
    let listed: Vec<&str> = FIELDS.iter().filter(|(_, cat, _)| *cat == "Document Information").map(|(n, _, _)| *n).collect();
    for name in ["COMMENTS", "KEYWORDS", "LASTSAVEDBY", "NUMCHARS", "REVNUM", "SUBJECT"] {
        assert!(listed.contains(&name), "{name} in {listed:?}");
    }
    let a = app();
    let mut f = FieldForm::new(&a);
    f.select("REVNUM");
    f.number_format = "ROMAN".into();
    f.preserve = false;
    assert_eq!(f.instruction(), r"REVNUM \* ROMAN");
    f.select("SUBJECT");
    assert_eq!(f.instruction(), "SUBJECT");
}

    #[test]
    fn date_time_dialog_uses_saved_pt_br_language() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.ui.date_time_language = "pt-BR".into();
        let InsertDialog::DateTime(mut f) = open("dateTime", &a).expect("dialog") else { panic!("not DateTime") };
        assert_eq!(f.format, "dd/MM/yyyy");
        assert_eq!(f.params()["language"], "pt-BR");
        f.update = false;
        assert_eq!(f.params()["update"], false);
    }

    /// Click the real egui language menu, date picture, checkbox and OK; ensure the resulting
    /// document contains pt-BR fixed text with the language on its run, not just JSON params.
    #[test]
    fn date_time_dialog_controls_insert_portuguese_fixed_text() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;

        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.ui.language = "en".into();
        app.ui.date_time_language = "en-US".into();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.state_mut().run("ui.dialog", json!({"name": "dateTime"})).unwrap();
        h.run_steps(4);

        // Go through the actual language combo box and then select a long Portuguese date.
        // Scope the click to this dialog: the ribbon has another English-language button.
        // The popup choice has a plain accessible label; the main ribbon uses a different control.
        h.get_by_role_and_label(Role::Window, "Date and Time").get_by_role(Role::ComboBox).click();
        h.run_steps(3);
        h.get_by_role_and_label(Role::Button, "Portuguese (Brazil)").click();
        h.run_steps(3);
        let sample = wordcraft_engine::cmd::insert::format_date_in("d 'de' MMMM 'de' yyyy", Some("pt-BR"));
        h.get_by_label(&sample).click();
        h.run_steps(2);
        h.get_by_label("Update automatically").click();
        h.run_steps(2);
        assert!(matches!(
            h.state().dialog.as_ref(),
            Some(crate::dialogs::Dialog::Insert(d)) if matches!(&**d, InsertDialog::DateTime(f) if f.language == "pt-BR" && f.format == "d 'de' MMMM 'de' yyyy" && !f.update)
        ));
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none(), "successful insertion closes the dialog");
        let doc = &h.state().session.doc;
        assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body), sample);
        let para = doc.para_at(&doc.start_of(wordcraft_doc::StoryRef::Body)).expect("inserted paragraph");
        assert_eq!(para.props_of_char(0).lang.as_deref(), Some("pt-BR"));
        assert!(para.objects.iter().all(|o| !matches!(o, wordcraft_doc::para::InlineObject::Field { .. })));
    }

    /// An invalid saved picture is reported in the modal, not silently dismissed when OK is clicked.
    #[test]
    fn date_time_dialog_keeps_open_and_displays_invalid_picture_error() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;

        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.ui.language = "en".into();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.state_mut().run("ui.dialog", json!({"name": "dateTime"})).unwrap();
        if let Some(crate::dialogs::Dialog::Insert(d)) = h.state_mut().dialog.as_mut()
            && let InsertDialog::DateTime(f) = &mut **d
        {
            f.format = "invalid\\picture".into();
        } else {
            panic!("date/time dialog expected");
        }
        h.run_steps(3);
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(3);
        assert!(matches!(
            h.state().dialog.as_ref(),
            Some(crate::dialogs::Dialog::Insert(d)) if matches!(&**d, InsertDialog::DateTime(f) if f.error.as_deref().is_some_and(|e| e.contains("invalid date picture")))
        ));
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "");
    }

