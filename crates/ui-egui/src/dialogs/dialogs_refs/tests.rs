use serde_json::json;
use wordcraft_doc::{InlineObject, StoryRef};
use wordcraft_engine::Session;

use super::*;
use crate::Services;

fn app(text: &str) -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::from_text(text)), Services::default())
}

fn form(a: &WordApp) -> RefsDialog {
    match &a.dialog {
        Some(Dialog::Refs(d)) => (**d).clone(),
        d => panic!("no references dialog: {d:?}"),
    }
}

fn instrs(a: &WordApp) -> Vec<String> {
    let doc = &a.session.doc;
    doc.para_paths(StoryRef::Body)
        .iter()
        .filter_map(|p| doc.para(StoryRef::Body, p))
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Field { instr, .. } => Some(instr.clone()),
            _ => None,
        })
        .collect()
}

/// #402: Insert Caption opens the dialog; a new label is kept with the interface settings,
/// and Numbering's chapter options become `STYLEREF` and `SEQ … \s` fields.
#[test]
fn caption_dialog_numbers_with_chapters() {
    let mut a = app("Results\nA chart");
    a.run("select.text", json!({"text": "Results"})).unwrap();
    a.run("styles.apply", json!({"style": "Heading 1"})).unwrap();
    a.run("select.text", json!({"text": "A chart"})).unwrap();
    assert_eq!(a.run("references.caption", json!({})).unwrap(), json!({"pending": "caption"}));
    let RefsDialog::Caption(mut f) = form(&a) else { panic!() };
    assert_eq!((f.label.as_str(), f.labels.len()), ("Figure", 3));
    // New Label…, then Numbering… with a chapter number.
    f.new_label = "Plate".into();
    a.dialog = None;
    a.ui.caption_labels.push(f.new_label.clone());
    f.label = "Plate".into();
    f.format = "ALPHABETIC".into();
    f.chapter = true;
    f.separator = ".".into();
    f.text = "Growth".into();
    assert_eq!(f.preview(), "Plate 1.A");
    a.run("references.caption", f.params()).unwrap();
    assert!(instrs(&a).contains(&"SEQ Plate \\* ALPHABETIC \\s 1".to_string()), "{:?}", instrs(&a));
    assert!(a.session.doc.plain_text(StoryRef::Body).contains("Plate 1.A: Growth"));
    // The label is offered next time and saved with the preferences.
    assert!(CaptionForm::read(&a).labels.contains(&"Plate".to_string()));
    assert_eq!(a.prefs().caption_labels, vec!["Plate".to_string()]);
}

/// #3120: Insert Caption picks the label used last, and saves it with the preferences.
#[test]
fn caption_dialog_remembers_the_last_label() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut a = app("A chart\nA photo");
    a.ui.caption_labels.push("Diagram".into());
    a.run("select.text", json!({"text": "A chart"})).unwrap();
    a.run("references.caption", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    h.get_by_value("Figure").click();
    h.run_steps(3);
    h.get_by_label("Diagram").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
    assert!(h.state().session.doc.plain_text(StoryRef::Body).contains("Diagram 1"));
    h.state_mut().run("select.text", json!({"text": "A photo"})).unwrap();
    h.state_mut().run("references.caption", json!({})).unwrap();
    h.run_steps(4);
    h.get_by_value("Diagram");
    assert_eq!(h.state().prefs().last_caption_label, "Diagram");
    // In a table it is still Table; a deleted label falls back to Figure.
    let a = h.state_mut();
    a.run("insert.table", json!({"rows": 1, "cols": 2})).unwrap();
    assert_eq!(CaptionForm::read(a).label, "Table");
    a.run("select.text", json!({"text": "A chart"})).unwrap();
    a.ui.caption_labels.clear();
    assert_eq!(CaptionForm::read(a).label, "Figure");
}

/// #916: Cross-reference from the ribbon opens a dialog listing the targets of each type
/// (equation captions too) and inserts a `REF` field to the one picked.
#[test]
fn cross_reference_dialog_lists_targets_and_inserts_a_ref_field() {
    let mut a = app("Kilns\nE=mc2\nSee ");
    a.run("select.text", json!({"text": "Kilns"})).unwrap();
    a.run("styles.apply", json!({"style": "Heading 1"})).unwrap();
    a.run("select.text", json!({"text": "E=mc2"})).unwrap();
    a.run("references.caption", json!({"label": "Equation", "text": "energy"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    assert_eq!(a.run("insert.crossReference", json!({})).unwrap(), json!({"pending": "crossReference"}));
    let RefsDialog::CrossRef(f) = form(&a) else { panic!() };
    assert_eq!((f.targets.clone(), f.selected), (vec!["Kilns".to_string()], Some(0)));
    let mut f = CrossRefForm::read(&a, "equation");
    assert_eq!(f.targets, vec!["Equation 1: energy".to_string()]);
    f.show = "number".into();
    a.dialog = None;
    a.run("insert.crossReference", f.params().unwrap()).unwrap();
    assert!(instrs(&a).iter().any(|i| i.starts_with("REF _Ref")), "{:?}", instrs(&a));
    assert!(a.session.doc.plain_text(StoryRef::Body).ends_with("See Equation 1"));
    // Nothing to pick: nothing to insert.
    assert_eq!(CrossRefForm::read(&a, "table").params(), None);
}

/// #2940: a label made with Insert Caption › New Label is a Reference type of the
/// Cross-reference dialog, whether the document has a caption with it yet or came with one.
#[test]
fn cross_reference_dialog_offers_custom_caption_labels() {
    let mut a = app("Rock\nSee ");
    let types = |a: &WordApp| ref_types(a).into_iter().map(|(v, _)| v).collect::<Vec<_>>();
    assert_eq!(types(&a), ["heading", "bookmark", "equation", "figure", "table"]);
    // Made in the Caption dialog but not used yet: listed, with nothing to pick.
    a.ui.caption_labels.push("Plate".into());
    assert!(types(&a).contains(&"label:Plate".to_string()));
    assert_eq!(CrossRefForm::read(&a, "label:Plate").params(), None);
    // A document's own captions bring theirs, once, and Figure stays the built-in one.
    a.ui.caption_labels.clear();
    a.run("select.text", json!({"text": "Rock"})).unwrap();
    a.run("references.caption", json!({"label": "Specimen", "text": "A rock"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("references.caption", json!({"label": "Figure", "text": "Another"})).unwrap();
    let t = types(&a);
    assert_eq!(t.iter().filter(|v| v.as_str() == "label:Specimen").count(), 1, "{t:?}");
    assert!(!t.iter().any(|v| v == "label:Figure"), "{t:?}");
    let mut f = CrossRefForm::read(&a, "label:Specimen");
    assert_eq!(f.targets, vec!["Specimen 1: A rock".to_string()]);
    f.show = "number".into();
    a.dialog = None;
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("insert.crossReference", f.params().unwrap()).unwrap();
    assert!(a.session.doc.plain_text(StoryRef::Body).ends_with("Specimen 1"), "{}", a.session.doc.plain_text(StoryRef::Body));
}

/// #2940: through the window: the dialog open on a custom label lists its caption, and the
/// Insert button puts a reference to it in the text. (The Reference type box and its items
/// are not in the accessibility tree the test harness clicks, so the type is set on the form.)
#[test]
fn cross_reference_dialog_inserts_a_reference_to_a_custom_label() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut a = app("Rock\nSee ");
    a.run("select.text", json!({"text": "Rock"})).unwrap();
    a.run("references.caption", json!({"label": "Specimen", "text": "A rock", "format": "ROMAN"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    assert_eq!(a.run("insert.crossReference", json!({})).unwrap(), json!({"pending": "crossReference"}));
    let form = CrossRefForm::read(&a, "label:Specimen");
    assert_eq!(form.targets, vec!["Specimen I: A rock".to_string()]);
    a.dialog = Some(Dialog::Refs(Box::new(RefsDialog::CrossRef(form))));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    h.get_by_role_and_label(Role::Window, "Cross-reference");
    h.get_all_by_role_and_label(Role::Button, "Insert").last().expect("the dialog's Insert button").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "Insert closes the dialog");
    let text = h.state().session.doc.plain_text(StoryRef::Body);
    assert!(text.ends_with("See Specimen I: A rock"), "{text}");
    assert!(instrs(h.state()).iter().any(|i| i.starts_with("REF _Ref")), "{:?}", instrs(h.state()));
}

/// #402: Mark Entry opens with the selection; Insert Index opens with Word's defaults and
/// reads the index back; both write their switches.
#[test]
fn index_and_mark_entry_dialogs_write_switches() {
    let mut a = app("Kilns are hot.\nMore text.");
    a.run("select.text", json!({"text": "Kilns"})).unwrap();
    assert_eq!(a.run("references.markEntry", json!({})).unwrap(), json!({"pending": "markEntry"}));
    let RefsDialog::MarkEntry(mut m) = form(&a) else { panic!() };
    assert_eq!(m.entry, "Kilns");
    m.subentry = "firing".into();
    m.italic = true;
    a.run("references.markEntry", m.params(false)).unwrap();
    assert!(instrs(&a).contains(&"XE \"Kilns:firing\" \\i".to_string()), "{:?}", instrs(&a));
    a.run("caret.docEnd", json!({})).unwrap();
    assert_eq!(a.run("references.index", json!({})).unwrap(), json!({"pending": "index"}));
    let RefsDialog::Index(mut f) = form(&a) else { panic!() };
    assert_eq!((f.columns, f.kind.as_str(), f.headings), (2, "indented", true));
    f.kind = "runIn".into();
    f.right_align = true;
    a.run("references.index", f.params()).unwrap();
    assert!(instrs(&a).contains(&"INDEX \\h \"A\" \\c \"2\" \\e \"\t\" \\r".to_string()), "{:?}", instrs(&a));
    assert_eq!(IndexForm::read(&a), f, "the dialog shows the index's settings");
}

/// #402: Custom Table of Contents and its Options make the TOC switches, and the dialog
/// opens again with them.
#[test]
fn custom_table_of_contents_dialog() {
    let mut a = app("Intro\nNote");
    a.run("select.text", json!({"text": "Intro"})).unwrap();
    a.run("styles.apply", json!({"style": "Heading 1"})).unwrap();
    a.run("select.text", json!({"text": "Note"})).unwrap();
    a.run("styles.apply", json!({"style": "Quote"})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    a.run("ui.dialog", json!({"name": "toc"})).unwrap();
    let RefsDialog::Toc(mut f) = form(&a) else { panic!() };
    assert_eq!((f.levels, f.hyperlinks, f.right_align, f.heading_styles, f.outline_levels), (3, true, true, true, true));
    f.levels = 2;
    f.hyperlinks = false;
    f.leader = "hyphen".into();
    // Options…: the Quote style at level 2.
    f.style_levels.iter_mut().find(|(n, _)| n == "Quote").unwrap().1 = 2;
    a.run("references.toc", f.params()).unwrap();
    assert_eq!(instrs(&a)[0], "TOC \\o \"1-2\" \\z \\u \\t \"Quote,2\"");
    let text = a.session.doc.plain_text(StoryRef::Body);
    assert!(text.contains("Intro\t1") && text.contains("Note\t1"), "{text}");
    assert_eq!(TocForm::read(&a), f);
}

/// #3013: Insert Table of Figures opens its dialog, which offers the document's own caption
/// labels, so a list of tables (or of a custom label) can be built next to a list of figures.
#[test]
fn table_of_figures_dialog_picks_the_caption_label() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut a = app("Kiln\nPress\nRock\n");
    for (text, label) in [("Kiln", "Figure"), ("Press", "Table"), ("Rock", "Specimen")] {
        a.run("select.text", json!({"text": text})).unwrap();
        a.run("references.caption", json!({"label": label, "text": text})).unwrap();
    }
    a.run("caret.docEnd", json!({})).unwrap();
    assert_eq!(a.run("references.tableOfFigures", json!({})).unwrap(), json!({"pending": "tableOfFigures"}));
    let RefsDialog::Figures(mut f) = form(&a) else { panic!() };
    assert_eq!(f.label, "Figure");
    assert_eq!(f.labels, ["Equation", "Figure", "Table", "Specimen"]);
    f.label = "Table".into();
    f.leader = "underscore".into();
    a.dialog = Some(Dialog::Refs(Box::new(RefsDialog::Figures(f))));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    h.get_by_role_and_label(Role::Window, "Table of Figures");
    // Without page numbers there is nothing to align: the margin choice is off until they are back.
    let margin = |h: &egui_kittest::Harness<'_, WordApp>| {
        h.get_by_role_and_label(Role::CheckBox, "Page numbers at the right margin").accesskit_node().is_disabled()
    };
    assert!(!margin(&h));
    h.get_by_role_and_label(Role::CheckBox, "Include page numbers").click();
    h.run_steps(2);
    assert!(margin(&h), "no page numbers: no right margin choice");
    h.get_by_role_and_label(Role::CheckBox, "Include page numbers").click();
    h.run_steps(2);
    assert!(!margin(&h));
    h.get_all_by_role_and_label(Role::Button, "OK").last().expect("the dialog's OK button").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
    let mut a = h.into_state();
    assert!(instrs(&a).contains(&r#"TOC \c "Table""#.to_string()), "{:?}", instrs(&a));
    let entries: Vec<(String, Option<wordcraft_doc::props::TabLeader>)> = a
        .session
        .doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref() == Some("ListEntry"))
        .map(|p| (p.plain_text(), p.props.tabs.as_ref().and_then(|t| t.last()).map(|t| t.leader)))
        .collect();
    assert_eq!(entries, [("Table 1: Press\t1".to_string(), Some(wordcraft_doc::props::TabLeader::Underscore))]);
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("ui.dialog", json!({"name": "tableOfFigures"})).unwrap();
    let RefsDialog::Figures(mut f) = form(&a) else { panic!() };
    f.label = "Specimen".into();
    f.include_label = false;
    f.right_align = false;
    a.run("references.tableOfFigures", f.params()).unwrap();
    assert!(instrs(&a).contains(&r#"TOC \a "Specimen" \p " ""#.to_string()), "{:?}", instrs(&a));
    assert!(a.session.doc.plain_text(StoryRef::Body).contains("Rock 1"));
}

/// #402: Insert Citation without a source opens Create Source; the type picks the fields,
/// "show all" adds the rest, and Edit Source from the Source Manager updates the citation.
#[test]
fn create_and_edit_source() {
    let mut a = app("Studios matter ");
    a.run("caret.docEnd", json!({})).unwrap();
    assert_eq!(a.run("references.citation", json!({})).unwrap(), json!({"pending": "createSource"}));
    let RefsDialog::Source(mut f) = form(&a) else { panic!() };
    let f = &mut *f;
    f.source.kind = "website".into();
    let shown = f.fields().len();
    assert!(f.fields().iter().any(|(k, l)| *k == "url" && *l == "URL"));
    f.show_all = true;
    assert!(f.fields().len() > shown && f.fields().iter().any(|(k, _)| *k == "doi"));
    for (k, v) in [("author", "Rivera, Alex"), ("title", "Kilns Online"), ("year", "2024"), ("url", "https://example.org"), ("doi", "10.1/x")] {
        *field_mut(&mut f.source, k).unwrap() = v.into();
    }
    a.run("references.citation", json!({"source": serde_json::to_value(&f.source).unwrap()})).unwrap();
    assert!(a.session.doc.plain_text(StoryRef::Body).contains("(Rivera, 2024)"));
    assert_eq!(a.session.doc.sources[0].doi, "10.1/x");
    // Manage Sources › Edit…: a new year shows in the citation.
    assert_eq!(a.run("references.sources", json!({})).unwrap(), json!({"pending": "sourceManager"}));
    let mut src = a.session.doc.sources[0].clone();
    src.year = "2025".into();
    a.run("references.sources", json!({"add": src})).unwrap();
    assert!(a.session.doc.plain_text(StoryRef::Body).contains("(Rivera, 2025)"), "{}", a.session.doc.plain_text(StoryRef::Body));
    assert_eq!(a.session.doc.sources.len(), 1);
}

/// #2640: the Source Manager's Import… asks the host's file dialog for a Word bibliography
/// file (`b:Sources`, Word's own `Sources.xml`) and adds its sources to the document, shown
/// in the list at once.
#[test]
fn the_source_manager_imports_a_word_sources_file() {
    use egui_kittest::kittest::Queryable;

    let dir = std::env::temp_dir().join(format!("wc-refs-import-sources-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Sources.xml");
    std::fs::write(
        &path,
        r#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
  <b:Source><b:Tag>Riv21</b:Tag><b:SourceType>Book</b:SourceType><b:Title>Shared Spaces</b:Title><b:Year>2021</b:Year></b:Source>
  <b:Source><b:Tag>Oka19</b:Tag><b:SourceType>Book</b:SourceType><b:Title>Kilns and Clay</b:Title><b:Year>2019</b:Year></b:Source>
</b:Sources>"#,
    )
    .unwrap();

    let mut a = app("Studios matter ");
    let asked = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let (seen, picked) = (asked.clone(), path.to_string_lossy().to_string());
    a.services.pick_open = Some(Box::new(move |purpose| {
        *seen.borrow_mut() = purpose.to_string();
        Some(picked.clone())
    }));
    assert_eq!(a.run("references.sources", json!({})).unwrap(), json!({"pending": "sourceManager"}));
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..4 {
        h.step();
    }
    h.get_by_label("Import…").click();
    for _ in 0..4 {
        h.step();
    }
    assert_eq!(*asked.borrow(), "sources", "the host can offer Word sources files");
    assert_eq!(h.state().session.doc.sources.len(), 2);
    assert!(h.state().status_msg.as_ref().is_some_and(|(m, _)| m.contains("Imported 2 sources")), "{:?}", h.state().status_msg);
    let _ = std::fs::remove_dir_all(&dir);
}
