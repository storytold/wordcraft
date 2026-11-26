use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;
use crate::WordApp;
use crate::dialogs::host::{FormHost, open_form, testing};

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Default::default())
}

fn drop_cap_form(a: &mut WordApp) -> Option<DropCapForm> {
    let id = crate::dialogs::open(a, "dropCap", &json!({}))?;
    a.dialog_host.get_by_id::<FormHost<DropCapForm>>(id).map(|h| h.form().clone())
}

/// #2601: Drop Cap Options… opens on the paragraph's drop cap (the defaults without one:
/// 3 lines, distance 0, the text's font), and OK runs `insert.dropCap` with the chosen values; it
/// reopens showing them. Choosing the text's own font sends none.
#[test]
fn drop_cap_dialog_prefills_and_applies() {
    let mut a = app();
    let mut f = drop_cap_form(&mut a).expect("the dialog");
    assert_eq!((f.position.as_str(), f.lines, f.distance), ("none", 3, 0.0));
    assert_eq!(f.font, f.text_font, "the text's font");
    assert!(!f.font.is_empty());
    assert_eq!(f.params(), json!({"position": "none"}));
    f.position = "margin".into();
    f.lines = 4;
    f.distance = wordcraft_geom::PT_PER_CM;
    a.run("insert.dropCap", f.params()).unwrap();
    let props = |a: &WordApp| a.session.doc.body.first().and_then(|b| b.as_para()).map(|p| p.props.clone()).unwrap_or_default();
    let p = props(&a);
    assert_eq!((p.drop_cap, p.drop_cap_margin, p.drop_cap_font.as_deref()), (Some(4), Some(true), None));
    assert!((p.drop_cap_distance.unwrap_or_default() - wordcraft_geom::PT_PER_CM).abs() < 0.01, "1 cm: {:?}", p.drop_cap_distance);
    let mut f = drop_cap_form(&mut a).expect("the dialog");
    assert_eq!((f.position.as_str(), f.lines), ("margin", 4));
    assert!((f.distance - wordcraft_geom::PT_PER_CM).abs() < 0.01, "{} pt", f.distance);
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

/// More Columns: two unequal columns apply as one undo step through the dialog's held OK, with the
/// section's separator kept, and the dialog reopens unequal.
#[test]
fn columns_dialog_applies_unequal_columns_in_one_step() {
    use craft::text::ui::columns::TextColumnsDialog;
    let mut a = app();
    a.run("layout.columns", json!({"count": 1, "separator": true})).unwrap();
    let mut h = testing::harness(a);
    let id = crate::dialogs::open(h.state_mut(), "columns", &json!({})).expect("the dialog");
    h.run_steps(2);
    {
        let k = h.state_mut().dialog_host.get_mut_by_id::<crate::dialogs::host::Keyed<TextColumnsDialog>>(id).expect("columns dialog");
        let d = k.inner_mut();
        d.set_count(2);
        // Equal widths give a width change to the spacing, so the columns are made unequal first, as the
        // Left preset or unticking Equal column width does.
        d.apply_preset(craft::text::ui::columns::ColumnsPreset::Left);
        d.set_width(0, 300.0);
    }
    let undo_before = h.state().session.undo_depth();
    testing::confirm(&mut h, id);
    let a = h.state();
    let c = wordcraft_engine::cmd::page::sect(&a.session).columns;
    assert_eq!((c.count, c.widths.len(), c.separator), (2, 2, true));
    assert_eq!(a.session.undo_depth(), undo_before + 1, "one undo step");
    assert!(a.dialog_host.get_dyn(id).is_none(), "the accepted dialog closed");
    let (again, _, _, _) = columns::read(a);
    assert_eq!((again.equal_width, again.widths.len()), (Some(false), 2), "the dialog reopens unequal");
    let a = h.state_mut();
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(wordcraft_engine::cmd::page::sect(&a.session).columns.count, 1);
}

/// With text selected, More Columns applies to the selected text by default, as in Word
/// (#2024): the other paragraphs keep one column.
#[test]
fn columns_dialog_applies_to_the_selected_text() {
    use craft::text::ui::columns::ColumnsApply;
    let mut a = app();
    assert!(!columns::read(&a).1.contains(&ColumnsApply::Selection), "no selection, no Selected text");
    a.run("select.paragraph", json!({})).unwrap();
    let (mut cols, choices, current, _) = columns::read(&a);
    assert!(choices.contains(&ColumnsApply::Selection));
    assert_eq!(current, ColumnsApply::Selection);
    cols.count = Some(2);
    a.run("layout.columns", columns::columns_params(&cols, current)).unwrap();
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

/// #7326: Recently used symbols and Symbols map use compact tile sizing so the dialog
/// fits comfortably within default window bounds without vertical overflow.
#[test]
fn symbol_dialog_uses_compact_tile_grids() {
    let mut a = app();
    remember_symbol(&mut a, "©", "DejaVu Sans");
    let mut h = testing::harness(a);
    let id = crate::dialogs::open(h.state_mut(), "symbol", &json!({})).expect("symbol dialog opens");
    h.run_steps(3);
    assert!(h.state().dialog_host.get_dyn(id).is_some(), "the symbol dialog is open");
    let dialog = h.get_by_role_and_label(egui::accesskit::Role::Dialog, "Symbol").rect();
    assert!(dialog.height() <= 900.0, "complete dialog should fit within default desktop viewport: {}", dialog.height());
    assert!(dialog.max.y <= 900.0, "complete dialog should not overflow viewport bottom: {}", dialog.max.y);
    let symbols = h.get_by_role_and_label(egui::accesskit::Role::ListBox, "Symbols").rect();
    assert!(symbols.height() < 500.0, "symbols grid should be compact: {}", symbols.height());
    let recent = h.get_by_role_and_label(egui::accesskit::Role::ListBox, "Recently used symbols:").rect();
    assert!(recent.height() < 80.0, "recently used symbols row should be compact: {}", recent.height());
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

/// Text fields take a capitalization Format: each choice writes its `\*` switch before
/// `\* MERGEFORMAT`, after the field's own arguments and switches, and (none) writes none.
#[test]
fn field_dialog_text_format_writes_its_switch() {
    let a = app();
    let mut f = FieldForm::new(&a);
    f.select("AUTHOR");
    assert_eq!(f.text_format, "");
    assert_eq!(f.instruction(), r"AUTHOR \* MERGEFORMAT");
    for v in ["Upper", "Lower", "FirstCap", "Caps"] {
        f.text_format = v.into();
        assert_eq!(f.instruction(), format!(r"AUTHOR \* {v} \* MERGEFORMAT"));
    }
    let labels: Vec<&str> = field::TEXT_FORMATS.iter().map(|(_, l)| *l).collect();
    assert_eq!(labels, ["(none)", "Uppercase", "Lowercase", "First capital", "Title case"]);
    f.select("REF");
    f.target = "intro".into();
    f.text_format = "Upper".into();
    assert_eq!(f.instruction(), r"REF intro \h \* Upper \* MERGEFORMAT");
    f.select("STYLEREF");
    f.style_switches = "l".into();
    f.text_format = "Caps".into();
    f.preserve = false;
    assert_eq!(f.instruction(), r#"STYLEREF "Heading 1" \l \* Caps"#);
    f.select("MERGEFIELD");
    f.target = "City".into();
    f.text_format = "Lower".into();
    assert_eq!(f.params().unwrap(), json!({"instr": r"MERGEFIELD City \* Lower", "result": "«City»"}));
    // Choosing another field starts from (none).
    f.select("TITLE");
    assert_eq!((f.text_format.as_str(), f.instruction().as_str()), ("", "TITLE"));
}

/// The list is offered for the text fields only; date, number, PAGEREF, SEQ and TOC keep their
/// own options and never take a capitalization switch from it.
#[test]
fn field_dialog_text_format_is_for_text_fields_only() {
    for name in ["AUTHOR", "COMMENTS", "FILENAME", "KEYWORDS", "LASTSAVEDBY", "SUBJECT", "TITLE", "REF", "STYLEREF", "MERGEFIELD"] {
        assert!(field::takes_text_format(name), "{name}");
    }
    let a = app();
    let mut f = FieldForm::new(&a);
    for name in ["DATE", "PAGE", "NUMPAGES", "PAGEREF", "SEQ", "TOC"] {
        assert!(!field::takes_text_format(name), "{name}");
        f.select(name);
        let before = f.instruction();
        f.text_format = "Upper".into();
        assert_eq!(f.instruction(), before, "{name}");
    }
}

/// Hide Codes sets the Format list from the typed code's last capitalization switch outside
/// quotes, and to (none) when the code has none.
#[test]
fn field_dialog_text_format_follows_the_typed_code() {
    assert_eq!(field::text_formats(r#"AUTHOR "a \* Lower" \* upper \* Caps \* MERGEFORMAT"#), ["Upper", "Caps"]);
    assert!(field::text_formats(r#"MERGEFIELD "x \* FirstCap""#).is_empty());
    assert!(field::text_formats(r"PAGE \* roman").is_empty());
    let a = app();
    let mut f = FieldForm::new(&a);
    f.select("SUBJECT");
    f.toggle_codes();
    assert_eq!(f.code, r"SUBJECT \* MERGEFORMAT");
    f.code = r"SUBJECT \* FIRSTCAP \* MERGEFORMAT".into();
    f.toggle_codes();
    assert!(!f.codes);
    assert_eq!(f.instruction(), r"SUBJECT \* FirstCap \* MERGEFORMAT");
    f.toggle_codes();
    f.code = "SUBJECT".into();
    f.toggle_codes();
    assert_eq!(f.text_format, "");
    // A code for another field leaves the list as it was.
    f.text_format = "Lower".into();
    f.toggle_codes();
    f.code = r"TITLE \* Upper".into();
    f.toggle_codes();
    assert_eq!(f.text_format, "Lower");
}

/// Hide Codes keeps every capitalization switch of the typed code, in order, so the inserted field
/// shows what the typed code shows; choosing another Format writes that one switch only.
#[test]
fn field_dialog_text_format_keeps_every_typed_case_switch() {
    use wordcraft_layout::fields::apply_case_switch;
    let a = app();
    let mut f = FieldForm::new(&a);
    f.select("SUBJECT");
    f.toggle_codes();
    f.code = r"SUBJECT \* Lower \* Caps".into();
    let typed = apply_case_switch(&f.code, "WEEKLY REPORT".into());
    assert_eq!(typed, "Weekly Report");
    f.toggle_codes();
    assert_eq!(f.text_format, "Caps");
    assert_eq!(f.final_code(), r"SUBJECT \* Lower \* Caps \* MERGEFORMAT");
    assert_eq!(apply_case_switch(&f.final_code(), "WEEKLY REPORT".into()), typed);
    // Field Codes shows the same switches again.
    f.toggle_codes();
    assert_eq!(f.code, r"SUBJECT \* Lower \* Caps \* MERGEFORMAT");
    f.toggle_codes();
    // A Format the user chooses replaces the typed switches, and choosing Title case again
    // does not bring them back.
    f.choose_text_format("Upper");
    assert_eq!(f.final_code(), r"SUBJECT \* Upper \* MERGEFORMAT");
    f.choose_text_format("Caps");
    assert_eq!(f.final_code(), r"SUBJECT \* Caps \* MERGEFORMAT");
    f.choose_text_format("");
    assert_eq!(f.final_code(), r"SUBJECT \* MERGEFORMAT");
    // Choosing another field forgets them.
    f.toggle_codes();
    f.code = r"SUBJECT \* Lower \* Caps".into();
    f.toggle_codes();
    f.select("SUBJECT");
    f.text_format = "Caps".into();
    assert_eq!(f.final_code(), r"SUBJECT \* Caps \* MERGEFORMAT");
}

/// A typed `MERGEFIELD` code's type ends at a quote as well as a space, as mail merge reads it.
#[test]
fn field_dialog_reads_a_typed_merge_field_with_no_space_before_its_quote() {
    let a = app();
    let mut f = FieldForm::new(&a);
    f.select("MERGEFIELD");
    f.toggle_codes();
    f.code = r#"MERGEFIELD"City" \* Lower"#.into();
    assert_eq!(f.params().unwrap()["result"], "«City»");
    f.toggle_codes();
    assert_eq!(f.text_format, "Lower");
    // Another field whose name only starts the same way is not this one.
    f.choose_text_format("");
    f.toggle_codes();
    f.code = r"MERGEFIELDX \* Lower".into();
    f.toggle_codes();
    assert_eq!(f.text_format, "");
}

/// The Date & Time dialog uses the saved pt-BR editing language when present in UI preferences.
#[test]
fn date_time_dialog_uses_saved_pt_br_language() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.ui.date_time_language = "pt-BR".into();
    let mut f = DateTimeForm::new(&a);
    assert_eq!(f.format, "dd/MM/yyyy");
    assert_eq!(f.params()["language"], "pt-BR");
    assert!(!f.update);
    f.update = false;
    assert_eq!(f.params()["update"], false);
}

/// Language tags with primary subtag `pt` (such as `pt_BR` or `pt-PT`) are normalized to `pt-BR`.
#[test]
fn date_time_dialog_normalizes_portuguese_language_tags() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.ui.date_time_language = "pt_BR".into();
    let f = DateTimeForm::new(&a);
    assert_eq!(f.language, "pt-BR");
    assert_eq!(f.format, "dd/MM/yyyy");

    a.ui.date_time_language = "pt-PT".into();
    let f = DateTimeForm::new(&a);
    assert_eq!(f.language, "pt-BR");
}

/// One table form to `insert.dateTime`: pt-BR fixed text carries its language on the run; `update`
/// inserts a refreshable field.
#[test]
fn date_time_dialog_controls_insert_portuguese_fixed_text() {
    let picture = "d 'de' MMMM 'de' yyyy";
    for (language, update) in [("pt-BR", false), ("en-US", true)] {
        let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        let form = DateTimeForm { format: if language == "pt-BR" { picture.into() } else { "M/d/yyyy".into() }, language: language.into(), update };
        let mut h = testing::harness(a);
        let id = open_form(h.state_mut(), None, form);
        h.run_steps(2);
        testing::confirm(&mut h, id);
        let doc = &h.state().session.doc;
        let para = doc.para_at(&doc.start_of(wordcraft_doc::StoryRef::Body)).expect("inserted paragraph");
        let has_field = para.objects.iter().any(|o| matches!(o, wordcraft_doc::para::InlineObject::Field { .. }));
        assert_eq!(has_field, update, "{language}: update gives a field");
        if !update {
            let sample = wordcraft_engine::cmd::insert::format_date_in(picture, Some("pt-BR"));
            assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body), sample);
            assert_eq!(para.props_of_char(0).lang.as_deref(), Some("pt-BR"));
        }
    }
}

/// An invalid picture is refused with the engine's message: the dialog stays open and the document
/// is unchanged.
#[test]
fn date_time_dialog_keeps_open_and_displays_invalid_picture_error() {
    let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let form = DateTimeForm { format: "invalid\\picture".into(), language: "en-US".into(), update: false };
    let mut h = testing::harness(a);
    let id = open_form(h.state_mut(), None, form);
    h.run_steps(2);
    testing::confirm(&mut h, id);
    let a = h.state();
    match a.dialog_host.held_state(id) {
        Some(craft::ui::dialog::HeldOk::Refused(message)) => assert!(message.as_str().contains("invalid date picture"), "{message:?}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(a.dialog_host.get_dyn(id).is_some(), "the dialog stays open");
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "");
}

/// #4832: Document Information fields are offered and format instructions match the chosen
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
